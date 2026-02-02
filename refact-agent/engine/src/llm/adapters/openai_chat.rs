use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};

use crate::call_validation::{ChatToolCall, ChatToolFunction, ChatUsage};
use crate::llm::adapter::{AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError};
use crate::llm::canonical::{CanonicalToolChoice, LlmRequest, LlmResponse, LlmStreamDelta};

pub struct OpenAiChatAdapter;

impl LlmWireAdapter for OpenAiChatAdapter {
    fn build_http(&self, req: &LlmRequest, settings: &AdapterSettings) -> Result<HttpParts, String> {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        if !settings.api_key.is_empty() {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", settings.api_key))
                    .map_err(|e| format!("invalid api_key for header: {e}"))?,
            );
        }
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&format!("refact-lsp/{}", env!("CARGO_PKG_VERSION")))
                .unwrap_or_else(|_| HeaderValue::from_static("refact-lsp")),
        );

        for (key, value) in &settings.extra_headers {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.insert(name, val);
            }
        }

        let messages = convert_messages_to_openai(&req.messages);

        let mut body = json!({
            "model": settings.model_name,
            "messages": messages,
            "stream": req.stream,
            "max_completion_tokens": req.params.max_tokens,
        });

        if req.stream {
            body["stream_options"] = json!({"include_usage": true});
        }

        if let Some(temp) = req.params.temperature {
            body["temperature"] = json!(temp);
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
                    body["parallel_tool_calls"] = json!(req.parallel_tool_calls);
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
        }

        if let Some(extra) = &req.extra_body {
            if let Some(obj) = body.as_object_mut() {
                for (k, v) in extra {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }

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
                error.get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error")
                    .to_string()
            ));
        }

        let mut deltas = Vec::new();

        if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
            for choice in choices {
                if let Some(delta) = choice.get("delta") {
                    if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                        if !content.is_empty() {
                            deltas.push(LlmStreamDelta::AppendContent { text: content.to_string() });
                        }
                    }

                    if let Some(reasoning) = delta.get("reasoning_content").and_then(|r| r.as_str()) {
                        if !reasoning.is_empty() {
                            deltas.push(LlmStreamDelta::AppendReasoning { text: reasoning.to_string() });
                        }
                    }

                    if let Some(tool_calls) = delta.get("tool_calls") {
                        if let Some(arr) = tool_calls.as_array() {
                            if !arr.is_empty() {
                                deltas.push(LlmStreamDelta::SetToolCalls { tool_calls: arr.clone() });
                            }
                        }
                    }
                }

                if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                    deltas.push(LlmStreamDelta::SetFinishReason { reason: reason.to_string() });
                }
            }
        }

        if let Some(usage) = json.get("usage") {
            if let Ok(u) = serde_json::from_value::<ChatUsage>(usage.clone()) {
                deltas.push(LlmStreamDelta::SetUsage { usage: u });
            }
        }

        Ok(deltas)
    }

    fn parse_response(&self, json: Value) -> Result<LlmResponse, String> {
        if let Some(error) = json.get("error") {
            return Err(error.get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error")
                .to_string());
        }

        let choices = json.get("choices")
            .and_then(|c| c.as_array())
            .ok_or("missing choices in response")?;

        let choice = choices.first().ok_or("empty choices array")?;
        let message = choice.get("message").ok_or("missing message in choice")?;

        let content = message.get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let reasoning_content = message.get("reasoning_content")
            .and_then(|r| r.as_str())
            .map(|s| s.to_string());

        let tool_calls = message.get("tool_calls")
            .and_then(|tc| tc.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|tc| parse_tool_call(tc))
                    .collect()
            })
            .unwrap_or_default();

        let finish_reason = choice.get("finish_reason")
            .and_then(|r| r.as_str())
            .map(|s| s.to_string());

        let usage = json.get("usage")
            .and_then(|u| serde_json::from_value(u.clone()).ok());

        Ok(LlmResponse {
            content,
            reasoning_content,
            tool_calls,
            finish_reason,
            usage,
            ..Default::default()
        })
    }
}

fn convert_messages_to_openai(messages: &[crate::call_validation::ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .filter_map(|msg| {
            let role = match msg.role.as_str() {
                "user" | "assistant" | "system" | "tool" => msg.role.clone(),
                "plain_text" | "cd_instruction" => "user".to_string(),
                _ => return None,
            };

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
                crate::call_validation::ChatContent::ContextFiles(files) => {
                    let text = files
                        .iter()
                        .map(|f| format!("{}:{}-{}\n```\n{}```", f.file_name, f.line1, f.line2, f.file_content))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    obj["content"] = json!(text);
                }
            }

            if let Some(tool_calls) = &msg.tool_calls {
                let tc: Vec<Value> = tool_calls
                    .iter()
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
                obj["tool_calls"] = json!(tc);
            }

            if !msg.tool_call_id.is_empty() {
                obj["tool_call_id"] = json!(msg.tool_call_id);
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

fn parse_tool_call(tc: &Value) -> Option<ChatToolCall> {
    Some(ChatToolCall {
        id: tc.get("id")?.as_str()?.to_string(),
        tool_type: tc.get("type").and_then(|t| t.as_str()).unwrap_or("function").to_string(),
        function: ChatToolFunction {
            name: tc.get("function")?.get("name")?.as_str()?.to_string(),
            arguments: tc.get("function")?.get("arguments")?.as_str()?.to_string(),
        },
        index: tc.get("index").and_then(|i| i.as_u64()).map(|i| i as usize),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatContent, ChatMessage};

    fn default_settings() -> AdapterSettings {
        AdapterSettings {
            api_key: "test-key".to_string(),
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            extra_headers: Default::default(),
            model_name: "gpt-4".to_string(),
            supports_tools: true,
            supports_reasoning: false,
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
        let req = LlmRequest::new("gpt-4".to_string(), vec![])
            .with_tools(tools, None);
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
            LlmStreamDelta::AppendContent { text } => assert_eq!(text, "Hello"),
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
    fn test_parse_response_basic() {
        let adapter = OpenAiChatAdapter;
        let json = json!({
            "choices": [{
                "message": {"content": "Hi there!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });

        let resp = adapter.parse_response(json).unwrap();

        assert_eq!(resp.content, "Hi there!");
        assert_eq!(resp.finish_reason, Some("stop".to_string()));
        assert!(resp.usage.is_some());
    }

    #[test]
    fn test_parse_response_with_tool_calls() {
        let adapter = OpenAiChatAdapter;
        let json = json!({
            "choices": [{
                "message": {
                    "content": "",
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\":\"NYC\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });

        let resp = adapter.parse_response(json).unwrap();

        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_123");
        assert_eq!(resp.tool_calls[0].function.name, "get_weather");
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
}
