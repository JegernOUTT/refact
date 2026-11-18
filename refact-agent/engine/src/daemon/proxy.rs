use std::io;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{self, CONNECTION, CONTENT_TYPE, COOKIE, HeaderName, HeaderValue};
use axum::http::{HeaderMap, Request, Response, StatusCode, Uri};
use futures::StreamExt;
use hyper::body::HttpBody;
use serde::Deserialize;
use serde_json::json;

use crate::daemon::projects::ProjectEntry;
use crate::daemon::state::DaemonState;
use crate::daemon::supervisor::{WorkerInfo, WorkerState};
use crate::http::routers::v1::v1_browser_project::{self, PROJECT_HEADER};

pub const PROXY_BODY_LIMIT: usize = 15 * 1024 * 1024;
pub(crate) const BROWSER_LAUNCH_HEADER: &str = "x-refact-browser-launch";
#[cfg(not(test))]
const PROXY_WORKER_READY_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const PROXY_WORKER_READY_TIMEOUT: Duration = Duration::from_secs(120);
#[cfg(not(test))]
const PROXY_STREAM_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const PROXY_STREAM_HEADER_TIMEOUT: Duration = Duration::from_secs(5);
const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

#[derive(Deserialize)]
pub struct ProxyV1Path {
    project_id: String,
}

pub async fn proxy_v1(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    AxumPath(path): AxumPath<ProxyV1Path>,
    request: Request<Body>,
) -> Response<Body> {
    let worker_path = worker_v1_path(&path.project_id, request.uri());
    // Classify the same normalized path that the HTTP client will send, before project lookup,
    // worker startup, or reading the request body.
    let Ok(target) = reqwest::Url::parse(&target_url(1, &worker_path, request.uri().query()))
    else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error":"invalid worker request URL"}),
        );
    };
    let browser_token = if v1_browser_project::is_worker_path(target.path()) {
        match authorize_project_browser(&state, &path.project_id, &request) {
            Ok(token) => Some(token),
            Err(status) => {
                let mut response = json_response(
                    status,
                    json!({
                        "error":"Project Browser requires an authenticated Refact daemon project session"
                    }),
                );
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                return response;
            }
        }
    } else {
        None
    };
    proxy_to_worker(state, path.project_id, worker_path, request, browser_token).await
}

#[cfg(target_os = "linux")]
pub(super) async fn proxy_isolated_project(
    state: Arc<DaemonState>,
    project_id: String,
    request: Request<Body>,
    launch: &super::browser_launch::BrowserLaunch,
) -> Response<Body> {
    let worker_path = worker_v1_path(&project_id, request.uri());
    let Ok(target) = reqwest::Url::parse(&target_url(1, &worker_path, request.uri().query()))
    else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error":"invalid worker request URL"}),
        );
    };
    if target.path() != "/v1" && !target.path().starts_with("/v1/") {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({"error":"invalid worker API path"}),
        );
    }
    let browser_token = if v1_browser_project::is_worker_path(target.path()) {
        if request.method() != axum::http::Method::POST
            || !launch.authorize_browser(request.headers(), &project_id)
        {
            return json_response(
                StatusCode::UNAUTHORIZED,
                json!({"error":"Open this project again with refact ui to grant Project Browser access"}),
            );
        }
        state.supervisor.project_browser_worker_token(&project_id)
    } else {
        None
    };
    proxy_to_worker(state, project_id, worker_path, request, browser_token).await
}

fn authorize_project_browser(
    state: &DaemonState,
    project_id: &str,
    request: &Request<Body>,
) -> Result<String, StatusCode> {
    use crate::daemon::auth::{self, DaemonAuthPolicy};

    if !state.config.auth.enabled
        || auth::requested_project_id(request.uri().path()) != Some(project_id)
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let policy = DaemonAuthPolicy {
        token: state.auth_token.clone(),
        username: state.config.auth.username.clone(),
        password: state.config.auth.password.clone(),
        ..DaemonAuthPolicy::default()
    };
    if !v1_browser_project::gui_origin_allowed(request.headers()) {
        return Err(StatusCode::FORBIDDEN);
    }
    let origin = request.headers()[header::ORIGIN]
        .to_str()
        .map_err(|_| StatusCode::FORBIDDEN)?;
    let authorized = if auth::origin_is_trusted_ide_webview(origin) {
        // IDE fetches explicitly carry a daemon bearer; never accept ambient HTTP auth here.
        state
            .auth_token
            .as_deref()
            .filter(|token| !token.trim().is_empty())
            .zip(auth::bearer_token_from_headers(request.headers()))
            .is_some_and(|(expected, provided)| auth::token_matches(provided, expected))
    } else {
        auth::remote_authorized(request, &policy)
    };
    if !authorized {
        return Err(StatusCode::UNAUTHORIZED);
    }
    state
        .supervisor
        .project_browser_worker_token(project_id)
        .ok_or(StatusCode::UNAUTHORIZED)
}

async fn proxy_to_worker(
    state: Arc<DaemonState>,
    project_id: String,
    worker_path: String,
    request: Request<Body>,
    browser_token: Option<String>,
) -> Response<Body> {
    if is_upgrade_request(&request) {
        return json_response(
            StatusCode::NOT_IMPLEMENTED,
            json!({"error": "websocket upgrades are not supported by the daemon proxy"}),
        );
    }

    if state.is_shutting_down() {
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "daemon is shutting down"}),
        );
    }

    let entry = match project_entry(&state, &project_id).await {
        Some(entry) => entry,
        None => {
            return json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "project not found", "project_id": project_id}),
            );
        }
    };

    state.update_proxy_activity(&project_id).await;
    let use_stream_client = is_sse_request(request.headers(), &worker_path);
    let guard = if use_stream_client {
        state.increment_live_proxy_stream(&project_id).await;
        Some(ProxyStreamGuard::new(state.clone(), project_id.clone()))
    } else {
        None
    };
    let worker = match ready_worker(&state, &entry).await {
        Ok(worker) => worker,
        Err(response) => return response,
    };

    let (parts, body) = request.into_parts();
    let body_limit = proxy_body_limit(&parts.method, &worker_path);
    let body = match limited_body_bytes(body, body_limit).await {
        Ok(body) => body,
        Err(response) => return response,
    };

    let client = if use_stream_client {
        &state.proxy_stream_client
    } else {
        &state.proxy_client
    };
    let request = match worker_request(
        client,
        &project_id,
        worker.http_port,
        &worker_path,
        &parts,
        body,
        crate::daemon::hooks::worker_forward_token(&state),
        browser_token.as_deref(),
    ) {
        Ok(request) => request,
        Err(response) => return response,
    };
    let response = send_worker_request(request, use_stream_client).await;

    match response {
        Ok(response) => worker_response(state, entry, worker.pid, response, guard).await,
        Err(ProxySendError::StreamHeaderTimeout) => json_response(
            StatusCode::GATEWAY_TIMEOUT,
            json!({"error": "worker stream response headers timed out"}),
        ),
        Err(ProxySendError::Request(error)) if error.is_timeout() => json_response(
            StatusCode::GATEWAY_TIMEOUT,
            json!({"error": "worker request timed out"}),
        ),
        Err(ProxySendError::Request(error)) => {
            worker_unreachable(
                state,
                entry,
                worker.pid,
                crate::daemon::auth::redact_daemon_query_token(&error.to_string()),
            )
            .await
        }
    }
}

fn worker_request(
    client: &reqwest::Client,
    project_id: &str,
    port: u16,
    worker_path: &str,
    parts: &axum::http::request::Parts,
    body: hyper::body::Bytes,
    token: Option<&str>,
    browser_token: Option<&str>,
) -> Result<reqwest::RequestBuilder, Response<Body>> {
    let method =
        reqwest::Method::from_bytes(parts.method.as_str().as_bytes()).map_err(|error| {
            json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": format!("invalid request method: {error}")}),
            )
        })?;
    let url =
        reqwest::Url::parse(&target_url(port, worker_path, parts.uri.query())).map_err(|_| {
            json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "invalid worker request URL"}),
            )
        })?;
    let browser = v1_browser_project::is_worker_path(url.path());
    let token = if browser {
        Some(browser_token.ok_or_else(|| {
            json_response(
                StatusCode::UNAUTHORIZED,
                json!({"error":"Project Browser proxy authorization required"}),
            )
        })?)
    } else {
        token.filter(|_| requires_worker_authorization(&method, url.path()))
    };
    let mut headers = request_headers(&parts.headers, project_id);
    if browser {
        headers.remove(reqwest::header::ORIGIN);
        headers.remove(reqwest::header::HOST);
        headers.remove(reqwest::header::COOKIE);
    }
    let request = client
        .request(method, url)
        .headers(headers)
        .body(body.to_vec());
    Ok(match token {
        Some(token) => request.bearer_auth(token),
        None => request,
    })
}

fn requires_worker_authorization(method: &reqwest::Method, path: &str) -> bool {
    if [
        "/v1/apps",
        "/v1/scheduler/automation",
        "/v1/scheduler/credentials",
    ]
    .iter()
    .any(|prefix| {
        path == *prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|tail| tail.starts_with('/'))
    }) {
        return true;
    }
    match method.as_str() {
        "GET" | "HEAD" => {
            matches!(
                path,
                "/v1/work"
                    | "/v1/work/subscribe"
                    | "/v1/work/wait-graph"
                    | "/v1/work/causal"
                    | "/v1/work/causal/subscribe"
                    | "/v1/attention"
                    | "/v1/metrics"
                    | "/v1/hooks"
            ) || path
                .strip_prefix("/v1/work/")
                .is_some_and(|id| !id.is_empty() && !id.contains('/') && id != "provenance")
        }
        "DELETE" => path == "/v1/work/provenance",
        _ => false,
    }
}

#[derive(Debug)]
enum ProxySendError {
    StreamHeaderTimeout,
    Request(reqwest::Error),
}

async fn send_worker_request(
    request: reqwest::RequestBuilder,
    stream: bool,
) -> Result<reqwest::Response, ProxySendError> {
    if stream {
        match tokio::time::timeout(PROXY_STREAM_HEADER_TIMEOUT, request.send()).await {
            Ok(result) => result.map_err(ProxySendError::Request),
            Err(_) => Err(ProxySendError::StreamHeaderTimeout),
        }
    } else {
        request.send().await.map_err(ProxySendError::Request)
    }
}

async fn ready_worker(
    state: &Arc<DaemonState>,
    entry: &ProjectEntry,
) -> Result<WorkerInfo, Response<Body>> {
    ready_worker_with_timeout(state, entry, PROXY_WORKER_READY_TIMEOUT).await
}

async fn ready_worker_with_timeout(
    state: &Arc<DaemonState>,
    entry: &ProjectEntry,
    timeout: Duration,
) -> Result<WorkerInfo, Response<Body>> {
    let supervisor = state.supervisor.clone();
    let entry_for_task = entry.clone();
    let mut readiness =
        tokio::spawn(async move { supervisor.ensure_ready_worker(&entry_for_task).await });

    match tokio::time::timeout(timeout, &mut readiness).await {
        Ok(Ok(Ok(worker))) => Ok(worker),
        Ok(Ok(Err(error))) => Err(json_response(
            StatusCode::BAD_GATEWAY,
            json!({"error": error, "project_id": entry.id}),
        )),
        Ok(Err(error)) => Err(json_response(
            StatusCode::BAD_GATEWAY,
            json!({"error": format!("worker readiness task failed: {error}"), "project_id": entry.id}),
        )),
        Err(_) => {
            let info = state.supervisor.worker_info(&entry.id).await;
            if let Some(info) = info {
                if matches!(info.state, WorkerState::Ready) {
                    return Ok(info);
                }
                let error = proxy_worker_timeout_reason(&info);
                return Err(json_response(
                    StatusCode::BAD_GATEWAY,
                    json!({"error": error, "project_id": entry.id}),
                ));
            }
            Err(json_response(
                StatusCode::BAD_GATEWAY,
                json!({"error": "worker unavailable", "project_id": entry.id}),
            ))
        }
    }
}

fn proxy_worker_timeout_reason(info: &WorkerInfo) -> String {
    match &info.state {
        WorkerState::Failed {
            reason,
        } => reason.clone(),
        WorkerState::Crashed => info
            .last_error
            .clone()
            .unwrap_or_else(|| "worker crashed".to_string()),
        WorkerState::Stopped => "worker stopped".to_string(),
        WorkerState::Stopping => "worker stopping".to_string(),
        WorkerState::Starting => info
            .last_error
            .clone()
            .unwrap_or_else(|| "worker readiness timed out".to_string()),
        WorkerState::Ready => "worker readiness timed out".to_string(),
    }
}

async fn project_entry(state: &DaemonState, project_id: &str) -> Option<ProjectEntry> {
    state.projects.read().await.get(project_id).cloned()
}

fn proxy_body_limit(method: &axum::http::Method, worker_path: &str) -> Option<usize> {
    let workspace_chat_id = worker_path
        .strip_prefix("/v1/chats/")
        .and_then(|path| path.strip_suffix("/app-workspace"));
    let workspace = workspace_chat_id
        .is_some_and(|id| crate::chat::trajectories::validate_trajectory_id(id).is_ok());
    if method == axum::http::Method::POST && (worker_path == "/v1/apps/commands" || workspace) {
        None
    } else {
        Some(PROXY_BODY_LIMIT)
    }
}

async fn limited_body_bytes(
    mut body: Body,
    limit: Option<usize>,
) -> Result<hyper::body::Bytes, Response<Body>> {
    let mut out = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                return Err(json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": format!("failed to read request body: {error}")}),
                ));
            }
        };
        if limit.is_some_and(|limit| out.len().saturating_add(chunk.len()) > limit) {
            return Err(json_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                json!({"error": "request body too large"}),
            ));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(hyper::body::Bytes::from(out))
}

async fn worker_response(
    state: Arc<DaemonState>,
    entry: ProjectEntry,
    worker_pid: Option<u32>,
    response: reqwest::Response,
    guard: Option<ProxyStreamGuard>,
) -> Response<Body> {
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let headers = response_headers(response.headers());
    let stream_state = state.clone();
    let stream_entry = entry.clone();
    let stream_refreshes_activity = guard.is_some();
    let stream = async_stream::stream! {
        let _guard = guard;
        let mut upstream = response.bytes_stream();
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(chunk) => {
                    if stream_refreshes_activity {
                        stream_state.refresh_proxy_activity(&stream_entry.id).await;
                    }
                    yield Ok::<_, io::Error>(chunk);
                }
                Err(error) => {
                    let message = crate::daemon::auth::redact_daemon_query_token(&error.to_string());
                    if stream_refreshes_activity {
                        stream_state.refresh_proxy_activity(&stream_entry.id).await;
                    }
                    let _ = stream_state.events.emit(
                        "proxy_worker_unreachable",
                        Some(stream_entry.id.clone()),
                        json!({"error": message}),
                    ).await;
                    stream_state
                        .supervisor
                        .notify_proxy_unreachable(
                            stream_entry.clone(),
                            stream_state.is_shutting_down(),
                            worker_pid,
                        )
                        .await;
                    yield Err(io::Error::new(io::ErrorKind::Other, message));
                    break;
                }
            }
        }
        if stream_refreshes_activity {
            stream_state.refresh_proxy_activity(&stream_entry.id).await;
        }
    };
    let mut proxied = Response::new(Body::wrap_stream(stream));
    *proxied.status_mut() = status;
    *proxied.headers_mut() = headers;
    proxied
}

async fn worker_unreachable(
    state: Arc<DaemonState>,
    entry: ProjectEntry,
    worker_pid: Option<u32>,
    error: String,
) -> Response<Body> {
    let _ = state
        .events
        .emit(
            "proxy_worker_unreachable",
            Some(entry.id.clone()),
            json!({"error": error}),
        )
        .await;
    state
        .supervisor
        .notify_proxy_unreachable(entry.clone(), state.is_shutting_down(), worker_pid)
        .await;
    json_response(
        StatusCode::BAD_GATEWAY,
        json!({"error": "worker unavailable", "project_id": entry.id}),
    )
}

fn worker_v1_path(project_id: &str, uri: &Uri) -> String {
    let prefix = format!("/p/{project_id}/v1");
    let suffix = uri.path().strip_prefix(&prefix).unwrap_or_default();
    if suffix.is_empty() {
        "/v1".to_string()
    } else {
        format!("/v1{suffix}")
    }
}

fn target_url(port: u16, path: &str, query: Option<&str>) -> String {
    match crate::daemon::auth::query_without_daemon_token(query) {
        Some(query) => format!("http://127.0.0.1:{port}{path}?{query}"),
        None => format!("http://127.0.0.1:{port}{path}"),
    }
}

fn is_upgrade_request<B>(request: &Request<B>) -> bool {
    request.headers().contains_key(axum::http::header::UPGRADE)
        || request
            .headers()
            .get(CONNECTION)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
            })
            .unwrap_or(false)
}

fn is_sse_request(headers: &HeaderMap, path: &str) -> bool {
    headers
        .get("accept")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.contains("text/event-stream"))
        .unwrap_or(false)
        || path_is_streaming(path)
}

fn path_is_streaming(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    path.ends_with("/subscribe")
        || path.ends_with("/chats/subscribe-many")
        || path.contains("/voice/stream/")
}

fn request_headers(headers: &HeaderMap, project_id: &str) -> reqwest::header::HeaderMap {
    let connection_tokens = connection_tokens(headers);
    let mut out = reqwest::header::HeaderMap::new();
    for (name, value) in headers.iter() {
        if should_strip_header(name.as_str(), &connection_tokens) {
            continue;
        }
        if name == axum::http::header::AUTHORIZATION || name.as_str() == BROWSER_LAUNCH_HEADER {
            continue;
        }
        if name == COOKIE {
            if let Some(value) = scrub_cookie_header(value) {
                out.append(reqwest::header::COOKIE, value);
            }
            continue;
        }
        let Ok(name) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) else {
            continue;
        };
        let Ok(value) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) else {
            continue;
        };
        out.append(name, value);
    }
    if let Ok(value) = reqwest::header::HeaderValue::from_str(project_id) {
        out.insert(PROJECT_HEADER, value);
    }
    out
}

fn scrub_cookie_header(value: &HeaderValue) -> Option<reqwest::header::HeaderValue> {
    let value = value.to_str().ok()?;
    let cookies = value
        .split(';')
        .filter_map(|cookie| {
            let cookie = cookie.trim();
            if cookie.is_empty() {
                return None;
            }
            if cookie
                .split_once('=')
                .map(|(name, _)| name.trim() == crate::daemon::auth::DAEMON_AUTH_COOKIE)
                .unwrap_or(false)
            {
                return None;
            }
            Some(cookie.to_string())
        })
        .collect::<Vec<_>>();
    if cookies.is_empty() {
        return None;
    }
    reqwest::header::HeaderValue::from_str(&cookies.join("; ")).ok()
}

fn response_headers(headers: &reqwest::header::HeaderMap) -> HeaderMap {
    let connection_tokens = reqwest_connection_tokens(headers);
    let mut out = HeaderMap::new();
    for (name, value) in headers.iter() {
        if should_strip_header(name.as_str(), &connection_tokens) {
            continue;
        }
        let Ok(name) = HeaderName::from_bytes(name.as_str().as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_bytes(value.as_bytes()) else {
            continue;
        };
        out.append(name, value);
    }
    out
}

fn connection_tokens(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn reqwest_connection_tokens(headers: &reqwest::header::HeaderMap) -> Vec<String> {
    headers
        .get_all(reqwest::header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn should_strip_header(name: &str, connection_tokens: &[String]) -> bool {
    let lower = name.to_ascii_lowercase();
    HOP_BY_HOP_HEADERS.contains(&lower.as_str())
        || connection_tokens.iter().any(|token| token == &lower)
}

fn json_response(status: StatusCode, value: serde_json::Value) -> Response<Body> {
    let body = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

struct ProxyStreamGuard {
    state: Arc<DaemonState>,
    project_id: String,
}

impl ProxyStreamGuard {
    fn new(state: Arc<DaemonState>, project_id: String) -> Self {
        Self {
            state,
            project_id,
        }
    }
}

impl Drop for ProxyStreamGuard {
    fn drop(&mut self) {
        self.state.decrement_live_proxy_stream(&self.project_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::config::DaemonConfig;
    use crate::daemon::events::EventBus;
    use crate::daemon::projects::ProjectEntry;
    use axum::http::header::HeaderValue;
    use axum::http::Method;
    use futures::StreamExt;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Instant;
    use tempfile::{tempdir, TempDir};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tower::ServiceExt;

    struct EnvGuard {
        keys: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn fake_worker() -> Option<Self> {
            let python = std::env::var("PYTHON3").unwrap_or_else(|_| "python3".to_string());
            if std::process::Command::new(&python)
                .arg("--version")
                .output()
                .is_err()
            {
                return None;
            }
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fake_worker.py");
            let keys = vec![
                (
                    "REFACT_DAEMON_WORKER_CMD",
                    std::env::var("REFACT_DAEMON_WORKER_CMD").ok(),
                ),
                (
                    "REFACT_DAEMON_SUPERVISOR_BACKOFF_MS",
                    std::env::var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS").ok(),
                ),
                ("FAKE_WORKER_CRASH", std::env::var("FAKE_WORKER_CRASH").ok()),
                (
                    "FAKE_WORKER_DELAY_READY",
                    std::env::var("FAKE_WORKER_DELAY_READY").ok(),
                ),
                (
                    "FAKE_WORKER_PORT_BUSY_EXIT",
                    std::env::var("FAKE_WORKER_PORT_BUSY_EXIT").ok(),
                ),
                (
                    "FAKE_WORKER_SKIP_LSP",
                    std::env::var("FAKE_WORKER_SKIP_LSP").ok(),
                ),
            ];
            std::env::set_var(
                "REFACT_DAEMON_WORKER_CMD",
                shell_words::join([python.as_str(), script.to_string_lossy().as_ref()]),
            );
            std::env::set_var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS", "1");
            std::env::remove_var("FAKE_WORKER_CRASH");
            std::env::remove_var("FAKE_WORKER_DELAY_READY");
            std::env::remove_var("FAKE_WORKER_PORT_BUSY_EXIT");
            std::env::remove_var("FAKE_WORKER_SKIP_LSP");
            Some(Self {
                keys,
            })
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.keys.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    async fn proxy_harness() -> (TempDir, Arc<DaemonState>, ProjectEntry) {
        let dir = tempdir().unwrap();
        let project_root = dir.path().join("proxy-project");
        std::fs::create_dir_all(&project_root).unwrap();
        let state = DaemonState::new_with_daemon_dir(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
            dir.path().join("daemon"),
            8488,
        );
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(project_root).await.unwrap()
        };
        state.sync_project_liveness(&entry).await;
        (dir, state, entry)
    }

    async fn wait_for_starting(state: &DaemonState, project_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if state
                .supervisor
                .worker_info(project_id)
                .await
                .map(|info| matches!(info.state, WorkerState::Starting))
                .unwrap_or(false)
            {
                return;
            }
            assert!(Instant::now() < deadline, "worker did not enter startup");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    #[test]
    fn target_url_strips_daemon_token_query() {
        let url = target_url(
            1234,
            "/v1/chats/subscribe",
            Some("chat_id=abc&daemon_token=secret-token&tail=1"),
        );

        assert_eq!(
            url,
            "http://127.0.0.1:1234/v1/chats/subscribe?chat_id=abc&tail=1"
        );
        assert!(!url.contains("secret-token"));
        assert!(!url.contains(crate::daemon::auth::DAEMON_AUTH_QUERY));
    }

    #[test]
    fn request_headers_strip_hop_by_hop_and_add_project_id() {
        let mut headers = HeaderMap::new();
        headers.insert(CONNECTION, HeaderValue::from_static("x-remove, keep-alive"));
        headers.insert("x-remove", HeaderValue::from_static("bad"));
        headers.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        headers.insert("x-keep", HeaderValue::from_static("ok"));

        let out = request_headers(&headers, "project");
        assert!(out.get("x-remove").is_none());
        assert!(out.get("keep-alive").is_none());
        assert_eq!(out.get("x-keep").unwrap(), "ok");
        assert_eq!(out.get(PROJECT_HEADER).unwrap(), "project");
    }

    #[test]
    fn request_headers_strips_authorization_and_daemon_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer malicious-browser-token"),
        );
        headers.insert(
            COOKIE,
            HeaderValue::from_static("refact_daemon_auth=secret; theme=dark"),
        );

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for token in [Some("worker-token"), None] {
            for (method, path, protected) in [
                (Method::GET, "/v1/work", true),
                (Method::GET, "/v1/work/run-1", true),
                (Method::GET, "/v1/work/subscribe", true),
                (Method::GET, "/v1/work/wait-graph", true),
                (Method::GET, "/v1/work/causal", true),
                (Method::GET, "/v1/work/causal/subscribe", true),
                (Method::DELETE, "/v1/work/provenance", true),
                (Method::GET, "/v1/attention", true),
                (Method::GET, "/v1/metrics", true),
                (Method::GET, "/v1/hooks", true),
                (Method::HEAD, "/v1/hooks", true),
                (Method::GET, "/v1/scheduler/automation", true),
                (Method::GET, "/v1/scheduler/automation/job-1", true),
                (Method::POST, "/v1/scheduler/automation", true),
                (Method::PUT, "/v1/scheduler/automation/job-1", true),
                (Method::DELETE, "/v1/scheduler/automation/job-1", true),
                (Method::POST, "/v1/scheduler/automation/job-1/control", true),
                (Method::GET, "/v1/scheduler/credentials", true),
                (Method::POST, "/v1/scheduler/credentials", true),
                (Method::DELETE, "/v1/scheduler/credentials/key-1", true),
                (Method::GET, "/v1/scheduler/automation-extra", false),
                (Method::POST, "/v1/scheduler/credentials-extra", false),
                (Method::GET, "/v1/worktrees", false),
                (Method::GET, "/v1/hooks-evil", false),
                (Method::POST, "/v1/hooks/fire", false),
                (Method::GET, "/v1/attention/action", false),
                (Method::POST, "/v1/attention", false),
                (Method::GET, "/v1/metrics-extra", false),
                (Method::GET, "/v1/work/run-1/extra", false),
                (Method::GET, "/v1/work/provenance", false),
                (Method::POST, "/v1/work", false),
                (Method::GET, "/v1/work/.", false),
                (Method::GET, "/v1/work/%2e%2e", false),
                (Method::GET, "/v1/chat/completions", false),
            ] {
                let mut incoming = Request::builder()
                    .method(method.clone())
                    .uri(format!(
                        "http://browser.invalid/p/project{path}?daemon_token=browser-query&limit=3"
                    ))
                    .body(Body::empty())
                    .unwrap();
                *incoming.headers_mut() = headers.clone();
                let (parts, _) = incoming.into_parts();
                let worker_path = worker_v1_path("project", &parts.uri);
                let outgoing = worker_request(
                    &client,
                    "project",
                    1234,
                    &worker_path,
                    &parts,
                    hyper::body::Bytes::new(),
                    token,
                    None,
                )
                .unwrap()
                .build()
                .unwrap();

                assert_eq!(outgoing.method().as_str(), method.as_str());
                let normalized_path = match path {
                    "/v1/work/." => "/v1/work/",
                    "/v1/work/%2e%2e" => "/v1/",
                    _ => path,
                };
                assert_eq!(
                    outgoing.url().as_str(),
                    format!("http://127.0.0.1:1234{normalized_path}?limit=3")
                );
                let authorization = outgoing
                    .headers()
                    .get(reqwest::header::AUTHORIZATION)
                    .map(|value| value.to_str().unwrap());
                assert_eq!(
                    authorization,
                    token.filter(|_| protected).map(|_| "Bearer worker-token"),
                    "{method} {path}"
                );
                assert_eq!(outgoing.headers().get(PROJECT_HEADER).unwrap(), "project");
                assert_eq!(
                    outgoing.headers().get(reqwest::header::COOKIE).unwrap(),
                    "theme=dark"
                );
            }
        }
    }

    fn browser_test_state(dir: &std::path::Path, enabled: bool) -> Arc<DaemonState> {
        let mut config = DaemonConfig::default();
        config.auth.enabled = enabled;
        config.auth.username = Some("operator".into());
        config.auth.password = Some("fixture-password".into());
        DaemonState::new_with_daemon_dir(
            config,
            EventBus::new(dir.join("events.jsonl")),
            Some("daemon-fixture".into()),
            dir.to_path_buf(),
            8488,
        )
    }

    fn browser_request(
        origin: Option<&str>,
        cookie: Option<&str>,
        bearer: Option<&str>,
    ) -> Request<Body> {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri("/p/proj/v1/browser/project/frame")
            .header(header::HOST, "127.0.0.1:8488")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(origin) = origin {
            req = req.header(header::ORIGIN, origin);
        }
        if let Some(cookie) = cookie {
            req = req.header(header::COOKIE, cookie);
        }
        if let Some(bearer) = bearer {
            req = req.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
        }
        let mut req = req.body(Body::empty()).unwrap();
        req.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
        ));
        req
    }

    #[tokio::test]
    async fn project_browser_proxy_requires_explicit_operator_authority_and_gui_origin() {
        let tmp = tempdir().unwrap();
        let state = browser_test_state(tmp.path(), true);
        let local = Some("http://127.0.0.1:8488");
        let ide = Some("vscode-webview://refact-view");
        let valid = Some("refact_daemon_auth=project:proj:daemon-fixture");
        for (origin, cookie, bearer, status) in [
            (local, None, None, StatusCode::UNAUTHORIZED),
            (
                local,
                Some("refact_daemon_auth=daemon-fixture"),
                None,
                StatusCode::UNAUTHORIZED,
            ),
            (
                local,
                Some("refact_daemon_auth=project:other:daemon-fixture"),
                None,
                StatusCode::UNAUTHORIZED,
            ),
            (local, None, Some("hook-fixture"), StatusCode::UNAUTHORIZED),
            (None, valid, None, StatusCode::FORBIDDEN),
            (Some("null"), valid, None, StatusCode::FORBIDDEN),
            (
                Some("https://example.test"),
                valid,
                None,
                StatusCode::FORBIDDEN,
            ),
            (
                Some("http://127.0.0.1:7777"),
                valid,
                None,
                StatusCode::FORBIDDEN,
            ),
            (ide, valid, None, StatusCode::UNAUTHORIZED),
        ] {
            assert_eq!(
                authorize_project_browser(&state, "proj", &browser_request(origin, cookie, bearer)),
                Err(status),
                "{origin:?} {cookie:?} {bearer:?}"
            );
        }
        for req in [
            browser_request(local, valid, None),
            browser_request(ide, None, Some("daemon-fixture")),
        ] {
            assert_eq!(
                authorize_project_browser(&state, "proj", &req).ok(),
                state.supervisor.project_browser_worker_token("proj")
            );
            assert!(authorize_project_browser(&state, "other", &req).is_err());
        }
        let mut duplicate = browser_request(local, valid, Some("daemon-fixture"));
        duplicate.headers_mut().append(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer daemon-fixture"),
        );
        assert_eq!(
            authorize_project_browser(&state, "proj", &duplicate),
            Err(StatusCode::UNAUTHORIZED)
        );
        let mut basic = browser_request(local, None, None);
        use base64::Engine;
        let basic_value = base64::prelude::BASE64_STANDARD.encode("operator:fixture-password");
        basic.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Basic {basic_value}").parse().unwrap(),
        );
        assert!(authorize_project_browser(&state, "proj", &basic).is_ok());
        basic.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("vscode-webview://refact-view"),
        );
        assert_eq!(
            authorize_project_browser(&state, "proj", &basic),
            Err(StatusCode::UNAUTHORIZED)
        );
        let auth_disabled = browser_test_state(&tmp.path().join("disabled"), false);
        assert_eq!(
            authorize_project_browser(
                &auth_disabled,
                "proj",
                &browser_request(local, valid, Some("daemon-fixture"))
            ),
            Err(StatusCode::UNAUTHORIZED)
        );
    }

    #[tokio::test]
    async fn project_browser_proxy_denies_before_worker_lookup_and_body_read() {
        let tmp = tempdir().unwrap();
        let state = browser_test_state(tmp.path(), true);
        let mut req = browser_request(Some("http://127.0.0.1:8488"), None, None);
        // A normalized worker path is protected even if its spelling changes in transit.
        *req.uri_mut() = "/p/proj/v1/alternate/%2e%2e/browser/project/input"
            .parse()
            .unwrap();
        *req.body_mut() = Body::wrap_stream(futures::stream::poll_fn(
            |_| -> std::task::Poll<Option<Result<hyper::body::Bytes, io::Error>>> {
                panic!("unauthenticated request body read")
            },
        ));
        let response = proxy_v1(
            State((state, 8488)),
            AxumPath(ProxyV1Path {
                project_id: "proj".into(),
            }),
            req,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[test]
    fn project_browser_worker_request_replaces_browser_credentials_with_scoped_proof() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let req = browser_request(
            Some("http://127.0.0.1:8488"),
            Some("refact_daemon_auth=project:proj:daemon-fixture"),
            Some("daemon-fixture"),
        );
        let (mut parts, _) = req.into_parts();
        parts.headers.insert(
            PROJECT_HEADER,
            HeaderValue::from_static("untrusted-client-project"),
        );
        parts.headers.insert(
            BROWSER_LAUNCH_HEADER,
            HeaderValue::from_static("document-proof"),
        );
        let build = |path, proof| {
            worker_request(
                &client,
                "proj",
                1234,
                path,
                &parts,
                hyper::body::Bytes::new(),
                Some("daemon-fixture"),
                proof,
            )
        };
        assert_eq!(
            build("/v1/browser/project/frame", None)
                .unwrap_err()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let outgoing = build("/v1/browser/project/frame", Some("scoped-proof"))
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            outgoing.headers()[reqwest::header::AUTHORIZATION],
            "Bearer scoped-proof"
        );
        assert_eq!(outgoing.headers()[PROJECT_HEADER], "proj");
        for name in ["cookie", "origin", "host", BROWSER_LAUNCH_HEADER] {
            assert!(!outgoing.headers().contains_key(name));
        }
        let unrelated = build("/v1/browser/status", Some("scoped-proof"))
            .unwrap()
            .build()
            .unwrap();
        assert!(!unrelated.headers().contains_key(BROWSER_LAUNCH_HEADER));
        assert!(!unrelated
            .headers()
            .contains_key(reqwest::header::AUTHORIZATION));
    }

    #[test]
    fn response_headers_strip_connection_named_headers() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::CONNECTION, "x-secret".parse().unwrap());
        headers.insert("x-secret", "bad".parse().unwrap());
        headers.insert("x-visible", "ok".parse().unwrap());

        let out = response_headers(&headers);
        assert!(out.get("x-secret").is_none());
        assert_eq!(out.get("x-visible").unwrap(), "ok");
    }

    #[test]
    fn sse_request_false_for_regular_api_path() {
        let headers = HeaderMap::new();

        assert!(!is_sse_request(&headers, "/v1/chat/completions"));
    }

    #[test]
    fn sse_request_detected_for_subscribe_endpoints_without_accept_header() {
        let headers = HeaderMap::new();

        assert!(is_sse_request(&headers, "/v1/sidebar/subscribe"));
        assert!(is_sse_request(&headers, "/v1/tasks/subscribe"));
        assert!(is_sse_request(
            &headers,
            "/v1/sidebar/subscribe?after_seq=3"
        ));
        assert!(is_sse_request(
            &headers,
            "/v1/voice/stream/abc123/subscribe"
        ));
        assert!(is_sse_request(
            &headers,
            "/v1/chats/subscribe-many?chat_ids=%5B%22chat%22%5D"
        ));
    }

    #[tokio::test]
    async fn limited_body_bytes_rejects_proxy_bodies_over_limit() {
        let body = Body::from(vec![b'x'; PROXY_BODY_LIMIT + 1]);

        let response = limited_body_bytes(body, Some(PROXY_BODY_LIMIT))
            .await
            .unwrap_err();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial(daemon)]
    async fn proxy_waits_for_existing_starting_worker_and_succeeds() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        std::env::set_var("FAKE_WORKER_DELAY_READY", "0.5");
        let (_dir, state, entry) = proxy_harness().await;
        let start = tokio::spawn({
            let state = state.clone();
            let entry = entry.clone();
            async move { state.supervisor.ensure_worker(&entry).await }
        });
        wait_for_starting(&state, &entry.id).await;

        let started = Instant::now();
        let response = crate::daemon::server::make_router(state.clone(), 8488)
            .oneshot(
                Request::builder()
                    .uri(format!("/p/{}/v1/echo", entry.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(started.elapsed() >= Duration::from_millis(200));
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["path"], "/v1/echo");
        assert_eq!(start.await.unwrap().unwrap().state, WorkerState::Ready);
        state.supervisor.stop_all().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial(daemon)]
    async fn proxy_times_out_waiting_for_starting_worker() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        std::env::set_var("FAKE_WORKER_DELAY_READY", "30");
        let (_dir, state, entry) = proxy_harness().await;
        let start = tokio::spawn({
            let state = state.clone();
            let entry = entry.clone();
            async move { state.supervisor.ensure_worker(&entry).await }
        });
        wait_for_starting(&state, &entry.id).await;

        let started = Instant::now();
        let error = ready_worker_with_timeout(&state, &entry, Duration::from_secs(2))
            .await
            .unwrap_err();
        assert!(started.elapsed() >= Duration::from_secs(2));
        assert_eq!(error.status(), StatusCode::BAD_GATEWAY);
        let body = hyper::body::to_bytes(error.into_body()).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"], "worker readiness timed out");
        state.supervisor.stop_all().await;
        let _ = tokio::time::timeout(Duration::from_secs(10), start)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial(daemon)]
    async fn proxy_surfaces_worker_spawn_failure() {
        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        std::env::set_var(
            "REFACT_DAEMON_WORKER_CMD",
            "/definitely/missing/refact-worker",
        );
        let (_dir, state, entry) = proxy_harness().await;

        let response = crate::daemon::server::make_router(state.clone(), 8488)
            .oneshot(
                Request::builder()
                    .uri(format!("/p/{}/v1/echo", entry.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("failed to spawn worker"));
    }

    #[tokio::test]
    async fn stream_request_times_out_waiting_for_headers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            let _ = accepted_tx.send(());
            std::future::pending::<()>().await;
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let request = client.get(format!("http://127.0.0.1:{port}/v1/chats/subscribe"));
        let mut task = tokio::spawn(send_worker_request(request, true));

        let accepted = tokio::time::timeout(Duration::from_secs(10), accepted_rx).await;
        let outcome = tokio::time::timeout(
            PROXY_STREAM_HEADER_TIMEOUT + Duration::from_secs(5),
            &mut task,
        )
        .await;
        server.abort();
        let _ = server.await;
        if outcome.is_err() {
            task.abort();
            let _ = task.await;
        }
        assert!(
            matches!(accepted, Ok(Ok(()))),
            "server did not accept request"
        );
        assert!(matches!(
            outcome
                .expect("header request did not settle")
                .expect("header request task panicked"),
            Err(ProxySendError::StreamHeaderTimeout)
        ));
    }

    #[tokio::test]
    async fn stream_request_keeps_body_unbounded_after_headers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
                assert!(request.len() <= 16 * 1024);
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(25)).await;
            socket.write_all(b"data: late\n\n").await.unwrap();
            std::future::pending::<()>().await;
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let request = client.get(format!("http://127.0.0.1:{port}/v1/chats/subscribe"));
        let response = send_worker_request(request, true).await.unwrap();
        let mut stream = response.bytes_stream();
        let next = tokio::spawn(async move { stream.next().await });

        let chunk = next.await.unwrap().unwrap().unwrap();
        assert_eq!(&chunk[..], b"data: late\n\n");
        server.abort();
    }

    #[test]
    fn proxy_stream_guard_decrements_live_streams_inline_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new(
            crate::daemon::config::DaemonConfig::default(),
            crate::daemon::events::EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        {
            let mut activity = state.proxy_activity.write();
            activity.insert(
                "project".to_string(),
                crate::daemon::state::ProxyActivity {
                    last_proxy_activity_ms: 0,
                    live_proxy_streams: 1,
                },
            );
        }

        drop(ProxyStreamGuard::new(state.clone(), "project".to_string()));

        assert_eq!(
            state
                .proxy_activity
                .read()
                .get("project")
                .unwrap()
                .live_proxy_streams,
            0
        );
    }
}
