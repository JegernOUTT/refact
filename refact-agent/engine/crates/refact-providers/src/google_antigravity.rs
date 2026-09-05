use std::any::Any;
use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex as AMutex, MutexGuard};

use refact_core::antigravity_client::antigravity_headers;
use refact_core::model_caps::{resolve_model_caps, ModelCapabilities};
use refact_core::provider_types::{available_model_from_catalog_and_live, LiveModelFields};
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

/// `fetchAvailableModels` reports nothing about tool calling, so tool support cannot be
/// read from the API. Every agent model served by this backend accepts function calls,
/// so the value is asserted once here — explicitly, unconditionally and provider-wide —
/// instead of being guessed per model id or fabricated only when a catalog entry is missing.
const SUPPORTS_TOOLS: bool = true;

/// Antigravity has no prompt-caching API, so no request may carry cache markers.
/// Declared once here instead of repeating the literal at every construction site.
const SUPPORTS_CACHE_CONTROL: bool = false;

/// Top-level keys of the `fetchAvailableModels` payload that list model ids reserved for
/// non-agent roles (inline completion, image generation, transcription, ...). This is the
/// API's own structured signal for "not a chat model"; it replaces the old name regex.
const NON_AGENT_ROLE_ID_KEYS: &[&str] = &[
    "tabModelIds",
    "imageGenerationModelIds",
    "commandModelIds",
    "mqueryModelIds",
    "audioTranscriptionModelIds",
    "commitMessageModelIds",
    "webSearchModelIds",
];

/// Collects every model id the payload assigns to a non-agent role.
fn non_agent_role_model_ids(response: &serde_json::Value) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    for key in NON_AGENT_ROLE_ID_KEYS {
        let Some(list) = response.get(key).and_then(|value| value.as_array()) else {
            continue;
        };
        ids.extend(
            list.iter()
                .filter_map(|value| value.as_str())
                .filter(|id| !id.is_empty())
                .map(str::to_string),
        );
    }
    ids
}

/// `supportedMimeTypes` is a map of mime type -> bool; an explicit `false` means unsupported.
fn supported_mime_types(metadata: &serde_json::Value) -> Vec<&str> {
    metadata
        .get("supportedMimeTypes")
        .and_then(|value| value.as_object())
        .map(|mime_types| {
            mime_types
                .iter()
                .filter(|(_, supported)| supported.as_bool() != Some(false))
                .map(|(mime, _)| mime.as_str())
                .collect()
        })
        .unwrap_or_default()
}

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
        let authoritative_empty = response
            .get("authoritativeEmpty")
            .or_else(|| response.get("authoritative_empty"))
            .and_then(|value| value.as_bool())
            == Some(true);
        let non_agent_ids = non_agent_role_model_ids(response);
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
            // The API's own structured exclusion signals: internal-only models and models the
            // payload reserves for a non-agent role (tab completion, image generation, ...).
            if metadata
                .get("isInternal")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                continue;
            }
            if non_agent_ids.contains(id.as_str()) {
                continue;
            }
            let caps = resolve_model_caps(model_caps, &format!("google_antigravity/{id}"))
                .or_else(|| resolve_model_caps(model_caps, id))
                .map(|resolved| resolved.caps);

            // Absent `supportsThinking` means the model does not think; never inferred from the id.
            let supports_thinking = metadata
                .get("supportsThinking")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            // `thinkingBudget` == -1 means the budget is dynamic/adaptive, > 0 is a token ceiling.
            let thinking_budget = metadata
                .get("thinkingBudget")
                .and_then(|value| value.as_i64())
                .filter(|_| supports_thinking);
            let mime_types = supported_mime_types(metadata);
            let supports_images = metadata
                .get("supportsImages")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
                || mime_types.iter().any(|mime| mime.starts_with("image/"));

            let live = LiveModelFields {
                display_name: metadata
                    .get("displayName")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                // `maxTokens` is the context window; the API is its only source here.
                n_ctx: metadata
                    .get("maxTokens")
                    .and_then(|value| value.as_u64())
                    .filter(|value| *value > 0)
                    .map(|value| value as usize),
                max_output_tokens: metadata
                    .get("maxOutputTokens")
                    .and_then(|value| value.as_u64())
                    .filter(|value| *value > 0)
                    .map(|value| value as usize),
                supports_tools: Some(SUPPORTS_TOOLS),
                supports_parallel_tools: Some(SUPPORTS_TOOLS),
                supports_multimodality: Some(supports_images),
                supports_video: metadata
                    .get("supportsVideo")
                    .and_then(|value| value.as_bool())
                    .or_else(|| {
                        mime_types
                            .iter()
                            .any(|mime| mime.starts_with("video/"))
                            .then_some(true)
                    }),
                supports_audio: Some(mime_types.iter().any(|mime| mime.starts_with("audio/"))),
                supports_pdf: Some(mime_types.iter().any(|mime| *mime == "application/pdf")),
                supports_thinking_budget: Some(supports_thinking),
                supports_adaptive_thinking_budget: Some(thinking_budget == Some(-1)),
                max_thinking_tokens: thinking_budget
                    .filter(|budget| *budget > 0)
                    .map(|budget| budget as usize),
                min_thinking_budget: metadata
                    .get("minThinkingBudget")
                    .and_then(|value| value.as_u64())
                    .filter(|_| supports_thinking)
                    .map(|value| value as usize),
                supports_cache_control: Some(SUPPORTS_CACHE_CONTROL),
                base_model: metadata
                    .get("vertexModelId")
                    .or_else(|| metadata.get("model"))
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                upstream_provider: metadata
                    .get("apiProvider")
                    .or_else(|| metadata.get("modelProvider"))
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                ..Default::default()
            };
            let enabled = enabled_set.contains(id.as_str())
                || enabled_set.contains(format!("google_antigravity/{id}").as_str());
            // 0 as the unknown-context default: this provider never fabricates a context size.
            let available =
                available_model_from_catalog_and_live(id, caps.as_ref(), &live, enabled, 0);
            // Second line of defence behind the role lists: neither the live payload nor the
            // catalog supplied a context window, so the model cannot be sized honestly.
            // `gemini-3.1-flash-image` is exactly this case.
            if available.n_ctx == 0 {
                tracing::warn!(
                    "Google Antigravity: skipping model {id}: no context window (maxTokens) was reported"
                );
                continue;
            }
            available_models.push(available);
        }

        if available_models.is_empty() && !authoritative_empty {
            return None;
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
        // The model list comes from fetchAvailableModels, which states which models are
        // internal or reserved for a non-agent role. Nothing is filtered by name.
        None
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
            supports_cache_control: SUPPORTS_CACHE_CONTROL,
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

    fn get_available_models_from_caps(
        &self,
        _model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        // models.dev may only enrich ids that fetchAvailableModels returned; it may never
        // contribute one. Callers reach this path when the live fetch could not be completed
        // (for example the discovery timeout in caps.rs and providers/http.rs), which is
        // exactly when the catalog would invent ids this backend does not serve.
        self.get_custom_models_only()
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        // fetchAvailableModels is the only authority on which models this account can use.
        // When it cannot be consulted, the user's own custom models are all we can honestly
        // offer: a catalog fallback would invent ids this backend does not serve.
        let fallback_models = || self.get_custom_models_only();
        if self.oauth_tokens.access_token.is_empty() || self.oauth_tokens.project_id.is_empty() {
            tracing::warn!(
                "Google Antigravity: no OAuth credentials; returning custom models only"
            );
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
                "Google Antigravity: available models request returned status {}; returning custom models only",
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
            Some(models) => models,
            _ => {
                tracing::warn!("Google Antigravity: available models response was empty or invalid; returning custom models only");
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

    /// A record shaped like a real `fetchAvailableModels` entry: the payload is a map of
    /// model id -> record, and every real record carries `maxTokens`.
    fn live_model(id: &str, extra: serde_json::Value) -> (String, serde_json::Value) {
        let mut record = json!({
            "displayName": id,
            "maxTokens": 200_000,
            "maxOutputTokens": 8_192
        });
        merge_object(&mut record, &extra);
        (id.to_string(), record)
    }

    fn merge_object(target: &mut serde_json::Value, extra: &serde_json::Value) {
        let (Some(target), Some(extra)) = (target.as_object_mut(), extra.as_object()) else {
            return;
        };
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }

    fn live_response(
        models: Vec<(String, serde_json::Value)>,
        extra: serde_json::Value,
    ) -> serde_json::Value {
        let mut model_map = serde_json::Map::new();
        for (id, record) in models {
            model_map.insert(id, record);
        }
        let mut response = json!({});
        merge_object(&mut response, &extra);
        response
            .as_object_mut()
            .unwrap()
            .insert("models".to_string(), serde_json::Value::Object(model_map));
        response
    }

    #[test]
    fn context_window_and_output_limit_come_from_the_api() {
        let provider = GoogleAntigravityProvider::default();
        let response = live_response(
            vec![live_model(
                "gemini-3-pro",
                json!({"maxTokens": 1_048_576, "maxOutputTokens": 65_536}),
            )],
            json!({}),
        );

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].n_ctx, 1_048_576);
        assert_eq!(models[0].max_output_tokens, Some(65_536));
        assert_eq!(models[0].live_fields.n_ctx, Some(1_048_576));
    }

    #[test]
    fn catalog_context_never_overrides_the_api_context() {
        let provider = GoogleAntigravityProvider::default();
        let response = live_response(
            vec![live_model("gemini-3-pro", json!({"maxTokens": 16_384}))],
            json!({}),
        );
        let caps = HashMap::from([(
            "google_antigravity/gemini-3-pro".to_string(),
            ModelCapabilities {
                n_ctx: 128_000,
                ..Default::default()
            },
        )]);

        let models = provider
            .available_models_from_live_response(&response, &caps)
            .unwrap();

        assert_eq!(models[0].n_ctx, 16_384);
    }

    #[test]
    fn internal_models_and_role_reserved_ids_are_excluded() {
        let provider = GoogleAntigravityProvider::default();
        let response = live_response(
            vec![
                live_model("gemini-3-pro", json!({})),
                live_model("gpt-oss-120b-medium", json!({"maxTokens": 131_072})),
                live_model("chat_20706", json!({"isInternal": true})),
                live_model("chat_23310", json!({"isInternal": true})),
                live_model("tab_flash_lite_preview", json!({})),
                live_model("tab_jump_flash_lite_preview", json!({})),
                live_model("gemini-3.1-flash-image", json!({})),
            ],
            json!({
                "tabModelIds": ["tab_flash_lite_preview", "tab_jump_flash_lite_preview"],
                "imageGenerationModelIds": ["gemini-3.1-flash-image"],
                "commandModelIds": [],
                "mqueryModelIds": [],
                "audioTranscriptionModelIds": [],
                "commitMessageModelIds": [],
                "webSearchModelIds": []
            }),
        );

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, vec!["gemini-3-pro", "gpt-oss-120b-medium"]);
    }

    #[test]
    fn a_model_without_a_context_window_is_skipped_rather_than_given_a_default() {
        let provider = GoogleAntigravityProvider::default();
        // No maxTokens at all, exactly like the live `gemini-3.1-flash-image` record.
        let record = json!({"displayName": "Flash Image"});
        let response = json!({
            "models": {
                "gemini-3.1-flash-image": record,
                "gemini-3-pro": {"displayName": "Pro", "maxTokens": 200_000}
            }
        });

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gemini-3-pro");
    }

    #[test]
    fn adaptive_thinking_budget_comes_from_a_negative_thinking_budget() {
        let provider = GoogleAntigravityProvider::default();
        let response = live_response(
            vec![
                live_model(
                    "gemini-3-pro-adaptive",
                    json!({
                        "supportsThinking": true,
                        "thinkingBudget": -1,
                        "minThinkingBudget": 128
                    }),
                ),
                live_model(
                    "gemini-3-pro-fixed",
                    json!({
                        "supportsThinking": true,
                        "thinkingBudget": 8_192,
                        "minThinkingBudget": 32
                    }),
                ),
            ],
            json!({}),
        );

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        let adaptive = models
            .iter()
            .find(|model| model.id == "gemini-3-pro-adaptive")
            .unwrap();
        assert!(adaptive.supports_thinking_budget);
        assert!(adaptive.supports_adaptive_thinking_budget);
        assert_eq!(adaptive.max_thinking_tokens, None);
        assert_eq!(adaptive.min_thinking_budget, Some(128));

        let fixed = models
            .iter()
            .find(|model| model.id == "gemini-3-pro-fixed")
            .unwrap();
        assert!(fixed.supports_thinking_budget);
        assert!(!fixed.supports_adaptive_thinking_budget);
        assert_eq!(fixed.max_thinking_tokens, Some(8_192));
        assert_eq!(fixed.min_thinking_budget, Some(32));
    }

    #[test]
    fn absent_supports_thinking_is_false_and_is_never_guessed_from_the_id() {
        let provider = GoogleAntigravityProvider::default();
        let response = live_response(
            vec![
                live_model("gemini-3-flash", json!({})),
                live_model("claude-sonnet-4-6", json!({})),
            ],
            json!({}),
        );
        let caps = HashMap::from([(
            "google_antigravity/claude-sonnet-4-6".to_string(),
            ModelCapabilities {
                n_ctx: 128_000,
                supports_thinking_budget: true,
                supports_adaptive_thinking_budget: true,
                ..Default::default()
            },
        )]);

        let models = provider
            .available_models_from_live_response(&response, &caps)
            .unwrap();

        for model in &models {
            assert!(
                !model.supports_thinking_budget,
                "{} must not claim thinking support",
                model.id
            );
            assert!(!model.supports_adaptive_thinking_budget);
        }
    }

    #[test]
    fn modality_flags_come_from_supports_images_and_supported_mime_types() {
        let provider = GoogleAntigravityProvider::default();
        let response = live_response(
            vec![
                live_model(
                    "gemini-3-pro",
                    json!({
                        "supportsImages": true,
                        "supportsVideo": true,
                        "supportedMimeTypes": {
                            "image/png": true,
                            "video/mp4": true,
                            "audio/mpeg": true,
                            "application/pdf": true
                        }
                    }),
                ),
                live_model("gpt-oss-120b-medium", json!({"supportedMimeTypes": {}})),
            ],
            json!({}),
        );

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        let multimodal = models
            .iter()
            .find(|model| model.id == "gemini-3-pro")
            .unwrap();
        assert!(multimodal.supports_multimodality);
        assert!(multimodal.supports_video);
        assert!(multimodal.supports_audio);
        assert!(multimodal.supports_pdf);

        let text_only = models
            .iter()
            .find(|model| model.id == "gpt-oss-120b-medium")
            .unwrap();
        assert!(!text_only.supports_multimodality);
        assert!(!text_only.supports_audio);
        assert!(!text_only.supports_pdf);
    }

    #[test]
    fn base_model_and_upstream_provider_come_from_the_api() {
        let provider = GoogleAntigravityProvider::default();
        let response = live_response(
            vec![
                live_model(
                    "gemini-3-pro",
                    json!({
                        "vertexModelId": "gemini-3-pro-preview-11-2025",
                        "model": "gemini-3-pro",
                        "apiProvider": "GOOGLE",
                        "modelProvider": "GEMINI"
                    }),
                ),
                live_model(
                    "claude-sonnet-4-6",
                    json!({"model": "claude-sonnet-4-6", "modelProvider": "ANTHROPIC"}),
                ),
            ],
            json!({}),
        );

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        let gemini = models
            .iter()
            .find(|model| model.id == "gemini-3-pro")
            .unwrap();
        assert_eq!(
            gemini.base_model.as_deref(),
            Some("gemini-3-pro-preview-11-2025")
        );
        assert_eq!(gemini.upstream_provider.as_deref(), Some("GOOGLE"));

        let claude = models
            .iter()
            .find(|model| model.id == "claude-sonnet-4-6")
            .unwrap();
        assert_eq!(claude.base_model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(claude.upstream_provider.as_deref(), Some("ANTHROPIC"));
    }

    #[test]
    fn tool_support_and_cache_control_are_provider_level_constants() {
        let provider = GoogleAntigravityProvider::default();
        let response = live_response(vec![live_model("gemini-3-pro", json!({}))], json!({}));
        let caps = HashMap::from([(
            "google_antigravity/gemini-3-pro".to_string(),
            ModelCapabilities {
                n_ctx: 128_000,
                supports_tools: false,
                supports_cache_control: true,
                ..Default::default()
            },
        )]);

        let models = provider
            .available_models_from_live_response(&response, &caps)
            .unwrap();

        assert_eq!(models[0].supports_tools, SUPPORTS_TOOLS);
        assert_eq!(models[0].supports_parallel_tools, SUPPORTS_TOOLS);
        assert_eq!(models[0].supports_cache_control, SUPPORTS_CACHE_CONTROL);
    }

    #[test]
    fn enabled_models_match_bare_and_provider_qualified_ids() {
        let provider = GoogleAntigravityProvider {
            enabled_models: vec![
                "gemini-3-pro".to_string(),
                "google_antigravity/claude-sonnet-4-6".to_string(),
            ],
            ..Default::default()
        };
        let response = live_response(
            vec![
                live_model("gemini-3-pro", json!({})),
                live_model("claude-sonnet-4-6", json!({})),
                live_model("gemini-3-flash", json!({})),
            ],
            json!({}),
        );

        let models = provider
            .available_models_from_live_response(&response, &HashMap::new())
            .unwrap();

        let enabled: Vec<&str> = models
            .iter()
            .filter(|model| model.enabled)
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(enabled, vec!["claude-sonnet-4-6", "gemini-3-pro"]);
    }

    #[test]
    fn empty_live_catalog_signals_failure_unless_marked_authoritative() {
        let provider = GoogleAntigravityProvider::default();

        assert!(provider
            .available_models_from_live_response(&json!({"models": {}}), &HashMap::new())
            .is_none());

        let models = provider
            .available_models_from_live_response(
                &json!({"models": {}, "authoritativeEmpty": true}),
                &HashMap::new(),
            )
            .unwrap();
        assert!(models.is_empty());
    }

    #[test]
    fn failure_path_returns_custom_models_only_and_never_catalog_ids() {
        let provider = GoogleAntigravityProvider {
            custom_models: HashMap::from([(
                "my-own-model".to_string(),
                CustomModelConfig {
                    n_ctx: Some(64_000),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        let caps = HashMap::from([(
            "google_antigravity/gemini-catalog-phantom".to_string(),
            ModelCapabilities {
                n_ctx: 128_000,
                ..Default::default()
            },
        )]);

        // The caps-only path (discovery timeout in caps.rs / providers/http.rs) must not
        // contribute an id the live API never returned.
        let models = provider.get_available_models_from_caps(&caps);

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "my-own-model");
        assert_eq!(models[0].n_ctx, 64_000);
        assert!(!models
            .iter()
            .any(|model| model.id.contains("gemini-catalog-phantom")));
    }

    #[test]
    fn no_name_based_model_filter_is_applied() {
        assert!(GoogleAntigravityProvider::default()
            .model_filter_regex()
            .is_none());
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
        assert_eq!(
            CLOUDCODE_MODELS_URL,
            "https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels"
        );
        assert_eq!(
            CLOUDCODE_QUOTA_URL,
            "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary"
        );
        assert_eq!(runtime.auth_token, "oauth-access-token");
        assert_eq!(runtime.supports_cache_control, SUPPORTS_CACHE_CONTROL);
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
