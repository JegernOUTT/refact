use std::sync::Arc;
use std::sync::atomic::Ordering;
use axum::extract::Path;
use axum::extract::Query;
use axum::http::{Response, StatusCode};
use axum::response::Json;
use axum::extract::State;
use hyper::Body;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use chrono::Utc;

use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use crate::custom_error::ScratchError;
use crate::tasks::types::{TaskMeta, TaskBoard, BoardCard, StatusUpdate, TaskStatus, TrajectoryInfo};
use crate::chat::trajectories::TrajectoryEvent;
use crate::chat::types::SessionState;
use crate::tasks::events::{TaskEvent, TaskEventEnvelope};
use crate::tasks::storage;
use crate::tools::tool_task_memory::{
    MemoryKind, MemoryNamespace, TaskMemoriesApiResponse, TaskMemoryArchiveApiResponse,
    TaskMemoryListFilters, TaskMemoryPinApiResponse, TaskMemoryTriageApiResponse,
    archive_task_memory_for_api, list_task_memories_for_api, mark_task_memories_triaged_for_api,
    set_task_memory_pinned_for_api,
};

#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub name: String,
    #[serde(default)]
    pub target_files: Vec<String>,
}

#[derive(Deserialize)]
pub struct TaskMemoriesQuery {
    pub since: Option<String>,
    pub kind: Option<String>,
    pub namespace: Option<String>,
    pub search: Option<String>,
}

#[derive(Deserialize)]
pub struct PinTaskMemoryRequest {
    pub pinned: bool,
}

#[derive(Deserialize)]
pub struct TriageDoneRequest {
    pub cursor: Option<String>,
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

async fn enrich_task_with_session_state(gcx: Arc<GlobalContext>, task: &mut TaskMeta) {
    let planner_chat_ids = storage::list_task_trajectories(gcx.clone(), &task.id, "planner", None)
        .await
        .map(|trajectories| {
            trajectories
                .into_iter()
                .map(|trajectory| trajectory.id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if planner_chat_ids.is_empty() {
        task.planner_session_state = None;
        return;
    }

    let session_arcs = {
        let sessions = gcx.chat_sessions.read().await;
        planner_chat_ids
            .iter()
            .filter_map(|planner_chat_id| sessions.get(planner_chat_id).cloned())
            .collect::<Vec<_>>()
    };

    let mut has_paused = false;
    let mut has_waiting_ide = false;
    let mut has_waiting_user_input = false;
    let mut has_generating = false;
    let mut has_executing_tools = false;
    let mut has_error = false;
    for session_arc in session_arcs {
        let session = session_arc.lock().await;
        match session.runtime.state {
            SessionState::Paused => has_paused = true,
            SessionState::WaitingIde => has_waiting_ide = true,
            SessionState::WaitingUserInput => has_waiting_user_input = true,
            SessionState::Generating => has_generating = true,
            SessionState::ExecutingTools => has_executing_tools = true,
            SessionState::Error => has_error = true,
            SessionState::Idle | SessionState::Completed => {}
        }
    }

    task.planner_session_state = if has_paused {
        Some(SessionState::Paused.to_string())
    } else if has_waiting_ide {
        Some(SessionState::WaitingIde.to_string())
    } else if has_waiting_user_input {
        Some(SessionState::WaitingUserInput.to_string())
    } else if has_generating {
        Some(SessionState::Generating.to_string())
    } else if has_executing_tools {
        Some(SessionState::ExecutingTools.to_string())
    } else if has_error {
        Some(SessionState::Error.to_string())
    } else {
        None
    };
}

pub async fn list_tasks_with_session_state(
    gcx: Arc<GlobalContext>,
) -> Result<Vec<TaskMeta>, String> {
    let mut tasks = storage::list_tasks(gcx.clone()).await?;
    for task in &mut tasks {
        enrich_task_with_session_state(gcx.clone(), task).await;
    }
    Ok(tasks)
}

pub async fn handle_list_tasks(
    State(app): State<AppState>,
) -> Result<Json<Vec<TaskMeta>>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
    let tasks = list_tasks_with_session_state(gcx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(tasks))
}

pub async fn handle_create_task(
    State(app): State<AppState>,
    Json(req): Json<CreateTaskRequest>,
) -> Result<Json<TaskMeta>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
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
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: now,
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            ab_variants: None,
            target_files: req.target_files,
            scope_guard_mode: Default::default(),
            team_members: vec![],
        });
        storage::save_board(gcx, &meta.id, &board)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    }
    Ok(Json(meta))
}

pub async fn handle_get_task(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
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
    State(app): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
    storage::delete_task(gcx, &task_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({"deleted": true})))
}

pub async fn handle_list_task_memories(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
    Query(query): Query<TaskMemoriesQuery>,
) -> Result<Json<TaskMemoriesApiResponse>, (StatusCode, String)> {
    let since = query
        .since
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(crate::tools::tool_task_memory::parse_rfc3339_utc)
        .transpose()
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let kind = query
        .kind
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::parse::<MemoryKind>)
        .transpose()
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let namespace = query
        .namespace
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::parse::<MemoryNamespace>)
        .transpose()
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let search = query.search.filter(|value| !value.trim().is_empty());
    let result = list_task_memories_for_api(
        app.gcx.clone(),
        &task_id,
        TaskMemoryListFilters {
            since,
            kind,
            namespace,
            search,
        },
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(result))
}

pub async fn handle_pin_task_memory(
    State(app): State<AppState>,
    Path((task_id, filename)): Path<(String, String)>,
    Json(req): Json<PinTaskMemoryRequest>,
) -> Result<Json<TaskMemoryPinApiResponse>, (StatusCode, String)> {
    let result = set_task_memory_pinned_for_api(app.gcx.clone(), &task_id, &filename, req.pinned)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(result))
}

pub async fn handle_archive_task_memory(
    State(app): State<AppState>,
    Path((task_id, filename)): Path<(String, String)>,
) -> Result<Json<TaskMemoryArchiveApiResponse>, (StatusCode, String)> {
    let result = archive_task_memory_for_api(app.gcx.clone(), &task_id, &filename)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(result))
}

pub async fn handle_task_memories_triage_done(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
    Json(req): Json<TriageDoneRequest>,
) -> Result<Json<TaskMemoryTriageApiResponse>, (StatusCode, String)> {
    let cursor = req
        .cursor
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(crate::tools::tool_task_memory::parse_rfc3339_utc)
        .transpose()
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let result = mark_task_memories_triaged_for_api(app.gcx.clone(), &task_id, cursor)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(result))
}

pub async fn handle_get_board(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<TaskBoard>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
    let board = storage::load_board(gcx, &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(board))
}

fn map_patch_board_error(error: String) -> (StatusCode, String) {
    if error.starts_with("Board rev mismatch:") {
        return (StatusCode::CONFLICT, error);
    }
    if error.contains(" not found") {
        return (StatusCode::NOT_FOUND, error);
    }
    if error.starts_with("Card ") || error.starts_with("Invalid column:") {
        return (StatusCode::BAD_REQUEST, error);
    }
    (StatusCode::INTERNAL_SERVER_ERROR, error)
}

pub async fn handle_patch_board(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateBoardRequest>,
) -> Result<Json<TaskBoard>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
    let expected_rev = req.rev;
    let patch = req.patch;
    let now = Utc::now().to_rfc3339();

    let update_result = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
        if board.rev != expected_rev {
            let conflict = storage::BoardConflict {
                expected: expected_rev,
                actual: board.rev,
            };
            return Err(conflict.message());
        }

        match patch {
            BoardPatch::CreateCard {
                id,
                title,
                priority,
                depends_on,
                instructions,
                target_files,
            } => {
                if board.cards.iter().any(|c| c.id == id) {
                    return Err(format!("Card {} already exists", id));
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
                    comments: vec![],
                    final_report: None,
                    final_report_structured: None,
                    verifier_report: None,
                    created_at: now.clone(),
                    started_at: None,
                    last_heartbeat_at: None,
                    completed_at: None,
                    agent_branch: None,
                    agent_worktree: None,
                    agent_worktree_name: None,
                    ab_variants: None,
                    target_files,
                    scope_guard_mode: Default::default(),
                    team_members: vec![],
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
                    .ok_or_else(|| format!("Card {} not found", id))?;
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
                    .ok_or_else(|| format!("Card {} not found", id))?;
                let valid_columns = ["planned", "doing", "done", "failed", "regressed"];
                if !valid_columns.contains(&column.as_str()) {
                    return Err(format!("Invalid column: {}", column));
                }
                if column == "doing" && card.started_at.is_none() {
                    card.started_at = Some(now.clone());
                }
                if (column == "done" || column == "failed" || column == "regressed")
                    && card.completed_at.is_none()
                {
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
                    .ok_or_else(|| format!("Card {} not found", card_id))?;
                card.assignee = Some(agent_id);
                card.agent_chat_id = Some(agent_chat_id);
                if card.started_at.is_none() {
                    card.started_at = Some(now.clone());
                }
            }
            BoardPatch::AddStatusUpdate { card_id, message } => {
                let card = board
                    .get_card_mut(&card_id)
                    .ok_or_else(|| format!("Card {} not found", card_id))?;
                card.status_updates.push(StatusUpdate {
                    timestamp: now.clone(),
                    message,
                });
            }
            BoardPatch::SetFinalReport { card_id, report } => {
                let card = board
                    .get_card_mut(&card_id)
                    .ok_or_else(|| format!("Card {} not found", card_id))?;
                card.final_report = Some(report);
                card.final_report_structured = None;
            }
            BoardPatch::DeleteCard { id } => {
                board.cards.retain(|c| c.id != id);
            }
        }

        Ok(())
    })
    .await;

    let (board, _) = match update_result {
        Ok(result) => result,
        Err(e) => return Err(map_patch_board_error(e)),
    };

    storage::update_task_stats(gcx, &task_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(board))
}

pub async fn handle_get_planner_instructions(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
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
    State(app): State<AppState>,
    Path(task_id): Path<String>,
    Json(req): Json<SetPlannerInstructionsRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
    storage::save_planner_instructions(gcx, &task_id, &req.content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({"saved": true})))
}

pub async fn handle_get_ready_cards(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
    let board = storage::load_board(gcx, &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    let ready = board.get_ready_cards();
    Ok(Json(json!(ready)))
}

pub async fn handle_update_task_status(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateTaskStatusRequest>,
) -> Result<Json<TaskMeta>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
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
                    crate::app_state::AppState::from_gcx(gcx.clone()).await,
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
                    crate::app_state::AppState::from_gcx(gcx.clone()).await,
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
    State(app): State<AppState>,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateTaskMetaRequest>,
) -> Result<Json<TaskMeta>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
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
    State(app): State<AppState>,
    Path((task_id, role)): Path<(String, String)>,
) -> Result<Json<Vec<TrajectoryInfo>>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
    let trajectories = storage::list_task_trajectories(gcx, &task_id, &role, None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(trajectories))
}

pub async fn handle_create_planner_chat(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
    storage::load_task_meta(gcx.clone(), &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    let chat_id = storage::next_planner_chat_id(gcx.clone(), &task_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    crate::chat::trajectories::save_initial_planner_trajectory(gcx.clone(), &task_id, &chat_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(json!({"chat_id": chat_id})))
}

async fn planner_agent_refs(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    planner_chat_id: &str,
) -> Result<Vec<String>, (StatusCode, String)> {
    let board = storage::load_board(gcx.clone(), task_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let sessions = {
        gcx.chat_sessions.clone()
    };
    let mut refs = Vec::new();
    for agent_chat_id in board
        .cards
        .iter()
        .filter_map(|card| card.agent_chat_id.as_deref())
    {
        let session_arc = {
            let sessions_guard = sessions.read().await;
            sessions_guard.get(agent_chat_id).cloned()
        };
        if let Some(session_arc) = session_arc {
            let session = session_arc.lock().await;
            if session
                .thread
                .task_meta
                .as_ref()
                .and_then(|meta| meta.planner_chat_id.as_deref())
                == Some(planner_chat_id)
            {
                refs.push(agent_chat_id.to_string());
            }
        }
    }
    Ok(refs)
}

pub async fn handle_delete_planner_chat(
    State(app): State<AppState>,
    Path((task_id, chat_id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let gcx = app.gcx.clone();
    crate::chat::trajectories::validate_trajectory_id(&chat_id)
        .map_err(|e| (e.status_code, e.message))?;
    storage::load_task_meta(gcx.clone(), &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    let task_dir = storage::find_task_dir(gcx.clone(), &task_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    let planner_dir = storage::get_task_trajectory_dir(&task_dir, "planner", None);
    let expected_file_path = planner_dir.join(format!("{}.json", chat_id));
    let file_path = if expected_file_path.exists() {
        expected_file_path
    } else {
        match crate::chat::trajectories::find_trajectory_path(gcx.clone(), &chat_id).await {
            Some(found_path) if found_path.exists() => found_path,
            _ => return Err((StatusCode::NOT_FOUND, "Planner chat not found".to_string())),
        }
    };
    let canon_dir = tokio::fs::canonicalize(&planner_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let canon_file = tokio::fs::canonicalize(&file_path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    if !canon_file.starts_with(&canon_dir) || canon_file.parent() != Some(canon_dir.as_path()) {
        return Err((
            StatusCode::FORBIDDEN,
            "Planner chat does not belong to this task".to_string(),
        ));
    }

    let agent_refs = planner_agent_refs(gcx.clone(), &task_id, &chat_id).await?;

    tokio::fs::remove_file(&file_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !agent_refs.is_empty() {
        tracing::warn!(
            "Deleted planner chat {} while {} task agent(s) still reference it",
            chat_id,
            agent_refs.len()
        );
    }

    let removed_session = {
        let sessions = gcx.chat_sessions.clone();
        let mut sessions_guard = sessions.write().await;
        sessions_guard.remove(&chat_id)
    };
    if let Some(session_arc) = removed_session {
        match tokio::time::timeout(std::time::Duration::from_millis(500), session_arc.lock()).await
        {
            Ok(mut session) => {
                session.abort_stream();
                session.close_event_channel();
                session.queue_notify.notify_waiters();
            }
            Err(_) => {
                tracing::warn!("Timed out closing deleted planner chat session {}", chat_id);
            }
        }
    }

    if let Some(tx) = &gcx.trajectory_events_tx {
        let _ = tx.send(TrajectoryEvent {
            event_type: "deleted".to_string(),
            id: chat_id,
            updated_at: None,
            title: None,
            is_title_generated: None,
            session_state: None,
            error: None,
            message_count: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            model: None,
            mode: None,
            worktree: None,
            total_lines_added: None,
            total_lines_removed: None,
            tasks_total: None,
            tasks_done: None,
            tasks_failed: None,
            total_prompt_tokens: None,
            total_completion_tokens: None,
            total_tokens: None,
            total_cache_read_tokens: None,
            total_cache_creation_tokens: None,
            total_cost_usd: None,
        });
    }

    Ok(Json(json!({"deleted": true})))
}

pub async fn handle_tasks_subscribe(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let (rx, seq_counter, tasks) = {
        let rx = match &gcx.task_events_tx {
            Some(tx) => tx.subscribe(),
            None => {
                return Err(ScratchError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Task events not available".to_string(),
                ))
            }
        };
        let seq_counter = gcx.task_events_seq.clone().ok_or_else(|| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Task events seq not available".to_string(),
            )
        })?;
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
                    let shutdown_flag = gcx.shutdown_flag.clone();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::trajectories::save_trajectory_snapshot;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use refact_chat_api::{ChatMessage, TaskMeta as ChatTaskMeta};
    use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;

    async fn setup_task(root: &std::path::Path, task_id: &str) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        let task_dir = root.join(".refact/tasks").join(task_id);
        tokio::fs::create_dir_all(task_dir.join("trajectories/planner"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(task_dir.join("trajectories/agents/agent-1"))
            .await
            .unwrap();
        let now = Utc::now().to_rfc3339();
        let meta = TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: task_id.to_string(),
            status: TaskStatus::Planning,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 0,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 0,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        storage::save_task_meta(gcx.clone(), task_id, &meta).await.unwrap();
        storage::save_board(gcx.clone(), task_id, &TaskBoard::default())
            .await
            .unwrap();
        gcx
    }

    fn app(gcx: Arc<GlobalContext>) -> AppState {
        gcx.app_state(gcx.clone())
    }

    fn snapshot(
        chat_id: &str,
        task_id: &str,
        role: &str,
        agent_id: Option<&str>,
    ) -> TrajectorySnapshot {
        TrajectorySnapshot {
            chat_id: chat_id.to_string(),
            title: chat_id.to_string(),
            model: "test-model".to_string(),
            mode: "task_planner".to_string(),
            tool_use: "agent".to_string(),
            messages: vec![ChatMessage::new("user".to_string(), "hello".to_string())],
            created_at: Utc::now().to_rfc3339(),
            boost_reasoning: false,
            checkpoints_enabled: false,
            context_tokens_cap: None,
            include_project_info: false,
            is_title_generated: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            autonomous_no_confirm: false,
            version: 1,
            task_meta: Some(ChatTaskMeta {
                task_id: task_id.to_string(),
                role: role.to_string(),
                agent_id: agent_id.map(str::to_string),
                card_id: None,
                planner_chat_id: (role == "planner").then(|| chat_id.to_string()),
            }),
            worktree: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
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
        }
    }

    async fn save_snapshot(gcx: Arc<GlobalContext>, snapshot: TrajectorySnapshot) {
        save_trajectory_snapshot(gcx, snapshot).await.unwrap();
    }

    fn status<T>(result: Result<Json<T>, (StatusCode, String)>) -> StatusCode {
        match result {
            Ok(_) => panic!("expected request to fail"),
            Err((status, _)) => status,
        }
    }

    fn create_card_request(rev: u64, id: &str) -> UpdateBoardRequest {
        UpdateBoardRequest {
            rev,
            patch: BoardPatch::CreateCard {
                id: id.to_string(),
                title: format!("Card {}", id),
                priority: None,
                depends_on: Vec::new(),
                instructions: String::new(),
                target_files: Vec::new(),
            },
        }
    }

    #[tokio::test]
    async fn handle_patch_board_serial_patches_increment_rev() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_task(temp.path(), "task-serial").await;

        let first = handle_patch_board(
            State(app(gcx.clone())),
            Path("task-serial".to_string()),
            Json(create_card_request(0, "card-a")),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(first.rev, 1);
        assert!(first.get_card("card-a").is_some());

        let second = handle_patch_board(
            State(app(gcx.clone())),
            Path("task-serial".to_string()),
            Json(create_card_request(1, "card-b")),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(second.rev, 2);
        assert!(second.get_card("card-a").is_some());
        assert!(second.get_card("card-b").is_some());
    }

    #[tokio::test]
    async fn handle_patch_board_concurrent_patches_one_succeeds_one_409() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_task(temp.path(), "task-concurrent").await;
        let mut board = storage::load_board(gcx.clone(), "task-concurrent").await.unwrap();
        board.rev = 5;
        storage::save_board(gcx.clone(), "task-concurrent", &board)
            .await
            .unwrap();

        static GATE_READY: AtomicUsize = AtomicUsize::new(0);
        GATE_READY.store(0, AtomicOrdering::SeqCst);
        async fn wait_for_both_ready() {
            GATE_READY.fetch_add(1, AtomicOrdering::SeqCst);
            while GATE_READY.load(AtomicOrdering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        }

        let first = {
            let gcx = gcx.clone();
            tokio::spawn(async move {
                wait_for_both_ready().await;
                handle_patch_board(
                    State(app(gcx)),
                    Path("task-concurrent".to_string()),
                    Json(create_card_request(5, "card-a")),
                )
                .await
            })
        };
        let second = {
            let gcx = gcx.clone();
            tokio::spawn(async move {
                wait_for_both_ready().await;
                handle_patch_board(
                    State(app(gcx)),
                    Path("task-concurrent".to_string()),
                    Json(create_card_request(5, "card-b")),
                )
                .await
            })
        };

        let results = vec![first.await.unwrap(), second.await.unwrap()];
        let successes = results.iter().filter(|result| result.is_ok()).count();
        let conflicts = results
            .iter()
            .filter(|result| {
                matches!(
                    result,
                    Err((StatusCode::CONFLICT, message)) if message.contains("actual 6")
                )
            })
            .count();

        assert_eq!(successes, 1);
        assert_eq!(conflicts, 1);

        let successful_board = results
            .into_iter()
            .find_map(Result::ok)
            .map(|json| json.0)
            .unwrap();
        assert_eq!(successful_board.rev, 6);
        let has_card_a = successful_board.get_card("card-a").is_some();
        let has_card_b = successful_board.get_card("card-b").is_some();
        assert_ne!(has_card_a, has_card_b);

        let stored = storage::load_board(gcx, "task-concurrent").await.unwrap();
        assert_eq!(stored.rev, 6);
        assert_eq!(stored.cards.len(), 1);
        assert_eq!(stored.get_card("card-a").is_some(), has_card_a);
        assert_eq!(stored.get_card("card-b").is_some(), has_card_b);
    }

    #[tokio::test]
    async fn delete_planner_chat_rejects_agent_trajectory() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_task(temp.path(), "task-1").await;
        save_snapshot(
            gcx.clone(),
            snapshot("shared-chat", "task-1", "agents", Some("agent-1")),
        )
        .await;

        let result = handle_delete_planner_chat(
            State(app(gcx)),
            Path(("task-1".to_string(), "shared-chat".to_string())),
        )
        .await;

        assert_eq!(status(result), StatusCode::FORBIDDEN);
        assert!(temp
            .path()
            .join(".refact/tasks/task-1/trajectories/agents/agent-1/shared-chat.json")
            .exists());
    }

    #[tokio::test]
    async fn delete_planner_chat_rejects_other_task() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_task(temp.path(), "task-1").await;
        storage::create_task(gcx.clone(), "other task").await.unwrap();
        let task_2_path = temp.path().join(".refact/tasks/task-2");
        let created_task_path = std::fs::read_dir(temp.path().join(".refact/tasks"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name().and_then(|name| name.to_str()) != Some("task-1")
                    && path.join("meta.yaml").exists()
            })
            .unwrap();
        std::fs::rename(&created_task_path, &task_2_path).unwrap();
        let meta_path = task_2_path.join("meta.yaml");
        let mut meta: TaskMeta =
            serde_yaml::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
        meta.id = "task-2".to_string();
        std::fs::write(&meta_path, serde_yaml::to_string(&meta).unwrap()).unwrap();
        save_snapshot(gcx.clone(), snapshot("shared-chat", "task-2", "planner", None)).await;

        let result = handle_delete_planner_chat(
            State(app(gcx)),
            Path(("task-1".to_string(), "shared-chat".to_string())),
        )
        .await;

        assert_eq!(status(result), StatusCode::FORBIDDEN);
        assert!(temp
            .path()
            .join(".refact/tasks/task-2/trajectories/planner/shared-chat.json")
            .exists());
    }

    #[tokio::test]
    async fn delete_planner_chat_rejects_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_task(temp.path(), "task-1").await;

        let result = handle_delete_planner_chat(
            State(app(gcx)),
            Path(("task-1".to_string(), "../../etc/passwd".to_string())),
        )
        .await;

        assert_eq!(status(result), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_planner_chat_rejects_invalid_chat_id() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_task(temp.path(), "task-1").await;

        let result = handle_delete_planner_chat(
            State(app(gcx)),
            Path(("task-1".to_string(), "bad.chat".to_string())),
        )
        .await;

        assert_eq!(status(result), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_planner_chat_happy_path() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_task(temp.path(), "task-1").await;
        save_snapshot(gcx.clone(), snapshot("planner-chat", "task-1", "planner", None)).await;
        let planner_path = temp
            .path()
            .join(".refact/tasks/task-1/trajectories/planner/planner-chat.json");
        assert!(planner_path.exists());

        let result = handle_delete_planner_chat(
            State(app(gcx)),
            Path(("task-1".to_string(), "planner-chat".to_string())),
        )
        .await;

        assert!(result.is_ok());
        assert!(!planner_path.exists());
    }
}
