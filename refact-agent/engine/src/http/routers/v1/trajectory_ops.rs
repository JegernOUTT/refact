use std::sync::Arc;
use axum::extract::Path;
use axum::http::{Response, StatusCode};
use axum::Extension;
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use uuid::Uuid;

use crate::call_validation::ChatContent;
use crate::chat::trajectory_ops::{CompressOptions, HandoffOptions, compress_in_place, handoff_select};
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
    pub before_tokens: usize,
    pub after_tokens: usize,
    pub estimated_reduction_percent: usize,
    pub actions: Vec<String>,
}

#[derive(Serialize)]
pub struct TransformApplyResponse {
    pub success: bool,
    pub new_token_count: usize,
}

#[derive(Serialize)]
pub struct HandoffPreviewResponse {
    pub new_chat_title: String,
    pub summary: String,
    pub key_files: Vec<String>,
    pub estimated_tokens: usize,
}

#[derive(Serialize)]
pub struct HandoffApplyResponse {
    pub success: bool,
    pub new_chat_id: String,
}

fn describe_transform_actions(opts: &CompressOptions) -> Vec<String> {
    let mut actions = Vec::new();
    if opts.drop_all_context {
        actions.push("Drop all context_file messages".to_string());
    } else if opts.dedup_and_compress_context {
        actions.push("Deduplicate and compress context files".to_string());
    }
    if opts.compress_non_agentic_tools {
        actions.push("Compress non-agentic tool results".to_string());
    }
    actions.push("Remove invalid tool calls and orphan results".to_string());
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

    let reduction_percent = if stats.before_approx_tokens > 0 {
        ((stats.before_approx_tokens - stats.after_approx_tokens) * 100) / stats.before_approx_tokens
    } else {
        0
    };

    let response = TransformPreviewResponse {
        before_tokens: stats.before_approx_tokens,
        after_tokens: stats.after_approx_tokens,
        estimated_reduction_percent: reduction_percent,
        actions: describe_transform_actions(&req.options),
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
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

        if session.runtime.state != SessionState::Idle {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                format!("Session is not idle, current state: {:?}", session.runtime.state),
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

    let response = TransformApplyResponse {
        success: true,
        new_token_count: stats.after_approx_tokens,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
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

    let preview_opts = HandoffOptions {
        llm_summary_for_excluded: false,
        ..req.options.clone()
    };

    let thread_title = {
        let session = session_arc.lock().await;
        session.thread.title.clone()
    };

    let (selected_messages, stats, _) = handoff_select(&messages, &preview_opts, gcx.clone()).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let key_files: Vec<String> = selected_messages
        .iter()
        .filter(|m| m.role == "context_file")
        .filter_map(|m| {
            if let ChatContent::ContextFiles(files) = &m.content {
                files.first().map(|f| f.file_name.clone())
            } else {
                None
            }
        })
        .collect();

    let response = HandoffPreviewResponse {
        new_chat_title: format!("Handoff from: {}", thread_title),
        summary: String::new(),
        key_files,
        estimated_tokens: stats.after_approx_tokens,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
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

        if session.runtime.state != SessionState::Idle {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                format!("Session is not idle, current state: {:?}", session.runtime.state),
            ));
        }

        (session.messages.clone(), session.thread.clone(), session.thread.task_meta.clone())
    };

    let (selected_messages, _stats, _) = handoff_select(&messages, &req.options, gcx.clone()).await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let new_chat_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let snapshot = TrajectorySnapshot {
        chat_id: new_chat_id.clone(),
        title: format!("Handoff from: {}", thread.title),
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
    };

    save_trajectory_snapshot_with_parent(gcx.clone(), snapshot, &chat_id, "handoff").await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let response = HandoffApplyResponse {
        success: true,
        new_chat_id,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
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
