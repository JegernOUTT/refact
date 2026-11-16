use std::collections::HashSet;

use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::common::{limit, limit_schema, require_root_chat, to_value};
use super::mask::item;
use super::{previewed, EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput, Preview};
use crate::integrations::mcp::redaction::{redact_tool_secrets, scrub_text};
use crate::integrations::setting_up_integrations::{self as configs, IntegrationRecord};
use crate::tools::tool_manage_mcp as mcp;

pub fn functions() -> Vec<EngineFn> {
    let named = || json!({"type":"object","additionalProperties":false,"required":["server"],"properties":{"server":{"type":"string","minLength":1,"description":"Short MCP server name as manage_mcp shows it."},"scope":{"enum":["project","global"],"description":"Needed when the name exists in both scopes."}}});
    vec![
        EngineFn {
            name: "integrations.list",
            summary: "Configured integrations (cmdline, service, MCP, notifier) with scope, availability and confirmation rules, plus config errors. MCP servers this chat's model may not use are left out.",
            class: EngineClass::RedactedRead,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"kind":{"enum":["all","mcp","cmdline","service","notifier"],"description":"all by default."},"include_templates":{"type":"boolean","description":"Also list integrations without a config file; false by default."},"limit":limit_schema()}}),
            output: || Some(json!({"type":"object","required":["integrations","total","truncated","errors"],"properties":{"integrations":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"kind":{"type":"string"},"scope":{"enum":["project","global"]},"project_path":{"type":"string"},"config_exists":{"type":"boolean"},"on_your_laptop":{"type":"boolean"},"when_isolated":{"type":"boolean"},"ask_user":{"type":"array","items":{"type":"string"}},"deny":{"type":"array","items":{"type":"string"}}}}},"total":{"type":"integer"},"truncated":{"type":"boolean"},"errors":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"},"line":{"type":"integer"},"message":{"type":"string"}}}}}})),
            handler: list,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "integrations.mcp_get",
            summary: "One MCP server: status, server info, tools with schemas, resources, prompts, metrics, recent progress and its settings, secrets masked. Logs: use the manage_mcp tool.",
            class: EngineClass::RedactedRead,
            input: named,
            output: || Some(json!({"type":"object","required":["name","transport","scope","status"],"properties":{"name":{"type":"string"},"transport":{"type":"string"},"scope":{"type":"string"},"status":{},"auth_status":{},"server_name":{"type":"string"},"server_version":{"type":"string"},"protocol_version":{"type":"string"},"capabilities":{"type":"object"},"tools":{"type":"array"},"resources":{"type":"array"},"prompts":{"type":"array"},"metrics":{"type":"object"},"active_progress":{"type":"array"},"settings":{"type":"object"}}})),
            handler: mcp_get,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "integrations.mcp_oauth_status",
            summary: "OAuth state of an MCP server: auth type, signed in, expiry, scopes, whether a login is needed. Never returns tokens.",
            class: EngineClass::Read,
            input: named,
            output: || Some(json!({"type":"object","required":["auth_type","authenticated","expires_at","needs_login"],"properties":{"auth_type":{"type":"string"},"authenticated":{"type":"boolean"},"expires_at":{"type":"integer","description":"ms; 0 means no expiry"},"scopes":{"type":"array","items":{"type":"string"}},"needs_login":{"type":"boolean"},"oauth_available":{"type":"boolean"},"suggested_scopes":{"type":"array","items":{"type":"string"}}}})),
            handler: mcp_oauth_status,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "integrations.mcp_interactions",
            summary: "Pending MCP elicitations and sampling approvals waiting for an answer, engine-wide, oldest first. A sampling preview can hold chat or file content the server received; no privacy records are attached.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"limit":limit_schema()}}),
            output: || Some(json!({"type":"object","required":["interactions","total","truncated"],"properties":{"interactions":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"server_name":{"type":"string"},"kind":{"enum":["elicitation","sampling_approval"]},"message":{"type":"string"},"requested_schema":{},"url":{"type":"string"},"message_count":{"type":"integer"},"max_tokens":{"type":"integer"},"preview":{"type":"string"},"created_at_ms":{"type":"integer"},"timeout_at_ms":{"type":"integer"}}}},"total":{"type":"integer"},"truncated":{"type":"boolean"}}})),
            handler: mcp_interactions,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "integrations.mcp_export",
            summary: "Global MCP server configs as a shareable bundle with secrets replaced by a placeholder. Project configs are not exported.",
            class: EngineClass::RedactedRead,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"names":{"type":"array","minItems":1,"items":{"type":"string","minLength":1},"description":"Config names such as mcp_http_x; every global MCP config by default."}}}),
            output: || Some(json!({"type":"object","required":["version","exported_at","servers"],"properties":{"version":{"type":"integer"},"exported_at":{"type":"string"},"servers":{"type":"array","items":{"type":"object","properties":{"config_name":{"type":"string"},"transport":{"type":"string"},"config":{"type":"object"},"tools_config":{"type":"object"},"confirmation":{"type":"object"}}}}}})),
            handler: mcp_export,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "integrations.mcp_interaction_respond",
            summary: "Answer a pending MCP elicitation or sampling approval (accept, decline or cancel) after the user approves. Accepting a sampling request is risky; requests that ask the user to open a page are refused.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["id","action"],"properties":{"id":{"type":"string","minLength":1},"action":{"enum":["accept","decline","cancel"]},"content":{"type":"object","description":"Form values of an elicitation; only with accept."}}}),
            output: || Some(json!({"type":"object","required":["id","action","delivered"],"properties":{"id":{"type":"string"},"action":{"type":"string"},"delivered":{"type":"boolean"}}})),
            handler: respond,
            preview: Some(respond_preview),
            timeout: None,
        },
        EngineFn {
            name: "integrations.save",
            summary: "Create or update an integration config (cmdline_*, service_*, mcp_*, notifier) with merged settings after the user approves; \"***\" keeps an existing secret. Tools appear in the next turn, not in this run.",
            class: EngineClass::Risky,
            input: || json!({"type":"object","additionalProperties":false,"required":["name","settings"],"properties":{"name":{"type":"string","minLength":1,"description":"Integration name such as cmdline_build or mcp_http_github."},"scope":{"enum":["project","global"],"description":"project by default."},"settings":{"type":"object","description":"Partial settings merged into the current config."}}}),
            output: || Some(json!({"type":"object","required":["name","scope","created"],"properties":{"name":{"type":"string"},"scope":{"type":"string"},"created":{"type":"boolean"},"note":{"type":"string"}}})),
            handler: save,
            preview: Some(save_preview),
            timeout: Some(std::time::Duration::from_secs(60)),
        },
        EngineFn {
            name: "integrations.delete",
            summary: "Delete an integration config file and stop its MCP session, after the user approves.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["name"],"properties":{"name":{"type":"string","minLength":1,"description":"Integration name as integrations.list shows it."},"scope":{"enum":["project","global"],"description":"Needed when the name exists in both scopes."}}}),
            output: || Some(json!({"type":"object","required":["name","scope","removed"],"properties":{"name":{"type":"string"},"scope":{"type":"string"},"removed":{"type":"boolean"}}})),
            handler: delete,
            preview: Some(delete_preview),
            timeout: None,
        },
        EngineFn {
            name: "integrations.mcp_oauth_logout",
            summary: "Sign an MCP server out (remove its stored OAuth tokens and reconnect) after the user approves. A script cannot sign back in.",
            class: EngineClass::Destructive,
            input: named,
            output: || Some(json!({"type":"object","required":["name","signed_out"],"properties":{"name":{"type":"string"},"signed_out":{"type":"boolean"}}})),
            handler: logout,
            preview: Some(logout_preview),
            timeout: Some(std::time::Duration::from_secs(60)),
        },
    ]
}

/// Values of the secrets and variables files, to scrub wherever a config could quote them.
async fn known_values(ctx: &EngineCtx) -> Vec<String> {
    configs::get_vars_for_replacements(ctx.app.gcx.clone(), &mut Vec::new())
        .await
        .into_values()
        .filter(|v| v.len() >= 4)
        .collect()
}

fn kind(record: &IntegrationRecord) -> &'static str {
    let name = record.integr_name.as_str();
    if name.starts_with("cmdline_") {
        "cmdline"
    } else if name.starts_with("service_") {
        "service"
    } else if name.starts_with("mcp_") {
        "mcp"
    } else {
        "notifier"
    }
}

async fn model(ctx: &EngineCtx) -> String {
    ctx.ccx.lock().await.current_model.clone()
}

fn list<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let gcx = ctx.app.gcx.clone();
        let wanted = args.get("kind").and_then(Value::as_str).unwrap_or("all");
        let templates = args.get("include_templates").and_then(Value::as_bool) == Some(true);
        let result = configs::integrations_all(gcx.clone(), templates).await;
        let known = known_values(ctx).await;
        let model = model(ctx).await;
        let mut seen = HashSet::new();
        let mut rows = Vec::new();
        for record in result.integrations {
            let kind = kind(&record);
            if (wanted != "all" && wanted != kind)
                || !seen.insert(record.integr_config_path.clone())
            {
                continue;
            }
            if kind == "mcp"
                && record.integr_config_exists
                && !crate::tools::tools_list::mcp_config_allowed_for_model(
                    gcx.clone(),
                    &model,
                    &record.integr_config_path,
                )
                .await
            {
                continue;
            }
            let scrub = |items: &[String]| -> Vec<String> {
                items.iter().map(|i| scrub_text(i, &known)).collect()
            };
            rows.push(json!({
                "name": record.integr_name,
                "kind": kind,
                "scope": if record.project_path.is_empty() { "global" } else { "project" },
                "project_path": record.project_path,
                "config_exists": record.integr_config_exists,
                "on_your_laptop": record.on_your_laptop,
                "when_isolated": record.when_isolated,
                "ask_user": scrub(&record.ask_user),
                "deny": scrub(&record.deny),
            }));
        }
        let errors: Vec<Value> = result
            .error_log
            .iter()
            .map(|e| json!({"path":e.path,"line":e.error_line,"message":scrub_text(&e.error_msg, &known)}))
            .collect();
        let total = rows.len();
        let mcp_count = rows.iter().filter(|r| r["kind"] == "mcp").count();
        let limit = limit(args);
        rows.truncate(limit);
        let text = format!(
            "{total} integrations ({mcp_count} MCP), {} config errors",
            errors.len()
        );
        Ok(EngineOutput::new(
            json!({"integrations":rows,"total":total,"truncated":total > limit,"errors":errors}),
            text,
        ))
    })
}

/// The MCP record for `name` (and `scope`) among the servers this chat's model may use.
async fn record(ctx: &EngineCtx, args: &Value) -> Result<IntegrationRecord, EngineError> {
    let records = mcp::mcp_records_for_model(ctx.app.gcx.clone(), &model(ctx).await).await;
    let name = args["server"].as_str().unwrap_or_default();
    let scope = args.get("scope").and_then(Value::as_str);
    mcp::resolve(&records, name, scope)
        .cloned()
        .map_err(EngineError::Type)
}

/// Secret values of a record: secrets file values, secret-looking fields and `$VAR` substitutions.
async fn sensitive(ctx: &EngineCtx, record: &IntegrationRecord) -> Vec<String> {
    let mut values = known_values(ctx).await;
    mcp::collect_sensitive(&record.config_unparsed, false, &mut values);
    if let Ok(raw) = mcp::read_raw(&record.integr_config_path).await {
        mcp::collect_sensitive(&raw, false, &mut values);
        mcp::substitutions(&raw, &record.config_unparsed, &mut values);
    }
    values.retain(|v| !v.is_empty());
    values
}

fn mcp_get<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let record = record(ctx, args).await?;
        let gcx = ctx.app.gcx.clone();
        let entry = mcp::entry(gcx.clone(), &record, true, None)
            .await
            .map_err(EngineError::Failed)?;
        let mut data = json!({
            "name": entry["name"],
            "transport": entry["transport"],
            "scope": entry["scope"],
            "status": entry["status"],
            "auth_status": entry["auth_status"],
            "settings": entry["settings"],
        });
        match crate::http::routers::v1::mcp_server_info::mcp_server_info(
            gcx,
            record.integr_config_path.clone(),
        )
        .await
        {
            Ok(info) => {
                let mut info = to_value(info)?;
                if let Some(map) = info.as_object_mut() {
                    map.remove("logs_tail");
                    map.remove("config_path");
                    map.remove("status");
                    map.remove("auth_status");
                    for (key, value) in std::mem::take(map) {
                        data[key] = value;
                    }
                }
            }
            Err(e) if e.status_code == hyper::StatusCode::NOT_FOUND => {}
            Err(e) => return Err(EngineError::Failed(e.message)),
        }
        let sensitive = sensitive(ctx, &record).await;
        mcp::sanitize(&mut data, &sensitive);
        let text = format!(
            "{}: {}, {} tools",
            data["name"].as_str().unwrap_or_default(),
            data["status"].as_str().unwrap_or("connected"),
            data["tools"].as_array().map_or(0, Vec::len)
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn mcp_oauth_status<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let record = record(ctx, args).await?;
        let name = args["server"].as_str().unwrap_or_default();
        let status = crate::http::routers::v1::mcp_oauth::mcp_oauth_status(
            &ctx.app.gcx,
            &record.integr_config_path,
        )
        .await
        .map_err(|e| {
            if e.status_code == hyper::StatusCode::NOT_FOUND {
                EngineError::Failed(format!(
                    "{name}: no active session (disabled or not started)"
                ))
            } else {
                EngineError::Failed(format!("{name}: OAuth status unavailable"))
            }
        })?;
        let data = to_value(status)?;
        let text = if data["authenticated"] == true {
            match data["expires_at"].as_i64() {
                Some(0) | None => format!("{name}: signed in"),
                Some(ms) => format!(
                    "{name}: signed in until {}",
                    chrono::DateTime::from_timestamp_millis(ms)
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_default()
                ),
            }
        } else if data["needs_login"] == true {
            format!("{name}: needs login")
        } else {
            format!("{name}: not signed in")
        };
        Ok(EngineOutput::new(data, text))
    })
}

/// Pending interactions of servers this chat's model may use; sampling previews hold another
/// chat's messages, so they are never returned.
async fn visible_interactions(
    ctx: &EngineCtx,
) -> Vec<crate::integrations::mcp::mcp_interactions::MCPInteractionInfo> {
    let model = model(ctx).await;
    let mut visible = Vec::new();
    for info in crate::integrations::mcp::mcp_interactions::list_mcp_interactions().await {
        if crate::tools::tools_list::mcp_config_allowed_for_model(
            ctx.app.gcx.clone(),
            &model,
            &info.config_path,
        )
        .await
        {
            visible.push(info);
        }
    }
    visible
}

fn mcp_interactions<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        require_root_chat(ctx, "integrations.mcp_interactions").await?;
        let all = visible_interactions(ctx).await;
        let known = known_values(ctx).await;
        let total = all.len();
        let limit = limit(args);
        let mut rows = Vec::new();
        for info in all.into_iter().take(limit) {
            let mut row = to_value(&info)?;
            if let Some(map) = row.as_object_mut() {
                map.remove("config_path");
                map.remove("elicitation_id");
                map.remove("preview");
            }
            redact_tool_secrets(&mut row);
            for key in ["message", "url"] {
                if let Some(text) = row[key].as_str() {
                    row[key] = json!(scrub_text(text, &known));
                }
            }
            rows.push(row);
        }
        let text = format!("{total} pending MCP interactions");
        Ok(EngineOutput::new(
            json!({"interactions":rows,"total":total,"truncated":total > limit}),
            text,
        ))
    })
}

fn mcp_export<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let names: Vec<String> = args
            .get("names")
            .and_then(Value::as_array)
            .map(|n| {
                n.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let bundle = crate::http::routers::v1::mcp_config_sharing::export_bundle(
            &ctx.app.gcx,
            &names,
            false,
        )
        .await;
        let known = known_values(ctx).await;
        let model = model(ctx).await;
        let dir = ctx.app.gcx.config_dir.join("integrations.d");
        let mut data = to_value(&bundle)?;
        if let Some(servers) = data["servers"].as_array_mut() {
            let mut kept = Vec::new();
            for server in servers.drain(..) {
                let Some(name) = server["config_name"].as_str() else {
                    continue;
                };
                let path = dir
                    .join(format!("{name}.yaml"))
                    .to_string_lossy()
                    .into_owned();
                if crate::tools::tools_list::mcp_config_allowed_for_model(
                    ctx.app.gcx.clone(),
                    &model,
                    &path,
                )
                .await
                {
                    kept.push(server);
                }
            }
            *servers = kept;
        }
        for server in data["servers"].as_array_mut().into_iter().flatten() {
            redact_tool_secrets(&mut server["config"]);
            scrub_strings(server, &known);
        }
        let text = format!(
            "exported {} servers",
            data["servers"].as_array().map_or(0, Vec::len)
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn scrub_strings(value: &mut Value, known: &[String]) {
    match value {
        Value::String(s) => *s = scrub_text(s, known),
        Value::Array(items) => items.iter_mut().for_each(|v| scrub_strings(v, known)),
        Value::Object(map) => map.values_mut().for_each(|v| scrub_strings(v, known)),
        _ => {}
    }
}

const SAVE_WARNING: &str = "This integration can run commands or send data to an external server without further confirmation rules you have not reviewed.";
const DELETE_WARNING: &str = "The integration's config file is deleted, including stored OAuth sign-in, and cannot be restored by the engine.";
const LOGOUT_WARNING: &str =
    "Stored sign-in tokens are deleted; signing in again needs the user in a browser.";
const SAMPLING_WARNING: &str = "Accepting lets this MCP server run a model call with the shown messages, which can send chat data to the server.";

fn respond_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    use crate::integrations::mcp::mcp_interactions::MCPInteractionKind;
    Box::pin(async move {
        require_root_chat(ctx, "integrations.mcp_interaction_respond").await?;
        let id = args["id"].as_str().unwrap_or_default();
        let action = args["action"].as_str().unwrap_or_default();
        let content = args.get("content");
        if content.is_some() && action != "accept" {
            return Err(EngineError::Type(
                "content is allowed only with action accept".into(),
            ));
        }
        let info = visible_interactions(ctx)
            .await
            .into_iter()
            .find(|info| info.id == id)
            .ok_or_else(|| EngineError::Type(format!("no pending MCP interaction {id}")))?;
        let known = known_values(ctx).await;
        let (request, sampling) = match &info.kind {
            MCPInteractionKind::Elicitation {
                message,
                requested_schema,
                url,
                ..
            } => {
                if url.is_some() {
                    return Err(EngineError::Permission(
                        "this request asks the user to open a page; a script cannot complete it"
                            .into(),
                    ));
                }
                if let (Some(schema), Some(content)) = (requested_schema, content) {
                    refact_tool_api::schema_validate::validate(schema, content).map_err(
                        |errors| {
                            EngineError::Type(format!(
                                "content does not match the requested form: {}",
                                errors
                                    .iter()
                                    .map(|e| e.message.clone())
                                    .collect::<Vec<_>>()
                                    .join("; ")
                            ))
                        },
                    )?;
                }
                (scrub_text(message, &known), false)
            }
            MCPInteractionKind::SamplingApproval {
                message_count,
                max_tokens,
                ..
            } => (
                format!("model call with {message_count} messages, up to {max_tokens} tokens"),
                true,
            ),
        };
        if action == "accept" && content.is_some() {
            super::knowledge::egress(
                ctx,
                refact_privacy::Destination {
                    id: refact_privacy::DestinationId(info.server_name.clone()),
                    kind: refact_privacy::DestinationKind::Mcp,
                    display_name: info.server_name.clone(),
                },
                "the answer",
            )
            .await?;
        }
        let keys = content
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
            .filter(|k| !k.is_empty());
        let answer = match keys {
            Some(keys) => format!("{action} ({keys})"),
            None => action.to_string(),
        };
        let risky = sampling && action == "accept";
        Ok(Preview {
            title: "answer an MCP request".into(),
            items: vec![
                item("server", None, Some(json!(info.server_name))),
                item(
                    "request",
                    None,
                    Some(json!(request.chars().take(200).collect::<String>())),
                ),
                item("answer", Some(json!("pending")), Some(json!(answer))),
            ],
            scope: Some("engine".into()),
            class: if risky {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning: risky.then(|| SAMPLING_WARNING.to_string()),
            plan: previewed::plan(json!(id)),
        })
    })
}

fn respond<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use crate::integrations::mcp::mcp_interactions::{respond_mcp_interaction, MCPInteractionResponse};
    Box::pin(async move {
        let id = args["id"].as_str().unwrap_or_default();
        let info = visible_interactions(ctx)
            .await
            .into_iter()
            .find(|info| info.id == id);
        let pending = info
            .as_ref()
            .map(|info| json!(info.id))
            .unwrap_or(Value::Null);
        previewed::take(ctx, "integrations.mcp_interaction_respond", &pending)?;
        let action = args["action"].as_str().unwrap_or_default().to_string();
        let server = info.map(|i| i.server_name).unwrap_or_default();
        respond_mcp_interaction(
            id,
            MCPInteractionResponse {
                action: action.clone(),
                content: args.get("content").cloned(),
            },
        )
        .await
        .map_err(EngineError::Failed)?;
        Ok(EngineOutput::new(
            json!({"id": id, "action": action, "delivered": true}),
            format!("{action} sent to {server}"),
        ))
    })
}

/// Config path of integration `name` in `scope`; names never carry a path.
async fn integration_path(
    ctx: &EngineCtx,
    name: &str,
    scope: &str,
) -> Result<(String, std::path::PathBuf), EngineError> {
    if name.is_empty()
        || name.contains(['/', '\\'])
        || name.contains("..")
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(EngineError::Type(format!(
            "{name:?} is not an integration name"
        )));
    }
    let family_ok = crate::integrations::integrations_list(false)
        .iter()
        .any(|template| {
            let family = template.trim_end_matches("TEMPLATE");
            if family.ends_with('_') {
                name.starts_with(family) && name.len() > family.len()
            } else {
                name == *template
            }
        });
    if !family_ok {
        return Err(EngineError::Type(format!(
            "{name} is not a known integration family (cmdline_*, service_*, mcp_*, {})",
            crate::integrations::notifier::integr_notifier_telegram::INTEGRATION_ID
        )));
    }
    let root = ctx.project_root.clone();
    let (dirs, global) = configs::get_config_dirs(ctx.app.gcx.clone(), &root).await;
    let dir =
        if scope == "global" {
            global
        } else {
            match dirs.as_slice() {
                [dir] => dir.clone(),
                _ => return Err(EngineError::Type(
                    "the project scope needs exactly one project workspace; use scope=\"global\""
                        .into(),
                )),
            }
        };
    let path = configs::join_config_path(&dir, name);
    if scope != "global" {
        let project = dir
            .parent()
            .ok_or_else(|| EngineError::Failed(format!("{} has no project", dir.display())))?;
        super::common::inside_project(
            project,
            std::path::Path::new(&path),
            &ctx.app.gcx.config_dir,
        )?;
    }
    crate::integrations::mcp::management::validate_config_save_path(
        std::path::Path::new(&path),
        &dir.join("integrations.d"),
    )
    .map_err(|e| EngineError::Permission(e.message))?;
    Ok((path, dir))
}

fn scope_label(ctx: &EngineCtx, scope: &str) -> String {
    if scope == "global" {
        "global (all projects)".into()
    } else {
        format!(
            "project {}",
            ctx.project_root
                .as_ref()
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_default()
        )
    }
}

/// (dotted key, old, new) of every leaf `settings` changes in `existing`.
fn changed_leaves(
    prefix: &str,
    existing: &Value,
    patch: &Value,
    out: &mut Vec<(String, Option<Value>, Value)>,
) {
    match patch {
        Value::Object(map) => {
            for (key, value) in map {
                let label = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                changed_leaves(
                    &label,
                    existing.get(key).unwrap_or(&Value::Null),
                    value,
                    out,
                );
            }
        }
        _ => {
            if existing != patch {
                out.push((
                    prefix.to_string(),
                    (!existing.is_null()).then(|| existing.clone()),
                    patch.clone(),
                ));
            }
        }
    }
}

fn masked_leaf(key: &str, value: Value) -> Value {
    let mut wrapped = json!({ key.rsplit('.').next().unwrap_or(key): value });
    redact_tool_secrets(&mut wrapped);
    wrapped
        .as_object_mut()
        .and_then(|m| m.values_mut().next().map(Value::take))
        .unwrap_or(Value::Null)
}

/// The plan state of a config file: a digest of its bytes under its path, never its text.
async fn file_state(path: &str) -> Result<Value, EngineError> {
    let digest = match tokio::fs::read(path).await {
        Ok(bytes) => Some(super::digest(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(EngineError::Failed(format!("cannot read the config: {e}"))),
    };
    Ok(json!({ path: digest }))
}

/// The save warning, naming the command line the saved integration runs and the environment
/// variables the call sets.
fn save_warning(merged: &Value, leaves: &[(String, Option<Value>, Value)]) -> String {
    let mut warning = SAVE_WARNING.to_string();
    if let Some(command) = merged
        .get("command")
        .and_then(Value::as_str)
        .filter(|c| !c.trim().is_empty())
    {
        warning.push_str(&format!(" It runs the command: {command}"));
        if let Some(args) = merged.get("args").and_then(Value::as_array) {
            for arg in args {
                warning.push(' ');
                warning.push_str(&arg.as_str().map_or_else(|| arg.to_string(), String::from));
            }
        }
        warning.push('.');
    }
    let env: Vec<&str> = leaves
        .iter()
        .filter_map(|(key, _, _)| key.strip_prefix("env."))
        .collect();
    if !env.is_empty() {
        warning.push_str(&format!(
            " It sets the environment variables {}, which can change what that command runs.",
            env.join(", ")
        ));
    }
    warning
}

#[derive(Clone)]
struct SavePlan {
    path: String,
    merged: Value,
}

async fn save_plan(
    ctx: &EngineCtx,
    args: &Value,
) -> Result<(String, Value, Vec<(String, Option<Value>, Value)>, bool), EngineError> {
    require_root_chat(ctx, "integrations.save").await?;
    let name = args["name"].as_str().unwrap_or_default();
    let scope = args
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("project");
    let settings = &args["settings"];
    if settings.get("oauth_tokens").is_some() {
        return Err(EngineError::Permission(
            "oauth_tokens cannot be set from a script; sign in through the GUI".into(),
        ));
    }
    let (path, _) = integration_path(ctx, name, scope).await?;
    if name.starts_with("mcp_")
        && !crate::tools::tools_list::mcp_config_allowed_for_model(
            ctx.app.gcx.clone(),
            &model(ctx).await,
            &path,
        )
        .await
    {
        return Err(EngineError::Permission(
            "this MCP server is not allowed for this chat's model provider".into(),
        ));
    }
    let exists = std::path::Path::new(&path).exists();
    let existing = if exists {
        mcp::read_raw(&path).await.map_err(EngineError::Failed)?
    } else {
        json!({})
    };
    let mut merged = existing.clone();
    mcp::merge_settings(&mut merged, settings, false);
    let mut leaves = Vec::new();
    changed_leaves("", &existing, &merged, &mut leaves);
    Ok((path, merged, leaves, exists))
}

fn save_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let (path, merged, leaves, exists) = save_plan(ctx, args).await?;
        let scope = args
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("project");
        let title = "save an integration".to_string();
        if leaves.is_empty() && exists {
            return Ok(Preview {
                title,
                items: Vec::new(),
                scope: Some(scope_label(ctx, scope)),
                class: EngineClass::Risky,
                warning: Some(SAVE_WARNING.into()),
                plan: super::Plan::nothing(),
            });
        }
        let warning = save_warning(&merged, &leaves);
        let mut items: Vec<_> = leaves
            .into_iter()
            .map(|(key, old, new)| item(&key, old.map(|v| masked_leaf(&key, v)), Some(new)))
            .collect();
        if items.is_empty() {
            items.push(item("config", None, Some(json!("created with defaults"))));
        }
        let state = file_state(&path).await?;
        Ok(Preview {
            title,
            items,
            scope: Some(scope_label(ctx, scope)),
            class: EngineClass::Risky,
            warning: Some(warning),
            plan: super::Plan::new(
                state,
                SavePlan {
                    path,
                    merged,
                },
            ),
        })
    })
}

fn save<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let name = args["name"].as_str().unwrap_or_default();
        let scope = args
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("project");
        let (path, _) = integration_path(ctx, name, scope).await?;
        let current = file_state(&path).await?;
        let plan: SavePlan = ctx.take_plan("integrations.save", &current)?;
        let created = current[&path].is_null();
        configs::integration_config_save(ctx.app.gcx.clone(), &plan.path, &plan.merged)
            .await
            .map_err(|_| {
                EngineError::Failed(
                    "integration settings validation or save failed; check the settings".into(),
                )
            })?;
        let destination =
            crate::integrations::mcp::mcp_interactions::server_name_from_config_path(&plan.path);
        let mut data = json!({"name": name, "scope": scope, "created": created});
        if name.starts_with("mcp_") {
            data["note"] = json!(format!("If a privacy zone guards data in this chat, add {destination} to that zone's destinations in Privacy settings. New tools appear in the next turn's tool list, not in this run."));
        }
        Ok(EngineOutput::new(data, format!("saved {name} ({scope})")))
    })
}

async fn existing_record(
    ctx: &EngineCtx,
    name: &str,
    scope: Option<&str>,
) -> Result<IntegrationRecord, EngineError> {
    let all = configs::integrations_all(ctx.app.gcx.clone(), false)
        .await
        .integrations;
    let matches: Vec<_> = all
        .into_iter()
        .filter(|r| {
            r.integr_config_exists
                && r.integr_name == name
                && scope.is_none_or(|s| mcp::scope(r) == s)
        })
        .collect();
    let mut unique: Vec<IntegrationRecord> = Vec::new();
    for record in matches {
        if !unique
            .iter()
            .any(|u| u.integr_config_path == record.integr_config_path)
        {
            unique.push(record);
        }
    }
    match unique.len() {
        1 => {
            let record = unique.remove(0);
            if mcp::scope(&record) == "project" {
                super::common::inside_project(
                    std::path::Path::new(&record.project_path),
                    std::path::Path::new(&record.integr_config_path),
                    &ctx.app.gcx.config_dir,
                )?;
            }
            Ok(record)
        }
        0 => Err(EngineError::Type(format!("no integration config {name}"))),
        _ => Err(EngineError::Type(format!(
            "{name} exists in more than one scope; pass scope"
        ))),
    }
}

fn delete_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        require_root_chat(ctx, "integrations.delete").await?;
        let name = args["name"].as_str().unwrap_or_default();
        let record = existing_record(ctx, name, args.get("scope").and_then(Value::as_str)).await?;
        let scope = mcp::scope(&record).to_string();
        let tools = if name.starts_with("mcp_") {
            mcp::entry(ctx.app.gcx.clone(), &record, false, None)
                .await
                .ok()
                .and_then(|e| {
                    e["tools"]
                        .as_u64()
                        .or(e["tools"].as_array().map(|t| t.len() as u64))
                })
                .unwrap_or(0)
        } else {
            0
        };
        let state = file_state(&record.integr_config_path).await?;
        Ok(Preview {
            title: "delete an integration".into(),
            items: vec![
                item(
                    "config",
                    Some(json!(format!("{name} ({scope})"))),
                    Some(json!("deleted")),
                ),
                item(
                    "tools",
                    Some(json!(format!("{tools} tools"))),
                    Some(json!("0")),
                ),
            ],
            scope: Some(scope_label(ctx, &scope)),
            class: EngineClass::Destructive,
            warning: Some(DELETE_WARNING.into()),
            plan: previewed::plan(state),
        })
    })
}

fn delete<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let name = args["name"].as_str().unwrap_or_default();
        let record = existing_record(ctx, name, args.get("scope").and_then(Value::as_str)).await?;
        let state = file_state(&record.integr_config_path).await?;
        previewed::take(ctx, "integrations.delete", &state)?;
        crate::integrations::mcp::management::delete_integration_config(
            ctx.app.gcx.clone(),
            std::path::Path::new(&record.integr_config_path),
        )
        .await
        .map_err(|e| EngineError::Failed(e.message))?;
        let scope = mcp::scope(&record).to_string();
        Ok(EngineOutput::new(
            json!({"name": name, "scope": scope, "removed": true}),
            format!("deleted {name}"),
        ))
    })
}

fn logout_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        require_root_chat(ctx, "integrations.mcp_oauth_logout").await?;
        let record = record(ctx, args).await?;
        let tokens =
            crate::integrations::mcp::mcp_auth::load_tokens_from_config(&record.integr_config_path)
                .await;
        let Some(tokens) = tokens else {
            return Ok(Preview {
                title: "sign out of an MCP server".into(),
                items: Vec::new(),
                scope: Some("engine".into()),
                class: EngineClass::Destructive,
                warning: Some(LOGOUT_WARNING.into()),
                plan: super::Plan::nothing(),
            });
        };
        let old = match chrono::DateTime::from_timestamp_millis(tokens.expires_at) {
            Some(t) if tokens.expires_at > 0 => format!("signed in (expires {})", t.to_rfc3339()),
            _ => "signed in".into(),
        };
        Ok(Preview {
            title: "sign out of an MCP server".into(),
            items: vec![item("oauth", Some(json!(old)), Some(json!("signed out")))],
            scope: Some("engine".into()),
            class: EngineClass::Destructive,
            warning: Some(LOGOUT_WARNING.into()),
            plan: previewed::plan(json!(true)),
        })
    })
}

fn logout<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let record = record(ctx, args).await?;
        let signed_in =
            crate::integrations::mcp::mcp_auth::load_tokens_from_config(&record.integr_config_path)
                .await
                .is_some();
        previewed::take(ctx, "integrations.mcp_oauth_logout", &json!(signed_in))?;
        crate::http::routers::v1::mcp_oauth::mcp_oauth_logout(
            ctx.app.gcx.clone(),
            &record.integr_config_path,
        )
        .await
        .map_err(|e| EngineError::Failed(e.message))?;
        let name = args["server"].as_str().unwrap_or_default();
        Ok(EngineOutput::new(
            json!({"name": name, "signed_out": true}),
            format!("{name} signed out"),
        ))
    })
}
