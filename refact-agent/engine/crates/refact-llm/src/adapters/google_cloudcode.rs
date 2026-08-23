use std::collections::HashMap;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

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
                json!({"role": "user", "parts": [{"text": system}]}),
            );
        }

        let tools_allowed =
            settings.supports_tools && !matches!(req.tool_choice, Some(CanonicalToolChoice::None));
        let mut has_tools = false;
        if tools_allowed {
            if let Some(tools) = req.tools.as_ref() {
                let declarations = convert_tools(tools);
                if !declarations.is_empty() {
                    has_tools = true;
                    inner.insert(
                        "tools".to_string(),
                        Value::Array(
                            declarations
                                .into_iter()
                                .map(|declaration| json!({"functionDeclarations": [declaration]}))
                                .collect(),
                        ),
                    );
                }
            }
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
                if !matches!(
                    key.as_str(),
                    "contents"
                        | "systemInstruction"
                        | "tools"
                        | "toolConfig"
                        | "generationConfig"
                        | "sessionId"
                ) {
                    inner.insert(key.clone(), value.clone());
                }
            }
        }
        if has_tools {
            match req.tool_choice.as_ref() {
                Some(CanonicalToolChoice::Required) => {
                    inner.insert(
                        "toolConfig".to_string(),
                        json!({"functionCallingConfig": {"mode": "VALIDATED"}}),
                    );
                }
                Some(CanonicalToolChoice::Function { name }) => {
                    inner.insert(
                        "toolConfig".to_string(),
                        json!({
                            "functionCallingConfig": {
                                "mode": "VALIDATED",
                                "allowedFunctionNames": [name]
                            }
                        }),
                    );
                }
                _ => {}
            }
        }
        let session_id = session_id(&req.messages);
        inner.insert("sessionId".to_string(), json!(session_id));
        let request_id = request_id(&project, &settings.model_name, &inner);

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
                "requestId": request_id,
                "requestType": "agent",
                "userAgent": "antigravity",
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
        if let Some(error) = response.get("error") {
            return Err(StreamParseError::FatalError(format_api_error(error)));
        }
        let mut deltas = Vec::new();
        let mut thinking_blocks = Vec::new();
        let mut replay_parts = Vec::new();

        let mut response_extra = Map::new();
        if let Some(response_id) = json
            .get("responseId")
            .filter(|value| !value.is_null())
            .or_else(|| response.get("responseId"))
            .filter(|value| !value.is_null())
        {
            response_extra.insert("response_id".to_string(), response_id.clone());
        }
        if let Some(model_version) = json
            .get("modelVersion")
            .filter(|value| !value.is_null())
            .or_else(|| response.get("modelVersion"))
            .filter(|value| !value.is_null())
        {
            response_extra.insert("model_version".to_string(), model_version.clone());
        }
        if let Some(prompt_feedback) = response
            .get("promptFeedback")
            .filter(|value| !value.is_null())
        {
            response_extra.insert("prompt_feedback".to_string(), prompt_feedback.clone());
        }

        if let Some(parts) = response
            .get("candidates")
            .and_then(|value| value.as_array())
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(|parts| parts.as_array())
        {
            for (index, part) in parts.iter().enumerate() {
                let replay_part = part.clone();
                if part.get("thought").and_then(|value| value.as_bool()) == Some(true) {
                    if let Some(text) = part.get("text").and_then(|value| value.as_str()) {
                        if !text.is_empty() {
                            deltas.push(LlmStreamDelta::AppendReasoning {
                                text: text.to_string(),
                                block_index: Some(index as u64),
                            });
                        }
                    }
                } else if let Some(text) = part.get("text").and_then(|value| value.as_str()) {
                    if !text.is_empty() {
                        deltas.push(LlmStreamDelta::AppendContent {
                            text: text.to_string(),
                            block_index: Some(index as u64),
                        });
                    }
                }

                if let Some(function_call) = part.get("functionCall") {
                    if let Some(name) = function_call.get("name").and_then(|value| value.as_str()) {
                        let call_id = function_call
                            .get("id")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| synthetic_tool_call_id(part, index));
                        let args = function_call
                            .get("args")
                            .cloned()
                            .unwrap_or_else(|| json!({}));
                        let arguments =
                            serde_json::to_string(&args).unwrap_or_else(|_| "{}".into());
                        let mut tool_call = json!({
                            "index": index,
                            "id": call_id,
                            "type": "function",
                            "function": {"name": name, "arguments": arguments},
                        });
                        let mut extra_content = json!({
                            "google_cloud_code_name": name,
                            "google_cloud_code_had_id": function_call
                                .get("id")
                                .and_then(Value::as_str)
                                .is_some_and(|id| !id.is_empty()),
                        });
                        if let Some(signature) = part
                            .get("thoughtSignature")
                            .and_then(Value::as_str)
                            .filter(|signature| !signature.is_empty())
                        {
                            extra_content["thoughtSignature"] = json!(signature);
                        }
                        tool_call["extra_content"] = extra_content;
                        deltas.push(LlmStreamDelta::FinalizeToolCalls {
                            tool_calls: vec![tool_call],
                        });
                    }
                }

                if let Some(inline_data) = part.get("inlineData") {
                    if inline_data
                        .get("data")
                        .and_then(Value::as_str)
                        .is_some_and(|data| !data.is_empty())
                    {
                        deltas.push(LlmStreamDelta::AddServerContentBlock {
                            block: json!({
                                "type": "google_cloud_code_inline_data",
                                "mime_type": inline_data
                                    .get("mimeType")
                                    .and_then(Value::as_str)
                                    .unwrap_or("application/octet-stream"),
                                "data": inline_data.get("data").cloned().unwrap_or(Value::Null),
                            }),
                        });
                    }
                }

                if let Some(signature) = part
                    .get("thoughtSignature")
                    .and_then(|value| value.as_str())
                {
                    if part.get("functionCall").is_none() {
                        if part.get("thought").and_then(Value::as_bool) == Some(true) {
                            thinking_blocks.push(json!({
                                "id": signature_block_id(signature),
                                "index": index,
                                "type": "thinking",
                                "provider": "google_cloud_code",
                                "thinking": part
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .unwrap_or(""),
                                "signature": signature,
                            }));
                        }
                    }
                }
                replay_parts.push(replay_part);
            }
        }

        if !thinking_blocks.is_empty() {
            deltas.push(LlmStreamDelta::SetThinkingBlocks {
                blocks: thinking_blocks,
            });
        }

        let first_candidate = response
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|candidates| candidates.first());
        if let Some(candidate) = first_candidate {
            append_grounding_deltas(candidate, &mut deltas, &mut response_extra);
            if let Some(safety_ratings) = candidate
                .get("safetyRatings")
                .filter(|value| !value.is_null())
            {
                response_extra.insert("safety_ratings".to_string(), safety_ratings.clone());
            }
        }

        if let Some(usage) = response.get("usageMetadata") {
            deltas.push(LlmStreamDelta::SetUsage {
                usage: parse_usage(usage),
            });
            response_extra.insert("usage_metadata".to_string(), usage.clone());
        }
        if let Some(candidates) = response.get("candidates").and_then(Value::as_array) {
            if candidates.len() > 1 {
                response_extra.insert(
                    "additional_candidates".to_string(),
                    json!(candidates.len() - 1),
                );
            }
        }
        if !replay_parts.is_empty() {
            let mut extra = Map::new();
            extra.insert(
                "_google_cloud_code_parts".to_string(),
                Value::Array(replay_parts),
            );
            deltas.push(LlmStreamDelta::MergeExtra { extra });
        }
        if !response_extra.is_empty() {
            let mut extra = Map::new();
            extra.insert(
                "google_cloud_code".to_string(),
                Value::Object(response_extra),
            );
            deltas.push(LlmStreamDelta::MergeExtra { extra });
        }

        let mut is_finished = false;
        if let Some(reason) = first_candidate
            .and_then(|candidate| candidate.get("finishReason"))
            .and_then(Value::as_str)
            .filter(|reason| *reason != "FINISH_REASON_UNSPECIFIED")
        {
            deltas.push(LlmStreamDelta::SetFinishReason {
                reason: finish_reason(reason),
            });
            is_finished = true;
        } else if response
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
            .is_some()
        {
            deltas.push(LlmStreamDelta::SetFinishReason {
                reason: "content_filter".to_string(),
            });
            is_finished = true;
        }
        if is_finished {
            deltas.push(LlmStreamDelta::Done);
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
                        let cloud_code_name = call
                            .extra_content
                            .as_ref()
                            .and_then(|extra| extra.get("google_cloud_code_name"))
                            .and_then(Value::as_str)
                            .unwrap_or(&call.function.name);
                        call_names.insert(call.id.clone(), cloud_code_name.to_string());
                    }
                }
            }
        }

        let (role, parts) = match message.role.as_str() {
            "user" => ("user", content_parts(&message.content)),
            "assistant" => {
                let parts = if let Some(parts) = exact_cloud_code_parts(message) {
                    parts
                } else {
                    let mut parts = signed_thinking_parts(message);
                    parts.extend(content_parts(&message.content));
                    if let Some(calls) = message.tool_calls.as_ref() {
                        for call in calls {
                            if call.function.name.is_empty() {
                                continue;
                            }
                            let cloud_code_name = call
                                .extra_content
                                .as_ref()
                                .and_then(|extra| extra.get("google_cloud_code_name"))
                                .and_then(Value::as_str)
                                .unwrap_or(&call.function.name);
                            let mut part = json!({
                                "functionCall": {
                                    "id": call.id,
                                    "name": cloud_code_name,
                                    "args": parse_arguments(&call.function.arguments),
                                }
                            });
                            if let Some(signature) = call
                                .extra_content
                                .as_ref()
                                .and_then(|extra| extra.get("thoughtSignature"))
                                .and_then(Value::as_str)
                            {
                                part["thoughtSignature"] = json!(signature);
                            }
                            parts.push(part);
                        }
                    }
                    parts
                };
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
                            "id": message.tool_call_id,
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

fn signed_thinking_parts(message: &ChatMessage) -> Vec<Value> {
    message
        .thinking_blocks
        .as_ref()
        .into_iter()
        .flatten()
        .filter_map(|block| {
            if block.get("type").and_then(Value::as_str) != Some("thinking") {
                return None;
            }
            if block.get("provider").and_then(Value::as_str) != Some("google_cloud_code") {
                return None;
            }
            let signature = block.get("signature").and_then(Value::as_str)?;
            if signature.is_empty() {
                return None;
            }
            Some(json!({
                "text": block.get("thinking").and_then(Value::as_str).unwrap_or(""),
                "thought": true,
                "thoughtSignature": signature,
            }))
        })
        .collect()
}

fn exact_cloud_code_parts(message: &ChatMessage) -> Option<Vec<Value>> {
    let parts = message
        .extra
        .get("_google_cloud_code_parts")
        .and_then(Value::as_array)
        .filter(|parts| !parts.is_empty())?;
    exact_parts_match_message(parts, message).then(|| parts.clone())
}

fn exact_parts_match_message(parts: &[Value], message: &ChatMessage) -> bool {
    let provider_text = parts
        .iter()
        .filter(|part| part.get("thought").and_then(Value::as_bool) != Some(true))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<String>();
    if provider_text != message.content.content_text_only() {
        return false;
    }

    let provider_thinking = parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.get("thought").and_then(Value::as_bool) == Some(true))
        .filter_map(|(index, part)| {
            Some((
                index,
                part.get("text").and_then(Value::as_str).unwrap_or(""),
                part.get("thoughtSignature")?.as_str()?,
            ))
        })
        .collect::<Vec<_>>();
    let current_thinking = message
        .thinking_blocks
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|block| {
            block.get("provider").and_then(Value::as_str) == Some("google_cloud_code")
                && block.get("type").and_then(Value::as_str) == Some("thinking")
        })
        .filter_map(|block| {
            Some((
                block.get("index")?.as_u64()? as usize,
                block.get("thinking").and_then(Value::as_str).unwrap_or(""),
                block.get("signature")?.as_str()?,
            ))
        })
        .collect::<Vec<_>>();
    if provider_thinking != current_thinking {
        return false;
    }

    let provider_inline_data = parts
        .iter()
        .filter_map(|part| part.get("inlineData"))
        .filter_map(|inline| {
            Some((
                inline.get("mimeType")?.as_str()?,
                inline.get("data")?.as_str()?,
            ))
        })
        .collect::<Vec<_>>();
    let current_inline_data = message
        .server_content_blocks
        .iter()
        .filter(|block| {
            block.get("type").and_then(Value::as_str) == Some("google_cloud_code_inline_data")
        })
        .filter_map(|block| {
            Some((
                block.get("mime_type")?.as_str()?,
                block.get("data")?.as_str()?,
            ))
        })
        .collect::<Vec<_>>();
    if provider_inline_data != current_inline_data {
        return false;
    }

    let provider_calls = parts
        .iter()
        .enumerate()
        .filter_map(|(index, part)| {
            let call = part.get("functionCall")?;
            let had_id = call
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty());
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| synthetic_tool_call_id(part, index));
            Some((
                id,
                had_id,
                call.get("name")?.as_str()?,
                call.get("args").cloned().unwrap_or_else(|| json!({})),
            ))
        })
        .collect::<Vec<_>>();
    let current_calls = message.tool_calls.as_deref().unwrap_or_default();
    if provider_calls.len() != current_calls.len() {
        return false;
    }
    provider_calls.iter().zip(current_calls).all(
        |((provider_id, provider_had_id, provider_name, provider_args), current)| {
            let current_name = current
                .extra_content
                .as_ref()
                .and_then(|extra| extra.get("google_cloud_code_name"))
                .and_then(Value::as_str)
                .unwrap_or(&current.function.name);
            let current_had_id = current
                .extra_content
                .as_ref()
                .and_then(|extra| extra.get("google_cloud_code_had_id"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
            provider_id == &current.id
                && provider_had_id == &current_had_id
                && *provider_name == current_name
                && *provider_args == parse_arguments(&current.function.arguments)
        },
    )
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

fn synthetic_tool_call_id(function_call: &Value, part_index: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(part_index.to_be_bytes());
    hasher.update(serde_json::to_vec(function_call).unwrap_or_default());
    let digest = hasher.finalize();
    let uuid = uuid::Uuid::from_slice(&digest[..16]).expect("SHA-256 prefix has UUID length");
    format!("call_{}", uuid.simple())
}

fn signature_block_id(signature: &str) -> String {
    let digest = Sha256::digest(signature.as_bytes());
    let suffix = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("gcc_sig_{suffix}")
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

fn session_id(messages: &[ChatMessage]) -> String {
    let first_user_content = messages
        .iter()
        .find(|message| message.role == "user")
        .map(|message| {
            let mut content = message.message_id.as_bytes().to_vec();
            content.push(0);
            content.extend(serde_json::to_vec(&message.content).unwrap_or_default());
            content
        })
        .filter(|content| !content.is_empty())
        .unwrap_or_else(|| b"empty-cloud-code-session".to_vec());

    let digest = Sha256::digest(first_user_content);
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    format!("-{}", u64::from_be_bytes(bytes) & i64::MAX as u64)
}

fn request_id(project: &str, model: &str, inner: &Map<String, Value>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(project.as_bytes());
    hasher.update([0]);
    hasher.update(model.as_bytes());
    hasher.update([0]);
    hasher.update(serde_json::to_vec(inner).unwrap_or_default());
    let digest = hasher.finalize();
    let suffix = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("agent/{suffix}")
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
            if object.contains_key("required") && !object.contains_key("properties") {
                object.remove("required");
            }
            let property_names =
                object
                    .get("properties")
                    .and_then(Value::as_object)
                    .map(|properties| {
                        properties
                            .keys()
                            .cloned()
                            .collect::<std::collections::HashSet<_>>()
                    });
            let remove_required = match (
                property_names.as_ref(),
                object.get_mut("required").and_then(Value::as_array_mut),
            ) {
                (Some(property_names), Some(required)) => {
                    required.retain(|name| {
                        name.as_str()
                            .is_some_and(|name| property_names.contains(name))
                    });
                    required.is_empty()
                }
                _ => false,
            };
            if remove_required {
                object.remove("required");
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
        match thinking_budget(&req.reasoning) {
            Some(budget) => {
                config.insert(
                    "thinkingConfig".to_string(),
                    json!({"includeThoughts": true, "thinkingBudget": budget}),
                );
            }
            None if matches!(req.reasoning, ReasoningIntent::NoReasoning) => {
                config.insert(
                    "thinkingConfig".to_string(),
                    json!({"includeThoughts": false, "thinkingBudget": 0}),
                );
            }
            None => {}
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
        .unwrap_or_else(|| {
            let text = error.to_string();
            text.chars().take(512).collect()
        })
}

fn append_grounding_deltas(
    candidate: &Value,
    deltas: &mut Vec<LlmStreamDelta>,
    response_extra: &mut Map<String, Value>,
) {
    if let Some(citation_metadata) = candidate
        .get("citationMetadata")
        .filter(|value| !value.is_null())
    {
        if let Some(sources) = citation_metadata
            .get("citationSources")
            .and_then(Value::as_array)
        {
            for source in sources {
                if !source.is_null() {
                    let url = source
                        .get("uri")
                        .or_else(|| source.get("url"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if url.is_empty() {
                        continue;
                    }
                    deltas.push(LlmStreamDelta::AddCitation {
                        citation: json!({
                            "type": "url_citation",
                            "url": url,
                            "title": source
                                .get("title")
                                .and_then(Value::as_str)
                                .unwrap_or("Source"),
                            "provider": "google_cloud_code",
                            "start_index": source.get("startIndex").cloned(),
                            "end_index": source.get("endIndex").cloned(),
                        }),
                    });
                }
            }
        }
    }

    if let Some(grounding) = candidate
        .get("groundingMetadata")
        .filter(|value| !value.is_null())
    {
        response_extra.insert("grounding_metadata".to_string(), grounding.clone());
        if let Some(chunks) = grounding.get("groundingChunks").and_then(Value::as_array) {
            for chunk in chunks {
                let Some(web) = chunk.get("web") else {
                    continue;
                };
                let Some(url) = web.get("uri").and_then(Value::as_str) else {
                    continue;
                };
                if url.is_empty() {
                    continue;
                }
                deltas.push(LlmStreamDelta::AddCitation {
                    citation: json!({
                        "type": "url_citation",
                        "url": url,
                        "title": web.get("title").and_then(Value::as_str).unwrap_or("Source"),
                        "provider": "google_cloud_code",
                    }),
                });
            }
        }
    }

    for key in ["urlContextMetadata", "groundingAttributions"] {
        if let Some(value) = candidate.get(key).filter(|value| !value.is_null()) {
            response_extra.insert(key.to_string(), value.clone());
        }
    }
}

fn finish_reason(reason: &str) -> String {
    match reason {
        "MAX_TOKENS" => "length".to_string(),
        "STOP" => "stop".to_string(),
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => {
            "content_filter".to_string()
        }
        "MALFORMED_FUNCTION_CALL" | "UNEXPECTED_TOOL_CALL" => "tool_error".to_string(),
        other => other.to_lowercase(),
    }
}

fn parse_usage(usage: &Value) -> ChatUsage {
    let prompt_with_cache = usage
        .get("promptTokenCount")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize;
    let cache_read_tokens = usage
        .get("cachedContentTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let prompt_tokens = prompt_with_cache.saturating_sub(cache_read_tokens);
    let candidates_tokens = usage
        .get("candidatesTokenCount")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize;
    let thoughts_tokens = usage
        .get("thoughtsTokenCount")
        .and_then(|value| value.as_u64())
        .unwrap_or(0) as usize;
    let completion_tokens = candidates_tokens + thoughts_tokens;
    let total_tokens = usage
        .get("totalTokenCount")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(prompt_with_cache + completion_tokens);
    ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_creation_tokens: None,
        cache_read_tokens: (cache_read_tokens > 0).then_some(cache_read_tokens),
        metering_usd: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::CanonicalToolChoice;
    use refact_core::chat_types::{ChatToolCall, ChatToolFunction};

    fn cloudcode_settings(supports_tools: bool, auth_token: &str) -> AdapterSettings {
        let mut extra_headers = HashMap::new();
        extra_headers.insert(PROJECT_HEADER.to_string(), "cloud-project-123".to_string());
        AdapterSettings {
            api_key: String::new(),
            auth_token: auth_token.to_string(),
            endpoint: "https://daily-cloudcode-pa.googleapis.com".to_string(),
            extra_headers,
            model_name: "gemini-2.5-pro".to_string(),
            supports_tools,
            supports_reasoning: false,
            reasoning_type: None,
            supports_temperature: false,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            supports_web_search: false,
            supports_cache_control: false,
        }
    }

    #[test]
    fn build_http_uses_cloudcode_envelope_with_request_attribution() {
        let auth_token = "oauth-token-must-not-be-in-body";
        let request = LlmRequest::new(
            "google/ignored-request-model".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        let settings = cloudcode_settings(false, auth_token);

        let repeated_request = request.clone();
        let http = GoogleCloudCodeAdapter
            .build_http(&refact_privacy::testing::cleared(request), &settings)
            .unwrap();
        let repeated_http = GoogleCloudCodeAdapter
            .build_http(
                &refact_privacy::testing::cleared(repeated_request),
                &settings,
            )
            .unwrap();

        assert_eq!(
            http.url,
            "https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse"
        );
        let request_id = http.body["requestId"].as_str().unwrap();
        assert!(request_id.starts_with("agent/"));
        assert_eq!(request_id.len(), "agent/".len() + 32);
        assert_eq!(request_id, repeated_http.body["requestId"]);
        assert_eq!(http.body["requestType"], "agent");
        assert_eq!(http.body["userAgent"], "antigravity");
        assert_eq!(http.body["model"], "gemini-2.5-pro");
        assert_eq!(http.body["project"], "cloud-project-123");
        assert_eq!(
            http.body["request"]["contents"],
            json!([{"role": "user", "parts": [{"text": "Hello"}]}])
        );
        let first_session_id = http.body["request"]["sessionId"].as_str().unwrap();
        assert!(first_session_id.starts_with('-'));
        assert!(first_session_id[1..].parse::<u64>().is_ok());
        assert!(!first_session_id.contains("Hello"));
        assert_eq!(
            first_session_id,
            repeated_http.body["request"]["sessionId"].as_str().unwrap()
        );
        assert!(http.body["request"].get("tools").is_none());
        assert!(http.body["request"].get("toolConfig").is_none());
        assert!(!http.body.to_string().contains(auth_token));
    }

    #[test]
    fn build_http_groups_tools_like_the_official_cli_without_tool_config() {
        let request = LlmRequest::new(
            "google/ignored-request-model".to_string(),
            vec![ChatMessage::new("user".to_string(), "Find it".to_string())],
        )
        .with_tools(
            vec![
                json!({
                    "type": "function",
                    "function": {
                        "name": "search",
                        "parameters": {"type": "object"}
                    }
                }),
                json!({
                    "type": "function",
                    "function": {
                        "name": "read",
                        "parameters": {"type": "object"}
                    }
                }),
            ],
            Some(CanonicalToolChoice::Auto),
        );

        let http = GoogleCloudCodeAdapter
            .build_http(
                &refact_privacy::testing::cleared(request),
                &cloudcode_settings(true, "oauth-token"),
            )
            .unwrap();

        let tools = http.body["request"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(
            tools[0]["functionDeclarations"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            tools[1]["functionDeclarations"].as_array().unwrap().len(),
            1
        );
        assert!(http.body["request"].get("toolConfig").is_none());
    }

    #[test]
    fn build_http_honors_tool_choice_and_protects_generated_fields() {
        let tool = json!({
            "type": "function",
            "function": {"name": "search", "parameters": {"type": "object"}}
        });
        let mut protected = HashMap::new();
        protected.insert(
            "contents".to_string(),
            json!([{"role": "user", "parts": []}]),
        );
        protected.insert("sessionId".to_string(), json!("attacker-session"));
        let none_request = LlmRequest::new(
            "google/model".to_string(),
            vec![ChatMessage::new("user".to_string(), "hello".to_string())],
        )
        .with_tools(vec![tool.clone()], Some(CanonicalToolChoice::None))
        .with_extra_body(protected);
        let none_http = GoogleCloudCodeAdapter
            .build_http(
                &refact_privacy::testing::cleared(none_request),
                &cloudcode_settings(true, "oauth-token"),
            )
            .unwrap();
        assert!(none_http.body["request"].get("tools").is_none());
        assert_eq!(
            none_http.body["request"]["contents"][0]["parts"][0]["text"],
            "hello"
        );
        assert_ne!(none_http.body["request"]["sessionId"], "attacker-session");

        let named_request = LlmRequest::new(
            "google/model".to_string(),
            vec![ChatMessage::new("user".to_string(), "hello".to_string())],
        )
        .with_tools(
            vec![tool],
            Some(CanonicalToolChoice::Function {
                name: "search".to_string(),
            }),
        );
        let named_http = GoogleCloudCodeAdapter
            .build_http(
                &refact_privacy::testing::cleared(named_request),
                &cloudcode_settings(true, "oauth-token"),
            )
            .unwrap();
        assert_eq!(
            named_http.body["request"]["toolConfig"]["functionCallingConfig"]["mode"],
            "VALIDATED"
        );
        assert_eq!(
            named_http.body["request"]["toolConfig"]["functionCallingConfig"]
                ["allowedFunctionNames"],
            json!(["search"])
        );
    }

    #[test]
    fn session_ids_are_stable_per_message_and_distinct_across_chats() {
        let mut first = ChatMessage::new("user".to_string(), "hello".to_string());
        first.message_id = "chat-a-first".to_string();
        let mut second = ChatMessage::new("user".to_string(), "hello".to_string());
        second.message_id = "chat-b-first".to_string();

        let first_id = session_id(&[first.clone()]);
        assert_eq!(first_id, session_id(&[first]));
        assert_ne!(first_id, session_id(&[second]));
    }

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

    #[test]
    fn tool_conversion_prunes_nested_required_fields_without_properties() {
        let declarations = convert_tools(&[json!({
            "type": "function",
            "function": {
                "name": "batch",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "request": {
                            "type": "object",
                            "properties": {
                                "steps": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "items": {
                                                "type": "array",
                                                "items": {
                                                    "type": "object",
                                                    "properties": {
                                                        "kept": {"type": "string"}
                                                    },
                                                    "required": ["missing_a", "kept", "missing_b"]
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        })]);

        let item_schema = &declarations[0]["parameters"]["properties"]["request"]["properties"]
            ["steps"]["items"]["properties"]["items"]["items"];
        assert_eq!(item_schema["required"], json!(["kept"]));
    }

    #[test]
    fn tool_conversion_removes_required_when_properties_are_absent() {
        let declarations = convert_tools(&[json!({
            "type": "function",
            "function": {
                "name": "broken",
                "parameters": {
                    "required": ["missing"],
                    "items": {
                        "type": ["object", "null"],
                        "required": ["also_missing"]
                    }
                }
            }
        })]);

        assert!(declarations[0]["parameters"].get("required").is_none());
        assert!(declarations[0]["parameters"]["items"]
            .get("required")
            .is_none());
    }

    #[test]
    fn parse_stream_finish_reason_emits_done() {
        let chunk = json!({
            "response": {
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{"text": "Hello"}]
                    },
                    "finishReason": "STOP"
                }],
                "usageMetadata": {
                    "promptTokenCount": 3,
                    "candidatesTokenCount": 1
                }
            }
        })
        .to_string();

        let deltas = GoogleCloudCodeAdapter.parse_stream_chunk(&chunk).unwrap();

        assert!(deltas.iter().any(
            |delta| matches!(delta, LlmStreamDelta::SetFinishReason { reason } if reason == "stop")
        ));
        assert!(matches!(deltas.last(), Some(LlmStreamDelta::Done)));
    }

    #[test]
    fn parse_stream_preserves_reasoning_tools_images_grounding_usage_and_metadata() {
        let chunk = json!({
            "responseId": "resp-123",
            "modelVersion": "gemini-3.6-flash-high",
            "response": {
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [
                            {
                                "text": "private reasoning",
                                "thought": true,
                                "thoughtSignature": "sig-reasoning"
                            },
                            {"text": "answer"},
                            {"inlineData": {"mimeType": "image/png", "data": "aW1n"}},
                            {
                                "functionCall": {
                                    "id": "call-upstream",
                                    "name": "search",
                                    "args": {"query": "rust"}
                                },
                                "thoughtSignature": "sig-tool"
                            }
                        ]
                    },
                    "finishReason": "STOP",
                    "safetyRatings": [{"category": "safe"}],
                    "citationMetadata": {
                        "citationSources": [{"uri": "https://citation.test", "startIndex": 0}]
                    },
                    "groundingMetadata": {
                        "webSearchQueries": ["rust"],
                        "groundingChunks": [{
                            "web": {"uri": "https://grounding.test", "title": "Grounding"}
                        }]
                    }
                }],
                "usageMetadata": {
                    "promptTokenCount": 100,
                    "cachedContentTokenCount": 20,
                    "candidatesTokenCount": 5,
                    "thoughtsTokenCount": 3,
                    "totalTokenCount": 108,
                    "candidatesTokensDetails": [{"modality": "IMAGE", "tokenCount": 2}]
                }
            }
        })
        .to_string();

        let deltas = GoogleCloudCodeAdapter.parse_stream_chunk(&chunk).unwrap();

        assert!(deltas.iter().any(|delta| matches!(
            delta,
            LlmStreamDelta::AppendReasoning { text, block_index: Some(0) }
                if text == "private reasoning"
        )));
        assert!(deltas.iter().any(|delta| matches!(
            delta,
            LlmStreamDelta::AddServerContentBlock { block }
                if block["type"] == "google_cloud_code_inline_data"
                    && block["mime_type"] == "image/png"
                    && block["data"] == "aW1n"
        )));
        let tool_call = deltas
            .iter()
            .find_map(|delta| match delta {
                LlmStreamDelta::FinalizeToolCalls { tool_calls } => tool_calls.first(),
                _ => None,
            })
            .unwrap();
        assert_eq!(tool_call["id"], "call-upstream");
        assert_eq!(tool_call["function"]["name"], "search");
        assert_eq!(tool_call["extra_content"]["thoughtSignature"], "sig-tool");
        let thinking = deltas
            .iter()
            .find_map(|delta| match delta {
                LlmStreamDelta::SetThinkingBlocks { blocks } => blocks.first(),
                _ => None,
            })
            .unwrap();
        assert_eq!(thinking["index"], 0);
        assert_eq!(thinking["provider"], "google_cloud_code");
        assert_eq!(thinking["signature"], "sig-reasoning");
        assert!(thinking["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("gcc_sig_")));
        assert_eq!(
            deltas
                .iter()
                .filter(|delta| matches!(delta, LlmStreamDelta::AddCitation { .. }))
                .count(),
            2
        );
        let usage = deltas
            .iter()
            .find_map(|delta| match delta {
                LlmStreamDelta::SetUsage { usage } => Some(usage),
                _ => None,
            })
            .unwrap();
        assert_eq!(usage.prompt_tokens, 80);
        assert_eq!(usage.completion_tokens, 8);
        assert_eq!(usage.cache_read_tokens, Some(20));
        assert_eq!(usage.total_tokens, 108);
        let extra = deltas
            .iter()
            .find_map(|delta| match delta {
                LlmStreamDelta::MergeExtra { extra } if extra.contains_key("google_cloud_code") => {
                    Some(extra)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(extra["google_cloud_code"]["response_id"], "resp-123");
        assert_eq!(
            extra["google_cloud_code"]["model_version"],
            "gemini-3.6-flash-high"
        );
        assert!(extra["google_cloud_code"]["grounding_metadata"].is_object());
        let replay_extra = deltas
            .iter()
            .filter_map(|delta| match delta {
                LlmStreamDelta::MergeExtra { extra } => extra
                    .get("_google_cloud_code_parts")
                    .and_then(Value::as_array),
                _ => None,
            })
            .next()
            .unwrap();
        assert_eq!(replay_extra.len(), 4);
        assert_eq!(replay_extra[0]["thoughtSignature"], "sig-reasoning");
        assert_eq!(replay_extra[1]["text"], "answer");
        assert_eq!(replay_extra[3]["functionCall"]["id"], "call-upstream");
        assert_eq!(replay_extra[3]["thoughtSignature"], "sig-tool");
        assert!(matches!(deltas.last(), Some(LlmStreamDelta::Done)));
    }

    #[test]
    fn convert_messages_replays_exact_provider_part_order() {
        let parts = json!([
            {"text": "first"},
            {"text": "reason", "thought": true, "thoughtSignature": "sig-r"},
            {
                "functionCall": {"id": "call-1", "name": "search", "args": {"q": "x"}},
                "thoughtSignature": "sig-t"
            },
            {"text": "after"}
        ]);
        let mut message = ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("firstafter".to_string()),
            thinking_blocks: Some(vec![json!({
                "id": signature_block_id("sig-r"),
                "index": 1,
                "type": "thinking",
                "provider": "google_cloud_code",
                "thinking": "reason",
                "signature": "sig-r"
            })]),
            tool_calls: Some(vec![ChatToolCall {
                id: "call-1".to_string(),
                index: Some(2),
                function: ChatToolFunction {
                    name: "search".to_string(),
                    arguments: r#"{"q":"x"}"#.to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: Some(json!({
                    "google_cloud_code_name": "search",
                    "google_cloud_code_had_id": true,
                    "thoughtSignature": "sig-t"
                })),
                started_at_ms: None,
                completed_at_ms: None,
            }]),
            ..Default::default()
        };
        message
            .extra
            .insert("_google_cloud_code_parts".to_string(), parts.clone());

        let (_, contents) = convert_messages(&[message]);

        assert_eq!(contents[0]["parts"], parts);
    }

    #[test]
    fn convert_messages_rejects_stale_exact_parts_after_tool_removal() {
        let parts = json!([
            {"text": "working"},
            {"functionCall": {"id": "call-1", "name": "search", "args": {"q": "x"}}}
        ]);
        let mut message = ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("working".to_string()),
            tool_calls: None,
            ..Default::default()
        };
        message
            .extra
            .insert("_google_cloud_code_parts".to_string(), parts);

        let (_, contents) = convert_messages(&[message]);

        assert_eq!(contents[0]["parts"], json!([{"text": "working"}]));
    }

    #[test]
    fn synthetic_function_call_ids_are_stable() {
        let function_call = json!({"name": "search", "args": {"q": "rust"}});
        let first = synthetic_tool_call_id(&function_call, 0);
        let second = synthetic_tool_call_id(&function_call, 0);
        let parallel = synthetic_tool_call_id(&function_call, 1);

        assert_eq!(first, second);
        assert_ne!(first, parallel);
        assert!(first.starts_with("call_"));
    }

    #[test]
    fn generation_config_explicitly_disables_reasoning_when_requested() {
        let mut settings = cloudcode_settings(false, "oauth-token");
        settings.supports_reasoning = true;
        let request = LlmRequest::new("google/model".to_string(), Vec::new())
            .with_reasoning(ReasoningIntent::NoReasoning);

        let config = generation_config(&request, &settings);

        assert_eq!(config["thinkingConfig"]["includeThoughts"], false);
        assert_eq!(config["thinkingConfig"]["thinkingBudget"], 0);
    }

    #[test]
    fn convert_messages_replays_signed_reasoning_and_tool_parts() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Search".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("working".to_string()),
                reasoning_content: Some("reasoning".to_string()),
                thinking_blocks: Some(vec![json!({
                    "index": 0,
                    "type": "thinking",
                    "provider": "google_cloud_code",
                    "thinking": "reasoning",
                    "signature": "sig-reasoning"
                })]),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call-1".to_string(),
                    index: Some(2),
                    function: ChatToolFunction {
                        name: "search".to_string(),
                        arguments: r#"{"query":"rust"}"#.to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: Some(json!({"thoughtSignature": "sig-tool"})),
                    started_at_ms: None,
                    completed_at_ms: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                tool_call_id: "call-1".to_string(),
                content: ChatContent::SimpleText("result".to_string()),
                ..Default::default()
            },
        ];

        let (_, contents) = convert_messages(&messages);
        let model_parts = contents[1]["parts"].as_array().unwrap();
        assert_eq!(model_parts[0]["thought"], true);
        assert_eq!(model_parts[0]["thoughtSignature"], "sig-reasoning");
        let function_part = model_parts
            .iter()
            .find(|part| part.get("functionCall").is_some())
            .unwrap();
        assert_eq!(function_part["functionCall"]["id"], "call-1");
        assert_eq!(function_part["thoughtSignature"], "sig-tool");
        let response_part = &contents[2]["parts"][0]["functionResponse"];
        assert_eq!(response_part["id"], "call-1");
        assert_eq!(response_part["name"], "search");
    }

    #[test]
    fn convert_messages_does_not_invent_text_signature_replay_without_exact_parts() {
        let message = ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("visible answer".to_string()),
            thinking_blocks: Some(vec![json!({
                "index": 1,
                "type": "google_cloud_code_signature",
                "provider": "google_cloud_code",
                "signature": "sig-after-text"
            })]),
            ..Default::default()
        };

        let (_, contents) = convert_messages(&[message]);
        let parts = contents[0]["parts"].as_array().unwrap();
        assert_eq!(parts, &vec![json!({"text": "visible answer"})]);
    }

    #[test]
    fn parse_stream_handles_blocked_and_nested_error_payloads() {
        let blocked = json!({
            "response": {
                "promptFeedback": {"blockReason": "PROHIBITED_CONTENT"}
            }
        })
        .to_string();
        let deltas = GoogleCloudCodeAdapter.parse_stream_chunk(&blocked).unwrap();
        assert!(deltas.iter().any(|delta| matches!(
            delta,
            LlmStreamDelta::SetFinishReason { reason } if reason == "content_filter"
        )));
        assert!(matches!(deltas.last(), Some(LlmStreamDelta::Done)));

        let nested_error = json!({
            "response": {"error": {"message": "nested failure"}}
        })
        .to_string();
        assert!(matches!(
            GoogleCloudCodeAdapter.parse_stream_chunk(&nested_error),
            Err(StreamParseError::FatalError(message)) if message == "nested failure"
        ));
    }

    #[test]
    fn finish_reason_maps_google_terminal_variants() {
        assert_eq!(finish_reason("STOP"), "stop");
        assert_eq!(finish_reason("MAX_TOKENS"), "length");
        assert_eq!(finish_reason("PROHIBITED_CONTENT"), "content_filter");
        assert_eq!(finish_reason("SPII"), "content_filter");
        assert_eq!(finish_reason("MALFORMED_FUNCTION_CALL"), "tool_error");
        assert_eq!(finish_reason("UNEXPECTED_TOOL_CALL"), "tool_error");
    }
}
