use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};

use crate::call_validation::ChatUsage;
use crate::llm::adapter::{AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError, extract_extra_fields, insert_extra_headers};
use crate::llm::canonical::{CanonicalToolChoice, LlmRequest, LlmStreamDelta};
use crate::llm::params::CacheControl;

const DEFAULT_THINKING_BUDGET: usize = 10000;
const PROTECTED_FIELDS: &[&str] = &[
    "model", "messages", "stream", "tools", "tool_choice", "stream_options",
    "max_completion_tokens", "temperature", "frequency_penalty", "stop", "n",
    "reasoning_effort", "thinking", "meta", "parallel_tool_calls", "n_ctx",
];

pub struct RefactAdapter;

impl LlmWireAdapter for RefactAdapter {
    fn build_http(
        &self,
        req: &LlmRequest,
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

        let reasoning_type = settings.reasoning_type.as_deref();
        let mut messages = convert_messages_to_refact(&req.messages, reasoning_type);

        // LiteLLM prompt caching via `cache_control` is intended for Anthropic-native routing.
        // Some backends (notably Vertex/Gemini) treat cache controls as CachedContent and reject
        // requests that also include system instruction / tools / tool_config.
        let is_anthropic_target = reasoning_type.is_some_and(|rt| rt.starts_with("anthropic"));
        if is_anthropic_target && matches!(req.cache_control, CacheControl::Ephemeral) {
            inject_cache_control(&mut messages);
        }

        let mut body = json!({
            "model": settings.model_name,
            "messages": messages,
            "stream": req.stream,
        });

        if let Some(n_ctx) = req.params.n_ctx {
            body["n_ctx"] = json!(n_ctx);
        }

        if settings.supports_max_completion_tokens {
            body["max_completion_tokens"] = json!(req.params.max_tokens);
        } else {
            body["max_tokens"] = json!(req.params.max_tokens);
        }

        if settings.supports_temperature {
            if let Some(temp) = req.params.temperature {
                body["temperature"] = json!(temp);
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
                        body["tool_choice"] = tool_choice_to_refact(choice);
                    }
                    if req.parallel_tool_calls {
                        body["parallel_tool_calls"] = json!(true);
                    }
                }
            }
        }

        if settings.supports_reasoning {
            let rtype = settings.reasoning_type.as_deref().unwrap_or("");
            match rtype {
                "anthropic_budget" => {
                    if let Some(budget) = req.reasoning.to_anthropic_budget(DEFAULT_THINKING_BUDGET) {
                        body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
                    }
                }
                "anthropic_effort" => {
                    match &req.reasoning {
                        crate::llm::params::ReasoningIntent::BudgetTokens(n) => {
                            body["thinking"] = json!({"type": "enabled", "budget_tokens": *n});
                        }
                        _ => {
                            if let Some(effort) = req.reasoning.to_anthropic_effort() {
                                body["reasoning_effort"] = json!(effort);
                                body["output_config"] = json!({"effort": effort});
                            }
                        }
                    }
                }
                "xai" => {
                    // do nothing since the reasoning supported only implicitly
                },
                _ => {
                    // openai, deepseek, xai, qwen, gemini, kimi, zhipu, mistral, etc.
                    if let Some(effort) = req.reasoning.to_openai_effort() {
                        body["reasoning_effort"] = json!(effort);
                    }
                }
            }
            body.as_object_mut().map(|obj| obj.remove("temperature"));
        }

        if let Some(meta) = &req.meta {
            if let Ok(meta_value) = serde_json::to_value(meta) {
                body["meta"] = meta_value;
            }
        }

        if let Some(extra) = &req.extra_body {
            if let Some(obj) = body.as_object_mut() {
                for (k, v) in extra {
                    if PROTECTED_FIELDS.contains(&k.as_str()) {
                        tracing::warn!("extra_body attempted to override protected field '{}', ignoring", k);
                        continue;
                    }
                    obj.insert(k.clone(), v.clone());
                }
            }
        }

        if req.stream {
            body["stream_options"] = json!({"include_usage": true});
        }

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
            cache_control = ?req.cache_control,
            has_meta = %req.meta.is_some(),
            has_extra_body = %req.extra_body.is_some(),
            messages_count = %req.messages.len(),
            "refact adapter request"
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

        // FastAPI-style error (Refact backend uses FastAPI)
        if let Some(detail) = json.get("detail") {
            let msg = match detail {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            return Err(StreamParseError::FatalError(msg));
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

                    if let Some(reasoning) = delta.get("reasoning_content").and_then(|r| r.as_str()) {
                        if !reasoning.is_empty() {
                            deltas.push(LlmStreamDelta::AppendReasoning {
                                text: reasoning.to_string(),
                                block_index: None,
                            });
                        }
                    }

                    // LiteLLM streams thinking_blocks with signatures for Anthropic models.
                    // Each chunk may contain partial blocks; blocks with a signature are final.
                    if let Some(blocks) = delta.get("thinking_blocks").and_then(|b| b.as_array()) {
                        let signed: Vec<Value> = blocks.iter()
                            // Some proxies send an empty string while a signature is still
                            // being computed/unknown. Treat empty signatures as "not signed"
                            // and skip them, otherwise multi-turn will resend invalid blocks.
                            .filter(|b| b.get("signature")
                                .and_then(|s| s.as_str())
                                .is_some_and(|s| !s.trim().is_empty()))
                            .cloned()
                            .collect();
                        if !signed.is_empty() {
                            deltas.push(LlmStreamDelta::SetThinkingBlocks { blocks: signed });
                        }
                    }

                    if let Some(tool_calls) = delta.get("tool_calls") {
                        if let Some(arr) = tool_calls.as_array() {
                            if !arr.is_empty() {
                                deltas.push(LlmStreamDelta::SetToolCalls {
                                    tool_calls: arr.clone(),
                                });
                            }
                        }
                    }

                    // Citations support (Refact cloud via litellm)
                    // Format 1: Perplexity/OpenAI-style flat array in delta
                    if let Some(citations) = delta.get("citations") {
                        if let Some(arr) = citations.as_array() {
                            for citation in arr {
                                deltas.push(LlmStreamDelta::AddCitation {
                                    citation: citation.clone(),
                                });
                            }
                        }
                    }

                    // Format 2: Anthropic citations via LiteLLM — streamed as
                    // delta.provider_specific_fields.citation (singular per chunk)
                    if let Some(psf) = delta.get("provider_specific_fields") {
                        // Singular citation object (streaming Anthropic via LiteLLM)
                        if let Some(citation) = psf.get("citation") {
                            if !citation.is_null() {
                                deltas.push(LlmStreamDelta::AddCitation {
                                    citation: citation.clone(),
                                });
                            }
                        }
                        // Array of citations (non-streaming or accumulated)
                        if let Some(citations) = psf.get("citations") {
                            if let Some(arr) = citations.as_array() {
                                for citation in arr {
                                    deltas.push(LlmStreamDelta::AddCitation {
                                        citation: citation.clone(),
                                    });
                                }
                            }
                        }
                    }
                }

                // Non-streaming responses: LiteLLM uses "message" instead of "delta"
                // Extract citations from message.provider_specific_fields
                if let Some(message) = choice.get("message") {
                    if let Some(psf) = message.get("provider_specific_fields") {
                        if let Some(citation) = psf.get("citation") {
                            if !citation.is_null() {
                                deltas.push(LlmStreamDelta::AddCitation {
                                    citation: citation.clone(),
                                });
                            }
                        }
                        if let Some(citations) = psf.get("citations") {
                            if let Some(arr) = citations.as_array() {
                                for citation in arr {
                                    deltas.push(LlmStreamDelta::AddCitation {
                                        citation: citation.clone(),
                                    });
                                }
                            }
                        }
                    }
                }

                if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                    deltas.push(LlmStreamDelta::SetFinishReason {
                        reason: reason.to_string(),
                    });
                }
            }
        }

        if let Some(usage) = json.get("usage") {
            if let Some(u) = parse_refact_usage(usage) {
                deltas.push(LlmStreamDelta::SetUsage { usage: u });
            }
        }

        let extra = extract_extra_fields(&json);
        if !extra.is_empty() {
            deltas.push(LlmStreamDelta::MergeExtra { extra });
        }

        Ok(deltas)
    }
}

fn convert_messages_to_refact(messages: &[crate::call_validation::ChatMessage], reasoning_type: Option<&str>) -> Vec<Value> {
    let is_anthropic_target = reasoning_type.map_or(false, |rt| rt.starts_with("anthropic"));
    let supports_reasoning_content = reasoning_type.is_some();
    messages
        .iter()
        .filter_map(|msg| {
            let role = match msg.role.as_str() {
                "user" | "assistant" | "system" | "tool" => msg.role.clone(),
                "diff" => "tool".to_string(),  // diff messages are tool results
                _ => return None,
            };

            // Filter out tool results for server-executed tools
            if (role == "tool" || msg.role == "diff") && msg.tool_call_id.starts_with("srvtoolu_") {
                return None;
            }

            let mut obj = json!({"role": role});

            match &msg.content {
                crate::call_validation::ChatContent::SimpleText(text) => {
                    obj["content"] = json!(text);
                }
                crate::call_validation::ChatContent::Multimodal(elements) => {
                    let content: Vec<Value> = elements
                        .iter()
                        .map(|el| {
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
                        })
                        .collect();
                    obj["content"] = json!(content);
                }
                crate::call_validation::ChatContent::ContextFiles(_) => {
                    obj["content"] = json!(msg.content.content_text_only());
                }
            }

            if let Some(tool_calls) = &msg.tool_calls {
                let tc: Vec<Value> = tool_calls
                    .iter()
                    .filter(|tc| !tc.id.starts_with("srvtoolu_"))  // Filter server-executed tools
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "index": tc.index,
                            "type": "function",
                            "function": {
                                "name": tc.function.name,
                                "arguments": tc.function.arguments
                            }
                        })
                    })
                    .collect();
                if !tc.is_empty() {
                    obj["tool_calls"] = json!(tc);
                }
            }

            if !msg.tool_call_id.is_empty() {
                obj["tool_call_id"] = json!(msg.tool_call_id);
            }

            if supports_reasoning_content {
                if let Some(reasoning) = &msg.reasoning_content {
                    if !reasoning.is_empty() {
                        obj["reasoning_content"] = json!(reasoning);
                    }
                }
            }

            if is_anthropic_target {
                if let Some(blocks) = &msg.thinking_blocks {
                    if !blocks.is_empty() {
                        // IMPORTANT: Anthropic thinking blocks are integrity-checked.
                        // Do NOT rebuild or mutate blocks here (whitespace, missing fields,
                        // dropped keys, etc.), or signatures may fail validation on multi-turn.
                        // Only filter out non-Anthropic block types (e.g. OpenAI Responses
                        // reasoning items) while preserving the raw JSON for valid blocks.
                        let keep: Vec<Value> = blocks
                            .iter()
                            .filter(|block| {
                                let is_anthropic_block = matches!(
                                    block.get("type").and_then(|t| t.as_str()),
                                    Some("thinking") | Some("redacted_thinking")
                                );
                                if !is_anthropic_block {
                                    return false;
                                }

                                // Anthropic validates integrity of thinking blocks.
                                // Blocks without a non-empty signature are invalid for replay.
                                block
                                    .get("signature")
                                    .and_then(|s| s.as_str())
                                    .is_some_and(|s| !s.trim().is_empty())
                            })
                            .cloned()
                            .collect();

                        if !keep.is_empty() {
                            obj["thinking_blocks"] = Value::Array(keep);
                        }
                    }
                }

                // Preserve interleaved text block ordering for Anthropic multi-turn.
                // LiteLLM proxies may forward these fields to Anthropic.
                if let Some(text_blocks) = msg.extra.get("_anthropic_text_blocks") {
                    if text_blocks.is_array() {
                        obj["_anthropic_text_blocks"] = text_blocks.clone();
                    }
                }
            }

            // Always strip encrypted citations in Refact wire — server_content_blocks
            // are never forwarded through LiteLLM, so encrypted citations would be
            // orphaned and cause validation errors on Anthropic round-trip.
            if !msg.citations.is_empty() {
                let valid_citations: Vec<&Value> = msg.citations.iter()
                    .filter(|c| c.get("encrypted_index").is_none())
                    .collect();
                if !valid_citations.is_empty() {
                    obj["citations"] = json!(valid_citations);
                }
            }

            Some(obj)
        })
        .collect()
}

/// Injects `cache_control` breakpoints into OpenAI-format messages for LiteLLM prompt caching.
///
/// Strategy:
///   - 4 message breakpoints, recomputed each request:
///     - last 2 non-system messages
///     - middle non-system message
///     - 1/4 point non-system message
///   - no system cache_control
///
/// For string content, converts to array-of-blocks format so `cache_control` can be attached.
/// LiteLLM passes these through to Anthropic's native API.
fn inject_cache_control(messages: &mut [Value]) {
    let cc = json!({"type": "ephemeral", "ttl": "1h"});

    fn add_cache_to_message(msg: &mut Value, cc: &Value) {
        let Some(content) = msg.get_mut("content") else { return };
        if let Some(text) = content.as_str().map(|s| s.to_string()) {
            // Convert string content to array-of-blocks format
            *content = json!([{"type": "text", "text": text, "cache_control": cc}]);
        } else if let Some(arr) = content.as_array_mut() {
            if let Some(last_block) = arr.last_mut() {
                if let Some(obj) = last_block.as_object_mut() {
                    obj.insert("cache_control".to_string(), cc.clone());
                }
            }
        }
    }

    // Cache selected non-system messages
    let non_system_indices: Vec<usize> = messages.iter().enumerate()
        .filter(|(_, m)| m.get("role").and_then(|r| r.as_str()) != Some("system"))
        .map(|(i, _)| i)
        .collect();

    let len = non_system_indices.len();
    let quarter_pos = len / 4;
    let middle_pos = len / 2;
    let last_pos = len - 1;
    let last2_pos = len.saturating_sub(2);

    let mut selected_positions = vec![quarter_pos, middle_pos, last2_pos, last_pos];
    selected_positions.sort_unstable();
    selected_positions.dedup();
    selected_positions.truncate(4);

    for pos in selected_positions {
        if let Some(&msg_idx) = non_system_indices.get(pos) {
            add_cache_to_message(&mut messages[msg_idx], &cc);
        }
    }
}

fn tool_choice_to_refact(choice: &CanonicalToolChoice) -> Value {
    match choice {
        CanonicalToolChoice::Auto => json!("auto"),
        CanonicalToolChoice::None => json!("none"),
        CanonicalToolChoice::Required => json!("required"),
        CanonicalToolChoice::Function { name } => json!({"type": "function", "function": {"name": name}}),
    }
}

fn parse_refact_usage(usage: &Value) -> Option<ChatUsage> {
    // Refact cloud uses LiteLLM to proxy various providers.
    //
    // OUTPUT CONTRACT (what all adapters must produce):
    //   prompt_tokens      = non-cached, non-cache-creation input tokens
    //                        (Anthropic's raw input_tokens)
    //   cache_creation     = tokens being newly written to cache
    //   cache_read         = tokens read from existing cache
    //   total_tokens       = prompt + completion + cache_creation + cache_read
    //
    // Context window used = prompt_tokens + cache_creation + cache_read
    //
    // LiteLLM bundles ALL input into prompt_tokens:
    //   prompt_tokens(LiteLLM) = input + cache_creation + cache_read
    // We subtract both cache_read AND cache_creation to isolate non-cached input.
    //
    // Cache fields location varies by provider:
    //   Anthropic: top-level cache_read_input_tokens, cache_creation_input_tokens
    //   OpenAI: prompt_tokens_details.cached_tokens (subset of prompt_tokens)
    //   LiteLLM may also nest Anthropic fields inside prompt_tokens_details

    let completion_tokens = parse_token_value(usage.get("completion_tokens"))
        .or_else(|| parse_token_value(usage.get("output_tokens")))
        .unwrap_or(0);

    // Anthropic-style cache fields (top-level, from LiteLLM passthrough).
    // Filter zeros so `.or()` falls through to nested/OpenAI fields correctly.
    let anthropic_cache_creation = parse_token_value(usage.get("cache_creation_input_tokens")).filter(|&v| v > 0);
    let anthropic_cache_read = parse_token_value(usage.get("cache_read_input_tokens")).filter(|&v| v > 0);

    // OpenAI-style cache fields (nested in prompt_tokens_details)
    let details = usage.get("prompt_tokens_details");
    let openai_cached = details.and_then(|d| parse_token_value(d.get("cached_tokens"))).filter(|&v| v > 0);
    // LiteLLM may also put Anthropic fields inside prompt_tokens_details
    let details_cache_creation = details.and_then(|d| parse_token_value(d.get("cache_creation_input_tokens"))).filter(|&v| v > 0);
    let details_cache_read = details.and_then(|d| parse_token_value(d.get("cache_read_input_tokens"))).filter(|&v| v > 0);

    // Merge: prefer top-level Anthropic fields, fall back to nested details
    let effective_cache_creation = anthropic_cache_creation.or(details_cache_creation);
    let effective_cache_read = anthropic_cache_read.or(details_cache_read).or(openai_cached);

    let raw_prompt = parse_token_value(usage.get("prompt_tokens")).unwrap_or(0);

    // Subtract both cache_read and cache_creation from prompt_tokens.
    // LiteLLM's prompt_tokens = input + cache_creation + cache_read (all input).
    // We need: prompt_tokens = input only (non-cached, non-creation).
    // Guard with saturating_sub for partial/delta chunks.
    let cache_read = effective_cache_read.unwrap_or(0);
    let cache_creation = effective_cache_creation.unwrap_or(0);
    let prompt_tokens = raw_prompt.saturating_sub(cache_read).saturating_sub(cache_creation);

    let total_tokens = prompt_tokens + completion_tokens + cache_creation + cache_read;

    let cache_creation_out = effective_cache_creation.filter(|&v| v > 0);
    let cache_read_out = effective_cache_read.filter(|&v| v > 0);

    Some(ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_creation_tokens: cache_creation_out,
        cache_read_tokens: cache_read_out,
        metering_usd: None,
    })
}

fn parse_token_value(value: Option<&Value>) -> Option<usize> {
    value.and_then(|v| {
        v.as_u64()
            .map(|n| n as usize)
            .or_else(|| v.as_str().and_then(|s| s.parse::<usize>().ok()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatMessage, ChatMeta};
    use reqwest::header::USER_AGENT;

    fn default_settings() -> AdapterSettings {
        AdapterSettings {
            api_key: "test-key".to_string(),
            auth_token: String::new(),
            endpoint: "https://inference.smallcloud.ai/v1/chat/completions".to_string(),
            extra_headers: Default::default(),
            model_name: "gpt-4".to_string(),
            supports_tools: true,
            supports_reasoning: false,
            reasoning_type: None,
            supports_temperature: true,
            supports_max_completion_tokens: false,
            support_metadata: true,
            eof_is_done: false,
            supports_web_search: false,
        }
    }

    fn anthropic_target_settings() -> AdapterSettings {
        AdapterSettings {
            supports_reasoning: true,
            reasoning_type: Some("anthropic_budget".to_string()),
            ..default_settings()
        }
    }

    #[test]
    fn test_user_agent_format() {
        let adapter = RefactAdapter;
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        let ua = http.headers.get(USER_AGENT).unwrap().to_str().unwrap();
        assert!(ua.starts_with("refact-lsp "), "UA should use space separator");
    }

    #[test]
    fn test_meta_included_when_provided() {
        let adapter = RefactAdapter;
        let meta = ChatMeta {
            chat_id: "test-123".to_string(),
            chat_mode: "agent".to_string(),
            ..Default::default()
        };
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]).with_meta(meta);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert!(http.body.get("meta").is_some());
        assert_eq!(http.body["meta"]["chat_id"], "test-123");
    }

    #[test]
    fn test_meta_not_included_when_absent() {
        let adapter = RefactAdapter;
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert!(http.body.get("meta").is_none());
    }

    #[test]
    fn test_no_backend_fields_in_request() {
        let adapter = RefactAdapter;
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert!(http.body.get("id").is_none());
        assert!(http.body.get("created").is_none());
        assert!(http.body.get("system_fingerprint").is_none());
    }

    #[test]
    fn test_stream_options_included() {
        let adapter = RefactAdapter;
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert_eq!(http.body["stream"], true);
        assert_eq!(http.body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn test_parallel_tool_calls_omitted_when_false() {
        let adapter = RefactAdapter;
        let tools = vec![json!({"type": "function", "function": {"name": "test"}})];
        let req = LlmRequest::new("gpt-4".to_string(), vec![])
            .with_tools(tools, Some(CanonicalToolChoice::Auto));

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert!(http.body.get("parallel_tool_calls").is_none(),
            "parallel_tool_calls should not be sent when false (default) to avoid litellm UnsupportedParamsError");
    }

    #[test]
    fn test_parallel_tool_calls_included_when_true() {
        let adapter = RefactAdapter;
        let tools = vec![json!({"type": "function", "function": {"name": "test"}})];
        let req = LlmRequest::new("gpt-4".to_string(), vec![])
            .with_tools(tools, Some(CanonicalToolChoice::Auto))
            .with_parallel_tool_calls(true);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert_eq!(http.body.get("parallel_tool_calls"), Some(&json!(true)),
            "parallel_tool_calls should be sent when explicitly enabled");
    }

    #[test]
    fn test_openai_reasoning_sends_effort_only() {
        use crate::llm::params::ReasoningIntent;

        let adapter = RefactAdapter;
        let mut settings = default_settings();
        settings.supports_reasoning = true;
        settings.reasoning_type = Some("openai".to_string());

        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]).with_reasoning(ReasoningIntent::High);

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(http.body["reasoning_effort"], "high");
        assert!(http.body.get("thinking").is_none(),
            "OpenAI models should not receive anthropic-style thinking param");
    }

    #[test]
    fn test_anthropic_reasoning_sends_thinking_only() {
        use crate::llm::params::ReasoningIntent;

        let adapter = RefactAdapter;
        let mut settings = default_settings();
        settings.supports_reasoning = true;
        settings.reasoning_type = Some("anthropic_budget".to_string());

        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]).with_reasoning(ReasoningIntent::High);

        let http = adapter.build_http(&req, &settings).unwrap();

        assert!(http.body.get("thinking").is_some());
        assert_eq!(http.body["thinking"]["type"], "enabled");
        assert_eq!(http.body["thinking"]["budget_tokens"], DEFAULT_THINKING_BUDGET);
        assert!(http.body.get("reasoning_effort").is_none(),
            "Anthropic models should not receive openai-style reasoning_effort param");
    }

    #[test]
    fn test_no_thinking_params_without_reasoning() {
        let adapter = RefactAdapter;
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert!(http.body.get("reasoning_effort").is_none());
        assert!(http.body.get("thinking").is_none());
    }
    #[test]
    fn test_temperature_omitted_when_unsupported() {
        let adapter = RefactAdapter;
        let mut settings = default_settings();
        settings.supports_temperature = false;

        let mut req = LlmRequest::new("gpt-5".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);
        req.params.temperature = Some(0.0);

        let http = adapter.build_http(&req, &settings).unwrap();

        assert!(http.body.get("temperature").is_none(),
            "temperature should not be sent when model does not support it");
    }


    #[test]
    fn test_parse_stream_with_metering() {
        let adapter = RefactAdapter;
        let chunk = r#"{"choices":[{"delta":{"content":"Hi"}}],"metering_balance":5000,"metering_prompt_tokens_n":10}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let has_content = deltas.iter().any(|d| matches!(d, LlmStreamDelta::AppendContent { text, .. } if text == "Hi"));
        let has_extra = deltas.iter().any(|d| matches!(d, LlmStreamDelta::MergeExtra { extra } if extra.contains_key("metering_balance")));

        assert!(has_content);
        assert!(has_extra);
    }

    #[test]
    fn test_parse_fastapi_error_string() {
        let adapter = RefactAdapter;
        let chunk = r#"{"detail": "The version of your Refact plugin is no longer supported"}"#;

        let result = adapter.parse_stream_chunk(chunk);

        match result {
            Err(StreamParseError::FatalError(msg)) => {
                assert!(msg.contains("no longer supported"));
            }
            _ => panic!("expected FatalError"),
        }
    }

    #[test]
    fn test_parse_fastapi_error_object() {
        let adapter = RefactAdapter;
        let chunk = r#"{"detail": {"code": "version_error", "message": "Update required"}}"#;

        let result = adapter.parse_stream_chunk(chunk);

        match result {
            Err(StreamParseError::FatalError(msg)) => {
                assert!(msg.contains("version_error"));
            }
            _ => panic!("expected FatalError"),
        }
    }

    #[test]
    fn test_parse_openai_error() {
        let adapter = RefactAdapter;
        let chunk = r#"{"error": {"message": "Rate limit exceeded", "type": "rate_limit"}}"#;

        let result = adapter.parse_stream_chunk(chunk);

        match result {
            Err(StreamParseError::FatalError(msg)) => {
                assert_eq!(msg, "Rate limit exceeded");
            }
            _ => panic!("expected FatalError"),
        }
    }

    #[test]
    fn test_parse_done() {
        let adapter = RefactAdapter;
        let deltas = adapter.parse_stream_chunk("[DONE]").unwrap();

        assert_eq!(deltas.len(), 1);
        assert!(matches!(deltas[0], LlmStreamDelta::Done));
    }

    #[test]
    fn test_parse_tool_calls_delta() {
        let adapter = RefactAdapter;
        let chunk = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_123","function":{"name":"test","arguments":""}}]}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let has_tool_calls = deltas.iter().any(|d| matches!(d, LlmStreamDelta::SetToolCalls { tool_calls } if !tool_calls.is_empty()));
        assert!(has_tool_calls);
    }

    #[test]
    fn test_n_parameter_included() {
        let adapter = RefactAdapter;
        let mut req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);
        req.params.n = Some(2);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert_eq!(http.body["n"], 2);
    }

    #[test]
    fn test_diff_role_converted_to_tool() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Edit the file".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                    id: "call_edit".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: crate::call_validation::ChatToolFunction {
                        name: "file_edit".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "diff".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("@@ -1 +1 @@\n-old\n+new".to_string()),
                tool_call_id: "call_edit".to_string(),
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, None);

        assert_eq!(converted.len(), 3);
        assert_eq!(converted[2]["role"], "tool");
        assert_eq!(converted[2]["tool_call_id"], "call_edit");
    }

    #[test]
    fn test_stream_citations_in_delta() {
        let adapter = RefactAdapter;
        let chunk = r#"{"id":"123","choices":[{"index":0,"delta":{"citations":[{"url":"https://example.com","title":"Example","snippet":"Some text"}]}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        let citation_count = deltas.iter().filter(|d| matches!(d, LlmStreamDelta::AddCitation { .. })).count();
        assert_eq!(citation_count, 1);

        // Verify citation content
        if let Some(LlmStreamDelta::AddCitation { citation }) = deltas.iter().find(|d| matches!(d, LlmStreamDelta::AddCitation { .. })) {
            assert_eq!(citation.get("url").and_then(|v| v.as_str()), Some("https://example.com"));
            assert_eq!(citation.get("title").and_then(|v| v.as_str()), Some("Example"));
        }
    }

    #[test]
    fn test_parse_usage_litellm_anthropic_style() {
        // LiteLLM includes ALL input in prompt_tokens:
        //   prompt_tokens = 1500 = 200(non-cached) + 300(creation) + 1000(read)
        let usage = serde_json::json!({
            "prompt_tokens": 1500,
            "completion_tokens": 200,
            "cache_read_input_tokens": 1000,
            "cache_creation_input_tokens": 300,
            "total_tokens": 1700
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        // prompt_tokens = 1500 - 1000(read) - 300(creation) = 200 (non-cached only)
        assert_eq!(parsed.prompt_tokens, 200);
        assert_eq!(parsed.completion_tokens, 200);
        assert_eq!(parsed.cache_creation_tokens, Some(300));
        assert_eq!(parsed.cache_read_tokens, Some(1000));
        // total = 200 + 200 + 300 + 1000 = 1700
        assert_eq!(parsed.total_tokens, 1700);
    }

    #[test]
    fn test_parse_usage_litellm_anthropic_no_cache() {
        // LiteLLM Anthropic with no cache (first request, below min cacheable)
        let usage = serde_json::json!({
            "prompt_tokens": 500,
            "completion_tokens": 200,
            "cache_read_input_tokens": 0,
            "cache_creation_input_tokens": 0,
            "total_tokens": 700
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        assert_eq!(parsed.prompt_tokens, 500);
        assert_eq!(parsed.completion_tokens, 200);
        assert_eq!(parsed.cache_creation_tokens, None);
        assert_eq!(parsed.cache_read_tokens, None);
        assert_eq!(parsed.total_tokens, 700);
    }

    #[test]
    fn test_parse_usage_litellm_anthropic_cache_read_only() {
        // LiteLLM Anthropic: second request, most tokens from cache
        // prompt_tokens = 5100 (includes 5000 cache_read), non-cached = 100
        let usage = serde_json::json!({
            "prompt_tokens": 5100,
            "completion_tokens": 200,
            "cache_read_input_tokens": 5000,
            "cache_creation_input_tokens": 0,
            "total_tokens": 5300
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        // prompt_tokens normalized: 5100 - 5000 = 100
        assert_eq!(parsed.prompt_tokens, 100);
        assert_eq!(parsed.completion_tokens, 200);
        assert_eq!(parsed.cache_creation_tokens, None);
        assert_eq!(parsed.cache_read_tokens, Some(5000));
        // total recomputed: 100 + 200 + 0 + 5000 = 5300
        assert_eq!(parsed.total_tokens, 5300);
    }

    #[test]
    fn test_parse_usage_zero_top_level_falls_through_to_nested() {
        // Top-level cache_read_input_tokens=0 should not block nested cached_tokens
        let usage = serde_json::json!({
            "prompt_tokens": 1500,
            "completion_tokens": 200,
            "cache_read_input_tokens": 0,
            "prompt_tokens_details": { "cached_tokens": 1000 }
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        // Should use nested cached_tokens=1000: 1500 - 1000 = 500
        assert_eq!(parsed.prompt_tokens, 500);
        assert_eq!(parsed.cache_read_tokens, Some(1000));
        assert_eq!(parsed.total_tokens, 1700);
    }

    #[test]
    fn test_parse_usage_cache_read_exceeds_prompt_saturates() {
        // Partial/delta chunk where cache_read > prompt_tokens (e.g., streaming)
        // saturating_sub clamps to 0
        let usage = serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 200,
            "cache_read_input_tokens": 5000
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        // saturating_sub: 100 - 5000 = 0
        assert_eq!(parsed.prompt_tokens, 0);
        assert_eq!(parsed.cache_read_tokens, Some(5000));
        // total = 0 + 200 + 5000 = 5200
        assert_eq!(parsed.total_tokens, 5200);
    }

    #[test]
    fn test_parse_usage_cache_creation_in_details_only() {
        // Cache creation nested in prompt_tokens_details (LiteLLM oddity)
        // prompt_tokens = 1000 = 200(non-cached) + 300(creation) + 500(read)
        let usage = serde_json::json!({
            "prompt_tokens": 1000,
            "completion_tokens": 200,
            "prompt_tokens_details": {
                "cache_creation_input_tokens": 300,
                "cached_tokens": 500
            }
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        // 1000 - 500(read) - 300(creation) = 200
        assert_eq!(parsed.prompt_tokens, 200);
        assert_eq!(parsed.cache_creation_tokens, Some(300));
        assert_eq!(parsed.cache_read_tokens, Some(500));
        // total = 200 + 200 + 300 + 500 = 1200
        assert_eq!(parsed.total_tokens, 1200);
    }

    #[test]
    fn test_parse_usage_openai_style_no_cache() {
        // Standard OpenAI without cache info
        let usage = serde_json::json!({
            "prompt_tokens": 1000,
            "completion_tokens": 200,
            "total_tokens": 1200
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        assert_eq!(parsed.prompt_tokens, 1000);
        assert_eq!(parsed.completion_tokens, 200);
        assert_eq!(parsed.cache_creation_tokens, None);
        assert_eq!(parsed.cache_read_tokens, None);
        assert_eq!(parsed.total_tokens, 1200);
    }

    #[test]
    fn test_parse_usage_openai_with_cached_tokens_details() {
        // OpenAI: prompt_tokens includes cached, details breaks it down
        let usage = serde_json::json!({
            "prompt_tokens": 1500,
            "completion_tokens": 200,
            "prompt_tokens_details": {
                "cached_tokens": 1000
            },
            "total_tokens": 1700
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        // prompt_tokens normalized: 1500 - 1000 = 500
        assert_eq!(parsed.prompt_tokens, 500);
        assert_eq!(parsed.completion_tokens, 200);
        assert_eq!(parsed.cache_read_tokens, Some(1000));
        assert_eq!(parsed.cache_creation_tokens, None);
        // total recomputed: 500 + 200 + 1000 = 1700
        assert_eq!(parsed.total_tokens, 1700);
    }

    #[test]
    fn test_parse_usage_string_numbers() {
        let usage = serde_json::json!({
            "prompt_tokens": "1000",
            "completion_tokens": "200",
            "total_tokens": "1200"
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        assert_eq!(parsed.prompt_tokens, 1000);
        assert_eq!(parsed.completion_tokens, 200);
        assert_eq!(parsed.total_tokens, 1200);
    }

    #[test]
    fn test_parse_usage_message_delta_output_only() {
        // Anthropic message_delta via LiteLLM: only output_tokens updated
        // (LiteLLM may send just completion_tokens in delta)
        let usage = serde_json::json!({
            "prompt_tokens": 0,
            "completion_tokens": 500
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        assert_eq!(parsed.prompt_tokens, 0);
        assert_eq!(parsed.completion_tokens, 500);
        assert_eq!(parsed.total_tokens, 500);
    }

    #[test]
    fn test_parse_stream_thinking_blocks_with_signature() {
        let adapter = RefactAdapter;
        let chunk = r#"{"choices":[{"delta":{"content":"","reasoning_content":"Let me think","thinking_blocks":[{"type":"thinking","thinking":"Let me think","signature":"sig_abc123"}]}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let has_reasoning = deltas.iter().any(|d| matches!(d, LlmStreamDelta::AppendReasoning { .. }));
        let has_thinking = deltas.iter().any(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. }));
        assert!(has_reasoning, "Should emit AppendReasoning");
        assert!(has_thinking, "Should emit SetThinkingBlocks for signed blocks");

        if let Some(LlmStreamDelta::SetThinkingBlocks { blocks }) = deltas.iter().find(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. })) {
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0]["signature"], "sig_abc123");
        }
    }

    #[test]
    fn test_parse_stream_thinking_blocks_empty_signature_skipped() {
        let adapter = RefactAdapter;
        // Some proxies stream thinking_blocks with signature="" (empty) during streaming.
        // These must be skipped; otherwise we will resend invalid signatures on the next turn.
        let chunk = r#"{"choices":[{"delta":{"thinking_blocks":[{"type":"thinking","thinking":"partial","signature":""}]}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        let has_thinking = deltas.iter().any(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. }));
        assert!(!has_thinking, "Empty-string signature blocks must be skipped");
    }

    #[test]
    fn test_parse_stream_thinking_blocks_without_signature_skipped() {
        let adapter = RefactAdapter;
        // LiteLLM sends partial thinking_blocks without signature during streaming
        let chunk = r#"{"choices":[{"delta":{"reasoning_content":"partial","thinking_blocks":[{"type":"thinking","thinking":"partial","signature":null}]}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        // Should have reasoning but NOT thinking blocks (signature is null, not a string)
        let has_reasoning = deltas.iter().any(|d| matches!(d, LlmStreamDelta::AppendReasoning { .. }));
        let has_thinking = deltas.iter().any(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. }));
        assert!(has_reasoning);
        assert!(!has_thinking, "Should skip thinking blocks without valid signature");
    }

    #[test]
    fn test_parse_stream_thinking_blocks_redacted() {
        let adapter = RefactAdapter;
        let chunk = r#"{"choices":[{"delta":{"thinking_blocks":[{"type":"redacted_thinking","data":"encrypted_data","signature":"sig_redacted"}]}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let has_thinking = deltas.iter().any(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. }));
        assert!(has_thinking, "Should capture redacted thinking blocks with signature");
    }

    #[test]
    fn test_thinking_blocks_included_in_assistant() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Solve this".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Answer".to_string()),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "index": 5,
                    "thinking": "Let me reason...",
                    "signature": "sig_abc123"
                })]),
                tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: crate::call_validation::ChatToolFunction {
                        name: "search".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: Some(7),
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Result".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, Some("anthropic_budget"));

        assert_eq!(converted.len(), 3);
        // Assistant message should have thinking_blocks when targeting Anthropic
        let assistant = &converted[1];
        assert!(assistant.get("thinking_blocks").is_some(),
            "Assistant message should include thinking_blocks for LiteLLM Anthropic target");
        let blocks = assistant["thinking_blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "thinking");
        assert_eq!(blocks[0]["signature"], "sig_abc123");
        assert_eq!(blocks[0]["index"], 5);

        let tool_calls = assistant["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls[0]["index"], 7);
    }

    #[test]
    fn test_no_thinking_blocks_when_none() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Hi".to_string()),
                thinking_blocks: None,
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, None);

        assert_eq!(converted.len(), 2);
        assert!(converted[1].get("thinking_blocks").is_none(),
            "No thinking_blocks field when None");
    }

    #[test]
    fn test_empty_thinking_blocks_not_included() {
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Hi".to_string()),
                thinking_blocks: Some(vec![]),
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, None);

        assert!(converted[0].get("thinking_blocks").is_none(),
            "Empty thinking_blocks should not be included");
    }

    #[test]
    fn test_reasoning_content_included_in_assistant() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Solve this".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("The answer is 42".to_string()),
                reasoning_content: Some("Let me think about this problem...".to_string()),
                tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    extra_content: None,
                    function: crate::call_validation::ChatToolFunction {
                        name: "search".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Result".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, Some("openai"));

        assert_eq!(converted.len(), 3);
        let assistant = &converted[1];
        assert_eq!(assistant["reasoning_content"], "Let me think about this problem...");
        assert_eq!(assistant["content"], "The answer is 42");
        assert!(assistant.get("tool_calls").is_some());
    }

    #[test]
    fn test_reasoning_content_not_included_when_none() {
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Hi".to_string()),
                reasoning_content: None,
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, None);

        assert!(converted[0].get("reasoning_content").is_none());
    }

    #[test]
    fn test_reasoning_content_not_included_when_empty() {
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Hi".to_string()),
                reasoning_content: Some(String::new()),
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, None);

        assert!(converted[0].get("reasoning_content").is_none());
    }

    #[test]
    fn test_cache_control_ephemeral_injects_into_messages() {
        let adapter = RefactAdapter;
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("system".to_string(), "You are helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage::new("assistant".to_string(), "Hi there".to_string()),
            ChatMessage::new("user".to_string(), "How are you?".to_string()),
        ]).with_cache_control(CacheControl::Ephemeral);

        let http = adapter.build_http(&req, &anthropic_target_settings()).unwrap();

        // No top-level cache_control field
        assert!(http.body.get("cache_control").is_none(),
            "cache_control should be in messages, not top-level");

        let messages = http.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4);

        // System message: no cache_control injected
        let sys = &messages[0];
        assert!(sys["content"].is_string());

        // Non-system slice is [1,2,3] => len=3
        // quarter=0, middle=1, last2=1, last=2 => selected positions {0,1,2}
        // => all 3 non-system messages should be converted to blocks and have cache_control.
        for idx in 1..=3 {
            let content = messages[idx]["content"].as_array().unwrap();
            assert!(content.last().unwrap().get("cache_control").is_some());
        }
    }

    #[test]
    fn test_cache_control_ephemeral_single_message() {
        let adapter = RefactAdapter;
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]).with_cache_control(CacheControl::Ephemeral);

        let http = adapter.build_http(&req, &anthropic_target_settings()).unwrap();

        let messages = http.body["messages"].as_array().unwrap();
        // Single non-system message: first == last, should get cache_control once
        let content = messages[0]["content"].as_array().unwrap();
        assert!(content[0].get("cache_control").is_some());
    }

    #[test]
    fn test_cache_control_off_no_injection() {
        let adapter = RefactAdapter;
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("system".to_string(), "You are helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert!(http.body.get("cache_control").is_none());
        let messages = http.body["messages"].as_array().unwrap();
        // System content should be plain string, not array
        assert!(messages[0]["content"].is_string());
        // User content should be plain string
        assert!(messages[1]["content"].is_string());
    }

    #[test]
    fn test_cache_control_ephemeral_non_anthropic_target_no_injection() {
        // Regression: Gemini/Vertex rejects cache controls alongside
        // system instruction / tools / tool config.
        let adapter = RefactAdapter;
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ])
        .with_cache_control(CacheControl::Ephemeral);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        let messages = http.body["messages"].as_array().unwrap();
        assert!(messages[0]["content"].is_string(), "Should not convert to multipart");
    }

    #[test]
    fn test_cache_control_multimodal_content() {
        use crate::scratchpads::multimodality::MultimodalElement;
        let adapter = RefactAdapter;
        let multimodal_msg = ChatMessage {
            role: "user".to_string(),
            content: crate::call_validation::ChatContent::Multimodal(vec![
                MultimodalElement {
                    m_type: "text/plain".to_string(),
                    m_content: "Describe this".to_string(),
                },
                MultimodalElement {
                    m_type: "image/png".to_string(),
                    m_content: "base64data".to_string(),
                },
            ]),
            ..Default::default()
        };
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            multimodal_msg,
        ]).with_cache_control(CacheControl::Ephemeral);

        let http = adapter.build_http(&req, &anthropic_target_settings()).unwrap();

        let messages = http.body["messages"].as_array().unwrap();
        let content = messages[0]["content"].as_array().unwrap();
        // cache_control should be on the last block (image)
        assert!(content.last().unwrap().get("cache_control").is_some(),
            "cache_control should be on last content block");
        // First block should NOT have cache_control
        assert!(content[0].get("cache_control").is_none(),
            "Only last block should have cache_control");
    }

    #[test]
    fn test_extra_body_merged() {
        let adapter = RefactAdapter;
        let mut extra = serde_json::Map::new();
        extra.insert("web_search_options".to_string(), json!({"search_context_size": "medium"}));
        extra.insert("custom_field".to_string(), json!("value"));

        let mut req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);
        req.extra_body = Some(extra);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert_eq!(http.body["web_search_options"]["search_context_size"], "medium");
        assert_eq!(http.body["custom_field"], "value");
    }

    #[test]
    fn test_extra_body_protected_fields_ignored() {
        let adapter = RefactAdapter;
        let mut extra = serde_json::Map::new();
        extra.insert("model".to_string(), json!("evil-model"));
        extra.insert("messages".to_string(), json!([]));
        extra.insert("allowed_field".to_string(), json!("ok"));

        let mut req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);
        req.extra_body = Some(extra);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert_eq!(http.body["model"], "gpt-4");
        assert_ne!(http.body["messages"], json!([]));
        assert_eq!(http.body["allowed_field"], "ok");
    }

    #[test]
    fn test_stream_anthropic_citation_in_provider_specific_fields() {
        // LiteLLM streams Anthropic citations as delta.provider_specific_fields.citation (singular)
        let adapter = RefactAdapter;
        let chunk = r#"{"id":"123","choices":[{"index":0,"delta":{"content":"the grass is green","provider_specific_fields":{"citation":{"type":"char_location","cited_text":"The grass is green.","document_index":0,"document_title":"My Document","start_char_index":0,"end_char_index":20}}}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let has_content = deltas.iter().any(|d| matches!(d, LlmStreamDelta::AppendContent { text, .. } if text == "the grass is green"));
        assert!(has_content, "Should have content delta");

        let citation_count = deltas.iter().filter(|d| matches!(d, LlmStreamDelta::AddCitation { .. })).count();
        assert_eq!(citation_count, 1, "Should have exactly one citation from provider_specific_fields");

        if let Some(LlmStreamDelta::AddCitation { citation }) = deltas.iter().find(|d| matches!(d, LlmStreamDelta::AddCitation { .. })) {
            assert_eq!(citation.get("type").and_then(|v| v.as_str()), Some("char_location"));
            assert_eq!(citation.get("cited_text").and_then(|v| v.as_str()), Some("The grass is green."));
            assert_eq!(citation.get("document_index").and_then(|v| v.as_u64()), Some(0));
        }
    }

    #[test]
    fn test_stream_anthropic_citations_array_in_provider_specific_fields() {
        // LiteLLM may also return citations as an array in provider_specific_fields
        let adapter = RefactAdapter;
        let chunk = r#"{"id":"123","choices":[{"index":0,"delta":{"content":"colors","provider_specific_fields":{"citations":[{"type":"char_location","cited_text":"The grass is green.","document_index":0,"start_char_index":0,"end_char_index":20},{"type":"char_location","cited_text":"The sky is blue.","document_index":0,"start_char_index":20,"end_char_index":36}]}}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let citation_count = deltas.iter().filter(|d| matches!(d, LlmStreamDelta::AddCitation { .. })).count();
        assert_eq!(citation_count, 2, "Should have two citations from provider_specific_fields array");
    }

    #[test]
    fn test_non_streaming_anthropic_citations_in_message() {
        // Non-streaming LiteLLM response: citations in message.provider_specific_fields.citations
        let adapter = RefactAdapter;
        let chunk = r#"{"id":"msg_123","choices":[{"index":0,"message":{"role":"assistant","content":"The grass is green and the sky is blue.","provider_specific_fields":{"citations":[{"type":"char_location","cited_text":"The grass is green.","document_index":0,"document_title":"My Document","start_char_index":0,"end_char_index":20}]}},"finish_reason":"stop"}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let citation_count = deltas.iter().filter(|d| matches!(d, LlmStreamDelta::AddCitation { .. })).count();
        assert_eq!(citation_count, 1, "Should extract citation from non-streaming message.provider_specific_fields");

        if let Some(LlmStreamDelta::AddCitation { citation }) = deltas.iter().find(|d| matches!(d, LlmStreamDelta::AddCitation { .. })) {
            assert_eq!(citation.get("type").and_then(|v| v.as_str()), Some("char_location"));
            assert_eq!(citation.get("document_title").and_then(|v| v.as_str()), Some("My Document"));
        }

        let has_finish = deltas.iter().any(|d| matches!(d, LlmStreamDelta::SetFinishReason { reason } if reason == "stop"));
        assert!(has_finish, "Should also have finish reason");
    }

    #[test]
    fn test_null_citation_in_provider_specific_fields_ignored() {
        // LiteLLM sends provider_specific_fields.citations: null when no citations
        let adapter = RefactAdapter;
        let chunk = r#"{"id":"123","choices":[{"index":0,"delta":{"content":"hello","provider_specific_fields":{"citation":null,"citations":null}}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let citation_count = deltas.iter().filter(|d| matches!(d, LlmStreamDelta::AddCitation { .. })).count();
        assert_eq!(citation_count, 0, "Null citations should be ignored");

        let has_content = deltas.iter().any(|d| matches!(d, LlmStreamDelta::AppendContent { text, .. } if text == "hello"));
        assert!(has_content, "Content should still be parsed");
    }

    #[test]
    fn test_citations_resent_in_multi_turn() {
        // Citations from prior assistant responses should be included when re-sending messages
        let messages = vec![
            ChatMessage::new("user".to_string(), "What color is the grass?".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("The grass is green.".to_string()),
                citations: vec![
                    json!({
                        "type": "char_location",
                        "cited_text": "The grass is green.",
                        "document_index": 0,
                        "document_title": "My Document",
                        "start_char_index": 0,
                        "end_char_index": 20
                    }),
                ],
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "And the sky?".to_string()),
        ];

        let converted = convert_messages_to_refact(&messages, None);

        assert_eq!(converted.len(), 3);
        // Assistant message should have citations
        let assistant = &converted[1];
        assert!(assistant.get("citations").is_some(),
            "Assistant message should include citations for multi-turn context");
        let citations = assistant["citations"].as_array().unwrap();
        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0]["type"], "char_location");
        assert_eq!(citations[0]["cited_text"], "The grass is green.");
    }

    #[test]
    fn test_empty_citations_not_included() {
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Hi".to_string()),
                citations: vec![],
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, None);

        assert!(converted[0].get("citations").is_none(),
            "Empty citations should not be included");
    }

    #[test]
    fn test_anthropic_pdf_page_location_citation() {
        // Anthropic PDF citations use page_location type
        let adapter = RefactAdapter;
        let chunk = r#"{"id":"123","choices":[{"index":0,"delta":{"content":"water is essential","provider_specific_fields":{"citation":{"type":"page_location","cited_text":"Water is essential for life.","document_index":1,"document_title":"PDF Document","start_page_number":5,"end_page_number":6}}}}]}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        let citation_count = deltas.iter().filter(|d| matches!(d, LlmStreamDelta::AddCitation { .. })).count();
        assert_eq!(citation_count, 1);

        if let Some(LlmStreamDelta::AddCitation { citation }) = deltas.iter().find(|d| matches!(d, LlmStreamDelta::AddCitation { .. })) {
            assert_eq!(citation.get("type").and_then(|v| v.as_str()), Some("page_location"));
            assert_eq!(citation.get("start_page_number").and_then(|v| v.as_u64()), Some(5));
            assert_eq!(citation.get("end_page_number").and_then(|v| v.as_u64()), Some(6));
        }
    }

    #[test]
    fn test_empty_thinking_blocks_filtered_in_convert() {
        use crate::call_validation::{ChatContent, ChatMessage};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("Response".to_string()),
                thinking_blocks: Some(vec![
                    json!({"type": "thinking", "thinking": "", "signature": "sig_empty"}),
                ]),
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, Some("anthropic_budget"));

        let assistant = &converted[1];
        // LiteLLM may send signed thinking blocks with empty thinking text ("thinking": "")
        // when the thinking content was streamed separately via reasoning_content.
        // For multi-turn compatibility, we must preserve these blocks verbatim.
        assert!(assistant.get("thinking_blocks").is_some());
        let blocks = assistant["thinking_blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "thinking");
        assert_eq!(blocks[0]["signature"], "sig_empty");
        assert_eq!(blocks[0]["thinking"], "");
    }

    #[test]
    fn test_valid_thinking_block_kept_empty_filtered_in_refact() {
        use crate::call_validation::{ChatContent, ChatMessage};

        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("Response".to_string()),
                thinking_blocks: Some(vec![
                    json!({"type": "thinking", "thinking": "Valid text", "signature": "sig1"}),
                    json!({"type": "thinking", "thinking": "", "signature": "sig_empty"}),
                    json!({"type": "redacted_thinking", "data": "enc", "signature": "sig_r"}),
                ]),
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, Some("anthropic_budget"));

        let blocks = converted[0]["thinking_blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 3, "All Anthropic blocks must be preserved verbatim: {:?}", blocks);
        assert_eq!(blocks[0]["thinking"], "Valid text");
        assert_eq!(blocks[1]["thinking"], "");
        assert_eq!(blocks[2]["type"], "redacted_thinking");
    }

    #[test]
    fn test_thinking_blocks_stripped_for_non_anthropic_target() {
        use crate::call_validation::{ChatContent, ChatMessage};

        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("Response".to_string()),
                thinking_blocks: Some(vec![
                    json!({"type": "thinking", "thinking": "Valid text", "signature": "sig1"}),
                ]),
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, Some("openai"));
        assert!(converted[0].get("thinking_blocks").is_none(),
            "thinking_blocks should be stripped for non-Anthropic targets");

        let converted_none = convert_messages_to_refact(&messages, None);
        assert!(converted_none[0].get("thinking_blocks").is_none(),
            "thinking_blocks should be stripped when no reasoning_type");
    }

    #[test]
    fn test_reasoning_content_stripped_when_no_reasoning_support() {
        use crate::call_validation::{ChatContent, ChatMessage};

        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("Response".to_string()),
                reasoning_content: Some("Reasoning text".to_string()),
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, None);
        assert!(converted[0].get("reasoning_content").is_none(),
            "reasoning_content should be stripped when no reasoning support");

        let converted_openai = convert_messages_to_refact(&messages, Some("openai"));
        assert_eq!(converted_openai[0]["reasoning_content"], "Reasoning text",
            "reasoning_content should be included for openai reasoning");
    }

    #[test]
    fn test_encrypted_citations_always_stripped_in_refact() {
        use crate::call_validation::{ChatContent, ChatMessage};

        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("Search results".to_string()),
                citations: vec![
                    json!({
                        "type": "web_search_result_location",
                        "url": "https://example.com",
                        "title": "Example",
                        "encrypted_index": "abc123",
                        "cited_text": "Found it."
                    }),
                    json!({
                        "type": "char_location",
                        "cited_text": "Local doc.",
                        "document_index": 0,
                        "start_char_index": 0,
                        "end_char_index": 10
                    }),
                ],
                server_content_blocks: vec![json!({"type": "server_tool_use"})],
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_refact(&messages, Some("anthropic_budget"));
        let citations = converted[0]["citations"].as_array().unwrap();
        assert_eq!(citations.len(), 1,
            "Encrypted citations should always be stripped in Refact wire");
        assert_eq!(citations[0]["type"], "char_location");
    }
}
