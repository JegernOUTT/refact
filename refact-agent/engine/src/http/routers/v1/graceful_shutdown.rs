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
        .send("HTTP /v1/graceful-shutdown".to_string())
        .map_err(|_| {
            tracing::error!(
                "Shutdown request could not be delivered; tool admission was not changed"
            );
            ScratchError::new(
                hyper::StatusCode::SERVICE_UNAVAILABLE,
                "Shutdown request unavailable; the engine remains active".to_string(),
            )
        })?;
    Ok(Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({"success": true, "status": "requested", "completed": false}).to_string(),
        ))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[tokio::test]
    async fn failed_shutdown_request_does_not_close_tool_admission() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let error = handle_v1_graceful_shutdown(State(app.clone()), hyper::body::Bytes::new())
            .await
            .unwrap_err();
        assert_eq!(error.status_code, hyper::StatusCode::SERVICE_UNAVAILABLE);
        assert!(!app.runtime.tool_jobs.scopes.is_shutdown());
        app.runtime
            .tool_jobs
            .scopes
            .acquire(
                &[app.runtime.tool_jobs.chat_scope_key("still-active")],
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap()
            .settle();
    }

    #[tokio::test]
    async fn delivered_request_is_not_a_completed_shutdown_or_a_scope_latch() {
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        let (sender, receiver) = std::sync::mpsc::channel();
        Arc::get_mut(&mut gcx).unwrap().ask_shutdown_sender =
            Arc::new(std::sync::Mutex::new(sender));
        let app = AppState::from_gcx(gcx).await;
        let response = handle_v1_graceful_shutdown(State(app.clone()), hyper::body::Bytes::new())
            .await
            .unwrap();
        assert_eq!(receiver.try_recv().unwrap(), "HTTP /v1/graceful-shutdown");
        assert!(!app.runtime.tool_jobs.scopes.is_shutdown());
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["status"], "requested");
        assert_eq!(value["completed"], false);
    }
}
