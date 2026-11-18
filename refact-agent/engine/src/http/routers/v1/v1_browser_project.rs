//! Human-controlled Browser workspace API over the one shared project browser runtime.
//!
//! The project is always the engine's trusted current project; clients never name a project,
//! chat or profile path. Frames are pulled, never pushed into chats, and no raw browser
//! control endpoint is ever returned.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, HeaderValue, Request, Response, StatusCode};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use base64::Engine;
use headless_chrome::protocol::cdp::{Input, Page};
use hyper::Body;
use refact_core::image_policy::{resize_to_policy, ImageFormat, ImagePolicy};
use serde::Deserialize;
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::http::routers::v1::browser_settings;
use crate::integrations::browser_runtime::project_browser::{
    adopt_legacy_profile, ensure_project_runtime, find_project_runtime, project_profile_state,
    skip_legacy_adoption, EnsureError, ProjectBrowserScope, ProjectProfileState,
};
use crate::integrations::browser_runtime::BrowserRuntime;

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
const START_DEADLINE: Duration = Duration::from_secs(120);

pub(crate) const PROXY_TOKEN_ENV: &str = "REFACT_PROJECT_BROWSER_PROXY_TOKEN";
pub(crate) const PROJECT_HEADER: &str = "x-refact-project-id";

pub(crate) fn is_worker_path(path: &str) -> bool {
    const PREFIX: &str = "/v1/browser/project";
    path == PREFIX
        || path
            .strip_prefix(PREFIX)
            .is_some_and(|tail| tail.starts_with('/'))
}

pub(crate) fn gui_origin_allowed(headers: &HeaderMap) -> bool {
    headers.contains_key(header::ORIGIN) && super::work::protected_origin_allowed(headers)
}

pub fn router(app: AppState) -> Router<AppState> {
    Router::new()
        .route("/browser/project/status", post(handle_status))
        .route("/browser/project/start", post(handle_start))
        .route("/browser/project/tab", post(handle_tab))
        .route("/browser/project/navigate", post(handle_navigate))
        .route("/browser/project/input", post(handle_input))
        .route("/browser/project/resize", post(handle_resize))
        .route("/browser/project/frame", post(handle_frame))
        .route(
            "/browser/project/migration/candidates",
            post(handle_candidates),
        )
        .route("/browser/project/migration/adopt", post(handle_adopt))
        .route("/browser/project/migration/skip", post(handle_skip))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .route_layer(middleware::from_fn_with_state(app, authorize_proxy))
}

async fn authorize_proxy(
    State(app): State<AppState>,
    request: Request<Body>,
    next: Next<Body>,
) -> axum::response::Response {
    let mut response = if proxy_authorized(&app, &request) {
        next.run(request).await
    } else {
        fail(
            StatusCode::UNAUTHORIZED,
            "Project Browser requires an authenticated Refact daemon project session",
        )
        .into_response()
    };
    response.extensions_mut().remove::<ScratchError>();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn proxy_authorized(app: &AppState, request: &Request<Body>) -> bool {
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;

    let cmdline = &app.gcx.cmdline;
    let headers = request.headers();
    if cmdline.daemon_endpoint.is_empty()
        || cmdline.project_id.is_empty()
        || headers.contains_key(header::ORIGIN)
        || !request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .is_some_and(|ConnectInfo(addr)| addr.ip().is_loopback())
    {
        return false;
    }
    let mut project_ids = headers.get_all(PROJECT_HEADER).iter();
    if !project_ids
        .next()
        .is_some_and(|id| id == cmdline.project_id.as_str())
        || project_ids.next().is_some()
    {
        return false;
    }
    let expected = cmdline
        .project_browser_proxy_token
        .as_ref()
        .and_then(|value| value.to_str().ok());
    let provided = crate::daemon::auth::bearer_token_from_headers(headers);
    expected
        .zip(provided)
        .is_some_and(|(expected, provided)| crate::daemon::auth::token_matches(provided, expected))
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

async fn scope(app: &AppState) -> Result<ProjectBrowserScope, ScratchError> {
    let root = &app.gcx.cmdline.workspace_folder;
    if root.is_empty() {
        return Err(fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "managed project root unavailable",
        ));
    }
    ProjectBrowserScope::from_source_root(std::path::Path::new(root))
        .map_err(|error| fail(StatusCode::SERVICE_UNAVAILABLE, error))
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
    let deadline = Instant::now() + UI_COMMAND_DEADLINE;
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

static PANE_SIZES: OnceLock<StdMutex<HashMap<String, (u32, u32)>>> = OnceLock::new();

fn pane_sizes() -> &'static StdMutex<HashMap<String, (u32, u32)>> {
    PANE_SIZES.get_or_init(Default::default)
}

fn pane_size(runtime_id: &str) -> Option<(u32, u32)> {
    pane_sizes()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(runtime_id)
        .copied()
}

fn remember_pane_size(runtime_id: &str, size: (u32, u32)) {
    let mut sizes = pane_sizes()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if sizes.len() > 64 {
        sizes.clear();
    }
    sizes.insert(runtime_id.to_string(), size);
}

pub fn tool_override_active(runtime: &BrowserRuntime) -> bool {
    runtime.context_state.viewport.is_some()
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

fn apply_pane_viewport(tab: &headless_chrome::Tab, size: (u32, u32)) -> Result<(), ScratchError> {
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
    .map_err(cdp)
}

/// Applies the remembered pane size to one tab unless a tool set an explicit viewport.
fn sync_pane_to_tab(
    runtime: &BrowserRuntime,
    runtime_id: &str,
    tab: &headless_chrome::Tab,
) -> Result<bool, ScratchError> {
    if tool_override_active(runtime) {
        return Ok(false);
    }
    match pane_size(runtime_id) {
        Some(size) => apply_pane_viewport(tab, size).map(|_| true),
        None => Ok(false),
    }
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

fn headless_modifiers(mask: u32) -> Vec<headless_chrome::browser::tab::ModifierKey> {
    use headless_chrome::browser::tab::ModifierKey;
    let mut keys = Vec::new();
    for (bit, key) in [
        (1, ModifierKey::Alt),
        (2, ModifierKey::Ctrl),
        (4, ModifierKey::Meta),
        (8, ModifierKey::Shift),
    ] {
        if mask & bit != 0 {
            keys.push(key);
        }
    }
    keys
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

/// Input is accepted only for exactly the runtime and tab the UI is displaying.
pub fn check_fence(
    expected_runtime: &str,
    actual_runtime: &str,
    expected_tab: &str,
    active_tab: Option<&str>,
) -> Result<(), FenceError> {
    if expected_runtime != actual_runtime {
        return Err(FenceError::RuntimeChanged);
    }
    if active_tab != Some(expected_tab) {
        return Err(FenceError::TabChanged);
    }
    Ok(())
}

fn fence_error(error: FenceError) -> ScratchError {
    fail(
        StatusCode::CONFLICT,
        match error {
            FenceError::RuntimeChanged => {
                "stale runtime_id: the project browser was replaced; refresh its status"
            }
            FenceError::TabChanged => {
                "stale tab_id: the active tab changed; refresh before sending input"
            }
        },
    )
}

fn active_tab_id(runtime: &BrowserRuntime) -> Option<String> {
    runtime
        .get_active_tab()
        .map(|tab| tab.get_target_id().to_string())
}

fn tab_by_id(runtime: &BrowserRuntime, tab_id: &str) -> Option<Arc<headless_chrome::Tab>> {
    runtime
        .browser
        .get_tabs()
        .lock()
        .ok()?
        .iter()
        .find(|tab| tab.get_target_id() == tab_id)
        .cloned()
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

fn migration_json(state: &ProjectProfileState) -> serde_json::Value {
    let label = if state.resolved {
        "resolved"
    } else if state.candidates.is_empty() {
        "not_needed"
    } else {
        "available"
    };
    serde_json::json!({
        "state": label,
        "candidate_count": state.candidates.len(),
    })
}

async fn status_value(
    app: &AppState,
    scope: &ProjectBrowserScope,
) -> Result<serde_json::Value, ScratchError> {
    let profile = project_profile_state(app, scope)
        .await
        .map_err(|error| fail(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let migration = migration_json(&profile);
    let Some((runtime_id, runtime)) = find_project_runtime(app, scope).await else {
        return Ok(serde_json::json!({
            "project_id": profile.project_id,
            "running": false,
            "connected": false,
            "tabs": [],
            "migration": migration,
        }));
    };
    let record = crate::integrations::browser_runtime::runtime_owner_record(&runtime_id);
    let Ok(mut rt) = tokio::time::timeout(UI_LOCK_WAIT, runtime.lock()).await else {
        return Ok(serde_json::json!({
            "project_id": profile.project_id,
            "running": true,
            "runtime_id": runtime_id,
            "connected": record.map(|record| record.is_connected()).unwrap_or(false),
            "busy": true,
            "tabs": [],
            "migration": migration,
        }));
    };
    blocking(|| {
        refact_browser::adopt_new_tabs(&mut rt, None);
    });
    let tabs = rt.list_tab_infos();
    let active = rt.get_active_tab();
    let (url, title) = match &active {
        Some(tab) => (tab.get_url(), tab.get_title().unwrap_or_default()),
        None => (String::new(), String::new()),
    };
    let viewport = active.as_ref().and_then(|tab| {
        blocking(|| tab.call_method(Page::GetLayoutMetrics(None)).ok()).map(|metrics| {
            serde_json::json!({
                "width": metrics.css_visual_viewport.client_width,
                "height": metrics.css_visual_viewport.client_height,
            })
        })
    });
    let pane = pane_size(&runtime_id).map(|(w, h)| serde_json::json!({"width": w, "height": h}));
    Ok(serde_json::json!({
        "project_id": profile.project_id,
        "running": true,
        "runtime_id": runtime_id,
        "connected": rt.is_connected,
        "headless": rt.headless(),
        "active_tab": active.as_ref().map(|tab| tab.get_target_id().to_string()),
        "url": url,
        "title": title,
        "tabs": tabs.iter().map(|tab| serde_json::json!({
            "tab_id": tab.id,
            "url": tab.url,
            "title": tab.title,
            "active": tab.active,
        })).collect::<Vec<_>>(),
        "viewport": viewport,
        "pane": pane,
        "tool_viewport_override": tool_override_active(&rt),
        "migration": migration,
    }))
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
    let ensured = match ensure_project_runtime(
        &app,
        &scope,
        options,
        Instant::now() + START_DEADLINE,
    )
    .await
    {
        Ok(ensured) => ensured,
        Err(
            error @ EnsureError::DecisionRequired {
                ..
            },
        ) => {
            let EnsureError::DecisionRequired {
                candidate_count,
            } = &error
            else {
                unreachable!()
            };
            return Ok(json_response(
                StatusCode::CONFLICT,
                serde_json::json!({
                    "error": "migration_required",
                    "detail": error.to_string(),
                    "candidate_count": candidate_count,
                }),
            ));
        }
        Err(EnsureError::Failed(error)) => {
            return Err(fail(StatusCode::INTERNAL_SERVER_ERROR, error));
        }
    };
    {
        let rt = lock_runtime(&ensured.runtime).await?;
        if let Some(tab) = rt.get_active_tab() {
            blocking(|| sync_pane_to_tab(&rt, &ensured.runtime_id, &tab))?;
        }
    }
    let mut value = status_value(&app, &scope).await?;
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

async fn handle_tab(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: TabBody = parse_body(&body)?;
    if !matches!(post.op.as_str(), "new" | "select" | "close") {
        return Err(fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown tab op; expected new, select or close",
        ));
    }
    let url = checked_url(post.url.as_deref())?;
    let scope = scope(&app).await?;
    let (runtime_id, runtime) = live_runtime(&app, &scope).await?;
    {
        let mut rt = lock_runtime(&runtime).await?;
        rt.touch();
        if post.op == "new" {
            let tab = blocking(|| rt.browser.new_tab()).map_err(cdp)?;
            blocking(|| {
                crate::integrations::browser_runtime::setup_recording_for_tab(&mut rt, tab.clone())
            })
            .map_err(cdp)?;
            rt.set_active_tab_target_id(tab.get_target_id().to_string());
            blocking(|| {
                tab.activate().map_err(cdp)?;
                sync_pane_to_tab(&rt, &runtime_id, &tab)?;
                match &url {
                    Some(url) => navigate_tab(&tab, url),
                    None => Ok(()),
                }
            })?;
        } else {
            let tab_id = post
                .tab_id
                .as_deref()
                .ok_or_else(|| fail(StatusCode::UNPROCESSABLE_ENTITY, "tab_id is required"))?;
            let tab = tab_by_id(&rt, tab_id)
                .ok_or_else(|| fail(StatusCode::CONFLICT, "stale tab_id: no such tab"))?;
            if post.op == "select" {
                rt.set_active_tab_target_id(tab_id.to_string());
                blocking(|| {
                    tab.activate().map_err(cdp)?;
                    sync_pane_to_tab(&rt, &runtime_id, &tab).map(|_| ())
                })?;
            } else {
                if rt.list_tab_infos().len() <= 1 {
                    return Err(fail(
                        StatusCode::CONFLICT,
                        "the last tab cannot be closed; navigate it instead",
                    ));
                }
                blocking(|| tab.close(false)).map_err(cdp)?;
                rt.select_tab_after_close(tab_id);
            }
        }
    }
    Ok(json_response(
        StatusCode::OK,
        status_value(&app, &scope).await?,
    ))
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

fn history_step(tab: &headless_chrome::Tab, offset: i64) -> Result<(), ScratchError> {
    let history = tab
        .call_method(Page::GetNavigationHistory(None))
        .map_err(cdp)?;
    let index = i64::from(history.current_index) + offset;
    let entry = usize::try_from(index)
        .ok()
        .and_then(|index| history.entries.get(index))
        .ok_or_else(|| fail(StatusCode::CONFLICT, "there is no page in that direction"))?;
    tab.call_method(Page::NavigateToHistoryEntry {
        entry_id: entry.id,
    })
    .map(|_| ())
    .map_err(cdp)
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
    let (_, runtime) = live_runtime(&app, &scope).await?;
    {
        let mut rt = lock_runtime(&runtime).await?;
        rt.touch();
        let tab = match post.tab_id.as_deref() {
            Some(id) => tab_by_id(&rt, id)
                .ok_or_else(|| fail(StatusCode::CONFLICT, "stale tab_id: no such tab"))?,
            None => rt
                .get_active_tab()
                .ok_or_else(|| fail(StatusCode::CONFLICT, "the project browser has no tab"))?,
        };
        blocking(|| match (&url, post.op.as_deref()) {
            (Some(url), _) => navigate_tab(&tab, url),
            (None, Some("back")) => history_step(&tab, -1),
            (None, Some("forward")) => history_step(&tab, 1),
            _ => tab.reload(false, None).map(|_| ()).map_err(cdp),
        })?;
    }
    Ok(json_response(
        StatusCode::OK,
        status_value(&app, &scope).await?,
    ))
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

fn mouse_event(
    tab: &headless_chrome::Tab,
    kind: Input::DispatchMouseEventTypeOption,
    x: f64,
    y: f64,
    modifiers: u32,
    button: Input::MouseButton,
    click_count: Option<u32>,
    delta: (Option<f64>, Option<f64>),
) -> Result<(), ScratchError> {
    tab.call_method(Input::DispatchMouseEvent {
        Type: kind,
        x,
        y,
        modifiers: Some(modifiers),
        timestamp: None,
        button: Some(button),
        buttons: None,
        click_count,
        force: None,
        tangential_pressure: None,
        tilt_x: None,
        tilt_y: None,
        twist: None,
        delta_x: delta.0,
        delta_y: delta.1,
        pointer_Type: None,
    })
    .map(|_| ())
    .map_err(cdp)
}

fn parse_button(button: Option<&str>) -> Result<Input::MouseButton, String> {
    match button.unwrap_or("left") {
        "left" => Ok(Input::MouseButton::Left),
        "middle" => Ok(Input::MouseButton::Middle),
        "right" => Ok(Input::MouseButton::Right),
        _ => Err("button must be left, middle or right".into()),
    }
}

fn unprocessable(error: String) -> ScratchError {
    fail(StatusCode::UNPROCESSABLE_ENTITY, error)
}

async fn handle_input(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: InputBody = parse_body(&body)?;
    let mask = modifier_mask(&post.modifiers).map_err(unprocessable)?;
    match post.kind.as_str() {
        "click" => {
            finite_coord("x", post.x).map_err(unprocessable)?;
            finite_coord("y", post.y).map_err(unprocessable)?;
            parse_button(post.button.as_deref()).map_err(unprocessable)?;
        }
        "wheel" => {
            finite_coord("x", post.x).map_err(unprocessable)?;
            finite_coord("y", post.y).map_err(unprocessable)?;
            wheel_delta("dx", post.dx).map_err(unprocessable)?;
            wheel_delta("dy", post.dy).map_err(unprocessable)?;
        }
        "key" if post.text.is_none() => {
            let key = post.key.as_deref().unwrap_or_default();
            if key.is_empty() || key.chars().count() > 32 {
                return Err(unprocessable("key must be 1..=32 characters".into()));
            }
        }
        "key" | "text" => {
            let text = post.text.as_deref().unwrap_or_default();
            if text.is_empty() || text.chars().count() > MAX_TEXT_CHARS {
                return Err(unprocessable(format!(
                    "text must be 1..={MAX_TEXT_CHARS} characters"
                )));
            }
        }
        _ => {
            return Err(unprocessable(
                "kind must be click, wheel, key or text".into(),
            ))
        }
    }
    let scope = scope(&app).await?;
    let (runtime_id, runtime) = live_runtime(&app, &scope).await?;
    let mut rt = lock_runtime(&runtime).await?;
    let active = active_tab_id(&rt);
    check_fence(
        &post.runtime_id,
        &runtime_id,
        &post.tab_id,
        active.as_deref(),
    )
    .map_err(fence_error)?;
    let tab = tab_by_id(&rt, &post.tab_id).ok_or_else(|| fence_error(FenceError::TabChanged))?;
    rt.touch();
    blocking(|| -> Result<(), ScratchError> {
        match post.kind.as_str() {
            "click" => {
                let (x, y) = (post.x.unwrap_or_default(), post.y.unwrap_or_default());
                let button = parse_button(post.button.as_deref()).map_err(unprocessable)?;
                mouse_event(
                    &tab,
                    Input::DispatchMouseEventTypeOption::MouseMoved,
                    x,
                    y,
                    mask,
                    Input::MouseButton::None,
                    None,
                    (None, None),
                )?;
                mouse_event(
                    &tab,
                    Input::DispatchMouseEventTypeOption::MousePressed,
                    x,
                    y,
                    mask,
                    button.clone(),
                    Some(1),
                    (None, None),
                )?;
                mouse_event(
                    &tab,
                    Input::DispatchMouseEventTypeOption::MouseReleased,
                    x,
                    y,
                    mask,
                    button,
                    Some(1),
                    (None, None),
                )
            }
            "wheel" => mouse_event(
                &tab,
                Input::DispatchMouseEventTypeOption::MouseWheel,
                post.x.unwrap_or_default(),
                post.y.unwrap_or_default(),
                mask,
                Input::MouseButton::None,
                None,
                (Some(post.dx.unwrap_or(0.0)), Some(post.dy.unwrap_or(0.0))),
            ),
            "key" if post.text.is_none() => {
                let key = post.key.as_deref().unwrap_or_default();
                let modifiers = headless_modifiers(mask);
                tab.press_key_with_modifiers(
                    crate::integrations::browser_actions::normalize_key_name(key),
                    (!modifiers.is_empty()).then_some(modifiers.as_slice()),
                )
                .map(|_| ())
                .map_err(|error| unprocessable(format!("unsupported key: {error}")))
            }
            _ => tab
                .call_method(Input::InsertText {
                    text: post.text.clone().unwrap_or_default(),
                })
                .map(|_| ())
                .map_err(cdp),
        }
    })?;
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
}

async fn handle_resize(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ResizeBody = parse_body(&body)?;
    let size = validate_pane(post.width, post.height).map_err(unprocessable)?;
    let scope = scope(&app).await?;
    let (runtime_id, runtime) = live_runtime(&app, &scope).await?;
    let rt = lock_runtime(&runtime).await?;
    remember_pane_size(&runtime_id, size);
    let (applied, reason) = if tool_override_active(&rt) {
        (false, Some("tool_viewport_override_active"))
    } else {
        match rt.get_active_tab() {
            Some(tab) => {
                blocking(|| apply_pane_viewport(&tab, size))?;
                (true, None)
            }
            None => (false, Some("no_active_tab")),
        }
    };
    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "runtime_id": runtime_id,
            "applied": applied,
            "reason": reason,
            "viewport": {"width": size.0, "height": size.1},
        }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameBody {
    #[serde(default)]
    max_side: Option<u32>,
}

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
    let rt = lock_runtime(&runtime).await?;
    let tab = rt
        .get_active_tab()
        .ok_or_else(|| fail(StatusCode::CONFLICT, "the project browser has no tab"))?;
    let captured = blocking(|| -> Result<_, ScratchError> {
        let shot = tab
            .call_method(Page::CaptureScreenshot {
                format: Some(Page::CaptureScreenshotFormatOption::Jpeg),
                clip: None,
                quality: Some(u32::from(FRAME_JPEG_QUALITY)),
                from_surface: Some(true),
                capture_beyond_viewport: Some(false),
                optimize_for_speed: Some(true),
            })
            .map_err(cdp)?;
        let metrics = tab.call_method(Page::GetLayoutMetrics(None)).map_err(cdp)?;
        Ok((
            shot.data,
            metrics.css_visual_viewport.client_width,
            metrics.css_visual_viewport.client_height,
            tab.get_url(),
            tab.get_title().unwrap_or_default(),
        ))
    })?;
    let (data, css_width, css_height, url, title) = captured;
    let tab_id = tab.get_target_id().to_string();
    drop(rt);
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
    Ok(json_response(
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
    ))
}

// ---------------------------------------------------------------------------------------
// One-time legacy profile migration (safe metadata only; no paths, no contents)
// ---------------------------------------------------------------------------------------

async fn handle_candidates(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let _: EmptyBody = parse_body(&body)?;
    let scope = scope(&app).await?;
    let state = project_profile_state(&app, &scope)
        .await
        .map_err(|error| fail(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    Ok(json_response(StatusCode::OK, profile_json(&state)))
}

fn profile_json(state: &ProjectProfileState) -> serde_json::Value {
    let mut value = migration_json(state);
    value["project_id"] = serde_json::json!(state.project_id);
    value["runtime_live"] = serde_json::json!(state.runtime_live);
    value["candidates"] = serde_json::json!(state
        .candidates
        .iter()
        .map(|candidate| serde_json::json!({
            "legacy_chat_id": candidate.chat_id,
            "last_used_ms": candidate.modified_ms,
            "size_class": serde_json::Value::Null,
            "locked": candidate.active,
        }))
        .collect::<Vec<_>>());
    value
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdoptBody {
    legacy_chat_id: String,
    #[serde(default)]
    confirm: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkipBody {
    #[serde(default)]
    confirm: bool,
}

async fn handle_adopt(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: AdoptBody = parse_body(&body)?;
    if !post.confirm {
        return Err(unprocessable(
            "adopting a chat browser profile requires confirm: true".into(),
        ));
    }
    let scope = scope(&app).await?;
    let outcome = adopt_legacy_profile(&app, &scope, &post.legacy_chat_id, true)
        .await
        .map_err(|error| fail(StatusCode::CONFLICT, error))?;
    let state = project_profile_state(&app, &scope)
        .await
        .map_err(|error| fail(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let mut value = profile_json(&state);
    value["adopted_chat_id"] = serde_json::json!(outcome.adopted_chat_id);
    Ok(json_response(StatusCode::OK, value))
}

async fn handle_skip(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: SkipBody = parse_body(&body)?;
    if !post.confirm {
        return Err(unprocessable(
            "starting with a fresh project profile requires confirm: true".into(),
        ));
    }
    let scope = scope(&app).await?;
    let state = skip_legacy_adoption(&app, &scope)
        .await
        .map_err(|error| fail(StatusCode::CONFLICT, error))?;
    Ok(json_response(StatusCode::OK, profile_json(&state)))
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
    fn input_fence_rejects_stale_runtime_and_stale_tab() {
        assert_eq!(check_fence("rt", "rt", "t1", Some("t1")), Ok(()));
        assert_eq!(
            check_fence("rt-old", "rt", "t1", Some("t1")),
            Err(FenceError::RuntimeChanged)
        );
        assert_eq!(
            check_fence("rt", "rt", "t1", Some("t2")),
            Err(FenceError::TabChanged)
        );
        assert_eq!(
            check_fence("rt", "rt", "t1", None),
            Err(FenceError::TabChanged)
        );
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

    struct Harness {
        router: axum::Router,
        project: tempfile::TempDir,
        cache: std::path::PathBuf,
    }

    const PROXY_TOKEN: &str = "browser-proxy-test";
    const PROJECT_ID: &str = "project-fixture";

    async fn harness() -> Harness {
        harness_with_token(Some(PROXY_TOKEN)).await
    }

    async fn harness_with_token(token: Option<&'static str>) -> Harness {
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        let project = tempfile::tempdir().unwrap();
        let cmdline = &mut Arc::get_mut(&mut gcx).unwrap().cmdline;
        cmdline.daemon_endpoint = "http://127.0.0.1:8488".into();
        cmdline.project_id = PROJECT_ID.into();
        cmdline.daemon_auth_token = Some("general-worker-auth".into());
        cmdline.project_browser_proxy_token = token.map(HeaderValue::from_static);
        cmdline.workspace_folder = project.path().to_str().unwrap().into();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().into()];
        let cache = gcx.cache_dir.clone();
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        Harness {
            router: crate::http::routers::make_refact_http_server(app),
            project,
            cache,
        }
    }

    fn request(path: &str, body: Body) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/v1/browser/project/{path}"))
            .header("content-type", "application/json")
            .header(PROJECT_HEADER, PROJECT_ID)
            .body(body)
            .unwrap();
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:8080".parse::<std::net::SocketAddr>().unwrap(),
        ));
        request
    }

    fn proxy_bearer() -> HeaderValue {
        format!("Bearer {PROXY_TOKEN}").parse().unwrap()
    }

    fn authorized_request(path: &str, body: Body) -> Request<Body> {
        let mut request = request(path, body);
        request
            .headers_mut()
            .insert(header::AUTHORIZATION, proxy_bearer());
        request
    }

    async fn post(
        router: &axum::Router,
        path: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = router
            .clone()
            .oneshot(authorized_request(path, Body::from(body.to_string())))
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn project_browser_routes_require_proxy_authority_before_body_extraction() {
        let h = harness().await;
        for path in [
            "status",
            "start",
            "tab",
            "navigate",
            "input",
            "resize",
            "frame",
            "migration/candidates",
            "migration/adopt",
            "migration/skip",
        ] {
            let body = Body::wrap_stream(futures::stream::poll_fn(
                move |_| -> std::task::Poll<Option<Result<hyper::body::Bytes, std::io::Error>>> {
                    panic!("unauthorized route read {path}'s body")
                },
            ));
            let response = h.router.clone().oneshot(request(path, body)).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        let profile = legacy_chat(&h, "chat-fixture");
        let response = h
            .router
            .clone()
            .oneshot(request(
                "migration/adopt",
                Body::from(r#"{"legacy_chat_id":"chat-fixture","confirm":true}"#),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(profile.exists());
        let (_, value) = post(&h.router, "migration/candidates", serde_json::json!({})).await;
        assert_eq!(value["state"], "available");
    }

    #[tokio::test]
    async fn proxy_authority_requires_the_managed_project_transport_and_unique_bearer() {
        let h = harness().await;
        for (name, header, value, append) in [
            (
                "unrelated worker credential",
                "authorization",
                "Bearer general-worker-auth",
                false,
            ),
            (
                "duplicate bearer",
                "authorization",
                "Bearer browser-proxy-test",
                true,
            ),
            ("wrong project", PROJECT_HEADER, "other-project", false),
            ("multiple projects", PROJECT_HEADER, PROJECT_ID, true),
            (
                "direct web origin",
                "origin",
                "http://127.0.0.1:8080",
                false,
            ),
        ] {
            let mut req = authorized_request("status", Body::empty());
            if append {
                req.headers_mut()
                    .append(header, HeaderValue::from_static(value));
            } else {
                req.headers_mut()
                    .insert(header, HeaderValue::from_static(value));
            }
            let response = h.router.clone().oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{name}");
        }
        for peer in [None, Some("192.0.2.5:8080")] {
            let mut req = authorized_request("status", Body::empty());
            req.extensions_mut().clear();
            if let Some(peer) = peer {
                req.extensions_mut().insert(axum::extract::ConnectInfo(
                    peer.parse::<std::net::SocketAddr>().unwrap(),
                ));
            }
            let response = h.router.clone().oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{peer:?}");
        }
        let disabled = harness_with_token(None).await;
        let response = disabled
            .router
            .oneshot(authorized_request("status", Body::empty()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = h
            .router
            .oneshot(authorized_request("status", Body::empty()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }

    fn legacy_chat(h: &Harness, chat_id: &str) -> std::path::PathBuf {
        let profile = h.cache.join("browser_profiles").join(chat_id);
        std::fs::create_dir_all(profile.join("Default")).unwrap();
        std::fs::write(
            profile.join("Default").join("Cookies"),
            b"secret-cookie-bytes",
        )
        .unwrap();
        let trajectories = h.project.path().join(".refact").join("trajectories");
        std::fs::create_dir_all(&trajectories).unwrap();
        std::fs::write(trajectories.join(format!("{chat_id}.json")), b"{}").unwrap();
        profile
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn status_without_runtime_reports_not_running_and_no_migration_needed() {
        let h = harness().await;
        let (status, value) = post(&h.router, "status", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["running"], false);
        assert_eq!(value["connected"], false);
        assert_eq!(value["tabs"], serde_json::json!([]));
        assert_eq!(value["migration"]["state"], "not_needed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn operations_require_a_running_project_browser() {
        let h = harness().await;
        for (path, body) in [
            ("tab", serde_json::json!({"op": "new"})),
            ("navigate", serde_json::json!({"op": "reload"})),
            ("resize", serde_json::json!({"width": 800, "height": 600})),
            ("frame", serde_json::json!({})),
            (
                "input",
                serde_json::json!({"runtime_id": "r", "tab_id": "t", "kind": "key", "key": "a"}),
            ),
        ] {
            let (status, value) = post(&h.router, path, body).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert!(value["detail"].as_str().unwrap().contains("not running"));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bad_requests_are_rejected_before_touching_a_browser() {
        let h = harness().await;
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
                "migration/adopt",
                serde_json::json!({"legacy_chat_id": "c1"}),
            ),
            ("migration/skip", serde_json::json!({})),
        ];
        for (path, body) in cases {
            let (status, _) = post(&h.router, path, body.clone()).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path} {body}");
        }
        let response = h
            .router
            .clone()
            .oneshot(authorized_request("tab", Body::from("{not json")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn candidates_expose_only_safe_metadata_for_this_project() {
        let h = harness().await;
        let profile = legacy_chat(&h, "chat-one");
        let other = h.cache.join("browser_profiles").join("chat-foreign");
        std::fs::create_dir_all(&other).unwrap();
        let (status, value) = post(&h.router, "migration/candidates", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["state"], "available");
        let candidates = value["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 1, "{value}");
        assert_eq!(candidates[0]["legacy_chat_id"], "chat-one");
        assert_eq!(candidates[0]["locked"], false);
        let text = value.to_string();
        assert!(!text.contains(profile.to_str().unwrap()));
        assert!(!text.contains("browser_profiles"));
        assert!(!text.contains("Cookies"));
        assert!(!text.contains("chat-foreign"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_is_blocked_until_the_user_decides_and_leaves_profiles_untouched() {
        let h = harness().await;
        let profile = legacy_chat(&h, "chat-one");
        let (status, value) = post(&h.router, "start", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(value["error"], "migration_required");
        assert_eq!(value["candidate_count"], 1);
        assert!(profile.join("Default").join("Cookies").exists());
        let (_, status_value) = post(&h.router, "status", serde_json::json!({})).await;
        assert_eq!(status_value["running"], false);
        assert_eq!(status_value["migration"]["state"], "available");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn adopt_requires_confirmation_and_moves_exactly_one_profile_once() {
        let h = harness().await;
        let profile = legacy_chat(&h, "chat-one");
        legacy_chat(&h, "chat-two");
        let (status, _) = post(
            &h.router,
            "migration/adopt",
            serde_json::json!({"legacy_chat_id": "chat-one", "confirm": false}),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(profile.exists());

        let (status, value) = post(
            &h.router,
            "migration/adopt",
            serde_json::json!({"legacy_chat_id": "../escape", "confirm": true}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{value}");

        let (status, value) = post(
            &h.router,
            "migration/adopt",
            serde_json::json!({"legacy_chat_id": "chat-one", "confirm": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(value["state"], "resolved");
        assert_eq!(value["adopted_chat_id"], "chat-one");
        assert!(!profile.exists());
        assert!(h.cache.join("browser_profiles").join("chat-two").exists());

        let (status, _) = post(
            &h.router,
            "migration/adopt",
            serde_json::json!({"legacy_chat_id": "chat-two", "confirm": true}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(h.cache.join("browser_profiles").join("chat-two").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn skip_resolves_migration_and_hides_candidates() {
        let h = harness().await;
        let profile = legacy_chat(&h, "chat-one");
        let (status, value) = post(
            &h.router,
            "migration/skip",
            serde_json::json!({"confirm": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(value["state"], "resolved");
        assert_eq!(value["candidates"], serde_json::json!([]));
        assert!(profile.exists(), "skipping never deletes a legacy profile");
        let (_, status_value) = post(&h.router, "status", serde_json::json!({})).await;
        assert_eq!(status_value["migration"]["state"], "resolved");
    }
}
