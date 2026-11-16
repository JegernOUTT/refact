use std::path::PathBuf;

use futures::future::BoxFuture;
use refact_privacy::{Destination, DestinationId, DestinationKind, FileRecord, PrivacyAuditError};
use serde_json::{json, Value};

use super::common::to_value;
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput};
use crate::integrations::mcp::redaction::scrub_text;

fn destination_arg() -> Value {
    json!({"type":"string","minLength":1,"description":"A model id like openai/gpt-4.1 (its provider) or mcp:<server>; this chat's model by default."})
}

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "privacy.status",
            summary: "Whether the privacy rules loaded cleanly, which files they come from, and whether shell read observation works on this machine.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["platform","observation","source_paths","has_project_overrides"],"properties":{"platform":{"type":"string"},"observation":{"type":"object","properties":{"platform_supported":{"type":"boolean"},"runtime_available":{"type":"boolean"},"last_error":{"type":"string"}}},"config_error":{"type":"string"},"source_paths":{"type":"array","items":{"type":"string"}},"has_project_overrides":{"type":"boolean"}}})),
            handler: status,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "privacy.policy",
            summary: "The privacy policy (effective global+project by default, or the global file only): blocked patterns, zones with allowed destinations, tool access and known destinations.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{
                "effective":{"type":"boolean","description":"Merged global and project policy; true by default. false: the global file only."},
                "match_counts":{"type":"boolean","description":"Count workspace files per zone; false by default."}
            }}),
            output: || Some(json!({"type":"object","required":["policy","destinations","has_project_overrides","source_paths"],"properties":{"policy":{"type":"object"},"destinations":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"kind":{"type":"string"},"display_name":{"type":"string"}}}},"match_counts":{"type":"object"},"has_project_overrides":{"type":"boolean"},"source_paths":{"type":"array"}}})),
            handler: policy,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "privacy.check",
            summary: "Which privacy zone each path is in, and whether a destination (model provider or MCP server) may receive it; includes this chat's derived zones.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["paths"],"properties":{
                "paths":{"type":"array","minItems":1,"maxItems":200,"items":{"type":"string","minLength":1},"description":"Absolute or workspace-relative paths."},
                "destination":destination_arg()
            }}),
            output: || Some(json!({"type":"object","required":["destination","paths"],"properties":{"destination":{"type":"object","properties":{"id":{"type":"string"},"kind":{"type":"string"}}},"paths":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"},"zone":{"type":"string"},"send_to":{"type":"array"},"sendable":{"type":"boolean"},"derived_in_this_chat":{"type":"string"}}}}}})),
            handler: check,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "privacy.chat",
            summary: "The privacy records and derived zones a chat carries (this chat by default, or one of its tree from a root chat), and whether its content may go to a destination. Never returns messages.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{
                "chat_id":{"type":"string","minLength":1,"description":"This chat by default."},
                "destination":destination_arg()
            }}),
            output: || Some(json!({"type":"object","required":["chat_id","records","derived"],"properties":{"chat_id":{"type":"string"},"records":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"},"zone":{"type":"string"},"attribution":{"type":"string"}}}},"derived":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"},"zone":{"type":"string"},"origin":{"type":"string"}}}},"sendable":{"type":"boolean"},"blocked":{"type":"array"},"refusal":{"type":"string"}}})),
            handler: chat,
            preview: None,
            timeout: None,
        },
    ]
    .into_iter()
    .chain(change_functions())
    .collect()
}

fn source_paths(
    gcx: &crate::global_context::GlobalContext,
) -> (Vec<String>, Option<String>, refact_privacy::PrivacyPolicy) {
    let load = gcx
        .privacy_policy_load
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    (
        load.source_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        load.error.clone(),
        load.policy.as_ref().clone(),
    )
}

fn status<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let gcx = ctx.app.gcx.clone();
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let (paths, error, effective) = source_paths(&gcx);
        let runtime = gcx
            .privacy_observation_runtime
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let observation = to_value(crate::http::routers::v1::privacy::observation_capability(
            runtime,
        ))?;
        let global = crate::privacy::global_only_policy(gcx.clone()).await;
        let overrides = global.is_some_and(|g| g != effective);
        let mut data = json!({"platform":std::env::consts::OS,"observation":observation,"source_paths":paths,"has_project_overrides":overrides});
        if let Some(map) = data["observation"].as_object_mut() {
            map.retain(|_, v| !v.is_null());
        }
        if let Some(error) = &error {
            data["config_error"] = json!(scrub_text(error, &[]));
        }
        let available = data["observation"]["runtime_available"] == true;
        let text = format!(
            "privacy rules {} from {} files; shell observation {}",
            if error.is_some() {
                "failed to load (previous policy in force)"
            } else {
                "OK"
            },
            paths.len(),
            if available {
                "available"
            } else {
                "unavailable"
            }
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn policy<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let gcx = ctx.app.gcx.clone();
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let (paths, _, effective) = source_paths(&gcx);
        let global = crate::privacy::global_only_policy(gcx.clone()).await;
        let overrides = global.as_ref().is_some_and(|g| *g != effective);
        let shown = if args.get("effective").and_then(Value::as_bool) == Some(false) {
            global.unwrap_or_else(|| effective.clone())
        } else {
            effective.clone()
        };
        let destinations =
            crate::http::routers::v1::privacy::collect_destinations(&ctx.app, &effective).await;
        let mut data = json!({"policy":to_value(&shown)?,"destinations":to_value(&destinations)?,"has_project_overrides":overrides,"source_paths":paths});
        if args.get("match_counts").and_then(Value::as_bool) == Some(true) {
            let (app, policy) = (ctx.app.clone(), effective.clone());
            let counts = tokio::task::spawn_blocking(move || {
                crate::http::routers::v1::privacy::live_match_counts(&app, &policy)
            })
            .await
            .map_err(|e| EngineError::Failed(e.to_string()))?
            .map_err(|e| EngineError::Failed(e.message))?;
            data["match_counts"] = to_value(counts)?;
        }
        let text = format!(
            "{} blocked patterns, {} zones, {} destinations",
            shown.blocked.len(),
            shown.zones.len(),
            destinations.len()
        );
        Ok(EngineOutput::new(data, text))
    })
}

async fn destination(ctx: &EngineCtx, args: &Value) -> Destination {
    match args.get("destination").and_then(Value::as_str) {
        Some(id) => match id.strip_prefix("mcp:") {
            Some(server) => Destination {
                id: DestinationId(server.to_string()),
                kind: DestinationKind::Mcp,
                display_name: server.to_string(),
            },
            None => crate::privacy::records::provider_destination(id),
        },
        None => crate::privacy::records::provider_destination(&ctx.ccx.lock().await.current_model),
    }
}

fn check<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let gcx = ctx.app.gcx.clone();
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let destination = destination(ctx, args).await;
        let roots = crate::files_correction::get_project_dirs(gcx.clone()).await;
        let requested: Vec<String> = args["paths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect();
        let absolute: Vec<PathBuf> = requested
            .iter()
            .map(|p| {
                let path = PathBuf::from(p);
                if path.is_absolute() {
                    path
                } else {
                    roots.first().map(|r| r.join(&path)).unwrap_or(path)
                }
            })
            .collect();
        let derived = ctx.ccx.lock().await.derived_privacy_zones.clone();
        let mut zones = None;
        for _ in 0..2 {
            let snapshot = crate::privacy::classification_cache::snapshot(&gcx).map_err(|e| {
                EngineError::Failed(format!(
                    "the privacy policy could not be loaded ({e}); nothing was returned"
                ))
            })?;
            let classified: Vec<refact_privacy::Zone> = absolute
                .iter()
                .map(|p| snapshot.classify(&gcx, p).zone)
                .collect();
            match snapshot.finish(&gcx) {
                Ok(()) => {
                    zones = Some(classified);
                    break;
                }
                Err(e) if e == refact_privacy::gate::PRIVACY_CONTEXT_CHANGED => continue,
                Err(e) => return Err(EngineError::Failed(e)),
            }
        }
        let zones = zones.ok_or_else(|| {
            EngineError::Failed("the privacy rules changed twice while checking; try again".into())
        })?;
        let derived = derived.read().unwrap_or_else(|e| e.into_inner()).clone();
        let (_, _, policy) = source_paths(&gcx);
        let mut blocked = 0;
        let mut sendable_count = 0;
        let rows: Vec<Value> = requested
            .iter()
            .zip(&absolute)
            .zip(&zones)
            .map(|((shown, path), zone)| {
                let mut sendable =
                    zone.name != "blocked" && destination.matches_send_to(&zone.send_to);
                let mut row = json!({"path":shown,"zone":zone.name,"send_to":zone.send_to});
                if let Some(d) = derived.get(path) {
                    row["derived_in_this_chat"] = json!(d.zone);
                    let derived_sendable = d.zone == "normal"
                        || policy
                            .zones
                            .iter()
                            .any(|z| z.name == d.zone && destination.matches_send_to(&z.send_to));
                    sendable &= derived_sendable;
                }
                blocked += usize::from(zone.name == "blocked");
                sendable_count += usize::from(sendable);
                row["sendable"] = json!(sendable);
                row
            })
            .collect();
        let text = format!(
            "{} paths: {blocked} blocked, {sendable_count} sendable to {}",
            rows.len(),
            destination.id.0
        );
        Ok(EngineOutput::new(
            json!({"destination":{"id":destination.id.0,"kind":destination.kind},"paths":rows}),
            text,
        ))
    })
}

struct Audited(Result<Vec<(usize, FileRecord)>, PrivacyAuditError>);
impl refact_privacy::PrivacyAudited for Audited {
    fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
        self.0.clone()
    }
}

fn chat<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let chat_id = args
            .get("chat_id")
            .and_then(Value::as_str)
            .unwrap_or(&ctx.chat_id)
            .to_string();
        if chat_id != ctx.chat_id {
            super::common::require_root_chat(ctx, "privacy.chat for another chat").await?;
            let root = {
                let context = ctx.ccx.lock().await;
                if context.root_chat_id.is_empty() {
                    ctx.chat_id.clone()
                } else {
                    context.root_chat_id.clone()
                }
            };
            let session = ctx.app.chat.sessions.read().await.get(&chat_id).cloned();
            let in_tree = match &session {
                Some(s) => {
                    let s = s.lock().await;
                    s.chat_id == root || s.thread.root_chat_id.as_deref() == Some(root.as_str())
                }
                None => false,
            };
            if !in_tree {
                return Err(EngineError::Permission(format!(
                    "chat {chat_id} is not a loaded chat of this chat's tree"
                )));
            }
        }
        let gcx = ctx.app.gcx.clone();
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let session = ctx
            .app
            .chat
            .sessions
            .read()
            .await
            .get(&chat_id)
            .cloned()
            .ok_or_else(|| EngineError::Failed(format!("chat {chat_id} has no live session")))?;
        let (indexed, derived) = {
            let s = session.lock().await;
            (
                refact_privacy::records_from_messages(&s.messages),
                s.derived_privacy_zones.clone(),
            )
        };
        let records: Vec<FileRecord> = indexed
            .as_ref()
            .map_err(|e| {
                EngineError::Failed(format!(
                    "the chat's privacy records could not be read: {}",
                    e.message
                ))
            })?
            .iter()
            .map(|(_, r)| r.clone())
            .collect();
        let mut derived: Vec<Value> = derived
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(path, d)| {
                let mut row = json!({"path":path,"zone":d.zone});
                if let Some(origin) = &d.origin {
                    row["origin"] = json!(origin);
                }
                row
            })
            .collect();
        derived.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        let restricted = records.iter().filter(|r| r.zone != "normal").count();
        let mut data = json!({"chat_id":chat_id,"records":to_value(&records)?,"derived":derived});
        let mut text = format!(
            "{} records ({restricted} in restricted zones)",
            records.len()
        );
        if args.get("destination").is_some() {
            let destination = destination(ctx, args).await;
            match crate::privacy::destinations::clear_current(&gcx, Audited(indexed), &destination)
            {
                Ok(_) => {
                    data["sendable"] = json!(true);
                    data["blocked"] = json!([]);
                }
                Err(refusal) => {
                    data["sendable"] = json!(false);
                    data["blocked"] = json!(refusal
                        .offending
                        .iter()
                        .map(|(i, r)| json!({"record_index":i,"path":r.path,"zone":r.zone}))
                        .collect::<Vec<_>>());
                    data["refusal"] = json!(refusal.message);
                }
            }
            text.push_str(&format!(
                "; sendable to {}: {}",
                destination.id.0,
                if data["sendable"] == true {
                    "yes"
                } else {
                    "no"
                }
            ));
        }
        Ok(EngineOutput::new(data, text))
    })
}

// ---------------------------------------------------------------------------
// Changes.

use super::{Plan, Preview};

pub(super) fn change_functions() -> Vec<EngineFn> {
    vec![relax_function(), EngineFn {
        name: "privacy.tighten",
        summary: "Make the global privacy rules stricter only: add blocked patterns, add patterns to a zone, remove destinations from a zone, add a zone, or narrow MCP access. Applies live.",
        class: EngineClass::Change,
        input: || {
            let patterns = json!({"type":"array","minItems":1,"items":{"type":"string","minLength":1}});
            json!({"type":"object","additionalProperties":false,"properties":{
                "block":patterns,
                "zone_add_patterns":{"type":"object","additionalProperties":patterns},
                "zone_remove_destinations":{"type":"object","additionalProperties":patterns},
                "new_zone":{"type":"object","additionalProperties":false,"required":["name","patterns","send_to"],"properties":{"name":{"type":"string","minLength":1},"patterns":patterns,"send_to":{"type":"array","items":{"type":"string","minLength":1}},"on_shell_read":{"enum":["withhold","ask","deny"]}}},
                "restrict_mcp":{"type":"object","additionalProperties":{"type":"array","items":{"type":"string","minLength":1}},"description":"provider: allowed MCP servers (intersected with the current list)."},
                "scope":{"const":"global"}
            }})
        },
        output: || Some(json!({"type":"object","required":["path","changed","effective_reloaded"],"properties":{"path":{"type":"string"},"changed":{"type":"array"},"effective_reloaded":{"type":"boolean"}}})),
        handler: tighten,
        preview: Some(tighten_preview),
        timeout: None,
    }]
}

fn bad_pattern(pattern: &str) -> bool {
    let normalized = pattern.replace('\\', "/");
    let literal = normalized.trim_matches(|c: char| c == '/' || c.is_whitespace());
    literal.is_empty()
        || literal == "."
        || (literal.len() == 2
            && literal.as_bytes()[0].is_ascii_alphabetic()
            && literal.ends_with(':'))
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(String::from))
        .collect()
}

/// Zones match first-hit in order; a path no zone matches falls to `normal` (the explicit zone
/// of that name wherever it stands, else an implicit zone sending everywhere). Adding patterns to
/// the zone at `position` moves paths from every later zone and from `normal` into it, so it must
/// be at least as strict as each of them: its destinations a subset of theirs and its shell
/// behaviour no laxer. Returns the name of the first zone that breaks this.
fn wider_than_later(policy: &refact_privacy::PrivacyPolicy, position: usize) -> Option<String> {
    fn shell_rank(b: refact_privacy::ShellBehavior) -> u8 {
        match b {
            refact_privacy::ShellBehavior::Ask => 0,
            refact_privacy::ShellBehavior::Withhold => 1,
            refact_privacy::ShellBehavior::Deny => 2,
        }
    }
    let zone = &policy.zones[position];
    let implicit_normal = refact_privacy::Zone {
        name: "normal".into(),
        send_to: vec!["*".into()],
        ..Default::default()
    };
    let normal = policy
        .zones
        .iter()
        .find(|z| z.name == "normal")
        .unwrap_or(&implicit_normal);
    policy.zones[position + 1..]
        .iter()
        .chain(std::iter::once(normal))
        .filter(|z| !std::ptr::eq(*z, zone))
        .find(|z| {
            let sends = z.send_to.iter().any(|d| d == "*")
                || zone.send_to.iter().all(|d| z.send_to.contains(d));
            !sends || shell_rank(zone.on_shell_read) < shell_rank(z.on_shell_read)
        })
        .map(|z| z.name.clone())
}

/// The stricter policy and the items it shows; refuses anything that would relax the input.
fn tightened(
    old: &refact_privacy::PrivacyPolicy,
    args: &Value,
) -> Result<
    (
        refact_privacy::PrivacyPolicy,
        Vec<refact_chat_api::PythonEngineItem>,
    ),
    EngineError,
> {
    let mut new = old.clone();
    let mut items = Vec::new();
    for pattern in strings(&args["block"]) {
        if bad_pattern(&pattern) {
            return Err(EngineError::Type(format!(
                "pattern {pattern:?} is empty, dot or a drive root; use ** to mean everything"
            )));
        }
        if !new.blocked.contains(&pattern) {
            items.push(super::mask::item("blocked", None, Some(json!(pattern))));
            new.blocked.push(pattern);
        }
    }
    for (zone, patterns) in args["zone_add_patterns"].as_object().into_iter().flatten() {
        let position = new
            .zones
            .iter()
            .position(|z| &z.name == zone)
            .ok_or_else(|| EngineError::Type(format!("no zone {zone}")))?;
        if let Some(later) = wider_than_later(&new, position) {
            return Err(EngineError::Permission(format!(
                "zone {zone} may send to destinations that zone {later} after it may not; zones match first-hit, so adding patterns to it could relax the rules"
            )));
        }
        let target = &mut new.zones[position];
        for pattern in strings(patterns) {
            if bad_pattern(&pattern) {
                return Err(EngineError::Type(format!(
                    "pattern {pattern:?} is empty, dot or a drive root"
                )));
            }
            if !target.patterns.contains(&pattern) {
                items.push(super::mask::named_item(
                    &format!("{zone}.patterns"),
                    None,
                    Some(json!(pattern)),
                ));
                target.patterns.push(pattern);
            }
        }
    }
    for (zone, destinations) in args["zone_remove_destinations"]
        .as_object()
        .into_iter()
        .flatten()
    {
        let target = new
            .zones
            .iter_mut()
            .find(|z| &z.name == zone)
            .ok_or_else(|| EngineError::Type(format!("no zone {zone}")))?;
        for destination in strings(destinations) {
            if target.send_to.contains(&destination) {
                items.push(super::mask::named_item(
                    &format!("{zone}.send_to"),
                    Some(json!(destination)),
                    None,
                ));
                target.send_to.retain(|d| *d != destination);
            }
        }
    }
    if let Some(zone) = args.get("new_zone").filter(|v| v.is_object()) {
        let name = zone["name"].as_str().unwrap_or_default().to_string();
        if new.zones.iter().any(|z| z.name == name) || matches!(name.as_str(), "normal" | "blocked")
        {
            return Err(EngineError::Type(format!("zone {name} already exists")));
        }
        let send_to = strings(&zone["send_to"]);
        if send_to.iter().any(|d| d == "*") {
            return Err(EngineError::Type(
                "a new zone cannot send to every destination (\"*\")".into(),
            ));
        }
        let patterns = strings(&zone["patterns"]);
        if let Some(bad) = patterns.iter().find(|p| bad_pattern(p)) {
            return Err(EngineError::Type(format!(
                "pattern {bad:?} is empty, dot or a drive root"
            )));
        }
        let on_shell_read: refact_privacy::ShellBehavior = serde_json::from_value(
            zone.get("on_shell_read")
                .cloned()
                .unwrap_or(json!("withhold")),
        )
        .map_err(|e| EngineError::Type(e.to_string()))?;
        items.push(super::mask::item(
            "zone",
            None,
            Some(json!({"name":name,"patterns":patterns,"send_to":send_to,"position":"first"})),
        ));
        new.zones.insert(
            0,
            refact_privacy::Zone {
                name,
                patterns,
                send_to,
                on_shell_read,
            },
        );
        if let Some(later) = wider_than_later(&new, 0) {
            return Err(EngineError::Permission(format!(
                "the new zone may send to destinations that zone {later} may not; it goes first, so it could relax the rules"
            )));
        }
    }
    for (provider, servers) in args["restrict_mcp"].as_object().into_iter().flatten() {
        let wanted = strings(servers);
        let current = new
            .tool_access
            .providers
            .get(provider)
            .map(|a| a.mcp.clone())
            .unwrap_or_else(|| vec!["*".into()]);
        let narrowed: Vec<String> = if current.iter().any(|s| s == "*") {
            wanted.clone()
        } else {
            current
                .iter()
                .filter(|s| wanted.contains(s))
                .cloned()
                .collect()
        };
        if narrowed != current {
            items.push(super::mask::named_item(
                &format!("{provider}.mcp"),
                Some(json!(current)),
                Some(json!(narrowed)),
            ));
            new.tool_access.providers.insert(
                provider.clone(),
                refact_privacy::ProviderToolAccess {
                    mcp: narrowed,
                },
            );
        }
    }
    new.compile()
        .map_err(|e| EngineError::Type(format!("invalid privacy policy: {e}")))?;
    let stricter = old.blocked.iter().all(|p| new.blocked.contains(p))
        && old.zones.iter().all(|z| {
            new.zones.iter().any(|n| {
                n.name == z.name
                    && z.patterns.iter().all(|p| n.patterns.contains(p))
                    && n.send_to.iter().all(|d| z.send_to.contains(d))
            })
        })
        && (new.subagents.report_declassifies <= old.subagents.report_declassifies);
    if !stricter {
        return Err(EngineError::Permission(
            "this would relax the privacy rules; privacy.tighten can only make them stricter"
                .into(),
        ));
    }
    Ok((new, items))
}

/// The global privacy rules alone; a global file that exists but does not parse is an error,
/// so a change is never computed from defaults that would replace it.
async fn global_policy(ctx: &EngineCtx) -> Result<refact_privacy::PrivacyPolicy, EngineError> {
    crate::privacy::load_privacy_if_needed(ctx.app.gcx.clone()).await;
    if let Some(policy) = crate::privacy::global_only_policy(ctx.app.gcx.clone()).await {
        return Ok(policy);
    }
    let (paths, _, _) = source_paths(&ctx.app.gcx);
    match paths.first().map(PathBuf::from).filter(|p| p.exists()) {
        Some(path) => Err(EngineError::Failed(format!(
            "the global privacy rules in {} do not parse; fix them first",
            path.display()
        ))),
        None => Ok(refact_privacy::PrivacyPolicy::default()),
    }
}

fn tighten_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let given = [
            "block",
            "zone_add_patterns",
            "zone_remove_destinations",
            "new_zone",
            "restrict_mcp",
        ]
        .iter()
        .any(|k| args.get(*k).is_some());
        if !given {
            return Err(EngineError::Type("give at least one of block, zone_add_patterns, zone_remove_destinations, new_zone, restrict_mcp".into()));
        }
        let old = global_policy(ctx).await?;
        let (new, items) = tightened(&old, args)?;
        let plan = if items.is_empty() {
            Plan::nothing()
        } else {
            Plan::new(to_value(&old)?, new)
        };
        Ok(Preview {
            title: "tighten privacy rules".into(),
            items,
            scope: Some("all chats and projects (global privacy.yaml)".into()),
            class: EngineClass::Change,
            warning: None,
            plan,
        })
    })
}

fn tighten<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let old = global_policy(ctx).await?;
        let new: refact_privacy::PrivacyPolicy =
            ctx.take_plan("privacy.tighten", &to_value(&old)?)?;
        crate::privacy::save_privacy_policy(ctx.app.gcx.clone(), new.clone())
            .await
            .map_err(EngineError::Failed)?;
        let (paths, error, _) = source_paths(&ctx.app.gcx);
        let path = paths.first().cloned().unwrap_or_default();
        let old_json = to_value(&old)?;
        let new_json = to_value(&new)?;
        let changed: Vec<Value> = ["blocked", "zones", "tool_access"]
            .iter()
            .filter(|k| old_json[**k] != new_json[**k])
            .map(|k| json!({"field":k,"old":old_json[*k],"new":new_json[*k]}))
            .collect();
        Ok(EngineOutput::new(
            json!({"path":path,"changed":changed,"effective_reloaded":error.is_none()}),
            "privacy rules tightened; chats whose content now falls in a stricter zone may be refused on their next request",
        ))
    })
}

fn relax_function() -> EngineFn {
    EngineFn {
        name: "privacy.relax",
        summary: "Relax the global privacy rules: unblock patterns, remove zone patterns, add destinations to a zone, widen MCP access or let subagent reports declassify. The card shows the effective change for this project (project rules can still restrict). Applies live.",
        class: EngineClass::Risky,
        input: || {
            let patterns = json!({"type":"array","minItems":1,"items":{"type":"string","minLength":1}});
            json!({"type":"object","additionalProperties":false,"minProperties":1,"properties":{
                "unblock":patterns,
                "zone_remove_patterns":{"type":"object","minProperties":1,"additionalProperties":patterns},
                "zone_add_destinations":{"type":"object","minProperties":1,"additionalProperties":patterns,"description":"zone: destination ids from privacy.policy destinations; \"*\" is refused."},
                "allow_mcp":{"type":"object","minProperties":1,"additionalProperties":patterns,"description":"provider: MCP servers to allow; \"*\" is refused."},
                "report_declassifies":{"const":true}
            }})
        },
        output: || Some(json!({"type":"object","required":["path","changed","effective_reloaded"],"properties":{"path":{"type":"string"},"changed":{"type":"array"},"effective_reloaded":{"type":"boolean"}}})),
        handler: relax,
        preview: Some(relax_preview),
        timeout: None,
    }
}

/// The global policy relaxed as asked, with one item per removed pattern or added destination.
fn relaxed(
    old: &refact_privacy::PrivacyPolicy,
    args: &Value,
) -> Result<
    (
        refact_privacy::PrivacyPolicy,
        Vec<refact_chat_api::PythonEngineItem>,
    ),
    EngineError,
> {
    let mut new = old.clone();
    let mut items = Vec::new();
    for pattern in strings(&args["unblock"]) {
        if !new.blocked.contains(&pattern) {
            return Err(EngineError::Type(format!(
                "{pattern:?} is not a blocked pattern of the global rules"
            )));
        }
        new.blocked.retain(|p| *p != pattern);
        items.push(super::mask::item("blocked", Some(json!(pattern)), None));
    }
    for (zone, patterns) in args["zone_remove_patterns"]
        .as_object()
        .into_iter()
        .flatten()
    {
        let target = new
            .zones
            .iter_mut()
            .find(|z| &z.name == zone)
            .ok_or_else(|| EngineError::Type(format!("no zone {zone}")))?;
        for pattern in strings(patterns) {
            if !target.patterns.contains(&pattern) {
                return Err(EngineError::Type(format!(
                    "zone {zone} has no pattern {pattern:?}"
                )));
            }
            target.patterns.retain(|p| *p != pattern);
            items.push(super::mask::named_item(
                &format!("{zone}.patterns"),
                Some(json!(pattern)),
                None,
            ));
        }
        if target.patterns.is_empty() && target.name != "normal" {
            return Err(EngineError::Permission(format!(
                "this would empty zone {zone}; deleting a whole zone is for you to do in the privacy settings"
            )));
        }
    }
    for (zone, destinations) in args["zone_add_destinations"]
        .as_object()
        .into_iter()
        .flatten()
    {
        let target = new
            .zones
            .iter_mut()
            .find(|z| &z.name == zone)
            .ok_or_else(|| EngineError::Type(format!("no zone {zone}")))?;
        for destination in strings(destinations) {
            if destination.trim() == "*" {
                return Err(EngineError::Permission(
                    "a zone cannot be opened to every destination (\"*\") from a script".into(),
                ));
            }
            if !target.send_to.contains(&destination) {
                items.push(super::mask::named_item(
                    &format!("{zone}.send_to"),
                    None,
                    Some(json!(destination)),
                ));
                target.send_to.push(destination);
            }
        }
    }
    for (provider, servers) in args["allow_mcp"].as_object().into_iter().flatten() {
        let wanted = strings(servers);
        if wanted.iter().any(|s| s.trim() == "*") {
            return Err(EngineError::Permission(
                "MCP access cannot be opened to every server (\"*\") from a script".into(),
            ));
        }
        let Some(access) = new.tool_access.providers.get_mut(provider) else {
            continue;
        };
        if access.mcp.iter().any(|s| s == "*") {
            continue;
        }
        for server in wanted {
            if !access.mcp.contains(&server) {
                items.push(super::mask::named_item(
                    &format!("{provider}.mcp"),
                    None,
                    Some(json!(server)),
                ));
                access.mcp.push(server);
            }
        }
    }
    if args.get("report_declassifies").and_then(Value::as_bool) == Some(true)
        && !new.subagents.report_declassifies
    {
        items.push(super::mask::item(
            "subagents.report_declassifies",
            Some(json!(false)),
            Some(json!(true)),
        ));
        new.subagents.report_declassifies = true;
    }
    let unrestricted = |p: &refact_privacy::PrivacyPolicy| {
        p.blocked.is_empty() && p.zones.iter().all(|z| z.send_to.iter().any(|d| d == "*"))
    };
    if unrestricted(&new) && !unrestricted(old) {
        return Err(EngineError::Permission(
            "this would leave no privacy restriction at all; resetting the rules is for you to do in the privacy settings"
                .into(),
        ));
    }
    new.compile()
        .map_err(|e| EngineError::Type(format!("invalid privacy policy: {e}")))?;
    Ok((new, items))
}

fn project_policies(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .map(|r| r.join(".refact").join("privacy.yaml"))
        .collect()
}

/// The global policy merged with this project's privacy files, as the engine loads it.
async fn effective_with(
    ctx: &EngineCtx,
    global: &refact_privacy::PrivacyPolicy,
) -> Result<refact_privacy::PrivacyPolicy, EngineError> {
    let roots = crate::files_correction::get_project_dirs(ctx.app.gcx.clone()).await;
    let mut merged = global.clone();
    for path in project_policies(&roots) {
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let project = refact_privacy::parse_policy_yaml(&content).map_err(|e| {
                    EngineError::Failed(format!("{} does not parse: {e}", path.display()))
                })?;
                merged = refact_privacy::merge_project(&merged, &project);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(super::common::failed(e)),
        }
    }
    Ok(merged)
}

/// Items for what changes effectively in this project once project rules are merged in.
fn effective_items(
    old: &refact_privacy::PrivacyPolicy,
    new: &refact_privacy::PrivacyPolicy,
    args: &Value,
) -> Vec<refact_chat_api::PythonEngineItem> {
    let mut items = Vec::new();
    if old.blocked != new.blocked {
        items.push(super::mask::item(
            "effective blocked here",
            Some(json!(old.blocked)),
            Some(json!(new.blocked)),
        ));
    }
    for pattern in strings(&args["unblock"]) {
        if new.blocked.contains(&pattern) {
            items.push(super::mask::named_item(
                &format!("{pattern} here"),
                None,
                Some(json!("still blocked here by project rules")),
            ));
        }
    }
    for zone in &new.zones {
        let before = old.zones.iter().find(|z| z.name == zone.name);
        if before.is_some_and(|b| b.patterns != zone.patterns || b.send_to != zone.send_to) {
            items.push(super::mask::named_item(
                &format!("effective zone {} here", zone.name),
                before.map(|b| json!({"patterns":b.patterns,"send_to":b.send_to})),
                Some(json!({"patterns":zone.patterns,"send_to":zone.send_to})),
            ));
        }
    }
    if old.tool_access != new.tool_access {
        items.push(super::mask::item(
            "effective MCP access here",
            to_value(&old.tool_access).ok(),
            to_value(&new.tool_access).ok(),
        ));
    }
    if items.is_empty() {
        items.push(super::mask::item(
            "effective here",
            None,
            Some(json!("unchanged: this project's rules still apply")),
        ));
    }
    items
}

fn relax_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let old = global_policy(ctx).await?;
        let (new, mut items) = relaxed(&old, args)?;
        let without_zones = refact_privacy::PrivacyPolicy {
            zones: Vec::new(),
            ..old.clone()
        };
        let known: Vec<String> =
            crate::http::routers::v1::privacy::collect_destinations(&ctx.app, &without_zones)
                .await
                .into_iter()
                .map(|d| d.id.0)
                .collect();
        let opens_all = |p: &refact_privacy::PrivacyPolicy| {
            !known.is_empty()
                && p.blocked.is_empty()
                && p.zones.iter().all(|z| {
                    z.send_to.iter().any(|d| d == "*")
                        || known.iter().all(|k| z.send_to.contains(k))
                })
        };
        if opens_all(&new) && !opens_all(&old) {
            return Err(EngineError::Permission(
                "this would let every zone send to every known destination, leaving no privacy restriction; that is for you to do in the privacy settings".into(),
            ));
        }
        if items.is_empty() {
            return Ok(Preview {
                title: "relax privacy rules".into(),
                items,
                scope: None,
                class: EngineClass::Risky,
                warning: None,
                plan: Plan::nothing(),
            });
        }
        let before = effective_with(ctx, &old).await?;
        let after = effective_with(ctx, &new).await?;
        items.extend(effective_items(&before, &after, args));
        Ok(Preview {
            title: "relax privacy rules".into(),
            items,
            scope: Some("all chats and projects (global privacy.yaml)".into()),
            class: EngineClass::Risky,
            warning: Some(
                "Files these rules protected may then be sent to model providers or MCP servers."
                    .into(),
            ),
            plan: Plan::new(to_value(&old)?, new),
        })
    })
}

fn relax<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let old = global_policy(ctx).await?;
        let new: refact_privacy::PrivacyPolicy =
            ctx.take_plan("privacy.relax", &to_value(&old)?)?;
        crate::privacy::save_privacy_policy(ctx.app.gcx.clone(), new.clone())
            .await
            .map_err(EngineError::Failed)?;
        let (paths, error, _) = source_paths(&ctx.app.gcx);
        let path = paths.first().cloned().unwrap_or_default();
        let (old_json, new_json) = (to_value(&old)?, to_value(&new)?);
        let changed: Vec<Value> = ["blocked", "zones", "tool_access", "subagents"]
            .iter()
            .filter(|k| old_json[**k] != new_json[**k])
            .map(|k| json!({"field":k,"old":old_json[*k],"new":new_json[*k]}))
            .collect();
        Ok(EngineOutput::new(
            json!({"path":path,"changed":changed,"effective_reloaded":error.is_none()}),
            "privacy rules relaxed; project rules still apply on top",
        ))
    })
}
