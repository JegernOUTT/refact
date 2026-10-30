use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use super::types::{ChatSession, ThreadParams};

pub(crate) const TASK_SCOPE_REFUSAL: &str = "task_worktree_scope_owned: Generic SetParams cannot change task-managed checkout scope or ownership. Retain WIP and ask the task planner to recover through the task lifecycle; do not reattach or switch workspaces.";

pub(crate) struct AttachmentTransitionGuard {
    _guard: OwnedMutexGuard<()>,
    session: Arc<AsyncMutex<ChatSession>>,
}

impl AttachmentTransitionGuard {
    pub(crate) fn is_for(&self, session: &Arc<AsyncMutex<ChatSession>>) -> bool {
        Arc::ptr_eq(&self.session, session)
    }
}

fn transition_mutex(session: &Arc<AsyncMutex<ChatSession>>) -> Arc<AsyncMutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<usize, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    locks
        .get(&(Arc::as_ptr(session) as usize))
        .and_then(Weak::upgrade)
        .unwrap_or_else(|| {
            let lock = Arc::new(AsyncMutex::new(()));
            locks.insert(Arc::as_ptr(session) as usize, Arc::downgrade(&lock));
            lock
        })
}

pub(crate) async fn transition_lock(
    session: &Arc<AsyncMutex<ChatSession>>,
) -> AttachmentTransitionGuard {
    AttachmentTransitionGuard {
        _guard: transition_mutex(session).lock_owned().await,
        session: session.clone(),
    }
}

pub(crate) fn try_transition_lock(
    session: &Arc<AsyncMutex<ChatSession>>,
) -> Result<AttachmentTransitionGuard, String> {
    transition_mutex(session)
        .try_lock_owned()
        .map(|_guard| AttachmentTransitionGuard {
            _guard,
            session: session.clone(),
        })
        .map_err(|_| "Parent checkout transition in progress; retain source and retry".into())
}

pub(crate) fn is_scope_patch(patch: &serde_json::Value) -> bool {
    [
        "worktree_id",
        "worktree",
        "task_meta",
        "root_chat_id",
        "parent_id",
        "link_type",
    ]
    .iter()
    .any(|key| patch.get(key).is_some())
}

pub(crate) fn has_managed_task_binding(thread: &ThreadParams) -> bool {
    thread
        .task_meta
        .as_ref()
        .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents"))
        || thread
            .worktree
            .as_ref()
            .is_some_and(|worktree| worktree.kind == "task_agent")
}

pub(crate) async fn check_managed_attachment(
    app: &crate::app_state::AppState,
    chat_id: &str,
    thread: &ThreadParams,
) -> Result<(), String> {
    if !has_managed_task_binding(thread) && !crate::tasks::admission::is_tracked(chat_id).await {
        return Ok(());
    }
    if app
        .agents
        .find_agent_id_by_child_chat_id(chat_id)
        .await
        .is_some()
    {
        return Ok(());
    }
    let validate = async {
        let worktree = thread
            .worktree
            .as_ref()
            .ok_or("accepted checkout is missing")?;
        let meta = thread
            .task_meta
            .as_ref()
            .ok_or("task ownership is missing")?;
        if !matches!(meta.role.as_str(), "agent" | "agents")
            || worktree.kind != "task_agent"
            || worktree.task_id.as_deref() != Some(meta.task_id.as_str())
            || worktree.card_id.is_none()
            || worktree.card_id != meta.card_id
        {
            return Err("checkout does not match accepted task ownership".to_string());
        }
        let service = crate::worktrees::service::WorktreeService::new_async(
            app.paths.cache_dir.clone(),
            worktree.source_workspace_root.clone(),
        )
        .await?;
        if service.validate_worktree_meta_strict(worktree).await? != *worktree {
            return Err("registered checkout identity changed".into());
        }
        Ok(())
    };
    validate.await.map_err(|error| {
        format!("Managed task scope is unavailable: {error}. Stop and ask the task planner to inspect the current card/agent attempt and use authorized lifecycle recovery. Retain the checkout and WIP; do not continue in the main workspace.")
    })
}

pub(crate) fn changes_scope_identity(thread: &ThreadParams, patch: &serde_json::Value) -> bool {
    patch
        .get("task_meta")
        .is_some_and(|meta| Some(meta) != serde_json::to_value(&thread.task_meta).ok().as_ref())
        || [
            ("root_chat_id", thread.root_chat_id.as_deref()),
            ("parent_id", thread.parent_id.as_deref()),
            ("link_type", thread.link_type.as_deref()),
        ]
        .iter()
        .any(|(key, before)| {
            patch.get(key).is_some_and(|value| {
                (!value.is_null() && !value.is_string()) || value.as_str() != *before
            })
        })
}

pub(crate) fn changes_attachment(thread: &ThreadParams, patch: &serde_json::Value) -> bool {
    patch.get("worktree_id").is_some_and(|id| {
        id.as_str()
            != thread
                .worktree
                .as_ref()
                .map(|worktree| worktree.id.as_str())
    }) || patch.get("worktree").is_some_and(|worktree| {
        worktree.is_null() || Some(worktree) != serde_json::to_value(&thread.worktree).ok().as_ref()
    })
}

pub(crate) fn check_idle_attachment(session: &ChatSession) -> Result<(), String> {
    if session.closed
        || !session.turn_finished()
        || session.interruptible_waits > 0
        || session.draft_message.is_some()
        || session.draft_usage.is_some()
        || session.stream_started_at.is_some()
        || session.runtime.paused
        || !session.runtime.pause_reasons.is_empty()
        || session.confirmation_paused_at.is_some()
        || session.planner_pause_pending_tools
        || session.has_pending_tool_result_window()
        || session.pending_context_rebuild.is_some()
        || session.pending_mode_handoff.is_some()
        || session.pending_browser_message.is_some()
        || super::context_rebuild::compression_attempt_active(session)
    {
        return Err(
            "Worktree scope cannot change while this chat is running; retry when idle".into(),
        );
    }
    Ok(())
}

pub(crate) fn check_unchanged_attachment(
    session: &ChatSession,
    before: &ThreadParams,
) -> Result<(), String> {
    if session.closed
        || session.chat_id != before.id
        || session.thread.id != before.id
        || session.thread.worktree != before.worktree
        || session.thread.task_meta != before.task_meta
        || session.thread.root_chat_id != before.root_chat_id
        || session.thread.parent_id != before.parent_id
        || session.thread.link_type != before.link_type
    {
        return Err("Worktree attachment canceled: chat scope or ownership changed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::SessionState;

    #[test]
    fn worktree_create_ui_attachment_refuses_executing_scope_but_accepts_idle() {
        let mut session = ChatSession::new("attachment-guard".into());
        session.set_runtime_state(SessionState::ExecutingTools, None);
        assert_eq!(
            check_idle_attachment(&session),
            Err("Worktree scope cannot change while this chat is running; retry when idle".into())
        );
        session.set_runtime_state(SessionState::Idle, None);
        session.turn_depth = 1;
        assert!(check_idle_attachment(&session).is_err());
        session.turn_depth = 0;
        session.begin_interruptible_wait();
        assert!(check_idle_attachment(&session).is_err());
        session.end_interruptible_wait();
        session.add_message(crate::call_validation::ChatMessage {
            role: "assistant".into(),
            tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                id: "pending-scope".into(),
                index: None,
                function: crate::call_validation::ChatToolFunction {
                    name: "cat".into(),
                    arguments: "{}".into(),
                },
                tool_type: "function".into(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            }]),
            ..Default::default()
        });
        assert!(check_idle_attachment(&session).is_err());
        session.add_message(crate::call_validation::ChatMessage {
            role: "tool".into(),
            tool_call_id: "pending-scope".into(),
            ..Default::default()
        });
        session.pending_context_rebuild =
            Some(super::super::context_rebuild::PendingContextRebuild {
                model: None,
                trigger: "attachment".into(),
            });
        assert!(check_idle_attachment(&session).is_err());
        session.pending_context_rebuild = None;
        assert_eq!(check_idle_attachment(&session), Ok(()));
        session.thread.task_meta = Some(super::super::types::TaskMeta {
            task_id: "task-1".into(),
            role: "agents".into(),
            agent_id: Some("agent-1".into()),
            card_id: Some("T-1".into()),
            planner_chat_id: Some("planner-1".into()),
        });
        session.thread.root_chat_id = Some("planner-1".into());
        let before = session.thread.clone();
        assert_eq!(check_unchanged_attachment(&session, &before), Ok(()));
        session.thread.task_meta.as_mut().unwrap().card_id = Some("T-2".into());
        assert_eq!(
            check_unchanged_attachment(&session, &before),
            Err("Worktree attachment canceled: chat scope or ownership changed".into())
        );
        session.thread = before.clone();
        session.thread.parent_id = Some("other-planner".into());
        assert_eq!(
            check_unchanged_attachment(&session, &before),
            Err("Worktree attachment canceled: chat scope or ownership changed".into())
        );
    }
}
