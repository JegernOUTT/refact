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
