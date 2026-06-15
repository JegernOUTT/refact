use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::global_context::GlobalContext;

pub use refact_tasks::events::{TaskEvent, TaskEventEnvelope};

pub async fn emit_task_event(gcx: Arc<GlobalContext>, event: TaskEvent) {
    if let (Some(tx), Some(seq_counter)) = (&gcx.task_events_tx, &gcx.task_events_seq) {
        let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
        let envelope = TaskEventEnvelope { seq, event };
        let _ = tx.send(envelope);
    }
}

pub async fn emit_task_comments_changed(gcx: Arc<GlobalContext>, task_id: &str, card_id: &str) {
    emit_task_event(
        gcx,
        TaskEvent::TaskCommentsChanged {
            task_id: task_id.to_string(),
            card_id: card_id.to_string(),
        },
    )
    .await;
}

pub async fn emit_task_document_changed(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    slug: Option<&str>,
) {
    emit_task_event(
        gcx,
        TaskEvent::TaskDocumentChanged {
            task_id: task_id.to_string(),
            slug: slug.map(str::to_string),
        },
    )
    .await;
}

pub async fn emit_task_memories_changed(gcx: Arc<GlobalContext>, task_id: &str) {
    emit_task_event(
        gcx,
        TaskEvent::TaskMemoriesChanged {
            task_id: task_id.to_string(),
        },
    )
    .await;
}
