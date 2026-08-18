use std::path::PathBuf;
use std::sync::Arc;
use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;
use serde::{Deserialize, Serialize};
use base64::Engine;
use refact_chat_api::WindowBounds;
use refact_core::image_policy::{resize_to_policy, ImageFormat, ImagePolicy};

use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use crate::chat::types::{BrowserTabInfo, ChatEvent, TimelineEntry};
use crate::custom_error::ScratchError;
use crate::integrations::browser_runtime::{
    BrowserLaunchOptions, BrowserProxyOptions, BrowserRuntime, PICKER_PAGE_TIMEOUT_MS,
    compute_frame_hash, ensure_frame_emitter, ensure_injection_into_all_tabs,
    get_browser_profile_dir, register_browser_runtime, relaunch_runtime_for_chat,
    remove_browser_runtime, find_runtime_by_chat_id, setup_recording_for_runtime,
};
use crate::integrations::browser_types::{RecorderEvent, ConsoleEntry, NetworkEntry};
use crate::integrations::browser_models::BrowserActionRequest;
use crate::integrations::browser_controller;

const FRAME_MAX_SIDE: u32 = 1280;

fn overlay_call_script(call: &str) -> String {
    format!(
        "(function(){{ var api = window.__refact_overlays; if (!api) return {{ overlays: 'unavailable' }}; return {{ overlays: 'ready', result: api.{call} }}; }})()"
    )
}

fn eval_overlay(
    tab: &headless_chrome::Tab,
    call: &str,
    context: &str,
) -> Result<serde_json::Value, ScratchError> {
    let evaluated = tab
        .call_method(headless_chrome::protocol::cdp::Runtime::Evaluate {
            expression: overlay_call_script(call),
            return_by_value: Some(true),
            generate_preview: Some(false),
            silent: Some(false),
            await_promise: Some(false),
            include_command_line_api: Some(false),
            user_gesture: Some(false),
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
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("{}: {}", context, e),
            )
        })?;

    if let Some(details) = evaluated.exception_details {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}: {}", context, details.text),
        ));
    }

    let parsed = evaluated.result.value.unwrap_or(serde_json::Value::Null);

    if parsed.get("overlays").and_then(|value| value.as_str()) != Some("ready") {
        return Err(ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "{}: injected overlays are not installed on this page",
                context
            ),
        ));
    }

    Ok(parsed
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

async fn image_policy_for_chat(app: &AppState, chat_id: &str) -> ImagePolicy {
    let model_id = {
        let sessions = app.chat.sessions.read().await;
        let Some(session) = sessions.get(chat_id).cloned() else {
            return ImagePolicy::default();
        };
        drop(sessions);
        let model = {
            let guard = session.lock().await;
            guard.thread.model.clone()
        };
        model
    };
    let Ok(caps) =
        crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0).await
    else {
        return ImagePolicy::default();
    };
    crate::caps::resolve_chat_model(caps, &model_id)
        .map(|model| ImagePolicy::for_model(&model.base))
        .unwrap_or_default()
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

#[derive(Deserialize)]
pub struct ChatIdBody {
    pub chat_id: String,
}

#[derive(Deserialize, Default)]
pub struct BrowserStartBody {
    pub chat_id: String,
    #[serde(default)]
    pub headless: Option<bool>,
    #[serde(default)]
    pub window_bounds: Option<WindowBounds>,
    #[serde(default)]
    pub extra_args: Option<Vec<String>>,
    #[serde(default)]
    pub chromium_sandbox: Option<bool>,
    #[serde(default)]
    pub proxy: Option<BrowserProxyOptions>,
    #[serde(default)]
    pub downloads_dir: Option<PathBuf>,
    #[serde(default)]
    pub ignore_https_errors: Option<bool>,
}

impl BrowserStartBody {
    fn launch_options(&self, base: BrowserLaunchOptions) -> BrowserLaunchOptions {
        BrowserLaunchOptions {
            headless: self.headless.unwrap_or(base.headless),
            window_bounds: self.window_bounds.clone().or(base.window_bounds),
            extra_args: self.extra_args.clone().unwrap_or(base.extra_args),
            chromium_sandbox: self.chromium_sandbox.unwrap_or(base.chromium_sandbox),
            proxy: self.proxy.clone().or(base.proxy),
            downloads_dir: self.downloads_dir.clone().or(base.downloads_dir),
            ignore_https_errors: self.ignore_https_errors.unwrap_or(base.ignore_https_errors),
            ..base
        }
    }
}

#[derive(Deserialize)]
pub struct ScreenshotBody {
    pub chat_id: String,
    #[serde(default)]
    pub full_page: bool,
}

#[derive(Deserialize)]
pub struct ContextBody {
    pub chat_id: String,
    pub max_bytes: Option<usize>,
    pub last_n_actions: Option<usize>,
    #[serde(default)]
    pub skip_cursor: bool,
}

#[derive(Deserialize)]
pub struct CurlBody {
    pub chat_id: String,
    pub request_index: Option<usize>,
}

#[derive(Deserialize)]
pub struct EvalBody {
    pub chat_id: String,
    pub expression: String,
}

#[derive(Deserialize)]
pub struct InjectCssBody {
    pub chat_id: String,
    pub css: String,
    pub id: Option<String>,
}

#[derive(Deserialize)]
pub struct RemoveCssBody {
    pub chat_id: String,
    pub style_id: String,
}

#[derive(Deserialize)]
pub struct DomSnapshotBody {
    pub chat_id: String,
    pub selector: String,
    pub max_chars: Option<usize>,
}

#[derive(Deserialize)]
pub struct RecordAnimationBody {
    pub chat_id: String,
    pub duration_ms: Option<u64>,
    pub fps: Option<u32>,
}

#[derive(Deserialize)]
pub struct HandoffBody {
    pub from_chat_id: String,
    pub to_chat_id: String,
}

#[derive(Serialize, Deserialize)]
struct AccessibilityNode {
    role: String,
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<AccessibilityNode>,
}

pub async fn handle_browser_start(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let post: BrowserStartBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    if let Some((rid, runtime_arc)) = find_runtime_by_chat_id(app.clone(), &post.chat_id).await {
        let (profile_dir, current_options) = {
            let rt = runtime_arc.lock().await;
            (rt.profile_dir.clone(), rt.launch_options.clone())
        };
        let requested_options = post.launch_options(BrowserLaunchOptions {
            headless: BrowserLaunchOptions::default().headless,
            ..current_options.clone()
        });
        if requested_options.headless != current_options.headless {
            drop(runtime_arc);
            let mode = requested_options.mode_label();
            let window_bounds = requested_options.window_bounds.clone();
            let runtime_id = relaunch_runtime_for_chat(
                app.clone(),
                &post.chat_id,
                profile_dir,
                requested_options.clone(),
                window_bounds,
            )
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

            ensure_frame_emitter(app.clone(), &post.chat_id, &runtime_id).await;

            return Ok(json_response(
                StatusCode::OK,
                serde_json::json!({
                    "runtime_id": runtime_id,
                    "status": "started",
                    "headless": requested_options.headless,
                    "previous_headless": current_options.headless,
                    "detail": format!("browser restarted {}: open tabs lost, cookies/localStorage persisted (same profile)", mode)
                }),
            ));
        }

        {
            let mut rt = runtime_arc.lock().await;
            if rt.recording_tab_target_id.is_none() {
                if let Err(e) = setup_recording_for_runtime(&mut rt) {
                    tracing::warn!(
                        "Browser recording setup on attach failed (non-fatal): {}",
                        e
                    );
                }
            }
        }
        ensure_frame_emitter(app.clone(), &post.chat_id, &rid).await;
        return Ok(json_response(
            StatusCode::OK,
            serde_json::json!({
                "runtime_id": rid,
                "status": "already_running",
                "headless": current_options.headless
            }),
        ));
    }

    let cache_dir = gcx.cache_dir.clone();
    let profile_dir = get_browser_profile_dir(&cache_dir, &post.chat_id);
    let launch_options = post.launch_options(BrowserLaunchOptions::default());
    let headless = launch_options.headless;

    let runtime = BrowserRuntime::launch(profile_dir, launch_options).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to launch browser: {}", e),
        )
    })?;

    let mut rt = runtime;
    rt.reattach(&post.chat_id);
    let runtime_id = register_browser_runtime(app.clone(), rt).await;

    let browser_runtimes = gcx.browser_runtimes.clone();
    let runtime_arc = {
        let browser_runtimes = browser_runtimes.lock().await;
        browser_runtimes.get(&runtime_id).cloned()
    };
    if let Some(runtime_arc) = runtime_arc {
        let mut rt = runtime_arc.lock().await;
        if let Err(e) = setup_recording_for_runtime(&mut rt) {
            tracing::warn!("Browser recording setup failed (non-fatal): {}", e);
        }
        rt.frame_emitter_active = true;
    }

    tokio::spawn(browser_frame_emission_task(
        gcx.clone(),
        post.chat_id.clone(),
        runtime_id.clone(),
    ));

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "runtime_id": runtime_id,
            "status": "started",
            "headless": headless
        }),
    ))
}

pub async fn handle_browser_stop(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (rid, _) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    remove_browser_runtime(app.clone(), &rid).await;

    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "status": "stopped"
        }),
    ))
}

pub async fn handle_browser_screenshot(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ScreenshotBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;
    let image_policy = image_policy_for_chat(&app, &post.chat_id).await;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.get_active_tab().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No active tab".to_string(),
        )
    })?;

    let url = tab.get_url();
    let title = tab.get_title().unwrap_or_default();

    let (format_option, mime, quality) = if post.full_page {
        (
            headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png,
            "image/png",
            None,
        )
    } else {
        (
            headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Jpeg,
            "image/jpeg",
            Some(75),
        )
    };

    let screenshot_result = tab
        .call_method(headless_chrome::protocol::cdp::Page::CaptureScreenshot {
            format: Some(format_option),
            clip: None,
            quality,
            from_surface: Some(true),
            capture_beyond_viewport: Some(post.full_page),
            optimize_for_speed: None,
        })
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Screenshot failed: {}", e),
            )
        })?;

    let raw_data = base64::prelude::BASE64_STANDARD
        .decode(&screenshot_result.data)
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Base64 decode failed: {}", e),
            )
        })?;

    let (resized_data, resized_mime) = resize_to_policy(&raw_data, mime, &image_policy)
        .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let b64 = base64::prelude::BASE64_STANDARD.encode(&resized_data);

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "mime": resized_mime,
            "data": b64,
            "url": url,
            "title": title
        }),
    ))
}

pub async fn handle_browser_context(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ContextBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let (url, title) = match rt.get_active_tab() {
        Some(tab) => (tab.get_url(), tab.get_title().unwrap_or_default()),
        None => (String::new(), String::new()),
    };

    let (actions_snapshot, console_snapshot, network_snapshot, mutations_snapshot) =
        if post.skip_cursor {
            (
                &rt.action_buffer[..],
                &rt.console_buffer[..],
                &rt.network_buffer[..],
                &rt.mutation_summary[..],
            )
        } else {
            let a0 = rt.last_send_action_cursor.min(rt.action_buffer.len());
            let c0 = rt.last_send_console_cursor.min(rt.console_buffer.len());
            let n0 = rt.last_send_network_cursor.min(rt.network_buffer.len());
            let m0 = rt.last_send_mutation_cursor.min(rt.mutation_summary.len());
            (
                &rt.action_buffer[a0..],
                &rt.console_buffer[c0..],
                &rt.network_buffer[n0..],
                &rt.mutation_summary[m0..],
            )
        };

    let mut actions_json = serde_json::to_value(actions_snapshot).unwrap_or(serde_json::json!([]));
    let mut console_json = serde_json::to_value(console_snapshot).unwrap_or(serde_json::json!([]));
    let mut network_json = serde_json::to_value(network_snapshot).unwrap_or(serde_json::json!([]));
    let mutations_json = serde_json::to_value(mutations_snapshot).unwrap_or(serde_json::json!([]));

    if let Some(last_n) = post.last_n_actions {
        if let Some(arr) = actions_json.as_array() {
            if arr.len() > last_n {
                actions_json = serde_json::json!(arr[arr.len() - last_n..]);
            }
        }
    }

    // Apply a default cap when skip_cursor is used to avoid unbounded payloads
    let effective_max_bytes = post.max_bytes.or_else(|| {
        if post.skip_cursor {
            Some(512 * 1024)
        } else {
            None
        }
    });

    if let Some(max_bytes) = effective_max_bytes {
        let bytes_before_trim = serde_json::to_string(&actions_json)
            .unwrap_or_default()
            .len()
            + serde_json::to_string(&console_json)
                .unwrap_or_default()
                .len()
            + serde_json::to_string(&network_json)
                .unwrap_or_default()
                .len()
            + serde_json::to_string(&mutations_json)
                .unwrap_or_default()
                .len();
        if bytes_before_trim > max_bytes {
            let trim_arrays = |arr: &mut serde_json::Value| {
                if let Some(a) = arr.as_array_mut() {
                    let budget = max_bytes / 4;
                    let n = a.len();
                    if n == 0 {
                        return;
                    }
                    // Estimate bytes per item and keep as many tail items as fit
                    let total_len = serde_json::to_string(a).unwrap_or_default().len();
                    let bytes_per_item = total_len / n;
                    let keep = if bytes_per_item > 0 {
                        (budget / bytes_per_item).max(1)
                    } else {
                        n
                    };
                    if keep < n {
                        a.drain(0..n - keep);
                    }
                }
            };
            trim_arrays(&mut actions_json);
            trim_arrays(&mut console_json);
            trim_arrays(&mut network_json);
        }
    }

    let total_bytes = serde_json::to_string(&actions_json)
        .unwrap_or_default()
        .len()
        + serde_json::to_string(&console_json)
            .unwrap_or_default()
            .len()
        + serde_json::to_string(&network_json)
            .unwrap_or_default()
            .len()
        + serde_json::to_string(&mutations_json)
            .unwrap_or_default()
            .len();

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "url": url,
            "title": title,
            "actions": actions_json,
            "console": console_json,
            "network": network_json,
            "mutations": mutations_json,
            "total_bytes": total_bytes
        }),
    ))
}

pub async fn handle_browser_context_commit(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let mut rt = runtime_arc.lock().await;
    let url = rt.get_active_tab().map(|tab| tab.get_url());
    rt.commit_cursors(url.as_deref());

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "status": "committed"
        }),
    ))
}

pub async fn handle_browser_element_pick(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.get_active_tab().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No active tab".to_string(),
        )
    })?;

    eval_overlay(
        &tab,
        &format!("startPicker({})", PICKER_PAGE_TIMEOUT_MS),
        "Failed to start element picker",
    )?;

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "status": "picker_active"
        }),
    ))
}

pub async fn handle_browser_element_pick_result(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.get_active_tab().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No active tab".to_string(),
        )
    })?;

    let parsed = eval_overlay(&tab, "readPicked()", "Failed to read picker result")?;
    if parsed.is_null() {
        return Ok(json_response(
            StatusCode::OK,
            serde_json::json!({ "status": "waiting" }),
        ));
    }
    Ok(json_response(StatusCode::OK, parsed))
}

pub async fn handle_browser_element_pick_cancel(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.get_active_tab().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No active tab".to_string(),
        )
    })?;

    eval_overlay(&tab, "cancelPicker()", "Failed to cancel picker")?;

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "status": "cancelled"
        }),
    ))
}

pub async fn handle_browser_annotate_start(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.get_active_tab().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No active tab".to_string(),
        )
    })?;

    let result = eval_overlay(&tab, "startAnnotate()", "Failed to start annotation overlay")?;
    let status = result.as_str().unwrap_or("started").to_string();

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "status": status
        }),
    ))
}

pub async fn handle_browser_annotate_result(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.get_active_tab().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No active tab".to_string(),
        )
    })?;

    let parsed = eval_overlay(&tab, "readAnnotations()", "Failed to read annotations")?;
    if parsed.is_null() {
        return Ok(json_response(
            StatusCode::OK,
            serde_json::json!({ "annotations": [], "active": false }),
        ));
    }
    Ok(json_response(StatusCode::OK, parsed))
}

pub async fn handle_browser_annotate_clear(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.get_active_tab().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No active tab".to_string(),
        )
    })?;

    eval_overlay(&tab, "clearAnnotations()", "Failed to clear annotations")?;

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "status": "cleared"
        }),
    ))
}

pub async fn handle_browser_curl(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: CurlBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let idx = post
        .request_index
        .unwrap_or_else(|| rt.network_buffer.len().saturating_sub(1));
    let entry = rt.network_buffer.get(idx).ok_or_else(|| {
        ScratchError::new(
            StatusCode::NOT_FOUND,
            "No network request at specified index".to_string(),
        )
    })?;

    let curl = format_curl_minimal(entry);

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "curl": curl,
            "url": entry.url,
            "method": entry.method,
            "status": entry.status.unwrap_or(0)
        }),
    ))
}

fn format_curl_minimal(entry: &crate::integrations::browser_types::NetworkEntry) -> String {
    let method_flag = if entry.method.is_empty() || entry.method == "GET" {
        String::new()
    } else {
        format!(" -X {}", entry.method)
    };
    format!("curl{} '{}'", method_flag, entry.url)
}

pub async fn handle_browser_eval(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: EvalBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let tab = {
        let rt = runtime_arc.lock().await;
        rt.get_active_tab().ok_or_else(|| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "No active tab".to_string(),
            )
        })?
    };

    let steps = vec![crate::integrations::browser_models::BrowserStep::Eval {
        expression: post.expression.clone(),
    }];
    let report = tokio::task::block_in_place(|| {
        browser_controller::execute_steps(&tab, &steps, &ImagePolicy::default())
    });

    // Push agent timeline entry
    {
        let mut rt = runtime_arc.lock().await;
        rt.touch();
        for sr in &report.steps {
            let action_type = if sr.ok { "eval" } else { "error" };
            rt.push_agent_action(action_type, &sr.summary);
        }
    }

    let first_step = report.steps.first();
    if !report.ok {
        let error_msg = first_step
            .and_then(|sr| sr.error.as_ref())
            .cloned()
            .unwrap_or_else(|| "Eval failed".to_string());
        return Ok(json_response(
            StatusCode::OK,
            serde_json::json!({
                "ok": false,
                "error": error_msg,
                "result": serde_json::Value::Null,
            }),
        ));
    }

    let result_str = first_step
        .and_then(|sr| sr.data.as_ref())
        .and_then(|d| d.get("value"))
        .map(|v| {
            if let Some(s) = v.as_str() {
                s.to_string()
            } else {
                serde_json::to_string(v).unwrap_or_default()
            }
        })
        .unwrap_or_else(|| "undefined".to_string());

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "ok": true,
            "result": result_str
        }),
    ))
}

pub async fn handle_browser_inject_css(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: InjectCssBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.get_active_tab().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No active tab".to_string(),
        )
    })?;

    let style_id = post
        .id
        .unwrap_or_else(|| format!("refact-css-{}", uuid::Uuid::new_v4()));
    let id_json = serde_json::to_string(&style_id).unwrap_or_else(|_| "\"\"".to_string());
    let css_json = serde_json::to_string(&post.css).unwrap_or_else(|_| "\"\"".to_string());
    let js = format!(
        r#"(function() {{
            var id = {id};
            var existing = document.getElementById(id);
            if (existing) existing.remove();
            var style = document.createElement('style');
            style.id = id;
            style.textContent = {css};
            document.head.appendChild(style);
        }})()"#,
        id = id_json,
        css = css_json,
    );

    tab.evaluate(&js, false).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("CSS injection failed: {}", e),
        )
    })?;

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "style_id": style_id
        }),
    ))
}

pub async fn handle_browser_remove_css(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: RemoveCssBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.get_active_tab().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No active tab".to_string(),
        )
    })?;

    let id_json = serde_json::to_string(&post.style_id).unwrap_or_else(|_| "\"\"".to_string());
    let js = format!(
        r#"(function() {{ var el = document.getElementById({id}); if (el) el.remove(); }})()"#,
        id = id_json,
    );

    tab.evaluate(&js, false).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("CSS removal failed: {}", e),
        )
    })?;

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "status": "removed"
        }),
    ))
}

pub async fn handle_browser_dom_snapshot(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: DomSnapshotBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let tab = {
        let rt = runtime_arc.lock().await;
        rt.get_active_tab().ok_or_else(|| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "No active tab".to_string(),
            )
        })?
    };

    let steps = vec![
        crate::integrations::browser_models::BrowserStep::DomSnapshot {
            selector: post.selector.clone(),
            max_chars: post.max_chars,
        },
    ];
    let report = tokio::task::block_in_place(|| {
        browser_controller::execute_steps(&tab, &steps, &ImagePolicy::default())
    });

    // Push agent timeline entry
    {
        let mut rt = runtime_arc.lock().await;
        rt.touch();
        for sr in &report.steps {
            let action_type = if sr.ok { "dom_snapshot" } else { "error" };
            rt.push_agent_action(action_type, &sr.summary);
        }
    }

    let first_step = report.steps.first();
    if !report.ok {
        let error_msg = first_step
            .and_then(|sr| sr.error.as_ref())
            .cloned()
            .unwrap_or_else(|| "DOM snapshot failed".to_string());
        return Ok(json_response(
            StatusCode::OK,
            serde_json::json!({
                "ok": false,
                "error": error_msg,
                "html": "",
                "truncated": false,
            }),
        ));
    }

    let html = first_step
        .and_then(|sr| sr.data.as_ref())
        .and_then(|d| d.get("html"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let max_chars = post.max_chars.unwrap_or(50000);
    let char_count = html.chars().count();
    let truncated = char_count > max_chars;
    let html_out: String = if truncated {
        html.chars().take(max_chars).collect()
    } else {
        html
    };

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "ok": true,
            "html": html_out,
            "truncated": truncated
        }),
    ))
}

pub async fn handle_browser_accessibility(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.get_active_tab().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No active tab".to_string(),
        )
    })?;

    let js = r#"
    (function() {
        function walk(el) {
            var role = el.getAttribute && el.getAttribute('role') || el.tagName || '';
            var name = el.getAttribute && (el.getAttribute('aria-label') || el.getAttribute('alt') || el.getAttribute('title') || '') || '';
            if (!name && el.textContent) {
                name = el.textContent.trim().substring(0, 100);
            }
            var children = [];
            for (var i = 0; i < (el.children ? el.children.length : 0); i++) {
                children.push(walk(el.children[i]));
            }
            return {role: role, name: name, children: children};
        }
        return JSON.stringify(walk(document.body));
    })()
    "#;

    let result = tab.evaluate(js, false).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Accessibility query failed: {}", e),
        )
    })?;

    let tree: Vec<AccessibilityNode> = match result.value {
        Some(val) => {
            let json_str = val.as_str().unwrap_or("[]");
            match serde_json::from_str::<AccessibilityNode>(json_str) {
                Ok(node) => vec![node],
                Err(_) => vec![],
            }
        }
        None => vec![],
    };

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "tree": tree
        }),
    ))
}

pub async fn handle_browser_record_animation(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: RecordAnimationBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.get_active_tab().ok_or_else(|| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No active tab".to_string(),
        )
    })?;

    let duration_ms = post.duration_ms.unwrap_or(2000).clamp(100, 10000);
    let fps = post.fps.unwrap_or(5).clamp(1, 60);
    let frame_interval = std::time::Duration::from_millis(1000 / fps as u64);
    let num_frames = (duration_ms as f64 / frame_interval.as_millis() as f64).ceil() as u32;

    drop(rt);

    let mut frames = Vec::new();
    for i in 0..num_frames {
        let screenshot_result = tab
            .call_method(headless_chrome::protocol::cdp::Page::CaptureScreenshot {
                format: Some(
                    headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Jpeg,
                ),
                clip: None,
                quality: Some(60),
                from_surface: Some(true),
                capture_beyond_viewport: Some(false),
                optimize_for_speed: Some(true),
            })
            .map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Frame capture failed: {}", e),
                )
            })?;

        let raw = base64::prelude::BASE64_STANDARD
            .decode(&screenshot_result.data)
            .map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Base64 decode failed: {}", e),
                )
            })?;

        let frame_policy = ImagePolicy {
            max_side: FRAME_MAX_SIDE,
            preferred_side: FRAME_MAX_SIDE,
            format: ImageFormat::Jpeg,
            quality: Some(60),
            ..ImagePolicy::default()
        };
        let (resized, _) = resize_to_policy(&raw, "image/jpeg", &frame_policy)
            .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
        let b64 = base64::prelude::BASE64_STANDARD.encode(&resized);
        let timestamp = i as f64 * frame_interval.as_millis() as f64;

        frames.push(serde_json::json!({
            "mime": "image/jpeg",
            "data": b64,
            "timestamp": timestamp
        }));

        if i < num_frames - 1 {
            tokio::time::sleep(frame_interval).await;
        }
    }

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "frames": frames
        }),
    ))
}

pub async fn handle_browser_handoff(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: HandoffBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (rid, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.from_chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.from_chat_id),
            )
        })?;

    let (profile_dir, tab_urls, window_bounds, mask_passwords, attach_screenshot) = {
        let mut rt = runtime_arc.lock().await;
        let profile_dir = rt.profile_dir.to_string_lossy().to_string();
        let tab_urls: Vec<String> = rt
            .browser
            .get_tabs()
            .lock()
            .map(|tabs| tabs.iter().map(|t| t.get_url()).collect())
            .unwrap_or_default();
        let window_bounds = rt.window_bounds().cloned();
        let mask_passwords = rt.mask_passwords();
        let attach_screenshot = false;

        rt.detach();
        rt.reattach(&post.to_chat_id);
        rt.touch();

        (
            profile_dir,
            tab_urls,
            window_bounds,
            mask_passwords,
            attach_screenshot,
        )
    };

    Ok(json_response(
        StatusCode::OK,
        serde_json::json!({
            "runtime_id": rid,
            "status": "transferred",
            "from_chat_id": post.from_chat_id,
            "to_chat_id": post.to_chat_id,
            "profile_dir": profile_dir,
            "tab_urls": tab_urls,
            "window_bounds": window_bounds,
            "mask_passwords": mask_passwords,
            "attach_screenshot_on_send": attach_screenshot
        }),
    ))
}

pub async fn handle_browser_status(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    match find_runtime_by_chat_id(app.clone(), &post.chat_id).await {
        Some((rid, runtime_arc)) => {
            let rt = runtime_arc.lock().await;
            let tab_infos = rt.list_tab_infos();
            let tab_urls: Vec<String> = tab_infos.iter().map(|t| t.url.clone()).collect();
            let (url, title) = match rt.get_active_tab() {
                Some(tab) => (tab.get_url(), tab.get_title().unwrap_or_default()),
                None => (String::new(), String::new()),
            };

            Ok(json_response(
                StatusCode::OK,
                serde_json::json!({
                    "runtime_id": rid,
                    "connected": rt.is_connected,
                    "headless": rt.headless(),
                    "window_bounds": rt.window_bounds(),
                    "active_tab": rt.active_tab_target_id().map(|s| s.to_string()),
                    "url": url,
                    "title": title,
                    "tab_urls": tab_urls,
                    "tabs": tab_infos.iter().map(|t| serde_json::json!({
                        "tab_id": t.id,
                        "url": t.url,
                        "title": t.title,
                    })).collect::<Vec<_>>(),
                    "idle_seconds": rt.last_activity.elapsed().as_secs(),
                    "idle_timeout": rt.idle_timeout.as_secs()
                }),
            ))
        }
        None => Ok(json_response(
            StatusCode::OK,
            serde_json::json!({
                "runtime_id": null,
                "connected": false
            }),
        )),
    }
}

pub async fn handle_browser_action(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    #[derive(Deserialize)]
    struct ActionBody {
        chat_id: String,
        #[serde(flatten)]
        request: BrowserActionRequest,
    }

    let post: ActionBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(app.clone(), &post.chat_id)
        .await
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("No browser runtime for chat_id={}", post.chat_id),
            )
        })?;
    let image_policy = image_policy_for_chat(&app, &post.chat_id).await;
    let report = browser_controller::execute_request_with_runtime_validated(
        runtime_arc,
        post.request,
        &image_policy,
        app.gcx.clone(),
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    let report_json = serde_json::to_value(&report).unwrap_or_default();
    Ok(json_response(StatusCode::OK, report_json))
}

fn format_ts(ts_ms: f64) -> String {
    let total_secs = (ts_ms / 1000.0) as u64;
    let hours = (total_secs / 3600) % 24;
    let minutes = (total_secs / 60) % 60;
    let seconds = total_secs % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

fn recorder_events_to_timeline(
    actions: &[RecorderEvent],
    console: &[ConsoleEntry],
    network: &[NetworkEntry],
) -> Vec<TimelineEntry> {
    let mut entries = Vec::new();

    for event in actions {
        let ts = format_ts(event.timestamp());
        let (entry_type, summary) = match event {
            RecorderEvent::Navigation { url, .. } => {
                ("navigation".to_string(), format!("navigate → {}", url))
            }
            RecorderEvent::Click {
                selector,
                text,
                x,
                y,
                ..
            } => {
                let label = if text.is_empty() {
                    selector.clone()
                } else {
                    format!("{} \"{}\"", selector, text)
                };
                (
                    "click".to_string(),
                    format!("click → {} (x:{}, y:{})", label, *x as i32, *y as i32),
                )
            }
            RecorderEvent::Input { selector, .. } => {
                ("input".to_string(), format!("input → {}", selector))
            }
            RecorderEvent::Keypress { key, modifiers, .. } => {
                let mods = if modifiers.is_empty() {
                    String::new()
                } else {
                    format!("{}+", modifiers.join("+"))
                };
                (
                    "keypress".to_string(),
                    format!("keypress → {}{}", mods, key),
                )
            }
            RecorderEvent::Submit {
                selector,
                method,
                action,
                ..
            } => (
                "submit".to_string(),
                format!("submit → {} {} {}", selector, method, action),
            ),
            RecorderEvent::Scroll {
                scroll_x, scroll_y, ..
            } => (
                "scroll".to_string(),
                format!("scroll → ({}, {})", *scroll_x as i32, *scroll_y as i32),
            ),
            RecorderEvent::MutationSummary {
                added,
                removed,
                changed,
                ..
            } => (
                "mutation".to_string(),
                format!("dom-change → +{} -{} ~{}", added, removed, changed),
            ),
            RecorderEvent::ToolbarAction { action, .. } => {
                ("toolbar".to_string(), format!("toolbar → {}", action))
            }
        };
        entries.push(TimelineEntry {
            timestamp: ts,
            source: "user".to_string(),
            entry_type,
            summary,
            details: None,
        });
    }

    for entry in console {
        entries.push(TimelineEntry {
            timestamp: format_ts(entry.timestamp),
            source: "agent".to_string(),
            entry_type: "console".to_string(),
            summary: format!("[{}] {}", entry.level, entry.text),
            details: None,
        });
    }

    for entry in network {
        entries.push(TimelineEntry {
            timestamp: format_ts(entry.timestamp),
            source: "agent".to_string(),
            entry_type: "network".to_string(),
            summary: format!(
                "{} {}{}",
                if entry.method.is_empty() {
                    "GET"
                } else {
                    &entry.method
                },
                entry.url,
                entry
                    .status
                    .map(|status| format!(" → {}", status))
                    .unwrap_or_default()
            ),
            details: None,
        });
    }

    entries
}

pub async fn browser_frame_emission_task(
    gcx: Arc<GlobalContext>,
    chat_id: String,
    runtime_id: String,
) {
    let sessions = gcx.chat_sessions.clone();
    let mut last_status_json: Option<String> = None;

    loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        if gcx.shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        let runtime_arc = {
            let browser_runtimes = gcx.browser_runtimes.clone();
            let browser_runtimes = browser_runtimes.lock().await;
            browser_runtimes.get(&runtime_id).cloned()
        };
        let runtime_arc = match runtime_arc {
            Some(arc) => arc,
            None => break,
        };

        let (
            toolbar_actions,
            timeline_entries,
            actions_len,
            console_len,
            network_len,
            mutation_len,
        ) = {
            let mut rt = runtime_arc.lock().await;
            rt.drain_raw_events();
            let toolbar_actions = rt.drain_toolbar_actions();
            let (new_actions, new_console, new_network) = rt.flush_timeline_events();
            let agent_actions = rt.drain_agent_actions();
            let mut timeline_entries =
                recorder_events_to_timeline(&new_actions, &new_console, &new_network);
            for aa in &agent_actions {
                timeline_entries.push(TimelineEntry {
                    timestamp: format_ts(aa.timestamp_ms),
                    source: "agent".to_string(),
                    entry_type: aa.action_type.clone(),
                    summary: aa.summary.clone(),
                    details: None,
                });
            }
            timeline_entries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
            (
                toolbar_actions,
                timeline_entries,
                rt.action_buffer.len(),
                rt.console_buffer.len(),
                rt.network_buffer.len(),
                rt.mutation_summary.len(),
            )
        };

        {
            let rt = runtime_arc.lock().await;
            ensure_injection_into_all_tabs(&rt);
        }

        {
            let rt = runtime_arc.lock().await;
            let all_tabs: Vec<Arc<headless_chrome::Tab>> = rt
                .browser
                .get_tabs()
                .lock()
                .map(|tabs| tabs.iter().cloned().collect())
                .unwrap_or_default();
            for tab in all_tabs {
                let url = tab.get_url();
                if url.starts_with("chrome://") || url == "about:blank" {
                    let is_active = rt
                        .active_tab_target_id()
                        .map(|id| id == tab.get_target_id())
                        .unwrap_or(false);
                    if !is_active
                        && rt.recording_tab_target_id.as_deref() != Some(tab.get_target_id())
                    {
                        let _ = tab.close(false);
                    }
                }
            }
        }

        let tab_for_counts = {
            let rt = runtime_arc.lock().await;
            rt.get_active_tab()
        };
        if let Some(tab) = tab_for_counts {
            let js = format!(
                "window.__refact_toolbar_setCounts && window.__refact_toolbar_setCounts({{actions:{},console:{},network:{},mutations:{}}});",
                actions_len, console_len, network_len, mutation_len
            );
            let _ = tab.evaluate(&js, false);
        }

        if !toolbar_actions.is_empty() || !timeline_entries.is_empty() {
            let session_arc = {
                let sessions_locked = sessions.read().await;
                sessions_locked.get(&chat_id).cloned()
            };
            if let Some(session_arc) = session_arc {
                let mut session = session_arc.lock().await;
                for action in toolbar_actions {
                    session.emit(ChatEvent::BrowserToolbarAction { action });
                }
                if !timeline_entries.is_empty() {
                    session.emit(ChatEvent::BrowserTimeline {
                        events: timeline_entries,
                    });
                }
            }
        }

        let status_event = {
            let rt = runtime_arc.lock().await;
            let tab_infos = rt
                .list_tab_infos()
                .into_iter()
                .map(|t| BrowserTabInfo {
                    tab_id: t.id,
                    url: t.url,
                    title: t.title,
                })
                .collect::<Vec<_>>();
            let (url, title) = match rt.get_active_tab() {
                Some(tab) => (
                    Some(tab.get_url()).filter(|s| !s.is_empty()),
                    Some(tab.get_title().unwrap_or_default()).filter(|s| !s.is_empty()),
                ),
                None => (None, None),
            };
            ChatEvent::BrowserStatus {
                runtime_id: runtime_id.clone(),
                connected: rt.is_connected,
                active_tab: rt.active_tab_target_id().map(|s| s.to_string()),
                url,
                title,
                tabs: tab_infos,
            }
        };
        let status_json = serde_json::to_string(&status_event).ok();
        if status_json != last_status_json {
            last_status_json = status_json;
            let session_arc = {
                let sessions_locked = sessions.read().await;
                sessions_locked.get(&chat_id).cloned()
            };
            if let Some(session_arc) = session_arc {
                let mut session = session_arc.lock().await;
                session.emit(status_event);
            }
        }

        let tab = {
            let rt = runtime_arc.lock().await;
            rt.get_active_tab()
        };
        let tab = match tab {
            Some(t) => t,
            None => continue,
        };

        let screenshot_result =
            tab.call_method(headless_chrome::protocol::cdp::Page::CaptureScreenshot {
                format: Some(
                    headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Jpeg,
                ),
                clip: None,
                quality: Some(60),
                from_surface: Some(true),
                capture_beyond_viewport: Some(false),
                optimize_for_speed: Some(true),
            });

        let raw_data = match screenshot_result {
            Ok(r) => match base64::prelude::BASE64_STANDARD.decode(&r.data) {
                Ok(d) => d,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        let new_hash = compute_frame_hash(&raw_data);

        {
            let rt = runtime_arc.lock().await;
            if !rt.should_emit_frame(new_hash) {
                continue;
            }
        }

        let frame_policy = ImagePolicy {
            max_side: FRAME_MAX_SIDE,
            preferred_side: FRAME_MAX_SIDE,
            format: ImageFormat::Jpeg,
            quality: Some(60),
            ..ImagePolicy::default()
        };
        let resized = match resize_to_policy(&raw_data, "image/jpeg", &frame_policy) {
            Ok((resized, _)) => resized,
            Err(_) => continue,
        };
        let b64 = base64::prelude::BASE64_STANDARD.encode(&resized);
        let tab_id = tab.get_target_id().clone();

        {
            let mut rt = runtime_arc.lock().await;
            rt.update_frame_state(new_hash, resized);
        }

        let session_arc = {
            let sessions_locked = sessions.read().await;
            sessions_locked.get(&chat_id).cloned()
        };
        if let Some(session_arc) = session_arc {
            let mut session = session_arc.lock().await;
            session.emit(ChatEvent::BrowserFrame {
                tab_id,
                mime: "image/jpeg".to_string(),
                data: b64,
                diff_boxes: vec![],
                changed_text: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use hyper::body::to_bytes;
    use tower::ServiceExt;

    #[test]
    fn test_handoff_body_deserialize() {
        let json = r#"{"from_chat_id":"chat-1","to_chat_id":"chat-2"}"#;
        let body: HandoffBody = serde_json::from_str(json).unwrap();
        assert_eq!(body.from_chat_id, "chat-1");
        assert_eq!(body.to_chat_id, "chat-2");
    }

    #[test]
    fn test_format_curl_minimal_get() {
        let entry = crate::integrations::browser_types::NetworkEntry {
            timestamp: 1.0,
            method: "GET".to_string(),
            url: "https://example.com/api?key=123".to_string(),
            resource_type: "Fetch".to_string(),
            status: Some(200),
            ..crate::integrations::browser_types::NetworkEntry::default()
        };
        let curl = format_curl_minimal(&entry);
        assert_eq!(curl, "curl 'https://example.com/api?key=123'");
    }

    #[test]
    fn test_format_curl_minimal_post() {
        let entry = crate::integrations::browser_types::NetworkEntry {
            timestamp: 1.0,
            method: "POST".to_string(),
            url: "https://example.com/api".to_string(),
            resource_type: "XHR".to_string(),
            status: Some(201),
            ..crate::integrations::browser_types::NetworkEntry::default()
        };
        let curl = format_curl_minimal(&entry);
        assert_eq!(curl, "curl -X POST 'https://example.com/api'");
    }

    #[test]
    fn test_format_curl_minimal_empty_method() {
        let entry = crate::integrations::browser_types::NetworkEntry {
            timestamp: 1.0,
            method: String::new(),
            url: "https://example.com".to_string(),
            resource_type: "Document".to_string(),
            status: Some(200),
            ..crate::integrations::browser_types::NetworkEntry::default()
        };
        let curl = format_curl_minimal(&entry);
        assert_eq!(curl, "curl 'https://example.com'");
    }

    #[test]
    fn test_resize_screenshot_small_image() {
        let img = image::RgbImage::new(100, 100);
        let mut buf = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut buf),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
        let policy = refact_core::image_policy::ImagePolicy::from_metadata(Some(800), Some(800));
        let (result, mime) =
            refact_core::image_policy::resize_to_policy(&buf, "image/jpeg", &policy).unwrap();
        assert!(!result.is_empty());
        assert_eq!(mime, "image/png");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_browser_action_route_rejects_invalid_json() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app_state = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let router = crate::http::routers::make_refact_http_server(app_state);

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/browser/action")
                    .header("content-type", "application/json")
                    .body(Body::from("{not valid json"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let body = to_bytes(response.into_body()).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let detail = payload["detail"].as_str().unwrap_or_default();
        assert!(detail.contains("JSON problem"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_browser_action_route_returns_not_found_without_runtime() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app_state = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let router = crate::http::routers::make_refact_http_server(app_state);

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/browser/action")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "chat_id": "missing-chat",
                            "steps": [{"action": "screenshot"}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = to_bytes(response.into_body()).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let detail = payload["detail"].as_str().unwrap_or_default();
        assert!(detail.contains("No browser runtime for chat_id=missing-chat"));
    }
}
