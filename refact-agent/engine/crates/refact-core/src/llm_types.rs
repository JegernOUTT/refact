use serde::{Deserialize, Serialize};
use tracing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireFormat {
    OpenaiChatCompletions,
    OpenaiResponses,
    AnthropicMessages,
    OllamaNative,
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
    #[serde(default, skip_serializing)]
    pub endpoint: String,
    #[serde(default, skip_serializing)]
    pub endpoint_style: String,
    #[serde(default, skip_serializing)]
    pub wire_format: WireFormat,
    #[serde(default, skip_serializing)]
    pub api_key: String,
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
    #[serde(skip_deserializing)]
    pub removable: bool,
    #[serde(skip_deserializing)]
    pub user_configured: bool,
}

pub trait HasBaseModelRecord {
    fn base(&self) -> &BaseModelRecord;
    fn base_mut(&mut self) -> &mut BaseModelRecord;
}

pub fn default_rejection_threshold() -> f32 {
    0.63
}

pub fn default_embedding_batch() -> usize {
    64
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct EmbeddingModelRecord {
    #[serde(flatten)]
    pub base: BaseModelRecord,
    pub embedding_size: i32,
    pub rejection_threshold: f32,
    pub embedding_batch: usize,
}

impl Default for EmbeddingModelRecord {
    fn default() -> Self {
        Self {
            base: BaseModelRecord::default(),
            embedding_size: 0,
            rejection_threshold: default_rejection_threshold(),
            embedding_batch: default_embedding_batch(),
        }
    }
}

impl HasBaseModelRecord for EmbeddingModelRecord {
    fn base(&self) -> &BaseModelRecord {
        &self.base
    }
    fn base_mut(&mut self) -> &mut BaseModelRecord {
        &mut self.base
    }
}

impl EmbeddingModelRecord {
    pub fn is_configured(&self) -> bool {
        !self.base.name.is_empty()
            && (self.embedding_size > 0 || self.embedding_batch > 0 || self.base.n_ctx > 0)
    }
}

impl<'de> Deserialize<'de> for EmbeddingModelRecord {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Input {
            String(String),
            Full(EmbeddingModelRecordHelper),
        }

        #[derive(Deserialize)]
        struct EmbeddingModelRecordHelper {
            #[serde(flatten)]
            base: BaseModelRecord,
            #[serde(default)]
            embedding_size: i32,
            #[serde(default = "default_rejection_threshold")]
            rejection_threshold: f32,
            #[serde(default = "default_embedding_batch")]
            embedding_batch: usize,
        }

        match Input::deserialize(deserializer)? {
            Input::String(name) => Ok(EmbeddingModelRecord {
                base: BaseModelRecord {
                    name,
                    ..Default::default()
                },
                ..Default::default()
            }),
            Input::Full(mut helper) => {
                if helper.embedding_batch > 256 {
                    tracing::warn!("embedding_batch can't be higher than 256");
                    helper.embedding_batch = default_embedding_batch();
                }
                Ok(EmbeddingModelRecord {
                    base: helper.base,
                    embedding_batch: helper.embedding_batch,
                    rejection_threshold: helper.rejection_threshold,
                    embedding_size: helper.embedding_size,
                })
            }
        }
    }
}
