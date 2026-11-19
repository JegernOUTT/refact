//! Human-controlled Browser workspace API over the one shared project browser runtime.
//!
//! The project is always the engine's trusted current project; clients never name a project,
//! chat or profile path. Frames are pulled, never pushed into chats, and no raw browser
//! control endpoint is ever returned.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderValue, Response, StatusCode};
use axum::middleware;
use axum::routing::post;
use axum::Router;
use base64::Engine;
use headless_chrome::protocol::cdp::Page;
use hyper::Body;
use refact_core::image_policy::{resize_to_policy, ImageFormat, ImagePolicy};
use serde::Deserialize;
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::http::routers::v1::browser_settings;
use crate::integrations::browser_runtime::project_browser::{
    current_project_scope, ensure_project_runtime, find_project_runtime, pinned_project_scope,
    ProjectBrowserScope,
};
use crate::integrations::browser_runtime::BrowserRuntime;
use refact_browser::tab_gate::{gate_use_label, GateUse, TabGate, TabGateGuard};
use refact_browser::{
    CdpKeyboardDispatcher, CdpMouseDispatcher, KeyEventPayload, KeyEventType, Keyboard,
    KeyboardDispatch, KeyboardDispatcher, MouseButton, MouseDispatch, MouseDispatcher,
    MouseEventPayload, MouseEventType,
};

const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_TEXT_CHARS: usize = 8192;
const MAX_URL_CHARS: usize = 8192;
const MAX_COORD: f64 = 20_000.0;
const MAX_WHEEL_DELTA: f64 = 10_000.0;
const MIN_PANE_SIDE: u32 = 120;
const MAX_PANE_SIDE: u32 = 4096;
const DEFAULT_FRAME_SIDE: u32 = 1280;
const MIN_FRAME_SIDE: u32 = 160;
const MAX_FRAME_SIDE: u32 = 1920;
const FRAME_JPEG_QUALITY: u8 = 70;
const UI_LOCK_WAIT: Duration = Duration::from_millis(1500);
const UI_COMMAND_DEADLINE: Duration = Duration::from_secs(20);
const RENDERER_DEADLINE: Duration = Duration::from_secs(3);
const FRAME_TITLE_DEADLINE: Duration = Duration::from_millis(150);
const RELEASE_DEADLINE: Duration = Duration::from_secs(1);
const FRAME_BUSY: &str = "the browser did not return a frame in time; try again";
const INPUT_BUSY: &str = "the browser did not confirm the input in time; it may have been applied";
const RESIZE_BUSY: &str = "the browser did not confirm the pane size in time; try again";
const START_DEADLINE: Duration = Duration::from_secs(120);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/browser/project/status", post(handle_status))
        .route("/browser/project/start", post(handle_start))
        .route("/browser/project/tab", post(handle_tab))
        .route("/browser/project/navigate", post(handle_navigate))
        .route("/browser/project/input", post(handle_input))
        .route("/browser/project/resize", post(handle_resize))
        .route("/browser/project/frame", post(handle_frame))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .route_layer(middleware::map_response(no_store))
}

/// Removing the error extension keeps the logging middleware from rebuilding the response,
/// which would drop this header.
async fn no_store(mut response: axum::response::Response) -> axum::response::Response {
    response.extensions_mut().remove::<ScratchError>();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn fail(status: StatusCode, message: impl Into<String>) -> ScratchError {
    ScratchError::new(status, message.into())
}

fn parse_body<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, ScratchError> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(fail(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request body is too large",
        ));
    }
    let bytes = if bytes.iter().all(u8::is_ascii_whitespace) {
        b"{}".as_slice()
    } else {
        bytes
    };
    serde_json::from_slice(bytes).map_err(|error| {
        fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {error}"),
        )
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyBody {}

/// A daemon-managed worker serves exactly the project root it was started for; editor
/// workspace updates must not move `/p/<project>/…` to another project. Direct workers use
/// the root a fresh chat would bind.
async fn scope(app: &AppState) -> Result<ProjectBrowserScope, ScratchError> {
    let resolved = match pinned_project_scope(app) {
        Some(pinned) => pinned,
        None => current_project_scope(app).await,
    };
    resolved.map_err(|error| {
        tracing::warn!("project browser scope unavailable: {error}");
        fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "project root unavailable; open a project first",
        )
    })
}

type RuntimeArc = Arc<AMutex<BrowserRuntime>>;

fn busy() -> ScratchError {
    fail(
        StatusCode::CONFLICT,
        "the project browser is busy with another operation; try again",
    )
}

async fn lock_runtime(
    runtime: &RuntimeArc,
) -> Result<tokio::sync::MutexGuard<'_, BrowserRuntime>, ScratchError> {
    let guard = tokio::time::timeout(UI_LOCK_WAIT, runtime.lock())
        .await
        .map_err(|_| busy())?;
    if !guard.ownership.is_current_generation() {
        return Err(fail(
            StatusCode::CONFLICT,
            "the project browser was replaced; refresh its status",
        ));
    }
    Ok(guard)
}

fn blocking<T>(f: impl FnOnce() -> T) -> T {
    blocking_within(UI_COMMAND_DEADLINE, f)
}

fn blocking_within<T>(budget: Duration, f: impl FnOnce() -> T) -> T {
    let deadline = Instant::now() + budget;
    tokio::task::block_in_place(|| {
        let _guard = headless_chrome::browser::transport::CommandDeadlineGuard::new(deadline);
        f()
    })
}

fn cdp(error: impl std::fmt::Display) -> ScratchError {
    fail(
        StatusCode::BAD_GATEWAY,
        refact_core::string_utils::redact_sensitive(&error.to_string()),
    )
}

// ---------------------------------------------------------------------------------------
// Pane sizing: a pure UI viewport that never writes the tool-owned `context_state`.
// ---------------------------------------------------------------------------------------

fn tool_override_active(runtime: &BrowserRuntime, tab_id: Option<&str>) -> bool {
    runtime.context_state.viewport.is_some()
        || tab_id.is_some_and(|id| runtime.tab_devices.contains_key(id))
}

fn validate_pane(width: u32, height: u32) -> Result<(u32, u32), String> {
    if !(MIN_PANE_SIDE..=MAX_PANE_SIDE).contains(&width)
        || !(MIN_PANE_SIDE..=MAX_PANE_SIDE).contains(&height)
    {
        return Err(format!(
            "pane size must be between {MIN_PANE_SIDE} and {MAX_PANE_SIDE} CSS pixels per side"
        ));
    }
    Ok((width, height))
}

fn apply_pane_viewport(
    tab: &headless_chrome::Tab,
    size: (u32, u32),
) -> Result<(), RendererFailure> {
    tab.call_method(
        headless_chrome::protocol::cdp::Emulation::SetDeviceMetricsOverride {
            width: size.0,
            height: size.1,
            device_scale_factor: 0.0,
            mobile: false,
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
        },
    )
    .map(|_| ())
    .map_err(RendererFailure::from_cdp)
}

/// Pane sync while the runtime lock is held: never waits for the tab's gate, so a busy tab is
/// left to its next frame.
fn sync_pane_now(rt: &mut BrowserRuntime, tab: &headless_chrome::Tab) -> Result<(), ScratchError> {
    let tab_id = tab.get_target_id().to_string();
    let Ok(guard) = rt
        .tab_gate(&tab_id)
        .acquire(GateUse::Resize, Instant::now())
    else {
        return Ok(());
    };
    let Some(size) = pane_to_apply(rt, &guard, &tab_id) else {
        return Ok(());
    };
    let applied = blocking_within(RENDERER_DEADLINE, || apply_pane_viewport(tab, size));
    guard.update_view(|view| view.applied_pane = applied.as_ref().ok().map(|_| size));
    applied.map_err(|failure| renderer_failure_status(failure, &RESIZE_FAILURE))?;
    rt.pane.keep_tab_size(&tab_id, size);
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------------------

pub fn validate_navigate_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_URL_CHARS {
        return Err("url must be 1..=8192 characters".into());
    }
    if trimmed == "about:blank" {
        return Ok(trimmed.to_string());
    }
    let parsed = url::Url::parse(trimmed).map_err(|error| format!("invalid url: {error}"))?;
    match parsed.scheme() {
        "http" | "https" if parsed.host_str().is_some() => Ok(parsed.to_string()),
        _ => Err("only http, https and about:blank urls can be opened from the Browser tab".into()),
    }
}

pub fn modifier_mask(modifiers: &[String]) -> Result<u32, String> {
    if modifiers.len() > 4 {
        return Err("at most four modifiers are accepted".into());
    }
    let mut mask = 0;
    for modifier in modifiers {
        mask |= match modifier.to_ascii_lowercase().as_str() {
            "alt" => 1,
            "ctrl" | "control" => 2,
            "meta" | "cmd" | "command" => 4,
            "shift" => 8,
            other => {
                return Err(format!(
                    "unknown modifier '{}'",
                    other.chars().take(16).collect::<String>()
                ))
            }
        };
    }
    Ok(mask)
}

fn finite_coord(name: &str, value: Option<f64>) -> Result<f64, String> {
    let value = value.ok_or_else(|| format!("{name} is required"))?;
    if !value.is_finite() || !(0.0..=MAX_COORD).contains(&value) {
        return Err(format!("{name} must be a CSS pixel in 0..={MAX_COORD}"));
    }
    Ok(value)
}

fn wheel_delta(name: &str, value: Option<f64>) -> Result<f64, String> {
    let value = value.unwrap_or(0.0);
    if !value.is_finite() || value.abs() > MAX_WHEEL_DELTA {
        return Err(format!("{name} must be within +/-{MAX_WHEEL_DELTA}"));
    }
    Ok(value)
}

#[derive(Debug, PartialEq, Eq)]
pub enum FenceError {
    RuntimeChanged,
    TabChanged,
}

/// Input is accepted only for the runtime the UI is displaying and a tab that still exists;
/// the tab does not have to be the tool's active tab. The fence is checked under the runtime
/// lock and the events are dispatched after it is released, with a short renderer deadline.
pub fn check_fence(
    expected_runtime: &str,
    actual_runtime: &str,
    tab_exists: bool,
) -> Result<(), FenceError> {
    if expected_runtime != actual_runtime {
        return Err(FenceError::RuntimeChanged);
    }
    if !tab_exists {
        return Err(FenceError::TabChanged);
    }
    Ok(())
}

fn fence_error(error: FenceError) -> ScratchError {
    fail(
        match error {
            FenceError::RuntimeChanged => StatusCode::CONFLICT,
            FenceError::TabChanged => StatusCode::GONE,
        },
        match error {
            FenceError::RuntimeChanged => {
                "stale runtime_id: the project browser was replaced; refresh its status"
            }
            FenceError::TabChanged => "stale tab_id: no such tab; refresh before sending input",
        },
    )
}

fn tab_title(tab: &headless_chrome::Tab) -> String {
    tab.get_target_title()
}

fn tab_by_id(runtime: &BrowserRuntime, tab_id: &str) -> Option<Arc<headless_chrome::Tab>> {
    runtime.find_tab(tab_id)
}

fn stale_tab() -> ScratchError {
    fail(StatusCode::GONE, "stale tab_id: no such tab")
}

/// The named tab, or the active tab when no id is given.
fn chosen_tab(
    runtime: &BrowserRuntime,
    tab_id: Option<&str>,
) -> Result<Arc<headless_chrome::Tab>, ScratchError> {
    match tab_id {
        Some(id) => tab_by_id(runtime, id).ok_or_else(stale_tab),
        None => runtime
            .get_active_tab()
            .ok_or_else(|| fail(StatusCode::CONFLICT, "the project browser has no tab")),
    }
}

pub fn tab_status_json(
    tab: &crate::integrations::browser_models::TabInfo,
    meta: Option<&refact_browser::TabMeta>,
) -> serde_json::Value {
    serde_json::json!({
        "tab_id": tab.id,
        "url": tab.url,
        "title": tab.title,
        "active": tab.active,
        "name": meta.and_then(|meta| meta.name.as_deref()),
        "opened_by_chat": meta.and_then(|meta| meta.opened_by_chat.as_deref()),
        "last_chat": meta.and_then(|meta| meta.last_chat.as_deref()),
        "last_used_ms": meta.and_then(|meta| meta.last_used_ms),
    })
}

async fn live_runtime(
    app: &AppState,
    scope: &ProjectBrowserScope,
) -> Result<(String, RuntimeArc), ScratchError> {
    find_project_runtime(app, scope).await.ok_or_else(|| {
        fail(
            StatusCode::NOT_FOUND,
            "the project browser is not running; start it first",
        )
    })
}

// ---------------------------------------------------------------------------------------
// Status / start
// ---------------------------------------------------------------------------------------

type TabEntry = (
    Arc<headless_chrome::Tab>,
    crate::integrations::browser_models::TabInfo,
    Option<refact_browser::TabMeta>,
);

/// Runtime state for a status response, taken under the runtime lock without CDP reads.
struct StatusCapture {
    runtime_id: String,
    tabs: Vec<TabEntry>,
    active: Option<Arc<headless_chrome::Tab>>,
    pane: Option<(u32, u32)>,
    ownership: Arc<refact_browser::RuntimeOwnership>,
    headless: bool,
    tool_viewport_override: bool,
}

/// Copies the runtime's own tab bookkeeping; tabs Chrome opened that are not adopted yet stay
/// out, since adopting them needs CDP and is left to standalone status requests.
fn capture_status(rt: &mut BrowserRuntime, runtime_id: &str) -> StatusCapture {
    rt.reconcile_tabs();
    let tabs = rt
        .live_tabs()
        .into_iter()
        .map(|tab| {
            let info = rt.tab_info(&tab, String::new());
            let meta = rt.tab_meta.get(&info.id).cloned();
            (tab, info, meta)
        })
        .collect();
    let active = rt.get_active_tab();
    let pane = rt
        .pane
        .desired(active.as_ref().map(|tab| tab.get_target_id().as_str()));
    let tool_viewport_override =
        tool_override_active(rt, active.as_ref().map(|tab| tab.get_target_id().as_str()));
    StatusCapture {
        runtime_id: runtime_id.to_string(),
        tabs,
        active,
        pane,
        ownership: rt.ownership.clone(),
        headless: rt.headless(),
        tool_viewport_override,
    }
}

/// Completes a capture with CDP reads made after the runtime lock is released, and reports
/// whether its runtime was still the current generation afterwards.
fn render_status(scope: &ProjectBrowserScope, capture: StatusCapture) -> (serde_json::Value, bool) {
    let StatusCapture {
        runtime_id,
        tabs,
        active,
        pane,
        ownership,
        headless,
        tool_viewport_override,
    } = capture;
    let (tabs, url, title, viewport) = blocking_within(RENDERER_DEADLINE, || {
        let tabs = tabs
            .into_iter()
            .map(|(tab, mut info, meta)| {
                info.title = tab_title(&tab);
                tab_status_json(&info, meta.as_ref())
            })
            .collect::<Vec<_>>();
        let (url, title, viewport) = match &active {
            Some(tab) => (
                tab.get_url(),
                tab_title(&tab),
                tab.call_method(Page::GetLayoutMetrics(None))
                    .ok()
                    .map(|metrics| {
                        serde_json::json!({
                            "width": metrics.css_visual_viewport.client_width,
                            "height": metrics.css_visual_viewport.client_height,
                        })
                    }),
            ),
            None => (String::new(), String::new(), None),
        };
        (tabs, url, title, viewport)
    });
    (
        serde_json::json!({
            "project_id": scope.project_id(),
            "running": true,
            "runtime_id": runtime_id,
            "connected": ownership.is_connected(),
            "headless": headless,
            "active_tab": active.as_ref().map(|tab| tab.get_target_id().to_string()),
            "url": url,
            "title": title,
            "tabs": tabs,
            "viewport": viewport,
            "pane": pane.map(|(w, h)| serde_json::json!({"width": w, "height": h})),
            "tool_viewport_override": tool_viewport_override,
        }),
        ownership.is_current_generation(),
    )
}

/// The project browser's status and whether the runtime it describes was still the current
/// generation once the snapshot was complete.
async fn status_snapshot(
    app: &AppState,
    scope: &ProjectBrowserScope,
) -> Result<(serde_json::Value, bool), ScratchError> {
    let Some((runtime_id, runtime)) = find_project_runtime(app, scope).await else {
        return Ok((
            serde_json::json!({
                "project_id": scope.project_id(),
                "running": false,
                "connected": false,
                "tabs": [],
            }),
            true,
        ));
    };
    let record = crate::integrations::browser_runtime::runtime_owner_record(&runtime_id);
    let Ok(mut rt) = tokio::time::timeout(UI_LOCK_WAIT, runtime.lock()).await else {
        return Ok((
            serde_json::json!({
                "project_id": scope.project_id(),
                "running": true,
                "runtime_id": runtime_id,
                "connected": record.map(|record| record.is_connected()).unwrap_or(false),
                "busy": true,
                "tabs": [],
            }),
            true,
        ));
    };
    blocking(|| {
        refact_browser::adopt_new_tabs(&mut rt, None);
    });
    let capture = capture_status(&mut rt, &runtime_id);
    drop(rt);
    Ok(render_status(scope, capture))
}

async fn status_value(
    app: &AppState,
    scope: &ProjectBrowserScope,
) -> Result<serde_json::Value, ScratchError> {
    match status_snapshot(app, scope).await? {
        (value, true) => Ok(value),
        (_, false) => Err(fail(
            StatusCode::CONFLICT,
            "the project browser was replaced; refresh its status",
        )),
    }
}

/// The status reported after an already-dispatched mutation, from the capture taken in the
/// mutation's own critical section, so a busy runtime never stands in for its result. A
/// capture that raced a runtime replacement is followed by one fresh snapshot.
async fn status_after_mutation(
    app: &AppState,
    scope: &ProjectBrowserScope,
    capture: StatusCapture,
) -> Result<serde_json::Value, ScratchError> {
    match render_status(scope, capture) {
        (value, true) => Ok(value),
        (_, false) => Ok(status_snapshot(app, scope).await?.0),
    }
}

async fn handle_status(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let _: EmptyBody = parse_body(&body)?;
    let scope = scope(&app).await?;
    Ok(json_response(
        StatusCode::OK,
        status_value(&app, &scope).await?,
    ))
}

async fn handle_start(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let _: EmptyBody = parse_body(&body)?;
    let scope = scope(&app).await?;
    let options = browser_settings::current().to_launch_options();
    let ensured = ensure_project_runtime(&app, &scope, options, Instant::now() + START_DEADLINE)
        .await
        .map_err(|error| fail(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let capture = {
        let mut rt = lock_runtime(&ensured.runtime).await?;
        if let Some(tab) = rt.get_active_tab() {
            sync_pane_now(&mut rt, &tab)?;
        }
        capture_status(&mut rt, &ensured.runtime_id)
    };
    let mut value = status_after_mutation(&app, &scope, capture).await?;
    value["launched"] = serde_json::json!(ensured.launched);
    Ok(json_response(StatusCode::OK, value))
}

// ---------------------------------------------------------------------------------------
// Tabs and navigation
// ---------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TabBody {
    op: String,
    #[serde(default)]
    tab_id: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    chat_id: Option<String>,
}

fn navigate_tab(tab: &headless_chrome::Tab, url: &str) -> Result<(), ScratchError> {
    tab.navigate_to(url).map(|_| ()).map_err(cdp)
}

fn checked_url(raw: Option<&str>) -> Result<Option<String>, ScratchError> {
    raw.map(|url| {
        validate_navigate_url(url).map_err(|error| fail(StatusCode::UNPROCESSABLE_ENTITY, error))
    })
    .transpose()
}

fn validate_tab_body(post: &TabBody) -> Result<(), ScratchError> {
    match post.op.as_str() {
        "new" if post.tab_id.is_some() => Err(unprocessable("op new does not take tab_id".into())),
        "new" => match post.chat_id.as_deref() {
            Some(chat_id) => crate::chat::trajectories::validate_trajectory_id(chat_id)
                .map_err(|_| unprocessable("invalid chat_id".into())),
            None => Ok(()),
        },
        "close" if post.url.is_some() || post.chat_id.is_some() => {
            Err(unprocessable("op close takes only tab_id".into()))
        }
        "close" if post.tab_id.is_none() => Err(unprocessable("tab_id is required".into())),
        "close" => Ok(()),
        _ => Err(unprocessable(
            "unknown tab op; expected new or close".into(),
        )),
    }
}

async fn handle_tab(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: TabBody = parse_body(&body)?;
    validate_tab_body(&post)?;
    let url = checked_url(post.url.as_deref())?;
    let scope = scope(&app).await?;
    let (runtime_id, runtime) = live_runtime(&app, &scope).await?;
    let (new_tab_id, capture) = {
        let mut rt = lock_runtime(&runtime).await?;
        rt.touch();
        let new_tab_id = match post.tab_id.as_deref() {
            None => {
                let tab = blocking(|| {
                    crate::integrations::browser_runtime::project_browser::open_project_tab(
                        &mut rt,
                        post.chat_id.as_deref(),
                    )
                })
                .map_err(cdp)?;
                sync_pane_now(&mut rt, &tab)?;
                if let Some(url) = &url {
                    blocking(|| navigate_tab(&tab, url))?;
                }
                Some(tab.get_target_id().to_string())
            }
            Some(tab_id) => {
                let tab = tab_by_id(&rt, tab_id).ok_or_else(stale_tab)?;
                if rt.live_tabs().len() <= 1 {
                    return Err(fail(
                        StatusCode::CONFLICT,
                        "the last tab cannot be closed; navigate it instead",
                    ));
                }
                if !blocking(|| tab.close(false)).map_err(cdp)? {
                    return Err(fail(
                        StatusCode::BAD_GATEWAY,
                        "the browser refused to close the tab; it is still open",
                    ));
                }
                rt.select_tab_after_close(tab_id);
                None
            }
        };
        (new_tab_id, capture_status(&mut rt, &runtime_id))
    };
    let value = match new_tab_id {
        Some(new_tab_id) => new_tab_status(&scope, capture, &new_tab_id)?,
        None => status_after_mutation(&app, &scope, capture).await?,
    };
    Ok(json_response(StatusCode::OK, value))
}

/// The status after `op: new`, from the inventory captured under the lock the tab was opened
/// with. It names the new tab unless that runtime generation was replaced meanwhile or the
/// inventory does not list the tab.
fn new_tab_status(
    scope: &ProjectBrowserScope,
    capture: StatusCapture,
    new_tab_id: &str,
) -> Result<serde_json::Value, ScratchError> {
    let (mut value, current) = render_status(scope, capture);
    if !current {
        return Err(fail(
            StatusCode::CONFLICT,
            "the project browser was replaced while opening the tab; open it again",
        ));
    }
    let listed = value["tabs"]
        .as_array()
        .is_some_and(|tabs| tabs.iter().any(|tab| tab["tab_id"] == new_tab_id));
    if !listed {
        return Err(fail(
            StatusCode::CONFLICT,
            "the new tab closed before it could be shown; open it again",
        ));
    }
    value["new_tab_id"] = serde_json::json!(new_tab_id);
    Ok(value)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NavigateBody {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    tab_id: Option<String>,
}

enum NavigationFailure {
    NoHistoryEntry,
    Cdp {
        error: RendererFailure,
        effect_possible: bool,
    },
}

impl NavigationFailure {
    fn cdp(error: impl std::fmt::Display, dispatch: bool) -> Self {
        let message = error.to_string();
        let error = RendererFailure::from_message(message);
        let effect_possible = dispatch
            && !(error.kind == RendererFailureKind::TimedOut
                && error.message.ends_with("(effect_unknown=false)"));
        Self::Cdp {
            error,
            effect_possible,
        }
    }
}

fn history_step(tab: &headless_chrome::Tab, offset: i64) -> Result<(), NavigationFailure> {
    let history = tab
        .call_method(Page::GetNavigationHistory(None))
        .map_err(|error| NavigationFailure::cdp(error, false))?;
    let index = i64::from(history.current_index) + offset;
    let entry = usize::try_from(index)
        .ok()
        .and_then(|index| history.entries.get(index))
        .ok_or(NavigationFailure::NoHistoryEntry)?;
    tab.call_method(Page::NavigateToHistoryEntry {
        entry_id: entry.id,
    })
    .map(|_| ())
    .map_err(|error| NavigationFailure::cdp(error, true))
}

async fn navigation_response(
    app: &AppState,
    scope: &ProjectBrowserScope,
    runtime: &RuntimeArc,
    fallback: StatusCapture,
) -> Response<Body> {
    let capture = match lock_runtime(runtime).await {
        Ok(mut rt) => capture_status(&mut rt, &fallback.runtime_id),
        Err(_) => fallback,
    };
    let (status, current) = render_status(scope, capture);
    if current {
        return json_response(StatusCode::OK, status);
    }
    let busy_status = |runtime_id: Option<&str>, connected| {
        let mut status = serde_json::json!({
            "project_id": scope.project_id(), "running": true,
            "connected": connected, "busy": true, "tabs": [],
        });
        if let Some(id) = runtime_id {
            status["runtime_id"] = serde_json::json!(id);
        }
        json_response(StatusCode::OK, status)
    };
    match tokio::time::timeout(UI_LOCK_WAIT, find_project_runtime(app, scope)).await {
        Ok(None) => json_response(
            StatusCode::OK,
            serde_json::json!({
                "project_id": scope.project_id(), "running": false, "connected": false, "tabs": [],
            }),
        ),
        Err(_) => busy_status(None, false),
        Ok(Some((id, runtime))) => {
            if let Ok(mut rt) = lock_runtime(&runtime).await {
                let capture = capture_status(&mut rt, &id);
                drop(rt);
                let (status, current) = render_status(scope, capture);
                if current {
                    return json_response(StatusCode::OK, status);
                }
            }
            let owner = crate::integrations::browser_runtime::runtime_owner_record(&id)
                .filter(|owner| owner.is_current_generation());
            busy_status(
                owner.as_ref().map(|_| id.as_str()),
                owner.is_some_and(|owner| owner.is_connected()),
            )
        }
    }
}

async fn handle_navigate(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: NavigateBody = parse_body(&body)?;
    if post.url.is_some() == post.op.is_some() {
        return Err(fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "provide exactly one of url or op (back, forward, reload)",
        ));
    }
    if let Some(op) = post.op.as_deref() {
        if !matches!(op, "back" | "forward" | "reload") {
            return Err(fail(
                StatusCode::UNPROCESSABLE_ENTITY,
                "unknown navigate op; expected back, forward or reload",
            ));
        }
    }
    let url = checked_url(post.url.as_deref())?;
    let scope = scope(&app).await?;
    let (runtime_id, runtime) = live_runtime(&app, &scope).await?;
    let (tab, gate, fence) = {
        let mut rt = lock_runtime(&runtime).await?;
        rt.touch();
        let tab = chosen_tab(&rt, post.tab_id.as_deref())?;
        let gate = rt.tab_gate(tab.get_target_id());
        let fence = TabFence::new(&rt, tab.clone());
        (tab, gate, fence)
    };
    let guard = take_gate(&gate, GateUse::Navigate, NAVIGATE_SKIPPED)?;
    let fallback = {
        let mut rt = fence.lock(&runtime, "").await?;
        capture_status(&mut rt, &runtime_id)
    };
    let dispatched = blocking(|| match (&url, post.op.as_deref()) {
        (Some(url), _) => tab
            .navigate_to(url)
            .map(|_| ())
            .map_err(|error| NavigationFailure::cdp(error, true)),
        (None, Some("back")) => history_step(&tab, -1),
        (None, Some("forward")) => history_step(&tab, 1),
        _ => tab
            .reload(false, None)
            .map(|_| ())
            .map_err(|error| NavigationFailure::cdp(error, true)),
    });
    drop(guard);
    match dispatched {
        Ok(()) => Ok(navigation_response(&app, &scope, &runtime, fallback).await),
        Err(NavigationFailure::NoHistoryEntry) => {
            drop(fence.lock(&runtime, "").await?);
            Err(fail(
                StatusCode::CONFLICT,
                "there is no page in that direction",
            ))
        }
        Err(NavigationFailure::Cdp {
            error,
            effect_possible,
        }) => {
            let effect = if effect_possible {
                "; the navigation may have been applied"
            } else {
                "; the navigation was not started"
            };
            let details = if effect_possible {
                &NAVIGATE_FAILURE
            } else {
                &NAVIGATE_UNSENT_FAILURE
            };
            Err(failure_status(&runtime, &fence, error, details, effect).await)
        }
    }
}

// ---------------------------------------------------------------------------------------
// Input (fenced by runtime_id + tab_id, CSS-pixel coordinates)
// ---------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputBody {
    runtime_id: String,
    tab_id: String,
    kind: String,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
    #[serde(default)]
    button: Option<String>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    modifiers: Vec<String>,
    #[serde(default)]
    dx: Option<f64>,
    #[serde(default)]
    dy: Option<f64>,
}

fn parse_button(button: Option<&str>) -> Result<MouseButton, String> {
    match button.unwrap_or("left") {
        "left" => Ok(MouseButton::Left),
        "middle" => Ok(MouseButton::Middle),
        "right" => Ok(MouseButton::Right),
        _ => Err("button must be left, middle or right".into()),
    }
}

fn unprocessable(error: String) -> ScratchError {
    fail(StatusCode::UNPROCESSABLE_ENTITY, error)
}

/// The key chord for `/input`, every token checked against the keyboard layout before any
/// event is sent.
fn key_chord(key: &str, mask: u32) -> Result<String, String> {
    let key = crate::integrations::browser_actions::normalize_key_name(key);
    if !refact_browser::is_known_key(key) {
        return Err(format!(
            "unsupported key '{}'",
            key.chars().take(32).collect::<String>()
        ));
    }
    let mut tokens = [(1, "Alt"), (2, "Control"), (4, "Meta"), (8, "Shift")]
        .into_iter()
        .filter(|(bit, name)| mask & bit != 0 && *name != key)
        .map(|(_, name)| name)
        .collect::<Vec<_>>();
    tokens.push(key);
    Ok(tokens.join("+"))
}

enum Gesture {
    Click { x: f64, y: f64, button: MouseButton },
    Wheel { x: f64, y: f64, dx: f64, dy: f64 },
    Key(String),
    Text(String),
}

fn validate_input(post: &InputBody, mask: u32) -> Result<Gesture, String> {
    match post.kind.as_str() {
        "click" => Ok(Gesture::Click {
            x: finite_coord("x", post.x)?,
            y: finite_coord("y", post.y)?,
            button: parse_button(post.button.as_deref())?,
        }),
        "wheel" => Ok(Gesture::Wheel {
            x: finite_coord("x", post.x)?,
            y: finite_coord("y", post.y)?,
            dx: wheel_delta("dx", post.dx)?,
            dy: wheel_delta("dy", post.dy)?,
        }),
        "key" if post.text.is_none() => {
            let key = post.key.as_deref().unwrap_or_default();
            if key.is_empty() || key.chars().count() > 32 {
                return Err("key must be 1..=32 characters".into());
            }
            key_chord(key, mask).map(Gesture::Key)
        }
        "key" | "text" => {
            let text = post.text.clone().unwrap_or_default();
            if text.is_empty() || text.chars().count() > MAX_TEXT_CHARS {
                return Err(format!("text must be 1..={MAX_TEXT_CHARS} characters"));
            }
            Ok(Gesture::Text(text))
        }
        _ => Err("kind must be click, wheel, key or text".into()),
    }
}

enum Release {
    Mouse(MouseEventPayload),
    Key(KeyEventPayload),
}

/// Remembers every press that may have reached the page until its release is confirmed, so a
/// gesture that fails half-way can release what it pressed.
struct PressTracker<D> {
    inner: D,
    pending: Vec<Release>,
}

impl<D> PressTracker<D> {
    fn new(inner: D) -> Self {
        Self {
            inner,
            pending: Vec::new(),
        }
    }
}

impl<D: MouseDispatcher + KeyboardDispatcher> PressTracker<D> {
    fn release_pending(&mut self) {
        while let Some(release) = self.pending.pop() {
            let _ = match release {
                Release::Mouse(payload) => {
                    MouseDispatcher::dispatch(&mut self.inner, MouseDispatch::Mouse(payload))
                }
                Release::Key(payload) => {
                    KeyboardDispatcher::dispatch(&mut self.inner, KeyboardDispatch::Key(payload))
                }
            };
        }
    }
}

impl<D: MouseDispatcher> MouseDispatcher for PressTracker<D> {
    fn dispatch(&mut self, event: MouseDispatch) -> Result<(), String> {
        let MouseDispatch::Mouse(payload) = &event else {
            return self.inner.dispatch(event);
        };
        match payload.event_type {
            MouseEventType::Pressed => {
                self.pending.push(Release::Mouse(MouseEventPayload {
                    event_type: MouseEventType::Released,
                    buttons: Some(0),
                    ..payload.clone()
                }));
                self.inner.dispatch(event)
            }
            MouseEventType::Released => {
                let button = payload.button;
                self.inner.dispatch(event)?;
                let released = self.pending.iter().rposition(
                    |release| matches!(release, Release::Mouse(pressed) if pressed.button == button),
                );
                if let Some(index) = released {
                    self.pending.remove(index);
                }
                Ok(())
            }
            _ => self.inner.dispatch(event),
        }
    }
}

impl<D: KeyboardDispatcher> KeyboardDispatcher for PressTracker<D> {
    fn dispatch(&mut self, event: KeyboardDispatch) -> Result<(), String> {
        let KeyboardDispatch::Key(payload) = &event else {
            return self.inner.dispatch(event);
        };
        match payload.event_type {
            KeyEventType::KeyDown | KeyEventType::RawKeyDown => {
                self.pending.push(Release::Key(KeyEventPayload {
                    event_type: KeyEventType::KeyUp,
                    text: None,
                    unmodified_text: None,
                    auto_repeat: None,
                    commands: None,
                    ..payload.clone()
                }));
                self.inner.dispatch(event)
            }
            KeyEventType::KeyUp => {
                let code = payload.code.clone();
                self.inner.dispatch(event)?;
                let released = self.pending.iter().rposition(
                    |release| matches!(release, Release::Key(pressed) if pressed.code == code),
                );
                if let Some(index) = released {
                    self.pending.remove(index);
                }
                Ok(())
            }
        }
    }
}

struct TabInput<'a> {
    mouse: CdpMouseDispatcher<'a>,
    keys: CdpKeyboardDispatcher<'a>,
}

impl<'a> TabInput<'a> {
    fn new(tab: &'a headless_chrome::Tab) -> Self {
        Self {
            mouse: CdpMouseDispatcher::new(tab),
            keys: CdpKeyboardDispatcher::new(tab),
        }
    }
}

impl MouseDispatcher for TabInput<'_> {
    fn dispatch(&mut self, event: MouseDispatch) -> Result<(), String> {
        self.mouse.dispatch(event)
    }
}

impl KeyboardDispatcher for TabInput<'_> {
    fn dispatch(&mut self, event: KeyboardDispatch) -> Result<(), String> {
        self.keys.dispatch(event)
    }
}

fn mouse_payload(event_type: MouseEventType, x: f64, y: f64, mask: u32) -> MouseEventPayload {
    MouseEventPayload {
        event_type,
        x,
        y,
        button: None,
        buttons: None,
        modifiers: mask,
        click_count: None,
        force: None,
        delta_x: None,
        delta_y: None,
    }
}

fn button_bit(button: MouseButton) -> u32 {
    match button {
        MouseButton::None => 0,
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 4,
    }
}

fn dispatch_gesture<D: MouseDispatcher + KeyboardDispatcher>(
    input: &mut PressTracker<D>,
    gesture: &Gesture,
    mask: u32,
) -> Result<(), String> {
    match gesture {
        Gesture::Click {
            x,
            y,
            button,
        } => {
            let pressed = |event_type, buttons| MouseEventPayload {
                button: Some(*button),
                buttons: Some(buttons),
                click_count: Some(1),
                ..mouse_payload(event_type, *x, *y, mask)
            };
            MouseDispatcher::dispatch(
                input,
                MouseDispatch::Mouse(mouse_payload(MouseEventType::Moved, *x, *y, mask)),
            )?;
            MouseDispatcher::dispatch(
                input,
                MouseDispatch::Mouse(pressed(MouseEventType::Pressed, button_bit(*button))),
            )?;
            MouseDispatcher::dispatch(
                input,
                MouseDispatch::Mouse(pressed(MouseEventType::Released, 0)),
            )
        }
        Gesture::Wheel {
            x,
            y,
            dx,
            dy,
        } => MouseDispatcher::dispatch(
            input,
            MouseDispatch::Mouse(MouseEventPayload {
                delta_x: Some(*dx),
                delta_y: Some(*dy),
                ..mouse_payload(MouseEventType::Wheel, *x, *y, mask)
            }),
        ),
        Gesture::Key(chord) => Keyboard::new(&mut *input).press(chord, None),
        Gesture::Text(text) => {
            KeyboardDispatcher::dispatch(input, KeyboardDispatch::InsertText(text.clone()))
        }
    }
}

/// A gesture that fails after a press may have reached the page gets one bounded release
/// attempt for everything it pressed, latest first.
fn dispatch_gesture_with_release<D: MouseDispatcher + KeyboardDispatcher>(
    input: &mut PressTracker<D>,
    gesture: &Gesture,
    mask: u32,
) -> Result<(), String> {
    let result = blocking_within(RENDERER_DEADLINE, || dispatch_gesture(input, gesture, mask));
    if result.is_err() && !input.pending.is_empty() {
        blocking_within(RELEASE_DEADLINE, || input.release_pending());
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RendererFailureKind {
    TimedOut,
    PageChanging,
    Failed,
}

struct RendererFailure {
    kind: RendererFailureKind,
    message: String,
}

/// The one place renderer errors are classified: a missed deadline, a page whose session is
/// detached or being swapped by a navigation while the target remains, or a real failure.
fn classify_renderer_error(message: &str) -> RendererFailureKind {
    const PAGE_CHANGING: [&str; 6] = [
        "Not attached to an active page",
        "Cannot find context with specified id",
        "Execution context was destroyed",
        "Inspected target navigated or closed",
        "Session with given id not found",
        "Target closed",
    ];
    if message.contains("CDP command deadline exceeded") {
        RendererFailureKind::TimedOut
    } else if PAGE_CHANGING.iter().any(|known| message.contains(known)) {
        RendererFailureKind::PageChanging
    } else {
        RendererFailureKind::Failed
    }
}

impl RendererFailure {
    fn from_message(message: String) -> Self {
        Self {
            kind: classify_renderer_error(&message),
            message,
        }
    }

    fn from_cdp<E: std::fmt::Display>(error: E) -> Self {
        Self::from_message(error.to_string())
    }
}

struct FailureDetails {
    timed_out: &'static str,
    page_changing: &'static str,
    failed_prefix: &'static str,
}

const FRAME_FAILURE: FailureDetails = FailureDetails {
    timed_out: FRAME_BUSY,
    page_changing: "the page is changing; no frame was captured; try again",
    failed_prefix: "capturing the frame failed: ",
};

const RESIZE_FAILURE: FailureDetails = FailureDetails {
    timed_out: RESIZE_BUSY,
    page_changing: "the page is changing; the pane size was not confirmed; try again",
    failed_prefix: "applying the pane size failed: ",
};

const INPUT_FAILURE: FailureDetails = FailureDetails {
    timed_out: INPUT_BUSY,
    page_changing: "the page was changing while the input was sent; it may have been applied",
    failed_prefix: "the input may have been partly applied: ",
};

const NAVIGATE_FAILURE: FailureDetails = FailureDetails {
    timed_out: "the browser did not confirm the navigation in time; it may have been applied",
    page_changing: "the page was changing when navigation was requested; it may have been applied",
    failed_prefix: "navigation failed and may have been applied: ",
};

const NAVIGATE_UNSENT_FAILURE: FailureDetails = FailureDetails {
    timed_out: "the browser did not return in time; the navigation was not started",
    page_changing: "the page is changing; the navigation was not started; try again",
    failed_prefix: "the navigation was not started: ",
};

struct TabFence {
    ownership: Arc<refact_browser::RuntimeOwnership>,
    epoch: u64,
    tab: Arc<headless_chrome::Tab>,
}

impl TabFence {
    fn new(rt: &BrowserRuntime, tab: Arc<headless_chrome::Tab>) -> Self {
        Self {
            ownership: rt.ownership.clone(),
            epoch: rt.ownership.connection_epoch(),
            tab,
        }
    }

    fn validate(&self, rt: &BrowserRuntime, effect: &str) -> Result<(), ScratchError> {
        if !self.ownership.is_current_generation()
            || !Arc::ptr_eq(&rt.ownership, &self.ownership)
            || !self.ownership.is_connection_epoch(self.epoch)
        {
            return Err(self.replaced(effect));
        }
        match rt.find_tab(self.tab.get_target_id()) {
            None => Err(fail(
                StatusCode::GONE,
                format!("stale tab_id: no such tab{effect}"),
            )),
            Some(tab) if !Arc::ptr_eq(&tab, &self.tab) => Err(self.replaced(effect)),
            Some(_) => Ok(()),
        }
    }

    fn replaced(&self, effect: &str) -> ScratchError {
        fail(
            StatusCode::CONFLICT,
            format!("the project browser was replaced or reconnected; refresh its status{effect}"),
        )
    }

    async fn lock<'a>(
        &self,
        runtime: &'a RuntimeArc,
        effect: &str,
    ) -> Result<tokio::sync::MutexGuard<'a, BrowserRuntime>, ScratchError> {
        let rt = tokio::time::timeout(UI_LOCK_WAIT, runtime.lock()).await;
        if !self.ownership.is_current_generation()
            || !self.ownership.is_connection_epoch(self.epoch)
        {
            return Err(self.replaced(effect));
        }
        let rt = rt.map_err(|_| {
            fail(
                StatusCode::CONFLICT,
                format!("the browser could not verify the tab before the deadline{effect}"),
            )
        })?;
        self.validate(&rt, effect)?;
        Ok(rt)
    }
}

async fn failure_status(
    runtime: &RuntimeArc,
    fence: &TabFence,
    failure: RendererFailure,
    details: &FailureDetails,
    effect: &str,
) -> ScratchError {
    if let Ok(rt) = tokio::time::timeout(UI_LOCK_WAIT, runtime.lock()).await {
        if let Err(error) = fence.validate(&rt, effect) {
            return error;
        }
    }
    if !fence.ownership.is_current_generation() || !fence.ownership.is_connection_epoch(fence.epoch)
    {
        return fence.replaced(effect);
    }
    renderer_failure_status(failure, details)
}

fn renderer_failure_status(failure: RendererFailure, details: &FailureDetails) -> ScratchError {
    match failure.kind {
        RendererFailureKind::TimedOut => fail(StatusCode::CONFLICT, details.timed_out),
        RendererFailureKind::PageChanging => fail(StatusCode::CONFLICT, details.page_changing),
        RendererFailureKind::Failed => fail(
            StatusCode::BAD_GATEWAY,
            format!(
                "{}{}",
                details.failed_prefix,
                refact_core::string_utils::redact_sensitive(&failure.message)
            ),
        ),
    }
}

const FRAME_SKIPPED: &str = "no frame was captured; try again";
const RESIZE_SKIPPED: &str = "the pane size was not applied; try again";
const INPUT_SKIPPED: &str = "the input was not sent";
const NAVIGATE_SKIPPED: &str = "the navigation was not started; try again";

fn gate_busy(holder: GateUse, skipped: &str) -> ScratchError {
    if holder == GateUse::Frame && skipped == FRAME_SKIPPED {
        return fail(
            StatusCode::CONFLICT,
            "a frame of this tab is already being captured; try again",
        );
    }
    fail(
        StatusCode::CONFLICT,
        format!(
            "this tab is busy with {}; {skipped}",
            gate_use_label(holder)
        ),
    )
}

/// Takes the tab's gate within the renderer budget; the runtime lock must not be held.
fn take_gate(
    gate: &Arc<TabGate>,
    user: GateUse,
    skipped: &str,
) -> Result<TabGateGuard, ScratchError> {
    blocking_within(RENDERER_DEADLINE, || {
        gate.acquire(user, Instant::now() + RENDERER_DEADLINE)
    })
    .map_err(|holder| gate_busy(holder, skipped))
}

/// The pane size the tab should be given now, if any; a tool viewport wins over the pane.
fn pane_to_apply(rt: &BrowserRuntime, guard: &TabGateGuard, tab_id: &str) -> Option<(u32, u32)> {
    if tool_override_active(rt, Some(tab_id)) {
        guard.update_view(|view| view.applied_pane = None);
        return None;
    }
    let size = rt.pane.desired(Some(tab_id))?;
    (guard.view().applied_pane != Some(size)).then_some(size)
}

async fn handle_input(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: InputBody = parse_body(&body)?;
    let mask = modifier_mask(&post.modifiers).map_err(unprocessable)?;
    let gesture = validate_input(&post, mask).map_err(unprocessable)?;
    let scope = scope(&app).await?;
    let (runtime_id, runtime) = live_runtime(&app, &scope).await?;
    let (tab, gate, fence) = {
        let mut rt = lock_runtime(&runtime).await?;
        let tab = tab_by_id(&rt, &post.tab_id);
        check_fence(&post.runtime_id, &runtime_id, tab.is_some()).map_err(fence_error)?;
        let tab = tab.ok_or_else(|| fence_error(FenceError::TabChanged))?;
        let gate = rt.tab_gate(&post.tab_id);
        let fence = TabFence::new(&rt, tab.clone());
        (tab, gate, fence)
    };
    let _guard = take_gate(&gate, GateUse::Input, INPUT_SKIPPED)?;
    fence.lock(&runtime, "").await?.touch();
    let mut input = PressTracker::new(TabInput::new(&tab));
    if let Err(error) = dispatch_gesture_with_release(&mut input, &gesture, mask) {
        return Err(failure_status(
            &runtime,
            &fence,
            RendererFailure::from_message(error),
            &INPUT_FAILURE,
            "; the input may have been applied",
        )
        .await);
    }
    let _final = fence.lock(&runtime, "; the input was sent").await?;
    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({"ok": true, "runtime_id": runtime_id, "tab_id": post.tab_id}),
    ))
}

// ---------------------------------------------------------------------------------------
// Resize and frame
// ---------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResizeBody {
    width: u32,
    height: u32,
    #[serde(default)]
    tab_id: Option<String>,
}

async fn handle_resize(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ResizeBody = parse_body(&body)?;
    let size = validate_pane(post.width, post.height).map_err(unprocessable)?;
    let scope = scope(&app).await?;
    let (runtime_id, runtime) = live_runtime(&app, &scope).await?;
    let (tab, gate, fence) = {
        let mut rt = lock_runtime(&runtime).await?;
        let tab = match post.tab_id.as_deref() {
            Some(id) => Some(tab_by_id(&rt, id).ok_or_else(stale_tab)?),
            None => rt.get_active_tab(),
        };
        let Some(tab) = tab else {
            rt.pane.remember(None, size);
            return Ok(resize_response(
                &runtime_id,
                size,
                false,
                Some("no_active_tab"),
            ));
        };
        let gate = rt.tab_gate(tab.get_target_id());
        let fence = TabFence::new(&rt, tab.clone());
        (tab, gate, fence)
    };
    let tab_id = tab.get_target_id().to_string();
    let guard = take_gate(&gate, GateUse::Resize, RESIZE_SKIPPED)?;
    {
        let mut rt = fence.lock(&runtime, "").await?;
        if tool_override_active(&rt, Some(&tab_id)) {
            guard.update_view(|view| view.applied_pane = None);
            rt.pane.remember(Some(&tab_id), size);
            return Ok(resize_response(
                &runtime_id,
                size,
                false,
                Some("tool_viewport_override_active"),
            ));
        }
    }
    if let Err(failure) = blocking_within(RENDERER_DEADLINE, || apply_pane_viewport(&tab, size)) {
        guard.update_view(|view| view.applied_pane = None);
        return Err(failure_status(
            &runtime,
            &fence,
            failure,
            &RESIZE_FAILURE,
            "; the pane size may have been applied",
        )
        .await);
    }
    // Mark the cache unknown until the runtime confirms it can commit the new size.
    guard.update_view(|view| view.applied_pane = None);
    let mut rt = fence.lock(&runtime, "; the pane size was applied").await?;
    rt.pane.remember(Some(&tab_id), size);
    guard.update_view(|view| view.applied_pane = Some(size));
    Ok(resize_response(&runtime_id, size, true, None))
}

fn resize_response(
    runtime_id: &str,
    size: (u32, u32),
    applied: bool,
    reason: Option<&str>,
) -> Response<Body> {
    json_response(
        StatusCode::OK,
        serde_json::json!({
            "runtime_id": runtime_id,
            "applied": applied,
            "reason": reason,
            "viewport": {"width": size.0, "height": size.1},
        }),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameBody {
    #[serde(default)]
    max_side: Option<u32>,
    #[serde(default)]
    tab_id: Option<String>,
}

/// The pane size, screenshot and layout metrics of one frame come from one hold of the tab's
/// gate, so the reported viewport always describes the captured pixels. Renderer work runs
/// without the runtime lock and within a short deadline.
async fn handle_frame(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: FrameBody = parse_body(&body)?;
    let max_side = post
        .max_side
        .unwrap_or(DEFAULT_FRAME_SIDE)
        .clamp(MIN_FRAME_SIDE, MAX_FRAME_SIDE);
    let scope = scope(&app).await?;
    let (runtime_id, runtime) = live_runtime(&app, &scope).await?;
    let (tab, gate, fence) = {
        let mut rt = lock_runtime(&runtime).await?;
        let tab = chosen_tab(&rt, post.tab_id.as_deref())?;
        let gate = rt.tab_gate(tab.get_target_id());
        let fence = TabFence::new(&rt, tab.clone());
        (tab, gate, fence)
    };
    let tab_id = tab.get_target_id().to_string();
    let guard = take_gate(&gate, GateUse::Frame, FRAME_SKIPPED)?;
    let pane_size = {
        let rt = fence.lock(&runtime, "").await?;
        pane_to_apply(&rt, &guard, &tab_id)
    };
    let mut pane_written = false;
    let captured = blocking_within(RENDERER_DEADLINE, || {
        if let Some(size) = pane_size {
            apply_pane_viewport(&tab, size)?;
            pane_written = true;
        }
        let shot = tab
            .call_method(Page::CaptureScreenshot {
                format: Some(Page::CaptureScreenshotFormatOption::Jpeg),
                clip: None,
                quality: Some(u32::from(FRAME_JPEG_QUALITY)),
                from_surface: Some(true),
                capture_beyond_viewport: Some(false),
                optimize_for_speed: Some(true),
            })
            .map_err(RendererFailure::from_cdp)?;
        let metrics = tab
            .call_method(Page::GetLayoutMetrics(None))
            .map_err(RendererFailure::from_cdp)?;
        Ok::<_, RendererFailure>((
            shot.data,
            metrics.css_visual_viewport.client_width,
            metrics.css_visual_viewport.client_height,
            tab.get_url(),
            tab_title(&tab),
        ))
    });
    if pane_size.is_some() {
        guard.update_view(|view| view.applied_pane = None);
    }
    let (data, css_width, css_height, url, title) = match captured {
        Ok(captured) => captured,
        Err(failure) => {
            return Err(failure_status(&runtime, &fence, failure, &FRAME_FAILURE, "").await)
        }
    };
    let raw = base64::prelude::BASE64_STANDARD
        .decode(&data)
        .map_err(|error| fail(StatusCode::BAD_GATEWAY, format!("bad frame data: {error}")))?;
    let policy = ImagePolicy {
        max_side,
        preferred_side: max_side,
        format: ImageFormat::Jpeg,
        quality: Some(FRAME_JPEG_QUALITY),
        ..ImagePolicy::default()
    };
    let (resized, mime) = resize_to_policy(&raw, "image/jpeg", &policy)
        .map_err(|error| fail(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let (width, height) = image::load_from_memory(&resized)
        .map(|image| (image.width(), image.height()))
        .map_err(|error| fail(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let scale = if css_width > 0.0 {
        f64::from(width) / css_width
    } else {
        1.0
    };
    if let Some(size) = pane_size.filter(|_| pane_written) {
        let mut rt = fence.lock(&runtime, "").await?;
        rt.pane.keep_tab_size(&tab_id, size);
        guard.update_view(|view| view.applied_pane = Some(size));
    }
    drop(guard);
    // TargetInfo can retain Chrome's provisional URL-as-title indefinitely. Title is optional
    // metadata, not part of the frame's capture/metrics transaction or its failure decision.
    let title = blocking_within(FRAME_TITLE_DEADLINE, || {
        let value = tab
            .evaluate(r"JSON.stringify([location.href, document.title])", false)
            .ok()?
            .value?;
        let [document_url, title] = serde_json::from_str::<[String; 2]>(value.as_str()?).ok()?;
        (document_url == url).then_some(title)
    })
    .unwrap_or(title);
    let response = json_response(
        StatusCode::OK,
        serde_json::json!({
            "runtime_id": runtime_id,
            "tab_id": tab_id,
            "mime": mime,
            "data": base64::prelude::BASE64_STANDARD.encode(&resized),
            "width": width,
            "height": height,
            "viewport": {"width": css_width, "height": css_height},
            "scale": scale,
            "url": url,
            "title": title,
        }),
    );
    let _final = fence.lock(&runtime, "").await?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use hyper::body::to_bytes;
    use tower::ServiceExt;

    #[test]
    fn navigate_urls_are_limited_to_http_https_and_blank() {
        assert_eq!(validate_navigate_url("about:blank").unwrap(), "about:blank");
        assert!(validate_navigate_url(" https://example.com/a ").is_ok());
        assert!(validate_navigate_url("http://localhost:3000").is_ok());
        for bad in [
            "",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "chrome://settings",
            "data:text/html,hi",
            "view-source:https://example.com",
            "https://",
            "not a url",
        ] {
            assert!(validate_navigate_url(bad).is_err(), "{bad}");
        }
        assert!(validate_navigate_url(&format!("https://e.com/{}", "a".repeat(9000))).is_err());
    }

    #[test]
    fn modifiers_are_mapped_and_bounded() {
        assert_eq!(modifier_mask(&[]).unwrap(), 0);
        let all: Vec<String> = ["Alt", "ctrl", "Meta", "SHIFT"]
            .iter()
            .map(|m| m.to_string())
            .collect();
        assert_eq!(modifier_mask(&all).unwrap(), 15);
        assert!(modifier_mask(&["Hyper".to_string()]).is_err());
        assert!(modifier_mask(&vec!["Alt".to_string(); 5]).is_err());
    }

    #[test]
    fn input_fence_rejects_stale_runtime_and_unknown_tab_but_not_background_tabs() {
        assert_eq!(check_fence("rt", "rt", true), Ok(()));
        assert_eq!(
            check_fence("rt-old", "rt", true),
            Err(FenceError::RuntimeChanged)
        );
        assert_eq!(
            check_fence("rt-old", "rt", false),
            Err(FenceError::RuntimeChanged)
        );
        assert_eq!(check_fence("rt", "rt", false), Err(FenceError::TabChanged));
    }

    #[test]
    fn pane_and_coordinates_are_bounded() {
        assert!(validate_pane(800, 600).is_ok());
        assert!(validate_pane(10, 600).is_err());
        assert!(validate_pane(800, 100_000).is_err());
        assert!(finite_coord("x", Some(f64::NAN)).is_err());
        assert!(finite_coord("x", Some(-1.0)).is_err());
        assert!(finite_coord("x", None).is_err());
        assert!(finite_coord("x", Some(10.5)).is_ok());
        assert!(wheel_delta("dy", Some(1e9)).is_err());
        assert_eq!(wheel_delta("dy", None).unwrap(), 0.0);
    }

    #[derive(Default)]
    struct ScriptedInput {
        sent: Vec<String>,
        fail_once: Vec<(String, &'static str)>,
    }

    impl ScriptedInput {
        fn failing(label: &str, error: &'static str) -> Self {
            Self {
                sent: Vec::new(),
                fail_once: vec![(label.to_string(), error)],
            }
        }

        fn record(&mut self, label: String) -> Result<(), String> {
            self.sent.push(label.clone());
            match self
                .fail_once
                .iter()
                .position(|(failing, _)| *failing == label)
            {
                Some(index) => Err(self.fail_once.remove(index).1.to_string()),
                None => Ok(()),
            }
        }
    }

    impl MouseDispatcher for ScriptedInput {
        fn dispatch(&mut self, event: MouseDispatch) -> Result<(), String> {
            match event {
                MouseDispatch::Mouse(payload) => self.record(format!(
                    "{:?}@{},{}:{:?}",
                    payload.event_type, payload.x, payload.y, payload.button
                )),
                MouseDispatch::Touch(_) => self.record("touch".into()),
            }
        }
    }

    impl KeyboardDispatcher for ScriptedInput {
        fn dispatch(&mut self, event: KeyboardDispatch) -> Result<(), String> {
            match event {
                KeyboardDispatch::Key(payload) => {
                    self.record(format!("{:?}:{}", payload.event_type, payload.key))
                }
                KeyboardDispatch::InsertText(text) => self.record(format!("text:{text}")),
            }
        }
    }

    const TIMEOUT: &str = "Failed to dispatch browser key event: CDP command deadline exceeded: Input.dispatchKeyEvent (effect_unknown=true)";

    fn run_gesture(
        input: ScriptedInput,
        gesture: Gesture,
        mask: u32,
    ) -> (Result<(), String>, Vec<String>) {
        let mut tracker = PressTracker::new(input);
        let result = dispatch_gesture_with_release(&mut tracker, &gesture, mask);
        assert!(tracker.pending.is_empty());
        (result, tracker.inner.sent)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_gesture_releases_what_it_pressed_exactly_once() {
        let click = || Gesture::Click {
            x: 10.0,
            y: 20.0,
            button: MouseButton::Left,
        };
        let (result, sent) = run_gesture(ScriptedInput::default(), click(), 0);
        assert!(result.is_ok());
        assert_eq!(
            sent,
            [
                "Moved@10,20:None",
                "Pressed@10,20:Some(Left)",
                "Released@10,20:Some(Left)"
            ]
        );

        let (result, sent) = run_gesture(
            ScriptedInput::failing("Released@10,20:Some(Left)", TIMEOUT),
            click(),
            0,
        );
        assert!(result.unwrap_err().contains("deadline exceeded"));
        assert_eq!(
            sent,
            [
                "Moved@10,20:None",
                "Pressed@10,20:Some(Left)",
                "Released@10,20:Some(Left)",
                "Released@10,20:Some(Left)",
            ],
            "the release is retried once after the gesture failed"
        );

        let (result, sent) = run_gesture(
            ScriptedInput::failing("Pressed@10,20:Some(Left)", TIMEOUT),
            click(),
            0,
        );
        assert!(result.is_err());
        assert_eq!(
            sent,
            [
                "Moved@10,20:None",
                "Pressed@10,20:Some(Left)",
                "Released@10,20:Some(Left)",
            ],
            "a press that may have been applied is released"
        );

        let (result, sent) = run_gesture(
            ScriptedInput::failing("Moved@10,20:None", TIMEOUT),
            click(),
            0,
        );
        assert!(result.is_err());
        assert_eq!(
            sent,
            ["Moved@10,20:None"],
            "nothing was pressed, nothing is released"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_key_chord_releases_the_key_then_the_modifiers() {
        let chord = || Gesture::Key(key_chord("a", 2).unwrap());
        let (result, sent) = run_gesture(ScriptedInput::default(), chord(), 2);
        assert!(result.is_ok());
        assert_eq!(
            sent,
            [
                "RawKeyDown:Control",
                "RawKeyDown:a",
                "KeyUp:a",
                "KeyUp:Control"
            ]
        );

        let mut both_ups_fail = ScriptedInput::failing("KeyUp:a", TIMEOUT);
        both_ups_fail
            .fail_once
            .push(("KeyUp:Control".to_string(), TIMEOUT));
        let (result, sent) = run_gesture(both_ups_fail, chord(), 2);
        assert!(result.is_err());
        assert_eq!(
            sent,
            [
                "RawKeyDown:Control",
                "RawKeyDown:a",
                "KeyUp:a",
                "KeyUp:Control",
                "KeyUp:a",
                "KeyUp:Control",
            ],
            "after the gesture fails the key is released first, then the modifier"
        );

        let (result, sent) =
            run_gesture(ScriptedInput::failing("RawKeyDown:a", TIMEOUT), chord(), 2);
        assert!(result.is_err());
        assert_eq!(
            sent,
            [
                "RawKeyDown:Control",
                "RawKeyDown:a",
                "KeyUp:a",
                "KeyUp:Control"
            ],
            "the keyboard's own release already confirmed both keys"
        );
    }

    #[test]
    fn keys_are_validated_before_dispatch_and_dispatch_errors_are_not_validation_errors() {
        assert_eq!(key_chord("Return", 0).unwrap(), "Enter");
        assert_eq!(key_chord("a", 1 | 8).unwrap(), "Alt+Shift+a");
        for (key, mask) in [("Alt", 1), ("Control", 2), ("Meta", 4), ("Shift", 8)] {
            assert_eq!(key_chord(key, mask).unwrap(), key);
        }
        assert_eq!(key_chord("Shift", 2 | 8).unwrap(), "Control+Shift");
        assert!(key_chord("NoSuchKey", 0)
            .unwrap_err()
            .contains("unsupported key"));
        let unrelated = renderer_failure_status(
            RendererFailure::from_message(
                "Failed to dispatch browser key event: Method call error -32602: Invalid parameters"
                    .into(),
            ),
            &INPUT_FAILURE,
        );
        assert_eq!(unrelated.status_code, StatusCode::BAD_GATEWAY);
        assert!(unrelated
            .message
            .starts_with("the input may have been partly applied"));
        let timed_out = renderer_failure_status(
            RendererFailure::from_message(TIMEOUT.into()),
            &INPUT_FAILURE,
        );
        assert_eq!(timed_out.status_code, StatusCode::CONFLICT);
        assert_eq!(timed_out.message, INPUT_BUSY);
    }

    #[test]
    fn a_page_mid_navigation_is_transient_not_a_gateway_error() {
        for message in [
            "Method call error -32000: Not attached to an active page",
            "Method call error -32000: Cannot find context with specified id",
            "Method call error -32000: Execution context was destroyed.",
            "Method call error -32000: Inspected target navigated or closed",
            "Method call error -32001: Session with given id not found.",
        ] {
            assert_eq!(
                classify_renderer_error(message),
                RendererFailureKind::PageChanging,
                "{message}"
            );
            for (details, expected) in [
                (&FRAME_FAILURE, FRAME_FAILURE.page_changing),
                (&RESIZE_FAILURE, RESIZE_FAILURE.page_changing),
                (&INPUT_FAILURE, INPUT_FAILURE.page_changing),
            ] {
                let status =
                    renderer_failure_status(RendererFailure::from_message(message.into()), details);
                assert_eq!(status.status_code, StatusCode::CONFLICT, "{message}");
                assert_eq!(status.message, expected);
            }
        }
        assert!(INPUT_FAILURE
            .page_changing
            .contains("may have been applied"));
        assert_eq!(
            classify_renderer_error(TIMEOUT),
            RendererFailureKind::TimedOut
        );
        assert_eq!(
            classify_renderer_error("Method call error -32602: Invalid parameters"),
            RendererFailureKind::Failed
        );
    }

    #[test]
    fn status_tabs_carry_name_and_ownership_or_explicit_nulls() {
        let tab = crate::integrations::browser_models::TabInfo {
            id: "t1".into(),
            target_id: "t1".into(),
            url: "https://example.com/".into(),
            title: "Example".into(),
            active: false,
            opener: None,
            opened_by_step: Some(2),
            name: Some("ignored, meta wins".into()),
        };
        assert_eq!(
            tab_status_json(&tab, None),
            serde_json::json!({
                "tab_id": "t1", "url": "https://example.com/", "title": "Example", "active": false,
                "name": null, "opened_by_chat": null, "last_chat": null, "last_used_ms": null,
            })
        );
        let meta = refact_browser::TabMeta {
            name: Some("Docs".into()),
            opened_by_chat: Some("chat-a".into()),
            last_chat: Some("chat-b".into()),
            last_used_ms: Some(1_700_000_000_000),
        };
        assert_eq!(
            tab_status_json(&tab, Some(&meta)),
            serde_json::json!({
                "tab_id": "t1", "url": "https://example.com/", "title": "Example", "active": false,
                "name": "Docs", "opened_by_chat": "chat-a", "last_chat": "chat-b",
                "last_used_ms": 1_700_000_000_000u64,
            })
        );
    }

    const PROJECT_ID: &str = "project-fixture";

    async fn harness(daemon_managed: bool) -> (axum::Router, tempfile::TempDir) {
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        let project = tempfile::tempdir().unwrap();
        let cmdline = &mut Arc::get_mut(&mut gcx).unwrap().cmdline;
        if daemon_managed {
            cmdline.daemon_endpoint = "http://127.0.0.1:8488".into();
            cmdline.project_id = PROJECT_ID.into();
            cmdline.daemon_auth_token = Some("general-worker-auth".into());
        }
        cmdline.workspace_folder = project.path().to_str().unwrap().into();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().into()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        (crate::http::routers::make_refact_http_server(app), project)
    }

    fn request(path: &str, body: Body) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(format!("/v1/browser/project/{path}"))
            .header("content-type", "application/json")
            .body(body)
            .unwrap()
    }

    async fn post(
        router: &axum::Router,
        path: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = router
            .clone()
            .oneshot(request(path, Body::from(body.to_string())))
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn project_browser_routes_need_no_credentials_in_any_mode() {
        for daemon_managed in [false, true] {
            let (router, _project) = harness(daemon_managed).await;
            for peer in [None, Some("192.0.2.5:8080"), Some("127.0.0.1:8080")] {
                let mut req = request("status", Body::empty());
                req.headers_mut().insert(
                    header::ORIGIN,
                    HeaderValue::from_static("http://devbox.lan:8001"),
                );
                if let Some(peer) = peer {
                    req.extensions_mut().insert(axum::extract::ConnectInfo(
                        peer.parse::<std::net::SocketAddr>().unwrap(),
                    ));
                }
                let response = router.clone().oneshot(req).await.unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::OK,
                    "{daemon_managed} {peer:?}"
                );
                assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scope_follows_the_session_project_when_the_cli_folder_is_empty() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        assert!(gcx.cmdline.workspace_folder.is_empty());
        let project = tempfile::tempdir().unwrap();
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let router = crate::http::routers::make_refact_http_server(app.clone());
        let (status, value) = post(&router, "status", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{value}");
        let detail = value["detail"].as_str().unwrap();
        assert!(!detail.contains('/') && !detail.contains('\\'), "{detail}");

        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().into()];
        let (status, value) = post(&router, "status", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        let expected = ProjectBrowserScope::from_source_root(project.path()).unwrap();
        assert_eq!(value["project_id"], expected.project_id());
        assert_eq!(scope(&app).await.unwrap(), expected);
        assert_eq!(
            crate::integrations::browser_runtime::project_browser::current_project_scope(&app)
                .await
                .unwrap(),
            expected
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn managed_worker_scope_stays_on_its_pinned_project_root() {
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        let project_a = tempfile::tempdir().unwrap();
        let project_b = tempfile::tempdir().unwrap();
        {
            let cmdline = &mut Arc::get_mut(&mut gcx).unwrap().cmdline;
            cmdline.daemon_endpoint = "http://127.0.0.1:8488".into();
            cmdline.project_id = PROJECT_ID.into();
            cmdline.workspace_folder = project_a.path().to_str().unwrap().into();
        }
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project_a.path().into()];
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let router = crate::http::routers::make_refact_http_server(app.clone());
        let expected = ProjectBrowserScope::from_source_root(project_a.path()).unwrap();
        for roots in [vec![project_b.path().to_path_buf()], Vec::new()] {
            *gcx.documents_state.workspace_folders.lock().unwrap() = roots;
            assert_eq!(scope(&app).await.unwrap(), expected);
            let (status, value) = post(&router, "status", serde_json::json!({})).await;
            assert_eq!(status, StatusCode::OK, "{value}");
            assert_eq!(value["project_id"], expected.project_id());
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn status_without_runtime_reports_not_running() {
        let (router, _project) = harness(false).await;
        let (status, value) = post(&router, "status", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["running"], false);
        assert_eq!(value["connected"], false);
        assert_eq!(value["tabs"], serde_json::json!([]));
        assert!(value["project_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn operations_require_a_running_project_browser() {
        let (router, _project) = harness(false).await;
        for (path, body) in [
            ("tab", serde_json::json!({"op": "new"})),
            ("tab", serde_json::json!({"op": "new", "chat_id": "chat-a"})),
            ("tab", serde_json::json!({"op": "close", "tab_id": "t"})),
            ("navigate", serde_json::json!({"op": "reload"})),
            ("resize", serde_json::json!({"width": 800, "height": 600})),
            ("frame", serde_json::json!({})),
            ("frame", serde_json::json!({"tab_id": "t"})),
            (
                "resize",
                serde_json::json!({"width": 800, "height": 600, "tab_id": "t"}),
            ),
            (
                "input",
                serde_json::json!({"runtime_id": "r", "tab_id": "t", "kind": "key", "key": "a"}),
            ),
        ] {
            let response = router
                .clone()
                .oneshot(request(path, Body::from(body.to_string())))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let bytes = to_bytes(response.into_body()).await.unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(value["detail"].as_str().unwrap().contains("not running"));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bad_requests_are_rejected_before_touching_a_browser() {
        let (router, _project) = harness(false).await;
        let cases = [
            ("status", serde_json::json!({"chat_id": "x"})),
            ("navigate", serde_json::json!({})),
            (
                "navigate",
                serde_json::json!({"url": "https://a.com", "op": "reload"}),
            ),
            ("navigate", serde_json::json!({"url": "file:///etc/passwd"})),
            ("navigate", serde_json::json!({"op": "sideways"})),
            ("tab", serde_json::json!({"op": "open"})),
            ("tab", serde_json::json!({"op": "select", "tab_id": "t"})),
            (
                "tab",
                serde_json::json!({"op": "new", "chat_id": "../escape"}),
            ),
            ("tab", serde_json::json!({"op": "new", "chat_id": ""})),
            ("tab", serde_json::json!({"op": "new", "tab_id": "t"})),
            ("tab", serde_json::json!({"op": "close"})),
            (
                "tab",
                serde_json::json!({"op": "close", "tab_id": "t", "chat_id": "chat-a"}),
            ),
            ("frame", serde_json::json!({"tab": "t"})),
            (
                "resize",
                serde_json::json!({"width": 800, "height": 600, "tab": "t"}),
            ),
            (
                "tab",
                serde_json::json!({"op": "new", "url": "javascript:1"}),
            ),
            ("resize", serde_json::json!({"width": 5, "height": 600})),
            (
                "input",
                serde_json::json!({"runtime_id": "r", "tab_id": "t", "kind": "click", "x": -4, "y": 1}),
            ),
            (
                "input",
                serde_json::json!({"runtime_id": "r", "tab_id": "t", "kind": "key", "key": "a", "modifiers": ["Hyper"]}),
            ),
            (
                "input",
                serde_json::json!({"runtime_id": "r", "tab_id": "t", "kind": "eval"}),
            ),
            (
                "input",
                serde_json::json!({"runtime_id": "r", "tab_id": "t", "kind": "key", "key": "NoSuchKey"}),
            ),
        ];
        for (path, body) in cases {
            let (status, _) = post(&router, path, body.clone()).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path} {body}");
        }
        let response = router
            .clone()
            .oneshot(request("tab", Body::from("{not json")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    const LIVE_PAGE: &str = "data:text/html,<html><body style='margin:0'><div id=c style='font:80px sans-serif'>0</div><input id=i style='position:absolute;left:0;top:200px;width:300px;height:40px'><script>let n=0;function tick(){n++;document.getElementById('c').textContent=String(n);requestAnimationFrame(tick)}requestAnimationFrame(tick);document.addEventListener('mousedown',e=>{if(e.target.id!=='i')document.body.style.background='red'})</script></body></html>";

    fn eval_string(tab: &headless_chrome::Tab, expression: &str) -> String {
        tab.evaluate(expression, false)
            .unwrap()
            .value
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default()
    }

    /// A local HTTP endpoint whose single request is held until released, so a batch can be
    /// paused mid-step while other requests use the runtime.
    async fn held_request_server() -> (
        String,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let hold_url = format!("http://{}/hold", listener.local_addr().unwrap());
        let (arrived_tx, arrived_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer).await.unwrap();
            arrived_tx.send(()).unwrap();
            release_rx.await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        (hold_url, arrived_rx, release_tx, server)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn pinned_switch_after_an_external_close_fails_with_real_chrome() {
        use crate::integrations::browser_controller::execute_request_with_runtime;
        use crate::integrations::browser_models::{parse_browser_action_request, TabTarget};
        use crate::integrations::browser_runtime::project_browser::chat_default_target;

        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let cache = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx).unwrap().cache_dir = cache.path().to_path_buf();
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let scope = ProjectBrowserScope::from_source_root(project.path()).unwrap();
        let ensured = ensure_project_runtime(
            &app,
            &scope,
            crate::integrations::browser_runtime::BrowserLaunchOptions::headless(true),
            Instant::now() + START_DEADLINE,
        )
        .await
        .unwrap();
        let runtime = ensured.runtime.clone();
        let target = |chat: &'static str| {
            let runtime = runtime.clone();
            async move {
                let mut rt = runtime.lock().await;
                tokio::task::block_in_place(|| match chat_default_target(&mut rt, chat).unwrap() {
                    TabTarget::Id {
                        id,
                    } => id,
                    TabTarget::Active => panic!("{chat} must get a tab id"),
                })
            }
        };
        let tab_a = target("chat-a").await;
        let tab_b = target("chat-b").await;
        let (hold_url, arrived_rx, release_tx, server) = held_request_server().await;
        let request = parse_browser_action_request(serde_json::json!({
            "target": {"type": "id", "id": tab_a},
            "page_context": "none",
            "continue_on_error": true,
            "steps": [
                {"action": "http_request", "url": hold_url},
                {"action": "switch_tab", "tab": {"type": "active"}},
                {"action": "page_content"},
            ],
        }))
        .unwrap();
        let batch = tokio::spawn({
            let runtime = runtime.clone();
            async move { execute_request_with_runtime(runtime, request, &ImagePolicy::default()).await }
        });
        tokio::time::timeout(Duration::from_secs(30), arrived_rx)
            .await
            .unwrap()
            .unwrap();
        {
            let mut rt = runtime.lock().await;
            let tab = tab_by_id(&rt, &tab_a).unwrap();
            rt.select_tab_after_close(&tab_a);
            assert!(tokio::task::block_in_place(|| tab.close(false).unwrap()));
        }
        release_tx.send(()).unwrap();
        let report = tokio::time::timeout(Duration::from_secs(60), batch)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        server.await.unwrap();
        for index in [1, 2] {
            let step = &report.steps[index];
            assert!(
                !step.ok && step.data.is_none(),
                "after its tab was closed elsewhere the batch must not switch or read: {step:?}"
            );
        }
        assert!(tab_by_id(&*runtime.lock().await, &tab_b).is_some());
        drop((ensured, runtime));
        assert!(
            crate::integrations::browser_runtime::project_browser::stop_project_runtime(
                &app, &scope
            )
            .await
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn interleaved_chat_batches_stay_on_their_own_tabs_with_real_chrome() {
        use crate::integrations::browser_controller::execute_request_with_runtime;
        use crate::integrations::browser_models::{parse_browser_action_request, TabTarget};
        use crate::integrations::browser_runtime::project_browser::chat_default_target;

        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let cache = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx).unwrap().cache_dir = cache.path().to_path_buf();
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let scope = ProjectBrowserScope::from_source_root(project.path()).unwrap();
        let ensured = ensure_project_runtime(
            &app,
            &scope,
            crate::integrations::browser_runtime::BrowserLaunchOptions::headless(true),
            Instant::now() + START_DEADLINE,
        )
        .await
        .unwrap();
        let runtime = ensured.runtime.clone();
        let (tab_a, tab_b) = {
            let mut rt = runtime.lock().await;
            tokio::task::block_in_place(|| {
                let TabTarget::Id {
                    id: a,
                } = chat_default_target(&mut rt, "chat-a").unwrap()
                else {
                    panic!("chat-a must get a tab id");
                };
                let TabTarget::Id {
                    id: b,
                } = chat_default_target(&mut rt, "chat-b").unwrap()
                else {
                    panic!("chat-b must get a tab id");
                };
                assert_ne!(a, b);
                for (id, marker) in [(&a, "page-a"), (&b, "page-b")] {
                    let tab = tab_by_id(&rt, id).unwrap();
                    tab.navigate_to(&format!("data:text/html,<p>{marker}</p>"))
                        .unwrap();
                    tab.wait_until_navigated().unwrap();
                }
                (a, b)
            })
        };

        let (hold_url, arrived_rx, release_tx, server) = held_request_server().await;

        let request_a = parse_browser_action_request(serde_json::json!({
            "target": {"type": "id", "id": tab_a},
            "page_context": "none",
            "continue_on_error": true,
            "steps": [
                {"action": "http_request", "url": hold_url},
                {"action": "page_content"},
                {"action": "close_tab"},
                {"action": "page_content"},
                {"action": "switch_tab", "tab": {"type": "active"}},
                {"action": "page_content"},
                {"action": "switch_tab", "tab": {"type": "id", "id": tab_a}},
            ],
        }))
        .unwrap();
        let batch_a = tokio::spawn({
            let runtime = runtime.clone();
            async move {
                execute_request_with_runtime(runtime, request_a, &ImagePolicy::default()).await
            }
        });
        tokio::time::timeout(Duration::from_secs(30), arrived_rx)
            .await
            .unwrap()
            .unwrap();

        let request_b = parse_browser_action_request(serde_json::json!({
            "target": {"type": "id", "id": tab_b},
            "page_context": "none",
            "steps": [{"action": "page_content"}],
        }))
        .unwrap();
        let report_b =
            execute_request_with_runtime(runtime.clone(), request_b, &ImagePolicy::default())
                .await
                .unwrap();
        assert!(report_b.ok, "{report_b:?}");
        assert!(report_b.steps[0].data.as_ref().unwrap()["html"]
            .as_str()
            .unwrap()
            .contains("page-b"));
        assert_eq!(
            runtime.lock().await.active_tab_target_id(),
            Some(tab_b.as_str()),
            "the peer batch moved the runtime's active tab"
        );

        release_tx.send(()).unwrap();
        let report_a = tokio::time::timeout(Duration::from_secs(60), batch_a)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        server.await.unwrap();
        assert!(
            report_a.steps[..3].iter().all(|step| step.ok),
            "{report_a:?}"
        );
        assert!(
            !report_a.steps[3].ok
                && report_a.steps[3].data.is_none()
                && report_a.steps[3]
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("No active tab")),
            "a step after closing the batch's own tab must not read a peer tab: {:?}",
            report_a.steps[3]
        );
        for index in [4, 5, 6] {
            let step = &report_a.steps[index];
            assert!(
                !step.ok && step.data.is_none(),
                "switching or reading after the batch's tab closed must not reach another tab: {step:?}"
            );
        }
        assert_eq!(
            runtime.lock().await.active_tab_target_id(),
            Some(tab_b.as_str()),
            "the closed batch must not take over the peer's tab"
        );
        let html = report_a.steps[1].data.as_ref().unwrap()["html"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            html.contains("page-a") && !html.contains("page-b"),
            "page_content must read the batch's own tab: {html}"
        );
        assert_eq!(
            report_a.tab_id, None,
            "closing its tab must not move the batch to a peer tab"
        );
        {
            let rt = runtime.lock().await;
            assert!(
                tab_by_id(&rt, &tab_a).is_none(),
                "implicit close_tab must close the batch's tab"
            );
            assert!(
                tab_by_id(&rt, &tab_b).is_some(),
                "the peer chat's tab must survive"
            );
        }
        drop((ensured, runtime));
        assert!(
            crate::integrations::browser_runtime::project_browser::stop_project_runtime(
                &app, &scope
            )
            .await
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn status_after_a_mutation_survives_a_runtime_replacement_with_real_chrome() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let cache = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx).unwrap().cache_dir = cache.path().to_path_buf();
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let scope = ProjectBrowserScope::from_source_root(project.path()).unwrap();
        let ensured = ensure_project_runtime(
            &app,
            &scope,
            crate::integrations::browser_runtime::BrowserLaunchOptions::headless(true),
            Instant::now() + START_DEADLINE,
        )
        .await
        .unwrap();
        let runtime_id = ensured.runtime_id.clone();
        let capture_now = || async {
            let mut rt = ensured.runtime.lock().await;
            capture_status(&mut rt, &runtime_id)
        };
        let (opened, active, capture) = {
            let mut rt = ensured.runtime.lock().await;
            let opened = tokio::task::block_in_place(|| {
                crate::integrations::browser_runtime::project_browser::open_project_tab(
                    &mut rt, None,
                )
                .unwrap()
            });
            let active = rt.get_active_tab().unwrap().get_target_id().to_string();
            (opened, active, capture_status(&mut rt, &runtime_id))
        };
        let opened_id = opened.get_target_id().to_string();
        let value = new_tab_status(&scope, capture, &opened_id).unwrap();
        assert_eq!(value["new_tab_id"], opened_id.as_str());
        ensured
            .runtime
            .lock()
            .await
            .select_tab_after_close(&opened_id);
        tokio::task::block_in_place(|| opened.close(false).unwrap());
        let gone = new_tab_status(&scope, capture_now().await, &opened_id).unwrap_err();
        assert_eq!(gone.status_code, StatusCode::CONFLICT, "{}", gone.message);
        assert!(gone.message.contains("closed"), "{}", gone.message);

        let capture = capture_now().await;
        ensured.runtime.lock().await.ownership.invalidate();
        let after = status_after_mutation(&app, &scope, capture)
            .await
            .expect("a dispatched mutation must not fail on its trailing status");
        assert_eq!(after["running"], false, "{after}");
        let replaced = new_tab_status(&scope, capture_now().await, &active).unwrap_err();
        assert_eq!(replaced.status_code, StatusCode::CONFLICT);
        assert!(
            replaced.message.contains("replaced"),
            "{}",
            replaced.message
        );

        drop(ensured);
        assert!(
            crate::integrations::browser_runtime::remove_browser_runtime(app.clone(), &runtime_id)
                .await
        );
    }

    async fn post_owned(
        router: axum::Router,
        path: &'static str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = router
            .oneshot(request(path, Body::from(body.to_string())))
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    struct LiveProject {
        _cache: tempfile::TempDir,
        _project: tempfile::TempDir,
        app: crate::app_state::AppState,
        router: axum::Router,
        scope: ProjectBrowserScope,
        ensured: crate::integrations::browser_runtime::project_browser::EnsuredProjectRuntime,
    }

    impl LiveProject {
        async fn start() -> Self {
            let cache = tempfile::tempdir().unwrap();
            let project = tempfile::tempdir().unwrap();
            let mut gcx = crate::global_context::tests::make_test_gcx().await;
            Arc::get_mut(&mut gcx).unwrap().cache_dir = cache.path().to_path_buf();
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().into()];
            let app = crate::app_state::AppState::from_gcx(gcx).await;
            let router = crate::http::routers::make_refact_http_server(app.clone());
            let scope = ProjectBrowserScope::from_source_root(project.path()).unwrap();
            let ensured = ensure_project_runtime(
                &app,
                &scope,
                crate::integrations::browser_runtime::BrowserLaunchOptions::headless(true),
                Instant::now() + START_DEADLINE,
            )
            .await
            .unwrap();
            Self {
                _cache: cache,
                _project: project,
                app,
                router,
                scope,
                ensured,
            }
        }

        async fn stop(self) {
            let Self {
                app,
                scope,
                ensured,
                ..
            } = self;
            drop(ensured);
            assert!(
                crate::integrations::browser_runtime::project_browser::stop_project_runtime(
                    &app, &scope
                )
                .await
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn new_tab_is_acknowledged_while_a_peer_holds_the_runtime_with_real_chrome() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let (hold_url, arrived_rx, release_tx, server) = held_request_server().await;

        // The new tab's page load is held, so the handler is inside its critical section; the
        // peer then queues on the runtime mutex and takes it the moment that section ends.
        let opening = tokio::spawn(post_owned(
            live.router.clone(),
            "tab",
            serde_json::json!({"op": "new", "url": hold_url}),
        ));
        tokio::time::timeout(Duration::from_secs(10), arrived_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(
            runtime.try_lock().is_err(),
            "the handler must hold the runtime while it opens the tab"
        );
        let mut peer = Box::pin(runtime.clone().lock_owned());
        assert!(futures::poll!(peer.as_mut()).is_pending());
        release_tx.send(()).unwrap();
        let held = tokio::time::timeout(Duration::from_secs(10), peer)
            .await
            .expect("the peer takes the runtime when the handler's critical section ends");
        let (status, created) = tokio::time::timeout(Duration::from_secs(10), opening)
            .await
            .expect("the response must not wait for the peer to release the runtime")
            .unwrap();
        assert!(
            runtime.try_lock().is_err(),
            "the peer still holds the runtime"
        );
        assert_eq!(
            status,
            StatusCode::OK,
            "a tab opened while a peer holds the runtime must be acknowledged: {created}"
        );
        let new_tab_id = created["new_tab_id"].as_str().unwrap().to_string();
        assert!(created.get("busy").is_none(), "{created}");
        assert!(created["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tab| tab["tab_id"] == new_tab_id.as_str()));
        assert!(held.find_tab(&new_tab_id).is_some());
        drop(held);
        server.await.unwrap();
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn mutation_responses_leave_foreign_tab_adoption_to_status_with_real_chrome() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let stray = {
            let rt = runtime.lock().await;
            tokio::task::block_in_place(|| rt.browser.new_tab().unwrap())
        };
        let stray_id = stray.get_target_id().to_string();
        let adopted = |rt: &BrowserRuntime| rt.adopted_tab_target_ids.contains(&stray_id);
        assert!(!adopted(&*runtime.lock().await));

        let (status, navigated) = post(
            &live.router,
            "navigate",
            serde_json::json!({"op": "reload"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{navigated}");
        let (status, opened) = post(&live.router, "tab", serde_json::json!({"op": "new"})).await;
        assert_eq!(status, StatusCode::OK, "{opened}");
        for response in [&navigated, &opened] {
            assert!(response["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .all(|tab| tab["tab_id"] != stray_id.as_str()));
        }
        assert!(
            !adopted(&*runtime.lock().await),
            "a mutation must not adopt tabs it did not open"
        );

        let (status, listed) = post(&live.router, "status", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert!(adopted(&*runtime.lock().await), "status adopts new tabs");
        assert!(listed["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tab| tab["tab_id"] == stray_id.as_str()));
        drop(stray);
        live.stop().await;
    }

    /// Serves a small page on every request until the test ends.
    async fn page_server() -> (u16, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    let _ = stream.read(&mut buffer).await;
                    let body = "<html><body><input id=i oninput=\"window.__typed=true\"><script>window.downs=0;window.ups=0;addEventListener('keydown',()=>downs++);addEventListener('keyup',()=>ups++)</script></body></html>";
                    let _ = stream
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                            .as_bytes(),
                        )
                        .await;
                });
            }
        });
        (port, server)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn a_page_that_does_not_paint_never_blocks_other_tabs_with_real_chrome() {
        use crate::integrations::browser_runtime::project_browser::open_project_tab;

        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let runtime_id = live.ensured.runtime_id.clone();
        let (port, server) = page_server().await;
        const BUSY_MS: u64 = 12_000;
        let (stuck, other) = {
            let mut rt = runtime.lock().await;
            tokio::task::block_in_place(|| {
                let stuck = open_project_tab(&mut rt, None).unwrap();
                let other = open_project_tab(&mut rt, None).unwrap();
                for (tab, host) in [(&stuck, "127.0.0.1"), (&other, "localhost")] {
                    tab.navigate_to(&format!("http://{host}:{port}/"))
                        .unwrap()
                        .wait_until_navigated()
                        .unwrap();
                }
                stuck
                    .evaluate(
                        &format!(
                            "setTimeout(() => {{ const end = Date.now() + {BUSY_MS}; while (Date.now() < end) {{}} }}, 0); 0"
                        ),
                        false,
                    )
                    .unwrap();
                (stuck, other)
            })
        };
        let busy_since = Instant::now();
        let (stuck_id, other_id) = (
            stuck.get_target_id().to_string(),
            other.get_target_id().to_string(),
        );

        let first = tokio::spawn(post_owned(
            live.router.clone(),
            "frame",
            serde_json::json!({"tab_id": stuck_id}),
        ));
        wait_for_gate_holder(&runtime, &stuck_id, GateUse::Frame).await;

        let started = Instant::now();
        assert!(
            runtime_lock_is_free(&runtime).await,
            "a frame capture must not hold the runtime lock"
        );
        let (status, second) = post(
            &live.router,
            "frame",
            serde_json::json!({"tab_id": stuck_id}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{second}");
        assert!(
            second["detail"].as_str().unwrap().contains("already"),
            "{second}"
        );
        let second_at = started.elapsed();
        let (status, frame) = post(
            &live.router,
            "frame",
            serde_json::json!({"tab_id": other_id}),
        )
        .await;
        let frame_at = started.elapsed();
        assert_eq!(status, StatusCode::OK, "{frame}");
        assert_eq!(frame["tab_id"], other_id.as_str());
        let (status, typed) = post(
            &live.router,
            "input",
            serde_json::json!({"runtime_id": runtime_id, "tab_id": other_id, "kind": "text", "text": "hi"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{typed}");
        let input_at = started.elapsed();
        let (status, listed) = post(&live.router, "status", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert!(listed.get("busy").is_none(), "{listed}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "other tabs must be served during the stall: second frame {second_at:?}, other frame {frame_at:?}, input {input_at:?}, status {:?}",
            started.elapsed()
        );
        assert!(
            busy_since.elapsed() < Duration::from_millis(BUSY_MS),
            "the stall ended before the other tabs were checked"
        );

        let (status, stalled) = first.await.unwrap();
        assert_eq!(status, StatusCode::CONFLICT, "{stalled}");
        assert_eq!(stalled["detail"], FRAME_BUSY);
        assert!(
            busy_since.elapsed() < RENDERER_DEADLINE + Duration::from_secs(2),
            "a stalled capture must give up after its short deadline, took {:?}",
            busy_since.elapsed()
        );

        let (status, pressed) = post(
            &live.router,
            "input",
            serde_json::json!({"runtime_id": runtime_id, "tab_id": stuck_id, "kind": "key", "key": "a"}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{pressed}");
        assert_eq!(pressed["detail"], INPUT_BUSY);
        assert!(
            busy_since.elapsed() < Duration::from_millis(BUSY_MS),
            "the key must have timed out during the stall"
        );

        tokio::time::sleep(Duration::from_millis(BUSY_MS).saturating_sub(busy_since.elapsed()))
            .await;
        let keys = tokio::task::block_in_place(|| {
            let read = || eval_string(&stuck, "String(window.downs) + '/' + String(window.ups)");
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let keys = read();
                if keys == "1/1" || Instant::now() >= deadline {
                    break keys;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        assert_eq!(
            keys, "1/1",
            "a timed-out key press is released once the page answers"
        );
        let (status, recovered) = post(
            &live.router,
            "frame",
            serde_json::json!({"tab_id": stuck_id}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{recovered}");
        server.abort();
        drop((stuck, other, runtime));
        live.stop().await;
    }

    async fn gate_of(runtime: &RuntimeArc, tab_id: &str) -> Arc<TabGate> {
        runtime.lock().await.tab_gate(tab_id)
    }

    async fn wait_for_gate_holder(runtime: &RuntimeArc, tab_id: &str, holder: GateUse) {
        let gate = gate_of(runtime, tab_id).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while gate.holder() != Some(holder) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the tab's gate must be taken");
    }

    async fn wait_for_dispatches(gate: &TabGate, count: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while gate.waiting_dispatches() < count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dispatchers must reach the held gate before the test continues");
    }

    async fn runtime_lock_is_free(runtime: &RuntimeArc) -> bool {
        tokio::time::timeout(Duration::from_millis(500), async {
            while runtime.try_lock().is_err() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .is_ok()
    }

    async fn open_served_tab(runtime: &RuntimeArc, url: String) -> Arc<headless_chrome::Tab> {
        let mut rt = runtime.lock().await;
        tokio::task::block_in_place(|| {
            let tab = crate::integrations::browser_runtime::project_browser::open_project_tab(
                &mut rt, None,
            )
            .unwrap();
            tab.navigate_to(&url)
                .unwrap()
                .wait_until_navigated()
                .unwrap();
            tab
        })
    }

    fn start_busy_loop(tab: &headless_chrome::Tab, busy_ms: u64) -> Instant {
        tokio::task::block_in_place(|| {
            tab.evaluate(
                &format!(
                    "setTimeout(() => {{ const end = Date.now() + {busy_ms}; while (Date.now() < end) {{}} }}, 0); 0"
                ),
                false,
            )
            .unwrap();
        });
        Instant::now()
    }

    fn frame_pixels(frame: &serde_json::Value) -> (u32, u32) {
        let bytes = base64::prelude::BASE64_STANDARD
            .decode(frame["data"].as_str().unwrap())
            .unwrap();
        let image = image::load_from_memory(&bytes).unwrap();
        (image.width(), image.height())
    }

    fn inner_width(tab: &headless_chrome::Tab) -> String {
        tokio::task::block_in_place(|| eval_string(tab, "String(window.innerWidth)"))
    }

    async fn tool_batch(
        runtime: &RuntimeArc,
        tab_id: &str,
        steps: serde_json::Value,
    ) -> tokio::task::JoinHandle<crate::integrations::browser_models::ExecutionReport> {
        use crate::integrations::browser_controller::execute_request_with_runtime;
        use crate::integrations::browser_models::parse_browser_action_request;
        let request = parse_browser_action_request(serde_json::json!({
            "target": {"type": "id", "id": tab_id},
            "page_context": "none",
            "steps": steps,
        }))
        .unwrap();
        let runtime = runtime.clone();
        tokio::spawn(async move {
            execute_request_with_runtime(runtime, request, &ImagePolicy::default())
                .await
                .unwrap()
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn viewport_writer_waits_for_a_chrome_tab_discovered_after_its_first_snapshot() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let tab = runtime.lock().await.get_active_tab().unwrap();
        let old_gate = gate_of(&runtime, tab.get_target_id()).await;
        let old_guard = old_gate.acquire(GateUse::Frame, Instant::now()).unwrap();
        let writer = tool_batch(
            &runtime,
            tab.get_target_id(),
            serde_json::json!([
                {"action": "set_viewport", "width": 501, "height": 401}
            ]),
        )
        .await;
        wait_for_dispatches(&old_gate, 1).await;
        // Bypass engine adoption: this is a target Chrome knows about but the writer has not seen.
        let late = {
            let rt = runtime.lock().await;
            tokio::task::block_in_place(|| rt.open_tab()).unwrap()
        };
        let gate = gate_of(&runtime, late.get_target_id()).await;
        let current_width = inner_width(&late);
        assert_ne!(current_width, "501");
        let hold = gate.acquire(GateUse::Frame, Instant::now()).unwrap();
        drop(old_guard);
        wait_for_dispatches(&gate, 1).await;
        assert_eq!(inner_width(&late), current_width);
        drop(hold);
        let report = writer.await.unwrap();
        assert!(report.ok, "{report:?}");
        assert_eq!(inner_width(&late), "501");
        drop((tab, late, runtime));
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn fill_waits_before_stealing_focus_and_types_into_its_own_field() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let (port, server) = page_server().await;
        let tab = open_served_tab(&runtime, format!("http://127.0.0.1:{port}/")).await;
        tokio::task::block_in_place(|| {
            tab.evaluate(
                "i.insertAdjacentHTML('afterend', '<input id=j>'); i.focus(); 'ready'",
                false,
            )
            .unwrap()
        });
        let gate = gate_of(&runtime, tab.get_target_id()).await;
        let held = gate.acquire(GateUse::Input, Instant::now()).unwrap();
        let fill = tool_batch(&runtime, tab.get_target_id(), serde_json::json!([
            {"action": "fill", "locator": {"by": "css", "value": "#j"}, "text": "tool", "verify": true}
        ])).await;
        wait_for_dispatches(&gate, 1).await;
        // The owner types into a different field. Fill must not have run its focus/select yet.
        tokio::task::block_in_place(|| {
            assert_eq!(eval_string(&tab, "document.activeElement.id"), "i");
            Keyboard::new(CdpKeyboardDispatcher::new(&tab))
                .insert_text("human")
                .unwrap();
        });
        drop(held);
        let report = fill.await.unwrap();
        assert!(report.ok, "{report:?}");
        tokio::task::block_in_place(|| {
            assert_eq!(
                eval_string(&tab, "[i.value,j.value].join('|')"),
                "human|tool"
            )
        });
        server.abort();
        drop((tab, runtime));
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn locator_keyboard_actions_retain_focus_while_a_peer_queues() {
        use crate::integrations::browser_controller::focus_for_keyboard;
        use crate::integrations::browser_models::BrowserLocator;
        use refact_browser::tab_gate::enter_step_gates;

        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let (port, server) = page_server().await;
        let tab = open_served_tab(&runtime, format!("http://127.0.0.1:{port}/")).await;
        let id = tab.get_target_id().clone();
        let gate = gate_of(&runtime, &id).await;
        let world = runtime.lock().await.world_manager.clone();
        tokio::task::block_in_place(|| {
            tab.evaluate(
                "i.insertAdjacentHTML('afterend', '<input id=j>'); i.focus()",
                false,
            )
            .unwrap()
        });
        for (action, text) in [
            ("insert_text", "inserted"),
            ("press_sequentially", "pressed"),
        ] {
            tokio::task::block_in_place(|| {
                tab.evaluate("i.value=''; j.value=''; i.focus()", false)
                    .unwrap()
            });
            let tab_copy = tab.clone();
            let gate_copy = gate.clone();
            let world_copy = world.clone();
            let (focused_tx, focused_rx) = tokio::sync::oneshot::channel();
            let (type_tx, type_rx) = std::sync::mpsc::channel();
            let work = tokio::task::spawn_blocking(move || {
                let tab = tab_copy;
                let _scope = enter_step_gates(
                    vec![(tab.get_target_id().clone(), gate_copy)],
                    Instant::now() + Duration::from_secs(12),
                );
                let locator: BrowserLocator =
                    serde_json::from_value(serde_json::json!({"by":"css", "value":"#j"})).unwrap();
                let hold = focus_for_keyboard(
                    &tab,
                    &world_copy,
                    0,
                    &locator,
                    action,
                    None,
                    &mut Vec::new(),
                    &ImagePolicy::default(),
                )
                .unwrap();
                assert_eq!(eval_string(&tab, "document.activeElement.id"), "j");
                focused_tx.send(()).unwrap();
                type_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                let mut keyboard = Keyboard::new(CdpKeyboardDispatcher::new(&tab));
                match action {
                    "insert_text" => keyboard.insert_text(text).unwrap(),
                    _ => keyboard.press_sequentially(text, None).unwrap(),
                }
                drop(hold);
            });
            tokio::time::timeout(Duration::from_secs(8), focused_rx)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                gate.holder(),
                Some(GateUse::Tool),
                "{action} must still own focus before dispatch"
            );
            let competing = tool_batch(&runtime, &id, serde_json::json!([
                {"action":"fill", "locator":{"by":"css", "value":"#i"}, "text":"peer", "verify":true}
            ])).await;
            wait_for_dispatches(&gate, 1).await;
            type_tx.send(()).unwrap();
            work.await.unwrap();
            assert!(competing.await.unwrap().ok);
            assert_eq!(
                tokio::task::block_in_place(|| eval_string(&tab, "[i.value,j.value].join('|')")),
                format!("peer|{text}")
            );
        }
        server.abort();
        drop((tab, runtime));
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn passive_wait_does_not_restore_a_mouse_snapshot_and_two_presses_compose() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let (port, server) = page_server().await;
        let tab = open_served_tab(&runtime, format!("http://127.0.0.1:{port}/")).await;
        let id = tab.get_target_id();
        tokio::task::block_in_place(|| {
            tab.evaluate("window.events=[]; addEventListener('mousedown', e => events.push([e.clientX,e.clientY,e.buttons])); addEventListener('contextmenu', e => e.preventDefault()); 'ready'", false).unwrap()
        });
        let moved = tool_batch(
            &runtime,
            id,
            serde_json::json!([{"action":"mouse_move", "x":50, "y":60}]),
        )
        .await
        .await
        .unwrap();
        assert!(moved.ok, "{moved:?}");
        let passive = tool_batch(&runtime, id, serde_json::json!([
            {"action":"wait_for_function", "expression":"() => { document.documentElement.dataset.waiting = 'yes'; return document.documentElement.dataset.finish === 'yes'; }", "timeout_ms":15000}
        ])).await;
        tokio::time::timeout(Duration::from_secs(10), async {
            while tokio::task::block_in_place(|| {
                eval_string(&tab, "document.documentElement.dataset.waiting")
            }) != "yes"
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let gate = gate_of(&runtime, id).await;
        let held = gate.acquire(GateUse::Frame, Instant::now()).unwrap();
        let left = tool_batch(
            &runtime,
            id,
            serde_json::json!([{"action":"mouse_down", "button":"left"}]),
        )
        .await;
        let right = tool_batch(
            &runtime,
            id,
            serde_json::json!([{"action":"mouse_down", "button":"right"}]),
        )
        .await;
        wait_for_dispatches(&gate, 2).await;
        drop(held);
        for press in [left, right] {
            let report = press.await.unwrap();
            assert!(report.ok, "{report:?}");
        }
        tokio::task::block_in_place(|| {
            tab.evaluate("document.documentElement.dataset.finish='yes'", false)
                .unwrap()
        });
        let waited = passive.await.unwrap();
        assert!(waited.ok, "{waited:?}");
        let middle = tool_batch(
            &runtime,
            id,
            serde_json::json!([{"action":"mouse_down", "button":"middle"}]),
        )
        .await
        .await
        .unwrap();
        assert!(middle.ok, "{middle:?}");
        let logged = tokio::task::block_in_place(|| eval_string(&tab, "JSON.stringify(events)"));
        assert!(
            logged == "[[50,60,1],[50,60,3],[50,60,7]]"
                || logged == "[[50,60,2],[50,60,3],[50,60,7]]",
            "event sequence: {logged}"
        );
        server.abort();
        drop((tab, runtime));
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn project_tool_open_tab_keeps_old_viewport_and_input_live_while_loading() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let (port, server) = page_server().await;
        let old = open_served_tab(&runtime, format!("http://127.0.0.1:{port}/")).await;
        let id = old.get_target_id();
        let original_ua = tokio::task::block_in_place(|| eval_string(&old, "navigator.userAgent"));
        let (status, result) = post(
            &live.router,
            "resize",
            serde_json::json!({"tab_id":id,"width":630,"height":470}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        let old_gate = gate_of(&runtime, id).await;
        let held = old_gate.acquire(GateUse::Frame, Instant::now()).unwrap();
        let (url, arrived, release, holding_server) = held_request_server().await;
        const OWNER: &str = "opener-chat";
        const PEER: &str = "peer-chat";
        const ORIGINAL_OWNER: &str = "original-tab-chat";
        {
            let mut rt = runtime.lock().await;
            refact_browser::record_chat_tab_use(
                &mut rt.tab_meta,
                ORIGINAL_OWNER,
                std::slice::from_ref(id),
                Some(id),
                None,
                1,
            );
        }
        let work = runtime.clone();
        let gcx = live.app.gcx.clone();
        let request =
            crate::integrations::browser_models::parse_browser_action_request(serde_json::json!({
                "target":{"type":"id","id":id}, "page_context":"none",
                "steps":[{"action":"open_tab","url":url}]
            }))
            .unwrap();
        let opening = tokio::spawn(async move {
            crate::integrations::browser_controller::execute_request_with_project_chat(
                work,
                request,
                &ImagePolicy::default(),
                gcx,
                Some(OWNER),
            )
            .await
            .unwrap()
        });
        tokio::time::timeout(Duration::from_secs(5), arrived)
            .await
            .unwrap()
            .unwrap();
        let (pending_id, peer_id) = {
            use crate::integrations::browser_models::TabTarget;
            use crate::integrations::browser_runtime::project_browser::chat_default_target;
            let mut rt = runtime.lock().await;
            let pending = rt.active_tab_target_id().unwrap().to_owned();
            assert_ne!(&pending, id);
            let TabTarget::Id {
                id: peer,
            } = tokio::task::block_in_place(|| chat_default_target(&mut rt, PEER)).unwrap()
            else {
                panic!("the peer must resolve to a concrete tab");
            };
            assert_ne!(peer, pending, "peer must not claim the loading tool tab");
            assert_ne!(&peer, id);
            assert_eq!(rt.tab_meta[&pending].opened_by_chat.as_deref(), Some(OWNER));
            assert_eq!(rt.tab_meta[&peer].opened_by_chat.as_deref(), Some(PEER));
            assert_eq!(
                rt.tab_meta[id].opened_by_chat.as_deref(),
                Some(ORIGINAL_OWNER)
            );
            (pending, peer)
        };
        drop(held);
        assert!(!opening.is_finished());
        let (status, clicked) = post(
            &live.router,
            "input",
            serde_json::json!({
                "runtime_id": live.ensured.runtime_id, "tab_id":id,"kind":"click","x":20,"y":14
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{clicked}");
        let (status, typed) = post(
            &live.router,
            "input",
            serde_json::json!({
                "runtime_id": live.ensured.runtime_id, "tab_id":id,"kind":"text","text":"human"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{typed}");
        release.send(()).unwrap();
        holding_server.await.unwrap();
        let plain = opening.await.unwrap();
        assert!(plain.ok, "{plain:?}");
        let plain_id = plain.tab_id.unwrap();
        assert_eq!(plain_id, pending_id);
        assert_ne!(plain_id, peer_id);
        let (status, resized) = post(
            &live.router,
            "resize",
            serde_json::json!({"tab_id":plain_id,"width":570,"height":420}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resized}");
        assert_eq!(resized["applied"], true, "{resized}");
        assert_eq!(inner_width(&old), "630");
        assert_eq!(
            tokio::task::block_in_place(|| eval_string(&old, "i.value")),
            "human"
        );
        let device = tool_batch(
            &runtime,
            id,
            serde_json::json!([{"action":"open_tab","device":"Desktop Chrome"}]),
        )
        .await
        .await
        .unwrap();
        assert!(device.ok, "{device:?}");
        let device_id = device.tab_id.unwrap();
        let (status, frame) = post(
            &live.router,
            "frame",
            serde_json::json!({"tab_id":device_id}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{frame}");
        assert_eq!(
            frame["viewport"],
            serde_json::json!({"width":1280.0,"height":720.0})
        );
        let rt = runtime.lock().await;
        assert!(rt.context_state.viewport.is_none() && rt.context_state.user_agent.is_none());
        let plain = rt.find_tab(&plain_id).unwrap();
        let device = rt.find_tab(&device_id).unwrap();
        drop(rt);
        assert_eq!(inner_width(&plain), "570");
        assert_eq!(inner_width(&old), "630");
        tokio::task::block_in_place(|| {
            assert_eq!(eval_string(&old, "navigator.userAgent"), original_ua);
            assert_eq!(eval_string(&plain, "navigator.userAgent"), original_ua);
            assert_eq!(
                eval_string(&device, "navigator.userAgent"),
                refact_browser::devices::lookup("Desktop Chrome")
                    .unwrap()
                    .user_agent
            );
        });
        server.abort();
        drop((old, plain, device, runtime));
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn navigation_acknowledges_dispatch_even_when_runtime_status_is_contended() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let (tab, fallback) = {
            let mut rt = runtime.lock().await;
            let tab = rt.get_active_tab().unwrap();
            (tab, capture_status(&mut rt, &live.ensured.runtime_id))
        };
        let (port, server) = page_server().await;
        let url = format!("http://127.0.0.1:{port}/accepted");
        tokio::task::block_in_place(|| {
            tab.navigate_to(&url)
                .unwrap()
                .wait_until_navigated()
                .unwrap()
        });
        assert_eq!(tab.get_url(), url);
        let locked = runtime.lock().await;
        let reply = tokio::time::timeout(
            Duration::from_secs(6),
            navigation_response(&live.app, &live.scope, &runtime, fallback),
        )
        .await
        .unwrap();
        assert_eq!(reply.status(), StatusCode::OK);
        let response: serde_json::Value =
            serde_json::from_slice(&to_bytes(reply.into_body()).await.unwrap()).unwrap();
        assert_eq!(response["runtime_id"], live.ensured.runtime_id);
        assert_eq!(response["url"], url);
        assert_eq!(response["active_tab"], tab.get_target_id().as_str());
        drop(locked);
        drop((tab, runtime));
        server.abort();
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn acknowledged_navigation_never_returns_the_replaced_generation_inventory() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let (port, server) = page_server().await;
        let old = live.ensured.runtime.clone();
        let old_id = live.ensured.runtime_id.clone();
        let (tab, fallback) = {
            let mut rt = old.lock().await;
            (
                rt.get_active_tab().unwrap(),
                capture_status(&mut rt, &old_id),
            )
        };
        let url = format!("http://127.0.0.1:{port}/old-generation");
        tokio::task::block_in_place(|| {
            tab.navigate_to(&url)
                .unwrap()
                .wait_until_navigated()
                .unwrap()
        });
        assert_eq!(tab.get_url(), url);
        old.lock().await.ownership.invalidate();
        let retired_capture = capture_status(&mut *old.lock().await, &old_id);
        let absent = navigation_response(&live.app, &live.scope, &old, retired_capture).await;
        assert_eq!(absent.status(), StatusCode::OK);
        let empty: serde_json::Value =
            serde_json::from_slice(&to_bytes(absent.into_body()).await.unwrap()).unwrap();
        assert_eq!(empty["running"], false);
        assert!(empty.get("runtime_id").is_none());
        assert_eq!(empty["tabs"], serde_json::json!([]));
        assert!(
            crate::integrations::browser_runtime::remove_browser_runtime(live.app.clone(), &old_id)
                .await
        );
        let replacement = ensure_project_runtime(
            &live.app,
            &live.scope,
            crate::integrations::browser_runtime::BrowserLaunchOptions::headless(true),
            Instant::now() + START_DEADLINE,
        )
        .await
        .unwrap();
        let new_id = replacement.runtime_id.clone();
        assert_ne!(old_id, new_id);
        let replacement_tab = replacement.runtime.lock().await.get_active_tab().unwrap();
        let sentinel = format!("http://127.0.0.1:{port}/replacement-sentinel");
        tokio::task::block_in_place(|| {
            replacement_tab
                .navigate_to(&sentinel)
                .unwrap()
                .wait_until_navigated()
                .unwrap()
        });
        assert_eq!(replacement_tab.get_url(), sentinel);
        let fallback_copy = capture_status(&mut *old.lock().await, &old_id);
        let held = replacement.runtime.lock().await;
        let busy = navigation_response(&live.app, &live.scope, &old, fallback_copy).await;
        drop(held);
        assert_eq!(busy.status(), StatusCode::OK);
        let busy: serde_json::Value =
            serde_json::from_slice(&to_bytes(busy.into_body()).await.unwrap()).unwrap();
        assert_eq!(busy["runtime_id"], new_id);
        assert_eq!(busy["running"], true);
        assert_eq!(busy["busy"], true);
        assert_eq!(busy["tabs"], serde_json::json!([]));
        assert_eq!(replacement_tab.get_url(), sentinel);
        let fallback_copy = capture_status(&mut *old.lock().await, &old_id);
        let registry = live.app.integrations.browser_runtimes.lock().await;
        let unknown = navigation_response(&live.app, &live.scope, &old, fallback_copy).await;
        drop(registry);
        assert_eq!(unknown.status(), StatusCode::OK);
        let unknown: serde_json::Value =
            serde_json::from_slice(&to_bytes(unknown.into_body()).await.unwrap()).unwrap();
        assert_eq!(unknown["running"], true);
        assert_eq!(unknown["busy"], true);
        assert_eq!(unknown["tabs"], serde_json::json!([]));
        assert!(unknown.get("runtime_id").is_none());
        let reply = navigation_response(&live.app, &live.scope, &old, fallback).await;
        assert_eq!(reply.status(), StatusCode::OK);
        let status: serde_json::Value =
            serde_json::from_slice(&to_bytes(reply.into_body()).await.unwrap()).unwrap();
        assert_eq!(status["runtime_id"], new_id);
        assert_eq!(
            status["active_tab"],
            replacement_tab.get_target_id().as_str()
        );
        assert_eq!(
            status["url"], sentinel,
            "navigation must not be replayed on replacement"
        );
        assert!(status["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["tab_id"] != tab.get_target_id().as_str()));
        server.abort();
        drop((old, tab, replacement, replacement_tab));
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn completed_renderer_work_is_fenced_against_tab_close_and_generation_replacement() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let (tab, fence) = {
            let rt = runtime.lock().await;
            let tab = rt.get_active_tab().unwrap();
            (tab.clone(), TabFence::new(&rt, tab))
        };
        let id = tab.get_target_id();
        assert!(
            blocking_within(RENDERER_DEADLINE, || apply_pane_viewport(&tab, (630, 470))).is_ok()
        );
        assert_eq!(inner_width(&tab), "630");
        tokio::task::block_in_place(|| assert!(tab.close(false).unwrap()));
        runtime.lock().await.select_tab_after_close(id);
        let closed = fence
            .lock(&runtime, "; the pane size was applied")
            .await
            .err()
            .unwrap();
        assert_eq!(closed.status_code, StatusCode::GONE);
        assert!(closed.message.ends_with("the pane size was applied"));
        assert_eq!(runtime.lock().await.pane.tab_size(id), None);
        runtime.lock().await.ownership.invalidate();
        let retired = fence
            .lock(&runtime, "; the input was sent")
            .await
            .err()
            .unwrap();
        assert_eq!(retired.status_code, StatusCode::CONFLICT);
        assert!(retired.message.ends_with("the input was sent"));
        let LiveProject {
            app,
            ensured,
            ..
        } = live;
        let runtime_id = ensured.runtime_id.clone();
        drop((tab, runtime, ensured));
        assert!(
            crate::integrations::browser_runtime::remove_browser_runtime(app, &runtime_id).await
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn frame_reports_the_current_document_title_after_navigation() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let (port, server) = page_server().await;
        let tab = open_served_tab(&runtime, format!("http://127.0.0.1:{port}/title")).await;
        let id = tab.get_target_id();
        tokio::task::block_in_place(|| {
            tab.evaluate("document.title='Round Eight Title'", false)
                .unwrap()
        });
        let (status, frame) = post(&live.router, "frame", serde_json::json!({"tab_id":id})).await;
        assert_eq!(status, StatusCode::OK, "{frame}");
        assert_eq!(frame["title"], "Round Eight Title");
        assert_eq!(frame["url"], format!("http://127.0.0.1:{port}/title"));
        server.abort();
        drop((tab, runtime));
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn same_tab_view_changes_wait_for_the_tab_gate_with_real_chrome() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let (port, server) = page_server().await;
        let tab = open_served_tab(&runtime, format!("http://127.0.0.1:{port}/")).await;
        let tab_id = tab.get_target_id().to_string();
        let frame_of = |router: axum::Router, tab_id: String| async move {
            let (status, frame) = post(
                &router,
                "frame",
                serde_json::json!({"tab_id": tab_id, "max_side": 1920}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{frame}");
            frame
        };

        let (status, resized) = post(
            &live.router,
            "resize",
            serde_json::json!({"width": 640, "height": 480, "tab_id": tab_id}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resized}");
        let frame = frame_of(live.router.clone(), tab_id.clone()).await;
        assert_eq!(frame_pixels(&frame), (640, 480), "{}", frame["viewport"]);
        assert_eq!(
            frame["viewport"],
            serde_json::json!({"width": 640.0, "height": 480.0})
        );

        let held = gate_of(&runtime, &tab_id)
            .await
            .acquire(GateUse::Frame, Instant::now())
            .unwrap();
        let resize = tokio::spawn(post_owned(
            live.router.clone(),
            "resize",
            serde_json::json!({"width": 800, "height": 600, "tab_id": tab_id}),
        ));
        let tool = tool_batch(
            &runtime,
            &tab_id,
            serde_json::json!([{"action": "set_viewport", "width": 500, "height": 400}]),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(
            inner_width(&tab),
            "640",
            "neither the pane resize nor the tool viewport may apply while a frame holds the tab"
        );
        assert!(!resize.is_finished() && !tool.is_finished());
        drop(held);
        let (status, resized) = resize.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{resized}");
        let report = tool.await.unwrap();
        assert!(report.ok, "{report:?}");
        assert_eq!(
            inner_width(&tab),
            "500",
            "the tool viewport wins over the pane"
        );

        let frame = frame_of(live.router.clone(), tab_id.clone()).await;
        assert_eq!(frame_pixels(&frame), (500, 400), "{}", frame["viewport"]);
        assert_eq!(
            frame["viewport"],
            serde_json::json!({"width": 500.0, "height": 400.0})
        );
        let (status, resized) = post(
            &live.router,
            "resize",
            serde_json::json!({"width": 700, "height": 500, "tab_id": tab_id}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resized}");
        assert_eq!(resized["applied"], false);
        assert_eq!(resized["reason"], "tool_viewport_override_active");
        frame_of(live.router.clone(), tab_id.clone()).await;
        assert_eq!(
            inner_width(&tab),
            "500",
            "a frame never overrides a tool viewport"
        );
        server.abort();
        drop((tab, runtime));
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn tool_dispatches_wait_for_the_tab_gate_but_passive_waits_do_not_with_real_chrome() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let runtime_id = live.ensured.runtime_id.clone();
        let (port, server) = page_server().await;
        let tab = open_served_tab(&runtime, format!("http://127.0.0.1:{port}/")).await;
        let other = open_served_tab(&runtime, format!("http://localhost:{port}/")).await;
        let (tab_id, other_id) = (
            tab.get_target_id().to_string(),
            other.get_target_id().to_string(),
        );

        let held = gate_of(&runtime, &tab_id)
            .await
            .acquire(GateUse::Input, Instant::now())
            .unwrap();
        let navigation = tool_batch(
            &runtime,
            &tab_id,
            serde_json::json!([{"action": "navigate", "url": format!("http://127.0.0.1:{port}/next")}]),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let started = Instant::now();
        for (path, body) in [
            ("frame", serde_json::json!({"tab_id": other_id})),
            (
                "input",
                serde_json::json!({"runtime_id": runtime_id, "tab_id": other_id, "kind": "text", "text": "x"}),
            ),
            ("status", serde_json::json!({})),
        ] {
            let (status, value) = post(&live.router, path, body).await;
            assert_eq!(status, StatusCode::OK, "{path} {value}");
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "other tabs must be served while one tab's gate is held, took {:?}",
            started.elapsed()
        );
        assert!(
            runtime_lock_is_free(&runtime).await,
            "the runtime lock must stay free"
        );
        assert!(!navigation.is_finished());
        assert!(
            !tab.get_url().ends_with("/next"),
            "a tool navigation must wait for the tab's gate"
        );
        drop(held);
        let report = tokio::time::timeout(Duration::from_secs(30), navigation)
            .await
            .unwrap()
            .unwrap();
        assert!(report.ok, "{report:?}");
        assert!(tab.get_url().ends_with("/next"), "{}", tab.get_url());

        let waiting = tool_batch(
            &runtime,
            &tab_id,
            serde_json::json!([{"action": "wait_for_function", "expression": "() => window.__typed === true", "timeout_ms": 15000}]),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let started = Instant::now();
        let (status, frame) =
            post(&live.router, "frame", serde_json::json!({"tab_id": tab_id})).await;
        assert_eq!(status, StatusCode::OK, "{frame}");
        for body in [
            serde_json::json!({"runtime_id": runtime_id, "tab_id": tab_id, "kind": "click", "x": 20, "y": 15}),
            serde_json::json!({"runtime_id": runtime_id, "tab_id": tab_id, "kind": "text", "text": "signed in"}),
        ] {
            let (status, typed) = post(&live.router, "input", body).await;
            assert_eq!(status, StatusCode::OK, "{typed}");
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the Browser view must work during a tool wait, took {:?}",
            started.elapsed()
        );
        let report = tokio::time::timeout(Duration::from_secs(20), waiting)
            .await
            .unwrap()
            .unwrap();
        assert!(report.ok, "the wait must see the user's input: {report:?}");
        server.abort();
        drop((tab, other, runtime));
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn renderer_failures_report_closed_tabs_and_replaced_runtimes_with_real_chrome() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let runtime_id = live.ensured.runtime_id.clone();
        let (port, server) = page_server().await;
        let closing = open_served_tab(&runtime, format!("http://127.0.0.1:{port}/")).await;
        let replacing = open_served_tab(&runtime, format!("http://localhost:{port}/")).await;
        let (closing_id, replacing_id) = (
            closing.get_target_id().to_string(),
            replacing.get_target_id().to_string(),
        );

        start_busy_loop(&closing, 10_000);
        let frame = tokio::spawn(post_owned(
            live.router.clone(),
            "frame",
            serde_json::json!({"tab_id": closing_id}),
        ));
        wait_for_gate_holder(&runtime, &closing_id, GateUse::Frame).await;
        let (status, closed) = post(
            &live.router,
            "tab",
            serde_json::json!({"op": "close", "tab_id": closing_id}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{closed}");
        let (status, stalled) = frame.await.unwrap();
        assert_eq!(
            status,
            StatusCode::GONE,
            "a capture of a tab closed meanwhile is gone: {stalled}"
        );

        start_busy_loop(&replacing, 10_000);
        let frame = tokio::spawn(post_owned(
            live.router.clone(),
            "frame",
            serde_json::json!({"tab_id": replacing_id}),
        ));
        wait_for_gate_holder(&runtime, &replacing_id, GateUse::Frame).await;
        runtime.lock().await.ownership.invalidate();
        let (status, stalled) = frame.await.unwrap();
        assert_eq!(status, StatusCode::CONFLICT, "{stalled}");
        assert!(
            stalled["detail"].as_str().unwrap().contains("replaced"),
            "{stalled}"
        );
        server.abort();
        let LiveProject {
            app,
            ensured,
            ..
        } = live;
        drop((closing, replacing, runtime, ensured));
        assert!(
            crate::integrations::browser_runtime::remove_browser_runtime(app, &runtime_id).await
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn frames_during_a_browser_view_navigation_are_never_gateway_errors_with_real_chrome() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let live = LiveProject::start().await;
        let runtime = live.ensured.runtime.clone();
        let tab = {
            let mut rt = runtime.lock().await;
            tokio::task::block_in_place(|| {
                crate::integrations::browser_runtime::project_browser::open_project_tab(
                    &mut rt, None,
                )
                .unwrap()
            })
        };
        let tab_id = tab.get_target_id().to_string();
        let (hold_url, arrived_rx, release_tx, server) = held_request_server().await;
        let navigation = tokio::spawn(post_owned(
            live.router.clone(),
            "navigate",
            serde_json::json!({"url": hold_url, "tab_id": tab_id}),
        ));
        let mut arrived = Box::pin(arrived_rx);
        let mut release = Some(release_tx);
        let mut released_at = None;
        let mut statuses = Vec::new();
        let mut committed = false;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(30) {
            if released_at.is_none() && futures::poll!(arrived.as_mut()).is_ready() {
                released_at.get_or_insert_with(|| Instant::now() + Duration::from_millis(300));
            }
            if released_at.is_some_and(|at| Instant::now() >= at) {
                if let Some(release) = release.take() {
                    release.send(()).unwrap();
                }
            }
            let (status, frame) =
                post(&live.router, "frame", serde_json::json!({"tab_id": tab_id})).await;
            statuses.push(status);
            assert!(
                status == StatusCode::OK || status == StatusCode::CONFLICT,
                "a frame during a navigation must be 200 or 409: {status} {frame}"
            );
            if status == StatusCode::OK && frame["url"] == hold_url.as_str() {
                committed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            committed,
            "frames must succeed once the page commits: {statuses:?}"
        );
        let (status, navigated) = navigation.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{navigated}");
        server.await.unwrap();
        drop((tab, runtime));
        live.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn popup_wait_ignores_tabs_opened_by_others_with_real_chrome() {
        use crate::integrations::browser_controller::execute_request_with_runtime;
        use crate::integrations::browser_models::{parse_browser_action_request, TabTarget};
        use crate::integrations::browser_runtime::project_browser::chat_default_target;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let cache = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx).unwrap().cache_dir = cache.path().to_path_buf();
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let scope = ProjectBrowserScope::from_source_root(project.path()).unwrap();
        let ensured = ensure_project_runtime(
            &app,
            &scope,
            crate::integrations::browser_runtime::BrowserLaunchOptions::headless(true),
            Instant::now() + START_DEADLINE,
        )
        .await
        .unwrap();
        let runtime = ensured.runtime.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (arrived_tx, mut arrived_rx) = tokio::sync::mpsc::unbounded_channel();
        let release = Arc::new(tokio::sync::Notify::new());
        let server = tokio::spawn({
            let release = release.clone();
            async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let arrived_tx = arrived_tx.clone();
                    let release = release.clone();
                    tokio::spawn(async move {
                        let mut buffer = [0u8; 4096];
                        let read = stream.read(&mut buffer).await.unwrap_or(0);
                        let held =
                            String::from_utf8_lossy(&buffer[..read]).starts_with("GET /hold");
                        let body = if held {
                            let released = release.notified();
                            let _ = arrived_tx.send(());
                            released.await;
                            "ok"
                        } else {
                            "<button id=open onclick=\"fetch('/hold').then(()=>window.open('about:blank'))\">open</button>"
                        };
                        let _ = stream
                            .write_all(
                                format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                    body.len()
                                )
                                .as_bytes(),
                            )
                            .await;
                    });
                }
            }
        });
        let page = format!("{origin}/page");
        let tab_a = {
            let mut rt = runtime.lock().await;
            tokio::task::block_in_place(|| {
                let TabTarget::Id {
                    id,
                } = chat_default_target(&mut rt, "chat-a").unwrap()
                else {
                    panic!("chat-a must get a tab id");
                };
                let tab = tab_by_id(&rt, &id).unwrap();
                tab.navigate_to(&page).unwrap();
                tab.wait_until_navigated().unwrap();
                id
            })
        };
        let request = parse_browser_action_request(serde_json::json!({
            "target": {"type": "id", "id": tab_a},
            "page_context": "none",
            "steps": [
                {"action": "wait_for_popup", "timeout_ms": 15000},
                {"action": "click", "locator": {"by": "css", "value": "#open"}},
            ],
        }))
        .unwrap();
        let batch = tokio::spawn({
            let runtime = runtime.clone();
            async move { execute_request_with_runtime(runtime, request, &ImagePolicy::default()).await }
        });
        tokio::time::timeout(Duration::from_secs(30), arrived_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let unrelated = {
            let rt = runtime.lock().await;
            tokio::task::block_in_place(|| rt.open_tab().unwrap().get_target_id().to_string())
        };
        tokio::time::sleep(Duration::from_millis(200)).await;
        release.notify_waiters();
        let report = tokio::time::timeout(Duration::from_secs(60), batch)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        server.abort();
        assert!(report.ok, "{report:?}");
        let popup = report.steps[0].data.as_ref().unwrap()["tab_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(
            popup, unrelated,
            "an unrelated tab must not satisfy the popup wait"
        );
        assert_eq!(
            runtime.lock().await.tab_openers[&popup].tab_id,
            tab_a,
            "the popup must be opened by the batch's tab"
        );
        drop((ensured, runtime));
        assert!(
            crate::integrations::browser_runtime::project_browser::stop_project_runtime(
                &app, &scope
            )
            .await
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires REFACT_BROWSER_E2E=1 and Chrome"]
    async fn background_project_tabs_render_and_take_input_with_real_chrome() {
        assert_eq!(std::env::var("REFACT_BROWSER_E2E").as_deref(), Ok("1"));
        let cache = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        let project = tempfile::tempdir().unwrap();
        {
            let gcx = Arc::get_mut(&mut gcx).unwrap();
            gcx.cache_dir = cache.path().to_path_buf();
            gcx.cmdline.workspace_folder = project.path().to_str().unwrap().into();
        }
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().into()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let router = crate::http::routers::make_refact_http_server(app.clone());
        let scope = ProjectBrowserScope::from_source_root(project.path()).unwrap();
        let ensured = ensure_project_runtime(
            &app,
            &scope,
            crate::integrations::browser_runtime::BrowserLaunchOptions::headless(true),
            Instant::now() + START_DEADLINE,
        )
        .await
        .unwrap();
        let runtime_id = ensured.runtime_id.clone();

        let (status, resized) = post(
            &router,
            "resize",
            serde_json::json!({"width": 640, "height": 480}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resized}");
        let (_, before) = post(&router, "status", serde_json::json!({})).await;
        let first = before["active_tab"].as_str().unwrap().to_string();
        assert_eq!(before["tabs"][0]["opened_by_chat"], serde_json::Value::Null);
        assert!(before.get("migration").is_none());

        let (status, created) = post(
            &router,
            "tab",
            serde_json::json!({"op": "new", "chat_id": "chat-live-1"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let second = created["new_tab_id"].as_str().unwrap().to_string();
        assert_ne!(second, first);
        assert_eq!(
            created["active_tab"],
            first.as_str(),
            "op new must keep the active tab"
        );
        let entry = created["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tab| tab["tab_id"] == second.as_str())
            .unwrap()
            .clone();
        assert_eq!(entry["opened_by_chat"], "chat-live-1");
        assert_eq!(entry["last_chat"], "chat-live-1");
        assert!(entry["last_used_ms"].as_u64().is_some());
        assert_eq!(entry["active"], false);
        assert_eq!(entry["name"], serde_json::Value::Null);

        let (first_tab, second_tab, browser) = {
            let rt = ensured.runtime.lock().await;
            (
                tab_by_id(&rt, &first).unwrap(),
                tab_by_id(&rt, &second).unwrap(),
                rt.browser.clone(),
            )
        };
        tokio::task::block_in_place(|| {
            for tab in [&first_tab, &second_tab] {
                tab.navigate_to(LIVE_PAGE).unwrap();
                tab.wait_until_navigated().unwrap();
            }
            let shared_window = browser.new_tab().unwrap();
            shared_window.navigate_to(LIVE_PAGE).unwrap();
            shared_window.wait_until_navigated().unwrap();
            std::thread::sleep(Duration::from_millis(500));
            eprintln!(
                "evidence: plain new_tab visibility={} rAF counter={} (shares a window); project tabs: first={} second={}",
                eval_string(&shared_window, "document.visibilityState"),
                eval_string(&shared_window, "document.getElementById('c').textContent"),
                eval_string(&first_tab, "document.visibilityState"),
                eval_string(&second_tab, "document.visibilityState"),
            );
            let _ = shared_window.close(false);
            for tab in [&first_tab, &second_tab] {
                assert_eq!(eval_string(tab, "document.visibilityState"), "visible");
                let before = eval_string(tab, "document.getElementById('c').textContent");
                std::thread::sleep(Duration::from_millis(300));
                let after = eval_string(tab, "document.getElementById('c').textContent");
                assert!(
                    after.parse::<u64>().unwrap() > before.parse::<u64>().unwrap() + 3,
                    "rAF must keep running in every project tab: {before} -> {after}"
                );
            }
        });

        let frame_of = |tab: String| {
            let router = router.clone();
            async move {
                let (status, frame) = post(
                    &router,
                    "frame",
                    serde_json::json!({"tab_id": tab, "max_side": 640}),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{frame}");
                frame
            }
        };
        let frame_a = frame_of(first.clone()).await;
        assert_eq!(frame_a["tab_id"], first.as_str());
        assert_eq!(frame_a["viewport"]["width"], 640.0);
        let frame_b1 = frame_of(second.clone()).await;
        assert_eq!(frame_b1["tab_id"], second.as_str());
        assert_eq!(
            frame_b1["viewport"]["width"], 640.0,
            "pane size applies to the viewed tab"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        let frame_b2 = frame_of(second.clone()).await;
        assert_ne!(
            frame_b1["data"], frame_b2["data"],
            "background tab frames must be live"
        );

        let (status, clicked) = post(
            &router,
            "input",
            serde_json::json!({"runtime_id": runtime_id, "tab_id": second, "kind": "click", "x": 500, "y": 400}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{clicked}");
        let (status, _) = post(
            &router,
            "input",
            serde_json::json!({"runtime_id": runtime_id, "tab_id": second, "kind": "click", "x": 20, "y": 215}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, typed) = post(
            &router,
            "input",
            serde_json::json!({"runtime_id": runtime_id, "tab_id": second, "kind": "text", "text": "hello"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{typed}");
        tokio::task::block_in_place(|| {
            assert_eq!(
                eval_string(&second_tab, "getComputedStyle(document.body).backgroundColor+'|'+document.getElementById('i').value"),
                "rgb(255, 0, 0)|hello"
            );
            assert_eq!(
                eval_string(&first_tab, "getComputedStyle(document.body).backgroundColor+'|'+document.getElementById('i').value"),
                "rgba(0, 0, 0, 0)|"
            );
        });
        let (_, after_input) = post(&router, "status", serde_json::json!({})).await;
        assert_eq!(after_input["active_tab"], first.as_str());

        let (status, created) = post(&router, "tab", serde_json::json!({"op": "new"})).await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let retired = created["new_tab_id"].as_str().unwrap().to_string();
        let retired_tab = {
            let mut rt = ensured.runtime.lock().await;
            let tab = tab_by_id(&rt, &retired).unwrap();
            rt.select_tab_after_close(&retired);
            tab
        };
        for (path, body) in [
            ("frame", serde_json::json!({"tab_id": retired})),
            (
                "resize",
                serde_json::json!({"width": 800, "height": 600, "tab_id": retired}),
            ),
            (
                "input",
                serde_json::json!({"runtime_id": runtime_id, "tab_id": retired, "kind": "key", "key": "a"}),
            ),
        ] {
            let (status, stale) = post(&router, path, body).await;
            assert_eq!(
                status,
                StatusCode::GONE,
                "a tab retired by close must answer 410 while Chrome still lists it: {path} {stale}"
            );
        }
        let (_, listed) = post(&router, "status", serde_json::json!({})).await;
        assert!(
            listed["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .all(|tab| tab["tab_id"] != retired.as_str()),
            "{listed}"
        );
        tokio::task::block_in_place(|| retired_tab.close(false).unwrap());

        for (path, body) in [
            ("frame", serde_json::json!({"tab_id": "missing"})),
            (
                "resize",
                serde_json::json!({"width": 800, "height": 600, "tab_id": "missing"}),
            ),
            (
                "navigate",
                serde_json::json!({"op": "reload", "tab_id": "missing"}),
            ),
            (
                "tab",
                serde_json::json!({"op": "close", "tab_id": "missing"}),
            ),
            (
                "input",
                serde_json::json!({"runtime_id": runtime_id, "tab_id": "missing", "kind": "key", "key": "a"}),
            ),
        ] {
            let (status, stale) = post(&router, path, body).await;
            assert_eq!(status, StatusCode::GONE, "{path} {stale}");
            assert!(
                stale["detail"].as_str().unwrap().contains("stale tab_id"),
                "{path} {stale}"
            );
        }
        let (status, _) = post(
            &router,
            "input",
            serde_json::json!({"runtime_id": "rt-old", "tab_id": second, "kind": "key", "key": "a"}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let (third, fourth) = {
            let mut rt = ensured.runtime.lock().await;
            tokio::task::block_in_place(|| {
                use crate::integrations::browser_models::TabTarget;
                use crate::integrations::browser_runtime::project_browser::chat_default_target;
                assert_eq!(
                    chat_default_target(&mut rt, "chat-live-1").unwrap(),
                    TabTarget::Id {
                        id: second.clone()
                    }
                );
                assert_eq!(
                    chat_default_target(&mut rt, "chat-live-2").unwrap(),
                    TabTarget::Id {
                        id: first.clone()
                    },
                    "an unowned active tab is claimed by id"
                );
                assert_eq!(
                    rt.tab_meta[&first].last_chat.as_deref(),
                    Some("chat-live-2"),
                    "the claim is stamped before the runtime lock is released"
                );
                let TabTarget::Id {
                    id: fourth,
                } = chat_default_target(&mut rt, "chat-live-4").unwrap()
                else {
                    panic!("a claimed active tab must not be shared");
                };
                assert!(fourth != first && fourth != second);
                refact_browser::record_chat_tab_use(
                    &mut rt.tab_meta,
                    "chat-live-2",
                    &[],
                    Some(&first),
                    Some("Docs"),
                    1,
                );
                let TabTarget::Id {
                    id: third,
                } = chat_default_target(&mut rt, "chat-live-3").unwrap()
                else {
                    panic!("an owned active tab must not be claimed");
                };
                assert!(third != first && third != second);
                assert_eq!(rt.active_tab_target_id(), Some(first.as_str()));
                assert_eq!(
                    rt.tab_meta[&third].opened_by_chat.as_deref(),
                    Some("chat-live-3")
                );
                assert_eq!(
                    rt.list_tab_infos()
                        .iter()
                        .find(|tab| tab.id == first)
                        .unwrap()
                        .name
                        .as_deref(),
                    Some("Docs")
                );
                (third, fourth)
            })
        };

        for (tab, size) in [(&third, 700), (&fourth, 900)] {
            let (status, resized) = post(
                &router,
                "resize",
                serde_json::json!({"width": size, "height": 500, "tab_id": tab}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{resized}");
        }
        let fourth_tab = tab_by_id(&*ensured.runtime.lock().await, &fourth).unwrap();
        assert!(tokio::task::block_in_place(|| fourth_tab
            .close(false)
            .unwrap()));
        let pruned = async {
            loop {
                let (status, _) = post(&router, "status", serde_json::json!({})).await;
                assert_eq!(status, StatusCode::OK);
                if ensured
                    .runtime
                    .lock()
                    .await
                    .pane
                    .tab_size(&fourth)
                    .is_none()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), pruned)
            .await
            .expect("a tab closed outside the Browser API must lose its pane size");
        assert_eq!(
            ensured.runtime.lock().await.pane.tab_size(&third),
            Some((700, 500)),
            "pruning must keep live tabs' sizes"
        );
        for tab in [&second, &third] {
            let (status, closed) = post(
                &router,
                "tab",
                serde_json::json!({"op": "close", "tab_id": tab}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{closed}");
        }
        {
            let rt = ensured.runtime.lock().await;
            assert!(!rt.tab_meta.contains_key(&second));
            assert_eq!(rt.pane.tab_size(&third), None);
        }
        let (status, last) = post(
            &router,
            "tab",
            serde_json::json!({"op": "close", "tab_id": first}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{last}");
        drop((first_tab, second_tab, browser, ensured));
        assert!(
            crate::integrations::browser_runtime::project_browser::stop_project_runtime(
                &app, &scope
            )
            .await
        );
    }
}
