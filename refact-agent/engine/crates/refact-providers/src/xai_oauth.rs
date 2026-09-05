use std::any::Any;
use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex as AMutex, MutexGuard};

use refact_core::model_caps::{resolve_model_caps, ModelCapabilities};
use refact_core::provider_types::{available_model_from_catalog_and_live, LiveModelFields};
use refact_core::llm_types::WireFormat;
use crate::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};
use crate::xai_oauth_flow::OAuthTokens;

const XAI_MODELS_URL: &str = "https://cli-chat-proxy.grok.com/v1/models";
const XAI_RESPONSES_URL: &str = "https://cli-chat-proxy.grok.com/v1/responses";
const XAI_CLIENT_VERSION: &str = "1.0.5";
const XAI_CLIENT_IDENTIFIER: &str = "grok-shell";
const RATE_LIMIT_SAMPLE_MAX_AGE_MS: i64 = 60_000;

/// The Grok CLI proxy reports nothing about tool support: `/v1/models` records
/// carry no `supports_tools`, `capabilities`, or equivalent flag. Every model
/// the proxy serves is driven through `/v1/responses`, which accepts tool calls
/// and parallel tool calls, so this is a deliberate provider-level constant
/// rather than a value guessed per model or borrowed from models.dev.
const SUPPORTS_TOOLS: bool = true;

/// This provider never fabricates a context size: a model whose context window
/// came from neither the live payload nor models.dev is dropped, not defaulted.
const UNKNOWN_N_CTX: usize = 0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XAIOAuthUsage {
    pub source: String,
    pub available: bool,
    pub message: String,
    #[serde(default)]
    pub windows: Vec<XAIOAuthUsageWindow>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XAIOAuthUsageWindow {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub remaining: Option<u64>,
    #[serde(default)]
    pub reset_at: Option<String>,
}

lazy_static::lazy_static! {
    static ref XAI_OAUTH_REFRESH_GUARD: AMutex<()> = AMutex::new(());
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XAIOAuthProvider {
    #[serde(default)]
    pub oauth_tokens: OAuthTokens,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
    /// Ephemeral response metadata. This is deliberately neither serialized nor
    /// deserialized so provider configuration cannot inject or persist headers.
    #[serde(default, skip_serializing, skip_deserializing)]
    cached_rate_limit_sample: Option<XAIRateLimitSample>,
}

#[derive(Debug, Clone)]
struct XAIRateLimitSample {
    headers: HashMap<String, String>,
    captured_at: i64,
}

impl XAIOAuthProvider {
    const RATE_LIMIT_HEADERS: [&'static str; 6] = [
        "x-ratelimit-limit-requests",
        "x-ratelimit-remaining-requests",
        "x-ratelimit-reset-requests",
        "x-ratelimit-limit-tokens",
        "x-ratelimit-remaining-tokens",
        "x-ratelimit-reset-tokens",
    ];

    /// Retains only xAI quota metadata. This is the single allowlist used by
    /// both response handling and the provider cache boundary.
    pub fn filter_rate_limit_headers<I, K, V>(headers: I) -> HashMap<String, String>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        headers
            .into_iter()
            .filter_map(|(name, value)| {
                let normalized_name = name.as_ref().trim().to_ascii_lowercase();
                Self::RATE_LIMIT_HEADERS
                    .contains(&normalized_name.as_str())
                    .then(|| (normalized_name, value.as_ref().trim().to_string()))
            })
            .collect()
    }

    fn cli_request_headers() -> HashMap<String, String> {
        HashMap::from([
            (
                "x-grok-client-version".to_string(),
                XAI_CLIENT_VERSION.to_string(),
            ),
            (
                "x-grok-client-identifier".to_string(),
                XAI_CLIENT_IDENTIFIER.to_string(),
            ),
        ])
    }

    pub async fn lock_refresh_guard() -> Result<MutexGuard<'static, ()>, String> {
        tokio::time::timeout(Duration::from_secs(30), XAI_OAUTH_REFRESH_GUARD.lock())
            .await
            .map_err(|_| "xAI Grok OAuth refresh guard timed out".to_string())
    }

    fn needs_refresh_on_start(expires_at: i64) -> bool {
        const REFRESH_BEFORE_EXPIRY_MS: i64 = 5 * 60 * 1000;
        expires_at == 0
            || chrono::Utc::now().timestamp_millis() >= expires_at - REFRESH_BEFORE_EXPIRY_MS
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
                .and_then(|value| value.as_mapping())
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

    /// The proxy labels records with `name`; it has never sent `display_name`.
    fn live_display_name(model: &serde_json::Value) -> Option<String> {
        model
            .get("name")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    /// The proxy reports context size as `context_window` (500000 for the
    /// current Grok models). It does not send `context_length`.
    fn live_n_ctx(model: &serde_json::Value) -> Option<usize> {
        model
            .get("context_window")
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
    }

    /// `reasoning_efforts` is an array of `{id, value, label, description,
    /// default}` records; only the `id` values are options we can send. The list
    /// is honored only when the record also sets `supports_reasoning_effort`.
    fn live_reasoning_effort_options(model: &serde_json::Value) -> Option<Vec<String>> {
        if !model
            .get("supports_reasoning_effort")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return None;
        }
        let efforts = model.get("reasoning_efforts")?.as_array()?;
        Some(
            efforts
                .iter()
                .filter_map(|effort| effort.get("id").and_then(|value| value.as_str()))
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect(),
        )
    }

    fn available_models_from_live_response(
        &self,
        response: &serde_json::Value,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Option<Vec<AvailableModel>> {
        let models = response.get("data")?.as_array()?;
        let enabled_set: std::collections::HashSet<&str> = self
            .enabled_models
            .iter()
            .map(|model| model.as_str())
            .collect();
        let mut available_models = Vec::new();

        for model in models {
            let Some(id) = model.get("id").and_then(|value| value.as_str()) else {
                continue;
            };

            let live = LiveModelFields {
                display_name: Self::live_display_name(model),
                n_ctx: Self::live_n_ctx(model),
                reasoning_effort_options: Self::live_reasoning_effort_options(model),
                supports_web_search: model
                    .get("supports_backend_search")
                    .and_then(|value| value.as_bool()),
                api_mode: model
                    .get("api_backend")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                base_model: model
                    .get("model")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                // The proxy never reports tool support; see SUPPORTS_TOOLS.
                supports_tools: Some(SUPPORTS_TOOLS),
                supports_parallel_tools: Some(SUPPORTS_TOOLS),
                ..Default::default()
            };
            // models.dev only enriches ids the proxy already returned: it fills
            // what the live payload omits (max_output_tokens, vision, pricing,
            // tokenizer) and can never contribute an id of its own.
            let caps = resolve_model_caps(model_caps, &format!("xai/{id}"))
                .or_else(|| resolve_model_caps(model_caps, &format!("xai_oauth/{id}")))
                .or_else(|| resolve_model_caps(model_caps, id))
                .map(|resolved| resolved.caps);
            let enabled = enabled_set.contains(id)
                || enabled_set.contains(format!("xai/{id}").as_str())
                || enabled_set.contains(format!("xai_oauth/{id}").as_str());
            // UNKNOWN_N_CTX is 0: this provider never fabricates a context size.
            let available = available_model_from_catalog_and_live(
                id,
                caps.as_ref(),
                &live,
                enabled,
                UNKNOWN_N_CTX,
            );
            if available.n_ctx == 0 {
                // Unreachable while the proxy keeps sending `context_window`
                // (500000 on both live models). If it ever stops, fail loudly
                // rather than silently serving an invented window.
                tracing::warn!(
                    "xAI OAuth: model '{id}' has no context window in the proxy payload \
                     (`context_window`) and none in models.dev; skipping it instead of \
                     advertising a fabricated context size."
                );
                continue;
            }
            available_models.push(available);
        }

        merge_custom_models(&mut available_models, &self.custom_models, &enabled_set);
        available_models.sort_by(|left, right| left.id.cmp(&right.id));
        Some(available_models)
    }

    pub fn passive_usage() -> XAIOAuthUsage {
        XAIOAuthUsage {
            source: "passive".to_string(),
            available: false,
            message: "xAI OAuth has no public usage endpoint; quota is available only when the proxy returns rate-limit metadata on normal requests.".to_string(),
            windows: Vec::new(),
            headers: HashMap::new(),
        }
    }

    /// Captures only known quota headers from a normal xAI proxy response.
    ///
    /// Header names are matched case-insensitively and stored in their canonical
    /// lowercase form. In particular, authorization, cookies, and arbitrary
    /// response headers can never enter the cache. A response without recognized
    /// headers leaves the last captured sample intact.
    pub fn capture_rate_limit_headers(&mut self, headers: &HashMap<String, String>) -> bool {
        self.capture_rate_limit_headers_at(headers, chrono::Utc::now().timestamp_millis())
    }

    /// Timestamp-explicit variant, useful when the caller already records the
    /// response time (and for deterministic consumers).
    pub fn capture_rate_limit_headers_at(
        &mut self,
        headers: &HashMap<String, String>,
        captured_at: i64,
    ) -> bool {
        let allowed = Self::filter_rate_limit_headers(headers);
        if allowed.is_empty() {
            return false;
        }

        self.cached_rate_limit_sample = Some(XAIRateLimitSample {
            headers: allowed,
            captured_at,
        });
        true
    }

    /// Returns normalized quota metadata from the most recently captured
    /// response, or the existing passive/unavailable result before any capture.
    pub fn captured_usage(&self) -> Option<XAIOAuthUsage> {
        self.captured_usage_at(chrono::Utc::now().timestamp_millis())
    }

    fn captured_usage_at(&self, now: i64) -> Option<XAIOAuthUsage> {
        let sample = self.cached_rate_limit_sample.as_ref()?;
        if now.saturating_sub(sample.captured_at) >= RATE_LIMIT_SAMPLE_MAX_AGE_MS {
            return None;
        }
        let headers = sample.headers.clone();
        let mut windows = Vec::new();
        if let Some(window) = Self::usage_window_from_headers(&headers, "requests") {
            windows.push(window);
        }
        if let Some(window) = Self::usage_window_from_headers(&headers, "tokens") {
            windows.push(window);
        }

        let captured = format!(" at {} ms since Unix epoch", sample.captured_at);
        Some(XAIOAuthUsage {
            source: "response_headers".to_string(),
            available: !windows.is_empty(),
            message: if windows.is_empty() {
                format!("xAI proxy returned rate-limit headers{captured}, but no valid quota values were present.")
            } else {
                format!("Quota captured from xAI proxy response headers{captured}.")
            },
            windows,
            headers,
        })
    }

    fn usage_window_from_headers(
        headers: &HashMap<String, String>,
        name: &str,
    ) -> Option<XAIOAuthUsageWindow> {
        let parse_u64 = |field: &str| {
            headers
                .get(&format!("x-ratelimit-{field}-{name}"))
                .and_then(|value| value.parse::<u64>().ok())
        };
        let limit = parse_u64("limit");
        let remaining = parse_u64("remaining");
        let reset_at = headers
            .get(&format!("x-ratelimit-reset-{name}"))
            .filter(|value| !value.is_empty())
            .cloned();
        if limit.is_none() && remaining.is_none() && reset_at.is_none() {
            return None;
        }
        Some(XAIOAuthUsageWindow {
            name: Some(name.to_string()),
            limit,
            remaining,
            reset_at,
        })
    }

    pub async fn fetch_usage(&self) -> Result<XAIOAuthUsage, String> {
        Ok(self.captured_usage().unwrap_or_else(Self::passive_usage))
    }
}

#[async_trait]
impl ProviderTrait for XAIOAuthProvider {
    fn name(&self) -> &str {
        "xai_oauth"
    }

    fn display_name(&self) -> &str {
        "xAI Grok (SuperGrok / X Premium+)"
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
        // The proxy is the only source of ids for this provider, so there is
        // nothing to filter: whatever `/v1/models` serves is servable.
        None
    }

    /// This provider has no catalog view. models.dev carries no `xai_oauth/*`
    /// keys, so deriving models from it would advertise ids the Grok proxy does
    /// not serve. Callers that reach for a catalog listing (for example the
    /// model-discovery timeout path in caps) get custom models only.
    fn get_available_models_from_caps(
        &self,
        _model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        self.get_custom_models_only()
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields: {}
oauth:
  supported: true
  methods:
    - id: subscription
      label: "SuperGrok / X Premium+"
      description: "Login with your SuperGrok or X Premium+ subscription"
description: |
  Use a SuperGrok or X Premium+ subscription instead of an API key to access xAI Grok models.

  Users whose subscription tier is not entitled will receive HTTP 403 and should use the
  API-key `xai_responses` provider instead.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(oauth_tokens) = yaml.get("oauth_tokens") {
            self.oauth_tokens = serde_yaml::from_value(oauth_tokens.clone()).unwrap_or_default();
        }
        if let Some(enabled) = yaml.get("enabled").and_then(|value| value.as_bool()) {
            self.enabled = enabled;
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        json!({
            "oauth_connected": !self.oauth_tokens.is_empty(),
            "enabled": self.enabled,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let api_key = self.oauth_tokens.access_token.clone();
        let extra_headers = Self::cli_request_headers();
        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !api_key.is_empty() && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: XAI_RESPONSES_URL.to_string(),
            completion_endpoint: String::new(),
            embedding_endpoint: String::new(),
            api_key,
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers,
            supports_cache_control: true,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn has_credentials(&self) -> bool {
        !self.oauth_tokens.access_token.is_empty()
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
        // The Grok CLI proxy is the only authority on which models this
        // subscription can call. models.dev carries no `xai_oauth/*` entries, so
        // a catalog fallback here would advertise bare `grok-*` ids the proxy
        // does not serve while omitting the ones it does. When the live call
        // fails we therefore surface custom models only.
        if self.oauth_tokens.access_token.is_empty() {
            return self.get_custom_models_only();
        }

        let mut request = http_client
            .get(XAI_MODELS_URL)
            .bearer_auth(&self.oauth_tokens.access_token);
        for (name, value) in Self::cli_request_headers() {
            request = request.header(name, value);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(
                    "xAI OAuth: could not reach the Grok proxy to list models ({error}); this is \
                     usually transient. No models are advertised for this attempt."
                );
                return self.get_custom_models_only();
            }
        };
        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::UNAUTHORIZED {
                tracing::warn!(
                    "xAI OAuth: the Grok proxy rejected the access token ({status}). The xAI \
                     OAuth session needs re-login in provider settings. Returning custom models \
                     only."
                );
            } else if status == reqwest::StatusCode::FORBIDDEN {
                tracing::warn!(
                    "xAI OAuth: the Grok proxy returned {status}, which for this provider means \
                     the subscription tier is not entitled to Grok CLI access. Use the API-key \
                     `xai_responses` provider instead. Returning custom models only."
                );
            } else if status.is_server_error() {
                tracing::warn!(
                    "xAI OAuth: the Grok proxy returned {status}; this is a transient upstream \
                     failure. Returning custom models only for this attempt."
                );
            } else {
                tracing::warn!(
                    "xAI OAuth: listing models returned unexpected status {status}. Returning \
                     custom models only for this attempt."
                );
            }
            return self.get_custom_models_only();
        }
        let response = match response.json::<serde_json::Value>().await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(
                    "xAI OAuth: the Grok proxy returned a models payload we could not parse \
                     ({error}). No models are advertised for this attempt."
                );
                return self.get_custom_models_only();
            }
        };

        match self.available_models_from_live_response(&response, model_caps) {
            Some(models) => models,
            None => {
                tracing::warn!(
                    "xAI OAuth: the Grok proxy models payload had no `data` array. No models are \
                     advertised for this attempt."
                );
                self.get_custom_models_only()
            }
        }
    }

    async fn startup_refresh_and_sync(
        &mut self,
        http_client: &reqwest::Client,
        config_dir: &std::path::Path,
        instance_id: &str,
    ) -> Result<(), String> {
        if self.oauth_tokens.is_empty() || !self.oauth_tokens.has_refresh_token() {
            return Ok(());
        }
        if !Self::needs_refresh_on_start(self.oauth_tokens.expires_at) {
            return Ok(());
        }
        tracing::info!("xAI OAuth: refreshing token on startup");
        let refreshed = match crate::xai_oauth_flow::refresh_access_token(
            http_client,
            &self.oauth_tokens.refresh_token,
        )
        .await
        {
            Ok(refreshed) => refreshed,
            Err(error) if crate::oauth_refresh::is_permanent_refresh_error(&error) => {
                crate::oauth_refresh::mark_invalid_refresh_token(
                    instance_id,
                    &self.oauth_tokens.refresh_token,
                );
                tracing::warn!(
                    "xAI OAuth refresh token is invalid; clearing saved OAuth tokens: {}",
                    error
                );
                self.oauth_tokens = OAuthTokens::default();
                self.save_oauth_tokens_config(config_dir, instance_id)
                    .await?;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.oauth_tokens = refreshed;
        self.save_oauth_tokens_config(config_dir, instance_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Mirrors a real record from GET https://cli-chat-proxy.grok.com/v1/models
    /// (HTTP 200). Every key here is one the live proxy actually sends; in
    /// particular the context size arrives as `context_window`, never as
    /// `context_length`.
    fn live_grok_4_6_record() -> serde_json::Value {
        json!({
            "id": "grok-4.6",
            "object": "model",
            "owned_by": "xai",
            "model": "grok-4-6",
            "model_family": "grok-4",
            "name": "Grok 4.6",
            "description": "Our most capable model",
            "context_window": 500000,
            "auto_compact_threshold_percent": 80,
            "system_prompt_label": "grok-4.6",
            "api_backend": "responses",
            "reasoning_effort": "high",
            "supports_reasoning_effort": true,
            "reasoning_efforts": [
                {"id": "xhigh", "value": "xhigh", "label": "Extra High", "description": "", "default": false},
                {"id": "high", "value": "high", "label": "High", "description": "", "default": true},
                {"id": "medium", "value": "medium", "label": "Medium", "description": "", "default": false},
                {"id": "low", "value": "low", "label": "Low", "description": "", "default": false}
            ],
            "supports_backend_search": true,
            "compactions_remaining": 5,
            "compaction_at_tokens": 400000
        })
    }

    /// The second model the live proxy serves. It has no `description` and a
    /// shorter reasoning-effort list.
    fn live_grok_4_5_record() -> serde_json::Value {
        json!({
            "id": "grok-4.5",
            "object": "model",
            "owned_by": "xai",
            "model": "grok-4-5",
            "model_family": "grok-4",
            "name": "Grok 4.5",
            "context_window": 500000,
            "auto_compact_threshold_percent": 80,
            "system_prompt_label": "grok-4.5",
            "api_backend": "responses",
            "reasoning_effort": "high",
            "supports_reasoning_effort": true,
            "reasoning_efforts": [
                {"id": "high", "value": "high", "label": "High", "description": "", "default": true},
                {"id": "medium", "value": "medium", "label": "Medium", "description": "", "default": false},
                {"id": "low", "value": "low", "label": "Low", "description": "", "default": false}
            ],
            "supports_backend_search": true,
            "compactions_remaining": 5,
            "compaction_at_tokens": 400000
        })
    }

    #[test]
    fn live_n_ctx_is_read_from_context_window() {
        let provider = XAIOAuthProvider {
            enabled_models: vec!["grok-4.6".to_string()],
            ..Default::default()
        };
        let response = json!({"data": [live_grok_4_6_record(), live_grok_4_5_record()]});

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        assert_eq!(models.len(), 2);
        let grok46 = models.iter().find(|model| model.id == "grok-4.6").unwrap();
        let grok45 = models.iter().find(|model| model.id == "grok-4.5").unwrap();
        // 500000 comes from `context_window`; nothing else in the payload has it.
        assert_eq!(grok46.n_ctx, 500_000);
        assert_eq!(grok45.n_ctx, 500_000);
        assert!(grok46.enabled);
        assert!(!grok45.enabled);
    }

    #[test]
    fn live_record_without_context_window_is_dropped_not_defaulted() {
        let provider = XAIOAuthProvider::default();
        let mut record = live_grok_4_6_record();
        record.as_object_mut().unwrap().remove("context_window");
        // `context_length` is not a key the proxy uses; reading it must not work.
        record.as_object_mut().unwrap().insert(
            "context_length".to_string(),
            serde_json::Value::from(128_000),
        );
        let response = json!({"data": [record]});

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        // No fabricated context size, and no zero-context model either.
        assert!(models.is_empty());
    }

    #[test]
    fn missing_context_window_is_recovered_from_models_dev_rather_than_dropped() {
        let provider = XAIOAuthProvider::default();
        let mut record = live_grok_4_6_record();
        record.as_object_mut().unwrap().remove("context_window");
        let response = json!({"data": [record]});
        let caps = HashMap::from([(
            "xai_oauth/grok-4.6".to_string(),
            ModelCapabilities {
                n_ctx: 256_000,
                ..Default::default()
            },
        )]);

        let models = provider
            .available_models_from_live_response(&response, &caps)
            .unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].n_ctx, 256_000);
    }

    #[test]
    fn live_reasoning_effort_options_come_from_reasoning_efforts_ids() {
        let provider = XAIOAuthProvider::default();
        let response = json!({"data": [live_grok_4_6_record(), live_grok_4_5_record()]});

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        let grok46 = models.iter().find(|model| model.id == "grok-4.6").unwrap();
        let grok45 = models.iter().find(|model| model.id == "grok-4.5").unwrap();
        let expected_46: Vec<String> = ["xhigh", "high", "medium", "low"]
            .iter()
            .map(|effort| effort.to_string())
            .collect();
        let expected_45: Vec<String> = ["high", "medium", "low"]
            .iter()
            .map(|effort| effort.to_string())
            .collect();
        assert_eq!(grok46.reasoning_effort_options, Some(expected_46));
        assert_eq!(grok45.reasoning_effort_options, Some(expected_45));
    }

    #[test]
    fn live_reasoning_efforts_are_ignored_when_unsupported() {
        let provider = XAIOAuthProvider::default();
        let mut record = live_grok_4_6_record();
        record.as_object_mut().unwrap().insert(
            "supports_reasoning_effort".to_string(),
            serde_json::Value::Bool(false),
        );
        let response = json!({"data": [record]});

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        assert_eq!(models[0].reasoning_effort_options, None);
    }

    #[test]
    fn live_record_fills_name_backend_search_api_backend_and_base_model() {
        let provider = XAIOAuthProvider::default();
        let response = json!({"data": [live_grok_4_6_record()]});

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        assert_eq!(models[0].display_name.as_deref(), Some("Grok 4.6"));
        assert!(models[0].supports_web_search);
        assert_eq!(models[0].api_mode.as_deref(), Some("responses"));
        assert_eq!(models[0].base_model.as_deref(), Some("grok-4-6"));
    }

    #[test]
    fn live_tool_support_is_a_provider_constant_not_a_catalog_guess() {
        let provider = XAIOAuthProvider::default();
        let response = json!({"data": [live_grok_4_6_record()]});
        // models.dev claims no tools; the proxy says nothing, so our constant wins.
        let caps = HashMap::from([(
            "xai_oauth/grok-4.6".to_string(),
            ModelCapabilities {
                n_ctx: 128_000,
                supports_tools: false,
                ..Default::default()
            },
        )]);

        let with_caps = provider
            .available_models_from_live_response(&response, &caps)
            .unwrap();
        let without_caps = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        assert_eq!(with_caps[0].supports_tools, SUPPORTS_TOOLS);
        assert_eq!(with_caps[0].supports_parallel_tools, SUPPORTS_TOOLS);
        assert_eq!(without_caps[0].supports_tools, SUPPORTS_TOOLS);
        assert_eq!(without_caps[0].supports_parallel_tools, SUPPORTS_TOOLS);
    }

    #[test]
    fn live_context_window_overrides_provider_qualified_catalog_caps() {
        let provider = XAIOAuthProvider {
            enabled_models: vec!["xai_oauth/grok-4.6".to_string()],
            ..Default::default()
        };
        let response = json!({"data": [live_grok_4_6_record()]});
        let caps = HashMap::from([(
            "xai_oauth/grok-4.6".to_string(),
            ModelCapabilities {
                n_ctx: 128_000,
                max_output_tokens: 64_000,
                supports_vision: true,
                ..Default::default()
            },
        )]);

        let models = provider
            .available_models_from_live_response(&response, &caps)
            .unwrap();

        assert_eq!(models[0].n_ctx, 500_000);
        assert!(models[0].enabled);
        // models.dev still enriches what the live payload never reports.
        assert_eq!(models[0].max_output_tokens, Some(64_000));
        assert!(models[0].supports_multimodality);
    }

    #[test]
    fn live_models_dev_never_contributes_an_id() {
        let provider = XAIOAuthProvider::default();
        let response = json!({"data": [live_grok_4_6_record()]});
        let caps = HashMap::from([
            (
                "xai_oauth/grok-4.6".to_string(),
                ModelCapabilities::default(),
            ),
            ("xai/grok-3".to_string(), ModelCapabilities::default()),
            ("xai_oauth/grok-2".to_string(), ModelCapabilities::default()),
        ]);

        let models = provider
            .available_models_from_live_response(&response, &caps)
            .unwrap();

        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["grok-4.6"]
        );
    }

    #[test]
    fn live_ids_that_do_not_start_with_grok_are_kept() {
        let provider = XAIOAuthProvider {
            enabled_models: vec!["some-future-model".to_string()],
            ..Default::default()
        };
        let response = json!({
            "data": [
                live_grok_4_6_record(),
                {
                    "id": "some-future-model",
                    "object": "model",
                    "owned_by": "xai",
                    "model": "some-future-model",
                    "name": "Some Future Model",
                    "context_window": 262144,
                    "api_backend": "responses",
                    "supports_reasoning_effort": false,
                    "supports_backend_search": false
                }
            ]
        });

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        let future = models
            .iter()
            .find(|model| model.id == "some-future-model")
            .expect("non-grok ids from the proxy must not be dropped");
        assert_eq!(future.n_ctx, 262_144);
        assert!(future.enabled);
        assert!(!future.supports_web_search);
    }

    #[test]
    fn provider_does_not_filter_model_ids() {
        assert_eq!(XAIOAuthProvider::default().model_filter_regex(), None);
    }

    #[test]
    fn catalog_listing_never_invents_models_from_models_dev() {
        let provider = XAIOAuthProvider {
            enabled_models: vec!["grok-4.6".to_string()],
            ..Default::default()
        };
        let caps = HashMap::from([
            ("xai/grok-3".to_string(), ModelCapabilities::default()),
            ("grok-2".to_string(), ModelCapabilities::default()),
            ("gpt-4.1".to_string(), ModelCapabilities::default()),
        ]);

        // No live payload, no custom models: nothing may be advertised.
        assert!(provider.get_available_models_from_caps(&caps).is_empty());
    }

    #[test]
    fn passive_usage_is_honest_and_has_no_fabricated_quota() {
        let usage = XAIOAuthProvider::passive_usage();
        assert_eq!(usage.source, "passive");
        assert!(!usage.available);
        assert!(usage.windows.is_empty());
        assert!(usage.headers.is_empty());
        assert!(usage.message.contains("rate-limit metadata"));
    }

    #[tokio::test]
    async fn no_captured_headers_retains_passive_usage() {
        let mut provider = XAIOAuthProvider::default();

        assert!(provider.captured_usage().is_none());
        assert!(!provider.capture_rate_limit_headers(&HashMap::new()));
        assert!(provider.captured_usage().is_none());
        assert_eq!(
            provider.fetch_usage().await.unwrap(),
            XAIOAuthProvider::passive_usage()
        );
    }

    #[test]
    fn captured_request_and_token_headers_are_normalized() {
        let mut provider = XAIOAuthProvider::default();
        let headers = HashMap::from([
            ("X-RateLimit-Limit-Requests".to_string(), " 60 ".to_string()),
            (
                "x-ratelimit-remaining-requests".to_string(),
                "42".to_string(),
            ),
            ("x-ratelimit-reset-requests".to_string(), "1s".to_string()),
            ("x-ratelimit-limit-tokens".to_string(), "100000".to_string()),
            (
                "x-ratelimit-remaining-tokens".to_string(),
                "75000".to_string(),
            ),
            ("x-ratelimit-reset-tokens".to_string(), "6m0s".to_string()),
            ("authorization".to_string(), "Bearer secret".to_string()),
            ("set-cookie".to_string(), "secret-cookie".to_string()),
        ]);

        assert!(provider.capture_rate_limit_headers_at(&headers, 1_700_000_000_000));
        let usage = provider.captured_usage_at(1_700_000_000_001).unwrap();

        assert_eq!(usage.source, "response_headers");
        assert!(usage.available);
        assert_eq!(usage.windows.len(), 2);
        assert_eq!(usage.windows[0].name.as_deref(), Some("requests"));
        assert_eq!(usage.windows[0].limit, Some(60));
        assert_eq!(usage.windows[0].remaining, Some(42));
        assert_eq!(usage.windows[0].reset_at.as_deref(), Some("1s"));
        assert_eq!(usage.windows[1].name.as_deref(), Some("tokens"));
        assert_eq!(usage.windows[1].limit, Some(100_000));
        assert_eq!(usage.windows[1].remaining, Some(75_000));
        assert_eq!(usage.windows[1].reset_at.as_deref(), Some("6m0s"));
        assert_eq!(usage.headers.len(), 6);
        assert!(!usage.headers.contains_key("authorization"));
        assert!(!usage.headers.contains_key("set-cookie"));
        assert!(usage.message.contains("1700000000000"));
    }

    #[test]
    fn malformed_headers_do_not_fabricate_numeric_quota() {
        let mut provider = XAIOAuthProvider::default();
        let headers = HashMap::from([
            ("x-ratelimit-limit-requests".to_string(), "many".to_string()),
            (
                "x-ratelimit-remaining-requests".to_string(),
                "-1".to_string(),
            ),
            ("x-ratelimit-limit-tokens".to_string(), "1.5".to_string()),
            ("x-ratelimit-reset-tokens".to_string(), "".to_string()),
            ("x-api-key".to_string(), "must-not-be-stored".to_string()),
        ]);

        assert!(provider.capture_rate_limit_headers_at(&headers, 123));
        let usage = provider.captured_usage_at(124).unwrap();

        assert!(!usage.available);
        assert!(usage.windows.is_empty());
        assert!(!usage.headers.contains_key("x-api-key"));
        assert!(usage.message.contains("no valid quota values"));
    }

    #[test]
    fn cached_headers_are_serde_defaulted_and_hidden() {
        let mut provider: XAIOAuthProvider = serde_yaml::from_str("enabled: true\n").unwrap();
        assert!(provider.captured_usage().is_none());
        assert!(provider.capture_rate_limit_headers_at(
            &HashMap::from([("x-ratelimit-limit-requests".to_string(), "10".to_string(),)]),
            456,
        ));

        let serialized = serde_yaml::to_string(&provider).unwrap();
        assert!(!serialized.contains("cached_rate_limit"));
        assert!(!serialized.contains("x-ratelimit"));

        let injected: XAIOAuthProvider = serde_yaml::from_str(
            "enabled: false\ncached_rate_limit_sample:\n  headers:\n    authorization: Bearer-secret\n  captured_at: 789\n",
        )
        .unwrap();
        assert!(injected.captured_usage().is_none());
    }

    #[test]
    fn captured_headers_expire_after_sixty_seconds() {
        let mut provider = XAIOAuthProvider::default();
        assert!(provider.capture_rate_limit_headers_at(
            &HashMap::from([(
                "x-ratelimit-remaining-requests".to_string(),
                "5".to_string(),
            )]),
            10_000,
        ));

        assert!(provider.captured_usage_at(69_999).is_some());
        assert!(provider.captured_usage_at(70_000).is_none());
    }

    #[test]
    fn provider_filter_rejects_non_quota_response_headers() {
        let filtered = XAIOAuthProvider::filter_rate_limit_headers([
            ("X-RateLimit-Remaining-Requests", " 3 "),
            ("retry-after", "10"),
            ("authorization", "Bearer secret"),
            ("set-cookie", "secret=true"),
        ]);

        assert_eq!(
            filtered,
            HashMap::from([(
                "x-ratelimit-remaining-requests".to_string(),
                "3".to_string(),
            )])
        );
    }

    #[test]
    fn runtime_uses_grok_subscription_proxy_identity() {
        let provider = XAIOAuthProvider {
            oauth_tokens: OAuthTokens {
                access_token: "oauth-access-token".to_string(),
                ..Default::default()
            },
            enabled: true,
            enabled_models: vec!["grok-4.6".to_string()],
            ..Default::default()
        };

        let runtime = provider.build_runtime().unwrap();

        assert_eq!(runtime.chat_endpoint, XAI_RESPONSES_URL);
        assert_eq!(runtime.api_key, "oauth-access-token");
        assert!(runtime.auth_token.is_empty());
        assert_eq!(
            runtime.extra_headers.get("x-grok-client-version"),
            Some(&XAI_CLIENT_VERSION.to_string())
        );
        assert_eq!(
            runtime.extra_headers.get("x-grok-client-identifier"),
            Some(&XAI_CLIENT_IDENTIFIER.to_string())
        );
        assert_eq!(
            XAIOAuthProvider::cli_request_headers(),
            runtime.extra_headers
        );
    }
}
