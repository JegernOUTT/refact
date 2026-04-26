use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;
use chrono::Utc;

use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;

async fn get_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    if let Some(id) = args.get("task_id").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }
    let ccx_lock = ccx.lock().await;
    ccx_lock
        .task_meta
        .as_ref()
        .map(|m| m.task_id.clone())
        .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())
}

pub struct ToolTaskMarkCardDone;
pub struct ToolTaskMarkCardFailed;

impl ToolTaskMarkCardDone {
    pub fn new() -> Self {
        Self
    }
}

impl ToolTaskMarkCardFailed {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskMarkCardDone {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_mark_card_done".to_string(),
            display_name: "Task Mark Card Done".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Manually mark a card as done. Use this if an agent completed work but forgot to call task_agent_finish(), or to finalize a card after reviewing the agent's work.".to_string(),
            input_schema: json_schema_from_params(&[("card_id", "string", "Card ID to mark as done"), ("report", "string", "Summary/report for the completed card")], &["card_id", "report"]),
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
        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let report = args
            .get("report")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'report'")?;

        let gcx = ccx.lock().await.global_context.clone();

        let card_title = {
            let card_id_owned = card_id.to_string();
            let report_owned = report.to_string();

            let (board, _) = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                let card = board
                    .get_card_mut(&card_id_owned)
                    .ok_or(format!("Card {} not found", card_id_owned))?;

                if card.column == "done" {
                    return Err(format!("Card {} is already done", card_id_owned));
                }

                card.final_report = Some(report_owned.clone());
                card.column = "done".to_string();
                card.completed_at = Some(Utc::now().to_rfc3339());
                card.status_updates.push(StatusUpdate {
                    timestamp: Utc::now().to_rfc3339(),
                    message: "Manually marked as done by planner".to_string(),
                });
                Ok(())
            })
            .await?;

            storage::update_task_stats(gcx.clone(), &task_id).await?;
            board
                .get_card(card_id)
                .map(|c| c.title.clone())
                .unwrap_or_default()
        };

        let result = format!(
            "✅ **Card marked as done:** {}\n\n**Report:**\n{}",
            card_title, report
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolTaskMarkCardFailed {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_mark_card_failed".to_string(),
            display_name: "Task Mark Card Failed".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Manually mark a card as failed. Use this to resolve stuck agents, mark cards that cannot be completed, or when an agent errored without calling task_agent_finish().".to_string(),
            input_schema: json_schema_from_params(&[("card_id", "string", "Card ID to mark as failed"), ("reason", "string", "Reason for failure")], &["card_id", "reason"]),
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
        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'reason'")?;

        let gcx = ccx.lock().await.global_context.clone();

        let card_title = {
            let card_id_owned = card_id.to_string();
            let reason_owned = reason.to_string();

            let (board, _) = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                let card = board
                    .get_card_mut(&card_id_owned)
                    .ok_or(format!("Card {} not found", card_id_owned))?;

                if card.column == "failed" {
                    return Err(format!("Card {} is already failed", card_id_owned));
                }

                card.final_report = Some(format!("FAILED: {}", reason_owned));
                card.column = "failed".to_string();
                card.completed_at = Some(Utc::now().to_rfc3339());
                card.status_updates.push(StatusUpdate {
                    timestamp: Utc::now().to_rfc3339(),
                    message: format!("Manually marked as failed: {}", reason_owned),
                });
                Ok(())
            })
            .await?;

            storage::update_task_stats(gcx.clone(), &task_id).await?;
            board
                .get_card(card_id)
                .map(|c| c.title.clone())
                .unwrap_or_default()
        };

        let result = format!(
            "❌ **Card marked as failed:** {}\n\n**Reason:**\n{}",
            card_title, reason
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
