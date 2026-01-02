use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use chrono::Utc;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType};
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;

fn make_source() -> ToolSource {
    ToolSource { source_type: ToolSourceType::Builtin, config_path: String::new() }
}

async fn get_task_id(ccx: &Arc<AMutex<AtCommandsContext>>, args: &HashMap<String, Value>) -> Result<String, String> {
    if let Some(id) = args.get("task_id").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }
    let ccx_lock = ccx.lock().await;
    ccx_lock.task_meta.as_ref()
        .map(|m| m.task_id.clone())
        .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())
}

pub struct ToolTaskAgentUpdate;
pub struct ToolTaskAgentComplete;
pub struct ToolTaskAgentFail;
pub struct ToolTaskAssignAgent;

impl ToolTaskAgentUpdate { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for ToolTaskAgentUpdate {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args.get("card_id").and_then(|v| v.as_str()).ok_or("Missing 'card_id'")?;
        let message = args.get("message").and_then(|v| v.as_str()).ok_or("Missing 'message'")?;

        let gcx = ccx.lock().await.global_context.clone();
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;

        let card = board.get_card_mut(card_id).ok_or(format!("Card {} not found", card_id))?;
        card.status_updates.push(StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: message.to_string(),
        });
        board.rev += 1;

        storage::save_board(gcx, &task_id, &board).await?;

        let result = format!("Added status update to card {}", card_id);
        Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })]))
    }

    fn tool_depends_on(&self) -> Vec<String> { vec![] }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_agent_update".to_string(),
            display_name: "Task Agent Update".to_string(),
            source: make_source(),
            agentic: true,
            experimental: false,
            description: "Add a progress update to the assigned card.".to_string(),
            parameters: vec![
                ToolParam { name: "card_id".to_string(), param_type: "string".to_string(), description: "Card ID".to_string() },
                ToolParam { name: "message".to_string(), param_type: "string".to_string(), description: "Progress message".to_string() },
            ],
            parameters_required: vec!["card_id".to_string(), "message".to_string()],
        }
    }
}

impl ToolTaskAgentComplete { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for ToolTaskAgentComplete {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args.get("card_id").and_then(|v| v.as_str()).ok_or("Missing 'card_id'")?;
        let report = args.get("final_report").and_then(|v| v.as_str()).ok_or("Missing 'final_report'")?;

        let gcx = ccx.lock().await.global_context.clone();
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;
        let now = Utc::now().to_rfc3339();

        let card = board.get_card_mut(card_id).ok_or(format!("Card {} not found", card_id))?;
        card.final_report = Some(report.to_string());
        card.column = "done".to_string();
        card.completed_at = Some(now);
        board.rev += 1;

        storage::save_board(gcx.clone(), &task_id, &board).await?;
        storage::update_task_stats(gcx, &task_id).await?;

        let result = format!("Completed card {} and moved to Done", card_id);
        Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })]))
    }

    fn tool_depends_on(&self) -> Vec<String> { vec![] }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_agent_complete".to_string(),
            display_name: "Task Agent Complete".to_string(),
            source: make_source(),
            agentic: true,
            experimental: false,
            description: "Mark the assigned card as complete with a final report.".to_string(),
            parameters: vec![
                ToolParam { name: "card_id".to_string(), param_type: "string".to_string(), description: "Card ID".to_string() },
                ToolParam { name: "final_report".to_string(), param_type: "string".to_string(), description: "Summary of what was done, decisions made, files modified".to_string() },
            ],
            parameters_required: vec!["card_id".to_string(), "final_report".to_string()],
        }
    }
}

impl ToolTaskAgentFail { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for ToolTaskAgentFail {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args.get("card_id").and_then(|v| v.as_str()).ok_or("Missing 'card_id'")?;
        let reason = args.get("reason").and_then(|v| v.as_str()).ok_or("Missing 'reason'")?;

        let gcx = ccx.lock().await.global_context.clone();
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;
        let now = Utc::now().to_rfc3339();

        let card = board.get_card_mut(card_id).ok_or(format!("Card {} not found", card_id))?;
        card.final_report = Some(format!("FAILED: {}", reason));
        card.column = "failed".to_string();
        card.completed_at = Some(now);
        board.rev += 1;

        storage::save_board(gcx.clone(), &task_id, &board).await?;
        storage::update_task_stats(gcx, &task_id).await?;

        let result = format!("Marked card {} as failed and moved to Failed column", card_id);
        Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })]))
    }

    fn tool_depends_on(&self) -> Vec<String> { vec![] }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_agent_fail".to_string(),
            display_name: "Task Agent Fail".to_string(),
            source: make_source(),
            agentic: true,
            experimental: false,
            description: "Mark the assigned card as failed with an explanation.".to_string(),
            parameters: vec![
                ToolParam { name: "card_id".to_string(), param_type: "string".to_string(), description: "Card ID".to_string() },
                ToolParam { name: "reason".to_string(), param_type: "string".to_string(), description: "Why the task failed".to_string() },
            ],
            parameters_required: vec!["card_id".to_string(), "reason".to_string()],
        }
    }
}

impl ToolTaskAssignAgent { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for ToolTaskAssignAgent {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args.get("card_id").and_then(|v| v.as_str()).ok_or("Missing 'card_id'")?;
        let agent_id = args.get("agent_id").and_then(|v| v.as_str()).ok_or("Missing 'agent_id'")?;
        let agent_chat_id = args.get("agent_chat_id").and_then(|v| v.as_str()).ok_or("Missing 'agent_chat_id'")?;

        let gcx = ccx.lock().await.global_context.clone();
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;
        let now = Utc::now().to_rfc3339();

        let card = board.get_card_mut(card_id).ok_or(format!("Card {} not found", card_id))?;
        card.assignee = Some(agent_id.to_string());
        card.agent_chat_id = Some(agent_chat_id.to_string());
        if card.started_at.is_none() {
            card.started_at = Some(now);
        }
        if card.column == "planned" {
            card.column = "doing".to_string();
        }
        board.rev += 1;

        storage::save_board(gcx.clone(), &task_id, &board).await?;
        storage::update_task_stats(gcx, &task_id).await?;

        let result = format!("Assigned agent {} to card {} (chat: {})", agent_id, card_id, agent_chat_id);
        Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })]))
    }

    fn tool_depends_on(&self) -> Vec<String> { vec![] }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_assign_agent".to_string(),
            display_name: "Task Assign Agent".to_string(),
            source: make_source(),
            agentic: true,
            experimental: false,
            description: "Assign an agent to a card and move it to Doing.".to_string(),
            parameters: vec![
                ToolParam { name: "card_id".to_string(), param_type: "string".to_string(), description: "Card ID to assign".to_string() },
                ToolParam { name: "agent_id".to_string(), param_type: "string".to_string(), description: "Agent UUID".to_string() },
                ToolParam { name: "agent_chat_id".to_string(), param_type: "string".to_string(), description: "Agent chat/trajectory ID".to_string() },
            ],
            parameters_required: vec!["card_id".to_string(), "agent_id".to_string(), "agent_chat_id".to_string()],
        }
    }
}
