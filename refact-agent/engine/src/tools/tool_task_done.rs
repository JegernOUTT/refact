use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tracing::error;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::memories::{memories_add_enriched, EnrichmentParams};
use crate::http::routers::v1::sidebar::NotificationEvent;

fn spawn_memory_enrichment_task(
    ccx: Arc<AMutex<AtCommandsContext>>,
    report: String,
    summary: String,
    files_changed: Vec<String>,
    root_chat_id: String,
) {
    tokio::spawn(async move {
        let enrichment_params = EnrichmentParams {
            base_tags: vec!["task-report".to_string(), "completed".to_string()],
            base_filenames: files_changed,
            base_kind: "task-report".to_string(),
            base_title: Some(summary.clone()),
            source_chat_id: (!root_chat_id.is_empty()).then_some(root_chat_id),
        };

        match memories_add_enriched(ccx, &report, enrichment_params).await {
            Ok(knowledge_path) => {
                tracing::info!("task_done: knowledge saved to {}", knowledge_path.display());
                tracing::info!(
                    "task_done: to load full content later, use cat(paths=\"{}\")",
                    knowledge_path.display()
                );
            }
            Err(e) => {
                error!("task_done: failed to save knowledge: {}", e);
            }
        }
    });
}

pub struct ToolTaskDone {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolTaskDone {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_done".to_string(),
            display_name: "Task Done".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Mark the current task as complete with a detailed report. Automatically saves to knowledge base. Use as the FINAL action when a task is fully completed.".to_string(),
            input_schema: json_schema_from_params(&[("report", "string", "Detailed markdown report of what was accomplished"), ("summary", "string", "One-line summary for notifications and titles"), ("files_changed", "string", "Comma-separated list or JSON array of file paths that were modified")], &["report", "summary"]),
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
        let report = match args.get("report") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `report` is not a string: {:?}", v)),
            None => return Err("argument `report` is missing".to_string()),
        };

        let summary = match args.get("summary") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `summary` is not a string: {:?}", v)),
            None => return Err("argument `summary` is missing".to_string()),
        };

        let files_changed: Vec<String> = match args.get("files_changed") {
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            Some(Value::String(s)) => {
                let trimmed = s.trim();
                if trimmed.starts_with('[') {
                    serde_json::from_str::<Vec<String>>(trimmed).unwrap_or_else(|_| {
                        s.split(',')
                            .map(|f| f.trim().to_string())
                            .filter(|f| !f.is_empty())
                            .collect()
                    })
                } else {
                    s.split(',')
                        .map(|f| f.trim().to_string())
                        .filter(|f| !f.is_empty())
                        .collect()
                }
            }
            _ => vec![],
        };

        let (root_chat_id, chat_id, abort_flag, gcx) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.root_chat_id.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.abort_flag.clone(),
                ccx_lock.global_context.clone(),
            )
        };

        abort_flag.store(true, Ordering::SeqCst);

        {
            let gcx_read = gcx.read().await;
            if let Some(ref tx) = gcx_read.notification_events_tx {
                let _ = tx.send(NotificationEvent::TaskDone {
                    chat_id: chat_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                    summary: summary.clone(),
                    knowledge_path: None,
                });
            }
        }

        spawn_memory_enrichment_task(
            ccx.clone(),
            report.clone(),
            summary.clone(),
            files_changed.clone(),
            root_chat_id.clone(),
        );

        let result = serde_json::json!({
            "type": "task_done",
            "summary": summary,
            "report": report,
            "files_changed": files_changed,
        });

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result.to_string()),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec!["knowledge".to_string()]
    }
}
