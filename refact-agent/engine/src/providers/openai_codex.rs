use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::caps::model_caps::ModelCapabilities;
use crate::llm::adapter::WireFormat;
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};
use crate::providers::pricing::openai_pricing;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OpenAICodexAuthMethod {
    Auto,
    ApiKey,
    OauthToken,
}

impl Default for OpenAICodexAuthMethod {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenAICodexProvider {
    pub enabled: bool,
    #[serde(default)]
    pub auth_method: OpenAICodexAuthMethod,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub oauth_token: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl OpenAICodexProvider {
    fn get_base_url(&self) -> String {
        self.base_url.as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("https://api.openai.com")
            .trim_end_matches('/')
            .to_string()
    }

    fn get_codex_cli_oauth_token(&self) -> Result<String, String> {
        let home = home::home_dir()
            .ok_or("Cannot determine home directory")?;

        // Codex CLI stores credentials in ~/.codex/ directory
        let candidates = [
            home.join(".codex/auth.json"),
            home.join(".config/codex/auth.json"),
        ];

        for creds_path in &candidates {
            if !creds_path.exists() {
                continue;
            }

            let content = std::fs::read_to_string(creds_path)
                .map_err(|e| format!("Failed to read {}: {}", creds_path.display(), e))?;

            let creds: serde_json::Value = serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse credentials at {}: {}", creds_path.display(), e))?;

            // Try common credential fields
            if let Some(token) = creds.get("token")
                .or_else(|| creds.get("access_token"))
                .or_else(|| creds.get("api_key"))
                .and_then(|v| v.as_str())
            {
                if !token.is_empty() {
                    tracing::info!("OpenAI Codex: found credentials at {}", creds_path.display());
                    return Ok(token.to_string());
                }
            }

            // Try nested structure like {"openai": {"accessToken": "..."}}
            if let Some(token) = creds.get("openai")
                .and_then(|v| v.get("accessToken").or_else(|| v.get("access_token")))
                .and_then(|v| v.as_str())
            {
                if !token.is_empty() {
                    tracing::info!("OpenAI Codex: found OAuth credentials at {}", creds_path.display());
                    return Ok(token.to_string());
                }
            }
        }

        Err(format!(
            "Codex CLI credentials not found. Checked: {}",
            candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
        ))
    }

    fn diagnose_auth_status(&self) -> String {
        match self.resolve_auth() {
            Ok((api_key, auth_token)) => {
                if !auth_token.is_empty() {
                    "OK (OAuth token)".to_string()
                } else if !api_key.is_empty() {
                    "OK (API key)".to_string()
                } else {
                    "No credentials found".to_string()
                }
            }
            Err(e) => {
                let first_line = e.lines().next().unwrap_or(&e);
                first_line.to_string()
            }
        }
    }

    /// Resolve auth credentials. Returns (api_key, auth_token) where:
    /// - api_key is set for standard OpenAI API keys (uses Authorization: Bearer header)
    /// - auth_token is set for OAuth tokens from Codex CLI (also uses Authorization: Bearer)
    /// For OpenAI, both use the same header format, but we keep them separate for clarity.
    fn resolve_auth(&self) -> Result<(String, String), String> {
        match self.auth_method {
            OpenAICodexAuthMethod::Auto => {
                // 1. Codex CLI OAuth token (subscription-first, like claude_code)
                if let Ok(token) = self.get_codex_cli_oauth_token() {
                    return Ok((String::new(), token));
                }

                // 2. Environment variables
                if let Ok(key) = std::env::var("CODEX_API_KEY") {
                    if !key.is_empty() && key != "***" {
                        tracing::info!("OpenAI Codex: using CODEX_API_KEY env var");
                        return Ok((key, String::new()));
                    }
                }

                if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                    if !key.is_empty() && key != "***" {
                        tracing::info!("OpenAI Codex: using OPENAI_API_KEY env var");
                        return Ok((key, String::new()));
                    }
                }

                // 3. Explicit config fields
                if !self.oauth_token.is_empty() && self.oauth_token != "***" {
                    tracing::info!("OpenAI Codex: using configured OAuth token");
                    return Ok((String::new(), self.oauth_token.clone()));
                }

                if !self.api_key.is_empty() && self.api_key != "***" {
                    tracing::info!("OpenAI Codex: using configured API key");
                    return Ok((self.api_key.clone(), String::new()));
                }

                Err(concat!(
                    "No authentication method available. Options:\n",
                    "  1. Install Codex CLI and authenticate\n",
                    "  2. Set CODEX_API_KEY or OPENAI_API_KEY environment variable\n",
                    "  3. Provide api_key or oauth_token in provider config"
                ).to_string())
            }
            OpenAICodexAuthMethod::ApiKey => {
                if !self.api_key.is_empty() && self.api_key != "***" {
                    return Ok((self.api_key.clone(), String::new()));
                }
                if let Ok(key) = std::env::var("CODEX_API_KEY") {
                    if !key.is_empty() && key != "***" {
                        return Ok((key, String::new()));
                    }
                }
                if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                    if !key.is_empty() && key != "***" {
                        return Ok((key, String::new()));
                    }
                }
                Err("API key not provided. Set api_key, CODEX_API_KEY, or OPENAI_API_KEY env var.".to_string())
            }
            OpenAICodexAuthMethod::OauthToken => {
                if !self.oauth_token.is_empty() && self.oauth_token != "***" {
                    return Ok((String::new(), self.oauth_token.clone()));
                }
                if let Ok(token) = self.get_codex_cli_oauth_token() {
                    return Ok((String::new(), token));
                }
                Err("OAuth token not provided. Set oauth_token or authenticate via Codex CLI.".to_string())
            }
        }
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
fields:
  api_key:
    f_type: string_long
    f_desc: "OpenAI API key or Codex API key"
    f_placeholder: "sk-..."
    f_label: "API Key"
    smartlinks:
      - sl_label: "Get API Key"
        sl_goto: "https://platform.openai.com/api-keys"
  auth_method:
    f_type: string_short
    f_desc: "Auth method: auto (default), api_key, or oauth_token"
    f_placeholder: "auto"
    f_label: "Auth Method"
    f_extra: true
  oauth_token:
    f_type: string_long
    f_desc: "OAuth token from Codex CLI session (only if not using auto-detection)"
    f_placeholder: ""
    f_label: "OAuth Token (optional)"
    f_extra: true
  base_url:
    f_type: string_long
    f_desc: "Custom base URL for OpenAI-compatible endpoints (not Azure-specific)"
    f_placeholder: "https://api.openai.com"
    f_label: "Base URL (optional)"
    f_extra: true
description: |
  OpenAI Codex models (GPT-5-Codex family) via the Responses API.
  Supports dynamic model discovery and CODEX_API_KEY / OPENAI_API_KEY environment variables.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(enabled) = yaml.get("enabled").and_then(|v| v.as_bool()) {
            self.enabled = enabled;
        }
        if let Some(api_key) = yaml.get("api_key").and_then(|v| v.as_str()) {
            if api_key != "***" {
                self.api_key = api_key.to_string();
            }
        }
        if let Some(oauth_token) = yaml.get("oauth_token").and_then(|v| v.as_str()) {
            if oauth_token != "***" {
                self.oauth_token = oauth_token.to_string();
            }
        }
        if let Some(base_url) = yaml.get("base_url").and_then(|v| v.as_str()) {
            if !base_url.is_empty() {
                self.base_url = Some(base_url.to_string());
            }
        }
        if let Some(auth_method) = yaml.get("auth_method") {
            self.auth_method = serde_yaml::from_value(auth_method.clone())
                .map_err(|e| format!("invalid auth_method: {}", e))?;
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        let auth_status = self.diagnose_auth_status();

        json!({
            "enabled": self.enabled,
            "auth_method": self.auth_method,
            "auth_status": auth_status,
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "oauth_token": if self.oauth_token.is_empty() { "" } else { "***" },
            "base_url": self.base_url.as_deref().unwrap_or(""),
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let (api_key, auth_token) = match self.resolve_auth() {
            Ok(creds) => creds,
            Err(e) => {
                if self.enabled {
                    tracing::warn!("OpenAI Codex auth failed: {}", e);
                }
                (String::new(), String::new())
            }
        };

        // For OpenAI, both api_key and oauth_token use Authorization: Bearer,
        // so we merge them into api_key for the adapter.
        let effective_api_key = if !api_key.is_empty() {
            api_key.clone()
        } else {
            auth_token.clone()
        };

        let has_auth = !effective_api_key.is_empty();
        let base_url = self.get_base_url();

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && has_auth,
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: format!("{}/v1/responses", base_url),
            completion_endpoint: String::new(),
            embedding_endpoint: String::new(),
            api_key: effective_api_key,
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: HashMap::new(),
            support_metadata: false,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
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
        let (api_key, auth_token) = match self.resolve_auth() {
            Ok(creds) => creds,
            Err(e) => {
                tracing::warn!("OpenAI Codex: cannot fetch models, auth failed: {}", e);
                return self.get_custom_models_only();
            }
        };

        let effective_key = if !api_key.is_empty() { &api_key } else { &auth_token };
        let base_url = self.get_base_url();

        let api_model_ids = fetch_openai_codex_model_ids(http_client, &base_url, effective_key).await;
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
        let date_regex = regex::Regex::new(r"^(.+?)-\d{8}$").unwrap();

        for api_id in &api_model_ids {
            let matches_filter = match &regex_opt {
                Some(regex) => regex.is_match(api_id),
                None => true,
            };
            if !matches_filter {
                continue;
            }

            // Strip date suffix for caps matching (e.g., "gpt-5-codex-20250611" → "gpt-5-codex")
            let api_id_without_date = date_regex
                .captures(api_id)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| api_id.clone());

            let enabled = enabled_set.is_empty() || enabled_set.contains(api_id.as_str());
            let pricing = self.model_pricing(api_id);

            if let Some(caps) = crate::caps::model_caps::resolve_model_caps(model_caps, &api_id_without_date) {
                let mut model = AvailableModel::from_caps(api_id, &caps.caps, enabled, pricing);
                if api_id != &caps.matched_key {
                    model.display_name = Some(api_id.clone());
                }
                models.push(model);
            } else {
                // Fallback: include the model with conservative defaults when model_caps
                // doesn't have a matching entry. Codex models are known to support tools
                // and have large context windows.
                tracing::info!("OpenAI Codex: no model_caps match for '{}', using defaults", api_id);
                models.push(AvailableModel {
                    id: api_id.clone(),
                    display_name: None,
                    n_ctx: 200_000,
                    supports_tools: true,
                    supports_multimodality: false,
                    supports_reasoning: Some("openai".to_string()),
                    tokenizer: None,
                    enabled,
                    is_custom: false,
                    pricing,
                });
            }
        }

        // Add custom models
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

/// Fetch available model IDs from the OpenAI API and filter for Codex models.
/// Returns model IDs (e.g., "gpt-5-codex", "codex-mini") that can be matched against model_caps.
pub async fn fetch_openai_codex_model_ids(
    http_client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Vec<String> {
    if api_key.is_empty() {
        return vec![];
    }

    let models_url = format!("{}/v1/models", base_url);
    let codex_filter = regex::Regex::new(r"(?i)(codex)").ok();

    let request = http_client
        .get(&models_url)
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
