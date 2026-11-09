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
    crate::chat::close_all_chat_sessions(app.clone())
        .await
        .map_err(|_| {
            ScratchError::new(
                hyper::StatusCode::SERVICE_UNAVAILABLE,
                "Shutdown blocked: background effects could not be confirmed stopped".to_string(),
            )
        })?;
    let global_context = app.gcx.clone();
    global_context
        .ask_shutdown_sender
        .lock()
        .unwrap()
        .send(format!("going-down"))
        .map_err(|_| {
            ScratchError::new(
                hyper::StatusCode::SERVICE_UNAVAILABLE,
                "Shutdown signal unavailable".to_string(),
            )
        })?;
    Ok(Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(json!({"success": true}).to_string()))
        .unwrap())
}
