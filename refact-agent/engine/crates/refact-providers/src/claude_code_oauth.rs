use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::header::HeaderMap;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AMutex;

use refact_llm::adapters::claude_code_compat;

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const SCOPE: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
const SESSION_TTL_SECS: i64 = 600;
const CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(5);
const CALLBACK_REQUEST_MAX_BYTES: usize = 8192;

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
    pub redirect_uri: String,
    #[allow(dead_code)]
    pub authorize_url: String,
    #[allow(dead_code)]
    pub mode: OAuthMode,
    pub provider_instance_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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

fn expires_at_ms(now_ms: i64, expires_in_secs: i64) -> i64 {
    now_ms.saturating_add(expires_in_secs.max(0).saturating_mul(1000))
}

fn oauth_tokens_from_token_response(token_resp: TokenResponse, now_ms: i64) -> OAuthTokens {
    OAuthTokens {
        access_token: token_resp.access_token,
        refresh_token: token_resp.refresh_token.trim().to_string(),
        expires_at: expires_at_ms(now_ms, token_resp.expires_in),
    }
}

fn oauth_tokens_from_refresh_response(
    token_resp: RefreshTokenResponse,
    old_refresh_token: &str,
    now_ms: i64,
) -> OAuthTokens {
    let refresh_token = token_resp
        .refresh_token
        .map(|refresh_token| refresh_token.trim().to_string())
        .filter(|refresh_token| !refresh_token.is_empty())
        .unwrap_or_else(|| old_refresh_token.to_string());

    OAuthTokens {
        access_token: token_resp.access_token,
        refresh_token,
        expires_at: expires_at_ms(now_ms, token_resp.expires_in),
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

fn build_authorize_url(
    _mode: &OAuthMode,
    code_challenge: &str,
    state: &str,
    redirect_uri: &str,
) -> String {
    let mut url = url::Url::parse("https://claude.ai/oauth/authorize").expect("valid base URL");

    url.query_pairs_mut()
        .append_pair("code", "true")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
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

pub async fn bind_callback_listener() -> Result<(tokio::net::TcpListener, u16), String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Cannot bind Claude Code OAuth callback listener: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Cannot read Claude Code OAuth callback port: {}", e))?
        .port();
    Ok((listener, port))
}

pub async fn start_oauth_session(
    mode: OAuthMode,
    callback_port: u16,
    provider_instance_id: impl Into<String>,
) -> (String, String) {
    let verifier = generate_code_verifier();
    let state = generate_state();
    let challenge = generate_code_challenge(&verifier);
    let redirect_uri = format!("http://localhost:{}/callback", callback_port);
    let authorize_url = build_authorize_url(&mode, &challenge, &state, &redirect_uri);

    let session_id = uuid::Uuid::new_v4().to_string();
    let session = PkceSession {
        verifier,
        state,
        redirect_uri,
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

async fn exchange_code_with_session(
    http_client: &reqwest::Client,
    session: PkceSession,
    code: &str,
    state: &str,
    expected_provider_instance_id: Option<&str>,
) -> Result<(OAuthTokens, String), String> {
    if let Some(expected_provider_instance_id) = expected_provider_instance_id {
        if session.provider_instance_id != expected_provider_instance_id {
            return Err(format!(
                "OAuth session belongs to provider '{}'",
                session.provider_instance_id
            ));
        }
    }

    if state != session.state {
        return Err("OAuth state mismatch".to_string());
    }

    let body = serde_json::json!({
        "code": code,
        "state": state,
        "grant_type": "authorization_code",
        "client_id": CLIENT_ID,
        "redirect_uri": session.redirect_uri,
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

    let tokens =
        oauth_tokens_from_token_response(token_resp, chrono::Utc::now().timestamp_millis());

    Ok((tokens, session.provider_instance_id))
}

fn parse_code_and_state_input(input: &str) -> Result<(String, String), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Authorization code is empty".to_string());
    }

    if let Ok(url) = url::Url::parse(trimmed) {
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        if let (Some(code), Some(state)) = (params.get("code"), params.get("state")) {
            if !code.is_empty() && !state.is_empty() {
                return Ok((code.clone(), state.clone()));
            }
        }
    }

    let query = trimmed.strip_prefix('?').unwrap_or(trimmed);
    if query.contains('=') && query.contains('&') {
        let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        if let (Some(code), Some(state)) = (params.get("code"), params.get("state")) {
            if !code.is_empty() && !state.is_empty() {
                return Ok((code.clone(), state.clone()));
            }
        }
    }

    let parts: Vec<&str> = trimmed.split('#').collect();
    if parts.len() > 1 && !parts[0].is_empty() && !parts[1].is_empty() {
        return Ok((parts[0].to_string(), parts[1].to_string()));
    }

    Err("Authorization input must include both code and state".to_string())
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

    let (code, state) = parse_code_and_state_input(code_raw)?;
    exchange_code_with_session(
        http_client,
        session,
        &code,
        &state,
        Some(expected_provider_instance_id),
    )
    .await
}

pub async fn exchange_code_for_state(
    http_client: &reqwest::Client,
    state: &str,
    code: &str,
) -> Result<(OAuthTokens, String), String> {
    let session = {
        let mut sessions = PENDING_SESSIONS.lock().await;
        prune_expired_sessions(&mut sessions);
        let session_id = sessions
            .iter()
            .find_map(|(session_id, session)| {
                if session.state == state {
                    Some(session_id.clone())
                } else {
                    None
                }
            })
            .ok_or_else(|| "Invalid or expired OAuth session".to_string())?;
        sessions
            .remove(&session_id)
            .ok_or_else(|| "Invalid or expired OAuth session".to_string())?
    };

    exchange_code_with_session(http_client, session, code, state, None).await
}

fn raw_http_response(status: u16, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Error",
    };
    format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        body.len(),
        body
    )
}

async fn send_http_response(stream: &mut tokio::net::TcpStream, status: u16, body: &str) {
    use tokio::io::AsyncWriteExt;
    let response = raw_http_response(status, body);
    let _ = stream.write_all(response.as_bytes()).await;
}

fn callback_html(success: bool, message: &str) -> String {
    let (title, heading, color) = if success {
        (
            "Authentication Successful",
            "&#x2713; Authentication Successful",
            "#4ade80",
        )
    } else {
        (
            "Authentication Failed",
            "&#x2717; Authentication Failed",
            "#ef4444",
        )
    };
    let escaped_message = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;");
    format!(
        r#"<!DOCTYPE html>
<html><head><title>{title}</title></head>
<body style="font-family: system-ui; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; background: #1a1a2e; color: #e0e0e0;">
<div style="text-align: center;">
<h1 style="color: {color};">{heading}</h1>
<p>{escaped_message}</p>
</div>
</body></html>"#
    )
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Result<String, String> {
    use tokio::io::AsyncReadExt;

    let read_fut = async {
        let mut buf = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];
        loop {
            if buf.len() >= CALLBACK_REQUEST_MAX_BYTES {
                break;
            }
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|e| format!("failed to read callback request: {}", e))?;
            if n == 0 {
                break;
            }
            let remaining = CALLBACK_REQUEST_MAX_BYTES.saturating_sub(buf.len());
            buf.extend_from_slice(&chunk[..n.min(remaining)]);
            if buf.windows(4).any(|window| window == b"\r\n\r\n")
                || buf.windows(2).any(|window| window == b"\n\n")
            {
                break;
            }
        }
        Ok::<String, String>(String::from_utf8_lossy(&buf).into_owned())
    };

    tokio::time::timeout(CALLBACK_READ_TIMEOUT, read_fut)
        .await
        .map_err(|_| "callback request read timed out".to_string())?
}

async fn handle_callback_stream<F, Fut>(
    mut stream: tokio::net::TcpStream,
    http_client: &reqwest::Client,
    on_success: &F,
) -> Option<Option<(OAuthTokens, String)>>
where
    F: Fn(OAuthTokens, String) -> Fut + Send + Sync,
    Fut: Future<Output = Result<(), String>> + Send,
{
    let request_str = match read_http_request(&mut stream).await {
        Ok(request) => request,
        Err(e) => {
            tracing::warn!("Claude Code OAuth: {}", e);
            return Some(None);
        }
    };
    let first_line = request_str.lines().next().unwrap_or("");
    let path_and_query = first_line.split_whitespace().nth(1).unwrap_or("");

    let parsed = match url::Url::parse(&format!("http://localhost{}", path_and_query)) {
        Ok(url) => url,
        Err(e) => {
            tracing::warn!("Claude Code OAuth: failed to parse callback URL: {}", e);
            send_http_response(&mut stream, 400, "Bad Request").await;
            return Some(None);
        }
    };
    if parsed.path() != "/callback" {
        send_http_response(&mut stream, 400, "Bad Request").await;
        return None;
    }

    let params: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
    if let Some(err) = params.get("error") {
        let desc = params
            .get("error_description")
            .map(|s| s.as_str())
            .unwrap_or("Unknown error");
        tracing::warn!("Claude Code OAuth error: {} — {}", err, desc);
        send_http_response(
            &mut stream,
            200,
            &callback_html(false, &format!("{}: {}", err, desc)),
        )
        .await;
        return Some(None);
    }

    let code = match params.get("code") {
        Some(code) if !code.is_empty() => code.clone(),
        _ => {
            send_http_response(
                &mut stream,
                200,
                &callback_html(false, "No authorization code received"),
            )
            .await;
            return Some(None);
        }
    };
    let state = match params.get("state") {
        Some(state) if !state.is_empty() => state.clone(),
        _ => {
            send_http_response(
                &mut stream,
                200,
                &callback_html(false, "Missing state parameter"),
            )
            .await;
            return Some(None);
        }
    };

    match exchange_code_for_state(http_client, &state, &code).await {
        Ok((tokens, provider_instance_id)) => {
            if let Err(e) = on_success(tokens.clone(), provider_instance_id.clone()).await {
                tracing::warn!("Claude Code OAuth: token save failed: {}", e);
                send_http_response(
                    &mut stream,
                    200,
                    &callback_html(false, &format!("Token save failed: {}", e)),
                )
                .await;
                return Some(None);
            }
            send_http_response(
                &mut stream,
                200,
                &callback_html(
                    true,
                    "Authentication successful. You can close this window.",
                ),
            )
            .await;
            Some(Some((tokens, provider_instance_id)))
        }
        Err(e) => {
            tracing::warn!("Claude Code OAuth: token exchange failed: {}", e);
            send_http_response(
                &mut stream,
                200,
                &callback_html(false, &format!("Token exchange failed: {}", e)),
            )
            .await;
            Some(None)
        }
    }
}

pub async fn start_callback_listener<F, Fut>(
    listener: tokio::net::TcpListener,
    http_client: reqwest::Client,
    on_success: F,
) -> Result<tokio::task::JoinHandle<Option<(OAuthTokens, String)>>, String>
where
    F: Fn(OAuthTokens, String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), String>> + Send + 'static,
{
    let port = listener
        .local_addr()
        .map_err(|e| format!("Cannot read Claude Code callback listener address: {}", e))?
        .port();
    tracing::info!(
        "Claude Code OAuth: callback listener started on port {}",
        port
    );

    let handle = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(SESSION_TTL_SECS as u64);
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                tracing::info!("Claude Code OAuth: callback listener timed out");
                return None;
            }
            let accept_result = tokio::time::timeout_at(deadline, listener.accept()).await;
            let (stream, _addr) = match accept_result {
                Ok(Ok((stream, addr))) => (stream, addr),
                Ok(Err(e)) => {
                    tracing::warn!("Claude Code OAuth: callback accept error: {}", e);
                    return None;
                }
                Err(_) => {
                    tracing::info!("Claude Code OAuth: callback listener timed out");
                    return None;
                }
            };
            match handle_callback_stream(stream, &http_client, &on_success).await {
                Some(result) => return result,
                None => continue,
            }
        }
    });

    Ok(handle)
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
    fn refresh_response_with_whitespace_around_refresh_token_trims_new_token() {
        let token_resp: RefreshTokenResponse = serde_json::from_value(serde_json::json!({
            "access_token": "new-access-token",
            "refresh_token": " refresh-new ",
            "expires_in": 3600,
        }))
        .unwrap();

        let tokens = oauth_tokens_from_refresh_response(token_resp, "old-refresh-token", 1000);

        assert_eq!(tokens.access_token, "new-access-token");
        assert_eq!(tokens.refresh_token, "refresh-new");
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

    #[test]
    fn exchange_response_trims_refresh_token() {
        let token_resp: TokenResponse = serde_json::from_value(serde_json::json!({
            "access_token": "new-access-token",
            "refresh_token": " exchange-new ",
            "expires_in": 3600,
        }))
        .unwrap();

        let tokens = oauth_tokens_from_token_response(token_resp, 1000);

        assert_eq!(tokens.access_token, "new-access-token");
        assert_eq!(tokens.refresh_token, "exchange-new");
        assert_eq!(tokens.expires_at, 3_601_000);
    }

    #[test]
    fn raw_callback_http_response_includes_csp() {
        let response = raw_http_response(200, "ok");

        assert!(response
            .contains("Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'"));
        assert!(response.contains("Content-Type: text/html; charset=utf-8"));
        assert!(response.contains("Content-Length: 2"));
    }

    #[test]
    fn parses_callback_url_authorization_input() {
        let (code, state) = parse_code_and_state_input(
            "http://localhost:34179/callback?code=abc%20123&state=state-456",
        )
        .unwrap();

        assert_eq!(code, "abc 123");
        assert_eq!(state, "state-456");
    }

    #[test]
    fn parses_raw_query_authorization_input() {
        let (code, state) = parse_code_and_state_input("?code=abc&state=state").unwrap();

        assert_eq!(code, "abc");
        assert_eq!(state, "state");
    }

    #[test]
    fn parses_legacy_code_hash_state_authorization_input() {
        let (code, state) = parse_code_and_state_input("abc#state").unwrap();

        assert_eq!(code, "abc");
        assert_eq!(state, "state");
    }

    #[test]
    fn expiry_saturates_for_huge_expires_in() {
        assert_eq!(expires_at_ms(1000, i64::MAX), i64::MAX);
    }

    #[test]
    fn expiry_clamps_negative_expires_in_to_now() {
        assert_eq!(expires_at_ms(1000, -3600), 1000);
    }

    #[tokio::test]
    async fn pending_oauth_session_tracks_provider_instance_id() {
        let _guard = pending_sessions_test_guard().await;
        clear_pending_sessions_for_test().await;
        let (session_id, _) = start_oauth_session(OAuthMode::Max, 34179, "claude_code_work").await;

        let provider_instance_id = pending_session_provider_instance_id(&session_id).await;
        assert_eq!(provider_instance_id.as_deref(), Some("claude_code_work"));

        clear_pending_sessions_for_test().await;
    }

    #[tokio::test]
    async fn oauth_session_uses_independent_authorize_state() {
        let _guard = pending_sessions_test_guard().await;
        clear_pending_sessions_for_test().await;
        let (session_id, authorize_url) =
            start_oauth_session(OAuthMode::Max, 34179, "claude_code_work").await;

        let sessions = PENDING_SESSIONS.lock().await;
        let session = sessions.get(&session_id).unwrap();
        assert_ne!(session.state, session.verifier);
        let url = url::Url::parse(&authorize_url).unwrap();
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(params.get("state"), Some(&session.state));
        assert_eq!(
            params.get("redirect_uri").map(String::as_str),
            Some("http://localhost:34179/callback")
        );
        assert!(params
            .get("scope")
            .is_some_and(|scope| scope.contains("user:sessions:claude_code")));
        assert_ne!(params.get("state"), Some(&session.verifier));
        assert!(!authorize_url.contains(&session.verifier));
        drop(sessions);

        clear_pending_sessions_for_test().await;
    }

    #[tokio::test]
    async fn mismatched_provider_exchange_rejects_and_removes_session() {
        let _guard = pending_sessions_test_guard().await;
        clear_pending_sessions_for_test().await;
        let (session_id, _) = start_oauth_session(OAuthMode::Max, 34179, "claude_code_work").await;
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
        let (session_id, _) = start_oauth_session(OAuthMode::Max, 34179, "claude_code_work").await;
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
        let (session_id, _) = start_oauth_session(OAuthMode::Max, 34179, "claude_code_work").await;
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
