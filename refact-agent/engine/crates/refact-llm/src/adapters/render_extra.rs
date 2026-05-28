//! Common rendering helpers for supplemental context message roles.
//!
//! The message roles `context_file`, `plain_text`, and `cd_instruction` carry
//! content that must reach the model but that standard LLM APIs do not know
//! about.  Each wire adapter is responsible for folding this content into the
//! appropriate API primitives; the functions here produce the canonical text
//! representation so every adapter formats it the same way.

use refact_core::chat_types::{ChatContent, ChatMessage};

pub const PLAN_META_KEY: &str = "plan";

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

pub fn is_event_role(role: &str) -> bool {
    role == "event"
}

pub fn is_plan_role(role: &str) -> bool {
    role == "plan"
}

pub fn render_event_message(msg: &ChatMessage) -> String {
    let meta = msg.extra.get("event");
    let subkind = meta
        .and_then(|m| m.get("subkind"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let source = meta
        .and_then(|m| m.get("source"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let payload = meta
        .and_then(|m| m.get("payload"))
        .unwrap_or(&serde_json::Value::Null);
    let payload_json = serde_json::to_string(payload).unwrap_or_else(|_| "null".to_string());
    let content = msg.content.content_text_only();
    format!(
        "<event subkind=\"{}\" source=\"{}\">\n<payload>{}</payload>\n<message>{}</message>\n</event>",
        escape_xml_attr(subkind),
        escape_xml_attr(source),
        escape_xml_text(&payload_json),
        escape_xml_text(&content)
    )
}

pub fn render_plan_message(msg: &ChatMessage) -> Option<String> {
    if !is_plan_role(&msg.role) {
        return None;
    }
    let meta = msg.extra.get(PLAN_META_KEY);
    let mode = meta
        .and_then(|m| m.get("mode"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let version = meta
        .and_then(|m| m.get("version"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    Some(format!(
        "<plan mode=\"{}\" version=\"{}\">\n{}\n</plan>",
        escape_xml_attr(mode),
        version,
        render_plan_content(&msg.content.content_text_only())
    ))
}

fn render_plan_content(content: &str) -> String {
    if content.contains('<') || content.contains('>') {
        format!("<![CDATA[{}]]>", content.replace("]]>", "]]]]><![CDATA[>"))
    } else {
        escape_xml_text(content)
    }
}

fn escape_xml_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_attr(input: &str) -> String {
    escape_xml_text(input)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
