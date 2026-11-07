use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use base64::Engine;
use headless_chrome::Tab;
use headless_chrome::protocol::cdp::{DOM, Emulation, IO, Page, Runtime, types::Event};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::integrations::browser_locators::{
    self, js_string_literal, parse_element_info, INSPECT_ELEMENT_JS,
};
use crate::integrations::browser_models::*;
use crate::integrations::browser_runtime::BrowserRuntime;
use crate::integrations::browser_types::{ConsoleEntry, NetworkEntry};
use refact_browser::{
    ActionKind, ActionabilityDiagnostic, ActionabilityDriver, ActionabilityEngine,
    ActionabilityExecutionMode, ActionabilityTimeouts, CDP_INLINE_RESULT_LIMIT_BYTES,
    CdpDragObserver, CdpGuardrail, CdpKeyboardDispatcher, CdpMouseDispatcher, ClockOp,
    DEFAULT_DISMISS_OVERLAYS_HANDLER, ElementHandle, ElementStateName, ExpectPollResult,
    FUNCTION_POLL_BACKOFF_MS, HitTargetController, HitTargetPoint, HitTargetResult, Keyboard,
    KeyboardDispatcher, LocatorGenerationOptions, LocatorHandler, LocatorHandlerLease,
    LocatorHandlerOperation, LocatorHandlerProbe, LocatorHandlerRegistry, LocatorOutcome,
    MainFrameCssPoint, Mouse, MouseButton, MouseDispatcher, MouseState, NetworkLoadState,
    NetworkMonitorHandle, NetworkWaitFilters, Ref, ScrollStrategy, SnapshotMode, SnapshotOptions,
    SystemClock, TabScopedHandle, UrlMatcher, WebSocketRegistry, WorldManager,
    apply_network_report_mode, classify_cdp_command, current_wall_ms, parse_clock_ticks,
    parse_clock_time, redact_cdp_result, required_states,
};
use refact_browser::artifacts::{
    ComposeLayout, ElementStateAction, ScreenshotMetrics, compose_sheet, element_state_sequence,
    pdf_payload, screenshot_capture,
};
use refact_browser::screencast::{
    build_filmstrip, capture_screencast_burst, capture_timed_frames_within, timed_capture_budget,
    select_evenly_spaced, CapturedFrame, FilmstripResult, FrameBurstPlan, ScreencastSessionOptions,
    DEFAULT_SCREENCAST_QUALITY, MAX_FRAME_COUNT, MAX_SESSION_DURATION_MS, MAX_SESSION_FRAMES,
};
use refact_browser::http_client;
use refact_core::image_policy::{resize_to_policy, ImageFormat, ImagePolicy};

use crate::global_context::GlobalContext;
use crate::http::routers::v1::browser_settings;

#[allow(dead_code)]
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 5_000;
#[allow(dead_code)]
const MAX_WAIT_TIMEOUT_MS: u64 = 60_000;
const MAX_WAIT_SECONDS: f64 = 60.0;
const MIN_WAIT_SECONDS: f64 = 0.0;
const NAVIGATION_LIFECYCLE_EVENT: &str = "load";

#[allow(dead_code)]
const MAX_DOM_SNAPSHOT_CHARS: usize = 100_000;
#[allow(dead_code)]
const MAX_EXTRACT_LINKS: usize = 500;
#[allow(dead_code)]
const MAX_EXTRACT_TABLE_ROWS: usize = 100;
#[allow(dead_code)]
const DEFAULT_ALL_TEXTS: usize = 50;
const MAX_ALL_TEXTS: usize = 500;
#[allow(dead_code)]
const DEFAULT_ARIA_SNAPSHOT_CHARS: usize = 20_000;
#[allow(dead_code)]
const MAX_ARIA_SNAPSHOT_CHARS: usize = 100_000;
#[allow(dead_code)]
const MAX_INLINE_SNAPSHOT_BYTES: usize = 6 * 1024;
const SNAPSHOT_SUMMARY_LINES: usize = 40;

const TAP_AMBIGUOUS_TARGET: &str = "tap requires either a locator or both x and y";
const TAP_REQUIRES_TOUCH: &str = "The page does not support tap: enable touch emulation first with a set_viewport step that sets has_touch to true";

fn clamp_timeout_ms(requested: Option<u64>) -> u64 {
    let settings = browser_settings::current();
    requested
        .unwrap_or(settings.timing.default_wait_timeout_ms)
        .min(settings.timing.max_wait_timeout_ms)
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

#[allow(dead_code)]
const DEFAULT_POLL_INTERVAL_MS: u64 = 200;
const REPORT_STABILIZATION_TIMEOUT_MS: u64 = 3_000;
const REPORT_STABILITY_INTERVAL_MS: u64 = 200;
const CONSOLE_POLL_INTERVAL_MS: u64 = 50;

pub fn resolve_tab(runtime: &BrowserRuntime, target: &TabTarget) -> Result<Arc<Tab>, String> {
    match target {
        TabTarget::Active => runtime
            .get_active_tab()
            .ok_or_else(|| "No active tab in browser runtime".to_string()),
        TabTarget::Id {
            id,
        } => {
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

pub fn session_tab(runtime: &mut BrowserRuntime) -> Result<Arc<Tab>, String> {
    if let Some(reason) = &runtime.recovery_fence {
        return Err(crate::integrations::browser_runtime::recovery_fence_error(
            reason,
        ));
    }
    refact_browser::adopt_new_tabs(runtime, None);
    resolve_tab(runtime, &TabTarget::Active)
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

/// Same contract as `eval_js_ok`, but the script runs in the isolated utility world: pages
/// that override `JSON.stringify`, `Array.prototype` or other globals cannot poison the
/// engine's own probes there, while the DOM stays fully readable.
fn eval_js_ok_in_utility(
    tab: &Tab,
    world: &WorldManager,
    js: &str,
) -> Result<serde_json::Value, String> {
    let value = world.eval_in_utility(tab, js)?;
    let result = match value.as_str() {
        Some(text) => serde_json::from_str(text)
            .map_err(|error| format!("Failed to parse JS JSON result: {error}"))?,
        None if value.is_object() || value.is_array() => value,
        None => return Err(format!("Unexpected JS result type: {value:?}")),
    };
    if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
        return Err(err.to_string());
    }
    Ok(result)
}

struct ResolvedElement {
    handle: ElementHandle,
    info: ElementInfo,
}

enum ResolveElementError {
    MultipleMatches { count: usize, previews: Vec<String> },
    Other(String),
}

impl ResolveElementError {
    fn into_message(self, locator: &BrowserLocator) -> String {
        match self {
            Self::MultipleMatches {
                count,
                previews,
            } => {
                refact_browser::strict_mode_violation(&describe_locator(locator), count, &previews)
            }
            Self::Other(message) => message,
        }
    }
}

fn locator_outcome_for_resolve_error(strategy: &LocatorStrategy, error: String) -> LocatorOutcome {
    if matches!(strategy, LocatorStrategy::Ref { .. })
        || refact_browser::is_transport_dead_error(&error)
    {
        LocatorOutcome::Error {
            description: error,
        }
    } else {
        LocatorOutcome::NotFound
    }
}

impl std::ops::Deref for ResolvedElement {
    type Target = ElementInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

struct ScopedElement<'a> {
    handle: TabScopedHandle<'a>,
    info: ElementInfo,
}

impl std::ops::Deref for ScopedElement<'_> {
    type Target = ElementInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

fn resolve_scoped_element<'a>(
    tab: &'a Tab,
    world: &'a WorldManager,
    locator: &BrowserLocator,
) -> Result<ScopedElement<'a>, String> {
    let resolved = resolve_element(tab, world, locator)?;
    Ok(ScopedElement {
        handle: world.wrap_scoped(tab, resolved.handle),
        info: resolved.info,
    })
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
    locator_echo: Option<String>,
    pointer_attempt: usize,
}

struct DragActionabilityDriver<'a> {
    tab: &'a Tab,
    world: &'a WorldManager,
    locator: &'a BrowserLocator,
    handlers: Option<&'a Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &'a mut Vec<LocatorHandlerFiring>,
    image_policy: &'a ImagePolicy,
    precheck_deadline: Instant,
    resolved: Option<ResolvedElement>,
    position: Option<BrowserPosition>,
}

impl Drop for DragActionabilityDriver<'_> {
    fn drop(&mut self) {
        if let Some(resolved) = self.resolved.take() {
            let _ = self.world.release_handle(self.tab, &resolved.handle);
        }
    }
}

impl ActionabilityDriver for DragActionabilityDriver<'_> {
    type Output = (String, ElementHandle, MainFrameCssPoint);

    fn resolve(&mut self) -> LocatorOutcome {
        if let Some(resolved) = self.resolved.take() {
            let _ = self.world.release_handle(self.tab, &resolved.handle);
        }
        match resolve_element_typed(self.tab, self.world, self.locator) {
            Ok(resolved) => {
                let preview = element_preview(self.tab, self.world, &resolved.handle);
                self.resolved = Some(resolved);
                LocatorOutcome::Found {
                    preview,
                }
            }
            Err(ResolveElementError::MultipleMatches {
                count,
                previews,
            }) => LocatorOutcome::MultipleMatches {
                count,
                previews,
            },
            Err(ResolveElementError::Other(error)) => {
                locator_outcome_for_resolve_error(&self.locator.strategy, error)
            }
        }
    }

    fn element_state(&mut self) -> Result<refact_browser::ElementState, ActionabilityDiagnostic> {
        let resolved = self
            .resolved
            .as_ref()
            .ok_or(ActionabilityDiagnostic::Detached)?;
        self.world
            .element_states(self.tab, &resolved.handle)
            .map_err(|error| ActionabilityDiagnostic::PrecheckFailed {
                description: error.to_string(),
            })
    }

    fn perform(&mut self) -> Result<Self::Output, ActionabilityDiagnostic> {
        let resolved = self
            .resolved
            .as_ref()
            .ok_or(ActionabilityDiagnostic::Detached)?;
        let point =
            element_drag_point(self.tab, &resolved.handle, self.position).map_err(|error| {
                ActionabilityDiagnostic::PrecheckFailed {
                    description: error.to_string(),
                }
            })?;
        match HitTargetController::default().expect_hit_target(
            self.tab,
            self.world,
            &resolved.handle,
            HitTargetPoint {
                x: point.x,
                y: point.y,
            },
        ) {
            Ok(HitTargetResult::Done) => {
                let resolved = self.resolved.take().unwrap();
                Ok((resolved.info.tag, resolved.handle, point))
            }
            Ok(HitTargetResult::Intercepted {
                description,
            }) => Err(intercepts_pointer_events(description)),
            Ok(HitTargetResult::NotConnected) => Err(ActionabilityDiagnostic::Detached),
            Ok(HitTargetResult::Skipped) => Err(ActionabilityDiagnostic::PrecheckFailed {
                description: "Browser drag hit-target check was skipped".to_string(),
            }),
            Err(error) => Err(ActionabilityDiagnostic::PrecheckFailed {
                description: error.to_string(),
            }),
        }
    }

    fn wait_for_navigation(&mut self) -> Result<(), ActionabilityDiagnostic> {
        wait_for_pending_navigation(self.tab, self.precheck_deadline).map_err(|description| {
            ActionabilityDiagnostic::PrecheckFailed {
                description,
            }
        })
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
        .map_err(|description| ActionabilityDiagnostic::PrecheckFailed {
            description,
        })
    }
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
        timeout: Duration,
    ) -> Self {
        let settings = browser_settings::current();
        Self {
            tab,
            world,
            locator,
            action,
            handlers,
            locator_handler_firings,
            image_policy,
            precheck_deadline: Instant::now()
                + timeout.min(Duration::from_millis(
                    settings.timing.default_wait_timeout_ms,
                )),
            resolved: None,
            locator_echo: None,
            pointer_attempt: 0,
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
        match resolve_element_typed(self.tab, self.world, self.locator) {
            Ok(resolved) => {
                let preview = element_preview(self.tab, self.world, &resolved.handle);
                self.resolved = Some(resolved);
                LocatorOutcome::Found {
                    preview,
                }
            }
            Err(ResolveElementError::MultipleMatches {
                count,
                previews,
            }) => LocatorOutcome::MultipleMatches {
                count,
                previews,
            },
            Err(ResolveElementError::Other(error)) => {
                locator_outcome_for_resolve_error(&self.locator.strategy, error)
            }
        }
    }

    fn element_state(&mut self) -> Result<refact_browser::ElementState, ActionabilityDiagnostic> {
        let resolved = self.resolved()?;
        self.world
            .element_states(self.tab, &resolved.handle)
            .map_err(|error| match error {
                refact_browser::HandleError::Invalidated {
                    ..
                } => ActionabilityDiagnostic::Detached,
                _ => ActionabilityDiagnostic::PrecheckFailed {
                    description: error.to_string(),
                },
            })
    }

    fn perform(&mut self) -> Result<Self::Output, ActionabilityDiagnostic> {
        self.locator_echo = self
            .resolved
            .as_ref()
            .and_then(|resolved| generate_locator_echo(self.tab, self.world, &resolved.handle));
        let scroll_strategy = ScrollStrategy::for_attempt(self.pointer_attempt);
        self.pointer_attempt += 1;
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
            .map_err(|description| ActionabilityDiagnostic::PrecheckFailed {
                description,
            });
        }
        if self.action == ActionKind::ScrollIntoViewIfNeeded {
            return dispatcher
                .scroll_into_view(&resolved.handle, ScrollStrategy::Protocol)
                .map(|_| resolved.info.tag.clone())
                .map_err(|error| ActionabilityDiagnostic::PrecheckFailed {
                    description: error.to_string(),
                });
        }
        if let Err(error) = dispatcher.scroll_into_view(&resolved.handle, scroll_strategy) {
            if refact_browser::is_transport_dead_error(&error.to_string()) {
                return Err(ActionabilityDiagnostic::PrecheckFailed {
                    description: error.to_string(),
                });
            }
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
            Ok(HitTargetResult::Intercepted {
                description,
            }) => {
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
            ActionKind::Tap => mouse.tap(point.x, point.y),
            _ => unreachable!(),
        };
        let hit_result = hit_target.take_result(self.tab, self.world, token);
        action_result.map_err(|error| ActionabilityDiagnostic::EffectUnknown {
            description: error.to_string(),
        })?;
        match hit_result {
            Ok(HitTargetResult::Done | HitTargetResult::Skipped) => Ok(resolved.info.tag.clone()),
            Ok(HitTargetResult::Intercepted {
                description,
            }) => Err(intercepts_pointer_events(description)),
            Ok(HitTargetResult::NotConnected) => Err(ActionabilityDiagnostic::EffectUnknown {
                description: "Pointer dispatch completed but target disconnected; effect unknown"
                    .to_string(),
            }),
            Err(error) => Err(ActionabilityDiagnostic::EffectUnknown {
                description: error.to_string(),
            }),
        }
    }

    fn wait_for_navigation(&mut self) -> Result<(), ActionabilityDiagnostic> {
        wait_for_pending_navigation(self.tab, self.precheck_deadline).map_err(|description| {
            ActionabilityDiagnostic::PrecheckFailed {
                description,
            }
        })
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
        .map_err(|description| ActionabilityDiagnostic::PrecheckFailed {
            description,
        })
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

fn generate_locator_echo(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
) -> Option<String> {
    world
        .generate_locator(tab, handle, LocatorGenerationOptions::default())
        .ok()
        .filter(|locator| !locator.is_empty())
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

fn resolve_element(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
) -> Result<ResolvedElement, String> {
    resolve_element_typed(tab, world, locator).map_err(|error| error.into_message(locator))
}

fn resolve_element_typed(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
) -> Result<ResolvedElement, ResolveElementError> {
    if locator_supports_count_precheck(locator) {
        let value = locator_without_frames(locator).map_err(ResolveElementError::Other)?;
        match world.count_injected_matches(tab, value, STRICT_PREVIEW_LIMIT) {
            Ok((0, _)) => {
                return Err(ResolveElementError::Other("Element not found".to_string()));
            }
            Ok((count, previews)) if count > 1 => {
                return Err(ResolveElementError::MultipleMatches {
                    count,
                    previews,
                });
            }
            Ok(_) | Err(_) => {}
        }
    }
    let handles =
        resolve_locator_handles(tab, world, locator).map_err(ResolveElementError::Other)?;
    let handle = match strict_locator_handle_result(tab, world, handles) {
        StrictLocatorHandle::None => {
            return Err(ResolveElementError::Other("Element not found".to_string()));
        }
        StrictLocatorHandle::One(handle) => handle,
        StrictLocatorHandle::Multiple {
            count,
            previews,
        } => {
            return Err(ResolveElementError::MultipleMatches {
                count,
                previews,
            });
        }
    };
    inspect_resolved_element(tab, world, handle).map_err(ResolveElementError::Other)
}

const STRICT_PREVIEW_LIMIT: usize = 5;

/// Ref locators resolve through the ref registry and frame-scoped locators through the frame
/// resolver; every other locator can be counted by value first, so strict-mode ambiguity
/// over hundreds of matches costs one round trip instead of one handle release per match.
fn locator_supports_count_precheck(locator: &BrowserLocator) -> bool {
    !matches!(locator.strategy, LocatorStrategy::Ref { .. }) && locator.frames.is_empty()
}

pub fn resolve_locator_element(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
) -> Result<ElementHandle, String> {
    resolve_element(tab, world, locator).map(|resolved| resolved.handle)
}

fn resolve_locator_handles(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
) -> Result<Vec<ElementHandle>, String> {
    match &locator.strategy {
        LocatorStrategy::Ref {
            value,
        } => {
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
            Ok(vec![
                world
                    .resolve_ref(tab, &reference)
                    .map_err(|error| error.to_string())?,
            ])
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
                    .map_err(|error| frame_locator_error_message(locator, error))
            }
        }
    }
}

/// Names the frame that went stale with the locator the caller wrote, so a replaced iframe reads as
/// a re-runnable condition instead of a raw CDP failure.
fn frame_locator_error_message(
    locator: &BrowserLocator,
    error: refact_browser::FrameLocatorError,
) -> String {
    match error {
        refact_browser::FrameLocatorError::FrameDetached {
            index,
            source,
        } => {
            let summary = locator.frames.get(index).map(describe_locator);
            refact_browser::frame_detached_message(index, summary.as_deref(), source.as_deref())
        }
        refact_browser::FrameLocatorError::FrameOwnerMissing {
            index,
        } => {
            let summary = locator.frames.get(index).map(describe_locator);
            refact_browser::frame_owner_missing_message(index, summary.as_deref())
        }
        error => error.to_string(),
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
    handles: Vec<ElementHandle>,
) -> Result<Option<ElementHandle>, String> {
    match strict_locator_handle_result(tab, world, handles) {
        StrictLocatorHandle::None => Ok(None),
        StrictLocatorHandle::One(handle) => Ok(Some(handle)),
        StrictLocatorHandle::Multiple {
            count,
            previews,
        } => Err(refact_browser::strict_mode_violation(
            &describe_locator(locator),
            count,
            &previews,
        )),
    }
}

enum StrictLocatorHandle {
    None,
    One(ElementHandle),
    Multiple { count: usize, previews: Vec<String> },
}

fn strict_locator_handle_result(
    tab: &Tab,
    world: &WorldManager,
    mut handles: Vec<ElementHandle>,
) -> StrictLocatorHandle {
    if handles.len() > 1 {
        let count = handles.len();
        let previews = strict_locator_previews(tab, world, &handles);
        release_locator_handles(tab, world, &handles);
        return StrictLocatorHandle::Multiple {
            count,
            previews,
        };
    }
    handles
        .pop()
        .map(StrictLocatorHandle::One)
        .unwrap_or(StrictLocatorHandle::None)
}

fn strict_locator_previews<'h>(
    tab: &Tab,
    world: &WorldManager,
    handles: impl IntoIterator<Item = &'h ElementHandle>,
) -> Vec<String> {
    handles
        .into_iter()
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
        .collect()
}

fn release_locator_handles(tab: &Tab, world: &WorldManager, handles: &[ElementHandle]) {
    for handle in handles {
        let _ = world.release_handle(tab, handle);
    }
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
    Ok(ResolvedElement {
        handle,
        info,
    })
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

/// Executes a JavaScript function declaration with the element handle bound as `this`.
///
/// Callers must pass the declaration itself, never an already-invoked function expression.
fn call_handle_json(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    function_declaration: &str,
) -> Result<serde_json::Value, String> {
    let value = world
        .call_function_on(tab, handle, function_declaration, Vec::new())
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
    let mut mouse_state = MouseState::default();

    for (idx, step) in steps.iter().enumerate() {
        let mut result = execute_single_step(
            tab,
            world,
            step,
            idx,
            pre_step_url.as_deref(),
            image_policy,
            Some(&handlers),
            &mut locator_handlers,
            &mut mouse_state,
        );
        if step_failure_is_fatal(step, &mut result) {
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
    mark_unexecuted_steps_skipped(&mut results, steps.len());

    let _ = world.release_all(tab);

    ExecutionReport {
        session_generation: None,
        recovery: transport_recovery(&results),
        ok: all_ok,
        steps: results,
        warnings: Vec::new(),
        url: Some(tab.get_url()),
        title: tab.get_title().ok(),
        page: None,
        stabilized: false,
        console: vec![],
        page_errors: vec![],
        network: vec![],
        network_summary: vec![],
        websockets: vec![],
        locator_handlers,
        dialogs: vec![],
        uploads: vec![],
        downloads: vec![],
        new_tabs: vec![],
        active_routes: vec![],
        intercepted_requests: vec![],
        context: None,
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

fn is_context_management_step(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::SetViewport { .. }
            | BrowserStep::EmulateMedia { .. }
            | BrowserStep::SetLocale { .. }
            | BrowserStep::SetTimezone { .. }
            | BrowserStep::SetUserAgent { .. }
            | BrowserStep::SetGeolocation { .. }
            | BrowserStep::SetOffline { .. }
            | BrowserStep::SetNetworkConditions { .. }
            | BrowserStep::SetCpuThrottling { .. }
            | BrowserStep::EmulateDevice { .. }
            | BrowserStep::ListDevices { .. }
            | BrowserStep::SetExtraHttpHeaders { .. }
            | BrowserStep::GetCookies { .. }
            | BrowserStep::SetCookies { .. }
            | BrowserStep::ClearCookies { .. }
            | BrowserStep::GetStorage { .. }
            | BrowserStep::SetStorage { .. }
            | BrowserStep::ClearStorage { .. }
            | BrowserStep::StorageState { .. }
            | BrowserStep::SetStorageState { .. }
            | BrowserStep::GrantPermissions { .. }
            | BrowserStep::ClearPermissions
            | BrowserStep::SetHttpCredentials { .. }
    )
}

fn download_step_result(idx: usize, download: Result<DownloadInfo, String>) -> StepResult {
    match download {
        Ok(download) if download.state == DownloadState::Canceled => StepResult::failure(
            idx,
            "Wait for download",
            format!(
                "Download failed ({}): {}",
                download
                    .failure_reason
                    .clone()
                    .unwrap_or_else(|| "canceled".to_string()),
                download.suggested_filename
            ),
        )
        .with_data(serde_json::to_value(download).unwrap_or_default()),
        Ok(download) => {
            StepResult::success(idx, format!("Downloaded {}", download.suggested_filename))
                .with_data(serde_json::to_value(download).unwrap_or_default())
        }
        Err(error) => StepResult::failure(idx, "Wait for download", error),
    }
}

fn cancel_download_step_result(idx: usize, download: Result<DownloadInfo, String>) -> StepResult {
    match download {
        Ok(download) => StepResult::success(
            idx,
            format!("Canceled download {}", download.suggested_filename),
        )
        .with_data(serde_json::to_value(download).unwrap_or_default()),
        Err(error) => StepResult::failure(idx, "Cancel download", error),
    }
}

fn apply_permission_state(
    current: &[String],
    permissions: &[String],
    state: BrowserPermissionState,
) -> Vec<String> {
    let mut granted = current
        .iter()
        .filter(|permission| !permissions.contains(permission))
        .cloned()
        .collect::<Vec<_>>();
    if state.is_granted() {
        granted.extend(permissions.iter().cloned());
    }
    granted
}

fn open_tab_for_device(
    runtime: &BrowserRuntime,
    device: &Option<String>,
) -> Result<(&'static refact_browser::DeviceDescriptor, Arc<Tab>), String> {
    let descriptor = refact_browser::devices::lookup(device.as_deref().unwrap_or("desktop"))?;
    let tab = runtime
        .browser
        .new_tab()
        .map_err(|error| format!("Failed: {error}"))?;
    Ok((descriptor, tab))
}

fn network_conditions_summary(
    offline: bool,
    conditions: Option<&refact_browser::NetworkConditions>,
) -> String {
    let throttling = conditions.map_or_else(
        || "no throttling".to_string(),
        refact_browser::NetworkConditions::summary,
    );
    format!(
        "Network conditions: {}, {throttling}",
        if offline { "offline" } else { "online" }
    )
}

fn apply_context_to_tabs(
    runtime: &mut BrowserRuntime,
    candidate: &refact_browser::ContextState,
) -> Result<(), String> {
    let tabs = runtime
        .browser
        .get_tabs()
        .lock()
        .map_err(|error| format!("Failed to list intended tabs: {error}"))?
        .clone();
    runtime.context_state.apply_candidate(
        candidate.clone(),
        &mut runtime.context_state_dirty,
        |candidate| {
            for tab in tabs {
                candidate.apply_to_tab(&tab)?;
            }
            Ok(())
        },
    )
}

fn context_summary(runtime: &BrowserRuntime) -> BrowserContextSummary {
    let Some(tab) = runtime.get_active_tab() else {
        return runtime.context_state.summary(0, 0, 0);
    };
    let cookies = refact_browser::context_state::get_cookies(&tab, None)
        .map(|cookies| cookies.len())
        .unwrap_or_default();
    let local = refact_browser::context_state::get_storage(&tab, BrowserStorageKind::Local, None)
        .map(|items| items.len())
        .unwrap_or_default();
    let session =
        refact_browser::context_state::get_storage(&tab, BrowserStorageKind::Session, None)
            .map(|items| items.len())
            .unwrap_or_default();
    runtime.context_state.summary(cookies, local, session)
}

fn execute_context_management_step(
    runtime: &mut BrowserRuntime,
    step: &BrowserStep,
    idx: usize,
) -> StepResult {
    let mut candidate = runtime.context_state.clone();
    let result: Result<StepResult, String> = (|| match step {
        BrowserStep::SetViewport {
            width,
            height,
            device_scale_factor,
            is_mobile,
            has_touch,
        } => {
            candidate.viewport = Some(refact_browser::ViewportState {
                width: *width,
                height: *height,
                device_scale_factor: device_scale_factor.unwrap_or(1.0),
                is_mobile: is_mobile.unwrap_or(false),
                has_touch: has_touch.unwrap_or(false),
            });
            apply_context_to_tabs(runtime, &candidate)?;
            Ok(StepResult::success(
                idx,
                format!("Set viewport to {width}x{height}"),
            ))
        }
        BrowserStep::EmulateMedia {
            color_scheme,
            reduced_motion,
            forced_colors,
            contrast,
            media,
        } => {
            candidate.media = refact_browser::MediaState {
                color_scheme: color_scheme.clone(),
                reduced_motion: reduced_motion.clone(),
                forced_colors: forced_colors.clone(),
                contrast: contrast.clone(),
                media: media.clone(),
            };
            apply_context_to_tabs(runtime, &candidate)?;
            Ok(StepResult::success(idx, "Applied media emulation"))
        }
        BrowserStep::SetLocale {
            locale,
        } => {
            candidate.locale = Some(locale.clone());
            apply_context_to_tabs(runtime, &candidate)?;
            Ok(StepResult::success(idx, format!("Set locale to {locale}")))
        }
        BrowserStep::SetTimezone {
            timezone,
        } => {
            candidate.timezone = Some(timezone.clone());
            apply_context_to_tabs(runtime, &candidate)?;
            Ok(StepResult::success(
                idx,
                format!("Set timezone to {timezone}"),
            ))
        }
        BrowserStep::SetUserAgent {
            user_agent,
            accept_language,
        } => {
            candidate.user_agent = Some((user_agent.clone(), accept_language.clone()));
            apply_context_to_tabs(runtime, &candidate)?;
            Ok(StepResult::success(idx, "Set user agent"))
        }
        BrowserStep::SetGeolocation {
            latitude,
            longitude,
            accuracy,
        } => {
            candidate.geolocation = Some((*latitude, *longitude, accuracy.unwrap_or(0.0)));
            apply_context_to_tabs(runtime, &candidate)?;
            Ok(StepResult::success(idx, "Set geolocation"))
        }
        BrowserStep::SetOffline {
            offline,
        } => {
            candidate.offline = *offline;
            apply_context_to_tabs(runtime, &candidate)?;
            Ok(StepResult::success(
                idx,
                if *offline {
                    "Went offline"
                } else {
                    "Went online"
                },
            ))
        }
        BrowserStep::SetNetworkConditions {
            offline,
            latency_ms,
            download_kbps,
            upload_kbps,
            preset,
        } => {
            let conditions = refact_browser::NetworkConditions::resolve(
                preset.as_deref(),
                *latency_ms,
                *download_kbps,
                *upload_kbps,
            )?;
            if let Some(offline) = offline {
                candidate.offline = *offline;
            }
            candidate.network_conditions = conditions;
            apply_context_to_tabs(runtime, &candidate)?;
            Ok(StepResult::success(
                idx,
                network_conditions_summary(candidate.offline, conditions.as_ref()),
            )
            .with_data(serde_json::json!({
                "network_conditions": conditions.map(|conditions| serde_json::json!({
                    "latency_ms": conditions.latency_ms,
                    "download_kbps": conditions.download_kbps,
                    "upload_kbps": conditions.upload_kbps,
                })),
                "offline": candidate.offline,
            })))
        }
        BrowserStep::SetCpuThrottling {
            rate,
        } => {
            if *rate < 1.0 {
                return Err(format!(
                    "rate must be at least 1 (1 disables CPU throttling), got {rate}"
                ));
            }
            candidate.cpu_throttling_rate = (*rate > 1.0).then_some(*rate);
            apply_context_to_tabs(runtime, &candidate)?;
            Ok(StepResult::success(
                idx,
                if *rate > 1.0 {
                    format!("Throttling CPU {rate}x slower")
                } else {
                    "CPU throttling off".to_string()
                },
            )
            .with_data(serde_json::json!({"cpu_throttling_rate": rate})))
        }
        BrowserStep::EmulateDevice {
            name,
        } => {
            let device = refact_browser::devices::lookup(name)?;
            candidate.viewport = Some(device.viewport.clone());
            candidate.user_agent = Some((device.user_agent.clone(), None));
            apply_context_to_tabs(runtime, &candidate)?;
            Ok(
                StepResult::success(idx, format!("Emulating {}", device.summary())).with_data(
                    serde_json::json!({"device": {
                        "name": device.name,
                        "width": device.viewport.width,
                        "height": device.viewport.height,
                        "device_scale_factor": device.viewport.device_scale_factor,
                        "is_mobile": device.viewport.is_mobile,
                        "has_touch": device.viewport.has_touch,
                        "user_agent": device.user_agent,
                    }}),
                ),
            )
        }
        BrowserStep::ListDevices {
            filter,
        } => {
            let names = refact_browser::devices::list(filter.as_deref());
            Ok(
                StepResult::success(idx, format!("{} matching device(s)", names.len())).with_data(
                    serde_json::json!({"devices": names, "aliases": ["mobile", "tablet", "desktop"]}),
                ),
            )
        }
        BrowserStep::SetExtraHttpHeaders {
            headers,
        } => {
            candidate.extra_http_headers = headers.clone();
            apply_context_to_tabs(runtime, &candidate)?;
            Ok(StepResult::success(
                idx,
                format!("Set {} extra HTTP header(s)", headers.len()),
            ))
        }
        BrowserStep::GetCookies {
            urls,
        } => {
            let cookies = refact_browser::context_state::get_cookies(
                runtime
                    .get_active_tab()
                    .ok_or_else(|| "No active tab in browser runtime".to_string())?
                    .as_ref(),
                urls.clone(),
            )?;
            Ok(
                StepResult::success(idx, format!("Read {} cookie(s)", cookies.len())).with_data(
                    serde_json::json!({
                        "cookies": refact_browser::context_state::mask_cookies(&cookies)
                    }),
                ),
            )
        }
        BrowserStep::SetCookies {
            cookies,
        } => {
            refact_browser::context_state::set_cookies(
                runtime
                    .get_active_tab()
                    .ok_or_else(|| "No active tab in browser runtime".to_string())?
                    .as_ref(),
                cookies,
            )?;
            Ok(StepResult::success(
                idx,
                format!("Set {} cookie(s)", cookies.len()),
            ))
        }
        BrowserStep::ClearCookies {
            name,
            domain,
            path,
        } => {
            let cleared = refact_browser::context_state::clear_cookies(
                runtime
                    .get_active_tab()
                    .ok_or_else(|| "No active tab in browser runtime".to_string())?
                    .as_ref(),
                name.as_deref(),
                domain.as_deref(),
                path.as_deref(),
            )?;
            Ok(StepResult::success(
                idx,
                format!("Cleared {cleared} cookie(s)"),
            ))
        }
        BrowserStep::GetStorage {
            kind,
            origin,
        } => {
            let items = refact_browser::context_state::get_storage(
                runtime
                    .get_active_tab()
                    .ok_or_else(|| "No active tab in browser runtime".to_string())?
                    .as_ref(),
                *kind,
                origin.as_deref(),
            )?;
            let masked = items
                .iter()
                .map(|item| BrowserStorageItem {
                    name: item.name.clone(),
                    value: "[REDACTED]".to_string(),
                })
                .collect::<Vec<_>>();
            Ok(
                StepResult::success(idx, format!("Read {} storage item(s)", items.len()))
                    .with_data(serde_json::json!({"items": masked})),
            )
        }
        BrowserStep::SetStorage {
            kind,
            items,
        } => {
            refact_browser::context_state::set_storage(
                runtime
                    .get_active_tab()
                    .ok_or_else(|| "No active tab in browser runtime".to_string())?
                    .as_ref(),
                *kind,
                items,
            )?;
            Ok(StepResult::success(
                idx,
                format!("Set {} storage item(s)", items.len()),
            ))
        }
        BrowserStep::ClearStorage {
            kind,
        } => {
            refact_browser::context_state::clear_storage(
                runtime
                    .get_active_tab()
                    .ok_or_else(|| "No active tab in browser runtime".to_string())?
                    .as_ref(),
                *kind,
            )?;
            Ok(StepResult::success(idx, "Cleared storage"))
        }
        BrowserStep::StorageState {
            save_as,
            indexed_db,
        } => {
            let state = refact_browser::context_state::storage_state(
                runtime
                    .get_active_tab()
                    .ok_or_else(|| "No active tab in browser runtime".to_string())?
                    .as_ref(),
                indexed_db.unwrap_or(false),
            )?;
            let masked = refact_browser::context_state::mask_storage_state(&state);
            let artifact = save_as
                .as_deref()
                .map(|save_as| {
                    refact_browser::context_state::save_storage_state(
                        &state,
                        &runtime.artifacts_dir,
                        save_as,
                    )
                })
                .transpose()?;
            match artifact {
                Some(artifact) => Ok(StepResult::success(
                    idx,
                    format!(
                        "Saved storage state to {} ({} bytes)",
                        artifact.path.display(),
                        artifact.bytes
                    ),
                )
                .with_data(serde_json::json!({
                    "state": masked,
                    "artifact": {
                        "kind": "storage_state",
                        "mime": "application/json",
                        "path": artifact.path,
                        "bytes": artifact.bytes,
                    }
                }))),
                None => Ok(StepResult::success(idx, "Captured storage state")
                    .with_data(serde_json::json!({"state": masked}))),
            }
        }
        BrowserStep::SetStorageState {
            state,
            indexed_db,
        } => {
            refact_browser::context_state::set_storage_state(
                runtime
                    .get_active_tab()
                    .ok_or_else(|| "No active tab in browser runtime".to_string())?
                    .as_ref(),
                state,
                indexed_db.unwrap_or(false),
            )?;
            Ok(StepResult::success(idx, "Restored storage state"))
        }
        BrowserStep::GrantPermissions {
            permissions,
            origin,
            state,
        } => {
            refact_browser::context_state::grant_permissions(
                runtime
                    .get_active_tab()
                    .ok_or_else(|| "No active tab in browser runtime".to_string())?
                    .as_ref(),
                permissions,
                origin.clone(),
                *state,
            )?;
            candidate.permissions =
                apply_permission_state(&candidate.permissions, permissions, *state);
            Ok(StepResult::success(
                idx,
                format!(
                    "Set {} permission(s) to {}",
                    permissions.len(),
                    state.label()
                ),
            ))
        }
        BrowserStep::ClearPermissions => {
            refact_browser::context_state::clear_permissions(
                runtime
                    .get_active_tab()
                    .ok_or_else(|| "No active tab in browser runtime".to_string())?
                    .as_ref(),
            )?;
            candidate.permissions.clear();
            Ok(StepResult::success(idx, "Cleared permissions"))
        }
        BrowserStep::SetHttpCredentials {
            username,
            password,
        } => {
            runtime.set_http_credentials(username.clone(), password.clone())?;
            candidate = runtime.context_state.clone();
            Ok(StepResult::success(idx, "Set HTTP credentials"))
        }
        _ => unreachable!(),
    })();
    match result {
        Ok(result) => {
            runtime.context_state = candidate;
            result
        }
        Err(error) => {
            runtime.context_state_dirty = true;
            StepResult::failure(
                idx,
                "Browser context",
                format!(
                    "{error}; managed configuration may be partially applied; known-good snapshot retained, live equivalence is not guaranteed"
                ),
            )
        }
    }
}

fn execute_route_management_step(
    runtime: &mut BrowserRuntime,
    step: &BrowserStep,
    idx: usize,
) -> Option<StepResult> {
    match step {
        BrowserStep::Route {
            pattern,
            handler,
            times,
        } => Some(
            match runtime.add_route(pattern.clone(), handler.clone(), *times) {
                Ok(()) => StepResult::success(idx, "Added network route")
                    .with_data(serde_json::json!({"routes": runtime.route_registry.list()})),
                Err(error) => StepResult::failure(idx, "Add network route", error),
            },
        ),
        BrowserStep::Unroute {
            pattern,
        } => Some(match runtime.remove_routes(pattern.as_ref()) {
            Ok(removed) => StepResult::success(
                idx,
                if pattern.is_some() {
                    format!("Removed {removed} matching network route(s)")
                } else {
                    format!("Removed all {removed} network route(s)")
                },
            )
            .with_data(serde_json::json!({"routes": runtime.route_registry.list()})),
            Err(error) => StepResult::failure(idx, "Remove network routes", error),
        }),
        BrowserStep::ListRoutes => {
            let routes = runtime.route_registry.list();
            Some(
                StepResult::success(idx, format!("Listed {} network route(s)", routes.len()))
                    .with_data(serde_json::json!({"routes": routes})),
            )
        }
        BrowserStep::RouteWebSocket {
            pattern,
            mode,
            on_page_message,
            on_server_message,
        } => Some(
            match runtime.websocket_registry.add_route(
                pattern.clone(),
                *mode,
                *on_page_message,
                *on_server_message,
            ) {
                Ok(()) => StepResult::success(idx, "Added WebSocket route").with_data(
                    serde_json::json!({"route_count": runtime.websocket_registry.route_count()}),
                ),
                Err(error) => StepResult::failure(idx, "Add WebSocket route", error),
            },
        ),
        BrowserStep::UnrouteWebSocket {
            pattern,
        } => {
            let removed = runtime.websocket_registry.remove_routes(pattern.as_ref());
            Some(StepResult::success(
                idx,
                if pattern.is_some() {
                    format!("Removed {removed} matching WebSocket route(s)")
                } else {
                    format!("Removed all {removed} WebSocket route(s)")
                },
            ))
        }
        BrowserStep::SendWebSocketMessage {
            pattern,
            text,
        } => {
            let result = runtime.websocket_registry.send_to_page(pattern, text);
            Some(match flush_websocket_commands(runtime, result) {
                Ok(sent) => {
                    StepResult::success(idx, format!("Sent WebSocket message to {sent} socket(s)"))
                }
                Err(error) => StepResult::failure(idx, "Send WebSocket message", error),
            })
        }
        BrowserStep::CloseWebSocket {
            pattern,
            code,
            reason,
        } => {
            let result =
                runtime
                    .websocket_registry
                    .close_sockets(pattern, *code, reason.as_deref());
            Some(match flush_websocket_commands(runtime, result) {
                Ok(route_ids) => {
                    runtime.websocket_registry.forget_sockets(&route_ids);
                    let closed = route_ids.len();
                    StepResult::success(idx, format!("Closed {closed} WebSocket(s)")).with_data(
                        serde_json::json!({"closed": closed, "code": code, "reason": reason}),
                    )
                }
                Err(error) => StepResult::failure(idx, "Close WebSocket", error),
            })
        }
        BrowserStep::StartHarRecording {
            path,
            mode,
            content,
            url_filter,
            update,
        } => Some(
            match runtime.har_recorder.start(
                &runtime.artifacts_dir,
                path.as_deref(),
                *mode,
                *content,
                url_filter.as_ref(),
                update.as_deref(),
            ) {
                Ok(path) => StepResult::success(idx, "Started HAR recording")
                    .with_data(serde_json::json!({"path": path})),
                Err(error) => StepResult::failure(idx, "Start HAR recording", error),
            },
        ),
        BrowserStep::StopHarRecording => Some(
            match runtime
                .get_store()
                .and_then(|store| runtime.har_recorder.stop_with_store(&store))
            {
                Ok(summary) => StepResult::success(
                    idx,
                    match &summary.updated_from {
                        Some(_) => format!(
                            "Updated HAR to {} entries ({} replaced, {} appended, {} bytes)",
                            summary.entry_count,
                            summary.replaced_entries,
                            summary.appended_entries,
                            summary.bytes
                        ),
                        None => format!(
                            "Saved HAR with {} entries ({} bytes)",
                            summary.entry_count, summary.bytes
                        ),
                    },
                )
                .with_data(serde_json::json!({"artifact": {
                    "kind": "har",
                    "mime": "application/json",
                    "path": summary.path,
                    "bytes": summary.bytes,
                    "entry_count": summary.entry_count,
                    "updated_from": summary.updated_from,
                    "replaced_entries": summary.replaced_entries,
                    "appended_entries": summary.appended_entries
                }})),
                Err(error) => StepResult::failure(idx, "Stop HAR recording", error),
            },
        ),
        BrowserStep::RouteFromHar {
            path,
            url_filter,
            not_found,
        } => Some(
            match runtime
                .contained_path(path, "HAR replay")
                .and_then(|resolved| {
                    refact_browser::har::HarReplay::load(&resolved, url_filter.as_ref(), *not_found)
                })
                .and_then(|replay| runtime.set_har_replay(replay))
            {
                Ok(()) => StepResult::success(idx, "Added HAR replay route")
                    .with_data(serde_json::json!({"routes": runtime.route_registry.list()})),
                Err(error) => StepResult::failure(idx, "Route from HAR", error),
            },
        ),
        _ => None,
    }
}

fn execute_init_script_step(
    runtime: &mut BrowserRuntime,
    step: &BrowserStep,
    idx: usize,
) -> StepResult {
    let tabs = runtime
        .browser
        .get_tabs()
        .lock()
        .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    match step {
        BrowserStep::AddInitScript {
            content,
        } => {
            for tab in &tabs {
                if let Err(error) = runtime.world_manager.ensure_utility_world(tab) {
                    return StepResult::failure(idx, "Add init script", error);
                }
            }
            match runtime.init_scripts.add(&tabs, content.clone()) {
                Ok(id) => StepResult::success(idx, format!("Added init script {id}"))
                    .with_data(serde_json::json!({"id": id})),
                Err(error) => StepResult::failure(idx, "Add init script", error),
            }
        }
        BrowserStep::RemoveInitScript {
            id,
        } => match runtime.init_scripts.remove(&tabs, id) {
            Ok(()) => StepResult::success(idx, format!("Removed init script {id}"))
                .with_data(serde_json::json!({"id": id})),
            Err(error) => StepResult::failure(idx, "Remove init script", error),
        },
        _ => StepResult::failure(idx, "Init script", "Unsupported init script step"),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct BrowserResetCounts {
    routes: usize,
    har_replays: usize,
    websocket_routes: usize,
    locator_handlers: usize,
    authenticators: usize,
    init_scripts: usize,
    clock: bool,
    service_worker_block: bool,
}

impl BrowserResetCounts {
    fn summary(&self) -> String {
        format!(
            "Reset: {}, {}, {}, {}, {}, {}, offline off, throttling off, emulation and device cleared, {}, {}",
            counted(self.routes, "route"),
            counted(self.har_replays, "har replay"),
            counted(self.websocket_routes, "ws route"),
            counted(self.locator_handlers, "locator handler"),
            counted(self.authenticators, "authenticator"),
            counted(self.init_scripts, "init script"),
            if self.clock {
                "clock cleared"
            } else {
                "clock off"
            },
            if self.service_worker_block {
                "service worker block cleared"
            } else {
                "service worker block off"
            },
        )
    }

    fn data(&self) -> Value {
        serde_json::json!({
            "reset": {
                "routes": self.routes,
                "har_replays": self.har_replays,
                "websocket_routes": self.websocket_routes,
                "locator_handlers": self.locator_handlers,
                "authenticators": self.authenticators,
                "init_scripts": self.init_scripts,
                "offline": false,
                "throttling_cleared": true,
                "emulation_cleared": true,
                "clock_cleared": self.clock,
                "service_worker_block_cleared": self.service_worker_block,
            }
        })
    }
}

fn counted(count: usize, singular: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {singular}s")
    }
}

fn flush_websocket_commands<T>(
    runtime: &BrowserRuntime,
    result: Result<T, String>,
) -> Result<T, String> {
    let tabs = runtime
        .browser
        .get_tabs()
        .lock()
        .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let count = result?;
    runtime.websocket_registry.flush_commands(&tabs)?;
    Ok(count)
}

fn reset_sticky_registries(
    routes: &[RouteInfo],
    websocket_registry: &WebSocketRegistry,
    locator_handlers: &Mutex<LocatorHandlerRegistry>,
    authenticators: usize,
    init_scripts: usize,
    clock: bool,
    service_worker_block: bool,
) -> Result<BrowserResetCounts, String> {
    let har_replays = routes.iter().filter(|route| route.har.is_some()).count();
    Ok(BrowserResetCounts {
        routes: routes.len() - har_replays,
        har_replays,
        websocket_routes: websocket_registry.remove_routes(None),
        locator_handlers: locator_handlers
            .lock()
            .map_err(|error| format!("Failed to lock locator handlers: {error}"))?
            .reset(),
        authenticators,
        init_scripts,
        clock,
        service_worker_block,
    })
}

fn execute_reset_step(runtime: &mut BrowserRuntime, idx: usize) -> StepResult {
    let tabs = runtime
        .browser
        .get_tabs()
        .lock()
        .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let previous_context = runtime.context_state.clone();
    let result: Result<BrowserResetCounts, String> = (|| {
        let routes = runtime.route_registry.list();
        runtime.remove_routes(None)?;
        let authenticators = runtime.webauthn_manager.cleanup(&tabs);
        let clock = runtime.clock.reset(&tabs)?;
        let init_scripts = runtime.init_scripts.reset(&tabs)?;
        let service_worker_block = runtime.context_state.block_service_workers;
        let counts = reset_sticky_registries(
            &routes,
            &runtime.websocket_registry,
            &runtime.locator_handlers,
            authenticators,
            init_scripts,
            clock,
            service_worker_block,
        )?;
        runtime.context_state.clear_overrides(&tabs)?;
        Ok(counts)
    })();
    match result {
        Ok(counts) => StepResult::success(idx, counts.summary()).with_data(counts.data()),
        Err(error) => {
            runtime.context_state = previous_context;
            runtime.context_state_dirty = true;
            StepResult::failure(
                idx,
                "Reset",
                format!("{error}; reset partially applied; live configuration is dirty"),
            )
        }
    }
}

fn save_cdp_result(
    rendered: &str,
    store: &refact_browser::artifact_store::ArtifactStore,
) -> Result<PathBuf, String> {
    store
        .write(rendered.as_bytes(), "application/json")
        .map(|artifact| artifact.path)
}

fn cdp_send_result(
    method: &str,
    target: CdpTarget,
    warnings: Vec<String>,
    result: Value,
    idx: usize,
    store: &refact_browser::artifact_store::ArtifactStore,
) -> StepResult {
    let redacted = redact_cdp_result(method, result);
    let rendered = serde_json::to_string_pretty(&redacted).unwrap_or_else(|_| redacted.to_string());
    let mut data = serde_json::json!({
        "cdp_send": {
            "method": method,
            "target": target.label(),
            "warnings": warnings,
            "bytes": rendered.len(),
        }
    });
    let entry = &mut data["cdp_send"];
    if rendered.len() > CDP_INLINE_RESULT_LIMIT_BYTES {
        match save_cdp_result(&rendered, store) {
            Ok(path) => {
                entry["artifact"] = serde_json::json!({
                    "kind": "cdp_result",
                    "mime": "application/json",
                    "path": path,
                    "bytes": rendered.len(),
                })
            }
            Err(error) => return StepResult::failure(idx, "cdp_send", error),
        }
    } else {
        entry["result"] = redacted;
    }
    let suffix = if warnings.is_empty() {
        String::new()
    } else {
        format!(" ({})", counted(warnings.len(), "warning"))
    };
    StepResult::success(
        idx,
        format!(
            "{method} on {} returned {} bytes{suffix}",
            target.label(),
            rendered.len()
        ),
    )
    .with_data(data)
}

fn execute_cdp_send_step(
    runtime: &mut BrowserRuntime,
    method: &str,
    params: Option<&Value>,
    target: CdpTarget,
    idx: usize,
) -> StepResult {
    let own_target_id = runtime
        .get_active_tab()
        .map(|tab| tab.get_target_id().to_string());
    let warnings = match classify_cdp_command(method, params, own_target_id.as_deref()) {
        CdpGuardrail::Denied {
            reason,
        } => {
            return StepResult::failure(idx, format!("cdp_send {method} blocked"), reason);
        }
        CdpGuardrail::Allowed {
            warnings,
        } => warnings,
    };
    let target_id = match target {
        CdpTarget::Page => match own_target_id {
            Some(target_id) => Some(target_id),
            None => {
                return StepResult::failure(
                    idx,
                    "cdp_send",
                    "No active tab in browser runtime".to_string(),
                );
            }
        },
        CdpTarget::Browser => None,
    };
    let store = match runtime.get_store() {
        Ok(store) => store,
        Err(error) => return StepResult::failure(idx, "cdp_send artifact store", error),
    };
    let session = match runtime.cdp_session() {
        Ok(session) => session,
        Err(error) => return StepResult::failure(idx, "cdp_send", error),
    };
    runtime.touch();
    match session.send(method, params, target_id.as_deref()) {
        Ok(result) => cdp_send_result(method, target, warnings, result, idx, &store),
        Err(error) => StepResult::failure(idx, format!("cdp_send {method} failed"), error),
    }
}

fn is_screencast_step(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::CaptureFrames { .. }
            | BrowserStep::ScreencastStart { .. }
            | BrowserStep::ScreencastStop { .. }
    )
}

fn screencast_quality(quality: Option<u32>) -> Result<u32, String> {
    match quality {
        Some(quality) if quality > 100 => Err(format!(
            "Screencast quality {quality} must be between 0 and 100"
        )),
        Some(quality) => Ok(quality),
        None => Ok(DEFAULT_SCREENCAST_QUALITY),
    }
}

fn burst_screenshot_options(policy: &ImagePolicy, full_page: bool) -> BrowserScreenshotOptions {
    BrowserScreenshotOptions {
        full_page,
        image_type: Some(BrowserScreenshotType::Jpeg),
        quality: policy.quality,
        ..Default::default()
    }
}

fn capture_scoped_frame(
    tab: &Tab,
    world: &WorldManager,
    locator: Option<&BrowserLocator>,
    options: &BrowserScreenshotOptions,
    policy: &ImagePolicy,
) -> Result<Vec<u8>, String> {
    let element = match locator {
        Some(locator) => {
            let resolved = resolve_element(tab, world, locator)?;
            let clip = resolved
                .bbox
                .as_ref()
                .filter(|bbox| bbox.width > 0.0 && bbox.height > 0.0)
                .ok_or_else(|| "Element has no visible bounds".to_string())
                .and_then(|bbox| {
                    screenshot_metrics(tab).map(|metrics| BrowserScreenshotClip {
                        x: metrics.page_x + bbox.x,
                        y: metrics.page_y + bbox.y,
                        width: bbox.width,
                        height: bbox.height,
                    })
                });
            let _ = world.release_handle(tab, &resolved.handle);
            Some(clip?)
        }
        None => None,
    };
    let capture = capture_screenshot(tab, world, options, element, policy)?;
    base64::prelude::BASE64_STANDARD
        .decode(capture.data)
        .map_err(|error| format!("Screencast frame decode failed: {error}"))
}

fn capture_frame_burst(
    tab: &Tab,
    world: &WorldManager,
    plan: &FrameBurstPlan,
    locator: Option<&BrowserLocator>,
    full_page: bool,
    policy: &ImagePolicy,
) -> Result<(Vec<CapturedFrame>, Vec<String>, bool), String> {
    let capture_started = Instant::now();
    let budget = timed_capture_budget(plan);
    let scoped = locator.is_some() || full_page;
    if !scoped {
        let quality = policy
            .quality
            .map(u32::from)
            .unwrap_or(DEFAULT_SCREENCAST_QUALITY)
            .min(100);
        let frames = capture_screencast_burst(tab, plan, quality)?;
        if frames.len() >= 2 {
            return Ok((frames, Vec::new(), capture_started.elapsed() >= budget));
        }
    }
    let options = burst_screenshot_options(policy, full_page && locator.is_none());
    let reason = if locator.is_some() {
        "element-scoped frames use timed screenshots"
    } else if full_page {
        "full-page frames use timed screenshots"
    } else {
        "the screencast produced too few frames, captured with timed screenshots instead"
    };
    let mut capture = capture_timed_frames_within(
        plan,
        budget.saturating_sub(capture_started.elapsed()),
        Instant::now,
        || capture_scoped_frame(tab, world, locator, &options, policy),
        |remaining| std::thread::sleep(remaining),
    )?;
    capture.budget_ms = budget.as_millis().min(u128::from(u64::MAX)) as u64;
    capture.elapsed_ms = capture_started
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let mut warnings = vec![reason.to_string()];
    warnings.extend(capture.warning());
    Ok((
        capture.frames,
        warnings,
        capture.budget_exhausted || capture_started.elapsed() >= budget,
    ))
}

fn filmstrip_step_data(result: &FilmstripResult) -> Value {
    serde_json::json!({
        "mime": result.filmstrip.mime,
        "data": result.filmstrip_data,
        "width": result.filmstrip.width,
        "height": result.filmstrip.height,
        "bytes": result.filmstrip.bytes,
        "artifact": result.filmstrip,
        "frames": result.frames,
        "frame_count": result.frames.len(),
        "columns": result.columns,
        "rows": result.rows,
        "duration_ms": result.duration_ms,
        "warnings": result.warnings,
    })
}

fn execute_screencast_step(
    runtime: &mut BrowserRuntime,
    step: &BrowserStep,
    idx: usize,
    image_policy: &ImagePolicy,
) -> StepResult {
    let result: Result<StepResult, String> = (|| {
        let tab = runtime
            .get_active_tab()
            .ok_or_else(|| "No active tab in browser runtime".to_string())?;
        match step {
            BrowserStep::CaptureFrames {
                duration_ms,
                frame_count,
                interval_ms,
                locator,
                full_page,
            } => {
                let plan = FrameBurstPlan::resolve(*duration_ms, *frame_count, *interval_ms)?;
                let (frames, warnings, budget_exhausted) = capture_frame_burst(
                    &tab,
                    &runtime.world_manager,
                    &plan,
                    locator.as_ref(),
                    full_page.unwrap_or(false),
                    image_policy,
                )?;
                let captured = frames.len();
                let elapsed_note = if budget_exhausted {
                    " (scheduling budget exhausted, see warnings)"
                } else {
                    ""
                };
                if captured < 2 {
                    return Ok(StepResult::failure(
                        idx,
                        "Insufficient capture evidence",
                        "Fewer than two frames were captured; no motion claim can be made",
                    )
                    .with_data(serde_json::json!({
                        "frame_count": captured,
                        "planned_frame_count": plan.frame_count,
                        "budget_exhausted": budget_exhausted,
                        "insufficient_motion_evidence": true,
                        "warnings": warnings,
                    })));
                }
                let filmstrip = build_filmstrip(
                    &frames,
                    runtime.get_store()?.as_ref(),
                    "burst",
                    image_policy,
                    warnings,
                )?;
                Ok(StepResult::success(
                    idx,
                    format!(
                        "Captured {captured} of {} planned frame(s) over {}ms{elapsed_note}",
                        plan.frame_count, plan.duration_ms
                    ),
                )
                .with_data(filmstrip_step_data(&filmstrip)))
            }
            BrowserStep::ScreencastStart {
                quality,
                max_width,
                max_height,
            } => {
                runtime.screencast_manager.start(
                    &tab,
                    ScreencastSessionOptions {
                        quality: screencast_quality(*quality)?,
                        max_width: *max_width,
                        max_height: *max_height,
                    },
                )?;
                Ok(StepResult::success(
                    idx,
                    format!(
                        "Started screencast, auto-stops after {MAX_SESSION_DURATION_MS}ms or {MAX_SESSION_FRAMES} frames"
                    ),
                ))
            }
            BrowserStep::ScreencastStop {
                compose,
            } => {
                let stopped = runtime.screencast_manager.stop(&tab)?;
                let mut warnings = Vec::new();
                if stopped.auto_stopped {
                    warnings.push(format!(
                        "The screencast auto-stopped at the {MAX_SESSION_DURATION_MS}ms / {MAX_SESSION_FRAMES} frame cap"
                    ));
                }
                let captured = stopped.frames.len();
                if !compose.unwrap_or(true) {
                    return Ok(StepResult::success(
                        idx,
                        format!("Stopped screencast after {captured} frame(s)"),
                    )
                    .with_data(serde_json::json!({
                        "frame_count": captured,
                        "duration_ms": stopped.duration_ms,
                        "warnings": warnings,
                    })));
                }
                let selected = select_evenly_spaced(stopped.frames, MAX_FRAME_COUNT);
                if selected.len() > 1 {
                    warnings.push(format!(
                        "Composed {} of {captured} captured frame(s)",
                        selected.len()
                    ));
                }
                let filmstrip = build_filmstrip(
                    &selected,
                    runtime.get_store()?.as_ref(),
                    "session",
                    image_policy,
                    warnings,
                )?;
                Ok(StepResult::success(
                    idx,
                    format!("Stopped screencast after {captured} frame(s)"),
                )
                .with_data(filmstrip_step_data(&filmstrip)))
            }
            _ => unreachable!(),
        }
    })();
    result.unwrap_or_else(|error| StepResult::failure(idx, "Screencast", error))
}

#[derive(Debug, Clone, PartialEq)]
struct WindowBoundsRequest {
    left: Option<u32>,
    top: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
}

fn validate_window_bounds_request(
    x: Option<i32>,
    y: Option<i32>,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<WindowBoundsRequest, String> {
    if x.is_none() && y.is_none() && width.is_none() && height.is_none() {
        return Err("set_window_bounds needs at least one of x, y, width, height".to_string());
    }
    for (name, value) in [("x", x), ("y", y)] {
        if let Some(value) = value {
            if value < 0 {
                return Err(format!(
                    "set_window_bounds {name}={value} is negative; the CDP client only accepts non-negative window positions"
                ));
            }
        }
    }
    for (name, value) in [("width", width), ("height", height)] {
        if value == Some(0) {
            return Err(format!(
                "set_window_bounds {name} must be greater than zero"
            ));
        }
    }
    Ok(WindowBoundsRequest {
        left: x.map(|value| value as u32),
        top: y.map(|value| value as u32),
        width,
        height,
    })
}

fn execute_set_window_bounds_step(
    runtime: &mut BrowserRuntime,
    x: Option<i32>,
    y: Option<i32>,
    width: Option<u32>,
    height: Option<u32>,
    idx: usize,
) -> StepResult {
    let request = match validate_window_bounds_request(x, y, width, height) {
        Ok(request) => request,
        Err(error) => return StepResult::failure(idx, "Set window bounds", error),
    };
    if runtime.headless() {
        return StepResult::success(
            idx,
            "Set window bounds skipped: headless has no OS window, use set_viewport for emulation",
        )
        .with_data(serde_json::json!({"applied": false, "headless": true}));
    }
    let result: Result<StepResult, String> = (|| {
        let tab = runtime
            .get_active_tab()
            .ok_or_else(|| "No active tab in browser runtime".to_string())?;
        tab.set_bounds(headless_chrome::types::Bounds::Normal {
            left: request.left,
            top: request.top,
            width: request.width.map(f64::from),
            height: request.height.map(f64::from),
        })
        .map_err(|error| format!("Failed to set window bounds: {error}"))?;
        let bounds = tab
            .get_bounds()
            .map_err(|error| format!("Failed to read back window bounds: {error}"))?;
        let applied = refact_chat_api::WindowBounds {
            x: bounds.left as i32,
            y: bounds.top as i32,
            width: bounds.width as u32,
            height: bounds.height as u32,
        };
        let summary = format!(
            "Set window to {}x{} at ({}, {})",
            applied.width, applied.height, applied.x, applied.y
        );
        let data = serde_json::json!({
            "applied": true,
            "headless": false,
            "bounds": applied.clone(),
        });
        runtime.set_window_bounds(applied);
        Ok(StepResult::success(idx, summary).with_data(data))
    })();
    match result {
        Ok(step) => step,
        Err(error) => StepResult::failure(idx, "Set window bounds", error),
    }
}

fn is_clock_step(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::ClockInstall { .. }
            | BrowserStep::ClockFastForward { .. }
            | BrowserStep::ClockPauseAt { .. }
            | BrowserStep::ClockResume
            | BrowserStep::ClockRunFor { .. }
            | BrowserStep::ClockSetFixedTime { .. }
            | BrowserStep::ClockSetSystemTime { .. }
    )
}

fn clock_op_from_step(step: &BrowserStep) -> Result<ClockOp, String> {
    match step {
        BrowserStep::ClockInstall {
            time,
        } => Ok(ClockOp::Install {
            time_ms: match time {
                Some(time) => parse_clock_time(time)?,
                None => current_wall_ms(),
            },
        }),
        BrowserStep::ClockFastForward {
            ticks,
        } => Ok(ClockOp::FastForward {
            ticks_ms: parse_clock_ticks(ticks)?,
        }),
        BrowserStep::ClockPauseAt {
            time,
        } => Ok(ClockOp::PauseAt {
            time_ms: parse_clock_time(time)?,
        }),
        BrowserStep::ClockResume => Ok(ClockOp::Resume),
        BrowserStep::ClockRunFor {
            ticks,
        } => Ok(ClockOp::RunFor {
            ticks_ms: parse_clock_ticks(ticks)?,
        }),
        BrowserStep::ClockSetFixedTime {
            time,
        } => Ok(ClockOp::SetFixedTime {
            time_ms: parse_clock_time(time)?,
        }),
        BrowserStep::ClockSetSystemTime {
            time,
        } => Ok(ClockOp::SetSystemTime {
            time_ms: parse_clock_time(time)?,
        }),
        _ => unreachable!(),
    }
}

fn execute_clock_step(runtime: &mut BrowserRuntime, step: &BrowserStep, idx: usize) -> StepResult {
    let tabs = runtime
        .browser
        .get_tabs()
        .lock()
        .map(|tabs| tabs.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let result: Result<ClockOp, String> = (|| {
        let op = clock_op_from_step(step)?;
        runtime.clock.run(&tabs, op, current_wall_ms())?;
        Ok(op)
    })();
    match result {
        Ok(op) => StepResult::success(idx, op.summary()).with_data(serde_json::json!({
            "clock": {
                "installed": runtime.clock.is_installed(),
                "paused": runtime.clock.is_paused(),
            }
        })),
        Err(error) => StepResult::failure(idx, "Browser clock", error),
    }
}

struct PreparedHttpRequest {
    spec: http_client::HttpRequestSpec,
    tab: Arc<Tab>,
    jar: Vec<refact_integrations::browser_models::BrowserCookie>,
    store: Arc<refact_browser::artifact_store::ArtifactStore>,
}

fn prepare_http_request(
    runtime: &mut BrowserRuntime,
    options: &BrowserHttpRequest,
) -> Result<PreparedHttpRequest, String> {
    let settings = browser_settings::current();
    let store = runtime.get_store()?;
    let prepared = (|| -> Result<(http_client::HttpRequestSpec, Arc<Tab>), String> {
        let spec = http_client::HttpRequestSpec {
            url: http_client::parse_http_url(&options.url)?,
            method: http_client::parse_http_method(options.method.as_deref())?,
            headers: options.headers.clone(),
            body: http_client::build_request_body(
                options.body.as_deref(),
                options.body_json.as_ref(),
                options.form.as_ref(),
            )?,
            timeout: Duration::from_millis(
                options
                    .timeout_ms
                    .unwrap_or(http_client::DEFAULT_HTTP_TIMEOUT_MS)
                    .min(settings.timing.max_wait_timeout_ms),
            ),
            max_redirects: options
                .max_redirects
                .unwrap_or(http_client::DEFAULT_HTTP_MAX_REDIRECTS)
                .min(http_client::DEFAULT_HTTP_MAX_REDIRECTS),
        };
        let tab = runtime
            .get_active_tab()
            .ok_or_else(|| "No active tab in browser runtime".to_string())?;
        Ok((spec, tab))
    })();
    let (spec, tab) = prepared?;
    let jar = refact_browser::context_state::get_cookies(&tab, None)?;
    Ok(PreparedHttpRequest {
        spec,
        tab,
        jar,
        store,
    })
}

fn apply_http_response(
    runtime: &mut BrowserRuntime,
    tab: &Tab,
    response: &http_client::HttpResponse,
) -> Result<(), String> {
    if !response.set_cookies.is_empty() {
        refact_browser::context_state::set_cookies(tab, &response.set_cookies)?;
    }
    runtime.touch();
    Ok(())
}

async fn execute_http_request_step(
    runtime: &mut BrowserRuntime,
    options: &BrowserHttpRequest,
    idx: usize,
) -> StepResult {
    let prepared = match prepare_http_request(runtime, options) {
        Ok(prepared) => prepared,
        Err(error) => return StepResult::failure(idx, "HTTP request", error),
    };
    let response = match http_client::send_http_request(&prepared.spec, &prepared.jar).await {
        Ok(response) => response,
        Err(error) => return StepResult::failure(idx, "HTTP request", error),
    };
    if let Err(error) = apply_http_response(runtime, &prepared.tab, &response) {
        return StepResult::failure(idx, "HTTP request", error);
    }
    http_request_result(&response, options, idx, &prepared.store)
}

fn http_request_result(
    response: &http_client::HttpResponse,
    options: &BrowserHttpRequest,
    idx: usize,
    store: &refact_browser::artifact_store::ArtifactStore,
) -> StepResult {
    let content_type = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone());
    let mut data = serde_json::json!({
        "http_request": {
            "method": response.method,
            "url": refact_browser::mask_text(&response.final_url),
            "status": response.status,
            "status_text": response.status_text,
            "redirects": response.redirects,
            "headers": http_client::summarize_response_headers(
                &response.headers,
                options.full_headers.unwrap_or(false),
            ),
            "body_bytes": response.body.len(),
            "set_cookies": {
                "count": response.set_cookies.len(),
                "names": response.set_cookies.iter().map(|cookie| cookie.name.clone()).collect::<Vec<_>>(),
            },
            "rejected_cookies": response.rejected_cookies,
        }
    });
    let entry = &mut data["http_request"];
    match http_client::split_response_body(&response.body, content_type.as_deref()) {
        http_client::HttpResponseBody::Empty => {}
        http_client::HttpResponseBody::Inline(text) => entry["body"] = Value::String(text),
        http_client::HttpResponseBody::Artifact => {
            match http_client::save_response_body(
                &response.body,
                store,
                content_type
                    .as_deref()
                    .unwrap_or("application/octet-stream"),
            ) {
                Ok(path) => {
                    entry["artifact"] = serde_json::json!({
                        "path": path,
                        "bytes": response.body.len(),
                    })
                }
                Err(error) => return StepResult::failure(idx, "HTTP request", error),
            }
        }
    }
    let summary = format!(
        "{} {} -> {} {} ({} bytes, {} cookies set)",
        response.method,
        refact_browser::mask_text(&response.final_url),
        response.status,
        response.status_text,
        response.body.len(),
        response.set_cookies.len()
    );
    if options.fail_on_status.unwrap_or(false) && !(200..300).contains(&response.status) {
        return StepResult::failure(
            idx,
            summary,
            format!("HTTP {} {}", response.status, response.status_text),
        )
        .with_data(data);
    }
    StepResult::success(idx, summary).with_data(data)
}

fn is_instrumentation_step(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::StartCoverage { .. }
            | BrowserStep::StopCoverage
            | BrowserStep::AddVirtualAuthenticator { .. }
            | BrowserStep::RemoveVirtualAuthenticator { .. }
            | BrowserStep::ListCredentials { .. }
            | BrowserStep::AddCredential { .. }
            | BrowserStep::ClearCredentials { .. }
            | BrowserStep::SetUserVerified { .. }
    )
}

fn virtual_authenticator_added(idx: usize, id: String) -> StepResult {
    StepResult::success(idx, format!("Added virtual authenticator {id}"))
        .with_data(serde_json::json!({"authenticator_id": id}))
}

fn execute_instrumentation_step(
    runtime: &mut BrowserRuntime,
    step: &BrowserStep,
    idx: usize,
) -> StepResult {
    let result: Result<StepResult, String> = (|| {
        let tab = runtime
            .get_active_tab()
            .ok_or_else(|| "No active tab in browser runtime".to_string())?;
        match step {
            BrowserStep::StartCoverage {
                js,
                css,
                reset_on_navigation,
            } => {
                let options = refact_browser::coverage::CoverageOptions::resolve(
                    *js,
                    *css,
                    *reset_on_navigation,
                );
                runtime.coverage_manager.start(&tab, options)?;
                Ok(StepResult::success(
                    idx,
                    format!(
                        "Started coverage (JavaScript: {}, CSS: {})",
                        options.js, options.css
                    ),
                ))
            }
            BrowserStep::StopCoverage => {
                let stopped = runtime
                    .coverage_manager
                    .stop(&tab, runtime.get_store()?.as_ref())?;
                let resource_count = stopped.artifact.resource_count;
                let unmeasured = stopped.artifact.unmeasured_resource_count;
                let message = if unmeasured > 0 {
                    format!(
                        "Stopped coverage for {resource_count} resource(s), {unmeasured} \
                         unmeasured (source retrieval failed, no percentage reported)"
                    )
                } else {
                    format!("Stopped coverage for {resource_count} resource(s)")
                };
                Ok(
                    StepResult::success(idx, message).with_data(serde_json::json!({
                        "coverage": stopped.summaries,
                        "artifact": stopped.artifact,
                    })),
                )
            }
            BrowserStep::AddVirtualAuthenticator {
                protocol,
                transport,
                has_resident_key,
                has_user_verification,
                is_user_verified,
                ..
            } => {
                let id = runtime.webauthn_manager.add_virtual_authenticator(
                    &tab,
                    protocol.unwrap_or_default(),
                    transport.unwrap_or_default(),
                    has_resident_key.unwrap_or(false),
                    has_user_verification.unwrap_or(false),
                    is_user_verified.unwrap_or(false),
                )?;
                Ok(virtual_authenticator_added(idx, id))
            }
            BrowserStep::RemoveVirtualAuthenticator {
                id,
            } => {
                runtime
                    .webauthn_manager
                    .remove_virtual_authenticator(&tab, id)?;
                Ok(StepResult::success(idx, "Removed virtual authenticator"))
            }
            BrowserStep::ListCredentials {
                id,
            } => {
                let credentials = runtime.webauthn_manager.list_credentials(&tab, id)?;
                Ok(
                    StepResult::success(idx, format!("Listed {} credential(s)", credentials.len()))
                        .with_data(serde_json::json!({"credentials": credentials})),
                )
            }
            BrowserStep::AddCredential {
                id,
                credential,
            } => {
                runtime
                    .webauthn_manager
                    .add_credential(&tab, id, credential)?;
                Ok(StepResult::success(
                    idx,
                    "Added virtual authenticator credential",
                ))
            }
            BrowserStep::ClearCredentials {
                id,
            } => {
                runtime.webauthn_manager.clear_credentials(&tab, id)?;
                Ok(StepResult::success(
                    idx,
                    "Cleared virtual authenticator credentials",
                ))
            }
            BrowserStep::SetUserVerified {
                id,
                verified,
            } => {
                runtime
                    .webauthn_manager
                    .set_user_verified(&tab, id, *verified)?;
                Ok(StepResult::success(
                    idx,
                    format!("Set virtual authenticator user verification to {verified}"),
                ))
            }
            _ => unreachable!(),
        }
    })();
    result.unwrap_or_else(|error| StepResult::failure(idx, "Browser instrumentation", error))
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
    let mut mouse_state = MouseState::default();
    let result = execute_single_step(
        tab,
        world,
        step,
        idx,
        None,
        image_policy,
        Some(&handlers),
        &mut Vec::new(),
        &mut mouse_state,
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
    let deadline = Instant::now() + BROWSER_REQUEST_DEADLINE;
    tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        validate_upload_paths(gcx.clone(), &request),
    )
    .await
    .map_err(|_| "Browser deadline exceeded during upload validation".to_string())??;
    let plan = {
        let rt = lock_browser_before(&runtime_arc, deadline).await?;
        rt.attached_chat_id()
            .filter(|_| rt.owns_browser_process())
            .map(|chat_id| (chat_id, rt.profile_dir.clone(), rt.launch_options.clone()))
    };
    let Some((chat_id, profile_dir, launch_options)) = plan else {
        return execute_request_with_runtime_before_deadline(
            runtime_arc,
            request,
            image_policy,
            deadline,
        )
        .await;
    };
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let transport_alive = {
        let mut rt = lock_browser_before(&runtime_arc, deadline).await?;
        let process_alive = rt
            .ownership
            .process_id()
            .is_none_or(crate::integrations::browser_runtime::pid_is_alive);
        let alive = process_alive
            && browser_block_in_place(
                deadline.min(Instant::now() + Duration::from_secs(5)),
                || rt.check_connection(),
            );
        if alive {
            let _ = browser_block_in_place(deadline, || rt.checkpoint_document_identity());
        }
        alive
    };
    if transport_alive {
        return execute_request_with_runtime_before_deadline(runtime_arc, request, image_policy, deadline)
            .await.map_err(|error| {
                if refact_browser::is_transport_dead_error(&error) {
                    format!("Browser transport was lost during dispatch; batch incomplete, effect unknown, retry_safe=false. No action was retried. Submit a new request and inspect/acknowledge the current session_generation before explicit bootstrap. Original error: {error}")
                } else { error }
            });
    }
    let original_process_alive = lock_browser_before(&runtime_arc, deadline)
        .await?
        .ownership
        .process_id()
        .is_some_and(crate::integrations::browser_runtime::pid_is_alive);
    if original_process_alive {
        let reconnect =
            crate::integrations::browser_runtime::reconnect_runtime_for_chat_before_deadline(
                app, &chat_id, deadline,
            )
            .await?;
        if !reconnect.same_state_verified {
            let rt = lock_browser_before(&runtime_arc, deadline).await?;
            return Ok(session_lost_report(&rt, request.steps.len(), false));
        }
        let mut report = execute_request_with_runtime_before_deadline(
            runtime_arc,
            request,
            image_policy,
            deadline,
        )
        .await?;
        report.warnings.push("CDP connection reattached to the same verified document; no interrupted action was retried".to_string());
        return Ok(report);
    }
    let replacement =
        relaunch_and_resolve(app, &chat_id, profile_dir, launch_options, deadline).await?;
    let rt = lock_browser_before(&replacement, deadline).await?;
    Ok(session_lost_report(&rt, request.steps.len(), false))
}

fn session_lost_report(
    runtime: &BrowserRuntime,
    total_steps: usize,
    effect_unknown: bool,
) -> ExecutionReport {
    let error = format!(
        "Browser session state was lost. Submit a NEW request with acknowledge_session_generation: {:?}, then explicitly navigate/open_tab/set_content. Old target ids and refs cannot be reused; no interrupted action was retried. Routes, mocks, eval/CDP, permissions and authentication are not restored.",
        runtime.runtime_id
    );
    let mut steps = vec![StepResult::failure(0, "Browser session state lost", error)];
    steps[0].skipped = true;
    steps[0].outcome = Some("skipped".to_string());
    mark_unexecuted_steps_skipped(&mut steps, total_steps);
    ExecutionReport {
        session_generation: Some(runtime.runtime_id.clone()),
        recovery: Some(BrowserRecovery {
            incomplete: true,
            state_lost: true,
            effect_unknown,
            retry_safe: false,
        }),
        ok: false,
        steps,
        ..ExecutionReport::default()
    }
}

fn restores_document_after_relaunch(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::Navigate { .. } | BrowserStep::SetContent { .. } | BrowserStep::OpenTab { .. }
    )
}

/// Terminal transport failures override every soft/non-fatal/continue policy.
fn terminalize_transport_result(result: &mut StepResult) -> bool {
    if !step_hit_dead_transport(result) && result.outcome.as_deref() != Some("effect_unknown") {
        return false;
    }
    result.ok = false;
    result.outcome = Some("effect_unknown".to_string());
    true
}

fn step_failure_is_fatal(step: &BrowserStep, result: &mut StepResult) -> bool {
    let terminal = terminalize_transport_result(result);
    !result.ok && (terminal || !is_non_fatal_step(step))
}

fn transport_recovery(results: &[StepResult]) -> Option<BrowserRecovery> {
    results
        .iter()
        .any(|step| {
            step_hit_dead_transport(step) || step.outcome.as_deref() == Some("effect_unknown")
        })
        .then_some(BrowserRecovery {
            incomplete: true,
            state_lost: true,
            effect_unknown: true,
            retry_safe: false,
        })
}

fn step_hit_dead_transport(step: &StepResult) -> bool {
    step.error
        .as_deref()
        .is_some_and(refact_browser::is_transport_dead_error)
}

fn report_hit_dead_transport(outcome: &Result<ExecutionReport, String>) -> bool {
    match outcome {
        Err(error) => refact_browser::is_transport_dead_error(error),
        Ok(report) => !report.ok && report.steps.iter().any(step_hit_dead_transport),
    }
}

async fn relaunch_and_resolve(
    app: crate::app_state::AppState,
    chat_id: &str,
    profile_dir: PathBuf,
    launch_options: refact_browser::BrowserLaunchOptions,
    deadline: Instant,
) -> Result<Arc<AMutex<BrowserRuntime>>, String> {
    let window_bounds = launch_options.window_bounds.clone();
    let runtime_id =
        crate::integrations::browser_runtime::relaunch_runtime_for_chat_before_deadline(
            app.clone(),
            chat_id,
            profile_dir,
            launch_options,
            window_bounds,
            deadline,
        )
        .await?;
    let browser_runtimes = app.integrations.browser_runtimes.clone();
    let browser_runtimes =
        crate::integrations::browser_runtime::before_deadline(deadline, browser_runtimes.lock())
            .await?;
    browser_runtimes
        .get(&runtime_id)
        .cloned()
        .ok_or_else(|| format!("Relaunched BrowserRuntime {} disappeared", runtime_id))
}

struct PendingPopupWait {
    step_index: usize,
    result_index: usize,
    deadline: Instant,
    timeout_ms: u64,
    baseline: std::collections::BTreeSet<String>,
}

impl PendingPopupWait {
    fn arm(
        step_index: usize,
        result_index: usize,
        timeout_ms: Option<u64>,
        baseline: std::collections::BTreeSet<String>,
    ) -> Self {
        let timeout_ms = clamp_timeout_ms(timeout_ms);
        Self {
            step_index,
            result_index,
            deadline: Instant::now() + Duration::from_millis(timeout_ms),
            timeout_ms,
            baseline,
        }
    }
}

fn popup_wait_conflict(armed_step_index: Option<usize>) -> Option<String> {
    armed_step_index.map(|index| format!("a popup wait is already armed at step {index}"))
}

fn popup_wait_opened_result(step_index: usize, tab_id: &str) -> StepResult {
    StepResult::success(step_index, format!("Popup opened: {tab_id}"))
        .with_data(serde_json::json!({"tab_id": tab_id}))
}

fn popup_wait_timeout_result(step_index: usize, timeout_ms: u64) -> StepResult {
    StepResult::failure(
        step_index,
        "Wait for popup",
        format!("Timed out after {timeout_ms}ms"),
    )
}

#[derive(Default)]
struct TabManagementSections {
    new_tabs: Vec<TabInfo>,
    intercepted_requests: Vec<RouteInterception>,
    locator_handlers: Vec<LocatorHandlerFiring>,
    dialogs: Vec<DialogInfo>,
    uploads: Vec<UploadInfo>,
    downloads: Vec<DownloadInfo>,
    websockets: Vec<WebSocketEvent>,
}

impl TabManagementSections {
    fn absorb(&mut self, report: ExecutionReport) -> Option<StepResult> {
        self.new_tabs.extend(report.new_tabs);
        self.intercepted_requests
            .extend(report.intercepted_requests);
        self.locator_handlers.extend(report.locator_handlers);
        self.dialogs.extend(report.dialogs);
        self.uploads.extend(report.uploads);
        self.downloads.extend(report.downloads);
        self.websockets.extend(report.websockets);
        report.steps.into_iter().next()
    }
}

const BROWSER_REQUEST_DEADLINE: Duration = Duration::from_secs(300);

async fn lock_browser_before<'a>(
    runtime: &'a Arc<AMutex<BrowserRuntime>>,
    deadline: Instant,
) -> Result<tokio::sync::MutexGuard<'a, BrowserRuntime>, String> {
    if Instant::now() >= deadline {
        return Err("Browser execution deadline exceeded; no further action was dispatched".into());
    }
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), runtime.lock())
        .await
        .map_err(|_| "Browser execution deadline exceeded while waiting for runtime; no further action was dispatched".to_string())
}

fn browser_block_in_place<T>(deadline: Instant, execute: impl FnOnce() -> T) -> T {
    tokio::task::block_in_place(|| {
        let _deadline = headless_chrome::browser::transport::CommandDeadlineGuard::new(deadline);
        execute()
    })
}

pub async fn execute_request_with_runtime(
    runtime_arc: Arc<AMutex<BrowserRuntime>>,
    request: BrowserActionRequest,
    image_policy: &ImagePolicy,
) -> Result<ExecutionReport, String> {
    execute_request_with_runtime_before_deadline(
        runtime_arc,
        request,
        image_policy,
        Instant::now() + BROWSER_REQUEST_DEADLINE,
    )
    .await
}

async fn execute_request_with_runtime_before_deadline(
    runtime_arc: Arc<AMutex<BrowserRuntime>>,
    request: BrowserActionRequest,
    image_policy: &ImagePolicy,
    deadline: Instant,
) -> Result<ExecutionReport, String> {
    let (ownership, connection_epoch) = {
        let runtime = lock_browser_before(&runtime_arc, deadline).await?;
        (
            runtime.ownership.clone(),
            runtime.ownership.connection_epoch(),
        )
    };
    let settings = browser_settings::current();
    if request.session != SessionPolicy::SharedDefault {
        return Err(format!(
            "Unsupported browser session policy: {:?}",
            request.session
        ));
    }
    {
        let mut rt = lock_browser_before(&runtime_arc, deadline).await?;
        rt.touch();
        if !rt.ownership.is_current_generation() {
            return Err("Browser generation was replaced; submit a new request using the current session_generation".to_string());
        }
        if let Some(generation) = &request.acknowledge_session_generation {
            rt.acknowledge_session_generation(generation)?;
        }
        if !rt.session_acknowledged && request.steps.iter().any(step_requires_session_ack) {
            return Ok(session_lost_report(&rt, request.steps.len(), false));
        }
        if let TabTarget::Id {
            id,
        } = &request.target
        {
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
        if let Some(block) = request.block_service_workers {
            browser_block_in_place(deadline, || rt.set_block_service_workers(block))?;
        }
    }

    let initial_url = {
        let rt = lock_browser_before(&runtime_arc, deadline).await?;
        rt.get_active_tab().map(|tab| tab.get_url())
    };
    let mut current_tab = {
        let rt = lock_browser_before(&runtime_arc, deadline).await?;
        rt.get_active_tab()
    };
    let world = {
        let rt = lock_browser_before(&runtime_arc, deadline).await?;
        rt.world_manager.clone()
    };
    let handlers = {
        let rt = lock_browser_before(&runtime_arc, deadline).await?;
        rt.locator_handlers.clone()
    };
    let network_monitor = {
        let rt = lock_browser_before(&runtime_arc, deadline).await?;
        rt.network_monitor.clone()
    };
    let websocket_registry = {
        let rt = lock_browser_before(&runtime_arc, deadline).await?;
        rt.websocket_registry.clone()
    };
    let download_monitor = {
        let rt = lock_browser_before(&runtime_arc, deadline).await?;
        rt.download_monitor.clone()
    };
    let file_chooser_manager = {
        let rt = lock_browser_before(&runtime_arc, deadline).await?;
        rt.file_chooser_manager.clone()
    };
    let armed_network_waits = request
        .steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| match step {
            BrowserStep::WaitForRequest {
                ..
            } => Some((index, network_monitor.request_cursor())),
            BrowserStep::WaitForResponse {
                ..
            } => Some((index, network_monitor.response_cursor())),
            BrowserStep::WaitForDownload {
                ..
            } => Some((index, download_monitor.cursor())),
            BrowserStep::WaitForWebSocketFrame {
                ..
            } => Some((index, websocket_registry.cursor())),
            _ => None,
        })
        .collect::<std::collections::HashMap<_, _>>();
    let (initial_tab_ids, armed_console_cursor) = {
        let mut rt = lock_browser_before(&runtime_arc, deadline).await?;
        browser_block_in_place(deadline, || refact_browser::adopt_new_tabs(&mut rt, None));
        rt.drain_raw_events();
        (rt.known_tab_ids(), rt.console_buffer.len())
    };
    let mut recovery_fence = lock_browser_before(&runtime_arc, deadline)
        .await?
        .recovery_fence
        .clone();
    let mut results: Vec<StepResult> = Vec::new();
    // Preserve completed evidence when a later lock/report phase exhausts the budget.
    macro_rules! runtime_before {
        () => {
            match lock_browser_before(&runtime_arc, deadline).await {
                Ok(runtime) => runtime,
                Err(error) => {
                    let mut retained = std::mem::take(&mut results);
                    if retained.len() < request.steps.len() {
                        let mut failed = StepResult::failure(
                            retained.len(),
                            "Browser deadline exceeded",
                            &error,
                        );
                        failed.outcome = Some("effect_unknown".to_string());
                        retained.push(failed);
                    }
                    mark_unexecuted_steps_skipped(&mut retained, request.steps.len());
                    return Ok(ExecutionReport {
                        session_generation: Some(ownership.runtime_id().to_string()),
                        recovery: Some(BrowserRecovery {
                            incomplete: true,
                            state_lost: false,
                            effect_unknown: true,
                            retry_safe: false,
                        }),
                        ok: false,
                        steps: retained,
                        warnings: vec![error],
                        ..ExecutionReport::default()
                    });
                }
            }
        };
    }
    let mut locator_handlers = Vec::new();
    let mut all_ok = true;
    let mut pending_popup_wait: Option<PendingPopupWait> = None;
    let mut new_tabs = Vec::new();
    let mut routed_requests = Vec::new();
    let mut nested_sections = TabManagementSections::default();
    for (idx, step) in request.steps.iter().enumerate() {
        if Instant::now() >= deadline {
            all_ok = false;
            results.push(StepResult::failure(
                idx,
                "Browser request deadline exceeded",
                "No further action was dispatched; split the batch or reduce the requested waits",
            ));
            break;
        }
        if !ownership.is_current_generation() || !ownership.is_connection_epoch(connection_epoch) {
            all_ok = false;
            let mut result = StepResult::failure(
                idx,
                "Browser generation replaced",
                "Session replaced during batch; effect unknown. Submit a new request acknowledging the current session_generation; no retry was performed.",
            );
            result.outcome = Some("effect_unknown".to_string());
            results.push(result);
            break;
        }
        let step_tab_ids = runtime_before!().known_tab_ids();
        if recovery_fence.is_none() {
            if let Some(tab) = &current_tab {
                let _ = browser_block_in_place(deadline, || {
                    tab.evaluate(NETWORK_INFLIGHT_TRACKER_JS, false)
                });
            }
        }
        let file_chooser_was_armed = file_chooser_manager.is_armed();
        let fenced = recovery_fence
            .as_deref()
            .filter(|_| step_needs_live_document(step))
            .map(crate::integrations::browser_runtime::recovery_fence_error);
        let mut result = if let Some(error) = fenced {
            StepResult::failure(idx, "Browser page was lost", error)
        } else if let BrowserStep::WaitForPopup {
            timeout_ms,
        } = step
        {
            match popup_wait_conflict(pending_popup_wait.as_ref().map(|wait| wait.step_index)) {
                Some(error) => StepResult::failure(idx, "Wait for popup", error),
                None => {
                    let baseline = runtime_before!().known_tab_ids();
                    pending_popup_wait = Some(PendingPopupWait::arm(
                        idx,
                        results.len(),
                        *timeout_ms,
                        baseline,
                    ));
                    StepResult::success(idx, "Armed popup wait")
                }
            }
        } else if let BrowserStep::WaitForDownload {
            timeout_ms,
            save_as,
        } = step
        {
            let timeout = Duration::from_millis(clamp_timeout_ms(*timeout_ms));
            download_step_result(
                idx,
                browser_block_in_place(deadline, || {
                    download_monitor.wait_for_download(
                        armed_network_waits
                            .get(&idx)
                            .copied()
                            .unwrap_or_else(|| download_monitor.cursor()),
                        timeout,
                        save_as.as_deref(),
                    )
                }),
            )
        } else if let BrowserStep::WaitForConsoleMessage {
            contains,
            level,
            timeout_ms,
        } = step
        {
            match tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                wait_for_console_message(
                    &runtime_arc,
                    idx,
                    contains.as_deref(),
                    *level,
                    armed_console_cursor,
                    clamp_timeout_ms(*timeout_ms),
                ),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => StepResult::failure(
                    idx,
                    "Wait for console",
                    "Browser request deadline exceeded",
                ),
            }
        } else if let BrowserStep::CancelDownload {
            id,
        } = step
        {
            cancel_download_step_result(
                idx,
                browser_block_in_place(deadline, || {
                    download_monitor.cancel_download(id.as_deref())
                }),
            )
        } else if let BrowserStep::ReadArtifact {
            id,
            offset,
            limit,
        } = step
        {
            let mut rt = runtime_before!();
            rt.touch();
            read_artifact_step(&rt, idx, id, *offset, *limit)
        } else if matches!(step, BrowserStep::TabLog) {
            let mut rt = runtime_before!();
            rt.drain_raw_events();
            tab_log_from_buffer(idx, &rt.console_buffer)
        } else if let BrowserStep::Pdf {
            options,
        } = step
        {
            let mut rt = runtime_before!();
            let result = match rt.get_active_tab() {
                Some(tab) => browser_block_in_place(deadline, || {
                    rt.get_store().map_or_else(
                        |error| StepResult::failure(idx, "PDF artifact store", error),
                        |store| step_pdf(&tab, idx, options, &store),
                    )
                }),
                None => StepResult::failure(idx, "PDF", "No active tab"),
            };
            rt.touch();
            result
        } else if matches!(step, BrowserStep::PageContent) {
            let mut rt = runtime_before!();
            let result = match rt.get_active_tab() {
                Some(tab) => browser_block_in_place(deadline, || {
                    rt.get_store().map_or_else(
                        |error| StepResult::failure(idx, "Page artifact store", error),
                        |store| step_page_content(&tab, idx, &store),
                    )
                }),
                None => StepResult::failure(idx, "Page content", "No active tab"),
            };
            rt.touch();
            result
        } else if matches!(
            step,
            BrowserStep::AddInitScript { .. } | BrowserStep::RemoveInitScript { .. }
        ) {
            let mut rt = runtime_before!();
            let result =
                browser_block_in_place(deadline, || execute_init_script_step(&mut rt, step, idx));
            rt.touch();
            result
        } else if matches!(
            step,
            BrowserStep::Route { .. }
                | BrowserStep::Unroute { .. }
                | BrowserStep::ListRoutes
                | BrowserStep::RouteWebSocket { .. }
                | BrowserStep::UnrouteWebSocket { .. }
                | BrowserStep::SendWebSocketMessage { .. }
                | BrowserStep::CloseWebSocket { .. }
                | BrowserStep::StartHarRecording { .. }
                | BrowserStep::StopHarRecording
                | BrowserStep::RouteFromHar { .. }
        ) {
            let mut rt = runtime_before!();
            browser_block_in_place(deadline, || {
                execute_route_management_step(&mut rt, step, idx)
            })
            .unwrap()
        } else if let BrowserStep::WaitForWebSocketFrame {
            pattern,
            timeout_ms,
        } = step
        {
            let matcher = pattern
                .as_ref()
                .map(|pattern| match pattern {
                    UrlPattern::Text(value) => UrlMatcher::text(value),
                    UrlPattern::Regex {
                        source,
                        flags,
                    } => UrlMatcher::regex(source, flags),
                })
                .transpose();
            let cursor = armed_network_waits.get(&idx).copied().unwrap_or_default();
            match browser_block_in_place(deadline, || {
                matcher.and_then(|matcher| {
                    websocket_registry.wait_for_frame(
                        matcher.as_ref(),
                        cursor,
                        Duration::from_millis(clamp_timeout_ms(*timeout_ms)),
                    )
                })
            }) {
                Ok(frame) => StepResult::success(idx, "Observed WebSocket frame")
                    .with_data(serde_json::json!({"frame": frame})),
                Err(error) => StepResult::failure(idx, "Wait for WebSocket frame", error),
            }
        } else if matches!(step, BrowserStep::Reset) {
            let mut rt = runtime_before!();
            browser_block_in_place(deadline, || execute_reset_step(&mut rt, idx))
        } else if let BrowserStep::CdpSend {
            method,
            params,
            target,
        } = step
        {
            let mut rt = runtime_before!();
            browser_block_in_place(deadline, || {
                execute_cdp_send_step(&mut rt, method, params.as_ref(), *target, idx)
            })
        } else if is_screencast_step(step) {
            let mut rt = runtime_before!();
            browser_block_in_place(deadline, || {
                execute_screencast_step(&mut rt, step, idx, image_policy)
            })
        } else if let BrowserStep::SetWindowBounds {
            x,
            y,
            width,
            height,
        } = step
        {
            let mut rt = runtime_before!();
            browser_block_in_place(deadline, || {
                execute_set_window_bounds_step(&mut rt, *x, *y, *width, *height, idx)
            })
        } else if is_clock_step(step) {
            let mut rt = runtime_before!();
            browser_block_in_place(deadline, || execute_clock_step(&mut rt, step, idx))
        } else if let BrowserStep::HttpRequest {
            options,
        } = step
        {
            let prepared = {
                let mut rt = runtime_before!();
                browser_block_in_place(deadline, || prepare_http_request(&mut rt, options))
            };
            match prepared {
                Ok(mut prepared) => {
                    prepared.spec.timeout = prepared
                        .spec
                        .timeout
                        .min(deadline.saturating_duration_since(Instant::now()));
                    match http_client::send_http_request(&prepared.spec, &prepared.jar).await {
                        Ok(response) => {
                            let applied = {
                                let mut rt = runtime_before!();
                                browser_block_in_place(deadline, || {
                                    apply_http_response(&mut rt, &prepared.tab, &response)
                                })
                            };
                            match applied {
                                Ok(()) => {
                                    http_request_result(&response, options, idx, &prepared.store)
                                }
                                Err(error) => StepResult::failure(idx, "HTTP request", error),
                            }
                        }
                        Err(error) => StepResult::failure(idx, "HTTP request", error),
                    }
                }
                Err(error) => StepResult::failure(idx, "HTTP request", error),
            }
        } else if is_instrumentation_step(step) {
            let mut rt = runtime_before!();
            browser_block_in_place(deadline, || {
                execute_instrumentation_step(&mut rt, step, idx)
            })
        } else if is_context_management_step(step) {
            let mut rt = runtime_before!();
            browser_block_in_place(deadline, || {
                execute_context_management_step(&mut rt, step, idx)
            })
        } else if is_tab_management_step(step) {
            let mut rt = runtime_before!();
            let step_report = browser_block_in_place(deadline, || {
                execute_steps_with_runtime(&mut rt, std::slice::from_ref(step), image_policy)
            });
            rt.touch();
            current_tab = rt.get_active_tab();
            drop(rt);
            let nested_step = nested_sections.absorb(step_report);
            new_tabs.append(&mut nested_sections.new_tabs);
            routed_requests.append(&mut nested_sections.intercepted_requests);
            locator_handlers.append(&mut nested_sections.locator_handlers);
            nested_step.unwrap_or_else(|| {
                StepResult::failure(idx, "Browser action", "No step result produced")
            })
        } else {
            if current_tab.is_none() {
                let mut rt = runtime_before!();
                rt.touch();
                current_tab = rt.get_active_tab();
            }
            match &current_tab {
                Some(tab) => {
                    let mut rt = runtime_before!();
                    browser_block_in_place(deadline, || {
                        let mouse_state = rt
                            .mouse_states
                            .entry(tab.get_target_id().to_string())
                            .or_default();
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
                            mouse_state,
                        )
                    })
                }
                None => StepResult::failure(
                    idx,
                    "No active tab",
                    "No tab available. Use OpenTab first.",
                ),
            }
        };
        if result.ok && is_image_capture_step(step) {
            match runtime_before!().get_store() {
                Ok(store) => result = persist_image_artifact(result, &store),
                Err(error) => result = StepResult::failure(idx, "Image artifact store", error),
            }
        }
        if file_chooser_was_armed && matches!(step, BrowserStep::Click { .. }) {
            if result.ok {
                result = match &current_tab {
                    Some(tab) => browser_block_in_place(deadline, || {
                        let completed = file_chooser_manager.complete(
                            tab,
                            Duration::from_millis(settings.timing.default_wait_timeout_ms),
                        );
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
                    let _ = browser_block_in_place(deadline, || {
                        tab.set_file_chooser_dialog_interception(false, None)
                    });
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
        if !ownership.is_current_generation() || !ownership.is_connection_epoch(connection_epoch) {
            result.ok = false;
            result.outcome = Some("effect_unknown".to_string());
            result.error = Some("Browser session generation replaced during dispatch; effect unknown; no action was retried".to_string());
        }
        if terminalize_transport_result(&mut result) {
            all_ok = false;
            results.push(result);
            runtime_before!().fence_after_relaunch(
                "Browser dispatch outcome is unknown; previous document state cannot be trusted",
            );
            break;
        }
        if result.ok && recovery_fence.is_some() && restores_document_after_relaunch(step) {
            recovery_fence = None;
            runtime_before!().lift_recovery_fence();
        }

        let resolves_popup_wait = pending_popup_wait.is_some()
            && !matches!(step, BrowserStep::WaitForPopup { .. })
            && !is_tab_management_step(step);
        if !matches!(step, BrowserStep::WaitForPopup { .. }) {
            if resolves_popup_wait {
                let wait = pending_popup_wait.take().unwrap();
                let popup = loop {
                    let popup = {
                        let mut rt = runtime_before!();
                        browser_block_in_place(deadline, || {
                            refact_browser::adopt_new_tabs(&mut rt, Some(idx))
                        });
                        let popup_id = browser_block_in_place(deadline, || rt.list_tab_infos())
                            .into_iter()
                            .find(|tab| !wait.baseline.contains(&tab.id))
                            .map(|tab| tab.id);
                        popup_id.and_then(|tab_id| {
                            rt.tab_opened_by_step.entry(tab_id.clone()).or_insert(idx);
                            rt.set_active_tab_target_id(tab_id.clone());
                            browser_block_in_place(deadline, || rt.list_tab_infos())
                                .into_iter()
                                .find(|tab| tab.id == tab_id)
                        })
                    };
                    if let Some(tab) = popup {
                        break Some(tab);
                    }
                    if Instant::now() >= wait.deadline {
                        break None;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                };
                match popup {
                    Some(tab) => {
                        new_tabs.push(tab.clone());
                        if let Some(wait_result) = results.get_mut(wait.result_index) {
                            *wait_result = popup_wait_opened_result(wait.step_index, &tab.id);
                        }
                        let rt = runtime_before!();
                        current_tab = rt.get_active_tab();
                    }
                    None => {
                        if let Some(wait_result) = results.get_mut(wait.result_index) {
                            *wait_result =
                                popup_wait_timeout_result(wait.step_index, wait.timeout_ms);
                        }
                        all_ok = false;
                    }
                }
            } else if pending_popup_wait.is_none() {
                let adopted = {
                    let mut rt = runtime_before!();
                    if result.ok
                        && matches!(
                            step,
                            BrowserStep::Click { .. } | BrowserStep::ClickIfExists { .. }
                        )
                    {
                        browser_block_in_place(deadline, || {
                            refact_browser::wait_for_new_tabs(
                                &mut rt,
                                &step_tab_ids,
                                Some(idx),
                                Duration::from_millis(500),
                            )
                        })
                    } else {
                        browser_block_in_place(deadline, || {
                            refact_browser::adopt_new_tabs(&mut rt, Some(idx))
                        })
                    }
                };
                new_tabs.extend(adopted);
            }
        }

        {
            let mut rt = runtime_before!();
            rt.touch();
            let action_type = if result.ok { "action" } else { "error" };
            rt.push_agent_action(action_type, &result.summary);
            if rt.route_interception_stale() {
                if let Err(error) =
                    browser_block_in_place(deadline, || rt.reconcile_route_interception())
                {
                    tracing::warn!("Failed to reconcile route interception: {error}");
                }
            }
        }

        if step_failure_is_fatal(step, &mut result) {
            all_ok = false;
            let transport_dead = result.outcome.as_deref() == Some("effect_unknown");
            results.push(result);
            if !request.continue_on_error || transport_dead {
                break;
            }
            continue;
        }
        results.push(result);
        if !all_ok && !request.continue_on_error {
            break;
        }
    }

    if let Some(wait) = pending_popup_wait.take() {
        if let Some(wait_result) = results.get_mut(wait.result_index) {
            *wait_result = popup_wait_timeout_result(wait.step_index, wait.timeout_ms);
        }
        all_ok = false;
    }
    mark_unexecuted_steps_skipped(&mut results, request.steps.len());

    if transport_recovery(&results).is_none() {
        let mut rt = runtime_before!();
        browser_block_in_place(deadline, || refact_browser::adopt_new_tabs(&mut rt, None));
        new_tabs.extend(
            browser_block_in_place(deadline, || rt.list_tab_infos())
                .into_iter()
                .filter(|tab| !initial_tab_ids.contains(&tab.id)),
        );
    }
    new_tabs.retain(|tab| !initial_tab_ids.contains(&tab.id));
    let mut seen_new_tabs = std::collections::HashSet::new();
    new_tabs.retain(|tab| seen_new_tabs.insert(tab.id.clone()));

    let active_tab = if transport_recovery(&results).is_some() || recovery_fence.is_some() {
        None
    } else {
        match current_tab {
            Some(tab) => Some(tab),
            None => runtime_before!().get_active_tab(),
        }
    };
    let page_context_mode = request.page_context_mode();
    let snapshot_store = runtime_before!().get_store();
    let mut warnings: Vec<String> = Vec::new();
    if runtime_before!().context_state_dirty {
        warnings.push("Managed browser configuration is dirty after a partial apply; the retained snapshot describes the last known-good configuration, not verified live equivalence. Explicitly reapply intended setup or reset before relying on it.".to_string());
    }
    let (url, title, stabilized, screenshot, snapshot) = if let Some(tab) = active_tab {
        let stabilized = browser_block_in_place(deadline, || {
            wait_for_report_stability(
                &tab,
                &world,
                &network_monitor,
                REPORT_STABILIZATION_TIMEOUT_MS,
            )
        });
        let url = tab.get_url();
        let page_changed = initial_url.as_deref() != Some(url.as_str())
            || request.steps.iter().any(replaces_document_in_place);
        let capture_requested = report_screenshot_requested(
            request.attach_screenshot,
            page_context_mode,
            request
                .steps
                .iter()
                .any(|step| matches!(step, BrowserStep::Screenshot { .. })),
        );
        let screenshot = if capture_requested {
            match browser_block_in_place(deadline, || capture_report_screenshot(&tab, image_policy))
            {
                Ok(screenshot) => Some(screenshot),
                Err(error) => {
                    warnings.push(format!("screenshot capture failed: {error}"));
                    None
                }
            }
        } else {
            None
        };
        let snapshot = if report_snapshot_requested(page_context_mode, page_changed) {
            match browser_block_in_place(deadline, || {
                snapshot_store
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|store| capture_page_snapshot(&tab, &world, store))
            }) {
                Ok(snapshot) => Some(snapshot),
                Err(error) => {
                    warnings.push(format!("page snapshot capture failed: {error}"));
                    None
                }
            }
        } else {
            None
        };
        (
            Some(url),
            browser_block_in_place(deadline, || tab.get_title().ok()),
            stabilized,
            screenshot,
            snapshot,
        )
    } else {
        (None, None, false, None, None)
    };
    let (
        console,
        page_errors,
        network,
        websockets,
        dialogs,
        uploads,
        downloads,
        active_routes,
        intercepted_requests,
    ) = {
        let mut rt = runtime_before!();
        rt.drain_raw_events();
        let mut console = rt
            .flush_report_console()
            .into_iter()
            .map(mask_console_entry)
            .collect::<Vec<_>>();
        let page_errors: Vec<String> = console
            .iter()
            .filter(|entry| entry.level == "page_error")
            .map(|entry| entry.text.clone())
            .collect();
        console.retain(|entry| entry.level != "page_error");
        let network = rt.flush_report_network();
        let mut websockets = nested_sections.websockets;
        websockets.extend(rt.websocket_registry.drain_report());
        let mut dialogs = nested_sections.dialogs;
        dialogs.extend(rt.dialog_manager.take_reports());
        let mut uploads = nested_sections.uploads;
        uploads.extend(rt.file_chooser_manager.take_uploads());
        let mut downloads = nested_sections.downloads;
        downloads.extend(rt.download_monitor.take_report());
        let active_routes = rt.route_registry.list();
        routed_requests.extend(rt.route_registry.drain_interceptions());
        (
            console,
            page_errors,
            network,
            websockets,
            dialogs,
            uploads,
            downloads,
            active_routes,
            routed_requests,
        )
    };

    let page = BrowserPageContext {
        status: notable_main_document_status(&network, url.as_deref()),
        console: console_counts(&console, &page_errors),
        snapshot,
    };
    let (session_generation, context) = {
        let runtime = runtime_before!();
        let context = if !all_ok || recovery_fence.is_some() || runtime.context_state_dirty {
            None
        } else {
            Some(browser_block_in_place(deadline, || {
                context_summary(&runtime)
            }))
        };
        (Some(runtime.runtime_id.clone()), context)
    };
    let mut report = ExecutionReport {
        session_generation,
        recovery: transport_recovery(&results),
        ok: all_ok,
        steps: results,
        warnings,
        url,
        title,
        page: (page != BrowserPageContext::default()).then_some(page),
        stabilized,
        console,
        page_errors,
        network,
        network_summary: Vec::new(),
        websockets,
        locator_handlers,
        dialogs,
        uploads,
        downloads,
        new_tabs,
        active_routes,
        intercepted_requests,
        context,
        screenshot,
    };
    apply_network_report_mode(&mut report, request.network);
    if !ownership.is_current_generation() || !ownership.is_connection_epoch(connection_epoch) {
        report.ok = false;
        report.recovery = Some(BrowserRecovery {
            incomplete: true,
            state_lost: true,
            effect_unknown: true,
            retry_safe: false,
        });
        report.context = None;
        report.page = None;
        report.screenshot = None;
        report.warnings.push(
            "Report belongs to a retired browser connection; no further action was dispatched"
                .to_string(),
        );
    } else if report.ok {
        let mut runtime = match lock_browser_before(&runtime_arc, deadline).await {
            Ok(runtime) => runtime,
            Err(error) => {
                report.warnings.push(error);
                return Ok(report);
            }
        };
        if let Err(error) =
            browser_block_in_place(deadline, || runtime.checkpoint_document_identity())
        {
            report
                .warnings
                .push(format!("Reconnect checkpoint unavailable: {error}"));
        }
    }
    Ok(report)
}

async fn validate_upload_paths(
    gcx: Arc<GlobalContext>,
    request: &BrowserActionRequest,
) -> Result<(), String> {
    for (idx, step) in request.steps.iter().enumerate() {
        let (action, paths) = match step {
            BrowserStep::SetInputFiles {
                paths, ..
            } => ("set_input_files", paths.as_slice()),
            BrowserStep::ExpectFileChooser {
                paths,
            } => ("expect_file_chooser", paths.as_slice()),
            BrowserStep::DropFiles {
                paths, ..
            } => ("drop_files", paths.as_slice()),
            _ => continue,
        };
        for path in paths {
            validate_upload_path(gcx.clone(), path)
                .await
                .map_err(|error| format!("step[{idx}] ({action}): {error}"))?;
        }
    }
    Ok(())
}

async fn validate_upload_path(gcx: Arc<GlobalContext>, path: &str) -> Result<(), String> {
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
    Ok(())
}

fn resolve_pending_popup(
    runtime: &mut BrowserRuntime,
    wait: &PendingPopupWait,
    step_index: usize,
) -> Option<TabInfo> {
    loop {
        refact_browser::adopt_new_tabs(runtime, Some(step_index));
        let popup_id = runtime
            .list_tab_infos()
            .into_iter()
            .find(|tab| !wait.baseline.contains(&tab.id))
            .map(|tab| tab.id);
        if let Some(tab_id) = popup_id {
            runtime
                .tab_opened_by_step
                .entry(tab_id.clone())
                .or_insert(step_index);
            runtime.set_active_tab_target_id(tab_id.clone());
            return runtime
                .list_tab_infos()
                .into_iter()
                .find(|tab| tab.id == tab_id);
        }
        if Instant::now() >= wait.deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub fn execute_steps_with_runtime(
    runtime: &mut BrowserRuntime,
    steps: &[BrowserStep],
    image_policy: &ImagePolicy,
) -> ExecutionReport {
    let settings = browser_settings::current();
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
    let mut pending_popup_wait: Option<PendingPopupWait> = None;
    let mut pre_step_url: Option<String> = current_tab.as_ref().map(|t| t.get_url());
    let armed_download_waits = steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| match step {
            BrowserStep::WaitForDownload {
                ..
            } => Some((index, runtime.download_monitor.cursor())),
            _ => None,
        })
        .collect::<std::collections::HashMap<_, _>>();

    for (idx, step) in steps.iter().enumerate() {
        let step_tab_ids = runtime.known_tab_ids();
        let mut result = match step {
            BrowserStep::OpenTab {
                device,
                url,
            } => match open_tab_for_device(runtime, device) {
                Ok((descriptor, new_tab)) => {
                    let device_label = descriptor.name.clone();
                    let target_id = new_tab.get_target_id().to_string();
                    let mut candidate = runtime.context_state.clone();
                    candidate.viewport = Some(descriptor.viewport.clone());
                    candidate.user_agent = Some((descriptor.user_agent.clone(), None));
                    if let Err(error) = apply_context_to_tabs(runtime, &candidate) {
                        runtime.context_state_dirty = true;
                        let _ = new_tab.close(false);
                        all_ok = false;
                        results.push(StepResult::failure(
                            idx,
                            "OpenTab configuration partially applied",
                            error,
                        ));
                        break;
                    }
                    runtime.context_state = candidate;
                    if let Err(error) =
                        crate::integrations::browser_runtime::setup_recording_for_tab(
                            runtime,
                            new_tab.clone(),
                        )
                    {
                        let _ = new_tab.close(false);
                        all_ok = false;
                        results.push(StepResult::failure(idx, "OpenTab", error));
                        break;
                    }
                    let navigation = url.as_ref().map(|url| {
                        run_and_wait_for_navigation(
                            &new_tab,
                            settings.timing.default_wait_timeout_ms,
                            || trigger_page_navigation(&new_tab, url),
                        )
                    });
                    let _ = new_tab.evaluate(INSPECT_ELEMENT_JS, false);
                    current_tab = Some(new_tab);
                    runtime.set_active_tab_target_id(target_id.clone());
                    match navigation.transpose() {
                        Err(error) => StepResult::failure(idx, "OpenTab", error),
                        Ok(warning) => merge_step_data(
                            navigation_step_success(
                                idx,
                                format!(
                                    "Opened new {} tab ({})",
                                    device_label,
                                    &target_id[..8.min(target_id.len())]
                                ),
                                warning.flatten(),
                            ),
                            serde_json::json!({"tab_id": target_id}),
                        ),
                    }
                }
                Err(error) => StepResult::failure(idx, "OpenTab", error),
            },
            BrowserStep::CloseTab {
                tab: target,
            } => {
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
                        runtime.world_manager.tab_closed(&target_id);
                        runtime.mouse_states.remove(&target_id);
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
            BrowserStep::SwitchTab {
                tab: tab_target,
            } => {
                let tabs = runtime
                    .browser
                    .get_tabs()
                    .lock()
                    .map(|t| t.clone())
                    .unwrap_or_default();
                let target_str = match tab_target {
                    TabTarget::Active => String::from("active"),
                    TabTarget::Id {
                        id,
                    } => id.clone(),
                };
                let found = match tab_target {
                    TabTarget::Active => runtime.get_active_tab().or_else(|| tabs.first().cloned()),
                    TabTarget::Id {
                        id,
                    } => tabs
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
            step @ (BrowserStep::Route {
                ..
            }
            | BrowserStep::Unroute {
                ..
            }
            | BrowserStep::ListRoutes
            | BrowserStep::RouteWebSocket {
                ..
            }
            | BrowserStep::UnrouteWebSocket {
                ..
            }
            | BrowserStep::SendWebSocketMessage {
                ..
            }
            | BrowserStep::CloseWebSocket {
                ..
            }
            | BrowserStep::StartHarRecording {
                ..
            }
            | BrowserStep::StopHarRecording
            | BrowserStep::RouteFromHar {
                ..
            }) => execute_route_management_step(runtime, step, idx).unwrap(),
            BrowserStep::Reset => execute_reset_step(runtime, idx),
            BrowserStep::CdpSend {
                method,
                params,
                target,
            } => execute_cdp_send_step(runtime, method, params.as_ref(), *target, idx),
            step if is_screencast_step(step) => {
                execute_screencast_step(runtime, step, idx, image_policy)
            }
            BrowserStep::SetWindowBounds {
                x,
                y,
                width,
                height,
            } => execute_set_window_bounds_step(runtime, *x, *y, *width, *height, idx),
            BrowserStep::HttpRequest {
                options,
            } => tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(execute_http_request_step(runtime, options, idx))
            }),
            step if is_instrumentation_step(step) => {
                execute_instrumentation_step(runtime, step, idx)
            }
            step if is_context_management_step(step) => {
                execute_context_management_step(runtime, step, idx)
            }
            BrowserStep::WaitForPopup {
                timeout_ms,
            } => {
                match popup_wait_conflict(pending_popup_wait.as_ref().map(|wait| wait.step_index)) {
                    Some(error) => StepResult::failure(idx, "Wait for popup", error),
                    None => {
                        pending_popup_wait = Some(PendingPopupWait::arm(
                            idx,
                            results.len(),
                            *timeout_ms,
                            runtime.known_tab_ids(),
                        ));
                        StepResult::success(idx, "Armed popup wait")
                    }
                }
            }
            BrowserStep::WaitForNetworkIdle {
                timeout_ms,
            } => wait_for_network_idle_step(
                &runtime.network_monitor,
                idx,
                clamp_timeout_ms(*timeout_ms),
            ),
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
            BrowserStep::ExpectFileChooser {
                paths,
            } => {
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
            } => download_step_result(
                idx,
                runtime.download_monitor.wait_for_download(
                    armed_download_waits
                        .get(&idx)
                        .copied()
                        .unwrap_or_else(|| runtime.download_monitor.cursor()),
                    Duration::from_millis(clamp_timeout_ms(*timeout_ms)),
                    save_as.as_deref(),
                ),
            ),
            BrowserStep::CancelDownload {
                id,
            } => cancel_download_step_result(
                idx,
                runtime.download_monitor.cancel_download(id.as_deref()),
            ),
            BrowserStep::Pdf {
                options,
            } => match &current_tab {
                Some(tab) => runtime.get_store().map_or_else(
                    |error| StepResult::failure(idx, "PDF artifact store", error),
                    |store| step_pdf(tab, idx, options, &store),
                ),
                None => StepResult::failure(idx, "PDF", "No active tab"),
            },
            BrowserStep::PageContent => match &current_tab {
                Some(tab) => runtime.get_store().map_or_else(
                    |error| StepResult::failure(idx, "Page artifact store", error),
                    |store| step_page_content(tab, idx, &store),
                ),
                None => StepResult::failure(idx, "Page content", "No active tab"),
            },
            BrowserStep::ReadArtifact {
                id,
                offset,
                limit,
            } => read_artifact_step(runtime, idx, id, *offset, *limit),
            BrowserStep::TabLog => {
                runtime.drain_raw_events();
                tab_log_from_buffer(idx, &runtime.console_buffer)
            }
            BrowserStep::AddInitScript {
                ..
            }
            | BrowserStep::RemoveInitScript {
                ..
            } => execute_init_script_step(runtime, step, idx),
            BrowserStep::Help {
                topic,
            } => StepResult::success(idx, "chrome help").with_data(serde_json::json!({
                "text": crate::tools::tool_chrome::chrome_help_text(topic.as_deref()),
            })),
            other => match &current_tab {
                Some(tab) => {
                    let chooser_was_armed = runtime.file_chooser_manager.is_armed();
                    let world = runtime.world_manager.clone();
                    let mut result = execute_single_step(
                        tab,
                        &world,
                        other,
                        idx,
                        pre_step_url.as_deref(),
                        image_policy,
                        Some(&handlers),
                        &mut locator_handlers,
                        runtime
                            .mouse_states
                            .entry(tab.get_target_id().to_string())
                            .or_default(),
                    );
                    if chooser_was_armed && matches!(other, BrowserStep::Click { .. }) {
                        if result.ok {
                            let completed = runtime.file_chooser_manager.complete(
                                tab,
                                Duration::from_millis(settings.timing.default_wait_timeout_ms),
                            );
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
                    if result.ok && is_image_capture_step(other) {
                        result = match runtime.get_store() {
                            Ok(store) => persist_image_artifact(result, &store),
                            Err(error) => StepResult::failure(idx, "Image artifact store", error),
                        };
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

        let resolves_popup_wait = pending_popup_wait.is_some()
            && !matches!(step, BrowserStep::WaitForPopup { .. })
            && !is_tab_management_step(step);
        if resolves_popup_wait {
            let wait = pending_popup_wait.take().unwrap();
            match resolve_pending_popup(runtime, &wait, idx) {
                Some(tab) => {
                    current_tab = runtime.get_active_tab();
                    new_tabs.push(tab.clone());
                    if let Some(wait_result) = results.get_mut(wait.result_index) {
                        *wait_result = popup_wait_opened_result(wait.step_index, &tab.id);
                    }
                }
                None => {
                    if let Some(wait_result) = results.get_mut(wait.result_index) {
                        *wait_result = popup_wait_timeout_result(wait.step_index, wait.timeout_ms);
                    }
                    all_ok = false;
                }
            }
        }

        if step_failure_is_fatal(step, &mut result) {
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
        if !resolves_popup_wait && pending_popup_wait.is_none() {
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
        }
        results.push(result);
        if !all_ok {
            break;
        }
    }

    if let Some(wait) = pending_popup_wait.take() {
        if let Some(wait_result) = results.get_mut(wait.result_index) {
            *wait_result = popup_wait_timeout_result(wait.step_index, wait.timeout_ms);
        }
        all_ok = false;
    }
    mark_unexecuted_steps_skipped(&mut results, steps.len());

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
        session_generation: None,
        recovery: transport_recovery(&results),
        ok: all_ok,

        steps: results,
        warnings: Vec::new(),
        url,
        title,
        page: None,
        stabilized: false,
        console: vec![],
        page_errors: vec![],
        network: vec![],
        network_summary: vec![],
        websockets: runtime.websocket_registry.drain_report(),
        locator_handlers,
        dialogs,
        uploads,
        downloads,
        new_tabs,
        active_routes: runtime.route_registry.list(),
        intercepted_requests: runtime.route_registry.drain_interceptions(),
        context: Some(context_summary(runtime)),
        screenshot: None,
    }
}

fn is_non_fatal_step(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::ClickIfExists { .. }
            | BrowserStep::Expect {
                soft: true,
                ..
            }
            | BrowserStep::ExpectPoll {
                soft: Some(true),
                ..
            }
    )
}

fn mark_unexecuted_steps_skipped(results: &mut Vec<StepResult>, total_steps: usize) {
    let executed = results
        .iter()
        .map(|step| step.step_index + 1)
        .max()
        .unwrap_or(0);
    if executed >= total_steps {
        return;
    }
    let failed_step_index = results
        .iter()
        .rev()
        .find(|result| !result.ok)
        .map(|result| result.step_index)
        .unwrap_or(executed.saturating_sub(1));
    results.extend((executed..total_steps).map(|idx| StepResult::skipped(idx, failed_step_index)));
}

fn replaces_document_in_place(step: &BrowserStep) -> bool {
    matches!(step, BrowserStep::SetContent { .. })
}

fn is_navigation_step(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::Navigate { .. }
            | BrowserStep::Reload
            | BrowserStep::GoBack
            | BrowserStep::GoForward
            | BrowserStep::SetContent { .. }
    )
}

fn read_artifact_step(
    runtime: &BrowserRuntime,
    idx: usize,
    id: &str,
    offset: Option<u64>,
    limit: Option<u64>,
) -> StepResult {
    let offset = match usize::try_from(offset.unwrap_or(0)) {
        Ok(value) => value,
        Err(_) => {
            return StepResult::failure(idx, "Read browser artifact", "Invalid artifact offset");
        }
    };
    let limit = limit.unwrap_or(8192).clamp(1, 32768) as usize;
    match runtime
        .get_store()
        .and_then(|store| store.read_report(id, offset, limit))
    {
        Ok(chunk) => {
            let mut result =
                StepResult::success(idx, "Read browser artifact").with_data(serde_json::json!({
                    "id": id,
                    "content_base64": base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        chunk.content.as_bytes(),
                    ),
                    "encoding": "base64",
                    "total_bytes": chunk.total_bytes,
                    "next_offset": chunk.next_offset,
                }));
            result.outcome = Some("artifact_read".to_string());
            result
        }
        Err(error) => StepResult::failure(idx, "Read browser artifact", error),
    }
}

fn step_requires_session_ack(step: &BrowserStep) -> bool {
    !matches!(
        step,
        BrowserStep::Help { .. }
            | BrowserStep::ListTabs
            | BrowserStep::ReadArtifact { .. }
            | BrowserStep::ListDevices { .. }
            | BrowserStep::SetViewport { .. }
            | BrowserStep::EmulateMedia { .. }
            | BrowserStep::SetLocale { .. }
            | BrowserStep::SetTimezone { .. }
            | BrowserStep::SetUserAgent { .. }
            | BrowserStep::SetGeolocation { .. }
            | BrowserStep::SetOffline { .. }
            | BrowserStep::SetNetworkConditions { .. }
            | BrowserStep::SetCpuThrottling { .. }
            | BrowserStep::EmulateDevice { .. }
            | BrowserStep::SetExtraHttpHeaders { .. }
    )
}

fn step_needs_live_document(step: &BrowserStep) -> bool {
    step_requires_session_ack(step) && !restores_document_after_relaunch(step)
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
    mouse_state: &mut MouseState,
) -> StepResult {
    match step {
        BrowserStep::SetContent {
            html,
            wait_until,
        } => step_set_content(tab, world, network_monitor, idx, html, *wait_until),
        BrowserStep::WaitForNetworkIdle {
            timeout_ms,
        } => wait_for_network_idle_step(network_monitor, idx, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitForLoadState {
            state,
            timeout_ms,
        } => wait_for_load_state(network_monitor, idx, *state, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitForRequest {
            pattern,
            method,
            timeout_ms,
        } => wait_for_network_entry(
            network_monitor,
            idx,
            pattern,
            &NetworkWaitFilters {
                method: method.clone(),
                status: None,
            },
            armed_cursor.unwrap_or_else(|| network_monitor.request_cursor()),
            clamp_timeout_ms(*timeout_ms),
            false,
        ),
        BrowserStep::WaitForResponse {
            pattern,
            method,
            status,
            timeout_ms,
        } => wait_for_network_entry(
            network_monitor,
            idx,
            pattern,
            &NetworkWaitFilters {
                method: method.clone(),
                status: *status,
            },
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
            mouse_state,
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
    wait_for_load_state_labelled(
        monitor,
        idx,
        state,
        timeout_ms,
        &format!("Wait for load state {state_name}"),
    )
}

fn wait_for_network_idle_step(
    monitor: &NetworkMonitorHandle,
    idx: usize,
    timeout_ms: u64,
) -> StepResult {
    wait_for_load_state_labelled(
        monitor,
        idx,
        BrowserLoadState::Networkidle,
        timeout_ms,
        "Wait for network idle",
    )
}

fn wait_for_load_state_labelled(
    monitor: &NetworkMonitorHandle,
    idx: usize,
    state: BrowserLoadState,
    timeout_ms: u64,
    summary: &str,
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
        Err(error) => StepResult::failure(idx, summary, error),
    }
}

fn wait_for_network_entry(
    monitor: &NetworkMonitorHandle,
    idx: usize,
    pattern: &UrlPattern,
    filters: &NetworkWaitFilters,
    cursor: u64,
    timeout_ms: u64,
    response: bool,
) -> StepResult {
    let matcher = match pattern {
        UrlPattern::Text(value) => UrlMatcher::text(value),
        UrlPattern::Regex {
            source,
            flags,
        } => UrlMatcher::regex(source, flags),
    };
    let matcher = match matcher {
        Ok(matcher) => matcher,
        Err(error) => return StepResult::failure(idx, "Invalid URL pattern", error),
    };
    let result = if response {
        monitor.wait_for_response(&matcher, filters, cursor, Duration::from_millis(timeout_ms))
    } else {
        monitor.wait_for_request(&matcher, filters, cursor, Duration::from_millis(timeout_ms))
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
            | BrowserStep::Tap {
                locator: Some(_),
                ..
            }
            | BrowserStep::Hover { .. }
            | BrowserStep::DragAndDrop { .. }
            | BrowserStep::DropFiles { .. }
            | BrowserStep::Focus { .. }
            | BrowserStep::InsertText {
                locator: Some(_),
                ..
            }
            | BrowserStep::PressSequentially { .. }
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
            | BrowserStep::Expect {
                locator: Some(_),
                ..
            }
            | BrowserStep::GetText { .. }
            | BrowserStep::GetHtml { .. }
            | BrowserStep::GetAttribute { .. }
            | BrowserStep::BoundingBox { .. }
            | BrowserStep::InputValue { .. }
            | BrowserStep::ElementState { .. }
            | BrowserStep::ScreenshotElement { .. }
            | BrowserStep::ScreenshotElements { .. }
            | BrowserStep::CaptureElementStates { .. }
            | BrowserStep::Styles { .. }
            | BrowserStep::HighlightElement { .. }
            | BrowserStep::Highlight { .. }
            | BrowserStep::Annotate { .. }
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
    let settings = browser_settings::current();
    let mut last_error: Option<String>;
    loop {
        match eval_js_value(tab, "document.readyState === 'loading'") {
            Ok(value) => {
                if !value.as_bool().unwrap_or(false) {
                    return Ok(());
                }
                last_error = None;
            }
            Err(error) if refact_browser::is_transport_dead_error(&error) => {
                return Err(format!("Polling failed: {error}"));
            }
            Err(error) => last_error = Some(error),
        }
        if Instant::now() >= deadline {
            return Err(poll_deadline_error(
                last_error,
                "Timed out waiting for navigation before action".to_string(),
            ));
        }
        std::thread::sleep(
            Duration::from_millis(settings.timing.default_poll_interval_ms)
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
            LocatorHandlerProbe::Hidden
            | LocatorHandlerProbe::MultipleMatches {
                ..
            } => continue,
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
            false,
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
        return eval_js_ok_in_utility(
            tab,
            world,
            &browser_locators::js_dismiss_overlays(true, false),
        )
        .map(|result| {
            if result
                .get("count")
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
        return Ok(LocatorHandlerProbe::MultipleMatches {
            count,
        });
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
    aggressive: bool,
) -> Result<String, String> {
    let outcome = match &lease.handler.operation {
        LocatorHandlerOperation::DismissOverlays => dismiss_overlays(tab, world, aggressive),
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
        LocatorHandlerOperation::Action(LocatorHandlerAction::Steps {
            steps,
        }) => {
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
                    &mut MouseState::default(),
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
        BrowserStep::WaitForSelector {
            locator,
            state,
            ..
        } => BrowserStep::WaitForSelector {
            locator: locator.clone(),
            state: *state,
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitForNavigation {
            ..
        } => BrowserStep::WaitForNavigation {
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitForUrl {
            pattern, ..
        } => BrowserStep::WaitForUrl {
            pattern: pattern.clone(),
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitForText {
            text, ..
        } => BrowserStep::WaitForText {
            text: text.clone(),
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::Expect {
            locator,
            matcher,
            soft,
            not,
            ..
        } => BrowserStep::Expect {
            locator: locator.clone(),
            matcher: matcher.clone(),
            timeout_ms: Some(remaining_ms),
            soft: *soft,
            not: *not,
        },
        BrowserStep::WaitForNetworkIdle {
            ..
        } => BrowserStep::WaitForNetworkIdle {
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitForElementHidden {
            locator, ..
        } => BrowserStep::WaitForElementHidden {
            locator: locator.clone(),
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitForElementStable {
            locator, ..
        } => BrowserStep::WaitForElementStable {
            locator: locator.clone(),
            timeout_ms: Some(remaining_ms),
        },
        BrowserStep::WaitSeconds {
            seconds,
        } => BrowserStep::WaitSeconds {
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
    let settings = browser_settings::current();
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
            Duration::from_millis(settings.timing.default_poll_interval_ms)
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
    mouse_state: &mut MouseState,
) -> StepResult {
    let settings = browser_settings::current();
    if needs_locator_handler_checkpoint(step) && !uses_actionability_engine(step) {
        if let Some(handlers) = handlers {
            if let Err(error) = perform_action_prechecks(
                tab,
                world,
                handlers,
                locator_handler_firings,
                image_policy,
                Instant::now() + Duration::from_millis(settings.timing.default_wait_timeout_ms),
            ) {
                return StepResult::failure(idx, "Locator handler checkpoint failed", error);
            }
        }
    }
    match step {
        BrowserStep::Navigate {
            url,
            timeout_ms,
        } => step_navigate(tab, idx, url, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::Reload => step_nav_js(tab, idx, "location.reload()", "Reloaded page"),
        BrowserStep::GoBack => step_nav_js(tab, idx, "history.back()", "Navigated back"),
        BrowserStep::GoForward => step_nav_js(tab, idx, "history.forward()", "Navigated forward"),

        BrowserStep::OpenTab {
            ..
        }
        | BrowserStep::CloseTab {
            ..
        }
        | BrowserStep::SwitchTab {
            ..
        }
        | BrowserStep::ListTabs
        | BrowserStep::WaitForPopup {
            ..
        }
        | BrowserStep::Route {
            ..
        }
        | BrowserStep::Unroute {
            ..
        }
        | BrowserStep::ListRoutes
        | BrowserStep::RouteWebSocket {
            ..
        }
        | BrowserStep::UnrouteWebSocket {
            ..
        }
        | BrowserStep::SendWebSocketMessage {
            ..
        }
        | BrowserStep::CloseWebSocket {
            ..
        }
        | BrowserStep::WaitForWebSocketFrame {
            ..
        }
        | BrowserStep::StartHarRecording {
            ..
        }
        | BrowserStep::StopHarRecording
        | BrowserStep::RouteFromHar {
            ..
        }
        | BrowserStep::Reset
        | BrowserStep::CdpSend {
            ..
        }
        | BrowserStep::CaptureFrames {
            ..
        }
        | BrowserStep::ScreencastStart {
            ..
        }
        | BrowserStep::ScreencastStop {
            ..
        }
        | BrowserStep::HttpRequest {
            ..
        }
        | BrowserStep::StartCoverage {
            ..
        }
        | BrowserStep::StopCoverage
        | BrowserStep::AddVirtualAuthenticator {
            ..
        }
        | BrowserStep::RemoveVirtualAuthenticator {
            ..
        }
        | BrowserStep::ListCredentials {
            ..
        }
        | BrowserStep::AddCredential {
            ..
        }
        | BrowserStep::ClearCredentials {
            ..
        }
        | BrowserStep::SetUserVerified {
            ..
        }
        | BrowserStep::SetViewport {
            ..
        }
        | BrowserStep::SetWindowBounds {
            ..
        }
        | BrowserStep::EmulateMedia {
            ..
        }
        | BrowserStep::SetLocale {
            ..
        }
        | BrowserStep::SetTimezone {
            ..
        }
        | BrowserStep::SetUserAgent {
            ..
        }
        | BrowserStep::SetGeolocation {
            ..
        }
        | BrowserStep::SetOffline {
            ..
        }
        | BrowserStep::SetNetworkConditions {
            ..
        }
        | BrowserStep::SetCpuThrottling {
            ..
        }
        | BrowserStep::EmulateDevice {
            ..
        }
        | BrowserStep::ListDevices {
            ..
        }
        | BrowserStep::SetExtraHttpHeaders {
            ..
        }
        | BrowserStep::GetCookies {
            ..
        }
        | BrowserStep::SetCookies {
            ..
        }
        | BrowserStep::ClearCookies {
            ..
        }
        | BrowserStep::GetStorage {
            ..
        }
        | BrowserStep::SetStorage {
            ..
        }
        | BrowserStep::ClearStorage {
            ..
        }
        | BrowserStep::StorageState {
            ..
        }
        | BrowserStep::SetStorageState {
            ..
        }
        | BrowserStep::GrantPermissions {
            ..
        }
        | BrowserStep::ClearPermissions
        | BrowserStep::SetHttpCredentials {
            ..
        }
        | BrowserStep::HandleDialog {
            ..
        } => StepResult::failure(
            idx,
            "Runtime management step",
            "Use execute_steps_with_runtime() for runtime management",
        ),

        BrowserStep::Expect {
            locator,
            matcher,
            timeout_ms,
            soft,
            not,
        } => step_expect(
            tab,
            world,
            idx,
            locator.as_ref(),
            matcher,
            timeout_ms.unwrap_or(ActionabilityTimeouts::default().expect.as_millis() as u64),
            *soft,
            not.unwrap_or(false),
        ),

        BrowserStep::ExpectPoll {
            expression,
            expected,
            matcher,
            timeout_ms,
            soft,
        } => step_expect_poll(
            tab,
            idx,
            expression,
            expected,
            *matcher,
            clamp_timeout_ms(*timeout_ms),
            soft.unwrap_or(false),
        ),

        BrowserStep::Click {
            locator,
            timeout_ms,
        } => step_locator_action_with_timeout(
            tab,
            world,
            idx,
            locator,
            "click",
            handlers,
            locator_handler_firings,
            image_policy,
            *timeout_ms,
        ),
        BrowserStep::ClickIfExists {
            locator,
        } => step_click_if_exists(
            tab,
            world,
            idx,
            locator,
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::Tap {
            locator,
            x,
            y,
        } => step_tap(
            tab,
            world,
            idx,
            locator.as_ref(),
            *x,
            *y,
            handlers,
            locator_handler_firings,
            image_policy,
            mouse_state,
        ),
        BrowserStep::Hover {
            locator,
            timeout_ms,
        } => step_locator_action_with_timeout(
            tab,
            world,
            idx,
            locator,
            "hover",
            handlers,
            locator_handler_firings,
            image_policy,
            *timeout_ms,
        ),
        BrowserStep::Focus {
            locator,
        } => step_locator_action(
            tab,
            world,
            idx,
            locator,
            "focus",
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::Blur {
            locator,
        } => step_locator_action(
            tab,
            world,
            idx,
            locator,
            "blur",
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::ScrollTo {
            locator,
        } => step_locator_action(
            tab,
            world,
            idx,
            locator,
            "scroll_to",
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::PressKey {
            key,
            modifiers,
        } => step_press_key(tab, idx, key, modifiers),
        BrowserStep::InsertText {
            locator,
            text,
        } => step_insert_text(
            tab,
            world,
            idx,
            locator.as_ref(),
            text,
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::PressSequentially {
            locator,
            text,
            delay_ms,
        } => step_press_sequentially(
            tab,
            world,
            idx,
            locator,
            text,
            *delay_ms,
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::DragAndDrop {
            source,
            target,
            source_position,
            target_position,
        } => step_drag_and_drop(
            tab,
            world,
            idx,
            source,
            target,
            *source_position,
            *target_position,
            handlers,
            locator_handler_firings,
            image_policy,
            mouse_state,
        ),
        BrowserStep::DropFiles {
            target,
            paths,
        } => step_drop_files(
            tab,
            world,
            idx,
            target,
            paths,
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        BrowserStep::MouseMove {
            x,
            y,
            steps,
        } => step_mouse(
            tab,
            idx,
            mouse_state,
            |mouse| mouse.move_to(*x, *y, steps.unwrap_or(1)),
            format!("Moved mouse to ({x}, {y})"),
        ),
        BrowserStep::MouseDown {
            button,
        } => step_mouse(
            tab,
            idx,
            mouse_state,
            |mouse| mouse.down(browser_mouse_button(*button), 1),
            format!("Pressed {button:?} mouse button"),
        ),
        BrowserStep::MouseUp {
            button,
        } => step_mouse(
            tab,
            idx,
            mouse_state,
            |mouse| mouse.up(browser_mouse_button(*button), 1),
            format!("Released {button:?} mouse button"),
        ),
        BrowserStep::MouseClickXy {
            x,
            y,
            button,
            click_count,
            delay,
        } => step_mouse(
            tab,
            idx,
            mouse_state,
            |mouse| {
                mouse.move_to(*x, *y, 1)?;
                for count in 1..=click_count.unwrap_or(1) {
                    mouse.down(browser_mouse_button(*button), count)?;
                    if let Some(delay) = delay {
                        std::thread::sleep(Duration::from_millis(*delay));
                    }
                    mouse.up(browser_mouse_button(*button), count)?;
                }
                Ok(())
            },
            format!("Clicked at ({x}, {y})"),
        ),
        BrowserStep::MouseDragXy {
            start_x,
            start_y,
            end_x,
            end_y,
        } => step_mouse(
            tab,
            idx,
            mouse_state,
            |mouse| {
                mouse.move_to(*start_x, *start_y, 1)?;
                mouse.down(MouseButton::Left, 1)?;
                mouse.move_to(*end_x, *end_y, 2)?;
                mouse.up(MouseButton::Left, 1)
            },
            format!("Dragged mouse from ({start_x}, {start_y}) to ({end_x}, {end_y})"),
        ),
        BrowserStep::MouseWheel {
            delta_x,
            delta_y,
        } => step_mouse(
            tab,
            idx,
            mouse_state,
            |mouse| mouse.wheel(*delta_x, *delta_y),
            format!("Scrolled mouse wheel by ({delta_x}, {delta_y})"),
        ),

        BrowserStep::Fill {
            locator,
            text,
            clear_first,
            verify,
        } => step_fill(tab, world, idx, locator, text, *clear_first, *verify),
        BrowserStep::Clear {
            locator,
            verify,
        } => step_clear(tab, world, idx, locator, *verify),
        BrowserStep::SelectOption {
            locator,
            value,
        } => step_select_option(tab, world, idx, locator, value),
        BrowserStep::Check {
            locator,
        } => step_check_uncheck(tab, world, idx, locator, true),
        BrowserStep::Uncheck {
            locator,
        } => step_check_uncheck(tab, world, idx, locator, false),
        BrowserStep::SetInputFiles {
            locator,
            paths,
        } => step_set_input_files(tab, world, idx, locator, paths),
        BrowserStep::ExpectFileChooser {
            ..
        } => StepResult::failure(
            idx,
            "Expect file chooser",
            "File chooser flow requires a browser runtime",
        ),

        BrowserStep::WaitForFunction {
            expression,
            locator,
            timeout_ms,
            polling_ms,
        } => step_wait_for_function(
            tab,
            world,
            idx,
            expression,
            locator.as_ref(),
            clamp_timeout_ms(*timeout_ms),
            *polling_ms,
        ),
        BrowserStep::WaitForSelector {
            locator,
            state,
            timeout_ms,
        } => step_wait_for_selector(
            tab,
            world,
            idx,
            locator,
            *state,
            clamp_timeout_ms(*timeout_ms),
        ),
        BrowserStep::WaitForNavigation {
            timeout_ms,
        } => step_wait_for_navigation(tab, idx, clamp_timeout_ms(*timeout_ms), pre_step_url),
        BrowserStep::WaitForUrl {
            pattern,
            timeout_ms,
        } => step_wait_for_url(tab, idx, pattern, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitForText {
            text,
            timeout_ms,
        } => step_wait_for_text(tab, idx, text, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitForNetworkIdle {
            timeout_ms,
        } => step_wait_for_network_idle(tab, idx, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitForLoadState {
            ..
        }
        | BrowserStep::WaitForRequest {
            ..
        }
        | BrowserStep::WaitForResponse {
            ..
        }
        | BrowserStep::WaitForDownload {
            ..
        }
        | BrowserStep::CancelDownload {
            ..
        } => StepResult::failure(
            idx,
            "Network wait",
            "Network waits require a browser runtime",
        ),
        BrowserStep::WaitForConsoleMessage {
            ..
        } => StepResult::failure(
            idx,
            "Wait for console message",
            "Console waits require a browser runtime",
        ),
        BrowserStep::WaitForElementHidden {
            locator,
            timeout_ms,
        } => step_wait_for_element_hidden(tab, world, idx, locator, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitForElementStable {
            locator,
            timeout_ms,
        } => step_wait_for_element_stable(tab, world, idx, locator, clamp_timeout_ms(*timeout_ms)),
        BrowserStep::WaitSeconds {
            seconds,
        } => step_wait_seconds(idx, clamp_wait_seconds(*seconds)),

        BrowserStep::GetText {
            locator,
        } => step_get_text(tab, world, idx, locator),
        BrowserStep::GetHtml {
            locator,
        } => step_get_html(tab, world, idx, locator),
        BrowserStep::GetAttribute {
            locator,
            attribute,
        } => step_get_attribute(tab, world, idx, locator, attribute),
        BrowserStep::BoundingBox {
            locator,
        } => step_bounding_box(tab, world, idx, locator),
        BrowserStep::Count {
            locator,
        } => step_count(tab, world, idx, locator),
        BrowserStep::InputValue {
            locator,
        } => step_input_value(tab, world, idx, locator),
        BrowserStep::AllTexts {
            locator,
            mode,
            limit,
        } => step_all_texts(tab, world, idx, locator, *mode, *limit),
        BrowserStep::ElementState {
            locator,
        } => step_element_state(tab, world, idx, locator),
        BrowserStep::ExtractLinks {
            locator,
            limit,
        } => step_extract_links(tab, world, idx, locator.as_ref(), *limit),
        BrowserStep::ExtractTable {
            locator,
            limit,
        } => step_extract_table(tab, world, idx, locator, *limit),
        BrowserStep::DomSnapshot {
            selector,
            max_chars,
        } => step_dom_snapshot(tab, idx, selector, *max_chars),
        BrowserStep::AccessibilitySnapshot {
            options,
        } => step_accessibility_snapshot(tab, world, idx, options),
        BrowserStep::Screenshot {
            options,
        } => step_screenshot(tab, world, idx, options, image_policy),
        BrowserStep::ScreenshotElement {
            locator,
            options,
        } => step_screenshot_element(tab, world, idx, locator, options, image_policy),
        BrowserStep::ScreenshotElements {
            locators,
            compose,
            labels,
            options,
        } => step_screenshot_elements(
            tab,
            world,
            idx,
            locators,
            *compose,
            *labels,
            options,
            image_policy,
        ),
        BrowserStep::CaptureElementStates {
            locator,
            states,
            labels,
            options,
        } => step_capture_element_states(
            tab,
            world,
            idx,
            locator,
            states,
            *labels,
            options,
            image_policy,
        ),
        BrowserStep::Pdf {
            ..
        } => StepResult::failure(idx, "PDF", "PDF generation requires a browser runtime"),

        BrowserStep::Eval {
            expression,
            timeout_ms,
        } => step_eval(tab, idx, expression, *timeout_ms),
        BrowserStep::Styles {
            locator,
            property_filter,
        } => step_styles(tab, world, idx, locator, property_filter.as_deref()),

        BrowserStep::SetContent {
            ..
        } => StepResult::failure(
            idx,
            "Set content",
            "Setting page content requires a browser runtime",
        ),
        BrowserStep::PageContent => StepResult::failure(
            idx,
            "Page content",
            "Reading page content requires a browser runtime",
        ),
        BrowserStep::AddScriptTag {
            url,
            content,
            script_type,
        } => step_add_script_tag(
            tab,
            idx,
            url.as_deref(),
            content.as_deref(),
            script_type.as_deref(),
        ),
        BrowserStep::AddStyleTag {
            url,
            content,
        } => step_add_style_tag(tab, idx, url.as_deref(), content.as_deref()),
        BrowserStep::AddInitScript {
            ..
        }
        | BrowserStep::RemoveInitScript {
            ..
        } => StepResult::failure(idx, "Init script", "Init scripts require a browser runtime"),
        BrowserStep::DispatchEvent {
            locator,
            event_type,
            event_init,
        } => step_dispatch_event(tab, world, idx, locator, event_type, event_init.as_ref()),

        BrowserStep::TabLog => step_tab_log(idx),
        BrowserStep::ReadArtifact {
            ..
        } => StepResult::failure(
            idx,
            "Read browser artifact",
            "Artifact retrieval requires the current browser runtime",
        ),

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
        BrowserStep::RemoveLocatorHandler {
            name,
        } => step_remove_locator_handler(idx, handlers, name),

        BrowserStep::DismissOverlays {
            aggressive,
        } => step_dismiss_overlays(
            tab,
            world,
            idx,
            handlers,
            locator_handler_firings,
            image_policy,
            *aggressive,
        ),
        BrowserStep::HighlightElement {
            locator,
        } => step_highlight(tab, world, idx, locator, None, None, true),
        BrowserStep::Highlight {
            locator,
            style,
            label,
        } => step_highlight(
            tab,
            world,
            idx,
            locator,
            style.as_deref(),
            label.as_deref(),
            false,
        ),
        BrowserStep::HideHighlight => step_hide_highlight(tab, world, idx),
        BrowserStep::Annotate {
            locator,
            text,
        } => step_highlight(tab, world, idx, locator, None, Some(text), false),
        BrowserStep::Help {
            topic,
        } => StepResult::success(idx, "chrome help").with_data(serde_json::json!({
            "text": crate::tools::tool_chrome::chrome_help_text(topic.as_deref()),
        })),
        BrowserStep::ClockInstall {
            ..
        }
        | BrowserStep::ClockFastForward {
            ..
        }
        | BrowserStep::ClockPauseAt {
            ..
        }
        | BrowserStep::ClockResume
        | BrowserStep::ClockRunFor {
            ..
        }
        | BrowserStep::ClockSetFixedTime {
            ..
        }
        | BrowserStep::ClockSetSystemTime {
            ..
        } => StepResult::failure(
            idx,
            "Browser clock",
            "Clock steps are session-scoped and run only from a browser request batch",
        ),
    }
}

fn expectation_label(matcher: &BrowserExpectation, negate: bool) -> String {
    if negate {
        format!("not {}", matcher.name())
    } else {
        matcher.name().to_string()
    }
}

fn step_expect(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: Option<&BrowserLocator>,
    matcher: &BrowserExpectation,
    timeout_ms: u64,
    soft: bool,
    negate: bool,
) -> StepResult {
    let label = expectation_label(matcher, negate);
    if matcher.requires_locator() && locator.is_none() {
        return StepResult::failure(
            idx,
            format!("Expect {label}"),
            "This matcher requires locator",
        );
    }
    if !matcher.requires_locator() && locator.is_some() {
        return StepResult::failure(
            idx,
            format!("Expect {label}"),
            "Page matchers do not accept locator",
        );
    }
    if let Err(error) = matcher.validate() {
        return StepResult::failure(idx, format!("Expect {label}"), error);
    }
    evaluate_expectation(
        idx,
        matcher,
        Duration::from_millis(clamp_timeout_ms(Some(timeout_ms))),
        soft,
        negate,
        || sample_expectation(tab, world, locator, matcher),
    )
}

fn evaluate_expectation(
    idx: usize,
    matcher: &BrowserExpectation,
    timeout: Duration,
    soft: bool,
    negate: bool,
    mut sample: impl FnMut() -> Result<(bool, Value), String>,
) -> StepResult {
    let label = expectation_label(matcher, negate);
    let expected = expectation_expected_value(matcher);
    let engine = ActionabilityEngine::new(SystemClock::default(), ActionabilityTimeouts::default());
    let result = engine.poll_expect(timeout, || {
        sample().map(|(matched, received)| (matched != negate, received))
    });
    let (passed, received, attempts, elapsed, terminal_error) = match result {
        ExpectPollResult::Matched {
            received,
            attempts,
            elapsed,
        } => (true, received, attempts, elapsed, None),
        ExpectPollResult::TimedOut {
            received,
            attempts,
            elapsed,
        } => (
            false,
            received.unwrap_or(Value::Null),
            attempts,
            elapsed,
            Some(format!("Timeout {}ms exceeded", timeout.as_millis())),
        ),
        ExpectPollResult::Failed {
            error,
            received,
            attempts,
            elapsed,
        } => (
            false,
            received.unwrap_or(Value::Null),
            attempts,
            elapsed,
            Some(error),
        ),
    };
    let diff = match matcher {
        BrowserExpectation::ToMatchAriaSnapshot {
            expected,
        } if !passed && !negate => received
            .as_str()
            .map(|actual| refact_browser::assertions::aria_snapshot_diff(expected, actual)),
        _ => None,
    };
    let assertion = BrowserAssertionResult {
        matcher: label.clone(),
        passed,
        soft,
        expected: expected.clone(),
        received: received.clone(),
        diff,
        attempts,
        elapsed_ms: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
    };
    let mut step = if passed {
        StepResult::success(idx, format!("Assertion passed: {label}"))
    } else {
        let message = format!(
            "Expected {}{} but received {}{}",
            if negate { "not " } else { "" },
            json_for_message(&expected),
            json_for_message(&received),
            terminal_error
                .as_deref()
                .map(|error| format!(" ({error})"))
                .unwrap_or_default()
        );
        let summary = if soft {
            format!("Soft assertion failed: {label}")
        } else {
            format!("Assertion failed: {label}")
        };
        StepResult::failure(idx, summary, message)
    };
    step.retries = expect_retries(attempts);
    step.assertion = Some(assertion);
    step
}

fn expect_retries(attempts: u32) -> u32 {
    attempts.saturating_sub(1)
}

fn elapsed_ms(elapsed: Duration) -> u64 {
    elapsed.as_millis().min(u128::from(u64::MAX)) as u64
}

const POLL_VALUE_BINDING: &str = "__refact_poll_value";

fn page_poll_js(expression: &str) -> String {
    format!(
        "(() => {{ const {POLL_VALUE_BINDING} = ({expression}); return typeof {POLL_VALUE_BINDING} === 'function' ? {POLL_VALUE_BINDING}() : {POLL_VALUE_BINDING}; }})()"
    )
}

fn element_poll_js(expression: &str) -> String {
    format!(
        "function() {{ const {POLL_VALUE_BINDING} = ({expression}); return typeof {POLL_VALUE_BINDING} === 'function' ? {POLL_VALUE_BINDING}(this) : {POLL_VALUE_BINDING}; }}"
    )
}

fn js_exception_message(exception: &Runtime::ExceptionDetails) -> String {
    exception
        .exception
        .as_ref()
        .and_then(|value| value.description.as_deref())
        .unwrap_or(&exception.text)
        .to_string()
}

fn evaluate_page_poll(tab: &Tab, expression: &str) -> Result<Value, String> {
    let evaluated = tab
        .call_method(Runtime::Evaluate {
            expression: page_poll_js(expression),
            object_group: None,
            include_command_line_api: None,
            silent: None,
            context_id: None,
            return_by_value: Some(true),
            generate_preview: None,
            user_gesture: None,
            await_promise: Some(true),
            throw_on_side_effect: None,
            timeout: None,
            disable_breaks: None,
            repl_mode: None,
            allow_unsafe_eval_blocked_by_csp: None,
            unique_context_id: None,
            serialization_options: None,
        })
        .map_err(|error| format!("JS evaluation failed: {error}"))?;
    if let Some(exception) = evaluated.exception_details {
        return Err(js_exception_message(&exception));
    }
    Ok(evaluated.result.value.unwrap_or(Value::Null))
}

fn sample_poll_expression(
    tab: &Tab,
    world: &WorldManager,
    expression: &str,
    locator: Option<&BrowserLocator>,
) -> Result<Value, String> {
    let Some(locator) = locator else {
        return evaluate_page_poll(tab, expression);
    };
    let handles = match resolve_locator_handles(tab, world, locator) {
        Ok(handles) => handles,
        Err(error) if matches!(locator.strategy, LocatorStrategy::Ref { .. }) => return Err(error),
        Err(_) => Vec::new(),
    };
    let Some(handle) = strict_locator_handle(tab, world, locator, handles)? else {
        return Ok(Value::Null);
    };
    let handle = world.wrap_scoped(tab, handle);
    let value = world.call_function_on(tab, &handle, &element_poll_js(expression), Vec::new());
    match value {
        Ok(value) => Ok(value),
        Err(refact_browser::HandleError::Invalidated {
            ..
        }) => Ok(Value::Null),
        Err(error) => Err(error.to_string()),
    }
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn poll_backoff_schedule(polling_ms: Option<u64>) -> Vec<u64> {
    match polling_ms {
        Some(interval) => vec![0, interval],
        None => FUNCTION_POLL_BACKOFF_MS.to_vec(),
    }
}

fn json_equals(left: &Value, right: &Value) -> bool {
    match (left.as_f64(), right.as_f64()) {
        (Some(left), Some(right)) => left == right,
        _ => left == right,
    }
}

fn compare_poll_numbers(received: &Value, expected: &Value) -> Option<Ordering> {
    received.as_f64()?.partial_cmp(&expected.as_f64()?)
}

fn poll_expectation_regex(expected: &Value) -> Result<LocatorRegex, String> {
    match expected {
        Value::String(source) => Ok(LocatorRegex {
            source: source.clone(),
            flags: String::new(),
        }),
        Value::Object(_) => serde_json::from_value(expected.clone())
            .map_err(|error| format!("Invalid matches_regex expectation: {error}")),
        _ => Err("matches_regex expects a regex string or a {source, flags} object".to_string()),
    }
}

fn poll_matcher_matches(
    matcher: BrowserPollMatcher,
    received: &Value,
    expected: &Value,
) -> Result<bool, String> {
    match matcher {
        BrowserPollMatcher::Equals => Ok(json_equals(received, expected)),
        BrowserPollMatcher::Contains => Ok(match (received, expected) {
            (Value::String(received), Value::String(expected)) => received.contains(expected),
            (Value::Array(received), expected) => {
                received.iter().any(|item| json_equals(item, expected))
            }
            _ => false,
        }),
        BrowserPollMatcher::Gt => {
            Ok(compare_poll_numbers(received, expected).is_some_and(Ordering::is_gt))
        }
        BrowserPollMatcher::Lt => {
            Ok(compare_poll_numbers(received, expected).is_some_and(Ordering::is_lt))
        }
        BrowserPollMatcher::MatchesRegex => {
            let regex = poll_expectation_regex(expected)?;
            match received.as_str() {
                Some(received) => refact_browser::assertions::matches_text(
                    received,
                    &BrowserExpectedText::Regex(regex),
                    refact_browser::assertions::TextMatchKind::Exact,
                    false,
                ),
                None => Ok(false),
            }
        }
    }
}

fn redact_poll_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(refact_core::string_utils::redact_sensitive(&text)),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_poll_value).collect()),
        Value::Object(entries) => Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key, redact_poll_value(value)))
                .collect(),
        ),
        other => other,
    }
}

fn step_wait_for_function(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    expression: &str,
    locator: Option<&BrowserLocator>,
    timeout_ms: u64,
    polling_ms: Option<u64>,
) -> StepResult {
    let engine = ActionabilityEngine::new(SystemClock::default(), ActionabilityTimeouts::default());
    let schedule = poll_backoff_schedule(polling_ms);
    let outcome =
        engine.poll_expect_with_backoff(Duration::from_millis(timeout_ms), &schedule, || {
            sample_poll_expression(tab, world, expression, locator)
                .map(|value| (is_truthy(&value), value))
        });
    let target = locator
        .map(|locator| format!(" ({})", describe_locator(locator)))
        .unwrap_or_default();
    let (received, attempts, elapsed, error) = match outcome {
        ExpectPollResult::Matched {
            received,
            attempts,
            elapsed,
        } => {
            let mut step = StepResult::success(idx, format!("Predicate satisfied{target}"))
                .with_data(serde_json::json!({
                    "value": redact_poll_value(received),
                    "attempts": attempts,
                    "elapsed_ms": elapsed_ms(elapsed),
                }));
            step.retries = expect_retries(attempts);
            return step;
        }
        ExpectPollResult::TimedOut {
            received,
            attempts,
            elapsed,
        } => (
            received.unwrap_or(Value::Null),
            attempts,
            elapsed,
            format!("Timeout {timeout_ms}ms exceeded"),
        ),
        ExpectPollResult::Failed {
            error,
            received,
            attempts,
            elapsed,
        } => (received.unwrap_or(Value::Null), attempts, elapsed, error),
    };
    let received = redact_poll_value(received);
    let mut step = StepResult::failure(
        idx,
        format!("Wait for function{target}"),
        format!("{error}; last value was {}", json_for_message(&received)),
    );
    step.retries = expect_retries(attempts);
    step.data = Some(serde_json::json!({
        "value": received,
        "attempts": attempts,
        "elapsed_ms": elapsed_ms(elapsed),
    }));
    step
}

fn step_expect_poll(
    tab: &Tab,
    idx: usize,
    expression: &str,
    expected: &Value,
    matcher: BrowserPollMatcher,
    timeout_ms: u64,
    soft: bool,
) -> StepResult {
    let engine = ActionabilityEngine::new(SystemClock::default(), ActionabilityTimeouts::default());
    let outcome = engine.poll_expect_with_backoff(
        Duration::from_millis(timeout_ms),
        FUNCTION_POLL_BACKOFF_MS,
        || {
            let received = evaluate_page_poll(tab, expression)?;
            let matched = poll_matcher_matches(matcher, &received, expected)?;
            Ok((matched, received))
        },
    );
    let (passed, received, attempts, elapsed, error) = match outcome {
        ExpectPollResult::Matched {
            received,
            attempts,
            elapsed,
        } => (true, received, attempts, elapsed, None),
        ExpectPollResult::TimedOut {
            received,
            attempts,
            elapsed,
        } => (
            false,
            received.unwrap_or(Value::Null),
            attempts,
            elapsed,
            Some(format!("Timeout {timeout_ms}ms exceeded")),
        ),
        ExpectPollResult::Failed {
            error,
            received,
            attempts,
            elapsed,
        } => (
            false,
            received.unwrap_or(Value::Null),
            attempts,
            elapsed,
            Some(error),
        ),
    };
    let received = redact_poll_value(received);
    let assertion = BrowserAssertionResult {
        matcher: matcher.name().to_string(),
        passed,
        soft,
        expected: expected.clone(),
        received: received.clone(),
        diff: None,
        attempts,
        elapsed_ms: elapsed_ms(elapsed),
    };
    let mut step = if passed {
        StepResult::success(idx, format!("Poll assertion passed: {}", matcher.name()))
    } else {
        let summary = if soft {
            format!("Soft poll assertion failed: {}", matcher.name())
        } else {
            format!("Poll assertion failed: {}", matcher.name())
        };
        StepResult::failure(
            idx,
            summary,
            format!(
                "Expected {} {} but received {}{}",
                matcher.name(),
                json_for_message(expected),
                json_for_message(&received),
                error
                    .as_deref()
                    .map(|error| format!(" ({error})"))
                    .unwrap_or_default()
            ),
        )
    };
    step.retries = expect_retries(attempts);
    step.assertion = Some(assertion);
    step
}

fn expectation_expected_value(matcher: &BrowserExpectation) -> Value {
    match matcher {
        BrowserExpectation::ToBeAttached
        | BrowserExpectation::ToBeVisible
        | BrowserExpectation::ToBeHidden
        | BrowserExpectation::ToBeEnabled
        | BrowserExpectation::ToBeDisabled
        | BrowserExpectation::ToBeEditable
        | BrowserExpectation::ToBeFocused
        | BrowserExpectation::ToBeEmpty => Value::Bool(true),
        BrowserExpectation::ToBeChecked {
            indeterminate: Some(true),
            ..
        } => Value::String("indeterminate".to_string()),
        BrowserExpectation::ToBeChecked {
            checked, ..
        } => Value::Bool(checked.unwrap_or(true)),
        BrowserExpectation::ToBeInViewport {
            ratio,
        } => ratio
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Bool(true)),
        BrowserExpectation::ToHaveAttribute {
            expected: None, ..
        } => Value::String("<present>".to_string()),
        BrowserExpectation::ToHaveText {
            expected, ..
        }
        | BrowserExpectation::ToContainText {
            expected, ..
        } => serde_json::to_value(expected).unwrap_or(Value::Null),
        BrowserExpectation::ToHaveValue {
            expected, ..
        }
        | BrowserExpectation::ToHaveClass {
            expected, ..
        }
        | BrowserExpectation::ToHaveId {
            expected, ..
        }
        | BrowserExpectation::ToHaveAccessibleName {
            expected, ..
        }
        | BrowserExpectation::ToHaveAccessibleDescription {
            expected, ..
        }
        | BrowserExpectation::ToHaveUrl {
            expected, ..
        }
        | BrowserExpectation::ToHaveTitle {
            expected, ..
        }
        | BrowserExpectation::ToHaveAttribute {
            expected: Some(expected),
            ..
        }
        | BrowserExpectation::ToHaveCss {
            expected, ..
        } => serde_json::to_value(expected).unwrap_or(Value::Null),
        BrowserExpectation::ToHaveValues {
            expected, ..
        } => serde_json::to_value(expected).unwrap_or(Value::Null),
        BrowserExpectation::ToContainClass {
            expected, ..
        }
        | BrowserExpectation::ToHaveRole {
            expected,
        } => Value::String(expected.clone()),
        BrowserExpectation::ToHaveCount {
            expected,
        } => Value::from(*expected),
        BrowserExpectation::ToHaveJsProperty {
            expected, ..
        } => expected.clone(),
        BrowserExpectation::ToMatchAriaSnapshot {
            expected,
        } => Value::String(expected.clone()),
    }
}

fn sample_expectation(
    tab: &Tab,
    world: &WorldManager,
    locator: Option<&BrowserLocator>,
    matcher: &BrowserExpectation,
) -> Result<(bool, Value), String> {
    match matcher {
        BrowserExpectation::ToHaveUrl {
            expected,
            ignore_case,
        } => {
            let received = tab.get_url();
            let matches = refact_browser::assertions::matches_text(
                &received,
                expected,
                refact_browser::assertions::TextMatchKind::Exact,
                *ignore_case,
            )?;
            return Ok((matches, Value::String(received)));
        }
        BrowserExpectation::ToHaveTitle {
            expected,
            ignore_case,
        } => {
            let received = tab
                .evaluate("document.title", false)
                .map_err(|error| error.to_string())?
                .value
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default();
            let matches = refact_browser::assertions::matches_text(
                &received,
                expected,
                refact_browser::assertions::TextMatchKind::Exact,
                *ignore_case,
            )?;
            return Ok((matches, Value::String(received)));
        }
        _ => {}
    }

    let locator = locator.unwrap();
    let handles = match resolve_locator_handles(tab, world, locator) {
        Ok(handles) => handles,
        Err(error) if matches!(locator.strategy, LocatorStrategy::Ref { .. }) => return Err(error),
        Err(_) => Vec::new(),
    };
    let handles = world.wrap_scoped_all(tab, handles);
    if let BrowserExpectation::ToHaveCount {
        expected,
    } = matcher
    {
        let count = handles.len();
        return Ok((count == *expected, Value::from(count)));
    }
    if let BrowserExpectation::ToBeHidden = matcher {
        if handles.is_empty() {
            return Ok((true, Value::String("detached".to_string())));
        }
    }
    if handles.len() > 1 && !matcher.is_multi_element() {
        let previews = strict_locator_previews(tab, world, handles.iter().map(|handle| &**handle));
        return Err(strict_expectation_error(matcher, locator, handles.len(), &previews).unwrap());
    }
    if let BrowserExpectation::ToHaveValues {
        expected,
        ignore_case,
    } = matcher
    {
        let mut received = Vec::new();
        for handle in &handles {
            let values = world
                .expectation_values(tab, handle)
                .map_err(|error| error.to_string())?;
            let value = values
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            received.push(value);
        }
        let matches = received.len() == expected.len()
            && received.iter().zip(expected).all(|(received, expected)| {
                refact_browser::assertions::matches_text(
                    received,
                    expected,
                    refact_browser::assertions::TextMatchKind::Exact,
                    *ignore_case,
                )
                .unwrap_or(false)
            });
        return Ok((matches, serde_json::to_value(received).unwrap_or_default()));
    }
    if let Some((expected, ignore_case, kind)) = expectation_text_list(matcher) {
        let mut received = Vec::new();
        for handle in &handles {
            let values = world
                .expectation_values(tab, handle)
                .map_err(|error| error.to_string())?;
            received.push(
                values
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            );
        }
        let matches =
            refact_browser::assertions::matches_text_list(&received, expected, kind, ignore_case)?;
        return Ok((matches, serde_json::to_value(received).unwrap_or_default()));
    }
    let Some(handle) = handles.first() else {
        return Ok((false, Value::String("detached".to_string())));
    };
    sample_single_element(tab, world, handle, matcher)
}

fn expectation_text_list(
    matcher: &BrowserExpectation,
) -> Option<(
    &[BrowserExpectedText],
    bool,
    refact_browser::assertions::TextMatchKind,
)> {
    match matcher {
        BrowserExpectation::ToHaveText {
            expected: BrowserExpectedTextOrList::Many(expected),
            ignore_case,
        } => Some((
            expected,
            *ignore_case,
            refact_browser::assertions::TextMatchKind::Exact,
        )),
        BrowserExpectation::ToContainText {
            expected: BrowserExpectedTextOrList::Many(expected),
            ignore_case,
        } => Some((
            expected,
            *ignore_case,
            refact_browser::assertions::TextMatchKind::Contains,
        )),
        _ => None,
    }
}

fn strict_expectation_error(
    matcher: &BrowserExpectation,
    locator: &BrowserLocator,
    count: usize,
    previews: &[String],
) -> Option<String> {
    (count > 1 && !matcher.is_multi_element())
        .then(|| refact_browser::strict_mode_violation(&describe_locator(locator), count, previews))
}

fn sample_single_element(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    matcher: &BrowserExpectation,
) -> Result<(bool, Value), String> {
    if let BrowserExpectation::ToMatchAriaSnapshot {
        expected,
    } = matcher
    {
        let snapshot = world
            .aria_snapshot(tab, Some(handle.clone()), SnapshotOptions::default())
            .map_err(|error| error.to_string())?;
        let matches = refact_browser::assertions::match_aria_snapshot(expected, &snapshot.yaml)?;
        return Ok((matches, Value::String(snapshot.yaml)));
    }
    if let BrowserExpectation::ToHaveAttribute {
        name, ..
    }
    | BrowserExpectation::ToHaveCss {
        name, ..
    }
    | BrowserExpectation::ToHaveJsProperty {
        name, ..
    } = matcher
    {
        let (function, arguments) = match matcher {
            BrowserExpectation::ToHaveAttribute {
                ..
            } => (
                "function(name) { return this.getAttribute(name); }",
                vec![Value::String(name.clone())],
            ),
            BrowserExpectation::ToHaveCss {
                pseudo, ..
            } => (
                "function(name, pseudo) { return getComputedStyle(this, pseudo).getPropertyValue(name); }",
                vec![
                    Value::String(name.clone()),
                    pseudo
                        .map(|pseudo| Value::String(pseudo.selector().to_string()))
                        .unwrap_or(Value::Null),
                ],
            ),
            BrowserExpectation::ToHaveJsProperty {
                ..
            } => (
                "function(name) { return this[name]; }",
                vec![Value::String(name.clone())],
            ),
            _ => unreachable!(),
        };
        let received = world
            .call_function_on(tab, handle, function, arguments)
            .map_err(|error| error.to_string())?;
        let matches = match matcher {
            BrowserExpectation::ToHaveAttribute {
                expected: None, ..
            } => !received.is_null(),
            BrowserExpectation::ToHaveAttribute {
                expected: Some(expected),
                ignore_case,
                ..
            }
            | BrowserExpectation::ToHaveCss {
                expected,
                ignore_case,
                ..
            } => received.as_str().is_some_and(|received| {
                refact_browser::assertions::matches_text(
                    received,
                    expected,
                    refact_browser::assertions::TextMatchKind::Exact,
                    *ignore_case,
                )
                .unwrap_or(false)
            }),
            BrowserExpectation::ToHaveJsProperty {
                expected, ..
            } => refact_browser::assertions::matches_json_property(&received, expected),
            _ => false,
        };
        return Ok((matches, received));
    }
    let values = world
        .expectation_values(tab, handle)
        .map_err(|error| error.to_string())?;
    let field = |name: &str| values.get(name).cloned().unwrap_or(Value::Null);
    let boolean = |name: &str, expected: bool| {
        let received = field(name);
        (received == Value::Bool(expected), received)
    };
    let text = |name: &str,
                expected: &BrowserExpectedText,
                kind: refact_browser::assertions::TextMatchKind,
                ignore_case: bool|
     -> Result<(bool, Value), String> {
        let received = field(name);
        let value = received.as_str().unwrap_or_default();
        Ok((
            refact_browser::assertions::matches_text(value, expected, kind, ignore_case)?,
            received,
        ))
    };
    match matcher {
        BrowserExpectation::ToBeAttached => Ok(boolean("attached", true)),
        BrowserExpectation::ToBeVisible => Ok(boolean("visible", true)),
        BrowserExpectation::ToBeHidden => Ok(boolean("visible", false)),
        BrowserExpectation::ToBeEnabled => Ok(boolean("enabled", true)),
        BrowserExpectation::ToBeDisabled => Ok(boolean("enabled", false)),
        BrowserExpectation::ToBeEditable => Ok(boolean("editable", true)),
        BrowserExpectation::ToBeChecked {
            indeterminate: Some(true),
            ..
        } => Ok(boolean("indeterminate", true)),
        BrowserExpectation::ToBeChecked {
            checked, ..
        } => Ok(boolean("checked", checked.unwrap_or(true))),
        BrowserExpectation::ToBeFocused => Ok(boolean("focused", true)),
        BrowserExpectation::ToBeEmpty => Ok(boolean("empty", true)),
        BrowserExpectation::ToBeInViewport {
            ratio: None,
        } => Ok(boolean("inViewport", true)),
        BrowserExpectation::ToBeInViewport {
            ratio: Some(ratio),
        } => {
            let received = field("viewportRatio");
            Ok((
                viewport_ratio_matches(received.as_f64().unwrap_or_default(), *ratio),
                received,
            ))
        }
        BrowserExpectation::ToHaveText {
            expected: BrowserExpectedTextOrList::One(expected),
            ignore_case,
        } => text(
            "text",
            expected,
            refact_browser::assertions::TextMatchKind::Exact,
            *ignore_case,
        ),
        BrowserExpectation::ToContainText {
            expected: BrowserExpectedTextOrList::One(expected),
            ignore_case,
        } => text(
            "text",
            expected,
            refact_browser::assertions::TextMatchKind::Contains,
            *ignore_case,
        ),
        BrowserExpectation::ToHaveValue {
            expected,
            ignore_case,
        } => text(
            "value",
            expected,
            refact_browser::assertions::TextMatchKind::Exact,
            *ignore_case,
        ),
        BrowserExpectation::ToHaveClass {
            expected,
            ignore_case,
        } => text(
            "class",
            expected,
            refact_browser::assertions::TextMatchKind::Exact,
            *ignore_case,
        ),
        BrowserExpectation::ToContainClass {
            expected,
            ignore_case,
        } => {
            let received = field("class");
            Ok((
                received.as_str().is_some_and(|received| {
                    refact_browser::assertions::matches_class_list(received, expected, *ignore_case)
                }),
                received,
            ))
        }
        BrowserExpectation::ToHaveId {
            expected,
            ignore_case,
        } => text(
            "id",
            expected,
            refact_browser::assertions::TextMatchKind::Exact,
            *ignore_case,
        ),
        BrowserExpectation::ToHaveRole {
            expected,
        } => Ok((
            field("role").as_str() == Some(expected.as_str()),
            field("role"),
        )),
        BrowserExpectation::ToHaveAccessibleName {
            expected,
            ignore_case,
        } => text(
            "accessibleName",
            expected,
            refact_browser::assertions::TextMatchKind::Exact,
            *ignore_case,
        ),
        BrowserExpectation::ToHaveAccessibleDescription {
            expected,
            ignore_case,
        } => text(
            "accessibleDescription",
            expected,
            refact_browser::assertions::TextMatchKind::Exact,
            *ignore_case,
        ),
        _ => Err(format!("Unsupported element matcher: {}", matcher.name())),
    }
}

const VIEWPORT_RATIO_TOLERANCE: f64 = 1e-9;

fn viewport_ratio_matches(received: f64, ratio: f64) -> bool {
    if ratio > 0.0 {
        received + VIEWPORT_RATIO_TOLERANCE >= ratio
    } else {
        received > 0.0
    }
}

fn json_for_message(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn uses_actionability_engine(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::Click { .. }
            | BrowserStep::ClickIfExists { .. }
            | BrowserStep::Tap {
                locator: Some(_),
                ..
            }
            | BrowserStep::Hover { .. }
            | BrowserStep::Focus { .. }
            | BrowserStep::InsertText {
                locator: Some(_),
                ..
            }
            | BrowserStep::PressSequentially { .. }
            | BrowserStep::ScrollTo { .. }
            | BrowserStep::DragAndDrop { .. }
    )
}

fn element_drag_point(
    tab: &Tab,
    handle: &ElementHandle,
    position: Option<BrowserPosition>,
) -> Result<MainFrameCssPoint, refact_browser::MouseError> {
    if let Some(position) = position {
        let quads = tab
            .call_method(DOM::GetContentQuads {
                node_id: None,
                backend_node_id: None,
                object_id: Some(handle.object_id.clone()),
            })
            .map_err(|error| {
                refact_browser::MouseError::Protocol(format!(
                    "Failed to read browser element content quads: {error}"
                ))
            })?;
        let Some(quad) = quads.quads.first() else {
            return Err(refact_browser::MouseError::NoQuads);
        };
        if quad.len() != 8 {
            return Err(refact_browser::MouseError::Protocol(
                "Browser content quad must contain 8 coordinates".to_string(),
            ));
        }
        return Ok(MainFrameCssPoint {
            x: quad[0] + position.x,
            y: quad[1] + position.y,
        });
    }
    CdpMouseDispatcher::new(tab).clickable_point(handle)
}

fn resolve_drag_endpoint(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
    position: Option<BrowserPosition>,
    action: ActionKind,
    mode: ActionabilityExecutionMode,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
) -> Result<
    refact_browser::ActionabilitySuccess<(String, ElementHandle, MainFrameCssPoint)>,
    refact_browser::ActionabilityError,
> {
    let settings = browser_settings::current();
    let engine = ActionabilityEngine::new(SystemClock::default(), ActionabilityTimeouts::default());
    let timeout = ActionabilityTimeouts::default().action;
    let mut driver = DragActionabilityDriver {
        tab,
        world,
        locator,
        handlers,
        locator_handler_firings: firings,
        image_policy,
        precheck_deadline: Instant::now()
            + timeout.min(Duration::from_millis(
                settings.timing.default_wait_timeout_ms,
            )),
        resolved: None,
        position,
    };
    engine.execute_with_timeout_in_mode(
        &describe_locator(locator),
        action,
        timeout,
        mode,
        &mut driver,
    )
}

fn step_drag_and_drop(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    source: &BrowserLocator,
    target: &BrowserLocator,
    source_position: Option<BrowserPosition>,
    target_position: Option<BrowserPosition>,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
    mouse_state: &mut MouseState,
) -> StepResult {
    let source = match resolve_drag_endpoint(
        tab,
        world,
        source,
        source_position,
        ActionKind::DragSource,
        ActionabilityExecutionMode::Standard,
        handlers,
        firings,
        image_policy,
    ) {
        Ok(source) => source,
        Err(error) => {
            let mut result = StepResult::failure(
                idx,
                "Drag source failed",
                actionability_error_text(&error, source),
            );
            result.actionability = Some(error.diagnostics(ActionKind::DragSource));
            return result;
        }
    };
    let target = match resolve_drag_endpoint(
        tab,
        world,
        target,
        target_position,
        ActionKind::DragTarget,
        ActionabilityExecutionMode::SkipLocatorHandlers,
        handlers,
        firings,
        image_policy,
    ) {
        Ok(target) => target,
        Err(error) => {
            let mut result = StepResult::failure(
                idx,
                "Drag target failed",
                actionability_error_text(&error, target),
            );
            result.actionability = Some(error.diagnostics(ActionKind::DragTarget));
            return result;
        }
    };
    let keyboard = Keyboard::new(CdpKeyboardDispatcher::new(tab));
    let dispatcher = CdpMouseDispatcher::new(tab);
    let mut mouse = Mouse::from_state(dispatcher, &keyboard, mouse_state.clone());
    let mut drag = CdpDragObserver::new(tab, world);
    let drag_result = refact_browser::drag_and_drop(
        &mut mouse,
        &mut drag,
        &source.output.1.frame_id,
        source.output.2,
        target.output.2,
        2,
    );
    *mouse_state = mouse.state();
    let _ = world.release_handle(tab, &source.output.1);
    let _ = world.release_handle(tab, &target.output.1);
    match drag_result {
        Ok(()) => {
            let mut result = StepResult::success(
                idx,
                format!("Dragged <{}> to <{}>", source.output.0, target.output.0),
            );
            result.retries = source.attempts + target.attempts;
            result
        }
        Err(error) => StepResult::failure(idx, "Drag and drop failed", error.to_string()),
    }
}

fn step_drop_files(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    target: &BrowserLocator,
    paths: &[String],
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
) -> StepResult {
    let resolved = match resolve_drag_endpoint(
        tab,
        world,
        target,
        None,
        ActionKind::DragTarget,
        ActionabilityExecutionMode::Standard,
        handlers,
        firings,
        image_policy,
    ) {
        Ok(resolved) => resolved,
        Err(error) => {
            let mut result = StepResult::failure(
                idx,
                "File drop target failed",
                actionability_error_text(&error, target),
            );
            result.actionability = Some(error.diagnostics(ActionKind::DragTarget));
            return result;
        }
    };
    let outcome = refact_browser::drop_files(tab, world, &resolved.output.1, paths);
    let _ = world.release_handle(tab, &resolved.output.1);
    let result = match outcome {
        Ok(result) => result,
        Err(error) => return StepResult::failure(idx, "File drop failed", error),
    };
    let data = serde_json::to_value(&result).unwrap_or_default();
    match refact_browser::verify_file_drop(&result, paths) {
        Ok(()) => {
            StepResult::success(idx, format!("Dropped {} file(s)", paths.len())).with_data(data)
        }
        Err(error) => {
            let mut failure = StepResult::failure(idx, "File drop rejected", error);
            failure.data = Some(data);
            failure
        }
    }
}

fn browser_mouse_button(button: BrowserMouseButton) -> MouseButton {
    match button {
        BrowserMouseButton::Left => MouseButton::Left,
        BrowserMouseButton::Middle => MouseButton::Middle,
        BrowserMouseButton::Right => MouseButton::Right,
    }
}

fn step_mouse(
    tab: &Tab,
    idx: usize,
    state: &mut MouseState,
    operation: impl FnOnce(
        &mut Mouse<'_, CdpMouseDispatcher<'_>, CdpKeyboardDispatcher<'_>>,
    ) -> Result<(), refact_browser::MouseError>,
    summary: String,
) -> StepResult {
    let keyboard = Keyboard::new(CdpKeyboardDispatcher::new(tab));
    let mut mouse = Mouse::from_state(CdpMouseDispatcher::new(tab), &keyboard, state.clone());
    match operation(&mut mouse) {
        Ok(()) => {
            *state = mouse.state();
            StepResult::success(idx, summary)
        }
        Err(error) => StepResult::failure(idx, "Coordinate mouse action failed", error.to_string()),
    }
}

fn step_set_input_files(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    paths: &[String],
) -> StepResult {
    let resolved = match resolve_scoped_element(tab, world, locator) {
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

fn step_navigate(tab: &Tab, idx: usize, url: &str, timeout_ms: u64) -> StepResult {
    match run_and_wait_for_navigation(tab, timeout_ms, || trigger_page_navigation(tab, url)) {
        Ok(warning) => navigation_step_success(idx, format!("Navigated to {}", url), warning),
        Err(e) => StepResult::failure(idx, format!("Navigate to {}", url), e.to_string()),
    }
}

fn step_nav_js(tab: &Tab, idx: usize, js: &str, success_msg: &str) -> StepResult {
    let settings = browser_settings::current();
    match run_and_wait_for_navigation(tab, settings.timing.default_wait_timeout_ms, || {
        let target = js_navigation_target(tab, js)?;
        tab.evaluate(js, false)
            .map_err(|error| format!("JS navigation trigger failed: {error}"))?;
        Ok(target)
    }) {
        Ok(warning) => navigation_step_success(idx, success_msg.to_string(), warning),
        Err(error) => StepResult::failure(idx, success_msg.to_string(), error),
    }
}

fn merge_step_data(mut result: StepResult, extra: serde_json::Value) -> StepResult {
    match (result.data.as_mut(), extra) {
        (Some(Value::Object(existing)), Value::Object(extra)) => existing.extend(extra),
        (_, extra) => result.data = Some(extra),
    }
    result
}

fn navigation_step_success(
    idx: usize,
    summary: String,
    warning: Option<NavigationTimeoutWarning>,
) -> StepResult {
    match warning {
        Some(warning) => StepResult::success(idx, format!("{summary} ({warning})")).with_data(
            serde_json::json!({
                "load_event": "not_observed",
                "ready_state": warning.ready_state,
                "waited_ms": warning.timeout_ms,
            }),
        ),
        None => StepResult::success(idx, summary),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum NavigationWaitTarget {
    Loader {
        frame_id: Page::FrameId,
        loader_id: Option<String>,
    },
    Triggered {
        frame_id: Page::FrameId,
        previous_loader_id: String,
        expected_url: Option<String>,
        allow_same_document: bool,
    },
    NoNavigation,
}

#[derive(Debug, Clone, PartialEq)]
enum NavigationWaitOutcome {
    Completed,
    TimedOut {
        committed: bool,
        expected_url: Option<String>,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum NavigationEvent {
    Load {
        frame_id: Page::FrameId,
        loader_id: String,
    },
    FrameNavigated {
        frame_id: Page::FrameId,
        loader_id: String,
        url: String,
        restored_from_back_forward_cache: bool,
    },
    SameDocument {
        frame_id: Page::FrameId,
        url: String,
    },
}

fn trigger_page_navigation(tab: &Tab, url: &str) -> Result<NavigationWaitTarget, String> {
    let response = tab
        .call_method(Page::Navigate {
            url: url.to_string(),
            referrer: None,
            transition_Type: None,
            frame_id: None,
            referrer_policy: None,
        })
        .map_err(|error| format!("CDP Page.navigate failed: {error}"))?;
    if let Some(error) = response.error_text {
        return Err(format!("CDP Page.navigate failed: {error}"));
    }
    Ok(NavigationWaitTarget::Loader {
        frame_id: response.frame_id,
        loader_id: response.loader_id,
    })
}

fn js_navigation_target(tab: &Tab, js: &str) -> Result<NavigationWaitTarget, String> {
    let frame = tab
        .call_method(Page::GetFrameTree(None))
        .map_err(|error| format!("Failed to read the main frame before navigation: {error}"))?
        .frame_tree
        .frame;
    let expected_url = match js {
        "history.back()" => history_target_url(tab, -1)?,
        "history.forward()" => history_target_url(tab, 1)?,
        _ => None,
    };
    if matches!(js, "history.back()" | "history.forward()") && expected_url.is_none() {
        return Ok(NavigationWaitTarget::NoNavigation);
    }
    Ok(NavigationWaitTarget::Triggered {
        frame_id: frame.id,
        previous_loader_id: frame.loader_id,
        expected_url,
        allow_same_document: js != "location.reload()",
    })
}

fn history_target_url(tab: &Tab, offset: i64) -> Result<Option<String>, String> {
    let history = tab
        .call_method(Page::GetNavigationHistory(None))
        .map_err(|error| format!("Failed to read browser navigation history: {error}"))?;
    let target_index = i64::from(history.current_index) + offset;
    if target_index < 0 {
        return Ok(None);
    }
    Ok(history
        .entries
        .get(target_index as usize)
        .map(|entry| entry.url.clone()))
}

fn page_frame_url(frame: &Page::Frame) -> String {
    format!(
        "{}{}",
        frame.url,
        frame.url_fragment.as_deref().unwrap_or_default()
    )
}

fn document_ready_state(tab: &Tab) -> String {
    eval_js_value(tab, "document.readyState")
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NavigationTimeoutWarning {
    ready_state: String,
    timeout_ms: u64,
}

impl std::fmt::Display for NavigationTimeoutWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{NAVIGATION_LIFECYCLE_EVENT} event not observed within {}ms; document.readyState={} — continuing",
            self.timeout_ms, self.ready_state
        )
    }
}

fn classify_navigation_timeout(
    ready_state: &str,
    committed: bool,
    timeout_ms: u64,
) -> Option<NavigationTimeoutWarning> {
    let document_readable = matches!(ready_state, "loading" | "interactive" | "complete");
    if !document_readable {
        return None;
    }
    if !matches!(ready_state, "interactive" | "complete") && !committed {
        return None;
    }
    Some(NavigationTimeoutWarning {
        ready_state: ready_state.to_string(),
        timeout_ms,
    })
}

fn run_and_wait_for_navigation(
    tab: &Tab,
    timeout_ms: u64,
    trigger: impl FnOnce() -> Result<NavigationWaitTarget, String>,
) -> Result<Option<NavigationTimeoutWarning>, String> {
    let (sender, receiver) = mpsc::channel();
    let listener = tab
        .add_event_listener(Arc::new(move |event: &Event| match event {
            Event::PageLifecycleEvent(event) if event.params.name == NAVIGATION_LIFECYCLE_EVENT => {
                let _ = sender.send(NavigationEvent::Load {
                    frame_id: event.params.frame_id.clone(),
                    loader_id: event.params.loader_id.clone(),
                });
            }
            Event::PageFrameNavigated(event) => {
                let frame = &event.params.frame;
                let _ = sender.send(NavigationEvent::FrameNavigated {
                    frame_id: frame.id.clone(),
                    loader_id: frame.loader_id.clone(),
                    url: page_frame_url(frame),
                    restored_from_back_forward_cache: matches!(
                        &event.params.Type,
                        Page::NavigationType::BackForwardCacheRestore
                    ),
                });
            }
            Event::PageNavigatedWithinDocument(event) => {
                let _ = sender.send(NavigationEvent::SameDocument {
                    frame_id: event.params.frame_id.clone(),
                    url: event.params.url.clone(),
                });
            }
            _ => {}
        }))
        .map_err(|error| format!("Failed to listen for CDP navigation events: {error}"))?;
    let result = trigger().and_then(|target| {
        wait_for_navigation_event(&receiver, &target, Duration::from_millis(timeout_ms))
    });
    let removal = tab
        .remove_event_listener(&listener)
        .map_err(|error| format!("Failed to remove CDP navigation listener: {error}"));
    let outcome = result?;
    removal?;
    match outcome {
        NavigationWaitOutcome::Completed => Ok(None),
        NavigationWaitOutcome::TimedOut {
            committed,
            expected_url,
            message,
        } => {
            let committed = committed || expected_url.is_some_and(|url| url == tab.get_url());
            let ready_state = document_ready_state(tab);
            match classify_navigation_timeout(&ready_state, committed, timeout_ms) {
                Some(warning) => Ok(Some(warning)),
                None if !matches!(ready_state.as_str(), "loading" | "interactive" | "complete") => {
                    Err(format!(
                        "{message}; document.readyState could not be read ({ready_state}), so the page is not evaluable"
                    ))
                }
                None => Err(message),
            }
        }
    }
}

fn wait_for_navigation_event(
    receiver: &Receiver<NavigationEvent>,
    target: &NavigationWaitTarget,
    timeout: Duration,
) -> Result<NavigationWaitOutcome, String> {
    let (frame_id, mut pending_loader_id, previous_loader_id, expected_url, allow_same_document) =
        match target {
            NavigationWaitTarget::NoNavigation => return Ok(NavigationWaitOutcome::Completed),
            NavigationWaitTarget::Loader {
                frame_id: _,
                loader_id: None,
            } => return Ok(NavigationWaitOutcome::Completed),
            NavigationWaitTarget::Loader {
                frame_id,
                loader_id: Some(loader_id),
            } => (frame_id, Some(loader_id.clone()), None, None, false),
            NavigationWaitTarget::Triggered {
                frame_id,
                previous_loader_id,
                expected_url,
                allow_same_document,
            } => (
                frame_id,
                None,
                Some(previous_loader_id),
                expected_url.as_ref(),
                *allow_same_document,
            ),
        };
    let deadline = Instant::now() + timeout;
    let mut loaded = std::collections::HashSet::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok(NavigationEvent::Load {
                frame_id: completed_frame,
                loader_id,
            }) if completed_frame == *frame_id => {
                if pending_loader_id.as_ref() == Some(&loader_id) {
                    return Ok(NavigationWaitOutcome::Completed);
                }
                loaded.insert(loader_id);
            }
            Ok(NavigationEvent::FrameNavigated {
                frame_id: completed_frame,
                loader_id,
                url,
                restored_from_back_forward_cache,
            }) if completed_frame == *frame_id
                && previous_loader_id.is_some_and(|previous| previous != &loader_id)
                && expected_url.is_none_or(|expected| expected == &url) =>
            {
                if restored_from_back_forward_cache || loaded.contains(&loader_id) {
                    return Ok(NavigationWaitOutcome::Completed);
                }
                pending_loader_id = Some(loader_id);
            }
            Ok(NavigationEvent::SameDocument {
                frame_id: completed_frame,
                url,
            }) if completed_frame == *frame_id
                && previous_loader_id.is_some()
                && allow_same_document
                && expected_url.is_none_or(|expected| expected == &url) =>
            {
                return Ok(NavigationWaitOutcome::Completed);
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                let missing = pending_loader_id.as_ref().map_or_else(
                    || {
                        format!(
                            "CDP Page.frameNavigated or Page.navigatedWithinDocument for frame {frame_id}"
                        )
                    },
                    |loader_id| {
                        format!(
                            "CDP Page.lifecycleEvent({NAVIGATION_LIFECYCLE_EVENT}) for frame {frame_id} and loader {loader_id}"
                        )
                    },
                );
                return Ok(NavigationWaitOutcome::TimedOut {
                    committed: pending_loader_id.is_some(),
                    expected_url: expected_url.cloned(),
                    message: format!(
                        "Timed out after {}ms waiting for {missing}",
                        timeout.as_millis()
                    ),
                });
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(format!(
                    "CDP navigation listener disconnected for frame {frame_id}"
                ));
            }
        }
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
    step_locator_action_with_timeout(
        tab,
        world,
        idx,
        locator,
        action,
        handlers,
        locator_handler_firings,
        image_policy,
        None,
    )
}

fn step_locator_action_with_timeout(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    action: &str,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
    timeout_ms: Option<u64>,
) -> StepResult {
    if let Some(action_kind) = match action {
        "click" => Some(ActionKind::Click),
        "hover" => Some(ActionKind::Hover),
        "tap" => Some(ActionKind::Tap),
        "focus" => Some(ActionKind::Focus),
        "scroll_to" => Some(ActionKind::ScrollIntoViewIfNeeded),
        _ => None,
    } {
        let timeout = timeout_ms
            .map(|ms| Duration::from_millis(clamp_timeout_ms(Some(ms))))
            .unwrap_or_else(|| ActionabilityTimeouts::default().action);
        return step_actionable_action_in_mode(
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
            timeout,
        );
    }
    match resolve_interactable(tab, world, locator) {
        Ok(info) => {
            let action_js = match action {
                "focus" => browser_locators::js_focus_element().to_string(),
                "blur" => browser_locators::js_blur_element().to_string(),
                "scroll_to" => browser_locators::js_scroll_to_element().to_string(),
                _ => {
                    return StepResult::failure(idx, action, format!("Unknown action: {}", action));
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

fn actionability_error_text(
    error: &refact_browser::ActionabilityError,
    locator: &BrowserLocator,
) -> String {
    format!("{}: {}", describe_locator(locator), error.headline())
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
        timeout,
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
            result.locator_echo = driver.locator_echo.take();
            result
        }
        Err(error) => {
            let diagnostics = error.diagnostics(action_kind);
            let mut result = StepResult::failure(
                idx,
                format!("{action} failed"),
                actionability_error_text(&error, locator),
            );
            if matches!(
                &error,
                refact_browser::ActionabilityError::Failed {
                    diagnostic: ActionabilityDiagnostic::EffectUnknown { .. },
                    ..
                }
            ) {
                result.outcome = Some("effect_unknown".to_string());
            }
            result.retries = diagnostics.attempts.unwrap_or_default();
            result.actionability = Some(diagnostics);
            result
        }
    }
}

fn click_if_exists_skip_reason(state: &refact_browser::ElementState) -> Option<&'static str> {
    let required = required_states(ActionKind::Click);
    if required.visible && !state.visible {
        Some("element is not visible")
    } else if required.enabled && !state.enabled {
        Some("element is not enabled")
    } else if required.stable && !state.stable {
        Some("element is not stable")
    } else {
        None
    }
}

fn probe_click_if_exists(
    tab: &Tab,
    world: &WorldManager,
    locator: &BrowserLocator,
) -> Result<(), String> {
    let resolved = match resolve_element_typed(tab, world, locator) {
        Ok(resolved) => resolved,
        Err(ResolveElementError::MultipleMatches {
            count, ..
        }) => {
            return Err(format!("locator resolved to {count} elements"));
        }
        Err(ResolveElementError::Other(message)) => return Err(message),
    };
    let state = world.element_states(tab, &resolved.handle);
    let _ = world.release_handle(tab, &resolved.handle);
    match state {
        Ok(state) => match click_if_exists_skip_reason(&state) {
            Some(reason) => Err(format!("not actionable, {reason}")),
            None => Ok(()),
        },
        Err(error) => Err(format!("not actionable, {error}")),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ClickIfExistsProbe {
    Proceed,
    Skip(String),
    Fatal(String),
}

fn classify_click_if_exists_probe(probe: Result<(), String>) -> ClickIfExistsProbe {
    match probe {
        Ok(()) => ClickIfExistsProbe::Proceed,
        Err(error) if refact_browser::is_transport_dead_error(&error) => {
            ClickIfExistsProbe::Fatal(error)
        }
        Err(error) => ClickIfExistsProbe::Skip(error),
    }
}

fn click_if_exists_action_result(mut result: StepResult) -> StepResult {
    let error = result
        .error
        .clone()
        .unwrap_or_else(|| result.summary.clone());
    if result.ok
        || refact_browser::is_transport_dead_error(&error)
        || result.outcome.as_deref() == Some("effect_unknown")
    {
        return result;
    }
    let mut data = result.data.take().unwrap_or_else(|| serde_json::json!({}));
    if let Some(object) = data.as_object_mut() {
        object.insert("clicked".to_string(), serde_json::Value::Bool(false));
        object.insert(
            "skip_reason".to_string(),
            serde_json::Value::String(error.clone()),
        );
    }
    StepResult {
        ok: true,
        summary: format!("Click skipped (non-fatal): {error}"),
        error: None,
        data: Some(data),
        ..result
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
    match classify_click_if_exists_probe(probe_click_if_exists(tab, world, locator)) {
        ClickIfExistsProbe::Proceed => {}
        ClickIfExistsProbe::Skip(reason) => {
            return StepResult::success(
                idx,
                format!(
                    "Skipped click_if_exists ({}): {reason}",
                    describe_locator(locator)
                ),
            );
        }
        ClickIfExistsProbe::Fatal(error) => {
            return StepResult::failure(
                idx,
                format!("click_if_exists failed ({})", describe_locator(locator)),
                error,
            );
        }
    }
    click_if_exists_action_result(step_actionable_action(
        tab,
        world,
        idx,
        locator,
        "click",
        ActionKind::Click,
        handlers,
        locator_handler_firings,
        image_policy,
    ))
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
                );
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

enum TapTarget<'a> {
    Element(&'a BrowserLocator),
    Point(f64, f64),
}

fn tap_target(
    locator: Option<&BrowserLocator>,
    x: Option<f64>,
    y: Option<f64>,
) -> Result<TapTarget<'_>, &'static str> {
    match (locator, x, y) {
        (Some(locator), None, None) => Ok(TapTarget::Element(locator)),
        (None, Some(x), Some(y)) => Ok(TapTarget::Point(x, y)),
        _ => Err(TAP_AMBIGUOUS_TARGET),
    }
}

fn touch_emulation_enabled(tab: &Tab) -> bool {
    eval_js_value(
        tab,
        "navigator.maxTouchPoints > 0 || 'ontouchstart' in window",
    )
    .ok()
    .and_then(|value| value.as_bool())
    .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn step_tap(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: Option<&BrowserLocator>,
    x: Option<f64>,
    y: Option<f64>,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
    mouse_state: &mut MouseState,
) -> StepResult {
    let target = match tap_target(locator, x, y) {
        Ok(target) => target,
        Err(error) => return StepResult::failure(idx, "Tap failed", error),
    };
    if !touch_emulation_enabled(tab) {
        return StepResult::failure(idx, "Tap failed", TAP_REQUIRES_TOUCH);
    }
    match target {
        TapTarget::Element(locator) => step_actionable_action(
            tab,
            world,
            idx,
            locator,
            "tap",
            ActionKind::Tap,
            handlers,
            locator_handler_firings,
            image_policy,
        ),
        TapTarget::Point(x, y) => step_mouse(
            tab,
            idx,
            mouse_state,
            |mouse| mouse.tap(x, y),
            format!("Tapped at ({x}, {y})"),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn focus_for_keyboard(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    action: &str,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
) -> Option<StepResult> {
    let result = step_actionable_action(
        tab,
        world,
        idx,
        locator,
        action,
        ActionKind::Focus,
        handlers,
        locator_handler_firings,
        image_policy,
    );
    (!result.ok).then_some(result)
}

#[allow(clippy::too_many_arguments)]
fn step_insert_text(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: Option<&BrowserLocator>,
    text: &str,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
) -> StepResult {
    if let Some(locator) = locator {
        if let Some(failure) = focus_for_keyboard(
            tab,
            world,
            idx,
            locator,
            "insert_text",
            handlers,
            locator_handler_firings,
            image_policy,
        ) {
            return failure;
        }
    }
    let mut keyboard = Keyboard::new(CdpKeyboardDispatcher::new(tab));
    match keyboard.insert_text(text) {
        Ok(()) => StepResult::success(
            idx,
            format!("Inserted {} chars without key events", text.chars().count()),
        ),
        Err(error) => StepResult::failure(idx, "Insert text failed", error),
    }
}

#[allow(clippy::too_many_arguments)]
fn step_press_sequentially(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    text: &str,
    delay_ms: Option<u64>,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    locator_handler_firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
) -> StepResult {
    if let Some(failure) = focus_for_keyboard(
        tab,
        world,
        idx,
        locator,
        "press_sequentially",
        handlers,
        locator_handler_firings,
        image_policy,
    ) {
        return failure;
    }
    let delay = delay_ms
        .filter(|delay_ms| *delay_ms > 0)
        .map(Duration::from_millis);
    let mut keyboard = Keyboard::new(CdpKeyboardDispatcher::new(tab));
    match keyboard.press_sequentially(text, delay) {
        Ok(()) => StepResult::success(
            idx,
            format!(
                "Pressed {} chars sequentially into ({})",
                text.chars().count(),
                describe_locator(locator)
            ),
        ),
        Err(error) => StepResult::failure(idx, "Press sequentially failed", error),
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
    let info = match resolve_scoped_element(tab, world, locator) {
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
            result.locator_echo = generate_locator_echo(tab, world, &info.handle);
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
            if error.effect_unknown {
                result.outcome = Some("effect_unknown".to_string());
            }
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
    let info = match resolve_scoped_element(tab, world, locator) {
        Ok(i) => i,
        Err(e) => return StepResult::failure(idx, "Clear: element resolution failed", e),
    };
    match refact_browser::forms::clear(tab, world, &info.handle, &info.info, verify) {
        Ok(outcome) => {
            let mut result = StepResult::success(idx, format!("Cleared <{}>", info.tag));
            result.locator_echo = generate_locator_echo(tab, world, &info.handle);
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
            if error.effect_unknown {
                result.outcome = Some("effect_unknown".to_string());
            }
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
    match resolve_scoped_element(tab, world, locator) {
        Ok(info) => match refact_browser::forms::select_option(tab, world, &info.handle, value) {
            Ok(outcome) => {
                let mut result =
                    StepResult::success(idx, format!("Selected '{}' in <{}>", value, info.tag))
                        .with_data(serde_json::json!({"selected": outcome.selected}))
                        .with_locator_echo(generate_locator_echo(tab, world, &info.handle));
                result.actionability = outcome.actionability;
                result
            }
            Err(error) => {
                let mut result = StepResult::failure(idx, "Select option failed", error.message);
                result.retries = error.retries;
                result.actionability = error.actionability;
                if error.effect_unknown {
                    result.outcome = Some("effect_unknown".to_string());
                }
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
    let info = match resolve_scoped_element(tab, world, locator) {
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
                }))
                .with_locator_echo(generate_locator_echo(tab, world, &info.handle));
            result.actionability = outcome.actionability;
            result
        }
        Err(error) => {
            let mut result = StepResult::failure(idx, format!("{} failed", action), error.message);
            result.retries = error.retries;
            result.actionability = error.actionability;
            if error.effect_unknown {
                result.outcome = Some("effect_unknown".to_string());
            }
            result
        }
    }
}

fn poll_deadline_error(last_error: Option<String>, timeout_message: String) -> String {
    match last_error {
        Some(error) if error.starts_with(refact_browser::FRAME_DETACHED_PREFIX) => error,
        Some(error) => format!("Polling failed: {error}"),
        None => timeout_message,
    }
}

fn poll_condition(
    tab: &Tab,
    js_condition: &str,
    timeout_ms: u64,
    interval_ms: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut last_error: Option<String>;
    loop {
        match eval_js_value(tab, js_condition) {
            Ok(val) if val.as_bool() == Some(true) => return Ok(()),
            Ok(_) => last_error = None,
            Err(error) if refact_browser::is_transport_dead_error(&error) => {
                return Err(format!("Polling failed: {error}"));
            }
            Err(error) => last_error = Some(error),
        }
        if Instant::now() >= deadline {
            return Err(poll_deadline_error(
                last_error,
                format!("Timed out after {}ms", timeout_ms),
            ));
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

fn console_entry_matches(
    entry: &refact_integrations::browser_types::ConsoleEntry,
    contains: Option<&str>,
    level: Option<BrowserConsoleLevel>,
) -> bool {
    let text_matches = contains.is_none_or(|needle| entry.text.contains(needle));
    let level_matches = level.is_none_or(|level| level.matches_level(&entry.level));
    text_matches && level_matches
}

async fn wait_for_console_message(
    runtime_arc: &Arc<AMutex<BrowserRuntime>>,
    idx: usize,
    contains: Option<&str>,
    level: Option<BrowserConsoleLevel>,
    armed_cursor: usize,
    timeout_ms: u64,
) -> StepResult {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut cursor = armed_cursor;
    loop {
        let matched = {
            let mut rt = runtime_arc.lock().await;
            rt.drain_raw_events();
            cursor = cursor.min(rt.console_buffer.len());
            let found = rt.console_buffer[cursor..]
                .iter()
                .position(|entry| console_entry_matches(entry, contains, level));
            match found {
                Some(offset) => {
                    let entry = rt.console_buffer[cursor + offset].clone();
                    cursor += offset + 1;
                    Some(mask_console_entry(entry))
                }
                None => {
                    cursor = rt.console_buffer.len();
                    None
                }
            }
        };
        if let Some(entry) = matched {
            return StepResult::success(
                idx,
                format!("Matched console {}: {}", entry.level, entry.text),
            )
            .with_data(serde_json::to_value(entry).unwrap_or_default());
        }
        if Instant::now() >= deadline {
            return StepResult::failure(
                idx,
                "Wait for console message",
                format!("Timed out after {timeout_ms}ms"),
            );
        }
        tokio::time::sleep(Duration::from_millis(CONSOLE_POLL_INTERVAL_MS)).await;
    }
}

fn wait_for_selector_matches(
    state: BrowserWaitState,
    attached: usize,
    visible: usize,
) -> Option<usize> {
    match state {
        BrowserWaitState::Attached => (attached > 0).then_some(attached),
        BrowserWaitState::Detached => (attached == 0).then_some(0),
        BrowserWaitState::Visible => (visible > 0).then_some(visible),
        BrowserWaitState::Hidden => (visible == 0).then_some(attached),
    }
}

fn handle_is_visible(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
) -> Result<bool, String> {
    world
        .element_state(tab, handle, ElementStateName::Visible)
        .map_err(|error| error.to_string())
        .map(|value| value.get("matches") == Some(&serde_json::Value::Bool(true)))
}

fn count_visible_handles(
    tab: &Tab,
    world: &WorldManager,
    handles: &[ElementHandle],
) -> Result<usize, String> {
    let mut visible = 0;
    for handle in handles {
        if handle_is_visible(tab, world, handle)? {
            visible += 1;
        }
    }
    Ok(visible)
}

fn step_wait_for_selector(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    state: Option<BrowserWaitState>,
    timeout_ms: u64,
) -> StepResult {
    let state = state.unwrap_or_default();
    match poll_locator_until(tab, world, locator, timeout_ms, |handles| {
        let visible = if state.needs_visibility() {
            count_visible_handles(tab, world, &handles)
        } else {
            Ok(0)
        };
        release_locator_handles(tab, world, &handles);
        Ok(wait_for_selector_matches(state, handles.len(), visible?))
    }) {
        Ok(matched) => StepResult::success(
            idx,
            format!(
                "{} ({}), {matched} match(es)",
                state.reached_label(),
                describe_locator(locator)
            ),
        ),
        Err(e) => StepResult::failure(
            idx,
            format!("Wait for selector ({})", describe_locator(locator)),
            e,
        ),
    }
}

fn navigation_load_state_step(idx: usize, summary: String, wait: Result<(), String>) -> StepResult {
    match wait {
        Ok(()) => StepResult::success(idx, summary),
        Err(error) if refact_browser::is_transport_dead_error(&error) => {
            StepResult::failure(idx, summary, error)
        }
        Err(error) => {
            StepResult::success(idx, format!("{summary} (load state not reached: {error})"))
                .with_data(serde_json::json!({"load_state_reached": false}))
        }
    }
}

fn step_wait_for_navigation(
    tab: &Tab,
    idx: usize,
    timeout_ms: u64,
    pre_step_url: Option<&str>,
) -> StepResult {
    let settings = browser_settings::current();
    let current_url = tab.get_url();
    let reference_url = pre_step_url.unwrap_or(&current_url);
    let complete_js = r#"(function() { return document.readyState === 'complete'; })()"#;

    if current_url != reference_url {
        return navigation_load_state_step(
            idx,
            format!("Navigation detected: {} -> {}", reference_url, current_url),
            poll_condition(
                tab,
                complete_js,
                timeout_ms,
                settings.timing.default_poll_interval_ms,
            ),
        );
    }

    let url_changed_js = format!(
        r#"(function() {{ return window.location.href !== {}; }})()"#,
        js_string_literal(reference_url),
    );

    match poll_condition(
        tab,
        &url_changed_js,
        timeout_ms,
        settings.timing.default_poll_interval_ms,
    ) {
        Ok(()) => {
            let end_url = tab.get_url();
            navigation_load_state_step(
                idx,
                format!("Navigation detected: {} -> {}", reference_url, end_url),
                poll_condition(
                    tab,
                    complete_js,
                    timeout_ms,
                    settings.timing.default_poll_interval_ms,
                ),
            )
        }
        Err(error) => StepResult::failure(
            idx,
            "Wait for navigation",
            format!("{error}; URL unchanged ({current_url})"),
        ),
    }
}

fn wait_for_url_matcher(pattern: &UrlPattern) -> Result<UrlMatcher, String> {
    match pattern {
        UrlPattern::Text(value) => UrlMatcher::substring(value),
        UrlPattern::Regex {
            source,
            flags,
        } => UrlMatcher::regex(source, flags),
    }
}

fn describe_url_pattern(pattern: &UrlPattern) -> String {
    match pattern {
        UrlPattern::Text(value) => format!("'{value}'"),
        UrlPattern::Regex {
            source,
            flags,
        } => format!("/{source}/{flags}"),
    }
}

fn poll_url_match(tab: &Tab, matcher: &UrlMatcher, timeout_ms: u64) -> Result<String, String> {
    let settings = browser_settings::current();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut last_error: Option<String>;
    loop {
        match eval_js_value(tab, "window.location.href") {
            Ok(value) => {
                let url = value.as_str().unwrap_or_default().to_string();
                if matcher.is_match(&url) {
                    return Ok(url);
                }
                last_error = None;
            }
            Err(error) if refact_browser::is_transport_dead_error(&error) => {
                return Err(format!("Polling failed: {error}"));
            }
            Err(error) => last_error = Some(error),
        }
        if Instant::now() >= deadline {
            return Err(poll_deadline_error(
                last_error,
                format!("Timed out after {}ms", timeout_ms),
            ));
        }
        std::thread::sleep(Duration::from_millis(
            settings.timing.default_poll_interval_ms,
        ));
    }
}

fn step_wait_for_url(tab: &Tab, idx: usize, pattern: &UrlPattern, timeout_ms: u64) -> StepResult {
    let matcher = match wait_for_url_matcher(pattern) {
        Ok(matcher) => matcher,
        Err(error) => return StepResult::failure(idx, "Invalid URL pattern", error),
    };
    let description = describe_url_pattern(pattern);
    match poll_url_match(tab, &matcher, timeout_ms) {
        Ok(url) => StepResult::success(idx, format!("URL matches {description}: {url}")),
        Err(error) => {
            StepResult::failure(idx, format!("Wait for URL matching {description}"), error)
        }
    }
}

fn step_wait_for_text(tab: &Tab, idx: usize, text: &str, timeout_ms: u64) -> StepResult {
    let settings = browser_settings::current();
    let js = browser_locators::js_check_text_present(text);
    match poll_condition(
        tab,
        &js,
        timeout_ms,
        settings.timing.default_poll_interval_ms,
    ) {
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
        let handle = world.wrap_scoped(tab, handle);
        handle_is_visible(tab, world, &handle).map(|visible| (!visible).then_some(()))
    }) {
        Ok(()) => StepResult::success(idx, "Element is hidden".to_string()),
        Err(e) => StepResult::failure(idx, "Wait for element hidden", e),
    }
}

fn injected_state_is_stable(state: &Value) -> bool {
    state
        .get("stable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn step_wait_for_element_stable(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    timeout_ms: u64,
) -> StepResult {
    match poll_locator_until(tab, world, locator, timeout_ms, |handles| {
        let Some(handle) = strict_locator_handle(tab, world, locator, handles)? else {
            return Ok(None);
        };
        let handle = world.wrap_scoped(tab, handle);
        let state = world
            .element_state(tab, &handle, ElementStateName::Stable)
            .map_err(|error| error.to_string())?;
        Ok(injected_state_is_stable(&state).then_some(()))
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
    let settings = browser_settings::current();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut last_error: Option<String>;
    loop {
        match resolve_locator_handles(tab, world, locator).and_then(&mut sample) {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => last_error = None,
            Err(error) if refact_browser::is_transport_dead_error(&error) => return Err(error),
            Err(error) if is_transient_page_error(&error) => last_error = Some(error),
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            return Err(poll_deadline_error(
                last_error,
                format!("Timed out after {}ms", timeout_ms),
            ));
        }
        std::thread::sleep(Duration::from_millis(
            settings.timing.default_poll_interval_ms,
        ));
    }
}

fn is_transient_page_error(error: &str) -> bool {
    if refact_browser::is_stale_frame_error(error) {
        return true;
    }
    let lower = error.to_ascii_lowercase();
    if lower.starts_with(refact_browser::FRAME_DETACHED_PREFIX) {
        return true;
    }
    [
        "inspected target navigated",
        "no node with given id",
        "could not find node with given id",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn step_wait_seconds(idx: usize, seconds: f64) -> StepResult {
    let ms = (seconds * 1000.0) as u64;
    std::thread::sleep(Duration::from_millis(ms));
    StepResult::success(idx, format!("Waited {:.1}s", seconds))
}

const NETWORK_IDLE_WINDOW_MS: u64 = refact_browser::NETWORK_IDLE_MS;

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
    let settings = browser_settings::current();
    let _ = tab.evaluate(NETWORK_INFLIGHT_TRACKER_JS, false);

    let snapshot_js = r#"(function() {
  var inflight = window.__refact_inflight_installed ? (window.__refact_inflight | 0) : -1;
  return JSON.stringify({inflight: inflight, ready: document.readyState});
})()"#;

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let idle_window = Duration::from_millis(NETWORK_IDLE_WINDOW_MS);
    let poll = Duration::from_millis(settings.timing.default_poll_interval_ms);
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
    match resolve_scoped_element(tab, world, locator) {
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
    match resolve_scoped_element(tab, world, locator) {
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
    match resolve_scoped_element(tab, world, locator) {
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

fn element_bounding_box(info: &ElementInfo) -> Value {
    match (&info.bbox, info.visible) {
        (Some(bbox), true) => serde_json::json!({
            "x": bbox.x,
            "y": bbox.y,
            "width": bbox.width,
            "height": bbox.height,
        }),
        _ => Value::Null,
    }
}

fn step_bounding_box(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
) -> StepResult {
    let resolved = match resolve_element(tab, world, locator) {
        Ok(resolved) => resolved,
        Err(error) => return StepResult::failure(idx, "Bounding box: resolution failed", error),
    };
    let bounding_box = element_bounding_box(&resolved.info);
    let _ = world.release_handle(tab, &resolved.handle);
    let summary = if bounding_box.is_null() {
        format!("<{}> is not visible and has no bounding box", resolved.tag)
    } else {
        format!("Got bounding box of <{}>", resolved.tag)
    };
    StepResult::success(idx, summary).with_data(serde_json::json!({"bounding_box": bounding_box}))
}

fn step_count(tab: &Tab, world: &WorldManager, idx: usize, locator: &BrowserLocator) -> StepResult {
    if locator_supports_count_precheck(locator) {
        let counted = locator_without_frames(locator)
            .and_then(|value| world.count_injected_matches(tab, value, 0));
        if let Ok((count, _)) = counted {
            return StepResult::success(
                idx,
                format!("Matched {count} element(s) ({})", describe_locator(locator)),
            )
            .with_data(serde_json::json!({"count": count}));
        }
    }
    match resolve_locator_handles(tab, world, locator) {
        Ok(handles) => {
            let count = handles.len();
            release_locator_handles(tab, world, &handles);
            StepResult::success(
                idx,
                format!("Matched {count} element(s) ({})", describe_locator(locator)),
            )
            .with_data(serde_json::json!({"count": count}))
        }
        Err(error) => StepResult::failure(idx, "Count failed", error),
    }
}

fn step_input_value(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
) -> StepResult {
    let resolved = match resolve_element(tab, world, locator) {
        Ok(resolved) => resolved,
        Err(error) => return StepResult::failure(idx, "Input value: resolution failed", error),
    };
    let read = call_handle_json(
        tab,
        world,
        &resolved.handle,
        browser_locators::js_input_value(),
    );
    let _ = world.release_handle(tab, &resolved.handle);
    match read {
        Ok(result) => {
            let value = result.get("value").cloned().unwrap_or(Value::Null);
            StepResult::success(idx, format!("Got input value of <{}>", resolved.tag))
                .with_data(serde_json::json!({"value": value}))
        }
        Err(error) => StepResult::failure(idx, "Input value failed", error),
    }
}

fn all_texts_limit(limit: Option<usize>) -> usize {
    let settings = browser_settings::current();
    limit
        .unwrap_or(settings.capture.default_all_texts)
        .min(MAX_ALL_TEXTS)
}

fn step_all_texts(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    mode: BrowserTextMode,
    limit: Option<usize>,
) -> StepResult {
    let handles = match resolve_locator_handles(tab, world, locator) {
        Ok(handles) => handles,
        Err(error) => return StepResult::failure(idx, "All texts: resolution failed", error),
    };
    let total = handles.len();
    let mut texts = Vec::new();
    let mut failure = None;
    for handle in handles.iter().take(all_texts_limit(limit)) {
        match call_handle_json(tab, world, handle, browser_locators::js_element_text(mode)) {
            Ok(result) => texts.push(result.get("text").cloned().unwrap_or(Value::Null)),
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    release_locator_handles(tab, world, &handles);
    match failure {
        Some(error) => StepResult::failure(idx, "All texts failed", error),
        None => StepResult::success(idx, format!("Read {} of {total} text(s)", texts.len()))
            .with_data(serde_json::json!({"texts": texts, "total": total})),
    }
}

fn step_element_state(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
) -> StepResult {
    let resolved = match resolve_element(tab, world, locator) {
        Ok(resolved) => resolved,
        Err(error) => return StepResult::failure(idx, "Element state: resolution failed", error),
    };
    let state = world.element_states(tab, &resolved.handle);
    let _ = world.release_handle(tab, &resolved.handle);
    match state {
        Ok(state) => StepResult::success(idx, format!("Read element state of <{}>", resolved.tag))
            .with_data(serde_json::json!({"state": state})),
        Err(error) => StepResult::failure(idx, "Element state failed", error.to_string()),
    }
}

fn step_extract_links(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: Option<&BrowserLocator>,
    limit: Option<usize>,
) -> StepResult {
    let settings = browser_settings::current();
    let effective_limit = limit.unwrap_or(50).min(settings.capture.max_extract_links);
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

fn truncate_table_rows(mut result: Value, limit: Option<usize>) -> Value {
    let settings = browser_settings::current();
    let effective_limit = limit
        .unwrap_or(settings.capture.max_extract_table_rows)
        .min(settings.capture.max_extract_table_rows);
    if let Some(rows) = result.get_mut("rows").and_then(Value::as_array_mut) {
        rows.truncate(effective_limit);
    }
    result
}

fn step_extract_table(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    limit: Option<usize>,
) -> StepResult {
    match resolve_scoped_element(tab, world, locator) {
        Ok(info) => match call_handle_json(
            tab,
            world,
            &info.handle,
            browser_locators::js_extract_table(),
        ) {
            Ok(result) => StepResult::success(idx, format!("Extracted table from <{}>", info.tag))
                .with_data(truncate_table_rows(result, limit)),
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
    let settings = browser_settings::current();
    let limit = max_chars
        .unwrap_or(5000)
        .min(settings.capture.max_dom_snapshot_chars);
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
    let settings = browser_settings::current();
    let root = match options.locator.as_ref() {
        Some(locator) => match resolve_element(tab, world, locator) {
            Ok(resolved) => Some(resolved.handle),
            Err(error) => {
                return StepResult::failure(
                    idx,
                    "Accessibility snapshot root resolution failed",
                    error,
                );
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
        depth: options.depth,
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
                .unwrap_or(settings.capture.default_aria_snapshot_chars)
                .min(settings.capture.max_aria_snapshot_chars);
            let frames = frames_for_snapshot(tab, &snapshot.yaml);
            snapshot.yaml = truncate_chars(snapshot.yaml, limit);
            let generation = snapshot
                .generation
                .as_ref()
                .map(|generation| (generation.document_generation, generation.frame_generation));
            let mut data = accessibility_snapshot_data(snapshot.yaml, generation);
            if !frames.is_empty() {
                if let Some(object) = data.as_object_mut() {
                    object.insert(
                        "frames".to_string(),
                        serde_json::to_value(&frames).unwrap_or_default(),
                    );
                    object.insert(
                        "frames_hint".to_string(),
                        Value::String(
                            "iframe contents are not part of this snapshot; act inside a frame with locator.frames (an outermost-first chain of iframe locators) or snapshot with a locator that targets the iframe"
                                .to_string(),
                        ),
                    );
                }
            }
            StepResult::success(idx, "Accessibility snapshot").with_data(data)
        }
        Err(error) => StepResult::failure(idx, "Accessibility snapshot failed", error.to_string()),
    }
}

fn accessibility_snapshot_data(yaml: String, generation: Option<(u64, u64)>) -> Value {
    let mut data = serde_json::json!({"yaml": yaml});
    if let (Some((document_generation, frame_generation)), Some(object)) =
        (generation, data.as_object_mut())
    {
        object.insert(
            "generation".to_string(),
            serde_json::json!({
                "document_generation": document_generation,
                "frame_generation": frame_generation,
            }),
        );
    }
    data
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
    let options = BrowserScreenshotOptions {
        image_type: Some(match policy.format {
            ImageFormat::Png => BrowserScreenshotType::Png,
            ImageFormat::Jpeg => BrowserScreenshotType::Jpeg,
            ImageFormat::Webp => BrowserScreenshotType::Webp,
        }),
        quality: policy.quality,
        ..Default::default()
    };
    capture_screenshot(tab, &WorldManager::default(), &options, None, policy)
        .map(|capture| (capture.data, capture.mime))
}

fn report_screenshot_requested(
    attach_screenshot: Option<bool>,
    page_context: PageContextMode,
    has_screenshot_step: bool,
) -> bool {
    match attach_screenshot {
        Some(attach) => attach,
        None => match page_context {
            PageContextMode::None => false,
            PageContextMode::Snapshot => has_screenshot_step,
            PageContextMode::Screenshot | PageContextMode::Both => true,
        },
    }
}

fn report_snapshot_requested(page_context: PageContextMode, page_changed: bool) -> bool {
    page_context.includes_snapshot() && page_changed
}

fn console_counts(console: &[ConsoleEntry], page_errors: &[String]) -> BrowserConsoleCounts {
    let level_is = |entry: &ConsoleEntry, names: &[&str]| {
        names
            .iter()
            .any(|name| entry.level.eq_ignore_ascii_case(name))
    };
    BrowserConsoleCounts {
        errors: page_errors.len()
            + console
                .iter()
                .filter(|entry| level_is(entry, &["error", "assert"]))
                .count(),
        warnings: console
            .iter()
            .filter(|entry| level_is(entry, &["warning", "warn"]))
            .count(),
    }
}

fn notable_main_document_status(network: &[NetworkEntry], url: Option<&str>) -> Option<u16> {
    let status = network
        .iter()
        .filter(|entry| entry.resource_type.eq_ignore_ascii_case("document"))
        .filter(|entry| url.is_none_or(|url| entry.url == url))
        .filter_map(|entry| entry.status)
        .next_back()?;
    (!(200..300).contains(&status)).then_some(status)
}

fn snapshot_head(yaml: &str, max_lines: usize) -> String {
    yaml.lines().take(max_lines).collect::<Vec<_>>().join("\n")
}

fn snapshot_mentions_iframe(yaml: &str) -> bool {
    yaml.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("- iframe") || trimmed.starts_with("- frame")
    })
}

fn child_frame_summaries(tab: &Tab) -> Vec<BrowserFrameSummary> {
    fn walk(node: &Page::FrameTree, depth: usize, out: &mut Vec<BrowserFrameSummary>) {
        if depth > 0 {
            out.push(BrowserFrameSummary {
                frame_id: node.frame.id.clone(),
                parent_id: node.frame.parent_id.clone(),
                depth,
                url: refact_browser::mask_text(&node.frame.url),
                name: node.frame.name.clone().filter(|name| !name.is_empty()),
            });
        }
        for child in node.child_frames.iter().flatten() {
            walk(child, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    if let Ok(tree) = tab.call_method(Page::GetFrameTree(None)) {
        walk(&tree.frame_tree, 0, &mut out);
    }
    out
}

fn frames_for_snapshot(tab: &Tab, yaml: &str) -> Vec<BrowserFrameSummary> {
    if snapshot_mentions_iframe(yaml) {
        child_frame_summaries(tab)
    } else {
        Vec::new()
    }
}

fn build_page_snapshot(
    yaml: String,
    store: &refact_browser::artifact_store::ArtifactStore,
) -> Result<BrowserPageSnapshot, String> {
    let settings = browser_settings::current();
    let bytes = yaml.len();
    let lines = yaml.lines().count();
    if bytes <= settings.capture.max_inline_snapshot_bytes {
        return Ok(BrowserPageSnapshot {
            yaml,
            lines,
            bytes,
            truncated: false,
            artifact: None,
            frames: Vec::new(),
        });
    }
    let path = store.write(yaml.as_bytes(), "text/yaml")?.path;
    Ok(BrowserPageSnapshot {
        yaml: snapshot_head(&yaml, SNAPSHOT_SUMMARY_LINES),
        lines,
        bytes,
        truncated: true,
        artifact: Some(BrowserSnapshotArtifact {
            kind: "aria_snapshot".to_string(),
            mime: "text/yaml".to_string(),
            path,
            bytes,
        }),
        frames: Vec::new(),
    })
}

fn capture_page_snapshot(
    tab: &Tab,
    world: &WorldManager,
    store: &refact_browser::artifact_store::ArtifactStore,
) -> Result<BrowserPageSnapshot, String> {
    let snapshot = world
        .aria_snapshot(
            tab,
            None,
            SnapshotOptions {
                mode: SnapshotMode::Ai,
                refs: true,
                ..Default::default()
            },
        )
        .map_err(|error| error.to_string())?;
    let frames = frames_for_snapshot(tab, &snapshot.yaml);
    let mut page_snapshot = build_page_snapshot(snapshot.yaml, store)?;
    page_snapshot.frames = frames;
    Ok(page_snapshot)
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

pub fn capture_viewport_screenshot_png(tab: &Tab) -> Result<(String, String), String> {
    let result = tab
        .call_method(Page::CaptureScreenshot {
            format: Some(Page::CaptureScreenshotFormatOption::Png),
            clip: None,
            quality: None,
            from_surface: Some(true),
            capture_beyond_viewport: Some(false),
            optimize_for_speed: None,
        })
        .map_err(|error| error.to_string())?;
    Ok((result.data, "image/png".to_string()))
}

struct PolicyScreenshot {
    data: String,
    mime: String,
    width: u32,
    height: u32,
    bytes: usize,
}

fn screenshot_metrics(tab: &Tab) -> Result<ScreenshotMetrics, String> {
    let metrics = tab
        .call_method(Page::GetLayoutMetrics(None))
        .map_err(|error| format!("Failed to read screenshot layout metrics: {error}"))?;
    let device_scale_factor = eval_js_value(tab, "window.devicePixelRatio")
        .ok()
        .and_then(|value| value.as_f64())
        .unwrap_or(1.0);
    Ok(ScreenshotMetrics {
        page_x: metrics.visual_viewport.page_x,
        page_y: metrics.visual_viewport.page_y,
        viewport_scale: metrics.visual_viewport.scale.max(f64::EPSILON),
        device_scale_factor,
        viewport_width: metrics.visual_viewport.client_width,
        viewport_height: metrics.visual_viewport.client_height,
        content_width: metrics.content_size.width,
        content_height: metrics.content_size.height,
    })
}

fn omit_background_applies(options: &BrowserScreenshotOptions) -> bool {
    options.omit_background && options.image_type.unwrap_or_default() != BrowserScreenshotType::Jpeg
}

fn screenshot_policy(options: &BrowserScreenshotOptions, policy: &ImagePolicy) -> ImagePolicy {
    policy.clone().with_format(
        match options.image_type.unwrap_or_default() {
            BrowserScreenshotType::Png => ImageFormat::Png,
            BrowserScreenshotType::Jpeg => ImageFormat::Jpeg,
            BrowserScreenshotType::Webp => ImageFormat::Webp,
        },
        options.quality.or(policy.quality),
    )
}

fn capture_screenshot(
    tab: &Tab,
    world: &WorldManager,
    options: &BrowserScreenshotOptions,
    element: Option<BrowserScreenshotClip>,
    policy: &ImagePolicy,
) -> Result<PolicyScreenshot, String> {
    let metrics = screenshot_metrics(tab)?;
    let capture = screenshot_capture(options, metrics, element)?;
    let cleanup = prepare_screenshot(tab, world, options)?;
    let transparent = omit_background_applies(options);
    let background_result = if transparent {
        tab.call_method(Emulation::SetDefaultBackgroundColorOverride {
            color: Some(DOM::RGBA {
                r: 0,
                g: 0,
                b: 0,
                a: Some(0.0),
            }),
        })
        .map(|_| ())
        .map_err(|error| format!("Failed to omit screenshot background: {error}"))
    } else {
        Ok(())
    };
    let raw = background_result.and_then(|_| {
        tab.call_method(Page::CaptureScreenshot {
            format: Some(capture.format),
            clip: Some(capture.clip),
            quality: capture.quality,
            from_surface: Some(true),
            capture_beyond_viewport: Some(capture.capture_beyond_viewport),
            optimize_for_speed: None,
        })
        .map_err(|error| format!("Screenshot capture failed: {error}"))
    });
    let background_cleanup = if transparent {
        tab.call_method(Emulation::SetDefaultBackgroundColorOverride {
            color: None,
        })
        .map(|_| ())
        .map_err(|error| format!("Failed to restore screenshot background: {error}"))
    } else {
        Ok(())
    };
    if cleanup {
        let _ = world.eval_in_utility(tab, "window.__refactScreenshotCleanup?.()");
    }
    let raw = raw?;
    background_cleanup?;
    let raw_bytes = base64::prelude::BASE64_STANDARD
        .decode(raw.data)
        .map_err(|error| format!("Screenshot decode failed: {error}"))?;
    let (processed, mime) = resize_to_policy(
        &raw_bytes,
        capture.mime,
        &screenshot_policy(options, policy),
    )?;
    let decoded = image::load_from_memory(&processed)
        .map_err(|error| format!("Processed screenshot decode failed: {error}"))?;
    Ok(PolicyScreenshot {
        data: base64::prelude::BASE64_STANDARD.encode(&processed),
        mime,
        width: decoded.width(),
        height: decoded.height(),
        bytes: processed.len(),
    })
}

fn prepare_screenshot(
    tab: &Tab,
    world: &WorldManager,
    options: &BrowserScreenshotOptions,
) -> Result<bool, String> {
    let mut boxes = Vec::new();
    for locator in &options.mask {
        let handles = resolve_locator_handles(tab, world, locator)?;
        if handles.is_empty() {
            continue;
        }
        for handle in handles {
            let value = world
                .call_function_on(
                    tab,
                    &handle,
                    "function() { const r = this.getBoundingClientRect(); return {x:r.x+scrollX,y:r.y+scrollY,width:r.width,height:r.height}; }",
                    Vec::new(),
                )
                .map_err(|error| error.to_string());
            let _ = world.release_handle(tab, &handle);
            boxes.push(value?);
        }
    }
    let hide_caret = options.caret.unwrap_or_default() == BrowserScreenshotCaret::Hide;
    let disable_animations =
        options.animations.unwrap_or_default() == BrowserScreenshotAnimations::Disabled;
    if boxes.is_empty() && options.style.is_none() && !hide_caret && !disable_animations {
        return Ok(false);
    }
    let script = prepare_screenshot_script(
        &boxes,
        options.style.as_deref().unwrap_or(""),
        hide_caret,
        disable_animations,
        options.mask_color.as_deref().unwrap_or("#FF00FF"),
    )?;
    world.eval_in_utility(tab, &script)?;
    Ok(true)
}

fn prepare_screenshot_script(
    boxes: &[serde_json::Value],
    style: &str,
    hide_caret: bool,
    disable_animations: bool,
    mask_color: &str,
) -> Result<String, String> {
    Ok(format!(
        r#"(() => {{
  window.__refactScreenshotCleanup?.();
  const root = document.documentElement;
  const css = {} + {} + {};
  const collectRoots = (scope, roots) => {{
    roots.push(scope);
    const walker = document.createTreeWalker(scope, NodeFilter.SHOW_ELEMENT);
    do {{
      const node = walker.currentNode;
      const shadow = node instanceof Element ? node.shadowRoot : null;
      if (shadow) collectRoots(shadow, roots);
    }} while (walker.nextNode());
    return roots;
  }};
  const roots = css || {} ? collectRoots(document, []) : [];
  const styles = [];
  if (css) {{
    for (const scope of roots) {{
      const style = document.createElement('style');
      style.dataset.refactScreenshot = 'true';
      style.textContent = css;
      (scope === document ? root : scope).appendChild(style);
      styles.push(style);
    }}
  }}
  const maskRoot = document.createElement('div');
  maskRoot.dataset.refactScreenshotMask = 'true';
  Object.assign(maskRoot.style, {{position:'absolute',left:'0',top:'0',pointerEvents:'none',zIndex:'2147483647'}});
  const boxes = {};
  const color = {};
  for (const box of boxes) {{
    const mask = document.createElement('div');
    Object.assign(mask.style, {{position:'absolute',left:box.x+'px',top:box.y+'px',width:box.width+'px',height:box.height+'px',background:color}});
    maskRoot.appendChild(mask);
  }}
  root.appendChild(maskRoot);
  const animations = {} ? roots.flatMap(scope => scope.getAnimations()).map(animation => ({{animation,currentTime:animation.currentTime,playState:animation.playState}})) : [];
  for (const saved of animations) {{ try {{ saved.animation.finish(); }} catch {{ saved.animation.cancel(); }} }}
  window.__refactScreenshotCleanup = () => {{
    for (const style of styles) style.remove();
    maskRoot.remove();
    for (const saved of animations) {{
      try {{ saved.animation.currentTime = saved.currentTime; if (saved.playState === 'running') saved.animation.play(); else saved.animation.pause(); }} catch {{}}
    }}
    delete window.__refactScreenshotCleanup;
  }};
}})()"#,
        js_string_literal(style),
        if hide_caret {
            js_string_literal("\n*,*::before,*::after{caret-color:transparent!important}")
        } else {
            js_string_literal("")
        },
        if disable_animations {
            js_string_literal(
                "\n*,*::before,*::after{transition-delay:0s!important;transition-duration:0s!important;animation-delay:0s!important;animation-duration:0s!important}",
            )
        } else {
            js_string_literal("")
        },
        disable_animations,
        serde_json::to_string(boxes).map_err(|error| error.to_string())?,
        js_string_literal(mask_color),
        disable_animations,
    ))
}

fn step_screenshot(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    options: &BrowserScreenshotOptions,
    policy: &ImagePolicy,
) -> StepResult {
    match capture_screenshot(tab, world, options, None, policy) {
        Ok(capture) => {
            StepResult::success(idx, "Screenshot captured").with_data(image_artifact_data(&capture))
        }
        Err(error) => StepResult::failure(idx, "Screenshot failed", error),
    }
}

fn image_artifact_data(capture: &PolicyScreenshot) -> Value {
    serde_json::json!({
        "artifact": {
            "kind": "image",
            "mime": capture.mime,
            "data": capture.data,
            "width": capture.width,
            "height": capture.height,
            "bytes": capture.bytes,
        },
    })
}

fn persist_image_artifact(
    mut result: StepResult,
    store: &refact_browser::artifact_store::ArtifactStore,
) -> StepResult {
    let Some(artifact) = result
        .data
        .as_mut()
        .and_then(Value::as_object_mut)
        .and_then(|data| data.get_mut("artifact"))
        .and_then(Value::as_object_mut)
    else {
        return result;
    };
    if artifact.get("kind").and_then(Value::as_str) != Some("image")
        || artifact.contains_key("path")
    {
        return result;
    }
    let (Some(mime), Some(encoded)) = (
        artifact.get("mime").and_then(Value::as_str),
        artifact.get("data").and_then(Value::as_str),
    ) else {
        return result;
    };
    let Ok(bytes) = base64::prelude::BASE64_STANDARD.decode(encoded) else {
        return result;
    };
    match store.write(&bytes, mime) {
        Ok(saved) => {
            artifact.insert("id".to_string(), Value::String(saved.id));
            artifact.insert(
                "path".to_string(),
                Value::String(saved.path.to_string_lossy().into_owned()),
            );
        }
        Err(error) => return StepResult::failure(result.step_index, "Image artifact store", error),
    }
    result
}

fn is_image_capture_step(step: &BrowserStep) -> bool {
    matches!(
        step,
        BrowserStep::Screenshot { .. } | BrowserStep::ScreenshotElement { .. }
    )
}

fn step_screenshot_element(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    options: &BrowserScreenshotOptions,
    policy: &ImagePolicy,
) -> StepResult {
    let info = match resolve_scoped_element(tab, world, locator) {
        Ok(i) => i,
        Err(e) => return StepResult::failure(idx, "Screenshot element: resolution failed", e),
    };

    let bbox = match &info.bbox {
        Some(b) if b.width > 0.0 && b.height > 0.0 => b,
        _ => {
            return StepResult::failure(idx, "Screenshot element", "Element has no visible bounds");
        }
    };
    let metrics = match screenshot_metrics(tab) {
        Ok(metrics) => metrics,
        Err(error) => return StepResult::failure(idx, "Screenshot element", error),
    };

    let clip = BrowserScreenshotClip {
        x: metrics.page_x + bbox.x,
        y: metrics.page_y + bbox.y,
        width: bbox.width,
        height: bbox.height,
    };

    match capture_screenshot(tab, world, options, Some(clip), policy) {
        Ok(capture) => StepResult::success(idx, format!("Element screenshot of <{}>", info.tag))
            .with_data(image_artifact_data(&capture)),
        Err(e) => StepResult::failure(idx, "Element screenshot failed", e),
    }
}

fn element_viewport_rect(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
) -> Result<BrowserScreenshotClip, String> {
    let value = call_handle_json(
        tab,
        world,
        handle,
        "function() { const r = this.getBoundingClientRect(); return JSON.stringify({x:r.x,y:r.y,width:r.width,height:r.height}); }",
    )?;
    let rect: BrowserScreenshotClip = serde_json::from_value(value)
        .map_err(|error| format!("Failed to read element bounds: {error}"))?;
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return Err("Element has no visible bounds".to_string());
    }
    Ok(rect)
}

fn capture_element(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    options: &BrowserScreenshotOptions,
    policy: &ImagePolicy,
) -> Result<PolicyScreenshot, String> {
    let rect = element_viewport_rect(tab, world, handle)?;
    let metrics = screenshot_metrics(tab)?;
    capture_screenshot(
        tab,
        world,
        options,
        Some(BrowserScreenshotClip {
            x: metrics.page_x + rect.x,
            y: metrics.page_y + rect.y,
            width: rect.width,
            height: rect.height,
        }),
        policy,
    )
}

fn compose_captures(
    captures: &[(String, PolicyScreenshot)],
    layout: ComposeLayout,
    labels: bool,
    options: &BrowserScreenshotOptions,
    policy: &ImagePolicy,
) -> Result<PolicyScreenshot, String> {
    let tiles = captures
        .iter()
        .map(|(label, capture)| {
            base64::prelude::BASE64_STANDARD
                .decode(&capture.data)
                .map(|bytes| (label.clone(), bytes))
                .map_err(|error| format!("Capture decode failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let sheet = compose_sheet(&tiles, layout, labels)?;
    let (processed, mime) =
        resize_to_policy(&sheet, "image/png", &screenshot_policy(options, policy))?;
    let decoded = image::load_from_memory(&processed)
        .map_err(|error| format!("Composed sheet decode failed: {error}"))?;
    Ok(PolicyScreenshot {
        data: base64::prelude::BASE64_STANDARD.encode(&processed),
        mime,
        width: decoded.width(),
        height: decoded.height(),
        bytes: processed.len(),
    })
}

fn capture_json(label: &str, capture: &PolicyScreenshot) -> serde_json::Value {
    serde_json::json!({
        "label": label,
        "mime": capture.mime,
        "data": capture.data,
        "width": capture.width,
        "height": capture.height,
        "bytes": capture.bytes,
    })
}

fn step_screenshot_elements(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locators: &[BrowserLocator],
    compose: BrowserComposeMode,
    labels: Option<bool>,
    options: &BrowserScreenshotOptions,
    policy: &ImagePolicy,
) -> StepResult {
    if locators.is_empty() {
        return StepResult::failure(
            idx,
            "Screenshot elements",
            "screenshot_elements requires at least one locator",
        );
    }
    if locators.len() > policy.max_images {
        return StepResult::failure(
            idx,
            "Screenshot elements",
            format!(
                "screenshot_elements accepts at most {} locators, got {}",
                policy.max_images,
                locators.len()
            ),
        );
    }
    let mut captures = Vec::with_capacity(locators.len());
    for locator in locators {
        let label = describe_locator(locator);
        let resolved = match resolve_element(tab, world, locator) {
            Ok(resolved) => resolved,
            Err(error) => {
                return StepResult::failure(
                    idx,
                    "Screenshot elements",
                    format!("{label}: {error}"),
                );
            }
        };
        let capture = capture_element(tab, world, &resolved.handle, options, policy);
        let _ = world.release_handle(tab, &resolved.handle);
        match capture {
            Ok(capture) => captures.push((label, capture)),
            Err(error) => {
                return StepResult::failure(
                    idx,
                    "Screenshot elements",
                    format!("{label}: {error}"),
                );
            }
        }
    }
    let labels = labels.unwrap_or(true);
    match compose {
        BrowserComposeMode::Separate => StepResult::success(
            idx,
            format!("Captured {} element screenshots", captures.len()),
        )
        .with_data(serde_json::json!({
            "compose": "separate",
            "count": captures.len(),
            "images": captures
                .iter()
                .map(|(label, capture)| capture_json(label, capture))
                .collect::<Vec<_>>(),
        })),
        BrowserComposeMode::Grid => {
            match compose_captures(&captures, ComposeLayout::Grid, labels, options, policy) {
                Ok(sheet) => StepResult::success(
                    idx,
                    format!("Composed {} element screenshots into a grid", captures.len()),
                )
                .with_data(serde_json::json!({
                    "compose": "grid",
                    "count": captures.len(),
                    "labels": captures.iter().map(|(label, _)| label).collect::<Vec<_>>(),
                    "artifact": {"kind": "image", "mime": sheet.mime, "width": sheet.width, "height": sheet.height, "bytes": sheet.bytes},
                    "images": [capture_json("grid", &sheet)],
                })),
                Err(error) => StepResult::failure(idx, "Screenshot elements", error),
            }
        }
    }
}

fn step_capture_element_states(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    states: &[BrowserElementState],
    labels: Option<bool>,
    options: &BrowserScreenshotOptions,
    policy: &ImagePolicy,
) -> StepResult {
    let resolved = match resolve_element(tab, world, locator) {
        Ok(resolved) => resolved,
        Err(error) => return StepResult::failure(idx, "Capture element states", error),
    };
    let outcome = drive_element_states(tab, world, &resolved.handle, states, options, policy);
    let _ = world.release_handle(tab, &resolved.handle);
    let captures = match outcome {
        Ok(captures) => captures,
        Err(error) => return StepResult::failure(idx, "Capture element states", error),
    };
    match compose_captures(
        &captures,
        ComposeLayout::Strip,
        labels.unwrap_or(true),
        options,
        policy,
    ) {
        Ok(strip) => StepResult::success(
            idx,
            format!(
                "Captured <{}> in {} states",
                resolved.info.tag,
                captures.len()
            ),
        )
        .with_data(serde_json::json!({
            "states": captures.iter().map(|(label, _)| label).collect::<Vec<_>>(),
            "artifact": {"kind": "image", "mime": strip.mime, "width": strip.width, "height": strip.height, "bytes": strip.bytes},
            "images": [capture_json("states", &strip)],
        })),
        Err(error) => StepResult::failure(idx, "Capture element states", error),
    }
}

pub fn run_element_state_sequence<T, F>(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    states: &[BrowserElementState],
    observe: F,
) -> Result<Vec<(BrowserElementState, T)>, String>
where
    F: FnMut(BrowserElementState) -> Result<T, String>,
{
    let keyboard = Keyboard::new(CdpKeyboardDispatcher::new(tab));
    let mut mouse = Mouse::new(CdpMouseDispatcher::new(tab), &keyboard);
    drive_element_state_actions(
        &mut mouse,
        element_state_sequence(states),
        || {
            CdpMouseDispatcher::new(tab)
                .clickable_point(handle)
                .map_err(|error| error.to_string())
        },
        |focused| {
            let script = if focused {
                browser_locators::js_focus_element()
            } else {
                browser_locators::js_blur_element()
            };
            call_handle_json(tab, world, handle, &script).map(|_| ())
        },
        observe,
    )
}

fn drive_element_state_actions<D, K, T, F>(
    mouse: &mut Mouse<'_, D, K>,
    actions: impl IntoIterator<Item = ElementStateAction>,
    mut hover_point: impl FnMut() -> Result<MainFrameCssPoint, String>,
    mut set_focus: impl FnMut(bool) -> Result<(), String>,
    mut observe: F,
) -> Result<Vec<(BrowserElementState, T)>, String>
where
    D: MouseDispatcher,
    K: KeyboardDispatcher,
    F: FnMut(BrowserElementState) -> Result<T, String>,
{
    let mut point: Option<MainFrameCssPoint> = None;
    let mut observations = Vec::new();
    let mut held = false;
    let mut outcome = Ok(());
    for action in actions {
        let step = match action {
            ElementStateAction::Hover => match point {
                Some(target) => Ok(target),
                None => hover_point().inspect(|resolved| point = Some(*resolved)),
            }
            .and_then(|target| {
                mouse
                    .hover(target.x, target.y)
                    .map_err(|error| error.to_string())
            }),
            ElementStateAction::MoveMouseAway => mouse
                .move_to(0.0, 0.0, 1)
                .map_err(|error| error.to_string()),
            ElementStateAction::PressAndHold => mouse
                .down(MouseButton::Left, 1)
                .map_err(|error| error.to_string())
                .inspect(|_| held = true),
            ElementStateAction::ReleaseMouse => mouse
                .up(MouseButton::Left, 1)
                .map_err(|error| error.to_string())
                .inspect(|_| held = false),
            ElementStateAction::Focus => set_focus(true),
            ElementStateAction::Blur => set_focus(false),
            ElementStateAction::Capture(state) => {
                observe(state).map(|observed| observations.push((state, observed)))
            }
        };
        if let Err(error) = step {
            outcome = Err(error);
            break;
        }
    }
    if held {
        let _ = mouse.up(MouseButton::Left, 1);
    }
    outcome.map(|()| observations)
}

fn drive_element_states(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    states: &[BrowserElementState],
    options: &BrowserScreenshotOptions,
    policy: &ImagePolicy,
) -> Result<Vec<(String, PolicyScreenshot)>, String> {
    let captures = run_element_state_sequence(tab, world, handle, states, |_| {
        capture_element(tab, world, handle, options, policy)
    })?;
    Ok(captures
        .into_iter()
        .map(|(state, capture)| (state.label().to_string(), capture))
        .collect())
}

const PDF_INLINE_LIMIT_BYTES: usize = 256 * 1024;

fn step_pdf(
    tab: &Tab,
    idx: usize,
    options: &BrowserPdfOptions,
    store: &refact_browser::artifact_store::ArtifactStore,
) -> StepResult {
    let result = (|| -> Result<(PathBuf, Vec<u8>), String> {
        let payload = pdf_payload(options)?;
        let response = tab
            .call_method(payload)
            .map_err(|error| format!("PDF generation failed: {error}"))?;
        let bytes = if let Some(stream) = response.stream {
            read_cdp_stream(tab, stream)?
        } else {
            base64::prelude::BASE64_STANDARD
                .decode(response.data)
                .map_err(|error| format!("PDF decode failed: {error}"))?
        };
        let path = store.write(&bytes, "application/pdf")?.path;
        Ok((path, bytes))
    })();
    match result {
        Ok((path, bytes)) => {
            let inline = (bytes.len() <= PDF_INLINE_LIMIT_BYTES)
                .then(|| base64::prelude::BASE64_STANDARD.encode(&bytes));
            StepResult::success(idx, format!("PDF saved to {}", path.display())).with_data(
                serde_json::json!({
                    "artifact": {
                        "kind": "pdf",
                        "mime": "application/pdf",
                        "path": path,
                        "bytes": bytes.len(),
                        "data": inline,
                    }
                }),
            )
        }
        Err(error) => StepResult::failure(idx, "PDF generation failed", error),
    }
}

fn read_cdp_stream(tab: &Tab, stream: IO::StreamHandle) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    loop {
        let chunk = tab
            .call_method(IO::Read {
                handle: stream.clone(),
                offset: None,
                size: Some(64 * 1024),
            })
            .map_err(|error| format!("PDF stream read failed: {error}"))?;
        if chunk.base_64_encoded.unwrap_or(false) {
            bytes.extend(
                base64::prelude::BASE64_STANDARD
                    .decode(chunk.data)
                    .map_err(|error| format!("PDF stream decode failed: {error}"))?,
            );
        } else {
            bytes.extend_from_slice(chunk.data.as_bytes());
        }
        if chunk.eof {
            break;
        }
    }
    tab.call_method(IO::Close {
        handle: stream,
    })
    .map_err(|error| format!("PDF stream close failed: {error}"))?;
    Ok(bytes)
}

fn eval_invocation_target(
    object_type: &Runtime::RemoteObjectType,
    object_id: Option<&String>,
) -> Option<String> {
    matches!(object_type, Runtime::RemoteObjectType::Function)
        .then(|| object_id.cloned())
        .flatten()
}

fn invoke_eval_function(tab: &Tab, object_id: String) -> Result<Runtime::RemoteObject, String> {
    let invoked = tab
        .call_method(Runtime::CallFunctionOn {
            function_declaration: "function() { return this(); }".to_string(),
            object_id: Some(object_id),
            arguments: None,
            silent: None,
            return_by_value: Some(false),
            generate_preview: Some(true),
            user_gesture: Some(false),
            await_promise: Some(true),
            execution_context_id: None,
            object_group: None,
            throw_on_side_effect: None,
            unique_context_id: None,
            serialization_options: None,
        })
        .map_err(|error| error.to_string())?;
    if let Some(exception) = invoked.exception_details {
        return Err(js_exception_message(&exception));
    }
    Ok(invoked.result)
}

const EVAL_DEFAULT_TIMEOUT_MS: u64 = 30_000;

fn step_eval(tab: &Tab, idx: usize, expression: &str, timeout_ms: Option<u64>) -> StepResult {
    let timeout_ms = timeout_ms.unwrap_or(EVAL_DEFAULT_TIMEOUT_MS);
    let evaluated = match tab.call_method(Runtime::Evaluate {
        expression: expression.to_string(),
        return_by_value: Some(false),
        generate_preview: Some(true),
        silent: Some(false),
        await_promise: Some(true),
        include_command_line_api: Some(false),
        user_gesture: Some(true),
        object_group: None,
        context_id: None,
        throw_on_side_effect: None,
        timeout: Some(timeout_ms as f64),
        disable_breaks: None,
        repl_mode: None,
        allow_unsafe_eval_blocked_by_csp: None,
        unique_context_id: None,
        serialization_options: None,
    }) {
        Ok(evaluated) => evaluated,
        Err(error) => {
            let message = error.to_string();
            let summary = if message.contains("timed out") || message.contains("Timeout") {
                format!("Eval timed out after {timeout_ms}ms")
            } else {
                "Eval failed".to_string()
            };
            return StepResult::failure(idx, summary, message);
        }
    };
    if let Some(exception) = evaluated.exception_details {
        return StepResult::failure(
            idx,
            "Eval threw an exception",
            js_exception_message(&exception),
        );
    }
    let remote =
        match eval_invocation_target(&evaluated.result.Type, evaluated.result.object_id.as_ref()) {
            Some(object_id) => match invoke_eval_function(tab, object_id) {
                Ok(remote) => remote,
                Err(error) => return StepResult::failure(idx, "Eval threw an exception", error),
            },
            None => evaluated.result,
        };
    let (value, description) = eval_remote_object_value(tab, remote);
    let limit = browser_settings::current().capture.max_eval_result_chars;
    let (value, truncation) = bound_eval_value(value, limit);
    let mut data = serde_json::json!({"value": value, "description": description});
    if let Some(truncation) = truncation {
        data["truncated"] = serde_json::json!(true);
        data["total_chars"] = serde_json::json!(truncation.total_chars);
        data["max_chars"] = serde_json::json!(limit);
        data["note"] = serde_json::json!(format!(
            "eval result was {} characters; only the first {} are inlined — return a smaller value, write the data to the page, or fetch it in slices",
            truncation.total_chars, limit
        ));
    }
    StepResult::success(idx, "Eval completed".to_string()).with_data(data)
}

struct EvalTruncation {
    total_chars: usize,
}

/// Caps an eval result at `limit` characters of JSON: strings keep their head, anything
/// else becomes a JSON-text preview, so one `'x'.repeat(5e6)` cannot swamp the tool result.
fn bound_eval_value(value: Value, limit: usize) -> (Value, Option<EvalTruncation>) {
    if limit == 0 {
        return (value, None);
    }
    match value {
        Value::String(text) => {
            let total_chars = text.chars().count();
            if total_chars <= limit {
                return (Value::String(text), None);
            }
            let head = text.chars().take(limit).collect::<String>();
            (
                Value::String(head),
                Some(EvalTruncation {
                    total_chars,
                }),
            )
        }
        other => {
            let serialized = other.to_string();
            let total_chars = serialized.chars().count();
            if total_chars <= limit {
                return (other, None);
            }
            let head = serialized.chars().take(limit).collect::<String>();
            (
                Value::String(head),
                Some(EvalTruncation {
                    total_chars,
                }),
            )
        }
    }
}

fn eval_remote_object_value(tab: &Tab, remote: Runtime::RemoteObject) -> (Value, String) {
    let description = remote
        .description
        .clone()
        .unwrap_or_else(|| format!("{:?}", remote.Type).to_ascii_lowercase());
    if let Some(value) = remote.value {
        return (value, description);
    }
    let Some(object_id) = remote.object_id else {
        return (Value::Null, description);
    };
    let serialized = tab
        .call_method(Runtime::CallFunctionOn {
            function_declaration: EVAL_SERIALIZE_JS.to_string(),
            object_id: Some(object_id),
            arguments: None,
            silent: Some(true),
            return_by_value: Some(true),
            generate_preview: None,
            user_gesture: Some(false),
            await_promise: Some(false),
            execution_context_id: None,
            object_group: None,
            throw_on_side_effect: None,
            unique_context_id: None,
            serialization_options: None,
        })
        .ok()
        .filter(|invoked| invoked.exception_details.is_none())
        .and_then(|invoked| invoked.result.value);
    match serialized {
        Some(value) => (value, description),
        None => (Value::Null, description),
    }
}

const EVAL_SERIALIZE_JS: &str = r#"function() {
  var seen = [];
  var replacer = function(key, value) {
    if (typeof value === 'bigint') return value.toString() + 'n';
    if (typeof value === 'function') return '[Function ' + (value.name || 'anonymous') + ']';
    if (typeof value === 'symbol') return value.toString();
    if (value && typeof value === 'object') {
      if (typeof Node !== 'undefined' && value instanceof Node) {
        return value.nodeType === 1 ? '<' + value.tagName.toLowerCase() + '>' : '[' + value.nodeName + ']';
      }
      if (value instanceof Map) return Object.fromEntries(value);
      if (value instanceof Set) return Array.from(value);
      if (value instanceof Error) return { name: value.name, message: value.message, stack: value.stack };
      if (seen.indexOf(value) !== -1) return '[Circular]';
      seen.push(value);
    }
    return value;
  };
  return JSON.parse(JSON.stringify(this, replacer));
}"#;

fn eval_js_awaited(tab: &Tab, expression: &str) -> Result<serde_json::Value, String> {
    let result = tab
        .call_method(Runtime::Evaluate {
            expression: expression.to_string(),
            return_by_value: Some(true),
            generate_preview: None,
            silent: Some(false),
            await_promise: Some(true),
            include_command_line_api: Some(false),
            user_gesture: Some(true),
            object_group: None,
            context_id: None,
            throw_on_side_effect: None,
            timeout: None,
            disable_breaks: None,
            repl_mode: None,
            allow_unsafe_eval_blocked_by_csp: None,
            unique_context_id: None,
            serialization_options: None,
        })
        .map_err(|error| format!("JS evaluation failed: {error}"))?;
    if let Some(exception) = result.exception_details {
        return Err(exception
            .exception
            .as_ref()
            .and_then(|value| value.description.clone())
            .unwrap_or(exception.text));
    }
    Ok(result.result.value.unwrap_or(serde_json::Value::Null))
}

const PAGE_CONTENT_JS: &str = r#"(function() {
  var out = '';
  if (document.doctype) out = new XMLSerializer().serializeToString(document.doctype);
  if (document.documentElement) out += document.documentElement.outerHTML;
  return out;
})()"#;

fn step_set_content(
    tab: &Tab,
    world: &WorldManager,
    network_monitor: &NetworkMonitorHandle,
    idx: usize,
    html: &str,
    wait_until: Option<BrowserLoadState>,
) -> StepResult {
    let settings = browser_settings::current();
    let frame_id = match tab.call_method(Page::GetFrameTree(None)) {
        Ok(response) => response.frame_tree.frame.id,
        Err(error) => {
            return StepResult::failure(
                idx,
                "Set content",
                format!("Failed to read browser frame tree: {error}"),
            );
        }
    };
    if let Err(error) = tab.call_method(Page::SetDocumentContent {
        frame_id,
        html: html.to_string(),
    }) {
        return StepResult::failure(
            idx,
            "Set content",
            format!("Failed to set document content: {error}"),
        );
    }
    let _ = world.release_all(tab);
    let state = wait_until.unwrap_or(BrowserLoadState::Load);
    let wait = wait_for_load_state(
        network_monitor,
        idx,
        state,
        settings.timing.default_wait_timeout_ms,
    );
    let summary = format!("Set page content ({} bytes)", html.len());
    if wait.ok {
        StepResult::success(idx, summary)
    } else {
        StepResult::failure(
            idx,
            summary,
            format!(
                "Content was set but {} failed: {}",
                wait.summary,
                wait.error.as_deref().unwrap_or(&wait.summary)
            ),
        )
    }
}

fn step_page_content(
    tab: &Tab,
    idx: usize,
    store: &refact_browser::artifact_store::ArtifactStore,
) -> StepResult {
    let html = match eval_js_value(tab, PAGE_CONTENT_JS) {
        Ok(value) => value.as_str().unwrap_or_default().to_string(),
        Err(error) => return StepResult::failure(idx, "Page content", error),
    };
    let bytes = html.len();
    if bytes <= http_client::HTTP_INLINE_BODY_LIMIT_BYTES {
        return StepResult::success(idx, format!("Read page content ({bytes} bytes)"))
            .with_data(serde_json::json!({"html": html, "bytes": bytes}));
    }
    match save_page_content_artifact(store, &html) {
        Ok(path) => StepResult::success(
            idx,
            format!(
                "Read page content ({bytes} bytes) saved to {}",
                path.display()
            ),
        )
        .with_data(serde_json::json!({
            "bytes": bytes,
            "artifact": {
                "kind": "page_content",
                "mime": "text/html",
                "path": path,
                "bytes": bytes,
            }
        })),
        Err(error) => StepResult::failure(idx, "Page content", error),
    }
}

fn save_page_content_artifact(
    store: &refact_browser::artifact_store::ArtifactStore,
    html: &str,
) -> Result<PathBuf, String> {
    store
        .write(html.as_bytes(), "text/html")
        .map(|artifact| artifact.path)
}

fn tag_source(url: Option<&str>, content: Option<&str>, action: &str) -> Result<(), String> {
    match (url, content) {
        (Some(_), Some(_)) => Err(format!("{action} accepts either url or content, not both")),
        (None, None) => Err(format!("{action} requires either url or content")),
        _ => Ok(()),
    }
}

fn add_script_tag_js(
    url: Option<&str>,
    content: Option<&str>,
    script_type: Option<&str>,
) -> String {
    let type_js = match script_type {
        Some(value) => format!("element.type = {};", js_string_literal(value)),
        None => String::new(),
    };
    let body = match url {
        Some(url) => format!(
            r#"element.src = {url};
    element.addEventListener('load', function() {{ resolve(true); }});
    element.addEventListener('error', function() {{ reject(new Error('Failed to load script ' + {url})); }});
    (document.head || document.documentElement).appendChild(element);"#,
            url = js_string_literal(url),
        ),
        None => format!(
            r#"element.text = {content};
    (document.head || document.documentElement).appendChild(element);
    resolve(true);"#,
            content = js_string_literal(content.unwrap_or_default()),
        ),
    };
    format!(
        r#"(function() {{
  return new Promise(function(resolve, reject) {{
    var element = document.createElement('script');
    {type_js}
    {body}
  }});
}})()"#
    )
}

fn add_style_tag_js(url: Option<&str>, content: Option<&str>) -> String {
    let body = match url {
        Some(url) => format!(
            r#"var element = document.createElement('link');
    element.rel = 'stylesheet';
    element.href = {url};
    element.addEventListener('load', function() {{ resolve(true); }});
    element.addEventListener('error', function() {{ reject(new Error('Failed to load stylesheet ' + {url})); }});
    (document.head || document.documentElement).appendChild(element);"#,
            url = js_string_literal(url),
        ),
        None => format!(
            r#"var element = document.createElement('style');
    element.textContent = {content};
    (document.head || document.documentElement).appendChild(element);
    resolve(true);"#,
            content = js_string_literal(content.unwrap_or_default()),
        ),
    };
    format!(
        r#"(function() {{
  return new Promise(function(resolve, reject) {{
    {body}
  }});
}})()"#
    )
}

fn step_add_script_tag(
    tab: &Tab,
    idx: usize,
    url: Option<&str>,
    content: Option<&str>,
    script_type: Option<&str>,
) -> StepResult {
    if let Err(error) = tag_source(url, content, "add_script_tag") {
        return StepResult::failure(idx, "Add script tag", error);
    }
    match eval_js_awaited(tab, &add_script_tag_js(url, content, script_type)) {
        Ok(_) => StepResult::success(
            idx,
            match url {
                Some(url) => format!("Added script tag {url}"),
                None => "Added inline script tag".to_string(),
            },
        ),
        Err(error) => StepResult::failure(idx, "Add script tag", error),
    }
}

fn step_add_style_tag(
    tab: &Tab,
    idx: usize,
    url: Option<&str>,
    content: Option<&str>,
) -> StepResult {
    if let Err(error) = tag_source(url, content, "add_style_tag") {
        return StepResult::failure(idx, "Add style tag", error);
    }
    match eval_js_awaited(tab, &add_style_tag_js(url, content)) {
        Ok(_) => StepResult::success(
            idx,
            match url {
                Some(url) => format!("Added style tag {url}"),
                None => "Added inline style tag".to_string(),
            },
        ),
        Err(error) => StepResult::failure(idx, "Add style tag", error),
    }
}

fn event_constructor(event_type: &str, event_init: Option<&Value>) -> &'static str {
    match event_type {
        "auxclick" | "click" | "dblclick" | "mousedown" | "mouseenter" | "mouseleave"
        | "mousemove" | "mouseout" | "mouseover" | "mouseup" | "mousewheel" => "MouseEvent",
        "keydown" | "keyup" | "keypress" | "textInput" => "KeyboardEvent",
        "touchstart" | "touchmove" | "touchend" | "touchcancel" => "TouchEvent",
        "pointerover" | "pointerout" | "pointerenter" | "pointerleave" | "pointerdown"
        | "pointerup" | "pointermove" | "pointercancel" | "gotpointercapture"
        | "lostpointercapture" => "PointerEvent",
        "focus" | "blur" => "FocusEvent",
        "drag" | "dragstart" | "dragend" | "dragover" | "dragenter" | "dragleave" | "dragexit"
        | "drop" => "DragEvent",
        "wheel" => "WheelEvent",
        "deviceorientation" | "deviceorientationabsolute" => "DeviceOrientationEvent",
        "devicemotion" => "DeviceMotionEvent",
        _ => match event_init.and_then(|init| init.get("detail")) {
            Some(_) => "CustomEvent",
            None => "Event",
        },
    }
}

fn dispatch_event_js(event_type: &str, event_init: Option<&Value>) -> String {
    let constructor = event_constructor(event_type, event_init);
    let init = event_init.cloned().unwrap_or_else(|| serde_json::json!({}));
    format!(
        r#"function() {{
  var init = Object.assign({{bubbles: true, cancelable: true, composed: true}}, {init});
  this.dispatchEvent(new {constructor}({event_type}, init));
  return JSON.stringify({{dispatched: true}});
}}"#,
        event_type = js_string_literal(event_type),
    )
}

fn step_dispatch_event(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    event_type: &str,
    event_init: Option<&Value>,
) -> StepResult {
    let info = match resolve_scoped_element(tab, world, locator) {
        Ok(info) => info,
        Err(error) => return StepResult::failure(idx, "Dispatch event: resolution failed", error),
    };
    let js = dispatch_event_js(event_type, event_init);
    match call_handle_json(tab, world, &info.handle, &js) {
        Ok(_) => StepResult::success(idx, format!("Dispatched {event_type} on <{}>", info.tag))
            .with_data(serde_json::json!({
                "event_type": event_type,
                "constructor": event_constructor(event_type, event_init),
            })),
        Err(error) => StepResult::failure(idx, format!("Dispatch event '{event_type}'"), error),
    }
}

fn step_styles(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    property_filter: Option<&str>,
) -> StepResult {
    match resolve_scoped_element(tab, world, locator) {
        Ok(info) => {
            let filter_js = match property_filter {
                Some(f) if !f.is_empty() => format!(
                    ".filter(function(s) {{ return s.includes({}); }})",
                    js_string_literal(f),
                ),
                _ => String::new(),
            };
            let js = format!(
                r#"function() {{
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
}}"#,
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

const TAB_LOG_MAX_ENTRIES: usize = 50;

fn tab_log_from_buffer(idx: usize, console: &[ConsoleEntry]) -> StepResult {
    let start = console.len().saturating_sub(TAB_LOG_MAX_ENTRIES);
    let entries = &console[start..];
    let summary = if entries.is_empty() {
        "Tab log: no console messages captured yet in this session".to_string()
    } else {
        format!(
            "Tab log: {} most recent console message(s) (session captured {})",
            entries.len(),
            console.len()
        )
    };
    StepResult::success(idx, summary).with_data(serde_json::json!({
        "entries": entries,
        "total_captured": console.len(),
        "truncated": start > 0,
    }))
}

fn step_tab_log(idx: usize) -> StepResult {
    StepResult::failure(
        idx,
        "Tab log",
        "tab_log needs the session console buffer, which is only available in a top-level batch; run it as a normal step rather than inside a locator handler",
    )
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

fn dismiss_overlays(tab: &Tab, world: &WorldManager, aggressive: bool) -> Result<String, String> {
    match eval_js_ok_in_utility(
        tab,
        world,
        &browser_locators::js_dismiss_overlays(false, aggressive),
    ) {
        Ok(result) => {
            let count = result.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
            Ok(format!("Dismissed {count} overlay(s)"))
        }
        Err(error) => Err(error),
    }
}

fn step_dismiss_overlays(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    handlers: Option<&Arc<Mutex<LocatorHandlerRegistry>>>,
    firings: &mut Vec<LocatorHandlerFiring>,
    image_policy: &ImagePolicy,
    aggressive: bool,
) -> StepResult {
    let settings = browser_settings::current();
    let Some(handlers) = handlers else {
        return match dismiss_overlays(tab, world, aggressive) {
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
            );
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
                );
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
        world,
        handlers,
        firings,
        image_policy,
        Instant::now() + Duration::from_millis(settings.timing.default_wait_timeout_ms),
        &lease,
        aggressive,
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
            );
        }
    }
    match result {
        Ok(outcome) => StepResult::success(idx, outcome),
        Err(error) => StepResult::failure(idx, "Dismiss overlays failed", error),
    }
}

fn step_highlight(
    tab: &Tab,
    world: &WorldManager,
    idx: usize,
    locator: &BrowserLocator,
    style: Option<&str>,
    label: Option<&str>,
    legacy: bool,
) -> StepResult {
    let info = match resolve_scoped_element(tab, world, locator) {
        Ok(info) => info,
        Err(error) => return StepResult::failure(idx, "Highlight: resolution failed", error),
    };
    if legacy {
        return match call_handle_json(
            tab,
            world,
            &info.handle,
            browser_locators::js_highlight_element(),
        ) {
            Ok(_) => StepResult::success(idx, format!("Highlighted <{}>", info.tag)),
            Err(error) => StepResult::failure(idx, "Highlight failed", error),
        };
    }
    let function = format!(
        r#"function() {{
  const el = this;
  if (!el) return JSON.stringify({{error:'No resolved element'}});
  window.__refactHideHighlights?.();
  const root = document.createElement('div');
  root.dataset.refactHighlight = 'true';
  const shadow = root.attachShadow({{mode:'closed'}});
  const rect = el.getBoundingClientRect();
  const frame = document.createElement('div');
  const base = {{position:'fixed',left:rect.x+'px',top:rect.y+'px',width:rect.width+'px',height:rect.height+'px',boxSizing:'border-box',outline:'3px solid #E7150D',outlineOffset:'2px',pointerEvents:'none',zIndex:'2147483647'}};
  Object.assign(frame.style, base);
  frame.style.cssText += {};
  shadow.appendChild(frame);
  const label = {};
  if (label) {{
    const tag = document.createElement('div');
    tag.textContent = label;
    Object.assign(tag.style, {{position:'fixed',left:rect.x+'px',top:Math.max(0,rect.y-24)+'px',padding:'3px 6px',font:'12px sans-serif',color:'white',background:'#E7150D',borderRadius:'3px',pointerEvents:'none',zIndex:'2147483647'}});
    shadow.appendChild(tag);
  }}
  document.documentElement.appendChild(root);
  window.__refactHideHighlights = () => {{ root.remove(); delete window.__refactHideHighlights; }};
  return JSON.stringify({{ok:true}});
}}"#,
        js_string_literal(style.unwrap_or("")),
        js_string_literal(label.unwrap_or("")),
    );
    match call_handle_json(tab, world, &info.handle, &function) {
        Ok(_) => StepResult::success(idx, format!("Highlighted <{}>", info.tag)),
        Err(error) => StepResult::failure(idx, "Highlight failed", error),
    }
}

fn step_hide_highlight(tab: &Tab, world: &WorldManager, idx: usize) -> StepResult {
    match world.eval_in_utility(tab, "window.__refactHideHighlights?.(); true") {
        Ok(_) => StepResult::success(idx, "Hidden browser highlight annotations"),
        Err(error) => StepResult::failure(idx, "Hide highlight failed", error),
    }
}

pub fn describe_locator(locator: &BrowserLocator) -> String {
    let mut description = match &locator.strategy {
        LocatorStrategy::Ref {
            value,
        } => format!("ref={}", value),
        LocatorStrategy::Css {
            value,
        } => format!("css={}", value),
        LocatorStrategy::Id {
            value,
        } => format!("id={}", value),
        LocatorStrategy::Name {
            value,
        } => format!("name={}", value),
        LocatorStrategy::TestId {
            value, ..
        } => format!("testid={}", value),
        LocatorStrategy::Placeholder {
            value, ..
        } => format!("placeholder={}", value),
        LocatorStrategy::AltText {
            value, ..
        } => format!("alt_text={}", value),
        LocatorStrategy::Title {
            value, ..
        } => format!("title={}", value),
        LocatorStrategy::Autocomplete {
            value,
        } => format!("autocomplete={}", value),
        LocatorStrategy::Text {
            value,
            exact,
            ..
        } => {
            if *exact {
                format!("text=\"{}\"", value)
            } else {
                format!("text~\"{}\"", value)
            }
        }
        LocatorStrategy::Label {
            value, ..
        } => format!("label={}", value),
        LocatorStrategy::Role {
            role,
            name,
            ..
        } => match name {
            Some(n) => format!("role={}[{}]", role, n),
            None => format!("role={}", role),
        },
        LocatorStrategy::Xpath {
            value,
        } => format!("xpath={}", value),
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
    use std::collections::BTreeMap;

    const DEAD_TRANSPORT: &str =
        "Unable to make method calls because underlying connection is closed";

    #[derive(Clone, Default)]
    struct RecordingMouse {
        events: Arc<Mutex<Vec<(refact_browser::MouseEventType, Option<MouseButton>)>>>,
    }

    impl MouseDispatcher for RecordingMouse {
        fn dispatch(&mut self, event: refact_browser::MouseDispatch) -> Result<(), String> {
            if let refact_browser::MouseDispatch::Mouse(payload) = event {
                self.events
                    .lock()
                    .unwrap()
                    .push((payload.event_type, payload.button));
            }
            Ok(())
        }
    }

    struct SilentKeyboard;

    impl KeyboardDispatcher for SilentKeyboard {
        fn dispatch(&mut self, _event: refact_browser::KeyboardDispatch) -> Result<(), String> {
            Ok(())
        }
    }

    fn button_events(
        recorded: &RecordingMouse,
    ) -> Vec<(refact_browser::MouseEventType, Option<MouseButton>)> {
        recorded
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|(event_type, _)| {
                matches!(
                    event_type,
                    refact_browser::MouseEventType::Pressed
                        | refact_browser::MouseEventType::Released
                )
            })
            .cloned()
            .collect()
    }

    fn drive_states<F>(
        recorded: &RecordingMouse,
        states: &[BrowserElementState],
        observe: F,
    ) -> Result<Vec<(BrowserElementState, ())>, String>
    where
        F: FnMut(BrowserElementState) -> Result<(), String>,
    {
        let keyboard = Keyboard::new(SilentKeyboard);
        let mut mouse = Mouse::new(recorded.clone(), &keyboard);
        drive_element_state_actions(
            &mut mouse,
            element_state_sequence(states),
            || {
                Ok(MainFrameCssPoint {
                    x: 5.0,
                    y: 5.0,
                })
            },
            |_| Ok(()),
            observe,
        )
    }

    #[test]
    fn a_failed_capture_while_pressed_still_releases_the_mouse_button() {
        let recorded = RecordingMouse::default();
        let outcome = drive_states(
            &recorded,
            &[BrowserElementState::Active, BrowserElementState::Default],
            |state| match state {
                BrowserElementState::Active => Err("screenshot failed".to_string()),
                _ => Ok(()),
            },
        );

        assert_eq!(outcome.unwrap_err(), "screenshot failed");
        let events = button_events(&recorded);
        assert_eq!(
            events,
            vec![
                (
                    refact_browser::MouseEventType::Pressed,
                    Some(MouseButton::Left)
                ),
                (
                    refact_browser::MouseEventType::Released,
                    Some(MouseButton::Left)
                ),
            ],
            "a held button must never outlive the failing sequence"
        );
    }

    fn report_with_step_error(error: Option<&str>) -> ExecutionReport {
        let step = match error {
            Some(error) => StepResult::failure(0, "OpenTab", error.to_string()),
            None => StepResult::success(0, "OpenTab"),
        };
        report_with_steps(vec![step])
    }

    fn report_with_steps(steps: Vec<StepResult>) -> ExecutionReport {
        ExecutionReport {
            session_generation: None,
            recovery: None,
            ok: steps.iter().all(|step| step.ok),
            steps,
            warnings: Vec::new(),
            url: None,
            title: None,
            page: None,
            stabilized: false,
            console: vec![],
            page_errors: vec![],
            network: vec![],
            network_summary: vec![],
            websockets: vec![],
            locator_handlers: vec![],
            dialogs: vec![],
            uploads: vec![],
            downloads: vec![],
            new_tabs: vec![],
            active_routes: vec![],
            intercepted_requests: vec![],
            context: None,
            screenshot: None,
        }
    }

    #[test]
    fn popup_wait_arming_clamps_the_timeout_and_remembers_the_armed_step() {
        let settings = browser_settings::current();
        let armed = PendingPopupWait::arm(
            4,
            2,
            Some(settings.timing.max_wait_timeout_ms + 1_000),
            std::collections::BTreeSet::new(),
        );
        assert_eq!(armed.step_index, 4);
        assert_eq!(armed.result_index, 2);
        assert_eq!(armed.timeout_ms, settings.timing.max_wait_timeout_ms);

        let defaulted = PendingPopupWait::arm(0, 0, None, std::collections::BTreeSet::new());
        assert_eq!(
            defaulted.timeout_ms,
            settings.timing.default_wait_timeout_ms
        );
    }

    #[test]
    fn dead_transport_is_detected_from_step_errors_and_dispatch_errors() {
        assert!(report_hit_dead_transport(&Ok(report_with_step_error(
            Some(DEAD_TRANSPORT)
        ))));
        assert!(report_hit_dead_transport(&Err(
            "MethodCallError(ConnectionClosed)".to_string()
        )));

        assert!(!report_hit_dead_transport(&Ok(report_with_step_error(
            Some("Timed out after 5000ms")
        ))));
        assert!(!report_hit_dead_transport(&Ok(report_with_step_error(
            None
        ))));
        assert!(!report_hit_dead_transport(&Err(
            "No tab found with id=abc".to_string()
        )));
    }

    fn navigate_step(url: &str) -> BrowserStep {
        BrowserStep::Navigate {
            url: url.to_string(),
            timeout_ms: None,
        }
    }

    fn screenshot_step() -> BrowserStep {
        serde_json::from_value(serde_json::json!({"action": "screenshot"})).unwrap()
    }

    fn console_entry(level: &str, text: &str) -> refact_integrations::browser_types::ConsoleEntry {
        refact_integrations::browser_types::ConsoleEntry {
            timestamp: 1.0,
            level: level.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn matched_console_entries_are_redacted_before_they_are_reported() {
        let entry = mask_console_entry(console_entry("Error", "token=super-secret-value failed"));

        assert!(!entry.text.contains("super-secret-value"), "{}", entry.text);
    }

    #[test]
    fn the_default_page_context_attaches_a_snapshot_and_no_screenshot() {
        assert_eq!(PageContextMode::default(), PageContextMode::Snapshot);
        assert!(report_snapshot_requested(PageContextMode::default(), true));
        assert!(!report_screenshot_requested(
            None,
            PageContextMode::default(),
            false
        ));
    }

    #[test]
    fn attach_screenshot_stays_the_authoritative_override_over_page_context() {
        for mode in [
            PageContextMode::Snapshot,
            PageContextMode::Screenshot,
            PageContextMode::Both,
            PageContextMode::None,
        ] {
            assert!(report_screenshot_requested(Some(true), mode, false));
            assert!(!report_screenshot_requested(Some(false), mode, true));
        }
    }

    #[test]
    fn an_oversize_snapshot_spills_to_an_artifact_and_inlines_only_a_head() {
        let settings = browser_settings::current();
        let directory = tempfile::tempdir().unwrap();
        let yaml = (1..=400)
            .map(|index| format!("- button \"Item {index}\" [ref=e{index}]"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(yaml.len() > settings.capture.max_inline_snapshot_bytes);

        let store = refact_browser::artifact_store::ArtifactStore::new(directory.path()).unwrap();
        let snapshot = build_page_snapshot(yaml.clone(), &store).unwrap();
        let artifact = snapshot.artifact.as_ref().unwrap();

        assert!(snapshot.truncated);
        assert_eq!(snapshot.bytes, yaml.len());
        assert_eq!(snapshot.lines, 400);
        assert_eq!(snapshot.yaml.lines().count(), SNAPSHOT_SUMMARY_LINES);
        assert!(snapshot.yaml.len() < yaml.len());
        assert!(yaml.starts_with(&snapshot.yaml));
        assert_eq!(artifact.kind, "aria_snapshot");
        assert_eq!(artifact.mime, "text/yaml");
        assert_eq!(artifact.bytes, yaml.len());
        assert_eq!(std::fs::read_to_string(&artifact.path).unwrap(), yaml);
        assert!(artifact.path.starts_with(directory.path()));
    }

    #[test]
    fn all_texts_limit_defaults_and_clamps_to_the_extraction_cap() {
        let settings = browser_settings::current();
        assert_eq!(all_texts_limit(None), settings.capture.default_all_texts);
        assert_eq!(all_texts_limit(Some(3)), 3);
        assert_eq!(all_texts_limit(Some(0)), 0);
        assert_eq!(all_texts_limit(Some(10_000)), MAX_ALL_TEXTS);
    }

    #[test]
    fn extract_table_limit_above_the_extraction_cap_is_clamped() {
        let settings = browser_settings::current();
        let rows = (0..settings.capture.max_extract_table_rows + 20)
            .map(|index| serde_json::json!([index.to_string()]))
            .collect::<Vec<_>>();
        let extracted = serde_json::json!({"ok": true, "rows": rows, "total_rows": 1_000});

        let truncated = truncate_table_rows(extracted, Some(1_000));

        assert_eq!(
            truncated["rows"].as_array().unwrap().len(),
            settings.capture.max_extract_table_rows
        );
        assert_eq!(truncated["total_rows"], serde_json::json!(1_000));
    }

    #[test]
    fn triggered_navigation_accepts_load_buffered_before_commit() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(NavigationEvent::Load {
                frame_id: "main".to_string(),
                loader_id: "new-loader".to_string(),
            })
            .unwrap();
        sender
            .send(NavigationEvent::FrameNavigated {
                frame_id: "main".to_string(),
                loader_id: "new-loader".to_string(),
                url: "https://example.test/next".to_string(),
                restored_from_back_forward_cache: false,
            })
            .unwrap();

        assert_eq!(
            wait_for_navigation_event(
                &receiver,
                &NavigationWaitTarget::Triggered {
                    frame_id: "main".to_string(),
                    previous_loader_id: "old-loader".to_string(),
                    expected_url: Some("https://example.test/next".to_string()),
                    allow_same_document: true,
                },
                Duration::ZERO,
            )
            .unwrap(),
            NavigationWaitOutcome::Completed
        );
    }

    #[test]
    fn triggered_navigation_ignores_unrelated_frame_and_url() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(NavigationEvent::SameDocument {
                frame_id: "child".to_string(),
                url: "https://example.test/next".to_string(),
            })
            .unwrap();
        sender
            .send(NavigationEvent::SameDocument {
                frame_id: "main".to_string(),
                url: "https://example.test/unrelated".to_string(),
            })
            .unwrap();

        let outcome = wait_for_navigation_event(
            &receiver,
            &NavigationWaitTarget::Triggered {
                frame_id: "main".to_string(),
                previous_loader_id: "loader".to_string(),
                expected_url: Some("https://example.test/next".to_string()),
                allow_same_document: true,
            },
            Duration::ZERO,
        )
        .unwrap();

        let NavigationWaitOutcome::TimedOut {
            committed,
            expected_url,
            message,
        } = outcome
        else {
            panic!("expected a navigation timeout outcome");
        };
        assert!(!committed);
        assert_eq!(expected_url.as_deref(), Some("https://example.test/next"));
        assert!(message.contains("Page.frameNavigated or Page.navigatedWithinDocument"));
    }

    #[test]
    fn reload_ignores_same_document_events() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(NavigationEvent::SameDocument {
                frame_id: "main".to_string(),
                url: "https://example.test/page#noise".to_string(),
            })
            .unwrap();

        let outcome = wait_for_navigation_event(
            &receiver,
            &NavigationWaitTarget::Triggered {
                frame_id: "main".to_string(),
                previous_loader_id: "loader".to_string(),
                expected_url: None,
                allow_same_document: false,
            },
            Duration::ZERO,
        )
        .unwrap();

        let NavigationWaitOutcome::TimedOut {
            committed,
            message,
            ..
        } = outcome
        else {
            panic!("expected a navigation timeout outcome");
        };
        assert!(!committed);
        assert!(message.contains("Page.frameNavigated or Page.navigatedWithinDocument"));
    }

    fn http_response(status: u16, body: Vec<u8>, content_type: &str) -> http_client::HttpResponse {
        http_client::HttpResponse {
            status,
            status_text: "OK".to_string(),
            final_url: "https://api.example.test/v1/session".to_string(),
            method: "GET".to_string(),
            redirects: 1,
            headers: BTreeMap::from([
                ("content-type".to_string(), content_type.to_string()),
                ("set-cookie".to_string(), "sid=supersecret".to_string()),
                ("server".to_string(), "fixture".to_string()),
            ]),
            set_cookies: vec![BrowserCookie {
                name: "sid".to_string(),
                value: "supersecret".to_string(),
                domain: "api.example.test".to_string(),
                path: "/".to_string(),
                expires: None,
                http_only: true,
                secure: true,
                same_site: None,
                url: None,
            }],
            rejected_cookies: Vec::new(),
            body,
        }
    }

    #[test]
    fn http_request_result_inlines_small_bodies_and_reports_cookies_without_their_values() {
        let dir = tempfile::tempdir().unwrap();
        let response = http_response(200, br#"{"session":"live"}"#.to_vec(), "application/json");
        let options = BrowserHttpRequest {
            url: "https://api.example.test/v1/session".to_string(),
            ..Default::default()
        };

        let result = http_request_result(
            &response,
            &options,
            3,
            &refact_browser::artifact_store::ArtifactStore::new(dir.path()).unwrap(),
        );

        assert!(result.ok);
        assert_eq!(
            result.summary,
            "GET https://api.example.test/v1/session -> 200 OK (18 bytes, 1 cookies set)"
        );
        let data = &result.data.as_ref().unwrap()["http_request"];
        assert_eq!(data["status"], 200);
        assert_eq!(data["redirects"], 1);
        assert_eq!(data["body"], "{\n  \"session\": \"live\"\n}");
        assert_eq!(data["body_bytes"], 18);
        assert_eq!(data["set_cookies"]["count"], 1);
        assert_eq!(data["set_cookies"]["names"][0], "sid");
        assert!(data.get("artifact").is_none());

        let payload = serde_json::to_string(data).unwrap();
        assert!(!payload.contains("supersecret"), "{payload}");
        assert!(!payload.contains("server"), "{payload}");
        assert_eq!(data["headers"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn http_request_result_spills_oversized_bodies_and_keeps_full_headers_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let body = vec![b'x'; http_client::HTTP_INLINE_BODY_LIMIT_BYTES + 1];
        let response = http_response(200, body.clone(), "text/plain");
        let store = refact_browser::artifact_store::ArtifactStore::new(dir.path()).unwrap();
        let options = BrowserHttpRequest {
            url: "https://api.example.test/v1/session".to_string(),
            full_headers: Some(true),
            ..Default::default()
        };

        let result = http_request_result(&response, &options, 0, &store);

        assert!(result.ok);
        let data = &result.data.as_ref().unwrap()["http_request"];
        assert!(data.get("body").is_none());
        assert_eq!(data["artifact"]["bytes"], body.len());
        let path = PathBuf::from(data["artifact"]["path"].as_str().unwrap());
        assert!(path.starts_with(dir.path()));
        assert_eq!(std::fs::read(&path).unwrap(), body);
        assert_eq!(data["headers"]["set-cookie"], "[REDACTED]");
        assert_eq!(data["headers"]["server"], "fixture");
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

    fn populated_route_registry() -> refact_browser::RouteRegistry {
        let registry = refact_browser::RouteRegistry::default();
        registry
            .add(
                UrlPattern::Text("**/api/**".to_string()),
                RouteHandler::Abort {
                    reason: "blockedbyclient".to_string(),
                },
                None,
            )
            .unwrap();
        registry
            .add(
                UrlPattern::Text("**/assets/**".to_string()),
                RouteHandler::Abort {
                    reason: "failed".to_string(),
                },
                None,
            )
            .unwrap();
        registry
    }

    fn har_replay_route() -> RouteInfo {
        RouteInfo {
            pattern: UrlPattern::Text("har-replay".to_string()),
            handler: RouteHandler::Abort {
                reason: "blockedbyclient".to_string(),
            },
            har: Some(HarRouteInfo {
                entry_count: 12,
                not_found: HarNotFound::Abort,
            }),
            times_remaining: None,
            order: 0,
        }
    }

    fn populated_locator_handlers() -> Mutex<LocatorHandlerRegistry> {
        let mut registry = LocatorHandlerRegistry::default();
        for name in ["cookie_banner", "survey_modal", "paywall"] {
            registry.register(
                LocatorHandler::registered(
                    name.to_string(),
                    BrowserLocator::css("#overlay"),
                    LocatorHandlerAction::Click,
                    None,
                    false,
                )
                .unwrap()
                .unwrap(),
            );
        }
        Mutex::new(registry)
    }

    #[test]
    fn cdp_send_spills_oversized_results_to_an_artifact_and_redacts_cookies() {
        let dir = tempfile::tempdir().unwrap();
        let store = refact_browser::artifact_store::ArtifactStore::new(dir.path()).unwrap();
        let cookies = (0..400)
            .map(|index| serde_json::json!({"name": format!("c{index}"), "value": "secret-value"}))
            .collect::<Vec<_>>();
        let step = cdp_send_result(
            "Network.getCookies",
            CdpTarget::Browser,
            Vec::new(),
            serde_json::json!({"cookies": cookies}),
            0,
            &store,
        );

        assert!(step.ok, "{step:?}");
        assert!(!step.summary.contains("warning"));
        let artifact = &step.data.as_ref().unwrap()["cdp_send"]["artifact"];
        assert_eq!(artifact["mime"], "application/json");
        assert!(
            artifact["bytes"].as_u64().unwrap() > CDP_INLINE_RESULT_LIMIT_BYTES as u64,
            "artifact must only be used past the inline limit"
        );
        assert!(step.data.as_ref().unwrap()["cdp_send"]["result"].is_null());

        let saved = std::fs::read_to_string(artifact["path"].as_str().unwrap()).unwrap();
        assert!(
            !saved.contains("secret-value"),
            "cookie values must be redacted before they reach an artifact"
        );
        assert!(saved.contains("[REDACTED]"));
    }

    #[test]
    fn reset_clears_every_sticky_registry_and_reports_truthful_counts() {
        let route_registry = populated_route_registry();
        let mut routes = route_registry.list();
        routes.push(har_replay_route());
        let websocket_registry = WebSocketRegistry::default();
        websocket_registry
            .add_route(
                UrlPattern::Text("wss://example.com/**".to_string()),
                WebSocketRouteMode::Mock,
                WebSocketMessageAction::Forward,
                WebSocketMessageAction::Forward,
            )
            .unwrap();
        let locator_handlers = populated_locator_handlers();

        let counts = reset_sticky_registries(
            &routes,
            &websocket_registry,
            &locator_handlers,
            1,
            2,
            true,
            true,
        )
        .unwrap();
        route_registry.remove(None);

        assert_eq!(
            counts,
            BrowserResetCounts {
                routes: 2,
                har_replays: 1,
                websocket_routes: 1,
                locator_handlers: 3,
                authenticators: 1,
                init_scripts: 2,
                clock: true,
                service_worker_block: true,
            }
        );
        assert_eq!(
            counts.summary(),
            "Reset: 2 routes, 1 har replay, 1 ws route, 3 locator handlers, 1 authenticator, 2 init scripts, offline off, throttling off, emulation and device cleared, clock cleared, service worker block cleared"
        );
        assert!(route_registry.is_empty());
        assert_eq!(websocket_registry.route_count(), 0);
        assert_eq!(
            locator_handlers
                .lock()
                .unwrap()
                .handlers()
                .iter()
                .map(|handler| handler.name.clone())
                .collect::<Vec<_>>(),
            vec![DEFAULT_DISMISS_OVERLAYS_HANDLER.to_string()]
        );
    }

    #[test]
    fn reset_on_clean_state_succeeds_with_zero_counts() {
        let websocket_registry = WebSocketRegistry::default();
        let locator_handlers = populated_locator_handlers();

        reset_sticky_registries(
            &[],
            &websocket_registry,
            &locator_handlers,
            0,
            0,
            false,
            false,
        )
        .unwrap();
        let repeated = reset_sticky_registries(
            &[],
            &websocket_registry,
            &locator_handlers,
            0,
            0,
            false,
            false,
        )
        .unwrap();

        assert_eq!(repeated, BrowserResetCounts::default());
        assert_eq!(
            repeated.summary(),
            "Reset: 0 routes, 0 har replays, 0 ws routes, 0 locator handlers, 0 authenticators, 0 init scripts, offline off, throttling off, emulation and device cleared, clock off, service worker block off"
        );
        assert_eq!(websocket_registry.route_count(), 0);
        assert_eq!(locator_handlers.lock().unwrap().handlers().len(), 1);
    }

    #[test]
    fn permission_state_tracks_only_the_permissions_that_stay_granted() {
        let granted = apply_permission_state(
            &[],
            &["geolocation".to_string(), "notifications".to_string()],
            BrowserPermissionState::Granted,
        );
        assert_eq!(granted, vec!["geolocation", "notifications"]);

        let denied = apply_permission_state(
            &granted,
            &["notifications".to_string()],
            BrowserPermissionState::Denied,
        );
        assert_eq!(denied, vec!["geolocation"]);

        let prompted = apply_permission_state(
            &denied,
            &["geolocation".to_string()],
            BrowserPermissionState::Prompt,
        );
        assert!(prompted.is_empty());

        let regranted = apply_permission_state(
            &["geolocation".to_string()],
            &["geolocation".to_string()],
            BrowserPermissionState::Granted,
        );
        assert_eq!(regranted, vec!["geolocation"]);
    }

    fn download(state: DownloadState, failure_reason: Option<&str>) -> DownloadInfo {
        DownloadInfo {
            guid: "guid-1".to_string(),
            url: "https://example.test/report.csv".to_string(),
            frame_id: "frame".to_string(),
            suggested_filename: "report.csv".to_string(),
            local_path: "/tmp/report.csv".to_string(),
            received_bytes: 4,
            total_bytes: 9,
            state,
            failure_reason: failure_reason.map(str::to_string),
        }
    }

    #[test]
    fn screencast_steps_are_routed_to_the_runtime_dispatch() {
        for step in [
            BrowserStep::CaptureFrames {
                duration_ms: None,
                frame_count: None,
                interval_ms: None,
                locator: None,
                full_page: None,
            },
            BrowserStep::ScreencastStart {
                quality: None,
                max_width: None,
                max_height: None,
            },
            BrowserStep::ScreencastStop {
                compose: None,
            },
        ] {
            assert!(is_screencast_step(&step));
            assert!(!is_instrumentation_step(&step));
            assert!(!is_context_management_step(&step));
            assert!(!is_tab_management_step(&step));
        }
        assert!(!is_screencast_step(&BrowserStep::StopCoverage));
    }

    #[test]
    fn reported_poll_values_are_redacted_without_reshaping_them() {
        let redacted = redact_poll_value(serde_json::json!({
            "headers": ["authorization: Bearer abcdef123456"],
            "token": "sk-abcdef123456789",
            "count": 3,
            "ready": true,
        }));

        let serialized = serde_json::to_string(&redacted).unwrap();
        assert!(!serialized.contains("abcdef123456"), "{serialized}");
        assert!(!serialized.contains("sk-abcdef"), "{serialized}");
        assert!(serialized.contains("[REDACTED]"), "{serialized}");
        assert_eq!(redacted["count"], serde_json::json!(3));
        assert_eq!(redacted["ready"], serde_json::json!(true));
        assert_eq!(redacted["headers"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn terminal_transport_overrides_each_soft_step_policy() {
        let steps: Vec<BrowserStep> = [
            serde_json::json!({"action":"click_if_exists","locator":{"by":"css","value":"button"}}),
            serde_json::json!({"action":"expect","locator":{"by":"css","value":"button"},"matcher":{"type":"to_be_visible"},"soft":true}),
            serde_json::json!({"action":"expect_poll","expression":"true","matcher":"equals","expected":true,"soft":true}),
        ].into_iter().map(|value| serde_json::from_value(value).unwrap()).collect();
        for step in steps {
            assert!(is_non_fatal_step(&step));
            let mut result = StepResult::failure(4, "interrupted", DEAD_TRANSPORT);
            assert!(step_failure_is_fatal(&step, &mut result));
            assert_eq!(result.outcome.as_deref(), Some("effect_unknown"));
            let mut results = vec![StepResult::success(0, "acknowledged"), result];
            // Preserve the original interrupted index: skipped tail is based on indexes,
            // not the number of accumulated results.
            mark_unexecuted_steps_skipped(&mut results, 6);
            assert!(results.last().unwrap().skipped);
            assert_eq!(results[1].step_index, 4);
            assert!(!transport_recovery(&results).unwrap().retry_safe);
        }
    }

    #[test]
    fn only_soft_poll_assertions_join_the_non_fatal_step_set() {
        let poll = |soft| BrowserStep::ExpectPoll {
            expression: "window.__count".to_string(),
            expected: serde_json::json!(1),
            matcher: BrowserPollMatcher::Equals,
            timeout_ms: None,
            soft,
        };

        assert!(is_non_fatal_step(&poll(Some(true))));
        assert!(!is_non_fatal_step(&poll(Some(false))));
        assert!(!is_non_fatal_step(&poll(None)));
        assert!(!is_non_fatal_step(&BrowserStep::WaitForFunction {
            expression: "() => true".to_string(),
            locator: None,
            timeout_ms: None,
            polling_ms: None,
        }));
    }

    fn css_strategy() -> LocatorStrategy {
        LocatorStrategy::Css {
            value: "#submit".to_string(),
        }
    }

    #[test]
    fn polling_waits_through_a_replaced_iframe_and_reports_it_verbatim() {
        let detached = frame_locator_error_message(
            &BrowserLocator::css(".inner").in_frames(vec![BrowserLocator::css("iframe")]),
            refact_browser::FrameLocatorError::FrameDetached {
                index: 0,
                source: Some("No frame for given id".to_string()),
            },
        );
        let missing = frame_locator_error_message(
            &BrowserLocator::css(".inner").in_frames(vec![BrowserLocator::css("iframe")]),
            refact_browser::FrameLocatorError::FrameOwnerMissing {
                index: 0,
            },
        );

        assert!(is_transient_page_error(&detached));
        assert!(is_transient_page_error(&missing));
        assert_eq!(
            poll_deadline_error(Some(detached.clone()), "Timed out after 5000ms".to_string()),
            detached
        );
        assert_eq!(
            poll_deadline_error(Some(missing.clone()), "Timed out after 5000ms".to_string()),
            missing
        );
    }

    #[test]
    fn click_if_exists_skips_only_genuine_non_actionable_probes() {
        assert_eq!(
            classify_click_if_exists_probe(Ok(())),
            ClickIfExistsProbe::Proceed
        );
        assert_eq!(
            classify_click_if_exists_probe(Err("Element not found".to_string())),
            ClickIfExistsProbe::Skip("Element not found".to_string())
        );
        assert_eq!(
            classify_click_if_exists_probe(Err(
                "not actionable, element is not visible".to_string()
            )),
            ClickIfExistsProbe::Skip("not actionable, element is not visible".to_string())
        );
        assert_eq!(
            classify_click_if_exists_probe(Err(DEAD_TRANSPORT.to_string())),
            ClickIfExistsProbe::Fatal(DEAD_TRANSPORT.to_string())
        );
    }

    #[test]
    fn click_if_exists_keeps_transport_dead_action_failures_fatal() {
        let succeeded = click_if_exists_action_result(StepResult::success(3, "click on <button>"));
        assert!(succeeded.ok);
        assert_eq!(succeeded.summary, "click on <button>");

        let non_fatal = click_if_exists_action_result(StepResult::failure(
            3,
            "click failed",
            "Element is not visible".to_string(),
        ));
        assert!(non_fatal.ok);
        assert_eq!(
            non_fatal.summary,
            "Click skipped (non-fatal): Element is not visible"
        );
        assert!(non_fatal.error.is_none());
        assert_eq!(non_fatal.data.as_ref().unwrap()["clicked"], false);
        assert_eq!(
            non_fatal.data.as_ref().unwrap()["skip_reason"],
            "Element is not visible"
        );

        let fatal = click_if_exists_action_result(StepResult::failure(
            3,
            "click failed",
            DEAD_TRANSPORT.to_string(),
        ));
        assert!(!fatal.ok);
        assert_eq!(fatal.error.as_deref(), Some(DEAD_TRANSPORT));
    }

    #[test]
    fn transport_dead_resolve_errors_surface_for_every_locator_strategy() {
        assert_eq!(
            locator_outcome_for_resolve_error(&css_strategy(), "Element not found".to_string()),
            LocatorOutcome::NotFound
        );
        assert_eq!(
            locator_outcome_for_resolve_error(&css_strategy(), DEAD_TRANSPORT.to_string()),
            LocatorOutcome::Error {
                description: DEAD_TRANSPORT.to_string()
            }
        );
        assert_eq!(
            locator_outcome_for_resolve_error(
                &LocatorStrategy::Ref {
                    value: "e1".to_string()
                },
                "Invalid browser ref e1".to_string()
            ),
            LocatorOutcome::Error {
                description: "Invalid browser ref e1".to_string()
            }
        );
    }

    #[test]
    fn navigation_load_state_failures_are_reported_on_the_step() {
        let reached = navigation_load_state_step(1, "Navigation detected".to_string(), Ok(()));
        assert!(reached.ok);
        assert_eq!(reached.summary, "Navigation detected");
        assert!(reached.data.is_none());

        let not_reached = navigation_load_state_step(
            1,
            "Navigation detected".to_string(),
            Err("Timed out after 5000ms".to_string()),
        );
        assert!(not_reached.ok);
        assert!(
            not_reached.summary.contains("load state not reached"),
            "unexpected summary: {}",
            not_reached.summary
        );
        assert_eq!(
            not_reached.data,
            Some(serde_json::json!({"load_state_reached": false}))
        );

        let dead = navigation_load_state_step(
            1,
            "Navigation detected".to_string(),
            Err(DEAD_TRANSPORT.to_string()),
        );
        assert!(!dead.ok);
        assert_eq!(dead.error.as_deref(), Some(DEAD_TRANSPORT));
    }
}
