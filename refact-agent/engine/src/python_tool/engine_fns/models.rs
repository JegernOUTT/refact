use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::common::{failed, to_value};
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput};
use crate::caps::{ChatModelRecord, CodeAssistantCaps};

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "models.list",
            summary: "Chat models available now with context window, output limit, features, prices and which default slots point at them.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{
                "provider":{"type":"string","minLength":1,"description":"Provider instance name to keep."},
                "include_disabled":{"type":"boolean","description":"Include disabled models; false by default."}
            }}),
            output: || Some(json!({"type":"object","required":["models"],"properties":{"models":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"provider":{"type":"string"},"display_name":{"type":"string"},"n_ctx":{"type":"integer"},"max_output_tokens":{"type":"integer"},"supports_tools":{"type":"boolean"},"supports_agent":{"type":"boolean"},"supports_multimodality":{"type":"boolean"},"supports_parallel_tools":{"type":"boolean"},"supports_temperature":{"type":"boolean"},"reasoning_effort_options":{"type":"array"},"supports_thinking_budget":{"type":"boolean"},"max_thinking_tokens":{"type":"integer"},"pricing":{"type":"object","description":"USD per 1M tokens as stored"},"slots":{"type":"array","items":{"enum":["chat","chat_model_2","task_planner","light","thinking","buddy"]}},"enabled":{"type":"boolean"},"experimental":{"type":"boolean"}}}}}})),
            handler: list,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "models.get",
            summary: "The full capabilities of one model as the engine resolves it, with the registry entry they came from.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["model"],"properties":{"model":{"type":"string","minLength":1,"description":"Model id, e.g. openai/gpt-4.1."}}}),
            output: || Some(json!({"type":"object","required":["id","in_caps"],"properties":{"id":{"type":"string"},"resolved":{"type":"boolean"},"n_ctx":{"type":"integer"},"max_output_tokens":{"type":"integer"},"features":{"type":"object"},"default_temperature":{"type":"number"},"default_max_tokens":{"type":"integer"},"registry":{"type":"object","properties":{"matched_key":{"type":"string"},"source":{"type":"string"}}},"in_caps":{"type":"boolean"}}})),
            handler: get,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "models.defaults",
            summary: "The default model slots and their parameters, globally and for this project, and the effective result.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["global","project","effective","resolved"],"properties":{"global":{"type":"object"},"project":{"type":"object","properties":{"available":{"type":"boolean"},"root":{"type":"string"},"path":{"type":"string"},"defaults":{"type":"object"}}},"effective":{"type":"object"},"resolved":{"type":"object","description":"slot models after fallback for models that no longer exist"}}})),
            handler: defaults,
            preview: None,
            timeout: None,
        },
    ]
    .into_iter()
    .chain(change_functions())
    .collect()
}

async fn caps(ctx: &EngineCtx) -> Result<Arc<CodeAssistantCaps>, EngineError> {
    crate::global_context::try_load_caps_quickly_if_not_present(ctx.app.gcx.clone(), 0)
        .await
        .map_err(|e| {
            EngineError::Failed(format!("model capabilities are unavailable: {}", e.message))
        })
}

fn features(m: &ChatModelRecord) -> Value {
    let mut out = json!({
        "supports_tools":m.supports_tools,
        "supports_agent":m.supports_agent,
        "supports_multimodality":m.supports_multimodality,
        "supports_parallel_tools":m.supports_parallel_tools,
        "supports_temperature":m.supports_temperature,
        "supports_thinking_budget":m.supports_thinking_budget,
    });
    if let Some(options) = &m.reasoning_effort_options {
        out["reasoning_effort_options"] = json!(options);
    }
    if let Some(max) = m.max_thinking_tokens {
        out["max_thinking_tokens"] = json!(max);
    }
    out
}

fn slots(caps: &CodeAssistantCaps, id: &str) -> Vec<&'static str> {
    let d = &caps.defaults;
    [
        (&d.chat_default_model, "chat"),
        (&d.chat_model_2, "chat_model_2"),
        (&d.task_planner_agent_model, "task_planner"),
        (&d.chat_light_model, "light"),
        (&d.chat_thinking_model, "thinking"),
        (&d.chat_buddy_model, "buddy"),
    ]
    .into_iter()
    .filter(|(slot, _)| slot.as_str() == id)
    .map(|(_, name)| name)
    .collect()
}

fn list<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let caps = caps(ctx).await?;
        let provider = args.get("provider").and_then(Value::as_str);
        let include_disabled = args.get("include_disabled").and_then(Value::as_bool) == Some(true);
        let mut models = Vec::new();
        for (id, m) in &caps.chat_models {
            let model_provider = id.split_once('/').map_or(id.as_str(), |(p, _)| p);
            if provider.is_some_and(|p| p != model_provider)
                || (!m.base.enabled && !include_disabled)
            {
                continue;
            }
            let mut row = features(m);
            row["id"] = json!(id);
            row["provider"] = json!(model_provider);
            row["n_ctx"] = json!(m.base.n_ctx);
            row["slots"] = json!(slots(&caps, id));
            row["enabled"] = json!(m.base.enabled);
            row["experimental"] = json!(m.base.experimental);
            if let Some(name) = &m.base.display_name {
                row["display_name"] = json!(name);
            }
            if let Some(max) = m.max_output_tokens {
                row["max_output_tokens"] = json!(max);
            }
            if let Some(p) = &m.pricing {
                let mut pricing = json!({"prompt":p.prompt,"generated":p.generated});
                if let Some(v) = p.cache_read {
                    pricing["cache_read"] = json!(v);
                }
                if let Some(v) = p.cache_creation {
                    pricing["cache_creation"] = json!(v);
                }
                row["pricing"] = pricing;
            }
            models.push(row);
        }
        let data = json!({"models":models});

        let text = format!(
            "{} chat models; default {}",
            models.len(),
            caps.defaults.chat_default_model
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn get<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let model = args["model"].as_str().unwrap_or_default();
        let caps = caps(ctx).await?;
        let registry = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            crate::caps::model_caps::get_model_caps(ctx.app.gcx.clone(), false),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .and_then(|all| crate::caps::model_caps::resolve_model_caps(&all, model));
        let registry_json = registry
            .as_ref()
            .map(|r| json!({"matched_key":r.matched_key,"source":r.source}));
        let mut data = match crate::caps::resolve_chat_model(caps.clone(), model) {
            Ok(m) => {
                let mut data = json!({"id":m.base.id,"resolved":true,"in_caps":true,"n_ctx":m.base.n_ctx,"features":features(&m)});
                if let Some(v) = m.max_output_tokens {
                    data["max_output_tokens"] = json!(v);
                }
                if let Some(v) = m.default_temperature {
                    data["default_temperature"] = json!(v);
                }
                if let Some(v) = m.default_max_tokens {
                    data["default_max_tokens"] = json!(v);
                }
                data
            }
            Err(error) => match &registry {
                Some(r) => {
                    let c = &r.caps;
                    json!({"id":model,"resolved":false,"in_caps":false,"n_ctx":c.n_ctx,"max_output_tokens":c.max_output_tokens,"features":{"supports_tools":c.supports_tools,"supports_vision":c.supports_vision,"supports_parallel_tools":c.supports_parallel_tools,"supports_temperature":c.supports_temperature,"supports_thinking_budget":c.supports_thinking_budget,"reasoning_effort_options":c.reasoning_effort_options}})
                }
                None => return Err(EngineError::Type(format!("unknown model {model}: {error}"))),
            },
        };
        if let Some(registry) = registry_json {
            data["registry"] = registry;
        }
        let text = format!(
            "{} ({}), context {}",
            data["id"].as_str().unwrap_or(model),
            if data["in_caps"] == true {
                "available"
            } else {
                "registry only"
            },
            data["n_ctx"]
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn defaults<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        use refact_core::provider_types::{
            project_model_defaults_path, ProjectModelDefaults, ProviderDefaults,
        };
        let gcx = ctx.app.gcx.clone();
        let global = ProviderDefaults::load(&gcx.config_dir)
            .await
            .map_err(failed)?;
        let root = crate::files_correction::get_project_dirs(gcx.clone())
            .await
            .into_iter()
            .next();
        let (project, project_json) = match &root {
            Some(root) => {
                let defaults = ProjectModelDefaults::load(root).await.map_err(failed)?;
                let json = json!({"available":true,"root":root,"path":project_model_defaults_path(root),"defaults":to_value(&defaults)?});
                (defaults, json)
            }
            None => (
                ProjectModelDefaults::default(),
                json!({"available":false,"defaults":{}}),
            ),
        };
        let mut effective = global.clone();
        effective.apply_project_overrides(&project);
        let caps = caps(ctx).await?;
        let d = &caps.defaults;
        let resolved = json!({"chat":d.chat_default_model,"chat_model_2":d.chat_model_2,"task_planner_agent_model":d.task_planner_agent_model,"chat_light":d.chat_light_model,"chat_thinking":d.chat_thinking_model,"chat_buddy":d.chat_buddy_model});
        let marks = |slot: &str, model: &str| {
            let overridden = to_value(&project).ok().is_some_and(|p| !p[slot].is_null());
            format!(
                "{slot}={}{}",
                if model.is_empty() { "-" } else { model },
                if overridden {
                    " (project override)"
                } else {
                    ""
                }
            )
        };
        let text = [
            marks("chat", &d.chat_default_model),
            marks("chat_light", &d.chat_light_model),
            marks("chat_thinking", &d.chat_thinking_model),
        ]
        .join(", ");
        Ok(EngineOutput::new(
            json!({"global":to_value(&global)?,"project":project_json,"effective":to_value(&effective)?,"resolved":resolved}),
            text,
        ))
    })
}

// ---------------------------------------------------------------------------
// Changes.

use refact_core::provider_types::{ModelTypeDefaults, ProjectModelDefaults, ProviderDefaults};

use super::{Plan, Preview};

const SLOTS: &[&str] = &[
    "chat",
    "chat_model_2",
    "task_planner_agent_model",
    "chat_light",
    "chat_thinking",
    "chat_buddy",
];

pub(super) fn change_functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "models.set_defaults",
            summary: "Set the model and parameters of default model slots, globally or for this project; only the given slots change. Risky when a slot moves to another provider.",
            class: EngineClass::Change,
            input: || {
                let slot = json!({"type":["object","null"],"additionalProperties":false,"properties":{
                    "model":{"type":["string","null"],"minLength":1},
                    "max_new_tokens":{"type":"integer","minimum":1,"maximum":1000000},
                    "temperature":{"type":"number","minimum":0,"maximum":2},
                    "top_p":{"type":"number","minimum":0,"maximum":1},
                    "boost_reasoning":{"type":"boolean"},
                    "reasoning_effort":{"type":"string","minLength":1},
                    "thinking_budget":{"type":"integer","minimum":1}
                },"description":"null: remove the project override, or reset the global slot."});
                let mut slots = serde_json::Map::new();
                for name in SLOTS {
                    slots.insert(name.to_string(), slot.clone());
                }
                slots.insert("completion_model".into(), json!({"type":["string","null"],"description":"Legacy ignored input; has no effect."}));
                slots.insert("embedding_model".into(), json!({"type":["string","null"],"description":"Legacy ignored input; has no effect."}));
                json!({"type":"object","additionalProperties":false,"required":["scope","slots"],"properties":{
                    "scope":{"enum":["global","project"]},
                    "slots":{"type":"object","additionalProperties":false,"minProperties":1,"properties":slots}
                }})
            },
            output: || Some(json!({"type":"object","required":["scope","path","changed"],"properties":{"scope":{"type":"string"},"path":{"type":"string"},"changed":{"type":"object"}}})),
            handler: set_defaults,
            preview: Some(set_defaults_preview),
            timeout: None,
        },
        EngineFn {
            name: "models.reload",
            summary: "Drop the cached model list and rebuild it from provider configs now (may call providers); refresh_registry also refetches the capability registry.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"refresh_registry":{"type":"boolean"}}}),
            output: || Some(json!({"type":"object","required":["chat_models","default_chat","took_ms"],"properties":{"chat_models":{"type":"integer"},"default_chat":{"type":"string"},"took_ms":{"type":"integer"}}})),
            handler: reload,
            preview: Some(reload_preview),
            timeout: Some(std::time::Duration::from_secs(120)),
        },
    ]
}

#[derive(Clone)]
enum DefaultsTarget {
    Global(ProviderDefaults),
    Project(std::path::PathBuf, ProjectModelDefaults),
}

fn slot_of<'a>(
    defaults: &'a mut ProviderDefaults,
    name: &str,
) -> Option<&'a mut ModelTypeDefaults> {
    Some(match name {
        "chat" => &mut defaults.chat,
        "chat_model_2" => &mut defaults.chat_model_2,
        "task_planner_agent_model" => &mut defaults.task_planner_agent_model,
        "chat_light" => &mut defaults.chat_light,
        "chat_thinking" => &mut defaults.chat_thinking,
        "chat_buddy" => &mut defaults.chat_buddy,
        _ => return None,
    })
}

fn project_slot<'a>(
    defaults: &'a mut ProjectModelDefaults,
    name: &str,
) -> Option<&'a mut Option<ModelTypeDefaults>> {
    Some(match name {
        "chat" => &mut defaults.chat,
        "chat_model_2" => &mut defaults.chat_model_2,
        "task_planner_agent_model" => &mut defaults.task_planner_agent_model,
        "chat_light" => &mut defaults.chat_light,
        "chat_thinking" => &mut defaults.chat_thinking,
        "chat_buddy" => &mut defaults.chat_buddy,
        _ => return None,
    })
}

async fn defaults_state(
    ctx: &EngineCtx,
    scope: &str,
) -> Result<(Value, DefaultsTarget, String), EngineError> {
    let gcx = ctx.app.gcx.clone();
    if scope == "global" {
        let global = ProviderDefaults::load(&gcx.config_dir)
            .await
            .map_err(failed)?;
        let path = gcx.config_dir.join("providers.d").join("defaults.yaml");
        return Ok((
            to_value(&global)?,
            DefaultsTarget::Global(global),
            path.to_string_lossy().into_owned(),
        ));
    }
    let root = crate::files_correction::get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
        .ok_or_else(|| EngineError::Type("no project directory is open".into()))?;
    let project = ProjectModelDefaults::load(&root).await.map_err(failed)?;
    let path = refact_core::provider_types::project_model_defaults_path(&root);
    Ok((
        to_value(&project)?,
        DefaultsTarget::Project(root, project),
        path.to_string_lossy().into_owned(),
    ))
}

fn validate_slot(
    caps: &Arc<CodeAssistantCaps>,
    slot: &str,
    value: &ModelTypeDefaults,
) -> Result<(), EngineError> {
    let Some(model) = value.model.as_deref().filter(|m| !m.is_empty()) else {
        return Ok(());
    };
    let record = crate::caps::resolve_chat_model(caps.clone(), model)
        .map_err(|e| EngineError::Type(format!("{slot}.model: {e}")))?;
    if let Some(effort) = &value.reasoning_effort {
        let options = record.reasoning_effort_options.clone().unwrap_or_default();
        if !options.contains(effort) {
            return Err(EngineError::Type(format!(
                "{slot}.reasoning_effort {effort:?} is not offered by {model}"
            )));
        }
    }
    if let Some(budget) = value.thinking_budget {
        if !record.supports_thinking_budget
            || record.max_thinking_tokens.is_some_and(|max| budget > max)
        {
            return Err(EngineError::Type(format!(
                "{slot}.thinking_budget {budget} is not supported by {model}"
            )));
        }
    }
    Ok(())
}

/// True when `new` names a model of a different provider than the one `old` resolves to.
fn moves_provider(old: Option<&str>, new: Option<&str>) -> bool {
    let provider = |m: &str| crate::privacy::records::provider_destination(m).id.0;
    match (old.filter(|m| !m.is_empty()), new.filter(|m| !m.is_empty())) {
        (Some(old), Some(new)) => provider(old) != provider(new),
        (None, Some(_)) => true,
        _ => false,
    }
}

/// The model a slot resolves to when its own value is unset: for a project override the global
/// slot, then the caps default for that slot.
async fn effective_slot_model(
    ctx: &EngineCtx,
    caps: &Arc<CodeAssistantCaps>,
    target: &DefaultsTarget,
    slot: &str,
) -> Result<Option<String>, EngineError> {
    if let DefaultsTarget::Project(..) = target {
        let mut global = ProviderDefaults::load(&ctx.app.gcx.config_dir)
            .await
            .map_err(failed)?;
        if let Some(model) = slot_of(&mut global, slot)
            .and_then(|s| s.model.clone())
            .filter(|m| !m.is_empty())
        {
            return Ok(Some(model));
        }
    }
    let d = &caps.defaults;
    let model = match slot {
        "chat" => &d.chat_default_model,
        "chat_model_2" => &d.chat_model_2,
        "task_planner_agent_model" => &d.task_planner_agent_model,
        "chat_light" => &d.chat_light_model,
        "chat_thinking" => &d.chat_thinking_model,
        "chat_buddy" => &d.chat_buddy_model,
        _ => return Ok(None),
    };
    Ok(Some(if model.is_empty() {
        d.chat_default_model.clone()
    } else {
        model.clone()
    })
    .filter(|m| !m.is_empty()))
}

fn set_defaults_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let scope = args["scope"].as_str().unwrap_or("project");
        let (state, mut target, path) = defaults_state(ctx, scope).await?;
        let caps = caps(ctx).await?;
        let mut items = Vec::new();
        let mut provider_moves = false;
        let slots = args["slots"].as_object().cloned().unwrap_or_default();
        for (name, value) in &slots {
            if matches!(name.as_str(), "completion_model" | "embedding_model") {
                continue;
            }
            let given: Option<ModelTypeDefaults> = if value.is_null() {
                None
            } else {
                Some(
                    serde_json::from_value(value.clone())
                        .map_err(|e| EngineError::Type(format!("{name}: {e}")))?,
                )
            };
            let (old, new) = match &mut target {
                DefaultsTarget::Global(global) => {
                    let slot = slot_of(global, name)
                        .ok_or_else(|| EngineError::Type(format!("unknown slot {name}")))?;
                    let old = slot.clone();
                    let new = match &given {
                        None => ModelTypeDefaults::default(),
                        Some(g) => {
                            let mut merged = to_value(&old)?;
                            for (k, v) in to_value(g)?.as_object().into_iter().flatten() {
                                merged[k] = v.clone();
                            }
                            serde_json::from_value(merged).map_err(failed)?
                        }
                    };
                    *slot = new.clone();
                    (Some(old), Some(new))
                }
                DefaultsTarget::Project(_, project) => {
                    let slot = project_slot(project, name)
                        .ok_or_else(|| EngineError::Type(format!("unknown slot {name}")))?;
                    let old = slot.clone();
                    let new = given
                        .as_ref()
                        .map(|g| -> Result<ModelTypeDefaults, EngineError> {
                            let mut merged = to_value(old.clone().unwrap_or_default())?;
                            for (k, v) in to_value(g)?.as_object().into_iter().flatten() {
                                merged[k] = v.clone();
                            }
                            serde_json::from_value(merged).map_err(failed)
                        })
                        .transpose()?;
                    *slot = new.clone();
                    (old, new)
                }
            };
            if let Some(new) = &new {
                validate_slot(&caps, name, new)?;
            }
            let old_json = to_value(&old)?;
            let new_json = to_value(&new)?;
            for field in [
                "model",
                "max_new_tokens",
                "temperature",
                "top_p",
                "boost_reasoning",
                "reasoning_effort",
                "thinking_budget",
            ] {
                let (o, n) = (&old_json[field], &new_json[field]);
                if o != n {
                    items.push(super::mask::item(
                        &format!("{name}.{field}"),
                        (!o.is_null()).then(|| o.clone()),
                        (!n.is_null()).then(|| n.clone()),
                    ));
                }
            }
            let before = match old_json["model"].as_str().filter(|m| !m.is_empty()) {
                Some(model) => Some(model.to_string()),
                None => effective_slot_model(ctx, &caps, &target, name).await?,
            };
            provider_moves |= moves_provider(before.as_deref(), new_json["model"].as_str());
        }
        let scope_text = match &target {
            DefaultsTarget::Global(_) => "all chats (global)".to_string(),
            DefaultsTarget::Project(root, _) => format!("this project ({})", root.display()),
        };
        let plan = if items.is_empty() {
            Plan::nothing()
        } else {
            Plan::new(state, (target, path))
        };
        Ok(Preview {
            title: "change default models".into(),
            items,
            scope: Some(scope_text),
            class: if provider_moves {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning: provider_moves.then(|| {
                "New chats using this slot will send their data to a different provider."
                    .to_string()
            }),
            plan,
        })
    })
}

fn set_defaults<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let scope = args["scope"].as_str().unwrap_or("project");
        let (state, _, _) = defaults_state(ctx, scope).await?;
        let (target, path): (DefaultsTarget, String) =
            ctx.take_plan("models.set_defaults", &state)?;
        let new_state = match &target {
            DefaultsTarget::Global(global) => {
                global.save(&ctx.app.gcx.config_dir).await.map_err(failed)?;
                to_value(global)?
            }
            DefaultsTarget::Project(root, project) => {
                project.save(root).await.map_err(failed)?;
                to_value(project)?
            }
        };
        crate::providers::http::invalidate_caps(ctx.app.gcx.clone()).await;
        let mut changed = serde_json::Map::new();
        for key in new_state
            .as_object()
            .map(|m| m.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .chain(
                state
                    .as_object()
                    .map(|m| m.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
            )
        {
            if state[&key] != new_state[&key] {
                changed.insert(
                    key.clone(),
                    json!({"old":state[&key],"new":new_state[&key]}),
                );
            }
        }
        let text = changed
            .keys()
            .map(|k| format!("{k} changed"))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(EngineOutput::new(
            json!({"scope":scope,"path":path,"changed":changed}),
            format!("{text} ({scope})"),
        ))
    })
}

fn reload_preview<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let count = ctx
            .app
            .gcx
            .caps_state
            .read()
            .await
            .caps
            .as_ref()
            .map(|c| c.chat_models.len());
        Ok(Preview {
            title: "reload the model list".into(),
            items: vec![super::mask::item(
                "cached models",
                count.map(|n| json!(n)),
                Some(json!("rebuilt")),
            )],
            scope: Some("engine".into()),
            class: EngineClass::Change,
            warning: None,
            plan: super::previewed::plan(json!({})),
        })
    })
}

fn reload<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        super::previewed::take(ctx, "models.reload", &json!({}))?;
        let started = std::time::Instant::now();
        let gcx = ctx.app.gcx.clone();
        if args.get("refresh_registry").and_then(Value::as_bool) == Some(true) {
            crate::caps::model_caps::get_model_caps(gcx.clone(), true)
                .await
                .map_err(EngineError::Failed)?;
        }
        crate::providers::http::invalidate_caps(gcx.clone()).await;
        let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx, 0)
            .await
            .map_err(|e| {
                EngineError::Failed(format!(
                    "the model list could not be rebuilt: {}",
                    e.message
                ))
            })?;
        let took = started.elapsed().as_millis() as u64;
        Ok(EngineOutput::new(
            json!({"chat_models":caps.chat_models.len(),"default_chat":caps.defaults.chat_default_model,"took_ms":took}),
            format!("{} chat models loaded in {took} ms", caps.chat_models.len()),
        ))
    })
}
