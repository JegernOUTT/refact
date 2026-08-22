use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::{Mutex as AMutex, MutexGuard};

use refact_core::provider_types::{available_model_from_catalog_and_live, LiveModelFields};
use refact_core::model_caps::ModelCapabilities;
use refact_core::llm_types::WireFormat;
use crate::claude_code_oauth::OAuthTokens;
use crate::traits::{
    AvailableModel, CustomModelConfig, ModelSource, ProviderRuntime, ProviderTrait,
    available_models_from_caps_for_provider, merge_custom_models, parse_custom_models,
    parse_enabled_models, set_model_enabled_impl,
};

const SUPPORTS_CACHE_CONTROL: bool = true;
pub const CLAUDE_CODE_OAUTH_EXPIRES_AT_HEADER: &str =
    "x-refact-internal-claude-code-oauth-expires-at";

lazy_static::lazy_static! {
    static ref CLAUDE_CODE_REFRESH_GUARD: AMutex<()> = AMutex::new(());
}

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
    pub async fn lock_refresh_guard() -> Result<MutexGuard<'static, ()>, String> {
        tokio::time::timeout(Duration::from_secs(30), CLAUDE_CODE_REFRESH_GUARD.lock())
            .await
            .map_err(|_| "Claude Code OAuth refresh guard timed out".to_string())
    }

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
    pub fn resolve_auth(&self) -> Result<String, String> {
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

    pub fn should_force_refresh_for_status(
        status: reqwest::StatusCode,
        refresh_token: &str,
        already_attempted: bool,
    ) -> bool {
        !already_attempted
            && !refresh_token.is_empty()
            && matches!(
                status,
                reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
            )
    }

    pub fn access_token_changed_since_rejection(
        &self,
        rejected_access_token: &str,
    ) -> Option<String> {
        (!self.oauth_tokens.access_token.is_empty()
            && self.oauth_tokens.access_token != rejected_access_token)
            .then(|| self.oauth_tokens.access_token.clone())
    }

    pub fn auth_state_matches(&self, tokens: &OAuthTokens) -> bool {
        &self.oauth_tokens == tokens
    }

    pub fn update_auth_state_from_if_current(
        &mut self,
        source: &ClaudeCodeProvider,
        previous_tokens: &OAuthTokens,
    ) -> bool {
        if source.auth_state_matches(previous_tokens) || !self.auth_state_matches(previous_tokens) {
            return false;
        }
        self.oauth_tokens = source.oauth_tokens.clone();
        true
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeUsageWindow {
    pub percent_used: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeScopedUsageWindow {
    pub label: String,
    pub model_id: Option<String>,
    pub window: ClaudeCodeUsageWindow,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeExtraUsage {
    pub is_enabled: bool,
    pub used_credits: Option<f64>,
    pub monthly_limit: Option<f64>,
    pub utilization: Option<f64>,
    pub currency: Option<String>,
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeUsage {
    pub five_hour: Option<ClaudeCodeUsageWindow>,
    pub seven_day: Option<ClaudeCodeUsageWindow>,
    pub seven_day_sonnet: Option<ClaudeCodeUsageWindow>,
    pub seven_day_oauth_apps: Option<ClaudeCodeUsageWindow>,
    pub seven_day_opus: Option<ClaudeCodeUsageWindow>,
    pub seven_day_cowork: Option<ClaudeCodeUsageWindow>,
    pub seven_day_omelette: Option<ClaudeCodeUsageWindow>,
    pub scoped_windows: Vec<ClaudeCodeScopedUsageWindow>,
    pub extra_usage: Option<ClaudeCodeExtraUsage>,
    pub cinder_cove: Option<Value>,
    pub iguana_necktie: Option<Value>,
    pub omelette_promotional: Option<Value>,
    pub tangelo: Option<Value>,
    pub raw_extra: Map<String, Value>,
}

pub enum ClaudeCodeUsageRequestError {
    Status(reqwest::StatusCode, String),
    Other(String),
}

impl ClaudeCodeProvider {
    pub async fn fetch_usage(
        &self,
        http_client: &reqwest::Client,
    ) -> Result<ClaudeCodeUsage, String> {
        let token = self.resolve_auth()?;
        self.fetch_usage_once(http_client, &token)
            .await
            .map_err(Self::usage_request_error_to_string)
    }

    pub async fn fetch_usage_once(
        &self,
        http_client: &reqwest::Client,
        access_token: &str,
    ) -> Result<ClaudeCodeUsage, ClaudeCodeUsageRequestError> {
        let resp = http_client
            .get("https://api.anthropic.com/api/oauth/usage")
            .header("Authorization", format!("Bearer {}", access_token))
            .header("anthropic-beta", "oauth-2025-04-20")
            .send()
            .await
            .map_err(|error| {
                ClaudeCodeUsageRequestError::Other(format!("Request failed: {}", error))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let truncated: String = body.chars().take(512).collect();
            return Err(ClaudeCodeUsageRequestError::Status(status, truncated));
        }

        let root: serde_json::Value = resp.json().await.map_err(|error| {
            ClaudeCodeUsageRequestError::Other(format!("Failed to parse usage response: {}", error))
        })?;

        Ok(Self::parse_usage_payload(&root))
    }

    pub fn usage_request_error_to_string(error: ClaudeCodeUsageRequestError) -> String {
        match error {
            ClaudeCodeUsageRequestError::Status(status, body) => {
                format!("Usage API returned {}: {}", status, body)
            }
            ClaudeCodeUsageRequestError::Other(message) => message,
        }
    }

    fn parse_usage_payload(root: &Value) -> ClaudeCodeUsage {
        let data = root.get("data").unwrap_or(root);
        let raw_extra = collect_raw_extra(
            data,
            &[
                "five_hour",
                "seven_day",
                "seven_day_sonnet",
                "seven_day_oauth_apps",
                "seven_day_opus",
                "seven_day_cowork",
                "seven_day_omelette",
                "fable_weekly",
                "fable_seven_day",
                "seven_day_fable",
                "extra_usage",
                "cinder_cove",
                "iguana_necktie",
                "omelette_promotional",
                "tangelo",
            ],
        );

        ClaudeCodeUsage {
            five_hour: parse_claude_usage_window(data, "five_hour"),
            seven_day: parse_claude_usage_window(data, "seven_day"),
            seven_day_sonnet: parse_claude_usage_window(data, "seven_day_sonnet"),
            seven_day_oauth_apps: parse_claude_usage_window(data, "seven_day_oauth_apps"),
            seven_day_opus: parse_claude_usage_window(data, "seven_day_opus"),
            seven_day_cowork: parse_claude_usage_window(data, "seven_day_cowork"),
            seven_day_omelette: parse_claude_usage_window(data, "seven_day_omelette"),
            scoped_windows: parse_claude_scoped_usage_windows(data),
            extra_usage: data.get("extra_usage").and_then(parse_claude_extra_usage),
            cinder_cove: data.get("cinder_cove").cloned(),
            iguana_necktie: data.get("iguana_necktie").cloned(),
            omelette_promotional: data.get("omelette_promotional").cloned(),
            tangelo: data.get("tangelo").cloned(),
            raw_extra,
        }
    }
}

fn collect_raw_extra(data: &Value, known_keys: &[&str]) -> Map<String, Value> {
    let Some(obj) = data.as_object() else {
        return Map::new();
    };
    obj.iter()
        .filter(|(key, _)| !known_keys.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn as_f64_loose(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|i| i as f64))
        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
}

fn parse_claude_usage_window(data: &Value, key: &str) -> Option<ClaudeCodeUsageWindow> {
    let w = data.get(key)?.as_object()?;
    let percent_used = w
        .get("utilization")
        .and_then(as_f64_loose)
        .or_else(|| w.get("percent_used").and_then(as_f64_loose))
        .or_else(|| w.get("used_percentage").and_then(as_f64_loose))?;
    if !percent_used.is_finite() {
        return None;
    }
    let resets_at =
        parse_claude_reset_at_from_object(w, &["resets_at", "resetsAt", "reset_at", "resetAt"]);
    Some(ClaudeCodeUsageWindow {
        percent_used,
        resets_at,
    })
}

fn normalize_claude_limit_token(value: Option<&Value>) -> Option<String> {
    let raw = value?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }

    let mut normalized = String::with_capacity(raw.len());
    let mut previous_was_separator = true;
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() {
            if character.is_ascii_uppercase()
                && !previous_was_separator
                && !normalized.ends_with('_')
            {
                normalized.push('_');
            }
            normalized.push(character.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            normalized.push('_');
            previous_was_separator = true;
        }
    }

    let normalized = normalized.trim_matches('_').to_string();
    (!normalized.is_empty()).then_some(normalized)
}

fn parse_non_empty_string(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        obj.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn parse_claude_reset_at(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(raw) = value.as_str() {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        if let Ok(timestamp) = raw.parse::<f64>() {
            return format_claude_reset_timestamp(timestamp);
        }
        return Some(raw.to_string());
    }
    as_f64_loose(value).and_then(format_claude_reset_timestamp)
}

fn parse_claude_reset_at_from_object(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| obj.get(*key))
        .find_map(|value| parse_claude_reset_at(Some(value)))
}

fn format_claude_reset_timestamp(timestamp: f64) -> Option<String> {
    if !timestamp.is_finite() {
        return None;
    }
    let millis = if timestamp.abs() < 10_000_000_000.0 {
        timestamp * 1_000.0
    } else {
        timestamp
    };
    if millis < i64::MIN as f64 || millis > i64::MAX as f64 {
        return None;
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis.round() as i64)
        .map(|date| date.to_rfc3339())
}

fn parse_claude_scoped_usage_windows(data: &Value) -> Vec<ClaudeCodeScopedUsageWindow> {
    let mut windows = Vec::new();
    let mut seen = HashSet::new();

    if let Some(limits) = data.get("limits").and_then(Value::as_array) {
        for limit in limits {
            let Some(limit) = limit.as_object() else {
                continue;
            };
            let kind = normalize_claude_limit_token(limit.get("kind"));
            let group = normalize_claude_limit_token(limit.get("group"));
            let is_weekly_scoped = matches!(
                kind.as_deref(),
                Some("weekly_scoped" | "weekly_model_scoped")
            ) || (kind.as_deref() == Some("model_scoped")
                && group.as_deref() == Some("weekly"));
            if !is_weekly_scoped
                || group.as_deref().is_some_and(|group| group != "weekly")
                || limit
                    .get("is_active")
                    .or_else(|| limit.get("isActive"))
                    .and_then(Value::as_bool)
                    == Some(false)
            {
                continue;
            }

            let Some(model) = limit
                .get("scope")
                .and_then(Value::as_object)
                .and_then(|scope| scope.get("model"))
                .and_then(Value::as_object)
            else {
                continue;
            };
            let Some(label) = parse_non_empty_string(model, &["display_name", "displayName"])
            else {
                continue;
            };
            let Some(percent_used) = limit.get("percent").and_then(as_f64_loose) else {
                continue;
            };
            if !percent_used.is_finite() {
                continue;
            }
            let model_id = parse_non_empty_string(model, &["id", "model_id", "modelId"]);
            let identity = model_id.as_deref().unwrap_or(&label).trim().to_lowercase();
            if !seen.insert(identity) {
                continue;
            }

            windows.push(ClaudeCodeScopedUsageWindow {
                label,
                model_id,
                window: ClaudeCodeUsageWindow {
                    percent_used,
                    resets_at: parse_claude_reset_at_from_object(
                        limit,
                        &["resets_at", "resetsAt", "reset_at", "resetAt"],
                    ),
                },
            });
        }
    }

    if !windows.iter().any(|window| {
        window.label.to_lowercase().starts_with("fable")
            || window
                .model_id
                .as_deref()
                .is_some_and(|model_id| model_id.to_lowercase().contains("fable"))
    }) {
        let legacy_window = ["fable_weekly", "fable_seven_day", "seven_day_fable"]
            .iter()
            .find_map(|key| parse_claude_usage_window(data, key));
        if let Some(window) = legacy_window {
            windows.push(ClaudeCodeScopedUsageWindow {
                label: "Fable".to_string(),
                model_id: None,
                window,
            });
        }
    }

    windows
}

fn parse_optional_string_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
}

fn parse_claude_extra_usage(e: &Value) -> Option<ClaudeCodeExtraUsage> {
    let obj = e.as_object()?;
    Some(ClaudeCodeExtraUsage {
        is_enabled: obj
            .get("is_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        used_credits: obj
            .get("used_credits")
            .and_then(as_f64_loose)
            .map(|value| value / 100.0),
        monthly_limit: obj
            .get("monthly_limit")
            .and_then(as_f64_loose)
            .map(|value| value / 100.0),
        utilization: obj.get("utilization").and_then(as_f64_loose),
        currency: parse_optional_string_field(obj, "currency"),
        disabled_reason: parse_optional_string_field(obj, "disabled_reason"),
    })
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
        let oauth_connected =
            !self.oauth_tokens.access_token.is_empty() && !self.oauth_tokens.is_expired();

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

        let mut extra_headers = HashMap::new();
        extra_headers.insert(
            CLAUDE_CODE_OAUTH_EXPIRES_AT_HEADER.to_string(),
            self.oauth_tokens.expires_at.to_string(),
        );

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
            extra_headers,
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
        if self.resolve_auth().is_ok() {
            ModelSource::Api
        } else {
            ModelSource::ModelCaps
        }
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

        let api_models = match fetch_claude_code_models(http_client, &auth_token).await {
            Ok(models) => models,
            Err(e) => {
                tracing::warn!("Claude Code: cannot fetch models from API: {}", e);
                return fallback_models();
            }
        };

        tracing::info!("Claude Code: API returned {} models", api_models.len());
        claude_available_models_from_live(self, &api_models, model_caps)
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

        let _guard = Self::lock_refresh_guard().await?;
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

fn resolve_claude_code_api_model_caps(
    model_caps: &HashMap<String, ModelCapabilities>,
    model_id: &str,
) -> Option<refact_core::model_caps::ResolvedCaps> {
    refact_core::model_caps::resolve_model_caps(model_caps, model_id).or_else(|| {
        refact_core::model_caps::resolve_model_caps(model_caps, &format!("anthropic/{model_id}"))
    })
}

fn claude_live_string(model: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        model
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn claude_live_usize(model: &Value, keys: &[&str]) -> Option<usize> {
    keys.iter()
        .find_map(|key| model.get(*key).and_then(Value::as_u64))
        .and_then(|value| usize::try_from(value).ok())
}

fn claude_available_models_from_live(
    provider: &ClaudeCodeProvider,
    api_models: &[Value],
    model_caps: &HashMap<String, ModelCapabilities>,
) -> Vec<AvailableModel> {
    let enabled_set: HashSet<_> = provider.enabled_models.iter().map(String::as_str).collect();
    let date_regex = regex::Regex::new(r"^(.+?)-\d{8}$").expect("valid static regex");
    let mut models = Vec::new();

    for api_model in api_models {
        let Some(api_id) = claude_live_string(api_model, &["id"]) else {
            continue;
        };
        if !api_id.starts_with("claude-") {
            continue;
        }
        let api_id_without_date = date_regex
            .captures(&api_id)
            .and_then(|caps| caps.get(1))
            .map(|matched| matched.as_str())
            .unwrap_or(&api_id);
        let resolved = resolve_claude_code_api_model_caps(model_caps, api_id_without_date);
        if resolved.is_none() {
            tracing::warn!(
                "Claude Code: model '{}' is missing model capabilities metadata; using API defaults",
                api_id
            );
        }
        let live = LiveModelFields {
            display_name: claude_live_string(api_model, &["display_name", "displayName"]).or_else(
                || {
                    resolved
                        .as_ref()
                        .and_then(|caps| (api_id != caps.matched_key).then(|| api_id.clone()))
                },
            ),
            n_ctx: claude_live_usize(
                api_model,
                &["context_window", "contextWindow", "max_context_window"],
            ),
            max_output_tokens: claude_live_usize(
                api_model,
                &["max_output_tokens", "maxOutputTokens"],
            ),
            supports_cache_control: Some(SUPPORTS_CACHE_CONTROL),
            pricing: provider.custom_model_pricing(&api_id),
            supports_tools: resolved.is_none().then_some(true),
            supports_parallel_tools: resolved.is_none().then_some(true),
            supports_multimodality: resolved.is_none().then_some(true),
            supports_thinking_budget: resolved.is_none().then_some(true),
            ..Default::default()
        };
        models.push(available_model_from_catalog_and_live(
            &api_id,
            resolved.as_ref().map(|caps| &caps.caps),
            &live,
            claude_live_model_is_enabled(&enabled_set, &api_id, api_id_without_date),
            200_000,
        ));
    }

    merge_custom_models(&mut models, &provider.custom_models, &enabled_set);
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models
}

fn claude_live_model_is_enabled(
    enabled_set: &HashSet<&str>,
    api_id: &str,
    api_id_without_date: &str,
) -> bool {
    enabled_set.iter().any(|enabled_id| {
        let unqualified = enabled_id
            .rsplit_once('/')
            .map_or(*enabled_id, |(_, id)| id);
        let enabled_without_date = unqualified
            .rsplit_once('-')
            .filter(|(_, suffix)| {
                suffix.len() == 8 && suffix.chars().all(|character| character.is_ascii_digit())
            })
            .map_or(unqualified, |(base, _)| base);
        unqualified == api_id
            || unqualified == api_id_without_date
            || enabled_without_date == api_id_without_date
    })
}

const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_MODELS_MAX_PAGES: usize = 20;

/// Fetch live model records from the Anthropic API using OAuth credentials.
pub async fn fetch_claude_code_models(
    http_client: &reqwest::Client,
    auth_token: &str,
) -> Result<Vec<Value>, String> {
    if auth_token.is_empty() {
        return Err("empty auth token".to_string());
    }

    let betas = refact_llm::adapters::claude_code_compat::CC_OAUTH_BETAS.join(",");
    let mut models = Vec::new();
    let mut after_id: Option<String> = None;
    for _ in 0..ANTHROPIC_MODELS_MAX_PAGES {
        let mut url = reqwest::Url::parse(ANTHROPIC_MODELS_URL)
            .map_err(|error| format!("Failed to build Claude Code models URL: {error}"))?;
        url.query_pairs_mut().append_pair("limit", "1000");
        if let Some(cursor) = after_id.as_deref() {
            url.query_pairs_mut().append_pair("after_id", cursor);
        }
        let request = http_client
            .get(url)
            .timeout(std::time::Duration::from_secs(8))
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .header("Authorization", format!("Bearer {}", auth_token))
            .header("anthropic-beta", betas.as_str())
            .header(
                "user-agent",
                refact_llm::adapters::claude_code_compat::USER_AGENT,
            );
        let response = request
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Claude Code models: {}", e))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let truncated: String = body.chars().take(512).collect();
            return Err(format!(
                "Claude Code models API returned status {}: {}",
                status, truncated
            ));
        }
        let json = response
            .json::<Value>()
            .await
            .map_err(|e| format!("Failed to parse Claude Code models response: {}", e))?;
        let page = json
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| "Claude Code models response missing data array".to_string())?;
        models.extend(page.iter().cloned());
        let Some(next_cursor) = claude_models_next_cursor(&json)? else {
            return Ok(models);
        };
        if after_id.as_deref() == Some(next_cursor.as_str()) {
            return Err("Claude Code models response repeated pagination cursor".to_string());
        }
        after_id = Some(next_cursor);
    }
    Err(format!(
        "Claude Code models API exceeded {} pagination pages",
        ANTHROPIC_MODELS_MAX_PAGES
    ))
}

fn claude_models_next_cursor(response: &Value) -> Result<Option<String>, String> {
    if response.get("has_more").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    response
        .get("last_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cursor| !cursor.is_empty())
        .map(ToString::to_string)
        .map(Some)
        .ok_or_else(|| "Claude Code models response has_more without last_id".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn claude_live_id_uses_catalog_then_live_and_custom_overrides() {
        let mut provider = ClaudeCodeProvider {
            enabled_models: vec!["claude-sonnet-4-20250514".to_string()],
            ..Default::default()
        };
        let model_caps = HashMap::from([(
            "anthropic/claude-sonnet-4".to_string(),
            ModelCapabilities {
                n_ctx: 200_000,
                supports_tools: true,
                tokenizer: "claude".to_string(),
                ..Default::default()
            },
        )]);
        let live = vec![json!({
            "id": "claude-sonnet-4-20250514",
            "display_name": "Live Sonnet",
            "context_window": 250_000,
            "max_output_tokens": 32_000
        })];

        let models = claude_available_models_from_live(&provider, &live, &model_caps);
        let model = models
            .iter()
            .find(|model| model.id == "claude-sonnet-4-20250514")
            .unwrap();
        assert!(model.supports_tools);
        assert_eq!(model.display_name.as_deref(), Some("Live Sonnet"));
        assert_eq!(model.n_ctx, 250_000);
        assert_eq!(model.max_output_tokens, Some(32_000));

        provider.custom_models.insert(
            "claude-sonnet-4-20250514".to_string(),
            CustomModelConfig {
                n_ctx: Some(123_000),
                supports_tools: Some(false),
                ..Default::default()
            },
        );
        let models = claude_available_models_from_live(&provider, &live, &model_caps);
        let model = models
            .iter()
            .find(|model| model.id == "claude-sonnet-4-20250514")
            .unwrap();
        assert!(model.is_custom);
        assert_eq!(model.n_ctx, 123_000);
        assert!(!model.supports_tools);
        assert!(model.supports_cache_control);
    }

    #[test]
    fn uncatalogued_claude_live_models_keep_api_fallback_capabilities() {
        let provider = ClaudeCodeProvider {
            enabled_models: vec!["anthropic/claude-future-5".to_string()],
            ..Default::default()
        };
        let models = claude_available_models_from_live(
            &provider,
            &[json!({ "id": "claude-future-5-20270101" })],
            &HashMap::new(),
        );
        let model = models
            .iter()
            .find(|model| model.id == "claude-future-5-20270101")
            .unwrap();

        assert!(model.enabled);
        assert!(model.supports_tools);
        assert!(model.supports_parallel_tools);
        assert!(model.supports_multimodality);
        assert!(model.supports_cache_control);
        assert!(model.supports_thinking_budget);
    }

    #[test]
    fn claude_live_models_accept_qualified_exact_base_and_dated_enabled_ids() {
        let api_id = "claude-sonnet-4-20250514";
        let base_id = "claude-sonnet-4";
        for enabled_id in [
            api_id,
            base_id,
            "anthropic/claude-sonnet-4",
            "claude_code/claude-sonnet-4-20250514",
        ] {
            let enabled_set = HashSet::from([enabled_id]);
            assert!(
                claude_live_model_is_enabled(&enabled_set, api_id, base_id),
                "{enabled_id} should enable {api_id}"
            );
        }
    }

    #[test]
    fn claude_models_pagination_uses_anthropic_cursor_fields() {
        assert_eq!(
            claude_models_next_cursor(&json!({
                "has_more": true,
                "first_id": "first",
                "last_id": "last"
            }))
            .unwrap(),
            Some("last".to_string())
        );
        assert_eq!(
            claude_models_next_cursor(&json!({ "has_more": false, "last_id": "last" })).unwrap(),
            None
        );
        assert!(claude_models_next_cursor(&json!({ "has_more": true })).is_err());
    }

    #[test]
    fn claude_code_model_source_uses_api_when_authenticated() {
        assert_eq!(
            ClaudeCodeProvider::default().model_source(),
            ModelSource::ModelCaps
        );
        let provider = ClaudeCodeProvider {
            oauth_tokens: OAuthTokens {
                access_token: "valid".to_string(),
                expires_at: i64::MAX,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(provider.model_source(), ModelSource::Api);
    }

    #[test]
    fn claude_code_usage_parser_preserves_raw_quota_fields() {
        let usage = ClaudeCodeProvider::parse_usage_payload(&json!({
            "five_hour": { "utilization": 12.5, "resets_at": "2026-06-10T12:00:00Z" },
            "seven_day": { "percent_used": "33.7", "reset_at": "2026-06-11T00:00:00Z" },
            "seven_day_sonnet": { "utilization": 44.0, "resets_at": null },
            "seven_day_oauth_apps": null,
            "seven_day_opus": { "utilization": 55 },
            "seven_day_cowork": { "utilization": 0 },
            "seven_day_omelette": { "utilization": 1 },
            "limits": [
                {
                    "kind": "weekly_scoped",
                    "group": "weekly",
                    "percent": 68,
                    "resets_at": "2026-06-11T00:00:00Z",
                    "scope": {
                        "model": {
                            "id": "claude-fable-5",
                            "display_name": "Fable"
                        }
                    }
                },
                {
                    "kind": "weeklyModelScoped",
                    "percent": "27.5",
                    "resets_at": null,
                    "resetAt": 1781222400,
                    "scope": { "model": { "displayName": "Future model" } }
                },
                {
                    "kind": "weekly_scoped",
                    "percent": 99,
                    "scope": {
                        "model": {
                            "id": "claude-fable-5",
                            "display_name": "Duplicate Fable"
                        }
                    }
                },
                null,
                "malformed",
                { "kind": "weekly_scoped", "percent": "not-a-number" },
                {
                    "kind": "weekly_scoped",
                    "percent": 80,
                    "is_active": false,
                    "scope": { "model": { "display_name": "Inactive" } }
                },
                {
                    "kind": "weekly_scoped",
                    "group": "monthly",
                    "percent": 90,
                    "scope": { "model": { "display_name": "Wrong group" } }
                }
            ],
            "fable_weekly": { "utilization": 100 },
            "extra_usage": {
                "is_enabled": false,
                "used_credits": null,
                "monthly_limit": "2550",
                "utilization": "4.5",
                "currency": "USD",
                "disabled_reason": "admin_disabled"
            },
            "cinder_cove": null,
            "iguana_necktie": { "future": true },
            "omelette_promotional": null,
            "tangelo": { "value": 1 },
            "future_window": { "utilization": 99.0 }
        }));

        assert_eq!(usage.five_hour.unwrap().percent_used, 12.5);
        assert_eq!(usage.seven_day.unwrap().percent_used, 33.7);
        assert_eq!(usage.seven_day_sonnet.unwrap().percent_used, 44.0);
        assert!(usage.seven_day_oauth_apps.is_none());
        assert_eq!(usage.seven_day_opus.unwrap().percent_used, 55.0);
        assert_eq!(usage.seven_day_cowork.unwrap().percent_used, 0.0);
        assert_eq!(usage.seven_day_omelette.unwrap().percent_used, 1.0);

        assert_eq!(usage.scoped_windows.len(), 2);
        assert_eq!(usage.scoped_windows[0].label, "Fable");
        assert_eq!(
            usage.scoped_windows[0].model_id.as_deref(),
            Some("claude-fable-5")
        );
        assert_eq!(usage.scoped_windows[0].window.percent_used, 68.0);
        assert_eq!(usage.scoped_windows[1].label, "Future model");
        assert_eq!(usage.scoped_windows[1].window.percent_used, 27.5);
        assert_eq!(
            usage.scoped_windows[1].window.resets_at.as_deref(),
            Some("2026-06-12T00:00:00+00:00")
        );

        let extra = usage.extra_usage.unwrap();
        assert!(!extra.is_enabled);
        assert_eq!(extra.used_credits, None);
        assert_eq!(extra.monthly_limit, Some(25.5));
        assert_eq!(extra.utilization, Some(4.5));
        assert_eq!(extra.currency.as_deref(), Some("USD"));
        assert_eq!(extra.disabled_reason.as_deref(), Some("admin_disabled"));

        assert!(usage.cinder_cove.unwrap().is_null());
        assert_eq!(usage.iguana_necktie.unwrap()["future"], json!(true));
        assert!(usage.omelette_promotional.unwrap().is_null());
        assert_eq!(usage.tangelo.unwrap()["value"], json!(1));
        assert_eq!(usage.raw_extra["future_window"]["utilization"], json!(99.0));
        assert!(usage.raw_extra["limits"].is_array());
    }

    #[test]
    fn claude_code_usage_parser_supports_legacy_fable_window() {
        let usage = ClaudeCodeProvider::parse_usage_payload(&json!({
            "fable_weekly": {
                "utilization": 42,
                "resets_at": "2026-06-13T00:00:00Z"
            }
        }));

        assert_eq!(usage.scoped_windows.len(), 1);
        assert_eq!(usage.scoped_windows[0].label, "Fable");
        assert_eq!(usage.scoped_windows[0].window.percent_used, 42.0);
        assert!(!usage.raw_extra.contains_key("fable_weekly"));
    }

    #[test]
    fn claude_code_extra_usage_converts_minor_currency_units() {
        let usage = ClaudeCodeProvider::parse_usage_payload(&json!({
            "extra_usage": {
                "is_enabled": true,
                "used_credits": 1300,
                "monthly_limit": 30000,
                "utilization": 4.333,
                "currency": "USD"
            }
        }));

        let extra = usage.extra_usage.unwrap();
        assert_eq!(extra.used_credits, Some(13.0));
        assert_eq!(extra.monthly_limit, Some(300.0));
        assert_eq!(extra.utilization, Some(4.333));
    }

    #[test]
    fn claude_code_auth_state_update_is_compare_and_swap() {
        let previous_tokens = OAuthTokens {
            access_token: "old-access".to_string(),
            refresh_token: "old-refresh".to_string(),
            expires_at: 1,
        };
        let mut current = ClaudeCodeProvider {
            oauth_tokens: previous_tokens.clone(),
            enabled_models: vec!["keep-model".to_string()],
            ..Default::default()
        };
        let refreshed = ClaudeCodeProvider {
            oauth_tokens: OAuthTokens {
                access_token: "new-access".to_string(),
                refresh_token: "new-refresh".to_string(),
                expires_at: 2,
            },
            enabled_models: vec!["clobber-model".to_string()],
            ..Default::default()
        };

        assert!(current.update_auth_state_from_if_current(&refreshed, &previous_tokens));
        assert_eq!(current.oauth_tokens, refreshed.oauth_tokens);
        assert_eq!(current.enabled_models, vec!["keep-model".to_string()]);

        current.oauth_tokens.access_token = "newer-login".to_string();
        assert!(!current.update_auth_state_from_if_current(&refreshed, &previous_tokens));
        assert_eq!(current.oauth_tokens.access_token, "newer-login");
    }

    #[test]
    fn claude_code_auth_rejection_refresh_is_bounded() {
        assert!(ClaudeCodeProvider::should_force_refresh_for_status(
            reqwest::StatusCode::UNAUTHORIZED,
            "refresh",
            false,
        ));
        assert!(ClaudeCodeProvider::should_force_refresh_for_status(
            reqwest::StatusCode::FORBIDDEN,
            "refresh",
            false,
        ));
        assert!(!ClaudeCodeProvider::should_force_refresh_for_status(
            reqwest::StatusCode::UNAUTHORIZED,
            "refresh",
            true,
        ));
        assert!(!ClaudeCodeProvider::should_force_refresh_for_status(
            reqwest::StatusCode::BAD_REQUEST,
            "refresh",
            false,
        ));
        assert!(!ClaudeCodeProvider::should_force_refresh_for_status(
            reqwest::StatusCode::UNAUTHORIZED,
            "",
            false,
        ));
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

    #[test]
    fn claude_code_expired_provider_is_not_reported_as_connected() {
        let provider = ClaudeCodeProvider {
            oauth_tokens: OAuthTokens {
                access_token: "expired".to_string(),
                refresh_token: "refresh".to_string(),
                expires_at: 1,
            },
            ..Default::default()
        };

        let settings = provider.provider_settings_as_json();
        assert_eq!(settings["oauth_connected"], false);
        assert_eq!(
            settings["auth_status"],
            "OAuth token expired — needs refresh"
        );
    }
}
