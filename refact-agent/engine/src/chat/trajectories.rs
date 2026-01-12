use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::Instant;
use axum::extract::Path;
use axum::http::{Response, StatusCode};
use axum::Extension;
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock, broadcast};
use tokio::fs;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{info, warn};
use uuid::Uuid;


use crate::call_validation::{ChatMessage, ChatContent};
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::files_correction::get_project_dirs;
use crate::subchat::run_subchat_once;

use super::types::{ThreadParams, SessionState, ChatSession};
use super::config::timeouts;

const TITLE_GENERATION_PROMPT: &str = "Summarize this chat in 2-4 words. Prefer filenames, classes, entities, and avoid generic terms. Write only the title, nothing else.";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrajectoryEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrajectoryMeta {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub mode: String,
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_state: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrajectoryData {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub mode: String,
    pub tool_use: String,
    pub messages: Vec<serde_json::Value>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

pub struct LoadedTrajectory {
    pub messages: Vec<ChatMessage>,
    pub thread: ThreadParams,
    pub created_at: String,
}

#[derive(Clone)]
pub struct TrajectorySnapshot {
    pub chat_id: String,
    pub title: String,
    pub model: String,
    pub mode: String,
    pub tool_use: String,
    pub messages: Vec<ChatMessage>,
    pub created_at: String,
    pub boost_reasoning: bool,
    pub checkpoints_enabled: bool,
    pub context_tokens_cap: Option<usize>,
    pub include_project_info: bool,
    pub is_title_generated: bool,
    pub automatic_patch: bool,
    pub version: u64,
    pub task_meta: Option<super::types::TaskMeta>,
    pub parent_id: Option<String>,
    pub link_type: Option<String>,
}

impl TrajectorySnapshot {
    pub fn from_session(session: &ChatSession) -> Self {
        Self {
            chat_id: session.chat_id.clone(),
            title: session.thread.title.clone(),
            model: session.thread.model.clone(),
            mode: session.thread.mode.clone(),
            tool_use: session.thread.tool_use.clone(),
            messages: session.messages.clone(),
            created_at: session.created_at.clone(),
            boost_reasoning: session.thread.boost_reasoning,
            checkpoints_enabled: session.thread.checkpoints_enabled,
            context_tokens_cap: session.thread.context_tokens_cap,
            include_project_info: session.thread.include_project_info,
            is_title_generated: session.thread.is_title_generated,
            automatic_patch: session.thread.automatic_patch,
            version: session.trajectory_version,
            task_meta: session.thread.task_meta.clone(),
            parent_id: session.thread.parent_id.clone(),
            link_type: session.thread.link_type.clone(),
        }
    }
}

pub async fn get_trajectories_dir(gcx: Arc<ARwLock<GlobalContext>>) -> Result<PathBuf, String> {
    let project_dirs = get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    Ok(workspace_root.join(".refact").join("trajectories"))
}

pub async fn get_all_trajectories_dirs(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    get_project_dirs(gcx)
        .await
        .into_iter()
        .map(|p| p.join(".refact").join("trajectories"))
        .filter(|p| p.exists())
        .collect()
}

async fn get_trajectories_dir_from_weak(
    gcx_weak: &Weak<ARwLock<GlobalContext>>,
) -> Option<PathBuf> {
    let gcx = gcx_weak.upgrade()?;
    get_trajectories_dir(gcx).await.ok()
}

fn fix_tool_call_indexes(messages: &mut [ChatMessage]) {
    for msg in messages.iter_mut() {
        if let Some(ref mut tool_calls) = msg.tool_calls {
            for (i, tc) in tool_calls.iter_mut().enumerate() {
                if tc.index.is_none() {
                    tc.index = Some(i);
                }
            }
        }
    }
}

async fn find_trajectory_path(gcx: Arc<ARwLock<GlobalContext>>, chat_id: &str) -> Option<PathBuf> {
    let traj_dirs = get_all_trajectories_dirs(gcx.clone()).await;
    if let Some(path) = traj_dirs
        .iter()
        .map(|dir| dir.join(format!("{}.json", chat_id)))
        .find(|p| p.exists())
    {
        return Some(path);
    }

    if let Ok(tasks_dir) = crate::tasks::storage::get_tasks_dir(gcx.clone()).await {
        if tasks_dir.exists() {
            if let Ok(mut entries) = tokio::fs::read_dir(&tasks_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let task_dir = entry.path();
                    if task_dir.is_dir() {
                        let traj_base = task_dir.join("trajectories");
                        for role in &["planner", "agents"] {
                            let role_dir = traj_base.join(role);
                            if role_dir.exists() {
                                let direct = role_dir.join(format!("{}.json", chat_id));
                                if direct.exists() {
                                    return Some(direct);
                                }
                                if let Ok(mut sub_entries) = tokio::fs::read_dir(&role_dir).await {
                                    while let Ok(Some(sub_entry)) = sub_entries.next_entry().await {
                                        let sub_path = sub_entry.path();
                                        if sub_path.is_dir() {
                                            let nested = sub_path.join(format!("{}.json", chat_id));
                                            if nested.exists() {
                                                return Some(nested);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

pub async fn load_trajectory_for_chat(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
) -> Option<LoadedTrajectory> {
    let traj_path = find_trajectory_path(gcx, chat_id).await?;
    let content = tokio::fs::read_to_string(&traj_path).await.ok()?;
    let t: serde_json::Value = serde_json::from_str(&content).ok()?;

    let mut messages: Vec<ChatMessage> = t
        .get("messages")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    fix_tool_call_indexes(&mut messages);
    for msg in &mut messages {
        if msg.message_id.is_empty() {
            msg.message_id = Uuid::new_v4().to_string();
        }
    }

    let task_meta: Option<super::types::TaskMeta> = t
        .get("task_meta")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let thread = ThreadParams {
        id: chat_id.to_string(),
        title: t
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("New Chat")
            .to_string(),
        model: t
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        mode: t
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("AGENT")
            .to_string(),
        tool_use: t
            .get("tool_use")
            .and_then(|v| v.as_str())
            .unwrap_or("agent")
            .to_string(),
        boost_reasoning: t
            .get("boost_reasoning")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        context_tokens_cap: t
            .get("context_tokens_cap")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize),
        include_project_info: t
            .get("include_project_info")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        checkpoints_enabled: t
            .get("checkpoints_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        is_title_generated: t
            .get("isTitleGenerated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        automatic_patch: t
            .get("automatic_patch")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        task_meta,
        parent_id: t
            .get("parent_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        link_type: t
            .get("link_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    };

    let created_at = t
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or(&chrono::Utc::now().to_rfc3339())
        .to_string();

    Some(LoadedTrajectory {
        messages,
        thread,
        created_at,
    })
}

pub async fn save_initial_planner_trajectory(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    chat_id: &str,
) -> Result<(), String> {
    let greeting = "## 🎯 Task Planner

I'm your **Task Planner**. I handle the complete task lifecycle - from investigation to execution.

**Planning Phase:**
- Analyze the codebase using search and exploration tools
- Create task cards with clear acceptance criteria
- Set priorities and dependencies between cards

**Execution Phase:**
- Spawn agents to work on ready cards (each in isolated git worktree)
- Monitor agent progress and receive completion notifications
- Merge successful work back to main branch
- Handle failures and coordinate retries

**How to use me:**
1. Describe what you want to accomplish
2. I'll investigate and create a structured plan (task cards)
3. When ready, I'll spawn agents to implement each card
4. I'll notify you as work completes and handle merging
5. We iterate until the task is done

**Ready when you are!** Tell me what you'd like to build or fix.";

    let now = chrono::Utc::now().to_rfc3339();
    let greeting_msg = ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "assistant".to_string(),
        content: ChatContent::SimpleText(greeting.to_string()),
        finish_reason: Some("stop".to_string()),
        reasoning_content: None,
        tool_calls: None,
        tool_call_id: String::new(),
        tool_failed: None,
        usage: None,
        checkpoints: vec![],
        thinking_blocks: None,
        citations: vec![],
        extra: serde_json::Map::new(),
        output_filter: None,
    };

    let messages_json = vec![serde_json::to_value(&greeting_msg).unwrap_or_default()];

    let task_meta = super::types::TaskMeta {
        task_id: task_id.to_string(),
        role: "planner".to_string(),
        agent_id: None,
        card_id: None,
    };

    let trajectory = json!({
        "id": chat_id,
        "title": "",
        "model": "",
        "mode": "TASK_PLANNER",
        "tool_use": "agent",
        "messages": messages_json,
        "created_at": now.clone(),
        "updated_at": now,
        "boost_reasoning": false,
        "checkpoints_enabled": true,
        "context_tokens_cap": null,
        "include_project_info": true,
        "isTitleGenerated": false,
        "automatic_patch": false,
        "task_meta": serde_json::to_value(&task_meta).unwrap_or_default(),
    });

    let task_dir = crate::tasks::storage::get_task_dir(gcx.clone(), task_id).await?;
    let traj_dir = crate::tasks::storage::get_task_trajectory_dir(&task_dir, "planner", None);
    tokio::fs::create_dir_all(&traj_dir)
        .await
        .map_err(|e| format!("Failed to create task trajectories dir: {}", e))?;

    let file_path = traj_dir.join(format!("{}.json", chat_id));
    let tmp_path = file_path.with_extension("json.tmp");
    let json_str = serde_json::to_string_pretty(&trajectory)
        .map_err(|e| format!("Failed to serialize trajectory: {}", e))?;
    tokio::fs::write(&tmp_path, &json_str)
        .await
        .map_err(|e| format!("Failed to write trajectory: {}", e))?;
    tokio::fs::rename(&tmp_path, &file_path)
        .await
        .map_err(|e| format!("Failed to rename trajectory: {}", e))?;

    info!(
        "Created initial planner trajectory for task {} at {:?}",
        task_id, file_path
    );

    Ok(())
}

pub async fn save_trajectory_as(
    gcx: Arc<ARwLock<GlobalContext>>,
    thread: &ThreadParams,
    messages: &[ChatMessage],
) {
    if messages.is_empty() {
        return;
    }
    let snapshot = TrajectorySnapshot {
        chat_id: thread.id.clone(),
        title: thread.title.clone(),
        model: thread.model.clone(),
        mode: thread.mode.clone(),
        tool_use: thread.tool_use.clone(),
        messages: messages.to_vec(),
        created_at: chrono::Utc::now().to_rfc3339(),
        boost_reasoning: thread.boost_reasoning,
        checkpoints_enabled: thread.checkpoints_enabled,
        context_tokens_cap: thread.context_tokens_cap,
        include_project_info: thread.include_project_info,
        is_title_generated: thread.is_title_generated,
        automatic_patch: thread.automatic_patch,
        version: 1,
        task_meta: thread.task_meta.clone(),
        parent_id: thread.parent_id.clone(),
        link_type: thread.link_type.clone(),
    };
    if let Err(e) = save_trajectory_snapshot(gcx, snapshot).await {
        warn!("Failed to save trajectory: {}", e);
    }
}

pub async fn save_trajectory_snapshot(
    gcx: Arc<ARwLock<GlobalContext>>,
    snapshot: TrajectorySnapshot,
) -> Result<(), String> {
    if snapshot.messages.is_empty() && snapshot.task_meta.is_none() {
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339();
    let messages_json: Vec<serde_json::Value> = snapshot
        .messages
        .iter()
        .map(|m| serde_json::to_value(m).unwrap_or_default())
        .collect();

    let mut trajectory = json!({
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
    });

    if let Some(ref parent_id) = snapshot.parent_id {
        trajectory["parent_id"] = serde_json::Value::String(parent_id.clone());
    }
    if let Some(ref link_type) = snapshot.link_type {
        trajectory["link_type"] = serde_json::Value::String(link_type.clone());
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
        let trajectories_dir = get_trajectories_dir(gcx.clone()).await?;
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

    info!(
        "Saved trajectory for chat {} ({} messages) to {:?}",
        snapshot.chat_id,
        snapshot.messages.len(),
        file_path
    );

    if let Some(vecdb) = gcx.read().await.vec_db.lock().await.as_ref() {
        vecdb
            .vectorizer_enqueue_files(&vec![file_path.to_string_lossy().to_string()], false)
            .await;
    }

    if snapshot.task_meta.is_none() {
        if let Some(tx) = &gcx.read().await.trajectory_events_tx {
            let event = TrajectoryEvent {
                event_type: "updated".to_string(),
                id: snapshot.chat_id.clone(),
                updated_at: Some(now),
                title: Some(snapshot.title.clone()),
            };
            let _ = tx.send(event);
        }

        let should_generate_title = is_placeholder_title(&snapshot.title)
            && !snapshot.is_title_generated
            && !snapshot.messages.is_empty();

        if should_generate_title {
            let trajectories_dir = get_trajectories_dir(gcx.clone()).await?;
            let _ = spawn_title_generation_task(
                gcx.clone(),
                snapshot.chat_id.clone(),
                messages_json,
                trajectories_dir,
            );
        }
    } else if let Some(ref task_meta) = snapshot.task_meta {
        if task_meta.role == "planner" {
            let user_message_count = count_user_messages(&messages_json);
            if user_message_count >= 1 {
                spawn_task_name_generation_task(
                    gcx.clone(),
                    task_meta.task_id.clone(),
                    messages_json,
                );
            }
        }
    }

    Ok(())
}

pub async fn maybe_save_trajectory(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
) {
    let snapshot = {
        let session = session_arc.lock().await;
        if !session.trajectory_dirty {
            return;
        }
        TrajectorySnapshot::from_session(&session)
    };

    let saved_version = snapshot.version;
    let chat_id = snapshot.chat_id.clone();

    match save_trajectory_snapshot(gcx, snapshot).await {
        Ok(()) => {
            let mut session = session_arc.lock().await;
            if session.trajectory_version == saved_version {
                session.trajectory_dirty = false;
            }
        }
        Err(e) => {
            warn!("Failed to save trajectory for {}: {}", chat_id, e);
        }
    }
}

pub async fn check_external_reload_pending(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
) {
    let (chat_id, should_reload) = {
        let session = session_arc.lock().await;
        (
            session.chat_id.clone(),
            session.external_reload_pending
                && session.runtime.state == SessionState::Idle
                && !session.trajectory_dirty,
        )
    };
    if !should_reload {
        return;
    }
    if let Some(loaded) = load_trajectory_for_chat(gcx.clone(), &chat_id).await {
        let mut session = session_arc.lock().await;
        if session.runtime.state == SessionState::Idle && !session.trajectory_dirty {
            info!("Applying pending external reload for {}", chat_id);
            session.messages = loaded.messages;
            session.thread = loaded.thread;
            session.created_at = loaded.created_at;
            session.external_reload_pending = false;
            let snapshot = session.snapshot();
            session.emit(snapshot);
        }
    }
}

async fn process_trajectory_change(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
    is_remove: bool,
) {
    if is_remove {
        if let Some(tx) = &gcx.read().await.trajectory_events_tx {
            let _ = tx.send(TrajectoryEvent {
                event_type: "deleted".to_string(),
                id: chat_id.to_string(),
                updated_at: None,
                title: None,
            });
        }
    } else {
        let (updated_at, title) = load_trajectory_for_chat(gcx.clone(), chat_id)
            .await
            .map(|t| (Some(chrono::Utc::now().to_rfc3339()), Some(t.thread.title)))
            .unwrap_or((None, None));
        if let Some(tx) = &gcx.read().await.trajectory_events_tx {
            let _ = tx.send(TrajectoryEvent {
                event_type: "updated".to_string(),
                id: chat_id.to_string(),
                updated_at,
                title,
            });
        }
    }

    let sessions = gcx.read().await.chat_sessions.clone();
    let session_arc = {
        let sessions_read = sessions.read().await;
        sessions_read.get(chat_id).cloned()
    };

    let Some(session_arc) = session_arc else {
        return;
    };

    let can_reload = {
        let session = session_arc.lock().await;
        session.runtime.state == SessionState::Idle && !session.trajectory_dirty
    };

    if !can_reload {
        let mut session = session_arc.lock().await;
        session.external_reload_pending = true;
        return;
    }

    if is_remove {
        let mut session = session_arc.lock().await;
        info!("Trajectory file removed externally for {}", chat_id);
        session.messages.clear();
        session.thread = ThreadParams {
            id: chat_id.to_string(),
            ..Default::default()
        };
        let snapshot = session.snapshot();
        session.emit(snapshot);
        return;
    }

    if let Some(loaded) = load_trajectory_for_chat(gcx.clone(), chat_id).await {
        let mut session = session_arc.lock().await;
        if session.runtime.state != SessionState::Idle || session.trajectory_dirty {
            session.external_reload_pending = true;
            return;
        }
        info!("Reloading trajectory for {} from external change", chat_id);
        session.messages = loaded.messages;
        session.thread = loaded.thread;
        session.created_at = loaded.created_at;
        session.external_reload_pending = false;
        let snapshot = session.snapshot();
        session.emit(snapshot);
    }
}

pub fn start_trajectory_watcher(gcx: Arc<ARwLock<GlobalContext>>) {
    let gcx_weak = Arc::downgrade(&gcx);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, bool)>();

    tokio::spawn(async move {
        let trajectories_dir = match get_trajectories_dir_from_weak(&gcx_weak).await {
            Some(dir) => dir,
            None => {
                warn!("No workspace folder found, trajectory watcher not started");
                return;
            }
        };

        if let Err(e) = tokio::fs::create_dir_all(&trajectories_dir).await {
            warn!("Failed to create trajectories dir for watcher: {}", e);
            return;
        }

        let tx_clone = tx.clone();
        let event_callback = move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                let dominated = matches!(
                    event.kind,
                    notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_)
                );
                if !dominated {
                    return;
                }
                let is_remove = matches!(event.kind, notify::EventKind::Remove(_));
                for path in event.paths {
                    if path.extension().map(|e| e == "tmp").unwrap_or(false) {
                        continue;
                    }
                    if let Some(chat_id) = path.file_stem().and_then(|s| s.to_str()) {
                        if path.extension().map(|e| e == "json").unwrap_or(false) {
                            let _ = tx_clone.send((chat_id.to_string(), is_remove));
                        }
                    }
                }
            }
        };

        let watcher = match RecommendedWatcher::new(event_callback, Config::default()) {
            Ok(w) => w,
            Err(e) => {
                warn!("Failed to create trajectory watcher: {}", e);
                return;
            }
        };

        let _watcher = Arc::new(std::sync::Mutex::new(watcher));
        {
            let mut w = _watcher.lock().unwrap();
            if let Err(e) = w.watch(&trajectories_dir, RecursiveMode::NonRecursive) {
                warn!("Failed to watch trajectories dir: {}", e);
                return;
            }
        }
        info!(
            "Trajectory watcher started for {}",
            trajectories_dir.display()
        );

        let mut pending: std::collections::HashMap<String, (Instant, bool)> =
            std::collections::HashMap::new();
        let debounce = timeouts().watcher_debounce;

        loop {
            let timeout = if pending.is_empty() {
                timeouts().watcher_idle
            } else {
                timeouts().watcher_poll
            };

            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some((chat_id, is_remove)) => {
                            pending.insert(chat_id, (Instant::now(), is_remove));
                        }
                        None => break,
                    }
                }
                _ = tokio::time::sleep(timeout) => {
                    if gcx_weak.upgrade().is_none() {
                        break;
                    }
                }
            }

            let now = Instant::now();
            let ready: Vec<_> = pending
                .iter()
                .filter(|(_, (t, _))| now.duration_since(*t) >= debounce)
                .map(|(k, v)| (k.clone(), v.1))
                .collect();

            for (chat_id, is_remove) in ready {
                pending.remove(&chat_id);
                if let Some(gcx) = gcx_weak.upgrade() {
                    process_trajectory_change(gcx, &chat_id, is_remove).await;
                }
            }
        }
    });
}

fn validate_trajectory_id(id: &str) -> Result<(), ScratchError> {
    if id.contains('/') || id.contains('\\') || id.contains("..") || id.contains('\0') {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid trajectory id".to_string(),
        ));
    }
    Ok(())
}

async fn atomic_write_json(path: &PathBuf, data: &impl Serialize) -> Result<(), String> {
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    fs::write(&tmp_path, &json)
        .await
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp_path, path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn is_placeholder_title(title: &str) -> bool {
    let normalized = title.trim().to_lowercase();
    normalized.is_empty() || normalized == "new chat" || normalized == "untitled"
}

fn is_placeholder_task_name(name: &str) -> bool {
    let normalized = name.trim().to_lowercase();
    normalized.is_empty() || normalized == "new task" || normalized == "untitled"
}

fn count_user_messages(messages: &[serde_json::Value]) -> usize {
    messages
        .iter()
        .filter(|msg| {
            msg.get("role")
                .and_then(|r| r.as_str())
                .map(|r| r == "user")
                .unwrap_or(false)
        })
        .count()
}

fn extract_first_user_message(messages: &[serde_json::Value]) -> Option<String> {
    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "user" {
            continue;
        }
        if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.chars().take(200).collect());
            }
        }
        if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
            for item in content_arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.chars().take(200).collect());
                    }
                }
                if let Some(text) = item.get("m_content").and_then(|t| t.as_str()) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.chars().take(200).collect());
                    }
                }
            }
        }
    }
    None
}

fn build_title_generation_context(messages: &[serde_json::Value]) -> String {
    let mut context = String::new();
    let max_messages = 6;
    let max_chars_per_message = 500;

    for (i, msg) in messages.iter().take(max_messages).enumerate() {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("unknown");
        if role == "tool" || role == "context_file" || role == "cd_instruction" {
            continue;
        }
        let content_text = if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
            content.to_string()
        } else if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
            content_arr
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .and_then(|t| t.as_str())
                        .or_else(|| item.get("m_content").and_then(|t| t.as_str()))
                })
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            continue;
        };
        let truncated: String = content_text.chars().take(max_chars_per_message).collect();
        if !truncated.trim().is_empty() {
            context.push_str(&format!("{}: {}\n\n", role, truncated));
        }
        if i >= max_messages - 1 {
            break;
        }
    }
    context
}

fn clean_generated_title(raw_title: &str) -> String {
    let cleaned = raw_title
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
        .trim_matches('*')
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.chars().count() > 60 {
        cleaned.chars().take(57).collect::<String>() + "..."
    } else {
        cleaned
    }
}

async fn generate_title_llm(
    gcx: Arc<ARwLock<GlobalContext>>,
    messages: &[serde_json::Value],
) -> Option<String> {
    let context = build_title_generation_context(messages);
    if context.trim().is_empty() {
        return None;
    }
    let prompt = format!(
        "Chat conversation:\n{}\n\n{}",
        context, TITLE_GENERATION_PROMPT
    );
    let chat_messages = vec![ChatMessage::new("user".to_string(), prompt)];

    match run_subchat_once(gcx, "title_generation", chat_messages).await {
        Ok(result) => {
            if let Some(last_msg) = result.messages.last() {
                let raw_title = last_msg.content.content_text_only();
                let cleaned = clean_generated_title(&raw_title);
                if !cleaned.is_empty() && cleaned.to_lowercase() != "new chat" {
                    info!("Generated title: {}", cleaned);
                    return Some(cleaned);
                }
            }
            None
        }
        Err(e) => {
            warn!("Title generation failed: {}", e);
            None
        }
    }
}

fn spawn_title_generation_task(
    gcx: Arc<ARwLock<GlobalContext>>,
    id: String,
    messages: Vec<serde_json::Value>,
    trajectories_dir: PathBuf,
) {
    tokio::spawn(async move {
        let generated_title = generate_title_llm(gcx.clone(), &messages).await;
        let title = match generated_title {
            Some(t) => t,
            None => match extract_first_user_message(&messages) {
                Some(first_msg) => {
                    let truncated: String = first_msg.chars().take(60).collect();
                    if truncated.len() < first_msg.len() {
                        format!("{}...", truncated.trim_end())
                    } else {
                        truncated
                    }
                }
                None => return,
            },
        };
        let sessions = gcx.read().await.chat_sessions.clone();
        let maybe_session_arc = {
            let sessions_read = sessions.read().await;
            sessions_read.get(&id).cloned()
        };
        if let Some(session_arc) = maybe_session_arc {
            let mut session = session_arc.lock().await;
            if session.thread.is_title_generated {
                info!("Title already generated for {}, skipping", id);
                return;
            }
            session.set_title(title.clone(), true);
            drop(session);
            maybe_save_trajectory(gcx.clone(), session_arc).await;
            info!("Updated session {} with generated title: {}", id, title);
            return;
        }
        let file_path = trajectories_dir.join(format!("{}.json", id));
        let content = match fs::read_to_string(&file_path).await {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read trajectory for title update: {}", e);
                return;
            }
        };
        let mut data: TrajectoryData = match serde_json::from_str(&content) {
            Ok(d) => d,
            Err(e) => {
                warn!("Failed to parse trajectory for title update: {}", e);
                return;
            }
        };
        let already_generated = data
            .extra
            .get("isTitleGenerated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if already_generated {
            info!("Title already generated for {}, skipping", id);
            return;
        }
        let now = chrono::Utc::now().to_rfc3339();
        data.title = title.clone();
        data.updated_at = now.clone();
        data.extra
            .insert("isTitleGenerated".to_string(), serde_json::json!(true));
        if let Err(e) = atomic_write_json(&file_path, &data).await {
            warn!("Failed to write trajectory with generated title: {}", e);
            return;
        }
        info!("Updated trajectory {} with generated title: {}", id, title);
        let event = TrajectoryEvent {
            event_type: "updated".to_string(),
            id: id.clone(),
            updated_at: Some(now),
            title: Some(title.clone()),
        };
        if let Some(tx) = &gcx.read().await.trajectory_events_tx {
            let _ = tx.send(event);
        }
    });
}

fn spawn_task_name_generation_task(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: String,
    messages: Vec<serde_json::Value>,
) {
    tokio::spawn(async move {
        let task_meta = match crate::tasks::storage::load_task_meta(gcx.clone(), &task_id).await {
            Ok(meta) => meta,
            Err(e) => {
                warn!("Failed to load task meta for name generation: {}", e);
                return;
            }
        };

        if task_meta.is_name_generated {
            return;
        }

        if !is_placeholder_task_name(&task_meta.name) {
            return;
        }

        let generated_name = generate_title_llm(gcx.clone(), &messages).await;
        let name = match generated_name {
            Some(n) => n,
            None => match extract_first_user_message(&messages) {
                Some(first_msg) => {
                    let truncated: String = first_msg.chars().take(60).collect();
                    if truncated.len() < first_msg.len() {
                        format!("{}...", truncated.trim_end())
                    } else {
                        truncated
                    }
                }
                None => return,
            },
        };

        match crate::tasks::storage::update_task_name(gcx.clone(), &task_id, &name).await {
            Ok(_) => {
                info!("Updated task {} with generated name: {}", task_id, name);
            }
            Err(e) => {
                warn!("Failed to update task name: {}", e);
            }
        }
    });
}

fn trajectory_data_to_meta(data: &TrajectoryData) -> TrajectoryMeta {
    let task_meta_json = data.extra.get("task_meta");
    let task_id = task_meta_json
        .and_then(|v| v.get("task_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let task_role = task_meta_json
        .and_then(|v| v.get("role"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let agent_id = task_meta_json
        .and_then(|v| v.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let card_id = task_meta_json
        .and_then(|v| v.get("card_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let parent_id = data.extra.get("parent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let link_type = data.extra.get("link_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    TrajectoryMeta {
        id: data.id.clone(),
        title: data.title.clone(),
        created_at: data.created_at.clone(),
        updated_at: data.updated_at.clone(),
        model: data.model.clone(),
        mode: data.mode.clone(),
        message_count: data.messages.len(),
        parent_id,
        link_type,
        task_id,
        task_role,
        agent_id,
        card_id,
        session_state: None,
    }
}

pub async fn handle_v1_trajectories_list(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let trajectories_dir = get_trajectories_dir(gcx.clone())
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let mut result: Vec<TrajectoryMeta> = Vec::new();
    if trajectories_dir.exists() {
        let mut entries = fs::read_dir(&trajectories_dir)
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path).await {
                if let Ok(data) = serde_json::from_str::<TrajectoryData>(&content) {
                    result.push(trajectory_data_to_meta(&data));
                }
            }
        }
    }
    enrich_with_session_state(gcx, &mut result).await;
    result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&result).unwrap()))
        .unwrap())
}

pub async fn handle_v1_trajectories_all(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let mut result: Vec<TrajectoryMeta> = Vec::new();

    let trajectories_dir = get_trajectories_dir(gcx.clone())
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if trajectories_dir.exists() {
        let mut entries = fs::read_dir(&trajectories_dir)
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path).await {
                if let Ok(data) = serde_json::from_str::<TrajectoryData>(&content) {
                    result.push(trajectory_data_to_meta(&data));
                }
            }
        }
    }

    if let Ok(tasks_dir) = crate::tasks::storage::get_tasks_dir(gcx.clone()).await {
        if tasks_dir.exists() {
            if let Ok(mut task_entries) = fs::read_dir(&tasks_dir).await {
                while let Ok(Some(task_entry)) = task_entries.next_entry().await {
                    let task_dir = task_entry.path();
                    if !task_dir.is_dir() {
                        continue;
                    }
                    let task_id = task_dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();

                    for role in &["planner", "agents"] {
                        let role_dir = task_dir.join("trajectories").join(role);
                        if !role_dir.exists() {
                            continue;
                        }
                        let trajectories = collect_task_trajectories(&role_dir, &task_id, role, None).await;
                        result.extend(trajectories);
                    }
                }
            }
        }
    }

    enrich_with_session_state(gcx, &mut result).await;
    result.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&result).unwrap()))
        .unwrap())
}

async fn enrich_with_session_state(
    gcx: Arc<ARwLock<GlobalContext>>,
    trajectories: &mut Vec<TrajectoryMeta>,
) {
    let gcx_locked = gcx.read().await;
    let sessions = gcx_locked.chat_sessions.read().await;
    for traj in trajectories.iter_mut() {
        if let Some(session_arc) = sessions.get(&traj.id) {
            let session = session_arc.lock().await;
            traj.session_state = Some(session.runtime.state.to_string());
        }
    }
}

async fn collect_task_trajectories(
    dir: &PathBuf,
    task_id: &str,
    role: &str,
    agent_id: Option<&str>,
) -> Vec<TrajectoryMeta> {
    let mut result = Vec::new();

    let Ok(mut entries) = fs::read_dir(dir).await else {
        return result;
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();

        if path.is_dir() {
            let sub_agent_id = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let sub_trajectories = Box::pin(collect_task_trajectories(&path, task_id, role, Some(sub_agent_id))).await;
            result.extend(sub_trajectories);
            continue;
        }

        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        if let Ok(content) = fs::read_to_string(&path).await {
            if let Ok(data) = serde_json::from_str::<TrajectoryData>(&content) {
                let mut meta = trajectory_data_to_meta(&data);
                if meta.task_id.is_none() {
                    meta.task_id = Some(task_id.to_string());
                }
                if meta.task_role.is_none() {
                    meta.task_role = Some(role.to_string());
                }
                if meta.agent_id.is_none() && agent_id.is_some() {
                    meta.agent_id = agent_id.map(|s| s.to_string());
                }
                result.push(meta);
            }
        }
    }

    result
}

pub async fn handle_v1_trajectories_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&id)?;
    let file_path = find_trajectory_path(gcx, &id)
        .await
        .ok_or_else(|| ScratchError::new(
            StatusCode::NOT_FOUND,
            "Trajectory not found".to_string(),
        ))?;
    let content = fs::read_to_string(&file_path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(content))
        .unwrap())
}

pub async fn handle_v1_trajectories_save(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&id)?;
    let data: TrajectoryData = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;
    if data.id != id {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "ID mismatch".to_string(),
        ));
    }
    let trajectories_dir = get_trajectories_dir(gcx.clone())
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    fs::create_dir_all(&trajectories_dir)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let file_path = trajectories_dir.join(format!("{}.json", id));
    let is_new = !file_path.exists();
    let is_title_generated = data
        .extra
        .get("isTitleGenerated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let should_generate_title =
        is_placeholder_title(&data.title) && !is_title_generated && !data.messages.is_empty();
    atomic_write_json(&file_path, &data)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let event = TrajectoryEvent {
        event_type: if is_new {
            "created".to_string()
        } else {
            "updated".to_string()
        },
        id: id.clone(),
        updated_at: Some(data.updated_at.clone()),
        title: if is_new {
            Some(data.title.clone())
        } else {
            None
        },
    };
    if let Some(tx) = &gcx.read().await.trajectory_events_tx {
        let _ = tx.send(event);
    }
    if should_generate_title {
        spawn_title_generation_task(
            gcx.clone(),
            id.clone(),
            data.messages.clone(),
            trajectories_dir,
        );
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"status":"ok"}"#))
        .unwrap())
}

pub async fn handle_v1_trajectories_delete(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&id)?;
    let file_path = find_trajectory_path(gcx.clone(), &id)
        .await
        .ok_or_else(|| ScratchError::new(
            StatusCode::NOT_FOUND,
            "Trajectory not found".to_string(),
        ))?;
    fs::remove_file(&file_path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let event = TrajectoryEvent {
        event_type: "deleted".to_string(),
        id: id.clone(),
        updated_at: None,
        title: None,
    };
    if let Some(tx) = &gcx.read().await.trajectory_events_tx {
        let _ = tx.send(event);
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"status":"ok"}"#))
        .unwrap())
}

pub async fn handle_v1_trajectories_subscribe(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let rx = {
        let gcx_locked = gcx.read().await;
        match &gcx_locked.trajectory_events_tx {
            Some(tx) => tx.subscribe(),
            None => {
                return Err(ScratchError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Trajectory events not available".to_string(),
                ))
            }
        }
    };
    let stream = async_stream::stream! {
        let mut rx = rx;
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let json = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
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

    #[test]
    fn test_validate_trajectory_id_rejects_path_traversal() {
        assert!(validate_trajectory_id("../etc/passwd").is_err());
        assert!(validate_trajectory_id("..").is_err());
        assert!(validate_trajectory_id("a/../b").is_err());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_forward_slash() {
        assert!(validate_trajectory_id("a/b").is_err());
        assert!(validate_trajectory_id("/absolute").is_err());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_backslash() {
        assert!(validate_trajectory_id("a\\b").is_err());
        assert!(validate_trajectory_id("\\windows\\path").is_err());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_null_byte() {
        assert!(validate_trajectory_id("test\0id").is_err());
    }

    #[test]
    fn test_validate_trajectory_id_accepts_valid() {
        assert!(validate_trajectory_id("abc-123").is_ok());
        assert!(validate_trajectory_id("chat_456").is_ok());
        assert!(validate_trajectory_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
    }

    #[test]
    fn test_is_placeholder_title_new_chat() {
        assert!(is_placeholder_title("New Chat"));
        assert!(is_placeholder_title("new chat"));
        assert!(is_placeholder_title("NEW CHAT"));
        assert!(is_placeholder_title("  New Chat  "));
    }

    #[test]
    fn test_is_placeholder_title_untitled() {
        assert!(is_placeholder_title("untitled"));
        assert!(is_placeholder_title("Untitled"));
        assert!(is_placeholder_title("UNTITLED"));
    }

    #[test]
    fn test_is_placeholder_title_empty() {
        assert!(is_placeholder_title(""));
        assert!(is_placeholder_title("   "));
    }

    #[test]
    fn test_is_placeholder_title_real_titles() {
        assert!(!is_placeholder_title("Fix authentication bug"));
        assert!(!is_placeholder_title("Refactor database module"));
        assert!(!is_placeholder_title("New feature implementation"));
    }

    #[test]
    fn test_clean_generated_title_strips_quotes() {
        assert_eq!(clean_generated_title("\"Hello World\""), "Hello World");
        assert_eq!(clean_generated_title("'Hello World'"), "Hello World");
        assert_eq!(clean_generated_title("`Hello World`"), "Hello World");
    }

    #[test]
    fn test_clean_generated_title_strips_asterisks() {
        assert_eq!(clean_generated_title("*Bold Title*"), "Bold Title");
        assert_eq!(clean_generated_title("**Strong Title**"), "Strong Title");
    }

    #[test]
    fn test_clean_generated_title_collapses_whitespace() {
        assert_eq!(clean_generated_title("Hello   World"), "Hello World");
        assert_eq!(
            clean_generated_title("  Multiple   Spaces  "),
            "Multiple Spaces"
        );
    }

    #[test]
    fn test_clean_generated_title_removes_newlines() {
        assert_eq!(clean_generated_title("Hello\nWorld"), "Hello World");
        assert_eq!(
            clean_generated_title("Line1\nLine2\nLine3"),
            "Line1 Line2 Line3"
        );
    }

    #[test]
    fn test_clean_generated_title_truncates_long() {
        let long_title = "A".repeat(100);
        let result = clean_generated_title(&long_title);
        assert!(result.len() <= 60);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_clean_generated_title_preserves_short() {
        let short_title = "Short Title";
        let result = clean_generated_title(short_title);
        assert_eq!(result, "Short Title");
        assert!(!result.ends_with("..."));
    }

    #[test]
    fn test_extract_first_user_message_string_content() {
        let messages = vec![
            json!({"role": "system", "content": "You are helpful"}),
            json!({"role": "user", "content": "Hello there"}),
        ];
        let result = extract_first_user_message(&messages);
        assert_eq!(result, Some("Hello there".to_string()));
    }

    #[test]
    fn test_extract_first_user_message_array_content_text() {
        let messages =
            vec![json!({"role": "user", "content": [{"type": "text", "text": "Array text"}]})];
        let result = extract_first_user_message(&messages);
        assert_eq!(result, Some("Array text".to_string()));
    }

    #[test]
    fn test_extract_first_user_message_array_content_m_content() {
        let messages = vec![
            json!({"role": "user", "content": [{"m_type": "text", "m_content": "M content"}]}),
        ];
        let result = extract_first_user_message(&messages);
        assert_eq!(result, Some("M content".to_string()));
    }

    #[test]
    fn test_extract_first_user_message_skips_empty() {
        let messages = vec![
            json!({"role": "user", "content": "   "}),
            json!({"role": "user", "content": "Second message"}),
        ];
        let result = extract_first_user_message(&messages);
        assert_eq!(result, Some("Second message".to_string()));
    }

    #[test]
    fn test_extract_first_user_message_truncates() {
        let long_message = "A".repeat(300);
        let messages = vec![json!({"role": "user", "content": long_message})];
        let result = extract_first_user_message(&messages);
        assert!(result.is_some());
        assert!(result.unwrap().len() <= 200);
    }

    #[test]
    fn test_extract_first_user_message_no_user() {
        let messages = vec![
            json!({"role": "system", "content": "System prompt"}),
            json!({"role": "assistant", "content": "Hello"}),
        ];
        let result = extract_first_user_message(&messages);
        assert!(result.is_none());
    }

    #[test]
    fn test_build_title_generation_context_skips_tool_messages() {
        let messages = vec![
            json!({"role": "user", "content": "User message"}),
            json!({"role": "tool", "content": "Tool result"}),
            json!({"role": "assistant", "content": "Response"}),
        ];
        let context = build_title_generation_context(&messages);
        assert!(context.contains("User message"));
        assert!(context.contains("Response"));
        assert!(!context.contains("Tool result"));
    }

    #[test]
    fn test_build_title_generation_context_skips_context_file() {
        let messages = vec![
            json!({"role": "user", "content": "Question"}),
            json!({"role": "context_file", "content": "File contents"}),
        ];
        let context = build_title_generation_context(&messages);
        assert!(context.contains("Question"));
        assert!(!context.contains("File contents"));
    }

    #[test]
    fn test_build_title_generation_context_limits_messages() {
        let messages: Vec<_> = (0..10)
            .map(|i| json!({"role": "user", "content": format!("Message {}", i)}))
            .collect();
        let context = build_title_generation_context(&messages);
        assert!(context.contains("Message 0"));
        assert!(context.contains("Message 5"));
        assert!(!context.contains("Message 9"));
    }

    #[test]
    fn test_build_title_generation_context_truncates_long_messages() {
        let long_content = "A".repeat(1000);
        let messages = vec![json!({"role": "user", "content": long_content})];
        let context = build_title_generation_context(&messages);
        assert!(context.len() < 600);
    }

    #[test]
    fn test_fix_tool_call_indexes_sets_missing() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        let mut messages = vec![ChatMessage {
            role: "assistant".to_string(),
            tool_calls: Some(vec![
                ChatToolCall {
                    id: "call_1".to_string(),
                    index: None,
                    function: ChatToolFunction {
                        name: "test".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                },
                ChatToolCall {
                    id: "call_2".to_string(),
                    index: None,
                    function: ChatToolFunction {
                        name: "test2".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                },
            ]),
            ..Default::default()
        }];
        fix_tool_call_indexes(&mut messages);
        let tool_calls = messages[0].tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls[0].index, Some(0));
        assert_eq!(tool_calls[1].index, Some(1));
    }

    #[test]
    fn test_fix_tool_call_indexes_preserves_existing() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        let mut messages = vec![ChatMessage {
            role: "assistant".to_string(),
            tool_calls: Some(vec![ChatToolCall {
                id: "call_1".to_string(),
                index: Some(5),
                function: ChatToolFunction {
                    name: "test".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
            }]),
            ..Default::default()
        }];
        fix_tool_call_indexes(&mut messages);
        let tool_calls = messages[0].tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls[0].index, Some(5));
    }

    #[test]
    fn test_trajectory_event_serialization() {
        let event = TrajectoryEvent {
            event_type: "updated".to_string(),
            id: "chat-123".to_string(),
            updated_at: Some("2024-01-01T00:00:00Z".to_string()),
            title: Some("Test Title".to_string()),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "updated");
        assert_eq!(json["id"], "chat-123");
    }

    #[test]
    fn test_trajectory_snapshot_from_session_captures_fields() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        use tokio::sync::{broadcast, Notify};
        use std::collections::VecDeque;

        let (tx, _rx) = broadcast::channel(16);
        let session = ChatSession {
            chat_id: "test-123".to_string(),
            thread: ThreadParams {
                id: "test-123".to_string(),
                title: "Test Thread".to_string(),
                model: "gpt-4".to_string(),
                mode: "AGENT".to_string(),
                tool_use: "agent".to_string(),
                boost_reasoning: true,
                context_tokens_cap: Some(8000),
                include_project_info: false,
                checkpoints_enabled: true,
                is_title_generated: true,
                automatic_patch: false,
                task_meta: None,
                parent_id: Some("parent-chat-id".to_string()),
                link_type: Some("subagent".to_string()),
            },
            messages: vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
            runtime: super::super::types::RuntimeState::default(),
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            event_seq: 0,
            event_tx: tx,
            recent_request_ids: VecDeque::new(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            last_activity: Instant::now(),
            trajectory_dirty: false,
            trajectory_version: 5,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            closed: false,
            external_reload_pending: false,
            last_prompt_messages: Vec::new(),
        };

        let snapshot = TrajectorySnapshot::from_session(&session);
        assert_eq!(snapshot.chat_id, "test-123");
        assert_eq!(snapshot.title, "Test Thread");
        assert_eq!(snapshot.model, "gpt-4");
        assert_eq!(snapshot.mode, "AGENT");
        assert!(snapshot.boost_reasoning);
        assert_eq!(snapshot.context_tokens_cap, Some(8000));
        assert!(!snapshot.include_project_info);
        assert!(snapshot.is_title_generated);
        assert_eq!(snapshot.version, 5);
        assert_eq!(snapshot.messages.len(), 1);
    }
}
