use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::common::{date_range, failed, limit, limit_schema, redact, to_value};
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput};
use crate::http::routers::v1::stats as route;

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "stats.usage",
            summary: "LLM usage and known cost over a date range: totals and breakdowns by model, provider, day, hour, mode, task role, agent, task, errors and top conversations. Calls without pricing add no cost.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"from":{"type":"string","description":"First day, YYYY-MM-DD."},"to":{"type":"string","description":"Last day, YYYY-MM-DD, inclusive."}}}),
            output: || Some(json!({"type":"object","required":["totals"],"properties":{"date_range":{},"totals":{"type":"object"},"by_model":{},"by_provider":{},"by_day":{},"by_mode":{},"by_task_role":{},"by_agent":{},"by_task":{},"by_hour":{},"errors":{"type":"object"},"top_conversations":{"type":"array"}}})),
            handler: usage,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "stats.events",
            summary: "Raw LLM call events, newest first, filtered by date, model prefix, provider, chat (this chat by default, \"*\" for all) or success. Other chats' error messages are dropped.",
            class: EngineClass::RedactedRead,
            input: || {
                let mut properties = json!({"model":{"type":"string","description":"Prefix of model_id."},"provider":{"type":"string"},"chat_id":{"type":"string","minLength":1,"description":"This chat by default; \"*\" for every chat."},"success":{"type":"boolean"},"offset":{"type":"integer","minimum":0}});
                properties["from"] = json!({"type":"string","description":"First day, YYYY-MM-DD."});
                properties["to"] = json!({"type":"string","description":"Last day, YYYY-MM-DD, inclusive."});
                properties["limit"] = limit_schema();
                json!({"type":"object","additionalProperties":false,"properties":properties})
            },
            output: || Some(json!({"type":"object","required":["events","total","truncated","limit","offset"],"properties":{"events":{"type":"array","items":{"type":"object"}},"total":{"type":"integer","description":"matches scanned, up to offset+limit+1"},"truncated":{"type":"boolean","description":"more matches exist after this page"},"limit":{"type":"integer"},"offset":{"type":"integer"}}})),
            handler: events,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "stats.estimate_cost",
            summary: "Estimate the USD cost of a call with the given token counts for a model, from the engine's pricing data; cost is null when no pricing is known.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["model","prompt_tokens","completion_tokens"],"properties":{"model":{"type":"string","minLength":1,"description":"provider/model"},"prompt_tokens":{"type":"integer","minimum":0},"completion_tokens":{"type":"integer","minimum":0},"cache_read_tokens":{"type":"integer","minimum":0},"cache_creation_tokens":{"type":"integer","minimum":0}}}),
            output: || Some(json!({"type":"object","required":["model","pricing_known","cost"],"properties":{"model":{"type":"string"},"pricing":{"type":"object","description":"USD per 1M tokens"},"cost":{"type":["object","null"],"properties":{"prompt_usd":{"type":"number"},"generated_usd":{"type":"number"},"cache_read_usd":{"type":"number"},"cache_creation_usd":{"type":"number"},"total_usd":{"type":"number"}}},"pricing_known":{"type":"boolean"}}})),
            handler: estimate_cost,
            preview: None,
            timeout: None,
        },
    ]
}

fn usage<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let (from, to) = date_range(args)?;
        let body = route::llm_summary_body(
            ctx.app.gcx.clone(),
            route::StatsQuery {
                from: from.clone(),
                to: to.clone(),
            },
        )
        .await
        .map_err(|e| EngineError::Failed(e.message))?;
        let data: Value = serde_json::from_str(&body).map_err(failed)?;
        let totals = &data["totals"];
        let text = format!(
            "{} calls, {} tokens, ${:.4} known cost from {} to {}",
            totals["total_calls"],
            totals["total_tokens"],
            totals["total_cost_usd"].as_f64().unwrap_or_default(),
            from.as_deref().unwrap_or("the start"),
            to.as_deref().unwrap_or("today")
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn events<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let (from, to) = date_range(args)?;
        let limit = limit(args);
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let chat_id = match args.get("chat_id").and_then(Value::as_str) {
            Some("*") => None,
            Some(id) => Some(id.to_string()),
            None => Some(ctx.chat_id.clone()),
        };
        let str_arg = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_string);
        let query = route::StatsEventsQuery {
            from,
            to,
            model: str_arg("model"),
            provider: str_arg("provider"),
            chat_id,
            success: args.get("success").and_then(Value::as_bool),
            limit: Some(limit),
            offset: Some(offset),
        };
        let dirs = crate::stats::get_stats_dirs_for_read(ctx.app.gcx.clone()).await;
        let target = offset.saturating_add(limit).saturating_add(1);
        let scanned = tokio::task::spawn_blocking(move || {
            route::scan_events_newest_first(&dirs, &query, target, |_| {})
        })
        .await
        .map_err(failed)?;
        let total = scanned.len();
        let truncated = total > offset.saturating_add(limit);
        let mut failed_count = 0;
        let mut rows = Vec::new();
        for event in scanned.into_iter().skip(offset).take(limit) {
            failed_count += usize::from(!event.success);
            let own = event.chat_id == ctx.chat_id;
            let mut row = to_value(&event)?;
            match row["error_message"].as_str() {
                Some(message) if own => row["error_message"] = json!(redact(message)),
                Some(_) => {
                    if let Some(map) = row.as_object_mut() {
                        map.remove("error_message");
                    }
                }
                None => {}
            }
            rows.push(row);
        }
        let text = format!("{} events ({failed_count} failed)", rows.len());
        Ok(EngineOutput::new(
            json!({"events":rows,"total":total,"truncated":truncated,"limit":limit,"offset":offset}),
            text,
        ))
    })
}

fn estimate_cost<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let model = args["model"].as_str().unwrap_or_default().to_string();
        let count = |key: &str| args.get(key).and_then(Value::as_u64).map(|n| n as usize);
        let pricing = crate::providers::pricing::lookup_model_pricing(&ctx.app.gcx, &model).await;
        let Some(pricing) = pricing else {
            return Ok(EngineOutput::new(
                json!({"model":model,"cost":null,"pricing_known":false}),
                format!("no pricing for {model}"),
            ));
        };
        let prompt_tokens = count("prompt_tokens").unwrap_or_default();
        let completion_tokens = count("completion_tokens").unwrap_or_default();
        let usage = refact_core::chat_types::ChatUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            cache_read_tokens: count("cache_read_tokens"),
            cache_creation_tokens: count("cache_creation_tokens"),
            ..Default::default()
        };
        let cost = crate::providers::pricing::compute_cost(&usage, &pricing);
        let text = match &cost {
            Some(cost) => format!("${:.6} for {model}", cost.total_usd),
            None => format!("no valid pricing for {model}"),
        };
        Ok(EngineOutput::new(
            json!({"model":model,"pricing":to_value(&pricing)?,"cost":cost.as_ref().map(to_value).transpose()?,"pricing_known":cost.is_some()}),
            text,
        ))
    })
}
