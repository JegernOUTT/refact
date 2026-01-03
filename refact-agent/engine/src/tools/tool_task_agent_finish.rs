use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;
use chrono::Utc;

use crate::tools::tools_description::{Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;
use crate::chat::get_or_create_session_with_trajectory;

async fn get_task_id(ccx: &Arc<AMutex<AtCommandsContext>>) -> Result<String, String> {
    let ccx_lock = ccx.lock().await;
    ccx_lock.task_meta.as_ref()
        .map(|m| m.task_id.clone())
        .ok_or_else(|| "This tool can only be used by task agents (chat not bound to a task)".to_string())
}

async fn get_card_id(ccx: &Arc<AMutex<AtCommandsContext>>) -> Result<String, String> {
    let ccx_lock = ccx.lock().await;
    ccx_lock.task_meta.as_ref()
        .and_then(|m| m.card_id.clone())
        .ok_or_else(|| "This tool can only be used by task agents (no card_id in task_meta)".to_string())
}

pub struct ToolTaskAgentFinish;

impl ToolTaskAgentFinish {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Tool for ToolTaskAgentFinish {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_agent_finish".to_string(),
            display_name: "Task Agent Finish".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            agentic: false,
            experimental: false,
            description: "Mark the current card as completed or failed. Task agents MUST call this exactly once when finished. This updates the task board and notifies the planner.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "success".to_string(),
                    param_type: "boolean".to_string(),
                    description: "true if the card was completed successfully, false if it failed".to_string(),
                },
                ToolParam {
                    name: "report".to_string(),
                    param_type: "string".to_string(),
                    description: "Summary of what was done (if success) or why it failed (if failure)".to_string(),
                },
            ],
            parameters_required: vec!["success".to_string(), "report".to_string()],
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx).await?;
        let card_id = get_card_id(&ccx).await?;

        let success = match args.get("success") {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s.to_lowercase() == "true",
            _ => return Err("Missing or invalid 'success' parameter (must be boolean)".to_string()),
        };

        let report = args.get("report")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'report' parameter")?
            .to_string();

        let gcx = ccx.lock().await.global_context.clone();

        let card_id_owned = card_id.clone();
        let report_clone = report.clone();
        let success_clone = success;

        let (_board, (card_title, agent_branch, all_finished)) = storage::update_board_atomic(
            gcx.clone(),
            &task_id,
            move |board| {
                let card = board.get_card_mut(&card_id_owned)
                    .ok_or(format!("Card {} not found in task", card_id_owned))?;

                if card.column == "done" || card.column == "failed" {
                    return Err(format!(
                        "Card {} is already in '{}' column. Cannot finish twice.",
                        card_id_owned, card.column
                    ));
                }

                let card_title = card.title.clone();
                let agent_branch = card.agent_branch.clone();

                if success_clone {
                    card.final_report = Some(report_clone.clone());
                    card.column = "done".to_string();
                    card.completed_at = Some(Utc::now().to_rfc3339());
                    card.status_updates.push(StatusUpdate {
                        timestamp: Utc::now().to_rfc3339(),
                        message: "Agent completed successfully".to_string(),
                    });
                } else {
                    card.final_report = Some(format!("FAILED: {}", report_clone));
                    card.column = "failed".to_string();
                    card.completed_at = Some(Utc::now().to_rfc3339());
                    card.status_updates.push(StatusUpdate {
                        timestamp: Utc::now().to_rfc3339(),
                        message: format!("Agent failed: {}", report_clone),
                    });
                }

                let agents_active = board.cards.iter()
                    .filter(|c| c.column == "doing" && c.assignee.is_some())
                    .count();
                let all_finished = agents_active == 0;

                Ok((card_title, agent_branch, all_finished))
            },
        ).await?;

        storage::update_task_stats(gcx.clone(), &task_id).await?;

        let result_message = if success {
            format!(
                "✅ **Card Completed: {}**\n\n**Report:**\n{}\n\nThe planner will be notified of completion.",
                card_title, report
            )
        } else {
            format!(
                "❌ **Card Failed: {}**\n\n**Reason:**\n{}\n\nThe planner will be notified of the failure.",
                card_title, report
            )
        };

        let report_preview: String = report.chars().take(100).collect();
        tracing::info!(
            "Agent finished card {} ({}): {}",
            card_id,
            if success { "success" } else { "failed" },
            report_preview
        );

        let status_str = if success { "success" } else { "failed" };
        let branch_str = agent_branch.as_deref().unwrap_or("(no branch)");

        let mut planner_message = format!(
            "Agent finished card {}:\n**Card:** {}\n**Status:** {}\n**Branch:** {}\n**Report:** {}",
            card_id, card_title, status_str, branch_str, report_preview
        );

        if all_finished {
            planner_message.push_str(
                "\n\n✅ **All agents have completed.** Run `task_check_agents` or `task_board_get` to review results."
            );
        }

        let sessions = {
            let gcx_locked = gcx.read().await;
            gcx_locked.chat_sessions.clone()
        };

        let planner_chat_id = storage::get_planner_chat_id(gcx.clone(), &task_id).await?;
        let planner_session = get_or_create_session_with_trajectory(gcx.clone(), &sessions, &planner_chat_id).await;

        {
            let mut session = planner_session.lock().await;
            session.add_message(ChatMessage {
                role: "system".to_string(),
                content: ChatContent::SimpleText(planner_message),
                ..Default::default()
            });
        }

        Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result_message),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })]))
    }

    fn tool_depends_on(&self) -> Vec<String> { vec![] }
}
