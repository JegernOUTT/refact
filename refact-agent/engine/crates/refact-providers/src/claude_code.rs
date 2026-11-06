use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::{Mutex as AMutex, MutexGuard};

use refact_core::claude_backend::{ClaudeBackend, ClaudeTransportConfig, CLAUDE_TRANSPORT_HEADER};
use refact_core::provider_types::{available_model_from_catalog_and_live, LiveModelFields};
use refact_core::model_caps::ModelCapabilities;
use refact_core::llm_types::WireFormat;
use crate::claude_code_oauth::OAuthTokens;
use crate::traits::{
    AvailableModel, CustomModelConfig, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, set_model_enabled_impl,
};

const SUPPORTS_CACHE_CONTROL: bool = true;
pub const CLAUDE_CODE_OAUTH_EXPIRES_AT_HEADER: &str =
    "x-refact-internal-claude-code-oauth-expires-at";

lazy_static::lazy_static! {
    static ref CLAUDE_CODE_REFRESH_GUARD: AMutex<()> = AMutex::new(());
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ClaudeCodeProvider {
    #[serde(flatten)]
    pub transport: ClaudeTransportConfig,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
    #[serde(default)]
    pub oauth_tokens: OAuthTokens,
    #[serde(default)]
    pub console_oauth_tokens: OAuthTokens,
}

impl std::fmt::Debug for ClaudeCodeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeCodeProvider")
            .field("backend", &self.transport.backend)
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

impl ClaudeCodeProvider {
    fn custom_models_with_metadata(
        &self,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let enabled: HashSet<&str> = self.enabled_models.iter().map(String::as_str).collect();
        let mut models = Vec::new();
        if self.transport.backend == ClaudeBackend::Bedrock {
            for id in self.custom_models.keys() {
                let Some(identity) = refact_core::claude_backend::bedrock_model(id) else {
                    continue;
                };
                let mut caps = model_caps
                    .get(&format!("amazon-bedrock/{id}"))
                    .or_else(|| {
                        model_caps.get(&format!("amazon-bedrock/{}", identity.foundation_id))
                    })
                    .cloned()
                    .or_else(|| {
                        resolve_claude_code_api_model_caps(model_caps, identity.anthropic_id)
                            .map(|resolved| resolved.caps)
                    })
                    .unwrap_or_else(|| identity.default_caps());
                caps.n_ctx = if self.transport.bedrock_context_1m_models.contains(id)
                    && identity.context != refact_core::claude_backend::BedrockContext::Standard
                {
                    1_000_000
                } else {
                    identity.default_n_ctx()
                };
                caps.tokenizer = refact_core::model_caps::ANTHROPIC_CLOUD_TOKENIZER.into();
                caps.supports_tools = true;
                caps.supports_vision = true;
                caps.supports_thinking_budget = !identity.adaptive_thinking;
                caps.supports_adaptive_thinking_budget = identity.adaptive_thinking;
                caps.reasoning_effort_options = if identity.adaptive_thinking {
                    Some(vec![
                        "low".into(),
                        "medium".into(),
                        "high".into(),
                        "max".into(),
                    ])
                } else if identity.anthropic_id == "claude-opus-4-5-20251101" {
                    Some(vec!["low".into(), "medium".into(), "high".into()])
                } else {
                    Some(Vec::new())
                };
                let mut model =
                    AvailableModel::from_caps(id, &caps, enabled.contains(id.as_str()), None);
                model.display_name = Some(identity.display_name.into());
                model.is_custom = true;
                model.live_fields = LiveModelFields {
                    display_name: model.display_name.clone(),
                    n_ctx: Some(model.n_ctx),
                    supports_tools: Some(model.supports_tools),
                    supports_multimodality: Some(model.supports_multimodality),
                    supports_thinking_budget: Some(model.supports_thinking_budget),
                    supports_adaptive_thinking_budget: Some(
                        model.supports_adaptive_thinking_budget,
                    ),
                    reasoning_effort_options: model.reasoning_effort_options.clone(),
                    tokenizer: model.tokenizer.clone(),
                    max_output_tokens: model.max_output_tokens,
                    ..Default::default()
                };
                models.push(model);
            }
        }
        merge_custom_models(&mut models, &self.custom_models, &enabled);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }
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
        let tokens = self.active_oauth_tokens().clone();
        let field = if self.is_console_oauth() {
            "console_oauth_tokens"
        } else {
            "oauth_tokens"
        };
        crate::config_store::update_provider_config(config_dir, instance_id, |existing| {
            let mut yaml_map = match existing {
                Some(value) => value.as_mapping().cloned().ok_or_else(|| {
                    "Config file root is not a YAML mapping. Cannot safely patch.".to_string()
                })?,
                None => serde_yaml::Mapping::new(),
            };

            let mut tokens_map = yaml_map
                .get(&serde_yaml::Value::String(field.to_string()))
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
                serde_yaml::Value::String("workspace_id".into()),
                serde_yaml::to_value(tokens.workspace_id)
                    .map_err(|_| "Invalid workspace metadata")?,
            );
            tokens_map.insert(
                serde_yaml::Value::String("oauth_mode".into()),
                serde_yaml::to_value(tokens.oauth_mode).map_err(|_| "Invalid OAuth mode")?,
            );
            yaml_map.insert(
                serde_yaml::Value::String(field.to_string()),
                serde_yaml::Value::Mapping(tokens_map),
            );

            Ok(serde_yaml::Value::Mapping(yaml_map))
        })
        .await
        .map(|_| ())
    }

    fn diagnose_auth_status(&self) -> String {
        if !self.supports_oauth() {
            return if self.has_credentials() {
                "Configured — credentials are checked when a request is made"
            } else {
                "Not configured — supply backend credentials"
            }
            .to_string();
        }
        if self.active_oauth_tokens().access_token.is_empty() {
            return "Not configured — log in via OAuth".to_string();
        }
        if self.active_oauth_tokens().is_expired() {
            return "OAuth token expired — needs refresh".to_string();
        }
        "OK (OAuth login)".to_string()
    }

    pub fn resolve_auth(&self) -> Result<String, String> {
        if !self.supports_oauth() {
            return Err("Select the subscription backend or Anthropic Console OAuth".into());
        }
        if self
            .active_oauth_tokens()
            .oauth_mode
            .as_ref()
            .is_some_and(|mode| Some(mode) != self.oauth_mode().as_ref())
        {
            return Err("Saved OAuth token mode does not match this backend; log in again".into());
        }
        if self.active_oauth_tokens().access_token.is_empty() {
            return Err("Claude Code: not logged in for this provider instance. \
                Click 'Login with Anthropic' in provider settings."
                .to_string());
        }
        if self.active_oauth_tokens().is_expired() {
            return Err("Claude Code: OAuth token expired — refresh needed.".to_string());
        }
        Ok(self.active_oauth_tokens().access_token.clone())
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
        (self.supports_oauth()
            && !self.active_oauth_tokens().access_token.is_empty()
            && self.active_oauth_tokens().access_token != rejected_access_token)
            .then(|| self.active_oauth_tokens().access_token.clone())
    }

    pub fn auth_state_matches(&self, tokens: &OAuthTokens) -> bool {
        self.active_oauth_tokens() == tokens
    }

    pub fn update_auth_state_from_if_current(
        &mut self,
        source: &ClaudeCodeProvider,
        previous_tokens: &OAuthTokens,
    ) -> bool {
        if !self.supports_oauth()
            || self.oauth_mode() != source.oauth_mode()
            || source.auth_state_matches(previous_tokens)
            || !self.auth_state_matches(previous_tokens)
        {
            return false;
        }
        self.set_active_oauth_tokens(source.active_oauth_tokens().clone());
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

pub const CLAUDE_CODE_RESET_PROGRAM_SAVED: &str = "cedar_ember";
pub const CLAUDE_CODE_RESET_PROGRAM_SESSION: &str = "juniper_tide";
const CLAUDE_CODE_RESET_STATUS_URL: &str =
    "https://api.anthropic.com/api/oauth/usage?cedar_ember=1&skip_spend=1";
const CLAUDE_CODE_PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const CLAUDE_CODE_RESET_CLAIM_TIMEOUT: Duration = Duration::from_secs(25);
const CLAUDE_CODE_RESET_STATUS_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeResetGrant {
    pub id: String,
    pub label: String,
    pub resets_total: u64,
    pub resets_left: u64,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub clears: Vec<String>,
    pub paused: bool,
    pub usable_now: bool,
    pub use_requires_limit: bool,
    pub percent_used: Map<String, Value>,
    pub blocking: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeSavedResets {
    pub eligible: bool,
    pub ineligible_reason: Option<String>,
    pub at_limit: bool,
    pub exhausted: Vec<String>,
    pub grants: Vec<ClaudeCodeResetGrant>,
    pub next_grant_id: Option<String>,
    pub weekly_resets_at: Option<String>,
    pub cooldown_until: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeSessionReset {
    pub eligible: bool,
    pub ineligible_reason: Option<String>,
    pub in_experiment: bool,
    pub arm: Option<String>,
    pub available: bool,
    pub next_available_at: Option<String>,
    pub weekly_resets_at: Option<String>,
    pub resets_per_week: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeResetStatus {
    pub cedar_ember: Option<ClaudeCodeSavedResets>,
    pub juniper_tide: Option<ClaudeCodeSessionReset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCodeResetRequest {
    pub program: String,
    pub grant_id: Option<String>,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeResetClaim {
    pub program: String,
    pub result: String,
    pub reason: Option<String>,
    pub resets_left: Option<u64>,
    pub cleared: Vec<String>,
    pub weekly_resets_at: Option<String>,
    pub cooldown_until: Option<String>,
    pub next_available_at: Option<String>,
}

impl ClaudeCodeResetRequest {
    pub fn parse(
        program: &str,
        grant_id: Option<&str>,
        request_id: Option<&str>,
    ) -> Result<Self, String> {
        let program = program.trim();
        let grant_id = grant_id.map(str::trim).filter(|value| !value.is_empty());
        let request_id = request_id.map(str::trim).filter(|value| !value.is_empty());
        match program {
            CLAUDE_CODE_RESET_PROGRAM_SAVED => {
                let grant_id = grant_id
                    .ok_or_else(|| "A saved reset needs the grant_id to use".to_string())?;
                if !is_claude_reset_token(grant_id, 40, false) {
                    return Err("grant_id is not a valid Claude Code reset grant id".to_string());
                }
                let request_id = request_id
                    .map(str::to_string)
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                if !is_claude_reset_token(&request_id, 64, true) {
                    return Err("request_id is not a valid Claude Code reset request id".to_string());
                }
                Ok(Self {
                    program: program.to_string(),
                    grant_id: Some(grant_id.to_string()),
                    request_id: Some(request_id),
                })
            }
            CLAUDE_CODE_RESET_PROGRAM_SESSION => Ok(Self {
                program: program.to_string(),
                grant_id: None,
                request_id: None,
            }),
            _ => Err(format!(
                "Unknown Claude Code reset program '{program}'; expected '{CLAUDE_CODE_RESET_PROGRAM_SAVED}' or '{CLAUDE_CODE_RESET_PROGRAM_SESSION}'"
            )),
        }
    }

    fn body(&self) -> Value {
        let mut body = json!({ "program": self.program });
        if let Some(grant_id) = &self.grant_id {
            body["grant_id"] = json!(grant_id);
        }
        if let Some(request_id) = &self.request_id {
            body["request_id"] = json!(request_id);
        }
        body
    }
}

fn is_claude_reset_token(value: &str, max_len: usize, allow_uppercase: bool) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.chars().all(|character| {
            character.is_ascii_digit()
                || character.is_ascii_lowercase()
                || (allow_uppercase && character.is_ascii_uppercase())
                || character == '_'
                || character == '-'
        })
}

impl ClaudeCodeProvider {
    fn oauth_request(
        http_client: &reqwest::Client,
        method: reqwest::Method,
        url: &str,
        access_token: &str,
        timeout: Duration,
    ) -> reqwest::RequestBuilder {
        http_client
            .request(method, url)
            .timeout(timeout)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("content-type", "application/json")
            .header(
                "user-agent",
                refact_llm::adapters::claude_code_compat::USER_AGENT,
            )
    }

    async fn oauth_json(
        request: reqwest::RequestBuilder,
        what: &str,
    ) -> Result<Value, ClaudeCodeUsageRequestError> {
        let resp = request.send().await.map_err(|error| {
            ClaudeCodeUsageRequestError::Other(format!("{what} request failed: {error}"))
        })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let truncated: String = body.chars().take(512).collect();
            return Err(ClaudeCodeUsageRequestError::Status(status, truncated));
        }
        resp.json().await.map_err(|error| {
            ClaudeCodeUsageRequestError::Other(format!("Failed to parse {what} response: {error}"))
        })
    }

    pub async fn fetch_reset_status_once(
        http_client: &reqwest::Client,
        access_token: &str,
    ) -> Result<ClaudeCodeResetStatus, ClaudeCodeUsageRequestError> {
        let request = Self::oauth_request(
            http_client,
            reqwest::Method::GET,
            CLAUDE_CODE_RESET_STATUS_URL,
            access_token,
            CLAUDE_CODE_RESET_STATUS_TIMEOUT,
        );
        let root = Self::oauth_json(request, "Claude Code reset status").await?;
        Ok(parse_claude_reset_status(&root))
    }

    pub async fn fetch_organization_uuid_once(
        http_client: &reqwest::Client,
        access_token: &str,
    ) -> Result<String, ClaudeCodeUsageRequestError> {
        let request = Self::oauth_request(
            http_client,
            reqwest::Method::GET,
            CLAUDE_CODE_PROFILE_URL,
            access_token,
            CLAUDE_CODE_RESET_STATUS_TIMEOUT,
        );
        let root = Self::oauth_json(request, "Claude Code profile").await?;
        root.get("organization")
            .and_then(|organization| organization.get("uuid"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|uuid| !uuid.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                ClaudeCodeUsageRequestError::Other(
                    "Claude Code profile has no organization; usage resets need an organization account".to_string(),
                )
            })
    }

    pub async fn claim_reset_once(
        http_client: &reqwest::Client,
        access_token: &str,
        request: &ClaudeCodeResetRequest,
    ) -> Result<ClaudeCodeResetClaim, ClaudeCodeUsageRequestError> {
        let organization_uuid =
            Self::fetch_organization_uuid_once(http_client, access_token).await?;
        let url = format!(
            "https://api.anthropic.com/api/organizations/{}/reset_rate_limits",
            organization_uuid
        );
        let http_request = Self::oauth_request(
            http_client,
            reqwest::Method::POST,
            &url,
            access_token,
            CLAUDE_CODE_RESET_CLAIM_TIMEOUT,
        )
        .json(&request.body());
        let root = Self::oauth_json(http_request, "Claude Code reset claim").await?;
        Ok(parse_claude_reset_claim(&request.program, &root))
    }

    pub fn reset_request_error_to_string(error: ClaudeCodeUsageRequestError) -> String {
        match error {
            ClaudeCodeUsageRequestError::Status(status, _)
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS =>
            {
                "Anthropic is rate limiting reset requests right now. Try again in a minute."
                    .to_string()
            }
            ClaudeCodeUsageRequestError::Status(status, body) => {
                format!("Claude Code reset request returned {}: {}", status, body)
            }
            ClaudeCodeUsageRequestError::Other(message) => message,
        }
    }
}

fn claude_string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn claude_optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn claude_u64(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(as_f64_loose)
        .filter(|number| number.is_finite() && *number >= 0.0 && number.fract() == 0.0)
        .map(|number| number as u64)
}

fn parse_claude_reset_grant(value: &Value) -> Option<ClaudeCodeResetGrant> {
    let grant = value.as_object()?;
    let id = claude_optional_string(grant.get("id"))?;
    let resets_left = claude_u64(grant.get("resets_left"))?;
    let percent_used = grant
        .get("percent_used")
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(key, value)| {
                    claude_u64(Some(value))
                        .filter(|percent| *percent <= 100)
                        .map(|percent| (key.clone(), Value::from(percent)))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ClaudeCodeResetGrant {
        id,
        label: grant
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        resets_total: claude_u64(grant.get("resets_total")).unwrap_or(0),
        resets_left,
        starts_at: claude_optional_string(grant.get("starts_at")),
        ends_at: claude_optional_string(grant.get("ends_at")),
        clears: claude_string_list(grant.get("clears")),
        paused: grant
            .get("paused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        usable_now: grant
            .get("usable_now")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        use_requires_limit: grant
            .get("use_requires_limit")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        percent_used,
        blocking: claude_string_list(grant.get("blocking")),
    })
}

fn parse_claude_saved_resets(value: &Value) -> Option<ClaudeCodeSavedResets> {
    let block = value.as_object()?;
    let eligible = block.get("eligible")?.as_bool()?;
    let grants: Vec<ClaudeCodeResetGrant> = block
        .get("grants")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(parse_claude_reset_grant).collect())
        .unwrap_or_default();
    let next_grant_id = claude_optional_string(block.get("next_grant_id"))
        .filter(|next| grants.iter().any(|grant| &grant.id == next));
    Some(ClaudeCodeSavedResets {
        eligible,
        ineligible_reason: claude_optional_string(block.get("ineligible_reason")),
        at_limit: block
            .get("at_limit")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        exhausted: claude_string_list(block.get("exhausted")),
        grants,
        next_grant_id,
        weekly_resets_at: claude_optional_string(block.get("weekly_resets_at")),
        cooldown_until: claude_optional_string(block.get("cooldown_until")),
    })
}

fn parse_claude_session_reset(value: &Value) -> Option<ClaudeCodeSessionReset> {
    let block = value.as_object()?;
    let eligible = block.get("eligible")?.as_bool()?;
    Some(ClaudeCodeSessionReset {
        eligible,
        ineligible_reason: claude_optional_string(block.get("ineligible_reason")),
        in_experiment: block
            .get("in_experiment")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        arm: claude_optional_string(block.get("arm")),
        available: block
            .get("available")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        next_available_at: claude_optional_string(block.get("next_available_at")),
        weekly_resets_at: claude_optional_string(block.get("weekly_resets_at")),
        resets_per_week: claude_u64(block.get("resets_per_week")).unwrap_or(1),
    })
}

pub fn parse_claude_reset_status(root: &Value) -> ClaudeCodeResetStatus {
    let data = root.get("data").unwrap_or(root);
    ClaudeCodeResetStatus {
        cedar_ember: data
            .get(CLAUDE_CODE_RESET_PROGRAM_SAVED)
            .and_then(parse_claude_saved_resets),
        juniper_tide: data
            .get(CLAUDE_CODE_RESET_PROGRAM_SESSION)
            .and_then(parse_claude_session_reset),
    }
}

pub fn parse_claude_reset_claim(program: &str, root: &Value) -> ClaudeCodeResetClaim {
    let data = root.get("data").unwrap_or(root);
    ClaudeCodeResetClaim {
        program: program.to_string(),
        result: claude_optional_string(data.get("result"))
            .unwrap_or_else(|| "unavailable".to_string()),
        reason: claude_optional_string(data.get("reason")),
        resets_left: claude_u64(data.get("resets_left")),
        cleared: claude_string_list(data.get("cleared")),
        weekly_resets_at: claude_optional_string(data.get("weekly_resets_at")),
        cooldown_until: claude_optional_string(data.get("cooldown_until")),
        next_available_at: claude_optional_string(data.get("next_available_at")),
    }
}

impl ClaudeCodeProvider {
    pub async fn fetch_usage(
        &self,
        http_client: &reqwest::Client,
    ) -> Result<ClaudeCodeUsage, String> {
        if !self.is_subscription() {
            return Err("Usage is only available for Claude subscriptions".into());
        }
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
        if !self.is_subscription() {
            return Err(ClaudeCodeUsageRequestError::Other(
                "Usage is only available for Claude subscriptions".into(),
            ));
        }
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
        // The Anthropic API is the source of truth for which ids exist. Filtering
        // by an id prefix silently drops models the account can actually use.
        None
    }

    fn provider_schema(&self) -> &'static str {
        crate::claude_backend::SCHEMA
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        self.apply_shared_settings(yaml)
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        let auth_status = self.diagnose_auth_status();
        let oauth_connected = self.supports_oauth() && self.resolve_auth().is_ok();

        let mut settings = serde_json::to_value(&self.transport).unwrap_or_else(|_| json!({}));
        for field in crate::claude_backend::SECRET_FIELDS {
            let mask = settings
                .get(*field)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|_| "***")
                .unwrap_or("");
            settings[*field] = json!(mask);
        }
        settings["enabled"] = json!(self.enabled);
        settings["auth_status"] = json!(auth_status);
        settings["oauth_connected"] = json!(oauth_connected);
        settings["enabled_models"] = json!(self.enabled_models);
        settings["custom_models"] = json!(self.custom_models);
        settings
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        self.validate_settings()?;
        let auth_token = if !self.is_subscription() {
            String::new()
        } else {
            match self.resolve_auth() {
                Ok(token) => token,
                Err(e) => {
                    if self.enabled {
                        tracing::warn!("Claude Code auth failed: {}", e);
                    }
                    String::new()
                }
            }
        };
        let has_auth = if self.is_subscription() {
            !auth_token.is_empty()
        } else {
            self.has_credentials()
        };

        let mut extra_headers = HashMap::new();
        if self.is_subscription() {
            extra_headers.insert(
                CLAUDE_CODE_OAUTH_EXPIRES_AT_HEADER.to_string(),
                self.oauth_tokens.expires_at.to_string(),
            );
        } else {
            extra_headers.insert(
                CLAUDE_TRANSPORT_HEADER.to_string(),
                serde_json::to_string(&self.resolved_transport())
                    .map_err(|_| "Invalid Claude transport")?,
            );
        }

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
        use refact_core::claude_backend::ClaudeAuthType;
        let t = self.resolved_transport();
        let present = |v: &Option<String>| v.as_ref().is_some_and(|s| !s.is_empty());
        match t.backend {
            ClaudeBackend::Oauth => !self.oauth_tokens.access_token.is_empty(),
            ClaudeBackend::Anthropic => {
                if self.is_console_oauth() {
                    self.resolve_auth().is_ok()
                } else {
                    present(&t.api_key)
                }
            }
            ClaudeBackend::Bedrock | ClaudeBackend::Vertex | ClaudeBackend::Foundry => true,
            ClaudeBackend::Custom => {
                present(&t.base_url)
                    && match t.auth_type {
                        ClaudeAuthType::ApiKey => present(&t.api_key),
                        ClaudeAuthType::Bearer => present(&t.auth_token),
                        ClaudeAuthType::None => true,
                        ClaudeAuthType::Oauth => false,
                    }
            }
        }
    }

    fn model_source(&self) -> ModelSource {
        if (self.is_subscription() && self.resolve_auth().is_ok())
            || (self.transport.backend == ClaudeBackend::Anthropic && self.has_credentials())
        {
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

    /// The models.dev catalog must never contribute an id for this provider:
    /// the Anthropic API is the only authority on what the subscription
    /// serves. `model_filter_regex` is intentionally `None` and the catalog
    /// has no `claude_code/` keys, so without this override the
    /// discovery-timeout path (caps.rs / providers/http.rs) would enumerate
    /// every bare catalog id — gpt-5, gemini-*, deepseek-* — as a Claude Code
    /// model. Custom models only.
    fn get_available_models_from_caps(
        &self,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        self.custom_models_with_metadata(model_caps)
    }

    fn get_custom_models_only(&self) -> Vec<AvailableModel> {
        self.custom_models_with_metadata(&HashMap::new())
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        // No catalog fallback: when the Anthropic API cannot be reached the only
        // models we can honestly offer are the user's own custom entries.
        let custom_models_only = || self.custom_models_with_metadata(model_caps);
        if !self.supports_live_discovery() {
            return custom_models_only();
        }
        let transport = self.resolved_transport();
        let result = if self.supports_oauth() {
            match self.resolve_auth() {
                Ok(token) => fetch_claude_code_models(http_client, &token).await,
                Err(_) => return custom_models_only(),
            }
        } else {
            let Some(key) = transport.api_key.as_deref().filter(|s| !s.is_empty()) else {
                return custom_models_only();
            };
            let url = format!(
                "{}/models",
                transport
                    .base_url
                    .as_deref()
                    .unwrap_or("https://api.anthropic.com")
                    .trim_end_matches('/')
                    .trim_end_matches("/v1")
                    .to_string()
                    + "/v1"
            );
            fetch_claude_models(http_client, key, &url, false).await
        };
        let api_models = match result {
            Ok(models) => models,
            Err(e) => {
                match &e {
                    ClaudeCodeModelsError::Http(message) => tracing::warn!(
                        "Claude Code: model discovery network failure ({}); \
                         serving custom models only",
                        message
                    ),
                    ClaudeCodeModelsError::Status(status, body) => tracing::warn!(
                        "Claude Code: model discovery HTTP {} ({}); serving custom models only",
                        status,
                        body
                    ),
                    ClaudeCodeModelsError::Payload(message) => tracing::warn!(
                        "Claude Code: model discovery returned an unusable payload ({}); \
                         serving custom models only",
                        message
                    ),
                }
                return custom_models_only();
            }
        };

        tracing::info!("Claude Code: API returned {} models", api_models.len());
        claude_available_models_from_live(self, &api_models, model_caps)
    }

    fn custom_model_pricing(&self, model_id: &str) -> Option<crate::traits::ModelPricing> {
        self.custom_models
            .get(model_id)
            .and_then(|config| config.pricing.clone())
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
        if !self.supports_oauth() {
            return;
        }
        let mut tokens = self.active_oauth_tokens().clone();
        tokens.access_token = access_token.to_string();
        tokens.refresh_token = refresh_token.to_string();
        tokens.expires_at = expires_at;
        self.set_active_oauth_tokens(tokens);
    }

    async fn startup_refresh_and_sync(
        &mut self,
        http_client: &reqwest::Client,
        config_dir: &std::path::Path,
        instance_id: &str,
    ) -> Result<(), String> {
        let Some(mode) = self.oauth_mode() else {
            return Ok(());
        };
        let previous = self.active_oauth_tokens().clone();
        if previous.is_empty()
            || previous.refresh_token.is_empty()
            || !Self::needs_refresh_on_start(previous.expires_at)
        {
            return Ok(());
        }
        let _guard = Self::lock_refresh_guard().await?;
        let mut refreshed = match crate::claude_code_oauth::refresh_access_token_for_mode(
            http_client,
            &previous.refresh_token,
            &mode,
        )
        .await
        {
            Ok(tokens) => tokens,
            Err(e) if crate::oauth_refresh::is_permanent_refresh_error(&e) => {
                crate::oauth_refresh::mark_invalid_refresh_token(
                    instance_id,
                    &previous.refresh_token,
                );
                self.set_active_oauth_tokens(OAuthTokens::default());
                return self.save_oauth_tokens_config(config_dir, instance_id).await;
            }
            Err(e) => return Err(e),
        };
        if refreshed.workspace_id.is_none() {
            refreshed.workspace_id = previous.workspace_id;
        }
        self.set_active_oauth_tokens(refreshed);
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

/// Canonical order the UI expects reasoning effort levels in.
const CLAUDE_EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// `capabilities` object of an Anthropic `/v1/models` record.
fn claude_capabilities(model: &Value) -> Option<&Value> {
    model.get("capabilities")
}

/// Anthropic reports every capability as `{ "supported": bool }`.
fn claude_capability_supported(capabilities: Option<&Value>, key: &str) -> Option<bool> {
    capabilities?
        .get(key)?
        .get("supported")
        .and_then(Value::as_bool)
}

/// `capabilities.effort` -> reasoning effort options.
///
/// `effort.supported == false` is authoritative "this model has no effort
/// control", which is represented as `Some(Vec::new())` so it overrides any
/// catalog value (see `LiveModelFields` docs: an empty vec is a real value,
/// `None` means "the endpoint said nothing").
fn claude_reasoning_effort_options(capabilities: Option<&Value>) -> Option<Vec<String>> {
    let effort = capabilities?.get("effort")?;
    if effort.get("supported").and_then(Value::as_bool) != Some(true) {
        return Some(Vec::new());
    }
    Some(
        CLAUDE_EFFORT_LEVELS
            .iter()
            .filter(|level| {
                effort
                    .get(**level)
                    .and_then(|value| value.get("supported"))
                    .and_then(Value::as_bool)
                    == Some(true)
            })
            .map(|level| (*level).to_string())
            .collect(),
    )
}

/// `capabilities.thinking.types.<kind>.supported`.
fn claude_thinking_type_supported(capabilities: Option<&Value>, kind: &str) -> Option<bool> {
    capabilities?
        .get("thinking")?
        .get("types")?
        .get(kind)?
        .get("supported")
        .and_then(Value::as_bool)
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
        let capabilities = claude_capabilities(api_model);
        let image_input = claude_capability_supported(capabilities, "image_input");
        let thinking_enabled = claude_thinking_type_supported(capabilities, "enabled");
        let live = LiveModelFields {
            display_name: claude_live_string(api_model, &["display_name", "displayName"]).or_else(
                || {
                    resolved
                        .as_ref()
                        .and_then(|caps| (api_id != caps.matched_key).then(|| api_id.clone()))
                },
            ),
            // Anthropic reports the context window as `max_input_tokens` and the
            // output cap as `max_tokens`; the other names are legacy aliases.
            n_ctx: claude_live_usize(
                api_model,
                &[
                    "max_input_tokens",
                    "context_window",
                    "contextWindow",
                    "max_context_window",
                ],
            ),
            max_output_tokens: claude_live_usize(
                api_model,
                &["max_tokens", "max_output_tokens", "maxOutputTokens"],
            ),
            supports_cache_control: Some(SUPPORTS_CACHE_CONTROL),
            pricing: provider.custom_model_pricing(&api_id),
            supports_strict_tools: claude_capability_supported(capabilities, "structured_outputs"),
            supports_multimodality: image_input.or_else(|| resolved.is_none().then_some(true)),
            supports_pdf: claude_capability_supported(capabilities, "pdf_input"),
            reasoning_effort_options: claude_reasoning_effort_options(capabilities),
            supports_thinking_budget: thinking_enabled
                .or_else(|| resolved.is_none().then_some(true)),
            supports_adaptive_thinking_budget: claude_thinking_type_supported(
                capabilities,
                "adaptive",
            ),
            supports_tools: resolved.is_none().then_some(true),
            supports_parallel_tools: resolved.is_none().then_some(true),
            ..Default::default()
        };
        let model = available_model_from_catalog_and_live(
            &api_id,
            resolved.as_ref().map(|caps| &caps.caps),
            &live,
            claude_live_model_is_enabled(&enabled_set, &api_id, api_id_without_date),
            // This provider never fabricates a context size.
            0,
        );
        if model.n_ctx == 0 {
            // Neither the live payload nor models.dev supplied a context window.
            // Unreachable while Anthropic sends `max_input_tokens` on every
            // record; a loud backstop for the next time that key is renamed.
            tracing::warn!(
                "Claude Code: dropping model '{}' — no context window from the API \
                 (`max_input_tokens` missing) and none in model capabilities",
                api_id
            );
            continue;
        }
        models.push(model);
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

/// Failure classes of the Anthropic `/v1/models` call, so callers can log a
/// distinguishable message instead of one opaque string.
#[derive(Debug, Clone)]
pub enum ClaudeCodeModelsError {
    /// Transport-level failure (DNS, TLS, timeout, no route) or a malformed URL.
    Http(String),
    /// The endpoint answered with a non-2xx status; body is truncated.
    Status(reqwest::StatusCode, String),
    /// 2xx but the body was not usable JSON, or the `data` array was missing,
    /// or pagination was inconsistent.
    Payload(String),
}

impl std::fmt::Display for ClaudeCodeModelsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(message) => write!(f, "network error: {message}"),
            Self::Status(status, body) => write!(f, "HTTP {status}: {body}"),
            Self::Payload(message) => write!(f, "bad payload: {message}"),
        }
    }
}

/// Fetch live model records from the Anthropic API using OAuth credentials.
pub async fn fetch_claude_code_models(
    http_client: &reqwest::Client,
    auth_token: &str,
) -> Result<Vec<Value>, ClaudeCodeModelsError> {
    fetch_claude_models(http_client, auth_token, ANTHROPIC_MODELS_URL, true).await
}

async fn fetch_claude_models(
    http_client: &reqwest::Client,
    auth_token: &str,
    endpoint: &str,
    subscription: bool,
) -> Result<Vec<Value>, ClaudeCodeModelsError> {
    if auth_token.is_empty() {
        return Err(ClaudeCodeModelsError::Http("empty auth token".to_string()));
    }

    let betas = refact_llm::adapters::claude_code_compat::CC_OAUTH_BETAS.join(",");
    let mut models = Vec::new();
    let mut after_id: Option<String> = None;
    for _ in 0..ANTHROPIC_MODELS_MAX_PAGES {
        let mut url = reqwest::Url::parse(endpoint).map_err(|error| {
            ClaudeCodeModelsError::Http(format!("Failed to build Claude Code models URL: {error}"))
        })?;
        url.query_pairs_mut().append_pair("limit", "1000");
        if let Some(cursor) = after_id.as_deref() {
            url.query_pairs_mut().append_pair("after_id", cursor);
        }
        let request = http_client
            .get(url)
            .timeout(std::time::Duration::from_secs(8))
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json");
        let request = if subscription {
            request
                .header("Authorization", format!("Bearer {}", auth_token))
                .header("anthropic-beta", betas.as_str())
                .header(
                    "user-agent",
                    refact_llm::adapters::claude_code_compat::USER_AGENT,
                )
        } else {
            request.header("x-api-key", auth_token)
        };
        let response = request.send().await.map_err(|e| {
            ClaudeCodeModelsError::Http(format!("Failed to fetch Claude Code models: {}", e))
        })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let truncated: String = body.chars().take(512).collect();
            return Err(ClaudeCodeModelsError::Status(status, truncated));
        }
        let json = response.json::<Value>().await.map_err(|e| {
            ClaudeCodeModelsError::Payload(format!(
                "Failed to parse Claude Code models response: {}",
                e
            ))
        })?;
        let page = json.get("data").and_then(Value::as_array).ok_or_else(|| {
            ClaudeCodeModelsError::Payload(
                "Claude Code models response missing data array".to_string(),
            )
        })?;
        models.extend(page.iter().cloned());
        let Some(next_cursor) = claude_models_next_cursor(&json)? else {
            return Ok(models);
        };
        if after_id.as_deref() == Some(next_cursor.as_str()) {
            return Err(ClaudeCodeModelsError::Payload(
                "Claude Code models response repeated pagination cursor".to_string(),
            ));
        }
        after_id = Some(next_cursor);
    }
    Err(ClaudeCodeModelsError::Payload(format!(
        "Claude Code models API exceeded {} pagination pages",
        ANTHROPIC_MODELS_MAX_PAGES
    )))
}

fn claude_models_next_cursor(response: &Value) -> Result<Option<String>, ClaudeCodeModelsError> {
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
        .ok_or_else(|| {
            ClaudeCodeModelsError::Payload(
                "Claude Code models response has_more without last_id".to_string(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bedrock_custom_metadata_infers_identity_without_catalog_offers() {
        let ids = [
            (
                "au.anthropic.claude-haiku-4-5-20251001-v1:0",
                "Claude Haiku 4.5",
                false,
            ),
            (
                "au.anthropic.claude-sonnet-4-5-20250929-v1:0",
                "Claude Sonnet 4.5",
                false,
            ),
            (
                "global.anthropic.claude-opus-4-5-20251101-v1:0",
                "Claude Opus 4.5",
                false,
            ),
            ("au.anthropic.claude-opus-4-6-v1", "Claude Opus 4.6", true),
        ];
        let mut provider = ClaudeCodeProvider::default();
        provider.transport.backend = ClaudeBackend::Bedrock;
        for (id, _, _) in ids {
            provider
                .custom_models
                .insert(id.into(), CustomModelConfig::default());
        }
        let caps = HashMap::from([
            (
                format!("amazon-bedrock/{}", ids[3].0),
                ModelCapabilities {
                    n_ctx: 1_000_000,
                    max_output_tokens: 12_345,
                    tokenizer: "fake".into(),
                    ..Default::default()
                },
            ),
            (
                "anthropic/claude-opus-4-6".into(),
                ModelCapabilities {
                    max_output_tokens: 999,
                    ..Default::default()
                },
            ),
            (
                "amazon-bedrock/unoffered".into(),
                ModelCapabilities::default(),
            ),
        ]);
        let models = provider.get_available_models_from_caps(&caps);
        assert_eq!(models.len(), 4);
        for (id, name, adaptive) in ids {
            let model = models.iter().find(|model| model.id == id).unwrap();
            assert_eq!(model.display_name.as_deref(), Some(name));
            assert_eq!(model.n_ctx, 200_000);
            assert!(model.supports_tools && model.supports_multimodality);
            assert!(model.is_custom);
            assert_eq!(model.supports_thinking_budget, !adaptive);
            assert_eq!(model.live_fields.supports_thinking_budget, Some(!adaptive));
            assert_eq!(model.live_fields.tokenizer.as_deref(), Some("anthropic"));
            assert_eq!(model.supports_adaptive_thinking_budget, adaptive);
            assert_eq!(model.tokenizer.as_deref(), Some("anthropic"));
            let expected_effort = if adaptive {
                vec!["low", "medium", "high", "max"]
            } else if id == ids[2].0 {
                vec!["low", "medium", "high"]
            } else {
                Vec::new()
            };
            assert_eq!(
                model
                    .reasoning_effort_options
                    .as_ref()
                    .map(|options| options.iter().map(String::as_str).collect::<Vec<_>>()),
                Some(expected_effort)
            );
        }
        assert_eq!(
            models
                .iter()
                .find(|m| m.id == ids[3].0)
                .unwrap()
                .max_output_tokens,
            Some(12_345)
        );
        let config = provider.custom_models.get_mut(ids[3].0).unwrap();
        config.supports_tools = Some(false);
        config.supports_multimodality = Some(false);
        config.supports_thinking_budget = Some(false);
        config.reasoning_effort_options = Some(Vec::new());
        config.n_ctx = Some(1234);
        config.tokenizer = Some(String::new());
        let overridden = provider.get_available_models_from_caps(&caps);
        let model = overridden.iter().find(|m| m.id == ids[3].0).unwrap();
        assert!(
            !model.supports_tools
                && !model.supports_multimodality
                && !model.supports_thinking_budget
        );
        assert_eq!(model.reasoning_effort_options, Some(Vec::new()));
        assert_eq!(model.tokenizer.as_deref(), Some(""));
        assert_eq!(model.n_ctx, 1234);
    }
    #[test]
    fn caps_path_never_contributes_a_catalog_id() {
        let mut provider = ClaudeCodeProvider::default();
        provider.custom_models.insert(
            "my-custom".to_string(),
            CustomModelConfig {
                n_ctx: Some(4096),
                ..Default::default()
            },
        );
        let claude = ModelCapabilities {
            n_ctx: 200_000,
            tokenizer: "claude".to_string(),
            ..Default::default()
        };
        let model_caps = HashMap::from([
            ("claude-sonnet-4-5".to_string(), claude.clone()),
            ("anthropic/claude-sonnet-4-5".to_string(), claude),
            (
                "gpt-5".to_string(),
                ModelCapabilities {
                    n_ctx: 400_000,
                    ..Default::default()
                },
            ),
        ]);

        let ids: Vec<_> = provider
            .get_available_models_from_caps(&model_caps)
            .into_iter()
            .map(|model| model.id)
            .collect();

        // The Anthropic API is the only source of ids. The discovery-timeout
        // path must yield custom models only - never a catalog id, Anthropic
        // or otherwise.
        assert_eq!(ids, vec!["my-custom".to_string()]);
    }

    /// A record shaped exactly like a real `GET /v1/models` entry.
    ///
    /// Verified live against `https://api.anthropic.com/v1/models?limit=1000`:
    /// context size arrives as `max_input_tokens`, the output cap as
    /// `max_tokens`, and every capability is `{ "supported": bool }`.
    fn api_model(
        id: &str,
        max_input_tokens: u64,
        max_tokens: u64,
        effort_supported: bool,
        thinking_enabled: bool,
        thinking_adaptive: bool,
    ) -> Value {
        json!({
            "id": id,
            "type": "model",
            "display_name": format!("Live {id}"),
            "created_at": "2025-05-14T00:00:00Z",
            "max_input_tokens": max_input_tokens,
            "max_tokens": max_tokens,
            "capabilities": {
                "batch": { "supported": true },
                "citations": { "supported": true },
                "code_execution": { "supported": true },
                "context_management": { "supported": true },
                "effort": {
                    "supported": effort_supported,
                    "low": { "supported": effort_supported },
                    "medium": { "supported": effort_supported },
                    "high": { "supported": effort_supported },
                    "xhigh": { "supported": false },
                    "max": { "supported": effort_supported }
                },
                "image_input": { "supported": true },
                "pdf_input": { "supported": true },
                "structured_outputs": { "supported": true },
                "thinking": {
                    "supported": thinking_enabled || thinking_adaptive,
                    "types": {
                        "enabled": { "supported": thinking_enabled },
                        "adaptive": { "supported": thinking_adaptive }
                    }
                }
            }
        })
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
        let live = vec![api_model(
            "claude-sonnet-4-20250514",
            250_000,
            32_000,
            true,
            true,
            false,
        )];

        let models = claude_available_models_from_live(&provider, &live, &model_caps);
        let model = models
            .iter()
            .find(|model| model.id == "claude-sonnet-4-20250514")
            .unwrap();
        assert!(model.supports_tools);
        assert_eq!(
            model.display_name.as_deref(),
            Some("Live claude-sonnet-4-20250514")
        );
        assert_eq!(model.n_ctx, 250_000);
        assert_eq!(model.max_output_tokens, Some(32_000));
        assert!(model.supports_cache_control);

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
    fn claude_live_models_without_any_context_window_are_dropped_not_faked() {
        let provider = ClaudeCodeProvider::default();
        // No `max_input_tokens` and no catalog entry: we must not invent 200k.
        let models = claude_available_models_from_live(
            &provider,
            &[json!({ "id": "claude-mystery-9", "max_tokens": 64_000 })],
            &HashMap::new(),
        );

        assert!(
            models.is_empty(),
            "a model with no context window from any source must be skipped"
        );
    }

    #[test]
    fn claude_live_capabilities_map_onto_live_model_fields() {
        let provider = ClaudeCodeProvider::default();
        // opus-4-6-shaped: enabled=true, adaptive=true, effort supported.
        let live = vec![api_model(
            "claude-opus-4-6",
            200_000,
            64_000,
            true,
            true,
            true,
        )];

        let models = claude_available_models_from_live(&provider, &live, &HashMap::new());
        let model = &models[0];

        assert!(model.supports_multimodality);
        assert!(model.supports_pdf);
        assert!(model.supports_strict_tools);
        assert!(model.supports_thinking_budget);
        assert!(model.supports_adaptive_thinking_budget);
        assert_eq!(
            model.reasoning_effort_options,
            Some(vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
                "max".to_string(),
            ]),
            "effort options keep the canonical low/medium/high/xhigh/max order \
             and drop unsupported levels"
        );
        assert!(model.supports_cache_control);
    }

    #[test]
    fn claude_live_models_without_effort_support_get_no_effort_options() {
        let provider = ClaudeCodeProvider::default();
        // haiku-4-5 / sonnet-4-5 shape: effort.supported == false.
        let live = vec![api_model(
            "claude-haiku-4-5-20251001",
            200_000,
            64_000,
            false,
            true,
            false,
        )];
        // Catalog claims effort options; the API's authoritative "no effort"
        // must win.
        let model_caps = HashMap::from([(
            "anthropic/claude-haiku-4-5".to_string(),
            ModelCapabilities {
                n_ctx: 200_000,
                reasoning_effort_options: Some(vec!["high".to_string()]),
                tokenizer: "claude".to_string(),
                ..Default::default()
            },
        )]);

        let models = claude_available_models_from_live(&provider, &live, &model_caps);
        let model = &models[0];

        assert_eq!(model.reasoning_effort_options, Some(Vec::new()));
        assert_eq!(model.live_fields.reasoning_effort_options, Some(Vec::new()));
        assert!(model.supports_thinking_budget);
        assert!(!model.supports_adaptive_thinking_budget);
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
    fn claude_code_reset_request_validates_programs_and_ids() {
        let saved =
            ClaudeCodeResetRequest::parse(" cedar_ember ", Some("launch_2026-09"), None).unwrap();
        assert_eq!(saved.program, CLAUDE_CODE_RESET_PROGRAM_SAVED);
        assert_eq!(saved.grant_id.as_deref(), Some("launch_2026-09"));
        let request_id = saved.request_id.clone().unwrap();
        assert!(uuid::Uuid::parse_str(&request_id).is_ok());
        assert_eq!(
            saved.body(),
            json!({
                "program": "cedar_ember",
                "grant_id": "launch_2026-09",
                "request_id": request_id
            })
        );

        let explicit =
            ClaudeCodeResetRequest::parse("cedar_ember", Some("launch_2026-09"), Some("Req_ID-42"))
                .unwrap();
        assert_eq!(explicit.request_id.as_deref(), Some("Req_ID-42"));

        let session = ClaudeCodeResetRequest::parse("juniper_tide", Some("ignored"), None).unwrap();
        assert_eq!(session.program, CLAUDE_CODE_RESET_PROGRAM_SESSION);
        assert!(session.grant_id.is_none() && session.request_id.is_none());
        assert_eq!(session.body(), json!({ "program": "juniper_tide" }));

        assert!(ClaudeCodeResetRequest::parse("cedar_ember", None, None).is_err());
        assert!(ClaudeCodeResetRequest::parse("cedar_ember", Some("Bad Grant"), None).is_err());
        assert!(ClaudeCodeResetRequest::parse("cedar_ember", Some(&"a".repeat(41)), None).is_err());
        assert!(
            ClaudeCodeResetRequest::parse("cedar_ember", Some("ok"), Some("no spaces")).is_err()
        );
        assert!(ClaudeCodeResetRequest::parse("mystery", None, None).is_err());
    }

    #[test]
    fn claude_code_auth_state_update_is_compare_and_swap() {
        let previous_tokens = OAuthTokens {
            access_token: "old-access".to_string(),
            refresh_token: "old-refresh".to_string(),
            expires_at: 1,
            ..Default::default()
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
                oauth_mode: Some(crate::claude_code_oauth::OAuthMode::Max),
                ..Default::default()
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
}
