use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::tools::tools_description::{Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::subchat::run_subchat;
use crate::postprocessing::pp_command_output::OutputFilter;

pub struct ToolSubagent {
    pub config_path: String,
}

static SUBAGENT_SYSTEM_PROMPT: &str = r#"You are a focused sub-agent executing a specific task. You have been delegated this task by a parent agent.

Your task is clearly defined below. Execute it efficiently and report your findings.

Guidelines:
- Stay focused on the assigned task only
- Use the provided tools to accomplish the task
- Be thorough but efficient - you have a limited step budget
- Report progress and findings clearly
- When you achieve the expected result, summarize what you found/did
- If you cannot complete the task, explain why and what you tried

Do NOT:
- Deviate from the assigned task
- Ask clarifying questions - work with what you have
- Exceed your step budget unnecessarily"#;

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

        let (gcx, parent_chat_id, parent_subchat_tx) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.global_context.clone(), ccx_lock.chat_id.clone(), ccx_lock.subchat_tx.clone())
        };

        let title = if task.len() > 60 {
            let end = task
                .char_indices()
                .take_while(|(i, _)| *i < 60)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(60.min(task.len()));
            format!("Subagent: {}...", &task[..end])
        } else {
            format!("Subagent: {}", task)
        };

        let config = crate::subchat::resolve_subchat_config_with_parent(
            gcx.clone(),
            "subagent",
            true,
            None,
            Some(title),
            Some(parent_chat_id),
            Some("subagent".to_string()),
            if tools.is_empty() { None } else { Some(tools.clone()) },
            max_steps,
            false,
            None,
            Some(tool_call_id.clone()),
            Some(parent_subchat_tx),
        ).await?;

        let user_prompt = build_task_prompt(&task, &expected_result, &tools, max_steps);

        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: ChatContent::SimpleText(SUBAGENT_SYSTEM_PROMPT.to_string()),
                ..Default::default()
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(user_prompt),
                ..Default::default()
            },
        ];

        tracing::info!("Starting subagent for task: {} (model: {})", task, config.model);

        let result = run_subchat(gcx, messages, config).await?;

        let last_assistant = result.messages.iter().rev().find(|m| m.role == "assistant");
        let result_content = last_assistant
            .map(|m| m.content.content_text_only())
            .unwrap_or_else(|| "Subagent completed but produced no response.".to_string());

        let result_message = format!(
            r#"# Subagent Result

**Task:** {}

**Expected Result:** {}

## Response

{}"#,
            task, expected_result, result_content
        );

        Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result_message),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            usage: Some(result.usage),
            output_filter: Some(OutputFilter::no_limits()),
            ..Default::default()
        })]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
