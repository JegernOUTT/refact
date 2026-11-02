//! One owner for model-based context reconstruction. Raw history is append-only.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use serde::Serialize;
use tokio::sync::Mutex;
use crate::call_validation::ChatMessage;
use crate::global_context::GlobalContext;
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{ControlVerdict, HookPayload, get_project_dir_string};
use crate::chat::types::{
    ChatSession, ThreadParams, SessionState, CompressionPhase, CompressionReason,
    TrajectoryCommitIntent,
};
use refact_core::active_context::{
    active_context, legacy_rebuild_input, requires_explicit_rebuild, make_reconstruction_report,
    ReconstructionMetadata,
};
use refact_chat_api::role_projection::{fold_role_messages, ProjectedRole, ProjectionLimits};

#[derive(Clone, Debug)]
pub struct PendingContextRebuild {
    pub model: Option<String>,
    pub trigger: String,
}

pub fn request_rebuild(
    session: &mut ChatSession,
    request: PendingContextRebuild,
) -> Result<(), String> {
    if session.closed
        || compression_attempt_active(session)
        || session.pending_context_rebuild.is_some()
        || session.pending_mode_handoff.is_some()
    {
        return Err("A context operation is already pending or the session is closed".into());
    }
    session.pending_context_rebuild = Some(request);
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ManualCompressionPreview {
    pub eligible: bool,
    pub trajectory_version: Option<u64>,
    pub resolved_model: Option<String>,
    pub context_window: Option<usize>,
    pub source_messages: usize,
    pub approximate_source_tokens: usize,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ManualCompressionApplyResult {
    pub applied: bool,
    pub resolved_model: Option<String>,
    pub context_window: Option<usize>,
    pub source_messages: usize,
    pub approximate_source_tokens: usize,
    pub before_message_count: usize,
    pub after_message_count: usize,
    pub before_approx_tokens: usize,
    pub after_approx_tokens: usize,
    pub reason: Option<String>,
}

pub(crate) fn compression_attempt_active(session: &ChatSession) -> bool {
    session.active_compression_attempt.is_some()
        || session.is_compressing
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

fn pending_tools(messages: &[ChatMessage]) -> bool {
    let mut pending = std::collections::HashSet::new();
    for message in messages {
        for call in message.tool_calls.iter().flatten() {
            pending.insert(call.id.clone());
        }
        if matches!(message.role.as_str(), "tool" | "diff" | "context_file") {
            pending.remove(&message.tool_call_id);
        }
    }
    !pending.is_empty()
}

pub(crate) const SKIP_SESSION_CHANGED: &str = "Context rebuild canceled: session changed";
pub(crate) const SKIP_CHANGED_DURING_HOOK: &str = "Chat changed while PreCompact hook was running";
pub(crate) const SKIP_NOT_IDLE: &str = "Rebuild requires an idle session with no active stream";
pub(crate) const SKIP_OTHER_PENDING: &str = "Another context operation is pending";
pub(crate) const SKIP_ALREADY_ATTEMPTED: &str = "Rebuild was already attempted for this context";
pub(crate) const SKIP_PENDING_TOOLS: &str =
    "Rebuild requires nonempty context and completed tool results";
const SKIP_CANCELLED: &str = "Context reconstruction cancelled";

pub(crate) fn is_routine_rebuild_skip(error: &str) -> bool {
    [
        SKIP_SESSION_CHANGED,
        SKIP_CHANGED_DURING_HOOK,
        SKIP_NOT_IDLE,
        SKIP_OTHER_PENDING,
        SKIP_ALREADY_ATTEMPTED,
        SKIP_PENDING_TOOLS,
        SKIP_CANCELLED,
    ]
    .contains(&error)
}

pub(crate) const ABANDONED_NOTE_SOURCE: &str = "chat.context_rebuild";
pub(crate) const ABANDONED_RESULT_EXTRA: &str = "abandoned_tool_call";
pub(crate) const NORMALIZATION_REPORT_EXTRA: &str = "reconstruction_input_normalization";
const MAX_LISTED_ABANDONED_CALLS: usize = 32;
pub(crate) const ABANDONED_OUTCOME_TEXT: &str = "Outcome unknown: this historical tool call has no recorded result and was superseded by a later turn. Its side effects were neither confirmed nor ruled out; do not assume it succeeded or that it never ran.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AbandonedCall {
    pub tool_call_id: String,
    pub name: String,
    pub assistant_message_id: String,
}

pub(crate) fn live_tool_call_ids(session: &ChatSession) -> std::collections::HashSet<String> {
    let mut ids: std::collections::HashSet<String> = session
        .runtime
        .pause_reasons
        .iter()
        .map(|reason| reason.tool_call_id.clone())
        .collect();
    if let Some((_, paused)) = &session.confirmation_paused_at {
        ids.extend(paused.iter().cloned());
    }
    ids.extend(
        session
            .pending_questions
            .iter()
            .filter(|set| set.status == refact_chat_api::PendingQuestionStatus::Pending)
            .map(|set| set.tool_call_id.clone()),
    );
    if matches!(
        session.runtime.state,
        SessionState::ExecutingTools
            | SessionState::Paused
            | SessionState::WaitingUserInput
            | SessionState::WaitingIde
    ) {
        let messages = active_context(&session.messages)
            .map(|v| v.messages)
            .unwrap_or_else(|_| session.messages.clone());
        if let Some(at) = messages
            .iter()
            .rposition(|m| m.role == "assistant" && m.tool_calls.is_some())
        {
            let answered: std::collections::HashSet<_> = messages[at + 1..]
                .iter()
                .filter(|m| matches!(m.role.as_str(), "tool" | "diff" | "context_file"))
                .map(|m| m.tool_call_id.as_str())
                .collect();
            ids.extend(
                messages[at]
                    .tool_calls
                    .iter()
                    .flatten()
                    .filter(|c| !answered.contains(c.id.as_str()))
                    .map(|c| c.id.clone()),
            );
        }
    }
    ids
}

pub(crate) fn explicit_context_ready(session: &ChatSession) -> bool {
    session.runtime.state != SessionState::Generating
        && session.draft_message.is_none()
        && session.draft_usage.is_none()
        && session.stream_started_at.is_none()
        && !session.closed
}

pub(crate) fn split_live_suffix(
    messages: &[ChatMessage],
    live_ids: &std::collections::HashSet<String>,
) -> (Vec<ChatMessage>, Vec<ChatMessage>) {
    let at = messages
        .iter()
        .position(|m| {
            m.tool_calls
                .iter()
                .flatten()
                .any(|c| live_ids.contains(&c.id))
        })
        .unwrap_or(messages.len());
    (messages[..at].to_vec(), messages[at..].to_vec())
}

pub(crate) fn place_report_before_live_suffix(
    messages: &[ChatMessage],
    live_ids: &std::collections::HashSet<String>,
    report: &mut ChatMessage,
) -> Result<usize, String> {
    if live_ids.is_empty() {
        return Ok(messages.len());
    }
    let active = match active_context(messages) {
        Ok(active) => active,
        Err(refact_core::active_context::ActiveContextError::NeedsExplicitRebuild) => {
            let start = messages
                .iter()
                .rposition(|m| refact_core::active_context::is_reconstruction_report(m))
                .map_or(0, |index| index + 1);
            if start > 0 {
                let previous = active_context(&messages[..start]).map_err(|e| e.to_string())?;
                if previous.messages.iter().any(|m| {
                    m.tool_calls
                        .iter()
                        .flatten()
                        .any(|c| live_ids.contains(&c.id))
                }) {
                    return place_report_before_live_suffix(&messages[..start], live_ids, report);
                }
            }
            return Ok(messages[start..]
                .iter()
                .position(|m| {
                    m.tool_calls
                        .iter()
                        .flatten()
                        .any(|c| live_ids.contains(&c.id))
                })
                .map(|index| index + start)
                .unwrap_or(messages.len()));
        }
        Err(error) => return Err(error.to_string()),
    };
    let start = active
        .messages
        .iter()
        .position(|m| {
            m.tool_calls
                .iter()
                .flatten()
                .any(|c| live_ids.contains(&c.id))
        })
        .unwrap_or(active.messages.len());
    let mut at = messages.len();
    let mut embedded = Vec::new();
    for (message, origin) in active.messages[start..]
        .iter()
        .zip(&active.origins[start..])
    {
        match origin {
            refact_core::active_context::MessageOrigin::Stored { message_index } => {
                at = *message_index;
                break;
            }
            refact_core::active_context::MessageOrigin::ReportPayload { .. } => {
                embedded.push(message.clone())
            }
        }
    }
    if !embedded.is_empty() {
        let payload = report
            .extra
            .get_mut("compression_report")
            .and_then(|r| r.pointer_mut("/payload/messages"))
            .and_then(serde_json::Value::as_array_mut)
            .ok_or("Missing reconstruction payload")?;
        let existing: std::collections::HashSet<String> = payload
            .iter()
            .filter_map(|m| {
                m.get("message_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        embedded.retain(|message| !existing.contains(&message.message_id));
        payload.extend(
            embedded
                .into_iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(at)
}

pub(crate) fn live_state_fingerprint(session: &ChatSession) -> serde_json::Value {
    serde_json::json!({"state": session.runtime.state, "pause_reasons": session.runtime.pause_reasons,
        "paused_message_index": session.runtime.paused_message_index,
        "questions": session.pending_questions, "approvals": session.approval_wrapper_calls,
        "confirmation": session.confirmation_paused_at.as_ref().map(|(_, ids)| { let mut ids: Vec<_> = ids.iter().collect(); ids.sort(); ids }),
        "abort": session.abort_flag.load(Ordering::SeqCst), "interrupt": session.user_interrupt_flag.load(Ordering::SeqCst)})
}

fn unique_message_id(base: String, used: &mut std::collections::HashSet<String>) -> String {
    let mut candidate = base.clone();
    let mut n = 1usize;
    while used.contains(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    used.insert(candidate.clone());
    candidate
}

pub(crate) fn normalize_rebuild_input(
    messages: &[ChatMessage],
    live_ids: &std::collections::HashSet<String>,
) -> (Vec<ChatMessage>, Vec<AbandonedCall>) {
    let answered: std::collections::HashSet<&str> = messages
        .iter()
        .filter(|m| matches!(m.role.as_str(), "tool" | "diff" | "context_file"))
        .map(|m| m.tool_call_id.as_str())
        .collect();
    let Some(last_assistant) = messages.iter().rposition(|m| m.role == "assistant") else {
        return (messages.to_vec(), Vec::new());
    };
    let mut used_ids: std::collections::HashSet<String> =
        messages.iter().map(|m| m.message_id.clone()).collect();
    let mut abandoned = Vec::new();
    let mut output = Vec::with_capacity(messages.len());
    let mut pending: Vec<ChatMessage> = Vec::new();
    let mut window: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (index, message) in messages.iter().enumerate() {
        let in_window = matches!(message.role.as_str(), "tool" | "diff" | "context_file")
            && window.contains(message.tool_call_id.as_str());
        if !in_window {
            output.append(&mut pending);
            window.clear();
        }
        output.push(message.clone());
        if message.role != "assistant" || index >= last_assistant {
            continue;
        }
        let calls = message.tool_calls.iter().flatten();
        let orphans: Vec<_> = calls
            .filter(|c| !answered.contains(c.id.as_str()) && !live_ids.contains(&c.id))
            .collect();
        if orphans.is_empty() {
            continue;
        }
        window = message
            .tool_calls
            .iter()
            .flatten()
            .map(|c| c.id.as_str())
            .collect();
        for call in orphans {
            let id = unique_message_id(format!("abandoned-result-{}", call.id), &mut used_ids);
            let mut result = ChatMessage::new("tool".into(), ABANDONED_OUTCOME_TEXT.into());
            result.message_id = id;
            result.tool_call_id = call.id.clone();
            result.tool_failed = Some(true);
            result.extra.insert(
                ABANDONED_RESULT_EXTRA.into(),
                serde_json::json!({"outcome": "unknown", "source": ABANDONED_NOTE_SOURCE}),
            );
            pending.push(result);
            abandoned.push(AbandonedCall {
                tool_call_id: call.id.clone(),
                name: call.function.name.clone(),
                assistant_message_id: message.message_id.clone(),
            });
        }
    }
    output.append(&mut pending);
    (output, abandoned)
}

fn abandoned_summary(abandoned: &[AbandonedCall]) -> serde_json::Value {
    serde_json::json!({
        "reason": "abandoned_tool_call",
        "outcome": "unknown",
        "count": abandoned.len(),
        "tool_calls": abandoned.iter().take(MAX_LISTED_ABANDONED_CALLS).collect::<Vec<_>>(),
        "truncated": abandoned.len() > MAX_LISTED_ABANDONED_CALLS,
    })
}

pub(crate) fn abandoned_calls_note(abandoned: &[AbandonedCall]) -> Option<ChatMessage> {
    if abandoned.is_empty() {
        return None;
    }
    let mut names: Vec<String> = abandoned
        .iter()
        .take(MAX_LISTED_ABANDONED_CALLS)
        .map(|c| format!("{} ({})", c.name, c.tool_call_id))
        .collect();
    if abandoned.len() > MAX_LISTED_ABANDONED_CALLS {
        names.push(format!(
            "and {} more",
            abandoned.len() - MAX_LISTED_ABANDONED_CALLS
        ));
    }
    Some(crate::chat::internal_roles::event(
        crate::chat::internal_roles::EventSubkind::CancellationNote,
        ABANDONED_NOTE_SOURCE,
        abandoned_summary(abandoned),
        format!(
            "Historical tool call(s) abandoned without a recorded result: {}. Outcome unknown — side effects were neither confirmed nor ruled out; verify before relying on them.",
            names.join(", ")
        ),
    ))
}

fn message_text_for_units(message: &ChatMessage) -> String {
    let mut text = if message.role == "event" {
        refact_llm::adapters::render_extra::render_event_message(message)
    } else {
        refact_core::edit_results::project_edit_result(message)
            .content
            .content_text_only()
    };
    if let Some(reasoning) = &message.reasoning_content {
        text.push_str(reasoning);
    }
    for call in message.tool_calls.iter().flatten() {
        text.push_str(&call.function.name);
        text.push_str(&call.function.arguments);
    }
    text
}

fn is_mandatory_control(message: &ChatMessage) -> bool {
    matches!(message.role.as_str(), "goal" | "plan")
        || refact_agentic::mode_transition::is_live_agents_snapshot(message)
        || message
            .extra
            .get("event")
            .and_then(|e| e.get("subkind"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|k| matches!(k, "plan_delta" | "goal_delta" | "goal_pursuit"))
}

pub(crate) fn token_cap_to_symbol_budget(
    messages: &[ChatMessage],
    cap_tokens: usize,
    tokenizer: Option<Arc<tokenizers::Tokenizer>>,
) -> usize {
    let (mut control_symbols, mut control_tokens) = (0usize, 0usize);
    let (mut symbols, mut tokens) = (0usize, 0usize);
    for message in messages {
        let text = message_text_for_units(message);
        let mut s = refact_agentic::mode_transition::message_symbols(message);
        if refact_agentic::mode_transition::is_live_agents_snapshot(message) {
            s = s.saturating_add(
                message
                    .extra
                    .get("event")
                    .map_or(0, |event| event.to_string().chars().count()),
            );
        }
        let t = crate::tokens::count_text_tokens(tokenizer.clone(), &text)
            .unwrap_or_else(|_| text.len() / 3 + 1);
        if is_mandatory_control(message) {
            control_symbols = control_symbols.saturating_add(s);
            control_tokens = control_tokens.saturating_add(t);
        } else {
            symbols = symbols.saturating_add(s);
            tokens = tokens.saturating_add(t);
        }
    }
    let Some(rest_tokens) = cap_tokens.checked_sub(control_tokens) else {
        return control_symbols;
    };
    let rest_symbols = if tokens == 0 {
        rest_tokens
    } else {
        ((rest_tokens as u128 * symbols as u128) / tokens as u128).min(usize::MAX as u128) as usize
    };
    control_symbols.saturating_add(rest_symbols.min(symbols.saturating_sub(1)))
}

pub(crate) fn is_abandoned_note(message: &ChatMessage) -> bool {
    message.role == "event"
        && message.extra.get("event").is_some_and(|e| {
            e.get("subkind").and_then(serde_json::Value::as_str) == Some("cancellation_note")
                && e.get("source").and_then(serde_json::Value::as_str)
                    == Some(ABANDONED_NOTE_SOURCE)
        })
}

fn noted_call_ids(note: &ChatMessage) -> impl Iterator<Item = &str> {
    note.extra
        .get("event")
        .and_then(|e| e.pointer("/payload/tool_calls"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("tool_call_id").and_then(serde_json::Value::as_str))
}

pub(crate) fn rebuild_notes(
    source: &[ChatMessage],
    abandoned: &[AbandonedCall],
) -> Vec<ChatMessage> {
    let mut seen = std::collections::HashSet::new();
    let mut notes: Vec<ChatMessage> = source
        .iter()
        .filter(|m| is_abandoned_note(m) && seen.insert(m.message_id.clone()))
        .cloned()
        .collect();
    let noted: std::collections::HashSet<String> = notes
        .iter()
        .flat_map(|n| noted_call_ids(n).map(str::to_owned).collect::<Vec<_>>())
        .collect();
    let fresh: Vec<AbandonedCall> = abandoned
        .iter()
        .filter(|c| !noted.contains(&c.tool_call_id))
        .cloned()
        .collect();
    notes.extend(abandoned_calls_note(&fresh));
    notes
}

fn notes_tokens(notes: &[ChatMessage], tokenizer: Option<Arc<tokenizers::Tokenizer>>) -> usize {
    notes
        .iter()
        .map(|note| {
            let text = format!(
                "{}{}",
                note.content.content_text_only(),
                serde_json::to_string(&note.extra).unwrap_or_default()
            );
            crate::tokens::count_text_tokens(tokenizer.clone(), &text)
                .unwrap_or_else(|_| text.len() / 3 + 1)
                .saturating_add(16)
        })
        .fold(0usize, usize::saturating_add)
}

fn raw_rebuild_input(messages: &[ChatMessage], explicit: bool) -> Result<Vec<ChatMessage>, String> {
    if explicit && requires_explicit_rebuild(messages) {
        legacy_rebuild_input(messages)
    } else {
        active_context(messages).map(|v| v.messages)
    }
    .map_err(|e| e.to_string())
}

pub(crate) fn with_abandoned_notes(
    mut output: Vec<ChatMessage>,
    notes: Vec<ChatMessage>,
) -> Vec<ChatMessage> {
    let ids: std::collections::HashSet<String> =
        notes.iter().map(|n| n.message_id.clone()).collect();
    output.retain(|m| !is_abandoned_note(m) && !ids.contains(&m.message_id));
    let at = output
        .iter()
        .rposition(|m| m.role == "user")
        .unwrap_or(output.len());
    output.splice(at..at, notes);
    output
}

pub(crate) fn fold_controls_for_rebuild(
    messages: Vec<ChatMessage>,
    limits: ProjectionLimits,
) -> Vec<ChatMessage> {
    let messages = fold_role_messages(ProjectedRole::Plan, &messages, limits);
    fold_role_messages(ProjectedRole::Goal, &messages, limits)
}

fn idle(session: &ChatSession) -> bool {
    session.runtime.state == SessionState::Idle
        && session.draft_message.is_none()
        && session.draft_usage.is_none()
        && session.stream_started_at.is_none()
        && !session.closed
        && !session.abort_flag.load(Ordering::SeqCst)
}

pub(super) fn status(
    session: &mut ChatSession,
    phase: CompressionPhase,
    reason: Option<CompressionReason>,
) {
    let running = matches!(
        phase,
        CompressionPhase::Checking | CompressionPhase::Running
    );
    session.is_compressing = running;
    session.runtime.is_compressing = running;
    session.compression_phase = Some(phase);
    session.runtime.compression_phase = Some(phase);
    session.compression_reason = reason;
    session.runtime.compression_reason = reason;
    if !running {
        session.active_compression_attempt = None;
        session.compression_attempt_started_at_ms = None;
        session.compression_abort_flag = None;
        session.queue_notify.notify_waiters();
    }
    session.refresh_goal_runtime_mirror();
    let event = session.runtime_update_event(
        session.runtime.state,
        session.runtime.error.clone(),
        running,
        Some(phase),
        reason,
    );
    session.emit(event);
}
pub(crate) fn emit_compression_skipped_status(
    session: &mut ChatSession,
    reason: CompressionReason,
) {
    if !compression_attempt_active(session) {
        status(session, CompressionPhase::Skipped, Some(reason));
    }
}

struct AttemptGuard {
    session: Arc<Mutex<ChatSession>>,
    attempt: u64,
    abort: Arc<AtomicBool>,
    gcx: Option<Arc<GlobalContext>>,
}
impl Drop for AttemptGuard {
    fn drop(&mut self) {
        self.abort.store(true, Ordering::SeqCst);
        let session = self.session.clone();
        let attempt = self.attempt;
        let gcx = self.gcx.take();
        let release = move |s: &mut ChatSession| {
            terminal_attempt(
                s,
                attempt,
                CompressionPhase::Failed,
                Some(CompressionReason::TransientFailure),
            )
        };
        let batch = match session.try_lock() {
            Ok(mut s) => Some(release(&mut s)),
            Err(_) => None,
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let batch = match batch {
                    Some(batch) => batch,
                    None => release(&mut *session.lock().await),
                };
                if let Some(gcx) = gcx {
                    finish_held_deliveries(gcx, session, batch).await;
                }
            });
        }
    }
}

fn terminal_attempt(
    session: &mut ChatSession,
    attempt: u64,
    phase: CompressionPhase,
    reason: Option<CompressionReason>,
) -> Option<crate::chat::delivery::DeliveryDrainBatch> {
    if session.active_compression_attempt != Some(attempt) {
        return None;
    }
    status(session, phase, reason);
    Some(crate::chat::delivery::collect_boundary_deliveries(session))
}

async fn finish_held_deliveries(
    gcx: Arc<GlobalContext>,
    session: Arc<Mutex<ChatSession>>,
    batch: Option<crate::chat::delivery::DeliveryDrainBatch>,
) {
    if let Some(batch) = batch {
        let finalizer = tokio::spawn(async move {
            let app = crate::app_state::AppState::from_gcx(gcx).await;
            crate::chat::delivery::finalize_boundary_deliveries(app, session, batch).await;
        });
        let _ = finalizer.await;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RebuildSource {
    messages: Vec<u8>,
    thread: Vec<u8>,
    task_goal: Vec<u8>,
    live_state: serde_json::Value,
}

impl RebuildSource {
    pub(crate) fn capture(session: &ChatSession) -> Result<Self, String> {
        Ok(Self {
            live_state: live_state_fingerprint(session),
            messages: serde_json::to_vec(&session.messages).map_err(|e| e.to_string())?,
            thread: serde_json::to_vec(&session.thread).map_err(|e| e.to_string())?,
            task_goal: serde_json::to_vec(&(
                &session.task_goal_messages,
                session
                    .goal
                    .as_ref()
                    .map(|goal| (&goal.owner, goal.revision)),
            ))
            .map_err(|e| e.to_string())?,
        })
    }

    fn matches(&self, session: &ChatSession) -> bool {
        Self::capture(session).is_ok_and(|current| current == *self)
    }
}

async fn model_tokenizer(
    gcx: Arc<GlobalContext>,
    model: &str,
) -> Option<Arc<tokenizers::Tokenizer>> {
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .ok()?;
    let record = crate::caps::resolve_chat_model(caps, model).ok()?;
    crate::tokens::cached_tokenizer(gcx, &record.base)
        .await
        .ok()
        .flatten()
}

fn estimated_tokens(
    messages: &[ChatMessage],
    tokenizer: Option<Arc<tokenizers::Tokenizer>>,
    fresh_usage: bool,
) -> usize {
    let Ok(visible) = crate::chat::linearize::apply_summarization_linearize(messages.to_vec())
    else {
        return usize::MAX;
    };
    let count = |messages: &[ChatMessage]| {
        let structural = refact_chat_history::history_limit::compute_context_budget_for_image_mode(
            messages,
            usize::MAX,
            refact_core::provider_types::ImageTokenMode::Provider,
        )
        .used_tokens_estimate;
        let tokens = messages
            .iter()
            .map(|message| {
                let content = if message.role == "event" {
                    crate::tokens::count_text_tokens(
                        tokenizer.clone(),
                        &refact_llm::adapters::render_extra::render_event_message(message),
                    )
                    .unwrap_or(0)
                } else {
                    message
                        .content
                        .count_tokens(tokenizer.clone(), &None)
                        .unwrap_or(0)
                        .max(0) as usize
                };
                let calls = message
                    .tool_calls
                    .as_ref()
                    .map(|calls| serde_json::to_string(calls).unwrap_or_default())
                    .unwrap_or_default();
                let calls = crate::tokens::count_text_tokens(tokenizer.clone(), &calls)
                    .unwrap_or_else(|_| calls.len() / 3 + 1);
                content.saturating_add(calls).saturating_add(8)
            })
            .fold(0usize, usize::saturating_add);
        structural.max(tokens)
    };
    let estimate = count(&visible);
    if !fresh_usage {
        return estimate;
    }
    visible
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, message)| {
            if message.role != "assistant" {
                return None;
            }
            let usage = message.usage.as_ref()?;
            let input = usage
                .prompt_tokens
                .saturating_add(usage.cache_read_tokens.unwrap_or(0))
                .saturating_add(usage.cache_creation_tokens.unwrap_or(0));
            (input > 0).then(|| input.saturating_add(count(&visible[index..])))
        })
        .unwrap_or(estimate)
}

fn at_auto_cap(used: usize, cap: usize) -> bool {
    cap > 0 && used >= cap
}

/// A prefix can budget the next request only when both mandatory parts are present
/// and the canonical tools are the array the wire adapters serialize.
fn budget_prefix_is_usable(prefix: &refact_chat_api::FrozenRequestPrefix) -> bool {
    prefix.schema_version == 1
        && crate::chat::trajectories::frozen_prefix_is_complete(prefix)
        && prefix
            .tools_canonical
            .as_ref()
            .is_some_and(serde_json::Value::is_array)
}

/// A snapshot-only evaluator: never generates prompts, runs tools, or mutates a session.
/// Missing mandatory request components are an unavailable estimate, not zero tokens.
fn payload_request_cap(
    thread: &ThreadParams,
    n_ctx: usize,
    output_reserve: usize,
    tokenizer: Option<Arc<tokenizers::Tokenizer>>,
) -> Result<usize, String> {
    let prefix = thread
        .frozen_request_prefix
        .as_ref()
        .filter(|p| p.schema_version == 1)
        .ok_or("Next-request budget unavailable: no frozen request prefix")?;
    if !budget_prefix_is_usable(prefix) {
        return Err("Next-request budget unavailable: incomplete frozen request prefix".into());
    }
    let system = prefix
        .system_prompt
        .as_ref()
        .ok_or("Next-request budget unavailable: missing system prompt")?;
    let tools = prefix
        .tools_canonical
        .as_ref()
        .filter(|v| v.is_array())
        .ok_or("Next-request budget unavailable: missing canonical tools")?;
    let tools = serde_json::to_string(tools).map_err(|e| e.to_string())?;
    let count = |text: &str| -> Result<usize, String> {
        crate::tokens::count_text_tokens(tokenizer.clone(), text)
    };
    let mandatory = count(system)?
        .saturating_add(count(&tools)?)
        .saturating_add(1024);
    thread
        .context_tokens_cap
        .filter(|v| *v > 0)
        .unwrap_or(n_ctx)
        .min(n_ctx)
        .checked_sub(output_reserve)
        .and_then(|v| v.checked_sub(mandatory))
        .filter(|v| *v > 0)
        .ok_or_else(|| "Mandatory system/tools and output reserve exceed request cap".into())
}

fn validate_output(
    source: &[ChatMessage],
    output: &[ChatMessage],
    cap: usize,
    tokenizer: Option<Arc<tokenizers::Tokenizer>>,
) -> Result<ReconstructionMetrics, String> {
    if !output
        .iter()
        .any(|m| !m.content.content_text_only().trim().is_empty())
        || pending_tools(output)
    {
        return Err("Reconstruction produced no useful completed context".into());
    }
    let before = estimated_tokens(source, tokenizer.clone(), false);
    let after = estimated_tokens(output, tokenizer, false);
    if before == usize::MAX || after == usize::MAX || after >= before || after > cap {
        return Err(format!("Reconstruction must be smaller and fit request cap (before={before}, after={after}, cap={cap})"));
    }
    Ok(ReconstructionMetrics {
        messages_before: source.len(),
        messages_after: output.len(),
        tokens_before: before,
        tokens_after: after,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReconstructionMetrics {
    messages_before: usize,
    messages_after: usize,
    tokens_before: usize,
    tokens_after: usize,
}

impl ReconstructionMetrics {
    fn to_json(self) -> serde_json::Value {
        let saved = self.tokens_before.saturating_sub(self.tokens_after);
        let reduction_percent = if self.tokens_before == 0 {
            0
        } else {
            saved.saturating_mul(100) / self.tokens_before
        };
        serde_json::json!({
            "messages_before": self.messages_before,
            "messages_after": self.messages_after,
            "tokens_before": self.tokens_before,
            "tokens_after": self.tokens_after,
            "estimated_tokens_saved": saved,
            "reduction_percent": reduction_percent,
        })
    }
}

/// Shared transaction seam: persistence must succeed before publishing any report.
async fn commit_report<F, Fut>(
    session: &mut ChatSession,
    mut report: ChatMessage,
    source: &RebuildSource,
    attempt: u64,
    abort: &AtomicBool,
    persist: F,
) -> Result<(), String>
where
    F: FnOnce(crate::chat::trajectories::TrajectorySnapshot) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    if session.active_compression_attempt != Some(attempt)
        || !explicit_context_ready(session)
        || abort.load(Ordering::SeqCst)
        || !source.matches(session)
    {
        return Err(SKIP_SESSION_CHANGED.into());
    }
    let version = session.trajectory_version.saturating_add(1);
    let mut snapshot = crate::chat::trajectories::trajectory_snapshot_from_session(session);
    let live_ids = live_tool_call_ids(session);
    let at = place_report_before_live_suffix(&session.messages, &live_ids, &mut report)?;
    let snapshot_at = session.messages[..at]
        .iter()
        .filter(|message| crate::chat::trajectories::retain_in_snapshot(message))
        .count();
    snapshot.messages.insert(snapshot_at, report.clone());
    active_context(&snapshot.messages).map_err(|e| e.to_string())?;
    snapshot.version = version;
    snapshot.previous_response_id = None;
    snapshot.frozen_request_prefix = None;
    snapshot.claude_code_identity = None;
    persist(snapshot).await?;
    session.messages.insert(at, report);
    if let Some(index) = session.runtime.paused_message_index.as_mut() {
        if *index >= at {
            *index += 1;
        }
    }
    session.touch();
    session.thread.previous_response_id = None;
    session.thread.frozen_request_prefix = None;
    session.thread.claude_code_identity = None;
    session.openai_codex_websocket = Default::default();
    session.reset_cache_guard_snapshot();
    session.provider_usage_stale = true;
    session.trajectory_version = version;
    session.trajectory_committed_version = version;
    session.trajectory_dirty = false;
    session.trajectory_save_error = None;
    session.last_rebuild_attempt_version = Some(version);
    let event = session.snapshot();
    session.emit(event);
    Ok(())
}

fn bounded_precompact_reason(reason: &str, max_bytes: usize) -> String {
    let message = format!("blocked by PreCompact hook: {reason}");
    if message.len() <= max_bytes {
        return message;
    }
    let mut end = max_bytes;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_string()
}

pub(super) async fn precompact_control(
    gcx: Arc<GlobalContext>,
    thread: &ThreadParams,
    trigger: &str,
    messages_before: usize,
) -> ControlVerdict {
    if !gcx.hooks_config.enabled {
        return ControlVerdict::Allow;
    }
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let project_dir = match &thread.worktree {
        Some(worktree) => worktree.root.to_string_lossy().into_owned(),
        None => get_project_dir_string(app.clone()).await,
    };
    let mut payload = HookPayload::new(&HookEvent::PreCompact, thread.id.clone(), project_dir);
    payload.extra.insert(
        "trigger".into(),
        serde_json::Value::String(trigger.to_string()),
    );
    payload.extra.insert(
        "messages_before".into(),
        serde_json::Value::from(messages_before),
    );
    crate::ext::hooks_runner::run_control_hooks(app, HookEvent::PreCompact, payload).await
}

pub(super) async fn postcompact_observation(
    gcx: Arc<GlobalContext>,
    thread: &ThreadParams,
    trigger: &str,
    messages_before: usize,
    messages_after: usize,
) {
    if !gcx.hooks_config.enabled {
        return;
    }
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let project_dir = match &thread.worktree {
        Some(worktree) => worktree.root.to_string_lossy().into_owned(),
        None => get_project_dir_string(app.clone()).await,
    };
    let mut payload = HookPayload::new(&HookEvent::PostCompact, thread.id.clone(), project_dir);
    payload.extra.insert(
        "trigger".into(),
        serde_json::Value::String(trigger.to_string()),
    );
    payload.extra.insert(
        "messages_before".into(),
        serde_json::Value::from(messages_before),
    );
    payload.extra.insert(
        "messages_after".into(),
        serde_json::Value::from(messages_after),
    );
    payload.extra.insert(
        "outcome".into(),
        serde_json::Value::String("applied".into()),
    );
    crate::ext::hooks_runner::run_observation_hooks(app, HookEvent::PostCompact, payload).await;
}

pub(super) async fn commit_report_then_observe<F, Fut>(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<Mutex<ChatSession>>,
    report: ChatMessage,
    source: &RebuildSource,
    attempt: u64,
    thread: &ThreadParams,
    abort: &AtomicBool,
    trigger: &str,
    messages_before: usize,
    persist: F,
) -> Result<(), String>
where
    F: FnOnce(crate::chat::trajectories::TrajectorySnapshot) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let (result, messages_after, batch) = {
        let mut session = session_arc.lock().await;
        let result = commit_report(&mut session, report, source, attempt, abort, persist).await;
        let messages_after = active_context(&session.messages)
            .map(|view| view.messages.len())
            .unwrap_or(session.messages.len());
        let batch = terminal_attempt(
            &mut session,
            attempt,
            if result.is_ok() {
                CompressionPhase::Applied
            } else {
                CompressionPhase::Failed
            },
            result.as_ref().err().map(|error| {
                if is_routine_rebuild_skip(error) {
                    CompressionReason::SourceChanged
                } else {
                    CompressionReason::TransientFailure
                }
            }),
        );
        (result, messages_after, batch)
    };
    finish_held_deliveries(gcx.clone(), session_arc.clone(), batch).await;
    if result.is_ok() {
        postcompact_observation(gcx, thread, trigger, messages_before, messages_after).await;
    }
    result
}

async fn resolve_model(
    gcx: Arc<GlobalContext>,
    thread: &ThreadParams,
    requested: Option<&str>,
) -> Result<(String, usize), String> {
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .map_err(|e| e.message)?;
    let model = requested
        .filter(|m| !m.trim().is_empty())
        .unwrap_or(&thread.model);
    let model = if model.is_empty() {
        caps.defaults.chat_default_model.as_str()
    } else {
        model
    };
    let record = crate::caps::resolve_chat_model(caps.clone(), model).map_err(|e| e.to_string())?;
    Ok((model.to_string(), record.base.n_ctx))
}

pub async fn preview_manual_context_rebuild(
    gcx: Arc<GlobalContext>,
    state: SessionState,
    compression_active: bool,
    messages: &[ChatMessage],
    live_ids: &std::collections::HashSet<String>,
    thread: &ThreadParams,
    requested_model: Option<&str>,
) -> ManualCompressionPreview {
    let input = raw_rebuild_input(messages, true);
    let mut preview = ManualCompressionPreview {
        eligible: false,
        trajectory_version: None,
        resolved_model: None,
        context_window: None,
        source_messages: 0,
        approximate_source_tokens: 0,
        reason: None,
    };
    let input = match input {
        Ok(v) => v,
        Err(e) => {
            preview.reason = Some(e);
            return preview;
        }
    };
    preview.source_messages = input.len();
    preview.approximate_source_tokens = crate::chat::trajectory_ops::approx_token_count(&input);
    let (prefix, _) = split_live_suffix(&input, live_ids);
    let (normalized, _) = normalize_rebuild_input(&prefix, live_ids);
    if state == SessionState::Generating
        || compression_active
        || pending_tools(&normalized)
        || prefix.is_empty()
    {
        preview.reason =
            Some("Rebuild requires a non-streaming session and completed context prefix".into());
        return preview;
    }
    match resolve_model(gcx, thread, requested_model).await {
        Ok((model, n_ctx)) => {
            preview.resolved_model = Some(model);
            preview.context_window = Some(n_ctx);
            preview.eligible = true;
        }
        Err(e) => preview.reason = Some(e),
    }
    preview
}

/// Build off-lock, then CAS and durably append a report. The disk-only commit is
/// serialized with the ordinary writer. No provider/network await holds a lock.
pub async fn rebuild_session(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<Mutex<ChatSession>>,
    request: PendingContextRebuild,
    expected_version: Option<u64>,
    explicit: bool,
) -> Result<(), String> {
    let mut guard = None;
    let result = rebuild_session_reserved(
        gcx.clone(),
        session_arc,
        request,
        expected_version,
        explicit,
        &mut guard,
    )
    .await;
    if let Some(mut guard) = guard.take() {
        let batch = {
            let mut session = session_arc.lock().await;
            terminal_attempt(
                &mut session,
                guard.attempt,
                CompressionPhase::Failed,
                Some(CompressionReason::TransientFailure),
            )
        };
        finish_held_deliveries(gcx, session_arc.clone(), batch).await;
        guard.gcx = None;
        drop(guard);
    }
    result
}

async fn rebuild_session_reserved(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<Mutex<ChatSession>>,
    request: PendingContextRebuild,
    expected_version: Option<u64>,
    explicit: bool,
    guard: &mut Option<AttemptGuard>,
) -> Result<(), String> {
    crate::tasks::goal::hydrate_session_task_goal(gcx.clone(), session_arc).await?;
    let (hook_messages, hook_thread, source, attempt, abort) = {
        let mut session = session_arc.lock().await;
        if expected_version.is_some_and(|v| v != session.trajectory_version) {
            return Err("Chat changed since preview; preview again".into());
        }
        if session.pending_mode_handoff.is_some() || session.pending_context_rebuild.is_some() {
            return Err(SKIP_OTHER_PENDING.into());
        }
        if !(if explicit {
            explicit_context_ready(&session)
        } else {
            idle(&session)
        }) || compression_attempt_active(&session)
        {
            return Err(SKIP_NOT_IDLE.into());
        }
        if !explicit && requires_explicit_rebuild(&session.messages) {
            return Err("Legacy context requires an explicit rebuild".into());
        }
        let messages = raw_rebuild_input(&session.messages, explicit)?;
        let (messages, abandoned) =
            normalize_rebuild_input(&messages, &live_tool_call_ids(&session));
        let (mut messages, _) = split_live_suffix(&messages, &live_tool_call_ids(&session));
        messages.extend(abandoned_calls_note(&abandoned));
        if session.task_goal_messages.is_some() {
            messages.retain(|message| {
                !matches!(message.role.as_str(), "goal" | "plan")
                    && !matches!(
                        message
                            .extra
                            .get("event")
                            .and_then(|event| event.get("subkind"))
                            .and_then(serde_json::Value::as_str),
                        Some("goal_delta" | "plan_delta" | "goal_pursuit")
                    )
            });
            messages.extend(session.try_accepted_control_projection()?.messages);
        }
        let (normalized, _) = normalize_rebuild_input(&messages, &live_tool_call_ids(&session));
        if messages.is_empty() || pending_tools(&normalized) {
            return Err(SKIP_PENDING_TOOLS.into());
        }
        if !explicit && session.last_rebuild_attempt_version == Some(session.trajectory_version) {
            return Err(SKIP_ALREADY_ATTEMPTED.into());
        }
        let source = RebuildSource::capture(&session)?;
        session.compression_attempt_generation =
            session.compression_attempt_generation.wrapping_add(1);
        let attempt = session.compression_attempt_generation;
        let abort = Arc::new(AtomicBool::new(false));
        session.active_compression_attempt = Some(attempt);
        session.compression_abort_flag = Some(abort.clone());
        session.compression_attempt_started_at_ms =
            Some(chrono::Utc::now().timestamp_millis() as u64);
        status(&mut session, CompressionPhase::Checking, None);
        (messages, session.thread.clone(), source, attempt, abort)
    };
    *guard = Some(AttemptGuard {
        session: session_arc.clone(),
        attempt,
        abort: abort.clone(),
        gcx: Some(gcx.clone()),
    });
    match precompact_control(
        gcx.clone(),
        &hook_thread,
        &request.trigger,
        hook_messages.len(),
    )
    .await
    {
        ControlVerdict::Deny(reason) => {
            let batch = {
                let mut session = session_arc.lock().await;
                if session.active_compression_attempt == Some(attempt)
                    && !explicit
                    && source.matches(&session)
                {
                    session.last_rebuild_attempt_version = Some(session.trajectory_version);
                }
                terminal_attempt(
                    &mut session,
                    attempt,
                    CompressionPhase::Skipped,
                    Some(CompressionReason::PreCompactDenied),
                )
            };
            finish_held_deliveries(gcx.clone(), session_arc.clone(), batch).await;
            return Err(bounded_precompact_reason(
                &reason,
                crate::ext::hooks_runner::HookLimits::from(&gcx.hooks_config).max_output_bytes,
            ));
        }
        ControlVerdict::Rewrite(_) => tracing::debug!(
            "PreCompact hook rewrite ignored for session {}",
            hook_thread.id
        ),
        ControlVerdict::Allow => {}
    }
    let (messages, abandoned, rebuild_input, thread, version, save_mutex, catalog, messages_before) = {
        let mut session = session_arc.lock().await;
        if session.active_compression_attempt != Some(attempt)
            || abort.load(Ordering::SeqCst)
            || !(if explicit {
                explicit_context_ready(&session)
            } else {
                idle(&session)
            })
            || !source.matches(&session)
        {
            return Err(SKIP_CHANGED_DURING_HOOK.into());
        }
        if session.pending_mode_handoff.is_some() || session.pending_context_rebuild.is_some() {
            return Err(SKIP_OTHER_PENDING.into());
        }
        let raw = hook_messages;
        let (normalized, abandoned) = normalize_rebuild_input(&raw, &live_tool_call_ids(&session));
        if normalized.is_empty() || pending_tools(&normalized) {
            return Err(SKIP_PENDING_TOOLS.into());
        }
        let rebuild_input = fold_controls_for_rebuild(
            normalized,
            crate::runtime_settings::plan_projection_limits(),
        );
        let messages_before = raw.len();
        session.last_rebuild_attempt_version = Some(session.trajectory_version);
        status(&mut session, CompressionPhase::Running, None);
        (
            raw,
            abandoned,
            rebuild_input,
            session.thread.clone(),
            session.trajectory_version,
            session.trajectory_save_mutex.clone(),
            session.tool_catalog.clone(),
            messages_before,
        )
    };
    let (target_model, n_ctx) = resolve_model(gcx.clone(), &thread, None).await?;
    let tokenizer = model_tokenizer(gcx.clone(), &target_model).await;
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.message)?;
    let record = crate::caps::resolve_chat_model(caps, &target_model).map_err(|e| e.to_string())?;
    let output_reserve = thread
        .max_tokens
        .or(record.default_max_tokens)
        .or(record.max_output_tokens)
        .unwrap_or(4096);
    // Rebuild clears the persisted prefix. Prepare a local-only replacement when
    // absent or incomplete (first-use, legacy, and repaired transition chats),
    // using the reserved catalog.
    let mut budget_thread = thread.clone();
    if !budget_thread
        .frozen_request_prefix
        .as_ref()
        .is_some_and(budget_prefix_is_usable)
    {
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let scope = thread
            .worktree
            .as_ref()
            .map(|w| w.root.to_string_lossy().into_owned());
        let tools = match catalog {
            Some(catalog) => catalog.index.tools.clone(),
            None => app
                .tool_registry
                .get_tools_index_for_mode_and_scope(
                    &thread.mode,
                    Some(&record.base.id),
                    scope.as_deref(),
                )
                .await
                .tools
                .clone(),
        };
        let canonical = crate::chat::prepare::build_canonical_openai_tools(
            gcx.clone(),
            &tools,
            record.supports_strict_tools,
            record.supports_tools,
        )
        .await;
        let meta = crate::call_validation::ChatMeta {
            chat_id: thread.id.clone(),
            chat_mode: thread.mode.clone(),
            chat_remote: false,
            current_config_file: String::new(),
            context_tokens_cap: thread.context_tokens_cap,
            include_project_info: thread.include_project_info,
            request_attempt_id: String::new(),
            worktree: thread.worktree.clone(),
        };
        // Do not call the preamble mixer: task briefing can run another LLM.
        let preamble = if let Some(system) = messages.iter().find(|m| m.role == "system") {
            system.content.content_text_only()
        } else {
            let base = crate::chat::prompts::get_mode_system_prompt(
                app.clone(),
                &thread.mode,
                Some(&target_model),
            )
            .await;
            crate::chat::prompts::system_prompt_add_extra_instructions(
                app,
                base,
                tools.iter().map(|t| t.name.clone()).collect(),
                &meta,
                &thread.task_meta,
                &thread.mode,
            )
            .await
        };
        budget_thread.frozen_request_prefix = Some(refact_chat_api::FrozenRequestPrefix {
            schema_version: 1,
            created_at: String::new(),
            system_prompt: Some(preamble),
            tools_canonical: Some(serde_json::Value::Array(canonical.tools)),
        });
    }
    let request_cap =
        payload_request_cap(&budget_thread, n_ctx, output_reserve, tokenizer.clone())?;
    let live_agents =
        crate::agents::compression_context::collect_live_agents(gcx.clone(), &thread.id).await?;
    let rebuild_input = crate::agentic::mode_transition::replace_live_agents_context(
        &rebuild_input,
        live_agents.to_message(),
    );
    let notes = rebuild_notes(&rebuild_input, &abandoned);
    let live_tail = {
        let session = session_arc.lock().await;
        let active = raw_rebuild_input(&session.messages, explicit)?;
        split_live_suffix(&active, &live_tool_call_ids(&session)).1
    };
    let tail_tokens = if live_tail.is_empty() {
        0
    } else {
        estimated_tokens(&live_tail, tokenizer.clone(), false)
    };
    let request_cap = request_cap
        .checked_sub(tail_tokens)
        .filter(|cap| *cap > 0)
        .ok_or("Live tool context exceeds request cap")?;
    let builder_cap = request_cap
        .checked_sub(notes_tokens(&notes, tokenizer.clone()))
        .filter(|cap| *cap > 0)
        .ok_or("Abandoned-call notice exceeds the request cap")?;
    let target_budget_symbols =
        token_cap_to_symbol_budget(&rebuild_input, builder_cap, tokenizer.clone());
    let outcome = crate::agentic::mode_transition::reconstruct_context(
        gcx.clone(),
        crate::agentic::mode_transition::ReconstructionRequest {
            messages: &rebuild_input,
            target_mode: &thread.mode,
            target_mode_description: "Continue the current conversation in the same mode",
            parent_chat_id: Some(&thread.id),
            model_override: request.model.clone(),
            chat_model: Some(thread.model.clone()),
            abort_flag: Some(abort.clone()),
            hints: None,
            target_budget_symbols: Some(target_budget_symbols),
            preserve_goal_messages: true,
            live_agents_scope: Some(&thread.id),
        },
    )
    .await;
    let outcome = match outcome {
        Ok(v) => v,
        Err(e) => {
            return Err(e);
        }
    };
    let output = with_abandoned_notes(outcome.messages, notes);
    let metrics = validate_output(&messages, &output, request_cap, tokenizer)?;
    let report = make_reconstruction_report(
        output,
        ReconstructionMetadata {
            source_version: Some(version),
            model: Some(outcome.model),
            trigger: Some(request.trigger.clone()),
            from_mode: Some(thread.mode.clone()),
            to_mode: Some(thread.mode.clone()),
            metrics: Some(metrics.to_json()),
        },
    )
    .map(|mut report| {
        if !abandoned.is_empty() {
            report.extra.insert(
                NORMALIZATION_REPORT_EXTRA.into(),
                abandoned_summary(&abandoned),
            );
        }
        report
    })
    .map_err(|e| e.to_string());
    let session_arc = session_arc.clone();
    tokio::spawn(async move {
        let writer = save_mutex.lock_owned().await;
        crate::tasks::goal::hydrate_session_task_goal(gcx.clone(), &session_arc).await?;
        match report {
            Ok(report) => {
                let persist_gcx = gcx.clone();
                commit_report_then_observe(
                    gcx,
                    &session_arc,
                    report,
                    &source,
                    attempt,
                    &thread,
                    &abort,
                    &request.trigger,
                    messages_before,
                    |snapshot| async move {
                        let result =
                            crate::chat::trajectories::persist_trajectory_snapshot_with_intent(
                                persist_gcx,
                                snapshot,
                                TrajectoryCommitIntent::Required,
                            )
                            .await;
                        drop(writer);
                        result
                    },
                )
                .await
            }
            Err(error) => {
                drop(writer);
                Err(error)
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

pub async fn apply_manual_context_rebuild(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<Mutex<ChatSession>>,
    requested_model: Option<&str>,
    expected_version: Option<u64>,
) -> ManualCompressionApplyResult {
    let (messages, live_ids, thread, state, active, version) = {
        let s = session_arc.lock().await;
        (
            s.messages.clone(),
            live_tool_call_ids(&s),
            s.thread.clone(),
            s.runtime.state,
            compression_attempt_active(&s),
            s.trajectory_version,
        )
    };
    let preview = preview_manual_context_rebuild(
        gcx.clone(),
        state,
        active,
        &messages,
        &live_ids,
        &thread,
        requested_model,
    )
    .await;
    let before = preview.approximate_source_tokens;
    let result = if expected_version != Some(version) {
        Err("Chat changed since preview; preview again".into())
    } else if !preview.eligible {
        Err(preview.reason.clone().unwrap_or_default())
    } else {
        rebuild_session(
            gcx,
            session_arc,
            PendingContextRebuild {
                model: requested_model.map(str::to_owned),
                trigger: "manual".into(),
            },
            expected_version,
            true,
        )
        .await
    };
    let after = {
        let s = session_arc.lock().await;
        active_context(&s.messages)
            .map(|v| v.messages)
            .unwrap_or_else(|_| messages.clone())
    };
    ManualCompressionApplyResult {
        applied: result.is_ok(),
        resolved_model: preview.resolved_model,
        context_window: preview.context_window,
        source_messages: preview.source_messages,
        approximate_source_tokens: before,
        before_message_count: preview.source_messages,
        after_message_count: after.len(),
        before_approx_tokens: before,
        after_approx_tokens: crate::chat::trajectory_ops::approx_token_count(&after),
        reason: result.err(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionOutcome {
    Applied,
    NothingToCompact,
    LlmUnavailable,
}
impl CompactionOutcome {
    pub fn applied(self) -> bool {
        self == Self::Applied
    }
}

pub async fn apply_context_rebuild_with_reason(
    gcx: Arc<GlobalContext>,
    session: &Arc<Mutex<ChatSession>>,
    thread: &ThreadParams,
    force: bool,
    _reason: Option<CompressionReason>,
) -> CompactionOutcome {
    if !thread.auto_compact_enabled_effective() {
        return CompactionOutcome::NothingToCompact;
    }
    if !force {
        let Ok((model, n_ctx)) = resolve_model(gcx.clone(), thread, None).await else {
            return CompactionOutcome::NothingToCompact;
        };
        let tokenizer = model_tokenizer(gcx.clone(), &model).await;
        let s = session.lock().await;
        if !idle(&s)
            || s.last_rebuild_attempt_version == Some(s.trajectory_version)
            || serde_json::to_value(&s.thread).ok() != serde_json::to_value(thread).ok()
        {
            return CompactionOutcome::NothingToCompact;
        }
        let Ok(view) = active_context(&s.messages) else {
            return CompactionOutcome::NothingToCompact;
        };
        let cap = [
            Some(n_ctx),
            thread.context_tokens_cap,
            thread.auto_compression_cap,
        ]
        .into_iter()
        .flatten()
        .filter(|v| *v > 0)
        .min()
        .unwrap_or(n_ctx);
        let used = estimated_tokens(&view.messages, tokenizer, !s.provider_usage_stale);
        if !at_auto_cap(used, cap) {
            return CompactionOutcome::NothingToCompact;
        }
    }
    match rebuild_session(
        gcx,
        session,
        PendingContextRebuild {
            model: None,
            trigger: if force { "overflow" } else { "automatic" }.into(),
        },
        None,
        false,
    )
    .await
    {
        Ok(()) => CompactionOutcome::Applied,
        Err(e) => record_rebuild_failure(&mut *session.lock().await, &e),
    }
}

pub(crate) fn record_rebuild_failure(s: &mut ChatSession, e: &str) -> CompactionOutcome {
    if e.starts_with("blocked by PreCompact hook:") {
        return CompactionOutcome::NothingToCompact;
    }
    if compression_attempt_active(s) || s.abort_flag.load(Ordering::SeqCst) {
        return CompactionOutcome::NothingToCompact;
    }
    if is_routine_rebuild_skip(e) {
        tracing::info!("context rebuild skipped for {}: {e}", s.chat_id);
        let reason = if e == SKIP_PENDING_TOOLS {
            CompressionReason::PendingToolCalls
        } else {
            CompressionReason::SourceChanged
        };
        status(s, CompressionPhase::Skipped, Some(reason));
        return CompactionOutcome::NothingToCompact;
    }
    let detail = format!(
        "Context rebuild failed: {}",
        crate::chat::diagnostics::safe_provider_error_diagnostic(e)
    );
    s.append_error_message_deduped(&detail);
    s.last_rebuild_attempt_version = Some(s.trajectory_version);
    s.set_runtime_state(SessionState::Error, Some(detail));
    CompactionOutcome::LlmUnavailable
}

pub(crate) fn record_requested_rebuild_skip(
    session: &mut ChatSession,
    trigger: &str,
    error: &str,
) -> Result<(), String> {
    if !is_routine_rebuild_skip(error) && !error.starts_with("blocked by PreCompact hook:") {
        return Err(error.to_string());
    }
    record_rebuild_failure(session, error);
    let reason = crate::chat::diagnostics::safe_provider_error_diagnostic(error);
    session.add_message(crate::chat::internal_roles::event(
        crate::chat::internal_roles::EventSubkind::SystemNotice,
        "chat.context_rebuild",
        serde_json::json!({"trigger": trigger, "applied": false, "reason": reason}),
        format!("Requested context rebuild was not applied: {reason}. Request it again when the conversation is ready."),
    ));
    Ok(())
}

/// Drain only after every sibling tool result and post-tool side effect landed.
pub async fn drain_pending(
    gcx: Arc<GlobalContext>,
    session_arc: &Arc<Mutex<ChatSession>>,
) -> Result<bool, String> {
    let (rebuild, handoff) = {
        let mut s = session_arc.lock().await;
        if !idle(&s)
            || s.pending_mode_handoff
                .as_ref()
                .is_some_and(|marker| marker.get("http_transition").is_some())
        {
            return Ok(false);
        }
        (
            s.pending_context_rebuild.take(),
            s.pending_mode_handoff.take(),
        )
    };
    if let Some(payload) = handoff {
        crate::tools::tool_handoff_to_mode::finalize_pending_handoff(gcx, session_arc, payload)
            .await?;
        return Ok(true);
    }
    if let Some(request) = rebuild {
        let trigger = request.trigger.clone();
        if let Err(error) = rebuild_session(gcx, session_arc, request, None, true).await {
            let mut s = session_arc.lock().await;
            record_requested_rebuild_skip(&mut s, &trigger, &error)?;
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::chat_types::{DeliveryOutcome, PendingDelivery, PushMode};

    fn fixture() -> (ChatSession, RebuildSource, u64, ChatMessage) {
        let mut s = ChatSession::new("transaction-test".into());
        s.add_message(ChatMessage::new(
            "user".into(),
            "ARCHIVED_SENTINEL ".repeat(100),
        ));
        let source = RebuildSource::capture(&s).unwrap();
        s.active_compression_attempt = Some(1);
        status(&mut s, CompressionPhase::Running, None);
        let mut context = ChatMessage::new("user".into(), "active request".into());
        context.message_id = "rebuilt-request".into();
        let report =
            make_reconstruction_report(vec![context], ReconstructionMetadata::default()).unwrap();
        let version = s.trajectory_version;
        (s, source, version, report)
    }

    #[tokio::test]
    async fn filtered_snapshot_commit_precedes_live_tool_suffix() {
        let (mut session, _, _, report) = fixture();
        let mut empty = ChatMessage::new("assistant".into(), "".into());
        empty.message_id = "filtered-prefix".into();
        session.messages.push(empty);
        let mut call = ChatMessage::new("assistant".into(), "pending batch".into());
        call.message_id = "live-batch".into();
        call.tool_calls = Some(vec![serde_json::from_value(serde_json::json!({
            "id": "pending", "type": "function",
            "function": {"name": "ask_questions", "arguments": "{}"}
        }))
        .unwrap()]);
        session.messages.push(call);
        session.runtime.state = SessionState::Paused;
        session.runtime.paused_message_index = Some(2);
        let archive = serde_json::to_value(&session.messages[..2]).unwrap();
        let suffix = serde_json::to_value(&session.messages[2..]).unwrap();
        let retained =
            crate::chat::trajectories::trajectory_snapshot_from_session(&session).messages;
        assert_eq!(retained.len(), 2);
        assert!(live_tool_call_ids(&session).contains("pending"));
        let source = RebuildSource::capture(&session).unwrap();
        let report_value = serde_json::to_value(&report).unwrap();
        let mut persisted = None;
        commit_report(
            &mut session,
            report,
            &source,
            1,
            &AtomicBool::new(false),
            |snapshot| {
                persisted = Some(snapshot.messages);
                async { Ok(()) }
            },
        )
        .await
        .unwrap();
        let persisted = persisted.unwrap();
        assert_eq!(persisted.len(), 3);
        assert_eq!(serde_json::to_value(&persisted[0]).unwrap(), archive[0]);
        assert_eq!(serde_json::to_value(&persisted[1]).unwrap(), report_value);
        assert_eq!(serde_json::to_value(&persisted[2..]).unwrap(), suffix);
        assert_eq!(
            serde_json::to_value(&session.messages[..2]).unwrap(),
            archive
        );
        assert_eq!(
            serde_json::to_value(&session.messages[2]).unwrap(),
            report_value
        );
        assert_eq!(
            serde_json::to_value(&session.messages[3..]).unwrap(),
            suffix
        );
        assert_eq!(session.runtime.paused_message_index, Some(3));
        let live_active = active_context(&session.messages).unwrap().messages;
        assert_eq!(live_active.last().unwrap().message_id, "live-batch");
        assert_eq!(
            serde_json::to_value(active_context(&persisted).unwrap().messages).unwrap(),
            serde_json::to_value(live_active).unwrap(),
        );
    }

    #[tokio::test]
    async fn live_batch_commit_keeps_raw_suffix_executable_and_rebases_pause() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        for (state, embedded) in [
            (SessionState::ExecutingTools, false),
            (SessionState::Paused, false),
            (SessionState::WaitingUserInput, false),
            (SessionState::WaitingUserInput, true),
        ] {
            let (mut session, _, _, report) = fixture();
            let mut call = ChatMessage::new("assistant".into(), "pending batch".into());
            call.message_id = "live-batch".into();
            call.tool_calls = Some(vec![ChatToolCall {
                id: "pending".into(),
                index: None,
                tool_type: "function".into(),
                function: ChatToolFunction {
                    name: "ask_questions".into(),
                    arguments: "{}".into(),
                },
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            }]);
            if embedded {
                let payload = vec![session.messages[0].clone(), call];
                session.messages =
                    vec![make_reconstruction_report(payload, Default::default()).unwrap()];
            } else {
                session.messages.push(call);
            }
            session.runtime.state = state;
            session.runtime.paused_message_index = if state == SessionState::Paused {
                Some(1)
            } else {
                None
            };
            let archive = serde_json::to_value(&session.messages[0]).unwrap();
            let suffix = serde_json::to_value(&session.messages[1..]).unwrap();
            let source = RebuildSource::capture(&session).unwrap();
            assert!(live_tool_call_ids(&session).contains("pending"));
            commit_report(
                &mut session,
                report,
                &source,
                1,
                &AtomicBool::new(false),
                |_| async { Ok(()) },
            )
            .await
            .unwrap();
            if !embedded {
                assert_eq!(
                    serde_json::to_value(&session.messages[2..]).unwrap(),
                    suffix
                );
            }
            assert_eq!(
                session.runtime.paused_message_index,
                if state == SessionState::Paused {
                    Some(2)
                } else {
                    None
                }
            );
            assert_eq!(serde_json::to_value(&session.messages[0]).unwrap(), archive);
            assert_eq!(session.runtime.state, state);
            assert!(!session.abort_flag.load(Ordering::SeqCst));
            let active = active_context(&session.messages).unwrap().messages;
            assert_eq!(active.last().unwrap().message_id, "live-batch");
            assert_eq!(
                active
                    .iter()
                    .flat_map(|m| m.tool_calls.iter().flatten())
                    .filter(|c| c.id == "pending")
                    .count(),
                1
            );
            assert!(!source.matches(&session));
        }
    }

    #[tokio::test]
    async fn live_source_changes_invalidate_reconstruction_cas() {
        for change in 0..3 {
            let (mut session, source, _, report) = fixture();
            match change {
                0 => session.runtime.state = SessionState::WaitingUserInput,
                1 => session.abort_flag.store(true, Ordering::SeqCst),
                _ => session.runtime.paused_message_index = Some(0),
            }
            let before = serde_json::to_value(&session.messages).unwrap();
            assert!(commit_report(
                &mut session,
                report,
                &source,
                1,
                &AtomicBool::new(false),
                |_| async { panic!("Changed live source must not reach persistence") }
            )
            .await
            .is_err());
            assert_eq!(serde_json::to_value(&session.messages).unwrap(), before);
        }
    }

    #[test]
    fn unavailable_legacy_estimate_is_not_zero_or_a_successful_fit() {
        let mut legacy = ChatMessage::new("assistant".into(), "legacy".into());
        legacy.summarized_range = Some((0, 0));
        assert_eq!(estimated_tokens(&[legacy.clone()], None, false), usize::MAX);
        assert!(validate_output(
            &[legacy],
            &[ChatMessage::new("user".into(), "x".into())],
            usize::MAX,
            None
        )
        .is_err());
    }

    #[test]
    fn smaller_output_cannot_hide_mandatory_prefix_or_missing_budget() {
        let s = ChatSession::new("budget".into());
        let mut thread = s.thread.clone();
        assert!(payload_request_cap(&thread, 4096, 512, None).is_err());
        thread.frozen_request_prefix = Some(refact_chat_api::FrozenRequestPrefix {
            schema_version: 1,
            created_at: String::new(),
            system_prompt: Some("system".repeat(2000)),
            tools_canonical: Some(serde_json::json!([])),
        });
        assert!(payload_request_cap(&thread, 4096, 512, None).is_err());
        thread.frozen_request_prefix.as_mut().unwrap().system_prompt = Some("system".into());
        let cap = payload_request_cap(&thread, 4096, 512, None).unwrap();
        let source = vec![ChatMessage::new("user".into(), "x".repeat(24000))];
        let output = vec![ChatMessage::new("user".into(), "x".repeat(9000))];
        assert!(validate_output(&source, &output, 4096, None).is_ok());
        assert!(validate_output(&source, &output, cap, None).is_err());
    }

    #[test]
    fn incomplete_frozen_prefix_is_an_unavailable_budget_not_a_missing_prompt() {
        let s = ChatSession::new("incomplete-prefix".into());
        let mut thread = s.thread.clone();
        thread.frozen_request_prefix = Some(refact_chat_api::FrozenRequestPrefix {
            schema_version: 1,
            created_at: String::new(),
            system_prompt: None,
            tools_canonical: Some(serde_json::json!([])),
        });
        let error = payload_request_cap(&thread, 4096, 512, None).unwrap_err();
        assert!(
            error.contains("incomplete frozen request prefix"),
            "{error}"
        );
        thread.frozen_request_prefix = Some(refact_chat_api::FrozenRequestPrefix {
            schema_version: 1,
            created_at: String::new(),
            system_prompt: Some("system".into()),
            tools_canonical: None,
        });
        let error = payload_request_cap(&thread, 4096, 512, None).unwrap_err();
        assert!(
            error.contains("incomplete frozen request prefix"),
            "{error}"
        );
        assert!(!crate::chat::trajectories::frozen_prefix_is_complete(
            thread.frozen_request_prefix.as_ref().unwrap()
        ));
        thread.frozen_request_prefix = Some(refact_chat_api::FrozenRequestPrefix {
            schema_version: 1,
            created_at: String::new(),
            system_prompt: Some("system".into()),
            tools_canonical: Some(serde_json::json!({})),
        });
        let error = payload_request_cap(&thread, 4096, 512, None).unwrap_err();
        assert!(
            error.contains("incomplete frozen request prefix"),
            "{error}"
        );
        assert!(!budget_prefix_is_usable(
            thread.frozen_request_prefix.as_ref().unwrap()
        ));
        thread
            .frozen_request_prefix
            .as_mut()
            .unwrap()
            .tools_canonical = Some(serde_json::json!([]));
        assert!(budget_prefix_is_usable(
            thread.frozen_request_prefix.as_ref().unwrap()
        ));
        assert!(payload_request_cap(&thread, 4096, 512, None).is_ok());
    }

    #[test]
    fn fresh_provider_usage_anchors_below_cap_context_over_inflated_local_estimate() {
        let system = ChatMessage::new("system".into(), "s".repeat(120_000));
        let user = ChatMessage::new("user".into(), "u".repeat(400_000));
        let mut answer = ChatMessage::new("assistant".into(), "answer".into());
        answer.usage = Some(crate::call_validation::ChatUsage {
            prompt_tokens: 1_592,
            completion_tokens: 542,
            total_tokens: 134_486,
            cache_read_tokens: Some(132_352),
            ..Default::default()
        });
        let follow_up = ChatMessage::new("user".into(), "continue".into());
        let messages = vec![system, user, answer, follow_up];
        let fresh = estimated_tokens(&messages, None, true);
        let stale = estimated_tokens(&messages, None, false);
        assert!(fresh >= 133_944, "{fresh}");
        assert!(fresh < stale, "fresh={fresh} stale={stale}");
        assert!(!at_auto_cap(fresh, 500_000));
        assert!(!at_auto_cap(fresh, 872_000));
    }

    #[test]
    fn exact_auto_boundary_and_no_savings_percentage_gate() {
        assert!(!at_auto_cap(850, 1000));
        assert!(!at_auto_cap(999, 1000));
        assert!(at_auto_cap(1000, 1000));
        assert!(at_auto_cap(1001, 1000));
        let source = vec![ChatMessage::new("user".into(), "a".repeat(1000))];
        let smaller = vec![ChatMessage::new("user".into(), "a".repeat(950))];
        assert!(validate_output(&source, &smaller, usize::MAX, None).is_ok());
        assert!(validate_output(&source, &source, usize::MAX, None).is_err());
        assert!(validate_output(&source, &smaller, 10, None).is_err());
        assert!(validate_output(&source, &[], usize::MAX, None).is_err());
    }

    #[test]
    fn explicit_nonstreaming_preserves_state_and_cancellation() {
        let mut generating = ChatSession::new("generating".into());
        generating.runtime.state = SessionState::Generating;
        assert!(!explicit_context_ready(&generating));
        for state in [
            SessionState::Idle,
            SessionState::Error,
            SessionState::Completed,
            SessionState::Paused,
            SessionState::WaitingUserInput,
            SessionState::WaitingIde,
            SessionState::ExecutingTools,
        ] {
            let mut s = ChatSession::new("manual".into());
            s.runtime.state = state;
            s.abort_flag.store(true, Ordering::SeqCst);
            assert!(explicit_context_ready(&s));
            assert_eq!(s.runtime.state, state);
            assert!(s.abort_flag.load(Ordering::SeqCst));
            s.draft_message = Some(ChatMessage::default());
            assert!(!explicit_context_ready(&s));
        }
    }

    #[tokio::test]
    async fn success_appends_report_after_persist_and_repeat_uses_active_only() {
        let (mut s, _, version, report) = fixture();
        let mut empty = ChatMessage::new("assistant".into(), " ".into());
        empty.message_id = "filtered-archive".into();
        s.messages.push(empty);
        let source = RebuildSource::capture(&s).unwrap();
        let archived = serde_json::to_value(&s.messages).unwrap();
        let retained = crate::chat::trajectories::trajectory_snapshot_from_session(&s).messages;
        assert_eq!(retained.len(), 1);
        let raw_at = s.messages.len();
        let report_value = serde_json::to_value(&report).unwrap();
        let mut persisted = None;
        commit_report(
            &mut s,
            report,
            &source,
            1,
            &AtomicBool::new(false),
            |snapshot| {
                assert_eq!(snapshot.version, version + 1);
                assert_eq!(snapshot.messages.len(), 2);
                assert_eq!(
                    serde_json::to_value(&snapshot.messages[..1]).unwrap(),
                    serde_json::to_value(&retained).unwrap()
                );
                assert_eq!(
                    serde_json::to_value(&snapshot.messages[1]).unwrap(),
                    report_value
                );
                assert!(snapshot.previous_response_id.is_none());
                assert!(snapshot.frozen_request_prefix.is_none());
                persisted = Some(snapshot.messages);
                async { Ok(()) }
            },
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(&s.messages[..raw_at]).unwrap(),
            archived
        );
        assert_eq!(
            serde_json::to_value(&s.messages[raw_at]).unwrap(),
            report_value
        );
        assert_eq!(
            serde_json::to_value(active_context(&persisted.unwrap()).unwrap().messages).unwrap(),
            serde_json::to_value(active_context(&s.messages).unwrap().messages).unwrap()
        );
        assert_eq!(s.trajectory_version, version + 1);
        assert_eq!(s.trajectory_committed_version, s.trajectory_version);
        assert!(!s.trajectory_dirty);
        assert!(s.provider_usage_stale);
        let active = active_context(&s.messages).unwrap().messages;
        assert_eq!(active.len(), 1);
        assert!(!serde_json::to_string(&active)
            .unwrap()
            .contains("ARCHIVED_SENTINEL"));
        s.add_message(ChatMessage::new("user".into(), "tail".into()));
        let second_input = active_context(&s.messages).unwrap().messages;
        assert_eq!(second_input.len(), 2);
        let report =
            make_reconstruction_report(second_input, ReconstructionMetadata::default()).unwrap();
        let source = RebuildSource::capture(&s).unwrap();
        commit_report(
            &mut s,
            report,
            &source,
            1,
            &AtomicBool::new(false),
            |_| async { Ok(()) },
        )
        .await
        .unwrap();
        assert_eq!(s.messages.len(), raw_at + 3);
        assert_eq!(active_context(&s.messages).unwrap().messages.len(), 2);
    }

    #[tokio::test]
    async fn failed_persistence_never_publishes_report() {
        let (mut s, source, version, report) = fixture();
        let before = serde_json::to_value(&s.messages).unwrap();
        assert!(commit_report(
            &mut s,
            report,
            &source,
            1,
            &AtomicBool::new(false),
            |_| async { Err("disk failed".into()) }
        )
        .await
        .is_err());
        assert_eq!(serde_json::to_value(&s.messages).unwrap(), before);
        assert_eq!(s.trajectory_version, version);
        let session = Arc::new(Mutex::new(s));
        drop(AttemptGuard {
            session: session.clone(),
            attempt: 1,
            abort: Arc::new(AtomicBool::new(false)),
            gcx: None,
        });
        assert!(!compression_attempt_active(&*session.lock().await));
    }

    fn delivery(id: &str, push: PushMode) -> PendingDelivery {
        PendingDelivery::with_id(
            id,
            vec![ChatMessage::new("user".into(), format!("delivery {id}"))],
            push,
            "test",
            false,
        )
    }

    #[tokio::test]
    async fn abort_history_settings_and_owner_races_never_persist() {
        for race in 0..4 {
            let (mut s, source, _, report) = fixture();
            let abort = AtomicBool::new(false);
            match race {
                0 => abort.store(true, Ordering::SeqCst),
                1 => s.add_message(ChatMessage::new("user".into(), "race".into())),
                2 => s.thread.model = "changed".into(),
                _ => s.active_compression_attempt = Some(2),
            }
            let before = serde_json::to_value(&s.messages).unwrap();
            assert!(
                commit_report(&mut s, report, &source, 1, &abort, |_| async {
                    panic!("CAS must precede persistence")
                })
                .await
                .is_err()
            );
            assert_eq!(serde_json::to_value(&s.messages).unwrap(), before);
        }
    }

    #[tokio::test]
    async fn queued_deliveries_goal_and_runtime_changes_commit_with_current_state() {
        let mut s = ChatSession::new("queued-commit".into());
        s.install_goal(
            "agent",
            "ship",
            true,
            refact_chat_api::GoalBudget::default(),
        );
        s.add_message(ChatMessage::new(
            "user".into(),
            "ARCHIVED_SENTINEL ".repeat(100),
        ));
        let source = RebuildSource::capture(&s).unwrap();
        s.active_compression_attempt = Some(1);
        status(&mut s, CompressionPhase::Running, None);
        let mut rebuilt = ChatMessage::new("user".into(), "active request".into());
        rebuilt.message_id = "queued-rebuilt-request".into();
        let report =
            make_reconstruction_report(vec![rebuilt], ReconstructionMetadata::default()).unwrap();
        let history = serde_json::to_value(&s.messages).unwrap();
        for (id, push) in [
            ("append", PushMode::Append),
            ("idle", PushMode::WhenIdle),
            ("urgent", PushMode::Preempt),
        ] {
            assert_eq!(
                s.enqueue_delivery(delivery(id, push)).unwrap(),
                DeliveryOutcome::Queued
            );
        }
        assert_eq!(
            s.enqueue_delivery(delivery("append", PushMode::Append))
                .unwrap(),
            DeliveryOutcome::Duplicate
        );
        s.add_message(ChatMessage::new("event".into(), "legacy event".into()));
        assert!(s.goal_set_snooze(Some(123)));
        assert_eq!(serde_json::to_value(&s.messages).unwrap(), history);
        assert!(!s.abort_flag.load(Ordering::SeqCst));
        assert!(compression_attempt_active(&s));
        let current = s.trajectory_version;
        let ledger = s.goal_ledger.len();
        commit_report(
            &mut s,
            report,
            &source,
            1,
            &AtomicBool::new(false),
            |snapshot| async move {
                assert_eq!(snapshot.version, current + 1);
                assert_eq!(snapshot.pending_deliveries.len(), 4);
                assert_eq!(snapshot.goal_ledger.len(), ledger);
                assert_eq!(snapshot.goal.as_ref().unwrap().snoozed_until_ms, Some(123));
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(s.trajectory_version, current + 1);
        assert_eq!(s.trajectory_committed_version, current + 1);
        assert_eq!(s.goal.as_ref().unwrap().snoozed_until_ms, Some(123));
        assert_eq!(s.pending_deliveries.len(), 4);
        status(&mut s, CompressionPhase::Applied, None);
        let (delivered, _) = s.drain_pending_deliveries();
        assert_eq!(delivered.len(), 4);
        assert_eq!(delivered[..3], ["append", "idle", "urgent"]);
        assert!(s.pending_deliveries.is_empty());
        let tail: Vec<_> = s.messages[history.as_array().unwrap().len() + 1..]
            .iter()
            .map(|m| m.content.content_text_only())
            .collect();
        assert_eq!(
            tail,
            vec![
                "delivery append",
                "delivery idle",
                "delivery urgent",
                "legacy event"
            ]
        );
        assert!(!s.abort_flag.load(Ordering::SeqCst));
        assert!(s.drain_pending_deliveries().0.is_empty());
    }

    #[tokio::test]
    async fn failed_or_denied_attempt_releases_held_deliveries() {
        for phase in [CompressionPhase::Failed, CompressionPhase::Skipped] {
            let (mut s, _, _, _) = fixture();
            assert_eq!(
                s.enqueue_delivery(delivery("held", PushMode::Append))
                    .unwrap(),
                DeliveryOutcome::Queued
            );
            assert!(s.drain_pending_deliveries().0.is_empty());
            status(&mut s, phase, Some(CompressionReason::TransientFailure));
            assert_eq!(s.drain_pending_deliveries().0, vec!["held"]);
        }
    }

    #[test]
    fn explicit_abort_still_cancels_rebuild_and_opens_boundary() {
        let (mut s, _, _, _) = fixture();
        let abort = Arc::new(AtomicBool::new(false));
        s.compression_abort_flag = Some(abort.clone());
        s.enqueue_delivery(delivery("held", PushMode::Append))
            .unwrap();
        s.abort_stream();
        assert!(abort.load(Ordering::SeqCst));
        assert!(!compression_attempt_active(&s));
        assert!(s.delivery_boundary_open(PushMode::Append));
    }

    #[tokio::test]
    async fn dropped_attempt_releases_after_contended_lock_without_clearing_new_owner() {
        let (s, _, _, _) = fixture();
        let session = Arc::new(Mutex::new(s));
        let abort = Arc::new(AtomicBool::new(false));
        let lock = session.lock().await;
        drop(AttemptGuard {
            session: session.clone(),
            attempt: 1,
            abort: abort.clone(),
            gcx: None,
        });
        assert!(abort.load(Ordering::SeqCst));
        drop(lock);
        tokio::task::yield_now().await;
        assert!(!compression_attempt_active(&*session.lock().await));
        session.lock().await.active_compression_attempt = Some(2);
        drop(AttemptGuard {
            session: session.clone(),
            attempt: 1,
            abort,
            gcx: None,
        });
        assert_eq!(session.lock().await.active_compression_attempt, Some(2));
    }

    #[test]
    fn overflow_finishes_stream_before_idle_and_preserves_text() {
        let mut s = ChatSession::new("overflow-test".into());
        let mut events = s.event_tx.subscribe();
        s.start_stream().unwrap();
        s.draft_message.as_mut().unwrap().content =
            crate::call_validation::ChatContent::SimpleText("accepted output".into());
        s.finish_stream_for_rebuild();
        assert!(idle(&s));
        assert_eq!(
            s.messages.last().unwrap().content.content_text_only(),
            "accepted output"
        );
        let mut recorded = Vec::new();
        while let Ok(event) = events.try_recv() {
            recorded.push(event.to_string());
        }
        assert!(recorded
            .iter()
            .any(|event| event.contains("stream_finished")));
    }

    #[test]
    fn pending_request_is_canceled_by_abort() {
        let mut s = ChatSession::new("abort-test".into());
        request_rebuild(
            &mut s,
            PendingContextRebuild {
                model: None,
                trigger: "tool".into(),
            },
        )
        .unwrap();
        s.abort_stream();
        assert!(s.pending_context_rebuild.is_none());
        assert!(!idle(&s));
    }

    #[tokio::test]
    async fn stale_manual_version_never_starts_builder() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut s = ChatSession::new("stale-test".into());
        s.add_message(ChatMessage::new("user".into(), "keep unchanged".into()));
        let before = serde_json::to_value(&s.messages).unwrap();
        let session = Arc::new(Mutex::new(s));
        let result = rebuild_session(
            gcx,
            &session,
            PendingContextRebuild {
                model: None,
                trigger: "test".into(),
            },
            Some(999),
            true,
        )
        .await;
        assert!(result.unwrap_err().contains("changed since preview"));
        let s = session.lock().await;
        assert_eq!(serde_json::to_value(&s.messages).unwrap(), before);
        assert!(!compression_attempt_active(&s));
    }

    #[tokio::test]
    async fn direct_rebuild_rejects_pending_owners_before_model_resolution() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        for handoff in [false, true] {
            let mut s = ChatSession::new("reserved-owner".into());
            s.add_message(ChatMessage::new("user".into(), "keep".into()));
            s.runtime.state = SessionState::Completed;
            s.abort_flag.store(true, Ordering::SeqCst);
            let request = PendingContextRebuild {
                model: None,
                trigger: "manual".into(),
            };
            if handoff {
                s.pending_mode_handoff = Some(serde_json::json!({"target_mode":"agent"}));
            } else {
                s.pending_context_rebuild = Some(request.clone());
            }
            let before = serde_json::to_value(&s.messages).unwrap();
            let session = Arc::new(Mutex::new(s));
            let error = rebuild_session(gcx.clone(), &session, request, None, true)
                .await
                .unwrap_err();
            assert!(error.contains("operation is pending"));
            let s = session.lock().await;
            assert_eq!(serde_json::to_value(&s.messages).unwrap(), before);
            assert_eq!(s.runtime.state, SessionState::Completed);
            assert!(s.abort_flag.load(Ordering::SeqCst));
            assert!(s.active_compression_attempt.is_none());
        }
    }

    #[tokio::test]
    async fn disabled_automatic_compression_blocks_forced_rebuild_without_model_call() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut s = ChatSession::new("disabled-auto".into());
        s.thread.auto_compact_enabled = Some(false);
        s.add_message(ChatMessage::new("user".into(), "keep".into()));
        let thread = s.thread.clone();
        let before = serde_json::to_value(&s.messages).unwrap();
        let session = Arc::new(Mutex::new(s));
        for force in [false, true] {
            let outcome =
                apply_context_rebuild_with_reason(gcx.clone(), &session, &thread, force, None)
                    .await;
            assert_eq!(outcome, CompactionOutcome::NothingToCompact);
        }
        let s = session.lock().await;
        assert_eq!(serde_json::to_value(&s.messages).unwrap(), before);
        assert!(s.last_rebuild_attempt_version.is_none());
    }

    #[test]
    fn payload_heavy_event_output_must_fit_rendered_wire_budget() {
        let source = vec![ChatMessage::new("user".into(), "history ".repeat(30_000))];
        let mut event = ChatMessage::new("event".into(), "task snapshot".into());
        event.extra.insert(
            "event".into(),
            serde_json::json!({
                "subkind": "tasks_snapshot",
                "payload": {"tasks": "<& task detail > ".repeat(2_000)}
            }),
        );
        let wire = refact_llm::adapters::render_extra::render_event_message(&event);
        let wire_tokens = crate::tokens::count_text_tokens(None, &wire).unwrap();
        assert!(wire_tokens > 128);
        assert!(estimated_tokens(&[event.clone()], None, false) >= wire_tokens);
        assert!(validate_output(&source, &[event.clone()], usize::MAX, None).is_ok());
        assert!(validate_output(&source, &[event], 128, None).is_err());
    }

    #[tokio::test]
    async fn dropped_gcx_guard_flushes_once_and_preserves_new_owner() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        for replace_owner in [false, true] {
            let (mut s, _, _, _) = fixture();
            s.enqueue_delivery(delivery("held", PushMode::Append))
                .unwrap();
            let session = Arc::new(Mutex::new(s));
            let abort = Arc::new(AtomicBool::new(false));
            let mut lock = session.lock().await;
            drop(AttemptGuard {
                session: session.clone(),
                attempt: 1,
                abort: abort.clone(),
                gcx: Some(gcx.clone()),
            });
            assert!(abort.load(Ordering::SeqCst));
            let new_abort = Arc::new(AtomicBool::new(false));
            if replace_owner {
                lock.active_compression_attempt = Some(2);
                lock.compression_abort_flag = Some(new_abort.clone());
                status(&mut lock, CompressionPhase::Checking, None);
            }
            drop(lock);
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while Arc::strong_count(&session) != 1 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("guard finalizer must finish");
            let mut s = session.lock().await;
            assert!(s.command_queue.is_empty());
            assert!(!s.queue_processor_running.load(Ordering::SeqCst));
            if replace_owner {
                assert_eq!(s.active_compression_attempt, Some(2));
                assert_eq!(s.compression_phase, Some(CompressionPhase::Checking));
                assert!(!new_abort.load(Ordering::SeqCst));
                assert_eq!(s.pending_deliveries.len(), 1);
                assert_eq!(s.messages.len(), 1);
            } else {
                assert!(!compression_attempt_active(&s));
                assert_eq!(s.compression_phase, Some(CompressionPhase::Failed));
                assert!(s.pending_deliveries.is_empty());
                assert_eq!(s.messages.len(), 2);
                assert_eq!(s.messages[1].content.content_text_only(), "delivery held");
                assert!(s.drain_pending_deliveries().0.is_empty());
            }
        }
    }
}
