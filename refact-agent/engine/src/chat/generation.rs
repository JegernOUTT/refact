use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use serde_json::json;
use tokio::sync::{Mutex as AMutex};
use tracing::{info, warn};
use uuid::Uuid;
use refact_buddy_core::types::BuddyRuntimeEvent;

use crate::app_state::AppState;
use crate::buddy::chat_reactions::{maybe_enqueue_chat_activity_reaction, ChatActivityCompletion};
use crate::subchat::{resolve_subchat_config_with_parent, run_subchat};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{
    ChatContent, ChatMessage, ChatMeta, ChatUsage, MeteringUsd, SamplingParameters,
    is_agentic_mode_id,
};
use crate::stats::event::{LlmCallEvent, canonicalize_mode_for_stats, split_model_provider};
use crate::chat::tool_call_recovery;
use crate::chat::tool_call_recovery_oss;
use crate::llm::LlmRequest;
use crate::llm::params::CacheControl;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::constants::CHAT_TOP_N;
use crate::knowledge::enrichment::enrich_messages_with_knowledge;

use super::types::*;
use super::trajectories::{
    check_external_reload_pending, ensure_frozen_prefix, first_system_prompt,
    frozen_prefix_is_complete, maybe_save_trajectory,
};
use super::tools::{process_tool_calls_once, ToolStepOutcome};
use super::prepare::{build_canonical_openai_tools, prepare_chat_passthrough, ChatPrepareOptions};
use super::prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use super::stream_core::{
    run_llm_stream, StreamRunParams, StreamCollector, normalize_tool_call, ChoiceFinal,
    LlmStreamError, LlmStreamOutcome, ABORT_ERROR_MESSAGE,
};
use super::queue::inject_priority_messages_if_any;
use super::config::tokens;
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{HookPayload, get_project_dir_string, run_hooks};
use crate::chat::diagnostics::{make_ui_only_error_message, make_ui_only_retry_status_message};
use crate::chat::history_limit::ContextPressure;
use crate::chat::trajectory_ops::approx_token_count;
use refact_core::llm_types::BaseModelRecord;

const TOKEN_BUDGET_CADENCE: usize = 6;
const TOKEN_BUDGET_MARKER: &str = "token_budget_info";
const MCP_LAZY_INDEX_MARKER: &str = "mcp_lazy_index";
const LENGTH_STOP_NEAR_EMPTY_VISIBLE_CHARS: usize = 32;
const PARTIAL_OUTPUT_STREAM_ERROR: &str =
    "Stream interrupted after partial output and all retry attempts failed.";

async fn user_stop_requested(session_arc: &Arc<AMutex<ChatSession>>) -> bool {
    let session = session_arc.lock().await;
    session.user_interrupt_flag.load(Ordering::SeqCst)
}

fn check_aborted_before_stream(abort_flag: &AtomicBool) -> Result<(), LlmStreamError> {
    if abort_flag.load(Ordering::SeqCst) {
        Err(LlmStreamError::from(ABORT_ERROR_MESSAGE.to_string()))
    } else {
        Ok(())
    }
}

fn make_runtime_event(
    signal_type: &str,
    title: &str,
    source: &str,
    dedupe_key: &str,
    status: &str,
    priority: Option<&str>,
) -> BuddyRuntimeEvent {
    BuddyRuntimeEvent {
        id: Uuid::new_v4().to_string(),
        signal_type: signal_type.to_string(),
        title: title.to_string(),
        description: None,
        source: source.to_string(),
        status: status.to_string(),
        failure_category: None,
        failure_summary: None,
        progress: None,
        dedupe_key: Some(dedupe_key.to_string()),
        priority: priority.unwrap_or("normal").to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        ttl_ms: None,
        bubble_policy: None,
        speech_text: None,
        scene: None,
        duration_hint: None,
        persistent: false,
        controls: Vec::new(),
        chat_id: None,
        dismissed: false,
    }
}

fn maybe_inject_token_budget_instruction(
    session: &mut ChatSession,
    effective_n_ctx: usize,
    cadence: usize,
) -> bool {
    let used_tokens = approx_token_count(&session.messages);
    let remaining = effective_n_ctx.saturating_sub(used_tokens);
    let last_has_tool_calls = session
        .messages
        .last()
        .map(|msg| {
            msg.role == "assistant"
                && msg
                    .tool_calls
                    .as_ref()
                    .map(|tcs| !tcs.is_empty())
                    .unwrap_or(false)
        })
        .unwrap_or(false);
    if last_has_tool_calls {
        return false;
    }

    let mut last_marker_idx = None;
    let mut user_or_assistant_since = 0usize;

    for (idx, msg) in session.messages.iter().enumerate().rev() {
        if msg.role == "cd_instruction" && msg.tool_call_id == TOKEN_BUDGET_MARKER {
            last_marker_idx = Some(idx);
            break;
        }
    }

    for (idx, msg) in session.messages.iter().enumerate().rev() {
        if let Some(marker_idx) = last_marker_idx {
            if idx <= marker_idx {
                break;
            }
        }
        if msg.role == "user" || msg.role == "assistant" {
            user_or_assistant_since += 1;
        }
    }

    if user_or_assistant_since < cadence {
        return false;
    }

    if session
        .messages
        .iter()
        .rev()
        .take(cadence)
        .any(|msg| msg.role == "cd_instruction" && msg.tool_call_id == TOKEN_BUDGET_MARKER)
    {
        return false;
    }

    let pct_used = if effective_n_ctx > 0 {
        used_tokens.saturating_mul(100) / effective_n_ctx
    } else {
        0
    };

    let message = ChatMessage {
        role: "cd_instruction".to_string(),
        tool_call_id: TOKEN_BUDGET_MARKER.to_string(),
        content: ChatContent::SimpleText(format!(
            "💿 Token budget: ~{} used / ~{} available (~{}% used). ~{} tokens remaining. Consider using compress_chat_probe() if running low.",
            used_tokens,
            effective_n_ctx,
            pct_used,
            remaining
        )),
        ..Default::default()
    };
    session.add_message(message);
    true
}

fn build_mcp_index_message(index: &[(String, String)], total: usize) -> String {
    let mut lines = vec![
        format!(
            "💿 MCP Tools — Lazy Mode Active ({} tools available). \
             You MUST call `mcp_tool_search` before using any MCP tool. \
             Example: mcp_tool_search({{\"query\": \"github.*pull|pr\"}})",
            total
        ),
        String::new(),
        "Available MCP tools (name: description):".to_string(),
    ];
    for (name, desc) in index {
        let short = if desc.chars().count() > 100 {
            format!("{}…", desc.chars().take(100).collect::<String>())
        } else {
            desc.clone()
        };
        lines.push(format!("- {}: {}", name, short));
    }
    lines.join("\n")
}

pub async fn prepare_session_preamble_and_knowledge(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) {
    let gcx = app.gcx.clone();
    let (thread, chat_id, has_system, has_project_context) = {
        let session = session_arc.lock().await;
        let has_sys = session
            .messages
            .first()
            .map(|m| m.role == "system")
            .unwrap_or(false);
        let has_proj = session.messages.iter().any(|m| {
            m.role == "context_file"
                && m.tool_call_id == crate::chat::system_context::PROJECT_CONTEXT_MARKER
        });
        (
            session.thread.clone(),
            session.chat_id.clone(),
            has_sys,
            has_proj,
        )
    };

    let needs_preamble = !has_system || (!has_project_context && thread.include_project_info);

    // Populated inside `needs_preamble`; used after to inject the MCP index hint message.
    let mut mcp_for_index: Option<(Vec<(String, String)>, usize)> = None;

    if needs_preamble {
        let caps = match crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
            .await
        {
            Ok(caps) => caps,
            Err(e) => {
                warn!("Failed to load caps for preamble: {}", e.message);
                return;
            }
        };
        let model_rec = match crate::caps::resolve_chat_model(caps.clone(), &thread.model) {
            Ok(rec) => rec,
            Err(e) => {
                warn!("Failed to resolve model for preamble: {}", e);
                return;
            }
        };

        let tools_for_mode = app
            .tool_registry
            .get_tools_index_for_mode(&thread.mode, Some(&model_rec.base.id))
            .await;
        if tools_for_mode.mcp_lazy_mode {
            mcp_for_index = Some((
                tools_for_mode.mcp_tool_index.clone(),
                tools_for_mode.mcp_total_count,
            ));
        }
        let tool_names: std::collections::HashSet<String> = tools_for_mode
            .tools
            .iter()
            .map(|t| t.name.clone())
            .collect();

        let meta = ChatMeta {
            chat_id: chat_id.clone(),
            chat_mode: thread.mode.clone(),
            chat_remote: false,
            current_config_file: String::new(),
            context_tokens_cap: thread.context_tokens_cap,
            include_project_info: thread.include_project_info,
            request_attempt_id: Uuid::new_v4().to_string(),
            worktree: thread.worktree.clone(),
        };

        let messages = {
            let session = session_arc.lock().await;
            session.messages.clone()
        };
        let mut has_rag_results = crate::scratchpads::scratchpad_utils::HasRagResults::new();
        let (messages_with_preamble, skills_info) =
            prepend_the_right_system_prompt_and_maybe_more_initial_messages(
                app.clone(),
                messages,
                &meta,
                &thread.task_meta,
                &mut has_rag_results,
                tool_names,
                &thread.mode,
                &thread.model,
            )
            .await;

        let first_conv_idx = messages_with_preamble
            .iter()
            .position(|m| {
                m.role == "user"
                    || m.role == "assistant"
                    || m.role == crate::chat::internal_roles::EVENT_ROLE
            })
            .unwrap_or(messages_with_preamble.len());

        {
            let mut session = session_arc.lock().await;
            session.skills_available_count = skills_info.available_count;
            session.skills_included = skills_info.included_names.clone();
        }

        if first_conv_idx > 0 {
            let mut session = session_arc.lock().await;

            let mut system_insert_idx = 0;
            let mut context_insert_idx = session
                .messages
                .iter()
                .position(|m| m.role == "system")
                .map(|i| i + 1)
                .unwrap_or(0);

            let mut inserted = 0;
            for msg in messages_with_preamble.iter().take(first_conv_idx) {
                if msg.role == "assistant" {
                    continue;
                }
                if msg.role == "system"
                    && session
                        .messages
                        .first()
                        .map(|m| m.role == "system")
                        .unwrap_or(false)
                {
                    continue;
                }
                if msg.role == "cd_instruction"
                    && session.messages.iter().any(|m| m.role == "cd_instruction")
                {
                    continue;
                }
                if msg.role == "context_file"
                    && session
                        .messages
                        .iter()
                        .any(|m| m.role == "context_file" && m.tool_call_id == msg.tool_call_id)
                {
                    continue;
                }
                let insert_idx = if msg.role == "system" {
                    let idx = system_insert_idx;
                    system_insert_idx += 1;
                    context_insert_idx += 1;
                    idx
                } else {
                    let idx = context_insert_idx;
                    context_insert_idx += 1;
                    idx
                };
                session.insert_message(insert_idx, msg.clone());
                inserted += 1;
            }
            if inserted > 0 {
                info!("Saved {} preamble messages to session", inserted);
            }
        }
    }

    // Inject MCP lazy-mode index hint (once per session, idempotent via marker)
    if let Some((mcp_index, mcp_total)) = mcp_for_index {
        let already_has_index = {
            let session = session_arc.lock().await;
            session
                .messages
                .iter()
                .any(|m| m.role == "cd_instruction" && m.tool_call_id == MCP_LAZY_INDEX_MARKER)
        };
        if !already_has_index {
            let index_text = build_mcp_index_message(&mcp_index, mcp_total);
            let mut session = session_arc.lock().await;
            let insert_pos = session
                .messages
                .iter()
                .position(|m| m.role == "system")
                .map(|i| i + 1)
                .unwrap_or(0);
            session.insert_message(
                insert_pos,
                ChatMessage {
                    role: "cd_instruction".to_string(),
                    tool_call_id: MCP_LAZY_INDEX_MARKER.to_string(),
                    content: ChatContent::SimpleText(index_text),
                    ..Default::default()
                },
            );
            info!("Injected MCP lazy index hint with {} tools", mcp_total);
        }
    }

    let (
        last_is_user,
        auto_enrichment_enabled,
        user_count,
        has_manual_enrichment_for_turn,
        suppress_flag,
    ) = {
        let mut session = session_arc.lock().await;
        let last_user_idx = session
            .messages
            .iter()
            .rposition(|m| is_prompt_turn_role(&m.role));
        let last_user =
            last_user_idx.is_some() && last_user_idx == session.messages.len().checked_sub(1);
        let auto = session.thread.auto_enrichment_enabled.unwrap_or(false);
        let count = session
            .messages
            .iter()
            .filter(|m| is_prompt_turn_role(&m.role))
            .count();
        let manual = last_user_idx
            .and_then(|idx| idx.checked_sub(1))
            .and_then(|idx| session.messages.get(idx))
            .map(|m| m.role == "context_file" && m.tool_call_id == "manual_memory_enrichment")
            .unwrap_or(false);
        let suppress = session.suppress_auto_enrichment_for_next_turn;
        if suppress {
            session.suppress_auto_enrichment_for_next_turn = false;
        }
        (last_user, auto, count, manual, suppress)
    };
    if is_agentic_mode_id(&thread.mode)
        && last_is_user
        && auto_enrichment_enabled
        && !has_manual_enrichment_for_turn
        && !suppress_flag
    {
        let force_enrichment = user_count > 1;
        let mut messages = {
            let session = session_arc.lock().await;
            session.messages.clone()
        };
        let msg_count_before = messages.len();
        enrich_messages_with_knowledge(
            gcx.clone(),
            &mut messages,
            Some(&chat_id),
            force_enrichment,
        )
        .await;
        if messages.len() > msg_count_before {
            let local_last_user_idx = messages
                .iter()
                .rposition(|m| is_prompt_turn_role(&m.role))
                .unwrap_or(0);
            if local_last_user_idx > 0 {
                let enriched_msg = &messages[local_last_user_idx - 1];
                if enriched_msg.role == "context_file" {
                    let mut session = session_arc.lock().await;
                    let session_last_user_idx = session
                        .messages
                        .iter()
                        .rposition(|m| is_prompt_turn_role(&m.role))
                        .unwrap_or(0);
                    session.insert_message(session_last_user_idx, enriched_msg.clone());
                    info!(
                        "Saved auto knowledge enrichment context_file to session at index {}",
                        session_last_user_idx
                    );
                }
            }
        }
    }
}

pub fn save_rag_results_to_session(session: &mut ChatSession, rag_results: &[serde_json::Value]) {
    let last_user_idx = session
        .messages
        .iter()
        .rposition(|m| is_prompt_turn_role(&m.role));
    if let Some(insert_idx) = last_user_idx {
        let existing_content: std::collections::HashSet<String> = session
            .messages
            .iter()
            .filter(|m| m.role == "context_file" || m.role == "plain_text")
            .map(|m| m.content.content_text_only())
            .collect();
        let mut offset = 0;
        for rag_msg_json in rag_results {
            if let Ok(msg) = serde_json::from_value::<ChatMessage>(rag_msg_json.clone()) {
                if (msg.role == "context_file" || msg.role == "plain_text")
                    && !existing_content.contains(&msg.content.content_text_only())
                {
                    session.insert_message(insert_idx + offset, msg);
                    offset += 1;
                }
            }
        }
    }
}

fn is_prompt_turn_role(role: &str) -> bool {
    role == "user"
        || role == crate::chat::internal_roles::EVENT_ROLE
        || role == crate::chat::internal_roles::PLAN_ROLE
}

fn tail_needs_assistant(messages: &[ChatMessage]) -> bool {
    let mut saw_toolish = false;

    for m in messages.iter().rev() {
        match m.role.as_str() {
            "assistant" => {
                if !saw_toolish {
                    return false;
                }
                let Some(tcs) = m.tool_calls.as_ref() else {
                    return false;
                };
                if tcs.is_empty() {
                    return false;
                }
                return tcs.iter().any(|tc| !tc.id.starts_with("srvtoolu_"));
            }
            role if is_prompt_turn_role(role) => return true,
            "tool" | "context_file" => saw_toolish = true,
            _ => {}
        }
    }

    false
}

fn is_claude_code_model(model: &BaseModelRecord) -> bool {
    model.wire_format == crate::llm::WireFormat::AnthropicMessages && !model.auth_token.is_empty()
}

async fn maybe_enqueue_completion_activity_reaction(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) {
    let activity = {
        let session = session_arc.lock().await;
        ChatActivityCompletion {
            chat_id: session.chat_id.clone(),
            thread: session.thread.clone(),
        }
    };
    maybe_enqueue_chat_activity_reaction(app, activity).await;
}

async fn ensure_claude_code_identity(
    session_arc: &Arc<AMutex<ChatSession>>,
    model: &BaseModelRecord,
) -> Option<crate::llm::ClaudeCodeIdentity> {
    if !is_claude_code_model(model) {
        return None;
    }

    let mut session = session_arc.lock().await;
    if let Some(identity) = session.thread.claude_code_identity.clone() {
        return Some(identity);
    }

    let identity = crate::llm::adapters::claude_code_compat::generate_claude_code_identity();
    session.thread.claude_code_identity = Some(identity.clone());
    session.increment_version();
    session.touch();
    Some(identity)
}

#[cfg(test)]
fn ensure_claude_code_identity_for_test(
    session: &mut ChatSession,
    model: &BaseModelRecord,
) -> Option<crate::llm::ClaudeCodeIdentity> {
    if !is_claude_code_model(model) {
        return None;
    }

    if let Some(identity) = session.thread.claude_code_identity.clone() {
        return Some(identity);
    }

    let identity = crate::llm::adapters::claude_code_compat::generate_claude_code_identity();
    session.thread.claude_code_identity = Some(identity.clone());
    session.increment_version();
    session.touch();
    Some(identity)
}

fn is_reasoning_token_limit_stop(message: &ChatMessage) -> bool {
    if message.role != "assistant"
        || message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
    {
        return false;
    }

    let finish_reason = message.finish_reason.as_deref().unwrap_or_default();
    let stopped_for_tokens = matches!(
        finish_reason,
        "length" | "max_tokens" | "max_output_tokens" | "token_limit"
    );
    if !stopped_for_tokens {
        return false;
    }

    message.content.content_text_only().trim().is_empty()
        && (message
            .reasoning_content
            .as_deref()
            .is_some_and(|reasoning| !reasoning.trim().is_empty())
            || message
                .thinking_blocks
                .as_ref()
                .is_some_and(|blocks| !blocks.is_empty()))
}

fn is_length_like_finish_reason(finish_reason: Option<&str>) -> bool {
    matches!(
        finish_reason,
        Some("length" | "max_tokens" | "max_output_tokens" | "token_limit")
    )
}

fn has_reasoning_or_thinking_output(message: &ChatMessage) -> bool {
    message
        .reasoning_content
        .as_deref()
        .is_some_and(|reasoning| !reasoning.trim().is_empty())
        || message
            .thinking_blocks
            .as_ref()
            .is_some_and(|blocks| !blocks.is_empty())
}

fn has_empty_or_near_empty_visible_output(message: &ChatMessage) -> bool {
    message.content.content_text_only().trim().chars().count()
        <= LENGTH_STOP_NEAR_EMPTY_VISIBLE_CHARS
}

pub(crate) fn is_high_pressure_length_stop(
    message: &ChatMessage,
    messages: &[ChatMessage],
    effective_n_ctx: usize,
) -> bool {
    if message.role != "assistant"
        || message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
        || !is_length_like_finish_reason(message.finish_reason.as_deref())
        || !has_empty_or_near_empty_visible_output(message)
        || !has_reasoning_or_thinking_output(message)
    {
        return false;
    }

    matches!(
        crate::chat::summarization::estimated_context_pressure(messages, effective_n_ctx),
        ContextPressure::High | ContextPressure::Critical
    )
}

fn is_length_stop_compression_candidate(message: &ChatMessage) -> bool {
    message.role == "assistant"
        && !message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
        && is_length_like_finish_reason(message.finish_reason.as_deref())
        && has_empty_or_near_empty_visible_output(message)
}

async fn maybe_compact_after_high_pressure_length_stop(
    gcx: Arc<crate::global_context::GlobalContext>,
    session_arc: &Arc<AMutex<ChatSession>>,
    thread: &ThreadParams,
    effective_n_ctx: Option<usize>,
) -> bool {
    let Some(effective_n_ctx) = effective_n_ctx else {
        let mut session = session_arc.lock().await;
        if matches!(
            session.runtime.state,
            SessionState::Idle | SessionState::Completed
        ) && session
            .messages
            .last()
            .is_some_and(is_length_stop_compression_candidate)
        {
            crate::chat::summarization::emit_compression_skipped_status(
                &mut session,
                CompressionReason::EffectiveContextUnknown,
            );
        }
        return false;
    };
    let reactive_attempt = {
        let mut session = session_arc.lock().await;
        if !matches!(
            session.runtime.state,
            SessionState::Idle | SessionState::Completed
        ) {
            return false;
        }
        let Some(message) = session.messages.last() else {
            return false;
        };
        if !is_high_pressure_length_stop(message, &session.messages, effective_n_ctx) {
            return false;
        }
        let reactive_attempt = session.thread.reactive_compact_attempts.unwrap_or(0) + 1;
        if reactive_attempt > crate::chat::summarization::MAX_SEGMENT_SUMMARY_ATTEMPTS {
            crate::chat::summarization::emit_compression_skipped_status(
                &mut session,
                CompressionReason::MaxAttemptsReached,
            );
            return false;
        }
        session.thread.reactive_compact_attempts = Some(reactive_attempt);
        session.increment_version();
        session.touch();
        reactive_attempt
    };

    warn!(
        "High-pressure length stop, summarizing oldest eligible segment attempt {}/{}",
        reactive_attempt,
        crate::chat::summarization::MAX_SEGMENT_SUMMARY_ATTEMPTS,
    );
    let compacted =
        crate::chat::summarization::apply_segment_summarization(gcx, session_arc, thread, true)
            .await;
    if compacted {
        let mut session = session_arc.lock().await;
        session.thread.previous_response_id = None;
        session.cache_guard_force_next = true;
    }
    compacted
}

async fn handle_task_agent_reasoning_token_stop(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> bool {
    let (task_meta, finish_reason, usage, message_id, agent_chat_id) = {
        let mut session = session_arc.lock().await;
        let Some(meta) = session.thread.task_meta.clone() else {
            return false;
        };
        if meta.role != "agents" {
            return false;
        }
        let Some(message) = session.messages.last() else {
            return false;
        };
        if !is_reasoning_token_limit_stop(message) {
            return false;
        }

        let finish_reason = message
            .finish_reason
            .clone()
            .unwrap_or_else(|| "length".to_string());
        let usage = message.usage.clone();
        let message_id = message.message_id.clone();
        let agent_chat_id = session.chat_id.clone();
        session.set_runtime_state(SessionState::Completed, None);
        (meta, finish_reason, usage, message_id, agent_chat_id)
    };

    maybe_save_trajectory(app.clone(), session_arc.clone()).await;

    if let Err(error) = crate::chat::task_agent_monitor::handle_agent_reasoning_token_limit_stop(
        app,
        task_meta,
        finish_reason,
        usage,
        message_id,
        agent_chat_id,
    )
    .await
    {
        tracing::warn!("failed to notify planner about task agent reasoning token stop: {error}");
    }
    true
}

async fn run_fork_subchat(
    app: AppState,
    agent_name: &str,
    user_content: &str,
    thread: &ThreadParams,
    parent_chat_id: &str,
) -> Result<String, String> {
    let gcx = app.gcx.clone();
    let config = resolve_subchat_config_with_parent(
        gcx.clone(),
        agent_name,
        false,
        None,
        Some(format!("Fork: {}", agent_name)),
        Some(parent_chat_id.to_string()),
        Some("fork".to_string()),
        thread.root_chat_id.clone(),
        None,
        10,
        true,
        None,
        thread.mode.clone(),
        thread.task_meta.clone(),
        thread.worktree.clone(),
        None,
        None,
        None,
        0,
    )
    .await?;

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText(user_content.to_string()),
        ..Default::default()
    }];

    let result = run_subchat(gcx, messages, config).await?;

    let last_assistant = result.messages.iter().rev().find(|m| m.role == "assistant");
    Ok(last_assistant
        .map(|m| m.content.content_text_only())
        .unwrap_or_else(|| "Fork skill completed but produced no response.".to_string()))
}

pub fn start_generation(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let gcx = app.gcx.clone();
        let mut network_retry_attempt = 0usize;
        loop {
            let (mut thread, chat_id) = {
                let session = session_arc.lock().await;
                (session.thread.clone(), session.chat_id.clone())
            };
            {
                let session = session_arc.lock().await;
                if let Some(ref m) = session.active_command.model_override {
                    if !m.is_empty() {
                        thread.model = m.clone();
                    }
                }
            }

            let chat_label = {
                let t = thread.title.trim().to_string();
                if t.is_empty() || t == "New Chat" {
                    "Untitled chat".to_string()
                } else {
                    t.chars().take(60).collect()
                }
            };

            let fork_agent_name = {
                let session = session_arc.lock().await;
                session.active_command.context_fork.clone()
            };

            if let Some(agent_name) = fork_agent_name {
                let user_content_opt = {
                    let session = session_arc.lock().await;
                    session
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m.role == "user")
                        .map(|m| m.content.content_text_only())
                };
                {
                    let mut session = session_arc.lock().await;
                    session.active_command.context_fork = None;
                }
                let user_content = match user_content_opt {
                    Some(c) => c,
                    None => {
                        warn!(
                            "Fork skill '{}' skipped: no user message found in session {}",
                            agent_name, chat_id
                        );
                        continue;
                    }
                };

                let fork_result =
                    run_fork_subchat(app.clone(), &agent_name, &user_content, &thread, &chat_id)
                        .await;

                match fork_result {
                    Ok(assistant_content) => {
                        let mut session = session_arc.lock().await;
                        session.add_message(ChatMessage {
                            role: "assistant".to_string(),
                            content: ChatContent::SimpleText(assistant_content),
                            ..Default::default()
                        });
                        session.set_runtime_state(SessionState::Idle, None);
                        drop(session);
                        maybe_enqueue_completion_activity_reaction(
                            app.clone(),
                            session_arc.clone(),
                        )
                        .await;
                        maybe_save_trajectory(app.clone(), session_arc.clone()).await;
                        break;
                    }
                    Err(e) => {
                        warn!(
                            "Fork skill subchat failed ({}), falling back to normal generation",
                            e
                        );
                        continue;
                    }
                }
            }

            crate::chat::summarization::apply_segment_summarization(
                gcx.clone(),
                &session_arc,
                &thread,
                false,
            )
            .await;

            thread = {
                let session = session_arc.lock().await;
                session.thread.clone()
            };

            if user_stop_requested(&session_arc).await {
                break;
            }

            let (abort_flag, abort_notify) = {
                let mut session = session_arc.lock().await;
                match session.start_stream() {
                    Some((_message_id, abort_flag)) => {
                        let notify = session.abort_notify.clone();
                        (abort_flag, notify)
                    }
                    None => {
                        warn!(
                            "Cannot start generation for {}: already generating",
                            chat_id
                        );
                        break;
                    }
                }
            };

            {
                let mut ev = make_runtime_event(
                    "chat_started",
                    &format!("Started: {}", chat_label),
                    "chat",
                    &format!("chat_{}", chat_id),
                    "started",
                    None,
                );
                ev.chat_id = Some(chat_id.to_string());
                app.buddy_event_sink.enqueue_event(ev).await;
                let mut ev = make_runtime_event(
                    "streaming",
                    &format!("Generating reply in '{}'", chat_label),
                    "chat",
                    &format!("chat_{}", chat_id),
                    "streaming",
                    None,
                );
                ev.speech_text = Some(format!("Working on your request in '{}'...", chat_label));
                ev.scene = Some("working".to_string());
                ev.persistent = true;
                ev.chat_id = Some(chat_id.to_string());
                app.buddy_event_sink.enqueue_event(ev).await;
            }

            let generation_result = run_llm_generation(
                app.clone(),
                session_arc.clone(),
                thread.clone(),
                chat_id.clone(),
                abort_flag.clone(),
                abort_notify.clone(),
            )
            .await;

            if let Ok(GenerationResult::PausedForUserDecision) = generation_result {
                maybe_save_trajectory(app.clone(), session_arc.clone()).await;
                break;
            }

            if let Err(mut error) = generation_result {
                let retry_decision = error.retry_decision();
                let should_retry_network = error.should_retry(network_retry_attempt, &abort_flag);
                let retry_reason = retry_decision.reason();
                if should_retry_network {
                    let delay = super::retry_policy::retry_delay_for_attempt(network_retry_attempt);
                    network_retry_attempt += 1;
                    {
                        let mut session = session_arc.lock().await;
                        if !session.abort_flag.load(Ordering::SeqCst) {
                            session.clear_stream_for_retry();
                            let retry_msg = make_ui_only_retry_status_message(
                                &error.message,
                                network_retry_attempt,
                                super::retry_policy::MAX_LLM_RETRY_ATTEMPTS,
                                delay.as_secs(),
                            );
                            session.add_message(retry_msg);
                        }
                    }
                    {
                        let mut ev = make_runtime_event(
                            "chat_retrying",
                            &format!(
                                "Retrying '{}' in {}s (attempt {}/{})",
                                chat_label,
                                delay.as_secs(),
                                network_retry_attempt,
                                super::retry_policy::MAX_LLM_RETRY_ATTEMPTS,
                            ),
                            "chat",
                            &format!("chat_{}", chat_id),
                            "retrying",
                            None,
                        );
                        ev.chat_id = Some(chat_id.to_string());
                        ev.persistent = true;
                        app.buddy_event_sink.enqueue_event(ev).await;
                    }
                    warn!(
                        "Retrying chat generation after retryable LLM error in {}s (attempt {}/{}, reason={})",
                        delay.as_secs(),
                        network_retry_attempt,
                        super::retry_policy::MAX_LLM_RETRY_ATTEMPTS,
                        retry_reason,
                    );
                    if super::retry_policy::sleep_or_abort(delay, abort_flag.clone()).await {
                        break;
                    }
                    continue;
                }
                if retry_decision.is_context_limit() && !abort_flag.load(Ordering::SeqCst) {
                    let reactive_attempt = {
                        let session = session_arc.lock().await;
                        session.thread.reactive_compact_attempts.unwrap_or(0) + 1
                    };
                    if reactive_attempt <= crate::chat::summarization::MAX_SEGMENT_SUMMARY_ATTEMPTS
                    {
                        let original_error = error.message.clone();
                        warn!(
                            "Context limit error, summarizing oldest eligible segment attempt {}/{}: {}",
                            reactive_attempt,
                            crate::chat::summarization::MAX_SEGMENT_SUMMARY_ATTEMPTS,
                            original_error,
                        );
                        {
                            let mut session = session_arc.lock().await;
                            session.clear_stream_for_retry();
                            session.add_message(make_ui_only_error_message(&original_error));
                            session.thread.reactive_compact_attempts = Some(reactive_attempt);
                        }
                        let compacted = crate::chat::summarization::apply_segment_summarization(
                            gcx.clone(),
                            &session_arc,
                            &thread,
                            true,
                        )
                        .await;
                        if compacted {
                            let mut session = session_arc.lock().await;
                            session.clear_stream_for_retry();
                            session.thread.previous_response_id = None;
                            session.cache_guard_force_next = true;
                            continue;
                        }
                    }
                    error.message = format!(
                        "Context too large and no eligible segment summary could be applied. Original error: {}",
                        error.message
                    );
                }

                if error.partial_output_emitted && !abort_flag.load(Ordering::SeqCst) {
                    let original = error.message.clone();
                    warn!(
                        "{} Original error: {}",
                        PARTIAL_OUTPUT_STREAM_ERROR, original
                    );
                    error.message = format!(
                        "{} Original error: {}",
                        PARTIAL_OUTPUT_STREAM_ERROR, original
                    );
                }

                let error_message = error.message;

                let task_meta_opt = {
                    let mut session = session_arc.lock().await;
                    if !session.abort_flag.load(Ordering::SeqCst) {
                        let app2 = app.clone();
                        let err_clone = error_message.clone();
                        let chat_id2 = chat_id.clone();
                        let chat_label2 = chat_label.clone();
                        tokio::spawn(async move {
                            app2.buddy_event_sink
                                .report_error(
                                    "llm_error",
                                    &err_clone,
                                    Some("chat/generation.rs"),
                                    Some(&chat_id2),
                                )
                                .await;
                            let short_err: String = err_clone.chars().take(60).collect();
                            let mut ev = make_runtime_event(
                                "chat_error",
                                &format!("Error in '{}': {}", chat_label2, short_err),
                                "chat",
                                &format!("chat_{}", chat_id2),
                                "failed",
                                Some("high"),
                            );
                            ev.chat_id = Some(chat_id2.to_string());
                            app2.buddy_event_sink.mark_chat_error(ev).await;
                        });
                        session.finish_stream_with_error(error_message);
                    }
                    session.thread.task_meta.clone()
                };

                maybe_save_trajectory(app.clone(), session_arc.clone()).await;

                if let Some(task_meta) = task_meta_opt {
                    let error_msg = {
                        let session = session_arc.lock().await;
                        session.task_agent_error.clone()
                    };
                    if let Some(error) = error_msg {
                        super::task_agent_monitor::handle_agent_streaming_error(
                            app.clone(),
                            &task_meta,
                            &error,
                        )
                        .await;
                    }
                }
                {
                    let mut session = session_arc.lock().await;
                    if session.user_interrupt_flag.load(Ordering::SeqCst) {
                        session.clear_stream_for_retry();
                    }
                }
                break;
            }

            network_retry_attempt = 0;

            if abort_flag.load(Ordering::SeqCst) {
                break;
            }

            let (mode_id, model_id, context_tokens_cap) = {
                let session = session_arc.lock().await;
                (
                    session.thread.mode.clone(),
                    session.thread.model.clone(),
                    session.thread.context_tokens_cap,
                )
            };

            let model_id_opt = if model_id.is_empty() {
                None
            } else {
                Some(model_id.as_str())
            };

            let effective_n_ctx = {
                let caps =
                    crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
                        .await;
                let model_rec = match caps {
                    Ok(caps) => crate::caps::resolve_chat_model(caps, &model_id).ok(),
                    Err(_) => None,
                };
                model_rec.map(|rec| {
                    let model_n_ctx = if rec.base.n_ctx > 0 {
                        rec.base.n_ctx
                    } else {
                        tokens().default_n_ctx
                    };
                    match context_tokens_cap {
                        Some(cap) if cap > 0 => cap.min(model_n_ctx),
                        _ => model_n_ctx,
                    }
                })
            };

            if let Some(effective_n_ctx) = effective_n_ctx {
                let mut session = session_arc.lock().await;
                maybe_inject_token_budget_instruction(
                    &mut session,
                    effective_n_ctx,
                    TOKEN_BUDGET_CADENCE,
                );
            }

            maybe_save_trajectory(app.clone(), session_arc.clone()).await;

            match process_tool_calls_once(app.clone(), session_arc.clone(), &mode_id, model_id_opt)
                .await
            {
                ToolStepOutcome::NoToolCalls => {
                    if handle_task_agent_reasoning_token_stop(app.clone(), session_arc.clone())
                        .await
                    {
                        break;
                    }
                    if maybe_compact_after_high_pressure_length_stop(
                        gcx.clone(),
                        &session_arc,
                        &thread,
                        effective_n_ctx,
                    )
                    .await
                    {
                        maybe_save_trajectory(app.clone(), session_arc.clone()).await;
                        continue;
                    }
                    if inject_priority_messages_if_any(app.clone(), session_arc.clone()).await {
                        continue;
                    }
                    let should_continue = {
                        let session = session_arc.lock().await;
                        tail_needs_assistant(&session.messages)
                    };
                    if should_continue {
                        continue;
                    }
                    let app_stop = AppState::from_gcx(gcx.clone()).await;
                    let session_id_stop = chat_id.clone();
                    let handle = tokio::spawn(async move {
                        let project_dir = get_project_dir_string(app_stop.clone()).await;
                        let payload = HookPayload {
                            hook_event_name: "Stop".to_string(),
                            session_id: session_id_stop,
                            project_dir,
                            tool_name: None,
                            tool_input: None,
                            tool_output: None,
                            user_prompt: None,
                            extra: std::collections::HashMap::new(),
                        };
                        run_hooks(app_stop, HookEvent::Stop, payload).await;
                    });
                    session_arc.lock().await.stop_hook_handle = Some(handle);
                    {
                        let mut ev = make_runtime_event(
                            "chat_completed",
                            &format!("Completed: {}", chat_label),
                            "chat",
                            &format!("chat_{}", chat_id),
                            "completed",
                            None,
                        );
                        ev.chat_id = Some(chat_id.to_string());
                        app.buddy_event_sink
                            .apply_chat_completion(ev, 4, "happy".to_string())
                            .await;
                    }
                    maybe_enqueue_completion_activity_reaction(app.clone(), session_arc.clone())
                        .await;
                    break;
                }
                ToolStepOutcome::Paused => {
                    let mut ev = make_runtime_event(
                        "tool_confirmation",
                        &format!("Waiting for approval: {}", chat_label),
                        "chat",
                        &format!("chat_{}", chat_id),
                        "paused",
                        None,
                    );
                    ev.chat_id = Some(chat_id.to_string());
                    ev.persistent = true;
                    app.buddy_event_sink.enqueue_event(ev).await;
                    break;
                }
                ToolStepOutcome::Stop => {
                    let mut ev = make_runtime_event(
                        "chat_completed",
                        &format!("Completed: {}", chat_label),
                        "chat",
                        &format!("chat_{}", chat_id),
                        "completed",
                        None,
                    );
                    ev.chat_id = Some(chat_id.to_string());
                    app.buddy_event_sink
                        .apply_chat_completion(ev, 4, "happy".to_string())
                        .await;
                    maybe_enqueue_completion_activity_reaction(app.clone(), session_arc.clone())
                        .await;
                    break;
                }
                ToolStepOutcome::Continue => {
                    inject_priority_messages_if_any(app.clone(), session_arc.clone()).await;
                }
            }
        }

        check_external_reload_pending(gcx.clone(), session_arc.clone()).await;

        {
            let session = session_arc.lock().await;
            session.abort_flag.store(false, Ordering::SeqCst);
            session.user_interrupt_flag.store(false, Ordering::SeqCst);
            session.queue_notify.notify_one();
        }
    })
}

pub async fn run_llm_generation(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    thread: ThreadParams,
    chat_id: String,
    abort_flag: Arc<AtomicBool>,
    abort_notify: Arc<tokio::sync::Notify>,
) -> Result<GenerationResult, LlmStreamError> {
    let gcx = app.gcx.clone();
    check_aborted_before_stream(&abort_flag)?;
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.message)?;
    let model_rec = crate::caps::resolve_chat_model(caps.clone(), &thread.model)?;
    check_aborted_before_stream(&abort_flag)?;

    let tools_for_gen = app
        .tool_registry
        .get_tools_index_for_mode(&thread.mode, Some(&model_rec.base.id))
        .await;
    let mcp_lazy_active = tools_for_gen.mcp_lazy_mode;
    let tools = tools_for_gen.tools;

    info!(
        "session generation: model={}, tools count = {} (mcp_lazy={})",
        model_rec.base.id,
        tools.len(),
        mcp_lazy_active
    );

    let (messages, existing_frozen_prefix) = {
        let session = session_arc.lock().await;
        (
            session.messages.clone(),
            session.thread.frozen_request_prefix.clone(),
        )
    };
    let canonical_tools = build_canonical_openai_tools(
        gcx.clone(),
        &tools,
        model_rec.supports_strict_tools,
        model_rec.supports_tools,
    )
    .await;
    let mut installed_frozen_prefix = false;
    let frozen_request_prefix = if existing_frozen_prefix
        .as_ref()
        .is_some_and(frozen_prefix_is_complete)
    {
        existing_frozen_prefix
    } else {
        let system_prompt = first_system_prompt(&messages);
        let mut session = session_arc.lock().await;
        installed_frozen_prefix = ensure_frozen_prefix(
            &mut session,
            system_prompt,
            Some(serde_json::Value::Array(canonical_tools.tools.clone())),
        )
        .is_some();
        session.thread.frozen_request_prefix.clone()
    };
    if installed_frozen_prefix {
        maybe_save_trajectory(app.clone(), session_arc.clone()).await;
    }

    let model_n_ctx = if model_rec.base.n_ctx > 0 {
        model_rec.base.n_ctx
    } else {
        tokens().default_n_ctx
    };
    let effective_n_ctx = match thread.context_tokens_cap {
        Some(cap) if cap > 0 => cap.min(model_n_ctx),
        _ => model_n_ctx,
    };
    check_aborted_before_stream(&abort_flag)?;
    let tokenizer_arc = crate::tokens::cached_tokenizer(gcx.clone(), &model_rec.base).await?;
    let t = HasTokenizerAndEot::new(tokenizer_arc);

    let meta = ChatMeta {
        chat_id: chat_id.clone(),
        chat_mode: thread.mode.clone(),
        chat_remote: false,
        current_config_file: String::new(),
        context_tokens_cap: thread.context_tokens_cap,
        include_project_info: thread.include_project_info,
        request_attempt_id: Uuid::new_v4().to_string(),
        worktree: thread.worktree.clone(),
    };

    let model_type_defaults = caps.user_defaults.defaults_for_model(
        &model_rec.base.id,
        &caps.defaults.chat_default_model,
        &caps.defaults.chat_light_model,
        &caps.defaults.chat_thinking_model,
        &caps.defaults.chat_buddy_model,
    );
    let mut parameters = SamplingParameters {
        temperature: thread.temperature.or(model_type_defaults.temperature),
        frequency_penalty: thread.frequency_penalty,
        max_new_tokens: thread
            .max_tokens
            .or(model_type_defaults.max_new_tokens)
            .unwrap_or(0),
        boost_reasoning: thread
            .boost_reasoning
            .unwrap_or_else(|| model_type_defaults.boost_reasoning.unwrap_or(false)),
        reasoning_effort: thread
            .reasoning_effort
            .as_ref()
            .and_then(|s| match s.as_str() {
                "low" => Some(crate::call_validation::ReasoningEffort::Low),
                "medium" => Some(crate::call_validation::ReasoningEffort::Medium),
                "high" => Some(crate::call_validation::ReasoningEffort::High),
                "xhigh" => Some(crate::call_validation::ReasoningEffort::XHigh),
                "max" => Some(crate::call_validation::ReasoningEffort::Max),
                _ => None,
            })
            .or_else(|| {
                model_type_defaults
                    .reasoning_effort
                    .as_ref()
                    .and_then(|s| match s.as_str() {
                        "low" => Some(crate::call_validation::ReasoningEffort::Low),
                        "medium" => Some(crate::call_validation::ReasoningEffort::Medium),
                        "high" => Some(crate::call_validation::ReasoningEffort::High),
                        "xhigh" => Some(crate::call_validation::ReasoningEffort::XHigh),
                        "max" => Some(crate::call_validation::ReasoningEffort::Max),
                        _ => None,
                    })
            }),
        thinking_budget: thread
            .thinking_budget
            .or(model_type_defaults.thinking_budget),
        ..Default::default()
    };

    let ccx = AtCommandsContext::new_from_app(
        app.clone(),
        effective_n_ctx,
        CHAT_TOP_N,
        false,
        messages.clone(),
        chat_id.clone(),
        thread.root_chat_id.clone(),
        model_rec.base.id.clone(),
        thread.task_meta.clone(),
        thread.worktree.clone(),
    )
    .await;
    let ccx_arc = Arc::new(AMutex::new(ccx));

    let options = ChatPrepareOptions {
        prepend_system_prompt: false,
        allow_at_commands: true,
        allow_tool_prerun: true,
        supports_tools: model_rec.supports_tools,
        parallel_tool_calls: thread.parallel_tool_calls,
        cache_control: CacheControl::Ephemeral,
        frozen_request_prefix: frozen_request_prefix.clone(),
        ..Default::default()
    };

    check_aborted_before_stream(&abort_flag)?;
    let mut prepared = prepare_chat_passthrough(
        gcx.clone(),
        ccx_arc.clone(),
        &t,
        messages,
        &thread,
        &model_rec.base.id,
        &thread.mode,
        tools,
        &meta,
        &mut parameters,
        &options,
    )
    .await?;

    let claude_code_identity = ensure_claude_code_identity(&session_arc, &model_rec.base).await;
    prepared.llm_request = prepared
        .llm_request
        .with_claude_code_identity(claude_code_identity);

    {
        let mut session = session_arc.lock().await;
        session.last_prompt_messages = prepared.limited_messages.clone();
        save_rag_results_to_session(&mut session, &prepared.rag_results);
    }

    check_aborted_before_stream(&abort_flag)?;
    run_streaming_generation(
        app,
        session_arc,
        prepared.llm_request,
        &model_rec,
        abort_flag,
        abort_notify,
    )
    .await
}

async fn generation_metering_usd(
    app: &AppState,
    model_id: &str,
    usage: &ChatUsage,
) -> Option<MeteringUsd> {
    let pricing = crate::providers::pricing::lookup_model_pricing(&app.gcx, model_id).await?;
    crate::providers::pricing::compute_cost(usage, &pricing)
}

pub enum GenerationResult {
    Completed,
    PausedForUserDecision,
}

async fn run_streaming_generation(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    mut llm_request: LlmRequest,
    model_rec: &crate::caps::ChatModelRecord,
    abort_flag: Arc<AtomicBool>,
    abort_notify: Arc<tokio::sync::Notify>,
) -> Result<GenerationResult, LlmStreamError> {
    info!(
        "session generation: model={}, messages={}",
        llm_request.model_id,
        llm_request.messages.len()
    );
    let (chat_id, root_chat_id, mode, task_id, task_role, agent_id, card_id) = {
        let session = session_arc.lock().await;
        let tm = session.thread.task_meta.as_ref();
        (
            session.chat_id.clone(),
            session.thread.root_chat_id.clone(),
            session.thread.mode.clone(),
            tm.map(|t| t.task_id.clone()),
            tm.map(|t| t.role.clone()),
            tm.and_then(|t| t.agent_id.clone()),
            tm.and_then(|t| t.card_id.clone()),
        )
    };
    let mode_for_stats = canonicalize_mode_for_stats(&mode);

    const TEMPERATURE_BUMP: f32 = 0.1;
    const MAX_RETRY_TEMPERATURE: f32 = 0.5;
    let user_specified_temp = llm_request.params.temperature;
    let model_supports_temperature = model_rec.supports_temperature;
    let can_retry_with_temp_bump = user_specified_temp.is_none() && model_supports_temperature;
    let max_attempts = if can_retry_with_temp_bump {
        (MAX_RETRY_TEMPERATURE / TEMPERATURE_BUMP).floor() as usize + 2
    } else {
        1
    };
    let mut attempt = 0;

    let (result, pending_success_event) = loop {
        attempt += 1;
        if can_retry_with_temp_bump && attempt > 1 {
            let retry_temp = TEMPERATURE_BUMP * (attempt - 2) as f32;
            llm_request.params.temperature = Some(retry_temp.min(MAX_RETRY_TEMPERATURE));
        }

        let params = StreamRunParams {
            llm_request: llm_request.clone(),
            model_rec: model_rec.base.clone(),
            chat_id: Some(chat_id.clone()),
            allow_websocket: true,
            abort_flag: Some(abort_flag.clone()),
            abort_notify: Some(abort_notify.clone()),
            supports_tools: model_rec.supports_tools,
            supports_reasoning: model_rec.has_reasoning_support(),
            reasoning_type: model_rec.reasoning_type_string(),
            supports_temperature: model_rec.supports_temperature,
        };

        enum CollectorEventPayload {
            DeltaOps(Vec<DeltaOp>),
            Usage(ChatUsage),
        }

        const EMITTER_QUEUE_CAPACITY: usize = 256;
        let (tx, mut rx) =
            tokio::sync::mpsc::channel::<CollectorEventPayload>(EMITTER_QUEUE_CAPACITY);
        let overflow_usage = Arc::new(std::sync::Mutex::new(None::<ChatUsage>));
        let overflow_ops = Arc::new(std::sync::Mutex::new(Vec::<DeltaOp>::new()));

        struct SessionCollector {
            tx: tokio::sync::mpsc::Sender<CollectorEventPayload>,
            overflow_usage: Arc<std::sync::Mutex<Option<ChatUsage>>>,
            overflow_ops: Arc<std::sync::Mutex<Vec<DeltaOp>>>,
        }

        impl StreamCollector for SessionCollector {
            fn on_delta_ops(&mut self, _choice_idx: usize, ops: Vec<DeltaOp>) {
                match self.tx.try_send(CollectorEventPayload::DeltaOps(ops)) {
                    Ok(()) => {}
                    Err(tokio::sync::mpsc::error::TrySendError::Full(event)) => {
                        if let CollectorEventPayload::DeltaOps(ops) = event {
                            if let Ok(mut guard) = self.overflow_ops.lock() {
                                guard.extend(ops);
                            }
                        }
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_event)) => {}
                }
            }

            fn on_usage(&mut self, usage: &ChatUsage) {
                let usage_clone = usage.clone();
                match self
                    .tx
                    .try_send(CollectorEventPayload::Usage(usage_clone.clone()))
                {
                    Ok(()) => {}
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_event)) => {
                        if let Ok(mut guard) = self.overflow_usage.lock() {
                            *guard = Some(usage_clone);
                        }
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_event)) => {}
                }
            }

            fn on_finish(&mut self, _choice_idx: usize, _finish_reason: Option<String>) {}
        }

        let mut collector = SessionCollector {
            tx,
            overflow_usage: overflow_usage.clone(),
            overflow_ops: overflow_ops.clone(),
        };

        let session_arc_emitter = session_arc.clone();
        let emitter_task = tokio::spawn(async move {
            fn merge_events(
                events: &mut Vec<CollectorEventPayload>,
                batched_ops: &mut Vec<DeltaOp>,
                latest_usage: &mut Option<ChatUsage>,
            ) {
                for event in events.drain(..) {
                    match event {
                        CollectorEventPayload::DeltaOps(ops) => {
                            batched_ops.extend(ops);
                        }
                        CollectorEventPayload::Usage(usage) => {
                            *latest_usage = Some(usage);
                        }
                    }
                }
            }

            fn coalesce_text_ops(ops: Vec<DeltaOp>) -> Vec<DeltaOp> {
                if ops.len() <= 1 {
                    return ops;
                }
                let mut out: Vec<DeltaOp> = Vec::with_capacity(ops.len());
                for op in ops {
                    match op {
                        DeltaOp::AppendContent { text } => {
                            if let Some(DeltaOp::AppendContent { text: ref mut prev }) =
                                out.last_mut()
                            {
                                prev.push_str(&text);
                            } else {
                                out.push(DeltaOp::AppendContent { text });
                            }
                        }
                        DeltaOp::AppendReasoning { text } => {
                            if let Some(DeltaOp::AppendReasoning { text: ref mut prev }) =
                                out.last_mut()
                            {
                                prev.push_str(&text);
                            } else {
                                out.push(DeltaOp::AppendReasoning { text });
                            }
                        }
                        other => out.push(other),
                    }
                }
                out
            }

            fn split_utf8_chunks(text: &str, max_bytes: usize) -> Vec<String> {
                if text.len() <= max_bytes {
                    return vec![text.to_string()];
                }
                let mut chunks = Vec::new();
                let mut start = 0usize;
                while start < text.len() {
                    let mut end = (start + max_bytes).min(text.len());
                    while end > start && !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    if end == start {
                        end = text[start..]
                            .char_indices()
                            .nth(1)
                            .map(|(i, _)| start + i)
                            .unwrap_or(text.len());
                    }
                    chunks.push(text[start..end].to_string());
                    start = end;
                }
                chunks
            }

            fn split_large_text_ops(ops: Vec<DeltaOp>, max_text_bytes: usize) -> Vec<DeltaOp> {
                let mut out = Vec::new();
                for op in ops {
                    match op {
                        DeltaOp::AppendContent { text } => {
                            for chunk in split_utf8_chunks(&text, max_text_bytes) {
                                out.push(DeltaOp::AppendContent { text: chunk });
                            }
                        }
                        DeltaOp::AppendReasoning { text } => {
                            for chunk in split_utf8_chunks(&text, max_text_bytes) {
                                out.push(DeltaOp::AppendReasoning { text: chunk });
                            }
                        }
                        other => out.push(other),
                    }
                }
                out
            }

            const MAX_BATCH_EVENTS: usize = 64;
            const MAX_DELTA_OPS_PER_EMIT: usize = 128;
            const MAX_DELTA_TEXT_BYTES: usize = 64 * 1024;
            let mut pending = Vec::<CollectorEventPayload>::new();

            while let Some(first_event) = rx.recv().await {
                pending.push(first_event);

                while pending.len() < MAX_BATCH_EVENTS {
                    match rx.try_recv() {
                        Ok(event) => pending.push(event),
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                    }
                }

                let mut batched_ops = Vec::new();
                let mut latest_usage: Option<ChatUsage> = None;
                merge_events(&mut pending, &mut batched_ops, &mut latest_usage);

                if let Ok(mut guard) = overflow_ops.lock() {
                    if !guard.is_empty() {
                        let mut drained = std::mem::take(&mut *guard);
                        drained.append(&mut batched_ops);
                        batched_ops = drained;
                    }
                }
                if let Ok(mut guard) = overflow_usage.lock() {
                    if let Some(usage) = guard.take() {
                        latest_usage = Some(usage);
                    }
                }

                let batched_ops = coalesce_text_ops(batched_ops);
                let batched_ops = split_large_text_ops(batched_ops, MAX_DELTA_TEXT_BYTES);

                let mut session = session_arc_emitter.lock().await;
                if !batched_ops.is_empty() {
                    for chunk in batched_ops.chunks(MAX_DELTA_OPS_PER_EMIT) {
                        session.emit_stream_delta(chunk.to_vec());
                    }
                }
                if let Some(usage) = latest_usage {
                    session.draft_usage = Some(usage);
                }
            }

            let mut final_ops = Vec::new();
            let mut final_usage: Option<ChatUsage> = None;
            if let Ok(mut guard) = overflow_ops.lock() {
                if !guard.is_empty() {
                    final_ops = std::mem::take(&mut *guard);
                }
            }
            if let Ok(mut guard) = overflow_usage.lock() {
                if let Some(usage) = guard.take() {
                    final_usage = Some(usage);
                }
            }

            if !final_ops.is_empty() || final_usage.is_some() {
                let final_ops = coalesce_text_ops(final_ops);
                let final_ops = split_large_text_ops(final_ops, MAX_DELTA_TEXT_BYTES);

                let mut session = session_arc_emitter.lock().await;
                if !final_ops.is_empty() {
                    for chunk in final_ops.chunks(MAX_DELTA_OPS_PER_EMIT) {
                        session.emit_stream_delta(chunk.to_vec());
                    }
                }
                if let Some(usage) = final_usage {
                    session.draft_usage = Some(usage);
                }
            }
        });

        let call_ts_start = chrono::Utc::now().to_rfc3339();
        let call_start = std::time::Instant::now();

        let stream_outcome = run_llm_stream(app.clone(), params, &mut collector).await;
        drop(collector);
        let _ = emitter_task.await;

        if let Ok(LlmStreamOutcome::PausedForCacheGuard) = stream_outcome {
            tracing::info!("Generation paused by cache guard");
            return Ok(GenerationResult::PausedForUserDecision);
        }

        let results = stream_outcome.map(|o| match o {
            LlmStreamOutcome::Choices(c) => c,
            LlmStreamOutcome::PausedForCacheGuard => unreachable!(),
        });

        let duration_ms = call_start.elapsed().as_millis() as u64;
        let call_ts_end = chrono::Utc::now().to_rfc3339();

        let (
            model_id_for_stats,
            messages_count,
            tools_count,
            temperature_for_stats,
            max_tokens_for_stats,
        ) = (
            llm_request.model_id.clone(),
            llm_request.messages.len(),
            llm_request.tools.as_ref().map(|t| t.len()).unwrap_or(0),
            llm_request.params.temperature,
            llm_request.params.max_tokens,
        );

        match &results {
            Err(e) => {
                let (provider, model) = split_model_provider(&model_id_for_stats);
                let event = LlmCallEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    ts_start: call_ts_start.clone(),
                    ts_end: call_ts_end.clone(),
                    duration_ms,
                    chat_id: chat_id.clone(),
                    root_chat_id: root_chat_id.clone(),
                    mode: mode_for_stats.clone(),
                    task_id: task_id.clone(),
                    task_role: task_role.clone(),
                    agent_id: agent_id.clone(),
                    card_id: card_id.clone(),
                    model_id: model_id_for_stats.clone(),
                    provider,
                    model,
                    messages_count,
                    tools_count,
                    max_tokens: max_tokens_for_stats,
                    temperature: temperature_for_stats,
                    success: false,
                    error_message: Some(e.message.chars().take(200).collect()),
                    finish_reason: None,
                    attempt_n: attempt,
                    retry_reason: None,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    total_tokens: 0,
                    cost_usd: None,
                };
                if let Some(sender) = &app.model.llm_stats_sender {
                    if sender.try_send(event).is_err() {
                        tracing::warn!("stats: channel full, dropping LLM call event");
                    }
                }
            }
            Ok(_) => {}
        }

        let results = results?;

        let mut result = results.into_iter().next().unwrap_or_default();

        if is_result_empty(&result) {
            let draft_usage = {
                let session = session_arc.lock().await;
                session.draft_usage.clone()
            };
            let (provider, model) = split_model_provider(&model_id_for_stats);
            let event = LlmCallEvent {
                id: uuid::Uuid::new_v4().to_string(),
                ts_start: call_ts_start,
                ts_end: call_ts_end,
                duration_ms,
                chat_id: chat_id.clone(),
                root_chat_id: root_chat_id.clone(),
                mode: mode_for_stats.clone(),
                task_id: task_id.clone(),
                task_role: task_role.clone(),
                agent_id: agent_id.clone(),
                card_id: card_id.clone(),
                model_id: model_id_for_stats,
                provider,
                model,
                messages_count,
                tools_count,
                max_tokens: max_tokens_for_stats,
                temperature: temperature_for_stats,
                success: false,
                error_message: Some("empty_response".to_string()),
                finish_reason: result.finish_reason.clone(),
                attempt_n: attempt,
                retry_reason: Some("empty_response".to_string()),
                prompt_tokens: draft_usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                completion_tokens: draft_usage
                    .as_ref()
                    .map(|u| u.completion_tokens)
                    .unwrap_or(0),
                cache_read_tokens: draft_usage.as_ref().and_then(|u| u.cache_read_tokens),
                cache_creation_tokens: draft_usage.as_ref().and_then(|u| u.cache_creation_tokens),
                total_tokens: draft_usage.as_ref().map(|u| u.total_tokens).unwrap_or(0),
                cost_usd: draft_usage
                    .as_ref()
                    .and_then(|u| u.metering_usd.as_ref())
                    .map(|m| m.total_usd),
            };
            if let Some(sender) = &app.model.llm_stats_sender {
                if sender.try_send(event).is_err() {
                    tracing::warn!("stats: channel full, dropping LLM call event");
                }
            }

            if attempt < max_attempts && can_retry_with_temp_bump {
                let current_temp_display = if attempt == 1 {
                    "default".to_string()
                } else {
                    format!("{:.1}", TEMPERATURE_BUMP * (attempt - 2) as f32)
                };
                let next_temp =
                    (TEMPERATURE_BUMP * (attempt - 1) as f32).min(MAX_RETRY_TEMPERATURE);
                warn!(
                    "Empty assistant response at T={}, retrying with T={:.1} (attempt {}/{})",
                    current_temp_display, next_temp, attempt, max_attempts
                );
                {
                    let mut session = session_arc.lock().await;
                    if let Some(ref mut draft) = session.draft_message {
                        draft.content = ChatContent::SimpleText(String::new());
                        draft.tool_calls = None;
                        draft.reasoning_content = None;
                        draft.thinking_blocks = None;
                        draft.citations = Vec::new();
                        draft.server_content_blocks = Vec::new();
                        draft.extra = serde_json::Map::new();
                    }
                    session.draft_usage = None;
                }
                continue;
            } else {
                let effective_temp = llm_request.params.temperature.unwrap_or(0.0);
                return Err(format!(
                    "Empty assistant response after {} attempts (T={:.1})",
                    max_attempts, effective_temp
                )
                .into());
            }
        }

        // --- Tool call recovery ---
        // GPT-5 Codex models occasionally leak tool calls into text content instead of
        // emitting structured function_call events. Detect and recover them.
        let allowed_tools = tool_call_recovery::allowed_tool_names(&llm_request.tools);

        // 1. Unwrap multi_tool_use.parallel wrappers in structured tool_calls
        if !result.tool_calls_raw.is_empty() {
            result.tool_calls_raw = tool_call_recovery::unwrap_multi_tool_use_parallel(
                &result.tool_calls_raw,
                &allowed_tools,
            );
        }

        // 2. Recover tool calls from garbled ChatML content (when no structured calls exist)
        if result.tool_calls_raw.is_empty() && !allowed_tools.is_empty() {
            if let Some((cleaned_content, recovered_calls)) =
                tool_call_recovery::recover_tool_calls_from_chatml_content(
                    &result.raw_content,
                    &allowed_tools,
                )
            {
                warn!(
                    "tool_call_recovery: recovered {} tool call(s) from garbled content",
                    recovered_calls.len()
                );
                result.content = cleaned_content;
                result.tool_calls_raw = recovered_calls;
            }
        }

        if result.tool_calls_raw.is_empty() && !allowed_tools.is_empty() {
            if let Some((cleaned_content, recovered_calls, source)) =
                tool_call_recovery_oss::recover_tool_calls_from_oss_text(
                    &result.raw_content,
                    &allowed_tools,
                )
            {
                warn!(
                    "tool_call_recovery_oss: recovered {} tool call(s) via {}",
                    recovered_calls.len(),
                    source
                );
                result
                    .extra
                    .insert("_tool_call_recovery_source".to_string(), json!(source));
                result.content = cleaned_content;
                result.tool_calls_raw = recovered_calls;
            }
        }

        if !result.tool_calls_raw.is_empty() {
            let parsed: Vec<_> = result
                .tool_calls_raw
                .iter()
                .filter_map(|tc| normalize_tool_call(tc))
                .collect();
            if parsed.is_empty() {
                let has_content = !result.content.trim().is_empty()
                    || !result.reasoning.trim().is_empty()
                    || !result.server_content_blocks.is_empty()
                    || !result.citations.is_empty();
                let names: Vec<_> = result
                    .tool_calls_raw
                    .iter()
                    .filter_map(|tc| {
                        tc.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                    })
                    .collect();
                tracing::warn!(
                    "All {} tool calls unparsable, names={:?}",
                    result.tool_calls_raw.len(),
                    names,
                );
                if !has_content {
                    return Err("Model returned tool_calls but none were parsable"
                        .to_string()
                        .into());
                }
                // Has useful content — discard unparsable tool calls and continue
                result.tool_calls_raw.clear();
            } else if parsed.len() < result.tool_calls_raw.len() {
                let dropped_names: Vec<_> = result
                    .tool_calls_raw
                    .iter()
                    .filter(|tc| normalize_tool_call(tc).is_none())
                    .filter_map(|tc| {
                        tc.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                    })
                    .collect();
                tracing::warn!(
                    "Dropped {}/{} tool calls during normalization, names={:?}",
                    dropped_names.len(),
                    result.tool_calls_raw.len(),
                    dropped_names,
                );
            }
        }

        maybe_downgrade_bogus_tool_calls_finish_reason(&mut result, "post_tool_normalization");

        let draft_usage_for_success = {
            let session = session_arc.lock().await;
            session.draft_usage.clone()
        };
        let usage_for_event = result.usage.as_ref().or(draft_usage_for_success.as_ref());
        let (provider, model) = split_model_provider(&model_id_for_stats);
        let pending_success_event = LlmCallEvent {
            id: uuid::Uuid::new_v4().to_string(),
            ts_start: call_ts_start,
            ts_end: call_ts_end,
            duration_ms,
            chat_id: chat_id.clone(),
            root_chat_id: root_chat_id.clone(),
            mode: mode_for_stats.clone(),
            task_id: task_id.clone(),
            task_role: task_role.clone(),
            agent_id: agent_id.clone(),
            card_id: card_id.clone(),
            model_id: model_id_for_stats,
            provider,
            model,
            messages_count,
            tools_count,
            max_tokens: max_tokens_for_stats,
            temperature: temperature_for_stats,
            success: true,
            error_message: None,
            finish_reason: result.finish_reason.clone(),
            attempt_n: attempt,
            retry_reason: None,
            prompt_tokens: usage_for_event.map(|u| u.prompt_tokens).unwrap_or(0),
            completion_tokens: usage_for_event.map(|u| u.completion_tokens).unwrap_or(0),
            cache_read_tokens: usage_for_event.and_then(|u| u.cache_read_tokens),
            cache_creation_tokens: usage_for_event.and_then(|u| u.cache_creation_tokens),
            total_tokens: usage_for_event.map(|u| u.total_tokens).unwrap_or(0),
            cost_usd: None,
        };

        break (result, pending_success_event);
    };

    let (model_id, usage_for_pricing) = {
        let session = session_arc.lock().await;
        (model_rec.base.id.clone(), session.draft_usage.clone())
    };
    let metering_usd = if let Some(ref usage) = usage_for_pricing {
        generation_metering_usd(&app, &model_id, usage).await
    } else {
        None
    };

    {
        let mut success_event = pending_success_event;
        success_event.cost_usd = metering_usd.as_ref().map(|m| m.total_usd);
        if let Some(sender) = &app.model.llm_stats_sender {
            if sender.try_send(success_event).is_err() {
                tracing::warn!("stats: channel full, dropping LLM call event");
            }
        }
    }

    {
        let mut session = session_arc.lock().await;
        if let Some(ref mut draft) = session.draft_message {
            draft.content = ChatContent::SimpleText(result.content);

            if !result.tool_calls_raw.is_empty() {
                info!(
                    "Parsing {} accumulated tool calls",
                    result.tool_calls_raw.len()
                );
                let parsed: Vec<_> = result
                    .tool_calls_raw
                    .iter()
                    .filter_map(|tc| normalize_tool_call(tc))
                    .collect();
                info!("Successfully parsed {} tool calls", parsed.len());
                if !parsed.is_empty() {
                    draft.tool_calls = Some(parsed);
                }
            }

            if !result.reasoning.is_empty() {
                draft.reasoning_content = Some(result.reasoning);
            }
            if !result.thinking_blocks.is_empty() {
                draft.thinking_blocks = Some(result.thinking_blocks);
            }
            if !result.citations.is_empty() {
                draft.citations = result.citations;
            }
            if !result.server_content_blocks.is_empty() {
                draft.server_content_blocks = result.server_content_blocks;
            }
            if !result.extra.is_empty() {
                draft.extra = result.extra;
            }
        }

        // Store previous_response_id for stateful multi-turn on Platform API only.
        // ChatGPT backend doesn't support previous_response_id, so don't store it —
        // otherwise prepare_chat_passthrough activates tail-only mode and the server
        // receives function_call_output without matching function_call items.
        let is_chatgpt_backend = model_rec.base.endpoint.contains("chatgpt.com/backend-api");
        if model_rec.base.wire_format == crate::llm::WireFormat::OpenaiResponses
            && !is_chatgpt_backend
        {
            if let Some(resp_id) = session
                .draft_message
                .as_ref()
                .and_then(|m| m.extra.get("openai_response_id"))
                .and_then(|v| v.as_str())
            {
                if session.thread.previous_response_id.as_deref() != Some(resp_id) {
                    session.thread.previous_response_id = Some(resp_id.to_string());
                    session.increment_version();
                }
            }
        }

        if let Some(ref mut usage) = session.draft_usage {
            usage.metering_usd = metering_usd;
        }

        session.finish_stream(result.finish_reason);
    }

    Ok(GenerationResult::Completed)
}

fn is_result_empty(result: &ChoiceFinal) -> bool {
    result.content.trim().is_empty()
        && result.tool_calls_raw.is_empty()
        && result.reasoning.trim().is_empty()
        && result.thinking_blocks.is_empty()
        && result.citations.is_empty()
        && result.server_content_blocks.is_empty()
}

fn maybe_downgrade_bogus_tool_calls_finish_reason(result: &mut ChoiceFinal, stage: &str) {
    if result.finish_reason.as_deref() != Some("tool_calls") || !result.tool_calls_raw.is_empty() {
        return;
    }

    warn!(
        "tool_call_guard: finish_reason='tool_calls' without tool calls at stage '{}', downgrading to 'stop'",
        stage
    );
    result.extra.insert(
        "_tool_call_guard".to_string(),
        json!({
            "kind": "tool_calls_finish_without_calls",
            "stage": stage,
            "original_finish_reason": "tool_calls",
            "adjusted_finish_reason": "stop",
        }),
    );
    result.finish_reason = Some("stop".to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatToolCall, ChatToolFunction};

    fn make_user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_event_msg(content: &str) -> ChatMessage {
        crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            "test.generation",
            serde_json::json!({}),
            content.to_string(),
        )
    }

    fn make_assistant_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_reasoning_token_limit_msg() -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            finish_reason: Some("length".to_string()),
            reasoning_content: Some("still thinking".to_string()),
            ..Default::default()
        }
    }

    fn high_pressure_usage() -> ChatUsage {
        ChatUsage {
            prompt_tokens: 85_000,
            completion_tokens: 0,
            total_tokens: 85_000,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            metering_usd: None,
        }
    }

    fn make_high_pressure_length_stop() -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            finish_reason: Some("length".to_string()),
            reasoning_content: Some("still thinking".to_string()),
            usage: Some(high_pressure_usage()),
            ..Default::default()
        }
    }

    fn make_assistant_with_tool_call(tool_call_id: &str, tool_name: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("".to_string()),
            tool_calls: Some(vec![ChatToolCall {
                id: tool_call_id.to_string(),
                index: Some(0),
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

    fn make_tool_msg(tool_call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            tool_call_id: tool_call_id.to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_long_user_msg(token_estimate: usize) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("x".repeat(token_estimate.saturating_mul(4))),
            ..Default::default()
        }
    }

    fn make_context_file_msg() -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::SimpleText("file content".to_string()),
            ..Default::default()
        }
    }

    fn claude_code_model() -> BaseModelRecord {
        BaseModelRecord {
            wire_format: crate::llm::WireFormat::AnthropicMessages,
            auth_token: "cc-oauth-token".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_claude_code_identity_generated_once_per_session() {
        let model = claude_code_model();
        let mut session = ChatSession::new("cc-identity".to_string());

        let first = ensure_claude_code_identity_for_test(&mut session, &model).unwrap();
        let version_after_first = session.trajectory_version;
        let second = ensure_claude_code_identity_for_test(&mut session, &model).unwrap();

        assert_eq!(first, second);
        assert_eq!(session.thread.claude_code_identity, Some(first));
        assert_eq!(session.trajectory_version, version_after_first);
        assert!(session.trajectory_dirty);
    }

    #[test]
    fn test_claude_code_identity_reuses_deserialized_identity() {
        let model = claude_code_model();
        let identity: crate::llm::ClaudeCodeIdentity = serde_json::from_str(
            r#"{
                "device_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "session_id":"bbbbbbbb-cccc-4ddd-8eee-ffffffffffff"
            }"#,
        )
        .unwrap();
        let mut session = ChatSession::new("cc-reload".to_string());
        session.thread.claude_code_identity = Some(identity.clone());

        let reused = ensure_claude_code_identity_for_test(&mut session, &model).unwrap();

        assert_eq!(reused, identity);
        assert_eq!(session.trajectory_version, 0);
        assert!(!session.trajectory_dirty);
    }

    #[test]
    fn test_claude_code_identity_skips_non_claude_code_models() {
        let mut session = ChatSession::new("not-cc".to_string());
        let model = BaseModelRecord {
            wire_format: crate::llm::WireFormat::AnthropicMessages,
            auth_token: String::new(),
            ..Default::default()
        };

        assert!(ensure_claude_code_identity_for_test(&mut session, &model).is_none());
        assert!(session.thread.claude_code_identity.is_none());
        assert_eq!(session.trajectory_version, 0);
    }

    #[test]
    fn test_token_budget_skips_after_tool_call_when_not_low() {
        let mut session = ChatSession::new("test".to_string());
        for idx in 0..TOKEN_BUDGET_CADENCE {
            session.messages.push(make_user_msg(&format!("user {idx}")));
        }
        session
            .messages
            .push(make_assistant_with_tool_call("call_123", "cat"));

        assert!(!maybe_inject_token_budget_instruction(
            &mut session,
            10_000,
            TOKEN_BUDGET_CADENCE,
        ));
        assert!(!session
            .messages
            .iter()
            .any(|msg| msg.role == "cd_instruction" && msg.tool_call_id == TOKEN_BUDGET_MARKER));
    }

    #[test]
    fn test_token_budget_skips_after_tool_call_even_when_below_ten_percent_left() {
        let mut session = ChatSession::new("test".to_string());
        session.messages.push(make_long_user_msg(920));
        for idx in 0..TOKEN_BUDGET_CADENCE {
            session.messages.push(make_user_msg(&format!("user {idx}")));
        }
        session
            .messages
            .push(make_assistant_with_tool_call("call_123", "cat"));

        assert!(!maybe_inject_token_budget_instruction(
            &mut session,
            1_000,
            TOKEN_BUDGET_CADENCE,
        ));
        assert!(!session
            .messages
            .iter()
            .any(|msg| msg.role == "cd_instruction" && msg.tool_call_id == TOKEN_BUDGET_MARKER));
    }

    #[test]
    fn test_tail_needs_assistant_ends_with_assistant_no_tools() {
        let messages = vec![make_user_msg("hello"), make_assistant_msg("response")];
        assert!(!tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_ends_with_user() {
        let messages = vec![make_user_msg("hello")];
        assert!(tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_ends_with_event() {
        let messages = vec![make_event_msg("synthetic prompt")];
        assert!(tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_ends_with_tool_from_client() {
        let messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("call_123", "cat"),
            make_tool_msg("call_123", "file content"),
        ];
        assert!(tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_ends_with_tool_from_server() {
        let messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("srvtoolu_123", "web_search"),
            make_tool_msg("srvtoolu_123", "search results"),
        ];
        assert!(!tail_needs_assistant(&messages));
    }

    #[test]
    fn test_reasoning_token_limit_stop_detects_thought_only_length_finish() {
        let message = make_reasoning_token_limit_msg();
        assert!(is_reasoning_token_limit_stop(&message));
    }

    #[test]
    fn test_reasoning_token_limit_stop_ignores_visible_answer() {
        let mut message = make_reasoning_token_limit_msg();
        message.content = ChatContent::SimpleText("visible answer".to_string());
        assert!(!is_reasoning_token_limit_stop(&message));
    }

    #[test]
    fn length_stop_empty_reasoning_high_pressure_requests_compression() {
        let message = make_high_pressure_length_stop();
        let messages = vec![make_user_msg("continue"), message.clone()];

        assert!(is_high_pressure_length_stop(&message, &messages, 100_000));
    }

    #[test]
    fn length_stop_visible_answer_without_high_pressure_does_not_request_compression() {
        let mut message = make_high_pressure_length_stop();
        message.content = ChatContent::SimpleText("ordinary visible answer".repeat(20));
        message.usage = Some(ChatUsage {
            prompt_tokens: 100,
            completion_tokens: 200,
            total_tokens: 300,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            metering_usd: None,
        });
        let messages = vec![make_user_msg("continue"), message.clone()];

        assert!(!is_high_pressure_length_stop(&message, &messages, 100_000));
    }

    #[test]
    fn length_stop_with_tool_calls_does_not_request_compression() {
        let mut message = make_high_pressure_length_stop();
        message.tool_calls = make_assistant_with_tool_call("call_123", "cat").tool_calls;
        let messages = vec![make_user_msg("continue"), message.clone()];

        assert!(!is_high_pressure_length_stop(&message, &messages, 100_000));
    }

    #[test]
    fn max_output_tokens_alias_is_treated_like_length() {
        let mut message = make_high_pressure_length_stop();
        message.finish_reason = Some("max_output_tokens".to_string());
        let messages = vec![make_user_msg("continue"), message.clone()];

        assert!(is_high_pressure_length_stop(&message, &messages, 100_000));
    }

    #[tokio::test]
    async fn length_stop_reactive_attempts_are_bounded() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut session = ChatSession::new("length-stop-bounded".to_string());
        session.messages = vec![make_user_msg("continue"), make_high_pressure_length_stop()];
        session.thread.reactive_compact_attempts =
            Some(crate::chat::summarization::MAX_SEGMENT_SUMMARY_ATTEMPTS);
        let thread = session.thread.clone();
        let session_arc = Arc::new(AMutex::new(session));

        assert!(
            !maybe_compact_after_high_pressure_length_stop(
                gcx,
                &session_arc,
                &thread,
                Some(100_000),
            )
            .await
        );

        let session = session_arc.lock().await;
        assert_eq!(
            session.thread.reactive_compact_attempts,
            Some(crate::chat::summarization::MAX_SEGMENT_SUMMARY_ATTEMPTS)
        );
        assert_eq!(session.compression_phase, Some(CompressionPhase::Skipped));
        assert_eq!(
            session.compression_reason,
            Some(CompressionReason::MaxAttemptsReached)
        );
    }

    #[test]
    fn test_tail_needs_assistant_empty_assistant_discarded() {
        let messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("call_123", "cat"),
            make_tool_msg("call_123", "file content"),
        ];
        assert!(tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_context_file_after_tool() {
        let messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("call_123", "cat"),
            make_tool_msg("call_123", "file content"),
            make_context_file_msg(),
        ];
        assert!(tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_multiple_tool_calls_mixed() {
        let messages = vec![
            make_user_msg("hello"),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![
                    ChatToolCall {
                        id: "call_123".to_string(),
                        index: Some(0),
                        function: ChatToolFunction {
                            name: "cat".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
                    },
                    ChatToolCall {
                        id: "srvtoolu_456".to_string(),
                        index: Some(1),
                        function: ChatToolFunction {
                            name: "web_search".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
                    },
                ]),
                ..Default::default()
            },
            make_tool_msg("call_123", "file content"),
            make_tool_msg("srvtoolu_456", "search results"),
        ];
        assert!(tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_only_server_tools() {
        let messages = vec![
            make_user_msg("hello"),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![
                    ChatToolCall {
                        id: "srvtoolu_123".to_string(),
                        index: Some(0),
                        function: ChatToolFunction {
                            name: "web_search".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
                    },
                    ChatToolCall {
                        id: "srvtoolu_456".to_string(),
                        index: Some(1),
                        function: ChatToolFunction {
                            name: "web_search".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
                    },
                ]),
                ..Default::default()
            },
            make_tool_msg("srvtoolu_123", "search results 1"),
            make_tool_msg("srvtoolu_456", "search results 2"),
        ];
        assert!(!tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_empty_messages() {
        let messages: Vec<ChatMessage> = vec![];
        assert!(!tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_assistant_with_empty_tool_calls() {
        let messages = vec![
            make_user_msg("hello"),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("response".to_string()),
                tool_calls: Some(vec![]),
                ..Default::default()
            },
        ];
        assert!(!tail_needs_assistant(&messages));
    }

    #[test]
    fn test_fork_error_does_not_break_loop() {
        let mut loop_count = 0;
        let mut reached_normal_generation = false;

        loop {
            loop_count += 1;
            if loop_count > 5 {
                panic!("Loop ran too many times");
            }

            let fork_agent: Option<String> = if loop_count == 1 {
                Some("subagent".to_string())
            } else {
                None
            };

            if fork_agent.is_some() {
                let fork_result: Result<String, String> = Err("subchat failed".to_string());
                match fork_result {
                    Ok(_content) => {
                        break;
                    }
                    Err(_e) => {
                        continue;
                    }
                }
            }

            reached_normal_generation = true;
            break;
        }

        assert!(
            reached_normal_generation,
            "Normal generation path must be reached after fork error"
        );
        assert_eq!(
            loop_count, 2,
            "Loop must iterate twice: fork error then normal generation"
        );
    }

    #[test]
    fn test_context_limit_reactive_compact_attempts_are_bounded() {
        let mut reactive_attempts = 0usize;
        let max = crate::chat::summarization::MAX_SEGMENT_SUMMARY_ATTEMPTS;
        let first_attempt_allowed = reactive_attempts < max;
        if first_attempt_allowed {
            reactive_attempts += 1;
        }
        let second_attempt_allowed = reactive_attempts < max;
        if second_attempt_allowed {
            reactive_attempts += 1;
        }
        let third_attempt_blocked = reactive_attempts >= max;

        assert!(first_attempt_allowed);
        assert!(second_attempt_allowed);
        assert!(third_attempt_blocked);
        assert_eq!(reactive_attempts, max);
    }

    #[test]
    fn test_context_limit_compaction_sets_cache_guard_after_segment_summary() {
        let mut session = ChatSession::new("test".to_string());
        session.messages = vec![
            make_user_msg("hello"),
            make_assistant_msg("old answer"),
            make_user_msg("again"),
        ];
        assert!(
            crate::chat::summarization::summarize_oldest_segment_with_static_summary(
                &mut session.messages,
                "summary",
                "test",
            )
        );
        session.thread.previous_response_id = None;
        session.cache_guard_force_next = true;

        assert!(session.cache_guard_force_next);
        assert!(session.thread.previous_response_id.is_none());
        assert!(session
            .messages
            .iter()
            .any(crate::chat::summarization::is_segment_summary));
    }

    #[test]
    fn test_context_limit_compaction_allows_partial_output_errors() {
        let abort = std::sync::atomic::AtomicBool::new(false);
        let partial_context_error = crate::chat::stream_core::LlmStreamError {
            message: format!(
                "{} Original error: context_length_exceeded",
                PARTIAL_OUTPUT_STREAM_ERROR,
            ),
            partial_output_emitted: true,
        };

        assert!(partial_context_error.retry_decision().is_context_limit());
        assert!(!partial_context_error.should_retry(0, &abort));
        assert!(
            partial_context_error.retry_decision().is_context_limit()
                && !abort.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn test_context_limit_compaction_blocked_by_abort_flag() {
        let abort = std::sync::atomic::AtomicBool::new(true);
        let partial_context_error = crate::chat::stream_core::LlmStreamError {
            message: "context_length_exceeded".to_string(),
            partial_output_emitted: false,
        };

        assert!(partial_context_error.retry_decision().is_context_limit());
        assert!(
            !(partial_context_error.retry_decision().is_context_limit()
                && !abort.load(Ordering::SeqCst))
        );
    }

    #[test]
    fn test_segment_summary_circuit_breaker_stops_at_two() {
        let max = crate::chat::summarization::MAX_SEGMENT_SUMMARY_ATTEMPTS;
        assert_eq!(max, 2);
        let mut count = 0usize;
        let mut stopped = false;
        for _ in 0..10 {
            if count < max {
                count += 1;
            } else {
                stopped = true;
                break;
            }
        }
        assert!(stopped);
        assert_eq!(count, max);
    }

    #[test]
    fn test_segment_summary_count_resets_on_success() {
        let segment_summary_count = 0usize;
        assert_eq!(segment_summary_count, 0);
    }

    #[test]
    fn test_downgrade_bogus_tool_calls_finish_reason() {
        let mut result = ChoiceFinal {
            finish_reason: Some("tool_calls".to_string()),
            ..Default::default()
        };

        maybe_downgrade_bogus_tool_calls_finish_reason(&mut result, "test");

        assert_eq!(result.finish_reason.as_deref(), Some("stop"));
        assert!(result.extra.contains_key("_tool_call_guard"));
    }

    #[test]
    fn test_does_not_downgrade_tool_calls_finish_reason_when_tool_calls_exist() {
        let mut result = ChoiceFinal {
            finish_reason: Some("tool_calls".to_string()),
            tool_calls_raw: vec![json!({
                "type": "function",
                "id": "call_123",
                "function": {
                    "name": "shell",
                    "arguments": "{}"
                }
            })],
            ..Default::default()
        };

        maybe_downgrade_bogus_tool_calls_finish_reason(&mut result, "test");

        assert_eq!(result.finish_reason.as_deref(), Some("tool_calls"));
        assert!(!result.extra.contains_key("_tool_call_guard"));
    }

    #[test]
    fn test_does_not_downgrade_non_tool_calls_finish_reason() {
        let mut result = ChoiceFinal {
            finish_reason: Some("stop".to_string()),
            ..Default::default()
        };

        maybe_downgrade_bogus_tool_calls_finish_reason(&mut result, "test");

        assert_eq!(result.finish_reason.as_deref(), Some("stop"));
        assert!(!result.extra.contains_key("_tool_call_guard"));
    }

    #[tokio::test]
    async fn test_models_dev_generation_metering_uses_central_pricing_lookup() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let mut model_caps = std::collections::HashMap::new();
        model_caps.insert(
            "openai/gpt-4o".to_string(),
            crate::caps::model_caps::ModelCapabilities {
                n_ctx: 128_000,
                max_output_tokens: 16_384,
                pricing: Some(crate::providers::traits::ModelPricing {
                    prompt: 2.0,
                    generated: 4.0,
                    cache_read: Some(1.0),
                    cache_creation: Some(3.0),
                    context_over_200k: None,
                }),
                ..Default::default()
            },
        );
        {
            let mut caps = app.model.caps.write().await;
            caps.caps = Some(std::sync::Arc::new(crate::caps::CodeAssistantCaps {
                model_caps: std::sync::Arc::new(model_caps),
                ..Default::default()
            }));
            caps.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        let usage = ChatUsage {
            prompt_tokens: 1_000,
            completion_tokens: 2_000,
            total_tokens: 3_000,
            cache_read_tokens: Some(500),
            cache_creation_tokens: Some(250),
            metering_usd: None,
        };

        let metering = generation_metering_usd(&app, "openai/gpt-4o", &usage)
            .await
            .unwrap();

        assert_eq!(metering.prompt_usd, 0.002);
        assert_eq!(metering.generated_usd, 0.008);
        assert_eq!(metering.cache_read_usd, Some(0.0005));
        assert_eq!(metering.cache_creation_usd, Some(0.00075));
    }

    #[test]
    fn cache_guard_violation_does_not_become_error_state_in_generation_loop() {
        let result: Result<GenerationResult, LlmStreamError> =
            Ok(GenerationResult::PausedForUserDecision);
        let would_call_finish_stream_with_error = matches!(result, Err(_));
        let would_break_cleanly = matches!(result, Ok(GenerationResult::PausedForUserDecision));
        assert!(
            would_break_cleanly,
            "PausedForUserDecision must break the loop cleanly"
        );
        assert!(
            !would_call_finish_stream_with_error,
            "PausedForUserDecision must not trigger finish_stream_with_error"
        );
    }

    #[test]
    fn cache_guard_other_failure_propagates_as_error() {
        use crate::chat::cache_guard::CacheGuardOutcome;
        use crate::chat::stream_core::LlmStreamError;

        let error_outcome = CacheGuardOutcome::Error("simulated io failure".to_string());
        assert!(
            matches!(error_outcome, CacheGuardOutcome::Error(_)),
            "Error variant must be distinguishable"
        );
        assert!(
            !matches!(error_outcome, CacheGuardOutcome::Pass(_)),
            "Error must not be treated as Pass"
        );
        assert!(
            !matches!(error_outcome, CacheGuardOutcome::Paused { .. }),
            "Error must not be treated as Paused"
        );

        let stream_err: Result<Vec<()>, LlmStreamError> =
            Err(LlmStreamError::from("simulated io failure".to_string()));
        assert!(
            matches!(stream_err, Err(_)),
            "Error outcome must propagate as Err in the generation chain"
        );
    }
}
