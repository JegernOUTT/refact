use refact_chat_api::WorktreeMeta;
use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn event_with_all_fields() -> TrajectoryEvent {
        TrajectoryEvent {
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
            worktree: Some(WorktreeMeta {
                id: "wt-1".to_string(),
                kind: "task_agent".to_string(),
                root: PathBuf::from("/tmp/refact-wt"),
                source_workspace_root: PathBuf::from("/tmp/refact-src"),
                repo_root: PathBuf::from("/tmp/refact-src"),
                branch: Some("refact/task/card".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some("abc123".to_string()),
                task_id: Some("task-123".to_string()),
                card_id: Some("card-123".to_string()),
                agent_id: Some("agent-123".to_string()),
                enforce: true,
            }),
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
        }
    }

    #[test]
    fn trajectory_event_serialization() {
        let event = event_with_all_fields();
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
        assert_eq!(json["worktree"]["id"], "wt-1");
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
    fn trajectory_event_roundtrips() {
        let encoded = serde_json::to_string(&event_with_all_fields()).unwrap();
        let decoded: TrajectoryEvent = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.event_type, "updated");
        assert_eq!(decoded.id, "chat-123");
        assert_eq!(decoded.worktree.unwrap().id, "wt-1");
        assert_eq!(decoded.total_cost_usd, Some(0.042));
    }

    #[test]
    fn trajectory_event_serialization_skips_none_metric_fields() {
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
}
