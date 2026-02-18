use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use base64::Engine;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::integrations::browser_runtime::{
    BrowserRuntime, get_browser_profile_dir, register_browser_runtime, remove_browser_runtime,
};

async fn find_runtime_by_chat_id(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
) -> Option<(String, Arc<tokio::sync::Mutex<BrowserRuntime>>)> {
    let gcx_locked = gcx.read().await;
    for (rid, arc) in &gcx_locked.browser_runtimes {
        let rt = arc.lock().await;
        if rt.attached_chat_id.as_deref() == Some(chat_id) {
            return Some((rid.clone(), arc.clone()));
        }
    }
    None
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
}

#[derive(Deserialize)]
pub struct ElementPickResultBody {
    pub chat_id: String,
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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    if let Some((rid, _)) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await {
        return Ok(json_response(StatusCode::OK, serde_json::json!({
            "runtime_id": rid,
            "status": "already_running"
        })));
    }

    let cache_dir = gcx.read().await.cache_dir.clone();
    let profile_dir = get_browser_profile_dir(&cache_dir, &post.chat_id);

    let runtime = BrowserRuntime::launch(
        profile_dir,
        None,
        None,
        None,
        true,
    ).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to launch browser: {}", e))
    })?;

    let mut rt = runtime;
    rt.reattach(&post.chat_id);
    let runtime_id = register_browser_runtime(gcx.clone(), rt).await;

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "runtime_id": runtime_id,
        "status": "started"
    })))
}

pub async fn handle_browser_stop(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (rid, _) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    remove_browser_runtime(gcx.clone(), &rid).await;

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "status": "stopped"
    })))
}

pub async fn handle_browser_screenshot(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ScreenshotBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.browser.get_tabs().lock().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {}", e))
    })?.first().cloned().ok_or_else(|| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No active tab".to_string())
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

    let screenshot_result = tab.call_method(
        headless_chrome::protocol::cdp::Page::CaptureScreenshot {
            format: Some(format_option),
            clip: None,
            quality,
            from_surface: Some(true),
            capture_beyond_viewport: Some(post.full_page),
            optimize_for_speed: None,
        },
    ).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Screenshot failed: {}", e))
    })?;

    let raw_data = base64::prelude::BASE64_STANDARD
        .decode(&screenshot_result.data)
        .map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Base64 decode failed: {}", e))
        })?;

    let resized_data = resize_screenshot(&raw_data, 800, mime)?;
    let b64 = base64::prelude::BASE64_STANDARD.encode(&resized_data);

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "mime": mime,
        "data": b64,
        "url": url,
        "title": title
    })))
}

fn resize_screenshot(data: &[u8], max_dim: u32, mime: &str) -> Result<Vec<u8>, ScratchError> {
    let format = if mime == "image/png" {
        image::ImageFormat::Png
    } else {
        image::ImageFormat::Jpeg
    };
    let reader = image::ImageReader::with_format(std::io::Cursor::new(data), format);
    let mut img = reader.decode().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Image decode failed: {}", e))
    })?;

    let scale = max_dim as f32 / std::cmp::max(img.width(), img.height()) as f32;
    if scale < 1.0 {
        let nw = (scale * img.width() as f32) as u32;
        let nh = (scale * img.height() as f32) as u32;
        img = img.resize(nw, nh, image::imageops::FilterType::Lanczos3);
    }

    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), format).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Image encode failed: {}", e))
    })?;
    Ok(out)
}

pub async fn handle_browser_context(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ContextBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let rt = runtime_arc.lock().await;

    let url = {
        let tabs = rt.browser.get_tabs().lock().map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {}", e))
        })?;
        tabs.first().map(|t| t.get_url()).unwrap_or_default()
    };
    let title = {
        let tabs = rt.browser.get_tabs().lock().map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {}", e))
        })?;
        tabs.first().and_then(|t| t.get_title().ok()).unwrap_or_default()
    };

    let actions_snapshot = &rt.action_buffer[rt.last_send_action_cursor..];
    let console_snapshot = &rt.console_buffer[rt.last_send_console_cursor..];
    let network_snapshot = &rt.network_buffer[rt.last_send_network_cursor..];
    let mutations_snapshot = &rt.mutation_summary[rt.last_send_mutation_cursor..];

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

    let total_bytes = serde_json::to_string(&actions_json).unwrap_or_default().len()
        + serde_json::to_string(&console_json).unwrap_or_default().len()
        + serde_json::to_string(&network_json).unwrap_or_default().len()
        + serde_json::to_string(&mutations_json).unwrap_or_default().len();

    if let Some(max_bytes) = post.max_bytes {
        if total_bytes > max_bytes {
            let trim_arrays = |arr: &mut serde_json::Value| {
                if let Some(a) = arr.as_array_mut() {
                    while serde_json::to_string(a).unwrap_or_default().len() > max_bytes / 4 && !a.is_empty() {
                        a.remove(0);
                    }
                }
            };
            trim_arrays(&mut actions_json);
            trim_arrays(&mut console_json);
            trim_arrays(&mut network_json);
        }
    }

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "url": url,
        "title": title,
        "actions": actions_json,
        "console": console_json,
        "network": network_json,
        "mutations": mutations_json,
        "total_bytes": total_bytes
    })))
}

pub async fn handle_browser_context_commit(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let mut rt = runtime_arc.lock().await;
    rt.flush_action_buffer();
    rt.flush_console_buffer();
    rt.flush_network_buffer();
    rt.flush_mutation_summary();

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "status": "committed"
    })))
}

pub async fn handle_browser_element_pick(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.browser.get_tabs().lock().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {}", e))
    })?.first().cloned().ok_or_else(|| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No active tab".to_string())
    })?;

    let picker_js = r#"
    (function() {
        if (window.__refact_picker_active) return;
        window.__refact_picker_active = true;
        window.__refact_picked_element = null;
        var overlay = document.createElement('div');
        overlay.id = '__refact_picker_overlay';
        overlay.style.cssText = 'position:fixed;top:0;left:0;width:100%;height:100%;z-index:2147483647;cursor:crosshair;';
        document.body.appendChild(overlay);
        overlay.addEventListener('mousemove', function(e) {
            overlay.style.display = 'none';
            var el = document.elementFromPoint(e.clientX, e.clientY);
            overlay.style.display = '';
            if (el) el.style.outline = '2px solid red';
        });
        overlay.addEventListener('click', function(e) {
            e.preventDefault();
            e.stopPropagation();
            overlay.style.display = 'none';
            var el = document.elementFromPoint(e.clientX, e.clientY);
            overlay.remove();
            window.__refact_picker_active = false;
            if (el) {
                var rect = el.getBoundingClientRect();
                var sel = el.id ? '#' + el.id : el.tagName.toLowerCase();
                window.__refact_picked_element = {
                    selector: sel,
                    innerText: (el.innerText || '').substring(0, 500),
                    bbox: {x: rect.x, y: rect.y, width: rect.width, height: rect.height}
                };
            }
        }, {once: true});
    })()
    "#;

    tab.evaluate(picker_js, false).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to inject picker: {}", e))
    })?;

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "status": "picker_active"
    })))
}

pub async fn handle_browser_element_pick_result(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.browser.get_tabs().lock().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {}", e))
    })?.first().cloned().ok_or_else(|| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No active tab".to_string())
    })?;

    let result = tab.evaluate("JSON.stringify(window.__refact_picked_element)", false).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read picker result: {}", e))
    })?;

    match result.value {
        Some(val) => {
            if val.is_null() {
                Ok(json_response(StatusCode::OK, serde_json::json!({ "status": "waiting" })))
            } else {
                let parsed: serde_json::Value = match val.as_str() {
                    Some(s) => serde_json::from_str(s).unwrap_or(serde_json::json!({ "status": "waiting" })),
                    None => val,
                };
                Ok(json_response(StatusCode::OK, parsed))
            }
        }
        None => Ok(json_response(StatusCode::OK, serde_json::json!({ "status": "waiting" }))),
    }
}

pub async fn handle_browser_curl(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: CurlBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let rt = runtime_arc.lock().await;

    let idx = post.request_index.unwrap_or_else(|| rt.network_buffer.len().saturating_sub(1));
    let entry = rt.network_buffer.get(idx).ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, "No network request at specified index".to_string())
    })?;

    let curl = sanitize_curl(entry);

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "curl": curl,
        "url": entry.url,
        "method": entry.method,
        "status": entry.status.unwrap_or(0)
    })))
}

fn sanitize_curl(entry: &crate::integrations::browser_types::NetworkEntry) -> String {
    let method_flag = if entry.method.is_empty() || entry.method == "GET" {
        String::new()
    } else {
        format!(" -X {}", entry.method)
    };
    format!("curl{} '{}'", method_flag, entry.url)
}

pub async fn handle_browser_eval(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: EvalBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.browser.get_tabs().lock().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {}", e))
    })?.first().cloned().ok_or_else(|| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No active tab".to_string())
    })?;

    let result = tab.evaluate(&post.expression, false).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Eval failed: {}", e))
    })?;

    let result_str = match result.value {
        Some(val) => {
            if let Some(s) = val.as_str() {
                s.to_string()
            } else {
                serde_json::to_string(&val).unwrap_or_default()
            }
        }
        None => "undefined".to_string(),
    };

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "result": result_str
    })))
}

pub async fn handle_browser_inject_css(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: InjectCssBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.browser.get_tabs().lock().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {}", e))
    })?.first().cloned().ok_or_else(|| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No active tab".to_string())
    })?;

    let style_id = post.id.unwrap_or_else(|| format!("refact-css-{}", uuid::Uuid::new_v4()));
    let escaped_css = post.css.replace('\\', "\\\\").replace('`', "\\`");
    let js = format!(
        r#"(function() {{
            var existing = document.getElementById('{}');
            if (existing) existing.remove();
            var style = document.createElement('style');
            style.id = '{}';
            style.textContent = `{}`;
            document.head.appendChild(style);
        }})()"#,
        style_id, style_id, escaped_css
    );

    tab.evaluate(&js, false).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("CSS injection failed: {}", e))
    })?;

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "style_id": style_id
    })))
}

pub async fn handle_browser_remove_css(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: RemoveCssBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.browser.get_tabs().lock().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {}", e))
    })?.first().cloned().ok_or_else(|| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No active tab".to_string())
    })?;

    let js = format!(
        r#"(function() {{ var el = document.getElementById('{}'); if (el) el.remove(); }})()"#,
        post.style_id
    );

    tab.evaluate(&js, false).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("CSS removal failed: {}", e))
    })?;

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "status": "removed"
    })))
}

pub async fn handle_browser_dom_snapshot(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: DomSnapshotBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.browser.get_tabs().lock().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {}", e))
    })?.first().cloned().ok_or_else(|| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No active tab".to_string())
    })?;

    let escaped_selector = post.selector.replace('\\', "\\\\").replace('\'', "\\'");
    let js = format!(
        "(function() {{ var el = document.querySelector('{}'); return el ? el.outerHTML : null; }})()",
        escaped_selector
    );

    let result = tab.evaluate(&js, false).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("DOM query failed: {}", e))
    })?;

    let html = match result.value {
        Some(val) => val.as_str().unwrap_or("").to_string(),
        None => String::new(),
    };

    let max_chars = post.max_chars.unwrap_or(50000);
    let truncated = html.len() > max_chars;
    let html_out = if truncated { html[..max_chars].to_string() } else { html };

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "html": html_out,
        "truncated": truncated
    })))
}

pub async fn handle_browser_accessibility(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.browser.get_tabs().lock().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {}", e))
    })?.first().cloned().ok_or_else(|| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No active tab".to_string())
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
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Accessibility query failed: {}", e))
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

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "tree": tree
    })))
}

pub async fn handle_browser_record_animation(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: RecordAnimationBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (_, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.chat_id))
    })?;

    let rt = runtime_arc.lock().await;

    let tab = rt.browser.get_tabs().lock().map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get tabs: {}", e))
    })?.first().cloned().ok_or_else(|| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, "No active tab".to_string())
    })?;

    let duration_ms = post.duration_ms.unwrap_or(2000);
    let fps = post.fps.unwrap_or(5);
    let frame_interval = std::time::Duration::from_millis(1000 / fps as u64);
    let num_frames = (duration_ms as f64 / frame_interval.as_millis() as f64).ceil() as u32;

    drop(rt);

    let mut frames = Vec::new();
    for i in 0..num_frames {
        let screenshot_result = tab.call_method(
            headless_chrome::protocol::cdp::Page::CaptureScreenshot {
                format: Some(headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Jpeg),
                clip: None,
                quality: Some(60),
                from_surface: Some(true),
                capture_beyond_viewport: Some(false),
                optimize_for_speed: Some(true),
            },
        ).map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Frame capture failed: {}", e))
        })?;

        let raw = base64::prelude::BASE64_STANDARD
            .decode(&screenshot_result.data)
            .map_err(|e| {
                ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Base64 decode failed: {}", e))
            })?;

        let resized = resize_screenshot(&raw, 800, "image/jpeg")?;
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

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "frames": frames
    })))
}

pub async fn handle_browser_handoff(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: HandoffBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    let (rid, runtime_arc) = find_runtime_by_chat_id(gcx.clone(), &post.from_chat_id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("No browser runtime for chat_id={}", post.from_chat_id))
    })?;

    let (profile_dir, tab_urls, window_bounds, mask_passwords, attach_screenshot) = {
        let mut rt = runtime_arc.lock().await;
        let profile_dir = rt.profile_dir.to_string_lossy().to_string();
        let tab_urls: Vec<String> = rt.browser.get_tabs().lock()
            .map(|tabs| tabs.iter().map(|t| t.get_url()).collect())
            .unwrap_or_default();
        let window_bounds = rt.window_bounds.clone();
        let mask_passwords = rt.mask_passwords;
        let attach_screenshot = false;

        rt.detach();
        rt.reattach(&post.to_chat_id);
        rt.touch();

        (profile_dir, tab_urls, window_bounds, mask_passwords, attach_screenshot)
    };

    Ok(json_response(StatusCode::OK, serde_json::json!({
        "runtime_id": rid,
        "status": "transferred",
        "from_chat_id": post.from_chat_id,
        "to_chat_id": post.to_chat_id,
        "profile_dir": profile_dir,
        "tab_urls": tab_urls,
        "window_bounds": window_bounds,
        "mask_passwords": mask_passwords,
        "attach_screenshot_on_send": attach_screenshot
    })))
}

pub async fn handle_browser_status(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post: ChatIdBody = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e))
    })?;

    match find_runtime_by_chat_id(gcx.clone(), &post.chat_id).await {
        Some((rid, runtime_arc)) => {
            let rt = runtime_arc.lock().await;
            let tab_urls: Vec<String> = rt.browser.get_tabs().lock()
                .map(|tabs| tabs.iter().map(|t| t.get_url()).collect())
                .unwrap_or_default();
            let title = rt.browser.get_tabs().lock()
                .ok()
                .and_then(|tabs| tabs.first().and_then(|t| t.get_title().ok()))
                .unwrap_or_default();
            let url = rt.browser.get_tabs().lock()
                .ok()
                .map(|tabs| tabs.first().map(|t| t.get_url()).unwrap_or_default())
                .unwrap_or_default();

            Ok(json_response(StatusCode::OK, serde_json::json!({
                "runtime_id": rid,
                "connected": rt.is_connected,
                "url": url,
                "title": title,
                "tab_urls": tab_urls,
                "idle_seconds": rt.last_activity.elapsed().as_secs(),
                "idle_timeout": rt.idle_timeout.as_secs()
            })))
        }
        None => {
            Ok(json_response(StatusCode::OK, serde_json::json!({
                "runtime_id": null,
                "connected": false
            })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handoff_body_deserialize() {
        let json = r#"{"from_chat_id":"chat-1","to_chat_id":"chat-2"}"#;
        let body: HandoffBody = serde_json::from_str(json).unwrap();
        assert_eq!(body.from_chat_id, "chat-1");
        assert_eq!(body.to_chat_id, "chat-2");
    }

    #[test]
    fn test_sanitize_curl_get() {
        let entry = crate::integrations::browser_types::NetworkEntry {
            timestamp: 1.0,
            method: "GET".to_string(),
            url: "https://example.com/api?key=123".to_string(),
            resource_type: "Fetch".to_string(),
            status: Some(200),
        };
        let curl = sanitize_curl(&entry);
        assert_eq!(curl, "curl 'https://example.com/api?key=123'");
    }

    #[test]
    fn test_sanitize_curl_post() {
        let entry = crate::integrations::browser_types::NetworkEntry {
            timestamp: 1.0,
            method: "POST".to_string(),
            url: "https://example.com/api".to_string(),
            resource_type: "XHR".to_string(),
            status: Some(201),
        };
        let curl = sanitize_curl(&entry);
        assert_eq!(curl, "curl -X POST 'https://example.com/api'");
    }

    #[test]
    fn test_sanitize_curl_empty_method() {
        let entry = crate::integrations::browser_types::NetworkEntry {
            timestamp: 1.0,
            method: String::new(),
            url: "https://example.com".to_string(),
            resource_type: "Document".to_string(),
            status: Some(200),
        };
        let curl = sanitize_curl(&entry);
        assert_eq!(curl, "curl 'https://example.com'");
    }

    #[test]
    fn test_resize_screenshot_small_image() {
        let img = image::RgbImage::new(100, 100);
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Jpeg).unwrap();
        let result = resize_screenshot(&buf, 800, "image/jpeg").unwrap();
        assert!(!result.is_empty());
    }
}
