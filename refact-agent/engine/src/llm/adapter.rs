use std::collections::HashMap;

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::llm::canonical::{LlmRequest, LlmStreamDelta};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireFormat {
    OpenaiChatCompletions,
    OpenaiResponses,
    AnthropicMessages,
}

impl Default for WireFormat {
    fn default() -> Self {
        Self::OpenaiChatCompletions
    }
}

impl std::fmt::Display for WireFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenaiChatCompletions => write!(f, "openai_chat_completions"),
            Self::OpenaiResponses => write!(f, "openai_responses"),
            Self::AnthropicMessages => write!(f, "anthropic_messages"),
        }
    }
}

pub struct HttpParts {
    pub url: String,
    pub headers: HeaderMap,
    pub body: serde_json::Value,
}

pub struct AdapterSettings {
    pub api_key: String,
    pub auth_token: String,
    pub endpoint: String,
    pub extra_headers: HashMap<String, String>,
    pub model_name: String,
    pub supports_tools: bool,
    pub supports_reasoning: bool,
    pub reasoning_type: Option<String>,
    pub supports_temperature: bool,
    pub supports_max_completion_tokens: bool,
    pub eof_is_done: bool,
    pub supports_web_search: bool,
    pub supports_cache_control: bool,
}

pub trait LlmWireAdapter: Send + Sync {
    fn build_http(&self, req: &LlmRequest, settings: &AdapterSettings)
        -> Result<HttpParts, String>;

    fn parse_stream_chunk(&self, data: &str) -> Result<Vec<LlmStreamDelta>, StreamParseError>;
}

#[derive(Debug)]
pub enum StreamParseError {
    Skip,
    MalformedChunk(String),
    FatalError(String),
}

use std::sync::OnceLock;

static OPENAI_CHAT_ADAPTER: OnceLock<crate::llm::adapters::openai_chat::OpenAiChatAdapter> =
    OnceLock::new();
static OPENAI_RESPONSES_ADAPTER: OnceLock<
    crate::llm::adapters::openai_responses::OpenAiResponsesAdapter,
> = OnceLock::new();
static ANTHROPIC_ADAPTER: OnceLock<crate::llm::adapters::anthropic::AnthropicAdapter> =
    OnceLock::new();

pub fn get_adapter(format: WireFormat) -> &'static dyn LlmWireAdapter {
    match format {
        WireFormat::OpenaiChatCompletions => {
            OPENAI_CHAT_ADAPTER.get_or_init(|| crate::llm::adapters::openai_chat::OpenAiChatAdapter)
        }
        WireFormat::OpenaiResponses => OPENAI_RESPONSES_ADAPTER
            .get_or_init(|| crate::llm::adapters::openai_responses::OpenAiResponsesAdapter),
        WireFormat::AnthropicMessages => {
            ANTHROPIC_ADAPTER.get_or_init(|| crate::llm::adapters::anthropic::AnthropicAdapter)
        }
    }
}

const PROTECTED_HEADERS: &[&str] = &[
    "authorization",
    "x-api-key",
    "anthropic-version",
    "api-key",
    "x-goog-api-key",
    "content-type",
    "content-length",
    "host",
    "connection",
    "transfer-encoding",
];

/// Insert extra headers while protecting security-sensitive headers from override
pub fn insert_extra_headers(headers: &mut HeaderMap, extra_headers: &HashMap<String, String>) {
    for (key, value) in extra_headers {
        let key_lower = key.to_lowercase();
        if key_lower.starts_with("x-refact-internal-") {
            continue;
        }
        if PROTECTED_HEADERS.contains(&key_lower.as_str()) {
            tracing::warn!(
                "extra_headers attempted to override protected header '{}', ignoring",
                key
            );
            continue;
        }
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(key.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            headers.insert(name, val);
        }
    }
}

/// Extract provider-specific extra fields from streaming response chunks.
/// These include billing, cost, cache fields and provider-specific data.
/// Handles both top-level fields and nested fields under "response" wrapper.
pub fn extract_extra_fields(json: &Value) -> Map<String, Value> {
    let mut result = Map::new();

    fn extract_from_object(obj: &serde_json::Map<String, Value>, result: &mut Map<String, Value>) {
        for (key, val) in obj {
            if val.is_null() {
                continue;
            }
            let is_extra = key.starts_with("billing_")
                || key.starts_with("cost_")
                || key.starts_with("cache_")
                || key == "system_fingerprint";
            if is_extra {
                result.insert(key.clone(), val.clone());
            }
        }
        if let Some(psf) = obj.get("provider_specific_fields") {
            if !psf.is_null() {
                result.insert("provider_specific_fields".to_string(), psf.clone());
            }
        }
    }

    // Extract from top-level
    if let Some(obj) = json.as_object() {
        extract_from_object(obj, &mut result);

        // Also extract from nested "response" wrapper (OpenAI Responses API)
        if let Some(response_obj) = obj.get("response").and_then(|r| r.as_object()) {
            extract_from_object(response_obj, &mut result);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_wire_format_display() {
        assert_eq!(
            WireFormat::OpenaiChatCompletions.to_string(),
            "openai_chat_completions"
        );
        assert_eq!(WireFormat::OpenaiResponses.to_string(), "openai_responses");
        assert_eq!(
            WireFormat::AnthropicMessages.to_string(),
            "anthropic_messages"
        );
    }

    #[test]
    fn test_wire_format_serde() {
        let json = serde_json::to_string(&WireFormat::AnthropicMessages).unwrap();
        assert_eq!(json, "\"anthropic_messages\"");
        let parsed: WireFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, WireFormat::AnthropicMessages);
    }

    #[test]
    fn test_extract_extra_fields_top_level() {
        let json = json!({
            "id": "chatcmpl-123",
            "billing_amount": 0.01,
            "cost_total": 0.02,
            "cache_status": "hit",
            "system_fingerprint": "fp_abc123"
        });

        let extra = extract_extra_fields(&json);

        assert_eq!(extra.get("billing_amount"), Some(&json!(0.01)));
        assert_eq!(extra.get("cost_total"), Some(&json!(0.02)));
        assert_eq!(extra.get("cache_status"), Some(&json!("hit")));
        assert_eq!(extra.get("system_fingerprint"), Some(&json!("fp_abc123")));
        // Non-extra fields should not be included
        assert!(extra.get("id").is_none());
    }

    #[test]
    fn test_extract_extra_fields_nested_response() {
        // OpenAI Responses API wraps data under "response"
        let json = json!({
            "type": "response.completed",
            "response": {
                "id": "resp_123",
                "system_fingerprint": "fp_nested"
            }
        });

        let extra = extract_extra_fields(&json);

        assert_eq!(extra.get("system_fingerprint"), Some(&json!("fp_nested")));
    }

    #[test]
    fn test_extract_extra_fields_ignores_null() {
        let json = json!({
            "billing_amount": null,
            "cost_total": 100
        });

        let extra = extract_extra_fields(&json);

        assert!(extra.get("billing_amount").is_none());
        assert_eq!(extra.get("cost_total"), Some(&json!(100)));
    }

    #[test]
    fn test_insert_extra_headers_protects_auth_headers() {
        use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
        use std::collections::HashMap;

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer secret-key"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("x-api-key", HeaderValue::from_static("api-key-123"));

        let mut extra = HashMap::new();
        extra.insert("Authorization".to_string(), "Bearer HACKED".to_string());
        extra.insert("x-api-key".to_string(), "HACKED-KEY".to_string());
        extra.insert("content-type".to_string(), "text/plain".to_string());
        extra.insert("X-Custom-Header".to_string(), "allowed-value".to_string());
        extra.insert(
            "x-refact-internal-test".to_string(),
            "internal-value".to_string(),
        );

        insert_extra_headers(&mut headers, &extra);

        // Protected headers should NOT be overwritten
        assert_eq!(headers.get(AUTHORIZATION).unwrap(), "Bearer secret-key");
        assert_eq!(headers.get("x-api-key").unwrap(), "api-key-123");
        assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");

        // Non-protected headers should be added
        assert_eq!(headers.get("X-Custom-Header").unwrap(), "allowed-value");
        assert!(headers.get("x-refact-internal-test").is_none());
    }
}
