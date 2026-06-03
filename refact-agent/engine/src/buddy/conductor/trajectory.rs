use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::buddy::jobs::autonomous_chats::redact_and_cap_text;
use crate::call_validation::{ChatContent, ChatMessage, ChatToolCall, ChatUsage, ContextFile};

pub const MAX_TRAJECTORY_DIGEST_JSON_CHARS: usize = 16_000;
const MAX_TRAJECTORY_DIGEST_TEXT_CHARS: usize = 8_000;
const MAX_RECENT_MESSAGES: usize = 8;
const MAX_TOOL_CALLS: usize = 16;
const MAX_SURGERY_MARKERS: usize = 12;
const MAX_PAUSE_REASONS: usize = 8;
const MAX_CONTEXT_FILES: usize = 4;
const MAX_ARGUMENT_KEYS: usize = 10;
const TEXT_TINY: usize = 64;
const TEXT_SHORT: usize = 140;
const TEXT_MEDIUM: usize = 220;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrajectorySurgeryError {
    EmptyMessageId,
    EmptyRemoval,
    MessageNotFound {
        message_id: String,
    },
    DuplicateMessageId {
        message_id: String,
    },
    InsertIndexOutOfBounds {
        index: usize,
        len: usize,
    },
    EmptyAssistantToolCallId {
        message_id: String,
    },
    EmptyToolResultId {
        message_id: String,
    },
    MissingToolResult {
        message_id: String,
        tool_call_id: String,
    },
    MissingToolCall {
        message_id: String,
        tool_call_id: String,
    },
}

impl fmt::Display for TrajectorySurgeryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMessageId => write!(f, "message_id must not be empty"),
            Self::EmptyRemoval => write!(f, "remove_messages requires at least one message_id"),
            Self::MessageNotFound { message_id } => {
                write!(f, "message not found: {message_id}")
            }
            Self::DuplicateMessageId { message_id } => {
                write!(f, "message_id is ambiguous: {message_id}")
            }
            Self::InsertIndexOutOfBounds { index, len } => {
                write!(
                    f,
                    "insert index {index} is out of bounds for {len} messages"
                )
            }
            Self::EmptyAssistantToolCallId { message_id } => {
                write!(
                    f,
                    "assistant message {message_id} has an empty tool call id"
                )
            }
            Self::EmptyToolResultId { message_id } => {
                write!(
                    f,
                    "tool result message {message_id} has an empty tool_call_id"
                )
            }
            Self::MissingToolResult {
                message_id,
                tool_call_id,
            } => write!(
                f,
                "assistant message {message_id} tool call {tool_call_id} has no tool result"
            ),
            Self::MissingToolCall {
                message_id,
                tool_call_id,
            } => write!(
                f,
                "tool result message {message_id} references missing tool call {tool_call_id}"
            ),
        }
    }
}

impl std::error::Error for TrajectorySurgeryError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertPosition {
    Index(usize),
    BeforeMessage(String),
    AfterMessage(String),
}

#[derive(Debug, Clone)]
pub struct TrajectoryDigestInput<'a> {
    pub chat_id: &'a str,
    pub title: Option<&'a str>,
    pub model: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub tool_use: Option<&'a str>,
    pub created_at: Option<&'a str>,
    pub updated_at: Option<&'a str>,
    pub version: Option<u64>,
    pub parent_id: Option<&'a str>,
    pub link_type: Option<&'a str>,
    pub root_chat_id: Option<&'a str>,
    pub task_id: Option<&'a str>,
    pub task_role: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    pub card_id: Option<&'a str>,
    pub session_state: Option<&'a str>,
    pub runtime_error: Option<&'a str>,
    pub pause_reasons: Vec<TrajectoryPauseReasonInput<'a>>,
    pub messages: &'a [ChatMessage],
}

impl<'a> TrajectoryDigestInput<'a> {
    pub fn from_messages(chat_id: &'a str, messages: &'a [ChatMessage]) -> Self {
        Self {
            chat_id,
            title: None,
            model: None,
            mode: None,
            tool_use: None,
            created_at: None,
            updated_at: None,
            version: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            task_id: None,
            task_role: None,
            agent_id: None,
            card_id: None,
            session_state: None,
            runtime_error: None,
            pause_reasons: Vec::new(),
            messages,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrajectoryPauseReasonInput<'a> {
    pub reason_type: &'a str,
    pub tool_name: &'a str,
    pub command: &'a str,
    pub rule: &'a str,
    pub tool_call_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuiltTrajectoryDigest {
    pub digest: TrajectoryDigest,
    pub json: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryDigest {
    pub schema_version: u32,
    pub digest_kind: String,
    pub size_cap_chars: usize,
    pub chat: TrajectoryChatDigest,
    pub runtime: TrajectoryRuntimeDigest,
    pub messages: TrajectoryMessagesDigest,
    pub tools: TrajectoryToolsDigest,
    pub usage: TrajectoryUsageDigest,
    pub context_files: TrajectoryContextFilesDigest,
    pub surgery: TrajectorySurgeryDigest,
    pub truncation_markers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryChatDigest {
    pub chat_id: String,
    pub title: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub tool_use: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub version: Option<u64>,
    pub parent_id: Option<String>,
    pub link_type: Option<String>,
    pub root_chat_id: Option<String>,
    pub task_id: Option<String>,
    pub task_role: Option<String>,
    pub agent_id: Option<String>,
    pub card_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryRuntimeDigest {
    pub session_state: Option<String>,
    pub runtime_error: Option<String>,
    pub pause_reason_count: usize,
    pub pause_reasons: Vec<TrajectoryPauseReasonDigest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryPauseReasonDigest {
    pub reason_type: String,
    pub tool_name: String,
    pub command: String,
    pub rule: String,
    pub tool_call_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryMessagesDigest {
    pub total: usize,
    pub role_counts: BTreeMap<String, usize>,
    pub recent_omitted: usize,
    pub recent_shown: usize,
    pub recent: Vec<TrajectoryMessagePreview>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryMessagePreview {
    pub index: usize,
    pub message_id: String,
    pub role: String,
    pub content_kind: String,
    pub preview: String,
    pub content_omitted: bool,
    pub tool_call_count: usize,
    pub has_tool_result_id: bool,
    pub finish_reason: Option<String>,
    pub usage: Option<TrajectoryUsageBrief>,
    pub summarized_range: Option<(usize, usize)>,
    pub summarization_tier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryToolsDigest {
    pub tool_calls_total: usize,
    pub tool_calls_shown: usize,
    pub tool_results_total: usize,
    pub failed_tool_results: usize,
    pub calls: Vec<TrajectoryToolCallSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryToolCallSummary {
    pub message_index: usize,
    pub message_id: String,
    pub tool_call_id: String,
    pub name: String,
    pub arguments_summary: String,
    pub argument_keys: Vec<String>,
    pub result: Option<TrajectoryToolResultSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryToolResultSummary {
    pub message_index: usize,
    pub message_id: String,
    pub role: String,
    pub failed: bool,
    pub preview: String,
    pub content_omitted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryUsageDigest {
    pub messages_with_usage: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryUsageBrief {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryContextFilesDigest {
    pub total: usize,
    pub shown: usize,
    pub files: Vec<TrajectoryContextFileSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryContextFileSummary {
    pub message_index: usize,
    pub message_id: String,
    pub file_name: String,
    pub line1: usize,
    pub line2: usize,
    pub content_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectorySurgeryDigest {
    pub total: usize,
    pub shown: usize,
    pub markers: Vec<TrajectorySurgeryMarker>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectorySurgeryMarker {
    pub message_index: usize,
    pub message_id: String,
    pub kind: String,
    pub detail: String,
}

pub fn build_trajectory_digest(input: TrajectoryDigestInput<'_>) -> BuiltTrajectoryDigest {
    let mut markers = BTreeSet::new();
    let mut digest = TrajectoryDigest {
        schema_version: 1,
        digest_kind: "conductor_trajectory_digest".to_string(),
        size_cap_chars: MAX_TRAJECTORY_DIGEST_JSON_CHARS,
        chat: trajectory_chat_digest(&input, &mut markers),
        runtime: trajectory_runtime_digest(&input, &mut markers),
        messages: trajectory_messages_digest(input.messages, &mut markers),
        tools: trajectory_tools_digest(input.messages, &mut markers),
        usage: trajectory_usage_digest(input.messages),
        context_files: trajectory_context_files_digest(input.messages, &mut markers),
        surgery: trajectory_surgery_digest(input.messages, &mut markers),
        truncation_markers: Vec::new(),
    };
    digest.truncation_markers = markers.into_iter().collect();
    let json = render_digest_json_capped(&mut digest);
    let text = render_digest_text_capped(&digest);
    BuiltTrajectoryDigest { digest, json, text }
}

fn trajectory_chat_digest(
    input: &TrajectoryDigestInput<'_>,
    markers: &mut BTreeSet<String>,
) -> TrajectoryChatDigest {
    TrajectoryChatDigest {
        chat_id: clean("chat.chat_id", input.chat_id, TEXT_TINY, markers),
        title: clean_opt("chat.title", input.title, TEXT_SHORT, markers),
        model: clean_opt("chat.model", input.model, TEXT_TINY, markers),
        mode: clean_opt("chat.mode", input.mode, TEXT_TINY, markers),
        tool_use: clean_opt("chat.tool_use", input.tool_use, TEXT_TINY, markers),
        created_at: clean_opt("chat.created_at", input.created_at, TEXT_TINY, markers),
        updated_at: clean_opt("chat.updated_at", input.updated_at, TEXT_TINY, markers),
        version: input.version,
        parent_id: clean_opt("chat.parent_id", input.parent_id, TEXT_TINY, markers),
        link_type: clean_opt("chat.link_type", input.link_type, TEXT_TINY, markers),
        root_chat_id: clean_opt("chat.root_chat_id", input.root_chat_id, TEXT_TINY, markers),
        task_id: clean_opt("chat.task_id", input.task_id, TEXT_TINY, markers),
        task_role: clean_opt("chat.task_role", input.task_role, TEXT_TINY, markers),
        agent_id: clean_opt("chat.agent_id", input.agent_id, TEXT_TINY, markers),
        card_id: clean_opt("chat.card_id", input.card_id, TEXT_TINY, markers),
    }
}

fn trajectory_runtime_digest(
    input: &TrajectoryDigestInput<'_>,
    markers: &mut BTreeSet<String>,
) -> TrajectoryRuntimeDigest {
    let mut pause_reasons = input.pause_reasons.iter().collect::<Vec<_>>();
    if pause_reasons.len() > MAX_PAUSE_REASONS {
        markers.insert("runtime.pause_reasons".to_string());
        pause_reasons.truncate(MAX_PAUSE_REASONS);
    }
    TrajectoryRuntimeDigest {
        session_state: clean_opt(
            "runtime.session_state",
            input.session_state,
            TEXT_TINY,
            markers,
        ),
        runtime_error: clean_opt("runtime.error", input.runtime_error, TEXT_MEDIUM, markers),
        pause_reason_count: input.pause_reasons.len(),
        pause_reasons: pause_reasons
            .into_iter()
            .map(|reason| TrajectoryPauseReasonDigest {
                reason_type: clean(
                    "runtime.pause_reason.type",
                    reason.reason_type,
                    TEXT_TINY,
                    markers,
                ),
                tool_name: clean(
                    "runtime.pause_reason.tool_name",
                    reason.tool_name,
                    TEXT_TINY,
                    markers,
                ),
                command: clean(
                    "runtime.pause_reason.command",
                    reason.command,
                    TEXT_SHORT,
                    markers,
                ),
                rule: clean("runtime.pause_reason.rule", reason.rule, TEXT_TINY, markers),
                tool_call_id: clean(
                    "runtime.pause_reason.tool_call_id",
                    reason.tool_call_id,
                    TEXT_TINY,
                    markers,
                ),
            })
            .collect(),
    }
}

fn trajectory_messages_digest(
    messages: &[ChatMessage],
    markers: &mut BTreeSet<String>,
) -> TrajectoryMessagesDigest {
    let mut role_counts = BTreeMap::new();
    for message in messages {
        let role = clean("message.role", &message.role, TEXT_TINY, markers);
        *role_counts.entry(role).or_insert(0) += 1;
    }

    let start = messages.len().saturating_sub(MAX_RECENT_MESSAGES);
    if start > 0 {
        markers.insert("messages.recent".to_string());
    }
    let recent = messages
        .iter()
        .enumerate()
        .skip(start)
        .map(|(index, message)| message_preview(index, message, markers))
        .collect::<Vec<_>>();

    TrajectoryMessagesDigest {
        total: messages.len(),
        role_counts,
        recent_omitted: start,
        recent_shown: recent.len(),
        recent,
    }
}

fn message_preview(
    index: usize,
    message: &ChatMessage,
    markers: &mut BTreeSet<String>,
) -> TrajectoryMessagePreview {
    let (preview, content_omitted) = message_content_preview(index, message, markers);
    TrajectoryMessagePreview {
        index,
        message_id: message_identifier(index, message, markers),
        role: clean("message.role", &message.role, TEXT_TINY, markers),
        content_kind: content_kind(message),
        preview,
        content_omitted,
        tool_call_count: message.tool_calls.as_ref().map_or(0, Vec::len),
        has_tool_result_id: !message.tool_call_id.is_empty(),
        finish_reason: clean_opt(
            "message.finish_reason",
            message.finish_reason.as_deref(),
            TEXT_TINY,
            markers,
        ),
        usage: message.usage.as_ref().map(usage_brief),
        summarized_range: message.summarized_range,
        summarization_tier: clean_opt(
            "message.summarization_tier",
            message.summarization_tier.as_deref(),
            TEXT_TINY,
            markers,
        ),
    }
}

fn trajectory_tools_digest(
    messages: &[ChatMessage],
    markers: &mut BTreeSet<String>,
) -> TrajectoryToolsDigest {
    let mut result_by_id = BTreeMap::new();
    let mut tool_results_total = 0usize;
    let mut failed_tool_results = 0usize;
    for (index, message) in messages.iter().enumerate() {
        if !is_tool_result(message) {
            continue;
        }
        tool_results_total += 1;
        if message.tool_failed.unwrap_or(false) {
            failed_tool_results += 1;
        }
        if !message.tool_call_id.is_empty() {
            result_by_id
                .entry(message.tool_call_id.clone())
                .or_insert((index, message));
        }
    }

    let mut total_calls = 0usize;
    let mut calls = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        let Some(tool_calls) = message.tool_calls.as_ref() else {
            continue;
        };
        for tool_call in tool_calls {
            total_calls += 1;
            if calls.len() >= MAX_TOOL_CALLS {
                continue;
            }
            calls.push(tool_call_summary(
                message_index,
                message,
                tool_call,
                &result_by_id,
                markers,
            ));
        }
    }
    if total_calls > calls.len() {
        markers.insert("tools.calls".to_string());
    }
    TrajectoryToolsDigest {
        tool_calls_total: total_calls,
        tool_calls_shown: calls.len(),
        tool_results_total,
        failed_tool_results,
        calls,
    }
}

fn tool_call_summary(
    message_index: usize,
    message: &ChatMessage,
    tool_call: &ChatToolCall,
    result_by_id: &BTreeMap<String, (usize, &ChatMessage)>,
    markers: &mut BTreeSet<String>,
) -> TrajectoryToolCallSummary {
    let result = result_by_id
        .get(&tool_call.id)
        .map(|(result_index, result)| tool_result_summary(*result_index, result, markers));
    let (argument_keys, arguments_summary) =
        summarize_tool_arguments(&tool_call.function.arguments, markers);
    TrajectoryToolCallSummary {
        message_index,
        message_id: message_identifier(message_index, message, markers),
        tool_call_id: clean("tool.call_id", &tool_call.id, TEXT_TINY, markers),
        name: clean("tool.name", &tool_call.function.name, TEXT_TINY, markers),
        arguments_summary,
        argument_keys,
        result,
    }
}

fn tool_result_summary(
    message_index: usize,
    message: &ChatMessage,
    markers: &mut BTreeSet<String>,
) -> TrajectoryToolResultSummary {
    let (preview, content_omitted) = message_content_preview(message_index, message, markers);
    TrajectoryToolResultSummary {
        message_index,
        message_id: message_identifier(message_index, message, markers),
        role: clean("tool.result.role", &message.role, TEXT_TINY, markers),
        failed: message.tool_failed.unwrap_or(false),
        preview,
        content_omitted,
    }
}

fn summarize_tool_arguments(
    arguments: &str,
    markers: &mut BTreeSet<String>,
) -> (Vec<String>, String) {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return (Vec::new(), "empty".to_string());
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(map)) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let total = keys.len();
            if keys.len() > MAX_ARGUMENT_KEYS {
                markers.insert("tools.argument_keys".to_string());
                keys.truncate(MAX_ARGUMENT_KEYS);
            }
            let keys = keys
                .into_iter()
                .map(|key| clean("tool.argument_key", &key, TEXT_TINY, markers))
                .collect::<Vec<_>>();
            (
                keys.clone(),
                format!("object keys shown {}/{}", keys.len(), total),
            )
        }
        Ok(Value::Array(items)) => (Vec::new(), format!("array len {}", items.len())),
        Ok(value) => (Vec::new(), value_type_summary(&value)),
        Err(_) => {
            markers.insert("tools.arguments".to_string());
            (
                Vec::new(),
                format!("unparsed text chars {}", trimmed.chars().count()),
            )
        }
    }
}

fn trajectory_usage_digest(messages: &[ChatMessage]) -> TrajectoryUsageDigest {
    let mut total = TrajectoryUsageDigest {
        messages_with_usage: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        cost_usd: None,
    };
    for usage in messages.iter().filter_map(|message| message.usage.as_ref()) {
        total.messages_with_usage += 1;
        total.prompt_tokens += usage.prompt_tokens as u64;
        total.completion_tokens += usage.completion_tokens as u64;
        total.total_tokens += usage.total_tokens as u64;
        total.cache_read_tokens += usage.cache_read_tokens.unwrap_or(0) as u64;
        total.cache_creation_tokens += usage.cache_creation_tokens.unwrap_or(0) as u64;
        if let Some(metering) = usage.metering_usd.as_ref() {
            *total.cost_usd.get_or_insert(0.0) += metering.total_usd;
        }
    }
    total
}

fn usage_brief(usage: &ChatUsage) -> TrajectoryUsageBrief {
    TrajectoryUsageBrief {
        prompt_tokens: usage.prompt_tokens as u64,
        completion_tokens: usage.completion_tokens as u64,
        total_tokens: usage.total_tokens as u64,
        cache_read_tokens: usage.cache_read_tokens.unwrap_or(0) as u64,
        cache_creation_tokens: usage.cache_creation_tokens.unwrap_or(0) as u64,
        cost_usd: usage
            .metering_usd
            .as_ref()
            .map(|metering| metering.total_usd),
    }
}

fn trajectory_context_files_digest(
    messages: &[ChatMessage],
    markers: &mut BTreeSet<String>,
) -> TrajectoryContextFilesDigest {
    let mut files = Vec::new();
    let mut total = 0usize;
    for (message_index, message) in messages.iter().enumerate() {
        for file in extract_context_files(message) {
            total += 1;
            if files.len() >= MAX_CONTEXT_FILES {
                continue;
            }
            files.push(TrajectoryContextFileSummary {
                message_index,
                message_id: message_identifier(message_index, message, markers),
                file_name: clean(
                    "context_file.file_name",
                    &file.file_name,
                    TEXT_SHORT,
                    markers,
                ),
                line1: file.line1,
                line2: file.line2,
                content_chars: file.file_content.chars().count(),
            });
        }
    }
    if total > files.len() {
        markers.insert("context_files.files".to_string());
    }
    TrajectoryContextFilesDigest {
        total,
        shown: files.len(),
        files,
    }
}

fn trajectory_surgery_digest(
    messages: &[ChatMessage],
    markers: &mut BTreeSet<String>,
) -> TrajectorySurgeryDigest {
    let mut all_markers = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        collect_message_surgery_markers(message_index, message, &mut all_markers, markers);
    }
    let total = all_markers.len();
    if all_markers.len() > MAX_SURGERY_MARKERS {
        markers.insert("surgery.markers".to_string());
        all_markers.truncate(MAX_SURGERY_MARKERS);
    }
    TrajectorySurgeryDigest {
        total,
        shown: all_markers.len(),
        markers: all_markers,
    }
}

fn collect_message_surgery_markers(
    message_index: usize,
    message: &ChatMessage,
    surgery_markers: &mut Vec<TrajectorySurgeryMarker>,
    markers: &mut BTreeSet<String>,
) {
    if message.summarized_range.is_some()
        || message.summarization_tier.is_some()
        || message.summarized_token_estimate.is_some()
    {
        let mut detail = Vec::new();
        if let Some((start, end)) = message.summarized_range {
            detail.push(format!("range={start}-{end}"));
        }
        if let Some(tier) = message.summarization_tier.as_deref() {
            detail.push(format!(
                "tier={}",
                clean("surgery.summarization_tier", tier, TEXT_TINY, markers)
            ));
        }
        if let Some(tokens) = message.summarized_token_estimate {
            detail.push(format!("tokens={tokens}"));
        }
        surgery_markers.push(surgery_marker(
            message_index,
            message,
            "summarization",
            &detail.join(" "),
            markers,
        ));
    }

    for key in ["conductor_surgery", "trajectory_surgery", "surgery"] {
        if let Some(value) = message.extra.get(key) {
            let detail = value_summary(value, markers);
            surgery_markers.push(surgery_marker(
                message_index,
                message,
                key,
                &detail,
                markers,
            ));
        }
    }

    if let Some(value) = message.extra.get("compression") {
        let detail = value_summary(value, markers);
        surgery_markers.push(surgery_marker(
            message_index,
            message,
            "compression",
            &detail,
            markers,
        ));
    }

    if message.role == "compression_report" || message.extra.contains_key("compression_report") {
        surgery_markers.push(surgery_marker(
            message_index,
            message,
            "compression_report",
            "compression report present",
            markers,
        ));
    }

    if let Some(event) = message.extra.get("event") {
        let subkind = event
            .get("subkind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if matches!(subkind, "summarization_marker" | "cancellation_note") {
            surgery_markers.push(surgery_marker(
                message_index,
                message,
                "event",
                subkind,
                markers,
            ));
        }
    }
}

fn surgery_marker(
    message_index: usize,
    message: &ChatMessage,
    kind: &str,
    detail: &str,
    markers: &mut BTreeSet<String>,
) -> TrajectorySurgeryMarker {
    TrajectorySurgeryMarker {
        message_index,
        message_id: message_identifier(message_index, message, markers),
        kind: clean("surgery.kind", kind, TEXT_TINY, markers),
        detail: clean("surgery.detail", detail, TEXT_MEDIUM, markers),
    }
}

fn render_digest_json_capped(digest: &mut TrajectoryDigest) -> String {
    let mut json = serde_json::to_string(digest).unwrap_or_else(|_| "{}".to_string());
    if json.len() <= MAX_TRAJECTORY_DIGEST_JSON_CHARS {
        return json;
    }
    push_digest_marker(digest, "digest.json");
    loop {
        json = serde_json::to_string(digest).unwrap_or_else(|_| "{}".to_string());
        if json.len() <= MAX_TRAJECTORY_DIGEST_JSON_CHARS {
            return json;
        }
        if digest.messages.recent.pop().is_some() {
            digest.messages.recent_shown = digest.messages.recent.len();
            digest.messages.recent_omitted = digest
                .messages
                .total
                .saturating_sub(digest.messages.recent.len());
            push_digest_marker(digest, "messages.recent");
            continue;
        }
        if digest.tools.calls.pop().is_some() {
            digest.tools.tool_calls_shown = digest.tools.calls.len();
            push_digest_marker(digest, "tools.calls");
            continue;
        }
        if digest.context_files.files.pop().is_some() {
            digest.context_files.shown = digest.context_files.files.len();
            push_digest_marker(digest, "context_files.files");
            continue;
        }
        if digest.surgery.markers.pop().is_some() {
            digest.surgery.shown = digest.surgery.markers.len();
            push_digest_marker(digest, "surgery.markers");
            continue;
        }
        if compact_digest_previews(digest) {
            push_digest_marker(digest, "previews");
            continue;
        }
        digest.messages.recent.clear();
        digest.messages.recent_shown = 0;
        digest.messages.recent_omitted = digest.messages.total;
        digest.tools.calls.clear();
        digest.tools.tool_calls_shown = 0;
        digest.context_files.files.clear();
        digest.context_files.shown = 0;
        digest.surgery.markers.clear();
        digest.surgery.shown = 0;
        json = serde_json::to_string(digest).unwrap_or_else(|_| "{}".to_string());
        if json.len() <= MAX_TRAJECTORY_DIGEST_JSON_CHARS {
            return json;
        }
        digest.messages.role_counts.clear();
        digest.runtime.pause_reasons.clear();
        digest.chat.title = None;
        digest.chat.parent_id = None;
        digest.chat.root_chat_id = None;
        digest.runtime.runtime_error = None;
        push_digest_marker(digest, "digest.minimal");
        return serde_json::to_string(digest).unwrap_or_else(|_| "{}".to_string());
    }
}

fn render_digest_text_capped(digest: &TrajectoryDigest) -> String {
    let mut lines = vec![
        format!("# Trajectory digest: {}", digest.chat.chat_id),
        format!(
            "Chat: title={} mode={} model={} state={}",
            digest.chat.title.as_deref().unwrap_or("none"),
            digest.chat.mode.as_deref().unwrap_or("none"),
            digest.chat.model.as_deref().unwrap_or("none"),
            digest.runtime.session_state.as_deref().unwrap_or("unknown")
        ),
        format!(
            "Messages: total={} roles={:?} recent_shown={} omitted={}",
            digest.messages.total,
            digest.messages.role_counts,
            digest.messages.recent_shown,
            digest.messages.recent_omitted
        ),
        format!(
            "Usage: prompt={} completion={} total={} cache_read={} cache_creation={} cost_usd={:?}",
            digest.usage.prompt_tokens,
            digest.usage.completion_tokens,
            digest.usage.total_tokens,
            digest.usage.cache_read_tokens,
            digest.usage.cache_creation_tokens,
            digest.usage.cost_usd
        ),
        format!(
            "Tools: calls_shown={}/{} results={} failed={}",
            digest.tools.tool_calls_shown,
            digest.tools.tool_calls_total,
            digest.tools.tool_results_total,
            digest.tools.failed_tool_results
        ),
    ];
    if !digest.context_files.files.is_empty() {
        lines.push(format!(
            "Context files: shown={}/{}",
            digest.context_files.shown, digest.context_files.total
        ));
        for file in &digest.context_files.files {
            lines.push(format!(
                "- [{}] {}:{}-{} chars={}",
                file.message_index, file.file_name, file.line1, file.line2, file.content_chars
            ));
        }
    }
    if !digest.messages.recent.is_empty() {
        lines.push("Recent messages:".to_string());
        for message in &digest.messages.recent {
            lines.push(format!(
                "- [{}] {} {}: {}",
                message.index, message.role, message.message_id, message.preview
            ));
        }
    }
    if !digest.tools.calls.is_empty() {
        lines.push("Tool calls:".to_string());
        for call in &digest.tools.calls {
            lines.push(format!(
                "- [{}] {} {} args={} result={}",
                call.message_index,
                call.name,
                call.tool_call_id,
                call.arguments_summary,
                call.result
                    .as_ref()
                    .map(|result| result.preview.as_str())
                    .unwrap_or("missing")
            ));
        }
    }
    if !digest.surgery.markers.is_empty() {
        lines.push("Surgery markers:".to_string());
        for marker in &digest.surgery.markers {
            lines.push(format!(
                "- [{}] {} {}: {}",
                marker.message_index, marker.message_id, marker.kind, marker.detail
            ));
        }
    }
    if !digest.truncation_markers.is_empty() {
        lines.push(format!(
            "Truncation markers: {}",
            digest.truncation_markers.join(", ")
        ));
    }
    redact_and_cap_text(&lines.join("\n"), MAX_TRAJECTORY_DIGEST_TEXT_CHARS)
}

fn compact_digest_previews(digest: &mut TrajectoryDigest) -> bool {
    let mut changed = false;
    for message in &mut digest.messages.recent {
        if !message.preview.is_empty() && message.preview != "[preview omitted]" {
            message.preview = "[preview omitted]".to_string();
            message.content_omitted = true;
            changed = true;
        }
    }
    for call in &mut digest.tools.calls {
        if let Some(result) = &mut call.result {
            if !result.preview.is_empty() && result.preview != "[preview omitted]" {
                result.preview = "[preview omitted]".to_string();
                result.content_omitted = true;
                changed = true;
            }
        }
    }
    changed
}

fn push_digest_marker(digest: &mut TrajectoryDigest, marker: &str) {
    if !digest
        .truncation_markers
        .iter()
        .any(|existing| existing == marker)
    {
        digest.truncation_markers.push(marker.to_string());
        digest.truncation_markers.sort();
    }
}

fn message_identifier(
    index: usize,
    message: &ChatMessage,
    markers: &mut BTreeSet<String>,
) -> String {
    if message.message_id.is_empty() {
        format!("index-{index}")
    } else {
        clean(
            "message.message_id",
            &message.message_id,
            TEXT_TINY,
            markers,
        )
    }
}

fn content_kind(message: &ChatMessage) -> String {
    match &message.content {
        ChatContent::SimpleText(_) => "text".to_string(),
        ChatContent::Multimodal(_) => "multimodal".to_string(),
        ChatContent::ContextFiles(_) => "context_files".to_string(),
    }
}

fn message_content_preview(
    index: usize,
    message: &ChatMessage,
    markers: &mut BTreeSet<String>,
) -> (String, bool) {
    match message.role.as_str() {
        "context_file" => {
            markers.insert(format!("messages.{index}.context_file"));
            let files = extract_context_files(message)
                .into_iter()
                .take(3)
                .map(|file| {
                    clean(
                        "context_file.file_name",
                        &file.file_name,
                        TEXT_TINY,
                        markers,
                    )
                })
                .collect::<Vec<_>>();
            if files.is_empty() {
                ("[context file body omitted]".to_string(), true)
            } else {
                (
                    format!("[context file bodies omitted: {}]", files.join(", ")),
                    true,
                )
            }
        }
        "diff" => {
            markers.insert(format!("messages.{index}.diff"));
            ("[diff body omitted]".to_string(), true)
        }
        "tool" => {
            markers.insert(format!("messages.{index}.tool"));
            ("[tool result omitted]".to_string(), true)
        }
        "system" => {
            markers.insert(format!("messages.{index}.system"));
            ("[system prompt omitted]".to_string(), true)
        }
        "plain_text" => {
            markers.insert(format!("messages.{index}.plain_text"));
            ("[plain_text body omitted]".to_string(), true)
        }
        "cd_instruction" => {
            markers.insert(format!("messages.{index}.cd_instruction"));
            ("[cd_instruction omitted]".to_string(), true)
        }
        _ => match &message.content {
            ChatContent::ContextFiles(_) => {
                markers.insert(format!("messages.{index}.context_files"));
                ("[context file bodies omitted]".to_string(), true)
            }
            ChatContent::SimpleText(text) => {
                let preview = clean(
                    &format!("messages.{index}.preview"),
                    text,
                    TEXT_MEDIUM,
                    markers,
                );
                (empty_preview(preview), false)
            }
            ChatContent::Multimodal(_) => {
                let text = message.content.to_text_with_image_placeholders();
                let preview = clean(
                    &format!("messages.{index}.preview"),
                    &text,
                    TEXT_MEDIUM,
                    markers,
                );
                (empty_preview(preview), false)
            }
        },
    }
}

fn empty_preview(preview: String) -> String {
    if preview.trim().is_empty() {
        "[empty]".to_string()
    } else {
        preview
    }
}

fn extract_context_files(message: &ChatMessage) -> Vec<ContextFile> {
    match &message.content {
        ChatContent::ContextFiles(files) => files.clone(),
        ChatContent::SimpleText(text) if message.role == "context_file" => {
            serde_json::from_str::<Vec<ContextFile>>(text).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn clean_opt(
    label: &str,
    value: Option<&str>,
    max_chars: usize,
    markers: &mut BTreeSet<String>,
) -> Option<String> {
    value.map(|value| clean(label, value, max_chars, markers))
}

fn clean(label: &str, value: &str, max_chars: usize, markers: &mut BTreeSet<String>) -> String {
    let (without_code, code_omitted) = omit_code_content(value);
    let normalized = without_code
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = redact_and_cap_text(&normalized, max_chars);
    if code_omitted
        || normalized.len() > max_chars
        || cleaned.contains("...[truncated]")
        || cleaned != normalized
    {
        markers.insert(label.to_string());
    }
    cleaned
}

fn omit_code_content(value: &str) -> (String, bool) {
    let (without_fences, fence_omitted) = omit_fenced_code_blocks(value);
    let (without_lines, lines_omitted) = omit_code_like_lines(&without_fences);
    (without_lines, fence_omitted || lines_omitted)
}

fn omit_fenced_code_blocks(value: &str) -> (String, bool) {
    let mut output = Vec::new();
    let mut in_fence = false;
    let mut omitted = false;
    for line in value.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if !in_fence {
                output.push("[code omitted]".to_string());
                omitted = true;
            }
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            output.push(line.to_string());
        } else {
            omitted = true;
        }
    }
    (output.join("\n"), omitted)
}

fn omit_code_like_lines(value: &str) -> (String, bool) {
    let mut output = Vec::new();
    let mut omitted = false;
    let mut last_omitted = false;
    for line in value.lines() {
        if looks_like_code_line(line) {
            if !last_omitted {
                output.push("[code omitted]".to_string());
            }
            omitted = true;
            last_omitted = true;
        } else {
            output.push(line.to_string());
            last_omitted = false;
        }
    }
    (output.join("\n"), omitted)
}

fn looks_like_code_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 4 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let starters = [
        "fn ",
        "pub fn ",
        "async fn ",
        "function ",
        "class ",
        "interface ",
        "type ",
        "struct ",
        "enum ",
        "impl ",
        "def ",
        "import ",
        "export ",
        "#include",
        "package ",
    ];
    if starters.iter().any(|starter| lower.starts_with(starter)) {
        return true;
    }
    if (lower.starts_with("let ") || lower.starts_with("const ") || lower.starts_with("var "))
        && (trimmed.contains('=') || trimmed.ends_with(';'))
    {
        return true;
    }
    let diff_trimmed = trimmed.trim_start_matches(|ch| ch == '+' || ch == '-');
    diff_trimmed != trimmed && looks_like_code_line(diff_trimmed)
}

fn value_summary(value: &Value, markers: &mut BTreeSet<String>) -> String {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            if keys.len() > MAX_ARGUMENT_KEYS {
                markers.insert("surgery.detail_keys".to_string());
                keys.truncate(MAX_ARGUMENT_KEYS);
            }
            let keys = keys
                .into_iter()
                .map(|key| clean("surgery.detail_key", &key, TEXT_TINY, markers))
                .collect::<Vec<_>>();
            format!("object keys: {}", keys.join(","))
        }
        Value::Array(items) => format!("array len {}", items.len()),
        Value::String(text) => clean("surgery.detail", text, TEXT_MEDIUM, markers),
        other => value_type_summary(other),
    }
}

fn value_type_summary(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(_) => "bool".to_string(),
        Value::Number(_) => "number".to_string(),
        Value::String(text) => format!("string chars {}", text.chars().count()),
        Value::Array(items) => format!("array len {}", items.len()),
        Value::Object(map) => format!("object keys {}", map.len()),
    }
}

pub fn validate_pairing(messages: &[ChatMessage]) -> Result<(), TrajectorySurgeryError> {
    let mut result_ids = HashSet::new();
    for message in messages.iter().filter(|message| is_tool_result(message)) {
        if message.tool_call_id.is_empty() {
            return Err(TrajectorySurgeryError::EmptyToolResultId {
                message_id: message.message_id.clone(),
            });
        }
        result_ids.insert(message.tool_call_id.clone());
    }

    let mut assistant_ids = HashSet::new();
    for message in messages
        .iter()
        .filter(|message| message.role == "assistant")
    {
        let Some(tool_calls) = message.tool_calls.as_ref() else {
            continue;
        };
        for tool_call in tool_calls {
            if tool_call.id.is_empty() {
                return Err(TrajectorySurgeryError::EmptyAssistantToolCallId {
                    message_id: message.message_id.clone(),
                });
            }
            if !result_ids.contains(&tool_call.id) {
                return Err(TrajectorySurgeryError::MissingToolResult {
                    message_id: message.message_id.clone(),
                    tool_call_id: tool_call.id.clone(),
                });
            }
            assistant_ids.insert(tool_call.id.clone());
        }
    }

    for message in messages.iter().filter(|message| is_tool_result(message)) {
        if !assistant_ids.contains(&message.tool_call_id) {
            return Err(TrajectorySurgeryError::MissingToolCall {
                message_id: message.message_id.clone(),
                tool_call_id: message.tool_call_id.clone(),
            });
        }
    }

    Ok(())
}

pub fn edit_message(
    messages: &[ChatMessage],
    message_id: &str,
    mut replacement: ChatMessage,
) -> Result<Vec<ChatMessage>, TrajectorySurgeryError> {
    let index = find_message_index(messages, message_id)?;
    replacement.message_id = messages[index].message_id.clone();

    let mut edited = messages.to_vec();
    edited[index] = replacement;
    validate_pairing(&edited)?;
    Ok(edited)
}

pub fn remove_messages<I, S>(
    messages: &[ChatMessage],
    message_ids: I,
) -> Result<Vec<ChatMessage>, TrajectorySurgeryError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let ids = collect_message_ids(message_ids)?;
    if ids.is_empty() {
        return Err(TrajectorySurgeryError::EmptyRemoval);
    }
    for id in &ids {
        find_message_index(messages, id)?;
    }

    let remove_ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let edited: Vec<ChatMessage> = messages
        .iter()
        .filter(|message| !remove_ids.contains(message.message_id.as_str()))
        .cloned()
        .collect();
    validate_pairing(&edited)?;
    Ok(edited)
}

pub fn insert_message(
    messages: &[ChatMessage],
    position: InsertPosition,
    message: ChatMessage,
) -> Result<Vec<ChatMessage>, TrajectorySurgeryError> {
    ensure_new_message_id(messages, &message)?;
    let index = resolve_insert_index(messages, position)?;

    let mut edited = messages.to_vec();
    edited.insert(index, message);
    validate_pairing(&edited)?;
    Ok(edited)
}

fn is_tool_result(message: &ChatMessage) -> bool {
    matches!(message.role.as_str(), "tool" | "diff")
}

fn collect_message_ids<I, S>(message_ids: I) -> Result<Vec<String>, TrajectorySurgeryError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut ids: Vec<String> = Vec::new();
    for message_id in message_ids {
        let message_id = message_id.as_ref();
        if message_id.is_empty() {
            return Err(TrajectorySurgeryError::EmptyMessageId);
        }
        if !ids.iter().any(|id| id.as_str() == message_id) {
            ids.push(message_id.to_string());
        }
    }
    Ok(ids)
}

fn find_message_index(
    messages: &[ChatMessage],
    message_id: &str,
) -> Result<usize, TrajectorySurgeryError> {
    if message_id.is_empty() {
        return Err(TrajectorySurgeryError::EmptyMessageId);
    }

    let mut found = None;
    for (index, message) in messages.iter().enumerate() {
        if message.message_id == message_id {
            if found.is_some() {
                return Err(TrajectorySurgeryError::DuplicateMessageId {
                    message_id: message_id.to_string(),
                });
            }
            found = Some(index);
        }
    }

    found.ok_or_else(|| TrajectorySurgeryError::MessageNotFound {
        message_id: message_id.to_string(),
    })
}

fn ensure_new_message_id(
    messages: &[ChatMessage],
    message: &ChatMessage,
) -> Result<(), TrajectorySurgeryError> {
    if message.message_id.is_empty() {
        return Err(TrajectorySurgeryError::EmptyMessageId);
    }
    if messages
        .iter()
        .any(|existing| existing.message_id == message.message_id)
    {
        return Err(TrajectorySurgeryError::DuplicateMessageId {
            message_id: message.message_id.clone(),
        });
    }
    Ok(())
}

fn resolve_insert_index(
    messages: &[ChatMessage],
    position: InsertPosition,
) -> Result<usize, TrajectorySurgeryError> {
    match position {
        InsertPosition::Index(index) => {
            if index <= messages.len() {
                Ok(index)
            } else {
                Err(TrajectorySurgeryError::InsertIndexOutOfBounds {
                    index,
                    len: messages.len(),
                })
            }
        }
        InsertPosition::BeforeMessage(message_id) => find_message_index(messages, &message_id),
        InsertPosition::AfterMessage(message_id) => {
            find_message_index(messages, &message_id).map(|index| index + 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

    fn user(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "user".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn assistant(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn assistant_tool(id: &str, tool_call_id: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            tool_calls: Some(vec![ChatToolCall {
                id: tool_call_id.to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    arguments: "{}".to_string(),
                    name: "test_tool".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }
    }

    fn tool(id: &str, tool_call_id: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "tool".to_string(),
            content: ChatContent::SimpleText("ok".to_string()),
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        }
    }

    fn pair_messages() -> Vec<ChatMessage> {
        vec![
            user("user-1", "hello"),
            assistant_tool("assistant-tool", "call-1"),
            tool("tool-1", "call-1"),
            assistant("assistant-final", "done"),
        ]
    }

    fn message_ids(messages: &[ChatMessage]) -> Vec<&str> {
        messages
            .iter()
            .map(|message| message.message_id.as_str())
            .collect()
    }

    #[test]
    fn valid_pair_accepted() {
        validate_pairing(&pair_messages()).unwrap();
    }

    #[test]
    fn assistant_only_removal_rejected() {
        let err = remove_messages(&pair_messages(), ["assistant-tool"]).unwrap_err();

        assert_eq!(
            err,
            TrajectorySurgeryError::MissingToolCall {
                message_id: "tool-1".to_string(),
                tool_call_id: "call-1".to_string(),
            }
        );
    }

    #[test]
    fn tool_only_removal_rejected() {
        let err = remove_messages(&pair_messages(), ["tool-1"]).unwrap_err();

        assert_eq!(
            err,
            TrajectorySurgeryError::MissingToolResult {
                message_id: "assistant-tool".to_string(),
                tool_call_id: "call-1".to_string(),
            }
        );
    }

    #[test]
    fn safe_non_tool_edit_accepted() {
        let edited =
            edit_message(&pair_messages(), "user-1", user("different-id", "changed")).unwrap();

        assert_eq!(edited[0].message_id, "user-1");
        assert_eq!(edited[0].content.content_text_only(), "changed");
        validate_pairing(&edited).unwrap();
    }

    #[test]
    fn tool_id_break_rejected() {
        let err = edit_message(
            &pair_messages(),
            "assistant-tool",
            assistant_tool("replacement-id", "call-2"),
        )
        .unwrap_err();

        assert_eq!(
            err,
            TrajectorySurgeryError::MissingToolResult {
                message_id: "assistant-tool".to_string(),
                tool_call_id: "call-2".to_string(),
            }
        );
    }

    #[test]
    fn valid_insert_before_and_after() {
        let before = insert_message(
            &pair_messages(),
            InsertPosition::BeforeMessage("assistant-final".to_string()),
            user("insert-before", "before"),
        )
        .unwrap();
        assert_eq!(
            message_ids(&before),
            vec![
                "user-1",
                "assistant-tool",
                "tool-1",
                "insert-before",
                "assistant-final"
            ]
        );

        let after = insert_message(
            &pair_messages(),
            InsertPosition::AfterMessage("user-1".to_string()),
            user("insert-after", "after"),
        )
        .unwrap();
        assert_eq!(
            message_ids(&after),
            vec![
                "user-1",
                "insert-after",
                "assistant-tool",
                "tool-1",
                "assistant-final"
            ]
        );
    }

    #[test]
    fn empty_removal_rejected() {
        let err = remove_messages(&pair_messages(), std::iter::empty::<&str>()).unwrap_err();

        assert_eq!(err, TrajectorySurgeryError::EmptyRemoval);
    }
}

#[cfg(test)]
mod digest {
    use super::*;
    use crate::call_validation::{
        ChatContent, ChatToolCall, ChatToolFunction, ChatUsage, ContextFile, MeteringUsd,
    };

    fn message(id: &str, role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: role.to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn assistant_tool(id: &str, tool_call_id: &str, name: &str, arguments: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("I'll inspect it.".to_string()),
            tool_calls: Some(vec![ChatToolCall {
                id: tool_call_id.to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    arguments: arguments.to_string(),
                    name: name.to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }
    }

    fn tool_result(id: &str, tool_call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        }
    }

    fn usage(prompt: usize, completion: usize, total: usize) -> ChatUsage {
        ChatUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
            cache_read_tokens: Some(3),
            cache_creation_tokens: Some(2),
            metering_usd: Some(MeteringUsd {
                prompt_usd: 0.1,
                generated_usd: 0.2,
                cache_read_usd: None,
                cache_creation_usd: None,
                total_usd: 0.3,
            }),
        }
    }

    fn digest_input<'a>(messages: &'a [ChatMessage]) -> TrajectoryDigestInput<'a> {
        TrajectoryDigestInput {
            chat_id: "agent-chat-1",
            title: Some("Investigate conductor card"),
            model: Some("provider/model"),
            mode: Some("agent"),
            tool_use: Some("agent"),
            created_at: Some("2026-06-03T00:00:00Z"),
            updated_at: Some("2026-06-03T00:01:00Z"),
            version: Some(7),
            parent_id: Some("planner-chat"),
            link_type: Some("task_agent"),
            root_chat_id: Some("planner-chat"),
            task_id: Some("task-1"),
            task_role: Some("agents"),
            agent_id: Some("agent-1"),
            card_id: Some("T-14"),
            session_state: Some("paused"),
            runtime_error: None,
            pause_reasons: vec![TrajectoryPauseReasonInput {
                reason_type: "confirmation",
                tool_name: "shell",
                command: "shell",
                rule: "ask",
                tool_call_id: "call-1",
            }],
            messages,
        }
    }

    fn output_text(built: &BuiltTrajectoryDigest) -> String {
        format!("{}\n{}", built.json, built.text)
    }

    #[test]
    fn role_tool_usage_summary_includes_runtime_and_surgery_markers() {
        let mut final_message = message(
            "assistant-final",
            "assistant",
            "Done with the safe summary.",
        );
        final_message.usage = Some(usage(11, 13, 24));
        final_message.summarized_range = Some((0, 1));
        final_message.summarization_tier = Some("tier2_reactive".to_string());
        final_message.summarized_token_estimate = Some(120);
        let messages = vec![
            message("user-1", "user", "Please inspect the trajectory."),
            assistant_tool(
                "assistant-tool",
                "call-1",
                "cat",
                r#"{"paths":"src/lib.rs","unused":true}"#,
            ),
            tool_result("tool-1", "call-1", "read ok"),
            final_message,
        ];

        let built = build_trajectory_digest(digest_input(&messages));
        let digest = &built.digest;

        assert_eq!(digest.chat.chat_id, "agent-chat-1");
        assert_eq!(digest.runtime.session_state.as_deref(), Some("paused"));
        assert_eq!(digest.runtime.pause_reason_count, 1);
        assert_eq!(digest.messages.total, 4);
        assert_eq!(digest.messages.role_counts.get("user"), Some(&1));
        assert_eq!(digest.messages.role_counts.get("assistant"), Some(&2));
        assert_eq!(digest.tools.tool_calls_total, 1);
        assert_eq!(digest.tools.tool_results_total, 1);
        assert_eq!(digest.tools.calls[0].name, "cat");
        assert_eq!(digest.tools.calls[0].argument_keys, vec!["paths", "unused"]);
        assert_eq!(
            digest.tools.calls[0].result.as_ref().unwrap().preview,
            "[tool result omitted]"
        );
        assert_eq!(digest.usage.messages_with_usage, 1);
        assert_eq!(digest.usage.prompt_tokens, 11);
        assert_eq!(digest.usage.completion_tokens, 13);
        assert_eq!(digest.usage.total_tokens, 24);
        assert_eq!(digest.usage.cache_read_tokens, 3);
        assert_eq!(digest.usage.cache_creation_tokens, 2);
        assert_eq!(digest.usage.cost_usd, Some(0.3));
        assert_eq!(digest.surgery.total, 1);
        assert_eq!(digest.surgery.markers[0].kind, "summarization");
        assert!(built
            .text
            .contains("Usage: prompt=11 completion=13 total=24"));
    }

    #[test]
    fn secret_redaction_removes_sensitive_values_from_all_outputs() {
        let messages = vec![
            message(
                "user-1",
                "user",
                "The failing request had token=verysecretvalue123 password=plainsecret and Bearer abcdef12345.",
            ),
            assistant_tool(
                "assistant-tool",
                "call-1",
                "shell",
                r#"{"command":"echo safe","api_key":"sk-abcdef1234567890"}"#,
            ),
            tool_result("tool-1", "call-1", "request failed token=toolsecretvalue"),
        ];
        let mut input = digest_input(&messages);
        input.runtime_error = Some("provider returned password=runtime_secret");

        let built = build_trajectory_digest(input);
        let output = output_text(&built);

        for raw in [
            "verysecretvalue123",
            "plainsecret",
            "abcdef12345",
            "sk-abcdef1234567890",
            "toolsecretvalue",
            "runtime_secret",
        ] {
            assert!(!output.contains(raw), "raw secret leaked: {raw}");
        }
        assert!(output.contains("[REDACTED"));
        assert!(built
            .digest
            .truncation_markers
            .iter()
            .any(|marker| marker.contains("preview") || marker == "runtime.error"));
    }

    #[test]
    fn long_content_truncates_with_visible_markers() {
        let messages = vec![message("user-1", "user", &"long-content ".repeat(80))];

        let built = build_trajectory_digest(digest_input(&messages));

        assert!(built.json.len() <= MAX_TRAJECTORY_DIGEST_JSON_CHARS);
        assert!(built.json.contains("...[truncated]"));
        assert!(built
            .digest
            .truncation_markers
            .iter()
            .any(|marker| marker.contains("preview")));
    }

    #[test]
    fn raw_code_and_context_file_bodies_are_not_included() {
        let context = ChatMessage {
            message_id: "context-1".to_string(),
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: "src/lib.rs".to_string(),
                file_content: "pub fn should_not_leak() { println!(\"secret\"); }".to_string(),
                line1: 1,
                line2: 1,
                ..Default::default()
            }]),
            ..Default::default()
        };
        let diff = message(
            "diff-1",
            "diff",
            "diff --git a/src/lib.rs b/src/lib.rs\n+fn raw_diff_body_should_not_leak() {}",
        );
        let assistant = message(
            "assistant-1",
            "assistant",
            "Summary before code.\n```rust\nfn fenced_body_should_not_leak() {}\n```\nSummary after code.",
        );
        let inline_code = message(
            "assistant-2",
            "assistant",
            "I saw this line:\nlet inline_body_should_not_leak = true;\nNo need to expose it.",
        );
        let messages = vec![context, diff, assistant, inline_code];

        let built = build_trajectory_digest(digest_input(&messages));
        let output = output_text(&built);

        for raw in [
            "should_not_leak",
            "raw_diff_body_should_not_leak",
            "fenced_body_should_not_leak",
            "inline_body_should_not_leak",
        ] {
            assert!(!output.contains(raw), "raw body leaked: {raw}");
        }
        assert!(output.contains("[context file bodies omitted"));
        assert!(output.contains("[diff body omitted]"));
        assert!(output.contains("[code omitted]"));
        assert_eq!(built.digest.context_files.total, 1);
        assert_eq!(built.digest.context_files.files[0].file_name, "src/lib.rs");
        assert!(built.digest.context_files.files[0].content_chars > 0);
    }
}
