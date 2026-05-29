use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage, ChatMeta, validate_mode_for_request};
use crate::chat::prepare::build_canonical_openai_tools;
use crate::chat::trajectories::{
    ensure_frozen_prefix, new_frozen_request_prefix, persist_frozen_prefix,
};
use crate::custom_error::ScratchError;
use crate::indexing_utils::wait_for_indexing_if_needed;
use crate::scratchpads::chat_utils_prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use crate::tools::tools_list::{apply_mcp_lazy_filter, get_tools_for_mode};

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
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
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

    let mode_tools = apply_mcp_lazy_filter(get_tools_for_mode(gcx.clone(), &mode_id, None).await);
    let tool_descs: Vec<_> = mode_tools
        .tools
        .into_iter()
        .map(|tool| tool.tool_description())
        .collect();
    let prompt_tool_names = tool_descs.iter().map(|t| t.name.clone()).collect();

    let (messages, _) = prepend_the_right_system_prompt_and_maybe_more_initial_messages(
        crate::app_state::AppState::from_gcx(gcx.clone()).await,
        post.messages,
        &post.chat_meta,
        &None,
        &mut has_rag_results,
        prompt_tool_names,
        &mode_id,
        "",
    )
    .await;

    let system_prompt = messages.iter().find_map(|message| {
        if message.role == "system" {
            match &message.content {
                ChatContent::SimpleText(text) => Some(text.clone()),
                _ => None,
            }
        } else {
            None
        }
    });
    let canonical_tools = build_canonical_openai_tools(gcx.clone(), &tool_descs, false, true).await;
    let frozen_prefix = new_frozen_request_prefix(
        system_prompt,
        Some(serde_json::Value::Array(canonical_tools.tools)),
    );
    let session_arc = {
        let sessions = app.gcx.chat_sessions.read().await;
        sessions.get(&post.chat_meta.chat_id).cloned()
    };
    if let Some(session_arc) = session_arc {
        let mut session = session_arc.lock().await;
        ensure_frozen_prefix(
            &mut session,
            frozen_prefix.system_prompt.clone(),
            frozen_prefix.tools_canonical.clone(),
        );
    }
    if let Err(error) =
        persist_frozen_prefix(gcx.clone(), &post.chat_meta.chat_id, frozen_prefix).await
    {
        tracing::warn!(
            "Failed to persist frozen request prefix for {}: {}",
            post.chat_meta.chat_id,
            error
        );
    }
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
