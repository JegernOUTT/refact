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
    ProviderVariant, merge_custom_models, parse_enabled_models, parse_custom_models,
    set_model_enabled_impl,
};

const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const OPENROUTER_KEY_URL: &str = "https://openrouter.ai/api/v1/key";
const OPENROUTER_AUTH_KEY_URL: &str = "https://openrouter.ai/api/v1/auth/key";
const OPENROUTER_CREDITS_URL: &str = "https://openrouter.ai/api/v1/credits";
const OPENROUTER_MODEL_ENDPOINTS_URL: &str = "https://openrouter.ai/api/v1/models";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenRouterProvider {
    pub api_key: String,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
    #[serde(default)]
    pub selected_providers: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenRouterAccountInfo {
    pub key_name: Option<String>,
    pub key_label: Option<String>,
    pub limit: Option<f64>,
    pub usage: Option<f64>,
    pub remaining: Option<f64>,
    pub is_free_tier: Option<bool>,
    pub rate_limit: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenRouterHealthInfo {
    pub ok: bool,
    pub key_label: Option<String>,
    pub key_name: Option<String>,
    pub rate_limit: Option<serde_json::Value>,
}

impl OpenRouterProvider {
    fn parse_price_value(raw: Option<&serde_json::Value>) -> Option<f64> {
        raw.and_then(|v| {
            if let Some(s) = v.as_str() {
                s.parse::<f64>().ok()
            } else {
                v.as_f64()
            }
        })
        .map(|per_token| per_token * 1_000_000.0)
    }

    fn parse_model_pricing(pricing: &serde_json::Value) -> Option<ModelPricing> {
        let prompt = Self::parse_price_value(pricing.get("prompt"))?;
        let generated = Self::parse_price_value(pricing.get("completion"))?;
        let cache_read = Self::parse_price_value(pricing.get("input_cache_read"));
        let cache_creation = Self::parse_price_value(pricing.get("input_cache_write"));

        Some(ModelPricing {
            prompt,
            generated,
            cache_read,
            cache_creation,
            context_over_200k: None,
        })
    }

    fn parse_reasoning_effort_options(endpoint: &serde_json::Value) -> Option<Vec<String>> {
        endpoint
            .get("supported_parameters")
            .and_then(|v| v.as_array())
            .and_then(|params| {
                if params.iter().any(|p| {
                    p.as_str() == Some("reasoning") || p.as_str() == Some("reasoning_effort")
                }) {
                    Some(vec![
                        "low".to_string(),
                        "medium".to_string(),
                        "high".to_string(),
                    ])
                } else {
                    None
                }
            })
    }

    fn parse_openrouter_model(
        model: &serde_json::Value,
        enabled: bool,
        selected_provider: Option<String>,
    ) -> Option<AvailableModel> {
        let id = model.get("id")?.as_str()?.to_string();
        let mut available_providers: Vec<String> = Vec::new();
        let mut provider_variants_map: HashMap<String, ProviderVariant> = HashMap::new();
        let mut selected_pricing: Option<ModelPricing> = None;
        let mut selected_n_ctx: Option<usize> = None;
        let mut selected_max_output: Option<usize> = None;
        let mut supports_tools = false;
        let mut supports_multimodality = false;
        let mut reasoning_effort_options: Option<Vec<String>> = None;

        if let Some(endpoints) = model.get("endpoints").and_then(|v| v.as_array()) {
            for ep in endpoints {
                let provider_tag = ep
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .or_else(|| ep.get("provider_name").and_then(|v| v.as_str()))
                    .unwrap_or_default()
                    .to_string();
                if !provider_tag.is_empty() && !available_providers.contains(&provider_tag) {
                    available_providers.push(provider_tag);
                }

                let pricing = ep.get("pricing").and_then(Self::parse_model_pricing);
                let supported_parameters = ep
                    .get("supported_parameters")
                    .and_then(|v| v.as_array())
                    .map(|params| {
                        params
                            .iter()
                            .filter_map(|p| p.as_str().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                    });

                let variant_id = ep
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .or_else(|| ep.get("provider_name").and_then(|v| v.as_str()))
                    .unwrap_or_default()
                    .to_string();

                if !variant_id.is_empty() {
                    provider_variants_map.insert(
                        variant_id.clone(),
                        ProviderVariant {
                            id: variant_id,
                            name: ep
                                .get("provider_name")
                                .and_then(|v| v.as_str())
                                .map(|v| v.to_string()),
                            tag: ep
                                .get("tag")
                                .and_then(|v| v.as_str())
                                .map(|v| v.to_string()),
                            context_length: ep
                                .get("context_length")
                                .and_then(|v| v.as_u64())
                                .map(|v| v as usize),
                            max_output_tokens: ep
                                .get("max_completion_tokens")
                                .and_then(|v| v.as_u64())
                                .map(|v| v as usize),
                            pricing,
                            latency_last_30m: ep.get("latency_last_30m").and_then(|v| v.as_f64()),
                            throughput_last_30m: ep
                                .get("throughput_last_30m")
                                .and_then(|v| v.as_f64()),
                            uptime_last_30m: ep.get("uptime_last_30m").and_then(|v| v.as_f64()),
                            supported_parameters,
                        },
                    );
                }
            }

            let sanitized_selected = selected_provider
                .as_ref()
                .filter(|provider| available_providers.contains(&provider.to_string()))
                .cloned();

            let selected_endpoint = sanitized_selected
                .as_ref()
                .and_then(|provider| {
                    endpoints.iter().find(|ep| {
                        ep.get("tag").and_then(|v| v.as_str()) == Some(provider.as_str())
                            || ep.get("provider_name").and_then(|v| v.as_str())
                                == Some(provider.as_str())
                    })
                })
                .or_else(|| endpoints.first());

            if let Some(ep) = selected_endpoint {
                selected_pricing = ep.get("pricing").and_then(Self::parse_model_pricing);
                selected_n_ctx = ep
                    .get("context_length")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                selected_max_output = ep
                    .get("max_completion_tokens")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);

                supports_tools = ep
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
                    .unwrap_or(false);

                supports_multimodality = ep
                    .get("supported_parameters")
                    .and_then(|v| v.as_array())
                    .map(|params| {
                        params.iter().any(|p| {
                            matches!(p.as_str(), Some("vision") | Some("image") | Some("images"))
                        })
                    })
                    .unwrap_or(false);

                reasoning_effort_options = Self::parse_reasoning_effort_options(ep);
            }
        }

        let mut provider_variants: Vec<ProviderVariant> =
            provider_variants_map.into_values().collect();
        available_providers.sort();
        provider_variants.sort_by(|a, b| a.id.cmp(&b.id));

        let fallback_pricing = model.get("pricing").and_then(Self::parse_model_pricing);
        let fallback_n_ctx = model
            .get("top_provider")
            .and_then(|tp| tp.get("context_length"))
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .or_else(|| {
                model
                    .get("context_length")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
            });
        let fallback_max_output = model
            .get("top_provider")
            .and_then(|tp| tp.get("max_completion_tokens"))
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let selected_provider = if available_providers.is_empty() {
            selected_provider
        } else {
            selected_provider.filter(|provider| available_providers.contains(provider))
        };

        Some(AvailableModel {
            id,
            display_name: model
                .get("name")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
            n_ctx: selected_n_ctx.or(fallback_n_ctx).unwrap_or(128_000),
            supports_tools,
            supports_parallel_tools: supports_tools,
            supports_strict_tools: false,
            supports_multimodality,
            reasoning_effort_options,
            supports_thinking_budget: false,
            supports_adaptive_thinking_budget: false,
            supports_cache_control: true,
            tokenizer: None,
            enabled,
            is_custom: false,
            pricing: selected_pricing.or(fallback_pricing),
            available_providers,
            selected_provider,
            max_output_tokens: selected_max_output.or(fallback_max_output),
            provider_variants,
            wire_format_override: None,
            endpoint_override: None,
            base_model: None,
        })
    }

    async fn fetch_key_json(
        &self,
        http_client: &reqwest::Client,
        api_key: &str,
    ) -> Result<serde_json::Value, String> {
        let auth_header = format!("Bearer {api_key}");
        let request = http_client
            .get(OPENROUTER_KEY_URL)
            .header(reqwest::header::AUTHORIZATION, auth_header.clone());
        let key_resp = request
            .send()
            .await
            .map_err(|e| format!("OpenRouter key request failed: {e}"))?;

        if key_resp.status().is_success() {
            return key_resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse OpenRouter key response: {e}"));
        }

        let fallback_resp = http_client
            .get(OPENROUTER_AUTH_KEY_URL)
            .header(reqwest::header::AUTHORIZATION, auth_header)
            .send()
            .await
            .map_err(|e| format!("OpenRouter auth/key request failed: {e}"))?;

        if !fallback_resp.status().is_success() {
            return Err(format!(
                "OpenRouter key endpoints returned status {}",
                fallback_resp.status()
            ));
        }

        fallback_resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse OpenRouter auth/key response: {e}"))
    }

    pub fn openrouter_variants_from_endpoints(
        endpoints: &[serde_json::Value],
    ) -> (Vec<ProviderVariant>, Vec<String>) {
        let mut available_providers: Vec<String> = Vec::new();
        let mut provider_variants: Vec<ProviderVariant> = Vec::new();

        for ep in endpoints {
            let provider_id = ep
                .get("tag")
                .and_then(|v| v.as_str())
                .or_else(|| ep.get("provider_name").and_then(|v| v.as_str()))
                .or_else(|| ep.get("name").and_then(|v| v.as_str()))
                .unwrap_or_default()
                .to_string();

            if provider_id.is_empty() {
                continue;
            }

            if !available_providers.contains(&provider_id) {
                available_providers.push(provider_id.clone());
            }

            let pricing = ep.get("pricing").and_then(Self::parse_model_pricing);
            let supported_parameters = ep
                .get("supported_parameters")
                .and_then(|v| v.as_array())
                .map(|params| {
                    params
                        .iter()
                        .filter_map(|p| p.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                });

            provider_variants.push(ProviderVariant {
                id: provider_id.clone(),
                name: ep
                    .get("provider_name")
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string()),
                tag: ep
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string()),
                context_length: ep
                    .get("context_length")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize),
                max_output_tokens: ep
                    .get("max_completion_tokens")
                    .or_else(|| ep.get("max_output_tokens"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize),
                pricing,
                latency_last_30m: ep.get("latency_last_30m").and_then(|v| v.as_f64()),
                throughput_last_30m: ep.get("throughput_last_30m").and_then(|v| v.as_f64()),
                uptime_last_30m: ep.get("uptime_last_30m").and_then(|v| v.as_f64()),
                supported_parameters,
            });
        }

        available_providers.sort();
        provider_variants.sort_by(|a, b| a.id.cmp(&b.id));

        (provider_variants, available_providers)
    }

    pub async fn fetch_model_endpoints(
        &self,
        http_client: &reqwest::Client,
        model_id: &str,
    ) -> Result<(Vec<ProviderVariant>, Vec<String>), String> {
        let api_key = resolve_env_var(&self.api_key, "", "openrouter api_key");
        if api_key.is_empty() {
            return Err("OpenRouter API key is not configured".to_string());
        }

        let mut parts = model_id.splitn(2, '/');
        let author = parts.next().unwrap_or("");
        let slug = parts.next().unwrap_or("");
        if author.is_empty() || slug.is_empty() {
            return Err("OpenRouter model id must be in author/slug format".to_string());
        }

        let url = format!(
            "{}/{}/{}/endpoints",
            OPENROUTER_MODEL_ENDPOINTS_URL, author, slug
        );

        let response = http_client
            .get(url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
            .send()
            .await
            .map_err(|e| format!("OpenRouter endpoints request failed: {e}"))?;

        if !response.status().is_success() {
            return Err(format!(
                "OpenRouter endpoints returned status {}",
                response.status()
            ));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse OpenRouter endpoints response: {e}"))?;

        let endpoints = json
            .get("data")
            .and_then(|d| d.get("endpoints"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| "OpenRouter endpoints response missing data.endpoints".to_string())?;

        Ok(Self::openrouter_variants_from_endpoints(endpoints))
    }

    pub async fn fetch_account_info(
        &self,
        http_client: &reqwest::Client,
    ) -> Result<OpenRouterAccountInfo, String> {
        let api_key = resolve_env_var(&self.api_key, "", "openrouter api_key");
        if api_key.is_empty() {
            return Err("OpenRouter API key is not configured".to_string());
        }

        let key_json = self.fetch_key_json(http_client, &api_key).await?;

        let credits_resp = http_client
            .get(OPENROUTER_CREDITS_URL)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
            .send()
            .await
            .map_err(|e| format!("OpenRouter credits request failed: {e}"))?;
        if !credits_resp.status().is_success() {
            return Err(format!(
                "OpenRouter credits returned status {}",
                credits_resp.status()
            ));
        }
        let credits_json: serde_json::Value = credits_resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse OpenRouter credits response: {e}"))?;

        let key_data = key_json.get("data");
        let credits_data = credits_json.get("data");

        let key_limit = key_data
            .and_then(|d| d.get("limit"))
            .and_then(|v| v.as_f64());
        let key_remaining = key_data
            .and_then(|d| d.get("limit_remaining"))
            .and_then(|v| v.as_f64());
        let key_usage = key_data
            .and_then(|d| d.get("usage"))
            .and_then(|v| v.as_f64());

        let credits_total = credits_data
            .and_then(|d| d.get("total_credits"))
            .and_then(|v| v.as_f64());
        let credits_usage = credits_data
            .and_then(|d| d.get("total_usage"))
            .and_then(|v| v.as_f64());

        let limit = key_limit.or(credits_total);
        let usage = key_usage.or(credits_usage);
        let remaining = key_remaining.or_else(|| match (limit, usage) {
            (Some(lim), Some(used)) => Some((lim - used).max(0.0)),
            _ => None,
        });

        Ok(OpenRouterAccountInfo {
            key_name: key_data
                .and_then(|d| d.get("name"))
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
            key_label: key_data
                .and_then(|d| d.get("label"))
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
            limit,
            usage,
            remaining,
            is_free_tier: key_data
                .and_then(|d| d.get("is_free_tier"))
                .and_then(|v| v.as_bool()),
            rate_limit: key_data.and_then(|d| d.get("rate_limit")).cloned(),
        })
    }

    pub async fn check_api_key_health(
        &self,
        http_client: &reqwest::Client,
    ) -> Result<OpenRouterHealthInfo, String> {
        let api_key = resolve_env_var(&self.api_key, "", "openrouter api_key");
        if api_key.is_empty() {
            return Err("OpenRouter API key is not configured".to_string());
        }

        let key_json = self.fetch_key_json(http_client, &api_key).await?;

        Ok(OpenRouterHealthInfo {
            ok: true,
            key_name: key_json
                .get("data")
                .and_then(|d| d.get("name"))
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
            key_label: key_json
                .get("data")
                .and_then(|d| d.get("label"))
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
            rate_limit: key_json
                .get("data")
                .and_then(|d| d.get("rate_limit"))
                .cloned(),
        })
    }
}

#[async_trait]
impl ProviderTrait for OpenRouterProvider {
    fn name(&self) -> &str {
        "openrouter"
    }

    fn display_name(&self) -> &str {
        "OpenRouter"
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
        None // OpenRouter has many models, use API instead
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  api_key:
    f_type: string_long
    f_desc: "OpenRouter API key from openrouter.ai"
    f_placeholder: "sk-or-..."
    f_label: "API Key"
    smartlinks:
      - sl_label: "Get API Key"
        sl_goto: "https://openrouter.ai/keys"
description: |
  OpenRouter aggregator - access models from multiple providers.
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
        if let Some(selected) = yaml.get("selected_providers").and_then(|v| v.as_mapping()) {
            self.selected_providers.clear();
            for (k, v) in selected {
                if let (Some(model), Some(provider)) = (k.as_str(), v.as_str()) {
                    if !provider.is_empty() {
                        self.selected_providers
                            .insert(model.to_string(), provider.to_string());
                    }
                }
            }
        }
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        json!({
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "enabled": self.enabled,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models,
            "selected_providers": self.selected_providers
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let api_key = resolve_env_var(&self.api_key, "", "openrouter api_key");

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !api_key.is_empty() && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: "https://openrouter.ai/api/v1/chat/completions".to_string(),
            completion_endpoint: String::new(),
            embedding_endpoint: String::new(),
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
        let key = resolve_env_var(&self.api_key, "", "openrouter api_key");
        !key.is_empty()
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::Api // OpenRouter has an API for models
    }

    fn enabled_models(&self) -> &[String] {
        &self.enabled_models
    }

    fn custom_models(&self) -> &HashMap<String, CustomModelConfig> {
        &self.custom_models
    }

    fn selected_providers(&self) -> &HashMap<String, String> {
        &self.selected_providers
    }

    fn set_model_enabled(&mut self, model_id: &str, enabled: bool) {
        set_model_enabled_impl(&mut self.enabled_models, model_id, enabled);
    }

    fn set_selected_provider(&mut self, model_id: &str, provider: Option<String>) {
        if let Some(provider_name) = provider.filter(|p| !p.is_empty()) {
            self.selected_providers
                .insert(model_id.to_string(), provider_name);
        } else {
            self.selected_providers.remove(model_id);
        }
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
        let api_key = resolve_env_var(&self.api_key, "", "openrouter api_key");
        if api_key.is_empty() {
            return self.get_custom_models_only();
        }

        let response = match http_client
            .get(OPENROUTER_MODELS_URL)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("OpenRouter: failed to fetch models: {}", e);
                return self.get_custom_models_only();
            }
        };

        if !response.status().is_success() {
            tracing::warn!(
                "OpenRouter: models endpoint returned status {}",
                response.status()
            );
            return self.get_custom_models_only();
        }

        let json: serde_json::Value = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("OpenRouter: failed to parse models response: {}", e);
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
                        let model_id = m.get("id").and_then(|v| v.as_str())?;
                        let enabled = enabled_set.contains(model_id);
                        let selected_provider = self.selected_providers.get(model_id).cloned();
                        Self::parse_openrouter_model(m, enabled, selected_provider)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        merge_custom_models(&mut models, &self.custom_models, &enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }
}
