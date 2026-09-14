use std::sync::Arc;
use tokio::sync::Mutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::types::SessionState;
use refact_work_core::PendingApproval;

pub(crate) struct ApprovalTarget {
    pub work_id: String,
    pub chat_id: String,
    pub parent_id: String,
    pub generation: u64,
    pub title: String,
    board: bool,
}

impl ApprovalTarget {
    pub async fn from_context(
        app: &AppState,
        ccx: &Arc<Mutex<AtCommandsContext>>,
    ) -> Result<Self, String> {
        let (agent_id, chat_id) = {
            let context = ccx.lock().await;
            (context.background_agent_id.clone(), context.chat_id.clone())
        };
        let target = match agent_id {
            Some(id) => Self::resolve(app, &id).await?,
            None => Self::resolve_board(app, &chat_id).await?,
        };
        if target.chat_id != chat_id {
            return Err("approval child identity does not match its execution context".into());
        }
        Ok(target)
    }

    pub async fn resolve(app: &AppState, work_id: &str) -> Result<Self, String> {
        if let Some(chat_id) = work_id.strip_prefix("board:") {
            return Self::resolve_board(app, chat_id).await;
        }
        let agent = app.agents.get_any(work_id).await?;
        if agent.status.is_terminal() {
            return Err("approval child is no longer running".into());
        }
        Ok(Self {
            work_id: agent.agent_id,
            chat_id: agent
                .child_chat_id
                .ok_or("child execution scope unavailable")?,
            parent_id: agent.parent_chat_id,
            generation: agent.run_generation,
            title: agent.title,
            board: false,
        })
    }

    async fn resolve_board(app: &AppState, chat_id: &str) -> Result<Self, String> {
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .ok_or("board child session unavailable")?;
        let (meta, source, title) = {
            let session = session.lock().await;
            if session.closed
                || matches!(
                    session.runtime.state,
                    SessionState::Completed | SessionState::Error
                )
            {
                return Err("board child is no longer running".into());
            }
            (
                session
                    .thread
                    .task_meta
                    .clone()
                    .ok_or("approval requires a child run")?,
                crate::coordination::scope::session_source_root(&session)
                    .ok_or("board child project unavailable")?,
                session.thread.title.clone(),
            )
        };
        if !matches!(meta.role.as_str(), "agent" | "agents") {
            return Err("approval requires a child run".into());
        }
        let parent_id = meta
            .planner_chat_id
            .as_deref()
            .ok_or("board planner unavailable")?;
        let parent = app
            .chat
            .sessions
            .read()
            .await
            .get(parent_id)
            .cloned()
            .ok_or("board planner unavailable")?;
        let parent_source = {
            let parent = parent.lock().await;
            if parent.closed
                || !parent.thread.task_meta.as_ref().is_some_and(|parent| {
                    parent.role == "planner" && parent.task_id == meta.task_id
                })
            {
                return Err("board planner authority unavailable".into());
            }
            crate::coordination::scope::session_source_root(&parent)
                .ok_or("board planner project unavailable")?
        };
        let source = crate::coordination::roots::resolve_source_workspace_root(
            app.gcx.clone(),
            source.to_str(),
        )
        .await?;
        let parent_source = crate::coordination::roots::resolve_source_workspace_root(
            app.gcx.clone(),
            parent_source.to_str(),
        )
        .await?;
        if source != parent_source {
            return Err("board approval project mismatch".into());
        }
        let task_dir = crate::tasks::storage::find_task_dir(app.gcx.clone(), &meta.task_id).await?;
        let task_dir = tokio::fs::canonicalize(task_dir)
            .await
            .map_err(|_| "board task scope unavailable")?;
        if !task_dir.starts_with(source.join(".refact").join("tasks")) {
            return Err("board task is outside the authorized project".into());
        }
        let board = crate::tasks::storage::load_board(app.gcx.clone(), &meta.task_id).await?;
        let card = board
            .get_card(meta.card_id.as_deref().ok_or("board card unavailable")?)
            .ok_or("board card unavailable")?;
        let bound = if let Some(variants) = &card.ab_variants {
            [&variants.a, &variants.b]
                .iter()
                .any(|variant| variant.chat_id == chat_id && variant.finish.is_none())
        } else if !card.team_members.is_empty() {
            card.team_members
                .iter()
                .any(|member| member.agent_chat_id.as_deref() == Some(chat_id))
        } else {
            card.agent_chat_id.as_deref() == Some(chat_id)
        };
        if card.column != "doing" || !bound {
            return Err("board approval belongs to a stale card attempt".into());
        }
        let execution = app
            .agent_executions
            .get(chat_id)
            .ok_or("board execution ownership unavailable")?;
        if execution.is_cancelled() || execution.state() == super::execution::ExecutionState::Gone {
            return Err("board execution ownership expired".into());
        }
        Ok(Self {
            work_id: format!("board:{chat_id}"),
            chat_id: chat_id.into(),
            parent_id: parent_id.into(),
            generation: execution.admission_generation(),
            title,
            board: true,
        })
    }

    pub fn check(&self, pending: &PendingApproval) -> Result<(), String> {
        if self.work_id != pending.child_work_id
            || self.parent_id != pending.parent_chat_id
            || self.generation != pending.run_generation
        {
            return Err("approval belongs to a stale or unauthorized child run".into());
        }
        Ok(())
    }

    pub async fn set_waiting(&self, app: &AppState, waiting: bool) -> Result<(), String> {
        if self.board {
            let session = app
                .chat
                .sessions
                .read()
                .await
                .get(&self.chat_id)
                .cloned()
                .ok_or("board child session unavailable")?;
            session.lock().await.set_runtime_state(
                if waiting {
                    SessionState::WaitingUserInput
                } else {
                    SessionState::ExecutingTools
                },
                None,
            );
        } else {
            let updated = if waiting {
                app.agents.mark_waiting_for_approval(&self.work_id).await?
            } else {
                app.agents
                    .mark_running(&self.work_id, self.chat_id.clone())
                    .await?
            };
            super::spawn::emit_background_agent_update(app.clone(), &updated).await;
        }
        Ok(())
    }
}
