use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AMutex;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const SCOPE: &str = "openid profile email offline_access";

const CODEX_CALLBACK_PORT: u16 = 1455;
const CODEX_FALLBACK_CALLBACK_PORT: u16 = 1457;
const SESSION_TTL_SECS: i64 = 600;
const BIND_RETRY_ATTEMPTS: u32 = 5;
const BIND_RETRY_DELAY: Duration = Duration::from_millis(200);
const MAX_CALLBACK_CONNECTIONS: usize = 16;

const OAUTH_TOKEN_HTTP_TIMEOUT: Duration = Duration::from_secs(20);

async fn read_oauth_error_body_bounded(mut response: reqwest::Response) -> String {
    const MAX_BYTES: usize = 16 * 1024;
    const READ_TIMEOUT: Duration = Duration::from_secs(5);

    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;

    let read_fut = async {
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    let remaining = MAX_BYTES.saturating_sub(buf.len());
                    if remaining == 0 {
                        truncated = true;
                        break;
                    }
                    if chunk.len() > remaining {
                        buf.extend_from_slice(&chunk[..remaining]);
                        truncated = true;
                        break;
                    }
                    buf.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
    };

    let timed_out = tokio::time::timeout(READ_TIMEOUT, read_fut).await.is_err();

    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if truncated {
        text.push_str(" [error body truncated]");
    }
    if timed_out {
        text.push_str(" [error body read timeout]");
    }
    text
}

#[derive(Debug, Clone)]
pub struct PkceSession {
    pub verifier: String,
    pub redirect_uri: String,
    pub created_at: i64,
    pub provider_instance_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OAuthTokens {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub expires_at: i64,
    /// ChatGPT workspace/account id used by ChatGPT backend endpoints.
    #[serde(default)]
    pub chatgpt_account_id: String,
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

    pub fn has_valid_access_token(&self) -> bool {
        !self.access_token.is_empty() && !self.is_expired()
    }

    pub fn has_refresh_token(&self) -> bool {
        !self.refresh_token.is_empty()
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    id_token: String,
}

lazy_static::lazy_static! {
    static ref PENDING_SESSIONS: Arc<AMutex<HashMap<String, PkceSession>>> =
        Arc::new(AMutex::new(HashMap::new()));
}

fn generate_code_verifier() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..64).map(|_| rng.gen::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}

fn generate_code_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}

fn decode_jwt_payload(jwt: &str) -> Option<serde_json::Value> {
    let mut parts = jwt.split('.');
    let _header_b64 = parts.next()?;
    let payload_b64 = parts.next()?;
    let _sig_b64 = parts.next()?;

    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    serde_json::from_slice(&payload_bytes).ok()
}

/// Multi-path fallback mirroring Codex CLI + Roo-Code reference impl:
/// `chatgpt_account_id` -> `https://api.openai.com/auth.chatgpt_account_id`
/// -> `organizations[0].id`.
pub fn extract_chatgpt_account_id_from_jwt(jwt: &str) -> Option<String> {
    let payload = decode_jwt_payload(jwt)?;
    if let Some(v) = payload
        .get("chatgpt_account_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(v.to_string());
    }
    if let Some(v) = payload
        .get("https://api.openai.com/auth")
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(v.to_string());
    }
    payload
        .get("organizations")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

pub fn extract_expiry_from_jwt(jwt: &str) -> Option<i64> {
    decode_jwt_payload(jwt)?
        .get("exp")
        .and_then(|v| v.as_i64())
        .filter(|exp| *exp > 0)
        .and_then(|exp| exp.checked_mul(1000))
}

/// RFC 3986 percent-encoding: encodes spaces as `%20` (not `+` like form-urlencoded).
/// Matches the encoding used by the real Codex CLI (`urlencoding::encode`).
fn percent_encode_param(input: &str) -> String {
    let mut result = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

fn build_authorize_url(code_challenge: &str, state: &str, redirect_uri: &str) -> String {
    let params = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect_uri),
        ("scope", SCOPE),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("state", state),
        ("originator", "codex_cli_rs"),
    ];
    let qs = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, percent_encode_param(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}?{}", AUTHORIZE_URL, qs)
}

async fn prune_expired_sessions(sessions: &mut HashMap<String, PkceSession>) {
    let now = chrono::Utc::now().timestamp();
    sessions.retain(|_, s| now - s.created_at < SESSION_TTL_SECS);
}

async fn register_session(provider_instance_id: String, callback_port: u16) -> (String, String) {
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);
    let session_id = uuid::Uuid::new_v4().to_string();

    let redirect_uri = format!("http://localhost:{}/auth/callback", callback_port);
    let authorize_url = build_authorize_url(&challenge, &session_id, &redirect_uri);

    let session = PkceSession {
        verifier,
        redirect_uri,
        created_at: chrono::Utc::now().timestamp(),
        provider_instance_id,
    };

    let mut sessions = PENDING_SESSIONS.lock().await;
    prune_expired_sessions(&mut sessions).await;
    sessions.insert(session_id.clone(), session);

    (session_id, authorize_url)
}

async fn try_bind(port: u16) -> Option<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(("127.0.0.1", port)).await.ok()
}

async fn send_cancel_request(port: u16) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const CANCEL_IO_TIMEOUT: Duration = Duration::from_secs(2);
    let connect = tokio::net::TcpStream::connect(("127.0.0.1", port));
    let mut stream = match tokio::time::timeout(CANCEL_IO_TIMEOUT, connect).await {
        Ok(Ok(s)) => s,
        _ => return,
    };
    let request = format!(
        "GET /cancel HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        port
    );
    if tokio::time::timeout(CANCEL_IO_TIMEOUT, stream.write_all(request.as_bytes()))
        .await
        .is_err()
    {
        return;
    }
    let mut buf = [0u8; 64];
    let _ = tokio::time::timeout(CANCEL_IO_TIMEOUT, stream.read(&mut buf)).await;
}

async fn bind_callback_listener() -> Result<tokio::net::TcpListener, String> {
    if let Some(listener) = try_bind(CODEX_CALLBACK_PORT).await {
        return Ok(listener);
    }

    send_cancel_request(CODEX_CALLBACK_PORT).await;
    for _ in 0..BIND_RETRY_ATTEMPTS {
        tokio::time::sleep(BIND_RETRY_DELAY).await;
        if let Some(listener) = try_bind(CODEX_CALLBACK_PORT).await {
            return Ok(listener);
        }
    }

    tracing::warn!(
        "OpenAI Codex OAuth: port {} unavailable, falling back to {}",
        CODEX_CALLBACK_PORT,
        CODEX_FALLBACK_CALLBACK_PORT
    );
    if let Some(listener) = try_bind(CODEX_FALLBACK_CALLBACK_PORT).await {
        return Ok(listener);
    }

    Err(format!(
        "OpenAI login callback ports {} and {} are both in use. \
         Close the application using them (for example a pending `codex` CLI login) and try again.",
        CODEX_CALLBACK_PORT, CODEX_FALLBACK_CALLBACK_PORT
    ))
}

pub async fn start_oauth_session(
    provider_instance_id: impl Into<String>,
) -> Result<(String, String, tokio::net::TcpListener), String> {
    let listener = bind_callback_listener().await?;
    let callback_port = listener
        .local_addr()
        .map_err(|e| format!("Cannot read OAuth callback port: {}", e))?
        .port();
    let (session_id, authorize_url) =
        register_session(provider_instance_id.into(), callback_port).await;
    Ok((session_id, authorize_url, listener))
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

pub async fn exchange_code_for_session(
    http_client: &reqwest::Client,
    session_id: &str,
    code: &str,
) -> Result<(OAuthTokens, String), String> {
    let session = {
        let mut sessions = PENDING_SESSIONS.lock().await;
        sessions
            .remove(session_id)
            .ok_or_else(|| "Invalid or expired OAuth session".to_string())?
    };
    let provider_instance_id = session.provider_instance_id.clone();

    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", session.redirect_uri.as_str()),
        ("client_id", CLIENT_ID),
        ("code_verifier", session.verifier.as_str()),
    ];

    let response = http_client
        .post(TOKEN_URL)
        .form(&params)
        .timeout(OAUTH_TOKEN_HTTP_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = read_oauth_error_body_bounded(response).await;
        return Err(format!("Token exchange failed ({}): {}", status, text));
    }

    let token_resp: TokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    let expires_at = if token_resp.expires_in > 0 {
        chrono::Utc::now().timestamp_millis() + token_resp.expires_in * 1000
    } else {
        chrono::Utc::now().timestamp_millis() + 8 * 24 * 3600 * 1000
    };

    let chatgpt_account_id = extract_chatgpt_account_id_from_jwt(&token_resp.id_token)
        .or_else(|| extract_chatgpt_account_id_from_jwt(&token_resp.access_token))
        .unwrap_or_default();
    let _ = http_client; // subscription-only flow no longer exchanges for an API key.

    Ok((
        OAuthTokens {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token,
            expires_at,
            chatgpt_account_id,
        },
        provider_instance_id,
    ))
}

pub async fn refresh_access_token(
    http_client: &reqwest::Client,
    refresh_token: &str,
) -> Result<OAuthTokens, String> {
    let params = [
        ("client_id", CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("scope", "openid profile email"),
    ];

    let response = http_client
        .post(TOKEN_URL)
        .form(&params)
        .timeout(OAUTH_TOKEN_HTTP_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Token refresh request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = read_oauth_error_body_bounded(response).await;
        return Err(format!("Token refresh failed ({}): {}", status, text));
    }

    let token_resp: TokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse refresh response: {}", e))?;

    let expires_at = if token_resp.expires_in > 0 {
        chrono::Utc::now().timestamp_millis() + token_resp.expires_in * 1000
    } else {
        chrono::Utc::now().timestamp_millis() + 8 * 24 * 3600 * 1000
    };

    let chatgpt_account_id = extract_chatgpt_account_id_from_jwt(&token_resp.id_token)
        .or_else(|| extract_chatgpt_account_id_from_jwt(&token_resp.access_token))
        .unwrap_or_default();

    Ok(OAuthTokens {
        access_token: token_resp.access_token,
        refresh_token: if token_resp.refresh_token.is_empty() {
            refresh_token.to_string()
        } else {
            token_resp.refresh_token
        },
        expires_at,
        chatgpt_account_id,
    })
}

pub fn start_callback_listener(
    listener: tokio::net::TcpListener,
    http_client: reqwest::Client,
) -> tokio::task::JoinHandle<Option<(OAuthTokens, String)>> {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
    tracing::info!(
        "OpenAI Codex OAuth: callback listener started on port {}",
        port
    );

    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;

        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(SESSION_TTL_SECS as u64);
        let mut connections_served = 0usize;

        while connections_served < MAX_CALLBACK_CONNECTIONS {
            let (mut stream, _addr) =
                match tokio::time::timeout_at(deadline, listener.accept()).await {
                    Ok(Ok(pair)) => pair,
                    Ok(Err(e)) => {
                        tracing::warn!("OpenAI Codex OAuth: callback accept error: {}", e);
                        return None;
                    }
                    Err(_) => {
                        tracing::info!("OpenAI Codex OAuth: callback listener timed out");
                        return None;
                    }
                };
            connections_served += 1;

            const CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(5);
            let mut buf = vec![0u8; 8192];
            let n = match tokio::time::timeout(CALLBACK_READ_TIMEOUT, stream.read(&mut buf)).await
            {
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    tracing::debug!("OpenAI Codex OAuth: failed to read callback request: {}", e);
                    continue;
                }
                Err(_) => {
                    tracing::debug!("OpenAI Codex OAuth: callback request read timed out");
                    continue;
                }
            };
            if n == 0 {
                continue;
            }
            let request_str = String::from_utf8_lossy(&buf[..n]);

            let first_line = request_str.lines().next().unwrap_or("");
            let path_and_query = first_line.split_whitespace().nth(1).unwrap_or("");

            let parsed = match url::Url::parse(&format!("http://localhost{}", path_and_query)) {
                Ok(u) => u,
                Err(e) => {
                    tracing::debug!("OpenAI Codex OAuth: failed to parse callback URL: {}", e);
                    send_http_response(&mut stream, 400, "Bad Request").await;
                    continue;
                }
            };

            match parsed.path() {
                "/cancel" => {
                    tracing::info!("OpenAI Codex OAuth: login cancelled by a newer login attempt");
                    send_http_response(&mut stream, 200, "Login cancelled").await;
                    return None;
                }
                "/auth/callback" => {}
                _ => {
                    send_http_response(&mut stream, 404, "Not Found").await;
                    continue;
                }
            }

            let params: HashMap<String, String> = parsed.query_pairs().into_owned().collect();

            if let Some(err) = params.get("error") {
                let desc = params
                    .get("error_description")
                    .map(|s| s.as_str())
                    .unwrap_or("Unknown error");
                tracing::warn!("OpenAI Codex OAuth error: {} — {}", err, desc);
                send_http_response(
                    &mut stream,
                    200,
                    &callback_html(false, &format!("{}: {}", err, desc)),
                )
                .await;
                return None;
            }

            let code = match params.get("code") {
                Some(c) if !c.is_empty() => c.clone(),
                _ => {
                    send_http_response(
                        &mut stream,
                        200,
                        &callback_html(false, "No authorization code received"),
                    )
                    .await;
                    return None;
                }
            };

            let session_id = match params.get("state") {
                Some(s) if !s.is_empty() => s.clone(),
                _ => {
                    send_http_response(
                        &mut stream,
                        200,
                        &callback_html(false, "Missing state parameter"),
                    )
                    .await;
                    return None;
                }
            };

            return match exchange_code_for_session(&http_client, &session_id, &code).await {
                Ok((tokens, provider_instance_id)) => {
                    send_http_response(
                        &mut stream,
                        200,
                        &callback_html(
                            true,
                            "Authentication successful. You can close this window.",
                        ),
                    )
                    .await;
                    Some((tokens, provider_instance_id))
                }
                Err(e) => {
                    tracing::warn!("OpenAI Codex OAuth: token exchange failed: {}", e);
                    send_http_response(
                        &mut stream,
                        200,
                        &callback_html(false, &format!("Token exchange failed: {}", e)),
                    )
                    .await;
                    None
                }
            };
        }

        tracing::warn!(
            "OpenAI Codex OAuth: too many connections without a login callback; giving up"
        );
        None
    })
}

fn raw_http_response(status: u16, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        reason,
        body.as_bytes().len(),
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
    // HTML-escape the message to prevent XSS
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pending_oauth_session_tracks_provider_instance_id() {
        clear_pending_sessions_for_test().await;
        let (session_id, authorize_url) =
            register_session("openai_codex_work".to_string(), CODEX_CALLBACK_PORT).await;

        let provider_instance_id = pending_session_provider_instance_id(&session_id).await;
        assert_eq!(provider_instance_id.as_deref(), Some("openai_codex_work"));
        assert!(authorize_url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));

        clear_pending_sessions_for_test().await;
    }

    #[tokio::test]
    async fn authorize_url_only_uses_hydra_allowlisted_ports() {
        clear_pending_sessions_for_test().await;
        let (_session_id, fallback_url) =
            register_session("openai_codex".to_string(), CODEX_FALLBACK_CALLBACK_PORT).await;
        assert!(fallback_url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1457%2Fauth%2Fcallback"));

        clear_pending_sessions_for_test().await;
    }

    #[test]
    fn raw_callback_http_response_includes_csp() {
        let response = raw_http_response(200, "ok");

        assert!(response
            .contains("Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'"));
        assert!(response.contains("Content-Type: text/html; charset=utf-8"));
        assert!(response.contains("Content-Length: 2"));
    }
}
