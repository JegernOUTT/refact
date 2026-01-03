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
use crate::tasks::types::BoardCard;

fn make_source() -> ToolSource {
    ToolSource { source_type: ToolSourceType::Builtin, config_path: String::new() }
}

fn parse_depends_on(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(arr)) => {
            arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
        }
        Some(Value::String(s)) => {
            s.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        _ => vec![],
    }
}

async fn get_task_id(ccx: &Arc<AMutex<AtCommandsContext>>, args: &HashMap<String, Value>) -> Result<String, String> {
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

pub struct ToolTaskBoardGet;
pub struct ToolTaskBoardCreateCard;
pub struct ToolTaskBoardUpdateCard;
pub struct ToolTaskBoardMoveCard;
pub struct ToolTaskBoardDeleteCard;
pub struct ToolTaskReadyCards;
pub struct ToolTaskSetPlannerInstructions;

impl ToolTaskBoardGet { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for ToolTaskBoardGet {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx, args).await?;
        let gcx = ccx.lock().await.global_context.clone();
        let board = storage::load_board(gcx, &task_id).await?;
        let result = serde_yaml::to_string(&board).map_err(|e| e.to_string())?;

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
            name: "task_board_get".to_string(),
            display_name: "Task Board Get".to_string(),
            source: make_source(),
            agentic: true,
            experimental: false,
            description: "Get the current state of the task board including all columns and cards.".to_string(),
            parameters: vec![
                ToolParam { name: "task_id".to_string(), param_type: "string".to_string(), description: "Task UUID (optional if in task context)".to_string() }
            ],
            parameters_required: vec![],
        }
    }
}

impl ToolTaskBoardCreateCard { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for ToolTaskBoardCreateCard {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (is_planner, gcx) = {
            let ccx_lock = ccx.lock().await;
            let is_planner = ccx_lock.task_meta.as_ref()
                .map(|m| m.role == "planner")
                .unwrap_or(false);
            let gcx = ccx_lock.global_context.clone();
            (is_planner, gcx)
        };

        if !is_planner {
            return Err(
                "task_board_create_card can only be called by the task planner. \
                 Switch to the planner chat to create cards.".to_string()
            );
        }

        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args.get("card_id").and_then(|v| v.as_str()).ok_or("Missing 'card_id'")?;
        let title = args.get("title").and_then(|v| v.as_str()).ok_or("Missing 'title'")?;
        let priority = args.get("priority").and_then(|v| v.as_str()).unwrap_or("P1");
        let instructions = args.get("instructions").and_then(|v| v.as_str()).unwrap_or("");
        let depends_on: Vec<String> = parse_depends_on(args.get("depends_on"));
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;

        if board.cards.iter().any(|c| c.id == card_id) {
            return Err(format!("Card {} already exists", card_id));
        }

        board.cards.push(BoardCard {
            id: card_id.to_string(),
            title: title.to_string(),
            column: "planned".to_string(),
            priority: priority.to_string(),
            depends_on,
            instructions: instructions.to_string(),
            assignee: None,
            agent_chat_id: None,
            status_updates: vec![],
            final_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
        });
        board.rev += 1;

        storage::save_board(gcx.clone(), &task_id, &board).await?;
        storage::update_task_stats(gcx, &task_id).await?;

        let result = format!("Created card {} in Planned column", card_id);
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
            name: "task_board_create_card".to_string(),
            display_name: "Task Board Create Card".to_string(),
            source: make_source(),
            agentic: true,
            experimental: false,
            description: "Create a new card on the task board.".to_string(),
            parameters: vec![
                ToolParam { name: "card_id".to_string(), param_type: "string".to_string(), description: "Card ID (e.g., T-1, T-2)".to_string() },
                ToolParam { name: "title".to_string(), param_type: "string".to_string(), description: "Card title".to_string() },
                ToolParam { name: "priority".to_string(), param_type: "string".to_string(), description: "Priority: P0, P1, or P2".to_string() },
                ToolParam { name: "instructions".to_string(), param_type: "string".to_string(), description: "Detailed instructions for the agent".to_string() },
                ToolParam { name: "depends_on".to_string(), param_type: "string".to_string(), description: "Comma-separated list of card IDs this card depends on (e.g., \"T-1, T-2\")".to_string() },
            ],
            parameters_required: vec!["card_id".to_string(), "title".to_string()],
        }
    }
}

impl ToolTaskBoardUpdateCard { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for ToolTaskBoardUpdateCard {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (is_planner, gcx) = {
            let ccx_lock = ccx.lock().await;
            let is_planner = ccx_lock.task_meta.as_ref()
                .map(|m| m.role == "planner")
                .unwrap_or(false);
            let gcx = ccx_lock.global_context.clone();
            (is_planner, gcx)
        };

        if !is_planner {
            return Err(
                "task_board_update_card can only be called by the task planner. \
                 Switch to the planner chat to update cards.".to_string()
            );
        }

        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args.get("card_id").and_then(|v| v.as_str()).ok_or("Missing 'card_id'")?;
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;

        let card = board.get_card_mut(card_id).ok_or(format!("Card {} not found", card_id))?;

        if let Some(title) = args.get("title").and_then(|v| v.as_str()) {
            card.title = title.to_string();
        }
        if let Some(priority) = args.get("priority").and_then(|v| v.as_str()) {
            card.priority = priority.to_string();
        }
        if let Some(instructions) = args.get("instructions").and_then(|v| v.as_str()) {
            card.instructions = instructions.to_string();
        }
        if args.contains_key("depends_on") {
            card.depends_on = parse_depends_on(args.get("depends_on"));
        }

        board.rev += 1;
        storage::save_board(gcx, &task_id, &board).await?;

        let result = format!("Updated card {}", card_id);
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
            name: "task_board_update_card".to_string(),
            display_name: "Task Board Update Card".to_string(),
            source: make_source(),
            agentic: true,
            experimental: false,
            description: "Update an existing card's fields.".to_string(),
            parameters: vec![
                ToolParam { name: "card_id".to_string(), param_type: "string".to_string(), description: "Card ID to update".to_string() },
                ToolParam { name: "title".to_string(), param_type: "string".to_string(), description: "New title".to_string() },
                ToolParam { name: "priority".to_string(), param_type: "string".to_string(), description: "New priority".to_string() },
                ToolParam { name: "instructions".to_string(), param_type: "string".to_string(), description: "New instructions".to_string() },
                ToolParam { name: "depends_on".to_string(), param_type: "string".to_string(), description: "Comma-separated list of new dependencies (e.g., \"T-1, T-2\")".to_string() },
            ],
            parameters_required: vec!["card_id".to_string()],
        }
    }
}

impl ToolTaskBoardMoveCard { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for ToolTaskBoardMoveCard {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (is_planner, gcx) = {
            let ccx_lock = ccx.lock().await;
            let is_planner = ccx_lock.task_meta.as_ref()
                .map(|m| m.role == "planner")
                .unwrap_or(false);
            let gcx = ccx_lock.global_context.clone();
            (is_planner, gcx)
        };

        if !is_planner {
            return Err(
                "task_board_move_card can only be called by the task planner. \
                 Switch to the planner chat to move cards.".to_string()
            );
        }

        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args.get("card_id").and_then(|v| v.as_str()).ok_or("Missing 'card_id'")?;
        let column = args.get("column").and_then(|v| v.as_str()).ok_or("Missing 'column'")?;

        let valid_columns = ["planned", "doing", "done", "failed"];
        if !valid_columns.contains(&column) {
            return Err(format!("Invalid column: {}. Must be one of: {:?}", column, valid_columns));
        }
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;
        let now = Utc::now().to_rfc3339();

        let card = board.get_card_mut(card_id).ok_or(format!("Card {} not found", card_id))?;
        let old_column = card.column.clone();

        if column == "doing" && card.started_at.is_none() {
            card.started_at = Some(now.clone());
        }
        if (column == "done" || column == "failed") && card.completed_at.is_none() {
            card.completed_at = Some(now);
        }
        card.column = column.to_string();
        board.rev += 1;

        storage::save_board(gcx.clone(), &task_id, &board).await?;
        storage::update_task_stats(gcx, &task_id).await?;

        let result = format!("Moved card {} from {} to {}", card_id, old_column, column);
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
            name: "task_board_move_card".to_string(),
            display_name: "Task Board Move Card".to_string(),
            source: make_source(),
            agentic: true,
            experimental: false,
            description: "Move a card to a different column.".to_string(),
            parameters: vec![
                ToolParam { name: "card_id".to_string(), param_type: "string".to_string(), description: "Card ID to move".to_string() },
                ToolParam { name: "column".to_string(), param_type: "string".to_string(), description: "Target column: planned, doing, done, or failed".to_string() },
            ],
            parameters_required: vec!["card_id".to_string(), "column".to_string()],
        }
    }
}

impl ToolTaskBoardDeleteCard { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for ToolTaskBoardDeleteCard {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (is_planner, gcx) = {
            let ccx_lock = ccx.lock().await;
            let is_planner = ccx_lock.task_meta.as_ref()
                .map(|m| m.role == "planner")
                .unwrap_or(false);
            let gcx = ccx_lock.global_context.clone();
            (is_planner, gcx)
        };

        if !is_planner {
            return Err(
                "task_board_delete_card can only be called by the task planner. \
                 Switch to the planner chat to delete cards.".to_string()
            );
        }

        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args.get("card_id").and_then(|v| v.as_str()).ok_or("Missing 'card_id'")?;
        let mut board = storage::load_board(gcx.clone(), &task_id).await?;

        let existed = board.cards.iter().any(|c| c.id == card_id);
        if !existed {
            return Err(format!("Card {} not found", card_id));
        }

        board.cards.retain(|c| c.id != card_id);
        board.rev += 1;

        storage::save_board(gcx.clone(), &task_id, &board).await?;
        storage::update_task_stats(gcx, &task_id).await?;

        let result = format!("Deleted card {}", card_id);
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
            name: "task_board_delete_card".to_string(),
            display_name: "Task Board Delete Card".to_string(),
            source: make_source(),
            agentic: true,
            experimental: false,
            description: "Delete a card from the board.".to_string(),
            parameters: vec![
                ToolParam { name: "card_id".to_string(), param_type: "string".to_string(), description: "Card ID to delete".to_string() },
            ],
            parameters_required: vec!["card_id".to_string()],
        }
    }
}

impl ToolTaskReadyCards { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for ToolTaskReadyCards {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx, args).await?;

        let gcx = ccx.lock().await.global_context.clone();
        let board = storage::load_board(gcx, &task_id).await?;
        let ready = board.get_ready_cards();

        let result = format!(
            "Ready cards (can start in parallel): {:?}\nBlocked (waiting for dependencies): {:?}\nIn progress: {:?}\nCompleted: {:?}\nFailed: {:?}",
            ready.ready, ready.blocked, ready.in_progress, ready.completed, ready.failed
        );

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
            name: "task_ready_cards".to_string(),
            display_name: "Task Ready Cards".to_string(),
            source: make_source(),
            agentic: true,
            experimental: false,
            description: "Get cards that are ready to be worked on (all dependencies satisfied).".to_string(),
            parameters: vec![],
            parameters_required: vec![],
        }
    }
}

impl ToolTaskSetPlannerInstructions { pub fn new() -> Self { Self } }

#[async_trait]
impl Tool for ToolTaskSetPlannerInstructions {
    fn as_any(&self) -> &dyn std::any::Any { self }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx, args).await?;
        let content = args.get("content").and_then(|v| v.as_str()).ok_or("Missing 'content'")?;

        let gcx = ccx.lock().await.global_context.clone();
        storage::save_planner_instructions(gcx, &task_id, content).await?;

        let result = "Saved planner instructions".to_string();
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
            name: "task_set_planner_instructions".to_string(),
            display_name: "Task Set Planner Instructions".to_string(),
            source: make_source(),
            agentic: true,
            experimental: false,
            description: "Set the planner instructions for the task.".to_string(),
            parameters: vec![
                ToolParam { name: "content".to_string(), param_type: "string".to_string(), description: "Markdown content for planner guidance".to_string() },
            ],
            parameters_required: vec!["content".to_string()],
        }
    }
}
