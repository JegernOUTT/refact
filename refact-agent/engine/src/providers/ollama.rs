use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::caps::model_caps::ModelCapabilities;
use crate::llm::adapter::WireFormat;
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, normalize_endpoint, derive_endpoint_from_chat_url,
    parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaProvider {
    pub endpoint: String,
    pub api_key: String,
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
            api_key: String::new(),
            enabled: false,
            enabled_models: Vec::new(),
            custom_models: HashMap::new(),
        }
    }
}

impl OllamaProvider {
    fn parse_ollama_model(model: &serde_json::Value, enabled: bool) -> Option<AvailableModel> {
        let name = model.get("name")?.as_str()?.to_string();
        let details = model.get("details");
        let family = details.and_then(|d| d.get("family")).and_then(|v| v.as_str()).unwrap_or("");
        let families: Vec<String> = details
            .and_then(|d| d.get("families"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let parameter_size = details
            .and_then(|d| d.get("parameter_size"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let supports_tools = Self::family_supports_tools(family, &families);
        let supports_multimodality = Self::family_supports_vision(family, &families);

        let display_name = if parameter_size.is_empty() {
            None
        } else {
            Some(format!("{} ({})", name, parameter_size))
        };

        Some(AvailableModel {
            id: name,
            display_name,
            n_ctx: 4096,
            supports_tools,
            supports_multimodality,
            reasoning_effort_options: None,
            supports_thinking_budget: false,
            supports_adaptive_thinking_budget: false,
            tokenizer: None,
            enabled,
            is_custom: false,
            pricing: None,
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens: None,
            provider_variants: Vec::new(),
        })
    }

    fn family_supports_tools(family: &str, families: &[String]) -> bool {
        let tool_families = [
            "llama", "qwen2", "qwen3", "command-r", "mistral",
            "gemma2", "gemma3", "phi3", "phi4", "deepseek2",
        ];
        let check = |f: &str| tool_families.iter().any(|tf| f.contains(tf));
        check(family) || families.iter().any(|f| check(f))
    }

    fn family_supports_vision(family: &str, families: &[String]) -> bool {
        let vision_families = ["llava", "gemma3", "phi4"];
        let check = |f: &str| vision_families.iter().any(|vf| f.contains(vf));
        check(family) || families.iter().any(|f| check(f))
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
  api_key:
    f_type: string_long
    f_desc: "API key (optional, for reverse proxy auth)"
    f_placeholder: ""
    f_label: "API Key"
    f_default: ""
description: |
  Local Ollama server for running open-source models.
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
            support_metadata: false,
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

    fn model_pricing(&self, model_id: &str) -> Option<ModelPricing> {
        self.custom_models.get(model_id).and_then(|c| c.pricing.clone())
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        _model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let base_url = normalize_endpoint(&self.endpoint);
        let tags_url = format!("{}/api/tags", base_url);

        let mut request = http_client
            .get(&tags_url)
            .timeout(std::time::Duration::from_secs(5));
        if !self.api_key.is_empty() {
            request = request.header(reqwest::header::AUTHORIZATION, format!("Bearer {}", self.api_key));
        }

        let response = match request.send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("Ollama: server not reachable at {}: {}", tags_url, e);
                return self.get_custom_models_only();
            }
        };

        if !response.status().is_success() {
            tracing::warn!("Ollama: /api/tags returned status {}", response.status());
            return self.get_custom_models_only();
        }

        let json: serde_json::Value = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Ollama: failed to parse /api/tags response: {}", e);
                return self.get_custom_models_only();
            }
        };

        let enabled_set: std::collections::HashSet<&str> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();

        let mut models: Vec<AvailableModel> = json
            .get("models")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        let name = m.get("name").and_then(|v| v.as_str())?;
                        let enabled = enabled_set.contains(name);
                        Self::parse_ollama_model(m, enabled)
                    })
                    .collect()
            })
            .unwrap_or_default();

        merge_custom_models(&mut models, &self.custom_models, &enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }
}
