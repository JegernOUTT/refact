use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::tools::tools_description::{Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;
use crate::chat::types::SessionState;

pub(crate) async fn get_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    if let Some(id) = args.get("task_id").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }
    let ccx_lock = ccx.lock().await;
    if let Some(ref meta) = ccx_lock.task_meta {
        return Ok(meta.task_id.clone());
    }
    storage::infer_task_id_from_chat_id(&ccx_lock.chat_id)
        .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())
}

pub struct ToolTaskCheckAgents;

impl ToolTaskCheckAgents {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug)]
pub(crate) struct AgentStatus {
    pub(crate) card_id: String,
    pub(crate) card_title: String,
    pub(crate) agent_chat_id: String,
    pub(crate) column: String,
    pub(crate) session_state: Option<SessionState>,
    pub(crate) last_status_update: Option<String>,
    pub(crate) final_report: Option<String>,
}

pub(crate) async fn get_agent_statuses(
    gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
    task_id: &str,
) -> Result<Vec<AgentStatus>, String> {
    let board = storage::load_board(gcx.clone(), task_id).await?;

    let sessions = {
        let gcx_locked = gcx.read().await;
        gcx_locked.chat_sessions.clone()
    };

    let mut statuses = Vec::new();

    for card in &board.cards {
        if let Some(agent_chat_id) = &card.agent_chat_id {
            let session_arc = {
                let sessions_read = sessions.read().await;
                sessions_read.get(agent_chat_id).cloned()
            };

            let session_state = if let Some(sa) = session_arc {
                Some(sa.lock().await.runtime.state)
            } else {
                None
            };

            let last_status_update = card
                .status_updates
                .last()
                .map(|u| format!("{}: {}", u.timestamp, u.message));

            statuses.push(AgentStatus {
                card_id: card.id.clone(),
                card_title: card.title.clone(),
                agent_chat_id: agent_chat_id.clone(),
                column: card.column.clone(),
                session_state,
                last_status_update,
                final_report: card.final_report.clone(),
            });
        }
    }

    Ok(statuses)
}

pub(crate) fn format_agent_status(status: &AgentStatus) -> String {
    let (state_emoji, state_text) = match status.column.as_str() {
        "done" => ("✅", "Completed"),
        "failed" => ("❌", "Failed"),
        "doing" => match &status.session_state {
            Some(SessionState::Generating) => ("🔄", "Generating response"),
            Some(SessionState::ExecutingTools) => ("⚙️", "Executing tools"),
            Some(SessionState::Paused) => ("⏸️", "Paused (awaiting confirmation)"),
            Some(SessionState::WaitingIde) => ("⏳", "Waiting for IDE"),
            Some(SessionState::WaitingUserInput) => ("❓", "Waiting for user input"),
            Some(SessionState::Completed) => ("✅", "Completed"),
            Some(SessionState::Error) => ("⚠️", "Error state (will be marked as failed)"),
            Some(SessionState::Idle) => ("💤", "Idle (waiting)"),
            None => (
                "❓",
                "Unknown/offline (will be marked as failed if stuck too long)",
            ),
        },
        _ => ("❓", "Unknown"),
    };

    let mut result = format!(
        "### {} {} ({})\n**Status:** {} | **Column:** {}\n",
        state_emoji, status.card_title, status.card_id, state_text, status.column
    );

    result.push_str(&format!(
        "📎 [View Agent Chat](refact://chat/{})\n",
        status.agent_chat_id
    ));

    if let Some(report) = &status.final_report {
        let preview: String = report.chars().take(300).collect();
        let preview = if preview.len() < report.len() {
            format!("{}...", preview)
        } else {
            preview
        };
        result.push_str(&format!("\n**Final Report:**\n{}\n", preview));
    } else if let Some(update) = &status.last_status_update {
        result.push_str(&format!("\n**Last Update:** {}\n", update));
    }

    result
}

#[async_trait]
impl Tool for ToolTaskCheckAgents {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_check_agents".to_string(),
            display_name: "Task Check Agents".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Check the status of all spawned agents for a task. Shows their board status (primary) and live session state (if available). Agents mark themselves done via task_agent_finish(). Agents that fail (streaming errors, timeouts, stuck) are automatically marked as failed.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "task_id".to_string(),
                    param_type: "string".to_string(),
                    description: "Task ID (optional if chat is bound to a task)".to_string(),
                },
            ],
            parameters_required: vec![],
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let ccx_lock = ccx.lock().await;

        let is_planner = ccx_lock
            .task_meta
            .as_ref()
            .map(|m| m.role == "planner")
            .unwrap_or(false);

        if !is_planner {
            return Err("task_check_agents can only be called by the task planner. \
                 Switch to the planner chat to check agent status."
                .to_string());
        }

        drop(ccx_lock);

        let task_id = get_task_id(&ccx, args).await?;
        let gcx = ccx.lock().await.global_context.clone();

        let statuses = get_agent_statuses(gcx, &task_id).await?;

        if statuses.is_empty() {
            let result = "# Agent Status\n\nNo agents have been spawned yet for this task.\n\nUse `task_spawn_agent(card_id)` to spawn an agent for a card.".to_string();

            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(result),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })],
            ));
        }

        let running: Vec<_> = statuses.iter().filter(|s| s.column == "doing").collect();
        let completed: Vec<_> = statuses.iter().filter(|s| s.column == "done").collect();
        let failed: Vec<_> = statuses.iter().filter(|s| s.column == "failed").collect();

        let mut result = format!(
            "# Agent Status Summary\n\n**Total:** {} agents | 🔄 Running: {} | ✅ Done: {} | ❌ Failed: {}\n\n",
            statuses.len(), running.len(), completed.len(), failed.len()
        );

        if !running.is_empty() {
            result.push_str("## 🔄 Running\n\n");
            for status in &running {
                result.push_str(&format_agent_status(status));
                result.push_str("\n---\n\n");
            }
        }

        if !completed.is_empty() {
            result.push_str("## ✅ Completed\n\n");
            for status in &completed {
                result.push_str(&format_agent_status(status));
                result.push_str("\n---\n\n");
            }
        }

        if !failed.is_empty() {
            result.push_str("## ❌ Failed\n\n");
            for status in &failed {
                result.push_str(&format_agent_status(status));
                result.push_str("\n---\n\n");
            }
        }

        if running.is_empty() && !completed.is_empty() && failed.is_empty() {
            result.push_str("🎉 **All agents have completed successfully!**\n");
        } else if !failed.is_empty() {
            result.push_str(
                "⚠️ **Some agents have failed.** Review their reports and consider replanning.\n",
            );
        } else if !running.is_empty() {
            result.push_str("⏳ **Agents are still working.** Do not check again, wait for the completion message to arrive.\n");
        }

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
