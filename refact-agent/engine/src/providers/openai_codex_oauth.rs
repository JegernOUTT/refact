use std::collections::HashMap;
use std::sync::Arc;

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

const CODEX_HOME_DIR: &str = ".codex";
const CODEX_CALLBACK_PORT: u16 = 1455;
const SESSION_TTL_SECS: i64 = 600;

#[derive(Debug, Clone)]
pub struct PkceSession {
    pub verifier: String,
    pub redirect_uri: String,
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
    /// Platform API key obtained via OAuth token-exchange (requested_token=openai-api-key).
    /// This is what should be used against https://api.openai.com endpoints.
    #[serde(default)]
    pub openai_api_key: String,
    /// ChatGPT workspace/account id used by ChatGPT backend endpoints.
    #[serde(default)]
    pub chatgpt_account_id: String,
    /// If token-exchange to OPENAI_API_KEY fails, store a short diagnostic here.
    #[serde(default)]
    pub api_key_exchange_error: String,
}

impl OAuthTokens {
    pub fn is_empty(&self) -> bool {
        self.access_token.is_empty()
            && self.refresh_token.is_empty()
            && self.openai_api_key.is_empty()
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

#[derive(Debug, Deserialize)]
struct CodexCliCredentials {
    #[serde(rename = "OPENAI_API_KEY")]
    #[allow(dead_code)]
    openai_api_key: Option<String>,
    tokens: Option<CodexCliTokens>,
}

#[derive(Debug, Deserialize)]
struct CodexCliTokens {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[allow(dead_code)]
    id_token: Option<serde_json::Value>,
}

fn json_string(value: &Option<serde_json::Value>) -> Option<&str> {
    value
        .as_ref()
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
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

fn codex_home_dir() -> Option<std::path::PathBuf> {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        let path = std::path::PathBuf::from(codex_home);
        if path.exists() {
            return Some(path);
        }
    }
    home::home_dir().map(|h| h.join(CODEX_HOME_DIR))
}

pub fn read_codex_cli_credentials() -> Result<OAuthTokens, String> {
    let codex_home = codex_home_dir().ok_or("Cannot determine Codex home directory")?;

    let auth_path = codex_home.join("auth.json");
    if !auth_path.exists() {
        return Err(format!(
            "Codex CLI credentials not found at {}. Run 'codex login' first.",
            auth_path.display()
        ));
    }

    let content = std::fs::read_to_string(&auth_path)
        .map_err(|e| format!("Failed to read {}: {}", auth_path.display(), e))?;

    let creds: CodexCliCredentials = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", auth_path.display(), e))?;

    // In Codex CLI, OPENAI_API_KEY is obtained via token exchange and is the correct
    // credential for api.openai.com endpoints. Prefer it if present.
    if let Some(api_key) = creds.openai_api_key.as_ref().filter(|k| !k.is_empty()) {
        return Ok(OAuthTokens {
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: 0,
            openai_api_key: api_key.clone(),
            chatgpt_account_id: String::new(),
            api_key_exchange_error: String::new(),
        });
    }

    let tokens = creds.tokens.ok_or_else(|| {
        "No Codex CLI credentials found (expected OPENAI_API_KEY or OAuth tokens). Run 'codex login' first."
            .to_string()
    })?;

    if tokens.access_token.is_empty() {
        return Err("Empty access token in Codex CLI credentials".to_string());
    }

    let chatgpt_account_id = json_string(&tokens.id_token)
        .and_then(extract_chatgpt_account_id_from_jwt)
        .or_else(|| extract_chatgpt_account_id_from_jwt(&tokens.access_token))
        .unwrap_or_default();
    let expires_at = extract_expiry_from_jwt(&tokens.access_token)
        .or_else(|| json_string(&tokens.id_token).and_then(extract_expiry_from_jwt))
        .unwrap_or(i64::MAX);

    Ok(OAuthTokens {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at,
        openai_api_key: String::new(),
        chatgpt_account_id,
        api_key_exchange_error: String::new(),
    })
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

fn extract_chatgpt_account_id_from_jwt(jwt: &str) -> Option<String> {
    decode_jwt_payload(jwt)?
        .get("https://api.openai.com/auth")
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn extract_expiry_from_jwt(jwt: &str) -> Option<i64> {
    decode_jwt_payload(jwt)?
        .get("exp")
        .and_then(|v| v.as_i64())
        .filter(|exp| *exp > 0)
        .and_then(|exp| exp.checked_mul(1000))
}

pub fn codex_cli_credentials_exist() -> bool {
    codex_home_dir()
        .map(|h| h.join("auth.json").exists())
        .unwrap_or(false)
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

/// Returns (session_id, authorize_url, callback_port).
/// The callback_port is the port used in the redirect_uri (1455 if available, fallback otherwise).
pub async fn start_oauth_session(fallback_port: u16) -> (String, String, u16) {
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);
    let session_id = uuid::Uuid::new_v4().to_string();

    // Use port 1455 (Codex CLI default) as primary; fall back to app port
    let callback_port = if port_available(CODEX_CALLBACK_PORT) {
        CODEX_CALLBACK_PORT
    } else {
        tracing::warn!(
            "OpenAI Codex OAuth: port {} unavailable, falling back to {}",
            CODEX_CALLBACK_PORT,
            fallback_port
        );
        fallback_port
    };

    let redirect_uri = format!("http://localhost:{}/auth/callback", callback_port);
    let authorize_url = build_authorize_url(&challenge, &session_id, &redirect_uri);

    let session = PkceSession {
        verifier,
        redirect_uri,
        created_at: chrono::Utc::now().timestamp(),
    };

    let mut sessions = PENDING_SESSIONS.lock().await;
    prune_expired_sessions(&mut sessions).await;
    sessions.insert(session_id.clone(), session);

    (session_id, authorize_url, callback_port)
}

fn port_available(port: u16) -> bool {
    std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok()
}

pub async fn exchange_code(
    http_client: &reqwest::Client,
    session_id: &str,
    code: &str,
) -> Result<OAuthTokens, String> {
    let session = {
        let mut sessions = PENDING_SESSIONS.lock().await;
        sessions
            .remove(session_id)
            .ok_or_else(|| "Invalid or expired OAuth session".to_string())?
    };

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

    let expires_at = if token_resp.expires_in > 0 {
        chrono::Utc::now().timestamp_millis() + token_resp.expires_in * 1000
    } else {
        chrono::Utc::now().timestamp_millis() + 8 * 24 * 3600 * 1000
    };

    let chatgpt_account_id = extract_chatgpt_account_id_from_jwt(&token_resp.id_token)
        .or_else(|| extract_chatgpt_account_id_from_jwt(&token_resp.access_token))
        .unwrap_or_default();

    let (openai_api_key, api_key_exchange_error) = if !token_resp.id_token.is_empty() {
        match obtain_openai_api_key(http_client, &token_resp.id_token).await {
            Ok(k) => (k, String::new()),
            Err(e) => {
                tracing::warn!(
                    "OpenAI Codex OAuth: failed to obtain OPENAI_API_KEY via token-exchange: {e}"
                );
                (String::new(), e)
            }
        }
    } else {
        (
            String::new(),
            "Token exchange response did not include id_token; cannot obtain OPENAI_API_KEY"
                .to_string(),
        )
    };

    Ok(OAuthTokens {
        access_token: token_resp.access_token,
        refresh_token: token_resp.refresh_token,
        expires_at,
        openai_api_key,
        chatgpt_account_id,
        api_key_exchange_error,
    })
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
        .send()
        .await
        .map_err(|e| format!("Token refresh request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
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

    Ok(OAuthTokens {
        access_token: token_resp.access_token,
        refresh_token: if token_resp.refresh_token.is_empty() {
            refresh_token.to_string()
        } else {
            token_resp.refresh_token
        },
        expires_at,
        openai_api_key: String::new(),
        chatgpt_account_id: String::new(),
        api_key_exchange_error: String::new(),
    })
}

async fn obtain_openai_api_key(
    http_client: &reqwest::Client,
    id_token: &str,
) -> Result<String, String> {
    // Mirrors Codex CLI flow: OAuth token exchange for an OpenAI Platform API key.
    // grant_type=urn:ietf:params:oauth:grant-type:token-exchange
    // requested_token=openai-api-key
    // subject_token_type=urn:ietf:params:oauth:token-type:id_token
    #[derive(Deserialize)]
    struct ExchangeResp {
        access_token: String,
    }

    let params = [
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:token-exchange",
        ),
        ("client_id", CLIENT_ID),
        ("requested_token", "openai-api-key"),
        ("subject_token", id_token),
        (
            "subject_token_type",
            "urn:ietf:params:oauth:token-type:id_token",
        ),
    ];

    let response = http_client
        .post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("API key token-exchange request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("API key token-exchange failed ({status}): {text}"));
    }

    let body: ExchangeResp = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse API key token-exchange response: {e}"))?;

    if body.access_token.is_empty() {
        return Err("API key token-exchange returned empty access_token".to_string());
    }

    Ok(body.access_token)
}

/// Starts a temporary HTTP listener on the given port to handle the OAuth callback.
/// Returns a JoinHandle that resolves when the callback is received or timeout expires.
/// The callback exchanges the authorization code for tokens and returns them.
pub async fn start_callback_listener(
    port: u16,
    http_client: reqwest::Client,
) -> Result<tokio::task::JoinHandle<Option<OAuthTokens>>, String> {
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .map_err(|e| format!("Cannot bind callback listener on port {}: {}", port, e))?;

    tracing::info!(
        "OpenAI Codex OAuth: callback listener started on port {}",
        port
    );

    let handle = tokio::spawn(async move {
        let timeout = tokio::time::Duration::from_secs(SESSION_TTL_SECS as u64);
        let accept_result = tokio::time::timeout(timeout, listener.accept()).await;

        let (mut stream, _addr) = match accept_result {
            Ok(Ok((s, a))) => (s, a),
            Ok(Err(e)) => {
                tracing::warn!("OpenAI Codex OAuth: callback accept error: {}", e);
                return None;
            }
            Err(_) => {
                tracing::info!("OpenAI Codex OAuth: callback listener timed out");
                return None;
            }
        };

        use tokio::io::AsyncReadExt;

        let mut buf = vec![0u8; 8192];
        let n = match stream.read(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("OpenAI Codex OAuth: failed to read callback request: {}", e);
                return None;
            }
        };
        let request_str = String::from_utf8_lossy(&buf[..n]);

        // Parse the GET request line to extract path + query
        let first_line = request_str.lines().next().unwrap_or("");
        let path_and_query = first_line.split_whitespace().nth(1).unwrap_or("");

        let parsed = match url::Url::parse(&format!("http://localhost{}", path_and_query)) {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("OpenAI Codex OAuth: failed to parse callback URL: {}", e);
                send_http_response(&mut stream, 400, "Bad Request").await;
                return None;
            }
        };

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

        match exchange_code(&http_client, &session_id, &code).await {
            Ok(tokens) => {
                send_http_response(
                    &mut stream,
                    200,
                    &callback_html(
                        true,
                        "Authentication successful. You can close this window.",
                    ),
                )
                .await;
                Some(tokens)
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
        }
    });

    Ok(handle)
}

async fn send_http_response(stream: &mut tokio::net::TcpStream, status: u16, body: &str) {
    use tokio::io::AsyncWriteExt;
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status, reason, body.len(), body
    );
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
