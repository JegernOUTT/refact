use serde::{Deserialize, Serialize};

use crate::provider_types::{CredentialSpec, ImageTokenMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireFormat {
    OpenaiChatCompletions,
    OpenaiResponses,
    AnthropicMessages,
    OllamaNative,
    GoogleCloudCode,
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
            Self::OllamaNative => write!(f, "ollama_native"),
            Self::GoogleCloudCode => write!(f, "google_cloudcode"),
        }
    }
}

pub fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Clone, Deserialize, Default, PartialEq)]
pub struct BaseModelRecord {
    #[serde(default)]
    pub n_ctx: usize,
    #[serde(default)]
    pub name: String,
    #[serde(skip_deserializing)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing)]
    pub endpoint: String,
    #[serde(default, skip_serializing)]
    pub endpoint_style: String,
    #[serde(default, skip_serializing)]
    pub wire_format: WireFormat,
    #[serde(default, skip_serializing)]
    pub api_key: String,
    #[serde(skip)]
    pub credential: Option<CredentialSpec>,
    #[serde(default, skip_serializing)]
    pub auth_token: String,
    #[serde(default, skip_serializing)]
    pub tokenizer_api_key: String,
    #[serde(default, skip_serializing)]
    pub extra_headers: std::collections::HashMap<String, String>,
    #[serde(default, skip_serializing)]
    pub similar_models: Vec<String>,
    #[serde(default)]
    pub tokenizer: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub experimental: bool,
    #[serde(default)]
    pub supports_max_completion_tokens: bool,
    #[serde(default)]
    pub eof_is_done: bool,
    #[serde(default)]
    pub supports_web_search: bool,
    #[serde(default = "default_true")]
    pub supports_cache_control: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_max_side_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_preferred_side_px: Option<u32>,
    #[serde(default)]
    pub image_token_mode: ImageTokenMode,
    #[serde(skip_deserializing)]
    pub removable: bool,
    #[serde(skip_deserializing)]
    pub user_configured: bool,
}

pub trait HasBaseModelRecord {
    fn base(&self) -> &BaseModelRecord;
    fn base_mut(&mut self) -> &mut BaseModelRecord;
}
