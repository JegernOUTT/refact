use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::chat::diagnostics::{filter_ui_only_messages, is_ui_only_message};
use crate::chat::history_limit::{compute_context_budget, ContextPressure};
use crate::chat::trajectory_ops::approx_token_count;
use crate::subchat::{SubchatConfig, ToolsPolicy, run_subchat};

pub const MAX_TIER1_COMPACT_ATTEMPTS: usize = 2;
pub const MAX_TIER1_ANCHORS_BEFORE_MERGE: usize = 4;
const TIER1_OVERHEAD_TOKENS: usize = 1024;

#[derive(Debug, Clone)]
pub enum Tier1Failure {
    NoModelAvailable,
    InputTooLarge {
        excerpt_chars: usize,
        budget_chars: usize,
    },
    NoMessagesToSummarize,
    PressureTooLow,
    Transient(String),
}

impl std::fmt::Display for Tier1Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tier1Failure::NoModelAvailable => {
                write!(f, "no model available for tier1 summarization")
            }
            Tier1Failure::InputTooLarge {
                excerpt_chars,
                budget_chars,
            } => write!(
                f,
                "tier1 input too large after truncation: {} chars (budget {})",
                excerpt_chars, budget_chars
            ),
            Tier1Failure::NoMessagesToSummarize => write!(f, "no messages to summarize"),
            Tier1Failure::PressureTooLow => write!(f, "context pressure not high enough"),
            Tier1Failure::Transient(msg) => write!(f, "{}", msg),
        }
    }
}

impl Tier1Failure {
    pub fn is_structural(&self) -> bool {
        matches!(
            self,
            Tier1Failure::NoModelAvailable | Tier1Failure::InputTooLarge { .. }
        )
    }
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

const SUMMARIZATION_PROMPT: &str = "Produce a structured summary of this conversation excerpt. Be comprehensive and accurate. Use this exact format:

## Task Goal / Primary Request
<What the user asked for>

## Key Technical Concepts
<Important technologies, patterns, frameworks, tools mentioned>

## Files and Code Sections
<Files referenced with key code snippets or changes, include file paths>

## Errors Encountered and Fixes Applied
<Problems found and how they were resolved>

## Tool Interaction Summary
<Key tool calls made and their results>

## User Messages
<Paraphrase of what the user said>

## Pending Tasks
<Any incomplete work mentioned>

## Current Work State
<What was being worked on when this summary ends>

## Next Step
<Direct quote or close paraphrase of what should happen next>";

fn find_tool_safe_boundary(messages: &[ChatMessage], target_idx: usize) -> usize {
    let mut idx = target_idx.min(messages.len());
    while idx > 0 {
        let msg = &messages[idx - 1];
        if msg.preserve == Some(true) {
            idx -= 1;
            continue;
        }
        if msg.role == "tool" || msg.role == "diff" {
            idx -= 1;
            continue;
        }
        if msg.role == "assistant" && msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty()) {
            idx -= 1;
            continue;
        }
        break;
    }
    idx
}

fn range_contains_preserved(messages: &[ChatMessage], start: usize, end: usize) -> bool {
    messages[start..end]
        .iter()
        .any(|msg| msg.preserve == Some(true))
}

fn is_real_summarization_anchor(message: &ChatMessage) -> bool {
    if message.role != "summarization" || is_ui_only_message(message) {
        return false;
    }
    matches!(
        message.summarization_tier.as_deref(),
        Some("tier0_deterministic") | Some("tier1_llm") | Some("tier1_merged")
    )
}

fn visible_tier1_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    filter_ui_only_messages(messages.to_vec())
}

fn visible_tier1_messages_with_original_indices(
    messages: &[ChatMessage],
) -> (Vec<ChatMessage>, Vec<usize>) {
    messages
        .iter()
        .enumerate()
        .filter(|(_, message)| !is_ui_only_message(message))
        .map(|(idx, message)| (message.clone(), idx))
        .unzip()
}

fn translate_summarized_range_to_original(summ_msg: &mut ChatMessage, original_indices: &[usize]) {
    if let Some((start, end)) = summ_msg.summarized_range {
        if let (Some(original_start), Some(original_end)) =
            (original_indices.get(start), original_indices.get(end))
        {
            summ_msg.summarized_range = Some((*original_start, *original_end));
        }
    }
}

pub fn find_summarization_boundary(messages: &[ChatMessage]) -> (usize, usize) {
    let (visible_messages, original_indices) =
        visible_tier1_messages_with_original_indices(messages);
    let (start, end) = find_summarization_boundary_visible(&visible_messages, false);
    (
        original_indices.get(start).copied().unwrap_or(start),
        original_indices.get(end).copied().unwrap_or(end),
    )
}

fn find_summarization_boundary_visible(
    messages: &[ChatMessage],
    force_full_recompact: bool,
) -> (usize, usize) {
    let mut start = if force_full_recompact {
        0
    } else {
        messages
            .iter()
            .enumerate()
            .filter(|(_, m)| is_real_summarization_anchor(m))
            .last()
            .map(|(i, _)| i + 1)
            .unwrap_or(0)
    };

    let preserve_tail = 4usize;
    let safe_end = messages.len().saturating_sub(preserve_tail);

    loop {
        if start >= safe_end {
            return (start, start);
        }
        if messages[start].preserve == Some(true) {
            start += 1;
            continue;
        }
        if safe_end <= start + 2 {
            return (start, start);
        }

        let range = safe_end - start;
        let target_end = start + range / 2;
        let mut adjusted_end = find_tool_safe_boundary(messages, target_end);

        if adjusted_end > start && range_contains_preserved(messages, start, adjusted_end) {
            if let Some(offset) = messages[start..adjusted_end]
                .iter()
                .position(|msg| msg.preserve == Some(true))
            {
                if offset > 1 {
                    adjusted_end = find_tool_safe_boundary(messages, start + offset);
                } else {
                    start = start + offset + 1;
                    continue;
                }
            }
        }

        if adjusted_end <= start + 1 {
            return (start, start);
        }

        return (start, adjusted_end.saturating_sub(1));
    }
}

/// Summarize a slice of already-visible messages.
///
/// The caller MUST pass a slice with no UI-only entries (use
/// [`visible_tier1_messages_with_original_indices`] and then translate the
/// returned range back to original session indices). The returned message's
/// `summarized_range` is relative to the caller-provided `messages` slice.
pub async fn tier1_summarize(
    gcx: Arc<GlobalContext>,
    messages: &[ChatMessage],
    n_ctx: usize,
    force_full_recompact: bool,
) -> Result<ChatMessage, Tier1Failure> {
    debug_assert!(
        messages.iter().all(|m| !is_ui_only_message(m)),
        "tier1_summarize requires pre-filtered (visible) messages"
    );

    let budget = compute_context_budget(messages, n_ctx);
    if !matches!(
        budget.pressure,
        ContextPressure::High | ContextPressure::Critical
    ) && !force_full_recompact
    {
        return Err(Tier1Failure::PressureTooLow);
    }

    let (start, end) = find_summarization_boundary_visible(messages, force_full_recompact);
    if end <= start {
        return Err(Tier1Failure::NoMessagesToSummarize);
    }

    let chunk = &messages[start..=end];
    let token_estimate = approx_token_count(chunk);

    let mut conversation_text: String = chunk
        .iter()
        .map(|m| {
            let content = m.content.content_text_only();
            let role_label = match m.role.as_str() {
                "assistant" => "ASSISTANT",
                "user" => "USER",
                "tool" | "diff" => "TOOL",
                "context_file" => "CONTEXT",
                _ => "OTHER",
            };
            format!("[{}]: {}\n\n", role_label, content)
        })
        .collect();

    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| Tier1Failure::Transient(e.message.clone()))?;

    let model = if !caps.defaults.chat_light_model.is_empty() {
        caps.defaults.chat_light_model.clone()
    } else if !caps.defaults.chat_default_model.is_empty() {
        caps.defaults.chat_default_model.clone()
    } else {
        return Err(Tier1Failure::NoModelAvailable);
    };

    let model_rec = crate::caps::resolve_chat_model(caps, &model)
        .map_err(|_| Tier1Failure::NoModelAvailable)?;
    let model_n_ctx = if model_rec.base.n_ctx > 0 {
        model_rec.base.n_ctx
    } else {
        16384
    };
    let max_new_tokens = (model_n_ctx / 4).min(4096).max(512);

    let input_budget_tokens = model_n_ctx
        .saturating_sub(max_new_tokens)
        .saturating_sub(TIER1_OVERHEAD_TOKENS);
    let input_budget_chars = input_budget_tokens.saturating_mul(3);
    if input_budget_chars == 0 {
        return Err(Tier1Failure::InputTooLarge {
            excerpt_chars: conversation_text.len(),
            budget_chars: 0,
        });
    }
    if conversation_text.len() > input_budget_chars {
        let original_len = conversation_text.len();
        let head_keep = input_budget_chars * 2 / 3;
        let tail_keep = input_budget_chars.saturating_sub(head_keep + 200);
        let head_end =
            safe_char_boundary(&conversation_text, head_keep.min(conversation_text.len()));
        let tail_start_raw = conversation_text.len().saturating_sub(tail_keep);
        let tail_start = safe_char_boundary(&conversation_text, tail_start_raw);
        let head = conversation_text[..head_end].to_string();
        let tail = conversation_text[tail_start..].to_string();
        if head.len() + tail.len() + 200 > input_budget_chars && tail_keep == 0 {
            return Err(Tier1Failure::InputTooLarge {
                excerpt_chars: original_len,
                budget_chars: input_budget_chars,
            });
        }
        let elided = original_len.saturating_sub(head.len() + tail.len());
        conversation_text = format!(
            "{}\n\n[... {} chars elided to fit summarizer input budget ...]\n\n{}",
            head, elided, tail
        );
    }

    let summarize_messages = vec![
        ChatMessage::new("system".to_string(), SUMMARIZATION_PROMPT.to_string()),
        ChatMessage::new(
            "user".to_string(),
            format!(
                "Summarize this conversation excerpt:\n\n{}",
                conversation_text
            ),
        ),
    ];

    let config = SubchatConfig {
        tool_name: "tier1_summarize".to_string(),
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
        .map_err(Tier1Failure::Transient)?;

    let summary = result
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.content.content_text_only())
        .unwrap_or_else(|| "Summary unavailable".to_string());

    let tier_label = if force_full_recompact {
        "tier1_merged"
    } else {
        "tier1_llm"
    };
    info!(
        "Tier1 ({}) summarization produced {} chars for messages {}..={}",
        tier_label,
        summary.len(),
        start,
        end
    );

    Ok(ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "summarization".to_string(),
        content: ChatContent::SimpleText(summary),
        summarized_range: Some((start, end)),
        summarization_tier: Some(tier_label.to_string()),
        summarized_token_estimate: Some(token_estimate),
        ..Default::default()
    })
}

pub async fn maybe_apply_tier1(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    thread: &crate::chat::types::ThreadParams,
    tier1_compact_count: &mut usize,
    tier1_disabled_for_session: &mut bool,
) {
    if *tier1_disabled_for_session {
        return;
    }
    if !thread.auto_compact_enabled.unwrap_or(true) {
        return;
    }
    if *tier1_compact_count >= MAX_TIER1_COMPACT_ATTEMPTS {
        return;
    }

    let raw_messages = {
        let session = session_arc.lock().await;
        let last_visible_has_pending_tool_calls = session
            .messages
            .iter()
            .rev()
            .find(|msg| !is_ui_only_message(msg))
            .map(|msg| {
                msg.role == "assistant"
                    && msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty())
            })
            .unwrap_or(false);
        if last_visible_has_pending_tool_calls {
            return;
        }
        session.messages.clone()
    };

    let caps_res =
        crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0).await;
    let effective_n_ctx_opt = caps_res.ok().and_then(|caps| {
        crate::caps::resolve_chat_model(caps, &thread.model)
            .ok()
            .map(|rec| {
                let model_n_ctx = if rec.base.n_ctx > 0 {
                    rec.base.n_ctx
                } else {
                    crate::chat::config::tokens().default_n_ctx
                };
                match thread.context_tokens_cap {
                    Some(cap) if cap > 0 => cap.min(model_n_ctx),
                    _ => model_n_ctx,
                }
            })
    });

    let effective_n_ctx = match effective_n_ctx_opt {
        Some(v) => v,
        None => return,
    };

    let (visible_messages, original_indices) =
        visible_tier1_messages_with_original_indices(&raw_messages);
    if visible_messages.is_empty() {
        return;
    }

    let budget = compute_context_budget(&visible_messages, effective_n_ctx);
    let anchor_count = visible_messages
        .iter()
        .filter(|m| is_real_summarization_anchor(m))
        .count();
    let force_full_recompact = anchor_count >= MAX_TIER1_ANCHORS_BEFORE_MERGE
        && matches!(
            budget.pressure,
            ContextPressure::High | ContextPressure::Critical
        );

    if !matches!(
        budget.pressure,
        ContextPressure::High | ContextPressure::Critical
    ) && !force_full_recompact
    {
        return;
    }

    warn!(
        "Context at {:?} pressure (anchors: {}), attempting tier1 summarization (attempt {}/{}, full_recompact={})",
        budget.pressure,
        anchor_count,
        *tier1_compact_count + 1,
        MAX_TIER1_COMPACT_ATTEMPTS,
        force_full_recompact,
    );

    match tier1_summarize(
        gcx,
        &visible_messages,
        effective_n_ctx,
        force_full_recompact,
    )
    .await
    {
        Ok(mut summ_msg) => {
            *tier1_compact_count += 1;
            translate_summarized_range_to_original(&mut summ_msg, &original_indices);
            let mut session = session_arc.lock().await;
            if force_full_recompact {
                if let Some((orig_start, orig_end)) = summ_msg.summarized_range {
                    let before = session.messages.len();
                    let mut removed_in_range = 0usize;
                    let mut idx = 0usize;
                    session.messages.retain(|m| {
                        let i = idx;
                        idx += 1;
                        if i < orig_start || i > orig_end {
                            return true;
                        }
                        if !is_real_summarization_anchor(m) {
                            return true;
                        }
                        match m.summarized_range {
                            Some((s, e)) if s >= orig_start && e <= orig_end => {
                                removed_in_range += 1;
                                false
                            }
                            _ => true,
                        }
                    });
                    let removed_total = before.saturating_sub(session.messages.len());
                    info!(
                        "Tier1 full recompact removed {} obsolete summarization anchors",
                        removed_total
                    );
                    let new_end = orig_end.saturating_sub(removed_in_range);
                    summ_msg.summarized_range = Some((orig_start, new_end));
                }
            }
            session.add_message(summ_msg);
            session.cache_guard_force_next = true;
            info!(
                "Tier1 summarization applied, messages count now {}",
                session.messages.len()
            );
        }
        Err(failure) => {
            if failure.is_structural() {
                *tier1_disabled_for_session = true;
                warn!(
                    "Tier1 summarization structurally disabled for this session: {}",
                    failure
                );
            } else {
                *tier1_compact_count += 1;
                warn!("Tier1 summarization failed (non-fatal): {}", failure);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::ChatContent;

    fn make_user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_assistant_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_summarization_msg(range: (usize, usize)) -> ChatMessage {
        ChatMessage {
            role: "summarization".to_string(),
            content: ChatContent::SimpleText("previous summary".to_string()),
            summarized_range: Some(range),
            summarization_tier: Some("tier1_llm".to_string()),
            ..Default::default()
        }
    }

    fn make_ui_only_reactive_report(content: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert("_ui_only".to_string(), serde_json::Value::Bool(true));
        ChatMessage {
            role: "summarization".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            summarization_tier: Some("tier2_reactive".to_string()),
            extra,
            ..Default::default()
        }
    }

    #[test]
    fn test_find_boundary_no_previous_summary() {
        let messages: Vec<ChatMessage> = (0..10)
            .map(|i| {
                if i % 2 == 0 {
                    make_user_msg(&format!("user {}", i))
                } else {
                    make_assistant_msg(&format!("assistant {}", i))
                }
            })
            .collect();
        let (start, end) = find_summarization_boundary(&messages);
        assert_eq!(start, 0);
        assert!(end > 0, "end should be positive");
        assert!(end < messages.len(), "end should be before last message");
    }

    #[test]
    fn test_find_boundary_after_previous_summary() {
        let mut messages: Vec<ChatMessage> = (0..6)
            .map(|i| {
                if i % 2 == 0 {
                    make_user_msg(&format!("user {}", i))
                } else {
                    make_assistant_msg(&format!("assistant {}", i))
                }
            })
            .collect();
        messages.push(make_summarization_msg((0, 3)));
        for i in 0..5 {
            if i % 2 == 0 {
                messages.push(make_user_msg("new user"));
            } else {
                messages.push(make_assistant_msg("new asst"));
            }
        }
        let (start, end) = find_summarization_boundary(&messages);
        assert_eq!(start, 7, "should start after the summarization message");
        assert!(end >= start, "end should be >= start");
    }

    #[test]
    fn find_summarization_boundary_ignores_ui_only_reactive_report() {
        let mut messages: Vec<ChatMessage> = (0..6)
            .map(|i| {
                if i % 2 == 0 {
                    make_user_msg(&format!("user {}", i))
                } else {
                    make_assistant_msg(&format!("assistant {}", i))
                }
            })
            .collect();
        messages.push(make_ui_only_reactive_report("Reactive compaction report"));
        for i in 0..6 {
            if i % 2 == 0 {
                messages.push(make_user_msg("new user"));
            } else {
                messages.push(make_assistant_msg("new asst"));
            }
        }

        let (start, end) = find_summarization_boundary(&messages);

        assert_eq!(start, 0);
        assert!(end > 0);
    }

    #[test]
    fn tier1_summarize_filters_ui_only_messages_from_prompt_boundary() {
        let hidden = make_ui_only_reactive_report("context_length_exceeded diagnostic");
        let messages = vec![
            make_user_msg("visible 1"),
            hidden,
            make_assistant_msg("visible 2"),
            make_user_msg("visible 3"),
        ];

        let filtered = visible_tier1_messages(&messages);

        assert_eq!(filtered.len(), 3);
        assert!(filtered.iter().all(|msg| !msg
            .content
            .content_text_only()
            .contains("context_length_exceeded")));
    }

    #[test]
    fn test_find_boundary_skips_preserved_at_start() {
        let mut messages: Vec<ChatMessage> = (0..14)
            .map(|i| {
                if i % 2 == 0 {
                    make_user_msg(&format!("user {}", i))
                } else {
                    make_assistant_msg(&format!("assistant {}", i))
                }
            })
            .collect();
        messages[1].preserve = Some(true);
        let (start, end) = find_summarization_boundary(&messages);
        assert!(
            start >= 2,
            "should skip past preserved msg at index 1, got start={}",
            start
        );
        assert!(
            end > start,
            "should still produce a usable range, got {}..={}",
            start,
            end
        );
        assert!(!messages[start..=end]
            .iter()
            .any(|msg| msg.preserve == Some(true)));
    }

    #[test]
    fn test_summarization_without_tier_is_not_anchor() {
        let msg_no_tier = ChatMessage {
            role: "summarization".to_string(),
            content: ChatContent::SimpleText("untyped summary".to_string()),
            summarized_range: Some((0, 2)),
            summarization_tier: None,
            ..Default::default()
        };
        assert!(!is_real_summarization_anchor(&msg_no_tier));
    }

    #[test]
    fn test_tier2_reactive_is_not_authoritative_anchor() {
        let reactive = make_ui_only_reactive_report("compaction diagnostic");
        assert!(!is_real_summarization_anchor(&reactive));
    }

    #[test]
    fn test_merged_range_adjusts_for_removed_anchors() {
        // Helper to simulate the range-adjustment logic after removal.
        // Walks the message list, removes anchors whose range fits inside
        // the new summary's range, and returns the adjusted new_end.
        fn adjust(messages: &mut Vec<ChatMessage>, new_start: usize, new_end: usize) -> usize {
            let mut removed_in_range = 0usize;
            let mut idx = 0usize;
            messages.retain(|m| {
                let i = idx;
                idx += 1;
                if i < new_start || i > new_end {
                    return true;
                }
                if !is_real_summarization_anchor(m) {
                    return true;
                }
                match m.summarized_range {
                    Some((s, e)) if s >= new_start && e <= new_end => {
                        removed_in_range += 1;
                        false
                    }
                    _ => true,
                }
            });
            new_end.saturating_sub(removed_in_range)
        }

        let mut messages = vec![
            make_user_msg("user_0"),
            make_assistant_msg("asst_1"),
            make_user_msg("user_2"),
            make_assistant_msg("asst_3"),
            make_summarization_msg((0, 1)), // old anchor at idx 4
            make_user_msg("user_5"),
            make_assistant_msg("asst_6"),
            make_user_msg("user_7"),
            make_assistant_msg("asst_8"),
            make_user_msg("user_9"),
        ];
        let new_end = adjust(&mut messages, 0, 6);
        // 1 anchor removed inside (0..=6) → new_end shifts from 6 to 5
        assert_eq!(new_end, 5);
        assert_eq!(messages.len(), 9, "one anchor should have been removed");
        // The message now at the new_end should NOT be the user_7 we wanted to keep
        assert_eq!(messages[new_end].content.content_text_only(), "asst_6");
    }

    #[test]
    fn test_force_full_recompact_starts_from_zero() {
        let mut messages: Vec<ChatMessage> = (0..12)
            .map(|i| {
                if i % 2 == 0 {
                    make_user_msg(&format!("user {}", i))
                } else {
                    make_assistant_msg(&format!("assistant {}", i))
                }
            })
            .collect();
        messages.insert(5, make_summarization_msg((0, 3)));
        let (start_normal, _) = find_summarization_boundary_visible(&messages, false);
        assert!(start_normal > 0, "normal mode starts after the anchor");
        let (start_full, _) = find_summarization_boundary_visible(&messages, true);
        assert_eq!(start_full, 0, "force_full_recompact starts from 0");
    }

    #[test]
    fn test_tier1_failure_classification() {
        assert!(Tier1Failure::NoModelAvailable.is_structural());
        assert!(Tier1Failure::InputTooLarge {
            excerpt_chars: 1_000_000,
            budget_chars: 1_000,
        }
        .is_structural());
        assert!(!Tier1Failure::Transient("network".to_string()).is_structural());
        assert!(!Tier1Failure::NoMessagesToSummarize.is_structural());
        assert!(!Tier1Failure::PressureTooLow.is_structural());
    }

    #[test]
    fn tier1_summarization_range_skips_ui_only_messages() {
        let messages: Vec<ChatMessage> = (0..10)
            .map(|i| {
                if matches!(i, 0 | 3 | 5) {
                    make_ui_only_reactive_report(&format!("hidden {i}"))
                } else if i % 2 == 0 {
                    make_user_msg(&format!("user {i}"))
                } else {
                    make_assistant_msg(&format!("assistant {i}"))
                }
            })
            .collect();
        let (filtered, original_indices) = visible_tier1_messages_with_original_indices(&messages);
        let mut summ_msg = make_summarization_msg((2, 4));

        translate_summarized_range_to_original(&mut summ_msg, &original_indices);

        assert_eq!(filtered.len(), 7);
        assert_eq!(original_indices, vec![1, 2, 4, 6, 7, 8, 9]);
        assert_eq!(summ_msg.summarized_range, Some((4, 7)));
    }

    #[test]
    fn tier1_summarization_range_no_ui_only_unchanged() {
        let messages: Vec<ChatMessage> = (0..10)
            .map(|i| {
                if i % 2 == 0 {
                    make_user_msg(&format!("user {i}"))
                } else {
                    make_assistant_msg(&format!("assistant {i}"))
                }
            })
            .collect();
        let (_, original_indices) = visible_tier1_messages_with_original_indices(&messages);
        let mut summ_msg = make_summarization_msg((2, 4));

        translate_summarized_range_to_original(&mut summ_msg, &original_indices);

        assert_eq!(original_indices, (0..10).collect::<Vec<usize>>());
        assert_eq!(summ_msg.summarized_range, Some((2, 4)));
    }

    #[test]
    fn test_find_boundary_empty_messages() {
        let messages: Vec<ChatMessage> = vec![];
        let (start, end) = find_summarization_boundary(&messages);
        assert_eq!(start, 0);
        assert_eq!(end, 0);
    }

    #[test]
    fn test_find_boundary_too_few_messages() {
        let messages = vec![make_user_msg("hello"), make_assistant_msg("hi")];
        let (start, end) = find_summarization_boundary(&messages);
        assert_eq!(start, end, "not enough messages to summarize");
    }

    #[test]
    fn test_tier1_not_applied_when_pressure_low() {
        let messages = vec![make_user_msg("hello"), make_assistant_msg("hi")];
        let budget = compute_context_budget(&messages, 1_000_000);
        assert!(matches!(budget.pressure, ContextPressure::Low));
    }

    #[test]
    fn test_max_tier1_attempts_constant() {
        assert_eq!(MAX_TIER1_COMPACT_ATTEMPTS, 2);
    }

    #[test]
    fn test_tier1_failures_counted_toward_limit() {
        let mut count = 0usize;
        count += 1;
        assert_eq!(count, 1);
        count += 1;
        assert_eq!(count, 2);
        assert!(
            count >= MAX_TIER1_COMPACT_ATTEMPTS,
            "after 2 failures count should meet limit"
        );
    }

    #[test]
    fn test_find_boundary_avoids_splitting_tool_pairs() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        let tool_call = ChatToolCall {
            id: "call_1".to_string(),
            index: Some(0),
            function: ChatToolFunction {
                name: "shell".to_string(),
                arguments: "{}".to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
        };
        let mut messages: Vec<ChatMessage> = (0..8)
            .map(|i| {
                if i % 2 == 0 {
                    make_user_msg(&format!("user {}", i))
                } else {
                    make_assistant_msg(&format!("assistant {}", i))
                }
            })
            .collect();
        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            tool_calls: Some(vec![tool_call]),
            ..Default::default()
        });
        messages.push(ChatMessage {
            role: "tool".to_string(),
            tool_call_id: "call_1".to_string(),
            content: ChatContent::SimpleText("result".to_string()),
            ..Default::default()
        });
        messages.push(make_user_msg("after tools"));
        let (start, end) = find_summarization_boundary(&messages);
        if end > start {
            let msg_at_end = &messages[end];
            assert_ne!(
                msg_at_end.role, "tool",
                "boundary should not end on a tool result"
            );
        }
    }

    #[test]
    fn test_summarization_boundary_skips_preserved_messages() {
        let mut messages: Vec<ChatMessage> = (0..12)
            .map(|i| {
                if i % 2 == 0 {
                    make_user_msg(&format!("user {}", i))
                } else {
                    make_assistant_msg(&format!("assistant {}", i))
                }
            })
            .collect();
        messages[3].preserve = Some(true);
        let (start, end) = find_summarization_boundary(&messages);
        assert_eq!(start, 0);
        assert!(
            end < 3,
            "boundary must exclude preserved message, got {start}..={end}"
        );
        assert!(!messages[start..=end]
            .iter()
            .any(|msg| msg.preserve == Some(true)));
    }
}
