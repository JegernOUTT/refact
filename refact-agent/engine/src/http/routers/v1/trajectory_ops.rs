use std::sync::Arc;
use axum::extract::Path;
use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use crate::chat::trajectory_ops::{
    CompressOptions, HandoffOptions, TransformStats, compress_in_place, handoff_select,
    sanitize_messages_for_new_thread,
};
use crate::integrations::browser_runtime::find_runtime_by_chat_id;
use crate::agentic::mode_transition::{
    AgenticPathContext, analyze_mode_transition, assemble_new_chat,
};
use crate::chat::types::SessionState;
use crate::chat::get_or_create_session_with_trajectory;
use crate::chat::trajectories::TrajectorySnapshot;
use crate::custom_error::ScratchError;

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
}

#[derive(Deserialize)]
pub struct PlannerFromTransitionRequest {
    pub source_chat_id: String,
    #[serde(default)]
    pub target_mode_description: String,
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
    let session_arc = get_or_create_session_with_trajectory(AppState::from_gcx(gcx.clone()).await, &sessions, &chat_id).await;

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
    let session_arc = get_or_create_session_with_trajectory(AppState::from_gcx(gcx.clone()).await, &sessions, &chat_id).await;

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

    crate::chat::trajectories::maybe_save_trajectory(AppState::from_gcx(gcx.clone()).await, session_arc).await;

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
    let session_arc = get_or_create_session_with_trajectory(AppState::from_gcx(gcx.clone()).await, &sessions, &chat_id).await;

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
    let session_arc = get_or_create_session_with_trajectory(AppState::from_gcx(gcx.clone()).await, &sessions, &chat_id).await;

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

    let selected_messages = sanitize_messages_for_new_thread(&selected_messages);

    let new_chat_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let snapshot = TrajectorySnapshot {
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
    };

    save_trajectory_snapshot_with_parent(gcx.clone(), snapshot, &chat_id, "handoff")
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let browser_runtime_id = if let Some((runtime_id, runtime_arc)) =
        find_runtime_by_chat_id(crate::app_state::AppState::from_gcx(gcx.clone()).await, &chat_id).await
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

    let sessions = gcx.chat_sessions.clone();
    let session_arc = get_or_create_session_with_trajectory(AppState::from_gcx(gcx.clone()).await, &sessions, &chat_id).await;

    let (messages, thread, task_meta, session_state) = {
        let session = session_arc.lock().await;
        (
            session.messages.clone(),
            session.thread.clone(),
            session.thread.task_meta.clone(),
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
        &req.target_mode,
        &req.target_mode_description,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let path_context = {
        AgenticPathContext::from_context(&*gcx)
    };
    let new_messages = assemble_new_chat(&path_context, &messages, &decisions)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let new_messages = sanitize_messages_for_new_thread(&new_messages);
    let new_chat_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let snapshot = TrajectorySnapshot {
        chat_id: new_chat_id.clone(),
        title: String::new(),
        model: thread.model.clone(),
        mode: req.target_mode.clone(),
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
    };

    save_trajectory_snapshot_with_parent(gcx.clone(), snapshot, &chat_id, "mode_transition")
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = ModeTransitionApplyResponse {
        new_chat_id,
        messages_count: new_messages.len(),
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

    // Verify the task exists before doing any work
    crate::tasks::storage::load_task_meta(gcx.clone(), &task_id)
        .await
        .map_err(|e| ScratchError::new(StatusCode::NOT_FOUND, e))?;

    let sessions = gcx.chat_sessions.clone();
    let session_arc =
        get_or_create_session_with_trajectory(AppState::from_gcx(gcx.clone()).await, &sessions, &req.source_chat_id).await;

    let (messages, thread, session_state) = {
        let session = session_arc.lock().await;
        (
            session.messages.clone(),
            session.thread.clone(),
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

    let target_mode = "task_planner".to_string();

    let decisions = analyze_mode_transition(
        gcx.clone(),
        &messages,
        &target_mode,
        &req.target_mode_description,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let path_context = {
        AgenticPathContext::from_context(&*gcx)
    };
    let new_messages = assemble_new_chat(&path_context, &messages, &decisions)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let new_messages = sanitize_messages_for_new_thread(&new_messages);

    let new_chat_id = crate::tasks::storage::next_planner_chat_id(gcx.clone(), &task_id)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let now = chrono::Utc::now().to_rfc3339();

    let task_meta = crate::chat::types::TaskMeta {
        task_id: task_id.clone(),
        role: "planner".to_string(),
        agent_id: None,
        card_id: None,
        planner_chat_id: Some(new_chat_id.clone()),
    };

    let snapshot = TrajectorySnapshot {
        chat_id: new_chat_id.clone(),
        title: String::new(),
        model: thread.model.clone(),
        mode: target_mode,
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
        root_chat_id: thread
            .root_chat_id
            .clone()
            .or_else(|| Some(req.source_chat_id.clone())),
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
    };

    // task_meta is set, so this saves into the task's planner directory
    save_trajectory_snapshot_with_parent(
        gcx.clone(),
        snapshot,
        &req.source_chat_id,
        "mode_transition",
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = ModeTransitionApplyResponse {
        new_chat_id,
        messages_count: new_messages.len(),
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

    #[tokio::test]
    async fn save_transition_snapshot_preserves_worktree_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() =
                vec![dir.path().to_path_buf()];
            drop(gcx);
            gcx.cache_dir = dir.path().join("cache");
        }
        let snapshot = TrajectorySnapshot {
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
}
