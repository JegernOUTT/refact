use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use refact_core::provider_types::{
    AvailableModel, LiveModelFields, available_model_from_catalog_and_live,
};
use refact_core::model_caps::ModelCapabilities;
use refact_core::llm_types::WireFormat;
use crate::traits::{
    CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, normalize_endpoint, derive_endpoint_from_chat_url, parse_enabled_models,
    parse_custom_models, set_model_enabled_impl,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LMStudioProvider {
    pub endpoint: String,
    pub api_key: String,
    pub enabled: bool,
    #[serde(default)]
    pub supports_cache_control: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl Default for LMStudioProvider {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:1234".to_string(),
            api_key: String::new(),
            enabled: false,
            supports_cache_control: false,
            enabled_models: Vec::new(),
            custom_models: HashMap::new(),
        }
    }
}

impl LMStudioProvider {
    fn catalog_caps<'a>(
        model_caps: &'a HashMap<String, ModelCapabilities>,
        id: &str,
        base_model: Option<&str>,
    ) -> Option<&'a ModelCapabilities> {
        let mut aliases = Vec::new();
        if let Some(base) = base_model {
            aliases.push(base);
        }
        aliases.push(id);
        for alias in aliases {
            let bare = alias.rsplit('/').next().unwrap_or(alias);
            for provider in ["lmstudio", "lm-studio", "lm_studio"] {
                if let Some(caps) = model_caps
                    .get(&format!("{provider}/{alias}"))
                    .or_else(|| model_caps.get(&format!("{provider}/{bare}")))
                {
                    return Some(caps);
                }
            }
            if let Some(caps) = model_caps.get(alias).or_else(|| model_caps.get(bare)) {
                return Some(caps);
            }
        }
        None
    }

    fn parse_openai_model(
        model: &serde_json::Value,
        enabled: bool,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Option<AvailableModel> {
        let id = model.get("id")?.as_str()?;
        let supported_parameters = model
            .get("supported_parameters")
            .and_then(|v| v.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>()
            });
        let parameter_support = |names: &[&str]| {
            supported_parameters
                .as_ref()
                .map(|params| params.iter().any(|p| names.iter().any(|name| p == name)))
        };
        let capability_support = |names: &[&str]| {
            model
                .get("capabilities")
                .and_then(|v| v.as_array())
                .map(|caps| {
                    caps.iter()
                        .any(|c| c.as_str().is_some_and(|c| names.contains(&c)))
                })
        };
        let base_model = model
            .get("base_model")
            .or_else(|| model.get("root"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let live = LiveModelFields {
            display_name: model
                .get("display_name")
                .or_else(|| model.get("name"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != id)
                .map(String::from),
            n_ctx: model
                .get("context_length")
                .or_else(|| model.get("max_context_length"))
                .or_else(|| model.get("max_model_len"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize),
            max_output_tokens: model
                .get("max_output_tokens")
                .or_else(|| model.get("max_tokens"))
                .or_else(|| model.get("max_completion_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize),
            supports_tools: model
                .get("supports_tools")
                .and_then(|v| v.as_bool())
                .or_else(|| parameter_support(&["tools", "tool_choice", "functions"]))
                .or_else(|| capability_support(&["tools"])),
            supports_parallel_tools: model
                .get("supports_parallel_tools")
                .or_else(|| model.get("supports_parallel_tool_calls"))
                .and_then(|v| v.as_bool())
                .or_else(|| parameter_support(&["parallel_tool_calls"])),
            supports_strict_tools: model
                .get("supports_strict_tools")
                .and_then(|v| v.as_bool())
                .or_else(|| parameter_support(&["strict"])),
            supports_multimodality: model
                .get("supports_multimodality")
                .or_else(|| model.get("supports_vision"))
                .and_then(|v| v.as_bool())
                .or_else(|| parameter_support(&["vision", "image", "images"]))
                .or_else(|| capability_support(&["vision", "image"])),
            supported_parameters,
            base_model: base_model.clone(),
            ..Default::default()
        };
        Some(available_model_from_catalog_and_live(
            id,
            Self::catalog_caps(model_caps, id, base_model.as_deref()),
            &live,
            enabled,
            32_768,
        ))
    }
}

#[async_trait]
impl ProviderTrait for LMStudioProvider {
    fn name(&self) -> &str {
        "lmstudio"
    }

    fn display_name(&self) -> &str {
        "LM Studio"
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
    f_desc: "LM Studio server endpoint"
    f_placeholder: "http://localhost:1234"
    f_label: "Endpoint"
    f_default: "http://localhost:1234"
  api_key:
    f_type: string_long
    f_desc: "API key (optional, for reverse proxy auth)"
    f_placeholder: ""
    f_label: "API Key"
    f_default: ""
  supports_cache_control:
    f_type: boolean
    f_desc: "Send Anthropic-style cache-control fields to the LM Studio server"
    f_label: "Enable Cache Control"
    f_default: false
    f_extra: true
description: |
  Local LM Studio server for running models.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(endpoint) = yaml.get("endpoint").and_then(|v| v.as_str()) {
            self.endpoint = normalize_endpoint(endpoint);
        } else if let Some(chat_ep) = yaml.get("chat_endpoint").and_then(|v| v.as_str()) {
            if let Some(derived) = derive_endpoint_from_chat_url(chat_ep) {
                self.endpoint = derived;
            }
        }
        if let Some(api_key) = yaml.get("api_key").and_then(|v| v.as_str()) {
            if api_key != "***" && api_key != "any-will-work" {
                self.api_key = api_key.to_string();
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
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        json!({
            "endpoint": self.endpoint,
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "enabled": self.enabled,
            "supports_cache_control": self.supports_cache_control,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let base_url = normalize_endpoint(&self.endpoint);

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !self.endpoint.is_empty() && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: format!("{}/v1/chat/completions", base_url),
            completion_endpoint: format!("{}/v1/completions", base_url),
            embedding_endpoint: format!("{}/v1/embeddings", base_url),
            api_key: self.api_key.clone(),
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: HashMap::new(),
            supports_cache_control: self.supports_cache_control,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn has_credentials(&self) -> bool {
        !self.endpoint.is_empty()
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::Api
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

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let base_url = normalize_endpoint(&self.endpoint);
        let models_url = format!("{}/v1/models", base_url);

        let mut request = http_client
            .get(&models_url)
            .timeout(std::time::Duration::from_secs(5));
        if !self.api_key.is_empty() {
            request = request.header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.api_key),
            );
        }

        let response = match request.send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("LM Studio: server not reachable at {}: {}", models_url, e);
                return self.get_custom_models_only();
            }
        };

        if !response.status().is_success() {
            tracing::warn!(
                "LM Studio: /v1/models returned status {}",
                response.status()
            );
            return self.get_custom_models_only();
        }

        let json: serde_json::Value = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("LM Studio: failed to parse /v1/models response: {}", e);
                return self.get_custom_models_only();
            }
        };

        let enabled_set: std::collections::HashSet<&str> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();

        let mut models: Vec<AvailableModel> = json
            .get("data")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        let id = m.get("id").and_then(|v| v.as_str())?;
                        let enabled = enabled_set.contains(id);
                        Self::parse_openai_model(m, enabled, model_caps)
                    })
                    .collect()
            })
            .unwrap_or_default();

        merge_custom_models(&mut models, &self.custom_models, &enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_model() -> ModelCapabilities {
        ModelCapabilities {
            n_ctx: 8_192,
            supports_tools: true,
            supports_parallel_tools: true,
            supports_vision: true,
            ..Default::default()
        }
    }

    #[test]
    fn lmstudio_live_fields_override_catalog_but_absent_fields_preserve_it() {
        let caps = HashMap::from([("lmstudio/model-a".to_string(), catalog_model())]);
        let model = LMStudioProvider::parse_openai_model(
            &json!({"id": "model-a", "context_length": 16_384, "supports_tools": false}),
            false,
            &caps,
        )
        .unwrap();

        assert_eq!(model.n_ctx, 16_384);
        assert!(!model.supports_tools);
        assert!(model.supports_multimodality);
    }

    #[test]
    fn lmstudio_server_only_model_is_retained_and_catalog_only_model_is_not_added() {
        let caps = HashMap::from([("lmstudio/catalog-only".to_string(), catalog_model())]);
        let models: Vec<_> = [json!({"id": "server-only"})]
            .iter()
            .filter_map(|m| LMStudioProvider::parse_openai_model(m, false, &caps))
            .collect();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "server-only");
        assert_eq!(models[0].n_ctx, 32_768);
    }

    #[test]
    fn lmstudio_custom_only_addition_is_preserved_for_backwards_compatibility() {
        let mut provider = LMStudioProvider::default();
        provider
            .custom_models
            .insert("custom-only".to_string(), CustomModelConfig::default());

        assert_eq!(provider.get_custom_models_only()[0].id, "custom-only");
    }

    #[test]
    fn lmstudio_matching_custom_fields_override_live_and_catalog_last() {
        let caps = HashMap::from([("lmstudio/model-a".to_string(), catalog_model())]);
        let mut models = vec![LMStudioProvider::parse_openai_model(
            &json!({"id": "model-a", "context_length": 16_384}),
            false,
            &caps,
        )
        .unwrap()];
        let custom = HashMap::from([(
            "model-a".to_string(),
            CustomModelConfig {
                n_ctx: Some(4_096),
                ..Default::default()
            },
        )]);
        merge_custom_models(&mut models, &custom, &std::collections::HashSet::new());

        assert_eq!(models[0].n_ctx, 4_096);
    }

    #[test]
    fn lmstudio_runtime_disables_cache_control_by_default() {
        let runtime = LMStudioProvider::default().build_runtime().unwrap();

        assert!(!runtime.supports_cache_control);
    }

    #[test]
    fn lmstudio_runtime_can_enable_cache_control() {
        let mut provider = LMStudioProvider::default();
        provider
            .provider_settings_apply(serde_yaml::from_str("supports_cache_control: true").unwrap())
            .unwrap();
        let runtime = provider.build_runtime().unwrap();

        assert!(runtime.supports_cache_control);
    }
}
