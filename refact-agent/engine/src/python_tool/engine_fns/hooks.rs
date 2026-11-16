use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::common::to_value;
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput};
use crate::integrations::mcp::redaction::scrub_text;

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "hooks.list",
            summary: "Configured hooks with event, matcher, source, trust and whether they can run (hooks that will not run are listed with the reason; entries of this project's file carry the index hooks.remove and hooks.edit expect), plus recent hook activity (engine-wide, in memory, last 100; not shown to child chats). Commands are scrubbed.",
            class: EngineClass::RedactedRead,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"activity_limit":{"type":"integer","minimum":0,"maximum":100,"description":"Recent activity records; 20 by default."}}}),
            output: || Some(json!({"type":"object","required":["enabled","registered","runnable","blocked","registrations"],"properties":{"enabled":{"type":"boolean"},"max_recursion_depth":{"type":"integer"},"registered":{"type":"integer"},"runnable":{"type":"integer"},"blocked":{"type":"integer"},"denials":{"type":"integer"},"registrations":{"type":"array","items":{"type":"object","properties":{"event":{"type":"string"},"kind":{"enum":["control","observation"]},"matcher":{"type":"string"},"command":{"type":"string"},"source":{"type":"string"},"trust":{"enum":["global","trusted_project","untrusted_project","installed_plugin"]},"runs":{"type":"boolean"},"blocked_reason":{"type":"string"},"scope":{"const":"local"},"index":{"type":"integer","description":"Position in this project's hooks file, as hooks.get, hooks.remove and hooks.edit use it."}}}},"recent":{"type":"array","items":{"type":"object"}}}})),
            handler: list,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "hooks.get",
            summary: "The hooks file of one scope (global or this project) as entries with their positions; commands are scrubbed, the raw YAML is not returned.",
            class: EngineClass::RedactedRead,
            input: || json!({"type":"object","additionalProperties":false,"required":["scope"],"properties":{"scope":{"enum":["global","local"]}}}),
            output: || Some(json!({"type":"object","required":["scope","file_path","hooks"],"properties":{"scope":{"type":"string"},"file_path":{"type":"string"},"hooks":{"type":"array","items":{"type":"object","properties":{"index":{"type":"integer"},"event":{"type":"string"},"matcher":{"type":"string"},"command":{"type":"string"},"timeout":{"type":"integer"}}}}}})),
            handler: get,
            preview: None,
            timeout: None,
        },
    ]
    .into_iter()
    .chain(change_functions())
    .collect()
}

/// Hooks of one file in the order the structured writer produces (events by name, entries in
/// file order), so positions do not depend on the loader's map iteration order.
fn ordered(mut hooks: Vec<crate::ext::hooks::HookConfig>) -> Vec<crate::ext::hooks::HookConfig> {
    hooks.sort_by(|a, b| a.event.as_str().cmp(b.event.as_str()));
    hooks
}

fn scrub_field(value: &mut Value, key: &str) {
    if let Some(text) = value.get(key).and_then(Value::as_str) {
        value[key] = json!(scrub_text(text, &[]));
    }
}

fn list<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let limit = args
            .get("activity_limit")
            .and_then(Value::as_u64)
            .unwrap_or(20) as usize;
        let child = {
            let context = ctx.ccx.lock().await;
            context.background_agent_id.is_some() || context.subchat_depth > 0
        };
        use crate::http::routers::v1::ext_management::{make_scope_ext_dirs, resolve_scope_dir};
        let surface = crate::ext::hooks_runner::hook_surface(ctx.app.clone(), limit).await;
        let local = match resolve_scope_dir(ctx.app.gcx.clone(), Some("local"), true).await {
            Ok((base, scope)) => {
                let label = crate::ext::hooks_runner::source_label(
                    &crate::ext::config_dirs::source_for_dir(&base, &[], &[]),
                );
                let hooks = ordered(
                    crate::ext::hooks::load_hooks(&make_scope_ext_dirs(base, &scope)).await,
                );
                Some((label, hooks))
            }
            Err(_) => None,
        };
        let mut used = Vec::new();
        let mut registrations = Vec::new();
        for registration in &surface.registrations {
            let mut row = to_value(registration)?;
            if let Some((label, hooks)) = &local {
                if &registration.source == label {
                    let index = hooks.iter().enumerate().position(|(i, h)| {
                        !used.contains(&i)
                            && h.event.as_str() == registration.event
                            && h.matcher == registration.matcher
                            && h.command == registration.command
                    });
                    if let Some(index) = index {
                        used.push(index);
                        row["scope"] = json!("local");
                        row["index"] = json!(index);
                    }
                }
            }
            scrub_field(&mut row, "command");
            if let Some(map) = row.as_object_mut() {
                map.retain(|_, v| !v.is_null());
            }
            registrations.push(row);
        }
        let registrations = Value::Array(registrations);
        let mut data = json!({
            "enabled":surface.enabled,"max_recursion_depth":surface.max_recursion_depth,
            "registered":surface.registered,"runnable":surface.runnable,"blocked":surface.blocked,
            "denials":surface.denials,"registrations":registrations,
        });
        if !child {
            let mut recent = to_value(&surface.recent)?;
            for record in recent.as_array_mut().into_iter().flatten() {
                for key in ["decided_by", "reason"] {
                    scrub_field(record, key);
                }
                for run in record["runs"].as_array_mut().into_iter().flatten() {
                    scrub_field(run, "command");
                    scrub_field(run, "detail");
                }
            }
            let privacy = crate::privacy::load_privacy_if_needed(ctx.app.gcx.clone()).await;
            crate::ext::hooks_runner::redact_privacy_blocked_paths(&mut recent, &privacy);
            data["recent"] = recent;
        }
        let text = format!(
            "{} hooks registered, {} runnable, {} blocked; {} denials recently",
            surface.registered, surface.runnable, surface.blocked, surface.denials
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn get<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        use crate::http::routers::v1::ext_management::{make_scope_ext_dirs, resolve_scope_dir};
        let scope = args["scope"].as_str().unwrap_or("local");
        let (base, scope_name) = resolve_scope_dir(ctx.app.gcx.clone(), Some(scope), true)
            .await
            .map_err(EngineError::Type)?;
        let file_path = base.join("hooks.yaml");
        let hooks =
            ordered(crate::ext::hooks::load_hooks(&make_scope_ext_dirs(base, &scope_name)).await);
        let rows: Vec<Value> = hooks
            .iter()
            .enumerate()
            .map(|(index, hook)| {
                let mut row = json!({"index":index,"event":hook.event,"command":scrub_text(&hook.command, &[])});
                if let Some(matcher) = &hook.matcher {
                    row["matcher"] = json!(matcher);
                }
                if let Some(timeout) = hook.timeout {
                    row["timeout"] = json!(timeout);
                }
                row
            })
            .collect();
        let text = format!("{} hooks in {}", rows.len(), file_path.display());
        Ok(EngineOutput::new(
            json!({"scope":scope_name,"file_path":file_path,"hooks":rows}),
            text,
        ))
    })
}

// ---------------------------------------------------------------------------
// Changes.

use super::{Plan, Preview};
use crate::http::routers::v1::ext_management::HookConfigFlat;

pub(super) fn change_functions() -> Vec<EngineFn> {
    let output = || {
        Some(
            json!({"type":"object","required":["scope","file_path"],"properties":{"scope":{"type":"string"},"file_path":{"type":"string"},"index":{"type":"integer"},"runs":{"type":"boolean"},"blocked_reason":{"type":"string"}}}),
        )
    };
    vec![
        EngineFn {
            name: "hooks.add",
            summary: "Add a hook (a shell command run on an engine event without asking) to this project's hooks file; reports whether it will run (untrusted projects never run their hooks). Global scope is refused.",
            class: EngineClass::Risky,
            input: || json!({"type":"object","additionalProperties":false,"required":["scope","event","command"],"properties":{
                "scope":{"enum":["global","local"],"description":"Only local is allowed from scripts."},
                "event":{"type":"string","minLength":1,"description":"PreToolUse, PostToolUse, UserPromptSubmit, PermissionDecision, SessionStart, SessionEnd, Stop, SubagentStart, SubagentStop, Notification, PreCompact, PostCompact or GoalValidation."},
                "command":{"type":"string","minLength":1,"maxLength":4000},
                "matcher":{"type":"string","minLength":1,"description":"Tool-name pattern for tool events."},
                "timeout":{"type":"integer","minimum":1,"maximum":600}
            }}),
            output,
            handler: add,
            preview: Some(add_preview),
            timeout: None,
        },
        EngineFn {
            name: "hooks.remove",
            summary: "Remove one hook from this project's hooks file; expect must match the entry at index. Global scope is refused.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["scope","index","expect"],"properties":{
                "scope":{"enum":["global","local"],"description":"Only local is allowed from scripts."},
                "index":{"type":"integer","minimum":0},
                "expect":{"type":"object","additionalProperties":false,"required":["event","command"],"properties":{"event":{"type":"string"},"command":{"type":"string"}}}
            }}),
            output,
            handler: remove,
            preview: Some(remove_preview),
            timeout: None,
        },
        EngineFn {
            name: "hooks.edit",
            summary: "Change the command, matcher or timeout of one hook in this project's hooks file; expect must match the entry at index. Global scope is refused.",
            class: EngineClass::Risky,
            input: || json!({"type":"object","additionalProperties":false,"required":["scope","index","expect"],"minProperties":4,"properties":{
                "scope":{"enum":["global","local"],"description":"Only local is allowed from scripts."},
                "index":{"type":"integer","minimum":0},
                "expect":{"type":"object","additionalProperties":false,"required":["event","command"],"properties":{"event":{"type":"string"},"command":{"type":"string"}}},
                "command":{"type":"string","minLength":1,"maxLength":4000},
                "matcher":{"type":"string","minLength":1},
                "timeout":{"type":"integer","minimum":1,"maximum":600}
            }}),
            output,
            handler: edit,
            preview: Some(edit_preview),
            timeout: None,
        },
    ]
}

/// The local hooks file: base directory, path, digest of its bytes and its entries.
struct HooksFile {
    base: std::path::PathBuf,
    path: std::path::PathBuf,
    state: Value,
    hooks: Vec<HookConfigFlat>,
}

async fn hooks_file(ctx: &EngineCtx, args: &Value) -> Result<HooksFile, EngineError> {
    use crate::http::routers::v1::ext_management::{make_scope_ext_dirs, resolve_scope_dir};
    if args["scope"].as_str() != Some("local") {
        return Err(EngineError::Permission(
            "global hooks run in every project without a trust check; scripts may change only this project's hooks (scope=\"local\")".into(),
        ));
    }
    let (base, scope) = resolve_scope_dir(ctx.app.gcx.clone(), Some("local"), true)
        .await
        .map_err(EngineError::Type)?;
    let path = base.join("hooks.yaml");
    let root = base
        .parent()
        .ok_or_else(|| EngineError::Type("no project root for local scope".into()))?;
    super::common::inside_project(root, &path, &ctx.app.gcx.config_dir)?;
    let (version, raw_entries) = match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let raw = serde_yaml::from_slice::<serde_yaml::Value>(&bytes).map_err(|e| {
                EngineError::Failed(format!(
                    "{} does not parse ({e}); fix it before changing hooks",
                    path.display()
                ))
            })?;
            let entries: usize = raw
                .get("hooks")
                .and_then(serde_yaml::Value::as_mapping)
                .into_iter()
                .flatten()
                .filter_map(|(_, groups)| groups.as_sequence())
                .flatten()
                .filter_map(|group| group.get("hooks").and_then(serde_yaml::Value::as_sequence))
                .map(Vec::len)
                .sum();
            (
                Some(crate::yaml_configs::customization_service::version(&bytes)),
                entries,
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, 0),
        Err(e) => return Err(super::common::failed(e)),
    };
    let hooks: Vec<HookConfigFlat> =
        ordered(crate::ext::hooks::load_hooks(&make_scope_ext_dirs(base.clone(), &scope)).await)
            .iter()
            .map(HookConfigFlat::from)
            .collect();
    if raw_entries != hooks.len() {
        return Err(EngineError::Permission(format!(
            "{} has {} entries the engine does not load (an unknown event, a non-command type, an empty command or an invalid timeout); rewriting it would drop them, so edit it by hand",
            path.display(),
            raw_entries.saturating_sub(hooks.len())
        )));
    }
    Ok(HooksFile {
        base,
        path,
        state: json!({ "file": version }),
        hooks,
    })
}

fn event(name: &str) -> Result<crate::ext::hooks::HookEvent, EngineError> {
    serde_json::from_value(json!(name)).map_err(|_| {
        EngineError::Type(format!(
            "unknown hook event {name:?}; see hooks.add for the event names"
        ))
    })
}

fn warning(event: &crate::ext::hooks::HookEvent) -> String {
    let mut text = format!(
        "This command will run automatically on {} events without asking you.",
        event.as_str()
    );
    if event.kind() == crate::ext::hooks::HookKind::Control {
        text.push_str(" It can block or rewrite tool calls.");
    }
    text
}

/// Whether a hook of this project would run, and why not.
fn registration(
    ctx: &EngineCtx,
    file: &HooksFile,
    hook: &HookConfigFlat,
) -> (bool, Option<String>) {
    let source = crate::ext::config_dirs::source_for_dir(&file.base, &[], &[]);
    let config = crate::ext::hooks::HookConfig {
        event: hook.event.clone(),
        matcher: hook.matcher.clone(),
        command: hook.command.clone(),
        timeout: hook.timeout,
        source,
    };
    let reg = crate::ext::hooks_runner::hook_registration(&config, &ctx.app.gcx.hooks_config);
    (reg.runs, reg.blocked_reason)
}

fn will_run(runs: &(bool, Option<String>)) -> Value {
    match runs {
        (true, _) => json!("yes"),
        (false, reason) => json!(format!("no: {}", reason.clone().unwrap_or_default())),
    }
}

/// The entry at `index` after checking that it is the one the script expects.
fn expected<'a>(
    file: &'a HooksFile,
    args: &Value,
) -> Result<(usize, &'a HookConfigFlat), EngineError> {
    let index = args["index"].as_u64().unwrap_or(u64::MAX) as usize;
    let hook = file.hooks.get(index).ok_or_else(|| {
        EngineError::Type(format!(
            "no hook at index {index} ({} hooks); read them again with hooks.get",
            file.hooks.len()
        ))
    })?;
    let expect = &args["expect"];
    if expect["event"].as_str() != Some(hook.event.as_str())
        || expect["command"].as_str() != Some(hook.command.as_str())
    {
        return Err(EngineError::Type(format!(
            "the hook at index {index} is not the expected one; read them again with hooks.get"
        )));
    }
    Ok((index, hook))
}

async fn write(file: &HooksFile, hooks: &[HookConfigFlat]) -> Result<(), EngineError> {
    let content = crate::http::routers::v1::ext_management::hooks_to_yaml_string(hooks)
        .map_err(EngineError::Failed)?;
    tokio::fs::create_dir_all(&file.base)
        .await
        .map_err(super::common::failed)?;
    crate::http::routers::v1::ext_management::write_file_atomic(&file.path, &content)
        .await
        .map_err(|e| EngineError::Failed(format!("write error: {e}")))?;
    crate::ext::hooks_runner::invalidate_hooks_cache().await;
    Ok(())
}

fn output(
    ctx: &EngineCtx,
    file: &HooksFile,
    index: Option<usize>,
    hook: Option<&HookConfigFlat>,
    verb: &str,
) -> EngineOutput {
    let mut data = json!({"scope":"local","file_path":file.path});
    let mut text = format!("hook {verb}");
    if let Some(index) = index {
        data["index"] = json!(index);
    }
    if let Some(hook) = hook {
        let (runs, reason) = registration(ctx, file, hook);
        data["runs"] = json!(runs);
        match &reason {
            Some(reason) => {
                data["blocked_reason"] = json!(reason);
                text.push_str(&format!("; it will not run because {reason}"));
            }
            None => text.push_str("; it will run"),
        }
    }
    EngineOutput::new(data, text)
}

fn new_hook(args: &Value) -> Result<HookConfigFlat, EngineError> {
    Ok(HookConfigFlat {
        event: event(args["event"].as_str().unwrap_or_default())?,
        matcher: args
            .get("matcher")
            .and_then(Value::as_str)
            .map(String::from),
        command: args["command"].as_str().unwrap_or_default().to_string(),
        timeout: args.get("timeout").and_then(Value::as_u64),
    })
}

fn add_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let file = hooks_file(ctx, args).await?;
        let hook = new_hook(args)?;
        if hook.command.trim().is_empty() {
            return Err(EngineError::Type("hook command cannot be empty".into()));
        }
        let mut items = vec![super::mask::item(
            "event",
            None,
            Some(json!(hook.event.as_str())),
        )];
        if let Some(matcher) = &hook.matcher {
            items.push(super::mask::item("matcher", None, Some(json!(matcher))));
        }
        items.push(super::mask::item(
            "command",
            None,
            Some(json!(hook.command)),
        ));
        if let Some(timeout) = hook.timeout {
            items.push(super::mask::item("timeout", None, Some(json!(timeout))));
        }
        items.push(super::mask::item(
            "will run",
            None,
            Some(will_run(&registration(ctx, &file, &hook))),
        ));
        let mut hooks = file.hooks.clone();
        hooks.push(hook.clone());
        Ok(Preview {
            title: "add a hook".into(),
            items,
            scope: Some("this project".into()),
            class: EngineClass::Risky,
            warning: Some(warning(&hook.event)),
            plan: Plan::new(file.state, hooks),
        })
    })
}

fn add<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let file = hooks_file(ctx, args).await?;
        let hooks: Vec<HookConfigFlat> = ctx.take_plan("hooks.add", &file.state)?;
        write(&file, &hooks).await?;
        let written = hooks_file(ctx, args).await?;
        let index = written.hooks.iter().rposition(|h| {
            hooks.last().is_some_and(|l| {
                h.event == l.event && h.command == l.command && h.matcher == l.matcher
            })
        });
        Ok(output(ctx, &written, index, hooks.last(), "added"))
    })
}

fn remove_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let file = hooks_file(ctx, args).await?;
        let (index, hook) = expected(&file, args)?;
        let mut text = "The hook definition will be deleted; the preview shows its command if you need to add it back.".to_string();
        if matches!(
            hook.event,
            crate::ext::hooks::HookEvent::PreToolUse
                | crate::ext::hooks::HookEvent::PermissionDecision
        ) {
            text.push_str(" It may have been blocking tool calls.");
        }
        let mut items = vec![super::mask::named_item(
            hook.event.as_str(),
            Some(json!(hook.command)),
            Some(json!("removed")),
        )];
        if let Some(matcher) = &hook.matcher {
            items.push(super::mask::item("matcher", Some(json!(matcher)), None));
        }
        let mut hooks = file.hooks.clone();
        hooks.remove(index);
        Ok(Preview {
            title: "remove a hook".into(),
            items,
            scope: Some("this project".into()),
            class: EngineClass::Destructive,
            warning: Some(text),
            plan: Plan::new(file.state, hooks),
        })
    })
}

fn remove<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let file = hooks_file(ctx, args).await?;
        let hooks: Vec<HookConfigFlat> = ctx.take_plan("hooks.remove", &file.state)?;
        write(&file, &hooks).await?;
        Ok(output(ctx, &file, None, None, "removed"))
    })
}

fn edit_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let file = hooks_file(ctx, args).await?;
        let (index, hook) = expected(&file, args)?;
        let mut new = hook.clone();
        if let Some(command) = args.get("command").and_then(Value::as_str) {
            if command.trim().is_empty() {
                return Err(EngineError::Type("hook command cannot be empty".into()));
            }
            new.command = command.to_string();
        }
        if let Some(matcher) = args.get("matcher").and_then(Value::as_str) {
            new.matcher = Some(matcher.to_string());
        }
        if let Some(timeout) = args.get("timeout").and_then(Value::as_u64) {
            new.timeout = Some(timeout);
        }
        let mut items = Vec::new();
        if new.command != hook.command {
            items.push(super::mask::item(
                "command",
                Some(json!(hook.command)),
                Some(json!(new.command)),
            ));
        }
        if new.matcher != hook.matcher {
            items.push(super::mask::item(
                "matcher",
                hook.matcher.clone().map(Value::from),
                new.matcher.clone().map(Value::from),
            ));
        }
        if new.timeout != hook.timeout {
            items.push(super::mask::item(
                "timeout",
                hook.timeout.map(Value::from),
                new.timeout.map(Value::from),
            ));
        }
        let plan = if items.is_empty() {
            Plan::nothing()
        } else {
            items.push(super::mask::item(
                "will run",
                None,
                Some(will_run(&registration(ctx, &file, &new))),
            ));
            let mut hooks = file.hooks.clone();
            hooks[index] = new.clone();
            Plan::new(file.state.clone(), (hooks, index))
        };
        Ok(Preview {
            title: "change a hook".into(),
            items,
            scope: Some("this project".into()),
            class: EngineClass::Risky,
            warning: Some(warning(&new.event)),
            plan,
        })
    })
}

fn edit<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let file = hooks_file(ctx, args).await?;
        let (hooks, index): (Vec<HookConfigFlat>, usize) =
            ctx.take_plan("hooks.edit", &file.state)?;
        write(&file, &hooks).await?;
        let written = hooks_file(ctx, args).await?;
        let hook = &hooks[index];
        let position = written.hooks.iter().position(|h| {
            h.event == hook.event && h.command == hook.command && h.matcher == hook.matcher
        });
        Ok(output(ctx, &written, position, Some(hook), "changed"))
    })
}
