use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::python_tool::workspace_view::{workspace_view, WorkspaceView};

#[derive(Deserialize)]
pub struct WorkspaceQuery {
    chat_id: Option<String>,
}

pub async fn handle_v1_python_workspace(
    State(app): State<AppState>,
    Query(query): Query<WorkspaceQuery>,
) -> Result<Json<WorkspaceView>, ScratchError> {
    let chat_id = query
        .chat_id
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ScratchError::new(StatusCode::BAD_REQUEST, "chat_id is required".into()))?;
    crate::chat::trajectories::validate_trajectory_id(&chat_id)?;
    workspace_view(&app, &chat_id)
        .await
        .map(Json)
        .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn missing_parameter_is_400_and_workspace_json_shape_is_exact() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        let router = crate::http::routers::make_refact_http_server(app);
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/python/workspace")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/v1/python/workspace?chat_id=unknown-python-workspace")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&hyper::body::to_bytes(response.into_body()).await.unwrap())
                .unwrap();
        fn keys(value: &serde_json::Value, expected: &[&str]) {
            let mut expected = expected.to_vec();
            expected.sort();
            let mut actual = value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>();
            actual.sort();
            assert_eq!(actual, expected);
        }
        keys(
            &value,
            &[
                "schema",
                "chat_id",
                "exists",
                "workspace_dir",
                "artifacts_dir",
                "session",
                "artifacts",
                "artifacts_truncated",
                "runs",
                "runs_truncated",
                "disk",
            ],
        );
        keys(
            &value["session"],
            &[
                "status",
                "live",
                "state_persisted",
                "state_bytes",
                "saved_at_ms",
                "last_run_id",
                "reset_reason",
                "variables",
                "variables_truncated",
                "last_run_restored",
            ],
        );
        keys(&value["disk"], &["workspace_bytes", "quota_bytes"]);
        assert_eq!(value["exists"], false);
        assert_eq!(value["schema"], 1);
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    use axum::response::IntoResponse;
    #[tokio::test]
    async fn invalid_id_is_bad_request_not_internal_error() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let error = handle_v1_python_workspace(
            State(app),
            Query(WorkspaceQuery {
                chat_id: Some("../bad".into()),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.into_response().status(), StatusCode::BAD_REQUEST);
    }
}
