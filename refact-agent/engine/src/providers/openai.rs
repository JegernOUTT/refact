use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::caps::model_caps::{resolve_model_caps, ModelCapabilities};
use crate::llm::adapter::WireFormat;
use crate::providers::config::{is_legacy_refact_model, resolve_env_var};
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};

const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";

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

#[async_trait]
impl ProviderTrait for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn display_name(&self) -> &str {
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
            (
                WireFormat::OpenaiResponses,
                "https://api.openai.com/v1/responses".to_string(),
            )
        } else {
            (
                WireFormat::OpenaiChatCompletions,
                "https://api.openai.com/v1/chat/completions".to_string(),
            )
        };

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !api_key.is_empty() && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format,
            chat_endpoint,
            completion_endpoint: "https://api.openai.com/v1/completions".to_string(),
            embedding_endpoint: "https://api.openai.com/v1/embeddings".to_string(),
            api_key,
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: HashMap::new(),
            supports_cache_control: true,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn has_credentials(&self) -> bool {
        let key = resolve_env_var(&self.api_key, "", "openai api_key");
        !key.is_empty()
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
            .and_then(|config| config.pricing.clone())
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let api_key = resolve_env_var(&self.api_key, "", "openai api_key");
        if api_key.is_empty() {
            return self.get_custom_models_only();
        }

        let response = match http_client
            .get(OPENAI_MODELS_URL)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("OpenAI: failed to fetch models: {}", e);
                return self.get_custom_models_only();
            }
        };

        if !response.status().is_success() {
            tracing::warn!(
                "OpenAI: models endpoint returned status {}",
                response.status()
            );
            return self.get_custom_models_only();
        }

        let json: serde_json::Value = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("OpenAI: failed to parse models response: {}", e);
                return self.get_custom_models_only();
            }
        };

        let filter_regex = self
            .model_filter_regex()
            .and_then(|pattern| Regex::new(pattern).ok());

        let enabled_set: std::collections::HashSet<&str> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();

        let mut models_map: HashMap<String, AvailableModel> = HashMap::new();

        if let Some(data) = json.get("data").and_then(|v| v.as_array()) {
            for model in data {
                let id = match model.get("id").and_then(|v| v.as_str()) {
                    Some(id) => id.to_string(),
                    None => continue,
                };

                if is_legacy_refact_model(&id) {
                    continue;
                }
                if !is_openai_chat_model_id(&id) {
                    continue;
                }

                let matches_filter = match &filter_regex {
                    Some(regex) => regex.is_match(&id),
                    None => true,
                };
                if !matches_filter {
                    continue;
                }

                let enabled = enabled_set.contains(id.as_str());
                let resolved_caps = resolve_model_caps(model_caps, &format!("openai/{id}"))
                    .or_else(|| resolve_model_caps(model_caps, &id));
                let pricing = self.custom_model_pricing(&id).or_else(|| {
                    resolved_caps
                        .as_ref()
                        .and_then(|resolved| resolved.caps.pricing.clone())
                });

                if let Some(resolved) = resolved_caps {
                    models_map.insert(
                        id.clone(),
                        AvailableModel::from_caps(&id, &resolved.caps, enabled, pricing),
                    );
                } else {
                    models_map.insert(
                        id.clone(),
                        AvailableModel {
                            id: id.clone(),
                            display_name: None,
                            n_ctx: 128_000,
                            supports_tools: true,
                            supports_parallel_tools: true,
                            supports_strict_tools: false,
                            supports_multimodality: true,
                            reasoning_effort_options: None,
                            supports_thinking_budget: false,
                            supports_adaptive_thinking_budget: false,
                            supports_cache_control: true,
                            tokenizer: None,
                            enabled,
                            is_custom: false,
                            pricing,
                            available_providers: Vec::new(),
                            selected_provider: None,
                            max_output_tokens: None,
                            provider_variants: Vec::new(),
                            wire_format_override: None,
                            endpoint_override: None,
                            base_model: None,
                        },
                    );
                }
            }
        }

        let mut models: Vec<AvailableModel> = models_map.into_values().collect();
        merge_custom_models(&mut models, &self.custom_models, &enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }
}

fn is_openai_chat_model_id(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    if [
        "audio",
        "dall-e",
        "embedding",
        "image",
        "moderation",
        "realtime",
        "speech",
        "transcribe",
        "tts",
        "whisper",
    ]
    .iter()
    .any(|blocked| id.contains(blocked))
    {
        return false;
    }

    id.starts_with("gpt-")
        || id == "o1"
        || id.starts_with("o1-")
        || id == "o3"
        || id.starts_with("o3-")
        || id == "o4"
        || id.starts_with("o4-")
}
