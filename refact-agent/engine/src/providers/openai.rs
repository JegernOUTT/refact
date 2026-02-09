use std::any::Any;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::llm::adapter::WireFormat;
use crate::providers::config::resolve_env_var;
use crate::providers::traits::{
    CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};
use crate::providers::pricing::openai_pricing;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenAIProvider {
    pub api_key: String,
    pub enabled: bool,
    #[serde(default)]
    pub use_responses_api: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl ProviderTrait for OpenAIProvider {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn display_name(&self) -> &'static str {
        "OpenAI"
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
        Some(r"^(gpt-|o1-|o1$|o3-|o3$|o4-)")
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  api_key:
    f_type: string_long
    f_desc: "OpenAI API key from platform.openai.com"
    f_placeholder: "sk-..."
    f_label: "API Key"
    smartlinks:
      - sl_label: "Get API Key"
        sl_goto: "https://platform.openai.com/api-keys"
  use_responses_api:
    f_type: boolean
    f_desc: "Use the OpenAI Responses API instead of Chat Completions"
    f_label: "Use Responses API"
    f_default: false
description: |
  Direct access to OpenAI models (GPT-4, GPT-4o, o1, etc.).
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(api_key) = yaml.get("api_key").and_then(|v| v.as_str()) {
            if api_key != "***" {
                self.api_key = api_key.to_string();
            }
        }
        if let Some(enabled) = yaml.get("enabled").and_then(|v| v.as_bool()) {
            self.enabled = enabled;
        }
        if let Some(use_responses) = yaml.get("use_responses_api").and_then(|v| v.as_bool()) {
            self.use_responses_api = use_responses;
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        json!({
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "enabled": self.enabled,
            "use_responses_api": self.use_responses_api,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let api_key = resolve_env_var(&self.api_key, "", "openai api_key");

        let (wire_format, chat_endpoint) = if self.use_responses_api {
            (WireFormat::OpenaiResponses, "https://api.openai.com/v1/responses".to_string())
        } else {
            (WireFormat::OpenaiChatCompletions, "https://api.openai.com/v1/chat/completions".to_string())
        };

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !api_key.is_empty(),
            readonly: false,
            wire_format,
            chat_endpoint,
            completion_endpoint: "https://api.openai.com/v1/completions".to_string(),
            embedding_endpoint: "https://api.openai.com/v1/embeddings".to_string(),
            api_key,
            tokenizer_api_key: String::new(),
            extra_headers: HashMap::new(),
            support_metadata: false,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::ModelCaps
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
        if let Some(config) = self.custom_models.get(model_id) {
            if config.pricing.is_some() {
                return config.pricing.clone();
            }
        }
        openai_pricing(model_id)
    }
}
