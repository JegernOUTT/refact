use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use refact_core::llm_types::{
    CompletionEndpointStyle, EmbeddingEndpointStyle, EmbeddingModelRecord, WireFormat,
};
use crate::config::resolve_env_var;
use crate::traits::{
    CustomModelConfig, ModelPricing, ModelSource, ProviderModel, ProviderRuntime, ProviderTrait,
    extra_headers_mapping_to_hash_map, parse_custom_models, parse_enabled_models,
    parse_extra_headers_value, set_model_enabled_impl,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomProvider {
    pub api_key: String,
    pub chat_endpoint: String,
    pub completion_endpoint: String,
    pub embedding_endpoint: String,
    #[serde(default)]
    pub completion_endpoint_style: String,
    #[serde(default)]
    pub embedding_endpoint_style: String,
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
    #[serde(default)]
    pub completion_models: HashMap<String, CustomModelConfig>,
    // Full embedding model config (name + operational fields) preserved verbatim
    // so the GUI round-trips size/batch/threshold/prefixes. A legacy bare-string
    // value is preserved as-is for back-compat.
    #[serde(default)]
    pub embedding_model: Option<serde_json::Value>,
}

impl CustomProvider {
    fn has_chat_role(&self) -> bool {
        !self.chat_endpoint.is_empty() && !self.enabled_models.is_empty()
    }

    fn has_completion_role(&self) -> bool {
        !self.completion_endpoint.is_empty() && !self.completion_models.is_empty()
    }

    fn has_embedding_role(&self) -> bool {
        !self.embedding_endpoint.is_empty() && self.embedding_model_name().is_some()
    }

    fn embedding_model_name(&self) -> Option<String> {
        let value = self.embedding_model.as_ref()?;
        let name = match value {
            serde_json::Value::String(name) => name.trim().to_string(),
            serde_json::Value::Object(map) => map
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            _ => String::new(),
        };
        (!name.is_empty()).then_some(name)
    }

    fn role_count(&self) -> usize {
        self.enabled_models.len()
            + self.completion_models.len()
            + usize::from(self.embedding_model.is_some())
    }

    fn runtime_completion_models(&self) -> Vec<ProviderModel> {
        self.completion_models
            .iter()
            .map(|(model_id, config)| custom_model_to_provider_model(model_id, config, true))
            .collect()
    }

    fn runtime_embedding_model(&self) -> Option<ProviderModel> {
        self.embedding_model_name()
            .map(|name| embedding_model_to_provider_model(&name))
    }
}

fn parse_custom_models_from_key(
    yaml: &serde_yaml::Value,
    key: &str,
    target: &mut HashMap<String, CustomModelConfig>,
) -> Result<(), String> {
    let Some(value) = yaml.get(key) else {
        return Ok(());
    };

    target.clear();
    if value.is_null() {
        return Ok(());
    }
    if let Some(models) = value.as_sequence() {
        for model in models.iter().filter_map(|v| v.as_str()) {
            let model = model.trim();
            if !model.is_empty() {
                target.insert(model.to_string(), CustomModelConfig::default());
            }
        }
        return Ok(());
    }
    if let Some(custom) = value.as_mapping() {
        for (model_key, value) in custom {
            if let Some(model_id) = model_key.as_str() {
                let config: CustomModelConfig = serde_yaml::from_value(value.clone())
                    .map_err(|e| format!("invalid {key} entry '{model_id}': {e}"))?;
                target.insert(model_id.to_string(), config);
            }
        }
        return Ok(());
    }

    Err(format!(
        "invalid {key}: expected mapping, sequence, or null"
    ))
}

fn parse_embedding_model_value(
    value: &serde_yaml::Value,
) -> Result<Option<serde_json::Value>, String> {
    if value.is_null() {
        return Ok(None);
    }
    if let Some(name) = value.as_str() {
        let name = name.trim();
        // Preserve the legacy bare-string shape as-is for back-compat.
        return Ok((!name.is_empty()).then(|| serde_json::Value::String(name.to_string())));
    }
    // Validate the object as an embedding record (catches malformed configs) but
    // preserve every field the GUI sent so the round-trip is lossless.
    let record: EmbeddingModelRecord = serde_yaml::from_value(value.clone())
        .map_err(|e| format!("invalid embedding_model: {e}"))?;
    if record.base.name.trim().is_empty() {
        return Ok(None);
    }
    let json = serde_json::to_value(value).map_err(|e| format!("invalid embedding_model: {e}"))?;
    Ok(Some(json))
}

fn custom_model_to_provider_model(
    id: &str,
    config: &CustomModelConfig,
    enabled: bool,
) -> ProviderModel {
    ProviderModel {
        id: id.to_string(),
        base_name: id.to_string(),
        enabled,
        n_ctx: config.n_ctx.unwrap_or(4096),
        supports_tools: config.supports_tools.unwrap_or(false),
        supports_multimodality: config.supports_multimodality.unwrap_or(false),
        supports_reasoning: config
            .reasoning_effort_options
            .as_ref()
            .map(|options| options.join(",")),
        supports_agent: config.supports_tools.unwrap_or(false),
        wire_format_override: None,
        endpoint_override: None,
        user_configured: true,
        removable: true,
    }
}

fn embedding_model_to_provider_model(id: &str) -> ProviderModel {
    ProviderModel {
        id: id.to_string(),
        base_name: id.to_string(),
        enabled: true,
        n_ctx: 8191,
        supports_tools: false,
        supports_multimodality: false,
        supports_reasoning: None,
        supports_agent: false,
        wire_format_override: None,
        endpoint_override: None,
        user_configured: true,
        removable: true,
    }
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
  completion_endpoint_style:
    f_type: string
    f_desc: "Completion API style: openai_completions or openai_chat_completions"
    f_default: "openai_completions"
    f_label: "Completion Endpoint Style"
    f_extra: true
  completion_models:
    f_type: string_long
    f_desc: "Explicit completion model ids for code completion/autocomplete"
    f_label: "Completion Models"
    f_extra: true
  embedding_endpoint:
    f_type: string_long
    f_desc: "Embeddings endpoint URL (optional)"
    f_placeholder: "https://your-server.com/v1/embeddings"
    f_label: "Embedding Endpoint"
    f_extra: true
  embedding_endpoint_style:
    f_type: string
    f_desc: "Embedding API style: openai or ollama_native"
    f_default: "openai"
    f_label: "Embedding Endpoint Style"
    f_extra: true
  embedding_model:
    f_type: string
    f_desc: "Explicit embedding model id for VecDB/RAG"
    f_label: "Embedding Model"
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
        if let Some(style) = yaml
            .get("completion_endpoint_style")
            .and_then(|v| v.as_str())
        {
            if style.trim().is_empty() {
                self.completion_endpoint_style.clear();
            } else {
                CompletionEndpointStyle::from_config(style, "completion_endpoint_style")?;
                self.completion_endpoint_style = style.to_string();
            }
        }
        if let Some(style) = yaml
            .get("embedding_endpoint_style")
            .and_then(|v| v.as_str())
        {
            if style.trim().is_empty() {
                self.embedding_endpoint_style.clear();
            } else {
                EmbeddingEndpointStyle::from_config(style, "embedding_endpoint_style")?;
                self.embedding_endpoint_style = style.to_string();
            }
        }
        if yaml.get("completion_models").is_some() {
            parse_custom_models_from_key(&yaml, "completion_models", &mut self.completion_models)?;
        }
        if let Some(model) = yaml.get("embedding_model") {
            self.embedding_model = parse_embedding_model_value(model)?;
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
            "completion_endpoint_style": self.completion_endpoint_style,
            "embedding_endpoint_style": self.embedding_endpoint_style,
            "wire_format": self.wire_format,
            "enabled": self.enabled,
            "supports_cache_control": self.supports_cache_control,
            "extra_headers": redacted_headers,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models,
            "completion_models": self.completion_models,
            "embedding_model": self.embedding_model
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let api_key = resolve_env_var(&self.api_key, "", "custom api_key");
        if !self.completion_endpoint_style.is_empty() {
            CompletionEndpointStyle::from_config(
                &self.completion_endpoint_style,
                "completion_endpoint_style",
            )?;
        }
        if !self.embedding_endpoint_style.is_empty() {
            EmbeddingEndpointStyle::from_config(
                &self.embedding_endpoint_style,
                "embedding_endpoint_style",
            )?;
        }
        let configured =
            self.has_chat_role() || self.has_completion_role() || self.has_embedding_role();

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && configured,
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: self.chat_endpoint.clone(),
            completion_endpoint: self.completion_endpoint.clone(),
            embedding_endpoint: self.embedding_endpoint.clone(),
            completion_endpoint_style: self.completion_endpoint_style.clone(),
            embedding_endpoint_style: self.embedding_endpoint_style.clone(),
            api_key,
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: self.extra_headers.clone(),
            supports_cache_control: self.supports_cache_control,
            chat_models: Vec::new(),
            completion_models: self.runtime_completion_models(),
            embedding_model: self.runtime_embedding_model(),
        })
    }

    fn has_credentials(&self) -> bool {
        !self.chat_endpoint.is_empty()
            || !self.completion_endpoint.is_empty()
            || !self.embedding_endpoint.is_empty()
    }

    fn selected_model_count(&self) -> usize {
        self.role_count()
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

    #[test]
    fn completion_only_custom_provider_runtime_is_enabled() {
        let provider = CustomProvider {
            enabled: true,
            completion_endpoint: "https://example.com/v1/completions".to_string(),
            completion_endpoint_style: "openai_completions".to_string(),
            completion_models: HashMap::from([(
                "qwen-coder".to_string(),
                CustomModelConfig::default(),
            )]),
            ..Default::default()
        };

        let runtime = provider.build_runtime().unwrap();

        assert!(runtime.enabled);
        assert!(provider.has_credentials());
        assert_eq!(provider.selected_model_count(), 1);
        assert_eq!(runtime.chat_endpoint, "");
        assert_eq!(
            runtime.completion_endpoint,
            "https://example.com/v1/completions"
        );
        assert_eq!(runtime.completion_endpoint_style, "openai_completions");
        assert_eq!(runtime.completion_models.len(), 1);
        assert_eq!(runtime.completion_models[0].id, "qwen-coder");
        assert!(runtime.embedding_model.is_none());
    }

    #[test]
    fn embedding_only_custom_provider_runtime_is_enabled() {
        let provider = CustomProvider {
            enabled: true,
            embedding_endpoint: "https://example.com/v1/embeddings".to_string(),
            embedding_endpoint_style: "openai".to_string(),
            embedding_model: Some(serde_json::json!({ "name": "text-embedding-3-small" })),
            ..Default::default()
        };

        let runtime = provider.build_runtime().unwrap();

        assert!(runtime.enabled);
        assert!(provider.has_credentials());
        assert_eq!(provider.selected_model_count(), 1);
        assert_eq!(runtime.chat_endpoint, "");
        assert_eq!(
            runtime.embedding_endpoint,
            "https://example.com/v1/embeddings"
        );
        assert_eq!(runtime.embedding_endpoint_style, "openai");
        assert!(runtime.completion_models.is_empty());
        let embedding_model = runtime.embedding_model.unwrap();
        assert_eq!(embedding_model.id, "text-embedding-3-small");
    }

    #[test]
    fn mixed_role_custom_provider_preserves_all_roles() {
        let provider = CustomProvider {
            enabled: true,
            chat_endpoint: "https://example.com/v1/chat/completions".to_string(),
            enabled_models: vec!["chat-model".to_string()],
            completion_endpoint: "https://example.com/v1/completions".to_string(),
            completion_endpoint_style: "openai_chat_completions".to_string(),
            completion_models: HashMap::from([(
                "completion-model".to_string(),
                CustomModelConfig::default(),
            )]),
            embedding_endpoint: "https://example.com/v1/embeddings".to_string(),
            embedding_endpoint_style: "ollama_native".to_string(),
            embedding_model: Some(serde_json::json!({ "name": "embedding-model" })),
            ..Default::default()
        };

        let runtime = provider.build_runtime().unwrap();

        assert!(runtime.enabled);
        assert_eq!(provider.enabled_models(), &["chat-model".to_string()]);
        assert_eq!(provider.selected_model_count(), 3);
        assert_eq!(runtime.completion_models[0].id, "completion-model");
        assert_eq!(runtime.embedding_model.unwrap().id, "embedding-model");
        assert_eq!(runtime.completion_endpoint_style, "openai_chat_completions");
        assert_eq!(runtime.embedding_endpoint_style, "ollama_native");
    }

    #[test]
    fn custom_provider_settings_roundtrip_role_sections_and_redacts_secrets() {
        let mut provider = CustomProvider::default();
        provider
            .provider_settings_apply(
                serde_yaml::from_str(
                    r#"
api_key: sk-secret
enabled: true
completion_endpoint: https://example.com/v1/completions
completion_endpoint_style: openai_chat_completions
completion_models:
  qwen-coder:
    n_ctx: 4096
embedding_endpoint: https://example.com/v1/embeddings
embedding_endpoint_style: openai
embedding_model: text-embedding-3-small
extra_headers:
  Authorization: bearer-secret
"#,
                )
                .unwrap(),
            )
            .unwrap();

        let settings = provider.provider_settings_as_json();

        assert_eq!(settings["api_key"], "***");
        assert_eq!(
            settings["completion_endpoint"],
            "https://example.com/v1/completions"
        );
        assert_eq!(
            settings["completion_endpoint_style"],
            "openai_chat_completions"
        );
        assert_eq!(settings["completion_models"]["qwen-coder"]["n_ctx"], 4096);
        assert_eq!(
            settings["embedding_endpoint"],
            "https://example.com/v1/embeddings"
        );
        assert_eq!(settings["embedding_endpoint_style"], "openai");
        assert_eq!(settings["embedding_model"], "text-embedding-3-small");
        assert_eq!(settings["extra_headers"]["Authorization"], "***");
    }

    #[test]
    fn custom_provider_embedding_object_roundtrips_all_fields() {
        let mut provider = CustomProvider::default();
        provider
            .provider_settings_apply(
                serde_yaml::from_str(
                    r#"
embedding_endpoint: https://example.com/v1/embeddings
embedding_endpoint_style: openai
embedding_model:
  name: nomic-embed-text
  n_ctx: 2048
  tokenizer: hf://Nomic/tokenizer
  embedding_size: 768
  embedding_batch: 16
  rejection_threshold: 0.42
  dimensions: 512
  query_prefix: "query: "
  document_prefix: "passage: "
"#,
                )
                .unwrap(),
            )
            .unwrap();

        let settings = provider.provider_settings_as_json();
        let embedding = &settings["embedding_model"];
        assert_eq!(embedding["name"], "nomic-embed-text");
        assert_eq!(embedding["embedding_size"], 768);
        assert_eq!(embedding["embedding_batch"], 16);
        assert_eq!(embedding["rejection_threshold"].as_f64(), Some(0.42));
        assert_eq!(embedding["dimensions"], 512);
        assert_eq!(embedding["query_prefix"], "query: ");
        assert_eq!(embedding["document_prefix"], "passage: ");

        // Runtime still derives the model id from the embedded name.
        let runtime = provider.build_runtime().unwrap();
        assert_eq!(runtime.embedding_model.unwrap().id, "nomic-embed-text");
    }

    #[test]
    fn custom_provider_embedding_legacy_string_is_accepted() {
        let mut provider = CustomProvider::default();
        provider
            .provider_settings_apply(
                serde_yaml::from_str(
                    r#"
embedding_endpoint: https://example.com/v1/embeddings
embedding_model: legacy-embed
"#,
                )
                .unwrap(),
            )
            .unwrap();

        // Legacy bare-string is preserved as-is for back-compat.
        let settings = provider.provider_settings_as_json();
        assert_eq!(settings["embedding_model"], "legacy-embed");
        assert_eq!(
            provider
                .build_runtime()
                .unwrap()
                .embedding_model
                .unwrap()
                .id,
            "legacy-embed"
        );
    }

    #[test]
    fn existing_chat_custom_provider_runtime_behavior_still_enabled_by_chat_role() {
        let provider = CustomProvider {
            enabled: true,
            chat_endpoint: "https://example.com/v1/chat/completions".to_string(),
            enabled_models: vec!["chat-model".to_string()],
            ..Default::default()
        };

        let runtime = provider.build_runtime().unwrap();

        assert!(runtime.enabled);
        assert!(runtime.completion_models.is_empty());
        assert!(runtime.embedding_model.is_none());
        assert_eq!(provider.selected_model_count(), 1);
    }
}
