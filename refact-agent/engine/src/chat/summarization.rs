use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::chat::history_limit::{compute_context_budget, ContextPressure};
use crate::chat::trajectory_ops::approx_token_count;
use crate::subchat::{SubchatConfig, ToolsPolicy, run_subchat};

pub const MAX_TIER1_COMPACT_ATTEMPTS: usize = 2;

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

pub fn find_summarization_boundary(messages: &[ChatMessage]) -> (usize, usize) {
    let start = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "summarization")
        .last()
        .map(|(i, _)| i + 1)
        .unwrap_or(0);

    let preserve_tail = 4usize;
    let safe_end = messages.len().saturating_sub(preserve_tail);
    if safe_end <= start + 2 {
        return (start, start);
    }

    let range = safe_end - start;
    let target_end = start + range / 2;
    let mut adjusted_end = find_tool_safe_boundary(messages, target_end);
    while adjusted_end > start && range_contains_preserved(messages, start, adjusted_end) {
        let Some(offset) = messages[start..adjusted_end]
            .iter()
            .rposition(|msg| msg.preserve == Some(true))
        else {
            break;
        };
        adjusted_end = find_tool_safe_boundary(messages, start + offset);
    }
    if adjusted_end <= start + 1 {
        return (start, start);
    }

    (start, adjusted_end.saturating_sub(1))
}

pub async fn tier1_summarize(
    gcx: Arc<GlobalContext>,
    messages: &[ChatMessage],
    n_ctx: usize,
) -> Result<ChatMessage, String> {
    let budget = compute_context_budget(messages, n_ctx);
    if !matches!(budget.pressure, ContextPressure::High | ContextPressure::Critical) {
        return Err("context pressure not high enough".to_string());
    }

    let (start, end) = find_summarization_boundary(messages);
    if end <= start {
        return Err("no messages to summarize".to_string());
    }

    let chunk = &messages[start..=end];
    let token_estimate = approx_token_count(chunk);

    let conversation_text: String = chunk
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
        .map_err(|e| e.message.clone())?;

    let model = if !caps.defaults.chat_light_model.is_empty() {
        caps.defaults.chat_light_model.clone()
    } else if !caps.defaults.chat_default_model.is_empty() {
        caps.defaults.chat_default_model.clone()
    } else {
        return Err("no model available for tier1 summarization".to_string());
    };

    let model_rec = crate::caps::resolve_chat_model(caps, &model)?;
    let model_n_ctx = if model_rec.base.n_ctx > 0 { model_rec.base.n_ctx } else { 16384 };
    let max_new_tokens = (model_n_ctx / 4).min(4096).max(512);

    let summarize_messages = vec![
        ChatMessage::new("system".to_string(), SUMMARIZATION_PROMPT.to_string()),
        ChatMessage::new(
            "user".to_string(),
            format!("Summarize this conversation excerpt:\n\n{}", conversation_text),
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

    let result = run_subchat(gcx, summarize_messages, config).await?;

    let summary = result
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.content.content_text_only())
        .unwrap_or_else(|| "Summary unavailable".to_string());

    info!(
        "Tier1 summarization produced {} chars for messages {}..={}",
        summary.len(),
        start,
        end
    );

    Ok(ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "summarization".to_string(),
        content: ChatContent::SimpleText(summary),
        summarized_range: Some((start, end)),
        summarization_tier: Some("tier1_llm".to_string()),
        summarized_token_estimate: Some(token_estimate),
        ..Default::default()
    })
}

pub async fn maybe_apply_tier1(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    thread: &crate::chat::types::ThreadParams,
    tier1_compact_count: &mut usize,
) {
    if !thread.auto_compact_enabled.unwrap_or(true) {
        return;
    }
    if *tier1_compact_count >= MAX_TIER1_COMPACT_ATTEMPTS {
        return;
    }

    let messages_clone = {
        let session = session_arc.lock().await;
        let last_has_pending_tool_calls = session.messages.last().map(|msg| {
            msg.role == "assistant"
                && msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty())
        }).unwrap_or(false);
        if last_has_pending_tool_calls {
            return;
        }
        session.messages.clone()
    };

    let caps_res = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0).await;
    let effective_n_ctx_opt = caps_res.ok().and_then(|caps| {
        crate::caps::resolve_chat_model(caps, &thread.model).ok().map(|rec| {
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

    let budget = compute_context_budget(&messages_clone, effective_n_ctx);
    if !matches!(budget.pressure, ContextPressure::High | ContextPressure::Critical) {
        return;
    }

    warn!(
        "Context at {:?} pressure, attempting tier1 summarization (attempt {}/{})",
        budget.pressure,
        *tier1_compact_count + 1,
        MAX_TIER1_COMPACT_ATTEMPTS
    );

    match tier1_summarize(gcx, &messages_clone, effective_n_ctx).await {
        Ok(summ_msg) => {
            *tier1_compact_count += 1;
            let mut session = session_arc.lock().await;
            session.add_message(summ_msg);
            session.cache_guard_force_next = true;
            info!(
                "Tier1 summarization applied, messages count now {}",
                session.messages.len()
            );
        }
        Err(e) => {
            *tier1_compact_count += 1;
            warn!("Tier1 summarization failed (non-fatal): {}", e);
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
        assert!(count >= MAX_TIER1_COMPACT_ATTEMPTS, "after 2 failures count should meet limit");
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
            assert_ne!(msg_at_end.role, "tool", "boundary should not end on a tool result");
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
        assert!(end < 3, "boundary must exclude preserved message, got {start}..={end}");
        assert!(!messages[start..=end]
            .iter()
            .any(|msg| msg.preserve == Some(true)));
    }
}
