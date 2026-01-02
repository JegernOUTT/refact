use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{RwLock as ARwLock, Mutex as AMutex};
use tokio::fs;
use tracing::warn;
use uuid::Uuid;
use chrono::Utc;

use crate::global_context::GlobalContext;
use crate::files_correction::get_project_dirs;
use super::types::{TaskMeta, TaskBoard, TaskStatus};

const TASKS_DIR: &str = "tasks";

static BOARD_LOCKS: std::sync::OnceLock<AMutex<HashMap<String, Arc<AMutex<()>>>>> =
    std::sync::OnceLock::new();

fn get_board_locks() -> &'static AMutex<HashMap<String, Arc<AMutex<()>>>> {
    BOARD_LOCKS.get_or_init(|| AMutex::new(HashMap::new()))
}

async fn get_board_lock(task_id: &str) -> Arc<AMutex<()>> {
    let mut locks = get_board_locks().lock().await;
    locks.entry(task_id.to_string())
        .or_insert_with(|| Arc::new(AMutex::new(())))
        .clone()
}

pub async fn get_tasks_dir(gcx: Arc<ARwLock<GlobalContext>>) -> Result<PathBuf, String> {
    let project_dirs = get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    Ok(workspace_root.join(".refact").join(TASKS_DIR))
}

pub async fn ensure_tasks_dir(gcx: Arc<ARwLock<GlobalContext>>) -> Result<PathBuf, String> {
    let dir = get_tasks_dir(gcx).await?;
    if !dir.exists() {
        fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

pub fn validate_task_id(task_id: &str) -> Result<(), String> {
    if task_id.is_empty() {
        return Err("Task ID cannot be empty".into());
    }
    if task_id.contains('/') || task_id.contains('\\') || task_id.contains("..") {
        return Err("Task ID contains invalid characters".into());
    }
    if task_id.len() > 100 {
        return Err("Task ID too long".into());
    }
    Ok(())
}

pub async fn get_task_dir(gcx: Arc<ARwLock<GlobalContext>>, task_id: &str) -> Result<PathBuf, String> {
    validate_task_id(task_id)?;
    let tasks_dir = get_tasks_dir(gcx).await?;
    Ok(tasks_dir.join(task_id))
}

pub async fn list_tasks(gcx: Arc<ARwLock<GlobalContext>>) -> Result<Vec<TaskMeta>, String> {
    let tasks_dir = get_tasks_dir(gcx.clone()).await?;
    if !tasks_dir.exists() {
        return Ok(vec![]);
    }

    let mut tasks = vec![];
    let mut entries = fs::read_dir(&tasks_dir).await.map_err(|e| e.to_string())?;

    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        if path.is_dir() {
            let meta_path = path.join("meta.yaml");
            if meta_path.exists() {
                match load_task_meta_from_path(&meta_path).await {
                    Ok(meta) => tasks.push(meta),
                    Err(e) => warn!("Failed to load task meta from {:?}: {}", meta_path, e),
                }
            }
        }
    }

    tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(tasks)
}

async fn load_task_meta_from_path(path: &PathBuf) -> Result<TaskMeta, String> {
    let content = fs::read_to_string(path).await.map_err(|e| e.to_string())?;
    serde_yaml::from_str(&content).map_err(|e| e.to_string())
}

pub async fn load_task_meta(gcx: Arc<ARwLock<GlobalContext>>, task_id: &str) -> Result<TaskMeta, String> {
    let task_dir = get_task_dir(gcx, task_id).await?;
    let meta_path = task_dir.join("meta.yaml");
    load_task_meta_from_path(&meta_path).await
}

pub async fn save_task_meta(gcx: Arc<ARwLock<GlobalContext>>, task_id: &str, meta: &TaskMeta) -> Result<(), String> {
    let task_dir = get_task_dir(gcx, task_id).await?;
    let meta_path = task_dir.join("meta.yaml");
    let content = serde_yaml::to_string(meta).map_err(|e| e.to_string())?;
    fs::write(&meta_path, content).await.map_err(|e| e.to_string())
}

pub async fn load_board(gcx: Arc<ARwLock<GlobalContext>>, task_id: &str) -> Result<TaskBoard, String> {
    let task_dir = get_task_dir(gcx, task_id).await?;
    let board_path = task_dir.join("board.yaml");
    if !board_path.exists() {
        return Ok(TaskBoard::default());
    }
    let content = fs::read_to_string(&board_path).await.map_err(|e| e.to_string())?;
    serde_yaml::from_str(&content).map_err(|e| e.to_string())
}

pub async fn save_board(gcx: Arc<ARwLock<GlobalContext>>, task_id: &str, board: &TaskBoard) -> Result<(), String> {
    let task_dir = get_task_dir(gcx, task_id).await?;
    let board_path = task_dir.join("board.yaml");
    let tmp_path = task_dir.join("board.yaml.tmp");
    let content = serde_yaml::to_string(board).map_err(|e| e.to_string())?;
    fs::write(&tmp_path, &content).await.map_err(|e| e.to_string())?;
    fs::rename(&tmp_path, &board_path).await.map_err(|e| e.to_string())
}

pub async fn update_board_atomic<F>(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    mut updater: F,
) -> Result<TaskBoard, String>
where
    F: FnMut(&mut TaskBoard) -> Result<(), String>,
{
    let lock = get_board_lock(task_id).await;
    let _guard = lock.lock().await;

    let mut board = load_board(gcx.clone(), task_id).await?;
    updater(&mut board)?;
    board.rev += 1;
    save_board(gcx, task_id, &board).await?;
    Ok(board)
}

pub async fn load_orchestrator_instructions(gcx: Arc<ARwLock<GlobalContext>>, task_id: &str) -> Result<String, String> {
    let task_dir = get_task_dir(gcx, task_id).await?;
    let path = task_dir.join("orchestrator_instructions.md");
    if !path.exists() {
        return Ok(String::new());
    }
    fs::read_to_string(&path).await.map_err(|e| e.to_string())
}

pub async fn save_orchestrator_instructions(gcx: Arc<ARwLock<GlobalContext>>, task_id: &str, content: &str) -> Result<(), String> {
    let task_dir = get_task_dir(gcx, task_id).await?;
    let path = task_dir.join("orchestrator_instructions.md");
    fs::write(&path, content).await.map_err(|e| e.to_string())
}

pub async fn create_task(gcx: Arc<ARwLock<GlobalContext>>, name: &str) -> Result<TaskMeta, String> {
    let tasks_dir = ensure_tasks_dir(gcx.clone()).await?;
    let task_id = Uuid::new_v4().to_string();
    let task_dir = tasks_dir.join(&task_id);

    fs::create_dir_all(&task_dir).await.map_err(|e| e.to_string())?;
    fs::create_dir_all(task_dir.join("trajectories").join("planner")).await.map_err(|e| e.to_string())?;
    fs::create_dir_all(task_dir.join("trajectories").join("orchestrator")).await.map_err(|e| e.to_string())?;
    fs::create_dir_all(task_dir.join("trajectories").join("agents")).await.map_err(|e| e.to_string())?;

    let now = Utc::now().to_rfc3339();
    let meta = TaskMeta {
        schema_version: 1,
        id: task_id.clone(),
        name: name.to_string(),
        status: TaskStatus::Planning,
        created_at: now.clone(),
        updated_at: now,
        cards_total: 0,
        cards_done: 0,
        cards_failed: 0,
        agents_active: 0,
    };

    save_task_meta(gcx.clone(), &task_id, &meta).await?;
    save_board(gcx.clone(), &task_id, &TaskBoard::default()).await?;
    save_orchestrator_instructions(gcx, &task_id, "").await?;

    Ok(meta)
}

pub async fn delete_task(gcx: Arc<ARwLock<GlobalContext>>, task_id: &str) -> Result<(), String> {
    let task_dir = get_task_dir(gcx, task_id).await?;
    if task_dir.exists() {
        fs::remove_dir_all(&task_dir).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub async fn update_task_stats(gcx: Arc<ARwLock<GlobalContext>>, task_id: &str) -> Result<TaskMeta, String> {
    let mut meta = load_task_meta(gcx.clone(), task_id).await?;
    let board = load_board(gcx.clone(), task_id).await?;

    meta.cards_total = board.cards.len();
    meta.cards_done = board.cards.iter().filter(|c| c.column == "done").count();
    meta.cards_failed = board.cards.iter().filter(|c| c.column == "failed").count();
    meta.agents_active = board.cards.iter().filter(|c| c.column == "doing" && c.assignee.is_some()).count();
    meta.updated_at = Utc::now().to_rfc3339();

    save_task_meta(gcx, task_id, &meta).await?;
    Ok(meta)
}

pub fn get_task_trajectory_dir(task_dir: &PathBuf, role: &str, agent_id: Option<&str>) -> PathBuf {
    let base = task_dir.join("trajectories").join(role);
    match agent_id {
        Some(id) => base.join(id),
        None => base,
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TaskTrajectoryMeta {
    pub task_id: String,
    pub role: String,
    pub agent_id: Option<String>,
    pub card_id: Option<String>,
}

pub async fn save_task_trajectory(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    role: &str,
    agent_id: Option<&str>,
    card_id: Option<&str>,
    chat_id: &str,
    messages: &[crate::call_validation::ChatMessage],
    title: &str,
    model: &str,
) -> Result<PathBuf, String> {
    let task_dir = get_task_dir(gcx.clone(), task_id).await?;
    let traj_dir = get_task_trajectory_dir(&task_dir, role, agent_id);
    fs::create_dir_all(&traj_dir).await.map_err(|e| e.to_string())?;

    let file_path = traj_dir.join(format!("{}.json", chat_id));
    let now = chrono::Utc::now().to_rfc3339();

    let messages_json: Vec<serde_json::Value> = messages
        .iter()
        .filter_map(|m| serde_json::to_value(m).ok())
        .collect();

    let trajectory = serde_json::json!({
        "id": chat_id,
        "title": title,
        "model": model,
        "mode": "AGENT",
        "tool_use": "agent",
        "messages": messages_json,
        "created_at": now,
        "updated_at": now,
        "task_meta": TaskTrajectoryMeta {
            task_id: task_id.to_string(),
            role: role.to_string(),
            agent_id: agent_id.map(|s| s.to_string()),
            card_id: card_id.map(|s| s.to_string()),
        },
    });

    let tmp_path = file_path.with_extension("json.tmp");
    let json_str = serde_json::to_string_pretty(&trajectory)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    fs::write(&tmp_path, &json_str).await.map_err(|e| e.to_string())?;
    fs::rename(&tmp_path, &file_path).await.map_err(|e| e.to_string())?;

    Ok(file_path)
}

pub async fn list_task_trajectories(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    role: &str,
    agent_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let task_dir = get_task_dir(gcx, task_id).await?;
    let traj_dir = get_task_trajectory_dir(&task_dir, role, agent_id);

    if !traj_dir.exists() {
        return Ok(vec![]);
    }

    let mut ids = vec![];
    let mut entries = fs::read_dir(&traj_dir).await.map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "json") {
            if let Some(stem) = path.file_stem() {
                ids.push(stem.to_string_lossy().to_string());
            }
        }
    }
    Ok(ids)
}

pub async fn get_orchestrator_chat_id(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
) -> Result<String, String> {
    let existing = list_task_trajectories(gcx.clone(), task_id, "orchestrator", None).await?;
    if let Some(id) = existing.first() {
        return Ok(id.clone());
    }
    Ok(format!("orch-{}", task_id))
}

pub fn infer_task_id_from_chat_id(chat_id: &str) -> Option<String> {
    if let Some(id) = chat_id.strip_prefix("orch-") {
        return Some(id.to_string());
    }
    if let Some(id) = chat_id.strip_prefix("plan-") {
        return Some(id.to_string());
    }
    if let Some(rest) = chat_id.strip_prefix("planner-") {
        if let Some((task_id, suffix)) = rest.rsplit_once('-') {
            if !task_id.is_empty() && !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
                return Some(task_id.to_string());
            }
        }
        return Some(rest.to_string());
    }
    None
}

pub async fn get_planner_chat_id(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
) -> Result<String, String> {
    let existing = list_task_trajectories(gcx.clone(), task_id, "planner", None).await?;
    if let Some(id) = existing.first() {
        return Ok(id.clone());
    }
    Ok(format!("plan-{}", task_id))
}
