use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::Stream;
use hyper::Server;
use serde::{Deserialize, Serialize};
use serde_json::json;

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
    Router::new()
        .route("/daemon/v1/status", get(status))
        .route("/daemon/v1/shutdown", post(shutdown))
        .route("/daemon/v1/events", get(events))
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
        workers: 0,
    })
}

async fn shutdown(
    State((state, _)): State<(Arc<DaemonState>, u16)>,
    Json(request): Json<ShutdownRequest>,
) -> Json<serde_json::Value> {
    state.request_shutdown(request.reason);
    Json(json!({"success": true}))
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
        assert_eq!(json["port"], 8488);
    }
}
