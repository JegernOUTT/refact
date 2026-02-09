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
    "reasoning_effort", "thinking", "meta", "parallel_tool_calls",
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

        let mut messages = convert_messages_to_refact(&req.messages);

        if matches!(req.cache_control, CacheControl::Ephemeral) {
            inject_cache_control(&mut messages);
        }

        let mut body = json!({
            "model": settings.model_name,
            "messages": messages,
            "stream": req.stream,
        });

        if req.params.max_tokens > 0 {
            body["max_completion_tokens"] = json!(req.params.max_tokens);
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
                "anthropic_budget" | "anthropic_effort" => {
                    if let Some(budget) = req.reasoning.to_anthropic_budget(DEFAULT_THINKING_BUDGET) {
                        body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
                    }
                }
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
                            });
                        }
                    }

                    if let Some(reasoning) = delta.get("reasoning_content").and_then(|r| r.as_str()) {
                        if !reasoning.is_empty() {
                            deltas.push(LlmStreamDelta::AppendReasoning {
                                text: reasoning.to_string(),
                            });
                        }
                    }

                    // LiteLLM streams thinking_blocks with signatures for Anthropic models.
                    // Each chunk may contain partial blocks; blocks with a signature are final.
                    if let Some(blocks) = delta.get("thinking_blocks").and_then(|b| b.as_array()) {
                        let signed: Vec<Value> = blocks.iter()
                            .filter(|b| b.get("signature").and_then(|s| s.as_str()).is_some())
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

fn convert_messages_to_refact(messages: &[crate::call_validation::ChatMessage]) -> Vec<Value> {
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

            if let Some(reasoning) = &msg.reasoning_content {
                if !reasoning.is_empty() {
                    obj["reasoning_content"] = json!(reasoning);
                }
            }

            if let Some(blocks) = &msg.thinking_blocks {
                if !blocks.is_empty() {
                    obj["thinking_blocks"] = json!(blocks);
                }
            }

            Some(obj)
        })
        .collect()
}

/// Injects `cache_control` breakpoints into OpenAI-format messages for LiteLLM prompt caching.
///
/// Strategy (matching anthropic.rs):
///   1. System message — static instructions, always stable
///   2. First non-system message [0] — stable prefix anchor
///   3. Last non-system message [-1] — current turn, caches full prefix
///
/// For string content, converts to array-of-blocks format so `cache_control` can be attached.
/// LiteLLM passes these through to Anthropic's native API.
fn inject_cache_control(messages: &mut [Value]) {
    let cc = json!({"type": "ephemeral"});

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

    // Cache system message
    if let Some(sys_msg) = messages.iter_mut().find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system")) {
        add_cache_to_message(sys_msg, &cc);
    }

    // Cache first and last non-system messages
    let non_system_indices: Vec<usize> = messages.iter().enumerate()
        .filter(|(_, m)| m.get("role").and_then(|r| r.as_str()) != Some("system"))
        .map(|(i, _)| i)
        .collect();

    if let Some(&first_idx) = non_system_indices.first() {
        add_cache_to_message(&mut messages[first_idx], &cc);
    }
    if non_system_indices.len() >= 2 {
        if let Some(&last_idx) = non_system_indices.last() {
            add_cache_to_message(&mut messages[last_idx], &cc);
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
    // Refact cloud uses LiteLLM to proxy various providers. Each has different
    // usage semantics that we must normalize to a consistent format:
    //
    // OUTPUT CONTRACT (what UI expects):
    //   prompt_tokens = NON-cached input tokens only
    //   cache_creation_tokens = newly cached tokens
    //   cache_read_tokens = tokens read from cache
    //   total_tokens = prompt + completion + cache_creation + cache_read
    //   UI displays context as: prompt_tokens + cache_creation + cache_read
    //
    // PROVIDER SEMANTICS:
    //
    // LiteLLM proxying Anthropic:
    //   prompt_tokens = Anthropic's input_tokens (NON-cached, already correct)
    //   completion_tokens = output_tokens
    //   total_tokens = input_tokens + output_tokens (does NOT include cache)
    //   cache_creation_input_tokens = passed through at top level
    //   cache_read_input_tokens = passed through at top level
    //   → Just pass prompt_tokens through, recompute total to include cache
    //
    // OpenAI (native or via LiteLLM):
    //   prompt_tokens = TOTAL input (includes cached tokens)
    //   prompt_tokens_details.cached_tokens = cached subset
    //   total_tokens = prompt_tokens + completion_tokens (includes everything)
    //   → Subtract cached_tokens from prompt_tokens to get non-cached
    //
    // Plain OpenAI (no caching):
    //   prompt_tokens = total input
    //   total_tokens = prompt + completion
    //   → Pass through as-is

    let completion_tokens = parse_token_value(usage.get("completion_tokens"))
        .or_else(|| parse_token_value(usage.get("output_tokens")))
        .unwrap_or(0);

    // Check for Anthropic-style cache fields (top-level, from LiteLLM passthrough)
    let anthropic_cache_creation = parse_token_value(usage.get("cache_creation_input_tokens"));
    let anthropic_cache_read = parse_token_value(usage.get("cache_read_input_tokens"));

    // Check for OpenAI-style cache fields (nested in prompt_tokens_details)
    let details = usage.get("prompt_tokens_details");
    let openai_cached = details.and_then(|d| parse_token_value(d.get("cached_tokens")));
    // LiteLLM may also put Anthropic fields inside prompt_tokens_details
    let details_cache_creation = details.and_then(|d| parse_token_value(d.get("cache_creation_input_tokens")));
    let details_cache_read = details.and_then(|d| parse_token_value(d.get("cache_read_input_tokens")));

    let has_anthropic_cache = anthropic_cache_creation.is_some()
        || anthropic_cache_read.is_some()
        || details_cache_creation.is_some()
        || details_cache_read.is_some();

    let effective_cache_creation = anthropic_cache_creation.or(details_cache_creation);
    let effective_cache_read = anthropic_cache_read.or(details_cache_read).or(openai_cached);

    let raw_prompt = parse_token_value(usage.get("prompt_tokens")).unwrap_or(0);

    let prompt_tokens = if has_anthropic_cache {
        // Anthropic via LiteLLM: prompt_tokens = Anthropic's input_tokens
        // This is already the NON-cached portion. Pass through as-is.
        raw_prompt
    } else if let Some(cached) = openai_cached {
        // OpenAI: prompt_tokens includes cached tokens. Subtract to get non-cached.
        raw_prompt.saturating_sub(cached)
    } else {
        // No caching info: pass through as-is
        raw_prompt
    };

    // Always recompute total to include cache tokens (provider's total may exclude them)
    let total_tokens = prompt_tokens
        + completion_tokens
        + effective_cache_creation.unwrap_or(0)
        + effective_cache_read.unwrap_or(0);

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
            endpoint: "https://app.refact.ai/v1/chat/completions".to_string(),
            extra_headers: Default::default(),
            model_name: "gpt-4".to_string(),
            supports_tools: true,
            supports_reasoning: false,
            reasoning_type: None,
            supports_temperature: true,
            supports_max_completion_tokens: false,
            support_metadata: true,
            eof_is_done: false,
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

        let has_content = deltas.iter().any(|d| matches!(d, LlmStreamDelta::AppendContent { text } if text == "Hi"));
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

        let converted = convert_messages_to_refact(&messages);

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
        // LiteLLM maps Anthropic's input_tokens → prompt_tokens (non-cached portion)
        // total_tokens = input_tokens + output_tokens (does NOT include cache)
        // Cache fields passed through at top level
        let usage = serde_json::json!({
            "prompt_tokens": 500,
            "completion_tokens": 200,
            "cache_read_input_tokens": 1000,
            "cache_creation_input_tokens": 300,
            "total_tokens": 700
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        // prompt_tokens passed through (already non-cached)
        assert_eq!(parsed.prompt_tokens, 500);
        assert_eq!(parsed.completion_tokens, 200);
        assert_eq!(parsed.cache_creation_tokens, Some(300));
        assert_eq!(parsed.cache_read_tokens, Some(1000));
        // total recomputed: 500 + 200 + 300 + 1000 = 2000
        assert_eq!(parsed.total_tokens, 2000);
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
        let usage = serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 200,
            "cache_read_input_tokens": 5000,
            "cache_creation_input_tokens": 0,
            "total_tokens": 300
        });

        let parsed = parse_refact_usage(&usage).unwrap();

        assert_eq!(parsed.prompt_tokens, 100);
        assert_eq!(parsed.completion_tokens, 200);
        assert_eq!(parsed.cache_creation_tokens, None);
        assert_eq!(parsed.cache_read_tokens, Some(5000));
        // total recomputed: 100 + 200 + 0 + 5000 = 5300
        assert_eq!(parsed.total_tokens, 5300);
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
                    "thinking": "Let me reason...",
                    "signature": "sig_abc123"
                })]),
                tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
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

        let converted = convert_messages_to_refact(&messages);

        assert_eq!(converted.len(), 3);
        // Assistant message should have thinking_blocks
        let assistant = &converted[1];
        assert!(assistant.get("thinking_blocks").is_some(),
            "Assistant message should include thinking_blocks for LiteLLM");
        let blocks = assistant["thinking_blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "thinking");
        assert_eq!(blocks[0]["signature"], "sig_abc123");
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

        let converted = convert_messages_to_refact(&messages);

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

        let converted = convert_messages_to_refact(&messages);

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

        let converted = convert_messages_to_refact(&messages);

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

        let converted = convert_messages_to_refact(&messages);

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

        let converted = convert_messages_to_refact(&messages);

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

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        // No top-level cache_control field
        assert!(http.body.get("cache_control").is_none(),
            "cache_control should be in messages, not top-level");

        let messages = http.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4);

        // System message: content converted to array with cache_control
        let sys = &messages[0];
        let sys_content = sys["content"].as_array().unwrap();
        assert_eq!(sys_content.len(), 1);
        assert_eq!(sys_content[0]["type"], "text");
        assert_eq!(sys_content[0]["text"], "You are helpful");
        assert!(sys_content[0].get("cache_control").is_some(),
            "System message should have cache_control");

        // First non-system [0] (user "Hello"): should have cache_control
        let first_user = &messages[1];
        let first_content = first_user["content"].as_array().unwrap();
        assert!(first_content[0].get("cache_control").is_some(),
            "First non-system message should have cache_control");

        // Middle message (assistant): NO cache_control
        let assistant = &messages[2];
        // assistant content is still a string (no cache_control injected)
        assert!(assistant["content"].is_string(),
            "Middle message should not be converted to array");

        // Last non-system [-1] (user "How are you?"): should have cache_control
        let last_user = &messages[3];
        let last_content = last_user["content"].as_array().unwrap();
        assert!(last_content[0].get("cache_control").is_some(),
            "Last non-system message should have cache_control");
    }

    #[test]
    fn test_cache_control_ephemeral_single_message() {
        let adapter = RefactAdapter;
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]).with_cache_control(CacheControl::Ephemeral);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

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

        let http = adapter.build_http(&req, &default_settings()).unwrap();

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
}
