use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use tokio::sync::Mutex as AMutex;
use tracing::{info, warn};

use crate::agents::types::AgentListFilter;
use crate::app_state::AppState;
use crate::exec::{ExecMode, ExecProcessFilter, ExecProcessSnapshot, ExecStatusKind};
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{HookPayload, get_project_dir_string, run_observation_hooks};

use super::session::{SessionsMap, should_replace_background_agent};
use super::types::*;
use super::types::session_cleanup_interval;
use super::trajectories::first_system_prompt;

fn background_process_cleanup_modes(include_services: bool) -> Vec<ExecMode> {
    if include_services {
        vec![ExecMode::Background, ExecMode::Service]
    } else {
        vec![ExecMode::Background]
    }
}

pub async fn clean_background_processes_for_chat(
    app: AppState,
    chat_id: &str,
    include_services: bool,
) -> Result<Vec<ExecProcessSnapshot>, String> {
    let mut killed = Vec::new();
    for mode in background_process_cleanup_modes(include_services) {
        for status in [ExecStatusKind::Starting, ExecStatusKind::Running] {
            killed.extend(
                app.runtime
                    .exec_registry
                    .remove_by_owner(ExecProcessFilter {
                        chat_id: Some(chat_id.to_string()),
                        mode: Some(mode.clone()),
                        status: Some(status),
                        ..ExecProcessFilter::default()
                    })
                    .await?,
            );
        }
    }
    killed.sort_by(|a, b| a.meta.process_id.as_str().cmp(b.meta.process_id.as_str()));
    Ok(killed)
}

pub fn snapshot_with_agents(
    app: AppState,
    session: &ChatSession,
) -> impl std::future::Future<Output = (ChatEvent, Vec<BackgroundAgentSummary>)> + Send + 'static {
    snapshot_with_agents_mode(app, session, true)
}

pub fn snapshot_with_agents_mode(
    app: AppState,
    session: &ChatSession,
    include_messages: bool,
) -> impl std::future::Future<Output = (ChatEvent, Vec<BackgroundAgentSummary>)> + Send + 'static {
    let snapshot = if include_messages {
        session.snapshot()
    } else {
        session.snapshot_without_messages()
    };
    snapshot_event_with_agents(app, session, snapshot)
}

fn snapshot_event_with_agents(
    app: AppState,
    session: &ChatSession,
    mut snapshot: ChatEvent,
) -> impl std::future::Future<Output = (ChatEvent, Vec<BackgroundAgentSummary>)> + Send + 'static {
    let chat_id = session.chat_id.clone();
    let base_background_agents: HashMap<String, BackgroundAgentSummary> =
        session.background_agents.clone();
    async move {
        let browser =
            crate::integrations::browser_runtime::browser_snapshot_for_chat(app.clone(), &chat_id)
                .await;
        let mut background_agents = base_background_agents;
        let agents = app
            .agents
            .list_for_parent(&chat_id, AgentListFilter::default())
            .await;
        for agent in agents.iter().map(BackgroundAgentSummary::from) {
            if should_replace_background_agent(background_agents.get(&agent.agent_id), &agent) {
                background_agents.insert(agent.agent_id.clone(), agent);
            }
        }
        let mut background_agents: Vec<_> = background_agents.into_values().collect();
        background_agents.sort_by(|a, b| {
            b.change_seq
                .cmp(&a.change_seq)
                .then(a.agent_id.cmp(&b.agent_id))
        });
        if let ChatEvent::Snapshot {
            background_agents: snapshot_background_agents,
            browser: snapshot_browser,
            ..
        } = &mut snapshot
        {
            *snapshot_background_agents = background_agents.clone();
            *snapshot_browser = browser;
        }
        (snapshot, background_agents)
    }
}

async fn migrate_legacy_frozen_prefix_on_open(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) {
    let installed = {
        let mut session = session_arc.lock().await;
        let prefix = session.thread.frozen_request_prefix.as_ref();
        let needs_prefix = prefix.is_none();
        let needs_legacy_save = prefix
            .map(|prefix| prefix.tools_canonical.is_none() && prefix.system_prompt.is_some())
            .unwrap_or(false);
        if needs_prefix {
            let system_prompt = first_system_prompt(&session.messages);
            super::trajectories::ensure_frozen_prefix(&mut session, system_prompt, None).is_some()
        } else if needs_legacy_save {
            session.increment_version();
            true
        } else {
            false
        }
    };
    if installed {
        super::trajectories::maybe_save_trajectory_with_intent(
            app,
            session_arc,
            TrajectoryCommitIntent::Required,
        )
        .await;
    }
}

async fn compact_reconstruction_privacy_on_open(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) {
    let compacted = {
        let mut session = session_arc.lock().await;
        let compacted =
            crate::privacy::records::compact_stored_reconstruction_privacy(&mut session.messages);
        if compacted {
            info!(
                "Compacted reconstruction privacy records for chat {}",
                session.chat_id
            );
            session.increment_version();
        }
        compacted
    };
    if compacted {
        super::trajectories::maybe_save_trajectory_with_intent(
            app,
            session_arc,
            TrajectoryCommitIntent::Required,
        )
        .await;
    }
}

async fn reconcile_loaded_card_pause(app: &AppState, session_arc: &Arc<AMutex<ChatSession>>) {
    let (meta, revision) = {
        let session = session_arc.lock().await;
        let Some(meta) = session
            .thread
            .task_meta
            .clone()
            .filter(|meta| meta.role == "agents")
        else {
            return;
        };
        (meta, session.planner_pause_revision)
    };
    let Some(card_id) = meta.card_id.as_deref() else {
        return;
    };
    let board = match crate::tasks::storage::load_board(app.gcx.clone(), &meta.task_id).await {
        Ok(board) => board,
        Err(error) => {
            warn!(
                "Failed to reconcile planner pause for {} card {card_id}: {error}",
                meta.task_id
            );
            return;
        }
    };
    let pause = board
        .get_card(card_id)
        .filter(|card| card.column == "doing")
        .and_then(|card| {
            card.paused_at
                .as_ref()
                .map(|since| (since.clone(), card.paused_by.clone()))
        });
    #[cfg(test)]
    super::planner_pause_load_race_tests::after_reconcile_board_read().await;

    let mut session = session_arc.lock().await;
    if session.planner_pause_revision == revision
        && session.update_card_planner_pause(&meta.task_id, card_id, pause)
    {
        session.planner_pause_at_boundary();
    }
}

pub(crate) async fn board_execution(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
) -> Result<
    Option<Arc<crate::agents::execution::RunExecution>>,
    crate::tasks::admission::ExecutionUnavailable,
> {
    use crate::tasks::admission::{current_execution, ExecutionUnavailable};

    let (chat_id, thread, source, closed, restored_pending_tools) = {
        let session = session_arc.lock().await;
        (
            session.chat_id.clone(),
            session.thread.clone(),
            crate::coordination::scope::session_source_root(&session),
            session.closed,
            session.board_restored_pending_tools,
        )
    };
    let Some(meta) = thread
        .task_meta
        .clone()
        .filter(|meta| matches!(meta.role.as_str(), "agent" | "agents"))
    else {
        return Ok(None);
    };
    if let Some(agent_id) = app.agents.find_agent_id_by_child_chat_id(&chat_id).await {
        if app
            .agent_executions
            .saved_config(&agent_id)
            .is_some_and(|config| {
                config.stateful
                    && config.chat_id.as_deref() == Some(chat_id.as_str())
                    && config.parent_id == thread.parent_id
                    && config.root_chat_id == thread.root_chat_id
                    && config.task_meta == thread.task_meta
            })
        {
            return Ok(None);
        }
        return Err(ExecutionUnavailable::StaleBinding);
    }
    let execution = current_execution(app, &chat_id)?;
    if restored_pending_tools {
        return Err(ExecutionUnavailable::PendingToolReconciliation);
    }
    let validate = async {
        if closed || meta.task_id.is_empty() {
            return Err(ExecutionUnavailable::StaleBinding);
        }
        let source = source.as_ref().ok_or(ExecutionUnavailable::StaleBinding)?;
        let source = crate::coordination::roots::resolve_source_workspace_root(
            app.gcx.clone(),
            source.to_str(),
        )
        .await
        .map_err(|_| ExecutionUnavailable::StaleBinding)?;
        let (dir, guard) = crate::tasks::storage::locked_task_dir(app.gcx.clone(), &meta.task_id)
            .await
            .map_err(|_| ExecutionUnavailable::StaleBinding)?;
        let belongs = [
            source.join(".refact/tasks").join(&meta.task_id),
            app.gcx.config_dir.join("tasks").join(&meta.task_id),
        ]
        .iter()
        .any(|path| dunce::canonicalize(path).ok().as_ref() == Some(&dir));
        if !belongs {
            return Err(ExecutionUnavailable::StaleBinding);
        }
        let board =
            crate::tasks::storage::load_board_locked(app.gcx.clone(), &meta.task_id, &guard)
                .await
                .map_err(|_| ExecutionUnavailable::StaleBinding)?;
        let planner = meta
            .planner_chat_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or(ExecutionUnavailable::StaleBinding)?;
        if board.owner_chat_id.as_deref() != Some(planner) {
            return Err(ExecutionUnavailable::StaleBinding);
        }
        let card = meta
            .card_id
            .as_deref()
            .and_then(|id| board.get_card(id))
            .filter(|card| card.column == "doing")
            .ok_or(ExecutionUnavailable::StaleBinding)?;
        let agent = meta
            .agent_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or(ExecutionUnavailable::StaleBinding)?;
        let bound = if let Some(variants) = &card.ab_variants {
            variants.winner.is_none()
                && [&variants.a, &variants.b].iter().any(|variant| {
                    variant.chat_id == chat_id
                        && variant.agent_id == agent
                        && variant.finish.is_none()
                })
        } else if !card.team_members.is_empty() {
            card.team_members
                .iter()
                .any(|member| member.agent_chat_id.as_deref() == Some(chat_id.as_str()))
        } else {
            card.agent_chat_id.as_deref() == Some(chat_id.as_str())
                && card.assignee.as_deref() == Some(agent)
        };
        if !bound
            || (card.ab_variants.is_none()
                && card.team_members.is_empty()
                && board
                    .card_claim(&card.id)
                    .is_some_and(|claim| claim.claimant_chat_id != chat_id))
        {
            return Err(ExecutionUnavailable::StaleBinding);
        }
        drop(guard);
        Ok(())
    }
    .await;
    validate?;
    let registered = app.chat.sessions.read().await.get(&chat_id).cloned();
    if !registered
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, session_arc))
    {
        return Err(ExecutionUnavailable::StaleBinding);
    }
    let session = session_arc.lock().await;
    if session.closed
        || session.chat_id != chat_id
        || session.thread.task_meta.as_ref() != Some(&meta)
        || crate::coordination::scope::session_source_root(&session) != source
    {
        return Err(ExecutionUnavailable::StaleBinding);
    }
    let current = current_execution(app, &chat_id)?;
    if !Arc::ptr_eq(&execution, &current) {
        return Err(ExecutionUnavailable::StaleBinding);
    }
    Ok(Some(execution))
}

pub(crate) fn interrupt_board_execution(
    session: &mut ChatSession,
    reason: crate::tasks::admission::ExecutionUnavailable,
) {
    if reason == crate::tasks::admission::ExecutionUnavailable::Held {
        return;
    }
    let state = if session.runtime.paused || !session.runtime.pause_reasons.is_empty() {
        SessionState::Paused
    } else {
        SessionState::WaitingUserInput
    };
    session.stream_started_at = None;
    session.set_runtime_state(state, Some(reason.recovery_message()));
}

pub(crate) fn board_execution_interrupted(session: &ChatSession) -> bool {
    matches!(
        session.runtime.state,
        SessionState::Paused | SessionState::WaitingUserInput
    ) && session
        .runtime
        .error
        .as_deref()
        .is_some_and(|error| error.starts_with("Interrupted: "))
}

pub(crate) struct BoardOperation {
    pub(crate) execution: Arc<crate::agents::execution::RunExecution>,
    _operation: crate::agents::execution::OperationGuard,
}

pub(crate) async fn begin_board_operation(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
) -> Result<Option<BoardOperation>, crate::tasks::admission::ExecutionUnavailable> {
    begin_board_operation_inner(app, session_arc, false).await
}

pub(crate) async fn begin_board_decision_operation(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
) -> Result<Option<BoardOperation>, crate::tasks::admission::ExecutionUnavailable> {
    begin_board_operation_inner(app, session_arc, true).await
}

async fn begin_board_operation_inner(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    confirmation: bool,
) -> Result<Option<BoardOperation>, crate::tasks::admission::ExecutionUnavailable> {
    use crate::tasks::admission::ExecutionUnavailable;

    let result = async {
        let Some(execution) = board_execution(app, session_arc).await? else {
            return Ok(None);
        };
        let abort = {
            let session = session_arc.lock().await;
            if session.planner_paused() || (!confirmation && (session.runtime.paused || !session.runtime.pause_reasons.is_empty())) {
                return Err(ExecutionUnavailable::Held);
            }
            session.abort_flag.clone()
        };
        let acquisition = execution.begin_operation();
        tokio::pin!(acquisition);
        let operation = loop {
            if abort.load(Ordering::SeqCst) || app.runtime.shutdown_flag.load(Ordering::Relaxed) {
                return Err(ExecutionUnavailable::Cancelled);
            }
            tokio::select! {
                result = &mut acquisition => break result.map_err(|_| ExecutionUnavailable::Cancelled)?,
                _ = tokio::time::sleep(refact_work_core::coordination_settings().lock_poll_interval()) => {}
            }
        };
        let current = board_execution(app, session_arc).await?
            .ok_or(ExecutionUnavailable::StaleBinding)?;
        if !Arc::ptr_eq(&execution, &current) {
            return Err(ExecutionUnavailable::StaleBinding);
        }
        Ok(Some(BoardOperation { execution: execution.clone(), _operation: operation }))
    }.await;
    if let Err(reason) = result.as_ref() {
        interrupt_board_execution(&mut *session_arc.lock().await, *reason);
    }
    result
}

pub async fn get_or_create_session_with_trajectory(
    app: AppState,
    sessions: &SessionsMap,
    chat_id: &str,
) -> Arc<AMutex<ChatSession>> {
    let gcx = app.gcx.clone();
    let maybe_existing = {
        let sessions_read = sessions.read().await;
        sessions_read.get(chat_id).cloned()
    };

    if let Some(session_arc) = maybe_existing {
        let is_closed = {
            let session = session_arc.lock().await;
            session.closed
        };
        if !is_closed {
            super::trajectories::refresh_session_from_trajectory_if_stale(
                app.clone(),
                chat_id,
                false,
            )
            .await;
            return session_arc;
        }
        let mut sessions_write = sessions.write().await;
        if let Some(current) = sessions_write.get(chat_id) {
            if Arc::ptr_eq(current, &session_arc) {
                sessions_write.remove(chat_id);
            }
        }
    }

    let trajectory_events_tx = app.chat.trajectory_events_tx.clone();

    let (mut session, is_new, transition_repair_patch) = if let Some(mut loaded) =
        super::trajectories::load_trajectory_for_chat(gcx.clone(), chat_id).await
    {
        info!(
            "Loaded trajectory for chat {} with {} messages",
            chat_id,
            loaded.messages.len()
        );
        super::trajectories::apply_mode_defaults_to_thread(
            gcx.clone(),
            &mut loaded.thread,
            loaded.auto_approve_editing_tools_present,
            loaded.auto_approve_dangerous_commands_present,
        )
        .await;
        let transition_identity_repaired = loaded.transition_identity_repaired;
        let transition_repair_patch = transition_identity_repaired.then(|| loaded.repair_patch());
        let work_project_root =
            crate::coordination::scope::restored_source_root(&app, &loaded.source_path).await;
        let mut session = ChatSession::new_with_trajectory(
            chat_id.to_string(),
            loaded.messages,
            loaded.thread,
            loaded.created_at,
            loaded.wake_up_at,
            loaded.waiting_for_card_ids,
            loaded.goal,
        );
        session.work_project_root = work_project_root;
        let dropped_deliveries = loaded.dropped_deliveries.clone();
        if let Some(agent_id) = app.agents.find_agent_id_by_child_chat_id(chat_id).await {
            if app.agents.has_runtime(&agent_id).await {
                session.restore_pending_deliveries(Vec::new());
                session.runner_pending_deliveries = app.agents.pending_deliveries(&agent_id).await;
            } else {
                session.restore_pending_deliveries(loaded.pending_deliveries);
            }
        } else {
            session.restore_pending_deliveries(loaded.pending_deliveries);
        }
        session.merge_dropped_deliveries(dropped_deliveries);
        session.runtime.recovery = loaded.recovery;
        session.set_goal_ledger(loaded.goal_ledger);
        session.pending_questions = loaded.pending_questions;
        if let Some(pending) = loaded.pending_browser_decision {
            super::browser_decisions::expire_restored_browser_decision(&mut session, pending);
        }
        session.goal_verification_blocked_until_ms = loaded.goal_verification_blocked_until_ms;
        if session
            .thread
            .task_meta
            .as_ref()
            .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents"))
            && app
                .agents
                .find_agent_id_by_child_chat_id(chat_id)
                .await
                .is_none()
            && crate::tasks::admission::current_execution(&app, chat_id).is_err()
        {
            session.board_restored_pending_tools = session.has_pending_tool_result_window();
        }
        if transition_identity_repaired {
            session.increment_version();
        }
        (session, false, transition_repair_patch)
    } else {
        let mut s = ChatSession::new(chat_id.to_string());
        s.work_project_root =
            crate::coordination::roots::resolve_source_workspace_root(gcx.clone(), None)
                .await
                .ok();
        s.increment_version();
        s.trajectory_dirty = false;
        (s, true, None)
    };

    let background_agents = app
        .agents
        .list_for_parent(chat_id, AgentListFilter::default())
        .await;
    session.upsert_background_agents(background_agents.iter().map(BackgroundAgentSummary::from));

    if is_new {
        session.thread.auto_enrichment_enabled = Some(true);
        if let Some(mode_config) = crate::yaml_configs::customization_registry::get_mode_config(
            gcx.clone(),
            &session.thread.mode,
            None,
        )
        .await
        {
            let defaults = &mode_config.thread_defaults;
            if let Some(v) = defaults.include_project_info {
                session.thread.include_project_info = v;
            }
            if let Some(v) = defaults.checkpoints_enabled {
                session.thread.checkpoints_enabled = v;
            }
            if let Some(v) = defaults.auto_approve_editing_tools {
                session.thread.auto_approve_editing_tools = v;
            }
            if let Some(v) = defaults.auto_approve_dangerous_commands {
                session.thread.auto_approve_dangerous_commands = v;
            }
        }
    }

    if let Some(meta) = session
        .thread
        .task_meta
        .clone()
        .filter(|meta| meta.role == "agents")
    {
        if let Some(card_id) = meta.card_id.as_deref() {
            match crate::tasks::storage::load_board(gcx.clone(), &meta.task_id).await {
                Ok(board) => {
                    let pause = board
                        .get_card(card_id)
                        .filter(|card| card.column == "doing")
                        .and_then(|card| {
                            card.paused_at
                                .as_ref()
                                .map(|since| (since.clone(), card.paused_by.clone()))
                        });
                    session.update_card_planner_pause(&meta.task_id, card_id, pause);
                    session.planner_pause_at_boundary();
                }
                Err(error) => warn!("Failed to restore planner pause for {chat_id}: {error}"),
            }
        }
    }
    session.trajectory_events_tx = Some(trajectory_events_tx.clone());

    #[cfg(test)]
    super::planner_pause_load_race_tests::before_session_insert(&session).await;

    let (session_arc, inserted) = {
        let mut sessions_write = sessions.write().await;
        match sessions_write.entry(chat_id.to_string()) {
            std::collections::hash_map::Entry::Vacant(e) => {
                let arc = Arc::new(AMutex::new(session));
                e.insert(arc.clone());
                (arc, true)
            }
            std::collections::hash_map::Entry::Occupied(e) => (e.get().clone(), false),
        }
    };

    if inserted {
        reconcile_loaded_card_pause(&app, &session_arc).await;
        if !is_new {
            if let Err(reason) = board_execution(&app, &session_arc).await {
                interrupt_board_execution(&mut *session_arc.lock().await, reason);
                return session_arc;
            }
        }
    }

    if inserted && is_new {
        let app_hook = AppState::from_gcx(gcx.clone()).await;
        let chat_id_clone = chat_id.to_string();
        tokio::spawn(async move {
            let project_dir = get_project_dir_string(app_hook.clone()).await;
            let payload = HookPayload::new(&HookEvent::SessionStart, chat_id_clone, project_dir);
            run_observation_hooks(app_hook, HookEvent::SessionStart, payload).await;
        });
    }

    if inserted && !is_new {
        if transition_repair_patch.is_none() {
            migrate_legacy_frozen_prefix_on_open(app.clone(), session_arc.clone()).await;
        }
        if let Some(repair_patch) = transition_repair_patch {
            let repaired_version = {
                let session = session_arc.lock().await;
                session.trajectory_version
            };
            if let Err(e) = super::trajectories::persist_loaded_trajectory_repair_raw(
                gcx.clone(),
                &repair_patch,
            )
            .await
            {
                warn!(
                    "Failed to persist repaired trajectory for {}: {}",
                    chat_id, e
                );
            } else {
                let mut session = session_arc.lock().await;
                session.complete_trajectory_commit(repaired_version);
            }
        }
        let version = session_arc.lock().await.trajectory_version;
        match crate::tasks::goal::hydrate_session_task_goal(gcx.clone(), &session_arc).await {
            Ok(()) if session_arc.lock().await.trajectory_version != version => {
                super::trajectories::maybe_save_trajectory_with_intent(
                    app.clone(),
                    session_arc.clone(),
                    TrajectoryCommitIntent::Required,
                )
                .await;
            }
            Ok(()) => {}
            Err(error) => warn!(%chat_id, %error, "Task goal hydration on open refused"),
        }
        compact_reconstruction_privacy_on_open(app.clone(), session_arc.clone()).await;
    }

    session_arc
}

pub async fn try_restore_session_if_trajectory_exists(
    app: AppState,
    sessions: &SessionsMap,
    chat_id: &str,
) -> bool {
    let maybe_existing = {
        let sessions_read = sessions.read().await;
        sessions_read.get(chat_id).cloned()
    };

    if let Some(session_arc) = maybe_existing {
        let is_closed = session_arc.lock().await.closed;
        if !is_closed {
            return true;
        }
        let mut sessions_write = sessions.write().await;
        if let Some(current) = sessions_write.get(chat_id) {
            if Arc::ptr_eq(current, &session_arc) {
                sessions_write.remove(chat_id);
            }
        }
    }

    if super::trajectories::load_trajectory_for_chat(app.gcx.clone(), chat_id)
        .await
        .is_none()
    {
        return false;
    }

    get_or_create_session_with_trajectory(app, sessions, chat_id).await;
    true
}

pub async fn close_all_chat_sessions(app: AppState) {
    let sessions = app.chat.sessions.clone();
    let session_arcs: Vec<Arc<AMutex<ChatSession>>> = {
        let sessions_read = sessions.read().await;
        sessions_read.values().cloned().collect()
    };
    let flushes = futures::future::join_all(session_arcs.iter().cloned().map(|session_arc| {
        super::trajectories::flush_trajectory_for_session(app.clone(), session_arc)
    }));
    match tokio::time::timeout(std::time::Duration::from_secs(5), flushes).await {
        Ok(results) => {
            for error in results.into_iter().filter_map(Result::err) {
                warn!(
                    "close_all_chat_sessions: trajectory flush failed: {}",
                    error
                );
            }
        }
        Err(_) => warn!("close_all_chat_sessions: trajectory flush timed out"),
    }
    for session_arc in session_arcs {
        let lock_result =
            tokio::time::timeout(std::time::Duration::from_millis(500), session_arc.lock()).await;
        match lock_result {
            Ok(mut session) => {
                session.abort_stream();
                session.close_event_channel(); // sets closed + closed_flag
                session.queue_notify.notify_waiters();
            }
            Err(_) => {
                // Could not acquire lock within timeout — notify_waiters best-effort
                // so the queue processor can eventually notice the shutdown flag.
                warn!(
                    "close_all_chat_sessions: session lock timeout, notifying waiters without lock"
                );
                session_arc
                    .try_lock()
                    .map(|s| s.queue_notify.notify_waiters())
                    .ok();
            }
        }
    }
}

async fn cleanup_idle_session(
    app: AppState,
    sessions: &SessionsMap,
    chat_id: &str,
    session_arc: Arc<AMutex<ChatSession>>,
) -> bool {
    if !save_idle_session_for_cleanup(app, session_arc.clone()).await {
        return false;
    }
    remove_idle_session_if_safe(sessions, chat_id, session_arc).await
}

async fn save_idle_session_for_cleanup(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> bool {
    match super::trajectories::try_save_trajectory_with_intent(
        app,
        session_arc,
        TrajectoryCommitIntent::Required,
    )
    .await
    {
        Ok(saved) => saved,
        Err(error) => {
            warn!("{}", error);
            false
        }
    }
}

async fn remove_idle_session_if_safe(
    sessions: &SessionsMap,
    chat_id: &str,
    session_arc: Arc<AMutex<ChatSession>>,
) -> bool {
    const MAX_REMOVAL_STRONG_REFS: usize = 3;

    let mut sessions_write = sessions.write().await;
    let current_matches = sessions_write
        .get(chat_id)
        .map_or(false, |current| Arc::ptr_eq(current, &session_arc));
    if !current_matches {
        return false;
    }

    let removal_snapshot = {
        let Ok(mut session) = session_arc.try_lock() else {
            return false;
        };
        let max_refs = MAX_REMOVAL_STRONG_REFS
            + usize::from(session.queue_processor_running.load(Ordering::SeqCst));
        if Arc::strong_count(&session_arc) > max_refs {
            return false;
        }
        if !close_idle_session_for_cleanup(&mut session) {
            return false;
        }
        CleanupRemovalSnapshot::capture(&session)
    };

    let safe_to_remove = {
        let Ok(session) = session_arc.try_lock() else {
            return false;
        };
        removal_snapshot.still_matches(&session)
    };
    if safe_to_remove {
        sessions_write.remove(chat_id);
        return true;
    }
    false
}

struct CleanupRemovalSnapshot {
    runtime_state: SessionState,
    runtime_paused: bool,
    runtime_error: Option<String>,
    pause_reasons_len: usize,
    messages_len: usize,
    command_queue_len: usize,
    pending_deliveries: Vec<PendingDelivery>,
    event_seq: u64,
    trajectory_version: u64,
    last_activity: Instant,
    draft_message_present: bool,
    pending_browser_message_present: bool,
    post_tool_side_effects_len: usize,
}

impl CleanupRemovalSnapshot {
    fn capture(session: &ChatSession) -> Self {
        Self {
            runtime_state: session.runtime.state,
            runtime_paused: session.runtime.paused,
            runtime_error: session.runtime.error.clone(),
            pause_reasons_len: session.runtime.pause_reasons.len(),
            messages_len: session.messages.len(),
            command_queue_len: session.command_queue.len(),
            pending_deliveries: session.pending_deliveries_for_snapshot(),
            event_seq: session.event_seq,
            trajectory_version: session.trajectory_version,
            last_activity: session.last_activity,
            draft_message_present: session.draft_message.is_some(),
            pending_browser_message_present: session.pending_browser_message.is_some(),
            post_tool_side_effects_len: session.post_tool_side_effects.len(),
        }
    }

    fn still_matches(&self, session: &ChatSession) -> bool {
        session.closed
            && session.closed_flag.load(Ordering::Relaxed)
            && session.is_idle_for_cleanup()
            && session.runtime.state == self.runtime_state
            && session.runtime.paused == self.runtime_paused
            && session.runtime.error == self.runtime_error
            && session.runtime.pause_reasons.len() == self.pause_reasons_len
            && session.messages.len() == self.messages_len
            && session.command_queue.len() == self.command_queue_len
            && session.pending_deliveries_for_snapshot() == self.pending_deliveries
            && session.event_seq == self.event_seq
            && session.trajectory_version == self.trajectory_version
            && session.last_activity == self.last_activity
            && session.draft_message.is_some() == self.draft_message_present
            && session.pending_browser_message.is_some() == self.pending_browser_message_present
            && session.post_tool_side_effects.len() == self.post_tool_side_effects_len
    }
}

fn close_idle_session_for_cleanup(session: &mut ChatSession) -> bool {
    if !session.is_idle_for_cleanup() {
        return false;
    }
    if !session.closed {
        session.close_event_channel();
    }
    session.queue_notify.notify_waiters();
    true
}

pub fn start_session_cleanup_task(app: AppState) {
    tokio::spawn(async move {
        let shutdown_flag = app.runtime.shutdown_flag.clone();
        let mut run_immediately = true;
        loop {
            if !run_immediately {
                tokio::select! {
                    _ = tokio::time::sleep(session_cleanup_interval()) => {}
                    _ = async {
                        while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                        }
                    } => {
                        tracing::info!("Session cleanup: shutdown detected, stopping");
                        return;
                    }
                }
            }
            run_immediately = false;

            let sessions = app.chat.sessions.clone();

            let candidates: Vec<(String, Arc<AMutex<ChatSession>>)> = {
                let sessions_read = sessions.read().await;
                sessions_read
                    .iter()
                    .map(|(chat_id, session_arc)| (chat_id.clone(), session_arc.clone()))
                    .collect()
            };

            let mut to_cleanup = Vec::new();
            for (chat_id, session_arc) in candidates {
                let session = session_arc.lock().await;
                if session.is_pending_wake_up() {
                    continue;
                }
                if session.is_idle_for_cleanup() {
                    drop(session);
                    to_cleanup.push((chat_id, session_arc));
                }
            }

            if to_cleanup.is_empty() {
                continue;
            }

            info!("Cleaning up {} idle sessions", to_cleanup.len());

            for (chat_id, session_arc) in &to_cleanup {
                if !cleanup_idle_session(app.clone(), &sessions, chat_id, session_arc.clone()).await
                {
                    continue;
                }

                let app_hook = app.clone();
                let chat_id_hook = chat_id.clone();
                tokio::spawn(async move {
                    let project_dir = get_project_dir_string(app_hook.clone()).await;
                    let payload =
                        HookPayload::new(&HookEvent::SessionEnd, chat_id_hook, project_dir);
                    run_observation_hooks(app_hook, HookEvent::SessionEnd, payload).await;
                });

                info!("Saved trajectory for closed session {}", chat_id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::session::create_sessions_map;
    use super::super::types::{
        ChatCommand, CommandRequest, PauseReason, TaskMeta, session_idle_timeout,
    };
    use crate::call_validation::{ChatContent, ChatMessage, ChatToolCall, ChatToolFunction};
    use crate::exec::{ExecMode, ExecProcessId, ExecProcessMeta};
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::Instant;
    use tokio::sync::Mutex as AMutex;

    fn make_session() -> ChatSession {
        ChatSession::new("test-chat".to_string())
    }

    async fn test_app_with_workspace(
        workspace: &std::path::Path,
    ) -> (crate::app_state::AppState, tempfile::TempDir) {
        let config_dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            workspace.join("cache"),
            config_dir.path().to_path_buf(),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.to_path_buf()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        (app, config_dir)
    }

    fn background_completion_meta(chat_id: &str) -> ExecProcessMeta {
        ExecProcessMeta::new(ExecMode::Background, "true".to_string())
            .with_process_id(ExecProcessId("exec_cleanup_order".to_string()))
            .with_chat_id(chat_id)
            .with_short_description("cleanup order".to_string())
    }

    fn legacy_reconstruction_trajectory(chat_id: &str) -> serde_json::Value {
        let record = serde_json::json!({
            "path": "a.rs",
            "zone": "secrets",
            "attribution": "declared",
        });
        let union = serde_json::json!({"files": [record]});
        let payload = ["user", "event", "goal", "user"]
            .into_iter()
            .enumerate()
            .map(|(index, role)| {
                let mut message = serde_json::json!({
                    "message_id": format!("payload-{index}"),
                    "role": role,
                    "content": format!("{role} {index}"),
                    "privacy": union.clone(),
                });
                match role {
                    "goal" => {
                        message["goal"] = serde_json::json!({"version": 1, "mode": "agent"});
                    }
                    "event" => {
                        message["event"] = serde_json::json!({
                            "subkind": "system_notice",
                            "source": "test",
                            "payload": {},
                        });
                    }
                    _ => {}
                }
                message
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "id": chat_id,
            "title": "Legacy reconstruction",
            "model": "model",
            "mode": "agent",
            "tool_use": "agent",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:01Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "messages": [
                {
                    "message_id": "archived",
                    "role": "user",
                    "content": "archived",
                },
                {
                    "message_id": "boundary",
                    "role": "compression_report",
                    "content": "Conversation context reconstructed.",
                    "compression_report": {
                        "kind": "reconstructed_history",
                        "schema_version": 1,
                        "payload": {"messages": payload},
                    },
                },
            ],
        })
    }

    fn payload_roles_carrying_privacy(messages: &[ChatMessage]) -> Vec<String> {
        refact_core::active_context::active_context(messages)
            .unwrap()
            .messages
            .into_iter()
            .filter(|message| message.extra.contains_key("privacy"))
            .map(|message| message.role)
            .collect()
    }

    #[tokio::test]
    async fn opening_a_legacy_reconstruction_trajectory_compacts_and_rewrites_it() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace_root =
            crate::files_correction::canonicalize_normalized_path(workspace.path().to_path_buf());
        let (app, _config_dir) = test_app_with_workspace(&workspace_root).await;
        let chat_id = "legacy-reconstruction-open";
        let path = workspace_root
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string(&legacy_reconstruction_trajectory(chat_id)).unwrap(),
        )
        .await
        .unwrap();

        let session_arc =
            get_or_create_session_with_trajectory(app.clone(), &app.chat.sessions, chat_id).await;

        let loaded_roles = {
            let session = session_arc.lock().await;
            payload_roles_carrying_privacy(&session.messages)
        };
        assert_eq!(loaded_roles, vec!["user", "user"]);

        let rewritten = serde_json::from_str::<serde_json::Value>(
            &tokio::fs::read_to_string(&path).await.unwrap(),
        )
        .unwrap();
        let stored = crate::call_validation::deserialize_messages_from_post(
            rewritten["messages"].as_array().unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload_roles_carrying_privacy(&stored),
            vec!["user", "user"]
        );
    }

    #[tokio::test]
    async fn opening_a_transition_trajectory_needing_identity_repair_still_compacts() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace_root =
            crate::files_correction::canonicalize_normalized_path(workspace.path().to_path_buf());
        let (app, _config_dir) = test_app_with_workspace(&workspace_root).await;
        let chat_id = "legacy-reconstruction-transition";
        let path = workspace_root
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        let mut trajectory = legacy_reconstruction_trajectory(chat_id);
        trajectory["link_type"] = serde_json::json!("mode_transition");
        trajectory["previous_response_id"] = serde_json::json!("resp-1");
        tokio::fs::write(&path, serde_json::to_string(&trajectory).unwrap())
            .await
            .unwrap();

        let session_arc =
            get_or_create_session_with_trajectory(app.clone(), &app.chat.sessions, chat_id).await;

        let loaded_roles = {
            let session = session_arc.lock().await;
            payload_roles_carrying_privacy(&session.messages)
        };
        assert_eq!(loaded_roles, vec!["user", "user"]);

        let rewritten = serde_json::from_str::<serde_json::Value>(
            &tokio::fs::read_to_string(&path).await.unwrap(),
        )
        .unwrap();
        assert!(rewritten.get("previous_response_id").is_none());
        let stored = crate::call_validation::deserialize_messages_from_post(
            rewritten["messages"].as_array().unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload_roles_carrying_privacy(&stored),
            vec!["user", "user"]
        );
    }

    #[tokio::test]
    async fn cleanup_saves_before_session_removal() {
        let workspace = tempfile::tempdir().unwrap();
        let (app, _config_dir) = test_app_with_workspace(workspace.path()).await;
        let chat_id = "cleanup-saves-before-removal";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
            session.last_activity =
                Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        assert!(
            cleanup_idle_session(
                app.clone(),
                &app.chat.sessions,
                chat_id,
                session_arc.clone(),
            )
            .await
        );

        assert!(app.chat.sessions.read().await.get(chat_id).is_none());
        assert!(
            super::super::trajectories::load_trajectory_for_chat(app.gcx.clone(), chat_id)
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn close_and_cleanup_retain_required_commit_boundaries() {
        let mut session = make_session();
        session.last_activity =
            Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);

        assert_eq!(
            super::super::trajectories::trajectory_commit_contract_for_test("close"),
            TrajectoryCommitIntent::Required
        );
        assert_eq!(
            super::super::trajectories::trajectory_commit_contract_for_test("graceful shutdown"),
            TrajectoryCommitIntent::Required
        );
        assert!(close_idle_session_for_cleanup(&mut session));
        assert!(session.closed);
    }

    #[tokio::test]
    async fn cleanup_save_failure_keeps_session_mapped_and_open() {
        let workspace = tempfile::tempdir().unwrap();
        let (app, _config_dir) = test_app_with_workspace(workspace.path()).await;
        let chat_id = "cleanup-save-failure-keeps-session";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.task_meta = Some(TaskMeta {
                task_id: "missing-task-for-save-failure".to_string(),
                role: "agents".to_string(),
                agent_id: Some("agent".to_string()),
                card_id: Some("card".to_string()),
                planner_chat_id: None,
            });
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
            session.last_activity =
                Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        assert!(
            !cleanup_idle_session(
                app.clone(),
                &app.chat.sessions,
                chat_id,
                session_arc.clone(),
            )
            .await
        );

        assert!(app.chat.sessions.read().await.get(chat_id).is_some());
        let session = session_arc.lock().await;
        assert!(!session.closed);
        assert!(session.trajectory_dirty);
    }

    #[tokio::test]
    async fn cleanup_keeps_session_when_completion_arrives_during_save() {
        let workspace = tempfile::tempdir().unwrap();
        let (app, _config_dir) = test_app_with_workspace(workspace.path()).await;
        let chat_id = "cleanup-keeps-completion-during-save";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
            session
                .queue_processor_running
                .store(true, Ordering::SeqCst);
            session.last_activity =
                Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        super::super::trajectories::maybe_save_trajectory_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Required,
        )
        .await;
        let process_id = app
            .runtime
            .exec_registry
            .register(background_completion_meta(chat_id), 4096)
            .await
            .meta
            .process_id;
        app.runtime
            .exec_registry
            .mark_started(&process_id)
            .await
            .unwrap();
        let mut completion_rx = app.runtime.exec_registry.subscribe_completion();
        app.runtime
            .exec_registry
            .mark_exited(&process_id, Some(0))
            .await
            .unwrap();
        let completion = completion_rx.recv().await.unwrap();
        crate::chat::notifications::handle_process_completion(app.gcx.clone(), completion).await;

        assert!(
            !cleanup_idle_session(
                app.clone(),
                &app.chat.sessions,
                chat_id,
                session_arc.clone(),
            )
            .await
        );

        let session = session_arc.lock().await;
        assert!(!session.closed);
        assert!(session.command_queue.is_empty());
        assert_eq!(session.delivery_wake_sources.len(), 1);
        assert!(session.last_activity.elapsed() <= session_idle_timeout());
        session
            .queue_processor_running
            .store(false, Ordering::SeqCst);
        drop(session);
        assert!(app.chat.sessions.read().await.get(chat_id).is_some());
        app.runtime.shutdown_flag.store(true, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn cleanup_persists_pending_deliveries_before_removal() {
        let workspace = tempfile::tempdir().unwrap();
        let (app, _config_dir) = test_app_with_workspace(workspace.path()).await;
        let chat_id = "cleanup-pending-deliveries";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.into())));
        let delivery = PendingDelivery::with_id(
            "cleanup-id",
            vec![ChatMessage::new("user".into(), "queued".into())],
            PushMode::WhenIdle,
            "test",
            false,
        );
        {
            let mut session = session_arc.lock().await;
            session.last_activity =
                Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
            session.pending_deliveries.push_back(delivery.clone());
            session.increment_version();
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.into(), session_arc.clone());
        assert!(cleanup_idle_session(app.clone(), &app.chat.sessions, chat_id, session_arc).await);
        let loaded = super::super::trajectories::load_trajectory_for_chat(app.gcx.clone(), chat_id)
            .await
            .unwrap();
        assert_eq!(loaded.pending_deliveries, vec![delivery.clone()]);
        let restored =
            get_or_create_session_with_trajectory(app.clone(), &app.chat.sessions, chat_id).await;
        let restored = restored.lock().await.pending_deliveries_for_snapshot();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, delivery.id);
        assert_eq!(
            restored[0]
                .messages
                .iter()
                .map(|message| message.content.content_text_only())
                .collect::<Vec<_>>(),
            delivery
                .messages
                .iter()
                .map(|message| message.content.content_text_only())
                .collect::<Vec<_>>()
        );
        assert_eq!(restored[0].push, delivery.push);
        assert_eq!(restored[0].enqueued_at_ms, delivery.enqueued_at_ms);
        assert!(restored[0].expires_at_ms.is_some());
    }

    #[test]
    fn cleanup_rechecks_pending_deliveries() {
        let mut session = make_session();
        session.last_activity =
            Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        assert!(close_idle_session_for_cleanup(&mut session));
        let snapshot = CleanupRemovalSnapshot::capture(&session);
        let delivery = PendingDelivery::new(
            vec![ChatMessage::new("user".into(), "queued".into())],
            PushMode::WhenIdle,
            "test",
            false,
        );
        session.pending_deliveries.push_back(delivery.clone());
        assert!(!snapshot.still_matches(&session));
        session.pending_deliveries.clear();
        session.runner_pending_deliveries.push(delivery);
        assert!(!snapshot.still_matches(&session));
    }

    #[test]
    fn cleanup_close_helper_rechecks_idle_state() {
        let mut session = make_session();
        session.last_activity =
            Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        assert!(close_idle_session_for_cleanup(&mut session));
        assert!(session.closed);

        let mut already_closed = make_session();
        already_closed.close_event_channel();
        already_closed.last_activity =
            Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        assert!(close_idle_session_for_cleanup(&mut already_closed));
        assert!(already_closed.closed);

        let mut touched = make_session();
        touched.last_activity = Instant::now();
        assert!(!close_idle_session_for_cleanup(&mut touched));
        assert!(!touched.closed);
    }

    #[tokio::test]
    async fn cleanup_final_removal_recheck_keeps_queued_command() {
        let sessions = create_sessions_map();
        let chat_id = "cleanup-final-recheck-queued";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
            session.last_activity =
                Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        }
        sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        {
            let mut session = session_arc.lock().await;
            session.enqueue_priority_command(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "queued-before-removal".to_string(),
                priority: false,
                command: ChatCommand::Regenerate {},
            });
        }

        assert!(!remove_idle_session_if_safe(&sessions, chat_id, session_arc.clone(),).await);
        assert!(sessions.read().await.get(chat_id).is_some());
        let session = session_arc.lock().await;
        assert!(session
            .command_queue
            .iter()
            .any(|request| request.client_request_id == "queued-before-removal"));
    }

    #[tokio::test]
    async fn cleanup_final_removal_recheck_keeps_outstanding_session_ref() {
        let sessions = create_sessions_map();
        let chat_id = "cleanup-final-recheck-ref";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
            session.last_activity =
                Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        }
        sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());
        let delayed_enqueue_ref = session_arc.clone();

        assert!(!remove_idle_session_if_safe(&sessions, chat_id, session_arc.clone(),).await);

        {
            let mut session = delayed_enqueue_ref.lock().await;
            session.enqueue_priority_command(CommandRequest {
                native_approval_guard: None,
                supervision_guard: None,
                client_request_id: "queued-from-outstanding-ref".to_string(),
                priority: false,
                command: ChatCommand::Regenerate {},
            });
        }

        let session = sessions.read().await.get(chat_id).cloned().unwrap();
        let session = session.lock().await;
        assert!(session
            .command_queue
            .iter()
            .any(|request| request.client_request_id == "queued-from-outstanding-ref"));
    }
}
