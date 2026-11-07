use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::chat::perf_telemetry::{PerformanceTelemetrySnapshot, PERFORMANCE_TELEMETRY_SCHEMA_VERSION};
use crate::custom_error::ScratchError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceTelemetrySettingsRequest {
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct PerformanceTelemetrySettingsResponse {
    schema_version: u8,
    enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct PerformanceTelemetryResetResponse {
    schema_version: u8,
    reset: bool,
    enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct PerformanceTelemetryResponse {
    #[serde(flatten)]
    telemetry: PerformanceTelemetrySnapshot,
    rollout_switches: RolloutSwitches,
    queue_diagnostics: crate::chat::live_diagnostics::QueueDiagnostics,
}

#[derive(Debug, Serialize)]
struct RolloutSwitches {
    trajectory_writer_enabled: bool,
    trajectory_index_coordinator_enabled: bool,
    trajectory_watcher_self_write_enabled: bool,
    tool_catalog_snapshots_enabled: bool,
    vecdb_path_coalescing_enabled: bool,
}

fn rollout_switches() -> RolloutSwitches {
    RolloutSwitches {
        trajectory_writer_enabled: crate::chat::trajectories::trajectory_writer_rollout_enabled(),
        trajectory_index_coordinator_enabled:
            crate::chat::trajectory_index::trajectory_index_coordinator_rollout_enabled(),
        trajectory_watcher_self_write_enabled:
            crate::chat::trajectories::trajectory_watcher_self_write_rollout_enabled(),
        tool_catalog_snapshots_enabled: crate::app_state::tool_catalog_snapshot_rollout_enabled(),
        vecdb_path_coalescing_enabled:
            refact_vecdb::vdb_thread::vecdb_path_coalescing_rollout_enabled(),
    }
}

pub async fn handle_v1_performance_telemetry_get(
    State(app): State<AppState>,
) -> Json<PerformanceTelemetryResponse> {
    Json(PerformanceTelemetryResponse {
        telemetry: app.gcx.performance_telemetry.snapshot(),
        rollout_switches: rollout_switches(),
        queue_diagnostics: crate::chat::live_diagnostics::sample(&app),
    })
}

pub async fn handle_v1_performance_telemetry_post(
    State(app): State<AppState>,
    Json(request): Json<PerformanceTelemetrySettingsRequest>,
) -> Json<PerformanceTelemetrySettingsResponse> {
    app.gcx.performance_telemetry.set_enabled(request.enabled);
    Json(PerformanceTelemetrySettingsResponse {
        schema_version: PERFORMANCE_TELEMETRY_SCHEMA_VERSION,
        enabled: app.gcx.performance_telemetry.enabled(),
    })
}

pub async fn handle_v1_performance_telemetry_reset(
    State(app): State<AppState>,
) -> Result<Json<PerformanceTelemetryResetResponse>, ScratchError> {
    app.gcx.performance_telemetry.reset();
    Ok(Json(PerformanceTelemetryResetResponse {
        schema_version: PERFORMANCE_TELEMETRY_SCHEMA_VERSION,
        reset: true,
        enabled: app.gcx.performance_telemetry.enabled(),
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::routing::get;
    use hyper::{Body, Request, StatusCode};
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    use super::*;

    async fn request(router: axum::Router, request: Request<Body>) -> serde_json::Value {
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&hyper::body::to_bytes(response.into_body()).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn live_queue_http_reports_partial_coverage_without_waiting_for_held_locks() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        app.gcx.performance_telemetry.set_enabled(false);
        let session = Arc::new(Mutex::new(crate::chat::types::ChatSession::new(
            "private-diagnostic-chat".into(),
        )));
        let command = serde_json::from_value(serde_json::json!({
            "client_request_id": "private-diagnostic-request",
            "priority": true,
            "type": "answer_questions",
            "tool_call_id": "private-diagnostic-question",
            "answers": []
        }))
        .unwrap();
        session.lock().await.command_queue.push_back(command);
        app.chat
            .sessions
            .write()
            .await
            .insert("private-diagnostic-chat".into(), session.clone());
        let router = axum::Router::new()
            .route(
                "/v1/performance/telemetry",
                get(handle_v1_performance_telemetry_get),
            )
            .with_state(app.clone());
        let get = || {
            Request::get("/v1/performance/telemetry")
                .body(Body::empty())
                .unwrap()
        };

        let initial = request(router.clone(), get()).await;
        assert_eq!(initial["enabled"], false);
        assert_eq!(initial["queue_diagnostics"]["commands"]["pending_count"], 1);
        assert_eq!(
            initial["queue_diagnostics"]["question_answers"]["pending_count"],
            1
        );
        assert_eq!(
            initial["queue_diagnostics"]["commands"]["unknown_age_count"],
            1
        );
        assert!(!initial.to_string().contains("private-diagnostic"));

        let held_session = session.lock().await;
        let partial = tokio::time::timeout(Duration::from_secs(1), request(router.clone(), get()))
            .await
            .expect("performance GET must not wait for a held chat lock");
        assert_eq!(partial["queue_diagnostics"]["coverage"]["skipped_busy"], 1);
        assert_eq!(partial["queue_diagnostics"]["coverage"]["partial"], true);
        drop(held_session);

        let held_registry = app.chat.sessions.write().await;
        let partial = tokio::time::timeout(Duration::from_secs(1), request(router.clone(), get()))
            .await
            .expect("performance GET must not wait for a held registry lock");
        assert_eq!(
            partial["queue_diagnostics"]["coverage"]["sessions_total"],
            serde_json::Value::Null
        );
        assert_eq!(partial["queue_diagnostics"]["coverage"]["partial"], true);
        drop(held_registry);

        let restored = request(router, get()).await;
        assert_eq!(
            restored["queue_diagnostics"]["commands"]["pending_count"],
            1
        );
        assert_eq!(restored["queue_diagnostics"]["coverage"]["partial"], false);
        assert_eq!(session.lock().await.command_queue.len(), 1);
    }
}
