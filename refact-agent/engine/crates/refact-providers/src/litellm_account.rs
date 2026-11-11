use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;
use std::time::Duration;

use refact_core::provider_types::CredentialSpec;
use serde::Serialize;
use serde_json::Value;

use crate::credential;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const MAX_ACCOUNT_BODY_BYTES: usize = 256 * 1024;
const MAX_TEXT_CHARS: usize = 256;
const MAX_MODEL_LIMITS: usize = 200;
const MAX_MCP_SERVERS: usize = 500;
const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiteLLMFeatureState {
    Ok,
    Unsupported,
    PermissionDenied,
    AuthRequired,
    NotConfigured,
    Unavailable,
    MalformedResponse,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiteLLMFailure {
    pub state: LiteLLMFeatureState,
    pub message: String,
}

impl LiteLLMFailure {
    pub(crate) fn new(state: LiteLLMFeatureState, message: &str) -> Self {
        Self {
            state,
            message: message.to_string(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LiteLLMBearer(String);

impl LiteLLMBearer {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for LiteLLMBearer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiteLLMBearer([REDACTED])")
    }
}

pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn native_client() -> Result<&'static reqwest::Client, LiteLLMFailure> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(REQUEST_TIMEOUT)
                .build()
                .ok()
        })
        .as_ref()
        .ok_or_else(|| {
            LiteLLMFailure::new(LiteLLMFeatureState::Unavailable, "HTTP client unavailable")
        })
}

pub(crate) fn validate_base_url(endpoint: &str) -> Result<String, LiteLLMFailure> {
    let invalid = || {
        LiteLLMFailure::new(
            LiteLLMFeatureState::NotConfigured,
            "LiteLLM endpoint must be HTTPS (HTTP only for loopback) without credentials, query or fragment",
        )
    };
    let normalized = crate::litellm::normalize_base_url(endpoint);
    let url = reqwest::Url::parse(&normalized).map_err(|_| invalid())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let host = url.host_str().ok_or_else(invalid)?;
    let loopback = matches!(host, "localhost" | "[::1]" | "::1")
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback());
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(invalid());
    }
    Ok(normalized)
}

pub(crate) struct NativeSession {
    pub(crate) base: String,
    pub(crate) bearer: LiteLLMBearer,
    instance_id: String,
    credential: Option<CredentialSpec>,
    extra_headers: HashMap<String, String>,
    secrets: Vec<String>,
    refreshed: bool,
    key_not_found: bool,
}

impl NativeSession {
    pub(crate) fn new(
        base: String,
        bearer: LiteLLMBearer,
        instance_id: &str,
        credential: Option<CredentialSpec>,
        extra_headers: HashMap<String, String>,
    ) -> Self {
        let mut secrets = vec![bearer.expose().to_string()];
        secrets.extend(
            extra_headers
                .iter()
                .filter(|(name, _)| is_sensitive_header(name))
                .map(|(_, value)| value.clone()),
        );
        Self {
            base,
            bearer,
            instance_id: instance_id.to_string(),
            credential,
            extra_headers,
            secrets,
            refreshed: false,
            key_not_found: false,
        }
    }

    async fn send(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<reqwest::Response, LiteLLMFailure> {
        let mut url = reqwest::Url::parse(&format!("{}{}", self.base, path)).map_err(|_| {
            LiteLLMFailure::new(LiteLLMFeatureState::NotConfigured, "Invalid LiteLLM URL")
        })?;
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        let mut request = native_client()?
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json");
        for (name, value) in &self.extra_headers {
            if is_auth_header(name) {
                continue;
            }
            request = request.header(name.as_str(), value.as_str());
        }
        request
            .bearer_auth(self.bearer.expose())
            .send()
            .await
            .map_err(|_| {
                LiteLLMFailure::new(LiteLLMFeatureState::Unavailable, "LiteLLM request failed")
            })
    }

    pub(crate) fn pin_identity(&mut self) {
        self.refreshed = true;
    }

    pub(crate) async fn get_json(
        &mut self,
        path: &str,
        query: &[(&str, String)],
        max_bytes: usize,
    ) -> Result<Value, LiteLLMFailure> {
        self.key_not_found = false;
        let mut response = self.send(path, query).await?;
        if response.status().as_u16() == 401 && !self.refreshed {
            if let Some(spec) = &self.credential {
                self.refreshed = true;
                let refreshed = credential::refresh_after_rejection(
                    &self.instance_id,
                    spec,
                    self.bearer.expose(),
                )
                .await
                .map_err(|_| {
                    LiteLLMFailure::new(
                        LiteLLMFeatureState::AuthRequired,
                        "Credential command refresh failed",
                    )
                })?;
                self.secrets.push(refreshed.clone());
                self.bearer = LiteLLMBearer::new(refreshed);
                response = self.send(path, query).await?;
            }
        }
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            if status == 404 && path == "/key/info" && query.is_empty() {
                if let Ok(body) = read_bounded(response, MAX_ERROR_BODY_BYTES).await {
                    self.key_not_found =
                        serde_json::from_slice::<Value>(&body)
                            .ok()
                            .is_some_and(|value| {
                                let error = &value["error"];
                                error["type"] == "not_found_error"
                                    && error["message"] == "Key not found in database"
                                    && error["code"] == "404"
                            });
                }
                if self.key_not_found {
                    self.pin_identity();
                    return Err(missing_key_failure());
                }
            }
            return Err(status_failure(status));
        }
        let body = read_bounded(response, max_bytes).await?;
        let mut value: Value = serde_json::from_slice(&body).map_err(|_| {
            LiteLLMFailure::new(
                LiteLLMFeatureState::MalformedResponse,
                "Invalid JSON response",
            )
        })?;
        for secret in &self.secrets {
            if secret.len() >= 4 {
                redact_secret(&mut value, secret);
            }
        }
        Ok(value)
    }
}

fn missing_key_failure() -> LiteLLMFailure {
    LiteLLMFailure::new(
        LiteLLMFeatureState::Unavailable,
        "Current bearer has no virtual-key record",
    )
}

pub(crate) fn permits_self_user(session: &NativeSession) -> bool {
    session.key_not_found
}

pub(crate) fn status_failure(status: u16) -> LiteLLMFailure {
    use LiteLLMFeatureState::*;
    match status {
        300..=399 => LiteLLMFailure::new(Unavailable, "LiteLLM redirect refused"),
        401 => LiteLLMFailure::new(AuthRequired, "LiteLLM rejected the API key"),
        403 => LiteLLMFailure::new(PermissionDenied, "Key is not permitted to read this data"),
        404 | 405 | 501 => LiteLLMFailure::new(Unsupported, "LiteLLM endpoint not supported"),
        408 | 429 | 500..=599 => {
            LiteLLMFailure::new(Unavailable, "LiteLLM is temporarily unavailable")
        }
        _ => LiteLLMFailure::new(Unknown, "Unexpected LiteLLM response status"),
    }
}

async fn read_bounded(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, LiteLLMFailure> {
    let too_large =
        || LiteLLMFailure::new(LiteLLMFeatureState::MalformedResponse, "Response too large");
    if response
        .content_length()
        .is_some_and(|len| len > max_bytes as u64)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        LiteLLMFailure::new(
            LiteLLMFeatureState::Unavailable,
            "Failed to read LiteLLM response",
        )
    })? {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn is_auth_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "api-key"
            | "apikey"
            | "x-api-key"
            | "x-apikey"
            | "x-auth-token"
            | "x-access-token"
            | "ocp-apim-subscription-key"
    ) || lower.starts_with("x-litellm")
        || lower.contains("api-key")
        || lower.contains("api_key")
}

fn is_sensitive_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    is_auth_header(name)
        || [
            "key",
            "auth",
            "token",
            "secret",
            "cookie",
            "password",
            "credential",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
}

pub(crate) fn redact_secret(value: &mut Value, secret: &str) {
    if secret.is_empty() {
        return;
    }
    match value {
        Value::String(text) if text.contains(secret) => *text = text.replace(secret, "[REDACTED]"),
        Value::Array(values) => values.iter_mut().for_each(|v| redact_secret(v, secret)),
        Value::Object(values) => {
            if values.keys().any(|key| key.contains(secret)) {
                let taken = std::mem::take(values);
                for (key, value) in taken {
                    let key = if key.contains(secret) {
                        key.replace(secret, "[REDACTED]")
                    } else {
                        key
                    };
                    if !values.contains_key(&key) {
                        values.insert(key, value);
                    }
                }
            }
            values.values_mut().for_each(|v| redact_secret(v, secret));
        }
        _ => {}
    }
}

pub(crate) fn safe_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    let lower = text.to_ascii_lowercase();
    if text.is_empty()
        || lower.starts_with("sk-")
        || lower.contains("bearer ")
        || lower.contains("[redacted]")
        || (text.starts_with("eyJ") && text.matches('.').count() == 2)
    {
        return None;
    }
    Some(
        text.chars()
            .filter(|c| !c.is_control())
            .take(MAX_TEXT_CHARS)
            .collect(),
    )
}

pub(crate) fn number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })
        .filter(|n: &f64| n.is_finite())
}

pub(crate) fn non_negative(value: Option<&Value>) -> Option<f64> {
    number(value).filter(|n| *n >= 0.0)
}

pub(crate) fn nullable_limit(value: Option<&Value>) -> Option<Option<f64>> {
    match value {
        Some(Value::Null) => Some(None),
        value => non_negative(value).map(Some),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiteLLMSubjectScope {
    Key,
    User,
    Team,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiteLLMSpendState {
    Known,
    Absent,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMSpend {
    pub value: Option<f64>,
    pub state: LiteLLMSpendState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiteLLMBudgetKind {
    Unlimited,
    Limited,
    Unknown,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMMaxBudget {
    pub kind: LiteLLMBudgetKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMRemaining {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    pub derived: bool,
    pub overage: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMModelLimit {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_budget: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spend: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpm_limit: Option<Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpm_limit: Option<Option<f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMAccountSubject {
    pub scope: LiteLLMSubjectScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub label: String,
    pub state: LiteLLMFeatureState,
    pub spend: LiteLLMSpend,
    pub max_budget: LiteLLMMaxBudget,
    pub remaining: LiteLLMRemaining,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_reset_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpm_limit: Option<Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpm_limit: Option<Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallel_requests: Option<Option<f64>>,
    pub model_limits: Vec<LiteLLMModelLimit>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMAccountSnapshot {
    pub state: LiteLLMFeatureState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub fetched_at: String,
    pub stale: bool,
    pub subjects: Vec<LiteLLMAccountSubject>,
}

impl LiteLLMAccountSnapshot {
    pub fn failed(failure: LiteLLMFailure) -> Self {
        Self {
            state: failure.state,
            message: Some(failure.message),
            fetched_at: now_rfc3339(),
            stale: false,
            subjects: Vec::new(),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct KeyIdentity {
    pub(crate) token_hash: Option<String>,
    pub(crate) user_id: Option<String>,
    pub(crate) team_id: Option<String>,
}

fn identifier(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty()
        && text.len() <= 256
        && !text.contains("[REDACTED]")
        && !text.to_ascii_lowercase().starts_with("sk-")
        && text.chars().all(|c| !c.is_control() && !c.is_whitespace()))
    .then(|| text.to_string())
}

fn spend_of(object: &Value) -> LiteLLMSpend {
    match object.get("spend") {
        None => LiteLLMSpend {
            value: None,
            state: LiteLLMSpendState::Unknown,
        },
        Some(Value::Null) => LiteLLMSpend {
            value: None,
            state: LiteLLMSpendState::Absent,
        },
        value => match non_negative(value) {
            Some(spend) => LiteLLMSpend {
                value: Some(spend),
                state: LiteLLMSpendState::Known,
            },
            None => LiteLLMSpend {
                value: None,
                state: LiteLLMSpendState::Unknown,
            },
        },
    }
}

fn max_budget_of(object: &Value) -> LiteLLMMaxBudget {
    match object.get("max_budget") {
        None => LiteLLMMaxBudget {
            kind: LiteLLMBudgetKind::Unknown,
            value: None,
        },
        Some(Value::Null) => LiteLLMMaxBudget {
            kind: LiteLLMBudgetKind::Unlimited,
            value: None,
        },
        value => match non_negative(value) {
            Some(max) => LiteLLMMaxBudget {
                kind: LiteLLMBudgetKind::Limited,
                value: Some(max),
            },
            None => LiteLLMMaxBudget {
                kind: LiteLLMBudgetKind::Unknown,
                value: None,
            },
        },
    }
}

fn remaining_of(object: &Value, spend: &LiteLLMSpend, max: &LiteLLMMaxBudget) -> LiteLLMRemaining {
    if let Some(reported) = number(object.get("remaining")) {
        return LiteLLMRemaining {
            value: Some(reported),
            derived: false,
            overage: reported < 0.0,
        };
    }
    match (max.kind, max.value, spend.value) {
        (LiteLLMBudgetKind::Limited, Some(max), Some(spend)) => LiteLLMRemaining {
            value: Some(max - spend),
            derived: true,
            overage: spend > max,
        },
        _ => LiteLLMRemaining {
            value: None,
            derived: false,
            overage: false,
        },
    }
}

fn limit_entry<'a>(
    limits: &'a mut BTreeMap<String, LiteLLMModelLimit>,
    model: &str,
) -> Option<&'a mut LiteLLMModelLimit> {
    let name = safe_text(Some(&Value::String(model.to_string())))?;
    if !limits.contains_key(&name) && limits.len() >= MAX_MODEL_LIMITS {
        return None;
    }
    Some(limits.entry(name.clone()).or_insert(LiteLLMModelLimit {
        model: name,
        max_budget: None,
        budget_duration: None,
        spend: None,
        rpm_limit: None,
        tpm_limit: None,
    }))
}

fn model_limits_of(object: &Value) -> Vec<LiteLLMModelLimit> {
    let metadata = object.get("metadata").filter(|v| v.is_object());
    let map_of = |name: &str| {
        object
            .get(name)
            .or_else(|| metadata.and_then(|m| m.get(name)))
            .and_then(Value::as_object)
    };
    let mut limits: BTreeMap<String, LiteLLMModelLimit> = BTreeMap::new();
    for (model, value) in map_of("model_max_budget").into_iter().flatten() {
        if let Some(limit) = limit_entry(&mut limits, model) {
            limit.max_budget = non_negative(Some(value))
                .or_else(|| non_negative(value.get("budget_limit")))
                .or_else(|| non_negative(value.get("max_budget")));
            limit.budget_duration = safe_text(
                value
                    .get("time_period")
                    .or_else(|| value.get("budget_duration")),
            );
        }
    }
    for (model, value) in map_of("model_spend").into_iter().flatten() {
        if let Some(limit) = limit_entry(&mut limits, model) {
            limit.spend = non_negative(Some(value));
        }
    }
    for (model, value) in map_of("model_rpm_limit").into_iter().flatten() {
        if let Some(limit) = limit_entry(&mut limits, model) {
            limit.rpm_limit = nullable_limit(Some(value));
        }
    }
    for (model, value) in map_of("model_tpm_limit").into_iter().flatten() {
        if let Some(limit) = limit_entry(&mut limits, model) {
            limit.tpm_limit = nullable_limit(Some(value));
        }
    }
    limits.into_values().collect()
}

pub(crate) fn subject_from(
    scope: LiteLLMSubjectScope,
    id: Option<String>,
    label: String,
    object: &Value,
) -> LiteLLMAccountSubject {
    let spend = spend_of(object);
    let max_budget = max_budget_of(object);
    let remaining = remaining_of(object, &spend, &max_budget);
    LiteLLMAccountSubject {
        scope,
        id,
        label,
        state: LiteLLMFeatureState::Ok,
        spend,
        max_budget,
        remaining,
        currency: "USD".to_string(),
        budget_reset_at: safe_text(object.get("budget_reset_at")),
        budget_duration: safe_text(object.get("budget_duration")),
        expires_at: safe_text(object.get("expires").or_else(|| object.get("expires_at"))),
        blocked: object.get("blocked").and_then(Value::as_bool),
        rpm_limit: nullable_limit(object.get("rpm_limit")),
        tpm_limit: nullable_limit(object.get("tpm_limit")),
        max_parallel_requests: nullable_limit(object.get("max_parallel_requests")),
        model_limits: model_limits_of(object),
    }
}

fn failed_subject(
    scope: LiteLLMSubjectScope,
    id: Option<String>,
    label: String,
    failure: &LiteLLMFailure,
) -> LiteLLMAccountSubject {
    let kind = if failure.state == LiteLLMFeatureState::PermissionDenied {
        LiteLLMBudgetKind::Denied
    } else {
        LiteLLMBudgetKind::Unknown
    };
    LiteLLMAccountSubject {
        scope,
        id,
        label,
        state: failure.state,
        spend: LiteLLMSpend {
            value: None,
            state: LiteLLMSpendState::Unknown,
        },
        max_budget: LiteLLMMaxBudget {
            kind,
            value: None,
        },
        remaining: LiteLLMRemaining {
            value: None,
            derived: false,
            overage: false,
        },
        currency: "USD".to_string(),
        budget_reset_at: None,
        budget_duration: None,
        expires_at: None,
        blocked: None,
        rpm_limit: None,
        tpm_limit: None,
        max_parallel_requests: None,
        model_limits: Vec::new(),
    }
}

pub(crate) async fn fetch_key_info(
    session: &mut NativeSession,
) -> Result<(KeyIdentity, Value), LiteLLMFailure> {
    let value = session
        .get_json("/key/info", &[], MAX_ACCOUNT_BODY_BYTES)
        .await?;
    let info = value
        .get("info")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| {
            LiteLLMFailure::new(LiteLLMFeatureState::MalformedResponse, "Missing key info")
        })?;
    session.pin_identity();
    let identity = KeyIdentity {
        token_hash: identifier(info.get("token")).or_else(|| identifier(value.get("key"))),
        user_id: identifier(info.get("user_id")),
        team_id: identifier(info.get("team_id")),
    };
    Ok((identity, info))
}

pub(crate) async fn fetch_account(session: &mut NativeSession) -> LiteLLMAccountSnapshot {
    let (identity, info) = match fetch_key_info(session).await {
        Ok(result) => result,
        Err(_) if permits_self_user(session) => return fetch_self_account(session).await,
        Err(failure) => return LiteLLMAccountSnapshot::failed(failure),
    };
    let key_label = safe_text(info.get("key_alias")).unwrap_or_else(|| "Current API key".into());
    let mut subjects = vec![subject_from(
        LiteLLMSubjectScope::Key,
        None,
        key_label,
        &info,
    )];
    if let Some(user_id) = identity.user_id.clone() {
        let query = [("user_id", user_id.clone())];
        subjects.push(
            match session
                .get_json("/user/info", &query, MAX_ACCOUNT_BODY_BYTES)
                .await
            {
                Ok(value) => match value.get("user_info").filter(|v| v.is_object()) {
                    Some(user) => {
                        let label =
                            safe_text(user.get("user_alias")).unwrap_or_else(|| "Your user".into());
                        subject_from(LiteLLMSubjectScope::User, Some(user_id), label, user)
                    }
                    None => failed_subject(
                        LiteLLMSubjectScope::User,
                        Some(user_id),
                        "Your user".into(),
                        &LiteLLMFailure::new(
                            LiteLLMFeatureState::MalformedResponse,
                            "Missing user info",
                        ),
                    ),
                },
                Err(failure) => failed_subject(
                    LiteLLMSubjectScope::User,
                    Some(user_id),
                    "Your user".into(),
                    &failure,
                ),
            },
        );
    }
    if let Some(team_id) = identity.team_id.clone() {
        let query = [("team_id", team_id.clone())];
        subjects.push(
            match session
                .get_json("/team/info", &query, MAX_ACCOUNT_BODY_BYTES)
                .await
            {
                Ok(value) => match value.get("team_info").filter(|v| v.is_object()) {
                    Some(team) => {
                        let label =
                            safe_text(team.get("team_alias")).unwrap_or_else(|| "Your team".into());
                        subject_from(LiteLLMSubjectScope::Team, Some(team_id), label, team)
                    }
                    None => failed_subject(
                        LiteLLMSubjectScope::Team,
                        Some(team_id),
                        "Your team".into(),
                        &LiteLLMFailure::new(
                            LiteLLMFeatureState::MalformedResponse,
                            "Missing team info",
                        ),
                    ),
                },
                Err(failure) => failed_subject(
                    LiteLLMSubjectScope::Team,
                    Some(team_id),
                    "Your team".into(),
                    &failure,
                ),
            },
        );
    }
    LiteLLMAccountSnapshot {
        state: LiteLLMFeatureState::Ok,
        message: None,
        fetched_at: now_rfc3339(),
        stale: false,
        subjects,
    }
}

pub(crate) async fn fetch_self_user(
    session: &mut NativeSession,
) -> Result<(KeyIdentity, Value), LiteLLMFailure> {
    session.pin_identity();
    let value = session
        .get_json("/user/info", &[], MAX_ACCOUNT_BODY_BYTES)
        .await?;
    let invalid = || {
        LiteLLMFailure::new(
            LiteLLMFeatureState::MalformedResponse,
            "Self-user response has missing or inconsistent user identity",
        )
    };
    let user = value
        .get("user_info")
        .filter(|v| v.is_object())
        .ok_or_else(invalid)?;
    let user_id = identifier(value.get("user_id")).ok_or_else(invalid)?;
    if value.get("user_id").and_then(Value::as_str) != Some(user_id.as_str())
        || user.get("user_id").and_then(Value::as_str) != Some(user_id.as_str())
        || (user_id.starts_with("eyJ") && user_id.matches('.').count() == 2)
    {
        return Err(invalid());
    }
    Ok((
        KeyIdentity {
            token_hash: None,
            user_id: Some(user_id),
            team_id: identifier(user.get("team_id")).filter(|id| {
                user.get("team_id").and_then(Value::as_str) == Some(id.as_str())
                    && safe_text(user.get("team_id")).as_deref() == Some(id.as_str())
            }),
        },
        user.clone(),
    ))
}

async fn fetch_self_account(session: &mut NativeSession) -> LiteLLMAccountSnapshot {
    let key = failed_subject(
        LiteLLMSubjectScope::Key,
        None,
        "Key quota unavailable: bearer has no virtual-key record".into(),
        &missing_key_failure(),
    );
    let (identity, user) = match fetch_self_user(session).await {
        Ok(result) => result,
        Err(failure) => return LiteLLMAccountSnapshot::failed(failure),
    };
    LiteLLMAccountSnapshot {
        state: LiteLLMFeatureState::Ok,
        message: None,
        fetched_at: now_rfc3339(),
        stale: false,
        subjects: vec![
            key,
            subject_from(
                LiteLLMSubjectScope::User,
                identity.user_id,
                "Your user".into(),
                &user,
            ),
            failed_subject(
                LiteLLMSubjectScope::Team,
                identity.team_id,
                "Team quota unknown for this bearer".into(),
                &LiteLLMFailure::new(LiteLLMFeatureState::Unknown, "Team quota unknown"),
            ),
        ],
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMMcpServer {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth2_flow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub mcp_endpoint_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMMcpInventory {
    pub state: LiteLLMFeatureState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub fetched_at: String,
    pub servers: Vec<LiteLLMMcpServer>,
}

impl LiteLLMMcpInventory {
    pub fn failed(failure: LiteLLMFailure) -> Self {
        Self {
            state: failure.state,
            message: Some(failure.message),
            fetched_at: now_rfc3339(),
            servers: Vec::new(),
        }
    }
}

pub fn validate_mcp_server_name(name: &str) -> Result<(), LiteLLMFailure> {
    let valid = !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    valid.then_some(()).ok_or_else(|| {
        LiteLLMFailure::new(
            LiteLLMFeatureState::MalformedResponse,
            "Invalid MCP server name",
        )
    })
}

pub(crate) fn parse_mcp_inventory(value: &Value) -> Result<Vec<LiteLLMMcpServer>, LiteLLMFailure> {
    let rows = value
        .as_array()
        .or_else(|| value.get("data").and_then(Value::as_array))
        .or_else(|| value.get("servers").and_then(Value::as_array))
        .ok_or_else(|| {
            LiteLLMFailure::new(
                LiteLLMFeatureState::MalformedResponse,
                "Unexpected MCP server list",
            )
        })?;
    let mut servers = Vec::new();
    for row in rows.iter().take(MAX_MCP_SERVERS) {
        let Some(name) = identifier(row.get("server_name"))
            .or_else(|| identifier(row.get("alias")))
            .or_else(|| identifier(row.get("name")))
            .filter(|name| validate_mcp_server_name(name).is_ok())
        else {
            continue;
        };
        let id = identifier(row.get("server_id")).unwrap_or_else(|| name.clone());
        let mcp_info = row.get("mcp_info").filter(|v| v.is_object());
        servers.push(LiteLLMMcpServer {
            id,
            mcp_endpoint_path: format!("/{name}/mcp"),
            alias: safe_text(row.get("alias")),
            display_name: safe_text(row.get("display_name"))
                .or_else(|| mcp_info.and_then(|i| safe_text(i.get("server_name")))),
            description: safe_text(row.get("description"))
                .or_else(|| mcp_info.and_then(|i| safe_text(i.get("description")))),
            auth_type: safe_text(row.get("auth_type")),
            oauth2_flow: safe_text(row.get("oauth2_flow")),
            transport: safe_text(row.get("transport")),
            status: safe_text(row.get("status")),
            name,
        });
    }
    Ok(servers)
}

pub(crate) async fn fetch_mcp_inventory(session: &mut NativeSession) -> LiteLLMMcpInventory {
    let value = match session
        .get_json("/v1/mcp/server", &[], MAX_ACCOUNT_BODY_BYTES * 4)
        .await
    {
        Ok(value) => value,
        Err(failure) => return LiteLLMMcpInventory::failed(failure),
    };
    match parse_mcp_inventory(&value) {
        Ok(servers) => LiteLLMMcpInventory {
            state: LiteLLMFeatureState::Ok,
            message: None,
            fetched_at: now_rfc3339(),
            servers,
        },
        Err(failure) => LiteLLMMcpInventory::failed(failure),
    }
}

pub const MAX_STATUS_SERVERS: usize = 16;
const STATUS_GLOBAL_BUDGET: Duration = Duration::from_secs(3);
const STATUS_REQUEST_BUDGET: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiteLLMMcpHealth {
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiteLLMMcpOauthState {
    Connected,
    AuthRequired,
    Unsupported,
    PermissionDenied,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMMcpServerStatus {
    pub server_id: String,
    pub health: LiteLLMMcpHealth,
    pub oauth_state: LiteLLMMcpOauthState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_expires_at: Option<String>,
    pub missing_setup: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteLLMMcpStatusSnapshot {
    pub state: LiteLLMFeatureState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub fetched_at: String,
    pub servers: Vec<LiteLLMMcpServerStatus>,
}

impl LiteLLMMcpStatusSnapshot {
    pub fn failed(failure: LiteLLMFailure) -> Self {
        Self {
            state: failure.state,
            message: Some(failure.message),
            fetched_at: now_rfc3339(),
            servers: Vec::new(),
        }
    }
}

fn status_server_ids(ids: &[String]) -> Result<Vec<String>, LiteLLMFailure> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        validate_mcp_server_name(id)?;
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    if out.len() > MAX_STATUS_SERVERS {
        return Err(LiteLLMFailure::new(
            LiteLLMFeatureState::MalformedResponse,
            "Too many MCP servers requested",
        ));
    }
    Ok(out)
}

fn health_map(value: &Value) -> Option<HashMap<String, LiteLLMMcpHealth>> {
    let rows = value
        .as_array()
        .or_else(|| value.get("data").and_then(Value::as_array))
        .or_else(|| value.get("servers").and_then(Value::as_array))?;
    let mut map = HashMap::new();
    for row in rows {
        let Some(id) = identifier(row.get("server_id")) else {
            continue;
        };
        let health = match row
            .get("status")
            .or_else(|| row.get("health"))
            .and_then(Value::as_str)
            .map(|s| s.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("healthy") => LiteLLMMcpHealth::Healthy,
            Some("unhealthy") => LiteLLMMcpHealth::Unhealthy,
            _ => LiteLLMMcpHealth::Unknown,
        };
        map.insert(id, health);
    }
    Some(map)
}

fn oauth_state_from(
    result: Result<Value, LiteLLMFailure>,
) -> (LiteLLMMcpOauthState, Option<String>) {
    use LiteLLMMcpOauthState::*;
    match result {
        Ok(value) => match (
            value.get("has_credential").and_then(Value::as_bool),
            value.get("is_expired").and_then(Value::as_bool),
        ) {
            (Some(true), Some(false)) => (Connected, safe_text(value.get("expires_at"))),
            (Some(true), Some(true)) => (AuthRequired, safe_text(value.get("expires_at"))),
            (Some(false), _) => (AuthRequired, None),
            _ => (Unknown, None),
        },
        Err(failure) => (
            match failure.state {
                LiteLLMFeatureState::Unsupported => Unsupported,
                LiteLLMFeatureState::PermissionDenied => PermissionDenied,
                LiteLLMFeatureState::AuthRequired => AuthRequired,
                _ => Unknown,
            },
            None,
        ),
    }
}

fn same_origin_setup_url(base: &str, raw: Option<&Value>) -> Option<String> {
    let text = raw?.as_str()?.trim();
    let url = reqwest::Url::parse(text).ok()?;
    let base = reqwest::Url::parse(base).ok()?;
    (url.origin() == base.origin()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| url.to_string())
}

async fn timed_get(
    session: &mut NativeSession,
    path: &str,
    query: &[(&str, String)],
    deadline: tokio::time::Instant,
    request_budget: Duration,
) -> Result<Value, LiteLLMFailure> {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    let budget = remaining.min(request_budget);
    if budget.is_zero() {
        return Err(LiteLLMFailure::new(
            LiteLLMFeatureState::Unavailable,
            "MCP status time budget exhausted",
        ));
    }
    tokio::time::timeout(
        budget,
        session.get_json(path, query, MAX_ACCOUNT_BODY_BYTES),
    )
    .await
    .unwrap_or_else(|_| {
        Err(LiteLLMFailure::new(
            LiteLLMFeatureState::Unavailable,
            "MCP status request timed out",
        ))
    })
}

pub(crate) async fn fetch_mcp_status(
    session: &mut NativeSession,
    server_ids: &[String],
) -> LiteLLMMcpStatusSnapshot {
    fetch_mcp_status_with_budget(
        session,
        server_ids,
        STATUS_GLOBAL_BUDGET,
        STATUS_REQUEST_BUDGET,
    )
    .await
}

async fn fetch_mcp_status_with_budget(
    session: &mut NativeSession,
    server_ids: &[String],
    global_budget: Duration,
    request_budget: Duration,
) -> LiteLLMMcpStatusSnapshot {
    let deadline = tokio::time::Instant::now() + global_budget;
    session.pin_identity();
    let ids = match status_server_ids(server_ids) {
        Ok(ids) => ids,
        Err(failure) => return LiteLLMMcpStatusSnapshot::failed(failure),
    };
    let health = if ids.is_empty() {
        HashMap::new()
    } else {
        let query: Vec<(&str, String)> = ids.iter().map(|id| ("server_ids", id.clone())).collect();
        match timed_get(
            session,
            "/v1/mcp/server/health",
            &query,
            deadline,
            request_budget,
        )
        .await
        {
            Ok(value) => health_map(&value).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    };
    let mut servers = Vec::new();
    for id in ids {
        let oauth = timed_get(
            session,
            &format!("/v1/mcp/server/{id}/oauth-user-credential/status"),
            &[],
            deadline,
            request_budget,
        )
        .await;
        let (oauth_state, oauth_expires_at) = oauth_state_from(oauth);
        let (missing_setup, setup_url) = match timed_get(
            session,
            &format!("/v1/mcp/server/{id}/user-env-vars"),
            &[],
            deadline,
            request_budget,
        )
        .await
        {
            Ok(value) => match non_negative(value.get("missing_count")) {
                Some(n) => {
                    let url = (n > 0.0)
                        .then(|| same_origin_setup_url(&session.base, value.get("setup_url")))
                        .flatten();
                    (Some(n > 0.0), url)
                }
                None => (None, None),
            },
            Err(_) => (None, None),
        };
        servers.push(LiteLLMMcpServerStatus {
            health: health
                .get(&id)
                .copied()
                .unwrap_or(LiteLLMMcpHealth::Unknown),
            server_id: id,
            oauth_state,
            oauth_expires_at,
            missing_setup,
            setup_url,
        });
    }
    LiteLLMMcpStatusSnapshot {
        state: LiteLLMFeatureState::Ok,
        message: None,
        fetched_at: now_rfc3339(),
        servers,
    }
}

#[cfg(test)]
pub(crate) mod test_server {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub(crate) struct Reply {
        pub(crate) status: &'static str,
        pub(crate) headers: &'static str,
        pub(crate) body: String,
    }

    pub(crate) fn json(status: &'static str, body: impl Into<String>) -> Reply {
        Reply {
            status,
            headers: "Content-Type: application/json\r\n",
            body: body.into(),
        }
    }

    pub(crate) async fn serve(
        replies: Vec<Reply>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0_u8; 16 * 1024];
                let mut size = 0;
                loop {
                    let n = socket.read(&mut buffer[size..]).await.unwrap();
                    size += n;
                    if n == 0 || buffer[..size].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&buffer[..size]).to_string());
                let response = format!(
                    "HTTP/1.1 {}\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.status,
                    reply.headers,
                    reply.body.len(),
                    reply.body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
            requests
        });
        (format!("http://{address}"), handle)
    }

    pub(crate) fn request_line(request: &str) -> &str {
        request.lines().next().unwrap_or_default()
    }

    pub(crate) fn bearer(request: &str) -> Option<String> {
        request.lines().find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("authorization: bearer ")
                .map(|_| line["authorization: bearer ".len()..].trim().to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::test_server::{bearer, json, request_line, serve, Reply};
    use super::*;
    use crate::litellm::LiteLLMProvider;
    use serde_json::json as j;

    fn provider(endpoint: &str, key: &str) -> LiteLLMProvider {
        LiteLLMProvider {
            endpoint: endpoint.to_string(),
            api_key: key.to_string(),
            ..Default::default()
        }
    }

    pub(crate) fn missing_key_reply() -> Reply {
        json(
            "404 Not Found",
            j!({"error": {"type": "not_found_error",
            "message": "Key not found in database", "code": "404"}})
            .to_string(),
        )
    }

    #[tokio::test]
    async fn jwt_self_account_separates_user_budget_and_discards_key_list() {
        let (base, server) = serve(vec![
            missing_key_reply(),
            json(
                "200 OK",
                j!({"user_id": "self-user", "user_info": {
                "user_id": "self-user", "spend": 7, "max_budget": 20,
                "rpm_limit": null, "tpm_limit": null, "teams": [], "team_id": null},
                "keys": [{"token": "listed-secret", "spend": 99, "max_budget": 100,
                    "team_id": "unproven-team"}], "teams": []})
                .to_string(),
            ),
        ])
        .await;
        let snapshot = provider(&base, "synthetic-jwt-bearer")
            .fetch_account("jwt-self")
            .await;
        let requests = server.await.unwrap();
        assert_eq!(request_line(&requests[0]), "GET /key/info HTTP/1.1");
        assert_eq!(request_line(&requests[1]), "GET /user/info HTTP/1.1");
        assert!(requests
            .iter()
            .all(|r| bearer(r).as_deref() == Some("synthetic-jwt-bearer")));
        assert_eq!(snapshot.state, LiteLLMFeatureState::Ok);
        assert_eq!(snapshot.subjects.len(), 3);
        let key = &snapshot.subjects[0];
        assert_eq!(key.scope, LiteLLMSubjectScope::Key);
        assert_eq!(key.state, LiteLLMFeatureState::Unavailable);
        assert_eq!(key.max_budget.kind, LiteLLMBudgetKind::Unknown);
        assert_eq!(key.spend.value, None);
        assert!(key.label.contains("no virtual-key record"));
        let user = &snapshot.subjects[1];
        assert_eq!(user.scope, LiteLLMSubjectScope::User);
        assert_eq!(user.id.as_deref(), Some("self-user"));
        assert_eq!(user.spend.value, Some(7.0));
        assert_eq!(user.remaining.value, Some(13.0));
        assert_eq!(snapshot.subjects[2].state, LiteLLMFeatureState::Unknown);
        assert_eq!(snapshot.subjects[2].id, None);
        let output = serde_json::to_string(&snapshot).unwrap();
        for secret in ["synthetic-jwt-bearer", "listed-secret", "unproven-team"] {
            assert!(!output.contains(secret));
        }
    }

    #[tokio::test]
    async fn jwt_fallback_rejects_missing_mismatched_and_secret_self_ids() {
        for body in [
            j!({"user_info": {"user_id": "u"}}),
            j!({"user_id": "", "user_info": {"user_id": ""}}),
            j!({"user_id": "u", "user_info": {}}),
            j!({"user_id": "u", "user_info": {"user_id": "other"}}),
            j!({"user_id": " u", "user_info": {"user_id": " u"}}),
            j!({"user_id": "synthetic-jwt-bearer", "user_info": {"user_id": "synthetic-jwt-bearer"}}),
            j!({"user_id": "eyJfake.payload.signature", "user_info": {"user_id": "eyJfake.payload.signature"}}),
        ] {
            let (base, server) =
                serve(vec![missing_key_reply(), json("200 OK", body.to_string())]).await;
            let result = provider(&base, "synthetic-jwt-bearer")
                .fetch_account("jwt-invalid")
                .await;
            assert_eq!(server.await.unwrap().len(), 2);
            assert_eq!(result.state, LiteLLMFeatureState::MalformedResponse);
            assert!(result.subjects.is_empty());
            assert!(!serde_json::to_string(&result)
                .unwrap()
                .contains("synthetic-jwt-bearer"));
        }
    }

    #[tokio::test]
    async fn only_exact_bounded_missing_key_error_permits_fallback() {
        for (status, body, expected) in [
            ("401 Unauthorized", j!({"error": {"type": "not_found_error", "message": "Key not found in database", "code": "404"}}).to_string(), LiteLLMFeatureState::AuthRequired),
            ("403 Forbidden", "synthetic-jwt-bearer".into(), LiteLLMFeatureState::PermissionDenied),
            ("404 Not Found", j!({"error": {"type": "not_found_error", "message": "synthetic-jwt-bearer", "code": "404"}}).to_string(), LiteLLMFeatureState::Unsupported),
            ("404 Not Found", "synthetic-jwt-bearer".repeat(MAX_ERROR_BODY_BYTES), LiteLLMFeatureState::Unsupported),
            ("404 Not Found", "not JSON".into(), LiteLLMFeatureState::Unsupported),
        ] {
            let (base, server) = serve(vec![json(status, body)]).await;
            let snapshot = provider(&base, "synthetic-jwt-bearer").fetch_account("jwt-errors").await;
            assert_eq!(server.await.unwrap().len(), 1);
            assert_eq!(snapshot.state, expected);
            assert!(!serde_json::to_string(&snapshot).unwrap().contains("synthetic-jwt-bearer"));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn missing_key_pins_command_bearer_before_self_user_rejection() {
        let (base, server) = serve(vec![missing_key_reply(), json("401 Unauthorized", "{}")]).await;
        let temp = tempfile::tempdir().unwrap();
        let counter = temp.path().join("count");
        let id = format!("jwt_pin_{}", uuid::Uuid::new_v4().simple());
        let p = LiteLLMProvider {
            endpoint: base,
            credential: Some(counting_credential(&counter)),
            ..Default::default()
        };
        let result = p.fetch_account(&id).await;
        assert_eq!(result.state, LiteLLMFeatureState::AuthRequired);
        assert_eq!(std::fs::read_to_string(counter).unwrap(), "1");
        assert!(server
            .await
            .unwrap()
            .iter()
            .all(|r| bearer(r).as_deref() == Some("tok1")));
        crate::credential::invalidate(&id);
    }

    #[test]
    fn base_url_policy_requires_https_or_loopback() {
        for ok in [
            "https://proxy.example/v1",
            "http://localhost:4000",
            "http://127.0.0.1:4000/v1/chat/completions",
            "http://[::1]:4000",
        ] {
            assert!(validate_base_url(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            validate_base_url("https://proxy.example/v1").unwrap(),
            "https://proxy.example"
        );
        for bad in [
            "http://proxy.example",
            "ftp://proxy.example",
            "https://user:pw@proxy.example",
            "https://proxy.example/?key=secret",
            "https://proxy.example/#frag",
            "not a url",
        ] {
            let error = validate_base_url(bad).unwrap_err();
            assert_eq!(error.state, LiteLLMFeatureState::NotConfigured, "{bad}");
            assert!(!error.message.contains("secret") && !error.message.contains("pw"));
        }
    }

    #[test]
    fn bearer_debug_and_failure_json_never_leak_secret() {
        let bearer = LiteLLMBearer::new("sk-very-secret".into());
        assert!(!format!("{bearer:?}").contains("sk-very-secret"));
        let failure = status_failure(403);
        assert_eq!(failure.state, LiteLLMFeatureState::PermissionDenied);
        assert_eq!(status_failure(401).state, LiteLLMFeatureState::AuthRequired);
        assert_eq!(status_failure(302).state, LiteLLMFeatureState::Unavailable);
        assert_eq!(status_failure(404).state, LiteLLMFeatureState::Unsupported);
    }

    #[test]
    fn quota_semantics_distinguish_missing_null_zero_and_overage() {
        let unlimited = subject_from(
            LiteLLMSubjectScope::Key,
            None,
            "k".into(),
            &j!({"spend": 4.5, "max_budget": null}),
        );
        assert_eq!(unlimited.spend.value, Some(4.5));
        assert_eq!(unlimited.max_budget.kind, LiteLLMBudgetKind::Unlimited);
        assert_eq!(unlimited.remaining.value, None);

        let missing = subject_from(LiteLLMSubjectScope::Key, None, "k".into(), &j!({}));
        assert_eq!(missing.spend.state, LiteLLMSpendState::Unknown);
        assert_eq!(missing.max_budget.kind, LiteLLMBudgetKind::Unknown);

        let null_spend = subject_from(
            LiteLLMSubjectScope::Key,
            None,
            "k".into(),
            &j!({"spend": null, "max_budget": 0}),
        );
        assert_eq!(null_spend.spend.state, LiteLLMSpendState::Absent);
        assert_eq!(null_spend.max_budget.kind, LiteLLMBudgetKind::Limited);
        assert_eq!(null_spend.max_budget.value, Some(0.0));

        let over = subject_from(
            LiteLLMSubjectScope::Key,
            None,
            "k".into(),
            &j!({"spend": "12.5", "max_budget": 10, "budget_reset_at": "2026-01-01T00:00:00Z",
                 "budget_duration": "30d", "expires": null, "blocked": true,
                 "rpm_limit": 60, "tpm_limit": null,
                 "model_max_budget": {"gpt": {"budget_limit": 5, "time_period": "1d"}},
                 "model_spend": {"gpt": 1.25},
                 "metadata": {"model_rpm_limit": {"gpt": 10}}}),
        );
        assert_eq!(
            over.remaining,
            LiteLLMRemaining {
                value: Some(-2.5),
                derived: true,
                overage: true
            }
        );
        assert_eq!(over.blocked, Some(true));
        assert_eq!(over.rpm_limit, Some(Some(60.0)));
        assert_eq!(over.tpm_limit, Some(None));
        assert_eq!(over.budget_duration.as_deref(), Some("30d"));
        assert_eq!(
            over.model_limits,
            vec![LiteLLMModelLimit {
                model: "gpt".into(),
                max_budget: Some(5.0),
                budget_duration: Some("1d".into()),
                spend: Some(1.25),
                rpm_limit: Some(Some(10.0)),
                tpm_limit: None,
            }]
        );

        let reported = subject_from(
            LiteLLMSubjectScope::Key,
            None,
            "k".into(),
            &j!({"spend": 1, "max_budget": 10, "remaining": 7}),
        );
        assert_eq!(
            reported.remaining,
            LiteLLMRemaining {
                value: Some(7.0),
                derived: false,
                overage: false
            }
        );
    }

    #[tokio::test]
    async fn account_reads_self_key_then_own_user_and_team_with_inference_key() {
        let (base, server) = serve(vec![
            json(
                "200 OK",
                j!({"key": "hashed-token", "info": {
                "token": "hashed-token", "key_alias": "laptop", "spend": 2, "max_budget": 10,
                "user_id": "user-1", "team_id": "team-1"}})
                .to_string(),
            ),
            json(
                "200 OK",
                j!({"user_id": "user-1", "user_info": {"spend": 5, "max_budget": null}})
                    .to_string(),
            ),
            json(
                "403 Forbidden",
                r#"{"error":"sk-inference-key not allowed"}"#,
            ),
        ])
        .await;
        let snapshot = provider(&format!("{base}/v1"), "sk-inference-key")
            .fetch_account("litellm")
            .await;
        let requests = server.await.unwrap();
        assert_eq!(request_line(&requests[0]), "GET /key/info HTTP/1.1");
        assert_eq!(
            request_line(&requests[1]),
            "GET /user/info?user_id=user-1 HTTP/1.1"
        );
        assert_eq!(
            request_line(&requests[2]),
            "GET /team/info?team_id=team-1 HTTP/1.1"
        );
        assert!(requests
            .iter()
            .all(|r| bearer(r).as_deref() == Some("sk-inference-key")));
        assert_eq!(snapshot.state, LiteLLMFeatureState::Ok);
        assert_eq!(snapshot.subjects.len(), 3);
        assert_eq!(snapshot.subjects[0].label, "laptop");
        assert_eq!(snapshot.subjects[0].remaining.value, Some(8.0));
        assert_eq!(
            snapshot.subjects[1].max_budget.kind,
            LiteLLMBudgetKind::Unlimited
        );
        assert_eq!(
            snapshot.subjects[2].state,
            LiteLLMFeatureState::PermissionDenied
        );
        assert_eq!(
            snapshot.subjects[2].max_budget.kind,
            LiteLLMBudgetKind::Denied
        );
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("sk-inference-key"));
        assert!(!serialized.contains("hashed-token"));
    }

    #[tokio::test]
    async fn key_without_user_or_team_never_queries_broader_scopes() {
        let (base, server) = serve(vec![json(
            "200 OK",
            j!({"info": {"spend": 1, "max_budget": null}}).to_string(),
        )])
        .await;
        let snapshot = provider(&base, "sk-a").fetch_account("litellm").await;
        assert_eq!(server.await.unwrap().len(), 1);
        assert_eq!(snapshot.subjects.len(), 1);
        assert_eq!(snapshot.subjects[0].scope, LiteLLMSubjectScope::Key);
    }

    #[tokio::test]
    async fn redirects_are_refused_and_not_followed() {
        let (base, server) = serve(vec![Reply {
            status: "302 Found",
            headers: "Location: https://evil.example/key/info\r\n",
            body: String::new(),
        }])
        .await;
        let snapshot = provider(&base, "sk-a").fetch_account("litellm").await;
        assert_eq!(server.await.unwrap().len(), 1);
        assert_eq!(snapshot.state, LiteLLMFeatureState::Unavailable);
        assert!(snapshot.subjects.is_empty());
    }

    #[tokio::test]
    async fn oversized_and_malformed_bodies_are_typed() {
        let (base, server) = serve(vec![
            json("200 OK", "x".repeat(MAX_ACCOUNT_BODY_BYTES + 1)),
            json("200 OK", "not json"),
            json("200 OK", "{}"),
        ])
        .await;
        let p = provider(&base, "sk-a");
        assert_eq!(
            p.fetch_account("litellm").await.state,
            LiteLLMFeatureState::MalformedResponse
        );
        assert_eq!(
            p.fetch_account("litellm").await.state,
            LiteLLMFeatureState::MalformedResponse
        );
        assert_eq!(
            p.fetch_account("litellm").await.state,
            LiteLLMFeatureState::MalformedResponse
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn static_key_401_is_auth_required_without_retry_and_403_not_elevated() {
        let (base, server) = serve(vec![
            json("401 Unauthorized", "{}"),
            json("403 Forbidden", "{}"),
        ])
        .await;
        let p = provider(&base, "sk-a");
        assert_eq!(
            p.fetch_account("litellm").await.state,
            LiteLLMFeatureState::AuthRequired
        );
        assert_eq!(
            p.fetch_account("litellm").await.state,
            LiteLLMFeatureState::PermissionDenied
        );
        assert_eq!(server.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn missing_key_and_invalid_endpoint_are_not_configured_without_requests() {
        let p = provider("http://127.0.0.1:1", "");
        assert_eq!(
            p.fetch_account("litellm").await.state,
            LiteLLMFeatureState::NotConfigured
        );
        let p = provider("http://proxy.example", "sk-a");
        assert_eq!(
            p.fetch_account("litellm").await.state,
            LiteLLMFeatureState::NotConfigured
        );
        assert!(p.mcp_endpoint("github").is_err());
    }

    #[test]
    fn signed_remaining_and_nullable_limits_keep_exact_semantics() {
        let reported = subject_from(
            LiteLLMSubjectScope::Key,
            None,
            "k".into(),
            &j!({"spend": 15, "max_budget": 10, "remaining": -5,
                 "rpm_limit": 0, "max_parallel_requests": null}),
        );
        assert_eq!(reported.remaining.value, Some(-5.0));
        assert!(reported.remaining.overage && !reported.remaining.derived);
        assert_eq!(reported.rpm_limit, Some(Some(0.0)));
        assert_eq!(reported.tpm_limit, None);
        assert_eq!(reported.max_parallel_requests, Some(None));
        let json = serde_json::to_value(&reported).unwrap();
        assert_eq!(json["rpm_limit"], j!(0.0));
        assert!(json.get("tpm_limit").is_none());
        assert!(json["max_parallel_requests"].is_null());
        assert!(json
            .as_object()
            .unwrap()
            .contains_key("max_parallel_requests"));
    }

    #[tokio::test]
    async fn bearer_in_object_keys_is_redacted_and_benign_headers_preserved() {
        let (base, server) = serve(vec![json(
            "200 OK",
            j!({"info": {"key_alias": "prodteam",
                "model_max_budget": {"arbitrarytoken123": {"budget_limit": 5}},
                "model_spend": {"hiddenkeyvalue-model": 1}}})
            .to_string(),
        )])
        .await;
        let p = LiteLLMProvider {
            endpoint: base,
            api_key: "arbitrarytoken123".into(),
            extra_headers: HashMap::from([
                ("X-Env".into(), "prod".into()),
                ("X-API-Key".into(), "hiddenkeyvalue".into()),
                ("api-key".into(), "other-auth".into()),
                ("Authorization".into(), "Bearer other".into()),
                ("X-Litellm-Api-Key".into(), "lit".into()),
                ("Cookie".into(), "c=1".into()),
            ]),
            ..Default::default()
        };
        let snapshot = p.fetch_account("litellm").await;
        let request = server.await.unwrap().remove(0).to_ascii_lowercase();
        assert!(request.contains("x-env: prod"));
        for forbidden in ["x-api-key", "api-key:", "x-litellm", "cookie", "other"] {
            assert!(!request.contains(forbidden), "{forbidden}");
        }
        assert_eq!(bearer(&request).as_deref(), Some("arbitrarytoken123"));
        assert_eq!(snapshot.subjects[0].label, "prodteam");
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("arbitrarytoken123"));
        assert!(!serialized.contains("hiddenkeyvalue"));
        let mut value = j!({"a": {"tok-secret-1": 1, "b[tok-secret-1]": {"tok-secret-1": 2}}});
        redact_secret(&mut value, "tok-secret-1");
        assert!(!value.to_string().contains("tok-secret-1"));
    }

    #[test]
    fn provider_debug_never_prints_secrets() {
        let p = LiteLLMProvider {
            endpoint: "https://u:pw@proxy.example".into(),
            api_key: "sk-debug-secret".into(),
            extra_headers: HashMap::from([("X-Tenant".into(), "hdr-secret".into())]),
            ..Default::default()
        };
        let text = format!("{p:?}");
        for secret in ["sk-debug-secret", "hdr-secret", "pw"] {
            assert!(!text.contains(secret), "{secret}");
        }
        assert!(text.contains("X-Tenant"));
    }

    #[cfg(unix)]
    fn counting_credential(counter: &std::path::Path) -> CredentialSpec {
        CredentialSpec::Command {
            command: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!(
                    "n=$(cat '{0}' 2>/dev/null || echo 0); n=$((n+1)); printf %s $n > '{0}'; printf tok$n",
                    counter.display()
                ),
            ],
            timeout_ms: 5_000,
            refresh_interval_ms: 300_000,
            cwd: None,
            env_passthrough: Vec::new(),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_credential_refreshes_once_on_401_keyed_by_instance() {
        let (base, server) = serve(vec![
            json("401 Unauthorized", "{}"),
            json(
                "200 OK",
                j!({"info": {"spend": 0, "max_budget": 1,
                "key_alias": "was tok1 hdr-secret-value"}})
                .to_string(),
            ),
            json("401 Unauthorized", "{}"),
            json("401 Unauthorized", "{}"),
        ])
        .await;
        let temp = tempfile::tempdir().unwrap();
        let counter = temp.path().join("count");
        let id = format!("litellm_acct_{}", uuid::Uuid::new_v4().simple());
        let p = LiteLLMProvider {
            endpoint: base,
            credential: Some(counting_credential(&counter)),
            extra_headers: HashMap::from([("X-Tenant-Token".into(), "hdr-secret-value".into())]),
            ..Default::default()
        };
        let snapshot = p.fetch_account(&id).await;
        assert_eq!(snapshot.state, LiteLLMFeatureState::Ok);
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("tok1") && !serialized.contains("hdr-secret-value"));
        assert_eq!(
            p.fetch_account(&id).await.state,
            LiteLLMFeatureState::AuthRequired
        );
        let requests = server.await.unwrap();
        let tokens = requests
            .iter()
            .map(|r| bearer(r).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(tokens, vec!["tok1", "tok2", "tok2", "tok3"]);
        let fingerprint = p.identity_fingerprint(&id).await.unwrap();
        assert_eq!(fingerprint.len(), 64);
        assert!(!fingerprint.contains("tok3"));
        crate::credential::invalidate(&id);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejection_after_key_info_never_switches_identity_mid_fetch() {
        let (base, server) = serve(vec![
            json(
                "200 OK",
                j!({"info": {"user_id": "u1", "team_id": "t1"}}).to_string(),
            ),
            json("401 Unauthorized", "{}"),
            json("404 Not Found", "{}"),
        ])
        .await;
        let temp = tempfile::tempdir().unwrap();
        let id = format!("litellm_pin_{}", uuid::Uuid::new_v4().simple());
        let p = LiteLLMProvider {
            endpoint: base,
            credential: Some(counting_credential(&temp.path().join("count"))),
            ..Default::default()
        };
        let snapshot = p.fetch_account(&id).await;
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests
            .iter()
            .all(|r| bearer(r).as_deref() == Some("tok1")));
        assert_eq!(
            snapshot.subjects[1].state,
            LiteLLMFeatureState::AuthRequired
        );
        assert_eq!(snapshot.subjects[2].state, LiteLLMFeatureState::Unsupported);
        assert_eq!(
            snapshot.subjects[2].max_budget.kind,
            LiteLLMBudgetKind::Unknown
        );
        crate::credential::invalidate(&id);
    }

    #[test]
    fn unknown_native_shapes_and_secret_shaped_ids_are_ignored() {
        let subject = subject_from(
            LiteLLMSubjectScope::Key,
            None,
            "k".into(),
            &j!({"spend": "NaN", "max_budget": -5, "remaining": "inf", "blocked": "yes",
                 "model_max_budget": [1, 2], "budget_duration": "Bearer abc"}),
        );
        assert_eq!(subject.spend.state, LiteLLMSpendState::Unknown);
        assert_eq!(subject.max_budget.kind, LiteLLMBudgetKind::Unknown);
        assert_eq!(subject.remaining.value, None);
        assert_eq!(subject.blocked, None);
        assert!(subject.model_limits.is_empty());
        assert_eq!(subject.budget_duration, None);
        assert_eq!(identifier(Some(&j!("sk-leaked"))), None);
        assert_eq!(identifier(Some(&j!("has space"))), None);
        assert!(parse_mcp_inventory(&j!({"unexpected": true})).is_err());
    }

    #[tokio::test]
    async fn fingerprint_changes_with_bearer_and_base() {
        let a = provider("https://proxy.example", "sk-a");
        let b = provider("https://proxy.example", "sk-b");
        let c = provider("https://other.example", "sk-a");
        let fa = a.identity_fingerprint("litellm").await.unwrap();
        assert_ne!(fa, b.identity_fingerprint("litellm").await.unwrap());
        assert_ne!(fa, c.identity_fingerprint("litellm").await.unwrap());
        assert_ne!(fa, a.identity_fingerprint("litellm_two").await.unwrap());
        assert_eq!(a.resolve_bearer("litellm").await.unwrap().expose(), "sk-a");
    }

    #[tokio::test]
    async fn mcp_status_reads_only_selected_ids_and_never_leaks_values() {
        let (base, server) = serve(vec![
            json(
                "200 OK",
                j!([{"server_id": "srv-1", "status": "healthy", "url": "https://up.internal"},
                    {"server_id": "other", "status": "unhealthy"}])
                .to_string(),
            ),
            json(
                "200 OK",
                j!({"server_id": "srv-1", "has_credential": true, "is_expired": false,
                    "expires_at": "2030-01-01T00:00:00Z arbitrarytoken123", "access_token": "tok-x"})
                .to_string(),
            ),
            json(
                "200 OK",
                j!({"server_id": "srv-1", "missing_count": 2,
                    "setup_url": "https://evil.example/setup",
                    "required": [{"name": "API", "value": "env-secret"}]})
                .to_string(),
            ),
            json("403 Forbidden", "{}"),
            json("404 Not Found", "{}"),
        ])
        .await;
        let p = provider(&base, "arbitrarytoken123");
        let snapshot = p
            .fetch_mcp_status("litellm", &["srv-1".into(), "srv-2".into(), "srv-1".into()])
            .await;
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 5);
        assert_eq!(
            request_line(&requests[0]),
            "GET /v1/mcp/server/health?server_ids=srv-1&server_ids=srv-2 HTTP/1.1"
        );
        assert_eq!(
            request_line(&requests[1]),
            "GET /v1/mcp/server/srv-1/oauth-user-credential/status HTTP/1.1"
        );
        assert_eq!(
            request_line(&requests[2]),
            "GET /v1/mcp/server/srv-1/user-env-vars HTTP/1.1"
        );
        assert_eq!(snapshot.state, LiteLLMFeatureState::Ok);
        let first = &snapshot.servers[0];
        assert_eq!(first.health, LiteLLMMcpHealth::Healthy);
        assert_eq!(first.oauth_state, LiteLLMMcpOauthState::Connected);
        assert_eq!(first.oauth_expires_at, None);
        assert_eq!(first.missing_setup, Some(true));
        assert!(first.setup_url.is_none());
        let second = &snapshot.servers[1];
        assert_eq!(second.health, LiteLLMMcpHealth::Unknown);
        assert_eq!(second.oauth_state, LiteLLMMcpOauthState::PermissionDenied);
        assert_eq!(second.missing_setup, None);
        let text = serde_json::to_string(&snapshot).unwrap();
        for secret in [
            "arbitrarytoken123",
            "tok-x",
            "env-secret",
            "up.internal",
            "other",
            "evil",
        ] {
            assert!(!text.contains(secret), "{secret}");
        }
        assert_eq!(
            same_origin_setup_url(
                "https://gw.example",
                Some(&j!("https://gw.example/ui/setup"))
            ),
            Some("https://gw.example/ui/setup".to_string())
        );
        assert_eq!(
            same_origin_setup_url("https://gw.example", Some(&j!("https://gw.example:8443/x"))),
            None
        );
    }

    #[tokio::test]
    async fn mcp_status_malformed_health_and_budget_are_unknown_not_zero() {
        let (base, server) = serve(vec![
            json("200 OK", r#"{"unexpected": 1}"#),
            json("200 OK", r#"{"has_credential": "yes"}"#),
            json("200 OK", r#"{"missing_count": "many"}"#),
        ])
        .await;
        let p = provider(&base, "sk-a");
        let snapshot = p.fetch_mcp_status("litellm", &["srv-1".into()]).await;
        server.await.unwrap();
        let s = &snapshot.servers[0];
        assert_eq!(s.health, LiteLLMMcpHealth::Unknown);
        assert_eq!(s.oauth_state, LiteLLMMcpOauthState::Unknown);
        assert_eq!(s.missing_setup, None);

        let mut session = NativeSession::new(
            "http://127.0.0.1:1".into(),
            LiteLLMBearer::new("sk-a".into()),
            "litellm",
            None,
            HashMap::new(),
        );
        let ids = vec!["srv-1".to_string()];
        let out = fetch_mcp_status_with_budget(
            &mut session,
            &ids,
            Duration::ZERO,
            Duration::from_secs(2),
        )
        .await;
        assert_eq!(out.servers[0].health, LiteLLMMcpHealth::Unknown);
        assert_eq!(out.servers[0].oauth_state, LiteLLMMcpOauthState::Unknown);
        assert_eq!(out.servers[0].missing_setup, None);

        let too_many: Vec<String> = (0..17).map(|i| format!("s{i}")).collect();
        assert_eq!(
            p.fetch_mcp_status("litellm", &too_many).await.state,
            LiteLLMFeatureState::MalformedResponse
        );
        let bad = vec!["../x".to_string()];
        assert_eq!(
            p.fetch_mcp_status("litellm", &bad).await.state,
            LiteLLMFeatureState::MalformedResponse
        );
    }

    #[tokio::test]
    async fn mcp_inventory_exposes_only_safe_fields() {
        let body = j!([
            {"server_id": "srv-1", "server_name": "github", "alias": "gh",
             "description": "GitHub", "url": "https://upstream.internal/mcp?token=x",
             "auth_type": "oauth2", "oauth2_flow": "authorization_code", "transport": "http",
             "credentials": {"auth_value": "ghp_secret"}, "env": {"TOKEN": "env-secret"},
             "static_headers": {"Authorization": "Bearer upstream"}, "command": "npx",
             "mcp_info": {"server_name": "GitHub MCP"}},
            {"server_id": "bad", "server_name": "../etc"}
        ]);
        let (base, server) = serve(vec![json("200 OK", body.to_string())]).await;
        let p = provider(&base, "sk-a");
        let inventory = p.fetch_mcp_inventory("litellm").await;
        let requests = server.await.unwrap();
        assert_eq!(request_line(&requests[0]), "GET /v1/mcp/server HTTP/1.1");
        assert_eq!(inventory.state, LiteLLMFeatureState::Ok);
        assert_eq!(inventory.servers.len(), 1);
        let server = &inventory.servers[0];
        assert_eq!(server.id, "srv-1");
        assert_eq!(server.mcp_endpoint_path, "/github/mcp");
        assert_eq!(server.display_name.as_deref(), Some("GitHub MCP"));
        assert_eq!(server.oauth2_flow.as_deref(), Some("authorization_code"));
        let serialized = serde_json::to_string(&inventory).unwrap();
        for secret in [
            "upstream.internal",
            "ghp_secret",
            "env-secret",
            "Bearer upstream",
            "npx",
        ] {
            assert!(!serialized.contains(secret), "{secret}");
        }
        assert_eq!(
            p.mcp_endpoint("github").unwrap(),
            format!("{base}/github/mcp")
        );
        assert!(p.mcp_endpoint("../x").is_err());
    }
}
