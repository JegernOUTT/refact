use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::global_context::GlobalContext;

pub use refact_tasks::events::{TaskEvent, TaskEventEnvelope};
use refact_tasks::types::TaskMeta;

pub async fn emit_task_event(gcx: Arc<GlobalContext>, event: TaskEvent) {
    if let (Some(tx), Some(seq_counter)) = (&gcx.task_events_tx, &gcx.task_events_seq) {
        let seq = seq_counter.fetch_add(1, Ordering::SeqCst);
        let envelope = TaskEventEnvelope { seq, event };
        let _ = tx.send(envelope);
    }
}

async fn enrich_task_meta_with_session_state(gcx: Arc<GlobalContext>, meta: &mut TaskMeta) {
    let Ok(planner_trajectories) =
        crate::tasks::storage::list_task_trajectories(gcx.clone(), &meta.id, "planner", None).await
    else {
        meta.planner_session_state = None;
        return;
    };

    let planner_chat_ids = planner_trajectories
        .into_iter()
        .map(|trajectory| trajectory.id)
        .collect::<Vec<_>>();

    if planner_chat_ids.is_empty() {
        meta.planner_session_state = None;
        return;
    }

    let session_arcs = {
        let sessions = gcx.chat_sessions.read().await;
        planner_chat_ids
            .iter()
            .filter_map(|planner_chat_id| sessions.get(planner_chat_id).cloned())
            .collect::<Vec<_>>()
    };

    let mut has_paused = false;
    let mut has_waiting_ide = false;
    let mut has_waiting_user_input = false;
    let mut has_generating = false;
    let mut has_executing_tools = false;
    let mut has_error = false;
    for session_arc in session_arcs {
        let session = session_arc.lock().await;
        match session.runtime.state {
            crate::chat::types::SessionState::Paused => has_paused = true,
            crate::chat::types::SessionState::WaitingIde => has_waiting_ide = true,
            crate::chat::types::SessionState::WaitingUserInput => has_waiting_user_input = true,
            crate::chat::types::SessionState::Generating => has_generating = true,
            crate::chat::types::SessionState::ExecutingTools => has_executing_tools = true,
            crate::chat::types::SessionState::Error => has_error = true,
            crate::chat::types::SessionState::Idle
            | crate::chat::types::SessionState::Completed => {}
        }
    }

    meta.planner_session_state = if has_paused {
        Some(crate::chat::types::SessionState::Paused.to_string())
    } else if has_waiting_ide {
        Some(crate::chat::types::SessionState::WaitingIde.to_string())
    } else if has_waiting_user_input {
        Some(crate::chat::types::SessionState::WaitingUserInput.to_string())
    } else if has_generating {
        Some(crate::chat::types::SessionState::Generating.to_string())
    } else if has_executing_tools {
        Some(crate::chat::types::SessionState::ExecutingTools.to_string())
    } else if has_error {
        Some(crate::chat::types::SessionState::Error.to_string())
    } else {
        None
    };
}

pub async fn enrich_task_with_session_state(gcx: Arc<GlobalContext>, task: &mut TaskMeta) {
    enrich_task_meta_with_session_state(gcx, task).await;
}

pub async fn emit_task_updated(gcx: Arc<GlobalContext>, task_id: String, mut meta: TaskMeta) {
    enrich_task_meta_with_session_state(gcx.clone(), &mut meta).await;
    emit_task_event(gcx, TaskEvent::TaskUpdated { task_id, meta }).await;
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
