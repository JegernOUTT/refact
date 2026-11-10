use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ContextEnum};
use super::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const NOT_FOUND: &str = "tool job not found: unknown id, or its record is no longer kept (engine restart or retention limit)";

pub struct ToolToolJob {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolToolJob {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "tool_job".into(), display_name: "Tool job".into(),
            source: ToolSource { source_type: ToolSourceType::Builtin, config_path: self.config_path.clone() },
            experimental: false, allow_parallel: true,
            description: "Inspect or cooperatively cancel a runtime-owned tool invocation in this chat. status never returns payload. result returns the exact native terminal messages after current authority/privacy checks. wait is bounded and never cancels or replays on timeout. cancel_requested is not proof of quiescence.".into(),
            input_schema: json!({"type":"object","additionalProperties":false,"properties":{"action":{"enum":["status","result","cancel","wait"]},"job_id":{"type":"string","minLength":1},"timeout_ms":{"type":"integer","minimum":0,"maximum":30000}},"required":["action","job_id"]}),
            output_schema: None, annotations: None,
        }
    }
    async fn tool_execute(
        &mut self,
        ccx: Arc<Mutex<AtCommandsContext>>,
        call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or("action required")?;
        if !matches!(action, "status" | "result" | "cancel" | "wait") {
            return Err("invalid tool_job action".into());
        }
        let job_id = args
            .get("job_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or("job_id required")?;
        let (app, chat_id) = {
            let context = ccx.lock().await;
            (context.app.clone(), context.chat_id.clone())
        };
        // Status and cancellation return no retained payload. Guessing another
        // chat's id does not grant access; get/cancel compare the owner.
        let mut snapshot = app
            .runtime
            .tool_jobs
            .get(&chat_id, job_id)
            .ok_or(NOT_FOUND)?;
        if action == "cancel" {
            app.runtime
                .tool_jobs
                .cancel(&chat_id, job_id)
                .ok_or(NOT_FOUND)?;
            snapshot = app
                .runtime
                .tool_jobs
                .get(&chat_id, job_id)
                .ok_or(NOT_FOUND)?;
        }
        if action == "wait" {
            let timeout = args
                .get("timeout_ms")
                .map(|v| v.as_u64().ok_or("timeout_ms must be an integer"))
                .transpose()?
                .unwrap_or(1000);
            if timeout > 30000 {
                return Err("timeout_ms exceeds 30000".into());
            }
            let mut events = app.runtime.tool_jobs.subscribe();
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);
            loop {
                snapshot = app
                    .runtime
                    .tool_jobs
                    .get(&chat_id, job_id)
                    .ok_or(NOT_FOUND)?;
                if snapshot.result.is_some() || snapshot.status.is_terminal() {
                    break;
                }
                match tokio::time::timeout_at(deadline, events.recv()).await {
                    Ok(Ok(event)) if event.job_id == job_id => {}
                    Ok(Ok(_)) | Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                        continue
                    }
                    _ => break,
                }
            }
        }
        if action == "result" {
            snapshot =
                crate::chat::tool_job_delivery::checked_snapshot(&app, &chat_id, job_id).await?;
            if snapshot.result.is_none() && snapshot.result_released {
                let meta = crate::chat::tool_job_delivery::metadata(
                    &snapshot.receipt,
                    snapshot.status,
                    snapshot.revision,
                    false,
                );
                let mut message =
                    ChatMessage::new("tool".into(), crate::tool_jobs::RESULT_NOT_KEPT.into());
                message.tool_call_id = call_id.clone();
                message.extra.insert("tool_job".into(), meta);
                return Ok((false, vec![ContextEnum::ChatMessage(message)]));
            }
            if let Some(result) = snapshot.result {
                let meta = crate::chat::tool_job_delivery::metadata(
                    &snapshot.receipt,
                    snapshot.status,
                    snapshot.revision,
                    true,
                );
                let mut messages = result.messages;
                if messages.is_empty() {
                    messages.push(ChatMessage::new("tool".into(), "Terminal result has no payload; effects may be unknown. Do not replay automatically.".into()));
                }
                for message in &mut messages {
                    if message.role == "tool" {
                        message.tool_call_id = call_id.clone();
                    }
                    message.extra.insert("tool_job".into(), meta.clone());
                }
                let mut output: Vec<_> =
                    messages.into_iter().map(ContextEnum::ChatMessage).collect();
                output.extend(
                    result
                        .context_files
                        .into_iter()
                        .map(ContextEnum::ContextFile),
                );
                return Ok((result.had_corrections, output));
            }
        }
        let meta = crate::chat::tool_job_delivery::metadata(
            &snapshot.receipt,
            snapshot.status,
            snapshot.revision,
            snapshot.result.is_some(),
        );
        let mut message = ChatMessage::new("tool".into(), meta.to_string());
        message.tool_call_id = call_id.clone();
        message.extra.insert("tool_job".into(), meta);
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }
    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}
