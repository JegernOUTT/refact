use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use crate::pickers::PickerItem;
use crate::protocol::TranscriptMessage;

fn deserialize_default_on_null<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PaginatedTrajectories {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub items: Vec<TrajectoryMeta>,
    pub next_cursor: Option<String>,
    pub has_more: Option<bool>,
    pub total_count: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeMeta {
    pub id: Option<String>,
    pub kind: Option<String>,
    pub root: Option<PathBuf>,
    pub source_workspace_root: Option<PathBuf>,
    pub repo_root: Option<PathBuf>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub branch: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub base_branch: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub base_commit: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub task_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub card_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub agent_id: Option<String>,
    pub enforce: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TrajectoryMeta {
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub title: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub created_at: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub updated_at: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub model: String,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub mode: String,
    pub message_count: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub parent_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub link_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub task_id: Option<String>,
    #[serde(
        default,
        alias = "role",
        deserialize_with = "deserialize_default_on_null"
    )]
    pub task_role: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub agent_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub card_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub session_state: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub root_chat_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub worktree: Option<WorktreeMeta>,
    pub total_lines_added: Option<i64>,
    pub total_lines_removed: Option<i64>,
    pub tasks_total: Option<i32>,
    pub tasks_done: Option<i32>,
    pub tasks_failed: Option<i32>,
    pub total_prompt_tokens: Option<u64>,
    pub total_completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub total_cache_read_tokens: Option<u64>,
    pub total_cache_creation_tokens: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub total_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTab {
    pub id: String,
    pub title: String,
    pub description: String,
    pub is_current: bool,
}

pub fn session_items_from_trajectories(
    mut trajectories: Vec<TrajectoryMeta>,
    now: DateTime<Utc>,
) -> Vec<PickerItem> {
    trajectories.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    trajectories
        .into_iter()
        .map(|trajectory| session_picker_item(&trajectory, now))
        .collect()
}

pub fn session_picker_item(trajectory: &TrajectoryMeta, now: DateTime<Utc>) -> PickerItem {
    let title = display_title(&trajectory.title);
    let mut parts = vec![age_label(&trajectory.updated_at, now)];
    if !trajectory.model.trim().is_empty() {
        parts.push(trajectory.model.clone());
    }
    if !trajectory.mode.trim().is_empty() {
        parts.push(trajectory.mode.clone());
    }
    parts.push(
        trajectory
            .message_count
            .map(message_count_label)
            .unwrap_or_else(|| "unknown messages".to_string()),
    );
    parts.push(short_chat_id(&trajectory.id));
    if let Some(state) = trajectory
        .session_state
        .as_deref()
        .filter(|state| !state.trim().is_empty())
    {
        parts.push(state.to_string());
    }
    PickerItem {
        id: trajectory.id.clone(),
        title,
        description: parts.join(" · "),
    }
}

pub fn session_tab_from_picker_item(item: PickerItem, current_chat_id: &str) -> SessionTab {
    SessionTab {
        is_current: item.id == current_chat_id,
        id: item.id,
        title: item.title,
        description: item.description,
    }
}

pub fn session_subtitle(
    project: Option<&str>,
    model: Option<&str>,
    mode: Option<&str>,
    chat_id: &str,
) -> String {
    let project = project.unwrap_or("no project");
    let model = model
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("default");
    let mode = mode
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("agent");
    format!("{project} · {model} · {mode} · {}", short_chat_id(chat_id))
}

pub fn display_title(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        "Untitled chat".to_string()
    } else {
        title.to_string()
    }
}

pub fn short_chat_id(chat_id: &str) -> String {
    chat_id.chars().take(8).collect()
}

pub fn age_label(updated_at: &str, now: DateTime<Utc>) -> String {
    let Ok(updated) = DateTime::parse_from_rfc3339(updated_at) else {
        return if updated_at.trim().is_empty() {
            "unknown age".to_string()
        } else {
            updated_at.to_string()
        };
    };
    let seconds = now
        .signed_duration_since(updated.with_timezone(&Utc))
        .num_seconds()
        .max(0);
    if seconds < 60 {
        "just now".to_string()
    } else if seconds < 60 * 60 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 60 * 60 * 24 {
        format!("{}h ago", seconds / 60 / 60)
    } else if seconds < 60 * 60 * 24 * 7 {
        format!("{}d ago", seconds / 60 / 60 / 24)
    } else {
        updated.format("%Y-%m-%d").to_string()
    }
}

pub fn last_branch_message_id(messages: &[TranscriptMessage]) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        message
            .message_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string)
    })
}

fn message_count_label(count: usize) -> String {
    match count {
        1 => "1 message".to_string(),
        count => format!("{count} messages"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    #[test]
    fn trajectory_meta_roundtrips_all_daemon_fields() {
        let fixture = json!({
            "id": "chat-123",
            "title": "Complete history",
            "created_at": "2026-06-10T10:00:00Z",
            "updated_at": "2026-06-11T11:00:00Z",
            "model": "gpt-5.6",
            "mode": "task",
            "message_count": 8,
            "parent_id": "parent-123",
            "link_type": "subagent",
            "task_id": "task-123",
            "task_role": "agents",
            "agent_id": "agent-123",
            "card_id": "T-36",
            "session_state": "idle",
            "root_chat_id": "root-123",
            "worktree": {
                "id": "worktree-123",
                "kind": "task_agent",
                "root": "/tmp/worktree",
                "source_workspace_root": "/tmp/source",
                "repo_root": "/tmp/source",
                "branch": "refact/task/T-36",
                "base_branch": "main",
                "base_commit": "abc123",
                "task_id": "task-123",
                "card_id": "T-36",
                "agent_id": "agent-123",
                "enforce": true
            },
            "total_lines_added": 100,
            "total_lines_removed": 25,
            "tasks_total": 4,
            "tasks_done": 3,
            "tasks_failed": 1,
            "total_prompt_tokens": 1000,
            "total_completion_tokens": 500,
            "total_tokens": 1500,
            "total_cache_read_tokens": 250,
            "total_cache_creation_tokens": 75,
            "total_cost_usd": 0.042
        });

        let trajectory: TrajectoryMeta = serde_json::from_value(fixture.clone()).unwrap();

        assert_eq!(trajectory.task_role.as_deref(), Some("agents"));
        assert_eq!(
            trajectory.worktree.as_ref().unwrap().branch.as_deref(),
            Some("refact/task/T-36")
        );
        assert_eq!(serde_json::to_value(trajectory).unwrap(), fixture);
    }
}
