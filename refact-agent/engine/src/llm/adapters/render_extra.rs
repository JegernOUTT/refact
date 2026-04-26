//! Common rendering helpers for supplemental context message roles.
//!
//! The message roles `context_file`, `plain_text`, and `cd_instruction` carry
//! content that must reach the model but that standard LLM APIs do not know
//! about.  Each wire adapter is responsible for folding this content into the
//! appropriate API primitives; the functions here produce the canonical text
//! representation so every adapter formats it the same way.

use crate::call_validation::{ChatContent, ChatMessage};

/// Returns `true` for message roles that carry supplemental context and must
/// be rendered into wire messages by each adapter rather than silently dropped.
pub fn is_context_role(role: &str) -> bool {
    matches!(role, "context_file" | "plain_text" | "cd_instruction")
}

/// Render `context_file` content with per-file filename + line-range headers.
///
/// Each file is formatted as:
/// ```text
/// 📄 path/to/file.py:10-50
/// <file content>
/// ```
/// Multiple files are separated by a blank line.
pub fn render_context_file_content(content: &ChatContent) -> String {
    match content {
        ChatContent::ContextFiles(files) => files
            .iter()
            .map(|f| {
                format!(
                    "📄 {}:{}-{}\n{}",
                    f.file_name, f.line1, f.line2, f.file_content
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => content.content_text_only(),
    }
}

/// Render any supplemental context message to plain text.
/// Returns `None` if the rendered text is empty or whitespace-only.
pub fn render_context_message(msg: &ChatMessage) -> Option<String> {
    let text = match msg.role.as_str() {
        "context_file" => render_context_file_content(&msg.content),
        "plain_text" | "cd_instruction" => msg.content.content_text_only(),
        _ => return None,
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Append `text` to the `"content"` field of a JSON tool message object,
/// separating existing content from the new text with two newlines.
///
/// Handles both string and array-of-blocks content:
/// - String → appends in-place
/// - Array  → extracts existing text, appends, writes back as string
/// - Other  → writes `text` as new string content
pub fn append_text_to_tool_json(msg: &mut serde_json::Value, text: &str) {
    let existing: String = match &msg["content"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    };
    msg["content"] = serde_json::json!(if existing.is_empty() {
        text.to_string()
    } else {
        format!("{}\n\n{}", existing, text)
    });
}
