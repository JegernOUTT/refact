use std::collections::HashMap;

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::llm::canonical::{LlmRequest, LlmResponse, LlmStreamDelta};

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
    pub endpoint: String,
    pub extra_headers: HashMap<String, String>,
    pub model_name: String,
    pub supports_tools: bool,
    pub supports_reasoning: bool,
}

pub trait LlmWireAdapter: Send + Sync {
    fn build_http(&self, req: &LlmRequest, settings: &AdapterSettings) -> Result<HttpParts, String>;

    fn parse_stream_chunk(&self, data: &str) -> Result<Vec<LlmStreamDelta>, StreamParseError>;

    fn parse_response(&self, json: serde_json::Value) -> Result<LlmResponse, String>;
}

#[derive(Debug)]
pub enum StreamParseError {
    Skip,
    MalformedChunk(String),
    FatalError(String),
}

use std::sync::OnceLock;

static OPENAI_CHAT_ADAPTER: OnceLock<crate::llm::adapters::openai_chat::OpenAiChatAdapter> = OnceLock::new();
static OPENAI_RESPONSES_ADAPTER: OnceLock<crate::llm::adapters::openai_responses::OpenAiResponsesAdapter> = OnceLock::new();
static ANTHROPIC_ADAPTER: OnceLock<crate::llm::adapters::anthropic::AnthropicAdapter> = OnceLock::new();

pub fn get_adapter(format: WireFormat) -> &'static dyn LlmWireAdapter {
    match format {
        WireFormat::OpenaiChatCompletions => {
            OPENAI_CHAT_ADAPTER.get_or_init(|| crate::llm::adapters::openai_chat::OpenAiChatAdapter)
        }
        WireFormat::OpenaiResponses => {
            OPENAI_RESPONSES_ADAPTER.get_or_init(|| crate::llm::adapters::openai_responses::OpenAiResponsesAdapter)
        }
        WireFormat::AnthropicMessages => {
            ANTHROPIC_ADAPTER.get_or_init(|| crate::llm::adapters::anthropic::AnthropicAdapter)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wire_format_display() {
        assert_eq!(WireFormat::OpenaiChatCompletions.to_string(), "openai_chat_completions");
        assert_eq!(WireFormat::OpenaiResponses.to_string(), "openai_responses");
        assert_eq!(WireFormat::AnthropicMessages.to_string(), "anthropic_messages");
    }

    #[test]
    fn test_wire_format_serde() {
        let json = serde_json::to_string(&WireFormat::AnthropicMessages).unwrap();
        assert_eq!(json, "\"anthropic_messages\"");
        let parsed: WireFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, WireFormat::AnthropicMessages);
    }
}
