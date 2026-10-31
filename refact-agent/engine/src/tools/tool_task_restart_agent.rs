use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::types::GoalSnapshotBudgetExt;
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, StatusUpdate};
use crate::tools::task_tool_helpers::AGENT_ABORT_TIMEOUT;
use crate::tasks::assignment::{self, Assignment, AssignmentScope};
use crate::tools::tool_task_spawn_agent::{
    build_agent_thread_params, mark_card_agent_started, prepare_agent_worktree,
    resolve_agent_model, restore_original_card_if_current_agent, AcceptedAgentSource,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::WorktreeMeta;
use refact_chat_api::ChatCommand;
use refact_runtime_api::CreateSessionRequest;

struct RestartRecovery {
    old_chat: Option<String>,
    session: Option<Arc<AMutex<crate::chat::types::ChatSession>>>,
    attempt: Option<crate::tasks::admission::Attempt>,
    revision: Option<u64>,
    accepted_state: Option<Value>,
    goal: Option<refact_chat_api::GoalSnapshot>,
    controls: Vec<ChatMessage>,
    stopped_state: Option<Value>,
    persisted: Option<(PathBuf, String)>,
}

impl RestartRecovery {
    async fn capture(
        app: &crate::app_state::AppState,
        source: &AcceptedAgentSource,
        task_id: &str,
        planner: &str,
        card: &BoardCard,
        force: bool,
    ) -> Result<Self, String> {
        let old_chat = card.agent_chat_id.clone();
        let session = match old_chat.as_deref() {
            Some(chat) => app.chat.sessions.read().await.get(chat).cloned(),
            None => None,
        };
        let attempt = match old_chat.as_deref() {
            Some(chat) => crate::tasks::admission::Attempt::capture_for_app(app, chat)?,
            None => None,
        };
        if attempt
            .as_ref()
            .is_some_and(|attempt| !attempt.belongs_to(app))
        {
            return Err(
                "Previous admission belongs to another runtime; force cannot replace it".into(),
            );
        }
        if old_chat
            .as_deref()
            .is_some_and(|chat| app.agent_executions.get(chat).is_some())
            && attempt.is_none()
        {
            return Err(
                "Previous execution has no matching board admission; force cannot acquire it"
                    .into(),
            );
        }
        if let Some(chat) = old_chat.as_deref().filter(|_| attempt.is_some()) {
            let expected_scope = crate::tasks::admission::board_scope(source.project(), planner);
            if !app.work_scheduler.snapshot().tickets.iter().any(|ticket| {
                ticket.run.work == refact_work_core::WorkKey::trajectory(chat.to_string())
                    && ticket.scope == expected_scope
            }) {
                return Err("Previous admission has no matching planner/project scope; force cannot stop it".into());
            }
        }
        let mut persisted = None;
        let mut accepted_state = None;
        let (thread, messages, goal, revision, held, busy) = if let Some(session) = &session {
            let session = session.lock().await;
            accepted_state = Some(Self::session_state(&session)?);
            let projection = session.try_accepted_control_projection()?;
            let mut messages = refact_core::active_context::active_context(&session.messages)
                .map_err(|error| error.to_string())?
                .messages;
            messages.retain(|message| {
                !matches!(message.role.as_str(), "goal" | "plan")
                    && !message.extra.get("event").is_some_and(|event| {
                        matches!(event["subkind"].as_str(), Some("goal_delta" | "plan_delta"))
                    })
            });
            messages.extend(projection.messages);
            (
                Some(session.thread.clone()),
                messages,
                session.goal.clone(),
                Some(session.trajectory_version),
                session.planner_paused()
                    || session.planner_pause_pending_tools
                    || session.board_restored_pending_tools
                    || !session.runtime.pause_reasons.is_empty()
                    || session.confirmation_paused_at.is_some()
                    || !session.pending_questions.is_empty()
                    || !session.approval_wrapper_calls.is_empty()
                    || session.pending_browser_message.is_some()
                    || matches!(
                        session.runtime.state,
                        refact_runtime_api::SessionState::Paused
                            | refact_runtime_api::SessionState::WaitingIde
                    )
                    || (session.runtime.state
                        == refact_runtime_api::SessionState::WaitingUserInput
                        && !session.runtime.error.as_deref().is_some_and(|error| {
                            error.starts_with("Interrupted: needs_fresh_admission.")
                        })),
                session.turn_depth > 0
                    || session
                        .queue_processor_running
                        .load(std::sync::atomic::Ordering::SeqCst)
                    || matches!(
                        session.runtime.state,
                        refact_runtime_api::SessionState::Generating
                            | refact_runtime_api::SessionState::ExecutingTools
                    ),
            )
        } else if let Some(chat) = old_chat.as_deref() {
            crate::chat::trajectories::validate_trajectory_id(chat)
                .map_err(|error| error.message)?;
            let task_dir = storage::find_task_dir(app.gcx.clone(), task_id).await?;
            let agent = card
                .assignee
                .as_deref()
                .ok_or("Previous agent identity missing")?;
            storage::validate_task_id(agent)?;
            let path = task_dir
                .join("trajectories/agents")
                .join(agent)
                .join(format!("{chat}.json"));
            {
                for parent in path.ancestors().skip(1).take(3) {
                    let meta = tokio::fs::symlink_metadata(parent)
                        .await
                        .map_err(|error| error.to_string())?;
                    if !meta.is_dir() || meta.file_type().is_symlink() {
                        return Err("Previous trajectory directory is not canonical".into());
                    }
                }
                storage::require_regular_task_file(&path).await?;
                let body = crate::coordination::board_bootstrap::read_bounded(
                    &path,
                    source.root(),
                    crate::file_activity::settings().max_work_history_bytes,
                )
                .await
                .map_err(|_| "Previous trajectory is unavailable or outside the accepted source")?;
                persisted = Some((path, crate::file_activity::digest_of(&body)));
                let value: Value =
                    serde_json::from_str(&body).map_err(|error| error.to_string())?;
                let thread: crate::chat::types::ThreadParams =
                    serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
                let messages: Vec<ChatMessage> = serde_json::from_value(value["messages"].clone())
                    .map_err(|error| error.to_string())?;
                let mut goal: Option<refact_chat_api::GoalSnapshot> = value
                    .get("goal")
                    .filter(|value| !value.is_null())
                    .map(|value| serde_json::from_value(value.clone()))
                    .transpose()
                    .map_err(|error| error.to_string())?;
                if let Some(ledger) = value.get("goal_ledger") {
                    let ledger: Vec<refact_chat_api::GoalLedgerEntry> =
                        serde_json::from_value(ledger.clone())
                            .map_err(|error| error.to_string())?;
                    if let Some(state) = refact_chat_api::reduce_goal_ledger(&ledger) {
                        let target = goal
                            .as_mut()
                            .ok_or("Previous goal ledger has no saved goal")?;
                        state.apply_to_snapshot(target);
                    }
                }
                let held = value
                    .get("pending_browser_decision")
                    .is_some_and(|value| !value.is_null())
                    || value.get("pending_questions").is_some_and(|value| {
                        !value.as_array().is_some_and(|items| items.is_empty())
                    });
                (Some(thread), messages, goal, None, held, false)
            }
        } else {
            return Err(
                "Previous card attempt identity is missing; inspect before restarting".into(),
            );
        };
        if let Some(thread) = &thread {
            let meta = thread
                .task_meta
                .as_ref()
                .ok_or("Previous task binding is missing")?;
            if thread.id != old_chat.as_deref().unwrap_or_default()
                || !matches!(meta.role.as_str(), "agent" | "agents")
                || meta.task_id != task_id
                || meta.card_id.as_deref() != Some(card.id.as_str())
                || meta.agent_id != card.assignee
                || meta.planner_chat_id.as_deref() != Some(planner)
                || thread.parent_id.as_deref() != Some(planner)
            {
                return Err("Previous runtime does not belong to this planner/card attempt".into());
            }
            let worktree = thread
                .worktree
                .as_ref()
                .ok_or("Previous checkout evidence is missing")?;
            if worktree.source_workspace_root != source.root()
                || card.agent_worktree_name.as_deref() != Some(worktree.id.as_str())
                || card
                    .agent_worktree
                    .as_deref()
                    .and_then(|path| dunce::canonicalize(path).ok())
                    .as_ref()
                    != Some(&worktree.root)
                || card.agent_branch != worktree.branch
            {
                return Err("Previous runtime checkout does not match the retained card".into());
            }
        }
        let active = refact_core::active_context::active_context(&messages)
            .map_err(|error| error.to_string())?;
        let mut pending = std::collections::HashSet::new();
        let mut denied = false;
        for message in &active.messages {
            for call in message.tool_calls.iter().flatten() {
                pending.insert(call.id.as_str());
            }
            if matches!(message.role.as_str(), "tool" | "diff" | "context_file") {
                pending.remove(message.tool_call_id.as_str());
            }
            denied |= message.extra.get("event").is_some_and(|event| {
                event["subkind"] == "tool_decision"
                    && matches!(
                        event["payload"]["decision"].as_str(),
                        Some("reject" | "rejected")
                    )
            });
        }
        let old_approvals = old_chat.as_ref().is_some_and(|chat| {
            app.agent_approvals
                .list_for_parent(planner, crate::agents::approval::now_ms())
                .iter()
                .any(|approval| approval.child_work_id == format!("board:{chat}"))
        });
        if held || !pending.is_empty() || denied || old_approvals {
            return Err("Previous attempt has a pause, approval or unknown tool outcome; authorized reconciliation is required before restart. No old request will be replayed or rebound".into());
        }
        if goal.is_none() && active.messages.iter().any(|message| message.role == "goal") {
            return Err("Previous goal has no authoritative usage snapshot; restart cannot reset its budget".into());
        }
        if let Some(base) = active
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "goal")
        {
            let _: refact_chat_api::GoalBudget = serde_json::from_value(
                base.extra
                    .get("goal")
                    .and_then(|meta| meta.get("budget"))
                    .cloned()
                    .ok_or("Previous goal budget evidence is missing")?,
            )
            .map_err(|_| "Previous goal budget evidence is invalid")?;
        }
        let goal =
            crate::chat::goal_snapshot_from_messages(&active.messages, goal.as_ref()).or(goal);
        if active.messages.iter().any(|message| message.role == "goal") && goal.is_none() {
            return Err("Previous goal state cannot be established; inspect before restart".into());
        }
        if goal.as_ref().is_some_and(|goal| {
            !goal.active
                || goal.status != refact_chat_api::GoalStatus::Active
                || goal.snoozed_until_ms.is_some()
                || goal.wait.is_some()
                || goal
                    .goal_budget_exhaustion_status_at(crate::agents::approval::now_ms())
                    .is_some()
        }) {
            return Err("Previous goal is held or exhausted; restart cannot clear its pause or finite budget".into());
        }
        if (busy || attempt.is_some()) && !force {
            return Err(
                "Previous attempt has live owned work; use force=true to stop it before restart"
                    .into(),
            );
        }
        if busy && attempt.is_none() {
            return Err(
                "Previous busy session has no owned board admission; force cannot stop it".into(),
            );
        }
        let controls = active
            .messages
            .into_iter()
            .filter(|message| {
                message.role == "goal"
                    || (message.role == "event"
                        && message
                            .extra
                            .get("event")
                            .is_some_and(|event| event["subkind"] == "goal_delta"))
            })
            .collect();
        Ok(Self {
            old_chat,
            session,
            attempt,
            revision,
            accepted_state,
            goal,
            controls,
            stopped_state: None,
            persisted,
        })
    }

    async fn stop(
        &mut self,
        app: &crate::app_state::AppState,
        source: &AcceptedAgentSource,
        task_id: &str,
        planner: &str,
        original: &BoardCard,
    ) -> Result<(), String> {
        let current_attempt = match self.old_chat.as_deref() {
            Some(chat) => crate::tasks::admission::Attempt::capture_for_app(app, chat)?,
            None => None,
        };
        if match (&self.attempt, &current_attempt) {
            (Some(expected), Some(current)) => !expected.same_attempt(current),
            (None, None) => false,
            _ => true,
        } {
            return Err("Previous admission changed before stop; checkout retained".into());
        }
        if let Some(attempt) = &self.attempt {
            if !attempt.belongs_to(app) {
                return Err("Previous admission changed before stop; checkout retained".into());
            }
            attempt.cancel_setup_and_wait(AGENT_ABORT_TIMEOUT).await?;
        }
        let Some(chat) = self.old_chat.as_deref() else {
            return Ok(());
        };
        let _transition = self
            .session
            .as_ref()
            .map(crate::chat::worktree_lifecycle::try_transition_lock)
            .transpose()?;
        let sessions = app.chat.sessions.read().await;
        match (&self.session, sessions.get(chat)) {
            (Some(expected), Some(current)) if Arc::ptr_eq(expected, current) => {
                let mut session = expected.lock().await;
                if Some(session.trajectory_version) != self.revision
                    || Some(Self::session_state(&session)?) != self.accepted_state
                {
                    return Err("Previous trajectory changed before stop; inspect again".into());
                }
                session.abort_stream();
                session.close_event_channel();
                session.queue_notify.notify_waiters();
            }
            (None, None) => {}
            _ => return Err("Previous live session changed; checkout retained".into()),
        }
        drop(sessions);
        let stopped = async {
            loop {
                let session_busy = if let Some(session) = &self.session {
                    let session = session.lock().await;
                    session.turn_depth > 0
                        || session
                            .queue_processor_running
                            .load(std::sync::atomic::Ordering::SeqCst)
                        || matches!(
                            session.runtime.state,
                            refact_runtime_api::SessionState::Generating
                                | refact_runtime_api::SessionState::ExecutingTools
                        )
                } else {
                    false
                };
                let operation_busy = app
                    .agent_executions
                    .get(chat)
                    .is_some_and(|execution| execution.active_operations() > 0);
                if !session_busy && !operation_busy {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        };
        tokio::time::timeout(AGENT_ABORT_TIMEOUT, stopped)
            .await
            .map_err(|_| {
                "Previous operation did not stop; checkout and ownership retained".to_string()
            })?;
        if let Some(expected) = &self.session {
            let final_recovery =
                Self::capture(app, source, task_id, planner, original, true).await?;
            if !final_recovery
                .session
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(expected, current))
            {
                return Err("Previous session replaced during stop; checkout retained".into());
            }
            self.goal = final_recovery.goal;
            self.controls = final_recovery.controls;
            let session = self
                .session
                .as_ref()
                .ok_or("Previous session unavailable")?;
            let session = session.lock().await;
            self.stopped_state = Some(Self::session_state(&session)?);
        }
        if let Some(attempt) = &self.attempt {
            attempt.end();
        }
        Ok(())
    }

    fn session_state(session: &crate::chat::types::ChatSession) -> Result<Value, String> {
        serde_json::to_value((
            &session.thread,
            &session.messages,
            &session.goal,
            session.planner_pause_revision,
            session.board_restored_pending_tools,
            session.trajectory_version,
            session.closed,
            &session.runtime.pause_reasons,
            &session.pending_questions,
            session.confirmation_paused_at.as_ref().map(|(_, ids)| ids),
            &session.draft_message,
            session.runtime.state,
            &session.approval_wrapper_calls,
            session.pending_browser_message.is_some(),
        ))
        .map_err(|error| error.to_string())
    }

    async fn validate_stopped(
        &self,
        app: &crate::app_state::AppState,
        planner: &str,
    ) -> Result<(), String> {
        let Some(chat) = self.old_chat.as_deref() else {
            return Ok(());
        };
        let _transition = self
            .session
            .as_ref()
            .map(crate::chat::worktree_lifecycle::try_transition_lock)
            .transpose()?;
        let current = app.chat.sessions.read().await.get(chat).cloned();
        match (&self.session, current) {
            (Some(expected), Some(current)) if Arc::ptr_eq(expected, &current) => {
                if Some(Self::session_state(&*current.lock().await)?) != self.stopped_state {
                    return Err(
                        "Previous attempt changed while queued; inspect before retry".into(),
                    );
                }
            }
            (None, None) => {}
            _ => return Err("Previous session replaced while queued".into()),
        }
        if let Some((path, digest)) = &self.persisted {
            storage::require_regular_task_file(path).await?;
            let body = crate::coordination::board_bootstrap::read_bounded(
                path,
                path.parent().ok_or("Previous trajectory parent missing")?,
                crate::file_activity::settings().max_work_history_bytes,
            )
            .await
            .map_err(|_| "Previous trajectory changed while queued")?;
            if crate::file_activity::digest_of(&body) != *digest {
                return Err("Previous saved trajectory changed while queued".into());
            }
        }
        if crate::tasks::admission::Attempt::capture_for_app(app, chat)?.is_some()
            || app.agent_executions.get(chat).is_some()
        {
            return Err("Previous attempt reacquired ownership while queued".into());
        }
        if app
            .agent_approvals
            .list_for_parent(planner, crate::agents::approval::now_ms())
            .iter()
            .any(|approval| approval.child_work_id == format!("board:{chat}"))
        {
            return Err("Previous approval appeared while queued; inspect before retry".into());
        }
        Ok(())
    }

    fn seed_controls(&self, chat: &str) -> Result<Vec<ChatMessage>, String> {
        let mut controls = self.controls.clone();
        let mut ids = std::collections::HashSet::new();
        controls.retain(|message| ids.insert(message.message_id.clone()));
        if let Some(goal) = &self.goal {
            if goal
                .goal_budget_exhaustion_status_at(crate::agents::approval::now_ms())
                .is_some()
            {
                return Err(
                    "Previous finite goal budget is exhausted; restart cannot reset it".into(),
                );
            }
            let base = controls
                .iter_mut()
                .rev()
                .find(|message| message.role == "goal")
                .ok_or("Previous goal control history is unavailable")?;
            let meta = base
                .extra
                .get_mut("goal")
                .ok_or("Previous goal metadata is unavailable")?;
            let snapshot = serde_json::to_value(goal).map_err(|error| error.to_string())?;
            for field in [
                "active", "status", "budget", "progress", "attempts", "events", "criteria",
            ] {
                meta[field] = snapshot[field].clone();
            }
            meta["owner"] = json!({"kind": "chat", "id": chat});
        }
        Ok(controls)
    }
}

async fn validate_restart_planner(
    app: &crate::app_state::AppState,
    task_id: &str,
    planner: &str,
    source: &AcceptedAgentSource,
) -> Result<(), String> {
    source.validate_task(app.gcx.clone(), task_id).await?;
    let board = storage::load_board(app.gcx.clone(), task_id).await?;
    board.require_owner(planner)?;
    let meta = storage::load_task_meta(app.gcx.clone(), task_id).await?;
    if !matches!(
        meta.status,
        crate::tasks::types::TaskStatus::Active | crate::tasks::types::TaskStatus::Planning
    ) {
        return Err("Task is paused or terminal; restart is not a resume control".into());
    }
    let task_dir = storage::find_task_dir(app.gcx.clone(), task_id).await?;
    let goal_path = task_dir.join("goal.json");
    if goal_path.exists() {
        storage::require_regular_task_file(&goal_path).await?;
        let body = crate::coordination::board_bootstrap::read_bounded(
            &goal_path,
            &task_dir,
            crate::file_activity::settings().max_work_history_bytes,
        )
        .await
        .map_err(|_| "Canonical task goal unavailable")?;
        let record: crate::tasks::goal::TaskGoalRecord =
            serde_json::from_str(&body).map_err(|error| error.to_string())?;
        if record.snapshot(task_id).is_some_and(|goal| {
            !goal.active
                || goal.status != refact_chat_api::GoalStatus::Active
                || goal.snoozed_until_ms.is_some()
                || goal.wait.is_some()
                || goal
                    .goal_budget_exhaustion_status_at(crate::agents::approval::now_ms())
                    .is_some()
        }) {
            return Err("Canonical task goal is held; restart cannot resume it".into());
        }
    }
    let planner = app
        .chat
        .sessions
        .read()
        .await
        .get(planner)
        .cloned()
        .ok_or("Current planner session is unavailable")?;
    let planner = planner.lock().await;
    if planner.closed
        || planner.planner_paused()
        || planner.runtime.paused
        || !planner.runtime.pause_reasons.is_empty()
        || !planner
            .thread
            .task_meta
            .as_ref()
            .is_some_and(|meta| meta.role == "planner" && meta.task_id == task_id)
        || crate::coordination::scope::session_source_root(&planner).as_deref()
            != Some(source.root())
        || planner.goal.as_ref().is_some_and(|goal| {
            !goal.active
                || goal.status != refact_chat_api::GoalStatus::Active
                || goal.snoozed_until_ms.is_some()
                || goal.wait.is_some()
                || goal
                    .goal_budget_exhaustion_status_at(crate::agents::approval::now_ms())
                    .is_some()
        })
    {
        return Err("Current owning planner is held or its project binding changed".into());
    }
    Ok(())
}

fn context_file_message(
    gcx: &Arc<GlobalContext>,
    context_files: Vec<ContextFile>,
) -> Result<ChatMessage, String> {
    let records = crate::privacy::records::declared_file_records(
        gcx,
        context_files
            .iter()
            .map(|file| PathBuf::from(&file.file_name)),
    )?;
    let mut message = ChatMessage {
        role: "context_file".to_string(),
        content: ChatContent::ContextFiles(context_files),
        tool_call_id: "initial_files".to_string(),
        ..Default::default()
    };
    crate::privacy::records::merge_records(&mut message, records);
    Ok(message)
}

async fn cleanup_old_worktree(
    gcx: Arc<GlobalContext>,
    source: &AcceptedAgentSource,
    agent_worktree_name: Option<&str>,
    agent_worktree: Option<&str>,
    agent_branch: Option<&str>,
) {
    let cleanup = async {
        source.validate(gcx.clone()).await?;
        let name = agent_worktree_name.ok_or("Previous worktree registration is missing")?;
        let service =
            WorktreeService::new_async(gcx.cache_dir.clone(), source.root().to_path_buf()).await?;
        let registered = service.get_worktree(name).await?;
        if registered.meta.branch.as_deref() != agent_branch
            || agent_worktree
                .and_then(|path| dunce::canonicalize(path).ok())
                .as_ref()
                != Some(&registered.meta.root)
        {
            return Err("Previous worktree identity changed".to_string());
        }
        service.delete_worktree(name, true, true).await.map(|_| ())
    }
    .await;
    if let Err(error) = cleanup {
        tracing::warn!(%error, "Previous task-agent worktree retained; reconciliation required");
    }
}

async fn fresh_restart_diff_report(
    card: &BoardCard,
    force: bool,
) -> Result<Option<String>, String> {
    if card.agent_worktree.is_none()
        && card.agent_worktree_name.is_none()
        && card.agent_branch.is_none()
    {
        return Ok(None);
    }
    let capture = match card.agent_worktree.as_deref() {
        Some(worktree) => {
            crate::chat::task_agent_monitor::capture_failed_agent_diff(
                worktree,
                card.base_commit.as_deref(),
                card.base_branch.as_deref(),
            )
            .await
        }
        None => Err("Previous worktree path is missing".to_string()),
    };
    match capture {
        Ok(diff) => Ok(Some(format!("Fresh restart: previous worktree diff captured.{diff}"))),
        Err(error) if force => Ok(Some(format!(
            "Fresh restart forced; no diff captured: {error}"
        ))),
        Err(error) => Err(format!(
            "Fresh restart refused: {error}. Previous worktree retained; use force=true to restart without capturing its diff."
        )),
    }
}

async fn restore_card_after_restart_failure(
    gcx: Arc<GlobalContext>,
    source: &AcceptedAgentSource,
    task_id: &str,
    mut original_card: BoardCard,
    guard_chat_id: String,
    status_update: Option<String>,
) {
    let _ = (task_id, &mut original_card, guard_chat_id);
    source
        .reconcile(
            gcx,
            true,
            status_update.as_deref().unwrap_or("Restart setup failed"),
        )
        .await;
}

fn mark_card_restarted_fresh(
    card: &mut BoardCard,
    new_agent_id: &str,
    new_agent_chat_id: &str,
    worktree_branch: Option<String>,
    worktree_path: Option<String>,
    worktree_name: Option<String>,
) {
    if let Some(prev_report) = card.final_report.take() {
        let preview: String = prev_report.chars().take(300).collect();
        card.status_updates.push(StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: format!("Previous failure: {}", preview),
        });
    }
    card.final_report_structured = None;
    card.column = "doing".to_string();
    card.completed_at = None;
    mark_card_agent_started(
        card,
        new_agent_id,
        new_agent_chat_id,
        worktree_branch,
        worktree_path,
        worktree_name,
    );
    card.status_updates.last_mut().map(|u| {
        u.message = "Fresh restart: new agent started".to_string();
    });
}

fn mark_card_restarted_resume(card: &mut BoardCard, new_agent_id: &str, new_agent_chat_id: &str) {
    if let Some(prev_report) = card.final_report.take() {
        let preview: String = prev_report.chars().take(300).collect();
        card.status_updates.push(StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: format!("Previous failure: {}", preview),
        });
    }
    card.final_report_structured = None;
    card.column = "doing".to_string();
    card.completed_at = None;
    card.assignee = Some(new_agent_id.to_string());
    card.agent_chat_id = Some(new_agent_chat_id.to_string());
    card.started_at = Some(Utc::now().to_rfc3339());
    card.status_updates.push(StatusUpdate {
        timestamp: Utc::now().to_rfc3339(),
        message: "Resume restart: new agent attached to existing worktree".to_string(),
    });
}

fn ensure_restart_source_current(card: &BoardCard, original: &BoardCard) -> Result<(), String> {
    if card.column == original.column
        && card.assignee == original.assignee
        && card.agent_chat_id == original.agent_chat_id
        && card.agent_branch == original.agent_branch
        && card.agent_worktree == original.agent_worktree
        && card.agent_worktree_name == original.agent_worktree_name
        && card.paused_at == original.paused_at
        && card.paused_by == original.paused_by
        && card.instructions == original.instructions
        && card.depends_on == original.depends_on
        && card.base_branch == original.base_branch
        && card.base_commit == original.base_commit
        && card.target_files == original.target_files
        && card.scope_guard_mode == original.scope_guard_mode
        && card.started_at == original.started_at
        && card.title == original.title
    {
        return Ok(());
    }
    Err(format!(
        "Card {} changed while restart was preparing; expected column={}, assignee={}, agent_chat_id={}, found column={}, assignee={}, agent_chat_id={}. Run check_agents and retry.",
        original.id,
        original.column,
        original.assignee.as_deref().unwrap_or("none"),
        original.agent_chat_id.as_deref().unwrap_or("none"),
        card.column,
        card.assignee.as_deref().unwrap_or("none"),
        card.agent_chat_id.as_deref().unwrap_or("none")
    ))
}

async fn claim_restart(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    original: &BoardCard,
    agent_id: &str,
    agent_chat_id: &str,
    mode: &str,
) -> Result<BoardCard, String> {
    let original = original.clone();
    let agent_id = agent_id.to_string();
    let agent_chat_id = agent_chat_id.to_string();
    let mode = mode.to_string();
    let (_, queued) = storage::update_board_atomic(gcx, task_id, move |board| {
        if board.card_claim(&original.id).is_some() {
            return Err(
                "Card claim changed before restart; inspect and release only the owned claim"
                    .into(),
            );
        }
        board
            .dependency_gate(&original.id)
            .map_err(storage::violation_error)?;
        let card = board
            .get_card_mut(&original.id)
            .ok_or("restart card removed")?;
        ensure_restart_source_current(card, &original)?;
        if card.paused_at.is_some() || card.paused_by.is_some() {
            return Err("Card is paused; restart cannot clear the pause".into());
        }
        card.column = "doing".into();
        card.assignee = Some(agent_id);
        card.agent_chat_id = Some(agent_chat_id);
        card.started_at = None;
        card.status_updates.push(StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: format!("Agent restart queued (mode={mode}); previous checkout retained"),
        });
        Ok(Some(card.clone()))
    })
    .await?;
    queued.ok_or_else(|| "restart claim returned no card".into())
}

async fn get_worktree_meta_for_resume(
    gcx: Arc<GlobalContext>,
    source: &AcceptedAgentSource,
    task_id: &str,
    card: &BoardCard,
) -> Result<WorktreeMeta, String> {
    let wt_name = card.agent_worktree_name.as_deref().ok_or_else(|| {
        format!(
            "Card {} has no retained worktree (agent_worktree_name is missing). Use mode=fresh to create a new worktree.",
            card.id
        )
    })?;
    let wt_path = card
        .agent_worktree
        .as_deref()
        .ok_or("Retained worktree path is missing")?;
    if !Path::new(wt_path).exists() {
        return Err(format!(
            "Retained worktree path '{}' for card {} no longer exists. Use mode=fresh to create a new worktree.",
            wt_path, card.id
        ));
    }
    let cache_dir = gcx.cache_dir.clone();
    source.validate(gcx.clone()).await?;
    let service = WorktreeService::new_async(cache_dir, source.root().to_path_buf()).await?;
    let view = service.get_worktree(wt_name).await.map_err(|e| {
        format!(
            "Retained worktree '{}' is no longer registered: {}. Use mode=fresh.",
            wt_name, e
        )
    })?;
    if !view.meta.root.exists() {
        return Err(format!(
            "Retained worktree root '{}' no longer exists on disk. Use mode=fresh.",
            view.meta.root.display()
        ));
    }
    if view.meta.task_id.as_deref() != Some(task_id)
        || view.meta.card_id.as_deref() != Some(card.id.as_str())
        || card.agent_branch != view.meta.branch
        || dunce::canonicalize(wt_path).ok().as_ref() != Some(&view.meta.root)
        || view.meta.kind != "task_agent"
        || view.meta.source_workspace_root != source.root()
        || view.meta.agent_id.is_none()
    {
        return Err("Retained worktree identity does not match the restarted card".into());
    }
    service.validate_worktree_meta_strict(&view.meta).await
}

pub struct ToolTaskRestartAgent;

impl ToolTaskRestartAgent {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskRestartAgent {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "restart_agent".to_string(),
            display_name: "Task Restart Agent".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Explicit owning-planner restart of a failed or interrupted task-card attempt through scheduler admission. mode=resume creates a NEW agent/chat on the SAME card and retained dirty worktree; it does not continue the old chat or replay its tools/approvals. mode=fresh (default) replaces the checkout after successful setup. Paused goals/cards, old approvals and unknown tool outcomes require authorized reconciliation first; force only permits stopping actually live owned work.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "card_id": {
                        "type": "string",
                        "description": "Card ID to restart"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["fresh", "resume"],
                        "description": "Restart mode: 'fresh' (default) deletes the old worktree and creates a new one; 'resume' reuses the retained worktree from the previous run."
                    },
                    "force": {
                        "type": "boolean",
                        "description": "If true, allow restarting an active agent (aborts it first) and allow a fresh restart when the previous worktree diff cannot be captured, recording the capture failure."
                    },
                    "suggested_steps": {
                        "type": "integer",
                        "description": "Suggested step budget for the agent (default: 30)."
                    },
                    "files_to_open": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of file paths to open immediately when the agent starts."
                    }
                },
                "required": ["card_id"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let ccx_lock = ccx.lock().await;
        let is_planner = ccx_lock
            .task_meta
            .as_ref()
            .map(|m| m.role == "planner")
            .unwrap_or(false);
        if !is_planner {
            return Err("restart_agent can only be called by the task planner.".to_string());
        }
        let task_id = ccx_lock
            .task_meta
            .as_ref()
            .map(|m| m.task_id.clone())
            .unwrap_or_default();
        let planner_chat_id = ccx_lock
            .task_meta
            .as_ref()
            .and_then(|m| m.planner_chat_id.clone())
            .unwrap_or_else(|| ccx_lock.chat_id.clone());
        let current_chat_id = ccx_lock.chat_id.clone();
        if planner_chat_id != current_chat_id {
            return Err("restart_agent requires the current bound planner, not a delegated planner identity".into());
        }
        let root_chat_id = ccx_lock.root_chat_id.clone();
        let depth = ccx_lock.subchat_depth;
        let planner_worktree = ccx_lock.execution_scope_worktree();
        let app = ccx_lock.app.clone();
        let current_model = ccx_lock.current_model.clone();
        let gcx = ccx_lock.app.gcx.clone();
        drop(ccx_lock);

        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("fresh");
        if mode != "fresh" && mode != "resume" {
            return Err(format!(
                "Invalid mode '{}', must be 'fresh' or 'resume'",
                mode
            ));
        }
        let force = args
            .get("force")
            .and_then(refact_tool_api::coerce_bool)
            .unwrap_or(false);
        let suggested_steps: usize = match args
            .get("suggested_steps")
            .or_else(|| args.get("max_steps"))
        {
            Some(Value::String(s)) => s.parse().unwrap_or(30),
            Some(Value::Number(n)) => n.as_u64().unwrap_or(30) as usize,
            _ => 30,
        };
        let suggested_steps = suggested_steps.min(50).max(1);
        let files_to_open: Vec<String> = args
            .get("files_to_open")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let source_root = planner_worktree
            .as_ref()
            .map(|w| w.source_workspace_root.clone())
            .or(crate::files_correction::get_active_project_path(gcx.clone()).await)
            .ok_or("No active project for restarted agent")?;
        let source =
            AcceptedAgentSource::capture(gcx.clone(), &source_root, planner_worktree.as_ref())
                .await?
                .bind_task(gcx.clone(), &task_id)
                .await?;
        let binding = source.task_binding(&task_id)?.clone();
        binding
            .scope(async {
                validate_restart_planner(&app, &task_id, &current_chat_id, &source).await?;
                let task_meta = storage::load_task_meta(gcx.clone(), &task_id).await?;
                let model = resolve_agent_model(gcx.clone(), &current_model).await?;
                crate::tools::task_tool_helpers::preflight_agent_model(gcx.clone(), &model).await?;

                let board = storage::load_board(gcx.clone(), &task_id).await?;
                let card = board
                    .get_card(card_id)
                    .ok_or_else(|| format!("Card {} not found", card_id))?;
                if board.card_claim(card_id).is_some() {
                    return Err("Card has an existing claimant; its owning claimant must explicitly release it before restart".into());
                }
                if card
                    .ab_variants
                    .as_ref()
                    .is_some_and(|variants| variants.winner.is_none())
                {
                    return Err(
                        "Pick or cancel the A/B variants before restarting this card".into(),
                    );
                }
                match card.column.as_str() {
                    "done" => {
                        return Err(format!(
                            "Card {} is already done and cannot be restarted.",
                            card_id
                        ))
                    }
                    "planned" => {
                        return Err(format!(
                "Card {} is in 'planned' column. Use spawn_agent to start it for the first time.",
                card_id
            ))
                    }
                    "failed" => {}
                    "doing" => {}
                    other => {
                        return Err(format!(
                            "Card {} is in unexpected column '{}'",
                            card_id, other
                        ))
                    }
                }
                board
                    .dependency_gate(card_id)
                    .map_err(storage::violation_error)?;
                if card.paused_at.is_some() || card.paused_by.is_some() {
                    return Err(
                        "Card is paused; use its authorized resume control before restarting"
                            .into(),
                    );
                }
                let mut recovery = RestartRecovery::capture(
                    &app,
                    &source,
                    &task_id,
                    &planner_chat_id,
                    card,
                    force,
                )
                .await?;
                let retained = if mode == "resume" {
                    Some(get_worktree_meta_for_resume(gcx.clone(), &source, &task_id, card).await?)
                } else {
                    None
                };
                if mode == "fresh" {
                    fresh_restart_diff_report(card, force).await?;
                }
                let card_title = card.title.clone();
                let card_instructions = card.instructions.clone();
                let dependency_context = board
                    .get_dependency_reports(card_id)
                    .into_iter()
                    .map(|(title, report)| format!("### {}\n{}", title, report))
                    .collect::<Vec<_>>()
                    .join("\n\n");

                let root = if root_chat_id.is_empty() {
                    &planner_chat_id
                } else {
                    &root_chat_id
                };
                let original = card.clone();
                let agent_id = Uuid::new_v4().to_string();
                let agent_chat_id = format!("agent-{card_id}-{}", &agent_id[..8]);
                recovery.seed_controls(&agent_chat_id)?;
                crate::tasks::admission::reserve(
                    &app,
                    &agent_chat_id,
                    source.project(),
                    root,
                    depth,
                )
                .await
                .map_err(|e| e.to_string())?;
                let attempt = crate::tasks::admission::current_attempt(&agent_chat_id)
                    .ok_or("Restart admission attempt unavailable")?;
                let source =
                    match source.with_attempt(&task_id, &original, &agent_id, &agent_chat_id, None)
                    {
                        Ok(source) => source,
                        Err(error) => {
                            attempt.end();
                            return Err(error);
                        }
                    };
                let setup_attempt = attempt.clone();
                let result_message = json!({
                    "summary": format!("Agent restart queued: {card_title}"),
                    "card_id": card_id, "agent_id": agent_id, "agent_chat_id": agent_chat_id,
                    "mode": mode, "execution_state": "queued", "model": model,
                    "worktree": null,
                })
                .to_string();
                let card_id = card_id.to_string();
                let mode = mode.to_string();
                let tracked_chat_id = agent_chat_id.clone();
                let (accepted, acceptance) = tokio::sync::oneshot::channel();
                let queued_binding = source.task_binding(&task_id)?.clone();
                let registered =
                    crate::tasks::admission::spawn_tracked(&tracked_chat_id, async move {
                        queued_binding
                            .scope(async move {
                                use futures::FutureExt;
                                let mut accepted = Some(accepted);
                                let setup = async {
                                    if accepted.as_ref().is_none_or(|tx| tx.is_closed()) {
                                        return Err(
                                            "restart caller cancelled before claim".to_string()
                                        );
                                    }
                                    source.validate(gcx.clone()).await?;
                                    source.validate_task(gcx.clone(), &task_id).await?;
                                    validate_restart_planner(
                                        &app,
                                        &task_id,
                                        &planner_chat_id,
                                        &source,
                                    )
                                    .await?;
                                    let current_recovery = RestartRecovery::capture(
                                        &app,
                                        &source,
                                        &task_id,
                                        &planner_chat_id,
                                        &original,
                                        force,
                                    )
                                    .await?;
                                    if current_recovery.accepted_state != recovery.accepted_state
                                        || current_recovery.persisted != recovery.persisted
                                        || match (&current_recovery.attempt, &recovery.attempt) {
                                            (Some(current), Some(expected)) => {
                                                !current.same_attempt(expected)
                                            }
                                            (None, None) => false,
                                            _ => true,
                                        }
                                        || current_recovery.revision != recovery.revision
                                        || serde_json::to_value(&current_recovery.goal)
                                            .map_err(|error| error.to_string())?
                                            != serde_json::to_value(&recovery.goal)
                                                .map_err(|error| error.to_string())?
                                    {
                                        return Err(
                                            "Previous attempt changed before acceptance".into()
                                        );
                                    }
                                    let queued = claim_restart(
                                        gcx.clone(),
                                        &task_id,
                                        &original,
                                        &agent_id,
                                        &agent_chat_id,
                                        &mode,
                                    )
                                    .await?;
                                    if accepted
                                        .take()
                                        .ok_or("restart acceptance lost")?
                                        .send(Ok(()))
                                        .is_err()
                                    {
                                        restore_card_after_restart_failure(
                                            gcx.clone(),
                                            &source,
                                            &task_id,
                                            original.clone(),
                                            agent_chat_id.clone(),
                                            None,
                                        )
                                        .await;
                                        return Err("restart caller cancelled during claim".into());
                                    }
                                    recovery
                                        .stop(&app, &source, &task_id, &planner_chat_id, &original)
                                        .await?;
                                    crate::tasks::admission::wait_for_permit(&agent_chat_id)
                                        .await
                                        .map_err(|e| e.to_string())?;
                                    if !setup_attempt.is_current() {
                                        return Err("Restart attempt replaced".into());
                                    }
                                    let execution = crate::tasks::admission::current_execution(
                                        &app,
                                        &agent_chat_id,
                                    )
                                    .map_err(|error| error.recovery_message())?;
                                    let _operation = execution
                                        .begin_operation()
                                        .await
                                        .map_err(|e| e.to_string())?;
                                    source.validate(gcx.clone()).await?;
                                    source.validate_task(gcx.clone(), &task_id).await?;
                                    validate_restart_planner(
                                        &app,
                                        &task_id,
                                        &planner_chat_id,
                                        &source,
                                    )
                                    .await?;
                                    let board = storage::load_board(gcx.clone(), &task_id).await?;
                                    board
                                        .dependency_gate(&card_id)
                                        .map_err(storage::violation_error)?;
                                    ensure_restart_source_current(
                                        board.get_card(&card_id).ok_or("restart card removed")?,
                                        &queued,
                                    )?;
                                    if execution.is_cancelled() {
                                        return Err("restart setup cancelled".into());
                                    }
                                    if let Some(retained) = &retained {
                                        if get_worktree_meta_for_resume(
                                            gcx.clone(),
                                            &source,
                                            &task_id,
                                            &original,
                                        )
                                        .await?
                                            != *retained
                                        {
                                            return Err(
                                                "Retained checkout changed while queued".into()
                                            );
                                        }
                                    }
                                    recovery.validate_stopped(&app, &planner_chat_id).await?;
                                    let tool = ToolTaskRestartAgent::new();
                                    if mode == "resume" {
                                        tool.execute_resume(
                                            app.clone(),
                                            &source,
                                            &task_id,
                                            &planner_chat_id,
                                            &current_chat_id,
                                            &card_id,
                                            &card_title,
                                            &card_instructions,
                                            &dependency_context,
                                            &model,
                                            suggested_steps,
                                            files_to_open,
                                            &original,
                                            &queued,
                                            &agent_id,
                                            &agent_chat_id,
                                            &recovery,
                                        )
                                        .await
                                    } else {
                                        tool.execute_fresh(
                                            app.clone(),
                                            &source,
                                            &task_id,
                                            &planner_chat_id,
                                            &task_meta,
                                            &card_id,
                                            &card_title,
                                            &card_instructions,
                                            &dependency_context,
                                            &model,
                                            suggested_steps,
                                            files_to_open,
                                            &original,
                                            &queued,
                                            &agent_id,
                                            &agent_chat_id,
                                            force,
                                            &recovery,
                                        )
                                        .await
                                    }
                                };
                                let result = std::panic::AssertUnwindSafe(setup)
                                    .catch_unwind()
                                    .await
                                    .unwrap_or_else(|_| Err("agent restart setup panicked".into()));
                                if let Err(error) = result {
                                    if let Some(accepted) = accepted.take() {
                                        let _ = accepted.send(Err(error.clone()));
                                    }
                                    source.reconcile(gcx.clone(), true, &error).await;
                                    setup_attempt.end();
                                }
                                if source.validate_task(gcx.clone(), &task_id).await.is_ok() {
                                    let _ = storage::update_task_stats(gcx, &task_id).await;
                                }
                            })
                            .await
                    })
                    .await;
                if let Err(error) = registered {
                    attempt.end();
                    return Err(error.to_string());
                }
                acceptance
                    .await
                    .map_err(|_| "restart setup ended before acceptance".to_string())??;

                Ok((
                    false,
                    vec![ContextEnum::ChatMessage(ChatMessage {
                        role: "tool".to_string(),
                        content: ChatContent::SimpleText(result_message),
                        tool_calls: None,
                        tool_call_id: tool_call_id.clone(),
                        ..Default::default()
                    })],
                ))
            })
            .await
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

impl ToolTaskRestartAgent {
    async fn execute_fresh(
        &self,
        app: crate::app_state::AppState,
        source: &AcceptedAgentSource,
        task_id: &str,
        planner_chat_id: &str,
        task_meta: &crate::tasks::types::TaskMeta,
        card_id: &str,
        card_title: &str,
        card_instructions: &str,
        dependency_context: &str,
        model: &str,
        suggested_steps: usize,
        files_to_open: Vec<String>,
        card: &BoardCard,
        queued_card: &BoardCard,
        agent_id: &str,
        agent_chat_id: &str,
        force: bool,
        recovery: &RestartRecovery,
    ) -> Result<String, String> {
        let gcx = app.gcx.clone();
        let original_card = card.clone();
        let diff_report = match fresh_restart_diff_report(&original_card, force).await {
            Ok(report) => report,
            Err(error) => {
                tracing::warn!(card_id, reason = %error, "Fresh restart refused after it was queued");
                restore_card_after_restart_failure(
                    gcx.clone(),
                    source,
                    task_id,
                    original_card,
                    agent_chat_id.to_string(),
                    Some(format!(
                        "Fresh restart refused after it was queued; retry with force=true to restart without the capture: {error}"
                    )),
                )
                .await;
                return Err(error);
            }
        };

        let mut restart_task_meta = task_meta.clone();
        if original_card.base_branch.is_some() || original_card.base_commit.is_some() {
            restart_task_meta.base_branch = original_card.base_branch.clone();
            restart_task_meta.base_commit = original_card.base_commit.clone();
        }
        let prepared_worktree = prepare_agent_worktree(
            gcx.clone(),
            source,
            &restart_task_meta,
            task_id,
            &agent_id,
            card_id,
            &agent_chat_id,
        )
        .await?;

        let worktree_branch = prepared_worktree.branch_name();
        let worktree_path_str = Some(
            prepared_worktree
                .worktree_path()
                .to_string_lossy()
                .to_string(),
        );
        let worktree_name = Some(prepared_worktree.worktree_name());
        let worktree_meta = prepared_worktree.worktree_meta();
        let prepared_base_branch = worktree_meta.base_branch.clone();
        let prepared_base_commit = worktree_meta.base_commit.clone();
        let card_id_owned = card_id.to_string();
        let agent_id_clone = agent_id.to_string();
        let agent_chat_id_clone = agent_chat_id.to_string();
        let source_card = queued_card.clone();

        let link_execution = app
            .agent_executions
            .get(agent_chat_id)
            .ok_or("restart execution lost")?;
        if let Err(error) = source.validate_task(gcx.clone(), task_id).await {
            prepared_worktree.cleanup_unlinked(gcx.clone()).await;
            return Err(error);
        }
        let board_update_result =
            storage::update_board_atomic(gcx.clone(), task_id, move |board| {
                if link_execution.is_cancelled() {
                    return Err("restart setup cancelled".into());
                }
                let card = board
                    .get_card_mut(&card_id_owned)
                    .ok_or(format!("Card {} not found", card_id_owned))?;
                ensure_restart_source_current(card, &source_card)?;
                if let Some(message) = diff_report {
                    card.status_updates.push(StatusUpdate {
                        timestamp: Utc::now().to_rfc3339(),
                        message,
                    });
                }
                mark_card_restarted_fresh(
                    card,
                    &agent_id_clone,
                    &agent_chat_id_clone,
                    worktree_branch.clone(),
                    worktree_path_str.clone(),
                    worktree_name.clone(),
                );
                card.base_branch = prepared_base_branch.clone();
                card.base_commit = prepared_base_commit.clone();
                Ok(())
            })
            .await;

        if let Err(e) = board_update_result {
            prepared_worktree.cleanup_unlinked(gcx.clone()).await;
            return Err(e);
        }

        let _ = storage::update_task_stats(gcx.clone(), task_id).await;

        if let Err(e) = self
            .create_and_dispatch_session(
                app,
                source,
                task_id,
                planner_chat_id,
                card_id,
                card_title,
                card_instructions,
                dependency_context,
                model,
                &agent_id,
                &agent_chat_id,
                worktree_meta.clone(),
                suggested_steps,
                files_to_open,
                recovery,
            )
            .await
        {
            prepared_worktree.retain(gcx.clone()).await;
            restore_card_after_restart_failure(
                gcx.clone(),
                source,
                task_id,
                original_card,
                agent_chat_id.to_string(),
                None,
            )
            .await;
            return Err(e);
        }

        cleanup_old_worktree(
            gcx.clone(),
            source,
            original_card.agent_worktree_name.as_deref(),
            original_card.agent_worktree.as_deref(),
            original_card.agent_branch.as_deref(),
        )
        .await;

        Ok(format!(
            "# Agent Restarted (Fresh): {}\n\n**Card:** {}\n**Agent ID:** {}\n**Model:** {}\n**Mode:** fresh\n**Status:** Running in background\n\nNew agent started from scratch. Previous checkout cleanup requires matching source and registration; otherwise it is retained for reconciliation.",
            card_title, card_id, agent_id, model
        ))
    }

    async fn execute_resume(
        &self,
        app: crate::app_state::AppState,
        source: &AcceptedAgentSource,
        task_id: &str,
        planner_chat_id: &str,
        resume_chat_id: &str,
        card_id: &str,
        card_title: &str,
        card_instructions: &str,
        dependency_context: &str,
        model: &str,
        suggested_steps: usize,
        files_to_open: Vec<String>,
        card: &BoardCard,
        queued_card: &BoardCard,
        agent_id: &str,
        agent_chat_id: &str,
        recovery: &RestartRecovery,
    ) -> Result<String, String> {
        let gcx = app.gcx.clone();
        let original_card = card.clone();

        let worktree_meta =
            get_worktree_meta_for_resume(gcx.clone(), source, task_id, card).await?;
        let card_id_owned = card_id.to_string();
        let agent_id_clone = agent_id.to_string();
        let agent_chat_id_clone = agent_chat_id.to_string();
        let _ = resume_chat_id;
        let source_card = queued_card.clone();

        let link_execution = app
            .agent_executions
            .get(agent_chat_id)
            .ok_or("restart execution lost")?;
        source.validate_task(gcx.clone(), task_id).await?;
        storage::update_board_atomic(gcx.clone(), task_id, move |board| {
            if link_execution.is_cancelled() {
                return Err("restart setup cancelled".into());
            }
            let card = board
                .get_card_mut(&card_id_owned)
                .ok_or(format!("Card {} not found", card_id_owned))?;
            ensure_restart_source_current(card, &source_card)?;
            mark_card_restarted_resume(card, &agent_id_clone, &agent_chat_id_clone);
            Ok(())
        })
        .await?;

        let _ = storage::update_task_stats(gcx.clone(), task_id).await;

        if let Err(e) = self
            .create_and_dispatch_session(
                app,
                source,
                task_id,
                planner_chat_id,
                card_id,
                card_title,
                card_instructions,
                dependency_context,
                model,
                &agent_id,
                &agent_chat_id,
                worktree_meta,
                suggested_steps,
                files_to_open,
                recovery,
            )
            .await
        {
            restore_card_after_restart_failure(
                gcx.clone(),
                source,
                task_id,
                original_card,
                agent_chat_id.to_string(),
                None,
            )
            .await;
            return Err(e);
        }

        Ok(format!(
            "# Agent Restarted (Resume): {}\n\n**Card:** {}\n**Agent ID:** {}\n**Model:** {}\n**Mode:** resume\n**Status:** Running in background\n\nAgent attached to retained worktree and continuing.",
            card_title, card_id, agent_id, model
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_and_dispatch_session(
        &self,
        app: crate::app_state::AppState,
        source: &AcceptedAgentSource,
        task_id: &str,
        planner_chat_id: &str,
        card_id: &str,
        card_title: &str,
        card_instructions: &str,
        dependency_context: &str,
        model: &str,
        agent_id: &str,
        agent_chat_id: &str,
        worktree_meta: WorktreeMeta,
        suggested_steps: usize,
        files_to_open: Vec<String>,
        recovery: &RestartRecovery,
    ) -> Result<(), String> {
        let gcx = app.gcx.clone();
        source.validate(gcx.clone()).await?;
        validate_restart_planner(&app, task_id, planner_chat_id, source).await?;
        let execution = crate::tasks::admission::current_execution(&app, agent_chat_id)
            .map_err(|error| error.recovery_message())?;
        if execution.is_cancelled() {
            return Err("restart setup cancelled".into());
        }

        let mut thread = build_agent_thread_params(
            agent_chat_id,
            card_title,
            model,
            task_id,
            agent_id,
            card_id,
            planner_chat_id,
            worktree_meta.clone(),
        );
        thread.auto_approve_dangerous_commands =
            crate::agents::approval::dangerous_confirmation_enabled(&app, planner_chat_id).await;

        let board = storage::load_board(gcx.clone(), task_id).await?;
        if board
            .card_claim(card_id)
            .is_some_and(|claim| claim.claimant_chat_id != agent_chat_id)
        {
            return Err("Card claim belongs to a previous or foreign attempt; owning claimant must release it before restart".into());
        }
        board
            .dependency_gate(card_id)
            .map_err(storage::violation_error)?;
        let card = board.get_card(card_id).ok_or("Restart card removed")?;
        if card.column != "doing"
            || card.agent_chat_id.as_deref() != Some(agent_chat_id)
            || card.assignee.as_deref() != Some(agent_id)
            || card.paused_at.is_some()
        {
            return Err("Restart claim changed before dispatch".into());
        }
        let scope = AssignmentScope {
            target_files: card.target_files.clone(),
            guard: card.scope_guard_mode,
        };
        let assignment = Assignment::new(card_id, card_title, card_instructions)
            .with_dependency_context(dependency_context)
            .with_suggested_steps(suggested_steps)
            .with_scope(scope);
        source.validate_task(gcx.clone(), task_id).await?;
        assignment::record_assignment(gcx.clone(), task_id, &assignment).await;
        let user_prompt = assignment.render_prompt();

        let user_msg = event(
            EventSubkind::SystemNotice,
            "tool.task_restart_agent",
            json!({
                "task_id": task_id,
                "planner_chat_id": planner_chat_id,
                "card_id": card_id,
                "card_title": card_title,
                "agent_id": agent_id,
                "agent_chat_id": agent_chat_id,
                "model": model,
                "suggested_steps": suggested_steps,
                "files_to_open": &files_to_open,
                "worktree": worktree_meta.root.to_string_lossy().to_string(),
                "worktree_name": &worktree_meta.id,
                "branch": worktree_meta.branch.as_deref(),
            }),
            user_prompt,
        );
        let mut messages = recovery.seed_controls(agent_chat_id)?;
        messages.push(user_msg);

        if !files_to_open.is_empty() {
            let mut context_files: Vec<ContextFile> = Vec::new();
            let wt_root = &worktree_meta.root;
            let worktree_canonical =
                dunce::canonicalize(wt_root).unwrap_or_else(|_| wt_root.clone());
            for path_str in &files_to_open {
                let orig = std::path::Path::new(path_str);
                let source_root = &worktree_meta.source_workspace_root;
                let resolved = match orig.strip_prefix(source_root) {
                    Ok(rel) => wt_root.join(rel),
                    Err(_) => wt_root.join(path_str.trim_start_matches('/')),
                };
                let canonical_resolved = match dunce::canonicalize(&resolved) {
                    Ok(p) => p,
                    Err(_) => {
                        tracing::warn!(
                            "restart_agent: files_to_open '{}' does not exist, skipping",
                            path_str
                        );
                        continue;
                    }
                };
                if !canonical_resolved.starts_with(&worktree_canonical) {
                    tracing::warn!(
                        "restart_agent: files_to_open '{}' escapes worktree, rejecting",
                        path_str
                    );
                    continue;
                }
                if crate::files_in_workspace::check_file_privacy_for_model_context(
                    gcx.clone(),
                    &canonical_resolved,
                )
                .await
                .is_err()
                {
                    tracing::warn!(
                        "restart_agent: files_to_open '{}' blocked by privacy settings, skipping",
                        path_str
                    );
                    continue;
                }
                match tokio::fs::read_to_string(&canonical_resolved).await {
                    Ok(content) => {
                        let line_count = content.lines().count().max(1);
                        context_files.push(ContextFile {
                            file_name: canonical_resolved.to_string_lossy().to_string(),
                            file_content: content,
                            line1: 1,
                            line2: line_count,
                            ..Default::default()
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            "restart_agent: could not read file {:?}: {}",
                            canonical_resolved,
                            e
                        );
                    }
                }
            }
            if !context_files.is_empty() {
                messages.push(context_file_message(&gcx, context_files)?);
            }
        }

        source.validate(gcx.clone()).await?;
        validate_restart_planner(&app, task_id, planner_chat_id, source).await?;
        recovery.validate_stopped(&app, planner_chat_id).await?;
        recovery.seed_controls(agent_chat_id)?;
        let service =
            WorktreeService::new_async(gcx.cache_dir.clone(), source.root().to_path_buf()).await?;
        if service
            .validate_worktree_meta_strict(&worktree_meta)
            .await?
            != worktree_meta
        {
            return Err("Retained checkout changed before session creation".into());
        }
        if execution.is_cancelled() {
            return Err("Restart setup cancelled".into());
        }
        app.chat
            .facade
            .create_session(CreateSessionRequest {
                chat_id: agent_chat_id.to_string(),
                thread,
                messages,
            })
            .await?;
        if let Some(session) = app.chat.sessions.read().await.get(agent_chat_id).cloned() {
            let mut session = session.lock().await;
            if let Some(goal) = session.goal.clone() {
                session.set_goal_ledger(refact_chat_api::seed_transferred_goal_ledger(
                    &goal,
                    recovery
                        .old_chat
                        .as_deref()
                        .ok_or("Previous chat identity unavailable")?,
                    crate::agents::approval::now_ms(),
                ));
            }
        } else {
            return Err("Restart session was not installed by the runtime".into());
        }
        app.chat.facade.maybe_save_session(agent_chat_id).await?;
        validate_restart_planner(&app, task_id, planner_chat_id, source).await?;
        recovery.validate_stopped(&app, planner_chat_id).await?;
        recovery.seed_controls(agent_chat_id)?;
        let board = storage::load_board(gcx.clone(), task_id).await?;
        board
            .dependency_gate(card_id)
            .map_err(storage::violation_error)?;
        let card = board
            .get_card(card_id)
            .ok_or("Restart card removed before dispatch")?;
        if card.column != "doing"
            || card.agent_chat_id.as_deref() != Some(agent_chat_id)
            || card.assignee.as_deref() != Some(agent_id)
            || card.paused_at.is_some()
            || card.paused_by.is_some()
            || execution.is_cancelled()
        {
            return Err("Restart claim cancelled or replaced before dispatch".into());
        }
        app.chat
            .facade
            .push_priority_command(agent_chat_id, ChatCommand::Regenerate {})
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::{BoardCard, TaskBoard};
    use refact_runtime_api::ChatSessionFacade;

    struct RecoveryFacade {
        app: crate::app_state::AppState,
        inner: Arc<crate::tools::tool_spawn_ab::tests::RecordingFacade>,
        seeds: std::sync::Mutex<Vec<Vec<ChatMessage>>>,
        generations: std::sync::Mutex<Vec<u64>>,
    }

    #[async_trait]
    impl ChatSessionFacade for RecoveryFacade {
        async fn session_snapshot(
            &self,
            id: &str,
        ) -> Result<refact_runtime_api::ChatSessionSnapshot, String> {
            self.inner.session_snapshot(id).await
        }
        async fn update_session(
            &self,
            id: &str,
            update: refact_runtime_api::ChatSessionUpdate,
        ) -> Result<(), String> {
            self.inner.update_session(id, update).await
        }
        async fn create_session(&self, request: CreateSessionRequest) -> Result<(), String> {
            let execution = crate::tasks::admission::current_execution(&self.app, &request.chat_id)
                .map_err(|error| error.recovery_message())?;
            assert!(execution.active_operations() > 0);
            self.seeds.lock().unwrap().push(request.messages.clone());
            let mut session = crate::chat::types::ChatSession::new(request.chat_id.clone());
            session.thread = request.thread.clone();
            session.replace_messages(request.messages.clone());
            self.app
                .chat
                .sessions
                .write()
                .await
                .insert(request.chat_id.clone(), Arc::new(AMutex::new(session)));
            self.inner.create_session(request).await
        }
        async fn push_command(&self, id: &str, command: ChatCommand) -> Result<(), String> {
            let target = crate::agents::approval_target::ApprovalTarget::resolve(
                &self.app,
                &format!("board:{id}"),
            )
            .await?;
            self.generations.lock().unwrap().push(target.generation);
            self.inner.push_command(id, command).await
        }
        async fn session_state(
            &self,
            id: &str,
        ) -> Result<Option<refact_runtime_api::SessionState>, String> {
            self.inner.session_state(id).await
        }
        async fn maybe_save_session(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn save_trajectory_snapshot(
            &self,
            _: refact_runtime_api::RuntimeTrajectorySnapshot,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    async fn restart_response(ccx: Arc<AMutex<AtCommandsContext>>) -> Value {
        let (_, result) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            ToolTaskRestartAgent::new().tool_execute(
                ccx,
                &"restart-call".into(),
                &HashMap::from([
                    ("card_id".into(), json!("T-1")),
                    ("mode".into(), json!("resume")),
                ]),
            ),
        )
        .await
        .expect("restart returns without waiting for capacity")
        .unwrap();
        let ContextEnum::ChatMessage(message) = &result[0] else {
            panic!("tool result");
        };
        serde_json::from_str(&message.content.content_text_only()).unwrap()
    }

    async fn restart_refused(ccx: Arc<AMutex<AtCommandsContext>>, reason: &str) {
        let result = ToolTaskRestartAgent::new()
            .tool_execute(
                ccx,
                &"refused-restart".into(),
                &HashMap::from([
                    ("card_id".into(), json!("T-1")),
                    ("mode".into(), json!("resume")),
                    ("force".into(), json!(true)),
                ]),
            )
            .await;
        assert!(result.is_err());
        let error = result.err().unwrap();
        assert!(error.contains(reason), "{error}");
    }

    #[tokio::test]
    async fn restart_tool_queues_at_capacity_then_admits_without_losing_retained_edits() {
        use crate::tools::tool_spawn_ab::tests::{setup_repo_task, RecordingFacade, await_started};
        use crate::app_state::AppState;
        let (gcx, _temp, source) = setup_repo_task().await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "test/model".into(),
            Arc::new(crate::caps::ChatModelRecord::default()),
        );
        gcx.caps_state.write().await.caps = Some(Arc::new(caps));
        gcx.caps_state.write().await.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let meta = storage::load_task_meta(gcx.clone(), "task-1")
            .await
            .unwrap();
        let accepted_source = AcceptedAgentSource::capture(gcx.clone(), &source, None)
            .await
            .unwrap();
        let old = prepare_agent_worktree(
            gcx.clone(),
            &accepted_source,
            &meta,
            "task-1",
            "old-agent",
            "T-1",
            "old-chat",
        )
        .await
        .unwrap();
        let old_root = old.worktree_path();
        std::fs::write(old_root.join("retained.txt"), "preserve me").unwrap();
        let mut failed = failed_card(
            "T-1",
            Some(old_root.to_string_lossy().into_owned()),
            old.branch_name(),
        );
        failed.agent_worktree_name = Some(old.worktree_name());
        failed.agent_chat_id = Some("old-chat".into());
        failed.column = "doing".into();
        failed.completed_at = None;
        failed.final_report = None;
        let original = failed.clone();
        storage::update_board_atomic(gcx.clone(), "task-1", move |board| {
            board.owner_chat_id = Some("planner".into());
            *board.get_card_mut("T-1").unwrap() = failed;
            Ok(())
        })
        .await
        .unwrap();
        let mut app = AppState::from_gcx(gcx.clone()).await;
        app.work_scheduler = Arc::new(refact_work_core::AdmissionScheduler::new(
            refact_work_core::CoordinationSettings {
                max_active_engine: 1,
                max_active_project: 1,
                max_active_root: 1,
                ..Default::default()
            },
        ));
        let facade = Arc::new(RecordingFacade::default());
        let recorder = Arc::new(RecoveryFacade {
            app: app.clone(),
            inner: facade.clone(),
            seeds: Default::default(),
            generations: Default::default(),
        });
        app.chat.facade = recorder.clone();
        let mut previous = crate::chat::types::ChatSession::new("old-chat".into());
        previous.thread = build_agent_thread_params(
            "old-chat",
            "Previous",
            "test/model",
            "task-1",
            "old-agent",
            "T-1",
            "planner",
            old.worktree_meta(),
        );
        previous.install_goal(
            "agent",
            "Keep working within this finite lifetime",
            true,
            refact_chat_api::GoalBudget {
                max_turns: Some(10),
                max_tokens: Some(200_000),
                max_minutes: Some(15),
                no_progress_turns: Some(2),
                explicit: true,
                ..Default::default()
            },
        );
        assert!(previous.goal_record_progress(137, true));
        previous.set_runtime_state(
            refact_runtime_api::SessionState::WaitingUserInput,
            Some("Interrupted: needs_fresh_admission.".into()),
        );
        let finite_goal = previous.goal.clone().unwrap();
        assert_eq!(finite_goal.progress.turns_used, 1);
        let previous = Arc::new(AMutex::new(previous));
        app.chat
            .sessions
            .write()
            .await
            .insert("old-chat".into(), previous.clone());
        assert!(app.agent_executions.get("old-chat").is_none());
        assert!(
            crate::tasks::admission::Attempt::capture_for_app(&app, "old-chat")
                .unwrap()
                .is_none()
        );
        let parent = Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
            "planner".into(),
        )));
        {
            let mut session = parent.lock().await;
            session.work_project_root = Some(source.clone());
            session.thread.model = "test/model".into();
            session.thread.task_meta = Some(refact_chat_api::TaskMeta {
                task_id: "task-1".into(),
                role: "planner".into(),
                planner_chat_id: Some("planner".into()),
                agent_id: None,
                card_id: None,
            });
            session.thread.auto_approve_dangerous_commands = true;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert("planner".into(), parent.clone());
        let project = crate::coordination::roots::project_identity(&source);
        let holder = app
            .work_scheduler
            .reserve(refact_work_core::AdmissionRequest::new(
                refact_work_core::WorkKey::background_agent("holder").run(1),
                crate::tasks::admission::board_scope(&project, "planner"),
            ))
            .unwrap()
            .acquire()
            .await
            .unwrap();
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                "planner".into(),
                Some("planner".into()),
                "test/model".into(),
                Some(refact_chat_api::TaskMeta {
                    task_id: "task-1".into(),
                    role: "planner".into(),
                    planner_chat_id: Some("planner".into()),
                    agent_id: None,
                    card_id: None,
                }),
                None,
            )
            .await,
        ));
        let board_path = source.join(".refact/tasks/task-1/board.yaml");
        let before = std::fs::read(&board_path).unwrap();
        let mut held = original.clone();
        held.paused_at = Some("manual-hold".into());
        storage::update_board_atomic(gcx.clone(), "task-1", move |board| {
            *board.get_card_mut("T-1").unwrap() = held;
            Ok(())
        })
        .await
        .unwrap();
        let paused_board = std::fs::read(&board_path).unwrap();
        restart_refused(ccx.clone(), "Card is paused").await;
        assert_eq!(std::fs::read(&board_path).unwrap(), paused_board);
        storage::update_board_atomic(gcx.clone(), "task-1", {
            let original = original.clone();
            move |board| {
                *board.get_card_mut("T-1").unwrap() = original;
                Ok(())
            }
        })
        .await
        .unwrap();
        previous
            .lock()
            .await
            .goal_set_status(refact_chat_api::GoalStatus::Paused);
        restart_refused(ccx.clone(), "Previous goal is held").await;
        previous
            .lock()
            .await
            .goal_set_status(refact_chat_api::GoalStatus::Active);
        let pending = refact_work_core::PendingApproval {
            id: "old-consent".into(),
            child_work_id: "board:old-chat".into(),
            parent_chat_id: "planner".into(),
            run_generation: 777,
            tool_call_id: "dangerous-old-call".into(),
            tool_name: "shell".into(),
            args: json!({"command":"delete-sentinel"}),
            digest: None,
            created_at: 1,
            expires_at: u64::MAX,
            execution: None,
        };
        app.agent_approvals.register(pending.clone(), 1).unwrap();
        app.agent_approvals
            .decide("planner", 777, &pending.id, false, 1)
            .unwrap();
        restart_refused(ccx.clone(), "authorized reconciliation").await;
        assert_eq!(
            app.agent_approvals.get("planner", &pending.id, 1).unwrap(),
            pending
        );
        app.agent_approvals = refact_work_core::ApprovalRegistry::new(8);
        ccx.lock().await.app.agent_approvals = app.agent_approvals.clone();
        let old_messages = previous.lock().await.messages.clone();
        previous.lock().await.add_message(ChatMessage {
            role: "assistant".into(),
            tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                id: "unknown-side-effect".into(),
                index: None,
                function: crate::call_validation::ChatToolFunction {
                    name: "shell".into(),
                    arguments: "{}".into(),
                },
                tool_type: "function".into(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            }]),
            ..Default::default()
        });
        restart_refused(ccx.clone(), "unknown tool outcome").await;
        previous.lock().await.replace_messages(old_messages);
        previous
            .lock()
            .await
            .set_goal_projection(Some(finite_goal.clone()));
        parent.lock().await.work_project_root = Some(old_root.clone());
        restart_refused(ccx.clone(), "project binding changed").await;
        parent.lock().await.work_project_root = Some(source.clone());
        assert_eq!(app.work_scheduler.snapshot().outstanding, 1);
        assert!(facade.created.lock().unwrap().is_empty());
        assert!(!before.is_empty());
        let response = restart_response(ccx.clone()).await;
        assert_eq!(response["execution_state"], "queued");
        assert!(response["worktree"].is_null());
        assert!(facade.created.lock().unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(old_root.join("retained.txt")).unwrap(),
            "preserve me"
        );
        let queued = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        assert!(queued.get_card("T-1").unwrap().started_at.is_none());
        let cancelled_chat = response["agent_chat_id"].as_str().unwrap().to_string();
        let cancelled_attempt = crate::tasks::admission::current_attempt(&cancelled_chat).unwrap();
        cancelled_attempt
            .cancel_setup_and_wait(std::time::Duration::from_secs(5))
            .await
            .unwrap();
        cancelled_attempt.end();
        let restored = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        assert_eq!(
            restored.get_card("T-1").unwrap().agent_chat_id,
            original.agent_chat_id
        );
        assert_eq!(
            restored.get_card("T-1").unwrap().agent_worktree,
            original.agent_worktree
        );
        assert_eq!(app.work_scheduler.snapshot().outstanding, 1);
        assert!(facade.created.lock().unwrap().is_empty());
        let response = restart_response(ccx.clone()).await;
        assert_ne!(response["agent_chat_id"], cancelled_chat);
        cancelled_attempt.end();
        assert_eq!(app.work_scheduler.snapshot().outstanding, 2);
        holder.release();
        await_started(&facade, 1).await;
        assert_eq!(*facade.confirmation_flags.lock().unwrap(), vec![true]);
        let chat = response["agent_chat_id"].as_str().unwrap();
        assert_eq!(
            facade.started.lock().unwrap().as_slice(),
            &[chat.to_string()]
        );
        assert_eq!(app.work_scheduler.snapshot().active, 1);
        let thread = facade.threads.lock().unwrap()[0].clone();
        assert_eq!(thread.id, chat);
        let seeded = app.chat.sessions.read().await.get(chat).cloned().unwrap();
        let seeded_goal = seeded.lock().await.goal.clone().unwrap();
        assert_eq!(seeded_goal.budget, finite_goal.budget);
        assert_eq!(seeded_goal.progress, finite_goal.progress);
        assert_eq!(seeded_goal.status, refact_chat_api::GoalStatus::Active);
        let ledger = seeded.lock().await.goal_ledger.clone();
        let reduced = refact_chat_api::reduce_goal_ledger(&ledger).unwrap();
        let mut after_reload = seeded_goal.clone();
        reduced.apply_to_snapshot(&mut after_reload);
        assert_eq!(after_reload.budget, finite_goal.budget);
        assert_eq!(after_reload.progress, finite_goal.progress);
        let seeds = recorder.seeds.lock().unwrap().clone();
        assert!(seeds[0]
            .iter()
            .all(|message| message.tool_calls.is_none() && message.tool_call_id.is_empty()));
        assert_eq!(
            recorder.generations.lock().unwrap().as_slice(),
            &[app
                .agent_executions
                .get(chat)
                .unwrap()
                .admission_generation()]
        );
        let target =
            crate::agents::approval_target::ApprovalTarget::resolve(&app, &format!("board:{chat}"))
                .await
                .unwrap();
        assert!(target.check(&pending).is_err());
        assert_ne!(target.generation, pending.run_generation);
        assert_eq!(thread.worktree.as_ref().unwrap().root, old_root);
        assert_ne!(
            thread.task_meta.as_ref().unwrap().agent_id,
            thread.worktree.as_ref().unwrap().agent_id
        );
        assert_eq!(
            crate::chat::worktree_lifecycle::check_managed_attachment(&app, chat, &thread).await,
            Ok(())
        );
        assert_eq!(
            std::fs::read_to_string(old_root.join("retained.txt")).unwrap(),
            "preserve me"
        );
        crate::tasks::admission::cancel_setup_and_wait(chat, std::time::Duration::from_secs(5))
            .await
            .unwrap();
        crate::tasks::admission::end(chat).await;
        app.chat.sessions.write().await.clear();
        old.cleanup_unlinked(gcx).await;
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn dropped_restart_caller_during_claim_keeps_previous_worktree_and_binding() {
        use std::future::Future;
        use crate::tools::tool_spawn_ab::tests::setup_repo_task;
        let (gcx, _temp, source) = setup_repo_task().await;
        let original = failed_card("T-1", Some("/retained".into()), None);
        storage::update_board_atomic(gcx.clone(), "task-1", {
            let original = original.clone();
            move |board| {
                *board.get_card_mut("T-1").unwrap() = original;
                Ok(())
            }
        })
        .await
        .unwrap();
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let id = format!("restart-test-{}", Uuid::new_v4());
        crate::tasks::admission::reserve(&app, &id, "proj", "root", 0)
            .await
            .unwrap();
        let (accepted, acceptance) = tokio::sync::oneshot::channel::<Result<(), String>>();
        let (entered, entry) = tokio::sync::oneshot::channel();
        let task_id = id.clone();
        let task_gcx = gcx.clone();
        let old = original.clone();
        let original_board_path = source.join(".refact/tasks/task-1/board.yaml");
        let (foreign_gcx, _foreign_temp, foreign_source) = setup_repo_task().await;
        storage::update_board_atomic(foreign_gcx, "task-1", |board| {
            board.cards[0].instructions = "foreign sentinel remains populated".into();
            Ok(())
        })
        .await
        .unwrap();
        let foreign_path = foreign_source.join(".refact/tasks/task-1/board.yaml");
        let foreign_before = std::fs::read(&foreign_path).unwrap();
        assert!(!foreign_before.is_empty());
        let workspace_gcx = gcx.clone();
        let source = AcceptedAgentSource::capture(gcx.clone(), &source, None)
            .await
            .unwrap()
            .bind_task(gcx.clone(), "task-1")
            .await
            .unwrap();
        let source = source
            .with_attempt("task-1", &original, "new-agent", &id, None)
            .unwrap();
        let binding = source.task_binding("task-1").unwrap().clone();
        let lock =
            crate::tasks::board_lock::acquire_board(&source.root().join(".refact/tasks/task-1"))
                .await
                .unwrap();
        crate::tasks::admission::spawn_tracked(&id, async move {
            binding
                .scope(async move {
                    let mut claim = Box::pin(claim_restart(
                        task_gcx.clone(),
                        "task-1",
                        &old,
                        "new-agent",
                        &task_id,
                        "resume",
                    ));
                    std::future::poll_fn(|cx| {
                        assert!(claim.as_mut().poll(cx).is_pending());
                        std::task::Poll::Ready(())
                    })
                    .await;
                    entered.send(()).unwrap();
                    match claim.await {
                        Ok(queued) => {
                            assert_eq!(queued.agent_chat_id.as_deref(), Some(task_id.as_str()));
                            assert_eq!(queued.column, "doing");
                            *workspace_gcx
                                .documents_state
                                .workspace_folders
                                .lock()
                                .unwrap() = vec![foreign_source];
                            if accepted.send(Ok(())).is_err() {
                                restore_card_after_restart_failure(
                                    task_gcx,
                                    &source,
                                    "task-1",
                                    old,
                                    task_id.clone(),
                                    None,
                                )
                                .await;
                            }
                        }
                        Err(error) => {
                            let _ = accepted.send(Err(error));
                        }
                    }
                    crate::tasks::admission::end(&task_id).await;
                })
                .await
        })
        .await
        .unwrap();
        entry.await.unwrap();
        drop(acceptance);
        drop(lock);
        crate::tasks::admission::shutdown(&app).await;
        let board: TaskBoard =
            serde_yaml::from_slice(&std::fs::read(&original_board_path).unwrap()).unwrap();
        assert_eq!(std::fs::read(&foreign_path).unwrap(), foreign_before);
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.agent_chat_id, original.agent_chat_id);
        assert_eq!(card.agent_worktree, original.agent_worktree);
        assert_eq!(card.column, "failed");
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn restart_claim_is_conditional_and_preserves_old_checkout_while_queued() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![temp.path().canonicalize().unwrap()];
        let mut planner = crate::chat::types::ChatSession::new("planner".into());
        planner.thread.model = "provider/model".into();
        planner.work_project_root = Some(temp.path().canonicalize().unwrap());
        planner.thread.task_meta = Some(refact_chat_api::TaskMeta {
            task_id: "restart-claim".into(),
            role: "planner".into(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some("planner".into()),
        });
        gcx.chat_sessions
            .write()
            .await
            .insert("planner".into(), Arc::new(tokio::sync::Mutex::new(planner)));
        let task_dir = temp.path().join(".refact/tasks/restart-claim");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let mut original = failed_card(
            "T-1",
            Some("/retained/worktree".into()),
            Some("retained-branch".into()),
        );
        original.agent_worktree_name = Some("retained-id".into());
        let board = TaskBoard {
            owner_chat_id: Some("planner".into()),
            cards: vec![original.clone()],
            ..Default::default()
        };
        storage::save_board(gcx.clone(), "restart-claim", &board)
            .await
            .unwrap();
        let mut held = original.clone();
        held.paused_at = Some("pause-before-claim".into());
        let mut paused_board = board.clone();
        *paused_board.get_card_mut("T-1").unwrap() = held.clone();
        storage::save_board(gcx.clone(), "restart-claim", &paused_board)
            .await
            .unwrap();
        let paused_bytes = std::fs::read(task_dir.join("board.yaml")).unwrap();
        let error = crate::file_activity::TOOL_ACTOR
            .scope(
                "planner".into(),
                claim_restart(
                    gcx.clone(),
                    "restart-claim",
                    &held,
                    "new-agent",
                    "new-chat",
                    "resume",
                ),
            )
            .await
            .unwrap_err();
        assert!(error.contains("paused"));
        assert_eq!(
            std::fs::read(task_dir.join("board.yaml")).unwrap(),
            paused_bytes
        );
        storage::update_board_atomic(gcx.clone(), "restart-claim", {
            let original = original.clone();
            move |board| {
                *board.get_card_mut("T-1").unwrap() = original;
                Ok(())
            }
        })
        .await
        .unwrap();
        let queued = crate::file_activity::TOOL_ACTOR
            .scope(
                "planner".into(),
                claim_restart(
                    gcx.clone(),
                    "restart-claim",
                    &original,
                    "new-agent",
                    "new-chat",
                    "fresh",
                ),
            )
            .await
            .unwrap();
        assert_eq!(queued.column, "doing");
        assert_eq!(queued.agent_chat_id.as_deref(), Some("new-chat"));
        assert!(queued.started_at.is_none());
        assert_eq!(queued.agent_worktree, original.agent_worktree);
        assert_eq!(queued.final_report, original.final_report);
        let error = crate::file_activity::TOOL_ACTOR
            .scope(
                "planner".into(),
                claim_restart(
                    gcx.clone(),
                    "restart-claim",
                    &original,
                    "duplicate",
                    "duplicate-chat",
                    "fresh",
                ),
            )
            .await
            .unwrap_err();
        assert!(error.contains("changed while restart"));
        let current = storage::load_board(gcx, "restart-claim").await.unwrap();
        assert_eq!(
            current.get_card("T-1").unwrap().agent_chat_id,
            queued.agent_chat_id
        );
    }

    #[tokio::test]
    async fn restart_agent_context_file_message_has_declared_record() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("safe.txt");
        std::fs::write(&path, "safe\n").unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let message = context_file_message(
            &gcx,
            vec![ContextFile {
                file_name: path.to_string_lossy().into_owned(),
                file_content: "safe\n".to_string(),
                line1: 1,
                line2: 1,
                ..Default::default()
            }],
        )
        .unwrap();
        let privacy: refact_privacy::PrivacyRecord =
            serde_json::from_value(message.extra["privacy"].clone()).unwrap();

        assert_eq!(privacy.files.len(), 1);
        assert!(privacy.files[0].path.ends_with("safe.txt"));
        assert_eq!(
            privacy.files[0].attribution,
            refact_privacy::Attribution::Declared
        );
    }

    fn failed_card(id: &str, worktree: Option<String>, branch: Option<String>) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: format!("Card {}", id),
            column: "failed".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: "Fix the bug".to_string(),
            assignee: Some("old-agent".to_string()),
            agent_chat_id: Some(format!("agent-{}-old", id)),
            status_updates: vec![StatusUpdate {
                timestamp: "2024-01-01T10:00:00Z".to_string(),
                message: "Agent started".to_string(),
            }],
            comments: vec![],
            final_report: Some("FAILED: network error after 3 retries".to_string()),
            final_report_structured: None,
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: Some(Utc::now().to_rfc3339()),
            last_heartbeat_at: None,
            paused_at: None,
            paused_by: None,
            completed_at: Some(Utc::now().to_rfc3339()),
            agent_branch: branch,
            agent_worktree: worktree,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
            attention: None,
            deliverable: None,
            verification: Vec::new(),
        }
    }

    #[test]
    fn restart_agent_fresh_clears_failure_and_sets_doing() {
        let mut card = failed_card("T-1", None, None);
        let new_id = "new-agent-1";
        let new_chat = "agent-T-1-new";

        mark_card_restarted_fresh(
            &mut card,
            new_id,
            new_chat,
            Some("refact/task/task-1/card/T-1/new".to_string()),
            Some("/tmp/wt".to_string()),
            Some("wt-new".to_string()),
        );

        assert_eq!(card.column, "doing");
        assert_eq!(card.assignee.as_deref(), Some(new_id));
        assert_eq!(card.agent_chat_id.as_deref(), Some(new_chat));
        assert!(card.completed_at.is_none());
        assert!(card.final_report.is_none());
        assert!(card
            .status_updates
            .iter()
            .any(|u| u.message.contains("Previous failure")));
        assert_eq!(card.agent_worktree.as_deref(), Some("/tmp/wt"));
        assert_eq!(card.agent_worktree_name.as_deref(), Some("wt-new"));
    }

    #[test]
    fn restart_agent_resume_keeps_worktree_fields() {
        let mut card = failed_card(
            "T-2",
            Some("/tmp/retained-wt".to_string()),
            Some("refact/task/task-1/card/T-2/old".to_string()),
        );
        let old_worktree = card.agent_worktree.clone();
        let old_branch = card.agent_branch.clone();
        let new_id = "resume-agent-1";
        let new_chat = "agent-T-2-resume";

        mark_card_restarted_resume(&mut card, new_id, new_chat);

        assert_eq!(card.column, "doing");
        assert_eq!(card.assignee.as_deref(), Some(new_id));
        assert_eq!(card.agent_chat_id.as_deref(), Some(new_chat));
        assert!(card.completed_at.is_none());
        assert!(card.final_report.is_none());
        assert!(card
            .status_updates
            .iter()
            .any(|u| u.message.contains("Previous failure")));
        assert!(card
            .status_updates
            .iter()
            .any(|u| u.message.contains("Resume restart")));
        assert_eq!(card.agent_worktree, old_worktree, "worktree path preserved");
        assert_eq!(card.agent_branch, old_branch, "branch preserved");
    }

    #[test]
    fn restart_agent_fresh_rollback_resets_card_to_failed() {
        let original = failed_card(
            "T-7",
            Some("/tmp/old-wt".to_string()),
            Some("old-branch".to_string()),
        );
        let mut board = TaskBoard::default();
        let mut restarted = original.clone();
        mark_card_restarted_fresh(
            &mut restarted,
            "new-agent",
            "agent-T-7-new",
            Some("new-branch".to_string()),
            Some("/tmp/new-wt".to_string()),
            Some("new-wt".to_string()),
        );
        board.cards.push(restarted);

        assert!(restore_original_card_if_current_agent(
            &mut board,
            &original,
            "agent-T-7-new"
        ));

        let card = board.get_card("T-7").unwrap();
        assert_eq!(card.column, original.column);
        assert_eq!(card.agent_chat_id, original.agent_chat_id);
        assert_eq!(card.assignee, original.assignee);
        assert_eq!(card.agent_worktree, original.agent_worktree);
        assert_eq!(card.agent_branch, original.agent_branch);
        assert_eq!(card.final_report, original.final_report);
        assert_eq!(card.completed_at, original.completed_at);
    }

    #[test]
    fn restart_agent_rollback_guard_keeps_newer_agent() {
        let original = failed_card("T-8", None, None);
        let mut board = TaskBoard::default();
        let mut newer = original.clone();
        mark_card_restarted_fresh(
            &mut newer,
            "newer-agent",
            "agent-T-8-newer",
            None,
            None,
            None,
        );
        board.cards.push(newer.clone());

        assert!(!restore_original_card_if_current_agent(
            &mut board,
            &original,
            "agent-T-8-stale"
        ));

        let card = board.get_card("T-8").unwrap();
        assert_eq!(card.agent_chat_id, newer.agent_chat_id);
        assert_eq!(card.assignee, newer.assignee);
        assert_eq!(card.column, "doing");
    }

    #[test]
    fn restart_agent_source_guard_rejects_stale_previous_binding() {
        let original = failed_card("T-10", Some("/tmp/original-wt".to_string()), None);
        let mut changed = original.clone();
        changed.agent_chat_id = Some("agent-T-10-newer".to_string());
        changed.assignee = Some("newer-agent".to_string());

        let err = ensure_restart_source_current(&changed, &original).unwrap_err();

        assert!(err.contains("changed while restart was preparing"), "{err}");
        assert!(err.contains("agent-T-10-newer"), "{err}");
    }
}
