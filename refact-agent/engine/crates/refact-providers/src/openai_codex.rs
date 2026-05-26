use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex as AMutex, MutexGuard};

use refact_core::model_caps::ModelCapabilities;
use refact_core::llm_types::WireFormat;
use crate::openai_codex_oauth::OAuthTokens;
use crate::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};

const CODEX_ORIGINATOR: &str = "refact-lsp";
const CHATGPT_CODEX_MODELS_URL: &str =
    "https://chatgpt.com/backend-api/codex/models?client_version=999.999.999";
const CHATGPT_CODEX_RESPONSES_WEBSOCKET_URL: &str = "wss://chatgpt.com/backend-api/codex/responses";
pub const CODEX_WEBSOCKET_ENDPOINT_HEADER: &str =
    "x-refact-internal-openai-codex-websocket-endpoint";
#[allow(dead_code)]
const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";

lazy_static::lazy_static! {
    static ref OPENAI_CODEX_REFRESH_GUARD: AMutex<()> = AMutex::new(());
}

fn new_codex_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn normalized_model_id(id: &str) -> String {
    id.trim().to_ascii_lowercase().replace('_', "-")
}

fn is_codex_named_model(id: &str) -> bool {
    let normalized = normalized_model_id(id);
    let parts: Vec<&str> = normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < 3 || parts.first() != Some(&"gpt") {
        return false;
    }
    let Some(codex_index) = parts.iter().position(|part| *part == "codex") else {
        return false;
    };
    if codex_index < 2 {
        return false;
    }
    let suffixes = &parts[codex_index + 1..];
    suffixes.is_empty()
        || (suffixes.len() == 1
            && matches!(suffixes[0], "latest" | "preview" | "mini" | "spark" | "max"))
}

fn is_gpt5_subscription_model(id: &str) -> bool {
    let normalized = normalized_model_id(id);
    if normalized == "gpt-5" {
        return true;
    }
    let Some(rest) = normalized.strip_prefix("gpt-5.") else {
        return false;
    };
    let mut parts = rest.split('-');
    let Some(version) = parts.next() else {
        return false;
    };
    if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let suffixes: Vec<&str> = parts.collect();
    matches!(suffixes.as_slice(), [] | ["mini"])
}

fn is_openai_codex_catalog_model(id: &str) -> bool {
    is_codex_named_model(id) || is_gpt5_subscription_model(id)
}

fn is_chatgpt_codex_live_model(id: &str) -> bool {
    is_openai_codex_catalog_model(id)
}

#[allow(dead_code)]
fn is_openai_api_codex_live_model(id: &str) -> bool {
    is_codex_named_model(id)
}

fn openai_codex_catalog_model_id(capability_key: &str) -> Option<&str> {
    ["openai/", "openai-codex/", "openai_codex/"]
        .iter()
        .find_map(|prefix| capability_key.strip_prefix(prefix))
}

fn resolve_openai_codex_catalog_caps<'a>(
    model_caps: &'a HashMap<String, ModelCapabilities>,
    model_id: &str,
) -> Option<&'a ModelCapabilities> {
    ["openai", "openai-codex", "openai_codex"]
        .iter()
        .find_map(|provider| model_caps.get(&format!("{provider}/{model_id}")))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AuthSource {
    InAppOAuth,
    None,
}

#[derive(Debug, Clone)]
enum CodexAuth {
    /// Subscription-only path: ChatGPT backend OAuth.
    ChatGptBackendOAuth {
        access_token: String,
        chatgpt_account_id: String,
    },
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAICodexProvider {
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
    #[serde(default)]
    pub oauth_tokens: OAuthTokens,
    #[serde(default = "new_codex_session_id")]
    pub session_id: String,
    #[serde(default)]
    pub use_websocket: bool,
}

impl Default for OpenAICodexProvider {
    fn default() -> Self {
        Self {
            enabled_models: Vec::new(),
            custom_models: HashMap::new(),
            oauth_tokens: OAuthTokens::default(),
            session_id: new_codex_session_id(),
            use_websocket: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexUsageWindow {
    pub used_percent: f64,
    pub reset_at: Option<String>,
    pub limit_window_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexRateLimit {
    pub limit_reached: bool,
    pub primary_window: Option<OpenAICodexUsageWindow>,
    pub secondary_window: Option<OpenAICodexUsageWindow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexCredits {
    pub balance: f64,
    pub unlimited: bool,
    pub has_credits: bool,
    pub granted: Option<f64>,
    pub used: Option<f64>,
    pub reset_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexUsage {
    pub plan_type: Option<String>,
    pub rate_limit: Option<OpenAICodexRateLimit>,
    pub code_review_rate_limit: Option<OpenAICodexRateLimit>,
    pub credits: Option<OpenAICodexCredits>,
}

pub enum UsageRequestError {
    Status(reqwest::StatusCode, String),
    Other(String),
}

pub struct WhamContext {
    pub access_token: String,
    pub chatgpt_account_id: String,
    pub source: AuthSource,
}

impl OpenAICodexProvider {
    pub async fn lock_refresh_guard() -> Result<MutexGuard<'static, ()>, String> {
        tokio::time::timeout(Duration::from_secs(30), OPENAI_CODEX_REFRESH_GUARD.lock())
            .await
            .map_err(|_| "OpenAI Codex OAuth refresh guard timed out".to_string())
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
        let session_id = self.session_id.clone();
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
            tokens_map.insert(
                serde_yaml::Value::String("chatgpt_account_id".to_string()),
                serde_yaml::Value::String(tokens.chatgpt_account_id),
            );

            yaml_map.insert(
                serde_yaml::Value::String("oauth_tokens".to_string()),
                serde_yaml::Value::Mapping(tokens_map),
            );
            // Subscription-only: never persist OPENAI_API_KEY at the provider level.
            yaml_map.remove(serde_yaml::Value::String("OPENAI_API_KEY".to_string()));
            yaml_map.insert(
                serde_yaml::Value::String("session_id".to_string()),
                serde_yaml::Value::String(session_id),
            );

            Ok(serde_yaml::Value::Mapping(yaml_map))
        })
        .await
        .map(|_| ())
    }

    fn resolve_auth(&self) -> (AuthSource, CodexAuth) {
        if self.oauth_tokens.has_valid_access_token() {
            return (
                AuthSource::InAppOAuth,
                CodexAuth::ChatGptBackendOAuth {
                    access_token: self.oauth_tokens.access_token.clone(),
                    chatgpt_account_id: self.oauth_tokens.chatgpt_account_id.clone(),
                },
            );
        }

        (AuthSource::None, CodexAuth::None)
    }

    pub fn resolve_wham_context(&self) -> Result<WhamContext, String> {
        if self.oauth_tokens.has_valid_access_token() {
            return Ok(WhamContext {
                access_token: self.oauth_tokens.access_token.clone(),
                chatgpt_account_id: self.oauth_tokens.chatgpt_account_id.clone(),
                source: AuthSource::InAppOAuth,
            });
        }
        if !self.oauth_tokens.access_token.is_empty() && self.oauth_tokens.has_refresh_token() {
            return Err(
                "OpenAI Codex usage access token is expired. Retry after provider refresh or log in again in OpenAI Codex provider settings."
                    .to_string(),
            );
        }
        Err(
            "OpenAI Codex: not logged in for this provider instance. Log in in OpenAI Codex provider settings."
                .to_string(),
        )
    }

    pub async fn fetch_usage_once(
        &self,
        http_client: &reqwest::Client,
        token: &str,
        chatgpt_account_id: &str,
    ) -> Result<OpenAICodexUsage, UsageRequestError> {
        let mut req = http_client
            .get("https://chatgpt.com/backend-api/wham/usage")
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json");
        for (key, value) in self.chatgpt_backend_metadata_headers(chatgpt_account_id) {
            req = req.header(key, value);
        }
        let resp = req.send().await.map_err(|e| {
            UsageRequestError::Other(format!("OpenAI Codex usage request failed: {}", e))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(UsageRequestError::Status(status, body));
        }

        let root: Value = resp.json().await.map_err(|e| {
            UsageRequestError::Other(format!(
                "Failed to parse OpenAI Codex usage response: {}",
                e
            ))
        })?;

        Ok(Self::parse_usage_payload(&root))
    }

    pub fn usage_request_error_to_string(error: UsageRequestError, _source: AuthSource) -> String {
        match error {
            UsageRequestError::Status(status, body) => {
                let truncated: String = body.chars().take(512).collect();
                format!(
                    "OpenAI Codex usage API returned {}. Check OpenAI Codex login/setup in provider settings or log in again: {}",
                    status, truncated
                )
            }
            UsageRequestError::Other(error) => error,
        }
    }

    pub async fn fetch_usage(
        &self,
        http_client: &reqwest::Client,
    ) -> Result<OpenAICodexUsage, String> {
        let context = self.resolve_wham_context()?;
        self.fetch_usage_once(
            http_client,
            &context.access_token,
            &context.chatgpt_account_id,
        )
        .await
        .map_err(|error| Self::usage_request_error_to_string(error, context.source))
    }

    fn parse_usage_payload(root: &Value) -> OpenAICodexUsage {
        let data = root.get("data").unwrap_or(root);
        let plan_type = Self::string_field(data, &["plan_type", "planType", "codex_plan_type"])
            .or_else(|| {
                data.get("plan")
                    .and_then(|plan| Self::string_field(plan, &["type", "name"]))
            });
        let rate_limit =
            Self::field(data, &["rate_limit", "rateLimit"]).map(Self::parse_rate_limit);
        let code_review_rate_limit = Self::field(
            data,
            &[
                "code_review_rate_limit",
                "codeReviewRateLimit",
                "code_review",
            ],
        )
        .map(Self::parse_rate_limit);
        let credits = Self::field(data, &["credits", "credit_balance"]).map(Self::parse_credits);
        OpenAICodexUsage {
            plan_type,
            rate_limit,
            code_review_rate_limit,
            credits,
        }
    }

    fn parse_rate_limit(rl: &Value) -> OpenAICodexRateLimit {
        let primary_window =
            Self::field(rl, &["primary_window", "primary"]).and_then(Self::parse_usage_window);
        let secondary_window =
            Self::field(rl, &["secondary_window", "secondary"]).and_then(Self::parse_usage_window);
        let limit_reached = Self::field(rl, &["limit_reached", "limitReached"])
            .and_then(Value::as_bool)
            .unwrap_or_else(|| {
                primary_window
                    .as_ref()
                    .map(|window| window.used_percent >= 100.0)
                    .unwrap_or(false)
                    || secondary_window
                        .as_ref()
                        .map(|window| window.used_percent >= 100.0)
                        .unwrap_or(false)
            });
        OpenAICodexRateLimit {
            limit_reached,
            primary_window,
            secondary_window,
        }
    }

    fn parse_usage_window(obj: &Value) -> Option<OpenAICodexUsageWindow> {
        let used_percent = Self::field(obj, &["used_percent", "usedPercent"])
            .and_then(Self::as_f64_loose)
            .map(|value| {
                if value.is_finite() {
                    value.max(0.0).min(100.0)
                } else {
                    0.0
                }
            })?;
        let reset_at = Self::field(obj, &["reset_at", "resets_at", "resetsAt"])
            .and_then(Self::timestamp_or_string);
        let limit_window_seconds = Self::field(
            obj,
            &[
                "limit_window_seconds",
                "limitWindowSeconds",
                "window_seconds",
            ],
        )
        .and_then(Self::as_u64_loose);
        Some(OpenAICodexUsageWindow {
            used_percent,
            reset_at,
            limit_window_seconds,
        })
    }

    fn parse_credits(c: &Value) -> OpenAICodexCredits {
        let balance = Self::field(c, &["balance", "remaining", "remaining_credits"])
            .and_then(Self::as_f64_loose)
            .unwrap_or(0.0);
        OpenAICodexCredits {
            balance,
            unlimited: Self::field(c, &["unlimited", "is_unlimited"])
                .and_then(Value::as_bool)
                .unwrap_or(false),
            has_credits: Self::field(c, &["has_credits", "hasCredits"])
                .and_then(Value::as_bool)
                .unwrap_or(balance > 0.0),
            granted: Self::field(c, &["granted", "total_granted", "total"])
                .and_then(Self::as_f64_loose),
            used: Self::field(c, &["used", "total_used"]).and_then(Self::as_f64_loose),
            reset_at: Self::field(c, &["reset_at", "expires_at", "expiresAt"])
                .and_then(Self::timestamp_or_string),
        }
    }

    fn field<'a>(obj: &'a Value, keys: &[&str]) -> Option<&'a Value> {
        keys.iter().find_map(|key| obj.get(*key))
    }

    fn string_field(obj: &Value, keys: &[&str]) -> Option<String> {
        Self::field(obj, keys).and_then(|value| value.as_str().map(ToString::to_string))
    }

    fn as_f64_loose(v: &Value) -> Option<f64> {
        v.as_f64()
            .or_else(|| v.as_i64().map(|i| i as f64))
            .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
    }

    fn as_u64_loose(v: &Value) -> Option<u64> {
        v.as_u64()
            .or_else(|| v.as_i64().and_then(|i| (i >= 0).then_some(i as u64)))
            .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
    }

    fn timestamp_or_string(v: &Value) -> Option<String> {
        if let Some(s) = v.as_str() {
            return Some(s.to_string());
        }
        let ts = v.as_i64()?;
        if ts < 0 {
            return None;
        }
        let secs = if ts > 1_000_000_000_000 {
            ts / 1000
        } else {
            ts
        };
        use std::time::{Duration, UNIX_EPOCH};
        let dt: chrono::DateTime<chrono::Utc> =
            (UNIX_EPOCH + Duration::from_secs(secs as u64)).into();
        Some(dt.to_rfc3339())
    }

    fn diagnose_auth_status(&self) -> String {
        if self.oauth_tokens.has_valid_access_token() {
            if self.oauth_tokens.chatgpt_account_id.is_empty() {
                return "OAuth login incomplete: missing chatgpt_account_id".to_string();
            }
            return "Connected (ChatGPT backend)".to_string();
        }
        if !self.oauth_tokens.is_empty() && self.oauth_tokens.has_refresh_token() {
            return "OAuth token expired — needs refresh".to_string();
        }
        "Not configured — log in via OAuth".to_string()
    }

    fn chatgpt_backend_metadata_headers(
        &self,
        chatgpt_account_id: &str,
    ) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        if !chatgpt_account_id.is_empty() {
            headers.insert(
                "chatgpt-account-id".to_string(),
                chatgpt_account_id.to_string(),
            );
        }
        headers.insert("originator".to_string(), CODEX_ORIGINATOR.to_string());
        headers.insert("session_id".to_string(), self.session_id.clone());
        headers
    }

    fn chatgpt_backend_streaming_headers(
        &self,
        chatgpt_account_id: &str,
    ) -> HashMap<String, String> {
        let mut headers = self.chatgpt_backend_metadata_headers(chatgpt_account_id);
        headers.insert(
            "OpenAI-Beta".to_string(),
            "responses=experimental".to_string(),
        );
        headers.insert("accept".to_string(), "text/event-stream".to_string());
        headers
    }

    async fn fetch_models_from_chatgpt_api(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
        access_token: &str,
        chatgpt_account_id: &str,
    ) -> Vec<AvailableModel> {
        let mut req = http_client.get(CHATGPT_CODEX_MODELS_URL).header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {access_token}"),
        );
        for (key, value) in self.chatgpt_backend_metadata_headers(chatgpt_account_id) {
            req = req.header(key, value);
        }

        let response = match req.send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("OpenAI Codex: failed to reach chatgpt backend /codex/models (network error): {}, using models.dev catalog fallback", e);
                return self.fetch_models_from_catalog(model_caps);
            }
        };

        let status = response.status();

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            tracing::warn!("OpenAI Codex: /codex/models returned {}. Refresh will be attempted on runtime 401/403; using models.dev catalog fallback for model list", status);
            return self.fetch_models_from_catalog(model_caps);
        }

        if !status.is_success() {
            tracing::warn!(
                "OpenAI Codex: /codex/models returned {} (transient), using models.dev catalog fallback",
                status
            );
            return self.fetch_models_from_catalog(model_caps);
        }

        let json: Value = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("OpenAI Codex: failed to parse /codex/models response: {}, using models.dev catalog fallback", e);
                return self.fetch_models_from_catalog(model_caps);
            }
        };

        let Some(models_array) = Self::models_array_from_live_response(&json) else {
            tracing::warn!("OpenAI Codex: /codex/models response missing a model array, using models.dev catalog fallback");
            return self.fetch_models_from_catalog(model_caps);
        };

        let models = self.available_models_from_live_chatgpt_models(models_array, model_caps);

        tracing::info!(
            "OpenAI Codex: {} models available (chatgpt backend + models.dev catalog)",
            models.len()
        );

        models
    }

    #[allow(dead_code)]
    async fn fetch_models_from_api(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
        api_key: &str,
    ) -> Vec<AvailableModel> {
        let response = match http_client
            .get(OPENAI_MODELS_URL)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("OpenAI Codex: failed to reach /v1/models (network error): {}, using models.dev catalog fallback", e);
                return self.fetch_models_from_catalog(model_caps);
            }
        };

        let status = response.status();

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            tracing::warn!("OpenAI Codex: /v1/models returned {}. Check OpenAI Codex provider setup or API key exchange; returning custom models only", status);
            return self.get_custom_models_only();
        }

        if !status.is_success() {
            tracing::warn!(
                "OpenAI Codex: /v1/models returned {} (transient), using models.dev catalog fallback",
                status
            );
            return self.fetch_models_from_catalog(model_caps);
        }

        let json: Value = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "OpenAI Codex: failed to parse /v1/models response: {}, using models.dev catalog fallback",
                    e
                );
                return self.fetch_models_from_catalog(model_caps);
            }
        };

        let Some(data_array) = Self::models_array_from_live_response(&json) else {
            tracing::warn!("OpenAI Codex: /v1/models response missing a model array, using models.dev catalog fallback");
            return self.fetch_models_from_catalog(model_caps);
        };

        let enabled_set: HashSet<&str> = self.enabled_models.iter().map(|s| s.as_str()).collect();
        let mut models_map = self.catalog_model_map(model_caps, &enabled_set);

        for model in data_array {
            let Some(id) = Self::live_model_id(model) else {
                continue;
            };
            if !is_openai_api_codex_live_model(id) || !Self::live_model_is_supported(model) {
                continue;
            }
            let enabled = enabled_set.contains(id);
            let pricing = self.custom_model_pricing(id);
            let mut available =
                if let Some(caps) = resolve_openai_codex_catalog_caps(model_caps, id) {
                    AvailableModel::from_caps(id, caps, enabled, pricing)
                } else {
                    self.unknown_live_codex_model(id.to_string(), enabled, pricing, model)
                };
            available.display_name =
                Self::live_model_display_name(model).or(available.display_name);
            models_map.insert(id.to_string(), available);
        }

        tracing::info!(
            "OpenAI Codex: {} models available (/v1/models + models.dev catalog)",
            models_map.len()
        );

        self.finish_models(models_map, &enabled_set)
    }

    fn fetch_models_from_catalog(
        &self,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let enabled_set: HashSet<&str> = self.enabled_models.iter().map(|s| s.as_str()).collect();
        let models_map = self.catalog_model_map(model_caps, &enabled_set);
        tracing::info!(
            "OpenAI Codex: {} models available (models.dev catalog fallback)",
            models_map.len()
        );
        self.finish_models(models_map, &enabled_set)
    }

    fn catalog_model_map(
        &self,
        model_caps: &HashMap<String, ModelCapabilities>,
        enabled_set: &HashSet<&str>,
    ) -> HashMap<String, AvailableModel> {
        let mut models_map: HashMap<String, AvailableModel> = HashMap::new();
        for (capability_key, caps) in model_caps {
            let Some(model_id) = openai_codex_catalog_model_id(capability_key) else {
                continue;
            };
            if !is_openai_codex_catalog_model(model_id) {
                continue;
            }
            let enabled =
                enabled_set.contains(model_id) || enabled_set.contains(capability_key.as_str());
            let pricing = self
                .custom_model_pricing(model_id)
                .or_else(|| self.custom_model_pricing(capability_key));
            models_map.insert(
                model_id.to_string(),
                AvailableModel::from_caps(model_id, caps, enabled, pricing),
            );
        }
        models_map
    }

    fn available_models_from_live_chatgpt_models(
        &self,
        models_array: &[Value],
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let enabled_set: HashSet<&str> = self.enabled_models.iter().map(|s| s.as_str()).collect();
        let mut models_map = self.catalog_model_map(model_caps, &enabled_set);

        for model in models_array {
            let Some(slug) = Self::live_model_id(model) else {
                continue;
            };
            if !is_chatgpt_codex_live_model(slug) || !Self::live_model_is_supported(model) {
                continue;
            }
            let enabled = enabled_set.contains(slug);
            let pricing = self.custom_model_pricing(slug);
            let display_name = Self::live_model_display_name(model);
            let mut available =
                if let Some(caps) = resolve_openai_codex_catalog_caps(model_caps, slug) {
                    AvailableModel::from_caps(slug, caps, enabled, pricing)
                } else {
                    self.unknown_live_codex_model(slug.to_string(), enabled, pricing, model)
                };
            available.display_name = display_name.or(available.display_name);
            models_map.insert(slug.to_string(), available);
        }

        self.finish_models(models_map, &enabled_set)
    }

    fn finish_models(
        &self,
        mut models_map: HashMap<String, AvailableModel>,
        enabled_set: &HashSet<&str>,
    ) -> Vec<AvailableModel> {
        let mut models: Vec<AvailableModel> = models_map.drain().map(|(_, model)| model).collect();
        merge_custom_models(&mut models, &self.custom_models, enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }

    fn models_array_from_live_response(json: &Value) -> Option<&Vec<Value>> {
        json.get("models")
            .or_else(|| json.get("data"))
            .and_then(Value::as_array)
    }

    fn live_model_id(model: &Value) -> Option<&str> {
        model
            .get("slug")
            .or_else(|| model.get("id"))
            .or_else(|| model.get("model"))
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
    }

    fn live_model_display_name(model: &Value) -> Option<String> {
        model
            .get("display_name")
            .or_else(|| model.get("displayName"))
            .or_else(|| model.get("name"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
    }

    fn live_model_context_window(model: &Value) -> Option<usize> {
        model
            .get("max_context_window")
            .or_else(|| model.get("context_window"))
            .or_else(|| model.get("contextWindow"))
            .and_then(Value::as_u64)
            .map(|v| v as usize)
    }

    fn live_model_max_output_tokens(model: &Value) -> Option<usize> {
        model
            .get("max_output_tokens")
            .or_else(|| model.get("maxOutputTokens"))
            .and_then(Value::as_u64)
            .map(|v| v as usize)
    }

    fn live_bool_field(model: &Value, keys: &[&str]) -> Option<bool> {
        keys.iter()
            .find_map(|key| model.get(*key).and_then(Value::as_bool))
    }

    fn live_status_is_disabled(model: &Value) -> bool {
        [
            "status",
            "availability",
            "access",
            "access_status",
            "entitlement",
            "entitlement_status",
            "policy",
        ]
        .iter()
        .filter_map(|key| model.get(*key).and_then(Value::as_str))
        .any(|status| {
            let normalized = status
                .trim()
                .to_ascii_lowercase()
                .replace('-', "_")
                .replace(' ', "_");
            matches!(
                normalized.as_str(),
                "disabled"
                    | "unsupported"
                    | "not_supported"
                    | "unavailable"
                    | "not_available"
                    | "retired"
                    | "deprecated"
                    | "restricted"
                    | "policy_disabled"
                    | "policy_restricted"
                    | "policy_denied"
                    | "not_entitled"
                    | "entitlement_required"
                    | "denied"
                    | "not_allowed"
                    | "disabled_by_policy"
            )
        })
    }

    fn live_model_is_supported(model: &Value) -> bool {
        if Self::live_bool_field(
            model,
            &[
                "supported_in_api",
                "supportedInApi",
                "supported",
                "api_supported",
                "apiSupported",
                "enabled",
                "available",
            ],
        )
        .is_some_and(|supported| !supported)
        {
            return false;
        }
        if Self::live_bool_field(
            model,
            &[
                "disabled",
                "is_disabled",
                "isDisabled",
                "api_disabled",
                "apiDisabled",
                "policy_disabled",
                "policyDisabled",
                "policy_restricted",
                "policyRestricted",
                "restricted",
                "not_entitled",
                "notEntitled",
                "denied",
                "unsupported",
                "not_supported",
                "notSupported",
                "is_unsupported",
                "isUnsupported",
                "not_available",
                "notAvailable",
            ],
        )
        .unwrap_or(false)
        {
            return false;
        }
        !Self::live_status_is_disabled(model)
    }

    fn live_model_supports_tools(model: &Value) -> bool {
        Self::live_bool_field(
            model,
            &[
                "supports_tool_calls",
                "supportsToolCalls",
                "supports_tools",
                "supportsTools",
            ],
        )
        .unwrap_or(false)
    }

    fn live_model_supports_parallel_tools(model: &Value) -> bool {
        Self::live_bool_field(
            model,
            &[
                "supports_parallel_tool_calls",
                "supportsParallelToolCalls",
                "supports_parallel_tools",
                "supportsParallelTools",
            ],
        )
        .unwrap_or(false)
    }

    fn live_model_supports_multimodality(model: &Value) -> bool {
        model
            .get("input_modalities")
            .or_else(|| model.get("inputModalities"))
            .and_then(Value::as_array)
            .map(|modalities| {
                modalities
                    .iter()
                    .any(|modality| modality.as_str() == Some("image"))
            })
            .unwrap_or(false)
    }

    fn live_model_reasoning_levels(model: &Value) -> Option<Vec<String>> {
        let levels = model
            .get("supported_reasoning_levels")
            .or_else(|| model.get("supportedReasoningLevels"))
            .and_then(Value::as_array)?
            .iter()
            .filter_map(|r| {
                r.get("effort")
                    .or_else(|| r.get("id"))
                    .or_else(|| r.get("name"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect::<Vec<_>>();
        (!levels.is_empty()).then_some(levels)
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

    fn merge_refreshed_tokens(&self, mut refreshed: OAuthTokens) -> OAuthTokens {
        if refreshed.chatgpt_account_id.is_empty() {
            refreshed.chatgpt_account_id = self.oauth_tokens.chatgpt_account_id.clone();
        }
        refreshed
    }

    pub fn clear_tokens_after_permanent_refresh_error(&mut self) {
        self.oauth_tokens.access_token.clear();
        self.oauth_tokens.refresh_token.clear();
        self.oauth_tokens.expires_at = 0;
    }

    pub async fn force_refresh_after_auth_rejection(
        &mut self,
        http_client: &reqwest::Client,
        config_dir: &std::path::Path,
        instance_id: &str,
    ) -> Result<Option<String>, String> {
        if self.oauth_tokens.refresh_token.is_empty() {
            return Ok(None);
        }

        let refreshed = match crate::openai_codex_oauth::refresh_access_token(
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
                self.clear_tokens_after_permanent_refresh_error();
                self.save_oauth_tokens_config(config_dir, instance_id)
                    .await?;
                return Err(format!(
                    "OpenAI Codex OAuth refresh token is invalid. Please log in again in OpenAI Codex provider settings: {}",
                    e
                ));
            }
            Err(e) => {
                return Err(format!(
                    "OpenAI Codex OAuth refresh failed after backend rejected the access token: {}",
                    e
                ));
            }
        };

        let refreshed = self.merge_refreshed_tokens(refreshed);
        let access_token = refreshed.access_token.clone();
        self.oauth_tokens = refreshed;
        self.save_oauth_tokens_config(config_dir, instance_id)
            .await?;
        Ok((!access_token.is_empty()).then_some(access_token))
    }

    pub fn access_token_changed_since_rejection(
        &self,
        rejected_access_token: &str,
    ) -> Option<String> {
        (!self.oauth_tokens.access_token.is_empty()
            && self.oauth_tokens.access_token != rejected_access_token)
            .then(|| self.oauth_tokens.access_token.clone())
    }

    pub fn auth_state_matches(&self, tokens: &OAuthTokens, session_id: &str) -> bool {
        &self.oauth_tokens == tokens && self.session_id == session_id
    }

    pub fn update_auth_state_from(&mut self, source: &OpenAICodexProvider) {
        self.oauth_tokens = source.oauth_tokens.clone();
        self.session_id = source.session_id.clone();
    }

    pub fn update_auth_state_from_if_current(
        &mut self,
        source: &OpenAICodexProvider,
        previous_tokens: &OAuthTokens,
        previous_session_id: &str,
    ) -> bool {
        if source.auth_state_matches(previous_tokens, previous_session_id)
            || !self.auth_state_matches(previous_tokens, previous_session_id)
        {
            return false;
        }
        self.update_auth_state_from(source);
        true
    }

    /// Legacy hook for CLI-managed refresh diagnostics. CLI fallback has been removed
    /// from the engine; this always returns `None` so callers see no special path.
    pub fn codex_cli_unmanaged_refresh_message(_rejected_access_token: &str) -> Option<String> {
        None
    }

    fn unknown_live_codex_model(
        &self,
        id: String,
        enabled: bool,
        pricing: Option<ModelPricing>,
        model: &Value,
    ) -> AvailableModel {
        let supports_tools = Self::live_model_supports_tools(model);
        let supports_parallel_tools =
            supports_tools && Self::live_model_supports_parallel_tools(model);
        AvailableModel {
            id,
            display_name: None,
            n_ctx: Self::live_model_context_window(model).unwrap_or(8192),
            supports_tools,
            supports_parallel_tools,
            supports_strict_tools: false,
            supports_multimodality: Self::live_model_supports_multimodality(model),
            reasoning_effort_options: Self::live_model_reasoning_levels(model),
            supports_thinking_budget: false,
            supports_adaptive_thinking_budget: false,
            supports_cache_control: true,
            tokenizer: None,
            enabled,
            is_custom: false,
            pricing,
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens: Self::live_model_max_output_tokens(model),
            provider_variants: Vec::new(),
            wire_format_override: None,
            endpoint_override: None,
            base_model: None,
        }
    }
}

#[async_trait]
impl ProviderTrait for OpenAICodexProvider {
    fn name(&self) -> &str {
        "openai_codex"
    }

    fn display_name(&self) -> &str {
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
        Some(
            r"(?i)^(?:gpt[-_]5(?:\.[0-9]+)?(?:[-_]mini)?|gpt[-_][a-z0-9.]+(?:[-_][a-z0-9.]+)*[-_]codex(?:[-_](?:latest|preview|mini|spark|max))?)$",
        )
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  use_websocket:
    f_type: boolean
    f_desc: "Use experimental WebSocket streaming for ChatGPT backend OAuth requests. HTTP SSE remains the fallback."
    f_label: "Use WebSocket streaming"
    f_default: false
oauth:
  supported: true
  methods:
    - id: chatgpt
      label: "ChatGPT Plus/Pro"
      description: "Login with your ChatGPT Plus or Pro subscription"
description: |
  Use your ChatGPT Plus/Pro subscription to access OpenAI Codex and GPT-5 subscription models.

  **Setup:** Click **Login with OpenAI** below, or install Codex CLI and run `codex login`.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(oauth_tokens) = yaml.get("oauth_tokens") {
            self.oauth_tokens = serde_yaml::from_value(oauth_tokens.clone()).unwrap_or_default();
        }
        if let Some(session_id) = yaml
            .get("session_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            self.session_id = session_id.to_string();
        }
        if let Some(use_websocket) = yaml.get("use_websocket").and_then(|v| v.as_bool()) {
            self.use_websocket = use_websocket;
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        let auth_status = self.diagnose_auth_status();
        let (resolved_source, resolved_auth) = self.resolve_auth();
        let resolved_oauth = matches!(&resolved_auth, CodexAuth::ChatGptBackendOAuth { .. });
        let oauth_connected = self.oauth_tokens.has_valid_access_token()
            || self.oauth_tokens.has_refresh_token()
            || resolved_oauth;
        let auth_source = match resolved_source {
            AuthSource::InAppOAuth => "in_app_oauth",
            AuthSource::None => "none",
        };

        json!({
            "auth_status": auth_status,
            "auth_source": auth_source,
            "oauth_connected": oauth_connected,
            "use_websocket": self.use_websocket,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let (_, auth) = self.resolve_auth();
        let mut extra_headers = HashMap::new();

        let (chat_endpoint, api_key) = match auth {
            CodexAuth::ChatGptBackendOAuth {
                access_token,
                chatgpt_account_id,
                ..
            } => {
                extra_headers = self.chatgpt_backend_streaming_headers(&chatgpt_account_id);
                if self.use_websocket {
                    extra_headers.insert(
                        CODEX_WEBSOCKET_ENDPOINT_HEADER.to_string(),
                        CHATGPT_CODEX_RESPONSES_WEBSOCKET_URL.to_string(),
                    );
                }
                (
                    "https://chatgpt.com/backend-api/codex/responses".to_string(),
                    access_token,
                )
            }
            CodexAuth::None => (String::new(), String::new()),
        };

        let has_auth = !api_key.is_empty() && !chat_endpoint.is_empty();

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: has_auth && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint,
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
        // Subscription-only: only per-instance OAuth tokens count.
        if self.oauth_tokens.has_valid_access_token() {
            return true;
        }
        self.oauth_tokens.has_refresh_token()
    }

    fn model_source(&self) -> ModelSource {
        // Subscription-only: catalog is the only source.
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
        let (_, auth) = self.resolve_auth();
        match auth {
            CodexAuth::None => {
                tracing::warn!("OpenAI Codex: no auth");
                self.get_custom_models_only()
            }
            CodexAuth::ChatGptBackendOAuth {
                ref access_token,
                ref chatgpt_account_id,
            } => {
                self.fetch_models_from_chatgpt_api(
                    http_client,
                    model_caps,
                    access_token,
                    chatgpt_account_id,
                )
                .await
            }
        }
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
        if let Some(config) = self.custom_models.get(model_id) {
            if config.pricing.is_some() {
                return config.pricing.clone();
            }
        }
        None
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

        tracing::info!("OpenAI Codex: refreshing OAuth token on startup");
        let mut refreshed = match crate::openai_codex_oauth::refresh_access_token(
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
                    "OpenAI Codex: OAuth refresh token is invalid; clearing saved refresh token. Please log in again if Codex stops working: {}",
                    e
                );
                self.oauth_tokens.access_token.clear();
                self.oauth_tokens.refresh_token.clear();
                self.oauth_tokens.expires_at = 0;
                self.save_oauth_tokens_config(config_dir, instance_id)
                    .await?;
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        if refreshed.chatgpt_account_id.is_empty() {
            refreshed.chatgpt_account_id = self.oauth_tokens.chatgpt_account_id.clone();
        }

        self.oauth_tokens = refreshed;
        self.save_oauth_tokens_config(config_dir, instance_id).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::OpenAICodexProvider;
    use refact_core::model_caps::ModelCapabilities;
    use crate::openai_codex_oauth::OAuthTokens;
    use crate::traits::{CustomModelConfig, ModelPricing, ModelSource, ProviderTrait};

    fn provider_with_oauth(access_token: &str, chatgpt_account_id: &str) -> OpenAICodexProvider {
        OpenAICodexProvider {
            oauth_tokens: OAuthTokens {
                access_token: access_token.to_string(),
                chatgpt_account_id: chatgpt_account_id.to_string(),
                expires_at: i64::MAX,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn codex_caps(n_ctx: usize) -> ModelCapabilities {
        ModelCapabilities {
            n_ctx,
            max_output_tokens: 16_384,
            supports_tools: true,
            supports_parallel_tools: true,
            supports_vision: true,
            reasoning_effort_options: Some(vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
            ]),
            pricing: Some(ModelPricing {
                prompt: 3.0,
                generated: 12.0,
                cache_read: Some(1.5),
                cache_creation: Some(4.5),
                context_over_200k: None,
            }),
            ..Default::default()
        }
    }

    fn caps_map() -> HashMap<String, ModelCapabilities> {
        HashMap::from([
            ("openai/gpt-5.2".to_string(), codex_caps(402_000)),
            ("openai/gpt-5.3-codex".to_string(), codex_caps(253_000)),
            ("openai-codex/gpt-5.4".to_string(), codex_caps(404_000)),
            ("openai_codex/gpt-5.5".to_string(), codex_caps(405_000)),
        ])
    }

    #[test]
    fn model_source_is_always_model_caps() {
        let p = provider_with_oauth("tok", "acct-123");
        assert_eq!(p.model_source(), ModelSource::ModelCaps);
        // Even with no auth, source is ModelCaps (no API path remains).
        let p_no_auth = OpenAICodexProvider::default();
        assert_eq!(p_no_auth.model_source(), ModelSource::ModelCaps);
    }

    #[test]
    fn unauthenticated_provider_reports_not_configured() {
        let p = OpenAICodexProvider::default();
        assert!(!p.has_credentials());
        assert_eq!(p.diagnose_auth_status(), "Not configured — log in via OAuth");
        assert!(p.resolve_wham_context().is_err());
    }

    #[test]
    fn logged_in_provider_uses_chatgpt_backend() {
        let mut p = provider_with_oauth("tok", "acct-123");
        p.enabled_models = vec!["gpt-5-codex".to_string()];

        let runtime = p.build_runtime().unwrap();
        assert!(runtime.enabled);
        assert_eq!(
            runtime.chat_endpoint,
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(runtime.api_key, "tok");
        assert_eq!(
            runtime
                .extra_headers
                .get("chatgpt-account-id")
                .map(String::as_str),
            Some("acct-123")
        );
        assert_eq!(p.diagnose_auth_status(), "Connected (ChatGPT backend)");
    }

    #[test]
    fn fetch_models_from_catalog_returns_subscription_models() {
        let p = provider_with_oauth("tok", "acct-123");
        let models = p.fetch_models_from_catalog(&caps_map());
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();

        assert!(ids.contains(&"gpt-5.2"));
        assert!(ids.contains(&"gpt-5.3-codex"));
        assert!(ids.contains(&"gpt-5.4"));
        assert!(ids.contains(&"gpt-5.5"));
    }

    #[test]
    fn custom_models_still_appear() {
        let mut p = provider_with_oauth("tok", "acct-123");
        p.enabled_models = vec!["my-custom".to_string()];
        p.custom_models.insert(
            "my-custom".to_string(),
            CustomModelConfig {
                n_ctx: Some(4096),
                supports_tools: Some(true),
                pricing: Some(ModelPricing {
                    prompt: 1.0,
                    generated: 2.0,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );

        let models = p.fetch_models_from_catalog(&caps_map());
        let custom = models.iter().find(|m| m.id == "my-custom").unwrap();
        assert!(custom.enabled);
        assert!(custom.is_custom);
    }

    #[test]
    fn codex_cli_unmanaged_refresh_message_always_returns_none() {
        // CLI fallback was removed entirely; this hook is preserved for callers
        // but never produces a message anymore.
        assert!(OpenAICodexProvider::codex_cli_unmanaged_refresh_message("cli-access").is_none());
        assert!(OpenAICodexProvider::codex_cli_unmanaged_refresh_message("").is_none());
    }

    #[test]
    fn websocket_setting_adds_chatgpt_backend_runtime_marker_only_when_enabled() {
        let mut p = provider_with_oauth("tok", "acct-123");
        p.enabled_models = vec!["gpt-5-codex".to_string()];

        let runtime = p.build_runtime().unwrap();
        assert!(!runtime
            .extra_headers
            .contains_key(super::CODEX_WEBSOCKET_ENDPOINT_HEADER));

        p.use_websocket = true;
        let runtime = p.build_runtime().unwrap();
        assert_eq!(
            runtime
                .extra_headers
                .get(super::CODEX_WEBSOCKET_ENDPOINT_HEADER)
                .map(String::as_str),
            Some(super::CHATGPT_CODEX_RESPONSES_WEBSOCKET_URL)
        );
    }

    #[test]
    fn provider_settings_as_json_does_not_expose_api_key_fields() {
        let p = provider_with_oauth("tok", "acct-123");
        let settings = p.provider_settings_as_json();
        let obj = settings.as_object().unwrap();
        assert!(!obj.contains_key("api_key_ready"));
        assert!(!obj.contains_key("api_key_exchange_error"));
        assert!(!obj.contains_key("cli_refresh_managed"));
        assert_eq!(obj["auth_source"], json!("in_app_oauth"));
        assert_eq!(obj["oauth_connected"], json!(true));
    }

    #[test]
    fn jwt_account_id_extraction_supports_three_paths() {
        use crate::openai_codex_oauth::extract_chatgpt_account_id_from_jwt;
        use base64::Engine;

        fn make_jwt(payload: serde_json::Value) -> String {
            let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{}");
            let payload_b = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&payload).unwrap());
            let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"sig");
            format!("{}.{}.{}", header, payload_b, sig)
        }

        // Top-level claim
        let jwt = make_jwt(json!({ "chatgpt_account_id": "top" }));
        assert_eq!(extract_chatgpt_account_id_from_jwt(&jwt).as_deref(), Some("top"));

        // Namespaced claim
        let jwt = make_jwt(json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "ns" }
        }));
        assert_eq!(extract_chatgpt_account_id_from_jwt(&jwt).as_deref(), Some("ns"));

        // organizations[0].id
        let jwt = make_jwt(json!({ "organizations": [{ "id": "org-1" }] }));
        assert_eq!(
            extract_chatgpt_account_id_from_jwt(&jwt).as_deref(),
            Some("org-1")
        );

        // No match
        let jwt = make_jwt(json!({}));
        assert!(extract_chatgpt_account_id_from_jwt(&jwt).is_none());
    }
}
