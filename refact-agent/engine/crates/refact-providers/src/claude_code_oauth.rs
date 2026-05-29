use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::header::HeaderMap;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AMutex;

use refact_llm::adapters::claude_code_compat;

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const SCOPE: &str = "org:create_api_key user:profile user:inference";
const SESSION_TTL_SECS: i64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OAuthMode {
    #[default]
    Max,
}

#[derive(Debug, Clone)]
pub struct PkceSession {
    pub verifier: String,
    pub state: String,
    #[allow(dead_code)]
    pub authorize_url: String,
    #[allow(dead_code)]
    pub mode: OAuthMode,
    pub provider_instance_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OAuthTokens {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub expires_at: i64,
}

impl OAuthTokens {
    pub fn is_empty(&self) -> bool {
        self.access_token.is_empty() && self.refresh_token.is_empty()
    }

    pub fn is_expired(&self) -> bool {
        if self.expires_at == 0 {
            return true;
        }
        chrono::Utc::now().timestamp_millis() >= self.expires_at
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

#[derive(Debug, Deserialize)]
struct RefreshTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: i64,
}

fn oauth_tokens_from_refresh_response(
    token_resp: RefreshTokenResponse,
    old_refresh_token: &str,
    now_ms: i64,
) -> OAuthTokens {
    let refresh_token = token_resp
        .refresh_token
        .filter(|refresh_token| !refresh_token.trim().is_empty())
        .unwrap_or_else(|| old_refresh_token.to_string());

    OAuthTokens {
        access_token: token_resp.access_token,
        refresh_token,
        expires_at: now_ms + token_resp.expires_in * 1000,
    }
}

fn token_request_headers() -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static(claude_code_compat::USER_AGENT),
    );
    claude_code_compat::apply_stainless_headers(&mut headers, None)?;
    Ok(headers)
}

lazy_static::lazy_static! {
    static ref PENDING_SESSIONS: Arc<AMutex<HashMap<String, PkceSession>>> =
        Arc::new(AMutex::new(HashMap::new()));
}

#[cfg(test)]
lazy_static::lazy_static! {
    static ref PENDING_SESSIONS_TEST_LOCK: AMutex<()> = AMutex::new(());
}

#[cfg(test)]
pub async fn pending_sessions_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    PENDING_SESSIONS_TEST_LOCK.lock().await
}

fn generate_code_verifier() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::thread_rng();
    (0..64)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill(&mut bytes[..]);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_code_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}

fn build_authorize_url(_mode: &OAuthMode, code_challenge: &str, state: &str) -> String {
    let mut url = url::Url::parse("https://claude.ai/oauth/authorize").expect("valid base URL");

    url.query_pairs_mut()
        .append_pair("code", "true")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);

    url.to_string()
}

fn prune_expired_sessions(sessions: &mut HashMap<String, PkceSession>) {
    let now = chrono::Utc::now().timestamp();
    sessions.retain(|_, session| now - session.created_at < SESSION_TTL_SECS);
}

pub async fn start_oauth_session(
    mode: OAuthMode,
    provider_instance_id: impl Into<String>,
) -> (String, String) {
    let verifier = generate_code_verifier();
    let state = generate_state();
    let challenge = generate_code_challenge(&verifier);
    let authorize_url = build_authorize_url(&mode, &challenge, &state);

    let session_id = uuid::Uuid::new_v4().to_string();
    let session = PkceSession {
        verifier,
        state,
        authorize_url: authorize_url.clone(),
        mode,
        provider_instance_id: provider_instance_id.into(),
        created_at: chrono::Utc::now().timestamp(),
    };

    let mut sessions = PENDING_SESSIONS.lock().await;
    prune_expired_sessions(&mut sessions);
    sessions.insert(session_id.clone(), session);

    (session_id, authorize_url)
}

pub async fn exchange_code(
    http_client: &reqwest::Client,
    session_id: &str,
    code_raw: &str,
    expected_provider_instance_id: &str,
) -> Result<(OAuthTokens, String), String> {
    let session = {
        let mut sessions = PENDING_SESSIONS.lock().await;
        prune_expired_sessions(&mut sessions);
        sessions
            .remove(session_id)
            .ok_or_else(|| "Invalid or expired OAuth session".to_string())?
    };
    if session.provider_instance_id != expected_provider_instance_id {
        return Err(format!(
            "OAuth session belongs to provider '{}'",
            session.provider_instance_id
        ));
    }

    let parts: Vec<&str> = code_raw.split('#').collect();
    let code = parts[0];
    let state = if parts.len() > 1 { parts[1] } else { "" };
    if state != session.state {
        return Err("OAuth state mismatch".to_string());
    }

    let body = serde_json::json!({
        "code": code,
        "state": state,
        "grant_type": "authorization_code",
        "client_id": CLIENT_ID,
        "redirect_uri": REDIRECT_URI,
        "code_verifier": session.verifier,
    });

    let response = http_client
        .post(TOKEN_URL)
        .headers(token_request_headers()?)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Token exchange failed ({}): {}", status, text));
    }

    let token_resp: TokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    let expires_at = chrono::Utc::now().timestamp_millis() + token_resp.expires_in * 1000;

    Ok((
        OAuthTokens {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token,
            expires_at,
        },
        session.provider_instance_id,
    ))
}

pub async fn refresh_access_token(
    http_client: &reqwest::Client,
    refresh_token: &str,
) -> Result<OAuthTokens, String> {
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
    });

    let response = http_client
        .post(TOKEN_URL)
        .headers(token_request_headers()?)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Token refresh request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Token refresh failed ({}): {}", status, text));
    }

    let token_resp: RefreshTokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse refresh response: {}", e))?;

    Ok(oauth_tokens_from_refresh_response(
        token_resp,
        refresh_token,
        chrono::Utc::now().timestamp_millis(),
    ))
}

#[cfg(test)]
pub async fn pending_session_provider_instance_id(session_id: &str) -> Option<String> {
    let sessions = PENDING_SESSIONS.lock().await;
    sessions
        .get(session_id)
        .map(|session| session.provider_instance_id.clone())
}

#[cfg(test)]
pub async fn clear_pending_sessions_for_test() {
    let mut sessions = PENDING_SESSIONS.lock().await;
    sessions.clear();
}

#[cfg(test)]
pub async fn expire_pending_session_for_test(session_id: &str) {
    let mut sessions = PENDING_SESSIONS.lock().await;
    if let Some(session) = sessions.get_mut(session_id) {
        session.created_at = chrono::Utc::now().timestamp() - SESSION_TTL_SECS - 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_request_headers_match_claude_cli_identity() {
        let headers = token_request_headers().unwrap();

        assert_eq!(
            headers.get(reqwest::header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        assert_eq!(
            headers.get(reqwest::header::USER_AGENT).unwrap(),
            claude_code_compat::USER_AGENT
        );
        assert_eq!(headers.get("x-app").unwrap(), "cli");
        assert_eq!(headers.get("x-stainless-lang").unwrap(), "js");
        assert_eq!(
            headers
                .get("anthropic-dangerous-direct-browser-access")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn refresh_response_without_refresh_token_preserves_old_token() {
        let token_resp: RefreshTokenResponse = serde_json::from_value(serde_json::json!({
            "access_token": "new-access-token",
            "expires_in": 3600,
        }))
        .unwrap();

        let tokens = oauth_tokens_from_refresh_response(token_resp, "old-refresh-token", 1000);

        assert_eq!(tokens.access_token, "new-access-token");
        assert_eq!(tokens.refresh_token, "old-refresh-token");
        assert_eq!(tokens.expires_at, 3_601_000);
    }

    #[test]
    fn refresh_response_with_empty_refresh_token_preserves_old_token() {
        let token_resp: RefreshTokenResponse = serde_json::from_value(serde_json::json!({
            "access_token": "new-access-token",
            "refresh_token": "",
            "expires_in": 3600,
        }))
        .unwrap();

        let tokens = oauth_tokens_from_refresh_response(token_resp, "old-refresh-token", 1000);

        assert_eq!(tokens.access_token, "new-access-token");
        assert_eq!(tokens.refresh_token, "old-refresh-token");
        assert_eq!(tokens.expires_at, 3_601_000);
    }

    #[test]
    fn refresh_response_with_whitespace_refresh_token_preserves_old_token() {
        let token_resp: RefreshTokenResponse = serde_json::from_value(serde_json::json!({
            "access_token": "new-access-token",
            "refresh_token": "   ",
            "expires_in": 3600,
        }))
        .unwrap();

        let tokens = oauth_tokens_from_refresh_response(token_resp, "old-refresh-token", 1000);

        assert_eq!(tokens.access_token, "new-access-token");
        assert_eq!(tokens.refresh_token, "old-refresh-token");
        assert_eq!(tokens.expires_at, 3_601_000);
    }

    #[test]
    fn refresh_response_with_new_refresh_token_uses_new_token() {
        let token_resp: RefreshTokenResponse = serde_json::from_value(serde_json::json!({
            "access_token": "new-access-token",
            "refresh_token": "new-refresh-token",
            "expires_in": 3600,
        }))
        .unwrap();

        let tokens = oauth_tokens_from_refresh_response(token_resp, "old-refresh-token", 1000);

        assert_eq!(tokens.access_token, "new-access-token");
        assert_eq!(tokens.refresh_token, "new-refresh-token");
        assert_eq!(tokens.expires_at, 3_601_000);
    }

    #[tokio::test]
    async fn pending_oauth_session_tracks_provider_instance_id() {
        let _guard = pending_sessions_test_guard().await;
        clear_pending_sessions_for_test().await;
        let (session_id, _) = start_oauth_session(OAuthMode::Max, "claude_code_work").await;

        let provider_instance_id = pending_session_provider_instance_id(&session_id).await;
        assert_eq!(provider_instance_id.as_deref(), Some("claude_code_work"));

        clear_pending_sessions_for_test().await;
    }

    #[tokio::test]
    async fn oauth_session_uses_independent_authorize_state() {
        let _guard = pending_sessions_test_guard().await;
        clear_pending_sessions_for_test().await;
        let (session_id, authorize_url) =
            start_oauth_session(OAuthMode::Max, "claude_code_work").await;

        let sessions = PENDING_SESSIONS.lock().await;
        let session = sessions.get(&session_id).unwrap();
        assert_ne!(session.state, session.verifier);
        let url = url::Url::parse(&authorize_url).unwrap();
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(params.get("state"), Some(&session.state));
        assert_ne!(params.get("state"), Some(&session.verifier));
        assert!(!authorize_url.contains(&session.verifier));
        drop(sessions);

        clear_pending_sessions_for_test().await;
    }

    #[tokio::test]
    async fn mismatched_provider_exchange_rejects_and_removes_session() {
        let _guard = pending_sessions_test_guard().await;
        clear_pending_sessions_for_test().await;
        let (session_id, _) = start_oauth_session(OAuthMode::Max, "claude_code_work").await;
        let client = reqwest::Client::new();

        let err = exchange_code(&client, &session_id, "code#state", "claude_code")
            .await
            .unwrap_err();

        assert!(err.contains("claude_code_work"));
        assert!(pending_session_provider_instance_id(&session_id)
            .await
            .is_none());
        clear_pending_sessions_for_test().await;
    }

    #[tokio::test]
    async fn mismatched_state_exchange_rejects_before_token_request() {
        let _guard = pending_sessions_test_guard().await;
        clear_pending_sessions_for_test().await;
        let (session_id, _) = start_oauth_session(OAuthMode::Max, "claude_code_work").await;
        let client = reqwest::Client::new();

        let err = exchange_code(
            &client,
            &session_id,
            "code#mismatched-state",
            "claude_code_work",
        )
        .await
        .unwrap_err();

        assert!(err.contains("OAuth state mismatch"));
        assert!(pending_session_provider_instance_id(&session_id)
            .await
            .is_none());
        clear_pending_sessions_for_test().await;
    }

    #[tokio::test]
    async fn expired_oauth_session_is_rejected_and_pruned() {
        let _guard = pending_sessions_test_guard().await;
        clear_pending_sessions_for_test().await;
        let (session_id, _) = start_oauth_session(OAuthMode::Max, "claude_code_work").await;
        expire_pending_session_for_test(&session_id).await;
        let client = reqwest::Client::new();

        let err = exchange_code(&client, &session_id, "code#state", "claude_code_work")
            .await
            .unwrap_err();

        assert!(err.contains("Invalid or expired OAuth session"));
        assert!(pending_session_provider_instance_id(&session_id)
            .await
            .is_none());
        clear_pending_sessions_for_test().await;
    }
}
