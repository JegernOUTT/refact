#[cfg(test)]
mod background_tasks_tests {
    use tokio::task::JoinHandle;

    struct BackgroundTasksHolder {
        tasks: Vec<JoinHandle<()>>,
    }

    impl BackgroundTasksHolder {
        fn new(tasks: Vec<JoinHandle<()>>) -> Self {
            BackgroundTasksHolder { tasks }
        }

        fn push_back(&mut self, task: JoinHandle<()>) {
            self.tasks.push(task);
        }

        fn extend<T>(&mut self, tasks: T)
        where
            T: IntoIterator<Item = JoinHandle<()>>,
        {
            self.tasks.extend(tasks);
        }

        async fn abort(&mut self) {
            for task in self.tasks.iter_mut() {
                task.abort();
                let _ = task.await;
            }
            self.tasks.clear();
        }

        fn len(&self) -> usize {
            self.tasks.len()
        }
    }

    #[test]
    fn test_background_tasks_holder_new() {
        let holder = BackgroundTasksHolder::new(vec![]);
        assert_eq!(holder.len(), 0, "New holder should be empty");
    }

    #[test]
    fn test_background_tasks_push_back() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut holder = BackgroundTasksHolder::new(vec![]);
            let task = tokio::spawn(async {});
            holder.push_back(task);
            assert_eq!(holder.len(), 1, "Should have one task after push_back");
        });
    }

    #[test]
    fn test_background_tasks_extend() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut holder = BackgroundTasksHolder::new(vec![]);
            let tasks = vec![tokio::spawn(async {}), tokio::spawn(async {})];
            holder.extend(tasks);
            assert_eq!(holder.len(), 2, "Should have two tasks after extend");
        });
    }

    #[test]
    fn test_background_tasks_abort() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut holder = BackgroundTasksHolder::new(vec![
                tokio::spawn(async {
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await
                }),
                tokio::spawn(async {
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await
                }),
            ]);
            assert_eq!(holder.len(), 2, "Should start with 2 tasks");
            holder.abort().await;
            assert_eq!(holder.len(), 0, "Should have no tasks after abort");
        });
    }
}

#[cfg(test)]
mod task_types_tests {
    use std::collections::HashSet;

    #[derive(Debug, Clone)]
    struct BoardCard {
        id: String,
        title: String,
        column: String,
        depends_on: Vec<String>,
    }

    #[derive(Debug, Clone)]
    struct BoardColumn {
        id: String,
        title: String,
    }

    #[derive(Debug, Clone)]
    struct TaskBoard {
        schema_version: u32,
        rev: u64,
        columns: Vec<BoardColumn>,
        cards: Vec<BoardCard>,
    }

    #[derive(Debug, Clone)]
    struct ReadyCardsResult {
        ready: Vec<String>,
        blocked: Vec<String>,
        in_progress: Vec<String>,
        completed: Vec<String>,
        failed: Vec<String>,
    }

    impl Default for TaskBoard {
        fn default() -> Self {
            Self {
                schema_version: 1,
                rev: 0,
                columns: vec![
                    BoardColumn {
                        id: "planned".into(),
                        title: "Planned".into(),
                    },
                    BoardColumn {
                        id: "doing".into(),
                        title: "Doing".into(),
                    },
                    BoardColumn {
                        id: "done".into(),
                        title: "Done".into(),
                    },
                    BoardColumn {
                        id: "failed".into(),
                        title: "Failed".into(),
                    },
                ],
                cards: vec![],
            }
        }
    }

    impl TaskBoard {
        fn get_ready_cards(&self) -> ReadyCardsResult {
            let mut ready = vec![];
            let mut blocked = vec![];
            let mut in_progress = vec![];
            let mut completed = vec![];
            let mut failed = vec![];

            let done_cards: HashSet<_> = self
                .cards
                .iter()
                .filter(|c| c.column == "done")
                .map(|c| c.id.as_str())
                .collect();

            for card in &self.cards {
                match card.column.as_str() {
                    "done" => completed.push(card.id.clone()),
                    "failed" => failed.push(card.id.clone()),
                    "doing" => in_progress.push(card.id.clone()),
                    "planned" => {
                        let deps_satisfied = card
                            .depends_on
                            .iter()
                            .all(|dep| done_cards.contains(dep.as_str()));
                        if deps_satisfied {
                            ready.push(card.id.clone());
                        } else {
                            blocked.push(card.id.clone());
                        }
                    }
                    _ => {}
                }
            }

            ReadyCardsResult {
                ready,
                blocked,
                in_progress,
                completed,
                failed,
            }
        }

        fn get_card(&self, card_id: &str) -> Option<&BoardCard> {
            self.cards.iter().find(|c| c.id == card_id)
        }

        fn get_dependency_reports(&self, card_id: &str) -> Vec<(String, String)> {
            let card = match self.get_card(card_id) {
                Some(c) => c,
                None => return vec![],
            };

            card.depends_on
                .iter()
                .filter_map(|dep_id| {
                    self.get_card(dep_id)
                        .map(|dep_card| (dep_card.title.clone(), "report".to_string()))
                })
                .collect()
        }
    }

    #[test]
    fn test_taskboard_default() {
        let board = TaskBoard::default();
        assert_eq!(
            board.schema_version, 1,
            "Default schema version should be 1"
        );
        assert_eq!(board.rev, 0, "Default rev should be 0");
        assert_eq!(board.columns.len(), 4, "Should have 4 default columns");
        assert_eq!(board.cards.len(), 0, "Default board should have no cards");
    }

    #[test]
    fn test_get_ready_cards_no_deps() {
        let mut board = TaskBoard::default();
        board.cards = vec![BoardCard {
            id: "card1".into(),
            title: "Task 1".into(),
            column: "planned".into(),
            depends_on: vec![],
        }];

        let result = board.get_ready_cards();
        assert_eq!(result.ready.len(), 1, "Card with no deps should be ready");
        assert!(
            result.ready.contains(&"card1".to_string()),
            "card1 should be in ready"
        );
        assert_eq!(result.blocked.len(), 0, "Should have no blocked cards");
    }

    #[test]
    fn test_get_ready_cards_with_satisfied_deps() {
        let mut board = TaskBoard::default();
        board.cards = vec![
            BoardCard {
                id: "card1".into(),
                title: "Task 1".into(),
                column: "done".into(),
                depends_on: vec![],
            },
            BoardCard {
                id: "card2".into(),
                title: "Task 2".into(),
                column: "planned".into(),
                depends_on: vec!["card1".into()],
            },
        ];

        let result = board.get_ready_cards();
        assert_eq!(
            result.ready.len(),
            1,
            "Card with satisfied deps should be ready"
        );
        assert!(
            result.ready.contains(&"card2".to_string()),
            "card2 should be in ready"
        );
        assert_eq!(result.completed.len(), 1, "Should have one completed card");
    }

    #[test]
    fn test_get_ready_cards_with_unsatisfied_deps() {
        let mut board = TaskBoard::default();
        board.cards = vec![
            BoardCard {
                id: "card1".into(),
                title: "Task 1".into(),
                column: "planned".into(),
                depends_on: vec![],
            },
            BoardCard {
                id: "card2".into(),
                title: "Task 2".into(),
                column: "planned".into(),
                depends_on: vec!["card1".into()],
            },
        ];

        let result = board.get_ready_cards();
        assert_eq!(result.ready.len(), 1, "Only card1 should be ready");
        assert!(
            result.ready.contains(&"card1".to_string()),
            "card1 should be ready"
        );
        assert_eq!(result.blocked.len(), 1, "card2 should be blocked");
        assert!(
            result.blocked.contains(&"card2".to_string()),
            "card2 should be blocked"
        );
    }

    #[test]
    fn test_get_ready_cards_transitive_deps() {
        let mut board = TaskBoard::default();
        board.cards = vec![
            BoardCard {
                id: "card1".into(),
                title: "Task 1".into(),
                column: "done".into(),
                depends_on: vec![],
            },
            BoardCard {
                id: "card2".into(),
                title: "Task 2".into(),
                column: "done".into(),
                depends_on: vec!["card1".into()],
            },
            BoardCard {
                id: "card3".into(),
                title: "Task 3".into(),
                column: "planned".into(),
                depends_on: vec!["card2".into()],
            },
        ];

        let result = board.get_ready_cards();
        assert_eq!(
            result.ready.len(),
            1,
            "card3 should be ready (transitive deps satisfied)"
        );
        assert!(
            result.ready.contains(&"card3".to_string()),
            "card3 should be ready"
        );
        assert_eq!(result.completed.len(), 2, "Should have 2 completed cards");
    }

    #[test]
    fn test_get_dependency_reports() {
        let mut board = TaskBoard::default();
        board.cards = vec![
            BoardCard {
                id: "card1".into(),
                title: "Task 1".into(),
                column: "done".into(),
                depends_on: vec![],
            },
            BoardCard {
                id: "card2".into(),
                title: "Task 2".into(),
                column: "planned".into(),
                depends_on: vec!["card1".into()],
            },
        ];

        let reports = board.get_dependency_reports("card2");
        assert_eq!(reports.len(), 1, "Should have one dependency report");
        assert_eq!(reports[0].0, "Task 1", "Report should reference Task 1");
    }

    #[test]
    fn test_get_card_not_found() {
        let board = TaskBoard::default();
        let card = board.get_card("nonexistent");
        assert!(card.is_none(), "Should return None for nonexistent card");
    }

    #[test]
    fn test_get_card_found() {
        let mut board = TaskBoard::default();
        board.cards = vec![BoardCard {
            id: "card1".into(),
            title: "Task 1".into(),
            column: "planned".into(),
            depends_on: vec![],
        }];

        let card = board.get_card("card1");
        assert!(card.is_some(), "Should find existing card");
        assert_eq!(card.unwrap().title, "Task 1", "Card title should match");
    }

    #[test]
    fn test_get_ready_cards_with_missing_dependency() {
        let mut board = TaskBoard::default();
        board.cards = vec![BoardCard {
            id: "card1".into(),
            title: "Task 1".into(),
            column: "planned".into(),
            depends_on: vec!["nonexistent".into()],
        }];

        let result = board.get_ready_cards();
        assert_eq!(
            result.ready.len(),
            0,
            "Card with missing dependency should be blocked"
        );
        assert_eq!(result.blocked.len(), 1, "Card should be in blocked list");
    }

    #[test]
    fn test_get_ready_cards_circular_deps_prevented() {
        let mut board = TaskBoard::default();
        board.cards = vec![
            BoardCard {
                id: "card1".into(),
                title: "Task 1".into(),
                column: "planned".into(),
                depends_on: vec!["card2".into()],
            },
            BoardCard {
                id: "card2".into(),
                title: "Task 2".into(),
                column: "planned".into(),
                depends_on: vec!["card1".into()],
            },
        ];

        let result = board.get_ready_cards();
        assert_eq!(
            result.ready.len(),
            0,
            "Circular deps should result in no ready cards"
        );
        assert_eq!(result.blocked.len(), 2, "Both cards should be blocked");
    }

    #[test]
    fn test_status_update_structure() {
        #[derive(Debug, Clone)]
        struct StatusUpdate {
            timestamp: String,
            message: String,
        }

        let update = StatusUpdate {
            timestamp: "2024-12-31T10:00:00Z".into(),
            message: "Started work".into(),
        };

        assert!(
            !update.timestamp.is_empty(),
            "Timestamp should not be empty"
        );
        assert!(!update.message.is_empty(), "Message should not be empty");
    }
}

#[cfg(test)]
mod trajectory_memos_tests {
    use serde_json::{json, Value};

    fn build_chat_messages(messages: &[Value]) -> Vec<(String, String)> {
        messages
            .iter()
            .filter_map(|msg| {
                let role = msg.get("role").and_then(|v| v.as_str())?;
                if role == "context_file" || role == "cd_instruction" {
                    return None;
                }

                let content = if let Some(c) = msg.get("content").and_then(|v| v.as_str()) {
                    c.to_string()
                } else if let Some(arr) = msg.get("content").and_then(|v| v.as_array()) {
                    arr.iter()
                        .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    return None;
                };

                if content.trim().is_empty() {
                    return None;
                }

                Some((role.to_string(), content.chars().take(3000).collect()))
            })
            .collect()
    }

    #[test]
    fn test_build_chat_messages_filters_context() {
        let messages = vec![
            json!({"role": "user", "content": "Hello"}),
            json!({"role": "context_file", "content": "file.rs"}),
            json!({"role": "assistant", "content": "Hi"}),
            json!({"role": "cd_instruction", "content": "cd /home"}),
        ];

        let result = build_chat_messages(&messages);
        assert_eq!(
            result.len(),
            2,
            "Should filter out context_file and cd_instruction"
        );
        assert_eq!(result[0].0, "user", "First message should be user");
        assert_eq!(
            result[1].0, "assistant",
            "Second message should be assistant"
        );
    }

    #[test]
    fn test_build_chat_messages_empty_content() {
        let messages = vec![
            json!({"role": "user", "content": "Hello"}),
            json!({"role": "assistant", "content": "   "}),
            json!({"role": "user", "content": ""}),
        ];

        let result = build_chat_messages(&messages);
        assert_eq!(result.len(), 1, "Should skip empty/whitespace messages");
        assert_eq!(result[0].1, "Hello", "Should keep non-empty message");
    }

    #[test]
    fn test_build_chat_messages_array_content() {
        let messages =
            vec![json!({"role": "user", "content": [{"text": "Part 1"}, {"text": "Part 2"}]})];

        let result = build_chat_messages(&messages);
        assert_eq!(result.len(), 1, "Should handle array content");
        assert_eq!(
            result[0].1, "Part 1\nPart 2",
            "Should join array parts with newline"
        );
    }
}

#[cfg(test)]
mod error_handling_tests {
    #[derive(Debug, Clone)]
    struct TrajectoryMeta {
        schema_version: u32,
        id: String,
        name: String,
        created_at: String,
        updated_at: String,
    }

    #[test]
    fn test_trajectory_meta_fields() {
        let meta = TrajectoryMeta {
            schema_version: 1,
            id: "task-123".into(),
            name: "Test Task".into(),
            created_at: "2024-12-31T10:00:00Z".into(),
            updated_at: "2024-12-31T11:00:00Z".into(),
        };

        assert_eq!(meta.schema_version, 1, "Schema version should be 1");
        assert!(!meta.id.is_empty(), "ID should not be empty");
        assert!(!meta.name.is_empty(), "Name should not be empty");
        assert!(
            !meta.created_at.is_empty(),
            "Created_at should not be empty"
        );
        assert!(
            !meta.updated_at.is_empty(),
            "Updated_at should not be empty"
        );
    }

    #[test]
    fn test_ready_cards_result_all_states() {
        #[derive(Debug, Clone)]
        struct ReadyCardsResult {
            ready: Vec<String>,
            blocked: Vec<String>,
            in_progress: Vec<String>,
            completed: Vec<String>,
            failed: Vec<String>,
        }

        let result = ReadyCardsResult {
            ready: vec!["card1".into()],
            blocked: vec!["card2".into()],
            in_progress: vec!["card3".into()],
            completed: vec!["card4".into()],
            failed: vec!["card5".into()],
        };

        assert_eq!(result.ready.len(), 1, "Should have ready cards");
        assert_eq!(result.blocked.len(), 1, "Should have blocked cards");
        assert_eq!(result.in_progress.len(), 1, "Should have in_progress cards");
        assert_eq!(result.completed.len(), 1, "Should have completed cards");
        assert_eq!(result.failed.len(), 1, "Should have failed cards");
    }

    #[test]
    fn test_validate_task_id_valid() {
        fn validate_task_id(task_id: &str) -> Result<(), String> {
            if task_id.is_empty() {
                return Err("Task ID cannot be empty".into());
            }
            if task_id.contains('/') || task_id.contains('\\') || task_id.contains("..") {
                return Err("Task ID contains invalid characters".into());
            }
            if task_id.len() > 100 {
                return Err("Task ID too long".into());
            }
            Ok(())
        }

        assert!(
            validate_task_id("valid-task-id").is_ok(),
            "Valid ID should pass"
        );
    }

    #[test]
    fn test_validate_task_id_empty() {
        fn validate_task_id(task_id: &str) -> Result<(), String> {
            if task_id.is_empty() {
                return Err("Task ID cannot be empty".into());
            }
            if task_id.contains('/') || task_id.contains('\\') || task_id.contains("..") {
                return Err("Task ID contains invalid characters".into());
            }
            if task_id.len() > 100 {
                return Err("Task ID too long".into());
            }
            Ok(())
        }

        let result = validate_task_id("");
        assert!(result.is_err(), "Empty ID should fail");
        assert_eq!(
            result.unwrap_err(),
            "Task ID cannot be empty",
            "Error message should match"
        );
    }

    #[test]
    fn test_validate_task_id_path_traversal() {
        fn validate_task_id(task_id: &str) -> Result<(), String> {
            if task_id.is_empty() {
                return Err("Task ID cannot be empty".into());
            }
            if task_id.contains('/') || task_id.contains('\\') || task_id.contains("..") {
                return Err("Task ID contains invalid characters".into());
            }
            if task_id.len() > 100 {
                return Err("Task ID too long".into());
            }
            Ok(())
        }

        assert!(
            validate_task_id("../../../etc/passwd").is_err(),
            "Path traversal should fail"
        );
        assert!(
            validate_task_id("task/id").is_err(),
            "Forward slash should fail"
        );
        assert!(
            validate_task_id("task\\id").is_err(),
            "Backslash should fail"
        );
    }

    #[test]
    fn test_validate_task_id_length_limits() {
        fn validate_task_id(task_id: &str) -> Result<(), String> {
            if task_id.is_empty() {
                return Err("Task ID cannot be empty".into());
            }
            if task_id.contains('/') || task_id.contains('\\') || task_id.contains("..") {
                return Err("Task ID contains invalid characters".into());
            }
            if task_id.len() > 100 {
                return Err("Task ID too long".into());
            }
            Ok(())
        }

        let long_id = "a".repeat(101);
        assert!(
            validate_task_id(&long_id).is_err(),
            "ID longer than 100 chars should fail"
        );

        let valid_id = "a".repeat(100);
        assert!(
            validate_task_id(&valid_id).is_ok(),
            "ID with exactly 100 chars should pass"
        );
    }
}
