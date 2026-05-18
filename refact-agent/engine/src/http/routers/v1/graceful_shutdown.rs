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
    let global_context = app.gcx.clone();
    global_context
        .ask_shutdown_sender
        .lock()
        .unwrap()
        .send(format!("going-down"))
        .unwrap();
    Ok(Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(json!({"success": true}).to_string()))
        .unwrap())
}
