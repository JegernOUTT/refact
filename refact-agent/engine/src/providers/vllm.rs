use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::caps::model_caps::ModelCapabilities;
use crate::llm::adapter::WireFormat;
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, normalize_endpoint, derive_endpoint_from_chat_url, parse_enabled_models,
    parse_custom_models, set_model_enabled_impl,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VLLMProvider {
    pub endpoint: String,
    pub api_key: String,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl Default for VLLMProvider {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:8000".to_string(),
            api_key: String::new(),
            enabled: false,
            enabled_models: Vec::new(),
            custom_models: HashMap::new(),
        }
    }
}

impl VLLMProvider {
    fn parse_openai_model(model: &serde_json::Value, enabled: bool) -> Option<AvailableModel> {
        let id = model.get("id")?.as_str()?.to_string();

        let max_model_len = model
            .get("max_model_len")
            .or_else(|| model.get("context_length"))
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let max_output_tokens = model
            .get("max_tokens")
            .or_else(|| model.get("max_completion_tokens"))
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let supports_tools = model
            .get("supports_tools")
            .and_then(|v| v.as_bool())
            .or_else(|| {
                model
                    .get("supported_parameters")
                    .and_then(|v| v.as_array())
                    .map(|params| {
                        params.iter().any(|p| {
                            matches!(
                                p.as_str(),
                                Some("tools") | Some("tool_choice") | Some("functions")
                            )
                        })
                    })
            })
            .unwrap_or(true);
        let supports_multimodality = model
            .get("supports_vision")
            .and_then(|v| v.as_bool())
            .or_else(|| {
                model
                    .get("supported_parameters")
                    .and_then(|v| v.as_array())
                    .map(|params| {
                        params.iter().any(|p| {
                            matches!(p.as_str(), Some("vision") | Some("image") | Some("images"))
                        })
                    })
            })
            .unwrap_or(false);

        let display_name = model
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != id)
            .map(String::from);
        let root = model
            .get("root")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);

        Some(AvailableModel {
            id,
            display_name,
            n_ctx: max_model_len.unwrap_or(32_768),
            supports_tools,
            supports_parallel_tools: supports_tools,
            supports_strict_tools: false,
            supports_multimodality,
            reasoning_effort_options: None,
            supports_thinking_budget: false,
            supports_adaptive_thinking_budget: false,
            supports_cache_control: false,
            tokenizer: None,
            enabled,
            is_custom: false,
            pricing: None,
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens,
            provider_variants: Vec::new(),
            wire_format_override: None,
            endpoint_override: None,
            base_model: root,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parse_openai_model_keeps_root_only_as_base_model() {
        let model = VLLMProvider::parse_openai_model(
            &json!({
                "id": "served-alias",
                "name": "Served Alias",
                "root": "Qwen/Qwen3.6-27B-FP8"
            }),
            true,
        )
        .unwrap();

        assert_eq!(model.id, "served-alias");
        assert_eq!(model.display_name.as_deref(), Some("Served Alias"));
        assert_eq!(model.base_model.as_deref(), Some("Qwen/Qwen3.6-27B-FP8"));
    }

    #[test]
    fn vllm_runtime_disables_cache_control() {
        let runtime = VLLMProvider::default().build_runtime().unwrap();

        assert!(!runtime.supports_cache_control);
    }
}

#[async_trait]
impl ProviderTrait for VLLMProvider {
    fn name(&self) -> &str {
        "vllm"
    }

    fn display_name(&self) -> &str {
        "vLLM"
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
    f_desc: "vLLM server endpoint"
    f_placeholder: "http://localhost:8000"
    f_label: "Endpoint"
    f_default: "http://localhost:8000"
  api_key:
    f_type: string_long
    f_desc: "API key (optional, depends on vLLM server config)"
    f_placeholder: ""
    f_label: "API Key"
    f_default: ""
description: |
  vLLM inference server for high-throughput model serving.
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
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        json!({
            "endpoint": self.endpoint,
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "enabled": self.enabled,
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
            supports_cache_control: false,
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
        _model_caps: &HashMap<String, ModelCapabilities>,
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
                tracing::warn!("vLLM: server not reachable at {}: {}", models_url, e);
                return self.get_custom_models_only();
            }
        };

        if !response.status().is_success() {
            tracing::warn!("vLLM: /v1/models returned status {}", response.status());
            return self.get_custom_models_only();
        }

        let json: serde_json::Value = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("vLLM: failed to parse /v1/models response: {}", e);
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
                        Self::parse_openai_model(m, enabled)
                    })
                    .collect()
            })
            .unwrap_or_default();

        merge_custom_models(&mut models, &self.custom_models, &enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }
}
