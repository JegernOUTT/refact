use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::call_validation::{ChatMessage, ChatToolCall, ChatUsage};
use crate::llm::params::{CommonParams, ReasoningIntent};

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
    pub extra_body: Option<serde_json::Map<String, Value>>,
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
            extra_body: None,
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

#[derive(Debug, Clone, Default)]
pub struct LlmResponse {
    pub content: String,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ChatToolCall>,
    pub thinking_blocks: Vec<Value>,
    pub citations: Vec<Value>,
    pub finish_reason: Option<String>,
    pub usage: Option<ChatUsage>,
    pub extra: serde_json::Map<String, Value>,
}

impl LlmResponse {
    pub fn into_message(self, role: &str) -> ChatMessage {
        let mut msg = ChatMessage::new(role.to_string(), self.content);
        msg.reasoning_content = self.reasoning_content;
        if !self.tool_calls.is_empty() {
            msg.tool_calls = Some(self.tool_calls);
        }
        if !self.thinking_blocks.is_empty() {
            msg.thinking_blocks = Some(self.thinking_blocks);
        }
        msg.citations = self.citations;
        msg.finish_reason = self.finish_reason;
        msg.usage = self.usage;
        msg.extra = self.extra;
        msg
    }
}

#[derive(Debug, Clone)]
pub enum LlmStreamDelta {
    AppendContent { text: String },
    AppendReasoning { text: String },
    SetToolCalls { tool_calls: Vec<Value> },
    SetThinkingBlocks { blocks: Vec<Value> },
    AddCitation { citation: Value },
    SetUsage { usage: ChatUsage },
    SetFinishReason { reason: String },
    MergeExtra { extra: serde_json::Map<String, Value> },
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_request_builder() {
        let req = LlmRequest::new("gpt-4".to_string(), vec![])
            .with_params(CommonParams { max_tokens: 1000, ..Default::default() })
            .with_reasoning(ReasoningIntent::Medium);

        assert_eq!(req.model_id, "gpt-4");
        assert_eq!(req.params.max_tokens, 1000);
        assert_eq!(req.reasoning, ReasoningIntent::Medium);
    }

    #[test]
    fn test_llm_response_into_message() {
        let resp = LlmResponse {
            content: "Hello".to_string(),
            finish_reason: Some("stop".to_string()),
            ..Default::default()
        };
        let msg = resp.into_message("assistant");
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.content.content_text_only(), "Hello");
        assert_eq!(msg.finish_reason, Some("stop".to_string()));
    }
}
