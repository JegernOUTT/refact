use std::any::Any;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};
use serde_json::Value;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use async_trait::async_trait;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::integrations::sessions::{IntegrationSession, get_session_hashmap_key};

use crate::global_context::GlobalContext;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::scratchpads::multimodality::MultimodalElement;

use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

use crate::integrations::browser_actions::{self, BrowserAction, DeviceType};
use crate::integrations::browser_controller;
use crate::integrations::browser_models::{BrowserActionRequest, ExecutionReport};
use crate::integrations::browser_runtime::{
    BrowserRuntime, find_runtime_by_chat_id, register_browser_runtime, get_browser_profile_dir,
    setup_recording_for_runtime, setup_recording_for_tab,
};

use chrono::DateTime;
use std::path::PathBuf;
use headless_chrome::Tab as HeadlessTab;
use headless_chrome::browser::tab::point::Point;


use headless_chrome::protocol::cdp::Emulation;
use headless_chrome::protocol::cdp::types::Event;

use serde::{Deserialize, Serialize};

use base64::Engine;
use std::io::Cursor;

use image::imageops::FilterType;
use image::{ImageFormat, ImageReader};

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct SettingsChrome {
    pub chrome_path: String,
    #[serde(default)]
    pub idle_browser_timeout: String,
    #[serde(default)]
    pub headless: String,
    // desktop
    #[serde(default)]
    pub window_width: String,
    #[serde(default)]
    pub window_height: String,
    #[serde(default)]
    pub scale_factor: String,
    #[serde(default)]
    // mobile
    pub mobile_window_width: String,
    #[serde(default)]
    pub mobile_window_height: String,
    #[serde(default)]
    pub mobile_scale_factor: String,
    // tablet
    #[serde(default)]
    pub tablet_window_width: String,
    #[serde(default)]
    pub tablet_window_height: String,
    #[serde(default)]
    pub tablet_scale_factor: String,
}

#[derive(Default)]
pub struct ToolChrome {
    pub settings_chrome: SettingsChrome,
    pub supports_clicks: bool,
    pub config_path: String,
}

// DeviceType is now in browser_actions module

const MAX_CACHED_LOG_LINES: usize = 1000;

#[derive(Clone)]
pub struct ChromeTab {
    headless_tab: Arc<HeadlessTab>,
    device: DeviceType,
    tab_id: String,
    screenshot_scale_factor: f64,
    tab_log: Arc<Mutex<Vec<String>>>,
}

impl ChromeTab {
    fn new(headless_tab: Arc<HeadlessTab>, device: &DeviceType, tab_id: &String) -> Self {
        Self {
            headless_tab,
            device: device.clone(),
            tab_id: tab_id.clone(),
            screenshot_scale_factor: 1.0,
            tab_log: Arc::new(Mutex::new(Vec::new())),
        }
    }
    pub fn state_string(&self) -> String {
        format!(
            "tab_id `{}` device `{}` uri `{}`",
            self.tab_id.clone(),
            self.device,
            self.headless_tab.get_url()
        )
    }
}

struct ChromeSession {
    runtime_id: String,
    tabs: HashMap<String, Arc<AMutex<ChromeTab>>>,
    idle_timeout: Duration,
    last_activity: Instant,
}

impl ChromeSession {
    fn touch(&mut self) {
        self.last_activity = Instant::now();
    }
}

impl IntegrationSession for ChromeSession {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn is_expired(&self) -> bool {
        self.last_activity.elapsed() > self.idle_timeout
    }
    fn try_stop(
        &mut self,
        _self_arc: Arc<AMutex<Box<dyn IntegrationSession>>>,
    ) -> Box<dyn Future<Output = String> + Send> {
        // Only detach session tab references — do NOT close actual tabs.
        // Tabs belong to the shared BrowserRuntime and may be used by
        // Browser Mode or other sessions.
        self.tabs.clear();
        // Browser process lifecycle managed by BrowserRuntime
        Box::new(async { "chrome session stopped".to_string() })
    }
}


#[async_trait]
impl Tool for ToolChrome {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.global_context.clone(), ccx_lock.chat_id.clone())
        };

        let session_hashmap_key = get_session_hashmap_key("chrome", &chat_id);
        let mut tool_log =
            setup_chrome_session(gcx.clone(), &self.settings_chrome, &session_hashmap_key, &chat_id).await?;

        let command_session = {
            let gcx_locked = gcx.read().await;
            gcx_locked
                .integration_sessions
                .get(&session_hashmap_key)
                .ok_or(format!(
                    "Error getting chrome session for chat: {}",
                    chat_id
                ))?
                .clone()
        };

        // Touch session to prevent idle expiry during tool execution
        {
            let mut session_locked = command_session.lock().await;
            if let Some(cs) = session_locked.as_any_mut().downcast_mut::<ChromeSession>() {
                cs.touch();
            }
        }

        let mut multimodal_els = vec![];
        let mut typed_content: Option<Vec<MultimodalElement>> = None;

        if let Some(request_value) = args.get("request") {
            let request: BrowserActionRequest = serde_json::from_value(request_value.clone())
                .map_err(|e| format!("argument `request` is invalid: {}", e))?;

            let runtime_id = {
                let mut session_locked = command_session.lock().await;
                let cs = session_locked
                    .as_any_mut()
                    .downcast_mut::<ChromeSession>()
                    .ok_or("Failed to downcast to ChromeSession")?;
                cs.touch();
                cs.runtime_id.clone()
            };
            let runtime_arc = {
                let gcx_locked = gcx.read().await;
                gcx_locked
                    .browser_runtimes
                    .get(&runtime_id)
                    .cloned()
                    .ok_or_else(|| format!(
                        "BrowserRuntime {} not found. Browser may have been closed.",
                        runtime_id
                    ))?
            };

            match browser_controller::execute_request_with_runtime(runtime_arc, request).await {
                Ok(report) => {
                    typed_content = Some(execution_report_to_multimodal(&report)?);
                    let (execute_log, command_multimodal_els) =
                        format_controller_report(&report, "");
                    tool_log.extend(execute_log);
                    multimodal_els.extend(command_multimodal_els);
                }
                Err(e) => {
                    tool_log.push(format!("Failed to execute typed browser request: {}.", e));
                }
            }
        } else {
            let commands_str = match args.get("commands") {
                Some(Value::String(s)) => s,
                Some(v) => return Err(format!("argument `commands` is not a string: {:?}", v)),
                None => {
                    return Err(
                        "Missing argument `request` or compatibility-only legacy `commands`"
                            .to_string(),
                    )
                }
            };

            let parsed_actions = browser_actions::parse_commands(commands_str);
            for (idx, parse_result) in parsed_actions.into_iter().enumerate() {
                let action = match parse_result {
                    Ok(action) => action,
                    Err(e) => {
                        tool_log.push(format!("Failed to parse command #{}: {}.", idx + 1, e));
                        break;
                    }
                };
                match chrome_command_exec(
                    &action,
                    command_session.clone(),
                    gcx.clone(),
                    &self.settings_chrome,
                )
                .await
                {
                    Ok((execute_log, command_multimodal_els)) => {
                        tool_log.extend(execute_log);
                        multimodal_els.extend(command_multimodal_els);
                    }
                    Err(e) => {
                        tool_log.push(format!("Failed to execute command: {}.", e));
                        break;
                    }
                };
            }
        }

        let content = if let Some(typed_content) = typed_content {
            typed_content
        } else {
            let mut content = vec![];
            content.push(MultimodalElement::new(
                "text".to_string(),
                tool_log.join("\n"),
            )?);
            content.extend(multimodal_els);
            content
        };

        let msg = ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::Multimodal(content),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        });

        Ok((false, vec![msg]))
    }

    fn tool_description(&self) -> ToolDesc {
        let mut supported_commands = vec![
            "open_tab <tab_id> <desktop|mobile|tablet>",
            "navigate_to <tab_id> <uri>",
            "scroll_to <tab_id> <element_selector>",
            "screenshot <tab_id>",
            "html <tab_id> <element_selector>",
            "reload <tab_id>",
            "press_key <tab_id> <KeyName> [<Alt|Ctrl|Meta|Shift>,...]",
            "type_text_at <tab_id> <text>",
            "fill_field <tab_id> <selector> <text>",
            "tab_log <tab_id>",
            "eval <tab_id> <expression>",
            "styles <tab_id> <element_selector> [--filter <property_filter>]",
            "wait_for <tab_id> <0.5-10>",
            "click_at_element <tab_id> <element_selector>",
            "wait_for_selector <tab_id> <element_selector>",
            "wait_for_navigation <tab_id>",
            "list_tabs",
            "close_tab <tab_id>",
        ];
        if self.supports_clicks {
            supported_commands.extend(vec!["click_at_point <tab_id> <x> <y>"]);
        }
        let description = format!(
            "A web browser for automation. One or several commands separated by newline. \
             The <tab_id> is an integer, for example 10, for you to identify the tab later. \
             Use wait_for_selector or wait_for_navigation to synchronize with page loading. \
             Supported commands:\n{}\n\
             The `commands` input is compatibility-only; prefer the typed `request` input for new callers.\n\
             Selectors and expressions can contain any characters — no quoting needed for most commands. \
             For `fill_field`, quote the selector when it contains spaces, e.g. `fill_field 1 \"form input[name=q]\" hello`. \
             For multiline expressions use heredoc: eval <tab_id> <<EOF\\n...\\nEOF\nBlank lines and lines starting with // or # are ignored.\n\
             \n\
             Preferred `request` input: pass a JSON object with a `steps` array. Each step is a JSON object \
             with an `action` field (snake_case) and action-specific fields. \
             Example steps: {{\"action\": \"open_tab\", \"device\": \"desktop\"}}, \
             {{\"action\": \"navigate\", \"url\": \"https://example.com\"}}, \
             {{\"action\": \"screenshot\"}}. \
             Locators use a `by` field (css/id/name/text/label/role/xpath/placeholder/autocomplete/test_id) and a `value` field \
             (except role locators which use `role` and optional `name` instead of `value`).",
            supported_commands.join("\n"));
        ToolDesc {
            name: "chrome".to_string(),
            display_name: "Chrome".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "commands": {
                        "type": "string",
                        "description": "Compatibility-only legacy newline-separated browser commands. Prefer `request`.",
                        "deprecated": true
                    },
                    "request": {
                        "type": "object",
                        "description": "Typed browser action request.",
                        "properties": {
                            "session": {"type": "string", "enum": ["shared_default"]},
                            "target": {
                                "type": "object",
                                "properties": {
                                    "type": {"type": "string", "enum": ["active", "id"]},
                                    "id": {"type": "string"}
                                },
                                "required": ["type"]
                            },
                            "steps": {
                                "type": "array",
                                "description": "List of browser steps. Each step must have an 'action' field (snake_case).",
                                "items": {
                                    "type": "object",
                                    "required": ["action"],
                                    "properties": {
                                        "action": {
                                            "type": "string",
                                            "description": "The browser action to perform.",
                                            "enum": [
                                                "navigate", "reload", "go_back", "go_forward",
                                                "open_tab", "close_tab", "switch_tab", "list_tabs",
                                                "click", "click_if_exists", "hover", "focus", "blur", "scroll_to",
                                                "press_key", "fill", "clear", "select_option", "check", "uncheck",
                                                "wait_for_selector", "wait_for_navigation", "wait_for_url", "wait_for_text",
                                                "wait_for_network_idle", "wait_for_element_hidden", "wait_for_element_stable", "wait_seconds",
                                                "get_text", "get_html", "get_attribute", "extract_links", "extract_table",
                                                "dom_snapshot", "accessibility_snapshot", "screenshot", "screenshot_element",
                                                "eval", "styles", "tab_log", "dismiss_overlays", "highlight_element"
                                            ]
                                        },
                                        "url": {"type": "string", "description": "URL for navigate action"},
                                        "device": {"type": "string", "enum": ["desktop", "mobile", "tablet"], "description": "Device type for open_tab"},
                                        "tab": {
                                            "type": "object",
                                            "description": "Tab target for switch_tab",
                                            "properties": {
                                                "type": {"type": "string", "enum": ["active", "id"]},
                                                "id": {"type": "string", "description": "Required when type is 'id'"}
                                            },
                                            "required": ["type"]
                                        },
                                        "locator": {
                                            "type": "object",
                                            "description": "Element locator. Must have 'by' field. For most strategies also include 'value' (e.g. {\"by\":\"css\",\"value\":\"#btn\"}). For 'role' strategy use 'role' and optional 'name' instead of 'value' (e.g. {\"by\":\"role\",\"role\":\"button\",\"name\":\"Submit\"}).",
                                            "required": ["by"],
                                            "properties": {
                                                "by": {"type": "string", "enum": ["css", "id", "name", "text", "label", "role", "xpath", "placeholder", "autocomplete", "test_id"]},
                                                "value": {"type": "string", "description": "Selector value for all strategies except 'role'"},
                                                "nth": {"type": "integer", "description": "0-based index when multiple elements match"},
                                                "within": {"type": "string", "description": "CSS selector to scope the search within"},
                                                "exact": {"type": "boolean", "description": "Exact match for 'text' strategy"},
                                                "role": {"type": "string", "description": "ARIA role name for 'role' strategy"},
                                                "name": {"type": "string", "description": "Accessible name filter for 'role' strategy"}
                                            }
                                        },
                                        "text": {"type": "string", "description": "Text for fill/wait_for_text actions"},
                                        "key": {"type": "string", "description": "Key name for press_key (e.g. Enter, Tab, Escape)"},
                                        "modifiers": {"type": "array", "items": {"type": "string"}, "description": "Modifiers for press_key: Alt, Ctrl, Meta, Shift"},
                                        "expression": {"type": "string", "description": "JavaScript expression for eval action"},
                                        "selector": {"type": "string", "description": "CSS selector for dom_snapshot action"},
                                        "contains": {"type": "string", "description": "URL substring to match in wait_for_url"},
                                        "value": {"type": "string", "description": "Option value for select_option"},
                                        "attribute": {"type": "string", "description": "Attribute name for get_attribute"},
                                        "seconds": {"type": "number", "description": "Seconds to wait for wait_seconds"},
                                        "timeout_ms": {"type": "integer", "description": "Timeout in milliseconds"},
                                        "limit": {"type": "integer", "description": "Max results for extract_links"},
                                        "clear_first": {"type": "boolean", "description": "Clear field before filling (default true)"},
                                        "verify": {"type": "boolean", "description": "Verify fill result (default true)"},
                                        "max_chars": {"type": "integer", "description": "Max characters for dom_snapshot"},
                                        "property_filter": {"type": "string", "description": "CSS property filter for styles action"}
                                    }
                                }
                            }
                        },
                        "required": ["steps"]
                    }
                }
            }),
            output_schema: None,
            annotations: None,
        }
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

async fn setup_chrome_session(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &SettingsChrome,
    session_hashmap_key: &String,
    chat_id: &str,
) -> Result<Vec<String>, String> {
    let mut setup_log = vec![];

    let session_entry = {
        let gcx_locked = gcx.read().await;
        gcx_locked
            .integration_sessions
            .get(session_hashmap_key)
            .cloned()
    };

    if let Some(session) = session_entry {
        let runtime_id = {
            let mut session_locked = session.lock().await;
            let chrome_session = session_locked
                .as_any_mut()
                .downcast_mut::<ChromeSession>()
                .ok_or("Failed to downcast to ChromeSession")?;
            chrome_session.runtime_id.clone()
        };

        let runtime_healthy = {
            let runtime_arc = {
                let gcx_locked = gcx.read().await;
                gcx_locked.browser_runtimes.get(&runtime_id).cloned()
            };
            if let Some(arc) = runtime_arc {
                let mut rt = arc.lock().await;
                rt.check_connection()
            } else {
                false
            }
        };

        if runtime_healthy {
            return Ok(setup_log);
        } else {
            setup_log.push("Browser session is disconnected. Trying to reconnect.".to_string());
            let mut gcx_locked = gcx.write().await;
            let should_remove = gcx_locked
                .integration_sessions
                .get(session_hashmap_key)
                .map(|current| Arc::ptr_eq(current, &session))
                .unwrap_or(false);
            if should_remove {
                gcx_locked.integration_sessions.remove(session_hashmap_key);
            }
        }
    }

    if let Some((runtime_id, _)) = find_runtime_by_chat_id(gcx.clone(), chat_id).await {
        setup_log.push("Reusing existing browser session.".to_string());
        let idle_browser_timeout = args
            .idle_browser_timeout
            .parse::<u64>()
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(600));
        let command_session: Box<dyn IntegrationSession> = Box::new(ChromeSession {
            runtime_id,
            tabs: HashMap::new(),
            idle_timeout: idle_browser_timeout,
            last_activity: Instant::now(),
        });
        gcx.write().await.integration_sessions.insert(
            session_hashmap_key.clone(),
            Arc::new(AMutex::new(command_session)),
        );
        return Ok(setup_log);
    }

    let idle_browser_timeout = args
        .idle_browser_timeout
        .parse::<u64>()
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(600));

    let runtime = if args.chrome_path.starts_with("ws://") {
        setup_log.push("Connect to existing web socket.".to_string());
        BrowserRuntime::connect(
            args.chrome_path.clone(),
            Some(idle_browser_timeout),
            true,
        )?
    } else if let Some(container_address) = args.chrome_path.strip_prefix("container://") {
        setup_log.push("Connect to chrome from container.".to_string());
        let response = reqwest::get(&format!("http://{container_address}/json"))
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "Response from {} resulted in status code: {}",
                args.chrome_path,
                response.status().as_u16()
            ));
        }
        let json: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        let ws_url_returned = json[0]["webSocketDebuggerUrl"]
            .as_str()
            .ok_or_else(|| "webSocketDebuggerUrl not found in the response JSON".to_string())?;
        setup_log.push("Extracted webSocketDebuggerUrl from HTTP response.".to_string());

        let mut ws_url_parts: Vec<&str> = ws_url_returned.split('/').collect();
        if ws_url_parts.len() > 2 {
            ws_url_parts[2] = container_address;
        }
        let ws_url = ws_url_parts.join("/");
        BrowserRuntime::connect(
            ws_url,
            Some(idle_browser_timeout),
            true,
        )?
    } else {
        let chrome_path = if args.chrome_path.is_empty() {
            None
        } else {
            Some(PathBuf::from(args.chrome_path.clone()))
        };
        let cache_dir = gcx.read().await.cache_dir.clone();
        let profile_dir = get_browser_profile_dir(&cache_dir, chat_id);
        let headless = args.headless.parse::<bool>().unwrap_or(false);

        setup_log.push("Started new chrome process.".to_string());
        BrowserRuntime::launch(
            profile_dir,
            None,
            chrome_path,
            Some(idle_browser_timeout),
            true,
            headless,
        )?
    };

    let runtime_id = {
        let mut rt = runtime;
        rt.reattach(chat_id);
        // Set up recording so Browser Mode can attach to this runtime later
        if let Err(e) = setup_recording_for_runtime(&mut rt) {
            tracing::warn!("Browser recording setup failed (non-fatal): {}", e);
        }
        register_browser_runtime(gcx.clone(), rt).await
    };

    setup_log.push("No opened tabs at this moment.".to_string());

    let command_session: Box<dyn IntegrationSession> = Box::new(ChromeSession {
        runtime_id,
        tabs: HashMap::new(),
        idle_timeout: idle_browser_timeout,
        last_activity: Instant::now(),
    });
    gcx.write().await.integration_sessions.insert(
        session_hashmap_key.clone(),
        Arc::new(AMutex::new(command_session)),
    );
    Ok(setup_log)
}

fn set_device_metrics_method(
    width: u32,
    height: u32,
    device_scale_factor: f64,
    mobile: bool,
) -> Emulation::SetDeviceMetricsOverride {
    Emulation::SetDeviceMetricsOverride {
        width,
        height,
        device_scale_factor,
        mobile,
        scale: None,
        screen_width: None,
        screen_height: None,
        position_x: None,
        position_y: None,
        dont_set_visible_size: None,
        screen_orientation: None,
        viewport: None,
        display_feature: None,
        device_posture: None,
    }
}

async fn session_open_tab(
    chrome_session: &mut ChromeSession,
    gcx: Arc<ARwLock<GlobalContext>>,
    tab_id: &String,
    device: &DeviceType,
    settings_chrome: &SettingsChrome,
) -> Result<String, String> {
    match chrome_session.tabs.get(tab_id) {
        Some(tab) => {
            let tab_lock = tab.lock().await;
            Err(format!(
                "Tab is already opened: {}\n",
                tab_lock.state_string()
            ))
        }
        None => {
            let headless_tab = {
                let runtime_arc = {
                    let gcx_locked = gcx.read().await;
                    gcx_locked
                        .browser_runtimes
                        .get(&chrome_session.runtime_id)
                        .ok_or_else(|| format!(
                            "BrowserRuntime {} not found. Browser may have been closed.",
                            chrome_session.runtime_id
                        ))?
                        .clone()
                };
                let runtime_lock = runtime_arc.lock().await;
                runtime_lock.browser.new_tab().map_err(|e| e.to_string())?
            };
            let method = match device {
                DeviceType::Desktop => {
                    let (width, height) = match (
                        settings_chrome.window_width.parse::<u32>(),
                        settings_chrome.window_height.parse::<u32>(),
                    ) {
                        (Ok(width), Ok(height)) => (width, height),
                        _ => (800, 600),
                    };
                    let scale_factor = match settings_chrome.scale_factor.parse::<f64>() {
                        Ok(scale_factor) => scale_factor,
                        _ => 0.0,
                    };
                    set_device_metrics_method(width, height, scale_factor, false)
                }
                DeviceType::Mobile => {
                    let (width, height) = match (
                        settings_chrome.mobile_window_width.parse::<u32>(),
                        settings_chrome.mobile_window_height.parse::<u32>(),
                    ) {
                        (Ok(width), Ok(height)) => (width, height),
                        _ => (400, 800),
                    };
                    let scale_factor = match settings_chrome.mobile_scale_factor.parse::<f64>() {
                        Ok(scale_factor) => scale_factor,
                        _ => 0.0,
                    };
                    set_device_metrics_method(width, height, scale_factor, true)
                }
                DeviceType::Tablet => {
                    let (width, height) = match (
                        settings_chrome.tablet_window_width.parse::<u32>(),
                        settings_chrome.tablet_window_height.parse::<u32>(),
                    ) {
                        (Ok(width), Ok(height)) => (width, height),
                        _ => (600, 800),
                    };
                    let scale_factor = match settings_chrome.tablet_scale_factor.parse::<f64>() {
                        Ok(scale_factor) => scale_factor,
                        _ => 0.0,
                    };
                    set_device_metrics_method(width, height, scale_factor, true)
                }
            };
            headless_tab
                .call_method(method)
                .map_err(|e| e.to_string())?;
            let tab = Arc::new(AMutex::new(ChromeTab::new(headless_tab, device, tab_id)));
            let tab_lock = tab.lock().await;
            let tab_log = Arc::clone(&tab_lock.tab_log);
            tab_lock
                .headless_tab
                .enable_log()
                .map_err(|e| e.to_string())?;
            tab_lock
                .headless_tab
                .add_event_listener(Arc::new(move |event: &Event| {
                    if let Event::LogEntryAdded(e) = event {
                        let ts_raw = e.params.entry.timestamp;
                        let formatted_ts = crate::integrations::browser_runtime::normalize_timestamp_ms_opt(ts_raw)
                            .and_then(|ms| DateTime::from_timestamp_millis(ms as i64))
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| format!("ts={}", ts_raw));
                        let mut tab_log_lock = tab_log.lock().unwrap();
                        tab_log_lock.push(format!(
                            "{} [{:?}]: {}",
                            formatted_ts, e.params.entry.level, e.params.entry.text
                        ));
                        if tab_log_lock.len() > MAX_CACHED_LOG_LINES {
                            tab_log_lock.remove(0);
                        }
                    }
                }))
                .map_err(|e| e.to_string())?;
            chrome_session.tabs.insert(tab_id.clone(), tab.clone());
            let target_id = tab_lock.headless_tab.get_target_id().to_string();
            let runtime_tab = tab_lock.headless_tab.clone();
            drop(tab_lock);
            {
                let gcx_locked = gcx.read().await;
                if let Some(rt_arc) = gcx_locked.browser_runtimes.get(&chrome_session.runtime_id).cloned() {
                    let mut rt = rt_arc.lock().await;
                    let _ = setup_recording_for_tab(&mut rt, &runtime_tab);
                    rt.set_active_tab_target_id(target_id.clone());
                    rt.recording_tab_target_id = Some(target_id);
                }
            }
            Ok(format!("Opened a new tab: {}\n", tab_id))
        }
    }
}

async fn session_get_tab_arc(
    chrome_session: &ChromeSession,
    tab_id: &str,
) -> Result<Arc<AMutex<ChromeTab>>, String> {
    match chrome_session.tabs.get(tab_id) {
        Some(tab) => Ok(tab.clone()),
        None => {
            let available: Vec<&String> = chrome_session.tabs.keys().collect();
            if available.is_empty() {
                Err(format!(
                    "tab_id '{}' is not opened. No tabs are currently open — use 'open_tab {} desktop' first.",
                    tab_id, tab_id
                ))
            } else {
                Err(format!(
                    "tab_id '{}' is not opened. Available tabs: {:?}",
                    tab_id, available
                ))
            }
        }
    }
}

async fn execute_via_controller(
    action: &BrowserAction,
    chrome_session: Arc<AMutex<Box<dyn IntegrationSession>>>,
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<(Vec<String>, Vec<MultimodalElement>), String> {
    let tab_id = browser_actions::get_tab_id(action)
        .ok_or("Action has no tab_id for controller execution")?;
    let steps = browser_actions::to_browser_steps(action)
        .ok_or("Action cannot be converted to BrowserStep")?;

    let (headless_tab, tab_state) = {
        let session_tab = {
            let mut session_locked = chrome_session.lock().await;
            let cs = session_locked
                .as_any_mut()
                .downcast_mut::<ChromeSession>()
                .ok_or("Failed to downcast to ChromeSession")?;
            cs.tabs.get(tab_id).cloned()
        };

        match session_tab {
            Some(tab_arc) => {
                let tab_lock = tab_arc.lock().await;
                (tab_lock.headless_tab.clone(), tab_lock.state_string())
            }
            None => {
                let available: Vec<String> = {
                    let mut session_locked = chrome_session.lock().await;
                    let cs = session_locked
                        .as_any_mut()
                        .downcast_mut::<ChromeSession>()
                        .ok_or("Failed to downcast to ChromeSession")?;
                    cs.tabs.keys().cloned().collect()
                };
                let suggestion = if available.is_empty() {
                    format!("No tabs are open. Use 'open_tab {} desktop' first.", tab_id)
                } else {
                    format!("Available tabs: {:?}.", available)
                };
                return Err(format!(
                    "Tab '{}' not found. {}",
                    tab_id, suggestion
                ));
            }
        }
    };

    {
        let runtime_id = {
            let mut session_locked = chrome_session.lock().await;
            let cs = session_locked
                .as_any_mut()
                .downcast_mut::<ChromeSession>()
                .ok_or("Failed to downcast to ChromeSession")?;
            cs.runtime_id.clone()
        };
        let runtime_arc = {
            let gcx_locked = gcx.read().await;
            gcx_locked.browser_runtimes.get(&runtime_id).cloned()
        };
        if let Some(arc) = runtime_arc {
            let mut rt = arc.lock().await;
            rt.touch();
        }
    }

    let report = tokio::task::block_in_place(|| browser_controller::execute_steps(&*headless_tab, &steps));

    {
        let runtime_id = {
            let mut session_locked = chrome_session.lock().await;
            let cs = session_locked
                .as_any_mut()
                .downcast_mut::<ChromeSession>()
                .ok_or("Failed to downcast to ChromeSession")?;
            cs.runtime_id.clone()
        };
        let runtime_arc = {
            let gcx_locked = gcx.read().await;
            gcx_locked.browser_runtimes.get(&runtime_id).cloned()
        };
        if let Some(arc) = runtime_arc {
            let mut rt = arc.lock().await;
            for step_result in &report.steps {
                let action_type = if step_result.ok { "action" } else { "error" };
                rt.push_agent_action(action_type, &step_result.summary);
            }
        }
    }

    Ok(format_controller_report(&report, &tab_state))
}

fn format_controller_report(
    report: &ExecutionReport,
    tab_state: &str,
) -> (Vec<String>, Vec<MultimodalElement>) {
    let mut log = Vec::new();
    let mut multimodal = Vec::new();

    for result in &report.steps {
        if result.ok {
            log.push(result.summary.clone());
        } else {
            let msg = match &result.error {
                Some(e) => format!("{}: {}", result.summary, e),
                None => result.summary.clone(),
            };
            log.push(msg);
        }

        if let Some(ref data) = result.data {
            if let (Some(mime), Some(b64_data)) = (
                data.get("mime").and_then(|v| v.as_str()),
                data.get("data").and_then(|v| v.as_str()),
            ) {
                if mime.starts_with("image/") {
                    match resize_screenshot_b64(b64_data) {
                        Ok(resized) => {
                            if let Ok(el) = MultimodalElement::new(
                                "image/jpeg".to_string(),
                                resized,
                            ) {
                                multimodal.push(el);
                            }
                        }
                        Err(e) => log.push(format!("Screenshot processing: {}", e)),
                    }
                }
            }

            format_step_data(data, &mut log);
        }
    }

    if let Some(last) = log.last() {
        if !last.contains("tab_id") {
            let idx = log.len().saturating_sub(1);
            if let Some(entry) = log.get_mut(idx) {
                if !entry.contains(tab_state) {
                    *entry = format!("{} at {}", entry, tab_state);
                }
            }
        }
    }

    (log, multimodal)
}

fn execution_report_to_multimodal(
    report: &ExecutionReport,
) -> Result<Vec<MultimodalElement>, String> {
    let mut content = Vec::new();

    let mut text_report = serde_json::to_value(report)
        .map_err(|e| format!("Failed to serialize browser report: {}", e))?;
    strip_image_data_for_text(&mut text_report);
    let text_pretty = serde_json::to_string_pretty(&text_report)
        .map_err(|e| format!("Failed to pretty-print browser report: {}", e))?;
    content.push(MultimodalElement::new("text".to_string(), text_pretty)?);

    for result in &report.steps {
        if let Some(ref data) = result.data {
            if let (Some(mime), Some(b64_data)) = (
                data.get("mime").and_then(|v| v.as_str()),
                data.get("data").and_then(|v| v.as_str()),
            ) {
                if mime.starts_with("image/") {
                    let resized = resize_screenshot_b64(b64_data)?;
                    if let Ok(el) = MultimodalElement::new("image/jpeg".to_string(), resized) {
                        content.push(el);
                    }
                }
            }
        }
    }

    Ok(content)
}

fn strip_image_data_for_text(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let is_image = map.get("mime")
                .and_then(|v| v.as_str())
                .map(|m| m.starts_with("image/"))
                .unwrap_or(false);
            if is_image {
                let b64_len = map.get("data")
                    .and_then(|v| v.as_str())
                    .map(|s| s.len())
                    .unwrap_or(0);
                if b64_len > 0 {
                    let bytes = b64_len * 3 / 4;
                    map.insert(
                        "data".to_string(),
                        serde_json::Value::String("<omitted>".to_string()),
                    );
                    map.insert(
                        "bytes".to_string(),
                        serde_json::Value::Number(serde_json::Number::from(bytes)),
                    );
                }
            }
            for v in map.values_mut() {
                strip_image_data_for_text(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                strip_image_data_for_text(v);
            }
        }
        _ => {}
    }
}

fn resize_screenshot_b64(b64_data: &str) -> Result<String, String> {
    let raw = base64::prelude::BASE64_STANDARD
        .decode(b64_data)
        .map_err(|e| format!("Base64 decode failed: {}", e))?;

    let reader = ImageReader::with_format(Cursor::new(&raw), ImageFormat::Jpeg);
    let mut image = reader.decode().map_err(|e| format!("Image decode failed: {}", e))?;

    let max_dim = 800.0f32;
    let scale = max_dim / std::cmp::max(image.width(), image.height()) as f32;
    if scale < 1.0 {
        let nw = (scale * image.width() as f32) as u32;
        let nh = (scale * image.height() as f32) as u32;
        image = image.resize(nw, nh, FilterType::Lanczos3);
    }

    let mut out = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
        .map_err(|e| format!("Image encode failed: {}", e))?;

    Ok(base64::prelude::BASE64_STANDARD.encode(out))
}

fn format_step_data(data: &serde_json::Value, log: &mut Vec<String>) {
    if let Some(value) = data.get("value") {
        if !value.is_null() {
            if let Some(desc) = data.get("description").and_then(|v| v.as_str()) {
                if !desc.is_empty() {
                    log.push(format!("result: description {:?}, value {:?}", desc, value));
                    return;
                }
            }
            if let Some(s) = value.as_str() {
                log.push(format!("result: value {:?}", s));
            } else {
                log.push(format!("result: value {:?}", value));
            }
        }
    }

    if let Some(styles) = data.get("styles").and_then(|v| v.as_array()) {
        for s in styles {
            if let Some(s_str) = s.as_str() {
                log.push(s_str.to_string());
            }
        }
    }

    if let Some(links) = data.get("links").and_then(|v| v.as_array()) {
        for link in links {
            if let (Some(url), Some(text)) = (
                link.get("url").and_then(|v| v.as_str()),
                link.get("text").and_then(|v| v.as_str()),
            ) {
                if text.is_empty() {
                    log.push(url.to_string());
                } else {
                    log.push(format!("{} — {}", text, url));
                }
            }
        }
    }

    if let Some(rows) = data.get("rows").and_then(|v| v.as_array()) {
        for row in rows {
            if let Some(cells) = row.as_array() {
                let cell_texts: Vec<&str> = cells.iter().filter_map(|c| c.as_str()).collect();
                log.push(cell_texts.join(" | "));
            }
        }
    }

    if let Some(tree) = data.get("tree") {
        if !tree.is_null() {
            if let Ok(pretty) = serde_json::to_string_pretty(tree) {
                log.push(pretty);
            }
        }
    }

    if let Some(entries) = data.get("entries").and_then(|v| v.as_array()) {
        for entry in entries {
            if let Some(s) = entry.as_str() {
                log.push(s.to_string());
            }
        }
    }

    if let Some(html) = data.get("html").and_then(|v| v.as_str()) {
        if !html.is_empty() {
            log.push(html.to_string());
        }
    }

    if let Some(text) = data.get("text").and_then(|v| v.as_str()) {
        if !text.is_empty() {
            log.push(text.to_string());
        }
    }
}

async fn chrome_command_exec(
    action: &BrowserAction,
    chrome_session: Arc<AMutex<Box<dyn IntegrationSession>>>,
    gcx: Arc<ARwLock<GlobalContext>>,
    settings_chrome: &SettingsChrome,
) -> Result<(Vec<String>, Vec<MultimodalElement>), String> {
    if browser_actions::to_browser_steps(action).is_some() {
        return execute_via_controller(action, chrome_session, gcx).await;
    }

    let mut tool_log = vec![];
    let multimodal_els = vec![];

    match action {
        BrowserAction::OpenTab { tab_id, device } => {
            let log = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked
                    .as_any_mut()
                    .downcast_mut::<ChromeSession>()
                    .ok_or("Failed to downcast to ChromeSession")?;
                session_open_tab(chrome_session, gcx.clone(), tab_id, device, settings_chrome)
                    .await?
            };
            tool_log.push(log);
        }
        BrowserAction::ClickAtPoint { tab_id, x, y } => {
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked
                    .as_any_mut()
                    .downcast_mut::<ChromeSession>()
                    .ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, tab_id).await?
            };
            let log = {
                let tab_lock = tab.lock().await;
                match {
                    let mapped_point = Point {
                        x: x / tab_lock.screenshot_scale_factor,
                        y: y / tab_lock.screenshot_scale_factor,
                    };
                    tab_lock
                        .headless_tab
                        .click_point(mapped_point)
                        .map_err(|e| e.to_string())?;
                    Ok::<(), String>(())
                } {
                    Ok(_) => {
                        format!(
                            "clicked `{} {}` at {}",
                            x, y,
                            tab_lock.state_string()
                        )
                    }
                    Err(e) => {
                        format!(
                            "clicked `{} {}` failed at {}: {}",
                            x, y,
                            tab_lock.state_string(),
                            e
                        )
                    }
                }
            };
            tool_log.push(log);
        }
        BrowserAction::TypeText { tab_id, text } => {
            let tab = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let chrome_session = chrome_session_locked
                    .as_any_mut()
                    .downcast_mut::<ChromeSession>()
                    .ok_or("Failed to downcast to ChromeSession")?;
                session_get_tab_arc(chrome_session, tab_id).await?
            };
            let log = {
                let tab_lock = tab.lock().await;
                match tab_lock.headless_tab.type_str(text.as_str()) {
                    Ok(_) => {
                        format!("type `{}` at {}", text, tab_lock.state_string())
                    }
                    Err(e) => {
                        format!(
                            "type text failed at {}: {}",
                            tab_lock.state_string(),
                            e
                        )
                    }
                }
            };
            tool_log.push(log);
        }
        BrowserAction::ListTabs => {
            let (session_tabs, runtime_id) = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let cs = chrome_session_locked
                    .as_any_mut()
                    .downcast_mut::<ChromeSession>()
                    .ok_or("Failed to downcast to ChromeSession")?;
                let tabs: Vec<(String, Arc<AMutex<ChromeTab>>)> = cs.tabs.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                (tabs, cs.runtime_id.clone())
            };
            let runtime_tabs = {
                let gcx_locked = gcx.read().await;
                if let Some(rt_arc) = gcx_locked.browser_runtimes.get(&runtime_id).cloned() {
                    let rt = rt_arc.lock().await;
                    rt.browser.get_tabs().lock()
                        .map(|tabs| tabs.iter().map(|t| (t.get_target_id().to_string(), t.get_url(), t.get_title().unwrap_or_default())).collect::<Vec<_>>())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            };
            if session_tabs.is_empty() && runtime_tabs.is_empty() {
                tool_log.push("No tabs are currently open. Use 'open_tab <tab_id> desktop' to open one.".to_string());
            } else {
                if !session_tabs.is_empty() {
                    tool_log.push(format!("Session tabs ({}):", session_tabs.len()));
                    for (_tab_id, tab_arc) in &session_tabs {
                        let tab_lock = tab_arc.lock().await;
                        tool_log.push(format!("  {}", tab_lock.state_string()));
                    }
                }
                let session_target_ids: Vec<String> = {
                    let mut ids = Vec::new();
                    for (_, tab_arc) in &session_tabs {
                        let tab_lock = tab_arc.lock().await;
                        ids.push(tab_lock.headless_tab.get_target_id().to_string());
                    }
                    ids
                };
                let extra_tabs: Vec<_> = runtime_tabs.iter()
                    .filter(|(tid, _, _)| !session_target_ids.contains(tid))
                    .collect();
                if !extra_tabs.is_empty() {
                    tool_log.push(format!("Runtime tabs ({}):", extra_tabs.len()));
                    for (tid, url, title) in &extra_tabs {
                        tool_log.push(format!("  target={} url={} title={}", &tid[..8.min(tid.len())], url, title));
                    }
                }
            }
        }
        BrowserAction::CloseTab { tab_id } => {
            let (tab_arc, available, runtime_id) = {
                let mut chrome_session_locked = chrome_session.lock().await;
                let cs = chrome_session_locked
                    .as_any_mut()
                    .downcast_mut::<ChromeSession>()
                    .ok_or("Failed to downcast to ChromeSession")?;
                let tab = cs.tabs.get(tab_id).cloned();
                let avail: Vec<String> = cs.tabs.keys().cloned().collect();
                (tab, avail, cs.runtime_id.clone())
            };
            match tab_arc {
                Some(tab_arc) => {
                    let tab_lock = tab_arc.lock().await;
                    let state = tab_lock.state_string();
                    let target_id = tab_lock.headless_tab.get_target_id().to_string();
                    match tab_lock.headless_tab.close(false) {
                        Ok(_) => {
                            drop(tab_lock);
                            {
                                let mut chrome_session_locked = chrome_session.lock().await;
                                if let Some(cs) = chrome_session_locked
                                    .as_any_mut()
                                    .downcast_mut::<ChromeSession>()
                                {
                                    cs.tabs.remove(tab_id);
                                }
                            }
                            let runtime_arc = {
                                let gcx_locked = gcx.read().await;
                                gcx_locked.browser_runtimes.get(&runtime_id).cloned()
                            };
                            if let Some(arc) = runtime_arc {
                                let mut rt = arc.lock().await;
                                if rt.recording_tab_target_id.as_deref() == Some(&target_id) {
                                    rt.recording_tab_target_id = None;
                                }
                                if rt.active_tab_target_id().as_deref() == Some(target_id.as_str()) {
                                    rt.active_tab_target_id = None;
                                }
                            }
                            tool_log.push(format!("Closed tab: {}.", state));
                        }
                        Err(e) => {
                            tool_log.push(format!(
                                "Failed to close tab {}: {}. Tab remains tracked.",
                                state, e
                            ));
                        }
                    }
                }
                None => {
                    if available.is_empty() {
                        tool_log.push(format!(
                            "Tab '{}' not found. No tabs are currently open.",
                            tab_id
                        ));
                    } else {
                        tool_log.push(format!(
                            "Tab '{}' not found. Available tabs: {:?}.",
                            tab_id, available
                        ));
                    }
                }
            }
        }
        // All other actions are handled by the controller pipeline above
        _ => unreachable!("Action should have been delegated to controller"),
    }

    Ok((tool_log, multimodal_els))
}

