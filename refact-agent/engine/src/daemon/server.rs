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
use tower_http::cors::CorsLayer;

use crate::daemon::config::DaemonConfig;
use crate::daemon::state::DaemonState;

#[derive(Debug, Serialize)]
struct StatusResponse {
    pid: u32,
    version: String,
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
    let ip = config
        .bind
        .parse::<IpAddr>()
        .map_err(|error| format!("invalid daemon bind address '{}': {error}", config.bind))?;
    crate::daemon::auth::validate_hooks_auth_policy(config, ip)?;
    let addr = SocketAddr::new(ip, config.port);
    let listener = TcpListener::bind(addr)
        .map_err(|error| format!("failed to bind daemon control API at {addr}: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set daemon listener nonblocking: {error}"))?;
    Ok(listener)
}

pub fn make_router(state: Arc<DaemonState>, port: u16) -> Router {
    let auth_token = state.auth_token.clone();
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
            "/p/:project_id/",
            get(crate::daemon::web::handle_project_gui_index),
        )
        .route(
            "/dist/chat/*path",
            get(crate::daemon::web::handle_daemon_gui_asset),
        )
        .route("/cron/status", get(cron_status))
        .route("/daemon/v1/status", get(status))
        .route("/daemon/v1/shutdown", post(shutdown))
        .route("/daemon/v1/events", get(events))
        .route("/daemon/v1/workers", get(workers))
        .route("/daemon/v1/logs", get(logs))
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
            let token = auth_token.clone();
            let hook_token = hook_token.clone();
            crate::daemon::auth::check_with_hooks(token, hook_token, open_hooks_allowed, req, next)
        }))
        .layer(CorsLayer::permissive())
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
        .serve(router.into_make_service())
        .with_graceful_shutdown(wait_for_shutdown(state));
    server
        .await
        .map_err(|error| format!("daemon server error: {error}"))
}

async fn status(State((state, port)): State<(Arc<DaemonState>, u16)>) -> Json<StatusResponse> {
    let uptime_secs =
        Duration::from_millis(crate::daemon::state::now_ms().saturating_sub(state.started_at_ms))
            .as_secs();
    Json(StatusResponse {
        pid: std::process::id(),
        version: state.version.clone(),
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
            hooks: crate::daemon::config::HooksConfig {
                enabled: true,
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let error = bind_listener(&wildcard).unwrap_err();
        assert!(error.contains("hooks without hooks.token or daemon auth"));
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
    async fn daemon_server_auth_enabled_rejects_missing_token() {
        use crate::daemon::config::AuthConfig;
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
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
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn daemon_server_auth_enabled_accepts_correct_token() {
        use crate::daemon::config::AuthConfig;
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
        let response = make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/daemon/v1/shutdown")
                    .header("content-type", "application/json")
                    .header("Authorization", "Bearer secret")
                    .body(Body::from(r#"{"reason":"t"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn daemon_server_auth_enabled_status_exempt() {
        use crate::daemon::config::AuthConfig;
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            auth: AuthConfig {
                enabled: true,
                token: Some("secret".to_string()),
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(
            config,
            EventBus::new(dir.path().join("events.jsonl")),
            Some("secret".to_string()),
        );
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

    async fn response_json(response: Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
}
