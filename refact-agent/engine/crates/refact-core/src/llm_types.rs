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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionEndpointStyle {
    OpenaiCompletions,
    OpenaiChatCompletions,
    OpenaiResponses,
}

impl CompletionEndpointStyle {
    pub fn from_config(value: &str, key: &str) -> Result<Self, String> {
        match value.trim() {
            "openai_completions" => Ok(Self::OpenaiCompletions),
            "openai_chat_completions" => Ok(Self::OpenaiChatCompletions),
            "openai_responses" => Ok(Self::OpenaiResponses),
            other => Err(format!("Invalid {key}: {other}")),
        }
    }

    pub fn is_supported(self) -> bool {
        matches!(self, Self::OpenaiCompletions | Self::OpenaiChatCompletions)
    }
}

impl std::fmt::Display for CompletionEndpointStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenaiCompletions => write!(f, "openai_completions"),
            Self::OpenaiChatCompletions => write!(f, "openai_chat_completions"),
            Self::OpenaiResponses => write!(f, "openai_responses"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingEndpointStyle {
    Openai,
    OllamaNative,
    AzureOpenai,
    Gemini,
    CohereV2,
    Voyage,
    Jina,
}

impl EmbeddingEndpointStyle {
    pub fn from_config(value: &str, key: &str) -> Result<Self, String> {
        match value.trim() {
            "" | "openai" => Ok(Self::Openai),
            "ollama_native" => Ok(Self::OllamaNative),
            "azure_openai" => Ok(Self::AzureOpenai),
            "gemini" => Ok(Self::Gemini),
            "cohere_v2" => Ok(Self::CohereV2),
            "voyage" => Ok(Self::Voyage),
            "jina" => Ok(Self::Jina),
            other => Err(format!("Invalid {key}: {other}")),
        }
    }

    pub fn is_supported(self) -> bool {
        matches!(self, Self::Openai | Self::OllamaNative)
    }
}

impl std::fmt::Display for EmbeddingEndpointStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Openai => write!(f, "openai"),
            Self::OllamaNative => write!(f, "ollama_native"),
            Self::AzureOpenai => write!(f, "azure_openai"),
            Self::Gemini => write!(f, "gemini"),
            Self::CohereV2 => write!(f, "cohere_v2"),
            Self::Voyage => write!(f, "voyage"),
            Self::Jina => write!(f, "jina"),
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub completion_endpoint_style: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub embedding_endpoint_style: String,
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

impl BaseModelRecord {
    pub fn effective_completion_endpoint_style(&self) -> Result<CompletionEndpointStyle, String> {
        let style = self.completion_endpoint_style.trim();
        if style.is_empty() {
            return Ok(CompletionEndpointStyle::OpenaiCompletions);
        }
        CompletionEndpointStyle::from_config(style, "completion_endpoint_style")
    }

    pub fn effective_embedding_endpoint_style(&self) -> Result<EmbeddingEndpointStyle, String> {
        let style = self.embedding_endpoint_style.trim();
        if !style.is_empty() {
            return EmbeddingEndpointStyle::from_config(style, "embedding_endpoint_style");
        }
        EmbeddingEndpointStyle::from_config(&self.endpoint_style, "embedding_endpoint_style")
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<usize>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub query_prefix: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub document_prefix: String,
    pub rejection_threshold: f32,
    pub embedding_batch: usize,
}

impl Default for EmbeddingModelRecord {
    fn default() -> Self {
        Self {
            base: BaseModelRecord::default(),
            embedding_size: 0,
            dimensions: None,
            query_prefix: String::new(),
            document_prefix: String::new(),
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
            #[serde(default)]
            dimensions: Option<usize>,
            #[serde(default)]
            query_prefix: String,
            #[serde(default)]
            document_prefix: String,
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
                    dimensions: helper.dimensions,
                    query_prefix: helper.query_prefix,
                    document_prefix: helper.document_prefix,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_endpoint_style_parses_supported_and_future_styles() {
        assert_eq!(
            CompletionEndpointStyle::from_config("openai_completions", "completion_endpoint_style")
                .unwrap(),
            CompletionEndpointStyle::OpenaiCompletions
        );
        assert!(CompletionEndpointStyle::OpenaiCompletions.is_supported());
        assert_eq!(
            CompletionEndpointStyle::from_config("openai_responses", "completion_endpoint_style")
                .unwrap(),
            CompletionEndpointStyle::OpenaiResponses
        );
        assert!(!CompletionEndpointStyle::OpenaiResponses.is_supported());
        assert_eq!(
            CompletionEndpointStyle::from_config("bad", "completion_endpoint_style").unwrap_err(),
            "Invalid completion_endpoint_style: bad"
        );
    }

    #[test]
    fn embedding_endpoint_style_parses_supported_and_future_styles() {
        assert_eq!(
            EmbeddingEndpointStyle::from_config("", "embedding_endpoint_style").unwrap(),
            EmbeddingEndpointStyle::Openai
        );
        assert!(EmbeddingEndpointStyle::OllamaNative.is_supported());
        assert_eq!(
            EmbeddingEndpointStyle::from_config("voyage", "embedding_endpoint_style").unwrap(),
            EmbeddingEndpointStyle::Voyage
        );
        assert!(!EmbeddingEndpointStyle::Voyage.is_supported());
        assert_eq!(
            EmbeddingEndpointStyle::from_config("bad", "embedding_endpoint_style").unwrap_err(),
            "Invalid embedding_endpoint_style: bad"
        );
    }

    #[test]
    fn base_model_effective_styles_keep_legacy_defaults() {
        let legacy = BaseModelRecord {
            endpoint_style: "openai".to_string(),
            ..Default::default()
        };

        assert_eq!(
            legacy.effective_completion_endpoint_style().unwrap(),
            CompletionEndpointStyle::OpenaiCompletions
        );
        assert_eq!(
            legacy.effective_embedding_endpoint_style().unwrap(),
            EmbeddingEndpointStyle::Openai
        );
    }

    #[test]
    fn embedding_model_metadata_survives_serde_roundtrip() {
        let yaml = r#"
name: embed
embedding_endpoint_style: ollama_native
embedding_size: 768
dimensions: 512
query_prefix: "query: "
document_prefix: "passage: "
"#;

        let decoded: EmbeddingModelRecord = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(decoded.base.embedding_endpoint_style, "ollama_native");
        assert_eq!(decoded.dimensions, Some(512));
        assert_eq!(decoded.query_prefix, "query: ");
        assert_eq!(decoded.document_prefix, "passage: ");

        let value = serde_yaml::to_value(&decoded).unwrap();
        assert_eq!(value.get("dimensions").and_then(|v| v.as_i64()), Some(512));
        assert_eq!(
            value
                .get("embedding_endpoint_style")
                .and_then(|v| v.as_str()),
            Some("ollama_native")
        );
    }
}
