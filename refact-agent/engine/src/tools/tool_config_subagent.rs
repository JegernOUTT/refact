use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use serde_json::json;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::subchat::run_subchat;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::yaml_configs::customization_types::SubagentConfig;

pub struct ToolConfigSubagent {
    pub config: SubagentConfig,
}

impl ToolConfigSubagent {
    pub fn new(config: SubagentConfig) -> Self {
        Self { config }
    }

    fn build_input_schema(&self) -> serde_json::Value {
        if let Some(ref tool_schema) = self.config.tool {
            let mut properties = serde_json::Map::new();
            for p in &tool_schema.parameters {
                properties.insert(
                    p.name.clone(),
                    json!({
                        "type": p.param_type,
                        "description": p.description
                    }),
                );
            }
            json!({
                "type": "object",
                "properties": properties,
                "required": tool_schema.required
            })
        } else {
            json_schema_from_params(&[("task", "string", "The task to execute")], &["task"])
        }
    }

    fn render_template(&self, template: &str, args: &HashMap<String, Value>) -> String {
        let mut result = template.to_string();
        for (key, value) in args {
            let placeholder = format!("{{{{{}}}}}", key);
            let replacement = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => value.to_string(),
            };
            result = result.replace(&placeholder, &replacement);
        }
        result
    }
}

#[async_trait]
impl Tool for ToolConfigSubagent {
    fn tool_description(&self) -> ToolDesc {
        let description = if let Some(ref tool_schema) = self.config.tool {
            tool_schema.description.clone()
        } else {
            self.config.description.clone()
        };

        let allow_parallel = self
            .config
            .tool
            .as_ref()
            .map(|t| t.allow_parallel)
            .unwrap_or(false);

        ToolDesc {
            name: self.config.id.clone(),
            display_name: self.config.title.clone(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel,
            description,
            input_schema: self.build_input_schema(),
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
        use crate::at_commands::at_commands::MAX_SUBCHAT_DEPTH;

        let (
            gcx,
            parent_chat_id,
            parent_root_chat_id,
            parent_subchat_tx,
            parent_abort_flag,
            current_depth,
            parent_task_meta,
            parent_worktree,
        ) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.global_context.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.root_chat_id.clone(),
                ccx_lock.subchat_tx.clone(),
                ccx_lock.abort_flag.clone(),
                ccx_lock.subchat_depth,
                ccx_lock.task_meta.clone(),
                ccx_lock.execution_scope_worktree(),
            )
        };

        if current_depth >= MAX_SUBCHAT_DEPTH {
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(format!(
                        "Error: Maximum subagent recursion depth ({}) exceeded",
                        MAX_SUBCHAT_DEPTH
                    )),
                    tool_call_id: tool_call_id.clone(),
                    tool_failed: Some(true),
                    output_filter: Some(OutputFilter::no_limits()),
                    ..Default::default()
                })],
            ));
        }

        let max_steps = self.config.subchat.max_steps.unwrap_or(10).min(50).max(1);

        let title = format!("{}: {}", self.config.title, self.config.id);

        let tools_list: Option<Vec<String>> = if self.config.tools.is_empty() {
            None
        } else {
            Some(self.config.tools.clone())
        };

        let config = crate::subchat::resolve_subchat_config_with_parent(
            gcx.clone(),
            &self.config.id,
            self.config.subchat.stateful,
            None,
            Some(title),
            Some(parent_chat_id),
            Some(self.config.id.clone()),
            Some(parent_root_chat_id),
            tools_list,
            max_steps,
            false,
            None,
            "agent".to_string(),
            parent_task_meta,
            parent_worktree,
            Some(tool_call_id.clone()),
            Some(parent_subchat_tx),
            Some(parent_abort_flag),
            current_depth + 1,
        )
        .await?;

        let mut messages = Vec::new();

        if let Some(ref system_prompt) = self.config.messages.system_prompt {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: ChatContent::SimpleText(system_prompt.clone()),
                ..Default::default()
            });
        }

        for pre_msg in &self.config.messages.pre_messages {
            messages.push(ChatMessage {
                role: pre_msg.role.clone(),
                content: ChatContent::SimpleText(self.render_template(&pre_msg.content, args)),
                ..Default::default()
            });
        }

        if let Some(ref user_template) = self.config.messages.user_template {
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(self.render_template(user_template, args)),
                ..Default::default()
            });
        } else {
            let task = args
                .get("task")
                .and_then(|v| v.as_str())
                .unwrap_or("Execute the task");
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(task.to_string()),
                ..Default::default()
            });
        }

        for post_msg in &self.config.messages.post_messages {
            messages.push(ChatMessage {
                role: post_msg.role.clone(),
                content: ChatContent::SimpleText(self.render_template(&post_msg.content, args)),
                ..Default::default()
            });
        }

        tracing::info!(
            "Starting config subagent '{}' (model: {})",
            self.config.id,
            config.model
        );

        let result = match run_subchat(gcx, messages, config).await {
            Ok(r) => r,
            Err(e) if e == "Aborted" || e.starts_with("Aborted") => {
                return Ok((
                    false,
                    vec![ContextEnum::ChatMessage(ChatMessage {
                        role: "tool".to_string(),
                        content: ChatContent::SimpleText(format!(
                            "{} aborted by user.",
                            self.config.title
                        )),
                        tool_calls: None,
                        tool_call_id: tool_call_id.clone(),
                        tool_failed: Some(true),
                        output_filter: Some(OutputFilter::no_limits()),
                        ..Default::default()
                    })],
                ));
            }
            Err(e) => return Err(e),
        };

        let last_assistant = result.messages.iter().rev().find(|m| m.role == "assistant");
        let result_content = last_assistant
            .map(|m| m.content.content_text_only())
            .unwrap_or_else(|| {
                format!("{} completed but produced no response.", self.config.title)
            });

        let result_message = format!("# {} Result\n\n{}", self.config.title, result_content);

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result_message),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                usage: None,
                extra: result.metering,
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
