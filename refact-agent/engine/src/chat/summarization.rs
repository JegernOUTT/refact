//! Context-pressure helpers.
use crate::call_validation::{ChatMessage, ChatUsage};
use crate::chat::diagnostics::{filter_ui_only_messages, is_ui_only_message};
use crate::chat::history_limit::{pressure_for_used_tokens, ContextPressure};
use refact_chat_history::trajectory_ops::COMPRESSION_REPORT_ROLE;
fn pressure_rank(pressure: &ContextPressure) -> usize {
    match pressure {
        ContextPressure::Low => 0,
        ContextPressure::Medium => 1,
        ContextPressure::High => 2,
        ContextPressure::Critical => 3,
    }
}

fn max_pressure(left: ContextPressure, right: ContextPressure) -> ContextPressure {
    if pressure_rank(&left) >= pressure_rank(&right) {
        left
    } else {
        right
    }
}

fn provider_usage_input_tokens(usage: &ChatUsage) -> Option<usize> {
    if usage.prompt_tokens > 0
        || usage.cache_read_tokens.is_some()
        || usage.cache_creation_tokens.is_some()
    {
        return Some(
            usage
                .prompt_tokens
                .saturating_add(usage.cache_read_tokens.unwrap_or(0))
                .saturating_add(usage.cache_creation_tokens.unwrap_or(0)),
        );
    }
    (usage.total_tokens > 0).then(|| usage.total_tokens.saturating_sub(usage.completion_tokens))
}

fn recent_provider_usage_input_tokens(messages: &[ChatMessage]) -> Option<usize> {
    recent_provider_usage_anchor(messages).map(|(_, tokens)| tokens)
}

fn recent_provider_usage_anchor(messages: &[ChatMessage]) -> Option<(usize, usize)> {
    messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, message)| {
            if message.role != "assistant" || is_ui_only_message(message) {
                return None;
            }
            message
                .usage
                .as_ref()
                .and_then(provider_usage_input_tokens)
                .map(|tokens| (index, tokens))
        })
}

fn provider_visible_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let Ok(view) = refact_core::active_context::active_context(messages) else {
        return Vec::new();
    };
    filter_ui_only_messages(view.messages)
        .into_iter()
        .filter(|message| message.role != COMPRESSION_REPORT_ROLE)
        .collect()
}

pub(crate) fn estimated_provider_context_pressure_with_usage_for_image_mode(
    messages: &[ChatMessage],
    effective_n_ctx: usize,
    usage_stale: bool,
    image_token_mode: refact_core::provider_types::ImageTokenMode,
) -> ContextPressure {
    let provider_messages = provider_visible_messages(messages);
    let provider_pressure =
        refact_chat_history::history_limit::compute_context_budget_for_image_mode(
            &provider_messages,
            effective_n_ctx,
            image_token_mode,
        )
        .pressure;
    if usage_stale {
        return provider_pressure;
    }
    let usage_pressure = recent_provider_usage_input_tokens(&provider_messages)
        .map(|used_tokens| pressure_for_used_tokens(used_tokens, effective_n_ctx))
        .unwrap_or(ContextPressure::Low);
    max_pressure(provider_pressure, usage_pressure)
}
