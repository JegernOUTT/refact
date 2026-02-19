use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;
use crate::yaml_configs::customization_registry::get_subagent_config;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

const SUBAGENT_ID: &str = "compress_trajectory";

pub async fn compress_trajectory(
    gcx: Arc<ARwLock<GlobalContext>>,
    messages: &Vec<ChatMessage>,
) -> Result<String, String> {
    if messages.is_empty() {
        return Err("The provided chat is empty".to_string());
    }

    let subagent_config = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
        .await
        .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

    let compression_prompt = subagent_config.messages.user_template
        .as_ref()
        .ok_or_else(|| format!("messages.user_template not defined for subagent '{}'", SUBAGENT_ID))?;

    let mut messages_compress = messages.clone();
    messages_compress.push(ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText(compression_prompt.clone()),
        ..Default::default()
    });

    let result = run_subchat_once(gcx, SUBAGENT_ID, messages_compress)
        .await
        .map_err(|e| format!("Error: {}", e))?;

    let content = result
        .messages
        .last()
        .and_then(|last_m| match &last_m.content {
            ChatContent::SimpleText(text) => Some(text.clone()),
            _ => None,
        })
        .ok_or("No traj message was generated".to_string())?;

    Ok(content)
}
