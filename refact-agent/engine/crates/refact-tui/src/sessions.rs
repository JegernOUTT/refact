use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::pickers::PickerItem;
use crate::protocol::TranscriptMessage;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaginatedTrajectories {
    #[serde(default)]
    pub items: Vec<TrajectoryMeta>,
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub total_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeMeta {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub root: PathBuf,
    #[serde(default)]
    pub source_workspace_root: PathBuf,
    #[serde(default)]
    pub repo_root: PathBuf,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub base_commit: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub card_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub enforce: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TrajectoryMeta {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub message_count: usize,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub link_type: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default, alias = "role")]
    pub task_role: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub card_id: Option<String>,
    #[serde(default)]
    pub session_state: Option<String>,
    #[serde(default)]
    pub root_chat_id: Option<String>,
    #[serde(default)]
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
    #[serde(default)]
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
    parts.push(message_count_label(trajectory.message_count));
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
    use crate::protocol::{TranscriptMessage, TranscriptRole};
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-06-12T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn session_picker_items_sort_by_recency_and_format_age() {
        let items = session_items_from_trajectories(
            vec![
                TrajectoryMeta {
                    id: "older-chat-id".to_string(),
                    title: "Older".to_string(),
                    created_at: String::new(),
                    updated_at: "2026-06-10T12:00:00Z".to_string(),
                    model: "gpt-old".to_string(),
                    mode: "agent".to_string(),
                    message_count: 1,
                    parent_id: None,
                    link_type: None,
                    session_state: None,
                    root_chat_id: None,
                    ..Default::default()
                },
                TrajectoryMeta {
                    id: "newer-chat-id".to_string(),
                    title: "Newer".to_string(),
                    created_at: String::new(),
                    updated_at: "2026-06-12T11:00:00Z".to_string(),
                    model: "gpt-new".to_string(),
                    mode: "explore".to_string(),
                    message_count: 3,
                    parent_id: None,
                    link_type: None,
                    session_state: Some("idle".to_string()),
                    root_chat_id: None,
                    ..Default::default()
                },
            ],
            now(),
        );

        assert_eq!(items[0].id, "newer-chat-id");
        assert_eq!(items[0].title, "Newer");
        assert!(items[0].description.contains("1h ago"));
        assert!(items[0].description.contains("gpt-new"));
        assert!(items[0].description.contains("3 messages"));
        assert_eq!(
            items[1].description,
            "2d ago · gpt-old · agent · 1 message · older-ch"
        );
    }

    #[test]
    fn session_picker_filter_matches_title_model_and_chat_id() {
        let items = session_items_from_trajectories(
            vec![TrajectoryMeta {
                id: "abc12345-chat".to_string(),
                title: "Fix tests".to_string(),
                created_at: String::new(),
                updated_at: "2026-06-12T11:55:00Z".to_string(),
                model: "claude-demo".to_string(),
                mode: "agent".to_string(),
                message_count: 2,
                parent_id: None,
                link_type: None,
                session_state: None,
                root_chat_id: None,
                ..Default::default()
            }],
            now(),
        );
        let mut picker =
            crate::pickers::PickerState::new(crate::pickers::PickerKind::Session, items);

        picker.set_filter("claude");
        assert_eq!(picker.filtered_items()[0].id, "abc12345-chat");
        picker.set_filter("abc123");
        assert_eq!(picker.filtered_items()[0].id, "abc12345-chat");
    }

    #[test]
    fn last_branch_message_id_uses_last_backend_message_id() {
        let mut user = TranscriptMessage::new(TranscriptRole::User);
        user.message_id = Some("u1".to_string());
        let mut assistant = TranscriptMessage::new(TranscriptRole::Assistant);
        assistant.message_id = Some("a1".to_string());

        assert_eq!(
            last_branch_message_id(&[user, assistant]),
            Some("a1".to_string())
        );
    }

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

    #[test]
    fn trajectory_response_accepts_missing_optional_fields() {
        let response: PaginatedTrajectories =
            serde_json::from_value(json!({"items": [{"id": "chat-123"}]})).unwrap();
        let trajectory = &response.items[0];

        assert_eq!(trajectory.id, "chat-123");
        assert!(trajectory.title.is_empty());
        assert_eq!(trajectory.message_count, 0);
        assert_eq!(trajectory.task_id, None);
        assert_eq!(trajectory.worktree, None);
        assert_eq!(trajectory.total_cost_usd, None);
        assert_eq!(response.next_cursor, None);
        assert!(!response.has_more);
        assert_eq!(response.total_count, 0);
    }
}
