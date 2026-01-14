use std::sync::Arc;
use std::sync::atomic::Ordering;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;
use super::types::{TaskMeta, TaskBoard};

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

pub async fn emit_task_event(gcx: Arc<ARwLock<GlobalContext>>, event: TaskEvent) {
    let gcx_locked = gcx.read().await;
    if let (Some(tx), Some(seq_counter)) = (&gcx_locked.task_events_tx, &gcx_locked.task_events_seq) {
        let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
        let envelope = TaskEventEnvelope { seq, event };
        let _ = tx.send(envelope);
    }
}
