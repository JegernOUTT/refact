use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::call_validation::ChatUsage;
use crate::llm::adapter::{AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError, extract_extra_fields, insert_extra_headers};
use crate::llm::canonical::{CanonicalToolChoice, LlmRequest, LlmStreamDelta};
use crate::llm::params::CacheControl;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_THINKING_BUDGET: usize = 8192;
const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";

const PROTECTED_FIELDS: &[&str] = &["model", "messages", "stream", "system", "tools", "tool_choice"];

pub struct AnthropicAdapter;

impl LlmWireAdapter for AnthropicAdapter {
    fn build_http(
        &self,
        req: &LlmRequest,
        settings: &AdapterSettings,
    ) -> Result<HttpParts, String> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&settings.api_key)
                .map_err(|e| format!("invalid api_key: {e}"))?,
        );
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );

        let is_effort_mode = settings.reasoning_type.as_deref() == Some("anthropic_effort");

        insert_extra_headers(&mut headers, &settings.extra_headers);

        let (system, messages) = convert_to_anthropic(&req.messages, req.cache_control);

        let mut body = json!({
            "model": settings.model_name,
            "messages": messages,
            "max_tokens": req.params.max_tokens,
            "stream": req.stream,
        });

        if let Some(sys) = system {
            body["system"] = sys;
        }

        if let Some(temp) = req.params.temperature {
            body["temperature"] = json!(temp);
        }

        if !req.params.stop.is_empty() {
            body["stop_sequences"] = json!(req.params.stop);
        }

        if settings.supports_tools {
            if let Some(tools) = &req.tools {
                if !tools.is_empty() {
                    let converted_tools = convert_tools_to_anthropic(tools);
                    body["tools"] = converted_tools;
                    if let Some(choice) = &req.tool_choice {
                        body["tool_choice"] = tool_choice_to_anthropic(choice);
                    }
                }
            }
        }

        if settings.supports_reasoning {
            if is_effort_mode {
                match &req.reasoning {
                    crate::llm::params::ReasoningIntent::BudgetTokens(n) => {
                        body["thinking"] = json!({"type": "enabled", "budget_tokens": *n});
                        let current_max = req.params.max_tokens;
                        if current_max <= *n {
                            let adjusted_max = *n + std::cmp::max(current_max, 1024);
                            body["max_tokens"] = json!(adjusted_max);
                            tracing::debug!(
                                "Adjusted max_tokens from {} to {} (thinking budget: {})",
                                current_max, adjusted_max, n
                            );
                        }
                    }
                    _ => {
                        if let Some(effort) = req.reasoning.to_anthropic_effort() {
                            body["thinking"] = json!({"type": "adaptive"});
                            body["output_config"] = json!({"effort": effort});
                        }
                    }
                }
            } else {
                if let Some(budget) = req.reasoning.to_anthropic_budget(DEFAULT_THINKING_BUDGET) {
                    body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
                    let current_max = req.params.max_tokens;
                    if current_max <= budget {
                        let adjusted_max = budget + std::cmp::max(current_max, 1024);
                        body["max_tokens"] = json!(adjusted_max);
                        tracing::debug!(
                            "Adjusted max_tokens from {} to {} (thinking budget: {})",
                            current_max, adjusted_max, budget
                        );
                    }
                }
            }
        }

        if body.get("thinking").and_then(|t| t.get("type")).and_then(|t| t.as_str()) == Some("enabled") {
            headers.insert(
                "anthropic-beta",
                HeaderValue::from_static(INTERLEAVED_THINKING_BETA),
            );
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

        tracing::info!(
            model = %settings.model_name,
            endpoint = %settings.endpoint,
            stream = %req.stream,
            max_tokens = %req.params.max_tokens,
            temperature = ?req.params.temperature,
            stop_sequences = ?req.params.stop.len(),
            tools_count = ?req.tools.as_ref().map(|t| t.len()),
            tool_choice = ?req.tool_choice,
            reasoning = ?req.reasoning,
            cache_control = ?req.cache_control,
            messages_count = %req.messages.len(),
            "anthropic adapter request"
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

        let json: Value = serde_json::from_str(trimmed)
            .map_err(|e| StreamParseError::MalformedChunk(format!("json: {e}")))?;

        if let Some(err) = json.get("error") {
            return Err(StreamParseError::FatalError(
                err.get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("error")
                    .to_string(),
            ));
        }

        let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let mut deltas = Vec::new();

        match event_type {
            "content_block_delta" => {
                if let Some(delta) = json.get("delta") {
                    let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match delta_type {
                        "text_delta" => {
                            if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                deltas.push(LlmStreamDelta::AppendContent {
                                    text: text.to_string(),
                                });
                            }
                        }
                        "thinking_delta" => {
                            if let Some(text) = delta.get("thinking").and_then(|t| t.as_str()) {
                                deltas.push(LlmStreamDelta::AppendReasoning {
                                    text: text.to_string(),
                                });
                            }
                        }
                        "signature_delta" => {
                            // Anthropic signature for thinking block verification
                            // Required for multi-turn tool calling conversations
                            if let Some(signature) = delta.get("signature").and_then(|s| s.as_str()) {
                                let block_index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                                deltas.push(LlmStreamDelta::SetThinkingBlocks {
                                    blocks: vec![json!({
                                        "index": block_index,
                                        "type": "thinking",
                                        "signature": signature
                                    })],
                                });
                            }
                        }
                        "input_json_delta" => {
                            if let Some(partial) =
                                delta.get("partial_json").and_then(|p| p.as_str())
                            {
                                let index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                                deltas.push(LlmStreamDelta::SetToolCalls {
                                    tool_calls: vec![
                                        json!({"index": index, "function": {"arguments": partial}}),
                                    ],
                                });
                            }
                        }
                        "citations_delta" => {
                            // Anthropic citations streaming - citation is in delta.citation
                            // Include content block index to preserve association
                            if let Some(citation) = delta.get("citation") {
                                let block_index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                                let mut enriched = citation.clone();
                                if let Some(obj) = enriched.as_object_mut() {
                                    obj.insert("_content_block_index".to_string(), json!(block_index));
                                }
                                deltas.push(LlmStreamDelta::AddCitation {
                                    citation: enriched,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            "message_start" => {
                if let Some(message) = json.get("message") {
                    if let Some(usage) = message.get("usage") {
                        if let Some(u) = parse_anthropic_usage(usage) {
                            deltas.push(LlmStreamDelta::SetUsage { usage: u });
                        }
                    }
                }
            }
            "message_delta" => {
                if let Some(delta) = json.get("delta") {
                    if let Some(reason) = delta.get("stop_reason").and_then(|r| r.as_str()) {
                        deltas.push(LlmStreamDelta::SetFinishReason {
                            reason: reason.to_string(),
                        });
                    }
                }
                if let Some(usage) = json.get("usage") {
                    if let Some(u) = parse_anthropic_usage(usage) {
                        deltas.push(LlmStreamDelta::SetUsage { usage: u });
                    }
                }
            }
            "message_stop" => {
                deltas.push(LlmStreamDelta::Done);
            }
            "content_block_start" => {
                if let Some(cb) = json.get("content_block") {
                    let block_type = cb.get("type").and_then(|t| t.as_str());
                    match block_type {
                        Some("tool_use") => {
                            if let (Some(id), Some(name)) = (
                                cb.get("id").and_then(|v| v.as_str()),
                                cb.get("name").and_then(|v| v.as_str()),
                            ) {
                                let index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                                deltas.push(LlmStreamDelta::SetToolCalls {
                                    tool_calls: vec![json!({
                                        "index": index,
                                        "id": id,
                                        "type": "function",
                                        "function": {"name": name}
                                    })],
                                });
                            }
                        }
                        Some("thinking") => {
                            // Anthropic thinking content is streamed incrementally via thinking_delta
                            // which emits AppendReasoning. We don't emit SetThinkingBlocks here
                            // because the content arrives via deltas, not as a complete block.
                            // The thinking content accumulates in ChoiceFinal.reasoning.
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                // Note: Anthropic's content_block_stop only contains {"type":"content_block_stop","index":N}
                // It does NOT include the content_block payload. Thinking content is already
                // streamed via thinking_delta -> AppendReasoning, so no action needed here.
            }
            _ => {}
        }

        // Extract Refact-specific extra fields on ALL events consistently
        let extra = extract_extra_fields(&json);
        if !extra.is_empty() {
            deltas.push(LlmStreamDelta::MergeExtra { extra });
        }

        Ok(deltas)
    }
}

fn convert_to_anthropic(
    messages: &[crate::call_validation::ChatMessage],
    cache: CacheControl,
) -> (Option<Value>, Vec<Value>) {
    let mut system_text = None;
    let mut result: Vec<Value> = Vec::new();
    let mut pending_tool_results: Vec<Value> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                system_text = Some(msg.content.content_text_only());
            }
            "user" | "assistant" => {
                let mut content = Vec::new();
                // Merge pending tool_results into user message to avoid consecutive user blocks
                if msg.role == "user" && !pending_tool_results.is_empty() {
                    content.extend(pending_tool_results.drain(..));
                } else {
                    flush_tool_results(&mut result, &mut pending_tool_results);
                }
                if msg.role == "assistant" {
                    if let Some(blocks) = &msg.thinking_blocks {
                        for block in blocks {
                            if let Some(block_type) = block.get("type").and_then(|t| t.as_str()) {
                                match block_type {
                                    "thinking" => {
                                        let mut tb = json!({"type": "thinking"});
                                        if let Some(thinking) = block.get("thinking") {
                                            tb["thinking"] = thinking.clone();
                                        }
                                        if let Some(sig) = block.get("signature") {
                                            tb["signature"] = sig.clone();
                                        }
                                        content.push(tb);
                                    }
                                    "redacted_thinking" => {
                                        let mut rb = json!({"type": "redacted_thinking"});
                                        if let Some(data) = block.get("data") {
                                            rb["data"] = data.clone();
                                        }
                                        content.push(rb);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                content.extend(msg_content_to_anthropic(&msg.content));
                if msg.role == "assistant" {
                    if let Some(tcs) = &msg.tool_calls {
                        let tool_blocks: Vec<Value> = tcs.iter()
                            .filter(|tc| !tc.id.starts_with("srvtoolu_"))  // Filter server-executed tools
                            .map(|tc| {
                                let input = match serde_json::from_str::<Value>(&tc.function.arguments) {
                                    Ok(v) => v,
                                    Err(e) => {
                                        tracing::warn!(
                                            "Invalid JSON in tool call arguments for '{}': {} - using empty object",
                                            tc.function.name, e
                                        );
                                        json!({})
                                    }
                                };
                                json!({
                                    "type": "tool_use",
                                    "id": tc.id,
                                    "name": tc.function.name,
                                    "input": input
                                })
                            }).collect();
                        content.extend(tool_blocks);
                    }
                }
                let content = sanitize_anthropic_content(content);
                result.push(json!({"role": msg.role, "content": content}));
            }
            "tool" | "diff" => {
                if !msg.tool_call_id.starts_with("srvtoolu_") {  // Filter server-executed tool results
                    let tool_text = msg.content.content_text_only();
                    let tool_text = if tool_text.is_empty() { "(empty)".to_string() } else { tool_text };
                    pending_tool_results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": msg.tool_call_id,
                        "content": tool_text
                    }));
                }
            }
            _ => {}
        }
    }

    flush_tool_results(&mut result, &mut pending_tool_results);

    let system = system_text.map(|text| match cache {
        CacheControl::Ephemeral => json!([{
            "type": "text",
            "text": text,
            "cache_control": {"type": "ephemeral", "ttl": "1h"}
        }]),
        CacheControl::Off => json!(text),
    });

    // Apply cache breakpoints for prefix-based caching.
    // Anthropic caches are prefix-based: the hash at each breakpoint covers ALL previous blocks.
    // Up to 4 breakpoints allowed; combined with the system breakpoint = 3 message breakpoints.
    //
    // CRITICAL: breakpoints must be STABLE across consecutive calls. If a message has
    // cache_control in call N but not in call N+1, the prefix hash changes and cache misses.
    //
    // In agentic flows the pattern is [user, assistant, user(tool_results), assistant, ...]
    // with only one real user message at [0]. A "moving" middle breakpoint (e.g. last assistant
    // before last user) shifts forward every call, removing cache_control from the previous
    // position and invalidating the entire prefix after [0].
    //
    // Strategy: only two message breakpoints (+ system = 3 total):
    //   1. [0] — first message. Stable across all calls.
    //   2. [-1] — last message. Changes each call but caches the full prefix.
    if cache == CacheControl::Ephemeral && !result.is_empty() {
        let len = result.len();
        let mut breakpoint_indices = vec![0, len - 1];
        breakpoint_indices.dedup();

        for &idx in &breakpoint_indices {
            add_cache_control_to_last_block(&mut result[idx]);
        }
    }

    (system, result)
}

/// Adds `cache_control` to the last content block of an Anthropic message.
/// Each message has a "content" array of blocks; the breakpoint goes on the last one.
fn add_cache_control_to_last_block(message: &mut Value) {
    let cc = json!({"type": "ephemeral", "ttl": "1h"});
    if let Some(content) = message.get_mut("content") {
        if let Some(arr) = content.as_array_mut() {
            if let Some(last_block) = arr.last_mut() {
                if let Some(obj) = last_block.as_object_mut() {
                    obj.insert("cache_control".to_string(), cc);
                }
            }
        }
    }
}

fn flush_tool_results(result: &mut Vec<Value>, pending: &mut Vec<Value>) {
    if pending.is_empty() {
        return;
    }
    result.push(json!({
        "role": "user",
        "content": pending.drain(..).collect::<Vec<_>>()
    }));
}

/// Anthropic rejects `{"type":"text","text":""}` content blocks with 400 Bad Request.
/// This removes empty text blocks, keeping non-text blocks (images, etc.) intact.
/// If nothing remains, inserts a placeholder so the message stays valid.
fn sanitize_anthropic_content(mut blocks: Vec<Value>) -> Vec<Value> {
    blocks.retain(|block| {
        let is_empty_text = block.get("type").and_then(|t| t.as_str()) == Some("text")
            && block.get("text").and_then(|t| t.as_str()).map_or(false, |s| s.is_empty());
        !is_empty_text
    });
    if blocks.is_empty() {
        blocks.push(json!({"type": "text", "text": "(empty)"}));
    }
    blocks
}

fn msg_content_to_anthropic(content: &crate::call_validation::ChatContent) -> Vec<Value> {
    match content {
        crate::call_validation::ChatContent::SimpleText(text) => vec![json!({"type": "text", "text": text})],
        crate::call_validation::ChatContent::Multimodal(elements) => {
            elements.iter().map(|el| {
                if el.is_image() {
                    json!({"type": "image", "source": {"type": "base64", "media_type": el.m_type, "data": el.m_content}})
                } else {
                    json!({"type": "text", "text": el.m_content})
                }
            }).collect()
        }
        crate::call_validation::ChatContent::ContextFiles(_) => {
            vec![json!({"type": "text", "text": content.content_text_only()})]
        }
    }
}

fn convert_tools_to_anthropic(tools: &[Value]) -> Value {
    let converted: Vec<Value> = tools.iter().filter_map(|t| {
        let f = t.get("function")?;
        Some(json!({"name": f.get("name")?, "description": f.get("description").unwrap_or(&json!("")), "input_schema": f.get("parameters").unwrap_or(&json!({}))}))
    }).collect();
    json!(converted)
}

fn tool_choice_to_anthropic(choice: &CanonicalToolChoice) -> Value {
    match choice {
        CanonicalToolChoice::Auto => json!({"type": "auto"}),
        CanonicalToolChoice::None => json!({"type": "none"}),
        CanonicalToolChoice::Required => json!({"type": "any"}),
        CanonicalToolChoice::Function { name } => json!({"type": "tool", "name": name}),
    }
}

fn parse_anthropic_usage(usage: &Value) -> Option<ChatUsage> {
    let prompt_tokens = usage
        .get("input_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as usize;
    let completion_tokens = usage
        .get("output_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as usize;
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(|t| t.as_u64())
        .map(|v| v as usize);
    let cache_read = usage
        .get("cache_read_input_tokens")
        .and_then(|t| t.as_u64())
        .map(|v| v as usize);
    let total_tokens = prompt_tokens
        + completion_tokens
        + cache_creation.unwrap_or(0)
        + cache_read.unwrap_or(0);
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
    use crate::call_validation::ChatMessage;

    fn settings() -> AdapterSettings {
        AdapterSettings {
            api_key: "sk-ant-test".to_string(),
            endpoint: "https://api.anthropic.com/v1/messages".to_string(),
            extra_headers: Default::default(),
            model_name: "claude-3-sonnet".to_string(),
            supports_tools: true,
            supports_reasoning: true,
            reasoning_type: Some("anthropic_budget".to_string()),
            supports_temperature: true,
            supports_max_completion_tokens: false,
            support_metadata: false,
            eof_is_done: false,
        }
    }

    #[test]
    fn test_build_http_headers() {
        let adapter = AnthropicAdapter;
        let req = LlmRequest::new("claude".to_string(), vec![]);
        let http = adapter.build_http(&req, &settings()).unwrap();
        assert!(http.headers.get("x-api-key").is_some());
        assert!(http.headers.get("anthropic-version").is_some());
    }

    #[test]
    fn test_interleaved_thinking_beta_header() {
        use crate::llm::params::ReasoningIntent;

        let adapter = AnthropicAdapter;

        let req_with_reasoning = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        ).with_reasoning(ReasoningIntent::High);

        let http = adapter.build_http(&req_with_reasoning, &settings()).unwrap();
        let beta = http.headers.get("anthropic-beta").map(|v| v.to_str().unwrap().to_string());
        assert_eq!(beta, Some(INTERLEAVED_THINKING_BETA.to_string()));
    }

    #[test]
    fn test_no_beta_header_without_reasoning() {
        let adapter = AnthropicAdapter;

        let req_no_reasoning = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        );

        let http = adapter.build_http(&req_no_reasoning, &settings()).unwrap();
        assert!(http.headers.get("anthropic-beta").is_none());
    }

    #[test]
    fn test_no_beta_header_when_reasoning_not_supported() {
        use crate::llm::params::ReasoningIntent;

        let adapter = AnthropicAdapter;
        let mut no_reasoning_settings = settings();
        no_reasoning_settings.supports_reasoning = false;

        let req = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        ).with_reasoning(ReasoningIntent::High);

        let http = adapter.build_http(&req, &no_reasoning_settings).unwrap();
        assert!(http.headers.get("anthropic-beta").is_none());
    }

    #[test]
    fn test_system_as_top_level() {
        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ];
        let (system, msgs) = convert_to_anthropic(&messages, CacheControl::Off);
        assert_eq!(system, Some(json!("Be helpful")));
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_system_with_cache_control() {
        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ];
        let (system, msgs) = convert_to_anthropic(&messages, CacheControl::Ephemeral);
        let expected =
            json!([{"type": "text", "text": "Be helpful", "cache_control": {"type": "ephemeral", "ttl": "1h"}}]);
        assert_eq!(system, Some(expected));
        assert_eq!(msgs.len(), 1);
        // Single message should get cache breakpoint at [0]
        assert!(msgs[0]["content"][0].get("cache_control").is_some());
    }

    #[test]
    fn test_parse_stream_text_delta() {
        let adapter = AnthropicAdapter;
        let chunk =
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello"}}"#;
        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        assert!(matches!(&deltas[0], LlmStreamDelta::AppendContent { text } if text == "Hello"));
    }

    #[test]
    fn test_parse_stream_tool_use_start() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_123","name":"get_weather"}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::SetToolCalls { tool_calls } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0]["id"], "toolu_123");
                assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
            }
            _ => panic!("expected SetToolCalls"),
        }
    }

    #[test]
    fn test_parse_stream_tool_use_input_delta() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"loc"}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::SetToolCalls { tool_calls } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0]["index"], 0);
                assert_eq!(tool_calls[0]["function"]["arguments"], "{\"loc");
            }
            _ => panic!("expected SetToolCalls"),
        }
    }

    #[test]
    fn test_parse_stream_content_block_stop() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_stop","index":0}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        assert!(deltas.is_empty());
    }

    #[test]
    fn test_parse_stream_message_stop() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"message_stop"}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        assert!(matches!(&deltas[0], LlmStreamDelta::Done));
    }

    #[test]
    fn test_parse_stream_thinking_delta() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think..."}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::AppendReasoning { text } => {
                assert_eq!(text, "Let me think...");
            }
            _ => panic!("expected AppendReasoning"),
        }
    }

    #[test]
    fn test_parse_stream_thinking_block_start() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        // Thinking blocks are NOT emitted on content_block_start - content arrives via thinking_delta
        // which emits AppendReasoning. This is intentional to avoid empty placeholder blocks.
        assert!(!deltas.iter().any(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. })));
    }

    #[test]
    fn test_extra_body_protected_fields_ignored() {
        let adapter = AnthropicAdapter;
        let mut req = LlmRequest::new("claude".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);
        req.extra_body = Some(serde_json::Map::from_iter([
            ("model".to_string(), json!("hacked-model")),
            ("messages".to_string(), json!([{"role": "user", "content": "hacked"}])),
            ("custom_field".to_string(), json!("allowed")),
        ]));

        let http = adapter.build_http(&req, &settings()).unwrap();

        assert_eq!(http.body["model"], "claude-3-sonnet");
        assert_ne!(http.body["messages"], json!([{"role": "user", "content": "hacked"}]));
        assert_eq!(http.body["custom_field"], "allowed");
    }

    #[test]
    fn test_multi_tool_results_grouped() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Do two things".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![
                    ChatToolCall {
                        id: "call_1".to_string(),
                        tool_type: "function".to_string(),
                        function: ChatToolFunction {
                            name: "tool_a".to_string(),
                            arguments: "{}".to_string(),
                        },
                        index: None,
                    },
                    ChatToolCall {
                        id: "call_2".to_string(),
                        tool_type: "function".to_string(),
                        function: ChatToolFunction {
                            name: "tool_b".to_string(),
                            arguments: "{}".to_string(),
                        },
                        index: None,
                    },
                ]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Result A".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Result B".to_string()),
                tool_call_id: "call_2".to_string(),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, CacheControl::Off);

        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "user");

        let tool_results = msgs[2]["content"].as_array().unwrap();
        assert_eq!(tool_results.len(), 2);
        assert_eq!(tool_results[0]["type"], "tool_result");
        assert_eq!(tool_results[0]["tool_use_id"], "call_1");
        assert_eq!(tool_results[1]["type"], "tool_result");
        assert_eq!(tool_results[1]["tool_use_id"], "call_2");
    }

    #[test]
    fn test_tool_result_merged_into_following_user() {
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

        // Simulates post-linearization input: tool reply followed by user message
        // (linearizer folds cf into tool; real user message stays separate)
        let messages = vec![
            ChatMessage::new("user".to_string(), "start".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("calling tool".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    function: ChatToolFunction {
                        name: "search".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("tool output".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "now fix it".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, CacheControl::Off);

        // Should be 3 messages: user, assistant, user(tool_result + text)
        // NOT 4: user, assistant, user(tool_result), user(text)
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "user");

        let content = msgs[2]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "call_1");
        assert_eq!(content[0]["content"], "tool output");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "now fix it");
    }

    #[test]
    fn test_diff_role_as_tool_result() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Edit file".to_string()),
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
                content: crate::call_validation::ChatContent::SimpleText("@@ -1 +1 @@".to_string()),
                tool_call_id: "call_edit".to_string(),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, CacheControl::Off);

        assert_eq!(msgs.len(), 3);
        let tool_result = &msgs[2]["content"][0];
        assert_eq!(tool_result["type"], "tool_result");
        assert_eq!(tool_result["tool_use_id"], "call_edit");
    }

    #[test]
    fn test_stream_tool_use_missing_fields_skipped() {
        let adapter = AnthropicAdapter;
        let chunk_missing_id = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","name":"get_weather"}}"#;
        let chunk_missing_name = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_123"}}"#;

        let deltas1 = adapter.parse_stream_chunk(chunk_missing_id).unwrap();
        let deltas2 = adapter.parse_stream_chunk(chunk_missing_name).unwrap();

        let has_tool_calls1 = deltas1.iter().any(|d| matches!(d, LlmStreamDelta::SetToolCalls { .. }));
        let has_tool_calls2 = deltas2.iter().any(|d| matches!(d, LlmStreamDelta::SetToolCalls { .. }));

        assert!(!has_tool_calls1);
        assert!(!has_tool_calls2);
    }

    #[test]
    fn test_stream_citations_delta() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_delta","index":2,"delta":{"type":"citations_delta","citation":{"type":"char_location","cited_text":"Some text","document_index":0,"document_title":"doc.txt","start_char_index":0,"end_char_index":10}}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        let has_citation = deltas.iter().any(|d| matches!(d, LlmStreamDelta::AddCitation { .. }));
        assert!(has_citation);

        // Verify citation content and block index preservation
        if let Some(LlmStreamDelta::AddCitation { citation }) = deltas.iter().find(|d| matches!(d, LlmStreamDelta::AddCitation { .. })) {
            assert_eq!(citation.get("type").and_then(|v| v.as_str()), Some("char_location"));
            assert_eq!(citation.get("cited_text").and_then(|v| v.as_str()), Some("Some text"));
            // Verify block index is preserved for multi-block association
            assert_eq!(citation.get("_content_block_index").and_then(|v| v.as_u64()), Some(2));
        }
    }

    #[test]
    fn test_thinking_block_start_no_empty_blocks() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        // Should NOT emit SetThinkingBlocks - thinking content comes via thinking_delta -> AppendReasoning
        let has_thinking_blocks = deltas.iter().any(|d| matches!(d, LlmStreamDelta::SetThinkingBlocks { .. }));
        assert!(!has_thinking_blocks);
    }

    #[test]
    fn test_thinking_max_tokens_adjustment() {
        use crate::llm::adapter::LlmWireAdapter;
        use crate::llm::params::ReasoningIntent;

        let adapter = AnthropicAdapter;

        // Test with max_tokens < thinking budget (should be adjusted)
        let mut req_low_max = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        );
        req_low_max.params.max_tokens = 4096;  // Less than DEFAULT_THINKING_BUDGET (10000)
        req_low_max.reasoning = ReasoningIntent::High;  // Will use DEFAULT_THINKING_BUDGET
        req_low_max.stream = true;

        let http = adapter.build_http(&req_low_max, &settings()).unwrap();
        // Should be adjusted: 10000 + max(4096, 1024) = 14096
        assert_eq!(http.body["max_tokens"], 14096);
        assert_eq!(http.body["thinking"]["budget_tokens"], DEFAULT_THINKING_BUDGET);

        // Test with max_tokens > thinking budget (should NOT be adjusted)
        let mut req_high_max = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        );
        req_high_max.params.max_tokens = 20000;  // More than DEFAULT_THINKING_BUDGET
        req_high_max.reasoning = ReasoningIntent::High;
        req_high_max.stream = true;

        let http2 = adapter.build_http(&req_high_max, &settings()).unwrap();
        // Should remain unchanged
        assert_eq!(http2.body["max_tokens"], 20000);

        // Test with reasoning off (no adjustment needed)
        let mut req_no_thinking = LlmRequest::new(
            "claude".to_string(),
            vec![ChatMessage::new("user".to_string(), "test".to_string())],
        );
        req_no_thinking.params.max_tokens = 4096;
        req_no_thinking.reasoning = ReasoningIntent::Off;
        req_no_thinking.stream = true;

        let http3 = adapter.build_http(&req_no_thinking, &settings()).unwrap();
        assert_eq!(http3.body["max_tokens"], 4096);
        assert!(http3.body.get("thinking").is_none());
    }

    #[test]
    fn test_cache_breakpoints_on_messages() {
        // After linearization: user, assistant+tool_use, tool_result, user
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "What does this do?".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("Let me check".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    function: ChatToolFunction {
                        name: "tool_a".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("Result".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "Thanks, now explain".to_string()),
        ];

        let (system, msgs) = convert_to_anthropic(&messages, CacheControl::Ephemeral);

        // System should have cache_control with 1h TTL
        let sys = system.unwrap();
        assert_eq!(sys[0]["cache_control"]["ttl"], "1h");

        // Messages: [0]=user, [1]=assistant+tool_use, [2]=user(tool_result+text)
        // Tool result is merged into the following user message (no consecutive user blocks)
        assert_eq!(msgs.len(), 3);

        // [0]=user always gets breakpoint for stable prefix caching
        assert!(msgs[0]["content"].as_array().unwrap().last().unwrap().get("cache_control").is_some(),
            "First user message should have cache breakpoint for stable prefix");

        // [1]=assistant has NO breakpoint (middle breakpoints are unstable in agentic flows)
        assert!(msgs[1]["content"].as_array().unwrap().last().unwrap().get("cache_control").is_none(),
            "Assistant should NOT have cache breakpoint (only [0] and [-1])");

        // [2]=user(tool_result+text) is [-1] → breakpoint
        assert!(msgs[2]["content"].as_array().unwrap().last().unwrap().get("cache_control").is_some(),
            "Last message should have cache breakpoint");

        // Verify the merged user message contains both tool_result and text
        let last_content = msgs[2]["content"].as_array().unwrap();
        let has_tool_result = last_content.iter().any(|b| b["type"] == "tool_result");
        let has_text = last_content.iter().any(|b| b["type"] == "text");
        assert!(has_tool_result, "Merged user message should contain tool_result");
        assert!(has_text, "Merged user message should contain user text");
    }

    #[test]
    fn test_cache_breakpoints_single_message() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, CacheControl::Ephemeral);

        assert_eq!(msgs.len(), 1);
        // Single message gets breakpoint at [-1]
        assert!(msgs[0]["content"][0].get("cache_control").is_some());
        assert_eq!(msgs[0]["content"][0]["cache_control"]["ttl"], "1h");
    }

    #[test]
    fn test_cache_breakpoints_two_messages() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage::new("assistant".to_string(), "Hi there".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, CacheControl::Ephemeral);

        assert_eq!(msgs.len(), 2);
        // Two messages: [0] (always) and [-1] get breakpoints
        assert!(msgs[0]["content"][0].get("cache_control").is_some());
        assert!(msgs[1]["content"][0].get("cache_control").is_some());
    }

    #[test]
    fn test_no_cache_breakpoints_when_off() {
        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage::new("assistant".to_string(), "Hi".to_string()),
            ChatMessage::new("user".to_string(), "Thanks".to_string()),
        ];

        let (system, msgs) = convert_to_anthropic(&messages, CacheControl::Off);

        // System should be plain text, no cache_control
        assert_eq!(system, Some(json!("Be helpful")));

        // No messages should have cache_control
        for msg in &msgs {
            if let Some(content) = msg["content"].as_array() {
                for block in content {
                    assert!(block.get("cache_control").is_none(),
                        "No cache breakpoints expected when CacheControl::Off");
                }
            }
        }
    }

    #[test]
    fn test_cache_breakpoint_on_tool_use_last_block() {
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Do something".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    function: ChatToolFunction {
                        name: "get_weather".to_string(),
                        arguments: r#"{"city":"London"}"#.to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("Sunny, 20C".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, CacheControl::Ephemeral);

        // [0]=user, [1]=assistant(text+tool_use), [2]=tool_result(user)
        assert_eq!(msgs.len(), 3);

        // [0]=user gets breakpoint (always first message)
        assert!(msgs[0]["content"].as_array().unwrap().last().unwrap().get("cache_control").is_some(),
            "First user message should have cache breakpoint");

        // [1]=assistant has NO breakpoint (only [0] and [-1] get breakpoints)
        let assistant_content = msgs[1]["content"].as_array().unwrap();
        let last_block = assistant_content.last().unwrap();
        assert_eq!(last_block["type"], "tool_use");
        assert!(last_block.get("cache_control").is_none(),
            "Assistant should NOT have cache breakpoint (middle breakpoints are unstable)");
    }

    #[test]
    fn test_thinking_blocks_included_in_assistant() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Solve this".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("The answer is 42".to_string()),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "thinking": "Let me work through this...",
                    "signature": "abc123signature"
                })]),
                ..Default::default()
            },
            ChatMessage::new("user".to_string(), "Explain more".to_string()),
        ];

        let (_, msgs) = convert_to_anthropic(&messages, CacheControl::Off);

        assert_eq!(msgs.len(), 3);
        let assistant_content = msgs[1]["content"].as_array().unwrap();
        // Thinking block should come first, then text
        assert_eq!(assistant_content[0]["type"], "thinking");
        assert_eq!(assistant_content[0]["thinking"], "Let me work through this...");
        assert_eq!(assistant_content[0]["signature"], "abc123signature");
        assert_eq!(assistant_content[1]["type"], "text");
        assert_eq!(assistant_content[1]["text"], "The answer is 42");
    }

    #[test]
    fn test_thinking_blocks_before_tool_use() {
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Search for X".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "thinking": "I should search for X",
                    "signature": "sig_search"
                })]),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    function: ChatToolFunction {
                        name: "search".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("Found results".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, CacheControl::Off);

        // assistant content: [thinking, (empty text removed), tool_use]
        let assistant_content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(assistant_content[0]["type"], "thinking");
        assert_eq!(assistant_content[0]["signature"], "sig_search");
        // Last block should be tool_use (empty text sanitized away)
        let last = assistant_content.last().unwrap();
        assert_eq!(last["type"], "tool_use");
    }

    #[test]
    fn test_redacted_thinking_blocks() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Test".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Response".to_string()),
                thinking_blocks: Some(vec![
                    json!({
                        "type": "thinking",
                        "thinking": "Normal thinking",
                        "signature": "sig1"
                    }),
                    json!({
                        "type": "redacted_thinking",
                        "data": "encrypted_data_here"
                    }),
                ]),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, CacheControl::Off);

        let content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "thinking");
        assert_eq!(content[0]["thinking"], "Normal thinking");
        assert_eq!(content[1]["type"], "redacted_thinking");
        assert_eq!(content[1]["data"], "encrypted_data_here");
        assert_eq!(content[2]["type"], "text");
    }

    #[test]
    fn test_no_thinking_blocks_when_none() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Hello".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Hi there".to_string()),
                thinking_blocks: None,
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, CacheControl::Off);

        let content = msgs[1]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }

    #[test]
    fn test_thinking_blocks_cache_breakpoint_on_last_block() {
        use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

        // Simulate call 2: user + assistant(thinking+tool_use) + tool_result
        let messages = vec![
            ChatMessage::new("user".to_string(), "Do something".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                thinking_blocks: Some(vec![json!({
                    "type": "thinking",
                    "thinking": "Let me think...",
                    "signature": "sig_abc"
                })]),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    tool_type: "function".to_string(),
                    function: ChatToolFunction {
                        name: "tool_a".to_string(),
                        arguments: "{}".to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("Result".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
        ];

        let (_, msgs) = convert_to_anthropic(&messages, CacheControl::Ephemeral);

        // Assistant content: [thinking, tool_use] (empty text sanitized)
        let assistant_content = msgs[1]["content"].as_array().unwrap();
        // No breakpoint on assistant (only [0] and [-1] get breakpoints)
        let last_block = assistant_content.last().unwrap();
        assert_eq!(last_block["type"], "tool_use");
        assert!(last_block.get("cache_control").is_none(),
            "Assistant should NOT have cache breakpoint (only [0] and [-1])");
        assert!(assistant_content[0].get("cache_control").is_none(),
            "Thinking block should not have cache_control");
    }
}
