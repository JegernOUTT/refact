use std::sync::Arc;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::diagnostics::{filter_ui_only_messages, is_ui_only_message};
use crate::chat::history_limit::{compute_context_budget, ContextPressure};
use crate::global_context::GlobalContext;
use crate::subchat::{run_subchat, SubchatConfig, ToolsPolicy};
use refact_chat_history::compression_exemption::{exemption_for, CompressionExemption};

pub const MAX_SEGMENT_SUMMARY_ATTEMPTS: usize = 2;
const SEGMENT_SUMMARY_OVERHEAD_TOKENS: usize = 1024;
const SUMMARY_KIND: &str = "llm_segment_summary";
const SUMMARY_SCHEMA_VERSION: u64 = 1;

#[derive(Debug, Clone)]
pub enum SegmentSummaryFailure {
    NoModelAvailable,
    InputTooLarge {
        excerpt_chars: usize,
        budget_chars: usize,
    },
    NoMessagesToSummarize,
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

const SEGMENT_SUMMARY_PROMPT: &str = "Produce a factual summary of the non-user conversation segment. The source segment contains assistant replies, tool calls, tool results, diffs, events, context files, and diagnostics that occurred between two immutable user messages.

Preserve facts that matter for continuing the task:
- tool calls and tool results that affected the work
- files read, edited, created, deleted, or discussed
- decisions, assumptions, approvals, rejections, and constraints
- errors encountered and fixes attempted
- current state at the end of the segment

Do not claim to be the user. Do not invent instructions, approvals, file changes, or tool results. Do not quote hidden system or plan text. Write concise Markdown suitable for an assistant message.";

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
    if message.role == "system" || message.role == "user" {
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

fn first_eligible_segment(messages: &[ChatMessage]) -> Option<SummarySegment> {
    closed_non_user_segments(messages)
        .into_iter()
        .find(|segment| !segment_is_matching_summary(messages, *segment))
}

fn estimated_context_pressure(messages: &[ChatMessage], effective_n_ctx: usize) -> ContextPressure {
    let visible_messages = filter_ui_only_messages(messages.to_vec());
    compute_context_budget(&visible_messages, effective_n_ctx).pressure
}

fn role_label(role: &str) -> &str {
    match role {
        "assistant" => "ASSISTANT",
        "tool" | "diff" => "TOOL",
        "context_file" => "CONTEXT_FILE",
        "event" => "EVENT",
        "error" => "ERROR",
        "cd_instruction" => "INSTRUCTION",
        other => other,
    }
}

fn segment_text(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|message| {
            let content = message.content.content_text_only();
            let mut parts = vec![format!("[{}]", role_label(&message.role))];
            if !message.tool_call_id.is_empty() {
                parts.push(format!("tool_call_id={}", message.tool_call_id));
            }
            if let Some(tool_calls) = &message.tool_calls {
                if !tool_calls.is_empty() {
                    let calls: Vec<String> = tool_calls
                        .iter()
                        .map(|call| format!("{}({})", call.function.name, call.id))
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
) -> Result<String, SegmentSummaryFailure> {
    let summarize_messages = vec![
        ChatMessage::new("system".to_string(), SEGMENT_SUMMARY_PROMPT.to_string()),
        ChatMessage::new(
            "user".to_string(),
            format!("Summarize this non-user segment:\n\n{}", text),
        ),
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

    Ok(result
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .map(|message| message.content.content_text_only())
        .unwrap_or_else(|| "Summary unavailable".to_string()))
}

fn make_segment_summary_message(
    summary: String,
    source_messages: &[ChatMessage],
    summary_model: &str,
) -> ChatMessage {
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
) -> Result<ChatMessage, SegmentSummaryFailure> {
    let mut text = segment_text(messages);
    let max_new_tokens = (model_n_ctx / 4).min(4096).max(512);
    let input_budget_tokens = model_n_ctx
        .saturating_sub(max_new_tokens)
        .saturating_sub(SEGMENT_SUMMARY_OVERHEAD_TOKENS);
    let input_budget_chars = input_budget_tokens.saturating_mul(3);
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

    let summary =
        summarize_segment_text(gcx, text, model.clone(), model_n_ctx, max_new_tokens).await?;
    Ok(make_segment_summary_message(summary, messages, &model))
}

async fn resolve_summary_model(
    gcx: Arc<GlobalContext>,
    thread_model: &str,
) -> Result<(String, usize), SegmentSummaryFailure> {
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .map_err(|e| SegmentSummaryFailure::Transient(e.message.clone()))?;
    let model = if !caps.defaults.chat_light_model.is_empty() {
        caps.defaults.chat_light_model.clone()
    } else if !caps.defaults.chat_default_model.is_empty() {
        caps.defaults.chat_default_model.clone()
    } else if !thread_model.is_empty() {
        thread_model.to_string()
    } else {
        return Err(SegmentSummaryFailure::NoModelAvailable);
    };
    let model_rec = crate::caps::resolve_chat_model(caps, &model)
        .map_err(|_| SegmentSummaryFailure::NoModelAvailable)?;
    let model_n_ctx = if model_rec.base.n_ctx > 0 {
        model_rec.base.n_ctx
    } else {
        16384
    };
    Ok((model, model_n_ctx))
}

async fn effective_n_ctx_for_thread(
    gcx: Arc<GlobalContext>,
    thread: &crate::chat::types::ThreadParams,
) -> Option<usize> {
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .ok()?;
    crate::caps::resolve_chat_model(caps, &thread.model)
        .ok()
        .map(|record| {
            let model_n_ctx = if record.base.n_ctx > 0 {
                record.base.n_ctx
            } else {
                crate::chat::config::tokens().default_n_ctx
            };
            match thread.context_tokens_cap {
                Some(cap) if cap > 0 => cap.min(model_n_ctx),
                _ => model_n_ctx,
            }
        })
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

pub fn summarize_oldest_segment_with_static_summary(
    messages: &mut Vec<ChatMessage>,
    summary_text: &str,
    summary_model: &str,
) -> bool {
    let Some(segment) = first_eligible_segment(messages) else {
        return false;
    };
    let source_messages = messages[segment.start..=segment.end].to_vec();
    let summary =
        make_segment_summary_message(summary_text.to_string(), &source_messages, summary_model);
    replace_segment(messages, segment, summary);
    true
}

pub async fn apply_segment_summarization(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    thread: &crate::chat::types::ThreadParams,
    force: bool,
) -> bool {
    if !thread.auto_compact_enabled_effective() {
        return false;
    }

    let raw_messages = {
        let session = session_arc.lock().await;
        if session.tier1_compaction_disabled && !force {
            return false;
        }
        if session.tier1_compact_attempts >= MAX_SEGMENT_SUMMARY_ATTEMPTS && !force {
            return false;
        }
        if last_visible_has_pending_tool_calls(&session.messages) {
            return false;
        }
        session.messages.clone()
    };

    let Some(segment) = first_eligible_segment(&raw_messages) else {
        return false;
    };
    let effective_n_ctx = match effective_n_ctx_for_thread(gcx.clone(), thread).await {
        Some(value) => value,
        None => return false,
    };
    let pressure = estimated_context_pressure(&raw_messages, effective_n_ctx);
    if !force && !matches!(pressure, ContextPressure::High | ContextPressure::Critical) {
        return false;
    }

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
            return false;
        }
    };

    let source_messages = raw_messages[segment.start..=segment.end].to_vec();
    let source_hash = source_hash_for_messages(&source_messages);
    info!(
        "Segment summarization attempting messages {}..={} ({} msgs, source_hash={})",
        segment.start,
        segment.end,
        source_messages.len(),
        source_hash,
    );

    match summarize_segment(gcx, &source_messages, model, model_n_ctx).await {
        Ok(summary) => {
            let mut session = session_arc.lock().await;
            let Some(current_segment) = first_eligible_segment(&session.messages) else {
                return false;
            };
            let current_source =
                session.messages[current_segment.start..=current_segment.end].to_vec();
            if source_hash_for_messages(&current_source) != source_hash {
                warn!("Segment summarization skipped because source messages changed while summarizing");
                return false;
            }
            replace_segment(&mut session.messages, current_segment, summary);
            session.tier1_compact_attempts += 1;
            session.tier1_compaction_disabled = false;
            session.thread.previous_response_id = None;
            session.cache_guard_force_next = true;
            session.increment_version();
            session.touch();
            info!(
                "Segment summarization applied, messages count now {}",
                session.messages.len()
            );
            true
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
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

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

    fn event(text: &str) -> ChatMessage {
        crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            "test.summarization",
            json!({}),
            text.to_string(),
        )
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
        assert_eq!(compression["schema_version"], json!(1));
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
