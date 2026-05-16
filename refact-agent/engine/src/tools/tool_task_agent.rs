use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use chrono::Utc;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::chat::types::TaskMeta;
use crate::global_context::SharedGlobalContext;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskAgentScope {
    task_id: String,
    card_id: Option<String>,
}

fn make_source() -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: String::new(),
    }
}

fn resolve_task_agent_scope(
    task_meta: Option<&TaskMeta>,
    chat_id: &str,
    args: &HashMap<String, Value>,
    require_card_id: bool,
    require_bound_card_id: bool,
) -> Result<TaskAgentScope, String> {
    let supplied_task_id = args.get("task_id").and_then(|v| v.as_str());
    let supplied_card_id = args.get("card_id").and_then(|v| v.as_str());

    if let Some(meta) = task_meta {
        if let Some(task_id) = supplied_task_id {
            if task_id != meta.task_id {
                return Err(format!(
                    "Supplied task_id '{}' does not match bound task_id '{}'",
                    task_id, meta.task_id
                ));
            }
        }

        let card_id = if require_bound_card_id {
            let bound_card_id = meta
                .card_id
                .as_deref()
                .ok_or_else(|| "Task context is not bound to a card".to_string())?;
            if let Some(card_id) = supplied_card_id {
                if card_id != bound_card_id {
                    return Err(format!(
                        "Supplied card_id '{}' does not match bound card_id '{}'",
                        card_id, bound_card_id
                    ));
                }
            }
            Some(bound_card_id.to_string())
        } else if let Some(bound_card_id) = meta.card_id.as_deref() {
            if let Some(card_id) = supplied_card_id {
                if card_id != bound_card_id {
                    return Err(format!(
                        "Supplied card_id '{}' does not match bound card_id '{}'",
                        card_id, bound_card_id
                    ));
                }
            }
            Some(bound_card_id.to_string())
        } else if let Some(card_id) = supplied_card_id {
            Some(card_id.to_string())
        } else if require_card_id {
            return Err("Missing 'card_id'".to_string());
        } else {
            None
        };

        return Ok(TaskAgentScope {
            task_id: meta.task_id.clone(),
            card_id,
        });
    }

    let task_id = if let Some(task_id) = supplied_task_id {
        task_id.to_string()
    } else {
        storage::infer_task_id_from_chat_id(chat_id)
            .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())?
    };

    let card_id = if let Some(card_id) = supplied_card_id {
        Some(card_id.to_string())
    } else if require_card_id {
        return Err("Missing 'card_id'".to_string());
    } else {
        None
    };

    Ok(TaskAgentScope { task_id, card_id })
}

async fn resolve_scope_from_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
    require_card_id: bool,
    require_bound_card_id: bool,
) -> Result<TaskAgentScope, String> {
    let (chat_id, task_meta) = {
        let ccx_lock = ccx.lock().await;
        (ccx_lock.chat_id.clone(), ccx_lock.task_meta.clone())
    };
    resolve_task_agent_scope(
        task_meta.as_ref(),
        &chat_id,
        args,
        require_card_id,
        require_bound_card_id,
    )
}

async fn enforce_bound_scope(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
    require_card_id: bool,
    require_bound_card_id: bool,
) -> Result<(), String> {
    let (chat_id, task_meta) = {
        let ccx_lock = ccx.lock().await;
        (ccx_lock.chat_id.clone(), ccx_lock.task_meta.clone())
    };
    if task_meta.is_some() {
        resolve_task_agent_scope(
            task_meta.as_ref(),
            &chat_id,
            args,
            require_card_id,
            require_bound_card_id,
        )?;
    }
    Ok(())
}

fn legacy_finish_tool_error(tool_name: &str) -> String {
    format!(
        "{} is deprecated and cannot update task boards. Use task_agent_finish instead.",
        tool_name
    )
}

async fn get_global_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
) -> SharedGlobalContext {
    ccx.lock().await.global_context.clone()
}

fn required_card_id(scope: &TaskAgentScope) -> Result<String, String> {
    scope
        .card_id
        .clone()
        .ok_or_else(|| "Missing 'card_id'".to_string())
}

pub struct ToolTaskAgentUpdate;
pub struct ToolTaskAgentComplete;
pub struct ToolTaskAgentFail;
pub struct ToolTaskAssignAgent;

impl ToolTaskAgentUpdate {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskAgentUpdate {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let scope = resolve_scope_from_context(&ccx, args, true, true).await?;
        let card_id = required_card_id(&scope)?;
        let task_id = scope.task_id;
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'message'")?
            .to_string();

        let gcx = get_global_context(&ccx).await;
        let card_id_for_update = card_id.clone();
        storage::update_board_atomic(gcx, &task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_for_update)
                .ok_or(format!("Card {} not found", card_id_for_update))?;
            card.status_updates.push(StatusUpdate {
                timestamp: Utc::now().to_rfc3339(),
                message: message.clone(),
            });
            Ok(())
        })
        .await?;

        let result = format!("Added status update to card {}", card_id);
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

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_agent_update".to_string(),
            display_name: "Task Agent Update".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Add a progress update to the assigned card.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("card_id", "string", "Card ID"),
                    ("message", "string", "Progress message"),
                ],
                &["card_id", "message"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

impl ToolTaskAgentComplete {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskAgentComplete {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        _tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        enforce_bound_scope(&ccx, args, true, true).await?;
        Err(legacy_finish_tool_error("task_agent_complete"))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_agent_complete".to_string(),
            display_name: "Task Agent Complete".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Deprecated. Always returns an error; use task_agent_finish instead."
                .to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("card_id", "string", "Card ID"),
                    (
                        "final_report",
                        "string",
                        "Summary of what was done, decisions made, files modified",
                    ),
                ],
                &["card_id", "final_report"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

impl ToolTaskAgentFail {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskAgentFail {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        _tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        enforce_bound_scope(&ccx, args, true, true).await?;
        Err(legacy_finish_tool_error("task_agent_fail"))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_agent_fail".to_string(),
            display_name: "Task Agent Fail".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Deprecated. Always returns an error; use task_agent_finish instead."
                .to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("card_id", "string", "Card ID"),
                    ("reason", "string", "Why the task failed"),
                ],
                &["card_id", "reason"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

impl ToolTaskAssignAgent {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskAssignAgent {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let scope = resolve_scope_from_context(&ccx, args, true, false).await?;
        let card_id = required_card_id(&scope)?;
        let task_id = scope.task_id;
        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'agent_id'")?
            .to_string();
        let agent_chat_id = args
            .get("agent_chat_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'agent_chat_id'")?
            .to_string();

        let gcx = get_global_context(&ccx).await;
        let card_id_for_update = card_id.clone();
        let agent_id_for_update = agent_id.clone();
        let agent_chat_id_for_update = agent_chat_id.clone();
        storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_for_update)
                .ok_or(format!("Card {} not found", card_id_for_update))?;
            card.assignee = Some(agent_id_for_update.clone());
            card.agent_chat_id = Some(agent_chat_id_for_update.clone());
            if card.started_at.is_none() {
                card.started_at = Some(Utc::now().to_rfc3339());
            }
            if card.column == "planned" {
                card.column = "doing".to_string();
            }
            Ok(())
        })
        .await?;
        storage::update_task_stats(gcx, &task_id).await?;

        let result = format!(
            "Assigned agent {} to card {} (chat: {})",
            agent_id, card_id, agent_chat_id
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

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_assign_agent".to_string(),
            display_name: "Task Assign Agent".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Assign an agent to a card and move it to Doing.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("card_id", "string", "Card ID to assign"),
                    ("agent_id", "string", "Agent UUID"),
                    ("agent_chat_id", "string", "Agent chat/trajectory ID"),
                ],
                &["card_id", "agent_id", "agent_chat_id"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_task_agent_scope;
    use crate::chat::types::TaskMeta;
    use serde_json::{json, Value};
    use std::collections::HashMap;

    fn bound_meta() -> TaskMeta {
        TaskMeta {
            task_id: "task-1".to_string(),
            role: "agents".to_string(),
            agent_id: Some("agent-1".to_string()),
            card_id: Some("card-1".to_string()),
            planner_chat_id: Some("planner-1".to_string()),
        }
    }

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn task_agent_scope_rejects_bound_task_id_mismatch() {
        let args = args(&[("task_id", json!("task-2")), ("card_id", json!("card-1"))]);
        let err = resolve_task_agent_scope(Some(&bound_meta()), "agent-chat", &args, true, true)
            .unwrap_err();

        assert!(err.contains("bound task_id 'task-1'"));
    }

    #[test]
    fn task_agent_scope_rejects_bound_card_id_mismatch() {
        let args = args(&[("task_id", json!("task-1")), ("card_id", json!("card-2"))]);
        let err = resolve_task_agent_scope(Some(&bound_meta()), "agent-chat", &args, true, true)
            .unwrap_err();

        assert!(err.contains("bound card_id 'card-1'"));
    }

    #[test]
    fn task_agent_scope_accepts_bound_matching_ids() {
        let args = args(&[("task_id", json!("task-1")), ("card_id", json!("card-1"))]);
        let scope = resolve_task_agent_scope(Some(&bound_meta()), "agent-chat", &args, true, true)
            .unwrap();

        assert_eq!(scope.task_id, "task-1");
        assert_eq!(scope.card_id.as_deref(), Some("card-1"));
    }
}
