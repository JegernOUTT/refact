use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::llm::adapter::WireFormat;
use crate::providers::traits::{CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait, parse_enabled_models, parse_custom_models, set_model_enabled_impl};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaProvider {
    pub endpoint: String,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".to_string(),
            enabled: false,
            enabled_models: Vec::new(),
            custom_models: HashMap::new(),
        }
    }
}

#[async_trait]
impl ProviderTrait for OllamaProvider {
    fn name(&self) -> &'static str {
        "ollama"
    }

    fn display_name(&self) -> &'static str {
        "Ollama"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProviderTrait> {
        Box::new(self.clone())
    }

    fn default_wire_format(&self) -> WireFormat {
        WireFormat::OpenaiChatCompletions
    }

    fn model_filter_regex(&self) -> Option<&'static str> {
        None
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  endpoint:
    f_type: string_long
    f_desc: "Ollama server endpoint"
    f_placeholder: "http://localhost:11434"
    f_label: "Endpoint"
    f_default: "http://localhost:11434"
description: |
  Local Ollama server for running open-source models.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(endpoint) = yaml.get("endpoint").and_then(|v| v.as_str()) {
            self.endpoint = endpoint.to_string();
        }
        if let Some(enabled) = yaml.get("enabled").and_then(|v| v.as_bool()) {
            self.enabled = enabled;
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        json!({
            "endpoint": self.endpoint,
            "enabled": self.enabled,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let base_url = self.endpoint.trim_end_matches('/');

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled,
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: format!("{}/v1/chat/completions", base_url),
            completion_endpoint: format!("{}/v1/completions", base_url),
            embedding_endpoint: format!("{}/v1/embeddings", base_url),
            api_key: String::new(),
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: HashMap::new(),
            support_metadata: false,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::Local  // Ollama discovers models locally
    }

    fn enabled_models(&self) -> &[String] {
        &self.enabled_models
    }

    fn custom_models(&self) -> &HashMap<String, CustomModelConfig> {
        &self.custom_models
    }

    fn set_model_enabled(&mut self, model_id: &str, enabled: bool) {
        set_model_enabled_impl(&mut self.enabled_models, model_id, enabled);
    }

    fn add_custom_model(&mut self, model_id: String, config: CustomModelConfig) {
        self.custom_models.insert(model_id, config);
    }

    fn remove_custom_model(&mut self, model_id: &str) -> bool {
        self.custom_models.remove(model_id).is_some()
    }

    fn model_pricing(&self, model_id: &str) -> Option<ModelPricing> {
        self.custom_models.get(model_id).and_then(|c| c.pricing.clone())
    }
}
