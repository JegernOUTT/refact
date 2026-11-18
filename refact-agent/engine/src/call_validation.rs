use serde::{Deserialize, Serialize};
use std::hash::Hash;
use axum::http::StatusCode;

use crate::custom_error::ScratchError;
use crate::llm::params::CacheControl;
use crate::worktrees::types::WorktreeMeta;

pub use refact_core::chat_types::{
    Checkpoint, ContextEnum, ContextFile, ChatContent, ChatMessage, ChatToolCall, ChatToolFunction,
    ChatUsage, MeteringUsd, MultimodalElement, OutputFilter, PostprocessSettings, ReasoningEffort,
    SamplingParameters, SearchResult, deserialize_path, format_search_results, serialize_path,
    normalize_mode_id, canonical_mode_id,
};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChatModelType {
    Light,
    Default,
    Thinking,
    Buddy,
}

impl Default for ChatModelType {
    fn default() -> Self {
        ChatModelType::Default
    }
}

fn default_subchat_cache_control() -> CacheControl {
    CacheControl::Ephemeral
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubchatParameters {
    #[serde(default)]
    pub subchat_model_type: ChatModelType,
    #[serde(default)]
    pub subchat_model: String,
    pub subchat_n_ctx: usize,
    pub subchat_max_new_tokens: usize,
    pub subchat_temperature: Option<f32>,
    pub subchat_tokens_for_rag: usize,
    pub subchat_reasoning_effort: Option<ReasoningEffort>,
    #[serde(default = "default_subchat_cache_control")]
    pub subchat_cache_control: CacheControl,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMeta {
    #[serde(default)]
    pub chat_id: String,
    #[serde(default)]
    pub request_attempt_id: String,
    #[serde(default)]
    pub chat_remote: bool,
    #[serde(default = "default_mode_id")]
    pub chat_mode: String,
    #[serde(default)]
    pub current_config_file: String,
    #[serde(default = "default_true")]
    pub include_project_info: bool,
    #[serde(default)]
    pub context_tokens_cap: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeMeta>,
}

fn default_mode_id() -> String {
    "agent".to_string()
}

impl Default for ChatMeta {
    fn default() -> Self {
        ChatMeta {
            chat_id: String::new(),
            request_attempt_id: String::new(),
            chat_remote: false,
            chat_mode: default_mode_id(),
            current_config_file: String::new(),
            include_project_info: true,
            context_tokens_cap: None,
            worktree: None,
        }
    }
}

/// Check if a mode ID is agentic (supports tool execution and knowledge enrichment).
pub fn is_agentic_mode_id(mode_id: &str) -> bool {
    matches!(mode_id, "agent" | "task_planner" | "task_agent")
}

/// Validate and canonicalize a mode ID with strict registry existence check.
/// Returns 422-compatible error if mode is invalid or doesn't exist in registry.
pub async fn validate_mode_for_request(
    gcx: std::sync::Arc<crate::global_context::GlobalContext>,
    mode: &str,
) -> Result<String, String> {
    let canonical = canonical_mode_id(mode)?;

    let mode_config =
        crate::yaml_configs::customization_registry::get_mode_config(gcx, &canonical, None).await;

    if mode_config.is_none() {
        return Err(format!("Mode '{}' does not exist in registry", canonical));
    }

    Ok(canonical)
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Hash, Debug, Eq, PartialEq, Default, Ord, PartialOrd)]
pub struct DiffChunk {
    pub file_name: String,
    pub file_action: String, // edit, rename, add, remove
    pub line1: usize,
    pub line2: usize,
    pub lines_remove: String,
    pub lines_add: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines_after: Option<String>,
    #[serde(default)]
    pub file_name_rename: Option<String>,
    #[serde(default = "default_true", skip_serializing)]
    pub is_file: bool,
    pub application_details: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool_fn(arguments: &str) -> ChatToolFunction {
        ChatToolFunction {
            arguments: arguments.to_string(),
            name: "test_tool".to_string(),
        }
    }

    #[test]
    fn test_parse_args_invalid_json_object() {
        // Starts with '{' but is malformed — should propagate the serde error
        let f = make_tool_fn("{broken json");
        assert!(f.parse_args().is_err());
    }
}

pub fn deserialize_messages_from_post(
    messages: &Vec<serde_json::Value>,
) -> Result<Vec<ChatMessage>, ScratchError> {
    let messages: Vec<ChatMessage> = messages
        .iter()
        .map(|x| serde_json::from_value(x.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            tracing::error!("can't deserialize ChatMessage: {}", e);
            ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
        })?;
    Ok(messages)
}
