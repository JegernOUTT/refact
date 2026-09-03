use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value, json};
use tokio::sync::Mutex as AMutex;

use crate::agents::spawn::{
    NotifyParent, SpawnGoal, SpawnRequest, SpawnWorktreeMode, spawn_background_agent,
};
use crate::agents::types::BgAgentKind;
use crate::at_commands::at_commands::{AtCommandsContext, MAX_SUBCHAT_DEPTH};
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::types::{GoalBudget, GoalCriterion};
use crate::global_context::try_load_caps_quickly_if_not_present;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::tools::tools_list::{get_available_tools, get_tools_for_mode};
use crate::yaml_configs::customization_registry::get_subagent_config;

pub const SUBAGENT_FORCE_TOOLS: &[&str] = &[
    "tasks_set",
    "progress_report",
    "agents_overview",
    "agent_message",
    "validate_goal",
];

const MODEL_TYPES: &[&str] = &[
    "default",
    "light",
    "thinking",
    "buddy",
    "model_2",
    "task_planner",
];

#[derive(Clone)]
pub struct ToolSubagent {
    pub config_path: String,
}

#[derive(Clone)]
struct SubagentArgs {
    task: String,
    expected_result: String,
    target_files: Vec<String>,
    tools: Option<Vec<String>>,
    model_type: Option<String>,
    model_name: Option<String>,
    goal: Option<SpawnGoal>,
    plan: Option<String>,
    worktree_mode: SpawnWorktreeMode,
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
            description: "Spawn a background-only, stateful child trajectory for research or implementation. The child inherits all parent-chat tools by default, can select a configured model or named model, and may receive a goal with a step budget, plan, target files, and an isolated worktree that auto-merges by default. A trajectory link is returned immediately and completion is auto-pushed. Use `agents_overview` and `agent_message` to coordinate with peers.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task": {"type": "string", "description": "Focused instructions for the background subagent."},
                    "expected_result": {"type": "string", "description": "Concrete outcome the subagent should deliver."},
                    "target_files": {"type": "array", "items": {"type": "string"}, "description": "Optional expected edit targets. When set, the child may edit only these files."},
                    "tools": {"type": "string", "description": "Optional comma-separated tool names. Omit to inherit all parent-chat tools."},
                    "model_type": {"type": "string", "enum": MODEL_TYPES, "description": "Configured model slot. Ignored when model_name is supplied."},
                    "model_name": {"type": "string", "description": "Concrete chat model id. Takes precedence over model_type."},
                    "goal": {"description": "Optional string or object {content, criteria?, budget?}; use goal.budget.max_turns for a step limit."},
                    "plan": {"type": "string", "description": "Optional installed plan; it is ground truth for the child."},
                    "worktree": {"type": "string", "enum": ["inherit", "isolated"], "default": "inherit", "description": "Use the parent worktree or create an isolated worktree."},
                    "auto_merge": {"type": "boolean", "default": true, "description": "Only for worktree=isolated. Squash-merge completion automatically."}
                },
                "required": ["task", "expected_result"]
            }),
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
        let args = parse_subagent_args(args)?;
        let (
            gcx,
            app,
            parent_chat_id,
            parent_root_chat_id,
            parent_subchat_tx,
            subchat_depth,
            parent_task_meta,
            parent_worktree,
            current_model,
        ) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.root_chat_id.clone(),
                ccx_lock.subchat_tx.clone(),
                ccx_lock.subchat_depth,
                ccx_lock.task_meta.clone(),
                ccx_lock.execution_scope_worktree(),
                ccx_lock.current_model.clone(),
            )
        };

        if subchat_depth >= MAX_SUBCHAT_DEPTH.saturating_sub(1) {
            return Err(format!(
                "subchat depth limit ({MAX_SUBCHAT_DEPTH}) exceeded"
            ));
        }

        let config = get_subagent_config(gcx.clone(), "subagent", None)
            .await
            .ok_or_else(|| "subagent config 'subagent' not found".to_string())?;
        let max_steps = config.subchat.max_steps.unwrap_or(60).max(1);
        let available_tools = available_tools(gcx.clone()).await;
        let spawn_tools = match args.tools.as_ref() {
            Some(requested) => Some(normalize_explicit_tools(requested, &available_tools)?),
            None => inherit_parent_tools(&app, gcx.clone(), &parent_chat_id, &current_model).await,
        };
        let (model, model_type) = resolve_model(
            gcx.clone(),
            args.model_name.as_deref(),
            args.model_type.as_deref(),
            &current_model,
        )
        .await?;
        let peer_snapshot = peer_snapshot(&app, &parent_root_chat_id, &args.target_files).await;
        let prompt = build_subagent_prompt(
            &args.task,
            &args.expected_result,
            &args.target_files,
            &peer_snapshot,
            args.goal.is_some(),
        );
        let req = SpawnRequest {
            kind: BgAgentKind::Subagent,
            parent_chat_id: parent_chat_id.clone(),
            parent_root_chat_id: Some(parent_root_chat_id),
            parent_tool_call_id: Some(tool_call_id.clone()),
            config_name: "subagent".to_string(),
            title: short_title("Subagent", &args.task),
            prompt,
            tools: spawn_tools,
            target_files: args.target_files.clone(),
            max_steps,
            model: model.clone(),
            model_type: model_type.clone(),
            goal: args.goal,
            plan: args.plan,
            worktree_mode: args.worktree_mode,
            parent_subchat_tx: Some(parent_subchat_tx),
            parent_worktree,
            parent_task_meta,
            subchat_depth,
            notify_parent: NotifyParent::Auto,
        };
        let handle = spawn_background_agent(app, req).await?;
        Ok((
            false,
            vec![build_background_start_tool_result(
                &handle,
                &args.task,
                &parent_chat_id,
                &model,
                model_type.as_deref(),
                &peer_snapshot,
                tool_call_id,
            )],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

fn parse_subagent_args(args: &HashMap<String, Value>) -> Result<SubagentArgs, String> {
    let task = parse_required_string(args, "task")?;
    let expected_result = parse_required_string(args, "expected_result")?;
    let target_files = parse_target_files(args)?;
    let tools = parse_optional_csv(args, "tools")?;
    let model_type = parse_optional_string(args, "model_type")?;
    if let Some(model_type) = &model_type {
        if !MODEL_TYPES.contains(&model_type.as_str()) {
            return Err(format!(
                "argument `model_type` must be one of: {}",
                MODEL_TYPES.join(", ")
            ));
        }
    }
    let model_name = parse_optional_string(args, "model_name")?;
    let goal = parse_goal(args)?;
    let plan = parse_optional_string(args, "plan")?;
    let worktree =
        parse_optional_string(args, "worktree")?.unwrap_or_else(|| "inherit".to_string());
    let auto_merge_present =
        args.contains_key("auto_merge") && !args.get("auto_merge").is_some_and(Value::is_null);
    let worktree_mode = match worktree.as_str() {
        "inherit" => {
            if auto_merge_present {
                return Err(
                    "argument `auto_merge` is only valid when `worktree` is `isolated`".to_string(),
                );
            }
            SpawnWorktreeMode::Inherit
        }
        "isolated" => SpawnWorktreeMode::Isolated {
            auto_merge: parse_optional_bool(args, "auto_merge", true)?,
        },
        _ => return Err("argument `worktree` must be `inherit` or `isolated`".to_string()),
    };
    Ok(SubagentArgs {
        task,
        expected_result,
        target_files,
        tools,
        model_type,
        model_name,
        goal,
        plan,
        worktree_mode,
    })
}

fn parse_required_string(args: &HashMap<String, Value>, name: &str) -> Result<String, String> {
    match args.get(name) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        Some(Value::String(_)) | None => Err(format!("Missing argument `{name}`")),
        Some(value) => Err(format!(
            "argument `{name}` must be a non-empty string: {value:?}"
        )),
    }
}

fn parse_optional_string(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Option<String>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.trim().to_string())),
        Some(value) => Err(format!("argument `{name}` must be a string: {value:?}")),
    }
}

fn parse_optional_csv(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = parse_optional_string(args, name)? else {
        return Ok(None);
    };
    let tools = value
        .split(',')
        .map(str::trim)
        .filter(|tool| !tool.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    Ok((!tools.is_empty()).then_some(tools))
}

fn parse_target_files(args: &HashMap<String, Value>) -> Result<Vec<String>, String> {
    match args.get("target_files") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| match value {
                Value::String(path) if !path.trim().is_empty() => Ok(path.trim().to_string()),
                Value::String(_) => {
                    Err("argument `target_files` contains an empty string".to_string())
                }
                other => Err(format!(
                    "argument `target_files` contains a non-string value: {other:?}"
                )),
            })
            .collect(),
        Some(value) => Err(format!(
            "argument `target_files` must be an array of strings: {value:?}"
        )),
    }
}

fn parse_optional_bool(
    args: &HashMap<String, Value>,
    name: &str,
    default: bool,
) -> Result<bool, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => refact_tool_api::coerce_bool(value)
            .ok_or_else(|| format!("argument `{name}` must be true or false: {value:?}")),
    }
}

fn parse_goal(args: &HashMap<String, Value>) -> Result<Option<SpawnGoal>, String> {
    let Some(value) = args.get("goal") else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::String(content) if !content.trim().is_empty() => Ok(Some(SpawnGoal {
            content: content.trim().to_string(),
            criteria: Vec::new(),
            budget: None,
        })),
        Value::String(_) => Err("argument `goal` must not be empty".to_string()),
        Value::Object(object) => {
            let content = object
                .get("content")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|content| !content.is_empty())
                .ok_or_else(|| "argument `goal.content` must be a non-empty string".to_string())?
                .to_string();
            let criteria = match object.get("criteria") {
                None | Some(Value::Null) => Vec::new(),
                Some(value) => serde_json::from_value::<Vec<GoalCriterion>>(value.clone())
                    .map_err(|error| format!("argument `goal.criteria` is malformed: {error}"))?,
            };
            let budget = match object.get("budget") {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    serde_json::from_value::<GoalBudget>(value.clone())
                        .map_err(|error| format!("argument `goal.budget` is malformed: {error}"))?,
                ),
            };
            Ok(Some(SpawnGoal {
                content,
                criteria,
                budget,
            }))
        }
        other => Err(format!(
            "argument `goal` must be a string or object: {other:?}"
        )),
    }
}

async fn available_tools(gcx: Arc<crate::global_context::GlobalContext>) -> HashSet<String> {
    let mut names = get_available_tools(gcx)
        .await
        .into_iter()
        .map(|tool| tool.tool_description().name)
        .collect::<HashSet<_>>();
    names.extend(SUBAGENT_FORCE_TOOLS.iter().map(|tool| (*tool).to_string()));
    names
}

async fn inherit_parent_tools(
    app: &crate::app_state::AppState,
    gcx: Arc<crate::global_context::GlobalContext>,
    parent_chat_id: &str,
    model: &str,
) -> Option<Vec<String>> {
    let session = {
        let sessions = app.chat.sessions.read().await;
        sessions.get(parent_chat_id)?.clone()
    };
    let parent_mode = session.lock().await.thread.mode.clone();
    let mut tools = get_tools_for_mode(gcx, &parent_mode, Some(model))
        .await
        .into_iter()
        .map(|tool| tool.tool_description().name)
        .collect::<Vec<_>>();
    for tool in SUBAGENT_FORCE_TOOLS {
        if !tools.iter().any(|candidate| candidate == tool) {
            tools.push((*tool).to_string());
        }
    }
    Some(tools)
}

fn canonical_tool_name(tool: &str) -> String {
    let normalized = tool.trim().to_ascii_lowercase();
    let normalized =
        if normalized.starts_with(crate::llm::adapters::claude_code_compat::MCP_TOOL_PREFIX) {
            crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(&normalized)
        } else {
            normalized
        };
    if normalized == "grep" {
        return "search_pattern".to_string();
    }
    crate::llm::adapters::claude_code_compat::CC_TOOL_RENAMES
        .iter()
        .find_map(|(original, renamed)| (*renamed == normalized).then(|| (*original).to_string()))
        .unwrap_or(normalized)
}

fn normalize_explicit_tools(
    requested: &[String],
    available: &HashSet<String>,
) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for tool in requested {
        let canonical = canonical_tool_name(tool);
        if !available.contains(&canonical) {
            let mut names = available.iter().cloned().collect::<Vec<_>>();
            names.sort();
            return Err(format!(
                "Unknown tool `{}`. Available tools: {}",
                tool.trim(),
                names.join(", ")
            ));
        }
        if seen.insert(canonical.clone()) {
            result.push(canonical);
        }
    }
    for tool in SUBAGENT_FORCE_TOOLS {
        if seen.insert((*tool).to_string()) {
            result.push((*tool).to_string());
        }
    }
    Ok(result)
}

async fn resolve_model(
    gcx: Arc<crate::global_context::GlobalContext>,
    model_name: Option<&str>,
    model_type: Option<&str>,
    parent_model: &str,
) -> Result<(String, Option<String>), String> {
    let caps = try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .map_err(|error| format!("failed to load caps: {error:?}"))?;
    let model_id = if let Some(model_name) = model_name {
        model_name.to_string()
    } else if let Some(model_type) = model_type {
        let slot = match model_type {
            "default" => &caps.defaults.chat_default_model,
            "light" => &caps.defaults.chat_light_model,
            "thinking" => &caps.defaults.chat_thinking_model,
            "buddy" => &caps.defaults.chat_buddy_model,
            "model_2" => &caps.defaults.chat_model_2,
            "task_planner" => &caps.defaults.task_planner_agent_model,
            _ => unreachable!("model type validated before resolution"),
        };
        if slot.trim().is_empty() {
            return Err(format!(
                "model_type `{model_type}` is not configured. Configured model slots: {}",
                configured_model_slots(&caps)
            ));
        }
        slot.clone()
    } else {
        parent_model.to_string()
    };
    let selected_type = model_name
        .is_none()
        .then(|| model_type.map(str::to_string))
        .flatten();
    let model = crate::caps::resolve_chat_model(caps, &model_id)
        .map_err(|error| format!("model `{model_id}` is not available: {error}"))?;
    Ok((model.base.id.clone(), selected_type))
}

fn configured_model_slots(caps: &crate::caps::CodeAssistantCaps) -> String {
    [
        ("default", &caps.defaults.chat_default_model),
        ("light", &caps.defaults.chat_light_model),
        ("thinking", &caps.defaults.chat_thinking_model),
        ("buddy", &caps.defaults.chat_buddy_model),
        ("model_2", &caps.defaults.chat_model_2),
        ("task_planner", &caps.defaults.task_planner_agent_model),
    ]
    .into_iter()
    .filter(|(_, model)| !model.trim().is_empty())
    .map(|(name, model)| format!("{name}={model}"))
    .collect::<Vec<_>>()
    .join(", ")
}

async fn peer_snapshot(
    app: &crate::app_state::AppState,
    root_chat_id: &str,
    target_files: &[String],
) -> String {
    let requested = target_files
        .iter()
        .map(|path| crate::agents::registry::normalize_path_for_overlap(path))
        .collect::<HashSet<_>>();
    let peers = app
        .agents
        .list_all()
        .await
        .into_iter()
        .filter(|record| {
            !record.status.is_terminal()
                && (record.parent_root_chat_id.as_deref() == Some(root_chat_id)
                    || record.parent_chat_id == root_chat_id)
        })
        .collect::<Vec<_>>();
    if peers.is_empty() {
        return "(no active peers)".to_string();
    }
    let mut lines = Vec::new();
    for peer in peers {
        lines.push(format!(
            "- {} — status: {}; target_files: {}; current_tool: {}",
            peer.title,
            peer.status.as_str(),
            files_label(&peer.target_files),
            peer.current_tool.unwrap_or_else(|| "-".to_string()),
        ));
        let overlaps = peer
            .target_files
            .iter()
            .filter(|path| {
                requested.contains(&crate::agents::registry::normalize_path_for_overlap(path))
            })
            .collect::<Vec<_>>();
        if !overlaps.is_empty() {
            lines.push(format!(
                "  ⚠ Overlap warning: {} targets {}",
                peer.title,
                overlaps
                    .into_iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    lines.join("\n")
}

fn build_subagent_prompt(
    task: &str,
    expected_result: &str,
    target_files: &[String],
    peers: &str,
    goal_installed: bool,
) -> String {
    let mut prompt = format!("# Your Task\n{task}\n\n# Expected Result\n{expected_result}\n\n");
    if !target_files.is_empty() {
        prompt.push_str("# Target Files\nONLY edit files in this list:\n");
        for path in target_files {
            prompt.push_str(&format!("- {path}\n"));
        }
        prompt.push('\n');
    }
    prompt.push_str(&format!("# Active peers\n{peers}\n\n"));
    prompt.push_str("# Constraints\n- Work independently in this background trajectory.\n- You MAY run tests, compilation, lint, or other verification when your tools allow it.\n- Publish progress with `tasks_set` and `progress_report`.\n- Sibling digest notices arrive in your context automatically; coordinate with `agents_overview` and `agent_message`.\n");
    if goal_installed {
        prompt.push_str("- An installed goal is ground truth. You MUST call `validate_goal` before finishing and report any unmet criteria.\n");
    }
    prompt.push_str("- End with: `Status: DONE | DONE_WITH_CONCERNS | NEEDS_CONTEXT | BLOCKED`, then Findings, Changes, Evidence, Concerns, and Next action.\n");
    prompt
}

fn build_background_start_tool_result(
    handle: &crate::agents::spawn::SpawnHandle,
    task: &str,
    parent_chat_id: &str,
    model: &str,
    model_type: Option<&str>,
    peers: &str,
    tool_call_id: &str,
) -> ContextEnum {
    let mut lines = vec![
        format!(
            "✓ Started background subagent: {}",
            truncate_chars(task, 60)
        ),
        format!("- agent_id: {}", handle.agent_id),
        "- status: running".to_string(),
        format!("- child_chat_id: {}", handle.child_chat_id),
        format!("- model: {model}"),
    ];
    if let Some(model_type) = model_type {
        lines.push(format!("- model_type: {model_type}"));
    }
    if let Some(branch) = &handle.worktree_branch {
        lines.push(format!("- worktree_branch: {branch}"));
        lines.push(format!(
            "- auto_merge: {}",
            handle.auto_merge.unwrap_or(false)
        ));
    }
    lines.extend([
        String::new(),
        format!(
            "Open the child trajectory: [view](refact://chat/{})",
            handle.child_chat_id
        ),
        String::new(),
        "The completion will be pushed back into this chat automatically.".to_string(),
    ]);
    if peers.contains("⚠ Overlap warning") {
        lines.extend([
            String::new(),
            "Peer overlap warnings:".to_string(),
            peers.to_string(),
        ]);
    }
    tool_message(
        lines.join("\n"),
        tool_call_id,
        Map::from_iter([
            ("background_agent_id".to_string(), json!(handle.agent_id)),
            ("background_agent_kind".to_string(), json!("subagent")),
            ("background_agent_status".to_string(), json!("running")),
            ("child_chat_id".to_string(), json!(handle.child_chat_id)),
            ("model".to_string(), json!(model)),
            ("model_type".to_string(), json!(model_type)),
            ("worktree_branch".to_string(), json!(handle.worktree_branch)),
            ("auto_merge".to_string(), json!(handle.auto_merge)),
            (
                "background_agent_parent_chat_id".to_string(),
                json!(parent_chat_id),
            ),
        ]),
    )
}

fn tool_message(content: String, tool_call_id: &str, extra: Map<String, Value>) -> ContextEnum {
    ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_call_id: tool_call_id.to_string(),
        preserve: Some(true),
        extra,
        output_filter: Some(OutputFilter::no_limits()),
        ..Default::default()
    })
}

fn files_label(files: &[String]) -> String {
    if files.is_empty() {
        "-".to_string()
    } else {
        files.join(", ")
    }
}

fn short_title(prefix: &str, task: &str) -> String {
    let truncated = truncate_chars(task, 50);
    if task.chars().count() > 50 {
        format!("{prefix}: {truncated}…")
    } else {
        format!("{prefix}: {truncated}")
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.trim().chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::app_state::AppState;
    use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
    use crate::subchat::{SubchatResult, ToolsPolicy};
    use serial_test::serial;

    fn args() -> HashMap<String, Value> {
        HashMap::from([
            ("task".to_string(), json!("Implement frog support")),
            ("expected_result".to_string(), json!("Frogs are supported")),
        ])
    }

    async fn install_test_caps(gcx: Arc<crate::global_context::GlobalContext>) {
        let mut caps = CodeAssistantCaps::default();
        for slot in [
            "default",
            "light",
            "thinking",
            "buddy",
            "model_2",
            "task_planner",
        ] {
            let model_id = format!("test/{slot}");
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
        }
        caps.defaults.chat_default_model = "test/default".to_string();
        caps.defaults.chat_light_model = "test/light".to_string();
        caps.defaults.chat_thinking_model = "test/thinking".to_string();
        caps.defaults.chat_buddy_model = "test/buddy".to_string();
        caps.defaults.chat_model_2 = "test/model_2".to_string();
        caps.defaults.task_planner_agent_model = "test/task_planner".to_string();
        let mut caps_state = gcx.caps_state.write().await;
        caps_state.caps = Some(Arc::new(caps));
        caps_state.last_attempted_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
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
                Some("root-chat".to_string()),
                "test/default".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    fn message(contexts: Vec<ContextEnum>) -> ChatMessage {
        match contexts.into_iter().next().expect("tool message") {
            ContextEnum::ChatMessage(message) => message,
            _ => panic!("expected chat message"),
        }
    }

    #[test]
    fn schema_is_background_only() {
        let schema = ToolSubagent {
            config_path: String::new(),
        }
        .tool_description()
        .input_schema;
        let properties = schema["properties"].as_object().unwrap();
        for absent in ["wait", "max_steps", "notify_parent"] {
            assert!(
                !properties.contains_key(absent),
                "{absent} must not be exposed"
            );
        }
        for present in [
            "task",
            "expected_result",
            "target_files",
            "tools",
            "model_type",
            "model_name",
            "goal",
            "plan",
            "worktree",
            "auto_merge",
        ] {
            assert!(
                properties.contains_key(present),
                "{present} must be exposed"
            );
        }
    }

    #[test]
    fn explicit_tools_accept_editing_aliases_and_force_tools() {
        let available = HashSet::from([
            "apply_patch".to_string(),
            "search_pattern".to_string(),
            "tasks_set".to_string(),
            "validate_goal".to_string(),
        ]);
        let tools = normalize_explicit_tools(
            &["t_apply".to_string(), "regex_search".to_string()],
            &available,
        )
        .unwrap();
        assert_eq!(&tools[..2], ["apply_patch", "search_pattern"]);
        for required in SUBAGENT_FORCE_TOOLS {
            assert!(tools.contains(&required.to_string()));
        }
    }

    #[tokio::test]
    async fn force_tools_are_registered() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let registered = get_available_tools(gcx)
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect::<HashSet<_>>();

        for tool in SUBAGENT_FORCE_TOOLS {
            assert!(registered.contains(*tool), "{tool} must be registered");
        }
    }

    #[test]
    fn unknown_tool_lists_registry_tools() {
        let error =
            normalize_explicit_tools(&["wat".to_string()], &HashSet::from(["cat".to_string()]))
                .unwrap_err();
        assert!(error.contains("Unknown tool `wat`"));
        assert!(error.contains("cat"));
    }

    #[test]
    fn goal_string_and_object_parse() {
        let mut string = args();
        string.insert("goal".to_string(), json!("Ship frogs"));
        let parsed = parse_subagent_args(&string).unwrap();
        assert_eq!(parsed.goal.unwrap().content, "Ship frogs");

        let mut object = args();
        object.insert(
            "goal".to_string(),
            json!({
                "content": "Ship frogs",
                "criteria": [{"id": "C1", "text": "tests", "verify_hint": "cargo test"}],
                "budget": {"max_turns": 3, "max_tokens": 200}
            }),
        );
        let parsed = parse_subagent_args(&object).unwrap();
        let goal = parsed.goal.unwrap();
        assert_eq!(goal.criteria[0].id, "C1");
        assert_eq!(goal.budget.unwrap().max_turns, Some(3));
    }

    #[test]
    fn worktree_auto_merge_validation() {
        let parsed = parse_subagent_args(&args()).unwrap();
        assert!(matches!(parsed.worktree_mode, SpawnWorktreeMode::Inherit));
        let mut isolated = args();
        isolated.insert("worktree".to_string(), json!("isolated"));
        assert!(matches!(
            parse_subagent_args(&isolated).unwrap().worktree_mode,
            SpawnWorktreeMode::Isolated { auto_merge: true }
        ));
        let mut inherited = args();
        inherited.insert("auto_merge".to_string(), json!(true));
        assert!(matches!(
            parse_subagent_args(&inherited),
            Err(error) if error.contains("only valid")
        ));
    }

    #[tokio::test]
    async fn all_model_type_slots_resolve_and_model_name_wins() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        install_test_caps(gcx.clone()).await;
        for model_type in MODEL_TYPES {
            let (model, selected) = resolve_model(gcx.clone(), None, Some(model_type), "parent")
                .await
                .unwrap();
            assert_eq!(model, format!("test/{model_type}"));
            assert_eq!(selected.as_deref(), Some(*model_type));
        }
        let (model, selected) = resolve_model(gcx, Some("test/light"), Some("thinking"), "parent")
            .await
            .unwrap();
        assert_eq!(model, "test/light");
        assert_eq!(selected, None);
    }

    #[tokio::test]
    async fn empty_model_slot_is_a_hard_error() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        install_test_caps(gcx.clone()).await;
        let mut caps = (*gcx.caps_state.read().await.caps.as_ref().unwrap())
            .as_ref()
            .clone();
        caps.defaults.chat_buddy_model.clear();
        gcx.caps_state.write().await.caps = Some(Arc::new(caps));
        let error = resolve_model(gcx, None, Some("buddy"), "parent")
            .await
            .unwrap_err();
        assert!(error.contains("not configured"));
        assert!(error.contains("Configured model slots"));
    }

    #[serial(test_runner)]
    #[tokio::test]
    async fn omitted_tools_spawn_with_all_tools_and_peer_prompt() {
        let ccx = test_context("parent").await;
        let app = ccx.lock().await.app.clone();
        let (peer, _, _) = app
            .agents
            .create(crate::agents::types::CreateAgentRequest {
                parent_chat_id: "parent".to_string(),
                parent_root_chat_id: Some("root-chat".to_string()),
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "subagent".to_string(),
                title: "Peer frog task".to_string(),
                prompt: String::new(),
                target_files: vec!["src/frog.rs".to_string()],
                model: "test/default".to_string(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        app.agents
            .mark_running(&peer.agent_id, "child-peer".to_string())
            .await
            .unwrap();
        let captured = Arc::new(std::sync::Mutex::new(None));
        let captured_runner = captured.clone();
        let _runner = crate::agents::spawn::install_test_runner(Arc::new(
            move |_gcx, mut messages, config| {
                *captured_runner.lock().unwrap() = Some((
                    config.tools,
                    messages
                        .iter()
                        .map(|message| message.content.content_text_only())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ));
                Box::pin(async move {
                    messages.push(ChatMessage::new(
                        "assistant".to_string(),
                        "done".to_string(),
                    ));
                    Ok(SubchatResult {
                        messages,
                        metering: Map::new(),
                        chat_id: Some("ignored".to_string()),
                        aborted: false,
                    })
                })
            },
        ));
        let mut args = args();
        args.insert("target_files".to_string(), json!(["src/frog.rs"]));
        let mut tool = ToolSubagent {
            config_path: String::new(),
        };
        let (_, contexts) = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();
        let result = app
            .agents
            .list_all()
            .await
            .into_iter()
            .find(|record| record.title.starts_with("Subagent:"))
            .unwrap();
        app.agents
            .wait("parent", &result.agent_id, Duration::from_secs(2))
            .await
            .unwrap();
        let (tools, prompt) = captured.lock().unwrap().clone().unwrap();
        assert!(matches!(tools, ToolsPolicy::All));
        assert!(prompt.contains("# Active peers"));
        assert!(prompt.contains("Peer frog task"));
        assert!(prompt.contains("Overlap warning"));
        let message = message(contexts);
        assert_eq!(message.extra["model"], "test/default");
        assert!(message
            .content
            .content_text_only()
            .contains("refact://chat/subchat-"));
    }

    #[serial(test_runner)]
    #[tokio::test]
    async fn omitted_tools_inherit_parent_mode_tools_and_force_tools() {
        let ccx = test_context("restricted-parent").await;
        let app = ccx.lock().await.app.clone();
        let parent_session = Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
            "restricted-parent".to_string(),
        )));
        parent_session.lock().await.thread.mode = "NO_TOOLS".to_string();
        app.chat
            .sessions
            .write()
            .await
            .insert("restricted-parent".to_string(), parent_session);
        let expected =
            inherit_parent_tools(&app, app.gcx.clone(), "restricted-parent", "test/default")
                .await
                .expect("parent mode tools");
        let captured = Arc::new(std::sync::Mutex::new(None));
        let captured_runner = captured.clone();
        let _runner =
            crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
                *captured_runner.lock().unwrap() = Some(config.tools);
                Box::pin(async move {
                    Ok(SubchatResult {
                        messages,
                        metering: Map::new(),
                        chat_id: Some("ignored".to_string()),
                        aborted: false,
                    })
                })
            }));
        let mut tool = ToolSubagent {
            config_path: String::new(),
        };
        tool.tool_execute(ccx, &"call".to_string(), &args())
            .await
            .unwrap();
        let record = app
            .agents
            .list_all()
            .await
            .into_iter()
            .find(|record| record.title.starts_with("Subagent:"))
            .expect("spawned agent");
        app.agents
            .wait(
                "restricted-parent",
                &record.agent_id,
                Duration::from_secs(2),
            )
            .await
            .expect("agent completion");

        assert!(matches!(
            captured.lock().unwrap().clone(),
            Some(ToolsPolicy::Only(tools)) if tools == expected
        ));
    }
}
