use std::path::{PathBuf, Path};
use std::sync::{Arc, Weak};
use std::time::Instant;
use axum::extract::Path as AxumPath;
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
use crate::yaml_configs::customization_registry::get_subagent_config;
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::WorktreeMeta;

pub async fn atomic_write_file(tmp_path: &Path, dest_path: &Path) -> Result<(), String> {
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

use super::types::{ThreadParams, SessionState, ChatSession};
use super::config::timeouts;
use super::SessionsMap;

const TITLE_GENERATION_SUBAGENT_ID: &str = "title_generation";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrajectoryEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_title_generated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_chat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines_added: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines_removed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks_total: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks_done: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks_failed: Option<i32>,
    pub total_prompt_tokens: Option<u64>,
    pub total_completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub total_cache_read_tokens: Option<u64>,
    pub total_cache_creation_tokens: Option<u64>,
    pub total_cost_usd: Option<f64>,
}

pub async fn get_session_state_for_chat(
    sessions: &SessionsMap,
    chat_id: &str,
) -> (String, Option<String>) {
    let session_arc = sessions.read().await.get(chat_id).cloned();
    match session_arc {
        Some(arc) => {
            let session = arc.lock().await;
            (
                session.runtime.state.to_string(),
                session.runtime.error.clone(),
            )
        }
        None => (SessionState::Idle.to_string(), None),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeMeta>,
    #[serde(default)]
    pub total_lines_added: i64,
    #[serde(default)]
    pub total_lines_removed: i64,
    #[serde(default)]
    pub tasks_total: i32,
    #[serde(default)]
    pub tasks_done: i32,
    #[serde(default)]
    pub tasks_failed: i32,
    #[serde(default)]
    pub total_prompt_tokens: u64,
    #[serde(default)]
    pub total_completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub total_cache_read_tokens: u64,
    #[serde(default)]
    pub total_cache_creation_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
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
    pub updated_at: String,
    pub auto_approve_editing_tools_present: bool,
    pub auto_approve_dangerous_commands_present: bool,
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
    pub auto_approve_editing_tools: bool,
    pub auto_approve_dangerous_commands: bool,
    pub version: u64,
    pub task_meta: Option<super::types::TaskMeta>,
    pub worktree: Option<WorktreeMeta>,
    pub parent_id: Option<String>,
    pub link_type: Option<String>,
    pub root_chat_id: Option<String>,
    pub reasoning_effort: Option<String>,
    pub thinking_budget: Option<usize>,
    pub temperature: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub max_tokens: Option<usize>,
    pub parallel_tool_calls: Option<bool>,

    pub previous_response_id: Option<String>,
    pub active_skill: Option<String>,
    pub auto_enrichment_enabled: Option<bool>,
    pub buddy_meta: Option<crate::buddy::types::BuddyThreadMeta>,
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
            boost_reasoning: session.thread.boost_reasoning.unwrap_or(false),
            checkpoints_enabled: session.thread.checkpoints_enabled,
            context_tokens_cap: session.thread.context_tokens_cap,
            include_project_info: session.thread.include_project_info,
            is_title_generated: session.thread.is_title_generated,
            auto_approve_editing_tools: session.thread.auto_approve_editing_tools,
            auto_approve_dangerous_commands: session.thread.auto_approve_dangerous_commands,
            version: session.trajectory_version,
            task_meta: session.thread.task_meta.clone(),
            worktree: session.thread.worktree.clone(),
            parent_id: session.thread.parent_id.clone(),
            link_type: session.thread.link_type.clone(),
            root_chat_id: session.thread.root_chat_id.clone(),
            reasoning_effort: session.thread.reasoning_effort.clone(),
            thinking_budget: session.thread.thinking_budget,
            temperature: session.thread.temperature,
            frequency_penalty: session.thread.frequency_penalty,
            max_tokens: session.thread.max_tokens,
            parallel_tool_calls: session.thread.parallel_tool_calls,
            previous_response_id: session.thread.previous_response_id.clone(),
            active_skill: session.thread.active_skill.clone(),
            auto_enrichment_enabled: session.thread.auto_enrichment_enabled,
            buddy_meta: session.thread.buddy_meta.clone(),
        }
    }
}

pub async fn apply_mode_defaults_to_thread(
    gcx: Arc<ARwLock<GlobalContext>>,
    thread: &mut ThreadParams,
    auto_approve_editing_present: bool,
    auto_approve_dangerous_present: bool,
) {
    if auto_approve_editing_present && auto_approve_dangerous_present {
        return;
    }
    if let Some(mode_config) = crate::yaml_configs::customization_registry::get_mode_config(
        gcx.clone(),
        &thread.mode,
        None,
    )
    .await
    {
        let defaults = &mode_config.thread_defaults;
        if !auto_approve_editing_present {
            if let Some(v) = defaults.auto_approve_editing_tools {
                thread.auto_approve_editing_tools = v;
            }
        }
        if !auto_approve_dangerous_present {
            if let Some(v) = defaults.auto_approve_dangerous_commands {
                thread.auto_approve_dangerous_commands = v;
            }
        }
    }
}

pub async fn get_trajectories_dir(gcx: Arc<ARwLock<GlobalContext>>) -> Result<PathBuf, String> {
    let project_dirs = get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    Ok(workspace_root.join(".refact").join("trajectories"))
}

pub async fn get_global_trajectories_dir(gcx: Arc<ARwLock<GlobalContext>>) -> PathBuf {
    let config_dir = gcx.read().await.config_dir.clone();
    config_dir.join("trajectories")
}

pub async fn get_all_trajectories_dirs(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .map(|p| p.join(".refact").join("trajectories"))
        .filter(|p| p.exists())
        .collect();

    let global_dir = get_global_trajectories_dir(gcx).await;
    if global_dir.exists() {
        dirs.push(global_dir);
    }

    dirs
}

async fn get_all_trajectories_dirs_from_weak(
    gcx_weak: &Weak<ARwLock<GlobalContext>>,
) -> Vec<PathBuf> {
    match gcx_weak.upgrade() {
        Some(gcx) => get_all_trajectories_dirs(gcx).await,
        None => vec![],
    }
}

pub async fn get_buddy_conversations_dir(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<PathBuf, String> {
    let project_dirs = get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    Ok(workspace_root.join(".refact/buddy/chats/conversations"))
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

pub async fn find_trajectory_path(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
) -> Option<PathBuf> {
    if let Ok(buddy_dir) = get_buddy_conversations_dir(gcx.clone()).await {
        let buddy_path = buddy_dir.join(format!("{}.json", chat_id));
        if buddy_path.exists() {
            return Some(buddy_path);
        }
    }

    let traj_dirs = get_all_trajectories_dirs(gcx.clone()).await;
    if let Some(path) = traj_dirs
        .iter()
        .map(|dir| dir.join(format!("{}.json", chat_id)))
        .find(|p| p.exists())
    {
        return Some(path);
    }

    let tasks_dirs = crate::tasks::storage::get_all_tasks_dirs(gcx.clone()).await;
    for tasks_dir in tasks_dirs {
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

fn parse_worktree_meta(value: &serde_json::Value) -> Option<WorktreeMeta> {
    if value.is_null() {
        return None;
    }
    serde_json::from_value(value.clone()).ok()
}

fn trajectory_worktree_from_extra(
    extra: &serde_json::Map<String, serde_json::Value>,
) -> Option<WorktreeMeta> {
    extra.get("worktree").and_then(parse_worktree_meta)
}

fn sanitize_worktree_extra(
    extra: &mut serde_json::Map<String, serde_json::Value>,
) -> Option<WorktreeMeta> {
    let Some(value) = extra.get("worktree") else {
        return None;
    };
    if value.is_null() {
        extra.remove("worktree");
        return None;
    }
    match serde_json::from_value::<WorktreeMeta>(value.clone()) {
        Ok(worktree) => Some(worktree),
        Err(e) => {
            warn!("Ignoring invalid trajectory worktree metadata: {}", e);
            extra.remove("worktree");
            None
        }
    }
}

async fn worktree_service_from_gcx(
    gcx: Arc<ARwLock<GlobalContext>>,
    requested_source_root: Option<&Path>,
) -> Result<WorktreeService, String> {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let project_dirs = get_project_dirs(gcx).await;
    if project_dirs.is_empty() {
        return Err("No project root available".to_string());
    }
    let source_root = match requested_source_root {
        Some(requested) => {
            let requested = std::fs::canonicalize(requested).map_err(|e| {
                format!(
                    "Failed to resolve worktree source root '{}': {}",
                    requested.display(),
                    e
                )
            })?;
            let requested = dunce::simplified(&requested).to_path_buf();
            let matches = project_dirs.iter().any(|dir| {
                std::fs::canonicalize(dir)
                    .map(|canonical| dunce::simplified(&canonical).to_path_buf() == requested)
                    .unwrap_or(false)
            });
            if !matches {
                return Err("Worktree source root is not a current workspace directory".to_string());
            }
            requested
        }
        None => project_dirs[0].clone(),
    };
    WorktreeService::new(cache_dir, source_root)
}

async fn validate_loaded_worktree_strict(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
    worktree: WorktreeMeta,
) -> Option<WorktreeMeta> {
    let service =
        match worktree_service_from_gcx(gcx.clone(), Some(&worktree.source_workspace_root)).await {
            Ok(service) => service,
            Err(e) => {
                warn!(
                    "Ignoring trajectory worktree metadata for chat {}: {}",
                    chat_id, e
                );
                return None;
            }
        };
    match service.validate_worktree_meta_strict(&worktree).await {
        Ok(validated) => Some(validated),
        Err(e) => {
            warn!(
                "Ignoring untrusted trajectory worktree metadata for chat {}: {}",
                chat_id, e
            );
            None
        }
    }
}

async fn validate_loaded_legacy_task_agent_worktree(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
    worktree: WorktreeMeta,
) -> Option<WorktreeMeta> {
    let service =
        match worktree_service_from_gcx(gcx.clone(), Some(&worktree.source_workspace_root)).await {
            Ok(service) => service,
            Err(e) => {
                warn!(
                    "Ignoring legacy task-agent worktree metadata for chat {}: {}",
                    chat_id, e
                );
                return None;
            }
        };
    match service
        .validate_legacy_task_agent_worktree_meta(&worktree)
        .await
    {
        Ok(validated) => Some(validated),
        Err(e) => {
            warn!(
                "Ignoring untrusted legacy task-agent worktree metadata for chat {}: {}",
                chat_id, e
            );
            None
        }
    }
}

async fn synthesize_legacy_task_agent_worktree(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
    task_meta: Option<&super::types::TaskMeta>,
) -> Option<WorktreeMeta> {
    let task_meta = task_meta?;
    if task_meta.role != "agents" {
        return None;
    }

    let source_workspace_root = get_project_dirs(gcx.clone()).await.into_iter().next();
    let task_record = crate::tasks::storage::load_task_meta(gcx.clone(), &task_meta.task_id)
        .await
        .ok();
    let board = crate::tasks::storage::load_board(gcx, &task_meta.task_id)
        .await
        .ok()?;

    let card = if let Some(card_id) = task_meta.card_id.as_deref() {
        board.cards.iter().find(|card| card.id == card_id)?
    } else {
        board
            .cards
            .iter()
            .find(|card| card.agent_chat_id.as_deref() == Some(chat_id))?
    };
    if card.agent_chat_id.as_deref() != Some(chat_id) {
        return None;
    }
    if let Some(agent_id) = task_meta.agent_id.as_deref() {
        if card.assignee.as_deref() != Some(agent_id) {
            return None;
        }
    }

    let root = PathBuf::from(card.agent_worktree.as_ref()?);
    let source_workspace_root = source_workspace_root.unwrap_or_else(|| root.clone());
    let id = card
        .agent_worktree_name
        .clone()
        .or_else(|| {
            root.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_string())
        })
        .unwrap_or_else(|| format!("{}-{}", task_meta.task_id, card.id));

    Some(WorktreeMeta {
        id,
        kind: "task_agent".to_string(),
        root,
        source_workspace_root: source_workspace_root.clone(),
        repo_root: source_workspace_root,
        branch: card.agent_branch.clone(),
        base_branch: task_record
            .as_ref()
            .and_then(|meta| meta.base_branch.clone()),
        base_commit: task_record
            .as_ref()
            .and_then(|meta| meta.base_commit.clone()),
        task_id: Some(task_meta.task_id.clone()),
        card_id: Some(card.id.clone()),
        agent_id: task_meta.agent_id.clone().or_else(|| card.assignee.clone()),
        enforce: true,
    })
}

pub async fn load_trajectory_for_chat(
    gcx: Arc<ARwLock<GlobalContext>>,
    chat_id: &str,
) -> Option<LoadedTrajectory> {
    let traj_path = find_trajectory_path(gcx.clone(), chat_id).await?;
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

        if let Some(tool_calls) = &msg.tool_calls {
            let filtered: Vec<_> = tool_calls
                .iter()
                .filter(|tc| !tc.function.name.is_empty())
                .cloned()
                .collect();

            if filtered.len() != tool_calls.len() {
                tracing::warn!(
                    "Filtered out {} tool call(s) with empty names from message {}",
                    tool_calls.len() - filtered.len(),
                    msg.message_id
                );
            }

            msg.tool_calls = if filtered.is_empty() {
                None
            } else {
                Some(filtered)
            };
        }
    }

    let task_meta: Option<super::types::TaskMeta> = t
        .get("task_meta")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let worktree = if let Some(candidate) = t.get("worktree").and_then(parse_worktree_meta) {
        validate_loaded_worktree_strict(gcx.clone(), chat_id, candidate).await
    } else if let Some(candidate) =
        synthesize_legacy_task_agent_worktree(gcx.clone(), chat_id, task_meta.as_ref()).await
    {
        validate_loaded_legacy_task_agent_worktree(gcx.clone(), chat_id, candidate).await
    } else {
        None
    };

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
        mode: crate::yaml_configs::customization_registry::map_legacy_mode_to_id(
            t.get("mode").and_then(|v| v.as_str()).unwrap_or("agent"),
        )
        .to_string(),
        tool_use: t
            .get("tool_use")
            .and_then(|v| v.as_str())
            .unwrap_or("agent")
            .to_string(),
        boost_reasoning: t.get("boost_reasoning").and_then(|v| v.as_bool()),
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
        auto_approve_editing_tools: t
            .get("auto_approve_editing_tools")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        auto_approve_dangerous_commands: t
            .get("auto_approve_dangerous_commands")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        task_meta,
        worktree,
        parent_id: t
            .get("parent_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        link_type: t
            .get("link_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        root_chat_id: t
            .get("root_chat_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        reasoning_effort: t
            .get("reasoning_effort")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        thinking_budget: t
            .get("thinking_budget")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize),
        temperature: t
            .get("temperature")
            .and_then(|v| v.as_f64())
            .map(|n| n as f32),
        frequency_penalty: t
            .get("frequency_penalty")
            .and_then(|v| v.as_f64())
            .map(|n| n as f32),
        max_tokens: t
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize),
        parallel_tool_calls: t.get("parallel_tool_calls").and_then(|v| v.as_bool()),

        previous_response_id: t
            .get("previous_response_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        browser_meta: t
            .get("browser_meta")
            .and_then(|v| serde_json::from_value(v.clone()).ok()),

        active_skill: t
            .get("active_skill")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        auto_enrichment_enabled: t.get("auto_enrichment_enabled").and_then(|v| v.as_bool()),

        buddy_meta: t
            .get("buddy_meta")
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
    };

    let auto_approve_editing_tools_present = t
        .get("auto_approve_editing_tools")
        .and_then(|v| v.as_bool())
        .is_some();
    let auto_approve_dangerous_commands_present = t
        .get("auto_approve_dangerous_commands")
        .and_then(|v| v.as_bool())
        .is_some();

    let created_at = t
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap_or(&chrono::Utc::now().to_rfc3339())
        .to_string();

    let updated_at = t
        .get("updated_at")
        .and_then(|v| v.as_str())
        .unwrap_or(&created_at)
        .to_string();

    Some(LoadedTrajectory {
        messages,
        thread,
        created_at,
        updated_at,
        auto_approve_editing_tools_present,
        auto_approve_dangerous_commands_present,
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
        server_content_blocks: vec![],
        extra: serde_json::Map::new(),
        output_filter: None,
    };

    let task_meta = super::types::TaskMeta {
        task_id: task_id.to_string(),
        role: "planner".to_string(),
        agent_id: None,
        card_id: None,
    };

    let snapshot = TrajectorySnapshot {
        chat_id: chat_id.to_string(),
        title: String::new(),
        model: String::new(),
        mode: "task_planner".to_string(),
        tool_use: "agent".to_string(),
        messages: vec![greeting_msg],
        created_at: chrono::Utc::now().to_rfc3339(),
        boost_reasoning: false,
        checkpoints_enabled: true,
        context_tokens_cap: None,
        include_project_info: true,
        is_title_generated: false,
        auto_approve_editing_tools: true,
        auto_approve_dangerous_commands: true,
        auto_enrichment_enabled: Some(false),
        task_meta: Some(task_meta),
        worktree: None,
        version: 1,
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
        buddy_meta: None,
    };

    save_trajectory_snapshot(gcx, snapshot).await
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
        boost_reasoning: thread.boost_reasoning.unwrap_or(false),
        checkpoints_enabled: thread.checkpoints_enabled,
        context_tokens_cap: thread.context_tokens_cap,
        include_project_info: thread.include_project_info,
        is_title_generated: thread.is_title_generated,
        auto_approve_editing_tools: thread.auto_approve_editing_tools,
        auto_approve_dangerous_commands: thread.auto_approve_dangerous_commands,
        version: 1,
        task_meta: thread.task_meta.clone(),
        worktree: thread.worktree.clone(),
        parent_id: thread.parent_id.clone(),
        link_type: thread.link_type.clone(),
        root_chat_id: thread.root_chat_id.clone(),
        reasoning_effort: thread.reasoning_effort.clone(),
        thinking_budget: thread.thinking_budget,
        temperature: thread.temperature,
        frequency_penalty: thread.frequency_penalty,
        max_tokens: thread.max_tokens,
        parallel_tool_calls: thread.parallel_tool_calls,
        previous_response_id: thread.previous_response_id.clone(),
        active_skill: thread.active_skill.clone(),
        auto_enrichment_enabled: thread.auto_enrichment_enabled,
        buddy_meta: thread.buddy_meta.clone(),
    };
    if let Err(e) = save_trajectory_snapshot(gcx, snapshot).await {
        warn!("Failed to save trajectory: {}", e);
    }
}

pub async fn save_trajectory_snapshot(
    gcx: Arc<ARwLock<GlobalContext>>,
    snapshot: TrajectorySnapshot,
) -> Result<(), String> {
    if snapshot.messages.is_empty() && snapshot.task_meta.is_none() && snapshot.buddy_meta.is_none()
    {
        return Ok(());
    }

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
        "messages": messages_json.clone(),
        "created_at": snapshot.created_at,
        "boost_reasoning": snapshot.boost_reasoning,
        "checkpoints_enabled": snapshot.checkpoints_enabled,
        "context_tokens_cap": snapshot.context_tokens_cap,
        "include_project_info": snapshot.include_project_info,
        "isTitleGenerated": snapshot.is_title_generated,
        "auto_approve_editing_tools": snapshot.auto_approve_editing_tools,
        "auto_approve_dangerous_commands": snapshot.auto_approve_dangerous_commands,
    });

    if let Some(ref effort) = snapshot.reasoning_effort {
        trajectory["reasoning_effort"] = serde_json::Value::String(effort.clone());
    }
    if let Some(budget) = snapshot.thinking_budget {
        trajectory["thinking_budget"] = json!(budget);
    }
    if let Some(temp) = snapshot.temperature {
        trajectory["temperature"] = json!(temp);
    }
    if let Some(freq) = snapshot.frequency_penalty {
        trajectory["frequency_penalty"] = json!(freq);
    }
    if let Some(max_t) = snapshot.max_tokens {
        trajectory["max_tokens"] = json!(max_t);
    }
    if let Some(ref prev) = snapshot.previous_response_id {
        trajectory["previous_response_id"] = serde_json::Value::String(prev.clone());
    }
    if let Some(parallel) = snapshot.parallel_tool_calls {
        trajectory["parallel_tool_calls"] = json!(parallel);
    }
    if let Some(ref skill) = snapshot.active_skill {
        trajectory["active_skill"] = serde_json::Value::String(skill.clone());
    }
    if let Some(auto_enrich) = snapshot.auto_enrichment_enabled {
        trajectory["auto_enrichment_enabled"] = json!(auto_enrich);
    }
    if let Some(ref buddy_meta) = snapshot.buddy_meta {
        trajectory["buddy_meta"] = serde_json::to_value(buddy_meta).unwrap_or_default();
    }
    if let Some(ref worktree) = snapshot.worktree {
        trajectory["worktree"] = serde_json::to_value(worktree).unwrap_or_default();
    }

    if let Some(ref parent_id) = snapshot.parent_id {
        trajectory["parent_id"] = serde_json::Value::String(parent_id.clone());
    }
    if let Some(ref link_type) = snapshot.link_type {
        trajectory["link_type"] = serde_json::Value::String(link_type.clone());
    }

    let effective_root = snapshot
        .root_chat_id
        .clone()
        .unwrap_or_else(|| snapshot.chat_id.clone());
    trajectory["root_chat_id"] = serde_json::Value::String(effective_root);

    if let Some(ref task_meta) = snapshot.task_meta {
        trajectory["task_meta"] = serde_json::to_value(task_meta).unwrap_or_default();
    }

    let file_path = if let Some(ref task_meta) = snapshot.task_meta {
        let task_dir =
            crate::tasks::storage::find_task_dir(gcx.clone(), &task_meta.task_id).await?;
        let traj_dir = crate::tasks::storage::get_task_trajectory_dir(
            &task_dir,
            &task_meta.role,
            task_meta.agent_id.as_deref(),
        );
        tokio::fs::create_dir_all(&traj_dir)
            .await
            .map_err(|e| format!("Failed to create task trajectories dir: {}", e))?;
        traj_dir.join(format!("{}.json", snapshot.chat_id))
    } else if snapshot.buddy_meta.is_some() {
        let buddy_dir = get_buddy_conversations_dir(gcx.clone()).await?;
        tokio::fs::create_dir_all(&buddy_dir)
            .await
            .map_err(|e| format!("Failed to create buddy conversations dir: {}", e))?;
        buddy_dir.join(format!("{}.json", snapshot.chat_id))
    } else {
        let trajectories_dir = get_trajectories_dir(gcx.clone()).await?;
        tokio::fs::create_dir_all(&trajectories_dir)
            .await
            .map_err(|e| format!("Failed to create trajectories dir: {}", e))?;
        trajectories_dir.join(format!("{}.json", snapshot.chat_id))
    };

    let now = chrono::Utc::now().to_rfc3339();
    let updated_at = match tokio::fs::read_to_string(&file_path).await {
        Ok(existing_content) => {
            match serde_json::from_str::<serde_json::Value>(&existing_content) {
                Ok(existing) => {
                    let existing_messages = existing.get("messages");
                    let same_messages =
                        existing_messages == Some(&serde_json::Value::Array(messages_json.clone()));
                    if same_messages {
                        existing
                            .get("updated_at")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&now)
                            .to_string()
                    } else {
                        now.clone()
                    }
                }
                Err(_) => now.clone(),
            }
        }
        Err(_) => now.clone(),
    };
    trajectory["updated_at"] = serde_json::Value::String(updated_at.clone());

    let tmp_path = file_path.with_extension("json.tmp");
    let json_str = serde_json::to_string_pretty(&trajectory)
        .map_err(|e| format!("Failed to serialize trajectory: {}", e))?;
    tokio::fs::write(&tmp_path, &json_str)
        .await
        .map_err(|e| format!("Failed to write trajectory: {}", e))?;
    atomic_write_file(&tmp_path, &file_path).await?;

    info!(
        "Saved trajectory for chat {} ({} messages) to {:?}",
        snapshot.chat_id,
        snapshot.messages.len(),
        file_path
    );

    let vec_db = gcx.read().await.vec_db.clone();
    if let Some(vecdb) = vec_db.lock().await.as_ref() {
        vecdb
            .vectorizer_enqueue_files(&vec![file_path.to_string_lossy().to_string()], false)
            .await;
    }

    if snapshot.task_meta.is_none() && snapshot.buddy_meta.is_none() {
        let effective_root = snapshot
            .root_chat_id
            .clone()
            .unwrap_or_else(|| snapshot.chat_id.clone());
        let sessions = gcx.read().await.chat_sessions.clone();
        let (session_state, session_error) =
            get_session_state_for_chat(&sessions, &snapshot.chat_id).await;
        let (total_lines_added, total_lines_removed) =
            calculate_line_changes_from_chat_messages(&snapshot.messages);
        let (tasks_total, tasks_done, tasks_failed) =
            calculate_task_progress_from_chat_messages(&snapshot.messages);
        let token_totals = calculate_token_totals_from_chat_messages(&snapshot.messages);
        if let Some(tx) = &gcx.read().await.trajectory_events_tx {
            let event = TrajectoryEvent {
                event_type: "updated".to_string(),
                id: snapshot.chat_id.clone(),
                updated_at: Some(updated_at),
                title: Some(snapshot.title.clone()),
                is_title_generated: Some(snapshot.is_title_generated),
                session_state: Some(session_state),
                error: session_error,
                message_count: Some(snapshot.messages.len()),
                parent_id: snapshot.parent_id.clone(),
                link_type: snapshot.link_type.clone(),
                root_chat_id: Some(effective_root),
                model: Some(snapshot.model.clone()),
                mode: Some(snapshot.mode.clone()),
                worktree: snapshot.worktree.clone(),
                total_lines_added: Some(total_lines_added),
                total_lines_removed: Some(total_lines_removed),
                tasks_total: Some(tasks_total),
                tasks_done: Some(tasks_done),
                tasks_failed: Some(tasks_failed),
                total_prompt_tokens: Some(token_totals.prompt_tokens),
                total_completion_tokens: Some(token_totals.completion_tokens),
                total_tokens: Some(token_totals.total_tokens),
                total_cache_read_tokens: Some(token_totals.cache_read_tokens),
                total_cache_creation_tokens: Some(token_totals.cache_creation_tokens),
                total_cost_usd: token_totals.cost_usd,
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
    if let Some(mut loaded) = load_trajectory_for_chat(gcx.clone(), &chat_id).await {
        apply_mode_defaults_to_thread(
            gcx.clone(),
            &mut loaded.thread,
            loaded.auto_approve_editing_tools_present,
            loaded.auto_approve_dangerous_commands_present,
        )
        .await;
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
    let sessions = gcx.read().await.chat_sessions.clone();
    if is_remove {
        if let Some(tx) = &gcx.read().await.trajectory_events_tx {
            let _ = tx.send(TrajectoryEvent {
                event_type: "deleted".to_string(),
                id: chat_id.to_string(),
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
    } else {
        let loaded = load_trajectory_for_chat(gcx.clone(), chat_id).await;
        let (
            updated_at,
            title,
            is_title_generated,
            message_count,
            parent_id,
            link_type,
            root_chat_id,
            model,
            mode,
            worktree,
            total_lines_added,
            total_lines_removed,
            tasks_total,
            tasks_done,
            tasks_failed,
            token_totals,
        ) = if let Some(t) = loaded {
            let effective_root = t
                .thread
                .root_chat_id
                .clone()
                .unwrap_or_else(|| t.thread.id.clone());
            let (lines_added, lines_removed) =
                calculate_line_changes_from_chat_messages(&t.messages);
            let (t_total, t_done, t_failed) =
                calculate_task_progress_from_chat_messages(&t.messages);
            let tok = calculate_token_totals_from_chat_messages(&t.messages);
            (
                Some(t.updated_at),
                Some(t.thread.title),
                Some(t.thread.is_title_generated),
                Some(t.messages.len()),
                t.thread.parent_id,
                t.thread.link_type,
                Some(effective_root),
                Some(t.thread.model),
                Some(t.thread.mode),
                t.thread.worktree,
                Some(lines_added),
                Some(lines_removed),
                Some(t_total),
                Some(t_done),
                Some(t_failed),
                Some(tok),
            )
        } else {
            (
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            )
        };
        let (session_state, session_error) = get_session_state_for_chat(&sessions, chat_id).await;
        if let Some(tx) = &gcx.read().await.trajectory_events_tx {
            let _ = tx.send(TrajectoryEvent {
                event_type: "updated".to_string(),
                id: chat_id.to_string(),
                updated_at,
                title,
                is_title_generated,
                session_state: Some(session_state),
                error: session_error,
                message_count,
                parent_id,
                link_type,
                root_chat_id,
                model,
                mode,
                worktree,
                total_lines_added,
                total_lines_removed,
                tasks_total,
                tasks_done,
                tasks_failed,
                total_prompt_tokens: token_totals.as_ref().map(|t| t.prompt_tokens),
                total_completion_tokens: token_totals.as_ref().map(|t| t.completion_tokens),
                total_tokens: token_totals.as_ref().map(|t| t.total_tokens),
                total_cache_read_tokens: token_totals.as_ref().map(|t| t.cache_read_tokens),
                total_cache_creation_tokens: token_totals.as_ref().map(|t| t.cache_creation_tokens),
                total_cost_usd: token_totals.and_then(|t| t.cost_usd),
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

    if let Some(mut loaded) = load_trajectory_for_chat(gcx.clone(), chat_id).await {
        apply_mode_defaults_to_thread(
            gcx.clone(),
            &mut loaded.thread,
            loaded.auto_approve_editing_tools_present,
            loaded.auto_approve_dangerous_commands_present,
        )
        .await;
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
        let trajectories_dirs = get_all_trajectories_dirs_from_weak(&gcx_weak).await;
        if trajectories_dirs.is_empty() {
            warn!("No trajectories directories found, trajectory watcher not started");
            return;
        }

        for dir in &trajectories_dirs {
            if let Err(e) = tokio::fs::create_dir_all(dir).await {
                warn!(
                    "Failed to create trajectories dir {:?} for watcher: {}",
                    dir, e
                );
            }
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
            for dir in &trajectories_dirs {
                if let Err(e) = w.watch(dir, RecursiveMode::NonRecursive) {
                    warn!("Failed to watch trajectories dir {:?}: {}", dir, e);
                }
            }
        }
        info!(
            "Trajectory watcher started for {} directories",
            trajectories_dirs.len()
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

pub fn validate_trajectory_id(id: &str) -> Result<(), ScratchError> {
    if id.is_empty() || id.len() > 128 {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid trajectory id".to_string(),
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid trajectory id".to_string(),
        ));
    }
    Ok(())
}

async fn atomic_write_json(path: &PathBuf, data: &impl Serialize) -> Result<(), String> {
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string(data).map_err(|e| e.to_string())?;
    fs::write(&tmp_path, &json)
        .await
        .map_err(|e| e.to_string())?;
    atomic_write_file(&tmp_path, path).await?;
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
        if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        if let Some(content) = msg
            .get("content")
            .and_then(extract_text_with_image_placeholders_from_json)
        {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.chars().take(200).collect());
            }
        }
    }
    None
}

pub fn extract_text_with_image_placeholders_from_json(
    content_value: &serde_json::Value,
) -> Option<String> {
    if let Some(content) = content_value.as_str() {
        return Some(content.to_string());
    }
    if let Some(content_arr) = content_value.as_array() {
        let parts: Vec<String> = content_arr
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("image_url") {
                    return Some("[image]".to_string());
                }
                if let Some(m_type) = item.get("m_type").and_then(|t| t.as_str()) {
                    if m_type.starts_with("image/") {
                        return Some("[image]".to_string());
                    }
                }
                item.get("text")
                    .and_then(|t| t.as_str())
                    .or_else(|| item.get("m_content").and_then(|t| t.as_str()))
                    .map(|s| s.to_string())
            })
            .collect();
        if !parts.is_empty() {
            return Some(parts.join("\n\n"));
        }
    }
    None
}

fn build_title_generation_context(messages: &[serde_json::Value]) -> String {
    let mut context = String::new();
    let max_messages = 6;
    let max_chars_per_message = 500;
    let mut included_count = 0;

    for msg in messages.iter() {
        if included_count >= max_messages {
            break;
        }
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("unknown");
        if role == "system" || role == "tool" || role == "context_file" || role == "cd_instruction"
        {
            continue;
        }
        let content_text = match msg
            .get("content")
            .and_then(extract_text_with_image_placeholders_from_json)
        {
            Some(text) => text,
            None => continue,
        };
        let truncated: String = content_text.chars().take(max_chars_per_message).collect();
        if !truncated.trim().is_empty() {
            context.push_str(&format!("{}: {}\n\n", role, truncated));
            included_count += 1;
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

    let subagent_config =
        match get_subagent_config(gcx.clone(), TITLE_GENERATION_SUBAGENT_ID, None).await {
            Some(config) => config,
            None => {
                warn!(
                    "subagent config '{}' not found",
                    TITLE_GENERATION_SUBAGENT_ID
                );
                return None;
            }
        };

    let title_prompt = match subagent_config.messages.user_template.as_ref() {
        Some(prompt) => prompt,
        None => {
            warn!(
                "messages.user_template not defined for subagent '{}'",
                TITLE_GENERATION_SUBAGENT_ID
            );
            return None;
        }
    };

    let prompt = format!("Chat conversation:\n{}\n\n{}", context, title_prompt);
    let chat_messages = vec![ChatMessage::new("user".to_string(), prompt)];

    match run_subchat_once(gcx, TITLE_GENERATION_SUBAGENT_ID, chat_messages).await {
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
        let gcx2 = gcx.clone();
        let messages2 = messages.clone();
        let generated_title = crate::buddy::workflows::buddy_wrap_workflow(
            gcx.clone(),
            "title_generation",
            "📋",
            5,
            |t: &String| format!("Title generated: {}", t),
            move || async move {
                generate_title_llm(gcx2, &messages2)
                    .await
                    .ok_or_else(|| "No title generated".to_string())
            },
        )
        .await
        .ok();
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
        let updated_at = data.updated_at.clone();
        data.title = title.clone();
        data.extra
            .insert("isTitleGenerated".to_string(), serde_json::json!(true));
        if let Err(e) = atomic_write_json(&file_path, &data).await {
            warn!("Failed to write trajectory with generated title: {}", e);
            return;
        }
        info!("Updated trajectory {} with generated title: {}", id, title);
        let (session_state, session_error) = get_session_state_for_chat(&sessions, &id).await;
        let worktree = if let Some(candidate) = trajectory_worktree_from_extra(&data.extra) {
            validate_loaded_worktree_strict(gcx.clone(), &id, candidate).await
        } else {
            None
        };
        let event = TrajectoryEvent {
            event_type: "updated".to_string(),
            id: id.clone(),
            updated_at: Some(updated_at),
            title: Some(title.clone()),
            is_title_generated: Some(true),
            session_state: Some(session_state),
            error: session_error,
            message_count: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            model: None,
            mode: None,
            worktree,
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

fn calculate_line_changes_from_messages(messages: &[serde_json::Value]) -> (i64, i64) {
    let mut total_added: i64 = 0;
    let mut total_removed: i64 = 0;

    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "diff" {
            continue;
        }

        let content = match msg.get("content") {
            Some(serde_json::Value::String(s)) => s.as_str(),
            _ => continue,
        };

        if let Ok(chunks) = serde_json::from_str::<Vec<serde_json::Value>>(content) {
            for chunk in chunks {
                if let Some(lines_add) = chunk.get("lines_add").and_then(|v| v.as_str()) {
                    if !lines_add.is_empty() {
                        total_added += lines_add.lines().count() as i64;
                    }
                }
                if let Some(lines_remove) = chunk.get("lines_remove").and_then(|v| v.as_str()) {
                    if !lines_remove.is_empty() {
                        total_removed += lines_remove.lines().count() as i64;
                    }
                }
            }
        }
    }

    (total_added, total_removed)
}

fn calculate_task_progress_from_messages(messages: &[serde_json::Value]) -> (i32, i32, i32) {
    // Build a set of successful tool call IDs (tool messages without tool_failed=true)
    let mut successful_tool_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "tool" {
            continue;
        }
        let tool_failed = msg
            .get("tool_failed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !tool_failed {
            if let Some(tool_call_id) = msg.get("tool_call_id").and_then(|v| v.as_str()) {
                successful_tool_ids.insert(tool_call_id.to_string());
            }
        }
    }

    // Find the last successful tasks_set tool call (iterate in reverse)
    for msg in messages.iter().rev() {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "assistant" {
            continue;
        }

        let tool_calls = match msg.get("tool_calls").and_then(|v| v.as_array()) {
            Some(tc) => tc,
            None => continue,
        };

        // Iterate tool_calls in reverse to find the last tasks_set
        for tc in tool_calls.iter().rev() {
            let function = match tc.get("function") {
                Some(f) => f,
                None => continue,
            };

            let name = function.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name != "tasks_set" {
                continue;
            }

            let tc_id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if tc_id.is_empty() || !successful_tool_ids.contains(tc_id) {
                continue;
            }

            // Parse the arguments
            let args_str = function
                .get("arguments")
                .and_then(|a| a.as_str())
                .unwrap_or("");
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(args_str) {
                if let Some(tasks) = args.get("tasks").and_then(|t| t.as_array()) {
                    let mut total = 0i32;
                    let mut done = 0i32;
                    let mut failed = 0i32;

                    for task in tasks {
                        total += 1;
                        let status = task.get("status").and_then(|s| s.as_str()).unwrap_or("");
                        match status.to_lowercase().as_str() {
                            "completed" | "done" | "complete" => done += 1,
                            "failed" | "error" => failed += 1,
                            _ => {}
                        }
                    }

                    return (total, done, failed);
                }
            }
        }
    }

    (0, 0, 0)
}

fn calculate_line_changes_from_chat_messages(messages: &[ChatMessage]) -> (i64, i64) {
    let mut total_added: i64 = 0;
    let mut total_removed: i64 = 0;

    for msg in messages {
        if msg.role != "diff" {
            continue;
        }

        let content = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
            _ => continue,
        };

        if let Ok(chunks) = serde_json::from_str::<Vec<serde_json::Value>>(content) {
            for chunk in chunks {
                if let Some(lines_add) = chunk.get("lines_add").and_then(|v| v.as_str()) {
                    if !lines_add.is_empty() {
                        total_added += lines_add.lines().count() as i64;
                    }
                }
                if let Some(lines_remove) = chunk.get("lines_remove").and_then(|v| v.as_str()) {
                    if !lines_remove.is_empty() {
                        total_removed += lines_remove.lines().count() as i64;
                    }
                }
            }
        }
    }

    (total_added, total_removed)
}

fn calculate_task_progress_from_chat_messages(messages: &[ChatMessage]) -> (i32, i32, i32) {
    // Build a set of successful tool call IDs (tool messages without tool_failed=true)
    let mut successful_tool_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for msg in messages {
        if msg.role != "tool" {
            continue;
        }
        let tool_failed = msg.tool_failed.unwrap_or(false);
        if !tool_failed && !msg.tool_call_id.is_empty() {
            successful_tool_ids.insert(msg.tool_call_id.clone());
        }
    }

    // Find the last successful tasks_set tool call (iterate in reverse)
    for msg in messages.iter().rev() {
        if msg.role != "assistant" {
            continue;
        }

        let tool_calls = match &msg.tool_calls {
            Some(tc) => tc,
            None => continue,
        };

        // Iterate tool_calls in reverse to find the last tasks_set
        for tc in tool_calls.iter().rev() {
            if tc.function.name != "tasks_set" {
                continue;
            }

            if tc.id.is_empty() || !successful_tool_ids.contains(&tc.id) {
                continue;
            }

            // Parse the arguments
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(&tc.function.arguments) {
                if let Some(tasks) = args.get("tasks").and_then(|t| t.as_array()) {
                    let mut total = 0i32;
                    let mut done = 0i32;
                    let mut failed = 0i32;

                    for task in tasks {
                        total += 1;
                        let status = task.get("status").and_then(|s| s.as_str()).unwrap_or("");
                        match status.to_lowercase().as_str() {
                            "completed" | "done" | "complete" => done += 1,
                            "failed" | "error" => failed += 1,
                            _ => {}
                        }
                    }

                    return (total, done, failed);
                }
            }
        }
    }

    (0, 0, 0)
}

struct TokenTotals {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    cost_usd: Option<f64>,
}

fn calculate_token_totals_from_messages(messages: &[serde_json::Value]) -> TokenTotals {
    let mut prompt_tokens: u64 = 0;
    let mut completion_tokens: u64 = 0;
    let mut total_tokens: u64 = 0;
    let mut cache_read_tokens: u64 = 0;
    let mut cache_creation_tokens: u64 = 0;
    let mut cost_usd: Option<f64> = None;

    for msg in messages {
        let usage = match msg.get("usage") {
            Some(u) if !u.is_null() => u,
            _ => continue,
        };
        if let Some(v) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
            prompt_tokens += v;
        }
        if let Some(v) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
            completion_tokens += v;
        }
        if let Some(v) = usage.get("total_tokens").and_then(|v| v.as_u64()) {
            total_tokens += v;
        }
        for key in &["cache_read_input_tokens", "cache_read_tokens"] {
            if let Some(v) = usage.get(key).and_then(|v| v.as_u64()) {
                cache_read_tokens += v;
                break;
            }
        }
        for key in &["cache_creation_input_tokens", "cache_creation_tokens"] {
            if let Some(v) = usage.get(key).and_then(|v| v.as_u64()) {
                cache_creation_tokens += v;
                break;
            }
        }
        if let Some(total) = usage
            .get("metering_usd")
            .and_then(|m| m.get("total_usd"))
            .and_then(|v| v.as_f64())
        {
            *cost_usd.get_or_insert(0.0) += total;
        }
    }

    TokenTotals {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        cost_usd,
    }
}

fn calculate_token_totals_from_chat_messages(messages: &[ChatMessage]) -> TokenTotals {
    let mut prompt_tokens: u64 = 0;
    let mut completion_tokens: u64 = 0;
    let mut total_tokens: u64 = 0;
    let mut cache_read_tokens: u64 = 0;
    let mut cache_creation_tokens: u64 = 0;
    let mut cost_usd: Option<f64> = None;

    for msg in messages {
        let usage = match &msg.usage {
            Some(u) => u,
            None => continue,
        };
        prompt_tokens += usage.prompt_tokens as u64;
        completion_tokens += usage.completion_tokens as u64;
        total_tokens += usage.total_tokens as u64;
        if let Some(v) = usage.cache_read_tokens {
            cache_read_tokens += v as u64;
        }
        if let Some(v) = usage.cache_creation_tokens {
            cache_creation_tokens += v as u64;
        }
        if let Some(ref m) = usage.metering_usd {
            *cost_usd.get_or_insert(0.0) += m.total_usd;
        }
    }

    TokenTotals {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        cost_usd,
    }
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

    let parent_id = data
        .extra
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let link_type = data
        .extra
        .get("link_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let root_chat_id = data
        .extra
        .get("root_chat_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let worktree = None;

    let (total_lines_added, total_lines_removed) =
        calculate_line_changes_from_messages(&data.messages);
    let (tasks_total, tasks_done, tasks_failed) =
        calculate_task_progress_from_messages(&data.messages);
    let token_totals = calculate_token_totals_from_messages(&data.messages);

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
        root_chat_id,
        worktree,
        total_lines_added,
        total_lines_removed,
        tasks_total,
        tasks_done,
        tasks_failed,
        total_prompt_tokens: token_totals.prompt_tokens,
        total_completion_tokens: token_totals.completion_tokens,
        total_tokens: token_totals.total_tokens,
        total_cache_read_tokens: token_totals.cache_read_tokens,
        total_cache_creation_tokens: token_totals.cache_creation_tokens,
        total_cost_usd: token_totals.cost_usd,
    }
}

#[derive(Debug, Deserialize)]
pub struct TrajectoriesListQuery {
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PaginatedTrajectories {
    pub items: Vec<TrajectoryMeta>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub total_count: usize,
}

fn encode_cursor(updated_at: &str, id: &str) -> String {
    use base64::Engine;
    let cursor_data = format!("{}|{}", updated_at, id);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(cursor_data.as_bytes())
}

fn decode_cursor(cursor: &str) -> Option<(String, String)> {
    use base64::Engine;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()?;
    let cursor_str = String::from_utf8(decoded).ok()?;
    let parts: Vec<&str> = cursor_str.splitn(2, '|').collect();
    if parts.len() == 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

async fn trajectory_data_to_meta_validated(
    gcx: Arc<ARwLock<GlobalContext>>,
    data: &TrajectoryData,
) -> TrajectoryMeta {
    let mut meta = trajectory_data_to_meta(data);
    if let Some(worktree) = trajectory_worktree_from_extra(&data.extra) {
        meta.worktree = validate_loaded_worktree_strict(gcx, &data.id, worktree).await;
    }
    meta
}

pub async fn handle_v1_trajectories_list(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::extract::Query(params): axum::extract::Query<TrajectoriesListQuery>,
) -> Result<Response<Body>, ScratchError> {
    let limit = params.limit.unwrap_or(50).min(200);
    let cursor_filter = match &params.cursor {
        Some(c) => {
            let decoded = decode_cursor(c);
            if decoded.is_none() {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    "Invalid cursor format".to_string(),
                ));
            }
            decoded
        }
        None => None,
    };

    let mut all_items: Vec<TrajectoryMeta> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for trajectories_dir in get_all_trajectories_dirs(gcx.clone()).await {
        if !trajectories_dir.exists() {
            continue;
        }
        let mut entries = match fs::read_dir(&trajectories_dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path).await {
                if let Ok(data) = serde_json::from_str::<TrajectoryData>(&content) {
                    if data.extra.get("buddy_meta").map_or(false, |v| !v.is_null()) {
                        continue;
                    }
                    if seen_ids.insert(data.id.clone()) {
                        all_items.push(trajectory_data_to_meta_validated(gcx.clone(), &data).await);
                    }
                }
            }
        }
    }
    enrich_with_session_state(gcx, &mut all_items).await;
    all_items.sort_by(|a, b| match b.updated_at.cmp(&a.updated_at) {
        std::cmp::Ordering::Equal => b.id.cmp(&a.id),
        other => other,
    });

    let total_count = all_items.len();

    let start_idx = if let Some((cursor_updated_at, cursor_id)) = cursor_filter {
        all_items
            .iter()
            .position(|item| {
                (item.updated_at.as_str(), item.id.as_str())
                    < (cursor_updated_at.as_str(), cursor_id.as_str())
            })
            .unwrap_or(all_items.len())
    } else {
        0
    };

    let page_items: Vec<TrajectoryMeta> = all_items
        .into_iter()
        .skip(start_idx)
        .take(limit + 1)
        .collect();

    let has_more = page_items.len() > limit;
    let items: Vec<TrajectoryMeta> = page_items.into_iter().take(limit).collect();

    let next_cursor = if has_more {
        items
            .last()
            .map(|last| encode_cursor(&last.updated_at, &last.id))
    } else {
        None
    };

    let response = PaginatedTrajectories {
        items,
        next_cursor,
        has_more,
        total_count,
    };

    let json = serde_json::to_string(&response).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Serialization error: {}", e),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json))
        .unwrap())
}

pub async fn list_all_trajectories_meta(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<Vec<TrajectoryMeta>, String> {
    let mut result: Vec<TrajectoryMeta> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for trajectories_dir in get_all_trajectories_dirs(gcx.clone()).await {
        if !trajectories_dir.exists() {
            continue;
        }
        let mut entries = match fs::read_dir(&trajectories_dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path).await {
                if let Ok(data) = serde_json::from_str::<TrajectoryData>(&content) {
                    if data.extra.get("buddy_meta").map_or(false, |v| !v.is_null()) {
                        continue;
                    }
                    if seen_ids.insert(data.id.clone()) {
                        result.push(trajectory_data_to_meta_validated(gcx.clone(), &data).await);
                    }
                }
            }
        }
    }

    for tasks_dir in crate::tasks::storage::get_all_tasks_dirs(gcx.clone()).await {
        if !tasks_dir.exists() {
            continue;
        }
        let mut task_entries = match fs::read_dir(&tasks_dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
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
                for traj in collect_task_trajectories(&role_dir, &task_id, role, None).await {
                    if seen_ids.insert(traj.id.clone()) {
                        result.push(traj);
                    }
                }
            }
        }
    }

    enrich_with_session_state(gcx, &mut result).await;
    result.sort_by(|a, b| match b.updated_at.cmp(&a.updated_at) {
        std::cmp::Ordering::Equal => b.id.cmp(&a.id),
        other => other,
    });

    Ok(result)
}

pub async fn handle_v1_trajectories_all(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let result = list_all_trajectories_meta(gcx)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
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
    let session_arcs: Vec<(usize, Arc<AMutex<ChatSession>>)> = {
        let gcx_locked = gcx.read().await;
        let sessions = gcx_locked.chat_sessions.read().await;
        trajectories
            .iter()
            .enumerate()
            .filter_map(|(idx, traj)| sessions.get(&traj.id).map(|arc| (idx, arc.clone())))
            .collect()
    };

    for (idx, session_arc) in session_arcs {
        let session = session_arc.lock().await;
        trajectories[idx].session_state = Some(session.runtime.state.to_string());
        if trajectories[idx].worktree.is_none() {
            trajectories[idx].worktree = session.thread.worktree.clone();
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
            let sub_trajectories = Box::pin(collect_task_trajectories(
                &path,
                task_id,
                role,
                Some(sub_agent_id),
            ))
            .await;
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
    AxumPath(id): AxumPath<String>,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&id)?;
    let file_path = find_trajectory_path(gcx, &id).await.ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, "Trajectory not found".to_string())
    })?;
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
    AxumPath(id): AxumPath<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&id)?;
    let data: TrajectoryData = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut data = data;
    data.updated_at = now.clone();
    if data.created_at.is_empty() {
        data.created_at = now.clone();
    }
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
    let worktree = if let Some(candidate) = sanitize_worktree_extra(&mut data.extra) {
        match validate_loaded_worktree_strict(gcx.clone(), &id, candidate).await {
            Some(validated) => {
                data.extra.insert(
                    "worktree".to_string(),
                    serde_json::to_value(&validated).unwrap_or_default(),
                );
                Some(validated)
            }
            None => {
                data.extra.remove("worktree");
                None
            }
        }
    } else {
        None
    };
    atomic_write_json(&file_path, &data)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let parent_id = data
        .extra
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let link_type = data
        .extra
        .get("link_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let effective_root = data
        .extra
        .get("root_chat_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| id.clone());
    let sessions = gcx.read().await.chat_sessions.clone();
    let (session_state, session_error) = get_session_state_for_chat(&sessions, &id).await;
    let (total_lines_added, total_lines_removed) =
        calculate_line_changes_from_messages(&data.messages);
    let (tasks_total, tasks_done, tasks_failed) =
        calculate_task_progress_from_messages(&data.messages);
    let token_totals = calculate_token_totals_from_messages(&data.messages);
    let event = TrajectoryEvent {
        event_type: if is_new {
            "created".to_string()
        } else {
            "updated".to_string()
        },
        id: id.clone(),
        updated_at: Some(data.updated_at.clone()),
        title: Some(data.title.clone()),
        is_title_generated: Some(is_title_generated),
        session_state: Some(session_state),
        error: session_error,
        message_count: Some(data.messages.len()),
        parent_id,
        link_type,
        root_chat_id: Some(effective_root),
        model: Some(data.model.clone()),
        mode: Some(data.mode.clone()),
        worktree,
        total_lines_added: Some(total_lines_added),
        total_lines_removed: Some(total_lines_removed),
        tasks_total: Some(tasks_total),
        tasks_done: Some(tasks_done),
        tasks_failed: Some(tasks_failed),
        total_prompt_tokens: Some(token_totals.prompt_tokens),
        total_completion_tokens: Some(token_totals.completion_tokens),
        total_tokens: Some(token_totals.total_tokens),
        total_cache_read_tokens: Some(token_totals.cache_read_tokens),
        total_cache_creation_tokens: Some(token_totals.cache_creation_tokens),
        total_cost_usd: token_totals.cost_usd,
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
    AxumPath(id): AxumPath<String>,
) -> Result<Response<Body>, ScratchError> {
    validate_trajectory_id(&id)?;
    let file_path = find_trajectory_path(gcx.clone(), &id)
        .await
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, "Trajectory not found".to_string())
        })?;
    fs::remove_file(&file_path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let event = TrajectoryEvent {
        event_type: "deleted".to_string(),
        id: id.clone(),
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
                    match serde_json::to_string(&event) {
                        Ok(json) => yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json)),
                        Err(e) => {
                            tracing::error!("Failed to serialize trajectory SSE event: {}", e);
                            break;
                        }
                    }
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
    use crate::chat::types::ActiveCommandContext;
    use std::path::Path;
    use std::process::Command;

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "main"]);
        run_git(root, &["config", "core.autocrlf", "false"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn trajectory_worktree_sample() -> WorktreeMeta {
        WorktreeMeta {
            id: "wt-1".to_string(),
            kind: "task_agent".to_string(),
            root: std::path::PathBuf::from("/tmp/refact-wt"),
            source_workspace_root: std::path::PathBuf::from("/tmp/refact-src"),
            repo_root: std::path::PathBuf::from("/tmp/refact-src"),
            branch: Some("refact/task/card".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: Some("abc123".to_string()),
            task_id: Some("task-1".to_string()),
            card_id: Some("card-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            enforce: true,
        }
    }

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
        assert!(validate_trajectory_id("planner-task-1").is_ok());
        assert!(validate_trajectory_id("A1b2C3").is_ok());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_empty() {
        assert!(validate_trajectory_id("").is_err());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_too_long() {
        let long_id = "a".repeat(129);
        assert!(validate_trajectory_id(&long_id).is_err());
        let max_id = "a".repeat(128);
        assert!(validate_trajectory_id(&max_id).is_ok());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_invalid_chars() {
        assert!(validate_trajectory_id("has space").is_err());
        assert!(validate_trajectory_id("has.dot").is_err());
        assert!(validate_trajectory_id("has@symbol").is_err());
        assert!(validate_trajectory_id("has#hash").is_err());
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
                    extra_content: None,
                },
                ChatToolCall {
                    id: "call_2".to_string(),
                    index: None,
                    function: ChatToolFunction {
                        name: "test2".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
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
                extra_content: None,
            }]),
            ..Default::default()
        }];
        fix_tool_call_indexes(&mut messages);
        let tool_calls = messages[0].tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls[0].index, Some(5));
    }

    #[test]
    fn test_calculate_token_totals_from_messages_with_usage() {
        let messages = vec![
            json!({
                "role": "assistant",
                "content": "Hello",
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "total_tokens": 15,
                    "cache_read_input_tokens": 3,
                    "cache_creation_input_tokens": 2,
                    "metering_usd": {
                        "prompt_usd": 0.001,
                        "generated_usd": 0.002,
                        "total_usd": 0.003
                    }
                }
            }),
            json!({
                "role": "assistant",
                "content": "World",
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 10,
                    "total_tokens": 30,
                    "metering_usd": {
                        "prompt_usd": 0.002,
                        "generated_usd": 0.004,
                        "total_usd": 0.006
                    }
                }
            }),
        ];
        let totals = calculate_token_totals_from_messages(&messages);
        assert_eq!(totals.prompt_tokens, 30);
        assert_eq!(totals.completion_tokens, 15);
        assert_eq!(totals.total_tokens, 45);
        assert_eq!(totals.cache_read_tokens, 3);
        assert_eq!(totals.cache_creation_tokens, 2);
        let cost = totals.cost_usd.unwrap();
        assert!((cost - 0.009).abs() < 1e-9);
    }

    #[test]
    fn test_calculate_token_totals_from_messages_no_usage() {
        let messages = vec![
            json!({"role": "user", "content": "Hello"}),
            json!({"role": "assistant", "content": "Hi"}),
        ];
        let totals = calculate_token_totals_from_messages(&messages);
        assert_eq!(totals.prompt_tokens, 0);
        assert_eq!(totals.completion_tokens, 0);
        assert_eq!(totals.total_tokens, 0);
        assert_eq!(totals.cache_read_tokens, 0);
        assert_eq!(totals.cache_creation_tokens, 0);
        assert!(totals.cost_usd.is_none());
    }

    #[test]
    fn test_calculate_token_totals_from_messages_null_usage() {
        let messages = vec![json!({"role": "assistant", "content": "Hi", "usage": null})];
        let totals = calculate_token_totals_from_messages(&messages);
        assert_eq!(totals.prompt_tokens, 0);
        assert!(totals.cost_usd.is_none());
    }

    #[test]
    fn test_calculate_token_totals_from_messages_alias_keys() {
        let messages = vec![json!({
            "role": "assistant",
            "content": "Hi",
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 3,
                "total_tokens": 8,
                "cache_read_tokens": 7,
                "cache_creation_tokens": 4,
            }
        })];
        let totals = calculate_token_totals_from_messages(&messages);
        assert_eq!(totals.cache_read_tokens, 7);
        assert_eq!(totals.cache_creation_tokens, 4);
    }

    #[test]
    fn test_calculate_token_totals_from_chat_messages_with_usage() {
        use crate::call_validation::{ChatUsage, MeteringUsd};
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                usage: Some(ChatUsage {
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    total_tokens: 150,
                    cache_read_tokens: Some(10),
                    cache_creation_tokens: Some(5),
                    metering_usd: Some(MeteringUsd {
                        prompt_usd: 0.01,
                        generated_usd: 0.02,
                        cache_read_usd: None,
                        cache_creation_usd: None,
                        total_usd: 0.03,
                    }),
                }),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                usage: Some(ChatUsage {
                    prompt_tokens: 200,
                    completion_tokens: 100,
                    total_tokens: 300,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    metering_usd: None,
                }),
                ..Default::default()
            },
        ];
        let totals = calculate_token_totals_from_chat_messages(&messages);
        assert_eq!(totals.prompt_tokens, 300);
        assert_eq!(totals.completion_tokens, 150);
        assert_eq!(totals.total_tokens, 450);
        assert_eq!(totals.cache_read_tokens, 10);
        assert_eq!(totals.cache_creation_tokens, 5);
        let cost = totals.cost_usd.unwrap();
        assert!((cost - 0.03).abs() < 1e-9);
    }

    #[test]
    fn test_calculate_token_totals_from_chat_messages_no_usage() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            ..Default::default()
        }];
        let totals = calculate_token_totals_from_chat_messages(&messages);
        assert_eq!(totals.prompt_tokens, 0);
        assert!(totals.cost_usd.is_none());
    }

    #[test]
    fn test_trajectory_event_serialization() {
        let event = TrajectoryEvent {
            event_type: "updated".to_string(),
            id: "chat-123".to_string(),
            updated_at: Some("2024-01-01T00:00:00Z".to_string()),
            title: Some("Test Title".to_string()),
            is_title_generated: Some(true),
            session_state: Some("generating".to_string()),
            error: Some("Test error".to_string()),
            message_count: Some(5),
            parent_id: Some("parent-123".to_string()),
            link_type: Some("subagent".to_string()),
            root_chat_id: Some("root-123".to_string()),
            model: Some("gpt-4".to_string()),
            mode: Some("AGENT".to_string()),
            worktree: None,
            total_lines_added: Some(100),
            total_lines_removed: Some(50),
            tasks_total: Some(5),
            tasks_done: Some(3),
            tasks_failed: Some(1),
            total_prompt_tokens: Some(1000),
            total_completion_tokens: Some(500),
            total_tokens: Some(1500),
            total_cache_read_tokens: Some(100),
            total_cache_creation_tokens: Some(50),
            total_cost_usd: Some(0.042),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "updated");
        assert_eq!(json["id"], "chat-123");
        assert_eq!(json["session_state"], "generating");
        assert_eq!(json["error"], "Test error");
        assert_eq!(json["message_count"], 5);
        assert_eq!(json["parent_id"], "parent-123");
        assert_eq!(json["link_type"], "subagent");
        assert_eq!(json["total_lines_added"], 100);
        assert_eq!(json["total_lines_removed"], 50);
        assert_eq!(json["tasks_total"], 5);
        assert_eq!(json["tasks_done"], 3);
        assert_eq!(json["tasks_failed"], 1);
        assert_eq!(json["total_prompt_tokens"], 1000);
        assert_eq!(json["total_completion_tokens"], 500);
        assert_eq!(json["total_tokens"], 1500);
        assert_eq!(json["total_cache_read_tokens"], 100);
        assert_eq!(json["total_cache_creation_tokens"], 50);
        assert!((json["total_cost_usd"].as_f64().unwrap() - 0.042).abs() < 1e-9);
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
                boost_reasoning: Some(true),
                reasoning_effort: None,
                thinking_budget: None,
                temperature: None,
                frequency_penalty: None,
                max_tokens: None,
                parallel_tool_calls: None,
                context_tokens_cap: Some(8000),
                include_project_info: false,
                checkpoints_enabled: true,
                is_title_generated: true,
                auto_approve_editing_tools: false,
                auto_approve_dangerous_commands: false,
                task_meta: None,
                worktree: None,
                parent_id: Some("parent-chat-id".to_string()),
                link_type: Some("subagent".to_string()),
                root_chat_id: Some("root-chat-id".to_string()),
                previous_response_id: None,
                browser_meta: None,
                active_skill: None,
                auto_enrichment_enabled: None,
                buddy_meta: None,
            },
            messages: vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
            runtime: super::super::types::RuntimeState::default(),
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            event_seq: 0,
            event_tx: tx,
            recent_request_ids: VecDeque::new(),
            recent_request_ids_set: std::collections::HashSet::new(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            user_interrupt_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            last_activity: Instant::now(),
            trajectory_dirty: false,
            trajectory_version: 5,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            closed: false,
            closed_flag: Arc::new(AtomicBool::new(false)),
            external_reload_pending: false,
            last_prompt_messages: Vec::new(),
            cache_guard_snapshot: None,
            cache_guard_force_next: false,
            task_agent_error: None,
            trajectory_events_tx: None,
            pending_browser_message: None,
            active_command: ActiveCommandContext::default(),
            skills_available_count: 0,
            skills_included: Vec::new(),
            pending_skill_deactivation: None,
            stop_hook_handle: None,
            suppress_auto_enrichment_for_next_turn: false,
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

    #[test]
    fn test_trajectory_roundtrip_active_skill() {
        use super::super::types::*;
        use super::super::types::ActiveCommandContext;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        use tokio::sync::{broadcast, Notify};
        use std::collections::VecDeque;
        use std::time::Instant;

        let (tx, _rx) = broadcast::channel(16);
        let mut session = ChatSession {
            chat_id: "skill-test".to_string(),
            thread: ThreadParams {
                id: "skill-test".to_string(),
                active_skill: Some("my-skill".to_string()),
                ..Default::default()
            },
            messages: vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
            runtime: RuntimeState::default(),
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            event_seq: 0,
            event_tx: tx,
            recent_request_ids: VecDeque::new(),
            recent_request_ids_set: std::collections::HashSet::new(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            user_interrupt_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            last_activity: Instant::now(),
            trajectory_dirty: false,
            trajectory_version: 1,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            closed: false,
            closed_flag: Arc::new(AtomicBool::new(false)),
            external_reload_pending: false,
            last_prompt_messages: Vec::new(),
            cache_guard_snapshot: None,
            cache_guard_force_next: false,
            task_agent_error: None,
            trajectory_events_tx: None,
            pending_browser_message: None,
            active_command: ActiveCommandContext::default(),
            skills_available_count: 0,
            skills_included: Vec::new(),
            pending_skill_deactivation: None,
            stop_hook_handle: None,
            suppress_auto_enrichment_for_next_turn: false,
        };

        let snapshot = TrajectorySnapshot::from_session(&session);
        assert_eq!(snapshot.active_skill, Some("my-skill".to_string()));

        session.thread.active_skill = None;
        let snapshot_none = TrajectorySnapshot::from_session(&session);
        assert!(snapshot_none.active_skill.is_none());
    }

    #[test]
    fn test_trajectory_load_without_active_skill_field() {
        let json_str = r#"{"id":"chat-1","title":"T","model":"m","mode":"agent","tool_use":"agent","messages":[],"created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z","include_project_info":true,"checkpoints_enabled":true}"#;
        let t: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let active_skill = t
            .get("active_skill")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        assert!(
            active_skill.is_none(),
            "Old trajectories must load with active_skill = None"
        );
    }

    #[test]
    fn trajectory_worktree_snapshot_from_session_captures_thread_worktree() {
        let worktree = trajectory_worktree_sample();
        let mut session = ChatSession::new("wt-snapshot".to_string());
        session.thread.worktree = Some(worktree.clone());
        let snapshot = TrajectorySnapshot::from_session(&session);
        assert_eq!(snapshot.worktree, Some(worktree));
    }

    #[test]
    fn trajectory_worktree_meta_creation_omits_unvalidated_worktree() {
        let worktree = trajectory_worktree_sample();
        let mut extra = serde_json::Map::new();
        extra.insert(
            "worktree".to_string(),
            serde_json::to_value(&worktree).unwrap(),
        );
        let data = TrajectoryData {
            id: "meta-chat".to_string(),
            title: "Meta".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            model: "model".to_string(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            messages: Vec::new(),
            extra,
        };
        let meta = trajectory_data_to_meta(&data);
        assert!(meta.worktree.is_none());
    }

    #[test]
    fn trajectory_worktree_invalid_extra_is_not_preserved() {
        let mut extra = serde_json::Map::new();
        extra.insert("worktree".to_string(), json!({"root":"/tmp/untrusted"}));
        let worktree = sanitize_worktree_extra(&mut extra);
        assert!(worktree.is_none());
        assert!(extra.get("worktree").is_none());
    }

    #[tokio::test]
    async fn trajectory_worktree_save_load_roundtrips_top_level_field() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let gcx_lock = gcx.read().await;
            *gcx_lock.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
            drop(gcx_lock);
            gcx.write().await.cache_dir = cache.clone();
        }
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/roundtrip".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        let worktree = created.worktree.meta.clone();
        let chat_id = "wt-roundtrip".to_string();
        let snapshot = TrajectorySnapshot {
            chat_id: chat_id.clone(),
            title: "Worktree Chat".to_string(),
            model: "model".to_string(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            messages: vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            boost_reasoning: false,
            checkpoints_enabled: true,
            context_tokens_cap: None,
            include_project_info: true,
            is_title_generated: true,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            version: 1,
            task_meta: None,
            worktree: Some(worktree.clone()),
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
        };

        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();
        let path = source
            .join(".refact")
            .join("trajectories")
            .join(format!("{}.json", chat_id));
        let raw = tokio::fs::read_to_string(path).await.unwrap();
        let raw_json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(raw_json["worktree"]["id"], worktree.id);
        assert_eq!(raw_json["worktree"]["branch"], "refact/chat/roundtrip");

        let loaded = load_trajectory_for_chat(gcx.clone(), &chat_id)
            .await
            .unwrap();
        assert_eq!(loaded.thread.worktree, Some(worktree.clone()));
        let listed = list_all_trajectories_meta(gcx).await.unwrap();
        let listed_worktree = listed
            .iter()
            .find(|item| item.id == chat_id)
            .and_then(|item| item.worktree.clone())
            .unwrap();
        assert_eq!(listed_worktree.id, worktree.id);
    }

    #[tokio::test]
    async fn save_trajectory_preserves_updated_at_when_messages_do_not_change() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let gcx_lock = gcx.read().await;
            *gcx_lock.documents_state.workspace_folders.lock().unwrap() =
                vec![dir.path().to_path_buf()];
        }

        fn snapshot(chat_id: &str, title: &str, messages: Vec<ChatMessage>) -> TrajectorySnapshot {
            TrajectorySnapshot {
                chat_id: chat_id.to_string(),
                title: title.to_string(),
                model: "model".to_string(),
                mode: "agent".to_string(),
                tool_use: "agent".to_string(),
                messages,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                boost_reasoning: false,
                checkpoints_enabled: true,
                context_tokens_cap: None,
                include_project_info: true,
                is_title_generated: true,
                auto_approve_editing_tools: false,
                auto_approve_dangerous_commands: false,
                version: 1,
                task_meta: None,
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
            }
        }

        let chat_id = "updated-at-message-activity";
        let messages = vec![ChatMessage::new("user".to_string(), "Hello".to_string())];
        save_trajectory_snapshot(gcx.clone(), snapshot(chat_id, "First", messages.clone()))
            .await
            .unwrap();
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{}.json", chat_id));
        let first_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        let first_updated_at = first_raw["updated_at"].as_str().unwrap().to_string();

        save_trajectory_snapshot(gcx.clone(), snapshot(chat_id, "Retitled", messages.clone()))
            .await
            .unwrap();
        let retitled_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(
            retitled_raw["updated_at"].as_str().unwrap(),
            first_updated_at
        );

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let mut changed_messages = messages;
        changed_messages.push(ChatMessage::new(
            "assistant".to_string(),
            "Generated".to_string(),
        ));
        save_trajectory_snapshot(gcx, snapshot(chat_id, "Retitled", changed_messages))
            .await
            .unwrap();
        let changed_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_ne!(
            changed_raw["updated_at"].as_str().unwrap(),
            first_updated_at
        );
    }

    #[tokio::test]
    async fn trajectory_worktree_old_json_without_worktree_loads_none() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let gcx_lock = gcx.read().await;
            *gcx_lock.documents_state.workspace_folders.lock().unwrap() =
                vec![dir.path().to_path_buf()];
        }
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("old-chat.json"),
            r#"{"id":"old-chat","title":"Old","model":"m","mode":"agent","tool_use":"agent","messages":[],"created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z","include_project_info":true,"checkpoints_enabled":true}"#,
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "old-chat").await.unwrap();
        assert!(loaded.thread.worktree.is_none());
    }

    #[tokio::test]
    async fn trajectory_worktree_unregistered_top_level_metadata_is_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let gcx_lock = gcx.read().await;
            *gcx_lock.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
            drop(gcx_lock);
            gcx.write().await.cache_dir = cache;
        }
        let traj_dir = source.join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        let untrusted = json!({
            "id": "untrusted-wt-chat",
            "title": "Untrusted",
            "model": "m",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "worktree": {
                "id": "wt-evil",
                "kind": "chat",
                "root": dir.path().join("evil").to_string_lossy().to_string(),
                "source_workspace_root": source.to_string_lossy().to_string(),
                "repo_root": source.to_string_lossy().to_string(),
                "enforce": true
            }
        });
        tokio::fs::write(
            traj_dir.join("untrusted-wt-chat.json"),
            serde_json::to_string(&untrusted).unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx.clone(), "untrusted-wt-chat")
            .await
            .unwrap();
        assert!(loaded.thread.worktree.is_none());
        let listed = list_all_trajectories_meta(gcx).await.unwrap();
        let listed_worktree = listed
            .iter()
            .find(|item| item.id == "untrusted-wt-chat")
            .and_then(|item| item.worktree.clone());
        assert!(listed_worktree.is_none());
    }

    #[tokio::test]
    async fn trajectory_worktree_legacy_task_agent_hydrates_from_board_mirror() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let gcx_lock = gcx.read().await;
            *gcx_lock.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
            drop(gcx_lock);
            gcx.write().await.cache_dir = cache;
        }

        let task_id = "task-legacy";
        let agent_id = "agent-1";
        let card_id = "card-1";
        let chat_id = "legacy-agent-chat";
        let task_dir = source.join(".refact").join("tasks").join(task_id);
        tokio::fs::create_dir_all(task_dir.join("trajectories").join("agents").join(agent_id))
            .await
            .unwrap();
        let meta = crate::tasks::types::TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: "Task".to_string(),
            status: crate::tasks::types::TaskStatus::Active,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            cards_total: 1,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 1,
            base_branch: Some("main".to_string()),
            base_commit: Some("base123".to_string()),
            default_agent_model: None,
            is_name_generated: true,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&meta).unwrap(),
        )
        .await
        .unwrap();
        let agent_worktree = dir.path().join("agent-worktree");
        let agent_worktree_arg = agent_worktree.to_string_lossy().to_string();
        run_git(
            &source,
            &[
                "worktree",
                "add",
                "-b",
                "refact/task/card",
                &agent_worktree_arg,
                "main",
            ],
        );
        let board = crate::tasks::types::TaskBoard {
            schema_version: 1,
            rev: 1,
            columns: Vec::new(),
            cards: vec![crate::tasks::types::BoardCard {
                id: card_id.to_string(),
                title: "Card".to_string(),
                column: "doing".to_string(),
                priority: "P1".to_string(),
                depends_on: Vec::new(),
                instructions: String::new(),
                assignee: Some(agent_id.to_string()),
                agent_chat_id: Some(chat_id.to_string()),
                status_updates: Vec::new(),
                final_report: None,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                started_at: None,
                last_heartbeat_at: None,
                completed_at: None,
                agent_branch: Some("refact/task/card".to_string()),
                agent_worktree: Some(agent_worktree.to_string_lossy().to_string()),
                agent_worktree_name: Some("wt-legacy".to_string()),
                target_files: Vec::new(),
            }],
        };
        tokio::fs::write(
            task_dir.join("board.yaml"),
            serde_yaml::to_string(&board).unwrap(),
        )
        .await
        .unwrap();
        let trajectory = json!({
            "id": chat_id,
            "title": "Legacy Agent",
            "model": "m",
            "mode": "task_agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "task_meta": {
                "task_id": task_id,
                "role": "agents",
                "agent_id": agent_id,
                "card_id": card_id
            }
        });
        tokio::fs::write(
            task_dir
                .join("trajectories")
                .join("agents")
                .join(agent_id)
                .join(format!("{}.json", chat_id)),
            serde_json::to_string(&trajectory).unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        let worktree = loaded.thread.worktree.unwrap();
        assert_eq!(worktree.id, "wt-legacy");
        assert_eq!(worktree.kind, "task_agent");
        assert_eq!(worktree.root, agent_worktree);
        assert_eq!(worktree.branch.as_deref(), Some("refact/task/card"));
        assert_eq!(worktree.base_branch.as_deref(), Some("main"));
        assert_eq!(worktree.base_commit.as_deref(), Some("base123"));
        assert!(worktree.enforce);
    }

    #[tokio::test]
    async fn trajectory_worktree_legacy_task_agent_rejects_mismatched_chat_identity() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let gcx_lock = gcx.read().await;
            *gcx_lock.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
            drop(gcx_lock);
            gcx.write().await.cache_dir = cache;
        }

        let task_id = "task-legacy-mismatch";
        let agent_id = "agent-1";
        let card_id = "card-1";
        let chat_id = "wrong-agent-chat";
        let task_dir = source.join(".refact").join("tasks").join(task_id);
        tokio::fs::create_dir_all(task_dir.join("trajectories").join("agents").join(agent_id))
            .await
            .unwrap();
        let meta = crate::tasks::types::TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: "Task".to_string(),
            status: crate::tasks::types::TaskStatus::Active,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            cards_total: 1,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 1,
            base_branch: Some("main".to_string()),
            base_commit: Some("base123".to_string()),
            default_agent_model: None,
            is_name_generated: true,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&meta).unwrap(),
        )
        .await
        .unwrap();
        let agent_worktree = dir.path().join("agent-worktree-mismatch");
        let agent_worktree_arg = agent_worktree.to_string_lossy().to_string();
        run_git(
            &source,
            &[
                "worktree",
                "add",
                "-b",
                "refact/task/card-mismatch",
                &agent_worktree_arg,
                "main",
            ],
        );
        let board = crate::tasks::types::TaskBoard {
            schema_version: 1,
            rev: 1,
            columns: Vec::new(),
            cards: vec![crate::tasks::types::BoardCard {
                id: card_id.to_string(),
                title: "Card".to_string(),
                column: "doing".to_string(),
                priority: "P1".to_string(),
                depends_on: Vec::new(),
                instructions: String::new(),
                assignee: Some(agent_id.to_string()),
                agent_chat_id: Some("actual-agent-chat".to_string()),
                status_updates: Vec::new(),
                final_report: None,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                started_at: None,
                last_heartbeat_at: None,
                completed_at: None,
                agent_branch: Some("refact/task/card-mismatch".to_string()),
                agent_worktree: Some(agent_worktree.to_string_lossy().to_string()),
                agent_worktree_name: Some("wt-legacy-mismatch".to_string()),
                target_files: Vec::new(),
            }],
        };
        tokio::fs::write(
            task_dir.join("board.yaml"),
            serde_yaml::to_string(&board).unwrap(),
        )
        .await
        .unwrap();
        let trajectory = json!({
            "id": chat_id,
            "title": "Legacy Agent",
            "model": "m",
            "mode": "task_agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "task_meta": {
                "task_id": task_id,
                "role": "agents",
                "agent_id": agent_id,
                "card_id": card_id
            }
        });
        tokio::fs::write(
            task_dir
                .join("trajectories")
                .join("agents")
                .join(agent_id)
                .join(format!("{}.json", chat_id)),
            serde_json::to_string(&trajectory).unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert!(loaded.thread.worktree.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn stress_atomic_write_json_pressure_baseline() {
        const MESSAGE_COUNT: usize = 1_000;
        const MESSAGE_SIZE: usize = 1_024;
        const WRITE_RUNS: usize = 120;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("stress-trajectory.json");

        let mut messages = Vec::with_capacity(MESSAGE_COUNT);
        for i in 0..MESSAGE_COUNT {
            messages.push(json!({
                "message_id": format!("m{}", i),
                "role": if i % 2 == 0 { "user" } else { "assistant" },
                "content": "x".repeat(MESSAGE_SIZE),
            }));
        }

        let payload = json!({
            "id": "stress-chat",
            "title": "Stress",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "model": "test/model",
            "mode": "agent",
            "tool_use": "agent",
            "messages": messages,
            "isTitleGenerated": false,
        });

        let start = Instant::now();
        for _ in 0..WRITE_RUNS {
            atomic_write_json(&file_path, &payload).await.unwrap();
        }
        let elapsed = start.elapsed();

        assert!(file_path.exists());
        let content = fs::read_to_string(&file_path).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(
            parsed.get("id").and_then(|v| v.as_str()),
            Some("stress-chat")
        );
        assert_eq!(
            parsed
                .get("messages")
                .and_then(|v| v.as_array())
                .map(|arr| arr.len()),
            Some(MESSAGE_COUNT)
        );

        println!(
            "STRESS_BASELINE trajectory_atomic_write: writes={}, messages={}, msg_size={}, elapsed_ms={}",
            WRITE_RUNS,
            MESSAGE_COUNT,
            MESSAGE_SIZE,
            elapsed.as_millis(),
        );
    }

    #[tokio::test]
    #[ignore]
    async fn stress_trajectory_json_read_parse_baseline() {
        const MESSAGE_COUNT: usize = 1_200;
        const MESSAGE_SIZE: usize = 512;
        const PARSE_RUNS: usize = 400;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("stress-parse.json");

        let mut messages = Vec::with_capacity(MESSAGE_COUNT);
        for i in 0..MESSAGE_COUNT {
            messages.push(json!({
                "message_id": format!("msg-{}", i),
                "role": if i % 3 == 0 { "assistant" } else { "user" },
                "content": "y".repeat(MESSAGE_SIZE),
            }));
        }

        let data = TrajectoryData {
            id: "parse-chat".to_string(),
            title: "Parse".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            model: "test/model".to_string(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            messages,
            extra: serde_json::Map::new(),
        };

        atomic_write_json(&file_path, &data).await.unwrap();
        let content = fs::read_to_string(&file_path).await.unwrap();

        let start = Instant::now();
        for _ in 0..PARSE_RUNS {
            let parsed: TrajectoryData = serde_json::from_str(&content).unwrap();
            assert_eq!(parsed.id, "parse-chat");
            assert_eq!(parsed.messages.len(), MESSAGE_COUNT);
        }
        let elapsed = start.elapsed();

        println!(
            "STRESS_BASELINE trajectory_read_parse: parses={}, messages={}, msg_size={}, elapsed_ms={}",
            PARSE_RUNS,
            MESSAGE_COUNT,
            MESSAGE_SIZE,
            elapsed.as_millis(),
        );
    }
}
