use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};

use crate::call_validation::{ChatToolCall, ChatToolFunction, ChatUsage};
use crate::llm::adapter::{AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError};
use crate::llm::canonical::{CanonicalToolChoice, LlmRequest, LlmResponse, LlmStreamDelta};

pub struct OpenAiResponsesAdapter;

impl LlmWireAdapter for OpenAiResponsesAdapter {
    fn build_http(&self, req: &LlmRequest, settings: &AdapterSettings) -> Result<HttpParts, String> {
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

        let (input, instructions) = convert_to_responses_format(&req.messages);

        let mut body = json!({
            "model": settings.model_name,
            "stream": req.stream,
            "max_output_tokens": req.params.max_tokens,
        });

        if !input.is_null() {
            body["input"] = input;
        }
        if let Some(inst) = instructions {
            body["instructions"] = json!(inst);
        }

        if let Some(temp) = req.params.temperature {
            body["temperature"] = json!(temp);
        }

        if settings.supports_tools {
            if let Some(tools) = &req.tools {
                if !tools.is_empty() {
                    body["tools"] = convert_tools_to_responses(tools);
                    if let Some(choice) = &req.tool_choice {
                        body["tool_choice"] = tool_choice_to_responses(choice);
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
                body["reasoning"] = json!({"effort": effort});
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

        let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let mut deltas = Vec::new();

        match event_type {
            "response.output_text.delta" => {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    deltas.push(LlmStreamDelta::AppendContent { text: delta.to_string() });
                }
            }
            "response.reasoning.delta" => {
                if let Some(delta) = json.get("delta").and_then(|d| d.as_str()) {
                    deltas.push(LlmStreamDelta::AppendReasoning { text: delta.to_string() });
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(tc) = extract_tool_call_delta(&json) {
                    deltas.push(LlmStreamDelta::SetToolCalls { tool_calls: vec![tc] });
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(tc) = extract_tool_call_done(&json) {
                    deltas.push(LlmStreamDelta::SetToolCalls { tool_calls: vec![tc] });
                }
            }
            "response.output_text.done" | "response.reasoning.done" => {
            }
            "response.completed" => {
                deltas.push(LlmStreamDelta::SetFinishReason { reason: "stop".to_string() });
                if let Some(usage) = extract_usage(&json) {
                    deltas.push(LlmStreamDelta::SetUsage { usage });
                }
                deltas.push(LlmStreamDelta::Done);
            }
            "response.failed" => {
                let error_msg = json.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("response failed");
                return Err(StreamParseError::FatalError(error_msg.to_string()));
            }
            "response.created" | "response.in_progress" => {
            }
            _ => {
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

        let output = json.get("output").and_then(|o| o.as_array());

        let mut content = String::new();
        let mut reasoning_content = None;
        let mut tool_calls = Vec::new();

        if let Some(items) = output {
            for item in items {
                let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match item_type {
                    "message" => {
                        if let Some(msg_content) = item.get("content").and_then(|c| c.as_array()) {
                            for part in msg_content {
                                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                    content.push_str(text);
                                }
                            }
                        }
                    }
                    "reasoning" => {
                        if let Some(summary) = item.get("summary").and_then(|s| s.as_array()) {
                            let reasoning: String = summary
                                .iter()
                                .filter_map(|s| s.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("");
                            if !reasoning.is_empty() {
                                reasoning_content = Some(reasoning);
                            }
                        }
                    }
                    "function_call" => {
                        if let Some(tc) = parse_responses_tool_call(item) {
                            tool_calls.push(tc);
                        }
                    }
                    _ => {}
                }
            }
        }

        let status = json.get("status").and_then(|s| s.as_str()).unwrap_or("unknown");
        let finish_reason = match status {
            "completed" => Some("stop".to_string()),
            "failed" => Some("error".to_string()),
            "incomplete" => Some("length".to_string()),
            _ => None,
        };

        let usage = extract_usage(&json);

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

fn convert_to_responses_format(messages: &[crate::call_validation::ChatMessage]) -> (Value, Option<String>) {
    let mut instructions = None;
    let mut input_messages = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                instructions = Some(msg.content.content_text_only());
            }
            "user" => {
                let content = msg_content_to_responses(&msg.content);
                input_messages.push(json!({
                    "role": "user",
                    "content": content
                }));
            }
            "assistant" => {
                let text_content = msg.content.content_text_only();
                if !text_content.is_empty() {
                    input_messages.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": text_content}]
                    }));
                }
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        input_messages.push(json!({
                            "type": "function_call",
                            "call_id": tc.id,
                            "name": tc.function.name,
                            "arguments": tc.function.arguments
                        }));
                    }
                }
            }
            "tool" => {
                input_messages.push(json!({
                    "type": "function_call_output",
                    "call_id": msg.tool_call_id,
                    "output": msg.content.content_text_only()
                }));
            }
            "plain_text" | "cd_instruction" => {
                input_messages.push(json!({
                    "role": "user",
                    "content": [{"type": "input_text", "text": msg.content.content_text_only()}]
                }));
            }
            _ => {}
        }
    }

    let input = if input_messages.is_empty() {
        Value::Null
    } else {
        json!(input_messages)
    };

    (input, instructions)
}

fn msg_content_to_responses(content: &crate::call_validation::ChatContent) -> Vec<Value> {
    match content {
        crate::call_validation::ChatContent::SimpleText(text) => {
            vec![json!({"type": "input_text", "text": text})]
        }
        crate::call_validation::ChatContent::Multimodal(elements) => {
            elements
                .iter()
                .map(|el| {
                    if el.is_image() {
                        json!({
                            "type": "input_image",
                            "image_url": format!("data:{};base64,{}", el.m_type, el.m_content)
                        })
                    } else {
                        json!({"type": "input_text", "text": el.m_content})
                    }
                })
                .collect()
        }
        crate::call_validation::ChatContent::ContextFiles(files) => {
            let text = files
                .iter()
                .map(|f| format!("{}:{}-{}\n```\n{}```", f.file_name, f.line1, f.line2, f.file_content))
                .collect::<Vec<_>>()
                .join("\n\n");
            vec![json!({"type": "input_text", "text": text})]
        }
    }
}

fn convert_tools_to_responses(tools: &[Value]) -> Value {
    let converted: Vec<Value> = tools
        .iter()
        .filter_map(|tool| {
            let func = tool.get("function")?;
            Some(json!({
                "type": "function",
                "name": func.get("name")?,
                "description": func.get("description").unwrap_or(&json!("")),
                "parameters": func.get("parameters").unwrap_or(&json!({}))
            }))
        })
        .collect();
    json!(converted)
}

fn tool_choice_to_responses(choice: &CanonicalToolChoice) -> Value {
    match choice {
        CanonicalToolChoice::Auto => json!("auto"),
        CanonicalToolChoice::None => json!("none"),
        CanonicalToolChoice::Required => json!("required"),
        CanonicalToolChoice::Function { name } => json!({"type": "function", "name": name}),
    }
}

fn extract_tool_call_delta(json: &Value) -> Option<Value> {
    Some(json!({
        "index": json.get("output_index")?,
        "id": json.get("item_id"),
        "type": "function",
        "function": {
            "arguments": json.get("delta")?
        }
    }))
}

fn extract_tool_call_done(json: &Value) -> Option<Value> {
    Some(json!({
        "index": json.get("output_index")?,
        "id": json.get("item_id"),
        "type": "function",
        "function": {
            "name": json.get("name")?,
            "arguments": json.get("arguments")?
        }
    }))
}

fn extract_usage(json: &Value) -> Option<ChatUsage> {
    let usage = json.get("usage")?;
    Some(ChatUsage {
        prompt_tokens: usage.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as usize,
        completion_tokens: usage.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as usize,
        total_tokens: usage.get("total_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as usize,
    })
}

fn parse_responses_tool_call(item: &Value) -> Option<ChatToolCall> {
    Some(ChatToolCall {
        id: item.get("call_id")?.as_str()?.to_string(),
        tool_type: "function".to_string(),
        function: ChatToolFunction {
            name: item.get("name")?.as_str()?.to_string(),
            arguments: item.get("arguments")?.as_str()?.to_string(),
        },
        index: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::ChatMessage;

    fn default_settings() -> AdapterSettings {
        AdapterSettings {
            api_key: "test-key".to_string(),
            endpoint: "https://api.openai.com/v1/responses".to_string(),
            extra_headers: Default::default(),
            model_name: "gpt-4.1".to_string(),
            supports_tools: true,
            supports_reasoning: true,
        }
    }

    #[test]
    fn test_build_http_basic() {
        let adapter = OpenAiResponsesAdapter;
        let req = LlmRequest::new(
            "gpt-4.1".to_string(),
            vec![
                ChatMessage::new("system".to_string(), "You are helpful".to_string()),
                ChatMessage::new("user".to_string(), "Hello".to_string()),
            ],
        );
        let settings = default_settings();

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(http.body["model"], "gpt-4.1");
        assert_eq!(http.body["instructions"], "You are helpful");
        assert!(http.body["input"].is_array());
    }

    #[test]
    fn test_build_http_with_reasoning() {
        let adapter = OpenAiResponsesAdapter;
        let req = LlmRequest::new("gpt-4.1".to_string(), vec![])
            .with_reasoning(crate::llm::params::ReasoningIntent::Medium);
        let settings = default_settings();

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(http.body["reasoning"]["effort"], "medium");
    }

    #[test]
    fn test_parse_stream_chunk_text_delta() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.output_text.delta","delta":"Hello"}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert_eq!(deltas.len(), 1);
        match &deltas[0] {
            LlmStreamDelta::AppendContent { text } => assert_eq!(text, "Hello"),
            _ => panic!("expected AppendContent"),
        }
    }

    #[test]
    fn test_parse_stream_chunk_completed() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.completed","usage":{"input_tokens":10,"output_tokens":5}}"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(deltas.iter().any(|d| matches!(d, LlmStreamDelta::Done)));
        assert!(deltas.iter().any(|d| matches!(d, LlmStreamDelta::SetFinishReason { .. })));
    }

    #[test]
    fn test_parse_stream_chunk_failed() {
        let adapter = OpenAiResponsesAdapter;
        let chunk = r#"{"type":"response.failed","error":{"message":"rate limit"}}"#;

        let result = adapter.parse_stream_chunk(chunk);

        assert!(matches!(result, Err(StreamParseError::FatalError(_))));
    }

    #[test]
    fn test_parse_response_basic() {
        let adapter = OpenAiResponsesAdapter;
        let json = json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "content": [{"type": "text", "text": "Hello!"}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}
        });

        let resp = adapter.parse_response(json).unwrap();

        assert_eq!(resp.content, "Hello!");
        assert_eq!(resp.finish_reason, Some("stop".to_string()));
    }

    #[test]
    fn test_parse_response_with_function_call() {
        let adapter = OpenAiResponsesAdapter;
        let json = json!({
            "status": "completed",
            "output": [{
                "type": "function_call",
                "call_id": "call_123",
                "name": "get_weather",
                "arguments": "{\"location\":\"NYC\"}"
            }]
        });

        let resp = adapter.parse_response(json).unwrap();

        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].function.name, "get_weather");
    }

    #[test]
    fn test_convert_system_to_instructions() {
        let messages = vec![
            ChatMessage::new("system".to_string(), "Be helpful".to_string()),
            ChatMessage::new("user".to_string(), "Hi".to_string()),
        ];

        let (input, instructions) = convert_to_responses_format(&messages);

        assert_eq!(instructions, Some("Be helpful".to_string()));
        assert_eq!(input.as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_convert_tool_loop_history() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};

        let messages = vec![
            ChatMessage::new("user".to_string(), "Get the weather".to_string()),
            ChatMessage {
                role: "assistant".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_123".to_string(),
                    tool_type: "function".to_string(),
                    function: ChatToolFunction {
                        name: "get_weather".to_string(),
                        arguments: r#"{"location":"NYC"}"#.to_string(),
                    },
                    index: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: crate::call_validation::ChatContent::SimpleText("Sunny, 72F".to_string()),
                tool_call_id: "call_123".to_string(),
                ..Default::default()
            },
        ];

        let (input, _) = convert_to_responses_format(&messages);
        let input_arr = input.as_array().unwrap();

        assert_eq!(input_arr.len(), 3);
        assert_eq!(input_arr[0]["role"], "user");
        assert_eq!(input_arr[1]["type"], "function_call");
        assert_eq!(input_arr[1]["call_id"], "call_123");
        assert_eq!(input_arr[1]["name"], "get_weather");
        assert_eq!(input_arr[2]["type"], "function_call_output");
        assert_eq!(input_arr[2]["call_id"], "call_123");
    }
}
