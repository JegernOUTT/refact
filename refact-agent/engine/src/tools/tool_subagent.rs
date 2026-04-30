use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::chat::types::TaskMeta;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::worktrees::types::WorktreeMeta;
use crate::subchat::run_subchat;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::yaml_configs::customization_registry::get_subagent_config;
use crate::knowledge_index::format_related_memories_section;
use regex::Regex;

const FILE_EDITING_TOOLS: &[&str] = &[
    "create_textdoc",
    "update_textdoc",
    "update_textdoc_anchored",
    "update_textdoc_by_lines",
    "update_textdoc_regex",
    "apply_patch",
    "undo_textdoc",
    "rm",
];

fn tools_contain_file_editing(tools: &[String]) -> bool {
    tools
        .iter()
        .any(|t| FILE_EDITING_TOOLS.contains(&t.as_str()))
}

fn tools_allow_editing_or_shell(tools: &[String]) -> bool {
    tools.is_empty()
        || tools
            .iter()
            .any(|t| t == "shell" || FILE_EDITING_TOOLS.contains(&t.as_str()))
}

fn parent_is_task_agent(task_meta: &Option<TaskMeta>) -> bool {
    task_meta
        .as_ref()
        .map(|meta| meta.role == "agent" || meta.role == "agents")
        .unwrap_or(false)
}

fn task_agent_scope_guard_error(
    task_meta: &Option<TaskMeta>,
    worktree: &Option<WorktreeMeta>,
    tools: &[String],
) -> Option<String> {
    if parent_is_task_agent(task_meta) && worktree.is_none() && tools_allow_editing_or_shell(tools)
    {
        Some(
            "Task-agent subagents with editing or shell tools require an active worktree scope"
                .to_string(),
        )
    } else {
        None
    }
}

#[derive(Clone)]
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

#[async_trait]
impl Tool for ToolSubagent {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "subagent".to_string(),
            display_name: "Subagent".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Delegate a specific task to a sub-agent that works independently. Use this when you need to perform a focused task that requires multiple tool calls without cluttering the main conversation. The subagent has its own context and does not see the parent conversation.".to_string(),
            input_schema: json_schema_from_params(&[("task", "string", "Clear description of what the subagent should do. Be specific about the goal and any constraints."), ("expected_result", "string", "Description of what the successful result should look like. This helps the subagent know when it has completed the task."), ("tools", "string", "Comma-separated list of tool names the subagent should use (e.g., 'cat,tree,search'). Leave empty to allow all available tools."), ("max_steps", "string", "Maximum number of steps (tool calls) the subagent can make. Default is 10. Use lower values for simple tasks, higher for complex ones.")], &["task", "expected_result", "tools", "max_steps"]),
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

        use crate::at_commands::at_commands::MAX_SUBCHAT_DEPTH;
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
                    ..Default::default()
                })],
            ));
        }

        let session_id_hook = parent_chat_id.clone();
        if let Some(error) =
            task_agent_scope_guard_error(&parent_task_meta, &parent_worktree, &tools)
        {
            return Err(error);
        }

        let has_editing_tools = tools_contain_file_editing(&tools);
        let config_name = if has_editing_tools {
            "subagent_with_editing"
        } else {
            "subagent"
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
            config_name,
            true,
            None,
            Some(title),
            Some(parent_chat_id),
            Some("subagent".to_string()),
            Some(parent_root_chat_id),
            if tools.is_empty() {
                None
            } else {
                Some(tools.clone())
            },
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

        let user_prompt = build_task_prompt(&task, &expected_result, &tools, max_steps);

        let subagent_config = get_subagent_config(gcx.clone(), config_name, None)
            .await
            .ok_or_else(|| format!("subagent config '{}' not found", config_name))?;

        let system_prompt = subagent_config
            .messages
            .system_prompt
            .as_ref()
            .ok_or_else(|| {
                format!(
                    "messages.system_prompt not defined for subagent '{}'",
                    config_name
                )
            })?;

        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: ChatContent::SimpleText(system_prompt.clone()),
                ..Default::default()
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(user_prompt),
                ..Default::default()
            },
        ];

        tracing::info!(
            "Starting subagent for task: {} (model: {})",
            task,
            config.model
        );

        let gcx_hook = gcx.clone();
        let project_dir = crate::ext::hooks_runner::get_project_dir_string(gcx_hook.clone()).await;
        let task_hook = task.clone();

        let subchat_result = run_subchat(gcx, messages, config).await;

        let final_status = match &subchat_result {
            Ok(_) => "completed",
            Err(e) if e == "Aborted" || e.starts_with("Aborted") => "aborted",
            Err(_) => "error",
        };
        {
            let mut extra = std::collections::HashMap::new();
            extra.insert("agent_name".to_string(), serde_json::json!(task_hook));
            extra.insert("final_status".to_string(), serde_json::json!(final_status));
            tokio::spawn(async move {
                let payload = crate::ext::hooks_runner::HookPayload {
                    hook_event_name: "SubagentStop".to_string(),
                    session_id: session_id_hook,
                    project_dir,
                    tool_name: None,
                    tool_input: None,
                    tool_output: None,
                    user_prompt: None,
                    extra,
                };
                crate::ext::hooks_runner::run_hooks(
                    gcx_hook,
                    crate::ext::hooks::HookEvent::SubagentStop,
                    payload,
                )
                .await;
            });
        }

        let result = match subchat_result {
            Ok(r) => r,
            Err(e) if e == "Aborted" || e.starts_with("Aborted") => {
                return Ok((
                    false,
                    vec![ContextEnum::ChatMessage(ChatMessage {
                        role: "tool".to_string(),
                        content: ChatContent::SimpleText("Subagent aborted by user.".to_string()),
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
            .unwrap_or_else(|| "Subagent completed but produced no response.".to_string());

        let result_message = format!(
            r#"# Subagent Result

**Task:** {}

**Expected Result:** {}

## Response

{}"#,
            task, expected_result, result_content
        );

        // Append related memories in short form (heuristic):
        // - detect file paths in task/expected_result
        // - retrieve related cards by filenames from in-memory index
        let related_section = {
            let combined = format!("{}\n{}", task, expected_result);
            let path_re =
                Regex::new(r"(?:^|[\s`])((?:[a-zA-Z0-9_-]+/)+[a-zA-Z0-9_-]+\.[a-zA-Z0-9]+)")
                    .unwrap();
            let mut files: Vec<String> = path_re
                .captures_iter(&combined)
                .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
                .collect();
            files.sort();
            files.dedup();

            let gcx = ccx.lock().await.global_context.clone();
            let gcx_read = gcx.read().await;
            let idx_guard = gcx_read.knowledge_index.lock().await;
            let mut cards = idx_guard.related_for_files(&files, 5);
            if cards.is_empty() {
                // Fall back to tag-based lookup if we have no file signals.
                cards = idx_guard.related_for_tags(
                    &vec![
                        "subagent".to_string(),
                        "report".to_string(),
                        "task-report".to_string(),
                    ],
                    5,
                );
            }
            format_related_memories_section(&cards, None)
        };

        let result_message = if related_section.trim().is_empty() {
            result_message
        } else {
            format!("{}{}", result_message, related_section)
        };

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn task_meta(role: &str) -> Option<TaskMeta> {
        Some(TaskMeta {
            task_id: "task-1".to_string(),
            role: role.to_string(),
            agent_id: Some("agent-1".to_string()),
            card_id: Some("card-1".to_string()),
        })
    }

    fn worktree() -> Option<WorktreeMeta> {
        Some(WorktreeMeta {
            id: "wt-subagent".to_string(),
            kind: "task_agent".to_string(),
            root: PathBuf::from("/tmp/wt"),
            source_workspace_root: PathBuf::from("/tmp/src"),
            repo_root: PathBuf::from("/tmp/src"),
            branch: Some("feature".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: Some("base".to_string()),
            task_id: Some("task-1".to_string()),
            card_id: Some("card-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            enforce: true,
        })
    }

    #[test]
    fn subchat_worktree_tool_subagent_requires_scope_for_task_agent_shell() {
        let tools = vec!["shell".to_string()];
        let error = task_agent_scope_guard_error(&task_meta("agents"), &None, &tools).unwrap();
        assert!(error.contains("worktree scope"));
    }

    #[test]
    fn subchat_worktree_tool_subagent_inherits_parent_scope_policy() {
        let tools = vec!["shell".to_string()];
        assert!(task_agent_scope_guard_error(&task_meta("agents"), &worktree(), &tools).is_none());
        assert!(task_agent_scope_guard_error(&task_meta("planner"), &None, &tools).is_none());
        assert!(task_agent_scope_guard_error(
            &task_meta("agents"),
            &None,
            &vec!["cat".to_string()]
        )
        .is_none());
        assert!(task_agent_scope_guard_error(&task_meta("agents"), &None, &Vec::new()).is_some());
    }
}
