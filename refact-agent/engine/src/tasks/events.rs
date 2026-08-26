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
    let all_session_arcs = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.values().cloned().collect::<Vec<_>>()
    };

    let mut has_paused = false;
    let mut has_waiting_ide = false;
    let mut has_waiting_user_input = false;
    let mut has_generating = false;
    let mut has_executing_tools = false;
    let mut has_error = false;
    for session_arc in all_session_arcs {
        let session = session_arc.lock().await;
        let is_planner =
            session.thread.task_meta.as_ref().is_some_and(|task_meta| {
                task_meta.role == "planner" && task_meta.task_id == meta.id
            });
        if !is_planner {
            continue;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_state_enrichment_does_not_list_task_trajectories() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut task = TaskMeta {
            schema_version: 1,
            id: "task-index-free".to_string(),
            name: "Task".to_string(),
            status: refact_tasks::types::TaskStatus::Active,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            cards_total: 0,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 0,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: Some("stale".to_string()),
        };

        crate::chat::trajectory_index::reset_trajectory_index_listing_counters();
        enrich_task_with_session_state(gcx, &mut task).await;
        let counters = crate::chat::trajectory_index::trajectory_index_listing_counters();

        assert!(task.planner_session_state.is_none());
        assert_eq!(
            counters
                .calls_by_caller
                .iter()
                .find(|(caller, _)| *caller == "task_trajectory_api")
                .unwrap()
                .1,
            0
        );
    }
}
