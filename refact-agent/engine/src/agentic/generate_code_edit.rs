pub use refact_agentic::generate_code_edit::remove_markdown_fences;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;
use crate::yaml_configs::customization_registry::get_subagent_config;
use std::sync::Arc;

const SUBAGENT_ID: &str = "code_edit";

pub async fn generate_code_edit(
    gcx: Arc<GlobalContext>,
    code: &str,
    instruction: &str,
    cursor_file: &str,
    cursor_line: i32,
) -> Result<String, String> {
    if code.is_empty() {
        return Err("The provided code is empty".to_string());
    }
    if instruction.is_empty() {
        return Err("The instruction is empty".to_string());
    }

    let subagent_config = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
        .await
        .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

    let system_prompt = subagent_config
        .messages
        .system_prompt
        .as_ref()
        .ok_or_else(|| {
            format!(
                "messages.system_prompt not defined for subagent '{}'",
                SUBAGENT_ID
            )
        })?;

    let user_message = format!(
        "File: {} (line {})\n\nCode to edit:\n```\n{}\n```\n\nInstruction: {}",
        cursor_file, cursor_line, code, instruction
    );

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(system_prompt.clone()),
            ..Default::default()
        },
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(user_message),
            ..Default::default()
        },
    ];

    let result = run_subchat_once(gcx, SUBAGENT_ID, messages)
        .await
        .map_err(|e| format!("Error generating code edit: {}", e))?;

    let edited_code = result
        .messages
        .last()
        .and_then(|msg| match &msg.content {
            ChatContent::SimpleText(text) => Some(text.clone()),
            _ => None,
        })
        .ok_or("No edited code was generated".to_string())?;

    Ok(remove_markdown_fences(&edited_code))
}
