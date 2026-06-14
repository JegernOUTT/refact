use serde::{Deserialize, Serialize};

use crate::types::{TaskBoard, TaskMeta};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskEvent {
    Snapshot {
        tasks: Vec<TaskMeta>,
    },
    TaskCreated {
        task_id: String,
        meta: TaskMeta,
    },
    TaskUpdated {
        task_id: String,
        meta: TaskMeta,
    },
    TaskDeleted {
        task_id: String,
    },
    BoardChanged {
        task_id: String,
        rev: u64,
        board: TaskBoard,
    },
    TaskCommentsChanged {
        task_id: String,
        card_id: String,
    },
    TaskDocumentChanged {
        task_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        slug: Option<String>,
    },
    TaskMemoriesChanged {
        task_id: String,
    },
    Heartbeat {
        ts: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskEventEnvelope {
    pub seq: u64,
    #[serde(flatten)]
    pub event: TaskEvent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TaskStatus;

    fn meta() -> TaskMeta {
        TaskMeta {
            schema_version: 1,
            id: "task-1".into(),
            name: "Task One".into(),
            status: TaskStatus::Active,
            created_at: "created".into(),
            updated_at: "updated".into(),
            cards_total: 2,
            cards_done: 1,
            cards_failed: 0,
            agents_active: 1,
            base_branch: Some("main".into()),
            base_commit: Some("abc123".into()),
            default_agent_model: Some("model".into()),
            is_name_generated: true,
            last_agents_summary_at: Some("summary".into()),
            planner_session_state: Some("idle".into()),
        }
    }

    #[test]
    fn task_event_envelope_roundtrips_with_flattened_event() {
        let envelope = TaskEventEnvelope {
            seq: 7,
            event: TaskEvent::TaskCreated {
                task_id: "task-1".into(),
                meta: meta(),
            },
        };

        let json = serde_json::to_string(&envelope).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let decoded: TaskEventEnvelope = serde_json::from_str(&json).unwrap();

        assert_eq!(value["seq"], 7);
        assert_eq!(value["type"], "task_created");
        assert_eq!(value["task_id"], "task-1");
        match decoded.event {
            TaskEvent::TaskCreated { task_id, meta } => {
                assert_eq!(decoded.seq, 7);
                assert_eq!(task_id, "task-1");
                assert_eq!(meta.name, "Task One");
                assert_eq!(meta.status, TaskStatus::Active);
                assert_eq!(meta.base_branch.as_deref(), Some("main"));
            }
            _ => panic!("unexpected event variant"),
        }
    }

    #[test]
    fn heartbeat_event_serializes_to_heartbeat_type() {
        let envelope = TaskEventEnvelope {
            seq: 5,
            event: TaskEvent::Heartbeat {
                ts: "2026-01-01T00:00:00Z".to_string(),
            },
        };

        let json = serde_json::to_string(&envelope).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["seq"], 5);
        assert_eq!(value["type"], "heartbeat");
        assert_eq!(value["ts"], "2026-01-01T00:00:00Z");
        assert!(value.get("tasks").is_none());
    }

    #[test]
    fn board_changed_event_roundtrips_with_board_defaults() {
        let envelope = TaskEventEnvelope {
            seq: 8,
            event: TaskEvent::BoardChanged {
                task_id: "task-1".into(),
                rev: 3,
                board: TaskBoard::default(),
            },
        };

        let json = serde_json::to_string(&envelope).unwrap();
        let decoded: TaskEventEnvelope = serde_json::from_str(&json).unwrap();

        match decoded.event {
            TaskEvent::BoardChanged {
                task_id,
                rev,
                board,
            } => {
                assert_eq!(decoded.seq, 8);
                assert_eq!(task_id, "task-1");
                assert_eq!(rev, 3);
                assert_eq!(board.schema_version, 1);
                assert_eq!(board.columns.len(), 5);
            }
            _ => panic!("unexpected event variant"),
        }
    }

    #[test]
    fn task_subresource_events_roundtrip_with_scoped_payloads() {
        let events = [
            (
                TaskEvent::TaskCommentsChanged {
                    task_id: "task-1".into(),
                    card_id: "T-7".into(),
                },
                "task_comments_changed",
                Some(("card_id", "T-7")),
            ),
            (
                TaskEvent::TaskDocumentChanged {
                    task_id: "task-1".into(),
                    slug: Some("main-plan".into()),
                },
                "task_document_changed",
                Some(("slug", "main-plan")),
            ),
            (
                TaskEvent::TaskMemoriesChanged {
                    task_id: "task-1".into(),
                },
                "task_memories_changed",
                None,
            ),
        ];

        for (event, expected_type, extra) in events {
            let json = serde_json::to_string(&TaskEventEnvelope { seq: 9, event }).unwrap();
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            let decoded: TaskEventEnvelope = serde_json::from_str(&json).unwrap();

            assert_eq!(value["seq"], 9);
            assert_eq!(value["type"], expected_type);
            assert_eq!(value["task_id"], "task-1");
            if let Some((key, expected)) = extra {
                assert_eq!(value[key], expected);
            }
            match decoded.event {
                TaskEvent::TaskCommentsChanged { task_id, card_id } => {
                    assert_eq!(task_id, "task-1");
                    assert_eq!(card_id, "T-7");
                }
                TaskEvent::TaskDocumentChanged { task_id, slug } => {
                    assert_eq!(task_id, "task-1");
                    assert_eq!(slug.as_deref(), Some("main-plan"));
                }
                TaskEvent::TaskMemoriesChanged { task_id } => {
                    assert_eq!(task_id, "task-1");
                }
                _ => panic!("unexpected event variant"),
            }
        }
    }
}
