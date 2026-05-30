use std::any::Any;
use std::collections::HashMap;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use refact_core::model_caps::ModelCapabilities;
use refact_core::llm_types::WireFormat;
use crate::claude_code_oauth::OAuthTokens;
use crate::traits::{
    AvailableModel, CustomModelConfig, ModelSource, ProviderRuntime, ProviderTrait,
    available_models_from_caps_for_provider, merge_custom_models, parse_custom_models,
    parse_enabled_models, set_model_enabled_impl,
};

const SUPPORTS_CACHE_CONTROL: bool = true;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeCodeProvider {
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
    #[serde(default)]
    pub oauth_tokens: OAuthTokens,
}

impl ClaudeCodeProvider {
    fn needs_refresh_on_start(expires_at: i64) -> bool {
        const REFRESH_BEFORE_EXPIRY_MS: i64 = 5 * 60 * 1000;
        if expires_at == 0 {
            return true;
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        now_ms >= expires_at - REFRESH_BEFORE_EXPIRY_MS
    }

    async fn save_oauth_tokens_config(
        &self,
        config_dir: &std::path::Path,
        instance_id: &str,
    ) -> Result<(), String> {
        let tokens = self.oauth_tokens.clone();
        crate::config_store::update_provider_config(config_dir, instance_id, |existing| {
            let mut yaml_map = match existing {
                Some(value) => value.as_mapping().cloned().ok_or_else(|| {
                    "Config file root is not a YAML mapping. Cannot safely patch.".to_string()
                })?,
                None => serde_yaml::Mapping::new(),
            };

            let mut tokens_map = yaml_map
                .get(&serde_yaml::Value::String("oauth_tokens".to_string()))
                .and_then(|v| v.as_mapping())
                .cloned()
                .unwrap_or_default();

            tokens_map.insert(
                serde_yaml::Value::String("access_token".to_string()),
                serde_yaml::Value::String(tokens.access_token),
            );
            tokens_map.insert(
                serde_yaml::Value::String("refresh_token".to_string()),
                serde_yaml::Value::String(tokens.refresh_token),
            );
            tokens_map.insert(
                serde_yaml::Value::String("expires_at".to_string()),
                serde_yaml::Value::Number(serde_yaml::Number::from(tokens.expires_at)),
            );

            yaml_map.insert(
                serde_yaml::Value::String("oauth_tokens".to_string()),
                serde_yaml::Value::Mapping(tokens_map),
            );

            Ok(serde_yaml::Value::Mapping(yaml_map))
        })
        .await
        .map(|_| ())
    }

    fn diagnose_auth_status(&self) -> String {
        if self.oauth_tokens.access_token.is_empty() {
            return "Not configured — log in via OAuth".to_string();
        }
        if self.oauth_tokens.is_expired() {
            return "OAuth token expired — needs refresh".to_string();
        }
        "OK (OAuth login)".to_string()
    }

    /// Subscription-only auth: returns the in-app OAuth access token for this
    /// provider instance, or an actionable error if not logged in / expired.
    fn resolve_auth(&self) -> Result<String, String> {
        if self.oauth_tokens.access_token.is_empty() {
            return Err("Claude Code: not logged in for this provider instance. \
                Click 'Login with Anthropic' in provider settings."
                .to_string());
        }
        if self.oauth_tokens.is_expired() {
            return Err("Claude Code: OAuth token expired — refresh needed.".to_string());
        }
        Ok(self.oauth_tokens.access_token.clone())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeUsageWindow {
    pub percent_used: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeExtraUsage {
    pub is_enabled: bool,
    pub used_credits: f64,
    pub monthly_limit: Option<f64>,
    pub utilization: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeUsage {
    pub five_hour: Option<ClaudeCodeUsageWindow>,
    pub seven_day: Option<ClaudeCodeUsageWindow>,
    pub extra_usage: Option<ClaudeCodeExtraUsage>,
}

impl ClaudeCodeProvider {
    pub async fn fetch_usage(
        &self,
        http_client: &reqwest::Client,
    ) -> Result<ClaudeCodeUsage, String> {
        let token = self.resolve_auth()?;

        let resp = http_client
            .get("https://api.anthropic.com/api/oauth/usage")
            .header("Authorization", format!("Bearer {}", token))
            .header("anthropic-beta", "oauth-2025-04-20")
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let truncated: String = body.chars().take(512).collect();
            return Err(format!("Usage API returned {}: {}", status, truncated));
        }

        let root: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse usage response: {}", e))?;

        let data = root.get("data").unwrap_or(&root);

        fn as_f64_loose(v: &serde_json::Value) -> Option<f64> {
            v.as_f64().or_else(|| v.as_i64().map(|i| i as f64))
        }

        let parse_window = |key: &str| -> Option<ClaudeCodeUsageWindow> {
            let w = data.get(key)?;
            let percent_used = w
                .get("utilization")
                .and_then(as_f64_loose)
                .or_else(|| w.get("percent_used").and_then(as_f64_loose))?;
            let resets_at = w
                .get("resets_at")
                .or_else(|| w.get("reset_at"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some(ClaudeCodeUsageWindow {
                percent_used,
                resets_at,
            })
        };

        let extra_usage = data.get("extra_usage").and_then(|e| {
            let used_credits = e.get("used_credits").and_then(as_f64_loose).unwrap_or(0.0);
            let is_enabled = e
                .get("is_enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let monthly_limit = e.get("monthly_limit").and_then(as_f64_loose);
            let utilization = e.get("utilization").and_then(as_f64_loose);
            Some(ClaudeCodeExtraUsage {
                is_enabled,
                used_credits,
                monthly_limit,
                utilization,
            })
        });

        Ok(ClaudeCodeUsage {
            five_hour: parse_window("five_hour"),
            seven_day: parse_window("seven_day"),
            extra_usage,
        })
    }
}

#[async_trait]
impl ProviderTrait for ClaudeCodeProvider {
    fn name(&self) -> &str {
        "claude_code"
    }

    fn display_name(&self) -> &str {
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
fields: {}
oauth:
  supported: true
  methods:
    - id: max
      label: "Claude Pro/Max"
      description: "Login with your Claude Pro or Max subscription"
description: |
  Use your Claude Code subscription to access Claude models.

  **Setup:** Click **Login with Anthropic** below. Each provider instance can be logged in to a separate Claude account.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(enabled) = yaml.get("enabled").and_then(|v| v.as_bool()) {
            self.enabled = enabled;
        }
        if let Some(oauth_tokens) = yaml.get("oauth_tokens") {
            self.oauth_tokens = serde_yaml::from_value(oauth_tokens.clone()).unwrap_or_default();
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        let auth_status = self.diagnose_auth_status();
        let oauth_connected = !self.oauth_tokens.access_token.is_empty();

        json!({
            "enabled": self.enabled,
            "auth_status": auth_status,
            "oauth_connected": oauth_connected,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let auth_token = match self.resolve_auth() {
            Ok(token) => token,
            Err(e) => {
                if self.enabled {
                    tracing::warn!("Claude Code auth failed: {}", e);
                }
                String::new()
            }
        };

        let has_auth = !auth_token.is_empty();

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && has_auth && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: "https://api.anthropic.com/v1/messages".to_string(),
            completion_endpoint: String::new(),
            embedding_endpoint: String::new(),
            api_key: String::new(),
            auth_token,
            tokenizer_api_key: String::new(),
            extra_headers: HashMap::new(),
            supports_cache_control: SUPPORTS_CACHE_CONTROL,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn has_credentials(&self) -> bool {
        // Subscription-only: only the per-instance OAuth tokens count.
        !self.oauth_tokens.access_token.is_empty()
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

    fn get_available_models_from_caps(
        &self,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let enabled_set: std::collections::HashSet<_> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();
        let custom_models = self.custom_models();
        let mut models = available_models_from_caps_for_provider(self, model_caps);
        for model in &mut models {
            model.supports_cache_control = SUPPORTS_CACHE_CONTROL;
        }
        merge_custom_models(&mut models, custom_models, &enabled_set);
        for model in &mut models {
            model.supports_cache_control = SUPPORTS_CACHE_CONTROL;
        }
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let fallback_models = || self.get_available_models_from_caps(model_caps);
        let auth_token = match self.resolve_auth() {
            Ok(token) => token,
            Err(e) => {
                tracing::warn!("Claude Code: cannot fetch models, auth failed: {}", e);
                return fallback_models();
            }
        };

        let api_model_ids = match fetch_claude_code_model_ids(http_client, &auth_token).await {
            Ok(models) => models,
            Err(e) => {
                tracing::warn!("Claude Code: cannot fetch models from API: {}", e);
                return fallback_models();
            }
        };

        tracing::info!("Claude Code: API returned {} models", api_model_ids.len());

        let enabled_set: std::collections::HashSet<_> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();
        let regex_opt = self
            .model_filter_regex()
            .and_then(|p| regex::Regex::new(p).ok());

        let date_regex = regex::Regex::new(r"^(.+?)-\d{8}$").expect("valid static regex");
        let mut models: Vec<AvailableModel> = Vec::new();
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

            if let Some(caps) = resolve_claude_code_api_model_caps(model_caps, &api_id_without_date)
            {
                let enabled = enabled_set.contains(api_id.as_str());
                let pricing = self.custom_model_pricing(api_id);
                let mut model = AvailableModel::from_caps(api_id, &caps.caps, enabled, pricing);
                model.supports_cache_control = SUPPORTS_CACHE_CONTROL;
                if api_id != &caps.matched_key {
                    model.display_name = Some(api_id.clone());
                }
                models.push(model);
            } else {
                tracing::warn!(
                    "Claude Code: model '{}' is missing model capabilities metadata; using API defaults",
                    api_id
                );
                let enabled = enabled_set.contains(api_id.as_str());
                models.push(claude_code_api_model_without_caps(api_id, enabled));
            }
        }

        merge_custom_models(&mut models, &self.custom_models, &enabled_set);

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

    fn apply_oauth_refresh_tokens(
        &mut self,
        access_token: &str,
        refresh_token: &str,
        expires_at: i64,
    ) {
        self.oauth_tokens.access_token = access_token.to_string();
        self.oauth_tokens.refresh_token = refresh_token.to_string();
        self.oauth_tokens.expires_at = expires_at;
    }

    async fn startup_refresh_and_sync(
        &mut self,
        http_client: &reqwest::Client,
        config_dir: &std::path::Path,
        instance_id: &str,
    ) -> Result<(), String> {
        if self.oauth_tokens.is_empty() || self.oauth_tokens.refresh_token.is_empty() {
            return Ok(());
        }

        if !Self::needs_refresh_on_start(self.oauth_tokens.expires_at) {
            return Ok(());
        }

        tracing::info!("Claude Code: refreshing OAuth token on startup");
        let refreshed = match crate::claude_code_oauth::refresh_access_token(
            http_client,
            &self.oauth_tokens.refresh_token,
        )
        .await
        {
            Ok(refreshed) => refreshed,
            Err(e) if crate::oauth_refresh::is_permanent_refresh_error(&e) => {
                crate::oauth_refresh::mark_invalid_refresh_token(
                    instance_id,
                    &self.oauth_tokens.refresh_token,
                );
                tracing::warn!(
                    "Claude Code: OAuth refresh token is invalid; clearing saved OAuth tokens. Please log in again: {}",
                    e
                );
                self.oauth_tokens = OAuthTokens::default();
                self.save_oauth_tokens_config(config_dir, instance_id)
                    .await?;
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.oauth_tokens.access_token = refreshed.access_token;
        if !refreshed.refresh_token.is_empty() {
            self.oauth_tokens.refresh_token = refreshed.refresh_token;
        }
        self.oauth_tokens.expires_at = refreshed.expires_at;

        self.save_oauth_tokens_config(config_dir, instance_id).await
    }
}

fn claude_code_api_model_without_caps(model_id: &str, enabled: bool) -> AvailableModel {
    AvailableModel {
        id: model_id.to_string(),
        display_name: None,
        n_ctx: 200000,
        supports_tools: true,
        supports_parallel_tools: true,
        supports_strict_tools: false,
        supports_multimodality: true,
        reasoning_effort_options: None,
        supports_thinking_budget: true,
        supports_adaptive_thinking_budget: false,
        supports_cache_control: SUPPORTS_CACHE_CONTROL,
        tokenizer: Some("claude".to_string()),
        enabled,
        is_custom: false,
        pricing: None,
        available_providers: Vec::new(),
        selected_provider: None,
        max_output_tokens: None,
        provider_variants: Vec::new(),
        wire_format_override: None,
        endpoint_override: None,
        base_model: None,
    }
}

fn resolve_claude_code_api_model_caps(
    model_caps: &HashMap<String, ModelCapabilities>,
    model_id: &str,
) -> Option<refact_core::model_caps::ResolvedCaps> {
    refact_core::model_caps::resolve_model_caps(model_caps, model_id).or_else(|| {
        refact_core::model_caps::resolve_model_caps(model_caps, &format!("anthropic/{model_id}"))
    })
}

const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Fetch available model IDs from the Anthropic API using OAuth credentials.
/// Returns model IDs (e.g., "claude-sonnet-4-20250514") that can be matched against model_caps.
pub async fn fetch_claude_code_model_ids(
    http_client: &reqwest::Client,
    auth_token: &str,
) -> Result<Vec<String>, String> {
    if auth_token.is_empty() {
        return Err("empty auth token".to_string());
    }

    let betas = refact_llm::adapters::claude_code_compat::CC_OAUTH_BETAS.join(",");
    let request = http_client
        .get(ANTHROPIC_MODELS_URL)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {}", auth_token))
        .header("anthropic-beta", betas)
        .header(
            "user-agent",
            refact_llm::adapters::claude_code_compat::USER_AGENT,
        );

    match request.send().await {
        Ok(response) => {
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let truncated: String = body.chars().take(512).collect();
                return Err(format!(
                    "Claude Code models API returned status {}: {}",
                    status, truncated
                ));
            }
            match response.json::<serde_json::Value>().await {
                Ok(json) => json
                    .get("data")
                    .and_then(|d| d.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| {
                                m.get("id").and_then(|id| id.as_str()).map(String::from)
                            })
                            .collect()
                    })
                    .ok_or_else(|| "Claude Code models response missing data array".to_string()),
                Err(e) => Err(format!(
                    "Failed to parse Claude Code models response: {}",
                    e
                )),
            }
        }
        Err(e) => Err(format!("Failed to fetch Claude Code models: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_resolves_real_api_ids_from_models_dev_snapshot() {
        let catalog = refact_core::models_dev::load_models_dev_snapshot_catalog().unwrap();
        let model_caps =
            refact_core::model_caps::model_caps_from_models_dev_catalog(&catalog).unwrap();

        for model_id in [
            "claude-opus-4-7",
            "claude-sonnet-4-6",
            "claude-opus-4-6",
            "claude-opus-4-5-20251101",
            "claude-haiku-4-5-20251001",
            "claude-sonnet-4-5-20250929",
            "claude-opus-4-1-20250805",
            "claude-opus-4-20250514",
            "claude-sonnet-4-20250514",
        ] {
            assert!(
                resolve_claude_code_api_model_caps(&model_caps, model_id).is_some(),
                "models.dev snapshot should resolve Claude Code API id {model_id}"
            );
        }
    }

    #[test]
    fn claude_code_available_models_enable_cache_control_even_when_caps_omit_it() {
        let provider = ClaudeCodeProvider {
            enabled_models: vec!["claude-sonnet-4".to_string()],
            ..Default::default()
        };
        let mut model_caps = HashMap::new();
        model_caps.insert(
            "claude-sonnet-4".to_string(),
            ModelCapabilities {
                n_ctx: 200_000,
                tokenizer: "claude".to_string(),
                ..Default::default()
            },
        );

        let models = provider.get_available_models_from_caps(&model_caps);
        let model = models
            .iter()
            .find(|model| model.id == "claude-sonnet-4")
            .expect("claude code model should be available");

        assert!(model.supports_cache_control);
    }

    #[test]
    fn claude_code_api_model_from_caps_enables_cache_control() {
        let caps = ModelCapabilities {
            n_ctx: 200_000,
            tokenizer: "claude".to_string(),
            ..Default::default()
        };
        let mut model = AvailableModel::from_caps("claude-sonnet-4", &caps, true, None);
        model.supports_cache_control = SUPPORTS_CACHE_CONTROL;

        assert!(model.supports_cache_control);
    }

    #[test]
    fn claude_code_unauthenticated_provider_reports_not_configured() {
        let provider = ClaudeCodeProvider::default();
        assert!(!provider.has_credentials());
        assert!(provider.resolve_auth().is_err());
        assert_eq!(
            provider.diagnose_auth_status(),
            "Not configured — log in via OAuth"
        );
    }

    #[test]
    fn claude_code_logged_in_provider_reports_ok() {
        let provider = ClaudeCodeProvider {
            oauth_tokens: OAuthTokens {
                access_token: "valid".to_string(),
                refresh_token: "refresh".to_string(),
                expires_at: i64::MAX,
            },
            ..Default::default()
        };
        assert!(provider.has_credentials());
        assert_eq!(provider.resolve_auth().unwrap(), "valid");
        assert_eq!(provider.diagnose_auth_status(), "OK (OAuth login)");
    }
}
