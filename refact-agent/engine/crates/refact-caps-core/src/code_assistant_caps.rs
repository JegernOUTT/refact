use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use refact_core::llm_types::EmbeddingModelRecord;
use refact_core::model_caps::ModelCapabilities;
use refact_core::provider_types::ProviderDefaults;

use crate::model_records::{
    CapsMetadata, ChatModelRecord, CompletionModelRecord, DefaultModels,
    default_hf_tokenizer_template,
};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CodeAssistantCaps {
    #[serde(skip_deserializing)]
    pub completion_models: IndexMap<String, Arc<CompletionModelRecord>>,
    #[serde(skip_deserializing)]
    pub chat_models: IndexMap<String, Arc<ChatModelRecord>>,
    #[serde(skip_deserializing)]
    pub embedding_model: EmbeddingModelRecord,

    #[serde(flatten, skip_deserializing)]
    pub defaults: DefaultModels,

    #[serde(default)]
    pub caps_version: i64,

    #[serde(default)]
    pub customization: String,

    #[serde(default = "default_hf_tokenizer_template")]
    pub hf_tokenizer_template: String,

    #[serde(default)]
    pub metadata: CapsMetadata,

    #[serde(skip)]
    pub model_caps: Arc<HashMap<String, ModelCapabilities>>,

    #[serde(skip)]
    pub user_defaults: ProviderDefaults,

    #[serde(skip)]
    pub provider_base_names: HashMap<String, Vec<String>>,
}

impl Default for CodeAssistantCaps {
    fn default() -> Self {
        Self {
            completion_models: IndexMap::new(),
            chat_models: IndexMap::new(),
            embedding_model: EmbeddingModelRecord::default(),
            defaults: DefaultModels::default(),
            caps_version: 0,
            customization: String::new(),
            hf_tokenizer_template: default_hf_tokenizer_template(),
            metadata: CapsMetadata::default(),
            model_caps: Arc::new(HashMap::new()),
            user_defaults: ProviderDefaults::default(),
            provider_base_names: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_caps_preserves_empty_runtime_fields() {
        let caps = CodeAssistantCaps::default();

        assert!(caps.completion_models.is_empty());
        assert!(caps.chat_models.is_empty());
        assert_eq!(caps.embedding_model, EmbeddingModelRecord::default());
        assert_eq!(caps.defaults, DefaultModels::default());
        assert_eq!(caps.caps_version, 0);
        assert_eq!(caps.customization, "");
        assert_eq!(caps.hf_tokenizer_template, default_hf_tokenizer_template());
        assert_eq!(caps.metadata.pricing, serde_json::json!({}));
        assert!(caps.model_caps.is_empty());
        assert!(caps.provider_base_names.is_empty());
    }

    #[test]
    fn serde_defaults_and_skip_fields_match_caps_contract() {
        let decoded: CodeAssistantCaps = serde_json::from_value(serde_json::json!({})).unwrap();

        assert!(decoded.completion_models.is_empty());
        assert!(decoded.chat_models.is_empty());
        assert_eq!(decoded.embedding_model, EmbeddingModelRecord::default());
        assert_eq!(decoded.defaults, DefaultModels::default());
        assert_eq!(
            decoded.hf_tokenizer_template,
            default_hf_tokenizer_template()
        );
        assert!(decoded.model_caps.is_empty());
        assert!(decoded.provider_base_names.is_empty());

        let mut model_caps = HashMap::new();
        model_caps.insert(
            "provider/model".to_string(),
            ModelCapabilities {
                n_ctx: 123,
                ..Default::default()
            },
        );
        let mut caps = CodeAssistantCaps {
            model_caps: Arc::new(model_caps),
            ..Default::default()
        };
        caps.provider_base_names
            .insert("provider".to_string(), vec!["base".to_string()]);

        let value = serde_json::to_value(&caps).unwrap();
        assert!(value.get("model_caps").is_none());
        assert!(value.get("provider_base_names").is_none());
        assert!(value.get("user_defaults").is_none());
    }
}
