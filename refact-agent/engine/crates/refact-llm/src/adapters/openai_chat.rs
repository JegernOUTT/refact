use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};

use refact_core::chat_types::ChatUsage;
use refact_privacy::Cleared;
use crate::adapter::{
    AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError, extract_extra_fields,
    insert_extra_headers,
};
use crate::canonical::{CanonicalToolChoice, LlmRequest, LlmStreamDelta, ResponseFormat};
use crate::params::CacheControl;

const PROTECTED_FIELDS: &[&str] = &[
    "model",
    "messages",
    "stream",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "stream_options",
    "cache_control",
    "top_p",
];

const BASE_PROVIDER_HEADER: &str = "x-refact-internal-base-provider";

pub struct OpenAiChatAdapter;

fn normalize_openai_tool_call_delta(tc: &Value, fallback_index: usize) -> Option<Value> {
    let index = tc
        .get("index")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(fallback_index);

    let mut out = json!({
        "index": index,
        "type": tc.get("type").cloned().unwrap_or_else(|| json!("function")),
        "function": {},
    });

    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
        if !id.is_empty() {
            out["id"] = json!(id);
        }
    }

    let func = tc.get("function")?;
    if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
        if !name.is_empty() {
            out["function"]["name"] = json!(name);
        }
    }

    if let Some(arguments) = func.get("arguments") {
        if !arguments.is_null() {
            out["function"]["arguments"] = arguments.clone();
        }
    }

    if out["function"]
        .as_object()
        .map(|o| o.is_empty())
        .unwrap_or(true)
        && out.get("id").is_none()
    {
        return None;
    }

    Some(out)
}

fn normalize_legacy_function_call_delta(fc: &Value) -> Option<Value> {
    let mut out = json!({
        "index": 0,
        "type": "function",
        "function": {},
    });

    if let Some(name) = fc.get("name").and_then(|v| v.as_str()) {
        if !name.is_empty() {
            out["function"]["name"] = json!(name);
        }
    }

    if let Some(arguments) = fc.get("arguments") {
        if !arguments.is_null() {
            out["function"]["arguments"] = arguments.clone();
        }
    }

    if out["function"]
        .as_object()
        .map(|o| o.is_empty())
        .unwrap_or(true)
    {
        return None;
    }

    Some(out)
}

impl LlmWireAdapter for OpenAiChatAdapter {
    fn build_http(
        &self,
        req: &Cleared<LlmRequest>,
        settings: &AdapterSettings,
    ) -> Result<HttpParts, String> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !settings.api_key.is_empty() {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", settings.api_key))
                    .map_err(|e| format!("invalid api_key for header: {e}"))?,
            );
        }
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&format!("refact-lsp {}", env!("CARGO_PKG_VERSION")))
                .unwrap_or_else(|_| HeaderValue::from_static("refact-lsp")),
        );

        insert_extra_headers(&mut headers, &settings.extra_headers);
        crate::provider_quirks::apply_github_copilot_request_headers(&mut headers, req, settings);

        let litellm = is_litellm(settings);
        let mut messages = convert_messages_to_openai(&req.messages, litellm);

        // OpenRouter Anthropic targets use provider automatic caching via the top-level field only.
        let use_top_level_cache_control = matches!(req.cache_control, CacheControl::Ephemeral)
            && settings.supports_cache_control
            && !litellm
            && is_openrouter_anthropic_model(settings);

        // LiteLLM requires explicit message block markers. Preserve the existing generic
        // strategy for non-Anthropic OpenAI-compatible endpoints.
        if matches!(req.cache_control, CacheControl::Ephemeral)
            && settings.supports_cache_control
            && !use_top_level_cache_control
            && (!is_anthropic_family_model(settings) || litellm)
        {
            if litellm {
                inject_cache_control_bounded(&mut messages, 4);
            } else {
                inject_cache_control(&mut messages);
            }
        }

        let mut body = json!({
            "model": settings.model_name,
            "messages": messages,
            "stream": req.stream,
        });

        if use_top_level_cache_control {
            body["cache_control"] = json!({"type": "ephemeral", "ttl": "1h"});
        }

        if settings.supports_max_completion_tokens {
            body["max_completion_tokens"] = json!(req.params.max_tokens);
        } else {
            body["max_tokens"] = json!(req.params.max_tokens);
        }

        if req.stream {
            body["stream_options"] = json!({"include_usage": true});
        }

        if settings.supports_temperature {
            if let Some(temp) = req.params.temperature {
                body["temperature"] = json!(temp);
            }

            if let Some(top_p) = req.params.top_p {
                body["top_p"] = json!(top_p);
            }
        }

        if let Some(freq_penalty) = req.params.frequency_penalty {
            body["frequency_penalty"] = json!(freq_penalty);
        }

        if !req.params.stop.is_empty() {
            body["stop"] = json!(req.params.stop);
        }

        if let Some(n) = req.params.n {
            body["n"] = json!(n);
        }

        if settings.supports_tools {
            if let Some(tools) = &req.tools {
                if !tools.is_empty() {
                    body["tools"] = json!(tools);
                    if let Some(choice) = &req.tool_choice {
                        body["tool_choice"] = tool_choice_to_openai(choice);
                    }
                    if req.parallel_tool_calls {
                        body["parallel_tool_calls"] = json!(true);
                    }
                }
            }
        } else if req.tools.is_some() {
            tracing::warn!(
                "model {} does not support tools, skipping tools in request",
                settings.model_name
            );
        }

        if settings.supports_reasoning {
            if !crate::provider_quirks::uses_openai_provider_reasoning_controls(req) {
                if let Some(effort) = req.reasoning.to_openai_effort() {
                    body["reasoning_effort"] = json!(effort);
                }
            }
            body.as_object_mut().map(|obj| obj.remove("temperature"));
            body.as_object_mut().map(|obj| obj.remove("top_p"));
        }

        if let Some(ref format) = req.response_format {
            body["response_format"] = response_format_to_openai(format);
        }

        if let Some(extra) = &req.extra_body {
            if let Some(obj) = body.as_object_mut() {
                for (k, v) in extra {
                    if PROTECTED_FIELDS.contains(&k.as_str()) {
                        tracing::warn!(
                            "extra_body attempted to override protected field '{}', ignoring",
                            k
                        );
                        continue;
                    }
                    obj.insert(k.clone(), v.clone());
                }
            }
        }

        if let Some(raw) = settings
            .extra_headers
            .get(crate::adapter::OPENROUTER_EXTRA_BODY_HEADER)
        {
            match serde_json::from_str::<serde_json::Value>(raw.trim()) {
                Ok(configured) => {
                    if let (Some(obj), Some(fields)) =
                        (body.as_object_mut(), configured.as_object())
                    {
                        for (k, v) in fields {
                            if PROTECTED_FIELDS.contains(&k.as_str()) {
                                tracing::warn!(
                                    "openrouter extra_body attempted to override protected field '{}', ignoring",
                                    k
                                );
                                continue;
                            }
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
                Err(_) => {
                    tracing::warn!("openrouter extra_body header is not valid JSON, ignoring")
                }
            }
        }

        crate::provider_quirks::apply_openai_chat_body_quirks(&mut body, req, settings);

        tracing::info!(
            model = %settings.model_name,
            endpoint = %settings.endpoint,
            stream = %req.stream,
            max_tokens = %req.params.max_tokens,
            temperature = ?req.params.temperature,
            frequency_penalty = ?req.params.frequency_penalty,
            n = ?req.params.n,
            stop_sequences = ?req.params.stop.len(),
            tools_count = ?req.tools.as_ref().map(|t| t.len()),
            tool_choice = ?req.tool_choice,
            reasoning = ?req.reasoning,
            response_format = ?req.response_format.is_some(),
            cache_control = ?req.cache_control,
            messages_count = %req.messages.len(),
            "openai chat adapter request"
        );

        Ok(HttpParts {
            url: settings.endpoint.clone(),
            headers,
            body,
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
            .map_err(|e| StreamParseError::MalformedChunk(format!("json parse: {e}")))?;

        if let Some(error) = json.get("error") {
            return Err(StreamParseError::FatalError(
                error
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error")
                    .to_string(),
            ));
        }

        let mut deltas = Vec::new();

        if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
            for choice in choices {
                if let Some(delta) = choice.get("delta") {
                    if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                        if !content.is_empty() {
                            deltas.push(LlmStreamDelta::AppendContent {
                                text: content.to_string(),
                                block_index: None,
                            });
                        }
                    }

                    if let Some(refusal) = delta.get("refusal").and_then(|r| r.as_str()) {
                        if !refusal.is_empty() {
                            deltas.push(LlmStreamDelta::AppendContent {
                                text: refusal.to_string(),
                                block_index: None,
                            });
                        }
                    }

                    if let Some(reasoning) = delta.get("reasoning_content").and_then(|r| r.as_str())
                    {
                        if !reasoning.is_empty() {
                            deltas.push(LlmStreamDelta::AppendReasoning {
                                text: reasoning.to_string(),
                                block_index: None,
                            });
                        }
                    }

                    if let Some(blocks) = nonempty_thinking_blocks(delta) {
                        deltas.push(LlmStreamDelta::SetThinkingBlocks {
                            blocks: blocks.clone(),
                        });
                    }

                    if let Some(tool_calls) = delta.get("tool_calls") {
                        if let Some(arr) = tool_calls.as_array() {
                            let normalized: Vec<_> = arr
                                .iter()
                                .enumerate()
                                .filter_map(|(idx, tc)| normalize_openai_tool_call_delta(tc, idx))
                                .collect();
                            if !normalized.is_empty() {
                                deltas.push(LlmStreamDelta::SetToolCalls {
                                    tool_calls: normalized,
                                });
                            }
                        }
                    }

                    if let Some(function_call) = delta.get("function_call") {
                        if let Some(tc) = normalize_legacy_function_call_delta(function_call) {
                            deltas.push(LlmStreamDelta::SetToolCalls {
                                tool_calls: vec![tc],
                            });
                        }
                    }

                    // Citations support (OpenAI-compatible, e.g., Perplexity, litellm)
                    if let Some(citations) = delta.get("citations") {
                        if let Some(arr) = citations.as_array() {
                            for citation in arr {
                                deltas.push(LlmStreamDelta::AddCitation {
                                    citation: citation.clone(),
                                });
                            }
                        }
                    }
                }

                if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                    deltas.push(LlmStreamDelta::SetFinishReason {
                        reason: reason.to_string(),
                    });
                }

                if let Some(message_tool_calls) = choice
                    .get("message")
                    .and_then(|m| m.get("tool_calls"))
                    .and_then(|v| v.as_array())
                {
                    let normalized: Vec<_> = message_tool_calls
                        .iter()
                        .enumerate()
                        .filter_map(|(idx, tc)| normalize_openai_tool_call_delta(tc, idx))
                        .collect();
                    if !normalized.is_empty() {
                        deltas.push(LlmStreamDelta::FinalizeToolCalls {
                            tool_calls: normalized,
                        });
                    }
                }

                if let Some(message) = choice.get("message") {
                    if let Some(blocks) = nonempty_thinking_blocks(message) {
                        deltas.push(LlmStreamDelta::SetThinkingBlocks {
                            blocks: blocks.clone(),
                        });
                    }
                }
            }
        }

        if let Some(usage) = json.get("usage") {
            if let Some(u) = parse_openai_usage(usage) {
                deltas.push(LlmStreamDelta::SetUsage { usage: u });
            }
        }

        // Extract Refact-specific extra fields (metering, billing, etc.)
        let extra = extract_extra_fields(&json);
        if !extra.is_empty() {
            deltas.push(LlmStreamDelta::MergeExtra { extra });
        }

        Ok(deltas)
    }
}

fn nonempty_thinking_blocks(value: &Value) -> Option<&Vec<Value>> {
    value
        .get("thinking_blocks")
        .and_then(Value::as_array)
        .filter(|blocks| !blocks.is_empty())
        .or_else(|| {
            value
                .get("provider_specific_fields")
                .and_then(|fields| fields.get("thinking_blocks"))
                .and_then(Value::as_array)
                .filter(|blocks| !blocks.is_empty())
        })
}

fn convert_messages_to_openai(
    messages: &[refact_core::chat_types::ChatMessage],
    is_litellm: bool,
) -> Vec<Value> {
    use super::render_extra::{
        append_text_to_tool_json, is_context_role, is_event_role, is_goal_role, is_plan_role,
        render_context_message, render_event_message, render_goal_message, render_plan_message,
    };

    let mut result: Vec<Value> = Vec::new();
    let mut pending_user_content: Vec<Value> = Vec::new();

    for msg in messages {
        if msg.extra.get("_ui_only").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let projected = refact_core::edit_results::project_edit_result(msg);
        let msg = projected.as_ref();
        if is_goal_role(&msg.role) || is_plan_role(&msg.role) {
            if !pending_user_content.is_empty() {
                result.push(json!({
                    "role": "user",
                    "content": std::mem::take(&mut pending_user_content),
                }));
            }
            let text = if is_goal_role(&msg.role) {
                render_goal_message(msg)
            } else {
                render_plan_message(msg)
            };
            if let Some(text) = text {
                result.push(json!({"role": "user", "content": text}));
            }
            continue;
        }

        if is_context_role(&msg.role) || is_event_role(&msg.role) {
            let text = if is_event_role(&msg.role) {
                Some(render_event_message(msg))
            } else {
                render_context_message(msg)
            };
            let Some(text) = text else {
                continue;
            };
            if is_context_role(&msg.role) && !pending_user_content.is_empty() {
                pending_user_content.push(json!({"type": "text", "text": text}));
            } else if is_context_role(&msg.role) {
                // Fold into the matching tool result by tool_call_id when possible
                // so the model receives file content as part of the correct tool output.
                // Fall back to the last tool message if tool_call_id is absent.
                let target = if !msg.tool_call_id.is_empty() {
                    result.iter_mut().rev().find(|m| {
                        m["role"].as_str() == Some("tool")
                            && m["tool_call_id"].as_str() == Some(msg.tool_call_id.as_str())
                    })
                } else {
                    result
                        .iter_mut()
                        .rev()
                        .find(|m| m["role"].as_str() == Some("tool"))
                };
                if let Some(tool_msg) = target {
                    append_text_to_tool_json(tool_msg, &text);
                } else {
                    pending_user_content.push(json!({"type": "text", "text": text}));
                }
            } else {
                pending_user_content.push(json!({"type": "text", "text": text}));
            }
            continue;
        }

        let role = match msg.role.as_str() {
            "developer" | "user" | "assistant" | "system" | "tool" => msg.role.clone(),
            "diff" => "tool".to_string(),
            _ => continue,
        };

        if (role == "tool" || msg.role == "diff") && msg.tool_call_id.starts_with("srvtoolu_") {
            continue;
        }

        if role != "user" && role != "tool" && !pending_user_content.is_empty() {
            result.push(json!({
                "role": "user",
                "content": std::mem::take(&mut pending_user_content),
            }));
        }

        let mut obj = json!({"role": role});

        match &msg.content {
            refact_core::chat_types::ChatContent::SimpleText(text) => {
                let text = text.clone();

                if role == "user" && !pending_user_content.is_empty() {
                    let mut content = std::mem::take(&mut pending_user_content);
                    if !text.is_empty() {
                        content.push(json!({"type": "text", "text": text}));
                    }
                    obj["content"] = json!(content);
                } else {
                    obj["content"] = json!(text);
                }
            }
            refact_core::chat_types::ChatContent::Multimodal(elements) => {
                // Only use array format when content actually contains images.
                // Text-only multimodal (e.g. from trajectory deserialization or clients
                // sending [{"type":"text","text":"..."}]) must be normalized to plain string —
                // OpenAI Chat Completions requires string content for assistant/tool messages.
                let has_images = elements.iter().any(|el| el.is_image());
                if role == "user" {
                    if !pending_user_content.is_empty() || has_images {
                        // Prepend pending blocks, then the message's own content blocks.
                        let mut content = std::mem::take(&mut pending_user_content);
                        if has_images {
                            content.extend(elements.iter().map(|el| {
                                if el.is_image() {
                                    json!({
                                        "type": "image_url",
                                        "image_url": {
                                            "url": format!("data:{};base64,{}", el.m_type, el.m_content)
                                        }
                                    })
                                } else {
                                    json!({"type": "text", "text": el.m_content})
                                }
                            }));
                        } else {
                            let plain = msg.content.content_text_only();
                            if !plain.is_empty() {
                                content.push(json!({"type": "text", "text": plain}));
                            }
                        }
                        obj["content"] = json!(content);
                    } else {
                        // No pending content and no images: collapse to plain string.
                        obj["content"] = json!(msg.content.content_text_only());
                    }
                } else {
                    // Non-user roles (tool, assistant, system) must carry string content.
                    // Tool images are collected below and deferred to the next user turn.
                    obj["content"] = json!(msg.content.content_text_only());
                }
            }
            refact_core::chat_types::ChatContent::ContextFiles(_) => {
                obj["content"] = json!(msg.content.content_text_only());
            }
        }

        if let Some(tool_calls) = &msg.tool_calls {
            let tc: Vec<Value> = tool_calls
                .iter()
                .filter(|tc| !tc.id.starts_with("srvtoolu_"))
                .map(|tc| {
                    let mut call = json!({
                        "id": tc.id,
                        "index": tc.index,
                        "type": "function",
                        "function": {
                            "name": tc.function.name,
                            "arguments": tc.function.arguments
                        }
                    });
                    if let Some(extra) = &tc.extra_content {
                        call["extra_content"] = extra.clone();
                    }
                    call
                })
                .collect();
            if !tc.is_empty() {
                obj["tool_calls"] = json!(tc);
            }
        }

        if !msg.tool_call_id.is_empty() {
            obj["tool_call_id"] = json!(msg.tool_call_id);
        }

        if let Some(reasoning) = &msg.reasoning_content {
            if !reasoning.is_empty() {
                obj["reasoning_content"] = json!(reasoning);
            }
        }

        if is_litellm && role == "assistant" {
            if let Some(blocks) = &msg.thinking_blocks {
                if !blocks.is_empty() {
                    obj["thinking_blocks"] = Value::Array(blocks.clone());
                }
            }
        }

        result.push(obj);

        if role == "tool" {
            if let refact_core::chat_types::ChatContent::Multimodal(elements) = &msg.content {
                for el in elements.iter().filter(|el| el.is_image()) {
                    pending_user_content.push(json!({
                        "type": "image_url",
                        "image_url": {
                            "url": format!("data:{};base64,{}", el.m_type, el.m_content)
                        }
                    }));
                }
            }
        }
    }

    if !pending_user_content.is_empty() {
        result.push(json!({
            "role": "user",
            "content": pending_user_content,
        }));
    }

    result
}

fn tool_choice_to_openai(choice: &CanonicalToolChoice) -> Value {
    match choice {
        CanonicalToolChoice::Auto => json!("auto"),
        CanonicalToolChoice::None => json!("none"),
        CanonicalToolChoice::Required => json!("required"),
        CanonicalToolChoice::Function { name } => json!({
            "type": "function",
            "function": {"name": name}
        }),
    }
}

fn response_format_to_openai(format: &ResponseFormat) -> Value {
    match format {
        ResponseFormat::Text => json!({"type": "text"}),
        ResponseFormat::JsonObject => json!({"type": "json_object"}),
        ResponseFormat::JsonSchema {
            name,
            description,
            schema,
            strict,
        } => {
            let mut json_schema = json!({
                "name": name,
                "schema": schema,
                "strict": strict,
            });
            if let Some(desc) = description {
                json_schema["description"] = json!(desc);
            }
            json!({"type": "json_schema", "json_schema": json_schema})
        }
    }
}

fn is_openrouter_anthropic_model(settings: &AdapterSettings) -> bool {
    let endpoint = settings.endpoint.to_ascii_lowercase();
    if !endpoint.contains("openrouter.ai") {
        return false;
    }

    is_anthropic_family_model(settings)
}

fn is_anthropic_family_model(settings: &AdapterSettings) -> bool {
    let model = settings.model_name.to_ascii_lowercase();
    model.starts_with("anthropic/") || model.contains("claude")
}

fn is_litellm(settings: &AdapterSettings) -> bool {
    settings
        .extra_headers
        .get(BASE_PROVIDER_HEADER)
        .is_some_and(|provider| provider == "litellm")
}

/// Inject cache_control breakpoints for non-OpenRouter routing.
/// Converts simple text messages to multipart format with cache_control on last block.
/// Strategy: cache system message + 4 strategically positioned messages (quarter, middle, last2, last).
fn inject_cache_control(messages: &mut [Value]) {
    inject_cache_control_bounded(messages, 5);
}

fn inject_cache_control_bounded(messages: &mut [Value], max_markers: usize) {
    let cc = json!({"type": "ephemeral", "ttl": "1h"});

    fn add_cache_to_message(msg: &mut Value, cc: &Value) {
        let Some(content) = msg.get_mut("content") else {
            return;
        };
        if let Some(text) = content.as_str().map(|s| s.to_string()) {
            // Convert string content to array-of-blocks format (Anthropic multipart)
            *content = json!([{"type": "text", "text": text, "cache_control": cc}]);
        } else if let Some(arr) = content.as_array_mut() {
            // Already multipart - add cache_control to last block
            if let Some(last_block) = arr.last_mut() {
                if let Some(obj) = last_block.as_object_mut() {
                    obj.insert("cache_control".to_string(), cc.clone());
                }
            }
        }
    }

    if messages.is_empty() || max_markers == 0 {
        return;
    }

    // Cache system message if present
    let mut marker_count = 0;
    if let Some(first) = messages.first_mut() {
        if first.get("role").and_then(|r| r.as_str()) == Some("system") {
            add_cache_to_message(first, &cc);
            marker_count += 1;
        }
    }

    // Cache selected non-system messages
    let non_system_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.get("role").and_then(|r| r.as_str()) != Some("system"))
        .map(|(i, _)| i)
        .collect();

    if non_system_indices.is_empty() {
        return;
    }

    let len = non_system_indices.len();
    let quarter_pos = len / 4;
    let middle_pos = len / 2;
    let last_pos = len - 1;
    let last2_pos = len.saturating_sub(2);

    let mut selected_positions = vec![quarter_pos, middle_pos, last2_pos, last_pos];
    selected_positions.sort_unstable();
    selected_positions.dedup();
    let available_markers = max_markers.saturating_sub(marker_count);
    selected_positions.truncate(available_markers);
    if available_markers > 0 && !selected_positions.contains(&last_pos) {
        selected_positions.pop();
        selected_positions.push(last_pos);
    }

    for pos in selected_positions {
        if let Some(&msg_idx) = non_system_indices.get(pos) {
            add_cache_to_message(&mut messages[msg_idx], &cc);
        }
    }
}

fn parse_openai_usage(usage: &Value) -> Option<ChatUsage> {
    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as usize;

    // When routing through OpenRouter to Anthropic models, cache fields come in Anthropic format:
    // - cache_creation_input_tokens (top-level)
    // - cache_read_input_tokens (top-level)
    // For native OpenAI: cached_tokens in prompt_tokens_details (subset of prompt_tokens)

    let anthropic_cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(|t| t.as_u64())
        .filter(|&v| v > 0)
        .map(|v| v as usize);

    let anthropic_cache_read = usage
        .get("cache_read_input_tokens")
        .and_then(|t| t.as_u64())
        .filter(|&v| v > 0)
        .map(|v| v as usize);

    let details = usage.get("prompt_tokens_details");
    let openai_cached = details
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|t| t.as_u64())
        .filter(|&v| v > 0)
        .map(|v| v as usize);

    let moonshot_cached = usage
        .get("cached_tokens")
        .and_then(|t| t.as_u64())
        .filter(|&v| v > 0)
        .map(|v| v as usize);

    let dashscope_cache_hit = usage
        .get("prompt_cache_hit_tokens")
        .and_then(|t| t.as_u64())
        .filter(|&v| v > 0)
        .map(|v| v as usize);

    let dashscope_cache_miss = usage
        .get("prompt_cache_miss_tokens")
        .and_then(|t| t.as_u64())
        .filter(|&v| v > 0)
        .map(|v| v as usize);

    // Merge: prefer Anthropic fields (when routing via OpenRouter), fall back to OpenAI fields
    let cache_creation = anthropic_cache_creation.or(dashscope_cache_miss);
    let cache_read = anthropic_cache_read
        .or(openai_cached)
        .or(moonshot_cached)
        .or(dashscope_cache_hit);

    let raw_prompt = usage
        .get("prompt_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as usize;

    // For Anthropic models (via OpenRouter), prompt_tokens includes all input
    // (cache_read + cache_creation + non-cached). Subtract both to isolate
    // non-cached input only.
    // For native OpenAI, cached_tokens is already a subset of prompt_tokens,
    // and there's no cache_creation, so only cache_read subtraction applies.
    let prompt_tokens = raw_prompt
        .saturating_sub(cache_read.unwrap_or(0))
        .saturating_sub(cache_creation.unwrap_or(0));

    let total_tokens =
        prompt_tokens + completion_tokens + cache_creation.unwrap_or(0) + cache_read.unwrap_or(0);

    Some(ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_creation_tokens: cache_creation,
        cache_read_tokens: cache_read,
        metering_usd: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::chat_types::{ChatContent, ChatMessage};

    fn cleared(request: &LlmRequest) -> Cleared<LlmRequest> {
        refact_privacy::testing::cleared(request.clone())
    }

    fn default_settings() -> AdapterSettings {
        AdapterSettings {
            api_key: "test-key".to_string(),
            auth_token: String::new(),
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            extra_headers: Default::default(),
            model_name: "gpt-4".to_string(),
            supports_tools: true,
            supports_reasoning: false,
            reasoning_type: None,
            supports_temperature: true,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            supports_web_search: false,
            supports_cache_control: true,
        }
    }

    #[test]
    fn openrouter_extra_body_header_protected_fields_are_rejected() {
        let req = LlmRequest::new(
            "openai/gpt-4o".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hi".to_string())],
        );
        let mut settings = default_settings();
        settings.endpoint = "https://openrouter.ai/api/v1/chat/completions".to_string();
        settings.model_name = "openai/gpt-4o".to_string();
        settings.extra_headers.insert(
            crate::adapter::OPENROUTER_EXTRA_BODY_HEADER.to_string(),
            r#"{"model":"hacked","messages":[],"stream":false,"top_k":40}"#.to_string(),
        );

        let http = OpenAiChatAdapter
            .build_http(&cleared(&req), &settings)
            .unwrap();

        assert_eq!(http.body["model"], json!("openai/gpt-4o"));
        assert!(http.body["messages"]
            .as_array()
            .is_some_and(|m| m.len() == 1));
        assert_eq!(http.body["stream"], json!(true));
        assert_eq!(http.body["top_k"], json!(40));
    }

    #[test]
    fn build_http_omits_privacy_metadata() {
        let mut message = ChatMessage::new("user".to_string(), "hello".to_string());
        message
            .extra
            .insert("privacy".to_string(), json!({"files": []}));
        let request = LlmRequest::new("gpt-4".to_string(), vec![message]);

        let http = OpenAiChatAdapter
            .build_http(&cleared(&request), &default_settings())
            .unwrap();

        assert!(!http.body.to_string().contains("\"privacy\""));
    }

    fn goal_message(mode: &str, version: u32, content: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "goal".to_string(),
            json!({"mode": mode, "version": version}),
        );
        ChatMessage {
            role: "goal".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            extra,
            ..Default::default()
        }
    }

    fn plan_message(mode: &str, version: u32, content: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "plan".to_string(),
            json!({"mode": mode, "version": version}),
        );
        ChatMessage {
            role: "plan".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            extra,
            ..Default::default()
        }
    }

    #[test]
    fn convert_goal_and_plan_to_user_wrapped_xml_in_order() {
        let messages = vec![
            goal_message("agent", 1, "Reach the goal"),
            plan_message("agent", 1, "Do the thing"),
        ];

        let converted = convert_messages_to_openai(&messages, false);
        let serialized = json!({"messages": converted}).to_string();

        assert_eq!(serialized.matches("<goal mode=").count(), 1);
        assert_eq!(serialized.matches("<plan mode=").count(), 1);
        assert!(serialized.find("<goal mode=").unwrap() < serialized.find("<plan mode=").unwrap());
        assert!(!serialized.contains("\"role\":\"goal\""));
    }

    #[test]
    fn github_copilot_openai_chat_adds_vision_header_only_for_images() {
        use refact_core::chat_types::MultimodalElement;

        let adapter = OpenAiChatAdapter;
        let image_message = ChatMessage {
            role: "user".to_string(),
            content: ChatContent::Multimodal(vec![MultimodalElement {
                m_type: "image/png".to_string(),
                m_content: "base64data".to_string(),
            }]),
            ..Default::default()
        };
        let req = LlmRequest::new("github_copilot/gpt-4.1".to_string(), vec![image_message]);
        let mut settings = default_settings();
        settings.endpoint = "https://api.githubcopilot.com/v1/chat/completions".to_string();
        settings.api_key = "copilot-token".to_string();

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();

        assert_eq!(
            http.headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer copilot-token"
        );
        assert_eq!(
            http.headers
                .get("Copilot-Vision-Request")
                .unwrap()
                .to_str()
                .unwrap(),
            "true"
        );
        assert_eq!(
            http.headers.get("Openai-Intent").unwrap().to_str().unwrap(),
            "conversation-edits"
        );
        assert_eq!(
            http.headers.get("x-initiator").unwrap().to_str().unwrap(),
            "user"
        );

        let text_req = LlmRequest::new(
            "github_copilot/gpt-4.1".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        let text_http = adapter.build_http(&cleared(&text_req), &settings).unwrap();
        assert!(text_http.headers.get("Copilot-Vision-Request").is_none());
    }

    #[test]
    fn github_copilot_openai_chat_protected_headers_remain_unoverridable() {
        let adapter = OpenAiChatAdapter;
        let req = LlmRequest::new(
            "github_copilot/gpt-4.1".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        let mut settings = default_settings();
        settings.endpoint = "https://api.githubcopilot.com/v1/chat/completions".to_string();
        settings.api_key = "copilot-token".to_string();
        settings
            .extra_headers
            .insert("Authorization".to_string(), "Bearer hacked".to_string());
        settings
            .extra_headers
            .insert("x-api-key".to_string(), "hacked".to_string());
        settings
            .extra_headers
            .insert("Copilot-Vision-Request".to_string(), "true".to_string());

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();

        assert_eq!(
            http.headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer copilot-token"
        );
        assert!(http.headers.get("x-api-key").is_none());
        assert!(http.headers.get("Copilot-Vision-Request").is_none());
    }

    #[test]
    fn test_build_http_omits_top_p_for_reasoning_models() {
        let adapter = OpenAiChatAdapter;
        let mut req = LlmRequest::new(
            "o3".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        req.params.temperature = Some(0.5);
        req.params.top_p = Some(0.9);

        let mut settings = default_settings();
        settings.model_name = "o3".to_string();
        settings.supports_reasoning = true;
        settings.supports_temperature = true;

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();

        assert!(http.body.get("temperature").is_none());
        assert!(http.body.get("top_p").is_none());
    }

    #[test]
    fn test_qwen_reasoning_enabled_body_contains_thinking_budget() {
        use crate::params::ReasoningIntent;

        let adapter = OpenAiChatAdapter;
        let req = LlmRequest::new(
            "qwen/qwen3-max".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        )
        .with_reasoning(ReasoningIntent::BudgetTokens(2048));

        let mut settings = default_settings();
        settings.model_name = "qwen3-max".to_string();
        settings.endpoint =
            "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions".to_string();
        settings.supports_reasoning = true;

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();

        assert_eq!(http.body["enable_thinking"], true);
        assert_eq!(http.body["thinking_budget"], 2048);
        assert!(http.body.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_qwen_reasoning_off_body_contains_enable_thinking_false() {
        let adapter = OpenAiChatAdapter;
        let req = LlmRequest::new(
            "qwen/qwen3-max".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );

        let mut settings = default_settings();
        settings.model_name = "qwen3-max".to_string();
        settings.supports_reasoning = true;

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();

        assert_eq!(http.body["enable_thinking"], false);
        assert!(http.body.get("thinking_budget").is_none());
        assert!(http.body.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_non_qwen_openai_model_never_gets_qwen_fields() {
        use crate::params::ReasoningIntent;

        let adapter = OpenAiChatAdapter;
        let req = LlmRequest::new(
            "custom/qwen3-max".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        )
        .with_reasoning(ReasoningIntent::BudgetTokens(2048));

        let mut settings = default_settings();
        settings.model_name = "qwen3-max".to_string();
        settings.supports_reasoning = true;

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();

        assert!(http.body.get("enable_thinking").is_none());
        assert!(http.body.get("thinking_budget").is_none());
        assert_eq!(http.body["reasoning_effort"], "high");
    }

    #[test]
    fn test_zhipu_reasoning_enabled_uses_glm_thinking_body() {
        use crate::params::ReasoningIntent;

        let adapter = OpenAiChatAdapter;
        let req = LlmRequest::new(
            "zhipu/glm-4.7".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        )
        .with_reasoning(ReasoningIntent::Medium);

        let mut settings = default_settings();
        settings.model_name = "glm-4.7".to_string();
        settings.supports_reasoning = true;

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();

        assert_eq!(http.body["thinking"], json!({"type": "enabled"}));
        assert!(http.body.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_zhipu_reasoning_off_uses_glm_thinking_disabled() {
        let adapter = OpenAiChatAdapter;
        let req = LlmRequest::new(
            "zhipu/glm-4.7".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );

        let mut settings = default_settings();
        settings.model_name = "glm-4.7".to_string();
        settings.supports_reasoning = true;

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();

        assert_eq!(http.body["thinking"], json!({"type": "disabled"}));
        assert!(http.body.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_parse_stream_chunk_content() {
        let adapter = OpenAiChatAdapter;
        let chunk = r#"{"choices":[{"delta":{"content":"Hello"}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::AppendContent { text, .. } => assert_eq!(text, "Hello"),
            _ => panic!("expected AppendContent"),
        }
    }

    #[test]
    fn test_parse_stream_chunk_refusal() {
        let adapter = OpenAiChatAdapter;
        let chunk = r#"{"choices":[{"delta":{"refusal":"I can’t help with that."}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::AppendContent { text, .. } => {
                assert_eq!(text, "I can’t help with that.")
            }
            _ => panic!("expected AppendContent"),
        }
    }

    #[test]
    fn test_parse_stream_chunk_preserves_signed_delta_thinking_blocks() {
        let adapter = OpenAiChatAdapter;
        let expected = vec![json!({
            "type": "thinking",
            "thinking": "I should call the tool.",
            "signature": "opaque-signed-delta==",
            "provider_extension": {"version": 1}
        })];
        let chunk = json!({
            "choices": [{"delta": {"thinking_blocks": expected.clone()}}]
        })
        .to_string();

        let deltas = adapter.parse_stream_chunk(&chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::SetThinkingBlocks { blocks } => assert_eq!(blocks, &expected),
            _ => panic!("expected SetThinkingBlocks"),
        }
    }

    #[test]
    fn test_parse_stream_chunk_preserves_signed_final_message_thinking_blocks() {
        let adapter = OpenAiChatAdapter;
        let expected = vec![json!({
            "type": "thinking",
            "thinking": "The accumulated thought.",
            "signature": "opaque-signed-final/+/="
        })];
        let chunk = json!({
            "choices": [{"message": {"thinking_blocks": expected.clone()}}]
        })
        .to_string();

        let deltas = adapter.parse_stream_chunk(&chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::SetThinkingBlocks { blocks } => assert_eq!(blocks, &expected),
            _ => panic!("expected SetThinkingBlocks"),
        }
    }

    #[test]
    fn test_parse_stream_chunk_preserves_provider_specific_thinking_blocks() {
        let adapter = OpenAiChatAdapter;
        let delta_blocks = vec![json!({
            "type": "thinking",
            "signature": "opaque-provider-delta==",
            "provider_extension": {"nested": [1, true, null]}
        })];
        let final_blocks = vec![json!({
            "type": "thinking",
            "signature": "opaque-provider-final/+/=",
            "unknown_field": "preserve verbatim"
        })];
        let chunk = json!({
            "choices": [{
                "delta": {
                    "provider_specific_fields": {"thinking_blocks": delta_blocks.clone()}
                },
                "message": {
                    "provider_specific_fields": {"thinking_blocks": final_blocks.clone()}
                }
            }]
        })
        .to_string();

        let deltas = adapter.parse_stream_chunk(&chunk).unwrap();

        assert_eq!(deltas.len(), 2);
        match &deltas[0] {
            LlmStreamDelta::SetThinkingBlocks { blocks } => assert_eq!(blocks, &delta_blocks),
            _ => panic!("expected delta SetThinkingBlocks"),
        }
        match &deltas[1] {
            LlmStreamDelta::SetThinkingBlocks { blocks } => assert_eq!(blocks, &final_blocks),
            _ => panic!("expected final SetThinkingBlocks"),
        }
    }

    #[test]
    fn test_parse_stream_chunk_done() {
        let adapter = OpenAiChatAdapter;
        let deltas = adapter.parse_stream_chunk("[DONE]").unwrap();

        assert_eq!(deltas.len(), 1);
        assert!(matches!(deltas[0], LlmStreamDelta::Done));
    }

    #[test]
    fn test_parse_stream_chunk_tool_calls_arguments_only_preserved() {
        let adapter = OpenAiChatAdapter;
        let chunk = r#"{"choices":[{"delta":{"tool_calls":[{"index":2,"function":{"arguments":"{\"q\""}}]}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        match &deltas[0] {
            LlmStreamDelta::SetToolCalls { tool_calls } => {
                assert_eq!(tool_calls[0]["index"], 2);
                assert_eq!(tool_calls[0]["function"]["arguments"], "{\"q\"");
            }
            _ => panic!("expected SetToolCalls"),
        }
    }

    #[test]
    fn test_parse_stream_chunk_malformed_skipped() {
        let adapter = OpenAiChatAdapter;
        let result = adapter.parse_stream_chunk("not json");

        assert!(matches!(result, Err(StreamParseError::MalformedChunk(_))));
    }

    #[test]
    fn test_build_http_uses_max_tokens_by_default() {
        let adapter = OpenAiChatAdapter;
        let mut req = LlmRequest::new("gpt-4".to_string(), vec![]);
        req.params.max_tokens = 500;
        let settings = default_settings();

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();

        assert_eq!(http.body["max_tokens"], 500);
        assert!(http.body.get("max_completion_tokens").is_none());
    }

    #[test]
    fn test_build_http_uses_max_completion_tokens_when_supported() {
        let adapter = OpenAiChatAdapter;
        let mut req = LlmRequest::new("o1".to_string(), vec![]);
        req.params.max_tokens = 500;
        let mut settings = default_settings();
        settings.supports_max_completion_tokens = true;

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();

        assert_eq!(http.body["max_completion_tokens"], 500);
        assert!(http.body.get("max_tokens").is_none());
    }

    #[test]
    fn test_extra_body_protected_fields_ignored() {
        let adapter = OpenAiChatAdapter;
        let mut req = LlmRequest::new(
            "gpt-4".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hi".to_string())],
        );
        req.extra_body = Some(serde_json::Map::from_iter([
            ("model".to_string(), json!("hacked-model")),
            (
                "messages".to_string(),
                json!([{"role": "user", "content": "hacked"}]),
            ),
            ("stream".to_string(), json!(false)),
            ("custom_field".to_string(), json!("allowed")),
        ]));

        let http = adapter
            .build_http(&cleared(&req), &default_settings())
            .unwrap();

        assert_eq!(http.body["model"], "gpt-4");
        assert_ne!(
            http.body["messages"],
            json!([{"role": "user", "content": "hacked"}])
        );
        assert_eq!(http.body["stream"], true);
        assert_eq!(http.body["custom_field"], "allowed");
    }

    #[test]
    fn test_server_executed_tools_filtered() {
        use refact_core::chat_types::{ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Search for something".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![
                    ChatToolCall {
                        id: "srvtoolu_123".to_string(), // Server-executed
                        index: Some(0),
                        tool_type: "function".to_string(),
                        extra_content: None,
                        started_at_ms: None,
                        completed_at_ms: None,
                        function: ChatToolFunction {
                            name: "web_search".to_string(),
                            arguments: r#"{"query":"test"}"#.to_string(),
                        },
                    },
                    ChatToolCall {
                        id: "call_456".to_string(), // Regular tool call
                        index: Some(1),
                        tool_type: "function".to_string(),
                        extra_content: None,
                        started_at_ms: None,
                        completed_at_ms: None,
                        function: ChatToolFunction {
                            name: "cat".to_string(),
                            arguments: r#"{"path":"file.txt"}"#.to_string(),
                        },
                    },
                ]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText(
                    "search results".to_string(),
                ),
                tool_call_id: "srvtoolu_123".to_string(), // Server-executed result
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: refact_core::chat_types::ChatContent::SimpleText(
                    "file content".to_string(),
                ),
                tool_call_id: "call_456".to_string(), // Regular tool result
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_openai(&messages, false);

        // Should have 3 messages: user, assistant (with only regular tool call), tool result (only regular)
        assert_eq!(converted.len(), 3);

        // Assistant message should only have the regular tool call
        let assistant = &converted[1];
        let tool_calls = assistant["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "call_456");

        // Only regular tool result should be present
        let tool_result = &converted[2];
        assert_eq!(tool_result["tool_call_id"], "call_456");
    }

    #[test]
    fn test_assistant_thinking_blocks_replayed_exactly_with_reasoning_and_tool_call() {
        use refact_core::chat_types::{ChatToolCall, ChatToolFunction};

        let thinking_blocks = vec![json!({
            "type": "thinking",
            "thinking": "Use the weather tool.",
            "signature": "opaque-signature-must-not-be-rebuilt==",
            "opaque": {"nested": [true, 7, null]}
        })];
        let messages = vec![ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            reasoning_content: Some("Use the weather tool.".to_string()),
            thinking_blocks: Some(thinking_blocks.clone()),
            tool_calls: Some(vec![ChatToolCall {
                id: "call_weather".to_string(),
                index: Some(0),
                tool_type: "function".to_string(),
                function: ChatToolFunction {
                    name: "get_weather".to_string(),
                    arguments: r#"{"city":"Paris"}"#.to_string(),
                },
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            }]),
            ..Default::default()
        }];

        let req = LlmRequest::new("anthropic/claude-sonnet".to_string(), messages);
        let mut settings = default_settings();
        settings.model_name = "anthropic/claude-sonnet".to_string();
        settings
            .extra_headers
            .insert(BASE_PROVIDER_HEADER.to_string(), "litellm".to_string());

        let http = OpenAiChatAdapter
            .build_http(&cleared(&req), &settings)
            .unwrap();
        let converted = http.body["messages"].as_array().unwrap();

        assert_eq!(converted.len(), 1);
        assert_eq!(
            converted[0]["thinking_blocks"],
            Value::Array(thinking_blocks)
        );
        assert_eq!(converted[0]["reasoning_content"], "Use the weather tool.");
        assert_eq!(converted[0]["tool_calls"][0]["id"], "call_weather");
        assert_eq!(
            converted[0]["tool_calls"][0]["function"]["arguments"],
            r#"{"city":"Paris"}"#
        );
    }

    #[test]
    fn test_non_litellm_request_omits_assistant_thinking_blocks() {
        let thinking_blocks = vec![json!({
            "type": "thinking",
            "thinking": "Do not send this provider field to regular OpenAI targets.",
            "signature": "opaque-signature=="
        })];
        let messages = vec![ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("Answer".to_string()),
            thinking_blocks: Some(thinking_blocks),
            ..Default::default()
        }];
        let req = LlmRequest::new("gpt-4".to_string(), messages);

        let http = OpenAiChatAdapter
            .build_http(&cleared(&req), &default_settings())
            .unwrap();

        assert!(http.body["messages"][0].get("thinking_blocks").is_none());
    }

    #[test]
    fn test_parse_openai_usage_with_anthropic_cache() {
        // OpenRouter -> Anthropic: top-level cache fields
        let usage = json!({
            "prompt_tokens": 1200,
            "completion_tokens": 100,
            "total_tokens": 1300,
            "cache_creation_input_tokens": 200,
            "cache_read_input_tokens": 800
        });

        let result = parse_openai_usage(&usage).unwrap();

        // prompt_tokens should be adjusted: 1200 - 800 (cache_read) - 200 (cache_creation) = 200
        assert_eq!(result.prompt_tokens, 200);
        assert_eq!(result.completion_tokens, 100);
        assert_eq!(result.cache_creation_tokens, Some(200));
        assert_eq!(result.cache_read_tokens, Some(800));
        assert_eq!(result.total_tokens, 1300);
    }

    #[test]
    fn test_parse_openai_usage_with_openai_cache() {
        // Native OpenAI: cached_tokens in prompt_tokens_details
        let usage = json!({
            "prompt_tokens": 1000,
            "completion_tokens": 100,
            "total_tokens": 1100,
            "prompt_tokens_details": {
                "cached_tokens": 800
            }
        });

        let result = parse_openai_usage(&usage).unwrap();

        assert_eq!(result.prompt_tokens, 200);
        assert_eq!(result.completion_tokens, 100);
        assert_eq!(result.cache_creation_tokens, None);
        assert_eq!(result.cache_read_tokens, Some(800));
    }

    #[test]
    fn test_parse_openai_usage_with_moonshot_top_level_cached_tokens() {
        let usage = json!({
            "prompt_tokens": 1000,
            "completion_tokens": 100,
            "total_tokens": 1100,
            "cached_tokens": 300
        });

        let result = parse_openai_usage(&usage).unwrap();

        assert_eq!(result.prompt_tokens, 700);
        assert_eq!(result.completion_tokens, 100);
        assert_eq!(result.cache_creation_tokens, None);
        assert_eq!(result.cache_read_tokens, Some(300));
        assert_eq!(result.total_tokens, 1100);
    }

    #[test]
    fn test_parse_openai_usage_with_dashscope_cache_metrics() {
        let usage = json!({
            "prompt_tokens": 1000,
            "completion_tokens": 100,
            "prompt_cache_hit_tokens": 700,
            "prompt_cache_miss_tokens": 200
        });

        let result = parse_openai_usage(&usage).unwrap();

        assert_eq!(result.prompt_tokens, 100);
        assert_eq!(result.completion_tokens, 100);
        assert_eq!(result.cache_creation_tokens, Some(200));
        assert_eq!(result.cache_read_tokens, Some(700));
        assert_eq!(result.total_tokens, 1100);
    }

    #[test]
    fn test_openrouter_anthropic_uses_top_level_cache_control() {
        let adapter = OpenAiChatAdapter;
        let req = LlmRequest::new(
            "anthropic/claude-sonnet-4.6".to_string(),
            vec![
                ChatMessage::new("system".to_string(), "You are helpful".to_string()),
                ChatMessage::new("user".to_string(), "Hello".to_string()),
            ],
        )
        .with_cache_control(CacheControl::Ephemeral);

        let mut settings = default_settings();
        settings.endpoint = "https://openrouter.ai/api/v1/chat/completions".to_string();
        settings.model_name = "anthropic/claude-sonnet-4.6".to_string();

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();
        assert_eq!(http.body["cache_control"]["type"], "ephemeral");
        assert_eq!(http.body["cache_control"]["ttl"], "1h");

        let messages = http.body["messages"].as_array().unwrap();
        for msg in messages {
            let content = &msg["content"];
            if let Some(arr) = content.as_array() {
                for block in arr {
                    assert!(block.get("cache_control").is_none());
                }
            }
        }
    }

    #[test]
    fn test_litellm_anthropic_on_openrouter_endpoint_uses_four_block_markers() {
        let adapter = OpenAiChatAdapter;
        let mut messages = vec![ChatMessage::new(
            "system".to_string(),
            "You are helpful".to_string(),
        )];
        for index in 0..6 {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            messages.push(ChatMessage::new(
                role.to_string(),
                format!("message {index}"),
            ));
        }
        let req = LlmRequest::new("anthropic/claude-sonnet-4.6".to_string(), messages)
            .with_cache_control(CacheControl::Ephemeral);
        let mut settings = default_settings();
        settings.endpoint = "https://openrouter.ai/api/v1/chat/completions".to_string();
        settings.model_name = "anthropic/claude-sonnet-4.6".to_string();
        settings
            .extra_headers
            .insert(BASE_PROVIDER_HEADER.to_string(), "litellm".to_string());

        let http = adapter.build_http(&cleared(&req), &settings).unwrap();
        let messages = http.body["messages"].as_array().unwrap();
        let marker_count = messages
            .iter()
            .flat_map(|message| message["content"].as_array().into_iter().flatten())
            .filter(|block| block.get("cache_control").is_some())
            .count();

        assert_eq!(marker_count, 4);
        assert!(messages.last().unwrap()["content"][0]
            .get("cache_control")
            .is_some());
        assert!(http.body.get("cache_control").is_none());
    }
}

#[cfg(test)]
mod edit_projection_wire_tests {
    use super::*;
    #[test]
    fn diff_wire_projection_and_legacy_fallback() {
        use refact_core::chat_types::ChatMessage;
        let call: ChatMessage = serde_json::from_value(serde_json::json!({"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"edit","arguments":"{}"}}]})).unwrap();
        for metadata in [
            Some(serde_json::json!({"version":1,"model_content":"SAFE_EXCERPT"})),
            None,
            Some(serde_json::json!({"version":2,"model_content":"BAD"})),
        ] {
            let mut diff: ChatMessage = serde_json::from_value(serde_json::json!({"role":"diff","content":"{\"lines_remove\":\"REMOVED_SENTINEL\"}","tool_call_id":"call_1","tool_failed":true})).unwrap();
            if let Some(value) = metadata {
                diff.extra.insert("edit_result".into(), value);
            }
            let expected = if diff
                .extra
                .get("edit_result")
                .and_then(|v| v.get("version"))
                .and_then(|v| v.as_u64())
                == Some(1)
            {
                "SAFE_EXCERPT"
            } else {
                "excerpt unavailable"
            };
            let before = serde_json::to_vec(&diff).unwrap();
            let messages = vec![call.clone(), diff];
            let wire = serde_json::to_value(convert_messages_to_openai(&messages, false))
                .unwrap()
                .to_string();
            assert!(wire.contains(expected), "{wire}");
            assert!(!wire.contains("REMOVED_SENTINEL"), "{wire}");
            assert!(!wire.contains("lines_remove"), "{wire}");
            assert!(wire.contains("call_1"), "{wire}");
            assert_eq!(serde_json::to_vec(&messages[1]).unwrap(), before);
        }
    }
}
