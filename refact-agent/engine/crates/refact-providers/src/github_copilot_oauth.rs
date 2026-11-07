use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AMutex;

const CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";
const SCOPE: &str = "read:user";
const PUBLIC_GITHUB_DOMAIN: &str = "github.com";
pub const DEFAULT_COPILOT_API_BASE: &str = "https://api.githubcopilot.com";
const REQUEST_TIMEOUT_SECS: u64 = 10;
const SESSION_TTL_SECS: i64 = 900;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OAuthTokens {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enterprise_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
}

impl OAuthTokens {
    pub fn is_expired(&self) -> bool {
        self.expires_at > 0 && chrono::Utc::now().timestamp_millis() >= self.expires_at
    }

    pub fn has_valid_access_token(&self) -> bool {
        !self.access_token.is_empty() && !self.is_expired()
    }
}

#[derive(Debug, Clone)]
struct PendingDeviceSession {
    device_code: String,
    poll_interval: u64,
    expires_at: i64,
    token_url: String,
    enterprise_url: Option<String>,
    api_base: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DeviceStartResponse {
    pub session_id: String,
    pub authorize_url: String,
    pub verification_uri: String,
    pub user_code: String,
    pub instructions: String,
    pub poll_interval: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DevicePollOutcome {
    Success(OAuthTokens),
    AuthorizationPending { poll_interval: u64 },
    SlowDown { poll_interval: u64 },
    ExpiredToken { message: String },
    AccessDenied { message: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub authorize_url: String,
    pub expires_in: i64,
    pub poll_interval: u64,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeWire {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct TokenWire {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
    #[serde(default)]
    interval: Option<u64>,
}

lazy_static::lazy_static! {
    static ref PENDING_SESSIONS: Arc<AMutex<HashMap<String, PendingDeviceSession>>> =
        Arc::new(AMutex::new(HashMap::new()));
}

pub fn normalize_enterprise_domain(input: &str) -> Result<String, String> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("GitHub Enterprise URL or domain is empty".to_string());
    }
    let url_text = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = Url::parse(&url_text)
        .map_err(|e| format!("Invalid GitHub Enterprise URL '{input}': {e}"))?;
    if url.scheme() != "https" {
        return Err("GitHub Enterprise URL must use https".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("GitHub Enterprise URL must not include userinfo".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("GitHub Enterprise URL must not include query or fragment".to_string());
    }
    if url.port().is_some_and(|port| port != 443) {
        return Err("GitHub Enterprise URL must not use a non-default port".to_string());
    }
    let path = url.path().trim_matches('/');
    if !path.is_empty() {
        return Err("GitHub Enterprise URL must be a domain, not a path".to_string());
    }
    let host =
        normalized_host(&url).ok_or_else(|| "GitHub Enterprise URL has no host".to_string())?;
    if is_local_or_private_host(&host) {
        return Err(format!(
            "GitHub Enterprise host '{host}' is local or private and cannot be used for OAuth"
        ));
    }
    Ok(host)
}

pub fn api_base_for_enterprise_domain(domain: &str) -> Result<String, String> {
    let normalized = normalize_enterprise_domain(domain)?;
    validate_api_base(&format!("https://copilot-api.{normalized}"))
}

pub fn validate_api_base(api_base: &str) -> Result<String, String> {
    let trimmed = api_base.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("GitHub Copilot API base is empty".to_string());
    }
    let url = Url::parse(trimmed)
        .map_err(|e| format!("Invalid GitHub Copilot API base '{api_base}': {e}"))?;
    if url.scheme() != "https" {
        return Err("GitHub Copilot API base must use https".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("GitHub Copilot API base must not include userinfo".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("GitHub Copilot API base must not include query or fragment".to_string());
    }
    if url.port().is_some_and(|port| port != 443) {
        return Err("GitHub Copilot API base must not use a non-default port".to_string());
    }
    let path = url.path().trim_matches('/');
    if !path.is_empty() {
        return Err("GitHub Copilot API base must be a base URL, not an endpoint path".to_string());
    }
    let host =
        normalized_host(&url).ok_or_else(|| "GitHub Copilot API base has no host".to_string())?;
    if is_local_or_private_host(&host) {
        return Err(format!(
            "GitHub Copilot API base host '{host}' is local or private and cannot receive credentials"
        ));
    }
    if host != "api.githubcopilot.com" && !host.starts_with("copilot-api.") {
        return Err(format!(
            "GitHub Copilot API base host '{host}' is not an explicit Copilot host"
        ));
    }
    Ok(format!("{}://{}", url.scheme(), host))
}

pub fn resolve_api_base(
    enterprise_url: Option<&str>,
    api_base: Option<&str>,
) -> Result<String, String> {
    if let Some(api_base) = api_base.map(str::trim).filter(|value| !value.is_empty()) {
        return validate_api_base(api_base);
    }
    if let Some(enterprise_url) = enterprise_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return api_base_for_enterprise_domain(enterprise_url);
    }
    Ok(DEFAULT_COPILOT_API_BASE.to_string())
}

pub fn parse_device_code_response(value: &serde_json::Value) -> Result<ParsedDeviceCode, String> {
    let wire: DeviceCodeWire = serde_json::from_value(value.clone())
        .map_err(|e| format!("Failed to parse GitHub device-code response: {e}"))?;
    if wire.device_code.trim().is_empty() {
        return Err("GitHub device-code response missing device_code".to_string());
    }
    if wire.user_code.trim().is_empty() {
        return Err("GitHub device-code response missing user_code".to_string());
    }
    if wire.verification_uri.trim().is_empty() {
        return Err("GitHub device-code response missing verification_uri".to_string());
    }
    let authorize_url = wire
        .verification_uri_complete
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&wire.verification_uri)
        .to_string();
    Ok(ParsedDeviceCode {
        device_code: wire.device_code,
        user_code: wire.user_code,
        verification_uri: wire.verification_uri,
        authorize_url,
        expires_in: if wire.expires_in > 0 {
            wire.expires_in
        } else {
            SESSION_TTL_SECS
        },
        poll_interval: if wire.interval > 0 { wire.interval } else { 5 },
    })
}

pub fn parse_token_poll_response(
    value: &serde_json::Value,
    current_poll_interval: u64,
    enterprise_url: Option<String>,
    api_base: String,
) -> Result<DevicePollOutcome, String> {
    let wire: TokenWire = serde_json::from_value(value.clone())
        .map_err(|e| format!("Failed to parse GitHub device token response: {e}"))?;
    if !wire.access_token.trim().is_empty() {
        let expires_at = if wire.expires_in > 0 {
            chrono::Utc::now().timestamp_millis() + wire.expires_in * 1000
        } else {
            0
        };
        return Ok(DevicePollOutcome::Success(OAuthTokens {
            access_token: wire.access_token,
            expires_at,
            enterprise_url,
            api_base: Some(validate_api_base(&api_base)?),
        }));
    }

    let description = if wire.error_description.trim().is_empty() {
        match wire.error.as_str() {
            "authorization_pending" => "Authorization is still pending".to_string(),
            "slow_down" => "GitHub requested a slower polling interval".to_string(),
            "expired_token" => {
                "The device authorization code expired. Start login again.".to_string()
            }
            "access_denied" => {
                "GitHub authorization was denied. Start login again if needed.".to_string()
            }
            other if !other.is_empty() => other.to_string(),
            _ => "GitHub device token response did not include an access token".to_string(),
        }
    } else {
        wire.error_description
    };

    match wire.error.as_str() {
        "authorization_pending" => Ok(DevicePollOutcome::AuthorizationPending {
            poll_interval: wire
                .interval
                .filter(|interval| *interval > 0)
                .unwrap_or(current_poll_interval),
        }),
        "slow_down" => Ok(DevicePollOutcome::SlowDown {
            poll_interval: wire
                .interval
                .filter(|interval| *interval > 0)
                .unwrap_or(current_poll_interval + 5),
        }),
        "expired_token" => Ok(DevicePollOutcome::ExpiredToken {
            message: description,
        }),
        "access_denied" => Ok(DevicePollOutcome::AccessDenied {
            message: description,
        }),
        _ => Ok(DevicePollOutcome::Error {
            message: description,
        }),
    }
}

pub async fn start_oauth_session(
    http_client: &reqwest::Client,
    enterprise_url: Option<&str>,
) -> Result<DeviceStartResponse, String> {
    let (domain, stored_enterprise_url, api_base) = if let Some(enterprise_url) = enterprise_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let domain = normalize_enterprise_domain(enterprise_url)?;
        let api_base = api_base_for_enterprise_domain(&domain)?;
        (domain.clone(), Some(domain), api_base)
    } else {
        (
            PUBLIC_GITHUB_DOMAIN.to_string(),
            None,
            DEFAULT_COPILOT_API_BASE.to_string(),
        )
    };
    let device_url = format!("https://{domain}/login/device/code");
    let token_url = format!("https://{domain}/login/oauth/access_token");
    let response = tokio::time::timeout(
        Duration::from_secs(REQUEST_TIMEOUT_SECS),
        http_client
            .post(&device_url)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(
                reqwest::header::USER_AGENT,
                format!("refact-lsp {}", env!("CARGO_PKG_VERSION")),
            )
            .json(&serde_json::json!({
                "client_id": CLIENT_ID,
                "scope": SCOPE,
            }))
            .send(),
    )
    .await
    .map_err(|_| "GitHub device-code request timed out".to_string())?
    .map_err(|e| format!("GitHub device-code request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!(
            "GitHub device-code request failed ({status}): {text}"
        ));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub device-code response: {e}"))?;
    let parsed = parse_device_code_response(&body)?;
    let session_id = uuid::Uuid::new_v4().to_string();
    let expires_at = chrono::Utc::now().timestamp() + parsed.expires_in;
    let session = PendingDeviceSession {
        device_code: parsed.device_code,
        poll_interval: parsed.poll_interval,
        expires_at,
        token_url,
        enterprise_url: stored_enterprise_url,
        api_base,
    };

    let mut sessions = PENDING_SESSIONS.lock().await;
    prune_expired_sessions(&mut sessions);
    sessions.insert(session_id.clone(), session);

    Ok(DeviceStartResponse {
        session_id,
        authorize_url: parsed.authorize_url,
        verification_uri: parsed.verification_uri,
        user_code: parsed.user_code.clone(),
        instructions: format!("Enter code: {}", parsed.user_code),
        poll_interval: parsed.poll_interval,
    })
}

pub async fn poll_oauth_session(
    http_client: &reqwest::Client,
    session_id: &str,
) -> Result<DevicePollOutcome, String> {
    let session = {
        let mut sessions = PENDING_SESSIONS.lock().await;
        prune_expired_sessions(&mut sessions);
        sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| "Invalid or expired GitHub Copilot OAuth session".to_string())?
    };

    if chrono::Utc::now().timestamp() >= session.expires_at {
        let mut sessions = PENDING_SESSIONS.lock().await;
        sessions.remove(session_id);
        return Ok(DevicePollOutcome::ExpiredToken {
            message: "The device authorization code expired. Start login again.".to_string(),
        });
    }

    let response = tokio::time::timeout(
        Duration::from_secs(REQUEST_TIMEOUT_SECS),
        http_client
            .post(&session.token_url)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(
                reqwest::header::USER_AGENT,
                format!("refact-lsp {}", env!("CARGO_PKG_VERSION")),
            )
            .json(&serde_json::json!({
                "client_id": CLIENT_ID,
                "device_code": session.device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            }))
            .send(),
    )
    .await
    .map_err(|_| "GitHub device token request timed out".to_string())?
    .map_err(|e| format!("GitHub device token request failed: {e}"))?;

    let status = response.status();
    let body_text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "GitHub device token request failed ({status}): {body_text}"
        ));
    }
    let body: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("Failed to parse GitHub device token response: {e}"))?;
    let outcome = parse_token_poll_response(
        &body,
        session.poll_interval,
        session.enterprise_url.clone(),
        session.api_base.clone(),
    )?;

    let mut sessions = PENDING_SESSIONS.lock().await;
    match &outcome {
        DevicePollOutcome::AuthorizationPending {
            poll_interval,
        }
        | DevicePollOutcome::SlowDown {
            poll_interval,
        } => {
            if let Some(stored) = sessions.get_mut(session_id) {
                stored.poll_interval = *poll_interval;
            }
        }
        DevicePollOutcome::Success(_)
        | DevicePollOutcome::ExpiredToken {
            ..
        }
        | DevicePollOutcome::AccessDenied {
            ..
        }
        | DevicePollOutcome::Error {
            ..
        } => {
            sessions.remove(session_id);
        }
    }

    Ok(outcome)
}

fn prune_expired_sessions(sessions: &mut HashMap<String, PendingDeviceSession>) {
    let now = chrono::Utc::now().timestamp();
    sessions.retain(|_, session| now < session.expires_at);
}

fn normalized_host(url: &Url) -> Option<String> {
    url.host_str().map(|host| {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        host.strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(&host)
            .to_string()
    })
}

fn is_local_or_private_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    IpAddr::from_str(host)
        .map(|ip| match ip {
            IpAddr::V4(ip) => is_private_ipv4(ip),
            IpAddr::V6(ip) => is_private_ipv6(ip),
        })
        .unwrap_or(false)
}

fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 198 && (18..=19).contains(&octets[1]))
}

fn is_private_ipv6(ip: Ipv6Addr) -> bool {
    ip.to_ipv4_mapped().map_or(false, is_private_ipv4)
        || ip.is_loopback()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || ip.is_unspecified()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn github_copilot_enterprise_domain_normalization_and_api_base() {
        assert_eq!(
            normalize_enterprise_domain("https://Company.GHE.COM/").unwrap(),
            "company.ghe.com"
        );
        assert_eq!(
            normalize_enterprise_domain("company.ghe.com").unwrap(),
            "company.ghe.com"
        );
        assert_eq!(
            api_base_for_enterprise_domain("company.ghe.com").unwrap(),
            "https://copilot-api.company.ghe.com"
        );
        assert_eq!(
            resolve_api_base(None, None).unwrap(),
            DEFAULT_COPILOT_API_BASE
        );
        assert!(normalize_enterprise_domain("http://company.ghe.com").is_err());
        assert!(normalize_enterprise_domain("https://company.ghe.com/path").is_err());
        assert!(normalize_enterprise_domain("https://localhost").is_err());
        assert!(validate_api_base("https://evil.example.com").is_err());
        assert!(validate_api_base("https://copilot-api.company.ghe.com/v1").is_err());
        assert!(validate_api_base("https://api.githubcopilot.com").is_ok());
    }

    #[test]
    fn github_copilot_device_code_start_response_parses() {
        let parsed = parse_device_code_response(&json!({
            "device_code": "device-123",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://github.com/login/device",
            "verification_uri_complete": "https://github.com/login/device?user_code=ABCD-EFGH",
            "expires_in": 900,
            "interval": 7
        }))
        .unwrap();

        assert_eq!(parsed.device_code, "device-123");
        assert_eq!(parsed.user_code, "ABCD-EFGH");
        assert_eq!(parsed.verification_uri, "https://github.com/login/device");
        assert_eq!(
            parsed.authorize_url,
            "https://github.com/login/device?user_code=ABCD-EFGH"
        );
        assert_eq!(parsed.poll_interval, 7);
        assert_eq!(parsed.expires_in, 900);
    }

    #[test]
    fn github_copilot_token_poll_response_parses_pending_slowdown_and_success() {
        let pending = parse_token_poll_response(
            &json!({"error": "authorization_pending"}),
            5,
            None,
            DEFAULT_COPILOT_API_BASE.to_string(),
        )
        .unwrap();
        assert_eq!(
            pending,
            DevicePollOutcome::AuthorizationPending {
                poll_interval: 5
            }
        );

        let slow_down = parse_token_poll_response(
            &json!({"error": "slow_down"}),
            5,
            None,
            DEFAULT_COPILOT_API_BASE.to_string(),
        )
        .unwrap();
        assert_eq!(
            slow_down,
            DevicePollOutcome::SlowDown {
                poll_interval: 10
            }
        );

        let server_slow_down = parse_token_poll_response(
            &json!({"error": "slow_down", "interval": 13}),
            5,
            None,
            DEFAULT_COPILOT_API_BASE.to_string(),
        )
        .unwrap();
        assert_eq!(
            server_slow_down,
            DevicePollOutcome::SlowDown {
                poll_interval: 13
            }
        );

        let success = parse_token_poll_response(
            &json!({"access_token": "gho-token"}),
            5,
            Some("company.ghe.com".to_string()),
            "https://copilot-api.company.ghe.com".to_string(),
        )
        .unwrap();
        assert_eq!(
            success,
            DevicePollOutcome::Success(OAuthTokens {
                access_token: "gho-token".to_string(),
                expires_at: 0,
                enterprise_url: Some("company.ghe.com".to_string()),
                api_base: Some("https://copilot-api.company.ghe.com".to_string()),
            })
        );
    }

    #[test]
    fn github_copilot_token_poll_response_parses_expired_and_denied() {
        let expired = parse_token_poll_response(
            &json!({"error": "expired_token"}),
            5,
            None,
            DEFAULT_COPILOT_API_BASE.to_string(),
        )
        .unwrap();
        assert!(matches!(expired, DevicePollOutcome::ExpiredToken { .. }));

        let denied = parse_token_poll_response(
            &json!({"error": "access_denied", "error_description": "user denied"}),
            5,
            None,
            DEFAULT_COPILOT_API_BASE.to_string(),
        )
        .unwrap();
        assert_eq!(
            denied,
            DevicePollOutcome::AccessDenied {
                message: "user denied".to_string()
            }
        );
    }
}
