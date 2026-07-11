use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::HeaderMap;
use axum::middleware;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post};
use axum::{Json, Router};
use futures::Stream;
use hyper::{Server, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::cors::{AllowCredentials, AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};

use crate::daemon::config::DaemonConfig;
use crate::daemon::state::DaemonState;

#[derive(Debug, Serialize)]
struct StatusResponse {
    pid: u32,
    version: String,
    executable_sha256: Option<String>,
    port: u16,
    started_at_ms: u64,
    uptime_secs: u64,
    workers: u64,
    cron_pending: std::collections::HashMap<String, u64>,
}

#[derive(Debug, Deserialize)]
struct ShutdownRequest {
    reason: String,
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    #[serde(default)]
    follow: bool,
    after_seq: Option<u64>,
    last_event_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    project_id: Option<String>,
    #[serde(default = "default_log_tail")]
    tail: usize,
}

fn default_log_tail() -> usize {
    200
}

pub fn bind_listener(config: &DaemonConfig) -> Result<TcpListener, String> {
    let requested_ip = config
        .bind
        .parse::<IpAddr>()
        .map_err(|error| format!("invalid daemon bind address '{}': {error}", config.bind))?;
    crate::daemon::auth::validate_hooks_auth_policy(config, requested_ip)?;
    let ip = if !requested_ip.is_loopback() && !crate::daemon::auth::daemon_lan_auth_ok(config) {
        tracing::warn!(
            "daemon bind {requested_ip} is non-loopback but no auth credentials are configured; \
             falling back to 127.0.0.1 (enable auth with a username/password to allow LAN access)"
        );
        IpAddr::from([127, 0, 0, 1])
    } else {
        requested_ip
    };
    let addr = SocketAddr::new(ip, config.port);
    let listener = TcpListener::bind(addr)
        .map_err(|error| format!("failed to bind daemon control API at {addr}: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set daemon listener nonblocking: {error}"))?;
    Ok(listener)
}

pub fn make_router(state: Arc<DaemonState>, port: u16) -> Router {
    let (auth_token, auth_username, auth_password) = if state.config.auth.enabled {
        (
            state.auth_token.clone(),
            state.config.auth.username.clone(),
            state.config.auth.password.clone(),
        )
    } else {
        (None, None, None)
    };
    let hook_token = state.config.hooks.token.clone();
    let open_hooks_allowed =
        crate::daemon::auth::hooks_unauthenticated_allowed_for_bind(&state.config.bind);
    let control = Router::new()
        .route("/", get(crate::daemon::web::handle_project_picker))
        .route("/hooks", post(crate::daemon::hooks::bare))
        .route("/hooks/", post(crate::daemon::hooks::bare))
        .route("/hooks/wake", post(crate::daemon::hooks::wake))
        .route("/hooks/agent", post(crate::daemon::hooks::agent))
        .route("/hooks/:name", post(crate::daemon::hooks::named))
        .route(
            "/p/:project_id",
            get(crate::daemon::web::handle_project_gui_redirect),
        )
        .route(
            "/p/:project_id/",
            get(crate::daemon::web::handle_project_gui_index),
        )
        .route(
            "/dist/chat/*path",
            get(crate::daemon::web::handle_daemon_gui_asset),
        )
        .route("/cron/status", get(cron_status))
        .route("/daemon/v1/status", get(status))
        .route("/daemon/v1/settings", get(get_settings))
        .route("/daemon/v1/settings", post(update_settings))
        .route("/daemon/v1/shutdown", post(shutdown))
        .route("/daemon/v1/events", get(events))
        .route("/daemon/v1/workers", get(workers))
        .route("/daemon/v1/logs", get(logs))
        .route("/daemon/v1/logs/stream", get(logs_stream))
        .route("/daemon/v1/worker-status", post(worker_status))
        .route(
            "/daemon/v1/projects/open",
            post(crate::daemon::projects::open_project),
        )
        .route(
            "/daemon/v1/projects",
            get(crate::daemon::projects::list_projects),
        )
        .route(
            "/daemon/v1/projects/:id",
            get(crate::daemon::projects::get_project),
        )
        .route(
            "/daemon/v1/projects/:id",
            delete(crate::daemon::projects::forget_project),
        )
        .route(
            "/daemon/v1/projects/:id/pin",
            post(crate::daemon::projects::pin_project),
        )
        .route(
            "/daemon/v1/projects/:id/restart",
            post(crate::daemon::projects::restart_project_worker),
        )
        .route(
            "/daemon/v1/projects/:id/stop",
            post(crate::daemon::projects::stop_project_worker),
        )
        .layer(DefaultBodyLimit::disable());
    let proxy = Router::new()
        .route("/p/:project_id/v1", any(crate::daemon::proxy::proxy_v1))
        .route(
            "/p/:project_id/v1/*path",
            any(crate::daemon::proxy::proxy_v1),
        )
        .layer(DefaultBodyLimit::max(
            crate::daemon::proxy::PROXY_BODY_LIMIT,
        ));
    control
        .merge(proxy)
        .layer(middleware::from_fn(move |req, next| {
            let policy = crate::daemon::auth::DaemonAuthPolicy {
                token: auth_token.clone(),
                username: auth_username.clone(),
                password: auth_password.clone(),
                hook_token: hook_token.clone(),
                open_hooks_allowed,
            };
            crate::daemon::auth::enforce(policy, req, next)
        }))
        .layer(
            CorsLayer::new()
                .allow_methods(AllowMethods::mirror_request())
                .allow_headers(AllowHeaders::mirror_request())
                .allow_credentials(AllowCredentials::predicate(|origin, _parts| {
                    origin
                        .to_str()
                        .map(crate::daemon::auth::origin_allowed_for_loopback_or_ide)
                        .unwrap_or(false)
                }))
                .allow_origin(AllowOrigin::predicate(|origin, _parts| {
                    origin
                        .to_str()
                        .map(crate::daemon::auth::origin_allowed_for_loopback_or_ide)
                        .unwrap_or(false)
                })),
        )
        .with_state((state, port))
}

pub async fn serve(
    listener: TcpListener,
    state: Arc<DaemonState>,
    port: u16,
) -> Result<(), String> {
    let router = make_router(state.clone(), port);
    let server = Server::from_tcp(listener)
        .map_err(|error| format!("failed to create daemon server: {error}"))?
        .serve(router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(wait_for_shutdown(state));
    server
        .await
        .map_err(|error| format!("daemon server error: {error}"))
}

async fn status(State((state, port)): State<(Arc<DaemonState>, u16)>) -> Json<StatusResponse> {
    let uptime_secs =
        Duration::from_millis(crate::daemon::state::now_ms().saturating_sub(state.started_at_ms))
            .as_secs();
    let executable_sha256 =
        tokio::task::spawn_blocking(crate::daemon::state::current_executable_sha256)
            .await
            .unwrap_or(None);
    Json(StatusResponse {
        pid: std::process::id(),
        version: state.version.clone(),
        executable_sha256,
        port,
        started_at_ms: state.started_at_ms,
        uptime_secs,
        workers: state.supervisor.worker_count().await,
        cron_pending: state.cron_pending_snapshot().await,
    })
}

async fn shutdown(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Json(request): Json<ShutdownRequest>,
) -> Json<serde_json::Value> {
    state.request_shutdown(request.reason);
    Json(json!({"success": true}))
}

async fn workers(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
) -> Json<Vec<crate::daemon::state::WorkerRow>> {
    Json(state.worker_rows().await)
}

async fn cron_status(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
) -> Json<crate::daemon::cron_clock::CronClockStatus> {
    Json(crate::daemon::cron_clock::status(&state).await)
}

async fn logs(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Query(query): Query<LogsQuery>,
) -> Response {
    let path = match log_path(&state, query.project_id.as_deref()).await {
        Ok(path) => path,
        Err(response) => return response,
    };
    match tail_file(&path, query.tail.clamp(1, 10_000)).await {
        Ok(text) => ([("content-type", "text/plain; charset=utf-8")], text).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        )
            .into_response(),
    }
}

#[derive(Debug, Serialize)]
struct SettingsResponse {
    bind: String,
    lan_enabled: bool,
    mdns_enabled: bool,
    auth_enabled: bool,
    username: Option<String>,
    has_password: bool,
    hostname_local: String,
    urls: crate::daemon::state::DaemonUrls,
}

#[derive(Debug, Deserialize)]
struct UpdateSettingsRequest {
    lan_enabled: bool,
    mdns_enabled: bool,
    auth_enabled: bool,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

fn bind_is_loopback(bind: &str) -> bool {
    bind.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

async fn get_settings(
    State((state, port)): State<(Arc<DaemonState>, u16)>,
) -> Json<SettingsResponse> {
    let info = state.daemon_info(port, state.config.bind.clone());
    let lan_enabled = !bind_is_loopback(&state.config.bind);
    let mdns_enabled = state.config.mdns.enabled.unwrap_or(lan_enabled);
    Json(SettingsResponse {
        bind: state.config.bind.clone(),
        lan_enabled,
        mdns_enabled,
        auth_enabled: state.config.auth.enabled,
        username: state.config.auth.username.clone(),
        has_password: state
            .config
            .auth
            .password
            .as_deref()
            .map(|value| !value.is_empty())
            .unwrap_or(false),
        hostname_local: info.hostname_local,
        urls: info.urls,
    })
}

async fn update_settings(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Json(request): Json<UpdateSettingsRequest>,
) -> Response {
    let mut config = state.config.clone();
    config.bind = if request.lan_enabled {
        "0.0.0.0".to_string()
    } else {
        "127.0.0.1".to_string()
    };
    config.mdns.enabled = Some(request.mdns_enabled);
    config.auth.enabled = request.auth_enabled;
    config.auth.username = request
        .username
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(password) = request.password.filter(|value| !value.is_empty()) {
        config.auth.password = Some(password);
    }
    if request.lan_enabled {
        if !config.auth.enabled {
            return settings_error("Enabling LAN access requires authentication to be enabled.");
        }
        let has_username = config
            .auth
            .username
            .as_deref()
            .map(|value| !value.is_empty())
            .unwrap_or(false);
        let has_password = config
            .auth
            .password
            .as_deref()
            .map(|value| !value.is_empty())
            .unwrap_or(false);
        if !has_username || !has_password {
            return settings_error("Enabling LAN access requires a username and password.");
        }
    }
    if config.auth.enabled
        && config
            .auth
            .token
            .as_deref()
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
    {
        config.auth.token = Some(crate::daemon::auth::generate_token());
    }
    if let Err(error) =
        crate::daemon::config::save_to_path(&config, &crate::daemon::paths::daemon_config_path())
            .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": error})),
        )
            .into_response();
    }
    spawn_settings_relaunch();
    state.request_shutdown("settings_changed".to_string());
    Json(json!({"success": true, "restarting": true})).into_response()
}

fn settings_error(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response()
}

fn spawn_settings_relaunch() {
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!("settings relaunch: cannot resolve current executable: {error}");
            return;
        }
    };
    let mut command = std::process::Command::new(exe);
    command
        .arg("daemon")
        .env("REFACT_DAEMON_RELAUNCH", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    if let Err(error) = command.spawn() {
        tracing::warn!("settings relaunch spawn failed: {error}");
    }
}

async fn logs_stream(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Query(query): Query<LogsQuery>,
) -> Response {
    let path = match log_path(&state, query.project_id.as_deref()).await {
        Ok(path) => path,
        Err(response) => return response,
    };
    let initial_tail = query.tail.clamp(1, 10_000);
    let mut shutdown_rx = state.shutdown_receiver();
    let stream = async_stream::stream! {
        let initial = tail_file(&path, initial_tail).await.unwrap_or_default();
        for line in initial.lines() {
            yield Ok::<Event, Infallible>(Event::default().data(line.to_string()));
        }
        let mut cursor = LogTailCursor::default();
        if let Ok(mut file) = tokio::fs::File::open(&path).await {
            if let Ok(meta) = file.metadata().await {
                let len = meta.len();
                cursor.head = log_head(&mut file, len).await;
                cursor.offset = len;
            }
        }
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = tokio::time::sleep(Duration::from_millis(1000)) => {
                    let (text, next, error) = read_log_tail(&path, cursor).await;
                    cursor = next;
                    if let Some(error) = error {
                        yield Ok(Event::default().data(format!("[refact: {error}]")));
                        continue;
                    }
                    if !text.is_empty() {
                        let redacted = crate::daemon::auth::redact_daemon_query_token(&text);
                        for line in redacted.lines() {
                            yield Ok(Event::default().data(line.to_string()));
                        }
                    }
                }
            }
        }
    };
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

const LOG_STREAM_READ_CAP: u64 = 256 * 1024;
const LOG_STREAM_HEAD_LEN: u64 = 256;

#[derive(Default)]
struct LogTailCursor {
    offset: u64,
    head: Vec<u8>,
}

async fn log_head(file: &mut tokio::fs::File, len: u64) -> Vec<u8> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let want = len.min(LOG_STREAM_HEAD_LEN);
    if want == 0 {
        return Vec::new();
    }
    if file.seek(std::io::SeekFrom::Start(0)).await.is_err() {
        return Vec::new();
    }
    let mut buf = vec![0u8; want as usize];
    match file.read_exact(&mut buf).await {
        Ok(_) => buf,
        Err(_) => Vec::new(),
    }
}

async fn read_log_tail(
    path: &std::path::Path,
    cursor: LogTailCursor,
) -> (String, LogTailCursor, Option<String>) {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (String::new(), LogTailCursor::default(), None);
        }
        Err(error) => {
            return (
                String::new(),
                cursor,
                Some(format!("failed to open log: {error}")),
            )
        }
    };
    let len = match file.metadata().await {
        Ok(meta) => meta.len(),
        Err(error) => {
            return (
                String::new(),
                cursor,
                Some(format!("failed to stat log: {error}")),
            )
        }
    };
    let head = log_head(&mut file, len).await;
    let rotated = !cursor.head.is_empty() && (head != cursor.head || len < cursor.offset);
    let start = if rotated { 0 } else { cursor.offset.min(len) };
    if len <= start {
        return (String::new(), LogTailCursor { offset: len, head }, None);
    }
    let to_read = (len - start).min(LOG_STREAM_READ_CAP);
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return (
            String::new(),
            LogTailCursor {
                offset: start,
                head,
            },
            Some("failed to seek log".to_string()),
        );
    }
    let mut buf = vec![0u8; to_read as usize];
    if let Err(error) = file.read_exact(&mut buf).await {
        return (
            String::new(),
            LogTailCursor {
                offset: start,
                head,
            },
            Some(format!("failed to read log: {error}")),
        );
    }
    (
        String::from_utf8_lossy(&buf).to_string(),
        LogTailCursor {
            offset: start + to_read,
            head,
        },
        None,
    )
}

async fn worker_status(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Json(report): Json<crate::daemon_link::WorkerStatusReport>,
) -> Response {
    match state.store_validated_worker_status(report).await {
        Ok(event_emitted) => {
            Json(json!({"success": true, "event_emitted": event_emitted})).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"success": false, "error": error})),
        )
            .into_response(),
    }
}

async fn events(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = query.follow.then(|| state.events.subscribe());
    let cursor = event_cursor(&headers, &query).unwrap_or(0);
    let initial = state.events.replay_after(cursor).await;
    let events = state.events.clone();
    let mut shutdown_rx = state.shutdown_receiver();
    let stream = async_stream::stream! {
        let mut last_seq = cursor;
        if let Some(gap) = initial.gap {
            let event = resync_event(&gap);
            if event.seq > last_seq {
                last_seq = event.seq;
                yield Ok(sse_event(&event));
            }
        }
        for event in initial.events {
            if event.seq > last_seq {
                last_seq = event.seq;
                yield Ok(sse_event(&event));
            }
        }
        if let Some(mut rx) = rx {
            loop {
                tokio::select! {
                    result = rx.recv() => match result {
                        Ok(event) => {
                            if event.seq > last_seq {
                                last_seq = event.seq;
                                yield Ok(sse_event(&event));
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let replay = events.replay_after(last_seq).await;
                            if let Some(gap) = replay.gap {
                                let event = resync_event(&gap);
                                if event.seq > last_seq {
                                    last_seq = event.seq;
                                    yield Ok(sse_event(&event));
                                }
                            }
                            for event in replay.events {
                                if event.seq > last_seq {
                                    last_seq = event.seq;
                                    yield Ok(sse_event(&event));
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    _ = shutdown_rx.recv() => break,
                }
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn event_cursor(headers: &HeaderMap, query: &EventsQuery) -> Option<u64> {
    query
        .after_seq
        .or(query.last_event_id)
        .or_else(|| header_seq(headers, "last-event-id"))
}

fn header_seq(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}

fn resync_event(gap: &crate::daemon::events::EventReplayGap) -> crate::daemon::events::DaemonEvent {
    crate::daemon::events::DaemonEvent {
        seq: gap.oldest_seq.saturating_sub(1),
        ts_ms: crate::daemon::state::now_ms(),
        kind: "daemon_events_resync_required".to_string(),
        project_id: None,
        payload: json!({
            "requested_after_seq": gap.requested_after_seq,
            "oldest_seq": gap.oldest_seq,
            "latest_seq": gap.latest_seq,
        }),
    }
}

async fn log_path(state: &Arc<DaemonState>, project_id: Option<&str>) -> Result<PathBuf, Response> {
    let Some(project_id) = project_id.filter(|value| !value.is_empty()) else {
        return Ok(state.daemon_dir.join("logs").join("daemon.log"));
    };
    let registry = state.projects.read().await;
    match registry.get(project_id) {
        Some(entry) => Ok(state
            .daemon_dir
            .join("logs")
            .join(format!("worker-{}.log", entry.slug))),
        None => Err((StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response()),
    }
}

async fn tail_file(path: &std::path::Path, tail: usize) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(format!("failed to open {}: {error}", path.display())),
    };
    let len = file
        .metadata()
        .await
        .map_err(|error| format!("failed to stat {}: {error}", path.display()))?
        .len();
    let start = len.saturating_sub(1024 * 1024);
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|error| format!("failed to seek {}: {error}", path.display()))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .await
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let text = String::from_utf8_lossy(&buf);
    let lines = text.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(tail);
    let mut out = lines[start..].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(crate::daemon::auth::redact_daemon_query_token(&out))
}

fn sse_event(event: &crate::daemon::events::DaemonEvent) -> Event {
    Event::default()
        .id(event.seq.to_string())
        .event("daemon")
        .data(serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string()))
}

async fn wait_for_shutdown(state: Arc<DaemonState>) {
    let mut rx = state.shutdown_receiver();
    let _ = rx.recv().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::events::EventBus;
    use crate::daemon::projects::ProjectRegistry;

    async fn test_state(dir: &tempfile::TempDir) -> Arc<DaemonState> {
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        *state.projects.write().await = ProjectRegistry::empty(dir.path().join("projects.json"));
        state
    }

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
            let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

    #[test]
    fn daemon_server_bind_rejects_invalid_host() {
        let config = DaemonConfig {
            bind: "localhost".to_string(),
            ..DaemonConfig::default()
        };
        assert!(bind_listener(&config).is_err());
    }

    #[test]
    fn daemon_server_bind_enforces_loopback_open_hooks_policy() {
        let loopback = DaemonConfig {
            bind: "127.0.0.1".to_string(),
            port: 0,
            hooks: crate::daemon::config::HooksConfig {
                enabled: true,
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let listener = bind_listener(&loopback).unwrap();
        assert!(listener.local_addr().unwrap().ip().is_loopback());

        let wildcard = DaemonConfig {
            bind: "0.0.0.0".to_string(),
            hooks: crate::daemon::config::HooksConfig {
                enabled: true,
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let error = bind_listener(&wildcard).unwrap_err();
        assert!(error.contains("hooks without hooks.token or daemon auth"));
    }

    #[test]
    fn daemon_server_bind_falls_back_to_loopback_without_lan_auth() {
        let wildcard = DaemonConfig {
            bind: "0.0.0.0".to_string(),
            port: 0,
            ..DaemonConfig::default()
        };
        let listener = bind_listener(&wildcard).unwrap();
        assert!(listener.local_addr().unwrap().ip().is_loopback());
    }

    #[tokio::test]
    async fn daemon_server_status_router_reports_workers_zero() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .uri("/daemon/v1/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["workers"], 0);
        assert_eq!(json["cron_pending"], serde_json::json!({}));
        assert_eq!(json["port"], 8488);
        let executable_sha256 = json["executable_sha256"]
            .as_str()
            .expect("daemon status must report executable_sha256 for staleness detection");
        assert_eq!(executable_sha256.len(), 64);
    }

    #[tokio::test]
    async fn daemon_cron_status_reports_pending_clock_shape() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        state.set_cron_pending("project-a", Some(200_000)).await;
        state.set_cron_pending("project-b", Some(150_000)).await;
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .uri("/cron/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["enabled"], true);
        assert_eq!(json["jobs"], 2);
        assert_eq!(json["next_wake_ms"], 60_000);
    }

    #[tokio::test]
    async fn daemon_server_auth_disabled_passthrough() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/daemon/v1/shutdown")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"reason":"t"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn update_settings_rejects_lan_without_credentials() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/daemon/v1/settings")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"lan_enabled":true,"mdns_enabled":true,"auth_enabled":false}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn daemon_server_auth_enabled_rejects_missing_token() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let mut request = Request::builder()
            .method("POST")
            .uri("/daemon/v1/shutdown")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"reason":"t"}"#))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000))));
        let response = make_router(state, 8488).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn daemon_server_auth_enabled_accepts_correct_token() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let mut request = Request::builder()
            .method("POST")
            .uri("/daemon/v1/shutdown")
            .header("content-type", "application/json")
            .header("Authorization", "Bearer secret")
            .body(Body::from(r#"{"reason":"t"}"#))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000))));
        let response = make_router(state, 8488).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn daemon_server_auth_enabled_accepts_basic_auth_from_remote() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use base64::Engine;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                username: Some("alice".to_string()),
                password: Some("hunter2".to_string()),
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let encoded = base64::prelude::BASE64_STANDARD.encode("alice:hunter2");
        let mut request = Request::builder()
            .uri("/daemon/v1/projects")
            .header("Authorization", format!("Basic {encoded}"))
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000))));
        let response = make_router(state, 8488).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn daemon_server_auth_enabled_loopback_needs_no_credentials() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                username: Some("alice".to_string()),
                password: Some("hunter2".to_string()),
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let mut request = Request::builder()
            .uri("/daemon/v1/projects")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        let response = make_router(state, 8488).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn daemon_server_auth_enabled_status_exempt() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let mut request = Request::builder()
            .uri("/daemon/v1/status")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000))));
        let response = make_router(state, 8488).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn daemon_control_plane_body_is_not_proxy_limited() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let body = serde_json::json!({
            "project_id": "missing",
            "pid": 1,
            "lsp_clients": 0,
            "busy_chats": 0,
            "exec_running": 0,
            "last_activity_ts": 0,
            "padding": "x".repeat(crate::daemon::proxy::PROXY_BODY_LIMIT + 1024),
        })
        .to_string();

        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/daemon/v1/worker-status")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn tail_file_redacts_daemon_token_query_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.log");
        tokio::fs::write(
            &path,
            "GET /p/project/v1?daemon_token=secret-token&chat=1 failed\n",
        )
        .await
        .unwrap();

        let text = tail_file(&path, 10).await.unwrap();

        assert!(!text.contains("secret-token"));
        assert!(text.contains("daemon_token=<redacted>&chat=1"));
    }

    #[tokio::test]
    async fn worker_status_handler_stores_report_and_emits_only_on_change() {
        use crate::daemon_link::WorkerStatusReport;
        use tokio::time::{timeout, Duration};

        fn report(
            project_id: &str,
            pid: u32,
            lsp_clients: usize,
            busy_chats: usize,
        ) -> WorkerStatusReport {
            WorkerStatusReport {
                project_id: project_id.to_string(),
                pid,
                instance_token: "token".to_string(),
                lsp_clients,
                busy_chats,
                exec_running: 0,
                last_activity_ts: 55,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let mut events = state.events.subscribe();
        let root = tempfile::tempdir().unwrap();
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(root.path().to_path_buf()).await.unwrap()
        };
        state
            .supervisor
            .set_test_worker_info(
                &entry.id,
                123,
                crate::daemon::supervisor::WorkerState::Ready,
                "token",
            )
            .await;

        let first = worker_status(
            State((state.clone(), 8488)),
            Json(report(&entry.id, 123, 1, 0)),
        )
        .await;
        let (status, first) = response_json(first).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first["event_emitted"], true);
        let event = timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.kind, "worker_status");
        assert_eq!(event.project_id.as_deref(), Some(entry.id.as_str()));
        assert_eq!(
            state
                .latest_worker_status(&entry.id)
                .await
                .unwrap()
                .lsp_clients,
            1
        );

        let second = worker_status(
            State((state.clone(), 8488)),
            Json(report(&entry.id, 123, 1, 0)),
        )
        .await;
        let (status, second) = response_json(second).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(second["event_emitted"], false);
        assert!(timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err());

        let third = worker_status(
            State((state.clone(), 8488)),
            Json(report(&entry.id, 123, 2, 0)),
        )
        .await;
        let (status, third) = response_json(third).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(third["event_emitted"], true);
        let event = timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.payload["lsp_clients"], 2);
    }

    #[tokio::test]
    async fn worker_status_handler_rejects_missing_project_and_wrong_pid() {
        use crate::daemon_link::WorkerStatusReport;

        fn report(project_id: &str, pid: u32) -> WorkerStatusReport {
            WorkerStatusReport {
                project_id: project_id.to_string(),
                pid,
                instance_token: "token".to_string(),
                lsp_clients: 1,
                busy_chats: 0,
                exec_running: 0,
                last_activity_ts: 55,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let root = tempfile::tempdir().unwrap();
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(root.path().to_path_buf()).await.unwrap()
        };
        state
            .supervisor
            .set_test_worker_info(
                &entry.id,
                123,
                crate::daemon::supervisor::WorkerState::Ready,
                "token",
            )
            .await;

        let missing =
            worker_status(State((state.clone(), 8488)), Json(report("missing", 123))).await;
        let (status, missing) = response_json(missing).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(missing["success"], false);
        assert!(missing["error"]
            .as_str()
            .unwrap()
            .contains("project not found"));

        let wrong_pid =
            worker_status(State((state.clone(), 8488)), Json(report(&entry.id, 999))).await;
        let (status, wrong_pid) = response_json(wrong_pid).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(wrong_pid["success"], false);
        assert!(wrong_pid["error"]
            .as_str()
            .unwrap()
            .contains("current worker"));
        assert!(state.latest_worker_status(&entry.id).await.is_none());
    }

    #[tokio::test]
    async fn cors_preflight_allows_ide_webview_origins_on_proxy_path() {
        use axum::http::header;
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/p/project/v1/ping")
                    .header(header::ORIGIN, "http://refactai")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("http://refactai")
        );
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
    }

    #[tokio::test]
    async fn cors_preflight_allows_vscode_webview_origin_on_subscribe_path() {
        use axum::http::header;
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let origin = "vscode-webview://11112222-3333-4444-5555-666677778888";
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/p/project/v1/sidebar/subscribe")
                    .header(header::ORIGIN, origin)
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some(origin)
        );
    }

    #[tokio::test]
    async fn cors_preflight_rejects_untrusted_origin() {
        use axum::http::header;
        use hyper::{Body, Request};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/p/project/v1/ping")
                    .header(header::ORIGIN, "http://evil.example")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn daemon_proxy_path_trusts_ide_webview_origin_and_rejects_others() {
        use axum::extract::ConnectInfo;
        use axum::http::header;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let Some(_env) = EnvGuard::fake_worker() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(&dir).await;
        let project_root = dir.path().join("proxy-project");
        std::fs::create_dir_all(&project_root).unwrap();
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(project_root).await.unwrap()
        };
        state.sync_project_liveness(&entry).await;
        let proxy_path = format!("/p/{}/v1/ping", entry.id);

        let mut trusted = Request::builder()
            .uri(&proxy_path)
            .header(header::ORIGIN, "http://refactai")
            .body(Body::empty())
            .unwrap();
        trusted
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        let response = make_router(state.clone(), 8488)
            .oneshot(trusted)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("http://refactai")
        );

        let mut untrusted = Request::builder()
            .uri(&proxy_path)
            .header(header::ORIGIN, "http://evil.example")
            .body(Body::empty())
            .unwrap();
        untrusted
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        let response = make_router(state.clone(), 8488)
            .oneshot(untrusted)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        state.supervisor.stop_all().await;
    }

    #[tokio::test]
    async fn daemon_auth_trusts_ide_webview_origin_but_requires_auth_from_remote() {
        use crate::daemon::config::AuthConfig;
        use axum::extract::ConnectInfo;
        use axum::http::header;
        use hyper::{Body, Request, StatusCode};
        use std::net::SocketAddr;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );

        let mut trusted = Request::builder()
            .uri("/daemon/v1/projects")
            .header(header::ORIGIN, "http://refactai")
            .body(Body::empty())
            .unwrap();
        trusted
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        let response = make_router(state.clone(), 8488)
            .oneshot(trusted)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("http://refactai")
        );

        for origin in ["http://evil.example", "null"] {
            let mut request = Request::builder()
                .uri("/daemon/v1/projects")
                .header(header::ORIGIN, origin)
                .body(Body::empty())
                .unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
            let response = make_router(state.clone(), 8488)
                .oneshot(request)
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "loopback origin {origin} must not be trusted"
            );
        }

        let mut remote = Request::builder()
            .uri("/daemon/v1/projects")
            .header(header::ORIGIN, "http://refactai")
            .body(Body::empty())
            .unwrap();
        remote
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40000))));
        let response = make_router(state, 8488).oneshot(remote).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    async fn response_json(response: Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
}
