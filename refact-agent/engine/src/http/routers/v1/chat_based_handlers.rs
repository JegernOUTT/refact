use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;
use serde::Deserialize;
use crate::agentic::generate_commit_message::generate_commit_message_by_diff;
use crate::agentic::compress_trajectory::compress_trajectory;
use crate::call_validation::ChatMessage;

#[derive(Deserialize)]
struct CommitMessageFromDiffPost {
    diff: String,
    #[serde(default)]
    text: Option<String>, // a prompt for the commit message
}

pub async fn handle_v1_commit_message_from_diff(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let post = serde_json::from_slice::<CommitMessageFromDiffPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let commit_message =
        generate_commit_message_by_diff(global_context.clone(), &post.diff, &post.text)
            .await
            .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Body::from(commit_message))
        .unwrap())
}

#[derive(Deserialize)]
struct CompressTrajectoryPost {
    #[allow(dead_code)]
    project: String,
    messages: Vec<ChatMessage>,
}

pub async fn handle_v1_trajectory_compress(
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let global_context = app.gcx.clone();
    let post = serde_json::from_slice::<CompressTrajectoryPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let trajectory = compress_trajectory(global_context.clone(), &post.messages)
        .await
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;

    let response = serde_json::json!({
        "goal": "compress it",
        "trajectory": trajectory,
    });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}
