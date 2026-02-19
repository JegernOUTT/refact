use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::call_validation::{ChatMessage, ChatMeta, ChatUsage};
use crate::llm::params::{CacheControl, CommonParams, ReasoningIntent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model_id: String,
    pub messages: Vec<ChatMessage>,
    pub params: CommonParams,
    #[serde(default)]
    pub reasoning: ReasoningIntent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<CanonicalToolChoice>,
    #[serde(default)]
    pub parallel_tool_calls: bool,
    #[serde(default)]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(default)]
    pub cache_control: CacheControl,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<serde_json::Map<String, Value>>,
    /// Metadata for Refact cloud (chat_id, mode, etc.) - sent when support_metadata is true
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ChatMeta>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        schema: Value,
        #[serde(default)]
        strict: bool,
    },
}

impl LlmRequest {
    pub fn new(model_id: String, messages: Vec<ChatMessage>) -> Self {
        Self {
            model_id,
            messages,
            params: CommonParams::default(),
            reasoning: ReasoningIntent::Off,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: false,
            stream: true,
            response_format: None,
            cache_control: CacheControl::Off,
            extra_body: None,
            meta: None,
            previous_response_id: None,
        }
    }

    pub fn with_params(mut self, params: CommonParams) -> Self {
        self.params = params;
        self
    }

    pub fn with_tools(mut self, tools: Vec<Value>, choice: Option<CanonicalToolChoice>) -> Self {
        if !tools.is_empty() {
            self.tools = Some(tools);
            self.tool_choice = choice;
        }
        self
    }

    pub fn with_reasoning(mut self, reasoning: ReasoningIntent) -> Self {
        self.reasoning = reasoning;
        self
    }

    pub fn with_parallel_tool_calls(mut self, parallel: bool) -> Self {
        self.parallel_tool_calls = parallel;
        self
    }

    pub fn with_meta(mut self, meta: ChatMeta) -> Self {
        self.meta = Some(meta);
        self
    }

    pub fn with_cache_control(mut self, cache_control: CacheControl) -> Self {
        self.cache_control = cache_control;
        self
    }

    pub fn with_previous_response_id(mut self, previous_response_id: Option<String>) -> Self {
        self.previous_response_id = previous_response_id;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalToolChoice {
    Auto,
    None,
    Required,
    Function { name: String },
}

impl Default for CanonicalToolChoice {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone)]
pub enum LlmStreamDelta {
    AppendContent {
        text: String,
        /// Content block index for per-block content tracking (Anthropic interleaved output).
        /// When set, the content text is associated with a specific content block.
        block_index: Option<u64>,
    },
    AppendReasoning {
        text: String,
        /// Content block index for per-block reasoning tracking (Anthropic interleaved thinking).
        /// When set, the reasoning text is associated with a specific thinking block.
        block_index: Option<u64>,
    },
    SetToolCalls {
        tool_calls: Vec<Value>,
    },
    FinalizeToolCalls {
        tool_calls: Vec<Value>,
    },
    SetThinkingBlocks {
        blocks: Vec<Value>,
    },
    AddCitation {
        citation: Value,
    },
    AddServerContentBlock {
        block: Value,
    },
    SetUsage {
        usage: ChatUsage,
    },
    SetFinishReason {
        reason: String,
    },
    MergeExtra {
        extra: serde_json::Map<String, Value>,
    },
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_request_builder() {
        let req = LlmRequest::new("gpt-4".to_string(), vec![])
            .with_params(CommonParams {
                max_tokens: 1000,
                ..Default::default()
            })
            .with_reasoning(ReasoningIntent::Medium);

        assert_eq!(req.model_id, "gpt-4");
        assert_eq!(req.params.max_tokens, 1000);
        assert_eq!(req.reasoning, ReasoningIntent::Medium);
    }

    #[test]
    fn test_stream_delta_variants() {
        use serde_json::json;

        let deltas = vec![
            LlmStreamDelta::AppendContent { text: "hello".to_string(), block_index: None },
            LlmStreamDelta::AppendReasoning { text: "thinking".to_string(), block_index: None },
            LlmStreamDelta::SetToolCalls { tool_calls: vec![json!({"id": "1"})] },
            LlmStreamDelta::SetThinkingBlocks { blocks: vec![json!({"type": "thinking", "text": "..."})] },
            LlmStreamDelta::AddCitation { citation: json!({"url": "https://example.com", "title": "Example"}) },
            LlmStreamDelta::SetUsage { usage: ChatUsage::default() },
            LlmStreamDelta::SetFinishReason { reason: "stop".to_string() },
            LlmStreamDelta::MergeExtra { extra: serde_json::Map::new() },
            LlmStreamDelta::Done,
        ];

        assert_eq!(deltas.len(), 9);
        assert!(matches!(&deltas[3], LlmStreamDelta::SetThinkingBlocks { blocks } if blocks.len() == 1));
        assert!(matches!(&deltas[4], LlmStreamDelta::AddCitation { citation } if citation.get("url").is_some()));
    }
}
