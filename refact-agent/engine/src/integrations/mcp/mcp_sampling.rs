use std::sync::Weak;
use tokio::sync::RwLock as ARwLock;
use rmcp::model::{
    CreateMessageRequestParams, CreateMessageResult, Role, SamplingMessage, SamplingContent,
    SamplingMessageContent,
};
use rmcp::ErrorData as McpError;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;

fn content_to_text(c: &SamplingMessageContent) -> String {
    match c {
        SamplingMessageContent::Text(t) => t.text.clone(),
        SamplingMessageContent::Image(_) => "[image content not supported]".to_string(),
        SamplingMessageContent::Audio(_) => "[audio content not supported]".to_string(),
        SamplingMessageContent::ToolResult(_) | SamplingMessageContent::ToolUse(_) => {
            "[tool content not supported]".to_string()
        }
    }
}

fn sampling_message_to_chat_message(msg: &SamplingMessage) -> ChatMessage {
    let text = match &msg.content {
        SamplingContent::Single(c) => content_to_text(c),
        SamplingContent::Multiple(cs) => cs
            .iter()
            .map(content_to_text)
            .collect::<Vec<_>>()
            .join("\n"),
    };
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    ChatMessage {
        role: role.to_string(),
        content: ChatContent::SimpleText(text),
        ..Default::default()
    }
}

pub async fn mcp_sampling_create_message(
    gcx_weak: Weak<ARwLock<GlobalContext>>,
    params: CreateMessageRequestParams,
    debug_name: &str,
) -> Result<CreateMessageResult, McpError> {
    let gcx = gcx_weak
        .upgrade()
        .ok_or_else(|| McpError::internal_error("Refact agent is shutting down", None))?;

    tracing::info!(
        "MCP sampling request from {}: {} messages, max_tokens={}",
        debug_name,
        params.messages.len(),
        params.max_tokens
    );

    let mut messages: Vec<ChatMessage> = params
        .messages
        .iter()
        .map(sampling_message_to_chat_message)
        .collect();

    if let Some(system_prompt) = &params.system_prompt {
        messages.insert(
            0,
            ChatMessage {
                role: "system".to_string(),
                content: ChatContent::SimpleText(system_prompt.clone()),
                ..Default::default()
            },
        );
    }

    let result = run_subchat_once(gcx, "mcp_sampling", messages)
        .await
        .map_err(|e| {
            tracing::warn!("MCP sampling subchat failed for {}: {}", debug_name, e);
            McpError::internal_error(
                "Sampling subchat failed",
                Some(serde_json::json!({"reason": e})),
            )
        })?;

    let last_assistant = result.messages.iter().rev().find(|m| m.role == "assistant");

    let response_text = last_assistant
        .map(|m| m.content.content_text_only())
        .unwrap_or_else(|| "No response generated.".to_string());

    tracing::info!(
        "MCP sampling response for {}: {} chars",
        debug_name,
        response_text.len()
    );

    let message = SamplingMessage::assistant_text(response_text);
    Ok(CreateMessageResult::new(message, "refact".to_string())
        .with_stop_reason(CreateMessageResult::STOP_REASON_END_TURN))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sampling_message_to_chat_message_user() {
        let msg = SamplingMessage::user_text("hello");
        let chat_msg = sampling_message_to_chat_message(&msg);
        assert_eq!(chat_msg.role, "user");
        assert_eq!(chat_msg.content.content_text_only(), "hello");
    }

    #[test]
    fn test_sampling_message_to_chat_message_assistant() {
        let msg = SamplingMessage::assistant_text("response");
        let chat_msg = sampling_message_to_chat_message(&msg);
        assert_eq!(chat_msg.role, "assistant");
        assert_eq!(chat_msg.content.content_text_only(), "response");
    }
}
