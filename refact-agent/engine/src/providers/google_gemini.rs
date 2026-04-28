use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::caps::model_caps::ModelCapabilities;
use crate::llm::adapter::WireFormat;
use crate::providers::config::resolve_env_var;
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};
use crate::providers::pricing::google_gemini_pricing;

const GEMINI_MODELS_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoogleGeminiProvider {
    pub api_key: String,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GoogleGeminiHealthInfo {
    pub ok: bool,
    pub model_count: usize,
}

impl GoogleGeminiProvider {
    fn parse_gemini_model(model: &serde_json::Value, enabled: bool) -> Option<AvailableModel> {
        let name = model.get("name")?.as_str()?;
        let id = name.strip_prefix("models/").unwrap_or(name).to_string();

        let supported_methods = model
            .get("supportedGenerationMethods")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();

        if !supported_methods.contains(&"generateContent") {
            return None;
        }

        let display_name = model
            .get("displayName")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let n_ctx = model
            .get("inputTokenLimit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(128_000);
        let max_output_tokens = model
            .get("outputTokenLimit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let supports_thinking = model
            .get("thinking")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let pricing = google_gemini_pricing(&id);

        Some(AvailableModel {
            id,
            display_name,
            n_ctx,
            supports_tools: true,
            supports_parallel_tools: true,
            supports_strict_tools: false,
            supports_multimodality: true,
            reasoning_effort_options: None,
            supports_thinking_budget: supports_thinking,
            supports_adaptive_thinking_budget: supports_thinking,
            tokenizer: None,
            enabled,
            is_custom: false,
            pricing,
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens,
            provider_variants: Vec::new(),
            base_model: None,
        })
    }

    pub async fn check_api_key_health(
        &self,
        http_client: &reqwest::Client,
    ) -> Result<GoogleGeminiHealthInfo, String> {
        let api_key = resolve_env_var(&self.api_key, "", "google_gemini api_key");
        if api_key.is_empty() {
            return Err("Google Gemini API key is not configured".to_string());
        }

        let url = format!("{}?key={}&pageSize=1", GEMINI_MODELS_URL, api_key);
        let response = http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Google Gemini models request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(format!(
                "Google Gemini API returned status {status}: {detail}"
            ));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse Google Gemini response: {e}"))?;

        let model_count = json
            .get("models")
            .and_then(|v| v.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0);

        Ok(GoogleGeminiHealthInfo {
            ok: true,
            model_count,
        })
    }
}

#[async_trait]
impl ProviderTrait for GoogleGeminiProvider {
    fn name(&self) -> &'static str {
        "google_gemini"
    }

    fn display_name(&self) -> &'static str {
        "Google Gemini"
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
        Some(r"^gemini-")
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  api_key:
    f_type: string_long
    f_desc: "Google AI API key from aistudio.google.com"
    f_placeholder: "AIza..."
    f_label: "API Key"
    smartlinks:
      - sl_label: "Get API Key"
        sl_goto: "https://aistudio.google.com/apikey"
description: |
  Google Gemini models via the OpenAI-compatible API.
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
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        json!({
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "enabled": self.enabled,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let api_key = resolve_env_var(&self.api_key, "", "google_gemini api_key");

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !api_key.is_empty() && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint:
                "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
                    .to_string(),
            completion_endpoint: String::new(),
            embedding_endpoint:
                "https://generativelanguage.googleapis.com/v1beta/openai/embeddings".to_string(),
            api_key,
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: HashMap::new(),
            support_metadata: false,
            supports_cache_control: true,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn has_credentials(&self) -> bool {
        let key = resolve_env_var(&self.api_key, "", "google_gemini api_key");
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

    fn model_pricing(&self, model_id: &str) -> Option<ModelPricing> {
        if let Some(config) = self.custom_models.get(model_id) {
            if config.pricing.is_some() {
                return config.pricing.clone();
            }
        }
        google_gemini_pricing(model_id)
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        _model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let api_key = resolve_env_var(&self.api_key, "", "google_gemini api_key");
        if api_key.is_empty() {
            return self.get_custom_models_only();
        }

        let enabled_set: std::collections::HashSet<&str> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();

        let mut all_models: Vec<AvailableModel> = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut url = format!("{}?key={}&pageSize=1000", GEMINI_MODELS_URL, api_key);
            if let Some(ref token) = page_token {
                url.push_str(&format!("&pageToken={}", token));
            }

            let response = match http_client.get(&url).send().await {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::warn!("Google Gemini: failed to fetch models: {}", e);
                    return self.get_custom_models_only();
                }
            };

            if !response.status().is_success() {
                tracing::warn!(
                    "Google Gemini: models endpoint returned status {}",
                    response.status()
                );
                return self.get_custom_models_only();
            }

            let json: serde_json::Value = match response.json().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("Google Gemini: failed to parse models response: {}", e);
                    return self.get_custom_models_only();
                }
            };

            if let Some(models) = json.get("models").and_then(|v| v.as_array()) {
                for m in models {
                    let model_id = m
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(|name| name.strip_prefix("models/").unwrap_or(name));

                    if let Some(id) = model_id {
                        let enabled = enabled_set.contains(id);
                        if let Some(model) = Self::parse_gemini_model(m, enabled) {
                            all_models.push(model);
                        }
                    }
                }
            }

            page_token = json
                .get("nextPageToken")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if page_token.is_none() {
                break;
            }
        }

        merge_custom_models(&mut all_models, &self.custom_models, &enabled_set);
        all_models.sort_by(|a, b| a.id.cmp(&b.id));
        all_models
    }
}
