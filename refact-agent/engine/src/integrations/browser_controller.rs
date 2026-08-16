use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use headless_chrome::Tab;
use headless_chrome::protocol::cdp::Page;
use tokio::sync::Mutex as AMutex;

use crate::integrations::browser_locators::{
    self, js_string_literal, parse_element_info, INSPECT_ELEMENT_JS,
};
use crate::integrations::browser_models::*;
use crate::integrations::browser_runtime::BrowserRuntime;
use refact_browser::{
    ActionKind, ActionabilityDiagnostic, ActionabilityDriver, ActionabilityEngine,
    ActionabilityExecutionMode, ActionabilityTimeouts, CdpKeyboardDispatcher, CdpMouseDispatcher,
    ElementHandle, HitTargetController, HitTargetPoint, HitTargetResult, Keyboard, LocatorHandler,
    LocatorHandlerLease, LocatorHandlerOperation, LocatorHandlerProbe, LocatorHandlerRegistry,
    LocatorOutcome, Mouse, MouseButton, NetworkLoadState, NetworkMonitorHandle, Ref,
    ScrollStrategy, SnapshotMode, SnapshotOptions, SystemClock, UrlMatcher, WorldManager,
    DEFAULT_DISMISS_OVERLAYS_HANDLER,
};
use refact_core::image_policy::{resize_to_policy, ImageFormat, ImagePolicy};

use crate::global_context::GlobalContext;

const DEFAULT_WAIT_TIMEOUT_MS: u64 = 5_000;
const MAX_WAIT_TIMEOUT_MS: u64 = 60_000;
const MAX_WAIT_SECONDS: f64 = 60.0;
const MIN_WAIT_SECONDS: f64 = 0.0;

const MAX_DOM_SNAPSHOT_CHARS: usize = 100_000;
const MAX_EXTRACT_LINKS: usize = 500;
const DEFAULT_ARIA_SNAPSHOT_CHARS: usize = 20_000;
const MAX_ARIA_SNAPSHOT_CHARS: usize = 100_000;

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

struct BrowserActionDriver<'a> {
    tab: &'a Tab,
    world: &'a WorldManager,
    locator: &'a BrowserLocator,
    action: ActionKind,
    handlers: Option<&'a Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &'a mut Vec<LocatorHandlerFiring>,
    image_policy: &'a ImagePolicy,
    precheck_deadline: Instant,
    resolved: Option<ResolvedElement>,
}

impl<'a> BrowserActionDriver<'a> {
    fn new(
        tab: &'a Tab,
        world: &'a WorldManager,
        locator: &'a BrowserLocator,
        action: ActionKind,
        handlers: Option<&'a Arc<Mutex<LocatorHandlerRegistry>>>,
        locator_handler_firings: &'a mut Vec<LocatorHandlerFiring>,
        image_policy: &'a ImagePolicy,
    ) -> Self {
        Self {
            tab,
            world,
            locator,
            action,
            handlers,
            locator_handler_firings,
            image_policy,
            precheck_deadline: Instant::now() + Duration::from_millis(DEFAULT_WAIT_TIMEOUT_MS),
            resolved: None,
        }
    }

    fn release_resolved(&mut self) {
        if let Some(resolved) = self.resolved.take() {
            let _ = self.world.release_handle(self.tab, &resolved.handle);
        }
    }

    fn resolved(&self) -> Result<&ResolvedElement, ActionabilityDiagnostic> {
        self.resolved
            .as_ref()
            .ok_or(ActionabilityDiagnostic::Detached)
    }
}

impl Drop for BrowserActionDriver<'_> {
    fn drop(&mut self) {
        self.release_resolved();
    }
}

impl ActionabilityDriver for BrowserActionDriver<'_> {
    type Output = String;

    fn resolve(&mut self) -> LocatorOutcome {
        self.release_resolved();
        match resolve_element(self.tab, self.world, self.locator) {
            Ok(resolved) => {
                let preview = element_preview(self.tab, self.world, &resolved.handle);
                self.resolved = Some(resolved);
                LocatorOutcome::Found { preview }
            }
            Err(error) => {
                if matches!(self.locator.strategy, LocatorStrategy::Ref { .. }) {
                    LocatorOutcome::Error { description: error }
                } else if let Some(count) = strict_mode_count(&error) {
                    LocatorOutcome::MultipleMatches { count }
                } else {
                    LocatorOutcome::NotFound
                }
            }
        }
    }

    fn element_state(&mut self) -> Result<refact_browser::ElementState, ActionabilityDiagnostic> {
        let resolved = self.resolved()?;
        self.world
            .element_states(self.tab, &resolved.handle)
            .map_err(|error| match error {
                refact_browser::HandleError::Invalidated { .. } => {
                    ActionabilityDiagnostic::Detached
                }
                _ => ActionabilityDiagnostic::PrecheckFailed {
                    description: error.to_string(),
                },
            })
    }

    fn perform(&mut self) -> Result<Self::Output, ActionabilityDiagnostic> {
        let resolved = self.resolved()?;
        let dispatcher = CdpMouseDispatcher::new(self.tab);
        if self.action == ActionKind::Focus {
            return call_handle_json(
                self.tab,
                self.world,
                &resolved.handle,
                &browser_locators::js_focus_element(),
            )
            .map(|_| resolved.info.tag.clone())
            .map_err(|description| ActionabilityDiagnostic::PrecheckFailed { description });
        }
        if self.action == ActionKind::ScrollIntoViewIfNeeded {
            return dispatcher
                .scroll_into_view(&resolved.handle, ScrollStrategy::Protocol)
                .map(|_| resolved.info.tag.clone())
                .map_err(|error| ActionabilityDiagnostic::PrecheckFailed {
                    description: error.to_string(),
                });
        }
        let point = dispatcher
            .clickable_point(&resolved.handle)
            .map_err(|error| match error {
                refact_browser::MouseError::OutsideViewport => {
                    ActionabilityDiagnostic::OutsideViewport
                }
                _ => ActionabilityDiagnostic::PrecheckFailed {
                    description: error.to_string(),
                },
            })?;
        let hit_target = HitTargetController::default();
        let hit_target_point = HitTargetPoint {
            x: point.x,
            y: point.y,
        };
        match hit_target.expect_hit_target(self.tab, self.world, &resolved.handle, hit_target_point)
        {
            Ok(HitTargetResult::Done) => {}
            Ok(HitTargetResult::Intercepted { description }) => {
                return Err(intercepts_pointer_events(description));
            }
            Ok(HitTargetResult::NotConnected) => {
                return Err(ActionabilityDiagnostic::Detached);
            }
            Ok(HitTargetResult::Skipped) => {}
            Err(error) => {
                return Err(ActionabilityDiagnostic::PrecheckFailed {
                    description: error.to_string(),
                });
            }
        }
        let token = hit_target
            .install_interceptor(
                self.tab,
                self.world,
                &resolved.handle,
                self.action,
                Some(hit_target_point),
            )
            .map_err(|error| ActionabilityDiagnostic::PrecheckFailed {
                description: error.to_string(),
            })?;
        let keyboard = Keyboard::new(CdpKeyboardDispatcher::new(self.tab));
        let mut mouse = Mouse::new(dispatcher, &keyboard);
        let action_result = match self.action {
            ActionKind::Click => mouse.click(point.x, point.y, MouseButton::Left),
            ActionKind::Hover => mouse.hover(point.x, point.y),
            _ => unreachable!(),
        };
        let hit_result = hit_target.take_result(self.tab, self.world, token);
        action_result.map_err(|error| ActionabilityDiagnostic::PrecheckFailed {
            description: error.to_string(),
        })?;
        match hit_result {
            Ok(HitTargetResult::Done | HitTargetResult::Skipped) => Ok(resolved.info.tag.clone()),
            Ok(HitTargetResult::Intercepted { description }) => {
                Err(intercepts_pointer_events(description))
            }
            Ok(HitTargetResult::NotConnected) => Err(ActionabilityDiagnostic::Detached),
            Err(error) => Err(ActionabilityDiagnostic::PrecheckFailed {
                description: error.to_string(),
            }),
        }
    }

    fn wait_for_navigation(&mut self) -> Result<(), ActionabilityDiagnostic> {
        wait_for_pending_navigation(self.tab, self.precheck_deadline)
            .map_err(|description| ActionabilityDiagnostic::PrecheckFailed { description })
    }

    fn locator_handlers_checkpoint(&mut self) -> Result<(), ActionabilityDiagnostic> {
        let Some(handlers) = self.handlers else {
            return Ok(());
        };
        perform_locator_handlers_checkpoint(
            self.tab,
            self.world,
            handlers,
            self.locator_handler_firings,
            self.image_policy,
            self.precheck_deadline,
        )
        .map_err(|description| ActionabilityDiagnostic::PrecheckFailed { description })
    }
}

fn intercepts_pointer_events(description: String) -> ActionabilityDiagnostic {
    ActionabilityDiagnostic::InterceptsPointerEvents {
        description: description
            .strip_suffix(" intercepts pointer events")
            .unwrap_or(&description)
            .to_string(),
    }
}

fn element_preview(tab: &Tab, world: &WorldManager, handle: &ElementHandle) -> String {
    world
        .call_function_on(
            tab,
            handle,
            "function() { return this.outerHTML.substring(0, 500); }",
            Vec::new(),
        )
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "<element>".to_string())
}

fn strict_mode_count(error: &str) -> Option<usize> {
    let (_, suffix) = error.split_once(" resolved to ")?;
    suffix.split_whitespace().next()?.parse().ok()
}

fn resolve_element(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
) -> Result<ResolvedElement, String> {
    let handles = resolve_locator_handles(tab, world, locator)?;
    let handle = strict_locator_handle(tab, world, locator, handles)?
        .ok_or_else(|| "Element not found".to_string())?;
    inspect_resolved_element(tab, world, handle)
}

fn resolve_locator_handles(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
) -> Result<Vec<ElementHandle>, String> {
    match &locator.strategy {
        LocatorStrategy::Ref { value } => {
            if !locator.frames.is_empty()
                || locator.nth.is_some()
                || locator.within.is_some()
                || locator.locator.is_some()
                || locator.filter.is_some()
                || locator.and.is_some()
                || locator.or.is_some()
                || locator.first.is_some()
                || locator.last.is_some()
            {
                return Err("Ref locators cannot be composed or filtered".to_string());
            }
            let reference = value
                .parse::<Ref>()
                .map_err(|error| format!("Invalid browser ref {value}: {error}"))?;
            Ok(vec![world
                .resolve_ref(tab, &reference)
                .map_err(|error| error.to_string())?])
        }
        _ => {
            let locator_value = locator_without_frames(locator)?;
            if locator.frames.is_empty() {
                world
                    .call_injected_handles(tab, "resolveAll", serde_json::json!([locator_value]))
                    .map_err(|error| error.to_string())
            } else {
                let owners = locator
                    .frames
                    .iter()
                    .map(locator_without_frames)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| format!("Failed to serialize frame locator: {error}"))?;
                world
                    .resolve_frame_locator(tab, &owners, locator_value)
                    .map_err(|error| error.to_string())
            }
        }
    }
}

fn locator_without_frames(locator: &BrowserLocator) -> Result<serde_json::Value, String> {
    let mut locator = serde_json::to_value(locator)
        .map_err(|error| format!("Failed to serialize browser locator: {error}"))?;
    locator
        .as_object_mut()
        .ok_or_else(|| "Browser locator did not serialize as an object".to_string())?
        .remove("frames");
    Ok(locator)
}

fn strict_locator_handle(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
    mut handles: Vec<ElementHandle>,
) -> Result<Option<ElementHandle>, String> {
    if handles.len() > 1 {
        let count = handles.len();
        let previews = handles
            .iter()
            .take(5)
            .filter_map(|handle| {
                world
                    .call_function_on(
                        tab,
                        handle,
                        "function() { return this.outerHTML.substring(0, 200); }",
                        Vec::new(),
                    )
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
            })
            .collect::<Vec<_>>();
        for handle in &handles {
            let _ = world.release_handle(tab, handle);
        }
        return Err(refact_browser::strict_mode_violation(
            &describe_locator(locator),
            count,
            &previews,
        ));
    }
    Ok(handles.pop())
}

fn inspect_resolved_element(
    tab: &Tab,
    world: &WorldManager,
    handle: ElementHandle,
) -> Result<ResolvedElement, String> {
    let inspect = format!(
        "function() {{ {INSPECT_ELEMENT_JS} return JSON.stringify(__refact_inspect_element(this, 1)); }}"
    );
    let val = match world.call_function_on(tab, &handle, &inspect, Vec::new()) {
        Ok(value) => value,
        Err(error) => {
            let _ = world.release_handle(tab, &handle);
            return Err(error.to_string());
        }
    };
    let json_str = match val.as_str() {
        Some(s) => s.to_string(),
        None => match serde_json::to_string(&val) {
            Ok(json) => json,
            Err(error) => {
                let _ = world.release_handle(tab, &handle);
                return Err(format!("Failed to serialize resolve result: {error}"));
            }
        },
    };
    let info = match parse_element_info(&json_str) {
        Ok(info) => info,
        Err(error) => {
            let _ = world.release_handle(tab, &handle);
            return Err(error);
        }
    };
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

    let mut results: Vec<StepResult> = Vec::new();
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
        network: vec![],
        locator_handlers,
        dialogs: vec![],
        uploads: vec![],
        downloads: vec![],
        new_tabs: vec![],
        screenshot: None,
    }
}

pub fn is_tab_management_step(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::OpenTab { .. }
            | BrowserStep::CloseTab { .. }
            | BrowserStep::SwitchTab { .. }
            | BrowserStep::ListTabs
            | BrowserStep::HandleDialog { .. }
            | BrowserStep::ExpectFileChooser { .. }
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

pub async fn execute_request_with_runtime_validated(
    runtime_arc: Arc<AMutex<BrowserRuntime>>,
    request: BrowserActionRequest,
    image_policy: &ImagePolicy,
    gcx: Arc<GlobalContext>,
) -> Result<ExecutionReport, String> {
    validate_upload_paths(gcx, &request).await?;
    execute_request_with_runtime(runtime_arc, request, image_policy).await
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
    let network_monitor = {
        let rt = runtime_arc.lock().await;
        rt.network_monitor.clone()
    };
    let download_monitor = {
        let rt = runtime_arc.lock().await;
        rt.download_monitor.clone()
    };
    let file_chooser_manager = {
        let rt = runtime_arc.lock().await;
        rt.file_chooser_manager.clone()
    };
    let armed_network_waits = request
        .steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| match step {
            BrowserStep::WaitForRequest { .. } => Some((index, network_monitor.request_cursor())),
            BrowserStep::WaitForResponse { .. } => Some((index, network_monitor.response_cursor())),
            BrowserStep::WaitForDownload { .. } => Some((index, download_monitor.cursor())),
            _ => None,
        })
        .collect::<std::collections::HashMap<_, _>>();
    let initial_tab_ids = {
        let mut rt = runtime_arc.lock().await;
        refact_browser::adopt_new_tabs(&mut rt, None);
        rt.known_tab_ids()
    };
    let mut results: Vec<StepResult> = Vec::new();
    let mut locator_handlers = Vec::new();
    let mut all_ok = true;
    let mut pending_popup_wait: Option<(usize, Instant, u64, std::collections::BTreeSet<String>)> =
        None;
    let mut new_tabs = Vec::new();

    for (idx, step) in request.steps.iter().enumerate() {
        let step_tab_ids = runtime_arc.lock().await.known_tab_ids();
        if let Some(tab) = &current_tab {
            let _ = tab.evaluate(NETWORK_INFLIGHT_TRACKER_JS, false);
        }
        let file_chooser_was_armed = file_chooser_manager.is_armed();
        let mut result = if let BrowserStep::WaitForPopup { timeout_ms } = step {
            let baseline = runtime_arc.lock().await.known_tab_ids();
            pending_popup_wait = Some((
                results.len(),
                Instant::now(),
                clamp_timeout_ms(*timeout_ms),
                baseline,
            ));
            StepResult::success(idx, "Armed popup wait")
        } else if let BrowserStep::WaitForDownload {
            timeout_ms,
            save_as,
        } = step
        {
            let timeout = Duration::from_millis(clamp_timeout_ms(*timeout_ms));
            match tokio::task::block_in_place(|| {
                download_monitor.wait_for_download(
                    armed_network_waits
                        .get(&idx)
                        .copied()
                        .unwrap_or_else(|| download_monitor.cursor()),
                    timeout,
                    save_as.as_deref(),
                )
            }) {
                Ok(download) => {
                    StepResult::success(idx, format!("Downloaded {}", download.suggested_filename))
                        .with_data(serde_json::to_value(download).unwrap_or_default())
                }
                Err(error) => StepResult::failure(idx, "Wait for download", error),
            }
        } else if is_tab_management_step(step) {
            let step_report = tokio::task::block_in_place(|| {
                let mut rt = runtime_arc.blocking_lock();
                execute_steps_with_runtime(&mut rt, std::slice::from_ref(step), image_policy)
            });
            {
                let mut rt = runtime_arc.lock().await;
                rt.touch();
                current_tab = rt.get_active_tab();
            }
            new_tabs.extend(step_report.new_tabs);
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
                    execute_runtime_network_step(
                        tab,
                        &world,
                        &network_monitor,
                        step,
                        idx,
                        armed_network_waits.get(&idx).copied(),
                        image_policy,
                        &handlers,
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
        if file_chooser_was_armed && matches!(step, BrowserStep::Click { .. }) {
            if result.ok {
                result = match &current_tab {
                    Some(tab) => tokio::task::block_in_place(|| {
                        let completed = file_chooser_manager
                            .complete(tab, Duration::from_millis(DEFAULT_WAIT_TIMEOUT_MS));
                        let _ = tab.set_file_chooser_dialog_interception(false, None);
                        match completed {
                            Ok(upload) => {
                                let mut completed = StepResult::success(
                                    idx,
                                    format!(
                                        "Selected {} file(s) from file chooser",
                                        upload.paths.len()
                                    ),
                                )
                                .with_data(serde_json::to_value(upload).unwrap_or_default());
                                completed.retries = result.retries;
                                completed.actionability = result.actionability;
                                completed
                            }
                            Err(error) => StepResult::failure(idx, "File chooser failed", error),
                        }
                    }),
                    None => StepResult::failure(idx, "File chooser failed", "No active tab"),
                };
            } else {
                file_chooser_manager.disarm();
                if let Some(tab) = &current_tab {
                    let _ = tab.set_file_chooser_dialog_interception(false, None);
                }
            }
        }
        if result.ok && matches!(step, BrowserStep::SetInputFiles { .. }) {
            if let Some(data) = result.data.clone() {
                if let Ok(upload) = serde_json::from_value(data) {
                    file_chooser_manager.record(upload);
                }
            }
        }
        result.step_index = idx;

        let resolves_popup_wait = pending_popup_wait.is_some()
            && !matches!(step, BrowserStep::WaitForPopup { .. })
            && !is_tab_management_step(step);
        if !matches!(step, BrowserStep::WaitForPopup { .. }) {
            if resolves_popup_wait {
                let (result_index, started, timeout_ms, baseline) = pending_popup_wait.unwrap();
                let deadline = started + Duration::from_millis(timeout_ms);
                let popup = loop {
                    let popup = {
                        let mut rt = runtime_arc.lock().await;
                        refact_browser::adopt_new_tabs(&mut rt, Some(idx));
                        let popup_id = rt
                            .list_tab_infos()
                            .into_iter()
                            .find(|tab| !baseline.contains(&tab.id))
                            .map(|tab| tab.id);
                        popup_id.and_then(|tab_id| {
                            rt.tab_opened_by_step.entry(tab_id.clone()).or_insert(idx);
                            rt.set_active_tab_target_id(tab_id.clone());
                            rt.list_tab_infos().into_iter().find(|tab| tab.id == tab_id)
                        })
                    };
                    if let Some(tab) = popup {
                        break Some(tab);
                    }
                    if Instant::now() >= deadline {
                        break None;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                };
                pending_popup_wait = None;
                match popup {
                    Some(tab) => {
                        new_tabs.push(tab.clone());
                        if let Some(wait_result) = results.get_mut(result_index) {
                            wait_result.summary = format!("Popup opened: {}", tab.id);
                            wait_result.data = Some(serde_json::json!({"tab_id": tab.id}));
                        }
                        let rt = runtime_arc.lock().await;
                        current_tab = rt.get_active_tab();
                    }
                    None => {
                        if let Some(wait_result) = results.get_mut(result_index) {
                            *wait_result = StepResult::failure(
                                wait_result.step_index,
                                "Wait for popup",
                                format!("Timed out after {timeout_ms}ms"),
                            );
                        }
                        all_ok = false;
                    }
                }
            } else if pending_popup_wait.is_none() {
                let adopted = {
                    let mut rt = runtime_arc.lock().await;
                    if result.ok
                        && matches!(
                            step,
                            BrowserStep::Click { .. } | BrowserStep::ClickIfExists { .. }
                        )
                    {
                        tokio::task::block_in_place(|| {
                            refact_browser::wait_for_new_tabs(
                                &mut rt,
                                &step_tab_ids,
                                Some(idx),
                                Duration::from_millis(500),
                            )
                        })
                    } else {
                        refact_browser::adopt_new_tabs(&mut rt, Some(idx))
                    }
                };
                new_tabs.extend(adopted);
            }
        }

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
        if !all_ok {
            break;
        }
    }

    if let Some((result_index, _, timeout_ms, _)) = pending_popup_wait.take() {
        if let Some(wait_result) = results.get_mut(result_index) {
            *wait_result = StepResult::failure(
                wait_result.step_index,
                "Wait for popup",
                format!("Timed out after {timeout_ms}ms"),
            );
        }
        all_ok = false;
    }

    {
        let mut rt = runtime_arc.lock().await;
        refact_browser::adopt_new_tabs(&mut rt, None);
        new_tabs.extend(
            rt.list_tab_infos()
                .into_iter()
                .filter(|tab| !initial_tab_ids.contains(&tab.id)),
        );
    }
    new_tabs.retain(|tab| !initial_tab_ids.contains(&tab.id));
    let mut seen_new_tabs = std::collections::HashSet::new();
    new_tabs.retain(|tab| seen_new_tabs.insert(tab.id.clone()));

    let active_tab = match current_tab {
        Some(tab) => Some(tab),
        None => runtime_arc.lock().await.get_active_tab(),
    };
    let (url, title, stabilized, screenshot) = if let Some(tab) = active_tab {
        let stabilized = tokio::task::block_in_place(|| {
            wait_for_report_stability(
                &tab,
                &world,
                &network_monitor,
                REPORT_STABILIZATION_TIMEOUT_MS,
            )
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
    let (console, page_errors, network, dialogs, uploads, downloads) = {
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
        let network = rt.flush_report_network();
        let dialogs = rt.dialog_manager.take_reports();
        let uploads = rt.file_chooser_manager.take_uploads();
        let downloads = rt.download_monitor.take_report();
        (console, page_errors, network, dialogs, uploads, downloads)
    };

    Ok(ExecutionReport {
        ok: all_ok,
        steps: results,
        url,
        title,
        stabilized,
        console,
        page_errors,
        network,
        locator_handlers,
        dialogs,
        uploads,
        downloads,
        new_tabs,
        screenshot,
    })
}

async fn validate_upload_paths(
    gcx: Arc<GlobalContext>,
    request: &BrowserActionRequest,
) -> Result<(), String> {
    for path in request.steps.iter().flat_map(|step| match step {
        BrowserStep::SetInputFiles { paths, .. } | BrowserStep::ExpectFileChooser { paths } => {
            paths.as_slice()
        }
        _ => &[],
    }) {
        let path = PathBuf::from(path);
        crate::files_correction::check_if_its_inside_a_workspace_or_config(gcx.clone(), &path)
            .await
            .map_err(|error| format!("Upload path is not allowed: {error}"))?;
        let canonical = path
            .canonicalize()
            .map_err(|_| format!("Upload path does not exist: {}", path.display()))?;
        if canonical.is_file() {
            crate::files_in_workspace::check_file_privacy_for_send(gcx.clone(), &canonical)
                .await
                .map_err(|error| format!("Upload path is blocked by privacy rules: {error}"))?;
        } else if canonical.is_dir() {
            for entry in walkdir::WalkDir::new(&canonical) {
                let entry = entry.map_err(|error| {
                    format!(
                        "Failed to inspect upload directory {}: {error}",
                        canonical.display()
                    )
                })?;
                if entry.file_type().is_symlink() {
                    return Err(format!(
                        "Upload directories cannot contain symbolic links: {}",
                        entry.path().display()
                    ));
                }
                if entry.file_type().is_file() {
                    crate::files_in_workspace::check_file_privacy_for_send(
                        gcx.clone(),
                        &entry.path().to_path_buf(),
                    )
                    .await
                    .map_err(|error| format!("Upload path is blocked by privacy rules: {error}"))?;
                }
            }
        } else {
            return Err(format!(
                "Upload path is not a file or directory: {}",
                canonical.display()
            ));
        }
    }
    Ok(())
}

pub fn execute_steps_with_runtime(
    runtime: &mut BrowserRuntime,
    steps: &[BrowserStep],
    image_policy: &ImagePolicy,
) -> ExecutionReport {
    refact_browser::adopt_new_tabs(runtime, None);
    let initial_tab_ids = runtime.known_tab_ids();
    let mut current_tab: Option<Arc<Tab>> = runtime.get_active_tab();
    if let Some(ref tab) = current_tab {
        let _ = tab.evaluate(INSPECT_ELEMENT_JS, false);
    }

    let mut results: Vec<StepResult> = Vec::new();
    let mut new_tabs = Vec::new();
    let handlers = runtime.locator_handlers.clone();
    let mut locator_handlers = Vec::new();
    let mut all_ok = true;
    let mut pre_step_url: Option<String> = current_tab.as_ref().map(|t| t.get_url());
    let armed_download_waits = steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| match step {
            BrowserStep::WaitForDownload { .. } => Some((index, runtime.download_monitor.cursor())),
            _ => None,
        })
        .collect::<std::collections::HashMap<_, _>>();

    for (idx, step) in steps.iter().enumerate() {
        let step_tab_ids = runtime.known_tab_ids();
        let result = match step {
            BrowserStep::OpenTab { device, url } => match runtime.browser.new_tab() {
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
                    if let Err(error) =
                        crate::integrations::browser_runtime::setup_recording_for_tab(
                            runtime, &new_tab,
                        )
                    {
                        let _ = new_tab.close(false);
                        return ExecutionReport {
                            ok: false,
                            steps: vec![StepResult::failure(idx, "OpenTab", error)],
                            url: current_tab.as_ref().map(|tab| tab.get_url()),
                            title: current_tab.as_ref().and_then(|tab| tab.get_title().ok()),
                            stabilized: false,
                            console: vec![],
                            page_errors: vec![],
                            network: vec![],
                            locator_handlers,
                            dialogs: runtime.dialog_manager.take_reports(),
                            uploads: runtime.file_chooser_manager.take_uploads(),
                            downloads: runtime.download_monitor.take_report(),
                            new_tabs,
                            screenshot: None,
                        };
                    }
                    let navigation_error = url.as_ref().and_then(|url| {
                        new_tab
                            .navigate_to(url)
                            .err()
                            .map(|error| error.to_string())
                    });
                    if navigation_error.is_none() && url.is_some() {
                        let _ = new_tab.wait_until_navigated();
                    }
                    let _ = new_tab.evaluate(INSPECT_ELEMENT_JS, false);
                    current_tab = Some(new_tab);
                    runtime.set_active_tab_target_id(target_id.clone());
                    if let Some(error) = navigation_error {
                        StepResult::failure(idx, "OpenTab", error)
                    } else {
                        StepResult::success(
                            idx,
                            format!(
                                "Opened new {} tab ({})",
                                device_label,
                                &target_id[..8.min(target_id.len())]
                            ),
                        )
                        .with_data(serde_json::json!({"tab_id": target_id}))
                    }
                }
                Err(e) => StepResult::failure(idx, "OpenTab", &format!("Failed: {}", e)),
            },
            BrowserStep::CloseTab { tab: target } => {
                let tab = match target {
                    Some(target) => match resolve_tab(runtime, target) {
                        Ok(tab) => tab,
                        Err(error) => {
                            all_ok = false;
                            results.push(StepResult::failure(idx, "CloseTab", error));
                            break;
                        }
                    },
                    None => match &current_tab {
                        Some(tab) => tab.clone(),
                        None => {
                            all_ok = false;
                            results.push(StepResult::failure(idx, "CloseTab", "No active tab"));
                            break;
                        }
                    },
                };
                let target_id = tab.get_target_id().to_string();
                match tab.close(false) {
                    Ok(_) => {
                        runtime.select_tab_after_close(&target_id);
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
            BrowserStep::WaitForPopup { timeout_ms } => {
                let before = runtime.known_tab_ids();
                let timeout_ms = clamp_timeout_ms(*timeout_ms);
                let deadline = Instant::now() + Duration::from_millis(timeout_ms);
                loop {
                    let adopted = refact_browser::adopt_new_tabs(runtime, Some(idx));
                    if let Some(tab) = adopted.into_iter().find(|tab| !before.contains(&tab.id)) {
                        runtime.set_active_tab_target_id(tab.id.clone());
                        current_tab = runtime.get_active_tab();
                        new_tabs.push(tab.clone());
                        break StepResult::success(idx, format!("Popup opened: {}", tab.id))
                            .with_data(serde_json::json!({"tab_id": tab.id}));
                    }
                    if Instant::now() >= deadline {
                        break StepResult::failure(
                            idx,
                            "Wait for popup",
                            format!("Timed out after {timeout_ms}ms"),
                        );
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
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
            BrowserStep::ExpectFileChooser { paths } => {
                let Some(tab) = &current_tab else {
                    all_ok = false;
                    results.push(StepResult::failure(
                        idx,
                        "Expect file chooser",
                        "No active tab",
                    ));
                    break;
                };
                match runtime.file_chooser_manager.arm(paths).and_then(|_| {
                    tab.set_file_chooser_dialog_interception(true, None)
                        .map_err(|error| error.to_string())
                }) {
                    Ok(()) => StepResult::success(idx, "Armed the next file chooser"),
                    Err(error) => {
                        runtime.file_chooser_manager.disarm();
                        StepResult::failure(idx, "Expect file chooser", error)
                    }
                }
            }
            BrowserStep::WaitForDownload {
                timeout_ms,
                save_as,
            } => match runtime.download_monitor.wait_for_download(
                armed_download_waits
                    .get(&idx)
                    .copied()
                    .unwrap_or_else(|| runtime.download_monitor.cursor()),
                Duration::from_millis(clamp_timeout_ms(*timeout_ms)),
                save_as.as_deref(),
            ) {
                Ok(download) => {
                    StepResult::success(idx, format!("Downloaded {}", download.suggested_filename))
                        .with_data(serde_json::to_value(download).unwrap_or_default())
                }
                Err(error) => StepResult::failure(idx, "Wait for download", error),
            },
            other => match &current_tab {
                Some(tab) => {
                    let chooser_was_armed = runtime.file_chooser_manager.is_armed();
                    let mut result = execute_single_step(
                        tab,
                        &runtime.world_manager,
                        other,
                        idx,
                        pre_step_url.as_deref(),
                        image_policy,
                        Some(&handlers),
                        &mut locator_handlers,
                    );
                    if chooser_was_armed && matches!(other, BrowserStep::Click { .. }) {
                        if result.ok {
                            let completed = runtime
                                .file_chooser_manager
                                .complete(tab, Duration::from_millis(DEFAULT_WAIT_TIMEOUT_MS));
                            let _ = tab.set_file_chooser_dialog_interception(false, None);
                            result = match completed {
                                Ok(upload) => {
                                    let mut completed = StepResult::success(
                                        idx,
                                        format!(
                                            "Selected {} file(s) from file chooser",
                                            upload.paths.len()
                                        ),
                                    )
                                    .with_data(serde_json::to_value(upload).unwrap_or_default());
                                    completed.retries = result.retries;
                                    completed.actionability = result.actionability;
                                    completed
                                }
                                Err(error) => {
                                    StepResult::failure(idx, "File chooser failed", error)
                                }
                            };
                        } else {
                            runtime.file_chooser_manager.disarm();
                            let _ = tab.set_file_chooser_dialog_interception(false, None);
                        }
                    }
                    if result.ok && matches!(other, BrowserStep::SetInputFiles { .. }) {
                        if let Some(data) = result.data.clone() {
                            if let Ok(upload) = serde_json::from_value(data) {
                                runtime.file_chooser_manager.record(upload);
                            }
                        }
                    }
                    result
                }
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
        if result.ok
            && matches!(
                step,
                BrowserStep::Click { .. } | BrowserStep::ClickIfExists { .. }
            )
        {
            new_tabs.extend(refact_browser::wait_for_new_tabs(
                runtime,
                &step_tab_ids,
                Some(idx),
                Duration::from_millis(500),
            ));
        } else {
            new_tabs.extend(refact_browser::adopt_new_tabs(runtime, Some(idx)));
        }
        results.push(result);
    }

    let (url, title) = match &current_tab {
        Some(tab) => (Some(tab.get_url()), tab.get_title().ok()),
        None => (None, None),
    };
    let dialogs = runtime.dialog_manager.take_reports();
    let uploads = runtime.file_chooser_manager.take_uploads();
    let downloads = runtime.download_monitor.take_report();
    refact_browser::adopt_new_tabs(runtime, None);
    new_tabs.extend(
        runtime
            .list_tab_infos()
            .into_iter()
            .filter(|tab| !initial_tab_ids.contains(&tab.id)),
    );
    new_tabs.retain(|tab| !initial_tab_ids.contains(&tab.id));
    let mut seen_new_tabs = std::collections::HashSet::new();
    new_tabs.retain(|tab| seen_new_tabs.insert(tab.id.clone()));
    ExecutionReport {
        ok: all_ok,
        steps: results,
        url,
        title,
        stabilized: false,
        console: vec![],
        page_errors: vec![],
        network: vec![],
        locator_handlers,
        dialogs,
        uploads,
        downloads,
        new_tabs,
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

fn execute_runtime_network_step(
    tab: &Tab,
    world: &WorldManager,
    network_monitor: &NetworkMonitorHandle,
    step: &BrowserStep,
    idx: usize,
    armed_cursor: Option<u64>,
    image_policy: &ImagePolicy,
    handlers: &Arc<Mutex<LocatorHandlerRegistry>>,
    locator_handler_firings: &mut Vec<LocatorHandlerFiring>,
) -> StepResult {
    match step {
        BrowserStep::WaitForNetworkIdle { timeout_ms } => wait_for_load_state(
            network_monitor,
            idx,
            BrowserLoadState::Networkidle,
            clamp_timeout_ms(*timeout_ms),
        ),
        BrowserStep::WaitForLoadState { state, timeout_ms } => {
            wait_for_load_state(network_monitor, idx, *state, clamp_timeout_ms(*timeout_ms))
        }
        BrowserStep::WaitForRequest {
            url_or_pattern,
            timeout_ms,
        } => wait_for_network_entry(
            network_monitor,
            idx,
            url_or_pattern,
            armed_cursor.unwrap_or_else(|| network_monitor.request_cursor()),
            clamp_timeout_ms(*timeout_ms),
            false,
        ),
        BrowserStep::WaitForResponse {
            url_or_pattern,
            timeout_ms,
        } => wait_for_network_entry(
            network_monitor,
            idx,
            url_or_pattern,
            armed_cursor.unwrap_or_else(|| network_monitor.response_cursor()),
            clamp_timeout_ms(*timeout_ms),
            true,
        ),
        _ => execute_single_step(
            tab,
            world,
            step,
            idx,
            None,
            image_policy,
            Some(handlers),
            locator_handler_firings,
        ),
    }
}

fn wait_for_load_state(
    monitor: &NetworkMonitorHandle,
    idx: usize,
    state: BrowserLoadState,
    timeout_ms: u64,
) -> StepResult {
    let state_name = match state {
        BrowserLoadState::Domcontentloaded => "domcontentloaded",
        BrowserLoadState::Load => "load",
        BrowserLoadState::Networkidle => "networkidle",
    };
    let monitor_state = match state {
        BrowserLoadState::Domcontentloaded => NetworkLoadState::Domcontentloaded,
        BrowserLoadState::Load => NetworkLoadState::Load,
        BrowserLoadState::Networkidle => NetworkLoadState::Networkidle,
    };
    match monitor.wait_for_load_state(monitor_state, Duration::from_millis(timeout_ms)) {
        Ok(()) => StepResult::success(idx, format!("Reached load state {state_name}")),
        Err(error) => StepResult::failure(idx, format!("Wait for load state {state_name}"), error),
    }
}

fn wait_for_network_entry(
    monitor: &NetworkMonitorHandle,
    idx: usize,
    pattern: &UrlPattern,
    cursor: u64,
    timeout_ms: u64,
    response: bool,
) -> StepResult {
    let matcher = match pattern {
        UrlPattern::Text(value) => UrlMatcher::text(value),
        UrlPattern::Regex { source, flags } => UrlMatcher::regex(source, flags),
    };
    let matcher = match matcher {
        Ok(matcher) => matcher,
        Err(error) => return StepResult::failure(idx, "Invalid URL pattern", error),
    };
    let result = if response {
        monitor.wait_for_response(&matcher, cursor, Duration::from_millis(timeout_ms))
    } else {
        monitor.wait_for_request(&matcher, cursor, Duration::from_millis(timeout_ms))
    };
    match result {
        Ok(entry) => {
            let kind = if response { "response" } else { "request" };
            StepResult::success(
                idx,
                format!("Matched {kind}: {} {}", entry.method, entry.url),
            )
            .with_data(serde_json::to_value(entry).unwrap_or_default())
        }
        Err(error) => {
            let kind = if response { "response" } else { "request" };
            StepResult::failure(idx, format!("Wait for {kind}"), error)
        }
    }
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
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Timed out while running locator handler click".to_string());
            }
            let result = step_actionable_action_in_mode(
                tab,
                world,
                0,
                &lease.handler.locator,
                "click",
                ActionKind::Click,
                Some(handlers),
                firings,
                image_policy,
                ActionabilityExecutionMode::SkipLocatorHandlers,
                remaining,
            );
            if result.ok {
                Ok(result.summary)
            } else {
                Err(result.error.unwrap_or(result.summary))
            }
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
    if needs_locator_handler_checkpoint(step) && !uses_actionability_engine(step) {
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
        | BrowserStep::CloseTab { .. }
        | BrowserStep::SwitchTab { .. }
        | BrowserStep::ListTabs
        | BrowserStep::WaitForPopup { .. }
        | BrowserStep::HandleDialog { .. } => StepResult::failure(
            idx,
            "Runtime management step",
            "Use execute_steps_with_runtime() for runtime management",
        ),

        BrowserStep::Click { locator } => step_locator_action(
            tab,
            world,
            idx,
            locator,
            "click",
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::ClickIfExists { locator } => step_click_if_exists(
            tab,
            world,
            idx,
            locator,
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::Hover { locator } => step_locator_action(
            tab,
            world,
            idx,
            locator,
            "hover",
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::Focus { locator } => step_locator_action(
            tab,
            world,
            idx,
            locator,
            "focus",
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::Blur { locator } => step_locator_action(
            tab,
            world,
            idx,
            locator,
            "blur",
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::ScrollTo { locator } => step_locator_action(
            tab,
            world,
            idx,
            locator,
            "scroll_to",
            handlers,
            locator_handler_firings,
            image_policy,
        ),
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
        BrowserStep::SetInputFiles { locator, paths } => {
            step_set_input_files(tab, world, idx, locator, paths)
        }
        BrowserStep::ExpectFileChooser { .. } => StepResult::failure(
            idx,
            "Expect file chooser",
            "File chooser flow requires a browser runtime",
        ),

        BrowserStep::WaitForSelector {
            locator,
            timeout_ms,
        } => step_wait_for_selector(tab, world, idx, locator, clamp_timeout_ms(*timeout_ms)),
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
        BrowserStep::WaitForLoadState { .. }
        | BrowserStep::WaitForRequest { .. }
        | BrowserStep::WaitForResponse { .. }
        | BrowserStep::WaitForDownload { .. } => StepResult::failure(
            idx,
            "Network wait",
            "Network waits require a browser runtime",
        ),
        BrowserStep::WaitForElementHidden {
            locator,
            timeout_ms,
        } => step_wait_for_element_hidden(tab, world, idx, locator, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitForElementStable {
            locator,
            timeout_ms,
        } => step_wait_for_element_stable(tab, world, idx, locator, clamp_timeout_ms(*timeout_ms)),
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
        BrowserStep::AccessibilitySnapshot { options } => {
            step_accessibility_snapshot(tab, world, idx, options)
        }
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

fn uses_actionability_engine(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::Click { .. }
            | BrowserStep::ClickIfExists { .. }
            | BrowserStep::Hover { .. }
            | BrowserStep::Focus { .. }
            | BrowserStep::ScrollTo { .. }
    )
}

fn step_set_input_files(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    paths: &[String],
) -> StepResult {
    let resolved = match resolve_element(tab, world, locator) {
        Ok(resolved) => resolved,
        Err(error) => return StepResult::failure(idx, "Set input files: resolution failed", error),
    };
    match refact_browser::files::set_input_files(tab, world, &resolved.handle, paths, "direct") {
        Ok(upload) => StepResult::success(
            idx,
            format!("Set {} file(s) on <input>", upload.paths.len()),
        )
        .with_data(serde_json::to_value(upload).unwrap_or_default()),
        Err(error) => StepResult::failure(idx, "Set input files failed", error),
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
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
) -> StepResult {
    if let Some(action_kind) = match action {
        "click" => Some(ActionKind::Click),
        "hover" => Some(ActionKind::Hover),
        "focus" => Some(ActionKind::Focus),
        "scroll_to" => Some(ActionKind::ScrollIntoViewIfNeeded),
        _ => None,
    } {
        return step_actionable_action(
            tab,
            world,
            idx,
            locator,
            action,
            action_kind,
            handlers,
            locator_handler_firings,
            image_policy,
        );
    }
    match resolve_interactable(tab, world, locator) {
        Ok(info) => {
            let action_js = match action {
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

fn step_actionable_action(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    action: &str,
    action_kind: ActionKind,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
) -> StepResult {
    step_actionable_action_in_mode(
        tab,
        world,
        idx,
        locator,
        action,
        action_kind,
        handlers,
        locator_handler_firings,
        image_policy,
        ActionabilityExecutionMode::Standard,
        ActionabilityTimeouts::default().action,
    )
}

fn step_actionable_action_in_mode(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    action: &str,
    action_kind: ActionKind,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
    mode: ActionabilityExecutionMode,
    timeout: Duration,
) -> StepResult {
    let engine = ActionabilityEngine::new(SystemClock::default(), ActionabilityTimeouts::default());
    let mut driver = BrowserActionDriver::new(
        tab,
        world,
        locator,
        action_kind,
        handlers,
        locator_handler_firings,
        image_policy,
    );
    match engine.execute_with_timeout_in_mode(
        &describe_locator(locator),
        action_kind,
        timeout,
        mode,
        &mut driver,
    ) {
        Ok(success) => {
            let mut result = StepResult::success(
                idx,
                format!(
                    "{action} on <{}> ({})",
                    success.output,
                    describe_locator(locator)
                ),
            );
            result.retries = success.attempts;
            if success.attempts > 0 {
                result.actionability = Some(success.diagnostics(action_kind));
            }
            result
        }
        Err(error) => {
            let diagnostics = error.diagnostics(action_kind);
            let mut result =
                StepResult::failure(idx, format!("{action} failed"), error.to_string());
            result.retries = diagnostics.attempts.unwrap_or_default();
            result.actionability = Some(diagnostics);
            result
        }
    }
}

fn step_click_if_exists(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
) -> StepResult {
    match resolve_element(tab, world, locator) {
        Ok(info) if info.visible => {
            let _ = world.release_handle(tab, &info.handle);
            let result = step_actionable_action(
                tab,
                world,
                idx,
                locator,
                "click",
                ActionKind::Click,
                handlers,
                locator_handler_firings,
                image_policy,
            );
            if result.ok {
                result
            } else {
                StepResult {
                    ok: true,
                    summary: format!(
                        "Click failed (non-fatal): {}",
                        result.error.as_deref().unwrap_or(&result.summary)
                    ),
                    ..result
                }
            }
        }
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
            result.actionability = outcome.actionability;
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
            result.actionability = error.actionability;
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
            result.actionability = outcome.actionability;
            result
        }
        Err(error) => {
            let mut result = StepResult::failure(idx, "Clear failed", error.message);
            result.field_kind = Some(info.field_kind.clone());
            result.retries = error.retries;
            result.actionability = error.actionability;
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
                let mut result =
                    StepResult::success(idx, format!("Selected '{}' in <{}>", value, info.tag))
                        .with_data(serde_json::json!({"selected": outcome.selected}));
                result.actionability = outcome.actionability;
                result
            }
            Err(error) => {
                let mut result = StepResult::failure(idx, "Select option failed", error.message);
                result.retries = error.retries;
                result.actionability = error.actionability;
                result
            }
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
        Ok(outcome) => {
            let mut result = StepResult::success(idx, format!("{}ed <{}>", action, info.tag))
                .with_data(serde_json::json!({
                    "checked": outcome.checked,
                    "changed": outcome.changed,
                    "verified": outcome.verified,
                }));
            result.actionability = outcome.actionability;
            result
        }
        Err(error) => {
            let mut result = StepResult::failure(idx, format!("{} failed", action), error.message);
            result.retries = error.retries;
            result.actionability = error.actionability;
            result
        }
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

fn wait_for_report_stability(
    tab: &Tab,
    world: &WorldManager,
    network_monitor: &NetworkMonitorHandle,
    timeout_ms: u64,
) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    if poll_until_deadline(tab, "document.readyState !== 'loading'", deadline).is_err() {
        return false;
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if network_monitor
        .wait_for_load_state(NetworkLoadState::Networkidle, remaining)
        .is_err()
    {
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

fn mask_console_entry(
    mut entry: refact_integrations::browser_types::ConsoleEntry,
) -> refact_integrations::browser_types::ConsoleEntry {
    entry.text = refact_core::string_utils::redact_sensitive(&entry.text);
    entry
}

fn step_wait_for_selector(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    timeout_ms: u64,
) -> StepResult {
    match poll_locator_until(tab, world, locator, timeout_ms, |handles| {
        if handles.is_empty() {
            Ok(None)
        } else {
            let handle = strict_locator_handle(tab, world, locator, handles)?;
            Ok(handle.map(|handle| {
                let _ = world.release_handle(tab, &handle);
            }))
        }
    }) {
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
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    timeout_ms: u64,
) -> StepResult {
    match poll_locator_until(tab, world, locator, timeout_ms, |handles| {
        let Some(handle) = strict_locator_handle(tab, world, locator, handles)? else {
            return Ok(Some(()));
        };
        let hidden = world
            .call_function_on(
                tab,
                &handle,
                "function() { const r = this.getBoundingClientRect(); return r.width === 0 || r.height === 0; }",
                Vec::new(),
            )
            .map_err(|error| error.to_string());
        let _ = world.release_handle(tab, &handle);
        hidden.map(|hidden| (hidden == serde_json::Value::Bool(true)).then_some(()))
    }) {
        Ok(()) => StepResult::success(idx, "Element is hidden".to_string()),
        Err(e) => StepResult::failure(idx, "Wait for element hidden", e),
    }
}

fn step_wait_for_element_stable(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    timeout_ms: u64,
) -> StepResult {
    let mut previous = None;
    match poll_locator_until(tab, world, locator, timeout_ms, |handles| {
        let Some(handle) = strict_locator_handle(tab, world, locator, handles)? else {
            previous = None;
            return Ok(None);
        };
        let bbox = world.call_function_on(
            tab,
            &handle,
            "function() { const r = this.getBoundingClientRect(); return {x: r.x, y: r.y, w: r.width, h: r.height}; }",
            Vec::new(),
        );
        let _ = world.release_handle(tab, &handle);
        let bbox = bbox.map_err(|error| error.to_string())?;
        let stable = previous.as_ref() == Some(&bbox);
        previous = Some(bbox);
        Ok(stable.then_some(()))
    }) {
        Ok(()) => StepResult::success(idx, "Element is stable".to_string()),
        Err(error) => StepResult::failure(idx, "Wait for element stable", error),
    }
}

fn poll_locator_until<T>(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
    timeout_ms: u64,
    mut sample: impl FnMut(Vec<ElementHandle>) -> Result<Option<T>, String>,
) -> Result<T, String> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match resolve_locator_handles(tab, world, locator).and_then(&mut sample) {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            return Err(format!("Timed out after {}ms", timeout_ms));
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
        Some(locator) => serde_json::to_value(locator)
            .map_err(|error| format!("Failed to serialize browser locator: {error}"))
            .and_then(|locator| {
                world.call_injected(
                    tab,
                    "extractLinks",
                    serde_json::json!([locator, effective_limit]),
                )
            }),
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

fn step_accessibility_snapshot(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    options: &AccessibilitySnapshotOptions,
) -> StepResult {
    let root = match options.root.as_ref() {
        Some(locator) => match resolve_element(tab, world, locator) {
            Ok(resolved) => Some(resolved.handle),
            Err(error) => {
                return StepResult::failure(
                    idx,
                    "Accessibility snapshot root resolution failed",
                    error,
                )
            }
        },
        None => None,
    };
    let snapshot_options = SnapshotOptions {
        mode: match options.mode {
            AccessibilitySnapshotMode::Ai => SnapshotMode::Ai,
            AccessibilitySnapshotMode::Default => SnapshotMode::Default,
        },
        refs: options.refs_enabled(),
        boxes: options.boxes,
        ..Default::default()
    };
    let snapshot = world.aria_snapshot(tab, root.clone(), snapshot_options);
    if let Some(root) = root {
        let _ = world.release_handle(tab, &root);
    }
    match snapshot {
        Ok(mut snapshot) => {
            let limit = options
                .max_chars
                .unwrap_or(DEFAULT_ARIA_SNAPSHOT_CHARS)
                .min(MAX_ARIA_SNAPSHOT_CHARS);
            snapshot.yaml = truncate_chars(snapshot.yaml, limit);
            StepResult::success(idx, "Accessibility snapshot").with_data(
                serde_json::to_value(snapshot)
                    .unwrap_or_else(|error| serde_json::json!({"error": error.to_string()})),
            )
        }
        Err(error) => StepResult::failure(idx, "Accessibility snapshot failed", error.to_string()),
    }
}

fn truncate_chars(value: String, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value;
    }
    const SUFFIX: &str = "\n# ... (truncated)";
    let suffix_chars = SUFFIX.chars().count();
    if max_chars <= suffix_chars {
        return value.chars().take(max_chars).collect();
    }
    let prefix = value
        .chars()
        .take(max_chars - suffix_chars)
        .collect::<String>();
    let mut truncated = prefix
        .rsplit_once('\n')
        .map(|(complete, _)| complete.to_string())
        .unwrap_or(prefix);
    truncated.push_str(SUFFIX);
    truncated
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
    let mut description = match &locator.strategy {
        LocatorStrategy::Ref { value } => format!("ref={}", value),
        LocatorStrategy::Css { value } => format!("css={}", value),
        LocatorStrategy::Id { value } => format!("id={}", value),
        LocatorStrategy::Name { value } => format!("name={}", value),
        LocatorStrategy::TestId { value, .. } => format!("testid={}", value),
        LocatorStrategy::Placeholder { value, .. } => format!("placeholder={}", value),
        LocatorStrategy::AltText { value, .. } => format!("alt_text={}", value),
        LocatorStrategy::Title { value, .. } => format!("title={}", value),
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
    };
    if let Some(inner) = &locator.locator {
        description = format!("{description}.locator({})", describe_locator(inner));
    }
    if locator.filter.is_some() {
        description.push_str(".filter(...)");
    }
    if let Some(other) = &locator.and {
        description = format!("{description}.and({})", describe_locator(other));
    }
    if let Some(other) = &locator.or {
        description = format!("{description}.or({})", describe_locator(other));
    }
    if locator.first == Some(true) {
        description.push_str(".first()");
    } else if locator.last == Some(true) {
        description.push_str(".last()");
    } else if let Some(index) = locator.nth {
        description.push_str(&format!(".nth({index})"));
    }
    for frame in locator.frames.iter().rev() {
        description = format!("frame({}).{description}", describe_locator(frame));
    }
    description
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intercepting_preview_excludes_failure_suffix() {
        assert_eq!(
            intercepts_pointer_events("<div class=overlay> intercepts pointer events".to_string()),
            ActionabilityDiagnostic::InterceptsPointerEvents {
                description: "<div class=overlay>".to_string(),
            }
        );
    }

    #[test]
    fn test_describe_locator_css() {
        let loc = BrowserLocator::css("#btn");
        assert_eq!(describe_locator(&loc), "css=#btn");
    }

    #[test]
    fn test_describe_locator_ref() {
        let locator = BrowserLocator::reference("f2e7");
        assert_eq!(describe_locator(&locator), "ref=f2e7");
    }

    #[test]
    fn aria_snapshot_truncation_preserves_complete_yaml_lines() {
        let value = "- button \"Save\"\n- textbox \"Search\"\n- link \"Guide\"".to_string();
        let truncated = truncate_chars(value, 40);
        assert!(truncated.ends_with("# ... (truncated)"));
        assert!(!truncated.contains("- textbox \"Sear"));
        assert!(truncated.chars().count() <= 40);
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
            frames: Vec::new(),
            nth: None,
            within: None,
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
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
            frames: Vec::new(),
            nth: None,
            within: None,
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
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
            frames: Vec::new(),
            nth: None,
            within: None,
            locator: None,
            filter: None,
            and: None,
            or: None,
            first: None,
            last: None,
        };
        assert_eq!(describe_locator(&loc), "xpath=//button");
    }

    #[test]
    fn describe_locator_includes_outermost_first_frame_chain() {
        let locator = BrowserLocator::role("button", Some("Save")).in_frames(vec![
            BrowserLocator::css("#outer"),
            BrowserLocator::role("iframe", Some("Editor")),
        ]);

        assert_eq!(
            describe_locator(&locator),
            "frame(css=#outer).frame(role=iframe[Editor]).role=button[Save]"
        );
    }

    #[test]
    fn injected_wait_payload_preserves_role_composition_without_frame_metadata() {
        let mut locator = BrowserLocator::role("button", Some("Save"))
            .in_frames(vec![BrowserLocator::css("#editor-frame")]);
        locator.locator = Some(Box::new(BrowserLocator::css("svg")));

        let payload = locator_without_frames(&locator).unwrap();

        assert_eq!(payload["by"], "role");
        assert_eq!(payload["name"], "Save");
        assert_eq!(
            payload["locator"],
            serde_json::json!({"by": "css", "value": "svg"})
        );
        assert!(payload.get("frames").is_none());
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
