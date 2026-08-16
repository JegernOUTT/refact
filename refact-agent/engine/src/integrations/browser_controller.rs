use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use headless_chrome::Tab;
use headless_chrome::protocol::cdp::Page;
use tokio::sync::Mutex as AMutex;

use crate::integrations::browser_locators::{
    self, generate_find_fragment_js, js_string_literal, parse_element_info, to_css_selector,
    INSPECT_ELEMENT_JS,
};
use crate::integrations::browser_models::*;
use crate::integrations::browser_runtime::BrowserRuntime;
use refact_browser::{
    CdpKeyboardDispatcher, ElementHandle, Keyboard, LocatorHandler, LocatorHandlerLease,
    LocatorHandlerOperation, LocatorHandlerProbe, LocatorHandlerRegistry, WorldManager,
    DEFAULT_DISMISS_OVERLAYS_HANDLER,
};
use refact_core::image_policy::{resize_to_policy, ImageFormat, ImagePolicy};

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
const REPORT_STABILIZATION_TIMEOUT_MS: u64 = 3_000;
const REPORT_STABILITY_INTERVAL_MS: u64 = 200;

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

struct ResolvedElement {
    handle: ElementHandle,
    info: ElementInfo,
}

impl std::ops::Deref for ResolvedElement {
    type Target = ElementInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

fn resolve_element(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
) -> Result<ResolvedElement, String> {
    let locator = serde_json::to_value(locator)
        .map_err(|error| format!("Failed to serialize browser locator: {error}"))?;
    let mut handles = world
        .call_injected_handles(tab, "resolveAll", serde_json::json!([locator]))
        .map_err(|error| error.to_string())?;
    if handles.is_empty() {
        return Err("Element not found".to_string());
    }
    if handles.len() > 1 {
        let count = handles.len();
        for handle in &handles {
            let _ = world.release_handle(tab, handle);
        }
        return Err(format!(
            "Strict mode violation: locator resolved to {count} elements"
        ));
    }
    let handle = handles.remove(0);
    let inspect = format!(
        "function() {{ {INSPECT_ELEMENT_JS} return JSON.stringify(__refact_inspect_element(this, 1)); }}"
    );
    let val = world
        .call_function_on(tab, &handle, &inspect, Vec::new())
        .map_err(|error| error.to_string())?;
    let json_str = match val.as_str() {
        Some(s) => s.to_string(),
        None => serde_json::to_string(&val)
            .map_err(|e| format!("Failed to serialize resolve result: {}", e))?,
    };
    let info = parse_element_info(&json_str)?;
    Ok(ResolvedElement { handle, info })
}

fn resolve_interactable(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
) -> Result<ResolvedElement, String> {
    let resolved = resolve_element(tab, world, locator)?;
    if !resolved.info.visible {
        let _ = world.release_handle(tab, &resolved.handle);
        return Err("Element is not visible".to_string());
    }
    if !resolved.info.enabled {
        let _ = world.release_handle(tab, &resolved.handle);
        return Err("Element is disabled".to_string());
    }
    Ok(resolved)
}

fn call_handle_json(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    action_js: &str,
) -> Result<serde_json::Value, String> {
    let function = format!("function() {{ return ({action_js}).call(this); }}");
    let value = world
        .call_function_on(tab, handle, &function, Vec::new())
        .map_err(|error| error.to_string())?;
    let result = match value.as_str() {
        Some(json) => serde_json::from_str(json)
            .map_err(|error| format!("Failed to parse JS JSON result: {error}"))?,
        None if value.is_object() || value.is_array() => value,
        None => return Err(format!("Unexpected JS result type: {value:?}")),
    };
    if let Some(error) = result.get("error").and_then(|value| value.as_str()) {
        return Err(error.to_string());
    }
    Ok(result)
}

pub fn execute_steps(
    tab: &Tab,
    steps: &[BrowserStep],
    image_policy: &ImagePolicy,
) -> ExecutionReport {
    execute_steps_with_world(tab, steps, image_policy, &WorldManager::default())
}

fn execute_steps_with_world(
    tab: &Tab,
    steps: &[BrowserStep],
    image_policy: &ImagePolicy,
    world: &WorldManager,
) -> ExecutionReport {
    let _ = tab.evaluate(INSPECT_ELEMENT_JS, false);

    let mut results = Vec::new();
    let handlers = Arc::new(Mutex::new(LocatorHandlerRegistry::default()));
    let mut locator_handlers = Vec::new();
    let mut all_ok = true;
    let mut pre_step_url: Option<String> = Some(tab.get_url());

    for (idx, step) in steps.iter().enumerate() {
        let result = execute_single_step(
            tab,
            world,
            step,
            idx,
            pre_step_url.as_deref(),
            image_policy,
            Some(&handlers),
            &mut locator_handlers,
        );
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

    let _ = world.release_all(tab);

    ExecutionReport {
        ok: all_ok,
        steps: results,
        url: Some(tab.get_url()),
        title: tab.get_title().ok(),
        stabilized: false,
        console: vec![],
        page_errors: vec![],
        locator_handlers,
        dialogs: vec![],
        screenshot: None,
    }
}

pub fn is_tab_management_step(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::OpenTab { .. }
            | BrowserStep::CloseTab
            | BrowserStep::SwitchTab { .. }
            | BrowserStep::ListTabs
            | BrowserStep::HandleDialog { .. }
    )
}

pub fn execute_step(
    tab: &Tab,
    step: &BrowserStep,
    idx: usize,
    image_policy: &ImagePolicy,
) -> StepResult {
    execute_step_with_world(tab, &WorldManager::default(), step, idx, image_policy)
}

fn execute_step_with_world(
    tab: &Tab,
    world: &WorldManager,
    step: &BrowserStep,
    idx: usize,
    image_policy: &ImagePolicy,
) -> StepResult {
    let _ = tab.evaluate(INSPECT_ELEMENT_JS, false);
    let handlers = Arc::new(Mutex::new(LocatorHandlerRegistry::default()));
    let result = execute_single_step(
        tab,
        world,
        step,
        idx,
        None,
        image_policy,
        Some(&handlers),
        &mut Vec::new(),
    );
    if result.ok && is_navigation_step(step) {
        let _ = tab.evaluate(INSPECT_ELEMENT_JS, false);
    }
    let _ = world.release_all(tab);
    result
}

pub async fn execute_request_with_runtime(
    runtime_arc: Arc<AMutex<BrowserRuntime>>,
    request: BrowserActionRequest,
    image_policy: &ImagePolicy,
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

    let initial_url = {
        let rt = runtime_arc.lock().await;
        rt.get_active_tab().map(|tab| tab.get_url())
    };
    let mut current_tab = {
        let rt = runtime_arc.lock().await;
        rt.get_active_tab()
    };
    let world = {
        let rt = runtime_arc.lock().await;
        rt.world_manager.clone()
    };
    let handlers = {
        let rt = runtime_arc.lock().await;
        rt.locator_handlers.clone()
    };
    let mut results = Vec::new();
    let mut locator_handlers = Vec::new();
    let mut all_ok = true;

    for (idx, step) in request.steps.iter().enumerate() {
        if let Some(tab) = &current_tab {
            let _ = tab.evaluate(NETWORK_INFLIGHT_TRACKER_JS, false);
        }
        let mut result = if is_tab_management_step(step) {
            let step_report = tokio::task::block_in_place(|| {
                let mut rt = runtime_arc.blocking_lock();
                execute_steps_with_runtime(&mut rt, std::slice::from_ref(step), image_policy)
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
                Some(tab) => tokio::task::block_in_place(|| {
                    execute_single_step(
                        tab,
                        &world,
                        step,
                        idx,
                        None,
                        image_policy,
                        Some(&handlers),
                        &mut locator_handlers,
                    )
                }),
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

    let active_tab = match current_tab {
        Some(tab) => Some(tab),
        None => runtime_arc.lock().await.get_active_tab(),
    };
    let (url, title, stabilized, screenshot) = if let Some(tab) = active_tab {
        let stabilized = tokio::task::block_in_place(|| {
            wait_for_report_stability(&tab, &world, REPORT_STABILIZATION_TIMEOUT_MS)
        });
        let url = tab.get_url();
        let page_changed = initial_url.as_deref() != Some(url.as_str());
        let capture_requested = request.attach_screenshot
            || page_changed
            || request
                .steps
                .iter()
                .any(|step| matches!(step, BrowserStep::Screenshot));
        let screenshot = if capture_requested {
            tokio::task::block_in_place(|| capture_report_screenshot(&tab, image_policy).ok())
        } else {
            None
        };
        (Some(url), tab.get_title().ok(), stabilized, screenshot)
    } else {
        (None, None, false, None)
    };
    let (console, page_errors, dialogs) = {
        let mut rt = runtime_arc.lock().await;
        rt.drain_raw_events();
        let mut console = rt
            .flush_report_console()
            .into_iter()
            .map(mask_console_entry)
            .collect::<Vec<_>>();
        let page_errors = console
            .iter()
            .filter(|entry| entry.level == "page_error")
            .map(|entry| entry.text.clone())
            .collect();
        console.retain(|entry| entry.level != "page_error");
        let dialogs = rt.dialog_manager.take_reports();
        (console, page_errors, dialogs)
    };

    Ok(ExecutionReport {
        ok: all_ok,
        steps: results,
        url,
        title,
        stabilized,
        console,
        page_errors,
        locator_handlers,
        dialogs,
        screenshot,
    })
}

pub fn execute_steps_with_runtime(
    runtime: &mut BrowserRuntime,
    steps: &[BrowserStep],
    image_policy: &ImagePolicy,
) -> ExecutionReport {
    let mut current_tab: Option<Arc<Tab>> = runtime.get_active_tab();
    if let Some(ref tab) = current_tab {
        let _ = tab.evaluate(INSPECT_ELEMENT_JS, false);
    }

    let mut results = Vec::new();
    let handlers = runtime.locator_handlers.clone();
    let mut locator_handlers = Vec::new();
    let mut all_ok = true;
    let mut pre_step_url: Option<String> = current_tab.as_ref().map(|t| t.get_url());

    for (idx, step) in steps.iter().enumerate() {
        let result = match step {
            BrowserStep::OpenTab { device } => match runtime.browser.new_tab() {
                Ok(new_tab) => {
                    let device_label = device.as_deref().unwrap_or("desktop");
                    let target_id = new_tab.get_target_id().to_string();
                    let (w, h, dpr, mobile) = match device.as_deref() {
                        Some("mobile") => (390, 844, 3.0, true),
                        Some("tablet") => (834, 1112, 2.0, true),
                        _ => (1440, 900, 2.0, false),
                    };
                    let _ = new_tab.call_method(
                        headless_chrome::protocol::cdp::Emulation::SetDeviceMetricsOverride {
                            width: w,
                            height: h,
                            device_scale_factor: dpr,
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
            BrowserStep::HandleDialog {
                accept,
                prompt_text,
            } => match runtime.dialog_manager.arm(*accept, prompt_text.clone()) {
                Ok(()) => StepResult::success(
                    idx,
                    if *accept {
                        "Armed acceptance for the next dialog"
                    } else {
                        "Armed dismissal for the next dialog"
                    },
                ),
                Err(error) => StepResult::failure(idx, "HandleDialog", error),
            },
            other => match &current_tab {
                Some(tab) => execute_single_step(
                    tab,
                    &runtime.world_manager,
                    other,
                    idx,
                    pre_step_url.as_deref(),
                    image_policy,
                    Some(&handlers),
                    &mut locator_handlers,
                ),
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
    let dialogs = runtime.dialog_manager.take_reports();
    ExecutionReport {
        ok: all_ok,
        steps: results,
        url,
        title,
        stabilized: false,
        console: vec![],
        page_errors: vec![],
        locator_handlers,
        dialogs,
        screenshot: None,
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

fn needs_locator_handler_checkpoint(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::Click { .. }
            | BrowserStep::ClickIfExists { .. }
            | BrowserStep::Hover { .. }
            | BrowserStep::Focus { .. }
            | BrowserStep::Blur { .. }
            | BrowserStep::ScrollTo { .. }
            | BrowserStep::Fill { .. }
            | BrowserStep::Clear { .. }
            | BrowserStep::SelectOption { .. }
            | BrowserStep::Check { .. }
            | BrowserStep::Uncheck { .. }
            | BrowserStep::WaitForSelector { .. }
            | BrowserStep::WaitForText { .. }
            | BrowserStep::WaitForElementHidden { .. }
            | BrowserStep::WaitForElementStable { .. }
            | BrowserStep::GetText { .. }
            | BrowserStep::GetHtml { .. }
            | BrowserStep::GetAttribute { .. }
            | BrowserStep::ScreenshotElement { .. }
            | BrowserStep::Styles { .. }
            | BrowserStep::HighlightElement { .. }
    )
}

fn perform_action_prechecks(
    tab: &Tab,
    world: &WorldManager,
    handlers: &Arc<Mutex<LocatorHandlerRegistry>>,
    firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
    deadline: Instant,
) -> Result<(), String> {
    wait_for_pending_navigation(tab, deadline)?;
    perform_locator_handlers_checkpoint(tab, world, handlers, firings, image_policy, deadline)?;
    wait_for_pending_navigation(tab, deadline)
}

fn wait_for_pending_navigation(tab: &Tab, deadline: Instant) -> Result<(), String> {
    loop {
        let loading = eval_js_value(tab, "document.readyState === 'loading'")
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if !loading {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("Timed out waiting for navigation before action".to_string());
        }
        std::thread::sleep(
            Duration::from_millis(DEFAULT_POLL_INTERVAL_MS)
                .min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn perform_locator_handlers_checkpoint(
    tab: &Tab,
    world: &WorldManager,
    handlers: &Arc<Mutex<LocatorHandlerRegistry>>,
    firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
    deadline: Instant,
) -> Result<(), String> {
    let handler_names = {
        let registry = handlers
            .lock()
            .map_err(|error| format!("Failed to lock locator handlers: {error}"))?;
        if registry.is_running() {
            return Ok(());
        }
        registry
            .handlers()
            .iter()
            .map(|handler| handler.name.clone())
            .collect::<Vec<_>>()
    };

    for name in handler_names {
        if Instant::now() >= deadline {
            return Err("Timed out while running locator handlers".to_string());
        }
        let handler = {
            handlers
                .lock()
                .map_err(|error| format!("Failed to lock locator handlers: {error}"))?
                .get(&name)
        };
        let Some(handler) = handler else {
            continue;
        };
        match probe_locator_handler(tab, world, &handler)? {
            LocatorHandlerProbe::Hidden | LocatorHandlerProbe::MultipleMatches { .. } => continue,
            LocatorHandlerProbe::Visible => {}
        }
        let lease = {
            handlers
                .lock()
                .map_err(|error| format!("Failed to lock locator handlers: {error}"))?
                .begin(&name)
        };
        let Some(lease) = lease else {
            continue;
        };
        let result = execute_locator_handler(
            tab,
            world,
            handlers,
            firings,
            image_policy,
            deadline,
            &lease,
        );
        let (ok, outcome) = match &result {
            Ok(outcome) => (true, outcome.clone()),
            Err(error) => (false, error.clone()),
        };
        let firing = handlers
            .lock()
            .map_err(|error| format!("Failed to lock locator handlers: {error}"))?
            .finish(lease, ok, outcome);
        firings.push(firing);
        result?;
    }
    Ok(())
}

fn probe_locator_handler(
    tab: &Tab,
    world: &WorldManager,
    handler: &LocatorHandler,
) -> Result<LocatorHandlerProbe, String> {
    if matches!(handler.operation, LocatorHandlerOperation::DismissOverlays) {
        return eval_js_ok(tab, browser_locators::js_dismiss_overlays_probe()).map(|result| {
            if result
                .get("dismissable")
                .and_then(|value| value.as_u64())
                .unwrap_or(0)
                == 0
            {
                LocatorHandlerProbe::Hidden
            } else {
                LocatorHandlerProbe::Visible
            }
        });
    }
    let locator = serde_json::to_value(&handler.locator)
        .map_err(|error| format!("Failed to serialize locator handler: {error}"))?;
    let handles = world
        .call_injected_handles(tab, "resolveAll", serde_json::json!([locator]))
        .map_err(|error| error.to_string())?;
    if handles.is_empty() {
        return Ok(LocatorHandlerProbe::Hidden);
    }
    if handles.len() > 1 {
        let count = handles.len();
        for handle in &handles {
            let _ = world.release_handle(tab, handle);
        }
        return Ok(LocatorHandlerProbe::MultipleMatches { count });
    }
    let handle = &handles[0];
    let inspect = format!(
        "function() {{ {INSPECT_ELEMENT_JS} return JSON.stringify(__refact_inspect_element(this, 1)); }}"
    );
    let value = world
        .call_function_on(tab, handle, &inspect, Vec::new())
        .map_err(|error| error.to_string());
    let _ = world.release_handle(tab, handle);
    let value = value?;
    let json = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string());
    let info = parse_element_info(&json)?;
    Ok(if info.visible {
        LocatorHandlerProbe::Visible
    } else {
        LocatorHandlerProbe::Hidden
    })
}

fn execute_locator_handler(
    tab: &Tab,
    world: &WorldManager,
    handlers: &Arc<Mutex<LocatorHandlerRegistry>>,
    firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
    deadline: Instant,
    lease: &LocatorHandlerLease,
) -> Result<String, String> {
    let outcome = match &lease.handler.operation {
        LocatorHandlerOperation::DismissOverlays => dismiss_overlays(tab),
        LocatorHandlerOperation::Action(LocatorHandlerAction::Click) => {
            let resolved = resolve_element(tab, world, &lease.handler.locator)?;
            call_handle_json(
                tab,
                world,
                &resolved.handle,
                "function() { this.click(); return JSON.stringify({ok: true}); }",
            )?;
            Ok(format!("Clicked <{}>", resolved.tag))
        }
        LocatorHandlerOperation::Action(LocatorHandlerAction::Steps { steps }) => {
            let mut summaries = Vec::new();
            for step in steps {
                if Instant::now() >= deadline {
                    return Err("Timed out while running locator handler steps".to_string());
                }
                let bounded_step = bound_handler_step(step, deadline);
                let result = execute_single_step(
                    tab,
                    world,
                    &bounded_step,
                    0,
                    None,
                    image_policy,
                    Some(handlers),
                    firings,
                );
                if !result.ok {
                    return Err(result.error.unwrap_or(result.summary));
                }
                summaries.push(result.summary);
            }
            Ok(summaries.join("; "))
        }
    }?;

    if !lease.handler.no_wait_after {
        wait_for_handler_hidden(tab, world, &lease.handler, deadline)?;
    }
    Ok(outcome)
}

fn bound_handler_step(step: &BrowserStep, deadline: Instant) -> BrowserStep {
    let remaining_ms = deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .min(u64::MAX as u128) as u64;
    match step {
        BrowserStep::WaitForSelector { locator, .. } => BrowserStep::WaitForSelector {
            locator: locator.clone(),
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitForNavigation { .. } => BrowserStep::WaitForNavigation {
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitForUrl { contains, .. } => BrowserStep::WaitForUrl {
            contains: contains.clone(),
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitForText { text, .. } => BrowserStep::WaitForText {
            text: text.clone(),
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitForNetworkIdle { .. } => BrowserStep::WaitForNetworkIdle {
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitForElementHidden { locator, .. } => BrowserStep::WaitForElementHidden {
            locator: locator.clone(),
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitForElementStable { locator, .. } => BrowserStep::WaitForElementStable {
            locator: locator.clone(),
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitSeconds { seconds } => BrowserStep::WaitSeconds {
            seconds: seconds.min(remaining_ms as f64 / 1_000.0),
        },
        _ => step.clone(),
    }
}

fn wait_for_handler_hidden(
    tab: &Tab,
    world: &WorldManager,
    handler: &LocatorHandler,
    deadline: Instant,
) -> Result<(), String> {
    loop {
        if matches!(
            probe_locator_handler(tab, world, handler)?,
            LocatorHandlerProbe::Hidden
        ) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Timed out waiting for locator handler '{}' to become hidden",
                handler.name
            ));
        }
        std::thread::sleep(
            Duration::from_millis(DEFAULT_POLL_INTERVAL_MS)
                .min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn execute_single_step(
    tab: &Tab,
    world: &WorldManager,
    step: &BrowserStep,
    idx: usize,
    pre_step_url: Option<&str>,
    image_policy: &ImagePolicy,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &mut Vec<LocatorHandlerFiring>,
) -> StepResult {
    if needs_locator_handler_checkpoint(step) {
        if let Some(handlers) = handlers {
            if let Err(error) = perform_action_prechecks(
                tab,
                world,
                handlers,
                locator_handler_firings,
                image_policy,
                Instant::now() + Duration::from_millis(DEFAULT_WAIT_TIMEOUT_MS),
            ) {
                return StepResult::failure(idx, "Locator handler checkpoint failed", error);
            }
        }
    }
    match step {
        BrowserStep::Navigate { url } => step_navigate(tab, idx, url),
        BrowserStep::Reload => step_nav_js(tab, idx, "location.reload()", "Reloaded page"),
        BrowserStep::GoBack => step_nav_js(tab, idx, "history.back()", "Navigated back"),
        BrowserStep::GoForward => step_nav_js(tab, idx, "history.forward()", "Navigated forward"),

        BrowserStep::OpenTab { .. }
        | BrowserStep::CloseTab
        | BrowserStep::SwitchTab { .. }
        | BrowserStep::ListTabs
        | BrowserStep::HandleDialog { .. } => StepResult::failure(
            idx,
            "Runtime management step",
            "Use execute_steps_with_runtime() for runtime management",
        ),

        BrowserStep::Click { locator } => step_locator_action(tab, world, idx, locator, "click"),
        BrowserStep::ClickIfExists { locator } => step_click_if_exists(tab, world, idx, locator),
        BrowserStep::Hover { locator } => step_locator_action(tab, world, idx, locator, "hover"),
        BrowserStep::Focus { locator } => step_locator_action(tab, world, idx, locator, "focus"),
        BrowserStep::Blur { locator } => step_locator_action(tab, world, idx, locator, "blur"),
        BrowserStep::ScrollTo { locator } => {
            step_locator_action(tab, world, idx, locator, "scroll_to")
        }
        BrowserStep::PressKey { key, modifiers } => step_press_key(tab, idx, key, modifiers),

        BrowserStep::Fill {
            locator,
            text,
            clear_first,
            verify,
        } => step_fill(tab, world, idx, locator, text, *clear_first, *verify),
        BrowserStep::Clear { locator, verify } => step_clear(tab, world, idx, locator, *verify),
        BrowserStep::SelectOption { locator, value } => {
            step_select_option(tab, world, idx, locator, value)
        }
        BrowserStep::Check { locator } => step_check_uncheck(tab, world, idx, locator, true),
        BrowserStep::Uncheck { locator } => step_check_uncheck(tab, world, idx, locator, false),

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

        BrowserStep::GetText { locator } => step_get_text(tab, world, idx, locator),
        BrowserStep::GetHtml { locator } => step_get_html(tab, world, idx, locator),
        BrowserStep::GetAttribute { locator, attribute } => {
            step_get_attribute(tab, world, idx, locator, attribute)
        }
        BrowserStep::ExtractLinks { locator, limit } => {
            step_extract_links(tab, world, idx, locator.as_ref(), *limit)
        }
        BrowserStep::ExtractTable { locator } => step_extract_table(tab, world, idx, locator),
        BrowserStep::DomSnapshot {
            selector,
            max_chars,
        } => step_dom_snapshot(tab, idx, selector, *max_chars),
        BrowserStep::AccessibilitySnapshot => step_accessibility_snapshot(tab, idx),
        BrowserStep::Screenshot => step_screenshot(tab, idx, image_policy),
        BrowserStep::ScreenshotElement { locator } => {
            step_screenshot_element(tab, world, idx, locator, image_policy)
        }

        BrowserStep::Eval { expression } => step_eval(tab, idx, expression),
        BrowserStep::Styles {
            locator,
            property_filter,
        } => step_styles(tab, world, idx, locator, property_filter.as_deref()),

        BrowserStep::TabLog => step_tab_log(tab, idx),

        BrowserStep::AddLocatorHandler {
            name,
            locator,
            handler,
            times,
            no_wait_after,
        } => step_add_locator_handler(
            idx,
            handlers,
            name,
            locator,
            handler,
            *times,
            *no_wait_after,
        ),
        BrowserStep::RemoveLocatorHandler { name } => {
            step_remove_locator_handler(idx, handlers, name)
        }

        BrowserStep::DismissOverlays => {
            step_dismiss_overlays(tab, idx, handlers, locator_handler_firings, image_policy)
        }
        BrowserStep::HighlightElement { locator } => {
            step_highlight_element(tab, world, idx, locator)
        }
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
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    action: &str,
) -> StepResult {
    match resolve_interactable(tab, world, locator) {
        Ok(info) => {
            if action == "click" {
                return match call_handle_json(
                    tab,
                    world,
                    &info.handle,
                    "function() { this.click(); return JSON.stringify({ok: true}); }",
                ) {
                    Ok(_) => StepResult::success(
                        idx,
                        format!("click on <{}> ({})", info.tag, describe_locator(locator)),
                    ),
                    Err(error) => StepResult::failure(idx, "click failed", error),
                };
            }
            let action_js = match action {
                "hover" => browser_locators::js_hover_element().to_string(),
                "focus" => browser_locators::js_focus_element().to_string(),
                "blur" => browser_locators::js_blur_element().to_string(),
                "scroll_to" => browser_locators::js_scroll_to_element().to_string(),
                _ => {
                    return StepResult::failure(idx, action, format!("Unknown action: {}", action))
                }
            };
            match call_handle_json(tab, world, &info.handle, &action_js) {
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

fn step_click_if_exists(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
) -> StepResult {
    match resolve_element(tab, world, locator) {
        Ok(info) if info.visible => match call_handle_json(
            tab,
            world,
            &info.handle,
            browser_locators::js_click_element(),
        ) {
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
    let mut tokens = Vec::with_capacity(modifiers.len() + 1);
    for modifier in modifiers {
        match modifier.as_str() {
            "Alt" | "Meta" | "Shift" => tokens.push(modifier.as_str()),
            "Ctrl" | "Control" => tokens.push("Control"),
            _ => {
                return StepResult::failure(
                    idx,
                    format!("Press key {key}"),
                    format!("Unknown key: \"{modifier}\""),
                )
            }
        }
    }
    tokens.push(key);
    let chord = tokens.join("+");
    let dispatcher = CdpKeyboardDispatcher::new(tab);
    let mut keyboard = Keyboard::new(dispatcher);
    match keyboard.press(&chord, None) {
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
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    text: &str,
    clear_first: bool,
    verify: bool,
) -> StepResult {
    let info = match resolve_element(tab, world, locator) {
        Ok(i) => i,
        Err(e) => return StepResult::failure(idx, "Fill: element resolution failed", e),
    };
    match refact_browser::forms::fill(
        tab,
        world,
        &info.handle,
        &info.info,
        text,
        clear_first,
        verify,
    ) {
        Ok(outcome) => {
            let mut result = StepResult::success(
                idx,
                format!("Filled <{}> with {} chars", info.tag, text.len()),
            );
            result.field_kind = Some(info.field_kind.clone());
            result.fill_strategy = Some(outcome.strategy);
            result.verified = outcome.verified;
            result.retries = outcome.retries;
            result
        }
        Err(error) => {
            let mut result = StepResult::failure(
                idx,
                format!("Fill failed after {} strategies", error.retries),
                error.message,
            );
            result.field_kind = Some(info.field_kind.clone());
            result.retries = error.retries;
            result
        }
    }
}

fn step_clear(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    verify: bool,
) -> StepResult {
    let info = match resolve_element(tab, world, locator) {
        Ok(i) => i,
        Err(e) => return StepResult::failure(idx, "Clear: element resolution failed", e),
    };
    match refact_browser::forms::clear(tab, world, &info.handle, &info.info, verify) {
        Ok(outcome) => {
            let mut result = StepResult::success(idx, format!("Cleared <{}>", info.tag));
            result.field_kind = Some(info.field_kind.clone());
            result.fill_strategy = outcome.strategy;
            result.verified = outcome.verified;
            result.retries = outcome.retries;
            result
        }
        Err(error) => {
            let mut result = StepResult::failure(idx, "Clear failed", error.message);
            result.field_kind = Some(info.field_kind.clone());
            result.retries = error.retries;
            result
        }
    }
}

fn step_select_option(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    value: &str,
) -> StepResult {
    match resolve_element(tab, world, locator) {
        Ok(info) => match refact_browser::forms::select_option(tab, world, &info.handle, value) {
            Ok(outcome) => {
                StepResult::success(idx, format!("Selected '{}' in <{}>", value, info.tag))
                    .with_data(serde_json::json!({"selected": outcome.selected}))
            }
            Err(error) => StepResult::failure(idx, "Select option failed", error.message),
        },
        Err(e) => StepResult::failure(idx, "Select: element resolution failed", e),
    }
}

fn step_check_uncheck(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    check: bool,
) -> StepResult {
    let action = if check { "check" } else { "uncheck" };
    let info = match resolve_element(tab, world, locator) {
        Ok(i) => i,
        Err(e) => return StepResult::failure(idx, "Check/uncheck: resolution failed", e),
    };
    match refact_browser::forms::set_checked(tab, world, &info.handle, check) {
        Ok(outcome) => StepResult::success(idx, format!("{}ed <{}>", action, info.tag)).with_data(
            serde_json::json!({
                "checked": outcome.checked,
                "changed": outcome.changed,
                "verified": outcome.verified,
            }),
        ),
        Err(error) => StepResult::failure(idx, format!("{} failed", action), error.message),
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

fn wait_for_report_stability(tab: &Tab, world: &WorldManager, timeout_ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    if poll_until_deadline(tab, "document.readyState !== 'loading'", deadline).is_err() {
        return false;
    }
    let _ = tab.evaluate(NETWORK_INFLIGHT_TRACKER_JS, false);
    if wait_for_network_quiet_until(tab, deadline).is_err() {
        return false;
    }

    let remaining = deadline.saturating_duration_since(Instant::now());
    let max_checks = (remaining.as_millis() / REPORT_STABILITY_INTERVAL_MS as u128) as usize;
    html_stabilizes_with(
        max_checks.max(1),
        || {
            world.eval_in_utility(
                tab,
                "document.documentElement ? document.documentElement.outerHTML : ''",
            )
        },
        || std::thread::sleep(Duration::from_millis(REPORT_STABILITY_INTERVAL_MS)),
    )
}

fn html_stabilizes_with<F, S>(max_checks: usize, mut sample: F, mut sleep: S) -> bool
where
    F: FnMut() -> Result<serde_json::Value, String>,
    S: FnMut(),
{
    let mut previous: Option<serde_json::Value> = None;
    for _ in 0..max_checks {
        let current = match sample() {
            Ok(value) => value,
            Err(_) => return false,
        };
        if previous.as_ref() == Some(&current) {
            return true;
        }
        previous = Some(current);
        sleep();
    }
    false
}

fn poll_until_deadline(tab: &Tab, js_condition: &str, deadline: Instant) -> Result<(), String> {
    loop {
        if eval_js_value(tab, js_condition)
            .ok()
            .and_then(|value| value.as_bool())
            == Some(true)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("Timed out".to_string());
        }
        std::thread::sleep(Duration::from_millis(REPORT_STABILITY_INTERVAL_MS));
    }
}

fn wait_for_network_quiet_until(tab: &Tab, deadline: Instant) -> Result<(), String> {
    let snapshot_js = r#"(function() {
  return window.__refact_inflight_installed ? (window.__refact_inflight | 0) : 0;
})()"#;
    let quiet_window = Duration::from_millis(NETWORK_IDLE_WINDOW_MS);
    let mut quiet_since: Option<Instant> = None;
    loop {
        let inflight = eval_js_value(tab, snapshot_js)
            .ok()
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        if inflight == 0 {
            if quiet_since.is_some_and(|since| since.elapsed() >= quiet_window) {
                return Ok(());
            }
            quiet_since.get_or_insert_with(Instant::now);
        } else {
            quiet_since = None;
        }
        if Instant::now() >= deadline {
            return Err("Timed out".to_string());
        }
        std::thread::sleep(Duration::from_millis(REPORT_STABILITY_INTERVAL_MS));
    }
}

fn mask_console_entry(
    mut entry: refact_integrations::browser_types::ConsoleEntry,
) -> refact_integrations::browser_types::ConsoleEntry {
    entry.text = refact_core::string_utils::redact_sensitive(&entry.text);
    entry
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

fn step_get_text(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
) -> StepResult {
    match resolve_element(tab, world, locator) {
        Ok(info) => {
            match call_handle_json(tab, world, &info.handle, browser_locators::js_get_text()) {
                Ok(result) => {
                    let text = result.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    StepResult::success(idx, format!("Got text from <{}>", info.tag))
                        .with_data(serde_json::json!({"text": text}))
                }
                Err(e) => StepResult::failure(idx, "Get text failed", e),
            }
        }
        Err(e) => StepResult::failure(idx, "Get text: resolution failed", e),
    }
}

fn step_get_html(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
) -> StepResult {
    match resolve_element(tab, world, locator) {
        Ok(info) => {
            match call_handle_json(tab, world, &info.handle, browser_locators::js_get_html()) {
                Ok(result) => {
                    let html = result.get("html").and_then(|v| v.as_str()).unwrap_or("");
                    StepResult::success(idx, format!("Got HTML from <{}>", info.tag))
                        .with_data(serde_json::json!({"html": html}))
                }
                Err(e) => StepResult::failure(idx, "Get HTML failed", e),
            }
        }
        Err(e) => StepResult::failure(idx, "Get HTML: resolution failed", e),
    }
}

fn step_get_attribute(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    attribute: &str,
) -> StepResult {
    match resolve_element(tab, world, locator) {
        Ok(info) => {
            let js = browser_locators::js_get_attribute(attribute);
            match call_handle_json(tab, world, &info.handle, &js) {
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
    world: &WorldManager,
    idx: usize,
    locator: Option<&BrowserLocator>,
    limit: Option<usize>,
) -> StepResult {
    let effective_limit = limit.unwrap_or(50).min(MAX_EXTRACT_LINKS);
    let js = browser_locators::js_extract_links(effective_limit);
    let result = match locator {
        Some(locator) => match resolve_element(tab, world, locator) {
            Ok(info) => call_handle_json(tab, world, &info.handle, &js),
            Err(error) => {
                return StepResult::failure(idx, "Extract links: resolution failed", error)
            }
        },
        None => eval_js_ok(tab, &js),
    };
    match result {
        Ok(result) => StepResult::success(idx, "Extracted links".to_string()).with_data(result),
        Err(e) => StepResult::failure(idx, "Extract links failed", e),
    }
}

fn step_extract_table(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
) -> StepResult {
    match resolve_element(tab, world, locator) {
        Ok(info) => match call_handle_json(
            tab,
            world,
            &info.handle,
            browser_locators::js_extract_table(),
        ) {
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

pub fn capture_viewport_screenshot(
    tab: &Tab,
    policy: &ImagePolicy,
) -> Result<(String, String), String> {
    let (format, mime, quality) = capture_options(policy);
    capture_viewport_screenshot_as(tab, format, mime, quality)
}

fn capture_report_screenshot(tab: &Tab, policy: &ImagePolicy) -> Result<BrowserScreenshot, String> {
    let (data, mime) = capture_viewport_screenshot_png(tab)?;
    let bytes = base64::prelude::BASE64_STANDARD
        .decode(data)
        .map_err(|error| format!("Screenshot decode failed: {error}"))?;
    let (resized, mime) = resize_to_policy(&bytes, &mime, policy)?;
    Ok(BrowserScreenshot {
        mime,
        data: base64::prelude::BASE64_STANDARD.encode(resized),
    })
}

fn capture_viewport_screenshot_as(
    tab: &Tab,
    format: Page::CaptureScreenshotFormatOption,
    mime: &'static str,
    quality: Option<u8>,
) -> Result<(String, String), String> {
    let result = tab
        .call_method(Page::CaptureScreenshot {
            format: Some(format),
            clip: None,
            quality: quality.map(|q| q as u32),
            from_surface: Some(true),
            capture_beyond_viewport: Some(false),
            optimize_for_speed: None,
        })
        .map_err(|error| error.to_string())?;
    Ok((result.data, mime.to_string()))
}

pub fn capture_viewport_screenshot_png(tab: &Tab) -> Result<(String, String), String> {
    capture_viewport_screenshot_as(
        tab,
        Page::CaptureScreenshotFormatOption::Png,
        "image/png",
        None,
    )
}

fn step_screenshot(tab: &Tab, idx: usize, policy: &ImagePolicy) -> StepResult {
    match capture_viewport_screenshot(tab, policy) {
        Ok((data, mime)) => StepResult::success(idx, "Screenshot captured".to_string()).with_data(
            serde_json::json!({
                "mime": mime,
                "data": data,
            }),
        ),
        Err(error) => StepResult::failure(idx, "Screenshot failed", error),
    }
}

fn step_screenshot_element(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    policy: &ImagePolicy,
) -> StepResult {
    let info = match resolve_element(tab, world, locator) {
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

    let (format, mime, quality) = capture_options(policy);
    match tab.call_method(Page::CaptureScreenshot {
        format: Some(format),
        clip: Some(clip),
        quality: quality.map(|q| q as u32),
        from_surface: Some(true),
        capture_beyond_viewport: Some(false),
        optimize_for_speed: None,
    }) {
        Ok(result) => StepResult::success(idx, format!("Element screenshot of <{}>", info.tag))
            .with_data(serde_json::json!({
                "mime": mime,
                "data": result.data,
            })),
        Err(e) => StepResult::failure(idx, "Element screenshot failed", e.to_string()),
    }
}

fn capture_options(
    policy: &ImagePolicy,
) -> (
    Page::CaptureScreenshotFormatOption,
    &'static str,
    Option<u8>,
) {
    match policy.format {
        ImageFormat::Jpeg => (
            Page::CaptureScreenshotFormatOption::Jpeg,
            "image/jpeg",
            policy.quality,
        ),
        ImageFormat::Png | ImageFormat::Webp => {
            (Page::CaptureScreenshotFormatOption::Png, "image/png", None)
        }
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
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    property_filter: Option<&str>,
) -> StepResult {
    match resolve_element(tab, world, locator) {
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
  var el = this;
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
            match call_handle_json(tab, world, &info.handle, &js) {
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

fn step_add_locator_handler(
    idx: usize,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    name: &str,
    locator: &BrowserLocator,
    action: &LocatorHandlerAction,
    times: Option<u32>,
    no_wait_after: bool,
) -> StepResult {
    let Some(handlers) = handlers else {
        return StepResult::failure(idx, "Add locator handler", "Handler registry unavailable");
    };
    let handler = match LocatorHandler::registered(
        name.to_string(),
        locator.clone(),
        action.clone(),
        times,
        no_wait_after,
    ) {
        Ok(Some(handler)) => handler,
        Ok(None) => {
            return StepResult::success(idx, format!("Locator handler '{name}' not registered"));
        }
        Err(error) => return StepResult::failure(idx, "Add locator handler", error),
    };
    match handlers.lock() {
        Ok(mut registry) => {
            registry.register(handler);
            StepResult::success(idx, format!("Registered locator handler '{name}'"))
        }
        Err(error) => StepResult::failure(
            idx,
            "Add locator handler",
            format!("Failed to lock locator handlers: {error}"),
        ),
    }
}

fn step_remove_locator_handler(
    idx: usize,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    name: &str,
) -> StepResult {
    let Some(handlers) = handlers else {
        return StepResult::failure(
            idx,
            "Remove locator handler",
            "Handler registry unavailable",
        );
    };
    match handlers.lock() {
        Ok(mut registry) => {
            let removed = registry.unregister(name);
            StepResult::success(
                idx,
                if removed {
                    format!("Removed locator handler '{name}'")
                } else {
                    format!("Locator handler '{name}' was not registered")
                },
            )
        }
        Err(error) => StepResult::failure(
            idx,
            "Remove locator handler",
            format!("Failed to lock locator handlers: {error}"),
        ),
    }
}

fn dismiss_overlays(tab: &Tab) -> Result<String, String> {
    match eval_js_ok(tab, browser_locators::js_dismiss_overlays()) {
        Ok(result) => {
            let count = result
                .get("dismissed")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Ok(format!("Dismissed {count} overlay(s)"))
        }
        Err(error) => Err(error),
    }
}

fn step_dismiss_overlays(
    tab: &Tab,
    idx: usize,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
) -> StepResult {
    let Some(handlers) = handlers else {
        return match dismiss_overlays(tab) {
            Ok(outcome) => StepResult::success(idx, outcome),
            Err(error) => StepResult::failure(idx, "Dismiss overlays failed", error),
        };
    };
    let existing = match handlers.lock() {
        Ok(registry) => registry.get(DEFAULT_DISMISS_OVERLAYS_HANDLER),
        Err(error) => {
            return StepResult::failure(
                idx,
                "Dismiss overlays failed",
                format!("Failed to lock locator handlers: {error}"),
            )
        }
    };
    let handler = existing.unwrap_or_else(LocatorHandler::dismiss_overlays);
    let lease = {
        let mut registry = match handlers.lock() {
            Ok(registry) => registry,
            Err(error) => {
                return StepResult::failure(
                    idx,
                    "Dismiss overlays failed",
                    format!("Failed to lock locator handlers: {error}"),
                )
            }
        };
        if registry.get(DEFAULT_DISMISS_OVERLAYS_HANDLER).is_none() {
            registry.register(handler);
        }
        registry.begin(DEFAULT_DISMISS_OVERLAYS_HANDLER)
    };
    let Some(lease) = lease else {
        return StepResult::failure(idx, "Dismiss overlays failed", "Handler is already running");
    };
    let result = execute_locator_handler(
        tab,
        &WorldManager::default(),
        handlers,
        firings,
        image_policy,
        Instant::now() + Duration::from_millis(DEFAULT_WAIT_TIMEOUT_MS),
        &lease,
    );
    let (ok, outcome) = match &result {
        Ok(outcome) => (true, outcome.clone()),
        Err(error) => (false, error.clone()),
    };
    match handlers.lock() {
        Ok(mut registry) => firings.push(registry.finish(lease, ok, outcome)),
        Err(error) => {
            return StepResult::failure(
                idx,
                "Dismiss overlays failed",
                format!("Failed to lock locator handlers: {error}"),
            )
        }
    }
    match result {
        Ok(outcome) => StepResult::success(idx, outcome),
        Err(error) => StepResult::failure(idx, "Dismiss overlays failed", error),
    }
}

fn step_highlight_element(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
) -> StepResult {
    match resolve_element(tab, world, locator) {
        Ok(info) => match call_handle_json(
            tab,
            world,
            &info.handle,
            browser_locators::js_highlight_element(),
        ) {
            Ok(_) => StepResult::success(idx, format!("Highlighted <{}>", info.tag)),
            Err(e) => StepResult::failure(idx, "Highlight failed", e),
        },
        Err(e) => StepResult::failure(idx, "Highlight: resolution failed", e),
    }
}

pub fn describe_locator(locator: &BrowserLocator) -> String {
    match &locator.strategy {
        LocatorStrategy::Css { value } => format!("css={}", value),
        LocatorStrategy::Id { value } => format!("id={}", value),
        LocatorStrategy::Name { value } => format!("name={}", value),
        LocatorStrategy::TestId { value, .. } => format!("testid={}", value),
        LocatorStrategy::Placeholder { value, .. } => format!("placeholder={}", value),
        LocatorStrategy::Autocomplete { value } => format!("autocomplete={}", value),
        LocatorStrategy::Text { value, exact, .. } => {
            if *exact {
                format!("text=\"{}\"", value)
            } else {
                format!("text~\"{}\"", value)
            }
        }
        LocatorStrategy::Label { value, .. } => format!("label={}", value),
        LocatorStrategy::Role { role, name, .. } => match name {
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
                regex: None,
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
                regex: None,
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
    fn console_report_text_is_redacted() {
        let entry = mask_console_entry(refact_integrations::browser_types::ConsoleEntry {
            timestamp: 1.0,
            level: "error".to_string(),
            text: "password=hunter2".to_string(),
        });

        assert_eq!(entry.text, "password=[REDACTED]");
    }

    #[test]
    fn html_stability_gate_returns_false_after_max_checks() {
        let mut value = 0;
        let stabilized = html_stabilizes_with(
            3,
            || {
                value += 1;
                Ok(serde_json::json!(value))
            },
            || {},
        );

        assert!(!stabilized);
    }

    #[test]
    fn html_stability_gate_accepts_two_equal_samples() {
        let stabilized = html_stabilizes_with(2, || Ok(serde_json::json!("stable")), || {});

        assert!(stabilized);
    }
}
