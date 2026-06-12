use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Query, State};
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
    Router::new()
        .route("/", get(crate::daemon::web::handle_project_picker))
        .route(
            "/p/:project_id/",
            get(crate::daemon::web::handle_project_gui_index),
        )
        .route(
            "/dist/chat/*path",
            get(crate::daemon::web::handle_daemon_gui_asset),
        )
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
        .route("/p/:project_id/v1", any(crate::daemon::proxy::proxy_v1))
        .route(
            "/p/:project_id/v1/*path",
            any(crate::daemon::proxy::proxy_v1),
        )
        .route(
            "/p/:project_id/build_info",
            any(crate::daemon::proxy::proxy_build_info),
        )
        .layer(DefaultBodyLimit::max(
            crate::daemon::proxy::PROXY_BODY_LIMIT,
        ))
        .layer(middleware::from_fn(move |req, next| {
            let token = auth_token.clone();
            crate::daemon::auth::check(token, req, next)
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
) -> Json<serde_json::Value> {
    let event_emitted = state.store_worker_status(report).await;
    Json(json!({"success": true, "event_emitted": event_emitted}))
}

async fn events(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let snapshot = state.events.snapshot().await;
    let mut rx = state.events.subscribe();
    let mut shutdown_rx = state.shutdown_receiver();
    let stream = async_stream::stream! {
        for event in snapshot {
            yield Ok(sse_event(&event));
        }
        if query.follow {
            loop {
                tokio::select! {
                    result = rx.recv() => match result {
                        Ok(event) => yield Ok(sse_event(&event)),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    _ = shutdown_rx.recv() => break,
                }
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
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
    Ok(out)
}

fn sse_event(event: &crate::daemon::events::DaemonEvent) -> Event {
    Event::default()
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

    #[test]
    fn daemon_server_bind_rejects_invalid_host() {
        let config = DaemonConfig {
            bind: "localhost".to_string(),
            ..DaemonConfig::default()
        };
        assert!(bind_listener(&config).is_err());
    }

    #[tokio::test]
    async fn daemon_server_status_router_reports_workers_zero() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
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
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["workers"], 0);
        assert_eq!(json["cron_pending"], serde_json::json!({}));
        assert_eq!(json["port"], 8488);
    }

    #[tokio::test]
    async fn daemon_server_auth_disabled_passthrough() {
        use hyper::{Body, Request, StatusCode};
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
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
    async fn worker_status_handler_stores_report_and_emits_only_on_change() {
        use crate::daemon_link::WorkerStatusReport;
        use tokio::time::{timeout, Duration};

        fn report(lsp_clients: usize, busy_chats: usize) -> WorkerStatusReport {
            WorkerStatusReport {
                project_id: "project".to_string(),
                pid: 123,
                lsp_clients,
                busy_chats,
                exec_running: 0,
                last_activity_ts: 55,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        let mut events = state.events.subscribe();

        let first = worker_status(State((state.clone(), 8488)), Json(report(1, 0))).await;
        assert_eq!(first.0["event_emitted"], true);
        let event = timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.kind, "worker_status");
        assert_eq!(event.project_id.as_deref(), Some("project"));
        assert_eq!(
            state
                .latest_worker_status("project")
                .await
                .unwrap()
                .lsp_clients,
            1
        );

        let second = worker_status(State((state.clone(), 8488)), Json(report(1, 0))).await;
        assert_eq!(second.0["event_emitted"], false);
        assert!(timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err());

        let third = worker_status(State((state.clone(), 8488)), Json(report(2, 0))).await;
        assert_eq!(third.0["event_emitted"], true);
        let event = timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.payload["lsp_clients"], 2);
    }
}
