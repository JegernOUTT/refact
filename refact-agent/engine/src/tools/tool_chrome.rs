use std::any::Any;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::integrations::sessions::{IntegrationSession, get_session_hashmap_key};

use crate::global_context::GlobalContext;
use crate::http::routers::v1::browser_settings;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::scratchpads::multimodality::MultimodalElement;

use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

use crate::integrations::browser_actions::{self, BrowserAction, DeviceType};
use crate::integrations::browser_controller;
use crate::integrations::browser_models::{ExecutionReport, parse_browser_action_request};
use crate::integrations::browser_runtime::{
    BrowserGpuMode, BrowserLaunchOptions, BrowserProxyOptions, BrowserRuntime,
    clear_stale_profile_lock, describe_launch_failure, find_runtime_by_chat_id,
    register_browser_runtime, get_browser_profile_dir, relaunch_lock_for_chat,
    remove_browser_runtime, setup_recording_for_runtime, setup_recording_for_tab,
};

use chrono::DateTime;
use std::path::PathBuf;
use headless_chrome::Tab as HeadlessTab;
use headless_chrome::browser::tab::point::Point;

use headless_chrome::protocol::cdp::Emulation;
use headless_chrome::protocol::cdp::types::Event;

use serde::{Deserialize, Serialize};

use base64::Engine;
use refact_browser::screencast::{MAX_BURST_DURATION_MS, MAX_FRAME_COUNT, MIN_FRAME_COUNT};
use refact_core::image_policy::{resize_to_policy, ImagePolicy};

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
    #[serde(default)]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub chromium_sandbox: String,
    #[serde(default)]
    pub proxy_server: String,
    #[serde(default)]
    pub proxy_bypass: String,
    #[serde(default)]
    pub downloads_dir: String,
    #[serde(default)]
    pub ignore_https_errors: String,
    #[serde(default)]
    pub gpu: String,
}

impl SettingsChrome {
    pub fn launch_options(&self) -> BrowserLaunchOptions {
        self.launch_options_over(browser_settings::current().to_launch_options())
    }

    pub fn launch_options_over(&self, base: BrowserLaunchOptions) -> BrowserLaunchOptions {
        BrowserLaunchOptions {
            headless: self.headless.parse().unwrap_or(base.headless),
            chrome_path: (!self.chrome_path.is_empty())
                .then(|| PathBuf::from(self.chrome_path.clone()))
                .or_else(|| base.chrome_path.clone()),
            idle_timeout: self
                .idle_browser_timeout
                .parse::<u64>()
                .ok()
                .map(Duration::from_secs)
                .or(base.idle_timeout),
            extra_args: if self.extra_args.is_empty() {
                base.extra_args.clone()
            } else {
                self.extra_args.clone()
            },
            chromium_sandbox: self
                .chromium_sandbox
                .parse()
                .unwrap_or(base.chromium_sandbox),
            proxy: if self.proxy_server.is_empty() {
                base.proxy.clone()
            } else {
                Some(BrowserProxyOptions {
                    server: self.proxy_server.clone(),
                    bypass: (!self.proxy_bypass.is_empty()).then(|| self.proxy_bypass.clone()),
                })
            },
            downloads_dir: (!self.downloads_dir.is_empty())
                .then(|| PathBuf::from(self.downloads_dir.clone()))
                .or_else(|| base.downloads_dir.clone()),
            ignore_https_errors: self
                .ignore_https_errors
                .parse()
                .unwrap_or(base.ignore_https_errors),
            gpu: if self.gpu.trim().is_empty() {
                base.gpu
            } else {
                BrowserGpuMode::parse(&self.gpu).unwrap_or(base.gpu)
            },
            ..base
        }
    }
}

#[cfg(test)]
mod launch_options_tests {
    use super::*;

    fn base_with_gpu(gpu: BrowserGpuMode) -> BrowserLaunchOptions {
        BrowserLaunchOptions {
            gpu,
            ..BrowserLaunchOptions::default()
        }
    }

    fn merged_gpu(gpu: &str, base: BrowserGpuMode) -> BrowserGpuMode {
        SettingsChrome {
            gpu: gpu.to_string(),
            ..SettingsChrome::default()
        }
        .launch_options_over(base_with_gpu(base))
        .gpu
    }

    #[test]
    fn unset_gpu_defers_to_base() {
        assert_eq!(
            merged_gpu("", BrowserGpuMode::Hardware),
            BrowserGpuMode::Hardware
        );
        assert_eq!(
            merged_gpu("", BrowserGpuMode::Software),
            BrowserGpuMode::Software
        );
        assert_eq!(
            merged_gpu("   ", BrowserGpuMode::Hardware),
            BrowserGpuMode::Hardware
        );
    }

    #[test]
    fn explicit_gpu_overrides_base() {
        assert_eq!(
            merged_gpu("disabled", BrowserGpuMode::Hardware),
            BrowserGpuMode::Disabled
        );
        assert_eq!(
            merged_gpu("software", BrowserGpuMode::Hardware),
            BrowserGpuMode::Software
        );
        assert_eq!(
            merged_gpu("hardware", BrowserGpuMode::Disabled),
            BrowserGpuMode::Hardware
        );
    }

    #[test]
    fn unparseable_gpu_falls_back_to_base() {
        assert_eq!(
            merged_gpu("metal", BrowserGpuMode::Hardware),
            BrowserGpuMode::Hardware
        );
    }

    #[test]
    fn unset_fields_defer_to_base() {
        let base = BrowserLaunchOptions {
            headless: true,
            chrome_path: Some(PathBuf::from("/usr/bin/chromium")),
            downloads_dir: Some(PathBuf::from("/tmp/downloads")),
            proxy: Some(BrowserProxyOptions {
                server: "http://127.0.0.1:8080".to_string(),
                bypass: Some("localhost".to_string()),
            }),
            extra_args: vec!["--mute-audio".to_string()],
            chromium_sandbox: false,
            ignore_https_errors: false,
            idle_timeout: Some(Duration::from_secs(42)),
            ..BrowserLaunchOptions::default()
        };

        let merged = SettingsChrome::default().launch_options_over(base.clone());

        assert_eq!(merged, base);
    }
}

#[derive(Default)]
pub struct ToolChrome {
    pub settings_chrome: SettingsChrome,
    pub config_path: String,
}

// DeviceType is now in browser_actions module

const MAX_CACHED_LOG_LINES: usize = 1000;

const CHROME_HELP: &str = include_str!("chrome_help.md");

const CHROME_HELP_TOPICS: &str = "overview, locators, navigation, input, forms, waits, network, routes, websockets, har, tabs, frames, dialogs, uploads, downloads, emulation, devices, clock, screenshots, artifacts, screencast, assertions, readouts, handlers, cdp, authenticators, reset, troubleshooting";

const CHROME_DESCRIPTION: &str = concat!(
    "Text-first batched browser automation. You read pages as text, not as pictures. Prefer the typed `request`: ONE call can carry many steps, unlike one-action-per-call servers. ",
    "The loop is: navigate -> read the returned snapshot refs -> act by ref -> repeat. Any batch that changes the page attaches a ref-annotated ARIA snapshot under `page.snapshot` automatically, so you do NOT need an `accessibility_snapshot` step after navigating; screenshots are opt-in and cost far more than the text tree. ",
    "Each `[ref=eN]` handle in that snapshot is an element address: act with `locator.by=ref`; refs come from the most recent snapshot. Use an explicit `accessibility_snapshot` step only to re-read a page that did NOT change, or to scope to a subtree with `locator`/`depth`.\n",
    "Canonical batch: {\"steps\":[{\"action\":\"navigate\",\"url\":\"https://example.com\"},{\"action\":\"click\",\"locator\":{\"by\":\"ref\",\"value\":\"e5\"}},{\"action\":\"fill\",\"locator\":{\"by\":\"ref\",\"value\":\"e7\"},\"text\":\"hi\"}]} Pass this object as `request`; e5/e7 stand for handles minted by the snapshot the previous batch returned.\n",
    "TUTORIAL: run {\"steps\":[{\"action\":\"help\"}]} for the full tutorial covering every action, parameter, and gotcha, or {\"action\":\"help\",\"topic\":\"<name>\"} for one section. Topics: ",
    "overview, locators, navigation, input, forms, waits, network, routes, websockets, har, tabs, frames, dialogs, uploads, downloads, emulation, devices, clock, screenshots, artifacts, screencast, assertions, readouts, handlers, cdp, authenticators, reset, troubleshooting",
    ". help answers from documentation and never launches the browser, so it is free to consult before a run.\n",
    "Capability groups, each with a help topic: navigation and tabs; locator-driven input plus a mouse_*/tap coordinate escape hatch; forms; waits; assertions (expect, expect_poll); readouts (bounding_box, count, input_value, all_texts, element_state); inspection and snapshots; screenshots, pdf, and filmstrip capture; network interception (route, HAR, WebSocket) and http_request; emulation, devices, storage, and cookies; the fake clock; locator handlers and dialogs; coverage and virtual authenticators; cdp_send and reset. The `action` enum below is the authoritative action list.\n",
    "Locators are ref-first with a fallback vocabulary (role, test_id, text, label, placeholder, alt_text, title, css, xpath, id, name, autocomplete) composable with nth/first/last, filter, and/or, and an outermost-first `frames` chain. Non-selecting actions are strict: ambiguous locators fail loudly with the match count.\n",
    "Page report: a page-changing batch returns `page` with the final URL and title, `page.status` for a non-2xx main document, `page.console` error/warning COUNTS (full text stays in `console` and `tab_log`), and `page.snapshot`, inlined when small and otherwise a `text/yaml` artifact plus the head, `lines`, `bytes`, and `truncated:true`. Locator-driven actions report `locator_echo`, a canonical locator generated for the element actually acted on.\n",
    "`page_context` picks the page-changed context: `snapshot` (the default) attaches the ref-annotated ARIA snapshot and NO image, `screenshot` attaches a policy-sized image instead, `both` attaches each, `none` attaches only the page header. `attach_screenshot` is the tri-state override and wins over `page_context`: true = always attach, false = never attach, omitted = follow `page_context`. `network` controls per-request report volume: `summary` (the default) emits one `method url status bytes ms` line per request, `full` keeps headers, `none` drops per-request entries.\n",
    "Click, hover, fill, clear, check, and uncheck auto-wait for actionability. Never use `wait_seconds` for readiness; use `wait_for_response`, `wait_for_load_state`, `wait_for_selector`, or `wait_for_function`."
);

pub fn chrome_help_text(topic: Option<&str>) -> String {
    let sections = chrome_help_sections();
    let index = format!(
        "chrome help topics: {}\nCall {{\"action\":\"help\",\"topic\":\"<name>\"}} for one section.",
        CHROME_HELP_TOPICS
    );
    let Some(topic) = topic.map(str::trim).filter(|topic| !topic.is_empty()) else {
        let overview = sections
            .iter()
            .find(|(name, _)| name == "overview")
            .map(|(_, body)| body.as_str())
            .unwrap_or_default();
        return format!("{index}\n\n## overview\n{overview}");
    };
    match sections
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(topic))
    {
        Some((name, body)) => format!("## {name}\n{body}"),
        None => format!("Unknown help topic {topic:?}.\n\n{index}"),
    }
}

fn chrome_help_sections() -> Vec<(String, String)> {
    let mut sections = Vec::new();
    for chunk in CHROME_HELP.split("\n## ").skip(1) {
        let (name, body) = chunk.split_once('\n').unwrap_or((chunk, ""));
        sections.push((name.trim().to_string(), body.trim().to_string()));
    }
    sections
}

pub fn help_only_request(request_value: &Value) -> bool {
    request_value
        .get("steps")
        .and_then(Value::as_array)
        .is_some_and(|steps| {
            !steps.is_empty()
                && steps
                    .iter()
                    .all(|step| step.get("action").and_then(Value::as_str) == Some("help"))
        })
}

fn help_only_response(request_value: &Value) -> String {
    request_value
        .get("steps")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .map(|step| chrome_help_text(step.get("topic").and_then(Value::as_str)))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default()
}

static BROWSER_LOCATOR_SCHEMA: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    serde_json::from_str(include_str!("tool_chrome_schemas/browser_locator.json"))
        .expect("embedded browser locator schema is valid JSON")
});

static BROWSER_STEP_SCHEMA: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    serde_json::from_str(include_str!("tool_chrome_schemas/browser_step.json"))
        .expect("embedded browser step schema is valid JSON")
});

static ROUTE_HANDLER_SCHEMA: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    serde_json::from_str(include_str!("tool_chrome_schemas/route_handler.json"))
        .expect("embedded route handler schema is valid JSON")
});

static LOCATOR_HANDLER_SCHEMA: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    serde_json::from_str(include_str!("tool_chrome_schemas/locator_handler.json"))
        .expect("embedded locator handler schema is valid JSON")
});

fn browser_locator_schema() -> serde_json::Value {
    BROWSER_LOCATOR_SCHEMA.clone()
}

fn tab_target_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["type"],
        "properties": {
            "type": {"type": "string", "enum": ["active", "id"]},
            "id": {"type": "string", "description": "Required when type is id"}
        }
    })
}

/// Compact per-action REQUIRED-field manifest rendered from [`BrowserStep::ACTION_FIELDS`],
/// the same table the deserializer contract is pinned to. Only required fields are listed:
/// they are the ones a caller cannot guess, and the full optional/alias table would blow the
/// tool-contract token budget. The optional fields and aliases live in `chrome_help.md`,
/// reachable through the `help` action.
fn browser_action_field_manifest(actions: &[&str]) -> String {
    let mut entries = Vec::new();
    let mut no_required_actions = Vec::new();
    for (action, required, _optional, _aliases) in
        crate::integrations::browser_models::BrowserStep::ACTION_FIELDS.iter()
    {
        if !actions.contains(action) {
            continue;
        }
        if required.is_empty() {
            no_required_actions.push(*action);
        } else {
            entries.push(format!("{action}: {}", required.join(" ")));
        }
    }
    if !no_required_actions.is_empty() {
        entries.push(format!(
            "no required fields: {}",
            no_required_actions.join(" ")
        ));
    }
    entries.join("; ")
}

/// Every field name any action accepts, so the flat property bag can never advertise a
/// field the deserializer rejects for every action.
fn browser_contract_field_names() -> std::collections::BTreeSet<&'static str> {
    crate::integrations::browser_models::BrowserStep::ACTION_FIELDS
        .iter()
        .flat_map(|(_, required, optional, aliases)| {
            required
                .iter()
                .chain(optional.iter())
                .chain(aliases.iter())
                .copied()
        })
        .collect()
}

fn browser_step_schema_with_actions(
    actions: &[&str],
    include_locator_handler: bool,
) -> serde_json::Value {
    let mut schema = BROWSER_STEP_SCHEMA.clone();
    let properties = schema["properties"]
        .as_object_mut()
        .expect("browser step schema properties are present");
    if include_locator_handler {
        properties.insert("locator".to_string(), browser_locator_schema());
    }
    let action_schema = properties
        .get_mut("action")
        .and_then(Value::as_object_mut)
        .expect("browser step action schema is present");
    action_schema.insert(
        "description".to_string(),
        Value::String(format!(
            "Browser action in snake_case. Each action accepts ONLY its own fields; anything else is rejected. Required fields per action (optional fields and aliases are in the `help` action's documentation): {}",
            browser_action_field_manifest(actions)
        )),
    );
    action_schema.insert(
        "enum".to_string(),
        Value::Array(
            actions
                .iter()
                .map(|action| Value::String((*action).to_string()))
                .collect(),
        ),
    );
    properties.insert("locator".to_string(), browser_locator_schema());
    if include_locator_handler {
        properties.insert("handler".to_string(), handler_schema());
    } else {
        properties.remove("handler");
    }
    properties.get_mut("duration_ms").unwrap()["maximum"] = Value::from(MAX_BURST_DURATION_MS);
    properties.get_mut("frame_count").unwrap()["minimum"] = Value::from(MIN_FRAME_COUNT);
    properties.get_mut("frame_count").unwrap()["maximum"] = Value::from(MAX_FRAME_COUNT);
    let contract_fields = browser_contract_field_names();
    properties.retain(|name, _| name == "action" || contract_fields.contains(name.as_str()));
    schema
}

fn route_handler_schema() -> serde_json::Value {
    ROUTE_HANDLER_SCHEMA.clone()
}

fn handler_schema() -> Value {
    let mut schema = Value::Object(serde_json::Map::new());
    schema["oneOf"] = Value::Array(vec![LOCATOR_HANDLER_SCHEMA.clone(), route_handler_schema()]);
    schema
}

fn browser_request_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "description": "Typed batched browser request",
        "required": ["steps"],
        "properties": {
            "session": {"type": "string", "enum": ["shared_default"]},
            "page_context": {"type": "string", "enum": ["snapshot", "screenshot", "both", "none"], "description": "snapshot (default) attaches the aria snapshot and no image; screenshot attaches a PNG; both; none = page header only"},
            "attach_screenshot": {"type": "boolean", "description": "Screenshot override: true = always, false = never, omitted = follow page_context"},
            "network": {"type": "string", "enum": ["none", "summary", "full"], "description": "summary (default) = one `method url status bytes ms` line per request, full keeps headers, none drops entries"},
            "block_service_workers": {"type": "boolean", "description": "Block service workers for this context (sticky until reset)"},
            "continue_on_error": {"type": "boolean", "description": "Run later steps after a failure (default false); ok stays false"},
            "target": tab_target_schema(),
            "steps": {
                "type": "array",
                "items": browser_step_schema_with_actions(
                    crate::integrations::browser_models::BrowserStep::ACTION_NAMES,
                    true,
                )
            }
        }
    })
}

fn chrome_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "request": browser_request_schema()
        }
    })
}

async fn image_policy_for_model(gcx: Arc<GlobalContext>, model_id: &str) -> ImagePolicy {
    let Ok(caps) = crate::global_context::try_load_caps_quickly_if_not_present(gcx, 0).await else {
        return ImagePolicy::default();
    };
    crate::caps::resolve_chat_model(caps, model_id)
        .map(|model| ImagePolicy::for_model(&model.base))
        .unwrap_or_default()
}

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
        if let Some(request_value) = args.get("request") {
            if help_only_request(request_value) {
                let msg = ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(help_only_response(request_value)),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                });
                return Ok((false, vec![msg]));
            }
        }

        let (gcx, chat_id, current_model) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.current_model.clone(),
            )
        };
        let image_policy = image_policy_for_model(gcx.clone(), &current_model).await;

        let session_hashmap_key = get_session_hashmap_key("chrome", &chat_id);
        let mut tool_log = match setup_chrome_session(
            gcx.clone(),
            &self.settings_chrome,
            &session_hashmap_key,
            &chat_id,
        )
        .await
        {
            Ok(log) => log,
            Err(e) => {
                crate::buddy::actor::report_error_persisted(
                    crate::app_state::AppState::from_gcx(gcx.clone()).await,
                    "browser_error",
                    &e,
                    Some("tools/tool_chrome.rs"),
                    Some(&chat_id),
                )
                .await;
                return Err(e);
            }
        };

        let command_session = {
            let integration_sessions = gcx.integration_sessions.clone();
            let integration_sessions = integration_sessions.lock().await;
            integration_sessions
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
        let mut browser_extra = None;
        let mut tool_failed = false;

        if let Some(request_value) = args.get("request") {
            let request = parse_browser_action_request(request_value.clone())
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
                let browser_runtimes = gcx.browser_runtimes.clone();
                let browser_runtimes = browser_runtimes.lock().await;
                browser_runtimes.get(&runtime_id).cloned().ok_or_else(|| {
                    format!(
                        "BrowserRuntime {} not found. Browser may have been closed.",
                        runtime_id
                    )
                })?
            };

            match browser_controller::execute_request_with_runtime_validated(
                runtime_arc,
                request,
                &image_policy,
                gcx.clone(),
            )
            .await
            {
                Ok(report) => {
                    tool_failed = !report.ok;
                    typed_content = Some(execution_report_to_multimodal(&report, &image_policy)?);
                    let (execute_log, command_multimodal_els) =
                        format_controller_report(&report, "", &image_policy);
                    tool_log.extend(execute_log);
                    multimodal_els.extend(command_multimodal_els);
                    browser_extra = Some(browser_metadata(&report));
                }
                Err(e) => {
                    let err_msg = format!("Failed to execute typed browser request: {}.", e);
                    tool_log.push(err_msg.clone());
                    crate::buddy::actor::report_error_persisted(
                        crate::app_state::AppState::from_gcx(gcx.clone()).await,
                        "browser_error",
                        &err_msg,
                        Some("tools/tool_chrome.rs"),
                        Some(&chat_id),
                    )
                    .await;
                    return Err(err_msg);
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
                        tool_failed = true;
                        tool_log.push(format!("Failed to parse command #{}: {}.", idx + 1, e));
                        break;
                    }
                };
                match chrome_command_exec(
                    &action,
                    command_session.clone(),
                    gcx.clone(),
                    &self.settings_chrome,
                    &image_policy,
                )
                .await
                {
                    Ok((execute_log, command_multimodal_els)) => {
                        tool_log.extend(execute_log);
                        multimodal_els.extend(command_multimodal_els);
                    }
                    Err(e) => {
                        tool_failed = true;
                        let err_msg = format!("Failed to execute command: {}.", e);
                        tool_log.push(err_msg.clone());
                        crate::buddy::actor::report_error_persisted(
                            crate::app_state::AppState::from_gcx(gcx.clone()).await,
                            "browser_error",
                            &err_msg,
                            Some("tools/tool_chrome.rs"),
                            Some(&chat_id),
                        )
                        .await;
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

        let mut extra = serde_json::Map::new();
        if let Some(browser_extra) = browser_extra {
            extra.insert("browser".to_string(), browser_extra);
        }
        let msg = ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::Multimodal(content),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            extra,
            ..Default::default()
        });

        Ok((tool_failed, vec![msg]))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "chrome".to_string(),
            display_name: "Chrome".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: CHROME_DESCRIPTION.to_string(),
            input_schema: chrome_input_schema(),
            output_schema: None,
            annotations: None,
        }
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn browser_locator_schema_snapshot() {
        insta::assert_snapshot!(serde_json::to_string(&browser_locator_schema()).unwrap());
    }

    #[test]
    fn browser_step_schema_snapshot_for_tool_caller() {
        insta::assert_snapshot!(serde_json::to_string(&browser_step_schema_with_actions(
            crate::integrations::browser_models::BrowserStep::ACTION_NAMES,
            true,
        ))
        .unwrap());
    }

    #[test]
    fn route_handler_schema_snapshot() {
        insta::assert_snapshot!(serde_json::to_string(&route_handler_schema()).unwrap());
    }

    #[test]
    fn embedded_browser_schemas_parse_on_first_use() {
        assert!(BROWSER_LOCATOR_SCHEMA.is_object());
        assert!(BROWSER_STEP_SCHEMA.is_object());
        assert!(ROUTE_HANDLER_SCHEMA.is_object());
        assert!(LOCATOR_HANDLER_SCHEMA.is_object());
    }

    fn schema_action_names(schema: &Value) -> BTreeSet<String> {
        schema
            .pointer("/properties/request/properties/steps/items/properties/action/enum")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn chrome_schema_serializes_and_covers_every_browser_step_variant() {
        let schema = chrome_input_schema();
        serde_json::to_string(&schema).unwrap();
        let model_actions = crate::integrations::browser_models::BrowserStep::ACTION_NAMES
            .iter()
            .map(|action| action.to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(schema_action_names(&schema), model_actions);
    }

    #[test]
    fn chrome_schema_exposes_current_browser_step_parameters() {
        let schema = chrome_input_schema();
        let properties = schema
            .pointer("/properties/request/properties/steps/items/properties")
            .and_then(Value::as_object)
            .unwrap();
        for field in [
            "accept",
            "animations",
            "attribute",
            "boxes",
            "button",
            "caret",
            "clear_first",
            "click_count",
            "clip",
            "compose",
            "credential",
            "css",
            "delay",
            "delta_x",
            "delta_y",
            "depth",
            "device",
            "end_x",
            "end_y",
            "expected",
            "expression",
            "format",
            "full_page",
            "handler",
            "height",
            "has_resident_key",
            "has_user_verification",
            "id",
            "is_user_verified",
            "js",
            "key",
            "label",
            "labels",
            "landscape",
            "limit",
            "locator",
            "locators",
            "margins",
            "max_chars",
            "mask",
            "mask_color",
            "mode",
            "modifiers",
            "name",
            "no_wait_after",
            "not",
            "omit_background",
            "outline",
            "page_ranges",
            "params",
            "paths",
            "pattern",
            "polling_ms",
            "prefer_css_page_size",
            "print_background",
            "prompt_text",
            "property_filter",
            "protocol",
            "quality",
            "refs",
            "reset_on_navigation",
            "save_as",
            "scale",
            "seconds",
            "selector",
            "source",
            "source_position",
            "start_x",
            "start_y",
            "state",
            "states",
            "steps",
            "style",
            "tab",
            "tagged",
            "target",
            "target_position",
            "text",
            "timeout_ms",
            "times",
            "transport",
            "type",
            "url",
            "value",
            "verify",
            "width",
            "x",
            "y",
        ] {
            assert!(
                properties.contains_key(field),
                "missing schema field {field}"
            );
        }
        assert!(
            schema.pointer("/properties/commands").is_none(),
            "the deprecated commands input must be invisible to models"
        );
        let url_pattern_required = schema
            .pointer(
                "/properties/request/properties/steps/items/properties/pattern/oneOf/1/required",
            )
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(url_pattern_required, &[Value::String("source".to_string())]);
        for legacy_field in ["url_or_pattern", "root", "data"] {
            assert!(
                !properties.contains_key(legacy_field),
                "legacy field {legacy_field} must not appear in the schema"
            );
        }
        assert!(
            properties["contains"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("wait_for_console_message")),
            "contains belongs to wait_for_console_message, never to wait_for_url"
        );
        let handler_types = schema
            .pointer("/properties/request/properties/steps/items/properties/handler/oneOf/1/oneOf")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(|handler| {
                handler
                    .pointer("/properties/type/enum/0")
                    .and_then(Value::as_str)
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            handler_types,
            BTreeSet::from([
                "abort",
                "continue",
                "fallback",
                "fetch_and_fulfill",
                "fulfill"
            ])
        );
        assert!(schema
            .pointer("/properties/request/properties/steps/items/properties/handler/oneOf/1/oneOf/0/properties/path/description")
            .and_then(Value::as_str)
            .is_some_and(|description| description.contains("artifact directory")));
        assert!(schema
            .pointer("/properties/request/properties/steps/items/properties/handler/oneOf/1/oneOf/0/properties/body/description")
            .and_then(Value::as_str)
            .is_some_and(|description| description.contains("UTF-8") && description.contains("base64")));
    }

    #[test]
    fn chrome_schema_matcher_accepts_expectation_objects_and_poll_comparators() {
        let schema = chrome_input_schema();
        let matcher = schema
            .pointer("/properties/request/properties/steps/items/properties/matcher/oneOf")
            .and_then(Value::as_array)
            .unwrap();

        assert_eq!(
            matcher[0].pointer("/type").and_then(Value::as_str),
            Some("object")
        );
        assert!(matcher[0]
            .pointer("/properties/type/enum")
            .and_then(Value::as_array)
            .unwrap()
            .contains(&Value::String("to_match_aria_snapshot".to_string())));
        for option in ["checked", "indeterminate", "pseudo", "ratio"] {
            assert!(
                matcher[0]
                    .pointer(&format!("/properties/{option}"))
                    .is_some(),
                "missing matcher option {option}"
            );
        }
        assert_eq!(
            matcher[0].pointer("/properties/pseudo/enum"),
            Some(&serde_json::json!(["before", "after"]))
        );
        assert_eq!(
            matcher[1].pointer("/type").and_then(Value::as_str),
            Some("string")
        );
        let comparators = matcher[1]
            .pointer("/enum")
            .and_then(Value::as_array)
            .unwrap();
        for comparator in ["equals", "contains", "gt", "lt", "matches_regex"] {
            assert!(
                comparators.contains(&Value::String(comparator.to_string())),
                "missing comparator {comparator}"
            );
        }
    }

    #[test]
    fn chrome_schema_defines_storage_state_item_required_fields() {
        let schema = chrome_input_schema();
        let item_schema = schema
            .pointer("/properties/request/properties/steps/items/properties/items/items")
            .and_then(Value::as_object)
            .unwrap();

        assert_eq!(
            item_schema.get("required"),
            Some(&serde_json::json!(["name", "value"]))
        );
        let properties = item_schema
            .get("properties")
            .and_then(Value::as_object)
            .unwrap();
        assert!(properties.contains_key("name"));
        assert!(properties.contains_key("value"));
    }

    #[test]
    fn chrome_description_points_at_waiting_and_help_keeps_the_detail() {
        let description = ToolChrome::default().tool_description().description;
        assert!(description.contains("wait_for_function"));
        for term in ["expect_poll", "polling_ms", "truthy", "re-resolves"] {
            assert!(
                CHROME_HELP.contains(term),
                "help must retain waiting documentation for {term}"
            );
        }
    }

    #[test]
    fn chrome_description_is_ref_first_batched_and_actionability_aware() {
        let description = ToolChrome::default().tool_description().description;
        let canonical = "{\"steps\":[{\"action\":\"navigate\",\"url\":\"https://example.com\"},{\"action\":\"click\",\"locator\":{\"by\":\"ref\",\"value\":\"e5\"}},{\"action\":\"fill\",\"locator\":{\"by\":\"ref\",\"value\":\"e7\"},\"text\":\"hi\"}]}";
        assert!(description.contains("ONE call can carry many steps"));
        assert!(description.contains(canonical));
        assert!(description.contains("refs come from the most recent snapshot"));
        assert!(description.contains("auto-wait for actionability"));
        assert!(description.contains("Never use `wait_seconds` for readiness"));
        assert!(description.contains("wait_for_response"));
        assert!(description.contains("wait_for_load_state"));
        assert!(description.contains("wait_for_selector"));
        assert!(!description.contains("Use `wait_seconds` for readiness"));
    }

    #[test]
    fn chrome_description_summarizes_capabilities_and_points_at_help() {
        let description = ToolChrome::default().tool_description().description;
        for term in [
            "Capability groups, each with a help topic",
            "The `action` enum below is the authoritative action list",
            "test_id",
            "outermost-first `frames` chain",
            "ambiguous locators fail loudly with the match count",
        ] {
            assert!(description.contains(term), "missing summary term {term}");
        }
        assert!(description.contains("{\"steps\":[{\"action\":\"help\"}]}"));
        assert!(description.contains("never launches the browser"));
        assert!(
            !description.contains("`commands`"),
            "the deprecated commands input must not be advertised to models"
        );
    }

    #[test]
    fn every_advertised_help_topic_resolves_to_a_section() {
        for topic in CHROME_HELP_TOPICS.split(", ") {
            let rendered = chrome_help_text(Some(topic));
            assert!(
                rendered.starts_with(&format!("## {topic}")),
                "advertised topic {topic} does not resolve to a section"
            );
            assert!(
                rendered.len() > format!("## {topic}").len() + 20,
                "help topic {topic} is empty"
            );
        }
    }

    #[test]
    fn help_documents_every_action_the_schema_advertises() {
        let undocumented = crate::integrations::browser_models::BrowserStep::ACTION_NAMES
            .iter()
            .filter(|action| !CHROME_HELP.contains(**action))
            .collect::<Vec<_>>();
        assert!(
            undocumented.is_empty(),
            "slimming the description must not lose capability knowledge: {undocumented:?}"
        );
    }

    #[test]
    fn chrome_tool_contract_stays_within_its_token_budget() {
        let schema_len = serde_json::to_string(&chrome_input_schema()).unwrap().len();
        let description_len = CHROME_DESCRIPTION.len();
        let total = description_len + schema_len;
        assert!(
            total < 32_000,
            "chrome description + schema is {total} chars (description {description_len}, schema {schema_len}), budget is 32000"
        );
        assert!(
            total < 30_500,
            "chrome description + schema is {total} chars (description {description_len}, schema {schema_len}), reclaimed headroom ceiling is 30500"
        );
        assert!(
            description_len < 4_500,
            "chrome description is {description_len} chars, budget is 4500"
        );
    }

    #[test]
    fn help_without_a_topic_returns_the_index_and_the_overview() {
        let rendered = chrome_help_text(None);
        assert!(rendered.starts_with("chrome help topics:"));
        assert!(rendered.contains("routes"));
        assert!(rendered.contains("## overview"));
        assert!(rendered.contains("Text-first batched browser automation"));
    }

    #[test]
    fn help_with_a_topic_returns_just_that_section() {
        let rendered = chrome_help_text(Some("routes"));
        assert!(rendered.starts_with("## routes"));
        assert!(rendered.contains("fetch_and_fulfill"));
        assert!(
            !rendered.contains("## websockets"),
            "a scoped topic must not leak the neighbouring section"
        );
    }

    #[test]
    fn an_unknown_help_topic_returns_the_index_with_an_error_note() {
        let rendered = chrome_help_text(Some("teleportation"));
        assert!(rendered.starts_with("Unknown help topic \"teleportation\""));
        assert!(rendered.contains("chrome help topics:"));
    }

    #[test]
    fn a_help_only_request_is_answered_without_a_browser_session() {
        assert!(help_only_request(
            &serde_json::json!({"steps": [{"action": "help"}]})
        ));
        assert!(help_only_request(
            &serde_json::json!({"steps": [{"action": "help", "topic": "routes"}, {"action": "help"}]})
        ));
        assert!(!help_only_request(&serde_json::json!({"steps": []})));
        assert!(!help_only_request(&serde_json::json!({
            "steps": [{"action": "help"}, {"action": "navigate", "url": "https://example.com"}]
        })));
        assert!(!help_only_request(&serde_json::json!({
            "steps": [{"action": "navigate", "url": "https://example.com"}]
        })));
    }

    #[test]
    fn a_help_only_request_answers_every_step_from_documentation() {
        let response = help_only_response(&serde_json::json!({
            "steps": [{"action": "help", "topic": "locators"}, {"action": "help", "topic": "clock"}]
        }));
        assert!(response.contains("## locators"));
        assert!(response.contains("## clock"));
    }

    #[test]
    fn chrome_help_documents_the_coordinate_handler_bypass_caveat() {
        assert!(CHROME_HELP.contains(
            "Locator handlers and overlay auto-dismiss do NOT guard `mouse_*` coordinate actions"
        ));
    }

    #[test]
    fn wait_for_url_substring_semantics_survive_in_the_schema_and_help() {
        let schema = chrome_input_schema();
        assert!(schema
            .pointer("/properties/request/properties/steps/items/properties/pattern/description")
            .and_then(Value::as_str)
            .is_some_and(|description| description.contains("wait_for_url")
                && description.contains("substring")));
        assert!(CHROME_HELP.contains(
            "`wait_for_url` reads a plain-string `pattern` as a case-sensitive substring"
        ));
    }

    #[test]
    fn help_pins_the_wait_for_function_argument_shape() {
        for phrase in [
            "write `expression` as a function",
            "only a function is handed the element",
            "a plain expression is evaluated without the element being passed to",
            "both a plain expression and a zero-argument function work",
        ] {
            assert!(CHROME_HELP.contains(phrase), "waits help lost {phrase:?}");
        }
    }

    #[test]
    fn help_pins_the_styles_property_filter_substring_semantics() {
        for phrase in [
            "the first 50 followed by a `... (N more)` marker",
            "case-sensitive substring test against each whole `property: value` line",
            "also matches `background-color`",
        ] {
            assert!(
                CHROME_HELP.contains(phrase),
                "readouts help lost {phrase:?}"
            );
        }
        assert!(chrome_input_schema()
            .pointer(
                "/properties/request/properties/steps/items/properties/property_filter/description"
            )
            .and_then(Value::as_str)
            .is_some_and(
                |description| description.contains("case-sensitive substring")
                    && description.contains("property: value")
            ));
    }

    #[test]
    fn help_pins_the_upload_path_policy() {
        for phrase in [
            "is checked before the batch runs",
            "the refact config directory, and pass the privacy rules",
            "take regular",
        ] {
            assert!(CHROME_HELP.contains(phrase), "uploads help lost {phrase:?}");
        }
    }

    #[test]
    fn chrome_schema_offers_the_gallery_and_element_state_captures() {
        let schema = chrome_input_schema();
        let properties = schema
            .pointer("/properties/request/properties/steps/items/properties")
            .and_then(Value::as_object)
            .unwrap();

        assert_eq!(properties["compose"]["anyOf"][0]["type"], "boolean");
        assert_eq!(
            properties["compose"]["anyOf"][1]["enum"],
            serde_json::json!(["grid", "separate"])
        );
        assert_eq!(properties["width"]["anyOf"][0]["type"], "integer");
        assert_eq!(properties["width"]["anyOf"][1]["type"], "string");
        assert_eq!(properties["height"]["anyOf"][1]["type"], "string");
        assert_eq!(
            properties["states"]["items"]["enum"],
            serde_json::json!(["default", "hover", "focus", "active"])
        );
        assert_eq!(properties["labels"]["type"], "boolean");
        assert_eq!(properties["locators"]["type"], "array");

        let actions = properties["action"]["enum"].as_array().unwrap();
        for action in ["screenshot_elements", "capture_element_states"] {
            assert!(
                actions.iter().any(|value| value == action),
                "missing action {action}"
            );
            assert!(CHROME_HELP.contains(action), "help must document {action}");
        }
    }

    #[test]
    fn chrome_schema_documents_tri_state_attach_screenshot() {
        let schema = chrome_input_schema();
        assert_eq!(
            schema
                .pointer("/properties/request/properties/attach_screenshot/description")
                .and_then(Value::as_str),
            Some(
                "Screenshot override: true = always, false = never, omitted = follow page_context"
            )
        );
        let description = ToolChrome::default().tool_description().description;
        assert!(description.contains("`attach_screenshot` is the tri-state override"));
        assert!(description.contains("false = never attach"));
        assert!(description.contains("omitted = follow `page_context`"));
    }

    #[test]
    fn chrome_schema_offers_every_page_context_mode() {
        let schema = chrome_input_schema();
        assert_eq!(
            schema.pointer("/properties/request/properties/page_context/enum"),
            Some(&serde_json::json!([
                "snapshot",
                "screenshot",
                "both",
                "none"
            ]))
        );
        assert!(schema
            .pointer("/properties/request/properties/page_context/description")
            .and_then(Value::as_str)
            .is_some_and(|description| description.contains("snapshot (default)")
                && description.contains("no image")));
    }

    #[test]
    fn chrome_description_teaches_the_text_first_loop_with_opt_in_screenshots() {
        let description = ToolChrome::default().tool_description().description;
        assert!(description.starts_with("Text-first batched browser automation."));
        assert!(description.contains("You read pages as text, not as pictures."));
        assert!(description
            .contains("navigate -> read the returned snapshot refs -> act by ref -> repeat"));
        assert!(description.contains("screenshots are opt-in"));
        assert!(description
            .contains("you do NOT need an `accessibility_snapshot` step after navigating"));
        assert!(description.contains(
            "`snapshot` (the default) attaches the ref-annotated ARIA snapshot and NO image"
        ));
        assert!(description.contains("`none` attaches only the page header"));
    }

    #[test]
    fn chrome_description_documents_the_page_header_and_snapshot_budget() {
        let description = ToolChrome::default().tool_description().description;
        assert!(description.contains("page.status"));
        assert!(description.contains("error/warning COUNTS"));
        assert!(description.contains("full text stays in `console` and `tab_log`"));
        assert!(description.contains("`text/yaml` artifact"));
        assert!(description.contains("locator_echo"));
        assert!(CHROME_HELP.contains("{artifact:{kind,mime,path,bytes}}"));
    }

    #[test]
    fn chrome_schema_documents_network_report_volume() {
        let schema = chrome_input_schema();
        assert_eq!(
            schema.pointer("/properties/request/properties/network/enum"),
            Some(&serde_json::json!(["none", "summary", "full"]))
        );
        assert!(schema
            .pointer("/properties/request/properties/network/description")
            .and_then(Value::as_str)
            .is_some_and(|description| description.contains("summary (default)")
                && description.contains("method url status bytes ms")));
        let description = ToolChrome::default().tool_description().description;
        assert!(description.contains("`network` controls per-request report volume"));
        assert!(description.contains("`summary` (the default)"));
        assert!(description.contains("`none` drops per-request entries"));
        assert!(CHROME_HELP.contains("Route interception telemetry"));
    }

    #[test]
    fn suppressed_report_screenshot_still_returns_the_explicit_step_screenshot() {
        const TINY_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let step =
            crate::integrations::browser_models::StepResult::success(0, "Screenshot captured")
                .with_data(serde_json::json!({
                    "mime": "image/png",
                    "data": TINY_PNG_BASE64,
                }));
        let report: ExecutionReport = serde_json::from_value(serde_json::json!({
            "ok": true,
            "steps": [step],
            "dialogs": [],
            "new_tabs": [],
        }))
        .unwrap();

        assert!(report.screenshot.is_none());
        let content = execution_report_to_multimodal(&report, &ImagePolicy::default()).unwrap();
        assert!(content.iter().any(|element| element.is_image()));
    }

    #[test]
    fn artifact_only_screenshot_payload_is_still_attached_and_keeps_its_path() {
        const TINY_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let data = serde_json::json!({
            "artifact": {
                "kind": "image",
                "mime": "image/png",
                "data": TINY_PNG_BASE64,
                "width": 1,
                "height": 1,
                "bytes": 68,
                "path": "/tmp/artifacts/screenshots/shot-1.png",
            }
        });
        assert_eq!(
            step_inline_image(&data),
            Some(("image/png", TINY_PNG_BASE64))
        );
        assert!(step_inline_image(&serde_json::json!({"artifact": {"kind": "pdf"}})).is_none());

        let step =
            crate::integrations::browser_models::StepResult::success(0, "Screenshot captured")
                .with_data(data);
        let report: ExecutionReport = serde_json::from_value(serde_json::json!({
            "ok": true,
            "steps": [step],
            "dialogs": [],
            "new_tabs": [],
        }))
        .unwrap();
        let content = execution_report_to_multimodal(&report, &ImagePolicy::default()).unwrap();
        assert!(content.iter().any(|element| element.is_image()));
        let text = content
            .iter()
            .filter(|element| !element.is_image())
            .map(|element| element.m_content.clone())
            .collect::<String>();
        assert!(
            text.contains("/tmp/artifacts/screenshots/shot-1.png"),
            "{text}"
        );
        assert!(
            !text.contains(TINY_PNG_BASE64),
            "base64 leaked into text: {text}"
        );
    }

    fn report_with_screenshot(step_data: Value) -> ExecutionReport {
        const TINY_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let step = crate::integrations::browser_models::StepResult::success(0, "Captured frames")
            .with_data(step_data);
        serde_json::from_value(serde_json::json!({
            "ok": true,
            "steps": [step],
            "dialogs": [],
            "new_tabs": [],
            "screenshot": {"mime": "image/png", "data": TINY_PNG_BASE64},
        }))
        .unwrap()
    }

    fn report_with_step_outcomes(report_ok: bool, step_ok: &[bool]) -> ExecutionReport {
        let steps = step_ok
            .iter()
            .enumerate()
            .map(|(index, ok)| {
                serde_json::json!({
                    "step_index": index,
                    "ok": ok,
                    "summary": "step",
                })
            })
            .collect::<Vec<_>>();
        serde_json::from_value(serde_json::json!({
            "ok": report_ok,
            "steps": steps,
            "dialogs": [],
            "new_tabs": [],
        }))
        .unwrap()
    }

    #[test]
    fn a_failed_batch_is_reported_as_a_failure_not_a_success() {
        let failed = report_with_step_outcomes(false, &[true, false]);
        assert_eq!(browser_outcome(&failed), "failure");
        assert!(!failed.ok, "tool_failed is derived from report.ok");
        assert_eq!(
            browser_metadata(&failed)["failed_steps"],
            serde_json::json!([1])
        );
        assert_eq!(browser_metadata(&failed)["outcome"], "failure");
    }

    #[test]
    fn non_fatal_step_failures_stay_partial_and_do_not_flip_the_batch() {
        let partial = report_with_step_outcomes(true, &[true, false]);
        assert_eq!(browser_outcome(&partial), "partial");
        assert!(partial.ok, "non-fatal failures must not flip report.ok");
        assert_eq!(browser_metadata(&partial)["ok"], true);

        let clean = report_with_step_outcomes(true, &[true, true]);
        assert_eq!(browser_outcome(&clean), "success");
        assert_eq!(
            browser_metadata(&clean)["failed_steps"],
            serde_json::json!([])
        );
    }

    #[test]
    fn browser_metadata_uses_safe_urls_and_synthetic_artifact_ids() {
        let report: ExecutionReport = serde_json::from_value(serde_json::json!({
            "ok": true,
            "steps": [{
                "step_index": 0,
                "ok": true,
                "summary": "Generated report",
                "data": {"artifact": {
                    "kind": "pdf",
                    "mime": "application/pdf",
                    "bytes": 4096,
                    "path": "/tmp/private/report.pdf",
                    "data": "secret payload"
                }}
            }],
            "url": "https://user:token@example.test/dashboard?token=secret",
            "network": [{
                "timestamp": 0.0,
                "method": "GET",
                "url": "https://example.test",
                "resource_type": "Document",
                "status": 200
            }],
            "dialogs": [],
            "new_tabs": [],
        }))
        .unwrap();

        let metadata = browser_metadata(&report);

        assert_eq!(metadata["page_url"], "https://example.test/dashboard");
        assert_eq!(metadata["network_count"], 1);
        assert_eq!(metadata["artifacts"][0]["kind"], "pdf");
        assert_eq!(metadata["artifacts"][0]["id"], "browser-0-pdf");
        let serialized = metadata.to_string();
        assert!(!serialized.contains("/tmp/private/report.pdf"));
        assert!(!serialized.contains("secret payload"));
    }

    #[test]
    fn a_snapshot_only_report_delivers_zero_image_bytes_to_the_model() {
        let report: ExecutionReport = serde_json::from_value(serde_json::json!({
            "ok": true,
            "steps": [{"step_index": 0, "ok": true, "summary": "Navigated to https://example.com"}],
            "url": "https://example.com",
            "title": "Example",
            "page": {
                "console": {"errors": 0, "warnings": 0},
                "snapshot": {
                    "yaml": "- button \"Save\" [ref=e1]",
                    "lines": 1,
                    "bytes": 24,
                    "truncated": false
                }
            },
            "dialogs": [],
            "new_tabs": [],
        }))
        .unwrap();

        let content = execution_report_to_multimodal(&report, &ImagePolicy::default()).unwrap();

        assert!(
            !content.iter().any(|element| element.is_image()),
            "the default page context must not ship any image bytes"
        );
        assert_eq!(content.len(), 1);
        assert!(content[0].m_content.contains("[ref=e1]"));
    }

    #[test]
    fn a_report_screenshot_reaches_the_model_on_the_typed_request_path() {
        const TINY_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let report: ExecutionReport = serde_json::from_value(serde_json::json!({
            "ok": true,
            "steps": [{"step_index": 0, "ok": true, "summary": "Navigated"}],
            "dialogs": [],
            "new_tabs": [],
            "screenshot": {"mime": "image/png", "data": TINY_PNG_BASE64},
        }))
        .unwrap();

        let content = execution_report_to_multimodal(&report, &ImagePolicy::default()).unwrap();

        assert_eq!(
            content.iter().filter(|element| element.is_image()).count(),
            1,
            "page_context screenshot mode is useless if the image never reaches the model"
        );
    }

    #[test]
    fn filmstrips_attach_even_when_a_report_screenshot_is_present() {
        const TINY_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let filmstrip = report_with_screenshot(serde_json::json!({
            "mime": "image/png",
            "data": TINY_PNG_BASE64,
            "artifact": {"kind": "filmstrip"},
        }));
        let plain = report_with_screenshot(serde_json::json!({
            "mime": "image/png",
            "data": TINY_PNG_BASE64,
        }));

        assert!(step_image_is_attachable(
            &filmstrip,
            filmstrip.steps[0].data.as_ref().unwrap()
        ));
        assert!(!step_image_is_attachable(
            &plain,
            plain.steps[0].data.as_ref().unwrap()
        ));
        assert_eq!(
            execution_report_to_multimodal(&filmstrip, &ImagePolicy::default())
                .unwrap()
                .iter()
                .filter(|element| element.is_image())
                .count(),
            2
        );
        assert_eq!(
            execution_report_to_multimodal(&plain, &ImagePolicy::default())
                .unwrap()
                .iter()
                .filter(|element| element.is_image())
                .count(),
            1
        );
    }

    #[test]
    fn chrome_schema_documents_the_motion_capture_steps() {
        use refact_browser::screencast::{MAX_SESSION_DURATION_MS, MAX_SESSION_FRAMES};

        let schema = chrome_input_schema();
        let properties = schema
            .pointer("/properties/request/properties/steps/items/properties")
            .and_then(Value::as_object)
            .unwrap();
        for field in [
            "duration_ms",
            "frame_count",
            "interval_ms",
            "compose",
            "max_width",
            "max_height",
        ] {
            assert!(
                properties.contains_key(field),
                "missing schema field {field}"
            );
        }
        assert_eq!(
            properties["duration_ms"]["maximum"],
            serde_json::json!(MAX_BURST_DURATION_MS)
        );
        assert_eq!(
            properties["frame_count"]["minimum"],
            serde_json::json!(MIN_FRAME_COUNT)
        );
        assert_eq!(
            properties["frame_count"]["maximum"],
            serde_json::json!(MAX_FRAME_COUNT)
        );
        assert!(CHROME_HELP.contains("capture_frames"));
        assert!(CHROME_HELP.contains(&MAX_SESSION_DURATION_MS.to_string()));
        assert!(CHROME_HELP.contains(&MAX_SESSION_FRAMES.to_string()));
    }
}

async fn setup_chrome_session(
    gcx: Arc<GlobalContext>,
    args: &SettingsChrome,
    session_hashmap_key: &String,
    chat_id: &str,
) -> Result<Vec<String>, String> {
    let mut setup_log = vec![];
    let launch_lock = relaunch_lock_for_chat(chat_id);
    let _launch_guard = launch_lock.lock().await;

    let session_entry = {
        let integration_sessions = gcx.integration_sessions.clone();
        let integration_sessions = integration_sessions.lock().await;
        integration_sessions.get(session_hashmap_key).cloned()
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
                let browser_runtimes = gcx.browser_runtimes.clone();
                let browser_runtimes = browser_runtimes.lock().await;
                browser_runtimes.get(&runtime_id).cloned()
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
            setup_log.push(
                "Browser session was dead; relaunching (open tabs lost, cookies/localStorage kept)."
                    .to_string(),
            );
            remove_browser_runtime(
                crate::app_state::AppState::from_gcx(gcx.clone()).await,
                &runtime_id,
            )
            .await;
            let integration_sessions = gcx.integration_sessions.clone();
            let mut integration_sessions = integration_sessions.lock().await;
            let should_remove = integration_sessions
                .get(session_hashmap_key)
                .map(|current| Arc::ptr_eq(current, &session))
                .unwrap_or(false);
            if should_remove {
                integration_sessions.remove(session_hashmap_key);
            }
        }
    }

    if let Some((runtime_id, _)) = find_runtime_by_chat_id(
        crate::app_state::AppState::from_gcx(gcx.clone()).await,
        chat_id,
    )
    .await
    {
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
        gcx.integration_sessions.lock().await.insert(
            session_hashmap_key.clone(),
            Arc::new(AMutex::new(command_session)),
        );
        return Ok(setup_log);
    }

    let launch_options = args.launch_options();
    let idle_browser_timeout = launch_options.idle_timeout_or_default();

    let runtime = if args.chrome_path.starts_with("ws://") {
        setup_log.push("Connect to existing web socket.".to_string());
        BrowserRuntime::connect(args.chrome_path.clone(), launch_options)?
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
        BrowserRuntime::connect(ws_url, launch_options)?
    } else {
        let cache_dir = gcx.cache_dir.clone();
        let profile_dir = get_browser_profile_dir(&cache_dir, chat_id);
        clear_stale_profile_lock(&profile_dir).map_err(|error| {
            format!("Cannot start chrome: {error}; the previous browser never exited")
        })?;

        setup_log.push(format!(
            "Started new chrome process ({}).",
            launch_options.mode_label()
        ));
        let mode = launch_options.mode_label();
        BrowserRuntime::launch(profile_dir.clone(), launch_options).map_err(|error| {
            format!(
                "Failed to start chrome in {mode} mode: {}",
                describe_launch_failure(&error, &profile_dir)
            )
        })?
    };

    let runtime_id = {
        let mut rt = runtime;
        rt.reattach(chat_id);
        // Set up recording so Browser Mode can attach to this runtime later
        if let Err(e) = setup_recording_for_runtime(&mut rt) {
            tracing::warn!("Browser recording setup failed (non-fatal): {}", e);
        }
        register_browser_runtime(crate::app_state::AppState::from_gcx(gcx.clone()).await, rt).await
    };

    setup_log.push("No opened tabs at this moment.".to_string());

    let command_session: Box<dyn IntegrationSession> = Box::new(ChromeSession {
        runtime_id,
        tabs: HashMap::new(),
        idle_timeout: idle_browser_timeout,
        last_activity: Instant::now(),
    });
    gcx.integration_sessions.lock().await.insert(
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
    gcx: Arc<GlobalContext>,
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
                    let browser_runtimes = gcx.browser_runtimes.clone();
                    let browser_runtimes = browser_runtimes.lock().await;
                    browser_runtimes
                        .get(&chrome_session.runtime_id)
                        .ok_or_else(|| {
                            format!(
                                "BrowserRuntime {} not found. Browser may have been closed.",
                                chrome_session.runtime_id
                            )
                        })?
                        .clone()
                };
                let runtime_lock = runtime_arc.lock().await;
                runtime_lock.browser.new_tab().map_err(|e| e.to_string())?
            };
            let current_settings = browser_settings::current();
            let browser_viewport = &current_settings.viewport;
            let method = match device {
                DeviceType::Desktop => {
                    let (width, height) = match (
                        settings_chrome.window_width.parse::<u32>(),
                        settings_chrome.window_height.parse::<u32>(),
                    ) {
                        (Ok(width), Ok(height)) => (width, height),
                        _ => (
                            browser_viewport.desktop.width,
                            browser_viewport.desktop.height,
                        ),
                    };
                    let scale_factor = match settings_chrome.scale_factor.parse::<f64>() {
                        Ok(scale_factor) => scale_factor,
                        _ => browser_viewport.desktop.scale_factor,
                    };
                    set_device_metrics_method(width, height, scale_factor, false)
                }
                DeviceType::Mobile => {
                    let (width, height) = match (
                        settings_chrome.mobile_window_width.parse::<u32>(),
                        settings_chrome.mobile_window_height.parse::<u32>(),
                    ) {
                        (Ok(width), Ok(height)) => (width, height),
                        _ => (
                            browser_viewport.mobile.width,
                            browser_viewport.mobile.height,
                        ),
                    };
                    let scale_factor = match settings_chrome.mobile_scale_factor.parse::<f64>() {
                        Ok(scale_factor) => scale_factor,
                        _ => browser_viewport.mobile.scale_factor,
                    };
                    set_device_metrics_method(width, height, scale_factor, true)
                }
                DeviceType::Tablet => {
                    let (width, height) = match (
                        settings_chrome.tablet_window_width.parse::<u32>(),
                        settings_chrome.tablet_window_height.parse::<u32>(),
                    ) {
                        (Ok(width), Ok(height)) => (width, height),
                        _ => (
                            browser_viewport.tablet.width,
                            browser_viewport.tablet.height,
                        ),
                    };
                    let scale_factor = match settings_chrome.tablet_scale_factor.parse::<f64>() {
                        Ok(scale_factor) => scale_factor,
                        _ => browser_viewport.tablet.scale_factor,
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
                        let formatted_ts =
                            crate::integrations::browser_runtime::normalize_timestamp_ms_opt(
                                ts_raw,
                            )
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
                let browser_runtimes = gcx.browser_runtimes.clone();
                let browser_runtimes = browser_runtimes.lock().await;
                if let Some(rt_arc) = browser_runtimes.get(&chrome_session.runtime_id).cloned() {
                    let mut rt = rt_arc.lock().await;
                    let _ = setup_recording_for_tab(&mut rt, runtime_tab.clone());
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
    gcx: Arc<GlobalContext>,
    image_policy: &ImagePolicy,
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
                return Err(format!("Tab '{}' not found. {}", tab_id, suggestion));
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
            let browser_runtimes = gcx.browser_runtimes.clone();
            let browser_runtimes = browser_runtimes.lock().await;
            browser_runtimes.get(&runtime_id).cloned()
        };
        if let Some(arc) = runtime_arc {
            let mut rt = arc.lock().await;
            rt.touch();
        }
    }

    let report = tokio::task::block_in_place(|| {
        browser_controller::execute_steps(&headless_tab, &steps, image_policy)
    });

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
            let browser_runtimes = gcx.browser_runtimes.clone();
            let browser_runtimes = browser_runtimes.lock().await;
            browser_runtimes.get(&runtime_id).cloned()
        };
        if let Some(arc) = runtime_arc {
            let mut rt = arc.lock().await;
            for step_result in &report.steps {
                let action_type = if step_result.ok { "action" } else { "error" };
                rt.push_agent_action(action_type, &step_result.summary);
            }
        }
    }

    Ok(format_controller_report(&report, &tab_state, image_policy))
}

fn step_image_is_attachable(report: &ExecutionReport, data: &Value) -> bool {
    report.screenshot.is_none() || data["artifact"]["kind"] == "filmstrip"
}

fn inline_image_fields(node: &Value) -> Option<(&str, &str)> {
    let mime = node.get("mime")?.as_str()?;
    let encoded = node.get("data")?.as_str()?;
    mime.starts_with("image/").then_some((mime, encoded))
}

fn step_inline_image(data: &Value) -> Option<(&str, &str)> {
    inline_image_fields(data).or_else(|| data.get("artifact").and_then(inline_image_fields))
}

/// `success` when nothing failed, `partial` when only non-fatal steps (soft assertions,
/// `click_if_exists`) failed and the batch still reports `ok`, `failure` otherwise.
fn browser_outcome(report: &ExecutionReport) -> &'static str {
    let any_step_failed = report.steps.iter().any(|step| !step.ok);
    match (report.ok, any_step_failed) {
        (false, _) => "failure",
        (true, true) => "partial",
        (true, false) => "success",
    }
}

fn browser_metadata(report: &ExecutionReport) -> Value {
    let mut artifacts = report
        .steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| step.data.as_ref().map(|data| (index, data)))
        .filter_map(|(index, data)| data.get("artifact").map(|artifact| (index, artifact)))
        .filter_map(|(index, artifact)| {
            let kind = artifact.get("kind")?.as_str()?;
            let mime = artifact.get("mime")?.as_str()?;
            let bytes = artifact.get("bytes")?.as_u64()?;
            Some(serde_json::json!({
                "id": format!("browser-{index}-{kind}"),
                "kind": kind,
                "mime": mime,
                "bytes": bytes,
            }))
        })
        .collect::<Vec<_>>();
    if let Some(snapshot) = report
        .page
        .as_ref()
        .and_then(|page| page.snapshot.as_ref())
        .and_then(|snapshot| snapshot.artifact.as_ref())
    {
        artifacts.push(serde_json::json!({
            "id": "browser-page-snapshot",
            "kind": snapshot.kind,
            "mime": snapshot.mime,
            "bytes": snapshot.bytes,
        }));
    }
    artifacts.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    artifacts.dedup_by(|left, right| left["id"] == right["id"]);

    serde_json::json!({
        "outcome": browser_outcome(report),
        "ok": report.ok,
        "failed_steps": report
            .steps
            .iter()
            .filter(|step| !step.ok)
            .map(|step| step.step_index)
            .collect::<Vec<_>>(),
        "page_url": refact_chat_api::sanitize_http_url(report.url.as_deref().unwrap_or_default()),
        "status": report.page.as_ref().and_then(|page| page.status),
        "network_count": report.network.len(),
        "console_errors": report.page.as_ref().map(|page| page.console.errors).unwrap_or_default(),
        "console_warnings": report.page.as_ref().map(|page| page.console.warnings).unwrap_or_default(),
        "artifacts": artifacts,
    })
}

fn format_controller_report(
    report: &ExecutionReport,
    tab_state: &str,
    image_policy: &ImagePolicy,
) -> (Vec<String>, Vec<MultimodalElement>) {
    let mut log = Vec::new();
    let mut multimodal = Vec::new();

    for warning in &report.warnings {
        log.push(format!("warning: {}", warning));
    }

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
            if step_image_is_attachable(report, data) {
                if let Some((mime, b64_data)) = step_inline_image(data) {
                    match resize_screenshot_b64(b64_data, mime, image_policy) {
                        Ok((resized, resized_mime)) => {
                            if let Ok(el) = MultimodalElement::new(resized_mime, resized) {
                                multimodal.push(el);
                            }
                        }
                        Err(e) => log.push(format!("Screenshot processing: {}", e)),
                    }
                }
            }
        }

        if let Some(ref data) = result.data {
            if data["artifact"]["kind"] == "pdf" {
                if let (Some(path), Some(bytes)) = (
                    data["artifact"]["path"].as_str(),
                    data["artifact"]["bytes"].as_u64(),
                ) {
                    log.push(format!("PDF artifact: {path} ({bytes} bytes)"));
                }
            } else if data["artifact"]["kind"] == "coverage" {
                if let (Some(path), Some(bytes), Some(resources)) = (
                    data["artifact"]["path"].as_str(),
                    data["artifact"]["bytes"].as_u64(),
                    data["artifact"]["resource_count"].as_u64(),
                ) {
                    log.push(format!(
                        "Coverage artifact: {path} ({resources} resources, {bytes} bytes)"
                    ));
                }
            }
            format_step_data(data, &mut log);
        }
    }

    if let Some(screenshot) = &report.screenshot {
        match resize_screenshot_b64(&screenshot.data, &screenshot.mime, image_policy) {
            Ok((resized, resized_mime)) => {
                if let Ok(element) = MultimodalElement::new(resized_mime, resized) {
                    multimodal.push(element);
                }
            }
            Err(error) => log.push(format!("Screenshot processing: {error}")),
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
    image_policy: &ImagePolicy,
) -> Result<Vec<MultimodalElement>, String> {
    let mut content = Vec::new();

    let mut text_report = serde_json::to_value(report)
        .map_err(|e| format!("Failed to serialize browser report: {}", e))?;
    redact_browser_report(&mut text_report);
    strip_binary_data_for_text(&mut text_report);
    let text_pretty = serde_json::to_string_pretty(&text_report)
        .map_err(|e| format!("Failed to pretty-print browser report: {}", e))?;
    content.push(MultimodalElement::new("text".to_string(), text_pretty)?);

    for result in &report.steps {
        if let Some(ref data) = result.data {
            if !step_image_is_attachable(report, data) {
                continue;
            }
            if let Some((mime, b64_data)) = step_inline_image(data) {
                let (resized, resized_mime) = resize_screenshot_b64(b64_data, mime, image_policy)?;
                if let Ok(el) = MultimodalElement::new(resized_mime, resized) {
                    content.push(el);
                }
            }
            for (mime, b64_data) in step_gallery_images(data) {
                let (resized, resized_mime) =
                    resize_screenshot_b64(&b64_data, &mime, image_policy)?;
                if let Ok(el) = MultimodalElement::new(resized_mime, resized) {
                    content.push(el);
                }
            }
        }
    }
    if let Some(screenshot) = &report.screenshot {
        let (resized, resized_mime) =
            resize_screenshot_b64(&screenshot.data, &screenshot.mime, image_policy)?;
        if let Ok(element) = MultimodalElement::new(resized_mime, resized) {
            content.push(element);
        }
    }
    Ok(content)
}

fn redact_browser_credential_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for key in [
                "private_key",
                "credential_id",
                "user_handle",
                "large_blob",
                "user_name",
                "user_display_name",
            ] {
                if map.contains_key(key) {
                    map.insert(
                        key.to_string(),
                        serde_json::Value::String("[REDACTED]".to_string()),
                    );
                }
            }
            for child in map.values_mut() {
                redact_browser_credential_fields(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                redact_browser_credential_fields(child);
            }
        }
        _ => {}
    }
}

/// Keys whose string payloads are free-form page/script output rather than engine-owned
/// identifiers, so their contents also take the path-and-URL pass from `redact_sensitive`.
const BROWSER_FREE_FORM_KEYS: &[&str] = &[
    "value",
    "values",
    "description",
    "text",
    "texts",
    "html",
    "content",
    "expected",
    "received",
    "diff",
    "arguments",
    "entries",
    "rows",
    "cells",
    "links",
    "styles",
    "tree",
    "snippet",
    "message",
    "body",
    "post_data",
];

/// Keys naming engine-minted artifacts the caller must reuse verbatim (`image_region`,
/// `route_from_har`), so they never take the path pass.
const BROWSER_OPAQUE_PATH_KEYS: &[&str] = &["path", "save_as", "artifact_path", "file_name"];

fn browser_secret_token_patterns() -> &'static [(regex::Regex, &'static str)] {
    static PATTERNS: std::sync::OnceLock<Vec<(regex::Regex, &'static str)>> =
        std::sync::OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            (
                regex::Regex::new(r"\bxox[abceprs]-[A-Za-z0-9-]{10,}").unwrap(),
                "[REDACTED_SLACK_TOKEN]",
            ),
            (
                regex::Regex::new(r"\bxapp-[0-9]-[A-Za-z0-9-]{10,}").unwrap(),
                "[REDACTED_SLACK_TOKEN]",
            ),
            (
                regex::Regex::new(
                    r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
                )
                .unwrap(),
                "[REDACTED_JWT]",
            ),
            (
                regex::Regex::new(r"\b(?:AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16}\b").unwrap(),
                "[REDACTED_AWS_KEY]",
            ),
            (
                regex::Regex::new(r"\bgh[opsur]_[A-Za-z0-9]{10,}\b").unwrap(),
                "[REDACTED_GH_TOKEN]",
            ),
            (
                regex::Regex::new(r"\bgithub_pat_[A-Za-z0-9_]{20,}\b").unwrap(),
                "[REDACTED_GH_TOKEN]",
            ),
            (
                regex::Regex::new(r"\b[sprk]k_(?:live|test)_[A-Za-z0-9]{10,}\b").unwrap(),
                "[REDACTED_STRIPE_KEY]",
            ),
            (
                regex::Regex::new(r"\bAIza[0-9A-Za-z_-]{30,}\b").unwrap(),
                "[REDACTED_GOOGLE_KEY]",
            ),
            (
                regex::Regex::new(r"\bnpm_[A-Za-z0-9]{30,}\b").unwrap(),
                "[REDACTED_NPM_TOKEN]",
            ),
            (
                regex::Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{12,}").unwrap(),
                "Bearer [REDACTED]",
            ),
            (
                regex::Regex::new(
                    r#"(?i)\b(api[_-]?key|apikey|access[_-]?token|auth[_-]?token|token|secret|password|passwd)\b\s*["']?\s*[:=]\s*["']?[^\s"',;)}\]]{6,}"#,
                )
                .unwrap(),
                "$1=[REDACTED]",
            ),
        ]
    })
}

fn redact_browser_secret_tokens(text: &str) -> String {
    let mut out = std::borrow::Cow::Borrowed(text);
    for (pattern, replacement) in browser_secret_token_patterns() {
        if pattern.is_match(&out) {
            out = std::borrow::Cow::Owned(pattern.replace_all(&out, *replacement).into_owned());
        }
    }
    out.into_owned()
}

fn redact_browser_payload_strings(value: &mut serde_json::Value, free_form: bool) {
    match value {
        serde_json::Value::String(text) => {
            let scanned = if free_form {
                refact_core::string_utils::redact_sensitive(text)
            } else {
                text.clone()
            };
            let redacted = redact_browser_secret_tokens(&scanned);
            if redacted != *text {
                *text = redacted;
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_browser_payload_strings(item, free_form);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                let child_free_form = if BROWSER_OPAQUE_PATH_KEYS.contains(&key.as_str()) {
                    false
                } else {
                    free_form || BROWSER_FREE_FORM_KEYS.contains(&key.as_str())
                };
                redact_browser_payload_strings(child, child_free_form);
            }
        }
        _ => {}
    }
}

fn redact_browser_report(value: &mut serde_json::Value) {
    redact_browser_credential_fields(value);
    redact_browser_payload_strings(value, false);
}

#[cfg(test)]
mod browser_credential_redaction_tests {
    use super::{redact_browser_credential_fields, redact_browser_report, redact_browser_secret_tokens};

    #[test]
    fn browser_credential_request_fields_are_redacted_recursively() {
        let mut value = serde_json::json!({
            "credential": {
                "credential_id": "secret-id",
                "private_key": "secret-key",
                "user_handle": "secret-handle",
                "rp_id": "example.com"
            }
        });
        redact_browser_credential_fields(&mut value);
        let serialized = serde_json::to_string(&value).unwrap();
        assert!(!serialized.contains("secret-id"));
        assert!(!serialized.contains("secret-key"));
        assert!(!serialized.contains("secret-handle"));
        assert!(serialized.contains("example.com"));
    }

    #[test]
    fn eval_results_and_readouts_lose_their_secrets() {
        let mut report = serde_json::json!({
            "steps": [
                {"data": {"value": "Bearer FAKEBEARER1234567890abcdefghij", "description": "String"}},
                {"data": {"value": {"slack": "xoxc-1234567890-abcdefghijklmnop"}}},
                {"data": {"texts": ["session token=hunter2hunter2", "harmless copy"]}},
                {"data": {"text": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1"}},
                {"assertion": {"expected": "xoxb-9876543210-zyxwvutsrq", "received": "xoxp-1122334455-abcdefghij"}}
            ]
        });
        redact_browser_report(&mut report);
        let serialized = serde_json::to_string(&report).unwrap();

        for secret in [
            "FAKEBEARER1234567890abcdefghij",
            "xoxc-1234567890-abcdefghijklmnop",
            "hunter2hunter2",
            "xoxb-9876543210-zyxwvutsrq",
            "xoxp-1122334455-abcdefghij",
            "dBjftJeZ4CVPmB92K27uhbUJU1p1",
        ] {
            assert!(
                !serialized.contains(secret),
                "leaked {secret}: {serialized}"
            );
        }
        assert!(serialized.contains("harmless copy"));
    }

    #[test]
    fn ordinary_page_text_and_artifact_paths_survive_redaction() {
        let mut report = serde_json::json!({
            "steps": [{
                "data": {
                    "text": "Sign in to continue. Forgot your password? Reset it here.",
                    "artifact": {"kind": "image", "path": "/home/dev/.cache/refact/browser/shot-1.png"}
                }
            }]
        });
        redact_browser_report(&mut report);

        assert_eq!(
            report["steps"][0]["data"]["artifact"]["path"],
            "/home/dev/.cache/refact/browser/shot-1.png"
        );
        let text = report["steps"][0]["data"]["text"].as_str().unwrap();
        assert!(
            text.contains("Sign in to continue"),
            "over-redacted: {text}"
        );
        assert!(
            text.contains("Forgot your password?"),
            "over-redacted: {text}"
        );
    }

    #[test]
    fn slack_and_high_entropy_shapes_are_covered_by_the_token_scan() {
        for secret in [
            "xoxc-2468013579-qwertyuiopasdfgh",
            "xoxb-1111111111-aaaaaaaaaaaa",
            "xapp-1-A012BCDEF-1234567890",
            "AKIAIOSFODNN7EXAMPLE",
            "ghs_abcdefghijklmnopqrstuvwxyz012345",
        ] {
            let redacted = redact_browser_secret_tokens(secret);
            assert!(!redacted.contains(secret), "not redacted: {redacted}");
        }
        assert_eq!(
            redact_browser_secret_tokens("click the Save button"),
            "click the Save button"
        );
    }
}

fn step_gallery_images(data: &serde_json::Value) -> Vec<(String, String)> {
    data.get("images")
        .and_then(|images| images.as_array())
        .map(|images| {
            images
                .iter()
                .filter_map(|image| {
                    let mime = image.get("mime").and_then(|value| value.as_str())?;
                    let encoded = image.get("data").and_then(|value| value.as_str())?;
                    mime.starts_with("image/")
                        .then(|| (mime.to_string(), encoded.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn strip_binary_data_for_text(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let is_binary = map
                .get("mime")
                .and_then(|v| v.as_str())
                .map(|mime| mime.starts_with("image/") || mime == "application/pdf")
                .unwrap_or(false);
            if is_binary {
                let b64_len = map
                    .get("data")
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
                strip_binary_data_for_text(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                strip_binary_data_for_text(v);
            }
        }
        _ => {}
    }
}

fn resize_screenshot_b64(
    b64_data: &str,
    mime: &str,
    image_policy: &ImagePolicy,
) -> Result<(String, String), String> {
    let raw = base64::prelude::BASE64_STANDARD
        .decode(b64_data)
        .map_err(|e| format!("Base64 decode failed: {}", e))?;
    let (resized, resized_mime) = resize_to_policy(&raw, mime, image_policy)?;
    Ok((
        base64::prelude::BASE64_STANDARD.encode(resized),
        resized_mime,
    ))
}

fn format_step_data(data: &serde_json::Value, log: &mut Vec<String>) {
    let mut redacted_data = data.clone();
    redact_browser_report(&mut redacted_data);
    let data = &redacted_data;
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
    gcx: Arc<GlobalContext>,
    settings_chrome: &SettingsChrome,
    image_policy: &ImagePolicy,
) -> Result<(Vec<String>, Vec<MultimodalElement>), String> {
    if browser_actions::to_browser_steps(action).is_some() {
        return execute_via_controller(action, chrome_session, gcx, image_policy).await;
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
                        format!("clicked `{} {}` at {}", x, y, tab_lock.state_string())
                    }
                    Err(e) => {
                        format!(
                            "clicked `{} {}` failed at {}: {}",
                            x,
                            y,
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
                        format!("type text failed at {}: {}", tab_lock.state_string(), e)
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
                let tabs: Vec<(String, Arc<AMutex<ChromeTab>>)> = cs
                    .tabs
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                (tabs, cs.runtime_id.clone())
            };
            let runtime_tabs = {
                let browser_runtimes = gcx.browser_runtimes.clone();
                let browser_runtimes = browser_runtimes.lock().await;
                if let Some(rt_arc) = browser_runtimes.get(&runtime_id).cloned() {
                    let rt = rt_arc.lock().await;
                    rt.browser
                        .get_tabs()
                        .lock()
                        .map(|tabs| {
                            tabs.iter()
                                .map(|t| {
                                    (
                                        t.get_target_id().to_string(),
                                        t.get_url(),
                                        t.get_title().unwrap_or_default(),
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            };
            if session_tabs.is_empty() && runtime_tabs.is_empty() {
                tool_log.push(
                    "No tabs are currently open. Use 'open_tab <tab_id> desktop' to open one."
                        .to_string(),
                );
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
                let extra_tabs: Vec<_> = runtime_tabs
                    .iter()
                    .filter(|(tid, _, _)| !session_target_ids.contains(tid))
                    .collect();
                if !extra_tabs.is_empty() {
                    tool_log.push(format!("Runtime tabs ({}):", extra_tabs.len()));
                    for (tid, url, title) in &extra_tabs {
                        tool_log.push(format!(
                            "  target={} url={} title={}",
                            &tid[..8.min(tid.len())],
                            url,
                            title
                        ));
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
                                let browser_runtimes = gcx.browser_runtimes.clone();
                                let browser_runtimes = browser_runtimes.lock().await;
                                browser_runtimes.get(&runtime_id).cloned()
                            };
                            if let Some(arc) = runtime_arc {
                                let mut rt = arc.lock().await;
                                if rt.recording_tab_target_id.as_deref() == Some(&target_id) {
                                    rt.recording_tab_target_id = None;
                                }
                                if rt.active_tab_target_id().as_deref() == Some(target_id.as_str())
                                {
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
