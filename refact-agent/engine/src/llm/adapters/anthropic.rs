use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::call_validation::{ChatToolCall, ChatToolFunction, ChatUsage};
use crate::llm::adapter::{AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError};
use crate::llm::canonical::{CanonicalToolChoice, LlmRequest, LlmResponse, LlmStreamDelta};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_THINKING_BUDGET: usize = 10000;

pub struct AnthropicAdapter;

impl LlmWireAdapter for AnthropicAdapter {
    fn build_http(&self, req: &LlmRequest, settings: &AdapterSettings) -> Result<HttpParts, String> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&settings.api_key)
                .map_err(|e| format!("invalid api_key: {e}"))?,
        );
        headers.insert("anthropic-version", HeaderValue::from_static(ANTHROPIC_VERSION));

        for (key, value) in &settings.extra_headers {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.insert(name, val);
            }
        }

        let (system, messages) = convert_to_anthropic(&req.messages);

        let mut body = json!({
            "model": settings.model_name,
            "messages": messages,
            "max_tokens": req.params.max_tokens,
            "stream": req.stream,
        });

        if let Some(sys) = system {
            body["system"] = json!(sys);
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
                    body["tools"] = convert_tools_to_anthropic(tools);
                    if let Some(choice) = &req.tool_choice {
                        body["tool_choice"] = tool_choice_to_anthropic(choice);
                    }
                }
            }
        }

        if settings.supports_reasoning {
            if let Some(budget) = req.reasoning.to_anthropic_budget(DEFAULT_THINKING_BUDGET) {
                body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
            }
        }

        if let Some(extra) = &req.extra_body {
            if let Some(obj) = body.as_object_mut() {
                for (k, v) in extra {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }

        Ok(HttpParts { url: settings.endpoint.clone(), headers, body })
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
                err.get("message").and_then(|m| m.as_str()).unwrap_or("error").to_string()
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
                                deltas.push(LlmStreamDelta::AppendContent { text: text.to_string() });
                            }
                        }
                        "thinking_delta" => {
                            if let Some(text) = delta.get("thinking").and_then(|t| t.as_str()) {
                                deltas.push(LlmStreamDelta::AppendReasoning { text: text.to_string() });
                            }
                        }
                        "input_json_delta" => {
                            if let Some(partial) = delta.get("partial_json").and_then(|p| p.as_str()) {
                                let index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                                deltas.push(LlmStreamDelta::SetToolCalls {
                                    tool_calls: vec![json!({"index": index, "function": {"arguments": partial}})]
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            "message_delta" => {
                if let Some(delta) = json.get("delta") {
                    if let Some(reason) = delta.get("stop_reason").and_then(|r| r.as_str()) {
                        deltas.push(LlmStreamDelta::SetFinishReason { reason: reason.to_string() });
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
                    if cb.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        let index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                        deltas.push(LlmStreamDelta::SetToolCalls {
                            tool_calls: vec![json!({
                                "index": index,
                                "id": cb.get("id"),
                                "type": "function",
                                "function": {"name": cb.get("name")}
                            })]
                        });
                    }
                }
            }
            _ => {}
        }

        Ok(deltas)
    }

    fn parse_response(&self, json: Value) -> Result<LlmResponse, String> {
        if let Some(err) = json.get("error") {
            return Err(err.get("message").and_then(|m| m.as_str()).unwrap_or("error").to_string());
        }

        let content_blocks = json.get("content").and_then(|c| c.as_array());
        let mut content = String::new();
        let mut reasoning = None;
        let mut tool_calls = Vec::new();
        let mut thinking_blocks = Vec::new();

        if let Some(blocks) = content_blocks {
            for block in blocks {
                match block.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            content.push_str(text);
                        }
                    }
                    Some("thinking") => {
                        if let Some(text) = block.get("thinking").and_then(|t| t.as_str()) {
                            reasoning = Some(text.to_string());
                        }
                        thinking_blocks.push(block.clone());
                    }
                    Some("tool_use") => {
                        if let Some(tc) = parse_anthropic_tool_use(block) {
                            tool_calls.push(tc);
                        }
                    }
                    _ => {}
                }
            }
        }

        let finish_reason = json.get("stop_reason").and_then(|r| r.as_str()).map(|s| s.to_string());
        let usage = json.get("usage").and_then(|u| parse_anthropic_usage(u));

        Ok(LlmResponse {
            content, reasoning_content: reasoning, tool_calls, thinking_blocks,
            finish_reason, usage, ..Default::default()
        })
    }
}

fn convert_to_anthropic(messages: &[crate::call_validation::ChatMessage]) -> (Option<String>, Vec<Value>) {
    let mut system = None;
    let mut result = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => system = Some(msg.content.content_text_only()),
            "user" | "assistant" => {
                let content = msg_content_to_anthropic(&msg.content);
                let mut obj = json!({"role": msg.role, "content": content});
                if msg.role == "assistant" {
                    if let Some(tcs) = &msg.tool_calls {
                        let blocks: Vec<Value> = tcs.iter().map(|tc| json!({
                            "type": "tool_use", "id": tc.id,
                            "name": tc.function.name,
                            "input": serde_json::from_str::<Value>(&tc.function.arguments).unwrap_or(json!({}))
                        })).collect();
                        if let Some(arr) = obj["content"].as_array_mut() {
                            arr.extend(blocks);
                        }
                    }
                }
                result.push(obj);
            }
            "tool" => {
                result.push(json!({
                    "role": "user",
                    "content": [{"type": "tool_result", "tool_use_id": msg.tool_call_id, "content": msg.content.content_text_only()}]
                }));
            }
            "plain_text" | "cd_instruction" => {
                result.push(json!({"role": "user", "content": [{"type": "text", "text": msg.content.content_text_only()}]}));
            }
            _ => {}
        }
    }
    (system, result)
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
        crate::call_validation::ChatContent::ContextFiles(files) => {
            let text = files.iter()
                .map(|f| format!("{}:{}-{}\n```\n{}```", f.file_name, f.line1, f.line2, f.file_content))
                .collect::<Vec<_>>().join("\n\n");
            vec![json!({"type": "text", "text": text})]
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
    let prompt_tokens = usage.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as usize;
    let completion_tokens = usage.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as usize;
    Some(ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens + completion_tokens,
    })
}

fn parse_anthropic_tool_use(block: &Value) -> Option<ChatToolCall> {
    Some(ChatToolCall {
        id: block.get("id")?.as_str()?.to_string(),
        tool_type: "function".to_string(),
        function: ChatToolFunction {
            name: block.get("name")?.as_str()?.to_string(),
            arguments: serde_json::to_string(block.get("input")?).ok()?,
        },
        index: None,
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
    fn test_system_as_top_level() {
        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ];
        let (system, msgs) = convert_to_anthropic(&messages);
        assert_eq!(system, Some("Be helpful".to_string()));
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_parse_stream_text_delta() {
        let adapter = AnthropicAdapter;
        let chunk = r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello"}}"#;
        let deltas = adapter.parse_stream_chunk(chunk).unwrap();
        assert!(matches!(&deltas[0], LlmStreamDelta::AppendContent { text } if text == "Hello"));
    }

    #[test]
    fn test_parse_response_with_tool_use() {
        let adapter = AnthropicAdapter;
        let json = json!({"content": [{"type": "tool_use", "id": "tu_1", "name": "search", "input": {"q": "test"}}], "stop_reason": "tool_use"});
        let resp = adapter.parse_response(json).unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].function.name, "search");
    }
}
