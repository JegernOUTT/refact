use std::sync::Arc;
use axum::Extension;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock as ARwLock;
use chrono::Utc;

use crate::global_context::GlobalContext;
use crate::tasks::types::{TaskMeta, TaskBoard, BoardCard, StatusUpdate, TaskStatus};
use crate::tasks::storage;

#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub name: String,
}

#[derive(Deserialize)]
pub struct UpdateBoardRequest {
    pub rev: u64,
    #[serde(flatten)]
    pub patch: BoardPatch,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BoardPatch {
    CreateCard {
        id: String,
        title: String,
        #[serde(default)]
        priority: Option<String>,
        #[serde(default)]
        depends_on: Vec<String>,
        #[serde(default)]
        instructions: String,
    },
    UpdateCard {
        id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        priority: Option<String>,
        #[serde(default)]
        depends_on: Option<Vec<String>>,
        #[serde(default)]
        instructions: Option<String>,
    },
    MoveCard {
        id: String,
        column: String,
    },
    AssignAgent {
        card_id: String,
        agent_id: String,
        agent_chat_id: String,
    },
    AddStatusUpdate {
        card_id: String,
        message: String,
    },
    SetFinalReport {
        card_id: String,
        report: String,
    },
    DeleteCard {
        id: String,
    },
}

pub async fn handle_list_tasks(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Json<Vec<TaskMeta>>, (StatusCode, String)> {
    let tasks = storage::list_tasks(gcx).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(tasks))
}

pub async fn handle_create_task(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Json(req): Json<CreateTaskRequest>,
) -> Result<Json<TaskMeta>, (StatusCode, String)> {
    let meta = storage::create_task(gcx, &req.name).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(meta))
}

pub async fn handle_get_task(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let meta = storage::load_task_meta(gcx.clone(), &task_id).await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    let board = storage::load_board(gcx.clone(), &task_id).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let ready = board.get_ready_cards();

    Ok(Json(json!({
        "meta": meta,
        "board_summary": {
            "rev": board.rev,
            "cards_count": board.cards.len(),
            "ready": ready,
        }
    })))
}

pub async fn handle_delete_task(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    storage::delete_task(gcx, &task_id).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({"deleted": true})))
}

pub async fn handle_get_board(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
) -> Result<Json<TaskBoard>, (StatusCode, String)> {
    let board = storage::load_board(gcx, &task_id).await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(board))
}

pub async fn handle_patch_board(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateBoardRequest>,
) -> Result<Json<TaskBoard>, (StatusCode, String)> {
    let mut board = storage::load_board(gcx.clone(), &task_id).await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    if board.rev != req.rev {
        return Err((StatusCode::CONFLICT, format!("Board rev mismatch: expected {}, got {}", board.rev, req.rev)));
    }

    let now = Utc::now().to_rfc3339();

    match req.patch {
        BoardPatch::CreateCard { id, title, priority, depends_on, instructions } => {
            if board.cards.iter().any(|c| c.id == id) {
                return Err((StatusCode::BAD_REQUEST, format!("Card {} already exists", id)));
            }
            board.cards.push(BoardCard {
                id,
                title,
                column: "planned".into(),
                priority: priority.unwrap_or_else(|| "P1".into()),
                depends_on,
                instructions,
                assignee: None,
                agent_chat_id: None,
                status_updates: vec![],
                final_report: None,
                created_at: now.clone(),
                started_at: None,
                completed_at: None,
            });
        }
        BoardPatch::UpdateCard { id, title, priority, depends_on, instructions } => {
            let card = board.get_card_mut(&id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Card {} not found", id)))?;
            if let Some(t) = title { card.title = t; }
            if let Some(p) = priority { card.priority = p; }
            if let Some(d) = depends_on { card.depends_on = d; }
            if let Some(i) = instructions { card.instructions = i; }
        }
        BoardPatch::MoveCard { id, column } => {
            let card = board.get_card_mut(&id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Card {} not found", id)))?;
            let valid_columns = ["planned", "doing", "done", "failed"];
            if !valid_columns.contains(&column.as_str()) {
                return Err((StatusCode::BAD_REQUEST, format!("Invalid column: {}", column)));
            }
            if column == "doing" && card.started_at.is_none() {
                card.started_at = Some(now.clone());
            }
            if (column == "done" || column == "failed") && card.completed_at.is_none() {
                card.completed_at = Some(now.clone());
            }
            card.column = column;
        }
        BoardPatch::AssignAgent { card_id, agent_id, agent_chat_id } => {
            let card = board.get_card_mut(&card_id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Card {} not found", card_id)))?;
            card.assignee = Some(agent_id);
            card.agent_chat_id = Some(agent_chat_id);
            if card.started_at.is_none() {
                card.started_at = Some(now.clone());
            }
        }
        BoardPatch::AddStatusUpdate { card_id, message } => {
            let card = board.get_card_mut(&card_id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Card {} not found", card_id)))?;
            card.status_updates.push(StatusUpdate {
                timestamp: now.clone(),
                message,
            });
        }
        BoardPatch::SetFinalReport { card_id, report } => {
            let card = board.get_card_mut(&card_id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Card {} not found", card_id)))?;
            card.final_report = Some(report);
        }
        BoardPatch::DeleteCard { id } => {
            board.cards.retain(|c| c.id != id);
        }
    }

    board.rev += 1;
    storage::save_board(gcx.clone(), &task_id, &board).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    storage::update_task_stats(gcx, &task_id).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(board))
}

pub async fn handle_get_orchestrator_instructions(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let content = storage::load_orchestrator_instructions(gcx, &task_id).await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(json!({"content": content})))
}

#[derive(Deserialize)]
pub struct SetOrchestratorInstructionsRequest {
    pub content: String,
}

pub async fn handle_set_orchestrator_instructions(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
    Json(req): Json<SetOrchestratorInstructionsRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    storage::save_orchestrator_instructions(gcx, &task_id, &req.content).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({"saved": true})))
}

pub async fn handle_get_ready_cards(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let board = storage::load_board(gcx, &task_id).await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    let ready = board.get_ready_cards();
    Ok(Json(json!(ready)))
}

pub async fn handle_update_task_status(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateTaskStatusRequest>,
) -> Result<Json<TaskMeta>, (StatusCode, String)> {
    let mut meta = storage::load_task_meta(gcx.clone(), &task_id).await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    meta.status = req.status;
    meta.updated_at = Utc::now().to_rfc3339();
    storage::save_task_meta(gcx, &task_id, &meta).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(meta))
}

#[derive(Deserialize)]
pub struct UpdateTaskStatusRequest {
    pub status: TaskStatus,
}

pub async fn handle_list_task_trajectories(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path((task_id, role)): Path<(String, String)>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    let ids = storage::list_task_trajectories(gcx, &task_id, &role, None).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(ids))
}
