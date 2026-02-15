use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};

use crate::call_validation::ChatUsage;
use crate::llm::adapter::{AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError, extract_extra_fields, insert_extra_headers};
use crate::llm::canonical::{
    CanonicalToolChoice, LlmRequest, LlmStreamDelta, ResponseFormat,
};
use crate::llm::params::CacheControl;

const PROTECTED_FIELDS: &[&str] = &["model", "messages", "stream", "tools", "tool_choice", "stream_options"];

pub struct OpenAiChatAdapter;

impl LlmWireAdapter for OpenAiChatAdapter {
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

        let mut messages = convert_messages_to_openai(&req.messages);

        // Inject cache_control for OpenRouter -> Anthropic routing
        if matches!(req.cache_control, CacheControl::Ephemeral) {
            inject_cache_control(&mut messages);
        }

        let mut body = json!({
            "model": settings.model_name,
            "messages": messages,
            "stream": req.stream,
        });

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
            if let Some(effort) = req.reasoning.to_openai_effort() {
                body["reasoning_effort"] = json!(effort);
            }
            body.as_object_mut().map(|obj| obj.remove("temperature"));
        }

        if let Some(ref format) = req.response_format {
            body["response_format"] = response_format_to_openai(format);
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
            frequency_penalty = ?req.params.frequency_penalty,
            n = ?req.params.n,
            stop_sequences = ?req.params.stop.len(),
            tools_count = ?req.tools.as_ref().map(|t| t.len()),
            tool_choice = ?req.tool_choice,
            reasoning = ?req.reasoning,
            response_format = ?req.response_format.is_some(),
            cache_control = ?req.cache_control,
            has_meta = %req.meta.is_some(),
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

                    if let Some(reasoning) = delta.get("reasoning_content").and_then(|r| r.as_str())
                    {
                        if !reasoning.is_empty() {
                            deltas.push(LlmStreamDelta::AppendReasoning {
                                text: reasoning.to_string(),
                                block_index: None,
                            });
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

fn convert_messages_to_openai(messages: &[crate::call_validation::ChatMessage]) -> Vec<Value> {
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

            let mut obj = json!({
                "role": role,
            });

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
                                json!({
                                    "type": "text",
                                    "text": el.m_content
                                })
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

            Some(obj)
        })
        .collect()
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

/// Inject cache_control breakpoints for OpenRouter -> Anthropic routing.
/// Converts simple text messages to multipart format with cache_control on last block.
/// Strategy: cache system message + 4 strategically positioned messages (quarter, middle, last2, last).
fn inject_cache_control(messages: &mut [Value]) {
    let cc = json!({"type": "ephemeral", "ttl": "1h"});

    fn add_cache_to_message(msg: &mut Value, cc: &Value) {
        let Some(content) = msg.get_mut("content") else { return };
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

    if messages.is_empty() {
        return;
    }

    // Cache system message if present
    if let Some(first) = messages.first_mut() {
        if first.get("role").and_then(|r| r.as_str()) == Some("system") {
            add_cache_to_message(first, &cc);
        }
    }

    // Cache selected non-system messages
    let non_system_indices: Vec<usize> = messages.iter().enumerate()
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
    selected_positions.truncate(4);

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

    // Merge: prefer Anthropic fields (when routing via OpenRouter), fall back to OpenAI fields
    let cache_creation = anthropic_cache_creation;
    let cache_read = anthropic_cache_read.or(openai_cached);

    let raw_prompt = usage
        .get("prompt_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as usize;

    // For Anthropic models (via OpenRouter), prompt_tokens includes cached tokens.
    // Subtract cache_read to get non-cached portion.
    // For native OpenAI, cached_tokens is already a subset, so this is a no-op.
    let prompt_tokens = if let Some(cached) = cache_read {
        raw_prompt.saturating_sub(cached)
    } else {
        raw_prompt
    };

    let total_tokens = usage
        .get("total_tokens")
        .and_then(|t| t.as_u64())
        .map(|v| v as usize)
        .unwrap_or_else(|| {
            prompt_tokens 
                + completion_tokens 
                + cache_creation.unwrap_or(0)
                + cache_read.unwrap_or(0)
        });

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
            support_metadata: false,
            eof_is_done: false,
            supports_web_search: false,
        }
    }

    #[test]
    fn test_build_http_basic() {
        let adapter = OpenAiChatAdapter;
        let req = LlmRequest::new(
            "gpt-4".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
        );
        let settings = default_settings();

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(http.url, "https://api.openai.com/v1/chat/completions");
        assert!(http.headers.contains_key(AUTHORIZATION));
        assert_eq!(http.body["model"], "gpt-4");
        assert_eq!(http.body["messages"][0]["role"], "user");
        assert_eq!(http.body["messages"][0]["content"], "Hello");
    }

    #[test]
    fn test_build_http_with_tools() {
        let adapter = OpenAiChatAdapter;
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "parameters": {"type": "object"}
            }
        })];
        let req = LlmRequest::new("gpt-4".to_string(), vec![])
            .with_tools(tools, Some(CanonicalToolChoice::Auto));
        let settings = default_settings();

        let http = adapter.build_http(&req, &settings).unwrap();

        assert!(http.body.get("tools").is_some());
        assert_eq!(http.body["tool_choice"], "auto");
    }

    #[test]
    fn test_build_http_tools_skipped_when_unsupported() {
        let adapter = OpenAiChatAdapter;
        let tools = vec![json!({"type": "function"})];
        let req = LlmRequest::new("gpt-4".to_string(), vec![]).with_tools(tools, None);
        let mut settings = default_settings();
        settings.supports_tools = false;

        let http = adapter.build_http(&req, &settings).unwrap();

        assert!(http.body.get("tools").is_none());
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
    fn test_parse_stream_chunk_done() {
        let adapter = OpenAiChatAdapter;
        let deltas = adapter.parse_stream_chunk("[DONE]").unwrap();

        assert_eq!(deltas.len(), 1);
        assert!(matches!(deltas[0], LlmStreamDelta::Done));
    }

    #[test]
    fn test_parse_stream_chunk_malformed_skipped() {
        let adapter = OpenAiChatAdapter;
        let result = adapter.parse_stream_chunk("not json");

        assert!(matches!(result, Err(StreamParseError::MalformedChunk(_))));
    }

    #[test]
    fn test_convert_messages_filters_unknown_roles() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "hi".to_string()),
            ChatMessage::new("unknown_role".to_string(), "ignored".to_string()),
            ChatMessage::new("assistant".to_string(), "hello".to_string()),
        ];

        let converted = convert_messages_to_openai(&messages);

        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[1]["role"], "assistant");
    }

    #[test]
    fn test_build_http_with_response_format_json_schema() {
        let adapter = OpenAiChatAdapter;
        let mut req = LlmRequest::new("gpt-4".to_string(), vec![]);
        req.response_format = Some(ResponseFormat::JsonSchema {
            name: "person".to_string(),
            description: Some("A person object".to_string()),
            schema: json!({"type": "object", "properties": {"name": {"type": "string"}}}),
            strict: true,
        });
        let settings = default_settings();

        let http = adapter.build_http(&req, &settings).unwrap();

        let rf = &http.body["response_format"];
        assert_eq!(rf["type"], "json_schema");
        assert_eq!(rf["json_schema"]["name"], "person");
        assert_eq!(rf["json_schema"]["strict"], true);
    }

    #[test]
    fn test_build_http_with_response_format_json_object() {
        let adapter = OpenAiChatAdapter;
        let mut req = LlmRequest::new("gpt-4".to_string(), vec![]);
        req.response_format = Some(ResponseFormat::JsonObject);
        let settings = default_settings();

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(http.body["response_format"]["type"], "json_object");
    }

    #[test]
    fn test_build_http_uses_max_tokens_by_default() {
        let adapter = OpenAiChatAdapter;
        let mut req = LlmRequest::new("gpt-4".to_string(), vec![]);
        req.params.max_tokens = 500;
        let settings = default_settings();

        let http = adapter.build_http(&req, &settings).unwrap();

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

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(http.body["max_completion_tokens"], 500);
        assert!(http.body.get("max_tokens").is_none());
    }

    #[test]
    fn test_extra_body_protected_fields_ignored() {
        let adapter = OpenAiChatAdapter;
        let mut req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);
        req.extra_body = Some(serde_json::Map::from_iter([
            ("model".to_string(), json!("hacked-model")),
            ("messages".to_string(), json!([{"role": "user", "content": "hacked"}])),
            ("stream".to_string(), json!(false)),
            ("custom_field".to_string(), json!("allowed")),
        ]));

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        assert_eq!(http.body["model"], "gpt-4");
        assert_ne!(http.body["messages"], json!([{"role": "user", "content": "hacked"}]));
        assert_eq!(http.body["stream"], true);
        assert_eq!(http.body["custom_field"], "allowed");
    }

    #[test]
    fn test_user_agent_format() {
        let adapter = OpenAiChatAdapter;
        let req = LlmRequest::new("gpt-4".to_string(), vec![
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ]);

        let http = adapter.build_http(&req, &default_settings()).unwrap();

        // User-Agent should use space separator (not slash) for Refact cloud compatibility
        let ua = http.headers.get(USER_AGENT).unwrap().to_str().unwrap();
        assert!(ua.starts_with("refact-lsp "), "User-Agent should start with 'refact-lsp ' (space, not slash)");
        // Should match format: "refact-lsp X.Y.Z"
        let parts: Vec<&str> = ua.split(' ').collect();
        assert_eq!(parts.len(), 2, "User-Agent should have exactly 2 parts");
        assert_eq!(parts[0], "refact-lsp");
        // Version should be semver-like
        assert!(parts[1].contains('.'), "Version should contain dots");
    }

    #[test]
    fn test_server_executed_tools_filtered() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Search for something".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![
                    ChatToolCall {
                        id: "srvtoolu_123".to_string(),  // Server-executed
                        index: Some(0),
                        tool_type: "function".to_string(),
                        extra_content: None,
                        function: ChatToolFunction {
                            name: "web_search".to_string(),
                            arguments: r#"{"query":"test"}"#.to_string(),
                        },
                    },
                    ChatToolCall {
                        id: "call_456".to_string(),  // Regular tool call
                        index: Some(1),
                        tool_type: "function".to_string(),
                        extra_content: None,
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
                content: crate::call_validation::ChatContent::SimpleText("search results".to_string()),
                tool_call_id: "srvtoolu_123".to_string(),  // Server-executed result
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("file content".to_string()),
                tool_call_id: "call_456".to_string(),  // Regular tool result
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_openai(&messages);

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
    fn test_stream_citations_in_delta() {
        let adapter = OpenAiChatAdapter;
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
    fn test_reasoning_content_included_in_assistant() {
        let messages = vec![
            ChatMessage::new("user".to_string(), "Solve this".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("The answer".to_string()),
                reasoning_content: Some("Let me reason through this...".to_string()),
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_openai(&messages);

        assert_eq!(converted.len(), 2);
        assert_eq!(converted[1]["reasoning_content"], "Let me reason through this...");
        assert_eq!(converted[1]["content"], "The answer");
    }

    #[test]
    fn test_reasoning_content_not_included_when_absent() {
        let messages = vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Hi".to_string()),
                reasoning_content: None,
                ..Default::default()
            },
        ];

        let converted = convert_messages_to_openai(&messages);

        assert!(converted[0].get("reasoning_content").is_none());
    }

    #[test]
    fn test_parse_openai_usage_with_anthropic_cache() {
        // OpenRouter -> Anthropic: top-level cache fields
        let usage = json!({
            "prompt_tokens": 1000,
            "completion_tokens": 100,
            "total_tokens": 1100,
            "cache_creation_input_tokens": 200,
            "cache_read_input_tokens": 800
        });

        let result = parse_openai_usage(&usage).unwrap();
        
        // prompt_tokens should be adjusted: 1000 - 800 = 200
        assert_eq!(result.prompt_tokens, 200);
        assert_eq!(result.completion_tokens, 100);
        assert_eq!(result.cache_creation_tokens, Some(200));
        assert_eq!(result.cache_read_tokens, Some(800));
        assert_eq!(result.total_tokens, 1100);
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
    fn test_parse_openai_usage_no_cache() {
        let usage = json!({
            "prompt_tokens": 1000,
            "completion_tokens": 100,
            "total_tokens": 1100
        });

        let result = parse_openai_usage(&usage).unwrap();
        
        assert_eq!(result.prompt_tokens, 1000);
        assert_eq!(result.completion_tokens, 100);
        assert_eq!(result.cache_creation_tokens, None);
        assert_eq!(result.cache_read_tokens, None);
        assert_eq!(result.total_tokens, 1100);
    }

    #[test]
    fn test_parse_openai_usage_zero_cache_tokens_filtered() {
        // Zero cache tokens should be filtered out (None instead of Some(0))
        let usage = json!({
            "prompt_tokens": 1000,
            "completion_tokens": 100,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0
        });

        let result = parse_openai_usage(&usage).unwrap();
        
        assert_eq!(result.cache_creation_tokens, None);
        assert_eq!(result.cache_read_tokens, None);
    }

    #[test]
    fn test_inject_cache_control_simple_messages() {
        let mut messages = vec![
            json!({"role": "system", "content": "You are a helpful assistant"}),
            json!({"role": "user", "content": "Hello"}),
            json!({"role": "assistant", "content": "Hi there"}),
            json!({"role": "user", "content": "How are you?"}),
            json!({"role": "assistant", "content": "I'm doing well"}),
        ];

        inject_cache_control(&mut messages);

        // System message should have cache_control
        let system_content = messages[0]["content"].as_array().unwrap();
        assert_eq!(system_content.len(), 1);
        assert_eq!(system_content[0]["type"], "text");
        assert_eq!(system_content[0]["text"], "You are a helpful assistant");
        assert_eq!(system_content[0]["cache_control"]["type"], "ephemeral");
        assert_eq!(system_content[0]["cache_control"]["ttl"], "1h");

        // Non-system messages: 4 total (indices 1,2,3,4). Selected positions: [1, 2, 3]
        // Which correspond to message indices: [2, 3, 4] (assistant1, user2, assistant2)
        // user1 (message[1]) is at position 0 in non-system array, which is NOT selected
        assert_eq!(messages[1]["content"].as_str(), Some("Hello"), 
            "user1 should remain as simple string (not cached)");

        let assistant1_content = messages[2]["content"].as_array().unwrap();
        assert!(assistant1_content[0].get("cache_control").is_some(), 
            "assistant1 should be cached (position 1/quarter)");

        let user2_content = messages[3]["content"].as_array().unwrap();
        assert!(user2_content[0].get("cache_control").is_some(), 
            "user2 should be cached (position 2/middle)");

        let assistant2_content = messages[4]["content"].as_array().unwrap();
        assert!(assistant2_content[0].get("cache_control").is_some(), 
            "assistant2 should be cached (position 3/last)");
    }

    #[test]
    fn test_inject_cache_control_multipart_messages() {
        let mut messages = vec![
            json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "What's in this image?"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,..."}}
                ]
            }),
            json!({"role": "assistant", "content": "I see a cat"}),
        ];

        inject_cache_control(&mut messages);

        // First message (user) already multipart - cache_control on last block
        let user_content = messages[0]["content"].as_array().unwrap();
        assert_eq!(user_content.len(), 2);
        assert!(user_content[0].get("cache_control").is_none(), "First block shouldn't have cache_control");
        assert_eq!(user_content[1]["cache_control"]["type"], "ephemeral", "Last block should have cache_control");

        // Second message (assistant) simple text - converted to multipart
        let assistant_content = messages[1]["content"].as_array().unwrap();
        assert_eq!(assistant_content.len(), 1);
        assert_eq!(assistant_content[0]["cache_control"]["ttl"], "1h");
    }

    #[test]
    fn test_inject_cache_control_no_system_message() {
        let mut messages = vec![
            json!({"role": "user", "content": "Hello"}),
            json!({"role": "assistant", "content": "Hi"}),
        ];

        inject_cache_control(&mut messages);

        // Both messages should be cached (positions 0 and 1)
        assert!(messages[0]["content"].as_array().unwrap()[0].get("cache_control").is_some());
        assert!(messages[1]["content"].as_array().unwrap()[0].get("cache_control").is_some());
    }

    #[test]
    fn test_inject_cache_control_empty_messages() {
        let mut messages: Vec<Value> = vec![];
        inject_cache_control(&mut messages);
        assert_eq!(messages.len(), 0, "Should handle empty messages gracefully");
    }

    #[test]
    fn test_inject_cache_control_only_system() {
        let mut messages = vec![
            json!({"role": "system", "content": "Be helpful"}),
        ];

        inject_cache_control(&mut messages);

        // System message should be cached
        let content = messages[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn test_inject_cache_control_deduplication() {
        // With 2 non-system messages: quarter=0, middle=1, last2=0, last=1
        // After dedup: [0, 1]
        let mut messages = vec![
            json!({"role": "user", "content": "First"}),
            json!({"role": "assistant", "content": "Second"}),
        ];

        inject_cache_control(&mut messages);

        // Both should be cached
        assert!(messages[0]["content"].as_array().unwrap()[0].get("cache_control").is_some());
        assert!(messages[1]["content"].as_array().unwrap()[0].get("cache_control").is_some());
    }
}
