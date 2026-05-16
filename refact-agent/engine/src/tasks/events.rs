use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;

pub use refact_tasks::events::{TaskEvent, TaskEventEnvelope};

pub async fn emit_task_event(gcx: Arc<ARwLock<GlobalContext>>, event: TaskEvent) {
    let gcx_locked = gcx.read().await;
    if let (Some(tx), Some(seq_counter)) = (&gcx_locked.task_events_tx, &gcx_locked.task_events_seq)
    {
        let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
        let envelope = TaskEventEnvelope { seq, event };
        let _ = tx.send(envelope);
    }
}
