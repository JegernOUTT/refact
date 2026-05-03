use std::sync::Arc;
use std::sync::atomic::Ordering;
use axum::Extension;
use axum::extract::Path;
use axum::http::{Response, StatusCode};
use axum::response::Json;
use hyper::Body;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{RwLock as ARwLock, broadcast};
use chrono::Utc;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::tasks::types::{TaskMeta, TaskBoard, BoardCard, StatusUpdate, TaskStatus, TrajectoryInfo};
use crate::tasks::events::{TaskEvent, TaskEventEnvelope};
use crate::tasks::storage;

#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub name: String,
    #[serde(default)]
    pub target_files: Vec<String>,
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
        #[serde(default)]
        target_files: Vec<String>,
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
        #[serde(default)]
        target_files: Option<Vec<String>>,
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

async fn enrich_task_with_session_state(gcx: Arc<ARwLock<GlobalContext>>, task: &mut TaskMeta) {
    if let Ok(planner_chat_id) = storage::get_planner_chat_id(gcx.clone(), &task.id).await {
        let gcx_locked = gcx.read().await;
        let sessions = gcx_locked.chat_sessions.read().await;
        if let Some(session_arc) = sessions.get(&planner_chat_id) {
            let session = session_arc.lock().await;
            task.planner_session_state = Some(session.runtime.state.to_string());
        }
    }
}

pub async fn list_tasks_with_session_state(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<Vec<TaskMeta>, String> {
    let mut tasks = storage::list_tasks(gcx.clone()).await?;
    for task in &mut tasks {
        enrich_task_with_session_state(gcx.clone(), task).await;
    }
    Ok(tasks)
}

pub async fn handle_list_tasks(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Json<Vec<TaskMeta>>, (StatusCode, String)> {
    let tasks = list_tasks_with_session_state(gcx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(tasks))
}

pub async fn handle_create_task(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Json(req): Json<CreateTaskRequest>,
) -> Result<Json<TaskMeta>, (StatusCode, String)> {
    let meta = storage::create_task(gcx.clone(), &req.name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !req.target_files.is_empty() {
        let now = Utc::now().to_rfc3339();
        let mut board = storage::load_board(gcx.clone(), &meta.id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        board.cards.push(BoardCard {
            id: "targets".to_string(),
            title: "Target files".to_string(),
            column: "planned".into(),
            priority: "P1".into(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: None,
            agent_chat_id: None,
            status_updates: vec![],
            final_report: None,
            created_at: now,
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            target_files: req.target_files,
        });
        storage::save_board(gcx, &meta.id, &board)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    }
    Ok(Json(meta))
}

pub async fn handle_get_task(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let meta = storage::load_task_meta(gcx.clone(), &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    let board = storage::load_board(gcx.clone(), &task_id)
        .await
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
    storage::delete_task(gcx, &task_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({"deleted": true})))
}

pub async fn handle_get_board(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
) -> Result<Json<TaskBoard>, (StatusCode, String)> {
    let board = storage::load_board(gcx, &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(board))
}

pub async fn handle_patch_board(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateBoardRequest>,
) -> Result<Json<TaskBoard>, (StatusCode, String)> {
    let mut board = storage::load_board(gcx.clone(), &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    if board.rev != req.rev {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "Board rev mismatch: expected {}, got {}",
                board.rev, req.rev
            ),
        ));
    }

    let now = Utc::now().to_rfc3339();

    match req.patch {
        BoardPatch::CreateCard {
            id,
            title,
            priority,
            depends_on,
            instructions,
            target_files,
        } => {
            if board.cards.iter().any(|c| c.id == id) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("Card {} already exists", id),
                ));
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
                last_heartbeat_at: None,
                completed_at: None,
                agent_branch: None,
                agent_worktree: None,
                agent_worktree_name: None,
                target_files,
            });
        }
        BoardPatch::UpdateCard {
            id,
            title,
            priority,
            depends_on,
            instructions,
            target_files,
        } => {
            let card = board
                .get_card_mut(&id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Card {} not found", id)))?;
            if let Some(t) = title {
                card.title = t;
            }
            if let Some(p) = priority {
                card.priority = p;
            }
            if let Some(d) = depends_on {
                card.depends_on = d;
            }
            if let Some(i) = instructions {
                card.instructions = i;
            }
            if let Some(files) = target_files {
                card.target_files = files;
            }
        }
        BoardPatch::MoveCard { id, column } => {
            let card = board
                .get_card_mut(&id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Card {} not found", id)))?;
            let valid_columns = ["planned", "doing", "done", "failed"];
            if !valid_columns.contains(&column.as_str()) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("Invalid column: {}", column),
                ));
            }
            if column == "doing" && card.started_at.is_none() {
                card.started_at = Some(now.clone());
            }
            if (column == "done" || column == "failed") && card.completed_at.is_none() {
                card.completed_at = Some(now.clone());
            }
            card.column = column;
        }
        BoardPatch::AssignAgent {
            card_id,
            agent_id,
            agent_chat_id,
        } => {
            let card = board
                .get_card_mut(&card_id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Card {} not found", card_id)))?;
            card.assignee = Some(agent_id);
            card.agent_chat_id = Some(agent_chat_id);
            if card.started_at.is_none() {
                card.started_at = Some(now.clone());
            }
        }
        BoardPatch::AddStatusUpdate { card_id, message } => {
            let card = board
                .get_card_mut(&card_id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Card {} not found", card_id)))?;
            card.status_updates.push(StatusUpdate {
                timestamp: now.clone(),
                message,
            });
        }
        BoardPatch::SetFinalReport { card_id, report } => {
            let card = board
                .get_card_mut(&card_id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Card {} not found", card_id)))?;
            card.final_report = Some(report);
        }
        BoardPatch::DeleteCard { id } => {
            board.cards.retain(|c| c.id != id);
        }
    }

    board.rev += 1;
    storage::save_board(gcx.clone(), &task_id, &board)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    crate::tasks::events::emit_task_event(
        gcx.clone(),
        TaskEvent::BoardChanged {
            task_id: task_id.clone(),
            rev: board.rev,
            board: board.clone(),
        },
    )
    .await;

    storage::update_task_stats(gcx, &task_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(board))
}

pub async fn handle_get_planner_instructions(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let content = storage::load_planner_instructions(gcx, &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(json!({"content": content})))
}

#[derive(Deserialize)]
pub struct SetPlannerInstructionsRequest {
    pub content: String,
}

pub async fn handle_set_planner_instructions(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
    Json(req): Json<SetPlannerInstructionsRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    storage::save_planner_instructions(gcx, &task_id, &req.content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({"saved": true})))
}

pub async fn handle_get_ready_cards(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let board = storage::load_board(gcx, &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    let ready = board.get_ready_cards();
    Ok(Json(json!(ready)))
}

pub async fn handle_update_task_status(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateTaskStatusRequest>,
) -> Result<Json<TaskMeta>, (StatusCode, String)> {
    let mut meta = storage::load_task_meta(gcx.clone(), &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    let old_status = meta.status;
    meta.status = req.status;
    meta.updated_at = Utc::now().to_rfc3339();
    storage::save_task_meta(gcx.clone(), &task_id, &meta)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    crate::tasks::events::emit_task_event(
        gcx.clone(),
        TaskEvent::TaskUpdated {
            task_id: task_id.clone(),
            meta: meta.clone(),
        },
    )
    .await;
    if old_status != meta.status {
        match meta.status {
            TaskStatus::Completed => {
                crate::buddy::actor::buddy_apply(
                    gcx.clone(),
                    crate::buddy::actor::BuddyMutation {
                        runtime_event: Some(crate::buddy::actor::make_runtime_event(
                            "task_completed",
                            &format!("Task done: {}", meta.name),
                            "task",
                            &format!("task_{}", task_id),
                            "completed",
                            None,
                        )),
                        xp: 30,
                        activity: Some(crate::buddy::types::BuddyActivity {
                            icon: "✅".to_string(),
                            title: format!("Task completed: {}", meta.name),
                            description: format!("Task {} finished successfully", task_id),
                            timestamp: Utc::now().to_rfc3339(),
                            activity_type: "task_completed".to_string(),
                            chat_id: None,
                        }),
                        mood: Some("excited".to_string()),
                    },
                )
                .await;
            }
            TaskStatus::Abandoned => {
                crate::buddy::actor::buddy_apply(
                    gcx.clone(),
                    crate::buddy::actor::BuddyMutation {
                        runtime_event: Some(crate::buddy::actor::make_runtime_event(
                            "task_abandoned",
                            &format!("Task abandoned: {}", meta.name),
                            "task",
                            &format!("task_{}", task_id),
                            "failed",
                            Some("high"),
                        )),
                        activity: Some(crate::buddy::types::BuddyActivity {
                            icon: "🗑️".to_string(),
                            title: format!("Task abandoned: {}", meta.name),
                            description: format!("Task {} was abandoned", task_id),
                            timestamp: Utc::now().to_rfc3339(),
                            activity_type: "task_abandoned".to_string(),
                            chat_id: None,
                        }),
                        mood: Some("worried".to_string()),
                        ..Default::default()
                    },
                )
                .await;
            }
            _ => {}
        }
    }
    Ok(Json(meta))
}

#[derive(Deserialize)]
pub struct UpdateTaskStatusRequest {
    pub status: TaskStatus,
}

#[derive(Deserialize)]
pub struct UpdateTaskMetaRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub base_commit: Option<String>,
    #[serde(default)]
    pub default_agent_model: Option<String>,
}

pub async fn handle_update_task_meta(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateTaskMetaRequest>,
) -> Result<Json<TaskMeta>, (StatusCode, String)> {
    let mut meta = storage::load_task_meta(gcx.clone(), &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    if let Some(name) = req.name {
        meta.name = name;
    }
    if let Some(branch) = req.base_branch {
        meta.base_branch = Some(branch);
    }
    if let Some(commit) = req.base_commit {
        meta.base_commit = Some(commit);
    }
    if let Some(model) = req.default_agent_model {
        meta.default_agent_model = Some(model);
    }
    meta.updated_at = Utc::now().to_rfc3339();
    storage::save_task_meta(gcx.clone(), &task_id, &meta)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    crate::tasks::events::emit_task_event(
        gcx,
        TaskEvent::TaskUpdated {
            task_id,
            meta: meta.clone(),
        },
    )
    .await;
    Ok(Json(meta))
}

pub async fn handle_list_task_trajectories(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path((task_id, role)): Path<(String, String)>,
) -> Result<Json<Vec<TrajectoryInfo>>, (StatusCode, String)> {
    let trajectories = storage::list_task_trajectories(gcx, &task_id, &role, None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(trajectories))
}

pub async fn handle_create_planner_chat(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let _ = storage::load_task_meta(gcx.clone(), &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    let chat_id = storage::next_planner_chat_id(gcx.clone(), &task_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    crate::chat::trajectories::save_initial_planner_trajectory(gcx, &task_id, &chat_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(json!({"chat_id": chat_id})))
}

pub async fn handle_tasks_subscribe(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let (rx, seq_counter, tasks) = {
        let gcx_locked = gcx.read().await;
        let rx = match &gcx_locked.task_events_tx {
            Some(tx) => tx.subscribe(),
            None => {
                return Err(ScratchError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Task events not available".to_string(),
                ))
            }
        };
        let seq_counter = gcx_locked.task_events_seq.clone().ok_or_else(|| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Task events seq not available".to_string(),
            )
        })?;
        drop(gcx_locked);
        let tasks = list_tasks_with_session_state(gcx.clone())
            .await
            .unwrap_or_default();
        (rx, seq_counter, tasks)
    };

    let stream = async_stream::stream! {
        let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
        let envelope = TaskEventEnvelope { seq, event: TaskEvent::Snapshot { tasks } };
        let json = serde_json::to_string(&envelope).unwrap_or_default();
        yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));

        let mut rx = rx;
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(10));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(envelope) => {
                            let json = serde_json::to_string(&envelope).unwrap_or_default();
                            yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let tasks = list_tasks_with_session_state(gcx.clone()).await.unwrap_or_default();
                            let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
                            let envelope = TaskEventEnvelope { seq, event: TaskEvent::Snapshot { tasks } };
                            let json = serde_json::to_string(&envelope).unwrap_or_default();
                            yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = heartbeat.tick() => {
                    let tasks = list_tasks_with_session_state(gcx.clone()).await.unwrap_or_default();
                    let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
                    let envelope = TaskEventEnvelope { seq, event: TaskEvent::Snapshot { tasks } };
                    let json = serde_json::to_string(&envelope).unwrap_or_default();
                    yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
                }

                _ = async {
                    let shutdown_flag = gcx.read().await.shutdown_flag.clone();
                    while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                } => {
                    break;
                }
            }
        }
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::wrap_stream(stream))
        .unwrap())
}
