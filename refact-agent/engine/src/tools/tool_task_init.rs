use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::tasks::storage;

pub struct ToolTaskInit;

impl ToolTaskInit {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskInit {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'name' argument")?;

        let gcx = ccx.lock().await.global_context.clone();
        let meta = storage::create_task(gcx, name).await?;

        let result = format!(
            "Created task workspace:\n- ID: {}\n- Name: {}\n- Path: .refact/tasks/{}/\n\nThe task is ready for planning. Use task_board_create_card to add cards.",
            meta.id, meta.name, meta.id
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
            name: "task_init".to_string(),
            display_name: "Task Init".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Create a new task workspace for planning and orchestrating work."
                .to_string(),
            input_schema: json_schema_from_params(
                &[(
                    "name",
                    "string",
                    "Name of the task (e.g., 'Auth Refactor', 'Database Migration')",
                )],
                &["name"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}
