use std::collections::HashMap;

use reqwest::header::HeaderMap;
use refact_privacy::Cleared;
use serde_json::{Map, Value};

use crate::canonical::{LlmRequest, LlmStreamDelta};

pub use refact_core::llm_types::WireFormat;

pub struct HttpParts {
    pub url: String,
    pub headers: HeaderMap,
    pub body: serde_json::Value,
}

pub const OPENROUTER_EXTRA_BODY_HEADER: &str = "x-refact-internal-openrouter-extra-body";

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
    fn build_http(
        &self,
        req: &Cleared<LlmRequest>,
        settings: &AdapterSettings,
    ) -> Result<HttpParts, String>;

    fn parse_stream_chunk(&self, data: &str) -> Result<Vec<LlmStreamDelta>, StreamParseError>;
}

#[derive(Debug)]
pub enum StreamParseError {
    Skip,
    MalformedChunk(String),
    FatalError(String),
}

use std::sync::OnceLock;

static OPENAI_CHAT_ADAPTER: OnceLock<crate::adapters::openai_chat::OpenAiChatAdapter> =
    OnceLock::new();
static OPENAI_RESPONSES_ADAPTER: OnceLock<
    crate::adapters::openai_responses::OpenAiResponsesAdapter,
> = OnceLock::new();
static ANTHROPIC_ADAPTER: OnceLock<crate::adapters::anthropic::AnthropicAdapter> = OnceLock::new();
static OLLAMA_ADAPTER: OnceLock<crate::adapters::ollama::OllamaAdapter> = OnceLock::new();
static GOOGLE_CLOUDCODE_ADAPTER: OnceLock<
    crate::adapters::google_cloudcode::GoogleCloudCodeAdapter,
> = OnceLock::new();

pub fn get_adapter(format: WireFormat) -> &'static dyn LlmWireAdapter {
    match format {
        WireFormat::OpenaiChatCompletions => {
            OPENAI_CHAT_ADAPTER.get_or_init(|| crate::adapters::openai_chat::OpenAiChatAdapter)
        }
        WireFormat::OpenaiResponses => OPENAI_RESPONSES_ADAPTER
            .get_or_init(|| crate::adapters::openai_responses::OpenAiResponsesAdapter),
        WireFormat::AnthropicMessages => {
            ANTHROPIC_ADAPTER.get_or_init(|| crate::adapters::anthropic::AnthropicAdapter)
        }
        WireFormat::OllamaNative => {
            OLLAMA_ADAPTER.get_or_init(|| crate::adapters::ollama::OllamaAdapter)
        }
        WireFormat::GoogleCloudCode => GOOGLE_CLOUDCODE_ADAPTER
            .get_or_init(|| crate::adapters::google_cloudcode::GoogleCloudCodeAdapter),
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
