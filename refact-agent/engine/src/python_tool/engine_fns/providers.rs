use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::common::to_value;
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput};
use crate::custom_error::ScratchError;
use crate::integrations::mcp::redaction::{redact_tool_secrets, scrub_text};
use crate::providers::http as routes;

const NETWORK_TIMEOUT: Duration = Duration::from_secs(20);

fn provider_arg() -> Value {
    json!({"type":"string","minLength":1,"description":"Provider instance name, e.g. openai or openai_2."})
}

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "providers.list",
            summary: "Provider instances with status, enabled flag, whether credentials are present and how many models are selected.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["providers"],"properties":{"providers":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"base_provider":{"type":"string"},"display_name":{"type":"string"},"enabled":{"type":"boolean"},"readonly":{"type":"boolean"},"has_credentials":{"type":"boolean"},"status":{"enum":["active","configured","not_configured"]},"model_count":{"type":"integer"}}}}}})),
            handler: list,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "providers.get",
            summary: "One provider's settings with every secret masked, its endpoints and selected models. Does not refresh tokens.",
            class: EngineClass::RedactedRead,
            input: || json!({"type":"object","additionalProperties":false,"required":["provider"],"properties":{"provider":provider_arg()}}),
            output: || Some(json!({"type":"object","required":["name","base_provider","status","settings"],"properties":{"name":{"type":"string"},"base_provider":{"type":"string"},"display_name":{"type":"string"},"enabled":{"type":"boolean"},"readonly":{"type":"boolean"},"has_credentials":{"type":"boolean"},"status":{"type":"string"},"settings":{"type":"object"},"endpoints":{"type":"object","properties":{"chat":{"type":"string"}}},"wire_format":{"type":"string"},"chat_models":{"type":"array"}}})),
            handler: get,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "providers.status",
            summary: "Provider health, account limits, subscription usage and quota windows. May call the provider (and refresh its tokens) when refresh is true or for health and usage; 20 s limit.",
            class: EngineClass::RedactedRead,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"provider":provider_arg(),"refresh":{"type":"boolean","description":"Fetch quotas now instead of a fresh cached copy; false by default."}}}),
            output: || Some(json!({"type":"object","required":["quotas"],"properties":{"quotas":{"type":"array","items":{"type":"object"}},"health":{"type":"object"},"usage":{"type":"object"}}})),
            handler: status,
            preview: None,
            timeout: Some(Duration::from_secs(60)),
        },
        EngineFn {
            name: "providers.available_models",
            summary: "Models a provider offers (from the registry or its API, 20 s limit) and which are enabled here.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["provider"],"properties":{"provider":provider_arg()}}),
            output: || Some(json!({"type":"object","required":["source","models"],"properties":{"source":{"type":"string"},"models":{"type":"array","items":{"type":"object"}},"error":{"type":"string"}}})),
            handler: available_models,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "providers.pressure",
            summary: "Request concurrency, backoff and circuit-breaker state per provider credential (keys are hashes, not secrets).",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["enabled","buckets"],"properties":{"enabled":{"type":"boolean"},"buckets":{"type":"array","items":{"type":"object","properties":{"bucket":{"type":"string"},"limit":{"type":"integer"},"in_flight":{"type":"integer"},"interactive_in_flight":{"type":"integer"},"consecutive_successes":{"type":"integer"},"consecutive_retryable_failures":{"type":"integer"},"retry_after_until_ms":{"type":"integer"},"breaker":{"type":"object"}}}}}})),
            handler: pressure,
            preview: None,
            timeout: None,
        },
    ]
    .into_iter()
    .chain(change_functions())
    .collect()
}

fn route_error(error: ScratchError) -> EngineError {
    let message = scrub_text(&error.message, &[]);
    if error.status_code.is_client_error() {
        EngineError::Type(message)
    } else {
        EngineError::Failed(message)
    }
}

/// Scrubs credentials out of URL strings (user:pass@, ?key=) anywhere in the value.
fn scrub_strings(value: &mut Value) {
    match value {
        Value::String(text) => *text = scrub_text(text, &[]),
        Value::Array(items) => items.iter_mut().for_each(scrub_strings),
        Value::Object(map) => map.values_mut().for_each(scrub_strings),
        _ => {}
    }
}

fn masked(mut value: Value) -> Value {
    redact_tool_secrets(&mut value);
    scrub_strings(&mut value);
    value
}

fn list<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let providers = to_value(routes::providers_list(ctx.app.gcx.clone()).await)?;
        let rows = providers.as_array().cloned().unwrap_or_default();
        let active = rows.iter().filter(|p| p["status"] == "active").count();
        Ok(EngineOutput::new(
            json!({"providers":rows}),
            format!("{} providers ({active} active)", rows.len()),
        ))
    })
}

fn get<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let provider = args["provider"].as_str().unwrap_or_default();
        let detail = to_value(
            routes::provider_detail(ctx.app.gcx.clone(), provider)
                .await
                .map_err(route_error)?,
        )?;
        let mut data = json!({
            "name":detail["name"],"base_provider":detail["base_provider"],"display_name":detail["display_name"],
            "enabled":detail["enabled"],"readonly":detail["readonly"],"has_credentials":detail["has_credentials"],
            "status":detail["status"],"settings":masked(detail["settings"].clone()),
        });
        let runtime = &detail["runtime"];
        if runtime.is_object() {
            data["endpoints"] = masked(json!({"chat":runtime["chat_endpoint"]}));
            data["wire_format"] = runtime["wire_format"].clone();
            data["chat_models"] = masked(runtime["chat_models"].clone());
        }
        let text = format!(
            "{} ({}): {}",
            data["name"].as_str().unwrap_or(provider),
            data["base_provider"].as_str().unwrap_or(""),
            data["status"].as_str().unwrap_or("")
        );
        Ok(EngineOutput::new(data, text))
    })
}

async fn timed<T>(
    future: impl std::future::Future<Output = T>,
    what: &str,
) -> Result<T, EngineError> {
    tokio::time::timeout(NETWORK_TIMEOUT, future)
        .await
        .map_err(|_| {
            EngineError::Failed(format!(
                "{what} took longer than {} s",
                NETWORK_TIMEOUT.as_secs()
            ))
        })
}

fn status<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let gcx = ctx.app.gcx.clone();
        let refresh = args.get("refresh").and_then(Value::as_bool) == Some(true);
        let mut data = json!({});
        let quotas = match args.get("provider").and_then(Value::as_str) {
            Some(provider) => {
                let identity = routes::resolve_provider_identity(&gcx, provider)
                    .await
                    .map_err(route_error)?;
                let snapshot = timed(
                    routes::provider_quota_cached(
                        gcx.clone(),
                        &identity.instance_id,
                        &identity.base_provider,
                        refresh,
                    ),
                    "the quota request",
                )
                .await?;
                if let Some(health) = timed(
                    routes::provider_health(gcx.clone(), provider),
                    "the health check",
                )
                .await?
                .map_err(route_error)?
                {
                    data["health"] = masked(health);
                }
                match tokio::time::timeout(
                    NETWORK_TIMEOUT,
                    routes::provider_usage(gcx.clone(), provider, Default::default()),
                )
                .await
                {
                    Ok(Ok(usage)) => data["usage"] = masked(usage),
                    Ok(Err(error)) if error.status_code == hyper::StatusCode::BAD_REQUEST => {}
                    Ok(Err(error)) => {
                        data["usage"] = json!({"error":scrub_text(&error.message, &[])})
                    }
                    Err(_) => {
                        data["usage"] = json!({"error":format!("the usage request took longer than {} s", NETWORK_TIMEOUT.as_secs())})
                    }
                }
                vec![snapshot]
            }
            None => timed(
                routes::provider_quotas(gcx.clone(), refresh),
                "the quota requests",
            )
            .await?
            .map_err(route_error)?,
        };
        let mut lines = Vec::new();
        for quota in &quotas {
            let windows: Vec<String> = quota
                .windows
                .iter()
                .filter_map(|w| {
                    let value = to_value(w).ok()?;
                    let name = value["name"].as_str()?.to_string();
                    let used = value
                        .get("used_percent")
                        .or(value.get("used"))
                        .filter(|v| !v.is_null())?;
                    Some(format!("{name} {used}"))
                })
                .collect();
            if quota.available {
                lines.push(format!(
                    "{}: {}",
                    quota.provider_name,
                    if windows.is_empty() {
                        "available".into()
                    } else {
                        windows.join(", ")
                    }
                ));
            }
        }
        data["quotas"] = masked(to_value(&quotas)?);
        let text = if lines.is_empty() {
            format!("{} providers, no quota data", quotas.len())
        } else {
            lines.join("; ")
        };
        Ok(EngineOutput::new(data, text))
    })
}

fn available_models<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let provider = args["provider"].as_str().unwrap_or_default();
        let response = routes::provider_available_models(ctx.app.gcx.clone(), provider)
            .await
            .map_err(route_error)?;
        let enabled = response.models.iter().filter(|m| m.enabled).count();
        let mut data = json!({"source":response.source,"models":to_value(&response.models)?});
        if let Some(error) = &response.error {
            data["error"] = json!(scrub_text(error, &[]));
        }
        let text = format!(
            "{} models from {} ({enabled} enabled)",
            response.models.len(),
            response.source
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn pressure<'a>(
    _ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let Some(registry) = refact_providers::pressure::pressure_registry() else {
            return Ok(EngineOutput::new(
                json!({"enabled":false,"buckets":[]}),
                "provider pressure control is off",
            ));
        };
        let mut buckets = to_value(registry.snapshots())?;
        let mut cooling = 0;
        for bucket in buckets.as_array_mut().into_iter().flatten() {
            if let Some(reason) = bucket.pointer_mut("/breaker/reason") {
                if let Some(text) = reason.as_str() {
                    *reason = json!(scrub_text(text, &[]));
                }
            }
            if !bucket["breaker"].is_null() || !bucket["retry_after_until_ms"].is_null() {
                cooling += 1;
            }
            if let Some(map) = bucket.as_object_mut() {
                map.retain(|_, v| !v.is_null());
            }
        }
        let count = buckets.as_array().map_or(0, Vec::len);
        Ok(EngineOutput::new(
            json!({"enabled":true,"buckets":buckets}),
            format!("{count} buckets, {cooling} cooling down"),
        ))
    })
}

// ---------------------------------------------------------------------------
// Changes.

use super::{Plan, Preview};

pub(super) fn change_functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "providers.set_model_enabled",
            summary: "Enable or disable one model of a provider for model selection; a disabled model in use shows who uses it.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["name","model","enabled"],"properties":{"name":provider_arg(),"model":{"type":"string","minLength":1,"maxLength":256},"enabled":{"type":"boolean"}}}),
            output: || Some(json!({"type":"object","required":["name","model","enabled"],"properties":{"name":{"type":"string"},"model":{"type":"string"},"enabled":{"type":"boolean"}}})),
            handler: set_model_enabled,
            preview: Some(set_model_enabled_preview),
            timeout: None,
        },
        EngineFn {
            name: "providers.add_custom_model",
            summary: "Add or update a custom model entry on a provider (context window, features, pricing).",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["name","id","config"],"properties":{"name":provider_arg(),"id":{"type":"string","minLength":1,"maxLength":256},"config":custom_model_schema()}}),
            output: || Some(json!({"type":"object","required":["name","id","replaced"],"properties":{"name":{"type":"string"},"id":{"type":"string"},"replaced":{"type":"boolean"}}})),
            handler: add_custom_model,
            preview: Some(add_custom_model_preview),
            timeout: None,
        },
        EngineFn {
            name: "providers.remove_custom_model",
            summary: "Remove a custom model entry from a provider (the preview shows its config, so it can be added back).",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["name","model"],"properties":{"name":provider_arg(),"model":{"type":"string","minLength":1,"maxLength":256}}}),
            output: || Some(json!({"type":"object","required":["name","model","removed"],"properties":{"name":{"type":"string"},"model":{"type":"string"},"removed":{"type":"boolean"}}})),
            handler: remove_custom_model,
            preview: Some(remove_custom_model_preview),
            timeout: None,
        },
        EngineFn {
            name: "providers.set",
            summary: "Change a provider's non-secret settings (endpoints, wire format, enabled flag, options); credentials and identity are refused. Risky when an endpoint host changes.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["provider","settings"],"properties":{"provider":provider_arg(),"settings":{"type":"object","minProperties":1,"description":"Keys from providers.get settings, e.g. chat_endpoint, wire_format, enabled; no keys, tokens, headers or credentials."}}}),
            output: || Some(json!({"type":"object","required":["provider","changed"],"properties":{"provider":{"type":"string"},"changed":{"type":"array","items":{"type":"string"}}}})),
            handler: set,
            preview: Some(set_preview),
            timeout: None,
        },
        EngineFn {
            name: "providers.sign_out",
            summary: "Sign out of a provider that uses OAuth, deleting its stored tokens; signing in again needs the browser flow in the provider settings.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["provider"],"properties":{"provider":provider_arg()}}),
            output: || Some(json!({"type":"object","required":["provider","signed_out"],"properties":{"provider":{"type":"string"},"signed_out":{"type":"boolean"}}})),
            handler: sign_out,
            preview: Some(sign_out_preview),
            timeout: None,
        },
    ]
}

fn custom_model_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "n_ctx":{"type":"integer","minimum":1024},
        "max_output_tokens":{"type":"integer","minimum":1},
        "supports_tools":{"type":"boolean"},
        "supports_parallel_tools":{"type":"boolean"},
        "supports_strict_tools":{"type":"boolean"},
        "supports_multimodality":{"type":"boolean"},
        "image_max_side_px":{"type":"integer","minimum":1},
        "image_preferred_side_px":{"type":"integer","minimum":1},
        "image_token_mode":{"type":"string"},
        "reasoning_effort_options":{"type":"array","items":{"type":"string"}},
        "supports_thinking_budget":{"type":"boolean"},
        "supports_adaptive_thinking_budget":{"type":"boolean"},
        "supports_cache_control":{"type":"boolean"},
        "tokenizer":{"type":"string"},
        "pricing":{"type":"object"}
    }})
}

/// The provider's model state as the preview shows it; refuses readonly and unknown providers.
async fn model_state(ctx: &EngineCtx, name: &str) -> Result<Value, EngineError> {
    let gcx = ctx.app.gcx.clone();
    routes::ensure_provider_for_model_management(gcx.clone(), name)
        .await
        .map_err(route_error)?;
    let registry = gcx.providers.read().await;
    let provider = registry
        .get(name)
        .ok_or_else(|| EngineError::Type(format!("provider {name} is not configured")))?;
    if provider.is_readonly() {
        return Err(EngineError::Permission(format!(
            "provider {name} is readonly"
        )));
    }
    Ok(json!({
        "enabled":provider.enabled_models(),
        "disabled":provider.disabled_models(),
        "custom":to_value(provider.custom_models())?,
    }))
}

fn set_model_enabled_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let name = args["name"].as_str().unwrap_or_default();
        let model = args["model"].as_str().unwrap_or_default();
        let enabled = args["enabled"].as_bool().unwrap_or(false);
        let state = model_state(ctx, name).await?;
        let known = state["custom"].get(model).is_some()
            || routes::provider_available_models(ctx.app.gcx.clone(), name)
                .await
                .map_err(route_error)?
                .models
                .iter()
                .any(|m| m.id == model);
        if !known {
            return Err(EngineError::Type(format!(
                "{name} offers no model {model}; see providers.available_models"
            )));
        }
        let full = format!("{name}/{model}");
        let caps =
            crate::global_context::try_load_caps_quickly_if_not_present(ctx.app.gcx.clone(), 0)
                .await
                .ok();
        let was = caps
            .as_ref()
            .map(|c| c.chat_models.contains_key(&full))
            .unwrap_or(false)
            || state["enabled"]
                .as_array()
                .is_some_and(|m| m.iter().any(|v| v == model));
        let mut items = vec![super::mask::named_item(
            &full,
            Some(json!(was)),
            Some(json!(enabled)),
        )];
        if !enabled {
            let mut users = Vec::new();
            if let Some(caps) = &caps {
                let d = &caps.defaults;
                for (slot, id) in [
                    ("chat", &d.chat_default_model),
                    ("light", &d.chat_light_model),
                    ("thinking", &d.chat_thinking_model),
                    ("chat_model_2", &d.chat_model_2),
                    ("task_planner", &d.task_planner_agent_model),
                    ("buddy", &d.chat_buddy_model),
                ] {
                    if *id == full {
                        users.push(format!("default slot {slot}"));
                    }
                }
            }
            let loaded: Vec<_> = ctx
                .app
                .chat
                .sessions
                .read()
                .await
                .values()
                .cloned()
                .collect();
            let mut chats = 0;
            for session in loaded {
                if let Ok(s) = session.try_lock() {
                    chats += usize::from(s.thread.model == full);
                }
            }
            if chats > 0 {
                users.push(format!("{chats} loaded chats"));
            }
            if !users.is_empty() {
                items.push(super::mask::item(
                    "used by",
                    None,
                    Some(json!(users.join(", "))),
                ));
            }
        }
        let plan = if was == enabled {
            Plan::nothing()
        } else {
            super::previewed::plan(state)
        };
        Ok(Preview {
            title: if enabled {
                "enable a model".into()
            } else {
                "disable a model".into()
            },
            items,
            scope: Some("all chats".into()),
            class: EngineClass::Change,
            warning: None,
            plan,
        })
    })
}

fn set_model_enabled<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let name = args["name"].as_str().unwrap_or_default();
        let model = args["model"].as_str().unwrap_or_default();
        let enabled = args["enabled"].as_bool().unwrap_or(false);
        super::previewed::take(
            ctx,
            "providers.set_model_enabled",
            &model_state(ctx, name).await?,
        )?;
        routes::set_provider_model_enabled(ctx.app.gcx.clone(), name, model, enabled)
            .await
            .map_err(route_error)?;
        Ok(EngineOutput::new(
            json!({"name":name,"model":model,"enabled":enabled}),
            format!(
                "{name}/{model} {}",
                if enabled { "enabled" } else { "disabled" }
            ),
        ))
    })
}

fn add_custom_model_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let name = args["name"].as_str().unwrap_or_default();
        let id = args["id"].as_str().unwrap_or_default();
        let config: refact_core::provider_types::CustomModelConfig =
            serde_json::from_value(args["config"].clone())
                .map_err(|e| EngineError::Type(format!("config: {e}")))?;
        let state = model_state(ctx, name).await?;
        let old = state["custom"].get(id).cloned();
        let new = to_value(&config)?;
        let mut items = Vec::new();
        let keys: std::collections::BTreeSet<String> = new
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, _)| k.clone())
            .chain(
                old.iter()
                    .flat_map(|o| o.as_object().into_iter().flatten().map(|(k, _)| k.clone())),
            )
            .collect();
        for key in keys {
            let o = old.as_ref().and_then(|o| o.get(&key)).cloned();
            let n = new.get(&key).cloned();
            if o != n {
                items.push(super::mask::item(&key, o, n));
            }
        }
        let plan = if items.is_empty() {
            Plan::nothing()
        } else {
            Plan::new(state, config)
        };
        Ok(Preview {
            title: if old.is_some() {
                "update a custom model".into()
            } else {
                "add a custom model".into()
            },
            items,
            scope: Some(format!("provider {name}, model {id}")),
            class: EngineClass::Change,
            warning: None,
            plan,
        })
    })
}

fn add_custom_model<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let name = args["name"].as_str().unwrap_or_default();
        let id = args["id"].as_str().unwrap_or_default();
        let config: refact_core::provider_types::CustomModelConfig =
            ctx.take_plan("providers.add_custom_model", &model_state(ctx, name).await?)?;
        let previous = routes::add_provider_custom_model(ctx.app.gcx.clone(), name, id, config)
            .await
            .map_err(route_error)?;
        Ok(EngineOutput::new(
            json!({"name":name,"id":id,"replaced":previous.is_some()}),
            format!(
                "custom model {name}/{id} {}",
                if previous.is_some() {
                    "updated"
                } else {
                    "added"
                }
            ),
        ))
    })
}

fn remove_custom_model_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let name = args["name"].as_str().unwrap_or_default();
        let model = args["model"].as_str().unwrap_or_default();
        let state = model_state(ctx, name).await?;
        let old = state["custom"]
            .get(model)
            .cloned()
            .ok_or_else(|| EngineError::Type(format!("{name} has no custom model {model}")))?;
        Ok(Preview {
            title: "remove a custom model".into(),
            items: vec![super::mask::named_item(
                model,
                Some(old),
                Some(json!("removed")),
            )],
            scope: Some(format!("provider {name}")),
            class: EngineClass::Change,
            warning: None,
            plan: super::previewed::plan(state),
        })
    })
}

fn remove_custom_model<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let name = args["name"].as_str().unwrap_or_default();
        let model = args["model"].as_str().unwrap_or_default();
        super::previewed::take(
            ctx,
            "providers.remove_custom_model",
            &model_state(ctx, name).await?,
        )?;
        routes::remove_provider_custom_model(ctx.app.gcx.clone(), name, model)
            .await
            .map_err(route_error)?;
        Ok(EngineOutput::new(
            json!({"name":name,"model":model,"removed":true}),
            format!("custom model {name}/{model} removed"),
        ))
    })
}

/// Keys a script may never write: credentials, command-backed credentials, headers and identity.
const REFUSED_KEYS: &[&str] = &[
    "api_key",
    "auth_token",
    "tokenizer_api_key",
    "extra_headers",
    "credential",
    "oauth_tokens",
    "console_oauth_tokens",
    "base_provider",
    "display_name",
];

const ENDPOINT_KEYS: &[&str] = &["chat_endpoint", "base_url", "endpoint", "api_base"];

/// Keys whose change is a plain settings change; any other key, and any value that is a URL or
/// host, changes how or where requests go and makes the card risky.
const PLAIN_KEYS: &[&str] = &["enabled", "wire_format", "supports_cache_control"];

fn url_or_host(value: &Value) -> bool {
    let Some(text) = value.as_str().map(str::trim) else {
        return false;
    };
    text.contains("://")
        || url::Url::parse(text).is_ok_and(|u| u.host().is_some())
        || (!text.contains(char::is_whitespace)
            && text.split(['/', ':']).next().is_some_and(|host| {
                host.contains('.')
                    && host.split('.').all(|part| {
                        !part.is_empty()
                            && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                    })
                    && host.chars().any(|c| c.is_ascii_alphabetic())
            }))
}

fn plain_change(key: &str, old: Option<&Value>, new: &Value) -> bool {
    let simple = |v: &Value| v.is_boolean() || v.is_number();
    let plain = PLAIN_KEYS.contains(&key) || (simple(new) && old.is_none_or(simple));
    plain && !url_or_host(new) && !old.is_some_and(url_or_host)
}

fn refused_key(path: &str, key: &str, value: &Value) -> Result<(), EngineError> {
    let lower = key.to_lowercase();
    if REFUSED_KEYS.contains(&lower.as_str())
        || lower.contains("oauth")
        || crate::integrations::mcp::redaction::is_secret_field(&lower)
        || super::mask::secret_looking(&lower)
        || refact_providers::CLAUDE_SECRET_FIELDS.contains(&lower.as_str())
    {
        return Err(EngineError::Permission(format!(
            "{path}{key} is a credential or identity field; set it in the provider settings in the GUI"
        )));
    }
    match value {
        Value::String(text) if text == "***" => Err(EngineError::Permission(format!(
            "{path}{key} is a masked value; scripts do not handle secret fields"
        ))),
        Value::String(text) if text.trim_start().starts_with('$') => {
            Err(EngineError::Permission(format!(
                "{path}{key} refers to an environment variable; scripts cannot point settings at environment values, set it in the provider settings in the GUI"
            )))
        }
        Value::Object(map) => map
            .iter()
            .try_for_each(|(k, v)| refused_key(&format!("{path}{key}."), k, v)),
        Value::Array(items) => items.iter().try_for_each(|v| refused_key(path, key, v)),
        _ => Ok(()),
    }
}

/// The provider's settings as `providers.get` shows them (masked) plus a digest of its file.
async fn provider_state(ctx: &EngineCtx, name: &str) -> Result<(Value, Value), EngineError> {
    let detail = to_value(
        routes::provider_detail(ctx.app.gcx.clone(), name)
            .await
            .map_err(route_error)?,
    )?;
    if detail["readonly"] == true {
        return Err(EngineError::Permission(format!(
            "provider {name} is readonly"
        )));
    }
    let path = crate::providers::config_store::provider_config_path(&ctx.app.gcx.config_dir, name);
    let file = match tokio::fs::read(&path).await {
        Ok(bytes) => Some(crate::yaml_configs::customization_service::version(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(super::common::failed(e)),
    };
    let settings = masked(detail["settings"].clone());
    Ok((json!({"file":file,"settings":settings}), detail))
}

fn host(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    Some(
        url::Url::parse(text)
            .ok()
            .and_then(|u| {
                u.host_str()
                    .map(|h| format!("{h}:{}", u.port_or_known_default().unwrap_or(0)))
            })
            .unwrap_or_else(|| text.to_lowercase()),
    )
}

#[derive(Clone)]
struct SetPlan {
    settings: serde_yaml::Value,
    keys: Vec<String>,
}

fn set_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let name = args["provider"].as_str().unwrap_or_default();
        let settings = args["settings"].as_object().cloned().unwrap_or_default();
        for (key, value) in &settings {
            refused_key("", key, value)?;
        }
        let (state, detail) = provider_state(ctx, name).await?;
        let current = &state["settings"];
        let mut items = Vec::new();
        let mut keys = Vec::new();
        let mut moves = false;
        let mut other = Vec::new();
        for (key, value) in &settings {
            let old = current.get(key).filter(|v| !v.is_null()).cloned();
            if old.as_ref() == Some(value) {
                continue;
            }
            if !plain_change(key, old.as_ref(), value) {
                other.push(key.clone());
            }
            if ENDPOINT_KEYS.contains(&key.as_str()) {
                let runtime_old = detail["runtime"][key.as_str()].clone();
                let before = old.as_ref().and_then(host).or_else(|| host(&runtime_old));
                if host(value) != before {
                    moves = true;
                }
            }
            items.push(super::mask::item(key, old, Some(value.clone())));
            keys.push(key.clone());
        }
        let plan = if items.is_empty() {
            Plan::nothing()
        } else {
            let yaml = serde_yaml::to_value(Value::Object(settings.clone()))
                .map_err(|e| EngineError::Type(format!("settings: {e}")))?;
            Plan::new(
                state,
                SetPlan {
                    settings: yaml,
                    keys,
                },
            )
        };
        Ok(Preview {
            title: "change provider settings".into(),
            items,
            scope: Some(format!("all chats using {name}")),
            class: if moves || !other.is_empty() {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning: if moves {
                Some("Requests for this provider will be sent to a different server.".to_string())
            } else {
                (!other.is_empty()).then(|| {
                    format!(
                        "{} may change where requests for this provider go or what is sent with them.",
                        other.join(", ")
                    )
                })
            },
            plan,
        })
    })
}

fn set<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let name = args["provider"].as_str().unwrap_or_default();
        let (state, _) = provider_state(ctx, name).await?;
        let plan: SetPlan = ctx.take_plan("providers.set", &state)?;
        routes::update_provider_settings(ctx.app.gcx.clone(), name, plan.settings)
            .await
            .map_err(route_error)?;
        Ok(EngineOutput::new(
            json!({"provider":name,"changed":plan.keys}),
            format!("{name}: {} changed", plan.keys.join(", ")),
        ))
    })
}

/// Whether the provider holds a sign-in, plus a digest of its file; refuses non-OAuth providers.
async fn sign_in_state(ctx: &EngineCtx, name: &str) -> Result<(String, Value, bool), EngineError> {
    let gcx = ctx.app.gcx.clone();
    let base = routes::oauth_base_provider_for_instance(&gcx, name)
        .await
        .map_err(route_error)?;
    let (state, _) = provider_state(ctx, name).await?;
    let connected = routes::provider_file_has_credentials(&gcx, name)
        .await
        .map_err(route_error)?
        .unwrap_or(false);
    Ok((base, state, connected))
}

fn sign_out_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let name = args["provider"].as_str().unwrap_or_default();
        let (_, state, connected) = sign_in_state(ctx, name).await?;
        Ok(Preview {
            title: "sign out of a provider".into(),
            items: vec![super::mask::item(
                "sign-in",
                Some(json!("connected")),
                Some(json!("signed out")),
            )],
            scope: Some(format!("all chats using {name}")),
            class: EngineClass::Destructive,
            warning: Some("The stored sign-in will be deleted; you must sign in again in the provider settings to use these models.".into()),
            plan: if connected {
                super::previewed::plan(state)
            } else {
                Plan::nothing()
            },
        })
    })
}

fn sign_out<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let name = args["provider"].as_str().unwrap_or_default();
        let (base, state, _) = sign_in_state(ctx, name).await?;
        super::previewed::take(ctx, "providers.sign_out", &state)?;
        routes::provider_sign_out(&ctx.app.gcx, name, &base)
            .await
            .map_err(route_error)?;
        Ok(EngineOutput::new(
            json!({"provider":name,"signed_out":true}),
            format!("signed out of {name}; sign in again in the provider settings"),
        ))
    })
}
