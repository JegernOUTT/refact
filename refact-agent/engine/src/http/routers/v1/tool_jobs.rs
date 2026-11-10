//! Inspection only: HTTP cannot submit executable tool jobs or mint authority.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use crate::app_state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/chats/:chat_id/tool-jobs/:job_id", get(inspect))
        .route("/chats/:chat_id/tool-jobs/:job_id/cancel", post(cancel))
}

const NOT_KEPT: &str = "tool job not found: unknown id, or its record is no longer kept (engine restart or retention limit)";

async fn inspect(
    State(app): State<AppState>,
    Path((chat_id, job_id)): Path<(String, String)>,
) -> Result<Json<crate::tool_jobs::JobSnapshot>, (StatusCode, &'static str)> {
    crate::chat::tool_job_delivery::checked_snapshot(&app, &chat_id, &job_id)
        .await
        .map(Json)
        .map_err(|_| (StatusCode::NOT_FOUND, NOT_KEPT))
}

async fn cancel(
    State(app): State<AppState>,
    Path((chat_id, job_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    app.runtime
        .tool_jobs
        .cancel(&chat_id, &job_id)
        .map(|status| Json(serde_json::json!({"job_id":job_id,"status":status})))
        .ok_or(StatusCode::NOT_FOUND)
}
