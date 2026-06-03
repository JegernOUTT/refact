use std::sync::Arc;
use std::sync::atomic::Ordering;

use refact_buddy_core::conductor::ConductorWakeReason;

use crate::buddy::conductor::wake::{enqueue_task_wake, refresh_conductor_wake_targets};
use crate::global_context::GlobalContext;

pub use refact_tasks::events::{TaskEvent, TaskEventEnvelope};

pub async fn emit_task_event(gcx: Arc<GlobalContext>, event: TaskEvent) {
    emit_conductor_wake_for_task_event(gcx.clone(), &event).await;
    if let (Some(tx), Some(seq_counter)) = (&gcx.task_events_tx, &gcx.task_events_seq) {
        let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
        let envelope = TaskEventEnvelope { seq, event };
        let _ = tx.send(envelope);
    }
}

async fn emit_conductor_wake_for_task_event(gcx: Arc<GlobalContext>, event: &TaskEvent) {
    match event {
        TaskEvent::TaskCreated { .. } | TaskEvent::TaskUpdated { .. } => {
            refresh_conductor_wake_targets(gcx).await;
        }
        TaskEvent::BoardChanged { task_id, .. } => {
            enqueue_task_wake(gcx, task_id, ConductorWakeReason::TaskBoard).await;
        }
        TaskEvent::TaskDeleted { .. }
        | TaskEvent::Snapshot { .. }
        | TaskEvent::Heartbeat { .. } => {}
    }
}
