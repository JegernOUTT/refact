use std::sync::Arc;
use axum::extract::Path;
use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use crate::chat::trajectory_ops::{
    CompressOptions, HandoffOptions, TransformStats, compress_in_place,
    demote_goal_ownership_for_branch, handoff_select, sanitize_messages_for_new_thread,
};
use crate::call_validation::ChatMessage;
use crate::integrations::browser_runtime::find_runtime_by_chat_id;
use crate::agentic::mode_transition::{
    AgenticPathContext, GoalTransferResult, analyze_mode_transition, assemble_new_chat,
    insert_goal_messages_before_plan, transfer_goal_ownership,
};
use crate::chat::types::SessionState;
use crate::chat::get_or_create_session_with_trajectory;
use refact_chat_api::GoalSnapshot;
use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;
use crate::custom_error::ScratchError;
use crate::yaml_configs::customization_registry::map_legacy_mode_to_id;

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
        crate::chat::trajectories::try_save_trajectory(
            AppState::from_gcx(gcx.clone()).await,
            session_arc,
        )
        .await?;
    }
    Ok(())
}

async fn create_initial_plan_document_for_transition(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    plan_text: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(plan_text) = plan_text.map(str::trim).filter(|text| !text.is_empty()) else {
        return (None, None);
    };
    let result = async {
        let documents_dir =
            crate::tools::tool_task_documents::documents_dir_for_task(gcx.clone(), task_id).await?;
        let slug = crate::tools::tool_task_documents::next_available_slug_at(
            &documents_dir,
            "initial-plan",
        )
        .await?;
        crate::tools::tool_task_documents::create_document_at(
            &documents_dir,
            &slug,
            "Initial Plan",
            "plan",
            plan_text,
            true,
            Vec::new(),
            "planner",
        )
        .await?;
        Ok::<String, String>(slug)
    }
    .await;
    match result {
        Ok(slug) => (Some(slug), None),
        Err(error) => {
            tracing::warn!(
                "failed to create initial-plan document for task {}: {}",
                task_id,
                error
            );
            (None, Some(error))
        }
    }
}

#[derive(Deserialize)]
pub struct TransformRequest {
    pub options: CompressOptions,
}

#[derive(Deserialize)]
pub struct HandoffRequest {
    pub options: HandoffOptions,
}

#[derive(Serialize)]
pub struct TransformPreviewResponse {
    pub stats: TransformStats,
    pub actions: Vec<String>,
}

#[derive(Serialize)]
pub struct TransformApplyResponse {
    pub stats: TransformStats,
}

#[derive(Serialize)]
pub struct HandoffPreviewResponse {
    pub stats: TransformStats,
    pub actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_summary: Option<String>,
}

#[derive(Serialize)]
pub struct HandoffApplyResponse {
    pub new_chat_id: String,
    pub stats: TransformStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_runtime_id: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_plan_document: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_plan_error: Option<String>,
}

#[derive(Deserialize)]
pub struct PlannerFromTransitionRequest {
    pub source_chat_id: String,
    #[serde(default)]
    pub target_mode_description: String,
    #[serde(default)]
    pub target_mode: Option<String>,
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
        actions.push(
            "Compress tool results (preserving deep_research, subagent, strategic_planning)"
                .to_string(),
        );
    }
    if opts.strip_metering {
        actions.push("Strip metering information".to_string());
    }
    actions.push("Remove invalid tool calls and orphan results".to_string());
    actions
}

fn describe_handoff_actions(opts: &HandoffOptions) -> Vec<String> {
    let mut actions = Vec::new();
    if opts.include_all_user_assistant_only {
        actions.push(
            "Include all user and assistant messages only (strip system, tools, context)"
                .to_string(),
        );
    }
    if opts.include_last_user_plus {
        actions.push("Include last user message and all following".to_string());
    }
    if opts.include_all_opened_context {
        actions.push("Include all opened context files".to_string());
    }
    if opts.include_all_edited_context {
        actions.push("Include all edited context (diffs)".to_string());
    }
    if opts.include_agentic_tools {
        actions.push("Include agentic tool calls and results".to_string());
    }
    if opts.llm_summary_for_excluded {
        actions.push("Generate LLM summary for excluded content".to_string());
    }
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

    let mut messages = {
        let session = session_arc.lock().await;
        session.messages.clone()
    };

    let stats = compress_in_place(&mut messages, &req.options)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = TransformPreviewResponse {
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

        if session.runtime.state != SessionState::Idle
            && session.runtime.state != SessionState::Error
        {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                format!(
                    "Session is not idle or error, current state: {:?}",
                    session.runtime.state
                ),
            ));
        }

        let stats = compress_in_place(&mut session.messages, &req.options)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

        session.increment_version();
        let snapshot = session.snapshot();
        session.emit(snapshot);

        stats
    };

    crate::chat::trajectories::maybe_save_trajectory(
        AppState::from_gcx(gcx.clone()).await,
        session_arc,
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

pub async fn handle_handoff_preview(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let req: HandoffRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let sessions = gcx.chat_sessions.clone();
    let session_arc = get_or_create_session_with_trajectory(
        AppState::from_gcx(gcx.clone()).await,
        &sessions,
        &chat_id,
    )
    .await;

    let messages = {
        let session = session_arc.lock().await;
        session.messages.clone()
    };

    let (_, stats, _) = handoff_select(&messages, &req.options, gcx.clone(), false, &chat_id)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = HandoffPreviewResponse {
        stats,
        actions: describe_handoff_actions(&req.options),
        llm_summary: None,
    };

    let body = serde_json::to_vec(&response)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn handle_handoff_apply(
    State(app): State<AppState>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let req: HandoffRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let sessions = gcx.chat_sessions.clone();
    let session_arc = get_or_create_session_with_trajectory(
        AppState::from_gcx(gcx.clone()).await,
        &sessions,
        &chat_id,
    )
    .await;

    let (messages, thread, task_meta) = {
        let session = session_arc.lock().await;
        (
            session.messages.clone(),
            session.thread.clone(),
            session.thread.task_meta.clone(),
        )
    };

    let (selected_messages, stats, _) =
        handoff_select(&messages, &req.options, gcx.clone(), true, &chat_id)
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let mut selected_messages = sanitize_messages_for_new_thread(&selected_messages);
    demote_goal_ownership_for_branch(&mut selected_messages);

    let new_chat_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let mut snapshot = TrajectorySnapshot {
        goal: None,
        goal_ledger: Vec::new(),
        goal_verification_blocked_until_ms: None,
        chat_id: new_chat_id.clone(),
        title: thread.title.clone(),
        model: thread.model.clone(),
        mode: thread.mode.clone(),
        tool_use: thread.tool_use.clone(),
        messages: selected_messages,
        created_at: now,
        boost_reasoning: thread.boost_reasoning.unwrap_or(false),
        checkpoints_enabled: thread.checkpoints_enabled,
        context_tokens_cap: thread.context_tokens_cap,
        include_project_info: thread.include_project_info,
        is_title_generated: false,
        auto_approve_editing_tools: thread.auto_approve_editing_tools,
        auto_approve_dangerous_commands: thread.auto_approve_dangerous_commands,
        autonomous_no_confirm: thread.autonomous_no_confirm,
        version: 1,
        task_meta,
        worktree: thread.worktree.clone(),
        parent_id: Some(chat_id.clone()),
        link_type: Some("handoff".to_string()),
        root_chat_id: thread
            .root_chat_id
            .clone()
            .or_else(|| Some(chat_id.clone())),
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
    };
    reset_transition_snapshot_identity(&mut snapshot);

    save_trajectory_snapshot_with_parent(gcx.clone(), snapshot, &chat_id, "handoff")
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let browser_runtime_id = if let Some((runtime_id, runtime_arc)) = find_runtime_by_chat_id(
        crate::app_state::AppState::from_gcx(gcx.clone()).await,
        &chat_id,
    )
    .await
    {
        let mut rt = runtime_arc.lock().await;
        rt.detach();
        rt.reattach(&new_chat_id);
        rt.touch();
        drop(rt);
        Some(runtime_id)
    } else {
        None
    };

    let response = HandoffApplyResponse {
        new_chat_id,
        stats,
        browser_runtime_id,
    };

    let body = serde_json::to_vec(&response)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

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

    let (messages, thread, task_meta, source_goal, session_state) = {
        let session = session_arc.lock().await;
        (
            session.messages.clone(),
            session.thread.clone(),
            session.thread.task_meta.clone(),
            session.goal.clone(),
            session.runtime.state.clone(),
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

    let decisions = analyze_mode_transition(
        gcx.clone(),
        &messages,
        &target_mode,
        &req.target_mode_description,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let path_context = { AgenticPathContext::from_context(&*gcx) };
    let mut new_messages = assemble_new_chat(&path_context, &messages, &decisions)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let new_chat_id = Uuid::new_v4().to_string();
    let transferred_goal = transfer_goal_into_transition_messages(
        &messages,
        source_goal.as_ref(),
        &[],
        &chat_id,
        &new_chat_id,
        &target_mode,
        &mut new_messages,
        epoch_ms_now(),
    );
    persist_live_source_goal_transfer(gcx.clone(), &chat_id, &transferred_goal)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let new_messages = sanitize_messages_for_new_thread(&new_messages);
    let now = chrono::Utc::now().to_rfc3339();

    let root_chat_id = thread
        .root_chat_id
        .clone()
        .or_else(|| Some(chat_id.clone()));

    let mut snapshot = TrajectorySnapshot {
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
    };
    reset_transition_snapshot_identity(&mut snapshot);

    save_trajectory_snapshot_with_parent(gcx.clone(), snapshot, &chat_id, "mode_transition")
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = ModeTransitionApplyResponse {
        new_chat_id,
        messages_count: new_messages.len(),
        root_chat_id,
        initial_plan_document: None,
        initial_plan_error: None,
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

    let (messages, thread, source_goal, session_state) = {
        let session = session_arc.lock().await;
        (
            session.messages.clone(),
            session.thread.clone(),
            session.goal.clone(),
            session.runtime.state.clone(),
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

    let decisions = analyze_mode_transition(
        gcx.clone(),
        &messages,
        &target_mode,
        &req.target_mode_description,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let path_context = { AgenticPathContext::from_context(&*gcx) };
    let mut new_messages = assemble_new_chat(&path_context, &messages, &decisions)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let new_chat_id = crate::tasks::storage::next_planner_chat_id(gcx.clone(), &task_id)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let transferred_goal = transfer_goal_into_transition_messages(
        &messages,
        source_goal.as_ref(),
        &[],
        &req.source_chat_id,
        &new_chat_id,
        &target_mode,
        &mut new_messages,
        epoch_ms_now(),
    );
    persist_live_source_goal_transfer(gcx.clone(), &req.source_chat_id, &transferred_goal)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let new_messages = sanitize_messages_for_new_thread(&new_messages);
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
    };
    reset_transition_snapshot_identity(&mut snapshot);

    // task_meta is set, so this saves into the task's planner directory
    save_trajectory_snapshot_with_parent(
        gcx.clone(),
        snapshot,
        &req.source_chat_id,
        "mode_transition",
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let (initial_plan_document, initial_plan_error) =
        if target_mode.eq_ignore_ascii_case("task_planner") {
            create_initial_plan_document_for_transition(
                gcx.clone(),
                &task_id,
                decisions.initial_plan.as_deref(),
            )
            .await
        } else {
            (None, None)
        };

    let response = ModeTransitionApplyResponse {
        new_chat_id,
        messages_count: new_messages.len(),
        root_chat_id,
        initial_plan_document,
        initial_plan_error,
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
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }

        let source_chat_id = "http-source-goal";
        let target_chat_id = "http-target-goal";
        let (source_session, budget) = source_session_with_active_goal(source_chat_id);
        let source_messages = source_session.messages.clone();
        let source_goal = source_session.goal.clone();
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
            "task_agent",
            &mut target_messages,
            at_ms,
        );

        assert!(transferred_goal.transferred());
        let target_goal = transferred_goal.target_goal.as_ref().unwrap();
        assert!(target_goal.active);
        assert_eq!(target_goal.status, GoalStatus::Active);
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
        assert_eq!(target_messages[2].role, "plan");

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
    fn http_mode_transition_goal_transfer_no_goal_is_noop() {
        let source_messages = vec![ChatMessage::new("user".to_string(), "hello".to_string())];
        let mut target_messages = vec![plan_message("Target plan")];
        let original_role = target_messages[0].role.clone();
        let original_content = target_messages[0].content.content_text_only();

        let transferred_goal = transfer_goal_into_transition_messages(
            &source_messages,
            None,
            &[],
            "source-chat",
            "target-chat",
            "task_agent",
            &mut target_messages,
            1,
        );

        assert!(!transferred_goal.transferred());
        assert!(transferred_goal.target_goal.is_none());
        assert!(transferred_goal.source_goal.is_none());
        assert_eq!(target_messages.len(), 1);
        assert_eq!(target_messages[0].role, original_role);
        assert_eq!(
            target_messages[0].content.content_text_only(),
            original_content
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
    fn trajectory_ops_handoff_snapshot_does_not_copy_source_identity() {
        let snapshot = transition_identity_snapshot("handoff");

        assert_eq!(snapshot.link_type.as_deref(), Some("handoff"));
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
            initial_plan_document: Some("initial-plan".to_string()),
            initial_plan_error: None,
        };

        let raw = serde_json::to_value(response).unwrap();
        assert_eq!(raw["new_chat_id"], "planner-chat");
        assert_eq!(raw["messages_count"], 3);
        assert_eq!(raw["root_chat_id"], "planner-chat");
        assert_eq!(raw["initial_plan_document"], "initial-plan");
        assert!(raw.get("initial_plan_error").is_none());
    }

    #[tokio::test]
    async fn generic_mode_transition_rejects_task_planner_target_before_session_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
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
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
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
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }
        let snapshot = TrajectorySnapshot {
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
        };

        save_trajectory_snapshot_with_parent(gcx, snapshot, "source-chat", "mode_transition")
            .await
            .unwrap();

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
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

    #[tokio::test]
    async fn transition_initial_plan_document_failure_is_non_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
        )
        .await;
        create_initial_plan_document_for_transition(
            gcx,
            "missing-task",
            Some("Wave 0\n- Card T-1\n- Acceptance Criteria: tests pass"),
        )
        .await;
    }
}
