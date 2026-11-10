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
    find_runtime_by_chat_id, launch_with_lock_recovery, runtime_owner_record, transport_death_note,
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
}

#[derive(Default)]
pub struct ToolChrome {
    pub settings_chrome: SettingsChrome,
    pub config_path: String,
}

// DeviceType is now in browser_actions module

const MAX_CACHED_LOG_LINES: usize = 1000;

const CHROME_HELP: &str = include_str!("chrome_help.md");

const CHROME_HELP_TOPICS: &str = "overview, locators, navigation, input, forms, waits, network, routes, websockets, har, tabs, frames, dialogs, uploads, downloads, emulation, devices, clock, screenshots, artifacts, screencast, assertions, readouts, handlers, cdp, authenticators, reset, sessions, discovery, troubleshooting";

const CHROME_DESCRIPTION: &str = concat!(
    "Text-first batched browser automation. You read pages as text, not as pictures. Prefer the typed `request`: ONE call can carry many steps, unlike one-action-per-call servers. ",
    "The loop is: navigate -> read the returned snapshot refs -> act by ref -> repeat. Any batch that changes the page attaches a ref-annotated ARIA snapshot under `page.snapshot` automatically, so you do NOT need an `accessibility_snapshot` step after navigating; screenshots are opt-in and cost far more than the text tree. ",
    "Each `[ref=eN]` handle in that snapshot is an element address: act with `locator.by=ref`; refs come from the most recent snapshot. Use an explicit `accessibility_snapshot` step only to re-read a page that did NOT change, or to scope to a subtree with `locator`/`depth`.\n",
    "Canonical batch: {\"steps\":[{\"action\":\"navigate\",\"url\":\"https://example.com\"},{\"action\":\"click\",\"locator\":{\"by\":\"ref\",\"value\":\"e5\"}},{\"action\":\"fill\",\"locator\":{\"by\":\"ref\",\"value\":\"e7\"},\"text\":\"hi\"}]} Pass this object as `request`; e5/e7 stand for handles minted by the snapshot the previous batch returned.\n",
    "TUTORIAL: run {\"steps\":[{\"action\":\"help\"}]} for the full tutorial covering every action, parameter, and gotcha, or {\"action\":\"help\",\"topic\":\"<name>\"} for one section. Topics: ",
    "overview, locators, navigation, input, forms, waits, network, routes, websockets, har, tabs, frames, dialogs, uploads, downloads, emulation, devices, clock, screenshots, artifacts, screencast, assertions, readouts, handlers, cdp, authenticators, reset, sessions, discovery, troubleshooting",
    ". help answers from documentation and never launches the browser, so it is free to consult before a run.\n",
    "Capability groups, each with a help topic: navigation and tabs; locator-driven input plus a mouse_*/tap coordinate escape hatch; forms; waits; assertions (expect, expect_poll); readouts (bounding_box, count, input_value, all_texts, element_state); inspection and snapshots; screenshots, pdf, and filmstrip capture; network interception (route, HAR, WebSocket) and http_request; emulation, devices, storage, and cookies; the fake clock; locator handlers and dialogs; coverage and virtual authenticators; cdp_send and reset. The `action` enum below is the authoritative action list.\n",
    "Locators are ref-first with a fallback vocabulary (role, test_id, text, label, placeholder, alt_text, title, css, xpath, id, name, autocomplete) composable with nth/first/last, filter, and/or, and an outermost-first `frames` chain. Non-selecting actions are strict: ambiguous locators fail loudly with the match count.\n",
    "Page report: a page-changing batch returns `page` with the final URL and title, `page.status` for a non-2xx main document, `page.console` error/warning COUNTS (full text stays in `console` and `tab_log`), and `page.snapshot`, inlined when small and otherwise a `text/yaml` artifact plus the head, `lines`, `bytes`, and `truncated:true`. Locator-driven actions report `locator_echo`, a canonical locator generated for the element actually acted on.\n",
    "`page_context` picks the page-changed context: `snapshot` (the default) attaches the ref-annotated ARIA snapshot and NO image, `screenshot` attaches a policy-sized image instead, `both` attaches each, `none` attaches only the page header. `attach_screenshot` is the tri-state override and wins over `page_context`: true = always attach, false = never attach, omitted = follow `page_context`. `network` controls per-request report volume: `summary` (the default) emits one `method url status bytes ms` line per request, `full` keeps headers, `none` drops per-request entries.\n",
    "Click, hover, fill, clear, check, and uncheck auto-wait for actionability. Never use `wait_seconds` for readiness; use `wait_for_response`, `wait_for_load_state`, `wait_for_selector`, or `wait_for_function`.\n",
    "Result: one JSON object with `tool`, `summary`, `completion` (steps_requested/reported, failed, unknown, skipped, incomplete) plus `ok`, `steps`, `page`. Under a tight budget it is reduced as JSON: failed and effect-unknown steps are prioritized, with explicit aggregate verdicts and omission counts when step details cannot fit. If even the minimum verdict cannot fit, delivery fails explicitly rather than reporting success. At most 64 steps per call; invalid requests are rejected before the browser starts.\n",
    "Recovery: if `recovery.state_lost` is true, nothing interrupted was retried; resend with `acknowledge_session_generation` set to the report's `session_generation`, then navigate again (old refs and tab ids are invalid).\n",
    "Large redacted reports may include `full_report.id`. Read it with `read_artifact` using that opaque id and byte `offset`/`limit` (default 8192, maximum 32768). The result's `content_base64` encodes the exact UTF-8 bytes; decode and concatenate chunks using `next_offset`. Retrieval is scoped to the current runtime, does not create another report artifact, and expired or evicted ids fail explicitly. Managed artifacts are bounded to 64 files, 32 MiB total, 8 MiB per file, with one-hour idle TTL. Secure managed storage is supported on Unix and Windows."
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
    for (action, required, optional, _) in
        crate::integrations::browser_models::BrowserStep::ACTION_FIELDS.iter()
    {
        if !actions.contains(action) {
            continue;
        }
        for field in required.iter().chain(optional.iter()) {
            properties.entry((*field).to_string()).or_insert_with(
                || serde_json::json!({"description": format!("{action} field; see help")}),
            );
        }
    }
    if let Some(offset) = properties.get_mut("offset") {
        *offset = serde_json::json!({"type": "integer", "minimum": 0, "description": "read_artifact byte offset"});
    }
    schema["anyOf"] = Value::Array(browser_action_variants(actions));
    schema
}

/// Nested per-action discriminated variants derived from [`BrowserStep::ACTION_FIELDS`].
/// Field types stay in the flat sibling `properties`; each variant only pins which keys the
/// strict parser accepts for its action. Providers that flatten unions (Google CloudCode
/// `sanitize_schema`) keep the flat properties, so the flat bag is the conservative fallback.
/// Actions with serde aliases only require `action`: the table does not map alias to field.
fn browser_action_variants(actions: &[&str]) -> Vec<Value> {
    crate::integrations::browser_models::BrowserStep::ACTION_FIELDS
        .iter()
        .filter(|(action, ..)| actions.contains(action))
        .map(|(action, required, optional, aliases)| {
            let mut properties = serde_json::Map::new();
            properties.insert("action".to_string(), serde_json::json!({"enum": [action]}));
            for field in required.iter().chain(optional.iter()).chain(aliases.iter()) {
                properties.insert((*field).to_string(), serde_json::json!({}));
            }
            let mut required_fields = vec![Value::String("action".to_string())];
            if aliases.is_empty() {
                required_fields.extend(required.iter().map(|f| Value::String((*f).to_string())));
            }
            serde_json::json!({
                "properties": properties,
                "required": required_fields,
                "additionalProperties": false
            })
        })
        .collect()
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
            "session": {"type": "string", "maxLength":64,"description":"shared_default (default), or an explicitly created named session"},
            "expected_session_id": {"type":"string","maxLength":128,"description":"Required opaque session UUID for every named request; prevents stale name reuse"},
            "acknowledge_session_generation": {"type": "string", "description": "Recovery handshake: after a report says browser state was lost, resend with the `session_generation` value it returned. Then navigate again; old refs/tab ids are invalid and interrupted actions were NOT retried"},
            "page_context": {"type": "string", "enum": ["snapshot", "screenshot", "both", "none"], "description": "snapshot (default) attaches the aria snapshot and no image; screenshot attaches a PNG; both; none = page header only"},
            "attach_screenshot": {"type": "boolean", "description": "Screenshot override: true = always, false = never, omitted = follow page_context"},
            "network": {"type": "string", "enum": ["none", "summary", "full"], "description": "summary (default) = one `method url status bytes ms` line per request, full keeps headers, none drops entries"},
            "block_service_workers": {"type": "boolean", "description": "Block service workers for this context (sticky until reset)"},
            "continue_on_error": {"type": "boolean", "description": "Run later steps after a failure (default false); ok stays false"},
            "target": tab_target_schema(),
            "steps": {
                "type": "array",
                "maxItems": MAX_CHROME_BATCH_STEPS,
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
        // Strict validation must precede even the documentation-only fast path.
        let preflight_request = match args.get("request") {
            Some(request_value) => Some(preflight_browser_request(request_value)?),
            None => None,
        };
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

        if let Some(request) = preflight_request.as_ref() {
            use crate::integrations::browser_models::{BrowserStep, SessionPolicy};
            use crate::integrations::browser_runtime::named_sessions;
            let lifecycle = request.steps.first().is_some_and(|step| {
                matches!(
                    step,
                    BrowserStep::CreateSession { .. }
                        | BrowserStep::ListSessions
                        | BrowserStep::CloseSession { .. }
                )
            });
            if lifecycle || matches!(request.session, SessionPolicy::Named(_)) {
                let (owner, gcx, model) = {
                    let context = ccx.lock().await;
                    (
                        named_sessions::NamedSessionOwner::from_context(&context)?,
                        context.app.gcx.clone(),
                        context.current_model.clone(),
                    )
                };
                let deadline = Instant::now() + Duration::from_secs(120);
                if lifecycle {
                    let lifecycle_result = match &request.steps[0] {
                        BrowserStep::CreateSession {
                            name,
                        } => named_sessions::create_named_session(
                            owner,
                            name,
                            self.settings_chrome.launch_options(),
                            deadline,
                        )
                        .await
                        .map(|created| (serde_json::json!(created), true)),
                        BrowserStep::ListSessions => named_sessions::list_named_sessions(&owner)
                            .map(|sessions| (serde_json::json!({"sessions": sessions}), true))
                            .map_err(named_sessions::NamedLifecycleError::from),
                        BrowserStep::CloseSession {
                            name,
                            expected_session_id,
                        } => named_sessions::close_named_session(
                            &owner,
                            name,
                            expected_session_id,
                            deadline,
                        )
                        .await
                        .map(|closed| {
                            let ok = closed.process_exited
                                && closed.targets_complete
                                && closed.profile_disposition == "removed";
                            (serde_json::json!(closed), ok)
                        }),
                        _ => unreachable!(),
                    };
                    let report = named_lifecycle_report(lifecycle_result);
                    let content = execution_report_to_multimodal_with_request(
                        &report,
                        &ImagePolicy::default(),
                        Some(1),
                    )?;
                    let message = ContextEnum::ChatMessage(ChatMessage {
                        role: "tool".into(),
                        tool_call_id: tool_call_id.clone(),
                        content: ChatContent::Multimodal(content),
                        ..Default::default()
                    });
                    return Ok((!report.ok, vec![message]));
                }
                if let SessionPolicy::Named(name) = &request.session {
                    let expected = request
                        .expected_session_id
                        .as_deref()
                        .ok_or("Named request requires expected_session_id")?;
                    let named = named_sessions::resolve_named_session(&owner, name, expected)?;
                    let image_policy = image_policy_for_model(gcx.clone(), &model).await;
                    let report_result = browser_controller::execute_request_with_runtime_validated(
                        named.runtime.clone(),
                        request.clone(),
                        &image_policy,
                        gcx,
                    )
                    .await;
                    let report = report_result?;
                    let mut content = execution_report_to_multimodal_with_request(
                        &report,
                        &image_policy,
                        Some(request.steps.len()),
                    )?;
                    retain_full_browser_report(
                        &named.runtime,
                        &mut content,
                        request
                            .steps
                            .iter()
                            .all(|step| matches!(step, BrowserStep::ReadArtifact { .. })),
                    )
                    .await?;
                    drop(named);
                    let message = ContextEnum::ChatMessage(ChatMessage {
                        role: "tool".into(),
                        tool_call_id: tool_call_id.clone(),
                        content: ChatContent::Multimodal(content),
                        ..Default::default()
                    });
                    return Ok((!report.ok, vec![message]));
                }
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

        if let Some(request) = preflight_request {
            let steps_requested = request.steps.len();
            let retrieval_only = request.steps.iter().all(|step| {
                matches!(
                    step,
                    crate::integrations::browser_models::BrowserStep::ReadArtifact { .. }
                )
            });

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
                runtime_arc.clone(),
                request,
                &image_policy,
                gcx.clone(),
            )
            .await
            {
                Ok(report) => {
                    tool_failed = !report.ok;
                    let mut content = execution_report_to_multimodal_with_request(
                        &report,
                        &image_policy,
                        Some(steps_requested),
                    )?;
                    retain_full_browser_report(&runtime_arc, &mut content, retrieval_only).await?;
                    typed_content = Some(content);
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
    use serde_json::json;

    #[test]
    fn retained_browser_reports_keep_redacted_bytes_in_the_selected_runtime_store() {
        let first_root = tempfile::tempdir().unwrap();
        let second_root = tempfile::tempdir().unwrap();
        let first = Arc::new(
            refact_browser::artifact_store::ArtifactStore::new(first_root.path()).unwrap(),
        );
        let second = Arc::new(
            refact_browser::artifact_store::ArtifactStore::new(second_root.path()).unwrap(),
        );
        let report: ExecutionReport = serde_json::from_value(json!({
            "ok": true,
            "steps": [{"step_index": 0, "ok": true, "summary": "Large result",
                "data": {"text": "x".repeat(20_000), "password": "known-fixture-secret"}}],
            "dialogs": [], "new_tabs": []
        }))
        .unwrap();
        let original = browser_report_text(&report, Some(1)).unwrap();
        assert!(!original.contains("known-fixture-secret"));
        let mut text = MultimodalElement::new("text".into(), original.clone()).unwrap();
        retain_full_browser_report_text(&mut text, Ok(first.clone())).unwrap();
        let envelope: Value = serde_json::from_str(&text.m_content).unwrap();
        let id = envelope["full_report"]["id"].as_str().unwrap();
        assert_eq!(envelope["full_report"]["bytes"], original.len());
        assert_eq!(first.read_report(id, 0, 32768).unwrap().content, original);
        assert!(second.read_report(id, 0, 32768).is_err());
        let mut failed = MultimodalElement::new("text".into(), original).unwrap();
        retain_full_browser_report_text(&mut failed, Err("store unavailable".into())).unwrap();
        let envelope: Value = serde_json::from_str(&failed.m_content).unwrap();
        assert_eq!(envelope["artifact_error"], "store unavailable");
        assert!(envelope.get("full_report").is_none());
    }

    #[test]
    fn named_lifecycle_errors_preserve_dispatch_stage_and_disjoint_completion() {
        use crate::integrations::browser_runtime::named_sessions::{
            NamedLifecycleError, NamedLifecycleStage,
        };
        for (stage, failed, unknown, incomplete) in [
            (NamedLifecycleStage::BeforeDispatch, 1, 0, false),
            (NamedLifecycleStage::AfterDispatchUnknown, 0, 1, true),
            (NamedLifecycleStage::AfterDispatchCleanupKnown, 1, 0, false),
        ] {
            let report = named_lifecycle_report(Err(NamedLifecycleError {
                stage,
                session_id: Some("session-id".into()),
                message: "lifecycle fixture".into(),
            }));
            let text = browser_report_text(&report, Some(1)).unwrap();
            let envelope: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(envelope["ok"], false);
            assert_eq!(envelope["completion"]["steps_reported"], 1);
            assert_eq!(envelope["completion"]["failed"], failed);
            assert_eq!(envelope["completion"]["unknown"], unknown);
            assert_eq!(envelope["completion"]["incomplete"], incomplete);
            assert_eq!(envelope["steps"][0]["data"]["session_id"], "session-id");
            if stage != NamedLifecycleStage::BeforeDispatch {
                assert_eq!(envelope["recovery"]["retry_safe"], false);
                assert_eq!(envelope["recovery"]["state_lost"], false);
            } else {
                assert!(envelope.get("recovery").is_none());
            }
        }
    }

    fn step_variant_admits(variants: &[Value], step: &Value) -> bool {
        let object = step.as_object().unwrap();
        variants.iter().any(|variant| {
            let properties = variant["properties"].as_object().unwrap();
            let action_ok = properties["action"]["enum"]
                .as_array()
                .unwrap()
                .contains(&object["action"]);
            let required_ok = variant["required"]
                .as_array()
                .unwrap()
                .iter()
                .all(|field| object.contains_key(field.as_str().unwrap()));
            let keys_ok = object.keys().all(|key| properties.contains_key(key));
            action_ok && required_ok && keys_ok
        })
    }

    #[test]
    fn action_variants_agree_with_the_strict_runtime_parser() {
        let schema = browser_request_schema();
        let step_schema = &schema["properties"]["steps"];
        assert_eq!(step_schema["maxItems"], MAX_CHROME_BATCH_STEPS);
        let variants = step_schema["items"]["anyOf"].as_array().unwrap();
        let fields = crate::integrations::browser_models::BrowserStep::ACTION_FIELDS;
        assert_eq!(variants.len(), fields.len());
        let flat = step_schema["items"]["properties"].as_object().unwrap();
        for (_, required, optional, _) in fields {
            for key in required.iter().chain(optional.iter()) {
                assert!(
                    flat.contains_key(*key),
                    "variant field {key} missing in flat bag"
                );
            }
        }
        let cases = [
            (
                json!({"action": "navigate", "url": "https://example.com"}),
                true,
            ),
            (
                json!({"action": "navigate", "url": "https://example.com", "timeout_ms": 10}),
                true,
            ),
            (json!({"action": "navigate"}), false),
            (
                json!({"action": "navigate", "url": "https://example.com", "text": "x"}),
                false,
            ),
            (
                json!({"action": "get_text", "locator": {"by": "ref", "value": "e1"}}),
                true,
            ),
            (json!({"action": "get_text"}), false),
            (
                json!({"action": "get_text", "locator": {"by": "ref", "value": "e1"}, "url": "u"}),
                false,
            ),
            (json!({"action": "tab_log"}), true),
            (json!({"action": "tab_log", "url": "u"}), false),
            (json!({"action": "help", "topic": "routes"}), true),
            (
                json!({"action": "help", "locator": {"by": "ref", "value": "e1"}}),
                false,
            ),
            (json!({"action": "wait_for_url", "contains": "x"}), true),
            (json!({"action": "wait_for_url", "pattern": "x"}), true),
            (
                json!({"action": "send_web_socket_message", "url_pattern": "x", "data": "y"}),
                true,
            ),
            (
                json!({"action": "accessibility_snapshot", "root": {"by": "ref", "value": "e1"}}),
                true,
            ),
            (json!({"action": "no_such_action"}), false),
        ];
        for (step, expected) in cases {
            let runtime = crate::integrations::browser_models::parse_browser_action_request(
                json!({"steps": [step.clone()]}),
            );
            assert_eq!(runtime.is_ok(), expected, "runtime {step}: {runtime:?}");
            assert_eq!(
                step_variant_admits(variants, &step),
                expected,
                "schema {step}"
            );
        }
        assert!(
            crate::integrations::browser_models::BrowserStep::ACTION_FIELDS
                .iter()
                .any(|(name, ..)| *name == "read_artifact")
        );
        assert_eq!(
            step_schema["items"]["properties"]["offset"]["type"],
            "integer"
        );
        for (step, expected) in [
            (
                json!({"action": "read_artifact", "id": "a1", "offset": 0, "limit": 10}),
                true,
            ),
            (json!({"action": "read_artifact"}), false),
            (
                json!({"action": "read_artifact", "id": "a1", "url": "u"}),
                false,
            ),
        ] {
            let runtime = crate::integrations::browser_models::parse_browser_action_request(
                json!({"steps": [step.clone()]}),
            );
            assert_eq!(runtime.is_ok(), expected, "runtime {step}: {runtime:?}");
            assert_eq!(
                step_variant_admits(variants, &step),
                expected,
                "schema {step}"
            );
        }
        let ack = json!({"acknowledge_session_generation": "g1", "steps": [{"action": "tab_log"}]});
        assert!(crate::integrations::browser_models::parse_browser_action_request(ack).is_ok());
        assert!(schema["properties"]
            .as_object()
            .unwrap()
            .contains_key("acknowledge_session_generation"));
    }

    #[test]
    fn provider_adapters_keep_a_usable_browser_schema() {
        use refact_llm::adapter::AdapterSettings;
        let tool = json!({"type": "function", "function": {
            "name": "chrome", "description": "browser", "parameters": chrome_input_schema()}});
        assert!(chrome_input_schema().get("anyOf").is_none());
        assert!(chrome_input_schema().get("oneOf").is_none());
        let settings = |format_header: bool| {
            let mut extra_headers = HashMap::new();
            if format_header {
                extra_headers.insert(
                    "x-refact-internal-cloudcode-project".to_string(),
                    "project".to_string(),
                );
            }
            AdapterSettings {
                api_key: "test".to_string(),
                auth_token: "test".to_string(),
                endpoint: "https://example.invalid".to_string(),
                extra_headers,
                model_name: "model".to_string(),
                supports_tools: true,
                supports_reasoning: false,
                reasoning_type: None,
                supports_temperature: false,
                supports_max_completion_tokens: false,
                eof_is_done: false,
                supports_web_search: false,
                supports_cache_control: false,
            }
        };
        let body = |format: refact_llm::WireFormat| {
            let request = refact_llm::LlmRequest::new(
                "model".to_string(),
                vec![ChatMessage::new("user".to_string(), "hi".to_string())],
            )
            .with_tools(vec![tool.clone()], None);
            refact_llm::get_adapter(format)
                .build_http(
                    &refact_privacy::testing::cleared(request),
                    &settings(format == refact_llm::WireFormat::GoogleCloudCode),
                )
                .unwrap()
                .body
        };
        let anthropic = body(refact_llm::WireFormat::AnthropicMessages);
        let input_schema = &anthropic["tools"][0]["input_schema"];
        assert!(input_schema.get("anyOf").is_none());
        let items = &input_schema["properties"]["request"]["properties"]["steps"]["items"];
        assert!(items["anyOf"].as_array().is_some_and(|v| !v.is_empty()));
        assert_eq!(
            input_schema["properties"]["request"]["properties"]["acknowledge_session_generation"]
                ["type"],
            "string"
        );
        assert!(step_variant_admits(
            items["anyOf"].as_array().unwrap(),
            &json!({"action": "read_artifact", "id": "a1", "offset": 0, "limit": 10})
        ));

        let google = body(refact_llm::WireFormat::GoogleCloudCode);
        let parameters = &google["request"]["tools"][0]["functionDeclarations"][0]["parameters"];
        let text = parameters.to_string();
        for forbidden in ["\"anyOf\"", "\"oneOf\"", "additionalProperties"] {
            assert!(
                !text.contains(forbidden),
                "{forbidden} survived google sanitize"
            );
        }
        let items = &parameters["properties"]["request"]["properties"]["steps"]["items"];
        assert_eq!(items["required"], json!(["action"]));
        assert_eq!(
            parameters["properties"]["request"]["properties"]["acknowledge_session_generation"]
                ["type"],
            "string"
        );
        assert!(items["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("read_artifact")));
        for field in [
            "url", "locator", "topic", "pattern", "contains", "id", "offset", "limit",
        ] {
            assert!(
                items["properties"].get(field).is_some(),
                "{field} lost in fallback"
            );
        }
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn invalid_help_tool_execution_never_creates_runtime_or_session() {
        let cache = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let command = crate::global_context::CommandLine::from_iter_safe([
            "strict-help-test",
            "--http-port",
            "0",
            "--lsp-port",
            "0",
            "--no-scheduler",
        ])
        .unwrap();
        let (gcx, _) = crate::global_context::create_global_context(
            cache.path().into(),
            config.path().into(),
            command,
        )
        .await;
        let context = AtCommandsContext::new(
            gcx.clone(),
            4096,
            10,
            false,
            vec![],
            "strict-help-owner".into(),
            None,
            String::new(),
            None,
            None,
        )
        .await;
        let context = Arc::new(AMutex::new(context));
        let mut tool = ToolChrome::default();
        for request in [
            json!({"steps":[{"action":"help","unknown":true}]}),
            json!({"unknown":true,"steps":[{"action":"help"}]}),
            json!({"steps":vec![json!({"action":"help"});65]}),
        ] {
            let args = HashMap::from([("request".into(), request)]);
            assert!(tool
                .tool_execute(context.clone(), &"help-test".into(), &args)
                .await
                .is_err());
        }
        let args = HashMap::from([("request".into(), json!({"steps":[{"action":"help"}]}))]);
        assert!(
            !tool
                .tool_execute(context, &"valid-help".into(), &args)
                .await
                .unwrap()
                .0
        );
        assert!(gcx.browser_runtimes.lock().await.is_empty());
        assert!(gcx.integration_sessions.lock().await.is_empty());
    }

    #[test]
    fn incomplete_cleanup_receipt_is_not_double_counted_as_unknown() {
        let report = ExecutionReport {
            ok: false,
            steps: vec![crate::integrations::browser_models::StepResult::failure(
                0,
                "Session cleanup incomplete",
                "Target inventory incomplete",
            )
            .with_data(json!({"process_exited":true,"targets_complete":false}))],
            ..Default::default()
        };
        let content =
            execution_report_to_multimodal_with_request(&report, &ImagePolicy::default(), Some(1))
                .unwrap();
        let value: Value = serde_json::from_str(&content[0].m_content).unwrap();
        assert_eq!(value["completion"]["failed"], 1);
        assert_eq!(value["completion"]["unknown"], 0);
        assert_eq!(value["steps"][0]["data"]["targets_complete"], false);
    }

    #[test]
    fn named_and_discovery_preflight_refuses_unsafe_contracts() {
        for invalid in [
            json!({"session":"named","steps":[{"action":"tab_log"}]}),
            json!({"session":"../escape","expected_session_id":"id","steps":[{"action":"tab_log"}]}),
            json!({"steps":[{"action":"create_session","name":"x"},{"action":"tab_log"}]}),
            json!({"session":"x","expected_session_id":"id","steps":[{"action":"list_sessions"}]}),
            json!({"steps":[{"action":"close_session","name":"x","expected_session_id":""}]}),
            json!({"steps":[{"action":"search_snapshot","pattern":"x","max_results":201}]}),
            json!({"steps":[{"action":"network_requests","limit":0}]}),
            json!({"steps":[{"action":"network_request","request_id":""}]}),
        ] {
            assert!(preflight_browser_request(&invalid).is_err(), "{invalid}");
        }
        for valid in [
            json!({"steps":[{"action":"create_session","name":"isolated-1"}]}),
            json!({"session":"isolated-1","expected_session_id":"opaque","steps":[{"action":"tab_log"}]}),
            json!({"steps":[{"action":"search_snapshot","pattern":"Save","include_frames":true,"max_frame_depth":2}]}),
            json!({"steps":[{"action":"network_request","request_id":"opaque","include_body":true}]}),
        ] {
            assert!(preflight_browser_request(&valid).is_ok(), "{valid}");
        }
        let schema = browser_request_schema();
        let variants = schema["properties"]["steps"]["items"]["anyOf"]
            .as_array()
            .unwrap();
        for step in [
            json!({"action":"create_session","name":"x"}),
            json!({"action":"search_snapshot","pattern":"x","include_frames":true}),
            json!({"action":"network_request","request_id":"id","include_body":true}),
        ] {
            assert!(step_variant_admits(variants, &step), "{step}");
        }
        assert_eq!(
            schema["properties"]["expected_session_id"]["type"],
            "string"
        );
    }

    #[test]
    fn help_fast_path_requires_strict_bounded_preflight() {
        for invalid in [
            json!({"steps": [{"action": "help", "unknown": true}]}),
            json!({"unknown": true, "steps": [{"action": "help"}]}),
            json!({"steps": [{"action": "help", "topic": 1}]}),
            json!({"steps": vec![json!({"action": "help"}); MAX_CHROME_BATCH_STEPS + 1]}),
        ] {
            assert!(help_only_request(&invalid));
            assert!(preflight_browser_request(&invalid).is_err(), "{invalid}");
        }
        let valid = json!({"steps": [{"action": "help", "topic": "waits"}]});
        assert!(preflight_browser_request(&valid).is_ok());
        assert!(help_only_response(&valid).starts_with("## waits\n"));
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
        let overview = chrome_help_text(Some("overview"));
        assert!(overview.contains("interrupted actions are never replayed"));
        assert!(overview.contains("acknowledge_session_generation"));
        assert!(!overview.contains("resumed after replaying"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn unhealthy_setup_retains_registered_runtime_and_owned_process() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let cache = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let cmdline = crate::global_context::CommandLine::from_iter_safe([
            "browser-setup-e2e",
            "--http-port",
            "0",
            "--lsp-port",
            "0",
            "--no-scheduler",
        ])
        .unwrap();
        let (gcx, _) = crate::global_context::create_global_context(
            cache.path().to_path_buf(),
            config.path().to_path_buf(),
            cmdline,
        )
        .await;
        let chat_id = format!("browser-setup-{}", uuid::Uuid::new_v4());
        let key = get_session_hashmap_key("chrome", &chat_id);
        let settings = SettingsChrome {
            headless: "true".to_string(),
            chromium_sandbox: "false".to_string(),
            ..Default::default()
        };
        setup_chrome_session(gcx.clone(), &settings, &key, &chat_id)
            .await
            .unwrap();
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let (generation, original) = find_runtime_by_chat_id(app.clone(), &chat_id)
            .await
            .unwrap();
        let pid = {
            let mut rt = original.lock().await;
            let pid = rt.ownership.process_id().unwrap();
            rt.browser.disconnect_transport();
            assert!(!rt.check_connection());
            pid
        };
        let result = setup_chrome_session(gcx.clone(), &settings, &key, &chat_id).await;
        let resolved = find_runtime_by_chat_id(app.clone(), &chat_id).await;
        let retained = resolved
            .as_ref()
            .is_some_and(|(id, runtime)| id == &generation && Arc::ptr_eq(runtime, &original));
        let process_alive = crate::integrations::browser_runtime::pid_is_alive(pid);
        gcx.integration_sessions.lock().await.remove(&key);
        if let Some((id, _)) = resolved {
            remove_browser_runtime(app.clone(), &id).await;
        }
        if !retained {
            remove_browser_runtime(app, &generation).await;
        }
        assert!(
            result.is_ok(),
            "setup must defer recovery to the controller: {result:?}"
        );
        assert!(
            retained,
            "unhealthy setup replaced the registered browser generation"
        );
        assert!(
            process_alive,
            "unhealthy setup terminated the owned live browser"
        );
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

    let mut dead_pid = None;
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

        let runtime_arc = gcx.browser_runtimes.lock().await.get(&runtime_id).cloned();
        if let Some(arc) = runtime_arc {
            let rt = arc.lock().await;
            if rt.attached_chat_id().as_deref() != Some(chat_id) {
                return Err("Browser session belongs to another chat".to_string());
            }

            return Ok(setup_log);
        } else {
            dead_pid = runtime_owner_record(&runtime_id).and_then(|owner| owner.process_id());
            setup_log.push(format!(
                "{}; relaunching (open tabs lost, cookies/localStorage kept).",
                transport_death_note(dead_pid, None)
            ));
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
        let mode = launch_options.mode_label();
        let runtime = launch_with_lock_recovery(profile_dir, launch_options, dead_pid)
            .await
            .map_err(|error| format!("Failed to start chrome in {mode} mode: {error}"))?;
        setup_log.push(format!("Started new chrome process ({mode})."));
        runtime
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

async fn retain_full_browser_report(
    runtime: &Arc<AMutex<BrowserRuntime>>,
    content: &mut [MultimodalElement],
    retrieval_only: bool,
) -> Result<(), String> {
    let Some(text) = content.iter_mut().find(|element| element.is_text()) else {
        return Ok(());
    };
    if retrieval_only || text.m_content.len() <= 16 * 1024 {
        return Ok(());
    }
    let store = runtime.lock().await.get_store();
    retain_full_browser_report_text(text, store)
}

fn retain_full_browser_report_text(
    text: &mut MultimodalElement,
    store: Result<Arc<refact_browser::artifact_store::ArtifactStore>, String>,
) -> Result<(), String> {
    let mut value: Value = serde_json::from_str(&text.m_content)
        .map_err(|error| format!("Invalid browser report: {error}"))?;
    match store.and_then(|store| store.write(text.m_content.as_bytes(), "application/json")) {
        Ok(artifact) => {
            value["full_report"] = serde_json::json!({
                "id": artifact.id,
                "mime": artifact.mime,
                "bytes": artifact.bytes,
                "retrieval": "read_artifact",
            });
        }
        Err(error) => value["artifact_error"] = Value::String(error),
    }
    text.m_content = serde_json::to_string(&value)
        .map_err(|error| format!("Invalid browser report: {error}"))?;
    Ok(())
}

fn named_lifecycle_report(
    result: Result<
        (Value, bool),
        crate::integrations::browser_runtime::named_sessions::NamedLifecycleError,
    >,
) -> ExecutionReport {
    use crate::integrations::browser_models::{BrowserRecovery, StepResult};
    use crate::integrations::browser_runtime::named_sessions::NamedLifecycleStage;

    let (step, recovery) = match result {
        Ok((value, true)) => (
            StepResult::success(0, "Session lifecycle completed").with_data(value),
            None,
        ),
        Ok((value, false)) => (
            StepResult::failure(
                0,
                "Session cleanup incomplete",
                "See cleanup receipt: incomplete target inventory or process exit verification; no action was replayed",
            )
            .with_data(value),
            None,
        ),
        Err(error) => {
            let unknown = error.stage == NamedLifecycleStage::AfterDispatchUnknown;
            let data = serde_json::json!({
                "stage": error.stage,
                "session_id": error.session_id,
                "retry_safe": error.stage == NamedLifecycleStage::BeforeDispatch,
            });
            let mut step = StepResult::failure(0, "Session lifecycle did not complete", error.message)
                .with_data(data);
            if unknown {
                step.outcome = Some("effect_unknown".into());
            }
            let recovery = (error.stage != NamedLifecycleStage::BeforeDispatch).then_some(
                BrowserRecovery {
                    incomplete: unknown,
                    state_lost: false,
                    effect_unknown: unknown,
                    retry_safe: false,
                },
            );
            (step, recovery)
        }
    };
    ExecutionReport {
        ok: step.ok,
        steps: vec![step],
        recovery,
        ..Default::default()
    }
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

/// Fixed upper bound on typed steps per call, checked before any browser setup. It bounds report
/// size but does not guarantee the report fits a given budget; the postprocessor reduces it.
const MAX_CHROME_BATCH_STEPS: usize = 64;

fn preflight_browser_request(
    request_value: &Value,
) -> Result<crate::integrations::browser_models::BrowserActionRequest, String> {
    if let Some(count) = request_value
        .get("steps")
        .and_then(Value::as_array)
        .map(Vec::len)
    {
        if count > MAX_CHROME_BATCH_STEPS {
            return Err(format!(
                "argument `request` is invalid: {count} steps exceed the limit of {MAX_CHROME_BATCH_STEPS} per call; split the batch. No browser action was run."
            ));
        }
    }
    let parsed = parse_browser_action_request(request_value.clone()).map_err(|e| {
        format!("argument `request` is invalid: {e}. No browser action was run; see {{\"steps\":[{{\"action\":\"help\"}}]}} for per-action fields.")
    })?;
    use crate::integrations::browser_models::{BrowserStep, SessionPolicy, valid_browser_session_name};
    let lifecycle_count = parsed
        .steps
        .iter()
        .filter(|step| {
            matches!(
                step,
                BrowserStep::CreateSession { .. }
                    | BrowserStep::ListSessions
                    | BrowserStep::CloseSession { .. }
            )
        })
        .count();
    if lifecycle_count > 0
        && request_value
            .as_object()
            .is_some_and(|object| object.keys().any(|key| key != "steps"))
    {
        return Err("Session lifecycle accepts only steps at the request boundary".into());
    }
    if lifecycle_count > 0
        && (parsed.steps.len() != 1
            || parsed.session != SessionPolicy::SharedDefault
            || parsed.expected_session_id.is_some()
            || parsed.acknowledge_session_generation.is_some()
            || parsed.block_service_workers.is_some()
            || !matches!(
                parsed.target,
                crate::integrations::browser_models::TabTarget::Active
            ))
    {
        return Err("Session lifecycle requires a standalone request without page/session options; no browser action was run".into());
    }
    match &parsed.session {
        SessionPolicy::Named(_)
            if parsed
                .expected_session_id
                .as_ref()
                .is_none_or(|id| id.is_empty() || id.len() > 128) =>
        {
            return Err(
                "Named request requires bounded expected_session_id; no browser action was run"
                    .into(),
            )
        }
        SessionPolicy::SharedDefault if parsed.expected_session_id.is_some() => {
            return Err("expected_session_id is only valid for named sessions".into())
        }
        _ => {}
    }
    for step in &parsed.steps {
        use crate::integrations::browser_models::BrowserStep;
        match step {
            BrowserStep::CreateSession {
                name,
            }
            | BrowserStep::CloseSession {
                name, ..
            } if !valid_browser_session_name(name) => {
                return Err("Invalid named session name".into())
            }
            BrowserStep::CloseSession {
                expected_session_id,
                ..
            } if expected_session_id.is_empty() || expected_session_id.len() > 128 => {
                return Err("Invalid expected_session_id".into())
            }
            BrowserStep::SearchSnapshot {
                pattern,
                max_results,
                max_frame_depth,
                subtree,
                ..
            } => {
                if pattern.is_empty()
                    || pattern.len() > 4096
                    || max_results.is_some_and(|n| !(1..=200).contains(&n))
                    || max_frame_depth.is_some_and(|n| n > 8)
                    || subtree.as_ref().is_some_and(|s| s.len() > 256)
                {
                    return Err(
                        "Invalid bounded snapshot search options; no browser action was run".into(),
                    );
                }
            }
            BrowserStep::AccessibilitySnapshot {
                options,
            } if options.max_frame_depth.is_some_and(|n| n > 8) => {
                return Err("max_frame_depth exceeds 8".into())
            }
            BrowserStep::NetworkRequests {
                limit,
                url_contains,
                method,
                ..
            } if limit.is_some_and(|n| !(1..=200).contains(&n))
                || url_contains.as_ref().is_some_and(|s| s.len() > 4096)
                || method.as_ref().is_some_and(|s| s.len() > 32) =>
            {
                return Err("Invalid bounded network filters".into())
            }
            BrowserStep::NetworkRequest {
                request_id, ..
            } if request_id.is_empty() || request_id.len() > 256 => {
                return Err("Invalid opaque network request ID".into())
            }
            _ => {}
        }
    }
    Ok(parsed)
}

/// Completion accounting for the compact report envelope. `unknown` counts steps whose
/// effect could not be established; they are never folded into success or failure.
fn browser_completion(report: &ExecutionReport, steps_requested: Option<usize>) -> Value {
    let outcome_is = |step: &crate::integrations::browser_models::StepResult, name: &str| {
        step.outcome.as_deref() == Some(name)
    };
    let unknown = report
        .steps
        .iter()
        .filter(|step| outcome_is(step, "effect_unknown"))
        .count();
    let skipped = report
        .steps
        .iter()
        .filter(|step| step.skipped || outcome_is(step, "skipped"))
        .count();
    let failed = report
        .steps
        .iter()
        .filter(|step| {
            !step.ok
                && !step.skipped
                && !outcome_is(step, "effect_unknown")
                && !outcome_is(step, "skipped")
        })
        .count();
    let reported = report.steps.len();
    let incomplete = report.recovery.as_ref().is_some_and(|r| r.incomplete)
        || steps_requested.is_some_and(|requested| requested > reported)
        || unknown > 0;
    serde_json::json!({
        "steps_requested": steps_requested,
        "steps_reported": reported,
        "failed": failed,
        "unknown": unknown,
        "skipped": skipped,
        "incomplete": incomplete,
        "context_warnings": report.warnings.len(),
    })
}

fn browser_report_summary(report: &ExecutionReport, completion: &Value) -> String {
    let count = |key: &str| completion[key].as_u64().unwrap_or(0);
    let mut summary = format!(
        "{}: {} of {} steps reported, {} failed, {} effect unknown, {} skipped",
        browser_outcome(report),
        count("steps_reported"),
        completion["steps_requested"]
            .as_u64()
            .unwrap_or(count("steps_reported")),
        count("failed"),
        count("unknown"),
        count("skipped"),
    );
    if completion["incomplete"] == true {
        summary.push_str("; INCOMPLETE");
    }
    if let Some(recovery) = &report.recovery {
        if recovery.state_lost {
            summary.push_str("; browser state lost: resend with acknowledge_session_generation");
        }
        if !recovery.retry_safe {
            summary.push_str("; not retry-safe");
        }
    }
    summary
}

/// Text half of the typed browser result: the redacted, binary-free report wrapped in an
/// envelope that names the producing tool and carries `summary` and `completion`. The
/// envelope keeps the report's own top-level fields (`ok`, `steps`, `page`, ...) and is the
/// shape the shared postprocessor reduces as JSON instead of cutting lines.
fn browser_report_text(
    report: &ExecutionReport,
    steps_requested: Option<usize>,
) -> Result<String, String> {
    let mut text_report = serde_json::to_value(report)
        .map_err(|e| format!("Failed to serialize browser report: {}", e))?;
    redact_browser_execution_report(&mut text_report, report);
    strip_binary_data_for_text(&mut text_report);
    let completion = browser_completion(report, steps_requested);
    let mut envelope = serde_json::Map::new();
    envelope.insert("tool".to_string(), Value::String("chrome".to_string()));
    envelope.insert(
        "summary".to_string(),
        Value::String(browser_report_summary(report, &completion)),
    );
    envelope.insert("completion".to_string(), completion);
    if let Value::Object(fields) = text_report {
        for (key, value) in fields {
            envelope.entry(key).or_insert(value);
        }
    }
    serde_json::to_string(&Value::Object(envelope))
        .map_err(|e| format!("Failed to encode browser report: {}", e))
}

fn execution_report_to_multimodal(
    report: &ExecutionReport,
    image_policy: &ImagePolicy,
) -> Result<Vec<MultimodalElement>, String> {
    execution_report_to_multimodal_with_request(report, image_policy, None)
}

fn execution_report_to_multimodal_with_request(
    report: &ExecutionReport,
    image_policy: &ImagePolicy,
    steps_requested: Option<usize>,
) -> Result<Vec<MultimodalElement>, String> {
    let report_span =
        refact_browser::profiling::Span::new(refact_browser::profiling::Stage::Report);
    let mut content = Vec::new();
    content.push(MultimodalElement::new(
        "text".to_string(),
        browser_report_text(report, steps_requested)?,
    )?);

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
    drop(report_span);
    // Process-wide diagnostics are opt-in and intended for serialized benchmark workers.
    // Never drain or serialize profiling data on the ordinary tool path.
    if headless_chrome::browser::transport::profiling_enabled() {
        let profile = refact_browser::profiling::take();
        let mut envelope: Value = serde_json::from_str(&content[0].m_content)
            .map_err(|error| format!("Invalid browser report: {error}"))?;
        envelope["browser_profile"] = serde_json::to_value(profile)
            .map_err(|error| format!("Invalid browser profile: {error}"))?;
        content[0].m_content = serde_json::to_string(&envelope)
            .map_err(|error| format!("Invalid browser report: {error}"))?;
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
                "password",
                "passwd",
                "pwd",
                "token",
                "access_token",
                "refresh_token",
                "api_key",
                "apikey",
                "authorization",
                "cookie",
                "set-cookie",
                "client_secret",
            ] {
                let matching = map
                    .keys()
                    .filter(|name| name.eq_ignore_ascii_case(key))
                    .cloned()
                    .collect::<Vec<_>>();
                for name in matching {
                    map.insert(name, serde_json::Value::String("[REDACTED]".to_string()));
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
                    r#"(?i)\b(api[_-]?key|apikey|access[_-]?token|auth[_-]?token|token|secret|password|passwd)\b\s*["']?\s*[:=]\s*["']?[^\s"',;)}]{6,}"#,
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
                if key == "body" {
                    if let Some(mut json) = child
                        .as_str()
                        .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    {
                        redact_browser_report(&mut json);
                        if let Ok(text) = serde_json::to_string(&json) {
                            *child = Value::String(text);
                            continue;
                        }
                    }
                }
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

fn redact_browser_execution_report(value: &mut Value, report: &ExecutionReport) {
    let chunks = report
        .steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| {
            if step.ok && step.outcome.as_deref() == Some("artifact_read") {
                step.data
                    .as_ref()?
                    .get("content_base64")?
                    .as_str()
                    .map(|text| (index, text))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    redact_browser_report(value);
    for (index, encoded) in chunks {
        if let Some(content) = value
            .get_mut("steps")
            .and_then(Value::as_array_mut)
            .and_then(|steps| steps.get_mut(index))
            .and_then(|step| step.get_mut("data"))
            .and_then(|data| data.get_mut("content_base64"))
        {
            *content = Value::String(encoded.to_string());
        }
    }
}

#[cfg(test)]
mod browser_credential_redaction_tests {
    use super::{
        browser_report_text, redact_browser_credential_fields, redact_browser_report,
        redact_browser_secret_tokens, ExecutionReport,
    };

    #[test]
    fn persisted_report_masks_key_context_and_retrieval_keeps_redacted_bytes() {
        let mut value = serde_json::json!({
            "data": {"value": {"password": "hunter22", "TOKEN": "bearer-value"}},
            "page": {"url": "https://example.com?x=1"},
        });
        redact_browser_report(&mut value);
        let body = serde_json::to_vec(&value).unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("hunter22"));
        assert!(!String::from_utf8_lossy(&body).contains("bearer-value"));
        let root = tempfile::tempdir().unwrap();
        let store = crate::refact_browser::artifact_store::ArtifactStore::new(root.path()).unwrap();
        let artifact = store.write(&body, "application/json").unwrap();
        let chunk = store.read_report(&artifact.id, 0, 32768).unwrap();
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            chunk.content.as_bytes(),
        );
        let mut response = serde_json::json!({"data": {"content_base64": encoded}});
        redact_browser_report(&mut response);
        let restored = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            response["data"]["content_base64"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(restored, body);
        let bytes = b"\x01\x22\x00BBBBBBBBBBBB";
        let artifact = store.write(bytes, "text/plain").unwrap();
        let chunk = store.read_report(&artifact.id, 0, 32768).unwrap();
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            chunk.content.as_bytes(),
        );
        assert_eq!(encoded, "ASIAQkJCQkJCQkJCQkJC");
        let mut step =
            crate::integrations::browser_models::StepResult::success(0, "Read browser artifact");
        step.outcome = Some("artifact_read".to_string());
        step.data = Some(serde_json::json!({"content_base64": encoded}));
        let report = ExecutionReport {
            ok: true,
            steps: vec![step],
            ..Default::default()
        };
        let text = browser_report_text(&report, Some(1)).unwrap();
        let envelope: serde_json::Value = serde_json::from_str(&text).unwrap();
        let restored = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            envelope["steps"][0]["data"]["content_base64"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(restored, bytes);
    }

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
    fn nested_json_response_bodies_remain_parseable_after_report_redaction() {
        let mut report = serde_json::json!({"steps": [{"data": {"body":
            r#"{"safe":"visible","nested":{"password":"[REDACTED]","token":"[REDACTED]","private_key":"fixture-private-key"}}"#
        }}]});
        redact_browser_report(&mut report);
        let first = report["steps"][0]["data"]["body"]
            .as_str()
            .unwrap()
            .to_owned();
        let parsed: serde_json::Value = serde_json::from_str(&first).unwrap();
        assert_eq!(parsed["safe"], "visible");
        assert_eq!(parsed["nested"]["password"], "[REDACTED]");
        assert_eq!(parsed["nested"]["token"], "[REDACTED]");
        assert_eq!(parsed["nested"]["private_key"], "[REDACTED]");
        redact_browser_report(&mut report);
        assert_eq!(report["steps"][0]["data"]["body"], first);
    }

    #[test]
    fn json_body_scalars_and_credential_strings_are_redacted_idempotently() {
        for json in [
            serde_json::json!({"safe": "token=[REDACTED]"}),
            serde_json::json!({"safe": "token=[REDACTED]fixture-long-secret"}),
            serde_json::json!({"safe": "token=fixture-long-secret"}),
            serde_json::json!("token=fixture-long-secret"),
            serde_json::json!(["token=fixture-long-secret", "ordinary"]),
            serde_json::json!(true),
            serde_json::json!(123),
            serde_json::Value::Null,
        ] {
            let mut report = serde_json::json!({"body": serde_json::to_string(&json).unwrap()});
            redact_browser_report(&mut report);
            let once = report["body"].as_str().unwrap().to_owned();
            let parsed: serde_json::Value = serde_json::from_str(&once).unwrap();
            assert!(!once.contains("fixture-long-secret"));
            assert!(!once.contains("[REDACTED]]"));
            redact_browser_report(&mut report);
            assert_eq!(report["body"], once, "changed parsed body {parsed}");
        }
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
                let encoded = map.get("data").and_then(|v| v.as_str()).unwrap_or_default();
                if !encoded.is_empty() && encoded != "<omitted>" {
                    let padding = encoded
                        .bytes()
                        .rev()
                        .take_while(|byte| *byte == b'=')
                        .count();
                    let bytes = (encoded.len() * 3 / 4).saturating_sub(padding);
                    map.insert(
                        "data".to_string(),
                        serde_json::Value::String("<omitted>".to_string()),
                    );
                    map.entry("bytes".to_string())
                        .or_insert_with(|| serde_json::json!(bytes));
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
        BrowserAction::OpenTab {
            tab_id,
            device,
        } => {
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
        BrowserAction::ClickAtPoint {
            tab_id,
            x,
            y,
        } => {
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
        BrowserAction::TypeText {
            tab_id,
            text,
        } => {
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
        BrowserAction::CloseTab {
            tab_id,
        } => {
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
