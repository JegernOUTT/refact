//! Common rendering helpers for supplemental context message roles.
//!
//! The message roles `context_file`, `plain_text`, and `cd_instruction` carry
//! content that must reach the model but that standard LLM APIs do not know
//! about.  Each wire adapter is responsible for folding this content into the
//! appropriate API primitives; the functions here produce the canonical text
//! representation so every adapter formats it the same way.

use refact_core::chat_types::{ChatContent, ChatMessage};

pub const PLAN_META_KEY: &str = "plan";
pub const PLAN_INLINED_IN_SYSTEM_PROMPT_KEY: &str = "inlined_in_system_prompt";

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

pub fn plan_inlined_in_system_prompt(msg: &ChatMessage) -> bool {
    msg.extra
        .get(PLAN_META_KEY)
        .and_then(|m| m.get(PLAN_INLINED_IN_SYSTEM_PROMPT_KEY))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
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

pub fn render_plan_system_blocks(messages: &[ChatMessage]) -> Vec<String> {
    let mut plans = Vec::new();
    for (position, msg) in messages.iter().enumerate() {
        if !is_plan_role(&msg.role) {
            continue;
        }
        let meta = msg.extra.get(PLAN_META_KEY);
        let mode = meta
            .and_then(|m| m.get("mode"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let Some(version) = meta.and_then(|m| m.get("version")).and_then(|v| v.as_u64()) else {
            continue;
        };
        plans.push(PlanBlock {
            mode,
            version,
            content: msg.content.content_text_only(),
            inlined_in_system_prompt: plan_inlined_in_system_prompt(msg),
            position,
        });
    }

    let Some((latest_index, latest)) = plans
        .iter()
        .enumerate()
        .max_by_key(|(_, plan)| (plan.version, plan.position))
    else {
        return Vec::new();
    };

    let mut distinct_versions = Vec::new();
    for plan in &plans {
        if !distinct_versions.contains(&plan.version) {
            distinct_versions.push(plan.version);
        }
    }

    let mut blocks = Vec::new();
    if distinct_versions.len() >= 2 {
        let mut older: Vec<_> = plans
            .iter()
            .enumerate()
            .filter(|(idx, plan)| *idx != latest_index && plan.version != latest.version)
            .map(|(_, plan)| plan)
            .collect();
        older.sort_by_key(|plan| (plan.version, plan.position));
        if !older.is_empty() {
            let bullets = older
                .into_iter()
                .map(|plan| {
                    format!(
                        "- v{}: {}",
                        plan.version,
                        escape_xml_text(&plan_history_snippet(&plan.content))
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            blocks.push(format!("<plan-history>\n{}\n</plan-history>", bullets));
        }
    }

    if !latest.inlined_in_system_prompt {
        blocks.push(format!(
            "<plan mode=\"{}\" version=\"{}\">\n{}\n</plan>",
            escape_xml_attr(&latest.mode),
            latest.version,
            render_plan_content(&latest.content)
        ));
    }
    blocks
}

pub fn append_plan_blocks(system_text: Option<String>, plan_blocks: Vec<String>) -> Option<String> {
    if plan_blocks.is_empty() {
        return system_text;
    }
    let plan_text = plan_blocks.join("\n\n");
    match system_text {
        Some(text) if !text.trim().is_empty() => Some(format!("{}\n\n{}", text, plan_text)),
        _ => Some(plan_text),
    }
}

fn render_plan_content(content: &str) -> String {
    if content.contains('<') || content.contains('>') {
        format!("<![CDATA[{}]]>", content.replace("]]>", "]]]]><![CDATA[>"))
    } else {
        escape_xml_text(content)
    }
}

fn plan_history_snippet(content: &str) -> String {
    content
        .chars()
        .take(80)
        .collect::<String>()
        .replace(['\r', '\n'], " ")
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

struct PlanBlock {
    mode: String,
    version: u64,
    content: String,
    inlined_in_system_prompt: bool,
    position: usize,
}
