use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::tool_task_check_agents::{get_task_id, get_agent_statuses, format_agent_status};

pub struct ToolTaskWaitForAgents;

impl ToolTaskWaitForAgents {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskWaitForAgents {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_wait_for_agents".to_string(),
            display_name: "Task Wait For Agents".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Check the status of all spawned agents for a task. Shows their board status (primary) and live session state (if available). Agents mark themselves done via task_agent_finish(). Agents that fail (streaming errors, timeouts, stuck) are automatically marked as failed.".to_string(),
            input_schema: json_schema_from_params(&[("task_id", "string", "Task ID (optional if chat is bound to a task)")], &[]),
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
        let ccx_lock = ccx.lock().await;

        let is_planner = ccx_lock
            .task_meta
            .as_ref()
            .map(|m| m.role == "planner")
            .unwrap_or(false);

        if !is_planner {
            return Err(
                "task_wait_for_agents can only be called by the task planner. \
                 Switch to the planner chat to check agent status."
                    .to_string(),
            );
        }

        drop(ccx_lock);

        let task_id = get_task_id(&ccx, args).await?;
        let gcx = ccx.lock().await.global_context.clone();

        let statuses = get_agent_statuses(gcx, &task_id).await?;

        if statuses.is_empty() {
            let result = "# Agent Status\n\nNo agents have been spawned yet for this task.\n\nUse `task_spawn_agent(card_id)` to spawn an agent for a card.".to_string();

            {
                let ccx_lock = ccx.lock().await;
                ccx_lock
                    .abort_flag
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }

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

        {
            let ccx_lock = ccx.lock().await;
            ccx_lock
                .abort_flag
                .store(true, std::sync::atomic::Ordering::SeqCst);
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
