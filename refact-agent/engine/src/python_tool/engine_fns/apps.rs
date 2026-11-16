use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput};
use crate::integrations::mcp::redaction::scrub_text;

pub fn functions() -> Vec<EngineFn> {
    vec![EngineFn {
        name: "apps.details",
        summary: "One app's catalog details: files, manifest, connector status (no cached data) and the chats that bind it (titles cut to 80 characters; own=false for other chats). Check coverage.complete on large projects.",
        class: EngineClass::RedactedRead,
        input: || json!({"type":"object","additionalProperties":false,"required":["app_id"],"properties":{"app_id":{"type":"string","pattern":"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"}}}),
        output: || Some(json!({"type":"object","required":["app","files","connectors","chats","coverage"],"properties":{"app":{"type":"object"},"files":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"}}}},"connectors":{"type":"array"},"chats":{"type":"array","items":{"type":"object","properties":{"chat_id":{"type":"string"},"title":{"type":"string"},"binding_ids":{"type":"array"},"own":{"type":"boolean"}}}},"coverage":{"type":"object"}}})),
        handler: details,
        preview: None,
        timeout: None,
    }]
}

fn scrub_strings(value: &mut Value) {
    match value {
        Value::String(s) => *s = scrub_text(s, &[]),
        Value::Array(items) => items.iter_mut().for_each(scrub_strings),
        Value::Object(map) => map.values_mut().for_each(scrub_strings),
        _ => {}
    }
}

fn details<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let app_id: uuid::Uuid = args["app_id"]
            .as_str()
            .unwrap_or_default()
            .parse()
            .map_err(|_| EngineError::Type("app_id must be a UUID".into()))?;
        let caller = crate::apps::Caller::HostChat {
            chat_id: ctx.chat_id.clone(),
        };
        let runtime = crate::apps::runtime(&ctx.app, &caller)
            .await
            .map_err(EngineError::Failed)?;
        crate::apps::privacy::preflight_metadata(&ctx.app, &runtime, &caller)
            .await
            .map_err(EngineError::Permission)?;
        let mut data = crate::apps::details::get(&ctx.app, &runtime, app_id)
            .await
            .map_err(EngineError::Failed)?;
        if let Some(sources) = data["app"]["manifest"].get_mut("sources") {
            scrub_strings(sources);
        }
        for connector in data["connectors"].as_array_mut().into_iter().flatten() {
            scrub_strings(connector);
        }
        for chat in data["chats"].as_array_mut().into_iter().flatten() {
            let own = chat["chat_id"] == ctx.chat_id.as_str();
            if let Some(title) = chat["title"].as_str() {
                chat["title"] = json!(title.chars().take(80).collect::<String>());
            }
            chat["own"] = json!(own);
        }
        let name = data["app"]["manifest"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let text = format!(
            "{name}: {} files, {} connectors, bound in {} chats",
            data["files"].as_array().map_or(0, Vec::len),
            data["connectors"].as_array().map_or(0, Vec::len),
            data["chats"].as_array().map_or(0, Vec::len)
        );
        Ok(EngineOutput::new(data, text))
    })
}
