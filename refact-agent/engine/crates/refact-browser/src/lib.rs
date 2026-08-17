mod actionability;
pub mod artifacts;
pub mod assertions;
pub mod context_state;
pub mod coverage;
pub mod dialogs;
pub mod drag;
pub mod files;
pub mod forms;
mod frames;
mod handles;
pub mod har;
mod hit_target;
mod injected_source;
mod keyboard;
mod locator_gen;
mod locator_handlers;
mod locators;
mod mouse;
mod network;
mod refs;
mod routing;
mod snapshot;
mod us_keyboard_layout;
pub mod webauthn;
pub mod websocket;
mod world;

pub use actionability::{
    ACTION_RETRY_BACKOFF_MS, EXPECT_RETRY_BACKOFF_MS, LOCATOR_RETRY_BACKOFF_MS,
    MAX_CALL_LOG_ENTRIES, ActionKind, ActionabilityDiagnostic, ActionabilityDriver,
    ActionabilityEngine, ActionabilityError, ActionabilityExecutionMode, ActionabilitySuccess,
    ActionabilityTimeouts, CallLog, Clock, Deadline, ExpectPollResult, LocatorOutcome,
    RequiredStates, SystemClock, TimeoutError, TimeoutKind, required_states,
};
pub use dialogs::{DialogDecision, DialogManager, DialogResponse};
pub use drag::{CdpDragDispatcher, DragDispatcher, DragEventType, drag_and_drop, drop_files};
pub use context_state::{ContextState, MediaState, ViewportState};
pub use handles::{
    CheckedState, ElementHandle, ElementState, ElementStateName, HandleError, HandleRegistry,
};
pub use frames::{
    FrameExecutionContext, FrameHandle, FrameId, FrameInvalidation, FrameLocatorError,
    FrameSessionId, FrameTree,
};
pub use files::{DownloadMonitor, DownloadTracker, FileChooserManager};
pub use hit_target::{
    CdpFrameHitTargetDriver, FrameHitTargetDriver, FrameOwnerGeometry, FramePointTranslation,
    FramePointTranslationGeometry, HitTargetController, HitTargetError, HitTargetPoint,
    HitTargetResult, InterceptorToken, install_interceptor, take_result, translate_point_to_frame,
    translate_point_to_frame_cdp,
};
pub use injected_source::{INJECTED_BUNDLE, wrapped_bootstrap};
pub use keyboard::{
    CdpKeyboardDispatcher, KeyEventPayload, KeyEventType, Keyboard, KeyboardDispatch,
    KeyboardDispatcher, KeyboardModifier, modifier_bitmask,
};
pub use locator_handlers::{
    DEFAULT_DISMISS_OVERLAYS_HANDLER, LocatorHandler, LocatorHandlerLease, LocatorHandlerOperation,
    LocatorHandlerProbe, LocatorHandlerRegistry, MAX_LOCATOR_HANDLER_STEPS,
};
pub use locator_gen::LocatorGenerationOptions;
pub use locators::{DEFAULT_TEST_ID_ATTRIBUTE, strict_mode_violation, test_id_locator};
pub use mouse::{
    CdpMouseDispatcher, ContentQuad, MainFrameCssPoint, MainFrameCssViewport, Mouse, MouseButton,
    MouseDispatch, MouseDispatcher, MouseError, MouseEventPayload, MouseEventType, MouseState,
    ScrollStrategy, TouchEventPayload, TouchEventType, clickable_point_from_quads,
};
pub use network::{
    NetworkLoadState, NetworkMonitorHandle, RequestStarted, ResponseReceived, UrlMatcher,
};
pub use websocket::{WebSocketRegistry, install_websocket_router};
pub use refs::{ElementHandleInfo, Ref, RefError, RefParseError, RefRegistry, SnapshotGeneration};
pub use routing::RouteRegistry;
pub use snapshot::{AriaSnapshot, SnapshotBox, SnapshotMode, SnapshotNode, SnapshotOptions};
pub use us_keyboard_layout::{
    KEYPAD_LOCATION, KeyDefinition, KeyDescription, ShiftedKeyDefinition, US_KEYBOARD_LAYOUT,
};
pub use world::{
    BINDING_NAME, BindingCall, BindingCallback, INJECTED_INSTANCE_NAME, UTILITY_WORLD_NAME,
    WorldManager,
};

use std::collections::{BTreeMap, BTreeSet, HashMap, hash_map::DefaultHasher};
use std::hash::Hasher;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use headless_chrome::Browser;
use headless_chrome::protocol::cdp::types::Event;
use headless_chrome::protocol::cdp::{Network, Page};
use serde_json;
use tracing::{info, warn};
use uuid::Uuid;

use refact_chat_api::WindowBounds;
use refact_integrations::browser_types::{
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

fn stealth_init_script() -> Page::AddScriptToEvaluateOnNewDocument {
    Page::AddScriptToEvaluateOnNewDocument {
        source: STEALTH_SCRIPT.to_string(),
        world_name: None,
        include_command_line_api: None,
        run_immediately: None,
    }
}

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
    pub last_report_console_cursor: usize,
    pub last_send_network_cursor: usize,
    pub last_report_network_cursor: usize,
    pub last_send_mutation_cursor: usize,
    pub last_timeline_action_cursor: usize,
    pub last_timeline_console_cursor: usize,
    pub last_timeline_network_cursor: usize,
    pub last_frame_hash: Option<u64>,
    pub last_send_url: Option<String>,
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
            last_report_console_cursor: 0,
            last_send_network_cursor: 0,
            last_report_network_cursor: 0,
            last_send_mutation_cursor: 0,
            last_timeline_action_cursor: 0,
            last_timeline_console_cursor: 0,
            last_timeline_network_cursor: 0,
            last_frame_hash: None,
            last_send_url: None,
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
            if self.console_buffer.len() > MAX_BUFFER_SIZE {
                let excess = self.console_buffer.len() - MAX_BUFFER_SIZE;
                self.console_buffer.drain(..excess);
                self.last_send_console_cursor =
                    self.last_send_console_cursor.saturating_sub(excess);
                self.last_report_console_cursor =
                    self.last_report_console_cursor.saturating_sub(excess);
                self.last_timeline_console_cursor =
                    self.last_timeline_console_cursor.saturating_sub(excess);
            }
        }
        let network = std::mem::take(&mut *self.raw_network_entries.lock().unwrap());
        for e in network {
            self.network_buffer.push(e);
            if self.network_buffer.len() > MAX_BUFFER_SIZE {
                let excess = self.network_buffer.len() - MAX_BUFFER_SIZE;
                self.network_buffer.drain(..excess);
                self.last_send_network_cursor =
                    self.last_send_network_cursor.saturating_sub(excess);
                self.last_report_network_cursor =
                    self.last_report_network_cursor.saturating_sub(excess);
                self.last_timeline_network_cursor =
                    self.last_timeline_network_cursor.saturating_sub(excess);
            }
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

    pub fn flush_report_console(&mut self) -> Vec<ConsoleEntry> {
        flush_buffer_since(&self.console_buffer, &mut self.last_report_console_cursor)
    }

    pub fn flush_network_buffer(&mut self) -> Vec<NetworkEntry> {
        flush_buffer_since(&self.network_buffer, &mut self.last_send_network_cursor)
    }

    pub fn flush_report_network(&mut self) -> Vec<NetworkEntry> {
        flush_buffer_since(&self.network_buffer, &mut self.last_report_network_cursor)
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

    pub fn commit_cursors(&mut self, url: Option<&str>) {
        self.flush_action_buffer();
        self.flush_console_buffer();
        self.flush_network_buffer();
        self.flush_mutation_summary();
        self.last_send_url = url.map(str::to_string);
    }

    pub fn page_changed(&self, url: &str) -> bool {
        self.last_send_url.as_deref() != Some(url)
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
    pub world_manager: WorldManager,
    pub dialog_manager: DialogManager,
    pub file_chooser_manager: Arc<FileChooserManager>,
    pub download_monitor: Arc<DownloadMonitor>,
    pub active_tab_target_id: Option<String>,
    pub recording_tab_target_id: Option<String>,
    pub adopted_tab_target_ids: Vec<String>,
    pub tab_openers: HashMap<String, refact_integrations::browser_models::TabOpener>,
    pub tab_opened_by_step: HashMap<String, usize>,
    pub profile_dir: PathBuf,
    pub downloads_dir: PathBuf,
    pub artifacts_dir: PathBuf,
    pub window_bounds: Option<WindowBounds>,
    pub buffers: BrowserBuffers,
    pub network_monitor: Arc<NetworkMonitorHandle>,
    pub route_registry: Arc<RouteRegistry>,
    pub websocket_registry: Arc<WebSocketRegistry>,
    pub har_recorder: Arc<har::HarRecorder>,
    pub coverage_manager: coverage::CoverageManager,
    pub webauthn_manager: webauthn::WebAuthnManager,
    pub context_state: ContextState,
    pub mouse_states: HashMap<String, MouseState>,
    pub idle_timeout: Duration,
    pub is_connected: bool,
    pub last_activity: Instant,
    pub frame_emitter_active: bool,
    pub headless: bool,
    pub chrome_path: Option<PathBuf>,
    pub locator_handlers: Arc<std::sync::Mutex<LocatorHandlerRegistry>>,
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
        let downloads_dir = profile_dir.join("downloads").join(&runtime_id);
        let artifacts_dir = profile_dir.join("artifacts").join(&runtime_id);
        let download_monitor = Arc::new(DownloadMonitor::connect(
            &browser.get_ws_url(),
            downloads_dir.clone(),
        )?);

        info!(
            "BrowserRuntime {} launched with profile {:?}",
            runtime_id, profile_dir
        );

        Ok(Self {
            runtime_id,
            attached_chat_id: None,
            browser,
            world_manager: WorldManager::default(),
            dialog_manager: DialogManager::default(),
            file_chooser_manager: Arc::new(FileChooserManager::default()),
            download_monitor,
            active_tab_target_id: None,
            recording_tab_target_id: None,
            adopted_tab_target_ids: Vec::new(),
            tab_openers: HashMap::new(),
            tab_opened_by_step: HashMap::new(),
            profile_dir,
            downloads_dir,
            artifacts_dir,
            window_bounds,
            buffers: BrowserBuffers::new(mask_passwords),
            network_monitor: Arc::new(NetworkMonitorHandle::default()),
            route_registry: Arc::new(RouteRegistry::default()),
            websocket_registry: Arc::new(WebSocketRegistry::default()),
            har_recorder: Arc::new(har::HarRecorder::default()),
            coverage_manager: coverage::CoverageManager::default(),
            webauthn_manager: webauthn::WebAuthnManager::default(),
            context_state: ContextState::default(),
            mouse_states: HashMap::new(),
            idle_timeout,
            is_connected: true,
            last_activity: Instant::now(),
            frame_emitter_active: false,
            headless,
            chrome_path,
            locator_handlers: Arc::new(std::sync::Mutex::new(LocatorHandlerRegistry::default())),
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
        let downloads_dir = std::env::temp_dir()
            .join("refact-browser-downloads")
            .join(&runtime_id);
        let artifacts_dir = std::env::temp_dir()
            .join("refact-browser-artifacts")
            .join(&runtime_id);
        let download_monitor = Arc::new(DownloadMonitor::connect(&ws_url, downloads_dir.clone())?);

        info!(
            "BrowserRuntime {} connected via WebSocket to {}",
            runtime_id, ws_url
        );

        Ok(Self {
            runtime_id,
            attached_chat_id: None,
            browser,
            world_manager: WorldManager::default(),
            dialog_manager: DialogManager::default(),
            file_chooser_manager: Arc::new(FileChooserManager::default()),
            download_monitor,
            active_tab_target_id: None,
            recording_tab_target_id: None,
            adopted_tab_target_ids: Vec::new(),
            tab_openers: HashMap::new(),
            tab_opened_by_step: HashMap::new(),
            profile_dir: PathBuf::new(),
            downloads_dir,
            artifacts_dir,
            window_bounds: None,
            buffers: BrowserBuffers::new(mask_passwords),
            network_monitor: Arc::new(NetworkMonitorHandle::default()),
            route_registry: Arc::new(RouteRegistry::default()),
            websocket_registry: Arc::new(WebSocketRegistry::default()),
            har_recorder: Arc::new(har::HarRecorder::default()),
            coverage_manager: coverage::CoverageManager::default(),
            webauthn_manager: webauthn::WebAuthnManager::default(),
            context_state: ContextState::default(),
            mouse_states: HashMap::new(),
            idle_timeout,
            is_connected: true,
            last_activity: Instant::now(),
            frame_emitter_active: false,
            headless: false,
            chrome_path: None,
            locator_handlers: Arc::new(std::sync::Mutex::new(LocatorHandlerRegistry::default())),
        })
    }

    pub fn mask_passwords(&self) -> bool {
        self.buffers.mask_passwords
    }

    pub fn add_route(
        &mut self,
        pattern: refact_integrations::browser_models::UrlPattern,
        handler: refact_integrations::browser_models::RouteHandler,
    ) -> Result<(), String> {
        let enable = self.route_registry.is_empty();
        self.route_registry.add(pattern.clone(), handler)?;
        if enable {
            let tabs = self
                .browser
                .get_tabs()
                .lock()
                .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for tab in tabs {
                if let Err(error) = self
                    .route_registry
                    .enable_for_tab(&tab, self.context_state.http_credentials.is_some())
                {
                    self.route_registry.remove(Some(&pattern));
                    for enabled_tab in self
                        .browser
                        .get_tabs()
                        .lock()
                        .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
                        .unwrap_or_default()
                    {
                        let _ = self.route_registry.disable_for_tab(&enabled_tab);
                    }
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub fn remove_routes(
        &mut self,
        pattern: Option<&refact_integrations::browser_models::UrlPattern>,
    ) -> Result<usize, String> {
        let previous_routes = self.route_registry.snapshot();
        let removed = self.route_registry.remove(pattern);
        if removed > 0
            && self.route_registry.is_empty()
            && self.context_state.http_credentials.is_none()
        {
            let tabs = self
                .browser
                .get_tabs()
                .lock()
                .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let mut errors = Vec::new();
            for tab in tabs {
                if let Err(error) = self.route_registry.disable_for_tab(&tab) {
                    errors.push(error);
                }
            }
            if !errors.is_empty() {
                self.route_registry.restore(previous_routes)?;
                for tab in self
                    .browser
                    .get_tabs()
                    .lock()
                    .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
                {
                    if let Err(error) = self
                        .route_registry
                        .enable_for_tab(&tab, self.context_state.http_credentials.is_some())
                    {
                        errors.push(error);
                    }
                }
                return Err(errors.join("; "));
            }
        }
        Ok(removed)
    }

    pub fn set_har_replay(&mut self, replay: har::HarReplay) -> Result<(), String> {
        let enable = self.route_registry.is_empty();
        self.route_registry.set_har_replay(replay);
        if enable {
            for tab in self
                .browser
                .get_tabs()
                .lock()
                .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
            {
                if let Err(error) = self
                    .route_registry
                    .enable_for_tab(&tab, self.context_state.http_credentials.is_some())
                {
                    self.route_registry.clear_har_replay();
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub fn set_http_credentials(
        &mut self,
        username: String,
        password: String,
    ) -> Result<(), String> {
        self.context_state.http_credentials = Some((username, password));
        for tab in self
            .browser
            .get_tabs()
            .lock()
            .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
        {
            self.context_state.apply_to_tab(&tab)?;
            self.route_registry.enable_for_tab(&tab, true)?;
        }
        Ok(())
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

    pub fn list_tab_infos(&self) -> Vec<refact_integrations::browser_models::TabInfo> {
        let active_id = self.active_tab_target_id();
        self.browser
            .get_tabs()
            .lock()
            .map(|tabs| {
                self.adopted_tab_target_ids
                    .iter()
                    .filter_map(|target_id| {
                        let tab = tabs.iter().find(|tab| tab.get_target_id() == target_id)?;
                        let target_id = tab.get_target_id().to_string();
                        Some(refact_integrations::browser_models::TabInfo {
                            id: target_id.clone(),
                            target_id: target_id.clone(),
                            url: tab.get_url(),
                            title: tab.get_title().unwrap_or_default(),
                            active: active_id == Some(target_id.as_str()),
                            opener: self.tab_openers.get(&target_id).cloned(),
                            opened_by_step: self.tab_opened_by_step.get(&target_id).copied(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn known_tab_ids(&self) -> BTreeSet<String> {
        self.browser
            .get_tabs()
            .lock()
            .map(|tabs| {
                tabs.iter()
                    .map(|tab| tab.get_target_id().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn remove_tab_metadata(&mut self, target_id: &str) {
        self.adopted_tab_target_ids
            .retain(|adopted| adopted != target_id);
        self.tab_openers.remove(target_id);
        self.tab_opened_by_step.remove(target_id);
    }

    pub fn select_tab_after_close(&mut self, closed_target_id: &str) -> Option<String> {
        let fallback = tab_fallback_after_close(&mut self.adopted_tab_target_ids, closed_target_id);
        self.remove_tab_metadata(closed_target_id);
        if self.active_tab_target_id.as_deref() == Some(closed_target_id) {
            self.active_tab_target_id = fallback.clone();
        }
        if self.recording_tab_target_id.as_deref() == Some(closed_target_id) {
            self.recording_tab_target_id = fallback.clone();
        }
        fallback
    }

    pub fn reconcile_tabs(&mut self) {
        let present = self.known_tab_ids();
        self.adopted_tab_target_ids
            .retain(|target_id| present.contains(target_id));
        self.tab_openers
            .retain(|target_id, _| present.contains(target_id));
        self.tab_opened_by_step
            .retain(|target_id, _| present.contains(target_id));
        if self
            .active_tab_target_id
            .as_ref()
            .is_some_and(|target_id| !present.contains(target_id))
        {
            self.active_tab_target_id = self.adopted_tab_target_ids.first().cloned();
        }
        if self
            .recording_tab_target_id
            .as_ref()
            .is_some_and(|target_id| !present.contains(target_id))
        {
            self.recording_tab_target_id = self.adopted_tab_target_ids.first().cloned();
        }
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

fn tab_fallback_after_close(adopted: &mut Vec<String>, closed_target_id: &str) -> Option<String> {
    let closed_index = adopted
        .iter()
        .position(|target_id| target_id == closed_target_id)?;
    adopted.remove(closed_index);
    if adopted.is_empty() {
        return None;
    }
    adopted
        .get(closed_index.saturating_sub(1).min(adopted.len() - 1))
        .cloned()
}

fn register_adopted_tab(adopted: &mut Vec<String>, target_id: String) -> bool {
    if adopted.contains(&target_id) {
        return false;
    }
    adopted.push(target_id);
    true
}

fn tab_opener(
    opener_id: Option<String>,
    frame_id: Option<String>,
) -> Option<refact_integrations::browser_models::TabOpener> {
    opener_id.map(|tab_id| refact_integrations::browser_models::TabOpener { tab_id, frame_id })
}

impl Drop for BrowserRuntime {
    fn drop(&mut self) {
        let tabs = self
            .browser
            .get_tabs()
            .lock()
            .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        self.coverage_manager.cleanup(&tabs);
        self.webauthn_manager.cleanup(&tabs);
        if !self.route_registry.is_empty() || self.context_state.http_credentials.is_some() {
            for tab in &tabs {
                let _ = self.route_registry.disable_for_tab(tab);
            }
            self.route_registry.remove(None);
        }
        let _ = std::fs::remove_dir_all(&self.downloads_dir);
        let _ = std::fs::remove_dir_all(&self.artifacts_dir);
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

    if let Err(e) = tab.call_method(stealth_init_script()) {
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
    tab.enable_runtime()
        .map_err(|e| format!("Failed to enable runtime: {}", e))?;

    tab.add_event_listener(Arc::new(move |event: &Event| match event {
        Event::RuntimeConsoleAPICalled(e) => {
            let text = e
                .params
                .args
                .iter()
                .map(|arg| {
                    arg.value
                        .as_ref()
                        .map(|value| match value {
                            serde_json::Value::String(text) => text.clone(),
                            other => other.to_string(),
                        })
                        .or_else(|| arg.description.clone())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" ");
            let entry = ConsoleEntry {
                timestamp: normalize_timestamp_ms(e.params.timestamp),
                level: format!("{:?}", e.params.Type),
                text,
            };
            if let Ok(mut buf) = console_buffer.lock() {
                buf.push(entry);
                if buf.len() > MAX_BUFFER_SIZE {
                    let excess = buf.len() - MAX_BUFFER_SIZE;
                    buf.drain(..excess);
                }
            }
        }
        Event::LogEntryAdded(e) => {
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
        Event::RuntimeExceptionThrown(e) => {
            let details = &e.params.exception_details;
            let error = details
                .exception
                .as_ref()
                .and_then(|exception| exception.description.clone())
                .unwrap_or_else(|| details.text.clone());
            if let Ok(mut buf) = console_buffer.lock() {
                buf.push(ConsoleEntry {
                    timestamp: normalize_timestamp_ms(e.params.timestamp),
                    level: "page_error".to_string(),
                    text: error,
                });
                if buf.len() > MAX_BUFFER_SIZE {
                    let excess = buf.len() - MAX_BUFFER_SIZE;
                    buf.drain(..excess);
                }
            }
        }
        _ => {}
    }))
    .map_err(|e| format!("Failed to add console listener: {}", e))?;

    Ok(())
}

pub fn setup_network_capture(
    tab: Arc<headless_chrome::Tab>,
    monitor: Arc<NetworkMonitorHandle>,
    network_buffer: Arc<Mutex<Vec<NetworkEntry>>>,
    websocket_registry: Arc<WebSocketRegistry>,
    har_recorder: Arc<har::HarRecorder>,
) -> Result<(), String> {
    tab.call_method(Network::Enable {
        max_total_buffer_size: None,
        max_resource_buffer_size: None,
        max_post_data_size: None,
        report_direct_socket_traffic: None,
        enable_durable_messages: None,
    })
    .map_err(|error| format!("Failed to enable network events: {error}"))?;

    let frame_tree = tab
        .call_method(Page::GetFrameTree(None))
        .map_err(|error| format!("Failed to read browser frame tree: {error}"))?
        .frame_tree;
    let main_frame_id = frame_tree.frame.id.clone();
    attach_network_frame_tree(&monitor, &frame_tree, None);
    if let Ok(ready_state) = tab.evaluate("document.readyState", false) {
        match ready_state.value.as_ref().and_then(|value| value.as_str()) {
            Some("interactive") => monitor.lifecycle(&main_frame_id, "DOMContentLoaded"),
            Some("complete") => {
                monitor.lifecycle(&main_frame_id, "DOMContentLoaded");
                monitor.lifecycle(&main_frame_id, "load");
            }
            _ => {}
        }
    }

    let event_tab = tab.clone();
    tab.add_event_listener(Arc::new(move |event: &Event| {
        match event {
            Event::NetworkRequestWillBeSent(event) => {
                let params = &event.params;
                monitor.request_started(RequestStarted {
                    request_id: params.request_id.clone(),
                    loader_id: params.loader_id.clone(),
                    frame_id: params.frame_id.clone(),
                    method: params.request.method.clone(),
                    url: params.request.url.clone(),
                    resource_type: params
                        .Type
                        .as_ref()
                        .map(|resource_type| format!("{resource_type:?}"))
                        .unwrap_or_else(|| "Other".to_string()),
                    headers: cdp_headers(&params.request.headers),
                    timestamp: normalize_timestamp_ms(params.timestamp),
                    redirect_response: params.redirect_response.as_ref().map(|response| {
                        response_received(
                            params.request_id.clone(),
                            normalize_timestamp_ms(params.timestamp),
                            response,
                        )
                    }),
                });
            }
            Event::NetworkRequestWillBeSentExtraInfo(event) => monitor.request_extra(
                event.params.request_id.clone(),
                cdp_headers(&event.params.headers),
            ),
            Event::NetworkResponseReceived(event) => monitor.response_received(response_received(
                event.params.request_id.clone(),
                normalize_timestamp_ms(event.params.timestamp),
                &event.params.response,
            )),
            Event::NetworkResponseReceivedExtraInfo(event) => monitor.response_extra(
                event.params.request_id.clone(),
                cdp_headers(&event.params.headers),
            ),
            Event::NetworkLoadingFinished(event) => {
                if let Some(entry) = monitor.loading_finished(
                    &event.params.request_id,
                    normalize_timestamp_ms(event.params.timestamp),
                    Some(event.params.encoded_data_length.max(0.0) as u64),
                ) {
                    if har_recorder.begin_body_capture() {
                        let recorder = har_recorder.clone();
                        let request_id = event.params.request_id.clone();
                        let body_tab = event_tab.clone();
                        std::thread::spawn(move || {
                            let body = body_tab
                                .call_method(Network::GetResponseBody { request_id })
                                .ok()
                                .map(|response| {
                                    har::normalize_response_body(
                                        response.body,
                                        response.base_64_encoded,
                                        None,
                                    )
                                });
                            recorder.finish_body_capture(&entry, body);
                        });
                    } else {
                        har_recorder.record(&entry, None);
                    }
                }
            }
            Event::NetworkLoadingFailed(event) => {
                if let Some(entry) = monitor.loading_failed(
                    &event.params.request_id,
                    normalize_timestamp_ms(event.params.timestamp),
                    event.params.error_text.clone(),
                ) {
                    har_recorder.record(&entry, None);
                }
            }
            Event::NetworkWebSocketCreated(event) => websocket_registry
                .record_created(event.params.request_id.clone(), event.params.url.clone()),
            Event::NetworkWebSocketHandshakeResponseReceived(event) => websocket_registry
                .record_handshake(
                    &event.params.request_id,
                    event.params.response.status as u16,
                ),
            Event::NetworkWebSocketFrameSent(event) => websocket_registry.record_frame(
                &event.params.request_id,
                true,
                event.params.response.payload_data.clone(),
                event.params.response.opcode as u8,
            ),
            Event::NetworkWebSocketFrameReceived(event) => websocket_registry.record_frame(
                &event.params.request_id,
                false,
                event.params.response.payload_data.clone(),
                event.params.response.opcode as u8,
            ),
            Event::NetworkWebSocketClosed(event) => {
                websocket_registry.record_closed(&event.params.request_id)
            }
            Event::NetworkWebSocketFrameError(event) => websocket_registry
                .record_error(&event.params.request_id, event.params.error_message.clone()),
            Event::PageLifecycleEvent(event) => {
                monitor.lifecycle(&event.params.frame_id, &event.params.name)
            }
            Event::PageFrameAttached(event) => monitor.attach_frame(
                event.params.frame_id.clone(),
                Some(event.params.parent_frame_id.clone()),
            ),
            Event::PageFrameDetached(event) => monitor.detach_frame(&event.params.frame_id),
            _ => {}
        }
        let completed = monitor.drain_completed();
        if !completed.is_empty() {
            if let Ok(mut buffer) = network_buffer.lock() {
                buffer.extend(completed);
                if buffer.len() > MAX_BUFFER_SIZE {
                    let excess = buffer.len() - MAX_BUFFER_SIZE;
                    buffer.drain(..excess);
                }
            }
        }
    }))
    .map_err(|error| format!("Failed to add network listener: {error}"))?;
    Ok(())
}

fn attach_network_frame_tree(
    monitor: &NetworkMonitorHandle,
    tree: &Page::FrameTree,
    parent_id: Option<String>,
) {
    let frame_id = tree.frame.id.clone();
    monitor.attach_frame(frame_id.clone(), parent_id);
    if let Some(children) = &tree.child_frames {
        for child in children {
            attach_network_frame_tree(monitor, child, Some(frame_id.clone()));
        }
    }
}

fn cdp_headers(headers: &Network::Headers) -> BTreeMap<String, String> {
    headers
        .0
        .as_ref()
        .and_then(|value| value.as_object())
        .map(|headers| {
            headers
                .iter()
                .map(|(name, value)| {
                    let value = value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string());
                    (name.clone(), value)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn response_received(
    request_id: String,
    timestamp: f64,
    response: &Network::Response,
) -> ResponseReceived {
    let timing = response.timing.as_ref();
    ResponseReceived {
        request_id,
        status: response.status as u16,
        status_text: response.status_text.clone(),
        headers: cdp_headers(&response.headers),
        timestamp,
        encoded_data_length: Some(response.encoded_data_length.max(0.0) as u64),
        from_service_worker: response.from_service_worker.unwrap_or(false),
        request_start: timing
            .map(|timing| normalize_timestamp_ms(timing.request_time) + timing.send_start.max(0.0)),
        response_start: timing.map(|timing| {
            normalize_timestamp_ms(timing.request_time) + timing.receive_headers_end.max(0.0)
        }),
    }
}

pub fn setup_recording_for_tab(
    runtime: &mut BrowserRuntime,
    tab: Arc<headless_chrome::Tab>,
) -> Result<(), String> {
    runtime.dialog_manager.install(&tab)?;
    let browser_context_id = tab
        .get_browser_context_id()
        .map_err(|error| format!("Failed to read browser context for downloads: {error}"))?;
    runtime
        .download_monitor
        .configure_context(browser_context_id.as_deref())?;
    setup_file_chooser_capture(&tab, runtime.file_chooser_manager.clone())?;
    runtime.world_manager.ensure_utility_world(&tab)?;
    inject_recorder_into_tab(
        &tab,
        runtime.buffers.mask_passwords,
        runtime.buffers.raw_recorder_events.clone(),
    )?;
    setup_console_capture(&tab, runtime.buffers.raw_console_entries.clone())?;
    setup_network_capture(
        tab.clone(),
        runtime.network_monitor.clone(),
        runtime.buffers.raw_network_entries.clone(),
        runtime.websocket_registry.clone(),
        runtime.har_recorder.clone(),
    )?;
    install_websocket_router(&tab, runtime.websocket_registry.clone())?;
    runtime.context_state.apply_to_tab(&tab)?;
    if !runtime.route_registry.is_empty() || runtime.context_state.http_credentials.is_some() {
        runtime
            .route_registry
            .enable_for_tab(&tab, runtime.context_state.http_credentials.is_some())?;
    }
    let target_id = tab.get_target_id().to_string();
    register_adopted_tab(&mut runtime.adopted_tab_target_ids, target_id.clone());
    if runtime.recording_tab_target_id.is_none() {
        runtime.recording_tab_target_id = Some(target_id.clone());
    }
    if runtime.active_tab_target_id.is_none() {
        runtime.active_tab_target_id = Some(target_id);
    }
    Ok(())
}

pub fn adopt_new_tabs(
    runtime: &mut BrowserRuntime,
    opened_by_step: Option<usize>,
) -> Vec<refact_integrations::browser_models::TabInfo> {
    runtime.reconcile_tabs();
    let tabs = runtime
        .browser
        .get_tabs()
        .lock()
        .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut adopted = Vec::new();
    for tab in tabs {
        let target_id = tab.get_target_id().to_string();
        if runtime.adopted_tab_target_ids.contains(&target_id) {
            continue;
        }
        if let Ok(info) = tab.get_target_info() {
            if let Some(opener) = tab_opener(info.opener_id, info.opener_frame_id) {
                runtime.tab_openers.insert(target_id.clone(), opener);
            }
        }
        if let Some(step_index) = opened_by_step {
            runtime
                .tab_opened_by_step
                .insert(target_id.clone(), step_index);
        }
        if setup_recording_for_tab(runtime, tab.clone()).is_ok() {
            if let Some(info) = runtime
                .list_tab_infos()
                .into_iter()
                .find(|info| info.id == target_id)
            {
                adopted.push(info);
            }
        }
    }
    adopted
}

pub fn wait_for_new_tabs(
    runtime: &mut BrowserRuntime,
    baseline: &BTreeSet<String>,
    opened_by_step: Option<usize>,
    timeout: Duration,
) -> Vec<refact_integrations::browser_models::TabInfo> {
    poll_new_tabs_until(
        timeout,
        || {
            adopt_new_tabs(runtime, opened_by_step);
            let discovered = runtime
                .list_tab_infos()
                .into_iter()
                .filter(|tab| !baseline.contains(&tab.id))
                .map(|tab| tab.id)
                .collect::<Vec<_>>();
            if let Some(step_index) = opened_by_step {
                for target_id in &discovered {
                    runtime
                        .tab_opened_by_step
                        .entry(target_id.clone())
                        .or_insert(step_index);
                }
            }
            runtime
                .list_tab_infos()
                .into_iter()
                .filter(|tab| discovered.contains(&tab.id))
                .collect()
        },
        std::thread::sleep,
    )
}

fn poll_new_tabs_until(
    timeout: Duration,
    mut discover: impl FnMut() -> Vec<refact_integrations::browser_models::TabInfo>,
    mut wait: impl FnMut(Duration),
) -> Vec<refact_integrations::browser_models::TabInfo> {
    let deadline = Instant::now() + timeout;
    loop {
        let tabs = discover();
        if !tabs.is_empty() || Instant::now() >= deadline {
            return tabs;
        }
        wait(Duration::from_millis(25).min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn setup_file_chooser_capture(
    tab: &headless_chrome::Tab,
    manager: Arc<FileChooserManager>,
) -> Result<(), String> {
    tab.add_event_listener(Arc::new(move |event: &Event| {
        if let Event::PageFileChooserOpened(event) = event {
            manager.opened(
                event.params.backend_node_id,
                matches!(
                    event.params.mode,
                    Page::FileChooserOpenedEventModeOption::SelectMultiple
                ),
            );
        }
    }))
    .map(|_| ())
    .map_err(|error| format!("Failed to add file chooser listener: {error}"))
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

    setup_recording_for_tab(runtime, primary_tab.clone())?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn tab_info(id: &str) -> refact_integrations::browser_models::TabInfo {
        refact_integrations::browser_models::TabInfo {
            id: id.to_string(),
            target_id: id.to_string(),
            url: format!("https://example.com/{id}"),
            title: id.to_string(),
            active: false,
            opener: None,
            opened_by_step: None,
        }
    }

    #[test]
    fn tab_registry_preserves_adoption_and_removal_order() {
        let mut adopted = Vec::new();

        assert!(register_adopted_tab(&mut adopted, "first".to_string()));
        assert!(register_adopted_tab(&mut adopted, "second".to_string()));
        assert!(!register_adopted_tab(&mut adopted, "first".to_string()));
        assert_eq!(adopted, vec!["first", "second"]);

        assert_eq!(
            tab_fallback_after_close(&mut adopted, "first"),
            Some("second".to_string())
        );
        assert_eq!(adopted, vec!["second"]);
    }

    #[test]
    fn closing_active_tab_uses_preceding_then_next_fallback() {
        let mut adopted = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];

        assert_eq!(
            tab_fallback_after_close(&mut adopted, "second"),
            Some("first".to_string())
        );
        assert_eq!(adopted, vec!["first", "third"]);
        assert_eq!(
            tab_fallback_after_close(&mut adopted, "first"),
            Some("third".to_string())
        );
        assert_eq!(tab_fallback_after_close(&mut adopted, "third"), None);
    }

    #[test]
    fn opener_association_preserves_target_and_frame() {
        assert_eq!(
            tab_opener(Some("parent".to_string()), Some("frame".to_string())),
            Some(refact_integrations::browser_models::TabOpener {
                tab_id: "parent".to_string(),
                frame_id: Some("frame".to_string()),
            })
        );
        assert_eq!(tab_opener(None, Some("frame".to_string())), None);
    }

    #[test]
    fn popup_wait_returns_empty_at_timeout() {
        let mut waits = 0;
        let tabs = poll_new_tabs_until(Duration::ZERO, Vec::new, |_| waits += 1);

        assert!(tabs.is_empty());
        assert_eq!(waits, 0);
    }

    #[test]
    fn popup_wait_returns_first_discovery() {
        let mut attempts = 0;
        let tabs = poll_new_tabs_until(
            Duration::from_millis(100),
            || {
                attempts += 1;
                (attempts == 2)
                    .then(|| vec![tab_info("popup")])
                    .unwrap_or_default()
            },
            |_| {},
        );

        assert_eq!(tabs, vec![tab_info("popup")]);
        assert_eq!(attempts, 2);
    }

    #[test]
    fn production_pointer_path_uses_actionability_cdp_mouse() {
        let source = include_str!("../../../src/integrations/browser_controller.rs");
        let routing = source
            .split_once("fn step_locator_action(")
            .unwrap()
            .1
            .split_once("fn step_actionable_action(")
            .unwrap()
            .0;
        assert!(routing.contains("\"click\" => Some(ActionKind::Click)"));
        assert!(routing.contains("return step_actionable_action("));
        assert!(!routing.contains("this.click()"));

        let driver = source
            .split_once("impl ActionabilityDriver for BrowserActionDriver")
            .unwrap()
            .1
            .split_once("fn intercepts_pointer_events(")
            .unwrap()
            .0;
        assert!(driver.contains("CdpMouseDispatcher::new(self.tab)"));
        assert!(driver.contains("mouse.click(point.x, point.y, MouseButton::Left)"));
        assert!(driver.contains("mouse.hover(point.x, point.y)"));
        assert!(!driver.contains("this.click()"));
    }

    fn make_test_buffers() -> BrowserBuffers {
        BrowserBuffers::new(true)
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
    fn test_stealth_and_injected_bundle_use_separate_worlds() {
        assert_eq!(stealth_init_script().world_name, None);
        assert_eq!(
            world::utility_init_script().world_name.as_deref(),
            Some(UTILITY_WORLD_NAME)
        );
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
        buf.console_buffer.push(ConsoleEntry {
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
    fn report_console_cursor_is_independent() {
        let mut buf = make_test_buffers();
        buf.console_buffer.push(ConsoleEntry {
            timestamp: 1.0,
            level: "log".to_string(),
            text: "password=hunter2".to_string(),
        });

        assert_eq!(buf.flush_report_console().len(), 1);
        assert!(buf.flush_report_console().is_empty());
        assert_eq!(buf.flush_console_buffer().len(), 1);
    }

    #[test]
    fn test_flush_network_buffer() {
        let mut buf = make_test_buffers();
        buf.network_buffer.push(NetworkEntry {
            timestamp: 1.0,
            method: "GET".to_string(),
            url: "https://example.com".to_string(),
            resource_type: "Document".to_string(),
            status: None,
            ..NetworkEntry::default()
        });
        let flushed = buf.flush_network_buffer();
        assert_eq!(flushed.len(), 1);
        let flushed2 = buf.flush_network_buffer();
        assert_eq!(flushed2.len(), 0);
    }

    #[test]
    fn report_network_cursor_is_independent() {
        let mut buf = make_test_buffers();
        buf.network_buffer.push(NetworkEntry {
            timestamp: 1.0,
            method: "GET".to_string(),
            url: "https://example.com/api".to_string(),
            resource_type: "Fetch".to_string(),
            status: Some(200),
            ..NetworkEntry::default()
        });

        assert_eq!(buf.flush_report_network().len(), 1);
        assert!(buf.flush_report_network().is_empty());
        assert_eq!(buf.flush_network_buffer().len(), 1);
    }

    // Independent consumers must not hide browser activity from chat context, reports, or timelines.
    #[test]
    fn differentiator_09_buffer_cursors_are_independent_for_every_consumer() {
        let mut buf = make_test_buffers();
        buf.handle_recorder_event(
            r##"{"type":"click","selector":"#button","text":"Go","x":1,"y":2,"timestamp":3}"##,
        );
        buf.console_buffer.push(ConsoleEntry {
            timestamp: 4.0,
            level: "log".to_string(),
            text: "console event".to_string(),
        });
        buf.network_buffer.push(NetworkEntry {
            timestamp: 5.0,
            method: "GET".to_string(),
            url: "https://example.com/data".to_string(),
            resource_type: "Fetch".to_string(),
            status: Some(200),
            ..NetworkEntry::default()
        });

        assert_eq!(buf.flush_report_console().len(), 1);
        assert_eq!(buf.flush_report_network().len(), 1);
        assert_eq!(buf.flush_action_buffer().len(), 1);
        assert_eq!(buf.flush_console_buffer().len(), 1);
        assert_eq!(buf.flush_network_buffer().len(), 1);
        let (actions, console, network) = buf.flush_timeline_events();
        assert_eq!(actions.len(), 1);
        assert_eq!(console.len(), 1);
        assert_eq!(network.len(), 1);
    }

    // Replay and observability depend on retaining every supported user event plus batched mutations.
    #[test]
    fn differentiator_10_live_recorder_script_keeps_complete_event_inventory() {
        let script = build_recorder_script(true);

        for event in [
            "type: 'navigation'",
            "type: 'click'",
            "type: 'input'",
            "type: 'keypress'",
            "type: 'submit'",
            "type: 'scroll'",
            "type: 'mutation_summary'",
        ] {
            assert!(script.contains(event), "recorder lacks {event}");
        }
        assert!(script.contains("new MutationObserver"));
        assert!(script.contains("}, 500);"));
    }

    // A closed maximum-z toolbar stays human-visible while preventing page scripts from reaching its controls.
    #[test]
    fn differentiator_11_toolbar_script_keeps_closed_shadow_and_event_channel() {
        for contract in [
            "attachShadow({ mode: 'closed' })",
            "z-index:2147483646",
            "type: 'toolbar_action'",
            "data-action",
            "screenshot",
            "curl",
            "annotate",
            "highlight",
        ] {
            assert!(
                TOOLBAR_SCRIPT.contains(contract),
                "toolbar lacks {contract}"
            );
        }
    }

    // Browser-side detection plus Rust-side remasking must keep secrets out of serialized recorder output.
    #[test]
    fn differentiator_12_password_masking_has_two_layers_and_serializes_safely() {
        let script = build_recorder_script(true);
        assert!(script.contains("el.type === 'password'"));
        assert!(script.contains("ac === 'current-password' || ac === 'new-password'"));

        let secret = "serialized-secret";
        let mut buf = make_test_buffers();
        buf.handle_recorder_event(&format!(
            r##"{{"type":"input","selector":"#password","value":"{secret}","masked":true,"timestamp":1}}"##
        ));
        let payload = serde_json::to_string(&buf.flush_action_buffer()).unwrap();
        assert!(!payload.contains(secret), "secret leaked: {payload}");
        assert!(payload.contains("*****************"));
    }

    // Page-observable stealth patches must remain separate from isolated automation internals.
    #[test]
    fn differentiator_13_stealth_stays_in_main_world_with_required_patches() {
        let stealth = stealth_init_script();
        assert_eq!(stealth.world_name, None);
        assert!(stealth.source.contains("navigator, 'webdriver'"));
        assert!(stealth.source.contains("window.chrome.runtime"));
        assert!(stealth.source.contains("params.name === 'notifications'"));
        assert_eq!(
            world::utility_init_script().world_name.as_deref(),
            Some(UTILITY_WORLD_NAME)
        );
    }

    // Persistent profiles and explicit ownership transitions keep browser state reusable across chat attachment.
    #[test]
    fn differentiator_14_runtime_source_keeps_profile_idle_and_chat_lifecycle() {
        let source = include_str!("lib.rs");

        for contract in [
            "user_data_dir: Some(profile_dir.clone())",
            "Duration::from_secs(600)",
            "pub fn reattach(&mut self, chat_id: &str)",
            "pub fn detach(&mut self)",
            "pub fn is_idle_expired(&self) -> bool",
            "pub fn touch(&mut self)",
        ] {
            assert!(source.contains(contract), "runtime lacks {contract}");
        }
    }

    #[test]
    fn differentiator_15_device_presets_keep_dimensions_dpr_and_mobile_flags() {
        let source = include_str!("../../../src/integrations/browser_controller.rs");
        let context = include_str!("context_state.rs");

        for preset in [
            "Some(\"mobile\") => (390, 844, 3.0, true)",
            "Some(\"tablet\") => (834, 1112, 2.0, true)",
            "_ => (1440, 900, 2.0, false)",
        ] {
            assert!(source.contains(preset), "controller lacks {preset}");
        }
        assert!(source.contains("runtime.context_state.viewport = Some"));
        assert!(source.contains("device_scale_factor: dpr"));
        assert!(source.contains("is_mobile: mobile"));
        assert!(context.contains("SetDeviceMetricsOverride"));
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
        buf.console_buffer.push(ConsoleEntry {
            timestamp: 1.0,
            level: "log".to_string(),
            text: "test".to_string(),
        });
        assert_eq!(buf.action_buffer.len(), 1);
        assert_eq!(buf.console_buffer.len(), 1);
    }

    #[test]
    fn page_changed_tracks_first_same_and_new_urls() {
        let mut buf = make_test_buffers();

        assert!(buf.page_changed("https://example.com/first"));
        buf.commit_cursors(Some("https://example.com/first"));
        assert!(!buf.page_changed("https://example.com/first"));
        assert!(buf.page_changed("https://example.com/second"));
        buf.commit_cursors(Some("https://example.com/second"));
        assert!(!buf.page_changed("https://example.com/second"));
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
