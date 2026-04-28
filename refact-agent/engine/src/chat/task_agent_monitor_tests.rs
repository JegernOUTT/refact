#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::{BoardCard, TaskBoard, TaskMeta, TaskStatus, StatusUpdate};
    use std::sync::Arc;
    use tokio::sync::RwLock as ARwLock;
    use crate::global_context::GlobalContext;

    fn create_test_card(id: &str, column: &str, assignee: Option<String>) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: format!("Card {}", id),
            column: column.to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: "Test instructions".to_string(),
            assignee,
            agent_chat_id: assignee.as_ref().map(|a| format!("agent-{}", a)),
            status_updates: vec![],
            final_report: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: Some(chrono::Utc::now().to_rfc3339()),
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            target_files: vec![],
        }
    }

    #[test]
    fn test_agent_stuck_timeout_constant() {
        assert_eq!(AGENT_STUCK_TIMEOUT.as_secs(), 20 * 60);
    }

    #[test]
    fn test_monitor_interval_constant() {
        assert_eq!(MONITOR_INTERVAL.as_secs(), 5 * 60);
    }

    #[test]
    fn test_create_test_card_structure() {
        let card = create_test_card("T-1", "doing", Some("agent-123".to_string()));
        
        assert_eq!(card.id, "T-1");
        assert_eq!(card.column, "doing");
        assert_eq!(card.assignee, Some("agent-123".to_string()));
        assert_eq!(card.agent_chat_id, Some("agent-agent-123".to_string()));
        assert!(card.started_at.is_some());
    }

    #[test]
    fn test_card_in_doing_with_assignee() {
        let card = create_test_card("T-1", "doing", Some("agent-123".to_string()));
        assert!(card.column == "doing" && card.assignee.is_some());
    }

    #[test]
    fn test_card_in_done_should_skip() {
        let card = create_test_card("T-1", "done", Some("agent-123".to_string()));
        assert!(card.column == "done");
    }

    #[test]
    fn test_card_in_failed_should_skip() {
        let card = create_test_card("T-1", "failed", Some("agent-123".to_string()));
        assert!(card.column == "failed");
    }

    #[test]
    fn test_card_without_assignee_should_skip() {
        let card = create_test_card("T-1", "doing", None);
        assert!(card.assignee.is_none());
    }

    #[test]
    fn test_agents_active_count() {
        let cards = vec![
            create_test_card("T-1", "doing", Some("agent-1".to_string())),
            create_test_card("T-2", "doing", Some("agent-2".to_string())),
            create_test_card("T-3", "done", Some("agent-3".to_string())),
            create_test_card("T-4", "planned", None),
        ];

        let active_count = cards
            .iter()
            .filter(|c| c.column == "doing" && c.assignee.is_some())
            .count();

        assert_eq!(active_count, 2);
    }

    #[test]
    fn test_timestamp_fallback_logic() {
        let mut card = create_test_card("T-1", "doing", Some("agent-1".to_string()));
        
        card.status_updates.push(StatusUpdate {
            timestamp: "2024-01-01T10:00:00Z".to_string(),
            message: "Test update".to_string(),
        });
        
        let last_activity = card.status_updates.last()
            .map(|u| u.timestamp.as_str())
            .or(card.started_at.as_deref())
            .unwrap_or(&card.created_at);
        
        assert_eq!(last_activity, "2024-01-01T10:00:00Z");
    }

    #[test]
    fn test_timestamp_fallback_to_started_at() {
        let card = create_test_card("T-1", "doing", Some("agent-1".to_string()));
        
        let last_activity = card.status_updates.last()
            .map(|u| u.timestamp.as_str())
            .or(card.started_at.as_deref())
            .unwrap_or(&card.created_at);
        
        assert_eq!(last_activity, card.started_at.as_ref().unwrap());
    }

    #[test]
    fn test_timestamp_fallback_to_created_at() {
        let mut card = create_test_card("T-1", "doing", Some("agent-1".to_string()));
        card.started_at = None;
        
        let last_activity = card.status_updates.last()
            .map(|u| u.timestamp.as_str())
            .or(card.started_at.as_deref())
            .unwrap_or(&card.created_at);
        
        assert_eq!(last_activity, &card.created_at);
    }

    #[test]
    fn test_all_finished_when_no_doing_cards() {
        let cards = vec![
            create_test_card("T-1", "done", Some("agent-1".to_string())),
            create_test_card("T-2", "failed", Some("agent-2".to_string())),
        ];

        let agents_active = cards
            .iter()
            .filter(|c| c.column == "doing" && c.assignee.is_some())
            .count();

        assert_eq!(agents_active, 0);
    }

    #[test]
    fn test_elapsed_time_calculation() {
        use std::time::Duration;
        let old_time = chrono::Utc::now() - chrono::Duration::seconds(25 * 60);
        let old_timestamp = old_time.to_rfc3339();
        
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&old_timestamp) {
            let elapsed = chrono::Utc::now().signed_duration_since(parsed.with_timezone(&chrono::Utc));
            let elapsed_secs = elapsed.num_seconds() as u64;
            
            assert!(elapsed_secs >= 25 * 60 - 5);
            assert!(elapsed_secs > AGENT_STUCK_TIMEOUT.as_secs());
        } else {
            panic!("Failed to parse timestamp");
        }
    }

    #[test]
    fn test_assignee_mismatch_detection() {
        let card = create_test_card("T-1", "doing", Some("agent-123".to_string()));
        let expected_agent = "agent-456";
        
        let mismatch = card.assignee.as_ref() != Some(&expected_agent.to_string());
        assert!(mismatch, "Should detect assignee mismatch");
    }

    #[test]
    fn test_assignee_match() {
        let card = create_test_card("T-1", "doing", Some("agent-123".to_string()));
        let expected_agent = "agent-123";
        
        let matches = card.assignee.as_ref() == Some(&expected_agent.to_string());
        assert!(matches, "Should detect assignee match");
    }
}
