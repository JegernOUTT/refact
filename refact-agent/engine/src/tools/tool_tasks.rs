use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskItem {
    pub id: String,
    pub content: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

pub struct ToolTasksSet {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolTasksSet {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "tasks_set".to_string(),
            display_name: "Set Tasks".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Set the task progress list shown to the user. Use to track multi-step work. \
                Pass complete task list each time (replaces previous). \
                Each task needs: id (unique string), content (description), status (pending/in_progress/completed/failed).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tasks": {
                        "type": "array",
                        "description": "Complete task list (replaces previous). Each task needs id, content, and status.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {
                                    "type": "string",
                                    "description": "Unique task identifier (1-50 chars)."
                                },
                                "content": {
                                    "type": "string",
                                    "description": "Task description (1-500 chars)."
                                },
                                "status": {
                                    "type": "string",
                                    "description": "Task status.",
                                    "enum": ["pending", "in_progress", "completed", "failed"]
                                }
                            },
                            "required": ["id", "content", "status"]
                        }
                    }
                },
                "required": ["tasks"]
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let tasks_value = match args.get("tasks") {
            Some(v) => v,
            None => return Err("argument `tasks` is missing".to_string()),
        };

        let tasks: Vec<TaskItem> = match serde_json::from_value(tasks_value.clone()) {
            Ok(t) => t,
            Err(e) => {
                return Err(format!(
                    "Invalid tasks format: {}. Expected array of {{id, content, status}}",
                    e
                ))
            }
        };

        if tasks.len() > 100 {
            return Err("Too many tasks (max 100)".to_string());
        }

        let mut seen_ids = std::collections::HashSet::new();
        for (i, task) in tasks.iter().enumerate() {
            if task.id.trim().is_empty() || task.id.len() > 50 {
                return Err(format!(
                    "Task {} has invalid id (must be 1-50 non-whitespace chars)",
                    i
                ));
            }
            if task.content.trim().is_empty() || task.content.len() > 500 {
                return Err(format!(
                    "Task {} has invalid content (must be 1-500 non-whitespace chars)",
                    i
                ));
            }
            if !seen_ids.insert(&task.id) {
                return Err(format!("Duplicate task id: {}", task.id));
            }
        }

        debug!("tasks_set: {} tasks", tasks.len());

        let result_msg = format!("✓ Task list updated ({} tasks)", tasks.len());

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result_msg),
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
