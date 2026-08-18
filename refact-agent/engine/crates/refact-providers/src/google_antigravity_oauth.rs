use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AMutex;

const CLIENT_ID: &str =
    "REDACTED-ANTIGRAVITY-CLIENT-ID.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "REDACTED-ANTIGRAVITY-CLIENT-SECRET";
const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const CALLBACK_PORT: u16 = 51121;
const REDIRECT_PATH: &str = "/oauth-callback";
const SESSION_TTL_SECS: i64 = 600;
const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/cclog",
    "https://www.googleapis.com/auth/experimentsandconfigs",
];
const ENDPOINT_PROD: &str = "https://cloudcode-pa.googleapis.com";
const ENDPOINT_DAILY: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";
const ENDPOINT_AUTOPUSH: &str = "https://autopush-cloudcode-pa.sandbox.googleapis.com";
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
    #[serde(default)]
    pub project_id: String,
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

fn redirect_uri() -> String {
    // The callback port and path are registered with this OAuth client, so they are fixed.
    format!("http://localhost:{}{}", CALLBACK_PORT, REDIRECT_PATH)
}

fn build_authorize_url(code_challenge: &str, state: &str, redirect_uri: &str) -> String {
    let scope = SCOPES.join(" ");
    let params = [
        ("client_id", CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", redirect_uri),
        ("scope", scope.as_str()),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        // Offline access with consent makes Google return a refresh token.
        ("access_type", "offline"),
        ("prompt", "consent"),
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
    let redirect_uri = redirect_uri();
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

pub async fn bind_callback_listener() -> Result<tokio::net::TcpListener, String> {
    tokio::net::TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .map_err(|error| {
            format!(
                "Cannot bind Google Antigravity OAuth callback port {}: {}. Another OAuth client \
                 may already be holding the registered port.",
                CALLBACK_PORT, error
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

fn token_expiry(expires_in: i64) -> i64 {
    let seconds = if expires_in > 0 { expires_in } else { 3600 };
    chrono::Utc::now().timestamp_millis() + seconds * 1000
}

pub use refact_core::antigravity_client::{
    antigravity_headers, antigravity_platform, ANTIGRAVITY_VERSION,
};

pub async fn discover_project_id(
    http_client: &reqwest::Client,
    access_token: &str,
) -> Result<String, String> {
    let metadata = serde_json::json!({
        "metadata": {
            "ideType": "ANTIGRAVITY",
            "platform": antigravity_platform(),
            "pluginType": "GEMINI"
        }
    });
    let mut failures = Vec::new();
    for endpoint in [ENDPOINT_PROD, ENDPOINT_DAILY, ENDPOINT_AUTOPUSH] {
        let url = format!("{}/v1internal:loadCodeAssist", endpoint);
        let mut request = http_client
            .post(&url)
            .bearer_auth(access_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json");
        for (name, value) in antigravity_headers() {
            let header_name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| format!("Invalid Antigravity header name {}: {}", name, error))?;
            request = request.header(header_name, value);
        }
        let response = match request
            .json(&metadata)
            .timeout(OAUTH_TOKEN_HTTP_TIMEOUT)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                failures.push(format!(
                    "status unavailable at {}: body unavailable (request failed: {})",
                    endpoint, error
                ));
                continue;
            }
        };
        let status = response.status();
        let body = read_oauth_error_body_bounded(response).await;
        if status.is_success() {
            match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(value) => {
                    let project = value.get("cloudaicompanionProject");
                    let project_id = project
                        .and_then(|value| value.as_str())
                        .or_else(|| {
                            project
                                .and_then(|value| value.as_object())
                                .and_then(|value| value.get("id"))
                                .and_then(|value| value.as_str())
                        })
                        .filter(|value| !value.is_empty());
                    if let Some(project_id) = project_id {
                        return Ok(project_id.to_string());
                    }
                    failures.push(format!(
                        "status {} at {} returned no Cloud Code project: {}",
                        status, endpoint, body
                    ));
                }
                Err(error) => failures.push(format!(
                    "status {} at {} returned invalid JSON ({}): {}",
                    status, endpoint, error, body
                )),
            }
        } else {
            failures.push(format!("status {} at {}: {}", status, endpoint, body));
        }
    }
    // A guessed project id causes misleading authorization failures, so discovery has no fallback.
    Err(format!(
        "Unable to discover an Antigravity Cloud Code project: {}",
        failures.join("; ")
    ))
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
        ("client_secret", CLIENT_SECRET),
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
    let project_id = discover_project_id(http_client, &token_response.access_token).await?;
    Ok((
        OAuthTokens {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token,
            expires_at: token_expiry(token_response.expires_in),
            project_id,
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
        ("client_secret", CLIENT_SECRET),
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
    let project_id = discover_project_id(http_client, &token_response.access_token).await?;
    // Google usually omits refresh_token on refresh, so retain the previous token when absent.
    let refreshed_refresh_token = if token_response.refresh_token.is_empty() {
        refresh_token.to_string()
    } else {
        token_response.refresh_token
    };
    Ok(OAuthTokens {
        access_token: token_response.access_token,
        refresh_token: refreshed_refresh_token,
        expires_at: token_expiry(token_response.expires_in),
        project_id,
    })
}

pub fn start_callback_listener(
    listener: tokio::net::TcpListener,
    http_client: reqwest::Client,
) -> tokio::task::JoinHandle<Option<(OAuthTokens, String)>> {
    tracing::info!(
        "Google Antigravity OAuth: callback listener started on port {}",
        CALLBACK_PORT
    );
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(SESSION_TTL_SECS as u64);
        let mut connections_served = 0usize;
        while connections_served < MAX_CALLBACK_CONNECTIONS {
            let (mut stream, _) = match tokio::time::timeout_at(deadline, listener.accept()).await {
                Ok(Ok(pair)) => pair,
                Ok(Err(error)) => {
                    tracing::warn!("Google Antigravity OAuth: callback accept error: {}", error);
                    return None;
                }
                Err(_) => {
                    tracing::info!("Google Antigravity OAuth: callback listener timed out");
                    return None;
                }
            };
            connections_served += 1;
            let mut buf = vec![0u8; 8192];
            let count = match tokio::time::timeout(
                Duration::from_secs(5),
                stream.read(&mut buf),
            )
            .await
            {
                Ok(Ok(count)) => count,
                Ok(Err(error)) => {
                    tracing::debug!(
                        "Google Antigravity OAuth: failed to read callback request: {}",
                        error
                    );
                    continue;
                }
                Err(_) => {
                    tracing::debug!("Google Antigravity OAuth: callback request read timed out");
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
            let parsed = match url::Url::parse(&format!("http://localhost{}", path_and_query)) {
                Ok(url) => url,
                Err(error) => {
                    tracing::debug!(
                        "Google Antigravity OAuth: failed to parse callback URL: {}",
                        error
                    );
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
                    tracing::warn!("Google Antigravity OAuth: token exchange failed: {}", error);
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
        tracing::warn!("Google Antigravity OAuth: too many callback connections; giving up");
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
        concat!(
            "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\n",
            "Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\n",
            "Content-Length: {}\r\nConnection: close\r\n\r\n{}"
        ),
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
