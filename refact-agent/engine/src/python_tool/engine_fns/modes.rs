use std::collections::HashSet;
use std::path::PathBuf;

use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::common::{failed, to_value};
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput};
use crate::integrations::mcp::redaction::scrub_text;

const KINDS: &[&str] = &["modes", "subagents", "toolbox_commands", "code_lens"];

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "modes.list",
            summary: "Modes with title, tool count, approval defaults, confirmation rules and where each is defined; broken mode files are listed in errors.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"include_specific":{"type":"boolean","description":"Include internal modes (task_agent, buddy, setup_*); false by default."}}}),
            output: || Some(json!({"type":"object","required":["modes","current","errors"],"properties":{"modes":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"title":{"type":"string"},"description":{"type":"string"},"specific":{"type":"boolean"},"scope":{"enum":["builtin","global","local"]},"tools_count":{"type":"integer"},"allow_integrations":{"type":"boolean"},"allow_mcp":{"type":"boolean"},"allow_subagents":{"type":"boolean"},"tool_confirm":{"type":"array"},"thread_defaults":{"type":"object"},"order":{"type":"integer"}}}},"current":{"type":"string"},"errors":{"type":"array","items":{"type":"object","properties":{"file_path":{"type":"string"},"error":{"type":"string"}}}}}})),
            handler: list,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "modes.get",
            summary: "One mode's (or subagent's, toolbox command's, code lens's) configuration with prompt and tools, resolved for a model, or the raw file of one scope with the version modes.save expects.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["mode"],"properties":{
                "mode":{"type":"string","minLength":1,"description":"Id of the mode or other configuration."},
                "model":{"type":"string","minLength":1,"description":"Resolve for this model; this chat's model by default."},
                "kind":{"enum":KINDS,"description":"modes by default."},
                "scope":{"enum":["global","local"],"description":"Return the raw file of this scope instead of the resolved config."}
            }}),
            output: || Some(json!({"type":"object","required":["config"],"properties":{"config":{"type":"object"},"resolved_for_model":{"type":"string"},"overlay_id":{"type":"string"},"dropped_tools":{"type":"array"},"unknown_tools":{"type":"array"},"gated_tools":{"type":"array"},"file_path":{"type":"string"},"scope":{"type":"string"},"version":{"type":"string"}}})),
            handler: get,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "modes.skills",
            summary: "Installed skills and slash commands, this chat's active skill and the skills auto-trigger setting.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["skills","slash_commands","skills_available","auto_trigger"],"properties":{"skills":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"description":{"type":"string"},"source":{"type":"string"},"scope":{"type":"string"},"read_only":{"type":"boolean"}}}},"slash_commands":{"type":"array"},"active_skill":{"type":"string"},"skills_available":{"type":"integer"},"auto_trigger":{"enum":["index_only","inject_full","off"]}}})),
            handler: skills,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "modes.project_info",
            summary: "Which project information blocks (instruction files, configs, memories, tree, git) new chats get and their sizes; content on request, with privacy records of the files.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"content":{"type":"boolean","description":"Include block text; false by default (sizes and titles only)."}}}),
            output: || Some(json!({"type":"object","required":["enabled","blocks","warnings"],"properties":{"enabled":{"type":"boolean"},"blocks":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"section":{"type":"string"},"title":{"type":"string"},"path":{"type":"string"},"enabled":{"type":"boolean"},"truncated":{"type":"boolean"},"char_count":{"type":"integer"},"content":{"type":"string"},"withheld":{"type":"boolean"}}}},"warnings":{"type":"array"}}})),
            handler: project_info,
            preview: None,
            timeout: None,
        },
    ]
    .into_iter()
    .chain(change_functions())
    .collect()
}

async fn project_root(ctx: &EngineCtx) -> Option<PathBuf> {
    crate::files_correction::get_project_dirs(ctx.app.gcx.clone())
        .await
        .into_iter()
        .next()
}

fn list<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let include_specific = args.get("include_specific").and_then(Value::as_bool) == Some(true);
        let config_dir = ctx.app.gcx.config_dir.clone();
        let root = project_root(ctx).await;
        let registry = crate::yaml_configs::customization_registry::load_merged_registry(
            &config_dir,
            root.as_deref(),
        )
        .await;
        let mut modes: Vec<Value> = registry
            .modes
            .values()
            .filter(|m| include_specific || !m.specific)
            .map(|m| {
                let file = format!("{}.yaml", m.id);
                let scope = if root.as_ref().is_some_and(|r| r.join(".refact/modes").join(&file).exists()) {
                    "local"
                } else if config_dir.join("modes").join(&file).exists() {
                    "global"
                } else {
                    "builtin"
                };
                let d = &m.thread_defaults;
                json!({
                    "id":m.id,"title":if m.title.is_empty() { &m.id } else { &m.title },"description":m.description,
                    "specific":m.specific,"scope":scope,"tools_count":m.tools.len(),
                    "allow_integrations":m.allow_integrations,"allow_mcp":m.allow_mcp,"allow_subagents":m.allow_subagents,
                    "tool_confirm":m.tool_confirm.rules.iter().map(|r| json!({"match":r.match_pattern,"action":r.action})).collect::<Vec<_>>(),
                    "thread_defaults":{"include_project_info":d.include_project_info.unwrap_or(true),"checkpoints_enabled":d.checkpoints_enabled.unwrap_or(true),"auto_approve_editing_tools":d.auto_approve_editing_tools.unwrap_or(false),"auto_approve_dangerous_commands":d.auto_approve_dangerous_commands.unwrap_or(false)},
                    "order":m.ui.order.unwrap_or(100),
                })
            })
            .collect();
        modes.sort_by(|a, b| {
            a["order"]
                .as_i64()
                .cmp(&b["order"].as_i64())
                .then(a["id"].as_str().cmp(&b["id"].as_str()))
        });
        let current = super::chat::this_session(ctx)
            .await?
            .lock()
            .await
            .thread
            .mode
            .clone();
        let errors: Vec<Value> = registry
            .errors
            .iter()
            .map(|e| json!({"file_path":e.file_path,"error":scrub_text(&e.error, &[])}))
            .collect();
        let text = format!(
            "{} modes; this chat uses {current}{}",
            modes.len(),
            if errors.is_empty() {
                String::new()
            } else {
                format!("; {} broken files", errors.len())
            }
        );
        Ok(EngineOutput::new(
            json!({"modes":modes,"current":current,"errors":errors}),
            text,
        ))
    })
}

fn get<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = args["mode"].as_str().unwrap_or_default();
        let kind = args.get("kind").and_then(Value::as_str).unwrap_or("modes");
        crate::yaml_configs::customization_service::validate_id(id).map_err(EngineError::Type)?;
        if let Some(scope) = args.get("scope").and_then(Value::as_str) {
            let path = match scope {
                "global" => ctx.app.gcx.config_dir.join(kind).join(format!("{id}.yaml")),
                _ => project_root(ctx)
                    .await
                    .ok_or_else(|| EngineError::Type("no project root for local scope".into()))?
                    .join(".refact")
                    .join(kind)
                    .join(format!("{id}.yaml")),
            };
            let bytes = match tokio::fs::read(&path).await {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(EngineError::Type(format!(
                        "{kind}/{id} has no {scope} file ({})",
                        path.display()
                    )))
                }
                Err(e) => return Err(failed(e)),
            };
            let config: Value = serde_yaml::from_slice(&bytes)
                .map_err(|e| EngineError::Failed(format!("yaml parse error: {e}")))?;
            let version = crate::yaml_configs::customization_service::version(&bytes);
            return Ok(EngineOutput::new(
                json!({"config":config,"file_path":path,"scope":scope,"version":version}),
                format!("{kind}/{id} from {}", path.display()),
            ));
        }
        if kind != "modes" {
            let registry = crate::yaml_configs::customization_registry::get_project_registry(
                ctx.app.gcx.clone(),
            )
            .await
            .ok_or_else(|| {
                EngineError::Failed("the customization registry is unavailable".into())
            })?;
            let config = match kind {
                "subagents" => registry.subagents.get(id).map(to_value),
                "toolbox_commands" => registry.toolbox_commands.get(id).map(to_value),
                _ => registry.code_lens.get(id).map(to_value),
            }
            .transpose()?
            .ok_or_else(|| EngineError::Type(format!("unknown {kind} id {id}")))?;
            return Ok(EngineOutput::new(
                json!({"config":config}),
                format!("{kind}/{id}"),
            ));
        }
        let model = match args.get("model").and_then(Value::as_str) {
            Some(model) => model.to_string(),
            None => ctx.ccx.lock().await.current_model.clone(),
        };
        let gcx = ctx.app.gcx.clone();
        let mode = crate::yaml_configs::customization_registry::get_mode_config(
            gcx.clone(),
            id,
            Some(&model),
        )
        .await
        .ok_or_else(|| EngineError::Type(format!("unknown mode {id}")))?;
        let registry =
            crate::yaml_configs::customization_registry::get_project_registry(gcx.clone()).await;
        let drift = registry.as_ref().and_then(|r| {
            crate::yaml_configs::customization_registry::mode_override_tool_drift(
                r,
                id,
                Some(&model),
            )
        });
        let (groups, gated) = crate::tools::tools_list::tool_groups_with_gating(gcx).await;
        let registered: HashSet<String> = groups
            .iter()
            .flat_map(|g| g.tools.iter().map(|t| t.tool_description().name))
            .collect();
        let unknown = crate::yaml_configs::mode_validation::unknown_tool_names(
            &mode.tools,
            &registered,
            &gated,
        );
        let gated_tools: Vec<Value> = crate::yaml_configs::mode_validation::gated_tool_names(&mode.tools, &gated)
            .into_iter()
            .map(|((capability, reason), tools)| json!({"capability":capability,"reason":reason,"tools":tools}))
            .collect();
        let mut data = json!({"config":to_value(&mode)?,"resolved_for_model":model,"dropped_tools":drift.as_ref().map(|d| d.1.clone()).unwrap_or_default(),"unknown_tools":unknown,"gated_tools":gated_tools});
        if let Some((overlay, _)) = drift {
            data["overlay_id"] = json!(overlay);
        }
        let text = format!("mode {id} for {model}: {} tools", mode.tools.len());
        Ok(EngineOutput::new(data, text))
    })
}

fn skills<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let registry =
            crate::http::routers::v1::ext_management::ext_registry(ctx.app.clone()).await;
        let row = |name: &str, description: &str, source: &str, scope: &str, read_only: bool| json!({"name":name,"description":description,"source":source,"scope":scope,"read_only":read_only});
        let skills: Vec<Value> = registry
            .skills
            .iter()
            .map(|s| row(&s.name, &s.description, &s.source, &s.scope, s.read_only))
            .collect();
        let commands: Vec<Value> = registry
            .slash_commands
            .iter()
            .map(|c| row(&c.name, &c.description, &c.source, &c.scope, c.read_only))
            .collect();
        let (available, active) = {
            let session = super::chat::this_session(ctx).await?;
            let s = session.lock().await;
            (s.skills_available_count, s.thread.active_skill.clone())
        };
        let trigger = crate::http::routers::v1::settings_config::skills_auto_trigger(&ctx.app)
            .await
            .map_err(EngineError::Failed)?;
        let mut data = json!({"skills":skills,"slash_commands":commands,"skills_available":available,"auto_trigger":to_value(trigger)?});
        if let Some(active) = &active {
            data["active_skill"] = json!(active);
        }
        let text = format!(
            "{} skills, {} slash commands, auto trigger {}{}",
            skills.len(),
            commands.len(),
            data["auto_trigger"].as_str().unwrap_or(""),
            active.map(|a| format!(", active {a}")).unwrap_or_default()
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn project_info<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let with_content = args.get("content").and_then(Value::as_bool) == Some(true);
        let gcx = ctx.app.gcx.clone();
        let config =
            crate::yaml_configs::project_information::load_project_information_config(gcx.clone())
                .await;
        let preview = crate::http::routers::v1::project_information::project_information_preview(
            &ctx.app, &config,
        )
        .await;
        let roots = crate::files_correction::get_project_dirs(gcx.clone()).await;
        let absolute = |path: &str| -> PathBuf {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                path
            } else {
                roots.first().map(|r| r.join(&path)).unwrap_or(path)
            }
        };
        let mut output = EngineOutput::new(Value::Null, "");
        let mut zones = std::collections::HashMap::new();
        if with_content {
            let paths: Vec<PathBuf> = preview
                .blocks
                .iter()
                .filter_map(|b| b.path.as_deref())
                .map(absolute)
                .filter(|p| p.is_file())
                .collect();
            if !paths.is_empty() {
                crate::privacy::load_privacy_if_needed(gcx.clone()).await;
                let records = crate::privacy::records::declared_file_records(&gcx, paths.clone()).map_err(|e| {
                    EngineError::Failed(format!("privacy records of the project information files could not be made ({e}); nothing was returned"))
                })?;
                for (path, record) in paths.iter().zip(&records) {
                    zones.insert(path.clone(), record.zone.clone());
                }
                output.privacy.push(json!({"files":to_value(&records)?}));
            }
        }
        let mut chars = 0;
        let blocks: Vec<Value> = preview
            .blocks
            .iter()
            .map(|b| {
                chars += b.char_count;
                let mut row = json!({"id":b.id,"section":b.section,"title":b.title,"enabled":b.enabled,"truncated":b.truncated,"char_count":b.char_count});
                if let Some(path) = &b.path {
                    row["path"] = json!(path);
                }
                if with_content {
                    let blocked = b.path.as_deref().map(absolute).and_then(|p| zones.get(&p)).is_some_and(|z| z == "blocked");
                    if blocked {
                        row["withheld"] = json!(true);
                    } else {
                        row["content"] = json!(scrub_text(&b.content, &[]));
                    }
                }
                row
            })
            .collect();
        output.text = format!("{} blocks, {chars} chars", blocks.len());
        output.data = json!({"enabled":config.enabled,"blocks":blocks,"warnings":preview.warnings});
        Ok(output)
    })
}

// ---------------------------------------------------------------------------
// Changes.

use super::{Plan, Preview};

pub(super) fn change_functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "modes.save",
            summary: "Create or update a mode, subagent, toolbox command or code lens definition; updates are merged into the file and validated. Risky when it widens tools or approvals. Text that other models read passes the privacy egress check first.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["kind","id","config"],"properties":{
                "kind":{"enum":KINDS},
                "id":{"type":"string","minLength":1},
                "config":{"type":"object","description":"Partial object merged into an existing file, or a full config with id and schema_version to create one."},
                "scope":{"enum":["global","local"],"description":"Where it exists, else local."},
                "expected_version":{"type":"string","minLength":1,"description":"version from modes.get with scope."}
            }}),
            output: || Some(json!({"type":"object","required":["kind","id","file_path","scope","created"],"properties":{"kind":{"type":"string"},"id":{"type":"string"},"file_path":{"type":"string"},"scope":{"type":"string"},"created":{"type":"boolean"},"version":{"type":"string"}}})),
            handler: save,
            preview: Some(save_preview),
            timeout: None,
        },
        EngineFn {
            name: "modes.set_project_info",
            summary: "Turn project information blocks for new chats on or off (whole feature or per section).",
            class: EngineClass::Change,
            input: || {
                let sections: serde_json::Map<String, Value> = SECTIONS
                    .iter()
                    .map(|s| (s.to_string(), json!({"type":"object","additionalProperties":false,"required":["enabled"],"properties":{"enabled":{"type":"boolean"}}})))
                    .collect();
                json!({"type":"object","additionalProperties":false,"properties":{"enabled":{"type":"boolean"},"sections":{"type":"object","additionalProperties":false,"minProperties":1,"properties":sections}}})
            },
            output: || Some(json!({"type":"object","required":["changed"],"properties":{"changed":{"type":"object"}}})),
            handler: set_project_info,
            preview: Some(set_project_info_preview),
            timeout: None,
        },
        EngineFn {
            name: "modes.set_skills_trigger",
            summary: "Choose how skills are offered to chats in this project: index only, full injection or off.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["auto_trigger"],"properties":{"auto_trigger":{"enum":["index_only","inject_full","off"]}}}),
            output: || Some(json!({"type":"object","required":["auto_trigger"],"properties":{"auto_trigger":{"type":"string"}}})),
            handler: set_skills_trigger,
            preview: Some(set_skills_trigger_preview),
            timeout: None,
        },
        EngineFn {
            name: "modes.delete",
            summary: "Delete a mode, subagent, toolbox command or code lens file of one scope. A change when a builtin or the other scope's definition takes over; destructive when nothing remains. The card shows the deleted content.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["kind","mode","scope"],"properties":{
                "kind":{"enum":KINDS},
                "mode":{"type":"string","minLength":1,"description":"Id of the mode or other configuration."},
                "scope":{"enum":["global","local"]}
            }}),
            output: || Some(json!({"type":"object","required":["kind","id","scope","deleted","falls_back_to"],"properties":{"kind":{"type":"string"},"id":{"type":"string"},"scope":{"type":"string"},"deleted":{"type":"boolean"},"falls_back_to":{"type":"string"},"errors":{"type":"array"}}})),
            handler: delete,
            preview: Some(delete_preview),
            timeout: None,
        },
    ]
}

const SECTIONS: &[&str] = &[
    "system_info",
    "models_info",
    "environment_instructions",
    "detected_environments",
    "git_info",
    "project_tree",
    "instruction_files",
    "project_configs",
    "memories",
];

#[derive(Clone)]
struct SavePlan {
    path: PathBuf,
    scope: String,
    expected: Option<String>,
    bytes: Vec<u8>,
    created: bool,
}

/// Rank of a confirmation action: lower runs with fewer questions.
fn action_rank(action: Option<&str>) -> u8 {
    match action {
        Some("auto") => 0,
        None => 1,
        Some("ask") => 2,
        _ => 3,
    }
}

fn confirm_rules(config: &Value) -> Vec<crate::yaml_configs::customization_types::ToolConfirmRule> {
    serde_json::from_value(config["tool_confirm"]["rules"].clone()).unwrap_or_default()
}

/// Whether some tool would be asked less under the new `tool_confirm` rules (first match wins).
/// Tool names are probed from both configs' tools and every rule pattern; when the rule order
/// changed among wildcard rules the result cannot be decided by probing, so it counts as widening.
fn confirm_widens(old: &Value, new: &Value) -> bool {
    use crate::yaml_configs::customization_registry::match_tool_confirm_action;
    let (old_rules, new_rules) = (confirm_rules(old), confirm_rules(new));
    let mut probes: Vec<String> = [old, new]
        .iter()
        .flat_map(|c| c["tools"].as_array().cloned().unwrap_or_default())
        .filter_map(|t| t.as_str().map(String::from))
        .collect();
    for rule in old_rules.iter().chain(&new_rules) {
        let p = &rule.match_pattern;
        probes.push(p.replace('*', ""));
        probes.push(p.replace('*', "zz"));
    }
    let key = |r: &crate::yaml_configs::customization_types::ToolConfirmRule| {
        (r.match_pattern.clone(), r.action.clone())
    };
    let new_keys: Vec<_> = new_rules.iter().map(key).collect();
    let common_old: Vec<_> = old_rules
        .iter()
        .map(key)
        .filter(|r| new_keys.contains(r))
        .collect();
    let common_new: Vec<_> = new_keys
        .iter()
        .filter(|r| common_old.contains(r))
        .cloned()
        .collect();
    let reordered = common_old != common_new;
    reordered
        || probes.iter().any(|name| {
            action_rank(match_tool_confirm_action(&new_rules, name).as_deref())
                < action_rank(match_tool_confirm_action(&old_rules, name).as_deref())
        })
}

/// Whether the new config lets the mode or subagent do more than the old one: more tools,
/// fewer confirmations, approval defaults switched on, integrations, MCP or subagents allowed.
pub(super) fn widens(old: &Value, new: &Value) -> bool {
    if crate::yaml_configs::customization_service::reject_authority_widening(old, new).is_err() {
        return true;
    }
    let flag = |v: &Value, k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    let tools = |v: &Value| -> Vec<String> {
        v["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(String::from))
            .collect()
    };
    let old_tools = tools(old);
    if tools(new).iter().any(|t| !old_tools.contains(t)) {
        return true;
    }
    for key in ["allow_integrations", "allow_mcp", "allow_subagents"] {
        if !flag(old, key) && flag(new, key) {
            return true;
        }
    }
    for section in ["thread_defaults", "subchat"] {
        for key in [
            "auto_approve_editing_tools",
            "auto_approve_dangerous_commands",
            "autonomous_no_confirm",
        ] {
            if !flag(&old[section], key) && flag(&new[section], key) {
                return true;
            }
        }
    }
    if confirm_widens(old, new) {
        return true;
    }
    let ovr = |v: &Value| v.get("override").cloned().unwrap_or(Value::Null);
    let (old_ovr, new_ovr) = (ovr(old), ovr(new));
    let old_added = tools(&json!({"tools": old_ovr["tools_add"]}));
    if tools(&json!({"tools": new_ovr["tools_add"]}))
        .iter()
        .any(|t| !old_added.contains(t))
        || new_ovr.get("tools_replace") != old_ovr.get("tools_replace")
            && !new_ovr["tools_replace"].is_null()
        || new_ovr.get("tool_confirm") != old_ovr.get("tool_confirm")
            && !new_ovr["tool_confirm"].is_null()
    {
        return true;
    }
    ["allow_integrations", "allow_mcp", "allow_subagents"]
        .iter()
        .any(|k| !flag(&old_ovr, k) && flag(&new_ovr, k))
        || [
            "auto_approve_editing_tools",
            "auto_approve_dangerous_commands",
        ]
        .iter()
        .any(|k| !flag(&old_ovr["thread_defaults"], k) && flag(&new_ovr["thread_defaults"], k))
}

/// For a file that overrides another mode or subagent (`base` + `match_models`): the base ids it
/// affects with their resolved configs before and after the change, for the models it matches.
async fn resolved_overrides(
    ctx: &EngineCtx,
    kind: &str,
    original: &Value,
    merged: &Value,
) -> Result<Vec<(String, Value, Value)>, EngineError> {
    use crate::yaml_configs::customization_types::ModeConfig;
    let is_override = |v: &Value| v["base"].is_string() && !v["match_models"].is_null();
    if !is_override(original) && !is_override(merged) {
        return Ok(Vec::new());
    }
    let root = project_root(ctx).await;
    let registry = crate::yaml_configs::customization_registry::load_merged_registry(
        &ctx.app.gcx.config_dir,
        root.as_deref(),
    )
    .await;
    let mut bases: Vec<String> = [original, merged]
        .iter()
        .filter(|v| is_override(v))
        .filter_map(|v| v["base"].as_str().map(String::from))
        .collect();
    bases.dedup();
    let mut out = Vec::new();
    for base in bases {
        let side = |v: &Value| -> Result<Option<Value>, EngineError> {
            if !is_override(v) || v["base"].as_str() != Some(base.as_str()) {
                return Ok(None);
            }
            Ok(Some(v.clone()))
        };
        let (before, after) = (side(original)?, side(merged)?);
        let resolve = |file: Option<Value>| -> Result<Value, EngineError> {
            match kind {
                "modes" => {
                    let base_mode = registry.modes.get(&base).cloned().ok_or_else(|| {
                        EngineError::Type(format!("override base mode {base} does not exist"))
                    })?;
                    let resolved = match file {
                        Some(file) => {
                            let file: ModeConfig = serde_json::from_value(file)
                                .map_err(|e| EngineError::Type(e.to_string()))?;
                            match &file.override_config {
                                Some(ovr) => base_mode.apply_override(ovr),
                                None => base_mode,
                            }
                        }
                        None => base_mode,
                    };
                    to_value(&resolved)
                }
                _ => {
                    let base_agent = registry.subagents.get(&base).cloned().ok_or_else(|| {
                        EngineError::Type(format!("override base subagent {base} does not exist"))
                    })?;
                    let resolved = match file {
                        Some(file) => base_agent.apply_override(
                            &serde_json::from_value(file)
                                .map_err(|e| EngineError::Type(e.to_string()))?,
                        ),
                        None => base_agent,
                    };
                    to_value(&resolved)
                }
            }
        };
        out.push((base.clone(), resolve(before)?, resolve(after)?));
    }
    Ok(out)
}

/// Models the saved config pins for itself, its overrides or its subchats.
fn pinned_models(config: &Value) -> Vec<String> {
    let mut models = Vec::new();
    for root in [config, &config["override"]] {
        for slot in ["default", "light", "thinking"] {
            if let Some(m) = root["model_defaults"][slot]["model"].as_str() {
                models.push(m.to_string());
            }
        }
        if let Some(m) = root["subchat"]["model"].as_str() {
            models.push(m.to_string());
        }
    }
    models
}

async fn save_target(
    ctx: &EngineCtx,
    args: &Value,
) -> Result<(PathBuf, String, Option<Vec<u8>>), EngineError> {
    let kind = args["kind"].as_str().unwrap_or("modes");
    let id = args["id"].as_str().unwrap_or_default();
    crate::yaml_configs::customization_service::validate_id(id).map_err(EngineError::Type)?;
    let global = ctx.app.gcx.config_dir.join(kind).join(format!("{id}.yaml"));
    let local = project_root(ctx)
        .await
        .map(|r| r.join(".refact").join(kind).join(format!("{id}.yaml")));
    let (path, scope) = match args.get("scope").and_then(Value::as_str) {
        Some("global") => (global, "global"),
        Some(_) => (
            local.ok_or_else(|| EngineError::Type("no project root for local scope".into()))?,
            "local",
        ),
        None => match local {
            Some(l) if l.exists() => (l, "local"),
            _ if global.exists() => (global, "global"),
            Some(l) => (l, "local"),
            None => (global, "global"),
        },
    };
    if scope == "local" {
        let root = project_root(ctx)
            .await
            .ok_or_else(|| EngineError::Type("no project root for local scope".into()))?;
        super::common::inside_project(&root, &path, &ctx.app.gcx.config_dir)?;
    }
    let current = match tokio::fs::read(&path).await {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(failed(e)),
    };
    Ok((path, scope.to_string(), current))
}

fn shorten(value: &Value) -> Value {
    match value {
        Value::String(s) if s.chars().count() > 200 => json!(format!(
            "{}… ({} chars)",
            s.chars().take(200).collect::<String>(),
            s.chars().count()
        )),
        other => other.clone(),
    }
}

fn save_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let kind = args["kind"].as_str().unwrap_or("modes");
        let id = args["id"].as_str().unwrap_or_default();
        let (path, scope, current) = save_target(ctx, args).await?;
        let version = current
            .as_deref()
            .map(crate::yaml_configs::customization_service::version);
        if let Some(expected) = args.get("expected_version").and_then(Value::as_str) {
            if version.as_deref() != Some(expected) {
                return Err(EngineError::Failed(format!(
                    "{kind}/{id} changed since expected_version; read it again with modes.get"
                )));
            }
        }
        let original = match &current {
            Some(bytes) => serde_yaml::from_slice::<Value>(bytes).map_err(|e| {
                EngineError::Failed(format!("the existing file does not parse: {e}"))
            })?,
            None => Value::Null,
        };
        let merged = if original.is_null() {
            args["config"].clone()
        } else {
            crate::yaml_configs::customization_service::merge_preserving_unknown(
                &original,
                &args["config"],
            )
            .map_err(EngineError::Type)?
        };
        crate::yaml_configs::customization_service::validate_config(kind, &merged, id)
            .map_err(EngineError::Type)?;
        if original.is_null() && kind == "modes" {
            if let Some(builtin) = crate::yaml_configs::customization_registry::get_mode_config(
                ctx.app.gcx.clone(),
                id,
                None,
            )
            .await
            {
                if builtin.specific {
                    return Err(EngineError::Permission(format!(
                        "mode {id} is an internal mode and cannot be redefined from a script"
                    )));
                }
            }
        }
        let mut widening = widens(&original, &merged)
            || (original.is_null() && merged["tools"].as_array().is_some_and(|t| !t.is_empty()));
        let current_mode = super::chat::this_session(ctx)
            .await?
            .lock()
            .await
            .thread
            .mode
            .clone();
        if widening && kind == "modes" && id == current_mode {
            return Err(EngineError::Permission(format!(
                "mode {id} is this chat's mode; widening it from a script would grant this chat new permissions, so change it in the GUI"
            )));
        }
        let mut widened_bases = Vec::new();
        if matches!(kind, "modes" | "subagents") {
            for (base, before, after) in resolved_overrides(ctx, kind, &original, &merged).await? {
                if widens(&before, &after) {
                    if kind == "modes" && base == current_mode {
                        return Err(EngineError::Permission(format!(
                            "this file overrides mode {base}, this chat's mode, and widens it; that would grant this chat new permissions, so change it in the GUI"
                        )));
                    }
                    widened_bases.push(base);
                }
            }
        }
        widening |= !widened_bases.is_empty();
        let mut destinations = vec![ctx.ccx.lock().await.current_model.clone()];
        destinations.extend(pinned_models(&merged));
        if let Ok(caps) =
            crate::global_context::try_load_caps_quickly_if_not_present(ctx.app.gcx.clone(), 0)
                .await
        {
            destinations.push(caps.defaults.chat_default_model.clone());
        }
        for model in destinations.iter().filter(|m| !m.is_empty()) {
            ctx.clear_for(&crate::privacy::records::provider_destination(model))
                .await
                .map_err(|refusal| {
                    EngineError::Permission(format!(
                        "this text would reach models of other chats: {refusal}"
                    ))
                })?;
        }
        let mut items = Vec::new();
        let keys: std::collections::BTreeSet<String> = merged
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, _)| k.clone())
            .chain(
                original
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(k, _)| k.clone()),
            )
            .collect();
        for key in keys {
            let (o, n) = (original.get(&key), merged.get(&key));
            if o == n {
                continue;
            }
            if key == "tools" {
                let names = |v: Option<&Value>| -> Vec<String> {
                    v.and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|t| t.as_str().map(String::from))
                        .collect()
                };
                let (old_t, new_t) = (names(o), names(n));
                let added: Vec<_> = new_t
                    .iter()
                    .filter(|t| !old_t.contains(t))
                    .cloned()
                    .collect();
                let removed: Vec<_> = old_t
                    .iter()
                    .filter(|t| !new_t.contains(t))
                    .cloned()
                    .collect();
                if !added.is_empty() {
                    items.push(super::mask::item(
                        "tools added",
                        None,
                        Some(json!(added.join(", "))),
                    ));
                }
                if !removed.is_empty() {
                    items.push(super::mask::item(
                        "tools removed",
                        Some(json!(removed.join(", "))),
                        None,
                    ));
                }
                continue;
            }
            items.push(super::mask::item(&key, o.map(shorten), n.cloned()));
        }
        if !widened_bases.is_empty() {
            items.push(super::mask::item(
                "widens",
                None,
                Some(json!(widened_bases.join(", "))),
            ));
        }
        let yaml = serde_yaml::to_string(&merged).map_err(failed)?;
        let kind_word = kind.trim_end_matches('s').replace('_', " ");
        let created = current.is_none();
        let plan = if items.is_empty() {
            Plan::nothing()
        } else {
            Plan::new(
                json!({"version":version}),
                SavePlan {
                    path: path.clone(),
                    scope: scope.clone(),
                    expected: version.clone(),
                    bytes: yaml.into_bytes(),
                    created,
                },
            )
        };
        Ok(Preview {
            title: format!(
                "{} a {kind_word}",
                if created { "create" } else { "change" }
            ),
            items,
            scope: Some(if scope == "global" {
                format!("all chats using {id} (global)")
            } else {
                "this project".into()
            }),
            class: if widening {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning: widening.then(|| {
                "This lets the mode use more tools or run them without asking you.".to_string()
            }),
            plan,
        })
    })
}

fn save<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let kind = args["kind"].as_str().unwrap_or("modes").to_string();
        let id = args["id"].as_str().unwrap_or_default().to_string();
        let (_, _, current) = save_target(ctx, args).await?;
        let version = current
            .as_deref()
            .map(crate::yaml_configs::customization_service::version);
        let plan: SavePlan = ctx.take_plan("modes.save", &json!({"version":version}))?;
        if let Some(parent) = plan.path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(failed)?;
        }
        let write = |bytes: Vec<u8>, expected: Option<String>| {
            let path = plan.path.clone();
            tokio::task::spawn_blocking(move || {
                crate::yaml_configs::customization_service::replace_checked(
                    &path,
                    expected.as_deref(),
                    &bytes,
                )
            })
        };
        write(plan.bytes.clone(), plan.expected.clone())
            .await
            .map_err(failed)?
            .map_err(|e| EngineError::Failed(format!("{kind}/{id} was not saved: {e}")))?;
        let gcx = ctx.app.gcx.clone();
        crate::yaml_configs::customization_registry::invalidate_all_registry_caches(gcx.clone())
            .await;
        let root = project_root(ctx).await;
        let registry = crate::yaml_configs::customization_registry::load_merged_registry(
            &gcx.config_dir,
            root.as_deref(),
        )
        .await;
        let path_text = plan.path.to_string_lossy().into_owned();
        let broken: Vec<_> = registry
            .errors
            .iter()
            .filter(|e| e.file_path == path_text)
            .map(|e| scrub_text(&e.error, &[]))
            .collect();
        if !broken.is_empty() {
            let new_version = crate::yaml_configs::customization_service::version(&plan.bytes);
            match &current {
                Some(old) => write(old.clone(), Some(new_version))
                    .await
                    .map_err(failed)?
                    .map_err(failed)?,
                None => tokio::fs::remove_file(&plan.path).await.map_err(failed)?,
            }
            crate::yaml_configs::customization_registry::invalidate_all_registry_caches(gcx).await;
            return Err(EngineError::Failed(format!(
                "{kind}/{id} does not load ({}); the previous file was restored",
                broken.join("; ")
            )));
        }
        let new_version = crate::yaml_configs::customization_service::version(&plan.bytes);
        Ok(EngineOutput::new(
            json!({"kind":kind,"id":id,"file_path":path_text,"scope":plan.scope,"created":plan.created,"version":new_version}),
            format!(
                "{} {kind}/{id} ({}); live from each chat's next turn",
                if plan.created { "created" } else { "saved" },
                plan.scope
            ),
        ))
    })
}

async fn project_info_state(
    ctx: &EngineCtx,
) -> Result<
    (
        crate::yaml_configs::project_information::ProjectInformationConfig,
        Value,
    ),
    EngineError,
> {
    let root = project_root(ctx)
        .await
        .ok_or_else(|| EngineError::Type("no project directory is open".into()))?;
    super::common::inside_project(
        &root,
        &root.join(".refact").join("project_information.yaml"),
        &ctx.app.gcx.config_dir,
    )?;
    let config = crate::yaml_configs::project_information::load_project_information_config(
        ctx.app.gcx.clone(),
    )
    .await;
    let state = to_value(&config)?;
    Ok((config, state))
}

fn set_project_info_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        if args.get("enabled").is_none() && args.get("sections").is_none() {
            return Err(EngineError::Type("give enabled or sections".into()));
        }
        let (config, state) = project_info_state(ctx).await?;
        let mut new = state.clone();
        let mut items = Vec::new();
        if let Some(enabled) = args.get("enabled").and_then(Value::as_bool) {
            if config.enabled != enabled {
                items.push(super::mask::item(
                    "project information",
                    Some(json!(config.enabled)),
                    Some(json!(enabled)),
                ));
            }
            new["enabled"] = json!(enabled);
        }
        for (section, value) in args["sections"].as_object().into_iter().flatten() {
            let enabled = value["enabled"].as_bool().unwrap_or(true);
            let old = state["sections"][section]["enabled"]
                .as_bool()
                .unwrap_or(true);
            if old != enabled {
                items.push(super::mask::item(
                    section,
                    Some(json!(old)),
                    Some(json!(enabled)),
                ));
            }
            new["sections"][section]["enabled"] = json!(enabled);
        }
        let plan = if items.is_empty() {
            Plan::nothing()
        } else {
            Plan::new(state, new)
        };
        Ok(Preview {
            title: "change project information settings".into(),
            items,
            scope: Some("this project, for new chats".into()),
            class: EngineClass::Change,
            warning: None,
            plan,
        })
    })
}

fn set_project_info<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let (_, state) = project_info_state(ctx).await?;
        let new: Value = ctx.take_plan("modes.set_project_info", &state)?;
        let config: crate::yaml_configs::project_information::ProjectInformationConfig =
            serde_json::from_value(new.clone()).map_err(failed)?;
        crate::yaml_configs::project_information::save_project_information_config(
            ctx.app.gcx.clone(),
            &config,
        )
        .await
        .map_err(failed)?;
        let mut changed = serde_json::Map::new();
        if state["enabled"] != new["enabled"] {
            changed.insert(
                "enabled".into(),
                json!({"old":state["enabled"],"new":new["enabled"]}),
            );
        }
        for section in SECTIONS {
            let (o, n) = (
                &state["sections"][section]["enabled"],
                &new["sections"][section]["enabled"],
            );
            if o != n {
                changed.insert(section.to_string(), json!({"old":o,"new":n}));
            }
        }
        let text = format!(
            "{} project information settings changed; new chats use them",
            changed.len()
        );
        Ok(EngineOutput::new(json!({"changed":changed}), text))
    })
}

async fn skills_file_inside(ctx: &EngineCtx) -> Result<(), EngineError> {
    let root = project_root(ctx)
        .await
        .ok_or_else(|| EngineError::Type("no project directory is open".into()))?;
    super::common::inside_project(
        &root,
        &root.join(".refact").join("skills.yaml"),
        &ctx.app.gcx.config_dir,
    )
}

fn set_skills_trigger_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        skills_file_inside(ctx).await?;
        let old = to_value(
            crate::http::routers::v1::settings_config::skills_auto_trigger(&ctx.app)
                .await
                .map_err(EngineError::Failed)?,
        )?;
        let new = args["auto_trigger"].clone();
        let mut items = vec![super::mask::item(
            "auto_trigger",
            Some(old.clone()),
            Some(new.clone()),
        )];
        if new == "inject_full" {
            items.push(super::mask::item(
                "prompt size",
                None,
                Some(json!("+ all skill texts")),
            ));
        }
        Ok(Preview {
            title: "change skills auto-trigger".into(),
            items,
            scope: Some("this project".into()),
            class: EngineClass::Change,
            warning: None,
            plan: if old == new {
                Plan::nothing()
            } else {
                super::previewed::plan(old)
            },
        })
    })
}

fn set_skills_trigger<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        skills_file_inside(ctx).await?;
        let old = to_value(
            crate::http::routers::v1::settings_config::skills_auto_trigger(&ctx.app)
                .await
                .map_err(EngineError::Failed)?,
        )?;
        super::previewed::take(ctx, "modes.set_skills_trigger", &old)?;
        let value = args["auto_trigger"].as_str().unwrap_or("index_only");
        crate::http::routers::v1::settings_config::save_skills_auto_trigger(&ctx.app, value)
            .await
            .map_err(EngineError::Failed)?;
        Ok(EngineOutput::new(
            json!({"auto_trigger":value}),
            format!("skills auto-trigger set to {value}"),
        ))
    })
}

struct DeleteTarget {
    path: PathBuf,
    scope: crate::yaml_configs::customization_registry::ConfigScope,
    root: Option<PathBuf>,
    bytes: Vec<u8>,
}

async fn delete_target(ctx: &EngineCtx, args: &Value) -> Result<DeleteTarget, EngineError> {
    use crate::http::routers::v1::customization_editor::customization_delete_target;
    let kind = args["kind"].as_str().unwrap_or_default();
    let id = args["mode"].as_str().unwrap_or_default();
    let (path, scope, root) = customization_delete_target(
        ctx.app.gcx.clone(),
        kind,
        id,
        args.get("scope").and_then(Value::as_str),
    )
    .await
    .map_err(|(_, message)| EngineError::Type(message))?;
    if scope == crate::yaml_configs::customization_registry::ConfigScope::Local {
        if let Some(root) = &root {
            super::common::inside_project(root, &path, &ctx.app.gcx.config_dir)?;
        }
    }
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(EngineError::Type(format!(
                "{kind}/{id} has no file in this scope ({}); builtin definitions have no file to delete",
                path.display()
            )))
        }
        Err(e) => return Err(failed(e)),
    };
    Ok(DeleteTarget {
        path,
        scope,
        root,
        bytes,
    })
}

/// What defines `kind/id` once the file at `path` is gone.
async fn fallback(ctx: &EngineCtx, kind: &str, id: &str, target: &DeleteTarget) -> String {
    use crate::yaml_configs::customization_registry::ConfigScope;
    use crate::yaml_configs::project_configs_bootstrap::{compute_checksum, get_default_checksum};
    let file = format!("{id}.yaml");
    let builtin = get_default_checksum(kind, &file);
    match target.scope {
        ConfigScope::Local => {
            let global = ctx.app.gcx.config_dir.join(kind).join(&file);
            match tokio::fs::read_to_string(&global).await {
                Ok(content) if builtin.as_deref() == Some(compute_checksum(&content).as_str()) => {
                    format!("builtin {kind}/{id}")
                }
                Ok(_) => format!("global {kind}/{id} ({})", global.display()),
                Err(_) if builtin.is_some() => format!("builtin {kind}/{id}"),
                Err(_) => "nothing".into(),
            }
        }
        ConfigScope::Global => {
            if let Some(local) = target
                .root
                .as_ref()
                .map(|r| r.join(".refact").join(kind).join(&file))
                .filter(|l| l.exists())
            {
                format!("local {kind}/{id} ({})", local.display())
            } else if builtin.is_some() {
                format!("builtin {kind}/{id} (written again on the next load)")
            } else {
                "nothing".into()
            }
        }
    }
}

/// The raw config that defines `kind/id` once the file is gone, read from the same place
/// `fallback` names; `None` when nothing replaces it.
async fn fallback_config(
    ctx: &EngineCtx,
    kind: &str,
    id: &str,
    target: &DeleteTarget,
) -> Result<Option<Value>, EngineError> {
    use crate::yaml_configs::customization_registry::ConfigScope;
    let file = format!("{id}.yaml");
    let other = match target.scope {
        ConfigScope::Local => Some(ctx.app.gcx.config_dir.join(kind).join(&file)),
        ConfigScope::Global => target
            .root
            .as_ref()
            .map(|r| r.join(".refact").join(kind).join(&file)),
    };
    let text = match other {
        Some(path) => match tokio::fs::read_to_string(&path).await {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(failed(e)),
        },
        None => None,
    };
    let text = text.or_else(|| {
        crate::yaml_configs::project_configs_bootstrap::embedded_defaults(kind)
            .into_iter()
            .find(|(name, _)| *name == file)
            .map(|(_, content)| content)
    });
    text.map(|text| {
        serde_yaml::from_str::<Value>(&text).map_err(|e| {
            EngineError::Failed(format!(
                "the definition that would replace {kind}/{id} does not parse: {e}"
            ))
        })
    })
    .transpose()
}

fn delete_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let kind = args["kind"].as_str().unwrap_or_default();
        let id = args["mode"].as_str().unwrap_or_default();
        let target = delete_target(ctx, args).await?;
        let falls_back = fallback(ctx, kind, id, &target).await;
        let nothing = falls_back == "nothing";
        let current = {
            let mode = super::chat::this_session(ctx)
                .await?
                .lock()
                .await
                .thread
                .mode
                .clone();
            crate::yaml_configs::customization_registry::map_legacy_mode_to_id(&mode).to_string()
        };
        if nothing && matches!(kind, "modes") && current == id {
            return Err(EngineError::Permission(format!(
                "mode {id} is this chat's mode and nothing would replace it; the chat would fail on its next turn"
            )));
        }
        let mut widening = false;
        if !nothing && matches!(kind, "modes" | "subagents") {
            let deleted: Value = serde_yaml::from_slice(&target.bytes).map_err(|e| {
                EngineError::Failed(format!("the file to delete does not parse: {e}"))
            })?;
            let mut widened = Vec::new();
            if let Some(successor) = fallback_config(ctx, kind, id, &target).await? {
                if widens(&deleted, &successor) {
                    widened.push(id.to_string());
                }
            }
            for (base, before, after) in
                resolved_overrides(ctx, kind, &deleted, &Value::Null).await?
            {
                if widens(&before, &after) {
                    widened.push(base);
                }
            }
            if kind == "modes" && widened.contains(&current) {
                return Err(EngineError::Permission(format!(
                    "after this delete mode {current}, this chat's mode, would be defined by a wider definition; that would grant this chat new permissions, so delete it in the GUI"
                )));
            }
            widening = !widened.is_empty();
        }
        let content = String::from_utf8_lossy(&target.bytes).into_owned();
        let version = crate::yaml_configs::customization_service::version(&target.bytes);
        let kind_word = kind.trim_end_matches('s').replace('_', " ");
        Ok(Preview {
            title: format!("delete a {kind_word}"),
            items: vec![
                super::mask::item(
                    "file",
                    Some(json!(target.path.to_string_lossy())),
                    Some(json!("deleted")),
                ),
                super::mask::item("falls back to", None, Some(json!(falls_back))),
                super::mask::item("content", Some(json!(content)), None),
            ],
            scope: Some(match target.scope {
                crate::yaml_configs::customization_registry::ConfigScope::Global => {
                    format!("all chats using {id} (global)")
                }
                crate::yaml_configs::customization_registry::ConfigScope::Local => {
                    "this project".into()
                }
            }),
            class: if nothing {
                EngineClass::Destructive
            } else if widening {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning: if nothing {
                Some("The definition file will be deleted and cannot be restored.".to_string())
            } else {
                widening.then(|| {
                    "What takes over lets the mode use more tools or run them without asking you."
                        .to_string()
                })
            },
            plan: Plan::new(json!({"version":version}), falls_back),
        })
    })
}

fn delete<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let kind = args["kind"].as_str().unwrap_or_default();
        let id = args["mode"].as_str().unwrap_or_default();
        let target = delete_target(ctx, args).await?;
        let version = crate::yaml_configs::customization_service::version(&target.bytes);
        let falls_back: String = ctx.take_plan("modes.delete", &json!({"version":version}))?;
        let response = crate::http::routers::v1::customization_editor::delete_customization_file(
            ctx.app.gcx.clone(),
            &target.path,
            target.scope,
            target.root.as_deref(),
        )
        .await
        .map_err(|(_, message)| EngineError::Failed(message))?;
        let errors: Vec<Value> = response
            .errors
            .iter()
            .map(|e| json!({"file_path":e.file_path,"error":scrub_text(&e.error, &[])}))
            .collect();
        let mut data = json!({"kind":kind,"id":id,"scope":response.scope,"deleted":true,"falls_back_to":falls_back});
        if !errors.is_empty() {
            data["errors"] = json!(errors);
        }
        Ok(EngineOutput::new(
            data,
            format!(
                "{kind}/{id} deleted ({}); now: {falls_back}",
                response.scope
            ),
        ))
    })
}
