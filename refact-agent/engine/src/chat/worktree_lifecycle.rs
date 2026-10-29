use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use super::types::{ChatSession, SessionState, ThreadParams};

pub(crate) const TASK_SCOPE_REFUSAL: &str = "task_worktree_scope_owned: Generic SetParams cannot change task-managed checkout scope or ownership. Retain WIP and ask the task planner to recover through the task lifecycle; do not reattach or switch workspaces.";

pub(crate) async fn transition_lock(session: &Arc<AsyncMutex<ChatSession>>) -> OwnedMutexGuard<()> {
    static LOCKS: OnceLock<Mutex<HashMap<usize, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let lock = {
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
    };
    lock.lock_owned().await
}

pub(crate) fn is_worktree_patch(patch: &serde_json::Value) -> bool {
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

pub(crate) fn changes_ownership(thread: &ThreadParams, patch: &serde_json::Value) -> bool {
    patch.get("task_meta").is_some_and(|value| {
        serde_json::from_value::<Option<super::types::TaskMeta>>(value.clone())
            .map_or(true, |meta| meta != thread.task_meta)
    }) || [
        ("root_chat_id", &thread.root_chat_id),
        ("parent_id", &thread.parent_id),
        ("link_type", &thread.link_type),
    ]
    .iter()
    .any(|(key, current)| {
        patch.get(key).is_some_and(|value| {
            serde_json::from_value::<Option<String>>(value.clone()).map_or(true, |value| {
                value.filter(|value| !value.is_empty()).as_ref() != current.as_ref()
            })
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
        || matches!(
            session.runtime.state,
            SessionState::Generating | SessionState::ExecutingTools
        )
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

    #[test]
    fn worktree_create_ui_attachment_refuses_executing_scope_but_accepts_idle() {
        let mut session = ChatSession::new("attachment-guard".into());
        session.set_runtime_state(SessionState::ExecutingTools, None);
        assert_eq!(
            check_idle_attachment(&session),
            Err("Worktree scope cannot change while this chat is running; retry when idle".into())
        );
        session.set_runtime_state(SessionState::Idle, None);
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
