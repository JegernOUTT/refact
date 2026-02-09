use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::{ChatContent, ChatMessage, ContextFile};
use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;
use crate::yaml_configs::customization_registry::get_subagent_config;

const SUBAGENT_ID: &str = "mode_transition";
const MAX_FILE_SIZE: usize = 1024 * 1024; // 1MB max file size

lazy_static! {
    static ref MEMORY_PATH_REGEX: Regex = Regex::new(
        r"(?:^|[\s\n])(/[^\s]+\.refact/(?:knowledge|trajectories|tasks/[^/]+/memories)/[^\s\n,)]+\.(?:md|json))"
    ).expect("Invalid memory path regex");

    static ref FILE_PATH_REGEX: Regex = Regex::new(
        r"(?m)^\s*(?:File|Path):\s*(\S+)"
    ).expect("Invalid file path regex");

    static ref DIFF_GIT_REGEX: Regex = Regex::new(
        r"(?m)^(?:diff --git [ab]/(\S+)|[+]{3} [ab]/(\S+))"
    ).expect("Invalid diff git regex");
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReference {
    pub path: String,
    pub source: String,
    pub msg_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct ConversationMetadata {
    pub annotated_messages: Vec<(String, ChatMessage)>,
    pub context_files: Vec<FileReference>,
    pub edited_files: Vec<FileReference>,
    pub memory_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParsedDecisions {
    pub summary: String,
    pub files_to_open: Vec<String>,
    pub messages_to_preserve: Vec<String>,
    pub memories_to_include: Vec<String>,
    pub tool_outputs_to_include: Vec<String>,
    pub pending_tasks: Vec<String>,
    pub handoff_message: String,
}



pub fn extract_conversation_metadata(messages: &[ChatMessage]) -> ConversationMetadata {
    let mut metadata = ConversationMetadata::default();
    let mut seen_files: HashSet<String> = HashSet::new();
    let mut seen_memories: HashSet<String> = HashSet::new();

    for (idx, msg) in messages.iter().enumerate() {
        let msg_id = format!("MSG_ID:{}", idx);
        metadata.annotated_messages.push((msg_id.clone(), msg.clone()));

        if msg.role == "context_file" {
            match &msg.content {
                ChatContent::ContextFiles(files) => {
                    for file in files {
                        if seen_files.insert(file.file_name.clone()) {
                            metadata.context_files.push(FileReference {
                                path: file.file_name.clone(),
                                source: "context_file".to_string(),
                                msg_id: msg_id.clone(),
                            });
                        }
                    }
                }
                ChatContent::SimpleText(text) => {
                    if let Ok(files) = serde_json::from_str::<Vec<ContextFile>>(text) {
                        for file in files {
                            if seen_files.insert(file.file_name.clone()) {
                                metadata.context_files.push(FileReference {
                                    path: file.file_name.clone(),
                                    source: "context_file".to_string(),
                                    msg_id: msg_id.clone(),
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if msg.role == "diff" || (msg.role == "tool" && is_diff_content(&msg.content)) {
            if let ChatContent::SimpleText(text) = &msg.content {
                for cap in FILE_PATH_REGEX.captures_iter(text) {
                    if let Some(path) = cap.get(1) {
                        let path_str = clean_path_string(path.as_str());
                        if !path_str.is_empty() && seen_files.insert(path_str.clone()) {
                            metadata.edited_files.push(FileReference {
                                path: path_str,
                                source: "diff".to_string(),
                                msg_id: msg_id.clone(),
                            });
                        }
                    }
                }
                for cap in DIFF_GIT_REGEX.captures_iter(text) {
                    let path_str = cap.get(1).or_else(|| cap.get(2))
                        .map(|m| clean_path_string(m.as_str()))
                        .unwrap_or_default();
                    if !path_str.is_empty() && seen_files.insert(path_str.clone()) {
                        metadata.edited_files.push(FileReference {
                            path: path_str,
                            source: "diff".to_string(),
                            msg_id: msg_id.clone(),
                        });
                    }
                }
            }
        }

        if msg.role == "tool" {
            if let ChatContent::SimpleText(text) = &msg.content {
                for cap in MEMORY_PATH_REGEX.captures_iter(text) {
                    if let Some(path) = cap.get(1) {
                        let path_str = clean_path_string(path.as_str());
                        if !path_str.is_empty() && seen_memories.insert(path_str.clone()) {
                            metadata.memory_paths.push(path_str);
                        }
                    }
                }
            }
        }
    }

    metadata
}

fn clean_path_string(s: &str) -> String {
    s.trim_end_matches(|c| c == ')' || c == ',' || c == ';' || c == ':' || c == '"' || c == '\'')
        .to_string()
}

fn is_diff_content(content: &ChatContent) -> bool {
    match content {
        ChatContent::SimpleText(text) => {
            text.contains("+++") && text.contains("---") ||
            text.contains("@@ ") ||
            text.starts_with("diff ")
        }
        _ => false,
    }
}

fn parse_xml_tag(content: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);

    let start = content.find(&open)?;
    let after_open = start + open.len();
    let end = content[after_open..].find(&close)? + after_open;

    if end > after_open {
        Some(content[after_open..end].trim().to_string())
    } else {
        None
    }
}

fn normalize_list_item(item: &str) -> String {
    let mut s = item.trim();
    if s.starts_with('-') || s.starts_with('*') || s.starts_with('+') {
        s = s[1..].trim_start();
    } else if let Some(rest) = s.strip_prefix(|c: char| c.is_ascii_digit()) {
        let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        if let Some(after) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')')) {
            s = after.trim_start();
        }
    }
    let s = s.trim_matches('`').trim_matches('"').trim_matches('\'').trim();
    s.to_string()
}

fn parse_list_tag(content: &str, tag: &str) -> Vec<String> {
    parse_xml_tag(content, tag)
        .map(|s| {
            s.lines()
                .map(|l| normalize_list_item(l))
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_llm_response(response: &str) -> ParsedDecisions {
    ParsedDecisions {
        summary: parse_xml_tag(response, "summary").unwrap_or_default(),
        files_to_open: parse_list_tag(response, "files_to_open"),
        messages_to_preserve: parse_list_tag(response, "messages_to_preserve"),
        memories_to_include: parse_list_tag(response, "memories_to_include"),
        tool_outputs_to_include: parse_list_tag(response, "tool_outputs_to_include"),
        pending_tasks: parse_list_tag(response, "pending_tasks"),
        handoff_message: parse_xml_tag(response, "handoff_message").unwrap_or_default(),
    }
}

fn format_annotated_messages(metadata: &ConversationMetadata) -> String {
    let mut result = String::new();

    for (msg_id, msg) in &metadata.annotated_messages {
        let role = &msg.role;
        let content_preview = match &msg.content {
            ChatContent::SimpleText(text) => {
                truncate_utf8(text, 500)
            }
            ChatContent::ContextFiles(files) => {
                format!("[Context files: {}]", files.iter().map(|f| f.file_name.as_str()).collect::<Vec<_>>().join(", "))
            }
            ChatContent::Multimodal(elements) => {
                let text_parts: Vec<String> = elements.iter()
                    .filter(|el| el.is_text())
                    .map(|el| truncate_utf8(&el.m_content, 200))
                    .collect();
                let image_count = elements.iter().filter(|el| el.is_image()).count();
                let text_preview = if text_parts.is_empty() {
                    String::new()
                } else {
                    text_parts.join(" ")
                };
                if image_count > 0 {
                    format!("{} [contains {} image(s)]", text_preview, image_count)
                } else {
                    text_preview
                }
            }
        };

        let tool_info = if let Some(tool_calls) = &msg.tool_calls {
            if !tool_calls.is_empty() {
                let tools: Vec<String> = tool_calls.iter()
                    .map(|tc| format!("{}({})", tc.function.name, truncate_utf8(&tc.function.arguments, 100)))
                    .collect();
                format!("\n[tool_calls: {}]", tools.join(", "))
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        result.push_str(&format!("[{}] [{}]\n{}{}\n\n", msg_id, role, content_preview, tool_info));
    }

    result
}

fn format_file_list(metadata: &ConversationMetadata) -> String {
    let mut lines = Vec::new();

    for file_ref in &metadata.context_files {
        lines.push(format!("- {} (from {}, {})", file_ref.path, file_ref.source, file_ref.msg_id));
    }

    for file_ref in &metadata.edited_files {
        lines.push(format!("- {} (edited, from {}, {})", file_ref.path, file_ref.source, file_ref.msg_id));
    }

    if lines.is_empty() {
        "No files found in conversation".to_string()
    } else {
        lines.join("\n")
    }
}

fn format_memory_list(metadata: &ConversationMetadata) -> String {
    if metadata.memory_paths.is_empty() {
        "No memory/knowledge files found".to_string()
    } else {
        metadata.memory_paths.iter()
            .map(|p| format!("- {}", p))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn truncate_utf8(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}

pub async fn analyze_mode_transition(
    gcx: Arc<ARwLock<GlobalContext>>,
    messages: &[ChatMessage],
    target_mode: &str,
    target_mode_description: &str,
) -> Result<ParsedDecisions, String> {
    if messages.is_empty() {
        return Err("The provided chat is empty".to_string());
    }

    let subagent_config = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
        .await
        .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

    let user_template = subagent_config.messages.user_template
        .as_ref()
        .ok_or_else(|| format!("messages.user_template not defined for subagent '{}'", SUBAGENT_ID))?;

    let metadata = extract_conversation_metadata(messages);

    let annotated_message_list = format_annotated_messages(&metadata);
    let file_list = format_file_list(&metadata);
    let memory_list = format_memory_list(&metadata);

    let user_prompt = user_template
        .replace("{target_mode}", target_mode)
        .replace("{target_mode_description}", target_mode_description)
        .replace("{annotated_message_list}", &annotated_message_list)
        .replace("{file_list}", &file_list)
        .replace("{memory_list}", &memory_list);

    let analysis_messages = vec![
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(user_prompt),
            ..Default::default()
        },
    ];

    let result = run_subchat_once(gcx, SUBAGENT_ID, analysis_messages)
        .await
        .map_err(|e| format!("Error analyzing mode transition: {}", e))?;

    let response_text = result
        .messages
        .last()
        .and_then(|msg| match &msg.content {
            ChatContent::SimpleText(text) => Some(text.clone()),
            _ => None,
        })
        .ok_or("No analysis response was generated".to_string())?;

    Ok(parse_llm_response(&response_text))
}

fn find_task_done_report(messages: &[ChatMessage]) -> Option<String> {
    let mut task_done_call_id: Option<String> = None;
    for msg in messages.iter().rev() {
        if msg.role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                for tc in tool_calls {
                    if tc.function.name == "task_done" {
                        task_done_call_id = Some(tc.id.clone());
                        break;
                    }
                }
            }
            if task_done_call_id.is_some() {
                break;
            }
        }
    }

    let call_id = task_done_call_id?;

    for msg in messages.iter().rev() {
        if msg.role == "tool" && msg.tool_call_id == call_id {
            if let ChatContent::SimpleText(text) = &msg.content {
                if let Ok(obj) = serde_json::from_str::<serde_json::Value>(text) {
                    let summary = obj.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                    let report = obj.get("report").and_then(|v| v.as_str()).unwrap_or("");
                    let files_changed: Vec<&str> = obj.get("files_changed")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                        .unwrap_or_default();

                    let mut result = String::new();
                    if !summary.is_empty() {
                        result.push_str(&format!("**{}**\n\n", summary));
                    }
                    if !report.is_empty() {
                        result.push_str(report);
                    }
                    if !files_changed.is_empty() {
                        result.push_str("\n\n**Files changed:**\n");
                        for f in &files_changed {
                            result.push_str(&format!("- `{}`\n", f));
                        }
                    }
                    if !result.is_empty() {
                        return Some(result);
                    }
                }
            }
        }
    }

    None
}

fn resolve_tool_name_for_output(metadata: &ConversationMetadata, tool_call_id: &str) -> String {
    if tool_call_id.is_empty() {
        return "tool".to_string();
    }
    for (_, msg) in &metadata.annotated_messages {
        if msg.role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                for tc in tool_calls {
                    if tc.id == tool_call_id {
                        return tc.function.name.clone();
                    }
                }
            }
        }
    }
    "tool".to_string()
}

fn format_conversation_entry(msg: &ChatMessage, metadata: &ConversationMetadata) -> String {
    match msg.role.as_str() {
        "user" => {
            let text = extract_text_content(&msg.content);
            if text.trim().is_empty() {
                return String::new();
            }
            format!("### 👤 User\n\n{}", text.trim())
        }
        "assistant" => {
            let text = extract_text_content(&msg.content);
            let tool_calls_md = if let Some(tool_calls) = &msg.tool_calls {
                if !tool_calls.is_empty() {
                    let calls: Vec<String> = tool_calls.iter()
                        .map(|tc| {
                            let args_preview = truncate_utf8(&tc.function.arguments, 120);
                            format!("- `{}({})`", tc.function.name, args_preview)
                        })
                        .collect();
                    format!("\n\n**Tool calls:**\n{}", calls.join("\n"))
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            if text.trim().is_empty() && tool_calls_md.is_empty() {
                return String::new();
            }
            let mut result = "### 🤖 Assistant\n\n".to_string();
            if !text.trim().is_empty() {
                result.push_str(text.trim());
            }
            result.push_str(&tool_calls_md);
            result
        }
        "tool" => {
            let text = extract_text_content(&msg.content);
            if text.trim().is_empty() {
                return String::new();
            }
            let tool_name = resolve_tool_name_for_output(metadata, &msg.tool_call_id);
            let truncated = truncate_utf8(text.trim(), 10000);
            format!("### 🔧 Tool: `{}`\n\n```\n{}\n```", tool_name, truncated)
        }
        "system" => {
            let text = extract_text_content(&msg.content);
            if text.trim().is_empty() {
                return String::new();
            }
            format!("### ⚙️ System\n\n{}", text.trim())
        }
        _ => String::new(),
    }
}

fn extract_text_content(content: &ChatContent) -> String {
    match content {
        ChatContent::SimpleText(text) => text.clone(),
        ChatContent::Multimodal(elements) => {
            elements.iter()
                .filter_map(|el| {
                    if el.is_text() { Some(el.m_content.clone()) } else { None }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        ChatContent::ContextFiles(_) => String::new(),
    }
}

pub async fn assemble_new_chat(
    gcx: Arc<ARwLock<GlobalContext>>,
    original_messages: &[ChatMessage],
    decisions: &ParsedDecisions,
) -> Result<Vec<ChatMessage>, String> {
    let metadata = extract_conversation_metadata(original_messages);
    let mut new_messages: Vec<ChatMessage> = Vec::new();
    let workspace_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;

    let allowed_files: HashSet<&str> = metadata.context_files.iter()
        .map(|f| f.path.as_str())
        .chain(metadata.edited_files.iter().map(|f| f.path.as_str()))
        .collect();
    let allowed_memories: HashSet<&str> = metadata.memory_paths.iter()
        .map(|s| s.as_str())
        .collect();

    // 1. All files batched in a single context_file message
    let mut file_contents: Vec<ContextFile> = Vec::new();
    for path in &decisions.files_to_open {
        if !allowed_files.contains(path.as_str()) {
            tracing::warn!("Skipping file {} - not in conversation allowlist", path);
            continue;
        }
        match read_file_content_safe(gcx.clone(), path, &workspace_dirs).await {
            Ok(content) => {
                file_contents.push(ContextFile {
                    file_name: path.clone(),
                    file_content: content.clone(),
                    line1: 1,
                    line2: content.lines().count(),
                    ..Default::default()
                });
            }
            Err(e) => {
                tracing::warn!("Failed to read file {}: {}", path, e);
            }
        }
    }
    if !file_contents.is_empty() {
        new_messages.push(ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(file_contents),
            ..Default::default()
        });
    }

    // 2. All memories batched in a single context_file message
    let mut memory_contents: Vec<ContextFile> = Vec::new();
    for memory_path in &decisions.memories_to_include {
        if !allowed_memories.contains(memory_path.as_str()) {
            tracing::warn!("Skipping memory {} - not in conversation allowlist", memory_path);
            continue;
        }
        match read_file_content_safe(gcx.clone(), memory_path, &workspace_dirs).await {
            Ok(content) => {
                memory_contents.push(ContextFile {
                    file_name: memory_path.clone(),
                    file_content: content.clone(),
                    line1: 1,
                    line2: content.lines().count(),
                    ..Default::default()
                });
            }
            Err(e) => {
                tracing::warn!("Failed to read memory {}: {}", memory_path, e);
            }
        }
    }
    if !memory_contents.is_empty() {
        new_messages.push(ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(memory_contents),
            ..Default::default()
        });
    }

    // 3. "Previous Conversation" message: preserved messages + tool outputs interleaved in order, summary at end
    let mut preserved_indices: HashSet<usize> = decisions.messages_to_preserve
        .iter()
        .filter_map(|msg_id_ref| {
            let id = msg_id_ref.trim_start_matches("MSG_ID:");
            id.parse::<usize>().ok()
        })
        .collect();
    let tool_output_indices: HashSet<usize> = decisions.tool_outputs_to_include
        .iter()
        .filter_map(|msg_id_ref| {
            let id = msg_id_ref.trim_start_matches("MSG_ID:");
            id.parse::<usize>().ok()
        })
        .collect();
    preserved_indices.extend(&tool_output_indices);

    let mut all_indices: Vec<usize> = preserved_indices.into_iter().collect();
    all_indices.sort();
    all_indices.dedup();

    let mut conversation_parts: Vec<String> = Vec::new();
    let mut preserved_images: Vec<crate::scratchpads::multimodality::MultimodalElement> = Vec::new();
    for idx in &all_indices {
        if let Some((_, msg)) = metadata.annotated_messages.get(*idx) {
            let formatted = format_conversation_entry(msg, &metadata);
            if !formatted.is_empty() {
                conversation_parts.push(formatted);
            }
            if let ChatContent::Multimodal(elements) = &msg.content {
                for el in elements {
                    if el.is_image() {
                        preserved_images.push(el.clone());
                    }
                }
            }
        }
    }

    let has_conversation = !conversation_parts.is_empty() || !decisions.summary.is_empty();
    if has_conversation {
        let mut conversation_text = String::new();
        if !conversation_parts.is_empty() {
            conversation_text.push_str("## Previous Conversation\n\n");
            conversation_text.push_str(&conversation_parts.join("\n\n---\n\n"));
        }
        if !decisions.summary.is_empty() {
            if !conversation_text.is_empty() {
                conversation_text.push_str("\n\n---\n\n");
            }
            conversation_text.push_str(&format!("## Summary\n\n{}", decisions.summary));
        }

        if preserved_images.is_empty() {
            new_messages.push(ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(conversation_text),
                ..Default::default()
            });
        } else {
            match crate::scratchpads::multimodality::MultimodalElement::new("text".to_string(), conversation_text.clone()) {
                Ok(text_element) => {
                    let mut elements = vec![text_element];
                    elements.extend(preserved_images);
                    new_messages.push(ChatMessage {
                        role: "user".to_string(),
                        content: ChatContent::Multimodal(elements),
                        ..Default::default()
                    });
                }
                Err(_) => {
                    new_messages.push(ChatMessage {
                        role: "user".to_string(),
                        content: ChatContent::SimpleText(conversation_text),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // 4. Task done report as a separate message (if present)
    let task_done_report = find_task_done_report(original_messages);
    if let Some(report) = &task_done_report {
        new_messages.push(ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(format!("## Task Completion Report\n\n{}", report)),
            ..Default::default()
        });
    }

    // 5. Handoff message (with pending tasks only if no task_done report)
    let mut handoff_text = String::new();
    if task_done_report.is_none() && !decisions.pending_tasks.is_empty() {
        let tasks = decisions.pending_tasks.iter()
            .map(|t| format!("- {}", t))
            .collect::<Vec<_>>()
            .join("\n");
        handoff_text.push_str(&format!("## Pending Tasks\n\n{}\n\n---\n\n", tasks));
    }
    if !decisions.handoff_message.is_empty() {
        handoff_text.push_str(&decisions.handoff_message);
    }
    if !handoff_text.is_empty() {
        new_messages.push(ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(handoff_text),
            ..Default::default()
        });
    }

    Ok(new_messages)
}



async fn read_file_content_safe(
    _gcx: Arc<ARwLock<GlobalContext>>,
    path: &str,
    workspace_dirs: &[PathBuf],
) -> Result<String, String> {
    let full_path = if std::path::Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else if let Some(workspace) = workspace_dirs.first() {
        workspace.join(path)
    } else {
        return Err("No workspace directory available".to_string());
    };

    let canonical_path = full_path.canonicalize()
        .map_err(|e| format!("Failed to canonicalize path {}: {}", full_path.display(), e))?;

    let is_in_workspace = workspace_dirs.iter().any(|ws| {
        if let Ok(canonical_ws) = ws.canonicalize() {
            canonical_path.starts_with(&canonical_ws)
        } else {
            false
        }
    });

    let is_refact_path = canonical_path.to_string_lossy().contains(".refact/");

    if !is_in_workspace && !is_refact_path {
        return Err(format!(
            "Path {} is outside allowed directories",
            canonical_path.display()
        ));
    }

    let metadata = tokio::fs::metadata(&canonical_path)
        .await
        .map_err(|e| format!("Failed to get metadata for {}: {}", canonical_path.display(), e))?;

    if metadata.len() > MAX_FILE_SIZE as u64 {
        return Err(format!(
            "File {} is too large ({} bytes, max {} bytes)",
            canonical_path.display(),
            metadata.len(),
            MAX_FILE_SIZE
        ));
    }

    tokio::fs::read_to_string(&canonical_path)
        .await
        .map_err(|e| format!("Failed to read file {}: {}", canonical_path.display(), e))
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_xml_tag() {
        let content = r#"
<summary>
This is a test summary.
Multiple lines.
</summary>
"#;
        let result = parse_xml_tag(content, "summary");
        assert!(result.is_some());
        assert!(result.unwrap().contains("This is a test summary"));
    }

    #[test]
    fn test_parse_xml_tag_missing() {
        let content = "No tags here";
        let result = parse_xml_tag(content, "summary");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_list_tag() {
        let content = r#"
<files_to_open>
/src/main.rs
/src/config.rs
/src/lib.rs
</files_to_open>
"#;
        let result = parse_list_tag(content, "files_to_open");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "/src/main.rs");
        assert_eq!(result[1], "/src/config.rs");
        assert_eq!(result[2], "/src/lib.rs");
    }

    #[test]
    fn test_parse_list_tag_empty() {
        let content = r#"
<files_to_open>
</files_to_open>
"#;
        let result = parse_list_tag(content, "files_to_open");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_llm_response_complete() {
        let response = r#"
<summary>
Building JWT auth system for Axum API.
Token generation complete.
</summary>

<files_to_open>
/src/auth.rs
/src/config.rs
</files_to_open>

<messages_to_preserve>
MSG_ID:1
MSG_ID:8
</messages_to_preserve>

<memories_to_include>
/project/.refact/knowledge/jwt-design.md
</memories_to_include>

<tool_outputs_to_include>
MSG_ID:7
MSG_ID:15
</tool_outputs_to_include>

<pending_tasks>
Implement refresh tokens
Add rate limiting
</pending_tasks>

<handoff_message>
Continue with refresh token implementation.
</handoff_message>
"#;
        let decisions = parse_llm_response(response);

        assert!(decisions.summary.contains("JWT auth system"));
        assert_eq!(decisions.files_to_open.len(), 2);
        assert_eq!(decisions.messages_to_preserve.len(), 2);
        assert_eq!(decisions.memories_to_include.len(), 1);
        assert_eq!(decisions.tool_outputs_to_include.len(), 2);
        assert_eq!(decisions.tool_outputs_to_include[0], "MSG_ID:7");
        assert_eq!(decisions.pending_tasks.len(), 2);
        assert!(decisions.handoff_message.contains("refresh token"));
    }

    #[test]
    fn test_extract_conversation_metadata_basic() {
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText("Hello".to_string()),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("Hi there".to_string()),
                ..Default::default()
            },
        ];

        let metadata = extract_conversation_metadata(&messages);
        assert_eq!(metadata.annotated_messages.len(), 2);
        assert_eq!(metadata.annotated_messages[0].0, "MSG_ID:0");
        assert_eq!(metadata.annotated_messages[1].0, "MSG_ID:1");
    }

    #[test]
    fn test_extract_conversation_metadata_with_context_files() {
        let messages = vec![
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(vec![
                    ContextFile {
                        file_name: "/src/main.rs".to_string(),
                        file_content: "fn main() {}".to_string(),
                        line1: 1,
                        line2: 1,
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            },
        ];

        let metadata = extract_conversation_metadata(&messages);
        assert_eq!(metadata.context_files.len(), 1);
        assert_eq!(metadata.context_files[0].path, "/src/main.rs");
    }

    #[test]
    fn test_is_diff_content() {
        let diff_content = ChatContent::SimpleText(
            "--- a/file.rs\n+++ b/file.rs\n@@ -1,3 +1,4 @@".to_string()
        );
        assert!(is_diff_content(&diff_content));

        let non_diff = ChatContent::SimpleText("Just some text".to_string());
        assert!(!is_diff_content(&non_diff));
    }

    #[test]
    fn test_truncate_utf8_ascii() {
        let text = "Hello, World!";
        assert_eq!(truncate_utf8(text, 5), "Hello...");
        assert_eq!(truncate_utf8(text, 100), "Hello, World!");
    }

    #[test]
    fn test_truncate_utf8_unicode() {
        let text = "Hello 👋 World 🌍!";
        let result = truncate_utf8(text, 8);
        assert!(result.ends_with("..."));
        for i in 0..20 {
            let _ = truncate_utf8(text, i);
        }
    }

    #[test]
    fn test_truncate_utf8_cyrillic() {
        let text = "Привет мир";
        let result = truncate_utf8(text, 6);
        assert_eq!(result, "Привет...");
    }

    #[test]
    fn test_parse_xml_tag_close_before_open() {
        let content = "Some text with </summary> and then <summary>actual content</summary>";
        let result = parse_xml_tag(content, "summary");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "actual content");
    }

    #[test]
    fn test_parse_xml_tag_multiple_tags() {
        let content = r#"
<summary>First summary</summary>
Some text
<summary>Second summary</summary>
"#;
        let result = parse_xml_tag(content, "summary");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "First summary");
    }

    #[test]
    fn test_parse_xml_tag_missing_close() {
        let content = "<summary>Content without close tag";
        let result = parse_xml_tag(content, "summary");
        assert!(result.is_none());
    }

    #[test]
    fn test_memory_path_extraction_tasks() {
        let tool_output = r#"
Memory saved successfully.
File: /project/.refact/tasks/task-123/memories/2024-01-15_abc123_jwt-decision.md
Task: task-123
"#;
        let messages = vec![
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(tool_output.to_string()),
                ..Default::default()
            },
        ];

        let metadata = extract_conversation_metadata(&messages);
        assert_eq!(metadata.memory_paths.len(), 1);
        assert!(metadata.memory_paths[0].contains(".refact/tasks/"));
        assert!(metadata.memory_paths[0].contains("/memories/"));
    }

    #[test]
    fn test_memory_path_extraction_knowledge() {
        let tool_output = "Loaded: /home/user/project/.refact/knowledge/2024-01-15_design.md";
        let messages = vec![
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(tool_output.to_string()),
                ..Default::default()
            },
        ];

        let metadata = extract_conversation_metadata(&messages);
        assert_eq!(metadata.memory_paths.len(), 1);
        assert!(metadata.memory_paths[0].contains(".refact/knowledge/"));
    }

    #[test]
    fn test_diff_git_extraction() {
        let diff_content = r#"
diff --git a/src/auth.rs b/src/auth.rs
index 1234567..abcdefg 100644
--- a/src/auth.rs
+++ b/src/auth.rs
@@ -1,3 +1,4 @@
+use jwt::Token;
"#;
        let messages = vec![
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(diff_content.to_string()),
                ..Default::default()
            },
        ];

        let metadata = extract_conversation_metadata(&messages);
        assert!(!metadata.edited_files.is_empty());
        assert!(metadata.edited_files.iter().any(|f| f.path.contains("auth.rs")));
    }

    #[test]
    fn test_clean_path_string() {
        assert_eq!(clean_path_string("/path/to/file.rs"), "/path/to/file.rs");
        assert_eq!(clean_path_string("/path/to/file.rs)"), "/path/to/file.rs");
        assert_eq!(clean_path_string("/path/to/file.rs,"), "/path/to/file.rs");
        assert_eq!(clean_path_string("/path/to/file.rs\""), "/path/to/file.rs");
    }

    #[test]
    fn test_normalize_list_item_bullets() {
        assert_eq!(normalize_list_item("- /src/main.rs"), "/src/main.rs");
        assert_eq!(normalize_list_item("* /src/main.rs"), "/src/main.rs");
        assert_eq!(normalize_list_item("+ /src/main.rs"), "/src/main.rs");
        assert_eq!(normalize_list_item("  - /src/main.rs"), "/src/main.rs");
    }

    #[test]
    fn test_normalize_list_item_numbered() {
        assert_eq!(normalize_list_item("1. /src/main.rs"), "/src/main.rs");
        assert_eq!(normalize_list_item("1) /src/main.rs"), "/src/main.rs");
        assert_eq!(normalize_list_item("12. /src/main.rs"), "/src/main.rs");
        assert_eq!(normalize_list_item("  3) /src/main.rs"), "/src/main.rs");
    }

    #[test]
    fn test_normalize_list_item_backticks() {
        assert_eq!(normalize_list_item("`/src/main.rs`"), "/src/main.rs");
        assert_eq!(normalize_list_item("- `/src/main.rs`"), "/src/main.rs");
        assert_eq!(normalize_list_item("1. `/src/main.rs`"), "/src/main.rs");
    }

    #[test]
    fn test_normalize_list_item_quotes() {
        assert_eq!(normalize_list_item("\"/src/main.rs\""), "/src/main.rs");
        assert_eq!(normalize_list_item("'/src/main.rs'"), "/src/main.rs");
    }

    #[test]
    fn test_normalize_list_item_msg_id() {
        assert_eq!(normalize_list_item("- MSG_ID:5"), "MSG_ID:5");
        assert_eq!(normalize_list_item("1) MSG_ID:12"), "MSG_ID:12");
    }

    #[test]
    fn test_format_conversation_entry_user() {
        let metadata = ConversationMetadata::default();
        let msg = ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("Please help me with this code".to_string()),
            ..Default::default()
        };
        let result = format_conversation_entry(&msg, &metadata);
        assert!(result.contains("### 👤 User"));
        assert!(result.contains("Please help me with this code"));
    }

    #[test]
    fn test_format_conversation_entry_assistant_with_tools() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        let metadata = ConversationMetadata::default();
        let msg = ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("I'll search for the file.".to_string()),
            tool_calls: Some(vec![ChatToolCall {
                id: "call_123".to_string(),
                index: None,
                function: ChatToolFunction {
                    name: "search".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
            }]),
            ..Default::default()
        };
        let result = format_conversation_entry(&msg, &metadata);
        assert!(result.contains("### 🤖 Assistant"));
        assert!(result.contains("`search({})`"));
        assert!(result.contains("I'll search for the file."));
    }

    #[test]
    fn test_format_conversation_entry_skips_context_file() {
        let metadata = ConversationMetadata::default();
        let msg = ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::SimpleText("file content".to_string()),
            ..Default::default()
        };
        let result = format_conversation_entry(&msg, &metadata);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_conversation_entry_tool_resolves_name() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        let metadata = ConversationMetadata {
            annotated_messages: vec![
                ("MSG_ID:0".to_string(), ChatMessage {
                    role: "assistant".to_string(),
                    content: ChatContent::SimpleText("Let me search.".to_string()),
                    tool_calls: Some(vec![ChatToolCall {
                        id: "call_abc".to_string(),
                        index: None,
                        function: ChatToolFunction {
                            name: "grep".to_string(),
                            arguments: r#"{"query":"test"}"#.to_string(),
                        },
                        tool_type: "function".to_string(),
                    }]),
                    ..Default::default()
                }),
                ("MSG_ID:1".to_string(), ChatMessage {
                    role: "tool".to_string(),
                    tool_call_id: "call_abc".to_string(),
                    content: ChatContent::SimpleText("Found 3 results".to_string()),
                    ..Default::default()
                }),
            ],
            ..Default::default()
        };
        let tool_msg = &metadata.annotated_messages[1].1;
        let result = format_conversation_entry(tool_msg, &metadata);
        assert!(result.contains("### 🔧 Tool: `grep`"));
        assert!(result.contains("Found 3 results"));
    }

    #[test]
    fn test_messages_to_preserve_sorted_by_index() {
        let decisions = parse_llm_response(r#"
<summary>Test summary</summary>
<files_to_open></files_to_open>
<messages_to_preserve>
MSG_ID:10
MSG_ID:2
MSG_ID:5
MSG_ID:2
</messages_to_preserve>
<memories_to_include></memories_to_include>
<tool_outputs_to_include></tool_outputs_to_include>
<pending_tasks></pending_tasks>
<handoff_message>Continue</handoff_message>
"#);
        assert_eq!(decisions.messages_to_preserve, vec!["MSG_ID:10", "MSG_ID:2", "MSG_ID:5", "MSG_ID:2"]);
    }

    #[test]
    fn test_find_task_done_report() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText("Do the task".to_string()),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("I'll complete this task.".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_123".to_string(),
                    index: None,
                    function: ChatToolFunction {
                        name: "task_done".to_string(),
                        arguments: r#"{"report": "Detailed report here", "summary": "All done"}"#.to_string(),
                    },
                    tool_type: "function".to_string(),
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                tool_call_id: "call_123".to_string(),
                content: ChatContent::SimpleText(
                    r#"{"type":"task_done","summary":"All done","report":"Detailed report here","files_changed":["src/main.rs"]}"#.to_string()
                ),
                ..Default::default()
            },
        ];

        let report = find_task_done_report(&messages);
        assert!(report.is_some());
        let report_text = report.unwrap();
        assert!(report_text.contains("**All done**"));
        assert!(report_text.contains("Detailed report here"));
        assert!(report_text.contains("`src/main.rs`"));
    }

    #[test]
    fn test_find_task_done_report_no_task_done() {
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText("Hello".to_string()),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("Hi there!".to_string()),
                ..Default::default()
            },
        ];

        let report = find_task_done_report(&messages);
        assert!(report.is_none());
    }
}
