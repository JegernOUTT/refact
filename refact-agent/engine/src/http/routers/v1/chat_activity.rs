use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use axum::extract::State;
use axum::Json;
use hyper::StatusCode;
use refact_core::recovery::RecoveryState;
use serde::Serialize;

use crate::app_state::AppState;
use crate::chat::trajectories::is_internal_trace_link_type;
use crate::chat::types::{ChatSession, SessionState};
use crate::custom_error::ScratchError;
use crate::scheduler::{session_cron_store, Job, Trigger};

#[derive(Debug, Serialize)]
pub struct ChatActivity {
    pub id: String,
    pub title: String,
    pub updated_at_ms: i64,
    pub session_state: Option<String>,
    pub recovery_state: Option<RecoveryState>,
    pub schedule_count: usize,
    pub paused_schedule_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ChatActivityResponse {
    pub chats: Vec<ChatActivity>,
}

fn attention(state: RecoveryState) -> bool {
    matches!(
        state,
        RecoveryState::Waiting
            | RecoveryState::Attempting
            | RecoveryState::NeedsAction
            | RecoveryState::Exhausted
    )
}

fn live_chat(
    session: &ChatSession,
    root: &Path,
    known_project_chat: bool,
    now_ms: i64,
) -> Option<ChatActivity> {
    let in_project = match session.work_project_root.as_deref() {
        Some(session_root) => {
            crate::files_correction::canonicalize_normalized_path(session_root.to_path_buf())
                == root
        }
        None => known_project_chat,
    };
    if session.closed
        || !in_project
        || session.thread.mode == "buddy"
        || session.thread.buddy_meta.is_some()
        || is_internal_trace_link_type(session.thread.link_type.as_deref())
    {
        return None;
    }
    Some(ChatActivity {
        id: session.chat_id.clone(),
        title: session.thread.title.clone(),
        updated_at_ms: now_ms.saturating_sub(
            session
                .last_activity
                .elapsed()
                .as_millis()
                .min(i64::MAX as u128) as i64,
        ),
        session_state: Some(session.runtime.state.to_string()),
        recovery_state: session.runtime.recovery.as_ref().map(|r| r.state),
        schedule_count: 0,
        paused_schedule_count: 0,
    })
}

fn add_schedules(chats: &mut BTreeMap<String, ChatActivity>, jobs: Vec<Job>) {
    let mut seen = HashSet::new();
    for job in jobs {
        if !matches!(
            job.trigger,
            Trigger::Cron { .. } | Trigger::Interval { .. } | Trigger::Once { .. }
        ) || !seen.insert(job.id.clone())
        {
            continue;
        }
        let Some(chat) = job.chat_id().and_then(|id| chats.get_mut(id)) else {
            continue;
        };
        chat.schedule_count += 1;
        if !job.enabled || job.paused_at_ms.is_some() {
            chat.paused_schedule_count += 1;
        }
    }
}

async fn real_directory(path: &Path) -> Result<bool, String> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Cannot inspect chat activity directory: {error}")),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Chat activity storage must be a real directory".into());
    }
    Ok(true)
}

async fn child_directories(path: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    if !real_directory(path).await? {
        return Ok(Vec::new());
    }
    let mut reader = tokio::fs::read_dir(path)
        .await
        .map_err(|error| format!("Cannot read chat activity directory: {error}"))?;
    let mut directories = Vec::new();
    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|error| format!("Cannot list chat activity directory: {error}"))?
    {
        if entry
            .file_type()
            .await
            .map_err(|error| format!("Cannot inspect chat activity entry: {error}"))?
            .is_dir()
        {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

async fn activity_directories(root: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let storage = root.join(".refact");
    if !real_directory(&storage).await? {
        return Ok(Vec::new());
    }
    let mut directories = Vec::new();
    let chats = storage.join("trajectories");
    if real_directory(&chats).await? {
        directories.push(chats);
    }
    for task in child_directories(&storage.join("tasks")).await? {
        let trajectories = task.join("trajectories");
        let mut pending: Vec<_> = child_directories(&trajectories)
            .await?
            .into_iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        matches!(name, "planner" | "linked" | "agents" | "subchats")
                    })
            })
            .collect();
        while let Some(directory) = pending.pop() {
            pending.extend(child_directories(&directory).await?);
            directories.push(directory);
        }
    }
    Ok(directories)
}

async fn activity(app: AppState) -> Result<ChatActivityResponse, String> {
    let Some(root) = crate::files_correction::get_active_project_path(app.gcx.clone()).await else {
        return Ok(ChatActivityResponse {
            chats: Vec::new(),
        });
    };
    let root = crate::files_correction::canonicalize_normalized_path(root);
    let mut chats = BTreeMap::new();
    let mut invalid_recovery = HashSet::new();
    for directory in activity_directories(&root).await? {
        let entries = app
            .chat
            .trajectory_index_coordinator
            .list_entries_shared(&directory, None)
            .await?;
        for entry in entries.iter().filter(|e| {
            e.mode != "buddy"
                && !matches!(
                    e.source,
                    crate::chat::trajectory_index::TrajectoryIndexSource::Buddy
                )
                && !is_internal_trace_link_type(e.link_type.as_deref())
        }) {
            if chats.contains_key(&entry.id) {
                continue;
            }
            if entry.recovery_projection_version == 0 || entry.recovery_projection_error {
                invalid_recovery.insert(entry.id.clone());
            }
            chats.insert(
                entry.id.clone(),
                ChatActivity {
                    id: entry.id.clone(),
                    title: entry.title.clone(),
                    updated_at_ms: chrono::DateTime::parse_from_rfc3339(&entry.updated_at)
                        .map(|timestamp| timestamp.timestamp_millis())
                        .unwrap_or(0),
                    session_state: None,
                    recovery_state: entry.recovery_state,
                    schedule_count: 0,
                    paused_schedule_count: 0,
                },
            );
        }
    }
    let sessions: Vec<_> = app.chat.sessions.read().await.values().cloned().collect();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut working = HashSet::new();
    for session in sessions {
        let session = session.lock().await;
        if let Some(chat) = live_chat(
            &session,
            &root,
            chats.contains_key(&session.chat_id),
            now_ms,
        ) {
            if matches!(
                session.runtime.state,
                SessionState::Generating | SessionState::ExecutingTools
            ) {
                working.insert(chat.id.clone());
            }
            invalid_recovery.remove(&chat.id);
            chats.insert(chat.id.clone(), chat);
        } else {
            invalid_recovery.remove(&session.chat_id);
            chats.remove(&session.chat_id);
        }
    }
    if !invalid_recovery.is_empty() {
        return Err(
            "Chat activity is unavailable because saved recovery metadata is invalid".into(),
        );
    }
    let mut jobs = session_cron_store().list().await;
    jobs.extend(crate::scheduler::store::read_durable_jobs(&root)?);
    add_schedules(&mut chats, jobs);
    let mut chats: Vec<_> = chats
        .into_values()
        .filter(|c| {
            working.contains(&c.id)
                || c.recovery_state.is_some_and(attention)
                || c.schedule_count > 0
        })
        .collect();
    chats.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(ChatActivityResponse {
        chats,
    })
}

pub async fn handle_v1_chats_activity(
    State(app): State<AppState>,
) -> Result<Json<ChatActivityResponse>, ScratchError> {
    activity(app)
        .await
        .map(Json)
        .map_err(|error| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, error))
}

#[cfg(test)]
#[path = "chat_activity_tests.rs"]
mod tests;
