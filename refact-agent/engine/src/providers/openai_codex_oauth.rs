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
    refresh_token: String,
    #[allow(dead_code)]
    id_token: Option<serde_json::Value>,
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
    let codex_home = codex_home_dir()
        .ok_or("Cannot determine Codex home directory")?;

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

    let tokens = creds.tokens
        .ok_or_else(|| "No OAuth tokens in Codex CLI credentials. Run 'codex login' (not API key mode).".to_string())?;

    if tokens.access_token.is_empty() {
        return Err("Empty access token in Codex CLI credentials".to_string());
    }

    Ok(OAuthTokens {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: 0,
    })
}

pub fn codex_cli_credentials_exist() -> bool {
    codex_home_dir()
        .map(|h| h.join("auth.json").exists())
        .unwrap_or(false)
}

fn build_authorize_url(code_challenge: &str, state: &str, redirect_uri: &str) -> String {
    let mut url = url::Url::parse(AUTHORIZE_URL).expect("valid base URL");

    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state);

    url.to_string()
}

async fn prune_expired_sessions(sessions: &mut HashMap<String, PkceSession>) {
    let now = chrono::Utc::now().timestamp();
    sessions.retain(|_, s| now - s.created_at < SESSION_TTL_SECS);
}

pub async fn start_oauth_session(callback_port: u16) -> (String, String) {
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);
    let session_id = uuid::Uuid::new_v4().to_string();
    let redirect_uri = format!("http://localhost:{}/v1/providers/openai_codex/oauth/callback", callback_port);
    let authorize_url = build_authorize_url(&challenge, &session_id, &redirect_uri);

    let session = PkceSession {
        verifier,
        redirect_uri,
        created_at: chrono::Utc::now().timestamp(),
    };

    let mut sessions = PENDING_SESSIONS.lock().await;
    prune_expired_sessions(&mut sessions).await;
    sessions.insert(session_id.clone(), session);

    (session_id, authorize_url)
}

pub async fn exchange_code(
    http_client: &reqwest::Client,
    session_id: &str,
    code: &str,
) -> Result<OAuthTokens, String> {
    let session = {
        let mut sessions = PENDING_SESSIONS.lock().await;
        sessions.remove(session_id)
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

    Ok(OAuthTokens {
        access_token: token_resp.access_token,
        refresh_token: token_resp.refresh_token,
        expires_at,
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
    })
}
