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
use crate::ext::hooks_runner::{HookPayload, get_project_dir_string, run_hooks};

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
    let chat_id = session.chat_id.clone();
    let base_background_agents: HashMap<String, BackgroundAgentSummary> =
        session.background_agents.clone();
    let mut snapshot = session.snapshot();
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
        super::trajectories::maybe_save_trajectory(app, session_arc).await;
    }
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
        let mut session = ChatSession::new_with_trajectory(
            chat_id.to_string(),
            loaded.messages,
            loaded.thread,
            loaded.created_at,
            loaded.wake_up_at,
            loaded.waiting_for_card_ids,
            loaded.goal,
        );
        if transition_identity_repaired {
            session.increment_version();
        }
        (session, false, transition_repair_patch)
    } else {
        let mut s = ChatSession::new(chat_id.to_string());
        s.increment_version();
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

    session.trajectory_events_tx = Some(trajectory_events_tx.clone());

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

    if inserted && is_new {
        let app_hook = AppState::from_gcx(gcx.clone()).await;
        let chat_id_clone = chat_id.to_string();
        tokio::spawn(async move {
            let project_dir = get_project_dir_string(app_hook.clone()).await;
            let payload = HookPayload {
                hook_event_name: "SessionStart".to_string(),
                session_id: chat_id_clone,
                project_dir,
                tool_name: None,
                tool_input: None,
                tool_output: None,
                user_prompt: None,
                extra: std::collections::HashMap::new(),
            };
            run_hooks(app_hook, HookEvent::SessionStart, payload).await;
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
                if session.trajectory_version == repaired_version {
                    session.trajectory_dirty = false;
                }
            }
        }
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
    match super::trajectories::try_save_trajectory(app, session_arc).await {
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
        let mut interval = tokio::time::interval(session_cleanup_interval());
        let shutdown_flag = app.runtime.shutdown_flag.clone();
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = async {
                    while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    }
                } => {
                    tracing::info!("Session cleanup: shutdown detected, stopping");
                    return;
                }
            }

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
                    let payload = HookPayload {
                        hook_event_name: "SessionEnd".to_string(),
                        session_id: chat_id_hook,
                        project_dir,
                        tool_name: None,
                        tool_input: None,
                        tool_output: None,
                        user_prompt: None,
                        extra: std::collections::HashMap::new(),
                    };
                    run_hooks(app_hook, HookEvent::SessionEnd, payload).await;
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
        ChatCommand, CommandRequest, PauseReason, SessionState, TaskMeta, session_idle_timeout,
    };
    use crate::call_validation::{ChatContent, ChatMessage, ChatToolCall, ChatToolFunction};
    use crate::exec::{ExecMode, ExecProcessId, ExecProcessMeta};
    use std::collections::HashSet;
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

        super::super::trajectories::maybe_save_trajectory(app.clone(), session_arc.clone()).await;
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
        assert!(session
            .command_queue
            .iter()
            .any(|request| matches!(request.command, ChatCommand::Regenerate {})));
        assert!(session.last_activity.elapsed() <= session_idle_timeout());
        session
            .queue_processor_running
            .store(false, Ordering::SeqCst);
        drop(session);
        assert!(app.chat.sessions.read().await.get(chat_id).is_some());
        app.runtime.shutdown_flag.store(true, Ordering::Relaxed);
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

    fn make_assistant_with_tool_calls(ids: &[&str]) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            tool_calls: Some(
                ids.iter()
                    .map(|id| ChatToolCall {
                        id: id.to_string(),
                        index: None,
                        function: ChatToolFunction {
                            name: "shell".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    fn make_pause_reason(tool_call_id: &str) -> PauseReason {
        PauseReason {
            reason_type: "confirmation".into(),
            tool_name: "shell".into(),
            command: "shell".into(),
            rule: "ask".into(),
            tool_call_id: tool_call_id.into(),
            integr_config_path: None,
        }
    }

    fn make_tool_result(tool_call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            tool_call_id: tool_call_id.to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            tool_failed: Some(false),
            ..Default::default()
        }
    }

    fn default_openai_settings() -> crate::llm::adapter::AdapterSettings {
        crate::llm::adapter::AdapterSettings {
            api_key: "test-key".to_string(),
            auth_token: String::new(),
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            extra_headers: Default::default(),
            model_name: "gpt-4.1".to_string(),
            supports_tools: true,
            supports_reasoning: true,
            reasoning_type: None,
            supports_temperature: true,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            supports_web_search: false,
            supports_cache_control: false,
        }
    }

    fn assert_openai_tool_results_follow_assistant(messages: Vec<ChatMessage>) {
        use crate::llm::adapter::LlmWireAdapter;
        use crate::llm::adapters::openai_chat::OpenAiChatAdapter;

        let req = crate::llm::canonical::LlmRequest::new("gpt-4.1".to_string(), messages);
        let body = OpenAiChatAdapter
            .build_http(&req, &default_openai_settings())
            .unwrap()
            .body;
        let wire_messages = body["messages"].as_array().unwrap();
        for (idx, message) in wire_messages.iter().enumerate() {
            if message["role"] != "assistant" || message.get("tool_calls").is_none() {
                continue;
            }
            let tool_calls = message["tool_calls"].as_array().unwrap();
            let expected_ids: HashSet<String> = tool_calls
                .iter()
                .filter_map(|tool_call| tool_call["id"].as_str().map(str::to_string))
                .collect();
            let actual_ids: HashSet<String> = wire_messages
                .iter()
                .skip(idx + 1)
                .take(tool_calls.len())
                .map(|tool_result| {
                    assert_eq!(
                        tool_result["role"], "tool",
                        "wire messages: {wire_messages:?}"
                    );
                    tool_result["tool_call_id"].as_str().unwrap().to_string()
                })
                .collect();
            assert_eq!(actual_ids, expected_ids, "wire messages: {wire_messages:?}");
        }
    }

    #[test]
    fn openai_wire_order_valid_after_tool_decision() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1", "tc2"]));
        session.runtime.pause_reasons.push(make_pause_reason("tc1"));
        session.runtime.pause_reasons.push(make_pause_reason("tc2"));
        session.set_runtime_state(SessionState::Paused, None);

        session.process_tool_decisions(&[
            ToolDecisionItem {
                tool_call_id: "tc1".into(),
                accepted: false,
            },
            ToolDecisionItem {
                tool_call_id: "tc2".into(),
                accepted: true,
            },
        ]);
        session.add_message(make_tool_result("tc2", "accepted result"));
        session.drain_post_tool_side_effects();

        assert_openai_tool_results_follow_assistant(session.messages.clone());
    }

    #[test]
    fn openai_wire_order_valid_after_multi_ide_callbacks() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1", "tc2"]));
        session.runtime.state = SessionState::WaitingIde;

        session.record_ide_tool_result("tc1".to_string(), "first".to_string(), false);
        session.record_ide_tool_result("tc2".to_string(), "second".to_string(), false);

        assert_openai_tool_results_follow_assistant(session.messages.clone());
    }
}
