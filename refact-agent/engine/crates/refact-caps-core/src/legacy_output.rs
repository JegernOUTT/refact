use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize)]
pub struct LegacyCapsOutput {
    pub completion_models: BTreeMap<String, serde_json::Value>,
    pub completion_default_model: String,
    pub embedding_model: LegacyDisabledEmbeddingOutput,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegacyDisabledEmbeddingOutput {
    #[serde(flatten)]
    base: refact_core::llm_types::BaseModelRecord,
    embedding_size: i32,
    rejection_threshold: f32,
    embedding_batch: usize,
}

impl Default for LegacyDisabledEmbeddingOutput {
    fn default() -> Self {
        Self {
            base: Default::default(),
            embedding_size: 0,
            rejection_threshold: 0.63,
            embedding_batch: 64,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LegacyProviderOutput {
    pub completion_endpoint: String,
    pub embedding_endpoint: String,
    pub completion_models: Vec<serde_json::Value>,
    pub embedding_model: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LegacyModelsOutput {
    pub completion_models: Vec<serde_json::Value>,
    pub embedding_model: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_output_shapes_are_inert() {
        let disabled_embedding = serde_json::json!({
            "n_ctx":0,"name":"","id":"","tokenizer":"","enabled":false,
            "experimental":false,"supports_max_completion_tokens":false,"eof_is_done":false,
            "supports_web_search":false,"supports_cache_control":false,"image_token_mode":"provider",
            "removable":false,"user_configured":false,"embedding_size":0,
            "rejection_threshold":0.63_f32,"embedding_batch":64
        });
        assert_eq!(
            serde_json::to_value(LegacyCapsOutput::default()).unwrap(),
            serde_json::json!({"completion_models": {}, "completion_default_model": "", "embedding_model": disabled_embedding})
        );
        assert_eq!(
            serde_json::to_value(LegacyProviderOutput::default()).unwrap(),
            serde_json::json!({"completion_endpoint":"", "embedding_endpoint":"", "completion_models":[], "embedding_model":null})
        );
        assert_eq!(
            serde_json::to_value(LegacyModelsOutput::default()).unwrap(),
            serde_json::json!({"completion_models":[], "embedding_model":null})
        );
    }
}
