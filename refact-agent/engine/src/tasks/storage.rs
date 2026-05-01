use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use git2::Repository;
use tokio::sync::{RwLock as ARwLock, Mutex as AMutex};
use tokio::fs;
use tracing::warn;
use uuid::Uuid;
use chrono::Utc;

use crate::global_context::GlobalContext;
use crate::files_correction::get_project_dirs;
use super::types::{TaskMeta, TaskBoard, TaskStatus, TrajectoryInfo};
use super::events::{TaskEvent, emit_task_event};

const TASKS_DIR: &str = "tasks";

async fn atomic_write_file(tmp_path: &Path, dest_path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if dest_path.exists() {
        fs::remove_file(dest_path)
            .await
            .map_err(|e| format!("Failed to remove existing file: {}", e))?;
    }
    fs::rename(tmp_path, dest_path)
        .await
        .map_err(|e| format!("Failed to rename: {}", e))
}

static BOARD_LOCKS: std::sync::OnceLock<AMutex<HashMap<String, Arc<AMutex<()>>>>> =
    std::sync::OnceLock::new();

fn get_board_locks() -> &'static AMutex<HashMap<String, Arc<AMutex<()>>>> {
    BOARD_LOCKS.get_or_init(|| AMutex::new(HashMap::new()))
}

async fn get_board_lock(task_id: &str) -> Arc<AMutex<()>> {
    let mut locks = get_board_locks().lock().await;
    locks
        .entry(task_id.to_string())
        .or_insert_with(|| Arc::new(AMutex::new(())))
        .clone()
}

pub async fn get_tasks_dir(gcx: Arc<ARwLock<GlobalContext>>) -> Result<PathBuf, String> {
    let project_dirs = get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    Ok(workspace_root.join(".refact").join(TASKS_DIR))
}

pub async fn get_global_tasks_dir(gcx: Arc<ARwLock<GlobalContext>>) -> PathBuf {
    let config_dir = gcx.read().await.config_dir.clone();
    config_dir.join(TASKS_DIR)
}

pub async fn get_all_tasks_dirs(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .map(|p| p.join(".refact").join(TASKS_DIR))
        .filter(|p| p.exists())
        .collect();

    let global_dir = get_global_tasks_dir(gcx).await;
    if global_dir.exists() {
        dirs.push(global_dir);
    }

    dirs
}

pub async fn find_task_dir(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
) -> Result<PathBuf, String> {
    validate_task_id(task_id)?;
    for tasks_dir in get_all_tasks_dirs(gcx).await {
        let candidate = tasks_dir.join(task_id);
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    Err(format!("Task not found: {}", task_id))
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
    if task_id.len() > 128 {
        return Err("Task ID too long".into());
    }
    if !task_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "Task ID must contain only alphanumeric characters, hyphens, or underscores".into(),
        );
    }
    Ok(())
}

pub async fn list_tasks(gcx: Arc<ARwLock<GlobalContext>>) -> Result<Vec<TaskMeta>, String> {
    let tasks_dirs = get_all_tasks_dirs(gcx.clone()).await;

    let mut tasks = vec![];
    let mut seen_ids = HashSet::new();

    for tasks_dir in tasks_dirs {
        if !tasks_dir.exists() {
            continue;
        }

        let mut entries = match fs::read_dir(&tasks_dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                let meta_path = path.join("meta.yaml");
                if meta_path.exists() {
                    match load_task_meta_from_path(&meta_path).await {
                        Ok(meta) => {
                            if seen_ids.insert(meta.id.clone()) {
                                tasks.push(meta);
                            }
                        }
                        Err(e) => warn!("Failed to load task meta from {:?}: {}", meta_path, e),
                    }
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

pub async fn load_task_meta(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
) -> Result<TaskMeta, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let meta_path = task_dir.join("meta.yaml");
    load_task_meta_from_path(&meta_path).await
}

pub async fn save_task_meta(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    meta: &TaskMeta,
) -> Result<(), String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let meta_path = task_dir.join("meta.yaml");
    let content = serde_yaml::to_string(meta).map_err(|e| e.to_string())?;
    fs::write(&meta_path, content)
        .await
        .map_err(|e| e.to_string())
}

pub async fn load_board(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
) -> Result<TaskBoard, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let board_path = task_dir.join("board.yaml");
    if !board_path.exists() {
        return Ok(TaskBoard::default());
    }
    let content = fs::read_to_string(&board_path)
        .await
        .map_err(|e| e.to_string())?;
    serde_yaml::from_str(&content).map_err(|e| e.to_string())
}

pub async fn save_board(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    board: &TaskBoard,
) -> Result<(), String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let board_path = task_dir.join("board.yaml");
    let tmp_path = task_dir.join("board.yaml.tmp");
    let content = serde_yaml::to_string(board).map_err(|e| e.to_string())?;
    fs::write(&tmp_path, &content)
        .await
        .map_err(|e| e.to_string())?;
    atomic_write_file(&tmp_path, &board_path).await
}

pub async fn update_board_atomic<F, T>(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    mut updater: F,
) -> Result<(TaskBoard, T), String>
where
    F: FnMut(&mut TaskBoard) -> Result<T, String>,
    T: Default,
{
    let lock = get_board_lock(task_id).await;
    let _guard = lock.lock().await;

    let mut board = load_board(gcx.clone(), task_id).await?;
    let result = updater(&mut board)?;
    board.rev += 1;
    save_board(gcx.clone(), task_id, &board).await?;
    emit_task_event(
        gcx,
        TaskEvent::BoardChanged {
            task_id: task_id.to_string(),
            rev: board.rev,
            board: board.clone(),
        },
    )
    .await;
    Ok((board, result))
}

pub async fn load_planner_instructions(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
) -> Result<String, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let path = task_dir.join("planner_instructions.md");
    if !path.exists() {
        return Ok(String::new());
    }
    fs::read_to_string(&path).await.map_err(|e| e.to_string())
}

pub async fn save_planner_instructions(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    content: &str,
) -> Result<(), String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let path = task_dir.join("planner_instructions.md");
    fs::write(&path, content).await.map_err(|e| e.to_string())
}

fn detect_git_branch_and_commit(workspace_root: &Path) -> (Option<String>, Option<String>) {
    let repo = match Repository::open(workspace_root) {
        Ok(r) => r,
        Err(_) => return (None, None),
    };

    let branch = match repo.head() {
        Ok(head) => head.shorthand().map(|s| s.to_string()),
        Err(_) => None,
    };

    let commit = match repo.head() {
        Ok(head) => match head.peel_to_commit() {
            Ok(c) => Some(c.id().to_string()),
            Err(_) => None,
        },
        Err(_) => None,
    };

    (branch, commit)
}

pub async fn create_task(gcx: Arc<ARwLock<GlobalContext>>, name: &str) -> Result<TaskMeta, String> {
    let tasks_dir = ensure_tasks_dir(gcx.clone()).await?;
    let task_id = Uuid::new_v4().to_string();
    let task_dir = tasks_dir.join(&task_id);

    fs::create_dir_all(&task_dir)
        .await
        .map_err(|e| e.to_string())?;
    fs::create_dir_all(task_dir.join("trajectories").join("planner"))
        .await
        .map_err(|e| e.to_string())?;
    fs::create_dir_all(task_dir.join("trajectories").join("agents"))
        .await
        .map_err(|e| e.to_string())?;

    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    let (base_branch, base_commit) = project_dirs
        .first()
        .map(|p| detect_git_branch_and_commit(p))
        .unwrap_or((None, None));

    let now = Utc::now().to_rfc3339();
    let has_user_provided_name = !name.trim().is_empty() && name.to_lowercase() != "new task";
    let meta = TaskMeta {
        schema_version: 1,
        id: task_id.clone(),
        name: if has_user_provided_name {
            name.to_string()
        } else {
            "New Task".to_string()
        },
        status: TaskStatus::Planning,
        created_at: now.clone(),
        updated_at: now,
        cards_total: 0,
        cards_done: 0,
        cards_failed: 0,
        agents_active: 0,
        base_branch,
        base_commit,
        default_agent_model: None,
        is_name_generated: has_user_provided_name,
        last_agents_summary_at: None,
        planner_session_state: None,
    };

    save_task_meta(gcx.clone(), &task_id, &meta).await?;
    save_board(gcx.clone(), &task_id, &TaskBoard::default()).await?;
    save_planner_instructions(gcx.clone(), &task_id, "").await?;

    let planner_chat_id = format!("planner-{}-1", task_id);
    crate::chat::trajectories::save_initial_planner_trajectory(
        gcx.clone(),
        &task_id,
        &planner_chat_id,
    )
    .await?;

    emit_task_event(
        gcx.clone(),
        TaskEvent::TaskCreated {
            task_id: task_id.clone(),
            meta: meta.clone(),
        },
    )
    .await;
    {
        let ev = crate::buddy::actor::make_runtime_event(
            "task_created",
            &format!("Task created: {}", meta.name),
            "task",
            &format!("task_{}", task_id),
            "completed",
            None,
        );
        crate::buddy::actor::buddy_enqueue_event(gcx, ev).await;
    }

    Ok(meta)
}

pub async fn delete_task(gcx: Arc<ARwLock<GlobalContext>>, task_id: &str) -> Result<(), String> {
    let task_dir = find_task_dir(gcx.clone(), task_id).await?;
    fs::remove_dir_all(&task_dir)
        .await
        .map_err(|e| e.to_string())?;
    emit_task_event(
        gcx,
        TaskEvent::TaskDeleted {
            task_id: task_id.to_string(),
        },
    )
    .await;
    Ok(())
}

pub async fn update_task_name(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    name: &str,
) -> Result<TaskMeta, String> {
    let mut meta = load_task_meta(gcx.clone(), task_id).await?;
    meta.name = name.to_string();
    meta.is_name_generated = true;
    meta.updated_at = Utc::now().to_rfc3339();
    save_task_meta(gcx.clone(), task_id, &meta).await?;
    emit_task_event(
        gcx,
        TaskEvent::TaskUpdated {
            task_id: task_id.to_string(),
            meta: meta.clone(),
        },
    )
    .await;
    Ok(meta)
}

pub async fn update_task_stats(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
) -> Result<TaskMeta, String> {
    let mut meta = load_task_meta(gcx.clone(), task_id).await?;
    let board = load_board(gcx.clone(), task_id).await?;

    meta.cards_total = board.cards.len();
    meta.cards_done = board.cards.iter().filter(|c| c.column == "done").count();
    meta.cards_failed = board.cards.iter().filter(|c| c.column == "failed").count();
    meta.agents_active = board
        .cards
        .iter()
        .filter(|c| c.column == "doing" && c.assignee.is_some())
        .count();
    meta.updated_at = Utc::now().to_rfc3339();

    save_task_meta(gcx.clone(), task_id, &meta).await?;
    emit_task_event(
        gcx,
        TaskEvent::TaskUpdated {
            task_id: task_id.to_string(),
            meta: meta.clone(),
        },
    )
    .await;
    Ok(meta)
}

pub fn get_task_trajectory_dir(task_dir: &PathBuf, role: &str, agent_id: Option<&str>) -> PathBuf {
    let base = task_dir.join("trajectories").join(role);
    match agent_id {
        Some(id) => base.join(id),
        None => base,
    }
}

fn should_list_task_trajectory(role: &str, data: &serde_json::Value) -> bool {
    if role != "planner" {
        return true;
    }
    match data.get("link_type").and_then(|v| v.as_str()) {
        Some("handoff" | "mode_transition" | "branch") | None => true,
        Some(_) => false,
    }
}

/// Compute the next planner chat id in the standard `planner-{task_id}-{n}` format.
///
/// NOTE: this is not strictly atomic — concurrent callers can pick the same
/// number. Callers that care about uniqueness must serialize themselves.
pub async fn next_planner_chat_id(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
) -> Result<String, String> {
    let existing = list_task_trajectories(gcx, task_id, "planner", None).await?;
    let prefix = format!("planner-{}-", task_id);
    let max_num = existing
        .iter()
        .filter_map(|t| {
            t.id.strip_prefix(&prefix)
                .and_then(|s| s.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);
    Ok(format!("planner-{}-{}", task_id, max_num + 1))
}

pub async fn list_task_trajectories(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    role: &str,
    agent_id: Option<&str>,
) -> Result<Vec<TrajectoryInfo>, String> {
    let task_dir = find_task_dir(gcx.clone(), task_id).await?;
    let traj_dir = get_task_trajectory_dir(&task_dir, role, agent_id);

    if !traj_dir.exists() {
        return Ok(vec![]);
    }

    let mut trajectories: Vec<TrajectoryInfo> = vec![];
    let mut entries = fs::read_dir(&traj_dir).await.map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "json") {
            if let Some(stem) = path.file_stem() {
                let id = stem.to_string_lossy().to_string();
                if let Ok(content) = fs::read_to_string(&path).await {
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                        if !should_list_task_trajectory(role, &data) {
                            continue;
                        }
                        let title = data
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let created_at = data
                            .get("created_at")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let updated_at = data
                            .get("updated_at")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        trajectories.push(TrajectoryInfo {
                            id,
                            title,
                            created_at,
                            updated_at,
                            session_state: None,
                        });
                    }
                }
            }
        }
    }

    trajectories.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    let gcx_locked = gcx.read().await;
    let sessions = gcx_locked.chat_sessions.read().await;
    for traj in &mut trajectories {
        if let Some(session_arc) = sessions.get(&traj.id) {
            let session = session_arc.lock().await;
            traj.session_state = Some(session.runtime.state.to_string());
        }
    }

    Ok(trajectories)
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
            if !task_id.is_empty()
                && !suffix.is_empty()
                && suffix.chars().all(|c| c.is_ascii_digit())
            {
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
    let trajectories = list_task_trajectories(gcx, task_id, "planner", None).await?;

    trajectories
        .iter()
        .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
        .map(|t| t.id.clone())
        .ok_or_else(|| format!("plan-{}", task_id))
        .or_else(|default| Ok(default))
}

#[cfg(test)]
mod tests {
    use super::should_list_task_trajectory;
    use serde_json::json;

    #[test]
    fn planner_listing_hides_subagentic_child_links() {
        assert!(!should_list_task_trajectory(
            "planner",
            &json!({"link_type": "gather_files"})
        ));
        assert!(!should_list_task_trajectory(
            "planner",
            &json!({"link_type": "subagent"})
        ));
    }

    #[test]
    fn planner_listing_keeps_real_planner_chats() {
        assert!(should_list_task_trajectory("planner", &json!({})));
        assert!(should_list_task_trajectory(
            "planner",
            &json!({"link_type": "handoff"})
        ));
        assert!(should_list_task_trajectory(
            "planner",
            &json!({"link_type": "mode_transition"})
        ));
    }

    #[test]
    fn non_planner_listing_keeps_child_links() {
        assert!(should_list_task_trajectory(
            "subchats",
            &json!({"link_type": "gather_files"})
        ));
    }
}
