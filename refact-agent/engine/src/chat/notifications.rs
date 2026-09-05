#[cfg(test)]
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::json;
#[cfg(test)]
use tokio::sync::Mutex as AMutex;
use tokio::task::JoinHandle;

use refact_core::chat_types::{DeliveryOutcome, PendingDelivery, PushMode};

use crate::call_validation::ChatMessage;
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::types::ChatEvent;
#[cfg(test)]
use crate::chat::types::ChatSession;
use crate::exec::{ExecStatus, ProcessCompletionEvent, ProcessSpawnEvent};
use crate::global_context::SharedGlobalContext;

/// Stable, restart-safe dedupe key for a process completion notice. The exec
/// process id is unique per process, so one completion can only land once even
/// if the broadcast is replayed or the engine restarts mid-delivery.
pub(crate) fn process_completion_delivery_id(event: &ProcessCompletionEvent) -> String {
    format!("process-completed-{}", event.process_id)
}

pub fn spawn_notification_subscriber(gcx: SharedGlobalContext) -> JoinHandle<()> {
    let mut completion_rx = gcx.exec_registry.subscribe_completion();
    let mut spawn_rx = gcx.exec_registry.subscribe_spawn();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = wait_for_shutdown(gcx.clone()) => break,
                event = completion_rx.recv() => match event {
                    Ok(event) => handle_process_completion(gcx.clone(), event).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        tracing::warn!("process completion notification subscriber lagged by {count} event(s)");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                event = spawn_rx.recv() => match event {
                    Ok(event) => handle_process_spawn(gcx.clone(), event).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        tracing::warn!("process spawn notification subscriber lagged by {count} event(s)");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    })
}

async fn wait_for_shutdown(gcx: SharedGlobalContext) {
    while !gcx.shutdown_flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn handle_process_spawn(gcx: SharedGlobalContext, event: ProcessSpawnEvent) {
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(&event.chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return;
    };
    let mut session = session_arc.lock().await;
    if session.closed {
        return;
    }
    let seq = session.event_seq.saturating_add(1);
    session.emit(ChatEvent::ExecProcessSpawned {
        chat_id: event.chat_id,
        seq,
        process: refact_chat_api::ExecProcessSpawn {
            process_id: event.process_id.to_string(),
            command_preview: event.command_preview,
            mode: event.mode.to_string(),
            tty: event.tty,
            status: status_label(&event.status).to_string(),
            started_at: event.started_at_ms,
        },
    });
}

/// Deliver a background/service process completion into its owning chat using
/// the unified delivery hub, with the `push` mode selected when the process was
/// started (default `append`).
///
/// The hub owns restoring an unloaded session, runner-owned routing, dedupe and
/// waking the queue, so this function only builds the delivery and reports
/// failures honestly instead of dropping them.
pub(crate) async fn handle_process_completion(
    gcx: SharedGlobalContext,
    event: ProcessCompletionEvent,
) {
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    if let Err(error) = deliver_process_completion(app, &event).await {
        tracing::warn!(
            process_id = %event.process_id,
            chat_id = %event.chat_id,
            "process completion notification was not delivered: {error}"
        );
    }
}

pub(crate) async fn deliver_process_completion(
    app: crate::app_state::AppState,
    event: &ProcessCompletionEvent,
) -> Result<DeliveryOutcome, String> {
    let chat_id = event.chat_id.clone();
    crate::chat::delivery::deliver_to_chat(app, &chat_id, process_completion_delivery(event)).await
}

pub(crate) fn process_completion_delivery(event: &ProcessCompletionEvent) -> PendingDelivery {
    PendingDelivery::with_id(
        process_completion_delivery_id(event),
        vec![process_completion_message(event)],
        event.push,
        "exec.registry".to_string(),
        true,
    )
}

#[cfg(test)]
pub(crate) fn inject_process_completion_message(
    session: &mut ChatSession,
    event: ProcessCompletionEvent,
) {
    let envelope = process_completion_envelope_event(&event);
    session.add_message(process_completion_message(&event));
    session.emit(envelope);
}

#[cfg(test)]
fn process_completion_envelope_event(completion: &ProcessCompletionEvent) -> ChatEvent {
    ChatEvent::ProcessCompleted {
        process_id: completion.process_id.to_string(),
        status: status_label(&completion.status).to_string(),
        exit_code: completion.exit_code,
        short_description: completion.short_description.clone(),
        mode: completion.mode.to_string(),
    }
}

fn process_completion_message(completion: &ProcessCompletionEvent) -> ChatMessage {
    let status = status_label(&completion.status);
    let mode = completion.mode.to_string();
    let exit_code = completion.exit_code;
    let process_id = completion.process_id.to_string();
    let duration_ms = completion.duration_ms;
    let short_description = completion.short_description.clone();
    let exit_text = exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let content = format!(
        "Background process '{}' {} (exit {})",
        short_description, status, exit_text
    );
    event(
        EventSubkind::ProcessCompleted,
        "exec.registry",
        json!({
            "process_id": process_id,
            "status": status,
            "exit_code": exit_code,
            "duration_ms": duration_ms,
            "short_description": short_description,
            "mode": mode,
            "push": PushMode::as_str(completion.push),
        }),
        content,
    )
}

fn status_label(status: &ExecStatus) -> &'static str {
    match status {
        ExecStatus::Starting => "starting",
        ExecStatus::Running => "running",
        ExecStatus::Exited { .. } => "exited",
        ExecStatus::SandboxLauncherFailed { .. } => "sandbox_launcher_failed",
        ExecStatus::Failed { .. } => "failed",
        ExecStatus::Killed => "killed",
        ExecStatus::TimedOut => "timed_out",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::{ExecMode, ExecOwnerMeta, ExecProcessId, ExecRegistry, ExecSpawnRequest};
    use crate::chat::trajectories::{save_trajectory_snapshot, TrajectorySnapshot};

    async fn test_gcx_with_workspace() -> (SharedGlobalContext, tempfile::TempDir) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        (gcx, workspace)
    }

    async fn test_session(gcx: &SharedGlobalContext, chat_id: &str) -> Arc<AMutex<ChatSession>> {
        let session = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        gcx.chat_sessions
            .write()
            .await
            .insert(chat_id.to_string(), session.clone());
        session
    }

    fn sleep_command(duration: &str) -> String {
        if cfg!(windows) {
            format!("Start-Sleep -Seconds {duration}")
        } else {
            format!("sleep {duration}")
        }
    }

    fn owner(chat_id: &str) -> ExecOwnerMeta {
        ExecOwnerMeta {
            chat_id: Some(chat_id.to_string()),
            tool_call_id: Some("tool-call".to_string()),
            service_name: Some("notify-service".to_string()),
            workspace: None,
        }
    }

    async fn wait_for_process_completed(session: &Arc<AMutex<ChatSession>>) -> ChatMessage {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(message) = find_process_completed(session).await {
                return message;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "process completion event not injected"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_pending_delivery(session: &Arc<AMutex<ChatSession>>) -> PendingDelivery {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            {
                let session = session.lock().await;
                if let Some(pending) = session.pending_deliveries.front() {
                    return pending.clone();
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "process completion delivery was not queued"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn completion_event(process_id: &str, chat_id: &str, push: PushMode) -> ProcessCompletionEvent {
        ProcessCompletionEvent {
            process_id: ExecProcessId(process_id.to_string()),
            chat_id: chat_id.to_string(),
            status: ExecStatus::Exited { exit_code: Some(0) },
            exit_code: Some(0),
            duration_ms: Some(5),
            short_description: "test process".to_string(),
            mode: ExecMode::Background,
            push,
        }
    }

    fn find_process_completed_in(session: &ChatSession) -> Option<ChatMessage> {
        session
            .messages
            .iter()
            .find(|message| is_process_completed_message(message))
            .cloned()
    }

    async fn find_process_completed(session: &Arc<AMutex<ChatSession>>) -> Option<ChatMessage> {
        let session = session.lock().await;
        session
            .messages
            .iter()
            .find(|message| is_process_completed_message(message))
            .cloned()
    }

    fn is_process_completed_message(message: &ChatMessage) -> bool {
        message.role == crate::chat::internal_roles::EVENT_ROLE
            && message
                .extra
                .get("event")
                .and_then(|event| event.get("subkind"))
                .and_then(serde_json::Value::as_str)
                == Some("process_completed")
    }

    fn process_payload(message: &ChatMessage) -> serde_json::Value {
        message.extra["event"]["payload"].clone()
    }

    async fn spawn_notification_test_process(
        registry: &ExecRegistry,
        mode: ExecMode,
        chat_id: &str,
        command: String,
    ) -> ExecProcessId {
        let mut request = ExecSpawnRequest::new(mode, command)
            .with_owner(owner(chat_id))
            .with_short_description("test process");
        if matches!(request.mode, ExecMode::Service) {
            request = request.with_startup_wait(Duration::from_millis(10));
        }
        let result = registry.spawn(request).await.unwrap();
        result.snapshot.meta.process_id
    }

    fn notification_test_snapshot(chat_id: &str) -> TrajectorySnapshot {
        TrajectorySnapshot {
            goal: None,
            goal_ledger: Vec::new(),
            goal_verification_blocked_until_ms: None,
            compression_retry_after_ms: Default::default(),
            chat_id: chat_id.to_string(),
            title: "Notification test".to_string(),
            model: "model".to_string(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            messages: vec![ChatMessage::new("user".to_string(), "hello".to_string())],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            boost_reasoning: false,
            checkpoints_enabled: true,
            context_tokens_cap: None,
            auto_compression_cap: None,
            include_project_info: true,
            is_title_generated: true,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            autonomous_no_confirm: false,
            version: 1,
            task_meta: None,
            worktree: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            reasoning_effort: None,
            thinking_budget: None,
            temperature: None,
            frequency_penalty: None,
            max_tokens: None,
            parallel_tool_calls: None,
            previous_response_id: None,
            active_skill: None,
            auto_enrichment_enabled: None,
            buddy_meta: None,
            auto_compact_enabled: None,
            frozen_request_prefix: None,
            claude_code_identity: None,
            reactive_compact_attempts: None,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
            pending_deliveries: Default::default(),
        }
    }

    #[tokio::test]
    async fn runner_completion_is_queued_in_actual_inbox_not_only_sse() {
        use crate::agents::types::{BgAgentKind, CreateAgentRequest};
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let session = test_session(&gcx, "runner-completion").await;
        let (record, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "parent".into(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "test".into(),
                title: "test".into(),
                prompt: "test".into(),
                target_files: vec![],
                model: "test".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        app.agents
            .mark_running(&record.agent_id, "runner-completion".into())
            .await
            .unwrap();
        let event = completion_event("exec_runner_notice", "runner-completion", PushMode::Append);
        assert_eq!(
            deliver_process_completion(app.clone(), &event)
                .await
                .unwrap(),
            DeliveryOutcome::Queued
        );
        assert_eq!(
            deliver_process_completion(app.clone(), &event)
                .await
                .unwrap(),
            DeliveryOutcome::Duplicate
        );
        assert!(find_process_completed(&session).await.is_none());
        let inbox = app
            .agents
            .drain_deliveries(&record.agent_id, false)
            .await
            .unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(
            inbox[0].messages[0].extra["event"]["payload"]["process_id"],
            "exec_runner_notice"
        );
        assert_eq!(inbox[0].id, process_completion_delivery_id(&event));
    }

    #[tokio::test]
    async fn background_process_exit_injects_event() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let subscriber = spawn_notification_subscriber(gcx.clone());
        let chat_id = "background-process-exit-injects-event";
        let session = test_session(&gcx, chat_id).await;

        let process_id = spawn_notification_test_process(
            &gcx.exec_registry,
            ExecMode::Background,
            chat_id,
            sleep_command("0.3"),
        )
        .await;
        let _ = gcx.exec_registry.wait(&process_id).await.unwrap();

        let message = wait_for_process_completed(&session).await;
        let payload = process_payload(&message);
        assert_eq!(payload["process_id"], json!(process_id));
        assert_eq!(payload["status"], json!("exited"));
        assert_eq!(payload["exit_code"], json!(0));
        assert!(payload["duration_ms"].is_number());
        assert_eq!(payload["short_description"], json!("test process"));
        assert_eq!(payload["mode"], json!("background"));
        subscriber.abort();
    }

    #[tokio::test]
    async fn service_process_exit_injects_event() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let subscriber = spawn_notification_subscriber(gcx.clone());
        let chat_id = "service-process-exit-injects-event";
        let session = test_session(&gcx, chat_id).await;

        let process_id = spawn_notification_test_process(
            &gcx.exec_registry,
            ExecMode::Service,
            chat_id,
            sleep_command("0.3"),
        )
        .await;
        let _ = gcx.exec_registry.wait(&process_id).await.unwrap();

        let message = wait_for_process_completed(&session).await;
        let payload = process_payload(&message);
        assert_eq!(payload["process_id"], json!(process_id));
        assert_eq!(payload["status"], json!("exited"));
        assert_eq!(payload["exit_code"], json!(0));
        assert_eq!(payload["short_description"], json!("test process"));
        assert_eq!(payload["mode"], json!("service"));
        subscriber.abort();
    }

    #[tokio::test]
    async fn process_completion_restores_unloaded_session_before_injection() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let subscriber = spawn_notification_subscriber(gcx.clone());
        let chat_id = "process-completion-restores-unloaded-session";
        save_trajectory_snapshot(gcx.clone(), notification_test_snapshot(chat_id))
            .await
            .unwrap();

        let process_id = spawn_notification_test_process(
            &gcx.exec_registry,
            ExecMode::Background,
            chat_id,
            sleep_command("0.1"),
        )
        .await;
        let _ = gcx.exec_registry.wait(&process_id).await.unwrap();

        let session = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(session) = gcx.chat_sessions.read().await.get(chat_id).cloned() {
                    return session;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("unloaded session was not restored for process completion");
        let message = wait_for_process_completed(&session).await;
        let payload = process_payload(&message);
        assert_eq!(payload["process_id"], json!(process_id));
        assert_eq!(payload["status"], json!("exited"));
        assert_eq!(payload["exit_code"], json!(0));
        subscriber.abort();
    }

    #[tokio::test]
    async fn foreground_process_no_injection() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let subscriber = spawn_notification_subscriber(gcx.clone());
        let chat_id = "foreground-process-no-injection";
        let session = test_session(&gcx, chat_id).await;

        let _ = gcx
            .exec_registry
            .spawn(
                ExecSpawnRequest::foreground(sleep_command("0.1"))
                    .with_owner(owner(chat_id))
                    .with_short_description("test process"),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(find_process_completed(&session).await.is_none());
        subscriber.abort();
    }

    #[tokio::test]
    async fn chat_owned_process_spawn_emits_sse_event() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let subscriber = spawn_notification_subscriber(gcx.clone());
        let chat_id = "chat-owned-process-spawn";
        let session = test_session(&gcx, chat_id).await;
        let mut events = session.lock().await.subscribe();
        tokio::task::yield_now().await;

        let result = gcx
            .exec_registry
            .spawn(
                ExecSpawnRequest::background(sleep_command("30"))
                    .with_owner(owner(chat_id))
                    .with_short_description("Live terminal".to_string())
                    .with_tty(true)
                    .with_chat_spawn_notification(),
            )
            .await
            .unwrap();

        let raw = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let raw = events.recv().await.unwrap();
                let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
                if value["type"] == "exec_process_spawned" {
                    return value;
                }
            }
        })
        .await
        .expect("process spawn event was not emitted");
        assert_eq!(raw["chat_id"], chat_id);
        assert!(raw["seq"].is_number());
        assert_eq!(
            raw["process"]["processId"],
            result.snapshot.meta.process_id.to_string()
        );
        assert_eq!(raw["process"]["commandPreview"], "Live terminal");
        assert_eq!(raw["process"]["mode"], "background");
        assert_eq!(raw["process"]["tty"], true);
        assert_eq!(raw["process"]["status"], "running");
        assert!(raw["process"]["startedAt"].as_u64().is_some());
        gcx.exec_registry
            .kill(&result.snapshot.meta.process_id)
            .await
            .unwrap();
        subscriber.abort();
    }

    #[tokio::test]
    async fn chatless_process_spawn_does_not_emit_sse_event() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let subscriber = spawn_notification_subscriber(gcx.clone());
        let session = test_session(&gcx, "chatless-process-spawn").await;
        let mut events = session.lock().await.subscribe();
        tokio::task::yield_now().await;

        let result = gcx
            .exec_registry
            .spawn(ExecSpawnRequest::background(sleep_command("30")).with_chat_spawn_notification())
            .await
            .unwrap();

        assert!(
            tokio::time::timeout(Duration::from_millis(200), events.recv())
                .await
                .is_err()
        );
        gcx.exec_registry
            .kill(&result.snapshot.meta.process_id)
            .await
            .unwrap();
        subscriber.abort();
    }

    /// B (append) is the default: a completion that arrives while the chat is
    /// streaming must NOT abort the draft, and must be queued rather than lost.
    #[tokio::test]
    async fn append_completion_during_stream_preserves_draft_and_queues_delivery() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let chat_id = "append-completion-during-stream";
        let session = test_session(&gcx, chat_id).await;
        {
            let mut session = session.lock().await;
            session.start_stream();
            session
                .queue_processor_running
                .store(true, Ordering::SeqCst);
        }

        let outcome = deliver_process_completion(
            app,
            &completion_event("exec_append_stream", chat_id, PushMode::Append),
        )
        .await
        .unwrap();

        assert_eq!(outcome, DeliveryOutcome::Queued);
        let pending = wait_for_pending_delivery(&session).await;
        assert_eq!(pending.id, "process-completed-exec_append_stream");
        assert_eq!(pending.push, PushMode::Append);
        let session = session.lock().await;
        // The in-flight draft survives: append never preempts.
        assert!(session.draft_message.is_some());
        assert!(!session.abort_flag.load(Ordering::SeqCst));
        assert!(find_process_completed_in(&session).is_none());
    }

    /// A (preempt) is explicit-only: it aborts the draft and lands immediately.
    #[tokio::test]
    async fn preempt_completion_cancels_the_draft_and_lands_now() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let chat_id = "preempt-completion-cancels-draft";
        let session = test_session(&gcx, chat_id).await;
        {
            let mut session = session.lock().await;
            session.start_stream();
            session
                .queue_processor_running
                .store(true, Ordering::SeqCst);
        }

        let outcome = deliver_process_completion(
            app,
            &completion_event("exec_preempt", chat_id, PushMode::Preempt),
        )
        .await
        .unwrap();

        assert_eq!(outcome, DeliveryOutcome::Delivered);
        let session = session.lock().await;
        assert!(session.abort_flag.load(Ordering::SeqCst));
        assert!(session.pending_deliveries.is_empty());
        assert!(find_process_completed_in(&session).is_some());
    }

    /// C (when_idle) waits for the whole turn, not just a message boundary.
    #[tokio::test]
    async fn when_idle_completion_waits_for_the_turn_to_finish() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let chat_id = "when-idle-completion-waits";
        let session = test_session(&gcx, chat_id).await;
        {
            let mut session = session.lock().await;
            session.turn_depth = 1;
            session.set_runtime_state(crate::chat::types::SessionState::Generating, None);
            session
                .queue_processor_running
                .store(true, Ordering::SeqCst);
        }

        let outcome = deliver_process_completion(
            app.clone(),
            &completion_event("exec_when_idle", chat_id, PushMode::WhenIdle),
        )
        .await
        .unwrap();

        assert_eq!(outcome, DeliveryOutcome::Queued);
        {
            let session = session.lock().await;
            assert!(find_process_completed_in(&session).is_none());
            assert_eq!(session.pending_deliveries.len(), 1);
        }

        {
            let mut session = session.lock().await;
            session.turn_depth = 0;
            session.set_runtime_state(crate::chat::types::SessionState::Idle, None);
        }
        crate::chat::delivery::drain_deliveries_at_boundary(app, session.clone()).await;

        let session = session.lock().await;
        assert!(session.pending_deliveries.is_empty());
        assert!(find_process_completed_in(&session).is_some());
    }

    /// The exec process id is a stable delivery id, so a replayed completion
    /// broadcast can never append the same notice twice.
    #[tokio::test]
    async fn duplicate_process_completion_delivery_is_idempotent() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let chat_id = "duplicate-process-completion-idempotent";
        let session = test_session(&gcx, chat_id).await;
        let event = completion_event("exec_duplicate_completion", chat_id, PushMode::Append);

        let first = deliver_process_completion(app.clone(), &event)
            .await
            .unwrap();
        let second = deliver_process_completion(app, &event).await.unwrap();

        assert_eq!(first, DeliveryOutcome::Delivered);
        assert_eq!(second, DeliveryOutcome::Duplicate);
        let session = session.lock().await;
        assert_eq!(
            session
                .messages
                .iter()
                .filter(|message| is_process_completed_message(message))
                .count(),
            1
        );
    }

    /// A closed chat is an honest error, not a silently swallowed completion.
    #[tokio::test]
    async fn closed_chat_delivery_reports_failure_instead_of_claiming_success() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let chat_id = "closed-chat-completion-error";
        let session = test_session(&gcx, chat_id).await;
        session.lock().await.closed = true;

        let error = deliver_process_completion(
            app,
            &completion_event("exec_closed_chat", chat_id, PushMode::Append),
        )
        .await
        .unwrap_err();

        assert!(error.contains("closed"), "unexpected error: {error}");
    }

    #[tokio::test]
    async fn ordinary_idle_session_gets_the_completion_message_immediately() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let chat_id = "ordinary-process-completion";
        let session = test_session(&gcx, chat_id).await;

        let outcome = deliver_process_completion(
            app,
            &completion_event("exec_ordinary", chat_id, PushMode::Append),
        )
        .await
        .unwrap();

        assert_eq!(outcome, DeliveryOutcome::Delivered);
        let session = session.lock().await;
        assert_eq!(
            session
                .messages
                .iter()
                .filter(|message| is_process_completed_message(message))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn closed_chat_drops_cleanly() {
        let (gcx, _workspace) = test_gcx_with_workspace().await;
        let subscriber = spawn_notification_subscriber(gcx.clone());
        let chat_id = "closed-chat-drops-cleanly";
        let session = test_session(&gcx, chat_id).await;
        {
            let mut session = session.lock().await;
            session.close_event_channel();
        }

        let process_id = spawn_notification_test_process(
            &gcx.exec_registry,
            ExecMode::Background,
            chat_id,
            sleep_command("0.1"),
        )
        .await;
        let _ = gcx.exec_registry.wait(&process_id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(find_process_completed(&session).await.is_none());
        subscriber.abort();
    }

    #[test]
    fn process_completion_message_has_expected_shape() {
        let completion = ProcessCompletionEvent {
            process_id: ExecProcessId("exec_shape".to_string()),
            chat_id: "chat-shape".to_string(),
            status: ExecStatus::Exited { exit_code: Some(3) },
            exit_code: Some(3),
            duration_ms: Some(42),
            short_description: "shape process".to_string(),
            mode: ExecMode::Background,
            push: PushMode::Append,
        };
        let message = process_completion_message(&completion);
        let payload = process_payload(&message);
        assert_eq!(message.role, crate::chat::internal_roles::EVENT_ROLE);
        assert_eq!(
            message.extra["event"]["subkind"],
            json!("process_completed")
        );
        assert_eq!(message.extra["event"]["source"], json!("exec.registry"));
        assert_eq!(payload["process_id"], json!("exec_shape"));
        assert_eq!(payload["status"], json!("exited"));
        assert_eq!(payload["exit_code"], json!(3));
        assert_eq!(payload["duration_ms"], json!(42));
        assert_eq!(payload["short_description"], json!("shape process"));
        assert_eq!(payload["mode"], json!("background"));
        assert_eq!(
            message.content.content_text_only(),
            "Background process 'shape process' exited (exit 3)"
        );
    }

    #[test]
    fn process_completion_envelope_has_expected_shape() {
        let event = process_completion_envelope_event(&ProcessCompletionEvent {
            process_id: ExecProcessId("exec_shape".to_string()),
            chat_id: "chat-shape".to_string(),
            status: ExecStatus::Exited { exit_code: Some(3) },
            exit_code: Some(3),
            duration_ms: Some(42),
            short_description: "shape process".to_string(),
            mode: ExecMode::Background,
            push: PushMode::Append,
        });

        match event {
            ChatEvent::ProcessCompleted {
                process_id,
                status,
                exit_code,
                short_description,
                mode,
            } => {
                assert_eq!(process_id, "exec_shape");
                assert_eq!(status, "exited");
                assert_eq!(exit_code, Some(3));
                assert_eq!(short_description, "shape process");
                assert_eq!(mode, "background");
            }
            other => panic!("expected process completed envelope, got {other:?}"),
        }
    }

    #[test]
    fn process_completion_event_appends_and_preserves_prefix() {
        let mut session = ChatSession::new("append-only-process".to_string());
        session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
        session.add_message(ChatMessage::new(
            "assistant".to_string(),
            "working".to_string(),
        ));
        session.add_message(ChatMessage {
            role: "tool".to_string(),
            tool_call_id: "call_1".to_string(),
            content: crate::call_validation::ChatContent::SimpleText("tool result".to_string()),
            ..Default::default()
        });
        let before = serde_json::to_value(&session.messages).unwrap();
        let before_len = session.messages.len();
        let chat_id = session.chat_id.clone();

        inject_process_completion_message(
            &mut session,
            ProcessCompletionEvent {
                process_id: ExecProcessId("exec_append".to_string()),
                chat_id,
                status: ExecStatus::Exited { exit_code: Some(0) },
                exit_code: Some(0),
                duration_ms: Some(7),
                short_description: "append process".to_string(),
                mode: ExecMode::Background,
                push: PushMode::Append,
            },
        );

        assert_eq!(session.messages.len(), before_len + 1);
        assert!(crate::chat::internal_roles::last_is_event(
            &session.messages
        ));
        assert_eq!(
            session.messages[before_len].extra["event"]["subkind"],
            json!("process_completed")
        );
        let after_prefix = serde_json::to_value(&session.messages[..before_len]).unwrap();
        assert_eq!(after_prefix, before);
    }

    #[test]
    fn repeated_process_completion_status_appends_new_event() {
        let mut session = ChatSession::new("append-only-status-repeat".to_string());
        session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
        let first = ProcessCompletionEvent {
            process_id: ExecProcessId("exec_repeat".to_string()),
            chat_id: session.chat_id.clone(),
            status: ExecStatus::Exited { exit_code: Some(1) },
            exit_code: Some(1),
            duration_ms: Some(3),
            short_description: "repeat process".to_string(),
            mode: ExecMode::Background,
            push: PushMode::Append,
        };
        inject_process_completion_message(&mut session, first.clone());
        let first_event = session.messages.last().unwrap().clone();
        let before_second = serde_json::to_value(&session.messages).unwrap();
        let before_len = session.messages.len();

        inject_process_completion_message(&mut session, first);

        assert_eq!(session.messages.len(), before_len + 1);
        assert!(crate::chat::internal_roles::last_is_event(
            &session.messages
        ));
        assert_eq!(
            serde_json::to_value(&session.messages[before_len - 1]).unwrap(),
            serde_json::to_value(&first_event).unwrap()
        );
        let after_prefix = serde_json::to_value(&session.messages[..before_len]).unwrap();
        assert_eq!(after_prefix, before_second);
        assert_ne!(
            session.messages[before_len - 1].message_id,
            session.messages[before_len].message_id
        );
        assert_eq!(
            session.messages[before_len].extra["event"]["subkind"],
            json!("process_completed")
        );
    }
}
