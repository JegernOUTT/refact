use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::llm::adapter::WireFormat;
use crate::providers::config::resolve_env_var;
use crate::providers::traits::{
    CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    extra_headers_mapping_to_hash_map, parse_custom_models, parse_enabled_models,
    parse_extra_headers_value, set_model_enabled_impl,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomProvider {
    pub api_key: String,
    pub chat_endpoint: String,
    pub completion_endpoint: String,
    pub embedding_endpoint: String,
    pub wire_format: Option<WireFormat>,
    pub enabled: bool,
    #[serde(default)]
    pub supports_cache_control: bool,
    #[serde(default)]
    pub extra_headers: HashMap<String, String>,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

#[async_trait]
impl ProviderTrait for CustomProvider {
    fn name(&self) -> &str {
        "custom"
    }

    fn display_name(&self) -> &str {
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
        self.wire_format
            .unwrap_or(WireFormat::OpenaiChatCompletions)
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
  supports_cache_control:
    f_type: boolean
    f_desc: "Send Anthropic-style cache-control fields to the custom endpoint"
    f_label: "Enable Cache Control"
    f_default: false
    f_extra: true
  extra_headers:
    f_type: string_long
    f_desc: "Advanced JSON/YAML object of additional HTTP headers. Values are redacted as *** when read back; send *** to preserve an existing header and omit or null a key to remove it."
    f_label: "Extra Headers"
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
        if let Some(completion_endpoint) = yaml.get("completion_endpoint").and_then(|v| v.as_str())
        {
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
        if let Some(supports_cache_control) =
            yaml.get("supports_cache_control").and_then(|v| v.as_bool())
        {
            self.supports_cache_control = supports_cache_control;
        }
        if let Some(headers_value) = yaml.get("extra_headers") {
            let headers = parse_extra_headers_value(headers_value)?;
            self.extra_headers =
                extra_headers_mapping_to_hash_map(Some(&self.extra_headers), &headers);
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        // Redact extra_headers values (may contain secrets like Authorization)
        let redacted_headers: std::collections::HashMap<String, String> = self
            .extra_headers
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
            "supports_cache_control": self.supports_cache_control,
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
            enabled: self.enabled
                && !self.chat_endpoint.is_empty()
                && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: self.chat_endpoint.clone(),
            completion_endpoint: self.completion_endpoint.clone(),
            embedding_endpoint: self.embedding_endpoint.clone(),
            api_key,
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: self.extra_headers.clone(),
            supports_cache_control: self.supports_cache_control,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn has_credentials(&self) -> bool {
        !self.chat_endpoint.is_empty()
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::Manual // Custom provider requires manual model definition
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

    fn custom_model_pricing(&self, model_id: &str) -> Option<ModelPricing> {
        self.custom_models
            .get(model_id)
            .and_then(|c| c.pricing.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_provider_cache_control_defaults_false_and_can_enable() {
        let mut provider = CustomProvider::default();

        assert!(!provider.supports_cache_control);
        assert!(!provider.build_runtime().unwrap().supports_cache_control);

        provider
            .provider_settings_apply(serde_yaml::from_str("supports_cache_control: true").unwrap())
            .unwrap();

        assert!(provider.supports_cache_control);
        assert!(provider.build_runtime().unwrap().supports_cache_control);
        assert_eq!(
            provider.provider_settings_as_json()["supports_cache_control"],
            true
        );
    }

    #[test]
    fn custom_provider_extra_headers_replace_preserve_and_remove() {
        let mut provider = CustomProvider::default();
        provider
            .extra_headers
            .insert("X-Secret".to_string(), "old-secret".to_string());
        provider
            .extra_headers
            .insert("X-Replaced".to_string(), "old-value".to_string());
        provider
            .extra_headers
            .insert("X-Remove-Null".to_string(), "old-null".to_string());
        provider
            .extra_headers
            .insert("X-Remove-Number".to_string(), "old-number".to_string());
        provider
            .extra_headers
            .insert("X-Absent".to_string(), "old-absent".to_string());

        provider
            .provider_settings_apply(
                serde_yaml::from_str(
                    r#"
extra_headers:
  X-Secret: "***"
  X-Replaced: new-value
  X-Remove-Null:
  X-Remove-Number: 7
"#,
                )
                .unwrap(),
            )
            .unwrap();

        assert_eq!(
            provider.extra_headers.get("X-Secret").unwrap(),
            "old-secret"
        );
        assert_eq!(
            provider.extra_headers.get("X-Replaced").unwrap(),
            "new-value"
        );
        assert!(!provider.extra_headers.contains_key("X-Remove-Null"));
        assert!(!provider.extra_headers.contains_key("X-Remove-Number"));
        assert!(!provider.extra_headers.contains_key("X-Absent"));

        let settings = provider.provider_settings_as_json();
        assert_eq!(settings["extra_headers"]["X-Secret"], "***");
        assert_eq!(settings["extra_headers"]["X-Replaced"], "***");
        assert!(settings["extra_headers"].get("X-Remove-Null").is_none());
        assert!(settings["extra_headers"].get("X-Remove-Number").is_none());
        assert!(settings["extra_headers"].get("X-Absent").is_none());
    }

    #[test]
    fn custom_provider_extra_headers_empty_map_clears_all() {
        let mut provider = CustomProvider::default();
        provider
            .extra_headers
            .insert("X-Secret".to_string(), "old-secret".to_string());

        provider
            .provider_settings_apply(serde_yaml::from_str("extra_headers: {}").unwrap())
            .unwrap();

        assert!(provider.extra_headers.is_empty());
    }

    #[test]
    fn custom_provider_extra_headers_null_clears_all() {
        let mut provider = CustomProvider::default();
        provider
            .extra_headers
            .insert("X-Secret".to_string(), "old-secret".to_string());

        provider
            .provider_settings_apply(serde_yaml::from_str("extra_headers:").unwrap())
            .unwrap();

        assert!(provider.extra_headers.is_empty());
    }

    #[test]
    fn custom_provider_extra_headers_absent_preserves_existing() {
        let mut provider = CustomProvider::default();
        provider
            .extra_headers
            .insert("X-Secret".to_string(), "old-secret".to_string());

        provider
            .provider_settings_apply(serde_yaml::from_str("enabled: true").unwrap())
            .unwrap();

        assert_eq!(
            provider.extra_headers.get("X-Secret").map(String::as_str),
            Some("old-secret")
        );
    }

    #[test]
    fn custom_provider_extra_headers_yaml_string_parses_and_applies() {
        let mut provider = CustomProvider::default();
        provider
            .extra_headers
            .insert("X-Secret".to_string(), "old-secret".to_string());
        provider
            .extra_headers
            .insert("X-Absent".to_string(), "old-absent".to_string());

        provider
            .provider_settings_apply(
                serde_yaml::from_str(
                    r#"
extra_headers: |
  X-Secret: "***"
  X-New: new-value
  X-Remove-Number: 7
"#,
                )
                .unwrap(),
            )
            .unwrap();

        assert_eq!(
            provider.extra_headers.get("X-Secret").map(String::as_str),
            Some("old-secret")
        );
        assert_eq!(
            provider.extra_headers.get("X-New").map(String::as_str),
            Some("new-value")
        );
        assert!(provider.extra_headers.get("X-Remove-Number").is_none());
        assert!(provider.extra_headers.get("X-Absent").is_none());
    }

    #[test]
    fn custom_provider_extra_headers_json_string_parses_and_applies() {
        let mut provider = CustomProvider::default();
        provider
            .extra_headers
            .insert("X-Secret".to_string(), "old-secret".to_string());

        provider
            .provider_settings_apply(
                serde_yaml::to_value(json!({
                    "extra_headers": "{\"X-Secret\":\"***\",\"X-Json\":\"json-value\",\"X-Remove\":7}"
                }))
                .unwrap(),
            )
            .unwrap();

        assert_eq!(
            provider.extra_headers.get("X-Secret").map(String::as_str),
            Some("old-secret")
        );
        assert_eq!(
            provider.extra_headers.get("X-Json").map(String::as_str),
            Some("json-value")
        );
        assert!(provider.extra_headers.get("X-Remove").is_none());
    }

    #[test]
    fn custom_provider_extra_headers_invalid_string_errors_and_preserves_existing() {
        let mut provider = CustomProvider::default();
        provider
            .extra_headers
            .insert("X-Secret".to_string(), "old-secret".to_string());

        let err = provider
            .provider_settings_apply(serde_yaml::from_str("extra_headers: '['").unwrap())
            .unwrap_err();

        assert!(err.contains("extra_headers"));
        assert_eq!(
            provider.extra_headers.get("X-Secret").map(String::as_str),
            Some("old-secret")
        );
    }

    #[test]
    fn custom_provider_schema_exposes_extra_headers() {
        let schema = CustomProvider::default().provider_schema();
        assert!(schema.contains("extra_headers:"));
        assert!(schema.contains("f_label: \"Extra Headers\""));
    }
}
