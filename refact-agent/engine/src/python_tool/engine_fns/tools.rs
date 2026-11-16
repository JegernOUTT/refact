use std::collections::HashSet;

use futures::future::BoxFuture;
use refact_tool_api::MatchConfirmDenyResult;
use serde_json::{json, Value};

use super::common::to_value;
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput};
use crate::integrations::mcp::redaction::scrub_text;
use crate::tools::shell_gate;

/// Tools whose own confirmation check runs the shell gate (LLM validation and an audit line).
const SHELL_GATED: &[&str] = &["shell", "process_start", "shell_service"];

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "tools.list",
            summary: "Every tool registered in the engine with group, source and enabled flag, which of them this chat has, and why unavailable ones are gated. MCP tools: see integrations.list.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{
                "group":{"type":"string","minLength":1,"description":"Keep only this group."},
                "enabled_only":{"type":"boolean","description":"Only enabled tools; false by default."}
            }}),
            output: || Some(json!({"type":"object","required":["groups","gated"],"properties":{"groups":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"category":{"type":"string"},"tools":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"display_name":{"type":"string"},"enabled":{"type":"boolean"},"source_type":{"type":"string"},"allow_parallel":{"type":"boolean"},"in_this_chat":{"type":"boolean"}}}}}}},"gated":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"capability":{"type":"string"},"reason":{"type":"string"},"enablement":{"type":"string"}}}}}})),
            handler: list,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "tools.check",
            summary: "Whether a call of a tool with given arguments would run, ask the user or be denied in this chat, and which rule decides. Executes nothing; for shell commands the LLM validation is not simulated and no audit line is written.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["tool","args"],"properties":{
                "tool":{"type":"string","minLength":1,"description":"Tool name in this chat's tools."},
                "args":{"type":"object","description":"The call's arguments."}
            }}),
            output: || Some(json!({"type":"object","required":["tool","decision","rule","layers"],"properties":{"tool":{"type":"string"},"decision":{"enum":["auto","ask","deny"]},"rule":{"type":"string"},"command":{"type":"string"},"layers":{"type":"object","properties":{"tool_rule":{"enum":["pass","confirmation","deny"]},"mode_rule":{"enum":["ask","deny","auto"]},"auto_approved_by_flag":{"type":"boolean"},"forced_ask":{"enum":["protected_config","automation","agent_shell"]}}}}})),
            handler: check,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "tools.shell_policy",
            summary: "This project's shell command policy: mode, deny/ask/allow rules, LLM validation settings and, on request, the risk catalogue.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"catalogue":{"type":"boolean","description":"Include the risk catalogue; false by default."}}}),
            output: || Some(json!({"type":"object","required":["mode","deny","ask","allow","llm_validation"],"properties":{"mode":{"enum":["strict","balanced","permissive","yolo"]},"deny":{"type":"array"},"ask":{"type":"array"},"allow":{"type":"array"},"trust_caller_confirmation":{"type":"boolean"},"llm_validation":{"type":"object"},"execution":{"type":"object"},"catalogue":{"type":"array"},"path":{"type":"string"}}})),
            handler: shell_policy,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "tools.shell_test",
            summary: "Dry-run a shell command against this project's shell policy: decision, rule, risk level and segments (LLM validation not included).",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["command"],"properties":{"command":{"type":"string","minLength":1,"maxLength":10000}}}),
            output: || Some(json!({"type":"object","required":["decision","rule","reason","segments"],"properties":{"decision":{"enum":["pass","confirmation","deny"]},"rule":{"type":"string"},"reason":{"type":"string"},"risk_level":{"type":["string","null"]},"segments":{"type":"array","items":{"type":"string"}}}})),
            handler: shell_test,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "tools.shell_audit",
            summary: "Recent shell policy decisions in this project, newest first, with tokens scrubbed from commands. Child chats see only their own tree's entries.",
            class: EngineClass::RedactedRead,
            input: || json!({"type":"object","additionalProperties":false,"properties":{
                "limit":{"type":"integer","minimum":1,"maximum":500,"description":"Entries; 50 by default."},
                "chat_id":{"type":"string","minLength":1,"description":"Only this chat's entries."}
            }}),
            output: || Some(json!({"type":"object","required":["entries","total","truncated"],"properties":{"entries":{"type":"array","items":{"type":"object","properties":{"ts_ms":{"type":"integer"},"chat_id":{"type":"string"},"command":{"type":"string"},"decision":{"type":"string"},"layer":{"type":"string"},"rule":{"type":"string"},"risk_level":{"type":"string"}}}},"total":{"type":"integer"},"truncated":{"type":"boolean"}}})),
            handler: shell_audit,
            preview: None,
            timeout: None,
        },
    ]
    .into_iter()
    .chain(change_functions())
    .collect()
}

fn list<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let group_filter = args.get("group").and_then(Value::as_str);
        let enabled_only = args.get("enabled_only").and_then(Value::as_bool) == Some(true);
        let in_chat: HashSet<String> = match super::chat::this_session(ctx).await {
            Ok(session) => session
                .lock()
                .await
                .tool_catalog
                .as_ref()
                .map(|c| c.index.tools.iter().map(|t| t.name.clone()).collect())
                .unwrap_or_default(),
            Err(_) => HashSet::new(),
        };
        let (groups, gated) =
            crate::tools::tools_list::tool_groups_with_gating(ctx.app.gcx.clone()).await;
        let (mut tools_total, mut enabled_total, mut chat_total, mut group_total) = (0, 0, 0, 0);
        let mut rows = Vec::new();
        for group in groups {
            if group_filter.is_some_and(|g| g != group.name) || group.tools.is_empty() {
                continue;
            }
            let tools: Vec<Value> = group
                .tools
                .iter()
                .filter_map(|tool| {
                    let desc = tool.tool_description();
                    let enabled = tool.config().unwrap_or_default().enabled;
                    if enabled_only && !enabled {
                        return None;
                    }
                    let here = in_chat.contains(&desc.name);
                    tools_total += 1;
                    enabled_total += usize::from(enabled);
                    chat_total += usize::from(here);
                    Some(json!({"name":desc.name,"display_name":desc.display_name,"enabled":enabled,"source_type":desc.source.source_type,"allow_parallel":desc.allow_parallel,"in_this_chat":here}))
                })
                .collect();
            group_total += 1;
            rows.push(json!({"name":group.name,"category":group.category,"tools":tools}));
        }
        let mut gated: Vec<Value> = gated
            .into_iter()
            .map(|(name, g)| json!({"name":name,"capability":g.capability,"reason":g.unavailable_reason,"enablement":g.enablement}))
            .collect();
        gated.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        Ok(EngineOutput::new(
            json!({"groups":rows,"gated":gated}),
            format!("{tools_total} tools in {group_total} groups, {enabled_total} enabled, {chat_total} in this chat; MCP tools: see integrations.list"),
        ))
    })
}

fn tool_rule_name(result: &MatchConfirmDenyResult) -> &'static str {
    match result {
        MatchConfirmDenyResult::PASS => "pass",
        MatchConfirmDenyResult::CONFIRMATION => "confirmation",
        MatchConfirmDenyResult::DENY => "deny",
    }
}

fn check<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let tool = args["tool"].as_str().unwrap_or_default().to_string();
        let call_args = args["args"].clone();
        let session = super::chat::this_session(ctx).await?;
        let (mut thread, catalog, pool, allowed_tools) = {
            let s = session.lock().await;
            (
                s.thread.clone(),
                s.tool_catalog.clone(),
                s.turn_tool_pool.clone(),
                s.active_command.allowed_tools.clone(),
            )
        };
        let catalog = catalog
            .ok_or_else(|| EngineError::Failed("this chat has no tool catalog yet".into()))?;
        let resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(&tool);
        let tool = catalog
            .index
            .tools
            .iter()
            .find(|t| t.name == tool)
            .or_else(|| catalog.index.tools.iter().find(|t| t.name == resolved))
            .map(|found| found.name.clone())
            .ok_or_else(|| EngineError::Type(format!("{tool} is not in this chat's tools")))?;
        let mode = crate::yaml_configs::customization_registry::get_mode_config(
            ctx.app.gcx.clone(),
            &thread.mode,
            Some(&thread.model),
        )
        .await;
        let rules = mode.map(|m| m.tool_confirm.rules).unwrap_or_default();
        let mode_rule =
            crate::yaml_configs::customization_registry::match_tool_confirm_action(&rules, &tool);
        thread.auto_approve_dangerous_commands &=
            crate::agents::approval::dangerous_confirmation_enabled(&ctx.app, &thread.id).await;
        let mut layers = json!({});
        if let Some(rule) = &mode_rule {
            layers["mode_rule"] = json!(rule);
        }
        let (decision, rule, command, auto_flag) = if SHELL_GATED.contains(&tool.as_str()) {
            let command = call_args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let map: std::collections::HashMap<String, Value> = call_args
                .as_object()
                .map(|m| m.clone().into_iter().collect())
                .unwrap_or_default();
            let roots = match ctx.ccx.lock().await.execution_scope.as_ref() {
                Some(scope) => vec![scope.effective_root().to_path_buf()],
                None => {
                    crate::files_correction::get_unscoped_project_dirs(ctx.app.gcx.clone()).await
                }
            };
            let policy = shell_gate::load_policy(ctx.app.gcx.clone()).await;
            let outcome = shell_gate::evaluate(
                &command,
                &policy,
                &shell_gate::GateContext {
                    workspace_roots: roots,
                    needs_confirmation: shell_gate::parse_needs_confirmation(&map),
                },
            );
            layers["tool_rule"] = json!(tool_rule_name(&outcome.decision.result));
            let allowed_by_list = !allowed_tools.is_empty() && allowed_tools.contains(&tool);
            let flag = crate::chat::tools::should_auto_approve_confirmation(&thread, &tool)
                || allowed_by_list;
            let mut action = crate::chat::tools::compute_final_action(
                &outcome.decision.result,
                mode_rule.as_deref(),
                flag,
                &tool,
            );
            if action != "deny"
                && !flag
                && outcome.decision.result == MatchConfirmDenyResult::CONFIRMATION
                && thread
                    .task_meta
                    .as_ref()
                    .is_some_and(|m| matches!(m.role.as_str(), "agent" | "agents"))
            {
                action = "ask";
                layers["forced_ask"] = json!("agent_shell");
            }
            let manual = crate::chat::tools::compute_final_action(
                &outcome.decision.result,
                mode_rule.as_deref(),
                false,
                &tool,
            );
            let rule = match mode_rule.as_deref() {
                Some(action) => format!("mode policy: {action}"),
                None => outcome.decision.rule.clone(),
            };
            let decision = match action {
                "deny" => "deny",
                "ask" => "ask",
                _ => "auto",
            };
            (
                decision,
                rule,
                Some(command),
                flag && manual == "ask" && decision == "auto",
            )
        } else {
            let call: crate::call_validation::ChatToolCall = serde_json::from_value(
                json!({"id":"engine-check","type":"function","function":{"name":tool,"arguments":call_args.to_string()}}),
            )
            .map_err(|e| EngineError::Type(e.to_string()))?;
            let messages = ctx.ccx.lock().await.messages.clone();
            let (asks, denials, auto_suppressed) =
                crate::chat::tools::check_tools_confirmation_reporting_auto(
                    ctx.app.clone(),
                    std::slice::from_ref(&call),
                    &messages,
                    &thread,
                    Some(&catalog),
                    &thread.mode,
                    Some(&thread.model),
                    &allowed_tools,
                    "python",
                    pool.as_ref(),
                )
                .await;
            let (decision, reason) = match (denials.first(), asks.first()) {
                (Some(d), _) => ("deny", Some(d)),
                (None, Some(a)) => ("ask", Some(a)),
                (None, None) => ("auto", None),
            };
            let rule = reason.map(|r| r.rule.clone()).unwrap_or_default();
            if rule == crate::chat::tools::PROTECTED_CONFIG_APPROVAL {
                layers["forced_ask"] = json!("protected_config");
            } else if rule == crate::scheduler::automation_approval::HUMAN_AUTOMATION_APPROVAL {
                layers["forced_ask"] = json!("automation");
            }
            let command = reason.map(|r| r.command.clone()).filter(|c| !c.is_empty());
            (decision, rule, command, auto_suppressed.contains(&call.id))
        };
        layers["auto_approved_by_flag"] = json!(auto_flag);
        let mut data = json!({"tool":tool,"decision":decision,"rule":rule,"layers":layers});
        if let Some(command) = &command {
            data["command"] = json!(command);
        }
        let shown = command
            .as_deref()
            .map(|c| format!(" `{}`", c.chars().take(80).collect::<String>()))
            .unwrap_or_default();
        let mut text = format!(
            "{tool}{shown} -> {decision}{}",
            if rule.is_empty() {
                String::new()
            } else {
                format!(" ({rule})")
            }
        );
        if SHELL_GATED.contains(&tool.as_str()) {
            text.push_str("; LLM validation not simulated");
        }
        Ok(EngineOutput::new(data, text))
    })
}

fn shell_policy<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let gcx = ctx.app.gcx.clone();
        let policy = shell_gate::load_policy(gcx.clone()).await;
        let mut data =
            to_value(crate::http::routers::v1::shell_policy::response_from_policy(policy))?;
        if args.get("catalogue").and_then(Value::as_bool) != Some(true) {
            if let Some(map) = data.as_object_mut() {
                map.remove("catalogue");
            }
        }
        if let Some(root) = crate::files_correction::get_unscoped_project_dirs(gcx)
            .await
            .first()
        {
            data["path"] = json!(shell_gate::policy_path(root));
        }
        let count = |key: &str| data[key].as_array().map_or(0, Vec::len);
        let text = format!(
            "mode {}, {} deny / {} ask / {} allow rules",
            data["mode"].as_str().unwrap_or(""),
            count("deny"),
            count("ask"),
            count("allow")
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn shell_test<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let command = args["command"].as_str().unwrap_or_default();
        let data =
            crate::http::routers::v1::shell_policy::shell_policy_test(&ctx.app, command).await;
        let text = format!(
            "{} ({}){}",
            data["decision"].as_str().unwrap_or(""),
            data["reason"].as_str().unwrap_or(""),
            data["risk_level"]
                .as_str()
                .map(|r| format!(", risk {r}"))
                .unwrap_or_default()
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn shell_audit<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
        let chat_filter = args.get("chat_id").and_then(Value::as_str);
        let (child, root) = {
            let context = ctx.ccx.lock().await;
            (
                context.background_agent_id.is_some() || context.subchat_depth > 0,
                if context.root_chat_id.is_empty() {
                    ctx.chat_id.clone()
                } else {
                    context.root_chat_id.clone()
                },
            )
        };
        let own_tree: Option<HashSet<String>> = if child {
            let mut own: HashSet<String> = [root.clone(), ctx.chat_id.clone()].into();
            let loaded: Vec<_> = ctx
                .app
                .chat
                .sessions
                .read()
                .await
                .values()
                .cloned()
                .collect();
            for session in loaded {
                if let Ok(s) = session.try_lock() {
                    if s.thread.root_chat_id.as_deref() == Some(root.as_str()) {
                        own.insert(s.chat_id.clone());
                    }
                }
            }
            Some(own)
        } else {
            None
        };
        let entries: Vec<_> = shell_gate::read_audit(ctx.app.gcx.clone(), 500)
            .await
            .into_iter()
            .filter(|e| chat_filter.is_none_or(|c| c == e.chat_id))
            .filter(|e| own_tree.as_ref().is_none_or(|own| own.contains(&e.chat_id)))
            .collect();
        let total = entries.len();
        let rows: Vec<Value> = entries
            .into_iter()
            .take(limit)
            .map(|e| {
                let mut row = json!({"ts_ms":e.ts_ms,"chat_id":e.chat_id,"command":scrub_text(&e.command, &[]),"decision":e.decision,"layer":e.layer,"rule":scrub_text(&e.rule, &[])});
                if let Some(level) = e.risk_level {
                    row["risk_level"] = json!(level);
                }
                row
            })
            .collect();
        let text = format!("{} of {total} recent shell decisions", rows.len());
        Ok(EngineOutput::new(
            json!({"entries":rows,"total":total,"truncated":total > limit}),
            text,
        ))
    })
}

// ---------------------------------------------------------------------------
// Changes.

use super::{Plan, Preview};

pub(super) fn change_functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "tools.set_enabled",
            summary: "Enable or disable a builtin or integration tool engine-wide from each chat's next turn. Enabling is risky; python and MCP tools are refused.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["tool","enabled"],"properties":{"tool":{"type":"string","minLength":1,"description":"Name from tools.list."},"enabled":{"type":"boolean"}}}),
            output: || Some(json!({"type":"object","required":["tool","old","new"],"properties":{"tool":{"type":"string"},"config_path":{"type":"string"},"old":{"type":"boolean"},"new":{"type":"boolean"}}})),
            handler: set_enabled,
            preview: Some(set_enabled_preview),
            timeout: None,
        },
        EngineFn {
            name: "tools.refresh",
            summary: "Rebuild another idle loaded chat's tool list from its current mode and settings (the next request is uncached); its system prompt is kept.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id"],"properties":{"chat_id":{"type":"string","minLength":1}}}),
            output: || Some(json!({"type":"object","required":["chat_id","refreshed","effective_next_turn"],"properties":{"chat_id":{"type":"string"},"refreshed":{"type":"boolean"},"effective_next_turn":{"type":"boolean"}}})),
            handler: refresh,
            preview: Some(refresh_preview),
            timeout: None,
        },
        EngineFn {
            name: "tools.set_shell_policy",
            summary: "Change this project's shell policy: mode, deny and ask rules, removal of allow rules, caller confirmation, LLM validation and risk catalogue entries. Risky when it weakens a check; mode yolo and adding allow rules are refused (you add allow rules in the shell policy settings).",
            class: EngineClass::Change,
            input: || {
                let patterns = json!({"type":"array","minItems":1,"items":{"type":"string","maxLength":2000}});
                json!({"type":"object","additionalProperties":false,"minProperties":1,"properties":{
                    "mode":{"enum":["strict","balanced","permissive","yolo"]},
                    "add_deny":patterns,"remove_deny":patterns,"add_ask":patterns,"remove_ask":patterns,"remove_allow":patterns,
                    "add_allow":{"type":"array","description":"Always refused: scripts cannot add allow rules; you add them in the shell policy settings."},
                    "trust_caller_confirmation":{"type":"boolean"},
                    "llm_validation":{"type":"object","additionalProperties":false,"minProperties":1,"properties":{
                        "enabled":{"type":"boolean"},"model":{"type":"string"},"authority":{"enum":["ask_only","ask_and_allow"]},
                        "timeout_secs":{"type":"integer","minimum":1,"maximum":120},"on_failure":{"enum":["pass","ask"]},"cache_per_chat":{"type":"boolean"}
                    }},
                    "catalogue":{"type":"array","minItems":1,"items":{"type":"object","additionalProperties":false,"required":["id"],"properties":{
                        "id":{"type":"string","minLength":1},"level":{"enum":["low","medium","high","critical"]},"enabled":{"type":"boolean"}
                    }}}
                }})
            },
            output: || Some(json!({"type":"object","required":["path","changed"],"properties":{"path":{"type":"string"},"changed":{"type":"object"}}})),
            handler: set_shell_policy,
            preview: Some(set_shell_policy_preview),
            timeout: None,
        },
    ]
}

/// The tool's config path (from the registry, never from the script) and its current config.
async fn tool_config(
    ctx: &EngineCtx,
    name: &str,
) -> Result<(String, crate::tools::tools_description::ToolConfig), EngineError> {
    if name == "python" {
        return Err(EngineError::Permission(
            "the python tool cannot be switched from a script".into(),
        ));
    }
    let (groups, _) = crate::tools::tools_list::tool_groups_with_gating(ctx.app.gcx.clone()).await;
    for group in groups {
        if matches!(group.category, refact_tool_api::ToolGroupCategory::MCP) {
            continue;
        }
        for tool in group.tools {
            let desc = tool.tool_description();
            if desc.name == name {
                if desc.source.config_path.is_empty() {
                    return Err(EngineError::Type(format!(
                        "{name} has no config file to switch it in"
                    )));
                }
                let config = tool.config().map_err(|e| {
                    EngineError::Failed(format!(
                        "the config of {name} cannot be read ({e}); its current state is unknown"
                    ))
                })?;
                return Ok((desc.source.config_path.clone(), config));
            }
        }
    }
    Err(EngineError::Type(format!("{name} is not a registered builtin or integration tool; see tools.list (MCP tools: integrations)")))
}

fn set_enabled_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let tool = args["tool"].as_str().unwrap_or_default();
        let enabled = args["enabled"].as_bool().unwrap_or(false);
        let (_, config) = tool_config(ctx, tool).await?;
        let enabling = enabled && !config.enabled;
        Ok(Preview {
            title: if enabled {
                "enable a tool".into()
            } else {
                "disable a tool".into()
            },
            items: vec![super::mask::named_item(
                tool,
                Some(json!(config.enabled)),
                Some(json!(enabled)),
            )],
            scope: Some("all chats, from their next turn".into()),
            class: if enabling {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning: enabling.then(|| "Chats will be able to use this tool again.".to_string()),
            plan: if config.enabled == enabled {
                Plan::nothing()
            } else {
                super::previewed::plan(json!(config.enabled))
            },
        })
    })
}

fn set_enabled<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let tool = args["tool"].as_str().unwrap_or_default();
        let enabled = args["enabled"].as_bool().unwrap_or(false);
        let (path, config) = tool_config(ctx, tool).await?;
        super::previewed::take(ctx, "tools.set_enabled", &json!(config.enabled))?;
        crate::tools::tools_description::set_tool_config(
            path.clone(),
            tool.to_string(),
            crate::tools::tools_description::ToolConfig {
                enabled,
                allow_parallel: config.allow_parallel,
            },
        )
        .await
        .map_err(EngineError::Failed)?;
        ctx.app.gcx.tool_catalog_generations.advance_integrations();
        Ok(EngineOutput::new(
            json!({"tool":tool,"config_path":path,"old":config.enabled,"new":enabled}),
            format!(
                "{tool} {} from each chat's next turn",
                if enabled { "enabled" } else { "disabled" }
            ),
        ))
    })
}

async fn refresh_target(
    ctx: &EngineCtx,
    chat_id: &str,
) -> Result<
    (
        std::sync::Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
        Value,
    ),
    EngineError,
> {
    if chat_id == ctx.chat_id {
        return Err(EngineError::Type(
            "this chat is busy running the script; refresh another chat".into(),
        ));
    }
    crate::chat::trajectories::validate_trajectory_id(chat_id)
        .map_err(|e| EngineError::Type(e.message))?;
    let session = ctx
        .app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or_else(|| EngineError::Type("Open this chat before refreshing tools".into()))?;
    let state = {
        let s = session.lock().await;
        if !s.turn_finished() {
            return Err(EngineError::Permission(format!(
                "chat {chat_id} is busy; refresh it when it is idle"
            )));
        }
        json!({"version":s.trajectory_version,"frozen":s.thread.frozen_request_prefix.as_ref().map(|p| p.created_at.clone())})
    };
    Ok((session, state))
}

fn refresh_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let chat_id = args["chat_id"].as_str().unwrap_or_default();
        let (_, state) = refresh_target(ctx, chat_id).await?;
        let pinned = state["frozen"]
            .as_str()
            .map(|at| format!("pinned at {at}"))
            .unwrap_or_else(|| "not pinned".into());
        Ok(Preview {
            title: "refresh another chat's tools".into(),
            items: vec![
                super::mask::item(
                    "tool schemas sent to the model",
                    Some(json!(pinned)),
                    Some(json!("rebuilt")),
                ),
                super::mask::item(
                    "prompt cache",
                    None,
                    Some(json!("reset (next request is uncached)")),
                ),
            ],
            scope: Some(format!("chat {chat_id}")),
            class: EngineClass::Change,
            warning: None,
            plan: super::previewed::plan(state),
        })
    })
}

fn refresh<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let chat_id = args["chat_id"].as_str().unwrap_or_default();
        let (session, state) = refresh_target(ctx, chat_id).await?;
        super::previewed::take(ctx, "tools.refresh", &state)?;
        let version = state["version"].as_u64();
        crate::http::routers::v1::trajectory_ops::refresh_session_tools(
            ctx.app.gcx.clone(),
            session,
            version,
        )
        .await
        .map_err(|e| EngineError::Failed(e.message))?;
        Ok(EngineOutput::new(
            json!({"chat_id":chat_id,"refreshed":true,"effective_next_turn":true}),
            format!("tools of {chat_id} rebuilt; effective from its next turn"),
        ))
    })
}

/// The project's shell policy file: its path, the digest of its bytes and the parsed policy.
/// A file that does not parse is refused, so a change never replaces it with defaults.
async fn policy_file(
    ctx: &EngineCtx,
) -> Result<(std::path::PathBuf, Value, shell_gate::ShellGatePolicy), EngineError> {
    let root = crate::files_correction::get_project_dirs(ctx.app.gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| EngineError::Type("no active project".into()))?;
    let path = shell_gate::policy_path(&root);
    super::common::inside_project(&root, &path, &ctx.app.gcx.config_dir)?;
    let (version, policy) = match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mut policy: shell_gate::ShellGatePolicy =
                serde_yaml::from_slice(&bytes).map_err(|e| {
                    EngineError::Failed(format!(
                        "{} does not parse ({e}); fix it before changing the policy",
                        path.display()
                    ))
                })?;
            policy.rebuild_catalogue();
            (
                Some(crate::yaml_configs::customization_service::version(&bytes)),
                policy,
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (None, shell_gate::ShellGatePolicy::default())
        }
        Err(e) => return Err(super::common::failed(e)),
    };
    Ok((path, json!({ "file": version }), policy))
}

fn mode_rank(mode: &shell_gate::ApprovalMode) -> u8 {
    match mode {
        shell_gate::ApprovalMode::Strict => 0,
        shell_gate::ApprovalMode::Balanced => 1,
        shell_gate::ApprovalMode::Permissive => 2,
        shell_gate::ApprovalMode::Yolo => 3,
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(String::from))
        .collect()
}

type PolicyChange = (
    shell_gate::ShellGatePolicy,
    Vec<refact_chat_api::PythonEngineItem>,
    bool,
);

/// The new policy, the card items and whether it weakens a check.
fn shell_policy_change(
    old: &shell_gate::ShellGatePolicy,
    args: &Value,
) -> Result<PolicyChange, EngineError> {
    use crate::http::routers::v1::shell_policy::{parse_authority, parse_mode, parse_on_failure};
    let mut new = old.clone();
    let mut items = Vec::new();
    let mut weakens = false;
    if let Some(mode) = args.get("mode").and_then(Value::as_str) {
        let mode = parse_mode(mode).map_err(EngineError::Type)?;
        if mode == shell_gate::ApprovalMode::Yolo {
            return Err(EngineError::Permission(
                "mode yolo runs every command without asking; only you can set it, in the shell policy settings".into(),
            ));
        }
        if mode != old.mode {
            weakens |= mode_rank(&mode) > mode_rank(&old.mode);
            items.push(super::mask::item(
                "mode",
                Some(to_value(&old.mode)?),
                Some(to_value(&mode)?),
            ));
            new.mode = mode;
        }
    }
    if args.get("add_allow").is_some() {
        return Err(EngineError::Permission(
            "scripts cannot add shell allow rules: an allow rule passes a whole compound command when one part of it matches; you add allow rules in the shell policy settings".into(),
        ));
    }
    for list in ["deny", "ask", "allow"] {
        let rules = match list {
            "deny" => &mut new.deny,
            "ask" => &mut new.ask,
            _ => &mut new.allow,
        };
        for pattern in strings(&args[format!("remove_{list}")]) {
            if !rules.contains(&pattern) {
                return Err(EngineError::Type(format!(
                    "{pattern:?} is not a {list} rule"
                )));
            }
            rules.retain(|r| *r != pattern);
            weakens |= list != "allow";
            items.push(super::mask::item(list, Some(json!(pattern)), None));
        }
        for pattern in strings(&args[format!("add_{list}")]) {
            if !rules.contains(&pattern) {
                items.push(super::mask::item(list, None, Some(json!(pattern))));
                rules.push(pattern);
            }
        }
    }
    if let Some(trust) = args
        .get("trust_caller_confirmation")
        .and_then(Value::as_bool)
    {
        if trust != old.trust_caller_confirmation {
            weakens |= !trust;
            items.push(super::mask::item(
                "trust_caller_confirmation",
                Some(json!(old.trust_caller_confirmation)),
                Some(json!(trust)),
            ));
            new.trust_caller_confirmation = trust;
        }
    }
    let llm = &args["llm_validation"];
    if llm.is_object() {
        let v = &mut new.llm_validation;
        if let Some(enabled) = llm.get("enabled").and_then(Value::as_bool) {
            weakens |= v.enabled && !enabled;
            v.enabled = enabled;
        }
        if let Some(model) = llm.get("model").and_then(Value::as_str) {
            v.model = model.to_string();
        }
        if let Some(authority) = llm.get("authority").and_then(Value::as_str) {
            let authority = parse_authority(authority).map_err(EngineError::Type)?;
            weakens |=
                authority == shell_gate::ShellLlmAuthority::AskAndAllow && v.authority != authority;
            v.authority = authority;
        }
        if let Some(timeout) = llm.get("timeout_secs").and_then(Value::as_u64) {
            v.timeout_secs = timeout;
        }
        if let Some(on_failure) = llm.get("on_failure").and_then(Value::as_str) {
            let on_failure = parse_on_failure(on_failure).map_err(EngineError::Type)?;
            weakens |= v.on_failure == shell_gate::ShellLlmOnFailure::Ask
                && on_failure == shell_gate::ShellLlmOnFailure::Pass;
            v.on_failure = on_failure;
        }
        if let Some(cache) = llm.get("cache_per_chat").and_then(Value::as_bool) {
            v.cache_per_chat = cache;
        }
        let approves = |v: &shell_gate::ShellLlmValidation| {
            v.enabled && v.authority == shell_gate::ShellLlmAuthority::AskAndAllow
        };
        weakens |=
            approves(v) && (!approves(&old.llm_validation) || v.model != old.llm_validation.model);
        weakens |= v.enabled
            && old.llm_validation.enabled
            && v.on_failure == shell_gate::ShellLlmOnFailure::Pass
            && v.timeout_secs < old.llm_validation.timeout_secs;
        let (o, n) = (
            to_value(&old.llm_validation)?,
            to_value(&new.llm_validation)?,
        );
        for (key, value) in n.as_object().into_iter().flatten() {
            if o[key] != *value {
                items.push(super::mask::item(
                    &format!("llm_validation.{key}"),
                    Some(o[key].clone()),
                    Some(value.clone()),
                ));
            }
        }
    }
    let defaults = refact_tool_api::default_catalogue();
    for entry in args["catalogue"].as_array().into_iter().flatten() {
        let id = entry["id"].as_str().unwrap_or_default();
        let default = defaults
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| EngineError::Type(format!("unknown risk catalogue id {id:?}")))?;
        let current = old.catalogue_overrides.iter().find(|o| o.id == id);
        let old_level = current.and_then(|o| o.level).unwrap_or(default.level);
        let old_enabled = current.and_then(|o| o.enabled).unwrap_or(true);
        let level: refact_tool_api::RiskLevel = match entry.get("level") {
            Some(level) => serde_json::from_value(level.clone())
                .map_err(|e| EngineError::Type(format!("catalogue {id}: {e}")))?,
            None => old_level,
        };
        let enabled = entry
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(old_enabled);
        if level != old_level {
            weakens |= level < old_level;
            items.push(super::mask::item(
                &format!("catalogue.{id}.level"),
                Some(to_value(old_level)?),
                Some(to_value(level)?),
            ));
        }
        if enabled != old_enabled {
            weakens |= !enabled;
            items.push(super::mask::item(
                &format!("catalogue.{id}.enabled"),
                Some(json!(old_enabled)),
                Some(json!(enabled)),
            ));
        }
        new.catalogue_overrides.retain(|o| o.id != id);
        if level != default.level || !enabled {
            new.catalogue_overrides.push(shell_gate::RiskEntryOverride {
                id: id.to_string(),
                level: (level != default.level).then_some(level),
                enabled: (!enabled).then_some(false),
            });
        }
    }
    new.rebuild_catalogue();
    Ok((new, items, weakens))
}

fn set_shell_policy_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let (_, state, old) = policy_file(ctx).await?;
        let (new, items, weakens) = shell_policy_change(&old, args)?;
        Ok(Preview {
            title: "change the shell policy".into(),
            plan: if items.is_empty() {
                Plan::nothing()
            } else {
                Plan::new(state, new)
            },
            items,
            scope: Some("this project, all chats".into()),
            class: if weakens {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning: weakens.then(|| {
                "Shell commands that needed your approval may then run without asking.".to_string()
            }),
        })
    })
}

fn set_shell_policy<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let (path, state, old) = policy_file(ctx).await?;
        let new: shell_gate::ShellGatePolicy = ctx.take_plan("tools.set_shell_policy", &state)?;
        shell_gate::save_policy(ctx.app.gcx.clone(), &new)
            .await
            .map_err(EngineError::Failed)?;
        let (o, n) = (to_value(&old)?, to_value(&new)?);
        let changed: serde_json::Map<String, Value> = n
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, v)| o[k.as_str()] != **v)
            .map(|(k, v)| (k.clone(), json!({"old":o[k.as_str()],"new":v})))
            .collect();
        let text = format!(
            "shell policy changed ({}); applies to the next shell call",
            changed.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        Ok(EngineOutput::new(
            json!({"path":path,"changed":changed}),
            text,
        ))
    })
}
