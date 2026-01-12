use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::Deserialize;
use tokio::sync::RwLock as ARwLock;
use crate::subchat::{run_subchat, run_subchat_once, resolve_subchat_config, resolve_subchat_params, resolve_subchat_model, WrapUpConfig};
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::call_validation::deserialize_messages_from_post;


#[derive(Deserialize)]
struct SubChatPost {
    messages: Vec<serde_json::Value>,
    wrap_up_depth: usize,
    wrap_up_tokens_cnt: usize,
    tools_turn_on: Vec<String>,
    wrap_up_prompt: String,
}

pub async fn handle_v1_subchat(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<SubChatPost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    let messages = deserialize_messages_from_post(&post.messages)?;

    let wrap_up = WrapUpConfig {
        depth: post.wrap_up_depth,
        tokens_cnt: post.wrap_up_tokens_cnt,
        prompt: post.wrap_up_prompt.clone(),
    };

    let config = resolve_subchat_config(
        global_context.clone(),
        "http_subchat",
        false,
        None,
        None,
        None,
        None,
        Some(post.tools_turn_on.clone()),
        post.wrap_up_depth.max(1),
        false,
        Some(wrap_up),
    ).await.map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let model_id = config.model.clone();

    let result = run_subchat(global_context.clone(), messages, config)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e)))?;

    let new_messages = vec![result.messages.iter()
        .map(|msg| msg.into_value(&None, &model_id))
        .collect::<Vec<_>>()];
    let resp_serialised = serde_json::to_string_pretty(&new_messages)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("JSON serialization error: {}", e)))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(resp_serialised))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))
}

#[derive(Deserialize)]
struct SubChatSinglePost {
    messages: Vec<serde_json::Value>,
}

pub async fn handle_v1_subchat_single(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> axum::response::Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<SubChatSinglePost>(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("JSON problem: {}", e)))?;
    let messages = deserialize_messages_from_post(&post.messages)?;

    let params = resolve_subchat_params(global_context.clone(), "http_subchat_single")
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let model_id = resolve_subchat_model(global_context.clone(), &params)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let result = run_subchat_once(global_context.clone(), "http_subchat_single", messages)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", e)))?;

    let new_messages = vec![result.messages.iter()
        .map(|msg| msg.into_value(&None, &model_id))
        .collect::<Vec<_>>()];
    let resp_serialised = serde_json::to_string_pretty(&new_messages)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("JSON serialization error: {}", e)))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(resp_serialised))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Response build error: {}", e)))
}
