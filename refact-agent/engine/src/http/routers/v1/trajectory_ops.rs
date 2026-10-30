use std::sync::Arc;
use axum::extract::Path;
use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use crate::chat::trajectory_ops::{CompressOptions, TransformStats, compress_in_place};
#[cfg(test)]
use crate::call_validation::ChatMessage;
use crate::agentic::mode_transition::{
    GoalTransferResult, ReconstructionRequest, reconstruct_context,
    insert_goal_messages_before_plan,
};
#[cfg(test)]
use crate::agentic::mode_transition::transfer_goal_ownership;
use crate::chat::types::SessionState;
use crate::tools::tool_handoff_to_mode::{persist_transition, transition_goal, transition_report};

struct TransitionReservation {
    session: Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    marker: serde_json::Value,
    fingerprint: serde_json::Value,
    abort: Arc<std::sync::atomic::AtomicBool>,
}

use crate::tools::tool_handoff_to_mode::transition_fingerprint;

impl TransitionReservation {
    async fn acquire(
        session: Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    ) -> Result<Self, ScratchError> {
        let mut source = session.lock().await;
        if !crate::chat::context_rebuild::explicit_context_ready(&source)
            || compression_attempt_active(&source)
            || source.pending_mode_handoff.is_some()
            || source.pending_context_rebuild.is_some()
        {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                "Another context operation is pending or source is busy".into(),
            ));
        }
        if source.draft_message.is_some() || source.stream_started_at.is_some() {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                "Source stream is not finished".into(),
            ));
        }
        let marker = serde_json::json!({"http_transition": Uuid::new_v4().to_string()});
        source.pending_mode_handoff = Some(marker.clone());
        let fingerprint = transition_fingerprint(&source);
        let abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
        drop(source);
        Ok(Self {
            session,
            marker,
            fingerprint,
            abort,
        })
    }
}

impl Drop for TransitionReservation {
    fn drop(&mut self) {
        self.abort.store(true, std::sync::atomic::Ordering::SeqCst);
        let session = self.session.clone();
        let marker = self.marker.clone();
        tokio::spawn(async move {
            let mut source = session.lock().await;
            if source.pending_mode_handoff.as_ref() == Some(&marker) {
                source.pending_mode_handoff = None;
                source.queue_notify.notify_waiters();
            }
        });
    }
}

use crate::chat::get_or_create_session_with_trajectory;
use crate::chat::context_rebuild::{
    ManualCompressionApplyResult, ManualCompressionPreview, apply_manual_context_rebuild,
    compression_attempt_active, live_tool_call_ids, preview_manual_context_rebuild,
};
use refact_chat_api::GoalSnapshot;
use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;
use crate::custom_error::ScratchError;
use crate::yaml_configs::customization_registry::map_legacy_mode_to_id;

fn transition_source_goal(session: &crate::chat::types::ChatSession) -> Option<GoalSnapshot> {
    crate::tools::tool_handoff_to_mode::transition_source_goal(session)
}

fn canonical_transition_mode(raw_mode: &str) -> String {
    map_legacy_mode_to_id(raw_mode.trim()).to_string()
}

fn reset_transition_snapshot_identity(snapshot: &mut TrajectorySnapshot) {
    snapshot.previous_response_id = None;
    snapshot.frozen_request_prefix = None;
    snapshot.claude_code_identity = None;
}

fn epoch_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
fn transfer_goal_into_transition_messages(
    source_messages: &[ChatMessage],
    source_goal: Option<&GoalSnapshot>,
    target_existing_messages: &[ChatMessage],
    source_chat_id: &str,
    target_chat_id: &str,
    target_mode: &str,
    target_messages: &mut Vec<ChatMessage>,
    at_ms: u64,
) -> GoalTransferResult {
    let transferred_goal = transfer_goal_ownership(
        source_messages,
        source_goal,
        target_existing_messages,
        source_chat_id,
        target_chat_id,
        target_mode,
        at_ms,
    );
    if transferred_goal.transferred() {
        insert_goal_messages_before_plan(target_messages, transferred_goal.target_messages.clone());
    }
    transferred_goal
}

#[cfg(test)]
async fn persist_live_source_goal_transfer(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    transferred_goal: &GoalTransferResult,
) -> Result<(), String> {
    if !transferred_goal.transferred() {
        return Ok(());
    }
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    if let Some(session_arc) = session_arc {
        {
            let mut session = session_arc.lock().await;
            session.replace_messages(transferred_goal.source_messages.clone());
            if let Some(target_chat_id) = transferred_goal
                .source_goal
                .as_ref()
                .and_then(|goal| goal.transferred_to.clone())
            {
                session.goal_ledger_append(refact_chat_api::GoalLedgerOp::TransferredOut {
                    target_chat_id,
                });
            }
            session.set_goal_projection(transferred_goal.source_goal.clone());
            session.emit_goal_status();
        }
        crate::chat::trajectories::try_save_trajectory_with_intent(
            AppState::from_gcx(gcx.clone()).await,
            session_arc,
            crate::chat::types::TrajectoryCommitIntent::Required,
        )
        .await?;
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct TransformRequest {
    pub options: CompressOptions,
    #[serde(default)]
    pub expected_trajectory_version: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LlmCompressOptions {
    #[serde(default)]
    pub summary_model: Option<String>,
    #[serde(default)]
    pub expected_trajectory_version: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LlmCompressRequest {
    #[serde(default)]
    pub options: LlmCompressOptions,
}

#[derive(Debug, Serialize)]
pub struct LlmCompressApplyResponse {
    pub applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<usize>,
    pub source_messages: usize,
    pub approximate_source_tokens: usize,
    pub stats: TransformStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl From<ManualCompressionApplyResult> for LlmCompressApplyResponse {
    fn from(result: ManualCompressionApplyResult) -> Self {
        Self {
            applied: result.applied,
            resolved_model: result.resolved_model,
            context_window: result.context_window,
            source_messages: result.source_messages,
            approximate_source_tokens: result.approximate_source_tokens,
            stats: TransformStats {
                before_message_count: result.before_message_count,
                after_message_count: result.after_message_count,
                before_approx_tokens: result.before_approx_tokens,
                after_approx_tokens: result.after_approx_tokens,
                context_messages_modified: 0,
                tool_messages_modified: 0,
            },
            reason: result.reason,
        }
    }
}

impl LlmCompressRequest {
    fn requested_model(&self) -> Option<&str> {
        self.options
            .summary_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
    }
}

fn parse_llm_compress_request(
    body_bytes: &hyper::body::Bytes,
) -> Result<LlmCompressRequest, ScratchError> {
    if body_bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(LlmCompressRequest::default());
    }
    serde_json::from_slice(body_bytes)
        .map_err(|_| ScratchError::new(StatusCode::BAD_REQUEST, "Invalid JSON request".to_string()))
}

fn json_response<T: Serialize>(value: &T) -> Result<Response<Body>, ScratchError> {
    let body = serde_json::to_vec(value)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_llm_compress_preview(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req = parse_llm_compress_request(&body_bytes)?;
    let gcx = app.gcx.clone();
    let session_arc = get_or_create_session_with_trajectory(
        AppState::from_gcx(gcx.clone()).await,
        &gcx.chat_sessions,
        &chat_id,
    )
    .await;
    let (state, active, messages, live_ids, thread, trajectory_version) = {
        let session = session_arc.lock().await;
        (
            if crate::chat::context_rebuild::explicit_context_ready(&session) {
                session.runtime.state
            } else {
                SessionState::Generating
            },
            compression_attempt_active(&session)
                || session.pending_mode_handoff.is_some()
                || session.pending_context_rebuild.is_some(),
            session.messages.clone(),
            live_tool_call_ids(&session),
            session.thread.clone(),
            session.trajectory_version,
        )
    };
    let mut preview: ManualCompressionPreview = preview_manual_context_rebuild(
        gcx,
        state,
        active,
        &messages,
        &live_ids,
        &thread,
        req.requested_model(),
    )
    .await;
    if preview.eligible {
        preview.trajectory_version = Some(trajectory_version);
    }
    json_response(&preview)
}

pub async fn handle_llm_compress_apply(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req = parse_llm_compress_request(&body_bytes)?;
    let gcx = app.gcx.clone();
    let session_arc = get_or_create_session_with_trajectory(
        AppState::from_gcx(gcx.clone()).await,
        &gcx.chat_sessions,
        &chat_id,
    )
    .await;
    let result: ManualCompressionApplyResult = apply_manual_context_rebuild(
        gcx.clone(),
        &session_arc,
        req.requested_model(),
        req.options.expected_trajectory_version,
    )
    .await;
    if result.applied {
        let _ = crate::chat::trajectories::try_save_trajectory_with_intent(
            AppState::from_gcx(gcx).await,
            session_arc,
            crate::chat::types::TrajectoryCommitIntent::Required,
        )
        .await;
    }
    json_response(&LlmCompressApplyResponse::from(result))
}

#[derive(Serialize)]
pub struct TransformPreviewResponse {
    pub trajectory_version: u64,
    pub stats: TransformStats,
    pub actions: Vec<String>,
}

#[derive(Serialize)]
pub struct TransformApplyResponse {
    pub stats: TransformStats,
}

#[derive(Deserialize)]
pub struct ModeTransitionApplyRequest {
    pub target_mode: String,
    #[serde(default)]
    pub target_mode_description: String,
}

#[derive(Serialize)]
pub struct ModeTransitionApplyResponse {
    pub new_chat_id: String,
    pub messages_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_chat_id: Option<String>,
}

#[derive(Deserialize)]
pub struct PlannerFromTransitionRequest {
    pub source_chat_id: String,
    #[serde(default)]
    pub target_mode_description: String,
    #[serde(default)]
    pub target_mode: Option<String>,
}

fn transition_completed_input(
    messages: &[crate::call_validation::ChatMessage],
    live_ids: &std::collections::HashSet<String>,
) -> Result<Vec<crate::call_validation::ChatMessage>, String> {
    let active =
        refact_core::active_context::legacy_rebuild_input(messages).map_err(|e| e.to_string())?;
    let (mut prefix, tail) = crate::chat::context_rebuild::split_live_suffix(&active, live_ids);
    if !tail.is_empty() {
        let detail = tail
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content.content_text_only()))
            .collect::<Vec<_>>()
            .join("\n");
        prefix.push(crate::call_validation::ChatMessage::new("user".into(), format!("Source conversation has pending tools or questions. They remain executable only in the source chat; do not replay or answer them here. Pending source context:\n{detail}")));
    }
    Ok(prefix)
}

fn compress_preserving_live(
    messages: &[crate::call_validation::ChatMessage],
    live_ids: &std::collections::HashSet<String>,
    options: &CompressOptions,
) -> Result<(Vec<crate::call_validation::ChatMessage>, TransformStats), String> {
    let active = refact_core::active_context::active_context(messages)
        .map_err(|e| e.to_string())?
        .messages;
    let (mut prefix, tail) = crate::chat::context_rebuild::split_live_suffix(&active, live_ids);
    if tail.is_empty() {
        let mut output = messages.to_vec();
        let stats = compress_in_place(&mut output, options)?;
        return Ok((output, stats));
    }
    let mut stats = if prefix.is_empty() {
        TransformStats {
            before_message_count: 0,
            after_message_count: 0,
            before_approx_tokens: 0,
            after_approx_tokens: 0,
            context_messages_modified: 0,
            tool_messages_modified: 0,
        }
    } else {
        compress_in_place(&mut prefix, options)?
    };
    if prefix.is_empty() {
        return Ok((
            messages.to_vec(),
            TransformStats {
                before_message_count: active.len(),
                after_message_count: active.len(),
                before_approx_tokens: crate::chat::trajectory_ops::approx_token_count(&active),
                after_approx_tokens: crate::chat::trajectory_ops::approx_token_count(&active),
                context_messages_modified: 0,
                tool_messages_modified: 0,
            },
        ));
    }
    for message in &mut prefix {
        if message.message_id.is_empty() {
            message.message_id = Uuid::new_v4().to_string();
        }
    }
    let mut report =
        refact_core::active_context::make_reconstruction_report(prefix, Default::default())
            .map_err(|e| e.to_string())?;
    let at = crate::chat::context_rebuild::place_report_before_live_suffix(
        messages,
        live_ids,
        &mut report,
    )?;
    let mut output = messages.to_vec();
    output.insert(at, report);
    stats.before_message_count += tail.len();
    stats.after_message_count += tail.len();
    let tail_tokens = crate::chat::trajectory_ops::approx_token_count(&tail);
    stats.before_approx_tokens += tail_tokens;
    stats.after_approx_tokens += tail_tokens;
    Ok((output, stats))
}

fn describe_transform_actions(opts: &CompressOptions) -> Vec<String> {
    let mut actions = Vec::new();
    if opts.drop_all_context {
        actions.push("Drop all context_file messages".to_string());
    } else if opts.dedup_and_compress_context {
        actions.push("Deduplicate and compress context files".to_string());
    }
    if opts.drop_all_memories {
        actions.push("Drop all memory/knowledge context".to_string());
    }
    if opts.drop_project_information {
        actions.push("Drop project information from system messages".to_string());
    }
    if opts.compress_non_agentic_tools {
        actions.push("Compress tool results (preserving subagent)".to_string());
    }
    if opts.strip_metering {
        actions.push("Strip metering information".to_string());
    }
    actions.push("Remove invalid tool calls and orphan results".to_string());
    actions
}

pub async fn handle_transform_preview(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let req: TransformRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let sessions = gcx.chat_sessions.clone();
    let session_arc = get_or_create_session_with_trajectory(
        AppState::from_gcx(gcx.clone()).await,
        &sessions,
        &chat_id,
    )
    .await;

    let (messages, live_ids, trajectory_version) = {
        let session = session_arc.lock().await;
        if !crate::chat::context_rebuild::explicit_context_ready(&session) {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                "Source stream is not finished".into(),
            ));
        }
        (
            session.messages.clone(),
            live_tool_call_ids(&session),
            session.trajectory_version,
        )
    };

    let (_, stats) = compress_preserving_live(&messages, &live_ids, &req.options)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = TransformPreviewResponse {
        trajectory_version,
        stats,
        actions: describe_transform_actions(&req.options),
    };

    let body = serde_json::to_vec(&response)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_transform_apply(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let req: TransformRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let sessions = gcx.chat_sessions.clone();
    let session_arc = get_or_create_session_with_trajectory(
        AppState::from_gcx(gcx.clone()).await,
        &sessions,
        &chat_id,
    )
    .await;

    let stats = {
        let mut session = session_arc.lock().await;

        if !crate::chat::context_rebuild::explicit_context_ready(&session) {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                format!(
                    "Session has an active stream, current state: {:?}",
                    session.runtime.state
                ),
            ));
        }

        if compression_attempt_active(&session)
            || session.pending_mode_handoff.is_some()
            || session.pending_context_rebuild.is_some()
            || req
                .expected_trajectory_version
                .is_some_and(|v| v != session.trajectory_version)
        {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                "Trajectory changed or compression is active".to_string(),
            ));
        }
        let live_ids = live_tool_call_ids(&session);
        let (messages, stats) =
            compress_preserving_live(&session.messages, &live_ids, &req.options)
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let added_at = messages.iter().position(|m| {
            m.role == "compression_report"
                && !session
                    .messages
                    .iter()
                    .any(|old| old.message_id == m.message_id)
        });
        session.messages = messages;
        if let (Some(at), Some(index)) = (added_at, session.runtime.paused_message_index.as_mut()) {
            if *index >= at {
                *index += 1;
            }
        }

        session.thread.previous_response_id = None;
        session.thread.frozen_request_prefix = None;
        session.thread.claude_code_identity = None;
        session.openai_codex_websocket = Default::default();
        session.reset_cache_guard_snapshot();
        session.provider_usage_stale = true;
        session.increment_version();
        let snapshot = session.snapshot();
        session.emit(snapshot);

        stats
    };

    crate::chat::trajectories::maybe_save_trajectory_with_intent(
        AppState::from_gcx(gcx.clone()).await,
        session_arc,
        crate::chat::types::TrajectoryCommitIntent::Required,
    )
    .await;

    let response = TransformApplyResponse { stats };

    let body = serde_json::to_vec(&response)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// Hold the source epoch stable until the destination is durable, then transfer
/// ownership. A failed reconstruction/save never overwrites newer source messages.
async fn commit_transition_snapshot(
    gcx: Arc<GlobalContext>,
    session_arc: Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    source_version: u64,
    snapshot: TrajectorySnapshot,
    transferred_goal: &GoalTransferResult,
    reservation: Option<&TransitionReservation>,
) -> Result<(), String> {
    let mut session = session_arc.lock().await;
    if session.trajectory_version != source_version
        || !crate::chat::context_rebuild::explicit_context_ready(&session)
        || compression_attempt_active(&session)
        || session.closed
        || session.pending_context_rebuild.is_some()
        || reservation.is_some_and(|r| {
            session.pending_mode_handoff.as_ref() != Some(&r.marker)
                || transition_fingerprint(&session) != r.fingerprint
                || r.abort.load(std::sync::atomic::Ordering::SeqCst)
        })
        || (reservation.is_none() && session.pending_mode_handoff.is_some())
    {
        return Err("Source trajectory changed during reconstruction".to_string());
    }
    persist_transition(gcx, &mut session, &snapshot, transferred_goal).await
}

#[cfg(test)]
async fn save_trajectory_snapshot_with_parent(
    gcx: Arc<GlobalContext>,
    mut snapshot: TrajectorySnapshot,
    parent_id: &str,
    link_type: &str,
) -> Result<(), String> {
    snapshot.parent_id = Some(parent_id.to_string());
    snapshot.link_type = Some(link_type.to_string());
    let chat_id = snapshot.chat_id.clone();
    crate::chat::trajectories::save_trajectory_snapshot(gcx, snapshot).await?;

    tracing::info!(
        "Saved handoff trajectory {} (parent: {}, link: {})",
        chat_id,
        parent_id,
        link_type
    );

    Ok(())
}

pub async fn handle_mode_transition_apply(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let req: ModeTransitionApplyRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;
    let target_mode = canonical_transition_mode(&req.target_mode);
    if target_mode == "task_planner" {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Use /v1/tasks/:task_id/planner-chats/from-transition for task_planner transitions"
                .to_string(),
        ));
    }

    let sessions = gcx.chat_sessions.clone();
    let session_arc = get_or_create_session_with_trajectory(
        AppState::from_gcx(gcx.clone()).await,
        &sessions,
        &chat_id,
    )
    .await;

    let reservation = TransitionReservation::acquire(session_arc.clone()).await?;
    let (messages, thread, task_meta, source_goal, session_state, source_version) = {
        let session = session_arc.lock().await;
        (
            session.messages.clone(),
            session.thread.clone(),
            session.thread.task_meta.clone(),
            transition_source_goal(&session),
            session.runtime.state.clone(),
            session.trajectory_version,
        )
    };

    // Check session state - only block when actively streaming (generating)
    if matches!(session_state, SessionState::Generating) {
        return Err(ScratchError::new(
            StatusCode::CONFLICT,
            format!("Cannot transition chat while generating, please wait or abort first"),
        ));
    }

    if messages.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Cannot transition an empty chat".to_string(),
        ));
    }
    let reconstruction_input =
        transition_completed_input(&messages, &live_tool_call_ids(&*session_arc.lock().await))
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let rebuilt = reconstruct_context(
        gcx.clone(),
        ReconstructionRequest {
            messages: &reconstruction_input,
            target_mode: &target_mode,
            target_mode_description: &req.target_mode_description,
            parent_chat_id: Some(chat_id.as_str()),
            model_override: None,
            chat_model: Some(thread.model.clone()),
            abort_flag: Some(reservation.abort.clone()),
            hints: None,
            target_budget_symbols: None,
            preserve_goal_messages: false,
            live_agents_scope: None,
        },
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let mut new_messages = rebuilt.messages;
    let _decisions = rebuilt.decisions;

    let new_chat_id = Uuid::new_v4().to_string();
    let transferred_goal = transition_goal(
        &messages,
        source_goal.as_ref(),
        &chat_id,
        &new_chat_id,
        &target_mode,
    )
    .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    if transferred_goal.transferred() {
        insert_goal_messages_before_plan(
            &mut new_messages,
            transferred_goal.target_messages.clone(),
        );
    }
    let new_messages = transition_report(
        &new_messages,
        source_version,
        rebuilt.model,
        thread.mode.clone(),
        target_mode.clone(),
    )
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let now = chrono::Utc::now().to_rfc3339();

    let root_chat_id = thread
        .root_chat_id
        .clone()
        .or_else(|| Some(chat_id.clone()));

    let mut snapshot = TrajectorySnapshot {
        recovery: None,
        recovery_policy: None,
        pending_browser_decision: None,
        pending_deliveries: Vec::new(),
        dropped_deliveries: Vec::new(),
        goal_verification_blocked_until_ms: None,
        goal: transferred_goal.target_goal.clone(),
        goal_ledger: transferred_goal
            .target_goal
            .as_ref()
            .map(|target| {
                refact_chat_api::seed_transferred_goal_ledger(target, &chat_id, epoch_ms_now())
            })
            .unwrap_or_default(),
        chat_id: new_chat_id.clone(),
        title: String::new(),
        model: thread.model.clone(),
        mode: target_mode.clone(),
        tool_use: thread.tool_use.clone(),
        messages: new_messages.clone(),
        created_at: now,
        boost_reasoning: thread.boost_reasoning.unwrap_or(false),
        checkpoints_enabled: thread.checkpoints_enabled,
        context_tokens_cap: thread.context_tokens_cap,
        auto_compression_cap: thread.auto_compression_cap,
        auto_compression_cap_pending: thread.auto_compression_cap.is_none(),
        include_project_info: thread.include_project_info,
        is_title_generated: false,
        auto_approve_editing_tools: thread.auto_approve_editing_tools,
        auto_approve_dangerous_commands: thread.auto_approve_dangerous_commands,
        autonomous_no_confirm: thread.autonomous_no_confirm,
        version: 1,
        task_meta,
        worktree: thread.worktree.clone(),
        parent_id: Some(chat_id.clone()),
        link_type: Some("mode_transition".to_string()),
        root_chat_id: root_chat_id.clone(),
        reasoning_effort: thread.reasoning_effort.clone(),
        thinking_budget: thread.thinking_budget,
        temperature: thread.temperature,
        frequency_penalty: thread.frequency_penalty,
        max_tokens: thread.max_tokens,
        parallel_tool_calls: thread.parallel_tool_calls,
        previous_response_id: None,
        active_skill: None,
        auto_enrichment_enabled: thread.auto_enrichment_enabled,
        buddy_meta: None,
        auto_compact_enabled: thread.auto_compact_enabled,
        frozen_request_prefix: thread.frozen_request_prefix.clone(),
        claude_code_identity: thread.claude_code_identity.clone(),
        reactive_compact_attempts: None,
        wake_up_at: None,
        waiting_for_card_ids: Vec::new(),
        pending_questions: Vec::new(),
    };
    reset_transition_snapshot_identity(&mut snapshot);

    commit_transition_snapshot(
        gcx.clone(),
        session_arc,
        source_version,
        snapshot,
        &transferred_goal,
        Some(&reservation),
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = ModeTransitionApplyResponse {
        new_chat_id,
        messages_count: new_messages.len(),
        root_chat_id,
    };

    let body = serde_json::to_vec(&response)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_planner_from_transition(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let req: PlannerFromTransitionRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let target_mode = req
        .target_mode
        .clone()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "task_planner".to_string());
    let target_mode = canonical_transition_mode(&target_mode);
    if target_mode != "task_planner" {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Only task_planner chats can be created here".to_string(),
        ));
    }

    // Verify the task exists before doing any work
    crate::tasks::storage::load_task_meta(gcx.clone(), &task_id)
        .await
        .map_err(|e| ScratchError::new(StatusCode::NOT_FOUND, e))?;

    let sessions = gcx.chat_sessions.clone();
    let session_arc = get_or_create_session_with_trajectory(
        AppState::from_gcx(gcx.clone()).await,
        &sessions,
        &req.source_chat_id,
    )
    .await;

    let reservation = TransitionReservation::acquire(session_arc.clone()).await?;
    let (messages, thread, source_goal, session_state, source_version) = {
        let session = session_arc.lock().await;
        (
            session.messages.clone(),
            session.thread.clone(),
            transition_source_goal(&session),
            session.runtime.state.clone(),
            session.trajectory_version,
        )
    };

    if matches!(session_state, SessionState::Generating) {
        return Err(ScratchError::new(
            StatusCode::CONFLICT,
            "Cannot transition chat while generating, please wait or abort first".to_string(),
        ));
    }

    if messages.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Cannot transition an empty chat".to_string(),
        ));
    }
    let reconstruction_input =
        transition_completed_input(&messages, &live_tool_call_ids(&*session_arc.lock().await))
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    crate::tasks::goal::preflight_planner_transition(
        gcx.clone(),
        &task_id,
        &req.source_chat_id,
        source_goal.as_ref(),
    )
    .await
    .map_err(|error| ScratchError::new(StatusCode::CONFLICT, error))?;

    let rebuilt = reconstruct_context(
        gcx.clone(),
        ReconstructionRequest {
            messages: &reconstruction_input,
            target_mode: &target_mode,
            target_mode_description: &req.target_mode_description,
            parent_chat_id: Some(req.source_chat_id.as_str()),
            model_override: None,
            chat_model: Some(thread.model.clone()),
            abort_flag: Some(reservation.abort.clone()),
            hints: None,
            target_budget_symbols: None,
            preserve_goal_messages: false,
            live_agents_scope: None,
        },
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let mut new_messages = rebuilt.messages;
    let _decisions = rebuilt.decisions;

    let new_chat_id = crate::tasks::storage::next_planner_chat_id(gcx.clone(), &task_id)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let transferred_goal = transition_goal(
        &messages,
        source_goal.as_ref(),
        &req.source_chat_id,
        &new_chat_id,
        &target_mode,
    )
    .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;
    if transferred_goal.transferred() {
        insert_goal_messages_before_plan(
            &mut new_messages,
            transferred_goal.target_messages.clone(),
        );
    }
    let new_messages = transition_report(
        &new_messages,
        source_version,
        rebuilt.model,
        thread.mode.clone(),
        target_mode.clone(),
    )
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let now = chrono::Utc::now().to_rfc3339();

    let task_meta = crate::chat::types::TaskMeta {
        task_id: task_id.clone(),
        role: "planner".to_string(),
        agent_id: None,
        card_id: None,
        planner_chat_id: Some(new_chat_id.clone()),
    };

    let root_chat_id = Some(new_chat_id.clone());

    let mut snapshot = TrajectorySnapshot {
        recovery: None,
        recovery_policy: None,
        pending_browser_decision: None,
        pending_deliveries: Vec::new(),
        dropped_deliveries: Vec::new(),
        goal_verification_blocked_until_ms: None,
        goal: transferred_goal.target_goal.clone(),
        goal_ledger: transferred_goal
            .target_goal
            .as_ref()
            .map(|target| {
                refact_chat_api::seed_transferred_goal_ledger(
                    target,
                    &req.source_chat_id,
                    epoch_ms_now(),
                )
            })
            .unwrap_or_default(),
        chat_id: new_chat_id.clone(),
        title: String::new(),
        model: thread.model.clone(),
        mode: target_mode.clone(),
        tool_use: thread.tool_use.clone(),
        messages: new_messages.clone(),
        created_at: now,
        boost_reasoning: thread.boost_reasoning.unwrap_or(false),
        checkpoints_enabled: thread.checkpoints_enabled,
        context_tokens_cap: thread.context_tokens_cap,
        auto_compression_cap: thread.auto_compression_cap,
        auto_compression_cap_pending: thread.auto_compression_cap.is_none(),
        include_project_info: thread.include_project_info,
        is_title_generated: false,
        auto_approve_editing_tools: thread.auto_approve_editing_tools,
        auto_approve_dangerous_commands: thread.auto_approve_dangerous_commands,
        autonomous_no_confirm: thread.autonomous_no_confirm,
        version: 1,
        task_meta: Some(task_meta),
        worktree: thread.worktree.clone(),
        parent_id: Some(req.source_chat_id.clone()),
        link_type: Some("mode_transition".to_string()),
        root_chat_id: root_chat_id.clone(),
        reasoning_effort: thread.reasoning_effort.clone(),
        thinking_budget: thread.thinking_budget,
        temperature: thread.temperature,
        frequency_penalty: thread.frequency_penalty,
        max_tokens: thread.max_tokens,
        parallel_tool_calls: thread.parallel_tool_calls,
        previous_response_id: None,
        active_skill: None,
        auto_enrichment_enabled: thread.auto_enrichment_enabled,
        buddy_meta: None,
        auto_compact_enabled: thread.auto_compact_enabled,
        frozen_request_prefix: thread.frozen_request_prefix.clone(),
        claude_code_identity: thread.claude_code_identity.clone(),
        reactive_compact_attempts: None,
        wake_up_at: None,
        waiting_for_card_ids: Vec::new(),
        pending_questions: Vec::new(),
    };
    reset_transition_snapshot_identity(&mut snapshot);

    // task_meta is set, so this saves into the task's planner directory
    commit_transition_snapshot(
        gcx.clone(),
        session_arc,
        source_version,
        snapshot,
        &transferred_goal,
        Some(&reservation),
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = ModeTransitionApplyResponse {
        new_chat_id,
        messages_count: new_messages.len(),
        root_chat_id,
    };

    let body = serde_json::to_vec(&response)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_chat_api::{GoalAttempt, GoalBudget, GoalProgress, GoalStatus};
    use serde_json::json;

    #[tokio::test]
    async fn reservation_preserves_every_nonstreaming_state_and_never_aborts_source() {
        for state in [
            SessionState::Idle,
            SessionState::Error,
            SessionState::Completed,
            SessionState::Paused,
            SessionState::WaitingUserInput,
            SessionState::WaitingIde,
            SessionState::ExecutingTools,
        ] {
            let mut session = crate::chat::types::ChatSession::new("source".into());
            session.runtime.state = state;
            let source = Arc::new(tokio::sync::Mutex::new(session));
            let reservation = TransitionReservation::acquire(source.clone())
                .await
                .unwrap();
            assert_eq!(source.lock().await.runtime.state, state);
            assert!(!Arc::ptr_eq(
                &reservation.abort,
                &source.lock().await.abort_flag
            ));
            drop(reservation);
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while source.lock().await.pending_mode_handoff.is_some() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("reservation cleanup must complete");
            let session = source.lock().await;
            assert_eq!(session.runtime.state, state);
            assert!(!session.abort_flag.load(std::sync::atomic::Ordering::SeqCst));
            assert!(session.pending_mode_handoff.is_none());
        }
    }

    #[test]
    fn static_compression_and_transition_do_not_consume_live_calls() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        let mut call = ChatMessage::new("assistant".into(), "question".into());
        call.message_id = "pending-batch".into();
        call.tool_calls = Some(vec![ChatToolCall {
            id: "live".into(),
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
        let messages = vec![
            ChatMessage::new("user".into(), "history".into()),
            call.clone(),
        ];
        let live = std::collections::HashSet::from(["live".to_string()]);
        let (output, _) =
            compress_preserving_live(&messages, &live, &CompressOptions::default()).unwrap();
        assert_eq!(
            serde_json::to_value(output.last().unwrap()).unwrap(),
            serde_json::to_value(&call).unwrap()
        );
        let active = refact_core::active_context::active_context(&output)
            .unwrap()
            .messages;
        assert_eq!(
            active
                .iter()
                .flat_map(|m| m.tool_calls.iter().flatten())
                .filter(|c| c.id == "live")
                .count(),
            1
        );
        let input = transition_completed_input(&messages, &live).unwrap();
        assert!(input.iter().all(|m| m.tool_calls.is_none()));
        assert!(input
            .last()
            .unwrap()
            .content
            .content_text_only()
            .contains("only in the source chat"));
    }

    #[tokio::test]
    async fn transition_commit_rejects_stale_source_before_destination_save() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let session = Arc::new(tokio::sync::Mutex::new(
            crate::chat::types::ChatSession::new("source".to_string()),
        ));
        let version = session.lock().await.trajectory_version;
        let transferred = transfer_goal_ownership(&[], None, &[], "source", "target", "agent", 1);
        let snapshot = transition_identity_snapshot("mode_transition");
        let error =
            commit_transition_snapshot(gcx, session, version + 1, snapshot, &transferred, None)
                .await
                .unwrap_err();
        assert!(error.contains("Source trajectory changed"));
    }

    #[tokio::test]
    async fn failed_destination_persistence_leaves_source_goal_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        std::fs::write(dir.path().join(".refact"), "not a directory").unwrap();
        let (source, _) = source_session_with_active_goal("source");
        let before = serde_json::to_value(&source.messages).unwrap();
        let version = source.trajectory_version;
        let transfer = transfer_goal_ownership(
            &source.messages,
            source.goal.as_ref(),
            &[],
            "source",
            "target",
            "agent",
            1,
        );
        let session = Arc::new(tokio::sync::Mutex::new(source));
        let result = commit_transition_snapshot(
            gcx,
            session.clone(),
            version,
            transition_identity_snapshot("mode_transition"),
            &transfer,
            None,
        )
        .await;
        assert!(result.is_err());
        let source = session.lock().await;
        assert_eq!(serde_json::to_value(&source.messages).unwrap(), before);
        assert!(source.goal.as_ref().unwrap().active);
    }

    #[tokio::test]
    async fn staged_destination_is_goal_free_when_source_save_fails() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let (mut source, _) = source_session_with_active_goal("invalid/source");
        let before = serde_json::to_vec(&source.messages).unwrap();
        let transfer = transition_goal(
            &source.messages,
            source.goal.as_ref(),
            "invalid/source",
            "staged-target",
            "agent",
        )
        .unwrap();
        let mut snapshot = transition_identity_snapshot("mode_transition");
        snapshot.chat_id = "staged-target".into();
        snapshot.parent_id = None;
        snapshot.root_chat_id = None;
        snapshot.goal = transfer.target_goal.clone();
        snapshot.goal_ledger = refact_chat_api::seed_transferred_goal_ledger(
            snapshot.goal.as_ref().unwrap(),
            "invalid/source",
            1,
        );
        insert_goal_messages_before_plan(&mut snapshot.messages, transfer.target_messages.clone());
        snapshot.messages = transition_report(
            &snapshot.messages,
            1,
            "model".into(),
            "agent".into(),
            "agent".into(),
        )
        .unwrap();
        let error = persist_transition(gcx.clone(), &mut source, &snapshot, &transfer)
            .await
            .unwrap_err();
        assert!(error.contains("inactive destination staged-target"));
        assert_eq!(serde_json::to_vec(&source.messages).unwrap(), before);
        assert!(source.goal.as_ref().unwrap().active);
        let saved: serde_json::Value = serde_json::from_slice(
            &tokio::fs::read(
                crate::chat::trajectories::find_trajectory_path(gcx.clone(), "staged-target")
                    .await
                    .unwrap(),
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert!(saved.get("goal").is_none_or(serde_json::Value::is_null));
        let messages: Vec<ChatMessage> = serde_json::from_value(saved["messages"].clone()).unwrap();
        assert_eq!(messages.len(), 1);
        let active = refact_core::active_context::active_context(&messages).unwrap();
        assert!(!active.messages.iter().any(|m| m.role == "goal"));
    }

    #[tokio::test]
    async fn reservation_rejects_concurrent_rebuild_and_settings_change() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let source = Arc::new(tokio::sync::Mutex::new(
            crate::chat::types::ChatSession::new("source".into()),
        ));
        let reservation = TransitionReservation::acquire(source.clone())
            .await
            .unwrap();
        assert!(TransitionReservation::acquire(source.clone())
            .await
            .is_err());
        let version = source.lock().await.trajectory_version;
        source.lock().await.thread.model = "changed".into();
        let transferred = transfer_goal_ownership(&[], None, &[], "source", "target", "agent", 1);
        let result = commit_transition_snapshot(
            gcx,
            source,
            version,
            transition_identity_snapshot("mode_transition"),
            &transferred,
            Some(&reservation),
        )
        .await;
        assert!(result.unwrap_err().contains("Source trajectory changed"));
    }

    #[tokio::test]
    async fn staged_commit_under_source_lock_saves_single_owner_and_archive() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let (mut session, _) = source_session_with_active_goal("durable-source");
        let mut raw = vec![ChatMessage::new("user".into(), "archive".into())];
        raw.extend(
            transition_report(
                &session.messages,
                1,
                "model".into(),
                "agent".into(),
                "agent".into(),
            )
            .unwrap(),
        );
        session.messages = raw;
        let before = serde_json::to_vec(&session.messages).unwrap();
        let transfer = transition_goal(
            &session.messages,
            session.goal.as_ref(),
            "durable-source",
            "durable-target",
            "agent",
        )
        .unwrap();
        let mut snapshot = transition_identity_snapshot("mode_transition");
        snapshot.chat_id = "durable-target".into();
        snapshot.parent_id = None;
        snapshot.root_chat_id = None;
        snapshot.goal = transfer.target_goal.clone();
        snapshot.goal_ledger = refact_chat_api::seed_transferred_goal_ledger(
            snapshot.goal.as_ref().unwrap(),
            "durable-source",
            1,
        );
        insert_goal_messages_before_plan(&mut snapshot.messages, transfer.target_messages.clone());
        snapshot.messages = transition_report(
            &snapshot.messages,
            1,
            "model".into(),
            "agent".into(),
            "agent".into(),
        )
        .unwrap();
        let session = Arc::new(tokio::sync::Mutex::new(session));
        gcx.chat_sessions
            .write()
            .await
            .insert("durable-source".into(), session.clone());
        let mut source = session.lock().await;
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            persist_transition(gcx.clone(), &mut source, &snapshot, &transfer),
        )
        .await
        .expect("source lock must not deadlock persistence")
        .unwrap();
        assert_eq!(serde_json::to_vec(&source.messages).unwrap(), before);
        assert_eq!(
            source.goal.as_ref().unwrap().status,
            GoalStatus::Transferred
        );
        for (id, active) in [("durable-source", false), ("durable-target", true)] {
            let saved: serde_json::Value = serde_json::from_slice(
                &tokio::fs::read(
                    crate::chat::trajectories::find_trajectory_path(gcx.clone(), id)
                        .await
                        .unwrap(),
                )
                .await
                .unwrap(),
            )
            .unwrap();
            assert_eq!(saved["goal"]["active"], active);
            let messages: Vec<ChatMessage> =
                serde_json::from_value(saved["messages"].clone()).unwrap();
            if !active {
                assert_eq!(serde_json::to_vec(&messages).unwrap(), before);
            } else {
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].role, "compression_report");
            }
        }
    }

    #[test]
    fn static_transform_request_accepts_expected_version() {
        let req: TransformRequest =
            serde_json::from_value(json!({"options": {}, "expected_trajectory_version": 42}))
                .unwrap();
        assert_eq!(req.expected_trajectory_version, Some(42));
    }

    #[test]
    fn llm_compress_request_parse_error_is_redacted() {
        let error = parse_llm_compress_request(&hyper::body::Bytes::from_static(
            br#"{"options":{"summary_model":"sk-secret""#,
        ))
        .unwrap_err();
        assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        assert_eq!(error.message, "Invalid JSON request");
        assert!(!error.message.contains("sk-secret"));
    }

    fn sample_worktree(root: &std::path::Path) -> crate::worktrees::types::WorktreeMeta {
        crate::worktrees::types::WorktreeMeta {
            id: "wt-transition".to_string(),
            kind: "chat".to_string(),
            root: root.join("worktree"),
            source_workspace_root: root.to_path_buf(),
            repo_root: root.to_path_buf(),
            branch: Some("refact/chat/preserve".to_string()),
            base_branch: Some("dev".to_string()),
            base_commit: Some("abc123".to_string()),
            task_id: None,
            card_id: None,
            agent_id: None,
            enforce: true,
        }
    }

    fn transition_identity_snapshot(link_type: &str) -> TrajectorySnapshot {
        let mut snapshot = TrajectorySnapshot {
            recovery: None,
            recovery_policy: None,
            pending_browser_decision: None,
            pending_deliveries: Vec::new(),
            dropped_deliveries: Vec::new(),
            goal: None,
            goal_ledger: Vec::new(),
            goal_verification_blocked_until_ms: None,
            chat_id: "transition-identity".to_string(),
            title: String::new(),
            model: "gpt-4".to_string(),
            mode: "task_planner".to_string(),
            tool_use: "agent".to_string(),
            messages: vec![crate::call_validation::ChatMessage::new(
                "user".to_string(),
                "hello".to_string(),
            )],
            created_at: chrono::Utc::now().to_rfc3339(),
            boost_reasoning: false,
            checkpoints_enabled: true,
            context_tokens_cap: None,
            auto_compression_cap: None,
            auto_compression_cap_pending: true,
            include_project_info: true,
            is_title_generated: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            autonomous_no_confirm: false,
            version: 1,
            task_meta: None,
            worktree: None,
            parent_id: Some("source-chat".to_string()),
            link_type: Some(link_type.to_string()),
            root_chat_id: Some("source-chat".to_string()),
            reasoning_effort: None,
            thinking_budget: None,
            temperature: None,
            frequency_penalty: None,
            max_tokens: None,
            parallel_tool_calls: None,
            previous_response_id: Some("resp_source".to_string()),
            active_skill: None,
            auto_enrichment_enabled: None,
            buddy_meta: None,
            auto_compact_enabled: None,
            frozen_request_prefix: Some(refact_chat_api::FrozenRequestPrefix {
                schema_version: 1,
                created_at: "2026-05-29T00:00:00Z".to_string(),
                system_prompt: Some("source system".to_string()),
                tools_canonical: Some(
                    json!([{"type":"function","function":{"name":"source_tool"}}]),
                ),
            }),
            claude_code_identity: Some(refact_chat_api::ClaudeCodeIdentity {
                device_id: "source-device".to_string(),
                session_id: "source-session".to_string(),
            }),
            reactive_compact_attempts: None,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
            pending_questions: Vec::new(),
        };
        reset_transition_snapshot_identity(&mut snapshot);
        snapshot
    }

    fn plan_message(content: &str) -> ChatMessage {
        crate::chat::internal_roles::plan("task_agent", 1, content, None)
    }

    fn source_session_with_active_goal(
        chat_id: &str,
    ) -> (crate::chat::types::ChatSession, GoalBudget) {
        let mut session = crate::chat::types::ChatSession::new(chat_id.to_string());
        session.thread.mode = "agent".to_string();
        session.thread.tool_use = "agent".to_string();
        session.thread.model = "model".to_string();
        session.add_message(ChatMessage::new(
            "user".to_string(),
            "Please pursue this goal".to_string(),
        ));
        let budget = GoalBudget {
            max_turns: Some(7),
            max_minutes: Some(11),
            max_tokens: Some(13_000),
            max_cost_cents: None,
            cooldown_ms: 1_234,
            no_progress_token_threshold: 55,
            no_progress_turns: Some(3),
            explicit: false,
        };
        session.install_goal("agent", "Ship the HTTP goal transfer", true, budget.clone());
        let goal = session.goal.as_mut().expect("goal installed");
        goal.progress = GoalProgress {
            turns_used: 4,
            tokens_used: 9_999,
            started_at_ms: 42,
            no_progress_turns: 2,
            last_nudge_at_ms: 77,
            cost_used_cents: 0,
        };
        goal.attempts.push(GoalAttempt {
            at_ms: 100,
            trigger: "finish".to_string(),
            verdict: "retry".to_string(),
            gaps: vec!["missing verification".to_string()],
            verifier_reply: "Run tests".to_string(),
            criteria_verdicts: Vec::new(),
        });
        (session, budget)
    }

    #[tokio::test]
    async fn http_mode_transition_goal_transfer_deactivates_source_and_preserves_progress() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            crate::test_tmp::scratch_dir("cfg"),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }

        let source_chat_id = "http-source-goal";
        let target_chat_id = "http-target-goal";
        let (mut source_session, budget) = source_session_with_active_goal(source_chat_id);
        source_session.install_plan("plan", "Approved execution plan");
        let original_goal = source_session.goal.clone().unwrap();
        source_session.set_goal_ledger(refact_chat_api::seed_transferred_goal_ledger(
            &original_goal,
            "prior-owner",
            100,
        ));
        source_session.goal_ledger_append(refact_chat_api::GoalLedgerOp::AttemptPushed {
            attempt: original_goal.attempts[0].clone(),
        });
        source_session.goal_set_status_reason(GoalStatus::Paused, "awaiting user approval");
        let source_messages = source_session.messages.clone();
        let source_goal = transition_source_goal(&source_session);
        gcx.chat_sessions.write().await.insert(
            source_chat_id.to_string(),
            Arc::new(tokio::sync::Mutex::new(source_session)),
        );

        let at_ms = 123_456_789;
        let mut target_messages = vec![plan_message("Target plan")];
        let transferred_goal = transfer_goal_into_transition_messages(
            &source_messages,
            source_goal.as_ref(),
            &[],
            source_chat_id,
            target_chat_id,
            "task_planner",
            &mut target_messages,
            at_ms,
        );

        assert!(transferred_goal.transferred());
        let target_goal = transferred_goal.target_goal.as_ref().unwrap();
        assert!(target_goal.active);
        assert_eq!(target_goal.status, GoalStatus::Paused);
        assert_eq!(
            target_goal.plan.as_ref().unwrap().content,
            "Approved execution plan"
        );
        let target_ledger =
            refact_chat_api::seed_transferred_goal_ledger(target_goal, source_chat_id, at_ms);
        assert_eq!(
            refact_chat_api::reduce_goal_ledger(&target_ledger)
                .unwrap()
                .status,
            GoalStatus::Paused
        );
        assert_eq!(
            target_goal.transferred_from.as_deref(),
            Some(source_chat_id)
        );
        assert_eq!(target_goal.transferred_to, None);
        assert_eq!(target_goal.budget, budget);
        assert_eq!(target_goal.progress.started_at_ms, 42);
        assert_eq!(target_goal.progress.turns_used, 4);
        assert_eq!(target_goal.progress.tokens_used, 9_999);
        assert_eq!(target_goal.progress.no_progress_turns, 2);
        assert_eq!(target_goal.progress.last_nudge_at_ms, 77);
        assert_eq!(target_goal.attempts.len(), 1);
        assert_eq!(target_messages[0].role, "goal");
        assert_eq!(target_messages[1].role, "event");
        assert_eq!(target_messages[2].role, "event");
        assert_eq!(target_messages[3].role, "plan");

        persist_live_source_goal_transfer(gcx.clone(), source_chat_id, &transferred_goal)
            .await
            .unwrap();

        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(source_chat_id)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        let source_goal = session.goal.as_ref().unwrap();
        assert!(!source_goal.active);
        assert_eq!(source_goal.status, GoalStatus::Transferred);
        assert_eq!(source_goal.transferred_to.as_deref(), Some(target_chat_id));
        let source_goal_message = session
            .messages
            .iter()
            .find(|message| message.role == "goal")
            .unwrap();
        assert_eq!(source_goal_message.extra["goal"]["active"], json!(false));
        assert_eq!(
            source_goal_message.extra["goal"]["status"],
            json!("transferred")
        );
        assert_eq!(
            source_goal_message.extra["goal"]["transferred_to"],
            json!(target_chat_id)
        );
    }

    #[test]
    fn http_mode_transition_goal_transfer_only_active_goals_transfer() {
        for status in [
            GoalStatus::Paused,
            GoalStatus::Stopped,
            GoalStatus::Completed,
            GoalStatus::BudgetExhausted,
            GoalStatus::NoProgress,
            GoalStatus::Verifying,
            GoalStatus::Transferred,
        ] {
            let (mut session, _) = source_session_with_active_goal("gate-source");
            session.goal.as_mut().unwrap().status = status;
            let mut target_messages = vec![plan_message("Target plan")];

            let transferred_goal = transfer_goal_into_transition_messages(
                &session.messages,
                session.goal.as_ref(),
                &[],
                "gate-source",
                "gate-target",
                "task_agent",
                &mut target_messages,
                1,
            );

            assert!(
                !transferred_goal.transferred(),
                "status {status:?} must not transfer"
            );
            assert!(transferred_goal.target_goal.is_none());
            assert_eq!(target_messages.len(), 1);
        }

        let (session, _) = source_session_with_active_goal("gate-source-active");
        let mut target_messages = vec![plan_message("Target plan")];
        let transferred_goal = transfer_goal_into_transition_messages(
            &session.messages,
            session.goal.as_ref(),
            &[],
            "gate-source-active",
            "gate-target",
            "task_agent",
            &mut target_messages,
            1,
        );
        assert!(transferred_goal.transferred());
    }

    #[test]
    fn http_mode_transition_goal_transfer_respects_live_stop_over_pinned_meta() {
        let (mut session, _) = source_session_with_active_goal("gate-live-stop");
        {
            let index = session
                .messages
                .iter()
                .position(|message| message.role == "goal")
                .unwrap();
            let meta = session.messages[index]
                .extra
                .get_mut("goal")
                .unwrap()
                .as_object_mut()
                .unwrap();
            meta.insert("status".to_string(), json!("active"));
            meta.insert("active".to_string(), json!(true));
        }
        session.goal.as_mut().unwrap().status = GoalStatus::Stopped;
        let mut target_messages = vec![plan_message("Target plan")];

        let transferred_goal = transfer_goal_into_transition_messages(
            &session.messages,
            session.goal.as_ref(),
            &[],
            "gate-live-stop",
            "gate-target",
            "task_agent",
            &mut target_messages,
            1,
        );

        assert!(!transferred_goal.transferred());
    }

    #[test]
    fn trajectory_ops_mode_transition_snapshot_does_not_copy_source_identity() {
        let snapshot = transition_identity_snapshot("mode_transition");

        assert_eq!(snapshot.link_type.as_deref(), Some("mode_transition"));
        assert!(snapshot.previous_response_id.is_none());
        assert!(snapshot.frozen_request_prefix.is_none());
        assert!(snapshot.claude_code_identity.is_none());
    }

    #[test]
    fn mode_transition_response_serializes_optional_planner_metadata() {
        let response = ModeTransitionApplyResponse {
            new_chat_id: "planner-chat".to_string(),
            messages_count: 3,
            root_chat_id: Some("planner-chat".to_string()),
        };

        let raw = serde_json::to_value(response).unwrap();
        assert_eq!(raw["new_chat_id"], "planner-chat");
        assert_eq!(raw["messages_count"], 3);
        assert_eq!(raw["root_chat_id"], "planner-chat");
    }

    #[tokio::test]
    async fn generic_mode_transition_rejects_task_planner_target_before_session_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            crate::test_tmp::scratch_dir("cfg"),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        let body = hyper::body::Bytes::from_static(br#"{"target_mode":" TASK_PLANNER "}"#);

        let err = handle_mode_transition_apply(State(app), Path("missing-chat".to_string()), body)
            .await
            .unwrap_err();

        assert_eq!(err.status_code, StatusCode::BAD_REQUEST);
        assert_eq!(
            err.message,
            "Use /v1/tasks/:task_id/planner-chats/from-transition for task_planner transitions"
        );
    }

    #[tokio::test]
    async fn planner_from_transition_rejects_non_planner_target_before_task_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            crate::test_tmp::scratch_dir("cfg"),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        let body = hyper::body::Bytes::from_static(
            br#"{"source_chat_id":"source-chat","target_mode":"agent"}"#,
        );

        let err =
            handle_planner_from_transition(State(app), Path("missing-task".to_string()), body)
                .await
                .unwrap_err();

        assert_eq!(err.status_code, StatusCode::BAD_REQUEST);
        assert_eq!(err.message, "Only task_planner chats can be created here");
    }

    #[tokio::test]
    async fn save_transition_snapshot_preserves_worktree_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            crate::test_tmp::scratch_dir("cfg"),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }
        let snapshot = TrajectorySnapshot {
            recovery: None,
            recovery_policy: None,
            pending_browser_decision: None,
            pending_deliveries: Vec::new(),
            dropped_deliveries: Vec::new(),
            goal: None,
            goal_ledger: Vec::new(),
            goal_verification_blocked_until_ms: None,
            chat_id: "transition-chat".to_string(),
            title: String::new(),
            model: "gpt-4".to_string(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            messages: vec![crate::call_validation::ChatMessage::new(
                "user".to_string(),
                "hello".to_string(),
            )],
            created_at: chrono::Utc::now().to_rfc3339(),
            boost_reasoning: false,
            checkpoints_enabled: true,
            context_tokens_cap: None,
            auto_compression_cap: None,
            auto_compression_cap_pending: true,
            include_project_info: true,
            is_title_generated: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            autonomous_no_confirm: false,
            version: 1,
            task_meta: None,
            worktree: Some(sample_worktree(dir.path())),
            parent_id: None,
            link_type: None,
            root_chat_id: Some("source-chat".to_string()),
            reasoning_effort: None,
            thinking_budget: None,
            temperature: None,
            frequency_penalty: None,
            max_tokens: None,
            parallel_tool_calls: None,
            previous_response_id: None,
            active_skill: None,
            auto_enrichment_enabled: None,
            buddy_meta: None,
            auto_compact_enabled: None,
            frozen_request_prefix: None,
            claude_code_identity: None,
            reactive_compact_attempts: None,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
            pending_questions: Vec::new(),
        };

        save_trajectory_snapshot_with_parent(gcx, snapshot, "source-chat", "mode_transition")
            .await
            .unwrap();

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join("source-chat")
            .join("transition-chat.json");
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(raw["parent_id"], "source-chat");
        assert_eq!(raw["link_type"], "mode_transition");
        assert_eq!(raw["worktree"]["id"], "wt-transition");
        assert_eq!(
            raw["worktree"]["root"],
            dir.path().join("worktree").display().to_string()
        );
    }
}
