use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
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
use crate::knowledge::enrichment::{
    enrich_messages_with_knowledge, enrichment_identity_from_context, enrichment_query_fingerprint,
    record_enrichment_identity_on_user_message,
};
use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome};

use super::goal_monitor::handle_goal_turn_end;
use super::types::*;
use super::trajectories::{
    check_external_reload_pending, ensure_frozen_prefix, first_system_prompt,
    frozen_prefix_is_complete, maybe_save_trajectory_background_with_intent,
};
use super::tools::{process_tool_calls_once, ToolStepOutcome};
use super::prepare::{
    build_canonical_openai_tools_from_aliases, prepare_chat_passthrough, ChatPrepareOptions,
    PrepareOutcome,
};
use super::prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use super::stream_core::{
    StreamRunParams, StreamCollector, normalize_tool_call, ChoiceFinal, LlmStreamError,
    LlmStreamOutcome, ABORT_ERROR_MESSAGE,
};
use super::config::tokens;
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{HookPayload, get_project_dir_string, run_observation_hooks};
use crate::chat::diagnostics::{make_ui_only_error_message, safe_provider_error_diagnostic};
use crate::chat::history_limit::ContextPressure;
use refact_core::llm_types::BaseModelRecord;
use refact_core::provider_types::{ModelTypeDefaults, ProviderDefaults};

const MCP_LAZY_INDEX_MARKER: &str = "mcp_lazy_index";
const LENGTH_STOP_NEAR_EMPTY_VISIBLE_CHARS: usize = 32;
const PARTIAL_OUTPUT_STREAM_ERROR: &str =
    "Stream interrupted after partial output and all retry attempts failed.";
const RESPONSES_INCOMPLETE_STREAM_ERROR: &str =
    "LLM stream ended unexpectedly without completion signal";
const RESPONSES_CONTEXT_CUTOFF_ERROR: &str =
    "context_length_exceeded: Responses stream ended before a terminal event at critical context pressure";

const DELTA_COALESCE_WINDOW: std::time::Duration = std::time::Duration::from_millis(10);
const MAX_DELTA_BATCH_EVENTS: usize = 64;
const MAX_DELTA_OPS_PER_EMIT: usize = 128;
const MAX_DELTA_TEXT_BYTES: usize = 64 * 1024;

fn delta_ops_contain_visible_content(ops: &[DeltaOp]) -> bool {
    ops.iter().any(|op| match op {
        DeltaOp::AppendContent {
            text,
        }
        | DeltaOp::AppendReasoning {
            text,
        }
        | DeltaOp::SetReasoning {
            text,
        } => !text.is_empty(),
        DeltaOp::SetThinkingBlocks {
            blocks,
        } => !blocks.is_empty(),
        _ => false,
    })
}

fn delta_ops_contain_assistant_content(ops: &[DeltaOp]) -> bool {
    ops.iter()
        .any(|op| matches!(op, DeltaOp::AppendContent { text } if !text.is_empty()))
}

fn delta_is_coalescible_text(op: &DeltaOp) -> bool {
    matches!(
        op,
        DeltaOp::AppendContent { .. } | DeltaOp::AppendReasoning { .. }
    )
}

fn delta_text_len(op: &DeltaOp) -> usize {
    match op {
        DeltaOp::AppendContent {
            text,
        }
        | DeltaOp::AppendReasoning {
            text,
        } => text.len(),
        _ => 0,
    }
}

fn append_coalesced_text_op(ops: &mut Vec<DeltaOp>, op: DeltaOp) {
    match op {
        DeltaOp::AppendContent {
            text,
        } => {
            if let Some(DeltaOp::AppendContent {
                text: previous,
            }) = ops.last_mut()
            {
                previous.push_str(&text);
            } else {
                ops.push(DeltaOp::AppendContent {
                    text,
                });
            }
        }
        DeltaOp::AppendReasoning {
            text,
        } => {
            if let Some(DeltaOp::AppendReasoning {
                text: previous,
            }) = ops.last_mut()
            {
                previous.push_str(&text);
            } else {
                ops.push(DeltaOp::AppendReasoning {
                    text,
                });
            }
        }
        _ => ops.push(op),
    }
}

fn split_delta_text(text: &str) -> Vec<String> {
    if text.len() <= MAX_DELTA_TEXT_BYTES {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let mut end = (start + MAX_DELTA_TEXT_BYTES).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = text[start..]
                .char_indices()
                .nth(1)
                .map(|(index, _)| start + index)
                .unwrap_or(text.len());
        }
        chunks.push(text[start..end].to_string());
        start = end;
    }
    chunks
}

fn split_large_delta_text_ops(ops: Vec<DeltaOp>) -> Vec<DeltaOp> {
    let mut split = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            DeltaOp::AppendContent {
                text,
            } => split.extend(split_delta_text(&text).into_iter().map(|text| {
                DeltaOp::AppendContent {
                    text,
                }
            })),
            DeltaOp::AppendReasoning {
                text,
            } => split.extend(split_delta_text(&text).into_iter().map(|text| {
                DeltaOp::AppendReasoning {
                    text,
                }
            })),
            DeltaOp::SetReasoning {
                text,
            } => {
                let chunks = split_delta_text(&text);
                if let Some((first, rest)) = chunks.split_first() {
                    split.push(DeltaOp::SetReasoning {
                        text: first.clone(),
                    });
                    split.extend(rest.iter().cloned().map(|text| DeltaOp::AppendReasoning {
                        text,
                    }));
                }
            }
            other => split.push(other),
        }
    }
    split
}

pub(crate) fn batch_stream_delta_ops(ops: Vec<DeltaOp>) -> Vec<Vec<DeltaOp>> {
    let mut batches = Vec::new();
    let mut text_ops = Vec::new();
    let mut text_bytes = 0usize;
    let flush_text = |batches: &mut Vec<Vec<DeltaOp>>, text_ops: &mut Vec<DeltaOp>| {
        if !text_ops.is_empty() {
            batches.push(std::mem::take(text_ops));
        }
    };
    for op in split_large_delta_text_ops(ops) {
        if !delta_is_coalescible_text(&op) {
            flush_text(&mut batches, &mut text_ops);
            text_bytes = 0;
            batches.push(vec![op]);
            continue;
        }
        let op_bytes = delta_text_len(&op);
        if text_ops.len() >= MAX_DELTA_OPS_PER_EMIT
            || text_bytes.saturating_add(op_bytes) > MAX_DELTA_TEXT_BYTES
        {
            flush_text(&mut batches, &mut text_ops);
        }
        append_coalesced_text_op(&mut text_ops, op);
        text_bytes = text_ops.iter().map(delta_text_len).sum();
    }
    flush_text(&mut batches, &mut text_ops);
    batches
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NormalizedStopReason {
    ProviderLengthStop,
    ContextLengthStop,
}

pub(crate) fn normalize_stop_reason(reason: &str) -> Option<NormalizedStopReason> {
    let lower = reason.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    if lower.contains("context_length_exceeded")
        || lower.contains("maximum context")
        || lower.contains("context window")
        || lower.contains("context length")
        || lower.contains("input too long")
        || lower.contains("input is too long")
        || lower.contains("model_length")
        || lower.contains("prompt is too long")
    {
        return Some(NormalizedStopReason::ContextLengthStop);
    }
    match lower.as_str() {
        "length"
        | "max_tokens"
        | "max_output_tokens"
        | "token_limit"
        | "max_tokens_stop"
        | "max_tokens_exceeded" => Some(NormalizedStopReason::ProviderLengthStop),
        _ => None,
    }
}

fn compression_reason_for_stop_reason(reason: NormalizedStopReason) -> CompressionReason {
    match reason {
        NormalizedStopReason::ProviderLengthStop => CompressionReason::ProviderLengthStop,
        NormalizedStopReason::ContextLengthStop => CompressionReason::ContextLengthStop,
    }
}

fn is_responses_incomplete_stream_error(error: &LlmStreamError) -> bool {
    error.message.contains(RESPONSES_INCOMPLETE_STREAM_ERROR)
        || error
            .message
            .contains("OpenAI Codex WebSocket ended before completion")
        || error
            .message
            .contains("OpenAI Codex WebSocket closed before completion")
}

fn responses_incomplete_stream_at_critical_pressure(
    error: &LlmStreamError,
    model_rec: &BaseModelRecord,
    messages: &[ChatMessage],
    effective_n_ctx: usize,
    usage_stale: bool,
) -> bool {
    model_rec.wire_format == crate::llm::WireFormat::OpenaiResponses
        && effective_n_ctx > 0
        && is_responses_incomplete_stream_error(error)
        && matches!(
            crate::chat::summarization::estimated_provider_context_pressure_with_usage_for_image_mode(
                messages,
                effective_n_ctx,
                usage_stale,
                model_rec.image_token_mode,
            ),
            ContextPressure::Critical
        )
}

fn synthesize_responses_context_cutoff_error_if_needed(
    mut error: LlmStreamError,
    model_rec: &BaseModelRecord,
    messages: &[ChatMessage],
    effective_n_ctx: usize,
    usage_stale: bool,
) -> LlmStreamError {
    if responses_incomplete_stream_at_critical_pressure(
        &error,
        model_rec,
        messages,
        effective_n_ctx,
        usage_stale,
    ) {
        error.message = format!(
            "{RESPONSES_CONTEXT_CUTOFF_ERROR}. Original error: {}",
            error.message
        );
    }
    error
}

fn context_limit_final_error_message(error: &str) -> String {
    format!(
        "Context too large and context reconstruction failed or the rebuilt context is still too large. Explicitly rebuild the context or start a new chat. Original error: {}",
        safe_provider_error_diagnostic(error)
    )
}

fn partial_output_stream_error_message(original: &str) -> String {
    format!(
        "{} Original error: {}",
        PARTIAL_OUTPUT_STREAM_ERROR,
        safe_provider_error_diagnostic(original),
    )
}

fn model_type_defaults_for_thread<'a>(
    user_defaults: &'a ProviderDefaults,
    defaults: &crate::caps::DefaultModels,
    thread: &ThreadParams,
    model_id: &str,
) -> &'a ModelTypeDefaults {
    if thread
        .task_meta
        .as_ref()
        .is_some_and(|meta| meta.role == "agents")
    {
        user_defaults.defaults_for_task_planner_agent_model(
            model_id,
            &defaults.task_planner_agent_model,
            &defaults.chat_default_model,
            &defaults.chat_model_2,
            &defaults.chat_light_model,
            &defaults.chat_thinking_model,
            &defaults.chat_buddy_model,
        )
    } else {
        user_defaults.defaults_for_model(
            model_id,
            &defaults.chat_default_model,
            &defaults.chat_model_2,
            &defaults.chat_light_model,
            &defaults.chat_thinking_model,
            &defaults.chat_buddy_model,
        )
    }
}

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

fn build_mcp_lazy_hint(total: usize) -> String {
    format!(
        "💿 MCP Tools — Lazy Mode Active ({total} tools available). \
         You MUST call `mcp_tool_search` before using any MCP tool. \
         Search by capability or provider, or use \
         mcp_tool_search({{\"query\": \".\", \"max_results\": {total}}}) to list all available MCP tools."
    )
}

fn inject_mcp_lazy_hint(session: &mut ChatSession, total: usize) -> bool {
    let active = match refact_core::active_context::active_context(&session.messages) {
        Ok(active) => active.messages,
        Err(error) => {
            warn!("Skipping MCP lazy hint for {}: {}", session.chat_id, error);
            return false;
        }
    };
    if active.iter().any(|message| {
        message.role == "cd_instruction" && message.tool_call_id == MCP_LAZY_INDEX_MARKER
    }) {
        return false;
    }

    let insert_pos = active
        .iter()
        .position(|message| message.role == "system")
        .map(|index| index + 1)
        .unwrap_or(0);
    let hint = ChatMessage {
        role: "cd_instruction".to_string(),
        tool_call_id: MCP_LAZY_INDEX_MARKER.to_string(),
        content: ChatContent::SimpleText(build_mcp_lazy_hint(total)),
        ..Default::default()
    };
    match session.insert_active_message(insert_pos, hint) {
        Ok(_) => true,
        Err(error) => {
            warn!("Failed to inject MCP lazy hint: {}", error);
            false
        }
    }
}

fn is_preamble_conversation_start(message: &ChatMessage) -> bool {
    message.role == "user"
        || message.role == "assistant"
        || message.role == crate::chat::internal_roles::EVENT_ROLE
}

fn preamble_already_present(active_view: &[ChatMessage], msg: &ChatMessage) -> bool {
    if !msg.message_id.is_empty() && active_view.iter().any(|m| m.message_id == msg.message_id) {
        return true;
    }
    match msg.role.as_str() {
        "assistant" | "compression_report" => true,
        "system" => active_view.first().is_some_and(|m| m.role == "system"),
        "cd_instruction" => active_view.iter().any(|m| m.role == "cd_instruction"),
        "context_file" => active_view
            .iter()
            .any(|m| m.role == "context_file" && m.tool_call_id == msg.tool_call_id),
        _ => false,
    }
}

/// Persist the preamble messages the prompt mixer placed ahead of the conversation
/// into the active view, so they reach the model even after a context rebuild.
fn persist_preamble_messages(
    session: &mut ChatSession,
    messages_with_preamble: &[ChatMessage],
) -> usize {
    let first_conv_idx = messages_with_preamble
        .iter()
        .position(is_preamble_conversation_start)
        .unwrap_or(messages_with_preamble.len());
    let Ok(active) = refact_core::active_context::active_context(&session.messages) else {
        return 0;
    };
    let mut active_view = active.messages;
    let mut system_insert_idx = 0;
    let mut context_insert_idx = active_view
        .iter()
        .position(|m| m.role == "system")
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut inserted = 0;
    for msg in messages_with_preamble.iter().take(first_conv_idx) {
        if preamble_already_present(&active_view, msg) {
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
        match session.insert_active_message(insert_idx, msg.clone()) {
            Ok(_) => {
                active_view.insert(insert_idx.min(active_view.len()), msg.clone());
                inserted += 1;
            }
            Err(error) => {
                warn!(
                    "Failed to persist preamble message ({}): {}",
                    msg.role, error
                );
                break;
            }
        }
    }
    inserted
}

fn enrichment_insertion_index(
    session: &ChatSession,
    turn: &EnrichmentTurnIdentity,
    identity: Option<&str>,
) -> Option<usize> {
    let user_index = session
        .messages
        .iter()
        .rposition(|message| message.role == "user")?;
    let user_message = session.messages.get(user_index)?;
    (session.trajectory_version == turn.revision
        && user_index == session.messages.len().saturating_sub(1)
        && user_message.message_id == turn.latest_user_id
        && enrichment_query_fingerprint(&user_message.content.content_text_only())
            == turn.query_fingerprint
        && identity.is_some_and(|identity| !session.has_enrichment_identity(identity)))
    .then_some(user_index)
}

// Construct checkpoint futures inside their own poll frame. In debug builds,
// each inline await otherwise reserves large construction temporaries in the
// caller, even while a different branch (such as generation) is being polled.
pub(super) fn maybe_save_trajectory_with_intent(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    intent: TrajectoryCommitIntent,
) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        super::trajectories::maybe_save_trajectory_with_intent(app, session_arc, intent).await;
    })
}

pub fn prepare_session_preamble_and_knowledge(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        let Ok(_operation) =
            super::session_runtime::begin_board_operation(&app, &session_arc).await
        else {
            return;
        };
        session_arc
            .lock()
            .await
            .live_diagnostics
            .phase("preamble", Instant::now());
        let preamble_span =
            perf_diagnostics::span(PerfComponent::StreamPreamblePrepare, None, None);
        ensure_session_preamble_inner(app.clone(), session_arc.clone()).await;
        let knowledge_span = perf_diagnostics::span(PerfComponent::PreambleKnowledge, None, None);
        prepare_session_knowledge_inner(app.clone(), session_arc.clone()).await;
        knowledge_span.finish(PerfOutcome::Success, None, None, None, None);
        let work_span = perf_diagnostics::span(PerfComponent::PreambleWorkContext, None, None);
        prepare_project_work_context(&app, &session_arc).await;
        work_span.finish(PerfOutcome::Success, None, None, None, None);
        preamble_span.finish(PerfOutcome::Success, None, None, None, None);
        session_arc
            .lock()
            .await
            .live_diagnostics
            .phase("request_preparing", Instant::now());
    })
}

/// A rebuilt or handed-off context starts without its preamble, so the generation
/// loop re-checks the active head before every request; the full preamble pipeline
/// only runs when the system prompt is actually missing.
pub fn ensure_session_preamble(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        let system_missing = {
            let session = session_arc.lock().await;
            refact_core::active_context::active_context(&session.messages)
                .is_ok_and(|active| !active.messages.first().is_some_and(|m| m.role == "system"))
        };
        if system_missing {
            ensure_session_preamble_inner(app, session_arc).await;
        }
    })
}

/// Only call with metadata copied from a live, trusted session. Stateless callers
/// retain the registry's Policy default; a mode name is never evidence of authority.
pub(crate) async fn tool_catalog_intent_for_thread(
    app: &AppState,
    thread: &ThreadParams,
) -> refact_runtime_api::ToolCatalogIntent {
    use refact_runtime_api::ToolCatalogIntent;
    let user_lineage = match thread.link_type.as_deref() {
        Some("handoff" | "mode_transition" | "branch") => true,
        None => thread.parent_id.is_none(),
        Some(_) => false,
    };
    if !user_lineage
        || thread
            .task_meta
            .as_ref()
            .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents"))
        || app
            .agents
            .find_agent_id_by_child_chat_id(&thread.id)
            .await
            .is_some()
    {
        ToolCatalogIntent::Policy
    } else {
        ToolCatalogIntent::UserFacing
    }
}

async fn ensure_session_preamble_inner(app: AppState, session_arc: Arc<AMutex<ChatSession>>) {
    let gcx = app.gcx.clone();
    let (thread, chat_id, has_system, has_project_context) = {
        let session = session_arc.lock().await;
        let active = match refact_core::active_context::active_context(&session.messages) {
            Ok(active) => active.messages,
            Err(error) => {
                warn!("Skipping preamble for {}: {}", session.chat_id, error);
                return;
            }
        };
        let has_sys = active.first().is_some_and(|m| m.role == "system");
        let has_proj = active.iter().any(|m| {
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

    // Populated inside `needs_preamble`; used after to inject the MCP lazy-mode hint message.
    let mut mcp_lazy_total: Option<usize> = None;

    if needs_preamble {
        let caps_span = perf_diagnostics::span(PerfComponent::PreambleCaps, None, None);
        let caps = match crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
            .await
        {
            Ok(caps) => caps,
            Err(e) => {
                caps_span.finish(PerfOutcome::Failure, None, None, None, None);
                warn!("Failed to load caps for preamble: {}", e.message);
                return;
            }
        };
        let model_rec = match crate::caps::resolve_chat_model(caps.clone(), &thread.model) {
            Ok(rec) => rec,
            Err(e) => {
                caps_span.finish(PerfOutcome::Failure, None, None, None, None);
                warn!("Failed to resolve model for preamble: {}", e);
                return;
            }
        };
        caps_span.finish(PerfOutcome::Success, None, None, None, None);

        let execution_scope = thread
            .worktree
            .as_ref()
            .map(|worktree| worktree.root.to_string_lossy().into_owned());
        let intent = tool_catalog_intent_for_thread(&app, &thread).await;
        let catalog_span = perf_diagnostics::span(PerfComponent::PreambleCatalog, None, None);
        let tools_for_mode = app
            .tool_registry
            .acquire_tool_catalog_with_intent(
                &thread.mode,
                Some(&model_rec.base.id),
                execution_scope.as_deref(),
                intent,
            )
            .await;
        let tools_for_mode =
            crate::tools::tools_list::filter_tool_catalog_for_chat(&app, &chat_id, tools_for_mode)
                .await;
        catalog_span.finish(PerfOutcome::Success, None, None, None, None);
        let tools_for_mode = &tools_for_mode.index;
        if tools_for_mode.mcp_lazy_mode {
            mcp_lazy_total = Some(tools_for_mode.mcp_total_count);
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

        let prompt_span = perf_diagnostics::span(PerfComponent::PreamblePrompt, None, None);
        let messages = {
            let session = session_arc.lock().await;
            match refact_core::active_context::active_context(&session.messages) {
                Ok(active) => active.messages,
                Err(error) => {
                    warn!("Skipping preamble for {}: {}", session.chat_id, error);
                    return;
                }
            }
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
        prompt_span.finish(PerfOutcome::Success, None, None, None, None);

        let mut session = session_arc.lock().await;
        session.skills_available_count = skills_info.available_count;
        session.skills_included = skills_info.included_names.clone();
        let inserted = persist_preamble_messages(&mut session, &messages_with_preamble);
        if inserted > 0 {
            info!("Saved {} preamble messages to session", inserted);
        }
    }

    // Inject MCP lazy-mode discovery hint (once per session, idempotent via marker).
    // Detailed names and schemas are returned only when mcp_tool_search is called.
    if let Some(mcp_total) = mcp_lazy_total {
        let inserted = {
            let mut session = session_arc.lock().await;
            inject_mcp_lazy_hint(&mut session, mcp_total)
        };
        if inserted {
            info!("Injected MCP lazy discovery hint with {} tools", mcp_total);
        }
    }
}

async fn prepare_session_knowledge_inner(app: AppState, session_arc: Arc<AMutex<ChatSession>>) {
    let gcx = app.gcx.clone();
    let (thread, chat_id) = {
        let session = session_arc.lock().await;
        (session.thread.clone(), session.chat_id.clone())
    };

    let (
        last_is_user,
        auto_enrichment_enabled,
        user_count,
        has_manual_enrichment_for_turn,
        suppress_flag,
        enrichment_turn,
    ) = {
        let snapshot_started = perf_diagnostics::is_enabled().then(std::time::Instant::now);
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
        let enrichment_turn = last_user_idx
            .filter(|idx| *idx == session.messages.len().saturating_sub(1))
            .and_then(|idx| session.messages.get(idx))
            .filter(|message| message.role == "user")
            .map(|message| EnrichmentTurnIdentity {
                revision: session.trajectory_version,
                latest_user_id: message.message_id.clone(),
                query_fingerprint: enrichment_query_fingerprint(
                    &message.content.content_text_only(),
                ),
            });
        let message_bytes = perf_diagnostics::is_enabled().then(|| {
            session
                .messages
                .iter()
                .map(|message| {
                    serde_json::to_vec(message)
                        .map(|encoded| encoded.len())
                        .unwrap_or(0)
                })
                .sum::<usize>()
        });
        perf_diagnostics::record_enrichment(
            PerfComponent::EnrichmentSessionSnapshot,
            &chat_id,
            PerfOutcome::Success,
            snapshot_started
                .map(|started| started.elapsed().as_micros().try_into().unwrap_or(u64::MAX))
                .unwrap_or(0),
            message_bytes.map(|bytes| bytes as u64),
            Some(session.messages.len() as u64),
            message_bytes.map(|bytes| (bytes as u64).saturating_add(3) / 4),
        );
        (last_user, auto, count, manual, suppress, enrichment_turn)
    };
    if is_agentic_mode_id(&thread.mode)
        && last_is_user
        && auto_enrichment_enabled
        && !has_manual_enrichment_for_turn
        && !suppress_flag
        && enrichment_turn.is_some()
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
                    let turn = enrichment_turn
                        .as_ref()
                        .expect("turn checked before enrichment");
                    let identity = enrichment_identity_from_context(enriched_msg);
                    let session_last_user_idx =
                        enrichment_insertion_index(&session, turn, identity.as_deref());
                    let insertion_stale = session_last_user_idx.is_none();
                    if let Some(session_last_user_idx) = session_last_user_idx {
                        let identity = identity.expect("identity checked before insertion");
                        if let Some(user_message) = session.messages.get_mut(session_last_user_idx)
                        {
                            record_enrichment_identity_on_user_message(
                                user_message,
                                &identity,
                                &turn.query_fingerprint,
                            );
                        }
                        session.record_enrichment_identity(identity);
                        session.insert_message(session_last_user_idx, enriched_msg.clone());
                    }
                    if insertion_stale {
                        perf_diagnostics::record_enrichment(
                            PerfComponent::EnrichmentInsertionStale,
                            &chat_id,
                            PerfOutcome::Skipped,
                            0,
                            None,
                            Some(1),
                            None,
                        );
                    } else {
                        perf_diagnostics::record_enrichment(
                            PerfComponent::EnrichmentPersistenceScheduling,
                            &chat_id,
                            PerfOutcome::Success,
                            0,
                            None,
                            Some(1),
                            None,
                        );
                    }
                    if !insertion_stale {
                        info!(
                            "Saved auto knowledge enrichment context_file to session at index {}",
                            session_last_user_idx.unwrap_or_default()
                        );
                    }
                }
            }
        }
    }
}

pub async fn prepare_project_work_context(app: &AppState, session_arc: &Arc<AMutex<ChatSession>>) {
    let snapshot_span = perf_diagnostics::span(PerfComponent::WorkContextSnapshot, None, None);
    let (thread, chat_id, version, source_root, mut state) = {
        let mut session = session_arc.lock().await;
        if session.closed || !is_agentic_mode_id(&session.thread.mode) {
            return;
        }
        let mut recovered = session.work_capsule_state.clone();
        recovered.reconcile(&session.messages);
        session.work_capsule_state = recovered;
        (
            session.thread.clone(),
            session.chat_id.clone(),
            session.trajectory_version,
            crate::coordination::scope::session_source_root(&session),
            session.work_capsule_state.clone(),
        )
    };
    snapshot_span.finish(PerfOutcome::Success, None, None, None, None);
    let original_state = state.clone();
    let message = crate::coordination::capsule::prepare_work_context(
        app,
        &mut state,
        &chat_id,
        &thread.model,
    )
    .await;
    let publish_span = perf_diagnostics::span(PerfComponent::WorkContextPublish, None, None);
    let mut session = session_arc.lock().await;
    if session.closed
        || session.trajectory_version != version
        || session.thread.model != thread.model
        || session.thread.worktree != thread.worktree
        || crate::coordination::scope::session_source_root(&session) != source_root
        || session.work_capsule_state != original_state
        || message
            .as_ref()
            .is_some_and(|message| !crate::coordination::capsule::still_cleared(app, message))
    {
        return;
    }
    session.work_capsule_state = state;
    if let Some(message) = message {
        session.add_message(message);
    }
    publish_span.finish(PerfOutcome::Success, None, None, None, None);
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

fn latest_assistant_tool_call_window_closed(messages: &[ChatMessage]) -> bool {
    let Some((assistant_index, assistant)) = messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| message.role == "assistant")
    else {
        return true;
    };

    let Some(tool_calls) = assistant.tool_calls.as_ref() else {
        return true;
    };
    if tool_calls.is_empty() {
        return true;
    }

    let mut answered_ids = std::collections::HashSet::new();
    for message in messages.iter().skip(assistant_index + 1) {
        match message.role.as_str() {
            "tool" | "diff" if !message.tool_call_id.is_empty() => {
                answered_ids.insert(message.tool_call_id.as_str());
            }
            role if is_prompt_turn_role(role) || role == "assistant" => {
                break;
            }
            _ => {}
        }
    }

    tool_calls
        .iter()
        .all(|tc| answered_ids.contains(tc.id.as_str()))
}

async fn inject_priority_messages_before_llm_if_safe(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> bool {
    let can_inject = {
        let session = session_arc.lock().await;
        latest_assistant_tool_call_window_closed(&session.messages)
    };

    can_inject && crate::chat::queue::inject_priority_messages_if_any(app, session_arc).await
}

fn is_claude_code_model(model: &BaseModelRecord) -> bool {
    model.wire_format == crate::llm::WireFormat::AnthropicMessages && !model.auth_token.is_empty()
}

async fn completion_activity(session_arc: &Arc<AMutex<ChatSession>>) -> ChatActivityCompletion {
    let session = session_arc.lock().await;
    ChatActivityCompletion {
        chat_id: session.chat_id.clone(),
        thread: session.thread.clone(),
    }
}

async fn spawn_completion_activity_reaction(app: AppState, session_arc: Arc<AMutex<ChatSession>>) {
    let activity = completion_activity(&session_arc).await;
    let mut session = session_arc.lock().await;
    if session.closed {
        return;
    }
    let handle = tokio::spawn(async move {
        maybe_enqueue_chat_activity_reaction(app, activity).await;
    });
    session.track_post_turn_task(handle);
}

async fn spawn_chat_completion_side_effects(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    chat_id: String,
    chat_label: String,
    run_stop_hook: bool,
) {
    let activity = completion_activity(&session_arc).await;
    let mut session = session_arc.lock().await;
    if session.closed {
        return;
    }
    let handle = tokio::spawn(async move {
        if run_stop_hook {
            let project_dir = get_project_dir_string(app.clone()).await;
            let payload = HookPayload::new(&HookEvent::Stop, chat_id.clone(), project_dir);
            run_observation_hooks(app.clone(), HookEvent::Stop, payload).await;
        }

        let mut event = make_runtime_event(
            "chat_completed",
            &format!("Completed: {}", chat_label),
            "chat",
            &format!("chat_{}", chat_id),
            "completed",
            None,
        );
        event.chat_id = Some(chat_id);
        app.buddy_event_sink
            .apply_chat_completion(event, 4, "happy".to_string())
            .await;
        maybe_enqueue_chat_activity_reaction(app, activity).await;
    });
    session.track_post_turn_task(handle);
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
    if normalize_stop_reason(finish_reason) != Some(NormalizedStopReason::ProviderLengthStop) {
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

fn length_like_finish_reason(finish_reason: Option<&str>) -> Option<NormalizedStopReason> {
    finish_reason.and_then(normalize_stop_reason)
}

fn has_empty_or_near_empty_visible_output(message: &ChatMessage) -> bool {
    message.content.content_text_only().trim().chars().count()
        <= LENGTH_STOP_NEAR_EMPTY_VISIBLE_CHARS
}

pub(crate) fn is_high_pressure_length_stop(
    message: &ChatMessage,
    messages: &[ChatMessage],
    effective_n_ctx: usize,
    usage_stale: bool,
    image_token_mode: refact_core::provider_types::ImageTokenMode,
) -> bool {
    if message.role != "assistant"
        || message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
        || length_like_finish_reason(message.finish_reason.as_deref()).is_none()
        || !has_empty_or_near_empty_visible_output(message)
    {
        return false;
    }

    matches!(
        crate::chat::summarization::estimated_provider_context_pressure_with_usage_for_image_mode(
            messages,
            effective_n_ctx,
            usage_stale,
            image_token_mode,
        ),
        ContextPressure::High | ContextPressure::Critical
    )
}

fn is_length_stop_compression_candidate(message: &ChatMessage) -> bool {
    message.role == "assistant"
        && !message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
        && length_like_finish_reason(message.finish_reason.as_deref()).is_some()
        && has_empty_or_near_empty_visible_output(message)
}

fn last_length_stop_message(messages: &[ChatMessage]) -> Option<&ChatMessage> {
    messages.iter().rev().find(|message| {
        !(message.role == "cd_instruction" && message.tool_call_id == "token_budget_info")
    })
}

async fn maybe_compact_after_high_pressure_length_stop(
    gcx: Arc<crate::global_context::GlobalContext>,
    session_arc: &Arc<AMutex<ChatSession>>,
    thread: &ThreadParams,
    effective_n_ctx: Option<usize>,
    image_token_mode: refact_core::provider_types::ImageTokenMode,
) -> bool {
    let Some(effective_n_ctx) = effective_n_ctx else {
        let mut session = session_arc.lock().await;
        if matches!(
            session.runtime.state,
            SessionState::Idle | SessionState::Completed
        ) && last_length_stop_message(&session.messages)
            .is_some_and(is_length_stop_compression_candidate)
        {
            crate::chat::context_rebuild::emit_compression_skipped_status(
                &mut session,
                CompressionReason::EffectiveContextUnknown,
            );
        }
        return false;
    };
    let reason = {
        let session = session_arc.lock().await;
        if !matches!(
            session.runtime.state,
            SessionState::Idle | SessionState::Completed
        ) {
            return false;
        }
        let Some(message) = last_length_stop_message(&session.messages) else {
            return false;
        };
        if !is_high_pressure_length_stop(
            message,
            &session.messages,
            effective_n_ctx,
            session.provider_usage_stale,
            image_token_mode,
        ) {
            return false;
        }
        length_like_finish_reason(message.finish_reason.as_deref())
            .map(compression_reason_for_stop_reason)
    };

    warn!("High-pressure length stop, rebuilding active context");
    let outcome = crate::chat::context_rebuild::apply_context_rebuild_with_reason(
        gcx,
        session_arc,
        thread,
        true,
        reason,
    )
    .await;
    if outcome.applied() {
        let mut session = session_arc.lock().await;
        session.thread.previous_response_id = None;
        session.reset_cache_guard_snapshot();
        return true;
    }

    false
}

const LENGTH_STOP_CONTINUE_MARKER: &str = "length_stop_continue";
const MAX_LENGTH_STOP_RECOVERY_ATTEMPTS: usize = 2;
const LENGTH_STOP_BOOSTED_MAX_NEW_TOKENS: usize = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LengthStopKind {
    EmptyOutput,
    PartialOutput,
}

fn length_stop_kind(message: &ChatMessage) -> Option<LengthStopKind> {
    if message.role != "assistant"
        || message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
        || length_like_finish_reason(message.finish_reason.as_deref())
            != Some(NormalizedStopReason::ProviderLengthStop)
    {
        return None;
    }
    if has_empty_or_near_empty_visible_output(message) {
        Some(LengthStopKind::EmptyOutput)
    } else {
        Some(LengthStopKind::PartialOutput)
    }
}

fn length_stop_recovery_attempts(messages: &[ChatMessage]) -> usize {
    let start = messages
        .iter()
        .rposition(|message| message.role == "user")
        .map_or(0, |idx| idx + 1);
    messages[start..]
        .iter()
        .filter(|message| {
            message.role == "cd_instruction" && message.tool_call_id == LENGTH_STOP_CONTINUE_MARKER
        })
        .count()
}

pub(crate) fn length_stop_continue_instruction(kind: LengthStopKind) -> ChatMessage {
    let text = match kind {
        LengthStopKind::EmptyOutput => {
            " The previous response was cut off by the output token limit before any visible text or complete tool call was produced. Respond again, keep internal reasoning brief, and produce the answer directly. If a tool payload is large, split it into several smaller calls."
        }
        LengthStopKind::PartialOutput => {
            " The previous message was cut off by the output token limit. Continue exactly where it stopped; do not repeat content that was already produced."
        }
    };
    ChatMessage {
        role: "cd_instruction".to_string(),
        tool_call_id: LENGTH_STOP_CONTINUE_MARKER.to_string(),
        content: ChatContent::SimpleText(text.to_string()),
        ..Default::default()
    }
}

pub(crate) async fn maybe_recover_after_length_stop(
    gcx: Arc<crate::global_context::GlobalContext>,
    session_arc: &Arc<AMutex<ChatSession>>,
    thread: &ThreadParams,
    effective_n_ctx: Option<usize>,
    image_token_mode: refact_core::provider_types::ImageTokenMode,
) -> bool {
    let app = AppState::from_gcx(gcx.clone()).await;
    let chat_id = session_arc.lock().await.chat_id.clone();
    if crate::agents::seal::is_sealed(&app, &chat_id).await {
        return false;
    }
    let (kind, dead_end_message_id, attempts) = {
        let session = session_arc.lock().await;
        if !matches!(
            session.runtime.state,
            SessionState::Idle | SessionState::Completed
        ) {
            return false;
        }
        let Some(last) = last_length_stop_message(&session.messages) else {
            return false;
        };
        let Some(kind) = length_stop_kind(last) else {
            return false;
        };
        (
            kind,
            last.message_id.clone(),
            length_stop_recovery_attempts(&session.messages),
        )
    };

    if attempts >= MAX_LENGTH_STOP_RECOVERY_ATTEMPTS {
        let mut session = session_arc.lock().await;
        session.add_message(make_ui_only_error_message(
            "Generation stopped by the output token limit repeatedly; automatic retries exhausted. Send a message to continue.",
        ));
        return false;
    }

    let compacted = maybe_compact_after_high_pressure_length_stop(
        gcx.clone(),
        session_arc,
        thread,
        effective_n_ctx,
        image_token_mode,
    )
    .await;

    if crate::agents::seal::is_sealed(&app, &chat_id).await {
        return false;
    }
    if thread.max_tokens.is_some() && kind == LengthStopKind::PartialOutput {
        if !compacted {
            return false;
        }
        let mut session = session_arc.lock().await;
        session.add_message(length_stop_continue_instruction(kind));
        warn!(
            "Recovering from {:?} length stop after compaction (attempt {}/{})",
            kind,
            attempts + 1,
            MAX_LENGTH_STOP_RECOVERY_ATTEMPTS,
        );
        return true;
    }

    let mut session = session_arc.lock().await;
    if kind == LengthStopKind::EmptyOutput {
        if !dead_end_message_id.is_empty() {
            session.remove_message(&dead_end_message_id);
        } else if let Some(index) = last_length_stop_message(&session.messages)
            .filter(|message| length_stop_kind(message) == Some(LengthStopKind::EmptyOutput))
            .and_then(|last| {
                session
                    .messages
                    .iter()
                    .rposition(|message| std::ptr::eq(message, last))
            })
        {
            session.messages.remove(index);
            session.increment_version();
            session.touch();
        }
    }
    if thread.max_tokens.is_none() {
        session.pending_max_new_tokens_boost = Some(LENGTH_STOP_BOOSTED_MAX_NEW_TOKENS);
    }
    session.add_message(length_stop_continue_instruction(kind));
    warn!(
        "Recovering from {:?} length stop (attempt {}/{}, compacted={})",
        kind,
        attempts + 1,
        MAX_LENGTH_STOP_RECOVERY_ATTEMPTS,
        compacted,
    );
    true
}

fn should_notify_task_agent_reasoning_token_stop(
    message: &ChatMessage,
    messages: &[ChatMessage],
    effective_n_ctx: Option<usize>,
    usage_stale: bool,
    image_token_mode: refact_core::provider_types::ImageTokenMode,
) -> bool {
    if !is_reasoning_token_limit_stop(message) {
        return false;
    }

    !effective_n_ctx.is_some_and(|n_ctx| {
        is_high_pressure_length_stop(message, messages, n_ctx, usage_stale, image_token_mode)
    })
}

async fn handle_task_agent_reasoning_token_stop(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    effective_n_ctx: Option<usize>,
    image_token_mode: refact_core::provider_types::ImageTokenMode,
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
        if !should_notify_task_agent_reasoning_token_stop(
            message,
            &session.messages,
            effective_n_ctx,
            session.provider_usage_stale,
            image_token_mode,
        ) {
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

    maybe_save_trajectory_with_intent(
        app.clone(),
        session_arc.clone(),
        TrajectoryCommitIntent::Checkpoint,
    )
    .await;

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

struct GenerationAttemptContext {
    thread: ThreadParams,
    chat_id: String,
    chat_label: String,
    abort_flag: Arc<AtomicBool>,
}

enum GenerationAttemptStart {
    Continue,
    Stop,
    Generated {
        context: GenerationAttemptContext,
        result: Result<GenerationResult, LlmStreamError>,
    },
}

enum GenerationLoopControl {
    Continue,
    Stop,
}

fn generation_chat_label(thread: &ThreadParams) -> String {
    let title = thread.title.trim();
    if title.is_empty() || title == "New Chat" {
        "Untitled chat".to_string()
    } else {
        title.chars().take(60).collect()
    }
}

async fn start_generation_iteration(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    gcx: &Arc<crate::global_context::GlobalContext>,
) -> GenerationAttemptStart {
    {
        let mut session = session_arc.lock().await;
        if park_pending_verifier_at_generation_boundary(&mut session) {
            return GenerationAttemptStart::Stop;
        }
    }
    let Ok(_operation) = super::session_runtime::begin_board_operation(app, session_arc).await
    else {
        return GenerationAttemptStart::Stop;
    };
    if session_arc
        .lock()
        .await
        .yield_to_queued_commands_at_boundary()
    {
        return GenerationAttemptStart::Stop;
    }
    let chat_id = session_arc.lock().await.chat_id.clone();
    if crate::agents::seal::is_sealed(app, &chat_id).await {
        return GenerationAttemptStart::Stop;
    }
    if let Err(error) =
        crate::tasks::goal::hydrate_session_task_goal(gcx.clone(), session_arc).await
    {
        let mut session = session_arc.lock().await;
        session.set_runtime_state(SessionState::Error, Some(error));
        return GenerationAttemptStart::Stop;
    }
    if session_arc.lock().await.planner_pause_at_boundary() {
        return GenerationAttemptStart::Stop;
    }
    match crate::chat::context_rebuild::drain_pending(gcx.clone(), session_arc).await {
        Ok(true) => return GenerationAttemptStart::Stop,
        Err(error) => {
            let mut session = session_arc.lock().await;
            let detail = format!(
                "Context operation failed: {}",
                safe_provider_error_diagnostic(&error)
            );
            session.append_error_message_deduped(&detail);
            session.set_runtime_state(SessionState::Error, Some(detail));
            return GenerationAttemptStart::Stop;
        }
        Ok(false) => {}
    }
    {
        let mut session = session_arc.lock().await;
        if let Err(error) = refact_core::active_context::active_context(&session.messages) {
            let error = format!(
                "Invalid active context: {error}. Explicit rebuild required before generation"
            );
            session.append_error_message_deduped(&error);
            session.set_runtime_state(SessionState::Error, Some(error));
            return GenerationAttemptStart::Stop;
        }
    }
    if inject_priority_messages_before_llm_if_safe(app.clone(), session_arc.clone()).await {
        return GenerationAttemptStart::Continue;
    }
    // `append` deliveries land here: before the next LLM request, with
    // the previous assistant/tool window closed.
    crate::chat::delivery::drain_deliveries_at_boundary(app.clone(), session_arc.clone()).await;

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

    let chat_label = generation_chat_label(&thread);

    let fork_agent_name = {
        let session = session_arc.lock().await;
        session.active_command.context_fork.clone()
    };

    if let Some(agent_name) = fork_agent_name {
        let user_content_opt = {
            let session = session_arc.lock().await;
            refact_core::active_context::active_context(&session.messages)
                .ok()
                .and_then(|view| {
                    view.messages
                        .into_iter()
                        .rev()
                        .find(|message| message.role == "user")
                        .map(|message| message.content.content_text_only())
                })
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
                return GenerationAttemptStart::Continue;
            }
        };

        let fork_result =
            run_fork_subchat(app.clone(), &agent_name, &user_content, &thread, &chat_id).await;

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
                spawn_completion_activity_reaction(app.clone(), session_arc.clone()).await;
                maybe_save_trajectory_with_intent(
                    app.clone(),
                    session_arc.clone(),
                    TrajectoryCommitIntent::Checkpoint,
                )
                .await;
                return GenerationAttemptStart::Stop;
            }
            Err(e) => {
                warn!(
                    "Fork skill subchat failed ({}), falling back to normal generation",
                    e
                );
                return GenerationAttemptStart::Continue;
            }
        }
    }

    // Resolve before the very first proactive check, with the command's
    // actual model override applied. Failed resolution remains pending.
    if thread.auto_compression_cap_pending {
        if let Ok(caps) =
            crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0).await
        {
            if let Ok(model) = crate::caps::resolve_chat_model(caps, &thread.model) {
                let window = if model.base.n_ctx > 0 {
                    model.base.n_ctx
                } else {
                    tokens().default_n_ctx
                };
                resolve_session_compression_cap(&app, &session_arc, &mut thread, window).await;
            }
        }
    }

    if matches!(
        crate::chat::context_rebuild::apply_context_rebuild_with_reason(
            gcx.clone(),
            &session_arc,
            &thread,
            false,
            None,
        )
        .await,
        crate::chat::context_rebuild::CompactionOutcome::LlmUnavailable
    ) {
        return GenerationAttemptStart::Stop;
    }

    thread = {
        let session = session_arc.lock().await;
        let mut refreshed = session.thread.clone();
        if let Some(model) = session
            .active_command
            .model_override
            .as_ref()
            .filter(|m| !m.is_empty())
        {
            refreshed.model = model.clone();
        }
        refreshed
    };

    // Rebuilds and handoffs replace the conversation but not the preamble.
    ensure_session_preamble(app.clone(), session_arc.clone()).await;

    if user_stop_requested(&session_arc).await {
        return GenerationAttemptStart::Stop;
    }

    let (abort_flag, abort_notify) = {
        let mut session = session_arc.lock().await;
        match session.start_stream() {
            Some((_message_id, abort_flag)) => {
                session.delivery_wake_sources.clear();
                let notify = session.abort_notify.clone();
                (abort_flag, notify)
            }
            None => {
                warn!(
                    "Cannot start generation for {}: already generating",
                    chat_id
                );
                return GenerationAttemptStart::Stop;
            }
        }
    };
    let stream_prepare_started_at = perf_diagnostics::is_enabled().then(Instant::now);

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

    drop(_operation);
    let generation_result = run_llm_generation(
        app.clone(),
        session_arc.clone(),
        thread.clone(),
        chat_id.clone(),
        abort_flag.clone(),
        abort_notify.clone(),
        stream_prepare_started_at,
    )
    .await;
    return GenerationAttemptStart::Generated {
        context: GenerationAttemptContext {
            thread,
            chat_id,
            chat_label,
            abort_flag,
        },
        result: generation_result,
    };
}

pub(crate) fn park_pending_verifier_at_generation_boundary(session: &mut ChatSession) -> bool {
    if session.verifier_readiness_pending.is_none() {
        return false;
    }
    session.set_runtime_state(SessionState::Idle, None);
    true
}

async fn handle_generation_error(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    gcx: &Arc<crate::global_context::GlobalContext>,
    context: &GenerationAttemptContext,
    error: LlmStreamError,
) -> GenerationLoopControl {
    let _ = gcx;
    let retry = super::recovery::handle_failure(app, session_arc, &error).await;
    if retry {
        return GenerationLoopControl::Continue;
    }
    let Some(Ok(_operation)) = super::recovery::within_budget(
        session_arc,
        super::session_runtime::begin_board_operation(app, session_arc),
    )
    .await
    else {
        return GenerationLoopControl::Stop;
    };
    let task_meta = {
        let mut session = session_arc.lock().await;
        if session.abort_flag.load(Ordering::SeqCst) {
            return GenerationLoopControl::Stop;
        }
        session.task_agent_error = Some(safe_provider_error_diagnostic(&error.message));
        session.thread.task_meta.clone()
    };
    app.buddy_event_sink
        .report_error_with_model(
            "llm_error",
            &safe_provider_error_diagnostic(&error.message),
            Some("chat/generation.rs"),
            Some(&context.chat_id),
            Some(&context.thread.model),
        )
        .await;
    if let Some(task_meta) = task_meta {
        super::task_agent_monitor::handle_agent_streaming_error(
            app.clone(),
            &task_meta,
            &safe_provider_error_diagnostic(&error.message),
        )
        .await;
    }
    GenerationLoopControl::Stop
}

async fn generation_context_budget(
    gcx: &Arc<crate::global_context::GlobalContext>,
    model_id: &str,
    context_tokens_cap: Option<usize>,
) -> (Option<usize>, refact_core::provider_types::ImageTokenMode) {
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0).await;
    let model_rec = match caps {
        Ok(caps) => crate::caps::resolve_chat_model(caps, model_id).ok(),
        Err(_) => None,
    };
    let image_token_mode = model_rec
        .as_ref()
        .map(|rec| rec.base.image_token_mode)
        .unwrap_or_default();
    let effective_n_ctx = model_rec.map(|rec| {
        let model_n_ctx = if rec.base.n_ctx > 0 {
            rec.base.n_ctx
        } else {
            tokens().default_n_ctx
        };
        match context_tokens_cap {
            Some(cap) if cap > 0 => cap.min(model_n_ctx),
            _ => model_n_ctx,
        }
    });
    (effective_n_ctx, image_token_mode)
}

async fn process_successful_generation(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    gcx: &Arc<crate::global_context::GlobalContext>,
    context: &GenerationAttemptContext,
) -> GenerationLoopControl {
    let GenerationAttemptContext {
        chat_id,
        abort_flag,
        ..
    } = context;

    if crate::agents::seal::is_sealed(app, chat_id).await {
        return GenerationLoopControl::Stop;
    }
    {
        let mut session = session_arc.lock().await;
        session.provider_usage_stale = false;
        if session.thread.reactive_compact_attempts.take().is_some() {
            session.increment_version();
            session.touch();
        }
    }

    if abort_flag.load(Ordering::SeqCst) {
        return GenerationLoopControl::Stop;
    }

    let Ok(operation) = super::session_runtime::begin_board_operation(app, session_arc).await
    else {
        return GenerationLoopControl::Stop;
    };
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

    let (effective_n_ctx, image_token_mode) =
        generation_context_budget(gcx, &model_id, context_tokens_cap).await;

    maybe_save_trajectory_background_with_intent(
        app.clone(),
        session_arc.clone(),
        TrajectoryCommitIntent::Checkpoint,
    );

    drop(operation);
    let outcome =
        process_tool_calls_once(app.clone(), session_arc.clone(), &mode_id, model_id_opt).await;
    handle_tool_step_outcome(
        app,
        session_arc,
        context,
        outcome,
        effective_n_ctx,
        image_token_mode,
    )
    .await
}

async fn handle_tool_step_outcome(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    context: &GenerationAttemptContext,
    outcome: ToolStepOutcome,
    effective_n_ctx: Option<usize>,
    image_token_mode: refact_core::provider_types::ImageTokenMode,
) -> GenerationLoopControl {
    let GenerationAttemptContext {
        thread,
        chat_id,
        chat_label,
        ..
    } = context;
    if matches!(
        outcome,
        ToolStepOutcome::NoToolCalls | ToolStepOutcome::Continue
    ) && session_arc
        .lock()
        .await
        .yield_to_queued_commands_at_boundary()
    {
        return GenerationLoopControl::Stop;
    }
    match outcome {
        ToolStepOutcome::NoToolCalls => {
            let Ok(_operation) =
                super::session_runtime::begin_board_operation(app, session_arc).await
            else {
                return GenerationLoopControl::Stop;
            };
            if handle_task_agent_reasoning_token_stop(
                app.clone(),
                session_arc.clone(),
                effective_n_ctx,
                image_token_mode,
            )
            .await
            {
                return GenerationLoopControl::Stop;
            }
            if maybe_recover_after_length_stop(
                app.gcx.clone(),
                &session_arc,
                &thread,
                effective_n_ctx,
                image_token_mode,
            )
            .await
            {
                maybe_save_trajectory_with_intent(
                    app.clone(),
                    session_arc.clone(),
                    TrajectoryCommitIntent::Checkpoint,
                )
                .await;
                return GenerationLoopControl::Continue;
            }
            if inject_priority_messages_before_llm_if_safe(app.clone(), session_arc.clone()).await {
                return GenerationLoopControl::Continue;
            }
            let should_continue = {
                let session = session_arc.lock().await;
                tail_needs_assistant(&session.messages)
            };
            if should_continue {
                return GenerationLoopControl::Continue;
            }
            if let Err(error) = maybe_record_goal_pursuit_progress(app, session_arc.clone()).await {
                session_arc
                    .lock()
                    .await
                    .set_runtime_state(SessionState::Error, Some(error));
                return GenerationLoopControl::Stop;
            }
            if handle_goal_turn_end(app.clone(), session_arc.clone()).await {
                return GenerationLoopControl::Stop;
            }
            spawn_chat_completion_side_effects(
                app.clone(),
                session_arc.clone(),
                chat_id.clone(),
                chat_label.clone(),
                true,
            )
            .await;
            return GenerationLoopControl::Stop;
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
            let origin = {
                let session = session_arc.lock().await;
                session.thread.worktree.as_ref().map(|worktree| {
                    (
                        worktree.source_workspace_root.clone(),
                        session.created_at.clone(),
                        session.event_seq,
                    )
                })
            };
            let mut correlated = false;
            if let Some((root, version, sequence)) = origin {
                if let Ok(root) = tokio::fs::canonicalize(root).await {
                    use refact_buddy_core::operator_events::{
                        OperatorSignal, OperatorSignalAuthority, OperatorSignalKind,
                        OPERATOR_SIGNAL_SCHEMA_VERSION_V1,
                    };
                    use refact_buddy_core::operator_types::{
                        NativeSourceIdentity, NativeSourceKind, ProjectIdentity, ResourceRef,
                    };
                    let canonical_root = root.to_string_lossy().into_owned();
                    let signal = OperatorSignal {
                        schema_version: OPERATOR_SIGNAL_SCHEMA_VERSION_V1,
                        kind: OperatorSignalKind::OperationState,
                        authority: OperatorSignalAuthority::Hint,
                        project: ProjectIdentity {
                            project_id: canonical_root.clone(),
                            canonical_root,
                        },
                        worker: None,
                        source: NativeSourceIdentity {
                            source_kind: NativeSourceKind::Chat,
                            source_id: chat_id.to_string(),
                            source_version: version.clone(),
                        },
                        source_sequence: Some(sequence),
                        chat: Some(ResourceRef {
                            resource_kind: "chat".into(),
                            resource_id: chat_id.to_string(),
                            version,
                        }),
                        tool: None,
                        goal: None,
                        runtime_event: ev.clone(),
                    };
                    correlated = app
                        .buddy_event_sink
                        .enqueue_operator_signal(signal)
                        .await
                        .is_ok();
                }
            }
            if !correlated {
                app.buddy_event_sink.enqueue_event(ev).await;
            }
            return GenerationLoopControl::Stop;
        }
        ToolStepOutcome::Stop => {
            let completed = {
                let session = session_arc.lock().await;
                session.runtime.state == SessionState::Completed
            };
            if completed {
                spawn_chat_completion_side_effects(
                    app.clone(),
                    session_arc.clone(),
                    chat_id.clone(),
                    chat_label.clone(),
                    false,
                )
                .await;
            }
            return GenerationLoopControl::Stop;
        }
        ToolStepOutcome::Continue => {
            if inject_priority_messages_before_llm_if_safe(app.clone(), session_arc.clone()).await {
                return GenerationLoopControl::Continue;
            }
        }
    }

    GenerationLoopControl::Continue
}

fn review_continuation_is_current(
    session: &ChatSession,
    pending: &ReviewReadinessContinuation,
) -> bool {
    !session.closed
        && pending.receipt.binding.chat_id == session.chat_id
        && !pending.cancelled.load(Ordering::SeqCst)
        && !session.user_interrupt_flag.load(Ordering::SeqCst)
        && session.runtime.interruption.is_none()
        && session.trajectory_version == pending.trajectory_version
        && serde_json::to_value(&session.thread).ok().as_ref() == Some(&pending.thread_fingerprint)
}

pub(crate) async fn take_ready_review_continuation(session_arc: &Arc<AMutex<ChatSession>>) -> bool {
    let mut session = session_arc.lock().await;
    let Some(pending) = session.review_readiness_pending.as_ref() else {
        return false;
    };
    if !review_continuation_is_current(&session, pending) {
        session.invalidate_review_readiness();
        return false;
    }
    if !pending.ready
        || !session.command_queue.is_empty()
        || session.turn_depth != 0
        || session.runtime.state != SessionState::Idle
        || session.planner_paused()
        || session.runtime.paused
        || !session.runtime.pause_reasons.is_empty()
        || session.runner_lease.is_some()
    {
        return false;
    }
    let pending = session
        .review_readiness_pending
        .take()
        .expect("checked pending");
    session.active_command = pending.command;
    session.review_readiness_resume_nonce = Some(pending.turn_nonce);
    true
}

async fn park_review_readiness(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    receipt: crate::tool_jobs::JobReceipt,
    turn_nonce: String,
) {
    let (chat_id, cancelled) = {
        let mut session = session_arc.lock().await;
        session.invalidate_review_readiness();
        if session.closed
            || session.user_interrupt_flag.load(Ordering::SeqCst)
            || !session.command_queue.is_empty()
        {
            session.finish_stream_with_next_state(None, SessionState::Idle);
            return;
        }
        let command = session.active_command.clone();
        // No assistant was generated. Close its empty draft without an error,
        // tool result, or a fabricated successful completion.
        session.finish_stream_with_next_state(None, SessionState::Idle);
        let cancelled = Arc::new(AtomicBool::new(false));
        session.review_readiness_pending = Some(ReviewReadinessContinuation {
            turn_nonce: turn_nonce.clone(),
            receipt: receipt.clone(),
            command,
            trajectory_version: session.trajectory_version,
            thread_fingerprint: serde_json::to_value(&session.thread).unwrap_or_default(),
            cancelled: cancelled.clone(),
            ready: false,
        });
        session.emit(ChatEvent::ThreadUpdated {
            params: json!({"review_readiness": {
                "status": "pending", "receipt": receipt, "turn_nonce": turn_nonce,
            }}),
        });
        (session.chat_id.clone(), cancelled)
    };
    // Detached observation owns no execution permit or transition lock. The
    // queue, not this observer, owns the eventual generation continuation.
    tokio::spawn(async move {
        let result =
            crate::review::readiness::wait_for_readiness(&app, &chat_id, &receipt, &turn_nonce)
                .await;
        if app.runtime.shutdown_flag.load(Ordering::Relaxed) || cancelled.load(Ordering::SeqCst) {
            return;
        }
        let mut session = session_arc.lock().await;
        let Some(pending) = session.review_readiness_pending.as_ref() else {
            return;
        };
        if pending.turn_nonce != turn_nonce {
            return;
        }
        if !review_continuation_is_current(&session, pending) {
            clear_stale_review_observer(&mut session, &turn_nonce);
        } else if result.is_ok() {
            session.review_readiness_pending.as_mut().unwrap().ready = true;
            session.emit(ChatEvent::ThreadUpdated {
                params: json!({"review_readiness": {
                    "status": "ready", "turn_nonce": turn_nonce,
                }}),
            });
            session.queue_notify.notify_one();
        } else {
            session.invalidate_review_readiness();
            session.emit(ChatEvent::ThreadUpdated {
                params: json!({"review_readiness": {
                    "status": "unavailable", "turn_nonce": turn_nonce,
                }}),
            });
        }
    });
}

fn clear_stale_review_observer(session: &mut ChatSession, nonce: &str) -> bool {
    if !session
        .review_readiness_pending
        .as_ref()
        .is_some_and(|pending| pending.turn_nonce == nonce)
    {
        return false;
    }
    let pending = session.review_readiness_pending.take().unwrap();
    pending.cancelled.store(true, Ordering::SeqCst);
    session.review_readiness_resume_nonce = None;
    session.emit(ChatEvent::ThreadUpdated {
        params: json!({"review_readiness": {"status": "unavailable", "turn_nonce": nonce}}),
    });
    session.queue_notify.notify_one();
    true
}

pub fn start_generation(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let gcx = app.gcx.clone();
        if app.runtime.shutdown_flag.load(Ordering::Relaxed) {
            session_arc.lock().await.invalidate_review_readiness();
            return;
        }
        {
            let session = session_arc.lock().await;
            if session.runner_lease.is_some()
                || session.review_readiness_pending.is_some()
                || session.verifier_readiness_pending.is_some()
            {
                return;
            }
        }
        let preparation = async {
            let board_execution = super::session_runtime::board_execution(&app, &session_arc).await;
            let board_execution = match board_execution {
                Ok(execution) => execution,
                Err(reason) => {
                    super::session_runtime::interrupt_board_execution(
                        &mut *session_arc.lock().await,
                        reason,
                    );
                    return None;
                }
            };
            if board_execution.is_some() {
                let session = session_arc.lock().await;
                if session.planner_paused()
                    || session.runtime.paused
                    || !session.runtime.pause_reasons.is_empty()
                {
                    return None;
                }
            }
            let turn_guard = {
                let _scope_transition =
                    super::worktree_lifecycle::transition_lock(&session_arc).await;
                let (chat_id, thread) = {
                    let session = session_arc.lock().await;
                    (session.chat_id.clone(), session.thread.clone())
                };
                if let Err(error) =
                    super::worktree_lifecycle::check_managed_attachment(&app, &chat_id, &thread)
                        .await
                {
                    let mut session = session_arc.lock().await;
                    session.append_error_message_deduped(&error);
                    if !session.planner_paused()
                        && !session.runtime.paused
                        && session.runtime.pause_reasons.is_empty()
                    {
                        session.set_runtime_state(SessionState::Error, Some(error));
                    }
                    return None;
                }
                if let Err(error) = super::recovery::install_budget(&session_arc).await {
                    session_arc
                        .lock()
                        .await
                        .set_runtime_state(SessionState::Error, Some(error));
                    return None;
                }
                TurnDepthGuard::enter(app.clone(), session_arc.clone()).await?
            };
            Some(turn_guard)
        };
        let Some(Some(mut turn_guard)) =
            super::recovery::within_budget(&session_arc, preparation).await
        else {
            maybe_save_trajectory_background_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Checkpoint,
            );
            return;
        };
        loop {
            let Some(Ok(operation)) = super::recovery::within_budget(
                &session_arc,
                super::session_runtime::begin_board_operation(&app, &session_arc),
            )
            .await
            else {
                break;
            };
            let resume_tools = {
                let mut session = session_arc.lock().await;
                if session.planner_paused() {
                    false
                } else {
                    std::mem::take(&mut session.planner_pause_pending_tools)
                }
            };
            if resume_tools {
                let chat_id = session_arc.lock().await.chat_id.clone();
                if crate::agents::seal::is_sealed(&app, &chat_id).await {
                    break;
                }
                let context = {
                    let session = session_arc.lock().await;
                    GenerationAttemptContext {
                        thread: session.thread.clone(),
                        chat_id,
                        chat_label: generation_chat_label(&session.thread),
                        abort_flag: session.abort_flag.clone(),
                    }
                };
                let (effective_n_ctx, image_token_mode) = generation_context_budget(
                    &gcx,
                    &context.thread.model,
                    context.thread.context_tokens_cap,
                )
                .await;
                drop(operation);
                let outcome = process_tool_calls_once(
                    app.clone(),
                    session_arc.clone(),
                    &context.thread.mode,
                    Some(&context.thread.model),
                )
                .await;
                match handle_tool_step_outcome(
                    &app,
                    &session_arc,
                    &context,
                    outcome,
                    effective_n_ctx,
                    image_token_mode,
                )
                .await
                {
                    GenerationLoopControl::Stop => break,
                    GenerationLoopControl::Continue => {}
                }
            } else {
                drop(operation);
            }
            if let Err(error) = super::recovery::install_budget(&session_arc).await {
                session_arc
                    .lock()
                    .await
                    .set_runtime_state(SessionState::Error, Some(error));
                break;
            }
            match crate::review::turn_completion::stop_at_boundary(&app, &session_arc).await {
                Ok(true) => break,
                Ok(false) => {}
                Err(error) => tracing::warn!(%error, "proposal completion boundary unavailable"),
            }
            let Some(attempt) = super::recovery::within_budget(
                &session_arc,
                start_generation_iteration(&app, &session_arc, &gcx),
            )
            .await
            else {
                break;
            };
            let (context, generation_result) = match attempt {
                GenerationAttemptStart::Continue => continue,
                GenerationAttemptStart::Stop => break,
                GenerationAttemptStart::Generated {
                    context,
                    result,
                } => (context, result),
            };

            if matches!(
                &generation_result,
                Ok(GenerationResult::PausedForUserDecision | GenerationResult::PendingReadiness)
            ) {
                maybe_save_trajectory_with_intent(
                    app.clone(),
                    session_arc.clone(),
                    TrajectoryCommitIntent::Checkpoint,
                )
                .await;
                break;
            }

            if let Err(error) = generation_result {
                match handle_generation_error(&app, &session_arc, &gcx, &context, error).await {
                    GenerationLoopControl::Continue => continue,
                    GenerationLoopControl::Stop => break,
                }
            }

            {
                let mut session = session_arc.lock().await;
                if session.abort_flag.load(Ordering::SeqCst) {
                    break;
                }
                super::recovery::succeeded(&mut session);
            }
            match process_successful_generation(&app, &session_arc, &gcx, &context).await {
                GenerationLoopControl::Continue => continue,
                GenerationLoopControl::Stop => break,
            }
        }

        {
            let mut session = session_arc.lock().await;
            session.recovery_budget = None;
        }
        maybe_save_trajectory_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        )
        .await;
        check_external_reload_pending(gcx.clone(), session_arc.clone()).await;

        {
            let session = session_arc.lock().await;
            session.abort_flag.store(false, Ordering::SeqCst);
            session.user_interrupt_flag.store(false, Ordering::SeqCst);
            session.queue_notify.notify_one();
        }

        turn_guard.release().await;
        crate::chat::delivery::drain_deliveries_at_boundary(app.clone(), session_arc.clone()).await;
    })
}

/// Tracks how many generation turns are in flight for a session, so
/// `when_idle` deliveries wait for the real end of the turn instead of an
/// intermediate `Idle` between tool steps.
struct TurnDepthGuard {
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    released: bool,
}

impl TurnDepthGuard {
    async fn enter(app: AppState, session_arc: Arc<AMutex<ChatSession>>) -> Option<Self> {
        {
            let mut session = session_arc.lock().await;
            if session.runner_lease.is_some() {
                return None;
            }
            if session.turn_depth == 0 {
                session.begin_execution_origin();
            }
            session.turn_depth = session.turn_depth.saturating_add(1);
        }
        Some(Self {
            app,
            session_arc,
            released: false,
        })
    }

    /// Release synchronously so a following drain observes the finished turn.
    async fn release(&mut self) {
        if self.released {
            return;
        }
        let mut session = self.session_arc.lock().await;
        session.turn_depth = session.turn_depth.saturating_sub(1);
        if session.turn_depth == 0 && session.queued_user_restart_target.is_none() {
            session.live_diagnostics.clear_accepted();
        }
        session.queue_notify.notify_one();
        self.released = true;
        let chat_id = session.chat_id.clone();
        drop(session);
        crate::tasks::admission::synchronize(&self.app, &chat_id).await;
    }
}

impl Drop for TurnDepthGuard {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        // Fallback for early returns/cancellation: never leave the turn pinned.
        let session_arc = self.session_arc.clone();
        let app = self.app.clone();
        tokio::spawn(async move {
            let mut session = session_arc.lock().await;
            session.turn_depth = session.turn_depth.saturating_sub(1);
            if session.turn_depth == 0 && session.queued_user_restart_target.is_none() {
                session.live_diagnostics.clear_accepted();
            }
            if matches!(
                session.runtime.state,
                SessionState::Generating | SessionState::ExecutingTools
            ) {
                session.abort_stream();
                session.clear_pending_tool_calls_for_interruption();
            }
            session.drain_post_tool_side_effects();
            session.queue_notify.notify_one();
            drop(session);
            crate::chat::delivery::drain_deliveries_at_boundary(app, session_arc).await;
        });
    }
}

/// Persist the new-chat default at the model-resolution boundary, not on load or
/// based on an empty history. Shared by the proactive gate and direct generation.
async fn resolve_session_compression_cap(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    thread: &mut ThreadParams,
    model_window: usize,
) {
    let changed = {
        let mut session = session_arc.lock().await;
        let changed = session
            .thread
            .resolve_pending_compression_cap(Some(model_window));
        thread.auto_compression_cap = session.thread.auto_compression_cap;
        thread.auto_compression_cap_pending = session.thread.auto_compression_cap_pending;
        if changed {
            let params = json!({
                "auto_compression_cap": session.thread.auto_compression_cap,
                "auto_compression_cap_pending": session.thread.auto_compression_cap_pending,
            });
            session.emit(ChatEvent::ThreadUpdated {
                params,
            });
            session.increment_version();
            session.touch();
        }
        changed
    };
    if changed {
        maybe_save_trajectory_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        )
        .await;
    }
}

pub async fn run_llm_generation(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    mut thread: ThreadParams,
    chat_id: String,
    abort_flag: Arc<AtomicBool>,
    abort_notify: Arc<tokio::sync::Notify>,
    stream_prepare_started_at: Option<Instant>,
) -> Result<GenerationResult, LlmStreamError> {
    let Ok(_board_operation) =
        super::session_runtime::begin_board_operation(&app, &session_arc).await
    else {
        return Ok(GenerationResult::PausedForUserDecision);
    };
    let gcx = app.gcx.clone();
    crate::tasks::goal::hydrate_session_task_goal(gcx.clone(), &session_arc).await?;
    check_aborted_before_stream(&abort_flag)?;
    let expected_board_execution = _board_operation
        .as_ref()
        .map(|operation| operation.execution.clone());
    let execution = app.agent_executions.get(&chat_id);
    let _operation = if _board_operation.is_some() {
        None
    } else if let Some(execution) = &execution {
        let acquisition = execution.begin_operation();
        tokio::pin!(acquisition);
        loop {
            if abort_flag.load(Ordering::SeqCst)
                || app.runtime.shutdown_flag.load(Ordering::Relaxed)
            {
                execution.cancel();
                return Err("Aborted while waiting for execution capacity"
                    .to_string()
                    .into());
            }
            tokio::select! {
                result = &mut acquisition => break Some(result.map_err(|e| e.to_string())?),
                _ = tokio::time::sleep(refact_work_core::coordination_settings().lock_poll_interval()) => {}
            }
        }
    } else {
        None
    };
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.message)?;
    let model_rec = crate::caps::resolve_chat_model(caps.clone(), &thread.model)?;
    let model_n_ctx = if model_rec.base.n_ctx > 0 {
        model_rec.base.n_ctx
    } else {
        tokens().default_n_ctx
    };
    resolve_session_compression_cap(&app, &session_arc, &mut thread, model_n_ctx).await;
    check_aborted_before_stream(&abort_flag)?;

    let execution_scope = thread
        .worktree
        .as_ref()
        .map(|worktree| worktree.root.to_string_lossy().into_owned());
    let intent = tool_catalog_intent_for_thread(&app, &thread).await;
    let existing_catalog = {
        let session = session_arc.lock().await;
        session
            .tool_catalog
            .clone()
            .filter(|catalog| catalog.intent == intent)
    };
    let catalog = match existing_catalog {
        Some(catalog) => catalog,
        None => {
            let catalog = app
                .tool_registry
                .acquire_tool_catalog_with_intent(
                    &thread.mode,
                    Some(&model_rec.base.id),
                    execution_scope.as_deref(),
                    intent,
                )
                .await;
            crate::tools::tools_list::filter_tool_catalog_for_chat(&app, &thread.id, catalog).await
        }
    };
    let mcp_lazy_active = catalog.index.mcp_lazy_mode;
    let tools = catalog.index.tools.clone();

    info!(
        "session generation: model={}, tools count = {} (mcp_lazy={})",
        model_rec.base.id,
        tools.len(),
        mcp_lazy_active
    );

    let (messages, existing_frozen_prefix) = {
        let mut session = session_arc.lock().await;
        let mut messages = refact_core::active_context::active_context(&session.messages)
            .map_err(|error| error.to_string())?
            .messages;
        if session.task_goal_messages.is_some() {
            let controls = session.try_accepted_control_projection()?.messages;
            let before = messages.len();
            super::linearize::append_task_goal_context(&mut messages, controls);
            for control in messages[before..].iter().cloned() {
                session.messages.push(control);
                session.increment_version();
                session.touch();
            }
        }
        (messages, session.thread.frozen_request_prefix.clone())
    };
    let canonical_tools = build_canonical_openai_tools_from_aliases(
        &tools,
        catalog.aliases.clone(),
        model_rec.supports_strict_tools,
        model_rec.supports_tools,
    );
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
        maybe_save_trajectory_background_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        );
    }

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
        request_attempt_id: session_arc
            .lock()
            .await
            .review_readiness_resume_nonce
            .take()
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        worktree: thread.worktree.clone(),
    };

    let model_type_defaults = model_type_defaults_for_thread(
        &caps.user_defaults,
        &caps.defaults,
        &thread,
        &model_rec.base.id,
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

    {
        let mut session = session_arc.lock().await;
        if let Some(boost) = session.pending_max_new_tokens_boost.take() {
            if parameters.max_new_tokens > 0 && parameters.max_new_tokens < boost {
                parameters.max_new_tokens = boost;
            }
        }
    }

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
    let prepared = prepare_chat_passthrough(
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
    let mut prepared = match prepared {
        PrepareOutcome::Ready(prepared) => prepared,
        PrepareOutcome::Pending {
            receipt,
            turn_nonce,
        } => {
            // Release all admission/transition guards before starting the observer.
            drop(_operation);
            drop(_board_operation);
            park_review_readiness(app, session_arc, receipt, turn_nonce).await;
            return Ok(GenerationResult::PendingReadiness);
        }
    };

    let claude_code_identity = ensure_claude_code_identity(&session_arc, &model_rec.base).await;
    prepared.llm_request = prepared
        .llm_request
        .with_claude_code_identity(claude_code_identity);

    {
        let mut session = session_arc.lock().await;
        session.last_prompt_messages = prepared.limited_messages.clone();
        session.tool_catalog = Some(catalog);
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
        stream_prepare_started_at,
        expected_board_execution,
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
    PendingReadiness,
}

enum StreamCollectorEventPayload {
    Activity,
    DeltaOps(Vec<DeltaOp>),
    Usage(ChatUsage),
}

struct SessionCollector {
    tx: tokio::sync::mpsc::Sender<StreamCollectorEventPayload>,
    overflow_usage: Arc<std::sync::Mutex<Option<ChatUsage>>>,
    overflow_ops: Arc<std::sync::Mutex<Vec<DeltaOp>>>,
    activity_pending: Arc<AtomicBool>,
}

impl StreamCollector for SessionCollector {
    fn on_activity(&mut self) {
        if !self.activity_pending.swap(true, Ordering::SeqCst) {
            let _ = self.tx.try_send(StreamCollectorEventPayload::Activity);
        }
    }

    fn on_delta_ops(&mut self, _choice_idx: usize, ops: Vec<DeltaOp>) {
        match self.tx.try_send(StreamCollectorEventPayload::DeltaOps(ops)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(event)) => {
                if let StreamCollectorEventPayload::DeltaOps(ops) = event {
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
            .try_send(StreamCollectorEventPayload::Usage(usage_clone.clone()))
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

fn spawn_stream_delta_emitter(
    mut rx: tokio::sync::mpsc::Receiver<StreamCollectorEventPayload>,
    overflow_ops: Arc<std::sync::Mutex<Vec<DeltaOp>>>,
    overflow_usage: Arc<std::sync::Mutex<Option<ChatUsage>>>,
    activity_pending: Arc<AtomicBool>,
    session_arc_emitter: Arc<AMutex<ChatSession>>,
    first_content_started_at: Option<Instant>,
    emitter_chat_id: String,
    expected_message_id: Option<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        fn merge_events(
            events: &mut Vec<StreamCollectorEventPayload>,
            batched_ops: &mut Vec<DeltaOp>,
            latest_usage: &mut Option<ChatUsage>,
        ) {
            for event in events.drain(..) {
                match event {
                    StreamCollectorEventPayload::Activity => {}
                    StreamCollectorEventPayload::DeltaOps(ops) => {
                        batched_ops.extend(ops);
                    }
                    StreamCollectorEventPayload::Usage(usage) => {
                        *latest_usage = Some(usage);
                    }
                }
            }
        }
        let mut pending = Vec::<StreamCollectorEventPayload>::new();
        let mut first_visible_delta_emitted = false;
        let mut first_content_delta_recorded = false;

        while let Some(first_event) = rx.recv().await {
            let flush_immediately = match &first_event {
                StreamCollectorEventPayload::Activity => true,
                StreamCollectorEventPayload::Usage(_) => true,
                StreamCollectorEventPayload::DeltaOps(ops) => {
                    !first_visible_delta_emitted && delta_ops_contain_visible_content(ops)
                        || ops.iter().any(|op| !delta_is_coalescible_text(op))
                }
            };
            pending.push(first_event);

            if !flush_immediately {
                let deadline = tokio::time::Instant::now() + DELTA_COALESCE_WINDOW;
                while pending.len() < MAX_DELTA_BATCH_EVENTS {
                    match tokio::time::timeout_at(deadline, rx.recv()).await {
                        Ok(Some(event)) => {
                            let must_flush = match &event {
                                StreamCollectorEventPayload::Activity => true,
                                StreamCollectorEventPayload::Usage(_) => true,
                                StreamCollectorEventPayload::DeltaOps(ops) => {
                                    (!first_visible_delta_emitted
                                        && delta_ops_contain_visible_content(ops))
                                        || ops.iter().any(|op| !delta_is_coalescible_text(op))
                                }
                            };
                            pending.push(event);
                            if must_flush {
                                break;
                            }
                        }
                        Ok(None) | Err(_) => break,
                    }
                }
            }

            let mut batched_ops = Vec::new();
            let mut latest_usage: Option<ChatUsage> = None;
            merge_events(&mut pending, &mut batched_ops, &mut latest_usage);

            if let Ok(mut guard) = overflow_ops.lock() {
                if !guard.is_empty() {
                    let mut drained = std::mem::take(&mut *guard);
                    batched_ops.append(&mut drained);
                }
            }
            if let Ok(mut guard) = overflow_usage.lock() {
                if let Some(usage) = guard.take() {
                    latest_usage = Some(usage);
                }
            }

            let batched_ops = batch_stream_delta_ops(batched_ops);

            let mut session = session_arc_emitter.lock().await;
            if session.abort_flag.load(Ordering::SeqCst)
                || session
                    .recovery_budget
                    .as_ref()
                    .is_some_and(|budget| budget.check().is_err())
                || session.draft_message.as_ref().map(|m| &m.message_id)
                    != expected_message_id.as_ref()
            {
                return;
            }
            if activity_pending.swap(false, Ordering::SeqCst) {
                session.mark_stream_activity();
            }
            if !batched_ops.is_empty() {
                for ops in batched_ops {
                    let carries_content = delta_ops_contain_assistant_content(&ops);
                    first_visible_delta_emitted |= delta_ops_contain_visible_content(&ops);
                    session.emit_stream_delta(ops);
                    if carries_content && !first_content_delta_recorded {
                        if let Some(started_at) = first_content_started_at {
                            perf_diagnostics::record(
                                PerfComponent::StreamFirstContentDelta,
                                Some(&emitter_chat_id),
                                PerfOutcome::Success,
                                started_at
                                    .elapsed()
                                    .as_micros()
                                    .try_into()
                                    .unwrap_or(u64::MAX),
                                None,
                                None,
                                None,
                            );
                        }
                        first_content_delta_recorded = true;
                    }
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

        if !final_ops.is_empty() || final_usage.is_some() || activity_pending.load(Ordering::SeqCst)
        {
            let final_ops = batch_stream_delta_ops(final_ops);

            let mut session = session_arc_emitter.lock().await;
            if session.abort_flag.load(Ordering::SeqCst)
                || session
                    .recovery_budget
                    .as_ref()
                    .is_some_and(|budget| budget.check().is_err())
                || session.draft_message.as_ref().map(|m| &m.message_id)
                    != expected_message_id.as_ref()
            {
                return;
            }
            if activity_pending.swap(false, Ordering::SeqCst) {
                session.mark_stream_activity();
            }
            if !final_ops.is_empty() {
                for ops in final_ops {
                    let carries_content = delta_ops_contain_assistant_content(&ops);
                    session.emit_stream_delta(ops);
                    if carries_content && !first_content_delta_recorded {
                        if let Some(started_at) = first_content_started_at {
                            perf_diagnostics::record(
                                PerfComponent::StreamFirstContentDelta,
                                Some(&emitter_chat_id),
                                PerfOutcome::Success,
                                started_at
                                    .elapsed()
                                    .as_micros()
                                    .try_into()
                                    .unwrap_or(u64::MAX),
                                None,
                                None,
                                None,
                            );
                        }
                        first_content_delta_recorded = true;
                    }
                }
            }
            if let Some(usage) = final_usage {
                session.draft_usage = Some(usage);
            }
        }
    })
}

struct StreamAttemptResult {
    results: Result<Vec<ChoiceFinal>, LlmStreamError>,
    cloud_input_usage: Option<crate::chat::cloud_token_count::CloudInputTokenCount>,
    duration_ms: u64,
    call_ts_start: String,
    call_ts_end: String,
}

enum StreamAttemptOutcome {
    Paused,
    Completed(StreamAttemptResult),
}

fn fence_contention_retry_delay(retries: usize) -> Option<std::time::Duration> {
    (retries < 3).then(|| std::time::Duration::from_secs(1 << retries))
}

#[cfg(test)]
tokio::task_local! { static FORCE_FENCE_HELD: bool; }

async fn run_stream_attempt(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    llm_request: &LlmRequest,
    model_rec: &crate::caps::ChatModelRecord,
    abort_flag: &Arc<AtomicBool>,
    abort_notify: &Arc<tokio::sync::Notify>,
    stream_prepare_started_at: Option<Instant>,
    chat_id: &str,
    task_role: Option<&str>,
    expected_board_execution: Option<&Arc<crate::agents::execution::RunExecution>>,
) -> StreamAttemptOutcome {
    let mut contention_retries = 0;
    let current = loop {
        let admission = async {
            #[cfg(test)]
            if FORCE_FENCE_HELD.try_with(|_| ()).is_ok() {
                return Err(crate::tasks::admission::ExecutionUnavailable::Held);
            }
            super::session_runtime::board_execution(app, session_arc).await
        }
        .await;
        match admission {
            Ok(current) => break current,
            Err(crate::tasks::admission::ExecutionUnavailable::Held) => {
                let Some(delay) = fence_contention_retry_delay(contention_retries) else {
                    let mut session = session_arc.lock().await;
                    let reason = "Session busy after execution-fence retries; retry generation when other session work finishes.";
                    session.add_message(make_ui_only_error_message(reason));
                    session.set_runtime_state(SessionState::Paused, Some(reason.to_string()));
                    let meta = session
                        .thread
                        .task_meta
                        .clone()
                        .filter(|meta| meta.role == "agents");
                    drop(session);
                    if let Some(meta) = meta {
                        if let Err(error) =
                            super::task_agent_monitor::record_paused_agent_attention(
                                app.clone(),
                                &meta,
                                chat_id,
                            )
                            .await
                        {
                            warn!("Could not record paused agent attention: {}", error);
                        }
                    }
                    return StreamAttemptOutcome::Paused;
                };
                contention_retries += 1;
                let deadline = tokio::time::Instant::now() + delay;
                loop {
                    if abort_flag.load(Ordering::Relaxed)
                        || app.runtime.shutdown_flag.load(Ordering::Relaxed)
                    {
                        let reason = "Execution-fence retry stopped by abort or shutdown; resume generation when ready.";
                        let mut session = session_arc.lock().await;
                        session.add_message(make_ui_only_error_message(reason));
                        session.set_runtime_state(SessionState::Paused, Some(reason.to_string()));
                        return StreamAttemptOutcome::Paused;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        break;
                    }
                    tokio::select! {
                        _ = abort_notify.notified() => {},
                        _ = tokio::time::sleep_until(deadline.min(tokio::time::Instant::now() + std::time::Duration::from_millis(100))) => {},
                    }
                }
            }
            Err(reason) => {
                super::session_runtime::interrupt_board_execution(
                    &mut *session_arc.lock().await,
                    reason,
                );
                return StreamAttemptOutcome::Paused;
            }
        }
    };
    if !match (expected_board_execution, current.as_ref()) {
        (Some(expected), Some(current)) => {
            Arc::ptr_eq(expected, current) && !expected.is_cancelled()
        }
        (None, None) => true,
        _ => false,
    } {
        super::session_runtime::interrupt_board_execution(
            &mut *session_arc.lock().await,
            crate::tasks::admission::ExecutionUnavailable::StaleBinding(
                crate::tasks::admission::StaleReason::ExecutionReplaced,
            ),
        );
        return StreamAttemptOutcome::Paused;
    }
    let params = StreamRunParams {
        recovery_budget: session_arc.lock().await.recovery_budget.clone(),
        llm_request: llm_request.clone(),
        model_rec: model_rec.base.clone(),
        chat_id: Some(chat_id.to_string()),
        allow_websocket: true,
        abort_flag: Some(abort_flag.clone()),
        abort_notify: Some(abort_notify.clone()),
        supports_tools: model_rec.supports_tools,
        supports_reasoning: model_rec.has_reasoning_support(),
        reasoning_type: model_rec.reasoning_type_string(),
        supports_temperature: model_rec.supports_temperature,
        priority: if task_role == Some("agents") {
            refact_providers::pressure::RequestPriority::Background
        } else {
            refact_providers::pressure::RequestPriority::Interactive
        },
    };

    let mut cloud_input_usage_task = crate::chat::cloud_token_count::spawn_input_token_count_task(
        app.gcx.clone(),
        app.runtime.http_client.clone(),
        &llm_request,
        &model_rec.base,
        Some(chat_id.to_string()),
    )
    .map(tokio_util::task::AbortOnDropHandle::new);

    const EMITTER_QUEUE_CAPACITY: usize = 256;
    let (tx, rx) =
        tokio::sync::mpsc::channel::<StreamCollectorEventPayload>(EMITTER_QUEUE_CAPACITY);
    let overflow_usage = Arc::new(std::sync::Mutex::new(None::<ChatUsage>));
    let overflow_ops = Arc::new(std::sync::Mutex::new(Vec::<DeltaOp>::new()));
    let activity_pending = Arc::new(AtomicBool::new(false));

    let mut collector = SessionCollector {
        tx,
        overflow_usage: overflow_usage.clone(),
        overflow_ops: overflow_ops.clone(),
        activity_pending: activity_pending.clone(),
    };

    let session_arc_emitter = session_arc.clone();
    let first_content_started_at = stream_prepare_started_at;
    let emitter_chat_id = chat_id.to_string();
    let emitter_task = spawn_stream_delta_emitter(
        rx,
        overflow_ops.clone(),
        overflow_usage.clone(),
        activity_pending,
        session_arc_emitter,
        first_content_started_at,
        emitter_chat_id,
        session_arc
            .lock()
            .await
            .draft_message
            .as_ref()
            .map(|m| m.message_id.clone()),
    );

    let call_ts_start = chrono::Utc::now().to_rfc3339();
    let call_start = std::time::Instant::now();

    session_arc
        .lock()
        .await
        .live_diagnostics
        .phase("stream_invocation", Instant::now());
    let stream_outcome = super::stream_core::run_llm_stream_with_prepare_started(
        app.clone(),
        params,
        &mut collector,
        stream_prepare_started_at,
    )
    .await;
    let mut cloud_input_usage = None;
    if stream_outcome.is_err() {
        if let Some(mut handle) = cloud_input_usage_task.take() {
            match tokio::time::timeout(std::time::Duration::from_secs(2), &mut handle).await {
                Ok(Ok(usage)) => cloud_input_usage = usage,
                Ok(Err(_)) => {}
                Err(_) => handle.abort(),
            }
        }
    } else if let Some(handle) = cloud_input_usage_task.take() {
        match handle.await {
            Ok(usage) => {
                cloud_input_usage = usage;
            }
            Err(err) => {
                if !err.is_cancelled() {
                    tracing::warn!(error = %err, "cloud token count task failed");
                }
            }
        }
    }
    drop(collector);
    let _ = emitter_task.await;

    if let Ok(LlmStreamOutcome::PausedForCacheGuard) = stream_outcome {
        tracing::info!("Generation paused by cache guard");
        return StreamAttemptOutcome::Paused;
    }

    let results = stream_outcome
        .map(|o| match o {
            LlmStreamOutcome::Choices(c) => c,
            LlmStreamOutcome::PausedForCacheGuard => unreachable!(),
        })
        .map_err(|error| {
            synthesize_responses_context_cutoff_error_if_needed(
                error,
                &model_rec.base,
                &llm_request.messages,
                llm_request.params.n_ctx.unwrap_or(model_rec.base.n_ctx),
                true,
            )
        });
    let duration_ms = call_start.elapsed().as_millis() as u64;
    let call_ts_end = chrono::Utc::now().to_rfc3339();
    StreamAttemptOutcome::Completed(StreamAttemptResult {
        results,
        cloud_input_usage,
        duration_ms,
        call_ts_start,
        call_ts_end,
    })
}

fn recover_stream_tool_calls(
    result: &mut ChoiceFinal,
    tools: &Option<Vec<serde_json::Value>>,
) -> Result<(), LlmStreamError> {
    let allowed_tools = tool_call_recovery::allowed_tool_names(tools);

    if !result.tool_calls_raw.is_empty() {
        result.tool_calls_raw = tool_call_recovery::unwrap_multi_tool_use_parallel(
            &result.tool_calls_raw,
            &allowed_tools,
        );
    }

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
            .filter_map(|tc| {
                super::stream_core::normalize_tool_call_with_finish(
                    tc,
                    result.finish_reason.as_deref(),
                )
            })
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

    maybe_downgrade_bogus_tool_calls_finish_reason(result, "post_tool_normalization");
    Ok(())
}

enum AttemptResult {
    Retry,
    Success(ChoiceFinal, LlmCallEvent),
}

async fn complete_stream_generation(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    model_rec: &crate::caps::ChatModelRecord,
    result: ChoiceFinal,
    pending_success_event: LlmCallEvent,
) -> bool {
    let (model_id, usage_for_pricing) = {
        let session = session_arc.lock().await;
        (model_rec.base.id.clone(), session.draft_usage.clone())
    };
    let metering_usd = if let Some(ref usage) = usage_for_pricing {
        generation_metering_usd(app, &model_id, usage).await
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

    let mut session = session_arc.lock().await;
    if session.abort_flag.load(Ordering::SeqCst)
        || session
            .recovery_budget
            .as_ref()
            .is_some_and(|budget| budget.check().is_err())
    {
        return false;
    }
    if session.draft_message.is_none() {
        return false;
    }
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
                .filter_map(|tc| {
                    super::stream_core::normalize_tool_call_with_finish(
                        tc,
                        result.finish_reason.as_deref(),
                    )
                })
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

    let is_chatgpt_backend = model_rec.base.endpoint.contains("chatgpt.com/backend-api");
    if model_rec.base.wire_format == crate::llm::WireFormat::OpenaiResponses && !is_chatgpt_backend
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

    let next_state = if session
        .draft_message
        .as_ref()
        .and_then(|m| m.tool_calls.as_ref())
        .is_some_and(|tool_calls| !tool_calls.is_empty())
    {
        SessionState::ExecutingTools
    } else {
        SessionState::Idle
    };
    session.finish_stream_with_next_state(result.finish_reason, next_state);
    true
}

async fn finish_stream_attempt(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    llm_request: &LlmRequest,
    chat_id: &str,
    root_chat_id: &Option<String>,
    mode_for_stats: &str,
    task_id: &Option<String>,
    task_role: &Option<String>,
    agent_id: &Option<String>,
    card_id: &Option<String>,
    attempt: usize,
    max_attempts: usize,
    can_retry_with_temp_bump: bool,
    cloud_input_usage: &Option<crate::chat::cloud_token_count::CloudInputTokenCount>,
    duration_ms: u64,
    call_ts_start: String,
    call_ts_end: String,
    results: Result<Vec<ChoiceFinal>, LlmStreamError>,
) -> Result<AttemptResult, LlmStreamError> {
    const TEMPERATURE_BUMP: f32 = 0.1;
    const MAX_RETRY_TEMPERATURE: f32 = 0.5;

    let model_id_for_stats = llm_request.model_id.clone();
    let messages_count = llm_request.messages.len();
    let tools_count = llm_request.tools.as_ref().map(|t| t.len()).unwrap_or(0);
    let temperature_for_stats = llm_request.params.temperature;
    let max_tokens_for_stats = llm_request.params.max_tokens;
    let results = match results {
        Err(error) => {
            let usage_for_error = {
                let session = session_arc.lock().await;
                session.draft_usage.clone()
            }
            .or_else(|| cloud_input_usage.as_ref().map(|count| count.usage.clone()));
            let (provider, model) = split_model_provider(&model_id_for_stats);
            let event = LlmCallEvent {
                id: uuid::Uuid::new_v4().to_string(),
                ts_start: call_ts_start.clone(),
                ts_end: call_ts_end.clone(),
                duration_ms,
                chat_id: chat_id.to_string(),
                root_chat_id: root_chat_id.clone(),
                mode: mode_for_stats.to_string(),
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
                error_message: Some(error.message.chars().take(200).collect()),
                finish_reason: None,
                attempt_n: attempt,
                retry_reason: None,
                prompt_tokens: usage_for_error
                    .as_ref()
                    .map(|u| u.prompt_tokens)
                    .unwrap_or(0),
                completion_tokens: usage_for_error
                    .as_ref()
                    .map(|u| u.completion_tokens)
                    .unwrap_or(0),
                cache_read_tokens: usage_for_error.as_ref().and_then(|u| u.cache_read_tokens),
                cache_creation_tokens: usage_for_error
                    .as_ref()
                    .and_then(|u| u.cache_creation_tokens),
                total_tokens: usage_for_error
                    .as_ref()
                    .map(|u| u.total_tokens)
                    .unwrap_or(0),
                cost_usd: None,
            };
            if let Some(sender) = &app.model.llm_stats_sender {
                if sender.try_send(event).is_err() {
                    tracing::warn!("stats: channel full, dropping LLM call event");
                }
            }
            return Err(error);
        }
        Ok(results) => results,
    };

    let mut result = results.into_iter().next().unwrap_or_default();

    if result.usage.is_none() {
        if let Some(count) = cloud_input_usage.clone() {
            result.usage = Some(count.usage);
        }
    }
    if let Some(usage) = result.usage.clone() {
        let mut session = session_arc.lock().await;
        session.draft_usage = Some(usage);
    }

    recover_stream_tool_calls(&mut result, &llm_request.tools)?;

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
            chat_id: chat_id.to_string(),
            root_chat_id: root_chat_id.clone(),
            mode: mode_for_stats.to_string(),
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
            let next_temp = (TEMPERATURE_BUMP * (attempt - 1) as f32).min(MAX_RETRY_TEMPERATURE);
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
            return Ok(AttemptResult::Retry);
        } else {
            let effective_temp = llm_request.params.temperature.unwrap_or(0.0);
            return Err(format!(
                "Empty assistant response after {} attempts (T={:.1})",
                max_attempts, effective_temp
            )
            .into());
        }
    }

    if !result.tool_calls_raw.is_empty()
        && crate::buddy::companion::is_companion(&session_arc.lock().await.thread)
    {
        return Err(
            "Buddy companion returned tool calls; no action was authorized or dispatched"
                .to_string()
                .into(),
        );
    }

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
        chat_id: chat_id.to_string(),
        root_chat_id: root_chat_id.clone(),
        mode: mode_for_stats.to_string(),
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

    Ok(AttemptResult::Success(result, pending_success_event))
}

async fn run_streaming_generation(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    mut llm_request: LlmRequest,
    model_rec: &crate::caps::ChatModelRecord,
    abort_flag: Arc<AtomicBool>,
    abort_notify: Arc<tokio::sync::Notify>,
    stream_prepare_started_at: Option<Instant>,
    expected_board_execution: Option<Arc<crate::agents::execution::RunExecution>>,
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
    let recovery_controlled = session_arc.lock().await.recovery_budget.is_some();
    let can_retry_with_temp_bump =
        !recovery_controlled && user_specified_temp.is_none() && model_supports_temperature;
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

        let attempt_result = run_stream_attempt(
            &app,
            &session_arc,
            &llm_request,
            model_rec,
            &abort_flag,
            &abort_notify,
            stream_prepare_started_at,
            &chat_id,
            task_role.as_deref(),
            expected_board_execution.as_ref(),
        )
        .await;
        let StreamAttemptOutcome::Completed(attempt_result) = attempt_result else {
            return Ok(GenerationResult::PausedForUserDecision);
        };
        let StreamAttemptResult {
            results,
            cloud_input_usage,
            duration_ms,
            call_ts_start,
            call_ts_end,
        } = attempt_result;

        match finish_stream_attempt(
            &app,
            &session_arc,
            &llm_request,
            &chat_id,
            &root_chat_id,
            &mode_for_stats,
            &task_id,
            &task_role,
            &agent_id,
            &card_id,
            attempt,
            max_attempts,
            can_retry_with_temp_bump,
            &cloud_input_usage,
            duration_ms,
            call_ts_start,
            call_ts_end,
            results,
        )
        .await?
        {
            AttemptResult::Retry => continue,
            AttemptResult::Success(result, event) => break (result, event),
        }
    };

    if complete_stream_generation(&app, &session_arc, model_rec, result, pending_success_event)
        .await
    {
        Ok(GenerationResult::Completed)
    } else {
        Ok(GenerationResult::PausedForUserDecision)
    }
}

fn is_result_empty(result: &ChoiceFinal) -> bool {
    length_like_finish_reason(result.finish_reason.as_deref())
        != Some(NormalizedStopReason::ProviderLengthStop)
        && result.content.trim().is_empty()
        && !result
            .tool_calls_raw
            .iter()
            .any(|call| normalize_tool_call(call).is_some())
        && !result
            .server_content_blocks
            .iter()
            .any(|block| is_standalone_artifact(block) || is_server_tool_content(block))
}

fn is_server_tool_content(block: &serde_json::Value) -> bool {
    matches!(
        block.get("type").and_then(|value| value.as_str()),
        Some(
            "server_tool_use"
                | "web_search_tool_result"
                | "web_fetch_tool_result"
                | "code_execution_tool_result"
                | "bash_code_execution_tool_result"
                | "text_editor_code_execution_tool_result"
        )
    )
}

fn is_standalone_artifact(block: &serde_json::Value) -> bool {
    let nonblank = |value: Option<&serde_json::Value>| {
        value
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.trim().is_empty())
    };
    match block.get("type").and_then(|value| value.as_str()) {
        Some("image_generation_call") => {
            block["payload"]["status"] == "completed" && nonblank(block["payload"].get("result"))
        }
        Some("audio") => nonblank(block["payload"].get("data")),
        Some("google_cloud_code_inline_data") => nonblank(block.get("data")),
        _ => false,
    }
}

fn latest_goal_pursuit_usage(session: &ChatSession) -> Option<ChatUsage> {
    let goal = session.goal.as_ref()?;
    if !goal.active || goal.status != GoalStatus::Active {
        return None;
    }
    let assistant = session
        .messages
        .iter()
        .rfind(|message| message.role == "assistant")?;
    if assistant
        .extra
        .get("goal_turn_accounted")
        .and_then(|value| value.as_bool())
        == Some(true)
        || assistant
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
    {
        return None;
    }
    let mut total = ChatUsage::default();
    for message in session.messages.iter().rev() {
        if message
            .extra
            .get("goal_turn_accounted")
            .and_then(|value| value.as_bool())
            == Some(true)
        {
            break;
        }
        if message.role != "assistant" {
            continue;
        }
        if let Some(usage) = &message.usage {
            total.prompt_tokens = total.prompt_tokens.saturating_add(usage.prompt_tokens);
            total.completion_tokens = total
                .completion_tokens
                .saturating_add(usage.completion_tokens);
            total.total_tokens = total.total_tokens.saturating_add(usage.total_tokens);
            if let Some(metering) = &usage.metering_usd {
                total
                    .metering_usd
                    .get_or_insert_with(Default::default)
                    .total_usd += metering.total_usd;
            }
        }
    }
    Some(total)
}

fn record_completed_goal_turn(session: &mut ChatSession) -> bool {
    let Some(usage) = latest_goal_pursuit_usage(session) else {
        return false;
    };
    if !session.goal_record_progress_from_usage(&usage) {
        return false;
    }
    if let Some(assistant) = session
        .messages
        .iter_mut()
        .rfind(|message| message.role == "assistant")
    {
        assistant
            .extra
            .insert("goal_turn_accounted".into(), serde_json::json!(true));
    }
    true
}

async fn maybe_record_goal_pursuit_progress(
    app: &AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> Result<bool, String> {
    if !crate::tasks::goal::hydrate_session_task_goal_for_pursuit(app.gcx.clone(), &session_arc)
        .await?
    {
        return Ok(false);
    }
    let changed = {
        let mut session = session_arc.lock().await;
        record_completed_goal_turn(&mut session)
    };
    if changed {
        crate::tasks::goal::commit_session_task_goal(app.gcx.clone(), &session_arc).await?;
    }
    Ok(changed)
}

fn maybe_downgrade_bogus_tool_calls_finish_reason(result: &mut ChoiceFinal, stage: &str) {
    let original = match result.finish_reason.as_deref() {
        Some(reason @ ("tool_calls" | "tool_use")) => reason.to_string(),
        _ => return,
    };
    if !result.tool_calls_raw.is_empty() {
        return;
    }

    warn!(
        "tool_call_guard: finish_reason='{}' without tool calls at stage '{}', downgrading to 'stop'",
        original, stage
    );
    result.extra.insert(
        "_tool_call_guard".to_string(),
        json!({
            "kind": "tool_calls_finish_without_calls",
            "stage": stage,
            "original_finish_reason": original,
            "adjusted_finish_reason": "stop",
        }),
    );
    result.finish_reason = Some("stop".to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatToolCall, ChatToolFunction};

    fn ready_continuation(session: &ChatSession, nonce: &str) -> ReviewReadinessContinuation {
        ReviewReadinessContinuation {
            turn_nonce: nonce.into(),
            receipt: crate::tool_jobs::JobReceipt {
                job_id: "readiness-job".into(),
                binding: crate::tool_jobs::JobBinding {
                    scope_keys: Vec::new(),
                    chat_id: session.chat_id.clone(),
                    tool_call_id: String::new(),
                    tool_name: "review_readiness".into(),
                    model: session.thread.model.clone(),
                    turn_id: nonce.into(),
                },
                status: crate::tool_jobs::JobStatus::Succeeded,
                result_url: String::new(),
                cancel_url: String::new(),
            },
            command: session.active_command.clone(),
            trajectory_version: session.trajectory_version,
            thread_fingerprint: serde_json::to_value(&session.thread).unwrap(),
            cancelled: Arc::new(AtomicBool::new(false)),
            ready: true,
        }
    }

    #[tokio::test]
    async fn readiness_resume_is_single_use_and_preserves_original_command() {
        let mut session = ChatSession::new("readiness-resume".into());
        session.active_command.model_override = Some("restricted-model".into());
        session.review_readiness_pending = Some(ready_continuation(&session, "original-attempt"));
        session.active_command = ActiveCommandContext::default();
        let session = Arc::new(AMutex::new(session));
        assert!(take_ready_review_continuation(&session).await);
        assert!(!take_ready_review_continuation(&session).await);
        let locked = session.lock().await;
        assert_eq!(
            locked.active_command.model_override.as_deref(),
            Some("restricted-model")
        );
        assert_eq!(
            locked.review_readiness_resume_nonce.as_deref(),
            Some("original-attempt")
        );
        assert!(locked.messages.is_empty());
    }

    #[test]
    fn readiness_stale_observer_clears_only_its_matching_continuation() {
        let mut session = ChatSession::new("readiness-stale-observer".into());
        session.review_readiness_pending = Some(ready_continuation(&session, "new-attempt"));
        assert!(!clear_stale_review_observer(&mut session, "old-attempt"));
        assert!(session.review_readiness_pending.is_some());
        let cancelled = session
            .review_readiness_pending
            .as_ref()
            .unwrap()
            .cancelled
            .clone();
        assert!(clear_stale_review_observer(&mut session, "new-attempt"));
        assert!(cancelled.load(Ordering::SeqCst));
        assert!(session.review_readiness_pending.is_none());
    }

    #[tokio::test]
    async fn readiness_rejects_cancelled_stale_scope_closed_and_human_priority() {
        for case in ["cancelled", "frontier", "scope", "closed", "human", "abort"] {
            let mut session = ChatSession::new(format!("readiness-{case}"));
            session.review_readiness_pending = Some(ready_continuation(&session, "old-attempt"));
            let cancellation = session
                .review_readiness_pending
                .as_ref()
                .unwrap()
                .cancelled
                .clone();
            match case {
                "cancelled" => cancellation.store(true, Ordering::SeqCst),
                "frontier" => session.increment_version(),
                "scope" => session.thread.mode = "different-policy".into(),
                "closed" => session.closed = true,
                "human" => {
                    session.enqueue_command(CommandRequest {
                        native_approval_guard: None,
                        supervision_guard: None,
                        client_request_id: "human-priority".into(),
                        priority: true,
                        command: ChatCommand::Regenerate {},
                    });
                }
                "abort" => session.abort_stream(),
                _ => unreachable!(),
            }
            let session = Arc::new(AMutex::new(session));
            assert!(!take_ready_review_continuation(&session).await, "{case}");
            assert!(
                session.lock().await.review_readiness_resume_nonce.is_none(),
                "{case}"
            );
            if matches!(case, "human" | "abort") {
                assert!(cancellation.load(Ordering::SeqCst));
            }
        }
    }

    #[tokio::test]
    async fn shutdown_invalidates_pending_without_starting_generation() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let mut session = ChatSession::new("readiness-shutdown".into());
        session.review_readiness_pending = Some(ready_continuation(&session, "never-resume"));
        let cancelled = session
            .review_readiness_pending
            .as_ref()
            .unwrap()
            .cancelled
            .clone();
        let session = Arc::new(AMutex::new(session));
        app.runtime.shutdown_flag.store(true, Ordering::Relaxed);
        start_generation(app, session.clone()).await;
        let session = session.lock().await;
        assert!(cancelled.load(Ordering::SeqCst));
        assert!(session.review_readiness_pending.is_none());
        assert!(session.draft_message.is_none());
        assert!(session.messages.is_empty());
        assert_eq!(session.turn_depth, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn execution_fence_contention_has_bounded_retry_schedule() {
        assert_eq!(
            fence_contention_retry_delay(0),
            Some(std::time::Duration::from_secs(1))
        );
        assert_eq!(
            fence_contention_retry_delay(1),
            Some(std::time::Duration::from_secs(2))
        );
        assert_eq!(
            fence_contention_retry_delay(2),
            Some(std::time::Duration::from_secs(4))
        );
        assert_eq!(fence_contention_retry_delay(3), None);
        for (cancelled, replaced, claimed_elsewhere) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let (_temp, app, task_id, chat_id, session, _planner) =
                super::super::task_agent_monitor::tests::setup_monitor_case(
                    "doing",
                    SessionState::Generating,
                    std::time::Duration::ZERO,
                    vec![],
                )
                .await;
            if replaced {
                crate::tasks::storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
                    board.get_card_mut("T-1").unwrap().agent_chat_id =
                        Some("replacement-chat".into());
                    Ok(())
                })
                .await
                .unwrap();
            }
            if claimed_elsewhere {
                crate::tasks::storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
                    board.card_claims.insert(
                        "T-1".into(),
                        crate::tasks::types::CardClaim {
                            claimant_chat_id: "replacement-chat".into(),
                            idempotency_key: "replacement-claim".into(),
                            claimed_at: chrono::Utc::now().to_rfc3339(),
                        },
                    );
                    Ok(())
                })
                .await
                .unwrap();
            }
            let (abort, notify) = {
                let live = session.lock().await;
                (live.abort_flag.clone(), live.abort_notify.clone())
            };
            abort.store(cancelled, Ordering::Relaxed);
            let model = crate::caps::ChatModelRecord::default();
            let outcome = FORCE_FENCE_HELD
                .scope(
                    true,
                    run_stream_attempt(
                        &app,
                        &session,
                        &LlmRequest::new("provider/model".into(), vec![]),
                        &model,
                        &abort,
                        &notify,
                        None,
                        &chat_id,
                        Some("agents"),
                        None,
                    ),
                )
                .await;
            assert!(matches!(outcome, StreamAttemptOutcome::Paused));
            let live = session.lock().await;
            assert_eq!(live.runtime.state, SessionState::Paused);
            assert!(!super::super::task_agent_monitor::session_is_failure(&live));
            assert!(live.runtime.interruption.is_none());
            assert!(live.runtime.error.is_some());
            assert!(live.messages.iter().any(|message| message
                .content
                .content_text_only()
                .contains(if cancelled {
                    "abort or shutdown"
                } else {
                    "Session busy"
                })));
            drop(live);
            super::super::task_agent_monitor::check_for_stuck_agents(app.clone())
                .await
                .unwrap();
            let board = crate::tasks::storage::load_board(app.gcx.clone(), &task_id)
                .await
                .unwrap();
            let card = board.get_card("T-1").unwrap();
            assert_eq!(card.column, "doing");
            if !cancelled && !replaced && !claimed_elsewhere {
                let attention = card
                    .attention
                    .as_ref()
                    .expect("busy agent must escalate to the board");
                assert_eq!(attention.kind, "no_session");
                assert_eq!(attention.count, 1);
                assert!(attention.reason.contains("resume the agent"));
                assert_eq!(
                    card.status_updates
                        .iter()
                        .filter(|update| update.message.contains("agent paused:"))
                        .count(),
                    1
                );
            } else {
                assert!(card.attention.is_none());
            }
            let live = session.lock().await;
            if !cancelled {
                assert!(live
                    .messages
                    .iter()
                    .any(|message| message.content.content_text_only().contains("Session busy")));
            }
        }
    }

    #[tokio::test]
    async fn nonactionable_stream_completions_retry_then_fail() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let session = Arc::new(AMutex::new(ChatSession::new("empty-completion".into())));
        let request = LlmRequest::new("provider/model".into(), vec![]);
        let fixtures = [
            ChoiceFinal {
                server_content_blocks: vec![json!({"type":"unhandled_openai_responses_event"})],
                ..Default::default()
            },
            ChoiceFinal {
                thinking_blocks: vec![json!({"type":"reasoning","encrypted_content":"opaque"})],
                ..Default::default()
            },
        ];
        for fixture in fixtures {
            for attempt in [1, 2] {
                let result = finish_stream_attempt(
                    &app,
                    &session,
                    &request,
                    "empty-completion",
                    &None,
                    "agent",
                    &None,
                    &None,
                    &None,
                    &None,
                    attempt,
                    2,
                    true,
                    &None,
                    0,
                    String::new(),
                    String::new(),
                    Ok(vec![fixture.clone()]),
                )
                .await;
                if attempt == 1 {
                    assert!(matches!(result, Ok(AttemptResult::Retry)));
                } else {
                    assert!(result
                        .err()
                        .unwrap()
                        .message
                        .starts_with("Empty assistant response"));
                }
                assert!(session.lock().await.messages.is_empty());
            }
        }
        assert!(!is_result_empty(&ChoiceFinal {
            content: "refusal or answer".into(),
            ..Default::default()
        }));
        assert!(!is_result_empty(&ChoiceFinal {
            tool_calls_raw: vec![json!({"id":"call","function":{"name":"cat","arguments":"{}"}})],
            ..Default::default()
        }));
        assert!(is_result_empty(&ChoiceFinal {
            tool_calls_raw: vec![json!({"bad":"call"})],
            ..Default::default()
        }));
        assert!(!is_result_empty(&ChoiceFinal {
            server_content_blocks: vec![
                json!({"type":"image_generation_call","payload":{"status":"completed","result":"image-bytes"}})
            ],
            ..Default::default()
        }));
        assert!(is_result_empty(&ChoiceFinal {
            server_content_blocks: vec![
                json!({"type":"image_generation_call","payload":{"status":"in_progress"}})
            ],
            ..Default::default()
        }));
    }

    #[tokio::test]
    async fn reasoning_length_stop_and_server_results_are_not_empty_retries() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let session = Arc::new(AMutex::new(ChatSession::new("nonempty".into())));
        for fixture in [
            ChoiceFinal {
                finish_reason: Some("length".into()),
                reasoning: "still thinking".into(),
                ..Default::default()
            },
            ChoiceFinal {
                server_content_blocks: vec![
                    json!({"type":"server_tool_use","id":"search"}),
                    json!({"type":"web_search_tool_result","tool_use_id":"search","content":[]}),
                ],
                ..Default::default()
            },
        ] {
            let result = finish_stream_attempt(
                &app,
                &session,
                &LlmRequest::new("provider/model".into(), vec![]),
                "nonempty",
                &None,
                "agent",
                &None,
                &None,
                &None,
                &None,
                1,
                1,
                false,
                &None,
                0,
                String::new(),
                String::new(),
                Ok(vec![fixture]),
            )
            .await;
            assert!(matches!(result, Ok(AttemptResult::Success(_, _))));
        }
    }

    #[tokio::test]
    async fn goal_turn_accounting_consumer_aggregates_tools_and_deduplicates_recovery() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let mut session = ChatSession::new("accounting".into());
        session.install_goal(
            "agent",
            "ship",
            true,
            refact_chat_api::GoalBudget::default(),
        );
        session.goal.as_mut().unwrap().snoozed_until_ms = Some(u64::MAX);
        let context = GenerationAttemptContext {
            thread: session.thread.clone(),
            chat_id: session.chat_id.clone(),
            chat_label: "accounting".into(),
            abort_flag: session.abort_flag.clone(),
        };
        let session = Arc::new(AMutex::new(session));
        for turn in 1..=2 {
            for step in 0..2 {
                {
                    let mut locked = session.lock().await;
                    let call_id = format!("call-{turn}-{step}");
                    let mut assistant = make_assistant_with_tool_call(&call_id, "cat");
                    assistant.usage = Some(ChatUsage {
                        total_tokens: 100,
                        completion_tokens: 10,
                        ..Default::default()
                    });
                    locked.messages.push(assistant);
                    locked.messages.push(make_tool_msg(&call_id, "result"));
                    if step == 0 {
                        locked
                            .messages
                            .push(ChatMessage::new("goal".into(), "anchored control".into()));
                    }
                }
                handle_tool_step_outcome(
                    &app,
                    &session,
                    &context,
                    ToolStepOutcome::Continue,
                    None,
                    Default::default(),
                )
                .await;
                assert_eq!(
                    session
                        .lock()
                        .await
                        .goal
                        .as_ref()
                        .unwrap()
                        .progress
                        .turns_used,
                    turn - 1
                );
            }
            session
                .lock()
                .await
                .messages
                .push(make_assistant_msg("done"));
            handle_tool_step_outcome(
                &app,
                &session,
                &context,
                ToolStepOutcome::NoToolCalls,
                None,
                Default::default(),
            )
            .await;
            {
                let locked = session.lock().await;
                assert_eq!(locked.goal.as_ref().unwrap().progress.turns_used, turn);
                assert_eq!(
                    locked.goal.as_ref().unwrap().progress.tokens_used,
                    u64::from(turn) * 200
                );
                assert_eq!(
                    locked
                        .goal_ledger
                        .iter()
                        .filter(|entry| matches!(
                            entry.op,
                            refact_chat_api::GoalLedgerOp::ProgressRecorded { .. }
                        ))
                        .count(),
                    turn as usize
                );
            }
            let restored = {
                let locked = session.lock().await;
                let mut restored = ChatSession::new("accounting-restored".into());
                restored.messages =
                    serde_json::from_value(serde_json::to_value(&locked.messages).unwrap())
                        .unwrap();
                restored.goal = locked.goal.clone();
                restored.goal_ledger = locked.goal_ledger.clone();
                Arc::new(AMutex::new(restored))
            };
            assert!(!maybe_record_goal_pursuit_progress(&app, restored)
                .await
                .unwrap());
            assert!(!maybe_record_goal_pursuit_progress(&app, session.clone())
                .await
                .unwrap());
            assert_eq!(
                session
                    .lock()
                    .await
                    .goal
                    .as_ref()
                    .unwrap()
                    .progress
                    .turns_used,
                turn
            );
        }
    }

    #[tokio::test]
    async fn private_reasoning_activity_reaches_session_without_display_or_stale_attempts() {
        for (stale, saturated) in [(false, false), (true, false), (false, true)] {
            let mut session = ChatSession::new("activity".into());
            session.start_stream().unwrap();
            session.last_stream_delta_at = None;
            let expected = session.draft_message.as_ref().map(|m| m.message_id.clone());
            let before = serde_json::to_value(&session.draft_message).unwrap();
            let version = session.trajectory_version;
            if stale {
                session.draft_message.as_mut().unwrap().message_id = "replacement".into();
            }
            let session = Arc::new(AMutex::new(session));
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let ops = Arc::new(std::sync::Mutex::new(Vec::new()));
            let usage = Arc::new(std::sync::Mutex::new(None));
            let activity = Arc::new(AtomicBool::new(false));
            let mut collector = SessionCollector {
                tx,
                overflow_ops: ops.clone(),
                overflow_usage: usage.clone(),
                activity_pending: activity.clone(),
            };
            if saturated {
                collector.on_usage(&ChatUsage::default());
            }
            super::super::stream_core::process_activity_fixture(
                r#"{"type":"response.reasoning.delta","delta":"PRIVATE_SENTINEL"}"#,
                &mut collector,
            );
            drop(collector);
            spawn_stream_delta_emitter(
                rx,
                ops,
                usage,
                activity,
                session.clone(),
                None,
                "activity".into(),
                expected,
            )
            .await
            .unwrap();
            let session = session.lock().await;
            assert_eq!(session.last_stream_delta_at.is_some(), !stale);
            assert_eq!(session.trajectory_version, version);
            if !stale {
                assert_eq!(
                    serde_json::to_value(&session.draft_message).unwrap(),
                    before
                );
            }
            assert!(!serde_json::to_string(&session.messages)
                .unwrap()
                .contains("PRIVATE_SENTINEL"));
        }
    }

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

    fn make_low_pressure_length_stop() -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            finish_reason: Some("length".to_string()),
            usage: Some(ChatUsage {
                prompt_tokens: 100,
                completion_tokens: 0,
                total_tokens: 100,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                metering_usd: None,
            }),
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
                started_at_ms: None,
                completed_at_ms: None,
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

    fn make_context_file_msg() -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::SimpleText("file content".to_string()),
            ..Default::default()
        }
    }

    fn context_limit_error(message: &str, partial_output_emitted: bool) -> LlmStreamError {
        LlmStreamError {
            message: message.to_string(),
            partial_output_emitted,
            pressure_failure: None,
            admission_refusal: None,
        }
    }

    fn claude_code_model() -> BaseModelRecord {
        BaseModelRecord {
            wire_format: crate::llm::WireFormat::AnthropicMessages,
            auth_token: "cc-oauth-token".to_string(),
            ..Default::default()
        }
    }

    fn shared_model_defaults() -> (ProviderDefaults, crate::caps::DefaultModels) {
        (
            ProviderDefaults {
                chat: ModelTypeDefaults {
                    temperature: Some(0.1),
                    ..Default::default()
                },
                task_planner_agent_model: ModelTypeDefaults {
                    temperature: Some(0.3),
                    ..Default::default()
                },
                ..Default::default()
            },
            crate::caps::DefaultModels {
                chat_default_model: "shared/model".to_string(),
                task_planner_agent_model: "shared/model".to_string(),
                ..Default::default()
            },
        )
    }

    fn final_context_limit_error_bound() -> usize {
        context_limit_final_error_message("").len()
            + crate::chat::diagnostics::SAFE_PROVIDER_ERROR_DIAGNOSTIC_MAX_CHARS
    }

    fn partial_output_error_bound() -> usize {
        PARTIAL_OUTPUT_STREAM_ERROR.len()
            + " Original error: ".len()
            + crate::chat::diagnostics::SAFE_PROVIDER_ERROR_DIAGNOSTIC_MAX_CHARS
    }

    fn assert_context_limit_secret_redacted(text: &str) {
        assert!(!text.contains("sk-test-secret"), "secret leaked: {text}");
        assert!(
            !text.contains("Authorization: Bearer sk-test-secret"),
            "raw authorization header leaked: {text}"
        );
        assert!(
            !text.contains("Bearer sk-test-secret"),
            "raw bearer token leaked: {text}"
        );
        assert!(
            text.contains("[REDACTED"),
            "redaction marker missing: {text}"
        );
    }

    #[test]
    fn context_limit_final_error_redacts_provider_secret() {
        let text = context_limit_final_error_message(
            "context_length_exceeded: Authorization: Bearer sk-test-secret",
        );

        assert!(text.starts_with("Context too large and context reconstruction failed"));
        assert!(text.contains("Explicitly rebuild"));
        assert!(text.contains("Original error:"));
        assert_context_limit_secret_redacted(&text);
        assert!(
            text.len() <= final_context_limit_error_bound(),
            "len={}",
            text.len()
        );
    }

    #[test]
    fn context_limit_final_error_windows_huge_provider_error() {
        let far_tail = "FAR_TAIL_MARKER";
        let error = format!(
            "context_length_exceeded: Authorization: Bearer sk-test-secret {} {}",
            "tail ".repeat(100_000),
            far_tail,
        );

        let text = context_limit_final_error_message(&error);

        assert_context_limit_secret_redacted(&text);
        assert!(text.contains(crate::chat::diagnostics::SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED));
        assert!(!text.contains(far_tail), "far-tail marker leaked: {text}");
        assert!(
            text.len() <= final_context_limit_error_bound(),
            "len={}",
            text.len()
        );
    }

    #[test]
    fn partial_output_stream_error_redacts_provider_secret() {
        let text = partial_output_stream_error_message(
            "provider failed: Authorization: Bearer sk-test-secret",
        );

        assert!(text.starts_with(PARTIAL_OUTPUT_STREAM_ERROR));
        assert_context_limit_secret_redacted(&text);
        assert!(
            text.len() <= partial_output_error_bound(),
            "len={}",
            text.len()
        );
    }

    #[test]
    fn partial_output_stream_error_windows_huge_provider_error() {
        let far_tail = "FAR_TAIL_MARKER";
        let error = format!(
            "provider failed: Authorization: Bearer sk-test-secret {} {}",
            "tail ".repeat(100_000),
            far_tail,
        );

        let text = partial_output_stream_error_message(&error);

        assert_context_limit_secret_redacted(&text);
        assert!(text.contains(crate::chat::diagnostics::SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED));
        assert!(!text.contains(far_tail), "far-tail marker leaked: {text}");
        assert!(
            text.len() <= partial_output_error_bound(),
            "len={}",
            text.len()
        );
    }

    #[test]
    fn wrapped_partial_output_context_limit_error_is_sanitized() {
        let wrapped = format!(
            "{} Original error: context_length_exceeded: Authorization: Bearer sk-test-secret",
            PARTIAL_OUTPUT_STREAM_ERROR,
        );
        let error = context_limit_error(&wrapped, true);

        let final_message = context_limit_final_error_message(&error.message);
        let wrapped_final = format!(
            "{} Original error: {}",
            PARTIAL_OUTPUT_STREAM_ERROR, final_message,
        );

        assert_context_limit_secret_redacted(&wrapped_final);
        assert!(
            wrapped_final.len()
                <= PARTIAL_OUTPUT_STREAM_ERROR.len() + 17 + final_context_limit_error_bound()
        );
    }

    fn identified(id: &str, mut message: ChatMessage) -> ChatMessage {
        message.message_id = id.to_string();
        message
    }

    fn marked_context_file(id: &str, marker: &str) -> ChatMessage {
        let mut message = ChatMessage::new("context_file".into(), format!("{marker} files"));
        message.message_id = id.to_string();
        message.tool_call_id = marker.to_string();
        message
    }

    /// A handed-off chat starts as a single reconstruction report whose payload begins
    /// with context files, exactly like the mode-transition destination.
    fn handed_off_session() -> ChatSession {
        use refact_core::active_context::{make_reconstruction_report, ReconstructionMetadata};
        let payload = vec![
            identified(
                "rebuilt-files",
                ChatMessage::new("context_file".into(), "rebuilt files".into()),
            ),
            identified(
                "rebuilt-memory",
                ChatMessage::new("context_file".into(), "rebuilt memory".into()),
            ),
            identified("previous", make_user_msg("## Previous Conversation")),
            identified("summary", make_user_msg("## Summary")),
        ];
        let mut session = ChatSession::new("handoff-destination".into());
        session.add_message(ChatMessage::new("system".into(), "archived prompt".into()));
        session.add_message(
            make_reconstruction_report(payload, ReconstructionMetadata::default()).unwrap(),
        );
        session
    }

    fn active_roles(session: &ChatSession) -> Vec<String> {
        refact_core::active_context::active_context(&session.messages)
            .unwrap()
            .messages
            .iter()
            .map(|m| m.role.clone())
            .collect()
    }

    #[test]
    fn preamble_lands_at_the_head_of_a_reconstructed_context() {
        let mut session = handed_off_session();
        let archive = serde_json::to_value(&session.messages[0]).unwrap();
        let report_id = session.messages[1].message_id.clone();
        let mut events = session.subscribe();
        let active = refact_core::active_context::active_context(&session.messages)
            .unwrap()
            .messages;
        let mut with_preamble = vec![ChatMessage::new("system".into(), "fresh prompt".into())];
        with_preamble.extend(active[..2].iter().cloned());
        with_preamble.push(marked_context_file(
            "project",
            crate::chat::system_context::PROJECT_CONTEXT_MARKER,
        ));
        with_preamble.push(marked_context_file("memories", "memories_context"));
        with_preamble.extend(active[2..].iter().cloned());

        assert_eq!(persist_preamble_messages(&mut session, &with_preamble), 3);
        assert_eq!(
            active_roles(&session),
            vec![
                "system",
                "context_file",
                "context_file",
                "context_file",
                "context_file",
                "user",
                "user"
            ]
        );
        let active = refact_core::active_context::active_context(&session.messages)
            .unwrap()
            .messages;
        assert_eq!(active[0].content.content_text_only(), "fresh prompt");
        assert_eq!(
            active[1].tool_call_id,
            crate::chat::system_context::PROJECT_CONTEXT_MARKER
        );
        assert_eq!(active[2].tool_call_id, "memories_context");
        assert_eq!(session.messages.len(), 2);
        assert_eq!(serde_json::to_value(&session.messages[0]).unwrap(), archive);
        assert_eq!(
            session
                .messages
                .iter()
                .filter(|m| m.role == "compression_report")
                .count(),
            1
        );
        let linearized =
            crate::chat::linearize::apply_summarization_linearize(session.messages.clone())
                .unwrap();
        assert!(crate::chat::history_limit::validate_chat_history(&linearized).is_ok());
        assert_eq!(
            crate::chat::trajectories::first_system_prompt(&linearized).as_deref(),
            Some("fresh prompt")
        );

        let mut updated_report = 0;
        while let Ok(event) = events.try_recv() {
            let event: serde_json::Value = serde_json::from_str(&event).unwrap();
            assert_ne!(event["type"], "message_added");
            if event["type"] == "message_updated" {
                assert_eq!(event["message_id"], report_id);
                updated_report += 1;
            }
        }
        assert_eq!(updated_report, 3);

        assert_eq!(persist_preamble_messages(&mut session, &with_preamble), 0);
    }

    #[tokio::test]
    async fn resolved_new_chat_default_persists_through_checkpoint_writer() {
        let root = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![root.path().canonicalize().unwrap()];
        let mut session = ChatSession::new("resolved-default-snapshot".into());
        session.thread.context_tokens_cap = Some(100_000);
        session.add_message(make_user_msg("ready for first request"));
        let session = Arc::new(AMutex::new(session));
        let mut thread = session.lock().await.thread.clone();
        assert!(thread.auto_compression_cap_pending);
        resolve_session_compression_cap(&app, &session, &mut thread, 128_000).await;
        assert_eq!(thread.auto_compression_cap, Some(90_000));
        assert!(!thread.auto_compression_cap_pending);
        crate::chat::trajectories::flush_trajectory_for_session(app, session.clone())
            .await
            .unwrap();
        let restored = crate::chat::trajectories::load_trajectory_for_chat(gcx, &thread.id)
            .await
            .unwrap();
        assert_eq!(restored.thread.auto_compression_cap, Some(90_000));
        assert!(!restored.thread.auto_compression_cap_pending);
        assert!(session.lock().await.draft_message.is_none());
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
    fn length_stop_with_tool_calls_does_not_request_compression() {
        let mut message = make_high_pressure_length_stop();
        message.tool_calls = make_assistant_with_tool_call("call_123", "cat").tool_calls;
        let messages = vec![make_user_msg("continue"), message.clone()];

        assert!(!is_high_pressure_length_stop(
            &message,
            &messages,
            100_000,
            false,
            refact_core::provider_types::ImageTokenMode::Provider
        ));
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
                        started_at_ms: None,
                        completed_at_ms: None,
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
                        started_at_ms: None,
                        completed_at_ms: None,
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
                        started_at_ms: None,
                        completed_at_ms: None,
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
                        started_at_ms: None,
                        completed_at_ms: None,
                    },
                ]),
                ..Default::default()
            },
            make_tool_msg("srvtoolu_123", "search results 1"),
            make_tool_msg("srvtoolu_456", "search results 2"),
        ];
        assert!(!tail_needs_assistant(&messages));
    }

    #[tokio::test]
    async fn priority_question_response_yields_before_next_generation() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let session = Arc::new(AMutex::new(ChatSession::new("question-boundary".into())));
        {
            let mut locked = session.lock().await;
            locked.set_runtime_state(SessionState::ExecutingTools, None);
            locked.messages = vec![make_assistant_with_tool_call("work", "cat")];
            assert_eq!(
                locked.enqueue_command(crate::chat::types::CommandRequest {
                    native_approval_guard: None,
                    supervision_guard: None,
                    client_request_id: "answer".into(),
                    priority: true,
                    command: crate::chat::types::ChatCommand::AnswerQuestions {
                        tool_call_id: "question".into(),
                        answers: vec![],
                        asked_at_ms: None,
                    },
                }),
                crate::chat::types::EnqueueCommandOutcome::Accepted
            );
            assert!(!locked.abort_flag.load(Ordering::SeqCst));
            assert!(!locked.yield_to_queued_commands_at_boundary());
            locked.messages.push(make_tool_msg("work", "finished"));
        }
        assert!(matches!(
            start_generation_iteration(&app, &session, &app.gcx).await,
            GenerationAttemptStart::Stop
        ));
        let mut locked = session.lock().await;
        assert_eq!(locked.runtime.state, SessionState::Idle);
        assert_eq!(locked.command_queue[0].client_request_id, "answer");
        assert_eq!(locked.messages[1].content.content_text_only(), "finished");
        assert!(locked.draft_message.is_none());
        locked.command_queue.clear();
        assert!(!locked.yield_to_queued_commands_at_boundary());
        locked.set_runtime_state(SessionState::Generating, None);
        assert_eq!(
            locked.enqueue_command(crate::chat::types::CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "late-answer".into(),
                priority: true,
                command: crate::chat::types::ChatCommand::AnswerQuestions {
                    tool_call_id: "late-question".into(),
                    answers: vec![],
                    asked_at_ms: None,
                },
            }),
            crate::chat::types::EnqueueCommandOutcome::Accepted
        );
        assert!(locked.start_stream().is_none());
        assert_eq!(locked.runtime.state, SessionState::Idle);
        assert_eq!(locked.command_queue[0].client_request_id, "late-answer");
        assert!(locked.draft_message.is_none());
    }

    #[test]
    fn test_priority_injection_guard_waits_for_tool_results() {
        let mut messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("call_123", "cat"),
        ];
        assert!(!latest_assistant_tool_call_window_closed(&messages));

        messages.push(make_tool_msg("call_123", "file content"));
        assert!(latest_assistant_tool_call_window_closed(&messages));
    }

    #[test]
    fn test_priority_injection_guard_ignores_stale_tool_results() {
        let messages = vec![
            make_user_msg("first"),
            make_assistant_with_tool_call("call_reused", "cat"),
            make_tool_msg("call_reused", "old result"),
            make_user_msg("second"),
            make_assistant_with_tool_call("call_reused", "cat"),
        ];

        assert!(!latest_assistant_tool_call_window_closed(&messages));
    }

    #[test]
    fn test_priority_injection_guard_waits_for_server_tool_placeholders() {
        let mut messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("srvtoolu_123", "web_search"),
        ];
        assert!(!latest_assistant_tool_call_window_closed(&messages));

        messages.push(make_tool_msg("srvtoolu_123", "server result placeholder"));
        assert!(latest_assistant_tool_call_window_closed(&messages));
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
    fn delta_batch_preserves_text_order_and_flushes_control_events() {
        let batches = batch_stream_delta_ops(vec![
            DeltaOp::AppendContent {
                text: "hel".to_string(),
            },
            DeltaOp::AppendContent {
                text: "lo".to_string(),
            },
            DeltaOp::AppendReasoning {
                text: "think".to_string(),
            },
            DeltaOp::SetToolCalls {
                tool_calls: vec![json!({"id": "tool-1"})],
            },
            DeltaOp::AppendContent {
                text: "after".to_string(),
            },
        ]);

        assert_eq!(batches.len(), 3);
        assert!(matches!(
            batches[0].as_slice(),
            [
                DeltaOp::AppendContent { text },
                DeltaOp::AppendReasoning { text: reasoning },
            ] if text == "hello" && reasoning == "think"
        ));
        assert!(matches!(
            batches[1].as_slice(),
            [DeltaOp::SetToolCalls { .. }]
        ));
        assert!(matches!(
            batches[2].as_slice(),
            [DeltaOp::AppendContent { text }] if text == "after"
        ));
    }

    #[test]
    fn delta_batch_caps_large_utf8_content_without_breaking_text() {
        let text = "😀".repeat((MAX_DELTA_TEXT_BYTES / 4) + 2);
        let batches = batch_stream_delta_ops(vec![DeltaOp::AppendContent {
            text: text.clone(),
        }]);
        let chunks = batches
            .into_iter()
            .flatten()
            .filter_map(|op| match op {
                DeltaOp::AppendContent {
                    text,
                } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(chunks.len() > 1);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.len() <= MAX_DELTA_TEXT_BYTES));
        assert_eq!(chunks.concat(), text);
    }

    fn length_stop_marker_msg() -> ChatMessage {
        length_stop_continue_instruction(LengthStopKind::PartialOutput)
    }

    #[test]
    fn length_stop_kind_classifies_empty_and_partial() {
        assert_eq!(
            length_stop_kind(&make_reasoning_token_limit_msg()),
            Some(LengthStopKind::EmptyOutput)
        );

        let mut partial = make_reasoning_token_limit_msg();
        partial.content = ChatContent::SimpleText("a long partial answer that was cut".repeat(4));
        assert_eq!(
            length_stop_kind(&partial),
            Some(LengthStopKind::PartialOutput)
        );

        let mut with_tools = make_reasoning_token_limit_msg();
        with_tools.tool_calls = make_assistant_with_tool_call("call_1", "cat").tool_calls;
        assert_eq!(length_stop_kind(&with_tools), None);

        let mut normal_finish = make_reasoning_token_limit_msg();
        normal_finish.finish_reason = Some("stop".to_string());
        assert_eq!(length_stop_kind(&normal_finish), None);

        let mut context_stop = make_reasoning_token_limit_msg();
        context_stop.finish_reason = Some("context_length_exceeded".to_string());
        assert_eq!(length_stop_kind(&context_stop), None);

        assert_eq!(length_stop_kind(&make_user_msg("hi")), None);
    }

    #[tokio::test]
    async fn empty_length_stop_low_pressure_retries_with_boost_and_marker() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut session = ChatSession::new("length-stop-retry".to_string());
        session.messages = vec![make_user_msg("continue"), make_low_pressure_length_stop()];
        let thread = session.thread.clone();
        let session_arc = Arc::new(AMutex::new(session));

        assert!(
            maybe_recover_after_length_stop(
                gcx,
                &session_arc,
                &thread,
                Some(100_000),
                refact_core::provider_types::ImageTokenMode::Provider,
            )
            .await
        );

        let session = session_arc.lock().await;
        assert_eq!(
            session.pending_max_new_tokens_boost,
            Some(LENGTH_STOP_BOOSTED_MAX_NEW_TOKENS)
        );
        assert!(!session
            .messages
            .iter()
            .any(|message| length_stop_kind(message) == Some(LengthStopKind::EmptyOutput)));
        let markers = session
            .messages
            .iter()
            .filter(|message| {
                message.role == "cd_instruction"
                    && message.tool_call_id == LENGTH_STOP_CONTINUE_MARKER
            })
            .count();
        assert_eq!(markers, 1);
    }

    #[tokio::test]
    async fn length_stop_recovery_preserves_legacy_budget_history_but_suppresses_it_on_wire() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut session = ChatSession::new("length-stop-trailing-budget".to_string());
        session.messages = vec![make_user_msg("continue"), make_low_pressure_length_stop()];
        session.add_message(ChatMessage {
            role: "cd_instruction".to_string(),
            tool_call_id: "token_budget_info".to_string(),
            content: ChatContent::SimpleText("budget".to_string()),
            ..Default::default()
        });
        let stored_notice = serde_json::to_value(&session.messages[2]).unwrap();
        let thread = session.thread.clone();
        let session_arc = Arc::new(AMutex::new(session));

        assert!(
            maybe_recover_after_length_stop(
                gcx,
                &session_arc,
                &thread,
                Some(100_000),
                refact_core::provider_types::ImageTokenMode::Provider,
            )
            .await
        );

        let session = session_arc.lock().await;
        assert_eq!(
            serde_json::to_value(&session.messages[1]).unwrap(),
            stored_notice
        );
        assert_eq!(
            session.pending_max_new_tokens_boost,
            Some(LENGTH_STOP_BOOSTED_MAX_NEW_TOKENS)
        );
        let wire = crate::chat::linearize::apply_summarization_linearize(session.messages.clone())
            .unwrap();
        assert!(!wire
            .iter()
            .any(|message| message.tool_call_id == "token_budget_info"));
        assert_eq!(wire.len(), 2);
        assert_eq!(
            session
                .messages
                .last()
                .map(|message| message.tool_call_id.clone()),
            Some(LENGTH_STOP_CONTINUE_MARKER.to_string())
        );
    }

    #[tokio::test]
    async fn partial_length_stop_keeps_message_and_appends_continue_marker() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut session = ChatSession::new("length-stop-partial".to_string());
        let mut partial = make_low_pressure_length_stop();
        partial.content = ChatContent::SimpleText("partial answer cut mid-".repeat(8));
        let partial_text = partial.content.content_text_only();
        session.messages = vec![make_user_msg("continue"), partial];
        let thread = session.thread.clone();
        let session_arc = Arc::new(AMutex::new(session));

        assert!(
            maybe_recover_after_length_stop(
                gcx,
                &session_arc,
                &thread,
                Some(100_000),
                refact_core::provider_types::ImageTokenMode::Provider,
            )
            .await
        );

        let session = session_arc.lock().await;
        assert!(session
            .messages
            .iter()
            .any(|message| message.content.content_text_only() == partial_text));
        assert_eq!(
            session
                .messages
                .last()
                .map(|message| message.tool_call_id.clone()),
            Some(LENGTH_STOP_CONTINUE_MARKER.to_string())
        );
    }

    #[tokio::test]
    async fn length_stop_recovery_exhausts_after_max_attempts_with_visible_notice() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut session = ChatSession::new("length-stop-exhausted".to_string());
        session.messages = vec![make_user_msg("continue")];
        for _ in 0..MAX_LENGTH_STOP_RECOVERY_ATTEMPTS {
            session.messages.push(length_stop_marker_msg());
        }
        session.messages.push(make_low_pressure_length_stop());
        let thread = session.thread.clone();
        let session_arc = Arc::new(AMutex::new(session));

        assert!(
            !maybe_recover_after_length_stop(
                gcx,
                &session_arc,
                &thread,
                Some(100_000),
                refact_core::provider_types::ImageTokenMode::Provider,
            )
            .await
        );

        let session = session_arc.lock().await;
        let last = session.messages.last().unwrap();
        assert_eq!(last.role, "error");
        assert!(crate::chat::diagnostics::is_ui_only_message(last));
        assert!(last
            .content
            .content_text_only()
            .contains("output token limit"));
    }

    #[tokio::test]
    async fn partial_length_stop_with_user_max_tokens_is_not_retried() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut session = ChatSession::new("length-stop-user-cap".to_string());
        let mut partial = make_low_pressure_length_stop();
        partial.content = ChatContent::SimpleText("partial answer".repeat(8));
        session.messages = vec![make_user_msg("continue"), partial];
        session.thread.max_tokens = Some(512);
        let thread = session.thread.clone();
        let before_len = session.messages.len();
        let session_arc = Arc::new(AMutex::new(session));

        assert!(
            !maybe_recover_after_length_stop(
                gcx,
                &session_arc,
                &thread,
                Some(100_000),
                refact_core::provider_types::ImageTokenMode::Provider,
            )
            .await
        );

        let session = session_arc.lock().await;
        assert_eq!(session.messages.len(), before_len);
        assert!(session.pending_max_new_tokens_boost.is_none());
    }
}
