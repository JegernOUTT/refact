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
    "Ref-first batched browser automation. Prefer the typed `request`: ONE call can carry many steps, unlike one-action-per-call servers. ",
    "Take `accessibility_snapshot`, read its `[ref=eN]` handles, then act with `locator.by=ref`; refs come from the most recent snapshot. ",
    "Canonical batch: {\"steps\":[{\"action\":\"accessibility_snapshot\"},{\"action\":\"click\",\"locator\":{\"by\":\"ref\",\"value\":\"e5\"}},{\"action\":\"fill\",\"locator\":{\"by\":\"ref\",\"value\":\"e7\"},\"text\":\"hi\"}]} Pass this object as `request`; e5/e7 stand for handles minted by the latest snapshot.\n",
    "Core: navigate, reload, go_back, go_forward, open_tab, close_tab, switch_tab, list_tabs, click, click_if_exists, hover, focus, blur, scroll_to, press_key. open_tab accepts optional device/url; close_tab accepts an optional tab and otherwise closes active. Closing active selects the preceding tab in adoption order, the next tab when closing the first, or leaves no active tab.\n",
    "Forms: fill, clear, select_option, check, uncheck.\n",
    "Assertions: expect retries with a 5000ms default and supports state, text/value, attribute/class/CSS/id/property, role/accessibility, count, URL/title, and ARIA snapshot matchers. Assertion failures report expected and last received values; set soft=true to record a failure and continue the batch.\n",
    "Waiting: wait_for_popup, wait_for_selector, wait_for_navigation, wait_for_url, wait_for_text, wait_for_network_idle, wait_for_load_state, wait_for_element_hidden, wait_for_element_stable. Put wait_for_popup immediately before the popup-producing click in ONE batch; the returned popup becomes active for later steps. ",
    "Click, hover, fill, clear, check, and uncheck auto-wait for actionability. Never use `wait_seconds` for readiness; use `wait_for_response`, `wait_for_load_state`, or `wait_for_selector` for genuine synchronization.\n",
    "Inspection: get_text, get_html, get_attribute, extract_links, extract_table, dom_snapshot, accessibility_snapshot, screenshot, screenshot_element, pdf, styles, tab_log. Screenshots support full_page, clip, type, quality, scale, omit_background, animations, caret, mask, mask_color, and style; screenshot_element uses locator or ref. PDF supports Chromium print options and returns an artifact path.\n",
    "Network: wait_for_request and wait_for_response accept a URL string or `{source,flags}` regex; completed requests also appear in the report. route registers a persistent `{pattern,handler}` with fulfill, abort, or continue modifications; unroute removes one pattern or all routes; list_routes returns active routes. Text route bodies are UTF-8 and encoded to base64 on the CDP wire; set body_base64=true when body already contains base64 binary data. Page-level routes may not observe requests served by a service worker.\n",
    "Context: set_viewport, emulate_media, set_locale, set_timezone, set_user_agent, set_geolocation, set_offline, and set_extra_http_headers persist across adopted tabs and popups. Cookie state uses get_cookies, set_cookies, clear_cookies. Web storage uses get_storage, set_storage, clear_storage with kind local or session. storage_state and set_storage_state use Playwright's {cookies,origins:[{origin,local_storage}]} login-reuse shape. grant_permissions and clear_permissions control origin permissions. set_http_credentials shares the lazy Fetch path with routing. Cookie, storage, and credential values are redacted in reports.\n",
    "Files: set_input_files, expect_file_chooser, wait_for_download.\n",
    "Dialogs: handle_dialog arms the next dialog with `accept` and optional `prompt_text`; unarmed dialogs auto-dismiss except beforeunload, which is accepted.\n",
    "Advanced: eval, add_locator_handler, remove_locator_handler, dismiss_overlays, highlight_element, highlight, hide_highlight, annotate, and fixed-delay wait_seconds. highlight accepts locator/ref plus optional style and label; annotate accepts locator/ref plus text. Locator handlers use `{type:\"click\"}` or `{type:\"steps\",steps:[...]}`.\n",
    "Locator fallback vocabulary: ref; role with name/description, exact or regex, and checked/pressed/selected/expanded/disabled/level/include_hidden filters; test_id with configurable `attribute`; text, label, placeholder, alt_text, title, css, xpath, id, name, and autocomplete. ",
    "Compose with zero-based `nth` (-1 is last), first/last, locator, filter (has/has_not/has_text/has_not_text/visible), and/or, or an outermost-first `frames` chain. Non-selecting actions are strict: ambiguous locators fail loudly with the match count. Same-process frames are supported; out-of-process frames fail explicitly.\n",
    "Set `attach_screenshot=true` for a policy-sized screenshot; page-changing batches capture automatically. The legacy newline-separated `commands` input remains accepted but is deprecated; new callers must use `request.steps`."
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
    properties.insert("root".to_string(), browser_locator_schema());
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
    properties.insert(
        "contains".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert("value".to_string(), serde_json::json!({"type": "string"}));
    properties.insert(
        "attribute".to_string(),
        serde_json::json!({"type": "string"}),
    );
    properties.insert("seconds".to_string(), serde_json::json!({"type": "number"}));
    properties.insert(
        "timeout_ms".to_string(),
        serde_json::json!({"type": "integer", "minimum": 0}),
    );
    properties.insert(
        "matcher".to_string(),
        serde_json::json!({
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
        serde_json::json!({"type": "string", "enum": ["ai", "default"], "description": "Accessibility snapshot mode; defaults to ai"}),
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
        "paths".to_string(),
        serde_json::json!({"type": "array", "items": {"type": "string"}}),
    );
    properties.insert(
        "state".to_string(),
        serde_json::json!({"type": "string", "enum": ["domcontentloaded", "load", "networkidle"]}),
    );
    properties.insert("url_or_pattern".to_string(), url_pattern_schema());
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
                "required": ["type", "status"],
                "properties": {
                    "type": {"type": "string", "enum": ["fulfill"]},
                    "status": {"type": "integer", "minimum": 100, "maximum": 599},
                    "headers": {"type": "object", "additionalProperties": {"type": "string"}},
                    "content_type": {"type": "string"},
                    "body": {"type": "string", "description": "UTF-8 text by default; set body_base64=true when this string contains base64-encoded binary response bytes"},
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
        .filter(|action| !matches!(*action, "route" | "unroute" | "list_routes"))
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
            "attach_screenshot": {"type": "boolean", "description": "Include a policy-sized screenshot; page-changing batches capture automatically"},
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
            "caret",
            "clear_first",
            "clip",
            "contains",
            "device",
            "expression",
            "format",
            "full_page",
            "handler",
            "height",
            "key",
            "label",
            "landscape",
            "limit",
            "locator",
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
            "paths",
            "pattern",
            "prefer_css_page_size",
            "print_background",
            "prompt_text",
            "property_filter",
            "quality",
            "refs",
            "root",
            "save_as",
            "scale",
            "seconds",
            "selector",
            "state",
            "style",
            "tab",
            "tagged",
            "text",
            "timeout_ms",
            "times",
            "type",
            "url",
            "url_or_pattern",
            "value",
            "verify",
            "width",
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
            .pointer("/properties/request/properties/steps/items/properties/url_or_pattern/oneOf/1/required")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(url_pattern_required, &[Value::String("source".to_string())]);
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
            BTreeSet::from(["abort", "continue", "fulfill"])
        );
        assert!(schema
            .pointer("/properties/request/properties/steps/items/properties/handler/oneOf/1/oneOf/0/properties/body/description")
            .and_then(Value::as_str)
            .is_some_and(|description| description.contains("UTF-8") && description.contains("base64")));
    }

    #[test]
    fn chrome_description_is_ref_first_batched_and_actionability_aware() {
        let description = ToolChrome::default().tool_description().description;
        let canonical = "{\"steps\":[{\"action\":\"accessibility_snapshot\"},{\"action\":\"click\",\"locator\":{\"by\":\"ref\",\"value\":\"e5\"}},{\"action\":\"fill\",\"locator\":{\"by\":\"ref\",\"value\":\"e7\"},\"text\":\"hi\"}]}";
        assert!(description.starts_with("Ref-first batched browser automation."));
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

    let idle_browser_timeout = args
        .idle_browser_timeout
        .parse::<u64>()
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(600));

    let runtime = if args.chrome_path.starts_with("ws://") {
        setup_log.push("Connect to existing web socket.".to_string());
        BrowserRuntime::connect(args.chrome_path.clone(), Some(idle_browser_timeout), true)?
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
        BrowserRuntime::connect(ws_url, Some(idle_browser_timeout), true)?
    } else {
        let chrome_path = if args.chrome_path.is_empty() {
            None
        } else {
            Some(PathBuf::from(args.chrome_path.clone()))
        };
        let cache_dir = gcx.cache_dir.clone();
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

        if report.screenshot.is_none() {
            if let Some(ref data) = result.data {
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
    strip_binary_data_for_text(&mut text_report);
    let text_pretty = serde_json::to_string_pretty(&text_report)
        .map_err(|e| format!("Failed to pretty-print browser report: {}", e))?;
    content.push(MultimodalElement::new("text".to_string(), text_pretty)?);

    if report.screenshot.is_none() {
        for result in &report.steps {
            if let Some(ref data) = result.data {
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
