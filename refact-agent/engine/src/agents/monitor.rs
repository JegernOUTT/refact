use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{TimeDelta, Utc};

use crate::agents::types::BgAgentStatus;
use crate::app_state::AppState;

const MONITOR_INTERVAL: Duration = Duration::from_secs(60);
const STUCK_AFTER: TimeDelta = TimeDelta::minutes(20);

pub async fn run_background_agent_monitor(app: AppState, shutdown: Arc<AtomicBool>) {
    tracing::info!("Starting background agent monitor");
    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        if let Err(error) = monitor_once(app.clone()).await {
            tracing::warn!("background agent monitor error: {}", error);
        }
        tokio::select! {
            _ = tokio::time::sleep(MONITOR_INTERVAL) => {}
            _ = wait_for_shutdown(shutdown.clone()) => break,
        }
    }
}

pub(crate) async fn monitor_once(app: AppState) -> Result<(), String> {
    let now = Utc::now();
    for record in app.agents.list_all().await {
        if record.status == BgAgentStatus::Running
            && now.signed_duration_since(record.last_update_at) > STUCK_AFTER
        {
            let updated = app
                .agents
                .update_progress(
                    &record.agent_id,
                    "⚠ no activity for 20+ minutes".to_string(),
                    record.step_count,
                    record.last_activity.clone(),
                )
                .await?;
            crate::agents::spawn::emit_background_agent_update(app.clone(), &updated).await;
        }

        if record.status == BgAgentStatus::Running
            && !app.agents.has_runtime(&record.agent_id).await
            && child_session_missing(&app, record.child_chat_id.as_deref()).await
        {
            let updated = app
                .agents
                .mark_interrupted(
                    &record.agent_id,
                    "Engine restarted; child session lost".to_string(),
                )
                .await?;
            crate::agents::spawn::emit_background_agent_update(app.clone(), &updated).await;
            crate::agents::push::push_completion_to_parent(app.clone(), &updated).await?;
        }
    }

    for record in app
        .agents
        .list_with_completion_message_id(&["pending", "deferred"])
        .await
    {
        if crate::agents::push::should_retry_completion_push(&record) {
            crate::agents::push::push_completion_to_parent(app.clone(), &record).await?;
        }
    }

    Ok(())
}

async fn child_session_missing(app: &AppState, child_chat_id: Option<&str>) -> bool {
    let Some(child_chat_id) = child_chat_id else {
        return true;
    };
    let sessions = app.chat.sessions.read().await;
    !sessions.contains_key(child_chat_id)
}

async fn wait_for_shutdown(shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::types::{BgAgentKind, CreateAgentRequest};

    #[tokio::test]
    async fn monitor_marks_old_running_record_with_progress() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let (record, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "parent-monitor".to_string(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,
                kind: BgAgentKind::Delegate,
                config_name: "delegate_with_editing".to_string(),
                title: "title".to_string(),
                prompt: "prompt".to_string(),
                target_files: vec![],
                model: "model".to_string(),
            })
            .await
            .unwrap();
        let running = app
            .agents
            .mark_running(&record.agent_id, "missing-child".to_string())
            .await
            .unwrap();
        app.agents
            .set_last_update_at_for_test(&record.agent_id, Utc::now() - TimeDelta::minutes(25))
            .await
            .unwrap();
        let _ = running;

        monitor_once(app.clone()).await.unwrap();

        let updated = app
            .agents
            .get("parent-monitor", &record.agent_id)
            .await
            .unwrap();
        assert_eq!(updated.status, BgAgentStatus::Running);
        assert!(updated
            .progress
            .as_deref()
            .unwrap_or("")
            .contains("20+ minutes"));
    }

    #[tokio::test]
    async fn monitor_retries_cooled_deferred_completion() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session = Arc::new(tokio::sync::Mutex::new(
            crate::chat::types::ChatSession::new("parent-monitor-deferred".to_string()),
        ));
        session
            .lock()
            .await
            .queue_processor_running
            .store(true, Ordering::SeqCst);
        app.chat
            .sessions
            .write()
            .await
            .insert("parent-monitor-deferred".to_string(), session.clone());
        let (record, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "parent-monitor-deferred".to_string(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,
                kind: BgAgentKind::Delegate,
                config_name: "delegate_with_editing".to_string(),
                title: "title".to_string(),
                prompt: "prompt".to_string(),
                target_files: vec![],
                model: "model".to_string(),
            })
            .await
            .unwrap();
        app.agents
            .mark_completed(
                &record.agent_id,
                crate::agents::types::AgentCompletion {
                    result_summary: "done".to_string(),
                    edited_files: vec![],
                    diff_summary: None,
                    conflict_summary: None,
                    child_chat_id: Some("child-monitor-deferred".to_string()),
                },
            )
            .await
            .unwrap();
        app.agents
            .set_completion_message_id(&record.agent_id, "deferred".to_string())
            .await
            .unwrap();
        app.agents
            .set_deferred_at_for_test(&record.agent_id, Utc::now() - TimeDelta::seconds(11))
            .await
            .unwrap();

        monitor_once(app.clone()).await.unwrap();

        assert_eq!(session.lock().await.command_queue.len(), 1);
        let updated = app
            .agents
            .get("parent-monitor-deferred", &record.agent_id)
            .await
            .unwrap();
        assert_ne!(updated.completion_message_id.as_deref(), Some("deferred"));
        assert!(updated.completion_pushed_at.is_some());
        assert!(updated.deferred_at.is_none());
    }
}
