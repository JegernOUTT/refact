use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::caps::model_caps::{resolve_model_caps, ModelCapabilities, ResolvedCaps};
use crate::llm::adapter::WireFormat;
use crate::providers::config::resolve_env_var;
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};

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
    fn parse_gemini_model(
        model: &serde_json::Value,
        enabled: bool,
        caps: &ModelCapabilities,
        pricing: Option<ModelPricing>,
    ) -> Option<AvailableModel> {
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
        let mut available = AvailableModel::from_caps(&id, caps, enabled, pricing);
        available.display_name = display_name;
        Some(available)
    }

    fn resolve_models_dev_caps(
        model_caps: &HashMap<String, ModelCapabilities>,
        id: &str,
    ) -> Option<ResolvedCaps> {
        resolve_model_caps(model_caps, &format!("google/{id}"))
            .or_else(|| resolve_model_caps(model_caps, id))
    }

    fn models_url(
        api_key: &str,
        page_size: usize,
        page_token: Option<&str>,
    ) -> Result<reqwest::Url, String> {
        let page_size = page_size.to_string();
        let mut params = vec![("key", api_key), ("pageSize", page_size.as_str())];
        if let Some(page_token) = page_token {
            params.push(("pageToken", page_token));
        }
        reqwest::Url::parse_with_params(GEMINI_MODELS_URL, params)
            .map_err(|e| format!("Failed to build Google Gemini models URL: {e}"))
    }

    fn models_request(
        http_client: &reqwest::Client,
        api_key: &str,
        page_size: usize,
        page_token: Option<&str>,
    ) -> Result<reqwest::RequestBuilder, String> {
        Ok(http_client.get(Self::models_url(api_key, page_size, page_token)?))
    }

    pub async fn check_api_key_health(
        &self,
        http_client: &reqwest::Client,
    ) -> Result<GoogleGeminiHealthInfo, String> {
        let api_key = resolve_env_var(&self.api_key, "", "google_gemini api_key");
        if api_key.is_empty() {
            return Err("Google Gemini API key is not configured".to_string());
        }

        let response = Self::models_request(http_client, &api_key, 1, None)?
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
    fn name(&self) -> &str {
        "google_gemini"
    }

    fn display_name(&self) -> &str {
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
        let api_key = resolve_env_var(&self.api_key, "", "google_gemini api_key");
        if api_key.is_empty() {
            return self.get_custom_models_only();
        }

        let enabled_set: std::collections::HashSet<&str> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();

        let mut all_models: Vec<AvailableModel> = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let request =
                match Self::models_request(http_client, &api_key, 1000, page_token.as_deref()) {
                    Ok(request) => request,
                    Err(e) => {
                        tracing::warn!("Google Gemini: failed to build models request: {}", e);
                        return self.get_custom_models_only();
                    }
                };
            let response = match request.send().await {
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
                        let Some(resolved_caps) = Self::resolve_models_dev_caps(model_caps, id)
                        else {
                            continue;
                        };
                        let pricing = self
                            .custom_model_pricing(id)
                            .or_else(|| resolved_caps.caps.pricing.clone());
                        if let Some(model) =
                            Self::parse_gemini_model(m, enabled, &resolved_caps.caps, pricing)
                        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_models_dev_gemini_parse_uses_resolved_caps() {
        let live_model = json!({
            "name": "models/gemini-test",
            "displayName": "Gemini Test",
            "supportedGenerationMethods": ["generateContent"],
            "inputTokenLimit": 999_999,
            "outputTokenLimit": 99_999,
            "thinking": true
        });
        let caps = ModelCapabilities {
            n_ctx: 12_345,
            max_output_tokens: 678,
            supports_tools: false,
            supports_parallel_tools: false,
            supports_strict_tools: false,
            supports_vision: false,
            supports_pdf: false,
            supports_thinking_budget: false,
            supports_adaptive_thinking_budget: false,
            tokenizer: "fake-gemini".to_string(),
            pricing: Some(ModelPricing {
                prompt: 1.0,
                generated: 2.0,
                cache_read: None,
                cache_creation: None,
                context_over_200k: None,
            }),
            ..Default::default()
        };

        let model =
            GoogleGeminiProvider::parse_gemini_model(&live_model, true, &caps, None).unwrap();

        assert_eq!(model.id, "gemini-test");
        assert_eq!(model.n_ctx, 12_345);
        assert_eq!(model.max_output_tokens, Some(678));
        assert!(!model.supports_tools);
        assert!(!model.supports_parallel_tools);
        assert!(!model.supports_strict_tools);
        assert!(!model.supports_multimodality);
        assert!(!model.supports_thinking_budget);
        assert!(!model.supports_adaptive_thinking_budget);
        assert_eq!(model.tokenizer.as_deref(), Some("fake-gemini"));
        assert_eq!(model.pricing.unwrap().prompt, 1.0);
    }

    #[test]
    fn test_models_dev_gemini_parse_uses_multimodality_from_resolved_caps() {
        let live_model = json!({
            "name": "models/gemini-vision-test",
            "supportedGenerationMethods": ["generateContent"]
        });
        let caps = ModelCapabilities {
            n_ctx: 128_000,
            max_output_tokens: 8_192,
            supports_tools: true,
            supports_parallel_tools: true,
            supports_vision: true,
            supports_pdf: true,
            ..Default::default()
        };

        let model =
            GoogleGeminiProvider::parse_gemini_model(&live_model, false, &caps, None).unwrap();

        assert!(model.supports_tools);
        assert!(model.supports_parallel_tools);
        assert!(model.supports_multimodality);
    }

    #[test]
    fn test_models_dev_gemini_unknown_live_models_are_hidden() {
        let empty_caps = HashMap::new();
        assert!(
            GoogleGeminiProvider::resolve_models_dev_caps(&empty_caps, "gemini-uncataloged")
                .is_none()
        );

        let mut model_caps = HashMap::new();
        model_caps.insert(
            "google/gemini-cataloged".to_string(),
            ModelCapabilities {
                n_ctx: 128_000,
                ..Default::default()
            },
        );
        assert!(
            GoogleGeminiProvider::resolve_models_dev_caps(&model_caps, "gemini-cataloged")
                .is_some()
        );
    }

    #[test]
    fn test_models_dev_gemini_request_uses_query_encoding() {
        let url = GoogleGeminiProvider::models_url(
            "key with spaces&symbols",
            1000,
            Some("token/with spaces&symbols"),
        )
        .unwrap();
        let query = url.query().unwrap();

        assert!(query.contains("key=key+with+spaces%26symbols"));
        assert!(query.contains("pageSize=1000"));
        assert!(query.contains("pageToken=token%2Fwith+spaces%26symbols"));
    }
}
