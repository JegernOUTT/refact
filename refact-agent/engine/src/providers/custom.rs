use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::llm::adapter::WireFormat;
use crate::providers::config::resolve_env_var;
use crate::providers::traits::{CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait, parse_enabled_models, parse_custom_models, set_model_enabled_impl};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomProvider {
    pub api_key: String,
    pub chat_endpoint: String,
    pub completion_endpoint: String,
    pub embedding_endpoint: String,
    pub wire_format: Option<WireFormat>,
    pub enabled: bool,
    #[serde(default)]
    pub extra_headers: HashMap<String, String>,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

#[async_trait]
impl ProviderTrait for CustomProvider {
    fn name(&self) -> &'static str {
        "custom"
    }

    fn display_name(&self) -> &'static str {
        "Custom"
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
        self.wire_format.unwrap_or(WireFormat::OpenaiChatCompletions)
    }

    fn supported_wire_formats(&self) -> Vec<WireFormat> {
        vec![
            WireFormat::OpenaiChatCompletions,
            WireFormat::OpenaiResponses,
            WireFormat::AnthropicMessages,
        ]
    }

    fn model_filter_regex(&self) -> Option<&'static str> {
        None
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  api_key:
    f_type: string_long
    f_desc: "API key for the custom endpoint"
    f_label: "API Key"
  chat_endpoint:
    f_type: string_long
    f_desc: "Chat completions endpoint URL"
    f_placeholder: "https://your-server.com/v1/chat/completions"
    f_label: "Chat Endpoint"
  completion_endpoint:
    f_type: string_long
    f_desc: "Completions endpoint URL (optional)"
    f_placeholder: "https://your-server.com/v1/completions"
    f_label: "Completion Endpoint"
    f_extra: true
  embedding_endpoint:
    f_type: string_long
    f_desc: "Embeddings endpoint URL (optional)"
    f_placeholder: "https://your-server.com/v1/embeddings"
    f_label: "Embedding Endpoint"
    f_extra: true
  wire_format:
    f_type: string
    f_desc: "API format: openai_chat_completions, openai_responses, or anthropic_messages"
    f_default: "openai_chat_completions"
    f_label: "Wire Format"
    f_extra: true
description: |
  Custom OpenAI-compatible endpoint.
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
        if let Some(chat_endpoint) = yaml.get("chat_endpoint").and_then(|v| v.as_str()) {
            self.chat_endpoint = chat_endpoint.to_string();
        }
        if let Some(completion_endpoint) = yaml.get("completion_endpoint").and_then(|v| v.as_str()) {
            self.completion_endpoint = completion_endpoint.to_string();
        }
        if let Some(embedding_endpoint) = yaml.get("embedding_endpoint").and_then(|v| v.as_str()) {
            self.embedding_endpoint = embedding_endpoint.to_string();
        }
        if let Some(wire_format) = yaml.get("wire_format") {
            match serde_yaml::from_value(wire_format.clone()) {
                Ok(wf) => self.wire_format = Some(wf),
                Err(e) => return Err(format!("invalid wire_format: {e}")),
            }
        }
        if let Some(enabled) = yaml.get("enabled").and_then(|v| v.as_bool()) {
            self.enabled = enabled;
        }
        if let Some(headers) = yaml.get("extra_headers").and_then(|v| v.as_mapping()) {
            for (key, value) in headers {
                if let (Some(k), Some(v)) = (key.as_str(), value.as_str()) {
                    // Skip "***" values to preserve existing secrets
                    if v != "***" {
                        self.extra_headers.insert(k.to_string(), v.to_string());
                    }
                }
            }
            // Remove headers that are no longer in the new config (but preserve "***" ones)
            let new_keys: std::collections::HashSet<&str> = headers.keys()
                .filter_map(|k| k.as_str())
                .collect();
            self.extra_headers.retain(|k, _| new_keys.contains(k.as_str()));
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        // Redact extra_headers values (may contain secrets like Authorization)
        let redacted_headers: std::collections::HashMap<String, String> = self.extra_headers
            .keys()
            .map(|k| (k.clone(), "***".to_string()))
            .collect();

        json!({
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "chat_endpoint": self.chat_endpoint,
            "completion_endpoint": self.completion_endpoint,
            "embedding_endpoint": self.embedding_endpoint,
            "wire_format": self.wire_format,
            "enabled": self.enabled,
            "extra_headers": redacted_headers,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let api_key = resolve_env_var(&self.api_key, "", "custom api_key");

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !self.chat_endpoint.is_empty() && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: self.chat_endpoint.clone(),
            completion_endpoint: self.completion_endpoint.clone(),
            embedding_endpoint: self.embedding_endpoint.clone(),
            api_key,
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: self.extra_headers.clone(),
            support_metadata: false,
            supports_cache_control: true,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn has_credentials(&self) -> bool {
        !self.chat_endpoint.is_empty()
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::Manual  // Custom provider requires manual model definition
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
