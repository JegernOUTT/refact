use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

const CODE_EDIT_SYSTEM_PROMPT: &str = r#"You are a code editing assistant. Your task is to modify the provided code according to the user's instruction.

# Rules
1. Return ONLY the edited code - no explanations, no markdown fences, no commentary
2. Preserve the original indentation style and formatting conventions
3. Make minimal changes necessary to fulfill the instruction
4. If the instruction is unclear, make the most reasonable interpretation
5. Keep all code that isn't directly related to the instruction unchanged

# Output Format
Return the edited code directly, without any wrapping or explanation. The output should be valid code that can directly replace the input."#;

fn remove_markdown_fences(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("```") {
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() >= 2 {
            // Find closing fence
            if let Some(end_idx) = lines.iter().rposition(|l| l.trim() == "```") {
                if end_idx > 0 {
                    // Skip first line (```language) and last line (```)
                    let start_idx = 1;
                    if start_idx < end_idx {
                        return lines[start_idx..end_idx].join("\n");
                    }
                }
            }
        }
    }
    text.to_string()
}

pub async fn generate_code_edit(
    gcx: Arc<ARwLock<GlobalContext>>,
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

    let user_message = format!(
        "File: {} (line {})\n\nCode to edit:\n```\n{}\n```\n\nInstruction: {}",
        cursor_file, cursor_line, code, instruction
    );

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(CODE_EDIT_SYSTEM_PROMPT.to_string()),
            ..Default::default()
        },
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(user_message),
            ..Default::default()
        },
    ];

    let result = run_subchat_once(gcx, "code_edit", messages)
        .await
        .map_err(|e| format!("Error generating code edit: {}", e))?;

    let edited_code = result.messages
        .last()
        .and_then(|msg| match &msg.content {
            ChatContent::SimpleText(text) => Some(text.clone()),
            _ => None,
        })
        .ok_or("No edited code was generated".to_string())?;

    Ok(remove_markdown_fences(&edited_code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_markdown_fences_with_language() {
        let input = "```python\ndef hello():\n    print('world')\n```";
        assert_eq!(
            remove_markdown_fences(input),
            "def hello():\n    print('world')"
        );
    }

    #[test]
    fn test_remove_markdown_fences_without_language() {
        let input = "```\nsome code\n```";
        assert_eq!(remove_markdown_fences(input), "some code");
    }

    #[test]
    fn test_remove_markdown_fences_no_fences() {
        let input = "plain code without fences";
        assert_eq!(remove_markdown_fences(input), "plain code without fences");
    }

    #[test]
    fn test_remove_markdown_fences_with_whitespace() {
        let input = "  ```rust\nfn main() {}\n```  ";
        assert_eq!(remove_markdown_fences(input), "fn main() {}");
    }
}
