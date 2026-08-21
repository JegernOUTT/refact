use std::collections::HashMap;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Map, Value};

use refact_core::chat_types::{ChatContent, ChatMessage, ChatUsage};
use refact_privacy::Cleared;
use refact_core::antigravity_client::{antigravity_headers, ANTIGRAVITY_VERSION};
use crate::adapter::{
    insert_extra_headers, AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError,
};
use crate::canonical::{CanonicalToolChoice, LlmRequest, LlmStreamDelta};
use crate::params::ReasoningIntent;

const PROJECT_HEADER: &str = "x-refact-internal-cloudcode-project";

pub struct GoogleCloudCodeAdapter;

impl LlmWireAdapter for GoogleCloudCodeAdapter {
    fn build_http(
        &self,
        req: &Cleared<LlmRequest>,
        settings: &AdapterSettings,
    ) -> Result<HttpParts, String> {
        let mut extra_headers = settings.extra_headers.clone();
        let project = extra_headers
            .remove(PROJECT_HEADER)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                "Antigravity account has no resolved Cloud Code project; reconnect the account"
                    .to_string()
            })?;

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", settings.auth_token))
                .map_err(|error| format!("invalid Antigravity auth token: {error}"))?,
        );
        for (name, value) in antigravity_headers() {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| format!("invalid Antigravity header name: {error}"))?;
            let value = HeaderValue::from_str(&value)
                .map_err(|error| format!("invalid Antigravity header value: {error}"))?;
            headers.insert(name, value);
        }
        insert_extra_headers(&mut headers, &extra_headers);

        let (system, contents) = convert_messages(&req.messages);
        let mut inner = Map::new();
        inner.insert("contents".to_string(), Value::Array(contents));
        if !system.is_empty() {
            inner.insert(
                "systemInstruction".to_string(),
                json!({"parts": [{"text": system}]}),
            );
        }

        if settings.supports_tools {
            if let Some(tools) = req.tools.as_ref() {
                let declarations = convert_tools(tools);
                if !declarations.is_empty() {
                    inner.insert(
                        "tools".to_string(),
                        json!([{"functionDeclarations": declarations}]),
                    );
                }
            }
        }

        if let Some(choice) = req.tool_choice.as_ref() {
            inner.insert("toolConfig".to_string(), tool_config(choice));
        }

        let generation_config = generation_config(req, settings);
        if !generation_config.is_empty() {
            inner.insert(
                "generationConfig".to_string(),
                Value::Object(generation_config),
            );
        }

        if let Some(extra) = req.extra_body.as_ref() {
            for (key, value) in extra {
                inner.insert(key.clone(), value.clone());
            }
        }

        tracing::debug!(
            antigravity_version = ANTIGRAVITY_VERSION,
            model = %settings.model_name,
            project = %project,
            "Google Cloud Code request"
        );

        Ok(HttpParts {
            url: format!(
                "{}/v1internal:streamGenerateContent?alt=sse",
                settings.endpoint.trim_end_matches('/')
            ),
            headers,
            body: json!({
                "model": settings.model_name,
                "project": project,
                "request": Value::Object(inner),
            }),
        })
    }

    fn parse_stream_chunk(&self, data: &str) -> Result<Vec<LlmStreamDelta>, StreamParseError> {
        let trimmed = data.trim();
        if trimmed.is_empty() {
            return Err(StreamParseError::Skip);
        }
        if trimmed == "[DONE]" {
            return Ok(vec![LlmStreamDelta::Done]);
        }

        let json: Value = serde_json::from_str(trimmed)
            .map_err(|error| StreamParseError::MalformedChunk(format!("json parse: {error}")))?;
        if let Some(error) = json.get("error") {
            return Err(StreamParseError::FatalError(format_api_error(error)));
        }

        let response = json.get("response").unwrap_or(&json);
        let mut deltas = Vec::new();
        let mut tool_calls = Vec::new();
        let mut thinking_blocks = Vec::new();

        if let Some(parts) = response
            .get("candidates")
            .and_then(|value| value.as_array())
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(|parts| parts.as_array())
        {
            for (index, part) in parts.iter().enumerate() {
                if part.get("thought").and_then(|value| value.as_bool()) == Some(true) {
                    if let Some(text) = part.get("text").and_then(|value| value.as_str()) {
                        deltas.push(LlmStreamDelta::AppendReasoning {
                            text: text.to_string(),
                            block_index: None,
                        });
                    }
                } else if let Some(text) = part.get("text").and_then(|value| value.as_str()) {
                    deltas.push(LlmStreamDelta::AppendContent {
                        text: text.to_string(),
                        block_index: None,
                    });
                }

                if let Some(function_call) = part.get("functionCall") {
                    if let Some(name) = function_call.get("name").and_then(|value| value.as_str()) {
                        let args = function_call
                            .get("args")
                            .cloned()
                            .unwrap_or_else(|| json!({}));
                        let arguments =
                            serde_json::to_string(&args).unwrap_or_else(|_| "{}".into());
                        tool_calls.push(json!({
                            "index": index,
                            "id": format!("call_{index}_{name}"),
                            "type": "function",
                            "function": {"name": name, "arguments": arguments},
                        }));
                        deltas.push(LlmStreamDelta::SetToolCalls {
                            tool_calls: tool_calls.clone(),
                        });
                    }
                }

                if let Some(signature) = part
                    .get("thoughtSignature")
                    .and_then(|value| value.as_str())
                {
                    thinking_blocks.push(json!({
                        "index": index,
                        "type": "thinking",
                        "signature": signature,
                    }));
                }
            }
        }

        if !thinking_blocks.is_empty() {
            deltas.push(LlmStreamDelta::SetThinkingBlocks {
                blocks: thinking_blocks,
            });
        }

        if let Some(reason) = response
            .get("candidates")
            .and_then(|value| value.as_array())
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.get("finishReason"))
            .and_then(|value| value.as_str())
            .filter(|reason| *reason != "FINISH_REASON_UNSPECIFIED")
        {
            deltas.push(LlmStreamDelta::SetFinishReason {
                reason: finish_reason(reason),
            });
        }

        if let Some(usage) = response.get("usageMetadata") {
            deltas.push(LlmStreamDelta::SetUsage {
                usage: parse_usage(usage),
            });
        }

        if deltas.is_empty() {
            Err(StreamParseError::Skip)
        } else {
            Ok(deltas)
        }
    }
}

fn convert_messages(messages: &[ChatMessage]) -> (String, Vec<Value>) {
    let mut systems = Vec::new();
    let mut contents = Vec::new();
    let mut call_names = HashMap::new();

    for message in messages {
        if message.role == "system" || message.role == "developer" {
            let text = message.content.content_text_only();
            if !text.is_empty() {
                systems.push(text);
            }
            continue;
        }

        if message.role == "assistant" {
            if let Some(calls) = message.tool_calls.as_ref() {
                for call in calls {
                    if !call.id.is_empty() && !call.function.name.is_empty() {
                        call_names.insert(call.id.clone(), call.function.name.clone());
                    }
                }
            }
        }

        let (role, parts) = match message.role.as_str() {
            "user" => ("user", content_parts(&message.content)),
            "assistant" => {
                let mut parts = content_parts(&message.content);
                if let Some(calls) = message.tool_calls.as_ref() {
                    for call in calls {
                        if call.function.name.is_empty() {
                            continue;
                        }
                        parts.push(json!({
                            "functionCall": {
                                "name": call.function.name,
                                "args": parse_arguments(&call.function.arguments),
                            }
                        }));
                    }
                }
                ("model", parts)
            }
            "tool" | "diff" => {
                let name = call_names
                    .get(&message.tool_call_id)
                    .cloned()
                    .or_else(|| literal_tool_name(message));
                let Some(name) = name else {
                    tracing::debug!(
                        tool_call_id = %message.tool_call_id,
                        "skipping Cloud Code tool result with unknown function name"
                    );
                    continue;
                };
                let result = tool_result_value(&message.content);
                (
                    "user",
                    vec![json!({
                        "functionResponse": {
                            "name": name,
                            "response": {"result": result},
                        }
                    })],
                )
            }
            _ => continue,
        };

        if parts.is_empty() {
            continue;
        }
        push_merged_content(&mut contents, role, parts);
    }

    (systems.join("\n\n"), contents)
}

fn content_parts(content: &ChatContent) -> Vec<Value> {
    match content {
        ChatContent::Multimodal(elements) => elements
            .iter()
            .filter_map(|element| {
                if element.is_text() {
                    Some(json!({"text": element.m_content}))
                } else if element.is_image() {
                    Some(json!({
                        "inlineData": {
                            "mimeType": element.m_type,
                            "data": raw_base64(&element.m_content),
                        }
                    }))
                } else {
                    None
                }
            })
            .collect(),
        _ => {
            let text = content.content_text_only();
            if text.is_empty() {
                Vec::new()
            } else {
                vec![json!({"text": text})]
            }
        }
    }
}

fn raw_base64(content: &str) -> String {
    content
        .strip_prefix("data:")
        .and_then(|_| content.split_once(',').map(|(_, data)| data.to_string()))
        .unwrap_or_else(|| content.to_string())
}

fn parse_arguments(arguments: &str) -> Value {
    serde_json::from_str::<Value>(arguments.trim())
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn literal_tool_name(message: &ChatMessage) -> Option<String> {
    message
        .extra
        .get("name")
        .or_else(|| message.extra.get("tool_name"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn tool_result_value(content: &ChatContent) -> Value {
    let text = content.content_text_only();
    serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text))
}

fn push_merged_content(contents: &mut Vec<Value>, role: &str, parts: Vec<Value>) {
    if let Some(previous) = contents.last_mut() {
        if previous.get("role").and_then(|value| value.as_str()) == Some(role) {
            if let Some(previous_parts) = previous
                .get_mut("parts")
                .and_then(|value| value.as_array_mut())
            {
                previous_parts.extend(parts);
                return;
            }
        }
    }
    contents.push(json!({"role": role, "parts": parts}));
}

fn convert_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            let function = tool.get("function")?.as_object()?;
            let name = function.get("name")?.as_str()?.trim();
            if name.is_empty() {
                return None;
            }
            let mut declaration = Map::new();
            declaration.insert("name".to_string(), json!(name));
            if let Some(description) = function.get("description") {
                declaration.insert("description".to_string(), description.clone());
            }
            let mut parameters = function
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"}));
            sanitize_schema(&mut parameters);
            declaration.insert("parameters".to_string(), parameters);
            Some(Value::Object(declaration))
        })
        .collect()
}

fn sanitize_schema(schema: &mut Value) {
    let source = schema.clone();
    sanitize_schema_inner(schema, &source, &mut Vec::new());
}

fn sanitize_schema_inner(schema: &mut Value, source: &Value, resolving_refs: &mut Vec<String>) {
    match schema {
        Value::Array(values) => {
            for value in values {
                sanitize_schema_inner(value, source, resolving_refs);
            }
        }
        Value::Object(object) => {
            if let Some(reference) = object
                .remove("$ref")
                .and_then(|value| value.as_str().map(str::to_string))
            {
                if let Some(pointer) = reference.strip_prefix('#') {
                    if !resolving_refs.contains(&reference) {
                        if let Some(mut referenced) = source.pointer(pointer).cloned() {
                            resolving_refs.push(reference);
                            sanitize_schema_inner(&mut referenced, source, resolving_refs);
                            resolving_refs.pop();
                            merge_schema(object, referenced);
                        }
                    }
                }
            }

            for union_key in ["oneOf", "anyOf"] {
                if let Some(Value::Array(mut variants)) = object.remove(union_key) {
                    for variant in &mut variants {
                        sanitize_schema_inner(variant, source, resolving_refs);
                    }
                    merge_union(object, variants);
                }
            }

            for key in [
                "additionalProperties",
                "$schema",
                "$defs",
                "definitions",
                "exclusiveMinimum",
                "exclusiveMaximum",
                "const",
            ] {
                object.remove(key);
            }
            for value in object.values_mut() {
                sanitize_schema_inner(value, source, resolving_refs);
            }
            if object.get("type").and_then(|value| value.as_str()) == Some("object")
                && !object.contains_key("properties")
            {
                object.insert("properties".to_string(), json!({}));
            }
        }
        _ => {}
    }
}

fn merge_schema(target: &mut Map<String, Value>, schema: Value) {
    if let Value::Object(schema) = schema {
        for (key, value) in schema {
            target.entry(key).or_insert(value);
        }
    }
}

fn merge_union(target: &mut Map<String, Value>, variants: Vec<Value>) {
    let variants: Vec<Map<String, Value>> = variants
        .into_iter()
        .filter_map(|variant| variant.as_object().cloned())
        .filter(|variant| variant.get("type").and_then(Value::as_str) != Some("null"))
        .collect();

    if variants.len() == 1 {
        merge_schema(target, Value::Object(variants[0].clone()));
        return;
    }
    if variants.is_empty() {
        return;
    }

    let common_type = variants[0].get("type").cloned().filter(|schema_type| {
        variants
            .iter()
            .all(|variant| variant.get("type") == Some(schema_type))
    });
    if let Some(schema_type) = common_type {
        target.entry("type".to_string()).or_insert(schema_type);
    }

    // An enum union can be represented exactly by collecting its alternatives.
    if variants
        .iter()
        .all(|variant| variant.get("enum").and_then(Value::as_array).is_some())
    {
        let mut values = Vec::new();
        for variant in &variants {
            for value in variant["enum"].as_array().unwrap() {
                if !values.contains(value) {
                    values.push(value.clone());
                }
            }
        }
        target
            .entry("enum".to_string())
            .or_insert(Value::Array(values));
    }

    // Combining object properties without making them required is a useful,
    // non-restrictive approximation of object alternatives.
    if variants
        .iter()
        .all(|variant| variant.get("type").and_then(Value::as_str) == Some("object"))
    {
        let mut properties = Map::new();
        for variant in &variants {
            if let Some(variant_properties) = variant.get("properties").and_then(Value::as_object) {
                for (name, property) in variant_properties {
                    match properties.entry(name.clone()) {
                        serde_json::map::Entry::Vacant(entry) => {
                            entry.insert(property.clone());
                        }
                        serde_json::map::Entry::Occupied(mut entry) if entry.get() != property => {
                            entry.insert(Value::Object(Map::new()));
                        }
                        serde_json::map::Entry::Occupied(_) => {}
                    }
                }
            }
        }
        if !properties.is_empty() {
            target
                .entry("properties".to_string())
                .or_insert(Value::Object(properties));
        }
    }
}

fn tool_config(choice: &CanonicalToolChoice) -> Value {
    let mode = match choice {
        CanonicalToolChoice::Auto => "AUTO",
        CanonicalToolChoice::Required | CanonicalToolChoice::Function { .. } => "ANY",
        CanonicalToolChoice::None => "NONE",
    };
    let mut config = json!({"functionCallingConfig": {"mode": mode}});
    if let CanonicalToolChoice::Function { name } = choice {
        config["functionCallingConfig"]["allowedFunctionNames"] = json!([name]);
    }
    config
}

fn generation_config(req: &LlmRequest, settings: &AdapterSettings) -> Map<String, Value> {
    let mut config = Map::new();
    if settings.supports_temperature {
        if let Some(temperature) = req.params.temperature {
            config.insert("temperature".to_string(), json!(temperature));
        }
    }
    if req.params.max_tokens > 0 {
        config.insert("maxOutputTokens".to_string(), json!(req.params.max_tokens));
    }
    if let Some(top_p) = req.params.top_p {
        config.insert("topP".to_string(), json!(top_p));
    }
    if !req.params.stop.is_empty() {
        config.insert("stopSequences".to_string(), json!(req.params.stop));
    }
    if settings.supports_reasoning {
        if let Some(budget) = thinking_budget(&req.reasoning) {
            config.insert(
                "thinkingConfig".to_string(),
                json!({"includeThoughts": true, "thinkingBudget": budget}),
            );
        }
    }
    config
}

fn thinking_budget(reasoning: &ReasoningIntent) -> Option<usize> {
    match reasoning {
        ReasoningIntent::Off | ReasoningIntent::NoReasoning => None,
        ReasoningIntent::Minimal | ReasoningIntent::Low => Some(4096),
        ReasoningIntent::Medium => Some(16384),
        ReasoningIntent::High | ReasoningIntent::XHigh | ReasoningIntent::Max => Some(32768),
        ReasoningIntent::BudgetTokens(budget) => Some(*budget),
    }
}

fn format_api_error(error: &Value) -> String {
    error
        .get("message")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

fn finish_reason(reason: &str) -> String {
    match reason {
        "MAX_TOKENS" => "length".to_string(),
        "STOP" => "stop".to_string(),
        "SAFETY" | "RECITATION" | "BLOCKLIST" => "content_filter".to_string(),
        other => other.to_lowercase(),
    }
}

fn parse_usage(usage: &Value) -> ChatUsage {
    let prompt_tokens = usage
        .get("promptTokenCount")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize;
    let candidates_tokens = usage
        .get("candidatesTokenCount")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize;
    let thoughts_tokens = usage
        .get("thoughtsTokenCount")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize;
    let completion_tokens = candidates_tokens + thoughts_tokens;
    ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens + completion_tokens,
        cache_creation_tokens: None,
        cache_read_tokens: None,
        metering_usd: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_no_forbidden_schema_keys(value: &Value) {
        match value {
            Value::Array(values) => {
                for value in values {
                    assert_no_forbidden_schema_keys(value);
                }
            }
            Value::Object(object) => {
                for key in ["oneOf", "anyOf", "$ref"] {
                    assert!(
                        !object.contains_key(key),
                        "found forbidden schema key {key}"
                    );
                }
                for value in object.values() {
                    assert_no_forbidden_schema_keys(value);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn tool_conversion_normalizes_unions_without_losing_useful_fields() {
        let declarations = convert_tools(&[json!({
            "type": "function",
            "function": {
                "name": "search",
                "description": "Search indexed files",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "mode": {
                            "description": "Search mode",
                            "anyOf": [
                                {"type": "string", "enum": ["literal"]},
                                {"type": "string", "enum": ["regex"]}
                            ]
                        },
                        "limit": {
                            "oneOf": [
                                {"type": "integer", "minimum": 1, "maximum": 100},
                                {"type": "null"}
                            ]
                        }
                    },
                    "required": ["mode"]
                }
            }
        })]);
        let function_declarations = json!({"functionDeclarations": declarations});

        assert_no_forbidden_schema_keys(&function_declarations);
        let declaration = &function_declarations["functionDeclarations"][0];
        assert_eq!(declaration["description"], "Search indexed files");
        assert_eq!(declaration["parameters"]["required"], json!(["mode"]));
        assert_eq!(
            declaration["parameters"]["properties"]["mode"],
            json!({
                "description": "Search mode",
                "type": "string",
                "enum": ["literal", "regex"]
            })
        );
        assert_eq!(
            declaration["parameters"]["properties"]["limit"],
            json!({"type": "integer", "minimum": 1, "maximum": 100})
        );
    }

    #[test]
    fn tool_conversion_inlines_local_refs_and_flattens_object_alternatives() {
        let declarations = convert_tools(&[json!({
            "type": "function",
            "function": {
                "name": "edit",
                "parameters": {
                    "type": "object",
                    "$defs": {
                        "path": {
                            "type": "string",
                            "description": "Workspace-relative path",
                            "minLength": 1
                        }
                    },
                    "properties": {
                        "path": {"$ref": "#/$defs/path"},
                        "change": {
                            "oneOf": [
                                {"type": "object", "properties": {
                                    "replacement": {"type": "string", "description": "New text"}
                                }},
                                {"type": "object", "properties": {
                                    "replacement": {"type": "null"},
                                    "delete": {"type": "boolean"}
                                }}
                            ]
                        }
                    }
                }
            }
        })]);

        assert_no_forbidden_schema_keys(&Value::Array(declarations.clone()));
        let parameters = &declarations[0]["parameters"];
        assert!(parameters.get("$defs").is_none());
        assert_eq!(
            parameters["properties"]["path"],
            json!({
                "type": "string",
                "description": "Workspace-relative path",
                "minLength": 1
            })
        );
        assert_eq!(parameters["properties"]["change"]["type"], "object");
        assert_eq!(
            parameters["properties"]["change"]["properties"]["replacement"],
            json!({})
        );
        assert_eq!(
            parameters["properties"]["change"]["properties"]["delete"]["type"],
            "boolean"
        );
    }
}
