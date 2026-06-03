use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;
use crate::yaml_configs::customization_registry::get_subagent_config;
use std::sync::Arc;

const SUBAGENT_ID: &str = "compress_trajectory";

fn assistant_text_after_prompt(messages: &[ChatMessage], prompt_idx: usize) -> Option<String> {
    messages
        .iter()
        .skip(prompt_idx.saturating_add(1))
        .rev()
        .find_map(|message| {
            if message.role != "assistant" {
                return None;
            }

            let content = message.content.content_text_only().trim().to_string();
            if content.is_empty() {
                None
            } else {
                Some(content)
            }
        })
}

pub async fn compress_trajectory(
    gcx: Arc<GlobalContext>,
    messages: &Vec<ChatMessage>,
) -> Result<String, String> {
    if messages.is_empty() {
        return Err("The provided chat is empty".to_string());
    }
    let messages = messages.clone();
    let gcx2 = gcx.clone();
    crate::buddy::workflows::buddy_wrap_workflow(
        crate::app_state::AppState::from_gcx(gcx).await,
        "compress_trajectory",
        "🗜",
        10,
        |_: &String| "Trajectory compressed".to_string(),
        move || async move {
            let subagent_config = get_subagent_config(gcx2.clone(), SUBAGENT_ID, None)
                .await
                .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

            let compression_prompt =
                subagent_config
                    .messages
                    .user_template
                    .as_ref()
                    .ok_or_else(|| {
                        format!(
                            "messages.user_template not defined for subagent '{}'",
                            SUBAGENT_ID
                        )
                    })?;

            let mut messages_compress = messages.clone();
            messages_compress.push(ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(compression_prompt.clone()),
                ..Default::default()
            });
            let compression_prompt_idx = messages_compress.len() - 1;

            let result = run_subchat_once(gcx2, SUBAGENT_ID, messages_compress)
                .await
                .map_err(|e| format!("compress_trajectory subchat failed: {}", e))?;

            let content = assistant_text_after_prompt(&result.messages, compression_prompt_idx)
                .ok_or_else(|| "Trajectory compression produced empty result".to_string())?;

            Ok(content)
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn assistant_text_after_prompt_ignores_source_assistant() {
        let messages = vec![
            message("user", "source user"),
            message("assistant", "old assistant"),
            message("user", "compression prompt"),
        ];

        assert_eq!(assistant_text_after_prompt(&messages, 2), None);
    }

    #[test]
    fn assistant_text_after_prompt_selects_new_assistant() {
        let messages = vec![
            message("user", "source user"),
            message("assistant", "old assistant"),
            message("user", "compression prompt"),
            message("assistant", " compressed trajectory\n"),
        ];

        assert_eq!(
            assistant_text_after_prompt(&messages, 2),
            Some("compressed trajectory".to_string())
        );
    }

    #[test]
    fn assistant_text_after_prompt_ignores_blank_new_assistant() {
        let messages = vec![
            message("user", "source user"),
            message("assistant", "old assistant"),
            message("user", "compression prompt"),
            message("assistant", "   \n"),
        ];

        assert_eq!(assistant_text_after_prompt(&messages, 2), None);
    }
}
