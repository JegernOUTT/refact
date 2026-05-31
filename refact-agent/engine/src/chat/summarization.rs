use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

use crate::call_validation::{ChatContent, ChatMessage, DiffChunk};
use crate::chat::diagnostics::{filter_ui_only_messages, is_ui_only_message};
use crate::chat::history_limit::{compute_context_budget, ContextPressure};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::types::{ChatEvent, ChatSession, CompressionPhase, CompressionReason, SessionState};
use crate::global_context::GlobalContext;
use crate::subchat::{run_subchat, SubchatConfig, ToolsPolicy};
use refact_chat_history::compression_exemption::{exemption_for, CompressionExemption};

pub const MAX_SEGMENT_SUMMARY_ATTEMPTS: usize = 2;
const SEGMENT_SUMMARY_OVERHEAD_TOKENS: usize = 1024;
const TOOL_CALL_ARGUMENTS_MAX_CHARS: usize = 1000;
const SEGMENT_MESSAGE_CONTENT_MAX_CHARS: usize = 6000;
const SEGMENT_REDACTION_SCAN_EXTRA_CHARS: usize = 4096;
const PUBLIC_COMPRESSION_FAILURE_MAX_CHARS: usize = 1024;
const GOAL_HINT_MAX_CHARS: usize = 4_000;
const GOAL_HINT_BUDGET_CUSHION_CHARS: usize = 256;
const CONTEXT_FILE_NAME_COMPONENT_MAX_CHARS: usize = 64;
const CONTEXT_FILE_NAME_MAX_CHARS: usize = 180;
const MESSAGE_CONTENT_TRUNCATED_MARKER: &str = "\n[... message content truncated ...]";
const TOOL_CALL_ARGUMENTS_TRUNCATED_MARKER: &str = "…";
const GOAL_HINT_TRUNCATED_MARKER: &str = "\n[... user goal truncated ...]";
const GOAL_HINT_PROMPT_PREFIX: &str = "User goal for this segment: ";
const SUMMARY_KIND: &str = "llm_segment_summary";
const SUMMARY_SCHEMA_VERSION: u64 = 2;

#[derive(Debug, Clone)]
pub enum SegmentSummaryFailure {
    NoModelAvailable,
    InputTooLarge {
        excerpt_chars: usize,
        budget_chars: usize,
    },
    NoMessagesToSummarize,
    EmptySummary,
    PressureTooLow,
    Transient(String),
}

impl std::fmt::Display for SegmentSummaryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SegmentSummaryFailure::NoModelAvailable => {
                write!(f, "no model available for segment summarization")
            }
            SegmentSummaryFailure::InputTooLarge {
                excerpt_chars,
                budget_chars,
            } => write!(
                f,
                "segment input too large after truncation: {} chars (budget {})",
                excerpt_chars, budget_chars
            ),
            SegmentSummaryFailure::NoMessagesToSummarize => write!(f, "no messages to summarize"),
            SegmentSummaryFailure::EmptySummary => {
                write!(f, "segment summarizer produced no assistant summary")
            }
            SegmentSummaryFailure::PressureTooLow => write!(f, "context pressure not high enough"),
            SegmentSummaryFailure::Transient(msg) => write!(f, "{}", msg),
        }
    }
}

impl SegmentSummaryFailure {
    pub fn is_structural(&self) -> bool {
        matches!(
            self,
            SegmentSummaryFailure::NoModelAvailable | SegmentSummaryFailure::InputTooLarge { .. }
        )
    }
}

fn public_compression_failure_text(failure: &SegmentSummaryFailure) -> String {
    let source = failure.to_string();
    let scan_cap =
        PUBLIC_COMPRESSION_FAILURE_MAX_CHARS.saturating_add(SEGMENT_REDACTION_SCAN_EXTRA_CHARS);
    let (window, truncated) = bounded_redaction_window(&source, scan_cap);
    let mut redacted = refact_core::string_utils::redact_sensitive(window);
    if truncated {
        if window.is_empty() {
            redacted.push_str(&omitted_long_token_marker(source.chars().count()));
        } else {
            redacted.push_str(MESSAGE_CONTENT_TRUNCATED_MARKER);
        }
    }
    if redacted.trim().is_empty() {
        redacted = "details omitted".to_string();
    }
    cap_redacted_message_content(&redacted, PUBLIC_COMPRESSION_FAILURE_MAX_CHARS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummarySegment {
    pub start: usize,
    pub end: usize,
}

fn safe_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

const SEGMENT_SUMMARY_PROMPT: &str =
    "Summarize the following non-user conversation segment for compact context storage. \
Minimize information loss. Every fact needed to continue the task must be preserved.

Use EXACTLY these Markdown sections (write \"(none)\" if a section has nothing):

## Current Task State
What was being worked on. Progress made. Current status or blocker.

## Key Files
Files EDITED, CREATED, or DELETED: exact path + what changed.
Files central to errors or needed next: path + why it matters.
Skip files only read without consequence.

## Decisions & Constraints
Explicit user requirements, approvals, rejections, constraints. Quote exact user instructions.
Confirmed assumptions and design decisions.

## Tool Outcomes
Results that changed state or revealed problems.
- Failed commands: exact error message and exit code
- Test failures: test name and failure message
- Successful writes/edits: file paths only
Format: `tool(args)` → result. Skip successful read-only operations.

## Dropped Context
One sentence: what was omitted (e.g. \"14 routine file reads omitted\").

Rules: include exact paths, error text, exit codes. No invented content. 150–500 words total.";

pub fn is_segment_summary(message: &ChatMessage) -> bool {
    if message.role != "assistant" || is_ui_only_message(message) {
        return false;
    }
    message
        .extra
        .get("compression")
        .and_then(|value| value.get("kind"))
        .and_then(|value| value.as_str())
        == Some(SUMMARY_KIND)
}

fn segment_summary_source_hash(message: &ChatMessage) -> Option<&str> {
    message
        .extra
        .get("compression")
        .and_then(|value| value.get("source_hash"))
        .and_then(|value| value.as_str())
}

fn is_excluded_from_segment(message: &ChatMessage) -> bool {
    if message.role == "system" || message.role == "user" || is_ui_only_message(message) {
        return true;
    }
    exemption_for(message) == CompressionExemption::Never
}

pub fn closed_non_user_segments(messages: &[ChatMessage]) -> Vec<SummarySegment> {
    let user_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(idx, msg)| (msg.role == "user").then_some(idx))
        .collect();
    if user_indices.len() < 2 {
        return Vec::new();
    }

    let mut segments = Vec::new();
    for pair in user_indices.windows(2) {
        let left_user = pair[0];
        let right_user = pair[1];
        let mut start = left_user + 1;
        while start < right_user && is_excluded_from_segment(&messages[start]) {
            start += 1;
        }
        let mut idx = start;
        while idx < right_user {
            if is_excluded_from_segment(&messages[idx]) {
                if start < idx {
                    segments.push(SummarySegment {
                        start,
                        end: idx - 1,
                    });
                }
                idx += 1;
                while idx < right_user && is_excluded_from_segment(&messages[idx]) {
                    idx += 1;
                }
                start = idx;
            } else {
                idx += 1;
            }
        }
        if start < right_user {
            segments.push(SummarySegment {
                start,
                end: right_user - 1,
            });
        }
    }

    segments
}

fn canonical_source_value(message: &ChatMessage) -> Value {
    let mut value = serde_json::to_value(message).unwrap_or_else(|_| json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.remove("message_id");
    }
    value
}

pub fn source_hash_for_messages(messages: &[ChatMessage]) -> String {
    let canonical: Vec<Value> = messages.iter().map(canonical_source_value).collect();
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn source_message_ids(messages: &[ChatMessage]) -> Vec<String> {
    messages
        .iter()
        .map(|message| message.message_id.clone())
        .collect()
}

fn segment_is_matching_summary(messages: &[ChatMessage], segment: SummarySegment) -> bool {
    segment.start == segment.end
        && is_segment_summary(&messages[segment.start])
        && segment_summary_source_hash(&messages[segment.start]).is_some()
}

fn assistant_finish_reason_requests_tools(message: &ChatMessage) -> bool {
    matches!(
        message.finish_reason.as_deref(),
        Some("tool_calls" | "function_call" | "tool_use")
    )
}

fn tail_has_pending_tool_calls(messages: &[ChatMessage]) -> bool {
    let mut completed_after_call = HashSet::new();
    for message in messages.iter().rev() {
        match message.role.as_str() {
            "tool" | "diff" if !message.tool_call_id.is_empty() => {
                completed_after_call.insert(message.tool_call_id.as_str());
            }
            "assistant" => {
                let Some(tool_calls) = message.tool_calls.as_ref() else {
                    if assistant_finish_reason_requests_tools(message) {
                        return true;
                    }
                    continue;
                };
                if assistant_finish_reason_requests_tools(message) && tool_calls.is_empty() {
                    return true;
                }
                if tool_calls
                    .iter()
                    .any(|tool_call| !completed_after_call.contains(tool_call.id.as_str()))
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn is_trimmable_tail_diagnostic(message: &ChatMessage) -> bool {
    is_excluded_from_segment(message) || matches!(message.role.as_str(), "event" | "error")
}

fn has_tail_summarizable_output(messages: &[ChatMessage]) -> bool {
    messages.iter().any(|message| {
        matches!(message.role.as_str(), "assistant" | "tool" | "diff")
            && !is_ui_only_message(message)
            && !message.content.content_text_only().trim().is_empty()
    })
}

pub fn eligible_tail_non_user_segment(messages: &[ChatMessage]) -> Option<SummarySegment> {
    let last_user = messages
        .iter()
        .rposition(|message| message.role == "user")?;
    let mut start = last_user + 1;
    while start < messages.len() && is_excluded_from_segment(&messages[start]) {
        start += 1;
    }
    if start >= messages.len() {
        return None;
    }

    let mut end = messages.len() - 1;
    while end >= start && is_trimmable_tail_diagnostic(&messages[end]) {
        if end == 0 {
            return None;
        }
        end -= 1;
    }
    if end < start {
        return None;
    }

    let tail = &messages[start..=end];
    if tail.iter().any(is_excluded_from_segment) {
        return None;
    }
    if !has_tail_summarizable_output(tail) {
        return None;
    }
    if tail_has_pending_tool_calls(tail) {
        return None;
    }

    let segment = SummarySegment { start, end };
    if segment_is_matching_summary(messages, segment) {
        return None;
    }
    Some(segment)
}

fn first_eligible_segment(messages: &[ChatMessage]) -> Option<SummarySegment> {
    closed_non_user_segments(messages)
        .into_iter()
        .find(|segment| !segment_is_matching_summary(messages, *segment))
        .or_else(|| eligible_tail_non_user_segment(messages))
}

fn estimated_context_pressure(messages: &[ChatMessage], effective_n_ctx: usize) -> ContextPressure {
    let visible_messages = filter_ui_only_messages(messages.to_vec());
    compute_context_budget(&visible_messages, effective_n_ctx).pressure
}

fn role_label(role: &str) -> &str {
    match role {
        "assistant" => "ASSISTANT",
        "tool" => "TOOL_RESULT",
        "diff" => "FILE_EDIT",
        "context_file" => "CONTEXT_FILE",
        "event" => "EVENT",
        "error" => "ERROR",
        "cd_instruction" => "INSTRUCTION",
        other => other,
    }
}

fn bounded_redacted_tool_arguments(arguments: &str) -> String {
    let scan_cap = TOOL_CALL_ARGUMENTS_MAX_CHARS.saturating_add(SEGMENT_REDACTION_SCAN_EXTRA_CHARS);
    let (window, truncated) = bounded_redaction_window(arguments, scan_cap);
    let mut redacted = refact_core::string_utils::redact_sensitive(window);
    if truncated {
        redacted.push_str(TOOL_CALL_ARGUMENTS_TRUNCATED_MARKER);
    }
    let truncated =
        refact_core::string_utils::safe_truncate(&redacted, TOOL_CALL_ARGUMENTS_MAX_CHARS);
    if truncated.len() == redacted.len() {
        truncated.to_string()
    } else {
        format!("{}{}", truncated, TOOL_CALL_ARGUMENTS_TRUNCATED_MARKER)
    }
}

fn is_redaction_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            ',' | ';' | ')' | ']' | '}' | '"' | '\'' | '`' | '<' | '>'
        )
}

fn bounded_redaction_window(text: &str, scan_cap: usize) -> (&str, bool) {
    if text.len() <= scan_cap {
        return (text, false);
    }

    let prefix = refact_core::string_utils::safe_truncate(text, scan_cap);
    if prefix
        .chars()
        .last()
        .map(is_redaction_boundary)
        .unwrap_or(true)
        || text[prefix.len()..]
            .chars()
            .next()
            .map(is_redaction_boundary)
            .unwrap_or(false)
    {
        return (prefix, true);
    }

    let end = prefix
        .char_indices()
        .rev()
        .find(|(_, ch)| is_redaction_boundary(*ch))
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);

    (&prefix[..end], true)
}

fn omitted_long_token_marker(omitted_chars: usize) -> String {
    format!("[long token omitted chars={}]", omitted_chars)
}

fn cap_redacted_message_content(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    if max_chars <= MESSAGE_CONTENT_TRUNCATED_MARKER.len() {
        return refact_core::string_utils::safe_truncate(
            MESSAGE_CONTENT_TRUNCATED_MARKER,
            max_chars,
        )
        .to_string();
    }
    let keep = max_chars - MESSAGE_CONTENT_TRUNCATED_MARKER.len();
    let prefix = refact_core::string_utils::safe_truncate(text, keep)
        .trim_end()
        .to_string();
    format!("{}{}", prefix, MESSAGE_CONTENT_TRUNCATED_MARKER)
}

fn redact_and_cap_message_content(text: &str) -> String {
    let scan_cap =
        SEGMENT_MESSAGE_CONTENT_MAX_CHARS.saturating_add(SEGMENT_REDACTION_SCAN_EXTRA_CHARS);
    let (window, truncated) = bounded_redaction_window(text, scan_cap);
    let mut redacted = refact_core::string_utils::redact_sensitive(window);
    if truncated {
        if window.is_empty() {
            redacted.push_str(&omitted_long_token_marker(text.chars().count()));
        } else {
            redacted.push_str(MESSAGE_CONTENT_TRUNCATED_MARKER);
        }
    }
    cap_redacted_message_content(&redacted, SEGMENT_MESSAGE_CONTENT_MAX_CHARS)
}

fn cap_goal_hint_with_marker(text: &str) -> String {
    if GOAL_HINT_MAX_CHARS <= GOAL_HINT_TRUNCATED_MARKER.len() {
        return refact_core::string_utils::safe_truncate(
            GOAL_HINT_TRUNCATED_MARKER,
            GOAL_HINT_MAX_CHARS,
        )
        .to_string();
    }
    let keep = GOAL_HINT_MAX_CHARS - GOAL_HINT_TRUNCATED_MARKER.len();
    let prefix = refact_core::string_utils::safe_truncate(text, keep)
        .trim_end()
        .to_string();
    format!("{}{}", prefix, GOAL_HINT_TRUNCATED_MARKER)
}

fn sanitize_goal_hint_text(raw: &str, source_truncated: bool) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() && !source_truncated {
        return None;
    }
    let scan_cap = GOAL_HINT_MAX_CHARS.saturating_add(SEGMENT_REDACTION_SCAN_EXTRA_CHARS);
    let (window, window_truncated) = bounded_redaction_window(trimmed, scan_cap);
    let was_truncated = source_truncated || window_truncated;
    let redacted = refact_core::string_utils::redact_sensitive(window)
        .trim()
        .to_string();
    if redacted.is_empty() && !was_truncated {
        return None;
    }
    let output = if was_truncated || redacted.len() > GOAL_HINT_MAX_CHARS {
        cap_goal_hint_with_marker(&redacted)
    } else {
        redacted
    };
    if output.trim().is_empty() {
        None
    } else {
        Some(output)
    }
}

fn sanitize_goal_hint(goal_hint: Option<String>) -> Option<String> {
    let raw = goal_hint?;
    sanitize_goal_hint_text(&raw, false)
}

fn append_goal_hint_text_piece(buffer: &mut String, piece: &str, scan_cap: usize) -> bool {
    if piece.is_empty() {
        return false;
    }
    if !buffer.is_empty() {
        let remaining = scan_cap.saturating_sub(buffer.len());
        if remaining < "\n\n".len() {
            if remaining > 0 {
                buffer.push_str(refact_core::string_utils::safe_truncate("\n\n", remaining));
            }
            return true;
        }
        buffer.push_str("\n\n");
    }
    let remaining = scan_cap.saturating_sub(buffer.len());
    if piece.len() <= remaining {
        buffer.push_str(piece);
        false
    } else {
        buffer.push_str(refact_core::string_utils::safe_truncate(piece, remaining));
        true
    }
}

fn bounded_goal_hint_from_message(message: &ChatMessage) -> Option<String> {
    if message.role != "user" {
        return None;
    }
    let scan_cap = GOAL_HINT_MAX_CHARS.saturating_add(SEGMENT_REDACTION_SCAN_EXTRA_CHARS);
    match &message.content {
        ChatContent::SimpleText(text) => {
            let (window, truncated) = bounded_redaction_window(text, scan_cap);
            sanitize_goal_hint_text(window, truncated)
        }
        ChatContent::Multimodal(elements) => {
            let mut text = String::with_capacity(scan_cap.min(1024));
            let mut truncated = false;
            for element in elements.iter().filter(|element| element.m_type == "text") {
                if append_goal_hint_text_piece(&mut text, &element.m_content, scan_cap) {
                    truncated = true;
                    break;
                }
            }
            sanitize_goal_hint_text(&text, truncated)
        }
        ChatContent::ContextFiles(_) => None,
    }
}

fn goal_hint_budget_overhead_chars(goal_hint: Option<&str>) -> usize {
    goal_hint
        .map(|hint| {
            GOAL_HINT_PROMPT_PREFIX
                .len()
                .saturating_add(hint.len())
                .saturating_add("\n\n".len())
                .saturating_add(GOAL_HINT_BUDGET_CUSHION_CHARS)
        })
        .unwrap_or(0)
}

fn segment_input_budget_chars(
    model_n_ctx: usize,
    max_new_tokens: usize,
    goal_hint: Option<&str>,
) -> usize {
    model_n_ctx
        .saturating_sub(max_new_tokens)
        .saturating_sub(SEGMENT_SUMMARY_OVERHEAD_TOKENS)
        .saturating_mul(3)
        .saturating_sub(goal_hint_budget_overhead_chars(goal_hint))
}

fn shorten_context_file_component(component: &str) -> String {
    if component.len() <= CONTEXT_FILE_NAME_COMPONENT_MAX_CHARS {
        return component.to_string();
    }
    let ext_len = component
        .rsplit_once('.')
        .map(|(_, ext)| ext.len().saturating_add(1))
        .filter(|len| *len <= 16)
        .unwrap_or(0);
    let suffix_budget = ext_len.min(CONTEXT_FILE_NAME_COMPONENT_MAX_CHARS / 2);
    let prefix_budget = CONTEXT_FILE_NAME_COMPONENT_MAX_CHARS.saturating_sub(suffix_budget + 1);
    let prefix = refact_core::string_utils::safe_truncate(component, prefix_budget);
    let suffix_start = safe_char_boundary(component, component.len().saturating_sub(suffix_budget));
    format!("{}…{}", prefix, &component[suffix_start..])
}

fn context_file_path_components(file_name: &str) -> Vec<&str> {
    file_name
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .collect()
}

fn sanitize_context_file_name(file_name: &str) -> String {
    let components = context_file_path_components(file_name)
        .into_iter()
        .map(refact_core::string_utils::redact_sensitive)
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>();
    if components.is_empty() {
        return "[redacted path]".to_string();
    }
    let keep_count = components.len().min(3);
    let start = components.len() - keep_count;
    let mut kept = components[start..]
        .iter()
        .map(|component| shorten_context_file_component(component))
        .collect::<Vec<_>>();
    if start > 0 || file_name.starts_with('/') || file_name.contains(":\\") {
        kept.insert(0, "…".to_string());
    }
    let short = kept.join("/");
    let capped = refact_core::string_utils::safe_truncate(&short, CONTEXT_FILE_NAME_MAX_CHARS);
    if capped.len() == short.len() {
        short
    } else {
        format!("{}…", capped.trim_end_matches('/'))
    }
}

fn segment_content_text(message: &ChatMessage) -> String {
    match &message.content {
        ChatContent::SimpleText(text) => redact_and_cap_message_content(text),
        ChatContent::Multimodal(elements) => elements
            .iter()
            .filter(|element| element.m_type == "text")
            .map(|element| redact_and_cap_message_content(&element.m_content))
            .collect::<Vec<_>>()
            .join("\n\n"),
        ChatContent::ContextFiles(files) => files
            .iter()
            .map(|file| {
                let content = redact_and_cap_message_content(&file.file_content);
                let file_name = sanitize_context_file_name(&file.file_name);
                format!("{}:{}-{}\n{}", file_name, file.line1, file.line2, content)
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

fn edited_file_names(messages: &[ChatMessage]) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for message in messages {
        if message.role == "diff" {
            match &message.content {
                ChatContent::ContextFiles(files) => {
                    for f in files {
                        names.insert(f.file_name.clone());
                    }
                }
                ChatContent::SimpleText(text) => {
                    if let Ok(chunks) = serde_json::from_str::<Vec<DiffChunk>>(text) {
                        for chunk in chunks {
                            names.insert(chunk.file_name);
                            if let Some(file_name_rename) = chunk.file_name_rename {
                                names.insert(file_name_rename);
                            }
                        }
                    }
                }
                ChatContent::Multimodal(_) => {}
            }
        }
    }
    names
}

fn segment_text(messages: &[ChatMessage]) -> String {
    let edited = edited_file_names(messages);
    messages
        .iter()
        .map(|message| {
            let content = segment_content_text(message);
            let importance_prefix = if message.role == "context_file" {
                if let ChatContent::ContextFiles(files) = &message.content {
                    if files.iter().any(|f| edited.contains(&f.file_name)) {
                        "[IMPORTANT] "
                    } else {
                        ""
                    }
                } else {
                    ""
                }
            } else {
                ""
            };
            let mut parts = vec![format!(
                "{}[{}]",
                importance_prefix,
                role_label(&message.role)
            )];
            if !message.tool_call_id.is_empty() {
                parts.push(format!("tool_call_id={}", message.tool_call_id));
            }
            if let Some(tool_calls) = &message.tool_calls {
                if !tool_calls.is_empty() {
                    let calls: Vec<String> = tool_calls
                        .iter()
                        .map(|call| {
                            format!(
                                "{}({}) args={}",
                                call.function.name,
                                call.id,
                                bounded_redacted_tool_arguments(&call.function.arguments)
                            )
                        })
                        .collect();
                    parts.push(format!("tool_calls={}", calls.join(", ")));
                }
            }
            format!("{}\n{}\n", parts.join(" "), content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn summarize_segment_text(
    gcx: Arc<GlobalContext>,
    text: String,
    model: String,
    model_n_ctx: usize,
    max_new_tokens: usize,
    goal_hint: Option<String>,
) -> Result<String, SegmentSummaryFailure> {
    let user_content = match goal_hint {
        Some(hint) if !hint.trim().is_empty() => {
            format!(
                "{}{}\n\nSummarize this segment:\n\n{}",
                GOAL_HINT_PROMPT_PREFIX,
                hint.trim(),
                text
            )
        }
        _ => format!("Summarize this segment:\n\n{}", text),
    };
    let summarize_messages = vec![
        ChatMessage::new("system".to_string(), SEGMENT_SUMMARY_PROMPT.to_string()),
        ChatMessage::new("user".to_string(), user_content),
    ];

    let config = SubchatConfig {
        tool_name: "segment_summarize".to_string(),
        stateful: false,
        autonomous_no_confirm: false,
        chat_id: None,
        title: None,
        parent_id: None,
        link_type: None,
        root_chat_id: None,
        tools: ToolsPolicy::None,
        max_steps: 1,
        prepend_system_prompt: false,
        wrap_up: None,
        task_meta: None,
        worktree: None,
        model,
        mode: "NO_TOOLS".to_string(),
        n_ctx: model_n_ctx,
        max_new_tokens,
        temperature: Some(0.0),
        reasoning_effort: None,
        parent_tool_call_id: None,
        parent_subchat_tx: None,
        abort_flag: None,
        subchat_depth: 0,
        buddy_meta: None,
    };

    let result = run_subchat(gcx, summarize_messages, config)
        .await
        .map_err(SegmentSummaryFailure::Transient)?;

    extract_non_empty_assistant_summary(&result.messages)
}

fn extract_non_empty_assistant_summary(
    messages: &[ChatMessage],
) -> Result<String, SegmentSummaryFailure> {
    let summary = messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant" && !is_ui_only_message(message))
        .map(|message| message.content.content_text_only())
        .unwrap_or_default();
    let summary = summary.trim().to_string();
    if summary.is_empty() {
        Err(SegmentSummaryFailure::EmptySummary)
    } else {
        Ok(summary)
    }
}

fn make_segment_summary_message(
    summary: String,
    source_messages: &[ChatMessage],
    summary_model: &str,
) -> ChatMessage {
    debug_assert!(!summary.trim().is_empty());
    let source_hash = source_hash_for_messages(source_messages);
    let source_ids = source_message_ids(source_messages);
    let created_at = chrono::Utc::now().to_rfc3339();
    let mut extra = serde_json::Map::new();
    extra.insert(
        "compression".to_string(),
        json!({
            "schema_version": SUMMARY_SCHEMA_VERSION,
            "kind": SUMMARY_KIND,
            "source_hash": source_hash,
            "source_message_ids": source_ids,
            "created_at": created_at,
            "summary_model": summary_model,
        }),
    );

    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "assistant".to_string(),
        content: ChatContent::SimpleText(summary),
        summarized_range: None,
        summarization_tier: Some(SUMMARY_KIND.to_string()),
        summarized_token_estimate: Some(crate::chat::trajectory_ops::approx_token_count(
            source_messages,
        )),
        extra,
        ..Default::default()
    }
}

async fn summarize_segment(
    gcx: Arc<GlobalContext>,
    messages: &[ChatMessage],
    model: String,
    model_n_ctx: usize,
    goal_hint: Option<String>,
) -> Result<ChatMessage, SegmentSummaryFailure> {
    let mut text = segment_text(messages);
    let goal_hint = sanitize_goal_hint(goal_hint);
    let max_new_tokens = (model_n_ctx / 4).min(6000).max(1024);
    let input_budget_chars =
        segment_input_budget_chars(model_n_ctx, max_new_tokens, goal_hint.as_deref());
    if input_budget_chars == 0 {
        return Err(SegmentSummaryFailure::InputTooLarge {
            excerpt_chars: text.len(),
            budget_chars: 0,
        });
    }
    if text.len() > input_budget_chars {
        let original_len = text.len();
        let head_keep = input_budget_chars * 2 / 3;
        let tail_keep = input_budget_chars.saturating_sub(head_keep + 200);
        let head_end = safe_char_boundary(&text, head_keep.min(text.len()));
        let tail_start_raw = text.len().saturating_sub(tail_keep);
        let tail_start = safe_char_boundary(&text, tail_start_raw);
        let head = text[..head_end].to_string();
        let tail = text[tail_start..].to_string();
        if head.len() + tail.len() + 200 > input_budget_chars && tail_keep == 0 {
            return Err(SegmentSummaryFailure::InputTooLarge {
                excerpt_chars: original_len,
                budget_chars: input_budget_chars,
            });
        }
        let elided = original_len.saturating_sub(head.len() + tail.len());
        text = format!(
            "{}\n\n[... {} chars elided to fit summarizer input budget ...]\n\n{}",
            head, elided, tail
        );
    }

    let summary = summarize_segment_text(
        gcx,
        text,
        model.clone(),
        model_n_ctx,
        max_new_tokens,
        goal_hint,
    )
    .await?;
    Ok(make_segment_summary_message(summary, messages, &model))
}

async fn resolve_summary_model(
    gcx: Arc<GlobalContext>,
    thread_model: &str,
) -> Result<(String, usize), SegmentSummaryFailure> {
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .map_err(|e| SegmentSummaryFailure::Transient(e.message.clone()))?;
    let mut candidates = Vec::new();
    for candidate in [
        thread_model,
        &caps.defaults.chat_light_model,
        &caps.defaults.chat_default_model,
    ] {
        if !candidate.is_empty() && !candidates.iter().any(|model| model == candidate) {
            candidates.push(candidate.to_string());
        }
    }

    for model in candidates {
        let Ok(model_rec) = crate::caps::resolve_chat_model(caps.clone(), &model) else {
            continue;
        };
        let model_n_ctx = if model_rec.base.n_ctx > 0 {
            model_rec.base.n_ctx
        } else {
            crate::chat::config::tokens().default_n_ctx
        };
        return Ok((model, model_n_ctx));
    }

    Err(SegmentSummaryFailure::NoModelAvailable)
}

fn effective_n_ctx_for_resolved_summary_model(
    model_n_ctx: usize,
    thread: &crate::chat::types::ThreadParams,
) -> usize {
    match thread.context_tokens_cap {
        Some(cap) if cap > 0 => cap.min(model_n_ctx),
        _ => model_n_ctx,
    }
}

fn last_visible_has_pending_tool_calls(messages: &[ChatMessage]) -> bool {
    messages
        .iter()
        .rev()
        .find(|message| !is_ui_only_message(message))
        .map(|message| {
            message.role == "assistant"
                && message
                    .tool_calls
                    .as_ref()
                    .map_or(false, |calls| !calls.is_empty())
        })
        .unwrap_or(false)
}

fn replace_segment(messages: &mut Vec<ChatMessage>, segment: SummarySegment, summary: ChatMessage) {
    messages.splice(segment.start..=segment.end, [summary]);
}

pub async fn summarize_oldest_segment_with_resolved_model(
    gcx: Arc<GlobalContext>,
    messages: &mut Vec<ChatMessage>,
    model: &str,
    model_n_ctx: usize,
) -> Result<bool, SegmentSummaryFailure> {
    if model.is_empty() {
        return Err(SegmentSummaryFailure::NoModelAvailable);
    }
    let Some(segment) = first_eligible_segment(messages) else {
        return Err(SegmentSummaryFailure::NoMessagesToSummarize);
    };
    let goal_hint = messages[..segment.start]
        .iter()
        .rev()
        .find_map(bounded_goal_hint_from_message);
    let source_messages = messages[segment.start..=segment.end].to_vec();
    let summary = summarize_segment(
        gcx,
        &source_messages,
        model.to_string(),
        model_n_ctx,
        goal_hint,
    )
    .await?;
    replace_segment(messages, segment, summary);
    Ok(true)
}

fn should_attempt_segment_summarization(
    thread: &crate::chat::types::ThreadParams,
    force: bool,
) -> bool {
    force || thread.auto_compact_enabled_effective()
}

fn emit_compression_status(
    session: &mut ChatSession,
    phase: CompressionPhase,
    reason: Option<CompressionReason>,
) {
    let is_compressing = matches!(
        phase,
        CompressionPhase::Checking | CompressionPhase::Running
    );
    session.is_compressing = is_compressing;
    session.runtime.is_compressing = is_compressing;
    session.compression_phase = Some(phase);
    session.compression_reason = reason;
    session.runtime.compression_phase = Some(phase);
    session.runtime.compression_reason = reason;
    let state = session.runtime.state;
    let error = session.runtime.error.clone();
    session.emit(ChatEvent::RuntimeUpdated {
        state,
        error,
        is_compressing,
        compression_phase: Some(phase),
        compression_reason: reason,
    });
}

fn emit_compression_running(session: &mut ChatSession) {
    emit_compression_status(session, CompressionPhase::Running, None);
}

fn emit_compression_applied(session: &mut ChatSession) {
    emit_compression_status(session, CompressionPhase::Applied, None);
}

fn emit_compression_skipped(session: &mut ChatSession, reason: CompressionReason) {
    emit_compression_status(session, CompressionPhase::Skipped, Some(reason));
}

fn emit_compression_failed(session: &mut ChatSession, reason: CompressionReason) {
    emit_compression_status(session, CompressionPhase::Failed, Some(reason));
}

fn compression_attempt_active(session: &ChatSession) -> bool {
    session.is_compressing
        || session.runtime.is_compressing
        || matches!(
            session.compression_phase,
            Some(CompressionPhase::Checking | CompressionPhase::Running)
        )
        || matches!(
            session.runtime.compression_phase,
            Some(CompressionPhase::Checking | CompressionPhase::Running)
        )
}

fn compression_failure_reason(failure: &SegmentSummaryFailure) -> CompressionReason {
    match failure {
        SegmentSummaryFailure::NoModelAvailable => CompressionReason::NoSummaryModel,
        SegmentSummaryFailure::InputTooLarge { .. } => CompressionReason::InputTooLarge,
        SegmentSummaryFailure::NoMessagesToSummarize => CompressionReason::NoEligibleSegment,
        SegmentSummaryFailure::PressureTooLow => CompressionReason::PressureLow,
        SegmentSummaryFailure::EmptySummary | SegmentSummaryFailure::Transient(_) => {
            CompressionReason::TransientFailure
        }
    }
}

pub fn summarize_oldest_segment_with_static_summary(
    messages: &mut Vec<ChatMessage>,
    summary_text: &str,
    summary_model: &str,
) -> bool {
    if summary_text.trim().is_empty() {
        return false;
    }
    let Some(segment) = first_eligible_segment(messages) else {
        return false;
    };
    let source_messages = messages[segment.start..=segment.end].to_vec();
    let summary =
        make_segment_summary_message(summary_text.to_string(), &source_messages, summary_model);
    replace_segment(messages, segment, summary);
    true
}

fn append_compression_failure_event(session: &mut ChatSession, failure: &SegmentSummaryFailure) {
    let public_failure = public_compression_failure_text(failure);
    let fail_event = event(
        EventSubkind::SystemNotice,
        "chat.summarizer",
        json!({ "failure": public_failure.clone() }),
        format!("Context compression failed: {}", public_failure),
    );
    let index = session.messages.len();
    session.messages.push(fail_event);
    let message = session.messages[index].clone();
    session.emit(ChatEvent::MessageAdded { message, index });
    session.increment_version();
    session.touch();
}

fn apply_resolved_segment_summary(
    session: &mut ChatSession,
    source_hash: &str,
    summary: ChatMessage,
) -> bool {
    if matches!(
        session.runtime.state,
        SessionState::Generating | SessionState::ExecutingTools
    ) || session.draft_message.is_some()
    {
        emit_compression_skipped(session, CompressionReason::NoEligibleSegment);
        return false;
    }
    let Some(current_segment) = first_eligible_segment(&session.messages) else {
        emit_compression_skipped(session, CompressionReason::NoEligibleSegment);
        return false;
    };
    let current_source = session.messages[current_segment.start..=current_segment.end].to_vec();
    if source_hash_for_messages(&current_source) != source_hash {
        warn!("Segment summarization skipped because source messages changed while summarizing");
        emit_compression_skipped(session, CompressionReason::SourceChanged);
        return false;
    }
    replace_segment(&mut session.messages, current_segment, summary);
    session.tier1_compact_attempts += 1;
    session.tier1_compaction_disabled = false;
    session.thread.previous_response_id = None;
    session.cache_guard_force_next = true;
    emit_compression_applied(session);
    session.increment_version();
    session.touch();
    let snapshot = session.snapshot();
    session.emit(snapshot);
    info!(
        "Segment summarization applied, messages count now {}",
        session.messages.len()
    );
    true
}

pub async fn apply_segment_summarization(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    thread: &crate::chat::types::ThreadParams,
    force: bool,
) -> bool {
    let raw_messages = {
        let mut session = session_arc.lock().await;
        if compression_attempt_active(&session) {
            return false;
        }
        if !should_attempt_segment_summarization(thread, force) {
            emit_compression_skipped(&mut session, CompressionReason::AutoCompactDisabled);
            return false;
        }
        emit_compression_status(&mut session, CompressionPhase::Checking, None);
        if session.tier1_compaction_disabled && !force {
            emit_compression_skipped(&mut session, CompressionReason::SessionCompactionDisabled);
            return false;
        }
        if session.tier1_compact_attempts >= MAX_SEGMENT_SUMMARY_ATTEMPTS && !force {
            emit_compression_skipped(&mut session, CompressionReason::MaxAttemptsReached);
            return false;
        }
        if matches!(
            session.runtime.state,
            SessionState::Generating | SessionState::ExecutingTools
        ) || session.draft_message.is_some()
        {
            emit_compression_skipped(&mut session, CompressionReason::NoEligibleSegment);
            return false;
        }
        if last_visible_has_pending_tool_calls(&session.messages) {
            emit_compression_skipped(&mut session, CompressionReason::PendingToolCalls);
            return false;
        }
        session.messages.clone()
    };

    let Some(segment) = first_eligible_segment(&raw_messages) else {
        let mut session = session_arc.lock().await;
        emit_compression_skipped(&mut session, CompressionReason::NoEligibleSegment);
        return false;
    };
    let (model, model_n_ctx) = match resolve_summary_model(gcx.clone(), &thread.model).await {
        Ok(value) => value,
        Err(failure) => {
            let mut session = session_arc.lock().await;
            if failure.is_structural() {
                session.tier1_compaction_disabled = true;
            } else {
                session.tier1_compact_attempts += 1;
            }
            warn!("Segment summarization failed before subchat: {}", failure);
            append_compression_failure_event(&mut session, &failure);
            emit_compression_failed(&mut session, compression_failure_reason(&failure));
            return false;
        }
    };
    let effective_n_ctx = effective_n_ctx_for_resolved_summary_model(model_n_ctx, thread);
    let pressure = estimated_context_pressure(&raw_messages, effective_n_ctx);
    if !force && !matches!(pressure, ContextPressure::High | ContextPressure::Critical) {
        let mut session = session_arc.lock().await;
        emit_compression_skipped(&mut session, CompressionReason::PressureLow);
        return false;
    }

    {
        let mut session = session_arc.lock().await;
        emit_compression_running(&mut session);
    }

    let source_messages = raw_messages[segment.start..=segment.end].to_vec();
    let source_hash = source_hash_for_messages(&source_messages);
    info!(
        "Segment summarization attempting messages {}..={} ({} msgs, source_hash={})",
        segment.start,
        segment.end,
        source_messages.len(),
        source_hash,
    );

    let goal_hint = raw_messages[..segment.start]
        .iter()
        .rev()
        .find_map(bounded_goal_hint_from_message);

    match summarize_segment(gcx, &source_messages, model, model_n_ctx, goal_hint).await {
        Ok(summary) => {
            let mut session = session_arc.lock().await;
            apply_resolved_segment_summary(&mut session, &source_hash, summary)
        }
        Err(failure) => {
            let mut session = session_arc.lock().await;
            if failure.is_structural() {
                session.tier1_compaction_disabled = true;
                warn!(
                    "Segment summarization structurally disabled for this session: {}",
                    failure
                );
            } else {
                session.tier1_compact_attempts += 1;
                warn!("Segment summarization failed: {}", failure);
            }
            append_compression_failure_event(&mut session, &failure);
            emit_compression_failed(&mut session, compression_failure_reason(&failure));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{
        ChatContent, ChatToolCall, ChatToolFunction, ContextFile, MultimodalElement,
    };
    use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
    use crate::global_context::tests::make_test_gcx;

    fn chat_model_record(id: &str, n_ctx: usize) -> Arc<ChatModelRecord> {
        Arc::new(ChatModelRecord {
            base: BaseModelRecord {
                id: id.to_string(),
                name: id.to_string(),
                n_ctx,
                endpoint: "https://example.com/v1/chat/completions".to_string(),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn install_caps(gcx: Arc<GlobalContext>, caps: CodeAssistantCaps) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_add(60);
        let mut caps_state = gcx.caps_state.write().await;
        caps_state.caps = Some(Arc::new(caps));
        caps_state.last_attempted_ts = now;
    }

    fn user(text: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn assistant(text: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn tool(text: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            tool_call_id: "call_1".to_string(),
            ..Default::default()
        }
    }

    fn context_file(text: &str) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn context_files(files: Vec<ContextFile>) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(files),
            ..Default::default()
        }
    }

    fn error_message(text: &str) -> ChatMessage {
        ChatMessage {
            role: "error".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn event(text: &str) -> ChatMessage {
        crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            "test.summarization",
            json!({}),
            text.to_string(),
        )
    }

    fn ui_only_event(text: &str) -> ChatMessage {
        let mut message = event(text);
        message.extra.insert("_ui_only".to_string(), json!(true));
        message
    }

    fn plan(text: &str) -> ChatMessage {
        ChatMessage {
            role: "plan".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn assistant_with_tool_call() -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            tool_calls: Some(vec![ChatToolCall {
                id: "call_1".to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: "shell".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }
    }

    fn assistant_with_tool_call_args(arguments: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            tool_calls: Some(vec![ChatToolCall {
                id: "call_args".to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: "shell".to_string(),
                    arguments: arguments.to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }
    }

    fn assistant_with_tool_call_id(id: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            tool_calls: Some(vec![ChatToolCall {
                id: id.to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: "shell".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            finish_reason: Some("tool_calls".to_string()),
            ..Default::default()
        }
    }

    fn tool_with_id(id: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            tool_call_id: id.to_string(),
            ..Default::default()
        }
    }

    fn context_file_named(file_name: &str, content: &str) -> ContextFile {
        ContextFile {
            file_name: file_name.to_string(),
            file_content: content.to_string(),
            line1: 10,
            line2: 20,
            ..Default::default()
        }
    }

    fn assert_no_raw_secrets(text: &str) {
        assert!(
            !text.contains("sk-abcdefghijklmnop"),
            "sk token leaked: {text}"
        );
        assert!(
            !text.contains("secret-bearer-value"),
            "bearer leaked: {text}"
        );
    }

    #[test]
    fn goal_hint_is_redacted_and_bounded() {
        let raw = format!(
            "  Keep working with api_key=sk-abcdefghijklmnop and Bearer secret-bearer-value. {}  ",
            "tail ".repeat(GOAL_HINT_MAX_CHARS)
        );

        let hint = sanitize_goal_hint(Some(raw)).unwrap();

        assert!(hint.len() <= GOAL_HINT_MAX_CHARS, "len={}", hint.len());
        assert!(hint.contains("api_key=[REDACTED]") || hint.contains("[REDACTED_SK_TOKEN]"));
        assert!(hint.contains("Bearer [REDACTED]"));
        assert!(hint.ends_with(GOAL_HINT_TRUNCATED_MARKER));
        assert_no_raw_secrets(&hint);
    }

    #[test]
    fn empty_goal_hint_is_removed() {
        assert_eq!(sanitize_goal_hint(Some(" \n\t ".to_string())), None);
        assert_eq!(sanitize_goal_hint(None), None);
    }

    #[test]
    fn goal_hint_from_large_user_message_is_bounded_before_use() {
        let huge = format!(
            "Keep working with api_key=sk-abcdefghijklmnop. {} end",
            "tail ".repeat(GOAL_HINT_MAX_CHARS * 10)
        );
        let huge_len = huge.len();
        let message = user(&huge);

        let hint = bounded_goal_hint_from_message(&message).unwrap();

        assert!(hint.len() <= GOAL_HINT_MAX_CHARS, "len={}", hint.len());
        assert!(hint.contains("api_key=[REDACTED]") || hint.contains("[REDACTED_SK_TOKEN]"));
        assert!(hint.ends_with(GOAL_HINT_TRUNCATED_MARKER));
        assert_no_raw_secrets(&hint);
        assert!(!hint.contains(" end"));
        assert!(hint.len() < huge_len);
    }

    #[test]
    fn goal_hint_from_multimodal_user_message_uses_text_only() {
        let message = ChatMessage {
            role: "user".to_string(),
            content: ChatContent::Multimodal(vec![
                MultimodalElement {
                    m_type: "text".to_string(),
                    m_content: "visible api_key=sk-abcdefghijklmnop".to_string(),
                },
                MultimodalElement {
                    m_type: "image/png".to_string(),
                    m_content: "sk-image-secret-should-not-appear".to_string(),
                },
            ]),
            ..Default::default()
        };

        let hint = bounded_goal_hint_from_message(&message).unwrap();

        assert!(
            hint.contains("visible api_key=[REDACTED]") || hint.contains("[REDACTED_SK_TOKEN]")
        );
        assert!(!hint.contains("sk-abcdefghijklmnop"));
        assert!(!hint.contains("sk-image-secret-should-not-appear"));
    }

    #[test]
    fn goal_hint_ignores_context_file_user_content() {
        let message = ChatMessage {
            role: "user".to_string(),
            content: ChatContent::ContextFiles(vec![context_file_named(
                "secret.png",
                "base64-like sk-image-secret-should-not-appear",
            )]),
            ..Default::default()
        };

        assert_eq!(bounded_goal_hint_from_message(&message), None);
    }

    #[test]
    fn goal_hint_budget_is_subtracted_from_segment_budget() {
        let max_new_tokens = 1_024;
        let no_hint_budget = segment_input_budget_chars(8_000, max_new_tokens, None);
        let hint = sanitize_goal_hint(Some("preserve edited files ".repeat(400))).unwrap();
        let hint_budget = segment_input_budget_chars(8_000, max_new_tokens, Some(&hint));

        assert!(hint.len() <= GOAL_HINT_MAX_CHARS);
        assert!(hint_budget < no_hint_budget);
        assert_eq!(
            hint_budget,
            no_hint_budget.saturating_sub(goal_hint_budget_overhead_chars(Some(&hint)))
        );
        assert_eq!(segment_input_budget_chars(1, 1_024, Some(&hint)), 0);
    }

    #[test]
    fn closed_segments_adjacent_users_has_no_segment() {
        let messages = vec![user("a"), user("b")];
        assert!(closed_non_user_segments(&messages).is_empty());
    }

    #[test]
    fn closed_segments_tail_non_user_run_is_not_included() {
        let messages = vec![user("a"), assistant("old"), user("b"), assistant("tail")];
        assert_eq!(
            closed_non_user_segments(&messages),
            vec![SummarySegment { start: 1, end: 1 }]
        );
    }

    #[test]
    fn closed_segments_include_event_tool_context_file_inside_run() {
        let messages = vec![
            user("a"),
            assistant_with_tool_call(),
            tool("result"),
            event("notice"),
            context_file("file"),
            user("b"),
        ];
        assert_eq!(
            closed_non_user_segments(&messages),
            vec![SummarySegment { start: 1, end: 4 }]
        );
    }

    #[test]
    fn closed_segments_never_include_user_messages() {
        let messages = vec![
            user("a"),
            assistant("x"),
            user("b"),
            assistant("y"),
            user("c"),
        ];
        for segment in closed_non_user_segments(&messages) {
            assert!(!messages[segment.start..=segment.end]
                .iter()
                .any(|message| message.role == "user"));
        }
    }

    #[test]
    fn closed_segments_skip_plan_role_inside_closed_run() {
        let messages = vec![
            user("a"),
            assistant("x"),
            plan("sacred"),
            assistant("y"),
            user("b"),
        ];
        assert_eq!(
            closed_non_user_segments(&messages),
            vec![
                SummarySegment { start: 1, end: 1 },
                SummarySegment { start: 3, end: 3 },
            ]
        );
    }

    #[test]
    fn tail_segment_one_user_huge_assistant_is_eligible() {
        let huge = "assistant output ".repeat(1000);
        let mut messages = vec![user("summarize this"), assistant(&huge)];

        assert_eq!(
            eligible_tail_non_user_segment(&messages),
            Some(SummarySegment { start: 1, end: 1 })
        );
        assert!(summarize_oldest_segment_with_static_summary(
            &mut messages,
            "compressed tail",
            "test-model",
        ));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert!(is_segment_summary(&messages[1]));
        assert_eq!(messages[1].content.content_text_only(), "compressed tail");
    }

    #[test]
    fn tail_segment_trims_trailing_ui_only_diagnostic() {
        let mut messages = vec![
            user("summarize this"),
            assistant("assistant output"),
            ui_only_event("diagnostic"),
        ];

        assert_eq!(
            eligible_tail_non_user_segment(&messages),
            Some(SummarySegment { start: 1, end: 1 })
        );
        assert!(summarize_oldest_segment_with_static_summary(
            &mut messages,
            "compressed tail",
            "test-model",
        ));
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert!(is_segment_summary(&messages[1]));
        assert_eq!(messages[2].role, "event");
    }

    #[test]
    fn tail_segment_pending_assistant_tool_call_is_not_eligible() {
        let mut pending = assistant_with_tool_call();
        pending.finish_reason = Some("tool_calls".to_string());
        let mut messages = vec![user("run tool"), pending];

        assert_eq!(eligible_tail_non_user_segment(&messages), None);
        assert!(!summarize_oldest_segment_with_static_summary(
            &mut messages,
            "should not apply",
            "test-model",
        ));
    }

    #[test]
    fn tail_segment_empty_tool_calls_finish_reason_is_not_eligible() {
        let pending = ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            finish_reason: Some("tool_calls".to_string()),
            tool_calls: Some(Vec::new()),
            ..Default::default()
        };
        let mut messages = vec![user("run tool"), pending];

        assert_eq!(eligible_tail_non_user_segment(&messages), None);
        assert!(!summarize_oldest_segment_with_static_summary(
            &mut messages,
            "should not apply",
            "test-model",
        ));
    }

    #[test]
    fn tail_segment_prior_tool_result_does_not_satisfy_pending_call() {
        let mut messages = vec![
            user("run tool"),
            tool_with_id("call_later", "stale result"),
            assistant_with_tool_call_id("call_later"),
        ];

        assert_eq!(eligible_tail_non_user_segment(&messages), None);
        assert!(!summarize_oldest_segment_with_static_summary(
            &mut messages,
            "should not apply",
            "test-model",
        ));
    }

    #[test]
    fn tail_segment_completed_tool_call_is_eligible() {
        let mut assistant_call = assistant_with_tool_call();
        assistant_call.finish_reason = Some("tool_calls".to_string());
        let mut messages = vec![user("run tool"), assistant_call, tool("result")];

        assert_eq!(
            eligible_tail_non_user_segment(&messages),
            Some(SummarySegment { start: 1, end: 2 })
        );
        assert!(summarize_oldest_segment_with_static_summary(
            &mut messages,
            "tool turn summary",
            "test-model",
        ));
        assert_eq!(messages.len(), 2);
        assert!(is_segment_summary(&messages[1]));
    }

    #[test]
    fn tail_segment_ui_only_event_is_not_eligible() {
        let mut messages = vec![user("hello"), event("just ui")];

        assert_eq!(eligible_tail_non_user_segment(&messages), None);
        assert!(!summarize_oldest_segment_with_static_summary(
            &mut messages,
            "should not apply",
            "test-model",
        ));
    }

    #[test]
    fn tail_segment_user_context_file_only_is_not_eligible() {
        let mut messages = vec![user("look here"), context_file("file context")];

        assert_eq!(eligible_tail_non_user_segment(&messages), None);
        assert!(!summarize_oldest_segment_with_static_summary(
            &mut messages,
            "should not apply",
            "test-model",
        ));
    }

    #[test]
    fn closed_segment_is_preferred_before_tail_segment() {
        let mut messages = vec![
            user("first"),
            assistant("closed old"),
            user("second"),
            assistant("tail new"),
        ];

        assert_eq!(
            eligible_tail_non_user_segment(&messages),
            Some(SummarySegment { start: 3, end: 3 })
        );
        assert_eq!(
            first_eligible_segment(&messages),
            Some(SummarySegment { start: 1, end: 1 })
        );
        assert!(summarize_oldest_segment_with_static_summary(
            &mut messages,
            "closed summary",
            "test-model",
        ));
        assert_eq!(messages[1].content.content_text_only(), "closed summary");
        assert_eq!(messages[3].content.content_text_only(), "tail new");
    }

    #[test]
    fn tail_segment_existing_summary_is_not_summarized_again() {
        let mut messages = vec![user("first"), assistant("tail")];
        assert!(summarize_oldest_segment_with_static_summary(
            &mut messages,
            "tail summary",
            "test-model",
        ));
        let once = serde_json::to_string(&messages).unwrap();

        assert_eq!(eligible_tail_non_user_segment(&messages), None);
        assert!(!summarize_oldest_segment_with_static_summary(
            &mut messages,
            "tail summary changed",
            "test-model",
        ));
        assert_eq!(serde_json::to_string(&messages).unwrap(), once);
    }

    #[test]
    fn ui_only_diagnostic_between_users_is_not_eligible_for_visible_summary() {
        let messages = vec![
            user("first"),
            crate::chat::diagnostics::make_ui_only_error_message("context_length_exceeded"),
            user("second"),
        ];

        assert!(closed_non_user_segments(&messages).is_empty());
        assert!(!summarize_oldest_segment_with_static_summary(
            &mut messages.clone(),
            "summary",
            "test-model",
        ));
    }

    #[test]
    fn assistant_tool_call_args_are_included_bounded_and_redacted_in_segment_text() {
        let long_tail = "x".repeat(TOOL_CALL_ARGUMENTS_MAX_CHARS + 200);
        let args = format!(
            "{{\"cmd\":\"sed -n '1,20p' src/foo.rs\",\"api_key\":\"sk-abcdefghijklmnop\",\"tail\":\"{}\"}}",
            long_tail
        );
        let text = segment_text(&[assistant_with_tool_call_args(&args)]);

        assert!(text.contains("shell(call_args) args="));
        assert!(text.contains("sed -n '1,20p' src/foo.rs"));
        assert!(text.contains("[REDACTED_SK_TOKEN]") || text.contains("api_key=[REDACTED]"));
        assert!(!text.contains("sk-abcdefghijklmnop"));
        assert!(text.contains('…'));
        assert!(text.len() < args.len());
    }

    #[test]
    fn huge_tool_call_args_are_windowed_before_redaction_and_still_redact_secrets() {
        let early_secret = "api_key=sk-abcdefghijklmnop";
        let huge_tail = "a".repeat(TOOL_CALL_ARGUMENTS_MAX_CHARS * 100);
        let args = format!(
            "{{\"cmd\":\"run\",\"{}\",\"tail\":\"{}\"}}",
            early_secret, huge_tail
        );
        let text = segment_text(&[assistant_with_tool_call_args(&args)]);

        assert!(text.contains("shell(call_args) args="));
        assert!(text.contains("api_key=[REDACTED]") || text.contains("[REDACTED_SK_TOKEN]"));
        assert!(!text.contains("sk-abcdefghijklmnop"));
        assert!(text.contains(TOOL_CALL_ARGUMENTS_TRUNCATED_MARKER));
        assert!(text.len() <= TOOL_CALL_ARGUMENTS_MAX_CHARS + 128);
    }

    #[test]
    fn message_content_is_redacted_for_segment_text_roles() {
        let messages = vec![
            assistant("assistant saw sk-abcdefghijklmnop and Bearer secret-bearer-value"),
            tool("tool result returned Bearer sk-abcdefghijklmnop"),
            context_file("context simple text has token=sk-abcdefghijklmnop"),
            error_message("error included api_key=sk-abcdefghijklmnop"),
        ];
        let text = segment_text(&messages);

        assert!(text.contains("[ASSISTANT]"));
        assert!(text.contains("[TOOL_RESULT] tool_call_id=call_1"));
        assert!(text.contains("[CONTEXT_FILE]"));
        assert!(text.contains("[ERROR]"));
        assert!(text.contains("[REDACTED_SK_TOKEN]") || text.contains("[REDACTED]"));
        assert_no_raw_secrets(&text);
    }

    #[test]
    fn segment_text_labels_diff_as_file_edit() {
        let diff_msg = ChatMessage {
            role: "diff".to_string(),
            content: ChatContent::SimpleText("diff content".to_string()),
            ..Default::default()
        };
        let text = segment_text(&[diff_msg]);
        assert!(text.contains("[FILE_EDIT]"), "got: {text}");
        assert!(
            !text.contains("[TOOL]"),
            "should not have old label: {text}"
        );
    }

    #[test]
    fn segment_text_marks_context_file_for_simple_text_diff_as_important() {
        let diff = DiffChunk {
            file_name: "src/edited.rs".to_string(),
            file_action: "edit".to_string(),
            line1: 1,
            line2: 1,
            lines_remove: "old".to_string(),
            lines_add: "new".to_string(),
            application_details: String::new(),
            ..Default::default()
        };
        let diff_msg = ChatMessage {
            role: "diff".to_string(),
            content: ChatContent::SimpleText(json!([diff]).to_string()),
            ..Default::default()
        };
        let important_context = context_files(vec![context_file_named("src/edited.rs", "edited")]);
        let routine_context = context_files(vec![context_file_named("src/read_only.rs", "read")]);

        let text = segment_text(&[diff_msg, important_context, routine_context]);

        assert!(text.contains("[IMPORTANT] [CONTEXT_FILE]\nsrc/edited.rs:10-20"));
        assert!(text.contains("\n[CONTEXT_FILE]\nsrc/read_only.rs:10-20"));
    }

    #[test]
    fn structured_context_file_content_is_redacted_and_bounded() {
        let huge_tail = "x".repeat(SEGMENT_MESSAGE_CONTENT_MAX_CHARS * 2);
        let messages = vec![context_files(vec![context_file_named(
            "src/secret.rs",
            &format!("prefix token=sk-abcdefghijklmnop\n{}", huge_tail),
        )])];
        let text = segment_text(&messages);

        assert!(text.contains("src/secret.rs:10-20"));
        assert!(text.contains(MESSAGE_CONTENT_TRUNCATED_MARKER));
        assert!(text.contains("token=[REDACTED]") || text.contains("[REDACTED_SK_TOKEN]"));
        assert!(!text.contains("sk-abcdefghijklmnop"));
        assert!(text.len() < huge_tail.len());
    }

    #[test]
    fn structured_context_file_name_is_shortened_and_redacted() {
        let file_name = "/home/alice/projects/customer-token=secret-bearer-value/deep/private/sk-abcdefghijklmnop/very_long_component_name_that_should_not_be_fully_preserved_because_it_is_noisy.rs";
        let messages = vec![context_files(vec![context_file_named(
            file_name,
            "safe body",
        )])];
        let text = segment_text(&messages);

        assert!(text.contains("[CONTEXT_FILE]"));
        assert!(text.contains("very_long_component_name_that_should_not_be_fully"));
        assert!(text.contains(":10-20"));
        assert!(!text.contains("because_it_is_noisy.rs"));
        assert!(text.contains("…/"));
        assert!(!text.contains("/home/alice"));
        assert!(!text.contains("customer-token=secret-bearer-value"));
        assert!(!text.contains("sk-abcdefghijklmnop"));
        assert!(text.len() < file_name.len() + 64);
    }

    #[test]
    fn multimodal_text_content_is_redacted_and_image_content_is_ignored() {
        let message = ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::Multimodal(vec![
                MultimodalElement {
                    m_type: "text".to_string(),
                    m_content: "visible token=sk-abcdefghijklmnop".to_string(),
                },
                MultimodalElement {
                    m_type: "image/png".to_string(),
                    m_content: "sk-image-secret-should-not-appear".to_string(),
                },
            ]),
            ..Default::default()
        };
        let text = segment_text(&[message]);

        assert!(text.contains("visible token=[REDACTED]") || text.contains("[REDACTED_SK_TOKEN]"));
        assert!(!text.contains("sk-abcdefghijklmnop"));
        assert!(!text.contains("sk-image-secret-should-not-appear"));
    }

    #[test]
    fn large_message_content_is_capped_before_segment_concatenation() {
        let huge = format!(
            "start {} end",
            "0123456789".repeat(SEGMENT_MESSAGE_CONTENT_MAX_CHARS)
        );
        let text = segment_text(&[assistant(&huge)]);

        assert!(text.contains("start"));
        assert!(text.contains(MESSAGE_CONTENT_TRUNCATED_MARKER));
        assert!(!text.contains("0123456789"));
        assert!(!text.contains(" end"));
        assert!(text.len() < huge.len());
        assert!(text.len() <= SEGMENT_MESSAGE_CONTENT_MAX_CHARS + 64);
    }

    #[test]
    fn long_no_boundary_message_content_gets_omission_marker() {
        let huge = "z".repeat(SEGMENT_MESSAGE_CONTENT_MAX_CHARS * 3);
        let text = segment_text(&[assistant(&huge)]);

        assert!(text.contains("[long token omitted chars="));
        assert!(!text.contains(MESSAGE_CONTENT_TRUNCATED_MARKER));
        assert!(text.len() < 128);
    }

    #[test]
    fn empty_or_missing_assistant_summary_is_an_error() {
        let empty = vec![assistant("   ")];
        assert!(matches!(
            extract_non_empty_assistant_summary(&empty),
            Err(SegmentSummaryFailure::EmptySummary)
        ));

        let missing = vec![tool("tool-only result")];
        assert!(matches!(
            extract_non_empty_assistant_summary(&missing),
            Err(SegmentSummaryFailure::EmptySummary)
        ));
    }

    #[test]
    fn static_summary_rejects_empty_placeholder_loss() {
        let mut messages = vec![user("a"), assistant("old"), user("b")];
        let before = serde_json::to_string(&messages).unwrap();

        assert!(!summarize_oldest_segment_with_static_summary(
            &mut messages,
            " ",
            "test-model",
        ));

        assert_eq!(serde_json::to_string(&messages).unwrap(), before);
        assert!(!messages
            .iter()
            .any(|message| message.content.content_text_only() == "Summary unavailable"));
    }

    #[test]
    fn static_summary_preserves_user_messages_byte_identically() {
        let mut messages = vec![
            user("first exact bytes"),
            assistant("old answer"),
            tool("tool result"),
            user("second exact bytes"),
            assistant("tail answer"),
        ];
        let before_users: Vec<String> = messages
            .iter()
            .filter(|message| message.role == "user")
            .map(|message| serde_json::to_string(message).unwrap())
            .collect();

        assert!(summarize_oldest_segment_with_static_summary(
            &mut messages,
            "summary",
            "test-model",
        ));

        let after_users: Vec<String> = messages
            .iter()
            .filter(|message| message.role == "user")
            .map(|message| serde_json::to_string(message).unwrap())
            .collect();
        assert_eq!(after_users, before_users);
    }

    #[test]
    fn static_summary_creates_assistant_compression_message() {
        let mut messages = vec![user("a"), assistant("old"), user("b")];
        assert!(summarize_oldest_segment_with_static_summary(
            &mut messages,
            "summary",
            "test-model",
        ));

        assert_eq!(messages[1].role, "assistant");
        assert!(is_segment_summary(&messages[1]));
        let compression = messages[1].extra.get("compression").unwrap();
        assert_eq!(compression["schema_version"], json!(2));
        assert_eq!(compression["kind"], json!(SUMMARY_KIND));
        assert_eq!(compression["summary_model"], json!("test-model"));
    }

    #[test]
    fn static_summary_is_idempotent_for_existing_summary_segment() {
        let mut messages = vec![user("a"), assistant("old"), user("b")];
        assert!(summarize_oldest_segment_with_static_summary(
            &mut messages,
            "summary",
            "test-model",
        ));
        let once = serde_json::to_string(&messages).unwrap();

        assert!(!summarize_oldest_segment_with_static_summary(
            &mut messages,
            "summary changed",
            "test-model",
        ));
        let twice = serde_json::to_string(&messages).unwrap();

        assert_eq!(twice, once);
    }

    #[test]
    fn static_summary_has_no_current_history_range_anchor() {
        let mut messages = vec![user("a"), assistant("old"), user("b")];
        assert!(summarize_oldest_segment_with_static_summary(
            &mut messages,
            "summary",
            "test-model",
        ));

        assert_eq!(messages[1].summarized_range, None);
        assert_eq!(
            messages[1].summarization_tier,
            Some(SUMMARY_KIND.to_string())
        );
    }

    #[test]
    fn static_summary_then_linearize_preserves_users_and_summary() {
        let mut messages = vec![
            user("first exact bytes"),
            assistant("old answer"),
            tool("tool result"),
            user("second exact bytes"),
        ];

        assert!(summarize_oldest_segment_with_static_summary(
            &mut messages,
            "summary",
            "test-model",
        ));
        let result = crate::chat::linearize::apply_summarization_linearize(messages);
        let text: Vec<String> = result
            .iter()
            .map(|message| message.content.content_text_only())
            .collect();
        let roles: Vec<String> = result.iter().map(|message| message.role.clone()).collect();

        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert_eq!(
            text,
            vec!["first exact bytes", "summary", "second exact bytes"]
        );
    }

    #[test]
    fn source_hash_ignores_message_id() {
        let mut left = assistant("same");
        left.message_id = "left".to_string();
        let mut right = assistant("same");
        right.message_id = "right".to_string();

        assert_eq!(
            source_hash_for_messages(&[left]),
            source_hash_for_messages(&[right])
        );
    }

    #[test]
    fn pressure_check_can_be_low() {
        let messages = vec![user("hello"), assistant("hi"), user("again")];
        assert!(matches!(
            estimated_context_pressure(&messages, 1_000_000),
            ContextPressure::Low
        ));
    }

    #[test]
    fn forced_context_limit_summarization_bypasses_auto_compact_disabled_gate() {
        let mut thread = crate::chat::types::ThreadParams::default();
        thread.auto_compact_enabled = Some(false);

        assert!(!should_attempt_segment_summarization(&thread, false));
        assert!(should_attempt_segment_summarization(&thread, true));
    }

    #[test]
    fn emit_compression_checking_sets_active_session_runtime_and_event() {
        let mut session = ChatSession::new("compression-checking".to_string());
        let mut rx = session.subscribe();

        emit_compression_status(&mut session, CompressionPhase::Checking, None);

        assert!(session.is_compressing);
        assert!(session.runtime.is_compressing);
        assert_eq!(session.compression_phase, Some(CompressionPhase::Checking));
        assert_eq!(session.runtime.compression_phase, Some(CompressionPhase::Checking));
        assert_eq!(session.compression_reason, None);
        assert_eq!(session.runtime.compression_reason, None);
        let json = rx.try_recv().unwrap();
        let envelope: crate::chat::types::EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::RuntimeUpdated {
                is_compressing,
                compression_phase,
                compression_reason,
                ..
            } => {
                assert!(is_compressing);
                assert_eq!(compression_phase, Some(CompressionPhase::Checking));
                assert_eq!(compression_reason, None);
            }
            other => panic!("expected RuntimeUpdated, got {other:?}"),
        }
    }

    #[test]
    fn emit_compression_runtime_sets_session_and_runtime_flags() {
        let mut session = ChatSession::new("compression-runtime".to_string());
        let mut rx = session.subscribe();

        emit_compression_running(&mut session);

        assert!(session.is_compressing);
        assert!(session.runtime.is_compressing);
        assert_eq!(session.compression_phase, Some(CompressionPhase::Running));
        assert_eq!(session.compression_reason, None);
        let json = rx.try_recv().unwrap();
        let envelope: crate::chat::types::EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::RuntimeUpdated {
                is_compressing,
                compression_phase,
                compression_reason,
                ..
            } => {
                assert!(is_compressing);
                assert_eq!(compression_phase, Some(CompressionPhase::Running));
                assert_eq!(compression_reason, None);
            }
            other => panic!("expected RuntimeUpdated, got {other:?}"),
        }
    }

    fn assert_emitted_runtime_status(
        rx: &mut tokio::sync::broadcast::Receiver<Arc<String>>,
        expected_is_compressing: bool,
        expected_phase: CompressionPhase,
        expected_reason: Option<CompressionReason>,
    ) {
        let json = rx.try_recv().unwrap();
        let envelope: crate::chat::types::EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::RuntimeUpdated {
                is_compressing,
                compression_phase,
                compression_reason,
                ..
            } => {
                assert_eq!(is_compressing, expected_is_compressing);
                assert_eq!(compression_phase, Some(expected_phase));
                assert_eq!(compression_reason, expected_reason);
            }
            other => panic!("expected RuntimeUpdated, got {other:?}"),
        }
    }

    #[test]
    fn emit_compression_skipped_records_reason() {
        let mut session = ChatSession::new("compression-skipped".to_string());
        let mut rx = session.subscribe();

        emit_compression_skipped(&mut session, CompressionReason::PressureLow);

        assert!(!session.is_compressing);
        assert!(!session.runtime.is_compressing);
        assert_eq!(session.compression_phase, Some(CompressionPhase::Skipped));
        assert_eq!(
            session.compression_reason,
            Some(CompressionReason::PressureLow)
        );
        assert_emitted_runtime_status(
            &mut rx,
            false,
            CompressionPhase::Skipped,
            Some(CompressionReason::PressureLow),
        );
    }

    #[test]
    fn emit_compression_terminal_phases_are_inactive() {
        let cases = [
            (CompressionPhase::Applied, None),
            (
                CompressionPhase::Skipped,
                Some(CompressionReason::NoEligibleSegment),
            ),
            (
                CompressionPhase::Failed,
                Some(CompressionReason::TransientFailure),
            ),
        ];

        for (phase, reason) in cases {
            let mut session = ChatSession::new(format!("compression-terminal-{phase:?}"));
            session.is_compressing = true;
            session.runtime.is_compressing = true;
            let mut rx = session.subscribe();

            emit_compression_status(&mut session, phase, reason);

            assert!(!session.is_compressing);
            assert!(!session.runtime.is_compressing);
            assert_eq!(session.compression_phase, Some(phase));
            assert_eq!(session.runtime.compression_phase, Some(phase));
            assert_eq!(session.compression_reason, reason);
            assert_eq!(session.runtime.compression_reason, reason);
            assert_emitted_runtime_status(&mut rx, false, phase, reason);
        }
    }

    #[test]
    fn compression_failure_maps_to_structured_reason() {
        assert_eq!(
            compression_failure_reason(&SegmentSummaryFailure::NoModelAvailable),
            CompressionReason::NoSummaryModel
        );
        assert_eq!(
            compression_failure_reason(&SegmentSummaryFailure::InputTooLarge {
                excerpt_chars: 10,
                budget_chars: 1,
            }),
            CompressionReason::InputTooLarge
        );
        assert_eq!(
            compression_failure_reason(&SegmentSummaryFailure::Transient("network".to_string())),
            CompressionReason::TransientFailure
        );
    }

    #[test]
    fn append_compression_failure_event_adds_system_notice() {
        let mut session = ChatSession::new("compression-failure".to_string());
        let mut rx = session.subscribe();
        let before_version = session.trajectory_version;

        append_compression_failure_event(&mut session, &SegmentSummaryFailure::NoModelAvailable);

        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].role,
            crate::chat::internal_roles::EVENT_ROLE
        );
        assert_eq!(
            session.messages[0].extra["event"]["subkind"],
            json!("system_notice")
        );
        assert!(session.messages[0]
            .content
            .content_text_only()
            .contains("Context compression failed"));
        assert!(session.trajectory_version > before_version);
        let json = rx.try_recv().unwrap();
        let envelope: crate::chat::types::EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::MessageAdded { message, index } => {
                assert_eq!(index, 0);
                assert_eq!(message.extra["event"]["subkind"], json!("system_notice"));
            }
            other => panic!("expected MessageAdded, got {other:?}"),
        }
    }

    #[test]
    fn append_compression_failure_event_redacts_transient_failure() {
        let mut session = ChatSession::new("compression-failure-redacted".to_string());
        let failure = SegmentSummaryFailure::Transient(
            "provider failed with Bearer secret-bearer-value api_key=sk-abcdefghijklmnop"
                .to_string(),
        );

        append_compression_failure_event(&mut session, &failure);

        let message = &session.messages[0];
        let content = message.content.content_text_only();
        let payload_failure = message.extra["event"]["payload"]["failure"]
            .as_str()
            .unwrap();
        assert!(content.contains("Context compression failed"));
        assert!(payload_failure.contains("Bearer [REDACTED]"));
        assert!(
            payload_failure.contains("api_key=[REDACTED]")
                || payload_failure.contains("[REDACTED_SK_TOKEN]")
        );
        assert_no_raw_secrets(&content);
        assert_no_raw_secrets(payload_failure);
    }

    #[test]
    fn append_compression_failure_event_caps_huge_transient_failure() {
        let mut session = ChatSession::new("compression-failure-capped".to_string());
        let failure = SegmentSummaryFailure::Transient(format!(
            "provider failed with Bearer secret-bearer-value api_key=sk-abcdefghijklmnop {}",
            "tail ".repeat(PUBLIC_COMPRESSION_FAILURE_MAX_CHARS * 20)
        ));

        append_compression_failure_event(&mut session, &failure);

        let message = &session.messages[0];
        let content = message.content.content_text_only();
        let payload_failure = message.extra["event"]["payload"]["failure"]
            .as_str()
            .unwrap();
        assert!(payload_failure.len() <= PUBLIC_COMPRESSION_FAILURE_MAX_CHARS);
        assert!(
            content.len()
                <= "Context compression failed: ".len() + PUBLIC_COMPRESSION_FAILURE_MAX_CHARS
        );
        assert!(content.contains("Context compression failed"));
        assert!(payload_failure.contains(MESSAGE_CONTENT_TRUNCATED_MARKER));
        assert!(content.contains(MESSAGE_CONTENT_TRUNCATED_MARKER));
        assert_no_raw_secrets(&content);
        assert_no_raw_secrets(payload_failure);
    }

    #[test]
    fn apply_resolved_segment_summary_emits_snapshot_with_summary() {
        let mut session = ChatSession::new("compression-success".to_string());
        session.messages = vec![user("first"), assistant("old answer"), user("second")];
        session.is_compressing = true;
        session.runtime.is_compressing = true;
        let segment = first_eligible_segment(&session.messages).unwrap();
        let source_messages = session.messages[segment.start..=segment.end].to_vec();
        let source_hash = source_hash_for_messages(&source_messages);
        let summary = make_segment_summary_message(
            "compressed summary".to_string(),
            &source_messages,
            "test-model",
        );
        let mut rx = session.subscribe();

        assert!(apply_resolved_segment_summary(
            &mut session,
            &source_hash,
            summary
        ));

        assert!(!session.is_compressing);
        assert!(!session.runtime.is_compressing);
        assert_eq!(session.messages.len(), 3);
        assert!(is_segment_summary(&session.messages[1]));
        assert_eq!(
            session.messages[1].content.content_text_only(),
            "compressed summary"
        );
        let mut saw_runtime_false = false;
        let mut saw_snapshot_with_summary = false;
        while let Ok(json) = rx.try_recv() {
            let envelope: crate::chat::types::EventEnvelope = serde_json::from_str(&json).unwrap();
            match envelope.event {
                ChatEvent::RuntimeUpdated {
                    is_compressing,
                    compression_phase,
                    compression_reason,
                    ..
                } => {
                    saw_runtime_false = !is_compressing
                        && compression_phase == Some(CompressionPhase::Applied)
                        && compression_reason.is_none();
                }
                ChatEvent::Snapshot {
                    runtime, messages, ..
                } => {
                    saw_snapshot_with_summary = !runtime.is_compressing
                        && runtime.compression_phase == Some(CompressionPhase::Applied)
                        && runtime.compression_reason.is_none()
                        && messages.len() == 3
                        && is_segment_summary(&messages[1])
                        && messages[1].content.content_text_only() == "compressed summary";
                }
                _ => {}
            }
        }
        assert!(saw_runtime_false);
        assert!(saw_snapshot_with_summary);
    }

    #[test]
    fn apply_resolved_segment_summary_skips_when_runtime_becomes_active() {
        let mut session = ChatSession::new("compression-active-skip".to_string());
        session.messages = vec![user("first"), assistant("old answer"), user("second")];
        session.is_compressing = true;
        session.runtime.is_compressing = true;
        let segment = first_eligible_segment(&session.messages).unwrap();
        let source_messages = session.messages[segment.start..=segment.end].to_vec();
        let source_hash = source_hash_for_messages(&source_messages);
        let summary = make_segment_summary_message(
            "compressed summary".to_string(),
            &source_messages,
            "test-model",
        );
        session.runtime.state = SessionState::Generating;

        assert!(!apply_resolved_segment_summary(
            &mut session,
            &source_hash,
            summary
        ));

        assert_eq!(session.messages.len(), 3);
        assert_eq!(
            session.messages[1].content.content_text_only(),
            "old answer"
        );
        assert!(!session.is_compressing);
        assert!(!session.runtime.is_compressing);
        assert_eq!(session.compression_phase, Some(CompressionPhase::Skipped));
        assert_eq!(
            session.compression_reason,
            Some(CompressionReason::NoEligibleSegment)
        );
    }

    #[tokio::test]
    async fn resolve_summary_model_prefers_thread_model_over_global_defaults() {
        let gcx = make_test_gcx().await;
        let thread_model = "private-thread-model";
        let light_model = "global-light-model";
        let default_model = "global-default-model";
        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            thread_model.to_string(),
            chat_model_record(thread_model, 12_345),
        );
        caps.chat_models.insert(
            light_model.to_string(),
            chat_model_record(light_model, 65_536),
        );
        caps.chat_models.insert(
            default_model.to_string(),
            chat_model_record(default_model, 65_536),
        );
        caps.defaults.chat_light_model = light_model.to_string();
        caps.defaults.chat_default_model = default_model.to_string();
        install_caps(gcx.clone(), caps).await;

        let (model, n_ctx) = resolve_summary_model(gcx, thread_model).await.unwrap();

        assert_eq!(model, thread_model);
        assert_eq!(n_ctx, 12_345);
    }

    #[tokio::test]
    async fn resolve_summary_model_falls_back_to_light_when_thread_model_is_stale() {
        let gcx = make_test_gcx().await;
        let light_model = "global-light-model";
        let default_model = "global-default-model";
        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            light_model.to_string(),
            chat_model_record(light_model, 65_536),
        );
        caps.chat_models.insert(
            default_model.to_string(),
            chat_model_record(default_model, 131_072),
        );
        caps.defaults.chat_light_model = light_model.to_string();
        caps.defaults.chat_default_model = default_model.to_string();
        install_caps(gcx.clone(), caps).await;

        let (model, n_ctx) = resolve_summary_model(gcx, "stale-thread-model")
            .await
            .unwrap();

        assert_eq!(model, light_model);
        assert_eq!(n_ctx, 65_536);
    }

    #[tokio::test]
    async fn resolve_summary_model_falls_back_to_default_when_thread_and_light_are_stale() {
        let gcx = make_test_gcx().await;
        let default_model = "global-default-model";
        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            default_model.to_string(),
            chat_model_record(default_model, 0),
        );
        caps.defaults.chat_light_model = "stale-light-model".to_string();
        caps.defaults.chat_default_model = default_model.to_string();
        install_caps(gcx.clone(), caps).await;

        let (model, n_ctx) = resolve_summary_model(gcx, "stale-thread-model")
            .await
            .unwrap();

        assert_eq!(model, default_model);
        assert_eq!(n_ctx, crate::chat::config::tokens().default_n_ctx);
    }

    #[tokio::test]
    async fn resolve_summary_model_returns_no_model_when_all_candidates_are_invalid() {
        let gcx = make_test_gcx().await;
        let mut caps = CodeAssistantCaps::default();
        caps.defaults.chat_light_model = "stale-light-model".to_string();
        caps.defaults.chat_default_model = "stale-default-model".to_string();
        install_caps(gcx.clone(), caps).await;

        let failure = resolve_summary_model(gcx, "stale-thread-model")
            .await
            .unwrap_err();

        assert!(matches!(failure, SegmentSummaryFailure::NoModelAvailable));
    }

    #[tokio::test]
    async fn stale_thread_model_with_valid_fallback_computes_attempt_budget() {
        let gcx = make_test_gcx().await;
        let light_model = "global-light-model";
        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            light_model.to_string(),
            chat_model_record(light_model, 4_096),
        );
        caps.defaults.chat_light_model = light_model.to_string();
        caps.defaults.chat_default_model = "stale-default-model".to_string();
        install_caps(gcx.clone(), caps).await;
        let thread = crate::chat::types::ThreadParams {
            model: "removed-thread-model".to_string(),
            ..Default::default()
        };
        let messages = vec![
            user("first"),
            assistant(&"old assistant output ".repeat(20_000)),
            user("second"),
        ];

        let (model, model_n_ctx) = resolve_summary_model(gcx, &thread.model).await.unwrap();
        let effective_n_ctx = effective_n_ctx_for_resolved_summary_model(model_n_ctx, &thread);
        let pressure = estimated_context_pressure(&messages, effective_n_ctx);

        assert_eq!(model, light_model);
        assert_eq!(effective_n_ctx, 4_096);
        assert_eq!(
            first_eligible_segment(&messages),
            Some(SummarySegment { start: 1, end: 1 })
        );
        assert!(matches!(
            pressure,
            ContextPressure::High | ContextPressure::Critical
        ));
    }

    #[tokio::test]
    async fn context_cap_bounds_resolved_fallback_model_context() {
        let gcx = make_test_gcx().await;
        let light_model = "global-light-model";
        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            light_model.to_string(),
            chat_model_record(light_model, 65_536),
        );
        caps.defaults.chat_light_model = light_model.to_string();
        install_caps(gcx.clone(), caps).await;
        let mut thread = crate::chat::types::ThreadParams {
            model: "removed-thread-model".to_string(),
            context_tokens_cap: Some(8_192),
            ..Default::default()
        };

        let (_, model_n_ctx) = resolve_summary_model(gcx, &thread.model).await.unwrap();

        assert_eq!(
            effective_n_ctx_for_resolved_summary_model(model_n_ctx, &thread),
            8_192
        );
        thread.context_tokens_cap = Some(0);
        assert_eq!(
            effective_n_ctx_for_resolved_summary_model(model_n_ctx, &thread),
            65_536
        );
        thread.context_tokens_cap = Some(131_072);
        assert_eq!(
            effective_n_ctx_for_resolved_summary_model(model_n_ctx, &thread),
            65_536
        );
    }

    #[tokio::test]
    async fn active_compression_guard_exits_without_overwriting_running_attempt() {
        let gcx = make_test_gcx().await;
        let mut session = ChatSession::new("compression-active-guard".to_string());
        session.messages = vec![user("first"), assistant("old answer"), user("second")];
        session.is_compressing = true;
        session.runtime.is_compressing = true;
        session.compression_phase = Some(CompressionPhase::Running);
        session.runtime.compression_phase = Some(CompressionPhase::Running);
        let segment = first_eligible_segment(&session.messages).unwrap();
        let source_messages = session.messages[segment.start..=segment.end].to_vec();
        let source_hash = source_hash_for_messages(&source_messages);
        let summary = make_segment_summary_message(
            "compressed summary".to_string(),
            &source_messages,
            "test-model",
        );
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));
        let thread = crate::chat::types::ThreadParams {
            auto_compact_enabled: Some(false),
            ..Default::default()
        };

        assert!(!apply_segment_summarization(gcx, &session_arc, &thread, false).await);
        {
            let session = session_arc.lock().await;
            assert!(session.is_compressing);
            assert!(session.runtime.is_compressing);
            assert_eq!(session.compression_phase, Some(CompressionPhase::Running));
            assert_eq!(session.compression_reason, None);
            assert_eq!(session.event_seq, 0);
        }
        {
            let mut session = session_arc.lock().await;
            assert!(apply_resolved_segment_summary(
                &mut session,
                &source_hash,
                summary
            ));
            assert_eq!(session.compression_phase, Some(CompressionPhase::Applied));
            assert_eq!(session.compression_reason, None);
        }
    }

    #[tokio::test]
    async fn reserved_skipped_early_return_clears_active_compression_status() {
        let gcx = make_test_gcx().await;
        let session_arc = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
            "compression-reserved-skip".to_string(),
        )));
        let thread = crate::chat::types::ThreadParams::default();

        assert!(!apply_segment_summarization(gcx, &session_arc, &thread, false).await);

        let session = session_arc.lock().await;
        assert!(!session.is_compressing);
        assert!(!session.runtime.is_compressing);
        assert_eq!(session.compression_phase, Some(CompressionPhase::Skipped));
        assert_eq!(
            session.compression_reason,
            Some(CompressionReason::NoEligibleSegment)
        );
    }

    #[tokio::test]
    async fn reserved_failed_early_return_clears_active_compression_status() {
        let gcx = make_test_gcx().await;
        let mut caps = CodeAssistantCaps::default();
        caps.defaults.chat_light_model = "stale-light-model".to_string();
        caps.defaults.chat_default_model = "stale-default-model".to_string();
        install_caps(gcx.clone(), caps).await;
        let mut session = ChatSession::new("compression-reserved-fail".to_string());
        session.messages = vec![user("first"), assistant("old answer"), user("second")];
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));
        let thread = crate::chat::types::ThreadParams {
            model: "removed-thread-model".to_string(),
            ..Default::default()
        };

        assert!(!apply_segment_summarization(gcx, &session_arc, &thread, false).await);

        let session = session_arc.lock().await;
        assert!(!session.is_compressing);
        assert!(!session.runtime.is_compressing);
        assert_eq!(session.compression_phase, Some(CompressionPhase::Failed));
        assert_eq!(
            session.compression_reason,
            Some(CompressionReason::NoSummaryModel)
        );
        assert!(session.tier1_compaction_disabled);
    }

    #[test]
    fn failure_classification_marks_model_and_size_structural() {
        assert!(SegmentSummaryFailure::NoModelAvailable.is_structural());
        assert!(SegmentSummaryFailure::InputTooLarge {
            excerpt_chars: 10,
            budget_chars: 1,
        }
        .is_structural());
        assert!(!SegmentSummaryFailure::Transient("network".to_string()).is_structural());
    }
}
