use std::sync::Arc;
use axum::extract::Path;
use axum::http::{Response, StatusCode};
use axum::Extension;
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use uuid::Uuid;

use crate::chat::trajectory_ops::{
    CompressOptions, HandoffOptions, TransformStats, compress_in_place, handoff_select,
    sanitize_messages_for_new_thread,
};
use crate::chat::types::SessionState;
use crate::chat::get_or_create_session_with_trajectory;
use crate::chat::trajectories::TrajectorySnapshot;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

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
        actions.push("Strip metering information (usage, coins)".to_string());
    }
    actions.push("Remove invalid tool calls and orphan results".to_string());
    actions
}

fn describe_handoff_actions(opts: &HandoffOptions) -> Vec<String> {
    let mut actions = Vec::new();
    if opts.include_all_user_assistant_only {
        actions.push("Include all user and assistant messages only (strip system, tools, context)".to_string());
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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: TransformRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let sessions = gcx.read().await.chat_sessions.clone();
    let session_arc = get_or_create_session_with_trajectory(gcx.clone(), &sessions, &chat_id).await;

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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: TransformRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let sessions = gcx.read().await.chat_sessions.clone();
    let session_arc = get_or_create_session_with_trajectory(gcx.clone(), &sessions, &chat_id).await;

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

    crate::chat::trajectories::maybe_save_trajectory(gcx.clone(), session_arc).await;

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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: HandoffRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let sessions = gcx.read().await.chat_sessions.clone();
    let session_arc = get_or_create_session_with_trajectory(gcx.clone(), &sessions, &chat_id).await;

    let messages = {
        let session = session_arc.lock().await;
        session.messages.clone()
    };

    let (_, stats, _) = handoff_select(
        &messages,
        &req.options,
        gcx.clone(),
        false,
        &chat_id,
    )
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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(chat_id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: HandoffRequest = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;

    let sessions = gcx.read().await.chat_sessions.clone();
    let session_arc = get_or_create_session_with_trajectory(gcx.clone(), &sessions, &chat_id).await;

    let (messages, thread, task_meta) = {
        let session = session_arc.lock().await;

        if session.runtime.state != SessionState::Idle
            && session.runtime.state != SessionState::Error
        {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                format!(
                    "Session is busy, current state: {:?}",
                    session.runtime.state
                ),
            ));
        }

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
        boost_reasoning: thread.boost_reasoning,
        checkpoints_enabled: thread.checkpoints_enabled,
        context_tokens_cap: thread.context_tokens_cap,
        include_project_info: thread.include_project_info,
        is_title_generated: false,
        automatic_patch: thread.automatic_patch,
        version: 1,
        task_meta,
        parent_id: Some(chat_id.clone()),
        link_type: Some("handoff".to_string()),
        root_chat_id: thread.root_chat_id.clone().or_else(|| Some(chat_id.clone())),
    };

    save_trajectory_snapshot_with_parent(gcx.clone(), snapshot, &chat_id, "handoff")
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = HandoffApplyResponse { new_chat_id, stats };

    let body = serde_json::to_vec(&response)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn save_trajectory_snapshot_with_parent(
    gcx: Arc<ARwLock<GlobalContext>>,
    snapshot: TrajectorySnapshot,
    parent_id: &str,
    link_type: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let messages_json: Vec<serde_json::Value> = snapshot
        .messages
        .iter()
        .map(|m| serde_json::to_value(m).unwrap_or_default())
        .collect();

    let mut trajectory = serde_json::json!({
        "id": snapshot.chat_id,
        "title": snapshot.title,
        "model": snapshot.model,
        "mode": snapshot.mode,
        "tool_use": snapshot.tool_use,
        "messages": messages_json,
        "created_at": snapshot.created_at,
        "updated_at": now,
        "boost_reasoning": snapshot.boost_reasoning,
        "checkpoints_enabled": snapshot.checkpoints_enabled,
        "context_tokens_cap": snapshot.context_tokens_cap,
        "include_project_info": snapshot.include_project_info,
        "isTitleGenerated": snapshot.is_title_generated,
        "automatic_patch": snapshot.automatic_patch,
        "parent_id": parent_id,
        "link_type": link_type,
    });

    if let Some(ref root_chat_id) = snapshot.root_chat_id {
        trajectory["root_chat_id"] = serde_json::Value::String(root_chat_id.clone());
    }

    if let Some(ref task_meta) = snapshot.task_meta {
        trajectory["task_meta"] = serde_json::to_value(task_meta).unwrap_or_default();
    }

    let file_path = if let Some(ref task_meta) = snapshot.task_meta {
        let task_dir = crate::tasks::storage::get_task_dir(gcx.clone(), &task_meta.task_id).await?;
        let traj_dir = crate::tasks::storage::get_task_trajectory_dir(
            &task_dir,
            &task_meta.role,
            task_meta.agent_id.as_deref(),
        );
        tokio::fs::create_dir_all(&traj_dir)
            .await
            .map_err(|e| format!("Failed to create task trajectories dir: {}", e))?;
        traj_dir.join(format!("{}.json", snapshot.chat_id))
    } else {
        let trajectories_dir = crate::chat::trajectories::get_trajectories_dir(gcx.clone()).await?;
        tokio::fs::create_dir_all(&trajectories_dir)
            .await
            .map_err(|e| format!("Failed to create trajectories dir: {}", e))?;
        trajectories_dir.join(format!("{}.json", snapshot.chat_id))
    };

    let tmp_path = file_path.with_extension("json.tmp");
    let json_str = serde_json::to_string_pretty(&trajectory)
        .map_err(|e| format!("Failed to serialize trajectory: {}", e))?;
    tokio::fs::write(&tmp_path, &json_str)
        .await
        .map_err(|e| format!("Failed to write trajectory: {}", e))?;
    tokio::fs::rename(&tmp_path, &file_path)
        .await
        .map_err(|e| format!("Failed to rename trajectory: {}", e))?;

    tracing::info!(
        "Saved handoff trajectory {} (parent: {}, link: {}) to {:?}",
        snapshot.chat_id,
        parent_id,
        link_type,
        file_path
    );

    Ok(())
}
