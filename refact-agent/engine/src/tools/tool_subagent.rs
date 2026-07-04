use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};
use tokio::sync::Mutex as AMutex;

use crate::agents::spawn::{
    NotifyParent, SpawnHandle, SpawnRequest, spawn_and_wait, spawn_background_agent,
};
use crate::agents::types::{BackgroundAgent, BgAgentKind};
use crate::at_commands::at_commands::{AtCommandsContext, MAX_SUBCHAT_DEPTH};
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::yaml_configs::customization_registry::get_subagent_config;

const ALLOWED_FOR_SUBAGENT: &[&str] = &[
    "cat",
    "tree",
    "search_pattern",
    "search_symbol_definition",
    "search_semantic",
    "codegraph_overview",
    "code_health",
    "git_risk",
    "code_why",
    "code_duplication",
    "dead_code",
    "security_scan",
    "pr_blast",
    "code_map",
    "knowledge",
    "search_trajectories",
    "get_trajectory_context",
    "web",
    "web_search",
    "shell",
    "tasks_set",
    "compress_chat_probe",
    "compress_chat_apply",
    "subagent_finish",
];

#[derive(Clone)]
pub struct ToolSubagent {
    pub config_path: String,
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
            description: "Spawn a non-editing research subagent that works independently. Background by default; pass wait=true to block until it finishes. Use this for investigation, code exploration, shell-backed inspection, and analysis. For implementation/editing tasks use `delegate()`.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    (
                        "task",
                        "string",
                        "What the subagent should investigate. Be specific about scope and goal.",
                    ),
                    (
                        "expected_result",
                        "string",
                        "What a successful finding looks like (e.g. 'list of files calling X with line numbers').",
                    ),
                    (
                        "tools",
                        "string",
                        "Optional comma-separated analysis tools (e.g. 'cat,tree,search_pattern,shell'). Empty means use the configured `subagent` toolset.",
                    ),
                    ("max_steps", "string", "Step budget (default 50, max 50)."),
                    (
                        "wait",
                        "string",
                        "If 'true', block until the subagent finishes and return its full result. Default 'false' (background).",
                    ),
                ],
                &["task", "expected_result"],
            ),
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
        let task = parse_required_string(args, "task")?;
        let expected_result = parse_required_string(args, "expected_result")?;
        let tools_arg = parse_optional_csv(args, "tools")?;
        let max_steps = parse_max_steps(args)?;
        let wait = parse_optional_bool(args, "wait", false)?;

        let (
            gcx,
            parent_chat_id,
            parent_root_chat_id,
            parent_subchat_tx,
            _parent_abort_flag,
            subchat_depth,
            parent_task_meta,
            parent_worktree,
            current_model,
        ) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.root_chat_id.clone(),
                ccx_lock.subchat_tx.clone(),
                ccx_lock.abort_flag.clone(),
                ccx_lock.subchat_depth,
                ccx_lock.task_meta.clone(),
                ccx_lock.execution_scope_worktree(),
                ccx_lock.current_model.clone(),
            )
        };

        let configured_tools = get_subagent_config(gcx.clone(), "subagent", None)
            .await
            .ok_or_else(|| "subagent config 'subagent' not found".to_string())?
            .tools;
        let spawn_tools = if let Some(tools) = &tools_arg {
            Some(normalize_read_only_tools(tools, &configured_tools)?)
        } else {
            Some(configured_tools)
        };
        let prompt_tools = tools_arg
            .as_ref()
            .map(|_| spawn_tools.clone().unwrap_or_default());

        if subchat_depth >= MAX_SUBCHAT_DEPTH.saturating_sub(1) {
            return Err(format!(
                "subchat depth limit ({MAX_SUBCHAT_DEPTH}) exceeded"
            ));
        }

        let req = SpawnRequest {
            kind: BgAgentKind::Subagent,
            parent_chat_id,
            parent_root_chat_id: Some(parent_root_chat_id),
            parent_tool_call_id: Some(tool_call_id.clone()),
            config_name: "subagent".to_string(),
            title: short_title("Subagent", &task),
            prompt: build_subagent_prompt(&task, &expected_result, &prompt_tools, max_steps),
            tools: spawn_tools,
            target_files: vec![],
            max_steps,
            model: current_model,
            parent_subchat_tx: Some(parent_subchat_tx),
            parent_worktree,
            parent_task_meta,
            subchat_depth,
            notify_parent: NotifyParent::Auto,
        };

        let app = crate::app_state::AppState::from_gcx(gcx).await;
        if wait {
            let req_silent = SpawnRequest {
                notify_parent: NotifyParent::Silent,
                ..req
            };
            let record =
                spawn_and_wait(app, req_silent, Some(Duration::from_secs(30 * 60))).await?;
            Ok((
                false,
                vec![build_foreground_tool_result(&record, tool_call_id)],
            ))
        } else {
            let handle = spawn_background_agent(app, req).await?;
            Ok((
                false,
                vec![build_background_start_tool_result(
                    &handle,
                    &task,
                    tool_call_id,
                )],
            ))
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

fn parse_required_string(args: &HashMap<String, Value>, name: &str) -> Result<String, String> {
    match args.get(name) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        Some(Value::String(_)) | None => Err(format!("Missing argument `{name}`")),
        Some(value) => Err(format!("argument `{name}` must be a string: {value:?}")),
    }
}

fn parse_optional_csv(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Option<Vec<String>>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let tools = value
                .split(',')
                .map(str::trim)
                .filter(|tool| !tool.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if tools.is_empty() {
                Ok(None)
            } else {
                Ok(Some(tools))
            }
        }
        Some(value) => Err(format!("argument `{name}` must be a string: {value:?}")),
    }
}

fn parse_optional_usize(
    args: &HashMap<String, Value>,
    name: &str,
    default: usize,
) -> Result<usize, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(default),
        Some(Value::String(value)) => value
            .trim()
            .parse::<usize>()
            .map_err(|_| format!("argument `{name}` must be a positive integer")),
        Some(Value::Number(value)) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| format!("argument `{name}` must be a positive integer")),
        Some(value) => Err(format!(
            "argument `{name}` must be a positive integer: {value:?}"
        )),
    }
}

fn parse_max_steps(args: &HashMap<String, Value>) -> Result<usize, String> {
    Ok(parse_optional_usize(args, "max_steps", 50)?.clamp(1, 50))
}

fn parse_optional_bool(
    args: &HashMap<String, Value>,
    name: &str,
    default: bool,
) -> Result<bool, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(default),
        Some(Value::String(value)) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" => Ok(false),
            _ => Err(format!("argument `{name}` must be true or false")),
        },
        Some(value) => Err(format!(
            "argument `{name}` must be true or false: {value:?}"
        )),
    }
}

fn normalize_read_only_tools(
    tools: &[String],
    configured_tools: &[String],
) -> Result<Vec<String>, String> {
    let mut normalized_tools = Vec::new();
    let mut seen = HashSet::new();
    for tool in tools {
        let canonical = canonical_subagent_tool(tool);
        if !is_allowed_for_subagent(&canonical)
            || !is_configured_for_subagent(&canonical, configured_tools)
        {
            let bad = tool.trim();
            let bad = if bad.is_empty() { tool.as_str() } else { bad };
            return Err(format!(
                "Tool '{}' is not in the allowed set for subagents ({}). Use delegate() for implementation/editing.",
                bad,
                format_allowed_tools(configured_tools)
            ));
        }
        if seen.insert(canonical.clone()) {
            normalized_tools.push(canonical);
        }
    }
    Ok(normalized_tools)
}

#[cfg(test)]
fn validate_read_only_tools(tools: &[String], configured_tools: &[String]) -> Result<(), String> {
    normalize_read_only_tools(tools, configured_tools).map(|_| ())
}

fn canonical_subagent_tool(tool: &str) -> String {
    let mut normalized = tool.trim().to_ascii_lowercase();
    if normalized.starts_with(crate::llm::adapters::claude_code_compat::MCP_TOOL_PREFIX) {
        normalized = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(&normalized);
    }
    if matches!(normalized.as_str(), "grep" | "glob") {
        return "search_pattern".to_string();
    }
    crate::llm::adapters::claude_code_compat::CC_TOOL_RENAMES
        .iter()
        .find_map(|(original, renamed)| {
            if *renamed == normalized.as_str() {
                Some((*original).to_string())
            } else {
                None
            }
        })
        .unwrap_or(normalized)
}

fn is_allowed_for_subagent(tool: &str) -> bool {
    let normalized = canonical_subagent_tool(tool);
    ALLOWED_FOR_SUBAGENT.contains(&normalized.as_str())
}

fn is_configured_for_subagent(tool: &str, configured_tools: &[String]) -> bool {
    let normalized = canonical_subagent_tool(tool);
    configured_tools
        .iter()
        .any(|configured| canonical_subagent_tool(configured) == normalized)
}

fn format_allowed_tools(configured_tools: &[String]) -> String {
    ALLOWED_FOR_SUBAGENT
        .iter()
        .copied()
        .filter(|tool| is_configured_for_subagent(tool, configured_tools))
        .collect::<Vec<_>>()
        .join(", ")
}

fn short_title(prefix: &str, task: &str) -> String {
    let task = task.trim();
    let truncated = truncate_chars(task, 50);
    if task.chars().count() > 50 {
        format!("{prefix}: {truncated}…")
    } else {
        format!("{prefix}: {truncated}")
    }
}

fn build_subagent_prompt(
    task: &str,
    expected_result: &str,
    tools: &Option<Vec<String>>,
    max_steps: usize,
) -> String {
    let tools_list = tools
        .as_ref()
        .filter(|tools| !tools.is_empty())
        .map(|tools| tools.join(", "))
        .unwrap_or_else(|| "configured `subagent` toolset".to_string());
    format!(
        r#"# Your Task
{task}

# Expected Result
{expected_result}

# Allowed Tools
{tools_list}

# Constraints
- Maximum steps: {max_steps}
- Read-only: do NOT attempt to modify files
- Use `tasks_set` to publish progress
- End with the Status report described in your system prompt"#
    )
}

fn build_background_start_tool_result(
    handle: &SpawnHandle,
    task: &str,
    tool_call_id: &String,
) -> ContextEnum {
    let task_preview = truncate_chars_with_ellipsis(task, 60);
    let content = format!(
        "✓ Started background subagent: {task_preview}\n- agent_id: {agent_id}\n- status: running\n- child_chat_id: {child_chat_id}\n\nOpen the child trajectory: [view](refact://chat/{child_chat_id})\n\nThe completion will be pushed back into this chat automatically. Use `agent_status`, `agent_wait`, or `agent_result` if you need to follow up sooner.",
        agent_id = handle.agent_id,
        child_chat_id = handle.child_chat_id,
    );
    tool_message(
        content,
        tool_call_id,
        background_agent_extra(&handle.agent_id, Some(&handle.child_chat_id), "running"),
    )
}

fn build_foreground_tool_result(record: &BackgroundAgent, tool_call_id: &String) -> ContextEnum {
    let status = record.status.as_str();
    let child_chat_id = record.child_chat_id.as_deref().unwrap_or_default();
    let result = record
        .result_summary
        .as_deref()
        .filter(|result| !result.trim().is_empty())
        .or(record.error.as_deref())
        .unwrap_or("Subagent finished without a result summary.");
    let link = if child_chat_id.is_empty() {
        String::new()
    } else {
        format!("\nOpen the child trajectory: [view](refact://chat/{child_chat_id})\n")
    };
    let content = format!(
        "# Subagent Result\n\n- agent_id: {agent_id}\n- status: {status}\n- child_chat_id: {child_chat_id}\n{link}\n## Result\n\n{result}",
        agent_id = record.agent_id,
    );
    tool_message(
        content,
        tool_call_id,
        background_agent_extra(&record.agent_id, record.child_chat_id.as_deref(), status),
    )
}

fn tool_message(content: String, tool_call_id: &String, extra: Map<String, Value>) -> ContextEnum {
    ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_call_id: tool_call_id.clone(),
        preserve: Some(true),
        extra,
        output_filter: Some(OutputFilter::no_limits()),
        ..Default::default()
    })
}

fn background_agent_extra(
    agent_id: &str,
    child_chat_id: Option<&str>,
    status: &str,
) -> Map<String, Value> {
    Map::from_iter([
        ("background_agent_id".to_string(), json!(agent_id)),
        ("background_agent_kind".to_string(), json!("subagent")),
        ("child_chat_id".to_string(), json!(child_chat_id)),
        ("background_agent_status".to_string(), json!(status)),
    ])
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn truncate_chars_with_ellipsis(text: &str, max_chars: usize) -> String {
    let truncated = truncate_chars(text.trim(), max_chars);
    if text.trim().chars().count() > max_chars {
        format!("{truncated}…")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::app_state::AppState;
    use crate::call_validation::ChatMessage;
    use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
    use crate::subchat::{SubchatResult, ToolsPolicy};
    use serial_test::serial;

    fn args_with(task: &str, expected_result: &str) -> HashMap<String, Value> {
        HashMap::from_iter([
            ("task".to_string(), json!(task)),
            ("expected_result".to_string(), json!(expected_result)),
        ])
    }

    fn single_message(contexts: Vec<ContextEnum>) -> ChatMessage {
        match contexts.into_iter().next().expect("tool message") {
            ContextEnum::ChatMessage(message) => message,
            ContextEnum::ContextFile(_) => panic!("expected chat message"),
        }
    }

    fn message_text(message: &ChatMessage) -> String {
        message.content.content_text_only()
    }

    fn configured_read_only_tools() -> Vec<String> {
        [
            "tree",
            "cat",
            "search_pattern",
            "search_symbol_definition",
            "search_semantic",
            "codegraph_overview",
            "code_health",
            "git_risk",
            "code_why",
            "code_duplication",
            "dead_code",
            "security_scan",
            "pr_blast",
            "code_map",
            "knowledge",
            "search_trajectories",
            "get_trajectory_context",
            "web",
            "web_search",
            "shell",
            "compress_chat_probe",
            "compress_chat_apply",
            "tasks_set",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    #[test]
    fn configured_tools_are_all_allowed_and_error_list_is_exact() {
        let configured_tools = configured_read_only_tools();

        for tool in &configured_tools {
            assert!(is_allowed_for_subagent(tool), "{tool} should be allowed");
        }
        assert_eq!(
            format_allowed_tools(&configured_tools),
            concat!(
                "cat, tree, search_pattern, search_symbol_definition, search_semantic, ",
                "codegraph_overview, code_health, git_risk, code_why, code_duplication, ",
                "dead_code, security_scan, pr_blast, code_map, knowledge, search_trajectories, ",
                "get_trajectory_context, web, ",
                "web_search, shell, tasks_set, compress_chat_probe, compress_chat_apply"
            )
        );
    }

    async fn install_test_caps(gcx: Arc<crate::global_context::GlobalContext>) {
        let model_id = "test/light".to_string();
        let mut caps = CodeAssistantCaps::default();
        caps.chat_models.insert(
            model_id.clone(),
            Arc::new(ChatModelRecord {
                base: BaseModelRecord {
                    id: model_id.clone(),
                    name: model_id.clone(),
                    n_ctx: 200_000,
                    endpoint: "https://example.com/v1/chat/completions".to_string(),
                    ..Default::default()
                },
                supports_tools: true,
                supports_agent: true,
                max_output_tokens: Some(16_000),
                ..Default::default()
            }),
        );
        caps.defaults.chat_default_model = model_id.clone();
        caps.defaults.chat_light_model = model_id.clone();
        caps.defaults.chat_thinking_model = model_id;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_add(60);
        let mut caps_state = gcx.caps_state.write().await;
        caps_state.caps = Some(Arc::new(caps));
        caps_state.last_attempted_ts = now;
    }

    async fn test_context(parent_chat_id: &str) -> Arc<AMutex<AtCommandsContext>> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        install_test_caps(gcx.clone()).await;
        let app = AppState::from_gcx(gcx).await;
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                20,
                false,
                vec![],
                parent_chat_id.to_string(),
                None,
                "parent/model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    #[test]
    fn delegate_tool_is_rejected_with_delegate_guidance() {
        let configured_tools = configured_read_only_tools();
        let error = validate_read_only_tools(&["delegate".to_string()], &configured_tools)
            .expect_err("delegate should be rejected");

        assert!(error.contains("Tool 'delegate'"));
        assert!(error.contains("Use delegate() for implementation/editing."));
        assert!(error.contains(&format!("({})", format_allowed_tools(&configured_tools))));
        assert!(error.contains("cat, tree, search_pattern"));
    }

    #[test]
    fn future_editing_tool_is_rejected() {
        let configured_tools = configured_read_only_tools();
        let error =
            validate_read_only_tools(&["future_editing_tool".to_string()], &configured_tools)
                .expect_err("unknown editing tool should be rejected");

        assert!(error.contains("Tool 'future_editing_tool'"));
        assert!(error.contains("not in the allowed set for subagents"));
        assert!(error.contains(&format_allowed_tools(&configured_tools)));
    }

    #[test]
    fn editing_tool_is_rejected() {
        let configured_tools = configured_read_only_tools();
        let error = validate_read_only_tools(&["apply_patch".to_string()], &configured_tools)
            .expect_err("editing tool should be rejected");

        assert!(error.contains("Tool 'apply_patch'"));
        assert!(error.contains("Use delegate() for implementation/editing."));
    }

    #[test]
    fn cat_tool_is_accepted_when_configured() {
        let configured_tools = configured_read_only_tools();

        assert!(validate_read_only_tools(&["cat".to_string()], &configured_tools).is_ok());
    }

    #[test]
    fn shell_tool_is_accepted_when_configured() {
        let configured_tools = configured_read_only_tools();

        assert!(validate_read_only_tools(&["shell".to_string()], &configured_tools).is_ok());
    }

    #[test]
    fn history_and_codegraph_tools_are_accepted_when_configured() {
        let configured_tools = configured_read_only_tools();

        assert_eq!(
            normalize_read_only_tools(
                &[
                    "hist_search".to_string(),
                    "hist_get".to_string(),
                    "codegraph_overview".to_string(),
                    "code_health".to_string(),
                    "git_risk".to_string(),
                    "code_why".to_string(),
                    "code_duplication".to_string(),
                    "security_scan".to_string(),
                    "pr_blast".to_string(),
                    "code_map".to_string(),
                ],
                &configured_tools,
            )
            .unwrap(),
            vec![
                "search_trajectories".to_string(),
                "get_trajectory_context".to_string(),
                "codegraph_overview".to_string(),
                "code_health".to_string(),
                "git_risk".to_string(),
                "code_why".to_string(),
                "code_duplication".to_string(),
                "security_scan".to_string(),
                "pr_blast".to_string(),
                "code_map".to_string(),
            ]
        );
    }

    #[test]
    fn cc_alias_tool_names_are_accepted_and_canonicalized() {
        let configured_tools = configured_read_only_tools();

        assert_eq!(
            normalize_read_only_tools(&["regex_search".to_string()], &configured_tools).unwrap(),
            vec!["search_pattern".to_string()]
        );
        assert_eq!(
            normalize_read_only_tools(&["t_regex_search".to_string()], &configured_tools).unwrap(),
            vec!["search_pattern".to_string()]
        );
        assert_eq!(
            normalize_read_only_tools(&["t_set_tasks".to_string()], &configured_tools).unwrap(),
            vec!["tasks_set".to_string()]
        );
        assert_eq!(
            normalize_read_only_tools(&["set_tasks".to_string()], &configured_tools).unwrap(),
            vec!["tasks_set".to_string()]
        );
        assert_eq!(
            normalize_read_only_tools(&["Grep".to_string()], &configured_tools).unwrap(),
            vec!["search_pattern".to_string()]
        );
        assert_eq!(
            normalize_read_only_tools(&["Glob".to_string()], &configured_tools).unwrap(),
            vec!["search_pattern".to_string()]
        );
    }

    #[test]
    fn duplicate_aliases_collapse_to_single_canonical_tool() {
        let configured_tools = configured_read_only_tools();

        assert_eq!(
            normalize_read_only_tools(
                &["search_pattern".to_string(), "regex_search".to_string()],
                &configured_tools,
            )
            .unwrap(),
            vec!["search_pattern".to_string()]
        );
    }

    #[test]
    fn tools_empty_uses_default_toolset_without_rejection() {
        let args = HashMap::from_iter([("tools".to_string(), json!("  ,  "))]);
        let tools = parse_optional_csv(&args, "tools").unwrap();
        assert_eq!(tools, None);
        let configured_tools = configured_read_only_tools();
        let spawn_tools = tools.clone().or_else(|| Some(configured_tools.clone()));
        assert_eq!(spawn_tools, Some(configured_tools));
        let prompt = build_subagent_prompt("look", "facts", &tools, 15);
        assert!(prompt.contains("configured `subagent` toolset"));
    }

    #[serial]
    #[tokio::test]
    async fn wait_false_default_returns_background_agent_id_in_extra() {
        let ccx = test_context("parent-bg").await;
        let app = ccx.lock().await.app.clone();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let release_rx = Arc::new(StdMutex::new(Some(release_rx)));
        let runner_rx = release_rx.clone();
        let _runner = crate::agents::spawn::install_test_runner(Arc::new(
            move |_gcx, mut messages, _config| {
                let release_rx = runner_rx.lock().unwrap().take();
                Box::pin(async move {
                    if let Some(release_rx) = release_rx {
                        let _ = release_rx.await;
                    }
                    messages.push(ChatMessage::new(
                        "assistant".to_string(),
                        "done".to_string(),
                    ));
                    Ok(SubchatResult {
                        messages,
                        metering: Map::new(),
                        chat_id: Some("ignored".to_string()),
                    })
                })
            },
        ));
        let mut tool = ToolSubagent {
            config_path: "builtin_tools.yaml".to_string(),
        };
        let tool_call_id = "call-bg".to_string();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            tool.tool_execute(
                ccx.clone(),
                &tool_call_id,
                &args_with("inspect frogs", "frog facts"),
            ),
        )
        .await
        .expect("wait=false should return before the subagent finishes")
        .unwrap();
        let message = single_message(result.1);
        let agent_id = message
            .extra
            .get("background_agent_id")
            .and_then(Value::as_str)
            .expect("agent id")
            .to_string();
        assert!(agent_id.starts_with("bgagent-"));
        assert_eq!(
            message
                .extra
                .get("background_agent_status")
                .and_then(Value::as_str),
            Some("running")
        );
        assert_eq!(
            message
                .extra
                .get("background_agent_kind")
                .and_then(Value::as_str),
            Some("subagent")
        );
        assert!(message_text(&message).contains("✓ Started background subagent"));
        assert!(message_text(&message).contains("refact://chat/subchat-"));
        let _ = release_tx.send(());
        let finished = app
            .agents
            .wait("parent-bg", &agent_id, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(finished.config_name, "subagent");
    }

    #[serial]
    #[tokio::test]
    async fn wait_true_returns_full_result_from_spawn_and_wait() {
        let ccx = test_context("parent-wait").await;
        let captured_max_steps = Arc::new(StdMutex::new(None));
        let captured_config_name = Arc::new(StdMutex::new(None));
        let captured_tools = Arc::new(StdMutex::new(None));
        let runner_steps = captured_max_steps.clone();
        let runner_config = captured_config_name.clone();
        let runner_tools = captured_tools.clone();
        let _runner = crate::agents::spawn::install_test_runner(Arc::new(
            move |_gcx, mut messages, config| {
                *runner_steps.lock().unwrap() = Some(config.max_steps);
                *runner_config.lock().unwrap() = Some(config.tool_name.clone());
                *runner_tools.lock().unwrap() = Some(match config.tools {
                    ToolsPolicy::All => vec!["ALL".to_string()],
                    ToolsPolicy::None => vec![],
                    ToolsPolicy::Only(tools) => tools,
                });
                Box::pin(async move {
                    messages.push(ChatMessage::new(
                        "assistant".to_string(),
                        "full wait result".to_string(),
                    ));
                    Ok(SubchatResult {
                        messages,
                        metering: Map::new(),
                        chat_id: Some("ignored".to_string()),
                    })
                })
            },
        ));
        let mut args = args_with("inspect wait path", "complete answer");
        args.insert("wait".to_string(), json!("true"));
        args.insert("max_steps".to_string(), json!("7"));
        let mut tool = ToolSubagent {
            config_path: "builtin_tools.yaml".to_string(),
        };
        let (_, contexts) = tool
            .tool_execute(ccx, &"call-wait".to_string(), &args)
            .await
            .unwrap();
        let message = single_message(contexts);
        let text = message_text(&message);
        assert!(text.contains("# Subagent Result"));
        assert!(text.contains("full wait result"));
        assert!(text.contains("refact://chat/subchat-"));
        assert_eq!(
            message
                .extra
                .get("background_agent_status")
                .and_then(Value::as_str),
            Some("completed")
        );
        assert!(message
            .extra
            .get("background_agent_id")
            .and_then(Value::as_str)
            .unwrap()
            .starts_with("bgagent-"));
        assert_eq!(*captured_max_steps.lock().unwrap(), Some(7));
        assert_eq!(
            captured_config_name.lock().unwrap().as_deref(),
            Some("subagent")
        );
        assert_eq!(
            *captured_tools.lock().unwrap(),
            Some(configured_read_only_tools())
        );
    }

    #[serial]
    #[tokio::test]
    async fn explicit_alias_tools_are_canonicalized_for_spawn_and_prompt() {
        let ccx = test_context("parent-alias-tools").await;
        let captured_tools = Arc::new(StdMutex::new(None));
        let captured_prompt = Arc::new(StdMutex::new(None));
        let runner_tools = captured_tools.clone();
        let runner_prompt = captured_prompt.clone();
        let _runner = crate::agents::spawn::install_test_runner(Arc::new(
            move |_gcx, mut messages, config| {
                *runner_prompt.lock().unwrap() = Some(
                    messages
                        .iter()
                        .map(|message| message.content.content_text_only())
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                *runner_tools.lock().unwrap() = Some(match config.tools {
                    ToolsPolicy::All => vec!["ALL".to_string()],
                    ToolsPolicy::None => vec![],
                    ToolsPolicy::Only(tools) => tools,
                });
                Box::pin(async move {
                    messages.push(ChatMessage::new(
                        "assistant".to_string(),
                        "alias wait result".to_string(),
                    ));
                    Ok(SubchatResult {
                        messages,
                        metering: Map::new(),
                        chat_id: Some("ignored".to_string()),
                    })
                })
            },
        ));
        let mut args = args_with("inspect alias path", "complete answer");
        args.insert("wait".to_string(), json!("true"));
        args.insert("tools".to_string(), json!("regex_search,t_set_tasks"));
        let mut tool = ToolSubagent {
            config_path: "builtin_tools.yaml".to_string(),
        };

        let (_, contexts) = tool
            .tool_execute(ccx, &"call-alias".to_string(), &args)
            .await
            .unwrap();

        assert_eq!(
            *captured_tools.lock().unwrap(),
            Some(vec!["search_pattern".to_string(), "tasks_set".to_string()])
        );
        let prompt = captured_prompt.lock().unwrap().clone().unwrap_or_default();
        assert!(prompt.contains("search_pattern, tasks_set"));
        assert!(!prompt.contains("regex_search,t_set_tasks"));
        let message = single_message(contexts);
        let text = message_text(&message);
        assert!(text.contains("alias wait result"));
    }

    #[test]
    fn max_steps_clamps_to_supported_range() {
        let low = HashMap::from_iter([("max_steps".to_string(), json!(0))]);
        let high = HashMap::from_iter([("max_steps".to_string(), json!(999))]);
        let default = HashMap::new();
        assert_eq!(parse_max_steps(&low).unwrap(), 1);
        assert_eq!(parse_max_steps(&high).unwrap(), 50);
        assert_eq!(parse_max_steps(&default).unwrap(), 50);
    }

    #[test]
    fn missing_task_or_expected_result_returns_clear_error() {
        let empty = HashMap::new();
        assert_eq!(
            parse_required_string(&empty, "task").unwrap_err(),
            "Missing argument `task`"
        );
        let task_only = HashMap::from_iter([("task".to_string(), json!("look"))]);
        assert_eq!(
            parse_required_string(&task_only, "expected_result").unwrap_err(),
            "Missing argument `expected_result`"
        );
    }

    #[test]
    fn title_truncation_uses_prefix_and_fifty_task_chars() {
        let long_task = "a".repeat(60);
        assert_eq!(
            short_title("Subagent", &long_task),
            format!("Subagent: {}…", "a".repeat(50))
        );
    }
}
