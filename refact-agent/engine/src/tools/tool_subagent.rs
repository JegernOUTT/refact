use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use async_trait::async_trait;
use uuid::Uuid;

use crate::tools::tools_description::{Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::chat::types::{ThreadParams, CommandRequest, ChatCommand};
use crate::chat::{get_or_create_session_with_trajectory, process_command_queue};

pub struct ToolSubagent {
    pub config_path: String,
}

fn build_task_prompt(
    task: &str,
    expected_result: &str,
    tools: &[String],
    max_steps: usize,
) -> String {
    format!(
        r#"# Your Task
{task}

# Expected Result
{expected_result}

# Available Tools
You have access to these tools: {tools_list}

# Constraints
- Maximum steps allowed: {max_steps}
- Focus only on this specific task
- Report findings clearly when done"#,
        task = task,
        expected_result = expected_result,
        tools_list = if tools.is_empty() {
            "all available".to_string()
        } else {
            tools.join(", ")
        },
        max_steps = max_steps
    )
}

async fn resolve_subagent_model(
    gcx: Arc<ARwLock<GlobalContext>>,
    current_model: &str,
) -> Result<String, String> {
    if !current_model.is_empty() {
        return Ok(current_model.to_string());
    }

    let caps = try_load_caps_quickly_if_not_present(gcx, 0).await
        .map_err(|e| format!("Failed to load caps for model resolution: {}", e))?;

    let default_model = &caps.defaults.chat_default_model;
    if !default_model.is_empty() {
        return Ok(default_model.clone());
    }

    Err("No model available: current_model and global default are both empty".to_string())
}

#[async_trait]
impl Tool for ToolSubagent {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "subagent".to_string(),
            display_name: "Subagent".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            agentic: true,
            experimental: false,
            description: "Delegate a specific task to a sub-agent that works independently. Use this when you need to perform a focused task that requires multiple tool calls without cluttering the main conversation. The subagent has its own context and does not see the parent conversation.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "task".to_string(),
                    param_type: "string".to_string(),
                    description: "Clear description of what the subagent should do. Be specific about the goal and any constraints.".to_string(),
                },
                ToolParam {
                    name: "expected_result".to_string(),
                    param_type: "string".to_string(),
                    description: "Description of what the successful result should look like. This helps the subagent know when it has completed the task.".to_string(),
                },
                ToolParam {
                    name: "tools".to_string(),
                    param_type: "string".to_string(),
                    description: "Comma-separated list of tool names the subagent should use (e.g., 'cat,tree,search'). Leave empty to allow all available tools.".to_string(),
                },
                ToolParam {
                    name: "max_steps".to_string(),
                    param_type: "string".to_string(),
                    description: "Maximum number of steps (tool calls) the subagent can make. Default is 10. Use lower values for simple tasks, higher for complex ones.".to_string(),
                },
            ],
            parameters_required: vec!["task".to_string(), "expected_result".to_string(), "tools".to_string(), "max_steps".to_string()],
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task = match args.get("task") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `task` is not a string: {:?}", v)),
            None => return Err("Missing argument `task`".to_string()),
        };

        let expected_result = match args.get("expected_result") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => {
                return Err(format!(
                    "argument `expected_result` is not a string: {:?}",
                    v
                ))
            }
            None => return Err("Missing argument `expected_result`".to_string()),
        };

        let tools: Vec<String> = match args.get("tools") {
            Some(Value::String(s)) if !s.trim().is_empty() => s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
            _ => vec![],
        };

        let max_steps: usize = match args.get("max_steps") {
            Some(Value::String(s)) => s.parse().unwrap_or(10),
            Some(Value::Number(n)) => n.as_u64().unwrap_or(10) as usize,
            _ => 10,
        };
        let max_steps = max_steps.min(50).max(1);

        let (gcx, current_model) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.global_context.clone(), ccx_lock.current_model.clone())
        };

        let model = resolve_subagent_model(gcx.clone(), &current_model).await?;

        let subagent_id = Uuid::new_v4().to_string();
        let subagent_chat_id = format!("subagent-{}", &subagent_id[..8]);

        let title = if task.len() > 60 {
            let end = task
                .char_indices()
                .take_while(|(i, _)| *i < 60)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(60.min(task.len()));
            format!("{}...", &task[..end])
        } else {
            task.clone()
        };

        let sessions = {
            let gcx_locked = gcx.read().await;
            gcx_locked.chat_sessions.clone()
        };

        let session_arc = get_or_create_session_with_trajectory(gcx.clone(), &sessions, &subagent_chat_id).await;

        {
            let mut session = session_arc.lock().await;

            session.thread = ThreadParams {
                id: subagent_chat_id.clone(),
                title: format!("Subagent: {}", title),
                model: model.clone(),
                mode: "AGENT".to_string(),
                tool_use: if tools.is_empty() { "agent".to_string() } else { tools.join(",") },
                boost_reasoning: false,
                context_tokens_cap: None,
                include_project_info: true,
                checkpoints_enabled: false,
                is_title_generated: true,
                automatic_patch: false,
                task_meta: None,
            };

            let user_prompt = build_task_prompt(&task, &expected_result, &tools, max_steps);
            let user_msg = ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(user_prompt),
                ..Default::default()
            };
            session.add_message(user_msg);

            session.increment_version();
        }

        crate::chat::maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;

        {
            let mut session = session_arc.lock().await;

            let request = CommandRequest {
                client_request_id: Uuid::new_v4().to_string(),
                priority: false,
                command: ChatCommand::Regenerate {},
            };
            session.command_queue.push_back(request);
            session.touch();

            let processor_running = session.queue_processor_running.clone();
            let queue_notify = session.queue_notify.clone();

            drop(session);

            if !processor_running.swap(true, Ordering::SeqCst) {
                tokio::spawn(process_command_queue(gcx.clone(), session_arc.clone(), processor_running));
            } else {
                queue_notify.notify_one();
            }
        }

        tracing::info!("Spawned subagent {} for task: {} (model: {})", subagent_id, task, model);

        let result_message = format!(
            r#"# Subagent Spawned

**Task:** {}

**Expected Result:** {}

**Subagent ID:** {}
**Model:** {}
**Status:** Running in background

📎 [View Subagent Chat](refact://chat/{})

The subagent is now working independently. Results will appear in the linked chat when complete."#,
            task, expected_result, subagent_id, model, subagent_chat_id
        );

        Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result_message),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
