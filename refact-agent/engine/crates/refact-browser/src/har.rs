use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use refact_integrations::browser_models::{
    HarContentPolicy, HarMode, HarNotFound, RouteHandler, UrlPattern,
};
use refact_integrations::browser_types::NetworkEntry;

use crate::network::{UrlMatcher, mask_headers, mask_text};

const HAR_BODY_QUEUE_CAP: usize = 64;
const HAR_BODY_JOIN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct HarResponseBody {
    pub body: String,
    pub base64: bool,
    pub mime_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarFile {
    pub log: HarLog,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarLog {
    pub version: String,
    pub creator: HarCreator,
    pub entries: Vec<HarEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarCreator {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarEntry {
    #[serde(rename = "startedDateTime")]
    pub started_date_time: String,
    pub time: f64,
    pub request: HarRequest,
    pub response: HarResponse,
    pub cache: BTreeMap<String, String>,
    pub timings: HarTimings,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarRequest {
    pub method: String,
    pub url: String,
    #[serde(rename = "httpVersion")]
    pub http_version: String,
    pub headers: Vec<HarNameValue>,
    #[serde(rename = "queryString")]
    pub query_string: Vec<HarNameValue>,
    pub cookies: Vec<HarNameValue>,
    #[serde(rename = "headersSize")]
    pub headers_size: i64,
    #[serde(rename = "bodySize")]
    pub body_size: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarResponse {
    pub status: u16,
    #[serde(rename = "statusText")]
    pub status_text: String,
    #[serde(rename = "httpVersion")]
    pub http_version: String,
    pub headers: Vec<HarNameValue>,
    pub cookies: Vec<HarNameValue>,
    pub content: HarContent,
    #[serde(rename = "redirectURL")]
    pub redirect_url: String,
    #[serde(rename = "headersSize")]
    pub headers_size: i64,
    #[serde(rename = "bodySize")]
    pub body_size: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarContent {
    pub size: i64,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarNameValue {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarTimings {
    pub send: f64,
    pub wait: f64,
    pub receive: f64,
}

#[derive(Clone, Debug)]
struct HarRecording {
    path: PathBuf,
    mode: HarMode,
    content: HarContentPolicy,
    matcher: Option<UrlMatcher>,
    entries: Vec<HarEntry>,
    base: Option<(PathBuf, Vec<HarEntry>)>,
    bodies_skipped: usize,
}

#[derive(Clone, Debug)]
pub struct HarReplay {
    entries: Vec<HarEntry>,
    matcher: Option<UrlMatcher>,
    not_found: HarNotFound,
    label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarRecordingSummary {
    pub path: String,
    pub entry_count: usize,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_from: Option<String>,
    #[serde(default)]
    pub replaced_entries: usize,
    #[serde(default)]
    pub appended_entries: usize,
    #[serde(default)]
    pub bodies_skipped: usize,
}

#[derive(Debug, Default)]
struct HarRecorderState {
    recording: Option<HarRecording>,
}

#[derive(Debug, Default)]
struct HarShared {
    state: Mutex<HarRecorderState>,
    finished: Mutex<bool>,
    changed: Condvar,
}

pub type HarBodyFetcher = Box<dyn FnOnce() -> Option<HarResponseBody> + Send>;

struct BodyWorker {
    sender: SyncSender<(NetworkEntry, HarBodyFetcher)>,
    handle: std::thread::JoinHandle<()>,
}

#[derive(Default)]
pub struct HarRecorder {
    shared: Arc<HarShared>,
    worker: Mutex<Option<BodyWorker>>,
}

impl std::fmt::Debug for HarRecorder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HarRecorder")
            .field("shared", &self.shared)
            .finish()
    }
}

impl HarRecorder {
    pub fn start(
        &self,
        artifacts_dir: &Path,
        path: Option<&str>,
        mode: HarMode,
        content: HarContentPolicy,
        url_filter: Option<&UrlPattern>,
        update: Option<&str>,
    ) -> Result<PathBuf, String> {
        let matcher = url_filter.map(matcher_for_pattern).transpose()?;
        let base = update
            .map(|update| {
                let path = artifact_har_path(artifacts_dir, update)?;
                let bytes = std::fs::read(&path).map_err(|error| {
                    format!("Failed to read HAR to update {}: {error}", path.display())
                })?;
                let file: HarFile = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Invalid HAR {}: {error}", path.display()))?;
                Ok::<_, String>((path, file.log.entries))
            })
            .transpose()?;
        let path = match (path, &base) {
            (Some(path), _) => artifact_har_path(artifacts_dir, path)?,
            (None, Some((base_path, _))) => base_path.clone(),
            (None, None) => artifacts_dir.join(format!("network-{}.har", now_millis())),
        };
        if path.extension().and_then(|value| value.to_str()) != Some("har") {
            return Err("HAR path must end in .har".to_string());
        }
        let mut state = self.shared.state.lock().unwrap();
        if state.recording.is_some() {
            return Err("HAR recording is already active".to_string());
        }
        state.recording = Some(HarRecording {
            path: path.clone(),
            mode,
            content,
            matcher,
            entries: Vec::new(),
            base,
            bodies_skipped: 0,
        });
        drop(state);
        self.start_worker();
        Ok(path)
    }

    fn start_worker(&self) {
        let mut worker = self.worker.lock().unwrap();
        if worker.is_some() {
            return;
        }
        let (sender, receiver) = sync_channel::<(NetworkEntry, HarBodyFetcher)>(HAR_BODY_QUEUE_CAP);
        let shared = self.shared.clone();
        *shared.finished.lock().unwrap() = false;
        let handle = std::thread::spawn(move || {
            for (entry, fetch) in receiver {
                let body = fetch();
                record_into(&shared, &entry, body);
            }
            *shared.finished.lock().unwrap() = true;
            shared.changed.notify_all();
        });
        *worker = Some(BodyWorker { sender, handle });
    }

    pub fn is_active(&self) -> bool {
        self.shared.state.lock().unwrap().recording.is_some()
    }

    pub fn record(&self, entry: &NetworkEntry, body: Option<HarResponseBody>) {
        record_into(&self.shared, entry, body);
    }

    pub fn capture_body(&self, entry: &NetworkEntry, fetch: HarBodyFetcher) {
        if !self.is_active() {
            return;
        }
        let queued = self
            .worker
            .lock()
            .unwrap()
            .as_ref()
            .map(|worker| worker.sender.try_send((entry.clone(), fetch)));
        match queued {
            Some(Ok(())) => {}
            Some(Err(TrySendError::Full((entry, _)))) => {
                record_into(&self.shared, &entry, None);
                self.count_skipped_body();
            }
            Some(Err(TrySendError::Disconnected((entry, _)))) => {
                record_into(&self.shared, &entry, None)
            }
            None => record_into(&self.shared, entry, None),
        }
    }

    fn count_skipped_body(&self) {
        if let Some(recording) = self.shared.state.lock().unwrap().recording.as_mut() {
            recording.bodies_skipped += 1;
        }
    }

    fn join_worker(&self, timeout: Duration) {
        let Some(worker) = self.worker.lock().unwrap().take() else {
            return;
        };
        drop(worker.sender);
        let mut finished = self.shared.finished.lock().unwrap();
        let deadline = std::time::Instant::now() + timeout;
        while !*finished {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return;
            }
            let (next, wait) = self
                .shared
                .changed
                .wait_timeout(finished, remaining)
                .unwrap();
            finished = next;
            if wait.timed_out() && !*finished {
                return;
            }
        }
        drop(finished);
        let _ = worker.handle.join();
    }

    pub fn stop(&self) -> Result<HarRecordingSummary, String> {
        self.join_worker(HAR_BODY_JOIN_TIMEOUT);
        let mut state = self.shared.state.lock().unwrap();
        let recording = state
            .recording
            .take()
            .ok_or_else(|| "HAR recording is not active".to_string())?;
        if let Some(parent) = recording.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create HAR directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let (updated_from, base_entries) = match recording.base {
            Some((path, entries)) => (Some(path.to_string_lossy().into_owned()), Some(entries)),
            None => (None, None),
        };
        let merge = merge_har_entries(base_entries.unwrap_or_default(), recording.entries);
        let file = HarFile {
            log: HarLog {
                version: "1.2".to_string(),
                creator: HarCreator {
                    name: "Refact Browser".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
                entries: merge.entries,
            },
        };
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|error| format!("Failed to serialize HAR: {error}"))?;
        std::fs::write(&recording.path, &bytes).map_err(|error| {
            format!("Failed to write HAR {}: {error}", recording.path.display())
        })?;
        Ok(HarRecordingSummary {
            path: recording.path.to_string_lossy().into_owned(),
            entry_count: file.log.entries.len(),
            bytes: bytes.len() as u64,
            updated_from,
            replaced_entries: merge.replaced,
            appended_entries: merge.appended,
            bodies_skipped: recording.bodies_skipped,
        })
    }
}

fn record_into(shared: &HarShared, entry: &NetworkEntry, body: Option<HarResponseBody>) {
    let mut state = shared.state.lock().unwrap();
    let Some(recording) = state.recording.as_mut() else {
        return;
    };
    if recording
        .matcher
        .as_ref()
        .is_some_and(|matcher| !matcher.is_match(&entry.url))
    {
        return;
    }
    recording.entries.push(entry_from_monitor(
        entry,
        body,
        recording.mode,
        recording.content,
    ));
}

#[derive(Debug, Default, PartialEq)]
pub struct HarMerge {
    pub entries: Vec<HarEntry>,
    pub replaced: usize,
    pub appended: usize,
}

pub fn merge_har_entries(base: Vec<HarEntry>, recorded: Vec<HarEntry>) -> HarMerge {
    let mut merge = HarMerge {
        entries: base,
        ..Default::default()
    };
    for entry in recorded {
        match merge.entries.iter_mut().find(|existing| {
            existing.request.url == entry.request.url
                && existing
                    .request
                    .method
                    .eq_ignore_ascii_case(&entry.request.method)
        }) {
            Some(existing) => {
                *existing = entry;
                merge.replaced += 1;
            }
            None => {
                merge.entries.push(entry);
                merge.appended += 1;
            }
        }
    }
    merge
}

fn artifact_har_path(artifacts_dir: &Path, path: &str) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(path);
    let file_name = candidate
        .file_name()
        .ok_or_else(|| "HAR path must name a file".to_string())?;
    if candidate.components().count() != 1 {
        return Err(
            "HAR path must be a file name inside the runtime artifact directory".to_string(),
        );
    }
    Ok(artifacts_dir.join(file_name))
}

impl HarReplay {
    pub fn load(
        path: &Path,
        url_filter: Option<&UrlPattern>,
        not_found: HarNotFound,
    ) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("Failed to read HAR {}: {error}", path.display()))?;
        let file: HarFile = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Invalid HAR {}: {error}", path.display()))?;
        let label = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
        Ok(Self {
            entries: file.log.entries,
            matcher: url_filter.map(matcher_for_pattern).transpose()?,
            not_found,
            label,
        })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn not_found(&self) -> HarNotFound {
        self.not_found
    }

    pub fn match_request(&self, method: &str, url: &str) -> Option<RouteHandler> {
        if self
            .matcher
            .as_ref()
            .is_some_and(|matcher| !matcher.is_match(url))
        {
            return None;
        }
        let masked_url = mask_text(url);
        if let Some(entry) = self.entries.iter().find(|entry| {
            entry.request.method.eq_ignore_ascii_case(method) && entry.request.url == masked_url
        }) {
            let headers = entry
                .response
                .headers
                .iter()
                .map(|header| (header.name.clone(), header.value.clone()))
                .collect();
            return Some(RouteHandler::Fulfill {
                status: entry.response.status,
                headers,
                body: entry.response.content.text.clone(),
                path: None,
                json: None,
                content_type: Some(entry.response.content.mime_type.clone()),
                body_base64: entry.response.content.encoding.as_deref() == Some("base64"),
            });
        }
        match self.not_found {
            HarNotFound::Abort => Some(RouteHandler::Abort {
                reason: "blockedbyclient".to_string(),
            }),
            HarNotFound::Fallback => None,
        }
    }
}

fn entry_from_monitor(
    entry: &NetworkEntry,
    body: Option<HarResponseBody>,
    mode: HarMode,
    content_policy: HarContentPolicy,
) -> HarEntry {
    let request_headers = mask_headers(entry.request_headers.clone());
    let response_headers = mask_headers(entry.response_headers.clone());
    let request_start = entry
        .timing
        .as_ref()
        .and_then(|timing| timing.request_start)
        .unwrap_or(entry.timestamp);
    let response_start = entry
        .timing
        .as_ref()
        .and_then(|timing| timing.response_start)
        .unwrap_or(request_start);
    let response_end = entry
        .timing
        .as_ref()
        .and_then(|timing| timing.response_end)
        .unwrap_or(response_start);
    let body_size = body
        .as_ref()
        .map(|body| body.body.len() as i64)
        .or_else(|| entry.encoded_data_length.map(|value| value as i64))
        .unwrap_or(0);
    let mime_type = body
        .as_ref()
        .and_then(|body| body.mime_type.clone())
        .or_else(|| header_value(&response_headers, "content-type"))
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let content = match content_policy {
        HarContentPolicy::Omit => HarContent {
            size: body_size,
            mime_type,
            text: None,
            encoding: None,
        },
        HarContentPolicy::Embed | HarContentPolicy::Attach => {
            let text = body.as_ref().map(|body| {
                if body.base64 {
                    body.body.clone()
                } else {
                    mask_text(&body.body)
                }
            });
            HarContent {
                size: body_size,
                mime_type,
                text,
                encoding: body
                    .as_ref()
                    .filter(|body| body.base64)
                    .map(|_| "base64".to_string()),
            }
        }
    };
    HarEntry {
        started_date_time: "1970-01-01T00:00:00.000Z".to_string(),
        time: (response_end - entry.timestamp).max(0.0),
        request: HarRequest {
            method: entry.method.clone(),
            url: mask_text(&entry.url),
            http_version: "HTTP/1.1".to_string(),
            headers: header_pairs(request_headers),
            query_string: query_pairs(&entry.url),
            cookies: Vec::new(),
            headers_size: if mode == HarMode::Full { 0 } else { -1 },
            body_size: 0,
        },
        response: HarResponse {
            status: entry.status.unwrap_or(0),
            status_text: entry.status_text.clone().unwrap_or_default(),
            http_version: "HTTP/1.1".to_string(),
            headers: header_pairs(response_headers),
            cookies: Vec::new(),
            content,
            redirect_url: header_value(&entry.response_headers, "location")
                .map(|value| mask_text(&value))
                .unwrap_or_default(),
            headers_size: if mode == HarMode::Full { 0 } else { -1 },
            body_size,
        },
        cache: BTreeMap::new(),
        timings: HarTimings {
            send: (request_start - entry.timestamp).max(0.0),
            wait: (response_start - request_start).max(0.0),
            receive: (response_end - response_start).max(0.0),
        },
    }
}

fn header_pairs(headers: BTreeMap<String, String>) -> Vec<HarNameValue> {
    headers
        .into_iter()
        .map(|(name, value)| HarNameValue { name, value })
        .collect()
}

fn query_pairs(url: &str) -> Vec<HarNameValue> {
    let Some(query) = url.split_once('?').map(|(_, query)| query) else {
        return Vec::new();
    };
    query
        .split('#')
        .next()
        .unwrap_or(query)
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            HarNameValue {
                name: name.to_string(),
                value: if is_sensitive_name(name) {
                    "[REDACTED]".to_string()
                } else {
                    mask_text(value)
                },
            }
        })
        .collect()
}

fn is_sensitive_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "password" | "passwd" | "token" | "api_key" | "api-key" | "authorization"
    )
}

fn header_value(headers: &BTreeMap<String, String>, name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

fn matcher_for_pattern(pattern: &UrlPattern) -> Result<UrlMatcher, String> {
    match pattern {
        UrlPattern::Text(value) => UrlMatcher::text(value),
        UrlPattern::Regex { source, flags } => UrlMatcher::regex(source, flags),
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn normalize_response_body(
    body: String,
    base64: bool,
    mime_type: Option<String>,
) -> HarResponseBody {
    let body = if base64 { body } else { mask_text(&body) };
    HarResponseBody {
        body,
        base64,
        mime_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use refact_integrations::browser_types::NetworkTiming;

    fn network_entry() -> NetworkEntry {
        NetworkEntry {
            timestamp: 10.0,
            method: "GET".to_string(),
            url: "https://example.test/page?token=secret".to_string(),
            resource_type: "Document".to_string(),
            status: Some(200),
            status_text: Some("OK".to_string()),
            request_headers: BTreeMap::from([(
                "Authorization".to_string(),
                "Bearer secret".to_string(),
            )]),
            response_headers: BTreeMap::from([(
                "Content-Type".to_string(),
                "text/html".to_string(),
            )]),
            timing: Some(NetworkTiming {
                start_time: 10.0,
                request_start: Some(10.1),
                response_start: Some(10.4),
                response_end: Some(10.7),
            }),
            encoded_data_length: Some(20),
            ..Default::default()
        }
    }

    fn distinct_entry(index: usize) -> NetworkEntry {
        NetworkEntry {
            url: format!("https://example.test/resource-{index}"),
            ..network_entry()
        }
    }

    fn started_recorder(dir: &Path) -> HarRecorder {
        let recorder = HarRecorder::default();
        recorder
            .start(
                dir,
                Some("bodies.har"),
                HarMode::Full,
                HarContentPolicy::Embed,
                None,
                None,
            )
            .unwrap();
        recorder
    }

    #[test]
    fn the_body_worker_drains_every_queued_capture_before_stop_returns() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = started_recorder(dir.path());
        let captured = Arc::new(Mutex::new(Vec::new()));
        for index in 0..8 {
            let captured = captured.clone();
            recorder.capture_body(
                &distinct_entry(index),
                Box::new(move || {
                    captured.lock().unwrap().push(index);
                    Some(HarResponseBody {
                        body: format!("body-{index}"),
                        base64: false,
                        mime_type: None,
                    })
                }),
            );
        }
        let summary = recorder.stop().unwrap();
        assert_eq!(*captured.lock().unwrap(), (0..8).collect::<Vec<_>>());
        assert_eq!(summary.entry_count, 8);
        assert_eq!(summary.bodies_skipped, 0);
    }

    #[test]
    fn captures_past_the_queue_cap_are_recorded_without_a_body_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = started_recorder(dir.path());
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let blocker = release.clone();
        recorder.capture_body(
            &distinct_entry(0),
            Box::new(move || {
                let (lock, changed) = &*blocker;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = changed.wait(released).unwrap();
                }
                None
            }),
        );
        for index in 1..HAR_BODY_QUEUE_CAP + 5 {
            recorder.capture_body(&distinct_entry(index), Box::new(|| None));
        }
        {
            let (lock, changed) = &*release;
            *lock.lock().unwrap() = true;
            changed.notify_all();
        }
        let summary = recorder.stop().unwrap();
        assert!(
            summary.bodies_skipped >= 3,
            "over-cap captures must be counted, got {}",
            summary.bodies_skipped
        );
        assert_eq!(summary.entry_count, HAR_BODY_QUEUE_CAP + 5);
    }

    #[test]
    fn stop_gives_up_on_a_wedged_capture_instead_of_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = started_recorder(dir.path());
        let wedged = Arc::new((Mutex::new(false), Condvar::new()));
        let blocker = wedged.clone();
        recorder.capture_body(
            &network_entry(),
            Box::new(move || {
                let (lock, changed) = &*blocker;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = changed.wait(released).unwrap();
                }
                None
            }),
        );
        let started = std::time::Instant::now();
        recorder.join_worker(Duration::from_millis(200));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "join must be bounded, took {:?}",
            started.elapsed()
        );
        assert!(recorder.stop().is_ok());
        let (lock, changed) = &*wedged;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }

    #[test]
    fn captures_are_dropped_when_no_recording_is_active() {
        let recorder = HarRecorder::default();
        let called = Arc::new(Mutex::new(false));
        let flag = called.clone();
        recorder.capture_body(
            &network_entry(),
            Box::new(move || {
                *flag.lock().unwrap() = true;
                None
            }),
        );
        assert!(!*called.lock().unwrap());
        assert!(recorder.stop().is_err());
    }

    #[test]
    fn har_entry_assembly_masks_headers_query_and_content() {
        let entry = entry_from_monitor(
            &network_entry(),
            Some(normalize_response_body(
                "<html>password=hunter2</html>".to_string(),
                false,
                Some("text/html".to_string()),
            )),
            HarMode::Full,
            HarContentPolicy::Embed,
        );
        assert_eq!(entry.request.headers[0].value, "[REDACTED]");
        assert_eq!(entry.request.query_string[0].value, "[REDACTED]");
        assert!(!entry.response.content.text.unwrap().contains("hunter2"));
        assert_eq!(entry.timings.wait, 0.3000000000000007);
    }

    fn entry_for(method: &str, url: &str, status: u16) -> HarEntry {
        let mut entry = entry_from_monitor(
            &network_entry(),
            None,
            HarMode::Full,
            HarContentPolicy::Omit,
        );
        entry.request.method = method.to_string();
        entry.request.url = url.to_string();
        entry.response.status = status;
        entry
    }

    #[test]
    fn merging_replaces_matched_method_and_url_and_appends_the_rest() {
        let merge = merge_har_entries(
            vec![
                entry_for("GET", "https://example.test/a", 200),
                entry_for("POST", "https://example.test/a", 201),
            ],
            vec![
                entry_for("get", "https://example.test/a", 503),
                entry_for("GET", "https://example.test/b", 200),
            ],
        );

        assert_eq!(merge.replaced, 1);
        assert_eq!(merge.appended, 1);
        assert_eq!(merge.entries.len(), 3);
        assert_eq!(merge.entries[0].response.status, 503);
        assert_eq!(merge.entries[1].response.status, 201);
        assert_eq!(merge.entries[2].request.url, "https://example.test/b");
    }

    #[test]
    fn recording_without_an_update_base_keeps_every_recorded_entry() {
        let merge = merge_har_entries(
            Vec::new(),
            vec![
                entry_for("GET", "https://example.test/a", 200),
                entry_for("GET", "https://example.test/b", 200),
            ],
        );

        assert_eq!((merge.replaced, merge.appended), (0, 2));
        assert_eq!(merge.entries.len(), 2);
    }

    #[test]
    fn update_mode_records_into_the_existing_har_and_reports_the_merge() {
        let dir = tempfile::tempdir().unwrap();
        let artifacts = dir.path();
        std::fs::create_dir_all(artifacts).unwrap();
        let base = HarFile {
            log: HarLog {
                version: "1.2".to_string(),
                creator: HarCreator {
                    name: "Refact Browser".to_string(),
                    version: "0".to_string(),
                },
                entries: vec![entry_from_monitor(
                    &network_entry(),
                    None,
                    HarMode::Full,
                    HarContentPolicy::Omit,
                )],
            },
        };
        std::fs::write(
            artifacts.join("login.har"),
            serde_json::to_vec_pretty(&base).unwrap(),
        )
        .unwrap();

        let recorder = HarRecorder::default();
        let path = recorder
            .start(
                artifacts,
                None,
                HarMode::Full,
                HarContentPolicy::Omit,
                None,
                Some("login.har"),
            )
            .unwrap();
        assert_eq!(path, artifacts.join("login.har"));
        recorder.record(&network_entry(), None);
        let summary = recorder.stop().unwrap();

        assert_eq!(summary.entry_count, 1);
        assert_eq!((summary.replaced_entries, summary.appended_entries), (1, 0));
        assert_eq!(
            summary.updated_from.as_deref(),
            Some(artifacts.join("login.har").to_string_lossy().as_ref())
        );
        let written: HarFile =
            serde_json::from_slice(&std::fs::read(artifacts.join("login.har")).unwrap()).unwrap();
        assert_eq!(written.log.entries.len(), 1);
    }

    #[test]
    fn update_mode_rejects_paths_outside_the_artifact_directory_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = HarRecorder::default();
        let error = recorder
            .start(
                dir.path(),
                None,
                HarMode::Full,
                HarContentPolicy::Omit,
                None,
                Some("nested/login.har"),
            )
            .unwrap_err();
        assert!(error.contains("must be a file name"), "{error}");

        let missing = recorder
            .start(
                dir.path(),
                None,
                HarMode::Full,
                HarContentPolicy::Omit,
                None,
                Some("absent.har"),
            )
            .unwrap_err();
        assert!(
            missing.contains("Failed to read HAR to update"),
            "{missing}"
        );
        assert!(!recorder.is_active());
    }

    #[test]
    fn replay_matching_fulfills_or_applies_not_found_policy() {
        let entry = entry_from_monitor(
            &network_entry(),
            Some(HarResponseBody {
                body: base64::engine::general_purpose::STANDARD.encode("offline page"),
                base64: true,
                mime_type: Some("text/html".to_string()),
            }),
            HarMode::Full,
            HarContentPolicy::Embed,
        );
        let replay = HarReplay {
            entries: vec![entry],
            matcher: None,
            not_found: HarNotFound::Abort,
            label: "page.har".to_string(),
        };
        assert!(matches!(
            replay.match_request("GET", "https://example.test/page?token=secret"),
            Some(RouteHandler::Fulfill {
                body_base64: true,
                ..
            })
        ));
        assert!(matches!(
            replay.match_request("GET", "https://example.test/missing"),
            Some(RouteHandler::Abort { .. })
        ));

        let fallback = HarReplay {
            not_found: HarNotFound::Fallback,
            ..replay
        };
        assert!(fallback
            .match_request("GET", "https://example.test/missing")
            .is_none());
    }
}
