use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::{ChatMessage, ChatMeta, validate_mode_for_request};
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::indexing_utils::wait_for_indexing_if_needed;
use crate::scratchpads::chat_utils_prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use crate::tools::tools_list::get_tools_for_mode;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PrependSystemPromptPost {
    pub messages: Vec<ChatMessage>,
    pub chat_meta: ChatMeta,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PrependSystemPromptResponse {
    pub messages: Vec<ChatMessage>,
    pub messages_to_stream_back: Vec<serde_json::Value>,
}

pub async fn handle_v1_prepend_system_prompt_and_maybe_more_initial_messages(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    wait_for_indexing_if_needed(gcx.clone()).await;

    let post = serde_json::from_slice::<PrependSystemPromptPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;
    let mut has_rag_results = HasRagResults::new();

    let mode_id = validate_mode_for_request(gcx.clone(), &post.chat_meta.chat_mode)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Invalid chat mode: {}", e),
            )
        })?;

    let (messages, _) = prepend_the_right_system_prompt_and_maybe_more_initial_messages(
        gcx.clone(),
        post.messages,
        &post.chat_meta,
        &None,
        &mut has_rag_results,
        get_tools_for_mode(gcx.clone(), &mode_id, None)
            .await
            .into_iter()
            .map(|t| t.tool_description().name)
            .collect(),
        &mode_id,
        "",
    )
    .await;
    let messages_to_stream_back = has_rag_results.in_json;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(
            serde_json::to_string(&PrependSystemPromptResponse {
                messages,
                messages_to_stream_back,
            })
            .unwrap(),
        ))
        .unwrap())
}
