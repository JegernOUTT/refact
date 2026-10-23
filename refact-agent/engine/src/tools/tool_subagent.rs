use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use refact_core::chat_types::PushMode;
use serde_json::{Map, Value, json};
use tokio::sync::Mutex as AMutex;

use crate::agents::launch_spec::{self, LaunchSpec, ResolvedLaunchContext};
use crate::agents::spawn::{
    NotifyParent, SpawnGoal, SpawnRequest, SpawnWorktreeMode, spawn_background_agent,
};
use crate::agents::types::BgAgentKind;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::types::{GoalBudget, GoalCriterion};
use crate::global_context::try_load_caps_quickly_if_not_present;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::tools::tools_list::get_available_tools;
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
    completion_push: PushMode,
    launch_spec: Option<LaunchSpec>,
    profile: Option<ProfileReference>,
    preview_only: bool,
    sandbox: Option<crate::exec::types::ExecSandboxMode>,
}

#[derive(Clone)]
struct ProfileReference {
    name: String,
    revision: Option<u64>,
}

#[async_trait]
impl Tool for ToolSubagent {
    fn tool_description(&self) -> ToolDesc {
        let context_entry_schema = json!({
            "type": "object",
            "properties": {
                "type": {"type": "string", "enum": ["raw_text", "message_range", "file_range", "task_doc", "memory_ref"]},
                "label": {"type": "string"}, "text": {"type": "string"},
                "start_message_id": {"type": "string"}, "end_message_id": {"type": "string"},
                "path": {"type": "string"}, "line1": {"type": "integer"}, "line2": {"type": "integer"},
                "task_id": {"type": "string"}, "slug": {"type": "string", "description": "Exact task document slug, required for task_doc."},
                "query": {"type": "string"}
            },
            "required": ["type"]
        });
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
                    "target_files": {"type": "array", "items": {"type": "string"}, "description": "Optional declared edit targets, used as collision hints. This is declared intent, not an enforced sandbox: it only restricts built-in mutations when the run's scope guard is strict."},
                    "sandbox": {"type": "string", "enum": ["read_only", "workspace_write", "full_access"], "description": "Optional confinement this run requires for shell execution. If this engine and terminal security mode cannot enforce it, the launch fails before dispatch instead of running unsandboxed. Omit to accept the engine default."},
                    "tools": {"type": "string", "description": "Optional comma-separated tool names. Omit to inherit all parent-chat tools."},
                    "model_type": {"type": "string", "enum": MODEL_TYPES, "description": "Configured model slot. Ignored when model_name is supplied."},
                    "model_name": {"type": "string", "description": "Concrete chat model id. Takes precedence over model_type."},
                    "goal": {"description": "Optional string or object {content, criteria?, budget?}; use goal.budget.max_turns for a step limit."},
                    "plan": {"type": "string", "description": "Optional installed plan; it is ground truth for the child."},
                    "worktree": {"type": "string", "enum": ["inherit", "isolated"], "default": "inherit", "description": "Use the parent worktree or create an isolated worktree."},
                    "push": PushMode::schema(),
                    "auto_merge": {"type": "boolean", "default": true, "description": "Only for worktree=isolated. Squash-merge completion automatically."},
                    "action": {"type": "string", "enum": ["run", "preview"], "default": "run", "description": "Use `preview` to resolve prompts/context and effective settings without spawning a child."},
                    "profile": {
                        "description": "Optional reusable agent profile: a name string, or {name, revision} to pin an exact revision. The profile supplies defaults (model, tools, role, skills, effort, hooks, path scope, approval, budget); this run may only narrow them. A profile never widens authority.",
                        "oneOf": [
                            {"type": "string"},
                            {
                                "type": "object",
                                "properties": {
                                    "name": {"type": "string"},
                                    "revision": {"type": "integer", "minimum": 1}
                                },
                                "required": ["name"]
                            }
                        ]
                    },
                    "launch_spec": {
                        "type": "object",
                        "description": "Optional parent-controlled child launch specification: custom instructions/profile augmentation and explicit initial context.",
                        "properties": {
                            "custom_instructions": {"type": "string", "description": "Extra instructions appended to the child's task."},
                            "profile_augmentation": {"type": "string", "description": "Extra profile/persona augmentation appended to the child's task."},
                            "history_mode": {"type": "string", "enum": ["fresh", "fork_completed_prefix"], "description": "fresh uses only explicit context; fork_completed_prefix also copies parent history up to its last complete tool boundary. Parent system policy is excluded; copied goals/plans are historical context, not child control state."},
                            "token_budget": {"type": "integer", "minimum": 1, "description": "Cap on estimated tokens for resolved context entries."},
                            "include_parent_goal": {"type": "boolean", "default": false, "description": "Copy the parent's current goal as historical reference context; it never becomes the child's goal."},
                            "include_parent_plan": {"type": "boolean", "default": false, "description": "Copy the parent's current plan as historical reference context; it is not installed as the child's plan."},
                            "deliverable": {
                                "type": "object",
                                "description": "Deliverable contract checked structurally on the child's final message: required Markdown sections, one ```json block validated against a JSON-schema subset, and an Evidence section. An unmet contract is reported to the parent; it is never proof of success.",
                                "properties": {
                                    "required_sections": {"type": "array", "items": {"type": "string"}},
                                    "output_schema": {"type": "object"},
                                    "evidence_required": {"type": "boolean", "default": false}
                                }
                            },
                            "context": {
                                "type": "array",
                                "description": "Explicit discriminated context entries; unmatched or over-budget entries are skipped with a reason, never silently dropped.",
                                "items": context_entry_schema
                            }
                        }
                    }
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
            parent_messages,
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
                ccx_lock.messages.clone(),
            )
        };

        let caller = if args
            .get("launch_spec")
            .is_some_and(|value| !value.is_null())
        {
            Some(
                crate::coordination::scope::model_caller(&app, &parent_chat_id, &current_model)
                    .await
                    .map_err(|_| "launch caller scope or model unavailable")?,
            )
        } else {
            None
        };
        let limits = match &caller {
            Some(caller) => {
                crate::runtime_settings::coordination_for_project(&caller.source_root).await?
            }
            None => crate::file_activity::settings(),
        };
        let args = parse_subagent_args_with_limits(args, &limits)?;
        crate::agents::sandbox_contract::preflight(
            args.sandbox.clone(),
            crate::exec::command_policy::baseline_sandbox_mode(Some("agent")),
            gcx.terminal_security_config.mode,
            refact_sandbox::sandbox_status(),
        )?;
        let profile = match args.profile.as_ref() {
            Some(reference) => Some(
                crate::agents::profile::load_profile(gcx.clone(), &reference.name, &limits)
                    .await
                    .map_err(|error| error.to_string())?,
            ),
            None => None,
        };
        if let Some(model_type) = profile.as_ref().and_then(|p| p.spec.model_type.as_deref()) {
            if !MODEL_TYPES.contains(&model_type) {
                return Err(crate::agents::profile::ProfileError::ProfileMalformed {
                    name: profile.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
                    detail: format!(
                        "model_type `{model_type}` must be one of: {}",
                        MODEL_TYPES.join(", ")
                    ),
                }
                .to_string());
            }
        }
        let (model, model_type) = resolve_model(
            gcx.clone(),
            args.model_name
                .as_deref()
                .or_else(|| profile.as_ref().and_then(|p| p.spec.model.as_deref())),
            args.model_type
                .as_deref()
                .or_else(|| profile.as_ref().and_then(|p| p.spec.model_type.as_deref())),
            &current_model,
        )
        .await?;
        let ceiling =
            crate::agents::tool_authority::current_ceiling(&app, &parent_chat_id, &model, "agent")
                .await?;
        let resolved_profile = match &profile {
            Some(profile) => Some(
                crate::agents::profile::resolve_profile_instance(
                    profile,
                    args.profile
                        .as_ref()
                        .and_then(|reference| reference.revision),
                    &crate::agents::profile::RunNarrowing {
                        tools: args.tools.clone(),
                        model: args.model_name.clone(),
                        model_type: args.model_type.clone(),
                        target_files: args.target_files.clone(),
                        max_steps: args
                            .goal
                            .as_ref()
                            .and_then(|goal| goal.budget.as_ref())
                            .and_then(|budget| budget.max_turns)
                            .filter(|turns| *turns > 0)
                            .map(|turns| turns as usize),
                    },
                    &ceiling,
                )
                .map_err(|error| error.to_string())?,
            ),
            None => None,
        };
        let target_files = resolved_profile
            .as_ref()
            .map(|profile| profile.target_files.clone())
            .unwrap_or_else(|| args.target_files.clone());
        let mut goal = args.goal.clone();
        if let (Some(goal), Some(budget)) = (
            goal.as_mut(),
            resolved_profile
                .as_ref()
                .and_then(|profile| profile.budget.clone()),
        ) {
            if goal.budget.is_none() {
                goal.budget = Some(budget);
            }
        }
        let requested_tools = match resolved_profile.as_ref() {
            Some(profile) => profile.tools.clone(),
            None => args.tools.clone(),
        };
        let requested = match requested_tools.as_ref() {
            Some(requested) => crate::subchat::ToolsPolicy::from_option(Some(
                normalize_explicit_tools(requested, &available_tools(gcx.clone()).await)?,
            )),
            None => crate::subchat::ToolsPolicy::All,
        };
        let frozen = crate::agents::tool_authority::freeze(
            &app,
            &parent_chat_id,
            &model,
            "agent",
            &requested,
        )
        .await?;
        let mut spawn_tools = match frozen {
            crate::subchat::ToolsPolicy::Only(names) => names,
            // A supervised ceiling is already a concrete, bounded set; the
            // exact names it carries are the only ones this child may get.
            crate::subchat::ToolsPolicy::Supervised { tools, .. } => tools,
            crate::subchat::ToolsPolicy::None => Vec::new(),
            crate::subchat::ToolsPolicy::All => {
                return Err("concrete child authority unavailable".into())
            }
        };
        for name in SUBAGENT_FORCE_TOOLS {
            if ceiling.allows_tool(name) && !spawn_tools.iter().any(|tool| tool == name) {
                spawn_tools.push(name.to_string());
            }
        }
        spawn_tools.sort();
        let handoff_limits = crate::agents::handoff::limits_from(&limits);
        let handoff_metric_max_records = limits.handoff_metric_max_records;
        let launch_scope = if let Some(caller) = caller {
            let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
                .await
                .map_err(|_| "launch model unavailable")?;
            let child_model = crate::caps::resolve_chat_model(caps, &model)?.base.clone();
            Some(launch_spec::LaunchScope {
                caller,
                child_model,
                limits,
            })
        } else {
            None
        };
        let resolved_launch = match args.launch_spec.as_ref() {
            Some(spec) => {
                let scope = launch_scope.as_ref().ok_or("launch scope unavailable")?;
                Some(
                    launch_spec::resolve_scoped_launch_spec(
                        gcx.clone(),
                        spec,
                        &parent_messages,
                        scope,
                    )
                    .await,
                )
            }
            None => None,
        };
        let max_steps = crate::runtime_settings::current()
            .coordination
            .subagent_default_steps;
        let peer_snapshot = peer_snapshot(&app, &parent_root_chat_id, &target_files).await;
        let deliverable = args
            .launch_spec
            .as_ref()
            .and_then(|spec| spec.deliverable.clone());
        let prompt = build_subagent_prompt(
            &args.task,
            &args.expected_result,
            &target_files,
            &peer_snapshot,
            goal.is_some(),
            deliverable.as_ref(),
            &handoff_limits,
        );
        let req = SpawnRequest {
            native_supervision_guard: None,
            kind: BgAgentKind::Subagent,
            parent_chat_id: parent_chat_id.clone(),
            parent_root_chat_id: Some(parent_root_chat_id.clone()),
            parent_tool_call_id: Some(tool_call_id.clone()),
            config_name: "subagent".to_string(),
            title: short_title("Subagent", &args.task),
            prompt,
            initial_context_messages: resolved_launch
                .as_ref()
                .map(|resolved| resolved.prepended_messages.clone())
                .unwrap_or_default(),
            history_source_model: args
                .launch_spec
                .as_ref()
                .filter(|spec| {
                    spec.history_mode == Some(launch_spec::HistoryForkMode::ForkCompletedPrefix)
                })
                .map(|_| current_model.clone()),
            custom_instructions: None,
            tools: Some(spawn_tools.clone()),
            target_files: target_files.clone(),
            max_steps,
            model: model.clone(),
            model_type: model_type.clone(),
            goal: goal.clone(),
            plan: args.plan.clone(),
            deliverable,
            profile: resolved_profile.clone(),
            worktree_mode: args.worktree_mode.clone(),
            parent_subchat_tx: Some(parent_subchat_tx),
            parent_worktree,
            parent_task_meta,
            subchat_depth,
            notify_parent: NotifyParent::Auto,
            completion_push: args.completion_push,
        };
        if args.preview_only {
            let effective = crate::agents::spawn::preview_launch(&app, &req).await;
            let mut preview = build_launch_preview_tool_result(
                &args,
                resolved_launch.as_ref(),
                resolved_profile.as_ref(),
                tool_call_id,
            );
            if let ContextEnum::ChatMessage(message) = &mut preview {
                message
                    .extra
                    .insert("effective_tools".into(), json!(spawn_tools));
                message.extra.insert("effective_model".into(), json!(model));
                if let Some(contract) = args
                    .launch_spec
                    .as_ref()
                    .and_then(|spec| spec.deliverable.as_ref())
                {
                    message.extra.insert("deliverable".into(), json!(contract));
                }
                match effective {
                    Ok(effective) => {
                        message
                            .extra
                            .insert("effective_launch".into(), json!(effective));
                    }
                    Err(error) => {
                        message.extra.insert("launch_blocked".into(), json!(error));
                        if let ChatContent::SimpleText(text) = &mut message.content {
                            text.push_str(&format!("\n\n## Launch Blocked\n- {error}"));
                        }
                    }
                }
            }
            let finished_children = app.agents.list_all().await;
            let handoff_key = crate::agents::handoff::metric_key(
                resolved_profile
                    .as_ref()
                    .map(|profile| profile.name.as_str()),
                &req.config_name,
            );
            let handoff_finding = crate::agents::handoff::finding_for(
                &finished_children,
                &handoff_key,
                &handoff_limits,
                crate::agents::handoff::FindingWindow::now(),
            );
            let mut handoff_metric = crate::agents::handoff::metric_for(
                &finished_children,
                &handoff_key,
                &handoff_limits,
                handoff_metric_max_records,
            );
            if let Some(metric) = handoff_metric.as_mut() {
                let destination =
                    crate::agents::handoff::parent_destination(&app, &parent_chat_id).await;
                crate::agents::handoff::redact_metric(&gcx, destination.as_ref(), metric);
            }
            if let (Some(metric), ContextEnum::ChatMessage(message)) =
                (&handoff_metric, &mut preview)
            {
                message.extra.insert("handoff_metric".into(), json!(metric));
                if let ChatContent::SimpleText(text) = &mut message.content {
                    text.push_str("\n\n## Handoff Sufficiency (prior children)\n");
                    text.push_str(&metric.summary_lines().join("\n"));
                }
            }
            if let ContextEnum::ChatMessage(message) = &mut preview {
                message
                    .extra
                    .insert("handoff_finding".into(), json!(handoff_finding));
                if let ChatContent::SimpleText(text) = &mut message.content {
                    text.push_str("\n\n## Handoff Brief Signal\n");
                    text.push_str(&handoff_finding.summary_lines().join("\n"));
                }
            }
            if let ContextEnum::ChatMessage(message) = &mut preview {
                let destination =
                    crate::agents::handoff::parent_destination(&app, &parent_chat_id).await;
                let advice = crate::coordination::decomposition::advise_for_model(
                    &app,
                    destination.as_ref(),
                    &crate::agents::handoff::metric_key(
                        resolved_profile
                            .as_ref()
                            .map(|profile| profile.name.as_str()),
                        &req.config_name,
                    ),
                )
                .await;
                message
                    .extra
                    .insert("decomposition_advice".into(), json!(advice));
                if let ChatContent::SimpleText(text) = &mut message.content {
                    text.push_str("\n\n## Decomposition Cost (prior runs)\n");
                    text.push_str(&crate::coordination::decomposition::render(&advice));
                }
            }
            if let (Some(scope), Some(resolved), ContextEnum::ChatMessage(message)) =
                (&launch_scope, &resolved_launch, &mut preview)
            {
                crate::privacy::records::carry_records_into(message, &resolved.prepended_messages)
                    .map_err(|_| "launch preview provenance unavailable")?;
                crate::privacy::records::carry_records_into(message, &parent_messages)
                    .map_err(|_| "launch parent provenance unavailable")?;
                scope.clear(&gcx, std::slice::from_ref(message))?;
                message.extra.insert("effective_model".into(), json!(model));
                message.extra.insert(
                    "launch_limits".into(),
                    json!({
                        "tokens": scope.limits.launch_context_tokens,
                        "messages": scope.limits.launch_context_messages,
                        "entries": scope.limits.launch_context_entries,
                        "bytes": scope.limits.launch_context_bytes,
                        "source_bytes": scope.limits.launch_source_bytes,
                    }),
                );
            }
            return Ok((false, vec![preview]));
        }

        let max_depth = crate::runtime_settings::current()
            .coordination
            .max_subchat_depth;
        if subchat_depth >= max_depth.saturating_sub(1) {
            return Err(format!("subchat depth limit ({max_depth}) exceeded"));
        }

        let _config = get_subagent_config(gcx.clone(), "subagent", None)
            .await
            .ok_or_else(|| "subagent config 'subagent' not found".to_string())?;
        if let (Some(scope), Some(resolved)) = (&launch_scope, &resolved_launch) {
            if !resolved.skipped.is_empty() {
                let skipped = resolved
                    .skipped
                    .iter()
                    .map(|entry| format!("{}: {}", entry.description, entry.reason))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(format!(
                    "launch context could not be fully resolved: {skipped}"
                ));
            }
            scope.clear(&gcx, &resolved.prepended_messages)?;
            let current =
                crate::coordination::scope::model_caller(&app, &parent_chat_id, &current_model)
                    .await
                    .map_err(|_| "launch caller changed during preparation")?;
            if current.source_root != scope.caller.source_root
                || current.checkout_root != scope.caller.checkout_root
            {
                return Err(
                    "launch scope changed during preparation; request a new preview".into(),
                );
            }
        }
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

#[cfg(test)]
fn parse_subagent_args(args: &HashMap<String, Value>) -> Result<SubagentArgs, String> {
    parse_subagent_args_with_limits(args, &crate::file_activity::settings())
}

fn parse_subagent_args_with_limits(
    args: &HashMap<String, Value>,
    limits: &refact_work_core::CoordinationSettings,
) -> Result<SubagentArgs, String> {
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
    let action = parse_optional_string(args, "action")?.unwrap_or_else(|| "run".to_string());
    let preview_only = match action.as_str() {
        "run" => false,
        "preview" => true,
        other => {
            return Err(format!(
                "argument `action` must be `run` or `preview`, got `{other}`"
            ))
        }
    };
    let launch_spec = launch_spec::parse_launch_spec_with_limits(args, limits)?;
    let profile = parse_profile_reference(args)?;
    let sandbox = match args.get("sandbox") {
        Some(value) => crate::agents::sandbox_contract::parse_requested(value)?,
        None => None,
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
        completion_push: PushMode::from_args(args)?,
        launch_spec,
        profile,
        preview_only,
        sandbox,
    })
}

fn parse_profile_reference(
    args: &HashMap<String, Value>,
) -> Result<Option<ProfileReference>, String> {
    let name_of = |value: &Value| -> Result<String, String> {
        value
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "argument `profile.name` must be a non-empty string".to_string())
    };
    match args.get("profile") {
        None | Some(Value::Null) => Ok(None),
        Some(value @ Value::String(_)) => Ok(Some(ProfileReference {
            name: name_of(value)?,
            revision: None,
        })),
        Some(Value::Object(object)) => {
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "name" | "revision"))
            {
                return Err("unsupported field in argument `profile`".to_string());
            }
            let name = name_of(object.get("name").unwrap_or(&Value::Null))?;
            let revision = match object.get("revision") {
                None | Some(Value::Null) => None,
                Some(Value::Number(number)) => Some(
                    number
                        .as_u64()
                        .filter(|revision| *revision > 0)
                        .ok_or_else(|| {
                            "argument `profile.revision` must be a positive integer".to_string()
                        })?,
                ),
                other => {
                    return Err(format!(
                        "argument `profile.revision` must be a positive integer: {other:?}"
                    ))
                }
            };
            Ok(Some(ProfileReference { name, revision }))
        }
        other => Err(format!(
            "argument `profile` must be a string or object: {other:?}"
        )),
    }
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
    deliverable: Option<&refact_chat_api::DeliverableContract>,
    handoff_limits: &refact_chat_api::HandoffLimits,
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
    if let Some(contract) = deliverable {
        prompt.push_str(&contract.render_instructions());
        prompt.push_str("\n\n");
    }
    prompt.push_str(&refact_chat_api::handoff::render_instructions(
        handoff_limits,
    ));
    prompt.push_str("\n\n");
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
            "✓ Accepted background subagent: {}",
            truncate_chars(task, 60)
        ),
        format!("- agent_id: {}", handle.agent_id),
        "- status: queued".to_string(),
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
            ("background_agent_status".to_string(), json!("queued")),
            ("child_chat_id".to_string(), json!(handle.child_chat_id)),
            ("launch_revision".to_string(), json!(handle.launch_revision)),
            ("launch_digest".to_string(), json!(handle.launch_digest)),
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

fn build_launch_preview_tool_result(
    args: &SubagentArgs,
    resolved: Option<&ResolvedLaunchContext>,
    profile: Option<&crate::agents::profile::ResolvedProfile>,
    tool_call_id: &str,
) -> ContextEnum {
    let mut lines = vec![
        "# Launch Preview (no child spawned)".to_string(),
        String::new(),
        format!("- task: {}", truncate_chars(&args.task, 200)),
        format!(
            "- worktree_mode: {}",
            match &args.worktree_mode {
                SpawnWorktreeMode::Inherit => "inherit".to_string(),
                SpawnWorktreeMode::Isolated { auto_merge } =>
                    format!("isolated (auto_merge={auto_merge})"),
            }
        ),
        format!(
            "- tools: {}",
            args.tools
                .as_ref()
                .map(|t| t.join(", "))
                .unwrap_or_else(|| "(inherit all)".to_string())
        ),
    ];
    match profile {
        None => lines.push("- profile: (none; run-only configuration)".to_string()),
        Some(profile) => {
            lines.push(String::new());
            lines.push("## Profile Provenance".to_string());
            lines.push(format!(
                "- profile: {} (revision {})",
                profile.name, profile.revision
            ));
            lines.push(format!(
                "- contributed: {}",
                label_or_dash(&profile.provenance.contributed)
            ));
            lines.push(format!(
                "- narrowed by this run: {}",
                label_or_dash(&profile.provenance.narrowed)
            ));
            for note in &profile.provenance.notes {
                lines.push(format!("- note: {note}"));
            }
        }
    }
    match resolved {
        None => lines.push("- launch_spec: (not provided; legacy defaults apply)".to_string()),
        Some(resolved) => {
            lines.push(String::new());
            lines.push("## Resolved Context".to_string());
            if resolved.prepended_messages.is_empty() {
                lines.push("(no context messages resolved)".to_string());
            } else {
                for message in &resolved.prepended_messages {
                    lines.push(format!(
                        "- [{}] {}",
                        message.role,
                        truncate_chars(&message.content.content_text_only(), 120)
                    ));
                }
            }
            if !resolved.skipped.is_empty() {
                lines.push(String::new());
                lines.push("## Skipped/Withheld Entries".to_string());
                for skipped in &resolved.skipped {
                    lines.push(format!("- {}: {}", skipped.description, skipped.reason));
                }
            }
            if !resolved.excluded.is_empty() {
                lines.push("## Fork Exclusions".into());
                for excluded in &resolved.excluded {
                    lines.push(format!("- {}: {}", excluded.description, excluded.reason));
                }
            }
            lines.push(String::new());
            lines.push(format!("- estimated_tokens: {}", resolved.estimated_tokens));
        }
    }
    let mut extra = Map::from_iter([
        ("preview".to_string(), json!(true)),
        (
            "skipped_count".to_string(),
            json!(resolved.map(|r| r.skipped.len()).unwrap_or(0)),
        ),
        (
            "estimated_tokens".to_string(),
            json!(resolved.map(|r| r.estimated_tokens).unwrap_or(0)),
        ),
        (
            "serialized_bytes".to_string(),
            json!(resolved.map(|r| r.serialized_bytes).unwrap_or(0)),
        ),
        (
            "context_sources".into(),
            json!(resolved
                .map(|resolved| resolved
                    .prepended_messages
                    .iter()
                    .filter_map(|message| message.extra.get("launch_context_source"))
                    .collect::<Vec<_>>())
                .unwrap_or_default()),
        ),
        (
            "skipped".into(),
            json!(resolved.map(|r| r.skipped.clone()).unwrap_or_default()),
        ),
        (
            "exclusions".into(),
            json!(resolved.map(|r| r.excluded.clone()).unwrap_or_default()),
        ),
    ]);
    if let Some(profile) = profile {
        extra.insert("profile".into(), json!(profile));
    }
    tool_message(lines.join("\n"), tool_call_id, extra)
}

fn label_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
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

    async fn test_context(
        parent_chat_id: &str,
    ) -> (Arc<AMutex<AtCommandsContext>>, tempfile::TempDir) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().expect("subagent test workspace");
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().canonicalize().unwrap()];
        install_test_caps(gcx.clone()).await;
        let app = AppState::from_gcx(gcx).await;
        let context = Arc::new(AMutex::new(
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
        ));
        (context, workspace)
    }

    #[serial(test_runner)]
    #[tokio::test]
    async fn an_unenforceable_sandbox_request_refuses_the_launch_before_dispatch() {
        let (ccx, _workspace) = test_context("parent").await;
        let app = ccx.lock().await.app.clone();
        let _runner =
            crate::agents::spawn::install_test_runner(Arc::new(|_gcx, _messages, _cfg| {
                panic!("a child was dispatched despite an unenforceable sandbox request")
            }));
        let mut args = args();
        args.insert("sandbox".to_string(), json!("workspace_write"));

        let error = ToolSubagent {
            config_path: String::new(),
        }
        .tool_execute(ccx.clone(), &"call".to_string(), &args)
        .await
        .unwrap_err();

        assert!(error.contains("launch refused before dispatch"), "{error}");
        assert!(error.contains("workspace_write"), "{error}");
        assert!(
            error.contains("would otherwise have run unsandboxed"),
            "{error}"
        );
        assert!(
            app.agents.list_all().await.is_empty(),
            "an agent record was created for a refused launch"
        );

        let mut accepted = args.clone();
        accepted.remove("sandbox");
        assert!(!ToolSubagent {
            config_path: String::new(),
        }
        .tool_execute(ccx, &"call".to_string(), &accepted)
        .await
        .is_err_and(|error| error.contains("launch refused before dispatch")));
    }

    #[test]
    fn the_target_files_schema_never_claims_enforcement_it_does_not_perform() {
        let schema = ToolSubagent {
            config_path: String::new(),
        }
        .tool_description()
        .input_schema;
        let target_files = schema["properties"]["target_files"]["description"]
            .as_str()
            .unwrap()
            .to_string();

        assert!(!target_files.contains("may edit only"));
        assert!(target_files.contains("declared intent"));
        assert!(target_files.contains("scope guard is strict"));
        assert!(schema["properties"]["sandbox"]["description"]
            .as_str()
            .unwrap()
            .contains("fails before dispatch"));
    }

    fn message(contexts: Vec<ContextEnum>) -> ChatMessage {
        match contexts.into_iter().next().expect("tool message") {
            ContextEnum::ChatMessage(message) => message,
            _ => panic!("expected chat message"),
        }
    }

    #[tokio::test]
    async fn public_launch_preview_enforces_scope_destination_and_preserves_source_records() {
        let (ccx, workspace) = test_context("launch-parent").await;
        let app = ccx.lock().await.app.clone();
        let mut parent = crate::chat::types::ChatSession::new("launch-parent".into());
        parent.thread.model = "test/default".into();
        parent.work_project_root = Some(workspace.path().into());
        app.chat
            .sessions
            .write()
            .await
            .insert("launch-parent".into(), Arc::new(AMutex::new(parent)));
        let (root, _) = crate::caps::project_model_defaults_state(app.gcx.clone()).await;
        app.gcx.caps_state.write().await.loaded_project_root = Some(root);
        let file = workspace.path().join("private.rs");
        std::fs::write(&file, "launch-source-body").unwrap();
        let mut request = args();
        request.insert("action".into(), json!("preview"));
        request.insert(
            "launch_spec".into(),
            json!({"context":[{"type":"file_range","path":"private.rs"}]}),
        );
        let execute = |request: HashMap<String, Value>| {
            let app = app.clone();
            let ccx = ccx.clone();
            async move {
                app.tool_registry
                    .execute_tool(
                        &ccx,
                        "agent",
                        Some("test/default"),
                        "preview",
                        "subagent",
                        request.into_iter().collect(),
                    )
                    .await
            }
        };
        let preview = execute(request.clone())
            .await
            .unwrap()
            .expect("subagent must be publicly reachable");
        let message = preview.messages.first().unwrap();
        assert!(message
            .content
            .content_text_only()
            .contains("launch-source-body"));
        assert!(crate::privacy::records::records_to_carry(&preview.messages)
            .unwrap()
            .iter()
            .any(|record| record.path == file.to_string_lossy()));
        assert!(app.agents.list_all().await.is_empty());
        let effective = &message.extra["effective_launch"];
        assert!(message.extra.get("launch_blocked").is_none());
        assert_eq!(effective["schema_version"], 1);
        assert_eq!(effective["revision"], 1);
        assert_eq!(effective["model"], message.extra["effective_model"]);
        assert_eq!(effective["mode"], "agent");
        match &effective["tools"] {
            Value::String(policy) => {
                assert_eq!(policy, "None");
                assert_eq!(message.extra["effective_tools"], json!([]));
            }
            policy => assert_eq!(policy["Only"], message.extra["effective_tools"]),
        }
        assert_eq!(effective["worktree_mode"], "inherit");
        assert_eq!(effective["stateful"], true);
        assert_eq!(effective["autonomous_no_confirm"], false);
        assert_eq!(effective["auto_approve_editing_tools"], false);
        assert_eq!(effective["auto_approve_dangerous_commands"], false);
        assert_eq!(
            effective["max_steps"],
            crate::runtime_settings::current()
                .coordination
                .subagent_default_steps
        );
        assert_eq!(effective["subchat_depth"], 1);
        assert!(effective["messages"].as_u64().unwrap() >= 1);
        assert!(effective["accepted_bytes"].as_u64().unwrap() > 0);
        assert!(
            effective["accepted_bytes"].as_u64().unwrap()
                <= effective["accepted_byte_limit"].as_u64().unwrap()
        );
        assert_eq!(message.extra["skipped"], json!([]));
        assert_eq!(message.extra["exclusions"], json!([]));
        assert!(app.agents.list_all().await.is_empty());
        let saved_depth = ccx.lock().await.subchat_depth;
        ccx.lock().await.subchat_depth = crate::runtime_settings::current()
            .coordination
            .max_subchat_depth;
        let blocked = execute(request.clone()).await.unwrap().unwrap();
        assert!(blocked.messages[0].extra["launch_blocked"]
            .as_str()
            .unwrap()
            .contains("depth"));
        assert!(blocked.messages[0].extra.get("effective_launch").is_none());
        assert!(blocked.messages[0]
            .content
            .content_text_only()
            .contains("## Launch Blocked"));
        ccx.lock().await.subchat_depth = saved_depth;
        assert!(app.agents.list_all().await.is_empty());
        let settings_path = crate::runtime_settings::project_coordination_path(workspace.path());
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(&settings_path, "launch_source_bytes: 1\n").unwrap();
        let bounded = execute(request.clone()).await.unwrap().unwrap();
        assert_eq!(bounded.messages[0].extra["skipped_count"], 1);
        assert_eq!(
            bounded.messages[0].extra["launch_limits"]["source_bytes"],
            1
        );
        assert!(!bounded.messages[0]
            .content
            .content_text_only()
            .contains("launch-source-body"));
        std::fs::write(&settings_path, "launch_source_bytes: 4096\n").unwrap();
        let restored = execute(request.clone()).await.unwrap().unwrap();
        assert_eq!(restored.messages[0].extra["skipped_count"], 0);
        assert!(restored.messages[0]
            .content
            .content_text_only()
            .contains("launch-source-body"));
        std::fs::write(&settings_path, "launch_source_bytes: 0\n").unwrap();
        assert!(execute(request.clone()).await.is_err());
        std::fs::write(&settings_path, "{}\n").unwrap();
        app.gcx.privacy_policy_load.write().unwrap().policy =
            Arc::new(refact_privacy::PrivacyPolicy {
                zones: vec![refact_privacy::Zone {
                    name: "private".into(),
                    patterns: vec![file.to_string_lossy().into_owned()],
                    send_to: vec!["other".into()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                }],
                ..Default::default()
            });
        let denied = execute(request.clone()).await.unwrap().unwrap();
        assert!(!denied.messages.iter().any(|message| message
            .content
            .content_text_only()
            .contains("launch-source-body")));
        request.insert("action".into(), json!("run"));
        assert!(execute(request.clone()).await.is_err());
        assert!(app.agents.list_all().await.is_empty());
        request.insert("action".into(), json!("preview"));
        request.insert("model_name".into(), json!("missing/model"));
        assert!(execute(request).await.is_err());
    }

    async fn install_profile(gcx: &Arc<crate::global_context::GlobalContext>, yaml: &str) {
        let mut registry =
            crate::yaml_configs::customization_registry::get_project_registry(gcx.clone())
                .await
                .expect("project registry");
        let config: crate::yaml_configs::customization_types::SubagentConfig =
            serde_yaml::from_str(yaml).expect("profile config");
        registry.subagents.insert(config.id.clone(), config);
        let root = crate::files_correction::get_project_dirs(gcx.clone())
            .await
            .first()
            .cloned()
            .expect("project root");
        gcx.project_registry_cache
            .write()
            .unwrap()
            .insert(root, registry);
    }

    fn reviewer_profile(tools: &str) -> String {
        format!(
            "schema_version: 7\nid: reviewer\ntitle: Reviewer\nmessages:\n  system_prompt: reviewer\nagent_profile:\n  tools: {tools}\n  role: Careful reviewer\n  max_steps: 4\n"
        )
    }

    async fn profile_parent() -> (
        Arc<AMutex<AtCommandsContext>>,
        AppState,
        Arc<AMutex<crate::chat::types::ChatSession>>,
        tempfile::TempDir,
    ) {
        let (ccx, workspace) = test_context("profile-parent").await;
        let app = ccx.lock().await.app.clone();
        let mut parent = crate::chat::types::ChatSession::new("profile-parent".into());
        parent.thread.model = "test/default".into();
        parent.thread.mode = "agent".into();
        parent.work_project_root = Some(workspace.path().into());
        let parent = Arc::new(AMutex::new(parent));
        app.chat
            .sessions
            .write()
            .await
            .insert("profile-parent".into(), parent.clone());
        let (root, _) = crate::caps::project_model_defaults_state(app.gcx.clone()).await;
        app.gcx.caps_state.write().await.loaded_project_root = Some(root);
        (ccx, app, parent, workspace)
    }

    async fn run_subagent(
        app: &AppState,
        ccx: &Arc<AMutex<AtCommandsContext>>,
        call: &str,
        request: HashMap<String, Value>,
    ) -> Result<Vec<ChatMessage>, String> {
        app.tool_registry
            .execute_tool(
                ccx,
                "agent",
                Some("test/default"),
                call,
                "subagent",
                request.into_iter().collect(),
            )
            .await
            .map(|result| {
                result
                    .expect("subagent must be publicly reachable")
                    .messages
            })
    }

    #[tokio::test]
    async fn launch_refusal_lists_skipped_entries() {
        let (ccx, app, _parent, _workspace) = profile_parent().await;
        let mut request = args();
        request.insert(
            "launch_spec".into(),
            json!({
                "token_budget": 1,
                "custom_instructions": "Never modify files.",
                "context": [
                    {"type": "raw_text", "label": "notes", "text": "Read only and be terse."},
                    {"type": "message_range", "start_message_id": "missing"}
                ]
            }),
        );
        let mut preview_request = request.clone();
        preview_request.insert("action".into(), json!("preview"));
        let preview = run_subagent(&app, &ccx, "preview", preview_request)
            .await
            .unwrap();
        let skipped = preview[0].extra["skipped"].as_array().unwrap();
        assert_eq!(skipped.len(), 3);
        let error = run_subagent(&app, &ccx, "run", request).await.unwrap_err();
        assert!(
            error.starts_with("launch context could not be fully resolved:"),
            "{error}"
        );
        for entry in skipped {
            let detail = format!(
                "{}: {}",
                entry["description"].as_str().unwrap(),
                entry["reason"].as_str().unwrap()
            );
            assert!(error.contains(&detail), "missing {detail:?} in {error}");
        }
        assert!(app.agents.list_all().await.is_empty());
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    #[serial(test_runner)]
    async fn accepted_profile_revision_is_pinned_against_a_later_profile_edit() {
        let (ccx, app, _parent, _workspace) = profile_parent().await;
        install_profile(&app.gcx, &reviewer_profile("[cat]")).await;
        let captured = Arc::new(std::sync::Mutex::new(None));
        let output = captured.clone();
        let _runner =
            crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
                *output.lock().unwrap() = Some((messages.clone(), config.tools.clone()));
                Box::pin(async move {
                    Ok(SubchatResult {
                        messages,
                        metering: Map::new(),
                        chat_id: config.chat_id,
                        aborted: false,
                    })
                })
            }));
        let mut request = args();
        request.insert("profile".into(), json!("reviewer"));

        let mut preview = request.clone();
        preview.insert("action".into(), json!("preview"));
        let previewed = run_subagent(&app, &ccx, "preview", preview.clone())
            .await
            .unwrap();
        let message = &previewed[0];
        assert_eq!(message.extra["profile"]["name"], "reviewer");
        let revision = message.extra["profile"]["revision"].as_u64().unwrap();
        assert!(revision > 0);
        assert!(message
            .content
            .content_text_only()
            .contains("## Profile Provenance"));
        assert!(message
            .content
            .content_text_only()
            .contains("- contributed: "));
        let contributed = message.extra["profile"]["provenance"]["contributed"]
            .as_array()
            .unwrap();
        assert!(contributed.iter().any(|entry| entry == "tools"));
        assert!(contributed.iter().any(|entry| entry == "instructions"));
        assert!(contributed.iter().any(|entry| entry == "max_steps"));
        let effective_tools = message.extra["effective_tools"].as_array().unwrap();
        assert!(effective_tools.iter().any(|tool| tool == "cat"));
        assert!(!effective_tools.iter().any(|tool| tool == "shell"));
        assert_eq!(message.extra["effective_launch"]["max_steps"], 4);
        assert!(app.agents.list_all().await.is_empty());

        let launched = run_subagent(&app, &ccx, "launch", request.clone())
            .await
            .unwrap();
        let agent = launched[0].extra["background_agent_id"]
            .as_str()
            .unwrap()
            .to_string();
        app.agents
            .wait("profile-parent", &agent, Duration::from_secs(5))
            .await
            .unwrap();
        let (child_messages, child_tools) = captured.lock().unwrap().clone().expect("child ran");
        assert!(child_messages.iter().any(|message| message
            .content
            .content_text_only()
            .contains("Careful reviewer")));
        assert!(child_tools.allows_tool("cat"));
        assert!(!child_tools.allows_tool("shell"));
        let accepted = app
            .agents
            .accepted_launch("profile-parent", &agent, 1024 * 1024)
            .await
            .unwrap();
        let accepted_bytes = accepted.as_bytes().to_vec();
        assert_eq!(
            accepted.decode::<Value>().unwrap()["profile"]["revision"],
            revision
        );

        install_profile(&app.gcx, &reviewer_profile("[cat, shell]")).await;
        let after_edit = run_subagent(&app, &ccx, "preview-2", preview.clone())
            .await
            .unwrap();
        let edited_revision = after_edit[0].extra["profile"]["revision"].as_u64().unwrap();
        assert_ne!(edited_revision, revision);
        let unchanged = app
            .agents
            .accepted_launch("profile-parent", &agent, 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(unchanged.as_bytes(), accepted_bytes.as_slice());
        assert_eq!(
            unchanged.decode::<Value>().unwrap()["profile"]["revision"],
            revision
        );

        let mut pinned = preview;
        pinned.insert(
            "profile".into(),
            json!({"name": "reviewer", "revision": revision}),
        );
        let error = run_subagent(&app, &ccx, "pinned", pinned)
            .await
            .unwrap_err();
        assert!(error.contains("is pinned"), "{error}");
    }

    fn handoff_block(path: &str) -> String {
        format!(
            "Status: DONE\n\n```handoff\n{{\"gaps\":[{{\"category\":\"missing_file\",\"detail\":\"needed the retry rule\",\"path\":\"{path}\",\"extra_steps\":3}}]}}\n```\n"
        )
    }

    #[tokio::test]
    #[serial(test_runner)]
    async fn a_child_handoff_declaration_is_recorded_and_an_absent_one_stays_unknown() {
        for declared in [true, false] {
            let (ccx, app, _parent, workspace) = profile_parent().await;
            let file = workspace.path().join("retry.rs");
            std::fs::write(&file, "fn retry() {}\n").unwrap();
            let path = file.to_string_lossy().into_owned();
            let final_text = if declared {
                handoff_block(&path)
            } else {
                "Status: DONE\nnothing declared".to_string()
            };
            let captured = Arc::new(std::sync::Mutex::new(None));
            let output = captured.clone();
            let _runner = crate::agents::spawn::install_test_runner(Arc::new(
                move |_gcx, mut messages, config| {
                    *output.lock().unwrap() = Some(messages.clone());
                    messages.push(ChatMessage::new("assistant".into(), final_text.clone()));
                    Box::pin(async move {
                        Ok(SubchatResult {
                            messages,
                            metering: Map::new(),
                            chat_id: config.chat_id,
                            aborted: false,
                        })
                    })
                },
            ));
            let launched = run_subagent(&app, &ccx, "handoff", args()).await.unwrap();
            let agent = launched[0].extra["background_agent_id"]
                .as_str()
                .unwrap()
                .to_string();
            let record = app
                .agents
                .wait("profile-parent", &agent, Duration::from_secs(5))
                .await
                .unwrap();
            assert_eq!(
                record.status,
                crate::agents::types::BgAgentStatus::Completed,
                "an absent declaration must never block completion"
            );
            let prompt = captured
                .lock()
                .unwrap()
                .clone()
                .unwrap()
                .iter()
                .map(|message| message.content.content_text_only())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(prompt.contains("# Handoff sufficiency (optional)"));
            assert!(prompt.contains("recorded as `unknown`, never as `sufficient`"));
            match (&record.handoff, declared) {
                (Some(report), true) => {
                    assert_eq!(
                        report.sufficiency,
                        refact_chat_api::HandoffSufficiency::GapsReported
                    );
                    assert_eq!(
                        report.source,
                        refact_chat_api::HANDOFF_SOURCE_CHILD_DECLARATION
                    );
                    assert_eq!(report.claimed_by, agent);
                    assert!(report.claimed_at_ms > 0);
                    assert_eq!(
                        report.gaps[0].category,
                        refact_chat_api::HandoffGapCategory::MissingFile
                    );
                    assert_eq!(report.gaps[0].path.as_deref(), Some(path.as_str()));
                    assert_eq!(report.gaps[0].extra_steps, Some(3));
                }
                (None, false) => {}
                other => panic!("unexpected handoff state: {other:?}"),
            }
            let summary = crate::agents::types::BackgroundAgentSummary::from(&record);
            assert_eq!(summary.handoff.is_some(), declared);
        }
    }

    #[tokio::test]
    async fn prior_handoff_gaps_surface_in_the_preview_without_widening_scope_or_authority() {
        let (ccx, app, _parent, workspace) = profile_parent().await;
        install_profile(&app.gcx, &reviewer_profile("[cat]")).await;
        let mut request = args();
        request.insert("profile".into(), json!("reviewer"));
        request.insert("action".into(), json!("preview"));
        let before = run_subagent(&app, &ccx, "before", request.clone())
            .await
            .unwrap();
        assert!(before[0].extra.get("handoff_metric").is_none());

        let file = workspace.path().join("retry.rs");
        std::fs::write(&file, "fn retry() {}\n").unwrap();
        let path = file.to_string_lossy().into_owned();
        let report = refact_chat_api::handoff::parse_report(
            &handoff_block(&path),
            "finished",
            7,
            &refact_chat_api::HandoffLimits::default(),
        )
        .unwrap();
        let (finished, _, _) = app
            .agents
            .create(crate::agents::types::CreateAgentRequest {
                parent_chat_id: "profile-parent".to_string(),
                parent_root_chat_id: Some("root-chat".to_string()),
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "subagent".to_string(),
                title: "Earlier reviewer child".to_string(),
                prompt: String::new(),
                target_files: Vec::new(),
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
            .install_profile_name(&finished.agent_id, "reviewer".to_string())
            .await
            .unwrap();
        app.agents
            .mark_completed(
                &finished.agent_id,
                crate::agents::types::AgentCompletion {
                    result_summary: "done".to_string(),
                    edited_files: Vec::new(),
                    diff_summary: None,
                    conflict_summary: None,
                    child_chat_id: Some("earlier-child".to_string()),
                    deliverable_status: None,
                    handoff: Some(report),
                },
            )
            .await
            .unwrap();

        let after = run_subagent(&app, &ccx, "after", request).await.unwrap();
        let metric = &after[0].extra["handoff_metric"];
        assert_eq!(metric["key"], "profile:reviewer");
        assert_eq!(metric["completions"], 1);
        assert_eq!(metric["with_gaps"], 1);
        assert_eq!(metric["gaps_by_category"]["missing_file"], 1);
        assert_eq!(metric["suggested_sources"], json!([path]));
        assert_eq!(metric["withheld_sources"], 0);
        let finding = &after[0].extra["handoff_finding"];
        assert_eq!(finding["status"], "insufficient_data");
        assert_eq!(finding["key"], "profile:reviewer");
        assert_eq!(finding["samples"], 1);
        let text = after[0].content.content_text_only();
        assert!(text.contains("## Handoff Sufficiency (prior children)"));
        assert!(text.contains("suggestion only; this launch's scope and authority are unchanged"));
        assert!(text.contains("## Handoff Brief Signal"));
        assert!(
            text.contains("insufficient_data"),
            "one child is not a trend: {text}"
        );
        assert_eq!(
            after[0].extra["effective_tools"],
            before[0].extra["effective_tools"]
        );
        assert_eq!(
            after[0].extra["effective_launch"]["tools"],
            before[0].extra["effective_launch"]["tools"]
        );
        assert_eq!(
            after[0].extra["effective_launch"]["max_steps"],
            before[0].extra["effective_launch"]["max_steps"]
        );
        assert_eq!(
            after[0].extra["profile"]["provenance"],
            before[0].extra["profile"]["provenance"]
        );
    }

    #[tokio::test]
    async fn unknown_profile_is_refused_before_any_record_or_ticket() {
        let (ccx, app, _parent, _workspace) = profile_parent().await;
        let mut request = args();
        request.insert("profile".into(), json!("not-defined"));
        let error = run_subagent(&app, &ccx, "unknown", request.clone())
            .await
            .unwrap_err();
        assert!(error.contains("not defined"), "{error}");
        assert!(app.agents.list_all().await.is_empty());
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);

        install_profile(
            &app.gcx,
            "schema_version: 7\nid: broken\nmessages:\n  system_prompt: broken\nagent_profile:\n  mdoel: typo\n",
        )
        .await;
        request.insert("profile".into(), json!("broken"));
        let error = run_subagent(&app, &ccx, "malformed", request)
            .await
            .unwrap_err();
        assert!(error.contains("is malformed"), "{error}");
        assert!(app.agents.list_all().await.is_empty());
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn profile_tools_narrow_to_the_caller_ceiling_and_runs_cannot_widen_the_profile() {
        let (ccx, app, parent, _workspace) = profile_parent().await;
        install_profile(&app.gcx, &reviewer_profile("[cat, shell]")).await;
        parent.lock().await.active_command.allowed_tools = vec!["cat".into()];
        let mut request = args();
        request.insert("profile".into(), json!("reviewer"));
        request.insert("action".into(), json!("preview"));
        let narrowed = run_subagent(&app, &ccx, "narrowed", request.clone())
            .await
            .unwrap();
        let message = &narrowed[0];
        assert_eq!(message.extra["effective_tools"], json!(["cat"]));
        let notes = message.extra["profile"]["provenance"]["notes"]
            .as_array()
            .unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].as_str().unwrap().contains("shell"));
        assert!(notes[0].as_str().unwrap().contains("not granted"));
        assert!(message.extra.get("launch_blocked").is_none());
        assert!(app.agents.list_all().await.is_empty());

        request.insert("tools".into(), json!("apply"));
        let error = run_subagent(&app, &ccx, "widened", request)
            .await
            .unwrap_err();
        assert!(error.contains("cannot widen"), "{error}");
        assert!(app.agents.list_all().await.is_empty());
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn a_launch_without_a_profile_keeps_its_current_behaviour() {
        let (ccx, app, _parent, _workspace) = profile_parent().await;
        install_profile(&app.gcx, &reviewer_profile("[cat]")).await;
        let mut request = args();
        request.insert("action".into(), json!("preview"));
        let previewed = run_subagent(&app, &ccx, "no-profile", request)
            .await
            .unwrap();
        let message = &previewed[0];
        assert!(message.extra.get("profile").is_none());
        assert!(message
            .content
            .content_text_only()
            .contains("- profile: (none; run-only configuration)"));
        assert!(message.extra["effective_tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "cat"));
        assert_eq!(
            message.extra["effective_launch"]["max_steps"],
            crate::runtime_settings::current()
                .coordination
                .subagent_default_steps
        );
        assert!(app.agents.list_all().await.is_empty());
    }

    #[test]
    fn profile_reference_accepts_a_name_or_a_pinned_revision() {
        let mut values = args();
        assert!(parse_subagent_args(&values).unwrap().profile.is_none());
        values.insert("profile".into(), json!("reviewer"));
        let parsed = parse_subagent_args(&values).unwrap().profile.unwrap();
        assert_eq!(parsed.name, "reviewer");
        assert_eq!(parsed.revision, None);
        values.insert("profile".into(), json!({"name":"reviewer","revision":42}));
        let parsed = parse_subagent_args(&values).unwrap().profile.unwrap();
        assert_eq!(parsed.revision, Some(42));
        for invalid in [
            json!(""),
            json!({}),
            json!({"name":"reviewer","revision":0}),
            json!({"name":"reviewer","tools":"cat"}),
            json!(7),
        ] {
            values.insert("profile".into(), invalid);
            assert!(parse_subagent_args(&values).is_err());
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
            "push",
        ] {
            assert!(
                properties.contains_key(present),
                "{present} must be exposed"
            );
        }
    }

    #[tokio::test]
    #[serial(test_runner)]
    async fn public_fork_launch_uses_completed_snapshot_and_reports_queued_without_side_effects() {
        let (ccx, workspace) = test_context("fork-parent").await;
        let app = ccx.lock().await.app.clone();
        let mut parent = crate::chat::types::ChatSession::new("fork-parent".into());
        parent.thread.model = "test/default".into();
        parent.work_project_root = Some(workspace.path().into());
        app.chat
            .sessions
            .write()
            .await
            .insert("fork-parent".into(), Arc::new(AMutex::new(parent)));
        let (root, _) = crate::caps::project_model_defaults_state(app.gcx.clone()).await;
        app.gcx.caps_state.write().await.loaded_project_root = Some(root);
        let completed = ChatMessage {
            message_id: "completed-answer".into(),
            role: "assistant".into(),
            content: ChatContent::SimpleText("completed parent answer".into()),
            thinking_blocks: Some(vec![
                json!({"type":"thinking","thinking":"exact thinking","signature":"opaque-signature"}),
            ]),
            ..Default::default()
        };
        let pending: ChatMessage = serde_json::from_value(json!({
            "message_id":"unfinished", "role":"assistant", "content":"pending",
            "tool_calls":[{"id":"pending-call","type":"function","function":{"name":"shell","arguments":"{}"}}]
        })).unwrap();
        ccx.lock().await.messages = vec![
            ChatMessage::new("system".into(), "parent policy is not child policy".into()),
            completed.clone(),
            pending,
        ];
        app.work_scheduler
            .configure(refact_work_core::CoordinationSettings {
                max_active_engine: 1,
                max_active_project: 1,
                max_active_root: 1,
                ..Default::default()
            })
            .unwrap();
        let blocker = app
            .work_scheduler
            .reserve(refact_work_core::AdmissionRequest::new(
                refact_work_core::WorkKey::background_agent("blocker").run(1),
                refact_work_core::WorkScope::new(
                    refact_work_core::ProjectKey::new("blocker"),
                    refact_work_core::CheckoutKey::new("blocker"),
                ),
            ))
            .unwrap()
            .acquire()
            .await
            .unwrap();
        let captured = Arc::new(std::sync::Mutex::new(None));
        let output = captured.clone();
        let _runner =
            crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
                *output.lock().unwrap() = Some((
                    crate::subchat::prepare_child_history(&messages, &config),
                    config,
                ));
                Box::pin(async move {
                    Ok(SubchatResult {
                        messages,
                        metering: Map::new(),
                        chat_id: None,
                        aborted: false,
                    })
                })
            }));
        let mut request = args();
        request.insert(
            "launch_spec".into(),
            json!({"history_mode":"fork_completed_prefix"}),
        );
        let result = app
            .tool_registry
            .execute_tool(
                &ccx,
                "agent",
                Some("test/default"),
                "fork-call",
                "subagent",
                request.into_iter().collect(),
            )
            .await
            .unwrap()
            .expect("public fork launch");
        let receipt = &result.messages[0];
        assert_eq!(receipt.extra["background_agent_status"], "queued");
        assert!(receipt.extra["worktree_branch"].is_null());
        let agent = receipt.extra["background_agent_id"].as_str().unwrap();
        let child = receipt.extra["child_chat_id"].as_str().unwrap();
        assert!(captured.lock().unwrap().is_none());
        assert!(!app.chat.sessions.read().await.contains_key(child));
        let launch_limit = crate::runtime_settings::coordination_for_project(workspace.path())
            .await
            .unwrap()
            .launch_context_bytes;
        let accepted_bytes = app
            .agents
            .accepted_launch("fork-parent", agent, launch_limit)
            .await
            .unwrap();
        let accepted = accepted_bytes.decode::<Value>().unwrap();
        assert_eq!(accepted["revision"], 1);
        assert_eq!(receipt.extra["launch_revision"], accepted["revision"]);
        assert_eq!(receipt.extra["launch_digest"], {
            use sha2::Digest;
            hex::encode(sha2::Sha256::digest(accepted_bytes.as_bytes()))
        });
        assert_eq!(accepted["config"]["chat_id"], child);
        assert_eq!(accepted["config"]["model"], "test/default");
        assert_eq!(accepted["config"]["autonomous_no_confirm"], false);
        assert!(accepted["config"].get("parent_subchat_tx").is_none());
        assert!(app
            .agents
            .accepted_launch("foreign-parent", agent, launch_limit)
            .await
            .is_err());
        assert!(app
            .agents
            .save_accepted_launch(agent, &accepted_bytes)
            .await
            .is_err());
        let parent = app
            .chat
            .sessions
            .read()
            .await
            .get("fork-parent")
            .unwrap()
            .clone();
        parent.lock().await.thread.autonomous_no_confirm = true;
        ccx.lock().await.messages.clear();
        drop(blocker);
        app.agents
            .wait("fork-parent", agent, Duration::from_secs(5))
            .await
            .unwrap();
        let (messages, config) = captured.lock().unwrap().clone().expect("child ran");
        assert!(!config.autonomous_no_confirm);
        assert!(!config.auto_approve_editing_tools);
        assert!(!config.auto_approve_dangerous_commands);
        assert_eq!(
            app.agents
                .accepted_launch("fork-parent", agent, launch_limit)
                .await
                .unwrap()
                .decode::<Value>()
                .unwrap(),
            accepted
        );
        assert_eq!(config.history_source_model.as_deref(), Some("test/default"));
        let source = messages
            .iter()
            .find(|message| message.message_id == "completed-answer")
            .unwrap();
        assert_eq!(source.thinking_blocks, completed.thinking_blocks);
        assert_eq!(
            source.extra["launch_context_source"]["type"],
            "completed_prefix"
        );
        assert!(!messages
            .iter()
            .any(|message| message.message_id == "unfinished"));
        assert!(!messages.iter().any(|message| message
            .content
            .content_text_only()
            .contains("parent policy is not child policy")));
    }

    #[tokio::test]
    #[serial(test_runner)]
    async fn public_launch_binds_deliverable_contract_and_reports_structural_outcome() {
        for met in [false, true] {
            let (ccx, workspace) = test_context("contract-parent").await;
            let app = ccx.lock().await.app.clone();
            let mut parent = crate::chat::types::ChatSession::new("contract-parent".into());
            parent.thread.model = "test/default".into();
            parent.work_project_root = Some(workspace.path().into());
            crate::chat::goal_role::install_goal(
                &mut parent,
                "agent",
                "Parent goal body",
                true,
                refact_chat_api::GoalBudget::default(),
            );
            crate::chat::plan_role::install_plan(&mut parent, "agent", "Parent plan body");
            parent
                .queue_processor_running
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let parent_messages = parent.messages.clone();
            app.chat
                .sessions
                .write()
                .await
                .insert("contract-parent".into(), Arc::new(AMutex::new(parent)));
            ccx.lock().await.messages = parent_messages;
            let (root, _) = crate::caps::project_model_defaults_state(app.gcx.clone()).await;
            app.gcx.caps_state.write().await.loaded_project_root = Some(root);
            let captured = Arc::new(std::sync::Mutex::new(None));
            let output = captured.clone();
            let _runner = crate::agents::spawn::install_test_runner(Arc::new(
                move |_gcx, mut messages, config| {
                    *output.lock().unwrap() = Some((messages.clone(), config.tools.clone()));
                    let final_text = if met {
                        "# Findings\nfound it\n# Evidence\ncargo test exit 0\n```json\n{\"status\":\"done\"}\n```\n"
                    } else {
                        "Status: DONE\nNo sections at all."
                    };
                    messages.push(ChatMessage::new("assistant".into(), final_text.into()));
                    Box::pin(async move {
                        Ok(SubchatResult {
                            messages,
                            metering: Map::new(),
                            chat_id: config.chat_id,
                            aborted: false,
                        })
                    })
                },
            ));
            let contract = json!({
                "required_sections": ["Findings"],
                "output_schema": {"type":"object","required":["status"],"properties":{"status":{"type":"string"}}},
                "evidence_required": true
            });
            let mut request = args();
            request.insert(
                "launch_spec".into(),
                json!({"include_parent_goal": true, "include_parent_plan": true, "deliverable": contract}),
            );
            let mut preview = request.clone();
            preview.insert("action".into(), json!("preview"));
            let previewed = app
                .tool_registry
                .execute_tool(
                    &ccx,
                    "agent",
                    Some("test/default"),
                    "preview",
                    "subagent",
                    preview.into_iter().collect(),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(previewed.messages[0].extra["deliverable"], contract);
            let sources = previewed.messages[0].extra["context_sources"]
                .as_array()
                .unwrap();
            assert!(sources.iter().any(|source| source["type"] == "parent_goal"
                && source["trusted_as_instructions"] == false));
            assert!(sources
                .iter()
                .any(|source| source["type"] == "parent_plan" && source["historical"] == true));
            assert!(app.agents.list_all().await.is_empty());
            let result = app
                .tool_registry
                .execute_tool(
                    &ccx,
                    "agent",
                    Some("test/default"),
                    "launch",
                    "subagent",
                    request.into_iter().collect(),
                )
                .await
                .unwrap()
                .unwrap();
            let agent = result.messages[0].extra["background_agent_id"]
                .as_str()
                .unwrap()
                .to_string();
            let record = app
                .agents
                .wait("contract-parent", &agent, Duration::from_secs(5))
                .await
                .unwrap();
            assert_eq!(
                record.status,
                crate::agents::types::BgAgentStatus::Completed
            );
            assert_eq!(
                serde_json::to_value(record.deliverable.as_ref().unwrap()).unwrap(),
                contract
            );
            let status = record.deliverable_status.clone().expect("evaluated");
            assert_eq!(status.check, refact_chat_api::DELIVERABLE_CHECK_STRUCTURAL);
            assert_eq!(status.met, met);
            if !met {
                assert_eq!(status.missing_sections, vec!["Findings"]);
                assert!(status.evidence_missing);
                assert!(status.schema_error.is_some());
            }
            let (messages, tools) = captured.lock().unwrap().clone().unwrap();
            assert!(matches!(tools, ToolsPolicy::Only(_)));
            let prompt = messages
                .iter()
                .map(|message| message.content.content_text_only())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(prompt.contains("# Deliverable contract"));
            assert!(prompt.contains("`# Findings`"));
            assert!(prompt.contains("Parent goal body"));
            assert!(prompt.contains("Parent plan body"));
            assert!(prompt.contains("It is not the child's goal"));
            let child_goal = crate::chat::goal_snapshot_from_messages(&messages, None);
            assert!(child_goal.is_none());
            let parent_session = app
                .chat
                .sessions
                .read()
                .await
                .get("contract-parent")
                .cloned()
                .unwrap();
            let notice = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let found = {
                        let session = parent_session.lock().await;
                        session
                            .messages
                            .iter()
                            .chain(
                                session
                                    .pending_deliveries
                                    .iter()
                                    .flat_map(|delivery| delivery.messages.iter()),
                            )
                            .map(|message| message.content.content_text_only())
                            .find(|text| text.contains("[background subagent finished]"))
                    };
                    if let Some(found) = found {
                        break found;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("completion notice");
            assert!(notice.contains(if met {
                "deliverable_contract: met"
            } else {
                "deliverable_contract: UNMET"
            }));
            if !met {
                assert!(notice.contains("missing sections: Findings"));
                assert!(notice.contains("claims are unverified"));
            }
            let summary = crate::agents::types::BackgroundAgentSummary::from(&record);
            assert_eq!(
                summary.deliverable_status.as_ref().map(|status| status.met),
                Some(met)
            );
        }
    }

    #[tokio::test]
    #[serial(test_runner)]
    async fn queued_public_launch_rechecks_scope_and_privacy_before_runner() {
        for change_scope in [false, true] {
            let (ccx, workspace) = test_context("queued-parent").await;
            let app = ccx.lock().await.app.clone();
            let mut parent = crate::chat::types::ChatSession::new("queued-parent".into());
            parent.thread.model = "test/default".into();
            parent.work_project_root = Some(workspace.path().into());
            let parent = Arc::new(AMutex::new(parent));
            app.chat
                .sessions
                .write()
                .await
                .insert("queued-parent".into(), parent.clone());
            let (root, _) = crate::caps::project_model_defaults_state(app.gcx.clone()).await;
            app.gcx.caps_state.write().await.loaded_project_root = Some(root);
            std::fs::write(workspace.path().join("context.txt"), "accepted source body").unwrap();
            app.work_scheduler
                .configure(refact_work_core::CoordinationSettings {
                    max_active_engine: 1,
                    max_active_project: 1,
                    max_active_root: 1,
                    ..Default::default()
                })
                .unwrap();
            let blocker = app
                .work_scheduler
                .reserve(refact_work_core::AdmissionRequest::new(
                    refact_work_core::WorkKey::background_agent("blocker").run(1),
                    refact_work_core::WorkScope::new(
                        refact_work_core::ProjectKey::new("blocker"),
                        refact_work_core::CheckoutKey::new("blocker"),
                    ),
                ))
                .unwrap()
                .acquire()
                .await
                .unwrap();
            let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let observed = ran.clone();
            let _runner =
                crate::agents::spawn::install_test_runner(Arc::new(move |_, messages, _| {
                    observed.store(true, std::sync::atomic::Ordering::SeqCst);
                    Box::pin(async move {
                        Ok(SubchatResult {
                            messages,
                            metering: Map::new(),
                            chat_id: None,
                            aborted: false,
                        })
                    })
                }));
            let mut request = args();
            request.insert(
                "launch_spec".into(),
                json!({"context":[{"type":"file_range","path":"context.txt"}]}),
            );
            let result = app
                .tool_registry
                .execute_tool(
                    &ccx,
                    "agent",
                    Some("test/default"),
                    "queued-call",
                    "subagent",
                    request.into_iter().collect(),
                )
                .await
                .unwrap()
                .unwrap();
            let receipt = &result.messages[0];
            let agent = receipt.extra["background_agent_id"].as_str().unwrap();
            let child = receipt.extra["child_chat_id"].as_str().unwrap();
            let foreign = tempfile::tempdir().unwrap();
            if change_scope {
                parent.lock().await.work_project_root = Some(foreign.path().into());
            } else {
                app.gcx.privacy_policy_load.write().unwrap().policy =
                    Arc::new(refact_privacy::PrivacyPolicy {
                        blocked: vec!["context.txt".into()],
                        ..Default::default()
                    });
            }
            drop(blocker);
            let stopped = app
                .agents
                .wait("queued-parent", agent, Duration::from_secs(5))
                .await
                .unwrap();
            assert_eq!(stopped.status, crate::agents::types::BgAgentStatus::Failed);
            assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
            assert!(!app.chat.sessions.read().await.contains_key(child));
            assert!(stopped.worktree_id.is_none());
        }
    }

    #[test]
    fn completion_push_defaults_to_append_and_validates_modes() {
        assert_eq!(
            parse_subagent_args(&args()).unwrap().completion_push,
            PushMode::Append
        );
        let mut values = args();
        values.insert("push".to_string(), json!("preempt"));
        assert_eq!(
            parse_subagent_args(&values).unwrap().completion_push,
            PushMode::Preempt
        );
        values.insert("push".to_string(), json!({}));
        assert!(parse_subagent_args(&values).is_err());
    }

    #[test]
    fn explicit_tools_accept_editing_aliases_without_adding_authority() {
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
        assert_eq!(tools, ["apply_patch", "search_pattern"]);
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
        let (ccx, _workspace) = test_context("parent").await;
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
        args.insert("push".to_string(), json!("preempt"));
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
        assert_eq!(result.completion_push, PushMode::Preempt);
        let (tools, prompt) = captured.lock().unwrap().clone().unwrap();
        assert!(
            matches!(tools, ToolsPolicy::Only(ref tools) if tools.iter().any(|name| name == "cat"))
        );
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
    async fn omitted_tools_cannot_add_force_tools_outside_parent_mode() {
        let (ccx, _workspace) = test_context("restricted-parent").await;
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
        let mut expected = crate::tools::tools_list::get_tools_for_mode(
            app.gcx.clone(),
            "NO_TOOLS",
            Some("test/default"),
        )
        .await
        .into_iter()
        .map(|tool| tool.tool_description().name)
        .collect::<Vec<_>>();
        expected.sort();
        assert!(!expected.is_empty());
        assert!(SUBAGENT_FORCE_TOOLS
            .iter()
            .any(|tool| !expected.iter().any(|name| name == tool)));
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
