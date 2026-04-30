use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Map, Value};

use crate::call_validation::{ChatContent, ChatMessage, ChatUsage};
use crate::llm::adapter::{
    AdapterSettings, HttpParts, LlmWireAdapter, StreamParseError, insert_extra_headers,
};
use crate::llm::canonical::{LlmRequest, LlmStreamDelta};
use crate::llm::params::ReasoningIntent;

pub const OLLAMA_NUM_CTX_HEADER: &str = "x-refact-internal-ollama-num-ctx";
pub const OLLAMA_KEEP_ALIVE_HEADER: &str = "x-refact-internal-ollama-keep-alive";

pub struct OllamaAdapter;

impl LlmWireAdapter for OllamaAdapter {
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

        let mut body = json!({
            "model": settings.model_name,
            "messages": convert_messages_to_ollama(&req.messages),
            "stream": req.stream,
        });

        if settings.supports_tools {
            if let Some(tools) = &req.tools {
                let tools = convert_tools_to_ollama(tools);
                if !tools.is_empty() {
                    body["tools"] = json!(tools);
                }
            }
        } else if req.tools.is_some() {
            tracing::warn!(
                "model {} does not support tools, skipping tools in request",
                settings.model_name
            );
        }

        if reasoning_requested(&req.reasoning) {
            body["think"] = json!(true);
        }

        let options = build_options(req, settings);
        if !options.is_empty() {
            body["options"] = Value::Object(options);
        }

        if let Some(keep_alive) = settings
            .extra_headers
            .get(OLLAMA_KEEP_ALIVE_HEADER)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            body["keep_alive"] = json!(keep_alive);
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
            return Err(StreamParseError::FatalError(format_ollama_error(error)));
        }

        let mut deltas = Vec::new();

        if let Some(message) = json.get("message") {
            if let Some(content) = message.get("content").and_then(|v| v.as_str()) {
                if !content.is_empty() {
                    deltas.push(LlmStreamDelta::AppendContent {
                        text: content.to_string(),
                        block_index: None,
                    });
                }
            }

            if let Some(thinking) = message.get("thinking").and_then(|v| v.as_str()) {
                if !thinking.is_empty() {
                    deltas.push(LlmStreamDelta::AppendReasoning {
                        text: thinking.to_string(),
                        block_index: None,
                    });
                }
            }

            if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
                let normalized: Vec<_> = tool_calls
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, tool_call)| {
                        normalize_ollama_tool_call(tool_call, idx, &json)
                    })
                    .collect();
                if !normalized.is_empty() {
                    deltas.push(LlmStreamDelta::FinalizeToolCalls {
                        tool_calls: normalized,
                    });
                }
            }
        }

        if json.get("prompt_eval_count").is_some() || json.get("eval_count").is_some() {
            deltas.push(LlmStreamDelta::SetUsage {
                usage: parse_ollama_usage(&json),
            });
        }

        if json.get("done").and_then(|v| v.as_bool()) == Some(true) {
            let reason = json
                .get("done_reason")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("stop");
            deltas.push(LlmStreamDelta::SetFinishReason {
                reason: reason.to_string(),
            });
            deltas.push(LlmStreamDelta::Done);
        }

        Ok(deltas)
    }
}

fn build_options(req: &LlmRequest, settings: &AdapterSettings) -> Map<String, Value> {
    let mut options = Map::new();

    if let Some(num_ctx) = req
        .params
        .n_ctx
        .or_else(|| parse_internal_usize(settings, OLLAMA_NUM_CTX_HEADER))
    {
        options.insert("num_ctx".to_string(), json!(num_ctx));
    }

    if req.params.max_tokens > 0 {
        options.insert("num_predict".to_string(), json!(req.params.max_tokens));
    }

    if settings.supports_temperature {
        if let Some(temperature) = req.params.temperature {
            options.insert("temperature".to_string(), json!(temperature));
        }
        if let Some(top_p) = req.params.top_p {
            options.insert("top_p".to_string(), json!(top_p));
        }
    }

    if !req.params.stop.is_empty() {
        options.insert("stop".to_string(), json!(req.params.stop));
    }

    options
}

fn parse_internal_usize(settings: &AdapterSettings, key: &str) -> Option<usize> {
    settings
        .extra_headers
        .get(key)
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
}

fn reasoning_requested(reasoning: &ReasoningIntent) -> bool {
    !matches!(
        reasoning,
        ReasoningIntent::Off | ReasoningIntent::NoReasoning
    )
}

fn convert_messages_to_ollama(messages: &[ChatMessage]) -> Vec<Value> {
    use super::render_extra::{append_text_to_tool_json, is_context_role, render_context_message};

    let mut result: Vec<Value> = Vec::new();
    let mut pending_user_text = Vec::new();
    let mut pending_user_images = Vec::new();

    // Ollama native chat accepts tool results as role "tool" messages matched by tool_call_id.
    // Tool-result images are deferred to the next user message because Ollama expects images
    // on user messages.
    for msg in messages {
        if is_context_role(&msg.role) {
            let Some(text) = render_context_message(msg) else {
                continue;
            };
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
                pending_user_text.push(text);
            }
            continue;
        }

        let role = match msg.role.as_str() {
            "developer" | "system" => "system",
            "user" => "user",
            "assistant" => "assistant",
            "tool" | "diff" => "tool",
            _ => continue,
        };

        if role == "tool" && msg.tool_call_id.starts_with("srvtoolu_") {
            continue;
        }

        if role != "user" {
            push_pending_user_message(
                &mut result,
                &mut pending_user_text,
                &mut pending_user_images,
            );
        }

        let (text, images) = ollama_text_and_images(&msg.content);
        let mut obj = json!({"role": role, "content": text});

        if role == "user" {
            let mut content = std::mem::take(&mut pending_user_text);
            if !obj["content"].as_str().unwrap_or("").is_empty() {
                content.push(obj["content"].as_str().unwrap_or("").to_string());
            }
            obj["content"] = json!(content.join("\n\n"));

            let mut all_images = std::mem::take(&mut pending_user_images);
            all_images.extend(images);
            if !all_images.is_empty() {
                obj["images"] = json!(all_images);
            }
        } else if role == "tool" {
            if !msg.tool_call_id.is_empty() {
                obj["tool_call_id"] = json!(msg.tool_call_id);
            }
            if !images.is_empty() {
                pending_user_images.extend(images);
            }
        }

        if role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                let converted: Vec<_> = tool_calls
                    .iter()
                    .filter(|tc| !tc.id.starts_with("srvtoolu_"))
                    .filter_map(|tc| {
                        let name = tc.function.name.trim();
                        if name.is_empty() {
                            return None;
                        }
                        Some(json!({
                            "function": {
                                "name": name,
                                "arguments": parse_arguments_object(&tc.function.arguments),
                            }
                        }))
                    })
                    .collect();
                if !converted.is_empty() {
                    obj["tool_calls"] = json!(converted);
                }
            }
        }

        result.push(obj);
    }

    push_pending_user_message(
        &mut result,
        &mut pending_user_text,
        &mut pending_user_images,
    );
    result
}

fn push_pending_user_message(
    result: &mut Vec<Value>,
    pending_user_text: &mut Vec<String>,
    pending_user_images: &mut Vec<String>,
) {
    if pending_user_text.is_empty() && pending_user_images.is_empty() {
        return;
    }
    let mut obj = json!({
        "role": "user",
        "content": std::mem::take(pending_user_text).join("\n\n"),
    });
    let images = std::mem::take(pending_user_images);
    if !images.is_empty() {
        obj["images"] = json!(images);
    }
    result.push(obj);
}

fn ollama_text_and_images(content: &ChatContent) -> (String, Vec<String>) {
    match content {
        ChatContent::Multimodal(elements) => {
            let text = elements
                .iter()
                .filter(|el| el.m_type == "text")
                .map(|el| el.m_content.clone())
                .collect::<Vec<_>>()
                .join("\n\n");
            let images = elements
                .iter()
                .filter(|el| el.is_image())
                .map(|el| raw_base64(&el.m_content))
                .collect();
            (text, images)
        }
        _ => (content.content_text_only(), Vec::new()),
    }
}

fn raw_base64(content: &str) -> String {
    content
        .strip_prefix("data:")
        .and_then(|_| {
            content
                .split_once(',')
                .map(|(_, encoded)| encoded.to_string())
        })
        .unwrap_or_else(|| content.to_string())
}

fn parse_arguments_object(arguments: &str) -> Value {
    serde_json::from_str::<Value>(arguments.trim())
        .ok()
        .filter(|value| value.as_object().is_some())
        .unwrap_or_else(|| json!({}))
}

fn convert_tools_to_ollama(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            let function = tool.get("function")?.as_object()?;
            let name = function.get("name")?.as_str()?.trim();
            if name.is_empty() {
                return None;
            }

            let mut out_function = Map::new();
            out_function.insert("name".to_string(), json!(name));
            if let Some(description) = function.get("description") {
                out_function.insert("description".to_string(), description.clone());
            }
            out_function.insert(
                "parameters".to_string(),
                function
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
            );

            Some(json!({
                "type": "function",
                "function": Value::Object(out_function),
            }))
        })
        .collect()
}

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn stable_hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn stable_hash_str(hash: &mut u64, value: &str) {
    stable_hash_bytes(hash, &(value.len() as u64).to_le_bytes());
    stable_hash_bytes(hash, value.as_bytes());
}

fn stable_hash_value(hash: &mut u64, value: &Value) {
    match value {
        Value::Null => stable_hash_str(hash, "null"),
        Value::Bool(value) => {
            stable_hash_str(hash, "bool");
            stable_hash_str(hash, if *value { "true" } else { "false" });
        }
        Value::Number(value) => {
            stable_hash_str(hash, "number");
            stable_hash_str(hash, &value.to_string());
        }
        Value::String(value) => {
            stable_hash_str(hash, "string");
            stable_hash_str(hash, value);
        }
        Value::Array(values) => {
            stable_hash_str(hash, "array");
            stable_hash_bytes(hash, &(values.len() as u64).to_le_bytes());
            for value in values {
                stable_hash_value(hash, value);
            }
        }
        Value::Object(values) => {
            stable_hash_str(hash, "object");
            stable_hash_bytes(hash, &(values.len() as u64).to_le_bytes());
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            for (key, value) in entries {
                stable_hash_str(hash, key);
                stable_hash_value(hash, value);
            }
        }
    }
}

fn ollama_tool_call_id(index: usize, name: &str, arguments: &Value, response: &Value) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    stable_hash_str(&mut hash, "index");
    stable_hash_str(&mut hash, &index.to_string());
    stable_hash_str(&mut hash, "name");
    stable_hash_str(&mut hash, name);
    stable_hash_str(&mut hash, "arguments");
    stable_hash_value(&mut hash, arguments);
    for key in ["created_at", "model"] {
        if let Some(value) = response.get(key) {
            stable_hash_str(&mut hash, key);
            stable_hash_value(&mut hash, value);
        }
    }
    format!("ollama-tool-{index}-{:012x}", hash & 0x0000_ffff_ffff_ffff)
}

fn normalize_ollama_tool_call(tool_call: &Value, index: usize, response: &Value) -> Option<Value> {
    let function = tool_call.get("function")?;
    let name = function.get("name").and_then(|v| v.as_str())?.trim();
    if name.is_empty() {
        return None;
    }
    let arguments = function
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    // If Ollama omits response-level seeds and repeats the same tool name/arguments at the
    // same index across turns, perfect uniqueness is impossible.
    let id = ollama_tool_call_id(index, name, &arguments, response);

    Some(json!({
        "index": index,
        "id": id,
        "type": "function",
        "function": {
            "name": name,
            "arguments": arguments,
        }
    }))
}

fn parse_ollama_usage(json: &Value) -> ChatUsage {
    let prompt_tokens = json
        .get("prompt_eval_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let completion_tokens = json.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens + completion_tokens,
        cache_creation_tokens: None,
        cache_read_tokens: None,
        metering_usd: None,
    }
}

fn format_ollama_error(error: &Value) -> String {
    error
        .as_str()
        .or_else(|| error.get("message").and_then(|v| v.as_str()))
        .unwrap_or("unknown error")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatToolCall, ChatToolFunction};
    use crate::scratchpads::multimodality::MultimodalElement;

    fn default_settings() -> AdapterSettings {
        AdapterSettings {
            api_key: "ollama-key".to_string(),
            auth_token: String::new(),
            endpoint: "http://localhost:11434/api/chat".to_string(),
            extra_headers: Default::default(),
            model_name: "llama3.1:8b".to_string(),
            supports_tools: true,
            supports_reasoning: true,
            reasoning_type: None,
            supports_temperature: true,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            supports_web_search: false,
            supports_cache_control: false,
        }
    }

    #[test]
    fn build_http_emits_native_messages_tools_images_and_options() {
        let adapter = OllamaAdapter;
        let messages = vec![
            ChatMessage::new("system".to_string(), "You are helpful".to_string()),
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::Multimodal(vec![
                    MultimodalElement {
                        m_type: "text".to_string(),
                        m_content: "Look".to_string(),
                    },
                    MultimodalElement {
                        m_type: "image/png".to_string(),
                        m_content: "data:image/png;base64,abc123".to_string(),
                    },
                ]),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText(String::new()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    index: Some(0),
                    function: ChatToolFunction {
                        name: "read_file".to_string(),
                        arguments: r#"{"path":"/tmp/a.txt"}"#.to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("contents".to_string()),
                tool_call_id: "call_1".to_string(),
                ..Default::default()
            },
        ];
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a file",
                "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}
            }
        })];
        let mut req =
            LlmRequest::new("ollama/llama3.1:8b".to_string(), messages).with_tools(tools, None);
        req.params.n_ctx = Some(8192);
        req.params.max_tokens = 256;
        req.params.temperature = Some(0.2);
        req.params.stop = vec!["STOP".to_string()];
        let mut settings = default_settings();
        settings
            .extra_headers
            .insert(OLLAMA_NUM_CTX_HEADER.to_string(), "32768".to_string());
        settings
            .extra_headers
            .insert(OLLAMA_KEEP_ALIVE_HEADER.to_string(), "10m".to_string());

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(http.url, "http://localhost:11434/api/chat");
        assert_eq!(http.body["model"], "llama3.1:8b");
        assert_eq!(http.body["stream"], true);
        assert_eq!(http.body["keep_alive"], "10m");
        assert_eq!(http.body["options"]["num_ctx"], 8192);
        assert_eq!(http.body["options"]["num_predict"], 256);
        assert!((http.body["options"]["temperature"].as_f64().unwrap() - 0.2).abs() < 0.000_001);
        assert_eq!(http.body["options"]["stop"], json!(["STOP"]));
        assert!(http.body.get("cache_control").is_none());

        let messages = http.body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "Look");
        assert_eq!(messages[1]["images"], json!(["abc123"]));
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["name"],
            "read_file"
        );
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["arguments"]["path"],
            "/tmp/a.txt"
        );
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "call_1");
        assert_eq!(http.body["tools"][0]["function"]["name"], "read_file");

        assert_eq!(
            http.headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer ollama-key"
        );
        assert!(http.headers.get(OLLAMA_NUM_CTX_HEADER).is_none());
        assert!(http.headers.get(OLLAMA_KEEP_ALIVE_HEADER).is_none());
    }

    #[test]
    fn build_http_uses_provider_num_ctx_when_request_does_not_override() {
        let adapter = OllamaAdapter;
        let req = LlmRequest::new(
            "ollama/llama3.1:8b".to_string(),
            vec![ChatMessage::new("user".to_string(), "Hi".to_string())],
        );
        let mut settings = default_settings();
        settings
            .extra_headers
            .insert(OLLAMA_NUM_CTX_HEADER.to_string(), "16384".to_string());

        let http = adapter.build_http(&req, &settings).unwrap();

        assert_eq!(http.body["options"]["num_ctx"], 16384);
        assert!(http.headers.get(OLLAMA_NUM_CTX_HEADER).is_none());
    }

    #[test]
    fn build_http_sets_think_only_when_reasoning_requested() {
        let adapter = OllamaAdapter;
        let messages = vec![ChatMessage::new("user".to_string(), "Hi".to_string())];
        let req = LlmRequest::new("ollama/llama3.1:8b".to_string(), messages.clone());
        let http = adapter.build_http(&req, &default_settings()).unwrap();
        assert!(http.body.get("think").is_none());

        let req = LlmRequest::new("ollama/llama3.1:8b".to_string(), messages)
            .with_reasoning(ReasoningIntent::High);
        let http = adapter.build_http(&req, &default_settings()).unwrap();
        assert_eq!(http.body["think"], true);
    }

    fn tool_call_ids_from_chunk(chunk: &str) -> Vec<String> {
        let adapter = OllamaAdapter;
        adapter
            .parse_stream_chunk(chunk)
            .unwrap()
            .into_iter()
            .find_map(|delta| match delta {
                LlmStreamDelta::FinalizeToolCalls { tool_calls } => Some(
                    tool_calls
                        .into_iter()
                        .map(|tool_call| tool_call["id"].as_str().unwrap().to_string())
                        .collect(),
                ),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn ollama_tool_call_ids_are_stable_and_less_collision_prone() {
        let chunk = r#"{
            "model": "llama3.1:8b",
            "created_at": "2026-04-30T00:00:00Z",
            "message": {
                "tool_calls": [
                    {"function": {"name": "read_file", "arguments": {"path": "a.txt"}}},
                    {"function": {"name": "read_file", "arguments": {"path": "a.txt"}}}
                ]
            }
        }"#;

        let ids = tool_call_ids_from_chunk(chunk);
        let repeated = tool_call_ids_from_chunk(chunk);
        let changed_args = tool_call_ids_from_chunk(
            r#"{
            "model": "llama3.1:8b",
            "created_at": "2026-04-30T00:00:00Z",
            "message": {
                "tool_calls": [
                    {"function": {"name": "read_file", "arguments": {"path": "b.txt"}}}
                ]
            }
        }"#,
        );
        let changed_seed = tool_call_ids_from_chunk(
            r#"{
            "model": "llama3.2:8b",
            "created_at": "2026-04-30T00:00:00Z",
            "message": {
                "tool_calls": [
                    {"function": {"name": "read_file", "arguments": {"path": "a.txt"}}}
                ]
            }
        }"#,
        );

        assert_eq!(ids, repeated);
        assert_eq!(ids[0].len(), "ollama-tool-0-".len() + 12);
        assert!(ids[0].starts_with("ollama-tool-0-"));
        assert!(ids[1].starts_with("ollama-tool-1-"));
        assert_ne!(ids[0], ids[1]);
        assert_ne!(ids[0], changed_args[0]);
        assert_ne!(ids[0], changed_seed[0]);
    }

    #[test]
    fn parse_stream_chunk_maps_native_events() {
        let adapter = OllamaAdapter;
        let chunk = r#"{
            "message": {
                "content": "Hello",
                "thinking": "Reasoning",
                "tool_calls": [{"function": {"name": "read_file", "arguments": {"path": "a.txt"}}}]
            },
            "prompt_eval_count": 12,
            "eval_count": 5,
            "done": true,
            "done_reason": "stop"
        }"#;

        let deltas = adapter.parse_stream_chunk(chunk).unwrap();

        assert!(
            matches!(&deltas[0], LlmStreamDelta::AppendContent { text, .. } if text == "Hello")
        );
        assert!(
            matches!(&deltas[1], LlmStreamDelta::AppendReasoning { text, .. } if text == "Reasoning")
        );
        match &deltas[2] {
            LlmStreamDelta::FinalizeToolCalls { tool_calls } => {
                let id = tool_calls[0]["id"].as_str().unwrap();
                assert!(id.starts_with("ollama-tool-0-"));
                assert_ne!(id, "ollama-tool-0");
                assert_eq!(tool_calls[0]["function"]["name"], "read_file");
                assert_eq!(tool_calls[0]["function"]["arguments"]["path"], "a.txt");
            }
            _ => panic!("expected tool calls"),
        }
        match &deltas[3] {
            LlmStreamDelta::SetUsage { usage } => {
                assert_eq!(usage.prompt_tokens, 12);
                assert_eq!(usage.completion_tokens, 5);
                assert_eq!(usage.total_tokens, 17);
            }
            _ => panic!("expected usage"),
        }
        assert!(
            matches!(&deltas[4], LlmStreamDelta::SetFinishReason { reason } if reason == "stop")
        );
        assert!(matches!(deltas[5], LlmStreamDelta::Done));
    }

    #[test]
    fn parse_stream_chunk_maps_error_to_fatal() {
        let adapter = OllamaAdapter;
        let err = adapter.parse_stream_chunk(r#"{"error":"model not found"}"#);

        assert!(
            matches!(err, Err(StreamParseError::FatalError(message)) if message == "model not found")
        );
    }
}
