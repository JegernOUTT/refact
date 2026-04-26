use std::sync::Arc;
use std::time::{Duration, Instant};

use headless_chrome::Tab;
use headless_chrome::browser::tab::ModifierKey;
use headless_chrome::protocol::cdp::Page;
use tokio::sync::Mutex as AMutex;

use crate::integrations::browser_locators::{
    self, generate_find_fragment_js, generate_resolve_js, js_string_literal, parse_element_info,
    to_css_selector, INSPECT_ELEMENT_JS,
};
use crate::integrations::browser_models::*;
use crate::integrations::browser_runtime::BrowserRuntime;

const DEFAULT_WAIT_TIMEOUT_MS: u64 = 5_000;
const MAX_WAIT_TIMEOUT_MS: u64 = 60_000;
const MAX_WAIT_SECONDS: f64 = 60.0;
const MIN_WAIT_SECONDS: f64 = 0.0;

const MAX_DOM_SNAPSHOT_CHARS: usize = 100_000;
const MAX_EXTRACT_LINKS: usize = 500;
const ACCESSIBILITY_MAX_NODES: usize = 1_000;
const ACCESSIBILITY_MAX_DEPTH: u32 = 6;
const ACCESSIBILITY_MAX_CHILDREN: u32 = 20;

fn clamp_timeout_ms(requested: Option<u64>) -> u64 {
    requested
        .unwrap_or(DEFAULT_WAIT_TIMEOUT_MS)
        .min(MAX_WAIT_TIMEOUT_MS)
}

fn clamp_wait_seconds(requested: f64) -> f64 {
    if requested.is_nan() || requested.is_infinite() || requested < MIN_WAIT_SECONDS {
        MIN_WAIT_SECONDS
    } else if requested > MAX_WAIT_SECONDS {
        MAX_WAIT_SECONDS
    } else {
        requested
    }
}

const DEFAULT_POLL_INTERVAL_MS: u64 = 200;

#[allow(dead_code)]
const SCREENSHOT_MAX_DIM: u32 = 800;

#[allow(dead_code)]
pub fn resolve_tab(runtime: &BrowserRuntime, target: &TabTarget) -> Result<Arc<Tab>, String> {
    match target {
        TabTarget::Active => runtime
            .get_active_tab()
            .ok_or_else(|| "No active tab in browser runtime".to_string()),
        TabTarget::Id { id } => {
            let tabs = runtime
                .browser
                .get_tabs()
                .lock()
                .map_err(|e| format!("Failed to lock browser tabs: {}", e))?;
            tabs.iter()
                .find(|t| t.get_target_id() == id)
                .cloned()
                .ok_or_else(|| format!("Tab not found with id: {}", id))
        }
    }
}

fn eval_js_value(tab: &Tab, js: &str) -> Result<serde_json::Value, String> {
    let remote = tab
        .evaluate(js, false)
        .map_err(|e| format!("JS evaluation failed: {}", e))?;
    remote
        .value
        .ok_or_else(|| "JS evaluation returned no value".to_string())
}

fn eval_js_json(tab: &Tab, js: &str) -> Result<serde_json::Value, String> {
    let val = eval_js_value(tab, js)?;
    match val.as_str() {
        Some(s) => {
            serde_json::from_str(s).map_err(|e| format!("Failed to parse JS JSON result: {}", e))
        }
        None if val.is_object() || val.is_array() => Ok(val),
        None => Err(format!("Unexpected JS result type: {:?}", val)),
    }
}

fn eval_js_ok(tab: &Tab, js: &str) -> Result<serde_json::Value, String> {
    let result = eval_js_json(tab, js)?;
    if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
        return Err(err.to_string());
    }
    Ok(result)
}

fn resolve_element(tab: &Tab, locator: &BrowserLocator) -> Result<ElementInfo, String> {
    let js = generate_resolve_js(locator);
    let val = eval_js_value(tab, &js)?;
    let json_str = match val.as_str() {
        Some(s) => s.to_string(),
        None => serde_json::to_string(&val)
            .map_err(|e| format!("Failed to serialize resolve result: {}", e))?,
    };
    parse_element_info(&json_str)
}

fn resolve_interactable(tab: &Tab, locator: &BrowserLocator) -> Result<ElementInfo, String> {
    let info = resolve_element(tab, locator)?;
    if !info.visible {
        return Err("Element is not visible".to_string());
    }
    if !info.enabled {
        return Err("Element is disabled".to_string());
    }
    Ok(info)
}

pub fn execute_steps(tab: &Tab, steps: &[BrowserStep]) -> ExecutionReport {
    let _ = tab.evaluate(INSPECT_ELEMENT_JS, false);

    let mut results = Vec::new();
    let mut all_ok = true;
    let mut pre_step_url: Option<String> = Some(tab.get_url());

    for (idx, step) in steps.iter().enumerate() {
        let result = execute_single_step(tab, step, idx, pre_step_url.as_deref());
        let is_non_fatal = matches!(step, BrowserStep::ClickIfExists { .. });
        if !result.ok && !is_non_fatal {
            all_ok = false;
            results.push(result);
            break;
        }
        if result.ok && is_navigation_step(step) {
            let _ = tab.evaluate(INSPECT_ELEMENT_JS, false);
        }
        pre_step_url = Some(tab.get_url());
        results.push(result);
    }

    ExecutionReport {
        ok: all_ok,
        steps: results,
        url: Some(tab.get_url()),
        title: tab.get_title().ok(),
    }
}

pub fn is_tab_management_step(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::OpenTab { .. }
            | BrowserStep::CloseTab
            | BrowserStep::SwitchTab { .. }
            | BrowserStep::ListTabs
    )
}

pub fn execute_step(tab: &Tab, step: &BrowserStep, idx: usize) -> StepResult {
    let _ = tab.evaluate(INSPECT_ELEMENT_JS, false);
    let result = execute_single_step(tab, step, idx, None);
    if result.ok && is_navigation_step(step) {
        let _ = tab.evaluate(INSPECT_ELEMENT_JS, false);
    }
    result
}

pub async fn execute_request_with_runtime(
    runtime_arc: Arc<AMutex<BrowserRuntime>>,
    request: BrowserActionRequest,
) -> Result<ExecutionReport, String> {
    if request.session != SessionPolicy::SharedDefault {
        return Err(format!(
            "Unsupported browser session policy: {:?}",
            request.session
        ));
    }

    {
        let mut rt = runtime_arc.lock().await;
        rt.touch();
        if let TabTarget::Id { id } = &request.target {
            let tabs = rt
                .browser
                .get_tabs()
                .lock()
                .map(|t| t.clone())
                .unwrap_or_default();
            let tab = tabs
                .iter()
                .find(|t| t.get_target_id() == id.as_str())
                .ok_or_else(|| format!("No tab found with id={}", id))?;
            rt.set_active_tab_target_id(tab.get_target_id().to_string());
        }
    }

    let mut current_tab = {
        let rt = runtime_arc.lock().await;
        rt.get_active_tab()
    };
    let mut results = Vec::new();
    let mut all_ok = true;

    for (idx, step) in request.steps.iter().enumerate() {
        let mut result = if is_tab_management_step(step) {
            let step_report = tokio::task::block_in_place(|| {
                let mut rt = runtime_arc.blocking_lock();
                execute_steps_with_runtime(&mut rt, std::slice::from_ref(step))
            });
            {
                let mut rt = runtime_arc.lock().await;
                rt.touch();
                current_tab = rt.get_active_tab();
            }
            step_report.steps.into_iter().next().unwrap_or_else(|| {
                StepResult::failure(idx, "Browser action", "No step result produced")
            })
        } else {
            if current_tab.is_none() {
                let mut rt = runtime_arc.lock().await;
                rt.touch();
                current_tab = rt.get_active_tab();
            }
            match &current_tab {
                Some(tab) => tokio::task::block_in_place(|| execute_step(tab, step, idx)),
                None => StepResult::failure(
                    idx,
                    "No active tab",
                    "No tab available. Use OpenTab first.",
                ),
            }
        };
        result.step_index = idx;

        {
            let mut rt = runtime_arc.lock().await;
            rt.touch();
            let action_type = if result.ok { "action" } else { "error" };
            rt.push_agent_action(action_type, &result.summary);
        }

        let is_non_fatal = matches!(step, BrowserStep::ClickIfExists { .. });
        if !result.ok && !is_non_fatal {
            all_ok = false;
            results.push(result);
            break;
        }
        results.push(result);
    }

    let (url, title) = if let Some(tab) = current_tab {
        (Some(tab.get_url()), tab.get_title().ok())
    } else {
        let rt = runtime_arc.lock().await;
        match rt.get_active_tab() {
            Some(tab) => (Some(tab.get_url()), tab.get_title().ok()),
            None => (None, None),
        }
    };

    Ok(ExecutionReport {
        ok: all_ok,
        steps: results,
        url,
        title,
    })
}

pub fn execute_steps_with_runtime(
    runtime: &mut BrowserRuntime,
    steps: &[BrowserStep],
) -> ExecutionReport {
    let mut current_tab: Option<Arc<Tab>> = runtime.get_active_tab();
    if let Some(ref tab) = current_tab {
        let _ = tab.evaluate(INSPECT_ELEMENT_JS, false);
    }

    let mut results = Vec::new();
    let mut all_ok = true;
    let mut pre_step_url: Option<String> = current_tab.as_ref().map(|t| t.get_url());

    for (idx, step) in steps.iter().enumerate() {
        let result = match step {
            BrowserStep::OpenTab { device } => match runtime.browser.new_tab() {
                Ok(new_tab) => {
                    let device_label = device.as_deref().unwrap_or("desktop");
                    let target_id = new_tab.get_target_id().to_string();
                    let (w, h, mobile) = match device.as_deref() {
                        Some("mobile") => (400, 800, true),
                        Some("tablet") => (600, 800, true),
                        _ => (800, 600, false),
                    };
                    let _ = new_tab.call_method(
                        headless_chrome::protocol::cdp::Emulation::SetDeviceMetricsOverride {
                            width: w,
                            height: h,
                            device_scale_factor: 0.0,
                            mobile,
                            screen_width: None,
                            screen_height: None,
                            position_x: None,
                            position_y: None,
                            dont_set_visible_size: None,
                            screen_orientation: None,
                            viewport: None,
                            display_feature: None,
                            device_posture: None,
                            scale: None,
                        },
                    );
                    let _ = crate::integrations::browser_runtime::setup_recording_for_tab(
                        runtime, &new_tab,
                    );
                    let _ = new_tab.evaluate(INSPECT_ELEMENT_JS, false);
                    current_tab = Some(new_tab);
                    runtime.set_active_tab_target_id(target_id.clone());
                    StepResult::success(
                        idx,
                        format!(
                            "Opened new {} tab ({})",
                            device_label,
                            &target_id[..8.min(target_id.len())]
                        ),
                    )
                    .with_data(serde_json::json!({"target_id": target_id}))
                }
                Err(e) => StepResult::failure(idx, "OpenTab", &format!("Failed: {}", e)),
            },
            BrowserStep::CloseTab => {
                let tab = match &current_tab {
                    Some(t) => t.clone(),
                    None => {
                        all_ok = false;
                        results.push(StepResult::failure(idx, "CloseTab", "No active tab"));
                        break;
                    }
                };
                let target_id = tab.get_target_id().to_string();
                match tab.close(false) {
                    Ok(_) => {
                        if runtime.recording_tab_target_id.as_deref() == Some(&target_id) {
                            runtime.recording_tab_target_id = None;
                        }
                        if runtime.active_tab_target_id().as_deref() == Some(target_id.as_str()) {
                            runtime.active_tab_target_id = None;
                        }
                        current_tab = runtime.get_active_tab();
                        StepResult::success(
                            idx,
                            format!("Closed tab {}", &target_id[..8.min(target_id.len())]),
                        )
                    }
                    Err(e) => StepResult::failure(idx, "CloseTab", &format!("Failed: {}", e)),
                }
            }
            BrowserStep::SwitchTab { tab: tab_target } => {
                let tabs = runtime
                    .browser
                    .get_tabs()
                    .lock()
                    .map(|t| t.clone())
                    .unwrap_or_default();
                let target_str = match tab_target {
                    TabTarget::Active => String::from("active"),
                    TabTarget::Id { id } => id.clone(),
                };
                let found = match tab_target {
                    TabTarget::Active => runtime.get_active_tab().or_else(|| tabs.first().cloned()),
                    TabTarget::Id { id } => tabs
                        .iter()
                        .find(|t| t.get_target_id() == id.as_str())
                        .cloned(),
                };
                match found {
                    Some(found_tab) => {
                        runtime.set_active_tab_target_id(found_tab.get_target_id().to_string());
                        let _ = found_tab.evaluate(INSPECT_ELEMENT_JS, false);
                        current_tab = Some(found_tab.clone());
                        StepResult::success(
                            idx,
                            format!("Switched to tab {} ({})", target_str, found_tab.get_url()),
                        )
                    }
                    None => StepResult::failure(
                        idx,
                        "SwitchTab",
                        format!("No tab matching '{}'", target_str),
                    ),
                }
            }
            BrowserStep::ListTabs => {
                let tab_list = runtime
                    .list_tab_infos()
                    .into_iter()
                    .map(|tab| serde_json::to_value(tab).unwrap_or_default())
                    .collect::<Vec<_>>();
                StepResult::success(idx, format!("Listed {} tabs", tab_list.len()))
                    .with_data(serde_json::json!({"tabs": tab_list}))
            }
            other => match &current_tab {
                Some(tab) => execute_single_step(tab, other, idx, pre_step_url.as_deref()),
                None => StepResult::failure(
                    idx,
                    "No active tab",
                    "No tab available. Use OpenTab first.",
                ),
            },
        };

        let is_non_fatal = matches!(step, BrowserStep::ClickIfExists { .. });
        if !result.ok && !is_non_fatal {
            all_ok = false;
            results.push(result);
            break;
        }
        if result.ok && is_navigation_step(step) {
            if let Some(ref tab) = current_tab {
                let _ = tab.evaluate(INSPECT_ELEMENT_JS, false);
            }
        }
        pre_step_url = current_tab.as_ref().map(|t| t.get_url());
        results.push(result);
    }

    let (url, title) = match &current_tab {
        Some(tab) => (Some(tab.get_url()), tab.get_title().ok()),
        None => (None, None),
    };
    ExecutionReport {
        ok: all_ok,
        steps: results,
        url,
        title,
    }
}

fn is_navigation_step(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::Navigate { .. }
            | BrowserStep::Reload
            | BrowserStep::GoBack
            | BrowserStep::GoForward
    )
}

fn execute_single_step(
    tab: &Tab,
    step: &BrowserStep,
    idx: usize,
    pre_step_url: Option<&str>,
) -> StepResult {
    match step {
        BrowserStep::Navigate { url } => step_navigate(tab, idx, url),
        BrowserStep::Reload => step_nav_js(tab, idx, "location.reload()", "Reloaded page"),
        BrowserStep::GoBack => step_nav_js(tab, idx, "history.back()", "Navigated back"),
        BrowserStep::GoForward => step_nav_js(tab, idx, "history.forward()", "Navigated forward"),

        BrowserStep::OpenTab { .. }
        | BrowserStep::CloseTab
        | BrowserStep::SwitchTab { .. }
        | BrowserStep::ListTabs => StepResult::failure(
            idx,
            "Tab management step",
            "Use execute_steps_with_runtime() for tab management",
        ),

        BrowserStep::Click { locator } => step_locator_action(tab, idx, locator, "click"),
        BrowserStep::ClickIfExists { locator } => step_click_if_exists(tab, idx, locator),
        BrowserStep::Hover { locator } => step_locator_action(tab, idx, locator, "hover"),
        BrowserStep::Focus { locator } => step_locator_action(tab, idx, locator, "focus"),
        BrowserStep::Blur { locator } => step_locator_action(tab, idx, locator, "blur"),
        BrowserStep::ScrollTo { locator } => step_locator_action(tab, idx, locator, "scroll_to"),
        BrowserStep::PressKey { key, modifiers } => step_press_key(tab, idx, key, modifiers),

        BrowserStep::Fill {
            locator,
            text,
            clear_first,
            verify,
        } => step_fill(tab, idx, locator, text, *clear_first, *verify),
        BrowserStep::Clear { locator, verify } => step_clear(tab, idx, locator, *verify),
        BrowserStep::SelectOption { locator, value } => {
            step_select_option(tab, idx, locator, value)
        }
        BrowserStep::Check { locator } => step_check_uncheck(tab, idx, locator, true),
        BrowserStep::Uncheck { locator } => step_check_uncheck(tab, idx, locator, false),

        BrowserStep::WaitForSelector {
            locator,
            timeout_ms,
        } => step_wait_for_selector(tab, idx, locator, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitForNavigation { timeout_ms } => {
            step_wait_for_navigation(tab, idx, clamp_timeout_ms(*timeout_ms), pre_step_url)
        }
        BrowserStep::WaitForUrl {
            contains,
            timeout_ms,
        } => step_wait_for_url(tab, idx, contains, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitForText { text, timeout_ms } => {
            step_wait_for_text(tab, idx, text, clamp_timeout_ms(*timeout_ms))
        }
        BrowserStep::WaitForNetworkIdle { timeout_ms } => {
            step_wait_for_network_idle(tab, idx, clamp_timeout_ms(*timeout_ms))
        }
        BrowserStep::WaitForElementHidden {
            locator,
            timeout_ms,
        } => step_wait_for_element_hidden(tab, idx, locator, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitForElementStable {
            locator,
            timeout_ms,
        } => step_wait_for_element_stable(tab, idx, locator, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitSeconds { seconds } => {
            step_wait_seconds(idx, clamp_wait_seconds(*seconds))
        }

        BrowserStep::GetText { locator } => step_get_text(tab, idx, locator),
        BrowserStep::GetHtml { locator } => step_get_html(tab, idx, locator),
        BrowserStep::GetAttribute { locator, attribute } => {
            step_get_attribute(tab, idx, locator, attribute)
        }
        BrowserStep::ExtractLinks { locator, limit } => {
            step_extract_links(tab, idx, locator.as_ref(), *limit)
        }
        BrowserStep::ExtractTable { locator } => step_extract_table(tab, idx, locator),
        BrowserStep::DomSnapshot {
            selector,
            max_chars,
        } => step_dom_snapshot(tab, idx, selector, *max_chars),
        BrowserStep::AccessibilitySnapshot => step_accessibility_snapshot(tab, idx),
        BrowserStep::Screenshot => step_screenshot(tab, idx),
        BrowserStep::ScreenshotElement { locator } => step_screenshot_element(tab, idx, locator),

        BrowserStep::Eval { expression } => step_eval(tab, idx, expression),
        BrowserStep::Styles {
            locator,
            property_filter,
        } => step_styles(tab, idx, locator, property_filter.as_deref()),

        BrowserStep::TabLog => step_tab_log(tab, idx),

        BrowserStep::DismissOverlays => step_dismiss_overlays(tab, idx),
        BrowserStep::HighlightElement { locator } => step_highlight_element(tab, idx, locator),
    }
}

fn step_navigate(tab: &Tab, idx: usize, url: &str) -> StepResult {
    match tab.navigate_to(url) {
        Ok(_) => {
            let _ = tab.wait_until_navigated();
            StepResult::success(idx, format!("Navigated to {}", url))
        }
        Err(e) => StepResult::failure(idx, format!("Navigate to {}", url), e.to_string()),
    }
}

fn step_nav_js(tab: &Tab, idx: usize, js: &str, success_msg: &str) -> StepResult {
    match tab.evaluate(js, false) {
        Ok(_) => {
            let _ = tab.wait_until_navigated();
            StepResult::success(idx, success_msg.to_string())
        }
        Err(e) => StepResult::failure(idx, success_msg.to_string(), e.to_string()),
    }
}

fn step_locator_action(
    tab: &Tab,
    idx: usize,
    locator: &BrowserLocator,
    action: &str,
) -> StepResult {
    match resolve_interactable(tab, locator) {
        Ok(info) => {
            let action_js = match action {
                "click" => browser_locators::js_click_element().to_string(),
                "hover" => browser_locators::js_hover_element().to_string(),
                "focus" => browser_locators::js_focus_element().to_string(),
                "blur" => browser_locators::js_blur_element().to_string(),
                "scroll_to" => browser_locators::js_scroll_to_element().to_string(),
                _ => {
                    return StepResult::failure(idx, action, format!("Unknown action: {}", action))
                }
            };
            match eval_js_ok(tab, &action_js) {
                Ok(_) => StepResult::success(
                    idx,
                    format!(
                        "{} on <{}> ({})",
                        action,
                        info.tag,
                        describe_locator(locator)
                    ),
                ),
                Err(e) => StepResult::failure(idx, format!("{} failed", action), e),
            }
        }
        Err(e) => StepResult::failure(idx, format!("{} failed", action), e),
    }
}

fn step_click_if_exists(tab: &Tab, idx: usize, locator: &BrowserLocator) -> StepResult {
    match resolve_element(tab, locator) {
        Ok(info) if info.visible => match eval_js_ok(tab, browser_locators::js_click_element()) {
            Ok(_) => StepResult::success(idx, format!("Clicked <{}> (found)", info.tag)),
            Err(e) => StepResult::success(
                idx,
                format!("Click on <{}> failed (non-fatal): {}", info.tag, e),
            ),
        },
        _ => StepResult::success(idx, "Element not found or not visible, skipped".to_string()),
    }
}

fn step_press_key(tab: &Tab, idx: usize, key: &str, modifiers: &[String]) -> StepResult {
    let modifier_keys: Option<Vec<ModifierKey>> = if modifiers.is_empty() {
        None
    } else {
        Some(
            modifiers
                .iter()
                .map(|m| match m.as_str() {
                    "Alt" => ModifierKey::Alt,
                    "Ctrl" => ModifierKey::Ctrl,
                    "Meta" => ModifierKey::Meta,
                    "Shift" => ModifierKey::Shift,
                    _ => ModifierKey::Shift,
                })
                .collect(),
        )
    };
    match tab.press_key_with_modifiers(key, modifier_keys.as_deref()) {
        Ok(_) => {
            let mod_str = if modifiers.is_empty() {
                String::new()
            } else {
                format!("{}+", modifiers.join("+"))
            };
            StepResult::success(idx, format!("Pressed {}{}", mod_str, key))
        }
        Err(e) => StepResult::failure(idx, format!("Press key {}", key), e.to_string()),
    }
}

fn step_fill(
    tab: &Tab,
    idx: usize,
    locator: &BrowserLocator,
    text: &str,
    clear_first: bool,
    verify: bool,
) -> StepResult {
    let info = match resolve_interactable(tab, locator) {
        Ok(i) => i,
        Err(e) => return StepResult::failure(idx, "Fill: element resolution failed", e),
    };

    if info.readonly {
        return StepResult::failure(idx, "Fill failed", "Element is readonly");
    }

    let strategies = choose_fill_strategies(&info.field_kind);
    if strategies.is_empty() {
        return StepResult::failure(
            idx,
            "Fill failed",
            format!(
                "Cannot fill {:?} — use select_option or check/uncheck instead",
                info.field_kind
            ),
        );
    }

    let mut last_error = String::new();
    let mut retries = 0u32;

    for strategy in &strategies {
        let fill_js = generate_fill_js(strategy, text, clear_first);
        match eval_js_json(tab, &fill_js) {
            Ok(result) => {
                let ok = result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                if !ok {
                    last_error = result
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Strategy returned not-ok")
                        .to_string();
                    retries += 1;
                    continue;
                }

                if verify {
                    match verify_field_value(tab, text, &info.field_kind) {
                        Ok(true) => {
                            let mut r = StepResult::success(
                                idx,
                                format!("Filled <{}> with {} chars", info.tag, text.len()),
                            );
                            r.field_kind = Some(info.field_kind.clone());
                            r.fill_strategy = Some(strategy.clone());
                            r.verified = Some(true);
                            r.retries = retries;
                            return r;
                        }
                        Ok(false) => {
                            last_error = "Verification failed: value mismatch".to_string();
                            retries += 1;
                            continue;
                        }
                        Err(e) => {
                            last_error = format!("Verification error: {}", e);
                            retries += 1;
                            continue;
                        }
                    }
                }

                let mut r = StepResult::success(
                    idx,
                    format!("Filled <{}> with {} chars", info.tag, text.len()),
                );
                r.field_kind = Some(info.field_kind.clone());
                r.fill_strategy = Some(strategy.clone());
                r.verified = if verify { Some(true) } else { None };
                r.retries = retries;
                return r;
            }
            Err(e) => {
                last_error = e;
                retries += 1;
            }
        }
    }

    let mut r = StepResult::failure(
        idx,
        format!("Fill failed after {} strategies", retries),
        last_error,
    );
    r.field_kind = Some(info.field_kind);
    r.retries = retries;
    r
}

fn step_clear(tab: &Tab, idx: usize, locator: &BrowserLocator, verify: bool) -> StepResult {
    let info = match resolve_interactable(tab, locator) {
        Ok(i) => i,
        Err(e) => return StepResult::failure(idx, "Clear: element resolution failed", e),
    };

    match info.field_kind {
        FieldKind::Checkbox | FieldKind::Radio => {
            return StepResult::failure(
                idx,
                "Clear not supported for this field",
                format!(
                    "Use uncheck instead for <{}> ({:?})",
                    info.tag, info.field_kind
                ),
            );
        }
        FieldKind::FileInput => {
            return StepResult::failure(
                idx,
                "Clear not supported for file inputs",
                "Security restrictions prevent clearing file inputs programmatically".to_string(),
            );
        }
        FieldKind::HiddenInput => {
            return StepResult::failure(
                idx,
                "Clear not supported for hidden inputs",
                format!("Element <{}> is a hidden input", info.tag),
            );
        }
        FieldKind::Select => {
            let js = r#"(function() {
  var el = window.__refact_resolved_el;
  if (!el || el.tagName !== 'SELECT') return JSON.stringify({error: 'Not a SELECT element'});
  var hadEmpty = false;
  el.selectedIndex = -1;
  for (var i = 0; i < el.options.length; i++) {
    if (el.options[i].value === '' || el.options[i].text.trim() === '') {
      el.selectedIndex = i;
      hadEmpty = true;
      break;
    }
  }
  el.dispatchEvent(new Event('change', {bubbles: true}));
  return JSON.stringify({ok: true, value: el.value, had_empty_option: hadEmpty});
})()"#;
            return match eval_js_ok(tab, js) {
                Ok(result) => {
                    let had_empty = result
                        .get("had_empty_option")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if verify && !had_empty {
                        StepResult::failure(
                            idx,
                            "Clear select: no empty option found",
                            "No option with empty value/text exists to select",
                        )
                    } else {
                        StepResult::success(idx, format!("Cleared <{}> (select)", info.tag))
                    }
                }
                Err(e) => StepResult::failure(idx, "Clear failed", e),
            };
        }
        _ => {}
    }

    let clear_js = generate_clear_js(&info.field_kind);
    match eval_js_ok(tab, &clear_js) {
        Ok(_) => {
            if verify {
                match verify_field_value(tab, "", &info.field_kind) {
                    Ok(true) => {
                        let mut r = StepResult::success(idx, format!("Cleared <{}>", info.tag));
                        r.verified = Some(true);
                        r
                    }
                    Ok(false) => StepResult::failure(
                        idx,
                        "Clear verification failed",
                        "Field still has content",
                    ),
                    Err(e) => StepResult::failure(idx, "Clear verification error", e),
                }
            } else {
                StepResult::success(idx, format!("Cleared <{}>", info.tag))
            }
        }
        Err(e) => StepResult::failure(idx, "Clear failed", e),
    }
}

fn step_select_option(tab: &Tab, idx: usize, locator: &BrowserLocator, value: &str) -> StepResult {
    match resolve_interactable(tab, locator) {
        Ok(info) => {
            let js = format!(
                r#"(function() {{
  var el = window.__refact_resolved_el;
  if (!el || el.tagName !== 'SELECT') return JSON.stringify({{error: 'Not a SELECT element'}});
  var val = {value};
  var found = false;
  for (var i = 0; i < el.options.length; i++) {{
    if (el.options[i].value === val || el.options[i].text.trim() === val) {{
      el.selectedIndex = i;
      found = true;
      break;
    }}
  }}
  if (!found) return JSON.stringify({{error: 'Option not found: ' + val}});
  el.dispatchEvent(new Event('change', {{bubbles: true}}));
  return JSON.stringify({{ok: true, selected: el.value}});
}})()"#,
                value = js_string_literal(value),
            );
            match eval_js_ok(tab, &js) {
                Ok(_) => {
                    StepResult::success(idx, format!("Selected '{}' in <{}>", value, info.tag))
                }
                Err(e) => StepResult::failure(idx, "Select option failed", e),
            }
        }
        Err(e) => StepResult::failure(idx, "Select: element resolution failed", e),
    }
}

fn step_check_uncheck(tab: &Tab, idx: usize, locator: &BrowserLocator, check: bool) -> StepResult {
    let action = if check { "check" } else { "uncheck" };
    let info = match resolve_interactable(tab, locator) {
        Ok(i) => i,
        Err(e) => return StepResult::failure(idx, "Check/uncheck: resolution failed", e),
    };

    if !check && info.field_kind == FieldKind::Radio {
        return StepResult::failure(
            idx,
            "Uncheck not supported for radio buttons",
            "Radio buttons cannot be unchecked; select a different radio instead".to_string(),
        );
    }

    let is_supported = matches!(info.field_kind, FieldKind::Checkbox | FieldKind::Radio);
    let is_aria = !is_supported && {
        let check_aria = r#"(function() {
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({ok: false});
  var role = el.getAttribute('role');
  var supported = role === 'checkbox' || role === 'switch' || role === 'radio';
  return JSON.stringify({ok: supported});
})()"#;
        eval_js_ok(tab, check_aria)
            .ok()
            .and_then(|v| v.get("ok").and_then(|b| b.as_bool()))
            .unwrap_or(false)
    };

    if !is_supported && !is_aria {
        return StepResult::failure(
            idx,
            format!("{} not supported for this element", action),
            format!(
                "Element <{}> has field_kind={:?} and no checkbox/radio/switch role",
                info.tag, info.field_kind
            ),
        );
    }

    let js = format!(
        r#"(function() {{
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  var want = {want};
  var role = el.getAttribute('role');
  var isAria = role === 'checkbox' || role === 'switch' || role === 'radio';
  if (isAria && !('checked' in el)) {{
    var current = el.getAttribute('aria-checked') === 'true';
    if (current !== want) {{
      el.click();
    }}
    var final_state = el.getAttribute('aria-checked') === 'true';
    return JSON.stringify({{ok: final_state === want, checked: final_state, verified: true}});
  }}
  if (el.checked !== want) {{
    el.click();
  }}
  return JSON.stringify({{ok: el.checked === want, checked: el.checked, verified: true}});
}})()"#,
        want = if check { "true" } else { "false" },
    );
    match eval_js_ok(tab, &js) {
        Ok(_) => StepResult::success(idx, format!("{}ed <{}>", action, info.tag)),
        Err(e) => StepResult::failure(idx, format!("{} failed", action), e),
    }
}

fn poll_condition(
    tab: &Tab,
    js_condition: &str,
    timeout_ms: u64,
    interval_ms: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match eval_js_value(tab, js_condition) {
            Ok(val) if val.as_bool() == Some(true) => return Ok(()),
            _ => {}
        }
        if Instant::now() >= deadline {
            return Err(format!("Timed out after {}ms", timeout_ms));
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
}

fn step_wait_for_selector(
    tab: &Tab,
    idx: usize,
    locator: &BrowserLocator,
    timeout_ms: u64,
) -> StepResult {
    let js = match (to_css_selector(locator), locator.nth) {
        (Some(css), None) => browser_locators::js_check_selector_present(&css),
        _ => {
            let find_fragment = generate_find_fragment_js(locator);
            format!(
                r#"(function() {{
  {find_fragment}
  return elements.length > 0;
}})()"#
            )
        }
    };
    match poll_condition(tab, &js, timeout_ms, DEFAULT_POLL_INTERVAL_MS) {
        Ok(()) => StepResult::success(
            idx,
            format!("Element found ({})", describe_locator(locator)),
        ),
        Err(e) => StepResult::failure(
            idx,
            format!("Wait for selector ({})", describe_locator(locator)),
            e,
        ),
    }
}

fn step_wait_for_navigation(
    tab: &Tab,
    idx: usize,
    timeout_ms: u64,
    pre_step_url: Option<&str>,
) -> StepResult {
    let current_url = tab.get_url();
    let reference_url = pre_step_url.unwrap_or(&current_url);

    if current_url != reference_url {
        let complete_js = r#"(function() { return document.readyState === 'complete'; })()"#;
        let _ = poll_condition(tab, complete_js, timeout_ms, DEFAULT_POLL_INTERVAL_MS);
        return StepResult::success(
            idx,
            format!("Navigation detected: {} -> {}", reference_url, current_url),
        );
    }

    let url_changed_js = format!(
        r#"(function() {{ return window.location.href !== {}; }})()"#,
        js_string_literal(reference_url),
    );
    let complete_js = r#"(function() { return document.readyState === 'complete'; })()"#;

    match poll_condition(tab, &url_changed_js, timeout_ms, DEFAULT_POLL_INTERVAL_MS) {
        Ok(()) => {
            let end_url = tab.get_url();
            let _ = poll_condition(tab, complete_js, timeout_ms, DEFAULT_POLL_INTERVAL_MS);
            StepResult::success(
                idx,
                format!("Navigation detected: {} -> {}", reference_url, end_url),
            )
        }
        Err(_) => StepResult::failure(
            idx,
            "Wait for navigation",
            format!(
                "Timed out after {}ms; URL unchanged ({})",
                timeout_ms, current_url
            ),
        ),
    }
}

fn step_wait_for_url(tab: &Tab, idx: usize, contains: &str, timeout_ms: u64) -> StepResult {
    let js = format!(
        r#"(function() {{ return window.location.href.includes({}); }})()"#,
        js_string_literal(contains),
    );
    match poll_condition(tab, &js, timeout_ms, DEFAULT_POLL_INTERVAL_MS) {
        Ok(()) => StepResult::success(idx, format!("URL contains '{}'", contains)),
        Err(e) => StepResult::failure(idx, format!("Wait for URL containing '{}'", contains), e),
    }
}

fn step_wait_for_text(tab: &Tab, idx: usize, text: &str, timeout_ms: u64) -> StepResult {
    let js = browser_locators::js_check_text_present(text);
    match poll_condition(tab, &js, timeout_ms, DEFAULT_POLL_INTERVAL_MS) {
        Ok(()) => StepResult::success(idx, format!("Text '{}' found on page", text)),
        Err(e) => StepResult::failure(idx, format!("Wait for text '{}'", text), e),
    }
}

fn step_wait_for_element_hidden(
    tab: &Tab,
    idx: usize,
    locator: &BrowserLocator,
    timeout_ms: u64,
) -> StepResult {
    let js = match (to_css_selector(locator), locator.nth) {
        (Some(css), None) => browser_locators::js_check_element_hidden(&css),
        _ => {
            let find_fragment = generate_find_fragment_js(locator);
            format!(
                r#"(function() {{
  {find_fragment}
  if (elements.length === 0) return true;
  var r = elements[0].getBoundingClientRect();
  return r.width === 0 || r.height === 0;
}})()"#
            )
        }
    };
    match poll_condition(tab, &js, timeout_ms, DEFAULT_POLL_INTERVAL_MS) {
        Ok(()) => StepResult::success(idx, "Element is hidden".to_string()),
        Err(e) => StepResult::failure(idx, "Wait for element hidden", e),
    }
}

fn step_wait_for_element_stable(
    tab: &Tab,
    idx: usize,
    locator: &BrowserLocator,
    timeout_ms: u64,
) -> StepResult {
    let find_fragment = generate_find_fragment_js(locator);
    let bbox_js = format!(
        r#"(function() {{
  {find_fragment}
  if (elements.length === 0) return JSON.stringify(null);
  var r = elements[0].getBoundingClientRect();
  return JSON.stringify({{x: r.x, y: r.y, w: r.width, h: r.height}});
}})()"#,
    );

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut prev_bbox: Option<String> = None;

    loop {
        let val = eval_js_value(tab, &bbox_js).ok();
        let bbox_str = val.and_then(|v| v.as_str().map(|s| s.to_string()));

        if let Some(ref current) = bbox_str {
            if current != "null" {
                if prev_bbox.as_ref() == Some(current) {
                    return StepResult::success(idx, "Element is stable".to_string());
                }
            }
        }
        prev_bbox = bbox_str;

        if Instant::now() >= deadline {
            return StepResult::failure(idx, "Wait for element stable", "Timed out");
        }
        std::thread::sleep(Duration::from_millis(DEFAULT_POLL_INTERVAL_MS));
    }
}

fn step_wait_seconds(idx: usize, seconds: f64) -> StepResult {
    let ms = (seconds * 1000.0) as u64;
    std::thread::sleep(Duration::from_millis(ms));
    StepResult::success(idx, format!("Waited {:.1}s", seconds))
}

const NETWORK_IDLE_WINDOW_MS: u64 = 500;

const NETWORK_INFLIGHT_TRACKER_JS: &str = r#"(function() {
  if (window.__refact_inflight_installed) return;
  window.__refact_inflight_installed = true;
  window.__refact_inflight = 0;
  var origFetch = window.fetch;
  if (typeof origFetch === 'function') {
    window.fetch = function() {
      window.__refact_inflight++;
      var p = origFetch.apply(this, arguments);
      var done = function() { window.__refact_inflight = Math.max(0, window.__refact_inflight - 1); };
      return p.then(function(r) { done(); return r; }, function(e) { done(); throw e; });
    };
  }
  var XHR = window.XMLHttpRequest;
  if (typeof XHR === 'function') {
    var origSend = XHR.prototype.send;
    XHR.prototype.send = function() {
      window.__refact_inflight++;
      var self = this;
      var done = false;
      var finish = function() {
        if (done) return;
        done = true;
        window.__refact_inflight = Math.max(0, window.__refact_inflight - 1);
      };
      self.addEventListener('loadend', finish);
      self.addEventListener('error', finish);
      self.addEventListener('abort', finish);
      self.addEventListener('timeout', finish);
      return origSend.apply(this, arguments);
    };
  }
})()"#;

fn step_wait_for_network_idle(tab: &Tab, idx: usize, timeout_ms: u64) -> StepResult {
    let _ = tab.evaluate(NETWORK_INFLIGHT_TRACKER_JS, false);

    let snapshot_js = r#"(function() {
  var inflight = window.__refact_inflight_installed ? (window.__refact_inflight | 0) : -1;
  return JSON.stringify({inflight: inflight, ready: document.readyState});
})()"#;

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let idle_window = Duration::from_millis(NETWORK_IDLE_WINDOW_MS);
    let poll = Duration::from_millis(DEFAULT_POLL_INTERVAL_MS);
    let mut idle_since: Option<Instant> = None;

    loop {
        let snapshot = eval_js_value(tab, snapshot_js).unwrap_or(serde_json::Value::Null);
        let (inflight, ready) = match snapshot.as_str() {
            Some(s) => {
                let parsed: serde_json::Value =
                    serde_json::from_str(s).unwrap_or(serde_json::Value::Null);
                let i = parsed
                    .get("inflight")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(-1);
                let r = parsed
                    .get("ready")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                (i, r)
            }
            None => (-1, String::new()),
        };

        let is_idle = inflight == 0 && ready == "complete";
        if is_idle {
            if let Some(since) = idle_since {
                if Instant::now().duration_since(since) >= idle_window {
                    return StepResult::success(
                        idx,
                        format!(
                            "Network idle (inflight=0, readyState=complete, window={}ms)",
                            NETWORK_IDLE_WINDOW_MS
                        ),
                    );
                }
            } else {
                idle_since = Some(Instant::now());
            }
        } else {
            idle_since = None;
        }

        if Instant::now() >= deadline {
            return StepResult::failure(
                idx,
                "Wait for network idle",
                format!(
                    "Timed out after {}ms (inflight={}, readyState={})",
                    timeout_ms, inflight, ready
                ),
            );
        }
        std::thread::sleep(poll);
    }
}

fn step_get_text(tab: &Tab, idx: usize, locator: &BrowserLocator) -> StepResult {
    match resolve_element(tab, locator) {
        Ok(info) => match eval_js_ok(tab, browser_locators::js_get_text()) {
            Ok(result) => {
                let text = result.get("text").and_then(|v| v.as_str()).unwrap_or("");
                StepResult::success(idx, format!("Got text from <{}>", info.tag))
                    .with_data(serde_json::json!({"text": text}))
            }
            Err(e) => StepResult::failure(idx, "Get text failed", e),
        },
        Err(e) => StepResult::failure(idx, "Get text: resolution failed", e),
    }
}

fn step_get_html(tab: &Tab, idx: usize, locator: &BrowserLocator) -> StepResult {
    match resolve_element(tab, locator) {
        Ok(info) => match eval_js_ok(tab, browser_locators::js_get_html()) {
            Ok(result) => {
                let html = result.get("html").and_then(|v| v.as_str()).unwrap_or("");
                StepResult::success(idx, format!("Got HTML from <{}>", info.tag))
                    .with_data(serde_json::json!({"html": html}))
            }
            Err(e) => StepResult::failure(idx, "Get HTML failed", e),
        },
        Err(e) => StepResult::failure(idx, "Get HTML: resolution failed", e),
    }
}

fn step_get_attribute(
    tab: &Tab,
    idx: usize,
    locator: &BrowserLocator,
    attribute: &str,
) -> StepResult {
    match resolve_element(tab, locator) {
        Ok(info) => {
            let js = browser_locators::js_get_attribute(attribute);
            match eval_js_ok(tab, &js) {
                Ok(result) => {
                    let value = result
                        .get("value")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    StepResult::success(
                        idx,
                        format!("Got attribute '{}' from <{}>", attribute, info.tag),
                    )
                    .with_data(serde_json::json!({"attribute": attribute, "value": value}))
                }
                Err(e) => StepResult::failure(idx, format!("Get attribute '{}'", attribute), e),
            }
        }
        Err(e) => StepResult::failure(idx, "Get attribute: resolution failed", e),
    }
}

fn step_extract_links(
    tab: &Tab,
    idx: usize,
    locator: Option<&BrowserLocator>,
    limit: Option<usize>,
) -> StepResult {
    if let Some(loc) = locator {
        if let Err(e) = resolve_element(tab, loc) {
            return StepResult::failure(idx, "Extract links: resolution failed", e);
        }
    } else {
        let _ = tab.evaluate("window.__refact_resolved_el = null", false);
    }
    let effective_limit = limit.unwrap_or(50).min(MAX_EXTRACT_LINKS);
    let js = browser_locators::js_extract_links(effective_limit);
    match eval_js_ok(tab, &js) {
        Ok(result) => StepResult::success(idx, "Extracted links".to_string()).with_data(result),
        Err(e) => StepResult::failure(idx, "Extract links failed", e),
    }
}

fn step_extract_table(tab: &Tab, idx: usize, locator: &BrowserLocator) -> StepResult {
    match resolve_element(tab, locator) {
        Ok(info) => match eval_js_ok(tab, browser_locators::js_extract_table()) {
            Ok(result) => StepResult::success(idx, format!("Extracted table from <{}>", info.tag))
                .with_data(result),
            Err(e) => StepResult::failure(idx, "Extract table failed", e),
        },
        Err(e) => StepResult::failure(idx, "Extract table: resolution failed", e),
    }
}

fn step_dom_snapshot(
    tab: &Tab,
    idx: usize,
    selector: &str,
    max_chars: Option<usize>,
) -> StepResult {
    let limit = max_chars.unwrap_or(5000).min(MAX_DOM_SNAPSHOT_CHARS);
    let js = format!(
        r#"(function() {{
  var el = document.querySelector({sel});
  if (!el) return JSON.stringify({{error: 'Selector not found'}});
  var full = el.outerHTML;
  var truncated = false;
  var html = full;
  if (html.length > {limit}) {{
    html = html.substring(0, {limit}) + '... (truncated)';
    truncated = true;
  }}
  return JSON.stringify({{ok: true, html: html, length: full.length, truncated: truncated, max_chars: {limit}}});
}})()"#,
        sel = js_string_literal(selector),
        limit = limit,
    );
    match eval_js_ok(tab, &js) {
        Ok(result) => {
            StepResult::success(idx, "DOM snapshot captured".to_string()).with_data(result)
        }
        Err(e) => StepResult::failure(idx, "DOM snapshot failed", e),
    }
}

fn step_accessibility_snapshot(tab: &Tab, idx: usize) -> StepResult {
    let js = format!(
        r#"(function() {{
  var MAX_NODES = {max_nodes};
  var MAX_DEPTH = {max_depth};
  var MAX_CHILDREN = {max_children};
  var nodeCount = 0;
  var truncated = false;
  function walk(el, depth) {{
    if (depth > MAX_DEPTH) return null;
    if (nodeCount >= MAX_NODES) {{ truncated = true; return null; }}
    nodeCount++;
    var role = el.getAttribute('role') || el.tagName.toLowerCase();
    var name = el.getAttribute('aria-label') || el.getAttribute('title') || '';
    if (!name && el.innerText) name = el.innerText.substring(0, 80);
    var children = [];
    for (var i = 0; i < el.children.length && children.length < MAX_CHILDREN; i++) {{
      if (nodeCount >= MAX_NODES) {{ truncated = true; break; }}
      var c = walk(el.children[i], depth + 1);
      if (c) children.push(c);
    }}
    return {{role: role, name: name.trim(), children: children}};
  }}
  if (!document.body) return JSON.stringify({{ok: false, error: 'document.body is null'}});
  var tree = walk(document.body, 0);
  return JSON.stringify({{ok: true, tree: tree, node_count: nodeCount, truncated: truncated, max_nodes: MAX_NODES}});
}})()"#,
        max_nodes = ACCESSIBILITY_MAX_NODES,
        max_depth = ACCESSIBILITY_MAX_DEPTH,
        max_children = ACCESSIBILITY_MAX_CHILDREN,
    );
    match eval_js_ok(tab, &js) {
        Ok(result) => {
            StepResult::success(idx, "Accessibility snapshot".to_string()).with_data(result)
        }
        Err(e) => StepResult::failure(idx, "Accessibility snapshot failed", e),
    }
}

fn step_screenshot(tab: &Tab, idx: usize) -> StepResult {
    match tab.call_method(Page::CaptureScreenshot {
        format: Some(Page::CaptureScreenshotFormatOption::Jpeg),
        clip: None,
        quality: Some(75),
        from_surface: Some(true),
        capture_beyond_viewport: Some(false),
        optimize_for_speed: None,
    }) {
        Ok(result) => StepResult::success(idx, "Screenshot captured".to_string()).with_data(
            serde_json::json!({
                "mime": "image/jpeg",
                "data": result.data,
            }),
        ),
        Err(e) => StepResult::failure(idx, "Screenshot failed", e.to_string()),
    }
}

fn step_screenshot_element(tab: &Tab, idx: usize, locator: &BrowserLocator) -> StepResult {
    let info = match resolve_element(tab, locator) {
        Ok(i) => i,
        Err(e) => return StepResult::failure(idx, "Screenshot element: resolution failed", e),
    };

    let bbox = match &info.bbox {
        Some(b) if b.width > 0.0 && b.height > 0.0 => b,
        _ => {
            return StepResult::failure(idx, "Screenshot element", "Element has no visible bounds")
        }
    };

    let clip = Page::Viewport {
        x: bbox.x,
        y: bbox.y,
        width: bbox.width,
        height: bbox.height,
        scale: 1.0,
    };

    match tab.call_method(Page::CaptureScreenshot {
        format: Some(Page::CaptureScreenshotFormatOption::Jpeg),
        clip: Some(clip),
        quality: Some(75),
        from_surface: Some(true),
        capture_beyond_viewport: Some(false),
        optimize_for_speed: None,
    }) {
        Ok(result) => StepResult::success(idx, format!("Element screenshot of <{}>", info.tag))
            .with_data(serde_json::json!({
                "mime": "image/jpeg",
                "data": result.data,
            })),
        Err(e) => StepResult::failure(idx, "Element screenshot failed", e.to_string()),
    }
}

fn step_eval(tab: &Tab, idx: usize, expression: &str) -> StepResult {
    match tab.evaluate(expression, false) {
        Ok(remote) => {
            let value = remote.value.unwrap_or(serde_json::Value::Null);
            let desc = remote.description.unwrap_or_default();
            StepResult::success(idx, "Eval completed".to_string())
                .with_data(serde_json::json!({"value": value, "description": desc}))
        }
        Err(e) => StepResult::failure(idx, "Eval failed", e.to_string()),
    }
}

fn step_styles(
    tab: &Tab,
    idx: usize,
    locator: &BrowserLocator,
    property_filter: Option<&str>,
) -> StepResult {
    match resolve_element(tab, locator) {
        Ok(info) => {
            let filter_js = match property_filter {
                Some(f) if !f.is_empty() => format!(
                    ".filter(function(s) {{ return s.includes({}); }})",
                    js_string_literal(f),
                ),
                _ => String::new(),
            };
            let js = format!(
                r#"(function() {{
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  var cs = window.getComputedStyle(el);
  var props = [];
  for (var i = 0; i < cs.length; i++) {{
    props.push(cs[i] + ': ' + cs.getPropertyValue(cs[i]));
  }}
  props = props{filter};
  if (props.length > 50) props = props.slice(0, 50).concat(['... (' + (props.length - 50) + ' more)']);
  return JSON.stringify({{ok: true, styles: props}});
}})()"#,
                filter = filter_js,
            );
            match eval_js_ok(tab, &js) {
                Ok(result) => StepResult::success(idx, format!("Got styles for <{}>", info.tag))
                    .with_data(result),
                Err(e) => StepResult::failure(idx, "Styles failed", e),
            }
        }
        Err(e) => StepResult::failure(idx, "Styles: resolution failed", e),
    }
}

fn step_tab_log(tab: &Tab, idx: usize) -> StepResult {
    let js = r#"(function() {
  if (!window.__refact_console_log) return JSON.stringify({ok: true, entries: []});
  return JSON.stringify({ok: true, entries: window.__refact_console_log.slice(-50)});
})()"#;
    match eval_js_ok(tab, js) {
        Ok(result) => StepResult::success(idx, "Tab log retrieved".to_string()).with_data(result),
        Err(_) => StepResult::success(
            idx,
            "Tab log: no captured logs available (use BrowserRuntime buffers for full logs)"
                .to_string(),
        ),
    }
}

fn step_dismiss_overlays(tab: &Tab, idx: usize) -> StepResult {
    match eval_js_ok(tab, browser_locators::js_dismiss_overlays()) {
        Ok(result) => {
            let count = result
                .get("dismissed")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            StepResult::success(idx, format!("Dismissed {} overlay(s)", count))
        }
        Err(e) => StepResult::failure(idx, "Dismiss overlays failed", e),
    }
}

fn step_highlight_element(tab: &Tab, idx: usize, locator: &BrowserLocator) -> StepResult {
    match resolve_element(tab, locator) {
        Ok(info) => match eval_js_ok(tab, browser_locators::js_highlight_element()) {
            Ok(_) => StepResult::success(idx, format!("Highlighted <{}>", info.tag)),
            Err(e) => StepResult::failure(idx, "Highlight failed", e),
        },
        Err(e) => StepResult::failure(idx, "Highlight: resolution failed", e),
    }
}

pub fn choose_fill_strategies(field_kind: &FieldKind) -> Vec<FillStrategy> {
    match field_kind {
        FieldKind::ContentEditable => vec![
            FillStrategy::ContentEditablePath,
            FillStrategy::NativeTyping,
        ],
        FieldKind::Textarea => vec![
            FillStrategy::DomValueSetter,
            FillStrategy::NativePrototypeSetter,
            FillStrategy::NativeTyping,
        ],
        FieldKind::Select => vec![],
        FieldKind::Checkbox | FieldKind::Radio => vec![],
        FieldKind::FileInput | FieldKind::HiddenInput => vec![],
        _ => vec![
            FillStrategy::DomValueSetter,
            FillStrategy::NativePrototypeSetter,
            FillStrategy::NativeTyping,
            FillStrategy::ClickAndType,
        ],
    }
}

fn generate_fill_js(strategy: &FillStrategy, text: &str, clear_first: bool) -> String {
    let text_lit = js_string_literal(text);

    match strategy {
        FillStrategy::DomValueSetter => {
            let clear = if clear_first { "el.value = '';" } else { "" };
            format!(
                r#"(function() {{
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  el.scrollIntoView({{block: 'center', behavior: 'instant'}});
  el.focus();
  {clear}
  el.value = {text};
  el.dispatchEvent(new Event('input', {{bubbles: true}}));
  el.dispatchEvent(new Event('change', {{bubbles: true}}));
  return JSON.stringify({{ok: true, value: el.value}});
}})()"#,
                text = text_lit,
            )
        }

        FillStrategy::NativePrototypeSetter => {
            let clear = if clear_first {
                "setter.call(el, ''); el.dispatchEvent(new Event('input', {bubbles:true}));"
            } else {
                ""
            };
            format!(
                r#"(function() {{
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  var proto = (el.tagName === 'TEXTAREA') ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  var desc = Object.getOwnPropertyDescriptor(proto, 'value');
  if (!desc || !desc.set) return JSON.stringify({{error: 'No value setter on prototype'}});
  var setter = desc.set;
  el.scrollIntoView({{block: 'center', behavior: 'instant'}});
  el.focus();
  {clear}
  setter.call(el, {text});
  el.dispatchEvent(new Event('input', {{bubbles: true}}));
  el.dispatchEvent(new Event('change', {{bubbles: true}}));
  return JSON.stringify({{ok: true, value: el.value}});
}})()"#,
                text = text_lit,
            )
        }

        FillStrategy::ContentEditablePath => {
            let clear = if clear_first {
                "document.execCommand('selectAll', false, null); document.execCommand('delete', false, null);"
            } else {
                ""
            };
            format!(
                r#"(function() {{
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  if (!el.isContentEditable) return JSON.stringify({{error: 'Element is not contentEditable'}});
  el.scrollIntoView({{block: 'center', behavior: 'instant'}});
  el.focus();
  {clear}
  document.execCommand('insertText', false, {text});
  var actual = (el.innerText || el.textContent || '').trim();
  return JSON.stringify({{ok: true, value: actual}});
}})()"#,
                text = text_lit,
            )
        }

        FillStrategy::NativeTyping => {
            let clear = if clear_first {
                "if (el.select) el.select(); document.execCommand('selectAll', false, null); document.execCommand('delete', false, null);"
            } else {
                ""
            };
            format!(
                r#"(function() {{
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  el.scrollIntoView({{block: 'center', behavior: 'instant'}});
  el.focus();
  {clear}
  document.execCommand('insertText', false, {text});
  var actual = el.value !== undefined ? el.value : (el.innerText || el.textContent || '').trim();
  return JSON.stringify({{ok: true, value: actual}});
}})()"#,
                text = text_lit,
            )
        }

        FillStrategy::ClickAndType => {
            let clear = if clear_first {
                r#"if (el.select) { el.select(); } else { document.execCommand('selectAll', false, null); }
  document.execCommand('delete', false, null);"#
            } else {
                ""
            };
            format!(
                r#"(function() {{
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  el.scrollIntoView({{block: 'center', behavior: 'instant'}});
  el.click();
  el.focus();
  {clear}
  var text = {text};
  for (var i = 0; i < text.length; i++) {{
    var ch = text[i];
    el.dispatchEvent(new KeyboardEvent('keydown', {{key: ch, bubbles: true}}));
    el.dispatchEvent(new KeyboardEvent('keypress', {{key: ch, bubbles: true}}));
    document.execCommand('insertText', false, ch);
    el.dispatchEvent(new KeyboardEvent('keyup', {{key: ch, bubbles: true}}));
  }}
  el.dispatchEvent(new Event('input', {{bubbles: true}}));
  el.dispatchEvent(new Event('change', {{bubbles: true}}));
  var actual = el.value !== undefined ? el.value : (el.innerText || el.textContent || '').trim();
  return JSON.stringify({{ok: true, value: actual}});
}})()"#,
                text = text_lit,
            )
        }
    }
}

fn generate_clear_js(field_kind: &FieldKind) -> String {
    match field_kind {
        FieldKind::ContentEditable => r#"(function() {
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.focus();
  document.execCommand('selectAll', false, null);
  document.execCommand('delete', false, null);
  return JSON.stringify({ok: true, value: (el.innerText || '').trim()});
})()"#
            .to_string(),
        _ => r#"(function() {
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({error: 'No resolved element'});
  el.focus();
  if (el.select) el.select();
  el.value = '';
  el.dispatchEvent(new Event('input', {bubbles: true}));
  el.dispatchEvent(new Event('change', {bubbles: true}));
  return JSON.stringify({ok: true, value: el.value});
})()"#
            .to_string(),
    }
}

fn verify_field_value(tab: &Tab, expected: &str, field_kind: &FieldKind) -> Result<bool, String> {
    let js = match field_kind {
        FieldKind::ContentEditable => format!(
            r#"(function() {{
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  var actual = (el.innerText || el.textContent || '').trim();
  return JSON.stringify({{actual: actual}});
}})()"#,
        ),
        FieldKind::PasswordInput => {
            format!(
                r#"(function() {{
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  return JSON.stringify({{actual_length: (el.value || '').length, expected_length: {len}}});
}})()"#,
                len = expected.len(),
            )
        }
        _ => format!(
            r#"(function() {{
  var el = window.__refact_resolved_el;
  if (!el) return JSON.stringify({{error: 'No resolved element'}});
  return JSON.stringify({{actual: el.value !== undefined ? String(el.value) : ''}});
}})()"#,
        ),
    };

    let result = eval_js_json(tab, &js)?;
    if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
        return Err(err.to_string());
    }

    match field_kind {
        FieldKind::PasswordInput => {
            let actual_len = result
                .get("actual_length")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Ok(actual_len == expected.len() as u64)
        }
        _ => {
            let actual = result.get("actual").and_then(|v| v.as_str()).unwrap_or("");
            Ok(actual == expected)
        }
    }
}

pub fn describe_locator(locator: &BrowserLocator) -> String {
    match &locator.strategy {
        LocatorStrategy::Css { value } => format!("css={}", value),
        LocatorStrategy::Id { value } => format!("id={}", value),
        LocatorStrategy::Name { value } => format!("name={}", value),
        LocatorStrategy::TestId { value } => format!("testid={}", value),
        LocatorStrategy::Placeholder { value } => format!("placeholder={}", value),
        LocatorStrategy::Autocomplete { value } => format!("autocomplete={}", value),
        LocatorStrategy::Text { value, exact } => {
            if *exact {
                format!("text=\"{}\"", value)
            } else {
                format!("text~\"{}\"", value)
            }
        }
        LocatorStrategy::Label { value } => format!("label={}", value),
        LocatorStrategy::Role { role, name } => match name {
            Some(n) => format!("role={}[{}]", role, n),
            None => format!("role={}", role),
        },
        LocatorStrategy::Xpath { value } => format!("xpath={}", value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strategies_text_input() {
        let strategies = choose_fill_strategies(&FieldKind::TextInput);
        assert_eq!(strategies.len(), 4);
        assert_eq!(strategies[0], FillStrategy::DomValueSetter);
        assert_eq!(strategies[1], FillStrategy::NativePrototypeSetter);
        assert_eq!(strategies[2], FillStrategy::NativeTyping);
        assert_eq!(strategies[3], FillStrategy::ClickAndType);
    }

    #[test]
    fn test_strategies_password_input() {
        let strategies = choose_fill_strategies(&FieldKind::PasswordInput);
        assert!(!strategies.is_empty());
        assert_eq!(strategies[0], FillStrategy::DomValueSetter);
    }

    #[test]
    fn test_strategies_textarea() {
        let strategies = choose_fill_strategies(&FieldKind::Textarea);
        assert_eq!(strategies.len(), 3);
        assert_eq!(strategies[0], FillStrategy::DomValueSetter);
    }

    #[test]
    fn test_strategies_content_editable() {
        let strategies = choose_fill_strategies(&FieldKind::ContentEditable);
        assert_eq!(strategies.len(), 2);
        assert_eq!(strategies[0], FillStrategy::ContentEditablePath);
        assert_eq!(strategies[1], FillStrategy::NativeTyping);
    }

    #[test]
    fn test_strategies_select_is_empty() {
        assert!(choose_fill_strategies(&FieldKind::Select).is_empty());
    }

    #[test]
    fn test_strategies_checkbox_is_empty() {
        assert!(choose_fill_strategies(&FieldKind::Checkbox).is_empty());
    }

    #[test]
    fn test_strategies_radio_is_empty() {
        assert!(choose_fill_strategies(&FieldKind::Radio).is_empty());
    }

    #[test]
    fn test_strategies_file_input_is_empty() {
        assert!(choose_fill_strategies(&FieldKind::FileInput).is_empty());
    }

    #[test]
    fn test_strategies_email_input() {
        let strategies = choose_fill_strategies(&FieldKind::EmailInput);
        assert_eq!(strategies.len(), 4);
    }

    #[test]
    fn test_strategies_search_input() {
        let strategies = choose_fill_strategies(&FieldKind::SearchInput);
        assert_eq!(strategies.len(), 4);
    }

    #[test]
    fn test_generate_fill_dom_setter_contains_value_assignment() {
        let js = generate_fill_js(&FillStrategy::DomValueSetter, "hello", true);
        assert!(js.contains("el.value ="));
        assert!(js.contains("'hello'"));
        assert!(js.contains("el.value = '';"));
        assert!(js.contains("dispatchEvent"));
    }

    #[test]
    fn test_generate_fill_dom_setter_no_clear() {
        let js = generate_fill_js(&FillStrategy::DomValueSetter, "test", false);
        assert!(!js.contains("el.value = '';"));
        assert!(js.contains("el.value ="));
    }

    #[test]
    fn test_generate_fill_prototype_setter() {
        let js = generate_fill_js(&FillStrategy::NativePrototypeSetter, "world", true);
        assert!(js.contains("getOwnPropertyDescriptor"));
        assert!(js.contains("HTMLInputElement.prototype"));
        assert!(js.contains("setter.call"));
        assert!(js.contains("'world'"));
    }

    #[test]
    fn test_generate_fill_contenteditable() {
        let js = generate_fill_js(&FillStrategy::ContentEditablePath, "rich text", true);
        assert!(js.contains("isContentEditable"));
        assert!(js.contains("insertText"));
        assert!(js.contains("selectAll"));
        assert!(js.contains("'rich text'"));
    }

    #[test]
    fn test_generate_fill_native_typing() {
        let js = generate_fill_js(&FillStrategy::NativeTyping, "typed", true);
        assert!(js.contains("insertText"));
        assert!(js.contains("'typed'"));
    }

    #[test]
    fn test_generate_fill_click_and_type() {
        let js = generate_fill_js(&FillStrategy::ClickAndType, "slow", true);
        assert!(js.contains("el.click()"));
        assert!(js.contains("KeyboardEvent"));
        assert!(js.contains("'slow'"));
    }

    #[test]
    fn test_generate_fill_escapes_special_chars() {
        let js = generate_fill_js(&FillStrategy::DomValueSetter, "it's \"quoted\"", false);
        assert!(js.contains("it\\'s"));
    }

    #[test]
    fn test_generate_clear_input() {
        let js = generate_clear_js(&FieldKind::TextInput);
        assert!(js.contains("el.value = ''"));
        assert!(js.contains("dispatchEvent"));
    }

    #[test]
    fn test_generate_clear_contenteditable() {
        let js = generate_clear_js(&FieldKind::ContentEditable);
        assert!(js.contains("selectAll"));
        assert!(js.contains("delete"));
        assert!(js.contains("innerText"));
    }

    #[test]
    fn test_describe_locator_css() {
        let loc = BrowserLocator::css("#btn");
        assert_eq!(describe_locator(&loc), "css=#btn");
    }

    #[test]
    fn test_describe_locator_id() {
        let loc = BrowserLocator::id("email");
        assert_eq!(describe_locator(&loc), "id=email");
    }

    #[test]
    fn test_describe_locator_name() {
        let loc = BrowserLocator::name("q");
        assert_eq!(describe_locator(&loc), "name=q");
    }

    #[test]
    fn test_describe_locator_label() {
        let loc = BrowserLocator::label("Email Address");
        assert_eq!(describe_locator(&loc), "label=Email Address");
    }

    #[test]
    fn test_describe_locator_role_with_name() {
        let loc = BrowserLocator::role("textbox", Some("Search"));
        assert_eq!(describe_locator(&loc), "role=textbox[Search]");
    }

    #[test]
    fn test_describe_locator_role_without_name() {
        let loc = BrowserLocator::role("button", None);
        assert_eq!(describe_locator(&loc), "role=button");
    }

    #[test]
    fn test_describe_locator_text_exact() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Text {
                value: "Submit".to_string(),
                exact: true,
            },
            nth: None,
            within: None,
        };
        assert_eq!(describe_locator(&loc), "text=\"Submit\"");
    }

    #[test]
    fn test_describe_locator_text_substring() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Text {
                value: "Sub".to_string(),
                exact: false,
            },
            nth: None,
            within: None,
        };
        assert_eq!(describe_locator(&loc), "text~\"Sub\"");
    }

    #[test]
    fn test_describe_locator_placeholder() {
        let loc = BrowserLocator::placeholder("Search...");
        assert_eq!(describe_locator(&loc), "placeholder=Search...");
    }

    #[test]
    fn test_describe_locator_testid() {
        let loc = BrowserLocator::test_id("submit-btn");
        assert_eq!(describe_locator(&loc), "testid=submit-btn");
    }

    #[test]
    fn test_describe_locator_xpath() {
        let loc = BrowserLocator {
            strategy: LocatorStrategy::Xpath {
                value: "//button".to_string(),
            },
            nth: None,
            within: None,
        };
        assert_eq!(describe_locator(&loc), "xpath=//button");
    }

    #[test]
    fn test_fill_js_all_strategies_produce_iife() {
        let strategies = vec![
            FillStrategy::DomValueSetter,
            FillStrategy::NativePrototypeSetter,
            FillStrategy::ContentEditablePath,
            FillStrategy::NativeTyping,
            FillStrategy::ClickAndType,
        ];
        for s in strategies {
            let js = generate_fill_js(&s, "test", true);
            assert!(
                js.starts_with("(function()"),
                "Strategy {:?} should be IIFE",
                s
            );
            assert!(
                js.ends_with("})()"),
                "Strategy {:?} should end with {{}})()",
                s
            );
            assert!(
                js.contains("JSON.stringify"),
                "Strategy {:?} should return JSON",
                s,
            );
        }
    }

    #[test]
    fn test_clear_js_produces_iife() {
        for kind in &[FieldKind::TextInput, FieldKind::ContentEditable] {
            let js = generate_clear_js(kind);
            assert!(js.starts_with("(function()"));
            assert!(js.contains("JSON.stringify"));
        }
    }

    #[test]
    fn test_select_option_js_contains_option_search() {
        let value = "Option A";
        let js_val = js_string_literal(value);
        assert_eq!(js_val, "'Option A'");
    }

    #[test]
    fn test_all_text_like_inputs_have_strategies() {
        let text_kinds = vec![
            FieldKind::TextInput,
            FieldKind::PasswordInput,
            FieldKind::EmailInput,
            FieldKind::SearchInput,
            FieldKind::NumberInput,
            FieldKind::TelInput,
            FieldKind::UrlInput,
        ];
        for kind in text_kinds {
            let strategies = choose_fill_strategies(&kind);
            assert!(
                !strategies.is_empty(),
                "FieldKind {:?} should have fill strategies",
                kind,
            );
        }
    }

    #[test]
    fn test_unfillable_kinds_have_no_strategies() {
        let unfillable = vec![
            FieldKind::Select,
            FieldKind::Checkbox,
            FieldKind::Radio,
            FieldKind::FileInput,
            FieldKind::HiddenInput,
        ];
        for kind in unfillable {
            let strategies = choose_fill_strategies(&kind);
            assert!(
                strategies.is_empty(),
                "FieldKind {:?} should have no fill strategies",
                kind,
            );
        }
    }
}
