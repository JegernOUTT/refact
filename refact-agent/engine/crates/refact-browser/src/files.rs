use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use headless_chrome::browser::transport::Transport;
use headless_chrome::protocol::cdp::types::Event;
use std::time::{Duration, Instant};

use headless_chrome::Tab;
use headless_chrome::protocol::cdp::DOM;
use refact_integrations::browser_models::{DownloadInfo, DownloadState, UploadInfo};
use serde_json::{Value, json};

use crate::{ElementHandle, WorldManager};

const CANCELED_REASON: &str = "canceled";
const DOWNLOAD_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputMetadata {
    tag: String,
    input_type: String,
    multiple: bool,
    webkitdirectory: bool,
}

pub fn set_input_files(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    paths: &[String],
    source: &str,
) -> Result<UploadInfo, String> {
    let target = world
        .call_function_handle_on(
            tab,
            handle,
            "function() { return this instanceof HTMLLabelElement ? this.control : this; }",
        )
        .map_err(|error| error.to_string())?;
    let result = input_metadata(tab, world, &target).and_then(|metadata| {
        let files = validate_input_paths(&metadata, paths)?;
        tab.call_method(DOM::SetFileInputFiles {
            files: files.clone(),
            node_id: None,
            backend_node_id: None,
            object_id: Some(target.object_id.clone()),
        })
        .map_err(|error| format!("Failed to set input files: {error}"))?;
        Ok(UploadInfo {
            paths: files.iter().map(|path| mask_text(path)).collect(),
            source: source.to_string(),
            in_memory_payloads: false,
        })
    });
    let _ = world.release_handle(tab, &target);
    result
}

fn input_metadata(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
) -> Result<InputMetadata, String> {
    let value = world
        .call_function_on(
            tab,
            handle,
            "function() { return { tag: this.tagName ? this.tagName.toLowerCase() : '', input_type: this.type || '', multiple: !!this.multiple, webkitdirectory: !!this.webkitdirectory }; }",
            Vec::new(),
        )
        .map_err(|error| error.to_string())?;
    serde_json::from_value::<InputMetadataWire>(value)
        .map(InputMetadata::from)
        .map_err(|error| format!("Failed to inspect file input: {error}"))
}

#[derive(serde::Deserialize)]
struct InputMetadataWire {
    tag: String,
    input_type: String,
    multiple: bool,
    webkitdirectory: bool,
}

impl From<InputMetadataWire> for InputMetadata {
    fn from(value: InputMetadataWire) -> Self {
        Self {
            tag: value.tag,
            input_type: value.input_type,
            multiple: value.multiple,
            webkitdirectory: value.webkitdirectory,
        }
    }
}

fn validate_input_paths(metadata: &InputMetadata, paths: &[String]) -> Result<Vec<String>, String> {
    if metadata.tag != "input" || metadata.input_type != "file" {
        return Err(format!(
            "set_input_files requires <input type=\"file\">, found <{}{}>",
            metadata.tag,
            if metadata.input_type.is_empty() {
                String::new()
            } else {
                format!(" type=\"{}\"", metadata.input_type)
            }
        ));
    }
    if paths.len() > 1 && !metadata.multiple && !metadata.webkitdirectory {
        return Err("File input does not accept multiple files".to_string());
    }
    let mut canonical = Vec::with_capacity(paths.len());
    for path in paths {
        let path = Path::new(path);
        let value = path
            .canonicalize()
            .map_err(|_| format!("Upload path does not exist: {}", path.display()))?;
        if metadata.webkitdirectory {
            if paths.len() != 1 || !value.is_dir() {
                return Err("Directory upload requires exactly one directory path".to_string());
            }
        } else if !value.is_file() {
            return Err(format!("Upload path is not a file: {}", value.display()));
        }
        canonical.push(value.to_string_lossy().into_owned());
    }
    Ok(canonical)
}

#[derive(Default)]
struct FileChooserState {
    armed_paths: Option<Vec<String>>,
    pending: Option<(u32, bool, Vec<String>)>,
    uploads: Vec<UploadInfo>,
}

#[derive(Default)]
pub struct FileChooserManager {
    state: Mutex<FileChooserState>,
}

impl FileChooserManager {
    pub fn arm(&self, paths: &[String]) -> Result<(), String> {
        let files = canonical_files(paths)?;
        let mut state = self.state.lock().unwrap();
        if state.armed_paths.is_some() {
            return Err("A file chooser is already armed".to_string());
        }
        state.armed_paths = Some(files);
        Ok(())
    }

    pub fn is_armed(&self) -> bool {
        self.state.lock().unwrap().armed_paths.is_some()
    }

    pub fn opened(&self, backend_node_id: Option<u32>, multiple: bool) {
        let mut state = self.state.lock().unwrap();
        let Some(paths) = state.armed_paths.take() else {
            return;
        };
        if let Some(backend_node_id) = backend_node_id {
            state.pending = Some((backend_node_id, multiple, paths));
        }
    }

    pub fn complete(&self, tab: &Tab, timeout: Duration) -> Result<UploadInfo, String> {
        let deadline = Instant::now() + timeout;
        let (backend_node_id, multiple, paths) = loop {
            if let Some(pending) = self.state.lock().unwrap().pending.take() {
                break pending;
            }
            if Instant::now() >= deadline {
                self.state.lock().unwrap().armed_paths = None;
                return Err("Timed out waiting for file chooser".to_string());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        if paths.len() > 1 && !multiple {
            return Err("File chooser does not accept multiple files".to_string());
        }
        tab.call_method(DOM::SetFileInputFiles {
            files: paths.clone(),
            node_id: None,
            backend_node_id: Some(backend_node_id),
            object_id: None,
        })
        .map_err(|error| format!("Failed to set file chooser files: {error}"))?;
        let upload = UploadInfo {
            paths: paths.iter().map(|path| mask_text(path)).collect(),
            source: "file_chooser".to_string(),
            in_memory_payloads: false,
        };
        self.state.lock().unwrap().uploads.push(upload.clone());
        Ok(upload)
    }

    pub fn take_uploads(&self) -> Vec<UploadInfo> {
        std::mem::take(&mut self.state.lock().unwrap().uploads)
    }

    pub fn record(&self, upload: UploadInfo) {
        self.state.lock().unwrap().uploads.push(upload);
    }

    pub fn disarm(&self) {
        let mut state = self.state.lock().unwrap();
        state.armed_paths = None;
        state.pending = None;
    }
}

fn canonical_files(paths: &[String]) -> Result<Vec<String>, String> {
    let metadata = InputMetadata {
        tag: "input".to_string(),
        input_type: "file".to_string(),
        multiple: true,
        webkitdirectory: false,
    };
    validate_input_paths(&metadata, paths)
}

#[derive(Debug, Clone)]
struct TrackedDownload {
    sequence: u64,
    info: DownloadInfo,
    reported: bool,
}

const RETAINED_TERMINAL_DOWNLOADS: usize = 50;

#[derive(Default)]
struct DownloadTrackerState {
    sequence: u64,
    downloads: Vec<TrackedDownload>,
    indexes: HashMap<String, usize>,
}

impl DownloadTrackerState {
    fn prune_terminal(&mut self) {
        let terminal = self
            .downloads
            .iter()
            .filter(|download| is_terminal(&download.info.state))
            .count();
        if terminal <= RETAINED_TERMINAL_DOWNLOADS {
            return;
        }
        let mut droppable = terminal - RETAINED_TERMINAL_DOWNLOADS;
        self.downloads.retain(|download| {
            if droppable == 0 || !is_terminal(&download.info.state) {
                return true;
            }
            droppable -= 1;
            false
        });
        self.indexes = self
            .downloads
            .iter()
            .enumerate()
            .map(|(index, download)| (download.info.guid.clone(), index))
            .collect();
    }
}

fn is_terminal(state: &DownloadState) -> bool {
    matches!(state, DownloadState::Completed | DownloadState::Canceled)
}

#[derive(Default)]
pub struct DownloadTracker {
    state: Mutex<DownloadTrackerState>,
}

impl DownloadTracker {
    pub fn cursor(&self) -> u64 {
        self.state.lock().unwrap().sequence
    }

    pub fn begin(
        &self,
        guid: String,
        url: String,
        frame_id: String,
        suggested_filename: String,
        local_path: String,
    ) {
        let mut state = self.state.lock().unwrap();
        state.prune_terminal();
        state.sequence += 1;
        let sequence = state.sequence;
        let index = state.downloads.len();
        state.indexes.insert(guid.clone(), index);
        state.downloads.push(TrackedDownload {
            sequence,
            info: DownloadInfo {
                guid,
                url: mask_text(&url),
                frame_id,
                suggested_filename: mask_text(&suggested_filename),
                local_path,
                received_bytes: 0,
                total_bytes: 0,
                state: DownloadState::InProgress,
                failure_reason: None,
            },
            reported: false,
        });
    }

    pub fn progress(
        &self,
        guid: &str,
        received_bytes: u64,
        total_bytes: u64,
        state: DownloadState,
    ) {
        let mut tracker = self.state.lock().unwrap();
        let Some(index) = tracker.indexes.get(guid).copied() else {
            return;
        };
        let download = &mut tracker.downloads[index].info;
        let already_canceled = download.state == DownloadState::Canceled;
        download.received_bytes = received_bytes;
        download.total_bytes = total_bytes;
        match state {
            DownloadState::Canceled => {
                download
                    .failure_reason
                    .get_or_insert_with(|| CANCELED_REASON.to_string());
            }
            DownloadState::InProgress if already_canceled => return,
            _ => download.failure_reason = None,
        }
        let terminal = is_terminal(&state);
        download.state = state;
        if terminal {
            tracker.prune_terminal();
        }
    }

    pub fn latest_active(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .downloads
            .iter()
            .rev()
            .find(|download| download.info.state == DownloadState::InProgress)
            .map(|download| download.info.guid.clone())
    }

    pub fn mark_canceled(&self, guid: &str, reason: &str) -> Option<DownloadInfo> {
        let mut tracker = self.state.lock().unwrap();
        let index = tracker.indexes.get(guid).copied()?;
        let download = &mut tracker.downloads[index].info;
        download.state = DownloadState::Canceled;
        download.failure_reason = Some(reason.to_string());
        Some(download.clone())
    }

    pub fn completed_after(&self, cursor: u64) -> Option<DownloadInfo> {
        self.state
            .lock()
            .unwrap()
            .downloads
            .iter()
            .find(|download| download.sequence > cursor && is_terminal(&download.info.state))
            .map(|download| download.info.clone())
    }

    pub fn take_report(&self) -> Vec<DownloadInfo> {
        let mut state = self.state.lock().unwrap();
        let downloads = state
            .downloads
            .iter_mut()
            .filter_map(|download| {
                if download.reported || !is_terminal(&download.info.state) {
                    return None;
                }
                download.reported = true;
                Some(download.info.clone())
            })
            .collect();
        downloads
    }

    fn update(&self, info: DownloadInfo) {
        let mut state = self.state.lock().unwrap();
        if let Some(index) = state.indexes.get(&info.guid).copied() {
            state.downloads[index].info = info;
        }
    }
}

pub struct DownloadMonitor {
    transport: Transport,
    events: Mutex<Receiver<Event>>,
    tracker: DownloadTracker,
    directory: PathBuf,
}

impl DownloadMonitor {
    pub fn connect(ws_url: &str, directory: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("Failed to create downloads directory: {error}"))?;
        let url = url::Url::parse(ws_url)
            .map_err(|error| format!("Invalid download CDP URL: {error}"))?;
        let transport = Transport::new(url, None, Duration::from_secs(86_400), None)
            .map_err(|error| format!("Failed to connect download CDP session: {error}"))?;
        let events = Mutex::new(transport.listen_to_browser_events());
        Ok(Self {
            transport,
            events,
            tracker: DownloadTracker::default(),
            directory,
        })
    }

    pub fn configure_context(&self, browser_context_id: Option<&str>) -> Result<(), String> {
        let configure = |context_id: Option<&str>| {
            let mut params = json!({
                "behavior": "allowAndName",
                "downloadPath": self.directory.to_string_lossy(),
                "eventsEnabled": true
            });
            if let Some(context_id) = context_id {
                params["browserContextId"] = Value::String(context_id.to_string());
            }
            self.send_command("Browser.setDownloadBehavior", params)
        };
        match configure(browser_context_id) {
            Err(error)
                if browser_context_id.is_some()
                    && error.contains("Failed to find browser context") =>
            {
                configure(None)
            }
            result => result,
        }
    }

    fn send_command(&self, method: &str, params: Value) -> Result<(), String> {
        self.transport
            .call_method_json(method, params, None)
            .map(|_| ())
            .map_err(|error| format!("Download CDP command failed: {error}"))
    }

    // A command never borrows this receiver or its lock. Only event consumers
    // serialize, and draining is capped so an event flood cannot starve callers.
    fn drain_events(&self) {
        if let Ok(events) = self.events.try_lock() {
            for _ in 0..256 {
                match events.try_recv() {
                    Ok(event) => self.handle_typed_event(event),
                    Err(_) => break,
                }
            }
        }
    }

    fn handle_typed_event(&self, event: Event) {
        let (method, params) = match event {
            Event::BrowserDownloadWillBegin(event) => (
                "Browser.downloadWillBegin",
                serde_json::to_value(event.params),
            ),
            Event::BrowserDownloadProgress(event) => (
                "Browser.downloadProgress",
                serde_json::to_value(event.params),
            ),
            _ => return,
        };
        if let Ok(params) = params {
            self.handle_event(&json!({"method": method, "params": params}));
        }
    }

    pub fn cursor(&self) -> u64 {
        self.drain_events();
        self.tracker.cursor()
    }

    pub fn cancel_download(&self, id: Option<&str>) -> Result<DownloadInfo, String> {
        self.drain_events();
        let guid = match id {
            Some(id) => id.to_string(),
            None => self
                .tracker
                .latest_active()
                .ok_or_else(|| "No download is in progress".to_string())?,
        };
        self.send_command("Browser.cancelDownload", json!({"guid": guid}))?;
        self.tracker
            .mark_canceled(&guid, CANCELED_REASON)
            .ok_or_else(|| format!("Unknown download id {guid}"))
    }

    pub fn wait_for_download(
        &self,
        cursor: u64,
        timeout: Duration,
        save_as: Option<&str>,
    ) -> Result<DownloadInfo, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(download) = self.tracker.completed_after(cursor) {
                if download.state == DownloadState::Canceled {
                    return Ok(download);
                }
                return self.finish_download(download, save_as);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "Timed out waiting for download after {}ms",
                    timeout.as_millis()
                ));
            }
            match self.events.try_lock() {
                Ok(events) => match events.recv_timeout(remaining.min(DOWNLOAD_POLL_INTERVAL)) {
                    Ok(event) => self.handle_typed_event(event),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err("Download CDP session closed".into());
                    }
                },
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(remaining.min(Duration::from_millis(5)))
                }
                Err(_) => return Err("Download event receiver poisoned".into()),
            }
        }
    }

    fn handle_event(&self, value: &Value) {
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            return;
        };
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "Browser.downloadWillBegin" => {
                let guid = string_field(&params, "guid");
                self.tracker.begin(
                    guid.clone(),
                    string_field(&params, "url"),
                    string_field(&params, "frameId"),
                    string_field(&params, "suggestedFilename"),
                    self.directory.join(guid).to_string_lossy().into_owned(),
                );
            }
            "Browser.downloadProgress" => {
                let state = match params.get("state").and_then(Value::as_str) {
                    Some("completed") => DownloadState::Completed,
                    Some("canceled") => DownloadState::Canceled,
                    _ => DownloadState::InProgress,
                };
                self.tracker.progress(
                    &string_field(&params, "guid"),
                    number_field(&params, "receivedBytes"),
                    number_field(&params, "totalBytes"),
                    state,
                );
            }
            _ => {}
        }
    }

    fn finish_download(
        &self,
        mut download: DownloadInfo,
        save_as: Option<&str>,
    ) -> Result<DownloadInfo, String> {
        let source = PathBuf::from(&download.local_path);
        if let Ok(metadata) = source.metadata() {
            download.received_bytes = metadata.len();
            download.total_bytes = download.total_bytes.max(metadata.len());
        }
        if let Some(save_as) = save_as {
            let file_name = Path::new(save_as)
                .file_name()
                .and_then(|value| value.to_str())
                .filter(|value| *value == save_as && !value.is_empty())
                .ok_or_else(|| {
                    "save_as must be a file name inside runtime download storage".to_string()
                })?;
            let target = self.directory.join(file_name);
            std::fs::copy(&source, &target)
                .map_err(|error| format!("Failed to save download as {file_name}: {error}"))?;
            download.local_path = target.to_string_lossy().into_owned();
        }
        self.tracker.update(download.clone());
        Ok(download)
    }

    pub fn take_report(&self) -> Vec<DownloadInfo> {
        self.drain_events();
        self.tracker.take_report()
    }
}

fn string_field(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn number_field(value: &Value, name: &str) -> u64 {
    value
        .get(name)
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .max(0.0) as u64
}

fn mask_text(value: &str) -> String {
    refact_core::string_utils::redact_sensitive(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use tempfile::tempdir;

    fn mock_monitor(
        script: impl FnOnce(tungstenite::WebSocket<std::net::TcpStream>) + Send + 'static,
    ) -> (
        DownloadMonitor,
        tempfile::TempDir,
        std::thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "ws://{}/devtools/browser/test",
            listener.local_addr().unwrap()
        );
        let peer = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            script(tungstenite::accept(stream).unwrap());
        });
        let dir = tempdir().unwrap();
        (
            DownloadMonitor::connect(&url, dir.path().join("downloads")).unwrap(),
            dir,
            peer,
        )
    }

    fn read_call(ws: &mut tungstenite::WebSocket<std::net::TcpStream>) -> Value {
        serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap()
    }
    fn send(ws: &mut tungstenite::WebSocket<std::net::TcpStream>, value: Value) {
        ws.send(tungstenite::Message::text(value.to_string()))
            .unwrap();
    }

    #[test]
    fn download_monitor_configures_events_and_cancels_during_wait_without_stealing_reply() {
        let (monitor, _dir, peer) = mock_monitor(|mut ws| {
            let configure = read_call(&mut ws);
            assert_eq!(configure["method"], "Browser.setDownloadBehavior");
            assert_eq!(configure["params"]["browserContextId"], "context");
            send(
                &mut ws,
                json!({"method":"Browser.downloadWillBegin", "params":{
                "guid":"download", "url":"https://example.test/file", "frameId":"frame", "suggestedFilename":"file.bin"}}),
            );
            send(&mut ws, json!({"id":configure["id"], "result":{}}));
            let cancel = read_call(&mut ws);
            assert_eq!(cancel["method"], "Browser.cancelDownload");
            assert_eq!(cancel["params"]["guid"], "download");
            send(
                &mut ws,
                json!({"method":"Browser.downloadProgress", "params":{
                "guid":"download", "receivedBytes":3, "totalBytes":9, "state":"canceled"}}),
            );
            send(&mut ws, json!({"id":cancel["id"], "result":{}}));
            let _ = ws.read();
        });
        monitor.configure_context(Some("context")).unwrap();
        std::thread::scope(|scope| {
            let waiting =
                scope.spawn(|| monitor.wait_for_download(0, Duration::from_secs(2), None));
            assert_eq!(
                monitor.cancel_download(Some("download")).unwrap().state,
                DownloadState::Canceled
            );
            assert_eq!(
                waiting.join().unwrap().unwrap().state,
                DownloadState::Canceled
            );
        });
        drop(monitor);
        peer.join().unwrap();
    }

    #[test]
    fn download_monitor_deadline_is_bounded_and_uncertain_cancel_is_not_replayed() {
        use headless_chrome::browser::transport::CommandDeadlineGuard;
        let (release, released) = std::sync::mpsc::channel();
        let (monitor, _dir, peer) = mock_monitor(move |mut ws| {
            let call = read_call(&mut ws);
            assert_eq!(call["method"], "Browser.cancelDownload");
            released.recv_timeout(Duration::from_secs(3)).unwrap();
            send(&mut ws, json!({"id":call["id"], "result":{}}));
            let next = read_call(&mut ws);
            assert_eq!(
                next["method"], "Browser.setDownloadBehavior",
                "uncertain cancel replayed"
            );
            send(&mut ws, json!({"id":next["id"], "result":{}}));
            let _ = ws.read();
        });
        let started = Instant::now();
        let guard = CommandDeadlineGuard::new(started + Duration::from_millis(100));
        let error = monitor.cancel_download(Some("download")).unwrap_err();
        drop(guard);
        assert!(
            error.contains("deadline exceeded") && error.contains("effect_unknown=true"),
            "{error}"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
        monitor.configure_context(None).unwrap();
        let started = Instant::now();
        assert!(
            monitor
                .wait_for_download(0, Duration::from_millis(100), None)
                .unwrap_err()
                .contains("Timed out waiting")
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(monitor);
        peer.join().unwrap();
    }

    fn metadata(multiple: bool) -> InputMetadata {
        InputMetadata {
            tag: "input".to_string(),
            input_type: "file".to_string(),
            multiple,
            webkitdirectory: false,
        }
    }

    fn begin_download(tracker: &DownloadTracker, guid: &str) {
        tracker.begin(
            guid.to_string(),
            format!("https://example.test/{guid}"),
            "frame".to_string(),
            format!("{guid}.bin"),
            format!("/tmp/{guid}.bin"),
        );
    }

    fn tracked_guids(tracker: &DownloadTracker) -> Vec<String> {
        tracker
            .state
            .lock()
            .unwrap()
            .downloads
            .iter()
            .map(|download| download.info.guid.clone())
            .collect()
    }

    #[test]
    fn pruning_keeps_active_downloads_and_a_consistent_index_map() {
        let tracker = DownloadTracker::default();
        begin_download(&tracker, "still-running");
        for index in 0..(RETAINED_TERMINAL_DOWNLOADS + 5) {
            let guid = format!("done-{index}");
            begin_download(&tracker, &guid);
            tracker.progress(&guid, 1, 1, DownloadState::Completed);
        }
        let retained = tracked_guids(&tracker);
        assert!(retained.contains(&"still-running".to_string()));
        assert_eq!(tracker.latest_active().as_deref(), Some("still-running"));

        let state = tracker.state.lock().unwrap();
        for (guid, index) in &state.indexes {
            assert_eq!(&state.downloads[*index].info.guid, guid);
        }
        assert_eq!(state.indexes.len(), state.downloads.len());
        drop(state);

        tracker.progress("done-54", 9, 9, DownloadState::Canceled);
        assert_eq!(
            tracker
                .mark_canceled("still-running", "aborted")
                .map(|info| info.guid)
                .as_deref(),
            Some("still-running")
        );
    }

    #[test]
    fn rejects_non_file_input() {
        let mut value = metadata(false);
        value.input_type = "text".to_string();
        assert!(
            validate_input_paths(&value, &[])
                .unwrap_err()
                .contains("requires <input type=\"file\">")
        );
    }

    #[test]
    fn rejects_multiple_files_for_single_input() {
        let dir = tempdir().unwrap();
        let first = dir.path().join("one.txt");
        let second = dir.path().join("two.txt");
        std::fs::write(&first, "one").unwrap();
        std::fs::write(&second, "two").unwrap();
        let paths = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
        ];
        assert_eq!(
            validate_input_paths(&metadata(false), &paths).unwrap_err(),
            "File input does not accept multiple files"
        );
    }

    #[test]
    fn rejects_missing_upload_path() {
        let error =
            validate_input_paths(&metadata(true), &["/missing/refact-upload.txt".to_string()])
                .unwrap_err();
        assert!(error.contains("Upload path does not exist"));
    }

    #[test]
    fn canceled_progress_records_a_failure_reason_and_completion_does_not() {
        let tracker = DownloadTracker::default();
        let cursor = tracker.cursor();
        tracker.begin(
            "a".into(),
            "https://example/a".into(),
            "f".into(),
            "a.txt".into(),
            "/tmp/a".into(),
        );
        tracker.progress("a", 3, 9, DownloadState::InProgress);
        assert!(tracker.completed_after(cursor).is_none());
        tracker.progress("a", 3, 9, DownloadState::Canceled);
        let canceled = tracker.completed_after(cursor).unwrap();
        assert_eq!(canceled.failure_reason.as_deref(), Some(CANCELED_REASON));

        let second = DownloadTracker::default();
        let cursor = second.cursor();
        second.begin(
            "b".into(),
            "https://example/b".into(),
            "f".into(),
            "b.txt".into(),
            "/tmp/b".into(),
        );
        second.progress("b", 9, 9, DownloadState::Completed);
        assert_eq!(second.completed_after(cursor).unwrap().failure_reason, None);
    }

    #[test]
    fn cancel_targets_the_latest_in_progress_download_and_keeps_its_reason() {
        let tracker = DownloadTracker::default();
        assert_eq!(tracker.latest_active(), None);
        tracker.begin(
            "a".into(),
            "https://example/a".into(),
            "f".into(),
            "a.txt".into(),
            "/tmp/a".into(),
        );
        tracker.begin(
            "b".into(),
            "https://example/b".into(),
            "f".into(),
            "b.txt".into(),
            "/tmp/b".into(),
        );
        assert_eq!(tracker.latest_active().as_deref(), Some("b"));

        let canceled = tracker.mark_canceled("b", CANCELED_REASON).unwrap();
        assert_eq!(canceled.state, DownloadState::Canceled);
        assert_eq!(canceled.failure_reason.as_deref(), Some(CANCELED_REASON));
        assert_eq!(tracker.latest_active().as_deref(), Some("a"));
        assert!(tracker.mark_canceled("missing", CANCELED_REASON).is_none());

        tracker.progress("b", 2, 9, DownloadState::Canceled);
        assert_eq!(
            tracker.take_report()[0].failure_reason.as_deref(),
            Some(CANCELED_REASON)
        );
    }
}
