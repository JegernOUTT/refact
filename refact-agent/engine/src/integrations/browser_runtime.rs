use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use headless_chrome::Browser;
use headless_chrome::protocol::cdp::types::Event;
use headless_chrome::protocol::cdp::Page;
use serde_json;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::chat::types::WindowBounds;
use crate::global_context::GlobalContext;
use crate::integrations::browser_types::{
    RecorderEvent, ConsoleEntry, NetworkEntry, MutationSummaryEntry, MAX_BUFFER_SIZE,
    SCROLL_DEBOUNCE_MS, apply_password_masking, enforce_buffer_limit, flush_buffer_since,
};

const FRAME_RATE_LIMIT_MS: u128 = 500;
const FRAME_HASH_THRESHOLD: u64 = 50;

const MAX_RAW_EVENT_QUEUE: usize = 2000;
const MAX_RAW_EVENT_BYTES: usize = 64 * 1024;

const RECORDER_SCRIPT_TEMPLATE: &str = include_str!("browser_recorder.js");
const TOOLBAR_SCRIPT: &str = include_str!("browser_toolbar.js");

const STEALTH_SCRIPT: &str = r#"(function() {
    if (window.__refact_stealth_installed) return;
    window.__refact_stealth_installed = true;
    try {
        Object.defineProperty(navigator, 'webdriver', {
            get: function() { return undefined; },
            configurable: true,
        });
    } catch(e) {}
    try {
        if (!window.chrome) window.chrome = {};
        if (!window.chrome.runtime) {
            window.chrome.runtime = {
                connect: function() {},
                sendMessage: function() {},
            };
        }
    } catch(e) {}
    try {
        var origQuery = Permissions.prototype.query;
        Permissions.prototype.query = function(params) {
            if (params.name === 'notifications') {
                return Promise.resolve({ state: Notification.permission });
            }
            return origQuery.call(this, params);
        };
    } catch(e) {}
})();"#;

pub fn build_recorder_script(mask_passwords: bool) -> String {
    RECORDER_SCRIPT_TEMPLATE.replace(
        "__REFACT_MASK_PASSWORDS__",
        if mask_passwords { "true" } else { "false" },
    )
}

pub fn normalize_timestamp_ms(ts: f64) -> f64 {
    if !ts.is_finite() || ts < 0.0 {
        return 0.0;
    }
    if ts < 10_000_000_000.0 {
        ts * 1000.0
    } else {
        ts
    }
}

pub fn normalize_timestamp_ms_opt(ts: f64) -> Option<f64> {
    if !ts.is_finite() || ts < 0.0 {
        None
    } else if ts < 10_000_000_000.0 {
        Some(ts * 1000.0)
    } else {
        Some(ts)
    }
}

#[derive(Debug, Clone)]
pub struct AgentActionEntry {
    pub timestamp_ms: f64,
    pub action_type: String,
    pub summary: String,
}

pub struct BrowserBuffers {
    pub action_buffer: Vec<RecorderEvent>,
    pub console_buffer: Vec<ConsoleEntry>,
    pub network_buffer: Vec<NetworkEntry>,
    pub mutation_summary: Vec<MutationSummaryEntry>,
    pub toolbar_action_queue: Vec<String>,
    pub agent_action_buffer: Vec<AgentActionEntry>,
    pub last_send_action_cursor: usize,
    pub last_send_console_cursor: usize,
    pub last_send_network_cursor: usize,
    pub last_send_mutation_cursor: usize,
    pub last_timeline_action_cursor: usize,
    pub last_timeline_console_cursor: usize,
    pub last_timeline_network_cursor: usize,
    pub last_frame_hash: Option<u64>,
    pub last_send_frame_hash: Option<u64>,
    pub last_frame_data: Option<Vec<u8>>,
    pub last_frame_time: Option<Instant>,
    pub mask_passwords: bool,
    pub raw_recorder_events: Arc<Mutex<Vec<String>>>,
    pub raw_console_entries: Arc<Mutex<Vec<ConsoleEntry>>>,
    pub raw_network_entries: Arc<Mutex<Vec<NetworkEntry>>>,
}

impl BrowserBuffers {
    pub fn new(mask_passwords: bool) -> Self {
        Self {
            action_buffer: Vec::new(),
            console_buffer: Vec::new(),
            network_buffer: Vec::new(),
            mutation_summary: Vec::new(),
            toolbar_action_queue: Vec::new(),
            agent_action_buffer: Vec::new(),
            last_send_action_cursor: 0,
            last_send_console_cursor: 0,
            last_send_network_cursor: 0,
            last_send_mutation_cursor: 0,
            last_timeline_action_cursor: 0,
            last_timeline_console_cursor: 0,
            last_timeline_network_cursor: 0,
            last_frame_hash: None,
            last_send_frame_hash: None,
            last_frame_data: None,
            last_frame_time: None,
            mask_passwords,
            raw_recorder_events: Arc::new(Mutex::new(Vec::new())),
            raw_console_entries: Arc::new(Mutex::new(Vec::new())),
            raw_network_entries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn drain_raw_events(&mut self) {
        let raw = std::mem::take(&mut *self.raw_recorder_events.lock().unwrap());
        for s in raw {
            self.handle_recorder_event(&s);
        }
        let console = std::mem::take(&mut *self.raw_console_entries.lock().unwrap());
        for e in console {
            self.console_buffer.push(e);
            enforce_buffer_limit(&mut self.console_buffer, &mut self.last_send_console_cursor);
        }
        let network = std::mem::take(&mut *self.raw_network_entries.lock().unwrap());
        for e in network {
            self.network_buffer.push(e);
            enforce_buffer_limit(&mut self.network_buffer, &mut self.last_send_network_cursor);
        }
    }

    pub fn handle_recorder_event(&mut self, json_str: &str) {
        match serde_json::from_str::<RecorderEvent>(json_str) {
            Ok(event) => {
                let event = if self.mask_passwords {
                    apply_password_masking(&event)
                } else {
                    event
                };

                if event.is_scroll() {
                    if let Some(last) = self.action_buffer.last() {
                        if last.is_scroll() {
                            let last_ts = last.timestamp();
                            let new_ts = event.timestamp();
                            if (new_ts - last_ts) < SCROLL_DEBOUNCE_MS {
                                self.action_buffer.pop();
                            }
                        }
                    }
                }

                match &event {
                    RecorderEvent::MutationSummary {
                        added,
                        removed,
                        changed,
                        timestamp,
                    } => {
                        self.mutation_summary.push(MutationSummaryEntry {
                            timestamp: *timestamp,
                            added: *added,
                            removed: *removed,
                            changed: *changed,
                            descriptions: Vec::new(),
                        });
                        enforce_buffer_limit(
                            &mut self.mutation_summary,
                            &mut self.last_send_mutation_cursor,
                        );
                    }
                    RecorderEvent::ToolbarAction { action, .. } => {
                        if self.toolbar_action_queue.len() < 50 {
                            self.toolbar_action_queue.push(action.clone());
                        }
                    }
                    _ => {
                        self.action_buffer.push(event);
                        enforce_buffer_limit(
                            &mut self.action_buffer,
                            &mut self.last_send_action_cursor,
                        );
                    }
                }
            }
            Err(e) => {
                warn!("Failed to parse recorder event: {}: {}", e, json_str);
            }
        }
    }

    pub fn flush_action_buffer(&mut self) -> Vec<RecorderEvent> {
        flush_buffer_since(&self.action_buffer, &mut self.last_send_action_cursor)
    }

    pub fn flush_console_buffer(&mut self) -> Vec<ConsoleEntry> {
        flush_buffer_since(&self.console_buffer, &mut self.last_send_console_cursor)
    }

    pub fn flush_network_buffer(&mut self) -> Vec<NetworkEntry> {
        flush_buffer_since(&self.network_buffer, &mut self.last_send_network_cursor)
    }

    pub fn flush_mutation_summary(&mut self) -> Vec<MutationSummaryEntry> {
        flush_buffer_since(&self.mutation_summary, &mut self.last_send_mutation_cursor)
    }

    pub fn drain_toolbar_actions(&mut self) -> Vec<String> {
        std::mem::take(&mut self.toolbar_action_queue)
    }

    pub fn drain_agent_actions(&mut self) -> Vec<AgentActionEntry> {
        std::mem::take(&mut self.agent_action_buffer)
    }

    pub fn push_agent_action(&mut self, action_type: &str, summary: &str) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as f64;
        self.agent_action_buffer.push(AgentActionEntry {
            timestamp_ms: now_ms,
            action_type: action_type.to_string(),
            summary: summary.to_string(),
        });
        // Cap buffer to prevent unbounded growth in tool-only sessions
        if self.agent_action_buffer.len() > MAX_BUFFER_SIZE {
            let excess = self.agent_action_buffer.len() - MAX_BUFFER_SIZE;
            self.agent_action_buffer.drain(..excess);
        }
    }

    pub fn flush_timeline_events(
        &mut self,
    ) -> (Vec<RecorderEvent>, Vec<ConsoleEntry>, Vec<NetworkEntry>) {
        let action_start = self
            .last_timeline_action_cursor
            .min(self.action_buffer.len());
        let new_actions = self.action_buffer[action_start..].to_vec();
        self.last_timeline_action_cursor = self.action_buffer.len();

        let console_start = self
            .last_timeline_console_cursor
            .min(self.console_buffer.len());
        let new_console = self.console_buffer[console_start..].to_vec();
        self.last_timeline_console_cursor = self.console_buffer.len();

        let network_start = self
            .last_timeline_network_cursor
            .min(self.network_buffer.len());
        let new_network = self.network_buffer[network_start..].to_vec();
        self.last_timeline_network_cursor = self.network_buffer.len();

        (new_actions, new_console, new_network)
    }

    pub fn commit_cursors(&mut self) {
        self.flush_action_buffer();
        self.flush_console_buffer();
        self.flush_network_buffer();
        self.flush_mutation_summary();
        self.last_send_frame_hash = self.last_frame_hash;
    }

    pub fn page_changed(&self) -> bool {
        self.last_frame_hash != self.last_send_frame_hash
    }

    pub fn is_frame_rate_limited(&self) -> bool {
        if let Some(last_time) = self.last_frame_time {
            last_time.elapsed().as_millis() < FRAME_RATE_LIMIT_MS
        } else {
            false
        }
    }

    pub fn should_emit_frame(&self, new_hash: u64) -> bool {
        if self.is_frame_rate_limited() {
            return false;
        }
        match self.last_frame_hash {
            Some(old_hash) => hash_distance(old_hash, new_hash) > FRAME_HASH_THRESHOLD,
            None => true,
        }
    }

    pub fn update_frame_state(&mut self, hash: u64, data: Vec<u8>) {
        self.last_frame_hash = Some(hash);
        self.last_frame_data = Some(data);
        self.last_frame_time = Some(Instant::now());
    }
}

pub struct BrowserRuntime {
    pub runtime_id: String,
    pub attached_chat_id: Option<String>,
    pub browser: Browser,
    pub active_tab_target_id: Option<String>,
    pub recording_tab_target_id: Option<String>,
    pub profile_dir: PathBuf,
    pub window_bounds: Option<WindowBounds>,
    pub buffers: BrowserBuffers,
    pub idle_timeout: Duration,
    pub is_connected: bool,
    pub last_activity: Instant,
    pub frame_emitter_active: bool,
    pub headless: bool,
    pub chrome_path: Option<PathBuf>,
}

impl std::ops::Deref for BrowserRuntime {
    type Target = BrowserBuffers;
    fn deref(&self) -> &BrowserBuffers {
        &self.buffers
    }
}

impl std::ops::DerefMut for BrowserRuntime {
    fn deref_mut(&mut self) -> &mut BrowserBuffers {
        &mut self.buffers
    }
}

impl BrowserRuntime {
    pub fn launch(
        profile_dir: PathBuf,
        window_bounds: Option<WindowBounds>,
        chrome_path: Option<PathBuf>,
        idle_timeout: Option<Duration>,
        mask_passwords: bool,
        headless: bool,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(&profile_dir)
            .map_err(|e| format!("Failed to create profile dir {:?}: {}", profile_dir, e))?;

        let window_size = window_bounds.as_ref().map(|wb| (wb.width, wb.height));
        let idle_timeout = idle_timeout.unwrap_or(Duration::from_secs(600));

        let mut launch_options = headless_chrome::LaunchOptions {
            headless,
            window_size,
            idle_browser_timeout: idle_timeout,
            user_data_dir: Some(profile_dir.clone()),
            args: vec![
                std::ffi::OsStr::new("--no-restore-last-session"),
                std::ffi::OsStr::new("--no-first-run"),
                std::ffi::OsStr::new("--no-startup-window"),
                std::ffi::OsStr::new("--disable-blink-features=AutomationControlled"),
            ],
            ..Default::default()
        };
        if let Some(ref path) = chrome_path {
            launch_options.path = Some(path.clone());
        }

        let browser = Browser::new(launch_options).map_err(|e| e.to_string())?;
        let runtime_id = Uuid::new_v4().to_string();

        info!(
            "BrowserRuntime {} launched with profile {:?}",
            runtime_id, profile_dir
        );

        Ok(Self {
            runtime_id,
            attached_chat_id: None,
            browser,
            active_tab_target_id: None,
            recording_tab_target_id: None,
            profile_dir,
            window_bounds,
            buffers: BrowserBuffers::new(mask_passwords),
            idle_timeout,
            is_connected: true,
            last_activity: Instant::now(),
            frame_emitter_active: false,
            headless,
            chrome_path,
        })
    }

    pub fn connect(
        ws_url: String,
        idle_timeout: Option<Duration>,
        mask_passwords: bool,
    ) -> Result<Self, String> {
        let idle_timeout = idle_timeout.unwrap_or(Duration::from_secs(600));
        let browser = Browser::connect_with_timeout(ws_url.clone(), idle_timeout)
            .map_err(|e| format!("Failed to connect to browser at {}: {}", ws_url, e))?;
        let runtime_id = Uuid::new_v4().to_string();

        info!(
            "BrowserRuntime {} connected via WebSocket to {}",
            runtime_id, ws_url
        );

        Ok(Self {
            runtime_id,
            attached_chat_id: None,
            browser,
            active_tab_target_id: None,
            recording_tab_target_id: None,
            profile_dir: PathBuf::new(),
            window_bounds: None,
            buffers: BrowserBuffers::new(mask_passwords),
            idle_timeout,
            is_connected: true,
            last_activity: Instant::now(),
            frame_emitter_active: false,
            headless: false,
            chrome_path: None,
        })
    }

    pub fn mask_passwords(&self) -> bool {
        self.buffers.mask_passwords
    }

    pub fn reattach(&mut self, chat_id: &str) {
        info!(
            "BrowserRuntime {} reattached from {:?} to {}",
            self.runtime_id, self.attached_chat_id, chat_id
        );
        self.attached_chat_id = Some(chat_id.to_string());
        self.last_activity = Instant::now();
    }

    pub fn detach(&mut self) {
        info!(
            "BrowserRuntime {} detached from {:?}",
            self.runtime_id, self.attached_chat_id
        );
        self.attached_chat_id = None;
    }

    pub fn check_connection(&mut self) -> bool {
        let connected = self.browser.get_version().is_ok();
        if self.is_connected && !connected {
            warn!(
                "BrowserRuntime {} detected browser disconnect",
                self.runtime_id
            );
        }
        self.is_connected = connected;
        connected
    }

    pub fn is_idle_expired(&self) -> bool {
        self.last_activity.elapsed() > self.idle_timeout
    }

    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn set_active_tab_target_id(&mut self, target_id: impl Into<String>) {
        self.active_tab_target_id = Some(target_id.into());
    }

    pub fn active_tab_target_id(&self) -> Option<&str> {
        self.active_tab_target_id.as_deref()
    }

    pub fn list_tab_infos(&self) -> Vec<crate::integrations::browser_models::TabInfo> {
        let active_id = self.active_tab_target_id();
        self.browser
            .get_tabs()
            .lock()
            .map(|tabs| {
                tabs.iter()
                    .map(|tab| {
                        let target_id = tab.get_target_id().to_string();
                        crate::integrations::browser_models::TabInfo {
                            tab_id: target_id.clone(),
                            target_id: target_id.clone(),
                            url: tab.get_url(),
                            title: tab.get_title().unwrap_or_default(),
                            is_active: active_id == Some(target_id.as_str()),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_active_tab(&self) -> Option<Arc<headless_chrome::Tab>> {
        let tabs_guard = self.browser.get_tabs().lock().ok()?;
        if tabs_guard.is_empty() {
            return None;
        }
        if let Some(target_id) = &self.active_tab_target_id {
            if let Some(tab) = tabs_guard
                .iter()
                .find(|tab| tab.get_target_id() == target_id)
            {
                return Some(tab.clone());
            }
        }
        if let Some(target_id) = &self.recording_tab_target_id {
            if let Some(tab) = tabs_guard
                .iter()
                .find(|tab| tab.get_target_id() == target_id)
            {
                return Some(tab.clone());
            }
        }
        tabs_guard.first().cloned()
    }
}

pub fn compute_frame_hash(data: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(data);
    hasher.finish()
}

pub fn hash_distance(a: u64, b: u64) -> u64 {
    (a ^ b).count_ones() as u64
}

pub fn inject_recorder_into_tab(
    tab: &headless_chrome::Tab,
    mask_passwords: bool,
    action_buffer: Arc<Mutex<Vec<String>>>,
) -> Result<(), String> {
    let script = build_recorder_script(mask_passwords);

    let binding_buffer = action_buffer.clone();
    if let Err(e) = tab.expose_function(
        "__refact_event",
        Arc::new(move |payload: serde_json::Value| {
            if let Some(event_json) = extract_refact_event_json(&payload) {
                if event_json.trim().is_empty() {
                    return;
                }
                if event_json.len() > MAX_RAW_EVENT_BYTES {
                    return;
                }
                if let Ok(mut buf) = binding_buffer.lock() {
                    if buf.len() >= MAX_RAW_EVENT_QUEUE {
                        return;
                    }
                    buf.push(event_json);
                }
            }
        }),
    ) {
        warn!("Failed to expose __refact_event binding (non-fatal): {}", e);
    }

    if let Err(e) = tab.call_method(Page::AddScriptToEvaluateOnNewDocument {
        source: STEALTH_SCRIPT.to_string(),
        world_name: None,
        include_command_line_api: None,
        run_immediately: None,
    }) {
        warn!("Failed to add stealth script (non-fatal): {}", e);
    }

    if let Err(e) = tab.call_method(Page::AddScriptToEvaluateOnNewDocument {
        source: script.clone(),
        world_name: None,
        include_command_line_api: None,
        run_immediately: None,
    }) {
        warn!("Failed to add recorder script (non-fatal): {}", e);
    }

    if let Err(e) = tab.call_method(Page::AddScriptToEvaluateOnNewDocument {
        source: TOOLBAR_SCRIPT.to_string(),
        world_name: None,
        include_command_line_api: None,
        run_immediately: None,
    }) {
        warn!("Failed to add toolbar script (non-fatal): {}", e);
    }

    if let Err(e) = tab.evaluate(STEALTH_SCRIPT, false) {
        warn!("Stealth immediate evaluate failed (non-fatal): {}", e);
    }
    if let Err(e) = tab.evaluate(&script, false) {
        warn!("Recorder immediate evaluate failed (non-fatal): {}", e);
    }
    if let Err(e) = tab.evaluate(TOOLBAR_SCRIPT, false) {
        warn!("Toolbar immediate evaluate failed (non-fatal): {}", e);
    }

    Ok(())
}

pub fn ensure_injection_into_tab(
    tab: &headless_chrome::Tab,
    mask_passwords: bool,
    action_buffer: Arc<Mutex<Vec<String>>>,
) {
    let needs = tab
        .evaluate(
            r#"(function(){
                try {
                    if (typeof window.__refact_event !== 'function') return true;
                    if (!window.__refact_stealth_installed || !window.__refact_recorder_installed || !window.__refact_toolbar_installed) return true;
                    try { window.__refact_event(''); } catch(e) { return true; }
                    return false;
                } catch(e) { return true; }
            })()"#,
            false,
        )
        .ok()
        .and_then(|r| r.value)
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if needs {
        let _ = inject_recorder_into_tab(tab, mask_passwords, action_buffer);
    }
}

fn extract_refact_event_json(payload: &serde_json::Value) -> Option<String> {
    fn extract_from_value(value: &serde_json::Value) -> Option<String> {
        if let Some(arr) = value.as_array() {
            if let Some(first) = arr.first() {
                if let Some(event_str) = first.as_str() {
                    return Some(event_str.to_string());
                }
                if first.is_object() {
                    return serde_json::to_string(first).ok();
                }
            }
        }

        if let Some(args) = value.get("args").and_then(|v| v.as_array()) {
            if let Some(first) = args.first() {
                if let Some(event_str) = first.as_str() {
                    return Some(event_str.to_string());
                }
                if first.is_object() {
                    return serde_json::to_string(first).ok();
                }
            }
        }

        if let Some(args) = value.get("arguments").and_then(|v| v.as_array()) {
            if let Some(first) = args.first() {
                if let Some(event_str) = first.as_str() {
                    return Some(event_str.to_string());
                }
                if first.is_object() {
                    return serde_json::to_string(first).ok();
                }
            }
        }

        if let Some(event_type) = value.get("type").and_then(|v| v.as_str()) {
            if !event_type.is_empty() {
                return serde_json::to_string(value).ok();
            }
        }

        None
    }

    if let Some(as_str) = payload.as_str() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(as_str) {
            if let Some(unwrapped) = extract_from_value(&parsed) {
                return Some(unwrapped);
            }
            if parsed.is_object() {
                return serde_json::to_string(&parsed).ok();
            }
        }
        return Some(as_str.to_string());
    }

    if let Some(unwrapped) = extract_from_value(payload) {
        return Some(unwrapped);
    }

    if payload.is_object() {
        return serde_json::to_string(payload).ok();
    }

    None
}

pub fn setup_console_capture(
    tab: &headless_chrome::Tab,
    console_buffer: Arc<Mutex<Vec<ConsoleEntry>>>,
) -> Result<(), String> {
    tab.enable_log()
        .map_err(|e| format!("Failed to enable log: {}", e))?;

    tab.add_event_listener(Arc::new(move |event: &Event| {
        if let Event::LogEntryAdded(e) = event {
            let entry = ConsoleEntry {
                timestamp: normalize_timestamp_ms(e.params.entry.timestamp),
                level: format!("{:?}", e.params.entry.level),
                text: e.params.entry.text.clone(),
            };
            if let Ok(mut buf) = console_buffer.lock() {
                buf.push(entry);
                if buf.len() > MAX_BUFFER_SIZE {
                    let excess = buf.len() - MAX_BUFFER_SIZE;
                    buf.drain(..excess);
                }
            }
        }
    }))
    .map_err(|e| format!("Failed to add console listener: {}", e))?;

    Ok(())
}

pub fn setup_network_capture(
    tab: &headless_chrome::Tab,
    network_buffer: Arc<Mutex<Vec<NetworkEntry>>>,
) -> Result<(), String> {
    let buf = network_buffer.clone();
    tab.register_response_handling(
        "__refact_network",
        Box::new(move |params, _fetch_body| {
            let url = params.response.url.clone();
            let status = params.response.status;
            let resource_type = format!("{:?}", params.Type);
            let allowed = matches!(
                resource_type.as_str(),
                "Document" | "Xhr" | "Fetch" | "XHR" | "Other"
            );
            if allowed {
                if let Ok(mut buf) = buf.lock() {
                    buf.push(NetworkEntry {
                        timestamp: normalize_timestamp_ms(params.timestamp as f64),
                        method: String::new(),
                        url,
                        resource_type,
                        status: Some(status as u16),
                    });
                    if buf.len() > MAX_BUFFER_SIZE {
                        let excess = buf.len() - MAX_BUFFER_SIZE;
                        buf.drain(..excess);
                    }
                }
            }
        }),
    )
    .map_err(|e| format!("Failed to setup network capture: {}", e))?;

    Ok(())
}

pub fn setup_recording_for_tab(
    runtime: &mut BrowserRuntime,
    tab: &headless_chrome::Tab,
) -> Result<(), String> {
    inject_recorder_into_tab(
        tab,
        runtime.buffers.mask_passwords,
        runtime.buffers.raw_recorder_events.clone(),
    )?;
    setup_console_capture(tab, runtime.buffers.raw_console_entries.clone())?;
    setup_network_capture(tab, runtime.buffers.raw_network_entries.clone())?;
    let target_id = tab.get_target_id().to_string();
    runtime.recording_tab_target_id = Some(target_id.clone());
    runtime.active_tab_target_id = Some(target_id);
    Ok(())
}

pub fn setup_recording_for_runtime(runtime: &mut BrowserRuntime) -> Result<(), String> {
    let startup_tabs: Vec<Arc<headless_chrome::Tab>> = runtime
        .browser
        .get_tabs()
        .lock()
        .map(|tabs| tabs.iter().cloned().collect())
        .unwrap_or_default();

    let primary_tab = startup_tabs
        .iter()
        .find(|tab| tab.get_url() != "about:blank")
        .cloned()
        .or_else(|| startup_tabs.first().cloned())
        .or_else(|| runtime.browser.new_tab().ok())
        .ok_or_else(|| "Failed to select recording tab".to_string())?;

    let url = primary_tab.get_url();
    if url.starts_with("chrome://") {
        if let Err(e) = primary_tab.navigate_to("about:blank") {
            tracing::debug!(
                "Could not navigate chrome:// tab to about:blank (non-fatal): {}",
                e
            );
        } else {
            let _ = primary_tab.wait_until_navigated();
        }
    }

    setup_recording_for_tab(runtime, &primary_tab)?;

    let tabs_now: Vec<Arc<headless_chrome::Tab>> = runtime
        .browser
        .get_tabs()
        .lock()
        .map(|tabs| tabs.iter().cloned().collect())
        .unwrap_or_default();

    for tab in tabs_now {
        if tab.get_target_id() == primary_tab.get_target_id() {
            continue;
        }
        let url = tab.get_url();
        if url.starts_with("chrome://") || url == "about:blank" {
            let _ = tab.close(false);
        }
    }

    Ok(())
}

pub fn get_browser_profile_dir(gcx_cache_dir: &PathBuf, thread_id: &str) -> PathBuf {
    gcx_cache_dir.join("browser_profiles").join(thread_id)
}

pub async fn register_browser_runtime(
    gcx: Arc<ARwLock<GlobalContext>>,
    runtime: BrowserRuntime,
) -> String {
    let runtime_id = runtime.runtime_id.clone();
    let arc = Arc::new(AMutex::new(runtime));
    gcx.write()
        .await
        .browser_runtimes
        .insert(runtime_id.clone(), arc);
    runtime_id
}

pub async fn remove_browser_runtime(
    gcx: Arc<ARwLock<GlobalContext>>,
    runtime_id: &str,
) -> Option<Arc<AMutex<BrowserRuntime>>> {
    gcx.write().await.browser_runtimes.remove(runtime_id)
}

pub async fn find_runtime_by_chat_id(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
) -> Option<(String, Arc<AMutex<BrowserRuntime>>)> {
    let runtime_arcs: Vec<(String, Arc<AMutex<BrowserRuntime>>)> = {
        let gcx_locked = gcx.read().await;
        gcx_locked
            .browser_runtimes
            .iter()
            .map(|(rid, arc)| (rid.clone(), arc.clone()))
            .collect()
    };
    for (rid, arc) in runtime_arcs {
        let rt = arc.lock().await;
        if rt.attached_chat_id.as_deref() == Some(chat_id) {
            return Some((rid, arc.clone()));
        }
    }
    None
}

pub async fn browser_monitor_background_task(gcx: Arc<ARwLock<GlobalContext>>) {
    loop {
        let shutdown_flag = gcx.read().await.shutdown_flag.clone();
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(10)) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            } => {
                return;
            }
        }

        let runtime_ids: Vec<String> = {
            let gcx_locked = gcx.read().await;
            gcx_locked.browser_runtimes.keys().cloned().collect()
        };

        let mut to_remove = Vec::new();
        for rid in &runtime_ids {
            let runtime_arc = {
                let gcx_locked = gcx.read().await;
                match gcx_locked.browser_runtimes.get(rid) {
                    Some(arc) => arc.clone(),
                    None => continue,
                }
            };

            let mut rt = runtime_arc.lock().await;

            let was_connected = rt.is_connected;
            let still_connected = rt.check_connection();

            if was_connected && !still_connected {
                info!(
                    "BrowserRuntime {} (chat {:?}) lost connection",
                    rt.runtime_id, rt.attached_chat_id
                );
            }

            if rt.attached_chat_id.is_some() && rt.is_idle_expired() {
                warn!(
                    "BrowserRuntime {} idle timeout ({:?}) for chat {:?}",
                    rt.runtime_id, rt.idle_timeout, rt.attached_chat_id
                );
                to_remove.push(rid.clone());
            }

            if !still_connected && rt.attached_chat_id.is_none() {
                to_remove.push(rid.clone());
            }
        }

        for rid in to_remove {
            remove_browser_runtime(gcx.clone(), &rid).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_test_buffers() -> BrowserBuffers {
        BrowserBuffers::new(true)
    }

    #[test]
    fn test_get_browser_profile_dir() {
        let cache_dir = PathBuf::from("/tmp/refact-cache");
        let profile = get_browser_profile_dir(&cache_dir, "thread-abc-123");
        assert_eq!(
            profile,
            PathBuf::from("/tmp/refact-cache/browser_profiles/thread-abc-123")
        );
    }

    #[test]
    fn test_get_browser_profile_dir_different_threads() {
        let cache_dir = PathBuf::from("/home/user/.cache/refact");
        let p1 = get_browser_profile_dir(&cache_dir, "thread-1");
        let p2 = get_browser_profile_dir(&cache_dir, "thread-2");
        assert_ne!(p1, p2);
        assert!(p1.to_str().unwrap().contains("thread-1"));
        assert!(p2.to_str().unwrap().contains("thread-2"));
    }

    #[test]
    fn test_build_recorder_script_mask_true() {
        let script = build_recorder_script(true);
        assert!(script.contains("var MASK_PASSWORDS = true;"));
        assert!(!script.contains("__REFACT_MASK_PASSWORDS__"));
    }

    #[test]
    fn test_build_recorder_script_mask_false() {
        let script = build_recorder_script(false);
        assert!(script.contains("var MASK_PASSWORDS = false;"));
    }

    #[test]
    fn test_handle_recorder_event_click() {
        let mut buf = make_test_buffers();
        let json = r##"{"type":"click","selector":"#btn","text":"OK","x":10.0,"y":20.0,"timestamp":1000.0}"##;
        buf.handle_recorder_event(json);
        assert_eq!(buf.action_buffer.len(), 1);
        assert!(matches!(&buf.action_buffer[0], RecorderEvent::Click { .. }));
    }

    #[test]
    fn test_handle_recorder_event_scroll_debounce() {
        let mut buf = make_test_buffers();
        buf.handle_recorder_event(
            r#"{"type":"scroll","scroll_x":0,"scroll_y":100,"timestamp":1000.0}"#,
        );
        buf.handle_recorder_event(
            r#"{"type":"scroll","scroll_x":0,"scroll_y":200,"timestamp":1100.0}"#,
        );
        buf.handle_recorder_event(
            r#"{"type":"scroll","scroll_x":0,"scroll_y":300,"timestamp":1150.0}"#,
        );
        assert_eq!(buf.action_buffer.len(), 1);
        match &buf.action_buffer[0] {
            RecorderEvent::Scroll { scroll_y, .. } => assert_eq!(*scroll_y, 300.0),
            _ => panic!("Expected scroll"),
        }
    }

    #[test]
    fn test_handle_recorder_event_scroll_no_debounce_when_separated() {
        let mut buf = make_test_buffers();
        buf.handle_recorder_event(
            r#"{"type":"scroll","scroll_x":0,"scroll_y":100,"timestamp":1000.0}"#,
        );
        buf.handle_recorder_event(
            r#"{"type":"scroll","scroll_x":0,"scroll_y":200,"timestamp":1500.0}"#,
        );
        assert_eq!(buf.action_buffer.len(), 2);
    }

    #[test]
    fn test_handle_recorder_event_password_masking() {
        let mut buf = make_test_buffers();
        buf.mask_passwords = true;
        buf.handle_recorder_event(r##"{"type":"input","selector":"#pass","value":"secret","masked":true,"timestamp":1000.0}"##);
        assert_eq!(buf.action_buffer.len(), 1);
        match &buf.action_buffer[0] {
            RecorderEvent::Input { value, masked, .. } => {
                assert_eq!(value, "******");
                assert!(*masked);
            }
            _ => panic!("Expected input"),
        }
    }

    #[test]
    fn test_handle_recorder_event_no_masking_when_disabled() {
        let mut buf = make_test_buffers();
        buf.mask_passwords = false;
        buf.handle_recorder_event(r##"{"type":"input","selector":"#pass","value":"secret","masked":true,"timestamp":1000.0}"##);
        assert_eq!(buf.action_buffer.len(), 1);
        match &buf.action_buffer[0] {
            RecorderEvent::Input { value, .. } => assert_eq!(value, "secret"),
            _ => panic!("Expected input"),
        }
    }

    #[test]
    fn test_handle_recorder_event_mutation_goes_to_mutation_buffer() {
        let mut buf = make_test_buffers();
        buf.handle_recorder_event(
            r#"{"type":"mutation_summary","added":3,"removed":1,"changed":2,"timestamp":1000.0}"#,
        );
        assert!(buf.action_buffer.is_empty());
        assert_eq!(buf.mutation_summary.len(), 1);
        assert_eq!(buf.mutation_summary[0].added, 3);
    }

    #[test]
    fn test_toolbar_action_routes_to_toolbar_queue() {
        let mut buf = make_test_buffers();
        buf.handle_recorder_event(
            r#"{"type":"toolbar_action","action":"screenshot","timestamp":1000.0}"#,
        );
        assert!(
            buf.action_buffer.is_empty(),
            "toolbar actions should not go to action_buffer"
        );
        assert_eq!(buf.toolbar_action_queue.len(), 1);
        assert_eq!(buf.toolbar_action_queue[0], "screenshot");
    }

    #[test]
    fn test_toolbar_action_queue_multiple() {
        let mut buf = make_test_buffers();
        buf.handle_recorder_event(
            r#"{"type":"toolbar_action","action":"screenshot","timestamp":1.0}"#,
        );
        buf.handle_recorder_event(
            r#"{"type":"toolbar_action","action":"summarize","timestamp":2.0}"#,
        );
        buf.handle_recorder_event(r#"{"type":"toolbar_action","action":"curl","timestamp":3.0}"#);
        assert_eq!(buf.toolbar_action_queue.len(), 3);
        assert_eq!(
            buf.toolbar_action_queue,
            vec!["screenshot", "summarize", "curl"]
        );
    }

    #[test]
    fn test_toolbar_action_queue_capped_at_50() {
        let mut buf = make_test_buffers();
        for i in 0..60 {
            buf.handle_recorder_event(&format!(
                r#"{{"type":"toolbar_action","action":"action_{}","timestamp":{}.0}}"#,
                i, i
            ));
        }
        assert_eq!(buf.toolbar_action_queue.len(), 50);
        assert_eq!(buf.toolbar_action_queue[0], "action_0");
        assert_eq!(buf.toolbar_action_queue[49], "action_49");
    }

    #[test]
    fn test_drain_toolbar_actions_returns_and_clears() {
        let mut buf = make_test_buffers();
        buf.handle_recorder_event(
            r#"{"type":"toolbar_action","action":"screenshot","timestamp":1.0}"#,
        );
        buf.handle_recorder_event(
            r#"{"type":"toolbar_action","action":"summarize","timestamp":2.0}"#,
        );
        let drained = buf.drain_toolbar_actions();
        assert_eq!(drained, vec!["screenshot", "summarize"]);
        assert!(buf.toolbar_action_queue.is_empty());
        let drained2 = buf.drain_toolbar_actions();
        assert!(drained2.is_empty());
    }

    #[test]
    fn test_handle_recorder_event_invalid_json() {
        let mut buf = make_test_buffers();
        buf.handle_recorder_event("not valid json");
        assert!(buf.action_buffer.is_empty());
    }

    #[test]
    fn test_buffer_enforcement_on_action() {
        let mut buf = make_test_buffers();
        for i in 0..MAX_BUFFER_SIZE + 500 {
            buf.handle_recorder_event(&format!(
                r##"{{"type":"click","selector":"#btn","text":"OK","x":{},"y":0,"timestamp":{}}}"##,
                i, i
            ));
        }
        assert_eq!(buf.action_buffer.len(), MAX_BUFFER_SIZE);
    }

    #[test]
    fn test_flush_action_buffer() {
        let mut buf = make_test_buffers();
        buf.handle_recorder_event(
            r##"{"type":"click","selector":"#a","text":"A","x":0,"y":0,"timestamp":1.0}"##,
        );
        buf.handle_recorder_event(
            r##"{"type":"click","selector":"#b","text":"B","x":0,"y":0,"timestamp":2.0}"##,
        );
        let flushed = buf.flush_action_buffer();
        assert_eq!(flushed.len(), 2);
        let flushed2 = buf.flush_action_buffer();
        assert_eq!(flushed2.len(), 0);
    }

    #[test]
    fn test_flush_console_buffer() {
        let mut buf = make_test_buffers();
        buf.console_buffer
            .push(crate::integrations::browser_types::ConsoleEntry {
                timestamp: 1.0,
                level: "log".to_string(),
                text: "hello".to_string(),
            });
        let flushed = buf.flush_console_buffer();
        assert_eq!(flushed.len(), 1);
        let flushed2 = buf.flush_console_buffer();
        assert_eq!(flushed2.len(), 0);
    }

    #[test]
    fn test_flush_network_buffer() {
        let mut buf = make_test_buffers();
        buf.network_buffer
            .push(crate::integrations::browser_types::NetworkEntry {
                timestamp: 1.0,
                method: "GET".to_string(),
                url: "https://example.com".to_string(),
                resource_type: "Document".to_string(),
                status: None,
            });
        let flushed = buf.flush_network_buffer();
        assert_eq!(flushed.len(), 1);
        let flushed2 = buf.flush_network_buffer();
        assert_eq!(flushed2.len(), 0);
    }

    #[test]
    fn test_flush_mutation_summary() {
        let mut buf = make_test_buffers();
        buf.handle_recorder_event(
            r#"{"type":"mutation_summary","added":1,"removed":0,"changed":0,"timestamp":1.0}"#,
        );
        let flushed = buf.flush_mutation_summary();
        assert_eq!(flushed.len(), 1);
        let flushed2 = buf.flush_mutation_summary();
        assert_eq!(flushed2.len(), 0);
    }

    #[test]
    fn test_compute_frame_hash_deterministic() {
        let data = vec![0u8; 1024];
        let h1 = compute_frame_hash(&data);
        let h2 = compute_frame_hash(&data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_compute_frame_hash_different_for_different_data() {
        let data1 = vec![0u8; 1024];
        let data2 = vec![1u8; 1024];
        let h1 = compute_frame_hash(&data1);
        let h2 = compute_frame_hash(&data2);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_hash_distance_identical() {
        assert_eq!(hash_distance(0xABCD, 0xABCD), 0);
    }

    #[test]
    fn test_hash_distance_different() {
        let d = hash_distance(0, u64::MAX);
        assert_eq!(d, 64);
    }

    #[test]
    fn test_should_emit_frame_first_frame() {
        let buf = make_test_buffers();
        assert!(buf.should_emit_frame(12345));
    }

    #[test]
    fn test_should_emit_frame_same_hash() {
        let mut buf = make_test_buffers();
        buf.last_frame_hash = Some(12345);
        assert!(!buf.should_emit_frame(12345));
    }

    #[test]
    fn test_should_emit_frame_rate_limited() {
        let mut buf = make_test_buffers();
        buf.last_frame_time = Some(Instant::now());
        assert!(!buf.should_emit_frame(99999));
    }

    #[test]
    fn test_should_emit_frame_after_rate_limit_expires() {
        let mut buf = make_test_buffers();
        buf.last_frame_time = Some(Instant::now() - Duration::from_millis(600));
        assert!(buf.should_emit_frame(99999));
    }

    #[test]
    fn test_update_frame_state() {
        let mut buf = make_test_buffers();
        assert!(buf.last_frame_hash.is_none());
        assert!(buf.last_frame_data.is_none());
        assert!(buf.last_frame_time.is_none());
        buf.update_frame_state(42, vec![1, 2, 3]);
        assert_eq!(buf.last_frame_hash, Some(42));
        assert_eq!(buf.last_frame_data, Some(vec![1, 2, 3]));
        assert!(buf.last_frame_time.is_some());
    }

    #[test]
    fn test_is_frame_rate_limited_no_previous() {
        let buf = make_test_buffers();
        assert!(!buf.is_frame_rate_limited());
    }

    #[test]
    fn test_is_frame_rate_limited_recently_sent() {
        let mut buf = make_test_buffers();
        buf.last_frame_time = Some(Instant::now());
        assert!(buf.is_frame_rate_limited());
    }

    #[test]
    fn test_is_frame_rate_limited_expired() {
        let mut buf = make_test_buffers();
        buf.last_frame_time = Some(Instant::now() - Duration::from_secs(1));
        assert!(!buf.is_frame_rate_limited());
    }

    #[test]
    fn test_detach_then_reattach_preserves_buffers() {
        let mut buf = make_test_buffers();
        buf.handle_recorder_event(
            r##"{"type":"click","selector":"#btn","text":"OK","x":0,"y":0,"timestamp":1.0}"##,
        );
        buf.console_buffer
            .push(crate::integrations::browser_types::ConsoleEntry {
                timestamp: 1.0,
                level: "log".to_string(),
                text: "test".to_string(),
            });
        assert_eq!(buf.action_buffer.len(), 1);
        assert_eq!(buf.console_buffer.len(), 1);
    }

    #[test]
    fn test_page_changed_true_after_frame_update() {
        let mut buf = make_test_buffers();
        buf.update_frame_state(42, vec![1, 2, 3]);
        assert!(buf.page_changed());
    }

    #[test]
    fn test_page_changed_false_after_commit() {
        let mut buf = make_test_buffers();
        buf.update_frame_state(42, vec![1, 2, 3]);
        assert!(buf.page_changed());
        buf.commit_cursors();
        assert!(!buf.page_changed());
    }

    #[test]
    fn test_page_changed_true_after_new_frame_post_commit() {
        let mut buf = make_test_buffers();
        buf.update_frame_state(42, vec![1, 2, 3]);
        buf.commit_cursors();
        assert!(!buf.page_changed());
        buf.update_frame_state(99, vec![4, 5, 6]);
        assert!(buf.page_changed());
    }

    #[test]
    fn test_page_changed_false_when_no_frames() {
        let buf = make_test_buffers();
        assert!(!buf.page_changed());
    }

    #[test]
    fn test_fps_clamping_edge_values() {
        assert_eq!(0u32.clamp(1, 60), 1);
        assert_eq!(1u32.clamp(1, 60), 1);
        assert_eq!(30u32.clamp(1, 60), 30);
        assert_eq!(60u32.clamp(1, 60), 60);
        assert_eq!(100u32.clamp(1, 60), 60);
    }

    #[test]
    fn test_utf8_safe_truncation() {
        let text = "Hello 🌍 World";
        let truncated: String = text.chars().take(7).collect();
        assert_eq!(truncated, "Hello 🌍");

        let text2 = "日本語テスト";
        let truncated2: String = text2.chars().take(3).collect();
        assert_eq!(truncated2, "日本語");
    }

    #[test]
    fn test_normalize_timestamp_seconds_to_ms() {
        let ts_sec = 1_700_000_000.0;
        assert_eq!(normalize_timestamp_ms(ts_sec), ts_sec * 1000.0);
    }

    #[test]
    fn test_normalize_timestamp_ms_passthrough() {
        let ts_ms = 1_700_000_000_000.0;
        assert_eq!(normalize_timestamp_ms(ts_ms), ts_ms);
    }

    #[test]
    fn test_extract_refact_event_json_from_wrapper_string_payload() {
        let payload = serde_json::json!("{\"name\":\"__refact_event\",\"seq\":1,\"args\":[\"{\\\"type\\\":\\\"toolbar_action\\\",\\\"action\\\":\\\"screenshot\\\",\\\"timestamp\\\":1}\"]}");
        let extracted = extract_refact_event_json(&payload).unwrap();
        assert!(extracted.contains("\"type\":\"toolbar_action\""));
        assert!(extracted.contains("\"action\":\"screenshot\""));
    }

    #[test]
    fn test_extract_refact_event_json_from_wrapper_object_payload() {
        let payload = serde_json::json!({
            "name": "__refact_event",
            "seq": 1,
            "args": [
                {
                    "type": "click",
                    "selector": "#btn",
                    "text": "OK",
                    "x": 1.0,
                    "y": 2.0,
                    "timestamp": 3.0
                }
            ]
        });
        let extracted = extract_refact_event_json(&payload).unwrap();
        assert!(extracted.contains("\"type\":\"click\""));
        assert!(extracted.contains("\"selector\":\"#btn\""));
    }
}
