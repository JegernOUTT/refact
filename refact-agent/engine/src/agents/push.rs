use chrono::{TimeDelta, Utc};
use uuid::Uuid;

use crate::agents::types::{BackgroundAgent, BgAgentKind, BgAgentStatus};
use crate::app_state::AppState;
use crate::chat::process_command_queue;
use crate::chat::types::{BurstGuardDecision, ChatCommand, CommandRequest};

const SUMMARY_LIMIT: usize = 1500;
const DEFERRED_RETRY_AFTER: TimeDelta = TimeDelta::seconds(10);

pub async fn push_completion_to_parent(
    app: AppState,
    record: &BackgroundAgent,
) -> Result<(), String> {
    if already_pushed(record) {
        return Ok(());
    }

    let session_arc = {
        let sessions = app.chat.sessions.read().await;
        sessions.get(&record.parent_chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        app.agents
            .set_completion_message_id(&record.agent_id, "pending".to_string())
            .await?;
        return Ok(());
    };

    let message_id = Uuid::new_v4().to_string();
    let processor_flag = {
        let mut session = session_arc.lock().await;
        if session.closed {
            app.agents
                .set_completion_message_id(&record.agent_id, "pending".to_string())
                .await?;
            return Ok(());
        }
        match session.background_completion_burst.record_and_check().await {
            BurstGuardDecision::Allow => {}
            BurstGuardDecision::Defer => {
                app.agents
                    .set_completion_message_id(&record.agent_id, "deferred".to_string())
                    .await?;
                return Ok(());
            }
        }
        session.command_queue.push_back(CommandRequest {
            client_request_id: format!("background-agent-finished-{}", message_id),
            priority: true,
            command: ChatCommand::UserMessage {
                content: serde_json::Value::String(build_push_message(record)),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        session.emit_queue_update();
        session.touch();
        session.queue_notify.notify_one();
        session.queue_processor_running.clone()
    };

    app.agents
        .set_completion_message_id(&record.agent_id, message_id)
        .await?;

    if !processor_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
        tokio::spawn(process_command_queue(app, session_arc, processor_flag));
    }

    Ok(())
}

pub async fn flush_pending_pushes_for_parent(
    app: AppState,
    parent_chat_id: &str,
) -> Result<usize, String> {
    let records = app
        .agents
        .list_for_parent(
            parent_chat_id,
            crate::agents::types::AgentListFilter::default(),
        )
        .await;
    let mut pushed = 0usize;
    for record in records {
        if should_retry_completion_push(&record) {
            push_completion_to_parent(app.clone(), &record).await?;
            pushed += 1;
        }
    }
    Ok(pushed)
}

pub(crate) fn should_retry_completion_push(record: &BackgroundAgent) -> bool {
    match record.completion_message_id.as_deref() {
        Some("pending") => true,
        Some("deferred") => record
            .deferred_at
            .map(|deferred_at| deferred_at <= Utc::now() - DEFERRED_RETRY_AFTER)
            .unwrap_or(false),
        _ => false,
    }
}

fn already_pushed(record: &BackgroundAgent) -> bool {
    record
        .completion_message_id
        .as_deref()
        .map(|id| id != "pending" && id != "deferred")
        .unwrap_or(false)
}

fn build_push_message(record: &BackgroundAgent) -> String {
    let noun = match record.kind {
        BgAgentKind::Subagent => "subagent",
        BgAgentKind::Delegate => "delegate",
    };
    let mut lines = vec![
        format!("[background {} finished]", noun),
        format!("agent_id: {}", record.agent_id),
        format!("status: {}", record.status.as_str()),
        format!("title: {}", record.title),
        format!(
            "child_chat_id: {}",
            record.child_chat_id.as_deref().unwrap_or("")
        ),
        String::new(),
    ];
    if !record.target_files.is_empty() {
        lines.push("Target files:".to_string());
        lines.extend(record.target_files.iter().map(|file| format!("- {}", file)));
        lines.push(String::new());
    }
    if record.kind == BgAgentKind::Delegate && !record.edited_files.is_empty() {
        lines.push("Edited files:".to_string());
        lines.extend(record.edited_files.iter().map(|file| format!("- {}", file)));
        lines.push(String::new());
    }

    match record.status {
        BgAgentStatus::Completed => {
            lines.push("Summary:".to_string());
            lines.push(truncate_chars(
                record.result_summary.as_deref().unwrap_or(""),
                SUMMARY_LIMIT,
            ));
        }
        BgAgentStatus::Failed => {
            lines.push("Error:".to_string());
            lines.push(truncate_chars(
                record.error.as_deref().unwrap_or(""),
                SUMMARY_LIMIT,
            ));
        }
        BgAgentStatus::Cancelled => {
            lines.push("Reason:".to_string());
            lines.push(truncate_chars(
                record.error.as_deref().unwrap_or("cancelled"),
                SUMMARY_LIMIT,
            ));
        }
        BgAgentStatus::Interrupted => {
            lines.push(
                "Note: engine restarted before this agent finished. State is preserved; the change cannot be auto-resumed."
                    .to_string(),
            );
        }
        BgAgentStatus::Queued | BgAgentStatus::Running | BgAgentStatus::WaitingForApproval => {
            lines.push("Summary:".to_string());
            lines.push(truncate_chars(
                record.progress.as_deref().unwrap_or(""),
                SUMMARY_LIMIT,
            ));
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "(call agent_result(agent_id=\"{}\") for the full report, or open the child trajectory)",
        record.agent_id
    ));
    lines.join("\n")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::types::{BgAgentKind, CreateAgentRequest};

    async fn completed_record(kind: BgAgentKind) -> BackgroundAgent {
        let temp = tempfile::tempdir().unwrap();
        let registry =
            crate::agents::registry::BackgroundAgentRegistry::new(temp.path().to_path_buf())
                .await
                .unwrap();
        let (record, _, _) = registry
            .create(CreateAgentRequest {
                parent_chat_id: "parent".to_string(),
                parent_root_chat_id: None,
                parent_tool_call_id: None,
                kind,
                config_name: kind.as_str().to_string(),
                title: "Fix retry parsing".to_string(),
                prompt: "prompt".to_string(),
                target_files: vec!["src/auth/retry.ts".to_string()],
                model: "model".to_string(),
            })
            .await
            .unwrap();
        registry
            .mark_completed(
                &record.agent_id,
                crate::agents::types::AgentCompletion {
                    result_summary: "done".to_string(),
                    edited_files: vec!["src/auth/retry.ts".to_string()],
                    diff_summary: None,
                    conflict_summary: None,
                    child_chat_id: Some("child".to_string()),
                },
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn delegate_push_message_includes_edited_files() {
        let record = completed_record(BgAgentKind::Delegate).await;
        let message = build_push_message(&record);
        assert!(message.contains("[background delegate finished]"));
        assert!(message.contains("Edited files:"));
        assert!(message.contains("src/auth/retry.ts"));
    }

    #[tokio::test]
    async fn subagent_push_message_drops_edited_files() {
        let record = completed_record(BgAgentKind::Subagent).await;
        let message = build_push_message(&record);
        assert!(message.contains("[background subagent finished]"));
        assert!(!message.contains("Edited files:"));
    }
}
