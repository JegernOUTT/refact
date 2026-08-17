use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use regex::{Regex, RegexBuilder};

use refact_core::string_utils::redact_sensitive;
use refact_integrations::browser_models::{ExecutionReport, NetworkReportMode};
use refact_integrations::browser_types::{NetworkEntry, NetworkTiming};

pub const NETWORK_IDLE_MS: u64 = 500;
const RECENT_EVENT_CAP: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkLoadState {
    Domcontentloaded,
    Load,
    Networkidle,
}

#[derive(Clone, Debug)]
pub struct RequestStarted {
    pub request_id: String,
    pub loader_id: String,
    pub frame_id: Option<String>,
    pub method: String,
    pub url: String,
    pub resource_type: String,
    pub headers: BTreeMap<String, String>,
    pub timestamp: f64,
    pub redirect_response: Option<ResponseReceived>,
}

#[derive(Clone, Debug)]
pub struct ResponseReceived {
    pub request_id: String,
    pub status: u16,
    pub status_text: String,
    pub headers: BTreeMap<String, String>,
    pub timestamp: f64,
    pub encoded_data_length: Option<u64>,
    pub from_service_worker: bool,
    pub request_start: Option<f64>,
    pub response_start: Option<f64>,
}

#[derive(Clone, Debug)]
struct TrackedRequest {
    entry: NetworkEntry,
    matching_url: String,
    counted: bool,
}

#[derive(Clone, Debug, Default)]
struct FrameNetworkState {
    parent_id: Option<String>,
    children: HashSet<String>,
    inflight: HashSet<String>,
    idle_since_ms: Option<u64>,
    domcontentloaded: bool,
    load: bool,
}

#[derive(Clone, Debug, Default)]
struct NetworkIdleTracker {
    frames: HashMap<String, FrameNetworkState>,
    main_frame_id: Option<String>,
}

impl NetworkIdleTracker {
    fn attach(&mut self, frame_id: String, parent_id: Option<String>, now_ms: u64) {
        if parent_id.is_none() && self.main_frame_id.is_none() {
            self.main_frame_id = Some(frame_id.clone());
        }
        let previous_parent = self
            .frames
            .get(&frame_id)
            .and_then(|frame| frame.parent_id.clone());
        if previous_parent != parent_id {
            if let Some(previous_parent) = previous_parent {
                if let Some(parent) = self.frames.get_mut(&previous_parent) {
                    parent.children.remove(&frame_id);
                }
            }
        }
        let frame = self.frames.entry(frame_id.clone()).or_default();
        frame.parent_id = parent_id.clone();
        frame.idle_since_ms.get_or_insert(now_ms);
        if let Some(parent_id) = parent_id {
            self.frames
                .entry(parent_id)
                .or_default()
                .children
                .insert(frame_id);
        }
    }

    fn detach(&mut self, frame_id: &str) {
        let children = self
            .frames
            .get(frame_id)
            .map(|frame| frame.children.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for child in children {
            self.detach(&child);
        }
        if let Some(frame) = self.frames.remove(frame_id) {
            if let Some(parent_id) = frame.parent_id {
                if let Some(parent) = self.frames.get_mut(&parent_id) {
                    parent.children.remove(frame_id);
                }
            }
        }
        if self.main_frame_id.as_deref() == Some(frame_id) {
            self.main_frame_id = None;
        }
    }

    fn request_started(&mut self, frame_id: &str, request_id: &str, now_ms: u64) {
        let frame = self.frames.entry(frame_id.to_string()).or_default();
        frame.inflight.insert(request_id.to_string());
        frame.idle_since_ms = None;
        if self.main_frame_id.is_none() {
            self.main_frame_id = Some(frame_id.to_string());
        }
        let _ = now_ms;
    }

    fn request_finished(&mut self, frame_id: &str, request_id: &str, now_ms: u64) {
        if let Some(frame) = self.frames.get_mut(frame_id) {
            frame.inflight.remove(request_id);
            if frame.inflight.is_empty() {
                frame.idle_since_ms = Some(now_ms);
            }
        }
    }

    fn lifecycle(&mut self, frame_id: &str, name: &str, now_ms: u64) {
        let frame = self.frames.entry(frame_id.to_string()).or_default();
        match name.to_ascii_lowercase().as_str() {
            "init" => {
                frame.domcontentloaded = false;
                frame.load = false;
                frame.idle_since_ms = if frame.inflight.is_empty() {
                    Some(now_ms)
                } else {
                    None
                };
            }
            "domcontentloaded" => frame.domcontentloaded = true,
            "load" => frame.load = true,
            _ => {}
        }
        if self.main_frame_id.is_none() {
            self.main_frame_id = Some(frame_id.to_string());
        }
    }

    fn load_state(&self, state: NetworkLoadState, now_ms: u64) -> bool {
        let Some(main_frame_id) = &self.main_frame_id else {
            return false;
        };
        match state {
            NetworkLoadState::Domcontentloaded => self
                .frames
                .get(main_frame_id)
                .is_some_and(|frame| frame.domcontentloaded),
            NetworkLoadState::Load => self
                .frames
                .get(main_frame_id)
                .is_some_and(|frame| frame.load),
            NetworkLoadState::Networkidle => self.subtree_idle(main_frame_id, now_ms),
        }
    }

    fn subtree_idle(&self, frame_id: &str, now_ms: u64) -> bool {
        let Some(frame) = self.frames.get(frame_id) else {
            return false;
        };
        let self_idle = frame.inflight.is_empty()
            && frame
                .idle_since_ms
                .is_some_and(|since| now_ms.saturating_sub(since) >= NETWORK_IDLE_MS);
        self_idle
            && frame
                .children
                .iter()
                .all(|child| self.subtree_idle(child, now_ms))
    }
}

#[derive(Clone, Debug)]
struct SequencedEntry {
    sequence: u64,
    matching_url: String,
    entry: NetworkEntry,
}

#[derive(Debug)]
struct NetworkMonitor {
    started_at: Instant,
    active: HashMap<String, TrackedRequest>,
    request_extra: HashMap<String, BTreeMap<String, String>>,
    response_extra: HashMap<String, BTreeMap<String, String>>,
    completed: Vec<NetworkEntry>,
    requests: VecDeque<SequencedEntry>,
    responses: VecDeque<SequencedEntry>,
    request_sequence: u64,
    response_sequence: u64,
    idle: NetworkIdleTracker,
}

impl Default for NetworkMonitor {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            active: HashMap::new(),
            request_extra: HashMap::new(),
            response_extra: HashMap::new(),
            completed: Vec::new(),
            requests: VecDeque::new(),
            responses: VecDeque::new(),
            request_sequence: 0,
            response_sequence: 0,
            idle: NetworkIdleTracker::default(),
        }
    }
}

impl NetworkMonitor {
    fn now_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    fn record_request(&mut self, event: RequestStarted) {
        let now_ms = self.now_ms();
        let matching_url = event.url.clone();
        let redirect_from = if let Some(redirect) = event.redirect_response.as_ref() {
            self.finish_redirect(&event.request_id, redirect.clone());
            self.completed.last().map(|entry| entry.url.clone())
        } else {
            None
        };
        let request_headers = self
            .request_extra
            .remove(&event.request_id)
            .unwrap_or(event.headers);
        let is_navigation_request = event.request_id == event.loader_id
            && event.resource_type.eq_ignore_ascii_case("document");
        let document_id = is_navigation_request.then(|| event.loader_id.clone());
        let entry = NetworkEntry {
            timestamp: event.timestamp,
            method: event.method,
            url: mask_text(&event.url),
            resource_type: event.resource_type,
            status: None,
            status_text: None,
            request_headers: mask_headers(request_headers),
            response_headers: BTreeMap::new(),
            frame_id: event.frame_id.clone(),
            loader_id: Some(event.loader_id),
            document_id,
            redirect_from,
            timing: Some(NetworkTiming {
                start_time: event.timestamp,
                request_start: None,
                response_start: None,
                response_end: None,
            }),
            encoded_data_length: None,
            transfer_size: None,
            failure_text: None,
            from_service_worker: false,
            is_navigation_request,
        };
        let counted = counts_for_network_idle(&matching_url, &entry.resource_type);
        if counted {
            if let Some(frame_id) = event.frame_id.as_deref() {
                self.idle
                    .request_started(frame_id, &event.request_id, now_ms);
            }
        }
        self.request_sequence += 1;
        push_recent(
            &mut self.requests,
            SequencedEntry {
                sequence: self.request_sequence,
                matching_url: matching_url.clone(),
                entry: entry.clone(),
            },
        );
        self.active.insert(
            event.request_id,
            TrackedRequest {
                entry,
                matching_url,
                counted,
            },
        );
    }

    fn record_request_extra(&mut self, request_id: String, headers: BTreeMap<String, String>) {
        if let Some(request) = self.active.get_mut(&request_id) {
            request.entry.request_headers = mask_headers(headers);
        } else {
            self.request_extra.insert(request_id, headers);
        }
    }

    fn record_response(&mut self, response: ResponseReceived) {
        let Some(request) = self.active.get_mut(&response.request_id) else {
            return;
        };
        request.entry.status = Some(response.status);
        request.entry.status_text = Some(mask_text(&response.status_text));
        if response.status >= 400 {
            request.entry.failure_text = Some(mask_text(&format!(
                "HTTP {} {}",
                response.status, response.status_text
            )));
        }
        request.entry.response_headers = mask_headers(
            self.response_extra
                .remove(&response.request_id)
                .unwrap_or(response.headers),
        );
        request.entry.encoded_data_length = response.encoded_data_length;
        request.entry.from_service_worker = response.from_service_worker;
        if let Some(timing) = request.entry.timing.as_mut() {
            timing.request_start = response.request_start;
            timing.response_start = response.response_start.or(Some(response.timestamp));
        }
        self.response_sequence += 1;
        push_recent(
            &mut self.responses,
            SequencedEntry {
                sequence: self.response_sequence,
                matching_url: request.matching_url.clone(),
                entry: request.entry.clone(),
            },
        );
    }

    fn record_response_extra(&mut self, request_id: String, headers: BTreeMap<String, String>) {
        if let Some(request) = self.active.get_mut(&request_id) {
            request.entry.response_headers = mask_headers(headers);
        } else {
            self.response_extra.insert(request_id, headers);
        }
    }

    fn finish(
        &mut self,
        request_id: &str,
        timestamp: f64,
        encoded_data_length: Option<u64>,
    ) -> Option<NetworkEntry> {
        let Some(mut request) = self.active.remove(request_id) else {
            return None;
        };
        if let Some(length) = encoded_data_length {
            request.entry.encoded_data_length = Some(length);
            request.entry.transfer_size = Some(length);
        }
        if let Some(timing) = request.entry.timing.as_mut() {
            timing.response_end = Some(timestamp);
        }
        self.finish_inflight(request_id, &request);
        self.completed.push(request.entry.clone());
        Some(request.entry)
    }

    fn fail(
        &mut self,
        request_id: &str,
        timestamp: f64,
        failure_text: String,
    ) -> Option<NetworkEntry> {
        let Some(mut request) = self.active.remove(request_id) else {
            return None;
        };
        let failure_text = mask_text(&failure_text);
        request.entry.failure_text = Some(match request.entry.failure_text.take() {
            Some(existing) => format!("{existing}; {failure_text}"),
            None => failure_text,
        });
        if let Some(timing) = request.entry.timing.as_mut() {
            timing.response_end = Some(timestamp);
        }
        self.finish_inflight(request_id, &request);
        self.completed.push(request.entry.clone());
        Some(request.entry)
    }

    fn finish_redirect(&mut self, request_id: &str, response: ResponseReceived) {
        self.record_response(response.clone());
        let _ = self.finish(request_id, response.timestamp, response.encoded_data_length);
    }

    fn finish_inflight(&mut self, request_id: &str, request: &TrackedRequest) {
        if request.counted {
            if let Some(frame_id) = request.entry.frame_id.as_deref() {
                self.idle
                    .request_finished(frame_id, request_id, self.now_ms());
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct NetworkMonitorHandle {
    state: Mutex<NetworkMonitor>,
    changed: Condvar,
}

impl NetworkMonitorHandle {
    pub fn attach_frame(&self, frame_id: String, parent_id: Option<String>) {
        let mut state = self.state.lock().unwrap();
        let now_ms = state.now_ms();
        state.idle.attach(frame_id, parent_id, now_ms);
        self.changed.notify_all();
    }

    pub fn detach_frame(&self, frame_id: &str) {
        self.state.lock().unwrap().idle.detach(frame_id);
        self.changed.notify_all();
    }

    pub fn lifecycle(&self, frame_id: &str, name: &str) {
        let mut state = self.state.lock().unwrap();
        let now_ms = state.now_ms();
        state.idle.lifecycle(frame_id, name, now_ms);
        self.changed.notify_all();
    }

    pub fn request_started(&self, event: RequestStarted) {
        self.state.lock().unwrap().record_request(event);
        self.changed.notify_all();
    }

    pub fn request_extra(&self, request_id: String, headers: BTreeMap<String, String>) {
        self.state
            .lock()
            .unwrap()
            .record_request_extra(request_id, headers);
        self.changed.notify_all();
    }

    pub fn response_received(&self, response: ResponseReceived) {
        self.state.lock().unwrap().record_response(response);
        self.changed.notify_all();
    }

    pub fn response_extra(&self, request_id: String, headers: BTreeMap<String, String>) {
        self.state
            .lock()
            .unwrap()
            .record_response_extra(request_id, headers);
        self.changed.notify_all();
    }

    pub fn loading_finished(
        &self,
        request_id: &str,
        timestamp: f64,
        length: Option<u64>,
    ) -> Option<NetworkEntry> {
        let entry = self
            .state
            .lock()
            .unwrap()
            .finish(request_id, timestamp, length);
        self.changed.notify_all();
        entry
    }

    pub fn loading_failed(
        &self,
        request_id: &str,
        timestamp: f64,
        failure_text: String,
    ) -> Option<NetworkEntry> {
        let entry = self
            .state
            .lock()
            .unwrap()
            .fail(request_id, timestamp, failure_text);
        self.changed.notify_all();
        entry
    }

    pub fn drain_completed(&self) -> Vec<NetworkEntry> {
        std::mem::take(&mut self.state.lock().unwrap().completed)
    }

    pub fn request_cursor(&self) -> u64 {
        self.state.lock().unwrap().request_sequence
    }

    pub fn response_cursor(&self) -> u64 {
        self.state.lock().unwrap().response_sequence
    }

    pub fn wait_for_request(
        &self,
        matcher: &UrlMatcher,
        after: u64,
        timeout: Duration,
    ) -> Result<NetworkEntry, String> {
        self.wait_for_entry(matcher, after, timeout, false)
    }

    pub fn wait_for_response(
        &self,
        matcher: &UrlMatcher,
        after: u64,
        timeout: Duration,
    ) -> Result<NetworkEntry, String> {
        self.wait_for_entry(matcher, after, timeout, true)
    }

    fn wait_for_entry(
        &self,
        matcher: &UrlMatcher,
        after: u64,
        timeout: Duration,
        response: bool,
    ) -> Result<NetworkEntry, String> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap();
        loop {
            let entries = if response {
                &state.responses
            } else {
                &state.requests
            };
            if let Some(found) = entries
                .iter()
                .find(|event| event.sequence > after && matcher.is_match(&event.matching_url))
            {
                return Ok(found.entry.clone());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("Timed out after {}ms", timeout.as_millis()));
            }
            let (next, wait) = self.changed.wait_timeout(state, remaining).unwrap();
            state = next;
            if wait.timed_out() {
                return Err(format!("Timed out after {}ms", timeout.as_millis()));
            }
        }
    }

    pub fn wait_for_load_state(
        &self,
        load_state: NetworkLoadState,
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap();
        loop {
            let now_ms = state.now_ms();
            if state.idle.load_state(load_state, now_ms) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("Timed out after {}ms", timeout.as_millis()));
            }
            let wake = remaining.min(Duration::from_millis(NETWORK_IDLE_MS));
            let (next, _) = self.changed.wait_timeout(state, wake).unwrap();
            state = next;
        }
    }
}

#[derive(Clone, Debug)]
pub struct UrlMatcher {
    regex: Regex,
}

impl UrlMatcher {
    pub fn exact(value: &str) -> Result<Self, String> {
        Self::regex(&format!("^(?:{})$", regex::escape(value)), "")
    }

    pub fn glob(value: &str) -> Result<Self, String> {
        Self::regex(&glob_to_regex(value)?, "")
    }

    pub fn regex(source: &str, flags: &str) -> Result<Self, String> {
        let unsupported = flags
            .chars()
            .filter(|flag| !matches!(flag, 'i' | 'm' | 's' | 'u'))
            .collect::<String>();
        if !unsupported.is_empty() {
            return Err(format!("Unsupported regex flags: {unsupported}"));
        }
        RegexBuilder::new(source)
            .case_insensitive(flags.contains('i'))
            .multi_line(flags.contains('m'))
            .dot_matches_new_line(flags.contains('s'))
            .unicode(true)
            .build()
            .map(|regex| Self { regex })
            .map_err(|error| format!("Invalid URL regex: {error}"))
    }

    pub fn text(value: &str) -> Result<Self, String> {
        if value.contains('*') || value.contains('{') {
            Self::glob(value)
        } else {
            Self::exact(value)
        }
    }

    pub fn is_match(&self, url: &str) -> bool {
        self.regex.is_match(url)
    }
}

fn glob_to_regex(glob: &str) -> Result<String, String> {
    let mut output = String::from("^(?:");
    let chars = glob.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' if chars.get(index + 1) == Some(&'*') => {
                output.push_str(".*");
                index += 2;
            }
            '*' => {
                output.push_str("[^/]*");
                index += 1;
            }
            '{' => {
                let Some(relative_end) = chars[index + 1..].iter().position(|ch| *ch == '}') else {
                    return Err("Unclosed URL glob alternation".to_string());
                };
                let end = index + 1 + relative_end;
                let alternatives = chars[index + 1..end]
                    .iter()
                    .collect::<String>()
                    .split(',')
                    .map(regex::escape)
                    .collect::<Vec<_>>();
                output.push_str("(?:");
                output.push_str(&alternatives.join("|"));
                output.push(')');
                index = end + 1;
            }
            ch => {
                output.push_str(&regex::escape(&ch.to_string()));
                index += 1;
            }
        }
    }
    output.push_str(")$");
    Ok(output)
}

fn counts_for_network_idle(url: &str, resource_type: &str) -> bool {
    let lower_type = resource_type.to_ascii_lowercase();
    if matches!(lower_type.as_str(), "eventsource" | "websocket") {
        return false;
    }
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    !without_query.to_ascii_lowercase().ends_with("/favicon.ico")
}

pub(crate) fn mask_headers(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .into_iter()
        .map(|(name, value)| {
            let lower = name.to_ascii_lowercase();
            let value = if matches!(
                lower.as_str(),
                "authorization" | "proxy-authorization" | "cookie" | "set-cookie" | "x-api-key"
            ) {
                "[REDACTED]".to_string()
            } else {
                mask_text(&value)
            };
            (name, value)
        })
        .collect()
}

pub(crate) fn mask_text(value: &str) -> String {
    let redacted = redact_sensitive(value);
    let expression =
        Regex::new(r"(?i)(password|passwd|token|api[_-]?key|authorization)(=|%3[dD])([^&\s]+)")
            .unwrap();
    expression
        .replace_all(&redacted, "$1$2[REDACTED]")
        .into_owned()
}

pub fn summarize_network_entry(entry: &NetworkEntry) -> String {
    let status = entry
        .status
        .map(|status| status.to_string())
        .unwrap_or_else(|| "-".to_string());
    let bytes = entry
        .transfer_size
        .or(entry.encoded_data_length)
        .map(|bytes| format!("{bytes}b"))
        .unwrap_or_else(|| "-".to_string());
    let elapsed = entry
        .timing
        .as_ref()
        .and_then(|timing| {
            timing
                .response_end
                .map(|end| (end - timing.start_time) * 1_000.0)
        })
        .filter(|elapsed| elapsed.is_finite() && *elapsed >= 0.0)
        .map(|elapsed| format!("{}ms", elapsed.round()))
        .unwrap_or_else(|| "-".to_string());
    let mut line = format!(
        "{} {} {} {} {}",
        entry.method, entry.url, status, bytes, elapsed
    );
    if let Some(failure) = entry.failure_text.as_deref() {
        line.push(' ');
        line.push_str(failure);
    }
    line
}

pub fn apply_network_report_mode(report: &mut ExecutionReport, mode: NetworkReportMode) {
    match mode {
        NetworkReportMode::Full => {}
        NetworkReportMode::Summary => {
            report.network_summary = report
                .network
                .iter()
                .map(summarize_network_entry)
                .collect::<Vec<_>>();
            report.network.clear();
        }
        NetworkReportMode::None => {
            report.network.clear();
            report.network_summary.clear();
        }
    }
}

fn push_recent(events: &mut VecDeque<SequencedEntry>, entry: SequencedEntry) {
    events.push_back(entry);
    while events.len() > RECENT_EVENT_CAP {
        events.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str, frame_id: &str, url: &str, resource_type: &str) -> RequestStarted {
        RequestStarted {
            request_id: id.to_string(),
            loader_id: id.to_string(),
            frame_id: Some(frame_id.to_string()),
            method: "GET".to_string(),
            url: url.to_string(),
            resource_type: resource_type.to_string(),
            headers: BTreeMap::new(),
            timestamp: 10.0,
            redirect_response: None,
        }
    }

    fn response(id: &str, status: u16, url_headers: BTreeMap<String, String>) -> ResponseReceived {
        ResponseReceived {
            request_id: id.to_string(),
            status,
            status_text: "Found".to_string(),
            headers: url_headers,
            timestamp: 11.0,
            encoded_data_length: Some(20),
            from_service_worker: false,
            request_start: Some(10.1),
            response_start: Some(10.5),
        }
    }

    #[test]
    fn glob_matches_whole_url_with_star_double_star_and_alternation() {
        let cases = [
            ("https://example.com/*", "https://example.com/a", true),
            ("https://example.com/*", "https://example.com/a/b", false),
            ("https://example.com/**", "https://example.com/a/b", true),
            (
                "https://example.com/{one,two}",
                "https://example.com/two",
                true,
            ),
            (
                "https://example.com/{one,two}",
                "https://example.com/three",
                false,
            ),
            ("*example.com*", "https://example.com/", false),
        ];
        for (pattern, url, expected) in cases {
            assert_eq!(UrlMatcher::glob(pattern).unwrap().is_match(url), expected);
        }
        assert!(UrlMatcher::exact("https://x.test/a")
            .unwrap()
            .is_match("https://x.test/a"));
        assert!(!UrlMatcher::exact("https://x.test/a")
            .unwrap()
            .is_match("https://x.test/ab"));
        assert!(UrlMatcher::regex(r"/items/\d+$", "")
            .unwrap()
            .is_match("https://x/items/42"));
    }

    #[test]
    fn request_and_response_waits_use_raw_urls_after_the_armed_cursor() {
        let monitor = NetworkMonitorHandle::default();
        monitor.attach_frame("main".to_string(), None);
        let request_cursor = monitor.request_cursor();
        let response_cursor = monitor.response_cursor();
        monitor.request_started(request(
            "fetch",
            "main",
            "https://x.test/api/items?token=secret",
            "Fetch",
        ));
        monitor.response_received(response("fetch", 200, BTreeMap::new()));

        let request = monitor
            .wait_for_request(
                &UrlMatcher::glob("https://x.test/**?token=*").unwrap(),
                request_cursor,
                Duration::ZERO,
            )
            .unwrap();
        let response = monitor
            .wait_for_response(
                &UrlMatcher::regex(r"/api/items\?token=secret$", "").unwrap(),
                response_cursor,
                Duration::ZERO,
            )
            .unwrap();

        assert!(!request.url.contains("secret"));
        assert_eq!(response.status, Some(200));
    }

    #[test]
    fn network_idle_requires_every_frame_subtree_for_five_hundred_ms() {
        let mut idle = NetworkIdleTracker::default();
        idle.attach("main".to_string(), None, 0);
        idle.attach("child".to_string(), Some("main".to_string()), 0);
        assert!(!idle.load_state(NetworkLoadState::Networkidle, 499));
        assert!(idle.load_state(NetworkLoadState::Networkidle, 500));

        idle.request_started("child", "slow", 600);
        assert!(!idle.load_state(NetworkLoadState::Networkidle, 1_500));
        idle.request_finished("child", "slow", 1_500);
        assert!(!idle.load_state(NetworkLoadState::Networkidle, 1_999));
        assert!(idle.load_state(NetworkLoadState::Networkidle, 2_000));
    }

    #[test]
    fn network_idle_excludes_favicon_eventsource_and_websocket() {
        for (url, resource_type) in [
            ("https://x.test/favicon.ico?v=1", "Image"),
            ("https://x.test/events", "EventSource"),
            ("wss://x.test/socket", "WebSocket"),
        ] {
            assert!(!counts_for_network_idle(url, resource_type));
        }
        assert!(counts_for_network_idle("https://x.test/api", "Fetch"));
    }

    #[test]
    fn redirect_chain_finishes_previous_request_and_links_next() {
        let mut monitor = NetworkMonitor::default();
        monitor.record_request(request("nav", "main", "https://x.test/start", "Document"));
        let mut redirected = request("nav", "main", "https://x.test/end", "Document");
        redirected.redirect_response = Some(response("nav", 302, BTreeMap::new()));
        monitor.record_request(redirected);
        monitor.record_response(response("nav", 200, BTreeMap::new()));
        monitor.finish("nav", 12.0, Some(50));

        assert_eq!(monitor.completed.len(), 2);
        assert_eq!(monitor.completed[0].status, Some(302));
        assert_eq!(
            monitor.completed[1].redirect_from.as_deref(),
            Some("https://x.test/start")
        );
        assert_eq!(monitor.completed[1].status, Some(200));
        assert_eq!(monitor.completed[1].transfer_size, Some(50));
    }

    fn reported_entries() -> Vec<NetworkEntry> {
        let mut monitor = NetworkMonitor::default();
        let mut started = request("api", "main", "https://x.test/api/items", "Fetch");
        started
            .headers
            .insert("Authorization".to_string(), "Bearer abcdefgh".to_string());
        monitor.record_request(started);
        monitor.record_response(response(
            "api",
            200,
            BTreeMap::from([(
                "content-security-policy".to_string(),
                "default-src 'self'".to_string(),
            )]),
        ));
        monitor.finish("api", 10.545, Some(1_234));
        monitor.record_request(request(
            "blocked",
            "main",
            "https://x.test/blocked",
            "Image",
        ));
        monitor.fail("blocked", 10.2, "net::ERR_BLOCKED_BY_CLIENT".to_string());
        monitor.completed
    }

    fn report_with_network(network: Vec<NetworkEntry>) -> ExecutionReport {
        serde_json::from_value(serde_json::json!({
            "ok": true,
            "steps": [{
                "step_index": 0,
                "ok": true,
                "summary": "Waited for response",
                "data": {
                    "url": "https://x.test/api/items",
                    "status": 200,
                    "response_headers": {"content-security-policy": "default-src 'self'"}
                }
            }],
            "dialogs": [],
            "new_tabs": [],
            "network": network,
            "intercepted_requests": [{
                "url": "https://x.test/blocked",
                "method": "GET",
                "pattern": "har-replay",
                "action": "abort",
                "reason": "blockedbyclient"
            }]
        }))
        .unwrap()
    }

    #[test]
    fn summary_mode_renders_one_line_per_request_without_headers() {
        let mut report = report_with_network(reported_entries());
        apply_network_report_mode(&mut report, NetworkReportMode::Summary);

        assert!(report.network.is_empty());
        assert_eq!(
            report.network_summary,
            vec![
                "GET https://x.test/api/items 200 1234b 545ms".to_string(),
                "GET https://x.test/blocked - - 200ms net::ERR_BLOCKED_BY_CLIENT".to_string(),
            ]
        );
        let payload = serde_json::to_string(&report.network_summary).unwrap();
        assert!(!payload.contains("content-security-policy"));
        assert!(!payload.contains("Authorization"));
    }

    #[test]
    fn none_mode_drops_entries_but_keeps_interception_and_step_detail() {
        let mut report = report_with_network(reported_entries());
        apply_network_report_mode(&mut report, NetworkReportMode::None);

        assert!(report.network.is_empty());
        assert!(report.network_summary.is_empty());
        assert_eq!(report.intercepted_requests.len(), 1);
        assert_eq!(report.intercepted_requests[0].action, "abort");
        assert_eq!(
            report.intercepted_requests[0].reason.as_deref(),
            Some("blockedbyclient")
        );
        assert_eq!(report.steps[0].data.as_ref().unwrap()["status"], 200);
    }

    #[test]
    fn full_mode_leaves_the_baseline_payload_untouched() {
        let baseline = report_with_network(reported_entries());
        let mut report = report_with_network(reported_entries());
        apply_network_report_mode(&mut report, NetworkReportMode::Full);

        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::to_value(&baseline).unwrap()
        );
        assert!(report.network_summary.is_empty());
        assert_eq!(report.network.len(), 2);
        assert!(report.network[0]
            .request_headers
            .contains_key("Authorization"));
        assert!(report.network[0]
            .response_headers
            .contains_key("content-security-policy"));
    }

    #[test]
    fn masks_sensitive_headers_urls_and_failures() {
        let mut monitor = NetworkMonitor::default();
        let mut started = request(
            "secret",
            "main",
            "https://x.test/?password=hunter2&safe=yes",
            "Fetch",
        );
        started
            .headers
            .insert("Authorization".to_string(), "Bearer abcdefgh".to_string());
        monitor.record_request(started);
        monitor.fail("secret", 12.0, "token=topsecret".to_string());

        let entry = &monitor.completed[0];
        assert!(!entry.url.contains("hunter2"));
        assert!(entry.url.contains("[REDACTED]"));
        assert_eq!(entry.request_headers["Authorization"], "[REDACTED]");
        assert_eq!(entry.failure_text.as_deref(), Some("token=[REDACTED]"));
    }
}
