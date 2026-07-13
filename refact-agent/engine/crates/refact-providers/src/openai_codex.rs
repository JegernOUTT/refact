use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::{Mutex as AMutex, MutexGuard};

use refact_core::model_caps::ModelCapabilities;
use refact_core::llm_types::WireFormat;
use crate::llm_http_retry::{
    insert_llm_http_header_retry_config, LLM_HTTP_HEADER_RETRY_MAX_ATTEMPTS_DEFAULT,
    LLM_HTTP_HEADER_RETRY_TIMEOUT_SECONDS_DEFAULT,
};
use crate::openai_codex_oauth::OAuthTokens;
use crate::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};

const CODEX_ORIGINATOR: &str = "refact-lsp";
const CHATGPT_CODEX_MODELS_URL: &str =
    "https://chatgpt.com/backend-api/codex/models?client_version=999.999.999";
const CHATGPT_CODEX_RESPONSES_WEBSOCKET_URL: &str = "wss://chatgpt.com/backend-api/codex/responses";
const CHATGPT_CODEX_RESET_REDEEM_URL: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume";
const GPT_5_6_CODEX_CONTEXT_WINDOW: usize = 372_000;
const GPT_5_5_CODEX_CONTEXT_WINDOW: usize = 272_000;
pub const CODEX_WEBSOCKET_ENDPOINT_HEADER: &str =
    "x-refact-internal-openai-codex-websocket-endpoint";
#[allow(dead_code)]
const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";
const CODEX_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(8);
const CODEX_USAGE_TIMEOUT: Duration = Duration::from_secs(8);

lazy_static::lazy_static! {
    static ref OPENAI_CODEX_REFRESH_GUARD: AMutex<()> = AMutex::new(());
}

fn new_codex_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn default_use_websocket() -> bool {
    false
}

fn default_http_response_header_retry_enabled() -> bool {
    true
}

fn default_http_response_header_retry_timeout_seconds() -> u64 {
    LLM_HTTP_HEADER_RETRY_TIMEOUT_SECONDS_DEFAULT
}

fn default_http_response_header_retry_max_attempts() -> usize {
    LLM_HTTP_HEADER_RETRY_MAX_ATTEMPTS_DEFAULT
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
    matches!(suffixes.as_slice(), [] | ["mini"] | ["sol"] | ["terra"])
}

// gpt-5.6-luna appears in /codex/models but /codex/responses rejects it with
// 404 "Model not found gpt-5.6-luna" (verified live 2026-07-13): it is the
// server-side safety-buffering draft model advertised through the
// x-codex-safety-buffering-faster-model response metadata header, not a
// user-requestable chat model. Keep it out of catalogs and live listings.
fn is_codex_internal_only_model(id: &str) -> bool {
    normalized_model_id(id) == "gpt-5.6-luna"
}

fn is_openai_codex_catalog_model(id: &str) -> bool {
    !is_codex_internal_only_model(id)
        && (is_codex_named_model(id) || is_gpt5_subscription_model(id))
}

fn is_chatgpt_codex_live_model(id: &str) -> bool {
    is_openai_codex_catalog_model(id)
}

#[allow(dead_code)]
fn is_openai_api_codex_live_model(id: &str) -> bool {
    is_codex_named_model(id)
}

fn codex_model_context_window_override(id: &str) -> Option<usize> {
    match normalized_model_id(id).as_str() {
        "gpt-5.6-sol" | "gpt-5.6-terra" => Some(GPT_5_6_CODEX_CONTEXT_WINDOW),
        "gpt-5.5" => Some(GPT_5_5_CODEX_CONTEXT_WINDOW),
        _ => None,
    }
}

fn apply_codex_model_overrides(model: &mut AvailableModel) {
    if let Some(n_ctx) = codex_model_context_window_override(&model.id) {
        model.n_ctx = n_ctx;
    }
}

fn codex_builtin_fallback_caps(model_id: &str) -> Option<ModelCapabilities> {
    let n_ctx = codex_model_context_window_override(model_id)?;
    Some(ModelCapabilities {
        n_ctx,
        max_output_tokens: 16_384,
        supports_tools: true,
        supports_parallel_tools: true,
        supports_vision: true,
        reasoning_effort_options: Some(vec![
            "minimal".to_string(),
            "low".to_string(),
            "medium".to_string(),
            "high".to_string(),
            "xhigh".to_string(),
        ]),
        ..Default::default()
    })
}

fn codex_builtin_fallback_model_ids() -> &'static [&'static str] {
    &["gpt-5.6-sol", "gpt-5.6-terra"]
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
    #[serde(default = "default_use_websocket")]
    pub use_websocket: bool,
    #[serde(default = "default_http_response_header_retry_enabled")]
    pub http_response_header_retry_enabled: bool,
    #[serde(default = "default_http_response_header_retry_timeout_seconds")]
    pub http_response_header_retry_timeout_seconds: u64,
    #[serde(default = "default_http_response_header_retry_max_attempts")]
    pub http_response_header_retry_max_attempts: usize,
}

impl Default for OpenAICodexProvider {
    fn default() -> Self {
        Self {
            enabled_models: Vec::new(),
            custom_models: HashMap::new(),
            oauth_tokens: OAuthTokens::default(),
            session_id: new_codex_session_id(),
            use_websocket: default_use_websocket(),
            http_response_header_retry_enabled: default_http_response_header_retry_enabled(),
            http_response_header_retry_timeout_seconds:
                default_http_response_header_retry_timeout_seconds(),
            http_response_header_retry_max_attempts:
                default_http_response_header_retry_max_attempts(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexUsageWindow {
    pub used_percent: f64,
    pub reset_at: Option<String>,
    pub reset_after_seconds: Option<u64>,
    pub limit_window_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexRateLimit {
    pub allowed: Option<bool>,
    pub limit_reached: bool,
    pub primary_window: Option<OpenAICodexUsageWindow>,
    pub secondary_window: Option<OpenAICodexUsageWindow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexAdditionalRateLimit {
    pub limit_name: Option<String>,
    pub metered_feature: Option<String>,
    pub rate_limit: Option<OpenAICodexRateLimit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexResetCredits {
    pub available_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexResetRedeem {
    pub code: String,
    pub windows_reset: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexCredits {
    pub balance: f64,
    pub unlimited: bool,
    pub has_credits: bool,
    pub granted: Option<f64>,
    pub used: Option<f64>,
    pub reset_at: Option<String>,
    pub overage_limit_reached: Option<bool>,
    pub approx_cloud_messages: Option<Vec<f64>>,
    pub approx_local_messages: Option<Vec<f64>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexSpendControl {
    pub individual_limit: Option<f64>,
    pub reached: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexUsage {
    pub account_id: Option<String>,
    pub user_id: Option<String>,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    pub rate_limit: Option<OpenAICodexRateLimit>,
    pub additional_rate_limits: Option<Vec<OpenAICodexAdditionalRateLimit>>,
    pub code_review_rate_limit: Option<OpenAICodexRateLimit>,
    pub rate_limit_reached_type: Option<String>,
    pub rate_limit_reset_credits: Option<OpenAICodexResetCredits>,
    pub credits: Option<OpenAICodexCredits>,
    pub spend_control: Option<OpenAICodexSpendControl>,
    pub promo: Option<Value>,
    pub referral_beacon: Option<Value>,
    pub raw_extra: Map<String, Value>,
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

    fn yaml_usize_field(yaml: &serde_yaml::Value, key: &str) -> Result<Option<usize>, String> {
        let Some(value) = yaml.get(key) else {
            return Ok(None);
        };
        if let Some(value) = value.as_u64() {
            return usize::try_from(value)
                .map(Some)
                .map_err(|_| format!("{key} is too large"));
        }
        if let Some(value) = value.as_i64() {
            return usize::try_from(value)
                .map(Some)
                .map_err(|_| format!("{key} must be a positive integer"));
        }
        if let Some(value) = value.as_str() {
            let value = value.trim();
            if value.is_empty() {
                return Ok(None);
            }
            return value
                .parse::<usize>()
                .map(Some)
                .map_err(|_| format!("{key} must be a positive integer"));
        }
        Err(format!("{key} must be a positive integer"))
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
            .timeout(CODEX_USAGE_TIMEOUT)
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

    pub async fn redeem_reset_credit_once(
        &self,
        http_client: &reqwest::Client,
        token: &str,
        chatgpt_account_id: &str,
        redeem_request_id: &str,
    ) -> Result<OpenAICodexResetRedeem, UsageRequestError> {
        let mut req = http_client
            .post(CHATGPT_CODEX_RESET_REDEEM_URL)
            .timeout(CODEX_USAGE_TIMEOUT)
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({ "redeem_request_id": redeem_request_id }));
        for (key, value) in self.chatgpt_backend_metadata_headers(chatgpt_account_id) {
            req = req.header(key, value);
        }
        let resp = req.send().await.map_err(|e| {
            UsageRequestError::Other(format!("OpenAI Codex reset redeem request failed: {}", e))
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(UsageRequestError::Status(status, body));
        }

        let root: Value = resp.json().await.map_err(|e| {
            UsageRequestError::Other(format!(
                "Failed to parse OpenAI Codex reset redeem response: {}",
                e
            ))
        })?;

        Ok(Self::parse_redeem_payload(&root))
    }

    pub async fn redeem_reset_credit(
        &self,
        http_client: &reqwest::Client,
        redeem_request_id: &str,
    ) -> Result<OpenAICodexResetRedeem, String> {
        let context = self.resolve_wham_context()?;
        self.redeem_reset_credit_once(
            http_client,
            &context.access_token,
            &context.chatgpt_account_id,
            redeem_request_id,
        )
        .await
        .map_err(|error| Self::usage_request_error_to_string(error, context.source))
    }

    fn parse_usage_payload(root: &Value) -> OpenAICodexUsage {
        let data = root.get("data").unwrap_or(root);
        let raw_extra = Self::collect_raw_extra(
            data,
            &[
                "account_id",
                "accountId",
                "user_id",
                "userId",
                "email",
                "plan_type",
                "planType",
                "codex_plan_type",
                "plan",
                "rate_limit",
                "rateLimit",
                "additional_rate_limits",
                "additionalRateLimits",
                "code_review_rate_limit",
                "codeReviewRateLimit",
                "code_review",
                "rate_limit_reached_type",
                "rateLimitReachedType",
                "rate_limit_reset_credits",
                "rateLimitResetCredits",
                "credits",
                "credit_balance",
                "spend_control",
                "spendControl",
                "promo",
                "referral_beacon",
                "referralBeacon",
            ],
        );
        let account_id = Self::string_field(data, &["account_id", "accountId"]);
        let user_id = Self::string_field(data, &["user_id", "userId"]);
        let email = Self::string_field(data, &["email"]);
        let plan_type = Self::string_field(data, &["plan_type", "planType", "codex_plan_type"])
            .or_else(|| {
                data.get("plan")
                    .and_then(|plan| Self::string_field(plan, &["type", "name"]))
            });
        let rate_limit =
            Self::field(data, &["rate_limit", "rateLimit"]).and_then(Self::parse_rate_limit);
        let additional_rate_limits =
            Self::field(data, &["additional_rate_limits", "additionalRateLimits"])
                .and_then(Self::parse_additional_rate_limits);
        let code_review_rate_limit = Self::field(
            data,
            &[
                "code_review_rate_limit",
                "codeReviewRateLimit",
                "code_review",
            ],
        )
        .and_then(Self::parse_rate_limit);
        let rate_limit_reached_type =
            Self::string_field(data, &["rate_limit_reached_type", "rateLimitReachedType"]);
        let rate_limit_reset_credits =
            Self::field(data, &["rate_limit_reset_credits", "rateLimitResetCredits"])
                .and_then(Self::parse_reset_credits);
        let credits = Self::field(data, &["credits", "credit_balance"]).map(Self::parse_credits);
        let spend_control = Self::field(data, &["spend_control", "spendControl"])
            .and_then(Self::parse_spend_control);
        OpenAICodexUsage {
            account_id,
            user_id,
            email,
            plan_type,
            rate_limit,
            additional_rate_limits,
            code_review_rate_limit,
            rate_limit_reached_type,
            rate_limit_reset_credits,
            credits,
            spend_control,
            promo: data.get("promo").filter(|value| !value.is_null()).cloned(),
            referral_beacon: Self::field(data, &["referral_beacon", "referralBeacon"])
                .filter(|value| !value.is_null())
                .cloned(),
            raw_extra,
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

    fn parse_rate_limit(rl: &Value) -> Option<OpenAICodexRateLimit> {
        if !rl.is_object() {
            return None;
        }
        let primary_window =
            Self::field(rl, &["primary_window", "primary"]).and_then(Self::parse_usage_window);
        let secondary_window =
            Self::field(rl, &["secondary_window", "secondary"]).and_then(Self::parse_usage_window);
        let allowed =
            Self::field(rl, &["allowed", "is_allowed", "isAllowed"]).and_then(Value::as_bool);
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
        Some(OpenAICodexRateLimit {
            allowed,
            limit_reached,
            primary_window,
            secondary_window,
        })
    }

    fn parse_additional_rate_limits(value: &Value) -> Option<Vec<OpenAICodexAdditionalRateLimit>> {
        let limits = value.as_array()?;
        Some(
            limits
                .iter()
                .filter_map(|limit| {
                    limit.as_object()?;
                    Some(OpenAICodexAdditionalRateLimit {
                        limit_name: Self::string_field(limit, &["limit_name", "limitName", "name"]),
                        metered_feature: Self::string_field(
                            limit,
                            &["metered_feature", "meteredFeature"],
                        ),
                        rate_limit: Self::field(limit, &["rate_limit", "rateLimit"])
                            .and_then(Self::parse_rate_limit),
                    })
                })
                .collect(),
        )
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
        let reset_after_seconds = Self::field(
            obj,
            &[
                "reset_after_seconds",
                "resetAfterSeconds",
                "resets_after_seconds",
                "resetsAfterSeconds",
            ],
        )
        .and_then(Self::as_u64_loose);
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
            reset_after_seconds,
            limit_window_seconds,
        })
    }

    fn parse_reset_credits(value: &Value) -> Option<OpenAICodexResetCredits> {
        if !value.is_object() {
            return None;
        }
        Some(OpenAICodexResetCredits {
            available_count: Self::field(value, &["available_count", "availableCount"])
                .and_then(Self::as_u64_loose),
        })
    }

    fn parse_redeem_payload(root: &Value) -> OpenAICodexResetRedeem {
        let data = root.get("data").unwrap_or(root);
        OpenAICodexResetRedeem {
            code: Self::string_field(data, &["code", "status"]).unwrap_or_default(),
            windows_reset: Self::field(data, &["windows_reset", "windowsReset"])
                .and_then(Self::as_u64_loose),
        }
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
            overage_limit_reached: Self::field(
                c,
                &["overage_limit_reached", "overageLimitReached"],
            )
            .and_then(Value::as_bool),
            approx_cloud_messages: Self::field(
                c,
                &["approx_cloud_messages", "approxCloudMessages"],
            )
            .and_then(Self::number_array),
            approx_local_messages: Self::field(
                c,
                &["approx_local_messages", "approxLocalMessages"],
            )
            .and_then(Self::number_array),
        }
    }

    fn parse_spend_control(value: &Value) -> Option<OpenAICodexSpendControl> {
        if !value.is_object() {
            return None;
        }
        Some(OpenAICodexSpendControl {
            individual_limit: Self::field(value, &["individual_limit", "individualLimit"])
                .and_then(Self::as_f64_loose),
            reached: Self::field(value, &["reached", "limit_reached", "limitReached"])
                .and_then(Value::as_bool),
        })
    }

    fn number_array(value: &Value) -> Option<Vec<f64>> {
        let values = value.as_array()?;
        Some(values.iter().filter_map(Self::as_f64_loose).collect())
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
        let mut req = http_client
            .get(CHATGPT_CODEX_MODELS_URL)
            .timeout(CODEX_DISCOVERY_TIMEOUT)
            .header(
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
            .timeout(CODEX_DISCOVERY_TIMEOUT)
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
            models_map.insert(model_id.to_string(), {
                let mut model = AvailableModel::from_caps(model_id, caps, enabled, pricing);
                apply_codex_model_overrides(&mut model);
                model
            });
        }
        for model_id in codex_builtin_fallback_model_ids() {
            if models_map.contains_key(*model_id) {
                continue;
            }
            let Some(caps) = codex_builtin_fallback_caps(model_id) else {
                continue;
            };
            let enabled = enabled_set.contains(*model_id);
            let pricing = self.custom_model_pricing(model_id);
            models_map.insert(
                (*model_id).to_string(),
                AvailableModel::from_caps(model_id, &caps, enabled, pricing),
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
            apply_codex_model_overrides(&mut available);
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
            r"(?i)^(?:gpt[-_]5(?:\.[0-9]+)?(?:[-_](?:mini|sol|terra|luna))?|gpt[-_][a-z0-9.]+(?:[-_][a-z0-9.]+)*[-_]codex(?:[-_](?:latest|preview|mini|spark|max))?)$",
        )
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  use_websocket:
    f_type: boolean
    f_desc: "Use WebSocket streaming for ChatGPT backend OAuth requests. When enabled, transient WebSocket failures are retried instead of falling back to HTTP SSE."
    f_label: "Use WebSocket streaming"
    f_default: false
  http_response_header_retry_enabled:
    f_type: boolean
    f_desc: "Retry ChatGPT backend transport requests when no HTTP response headers or WebSocket connection arrive before the timeout."
    f_label: "Retry stalled transport requests"
    f_default: true
  http_response_header_retry_timeout_seconds:
    f_type: integer
    f_desc: "Seconds to wait for HTTP response headers or WebSocket connection before retrying."
    f_label: "Transport timeout, seconds"
    f_default: 10
  http_response_header_retry_max_attempts:
    f_type: integer
    f_desc: "Maximum transport attempts before surfacing the timeout error."
    f_label: "Max retry attempts"
    f_default: 10
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
        if let Some(enabled) = yaml
            .get("http_response_header_retry_enabled")
            .and_then(|v| v.as_bool())
        {
            self.http_response_header_retry_enabled = enabled;
        }
        if let Some(timeout_seconds) =
            Self::yaml_usize_field(&yaml, "http_response_header_retry_timeout_seconds")?
        {
            self.http_response_header_retry_timeout_seconds = timeout_seconds.max(1) as u64;
        }
        if let Some(max_attempts) =
            Self::yaml_usize_field(&yaml, "http_response_header_retry_max_attempts")?
        {
            self.http_response_header_retry_max_attempts = max_attempts.max(1);
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
            "http_response_header_retry_enabled": self.http_response_header_retry_enabled,
            "http_response_header_retry_timeout_seconds": self.http_response_header_retry_timeout_seconds,
            "http_response_header_retry_max_attempts": self.http_response_header_retry_max_attempts,
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
                insert_llm_http_header_retry_config(
                    &mut extra_headers,
                    self.http_response_header_retry_enabled,
                    self.http_response_header_retry_timeout_seconds,
                    self.http_response_header_retry_max_attempts,
                );
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
            (
                "openai_codex/gpt-5.6-sol".to_string(),
                codex_caps(1_050_000),
            ),
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
        assert_eq!(
            p.diagnose_auth_status(),
            "Not configured — log in via OAuth"
        );
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
        assert!(ids.contains(&"gpt-5.6-sol"));
        assert!(ids.contains(&"gpt-5.6-terra"));
        // luna is the internal safety-buffering draft model; /codex/responses 404s it.
        assert!(!ids.contains(&"gpt-5.6-luna"));
    }

    #[test]
    fn codex_model_filters_accept_known_gpt56_variants_only() {
        assert!(super::is_chatgpt_codex_live_model("gpt-5.6-sol"));
        assert!(super::is_chatgpt_codex_live_model("gpt-5.6-terra"));
        // Listed by /codex/models but rejected by /codex/responses (internal draft model).
        assert!(!super::is_chatgpt_codex_live_model("gpt-5.6-luna"));
        assert!(super::is_chatgpt_codex_live_model("gpt-5.6"));
        assert!(!super::is_chatgpt_codex_live_model("gpt-5.6-random"));
        assert!(!super::is_chatgpt_codex_live_model("gpt-5.6-sol-preview"));

        let regex = regex::Regex::new(OpenAICodexProvider::default().model_filter_regex().unwrap())
            .unwrap();
        assert!(regex.is_match("gpt-5.6-sol"));
        assert!(regex.is_match("gpt-5.6-terra"));
        assert!(regex.is_match("gpt-5.6-luna"));
        assert!(!regex.is_match("gpt-5.6-random"));
    }

    #[test]
    fn codex_context_overrides_pin_subscription_windows() {
        assert_eq!(
            super::codex_model_context_window_override("gpt-5.6-sol"),
            Some(372_000)
        );
        assert_eq!(
            super::codex_model_context_window_override("gpt-5.6-terra"),
            Some(372_000)
        );
        assert_eq!(
            super::codex_model_context_window_override("gpt-5.6-luna"),
            None
        );
        assert_eq!(
            super::codex_model_context_window_override("gpt-5.5"),
            Some(272_000)
        );
        assert_eq!(super::codex_model_context_window_override("gpt-5.6"), None);
        assert_eq!(super::codex_model_context_window_override("gpt-5.4"), None);
    }

    #[test]
    fn catalog_fallback_adds_gpt56_variants_when_models_dev_is_stale() {
        let p = provider_with_oauth("tok", "acct-123");
        let stale_caps = HashMap::from([("openai_codex/gpt-5.5".to_string(), codex_caps(405_000))]);
        let models = p.fetch_models_from_catalog(&stale_caps);

        assert!(models.iter().all(|m| m.id != "gpt-5.6-luna"));
        for model_id in ["gpt-5.6-sol", "gpt-5.6-terra"] {
            let model = models.iter().find(|m| m.id == model_id).unwrap();
            assert_eq!(model.n_ctx, 372_000);
            assert_eq!(
                model.reasoning_effort_options.as_deref(),
                Some(
                    &[
                        "minimal".to_string(),
                        "low".to_string(),
                        "medium".to_string(),
                        "high".to_string(),
                        "xhigh".to_string(),
                    ][..]
                )
            );
        }
    }

    #[test]
    fn codex_context_overrides_apply_to_live_and_catalog_models() {
        let p = provider_with_oauth("tok", "acct-123");
        let catalog_models = p.fetch_models_from_catalog(&caps_map());
        let sol_from_catalog = catalog_models
            .iter()
            .find(|m| m.id == "gpt-5.6-sol")
            .unwrap();
        let gpt55_from_catalog = catalog_models.iter().find(|m| m.id == "gpt-5.5").unwrap();
        assert_eq!(sol_from_catalog.n_ctx, 372_000);
        assert_eq!(gpt55_from_catalog.n_ctx, 272_000);

        let live_models = p.available_models_from_live_chatgpt_models(
            &[
                json!({
                    "slug": "gpt-5.6-terra",
                    "display_name": "GPT-5.6 Terra",
                    "max_context_window": 1_050_000,
                    "supported": true,
                    "supported_reasoning_levels": ["low", "medium", "high", "xhigh"]
                }),
                json!({
                    "slug": "gpt-5.6-random",
                    "display_name": "GPT-5.6 Random",
                    "max_context_window": 372_000,
                    "supported": true
                }),
            ],
            &caps_map(),
        );
        let terra = live_models
            .iter()
            .find(|m| m.id == "gpt-5.6-terra")
            .unwrap();
        assert_eq!(terra.n_ctx, 372_000);
        assert_eq!(terra.display_name.as_deref(), Some("GPT-5.6 Terra"));
        assert!(!live_models.iter().any(|m| m.id == "gpt-5.6-random"));
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
    fn websocket_setting_is_opt_in_for_chatgpt_backend_runtime() {
        let mut p = provider_with_oauth("tok", "acct-123");
        p.enabled_models = vec!["gpt-5-codex".to_string()];

        let runtime = p.build_runtime().unwrap();
        assert!(!runtime
            .extra_headers
            .contains_key(super::CODEX_WEBSOCKET_ENDPOINT_HEADER));
        assert_eq!(
            runtime
                .extra_headers
                .get(crate::llm_http_retry::LLM_HTTP_HEADER_RETRY_ENABLED_HEADER)
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            runtime
                .extra_headers
                .get(crate::llm_http_retry::LLM_HTTP_HEADER_RETRY_TIMEOUT_SECONDS_HEADER)
                .map(String::as_str),
            Some("10")
        );
        assert_eq!(
            runtime
                .extra_headers
                .get(crate::llm_http_retry::LLM_HTTP_HEADER_RETRY_MAX_ATTEMPTS_HEADER)
                .map(String::as_str),
            Some("10")
        );

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
    fn websocket_setting_defaults_off_when_missing_from_config() {
        let provider: OpenAICodexProvider = serde_yaml::from_str("enabled_models: []").unwrap();

        assert!(!provider.use_websocket);
    }

    #[test]
    fn http_header_retry_settings_apply_and_roundtrip() {
        let mut p = OpenAICodexProvider::default();
        p.provider_settings_apply(
            serde_yaml::from_str(
                r#"
http_response_header_retry_enabled: false
http_response_header_retry_timeout_seconds: "7"
http_response_header_retry_max_attempts: 12
"#,
            )
            .unwrap(),
        )
        .unwrap();

        assert!(!p.http_response_header_retry_enabled);
        assert_eq!(p.http_response_header_retry_timeout_seconds, 7);
        assert_eq!(p.http_response_header_retry_max_attempts, 12);

        let settings = p.provider_settings_as_json();
        assert_eq!(settings["http_response_header_retry_enabled"], json!(false));
        assert_eq!(
            settings["http_response_header_retry_timeout_seconds"],
            json!(7)
        );
        assert_eq!(
            settings["http_response_header_retry_max_attempts"],
            json!(12)
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
    fn openai_codex_usage_parser_preserves_wham_quota_fields() {
        let usage = OpenAICodexProvider::parse_usage_payload(&json!({
            "account_id": "acct-redacted",
            "user_id": "user-redacted",
            "email": "user@example.com",
            "plan_type": "pro",
            "rate_limit": {
                "allowed": true,
                "limit_reached": false,
                "primary_window": {
                    "limit_window_seconds": 18000,
                    "reset_after_seconds": 1234,
                    "reset_at": 1781096163,
                    "used_percent": 38
                },
                "secondary_window": {
                    "limit_window_seconds": 604800,
                    "reset_after_seconds": "5678",
                    "reset_at": "2026-06-11T00:27:25+00:00",
                    "used_percent": "77.5"
                }
            },
            "additional_rate_limits": [
                {
                    "limit_name": "GPT-5.3-Codex-Spark",
                    "metered_feature": "codex_bengalfox",
                    "rate_limit": {
                        "allowed": true,
                        "limit_reached": false,
                        "primary_window": {
                            "limit_window_seconds": 18000,
                            "reset_after_seconds": 18000,
                            "reset_at": 1781096289,
                            "used_percent": 0
                        },
                        "secondary_window": {
                            "limit_window_seconds": 604800,
                            "reset_after_seconds": 604800,
                            "reset_at": 1781683089,
                            "used_percent": 0
                        }
                    }
                }
            ],
            "code_review_rate_limit": null,
            "rate_limit_reached_type": "weekly",
            "rate_limit_reset_credits": { "available_count": "2" },
            "credits": {
                "balance": "0",
                "has_credits": false,
                "unlimited": false,
                "overage_limit_reached": true,
                "approx_cloud_messages": [1, "2.5"],
                "approx_local_messages": [3, 4]
            },
            "spend_control": {
                "individual_limit": "10.5",
                "reached": false
            },
            "promo": { "kind": "test" },
            "referral_beacon": { "seen": true },
            "future_quota": { "provider_added": true }
        }));

        assert_eq!(usage.account_id.as_deref(), Some("acct-redacted"));
        assert_eq!(usage.user_id.as_deref(), Some("user-redacted"));
        assert_eq!(usage.email.as_deref(), Some("user@example.com"));
        assert_eq!(usage.plan_type.as_deref(), Some("pro"));

        let rate_limit = usage.rate_limit.unwrap();
        assert_eq!(rate_limit.allowed, Some(true));
        assert!(!rate_limit.limit_reached);
        let primary = rate_limit.primary_window.unwrap();
        assert_eq!(primary.used_percent, 38.0);
        assert_eq!(primary.limit_window_seconds, Some(18_000));
        assert_eq!(primary.reset_after_seconds, Some(1_234));
        assert_eq!(
            primary.reset_at.as_deref(),
            Some("2026-06-10T12:56:03+00:00")
        );
        let secondary = rate_limit.secondary_window.unwrap();
        assert_eq!(secondary.used_percent, 77.5);
        assert_eq!(secondary.reset_after_seconds, Some(5_678));
        assert_eq!(
            secondary.reset_at.as_deref(),
            Some("2026-06-11T00:27:25+00:00")
        );

        let additional = usage.additional_rate_limits.unwrap();
        assert_eq!(additional.len(), 1);
        assert_eq!(
            additional[0].limit_name.as_deref(),
            Some("GPT-5.3-Codex-Spark")
        );
        assert_eq!(
            additional[0].metered_feature.as_deref(),
            Some("codex_bengalfox")
        );
        assert_eq!(
            additional[0]
                .rate_limit
                .as_ref()
                .and_then(|rl| rl.primary_window.as_ref())
                .and_then(|window| window.limit_window_seconds),
            Some(18_000)
        );

        assert!(usage.code_review_rate_limit.is_none());
        assert_eq!(usage.rate_limit_reached_type.as_deref(), Some("weekly"));
        assert_eq!(
            usage.rate_limit_reset_credits.unwrap().available_count,
            Some(2)
        );

        let credits = usage.credits.unwrap();
        assert_eq!(credits.balance, 0.0);
        assert!(!credits.has_credits);
        assert!(!credits.unlimited);
        assert_eq!(credits.overage_limit_reached, Some(true));
        assert_eq!(credits.approx_cloud_messages, Some(vec![1.0, 2.5]));
        assert_eq!(credits.approx_local_messages, Some(vec![3.0, 4.0]));

        let spend_control = usage.spend_control.unwrap();
        assert_eq!(spend_control.individual_limit, Some(10.5));
        assert_eq!(spend_control.reached, Some(false));
        assert_eq!(usage.promo.unwrap()["kind"], json!("test"));
        assert_eq!(usage.referral_beacon.unwrap()["seen"], json!(true));
        assert_eq!(
            usage.raw_extra["future_quota"]["provider_added"],
            json!(true)
        );
    }

    #[test]
    fn openai_codex_redeem_payload_parses_code_and_windows() {
        let redeem = OpenAICodexProvider::parse_redeem_payload(&json!({
            "code": "reset",
            "windows_reset": 2
        }));
        assert_eq!(redeem.code, "reset");
        assert_eq!(redeem.windows_reset, Some(2));

        // Tolerates a `data` wrapper, camelCase key, and string-encoded number.
        let wrapped = OpenAICodexProvider::parse_redeem_payload(&json!({
            "data": { "code": "already_redeemed", "windowsReset": "1" }
        }));
        assert_eq!(wrapped.code, "already_redeemed");
        assert_eq!(wrapped.windows_reset, Some(1));

        // Missing fields degrade gracefully.
        let empty = OpenAICodexProvider::parse_redeem_payload(&json!({}));
        assert_eq!(empty.code, "");
        assert_eq!(empty.windows_reset, None);
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
        assert_eq!(
            extract_chatgpt_account_id_from_jwt(&jwt).as_deref(),
            Some("top")
        );

        // Namespaced claim
        let jwt = make_jwt(json!({
            "https://api.openai.com/auth": { "chatgpt_account_id": "ns" }
        }));
        assert_eq!(
            extract_chatgpt_account_id_from_jwt(&jwt).as_deref(),
            Some("ns")
        );

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
