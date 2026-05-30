use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Instant;
use axum::extract::Path as AxumPath;
use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex as AMutex, broadcast};
use tokio::fs;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{debug, info, warn};
use uuid::Uuid;
use refact_chat_api::FrozenRequestPrefix;

use crate::call_validation::{ChatMessage, ChatContent};
use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::files_correction::get_project_dirs;
use crate::subchat::run_subchat_once;
use crate::yaml_configs::customization_registry::get_subagent_config;
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::WorktreeMeta;

pub async fn atomic_write_file(tmp_path: &Path, dest_path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        if dest_path.exists() {
            let backup_extension = format!(
                "{}.replace.{}",
                dest_path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("tmp"),
                Uuid::new_v4().simple()
            );
            let backup_path = dest_path.with_extension(backup_extension);
            fs::rename(dest_path, &backup_path)
                .await
                .map_err(|e| format!("Failed to move existing file aside: {}", e))?;
            match fs::rename(tmp_path, dest_path).await {
                Ok(()) => {
                    let _ = fs::remove_file(&backup_path).await;
                    return Ok(());
                }
                Err(e) => {
                    let _ = fs::rename(&backup_path, dest_path).await;
                    return Err(format!("Failed to rename: {}", e));
                }
            }
        }
    }
    fs::rename(tmp_path, dest_path)
        .await
        .map_err(|e| format!("Failed to rename: {}", e))
}

fn unique_trajectory_tmp_path(file_path: &Path) -> PathBuf {
    let random = Uuid::new_v4().simple().to_string();
    file_path.with_extension(format!("json.tmp.{}", &random[..8]))
}

async fn atomic_write_json_with_tmp_path(
    path: &Path,
    tmp_path: &Path,
    json_result: Result<String, String>,
    write_error_prefix: Option<&str>,
) -> Result<(), String> {
    let result = async {
        let json = json_result?;
        fs::write(tmp_path, &json).await.map_err(|e| {
            write_error_prefix
                .map(|prefix| format!("{}: {}", prefix, e))
                .unwrap_or_else(|| e.to_string())
        })?;
        atomic_write_file(tmp_path, path).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(tmp_path).await;
    }
    result
}

use super::types::{ThreadParams, SessionState, ChatSession};
use super::session::has_displayable_assistant_content;
use super::config::timeouts;
use super::SessionsMap;

const TITLE_GENERATION_SUBAGENT_ID: &str = "title_generation";
const TRAJECTORY_META_TITLE_MAX_CHARS: usize = 120;

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
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_completion_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cache_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cache_creation_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Deserialize)]
struct TrajectoryListData {
    id: String,
    updated_at: String,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

struct TrajectoryListCandidate {
    id: String,
    updated_at: String,
    path: PathBuf,
}

#[derive(Clone)]
pub struct LoadedTrajectory {
    pub source_path: PathBuf,
    pub messages: Vec<ChatMessage>,
    pub thread: ThreadParams,
    pub created_at: String,
    pub updated_at: String,
    pub wake_up_at: Option<chrono::DateTime<chrono::Utc>>,
    pub waiting_for_card_ids: Vec<String>,
    pub auto_approve_editing_tools_present: bool,
    pub auto_approve_dangerous_commands_present: bool,
    pub transition_identity_repaired: bool,
}

#[derive(Clone)]
pub(crate) struct TrajectoryRepairPatch {
    pub chat_id: String,
    pub source_path: PathBuf,
    pub created_at: String,
    pub frozen_request_prefix: Option<FrozenRequestPrefix>,
    pub auto_approve_editing_tools: bool,
    pub auto_approve_dangerous_commands: bool,
}

impl LoadedTrajectory {
    pub(crate) fn repair_patch(&self) -> TrajectoryRepairPatch {
        TrajectoryRepairPatch {
            chat_id: self.thread.id.clone(),
            source_path: self.source_path.clone(),
            created_at: self.created_at.clone(),
            frozen_request_prefix: self.thread.frozen_request_prefix.clone(),
            auto_approve_editing_tools: self.thread.auto_approve_editing_tools,
            auto_approve_dangerous_commands: self.thread.auto_approve_dangerous_commands,
        }
    }
}

pub use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;

fn trajectory_snapshot_from_session(session: &ChatSession) -> TrajectorySnapshot {
    let messages = session
        .messages
        .iter()
        .filter(|message| message.role != "assistant" || has_displayable_assistant_content(message))
        .cloned()
        .collect();

    let mut snapshot = TrajectorySnapshot::from_thread_parts(
        session.chat_id.clone(),
        &session.thread,
        messages,
        session.created_at.clone(),
        session.trajectory_version,
    );
    snapshot.wake_up_at = session.wake_up_at;
    snapshot.waiting_for_card_ids = session.waiting_for_card_ids.clone();
    snapshot
}

pub async fn apply_mode_defaults_to_thread(
    gcx: Arc<GlobalContext>,
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

pub async fn get_trajectories_dir(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    let project_dirs = get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    Ok(workspace_root.join(".refact").join("trajectories"))
}

pub async fn get_global_trajectories_dir(gcx: Arc<GlobalContext>) -> PathBuf {
    let app = AppState::from_gcx(gcx).await;
    let config_dir = app.paths.config_dir.clone();
    config_dir.join("trajectories")
}

pub async fn get_all_trajectories_dirs(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for dir in get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .map(|p| p.join(".refact").join("trajectories"))
    {
        if is_real_dir(&dir).await {
            dirs.push(dir);
        }
    }

    let global_dir = get_global_trajectories_dir(gcx).await;
    if is_real_dir(&global_dir).await {
        dirs.push(global_dir);
    }

    dirs
}

async fn get_all_task_roots(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .map(|p| p.join(".refact").join("tasks"))
        .collect();

    dirs.push(crate::tasks::storage::get_global_tasks_dir(gcx).await);
    dirs
}

async fn get_all_task_roots_from_weak(gcx_weak: &Weak<GlobalContext>) -> Vec<PathBuf> {
    match gcx_weak.upgrade() {
        Some(gcx) => get_all_task_roots(gcx).await,
        None => vec![],
    }
}

pub(crate) async fn list_task_trajectory_dirs(gcx: &Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for tasks_dir in crate::tasks::storage::get_all_tasks_dirs(gcx.clone()).await {
        if !tasks_dir.exists() {
            continue;
        }
        let mut task_entries = match fs::read_dir(&tasks_dir).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        while let Ok(Some(task_entry)) = task_entries.next_entry().await {
            let task_dir = task_entry.path();
            if !is_real_dir(&task_dir).await {
                continue;
            }
            for role in ["planner", "agents"] {
                collect_existing_dirs(task_dir.join("trajectories").join(role), &mut dirs).await;
            }
        }
    }
    dirs
}

async fn collect_existing_dirs(root: PathBuf, dirs: &mut Vec<PathBuf>) {
    let mut pending = vec![root];
    while let Some(dir) = pending.pop() {
        if !is_real_dir(&dir).await {
            continue;
        }
        dirs.push(dir.clone());
        let mut entries = match fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if is_real_dir(&path).await {
                pending.push(path);
            }
        }
    }
}

pub(crate) async fn list_trajectory_dirs(gcx: &Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs = get_all_trajectories_dirs(gcx.clone()).await;
    dirs.extend(list_task_trajectory_dirs(gcx).await);
    dirs
}

async fn get_all_trajectories_dirs_from_weak(gcx_weak: &Weak<GlobalContext>) -> Vec<PathBuf> {
    match gcx_weak.upgrade() {
        Some(gcx) => get_all_trajectories_dirs(gcx).await,
        None => vec![],
    }
}

pub async fn get_buddy_conversations_dir(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    let project_dirs = get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    Ok(workspace_root.join(".refact/buddy/chats/conversations"))
}

fn normalize_system_prompt(system_prompt: Option<String>) -> Option<String> {
    system_prompt.filter(|text| !text.trim().is_empty())
}

fn frozen_prefix_has_system_prompt(prefix: &FrozenRequestPrefix) -> bool {
    prefix
        .system_prompt
        .as_ref()
        .is_some_and(|text| !text.trim().is_empty())
}

fn frozen_prefix_has_tools(prefix: &FrozenRequestPrefix) -> bool {
    prefix.tools_canonical.is_some()
}

fn is_mode_transition_or_handoff(link_type: Option<&str>) -> bool {
    matches!(link_type, Some("mode_transition" | "handoff"))
}

pub fn frozen_prefix_is_complete(prefix: &FrozenRequestPrefix) -> bool {
    frozen_prefix_has_system_prompt(prefix) && frozen_prefix_has_tools(prefix)
}

pub fn first_system_prompt(messages: &[ChatMessage]) -> Option<String> {
    messages.iter().find_map(|message| {
        if message.role == "system" {
            match &message.content {
                ChatContent::SimpleText(text) if !text.trim().is_empty() => Some(text.clone()),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn parsed_rfc3339_utc(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn raw_frozen_prefix_has_tools(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(|value| value.get("tools_canonical"))
        .is_some_and(|value| !value.is_null())
}

fn legacy_incomplete_prefix_from_messages(messages: &[ChatMessage]) -> Option<FrozenRequestPrefix> {
    first_system_prompt(messages)
        .map(|system_prompt| new_legacy_incomplete_frozen_request_prefix(Some(system_prompt)))
}

fn repair_transition_frozen_prefix(
    link_type: Option<&str>,
    messages: &[ChatMessage],
    frozen_request_prefix: Option<FrozenRequestPrefix>,
    raw_frozen_prefix_had_tools: bool,
    trajectory_created_at: Option<&str>,
    provider_state_present: bool,
) -> (Option<FrozenRequestPrefix>, bool) {
    let is_transition = is_mode_transition_or_handoff(link_type);
    let Some(prefix) = frozen_request_prefix else {
        let repaired = is_transition && (provider_state_present || raw_frozen_prefix_had_tools);
        return (legacy_incomplete_prefix_from_messages(messages), repaired);
    };
    if !is_transition {
        return (Some(prefix), false);
    }

    let first_system = first_system_prompt(messages);
    let system_mismatch = match (
        prefix
            .system_prompt
            .as_ref()
            .filter(|text| !text.trim().is_empty()),
        first_system.as_ref(),
    ) {
        (Some(frozen_system), Some(first_system)) => frozen_system != first_system,
        _ => false,
    };
    let missing_system_with_tools =
        !frozen_prefix_has_system_prompt(&prefix) && frozen_prefix_has_tools(&prefix);
    let prefix_created_at_invalid = parsed_rfc3339_utc(&prefix.created_at).is_none();
    let invalid_created_at_with_state =
        prefix_created_at_invalid && (provider_state_present || frozen_prefix_has_tools(&prefix));
    let trajectory_created_at_invalid = trajectory_created_at
        .map(|created_at| parsed_rfc3339_utc(created_at).is_none())
        .unwrap_or(true);
    let invalid_trajectory_created_at_with_state = trajectory_created_at_invalid
        && (provider_state_present || frozen_prefix_has_tools(&prefix));
    // Transition prefixes older than trajectory creation are copied by definition: new transition
    // snapshots start without a prefix, and target-chat lazy prefixes are created after trajectory creation.
    let prefix_created_before_trajectory = trajectory_created_at
        .and_then(parsed_rfc3339_utc)
        .zip(parsed_rfc3339_utc(&prefix.created_at))
        .is_some_and(|(trajectory_created_at, prefix_created_at)| {
            prefix_created_at < trajectory_created_at
        });

    if !(system_mismatch
        || prefix_created_before_trajectory
        || invalid_created_at_with_state
        || invalid_trajectory_created_at_with_state
        || missing_system_with_tools)
    {
        return (Some(prefix), false);
    }

    (
        first_system
            .map(|system_prompt| new_legacy_incomplete_frozen_request_prefix(Some(system_prompt))),
        true,
    )
}

pub fn new_frozen_request_prefix(
    system_prompt: Option<String>,
    tools_canonical: serde_json::Value,
) -> FrozenRequestPrefix {
    FrozenRequestPrefix {
        schema_version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        system_prompt: normalize_system_prompt(system_prompt),
        tools_canonical: Some(tools_canonical),
    }
}

pub(crate) fn new_legacy_incomplete_frozen_request_prefix(
    system_prompt: Option<String>,
) -> FrozenRequestPrefix {
    FrozenRequestPrefix {
        schema_version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        system_prompt: normalize_system_prompt(system_prompt),
        tools_canonical: None,
    }
}

#[cfg(test)]
async fn persist_frozen_prefix(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    frozen_request_prefix: FrozenRequestPrefix,
) -> Result<(), String> {
    validate_trajectory_id(chat_id).map_err(|e| e.message)?;
    let Some(file_path) = find_trajectory_or_buddy_path(gcx.clone(), chat_id).await else {
        return Ok(());
    };

    let content = tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| format!("Failed to read trajectory: {}", e))?;
    let mut trajectory = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|e| format!("Failed to parse trajectory: {}", e))?;

    if trajectory
        .get("frozen_request_prefix")
        .is_some_and(|value| !value.is_null())
    {
        return Ok(());
    }

    trajectory["frozen_request_prefix"] =
        serde_json::to_value(frozen_request_prefix).map_err(|e| e.to_string())?;
    trajectory["updated_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());

    let tmp_path = unique_trajectory_tmp_path(&file_path);
    let json_result = serde_json::to_string_pretty(&trajectory)
        .map_err(|e| format!("Failed to serialize trajectory: {}", e));
    atomic_write_json_with_tmp_path(
        &file_path,
        &tmp_path,
        json_result,
        Some("Failed to write trajectory"),
    )
    .await
}

pub fn ensure_frozen_prefix(
    session: &mut ChatSession,
    system_prompt: Option<String>,
    tools_canonical: Option<serde_json::Value>,
) -> Option<FrozenRequestPrefix> {
    let system_prompt = normalize_system_prompt(system_prompt);
    if system_prompt.is_none() && tools_canonical.is_none() {
        return None;
    }

    match session.thread.frozen_request_prefix.as_mut() {
        Some(prefix) => {
            if frozen_prefix_is_complete(prefix) {
                return None;
            }

            let mut changed = false;
            if !frozen_prefix_has_system_prompt(prefix) {
                if let Some(system_prompt) = system_prompt {
                    prefix.system_prompt = Some(system_prompt);
                    changed = true;
                }
            }
            if !frozen_prefix_has_tools(prefix) {
                if let Some(tools_canonical) = tools_canonical {
                    prefix.tools_canonical = Some(tools_canonical);
                    changed = true;
                }
            }
            if changed {
                let prefix = prefix.clone();
                session.increment_version();
                Some(prefix)
            } else {
                None
            }
        }
        None => {
            let prefix = match tools_canonical {
                Some(tools_canonical) => new_frozen_request_prefix(system_prompt, tools_canonical),
                None => new_legacy_incomplete_frozen_request_prefix(system_prompt),
            };
            session.thread.frozen_request_prefix = Some(prefix.clone());
            session.increment_version();
            Some(prefix)
        }
    }
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

async fn normal_trajectory_candidate_paths(gcx: Arc<GlobalContext>, chat_id: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for dir in get_all_trajectories_dirs(gcx).await {
        if let Some(path) = safe_trajectory_file_in_dir(&dir, chat_id).await {
            paths.push(path);
        }
    }
    paths
}

async fn trajectory_candidate_paths(gcx: Arc<GlobalContext>, chat_id: &str) -> Vec<PathBuf> {
    let mut candidates = normal_trajectory_candidate_paths(gcx.clone(), chat_id).await;
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
                                    candidates.push(direct);
                                }
                                if let Ok(mut sub_entries) = tokio::fs::read_dir(&role_dir).await {
                                    while let Ok(Some(sub_entry)) = sub_entries.next_entry().await {
                                        let sub_path = sub_entry.path();
                                        if sub_path.is_dir() {
                                            let nested = sub_path.join(format!("{}.json", chat_id));
                                            if nested.exists() {
                                                candidates.push(nested);
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
    candidates
}

struct ValidTrajectoryCandidate {
    path: PathBuf,
    content: String,
    json: serde_json::Value,
}

async fn read_valid_trajectory_candidate(
    path: PathBuf,
    chat_id: &str,
) -> Option<ValidTrajectoryCandidate> {
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(e) => {
            warn!(
                "Skipping trajectory candidate {} for chat {}: failed to read: {}",
                path.display(),
                chat_id,
                e
            );
            return None;
        }
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(json) => json,
        Err(e) => {
            warn!(
                "Skipping trajectory candidate {} for chat {}: failed to parse: {}",
                path.display(),
                chat_id,
                e
            );
            return None;
        }
    };
    if !trajectory_candidate_has_minimum_schema(&json, chat_id, &path) {
        return None;
    }
    Some(ValidTrajectoryCandidate {
        path,
        content,
        json,
    })
}

async fn first_valid_trajectory_candidate(
    paths: Vec<PathBuf>,
    chat_id: &str,
) -> Option<ValidTrajectoryCandidate> {
    for path in paths {
        if let Some(candidate) = read_valid_trajectory_candidate(path, chat_id).await {
            return Some(candidate);
        }
    }
    None
}

async fn find_normal_trajectory_file(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<ValidTrajectoryCandidate> {
    validate_trajectory_id(chat_id).ok()?;
    first_valid_trajectory_candidate(
        normal_trajectory_candidate_paths(gcx, chat_id).await,
        chat_id,
    )
    .await
}

async fn find_trajectory_file(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<ValidTrajectoryCandidate> {
    validate_trajectory_id(chat_id).ok()?;
    first_valid_trajectory_candidate(trajectory_candidate_paths(gcx, chat_id).await, chat_id).await
}

async fn find_normal_trajectory_path(gcx: Arc<GlobalContext>, chat_id: &str) -> Option<PathBuf> {
    find_normal_trajectory_file(gcx, chat_id)
        .await
        .map(|candidate| candidate.path)
}

pub async fn find_trajectory_path(gcx: Arc<GlobalContext>, chat_id: &str) -> Option<PathBuf> {
    find_trajectory_file(gcx, chat_id)
        .await
        .map(|candidate| candidate.path)
}

pub async fn find_trajectory_or_buddy_path(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<PathBuf> {
    find_trajectory_or_buddy_file(gcx, chat_id)
        .await
        .map(|candidate| candidate.path)
}

async fn find_trajectory_or_buddy_file(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<ValidTrajectoryCandidate> {
    validate_trajectory_id(chat_id).ok()?;
    let mut candidates = trajectory_candidate_paths(gcx.clone(), chat_id).await;
    if let Ok(buddy_dir) = get_buddy_conversations_dir(gcx).await {
        let buddy_path = buddy_dir.join(format!("{}.json", chat_id));
        if buddy_path.exists() {
            candidates.push(buddy_path);
        }
    }
    first_valid_trajectory_candidate(candidates, chat_id).await
}

async fn ensure_existing_trajectory_file_matches(path: &Path, chat_id: &str) -> Result<(), String> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "Existing trajectory file is not a real file for {}",
                    path.display()
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Failed to check trajectory existence: {}", e)),
    }
    let content = fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read existing trajectory: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse existing trajectory: {}", e))?;
    if trajectory_root_id_matches(&json, chat_id, path) {
        Ok(())
    } else {
        Err(format!(
            "Existing trajectory file id mismatch for {}",
            path.display()
        ))
    }
}

async fn read_existing_trajectory_object(
    path: &Path,
    chat_id: &str,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "Existing trajectory file is not a real file for {}",
                    path.display()
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to check trajectory existence: {}", e)),
    }
    let content = fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read existing trajectory: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse existing trajectory: {}", e))?;
    if !trajectory_root_id_matches(&json, chat_id, path) {
        return Err(format!(
            "Existing trajectory file id mismatch for {}",
            path.display()
        ));
    }
    json.as_object()
        .cloned()
        .ok_or_else(|| {
            format!(
                "Existing trajectory JSON root must be an object for {}",
                path.display()
            )
        })
        .map(Some)
}

fn is_known_trajectory_top_level_key(key: &str) -> bool {
    matches!(
        key,
        "id" | "title"
            | "model"
            | "mode"
            | "tool_use"
            | "messages"
            | "created_at"
            | "updated_at"
            | "boost_reasoning"
            | "checkpoints_enabled"
            | "context_tokens_cap"
            | "include_project_info"
            | "isTitleGenerated"
            | "auto_approve_editing_tools"
            | "auto_approve_dangerous_commands"
            | "autonomous_no_confirm"
            | "reasoning_effort"
            | "thinking_budget"
            | "temperature"
            | "frequency_penalty"
            | "max_tokens"
            | "previous_response_id"
            | "parallel_tool_calls"
            | "active_skill"
            | "auto_enrichment_enabled"
            | "buddy_meta"
            | "auto_compact_enabled"
            | "frozen_request_prefix"
            | "claude_code_identity"
            | "reactive_compact_attempts"
            | "wake_up_at"
            | "waiting_for_card_ids"
            | "worktree"
            | "parent_id"
            | "link_type"
            | "root_chat_id"
            | "task_meta"
            | "browser_meta"
    )
}

fn preserve_existing_trajectory_metadata(
    trajectory: &mut serde_json::Value,
    existing: Option<serde_json::Map<String, serde_json::Value>>,
) {
    let Some(existing) = existing else {
        return;
    };
    let Some(trajectory_object) = trajectory.as_object_mut() else {
        return;
    };
    for (key, value) in existing {
        let preserve_browser_meta = key == "browser_meta" && !trajectory_object.contains_key(&key);
        let preserve_unknown =
            !is_known_trajectory_top_level_key(&key) && !trajectory_object.contains_key(&key);
        if preserve_browser_meta || preserve_unknown {
            trajectory_object.insert(key, value);
        }
    }
}

async fn resolve_trajectory_data_save_path(
    gcx: Arc<GlobalContext>,
    id: &str,
    data: &TrajectoryData,
) -> Result<PathBuf, String> {
    validate_trajectory_id(id).map_err(|e| e.message)?;
    if data.id != id {
        return Err("ID mismatch".to_string());
    }

    let task_meta_value = data.extra.get("task_meta").filter(|value| !value.is_null());
    let buddy_meta_present = data
        .extra
        .get("buddy_meta")
        .is_some_and(|value| !value.is_null());

    if task_meta_value.is_some() && buddy_meta_present {
        return Err("Trajectory cannot contain both task_meta and buddy_meta".to_string());
    }

    if let Some(value) = task_meta_value {
        let task_meta = serde_json::from_value::<super::types::TaskMeta>(value.clone())
            .map_err(|e| format!("Invalid task_meta: {}", e))?;
        let task_dir =
            crate::tasks::storage::find_task_dir(gcx.clone(), &task_meta.task_id).await?;
        let traj_dir = crate::tasks::storage::get_task_trajectory_dir(
            &task_dir,
            &task_meta.role,
            task_meta.agent_id.as_deref(),
        );
        fs::create_dir_all(&traj_dir)
            .await
            .map_err(|e| format!("Failed to create task trajectories dir: {}", e))?;
        return Ok(traj_dir.join(format!("{}.json", id)));
    }

    if buddy_meta_present {
        let buddy_dir = get_buddy_conversations_dir(gcx.clone()).await?;
        fs::create_dir_all(&buddy_dir)
            .await
            .map_err(|e| format!("Failed to create buddy conversations dir: {}", e))?;
        return Ok(buddy_dir.join(format!("{}.json", id)));
    }

    if let Some(candidate) = find_normal_trajectory_file(gcx.clone(), id).await {
        return Ok(candidate.path);
    }
    let trajectories_dir = get_trajectories_dir(gcx.clone()).await?;
    safe_new_trajectory_file_in_dir(&trajectories_dir, id).await
}

async fn title_generation_backing_file_matches(file_path: &Path, chat_id: &str) -> bool {
    let content = match fs::read_to_string(file_path).await {
        Ok(content) => content,
        Err(e) => {
            warn!(
                "Skipping title update for {}: failed to read backing trajectory: {}",
                file_path.display(),
                e
            );
            return false;
        }
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(json) => json,
        Err(e) => {
            warn!(
                "Skipping title update for {}: failed to parse backing trajectory: {}",
                file_path.display(),
                e
            );
            return false;
        }
    };
    trajectory_root_id_matches(&json, chat_id, file_path)
}

fn repaired_created_at(raw_created_at: Option<&str>, transition_identity_repaired: bool) -> String {
    if transition_identity_repaired {
        if let Some(created_at) = raw_created_at {
            if parsed_rfc3339_utc(created_at).is_some() {
                return created_at.to_string();
            }
        }
        return chrono::Utc::now().to_rfc3339();
    }

    raw_created_at
        .map(ToString::to_string)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339())
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
    app: AppState,
    requested_source_root: Option<&Path>,
) -> Result<WorktreeService, String> {
    let cache_dir = app.paths.cache_dir.clone();
    let project_dirs = get_project_dirs(app.gcx.clone()).await;
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
    app: AppState,
    chat_id: &str,
    worktree: WorktreeMeta,
) -> Option<WorktreeMeta> {
    let service =
        match worktree_service_from_gcx(app.clone(), Some(&worktree.source_workspace_root)).await {
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
            debug!(
                "Ignoring untrusted trajectory worktree metadata for chat {}: {}",
                chat_id, e
            );
            None
        }
    }
}

async fn validate_loaded_legacy_task_agent_worktree(
    app: AppState,
    chat_id: &str,
    worktree: WorktreeMeta,
) -> Option<WorktreeMeta> {
    let service =
        match worktree_service_from_gcx(app.clone(), Some(&worktree.source_workspace_root)).await {
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
    gcx: Arc<GlobalContext>,
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

fn trajectory_root_id_matches(t: &serde_json::Value, chat_id: &str, path: &Path) -> bool {
    match t.get("id").and_then(|value| value.as_str()) {
        Some(id) if id == chat_id => true,
        Some(id) => {
            warn!(
                "Rejecting trajectory {}: JSON id mismatch, expected {}, found {}",
                path.display(),
                chat_id,
                id
            );
            false
        }
        None => {
            warn!(
                "Rejecting trajectory {}: missing or non-string JSON id for requested chat {}",
                path.display(),
                chat_id
            );
            false
        }
    }
}

fn trajectory_candidate_has_minimum_schema(
    t: &serde_json::Value,
    chat_id: &str,
    path: &Path,
) -> bool {
    let Some(root) = t.as_object() else {
        warn!(
            "Rejecting trajectory {}: JSON root must be an object for requested chat {}",
            path.display(),
            chat_id
        );
        return false;
    };
    if !trajectory_root_id_matches(t, chat_id, path) {
        return false;
    }
    if root
        .get("messages")
        .is_some_and(|messages| messages.is_array())
    {
        return true;
    }
    warn!(
        "Rejecting trajectory {}: missing or non-array JSON messages for requested chat {}",
        path.display(),
        chat_id
    );
    false
}

fn trajectory_path_stem_matches_id(path: &Path, id: &str) -> bool {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    if stem == id {
        return true;
    }
    warn!(
        "Ignoring trajectory {} in list: filename id mismatch, expected {}, found {}",
        path.display(),
        stem,
        id
    );
    false
}

pub async fn load_trajectory_for_chat(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<LoadedTrajectory> {
    let app = AppState::from_gcx(gcx.clone()).await;
    let candidate = find_trajectory_or_buddy_file(gcx.clone(), chat_id).await?;
    let traj_path = candidate.path;
    let t = candidate.json;

    let mut messages: Vec<ChatMessage> = t
        .get("messages")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    fix_tool_call_indexes(&mut messages);

    for (message_index, msg) in messages.iter_mut().enumerate() {
        if msg.message_id.is_empty() {
            let role = msg.role.clone();
            let content = msg.content.content_text_only();
            let source = msg
                .extra
                .get("event")
                .and_then(|event| event.get("source"))
                .and_then(|source| source.as_str())
                .unwrap_or_default();
            msg.message_id = format!(
                "legacy:{}:{:x}",
                role,
                md5::compute(format!("{message_index}\n{role}\n{source}\n{content}").as_bytes())
            );
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
        validate_loaded_worktree_strict(app.clone(), chat_id, candidate).await
    } else if let Some(candidate) =
        synthesize_legacy_task_agent_worktree(gcx.clone(), chat_id, task_meta.as_ref()).await
    {
        validate_loaded_legacy_task_agent_worktree(app.clone(), chat_id, candidate).await
    } else {
        None
    };

    let wake_up_at = t
        .get("wake_up_at")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let waiting_for_card_ids: Vec<String> = t
        .get("waiting_for_card_ids")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let parent_id = t
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let link_type = t
        .get("link_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let root_chat_id = t
        .get("root_chat_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let previous_response_id = t
        .get("previous_response_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let frozen_request_prefix_value = t.get("frozen_request_prefix").filter(|v| !v.is_null());
    let loaded_frozen_request_prefix =
        frozen_request_prefix_value.and_then(|v| serde_json::from_value(v.clone()).ok());
    let claude_code_identity_value = t.get("claude_code_identity").filter(|v| !v.is_null());
    let provider_state_present =
        previous_response_id.is_some() || claude_code_identity_value.is_some();
    let (frozen_request_prefix, transition_prefix_repaired) = repair_transition_frozen_prefix(
        link_type.as_deref(),
        &messages,
        loaded_frozen_request_prefix,
        raw_frozen_prefix_has_tools(frozen_request_prefix_value),
        t.get("created_at").and_then(|v| v.as_str()),
        provider_state_present,
    );
    let claude_code_identity = if transition_prefix_repaired {
        None
    } else {
        claude_code_identity_value.and_then(|v| serde_json::from_value(v.clone()).ok())
    };
    let previous_response_id = if transition_prefix_repaired {
        None
    } else {
        previous_response_id
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
        autonomous_no_confirm: t
            .get("autonomous_no_confirm")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        task_meta,
        worktree,
        parent_id,
        link_type,
        root_chat_id,
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

        previous_response_id,

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

        auto_compact_enabled: t.get("auto_compact_enabled").and_then(|v| v.as_bool()),
        frozen_request_prefix,
        claude_code_identity,
        reactive_compact_attempts: t
            .get("reactive_compact_attempts")
            .and_then(|v| v.as_u64())
            .map(|n| if n > 2 { 1 } else { n as usize }),
    };

    let auto_approve_editing_tools_present = t
        .get("auto_approve_editing_tools")
        .and_then(|v| v.as_bool())
        .is_some();
    let auto_approve_dangerous_commands_present = t
        .get("auto_approve_dangerous_commands")
        .and_then(|v| v.as_bool())
        .is_some();

    let created_at = repaired_created_at(
        t.get("created_at").and_then(|v| v.as_str()),
        transition_prefix_repaired,
    );

    let updated_at = t
        .get("updated_at")
        .and_then(|v| v.as_str())
        .unwrap_or(&created_at)
        .to_string();

    Some(LoadedTrajectory {
        source_path: traj_path,
        messages,
        thread,
        created_at,
        updated_at,
        wake_up_at,
        waiting_for_card_ids,
        auto_approve_editing_tools_present,
        auto_approve_dangerous_commands_present,
        transition_identity_repaired: transition_prefix_repaired,
    })
}

pub async fn save_initial_planner_trajectory(
    gcx: Arc<GlobalContext>,
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
        ..Default::default()
    };

    let task_meta = super::types::TaskMeta {
        task_id: task_id.to_string(),
        role: "planner".to_string(),
        agent_id: None,
        card_id: None,
        planner_chat_id: Some(chat_id.to_string()),
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
        autonomous_no_confirm: false,
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
        auto_compact_enabled: None,
        frozen_request_prefix: None,
        claude_code_identity: None,
        reactive_compact_attempts: None,
        wake_up_at: None,
        waiting_for_card_ids: Vec::new(),
    };

    save_trajectory_snapshot(gcx, snapshot).await
}

pub async fn save_trajectory_as(
    gcx: Arc<GlobalContext>,
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
        autonomous_no_confirm: thread.autonomous_no_confirm,
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
        auto_compact_enabled: thread.auto_compact_enabled,
        frozen_request_prefix: thread.frozen_request_prefix.clone(),
        claude_code_identity: thread.claude_code_identity.clone(),
        reactive_compact_attempts: thread.reactive_compact_attempts,
        wake_up_at: None,
        waiting_for_card_ids: Vec::new(),
    };
    if let Err(e) = save_trajectory_snapshot(gcx, snapshot).await {
        warn!("Failed to save trajectory: {}", e);
    }
}

pub async fn save_trajectory_snapshot(
    gcx: Arc<GlobalContext>,
    snapshot: TrajectorySnapshot,
) -> Result<(), String> {
    validate_trajectory_id(&snapshot.chat_id).map_err(|e| e.message)?;
    let app = AppState::from_gcx(gcx.clone()).await;
    let existing_no_meta_path = if snapshot.task_meta.is_none() && snapshot.buddy_meta.is_none() {
        find_normal_trajectory_path(gcx.clone(), &snapshot.chat_id).await
    } else {
        None
    };
    if snapshot.messages.is_empty()
        && snapshot.task_meta.is_none()
        && snapshot.buddy_meta.is_none()
        && snapshot.frozen_request_prefix.is_none()
        && existing_no_meta_path.is_none()
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
        "autonomous_no_confirm": snapshot.autonomous_no_confirm,
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
    if let Some(auto_compact) = snapshot.auto_compact_enabled {
        trajectory["auto_compact_enabled"] = json!(auto_compact);
    }
    if let Some(ref frozen_request_prefix) = snapshot.frozen_request_prefix {
        trajectory["frozen_request_prefix"] =
            serde_json::to_value(frozen_request_prefix).unwrap_or_default();
    }
    if let Some(ref claude_code_identity) = snapshot.claude_code_identity {
        trajectory["claude_code_identity"] =
            serde_json::to_value(claude_code_identity).unwrap_or_default();
    }
    if let Some(reactive_compact_attempts) = snapshot.reactive_compact_attempts {
        trajectory["reactive_compact_attempts"] = json!(reactive_compact_attempts);
    }
    if let Some(wake_up_at) = snapshot.wake_up_at {
        trajectory["wake_up_at"] = json!(wake_up_at);
    }
    if !snapshot.waiting_for_card_ids.is_empty() {
        trajectory["waiting_for_card_ids"] = json!(snapshot.waiting_for_card_ids);
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
    } else if let Some(path) = existing_no_meta_path {
        path
    } else {
        let trajectories_dir = get_trajectories_dir(gcx.clone()).await?;
        safe_new_trajectory_file_in_dir(&trajectories_dir, &snapshot.chat_id).await?
    };
    let existing_trajectory =
        read_existing_trajectory_object(&file_path, &snapshot.chat_id).await?;

    let updated_at = chrono::Utc::now().to_rfc3339();
    trajectory["updated_at"] = serde_json::Value::String(updated_at.clone());
    preserve_existing_trajectory_metadata(&mut trajectory, existing_trajectory);

    let tmp_path = unique_trajectory_tmp_path(&file_path);
    let json_result = serde_json::to_string_pretty(&trajectory)
        .map_err(|e| format!("Failed to serialize trajectory: {}", e));
    atomic_write_json_with_tmp_path(
        &file_path,
        &tmp_path,
        json_result,
        Some("Failed to write trajectory"),
    )
    .await?;

    info!(
        "Saved trajectory for chat {} ({} messages) to {:?}",
        snapshot.chat_id,
        snapshot.messages.len(),
        file_path
    );

    let vec_db = app.workspace.vec_db.clone();
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
        let sessions = app.chat.sessions.clone();
        let (session_state, session_error) =
            get_session_state_for_chat(&sessions, &snapshot.chat_id).await;
        let (total_lines_added, total_lines_removed) =
            calculate_line_changes_from_chat_messages(&snapshot.messages);
        let (tasks_total, tasks_done, tasks_failed) =
            calculate_task_progress_from_chat_messages(&snapshot.messages);
        let token_totals = calculate_token_totals_from_chat_messages(&snapshot.messages);
        let tx = &app.chat.trajectory_events_tx;
        {
            let event = TrajectoryEvent {
                event_type: "updated".to_string(),
                id: snapshot.chat_id.clone(),
                updated_at: Some(updated_at),
                title: Some(trajectory_meta_title(&snapshot.title)),
                is_title_generated: Some(snapshot.is_title_generated),
                session_state: Some(session_state),
                error: session_error,
                message_count: Some(snapshot.messages.len()),
                parent_id: snapshot.parent_id.clone(),
                link_type: snapshot.link_type.clone(),
                root_chat_id: Some(effective_root),
                task_id: None,
                task_role: None,
                agent_id: None,
                card_id: None,
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
            let trajectories_dir = file_path
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| "Trajectory file path has no parent directory".to_string())?;
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

pub async fn try_save_trajectory(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> Result<bool, String> {
    let snapshot = {
        let session = session_arc.lock().await;
        if !session.trajectory_dirty {
            return Ok(true);
        }
        trajectory_snapshot_from_session(&session)
    };

    let saved_version = snapshot.version;
    let chat_id = snapshot.chat_id.clone();

    save_trajectory_snapshot(app.gcx.clone(), snapshot)
        .await
        .map_err(|e| format!("Failed to save trajectory for {}: {}", chat_id, e))?;

    let mut session = session_arc.lock().await;
    if session.trajectory_version == saved_version {
        session.trajectory_dirty = false;
    }
    Ok(!session.trajectory_dirty)
}

pub async fn maybe_save_trajectory(app: AppState, session_arc: Arc<AMutex<ChatSession>>) {
    if let Err(e) = try_save_trajectory(app, session_arc).await {
        warn!("{}", e);
    }
}

pub(crate) async fn persist_loaded_trajectory_repair_raw(
    repair: &TrajectoryRepairPatch,
) -> Result<(), String> {
    let chat_id = &repair.chat_id;
    validate_trajectory_id(chat_id).map_err(|e| e.message)?;
    let file_path = &repair.source_path;
    let content = tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| format!("Failed to read trajectory: {}", e))?;
    let mut trajectory = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|e| format!("Failed to parse trajectory: {}", e))?;
    let trajectory_object = trajectory
        .as_object_mut()
        .ok_or_else(|| "Trajectory JSON root must be an object".to_string())?;
    let current_chat_id = trajectory_object.get("id").and_then(|value| value.as_str());
    if current_chat_id != Some(chat_id.as_str()) {
        return Err(format!(
            "Trajectory source id mismatch for repair: expected {}, found {}",
            chat_id,
            current_chat_id.unwrap_or("<missing>")
        ));
    }

    trajectory_object.remove("previous_response_id");
    trajectory_object.remove("claude_code_identity");
    match &repair.frozen_request_prefix {
        Some(frozen_request_prefix) => {
            trajectory_object.insert(
                "frozen_request_prefix".to_string(),
                serde_json::to_value(frozen_request_prefix).map_err(|e| e.to_string())?,
            );
        }
        None => {
            trajectory_object.remove("frozen_request_prefix");
        }
    }
    trajectory_object.insert(
        "created_at".to_string(),
        serde_json::Value::String(repair.created_at.clone()),
    );
    trajectory_object.insert(
        "auto_approve_editing_tools".to_string(),
        json!(repair.auto_approve_editing_tools),
    );
    trajectory_object.insert(
        "auto_approve_dangerous_commands".to_string(),
        json!(repair.auto_approve_dangerous_commands),
    );
    trajectory_object.insert(
        "updated_at".to_string(),
        serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
    );

    let tmp_path = unique_trajectory_tmp_path(file_path);
    let json_result = serde_json::to_string_pretty(&trajectory)
        .map_err(|e| format!("Failed to serialize trajectory: {}", e));
    atomic_write_json_with_tmp_path(
        file_path,
        &tmp_path,
        json_result,
        Some("Failed to write trajectory"),
    )
    .await
}

async fn persist_loaded_trajectory_repair(gcx: Arc<GlobalContext>, mut loaded: LoadedTrajectory) {
    let chat_id = loaded.thread.id.clone();
    apply_mode_defaults_to_thread(
        gcx.clone(),
        &mut loaded.thread,
        loaded.auto_approve_editing_tools_present,
        loaded.auto_approve_dangerous_commands_present,
    )
    .await;
    if let Err(e) = persist_loaded_trajectory_repair_raw(&loaded.repair_patch()).await {
        warn!(
            "Failed to persist repaired trajectory for {}: {}",
            chat_id, e
        );
    }
}

pub async fn check_external_reload_pending(
    gcx: Arc<GlobalContext>,
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
        let transition_identity_repaired = loaded.transition_identity_repaired;
        apply_mode_defaults_to_thread(
            gcx.clone(),
            &mut loaded.thread,
            loaded.auto_approve_editing_tools_present,
            loaded.auto_approve_dangerous_commands_present,
        )
        .await;
        let repair_patch = loaded.repair_patch();
        let repaired_version = {
            let mut session = session_arc.lock().await;
            if session.runtime.state == SessionState::Idle && !session.trajectory_dirty {
                info!("Applying pending external reload for {}", chat_id);
                session.messages = loaded.messages;
                session.thread = loaded.thread;
                session.reset_compaction_runtime_state();
                session.created_at = loaded.created_at;
                session.wake_up_at = loaded.wake_up_at;
                session.waiting_for_card_ids = loaded.waiting_for_card_ids;
                session.external_reload_pending = false;
                if transition_identity_repaired {
                    session.increment_version();
                }
                let snapshot = session.snapshot();
                session.emit(snapshot);
                transition_identity_repaired.then_some(session.trajectory_version)
            } else {
                None
            }
        };
        if let Some(repaired_version) = repaired_version {
            if let Err(e) = persist_loaded_trajectory_repair_raw(&repair_patch).await {
                warn!(
                    "Failed to persist repaired trajectory for {}: {}",
                    chat_id, e
                );
            } else {
                let mut session = session_arc.lock().await;
                if session.trajectory_version == repaired_version {
                    session.trajectory_dirty = false;
                }
            }
        }
    }
}

async fn process_trajectory_change(gcx: Arc<GlobalContext>, chat_id: &str, is_remove: bool) {
    let app = AppState::from_gcx(gcx.clone()).await;
    let sessions = app.chat.sessions.clone();
    let mut loaded = if is_remove {
        None
    } else {
        load_trajectory_for_chat(gcx.clone(), chat_id).await
    };

    if is_remove {
        let tx = &app.chat.trajectory_events_tx;
        {
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
                task_id: None,
                task_role: None,
                agent_id: None,
                card_id: None,
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
        ) = if let Some(t) = loaded.as_ref() {
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
                Some(t.updated_at.clone()),
                Some(trajectory_meta_title(&t.thread.title)),
                Some(t.thread.is_title_generated),
                Some(t.messages.len()),
                t.thread.parent_id.clone(),
                t.thread.link_type.clone(),
                Some(effective_root),
                Some(t.thread.model.clone()),
                Some(t.thread.mode.clone()),
                t.thread.worktree.clone(),
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
        let tx = &app.chat.trajectory_events_tx;
        {
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
                task_id: None,
                task_role: None,
                agent_id: None,
                card_id: None,
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

    let sessions = app.chat.sessions.clone();
    let session_arc = {
        let sessions_read = sessions.read().await;
        sessions_read.get(chat_id).cloned()
    };

    let Some(session_arc) = session_arc else {
        if let Some(loaded) = loaded.take() {
            if loaded.transition_identity_repaired {
                persist_loaded_trajectory_repair(gcx, loaded).await;
            }
        }
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

    if let Some(mut loaded) = loaded.take() {
        let transition_identity_repaired = loaded.transition_identity_repaired;
        apply_mode_defaults_to_thread(
            gcx.clone(),
            &mut loaded.thread,
            loaded.auto_approve_editing_tools_present,
            loaded.auto_approve_dangerous_commands_present,
        )
        .await;
        let repair_patch = loaded.repair_patch();
        let repaired_version = {
            let mut session = session_arc.lock().await;
            if session.runtime.state != SessionState::Idle || session.trajectory_dirty {
                session.external_reload_pending = true;
                return;
            }
            info!("Reloading trajectory for {} from external change", chat_id);
            session.messages = loaded.messages;
            session.thread = loaded.thread;
            session.reset_compaction_runtime_state();
            session.created_at = loaded.created_at;
            session.wake_up_at = loaded.wake_up_at;
            session.waiting_for_card_ids = loaded.waiting_for_card_ids;
            session.external_reload_pending = false;
            if transition_identity_repaired {
                session.increment_version();
            }
            let snapshot = session.snapshot();
            session.emit(snapshot);
            transition_identity_repaired.then_some(session.trajectory_version)
        };
        if let Some(repaired_version) = repaired_version {
            if let Err(e) = persist_loaded_trajectory_repair_raw(&repair_patch).await {
                warn!(
                    "Failed to persist repaired trajectory for {}: {}",
                    chat_id, e
                );
            } else {
                let mut session = session_arc.lock().await;
                if session.trajectory_version == repaired_version {
                    session.trajectory_dirty = false;
                }
            }
        }
    }
}

fn task_trajectory_context_from_path(
    path: &Path,
    task_roots: &[PathBuf],
) -> Option<(String, String, Option<String>)> {
    for root in task_roots {
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let parts: Vec<String> = relative
            .components()
            .filter_map(|component| component.as_os_str().to_str().map(|s| s.to_string()))
            .collect();
        if parts.len() < 4 || parts.get(1).map(|s| s.as_str()) != Some("trajectories") {
            continue;
        }
        let role = parts[2].as_str();
        if role != "planner" && role != "agents" {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let agent_id = if role == "agents" && parts.len() >= 5 {
            Some(parts[3].clone())
        } else {
            None
        };
        return Some((parts[0].clone(), role.to_string(), agent_id));
    }
    None
}

fn is_under_task_root(path: &Path, task_roots: &[PathBuf]) -> bool {
    task_roots.iter().any(|root| path.starts_with(root))
}

fn should_dispatch_trajectory_path(path: &Path, task_roots: &[PathBuf]) -> bool {
    if path.extension().and_then(|e| e.to_str()) != Some("json") {
        return false;
    }
    if !is_under_task_root(path, task_roots) {
        return true;
    }
    task_trajectory_context_from_path(path, task_roots).is_some()
}

async fn collect_task_trajectory_chat_ids_under_path(
    path: &Path,
    task_roots: &[PathBuf],
) -> Vec<String> {
    if !is_under_task_root(path, task_roots) {
        return Vec::new();
    }

    let mut chat_ids = Vec::new();
    let mut pending = vec![path.to_path_buf()];
    while let Some(path) = pending.pop() {
        if should_dispatch_trajectory_path(&path, task_roots) {
            if let Some(chat_id) = path.file_stem().and_then(|s| s.to_str()) {
                chat_ids.push(chat_id.to_string());
            }
            continue;
        }

        if !is_real_dir(&path).await {
            continue;
        }

        let mut entries = match fs::read_dir(&path).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            pending.push(entry.path());
        }
    }
    chat_ids
}

enum TrajectoryWatcherMessage {
    Trajectory { chat_id: String, is_remove: bool },
    ScanPath(PathBuf),
}

pub fn start_trajectory_watcher(gcx: Arc<GlobalContext>) {
    let gcx_weak = Arc::downgrade(&gcx);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TrajectoryWatcherMessage>();

    tokio::spawn(async move {
        let trajectories_dirs = get_all_trajectories_dirs_from_weak(&gcx_weak).await;
        let task_roots = get_all_task_roots_from_weak(&gcx_weak).await;
        if trajectories_dirs.is_empty() && task_roots.is_empty() {
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
        for dir in &task_roots {
            if let Err(e) = tokio::fs::create_dir_all(dir).await {
                warn!("Failed to create tasks dir {:?} for watcher: {}", dir, e);
            }
        }

        let tx_clone = tx.clone();
        let task_roots_for_callback = task_roots.clone();
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
                    if should_dispatch_trajectory_path(&path, &task_roots_for_callback) {
                        if let Some(chat_id) = path.file_stem().and_then(|s| s.to_str()) {
                            let _ = tx_clone.send(TrajectoryWatcherMessage::Trajectory {
                                chat_id: chat_id.to_string(),
                                is_remove,
                            });
                        }
                    } else if !is_remove && is_under_task_root(&path, &task_roots_for_callback) {
                        let _ = tx_clone.send(TrajectoryWatcherMessage::ScanPath(path));
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
            for dir in &task_roots {
                if let Err(e) = w.watch(dir, RecursiveMode::Recursive) {
                    warn!("Failed to watch tasks dir {:?}: {}", dir, e);
                }
            }
        }
        info!(
            "Trajectory watcher started for {} trajectory directories and {} task roots",
            trajectories_dirs.len(),
            task_roots.len()
        );

        let mut pending: std::collections::HashMap<String, (Instant, bool)> =
            std::collections::HashMap::new();
        let mut pending_scans: std::collections::HashMap<PathBuf, Instant> =
            std::collections::HashMap::new();
        let debounce = timeouts().watcher_debounce;

        loop {
            let timeout = if pending.is_empty() && pending_scans.is_empty() {
                timeouts().watcher_idle
            } else {
                timeouts().watcher_poll
            };

            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(TrajectoryWatcherMessage::Trajectory { chat_id, is_remove }) => {
                            pending.insert(chat_id, (Instant::now(), is_remove));
                        }
                        Some(TrajectoryWatcherMessage::ScanPath(path)) => {
                            pending_scans.insert(path, Instant::now());
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
            let ready_scans: Vec<PathBuf> = pending_scans
                .iter()
                .filter(|(_, t)| now.duration_since(**t) >= debounce)
                .map(|(path, _)| path.clone())
                .collect();

            for path in ready_scans {
                pending_scans.remove(&path);
                for chat_id in collect_task_trajectory_chat_ids_under_path(&path, &task_roots).await
                {
                    pending.insert(chat_id, (Instant::now(), false));
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
    let tmp_path = unique_trajectory_tmp_path(path);
    let json_result = serde_json::to_string(data).map_err(|e| e.to_string());
    atomic_write_json_with_tmp_path(path, &tmp_path, json_result, None).await
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

fn json_message_is_ui_only(msg: &serde_json::Value) -> bool {
    msg.get("_ui_only").and_then(|v| v.as_bool()) == Some(true)
        || msg
            .get("extra")
            .and_then(|extra| extra.get("_ui_only"))
            .and_then(|v| v.as_bool())
            == Some(true)
}

fn extract_first_user_message(messages: &[serde_json::Value]) -> Option<String> {
    for msg in messages {
        if json_message_is_ui_only(msg) {
            continue;
        }
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
        if json_message_is_ui_only(msg) {
            continue;
        }
        if role == "error" {
            continue;
        }
        if role == "system"
            || role == "tool"
            || role == "context_file"
            || role == "cd_instruction"
            || role == "plan"
            || role == "event"
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

fn truncate_text_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }
    text.chars().take(max_chars - 3).collect::<String>() + "..."
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
    truncate_text_chars(&cleaned, 60)
}

pub(crate) fn trajectory_meta_title(title: &str) -> String {
    let cleaned = title.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_text_chars(&cleaned, TRAJECTORY_META_TITLE_MAX_CHARS)
}

pub(crate) fn task_context_from_task_meta(
    task_meta: Option<&super::types::TaskMeta>,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    task_meta
        .map(|meta| {
            (
                Some(meta.task_id.clone()),
                Some(meta.role.clone()),
                meta.agent_id.clone(),
                meta.card_id.clone(),
            )
        })
        .unwrap_or((None, None, None, None))
}

async fn generate_title_llm(
    gcx: Arc<GlobalContext>,
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
    gcx: Arc<GlobalContext>,
    id: String,
    messages: Vec<serde_json::Value>,
    trajectories_dir: PathBuf,
) {
    tokio::spawn(async move {
        let app = AppState::from_gcx(gcx.clone()).await;
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
        let sessions = app.chat.sessions.clone();
        let file_path = trajectories_dir.join(format!("{}.json", id));
        let maybe_session_arc = {
            let sessions_read = sessions.read().await;
            sessions_read.get(&id).cloned()
        };
        if let Some(session_arc) = maybe_session_arc {
            if !title_generation_backing_file_matches(&file_path, &id).await {
                return;
            }
            let mut session = session_arc.lock().await;
            if session.thread.is_title_generated {
                info!("Title already generated for {}, skipping", id);
                return;
            }
            session.set_title(title.clone(), true);
            drop(session);
            maybe_save_trajectory(app.clone(), session_arc).await;
            info!("Updated session {} with generated title: {}", id, title);
            return;
        }
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
        if data.id != id {
            warn!(
                "Skipping title update for {}: JSON id mismatch, found {}",
                file_path.display(),
                data.id
            );
            return;
        }
        let already_generated = data
            .extra
            .get("isTitleGenerated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if already_generated {
            info!("Title already generated for {}, skipping", id);
            return;
        }
        let updated_at = chrono::Utc::now().to_rfc3339();
        data.title = title.clone();
        data.updated_at = updated_at.clone();
        data.extra
            .insert("isTitleGenerated".to_string(), serde_json::json!(true));
        if let Err(e) = atomic_write_json(&file_path, &data).await {
            warn!("Failed to write trajectory with generated title: {}", e);
            return;
        }
        info!("Updated trajectory {} with generated title: {}", id, title);
        let (session_state, session_error) = get_session_state_for_chat(&sessions, &id).await;
        let worktree = if let Some(candidate) = trajectory_worktree_from_extra(&data.extra) {
            validate_loaded_worktree_strict(app.clone(), &id, candidate).await
        } else {
            None
        };
        let event = TrajectoryEvent {
            event_type: "updated".to_string(),
            id: id.clone(),
            updated_at: Some(updated_at),
            title: Some(trajectory_meta_title(&title)),
            is_title_generated: Some(true),
            session_state: Some(session_state),
            error: session_error,
            message_count: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            task_id: None,
            task_role: None,
            agent_id: None,
            card_id: None,
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
        let tx = &app.chat.trajectory_events_tx;
        {
            let _ = tx.send(event);
        }
    });
}

fn spawn_task_name_generation_task(
    gcx: Arc<GlobalContext>,
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
        title: trajectory_meta_title(&data.title),
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

async fn trajectory_data_to_meta_validated(app: AppState, data: &TrajectoryData) -> TrajectoryMeta {
    let mut meta = trajectory_data_to_meta(data);
    if let Some(worktree) = trajectory_worktree_from_extra(&data.extra) {
        meta.worktree = validate_loaded_worktree_strict(app, &data.id, worktree).await;
    }
    meta
}

fn apply_task_trajectory_context(path: &Path, task_roots: &[PathBuf], meta: &mut TrajectoryMeta) {
    if let Some((task_id, role, agent_id)) = task_trajectory_context_from_path(path, task_roots) {
        if meta.task_id.is_none() {
            meta.task_id = Some(task_id);
        }
        if meta.task_role.is_none() {
            meta.task_role = Some(role);
        }
        if meta.agent_id.is_none() {
            meta.agent_id = agent_id;
        }
    }
}

fn trajectory_list_candidate_matches_hydrated_data(
    candidate: &TrajectoryListCandidate,
    data: &TrajectoryData,
) -> bool {
    if data.id == candidate.id && trajectory_path_stem_matches_id(&candidate.path, &data.id) {
        return true;
    }
    warn!(
        "Ignoring trajectory {} during hydration: expected id {}, found {}",
        candidate.path.display(),
        candidate.id,
        data.id
    );
    false
}

async fn hydrate_trajectory_list_candidate(
    app: AppState,
    candidate: &TrajectoryListCandidate,
    task_roots: &[PathBuf],
) -> Option<TrajectoryMeta> {
    let content = fs::read_to_string(&candidate.path).await.ok()?;
    let data = serde_json::from_str::<TrajectoryData>(&content).ok()?;
    if !trajectory_list_candidate_matches_hydrated_data(candidate, &data) {
        return None;
    }
    let mut meta = trajectory_data_to_meta_validated(app, &data).await;
    apply_task_trajectory_context(&candidate.path, task_roots, &mut meta);
    Some(meta)
}

async fn hydrate_trajectory_list_page(
    app: AppState,
    candidates: Vec<TrajectoryListCandidate>,
    limit: usize,
    task_roots: &[PathBuf],
) -> (Vec<TrajectoryMeta>, bool) {
    let mut items = Vec::with_capacity(limit);
    let mut has_more = false;
    for candidate in candidates {
        let Some(meta) =
            hydrate_trajectory_list_candidate(app.clone(), &candidate, task_roots).await
        else {
            continue;
        };
        if items.len() == limit {
            has_more = true;
            break;
        }
        items.push(meta);
    }
    (items, has_more)
}

async fn collect_trajectory_list_candidates(
    gcx: &Arc<GlobalContext>,
    cursor_filter: Option<&(String, String)>,
) -> Vec<TrajectoryListCandidate> {
    let mut candidates = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for trajectories_dir in list_trajectory_dirs(gcx).await {
        if !is_real_dir(&trajectories_dir).await {
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
            if !is_real_file(&path).await {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path).await else {
                continue;
            };
            let Ok(data) = serde_json::from_str::<TrajectoryListData>(&content) else {
                continue;
            };
            if data.extra.get("buddy_meta").map_or(false, |v| !v.is_null()) {
                continue;
            }
            if !trajectory_path_stem_matches_id(&path, &data.id) {
                continue;
            }
            if !seen_ids.insert(data.id.clone()) {
                continue;
            }
            if let Some((cursor_updated_at, cursor_id)) = cursor_filter {
                if !cursor_precedes_item(
                    (data.updated_at.as_str(), data.id.as_str()),
                    (cursor_updated_at.as_str(), cursor_id.as_str()),
                ) {
                    continue;
                }
            }
            candidates.push(TrajectoryListCandidate {
                id: data.id,
                updated_at: data.updated_at,
                path,
            });
        }
    }

    candidates
}

async fn is_real_dir(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path).await, Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink())
}

async fn is_real_file(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path).await, Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink())
}

async fn ensure_real_dir_tree(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::Normal(_) => {
                current.push(component.as_os_str());
            }
            Component::ParentDir => {
                return Err(format!(
                    "Refusing to create directory with parent component: {}",
                    path.display()
                ));
            }
        }
        if current.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(&current).await {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "Refusing to use non-real directory component: {}",
                        current.display()
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).await.map_err(|e| {
                    format!("Failed to create directory {}: {}", current.display(), e)
                })?;
                let metadata = fs::symlink_metadata(&current).await.map_err(|e| {
                    format!(
                        "Failed to inspect created directory {}: {}",
                        current.display(),
                        e
                    )
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "Created directory is not a real directory: {}",
                        current.display()
                    ));
                }
            }
            Err(e) => {
                return Err(format!(
                    "Failed to inspect directory component {}: {}",
                    current.display(),
                    e
                ));
            }
        }
    }
    if is_real_dir(path).await {
        Ok(())
    } else {
        Err(format!(
            "Directory is not a real directory: {}",
            path.display()
        ))
    }
}

async fn canonical_child_path_under_root(root: &Path, child: &Path) -> Result<PathBuf, String> {
    let root_canonical = std::fs::canonicalize(root)
        .map_err(|e| format!("Failed to resolve root {}: {}", root.display(), e))?;
    if !is_real_dir(root).await {
        return Err(format!("Root is not a real directory: {}", root.display()));
    }

    if is_real_file(child).await {
        let child_canonical = std::fs::canonicalize(child)
            .map_err(|e| format!("Failed to resolve child {}: {}", child.display(), e))?;
        if child_canonical.starts_with(&root_canonical) {
            return Ok(child.to_path_buf());
        }
        return Err(format!(
            "Child {} is outside root {}",
            child.display(),
            root.display()
        ));
    }

    if let Ok(metadata) = fs::symlink_metadata(child).await {
        if metadata.file_type().is_symlink() {
            return Err(format!("Refusing symlink child file: {}", child.display()));
        }
        return Err(format!("Child is not a real file: {}", child.display()));
    }

    let parent = child
        .parent()
        .ok_or_else(|| format!("Child has no parent: {}", child.display()))?;
    let parent_canonical = std::fs::canonicalize(parent)
        .map_err(|e| format!("Failed to resolve child parent {}: {}", parent.display(), e))?;
    if parent_canonical.starts_with(&root_canonical) {
        Ok(child.to_path_buf())
    } else {
        Err(format!(
            "Child parent {} is outside root {}",
            parent.display(),
            root.display()
        ))
    }
}

async fn safe_trajectory_file_in_dir(dir: &Path, chat_id: &str) -> Option<PathBuf> {
    validate_trajectory_id(chat_id).ok()?;
    ensure_real_dir_tree(dir).await.ok()?;
    let child = dir.join(format!("{}.json", chat_id));
    if !is_real_file(&child).await {
        return None;
    }
    canonical_child_path_under_root(dir, &child).await.ok()
}

async fn safe_new_trajectory_file_in_dir(dir: &Path, chat_id: &str) -> Result<PathBuf, String> {
    validate_trajectory_id(chat_id).map_err(|e| e.message)?;
    ensure_real_dir_tree(dir).await?;
    canonical_child_path_under_root(dir, &dir.join(format!("{}.json", chat_id))).await
}

fn cursor_precedes_item(item: (&str, &str), cursor: (&str, &str)) -> bool {
    item < cursor
}

pub async fn list_trajectories_page(
    app: AppState,
    limit: usize,
    cursor: Option<String>,
) -> Result<PaginatedTrajectories, String> {
    let gcx = app.gcx.clone();
    let limit = limit.clamp(1, 200);
    let cursor_filter = match cursor.as_deref() {
        Some(cursor) => {
            Some(decode_cursor(cursor).ok_or_else(|| "Invalid cursor format".to_string())?)
        }
        None => None,
    };

    let mut candidates = collect_trajectory_list_candidates(&gcx, cursor_filter.as_ref()).await;
    let total_count = if cursor_filter.is_none() {
        candidates.len()
    } else {
        collect_trajectory_list_candidates(&gcx, None).await.len()
    };
    candidates.sort_by(|a, b| match b.updated_at.cmp(&a.updated_at) {
        std::cmp::Ordering::Equal => b.id.cmp(&a.id),
        other => other,
    });

    let task_roots = get_all_task_roots(gcx.clone()).await;
    let (mut items, has_more) =
        hydrate_trajectory_list_page(app.clone(), candidates, limit, &task_roots).await;
    enrich_with_session_state(app, &mut items).await;

    let next_cursor = if has_more {
        items
            .last()
            .map(|last| encode_cursor(&last.updated_at, &last.id))
    } else {
        None
    };

    Ok(PaginatedTrajectories {
        items,
        next_cursor,
        has_more,
        total_count,
    })
}

pub async fn handle_v1_trajectories_list(
    State(app): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<TrajectoriesListQuery>,
) -> Result<Response<Body>, ScratchError> {
    let response = list_trajectories_page(app, params.limit.unwrap_or(50), params.cursor)
        .await
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

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

pub async fn list_all_trajectories_meta(app: AppState) -> Result<Vec<TrajectoryMeta>, String> {
    let gcx = app.gcx.clone();
    let mut result: Vec<TrajectoryMeta> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    let task_roots = get_all_task_roots(gcx.clone()).await;

    for trajectories_dir in list_trajectory_dirs(&gcx).await {
        if !is_real_dir(&trajectories_dir).await {
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
            if !is_real_file(&path).await {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path).await {
                if let Ok(data) = serde_json::from_str::<TrajectoryData>(&content) {
                    if data.extra.get("buddy_meta").map_or(false, |v| !v.is_null()) {
                        continue;
                    }
                    if !trajectory_path_stem_matches_id(&path, &data.id) {
                        continue;
                    }
                    if seen_ids.insert(data.id.clone()) {
                        let mut meta = trajectory_data_to_meta_validated(app.clone(), &data).await;
                        apply_task_trajectory_context(&path, &task_roots, &mut meta);
                        result.push(meta);
                    }
                }
            }
        }
    }

    enrich_with_session_state(app, &mut result).await;
    result.sort_by(|a, b| match b.updated_at.cmp(&a.updated_at) {
        std::cmp::Ordering::Equal => b.id.cmp(&a.id),
        other => other,
    });

    Ok(result)
}

pub async fn handle_v1_trajectories_all(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let result = list_all_trajectories_meta(app)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&result).unwrap()))
        .unwrap())
}

async fn enrich_with_session_state(app: AppState, trajectories: &mut Vec<TrajectoryMeta>) {
    let session_arcs: Vec<(usize, Arc<AMutex<ChatSession>>)> = {
        let sessions = app.chat.sessions.read().await;
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

pub async fn handle_v1_trajectories_get(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_trajectory_id(&id)?;
    let candidate = find_trajectory_or_buddy_file(gcx, &id)
        .await
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, "Trajectory not found".to_string())
        })?;
    let content = candidate.content;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(content))
        .unwrap())
}

pub async fn handle_v1_trajectories_save(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
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
    let file_path = resolve_trajectory_data_save_path(gcx.clone(), &id, &data)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    ensure_existing_trajectory_file_matches(&file_path, &id)
        .await
        .map_err(|e| ScratchError::new(StatusCode::CONFLICT, e))?;
    let is_new = !is_real_file(&file_path).await;
    let is_title_generated = data
        .extra
        .get("isTitleGenerated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let should_generate_title =
        is_placeholder_title(&data.title) && !is_title_generated && !data.messages.is_empty();
    let worktree = if let Some(candidate) = sanitize_worktree_extra(&mut data.extra) {
        match validate_loaded_worktree_strict(app.clone(), &id, candidate).await {
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
    let sessions = app.chat.sessions.clone();
    let (session_state, session_error) = get_session_state_for_chat(&sessions, &id).await;
    let (total_lines_added, total_lines_removed) =
        calculate_line_changes_from_messages(&data.messages);
    let (tasks_total, tasks_done, tasks_failed) =
        calculate_task_progress_from_messages(&data.messages);
    let token_totals = calculate_token_totals_from_messages(&data.messages);
    let (task_id, task_role, agent_id, card_id) = data
        .extra
        .get("task_meta")
        .and_then(|value| serde_json::from_value::<super::types::TaskMeta>(value.clone()).ok())
        .map(|meta| task_context_from_task_meta(Some(&meta)))
        .unwrap_or((None, None, None, None));
    let event = TrajectoryEvent {
        event_type: if is_new {
            "created".to_string()
        } else {
            "updated".to_string()
        },
        id: id.clone(),
        updated_at: Some(data.updated_at.clone()),
        title: Some(trajectory_meta_title(&data.title)),
        is_title_generated: Some(is_title_generated),
        session_state: Some(session_state),
        error: session_error,
        message_count: Some(data.messages.len()),
        parent_id,
        link_type,
        root_chat_id: Some(effective_root),
        task_id,
        task_role,
        agent_id,
        card_id,
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
    let _ = app.chat.trajectory_events_tx.send(event);
    if should_generate_title {
        let trajectories_dir = file_path.parent().map(Path::to_path_buf).ok_or_else(|| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Trajectory file path has no parent directory".to_string(),
            )
        })?;
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
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_trajectory_id(&id)?;
    let candidate = find_trajectory_file(gcx.clone(), &id)
        .await
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, "Trajectory not found".to_string())
        })?;
    let file_path = candidate.path;
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
        task_id: None,
        task_role: None,
        agent_id: None,
        card_id: None,
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
    let _ = app.chat.trajectory_events_tx.send(event);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"status":"ok"}"#))
        .unwrap())
}

pub async fn handle_v1_trajectories_subscribe(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let rx = app.chat.trajectory_events_tx.subscribe();
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
    use crate::chat::diagnostics::{is_ui_only_message, make_ui_only_error_message};
    use crate::chat::types::{ActiveCommandContext, BurstGuard};
    use refact_chat_api::{ClaudeCodeIdentity, FrozenRequestPrefix};
    use serial_test::serial;
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

    async fn make_app_with_workspace(root: &Path) -> (Arc<GlobalContext>, AppState) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![root.to_path_buf()];
        (gcx, app)
    }

    fn sample_trajectory(id: &str, title: &str, updated_at: &str) -> serde_json::Value {
        json!({
            "id": id,
            "title": title,
            "model": "model",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": updated_at,
            "include_project_info": true,
            "checkpoints_enabled": true
        })
    }

    async fn write_trajectory_file(path: &Path, id: &str, title: &str, updated_at: &str) {
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            path,
            serde_json::to_string(&sample_trajectory(id, title, updated_at)).unwrap(),
        )
        .await
        .unwrap();
    }

    async fn write_schema_incomplete_trajectory_file(path: &Path, id: &str) {
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(path, serde_json::to_string(&json!({"id": id})).unwrap())
            .await
            .unwrap();
    }

    async fn write_buddy_conversation_file(path: &Path, id: &str, title: &str) {
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            path,
            serde_json::to_string(&json!({
                "id": id,
                "chat_id": id,
                "title": title,
                "model": "model",
                "mode": "buddy",
                "tool_use": "agent",
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": false,
                "messages": [{"role":"user","content":"hello buddy"}],
                "buddy_meta": {"is_buddy_chat": true, "buddy_chat_kind": "investigation"}
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    }

    async fn wait_for_watcher_start() {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }

    fn test_snapshot(chat_id: &str, title: &str, messages: Vec<ChatMessage>) -> TrajectorySnapshot {
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
            autonomous_no_confirm: false,
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
            auto_compact_enabled: None,
            frozen_request_prefix: None,
            claude_code_identity: None,
            reactive_compact_attempts: None,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
        }
    }

    async fn wait_for_trajectory_event(
        rx: &mut broadcast::Receiver<TrajectoryEvent>,
        id: &str,
    ) -> TrajectoryEvent {
        // Generous timeout: file-watcher notify events can be delayed under
        // heavy parallel test load (notify backend + tokio scheduler contention).
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                match rx.recv().await {
                    Ok(event) if event.id == id => return event,
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(err) => panic!("trajectory event channel closed: {}", err),
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for trajectory event {id}"))
    }

    async fn drain_trajectory_events(rx: &mut broadcast::Receiver<TrajectoryEvent>) {
        loop {
            match tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await {
                Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => break,
            }
        }
    }

    async fn assert_no_trajectory_event_for(
        rx: &mut broadcast::Receiver<TrajectoryEvent>,
        duration: std::time::Duration,
    ) {
        match tokio::time::timeout(duration, rx.recv()).await {
            Err(_) | Ok(Err(broadcast::error::RecvError::Closed)) => {}
            Ok(Ok(event)) => panic!("unexpected trajectory event: {:?}", event),
            Ok(Err(broadcast::error::RecvError::Lagged(skipped))) => {
                panic!("unexpected trajectory event lag, skipped {skipped}")
            }
        }
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

    #[serial]
    #[tokio::test]
    async fn watcher_picks_up_external_edit_to_task_planner_trajectory() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();

        start_trajectory_watcher(gcx);
        wait_for_watcher_start().await;
        drain_trajectory_events(&mut rx).await;

        let chat_id = "planner-watch-chat";
        let path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-watch")
            .join("trajectories")
            .join("planner")
            .join(format!("{}.json", chat_id));
        write_trajectory_file(&path, chat_id, "Planner Watch", "2024-01-01T00:00:01Z").await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.title.as_deref(), Some("Planner Watch"));
    }

    #[serial]
    #[tokio::test]
    async fn watcher_ignores_non_trajectory_files_in_tasks_dir() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();

        start_trajectory_watcher(gcx);
        wait_for_watcher_start().await;
        drain_trajectory_events(&mut rx).await;

        let task_dir = dir.path().join(".refact").join("tasks").join("task-ignore");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        tokio::fs::write(task_dir.join("meta.yaml"), "id: task-ignore\n")
            .await
            .unwrap();

        assert_no_trajectory_event_for(&mut rx, std::time::Duration::from_millis(700)).await;
    }

    #[serial]
    #[tokio::test]
    async fn watcher_picks_up_new_task_dir_created_after_startup() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();

        start_trajectory_watcher(gcx);
        wait_for_watcher_start().await;
        drain_trajectory_events(&mut rx).await;

        let chat_id = "new-task-agent-chat";
        let path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-created-later")
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{}.json", chat_id));
        write_trajectory_file(&path, chat_id, "New Task Agent", "2024-01-01T00:00:02Z").await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.title.as_deref(), Some("New Task Agent"));
    }

    #[tokio::test]
    async fn paginated_list_includes_task_planner_and_agent_trajectories() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let root = dir.path().join(".refact");

        write_trajectory_file(
            &root.join("trajectories").join("project-chat.json"),
            "project-chat",
            "Project Chat",
            "2024-01-01T00:00:03Z",
        )
        .await;
        write_trajectory_file(
            &root
                .join("tasks")
                .join("task-list")
                .join("trajectories")
                .join("planner")
                .join("planner-chat.json"),
            "planner-chat",
            "Planner Chat",
            "2024-01-01T00:00:02Z",
        )
        .await;
        write_trajectory_file(
            &root
                .join("tasks")
                .join("task-list")
                .join("trajectories")
                .join("agents")
                .join("agent-1")
                .join("agent-chat.json"),
            "agent-chat",
            "Agent Chat",
            "2024-01-01T00:00:01Z",
        )
        .await;

        let response = handle_v1_trajectories_list(
            State(app),
            axum::extract::Query(TrajectoriesListQuery {
                limit: Some(10),
                cursor: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let items = payload["items"].as_array().unwrap();
        let ids: std::collections::HashSet<_> = items
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect();

        assert_eq!(payload["total_count"].as_u64(), Some(3));
        assert!(ids.contains("project-chat"));
        assert!(ids.contains("planner-chat"));
        assert!(ids.contains("agent-chat"));

        let planner = items
            .iter()
            .find(|item| item["id"].as_str() == Some("planner-chat"))
            .unwrap();
        assert_eq!(planner["task_id"].as_str(), Some("task-list"));
        assert_eq!(planner["task_role"].as_str(), Some("planner"));

        let agent = items
            .iter()
            .find(|item| item["id"].as_str() == Some("agent-chat"))
            .unwrap();
        assert_eq!(agent["task_id"].as_str(), Some("task-list"));
        assert_eq!(agent["task_role"].as_str(), Some("agents"));
        assert_eq!(agent["agent_id"].as_str(), Some("agent-1"));
    }

    #[tokio::test]
    async fn paginated_list_backfills_when_hydration_skips_changed_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let root = dir.path().join(".refact").join("trajectories");
        write_trajectory_file(
            &root.join("top-chat.json"),
            "top-chat",
            "Top Chat",
            "2024-01-01T00:00:03Z",
        )
        .await;
        write_trajectory_file(
            &root.join("skip-chat.json"),
            "skip-chat",
            "Skip Chat",
            "2024-01-01T00:00:02Z",
        )
        .await;
        write_trajectory_file(
            &root.join("backfill-chat.json"),
            "backfill-chat",
            "Backfill Chat",
            "2024-01-01T00:00:01Z",
        )
        .await;
        tokio::fs::write(
            root.join("skip-chat.json"),
            serde_json::to_string(&sample_trajectory(
                "other-chat",
                "Changed",
                "2024-01-01T00:00:02Z",
            ))
            .unwrap(),
        )
        .await
        .unwrap();

        let candidates = vec![
            TrajectoryListCandidate {
                id: "top-chat".to_string(),
                updated_at: "2024-01-01T00:00:03Z".to_string(),
                path: root.join("top-chat.json"),
            },
            TrajectoryListCandidate {
                id: "skip-chat".to_string(),
                updated_at: "2024-01-01T00:00:02Z".to_string(),
                path: root.join("skip-chat.json"),
            },
            TrajectoryListCandidate {
                id: "backfill-chat".to_string(),
                updated_at: "2024-01-01T00:00:01Z".to_string(),
                path: root.join("backfill-chat.json"),
            },
        ];
        let task_roots = Vec::new();

        let (items, has_more) = hydrate_trajectory_list_page(app, candidates, 2, &task_roots).await;
        let ids: Vec<_> = items.iter().map(|item| item.id.as_str()).collect();

        assert_eq!(ids, vec!["top-chat", "backfill-chat"]);
        assert!(!has_more);
    }

    #[test]
    fn test_validate_trajectory_id_rejects_path_traversal() {
        assert!(validate_trajectory_id("../etc/passwd").is_err());
        assert!(validate_trajectory_id("..").is_err());
        assert!(validate_trajectory_id("a/../b").is_err());
    }

    #[tokio::test]
    async fn trajectory_path_helpers_and_save_reject_malformed_chat_id() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        tokio::fs::create_dir_all(dir.path().join(".refact").join("trajectories"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(
            dir.path()
                .join(".refact")
                .join("buddy")
                .join("chats")
                .join("conversations"),
        )
        .await
        .unwrap();
        tokio::fs::write(dir.path().join(".refact").join("bad.json"), "{}")
            .await
            .unwrap();
        tokio::fs::write(
            dir.path()
                .join(".refact")
                .join("buddy")
                .join("chats")
                .join("bad.json"),
            "{}",
        )
        .await
        .unwrap();

        assert!(find_trajectory_path(gcx.clone(), "../bad").await.is_none());
        assert!(find_trajectory_or_buddy_path(gcx.clone(), "../bad")
            .await
            .is_none());

        let err = save_trajectory_snapshot(
            gcx,
            test_snapshot(
                "../save-bad",
                "Bad",
                vec![ChatMessage::new("user".to_string(), "hello".to_string())],
            ),
        )
        .await
        .unwrap_err();
        assert_eq!(err, "Invalid trajectory id");
        assert!(
            !tokio::fs::try_exists(dir.path().join(".refact").join("save-bad.json"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn generic_trajectory_delete_does_not_delete_buddy_conversation_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "buddy-delete-isolated";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{}.json", chat_id));
        write_buddy_conversation_file(&buddy_path, chat_id, "Keep Buddy").await;

        let err = handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap_err();

        assert_eq!(err.status_code, StatusCode::NOT_FOUND);
        assert!(tokio::fs::try_exists(&buddy_path).await.unwrap());
    }

    #[tokio::test]
    async fn explicit_buddy_inclusive_lookup_loads_buddy_conversation_file() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "buddy-read-inclusive";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{}.json", chat_id));
        write_buddy_conversation_file(&buddy_path, chat_id, "Readable Buddy").await;

        assert!(find_trajectory_path(gcx.clone(), chat_id).await.is_none());
        assert_eq!(
            find_trajectory_or_buddy_path(gcx.clone(), chat_id).await,
            Some(buddy_path.clone())
        );
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();

        assert_eq!(loaded.thread.title, "Readable Buddy");
        assert_eq!(
            loaded.thread.buddy_meta.unwrap().buddy_chat_kind,
            "investigation"
        );
        assert_eq!(loaded.messages.len(), 1);
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_is_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join("safe.json");
        write_trajectory_file(&path, "other-chat", "Mismatched", "2024-01-01T00:00:00Z").await;

        assert!(load_trajectory_for_chat(gcx, "safe").await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_normal_trajectory_root_is_ignored_for_load() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "symlink-load-ignored";
        let refact_dir = dir.path().join(".refact");
        tokio::fs::create_dir_all(&refact_dir).await.unwrap();
        std::os::unix::fs::symlink(outside.path(), refact_dir.join("trajectories")).unwrap();
        let outside_path = outside.path().join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Outside Symlink Load",
            "2024-01-01T00:00:00Z",
        )
        .await;

        assert!(find_trajectory_path(gcx.clone(), chat_id).await.is_none());
        assert!(load_trajectory_for_chat(gcx, chat_id).await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_normal_trajectory_root_is_not_used_for_save() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "symlink-save-blocked";
        let refact_dir = dir.path().join(".refact");
        tokio::fs::create_dir_all(&refact_dir).await.unwrap();
        std::os::unix::fs::symlink(outside.path(), refact_dir.join("trajectories")).unwrap();
        let outside_path = outside.path().join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Keep Outside Save",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let before = tokio::fs::read_to_string(&outside_path).await.unwrap();

        let err = save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "Should Not Escape",
                vec![ChatMessage::new("user".to_string(), "hello".to_string())],
            ),
        )
        .await
        .unwrap_err();

        assert!(err.contains("Refusing to use non-real directory component"));
        assert_eq!(
            tokio::fs::read_to_string(outside_path).await.unwrap(),
            before
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_normal_trajectory_root_is_not_used_for_delete() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "symlink-delete-blocked";
        let refact_dir = dir.path().join(".refact");
        tokio::fs::create_dir_all(&refact_dir).await.unwrap();
        std::os::unix::fs::symlink(outside.path(), refact_dir.join("trajectories")).unwrap();
        let outside_path = outside.path().join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Keep Outside Delete",
            "2024-01-01T00:00:00Z",
        )
        .await;

        let err = handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap_err();

        assert_eq!(err.status_code, StatusCode::NOT_FOUND);
        assert!(tokio::fs::try_exists(outside_path).await.unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_normal_trajectory_file_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "symlink-file-blocked";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let outside_path = outside.path().join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Keep Outside File",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let before = tokio::fs::read_to_string(&outside_path).await.unwrap();
        std::os::unix::fs::symlink(
            &outside_path,
            trajectories_dir.join(format!("{chat_id}.json")),
        )
        .unwrap();

        assert!(find_trajectory_path(gcx.clone(), chat_id).await.is_none());
        let delete_err = handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap_err();
        assert_eq!(delete_err.status_code, StatusCode::NOT_FOUND);

        let save_err = save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "Should Not Follow File",
                vec![ChatMessage::new("user".to_string(), "hello".to_string())],
            ),
        )
        .await
        .unwrap_err();

        assert!(save_err.contains("Refusing symlink child file"));
        assert_eq!(
            tokio::fs::read_to_string(outside_path).await.unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_normal_candidate_does_not_shadow_task_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "safe-task-shadow";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-shadow")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            "other-chat",
            "Mismatched Normal",
            "2024-01-01T00:00:00Z",
        )
        .await;
        write_trajectory_file(&task_path, chat_id, "Valid Task", "2024-01-01T00:00:01Z").await;

        assert_eq!(
            find_trajectory_path(gcx.clone(), chat_id).await,
            Some(task_path)
        );
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.thread.title, "Valid Task");
    }

    #[tokio::test]
    async fn schema_incomplete_normal_candidate_does_not_shadow_task_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "schema-task-shadow";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-schema-shadow")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_schema_incomplete_trajectory_file(&normal_path, chat_id).await;
        write_trajectory_file(&task_path, chat_id, "Valid Task", "2024-01-01T00:00:01Z").await;

        assert_eq!(
            find_trajectory_path(gcx.clone(), chat_id).await,
            Some(task_path)
        );
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.thread.title, "Valid Task");
    }

    #[tokio::test]
    async fn schema_incomplete_candidate_only_is_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "schema-incomplete-only";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_schema_incomplete_trajectory_file(&path, chat_id).await;

        assert!(find_trajectory_path(gcx.clone(), chat_id).await.is_none());
        assert!(load_trajectory_for_chat(gcx, chat_id).await.is_none());
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_workspace_candidate_does_not_shadow_global_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "safe-global-shadow";
        let workspace_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let global_dir = get_global_trajectories_dir(gcx.clone()).await;
        let global_path = global_dir.join(format!("{chat_id}.json"));
        write_trajectory_file(
            &workspace_path,
            "other-chat",
            "Mismatched Workspace",
            "2024-01-01T00:00:00Z",
        )
        .await;
        write_trajectory_file(
            &global_path,
            chat_id,
            "Valid Global",
            "2024-01-01T00:00:01Z",
        )
        .await;

        assert_eq!(
            find_trajectory_path(gcx.clone(), chat_id).await,
            Some(global_path)
        );
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.thread.title, "Valid Global");
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_delete_skips_invalid_higher_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "delete-shadow";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-delete-shadow")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            "other-chat",
            "Keep Mismatched",
            "2024-01-01T00:00:00Z",
        )
        .await;
        write_trajectory_file(&task_path, chat_id, "Delete Valid", "2024-01-01T00:00:01Z").await;

        handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap();

        assert!(tokio::fs::try_exists(&normal_path).await.unwrap());
        assert!(!tokio::fs::try_exists(&task_path).await.unwrap());
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_save_snapshot_does_not_overwrite_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "save-mismatch-guard";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &path,
            "other-chat",
            "Keep Mismatched",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let before = tokio::fs::read_to_string(&path).await.unwrap();

        let err = save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "Should Not Save",
                vec![ChatMessage::new("user".to_string(), "hello".to_string())],
            ),
        )
        .await
        .unwrap_err();

        assert!(err.contains("Existing trajectory file id mismatch"));
        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), before);
    }

    #[tokio::test]
    async fn ordinary_save_preserves_existing_browser_meta_and_unknown_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "metadata-preserve-browser-custom";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Existing Metadata",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [{"role":"user","content":"old"}],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-preserve",
                    "tab_urls": ["https://example.com/preserve"],
                    "active_tab_id": "tab-preserve",
                    "attach_screenshot_on_send": true
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 57}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "Updated Metadata",
                vec![ChatMessage::new("user".to_string(), "new".to_string())],
            ),
        )
        .await
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Updated Metadata");
        assert_eq!(
            saved["browser_meta"]["browser_runtime_id"],
            "browser-preserve"
        );
        assert_eq!(saved["custom_future_field"]["nested"]["value"], 57);
    }

    #[tokio::test]
    async fn ordinary_save_removes_stale_provider_identity_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "metadata-clear-stale-provider";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Provider Existing",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [{"role":"user","content":"old"}],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "previous_response_id": "resp_stale",
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "stale system",
                    "tools_canonical": [{"type":"function"}]
                },
                "claude_code_identity": {
                    "device_id": "device-stale",
                    "session_id": "session-stale"
                },
                "custom_future_field": {"keep": true}
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "Provider Cleared",
                vec![ChatMessage::new("user".to_string(), "new".to_string())],
            ),
        )
        .await
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Provider Cleared");
        assert!(saved.get("previous_response_id").is_none());
        assert!(saved.get("frozen_request_prefix").is_none());
        assert!(saved.get("claude_code_identity").is_none());
        assert_eq!(saved["custom_future_field"]["keep"], true);
    }

    #[tokio::test]
    async fn title_only_ordinary_save_preserves_unknown_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "metadata-title-only-preserve";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Old Title",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "custom_future_field": {"nested": {"value": 58}}
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        save_trajectory_snapshot(gcx, test_snapshot(chat_id, "Retitled", Vec::new()))
            .await
            .unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Retitled");
        assert_eq!(saved["custom_future_field"]["nested"]["value"], 58);
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_http_save_does_not_overwrite_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "http-save-mismatch-guard";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &path,
            "other-chat",
            "Keep Mismatched",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let before = tokio::fs::read_to_string(&path).await.unwrap();
        let payload = sample_trajectory(chat_id, "Should Not Save", "2024-01-01T00:00:01Z");

        let err = handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status_code, StatusCode::CONFLICT);
        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), before);
    }

    #[tokio::test]
    async fn http_save_updates_existing_global_trajectory_without_project_shadow() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "http-save-global-original";
        let global_dir = get_global_trajectories_dir(gcx).await;
        let global_path = global_dir.join(format!("{chat_id}.json"));
        write_trajectory_file(
            &global_path,
            chat_id,
            "Global Original",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let mut payload = sample_trajectory(chat_id, "Global Updated", "2024-01-01T00:00:01Z");
        payload["messages"] = json!([{"role":"user","content":"saved globally"}]);

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&global_path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Global Updated");
        assert_eq!(saved["messages"].as_array().unwrap().len(), 1);
        let project_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        assert!(!tokio::fs::try_exists(project_path).await.unwrap());
    }

    #[tokio::test]
    async fn http_save_updates_existing_task_trajectory_without_project_shadow() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "http-save-task-original";
        let task_id = "task-http-save";
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join(task_id)
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&task_path, chat_id, "Task Original", "2024-01-01T00:00:00Z").await;
        let mut payload = sample_trajectory(chat_id, "Task Updated", "2024-01-01T00:00:01Z");
        payload["task_meta"] = json!({
            "task_id": task_id,
            "role": "agents",
            "agent_id": "agent-1",
            "card_id": "card-1"
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&task_path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Task Updated");
        assert_eq!(saved["task_meta"]["task_id"], task_id);
        let project_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        assert!(!tokio::fs::try_exists(project_path).await.unwrap());
    }

    #[tokio::test]
    async fn http_save_updates_existing_buddy_trajectory_without_project_shadow() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "http-save-buddy-original";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        write_buddy_conversation_file(&buddy_path, chat_id, "Buddy Original").await;
        let mut payload = sample_trajectory(chat_id, "Buddy Updated", "2024-01-01T00:00:01Z");
        payload["mode"] = json!("buddy");
        payload["buddy_meta"] = json!({
            "is_buddy_chat": true,
            "buddy_chat_kind": "investigation"
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&buddy_path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Buddy Updated");
        assert!(saved["buddy_meta"]["is_buddy_chat"].as_bool().unwrap());
        let project_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        assert!(!tokio::fs::try_exists(project_path).await.unwrap());
    }

    #[tokio::test]
    async fn trajectory_save_namespace_generic_save_does_not_overwrite_existing_buddy_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-generic-buddy-collision";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        write_buddy_conversation_file(&buddy_path, chat_id, "Keep Buddy").await;
        let before = tokio::fs::read_to_string(&buddy_path).await.unwrap();
        let mut payload = sample_trajectory(chat_id, "Generic Saved", "2024-01-01T00:00:01Z");
        payload["messages"] = json!([{ "role": "user", "content": "generic" }]);

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&buddy_path).await.unwrap(),
            before
        );
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&normal_path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Generic Saved");
        assert!(saved.get("buddy_meta").is_none());
    }

    #[tokio::test]
    async fn trajectory_save_namespace_generic_save_does_not_overwrite_existing_task_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-generic-task-collision";
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-generic-collision")
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&task_path, chat_id, "Keep Task", "2024-01-01T00:00:00Z").await;
        let before = tokio::fs::read_to_string(&task_path).await.unwrap();
        let payload = sample_trajectory(chat_id, "Generic Saved", "2024-01-01T00:00:01Z");

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        assert_eq!(tokio::fs::read_to_string(&task_path).await.unwrap(), before);
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&normal_path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Generic Saved");
        assert!(saved.get("task_meta").is_none());
    }

    #[tokio::test]
    async fn trajectory_save_namespace_task_save_does_not_overwrite_existing_normal_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-task-normal-collision";
        let task_id = "task-normal-collision";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&normal_path, chat_id, "Keep Normal", "2024-01-01T00:00:00Z").await;
        let before = tokio::fs::read_to_string(&normal_path).await.unwrap();
        let task_dir = dir.path().join(".refact").join("tasks").join(task_id);
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let mut payload = sample_trajectory(chat_id, "Task Saved", "2024-01-01T00:00:01Z");
        payload["task_meta"] = json!({
            "task_id": task_id,
            "role": "agents",
            "agent_id": "agent-1",
            "card_id": "card-1"
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&normal_path).await.unwrap(),
            before
        );
        let task_path = task_dir
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{chat_id}.json"));
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&task_path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Task Saved");
        assert_eq!(saved["task_meta"]["task_id"], task_id);
    }

    #[tokio::test]
    async fn trajectory_save_namespace_buddy_save_does_not_overwrite_existing_normal_or_task_file()
    {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-buddy-normal-task-collision";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-buddy-collision")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&normal_path, chat_id, "Keep Normal", "2024-01-01T00:00:00Z").await;
        write_trajectory_file(&task_path, chat_id, "Keep Task", "2024-01-01T00:00:00Z").await;
        let normal_before = tokio::fs::read_to_string(&normal_path).await.unwrap();
        let task_before = tokio::fs::read_to_string(&task_path).await.unwrap();
        let mut payload = sample_trajectory(chat_id, "Buddy Saved", "2024-01-01T00:00:01Z");
        payload["mode"] = json!("buddy");
        payload["buddy_meta"] = json!({
            "is_buddy_chat": true,
            "buddy_chat_kind": "investigation",
            "workflow_id": null
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&normal_path).await.unwrap(),
            normal_before
        );
        assert_eq!(
            tokio::fs::read_to_string(&task_path).await.unwrap(),
            task_before
        );
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&buddy_path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Buddy Saved");
        assert_eq!(saved["buddy_meta"]["is_buddy_chat"], true);
    }

    #[tokio::test]
    async fn trajectory_save_namespace_task_and_buddy_meta_rejects_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-task-buddy-meta-conflict";
        let task_id = "task-meta-conflict";
        tokio::fs::create_dir_all(dir.path().join(".refact").join("tasks").join(task_id))
            .await
            .unwrap();
        let mut payload = sample_trajectory(chat_id, "Meta Conflict", "2024-01-01T00:00:01Z");
        payload["task_meta"] = json!({
            "task_id": task_id,
            "role": "planner",
            "planner_chat_id": chat_id
        });
        payload["buddy_meta"] = json!({
            "is_buddy_chat": true,
            "buddy_chat_kind": "investigation",
            "workflow_id": null
        });

        let err = handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status_code, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.message.contains("both task_meta and buddy_meta"));
        assert!(!tokio::fs::try_exists(
            dir.path()
                .join(".refact")
                .join("trajectories")
                .join(format!("{chat_id}.json"))
        )
        .await
        .unwrap());
        assert!(!tokio::fs::try_exists(
            dir.path()
                .join(".refact")
                .join("buddy")
                .join("chats")
                .join("conversations")
                .join(format!("{chat_id}.json"))
        )
        .await
        .unwrap());
        assert!(!tokio::fs::try_exists(
            dir.path()
                .join(".refact")
                .join("tasks")
                .join(task_id)
                .join("trajectories")
                .join("planner")
                .join(format!("{chat_id}.json"))
        )
        .await
        .unwrap());
    }

    #[tokio::test]
    async fn trajectory_save_namespace_malformed_non_null_task_meta_rejects_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-malformed-task-meta";
        let mut payload = sample_trajectory(chat_id, "Malformed Task", "2024-01-01T00:00:01Z");
        payload["task_meta"] = json!({"task_id": "task-malformed"});

        let err = handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status_code, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.message.contains("Invalid task_meta"));
        assert!(!tokio::fs::try_exists(
            dir.path()
                .join(".refact")
                .join("trajectories")
                .join(format!("{chat_id}.json"))
        )
        .await
        .unwrap());
        assert!(!tokio::fs::try_exists(
            dir.path()
                .join(".refact")
                .join("tasks")
                .join("task-malformed")
        )
        .await
        .unwrap());
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_title_generation_does_not_update_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "title-mismatch-guard";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        write_trajectory_file(&path, "other-chat", "New Chat", "2024-01-01T00:00:00Z").await;
        let before = tokio::fs::read_to_string(&path).await.unwrap();

        spawn_title_generation_task(
            gcx,
            chat_id.to_string(),
            vec![json!({"role":"user","content":"Generate guarded title"})],
            trajectories_dir,
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), before);
    }

    #[tokio::test]
    async fn title_generation_mismatched_backing_file_does_not_update_active_session() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "title-active-mismatch-guard";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        write_trajectory_file(&path, "other-chat", "New Chat", "2024-01-01T00:00:00Z").await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "New Chat".to_string();
            session.thread.is_title_generated = false;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        spawn_title_generation_task(
            gcx,
            chat_id.to_string(),
            vec![json!({"role":"user","content":"Generate guarded active title"})],
            trajectories_dir,
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let session = session_arc.lock().await;
        assert_eq!(session.thread.title, "New Chat");
        assert!(!session.thread.is_title_generated);
    }

    #[test]
    fn trajectory_id_mismatch_list_hydration_recheck_rejects_changed_data() {
        let candidate = TrajectoryListCandidate {
            id: "hydration-chat".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            path: PathBuf::from("hydration-chat.json"),
        };
        let data = TrajectoryData {
            id: "other-chat".to_string(),
            title: "Changed".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:01Z".to_string(),
            model: "model".to_string(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            messages: Vec::new(),
            extra: serde_json::Map::new(),
        };

        assert!(!trajectory_list_candidate_matches_hydrated_data(
            &candidate, &data
        ));
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_open_creates_fresh_session_without_file_state() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join("safe.json");
        write_trajectory_file(&path, "other-chat", "Mismatched", "2024-01-01T00:00:00Z").await;

        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            "safe",
        )
        .await;
        let session = session_arc.lock().await;

        assert_eq!(session.chat_id, "safe");
        assert_eq!(session.thread.id, "safe");
        assert_ne!(session.thread.title, "Mismatched");
        assert!(session.messages.is_empty());
    }

    #[tokio::test]
    async fn trajectory_id_missing_is_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join("missing-id.json");
        let mut payload = sample_trajectory("missing-id", "Missing Id", "2024-01-01T00:00:00Z");
        payload.as_object_mut().unwrap().remove("id");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&payload).unwrap())
            .await
            .unwrap();

        assert!(load_trajectory_for_chat(gcx, "missing-id").await.is_none());
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_is_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let root = dir.path().join(".refact").join("trajectories");
        write_trajectory_file(
            &root.join("safe.json"),
            "other-chat",
            "Mismatched",
            "2024-01-01T00:00:00Z",
        )
        .await;
        write_trajectory_file(
            &root.join("valid-chat.json"),
            "valid-chat",
            "Valid",
            "2024-01-01T00:00:01Z",
        )
        .await;

        let listed = list_all_trajectories_meta(app).await.unwrap();
        let ids: std::collections::HashSet<_> =
            listed.iter().map(|item| item.id.as_str()).collect();

        assert!(ids.contains("valid-chat"));
        assert!(!ids.contains("other-chat"));
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
    fn test_trajectory_meta_title_truncates_oversized_stored_title() {
        let long_title = "A".repeat(1024 * 1024);
        let result = trajectory_meta_title(&long_title);
        assert_eq!(result.chars().count(), TRAJECTORY_META_TITLE_MAX_CHARS);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_trajectory_meta_uses_bounded_title() {
        let data = TrajectoryData {
            id: "big-title-chat".to_string(),
            title: "B".repeat(1024 * 1024),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            model: "model".to_string(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            messages: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let meta = trajectory_data_to_meta(&data);
        assert_eq!(meta.title.chars().count(), TRAJECTORY_META_TITLE_MAX_CHARS);
    }

    #[test]
    fn test_cursor_precedes_item_matches_descending_order() {
        assert!(cursor_precedes_item(
            ("2024-01-01T00:00:00Z", "a"),
            ("2024-01-02T00:00:00Z", "b"),
        ));
        assert!(cursor_precedes_item(
            ("2024-01-01T00:00:00Z", "a"),
            ("2024-01-01T00:00:00Z", "b"),
        ));
        assert!(!cursor_precedes_item(
            ("2024-01-03T00:00:00Z", "a"),
            ("2024-01-02T00:00:00Z", "b"),
        ));
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
    fn build_title_generation_context_skips_hidden_plan_and_event_roles() {
        let messages = vec![
            json!({"role": "user", "content": "Visible user request"}),
            json!({
                "role": "plan",
                "content": "Hidden base plan must not title the chat",
                "extra": {"plan": {"mode": "agent", "version": 1}}
            }),
            json!({
                "role": "event",
                "content": "Hidden plan delta must not title the chat",
                "extra": {
                    "event": {
                        "subkind": "plan_delta",
                        "source": "tool.update_plan",
                        "payload": {"seq": 1}
                    }
                }
            }),
            json!({"role": "assistant", "content": "Visible assistant response"}),
        ];

        let context = build_title_generation_context(&messages);

        assert!(context.contains("Visible user request"));
        assert!(context.contains("Visible assistant response"));
        assert!(!context.contains("Hidden base plan"));
        assert!(!context.contains("Hidden plan delta"));
    }

    #[test]
    fn build_title_generation_context_skips_ui_only_error() {
        let messages = vec![
            json!({"role": "error", "content": "context_length_exceeded", "_ui_only": true}),
            json!({"role": "user", "content": "Implement title filtering"}),
        ];
        let context = build_title_generation_context(&messages);
        assert!(context.contains("Implement title filtering"));
        assert!(!context.contains("context_length_exceeded"));
    }

    #[test]
    fn build_title_generation_context_skips_ui_only_reactive_compaction_report() {
        let messages = vec![
            json!({
                "role": "summarization",
                "content": "Legacy diagnostic report",
                "summarization_tier": "legacy_reactive",
                "_ui_only": true
            }),
            json!({"role": "user", "content": "Fix sanitizers"}),
        ];
        let context = build_title_generation_context(&messages);
        assert!(context.contains("Fix sanitizers"));
        assert!(!context.contains("Legacy diagnostic report"));
    }

    #[test]
    fn build_title_generation_context_skips_extra_ui_only_reactive_report() {
        let messages = vec![
            json!({
                "role": "summarization",
                "content": "Legacy diagnostic report",
                "summarization_tier": "legacy_reactive",
                "extra": {"_ui_only": true}
            }),
            json!({"role": "user", "content": "Fix sanitizers"}),
        ];
        let context = build_title_generation_context(&messages);
        assert!(context.contains("Fix sanitizers"));
        assert!(!context.contains("Legacy diagnostic report"));
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
            task_id: Some("task-123".to_string()),
            task_role: Some("agents".to_string()),
            agent_id: Some("agent-123".to_string()),
            card_id: Some("card-123".to_string()),
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
        assert_eq!(json["task_id"], "task-123");
        assert_eq!(json["task_role"], "agents");
        assert_eq!(json["agent_id"], "agent-123");
        assert_eq!(json["card_id"], "card-123");
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
    fn test_trajectory_event_serialization_skips_none_metric_fields() {
        let event = TrajectoryEvent {
            event_type: "updated".to_string(),
            id: "chat-no-metrics".to_string(),
            updated_at: None,
            title: Some("Retitled".to_string()),
            is_title_generated: None,
            session_state: None,
            error: None,
            message_count: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            task_id: None,
            task_role: None,
            agent_id: None,
            card_id: None,
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
        let json = serde_json::to_value(&event).unwrap();
        assert!(json.get("total_prompt_tokens").is_none());
        assert!(json.get("total_completion_tokens").is_none());
        assert!(json.get("total_tokens").is_none());
        assert!(json.get("total_cache_read_tokens").is_none());
        assert!(json.get("total_cache_creation_tokens").is_none());
        assert!(json.get("total_cost_usd").is_none());
    }

    #[test]
    fn trajectory_snapshot_from_session_preserves_ui_only_diagnostics() {
        let mut session = ChatSession::new("ui-only-snapshot".to_string());
        let diagnostic = make_ui_only_error_message("context_length_exceeded");
        session
            .messages
            .push(ChatMessage::new("user".to_string(), "Hello".to_string()));
        session.messages.push(diagnostic.clone());

        let snapshot = trajectory_snapshot_from_session(&session);

        assert_eq!(snapshot.messages.len(), 2);
        assert!(snapshot.messages.iter().any(|message| {
            message.message_id == diagnostic.message_id
                && message.role == "error"
                && is_ui_only_message(message)
        }));
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
                autonomous_no_confirm: false,
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
                auto_compact_enabled: None,
                frozen_request_prefix: None,
                claude_code_identity: None,
                reactive_compact_attempts: None,
            },
            messages: vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
            runtime: super::super::types::RuntimeState::default(),
            is_compressing: false,
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            event_seq: 0,
            event_tx: tx,
            recent_request_ids: VecDeque::new(),
            recent_request_ids_set: std::collections::HashSet::new(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            abort_notify: Arc::new(Notify::new()),
            user_interrupt_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            last_activity: Instant::now(),
            last_stream_delta_at: None,
            last_tool_started_at: None,
            last_tool_progress_at: None,
            trajectory_dirty: false,
            trajectory_version: 5,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            closed: false,
            closed_flag: Arc::new(AtomicBool::new(false)),
            external_reload_pending: false,
            last_prompt_messages: Vec::new(),
            tier1_compact_attempts: 0,
            tier1_compaction_disabled: false,
            cache_guard_snapshot: None,
            cache_guard_force_next: false,
            task_agent_error: None,
            trajectory_events_tx: None,
            pending_browser_message: None,
            post_tool_side_effects: VecDeque::new(),
            active_command: ActiveCommandContext::default(),
            skills_available_count: 0,
            skills_included: Vec::new(),
            pending_skill_deactivation: None,
            stop_hook_handle: None,
            openai_codex_websocket: Default::default(),
            suppress_auto_enrichment_for_next_turn: false,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
            background_completion_burst: BurstGuard::new(),
            background_agents: std::collections::HashMap::new(),
        };

        let snapshot = trajectory_snapshot_from_session(&session);
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
            is_compressing: false,
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            event_seq: 0,
            event_tx: tx,
            recent_request_ids: VecDeque::new(),
            recent_request_ids_set: std::collections::HashSet::new(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            abort_notify: Arc::new(Notify::new()),
            user_interrupt_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            last_activity: Instant::now(),
            last_stream_delta_at: None,
            last_tool_started_at: None,
            last_tool_progress_at: None,
            trajectory_dirty: false,
            trajectory_version: 1,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            closed: false,
            closed_flag: Arc::new(AtomicBool::new(false)),
            external_reload_pending: false,
            last_prompt_messages: Vec::new(),
            tier1_compact_attempts: 0,
            tier1_compaction_disabled: false,
            cache_guard_snapshot: None,
            cache_guard_force_next: false,
            task_agent_error: None,
            trajectory_events_tx: None,
            pending_browser_message: None,
            post_tool_side_effects: VecDeque::new(),
            active_command: ActiveCommandContext::default(),
            skills_available_count: 0,
            skills_included: Vec::new(),
            pending_skill_deactivation: None,
            stop_hook_handle: None,
            openai_codex_websocket: Default::default(),
            suppress_auto_enrichment_for_next_turn: false,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
            background_completion_burst: BurstGuard::new(),
            background_agents: std::collections::HashMap::new(),
        };

        let snapshot = trajectory_snapshot_from_session(&session);
        assert_eq!(snapshot.active_skill, Some("my-skill".to_string()));

        session.thread.active_skill = None;
        let snapshot_none = trajectory_snapshot_from_session(&session);
        assert!(snapshot_none.active_skill.is_none());
    }

    #[test]
    fn trajectory_snapshot_from_session_captures_wake_up_at() {
        let wake_up_at = chrono::Utc::now() + chrono::Duration::minutes(5);
        let mut session = ChatSession::new("wake-snapshot".to_string());
        session.wake_up_at = Some(wake_up_at);

        let snapshot = trajectory_snapshot_from_session(&session);

        assert_eq!(snapshot.wake_up_at, Some(wake_up_at));
    }

    #[tokio::test]
    async fn reopen_preserves_pinned_plan() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let mut plan_extra = serde_json::Map::new();
        plan_extra.insert(
            "plan".to_string(),
            json!({
                "mode": "agent",
                "version": 1,
                "created_at_ms": 123,
                "supersedes": null,
            }),
        );
        let plan = ChatMessage {
            role: "plan".to_string(),
            content: ChatContent::SimpleText("base plan".to_string()),
            preserve: Some(true),
            extra: plan_extra,
            ..Default::default()
        };
        let delta = crate::chat::internal_roles::plan_delta(
            "tool.set_plan",
            json!({"seq": 1}),
            "append update",
        );
        let mut session = ChatSession::new("plan-reopen".to_string());
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.add_message(plan);
        session.add_message(delta);

        let snapshot = trajectory_snapshot_from_session(&session);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "plan-reopen").await.unwrap();
        let plans: Vec<_> = loaded
            .messages
            .iter()
            .filter(|message| message.role == "plan")
            .collect();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].content.content_text_only(), "base plan");
        assert_eq!(plans[0].preserve, Some(true));
        assert_eq!(plans[0].extra["plan"]["version"], json!(1));
        let deltas: Vec<_> = loaded
            .messages
            .iter()
            .filter(|message| {
                message.role == "event" && message.extra["event"]["subkind"] == json!("plan_delta")
            })
            .collect();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].content.content_text_only(), "append update");
    }

    #[tokio::test]
    async fn legacy_fallback_ids_are_distinct_for_repeated_hidden_events() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        tokio::fs::write(
            trajectories_dir.join("legacy-hidden-events.json"),
            serde_json::to_string(&json!({
                "id": "legacy-hidden-events",
                "title": "Legacy Hidden Events",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [
                    {
                        "role": "event",
                        "content": "append update",
                        "extra": {
                            "event": {
                                "subkind": "plan_delta",
                                "source": "tool.update_plan",
                                "payload": {"seq": 1}
                            }
                        }
                    },
                    {
                        "role": "event",
                        "content": "append update",
                        "extra": {
                            "event": {
                                "subkind": "plan_delta",
                                "source": "tool.update_plan",
                                "payload": {"seq": 2}
                            }
                        }
                    },
                    {
                        "message_id": "kept-existing-id",
                        "role": "user",
                        "content": "visible"
                    }
                ],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "legacy-hidden-events")
            .await
            .unwrap();

        assert_eq!(loaded.messages.len(), 3);
        assert!(loaded.messages[0].message_id.starts_with("legacy:event:"));
        assert!(loaded.messages[1].message_id.starts_with("legacy:event:"));
        assert_ne!(loaded.messages[0].message_id, loaded.messages[1].message_id);
        assert_eq!(loaded.messages[2].message_id, "kept-existing-id");
    }

    #[tokio::test]
    async fn wake_up_at_round_trips_through_trajectory_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }

        let wake_up_at = chrono::Utc::now() + chrono::Duration::minutes(5);
        let mut session = ChatSession::new("wake-roundtrip".to_string());
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.wake_up_at = Some(wake_up_at);
        session.add_message(ChatMessage::new("user".to_string(), "wait".to_string()));

        let snapshot = trajectory_snapshot_from_session(&session);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        drop(session);

        let loaded = load_trajectory_for_chat(gcx, "wake-roundtrip")
            .await
            .unwrap();
        assert_eq!(loaded.wake_up_at, Some(wake_up_at));
    }

    #[tokio::test]
    async fn wake_up_at_is_none_in_trajectories_created_before_field_existed() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }

        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        tokio::fs::write(
            trajectories_dir.join("legacy-wake.json"),
            r#"{
                "id":"legacy-wake",
                "title":"Legacy",
                "created_at":"2024-01-01T00:00:00Z",
                "updated_at":"2024-01-01T00:00:00Z",
                "model":"model",
                "mode":"agent",
                "tool_use":"agent",
                "messages":[{"role":"user","content":"hello"}],
                "include_project_info":true,
                "checkpoints_enabled":true
            }"#,
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "legacy-wake").await.unwrap();
        assert!(loaded.wake_up_at.is_none());
    }

    #[tokio::test]
    async fn trajectory_snapshot_roundtrip_preserves_waiting_for_card_ids() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }

        let card_ids = vec!["T-1".to_string(), "T-10".to_string(), "T-2".to_string()];
        let mut session = ChatSession::new("wait-card-roundtrip".to_string());
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.waiting_for_card_ids = card_ids.clone();
        session.add_message(ChatMessage::new("user".to_string(), "waiting".to_string()));

        let snapshot = trajectory_snapshot_from_session(&session);
        assert_eq!(snapshot.waiting_for_card_ids, card_ids);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "wait-card-roundtrip")
            .await
            .unwrap();
        assert_eq!(loaded.waiting_for_card_ids, card_ids);
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
        let snapshot = trajectory_snapshot_from_session(&session);
        assert_eq!(snapshot.worktree, Some(worktree));
    }

    #[test]
    fn trajectory_snapshot_from_session_filters_empty_assistant_messages() {
        let mut session = ChatSession::new("empty-assistant-snapshot".to_string());
        session.messages.push(ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("hello".to_string()),
            ..Default::default()
        });
        session.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("   \n".to_string()),
            ..Default::default()
        });
        session.messages.push(ChatMessage {
            role: "error".to_string(),
            content: ChatContent::SimpleText("LLM error".to_string()),
            ..Default::default()
        });

        let snapshot = trajectory_snapshot_from_session(&session);

        assert_eq!(snapshot.messages.len(), 2);
        assert_eq!(snapshot.messages[0].role, "user");
        assert_eq!(snapshot.messages[1].role, "error");
    }

    #[test]
    fn trajectory_snapshot_from_session_preserves_ui_only_messages() {
        let mut session = ChatSession::new("ui-only-snapshot".to_string());
        session
            .messages
            .push(ChatMessage::new("user".to_string(), "visible".to_string()));
        session
            .messages
            .push(make_ui_only_error_message("context_length_exceeded"));

        let snapshot = trajectory_snapshot_from_session(&session);

        assert_eq!(snapshot.messages.len(), 2);
        assert_eq!(snapshot.messages[0].role, "user");
        assert_eq!(snapshot.messages[1].role, "error");
        assert!(is_ui_only_message(&snapshot.messages[1]));
    }

    #[test]
    fn trajectory_snapshot_from_session_filters_metadata_only_assistant_messages() {
        use crate::call_validation::ChatUsage;

        let mut session = ChatSession::new("metadata-only-assistant-snapshot".to_string());
        session.messages.push(ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("hello".to_string()),
            ..Default::default()
        });
        session.messages.push(ChatMessage {
            role: "assistant".to_string(),
            usage: Some(ChatUsage {
                prompt_tokens: 10,
                completion_tokens: 0,
                total_tokens: 10,
                cache_creation_tokens: None,
                cache_read_tokens: None,
                metering_usd: None,
            }),
            extra: serde_json::Map::from_iter([(
                "openai_response_id".to_string(),
                json!("resp_123"),
            )]),
            ..Default::default()
        });
        session.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("visible".to_string()),
            ..Default::default()
        });

        let snapshot = trajectory_snapshot_from_session(&session);

        assert_eq!(snapshot.messages.len(), 2);
        assert_eq!(snapshot.messages[0].role, "user");
        assert_eq!(snapshot.messages[1].role, "assistant");
        assert_eq!(snapshot.messages[1].content.content_text_only(), "visible");
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
    async fn trajectory_save_removes_malformed_worktree_extra_from_json() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }
        let chat_id = "malformed-worktree-save";
        let payload = json!({
            "id": chat_id,
            "title": "Malformed Worktree",
            "model": "m",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "worktree": {"root":"/tmp/untrusted"}
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{}.json", chat_id));
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert!(saved.get("worktree").is_none());
    }

    #[tokio::test]
    async fn trajectory_save_preserves_valid_worktree_extra_in_json() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let app = AppState::from_gcx(gcx.clone()).await;
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/save-preserve".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let worktree = created.worktree.meta.clone();
        let chat_id = "valid-worktree-save";
        let payload = json!({
            "id": chat_id,
            "title": "Valid Worktree",
            "model": "m",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "worktree": serde_json::to_value(&worktree).unwrap()
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        let path = source
            .join(".refact")
            .join("trajectories")
            .join(format!("{}.json", chat_id));
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(saved["worktree"]["id"], worktree.id);
        assert_eq!(saved["worktree"]["branch"], "refact/chat/save-preserve");
    }

    #[tokio::test]
    async fn trajectory_worktree_save_load_roundtrips_top_level_field() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let app = AppState::from_gcx(gcx.clone()).await;
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
            autonomous_no_confirm: false,
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
            auto_compact_enabled: None,
            frozen_request_prefix: None,
            claude_code_identity: None,
            reactive_compact_attempts: None,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
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
        let listed = list_all_trajectories_meta(app).await.unwrap();
        let listed_worktree = listed
            .iter()
            .find(|item| item.id == chat_id)
            .and_then(|item| item.worktree.clone())
            .unwrap();
        assert_eq!(listed_worktree.id, worktree.id);
    }

    #[tokio::test]
    async fn frozen_request_prefix_and_claude_code_identity_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let tools_canonical = json!([
            {
                "type": "function",
                "function": {
                    "name": "cat",
                    "description": "Read a file",
                    "parameters": {"type": "object"}
                }
            }
        ]);
        let frozen_request_prefix = FrozenRequestPrefix {
            schema_version: 1,
            created_at: "2026-05-29T00:00:00Z".to_string(),
            system_prompt: Some("verbatim system".to_string()),
            tools_canonical: Some(tools_canonical.clone()),
        };
        let claude_code_identity = ClaudeCodeIdentity {
            device_id: "device-123".to_string(),
            session_id: "session-456".to_string(),
        };
        let mut snapshot = test_snapshot(
            "frozen-prefix-roundtrip",
            "Frozen Prefix",
            vec![ChatMessage::new("user".to_string(), "hello".to_string())],
        );
        snapshot.frozen_request_prefix = Some(frozen_request_prefix.clone());
        snapshot.claude_code_identity = Some(claude_code_identity.clone());

        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join("frozen-prefix-roundtrip.json");
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["tools_canonical"],
            tools_canonical
        );
        assert_eq!(raw["claude_code_identity"]["device_id"], "device-123");

        let loaded = load_trajectory_for_chat(gcx, "frozen-prefix-roundtrip")
            .await
            .unwrap();
        assert_eq!(
            loaded.thread.frozen_request_prefix,
            Some(frozen_request_prefix)
        );
        assert_eq!(
            loaded.thread.claude_code_identity,
            Some(claude_code_identity)
        );
    }

    #[tokio::test]
    async fn frozen_request_prefix_migrates_system_prompt_from_legacy_trajectory() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("legacy-frozen-prefix.json"),
            r#"{
                "id":"legacy-frozen-prefix",
                "title":"Legacy",
                "created_at":"2024-01-01T00:00:00Z",
                "updated_at":"2024-01-01T00:00:00Z",
                "model":"model",
                "mode":"agent",
                "tool_use":"agent",
                "messages":[
                    {"role":"system","content":"legacy system"},
                    {"role":"user","content":"hello"}
                ],
                "include_project_info":true,
                "checkpoints_enabled":true
            }"#,
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "legacy-frozen-prefix")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(prefix.system_prompt.as_deref(), Some("legacy system"));
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
    }

    #[tokio::test]
    async fn mode_transition_load_discards_stale_copied_frozen_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-stale-frozen.json"),
            serde_json::to_string(&json!({
                "id":"transition-stale-frozen",
                "title":"Transition",
                "created_at":"2024-01-01T00:00:00Z",
                "updated_at":"2024-01-01T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "old planning system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-stale-frozen")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
    }

    #[tokio::test]
    async fn mode_transition_no_frozen_prefix_with_copied_provider_state_is_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-no-prefix-provider.json"),
            serde_json::to_string(&json!({
                "id":"transition-no-prefix-provider",
                "title":"Transition",
                "created_at":"2024-01-02T00:00:00Z",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-no-prefix-provider")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
    }

    #[tokio::test]
    async fn mode_transition_matching_system_with_stale_tools_is_repaired_by_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-stale-tools-timestamp.json"),
            serde_json::to_string(&json!({
                "id":"transition-stale-tools-timestamp",
                "title":"Transition",
                "created_at":"2024-01-02T00:00:00Z",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-01T00:00:00Z",
                    "system_prompt": "target task planner system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-stale-tools-timestamp")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
    }

    #[tokio::test]
    async fn mode_transition_malformed_frozen_prefix_with_provider_identity_is_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-malformed-prefix-provider.json"),
            serde_json::to_string(&json!({
                "id":"transition-malformed-prefix-provider",
                "title":"Transition",
                "created_at":"2024-01-02T00:00:00Z",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": "bad",
                    "created_at": 7,
                    "system_prompt": ["source system"],
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-malformed-prefix-provider")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
    }

    #[tokio::test]
    async fn mode_transition_invalid_prefix_timestamp_with_provider_state_is_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-invalid-prefix-time-provider.json"),
            serde_json::to_string(&json!({
                "id":"transition-invalid-prefix-time-provider",
                "title":"Transition",
                "created_at":"2024-01-02T00:00:00Z",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "not-a-date",
                    "system_prompt": "target task planner system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-invalid-prefix-time-provider")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
        assert!(loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn mode_transition_invalid_trajectory_created_at_with_tools_and_provider_state_is_repaired(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-invalid-trajectory-time-provider.json"),
            serde_json::to_string(&json!({
                "id":"transition-invalid-trajectory-time-provider",
                "title":"Transition",
                "created_at":"not-a-date",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-03T00:00:00Z",
                    "system_prompt": "target task planner system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-invalid-trajectory-time-provider")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
        assert!(loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn mode_transition_missing_trajectory_created_at_with_tools_and_provider_state_is_repaired(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-missing-trajectory-time-provider.json"),
            serde_json::to_string(&json!({
                "id":"transition-missing-trajectory-time-provider",
                "title":"Transition",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-03T00:00:00Z",
                    "system_prompt": "target task planner system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-missing-trajectory-time-provider")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
        assert!(loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn mode_transition_missing_frozen_system_with_tools_is_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-missing-system-tools.json"),
            serde_json::to_string(&json!({
                "id":"transition-missing-system-tools",
                "title":"Transition",
                "created_at":"2024-01-01T00:00:00Z",
                "updated_at":"2024-01-01T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-02T00:00:00Z",
                    "system_prompt": null,
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-missing-system-tools")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn handoff_load_discards_stale_copied_frozen_prefix_and_provider_state() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("handoff-stale-frozen-provider.json"),
            serde_json::to_string(&json!({
                "id":"handoff-stale-frozen-provider",
                "title":"Handoff",
                "created_at":"2024-01-02T00:00:00Z",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"agent",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"handoff",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target handoff system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-01T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "handoff-stale-frozen-provider")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target handoff system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
        assert!(loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn mode_transition_preserves_legitimate_prefix_created_after_trajectory() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-legitimate-prefix.json"),
            serde_json::to_string(&json!({
                "id":"transition-legitimate-prefix",
                "title":"Transition",
                "created_at":"2024-01-01T00:00:00Z",
                "updated_at":"2024-01-01T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-02T00:00:00Z",
                    "system_prompt": "target task planner system",
                    "tools_canonical": [{"type":"function","function":{"name":"target_tool"}}]
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-legitimate-prefix")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.tools_canonical,
            Some(json!([{"type":"function","function":{"name":"target_tool"}}]))
        );
        assert!(!loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn ordinary_branch_load_preserves_copied_frozen_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("branch-preserve-frozen.json"),
            serde_json::to_string(&json!({
                "id":"branch-preserve-frozen",
                "title":"Branch",
                "created_at":"2024-01-01T00:00:00Z",
                "updated_at":"2024-01-01T00:00:00Z",
                "model":"model",
                "mode":"agent",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"branch",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"branch system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "branch-preserve-frozen")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(prefix.system_prompt.as_deref(), Some("source system"));
        assert_eq!(
            prefix.tools_canonical,
            Some(json!([{"type":"function","function":{"name":"source_tool"}}]))
        );
        assert!(loaded.thread.claude_code_identity.is_some());
        assert_eq!(
            loaded.thread.previous_response_id.as_deref(),
            Some("resp_source")
        );
    }

    #[tokio::test]
    async fn mode_transition_open_persists_repaired_frozen_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-open-repair";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Transition Open",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "old planning system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-open",
                    "tab_urls": ["https://example.com/open"],
                    "active_tab_id": "tab-open",
                    "attach_screenshot_on_send": true
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 43}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await;
        {
            let session = session_arc.lock().await;
            let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
            assert_eq!(
                prefix.system_prompt.as_deref(),
                Some("target task planner system")
            );
            assert!(prefix.tools_canonical.is_none());
            assert!(session.thread.claude_code_identity.is_none());
        }

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "target task planner system"
        );
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(raw["browser_meta"]["browser_runtime_id"], "browser-open");
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 43);
    }

    #[tokio::test]
    async fn mode_transition_no_system_provider_cleanup_persists_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-no-system-provider-open";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Transition Open",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"user","content":"hello without system"}
                ],
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-immediate",
                    "tab_urls": ["https://example.com/immediate"],
                    "active_tab_id": "tab-immediate",
                    "attach_screenshot_on_send": false
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 44}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await;
        {
            let session = session_arc.lock().await;
            assert!(session.thread.frozen_request_prefix.is_none());
            assert!(session.thread.claude_code_identity.is_none());
            assert!(session.thread.previous_response_id.is_none());
        }

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert!(raw.get("frozen_request_prefix").is_none());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(
            raw["browser_meta"]["browser_runtime_id"],
            "browser-immediate"
        );
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 44);
    }

    #[tokio::test]
    async fn mode_transition_empty_provider_cleanup_persists_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-empty-provider-open";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Transition Open",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [],
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await;
        {
            let session = session_arc.lock().await;
            assert!(session.messages.is_empty());
            assert!(session.thread.frozen_request_prefix.is_none());
            assert!(session.thread.claude_code_identity.is_none());
            assert!(session.thread.previous_response_id.is_none());
        }

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(raw["messages"].as_array().unwrap().len(), 0);
        assert!(raw.get("frozen_request_prefix").is_none());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
    }

    #[tokio::test]
    async fn mode_transition_empty_provider_cleanup_persists_to_original_global_path_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-empty-provider-global-open";
        let global_trajectories_dir = get_global_trajectories_dir(gcx.clone()).await;
        tokio::fs::create_dir_all(&global_trajectories_dir)
            .await
            .unwrap();
        let path = global_trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Transition Global Open",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [],
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-global",
                    "tab_urls": ["https://example.com/global"],
                    "active_tab_id": "tab-global",
                    "attach_screenshot_on_send": true
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 46}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let project_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));

        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await;
        {
            let session = session_arc.lock().await;
            assert!(session.messages.is_empty());
            assert!(session.thread.frozen_request_prefix.is_none());
            assert!(session.thread.claude_code_identity.is_none());
            assert!(session.thread.previous_response_id.is_none());
        }

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(raw["messages"].as_array().unwrap().len(), 0);
        assert!(raw.get("frozen_request_prefix").is_none());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(raw["browser_meta"]["browser_runtime_id"], "browser-global");
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 46);
        assert!(!tokio::fs::try_exists(project_path).await.unwrap());
    }

    #[tokio::test]
    async fn mode_transition_global_placeholder_title_save_uses_original_path_without_workspace() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let chat_id = "transition-global-placeholder-title-save";
        let global_trajectories_dir = get_global_trajectories_dir(gcx.clone()).await;
        tokio::fs::create_dir_all(&global_trajectories_dir)
            .await
            .unwrap();
        let path = global_trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "New Chat",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [{"role":"user","content":"hello from global"}],
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "isTitleGenerated": false
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let mut snapshot = test_snapshot(
            chat_id,
            "New Chat",
            vec![ChatMessage::new(
                "user".to_string(),
                "hello from global".to_string(),
            )],
        );
        snapshot.mode = "task_planner".to_string();
        snapshot.parent_id = Some("source-chat".to_string());
        snapshot.link_type = Some("mode_transition".to_string());
        snapshot.previous_response_id = Some("resp_source".to_string());
        snapshot.claude_code_identity = Some(ClaudeCodeIdentity {
            device_id: "source-device".to_string(),
            session_id: "source-session".to_string(),
        });
        snapshot.is_title_generated = false;
        let project_dir_error = get_trajectories_dir(gcx.clone()).await.unwrap_err();
        assert_eq!(project_dir_error, "No workspace folder found");

        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(raw["messages"].as_array().unwrap().len(), 1);
        assert_eq!(
            find_trajectory_or_buddy_path(gcx.clone(), chat_id).await,
            Some(path)
        );
        let all_dirs = get_all_trajectories_dirs(gcx).await;
        assert_eq!(all_dirs, vec![global_trajectories_dir]);
    }

    #[tokio::test]
    async fn no_meta_snapshot_does_not_overwrite_buddy_conversation_collision() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "buddy-normal-collision";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        write_buddy_conversation_file(&buddy_path, chat_id, "Keep Buddy Collision").await;

        save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(
                chat_id,
                "Normal Chat",
                vec![ChatMessage::new(
                    "user".to_string(),
                    "hello normal".to_string(),
                )],
            ),
        )
        .await
        .unwrap();

        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        assert!(tokio::fs::try_exists(&normal_path).await.unwrap());
        let buddy_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&buddy_path).await.unwrap()).unwrap();
        assert_eq!(buddy_raw["title"], "Keep Buddy Collision");
        assert!(buddy_raw.get("buddy_meta").is_some());
        assert_eq!(
            find_trajectory_path(gcx.clone(), chat_id).await,
            Some(normal_path)
        );
        assert_ne!(
            find_trajectory_or_buddy_path(gcx, chat_id).await,
            Some(buddy_path)
        );
    }

    #[tokio::test]
    async fn no_meta_snapshot_does_not_overwrite_task_trajectory_collision() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "task-normal-collision";
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-collision")
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &task_path,
            chat_id,
            "Keep Task Collision",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let task_before = tokio::fs::read_to_string(&task_path).await.unwrap();

        save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(
                chat_id,
                "Normal Chat",
                vec![ChatMessage::new(
                    "user".to_string(),
                    "hello normal".to_string(),
                )],
            ),
        )
        .await
        .unwrap();

        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        assert!(tokio::fs::try_exists(&normal_path).await.unwrap());
        assert_eq!(
            tokio::fs::read_to_string(&task_path).await.unwrap(),
            task_before
        );
        let normal_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&normal_path).await.unwrap()).unwrap();
        assert_eq!(normal_raw["title"], "Normal Chat");
        let task_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&task_path).await.unwrap()).unwrap();
        assert_eq!(task_raw["title"], "Keep Task Collision");
        assert_eq!(
            find_trajectory_path(gcx.clone(), chat_id).await,
            Some(normal_path)
        );
        assert_ne!(find_trajectory_path(gcx, chat_id).await, Some(task_path));
    }

    #[tokio::test]
    async fn mode_transition_new_empty_snapshot_without_existing_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-new-empty-skip";

        save_trajectory_snapshot(gcx, test_snapshot(chat_id, "New Empty", Vec::new()))
            .await
            .unwrap();

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        assert!(!tokio::fs::try_exists(path).await.unwrap());
    }

    #[tokio::test]
    async fn mode_transition_external_reload_persists_provider_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-external-reload-provider";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Transition External Reload",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [],
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-immediate",
                    "tab_urls": ["https://example.com/immediate"],
                    "active_tab_id": "tab-immediate",
                    "attach_screenshot_on_send": false
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 44}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change(gcx, chat_id, false).await;

        {
            let session = session_arc.lock().await;
            assert!(session.messages.is_empty());
            assert!(session.thread.frozen_request_prefix.is_none());
            assert!(session.thread.claude_code_identity.is_none());
            assert!(session.thread.previous_response_id.is_none());
            assert!(!session.trajectory_dirty);
        }
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(raw["messages"].as_array().unwrap().len(), 0);
        assert!(raw.get("frozen_request_prefix").is_none());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(
            raw["browser_meta"]["browser_runtime_id"],
            "browser-immediate"
        );
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 44);
    }

    #[tokio::test]
    async fn mode_transition_pending_external_reload_persists_provider_cleanup_when_idle() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-pending-external-reload-provider";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Pending Transition External Reload",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-pending",
                    "tab_urls": ["https://example.com/pending"],
                    "active_tab_id": "tab-pending",
                    "attach_screenshot_on_send": true
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 45}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.runtime.state = SessionState::Generating;
            session.trajectory_dirty = true;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change(gcx.clone(), chat_id, false).await;
        {
            let session = session_arc.lock().await;
            assert!(session.external_reload_pending);
            assert!(session.thread.claude_code_identity.is_none());
            assert!(session.thread.frozen_request_prefix.is_none());
        }

        {
            let mut session = session_arc.lock().await;
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }
        check_external_reload_pending(gcx, session_arc.clone()).await;

        {
            let session = session_arc.lock().await;
            assert!(!session.external_reload_pending);
            let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
            assert_eq!(
                prefix.system_prompt.as_deref(),
                Some("target task planner system")
            );
            assert!(prefix.tools_canonical.is_none());
            assert!(session.thread.claude_code_identity.is_none());
            assert!(session.thread.previous_response_id.is_none());
            assert!(!session.trajectory_dirty);
        }
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "target task planner system"
        );
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(raw["browser_meta"]["browser_runtime_id"], "browser-pending");
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 45);
    }

    #[tokio::test]
    async fn mode_transition_no_active_session_external_change_persists_provider_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-no-active-session-repair";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "No Active Session Repair",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-1",
                    "tab_urls": ["https://example.com"],
                    "active_tab_id": "tab-1",
                    "attach_screenshot_on_send": true
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 42}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        process_trajectory_change(gcx, chat_id, false).await;

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "target task planner system"
        );
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(raw["browser_meta"]["browser_runtime_id"], "browser-1");
        assert_eq!(
            raw["browser_meta"]["tab_urls"],
            json!(["https://example.com"])
        );
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 42);
    }

    #[tokio::test]
    async fn repair_source_id_mismatch_errors_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "repair-source-original";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Repair Source Original",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "custom_future_field": {"keep": true}
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let mut loaded = load_trajectory_for_chat(gcx.clone(), chat_id)
            .await
            .unwrap();
        assert!(loaded.transition_identity_repaired);
        apply_mode_defaults_to_thread(
            gcx.clone(),
            &mut loaded.thread,
            loaded.auto_approve_editing_tools_present,
            loaded.auto_approve_dangerous_commands_present,
        )
        .await;
        let repair_patch = loaded.repair_patch();
        let replacement = serde_json::to_string(&json!({
            "id": "repair-source-replacement",
            "title": "Replacement Must Stay",
            "model": "model",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "previous_response_id": "replacement-response"
        }))
        .unwrap();
        tokio::fs::write(&path, &replacement).await.unwrap();

        let err = persist_loaded_trajectory_repair_raw(&repair_patch)
            .await
            .unwrap_err();

        assert!(err.contains("Trajectory source id mismatch for repair"));
        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), replacement);
    }

    #[tokio::test]
    async fn mode_transition_repair_persists_to_loaded_task_path_with_normal_collision() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-task-normal-collision-repair";
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-loaded-repair")
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(task_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &task_path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Task Collision Repair",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "task-response",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "task_meta": {
                    "task_id": "task-loaded-repair",
                    "role": "agents",
                    "agent_id": "agent-1",
                    "card_id": "T-1"
                },
                "custom_future_field": {"task": true}
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let mut loaded = load_trajectory_for_chat(gcx.clone(), chat_id)
            .await
            .unwrap();
        assert_eq!(loaded.source_path, task_path);
        assert!(loaded.transition_identity_repaired);
        apply_mode_defaults_to_thread(
            gcx.clone(),
            &mut loaded.thread,
            loaded.auto_approve_editing_tools_present,
            loaded.auto_approve_dangerous_commands_present,
        )
        .await;
        let repair_patch = loaded.repair_patch();

        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(normal_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &normal_path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Normal Collision Must Stay",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "previous_response_id": "normal-response",
                "custom_future_field": {"normal": true}
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let normal_before = tokio::fs::read_to_string(&normal_path).await.unwrap();

        persist_loaded_trajectory_repair_raw(&repair_patch)
            .await
            .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&normal_path).await.unwrap(),
            normal_before
        );
        let task_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&task_path).await.unwrap()).unwrap();
        assert_eq!(
            task_raw["frozen_request_prefix"]["system_prompt"],
            "target task planner system"
        );
        assert!(task_raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert!(task_raw.get("claude_code_identity").is_none());
        assert!(task_raw.get("previous_response_id").is_none());
        assert_eq!(task_raw["custom_future_field"]["task"], true);
    }

    #[tokio::test]
    async fn handoff_invalid_or_missing_created_at_repairs_and_normalizes_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();

        for (chat_id, created_at) in [
            ("handoff-invalid-created-at-open", Some(json!("not-a-date"))),
            ("handoff-missing-created-at-open", None),
        ] {
            let path = trajectories_dir.join(format!("{chat_id}.json"));
            let mut payload = json!({
                "id": chat_id,
                "title": "Handoff Open",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "handoff",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target handoff system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-03T00:00:00Z",
                    "system_prompt": "target handoff system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true
            });
            if let Some(created_at) = created_at {
                payload["created_at"] = created_at;
            }
            tokio::fs::write(&path, serde_json::to_string(&payload).unwrap())
                .await
                .unwrap();

            let session_arc = crate::chat::get_or_create_session_with_trajectory(
                app.clone(),
                &app.chat.sessions,
                chat_id,
            )
            .await;
            {
                let session = session_arc.lock().await;
                let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
                assert_eq!(
                    prefix.system_prompt.as_deref(),
                    Some("target handoff system")
                );
                assert!(prefix.tools_canonical.is_none());
                assert!(session.thread.claude_code_identity.is_none());
                assert!(session.thread.previous_response_id.is_none());
                assert!(parsed_rfc3339_utc(&session.created_at).is_some());
            }

            let raw: serde_json::Value =
                serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
            let saved_created_at = raw["created_at"].as_str().unwrap();
            assert!(parsed_rfc3339_utc(saved_created_at).is_some());
            assert_ne!(saved_created_at, "not-a-date");
            assert_eq!(
                raw["frozen_request_prefix"]["system_prompt"],
                "target handoff system"
            );
            assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
            assert!(raw.get("claude_code_identity").is_none());
            assert!(raw.get("previous_response_id").is_none());
        }
    }

    #[test]
    fn frozen_lazy_ensure_noops_on_empty_inputs_and_preserves_existing_prefix() {
        let mut session = ChatSession::new("lazy-frozen".to_string());
        assert!(ensure_frozen_prefix(&mut session, None, None).is_none());
        assert!(session.thread.frozen_request_prefix.is_none());
        assert_eq!(session.trajectory_version, 0);

        let no_tools = ensure_frozen_prefix(
            &mut session,
            Some("system no tools".to_string()),
            Some(json!([])),
        )
        .expect("known no-tools prefix should be installed");
        assert_eq!(no_tools.tools_canonical, Some(json!([])));
        assert!(frozen_prefix_is_complete(&no_tools));
        let version_after_no_tools = session.trajectory_version;

        let with_tools = ensure_frozen_prefix(
            &mut session,
            Some("system with tools".to_string()),
            Some(json!([{"type":"function","function":{"name":"cat"}}])),
        );
        assert!(with_tools.is_none());
        assert_eq!(session.trajectory_version, version_after_no_tools);
        assert_eq!(
            session
                .thread
                .frozen_request_prefix
                .as_ref()
                .and_then(|prefix| prefix.tools_canonical.clone()),
            Some(json!([]))
        );

        let mut session = ChatSession::new("lazy-frozen".to_string());

        let first = ensure_frozen_prefix(
            &mut session,
            Some("system one".to_string()),
            Some(json!([{"type":"function","function":{"name":"cat"}}])),
        )
        .expect("missing prefix should be installed");

        assert_eq!(first.schema_version, 1);
        assert_eq!(first.system_prompt.as_deref(), Some("system one"));
        assert!(session.thread.frozen_request_prefix.is_some());
        let version_after_first = session.trajectory_version;

        let second = ensure_frozen_prefix(
            &mut session,
            Some("system two".to_string()),
            Some(json!([{"type":"function","function":{"name":"shell"}}])),
        );
        assert!(second.is_none());
        assert_eq!(session.trajectory_version, version_after_first);
        let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
        assert_eq!(prefix.system_prompt.as_deref(), Some("system one"));
        assert_eq!(
            prefix.tools_canonical,
            Some(json!([{"type":"function","function":{"name":"cat"}}]))
        );
    }

    #[test]
    fn frozen_lazy_ensure_fills_partial_prefix_once() {
        let mut session = ChatSession::new("partial-frozen".to_string());
        let partial = ensure_frozen_prefix(&mut session, Some("system only".to_string()), None)
            .expect("partial prefix should be installed");
        assert_eq!(partial.system_prompt.as_deref(), Some("system only"));
        assert!(partial.tools_canonical.is_none());
        let version_after_partial = session.trajectory_version;

        let filled = ensure_frozen_prefix(
            &mut session,
            Some("replacement system".to_string()),
            Some(json!([{"type":"function","function":{"name":"cat"}}])),
        )
        .expect("partial prefix should be filled");
        assert_eq!(filled.system_prompt.as_deref(), Some("system only"));
        assert_eq!(
            filled.tools_canonical,
            Some(json!([{"type":"function","function":{"name":"cat"}}]))
        );
        assert_eq!(session.trajectory_version, version_after_partial + 1);

        let version_after_fill = session.trajectory_version;
        assert!(ensure_frozen_prefix(
            &mut session,
            Some("new system".to_string()),
            Some(json!([{"type":"function","function":{"name":"shell"}}])),
        )
        .is_none());
        assert_eq!(session.trajectory_version, version_after_fill);
        let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
        assert_eq!(prefix.system_prompt.as_deref(), Some("system only"));
        assert_eq!(
            prefix.tools_canonical,
            Some(json!([{"type":"function","function":{"name":"cat"}}]))
        );
    }

    #[test]
    fn frozen_complete_existing_prefix_is_never_overwritten() {
        let mut session = ChatSession::new("complete-frozen".to_string());
        let original = FrozenRequestPrefix {
            schema_version: 1,
            created_at: "2026-05-29T00:00:00Z".to_string(),
            system_prompt: Some("original system".to_string()),
            tools_canonical: Some(json!([])),
        };
        session.thread.frozen_request_prefix = Some(original.clone());

        let result = ensure_frozen_prefix(
            &mut session,
            Some("replacement system".to_string()),
            Some(json!([{"type":"function","function":{"name":"cat"}}])),
        );

        assert!(result.is_none());
        assert_eq!(session.thread.frozen_request_prefix, Some(original));
        assert_eq!(session.trajectory_version, 0);
    }

    #[test]
    fn frozen_racy_persist_prefix_helper_is_not_available_in_production() {
        let _helper = persist_frozen_prefix;
    }

    #[tokio::test]
    async fn frozen_legacy_normal_session_migrates_through_session_save() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "legacy-freeze-normal";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        tokio::fs::write(
            trajectories_dir.join(format!("{chat_id}.json")),
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Legacy",
                "model": "test/model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [
                    {"role":"system","content":"legacy system"},
                    {"role":"user","content":"hello"}
                ],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await;
        {
            let session = session_arc.lock().await;
            let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
            assert_eq!(prefix.system_prompt.as_deref(), Some("legacy system"));
            assert!(prefix.tools_canonical.is_none());
        }

        let raw: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(trajectories_dir.join(format!("{chat_id}.json")))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "legacy system"
        );
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert_eq!(raw["messages"][0]["content"], "legacy system");
    }

    #[tokio::test]
    async fn frozen_legacy_unknown_tools_can_be_completed_once() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "legacy-freeze-complete-once";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        tokio::fs::write(
            trajectories_dir.join(format!("{chat_id}.json")),
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Legacy",
                "model": "test/model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [
                    {"role":"system","content":"legacy system"},
                    {"role":"user","content":"hello"}
                ],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await;

        let mut session = session_arc.lock().await;
        let completed = ensure_frozen_prefix(
            &mut session,
            Some("replacement system".to_string()),
            Some(json!([{"type":"function","function":{"name":"cat"}}])),
        )
        .expect("legacy unknown tools prefix should complete once");
        assert_eq!(completed.system_prompt.as_deref(), Some("legacy system"));
        assert_eq!(
            completed.tools_canonical,
            Some(json!([{"type":"function","function":{"name":"cat"}}]))
        );
        let version_after_complete = session.trajectory_version;

        assert!(ensure_frozen_prefix(
            &mut session,
            Some("new system".to_string()),
            Some(json!([{"type":"function","function":{"name":"shell"}}])),
        )
        .is_none());
        assert_eq!(session.trajectory_version, version_after_complete);
        assert_eq!(
            session
                .thread
                .frozen_request_prefix
                .as_ref()
                .and_then(|prefix| prefix.tools_canonical.clone()),
            Some(json!([{"type":"function","function":{"name":"cat"}}]))
        );
    }

    #[tokio::test]
    async fn frozen_legacy_task_session_migration_does_not_create_generic_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let task_id = "task-legacy-freeze";
        let agent_id = "agent-1";
        let chat_id = "legacy-task-freeze";
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join(task_id)
            .join("trajectories")
            .join("agents")
            .join(agent_id)
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(task_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &task_path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Legacy Task",
                "model": "model",
                "mode": "task_agent",
                "tool_use": "agent",
                "messages": [
                    {"role":"system","content":"task system"},
                    {"role":"user","content":"hello task"}
                ],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "task_meta": {
                    "task_id": task_id,
                    "role": "agents",
                    "agent_id": agent_id,
                    "card_id": "T-1"
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::create_dir_all(dir.path().join(".refact").join("tasks").join(task_id))
            .await
            .unwrap();
        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await;
        {
            let session = session_arc.lock().await;
            let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
            assert_eq!(prefix.system_prompt.as_deref(), Some("task system"));
            assert!(prefix.tools_canonical.is_none());
        }

        let generic_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&task_path).await.unwrap()).unwrap();
        assert!(!tokio::fs::try_exists(generic_path).await.unwrap());
        assert_eq!(raw["frozen_request_prefix"]["system_prompt"], "task system");
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
    }

    #[tokio::test]
    async fn frozen_empty_normal_snapshot_with_prefix_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let mut snapshot = test_snapshot("empty-frozen-normal", "Frozen Empty", vec![]);
        snapshot.frozen_request_prefix = Some(new_frozen_request_prefix(
            Some("empty snapshot system".to_string()),
            json!([]),
        ));

        save_trajectory_snapshot(gcx, snapshot).await.unwrap();

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join("empty-frozen-normal.json");
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "empty snapshot system"
        );
        assert_eq!(raw["messages"].as_array().unwrap().len(), 0);
    }
    #[tokio::test]
    async fn trajectory_updated_at_changes_when_title_changes_without_messages() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;

        let chat_id = "updated-at-title-change";
        let messages = vec![ChatMessage::new("user".to_string(), "Hello".to_string())];
        save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(chat_id, "First", messages.clone()),
        )
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

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        save_trajectory_snapshot(gcx, test_snapshot(chat_id, "Retitled", messages))
            .await
            .unwrap();
        let retitled_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_ne!(
            retitled_raw["updated_at"].as_str().unwrap(),
            first_updated_at
        );
    }

    #[tokio::test]
    async fn trajectory_updated_at_changes_when_worktree_changes() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;

        let chat_id = "updated-at-worktree-change";
        let messages = vec![ChatMessage::new("user".to_string(), "Hello".to_string())];
        save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(chat_id, "Title", messages.clone()),
        )
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

        let mut snapshot = test_snapshot(chat_id, "Title", messages);
        snapshot.worktree = Some(trajectory_worktree_sample());
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        save_trajectory_snapshot(gcx, snapshot).await.unwrap();
        let changed_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_ne!(
            changed_raw["updated_at"].as_str().unwrap(),
            first_updated_at
        );
    }

    #[tokio::test]
    async fn trajectory_persistence_preserves_ui_only_messages() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let mut session = ChatSession::new("ui-only-roundtrip".to_string());
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
        session.add_message(make_ui_only_error_message("context_length_exceeded"));

        let snapshot = trajectory_snapshot_from_session(&session);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "ui-only-roundtrip")
            .await
            .unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].role, "user");
        assert_eq!(loaded.messages[1].role, "error");
        assert!(is_ui_only_message(&loaded.messages[1]));
    }

    #[tokio::test]
    async fn cache_guard_snapshot_is_runtime_only_and_not_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let raw_request_marker = "raw-cache-guard-request-body-marker";
        let mut session = ChatSession::new("cache-guard-runtime-only".to_string());
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.cache_guard_snapshot = Some(json!({
            "messages": [{"role": "user", "content": raw_request_marker}],
            "provider_specific_fields": {"semantic": "value"}
        }));
        session.add_message(ChatMessage::new("user".to_string(), "visible".to_string()));

        let snapshot = trajectory_snapshot_from_session(&session);
        let snapshot_messages_json = serde_json::to_string(&snapshot.messages).unwrap();
        assert!(!snapshot_messages_json.contains(raw_request_marker));

        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join("cache-guard-runtime-only.json");
        let saved_json = tokio::fs::read_to_string(path).await.unwrap();
        assert!(!saved_json.contains("cache_guard_snapshot"));
        assert!(!saved_json.contains(raw_request_marker));
    }

    #[tokio::test]
    async fn reactive_compact_attempts_roundtrip_and_clamp() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let mut snapshot = test_snapshot(
            "reactive-attempts-roundtrip",
            "Reactive Attempts",
            vec![ChatMessage::new("user".to_string(), "visible".to_string())],
        );
        snapshot.reactive_compact_attempts = Some(2);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx.clone(), "reactive-attempts-roundtrip")
            .await
            .unwrap();
        assert_eq!(loaded.thread.reactive_compact_attempts, Some(2));

        let traj_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join("reactive-attempts-roundtrip.json");
        let mut raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&traj_path).await.unwrap()).unwrap();
        raw["reactive_compact_attempts"] = json!(99);
        tokio::fs::write(&traj_path, serde_json::to_string(&raw).unwrap())
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "reactive-attempts-roundtrip")
            .await
            .unwrap();
        assert_eq!(loaded.thread.reactive_compact_attempts, Some(1));
    }

    #[tokio::test]
    async fn trajectory_persistence_keeps_normal_error_messages() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let mut session = ChatSession::new("normal-error-roundtrip".to_string());
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
        session.add_message(ChatMessage::new(
            "error".to_string(),
            "LLM failed".to_string(),
        ));

        let snapshot = trajectory_snapshot_from_session(&session);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "normal-error-roundtrip")
            .await
            .unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[1].role, "error");
        assert_eq!(loaded.messages[1].content.content_text_only(), "LLM failed");
    }

    #[test]
    fn trajectory_save_uses_unique_tmp() {
        let file_path = PathBuf::from("chat.json");
        let first = unique_trajectory_tmp_path(&file_path);
        let second = unique_trajectory_tmp_path(&file_path);

        assert_ne!(first, second);
        assert_eq!(
            first
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .len(),
            22
        );
        assert!(first
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap()
            .starts_with("chat.json.tmp."));
        assert!(second
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap()
            .starts_with("chat.json.tmp."));
    }

    #[tokio::test]
    async fn trajectory_save_cleans_up_on_error() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("chat.json");
        let tmp_path = unique_trajectory_tmp_path(&file_path);
        tokio::fs::write(&tmp_path, "stale").await.unwrap();

        let err = atomic_write_json_with_tmp_path(
            &file_path,
            &tmp_path,
            Err("Failed to serialize trajectory: injected".to_string()),
            Some("Failed to write trajectory"),
        )
        .await
        .unwrap_err();

        assert_eq!(err, "Failed to serialize trajectory: injected");
        assert!(!tmp_path.exists());
        let leftovers = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| name.contains(".tmp"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[tokio::test]
    async fn trajectory_worktree_old_json_without_worktree_loads_none() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
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
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let app = AppState::from_gcx(gcx.clone()).await;
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
        let listed = list_all_trajectories_meta(app).await.unwrap();
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
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let _app = AppState::from_gcx(gcx.clone()).await;

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
        let project_hash = crate::worktrees::service::project_hash_for_path(
            &dunce::simplified(&source.canonicalize().unwrap()).to_path_buf(),
        );
        let worktree_cache_dir = cache.join("worktrees").join(&project_hash);
        std::fs::create_dir_all(&worktree_cache_dir).unwrap();
        let agent_worktree = worktree_cache_dir.join("agent-worktree");
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
                comments: vec![],
                final_report: None,
                final_report_structured: None,
                verifier_report: None,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                started_at: None,
                last_heartbeat_at: None,
                completed_at: None,
                agent_branch: Some("refact/task/card".to_string()),
                agent_worktree: Some(agent_worktree.to_string_lossy().to_string()),
                agent_worktree_name: Some("wt-legacy".to_string()),
                ab_variants: None,
                team_members: vec![],
                target_files: Vec::new(),
                scope_guard_mode: Default::default(),
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
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let _app = AppState::from_gcx(gcx.clone()).await;

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
                comments: vec![],
                final_report: None,
                final_report_structured: None,
                verifier_report: None,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                started_at: None,
                last_heartbeat_at: None,
                completed_at: None,
                agent_branch: Some("refact/task/card-mismatch".to_string()),
                agent_worktree: Some(agent_worktree.to_string_lossy().to_string()),
                agent_worktree_name: Some("wt-legacy-mismatch".to_string()),
                ab_variants: None,
                team_members: vec![],
                target_files: Vec::new(),
                scope_guard_mode: Default::default(),
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
