use std::any::Any;
use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex as AMutex, MutexGuard};

use refact_core::antigravity_client::antigravity_headers;
use refact_core::model_caps::{resolve_model_caps, ModelCapabilities};
use refact_core::llm_types::WireFormat;
use crate::google_antigravity_oauth::OAuthTokens;
use crate::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, parse_custom_models, parse_enabled_models, set_model_enabled_impl,
};

const CLOUDCODE_PROJECT_HEADER: &str = "x-refact-internal-cloudcode-project";
const CLOUDCODE_BASE_URL: &str = "https://daily-cloudcode-pa.googleapis.com";
const CLOUDCODE_MODELS_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels";
const CLOUDCODE_QUOTA_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoogleAntigravityQuotaSummary {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub groups: Vec<GoogleAntigravityQuotaGroup>,
    #[serde(default)]
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoogleAntigravityQuotaGroup {
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub buckets: Vec<GoogleAntigravityQuotaBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoogleAntigravityQuotaBucket {
    pub bucket_id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub remaining_fraction: Option<f64>,
    #[serde(default)]
    pub reset: Option<String>,
}

#[derive(Debug)]
pub enum GoogleAntigravityQuotaRequestError {
    Status(reqwest::StatusCode, String),
    Other(String),
}

#[derive(Deserialize)]
struct QuotaSummaryWire {
    #[serde(default)]
    description: Option<String>,
    #[serde(default, alias = "quotaGroups", alias = "quota_groups")]
    groups: Vec<QuotaGroupWire>,
}

#[derive(Deserialize)]
struct QuotaGroupWire {
    #[serde(default, alias = "displayName")]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, alias = "quotaBuckets", alias = "quota_buckets")]
    buckets: Vec<QuotaBucketWire>,
}

#[derive(Deserialize)]
struct QuotaBucketWire {
    #[serde(default, alias = "bucketId")]
    bucket_id: Option<String>,
    #[serde(default, alias = "displayName")]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, alias = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(default)]
    remaining: Option<QuotaRemainingWire>,
    #[serde(default, alias = "resetTime")]
    reset: Option<String>,
}

#[derive(Deserialize)]
struct QuotaRemainingWire {
    #[serde(default, alias = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(default, alias = "resetTime")]
    reset: Option<String>,
}

lazy_static::lazy_static! {
    static ref GOOGLE_ANTIGRAVITY_REFRESH_GUARD: AMutex<()> = AMutex::new(());
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoogleAntigravityProvider {
    #[serde(default)]
    pub oauth_tokens: OAuthTokens,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl GoogleAntigravityProvider {
    pub async fn lock_refresh_guard() -> Result<MutexGuard<'static, ()>, String> {
        tokio::time::timeout(
            Duration::from_secs(30),
            GOOGLE_ANTIGRAVITY_REFRESH_GUARD.lock(),
        )
        .await
        .map_err(|_| "Google Antigravity OAuth refresh guard timed out".to_string())
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
            tokens_map.insert(
                serde_yaml::Value::String("project_id".to_string()),
                serde_yaml::Value::String(tokens.project_id),
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

    fn available_models_from_live_response(
        &self,
        response: &serde_json::Value,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Option<Vec<AvailableModel>> {
        let models = response.get("models")?.as_object()?;
        let enabled_set: std::collections::HashSet<&str> = self
            .enabled_models
            .iter()
            .map(|model| model.as_str())
            .collect();
        let mut available_models = Vec::new();

        for (id, metadata) in models {
            if id.is_empty() {
                continue;
            }
            let caps = resolve_model_caps(model_caps, &format!("google_antigravity/{id}"))
                .or_else(|| resolve_model_caps(model_caps, &format!("google/{id}")))
                .or_else(|| resolve_model_caps(model_caps, id))
                .map(|resolved| resolved.caps)
                .unwrap_or_else(|| ModelCapabilities {
                    n_ctx: 128_000,
                    supports_tools: true,
                    supports_parallel_tools: true,
                    ..Default::default()
                });
            let pricing = self
                .custom_model_pricing(id)
                .or_else(|| caps.pricing.clone());
            let mut available =
                AvailableModel::from_caps(id, &caps, enabled_set.contains(id.as_str()), pricing);
            available.display_name = metadata
                .get("displayName")
                .or_else(|| metadata.get("display_name"))
                .and_then(|value| value.as_str())
                .map(str::to_string);
            available_models.push(available);
        }

        merge_custom_models(&mut available_models, &self.custom_models, &enabled_set);
        available_models.sort_by(|left, right| left.id.cmp(&right.id));
        Some(available_models)
    }

    pub fn parse_quota_summary(
        payload: &serde_json::Value,
    ) -> Result<GoogleAntigravityQuotaSummary, String> {
        let parsed: QuotaSummaryWire =
            serde_json::from_value(payload.clone()).map_err(|error| {
                format!("Failed to parse Google Antigravity quota response: {error}")
            })?;
        Ok(GoogleAntigravityQuotaSummary {
            description: parsed.description,
            groups: parsed
                .groups
                .into_iter()
                .map(|group| GoogleAntigravityQuotaGroup {
                    display_name: group.display_name.unwrap_or_default(),
                    description: group.description,
                    buckets: group
                        .buckets
                        .into_iter()
                        .map(|bucket| GoogleAntigravityQuotaBucket {
                            bucket_id: bucket.bucket_id.unwrap_or_default(),
                            display_name: bucket.display_name.unwrap_or_default(),
                            description: bucket.description,
                            remaining_fraction: bucket.remaining_fraction.or_else(|| {
                                bucket
                                    .remaining
                                    .as_ref()
                                    .and_then(|remaining| remaining.remaining_fraction)
                            }),
                            reset: bucket
                                .reset
                                .or_else(|| bucket.remaining.and_then(|remaining| remaining.reset)),
                        })
                        .collect(),
                })
                .collect(),
            raw: payload.clone(),
        })
    }

    pub async fn fetch_quota_summary_once(
        &self,
        http_client: &reqwest::Client,
        access_token: &str,
    ) -> Result<GoogleAntigravityQuotaSummary, GoogleAntigravityQuotaRequestError> {
        if self.oauth_tokens.project_id.is_empty() {
            return Err(GoogleAntigravityQuotaRequestError::Other(
                "Google Antigravity project ID is missing; log in again".to_string(),
            ));
        }
        let mut request = http_client
            .post(CLOUDCODE_QUOTA_URL)
            .bearer_auth(access_token)
            .timeout(Duration::from_secs(10))
            .json(&json!({"project": self.oauth_tokens.project_id}));
        for (name, value) in antigravity_headers() {
            request = request.header(name, value);
        }
        let response = request.send().await.map_err(|error| {
            GoogleAntigravityQuotaRequestError::Other(format!(
                "Failed to request Google Antigravity quota: {error}"
            ))
        })?;
        if !response.status().is_success() {
            let status = response.status();
            let body: String = response
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(512)
                .collect();
            return Err(GoogleAntigravityQuotaRequestError::Status(status, body));
        }
        let payload = response
            .json::<serde_json::Value>()
            .await
            .map_err(|error| {
                GoogleAntigravityQuotaRequestError::Other(format!(
                    "Failed to parse Google Antigravity quota response: {error}"
                ))
            })?;
        Self::parse_quota_summary(&payload).map_err(GoogleAntigravityQuotaRequestError::Other)
    }

    pub async fn fetch_quota_summary(
        &self,
        http_client: &reqwest::Client,
    ) -> Result<GoogleAntigravityQuotaSummary, GoogleAntigravityQuotaRequestError> {
        self.fetch_quota_summary_once(http_client, &self.oauth_tokens.access_token)
            .await
    }

    pub fn quota_request_error_to_string(error: GoogleAntigravityQuotaRequestError) -> String {
        match error {
            GoogleAntigravityQuotaRequestError::Status(status, body) => {
                format!("Google Antigravity quota API returned {status}: {body}")
            }
            GoogleAntigravityQuotaRequestError::Other(message) => message,
        }
    }
}

#[async_trait]
impl ProviderTrait for GoogleAntigravityProvider {
    fn name(&self) -> &str {
        "google_antigravity"
    }

    fn display_name(&self) -> &str {
        "Google Antigravity (Gemini)"
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
        WireFormat::GoogleCloudCode
    }

    fn model_filter_regex(&self) -> Option<&'static str> {
        Some(r"^gemini-")
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields: {}
oauth:
  supported: true
  warning: "Google's terms prohibit accessing Antigravity / Gemini CLI backends from third-party
    clients, and Google has suspended accounts for doing so. Signing in here uses your Google account
    at your own risk. For a supported path, use the Google Gemini provider with an API key instead."
  methods:
    - id: subscription
      label: "Google Account (Antigravity)"
description: "Uses your Google account's Antigravity subscription instead of a Gemini API key."
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
            "oauth_tokens": {
                "access_token": if self.oauth_tokens.access_token.is_empty() { "" } else { "***" },
                "refresh_token": if self.oauth_tokens.refresh_token.is_empty() { "" } else { "***" },
                "expires_at": self.oauth_tokens.expires_at,
                "project_id": self.oauth_tokens.project_id,
            },
            "enabled": self.enabled,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models,
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let auth_token = self.oauth_tokens.access_token.clone();
        let mut extra_headers = HashMap::new();
        extra_headers.insert(
            CLOUDCODE_PROJECT_HEADER.to_string(),
            self.oauth_tokens.project_id.clone(),
        );
        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !auth_token.is_empty() && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: CLOUDCODE_BASE_URL.to_string(),
            completion_endpoint: String::new(),
            embedding_endpoint: String::new(),
            api_key: String::new(),
            auth_token,
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
        let fallback_models = || self.get_available_models_from_caps(model_caps);
        if self.oauth_tokens.access_token.is_empty() || self.oauth_tokens.project_id.is_empty() {
            return fallback_models();
        }

        let mut request = http_client
            .post(CLOUDCODE_MODELS_URL)
            .bearer_auth(&self.oauth_tokens.access_token)
            .json(&json!({"project": self.oauth_tokens.project_id}));
        for (name, value) in antigravity_headers() {
            request = request.header(name, value);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!("Google Antigravity: failed to fetch available models: {error}");
                return fallback_models();
            }
        };
        if !response.status().is_success() {
            tracing::warn!(
                "Google Antigravity: available models request returned status {}; using catalog fallback",
                response.status()
            );
            return fallback_models();
        }
        let response = match response.json::<serde_json::Value>().await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(
                    "Google Antigravity: failed to parse available models response: {error}"
                );
                return fallback_models();
            }
        };

        match self.available_models_from_live_response(&response, model_caps) {
            Some(models) if !models.is_empty() => models,
            _ => {
                tracing::warn!("Google Antigravity: available models response was empty or invalid; using catalog fallback");
                fallback_models()
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
        tracing::info!("Google Antigravity OAuth: refreshing token on startup");
        let mut refreshed = match crate::google_antigravity_oauth::refresh_access_token(
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
                    "Google Antigravity OAuth refresh token is invalid; clearing saved OAuth tokens: {}",
                    error
                );
                self.oauth_tokens = OAuthTokens::default();
                self.save_oauth_tokens_config(config_dir, instance_id)
                    .await?;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if refreshed.project_id.is_empty() {
            refreshed.project_id = self.oauth_tokens.project_id.clone();
        }
        self.oauth_tokens = refreshed;
        self.save_oauth_tokens_config(config_dir, instance_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn live_catalog_includes_models_not_present_in_models_dev() {
        let provider = GoogleAntigravityProvider {
            enabled_models: vec!["claude-opus-4.6-thinking".to_string()],
            ..Default::default()
        };
        let response = json!({
            "models": {
                "claude-opus-4.6-thinking": {"displayName": "Claude Opus 4.6"},
                "gemini-3.7-flash": {"displayName": "Gemini 3.7 Flash"}
            }
        });

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        assert_eq!(models.len(), 2);
        let claude = models
            .iter()
            .find(|model| model.id == "claude-opus-4.6-thinking")
            .unwrap();
        assert_eq!(claude.display_name.as_deref(), Some("Claude Opus 4.6"));
        assert!(claude.enabled);
        assert!(claude.supports_tools);
    }

    #[test]
    fn runtime_uses_the_antigravity_cli_backend() {
        let provider = GoogleAntigravityProvider {
            oauth_tokens: OAuthTokens {
                access_token: "oauth-access-token".to_string(),
                project_id: "project-id".to_string(),
                ..Default::default()
            },
            enabled: true,
            enabled_models: vec!["claude-sonnet-4-6".to_string()],
            ..Default::default()
        };

        let runtime = provider.build_runtime().unwrap();

        assert_eq!(
            runtime.chat_endpoint,
            "https://daily-cloudcode-pa.googleapis.com"
        );
        assert_eq!(runtime.auth_token, "oauth-access-token");
        assert_eq!(
            runtime.extra_headers.get(CLOUDCODE_PROJECT_HEADER),
            Some(&"project-id".to_string())
        );
    }

    #[test]
    fn parses_nested_quota_remaining_and_preserves_raw_payload() {
        let payload = json!({
            "description": "Subscription quota",
            "groups": [{
                "displayName": "Models",
                "buckets": [{
                    "bucketId": "gemini-pro",
                    "displayName": "Gemini Pro",
                    "remaining": {"remainingFraction": 0.42, "resetTime": "later"}
                }]
            }],
            "unknownTopLevel": {"kept": true}
        });
        let quota = GoogleAntigravityProvider::parse_quota_summary(&payload).unwrap();
        assert_eq!(quota.groups[0].buckets[0].remaining_fraction, Some(0.42));
        assert_eq!(quota.groups[0].buckets[0].reset.as_deref(), Some("later"));
        assert_eq!(quota.raw, payload);
    }

    #[test]
    fn parses_snake_case_aliases_and_flattened_remaining() {
        let payload = json!({
            "quota_groups": [{
                "display_name": "Requests",
                "quota_buckets": [{
                    "bucket_id": "daily",
                    "display_name": "Daily",
                    "remaining_fraction": 0.75
                }]
            }]
        });
        let quota = GoogleAntigravityProvider::parse_quota_summary(&payload).unwrap();
        assert_eq!(quota.groups[0].display_name, "Requests");
        assert_eq!(quota.groups[0].buckets[0].bucket_id, "daily");
        assert_eq!(quota.groups[0].buckets[0].remaining_fraction, Some(0.75));
    }

    #[test]
    fn tolerates_null_quota_labels() {
        let payload = json!({
            "groups": [{
                "displayName": null,
                "buckets": [{
                    "bucketId": null,
                    "displayName": null,
                    "remainingFraction": 0.5
                }]
            }]
        });

        let quota = GoogleAntigravityProvider::parse_quota_summary(&payload).unwrap();

        assert_eq!(quota.groups[0].display_name, "");
        assert_eq!(quota.groups[0].buckets[0].bucket_id, "");
        assert_eq!(quota.groups[0].buckets[0].display_name, "");
    }
}
