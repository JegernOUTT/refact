use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::caps::model_caps::ModelCapabilities;
use crate::llm::adapter::WireFormat;
use crate::providers::openai_codex_oauth::OAuthTokens;
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};
use crate::providers::pricing::openai_pricing;

#[derive(Debug, Clone, Copy, PartialEq)]
enum AuthSource {
    InAppOAuth,
    CodexCli,
    None,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenAICodexProvider {
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
    #[serde(default)]
    pub oauth_tokens: OAuthTokens,
}

impl OpenAICodexProvider {
    fn resolve_auth(&self) -> (AuthSource, String) {
        if self.oauth_tokens.has_valid_access_token() {
            return (AuthSource::InAppOAuth, self.oauth_tokens.access_token.clone());
        }

        if let Ok(cli_tokens) = crate::providers::openai_codex_oauth::read_codex_cli_credentials() {
            if !cli_tokens.access_token.is_empty() {
                return (AuthSource::CodexCli, cli_tokens.access_token);
            }
        }

        (AuthSource::None, String::new())
    }

    fn diagnose_auth_status(&self) -> String {
        if self.oauth_tokens.has_valid_access_token() {
            return "OK (OAuth login)".to_string();
        }
        if !self.oauth_tokens.is_empty() && self.oauth_tokens.has_refresh_token() {
            return "OAuth token expired — needs refresh".to_string();
        }
        if crate::providers::openai_codex_oauth::codex_cli_credentials_exist() {
            return "OK (Codex CLI session)".to_string();
        }
        "No credentials found".to_string()
    }
}

#[async_trait]
impl ProviderTrait for OpenAICodexProvider {
    fn name(&self) -> &'static str {
        "openai_codex"
    }

    fn display_name(&self) -> &'static str {
        "OpenAI Codex"
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
        WireFormat::OpenaiResponses
    }

    fn model_filter_regex(&self) -> Option<&'static str> {
        Some(r"^(gpt-.*codex|codex-)")
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields: {}
oauth:
  supported: true
  methods:
    - id: chatgpt
      label: "ChatGPT Plus/Pro"
      description: "Login with your ChatGPT Plus or Pro subscription"
description: |
  Use your ChatGPT Plus/Pro subscription to access OpenAI Codex models (GPT-5-Codex family).

  **Setup:** Click **Login with OpenAI** below, or install Codex CLI and run `codex login`.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(oauth_tokens) = yaml.get("oauth_tokens") {
            self.oauth_tokens = serde_yaml::from_value(oauth_tokens.clone())
                .unwrap_or_default();
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        let auth_status = self.diagnose_auth_status();
        let oauth_connected = self.oauth_tokens.has_valid_access_token();

        json!({
            "auth_status": auth_status,
            "oauth_connected": oauth_connected,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let (_, auth_token) = self.resolve_auth();
        let has_auth = !auth_token.is_empty();

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: has_auth && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: "https://api.openai.com/v1/responses".to_string(),
            completion_endpoint: String::new(),
            embedding_endpoint: String::new(),
            api_key: auth_token,
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
        if self.oauth_tokens.has_valid_access_token() {
            return true;
        }
        if self.oauth_tokens.has_refresh_token() {
            return true;
        }
        crate::providers::openai_codex_oauth::codex_cli_credentials_exist()
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::ModelCaps
    }

    fn enabled_models(&self) -> &[String] {
        &self.enabled_models
    }

    fn custom_models(&self) -> &HashMap<String, CustomModelConfig> {
        &self.custom_models
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let (_, auth_token) = self.resolve_auth();
        if auth_token.is_empty() {
            tracing::warn!("OpenAI Codex: cannot fetch models, no auth");
            return self.get_custom_models_only();
        }

        let api_model_ids = fetch_openai_codex_model_ids(http_client, &auth_token).await;
        if api_model_ids.is_empty() {
            tracing::warn!("OpenAI Codex: API returned no matching models, falling back to caps-based discovery");
            return self.get_available_models_from_caps(model_caps);
        }

        tracing::info!("OpenAI Codex: API returned {} matching models", api_model_ids.len());

        let enabled_set: std::collections::HashSet<_> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();
        let regex_opt = self.model_filter_regex()
            .and_then(|p| regex::Regex::new(p).ok());

        let mut models: Vec<AvailableModel> = Vec::new();
        let date_regex = regex::Regex::new(r"^(.+?)-\d{8}$").expect("valid static regex");

        for api_id in &api_model_ids {
            let matches_filter = match &regex_opt {
                Some(regex) => regex.is_match(api_id),
                None => true,
            };
            if !matches_filter {
                continue;
            }

            let api_id_without_date = date_regex
                .captures(api_id)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| api_id.clone());

            let enabled = enabled_set.contains(api_id.as_str());
            let pricing = self.model_pricing(api_id);

            if let Some(caps) = crate::caps::model_caps::resolve_model_caps(model_caps, &api_id_without_date) {
                let mut model = AvailableModel::from_caps(api_id, &caps.caps, enabled, pricing);
                if api_id != &caps.matched_key {
                    model.display_name = Some(api_id.clone());
                }
                models.push(model);
            } else {
                tracing::info!("OpenAI Codex: no model_caps match for '{}', using defaults", api_id);
                models.push(AvailableModel {
                    id: api_id.clone(),
                    display_name: None,
                    n_ctx: 200_000,
                    supports_tools: true,
                    supports_multimodality: false,
                    reasoning_effort_options: Some(vec!["low".to_string(), "medium".to_string(), "high".to_string()]),
                    supports_thinking_budget: false,
                    supports_adaptive_thinking_budget: false,
                    tokenizer: None,
                    enabled,
                    is_custom: false,
                    pricing,
                });
            }
        }

        for (id, config) in &self.custom_models {
            let enabled = enabled_set.contains(id.as_str());
            models.push(AvailableModel::from_custom(id, config, enabled));
        }

        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
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
        openai_pricing(model_id)
    }
}

pub async fn fetch_openai_codex_model_ids(
    http_client: &reqwest::Client,
    api_key: &str,
) -> Vec<String> {
    if api_key.is_empty() {
        return vec![];
    }

    let models_url = "https://api.openai.com/v1/models";
    let codex_filter = regex::Regex::new(r"(?i)(codex)").ok();

    let request = http_client
        .get(models_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("content-type", "application/json");

    match request.send().await {
        Ok(response) => {
            if !response.status().is_success() {
                tracing::warn!(
                    "OpenAI Codex models API returned status {}",
                    response.status()
                );
                return vec![];
            }
            match response.json::<serde_json::Value>().await {
                Ok(json) => {
                    json.get("data")
                        .and_then(|d| d.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| {
                                    m.get("id")
                                        .and_then(|id| id.as_str())
                                        .map(String::from)
                                })
                                .filter(|id| {
                                    codex_filter.as_ref()
                                        .map(|re| re.is_match(id))
                                        .unwrap_or(true)
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }
                Err(e) => {
                    tracing::warn!("Failed to parse OpenAI Codex models response: {}", e);
                    vec![]
                }
            }
        }
        Err(e) => {
            tracing::warn!("Failed to fetch OpenAI Codex models: {}", e);
            vec![]
        }
    }
}
