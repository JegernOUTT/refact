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
use crate::call_validation::{ChatContent, ChatMessage};
use crate::scratchpads::multimodality::MultimodalElement;

use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

use crate::integrations::browser_actions::{self, BrowserAction, DeviceType};
use crate::integrations::browser_controller;
use crate::integrations::browser_models::{ExecutionReport, parse_browser_action_request};
use crate::integrations::browser_runtime::{
    BrowserLaunchOptions, BrowserProxyOptions, BrowserRuntime, find_runtime_by_chat_id,
    register_browser_runtime, get_browser_profile_dir, setup_recording_for_runtime,
    setup_recording_for_tab,
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
}

impl SettingsChrome {
    pub fn launch_options(&self) -> BrowserLaunchOptions {
        let defaults = BrowserLaunchOptions::default();
        BrowserLaunchOptions {
            headless: self.headless.parse().unwrap_or(defaults.headless),
            chrome_path: (!self.chrome_path.is_empty())
                .then(|| PathBuf::from(self.chrome_path.clone())),
            idle_timeout: self
                .idle_browser_timeout
                .parse::<u64>()
                .ok()
                .map(Duration::from_secs),
            extra_args: self.extra_args.clone(),
            chromium_sandbox: self
                .chromium_sandbox
                .parse()
                .unwrap_or(defaults.chromium_sandbox),
            proxy: (!self.proxy_server.is_empty()).then(|| BrowserProxyOptions {
                server: self.proxy_server.clone(),
                bypass: (!self.proxy_bypass.is_empty()).then(|| self.proxy_bypass.clone()),
            }),
            downloads_dir: (!self.downloads_dir.is_empty())
                .then(|| PathBuf::from(self.downloads_dir.clone())),
            ignore_https_errors: self
                .ignore_https_errors
                .parse()
                .unwrap_or(defaults.ignore_https_errors),
            ..defaults
        }
    }
}

#[derive(Default)]
pub struct ToolChrome {
    pub settings_chrome: SettingsChrome,
    pub config_path: String,
}

// DeviceType is now in browser_actions module

const MAX_CACHED_LOG_LINES: usize = 1000;

const LOCATOR_HANDLER_STEP_ACTIONS: &[&str] =
    crate::integrations::browser_models::BrowserStep::ACTION_NAMES;

const CHROME_DESCRIPTION: &str = concat!(
    "Text-first batched browser automation. You read pages as text, not as pictures. Prefer the typed `request`: ONE call can carry many steps, unlike one-action-per-call servers. ",
    "The loop is: navigate -> read the returned snapshot refs -> act by ref -> repeat. Any batch that changes the page attaches a ref-annotated ARIA snapshot under `page.snapshot` automatically, so you do NOT need an `accessibility_snapshot` step after navigating; screenshots are opt-in and cost far more than the text tree. ",
    "Each `[ref=eN]` handle in that snapshot is an element address: act with `locator.by=ref`; refs come from the most recent snapshot. ",
    "Canonical batch: {\"steps\":[{\"action\":\"navigate\",\"url\":\"https://example.com\"},{\"action\":\"click\",\"locator\":{\"by\":\"ref\",\"value\":\"e5\"}},{\"action\":\"fill\",\"locator\":{\"by\":\"ref\",\"value\":\"e7\"},\"text\":\"hi\"}]} Pass this object as `request`; e5/e7 stand for handles minted by the snapshot the previous batch returned. Use an explicit `accessibility_snapshot` step only to re-read a page that did NOT change, or to scope to a subtree with `locator`/`depth`.\n",
    "Page report: a page-changing batch returns `page` with the final URL and title, `page.status` when the main document answered with a non-2xx status, `page.console` error/warning COUNTS (full text stays in `console` and `tab_log`), and `page.snapshot`. Snapshots inline their YAML when small; a large tree is written to a `text/yaml` artifact and `page.snapshot` carries the head plus `{artifact:{kind,mime,path,bytes}}`, `lines`, `bytes`, and `truncated:true`. Locator-driven actions echo a canonical Playwright-style locator in `locator_echo` so a run stays auditable after the refs expire.\n",
    "Core: navigate, reload, go_back, go_forward, open_tab, close_tab, switch_tab, list_tabs, click, click_if_exists, hover, focus, blur, scroll_to, press_key, drag_and_drop, and drop_files. drag_and_drop accepts source/target locators or refs plus optional source_position/target_position. open_tab accepts optional device/url; close_tab accepts an optional tab and otherwise closes active. Closing active selects the preceding tab in adoption order, the next tab when closing the first, or leaves no active tab.\n",
    "Coordinate mouse escape hatch: mouse_move, mouse_down, mouse_up, mouse_click_xy, mouse_drag_xy, and mouse_wheel use main-frame viewport CSS pixels and bypass locator resolution. Use these only for canvas, map, and vision-driven UIs with no addressable element; locator/ref actions remain the default. ",
    "Locator handlers and overlay auto-dismiss do NOT guard `mouse_*` coordinate actions: an overlay that would be dismissed before a locator action will still swallow a coordinate click.\n",
    "Network: route/unroute/list_routes control HTTP interception. route_web_socket and unroute_web_socket install page-level WebSocket routing; send_web_socket_message supplies mock page messages and wait_for_web_socket_frame waits for observed traffic. start_har_recording and stop_har_recording write a runtime-owned HAR artifact; route_from_har replays it with abort or fallback for misses. HAR output is returned as a path and summary, never inlined. ",
    "reset is the escape hatch for sticky plumbing: one call drops every network route, HAR replay, WebSocket route, locator handler, virtual authenticator, and the fake clock, turns offline off, drops network and CPU throttling, and clears media, viewport, device, geolocation, and permission overrides, reporting what it cleared with counts. It leaves cookies, storage, open tabs, and the current page untouched.\n",
    "Clock: clock_install pins fake time (optional `time` as unix ms or ISO string, current time by default) and must run before the page caches Date; clock_fast_forward jumps ahead firing each due timer AT MOST ONCE while clock_run_for advances firing ALL callbacks along the way, so a 60s interval fires once under fast_forward and 60 times under run_for. clock_pause_at stops time at an instant, clock_resume restarts it, clock_set_fixed_time freezes Date.now while leaving timers running, and clock_set_system_time shifts time silently without firing timers. `ticks` takes milliseconds or \"MM:SS\"/\"HH:MM:SS\"; the clock is session-scoped across tabs and navigations until reset clears it.\n",
    "reset is the escape hatch for sticky plumbing: one call drops every network route, HAR replay, WebSocket route, locator handler, and virtual authenticator, turns offline off, and clears media, viewport, device, geolocation, and permission overrides, reporting what it cleared with counts. It leaves cookies, storage, open tabs, and the current page untouched. ",
    "cdp_send is the raw Chrome DevTools Protocol escape hatch for the long tail that has no dedicated step: send `method` plus optional `params`, with `target` \"page\" (default, the active tab) or \"browser\". Prefer a dedicated step whenever one exists, because those carry actionability, redaction, and reset bookkeeping that raw CDP does not. State set through cdp_send is invisible to list_routes and reset, so undo it yourself; Emulation and Network mutations come back with a warning saying exactly that. Browser.close is refused, and so is Target.closeTarget aimed at the tab this session drives. Results return inline as JSON under 8KB and as an artifact path beyond it, cookie and storage values are redacted, and CDP errors surface verbatim on one bounded line.\n",
    "http_request sends an HTTP call that shares the page's cookie jar in both directions: matching cookies for the target domain and path are attached, and response Set-Cookie headers are written back into the browser, so a logged-in page and the API call see the same session. Send url plus optional method, headers, and exactly one of body, body_json (auto application/json), or form (auto urlencoded); http and https only. Results carry status, final URL after redirects, content-type/content-length (set full_headers=true for every header), and the body inline when it stays under 8KB, otherwise an artifact path. Cookie values are never inlined, only the count and names. Set fail_on_status=true to fail the step on a non-2xx status.\n",
    "Instrumentation: start_coverage and stop_coverage opt into precise JavaScript and CSS usage tracking and return bounded per-URL summaries plus a full JSON artifact. add_virtual_authenticator enables passkey testing and mints the authenticator id it returns, so never send it an id; remove_virtual_authenticator, list_credentials, add_credential, clear_credentials, and set_user_verified address that returned id. Credential ids, private keys, user handles, blobs, and user names are redacted from reports.\n",
    "Motion: capture_frames records a burst and returns ONE composed filmstrip image (up to a 4x6 grid, each cell labelled +NNNms) plus per-frame artifact paths and the percentage of pixels that changed against the previous frame, so animations and transient UI are readable even without looking at pixels. It takes duration_ms (defaults to 1000, capped at 10000) with either frame_count (2-24, defaults to 8) or interval_ms, and scopes to an element with locator or to the whole document with full_page. Out-of-range values are hard errors. screencast_start and screencast_stop bracket a manual session that auto-stops at 30000ms or 60 frames and reports that cap as a warning; screencast_stop composes a filmstrip unless compose=false. The filmstrip is always attached, even when attach_screenshot is false.\n",
    "Touch and low-level keyboard: tap takes either a locator (full actionability and hit-target checks, like click) or x/y coordinates, and requires touch emulation from an earlier set_viewport step with has_touch true. insert_text types into the focused element with one input event and no key events, which suits IME-style entry but skips keyboard shortcuts; it focuses an optional locator first. press_sequentially focuses its locator and then sends real per-character key events with an optional delay_ms (default 0) for inputs driven by keystroke handlers such as autocomplete; prefer fill for ordinary form entry.\n",
    "Forms: fill, clear, select_option, check, uncheck.\n",
    "Assertions: expect retries with a 5000ms default and supports state, text/value, attribute/class/CSS/id/property, role/accessibility, count, URL/title, and ARIA snapshot matchers. Assertion failures report expected and last received values; set soft=true to record a failure and continue the batch. ",
    "expect_poll evaluates `expression` and retries until the value satisfies `matcher` (equals, contains, gt, lt, matches_regex) against `expected`, reporting attempts and elapsed like expect; it also honours soft.\n",
    "Waiting: wait_for_function is the way to wait on arbitrary app state: it evaluates `expression` until the result is truthy, defaults to 100/250/500/1000ms poll intervals unless `polling_ms` fixes one, and with a `locator` re-resolves the element each retry and passes it as the first argument, so a re-rendered node is tolerated. A thrown expression fails immediately instead of retrying. ",
    "wait_for_popup, wait_for_selector, wait_for_navigation, wait_for_url, wait_for_text, wait_for_network_idle, wait_for_load_state, wait_for_element_hidden, wait_for_element_stable. Put wait_for_popup immediately before the popup-producing click in ONE batch; the returned popup becomes active for later steps. wait_for_url takes a plain substring in `pattern` and matches when the current URL contains it, unlike the glob/regex `pattern` used by route and wait_for_request. ",
    "Click, hover, fill, clear, check, and uncheck auto-wait for actionability. Never use `wait_seconds` for readiness; use `wait_for_response`, `wait_for_load_state`, or `wait_for_selector` for genuine synchronization.\n",
    "Inspection: get_text, get_html, get_attribute, extract_links, extract_table, dom_snapshot, accessibility_snapshot, screenshot, screenshot_element, screenshot_elements, capture_element_states, pdf, styles, tab_log. Screenshots support full_page, clip, type, quality, scale, omit_background, animations, caret, mask, mask_color, and style; screenshot_element uses locator or ref. screenshot_elements takes locators plus compose (grid composes one labeled contact sheet, separate returns one image per locator). capture_element_states captures one locator across states (default, hover, focus, active) as a labeled strip. PDF supports Chromium print options and returns an artifact path.\n",
    "Readouts (never fake these with eval or expect): bounding_box returns viewport CSS-pixel x/y/width/height or null when the element is not visible; count returns the match count without strictness; input_value returns the live value property of an input, textarea, or select and fails on any other element; all_texts returns the text of every match with `mode` inner_text or text_content plus an optional `limit`, reporting the true total; element_state returns visible, enabled, editable, checked, and stable in one read.\n",
    "Network: wait_for_request and wait_for_response accept a URL string or `{source,flags}` regex; completed requests also appear in the report. route registers a persistent `{pattern,handler}` with fulfill, abort, or continue modifications; unroute removes one pattern or all routes; list_routes returns active routes. Text route bodies are UTF-8 and encoded to base64 on the CDP wire; set body_base64=true when body already contains base64 binary data. Page-level routes may not observe requests served by a service worker.\n",
    "Window vs viewport: set_viewport is device-metrics emulation (it changes what the page measures, not the window on screen); set_window_bounds moves and resizes the actual OS window with x/y/width/height, any subset. set_window_bounds needs a headed browser: in headless there is no OS window, so it succeeds without applying and tells you to use set_viewport. reset does not touch window bounds.\n",
    "Network: wait_for_request and wait_for_response accept a URL string or `{source,flags}` regex; completed requests also appear in the report. route registers a persistent `{pattern,handler}` with fulfill, abort, continue, fallback, or fetch_and_fulfill; unroute removes one pattern or all routes; list_routes returns active routes in evaluation order with `order` and `times_remaining`. Several routes may share a pattern: the newest matching route runs first, a fallback handler hands the request to the next older matching route, then to the HAR replay, then to the network. Optional `times` on a route expires it after that many matches, including matches consumed by a traversed fallback. fulfill takes `body`, or `path` to serve a file (relative paths stay inside the runtime artifact directory, content type inferred from the extension), or `json` for a JSON body; status defaults to 200. fetch_and_fulfill performs the real request from the engine (up to 20 redirects, forwarding the page's own request headers) and fulfills with the real response, optionally overriding status, response_headers, and body. Cookie, Host, and Content-Length request headers keep their original values on continue and fetch_and_fulfill. Text route bodies are UTF-8 and encoded to base64 on the CDP wire; set body_base64=true when body already contains base64 binary data. URL patterns are globs (`*`, `**`, `{a,b}`) or `{source,flags}` regexes; `?` is literal and JavaScript route predicates are not supported. Page-level routes may not observe requests served by a service worker.\n",
    "Devices and throttling: emulate_device applies one named Playwright device (viewport, DPR, mobile, touch, and user agent together) — list_devices returns the 200+ names with an optional filter, and mobile, tablet, and desktop stay as aliases accepted by both emulate_device and open_tab. An unknown name is a hard error listing the closest matches. set_network_conditions takes latency_ms, download_kbps, upload_kbps, an optional offline flag, and an optional preset of slow-3g, fast-3g, or slow-4g using Chrome DevTools values; explicit parameters override the preset and omitted bandwidth stays unlimited. set_cpu_throttling takes rate, a slowdown multiplier where 1 is off. reset clears both.\n",
    "Context: set_viewport, emulate_media, set_locale, set_timezone, set_user_agent, set_geolocation, set_offline, and set_extra_http_headers persist across adopted tabs and popups. Cookie state uses get_cookies, set_cookies, clear_cookies. Web storage uses get_storage, set_storage, clear_storage with kind local or session. storage_state and set_storage_state use Playwright's {cookies,origins:[{origin,local_storage}]} login-reuse shape. grant_permissions and clear_permissions control origin permissions. set_http_credentials shares the lazy Fetch path with routing. Cookie, storage, and credential values are redacted in reports.\n",
    "Files: set_input_files, expect_file_chooser, wait_for_download.\n",
    "Dialogs: handle_dialog arms the next dialog with `accept` and optional `prompt_text`; unarmed dialogs auto-dismiss except beforeunload, which is accepted.\n",
    "Advanced: eval, add_locator_handler, remove_locator_handler, dismiss_overlays, highlight_element, highlight, hide_highlight, annotate, and fixed-delay wait_seconds. highlight accepts locator/ref plus optional style and label; annotate accepts locator/ref plus text. Locator handlers use `{type:\"click\"}` or `{type:\"steps\",steps:[...]}`.\n",
    "Locator fallback vocabulary: ref; role with name/description, exact or regex, and checked/pressed/selected/expanded/disabled/level/include_hidden filters; test_id with configurable `attribute`; text, label, placeholder, alt_text, title, css, xpath, id, name, and autocomplete. ",
    "Compose with zero-based `nth` (-1 is last), first/last, locator, filter (has/has_not/has_text/has_not_text/visible), and/or, or an outermost-first `frames` chain. Non-selecting actions are strict: ambiguous locators fail loudly with the match count. Same-process frames are supported; out-of-process frames fail explicitly.\n",
    "`page_context` picks the page-changed context: `snapshot` (the default) attaches the ref-annotated ARIA snapshot and NO image, `screenshot` attaches a policy-sized image instead, `both` attaches each, `none` attaches only the page header. The snapshot is attached only when the batch actually changed the page. ",
    "`attach_screenshot` remains the tri-state screenshot override and wins over `page_context`: true = always attach, false = never attach, omitted = follow `page_context`. An explicit `screenshot` step still returns its own image even when false, and still adds the report screenshot under the default `snapshot` mode.\n",
    "`network` controls per-request report volume: `summary` (the default) emits one `method url status bytes ms` line per request, `full` keeps request and response headers, `none` drops per-request entries. Route interception telemetry and the detail returned by wait_for_request and wait_for_response stay visible in every mode. ",
    "The legacy newline-separated `commands` input remains accepted but is deprecated; new callers must use `request.steps`."
);

fn locator_regex_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["source"],
        "properties": {
            "source": {"type": "string"},
            "flags": {"type": "string"}
        }
    })
}

fn browser_locator_schema() -> serde_json::Value {
    let mut schema = serde_json::json!({
        "type": "object",
        "description": "Ref-first element address or composable fallback locator. Ambiguous strict actions fail with the match count.",
        "required": ["by"],
        "properties": {
            "by": {"type": "string", "enum": ["ref", "css", "id", "name", "text", "label", "role", "xpath", "placeholder", "alt_text", "title", "autocomplete", "test_id"]},
            "value": {"type": "string", "description": "Snapshot ref such as e12 or f2e7, or selector value for non-role strategies"},
            "frames": {"type": "array", "items": {"type": "object"}, "description": "Outermost-first iframe-owner locator chain. Each owner must resolve to exactly one iframe or frame element."},
            "nth": {"type": "integer", "description": "Zero-based match index; -1 selects the last match. CSS :nth-match is one-based."},
            "within": {"type": "string", "description": "Deprecated CSS scope kept for compatibility; use locator for chaining"},
            "locator": {"type": "object", "description": "Nested BrowserLocator evaluated under each outer match"},
            "filter": {
                "type": "object",
                "properties": {
                    "has": {"type": "object", "description": "Relative BrowserLocator required under the candidate"},
                    "has_not": {"type": "object", "description": "Relative BrowserLocator forbidden under the candidate"},
                    "has_text": {"oneOf": [{"type": "string"}, {"type": "object", "required": ["source"], "properties": {"source": {"type": "string"}, "flags": {"type": "string"}}}]},
                    "has_not_text": {"oneOf": [{"type": "string"}, {"type": "object", "required": ["source"], "properties": {"source": {"type": "string"}, "flags": {"type": "string"}}}]},
                    "visible": {"type": "boolean"}
                }
            },
            "and": {"type": "object", "description": "BrowserLocator whose matches intersect this locator"},
            "or": {"type": "object", "description": "BrowserLocator whose matches union with this locator in DOM order"},
            "first": {"type": "boolean", "description": "Select the first match"},
            "last": {"type": "boolean", "description": "Select the last match"},
            "exact": {"type": "boolean", "description": "Case-sensitive whole-string match; regex ignores it"},
            "attribute": {"type": "string", "description": "Test-id attribute; defaults to data-testid"},
            "role": {"type": "string", "description": "ARIA role"},
            "name": {"type": "string", "description": "Accessible name"},
            "description": {"type": "string", "description": "Accessible description"},
            "checked": {"oneOf": [{"type": "boolean"}, {"type": "string", "enum": ["mixed"]}]},
            "pressed": {"oneOf": [{"type": "boolean"}, {"type": "string", "enum": ["mixed"]}]},
            "selected": {"type": "boolean"},
            "expanded": {"type": "boolean"},
            "disabled": {"type": "boolean"},
            "level": {"type": "integer"},
            "include_hidden": {"type": "boolean"}
        }
    });
    let properties = schema["properties"]
        .as_object_mut()
        .expect("locator properties are present");
    properties.insert("regex".to_string(), locator_regex_schema());
    properties.insert("name_regex".to_string(), locator_regex_schema());
    properties.insert("description_regex".to_string(), locator_regex_schema());
    schema
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

fn url_pattern_schema() -> serde_json::Value {
    serde_json::json!({
        "oneOf": [
            {"type": "string"},
            {
                "type": "object",
                "required": ["source"],
                "properties": {
                    "source": {"type": "string"},
                    "flags": {"type": "string"}
                }
            }
        ]
    })
}

fn browser_step_schema_with_actions(
    actions: &[&str],
    include_locator_handler: bool,
) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "action".to_string(),
        serde_json::json!({
            "type": "string",
            "description": "Browser action in snake_case",
            "enum": actions
        }),
    );
    properties.insert("url".to_string(), serde_json::json!({"type": "string"}));
    properties.insert(
        "device".to_string(),
        serde_json::json!({"type": "string", "enum": ["desktop", "mobile", "tablet"]}),
    );
    properties.insert("tab".to_string(), tab_target_schema());
    properties.insert("locator".to_string(), browser_locator_schema());
    properties.insert("source".to_string(), browser_locator_schema());
    properties.insert(
        "target".to_string(),
        serde_json::json!({
            "oneOf": [
                browser_locator_schema(),
                {"type": "string", "enum": ["page", "browser"], "description": "cdp_send target, page by default"}
            ]
        }),
    );
    let position_schema = serde_json::json!({
        "type": "object",
        "required": ["x", "y"],
        "properties": {"x": {"type": "number"}, "y": {"type": "number"}}
    });
    properties.insert("source_position".to_string(), position_schema.clone());
    properties.insert("target_position".to_string(), position_schema);
    properties.insert("text".to_string(), serde_json::json!({"type": "string"}));
    properties.insert("key".to_string(), serde_json::json!({"type": "string"}));
    properties.insert(
        "modifiers".to_string(),
        serde_json::json!({"type": "array", "items": {"type": "string", "enum": ["Alt", "Ctrl", "Meta", "Shift"]}}),
    );
    properties.insert(
        "expression".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert(
        "selector".to_string(),
        serde_json::json!({"type": "string", "description": "CSS selector for dom_snapshot"}),
    );
    properties.insert("value".to_string(), serde_json::json!({"type": "string"}));
    properties.insert(
        "attribute".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert("seconds".to_string(), serde_json::json!({"type": "number"}));
    properties.insert("x".to_string(), serde_json::json!({"type": "number"}));
    properties.insert("y".to_string(), serde_json::json!({"type": "number"}));
    properties.insert("start_x".to_string(), serde_json::json!({"type": "number"}));
    properties.insert("start_y".to_string(), serde_json::json!({"type": "number"}));
    properties.insert("end_x".to_string(), serde_json::json!({"type": "number"}));
    properties.insert("end_y".to_string(), serde_json::json!({"type": "number"}));
    properties.insert("delta_x".to_string(), serde_json::json!({"type": "number"}));
    properties.insert("delta_y".to_string(), serde_json::json!({"type": "number"}));
    properties.insert(
        "steps".to_string(),
        serde_json::json!({"type": "integer", "minimum": 1}),
    );
    properties.insert(
        "button".to_string(),
        serde_json::json!({"type": "string", "enum": ["left", "middle", "right"]}),
    );
    properties.insert(
        "click_count".to_string(),
        serde_json::json!({"type": "integer", "minimum": 1}),
    );
    properties.insert(
        "delay".to_string(),
        serde_json::json!({"type": "integer", "minimum": 0}),
    );
    properties.insert(
        "delay_ms".to_string(),
        serde_json::json!({"type": "integer", "minimum": 0}),
    );
    properties.insert(
        "timeout_ms".to_string(),
        serde_json::json!({"type": "integer", "minimum": 0}),
    );
    properties.insert(
        "ticks".to_string(),
        serde_json::json!({
            "oneOf": [
                {"type": "integer", "description": "Milliseconds to advance the clock by"},
                {"type": "string", "description": "Human-readable duration: \"08\", \"01:00\" or \"02:34:10\""}
            ]
        }),
    );
    properties.insert(
        "time".to_string(),
        serde_json::json!({
            "oneOf": [
                {"type": "integer", "description": "Unix time in milliseconds"},
                {"type": "string", "description": "ISO date or date-time, for example \"2020-02-02\" or \"2020-02-02T10:00:00Z\""}
            ]
        }),
    );
    properties.insert(
        "matcher".to_string(),
        serde_json::json!({
            "description": "Matcher for expect (object) or expect_poll (comparator name).",
            "oneOf": [
                {
                    "type": "object",
                    "description": "Expectation matcher. Use type plus expected/name/ignore_case as required. Text expectations accept a string or {source,flags} regex.",
                    "required": ["type"],
                    "properties": {
                        "type": {
                            "type": "string",
                            "enum": [
                                "to_be_attached", "to_be_visible", "to_be_hidden", "to_be_enabled",
                                "to_be_disabled", "to_be_editable", "to_be_checked", "to_be_focused",
                                "to_be_empty", "to_be_in_viewport", "to_have_text", "to_contain_text",
                                "to_have_value", "to_have_values", "to_have_attribute", "to_have_class",
                                "to_contain_class", "to_have_count", "to_have_css", "to_have_id",
                                "to_have_js_property", "to_have_role", "to_have_accessible_name",
                                "to_have_accessible_description", "to_have_url", "to_have_title",
                                "to_match_aria_snapshot"
                            ]
                        },
                        "expected": {},
                        "name": {"type": "string"},
                        "ignore_case": {"type": "boolean"}
                    }
                },
                {
                    "type": "string",
                    "description": "expect_poll comparator applied to the evaluated value against `expected`.",
                    "enum": ["equals", "contains", "gt", "lt", "matches_regex"]
                }
            ]
        }),
    );
    properties.insert(
        "expected".to_string(),
        serde_json::json!({
            "description": "expect_poll expectation compared against the evaluated value. matches_regex takes a regex string or {source,flags}."
        }),
    );
    properties.insert(
        "polling_ms".to_string(),
        serde_json::json!({
            "type": "integer",
            "minimum": 0,
            "description": "wait_for_function fixed poll interval; defaults to 100/250/500/1000ms repeating the last"
        }),
    );
    properties.insert(
        "soft".to_string(),
        serde_json::json!({"type": "boolean", "description": "Record assertion failure and continue the batch"}),
    );
    properties.insert(
        "limit".to_string(),
        serde_json::json!({"type": "integer", "minimum": 0}),
    );
    properties.insert(
        "clear_first".to_string(),
        serde_json::json!({"type": "boolean", "description": "Defaults to true"}),
    );
    properties.insert(
        "verify".to_string(),
        serde_json::json!({"type": "boolean", "description": "Defaults to true for fill and clear"}),
    );
    properties.insert("js".to_string(), serde_json::json!({"type": "boolean"}));
    properties.insert("css".to_string(), serde_json::json!({"type": "boolean"}));
    properties.insert(
        "reset_on_navigation".to_string(),
        serde_json::json!({"type": "boolean", "description": "Reset coverage on navigation; defaults to true"}),
    );
    properties.insert(
        "duration_ms".to_string(),
        serde_json::json!({"type": "integer", "minimum": 1, "maximum": MAX_BURST_DURATION_MS, "description": "capture_frames burst length; defaults to 1000ms"}),
    );
    properties.insert(
        "frame_count".to_string(),
        serde_json::json!({"type": "integer", "minimum": MIN_FRAME_COUNT, "maximum": MAX_FRAME_COUNT, "description": "capture_frames frame count; defaults to 8, mutually exclusive with interval_ms"}),
    );
    properties.insert(
        "interval_ms".to_string(),
        serde_json::json!({"type": "integer", "minimum": 1, "description": "capture_frames spacing; mutually exclusive with frame_count"}),
    );
    properties.insert(
        "compose".to_string(),
        serde_json::json!({"type": "boolean", "description": "screencast_stop composes a filmstrip; defaults to true"}),
    );
    properties.insert(
        "max_width".to_string(),
        serde_json::json!({"type": "integer", "minimum": 1, "description": "screencast_start frame width cap"}),
    );
    properties.insert(
        "max_height".to_string(),
        serde_json::json!({"type": "integer", "minimum": 1, "description": "screencast_start frame height cap"}),
    );
    properties.insert(
        "protocol".to_string(),
        serde_json::json!({"type": "string", "enum": ["u2f", "ctap2"]}),
    );
    properties.insert(
        "transport".to_string(),
        serde_json::json!({"type": "string", "enum": ["usb", "nfc", "ble", "cable", "internal"]}),
    );
    properties.insert(
        "has_resident_key".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert(
        "has_user_verification".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert(
        "is_user_verified".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert(
        "id".to_string(),
        serde_json::json!({
            "type": "string",
            "description": "Virtual authenticator id minted by add_virtual_authenticator and returned in its result. Required by remove_virtual_authenticator, list_credentials, add_credential, clear_credentials, and set_user_verified; sending it to add_virtual_authenticator is an error."
        }),
    );
    properties.insert(
        "credential".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["credential_id", "private_key"],
            "properties": {
                "credential_id": {"type": "string"},
                "is_resident_credential": {"type": "boolean"},
                "rp_id": {"type": "string"},
                "private_key": {"type": "string"},
                "user_handle": {"type": "string"},
                "sign_count": {"type": "integer", "minimum": 0},
                "large_blob": {"type": "string"},
                "backup_eligibility": {"type": "boolean"},
                "backup_state": {"type": "boolean"},
                "user_name": {"type": "string"},
                "user_display_name": {"type": "string"}
            }
        }),
    );
    properties.insert(
        "max_chars".to_string(),
        serde_json::json!({"type": "integer", "minimum": 0}),
    );
    properties.insert(
        "property_filter".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert(
        "mode".to_string(),
        serde_json::json!({"type": "string", "enum": ["ai", "default", "inner_text", "text_content"], "description": "Accessibility snapshot mode ai or default, defaulting to ai; all_texts mode inner_text or text_content, defaulting to inner_text"}),
    );
    properties.insert(
        "refs".to_string(),
        serde_json::json!({"type": "boolean", "description": "Mint refs in accessibility_snapshot; defaults to true in ai mode"}),
    );
    properties.insert(
        "boxes".to_string(),
        serde_json::json!({"type": "boolean", "description": "Include element boxes in accessibility_snapshot"}),
    );
    properties.insert(
        "depth".to_string(),
        serde_json::json!({"type": "integer", "minimum": 1, "description": "Limit accessibility_snapshot nesting; deeper children collapse into a truncated-count marker"}),
    );
    properties.insert(
        "paths".to_string(),
        serde_json::json!({"type": "array", "items": {"type": "string"}}),
    );
    properties.insert(
        "state".to_string(),
        serde_json::json!({"type": "string", "enum": ["domcontentloaded", "load", "networkidle"]}),
    );
    properties.insert("pattern".to_string(), url_pattern_schema());
    properties.insert("save_as".to_string(), serde_json::json!({"type": "string"}));
    properties.insert(
        "full_page".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert(
        "clip".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["x", "y", "width", "height"],
            "properties": {
                "x": {"type": "number"},
                "y": {"type": "number"},
                "width": {"type": "number", "exclusiveMinimum": 0},
                "height": {"type": "number", "exclusiveMinimum": 0}
            }
        }),
    );
    properties.insert(
        "type".to_string(),
        serde_json::json!({"type": "string", "enum": ["png", "jpeg", "webp"]}),
    );
    properties.insert(
        "quality".to_string(),
        serde_json::json!({"type": "integer", "minimum": 0, "maximum": 100}),
    );
    properties.insert(
        "scale".to_string(),
        serde_json::json!({"description": "Screenshot scale css|device, or numeric PDF scale 0.1-2", "anyOf": [{"type": "string", "enum": ["css", "device"]}, {"type": "number", "minimum": 0.1, "maximum": 2.0}]}),
    );
    properties.insert(
        "omit_background".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert(
        "animations".to_string(),
        serde_json::json!({"type": "string", "enum": ["allow", "disabled"]}),
    );
    properties.insert(
        "caret".to_string(),
        serde_json::json!({"type": "string", "enum": ["hide", "initial"]}),
    );
    properties.insert(
        "mask".to_string(),
        serde_json::json!({"type": "array", "items": browser_locator_schema()}),
    );
    properties.insert(
        "mask_color".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert("style".to_string(), serde_json::json!({"type": "string"}));
    properties.insert(
        "locators".to_string(),
        serde_json::json!({"type": "array", "items": browser_locator_schema()}),
    );
    properties.insert(
        "compose".to_string(),
        serde_json::json!({"type": "string", "enum": ["grid", "separate"], "description": "grid composes one labeled contact sheet, separate returns one image per locator"}),
    );
    properties.insert(
        "states".to_string(),
        serde_json::json!({"type": "array", "items": {"type": "string", "enum": ["default", "hover", "focus", "active"]}}),
    );
    properties.insert("labels".to_string(), serde_json::json!({"type": "boolean"}));
    properties.insert("label".to_string(), serde_json::json!({"type": "string"}));
    properties.insert(
        "landscape".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert(
        "print_background".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert(
        "format".to_string(),
        serde_json::json!({"type": "string", "enum": ["Letter", "Legal", "Tabloid", "Ledger", "A0", "A1", "A2", "A3", "A4", "A5", "A6"]}),
    );
    properties.insert("width".to_string(), serde_json::json!({"type": "string"}));
    properties.insert("height".to_string(), serde_json::json!({"type": "string"}));
    properties.insert(
        "margins".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": {
                "top": {"type": "string"},
                "right": {"type": "string"},
                "bottom": {"type": "string"},
                "left": {"type": "string"}
            }
        }),
    );
    properties.insert(
        "page_ranges".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert(
        "prefer_css_page_size".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert("tagged".to_string(), serde_json::json!({"type": "boolean"}));
    properties.insert(
        "outline".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert(
        "width".to_string(),
        serde_json::json!({"type": "integer", "minimum": 1}),
    );
    properties.insert(
        "height".to_string(),
        serde_json::json!({"type": "integer", "minimum": 1}),
    );
    properties.insert(
        "device_scale_factor".to_string(),
        serde_json::json!({"type": "number", "minimum": 0}),
    );
    properties.insert(
        "is_mobile".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert(
        "has_touch".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert(
        "color_scheme".to_string(),
        serde_json::json!({"type": "string", "enum": ["light", "dark", "no-preference"]}),
    );
    properties.insert(
        "reduced_motion".to_string(),
        serde_json::json!({"type": "string", "enum": ["reduce", "no-preference"]}),
    );
    properties.insert(
        "forced_colors".to_string(),
        serde_json::json!({"type": "string", "enum": ["active", "none"]}),
    );
    properties.insert(
        "contrast".to_string(),
        serde_json::json!({"type": "string", "enum": ["more", "less", "custom", "no-preference"]}),
    );
    properties.insert("media".to_string(), serde_json::json!({"type": "string"}));
    properties.insert("locale".to_string(), serde_json::json!({"type": "string"}));
    properties.insert(
        "timezone".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert(
        "user_agent".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert(
        "accept_language".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert(
        "latitude".to_string(),
        serde_json::json!({"type": "number"}),
    );
    properties.insert(
        "longitude".to_string(),
        serde_json::json!({"type": "number"}),
    );
    properties.insert(
        "accuracy".to_string(),
        serde_json::json!({"type": "number", "minimum": 0}),
    );
    properties.insert(
        "offline".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert(
        "headers".to_string(),
        serde_json::json!({"type": "object", "additionalProperties": {"type": "string"}}),
    );
    properties.insert(
        "method".to_string(),
        serde_json::json!({"type": "string", "description": "HTTP method for http_request, GET by default; CDP method such as Runtime.evaluate for cdp_send"}),
    );
    properties.insert(
        "params".to_string(),
        serde_json::json!({"type": "object", "description": "cdp_send CDP parameters, omitted when the method takes none"}),
    );
    properties.insert(
        "body".to_string(),
        serde_json::json!({"type": "string", "description": "Raw http_request body; mutually exclusive with body_json and form"}),
    );
    properties.insert(
        "body_json".to_string(),
        serde_json::json!({"description": "http_request JSON body, sent as application/json"}),
    );
    properties.insert(
        "form".to_string(),
        serde_json::json!({"type": "object", "additionalProperties": {"type": "string"}, "description": "http_request form fields, sent as application/x-www-form-urlencoded"}),
    );
    properties.insert(
        "max_redirects".to_string(),
        serde_json::json!({"type": "integer", "minimum": 0, "maximum": 20}),
    );
    properties.insert(
        "fail_on_status".to_string(),
        serde_json::json!({"type": "boolean", "description": "Fail the http_request step when the status is not 2xx"}),
    );
    properties.insert(
        "full_headers".to_string(),
        serde_json::json!({"type": "boolean", "description": "Return every http_request response header instead of content-type and content-length only"}),
    );
    properties.insert(
        "urls".to_string(),
        serde_json::json!({"type": "array", "items": {"type": "string"}}),
    );
    properties.insert(
        "cookies".to_string(),
        serde_json::json!({"type": "array", "items": {"type": "object"}}),
    );
    properties.insert("domain".to_string(), serde_json::json!({"type": "string"}));
    properties.insert("path".to_string(), serde_json::json!({"type": "string"}));
    properties.insert(
        "kind".to_string(),
        serde_json::json!({"type": "string", "enum": ["local", "session"]}),
    );
    properties.insert("origin".to_string(), serde_json::json!({"type": "string"}));
    properties.insert("items".to_string(), serde_json::json!({"type": "array", "items": {"type": "object", "required": ["name", "value"]}}));
    properties.insert(
        "permissions".to_string(),
        serde_json::json!({"type": "array", "items": {"type": "string"}}),
    );
    properties.insert(
        "username".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert(
        "password".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert("name".to_string(), serde_json::json!({"type": "string"}));
    properties.insert(
        "times".to_string(),
        serde_json::json!({"type": "integer", "minimum": 1}),
    );
    properties.insert(
        "no_wait_after".to_string(),
        serde_json::json!({"type": "boolean"}),
    );
    properties.insert("accept".to_string(), serde_json::json!({"type": "boolean"}));
    properties.insert(
        "prompt_text".to_string(),
        serde_json::json!({"type": "string"}),
    );
    if include_locator_handler {
        properties.insert("handler".to_string(), handler_schema());
    }
    serde_json::json!({
        "type": "object",
        "required": ["action"],
        "properties": properties
    })
}

fn route_handler_schema() -> serde_json::Value {
    serde_json::json!({
        "oneOf": [
            {
                "type": "object",
                "required": ["type"],
                "properties": {
                    "type": {"type": "string", "enum": ["fulfill"]},
                    "status": {"type": "integer", "minimum": 100, "maximum": 599, "description": "Defaults to 200"},
                    "headers": {"type": "object", "additionalProperties": {"type": "string"}},
                    "content_type": {"type": "string"},
                    "body": {"type": "string", "description": "UTF-8 text by default; set body_base64=true when this string contains base64-encoded binary response bytes"},
                    "path": {"type": "string", "description": "Serve this file as the response body; relative paths resolve inside the runtime artifact directory and may not escape it. Content type is inferred from the extension unless content_type is set"},
                    "json": {"description": "Serialize this value as the response body with content type application/json"},
                    "body_base64": {"type": "boolean", "description": "Treat body as already-base64-encoded binary bytes instead of UTF-8 text"}
                }
            },
            {
                "type": "object",
                "required": ["type", "reason"],
                "properties": {
                    "type": {"type": "string", "enum": ["abort"]},
                    "reason": {"type": "string", "enum": ["failed", "aborted", "timedout", "accessdenied", "connectionclosed", "connectionreset", "connectionrefused", "connectionaborted", "connectionfailed", "namenotresolved", "internetdisconnected", "addressunreachable", "blockedbyclient", "blockedbyresponse"]}
                }
            },
            {
                "type": "object",
                "required": ["type"],
                "properties": {
                    "type": {"type": "string", "enum": ["continue"]},
                    "url": {"type": "string"},
                    "method": {"type": "string"},
                    "headers": {"type": "object", "additionalProperties": {"type": "string"}},
                    "post_data": {"type": "string", "description": "UTF-8 request body replacement"}
                }
            },
            {
                "type": "object",
                "required": ["type"],
                "properties": {
                    "type": {"type": "string", "enum": ["fallback"], "description": "Hand the request to the next older route registered for a matching pattern, then the HAR replay, then the network"}
                }
            },
            {
                "type": "object",
                "required": ["type"],
                "properties": {
                    "type": {"type": "string", "enum": ["fetch_and_fulfill"], "description": "Perform the real request from the engine, then fulfill the page with the response"},
                    "url": {"type": "string", "description": "Request URL override; defaults to the intercepted URL"},
                    "method": {"type": "string", "description": "Request method override"},
                    "headers": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Request header overrides merged over the intercepted headers"},
                    "post_data": {"type": "string", "description": "UTF-8 request body replacement"},
                    "status": {"type": "integer", "minimum": 100, "maximum": 599, "description": "Response status override; defaults to the real status"},
                    "response_headers": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Response header overrides merged over the real response headers"},
                    "body": {"type": "string", "description": "Response body replacement; defaults to the real response body"},
                    "body_base64": {"type": "boolean", "description": "Treat body as already-base64-encoded binary bytes instead of UTF-8 text"}
                }
            }
        ]
    })
}

fn handler_schema() -> serde_json::Value {
    serde_json::json!({
        "oneOf": [locator_handler_schema(), route_handler_schema()]
    })
}

fn locator_handler_schema() -> serde_json::Value {
    let actions = LOCATOR_HANDLER_STEP_ACTIONS
        .iter()
        .copied()
        .filter(|action| {
            !matches!(
                *action,
                "route" | "unroute" | "list_routes" | "reset" | "http_request" | "cdp_send"
            ) && !action.starts_with("clock_")
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "oneOf": [
            {
                "type": "object",
                "required": ["type"],
                "properties": {"type": {"type": "string", "enum": ["click"]}}
            },
            {
                "type": "object",
                "required": ["type", "steps"],
                "properties": {
                    "type": {"type": "string", "enum": ["steps"]},
                    "steps": {
                        "type": "array",
                        "items": browser_step_schema_with_actions(&actions, false)
                    }
                }
            }
        ]
    })
}

fn browser_request_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "description": "Typed batched browser request",
        "required": ["steps"],
        "properties": {
            "session": {"type": "string", "enum": ["shared_default"]},
            "page_context": {"type": "string", "enum": ["snapshot", "screenshot", "both", "none"], "description": "What to attach as page-changed context. snapshot (default) attaches the ref-annotated aria snapshot and no image, screenshot attaches a policy-sized PNG instead, both attaches each, none attaches only the page header"},
            "attach_screenshot": {"type": "boolean", "description": "Screenshot override: true = always attach, false = never attach, omitted = follow page_context"},
            "network": {"type": "string", "enum": ["none", "summary", "full"], "description": "Per-request report volume. summary (default) emits one `method url status bytes ms` line per request, full keeps request and response headers, none drops per-request entries"},
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
            "request": browser_request_schema(),
            "commands": {
                "type": "string",
                "description": "Deprecated compatibility-only newline-separated browser commands; use request.steps",
                "deprecated": true
            }
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
                    typed_content = Some(execution_report_to_multimodal(&report, &image_policy)?);
                    let (execute_log, command_multimodal_els) =
                        format_controller_report(&report, "", &image_policy);
                    tool_log.extend(execute_log);
                    multimodal_els.extend(command_multimodal_els);
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
                    &image_policy,
                )
                .await
                {
                    Ok((execute_log, command_multimodal_els)) => {
                        tool_log.extend(execute_log);
                        multimodal_els.extend(command_multimodal_els);
                    }
                    Err(e) => {
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
        assert_eq!(
            schema.pointer("/properties/commands/deprecated"),
            Some(&Value::Bool(true))
        );
        let url_pattern_required = schema
            .pointer(
                "/properties/request/properties/steps/items/properties/pattern/oneOf/1/required",
            )
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(url_pattern_required, &[Value::String("source".to_string())]);
        for legacy_field in ["url_or_pattern", "contains"] {
            assert!(
                !properties.contains_key(legacy_field),
                "legacy field {legacy_field} must not appear in the schema"
            );
        }
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
    fn chrome_description_documents_arbitrary_state_waiting() {
        let description = ToolChrome::default().tool_description().description;
        assert!(description.contains("wait_for_function"));
        assert!(description.contains("expect_poll"));
        assert!(description.contains("polling_ms"));
        assert!(description.contains("truthy"));
        assert!(description.contains("re-resolves the element each retry"));
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
    fn chrome_description_groups_capabilities_and_documents_locators() {
        let description = ToolChrome::default().tool_description().description;
        for heading in [
            "Core:",
            "Forms:",
            "Waiting:",
            "Inspection:",
            "Network:",
            "Files:",
            "Dialogs:",
            "Advanced:",
        ] {
            assert!(description.contains(heading), "missing heading {heading}");
        }
        for locator_term in [
            "ref;",
            "role with",
            "test_id",
            "text, label, placeholder, alt_text, title, css, xpath",
            "zero-based `nth`",
            "filter (has/has_not/has_text/has_not_text/visible)",
            "outermost-first `frames` chain",
            "ambiguous locators fail loudly with the match count",
        ] {
            assert!(
                description.contains(locator_term),
                "missing locator documentation {locator_term}"
            );
        }
        assert!(description.contains("legacy newline-separated `commands` input"));
        assert!(description.contains("deprecated"));
    }

    #[test]
    fn chrome_description_documents_the_coordinate_handler_bypass_caveat() {
        let description = ToolChrome::default().tool_description().description;
        assert!(description.contains(
            "Locator handlers and overlay auto-dismiss do NOT guard `mouse_*` coordinate actions"
        ));
    }

    #[test]
    fn chrome_description_documents_wait_for_url_substring_semantics() {
        let description = ToolChrome::default().tool_description().description;
        assert!(description.contains("wait_for_url takes a plain substring in `pattern`"));
    }

    #[test]
    fn chrome_schema_offers_the_gallery_and_element_state_captures() {
        let schema = chrome_input_schema();
        let properties = schema
            .pointer("/properties/request/properties/steps/items/properties")
            .and_then(Value::as_object)
            .unwrap();

        assert_eq!(
            properties["compose"]["enum"],
            serde_json::json!(["grid", "separate"])
        );
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
            assert!(
                CHROME_DESCRIPTION.contains(action),
                "description must mention {action}"
            );
        }
    }

    #[test]
    fn chrome_schema_documents_tri_state_attach_screenshot() {
        let schema = chrome_input_schema();
        assert_eq!(
            schema
                .pointer("/properties/request/properties/attach_screenshot/description")
                .and_then(Value::as_str),
            Some("Screenshot override: true = always attach, false = never attach, omitted = follow page_context")
        );
        let description = ToolChrome::default().tool_description().description;
        assert!(description.contains("`attach_screenshot` remains the tri-state screenshot override"));
        assert!(description.contains("false = never attach"));
        assert!(description.contains("omitted = follow `page_context`"));
    }

    #[test]
    fn chrome_schema_offers_every_page_context_mode() {
        let schema = chrome_input_schema();
        assert_eq!(
            schema.pointer("/properties/request/properties/page_context/enum"),
            Some(&serde_json::json!(["snapshot", "screenshot", "both", "none"]))
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
        assert!(description.contains("`snapshot` (the default) attaches the ref-annotated ARIA snapshot and NO image"));
        assert!(description.contains("`none` attaches only the page header"));
    }

    #[test]
    fn chrome_description_documents_the_page_header_and_snapshot_budget() {
        let description = ToolChrome::default().tool_description().description;
        assert!(description.contains("page.status"));
        assert!(description.contains("error/warning COUNTS"));
        assert!(description.contains("full text stays in `console` and `tab_log`"));
        assert!(description.contains("{artifact:{kind,mime,path,bytes}}"));
        assert!(description.contains("locator_echo"));
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
        assert!(description.contains("Route interception telemetry"));
    }

    #[test]
    fn suppressed_report_screenshot_still_returns_the_explicit_step_screenshot() {
        const TINY_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let step = crate::integrations::browser_models::StepResult::success(0, "Screenshot captured")
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
        let description = ToolChrome::default().tool_description().description;
        assert!(description.contains("capture_frames records a burst"));
        assert!(description.contains(&MAX_SESSION_DURATION_MS.to_string()));
        assert!(description.contains(&MAX_SESSION_FRAMES.to_string()));
    }
}

async fn setup_chrome_session(
    gcx: Arc<GlobalContext>,
    args: &SettingsChrome,
    session_hashmap_key: &String,
    chat_id: &str,
) -> Result<Vec<String>, String> {
    let mut setup_log = vec![];

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
            setup_log.push("Browser session is disconnected. Trying to reconnect.".to_string());
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

        setup_log.push(format!(
            "Started new chrome process ({}).",
            launch_options.mode_label()
        ));
        BrowserRuntime::launch(profile_dir, launch_options)?
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
            let method = match device {
                DeviceType::Desktop => {
                    let (width, height) = match (
                        settings_chrome.window_width.parse::<u32>(),
                        settings_chrome.window_height.parse::<u32>(),
                    ) {
                        (Ok(width), Ok(height)) => (width, height),
                        _ => (1440, 900),
                    };
                    let scale_factor = match settings_chrome.scale_factor.parse::<f64>() {
                        Ok(scale_factor) => scale_factor,
                        _ => 2.0,
                    };
                    set_device_metrics_method(width, height, scale_factor, false)
                }
                DeviceType::Mobile => {
                    let (width, height) = match (
                        settings_chrome.mobile_window_width.parse::<u32>(),
                        settings_chrome.mobile_window_height.parse::<u32>(),
                    ) {
                        (Ok(width), Ok(height)) => (width, height),
                        _ => (390, 844),
                    };
                    let scale_factor = match settings_chrome.mobile_scale_factor.parse::<f64>() {
                        Ok(scale_factor) => scale_factor,
                        _ => 3.0,
                    };
                    set_device_metrics_method(width, height, scale_factor, true)
                }
                DeviceType::Tablet => {
                    let (width, height) = match (
                        settings_chrome.tablet_window_width.parse::<u32>(),
                        settings_chrome.tablet_window_height.parse::<u32>(),
                    ) {
                        (Ok(width), Ok(height)) => (width, height),
                        _ => (834, 1112),
                    };
                    let scale_factor = match settings_chrome.tablet_scale_factor.parse::<f64>() {
                        Ok(scale_factor) => scale_factor,
                        _ => 2.0,
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

fn format_controller_report(
    report: &ExecutionReport,
    tab_state: &str,
    image_policy: &ImagePolicy,
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
            if step_image_is_attachable(report, data) {
                if let (Some(mime), Some(b64_data)) = (
                    data.get("mime").and_then(|v| v.as_str()),
                    data.get("data").and_then(|v| v.as_str()),
                ) {
                    if mime.starts_with("image/") {
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
    redact_browser_credential_fields(&mut text_report);
    strip_binary_data_for_text(&mut text_report);
    let text_pretty = serde_json::to_string_pretty(&text_report)
        .map_err(|e| format!("Failed to pretty-print browser report: {}", e))?;
    content.push(MultimodalElement::new("text".to_string(), text_pretty)?);

    for result in &report.steps {
        if let Some(ref data) = result.data {
            if !step_image_is_attachable(report, data) {
                continue;
            }
            if let (Some(mime), Some(b64_data)) = (
                data.get("mime").and_then(|v| v.as_str()),
                data.get("data").and_then(|v| v.as_str()),
            ) {
                if mime.starts_with("image/") {
                    let (resized, resized_mime) =
                        resize_screenshot_b64(b64_data, mime, image_policy)?;
                    if let Ok(el) = MultimodalElement::new(resized_mime, resized) {
                        content.push(el);
                    }
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

#[cfg(test)]
mod browser_credential_redaction_tests {
    use super::redact_browser_credential_fields;

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
