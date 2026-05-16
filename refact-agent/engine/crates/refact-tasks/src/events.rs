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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskEventEnvelope {
    pub seq: u64,
    #[serde(flatten)]
    pub event: TaskEvent,
}
