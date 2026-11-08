use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Condvar, Mutex, OnceLock};
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
    base_request_headers: BTreeMap<String, String>,
    base_response_headers: BTreeMap<String, String>,
    response_extra_seen: bool,
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

    fn inflight_request_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .frames
            .values()
            .flat_map(|frame| frame.inflight.iter().cloned())
            .collect();
        ids.sort();
        ids.dedup();
        ids
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

#[derive(Clone, Debug)]
struct DiscoveryEntry {
    cdp_id: String,
    entry: NetworkEntry,
    // Base events identify the hop; ExtraInfo does not. Keep these to undo a
    // speculative merge if a later redirect reveals that the ID is shared.
    base_request_headers: BTreeMap<String, String>,
    base_response_headers: BTreeMap<String, String>,
    response_extra_seen: bool,
}

#[derive(Debug)]
struct NetworkMonitor {
    started_at: Instant,
    namespace: String,
    discovery: VecDeque<DiscoveryEntry>,
    active: HashMap<String, TrackedRequest>,
    request_extra: HashMap<String, BTreeMap<String, String>>,
    response_extra: HashMap<String, BTreeMap<String, String>>,
    response_extra_order: VecDeque<String>,
    request_extra_order: VecDeque<String>,
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
            namespace: uuid::Uuid::new_v4().to_string(),
            discovery: VecDeque::new(),
            active: HashMap::new(),
            request_extra: HashMap::new(),
            response_extra: HashMap::new(),
            response_extra_order: VecDeque::new(),
            request_extra_order: VecDeque::new(),
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
        // Fence the active previous hop before finish removes it, even if its
        // discovery record has already aged out. No extra history map is needed.
        if event.redirect_response.is_some() {
            self.mark_header_ambiguity(&event.request_id);
        }
        let redirect_from = if let Some(redirect) = event.redirect_response.as_ref() {
            self.finish_redirect(&event.request_id, redirect.clone());
            self.completed.last().map(|entry| entry.url.clone())
        } else {
            None
        };
        let ambiguous = event.redirect_response.is_some()
            || self
                .discovery
                .iter()
                .any(|item| item.cdp_id == event.request_id);
        if ambiguous {
            self.mark_header_ambiguity(&event.request_id);
        }
        let base_request_headers = mask_headers(event.headers.clone());
        self.request_extra_order
            .retain(|id| id != &event.request_id);
        let request_headers = self
            .request_extra
            .remove(&event.request_id)
            .unwrap_or(event.headers);
        let is_navigation_request = event
            .request_id
            .rsplit_once('\0')
            .map_or(event.request_id.as_str(), |(_, id)| id)
            == event.loader_id
            && event.resource_type.eq_ignore_ascii_case("document");
        let document_id = is_navigation_request.then(|| event.loader_id.clone());
        let entry = NetworkEntry {
            request_id: Some(format!(
                "net_{}_{}",
                self.namespace,
                self.request_sequence + 1
            )),
            timestamp: event.timestamp,
            method: event.method,
            url: mask_text(&event.url),
            resource_type: event.resource_type,
            status: None,
            status_text: None,
            request_headers: if ambiguous {
                base_request_headers.clone()
            } else {
                mask_headers(request_headers)
            },
            header_attribution: ambiguous.then(|| "redirect_extra_info_unverified".into()),
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
        self.discovery.push_back(DiscoveryEntry {
            cdp_id: event.request_id.clone(),
            entry: entry.clone(),
            base_request_headers: base_request_headers.clone(),
            base_response_headers: BTreeMap::new(),
            response_extra_seen: false,
        });
        if self.discovery.len() > RECENT_EVENT_CAP {
            self.discovery.pop_front();
        }
        self.active.insert(
            event.request_id,
            TrackedRequest {
                entry,
                matching_url,
                counted,
                base_request_headers,
                base_response_headers: BTreeMap::new(),
                response_extra_seen: false,
            },
        );
    }

    fn record_request_extra(&mut self, request_id: String, headers: BTreeMap<String, String>) {
        self.record_extra(request_id, headers, false);
    }

    fn record_extra(
        &mut self,
        request_id: String,
        headers: BTreeMap<String, String>,
        response: bool,
    ) {
        let headers = mask_headers(headers);
        if let Some(item) = self
            .discovery
            .iter_mut()
            .rev()
            .find(|item| item.cdp_id == request_id)
        {
            if item.entry.header_attribution.is_some() {
                return; // No hop ordinal/hasExtraInfo is supplied by our event API.
            }
            if response {
                item.entry.response_headers = headers.clone();
                item.response_extra_seen = true;
            } else {
                item.entry.request_headers = headers.clone();
            }
            let id = item.entry.request_id.clone();
            if let Some(active) = self.active.get_mut(&request_id) {
                if active.entry.request_id == id {
                    if response {
                        active.response_extra_seen = true;
                        active.entry.response_headers = headers.clone();
                    } else {
                        active.entry.request_headers = headers.clone();
                    }
                }
            }
            for entry in self
                .completed
                .iter_mut()
                .chain(self.requests.iter_mut().map(|event| &mut event.entry))
                .chain(self.responses.iter_mut().map(|event| &mut event.entry))
            {
                if entry.request_id == id {
                    if response {
                        entry.response_headers = headers.clone();
                    } else {
                        entry.request_headers = headers.clone();
                    }
                }
            }
            return;
        }
        // Discovery is a bounded index, not the lifetime of an in-flight
        // request. Preserve report behavior for a slow request evicted from it.
        if let Some(active) = self.active.get_mut(&request_id) {
            if active.entry.header_attribution.is_some() {
                return;
            }
            if response {
                active.entry.response_headers = headers.clone();
                active.response_extra_seen = true;
            } else {
                active.entry.request_headers = headers.clone();
            }
            let id = active.entry.request_id.clone();
            for event in self.requests.iter_mut().chain(self.responses.iter_mut()) {
                if event.entry.request_id == id {
                    if response {
                        event.entry.response_headers = headers.clone();
                    } else {
                        event.entry.request_headers = headers.clone();
                    }
                }
            }
            return;
        }
        let (map, order) = if response {
            (&mut self.response_extra, &mut self.response_extra_order)
        } else {
            (&mut self.request_extra, &mut self.request_extra_order)
        };
        if map.insert(request_id.clone(), headers).is_none() {
            order.push_back(request_id);
        }
        while order.len() > RECENT_EVENT_CAP {
            if let Some(stale) = order.pop_front() {
                map.remove(&stale);
            }
        }
    }

    fn mark_header_ambiguity(&mut self, request_id: &str) {
        if let Some(active) = self.active.get_mut(request_id) {
            active.entry.request_headers = active.base_request_headers.clone();
            active.entry.response_headers = active.base_response_headers.clone();
            active.entry.header_attribution = Some("redirect_extra_info_unverified".into());
            let id = active.entry.request_id.clone();
            for entry in self
                .completed
                .iter_mut()
                .chain(self.requests.iter_mut().map(|event| &mut event.entry))
                .chain(self.responses.iter_mut().map(|event| &mut event.entry))
            {
                if entry.request_id == id {
                    entry.request_headers = active.entry.request_headers.clone();
                    entry.response_headers = active.entry.response_headers.clone();
                    entry.header_attribution = active.entry.header_attribution.clone();
                }
            }
        }
        self.request_extra.remove(request_id);
        self.response_extra.remove(request_id);
        self.request_extra_order.retain(|id| id != request_id);
        self.response_extra_order.retain(|id| id != request_id);
        for item in self
            .discovery
            .iter_mut()
            .filter(|item| item.cdp_id == request_id)
        {
            item.entry.request_headers = item.base_request_headers.clone();
            item.entry.response_headers = item.base_response_headers.clone();
            item.entry.header_attribution = Some("redirect_extra_info_unverified".into());
            let id = &item.entry.request_id;
            for entry in self
                .completed
                .iter_mut()
                .chain(self.requests.iter_mut().map(|event| &mut event.entry))
                .chain(self.responses.iter_mut().map(|event| &mut event.entry))
                .chain(self.active.values_mut().map(|active| &mut active.entry))
            {
                if &entry.request_id == id {
                    entry.request_headers = item.entry.request_headers.clone();
                    entry.response_headers = item.entry.response_headers.clone();
                    entry.header_attribution = item.entry.header_attribution.clone();
                }
            }
        }
    }

    fn record_response(&mut self, response: ResponseReceived) {
        let Some(request) = self.active.get_mut(&response.request_id) else {
            return;
        };
        request.base_response_headers = mask_headers(response.headers.clone());
        request.entry.status = Some(response.status);
        request.entry.status_text = Some(mask_text(&response.status_text));
        if response.status >= 400 {
            request.entry.failure_text = Some(mask_text(&format!(
                "HTTP {} {}",
                response.status, response.status_text
            )));
        }
        if let Some(item) = self
            .discovery
            .iter_mut()
            .find(|item| item.entry.request_id == request.entry.request_id)
        {
            item.base_response_headers = mask_headers(response.headers.clone());
        }
        self.response_extra_order
            .retain(|id| id != &response.request_id);
        let active_extra = (request.response_extra_seen
            && request.entry.header_attribution.is_none())
        .then(|| request.entry.response_headers.clone());
        let attached_extra = self
            .discovery
            .iter()
            .find(|item| item.entry.request_id == request.entry.request_id)
            .filter(|item| item.response_extra_seen && item.entry.header_attribution.is_none())
            .map(|item| item.entry.response_headers.clone());
        request.entry.response_headers = mask_headers(
            active_extra
                .or(attached_extra)
                .or_else(|| self.response_extra.remove(&response.request_id))
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
        self.record_extra(request_id, headers, true);
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
        self.update_discovery(&request.entry, true);
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
        self.update_discovery(&request.entry, true);
        self.completed.push(request.entry.clone());
        Some(request.entry)
    }

    fn finish_redirect(&mut self, request_id: &str, response: ResponseReceived) {
        self.record_response(response.clone());
        let _ = self.finish(request_id, response.timestamp, response.encoded_data_length);
    }

    fn update_discovery(&mut self, entry: &NetworkEntry, finished: bool) {
        if let Some(item) = self
            .discovery
            .iter_mut()
            .find(|item| item.entry.request_id == entry.request_id)
        {
            item.entry = entry.clone();
            let _ = finished;
        }
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
    /// Return at most the latest 1000 requests, without changing wait/report cursors.
    pub fn network_requests(&self, limit: usize) -> Vec<NetworkEntry> {
        let state = self.state.lock().unwrap();
        state
            .discovery
            .iter()
            .rev()
            .take(limit.min(RECENT_EVENT_CAP))
            .map(|item| {
                state
                    .active
                    .get(&item.cdp_id)
                    .filter(|active| active.entry.request_id == item.entry.request_id)
                    .map(|active| active.entry.clone())
                    .unwrap_or_else(|| item.entry.clone())
            })
            .collect()
    }

    pub fn network_requests_for_tab(&self, tab_id: &str, limit: usize) -> Vec<NetworkEntry> {
        let state = self.state.lock().unwrap();
        state
            .discovery
            .iter()
            .rev()
            .filter(|item| {
                item.cdp_id
                    .split_once('\0')
                    .is_some_and(|(target, _)| target == tab_id)
            })
            .take(limit.min(RECENT_EVENT_CAP))
            .map(|item| {
                state
                    .active
                    .get(&item.cdp_id)
                    .filter(|active| active.entry.request_id == item.entry.request_id)
                    .map(|active| active.entry.clone())
                    .unwrap_or_else(|| item.entry.clone())
            })
            .collect()
    }

    pub fn network_request_for_tab(&self, tab_id: &str, id: &str) -> Result<NetworkEntry, String> {
        let state = self.state.lock().unwrap();
        let item = state
            .discovery
            .iter()
            .find(|item| {
                item.entry.request_id.as_deref() == Some(id)
                    && item
                        .cdp_id
                        .split_once('\0')
                        .is_some_and(|(target, _)| target == tab_id)
            })
            .ok_or_else(|| {
                "Unknown, stale, evicted, or cross-tab network request ID".to_string()
            })?;
        Ok(state
            .active
            .get(&item.cdp_id)
            .filter(|active| active.entry.request_id == item.entry.request_id)
            .map(|active| active.entry.clone())
            .unwrap_or_else(|| item.entry.clone()))
    }

    pub fn network_request(&self, id: &str) -> Result<NetworkEntry, String> {
        let state = self.state.lock().unwrap();
        let item = state
            .discovery
            .iter()
            .find(|item| item.entry.request_id.as_deref() == Some(id))
            .ok_or_else(|| "Unknown, stale, or evicted network request ID".to_string())?;
        Ok(state
            .active
            .get(&item.cdp_id)
            .filter(|active| active.entry.request_id == item.entry.request_id)
            .map(|active| active.entry.clone())
            .unwrap_or_else(|| item.entry.clone()))
    }

    /// Resolve only monitor-issued IDs against the originating CDP target.
    /// CDP is not streaming: a dishonest content-length can still cause a larger transient allocation.
    pub fn network_response_body(
        &self,
        tab: &headless_chrome::Tab,
        id: &str,
    ) -> Result<String, String> {
        use headless_chrome::protocol::cdp::Network;
        let cdp_id = self.response_body_request_id(tab.get_target_id(), id)?;
        let response = tab
            .call_method(Network::GetResponseBody {
                request_id: cdp_id,
            })
            .map_err(|_| "Response body unavailable from browser".to_string())?;
        if response.base_64_encoded || response.body.len() > 65536 {
            return Err("Binary or oversized response body unavailable".into());
        }
        let mut json: serde_json::Value = serde_json::from_str(&response.body)
            .map_err(|_| "Invalid JSON response body".to_string())?;
        redact_json(&mut json);
        let output = serde_json::to_string(&json)
            .map_err(|_| "Cannot serialize response body".to_string())?;
        if output.len() > 65536 {
            return Err("Redacted response body exceeds 65536 bytes".into());
        }
        Ok(output)
    }

    // Validation returns owned data so the monitor lock ends before transport or parsing.
    fn response_body_request_id(&self, tab_id: &str, id: &str) -> Result<String, String> {
        let state = self.state.lock().unwrap();
        let item = state
            .discovery
            .iter()
            .find(|item| item.entry.request_id.as_deref() == Some(id))
            .ok_or_else(|| "Unknown, stale, or evicted network request ID".to_string())?;
        if state
            .discovery
            .iter()
            .rev()
            .find(|candidate| candidate.cdp_id == item.cdp_id)
            .is_none_or(|candidate| candidate.entry.request_id != item.entry.request_id)
        {
            return Err("Response body unavailable for an earlier request hop".into());
        }
        if item.entry.header_attribution.is_some() {
            return Err("Response body unavailable: redirect header attribution unknown".into());
        }
        let (target, cdp_id) = item
            .cdp_id
            .split_once('\0')
            .ok_or("Request source tab unavailable")?;
        if target != tab_id {
            return Err("Network request belongs to another tab".into());
        }
        if item
            .entry
            .timing
            .as_ref()
            .and_then(|timing| timing.response_end)
            .is_none()
        {
            return Err("Response body unavailable for pending request".into());
        }
        if item
            .entry
            .status
            .is_none_or(|status| (300..400).contains(&status))
        {
            return Err("Response body unavailable for redirect or failed request".into());
        }
        let header = |name: &str| {
            item.entry
                .response_headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        };
        let length = header("content-length")
            .and_then(|v| v.parse::<usize>().ok())
            .ok_or("Response body size unknown")?;
        if length > 65536 {
            return Err("Response body exceeds 65536 bytes".into());
        }
        if header("content-encoding").is_some_and(|v| !v.eq_ignore_ascii_case("identity")) {
            return Err("Encoded response body unavailable".into());
        }
        let mime = header("content-type").unwrap_or("").to_ascii_lowercase();
        if !mime.contains("json") {
            return Err("Only JSON response bodies are available for safe redaction".into());
        }
        Ok(cdp_id.to_string())
    }

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
        filters: &NetworkWaitFilters,
        after: u64,
        timeout: Duration,
    ) -> Result<NetworkEntry, String> {
        self.wait_for_entry(matcher, filters, after, timeout, false)
    }

    pub fn wait_for_response(
        &self,
        matcher: &UrlMatcher,
        filters: &NetworkWaitFilters,
        after: u64,
        timeout: Duration,
    ) -> Result<NetworkEntry, String> {
        self.wait_for_entry(matcher, filters, after, timeout, true)
    }

    fn wait_for_entry(
        &self,
        matcher: &UrlMatcher,
        filters: &NetworkWaitFilters,
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
            if let Some(found) = entries.iter().find(|event| {
                event.sequence > after
                    && matcher.is_match(&event.matching_url)
                    && filters.matches(&event.entry)
            }) {
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
                return Err(load_state_timeout_message(&state, load_state, timeout));
            }
            let wake = remaining.min(Duration::from_millis(NETWORK_IDLE_MS));
            let (next, _) = self.changed.wait_timeout(state, wake).unwrap();
            state = next;
        }
    }
}

const LOAD_STATE_TIMEOUT_URL_SAMPLE: usize = 5;

fn load_state_timeout_message(
    state: &NetworkMonitor,
    load_state: NetworkLoadState,
    timeout: Duration,
) -> String {
    let mut message = format!("Timed out after {}ms", timeout.as_millis());
    if load_state != NetworkLoadState::Networkidle {
        return message;
    }
    let inflight = state.idle.inflight_request_ids();
    let sample: Vec<String> = inflight
        .iter()
        .filter_map(|request_id| state.active.get(request_id))
        .take(LOAD_STATE_TIMEOUT_URL_SAMPLE)
        .map(|request| {
            format!(
                "{} {}",
                request.entry.method,
                summarize_network_url(&request.entry.url)
            )
        })
        .collect();
    if inflight.is_empty() {
        message.push_str(
            "; the network went quiet but never stayed idle for 500ms — pages that poll or keep sockets open never reach networkidle",
        );
    } else {
        message.push_str(&format!(
            "; {} request(s) still in flight: {}{}",
            inflight.len(),
            sample.join(", "),
            if inflight.len() > sample.len() {
                ", …"
            } else {
                ""
            }
        ));
    }
    message.push_str(
        "; use wait_for_response, wait_for_function, wait_for_text or wait_for_selector instead of networkidle on live apps",
    );
    message
}

#[derive(Clone, Debug, Default)]
pub struct NetworkWaitFilters {
    pub method: Option<String>,
    pub status: Option<u16>,
}

impl NetworkWaitFilters {
    pub fn matches(&self, entry: &NetworkEntry) -> bool {
        let method_matches = self
            .method
            .as_ref()
            .is_none_or(|method| entry.method.eq_ignore_ascii_case(method.trim()));
        let status_matches = self
            .status
            .is_none_or(|status| entry.status == Some(status));
        method_matches && status_matches
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
        let flags = flags
            .chars()
            .filter(|flag| !matches!(flag, 'g' | 'y'))
            .collect::<String>();
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
            .map(|regex| Self {
                regex,
            })
            .map_err(|error| format!("Invalid URL regex: {error}"))
    }

    pub fn text(value: &str) -> Result<Self, String> {
        if value.contains('*') || value.contains('{') {
            Self::glob(value)
        } else {
            Self::exact(value)
        }
    }

    pub fn substring(value: &str) -> Result<Self, String> {
        Self::regex(&regex::escape(value), "")
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
                "authorization"
                    | "proxy-authorization"
                    | "cookie"
                    | "set-cookie"
                    | "x-api-key"
                    | "x-auth-token"
                    | "x-api-token"
                    | "x-csrf-token"
                    | "x-xsrf-token"
                    | "csrf-token"
                    | "xsrf-token"
            ) {
                "[REDACTED]".to_string()
            } else {
                mask_text(&value)
            };
            (name, value)
        })
        .collect()
}

fn redact_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                let key = key.to_ascii_lowercase();
                if [
                    "password",
                    "passwd",
                    "token",
                    "secret",
                    "credential",
                    "authorization",
                    "cookie",
                    "api_key",
                    "api-key",
                    "apikey",
                    "signature",
                    "private_key",
                    "private-key",
                    "privatekey",
                ]
                .iter()
                .any(|part| key.contains(part))
                {
                    *value = serde_json::Value::String("[REDACTED]".into());
                } else {
                    redact_json(value);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json(item);
            }
        }
        serde_json::Value::String(text) => *text = mask_text(text),
        _ => {}
    }
}

pub fn mask_text(value: &str) -> String {
    // Recognizable private-key envelopes are secrets even under an innocuous JSON key.
    let upper = value.to_ascii_uppercase();
    if upper.contains("-----BEGIN ") && upper.contains("PRIVATE KEY-----") {
        return "[REDACTED]".into();
    }
    let redacted = redact_sensitive(value);
    static EXPRESSION: OnceLock<Regex> = OnceLock::new();
    let expression = EXPRESSION.get_or_init(|| {
        Regex::new(r"(?i)(password|passwd|token|api[_-]?key|authorization|signature|x-amz-signature|x-goog-signature|credential|secret)(=|%3[dD])([^&\s]+)")
            .unwrap()
    });
    expression
        .replace_all(&redacted, "$1$2[REDACTED]")
        .into_owned()
}

const SUMMARY_URL_MAX_CHARS: usize = 200;

pub fn summarize_network_url(url: &str) -> String {
    let lower_prefix: String = url.chars().take(5).collect::<String>().to_ascii_lowercase();
    if lower_prefix.starts_with("data:") || lower_prefix.starts_with("blob:") {
        let scheme_end = url.find(':').map(|index| index + 1).unwrap_or(url.len());
        let payload = &url[scheme_end..];
        let media_type: String = payload
            .chars()
            .take_while(|ch| *ch != ',' && *ch != ';')
            .take(60)
            .collect();
        return format!(
            "{}{} [embedded, {} chars]",
            &url[..scheme_end],
            media_type,
            url.chars().count()
        );
    }
    if url.chars().count() > SUMMARY_URL_MAX_CHARS {
        let head: String = url.chars().take(SUMMARY_URL_MAX_CHARS).collect();
        return format!("{head}… [{} chars]", url.chars().count());
    }
    url.to_string()
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
        .and_then(|timing| timing.response_end.map(|end| end - timing.start_time))
        .filter(|elapsed| elapsed.is_finite() && *elapsed >= 0.0)
        .map(|elapsed| format!("{}ms", elapsed.round()))
        .unwrap_or_else(|| "-".to_string());
    let mut line = format!(
        "{} {} {} {} {}",
        entry.method,
        summarize_network_url(&entry.url),
        status,
        bytes,
        elapsed
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
    #[ignore = "launches an isolated owned Chrome; enable with REFACT_NETWORK_CHROME=1"]
    fn owned_chrome_burst_delivers_response_fence_and_evicts_old_request() {
        use std::io::{Read, Write};
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        assert_eq!(std::env::var("REFACT_NETWORK_CHROME").as_deref(), Ok("1"));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        struct Server(Arc<AtomicBool>, Option<std::thread::JoinHandle<()>>);
        impl Drop for Server {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
                self.1.take().unwrap().join().unwrap();
            }
        }
        let stopped = Arc::new(AtomicBool::new(false));
        let server_stopped = stopped.clone();
        let _server = Server(
            stopped,
            Some(std::thread::spawn(move || {
                while !server_stopped.load(Ordering::Relaxed) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut bytes = [0; 8192];
                    let _ = stream.read(&mut bytes);
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}");
                }
            })),
        );
        let profile = tempfile::tempdir().unwrap();
        let options = crate::BrowserLaunchOptions {
            headless: true,
            chromium_sandbox: false,
            ..Default::default()
        };
        let args = options.chrome_args();
        let ignored = options.ignored_default_args();
        let browser = headless_chrome::Browser::new(crate::chrome_launch_options(
            &options,
            profile.path(),
            &args,
            &ignored,
        ))
        .unwrap();
        let tab = browser.new_tab().unwrap();
        let monitor = Arc::new(NetworkMonitorHandle::default());
        crate::setup_network_capture(
            tab.clone(),
            monitor.clone(),
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(crate::websocket::WebSocketRegistry::default()),
            Arc::new(crate::har::HarRecorder::default()),
        )
        .unwrap();
        let base = format!("http://{address}");
        let cursor = monitor.response_cursor();
        tab.navigate_to(&base)
            .unwrap()
            .wait_until_navigated()
            .unwrap();
        let old = monitor
            .wait_for_response(
                &UrlMatcher::glob(&format!("{base}/")).unwrap(),
                &NetworkWaitFilters {
                    status: Some(200),
                    ..Default::default()
                },
                cursor,
                Duration::from_secs(10),
            )
            .unwrap()
            .request_id
            .unwrap();
        let cursor = monitor.response_cursor();
        let started = Instant::now();
        let result = tab.evaluate(
            "(async()=>{for(let i=0;i<1001;i++){if(!(await fetch('/fast?evict='+i)).ok)return false;}return (await fetch('/eviction-fence')).ok;})()",
            true,
        ).unwrap();
        let eval_elapsed = started.elapsed();
        assert_eq!(result.value, Some(serde_json::json!(true)));
        let fence = monitor.wait_for_response(
            &UrlMatcher::glob("**/eviction-fence").unwrap(),
            &NetworkWaitFilters {
                status: Some(200),
                ..Default::default()
            },
            cursor,
            Duration::from_secs(10),
        );
        eprintln!("burst eval={eval_elapsed:?}, fence elapsed={:?}, captured requests={}, responses={}, fence={:?}", started.elapsed(), monitor.request_cursor(), monitor.response_cursor(), fence.as_ref().map(|entry| entry.status));
        fence.unwrap();
        assert!(monitor.network_request(&old).is_err());
        assert_eq!(monitor.network_requests(usize::MAX).len(), RECENT_EVENT_CAP);
    }

    #[test]
    fn active_evicted_request_keeps_extra_headers_without_discovery_resurrection() {
        let monitor = NetworkMonitorHandle::default();
        let raw = "tab-a\0slow";
        monitor.request_started(request(raw, "main", "https://x.test/slow", "Fetch"));
        let id = monitor.network_requests(1)[0].request_id.clone().unwrap();
        monitor.response_received(response(raw, 200, BTreeMap::new()));
        for index in 0..RECENT_EVENT_CAP {
            monitor.request_started(request(
                &format!("tab-a\0filler-{index}"),
                "main",
                "https://x.test/fill",
                "Fetch",
            ));
        }
        assert!(monitor.network_request(&id).is_err());
        assert!(monitor.state.lock().unwrap().active.contains_key(raw));
        monitor.request_extra(
            raw.into(),
            BTreeMap::from([("x-late-request".into(), "request-extra".into())]),
        );
        monitor.response_extra(
            raw.into(),
            BTreeMap::from([("x-late-response".into(), "response-extra".into())]),
        );
        let done = monitor.loading_finished(raw, 12.0, Some(2)).unwrap();
        assert_eq!(
            done.request_headers
                .get("x-late-request")
                .map(String::as_str),
            Some("request-extra")
        );
        assert_eq!(
            done.response_headers
                .get("x-late-response")
                .map(String::as_str),
            Some("response-extra")
        );
        assert!(monitor.network_request(&id).is_err());
        assert_eq!(monitor.network_requests(usize::MAX).len(), RECENT_EVENT_CAP);
    }

    #[test]
    fn redirect_after_discovery_eviction_clears_uncertain_retained_headers() {
        let mut monitor = NetworkMonitor::default();
        let raw = "tab-a\0redirect-evicted";
        monitor.record_request(request(raw, "main", "https://x.test/start", "Fetch"));
        let old_id = monitor.active[raw].entry.request_id.clone();
        monitor.record_request_extra(
            raw.into(),
            BTreeMap::from([("x-uncertain".into(), "old-extra".into())]),
        );
        monitor.record_response_extra(
            raw.into(),
            BTreeMap::from([("x-uncertain".into(), "old-extra".into())]),
        );
        for index in 0..RECENT_EVENT_CAP {
            monitor.record_request(request(
                &format!("tab-a\0filler-{index}"),
                "main",
                "https://x.test/fill",
                "Fetch",
            ));
        }
        assert!(!monitor
            .discovery
            .iter()
            .any(|item| item.entry.request_id == old_id));
        let mut next = request(raw, "main", "https://x.test/end", "Fetch");
        next.redirect_response = Some(response(raw, 302, BTreeMap::new()));
        monitor.record_request(next);
        let old = monitor
            .completed
            .iter()
            .find(|entry| entry.request_id == old_id)
            .unwrap();
        assert!(!old.request_headers.contains_key("x-uncertain"));
        assert!(!old.response_headers.contains_key("x-uncertain"));
        assert_eq!(
            old.header_attribution.as_deref(),
            Some("redirect_extra_info_unverified")
        );
    }

    #[test]
    fn known_csrf_private_key_and_pem_secrets_do_not_escape() {
        let headers = mask_headers(BTreeMap::from([
            ("X-CSRF-Token".into(), "opaque-csrf-value".into()),
            ("X-XSRF-Token".into(), "opaque-xsrf-value".into()),
        ]));
        assert_eq!(headers["X-CSRF-Token"], "[REDACTED]");
        assert_eq!(headers["X-XSRF-Token"], "[REDACTED]");
        let mut body = serde_json::json!({"private_key": "opaque-private-material", "nested": [{"privateKey": "other-private-material"}], "safe": "hello", "pem": "-----BEGIN RSA PRIVATE KEY-----\nfixture-material\n-----END RSA PRIVATE KEY-----"});
        redact_json(&mut body);
        assert_eq!(body["private_key"], "[REDACTED]");
        assert_eq!(body["nested"][0]["privateKey"], "[REDACTED]");
        assert_eq!(body["pem"], "[REDACTED]");
        assert_eq!(body["safe"], "hello");
    }

    #[test]
    fn completed_discovery_accepts_late_unambiguous_extra_headers() {
        let monitor = NetworkMonitorHandle::default();
        monitor.request_started(request("tab-a\0late", "main", "https://x.test/", "Fetch"));
        monitor.response_received(response("tab-a\0late", 200, BTreeMap::new()));
        monitor.loading_finished("tab-a\0late", 12.0, Some(2));
        let id = monitor.network_requests_for_tab("tab-a", 1)[0]
            .request_id
            .clone()
            .unwrap();
        monitor.request_extra(
            "tab-a\0late".into(),
            BTreeMap::from([("X-CSRF-Token".into(), "opaque".into())]),
        );
        monitor.response_extra(
            "tab-a\0late".into(),
            BTreeMap::from([
                ("Content-Type".into(), "application/json".into()),
                ("Content-Length".into(), "2".into()),
            ]),
        );
        let detail = monitor.network_request_for_tab("tab-a", &id).unwrap();
        assert_eq!(
            detail
                .request_headers
                .get("X-CSRF-Token")
                .map(String::as_str),
            Some("[REDACTED]")
        );
        assert_eq!(
            detail
                .response_headers
                .get("Content-Length")
                .map(String::as_str),
            Some("2")
        );
        assert_eq!(
            monitor.response_body_request_id("tab-a", &id).unwrap(),
            "late"
        );
        assert!(monitor.response_body_request_id("tab-b", &id).is_err());
        assert!(NetworkMonitorHandle::default()
            .response_body_request_id("tab-a", &id)
            .is_err());
    }

    #[test]
    fn redirected_extra_info_is_never_assigned_to_a_guessed_hop() {
        // ExtraInfo may precede redirect requestWillBeSent, or follow it. In
        // either order no ordinal/hasExtraInfo exists in our event interface.
        for extra_before_redirect in [false, true] {
            let monitor = NetworkMonitorHandle::default();
            let raw = "tab-a\0redirect";
            let mut first = request(raw, "main", "https://x.test/start", "Fetch");
            first.headers.insert("x-hop".into(), "first-base".into());
            monitor.request_started(first);
            let old = monitor.network_requests(1)[0].request_id.clone().unwrap();
            let extras = BTreeMap::from([("x-hop".into(), "unattributable-extra".into())]);
            if extra_before_redirect {
                monitor.request_extra(raw.into(), extras.clone());
                monitor.response_extra(raw.into(), extras.clone());
            }
            let mut next = request(raw, "main", "https://x.test/end", "Fetch");
            next.headers.insert("x-hop".into(), "second-base".into());
            next.redirect_response = Some(response(
                raw,
                302,
                BTreeMap::from([("x-hop".into(), "redirect-base".into())]),
            ));
            monitor.request_started(next);
            if !extra_before_redirect {
                monitor.request_extra(raw.into(), extras.clone());
                monitor.response_extra(raw.into(), extras);
            }
            let headers = BTreeMap::from([
                ("Content-Length".into(), "2".into()),
                ("Content-Type".into(), "application/json".into()),
            ]);
            monitor.response_received(response(raw, 200, headers));
            monitor.loading_finished(raw, 12.0, Some(2));
            let entries = monitor.network_requests_for_tab("tab-a", 10);
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].request_headers["x-hop"], "second-base");
            assert_eq!(entries[1].request_headers["x-hop"], "first-base");
            assert_eq!(entries[1].response_headers["x-hop"], "redirect-base");
            for entry in entries {
                assert_eq!(
                    entry.header_attribution.as_deref(),
                    Some("redirect_extra_info_unverified")
                );
                assert!(entry.failure_text.is_none());
                let id = entry.request_id.unwrap();
                assert!(monitor.response_body_request_id("tab-a", &id).is_err());
                assert_eq!(
                    monitor
                        .network_request(&id)
                        .unwrap()
                        .header_attribution
                        .as_deref(),
                    Some("redirect_extra_info_unverified")
                );
            }
            assert!(monitor.response_body_request_id("tab-a", &old).is_err());
        }
    }

    #[test]
    fn extra_response_before_base_response_and_orphan_buffers_are_bounded() {
        let monitor = NetworkMonitorHandle::default();
        monitor.request_started(request("tab-a\0early", "main", "https://x.test/", "Fetch"));
        monitor.response_extra(
            "tab-a\0early".into(),
            BTreeMap::from([("x-source".into(), "extra".into())]),
        );
        monitor.response_received(response(
            "tab-a\0early",
            200,
            BTreeMap::from([("x-source".into(), "base".into())]),
        ));
        monitor.loading_finished("tab-a\0early", 12.0, Some(2));
        assert_eq!(
            monitor.network_requests(1)[0].response_headers["x-source"],
            "extra"
        );
        for index in 0..(RECENT_EVENT_CAP + 250) {
            monitor.request_extra(format!("orphan-{index}"), BTreeMap::new());
            monitor.response_extra(format!("orphan-{index}"), BTreeMap::new());
        }
        let state = monitor.state.lock().unwrap();
        assert_eq!(state.request_extra.len(), RECENT_EVENT_CAP);
        assert_eq!(state.request_extra_order.len(), RECENT_EVENT_CAP);
        assert_eq!(state.response_extra.len(), RECENT_EVENT_CAP);
        assert_eq!(state.response_extra_order.len(), RECENT_EVENT_CAP);
        assert!(!state.request_extra.contains_key("orphan-0"));
    }

    #[test]
    fn orphan_response_headers_stay_capped_at_the_recent_event_ring() {
        let monitor = NetworkMonitorHandle::default();
        for index in 0..(RECENT_EVENT_CAP + 250) {
            monitor.response_extra(
                format!("orphan-{index}"),
                BTreeMap::from([("x-trace".to_string(), index.to_string())]),
            );
        }
        let state = monitor.state.lock().unwrap();
        assert_eq!(state.response_extra.len(), RECENT_EVENT_CAP);
        assert_eq!(state.response_extra_order.len(), RECENT_EVENT_CAP);
        assert!(!state.response_extra.contains_key("orphan-0"));
        assert!(state
            .response_extra
            .contains_key(&format!("orphan-{}", RECENT_EVENT_CAP + 249)));
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
    fn regex_flags_drop_javascript_only_modifiers_and_honor_the_rest() {
        for flags in ["g", "y", "gy", "gi", "iy"] {
            assert!(
                UrlMatcher::regex("/API/", flags)
                    .unwrap()
                    .is_match("https://x.test/API/items"),
                "flags {flags} must be accepted"
            );
        }
        assert!(UrlMatcher::regex("/api/", "gi")
            .unwrap()
            .is_match("https://x.test/API/items"));
        assert!(!UrlMatcher::regex("/api/", "g")
            .unwrap()
            .is_match("https://x.test/API/items"));
        assert!(UrlMatcher::regex("a.b", "s").unwrap().is_match("a\nb"));
        assert!(UrlMatcher::regex("^b", "m").unwrap().is_match("a\nb"));

        assert_eq!(
            UrlMatcher::regex("/api/", "d").unwrap_err(),
            "Unsupported regex flags: d"
        );
        let lookahead = UrlMatcher::regex("/api/(?=v2)", "").unwrap_err();
        assert!(lookahead.starts_with("Invalid URL regex"), "{lookahead}");
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
                &NetworkWaitFilters::default(),
                request_cursor,
                Duration::ZERO,
            )
            .unwrap();
        let response = monitor
            .wait_for_response(
                &UrlMatcher::regex(r"/api/items\?token=secret$", "").unwrap(),
                &NetworkWaitFilters::default(),
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
    #[test]
    fn discovery_scopes_redirects_eviction_and_header_updates() {
        let monitor = NetworkMonitorHandle::default();
        let mut first = request(
            "tab-a\0same",
            "main",
            "https://x.test/?token=secret",
            "Fetch",
        );
        first.headers.insert("x-api-key".into(), "hidden".into());
        monitor.request_started(first);
        let old = monitor.network_requests_for_tab("tab-a", 10)[0]
            .request_id
            .clone()
            .unwrap();
        monitor.request_extra(
            "tab-a\0same".into(),
            BTreeMap::from([("Cookie".into(), "secret".into())]),
        );
        assert_eq!(
            monitor
                .network_request_for_tab("tab-a", &old)
                .unwrap()
                .request_headers["Cookie"],
            "[REDACTED]"
        );
        let mut next = request("tab-a\0same", "main", "https://x.test/next", "Fetch");
        next.redirect_response = Some(response("tab-a\0same", 302, BTreeMap::new()));
        monitor.request_started(next);
        let entries = monitor.network_requests_for_tab("tab-a", 10);
        assert_eq!(entries.len(), 2);
        assert_ne!(entries[0].request_id, entries[1].request_id);
        assert_eq!(entries[1].request_id.as_deref(), Some(old.as_str()));
        assert!(!entries[1].url.contains("secret"));
        assert!(monitor.network_request_for_tab("tab-b", &old).is_err());
        assert!(NetworkMonitorHandle::default()
            .network_request(&old)
            .is_err());
        assert!(monitor.network_request("arbitrary").is_err());
        for i in 0..RECENT_EVENT_CAP {
            monitor.request_started(request(
                &format!("tab-a\0{i}"),
                "main",
                "https://x.test/",
                "Fetch",
            ));
        }
        assert!(monitor.network_request(&old).is_err());
        assert_eq!(
            monitor.network_requests_for_tab("tab-a", usize::MAX).len(),
            RECENT_EVENT_CAP
        );
    }

    #[test]
    fn structured_body_secrets_are_redacted_recursively() {
        let mut value = serde_json::json!({"nested": [{"password": "hunter2", "x-api-key": "abc", "safe": "token=secret"}], "signature": "signed"});
        redact_json(&mut value);
        assert_eq!(value["nested"][0]["password"], "[REDACTED]");
        assert_eq!(value["nested"][0]["x-api-key"], "[REDACTED]");
        assert_eq!(value["nested"][0]["safe"], "token=[REDACTED]");
        assert_eq!(value["signature"], "[REDACTED]");
    }
    #[test]
    fn body_preflight_rejects_cross_target_non_json_and_stale_hop() {
        let monitor = NetworkMonitorHandle::default();
        monitor.request_started(request(
            "tab-a\0request",
            "main",
            "https://x.test/",
            "Fetch",
        ));
        let first = monitor.network_requests_for_tab("tab-a", 1)[0]
            .request_id
            .clone()
            .unwrap();
        let headers = BTreeMap::from([
            ("Content-Length".into(), "2".into()),
            ("Content-Type".into(), "application/json".into()),
        ]);
        monitor.response_received(response("tab-a\0request", 200, headers));
        monitor.loading_finished("tab-a\0request", 12.0, Some(2));
        assert_eq!(
            monitor.response_body_request_id("tab-a", &first).unwrap(),
            "request"
        );
        assert!(monitor.response_body_request_id("tab-b", &first).is_err());
        let mut next = request("tab-a\0request", "main", "https://x.test/next", "Fetch");
        next.redirect_response = Some(response("tab-a\0request", 302, BTreeMap::new()));
        monitor.request_started(next);
        assert!(monitor.response_body_request_id("tab-a", &first).is_err());
        // Use an independent request: the redirect chain is now deliberately
        // refused earlier for unknown header attribution, not MIME type.
        monitor.request_started(request(
            "tab-a\0plain",
            "main",
            "https://x.test/plain",
            "Fetch",
        ));
        let second = monitor.network_requests_for_tab("tab-a", 1)[0]
            .request_id
            .clone()
            .unwrap();
        let headers = BTreeMap::from([
            ("Content-Length".into(), "5".into()),
            ("Content-Type".into(), "text/plain".into()),
        ]);
        monitor.response_received(response("tab-a\0plain", 200, headers));
        monitor.loading_finished("tab-a\0plain", 13.0, Some(5));
        assert_eq!(
            monitor
                .response_body_request_id("tab-a", &second)
                .unwrap_err(),
            "Only JSON response bodies are available for safe redaction"
        );
    }
}
