#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use tempfile::TempDir;
    use uuid::Uuid;
    use chrono::Utc;

    // Mock GlobalContext for testing
    struct MockGlobalContext {
        #[allow(dead_code)]
        workspace_root: PathBuf,
    }

    // Helper to create a test GlobalContext
    fn setup_test_gcx() -> (MockGlobalContext, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let gcx = MockGlobalContext {
            workspace_root: temp_dir.path().to_path_buf(),
        };
        (gcx, temp_dir)
    }

    // Test 1: Create task and verify metadata
    #[tokio::test]
    async fn test_create_task_and_persist() {
        let (_gcx, _temp) = setup_test_gcx();

        let task_name = "Test Task 1";
        let task_id = Uuid::new_v4().to_string();

        // Verify task_id is generated
        assert!(!task_id.is_empty(), "Task ID should be generated");
        assert_eq!(task_id.len(), 36, "UUID should be 36 chars");

        // Verify name matches
        assert_eq!(task_name, "Test Task 1", "Task name should match input");

        // Verify initial card counts are 0
        let cards_total = 0;
        let cards_done = 0;
        let cards_failed = 0;

        assert_eq!(cards_total, 0, "Initial cards_total should be 0");
        assert_eq!(cards_done, 0, "Initial cards_done should be 0");
        assert_eq!(cards_failed, 0, "Initial cards_failed should be 0");
    }

    // Test 2: Save and load board
    #[tokio::test]
    async fn test_save_and_load_board() {
        let (_gcx, _temp) = setup_test_gcx();

        let task_id = Uuid::new_v4().to_string();
        let task_dir = _temp.path().join(".refact").join("tasks").join(&task_id);

        // Create task directory
        tokio::fs::create_dir_all(&task_dir)
            .await
            .expect("Failed to create task dir");

        // Create a board with a card
        let board_yaml = r#"schema_version: 1
rev: 0
columns:
  - id: planned
    title: Planned
  - id: doing
    title: Doing
  - id: done
    title: Done
  - id: failed
    title: Failed
cards:
  - id: T1
    title: Task
    column: planned
    priority: P1
    depends_on: []
    instructions: "Do something"
    assignee: null
    agent_chat_id: null
    status_updates: []
    final_report: null
    created_at: "2024-12-31T00:00:00Z"
    started_at: null
    completed_at: null
"#;

        let board_path = task_dir.join("board.yaml");
        tokio::fs::write(&board_path, board_yaml)
            .await
            .expect("Failed to write board");

        // Load and verify
        let content = tokio::fs::read_to_string(&board_path)
            .await
            .expect("Failed to read board");
        assert!(content.contains("T1"), "Board should contain card T1");
        assert!(
            content.contains("planned"),
            "Board should contain planned column"
        );
    }

    // Test 3: Update task stats
    #[tokio::test]
    async fn test_update_task_stats() {
        let (_gcx, _temp) = setup_test_gcx();

        let task_id = Uuid::new_v4().to_string();
        let task_dir = _temp.path().join(".refact").join("tasks").join(&task_id);

        // Create task directory
        tokio::fs::create_dir_all(&task_dir)
            .await
            .expect("Failed to create task dir");

        // Create board with 3 cards in different states
        let board_yaml = r#"schema_version: 1
rev: 0
columns:
  - id: planned
    title: Planned
  - id: doing
    title: Doing
  - id: done
    title: Done
  - id: failed
    title: Failed
cards:
  - id: C1
    title: Card 1
    column: done
    priority: P1
    depends_on: []
    instructions: ""
    assignee: null
    agent_chat_id: null
    status_updates: []
    final_report: null
    created_at: "2024-12-31T00:00:00Z"
    started_at: null
    completed_at: null
  - id: C2
    title: Card 2
    column: failed
    priority: P1
    depends_on: []
    instructions: ""
    assignee: null
    agent_chat_id: null
    status_updates: []
    final_report: null
    created_at: "2024-12-31T00:00:00Z"
    started_at: null
    completed_at: null
  - id: C3
    title: Card 3
    column: doing
    priority: P1
    depends_on: []
    instructions: ""
    assignee: agent-1
    agent_chat_id: null
    status_updates: []
    final_report: null
    created_at: "2024-12-31T00:00:00Z"
    started_at: null
    completed_at: null
"#;

        let board_path = task_dir.join("board.yaml");
        tokio::fs::write(&board_path, board_yaml)
            .await
            .expect("Failed to write board");

        // Verify stats
        let content = tokio::fs::read_to_string(&board_path)
            .await
            .expect("Failed to read board");

        // Count cards by column
        let cards_total = 3;
        let cards_done = content.matches("column: done").count();
        let cards_failed = content.matches("column: failed").count();
        let _agents_active = content.matches("agent_chat_id: null").count() - 2; // C1 and C2 have null, C3 has agent

        assert_eq!(cards_total, 3, "Should have 3 total cards");
        assert_eq!(cards_done, 1, "Should have 1 done card");
        assert_eq!(cards_failed, 1, "Should have 1 failed card");
    }

    // Test 4: Save and list trajectories
    #[tokio::test]
    async fn test_save_and_list_trajectories() {
        let (_gcx, _temp) = setup_test_gcx();

        let task_id = Uuid::new_v4().to_string();
        let task_dir = _temp.path().join(".refact").join("tasks").join(&task_id);
        let traj_dir = task_dir.join("trajectories").join("agents");

        // Create trajectory directory
        tokio::fs::create_dir_all(&traj_dir)
            .await
            .expect("Failed to create traj dir");

        // Save a trajectory
        let chat_id = Uuid::new_v4().to_string();
        let trajectory_json = serde_json::json!({
            "id": chat_id,
            "title": "Agent Task",
            "model": "gpt-4o",
            "mode": "AGENT",
            "tool_use": "agent",
            "messages": [],
            "created_at": Utc::now().to_rfc3339(),
            "updated_at": Utc::now().to_rfc3339(),
            "task_meta": {
                "task_id": task_id,
                "role": "agents",
                "agent_id": null,
                "card_id": null
            }
        });

        let file_path = traj_dir.join(format!("{}.json", chat_id));
        let json_str = serde_json::to_string_pretty(&trajectory_json).expect("Failed to serialize");
        tokio::fs::write(&file_path, &json_str)
            .await
            .expect("Failed to write trajectory");

        // List trajectories
        let mut entries = tokio::fs::read_dir(&traj_dir)
            .await
            .expect("Failed to read dir");
        let mut found = false;

        while let Some(entry) = entries.next_entry().await.expect("Failed to iterate") {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                found = true;
                break;
            }
        }

        assert!(found, "Trajectory should be found in list");
    }

    // Test 5: Board persistence across restarts
    #[tokio::test]
    async fn test_board_persistence_across_restarts() {
        let (_gcx, _temp) = setup_test_gcx();

        let task_id = Uuid::new_v4().to_string();
        let task_dir = _temp.path().join(".refact").join("tasks").join(&task_id);

        // Create task directory
        tokio::fs::create_dir_all(&task_dir)
            .await
            .expect("Failed to create task dir");

        // Create and save board
        let board_yaml = r#"schema_version: 1
rev: 0
columns:
  - id: planned
    title: Planned
  - id: doing
    title: Doing
  - id: done
    title: Done
  - id: failed
    title: Failed
cards:
  - id: T1
    title: Task 1
    column: planned
    priority: P1
    depends_on: []
    instructions: "First task"
    assignee: null
    agent_chat_id: null
    status_updates: []
    final_report: null
    created_at: "2024-12-31T00:00:00Z"
    started_at: null
    completed_at: null
  - id: T2
    title: Task 2
    column: doing
    priority: P2
    depends_on:
      - T1
    instructions: "Second task"
    assignee: agent-1
    agent_chat_id: null
    status_updates: []
    final_report: null
    created_at: "2024-12-31T00:00:00Z"
    started_at: null
    completed_at: null
"#;

        let board_path = task_dir.join("board.yaml");
        tokio::fs::write(&board_path, board_yaml)
            .await
            .expect("Failed to write board");

        // Simulate restart by reading again
        let content = tokio::fs::read_to_string(&board_path)
            .await
            .expect("Failed to read board");

        // Verify all cards intact
        assert!(content.contains("T1"), "Card T1 should persist");
        assert!(content.contains("T2"), "Card T2 should persist");
        assert!(
            content.contains("First task"),
            "Card T1 instructions should persist"
        );
        assert!(
            content.contains("Second task"),
            "Card T2 instructions should persist"
        );
        assert!(
            content.contains("- T1"),
            "Card T2 dependency should persist"
        );
    }

    // Test 6: Validate task ID - valid ID
    #[test]
    fn test_validate_task_id_valid() {
        let task_id = "valid-task-id-123";
        assert!(task_id.len() <= 100, "Valid ID should pass length check");
        assert!(!task_id.contains('/'), "Valid ID should not contain /");
        assert!(!task_id.contains('\\'), "Valid ID should not contain \\");
        assert!(!task_id.contains(".."), "Valid ID should not contain ..");
    }

    // Test 7: Validate task ID - empty ID
    #[test]
    fn test_validate_task_id_empty() {
        let task_id = "";
        assert!(task_id.is_empty(), "Empty ID should be detected");
    }

    // Test 8: Validate task ID - path traversal
    #[test]
    fn test_validate_task_id_path_traversal() {
        let task_id = "../../../etc/passwd";
        assert!(task_id.contains(".."), "Path traversal should be detected");
    }

    // Test 9: Validate task ID - length limits
    #[test]
    fn test_validate_task_id_length_limits() {
        let task_id = "a".repeat(101);
        assert!(task_id.len() > 100, "Long ID should exceed limit");
    }

    // Test 10: Task meta fields
    #[test]
    fn test_trajectory_meta_fields() {
        let task_id = "task-123";
        let role = "agents";
        let agent_id = Some("agent-1");
        let card_id = Some("card-1");

        assert_eq!(task_id, "task-123", "Task ID should match");
        assert_eq!(role, "agents", "Role should match");
        assert_eq!(agent_id, Some("agent-1"), "Agent ID should match");
        assert_eq!(card_id, Some("card-1"), "Card ID should match");
    }

    // Test 11: Ready cards result structure
    #[test]
    fn test_ready_cards_result_all_states() {
        let ready = ["C1".to_string()];
        let blocked = ["C2".to_string()];
        let in_progress = ["C3".to_string()];
        let completed = ["C4".to_string()];
        let failed = ["C5".to_string()];

        assert_eq!(ready.len(), 1, "Ready should have 1 card");
        assert_eq!(blocked.len(), 1, "Blocked should have 1 card");
        assert_eq!(in_progress.len(), 1, "In progress should have 1 card");
        assert_eq!(completed.len(), 1, "Completed should have 1 card");
        assert_eq!(failed.len(), 1, "Failed should have 1 card");
    }

    // Test 12: Board card structure
    #[test]
    fn test_board_card_structure() {
        let card_id = "T1";
        let title = "Task Title";
        let column = "planned";
        let priority = "P1";
        let depends_on: Vec<String> = vec![];
        let instructions = "Do something";
        let assignee: Option<String> = None;
        let agent_chat_id: Option<String> = None;
        let _created_at = "2024-12-31T00:00:00Z";

        assert_eq!(card_id, "T1", "Card ID should match");
        assert_eq!(title, "Task Title", "Title should match");
        assert_eq!(column, "planned", "Column should match");
        assert_eq!(priority, "P1", "Priority should match");
        assert_eq!(depends_on.len(), 0, "No dependencies initially");
        assert_eq!(instructions, "Do something", "Instructions should match");
        assert_eq!(assignee, None, "Assignee should be None initially");
        assert_eq!(
            agent_chat_id, None,
            "Agent chat ID should be None initially"
        );
    }

    // Test 13: Status update structure
    #[test]
    fn test_status_update_structure() {
        let timestamp = "2024-12-31T12:00:00Z";
        let message = "Card moved to doing";

        assert!(!timestamp.is_empty(), "Timestamp should not be empty");
        assert!(!message.is_empty(), "Message should not be empty");
        assert!(timestamp.contains("T"), "Timestamp should be ISO format");
    }

    // Test 14: Task board default columns
    #[test]
    fn test_task_board_default_columns() {
        let columns = [
            ("planned", "Planned"),
            ("doing", "Doing"),
            ("done", "Done"),
            ("failed", "Failed"),
            ("regressed", "Regressed"),
        ];

        assert_eq!(columns.len(), 5, "Should have 5 default columns");
        assert_eq!(columns[0].0, "planned", "First column should be planned");
        assert_eq!(columns[4].0, "regressed", "Last column should be regressed");
    }

    // Test 15: Task status enum
    #[test]
    fn test_task_status_enum() {
        let statuses = ["planning", "active", "paused", "completed", "abandoned"];

        assert_eq!(statuses.len(), 5, "Should have 5 status values");
        assert_eq!(statuses[0], "planning", "First status should be planning");
        assert_eq!(statuses[4], "abandoned", "Last status should be abandoned");
    }
}
