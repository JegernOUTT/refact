use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::json;
use tokio::sync::Mutex as AMutex;
use tokio::task::JoinHandle;

use crate::call_validation::ChatMessage;
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::types::{ChatCommand, ChatEvent, ChatSession, CommandRequest};
use crate::exec::{ExecStatus, ProcessCompletionEvent};
use crate::global_context::SharedGlobalContext;

pub fn spawn_notification_subscriber(gcx: SharedGlobalContext) -> JoinHandle<()> {
    let mut rx = gcx.exec_registry.subscribe_completion();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = wait_for_shutdown(gcx.clone()) => break,
                event = rx.recv() => match event {
                    Ok(event) => handle_process_completion(gcx.clone(), event).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        tracing::warn!("process completion notification subscriber lagged by {count} event(s)");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    })
}

async fn wait_for_shutdown(gcx: SharedGlobalContext) {
    while !gcx.shutdown_flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(crate) async fn handle_process_completion(
    gcx: SharedGlobalContext,
    event: ProcessCompletionEvent,
) {
    let session_arc = process_completion_session(gcx.clone(), &event.chat_id).await;
    let Some(session_arc) = session_arc else {
        return;
    };
    if inject_as_priority(gcx.clone(), session_arc, event.clone()).await {
        return;
    }
    if let Some(session_arc) = process_completion_session(gcx.clone(), &event.chat_id).await {
        inject_as_priority(gcx, session_arc, event).await;
    }
}

async fn process_completion_session(
    gcx: SharedGlobalContext,
    chat_id: &str,
) -> Option<Arc<AMutex<ChatSession>>> {
    if let Some(session_arc) = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    } {
        let is_closed = session_arc.lock().await.closed;
        if !is_closed {
            return Some(session_arc);
        }
    }

    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    if !super::session::try_restore_session_if_trajectory_exists(app, &gcx.chat_sessions, chat_id)
        .await
    {
        tracing::warn!(
            "process completion notification skipped: chat session {chat_id} is not loaded and has no trajectory"
        );
        return None;
    }

    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    if session_arc.is_none() {
        tracing::warn!(
            "process completion notification skipped: restored chat session {chat_id} is unavailable"
        );
    }
    session_arc
}

async fn inject_as_priority(
    gcx: SharedGlobalContext,
    session_arc: Arc<AMutex<ChatSession>>,
    event: ProcessCompletionEvent,
) -> bool {
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let processor_running = {
        let mut session = session_arc.lock().await;
        if session.closed {
            return false;
        }
        inject_process_completion_message(&mut session, event.clone());
        session.enqueue_priority_command(CommandRequest {
            client_request_id: format!("process-completed-{}", event.process_id),
            priority: true,
            command: ChatCommand::Regenerate {},
        });
        session.queue_processor_running.clone()
    };
    if !processor_running.swap(true, Ordering::SeqCst) {
        tokio::spawn(crate::chat::queue::process_command_queue(
            app,
            session_arc,
            processor_running,
        ));
    }
    true
}

pub(crate) fn inject_process_completion_message(
    session: &mut ChatSession,
    event: ProcessCompletionEvent,
) {
    let envelope = process_completion_envelope_event(&event);
    session.add_message(process_completion_message(&event));
    session.emit(envelope);
}

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
        }),
        content,
    )
}

fn status_label(status: &ExecStatus) -> &'static str {
    match status {
        ExecStatus::Starting => "starting",
        ExecStatus::Running => "running",
        ExecStatus::Exited { .. } => "exited",
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

    async fn wait_for_queued_regenerate(session: &Arc<AMutex<ChatSession>>) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            {
                let session = session.lock().await;
                if session
                    .command_queue
                    .iter()
                    .any(|request| matches!(request.command, ChatCommand::Regenerate {}))
                {
                    return;
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "process completion regenerate was not queued"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
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
        }
    }

    #[tokio::test]
    async fn background_process_exit_injects_event() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
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
        let gcx = crate::global_context::tests::make_test_gcx().await;
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
        let gcx = crate::global_context::tests::make_test_gcx().await;
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
        let gcx = crate::global_context::tests::make_test_gcx().await;
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
    async fn injection_interrupts_busy_chat() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let subscriber = spawn_notification_subscriber(gcx.clone());
        let chat_id = "injection-interrupts-busy-chat";
        let session = test_session(&gcx, chat_id).await;
        {
            let mut session = session.lock().await;
            session.start_stream();
            session
                .queue_processor_running
                .store(true, Ordering::SeqCst);
        }

        let process_id = spawn_notification_test_process(
            &gcx.exec_registry,
            ExecMode::Background,
            chat_id,
            sleep_command("0.1"),
        )
        .await;
        let _ = gcx.exec_registry.wait(&process_id).await.unwrap();
        let message = wait_for_process_completed(&session).await;
        assert_eq!(process_payload(&message)["process_id"], json!(process_id));
        wait_for_queued_regenerate(&session).await;
        {
            let session = session.lock().await;
            assert!(session.abort_flag.load(Ordering::SeqCst));
            assert_eq!(
                session.runtime.state,
                crate::chat::types::SessionState::Idle
            );
        }
        {
            let session = session.lock().await;
            session
                .queue_processor_running
                .store(false, Ordering::SeqCst);
        }
        subscriber.abort();
    }

    #[tokio::test]
    async fn closed_chat_drops_cleanly() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
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
}
