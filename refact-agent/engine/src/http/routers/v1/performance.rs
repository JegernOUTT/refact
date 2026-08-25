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
    use hyper::{Body, Request, StatusCode};
    use tower::ServiceExt;

    use super::*;
    use crate::chat::perf_diagnostics::{PerfComponent, PerfOutcome};

    async fn test_router() -> axum::Router {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::chat::perf_diagnostics::clear_process_telemetry_for_test();
        gcx.performance_telemetry.set_enabled(false);
        let app_state = AppState::from_gcx(gcx).await;
        crate::http::routers::v1::make_v1_router(app_state.clone()).with_state(app_state)
    }

    async fn request(router: axum::Router, request: Request<Body>) -> serde_json::Value {
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&hyper::body::to_bytes(response.into_body()).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn perf_telemetry_http_reports_disabled_and_runtime_enable_disable_reset() {
        let router = test_router().await;
        let initial = request(
            router.clone(),
            Request::builder()
                .uri("/performance/telemetry")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(initial["schema_version"], 1);
        assert_eq!(initial["enabled"], false);
        assert_eq!(
            initial["components"].as_array().unwrap().len(),
            PerfComponent::ALL.len()
        );
        assert!(initial["rollout_switches"].is_object());

        crate::chat::perf_diagnostics::record(
            PerfComponent::ToolRuntime,
            Some("private-chat"),
            PerfOutcome::Success,
            9,
            None,
            None,
            None,
        );
        let disabled = request(
            router.clone(),
            Request::builder()
                .uri("/performance/telemetry")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(disabled["components"][25]["sample_count"], 0);

        let enabled = request(
            router.clone(),
            Request::builder()
                .method("POST")
                .uri("/performance/telemetry")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"enabled":true}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(enabled["enabled"], true);

        crate::chat::perf_diagnostics::record(
            PerfComponent::ToolRuntime,
            Some("private-chat"),
            PerfOutcome::Success,
            9,
            None,
            None,
            None,
        );
        let populated = request(
            router.clone(),
            Request::builder()
                .uri("/performance/telemetry")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(populated["components"][25]["sample_count"], 1);

        let reset = request(
            router.clone(),
            Request::builder()
                .method("POST")
                .uri("/performance/telemetry/reset")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(reset["reset"], true);
        assert_eq!(reset["enabled"], true);

        let disabled = request(
            router,
            Request::builder()
                .method("POST")
                .uri("/performance/telemetry")
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"enabled":false}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(disabled["enabled"], false);
    }

    #[tokio::test]
    async fn perf_telemetry_http_response_has_no_private_event_fields() {
        let router = test_router().await;
        let response = request(
            router,
            Request::builder()
                .uri("/performance/telemetry")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let rendered = response.to_string();
        for forbidden in ["chat_id", "path", "query", "prompt", "arguments", "content"] {
            assert!(!rendered.contains(&format!("\"{forbidden}\"")));
        }
    }
}
