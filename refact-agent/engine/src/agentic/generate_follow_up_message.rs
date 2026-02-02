use std::sync::Arc;
use serde::Deserialize;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::MapErrToString;
use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::json_utils;
use crate::yaml_configs::customization_registry::get_subagent_config;

const SUBAGENT_ID: &str = "follow_up";

#[derive(Deserialize, Clone)]
pub struct FollowUpResponse {
    pub follow_ups: Vec<String>,
    pub topic_changed: bool,
}

fn _make_conversation(messages: &Vec<ChatMessage>, system_prompt: &str) -> Vec<ChatMessage> {
    let mut history_message = "*Conversation:*\n".to_string();
    for m in messages.iter().rev().take(2) {
        let content = m.content.to_text_with_image_placeholders();
        let char_count = content.chars().count();
        let limited_content = if char_count > 5000 {
            let skip_count = char_count - 5000;
            format!(
                "...{}",
                content.chars().skip(skip_count).collect::<String>()
            )
        } else {
            content
        };
        let message_row = match m.role.as_str() {
            "user" => format!("👤:{}\n\n", limited_content),
            "assistant" => format!("🤖:{}\n\n", limited_content),
            _ => continue,
        };
        history_message.insert_str(0, &message_row);
    }
    vec![
        ChatMessage::new("system".to_string(), system_prompt.to_string()),
        ChatMessage::new("user".to_string(), history_message),
    ]
}

pub async fn generate_follow_up_message(
    messages: Vec<ChatMessage>,
    gcx: Arc<ARwLock<GlobalContext>>,
    _model_id: &str,
    _chat_id: &str,
) -> Result<FollowUpResponse, String> {
    let subagent_config = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
        .await
        .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

    let system_prompt = subagent_config.messages.system_prompt
        .as_ref()
        .ok_or_else(|| format!("messages.system_prompt not defined for subagent '{}'", SUBAGENT_ID))?;

    let result = run_subchat_once(gcx, SUBAGENT_ID, _make_conversation(&messages, system_prompt)).await?;

    let response = result
        .messages
        .last()
        .and_then(|last_m| match &last_m.content {
            ChatContent::SimpleText(text) => Some(text.clone()),
            _ => None,
        })
        .ok_or("No follow-up message was generated".to_string())?;

    tracing::info!("follow-up model says {:?}", response);

    let response: FollowUpResponse =
        json_utils::extract_json_object(&response).map_err_with_prefix("Failed to parse json:")?;
    Ok(response)
}
