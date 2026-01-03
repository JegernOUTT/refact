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

async fn get_task_id(ccx: &Arc<AMutex<AtCommandsContext>>) -> Result<String, String> {
    let ccx_lock = ccx.lock().await;
    ccx_lock.task_meta.as_ref()
        .map(|m| m.task_id.clone())
        .ok_or_else(|| "This tool can only be used by task planners (chat not bound to a task)".to_string())
}

pub struct ToolTaskPlannerFinish;

impl ToolTaskPlannerFinish {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Tool for ToolTaskPlannerFinish {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_planner_finish".to_string(),
            display_name: "Task Planner Finish".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            agentic: false,
            experimental: false,
            description: "Mark planning as complete. Call this when you've finished creating the task board.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "summary".to_string(),
                    param_type: "string".to_string(),
                    description: "Summary of what was planned".to_string(),
                },
            ],
            parameters_required: vec!["summary".to_string()],
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx).await?;

        {
            let ccx_lock = ccx.lock().await;
            if let Some(ref meta) = ccx_lock.task_meta {
                if meta.role != "planner" {
                    return Err(format!(
                        "task_planner_finish can only be called by planner chats, not '{}'",
                        meta.role
                    ));
                }
            }
        }

        let summary = args.get("summary")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'summary' parameter")?
            .to_string();

        let gcx = ccx.lock().await.global_context.clone();

        let mut meta = storage::load_task_meta(gcx.clone(), &task_id).await?;

        if meta.status == crate::tasks::types::TaskStatus::Planning {
            meta.status = crate::tasks::types::TaskStatus::Active;
            meta.updated_at = Utc::now().to_rfc3339();
            storage::save_task_meta(gcx.clone(), &task_id, &meta).await?;
            storage::update_task_stats(gcx.clone(), &task_id).await?;
        }

        let result_message = format!(
            "✅ **Planning Complete**\n\n\
             **Summary:** {}\n\n\
             Task is now active. You can now:\n\
             - Use `task_ready_cards` to see which cards are ready\n\
             - Use `task_spawn_agent` to start agents on ready cards\n\
             - Monitor progress with `task_check_agents`",
            summary
        );

        tracing::info!(
            "Planner finished planning for task {}: {}",
            task_id,
            summary.chars().take(100).collect::<String>()
        );

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
