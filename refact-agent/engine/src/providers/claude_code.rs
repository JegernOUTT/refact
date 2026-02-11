use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::caps::model_caps::ModelCapabilities;
use crate::llm::adapter::WireFormat;
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelSource, ProviderRuntime, ProviderTrait,
    parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeCodeAuthMethod {
    Auto,
    CliSession,
    OauthToken,
    ApiKey,
}

impl Default for ClaudeCodeAuthMethod {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeCodeProvider {
    pub enabled: bool,
    #[serde(default)]
    pub auth_method: ClaudeCodeAuthMethod,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub oauth_token: String,
    #[serde(default)]
    pub cli_path: Option<String>,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl ClaudeCodeProvider {
    fn detect_cli_path(&self) -> Option<String> {
        if let Some(ref p) = self.cli_path {
            if std::path::Path::new(p).exists() {
                return Some(p.clone());
            }
        }

        if let Ok(output) = std::process::Command::new("which").arg("claude").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(path);
                }
            }
        }

        let candidates = [
            "/usr/local/bin/claude",
            "/opt/homebrew/bin/claude",
        ];
        for c in &candidates {
            if std::path::Path::new(c).exists() {
                return Some(c.to_string());
            }
        }
        if let Some(home) = home::home_dir() {
            let local = home.join(".local/bin/claude");
            if local.exists() {
                return Some(local.to_string_lossy().to_string());
            }
        }
        None
    }

    fn get_cli_oauth_token(&self) -> Result<String, String> {
        let home = home::home_dir()
            .ok_or("Cannot determine home directory")?;

        let creds_path = home.join(".claude/.credentials.json");
        if !creds_path.exists() {
            return Err(format!(
                "Claude CLI credentials not found at {}. Run 'claude auth login' first.",
                creds_path.display()
            ));
        }

        let content = std::fs::read_to_string(&creds_path)
            .map_err(|e| format!("Failed to read {}: {}", creds_path.display(), e))?;

        let creds: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse credentials: {}", e))?;

        creds["claudeAiOauth"]["accessToken"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "Access token not found in credentials file".to_string())
    }

    fn diagnose_auth_status(&self) -> String {
        match self.resolve_auth() {
            Ok((api_key, auth_token)) => {
                if !auth_token.is_empty() {
                    "OK (OAuth token from CLI session)".to_string()
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

    /// Resolved auth credentials. Returns (api_key, auth_token) where:
    /// - api_key is set for standard Anthropic API keys (uses x-api-key header)
    /// - auth_token is set for OAuth tokens from Claude CLI (uses Authorization: Bearer header)
    /// This mirrors the official Anthropic SDK's dual auth support.
    fn resolve_auth(&self) -> Result<(String, String), String> {
        match self.auth_method {
            ClaudeCodeAuthMethod::Auto => {
                // OAuth sources first (use Authorization: Bearer)
                if let Ok(token) = self.get_cli_oauth_token() {
                    tracing::info!("Claude Code: using CLI session OAuth token from credentials file");
                    return Ok((String::new(), token));
                }

                if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
                    if !token.is_empty() && token != "***" {
                        tracing::info!("Claude Code: using CLAUDE_CODE_OAUTH_TOKEN env var");
                        return Ok((String::new(), token));
                    }
                }

                if !self.oauth_token.is_empty() && self.oauth_token != "***" {
                    tracing::info!("Claude Code: using configured OAuth token");
                    return Ok((String::new(), self.oauth_token.clone()));
                }

                // API key sources (use x-api-key)
                if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
                    if !key.is_empty() && key != "***" {
                        tracing::info!("Claude Code: using ANTHROPIC_API_KEY env var");
                        return Ok((key, String::new()));
                    }
                }

                if !self.api_key.is_empty() && self.api_key != "***" {
                    tracing::info!("Claude Code: using configured API key");
                    return Ok((self.api_key.clone(), String::new()));
                }

                Err(concat!(
                    "No authentication method available. Options:\n",
                    "  1. Install Claude CLI and run 'claude auth login'\n",
                    "  2. Set CLAUDE_CODE_OAUTH_TOKEN environment variable\n",
                    "  3. Set ANTHROPIC_API_KEY environment variable\n",
                    "  4. Provide api_key or oauth_token in provider config"
                ).to_string())
            }
            ClaudeCodeAuthMethod::CliSession => {
                let token = self.get_cli_oauth_token()?;
                Ok((String::new(), token))
            }
            ClaudeCodeAuthMethod::OauthToken => {
                if !self.oauth_token.is_empty() && self.oauth_token != "***" {
                    return Ok((String::new(), self.oauth_token.clone()));
                }
                if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
                    if !token.is_empty() && token != "***" {
                        return Ok((String::new(), token));
                    }
                }
                Err("OAuth token not provided. Set oauth_token or CLAUDE_CODE_OAUTH_TOKEN env var.".to_string())
            }
            ClaudeCodeAuthMethod::ApiKey => {
                if !self.api_key.is_empty() && self.api_key != "***" {
                    return Ok((self.api_key.clone(), String::new()));
                }
                if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
                    if !key.is_empty() && key != "***" {
                        return Ok((key, String::new()));
                    }
                }
                Err("API key not provided. Set api_key or ANTHROPIC_API_KEY env var.".to_string())
            }
        }
    }
}

#[async_trait]
impl ProviderTrait for ClaudeCodeProvider {
    fn name(&self) -> &'static str {
        "claude_code"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
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
        WireFormat::AnthropicMessages
    }

    fn model_filter_regex(&self) -> Option<&'static str> {
        Some(r"^claude-")
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  api_key:
    f_type: string_long
    f_desc: "Anthropic API key (only if not using Claude CLI)"
    f_placeholder: "sk-ant-..."
    f_label: "API Key (optional)"
    f_extra: true
  oauth_token:
    f_type: string_long
    f_desc: "OAuth token (only if not using Claude CLI)"
    f_placeholder: "sk-ant-oat01-..."
    f_label: "OAuth Token (optional)"
    f_extra: true
description: |
  Use your Claude Code subscription to access Claude models.

  **Setup:** Install Claude CLI and run `claude auth login` — credentials are detected automatically.
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
        if let Some(cli_path) = yaml.get("cli_path").and_then(|v| v.as_str()) {
            if !cli_path.is_empty() {
                self.cli_path = Some(cli_path.to_string());
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
        let cli_detected = self.detect_cli_path().unwrap_or_default();
        let auth_status = self.diagnose_auth_status();

        json!({
            "enabled": self.enabled,
            "auth_status": auth_status,
            "claude_cli_path": if cli_detected.is_empty() { "not found".to_string() } else { cli_detected },
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "oauth_token": if self.oauth_token.is_empty() { "" } else { "***" },
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let (api_key, auth_token) = match self.resolve_auth() {
            Ok(creds) => creds,
            Err(e) => {
                if self.enabled {
                    tracing::warn!("Claude Code auth failed: {}", e);
                }
                (String::new(), String::new())
            }
        };

        let has_auth = !api_key.is_empty() || !auth_token.is_empty();

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && has_auth,
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: "https://api.anthropic.com/v1/messages".to_string(),
            completion_endpoint: String::new(),
            embedding_endpoint: String::new(),
            api_key,
            auth_token,
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
                tracing::warn!("Claude Code: cannot fetch models, auth failed: {}", e);
                return self.get_custom_models_only();
            }
        };

        let api_model_ids = fetch_claude_code_model_ids(http_client, &api_key, &auth_token).await;
        if api_model_ids.is_empty() {
            tracing::warn!("Claude Code: API returned no models, falling back to custom models only");
            return self.get_custom_models_only();
        }

        tracing::info!("Claude Code: API returned {} models", api_model_ids.len());

        let enabled_set: std::collections::HashSet<_> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();
        let regex_opt = self.model_filter_regex()
            .and_then(|p| regex::Regex::new(p).ok());

        let mut models: Vec<AvailableModel> = Vec::new();

        for api_id in &api_model_ids {
            // Match against model_caps using the same regex filter
            let matches_filter = match &regex_opt {
                Some(regex) => regex.is_match(api_id),
                None => true,
            };
            if !matches_filter {
                continue;
            }
            
            let date_regex = regex::Regex::new(r"^(.+?)-\d{8}$").unwrap();
            let api_id_without_date = date_regex
                .captures(api_id)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| api_id.clone());

            if let Some(caps) = crate::caps::model_caps::resolve_model_caps(model_caps, &api_id_without_date) {
                let enabled = enabled_set.is_empty() || enabled_set.contains(api_id.as_str());
                let pricing = self.model_pricing(api_id);
                let mut model = AvailableModel::from_caps(api_id, &caps.caps, enabled, pricing);
                if api_id != &caps.matched_key {
                    model.display_name = Some(api_id.clone());
                }
                models.push(model);
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
}

const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Fetch available model IDs from the Anthropic API using the provider's credentials.
/// Returns model IDs (e.g., "claude-sonnet-4-20250514") that can be matched against model_caps.
/// Uses OAuth Bearer auth when available, falls back to API key auth.
pub async fn fetch_claude_code_model_ids(
    http_client: &reqwest::Client,
    api_key: &str,
    auth_token: &str,
) -> Vec<String> {
    let mut request = http_client
        .get(ANTHROPIC_MODELS_URL)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json");

    if !auth_token.is_empty() {
        request = request
            .header("Authorization", format!("Bearer {}", auth_token))
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("user-agent", "claude-cli/2.1.2 (external, cli)");
    } else if !api_key.is_empty() {
        request = request.header("x-api-key", api_key);
    } else {
        return vec![];
    }

    match request.send().await {
        Ok(response) => {
            if !response.status().is_success() {
                tracing::warn!(
                    "Claude Code models API returned status {}",
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
                                .collect()
                        })
                        .unwrap_or_default()
                }
                Err(e) => {
                    tracing::warn!("Failed to parse Claude Code models response: {}", e);
                    vec![]
                }
            }
        }
        Err(e) => {
            tracing::warn!("Failed to fetch Claude Code models: {}", e);
            vec![]
        }
    }
}
