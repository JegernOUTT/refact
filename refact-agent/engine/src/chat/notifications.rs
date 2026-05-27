use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::json;
use tokio::sync::Mutex as AMutex;
use tokio::task::JoinHandle;

use crate::call_validation::ChatMessage;
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::types::{ChatSession, SessionState};
use crate::exec::{ExecStatus, ProcessCompletionEvent};
use crate::global_context::SharedGlobalContext;

const IDLE_WAIT_TIMEOUT: Duration = Duration::from_secs(1);

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

async fn handle_process_completion(gcx: SharedGlobalContext, event: ProcessCompletionEvent) {
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(&event.chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return;
    };
    inject_when_idle(session_arc, event).await;
}

async fn inject_when_idle(session_arc: Arc<AMutex<ChatSession>>, event: ProcessCompletionEvent) {
    loop {
        let notify = {
            let mut session = session_arc.lock().await;
            if session.closed {
                return;
            }
            if is_stream_busy(session.runtime.state) {
                session.queue_notify.clone()
            } else {
                inject_process_completion_message(&mut session, event);
                return;
            }
        };
        let _ = tokio::time::timeout(IDLE_WAIT_TIMEOUT, notify.notified()).await;
    }
}

fn is_stream_busy(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Generating | SessionState::ExecutingTools
    )
}

fn inject_process_completion_message(session: &mut ChatSession, event: ProcessCompletionEvent) {
    session.add_message(process_completion_message(event));
}

fn process_completion_message(completion: ProcessCompletionEvent) -> ChatMessage {
    let status = status_label(&completion.status);
    let mode = completion.mode.to_string();
    let exit_code = completion.exit_code;
    let process_id = completion.process_id.clone();
    let duration_ms = completion.duration_ms;
    let short_description = completion.short_description;
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
    async fn injection_waits_for_idle_chat() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let subscriber = spawn_notification_subscriber(gcx.clone());
        let chat_id = "injection-waits-for-idle-chat";
        let session = test_session(&gcx, chat_id).await;
        {
            let mut session = session.lock().await;
            session.set_runtime_state(SessionState::Generating, None);
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

        {
            let mut session = session.lock().await;
            session.set_runtime_state(SessionState::Idle, None);
            session.queue_notify.notify_waiters();
        }
        let message = wait_for_process_completed(&session).await;
        assert_eq!(process_payload(&message)["process_id"], json!(process_id));
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
        let message = process_completion_message(ProcessCompletionEvent {
            process_id: ExecProcessId("exec_shape".to_string()),
            chat_id: "chat-shape".to_string(),
            status: ExecStatus::Exited { exit_code: Some(3) },
            exit_code: Some(3),
            duration_ms: Some(42),
            short_description: "shape process".to_string(),
            mode: ExecMode::Background,
        });
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
}
