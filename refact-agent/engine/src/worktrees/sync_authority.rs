use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app_state::AppState;
use crate::chat::types::ChatSession;
use crate::worktrees::types::{WorktreeMeta, WorktreeReference};

async fn session(app: &AppState, chat_id: &str) -> Option<Arc<Mutex<ChatSession>>> {
    app.chat.sessions.read().await.get(chat_id).cloned()
}

fn execution_live(app: &AppState, chat_id: &str) -> bool {
    app.agent_executions
        .get(chat_id)
        .is_some_and(|run| run.state() != crate::agents::execution::ExecutionState::Gone)
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WriterBlockerKind {
    AgentPending,
    ExecutionActive,
    BoardAdmitted,
    ChatCommandsQueued,
    ChatActive,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WriterRecoveryAction {
    Wait,
    InspectAgent,
    StopChat,
    UserRequired,
    RetryStatus,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct WriterBlocker {
    pub kind: WriterBlockerKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_generation: Option<u64>,
    pub recovery_actions: Vec<WriterRecoveryAction>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct WriterReadiness {
    pub ready: bool,
    pub blockers: Vec<WriterBlocker>,
    pub truncated: bool,
}

pub(crate) async fn ensure_source_quiescent(
    app: &AppState,
    meta: &WorktreeMeta,
    references: &[WorktreeReference],
    actor: Option<&str>,
) -> Result<(), String> {
    let readiness = source_writer_readiness(app, meta, references, actor).await?;
    if readiness.ready {
        Ok(())
    } else {
        Err("Source synchronization refused: source writer is active or queued; refresh synchronization status in the worktree panel for writer readiness and recovery actions".into())
    }
}

pub(crate) async fn source_writer_readiness(
    app: &AppState,
    meta: &WorktreeMeta,
    references: &[WorktreeReference],
    actor: Option<&str>,
) -> Result<WriterReadiness, String> {
    let agents = app.agents.list_all().await;
    let mut chats: std::collections::BTreeSet<String> = references
        .iter()
        .filter_map(|r| r.chat_id.clone())
        .collect();
    let mut pending = Vec::new();
    for agent in &agents {
        if agent.worktree_id.as_deref() == Some(&meta.id) {
            if let Some(chat) = &agent.child_chat_id {
                chats.insert(chat.clone());
            }
            if !agent.status.is_terminal()
                && !agent
                    .child_chat_id
                    .as_ref()
                    .is_some_and(|chat| actor == Some(format!("chat:{chat}").as_str()))
            {
                pending.push((
                    WriterBlockerKind::AgentPending,
                    agent.child_chat_id.clone(),
                    Some(agent),
                ));
            }
        }
    }
    let sessions: Vec<_> = app.chat.sessions.read().await.values().cloned().collect();
    for session in sessions {
        let s = session.lock().await;
        if s.thread.worktree.as_ref().is_some_and(|w| {
            w.id == meta.id && w.source_workspace_root == meta.source_workspace_root
        }) {
            chats.insert(s.chat_id.clone());
        }
    }
    for chat in chats {
        if actor == Some(format!("chat:{chat}").as_str()) {
            continue;
        }
        let agent = agents
            .iter()
            .find(|a| a.child_chat_id.as_deref() == Some(&chat));
        if execution_live(app, &chat) || app.agent_executions.task_active(&chat) {
            pending.push((
                WriterBlockerKind::ExecutionActive,
                Some(chat.clone()),
                agent,
            ));
        }
        if crate::tasks::admission::is_tracked(&chat).await {
            pending.push((WriterBlockerKind::BoardAdmitted, Some(chat.clone()), agent));
        }
        if let Some(agent) = agent {
            if !agent.status.is_terminal() && agent.worktree_id.as_deref() != Some(&meta.id) {
                pending.push((
                    WriterBlockerKind::AgentPending,
                    Some(chat.clone()),
                    Some(agent),
                ));
            }
        }
        if let Some(session) = session(app, &chat).await {
            let s = session.lock().await;
            if !s.command_queue.is_empty() {
                pending.push((
                    WriterBlockerKind::ChatCommandsQueued,
                    Some(chat.clone()),
                    agent,
                ));
            }
            if matches!(
                s.runtime.state,
                refact_runtime_api::SessionState::Generating
                    | refact_runtime_api::SessionState::ExecutingTools
                    | refact_runtime_api::SessionState::WaitingIde
                    | refact_runtime_api::SessionState::Paused
            ) {
                pending.push((WriterBlockerKind::ChatActive, Some(chat.clone()), agent));
            }
        }
    }
    let ready = pending.is_empty();
    let truncated = pending.len() > 32;
    let owner = actor.and_then(|a| {
        a.strip_prefix("chat:")
            .or_else(|| {
                a.strip_prefix("parent:")
                    .and_then(|rest| rest.split(':').next())
            })
            .or_else(|| {
                a.strip_prefix("card:")
                    .and_then(|rest| rest.rsplit(':').next())
            })
    });
    let blockers = pending
        .into_iter()
        .take(32)
        .map(|(kind, chat, agent)| {
            let authorized = actor.is_none()
                || agent.is_some_and(|a| Some(a.parent_chat_id.as_str()) == owner)
                || chat.as_deref() == owner;
            let recovery_actions = if authorized && agent.is_some() {
                vec![
                    WriterRecoveryAction::InspectAgent,
                    WriterRecoveryAction::Wait,
                    WriterRecoveryAction::RetryStatus,
                ]
            } else if authorized && actor.is_none() {
                vec![
                    WriterRecoveryAction::StopChat,
                    WriterRecoveryAction::RetryStatus,
                ]
            } else {
                vec![
                    WriterRecoveryAction::UserRequired,
                    WriterRecoveryAction::RetryStatus,
                ]
            };
            WriterBlocker {
                kind,
                chat_id: authorized.then_some(chat).flatten(),
                agent_id: agent.filter(|_| authorized).map(|a| a.agent_id.clone()),
                run_generation: agent.filter(|_| authorized).map(|a| a.run_generation),
                recovery_actions,
            }
        })
        .collect();
    Ok(WriterReadiness {
        ready,
        blockers,
        truncated,
    })
}
