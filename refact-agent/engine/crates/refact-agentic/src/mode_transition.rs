use std::collections::HashSet;
use std::path::PathBuf;
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};

use refact_core::chat_types::{ChatContent, ChatMessage, ContextFile, MultimodalElement};

const MAX_FILE_SIZE: usize = 1024 * 1024;
const MODE_TRANSITION_CONTEXT_BUDGET_PERCENT: usize = 30;
const MODE_TRANSITION_FILES_BUDGET_PERCENT: usize = 70;
const MODE_TRANSITION_MAX_IMAGES: usize = 1;
const MODE_TRANSITION_INITIAL_PLAN_SYMBOL_CAP: usize = 120_000;

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

    static ref TASK_CARD_MARKER_REGEX: Regex = Regex::new(
        r"\bT-\d+\b"
    ).expect("Invalid task card marker regex");
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_plan: Option<String>,
    /// Optional `MSG_ID:N` reference the analysis model chose to pin verbatim as the
    /// new chat's plan banner. Preferred over `initial_plan` because it preserves the
    /// exact source artifact (e.g. a `plan()` report) instead of a paraphrase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionContextBudget {
    pub previous_symbols: usize,
    pub total_symbols: usize,
    pub files_symbols: usize,
    pub messages_symbols: usize,
    pub max_images: usize,
}

pub fn text_symbols(text: &str) -> usize {
    text.chars().count()
}

pub fn message_symbols(msg: &ChatMessage) -> usize {
    let mut symbols = text_symbols(&msg.content.content_text_only());
    if let Some(reasoning) = &msg.reasoning_content {
        symbols += text_symbols(reasoning);
    }
    if let Some(tool_calls) = &msg.tool_calls {
        for tool_call in tool_calls {
            symbols += text_symbols(&tool_call.function.name);
            symbols += text_symbols(&tool_call.function.arguments);
        }
    }
    symbols
}

pub fn calculate_transition_context_budget(messages: &[ChatMessage]) -> TransitionContextBudget {
    let previous_symbols = messages.iter().map(message_symbols).sum::<usize>();
    let total_symbols = previous_symbols * MODE_TRANSITION_CONTEXT_BUDGET_PERCENT / 100;
    let files_symbols = total_symbols * MODE_TRANSITION_FILES_BUDGET_PERCENT / 100;
    let messages_symbols = total_symbols.saturating_sub(files_symbols);

    TransitionContextBudget {
        previous_symbols,
        total_symbols,
        files_symbols,
        messages_symbols,
        max_images: MODE_TRANSITION_MAX_IMAGES,
    }
}

fn truncate_utf8_to_budget(text: &str, max_symbols: usize) -> String {
    let symbol_count = text_symbols(text);
    if symbol_count <= max_symbols {
        return text.to_string();
    }
    if max_symbols == 0 {
        return String::new();
    }
    if max_symbols <= 3 {
        return text.chars().take(max_symbols).collect();
    }

    let mut truncated: String = text.chars().take(max_symbols - 3).collect();
    truncated.push_str("...");
    truncated
}

fn take_from_symbol_budget(text: &str, remaining_symbols: &mut usize) -> Option<String> {
    if *remaining_symbols == 0 || text.trim().is_empty() {
        return None;
    }

    let limited = truncate_utf8_to_budget(text, *remaining_symbols);
    let used = text_symbols(&limited);
    *remaining_symbols = remaining_symbols.saturating_sub(used);

    if limited.trim().is_empty() {
        None
    } else {
        Some(limited)
    }
}

pub fn context_file_rendered_symbols(file: &ContextFile) -> usize {
    text_symbols(&format!(
        "{}:{}-{}\n{}",
        file.file_name, file.line1, file.line2, file.file_content
    ))
}

fn context_file_prefix_symbols(file_name: &str, line1: usize, line2: usize) -> usize {
    text_symbols(&format!("{}:{}-{}\n", file_name, line1, line2))
}

pub fn push_context_file_with_budget(
    context_files: &mut Vec<ContextFile>,
    file_name: String,
    file_content: String,
    remaining_symbols: &mut usize,
) {
    let separator_symbols = if context_files.is_empty() { 0 } else { 2 };
    if *remaining_symbols <= separator_symbols {
        return;
    }

    let original_line_count = file_content.lines().count().max(1);
    let available_symbols = *remaining_symbols - separator_symbols;
    let prefix_symbols = context_file_prefix_symbols(&file_name, 1, original_line_count);
    if available_symbols <= prefix_symbols {
        return;
    }

    let content_budget = available_symbols - prefix_symbols;
    let limited_content = truncate_utf8_to_budget(&file_content, content_budget);
    if limited_content.is_empty() {
        return;
    }

    let mut context_file = ContextFile {
        file_name,
        file_content: limited_content,
        line1: 1,
        line2: original_line_count,
        ..Default::default()
    };
    context_file.line2 = context_file.file_content.lines().count().max(1);

    let used_symbols = separator_symbols + context_file_rendered_symbols(&context_file);
    if used_symbols <= *remaining_symbols {
        *remaining_symbols -= used_symbols;
        context_files.push(context_file);
    }
}

pub fn count_images_in_messages(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .filter_map(|msg| match &msg.content {
            ChatContent::Multimodal(elements) => {
                Some(elements.iter().filter(|el| el.is_image()).count())
            }
            _ => None,
        })
        .sum()
}

pub fn extract_conversation_metadata(messages: &[ChatMessage]) -> ConversationMetadata {
    let mut metadata = ConversationMetadata::default();
    let mut seen_files: HashSet<String> = HashSet::new();
    let mut seen_memories: HashSet<String> = HashSet::new();

    for (idx, msg) in messages.iter().enumerate() {
        let msg_id = format!("MSG_ID:{}", idx);
        metadata
            .annotated_messages
            .push((msg_id.clone(), msg.clone()));

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
                    let path_str = cap
                        .get(1)
                        .or_else(|| cap.get(2))
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
            text.contains("+++") && text.contains("---")
                || text.contains("@@ ")
                || text.starts_with("diff ")
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
    let s = s
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'')
        .trim();
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

fn has_substantive_plan_markers(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let mut markers = 0;
    for marker in [
        "implementation plan",
        "## tasks",
        "### task",
        "acceptance criteria",
        "verification",
        "final verification",
        "file map",
        "wave",
        "card",
    ] {
        if lower.contains(marker) {
            markers += 1;
        }
    }
    if TASK_CARD_MARKER_REGEX.is_match(text) {
        markers += 1;
    }
    text_symbols(text) > 500 && markers >= 2
}

pub fn extract_initial_plan_text(source_content: &str, handoff_message: &str) -> Option<String> {
    if let Some(plan) = parse_xml_tag(source_content, "plan") {
        if !plan.trim().is_empty() {
            return Some(plan);
        }
    }
    let handoff_message = handoff_message.trim();
    if has_substantive_plan_markers(handoff_message) {
        Some(handoff_message.to_string())
    } else {
        None
    }
}

pub fn parse_llm_response(response: &str) -> ParsedDecisions {
    let handoff_message = parse_xml_tag(response, "handoff_message").unwrap_or_default();
    ParsedDecisions {
        summary: parse_xml_tag(response, "summary").unwrap_or_default(),
        files_to_open: parse_list_tag(response, "files_to_open"),
        messages_to_preserve: parse_list_tag(response, "messages_to_preserve"),
        memories_to_include: parse_list_tag(response, "memories_to_include"),
        tool_outputs_to_include: parse_list_tag(response, "tool_outputs_to_include"),
        pending_tasks: parse_list_tag(response, "pending_tasks"),
        initial_plan: extract_initial_plan_text(response, &handoff_message),
        plan_source: parse_xml_tag(response, "plan_source")
            .map(|s| normalize_list_item(&s))
            .filter(|s| !s.is_empty()),
        handoff_message,
    }
}

pub fn format_annotated_messages(metadata: &ConversationMetadata) -> String {
    let mut result = String::new();

    let mut tool_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (_, message) in &metadata.annotated_messages {
        if let Some(tool_calls) = &message.tool_calls {
            for tool_call in tool_calls {
                tool_names.insert(tool_call.id.clone(), tool_call.function.name.clone());
            }
        }
    }

    for (msg_id, msg) in &metadata.annotated_messages {
        // Surface which tool produced a `tool` result (e.g. `tool: plan`) so the
        // analysis model can pick the right plan-like artifact to pin.
        let role = if msg.role == "tool" {
            match tool_names.get(&msg.tool_call_id) {
                Some(name) => format!("tool: {}", name),
                None => "tool".to_string(),
            }
        } else {
            msg.role.clone()
        };
        let role = &role;
        let content_preview = match &msg.content {
            ChatContent::SimpleText(text) => truncate_utf8(text, 500),
            ChatContent::ContextFiles(files) => {
                format!(
                    "[Context files: {}]",
                    files
                        .iter()
                        .map(|f| f.file_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            ChatContent::Multimodal(elements) => {
                let text_parts: Vec<String> = elements
                    .iter()
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
                let tools: Vec<String> = tool_calls
                    .iter()
                    .map(|tc| {
                        format!(
                            "{}({})",
                            tc.function.name,
                            truncate_utf8(&tc.function.arguments, 100)
                        )
                    })
                    .collect();
                format!("\n[tool_calls: {}]", tools.join(", "))
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        result.push_str(&format!(
            "[{}] [{}]\n{}{}\n\n",
            msg_id, role, content_preview, tool_info
        ));
    }

    result
}

pub fn format_file_list(metadata: &ConversationMetadata) -> String {
    let mut lines = Vec::new();

    for file_ref in &metadata.context_files {
        lines.push(format!(
            "- {} (from {}, {})",
            file_ref.path, file_ref.source, file_ref.msg_id
        ));
    }

    for file_ref in &metadata.edited_files {
        lines.push(format!(
            "- {} (edited, from {}, {})",
            file_ref.path, file_ref.source, file_ref.msg_id
        ));
    }

    if lines.is_empty() {
        "No files found in conversation".to_string()
    } else {
        lines.join("\n")
    }
}

pub fn format_memory_list(metadata: &ConversationMetadata) -> String {
    if metadata.memory_paths.is_empty() {
        "No memory/knowledge files found".to_string()
    } else {
        metadata
            .memory_paths
            .iter()
            .map(|p| format!("- {}", p))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub fn truncate_utf8(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}

pub fn format_budget_summary(budget: TransitionContextBudget, messages: &[ChatMessage]) -> String {
    format!(
        "Previous context size: {} symbols. Preserve at most {} symbols total ({}%): about {} symbols for files/memories ({}%) and {} symbols for messages/tool outputs/summary. Preserve at most {} image(s); previous context contains {} image(s).",
        budget.previous_symbols,
        budget.total_symbols,
        MODE_TRANSITION_CONTEXT_BUDGET_PERCENT,
        budget.files_symbols,
        MODE_TRANSITION_FILES_BUDGET_PERCENT,
        budget.messages_symbols,
        budget.max_images,
        count_images_in_messages(messages),
    )
}

fn find_finish_report(messages: &[ChatMessage]) -> Option<String> {
    let mut finish_call_id: Option<String> = None;
    for msg in messages.iter().rev() {
        if msg.role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                for tc in tool_calls {
                    if tc.function.name == "finish" {
                        finish_call_id = Some(tc.id.clone());
                        break;
                    }
                }
            }
            if finish_call_id.is_some() {
                break;
            }
        }
    }

    let call_id = finish_call_id?;

    for msg in messages.iter().rev() {
        if msg.role == "tool" && msg.tool_call_id == call_id {
            if let ChatContent::SimpleText(text) = &msg.content {
                if let Ok(obj) = serde_json::from_str::<serde_json::Value>(text) {
                    let summary = obj.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                    let report = obj.get("report").and_then(|v| v.as_str()).unwrap_or("");
                    let files_changed: Vec<&str> = obj
                        .get("files_changed")
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
                    let calls: Vec<String> = tool_calls
                        .iter()
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
        ChatContent::Multimodal(elements) => elements
            .iter()
            .filter_map(|el| {
                if el.is_text() {
                    Some(el.m_content.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ChatContent::ContextFiles(_) => String::new(),
    }
}

fn plan_version(message: &ChatMessage) -> Option<u32> {
    if message.role != "plan" {
        return None;
    }
    message
        .extra
        .get("plan")?
        .get("version")?
        .as_u64()
        .and_then(|version| u32::try_from(version).ok())
}

fn current_base_plan_message(messages: &[ChatMessage]) -> Option<&ChatMessage> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            plan_version(message).map(|version| (index, version, message))
        })
        .max_by_key(|(index, version, _)| (*version, *index))
        .map(|(_, _, message)| message)
}

fn is_plan_delta_event(message: &ChatMessage) -> bool {
    message.role == "event"
        && message
            .extra
            .get("event")
            .and_then(|event| event.get("subkind"))
            .and_then(|subkind| subkind.as_str())
            == Some("plan_delta")
}

/// Tools whose result message is a self-contained plan/spec artifact worth pinning
/// as the new chat's plan banner. `finish` is intentionally excluded: its report is
/// already carried as a "Task Completion Report" user message by `find_finish_report`.
fn is_plan_report_tool(name: &str) -> bool {
    matches!(name, "plan" | "task_done")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Map `tool_call_id` -> tool name from every assistant `tool_calls` entry so we can
/// label and classify the matching `tool` result messages.
fn collect_tool_call_names(messages: &[ChatMessage]) -> std::collections::HashMap<String, String> {
    let mut names = std::collections::HashMap::new();
    for message in messages {
        if let Some(tool_calls) = &message.tool_calls {
            for tool_call in tool_calls {
                names.insert(tool_call.id.clone(), tool_call.function.name.clone());
            }
        }
    }
    names
}

/// When the conversation has exactly one plan-like tool report, pin it deterministically
/// even if the analysis model selected nothing (covers tool/handoff paths that have no
/// model step, and acts as a safety net for the mode-transition path).
fn deterministic_plan_report_text(messages: &[ChatMessage]) -> Option<String> {
    let tool_names = collect_tool_call_names(messages);
    let mut candidate: Option<&ChatMessage> = None;
    for message in messages {
        if message.role != "tool" {
            continue;
        }
        let is_plan_report = tool_names
            .get(&message.tool_call_id)
            .map(|name| is_plan_report_tool(name))
            .unwrap_or(false);
        if !is_plan_report {
            continue;
        }
        if candidate.is_some() {
            // Ambiguous (more than one plan-like report); defer to the model instead.
            return None;
        }
        candidate = Some(message);
    }
    let text = candidate?.content.content_text_only();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Resolve a `MSG_ID:N` reference (as emitted by the analysis model in `<plan_source>`)
/// to that message's verbatim text content.
fn plan_text_from_source_reference(messages: &[ChatMessage], reference: &str) -> Option<String> {
    let idx = reference
        .trim()
        .trim_start_matches("MSG_ID:")
        .trim()
        .parse::<usize>()
        .ok()?;
    let text = messages.get(idx)?.content.content_text_only();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Choose the plan text to pin into the new chat when there is no existing `plan`-role
/// message to carry. Priority: explicit `<plan_source>` MSG_ID -> model-authored
/// `<plan>`/substantive handoff -> single deterministic plan-like report.
fn resolve_pinned_plan_text(
    messages: &[ChatMessage],
    decisions: &ParsedDecisions,
) -> Option<String> {
    if let Some(reference) = decisions.plan_source.as_deref() {
        if let Some(text) = plan_text_from_source_reference(messages, reference) {
            return Some(text);
        }
    }
    if let Some(plan) = decisions
        .initial_plan
        .as_deref()
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
    {
        return Some(plan.to_string());
    }
    deterministic_plan_report_text(messages)
}

/// Build a fresh `plan`-role message (the PlanBanner artifact) from pinned plan text,
/// capping its size to keep new-chat context bounded.
pub fn make_pinned_plan_message(text: &str, symbol_cap: usize) -> ChatMessage {
    let body: String = if text_symbols(text) > symbol_cap {
        text.chars().take(symbol_cap).collect()
    } else {
        text.to_string()
    };
    let mut extra = serde_json::Map::new();
    extra.insert(
        "plan".to_string(),
        serde_json::json!({
            "mode": "",
            "version": 1,
            "created_at_ms": now_ms(),
            "supersedes": null,
        }),
    );
    ChatMessage {
        role: "plan".to_string(),
        content: ChatContent::SimpleText(body),
        preserve: Some(true),
        extra,
        ..Default::default()
    }
}

/// Plan messages to carry into a new chat for paths that have no analysis model
/// (handoff, deterministic tool paths): an existing pinned plan plus its deltas, or a
/// single deterministic plan-like report pinned as a fresh plan banner.
pub fn carried_plan_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    if let Some(existing_plan) = current_base_plan_message(messages) {
        let mut carried = vec![existing_plan.clone()];
        carried.extend(
            messages
                .iter()
                .filter(|message| is_plan_delta_event(message))
                .cloned(),
        );
        return carried;
    }
    if let Some(text) = deterministic_plan_report_text(messages) {
        return vec![make_pinned_plan_message(
            &text,
            MODE_TRANSITION_INITIAL_PLAN_SYMBOL_CAP,
        )];
    }
    Vec::new()
}

pub async fn assemble_new_chat(
    original_messages: &[ChatMessage],
    decisions: &ParsedDecisions,
    workspace_dirs: &[PathBuf],
) -> Result<Vec<ChatMessage>, String> {
    let metadata = extract_conversation_metadata(original_messages);
    let budget = calculate_transition_context_budget(original_messages);
    let mut remaining_files_symbols = budget.files_symbols;
    let mut remaining_messages_symbols = budget.messages_symbols;
    let mut remaining_images = budget.max_images;
    let mut new_messages: Vec<ChatMessage> = Vec::new();

    let allowed_files: HashSet<&str> = metadata
        .context_files
        .iter()
        .map(|f| f.path.as_str())
        .chain(metadata.edited_files.iter().map(|f| f.path.as_str()))
        .collect();
    let allowed_memories: HashSet<&str> =
        metadata.memory_paths.iter().map(|s| s.as_str()).collect();

    let mut file_contents: Vec<ContextFile> = Vec::new();
    for path in &decisions.files_to_open {
        if !allowed_files.contains(path.as_str()) {
            tracing::warn!("Skipping file {} - not in conversation allowlist", path);
            continue;
        }
        match read_file_content_safe(path, workspace_dirs).await {
            Ok(content) => {
                push_context_file_with_budget(
                    &mut file_contents,
                    path.clone(),
                    content,
                    &mut remaining_files_symbols,
                );
            }
            Err(e) => {
                tracing::warn!("Failed to read file {}: {}", path, e);
            }
        }
    }
    if !file_contents.is_empty() {
        new_messages.push(ChatMessage {
            role: "context_file".to_string(),
            content: refact_core::chat_types::ChatContent::ContextFiles(file_contents),
            ..Default::default()
        });
    }

    let mut memory_contents: Vec<ContextFile> = Vec::new();
    for memory_path in &decisions.memories_to_include {
        if !allowed_memories.contains(memory_path.as_str()) {
            tracing::warn!(
                "Skipping memory {} - not in conversation allowlist",
                memory_path
            );
            continue;
        }
        match read_file_content_safe(memory_path, workspace_dirs).await {
            Ok(content) => {
                push_context_file_with_budget(
                    &mut memory_contents,
                    memory_path.clone(),
                    content,
                    &mut remaining_files_symbols,
                );
            }
            Err(e) => {
                tracing::warn!("Failed to read memory {}: {}", memory_path, e);
            }
        }
    }
    if !memory_contents.is_empty() {
        new_messages.push(ChatMessage {
            role: "context_file".to_string(),
            content: refact_core::chat_types::ChatContent::ContextFiles(memory_contents),
            ..Default::default()
        });
    }

    let mut preserved_indices: HashSet<usize> = decisions
        .messages_to_preserve
        .iter()
        .filter_map(|msg_id_ref| {
            let id = msg_id_ref.trim_start_matches("MSG_ID:");
            id.parse::<usize>().ok()
        })
        .collect();
    let tool_output_indices: HashSet<usize> = decisions
        .tool_outputs_to_include
        .iter()
        .filter_map(|msg_id_ref| {
            let id = msg_id_ref.trim_start_matches("MSG_ID:");
            id.parse::<usize>().ok()
        })
        .collect();
    preserved_indices.extend(&tool_output_indices);
    preserved_indices.extend(
        metadata
            .annotated_messages
            .iter()
            .enumerate()
            .filter_map(|(idx, (_, msg))| (msg.preserve == Some(true)).then_some(idx)),
    );

    let mut all_indices: Vec<usize> = preserved_indices.into_iter().collect();
    all_indices.sort();
    all_indices.dedup();

    let mut conversation_parts: Vec<String> = Vec::new();
    let mut preserved_images: Vec<MultimodalElement> = Vec::new();
    for idx in &all_indices {
        if let Some((_, msg)) = metadata.annotated_messages.get(*idx) {
            let formatted = format_conversation_entry(msg, &metadata);
            let framing_symbols = if conversation_parts.is_empty() {
                text_symbols("## Previous Conversation\n\n")
            } else {
                text_symbols("\n\n---\n\n")
            };
            if !formatted.is_empty() && remaining_messages_symbols > framing_symbols {
                let mut entry_budget = remaining_messages_symbols - framing_symbols;
                if let Some(limited) = take_from_symbol_budget(&formatted, &mut entry_budget) {
                    let used = framing_symbols + text_symbols(&limited);
                    remaining_messages_symbols = remaining_messages_symbols.saturating_sub(used);
                    conversation_parts.push(limited);
                }
            }
            if let ChatContent::Multimodal(elements) = &msg.content {
                for el in elements {
                    if el.is_image() && remaining_images > 0 {
                        preserved_images.push(el.clone());
                        remaining_images -= 1;
                    }
                }
            }
        }
    }

    let has_conversation = !conversation_parts.is_empty()
        || (!decisions.summary.is_empty() && remaining_messages_symbols > 0);
    if has_conversation {
        let mut conversation_text = String::new();
        if !conversation_parts.is_empty() {
            conversation_text.push_str("## Previous Conversation\n\n");
            conversation_text.push_str(&conversation_parts.join("\n\n---\n\n"));
        }
        if !decisions.summary.is_empty() && remaining_messages_symbols > 0 {
            let summary_prefix = if conversation_text.is_empty() {
                "## Summary\n\n"
            } else {
                "\n\n---\n\n## Summary\n\n"
            };
            let prefix_symbols = text_symbols(summary_prefix);
            if remaining_messages_symbols > prefix_symbols {
                conversation_text.push_str(summary_prefix);
                remaining_messages_symbols -= prefix_symbols;
                if let Some(summary) =
                    take_from_symbol_budget(&decisions.summary, &mut remaining_messages_symbols)
                {
                    conversation_text.push_str(&summary);
                }
            }
        }

        if preserved_images.is_empty() {
            new_messages.push(ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(conversation_text),
                ..Default::default()
            });
        } else {
            match MultimodalElement::new("text".to_string(), conversation_text.clone()) {
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

    if let Some(existing_plan) = current_base_plan_message(original_messages) {
        new_messages.push(existing_plan.clone());
        new_messages.extend(
            original_messages
                .iter()
                .filter(|message| is_plan_delta_event(message))
                .cloned(),
        );
    } else if let Some(pinned_plan_text) = resolve_pinned_plan_text(original_messages, decisions) {
        let initial_plan = pinned_plan_text.trim();
        if !initial_plan.is_empty() {
            let plan_budget = remaining_messages_symbols
                .max(MODE_TRANSITION_INITIAL_PLAN_SYMBOL_CAP.min(text_symbols(initial_plan)));
            let plan_message = make_pinned_plan_message(initial_plan, plan_budget);
            let used = text_symbols(&plan_message.content.content_text_only());
            new_messages.push(plan_message);
            remaining_messages_symbols = remaining_messages_symbols.saturating_sub(used);
        }
    }

    let finish_report = find_finish_report(original_messages);
    if let Some(report) = &finish_report {
        let prefix = "## Task Completion Report\n\n";
        let prefix_symbols = text_symbols(prefix);
        if remaining_messages_symbols > prefix_symbols {
            let mut text = prefix.to_string();
            remaining_messages_symbols -= prefix_symbols;
            if let Some(report) = take_from_symbol_budget(report, &mut remaining_messages_symbols) {
                text.push_str(&report);
                new_messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: ChatContent::SimpleText(text),
                    ..Default::default()
                });
            }
        }
    }

    let mut handoff_text = String::new();
    if finish_report.is_none() && !decisions.pending_tasks.is_empty() {
        let tasks = decisions
            .pending_tasks
            .iter()
            .map(|t| format!("- {}", t))
            .collect::<Vec<_>>()
            .join("\n");
        handoff_text.push_str(&format!("## Pending Tasks\n\n{}\n\n---\n\n", tasks));
    }
    if !decisions.handoff_message.is_empty() {
        handoff_text.push_str(&decisions.handoff_message);
    }
    if !handoff_text.is_empty() {
        if let Some(limited_handoff) =
            take_from_symbol_budget(&handoff_text, &mut remaining_messages_symbols)
        {
            new_messages.push(ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(limited_handoff),
                ..Default::default()
            });
        }
    }

    Ok(new_messages)
}

async fn read_file_content_safe(path: &str, workspace_dirs: &[PathBuf]) -> Result<String, String> {
    let full_path = if std::path::Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else if let Some(workspace) = workspace_dirs.first() {
        workspace.join(path)
    } else {
        return Err("No workspace directory available".to_string());
    };

    let canonical_path = full_path
        .canonicalize()
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|e| format!("Failed to canonicalize path {}: {}", full_path.display(), e))?;

    let is_in_workspace = workspace_dirs.iter().any(|ws| {
        if let Ok(canonical_ws) = ws.canonicalize() {
            let canonical_ws = dunce::simplified(&canonical_ws).to_path_buf();
            canonical_path.starts_with(&canonical_ws)
        } else {
            false
        }
    });

    let is_refact_path = canonical_path.components().any(
        |component| matches!(component, std::path::Component::Normal(name) if name == ".refact"),
    );

    if !is_in_workspace && !is_refact_path {
        return Err(format!(
            "Path {} is outside allowed directories",
            canonical_path.display()
        ));
    }

    let metadata = tokio::fs::metadata(&canonical_path).await.map_err(|e| {
        format!(
            "Failed to get metadata for {}: {}",
            canonical_path.display(),
            e
        )
    })?;

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
    use refact_core::chat_types::{ChatToolCall, ChatToolFunction, ContextFile};

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
        assert!(decisions.initial_plan.is_none());
    }

    #[test]
    fn test_parse_plan_tag_for_initial_plan() {
        let response = r#"
<summary>Move this to a task plan.</summary>
<plan>
Wave 0
- Card T-1: Build storage
- Acceptance Criteria: tests pass
</plan>
<handoff_message>Continue with setup.</handoff_message>
"#;

        let decisions = parse_llm_response(response);

        let plan = decisions.initial_plan.unwrap();
        assert!(plan.contains("Wave 0"));
        assert!(plan.contains("Card T-1"));
        assert!(!plan.contains("<plan>"));
    }

    #[test]
    fn test_heuristic_initial_plan_from_substantive_handoff() {
        let handoff = format!(
            "Wave 0 ready. Card T-1 implements storage. Acceptance Criteria: cargo test passes. {}",
            "Create follow-up cards and preserve dependencies. ".repeat(20)
        );
        assert!(text_symbols(&handoff) > 500);

        let plan = extract_initial_plan_text("", &handoff).unwrap();

        assert!(plan.contains("Wave 0"));
        assert!(plan.contains("Acceptance Criteria"));
    }

    #[test]
    fn test_heuristic_initial_plan_is_conservative() {
        let handoff = format!(
            "Continue the conversation with these implementation details. {}",
            "No structured planning markers here. ".repeat(30)
        );

        assert!(extract_initial_plan_text("", &handoff).is_none());
    }

    #[test]
    fn test_heuristic_initial_plan_accepts_task_plan_format() {
        let handoff = format!(
            "# Feature Implementation Plan\n\n## File Map\n- Modify: `src/lib.rs`\n\n## Tasks\n\n### Task 1: Update behavior\n- [ ] Step 1: Write test\n\n## Final Verification\n- `cargo test`\n\n{}",
            "Preserve exact files, verification, and acceptance criteria. ".repeat(20)
        );

        let plan = extract_initial_plan_text("", &handoff).unwrap();

        assert!(plan.contains("Feature Implementation Plan"));
        assert!(plan.contains("Final Verification"));
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
        let messages = vec![ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: "/src/main.rs".to_string(),
                file_content: "fn main() {}".to_string(),
                line1: 1,
                line2: 1,
                ..Default::default()
            }]),
            ..Default::default()
        }];

        let metadata = extract_conversation_metadata(&messages);
        assert_eq!(metadata.context_files.len(), 1);
        assert_eq!(metadata.context_files[0].path, "/src/main.rs");
    }

    #[test]
    fn test_is_diff_content() {
        let diff_content =
            ChatContent::SimpleText("--- a/file.rs\n+++ b/file.rs\n@@ -1,3 +1,4 @@".to_string());
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
    fn test_transition_context_budget_splits_previous_symbols() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("x".repeat(1000)),
            ..Default::default()
        }];

        let budget = calculate_transition_context_budget(&messages);

        assert_eq!(budget.previous_symbols, 1000);
        assert_eq!(budget.total_symbols, 300);
        assert_eq!(budget.files_symbols, 210);
        assert_eq!(budget.messages_symbols, 90);
        assert_eq!(budget.max_images, 1);
    }

    #[test]
    fn test_context_file_budget_truncates_rendered_file() {
        let mut files = Vec::new();
        let mut remaining_symbols = 80;

        push_context_file_with_budget(
            &mut files,
            "src/main.rs".to_string(),
            "x".repeat(1000),
            &mut remaining_symbols,
        );

        assert_eq!(files.len(), 1);
        assert!(context_file_rendered_symbols(&files[0]) <= 80);
        assert!(files[0].file_content.ends_with("..."));
        assert_eq!(
            remaining_symbols + context_file_rendered_symbols(&files[0]),
            80
        );
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
        let messages = vec![ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(tool_output.to_string()),
            ..Default::default()
        }];

        let metadata = extract_conversation_metadata(&messages);
        assert_eq!(metadata.memory_paths.len(), 1);
        assert!(metadata.memory_paths[0].contains(".refact/tasks/"));
        assert!(metadata.memory_paths[0].contains("/memories/"));
    }

    #[test]
    fn test_memory_path_extraction_knowledge() {
        let tool_output = "Loaded: /home/user/project/.refact/knowledge/2024-01-15_design.md";
        let messages = vec![ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(tool_output.to_string()),
            ..Default::default()
        }];

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
        let messages = vec![ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(diff_content.to_string()),
            ..Default::default()
        }];

        let metadata = extract_conversation_metadata(&messages);
        assert!(!metadata.edited_files.is_empty());
        assert!(metadata
            .edited_files
            .iter()
            .any(|f| f.path.contains("auth.rs")));
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
                extra_content: None,
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
        let metadata = ConversationMetadata {
            annotated_messages: vec![
                (
                    "MSG_ID:0".to_string(),
                    ChatMessage {
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
                            extra_content: None,
                        }]),
                        ..Default::default()
                    },
                ),
                (
                    "MSG_ID:1".to_string(),
                    ChatMessage {
                        role: "tool".to_string(),
                        tool_call_id: "call_abc".to_string(),
                        content: ChatContent::SimpleText("Found 3 results".to_string()),
                        ..Default::default()
                    },
                ),
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
        let decisions = parse_llm_response(
            r#"
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
"#,
        );
        assert_eq!(
            decisions.messages_to_preserve,
            vec!["MSG_ID:10", "MSG_ID:2", "MSG_ID:5", "MSG_ID:2"]
        );
    }

    #[tokio::test]
    async fn assemble_new_chat_emits_plan_role_not_user_message() {
        let plan = "# Feature Implementation Plan\n\n## Tasks\n\n### Task 1: Build\n- [ ] Verify: `cargo test`";
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("prepare handoff".to_string()),
            ..Default::default()
        }];
        let decisions = ParsedDecisions {
            initial_plan: Some(plan.to_string()),
            ..Default::default()
        };

        let new_messages = assemble_new_chat(&messages, &decisions, &[]).await.unwrap();
        let plan_messages: Vec<_> = new_messages
            .iter()
            .filter(|msg| msg.role == "plan")
            .collect();

        assert_eq!(plan_messages.len(), 1);
        assert_eq!(plan_messages[0].content.content_text_only(), plan);
        assert_eq!(plan_messages[0].preserve, Some(true));
        assert_eq!(
            plan_messages[0].extra["plan"]["mode"],
            serde_json::json!("")
        );
        assert_eq!(
            plan_messages[0].extra["plan"]["version"],
            serde_json::json!(1)
        );
        assert!(plan_messages[0].extra["plan"]["supersedes"].is_null());
        assert!(
            plan_messages[0].extra["plan"]["created_at_ms"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );
        assert!(!new_messages.iter().any(|msg| {
            msg.role == "user" && msg.content.content_text_only().contains("## Initial Plan")
        }));
    }

    #[tokio::test]
    async fn assemble_new_chat_preserves_existing_plan_and_deltas() {
        let mut older_plan_extra = serde_json::Map::new();
        older_plan_extra.insert(
            "plan".to_string(),
            serde_json::json!({
                "mode": "agent",
                "version": 1,
                "created_at_ms": 1000,
                "supersedes": null,
            }),
        );
        let mut current_plan_extra = serde_json::Map::new();
        current_plan_extra.insert(
            "plan".to_string(),
            serde_json::json!({
                "mode": "task_agent",
                "version": 2,
                "created_at_ms": 2000,
                "supersedes": "old-plan-id",
                "truncated": true,
                "original_chars": 12345,
            }),
        );
        let mut first_delta_extra = serde_json::Map::new();
        first_delta_extra.insert(
            "event".to_string(),
            serde_json::json!({
                "subkind": "plan_delta",
                "source": "tool.update_plan",
                "payload": {"seq": 1, "summary": "first summary"},
            }),
        );
        let mut other_event_extra = serde_json::Map::new();
        other_event_extra.insert(
            "event".to_string(),
            serde_json::json!({
                "subkind": "system_notice",
                "source": "test",
                "payload": {"ignore": true},
            }),
        );
        let mut second_delta_extra = serde_json::Map::new();
        second_delta_extra.insert(
            "event".to_string(),
            serde_json::json!({
                "subkind": "plan_delta",
                "source": "tool.update_plan",
                "payload": {"seq": 2, "summary": "second summary"},
            }),
        );
        let messages = vec![
            ChatMessage {
                role: "plan".to_string(),
                message_id: "older-plan-id".to_string(),
                content: ChatContent::SimpleText("older plan".to_string()),
                preserve: Some(true),
                extra: older_plan_extra,
                ..Default::default()
            },
            ChatMessage {
                role: "event".to_string(),
                message_id: "delta-1".to_string(),
                content: ChatContent::SimpleText("first update".to_string()),
                extra: first_delta_extra,
                ..Default::default()
            },
            ChatMessage {
                role: "event".to_string(),
                message_id: "other-event".to_string(),
                content: ChatContent::SimpleText("do not copy".to_string()),
                extra: other_event_extra,
                ..Default::default()
            },
            ChatMessage {
                role: "plan".to_string(),
                message_id: "current-plan-id".to_string(),
                content: ChatContent::SimpleText("current base plan bytes".to_string()),
                preserve: Some(true),
                extra: current_plan_extra,
                ..Default::default()
            },
            ChatMessage {
                role: "event".to_string(),
                message_id: "delta-2".to_string(),
                content: ChatContent::SimpleText("second update".to_string()),
                extra: second_delta_extra,
                ..Default::default()
            },
        ];
        let decisions = ParsedDecisions {
            initial_plan: Some("fallback".to_string()),
            ..Default::default()
        };

        let new_messages = assemble_new_chat(&messages, &decisions, &[]).await.unwrap();
        let hidden_messages: Vec<_> = new_messages
            .iter()
            .filter(|message| message.role == "plan" || message.role == "event")
            .collect();

        assert_eq!(hidden_messages.len(), 3);
        assert_eq!(hidden_messages[0].role, "plan");
        assert_eq!(hidden_messages[0].message_id, "current-plan-id");
        assert_eq!(
            hidden_messages[0].content.content_text_only(),
            "current base plan bytes"
        );
        assert_eq!(
            hidden_messages[0].extra["plan"]["mode"],
            serde_json::json!("task_agent")
        );
        assert_eq!(
            hidden_messages[0].extra["plan"]["version"],
            serde_json::json!(2)
        );
        assert_eq!(
            hidden_messages[0].extra["plan"]["created_at_ms"],
            serde_json::json!(2000)
        );
        assert_eq!(
            hidden_messages[0].extra["plan"]["supersedes"],
            serde_json::json!("old-plan-id")
        );
        assert_eq!(
            hidden_messages[0].extra["plan"]["truncated"],
            serde_json::json!(true)
        );
        assert_eq!(
            hidden_messages[0].extra["plan"]["original_chars"],
            serde_json::json!(12345)
        );
        assert_eq!(hidden_messages[1].message_id, "delta-1");
        assert_eq!(
            hidden_messages[1].content.content_text_only(),
            "first update"
        );
        assert_eq!(
            hidden_messages[1].extra["event"]["payload"],
            serde_json::json!({"seq": 1, "summary": "first summary"})
        );
        assert_eq!(hidden_messages[2].message_id, "delta-2");
        assert_eq!(
            hidden_messages[2].content.content_text_only(),
            "second update"
        );
        assert_eq!(
            hidden_messages[2].extra["event"]["payload"],
            serde_json::json!({"seq": 2, "summary": "second summary"})
        );
        assert!(!new_messages
            .iter()
            .any(|message| message.content.content_text_only() == "fallback"));
        assert!(!new_messages
            .iter()
            .any(|message| message.message_id == "other-event"));
    }

    #[tokio::test]
    async fn test_assemble_new_chat_includes_preserved_flag_messages() {
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText("current request ".repeat(2000)),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                tool_call_id: "call_plan".to_string(),
                content: ChatContent::SimpleText("important preserved plan".to_string()),
                preserve: Some(true),
                ..Default::default()
            },
        ];
        let new_messages = assemble_new_chat(&messages, &ParsedDecisions::default(), &[])
            .await
            .unwrap();
        let text = new_messages
            .iter()
            .map(|msg| msg.content.content_text_only())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("important preserved plan"));
    }

    #[test]
    fn test_find_finish_report() {
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
                        name: "finish".to_string(),
                        arguments: r#"{"report": "Detailed report here", "summary": "All done"}"#.to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                tool_call_id: "call_123".to_string(),
                content: ChatContent::SimpleText(
                    r#"{"type":"finish","summary":"All done","report":"Detailed report here","files_changed":["src/main.rs"]}"#.to_string()
                ),
                ..Default::default()
            },
        ];

        let report = find_finish_report(&messages);
        assert!(report.is_some());
        let report_text = report.unwrap();
        assert!(report_text.contains("**All done**"));
        assert!(report_text.contains("Detailed report here"));
        assert!(report_text.contains("`src/main.rs`"));
    }

    #[test]
    fn test_find_finish_report_no_finish() {
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

        let report = find_finish_report(&messages);
        assert!(report.is_none());
    }

    fn assistant_tool_call(call_id: &str, tool_name: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            tool_calls: Some(vec![ChatToolCall {
                id: call_id.to_string(),
                index: None,
                function: ChatToolFunction {
                    name: tool_name.to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }
    }

    fn tool_result(call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            tool_call_id: call_id.to_string(),
            ..Default::default()
        }
    }

    fn plan_role_message(version: u32, content: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "plan".to_string(),
            serde_json::json!({
                "mode": "agent",
                "version": version,
                "created_at_ms": 1000,
                "supersedes": null,
            }),
        );
        ChatMessage {
            role: "plan".to_string(),
            message_id: format!("plan-{version}"),
            content: ChatContent::SimpleText(content.to_string()),
            preserve: Some(true),
            extra,
            ..Default::default()
        }
    }

    #[test]
    fn parse_llm_response_parses_plan_source_msg_id() {
        let decisions = parse_llm_response(
            r#"
<summary>S</summary>
<plan_source>MSG_ID:3</plan_source>
<handoff_message>Continue</handoff_message>
"#,
        );
        assert_eq!(decisions.plan_source.as_deref(), Some("MSG_ID:3"));
    }

    #[test]
    fn parse_llm_response_empty_plan_source_is_none() {
        let decisions = parse_llm_response(
            "<summary>S</summary>\n<plan_source>\n</plan_source>\n<handoff_message>go</handoff_message>",
        );
        assert!(decisions.plan_source.is_none());
    }

    #[tokio::test]
    async fn assemble_new_chat_pins_plan_from_plan_source_reference() {
        let plan_body = "# Strategic Plan\n\n## Tasks\n- Build the thing\n- Verify: `cargo test`";
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText("plan this".to_string()),
                ..Default::default()
            },
            assistant_tool_call("call_plan", "plan"),
            tool_result("call_plan", plan_body),
        ];
        let decisions = ParsedDecisions {
            plan_source: Some("MSG_ID:2".to_string()),
            ..Default::default()
        };

        let new_messages = assemble_new_chat(&messages, &decisions, &[]).await.unwrap();
        let plan_messages: Vec<_> = new_messages
            .iter()
            .filter(|msg| msg.role == "plan")
            .collect();

        assert_eq!(plan_messages.len(), 1);
        assert_eq!(plan_messages[0].content.content_text_only(), plan_body);
        assert_eq!(plan_messages[0].preserve, Some(true));
    }

    #[tokio::test]
    async fn assemble_new_chat_existing_plan_wins_over_plan_source() {
        let messages = vec![
            plan_role_message(1, "EXISTING_BASE_PLAN"),
            assistant_tool_call("call_plan", "plan"),
            tool_result("call_plan", "PLAN_REPORT_BODY"),
        ];
        let decisions = ParsedDecisions {
            plan_source: Some("MSG_ID:2".to_string()),
            ..Default::default()
        };

        let new_messages = assemble_new_chat(&messages, &decisions, &[]).await.unwrap();
        let plan_messages: Vec<_> = new_messages
            .iter()
            .filter(|msg| msg.role == "plan")
            .collect();

        assert_eq!(plan_messages.len(), 1);
        assert_eq!(
            plan_messages[0].content.content_text_only(),
            "EXISTING_BASE_PLAN"
        );
    }

    #[tokio::test]
    async fn assemble_new_chat_auto_pins_single_plan_report() {
        let plan_body = "# Auto Plan\n- step one\n- step two";
        let messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText("go".to_string()),
                ..Default::default()
            },
            assistant_tool_call("call_plan", "plan"),
            tool_result("call_plan", plan_body),
        ];
        let decisions = ParsedDecisions::default();

        let new_messages = assemble_new_chat(&messages, &decisions, &[]).await.unwrap();
        let plan_messages: Vec<_> = new_messages
            .iter()
            .filter(|msg| msg.role == "plan")
            .collect();

        assert_eq!(plan_messages.len(), 1);
        assert_eq!(plan_messages[0].content.content_text_only(), plan_body);
    }

    #[tokio::test]
    async fn assemble_new_chat_does_not_auto_pin_ambiguous_plan_reports() {
        let messages = vec![
            assistant_tool_call("call_a", "plan"),
            tool_result("call_a", "FIRST PLAN REPORT"),
            assistant_tool_call("call_b", "task_done"),
            tool_result("call_b", "SECOND REPORT"),
        ];
        let decisions = ParsedDecisions::default();

        let new_messages = assemble_new_chat(&messages, &decisions, &[]).await.unwrap();
        assert!(!new_messages.iter().any(|msg| msg.role == "plan"));
    }

    #[test]
    fn carried_plan_messages_returns_existing_plan_with_deltas() {
        let delta = {
            let mut extra = serde_json::Map::new();
            extra.insert(
                "event".to_string(),
                serde_json::json!({
                    "subkind": "plan_delta",
                    "source": "tool.update_plan",
                    "payload": {"seq": 1},
                }),
            );
            ChatMessage {
                role: "event".to_string(),
                content: ChatContent::SimpleText("update".to_string()),
                extra,
                ..Default::default()
            }
        };
        let messages = vec![plan_role_message(2, "BASE"), delta];

        let carried = carried_plan_messages(&messages);
        assert_eq!(carried.len(), 2);
        assert_eq!(carried[0].role, "plan");
        assert_eq!(carried[0].content.content_text_only(), "BASE");
        assert_eq!(carried[1].role, "event");
    }

    #[test]
    fn carried_plan_messages_pins_single_report_when_no_plan_role() {
        let messages = vec![
            assistant_tool_call("call_plan", "plan"),
            tool_result("call_plan", "REPORT BODY"),
        ];

        let carried = carried_plan_messages(&messages);
        assert_eq!(carried.len(), 1);
        assert_eq!(carried[0].role, "plan");
        assert_eq!(carried[0].content.content_text_only(), "REPORT BODY");
        assert_eq!(carried[0].preserve, Some(true));
    }

    #[test]
    fn carried_plan_messages_empty_when_no_plan_artifact() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("hi".to_string()),
            ..Default::default()
        }];
        assert!(carried_plan_messages(&messages).is_empty());
    }

    #[test]
    fn format_annotated_messages_labels_tool_result_with_tool_name() {
        let messages = vec![
            assistant_tool_call("call_plan", "plan"),
            tool_result("call_plan", "the plan report"),
        ];
        let metadata = extract_conversation_metadata(&messages);
        let formatted = format_annotated_messages(&metadata);
        assert!(formatted.contains("[tool: plan]"), "{formatted}");
    }
}
