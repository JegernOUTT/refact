use axum::extract::State;
use axum::response::Result;
use hyper::{Body, Response};
use serde_json::json;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;

pub async fn handle_v1_graceful_shutdown(
    State(app): State<AppState>,
    _: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    // Do not signal process shutdown or report success while background
    // effects are unresolved. The lifecycle barrier closes admission first.
    app.runtime
        .tool_jobs
        .scopes
        .begin_shutdown_requested("HTTP /v1/graceful-shutdown");
    crate::chat::close_all_chat_sessions(app.clone())
        .await
        .map_err(|_| {
            tracing::error!("Shutdown blocked; tool admission is closed; engine restart required");
            ScratchError::new(
                hyper::StatusCode::SERVICE_UNAVAILABLE,
                "Shutdown blocked: background effects could not be confirmed stopped. Tool admission is closed; restart the engine".to_string(),
            )
        })?;
    let global_context = app.gcx.clone();
    global_context
        .ask_shutdown_sender
        .lock()
        .unwrap()
        .send(format!("going-down"))
        .map_err(|_| {
            tracing::error!(
                "Shutdown signal unavailable; tool admission is closed; engine restart required"
            );
            ScratchError::new(
                hyper::StatusCode::SERVICE_UNAVAILABLE,
                "Shutdown signal unavailable. Tool admission is closed; restart the engine"
                    .to_string(),
            )
        })?;
    Ok(Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(json!({"success": true}).to_string()))
        .unwrap())
}
