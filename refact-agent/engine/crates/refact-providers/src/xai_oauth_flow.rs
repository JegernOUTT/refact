use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AMutex;

const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const AUTHORIZE_URL: &str = "https://auth.x.ai/oauth2/authorize";
const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
const XAI_CALLBACK_PORT: u16 = 56121;
const REDIRECT_PATH: &str = "/callback";
const SESSION_TTL_SECS: i64 = 600;
const MAX_CALLBACK_CONNECTIONS: usize = 16;
const OAUTH_TOKEN_HTTP_TIMEOUT: Duration = Duration::from_secs(20);

async fn read_oauth_error_body_bounded(mut response: reqwest::Response) -> String {
    const MAX_BYTES: usize = 16 * 1024;
    const READ_TIMEOUT: Duration = Duration::from_secs(5);

    let mut buf = Vec::new();
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
}

impl OAuthTokens {
    pub fn is_empty(&self) -> bool {
        self.access_token.is_empty() && self.refresh_token.is_empty()
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at == 0 || chrono::Utc::now().timestamp_millis() >= self.expires_at
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
}

lazy_static::lazy_static! {
    static ref PENDING_SESSIONS: Arc<AMutex<HashMap<String, PkceSession>>> =
        Arc::new(AMutex::new(HashMap::new()));
}

fn generate_code_verifier() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..64).map(|_| rng.gen::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn decode_jwt_payload(jwt: &str) -> Option<serde_json::Value> {
    let mut parts = jwt.split('.');
    parts.next()?;
    let payload = parts.next()?;
    parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn extract_expiry_from_jwt(jwt: &str) -> Option<i64> {
    // This unsigned decode is used solely to schedule refresh, never for trust decisions.
    decode_jwt_payload(jwt)?
        .get("exp")
        .and_then(|value| value.as_i64())
        .filter(|expiry| *expiry > 0)
        .and_then(|expiry| expiry.checked_mul(1000))
}

fn percent_encode_param(input: &str) -> String {
    let mut result = String::with_capacity(input.len() * 3);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => result.push_str(&format!("%{:02X}", byte)),
        }
    }
    result
}

fn build_authorize_url(code_challenge: &str, state: &str, nonce: &str) -> String {
    let redirect_uri = format!("http://127.0.0.1:{}{}", XAI_CALLBACK_PORT, REDIRECT_PATH);
    let params = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect_uri.as_str()),
        ("scope", SCOPE),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("nonce", nonce),
        // plan=generic is required for xAI to accept loopback OAuth for this client.
        ("plan", "generic"),
        ("referrer", "refact"),
    ];
    let query = params
        .iter()
        .map(|(key, value)| format!("{}={}", key, percent_encode_param(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}?{}", AUTHORIZE_URL, query)
}

async fn prune_expired_sessions(sessions: &mut HashMap<String, PkceSession>) {
    let now = chrono::Utc::now().timestamp();
    sessions.retain(|_, session| now - session.created_at < SESSION_TTL_SECS);
}

async fn register_session(provider_instance_id: String) -> (String, String) {
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);
    let session_id = uuid::Uuid::new_v4().to_string();
    let nonce = uuid::Uuid::new_v4().to_string();
    let redirect_uri = format!("http://127.0.0.1:{}{}", XAI_CALLBACK_PORT, REDIRECT_PATH);
    let authorize_url = build_authorize_url(&challenge, &session_id, &nonce);
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

pub async fn bind_callback_listener() -> Result<tokio::net::TcpListener, String> {
    // xAI pins the registered loopback redirect to port 56121, so no fallback port is valid.
    tokio::net::TcpListener::bind(("127.0.0.1", XAI_CALLBACK_PORT))
        .await
        .map_err(|error| {
            format!(
                "Cannot bind xAI OAuth callback port {}: {}. Grok CLI or another \
                 Grok-OAuth client is probably already holding port {}.",
                XAI_CALLBACK_PORT, error, XAI_CALLBACK_PORT
            )
        })
}

pub async fn start_oauth_session(
    provider_instance_id: impl Into<String>,
) -> Result<(String, String, tokio::net::TcpListener), String> {
    let listener = bind_callback_listener().await?;
    let (session_id, authorize_url) = register_session(provider_instance_id.into()).await;
    Ok((session_id, authorize_url, listener))
}

pub async fn pending_session_provider_instance_id(session_id: &str) -> Option<String> {
    let sessions = PENDING_SESSIONS.lock().await;
    sessions
        .get(session_id)
        .map(|session| session.provider_instance_id.clone())
}

pub async fn clear_pending_sessions_for_test() {
    PENDING_SESSIONS.lock().await.clear();
}

fn token_expiry(access_token: &str, expires_in: i64) -> i64 {
    extract_expiry_from_jwt(access_token).unwrap_or_else(|| {
        let seconds = if expires_in > 0 { expires_in } else { 3600 };
        chrono::Utc::now().timestamp_millis() + seconds * 1000
    })
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
        .map_err(|error| format!("Token exchange request failed: {}", error))?;
    if !response.status().is_success() {
        let status = response.status();
        let text = read_oauth_error_body_bounded(response).await;
        return Err(format!("Token exchange failed ({}): {}", status, text));
    }
    let token_response: TokenResponse = response
        .json()
        .await
        .map_err(|error| format!("Failed to parse token response: {}", error))?;
    let expires_at = token_expiry(&token_response.access_token, token_response.expires_in);
    Ok((
        OAuthTokens {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token,
            expires_at,
        },
        provider_instance_id,
    ))
}

pub async fn refresh_access_token(
    http_client: &reqwest::Client,
    refresh_token: &str,
) -> Result<OAuthTokens, String> {
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", CLIENT_ID),
    ];
    let response = http_client
        .post(TOKEN_URL)
        .form(&params)
        .timeout(OAUTH_TOKEN_HTTP_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("Token refresh request failed: {}", error))?;
    if !response.status().is_success() {
        let status = response.status();
        let text = read_oauth_error_body_bounded(response).await;
        return Err(format!("Token refresh failed ({}): {}", status, text));
    }
    let token_response: TokenResponse = response
        .json()
        .await
        .map_err(|error| format!("Failed to parse refresh response: {}", error))?;
    let expires_at = token_expiry(&token_response.access_token, token_response.expires_in);
    Ok(OAuthTokens {
        access_token: token_response.access_token,
        refresh_token: if token_response.refresh_token.is_empty() {
            refresh_token.to_string()
        } else {
            token_response.refresh_token
        },
        expires_at,
    })
}

pub fn start_callback_listener(
    listener: tokio::net::TcpListener,
    http_client: reqwest::Client,
) -> tokio::task::JoinHandle<Option<(OAuthTokens, String)>> {
    tracing::info!(
        "xAI OAuth: callback listener started on port {}",
        XAI_CALLBACK_PORT
    );
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(SESSION_TTL_SECS as u64);
        let mut connections_served = 0usize;
        while connections_served < MAX_CALLBACK_CONNECTIONS {
            let (mut stream, _) = match tokio::time::timeout_at(deadline, listener.accept()).await {
                Ok(Ok(pair)) => pair,
                Ok(Err(error)) => {
                    tracing::warn!("xAI OAuth: callback accept error: {}", error);
                    return None;
                }
                Err(_) => {
                    tracing::info!("xAI OAuth: callback listener timed out");
                    return None;
                }
            };
            connections_served += 1;
            let mut buf = vec![0u8; 8192];
            let count =
                match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await {
                    Ok(Ok(count)) => count,
                    Ok(Err(error)) => {
                        tracing::debug!("xAI OAuth: failed to read callback request: {}", error);
                        continue;
                    }
                    Err(_) => {
                        tracing::debug!("xAI OAuth: callback request read timed out");
                        continue;
                    }
                };
            if count == 0 {
                continue;
            }
            let request = String::from_utf8_lossy(&buf[..count]);
            let path_and_query = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("");
            let parsed = match url::Url::parse(&format!("http://127.0.0.1{}", path_and_query)) {
                Ok(url) => url,
                Err(error) => {
                    tracing::debug!("xAI OAuth: failed to parse callback URL: {}", error);
                    send_http_response(&mut stream, 400, "Bad Request").await;
                    continue;
                }
            };
            match parsed.path() {
                "/cancel" => {
                    send_http_response(&mut stream, 200, "Login cancelled").await;
                    return None;
                }
                REDIRECT_PATH => {}
                _ => {
                    send_http_response(&mut stream, 404, "Not Found").await;
                    continue;
                }
            }
            let params: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
            if let Some(error) = params.get("error") {
                let description = params
                    .get("error_description")
                    .map(String::as_str)
                    .unwrap_or("Unknown error");
                send_http_response(
                    &mut stream,
                    200,
                    &callback_html(false, &format!("{}: {}", error, description)),
                )
                .await;
                return None;
            }
            let state = match params.get("state").filter(|state| !state.is_empty()) {
                Some(state) => state.clone(),
                None => {
                    send_http_response(
                        &mut stream,
                        200,
                        &callback_html(false, "CSRF validation failed: missing state parameter"),
                    )
                    .await;
                    return None;
                }
            };
            if pending_session_provider_instance_id(&state).await.is_none() {
                send_http_response(
                    &mut stream,
                    200,
                    &callback_html(false, "CSRF validation failed: state mismatch"),
                )
                .await;
                return None;
            }
            let code = match params.get("code").filter(|code| !code.is_empty()) {
                Some(code) => code.clone(),
                None => {
                    send_http_response(
                        &mut stream,
                        200,
                        &callback_html(false, "No authorization code received"),
                    )
                    .await;
                    return None;
                }
            };
            return match exchange_code_for_session(&http_client, &state, &code).await {
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
                Err(error) => {
                    tracing::warn!("xAI OAuth: token exchange failed: {}", error);
                    send_http_response(
                        &mut stream,
                        200,
                        &callback_html(false, &format!("Token exchange failed: {}", error)),
                    )
                    .await;
                    None
                }
            };
        }
        tracing::warn!("xAI OAuth: too many callback connections; giving up");
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
    let escaped_message = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;");
    if success {
        return concat!(
            "<!DOCTYPE html><html><head><title>Authentication Successful</title></head>",
            "<body><p>Authentication successful. You can close this window.</p></body></html>"
        )
        .to_string();
    }
    format!(
        concat!(
            "<!DOCTYPE html><html><head><title>Authentication Failed</title></head>",
            "<body><h1>Authentication Failed</h1><p>{}</p></body></html>"
        ),
        escaped_message
    )
}
