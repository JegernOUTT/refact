use std::io;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::header::{CONNECTION, CONTENT_TYPE, COOKIE, HeaderName, HeaderValue};
use axum::http::{HeaderMap, Request, Response, StatusCode, Uri};
use futures::StreamExt;
use hyper::body::HttpBody;
use serde::Deserialize;
use serde_json::json;

use crate::daemon::projects::ProjectEntry;
use crate::daemon::state::DaemonState;
use crate::daemon::supervisor::{WorkerInfo, WorkerState};

pub const PROXY_BODY_LIMIT: usize = 15 * 1024 * 1024;
#[cfg(not(test))]
const PROXY_WORKER_READY_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const PROXY_WORKER_READY_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(not(test))]
const PROXY_STREAM_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const PROXY_STREAM_HEADER_TIMEOUT: Duration = Duration::from_secs(5);
const PROJECT_HEADER: &str = "x-refact-project-id";
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
    proxy_to_worker(state, path.project_id, worker_path, request).await
}

async fn proxy_to_worker(
    state: Arc<DaemonState>,
    project_id: String,
    worker_path: String,
    request: Request<Body>,
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
    let body = match limited_body_bytes(body).await {
        Ok(body) => body,
        Err(response) => return response,
    };

    let url = target_url(worker.http_port, &worker_path, parts.uri.query());
    let method = match reqwest::Method::from_bytes(parts.method.as_str().as_bytes()) {
        Ok(method) => method,
        Err(error) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": format!("invalid request method: {error}")}),
            );
        }
    };
    let headers = request_headers(&parts.headers, &project_id);
    let client = if use_stream_client {
        &state.proxy_stream_client
    } else {
        &state.proxy_client
    };
    let request = client
        .request(method, url)
        .headers(headers)
        .body(body.to_vec());
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
    let supervisor = state.supervisor.clone();
    let entry_for_task = entry.clone();
    let mut readiness =
        tokio::spawn(async move { supervisor.ensure_ready_worker(&entry_for_task).await });

    match tokio::time::timeout(PROXY_WORKER_READY_TIMEOUT, &mut readiness).await {
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
        WorkerState::Failed { reason } => reason.clone(),
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

async fn limited_body_bytes(mut body: Body) -> Result<hyper::body::Bytes, Response<Body>> {
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
        if out.len().saturating_add(chunk.len()) > PROXY_BODY_LIMIT {
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
        || path.contains("/chats/subscribe")
}

fn request_headers(headers: &HeaderMap, project_id: &str) -> reqwest::header::HeaderMap {
    let connection_tokens = connection_tokens(headers);
    let mut out = reqwest::header::HeaderMap::new();
    for (name, value) in headers.iter() {
        if should_strip_header(name.as_str(), &connection_tokens) {
            continue;
        }
        if name == axum::http::header::AUTHORIZATION {
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
        Self { state, project_id }
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
    use tokio::io::AsyncWriteExt;
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
            Some(Self { keys })
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
    fn worker_v1_path_preserves_raw_suffix() {
        let uri: Uri = "/p/abc123/v1/chats/subscribe?chat_id=a%2Fb"
            .parse()
            .unwrap();
        assert_eq!(worker_v1_path("abc123", &uri), "/v1/chats/subscribe");
    }

    #[test]
    fn target_url_preserves_query() {
        assert_eq!(
            target_url(1234, "/v1/echo", Some("a=1&b=two")),
            "http://127.0.0.1:1234/v1/echo?a=1&b=two"
        );
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
            HeaderValue::from_static("Bearer daemon-token"),
        );
        headers.insert(
            COOKIE,
            HeaderValue::from_static("refact_daemon_auth=secret; theme=dark"),
        );

        let out = request_headers(&headers, "project");

        assert!(out.get(reqwest::header::AUTHORIZATION).is_none());
        assert_eq!(out.get(reqwest::header::COOKIE).unwrap(), "theme=dark");
    }

    #[test]
    fn request_headers_preserves_non_daemon_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("theme=dark; sid=abc"));

        let out = request_headers(&headers, "project");

        assert_eq!(
            out.get(reqwest::header::COOKIE).unwrap(),
            "theme=dark; sid=abc"
        );
    }

    #[test]
    fn request_headers_omits_cookie_header_when_only_daemon_cookie_remains() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("refact_daemon_auth=secret"),
        );

        let out = request_headers(&headers, "project");

        assert!(out.get(reqwest::header::COOKIE).is_none());
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
    fn upgrade_request_is_detected_from_connection_header() {
        let request = Request::builder()
            .method(Method::GET)
            .uri("/p/project/v1/ws")
            .header(CONNECTION, "keep-alive, Upgrade")
            .body(Body::empty())
            .unwrap();
        assert!(is_upgrade_request(&request));
    }

    #[test]
    fn sse_request_detected_from_accept_header() {
        let mut headers = HeaderMap::new();
        headers.insert("accept", HeaderValue::from_static("text/event-stream"));

        assert!(is_sse_request(&headers, "/v1/chats"));
    }

    #[test]
    fn sse_request_detected_from_subscribe_path() {
        let headers = HeaderMap::new();

        assert!(is_sse_request(&headers, "/v1/chats/subscribe"));
    }

    #[test]
    fn sse_request_false_for_regular_api_path() {
        let headers = HeaderMap::new();

        assert!(!is_sse_request(&headers, "/v1/chat/completions"));
    }

    #[tokio::test]
    async fn limited_body_bytes_rejects_proxy_bodies_over_limit() {
        let body = Body::from(vec![b'x'; PROXY_BODY_LIMIT + 1]);

        let response = limited_body_bytes(body).await.unwrap_err();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
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
    #[serial_test::serial]
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
        assert!(started.elapsed() >= PROXY_WORKER_READY_TIMEOUT);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"], "worker readiness timed out");
        state.supervisor.stop_all().await;
        let _ = tokio::time::timeout(Duration::from_secs(10), start)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
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

    #[tokio::test(start_paused = true)]
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
        let task = tokio::spawn(send_worker_request(request, true));

        accepted_rx.await.unwrap();
        tokio::time::advance(PROXY_STREAM_HEADER_TIMEOUT + Duration::from_millis(1)).await;
        assert!(matches!(
            task.await.unwrap(),
            Err(ProxySendError::StreamHeaderTimeout)
        ));
        server.abort();
    }

    #[tokio::test]
    async fn stream_request_keeps_body_unbounded_after_headers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
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
