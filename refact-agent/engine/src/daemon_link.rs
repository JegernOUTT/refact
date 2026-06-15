use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::chat::types::SessionState;
use crate::exec::{ExecProcessFilter, ExecStatusKind};
use crate::global_context::GlobalContext;

pub const DAEMON_LINK_INTERVAL: Duration = Duration::from_secs(10);
const DAEMON_LINK_POST_TIMEOUT: Duration = Duration::from_secs(2);
pub const DAEMON_LINK_FAILURE_LIMIT: u32 = 6;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerStatusReport {
    pub project_id: String,
    pub pid: u32,
    pub lsp_clients: usize,
    pub busy_chats: usize,
    pub exec_running: usize,
    pub last_activity_ts: u64,
}

pub fn should_self_terminate(consecutive_failures: u32) -> bool {
    consecutive_failures >= DAEMON_LINK_FAILURE_LIMIT
}

pub async fn collect_status_snapshot(gcx: Arc<GlobalContext>) -> WorkerStatusReport {
    let project_id = gcx.cmdline.project_id.clone();
    let lsp_clients = gcx.lsp_tcp_client_count.load(Ordering::SeqCst);
    let (busy_chats, last_activity_ts) = collect_chat_status(gcx.clone()).await;
    let exec_running = gcx
        .exec_registry
        .list(ExecProcessFilter {
            status: Some(ExecStatusKind::Running),
            ..Default::default()
        })
        .await
        .len();
    WorkerStatusReport {
        project_id,
        pid: std::process::id(),
        lsp_clients,
        busy_chats,
        exec_running,
        last_activity_ts,
    }
}

async fn collect_chat_status(gcx: Arc<GlobalContext>) -> (usize, u64) {
    let sessions = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.values().cloned().collect::<Vec<_>>()
    };
    let mut busy_chats = 0;
    let mut newest_activity: Option<u64> = None;
    for session_arc in sessions {
        let session = session_arc.lock().await;
        if matches!(
            session.runtime.state,
            SessionState::Generating | SessionState::ExecutingTools
        ) {
            busy_chats += 1;
        }
        let activity = std::time::Instant::now()
            .checked_duration_since(session.last_activity)
            .and_then(|elapsed| {
                current_timestamp_ms()
                    .checked_sub(elapsed.as_millis().min(u128::from(u64::MAX)) as u64)
            })
            .unwrap_or_else(current_timestamp_ms);
        newest_activity = Some(newest_activity.map_or(activity, |current| current.max(activity)));
    }
    (
        busy_chats,
        newest_activity.unwrap_or_else(current_timestamp_ms),
    )
}

pub async fn daemon_link_task(gcx: Arc<GlobalContext>) {
    let daemon_endpoint = gcx
        .cmdline
        .daemon_endpoint
        .trim_end_matches('/')
        .to_string();
    if daemon_endpoint.is_empty() {
        return;
    }
    let url = format!("{daemon_endpoint}/daemon/v1/worker-status");
    let mut interval = tokio::time::interval(DAEMON_LINK_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    let mut consecutive_failures = 0;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let report = collect_status_snapshot(gcx.clone()).await;
                match post_worker_status(gcx.clone(), &url, &report).await {
                    Ok(()) => consecutive_failures = 0,
                    Err(error) => {
                        consecutive_failures += 1;
                        tracing::warn!(
                            "daemon link status push failed ({consecutive_failures}/{}): {error}",
                            DAEMON_LINK_FAILURE_LIMIT
                        );
                        if should_self_terminate(consecutive_failures) {
                            tracing::error!("daemon lost, self-terminating");
                            let _ = gcx
                                .ask_shutdown_sender
                                .lock()
                                .unwrap()
                                .send("daemon-lost".to_string());
                            return;
                        }
                    }
                }
            }
            _ = wait_for_shutdown(gcx.clone()) => {
                return;
            }
        }
    }
}

async fn wait_for_shutdown(gcx: Arc<GlobalContext>) {
    while !gcx.shutdown_flag.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn post_worker_status(
    gcx: Arc<GlobalContext>,
    url: &str,
    report: &WorkerStatusReport,
) -> Result<(), String> {
    let mut request = gcx.http_client.post(url).json(report);
    if let Some(token) = &gcx.cmdline.daemon_auth_token {
        request = request.bearer_auth(token);
    }
    let response = tokio::time::timeout(DAEMON_LINK_POST_TIMEOUT, request.send())
        .await
        .map_err(|_| "request timed out".to_string())?
        .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("daemon returned {}", response.status()))
    }
}

fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::ChatSession;
    use crate::exec::{ExecMode, ExecProcessMeta, ExecStatus};
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::json;
    use tokio::sync::Mutex as AMutex;

    #[test]
    fn self_terminate_threshold_is_six_failures() {
        assert!(!should_self_terminate(5));
        assert!(should_self_terminate(6));
        assert!(should_self_terminate(7));
    }

    #[test]
    fn worker_status_serialization_roundtrip() {
        let report = WorkerStatusReport {
            project_id: "abc123".to_string(),
            pid: 42,
            lsp_clients: 2,
            busy_chats: 1,
            exec_running: 3,
            last_activity_ts: 99,
        };
        let encoded = serde_json::to_string(&report).unwrap();
        let decoded: WorkerStatusReport = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, report);
    }

    #[tokio::test]
    async fn collect_status_snapshot_counts_runtime_state() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.lsp_tcp_client_count.store(2, Ordering::SeqCst);
        let mut generating = ChatSession::new("generating".to_string());
        generating.runtime.state = SessionState::Generating;
        let mut idle = ChatSession::new("idle".to_string());
        idle.runtime.state = SessionState::Idle;
        gcx.chat_sessions
            .write()
            .await
            .insert("generating".to_string(), Arc::new(AMutex::new(generating)));
        gcx.chat_sessions
            .write()
            .await
            .insert("idle".to_string(), Arc::new(AMutex::new(idle)));
        let snapshot = gcx
            .exec_registry
            .register(
                ExecProcessMeta::new(ExecMode::Background, "sleep".to_string()),
                1024,
            )
            .await;
        gcx.exec_registry
            .set_status(&snapshot.meta.process_id, ExecStatus::Running)
            .await
            .unwrap();

        let report = collect_status_snapshot(gcx).await;

        assert_eq!(report.lsp_clients, 2);
        assert_eq!(report.busy_chats, 1);
        assert_eq!(report.exec_running, 1);
        assert!(report.last_activity_ts > 0);
    }

    #[tokio::test]
    async fn post_worker_status_sends_bearer_token() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = Router::new().route(
            "/daemon/v1/worker-status",
            post(
                |headers: axum::http::HeaderMap, Json(_): Json<WorkerStatusReport>| async move {
                    if headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        == Some("Bearer secret-token")
                    {
                        (axum::http::StatusCode::OK, Json(json!({"success": true})))
                    } else {
                        (
                            axum::http::StatusCode::UNAUTHORIZED,
                            Json(json!({"error": "Unauthorized"})),
                        )
                    }
                },
            ),
        );
        let server = axum::Server::from_tcp(listener.into_std().unwrap())
            .unwrap()
            .serve(app.into_make_service());
        let task = tokio::spawn(server);
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx).unwrap().cmdline.daemon_auth_token =
            Some("secret-token".to_string());
        let report = WorkerStatusReport {
            project_id: "project".to_string(),
            pid: 42,
            lsp_clients: 0,
            busy_chats: 0,
            exec_running: 0,
            last_activity_ts: 1,
        };

        let result = post_worker_status(
            gcx,
            &format!("http://127.0.0.1:{port}/daemon/v1/worker-status"),
            &report,
        )
        .await;

        assert_eq!(result, Ok(()));
        task.abort();
    }
}
