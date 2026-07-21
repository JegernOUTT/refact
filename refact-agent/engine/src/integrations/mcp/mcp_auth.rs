use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AMutex;
use tracing::warn;
use uuid::Uuid;

use oauth2::{StandardTokenResponse, basic::BasicTokenType, TokenResponse};
use rmcp::transport::auth::{OAuthState, AuthorizationManager, VendorExtraTokenFields};
use crate::integrations::sessions::IntegrationSession;

pub fn deserialize_string_list<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<String>, D::Error> {
    use serde::de::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ScopesValue {
        List(Vec<String>),
        Str(String),
    }
    let value = ScopesValue::deserialize(d)?;
    match value {
        ScopesValue::List(v) => Ok(v),
        ScopesValue::Str(s) => {
            if s.is_empty() {
                Ok(vec![])
            } else {
                Ok(s.split(|c: char| c == ',' || c == ' ')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    #[default]
    None,
    Bearer,
    #[serde(alias = "oauth2")]
    Oauth2ClientCredentials,
    Oauth2Pkce,
}

#[derive(Deserialize, Serialize, Clone, Default, Debug, PartialEq)]
pub struct MCPAuthSettings {
    #[serde(default)]
    pub auth_type: AuthType,
    #[serde(default)]
    pub bearer_token: String,
    #[serde(default)]
    pub oauth2_client_id: String,
    #[serde(default)]
    pub oauth2_client_secret: String,
    #[serde(default)]
    pub oauth2_token_url: String,
    #[serde(default, deserialize_with = "deserialize_string_list")]
    pub oauth2_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_tokens: Option<MCPOAuthTokens>,
}

#[derive(Deserialize, Serialize, Clone, Default, Debug, PartialEq)]
pub struct MCPOAuthTokens {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// The MCP server URL these tokens were minted for. Tokens are refused
    /// when the config's URL changes so credentials for one server can never
    /// be replayed against another. Legacy configs without this field keep
    /// working and get bound on the next token refresh/login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_url: Option<String>,
}

pub async fn save_tokens_to_config(
    config_path: &str,
    tokens: &MCPOAuthTokens,
) -> Result<(), String> {
    let path = PathBuf::from(config_path);
    let existing = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read config {}: {}", config_path, e))?;
    let mut mapping: serde_yaml::Mapping = serde_yaml::from_str(&existing)
        .map_err(|e| format!("Failed to parse config YAML {}: {}", config_path, e))?;
    let tokens_value =
        serde_yaml::to_value(tokens).map_err(|e| format!("serialize tokens: {}", e))?;
    mapping.insert(
        serde_yaml::Value::String("oauth_tokens".to_string()),
        tokens_value,
    );
    let yaml_str = serde_yaml::to_string(&serde_yaml::Value::Mapping(mapping))
        .map_err(|e| format!("serialize yaml: {}", e))?;
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, &yaml_str)
        .await
        .map_err(|e| format!("write {:?}: {}", tmp, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).await;
    }
    #[cfg(target_os = "windows")]
    if path.exists() {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|e| format!("remove {:?}: {}", path, e))?;
    }
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| format!("rename {:?} -> {:?}: {}", tmp, path, e))?;
    Ok(())
}

pub async fn load_tokens_from_config(config_path: &str) -> Option<MCPOAuthTokens> {
    let content = tokio::fs::read_to_string(config_path).await.ok()?;
    let value: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    let tokens_value = value.get("oauth_tokens")?;
    serde_yaml::from_value(tokens_value.clone()).ok()
}

pub async fn clear_tokens_from_config(config_path: &str) -> Result<(), String> {
    let path = PathBuf::from(config_path);
    let existing = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read config {}: {}", config_path, e))?;
    let mut mapping: serde_yaml::Mapping = serde_yaml::from_str(&existing)
        .map_err(|e| format!("Failed to parse config YAML {}: {}", config_path, e))?;
    mapping.remove(serde_yaml::Value::String("oauth_tokens".to_string()));
    let yaml_str = serde_yaml::to_string(&serde_yaml::Value::Mapping(mapping))
        .map_err(|e| format!("serialize yaml: {}", e))?;
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, &yaml_str)
        .await
        .map_err(|e| format!("write {:?}: {}", tmp, e))?;
    #[cfg(target_os = "windows")]
    if path.exists() {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|e| format!("remove {:?}: {}", path, e))?;
    }
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| format!("rename {:?} -> {:?}: {}", tmp, path, e))?;
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WwwAuthenticateInfo {
    pub scheme: String,
    pub realm: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
    pub resource_metadata: Option<String>,
}

fn find_bearer_challenge(header_value: &str) -> Option<&str> {
    let lower = header_value.to_lowercase();
    let bytes = lower.as_bytes();
    let mut idx = 0;
    while let Some(pos) = lower[idx..].find("bearer") {
        let abs = idx + pos;
        let end = abs + "bearer".len();
        let before_ok = abs == 0 || !bytes[abs - 1].is_ascii_alphanumeric();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return Some(&header_value[abs..]);
        }
        idx = end;
    }
    None
}

pub fn parse_www_authenticate(header_value: &str) -> Option<WwwAuthenticateInfo> {
    let trimmed = header_value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (scheme, params_str) = match trimmed.split_once(char::is_whitespace) {
        Some((s, rest)) => (s.to_string(), rest.trim()),
        None => (trimmed.to_string(), ""),
    };
    let mut info = WwwAuthenticateInfo {
        scheme,
        ..Default::default()
    };

    let mut rest = params_str;
    while !rest.is_empty() {
        let eq = match rest.find('=') {
            Some(i) => i,
            None => break,
        };
        let key = rest[..eq]
            .trim_matches(|c: char| c == ',' || c.is_whitespace())
            .to_lowercase();
        rest = rest[eq + 1..].trim_start();
        let value: String;
        if rest.starts_with('"') {
            let mut out = String::new();
            let mut chars = rest[1..].char_indices();
            let mut consumed = rest.len();
            let mut escaped = false;
            for (i, c) in &mut chars {
                if escaped {
                    out.push(c);
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    consumed = i + 2;
                    break;
                } else {
                    out.push(c);
                }
            }
            value = out;
            rest = rest
                .get(consumed..)
                .unwrap_or("")
                .trim_start_matches(|c: char| c == ',' || c.is_whitespace());
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            value = rest[..end].trim().to_string();
            rest = rest.get(end + 1..).unwrap_or("").trim_start();
        }
        match key.as_str() {
            "realm" => info.realm = Some(value),
            "scope" => info.scope = Some(value),
            "error" => info.error = Some(value),
            "error_description" => info.error_description = Some(value),
            "resource_metadata" => info.resource_metadata = Some(value),
            _ => {}
        }
    }
    Some(info)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MCPAuthProbeResult {
    pub needs_auth: bool,
    pub oauth_available: bool,
    #[serde(default)]
    pub suggested_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub www_authenticate: Option<WwwAuthenticateInfo>,
    /// HTTP status of the probe request, for callers that need to distinguish
    /// a healthy server from a reachable-but-broken endpoint (404/500/...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

pub async fn probe_mcp_auth(
    url: &str,
    headers: &HashMap<String, String>,
    auth: &MCPAuthSettings,
) -> Result<MCPAuthProbeResult, String> {
    let mut effective_headers = headers.clone();
    if auth.auth_type == AuthType::Bearer && !auth.bearer_token.is_empty() {
        effective_headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", auth.bearer_token),
        );
    }
    let mut header_map = reqwest::header::HeaderMap::new();
    for (k, v) in &effective_headers {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            header_map.insert(name, value);
        }
    }
    header_map.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json, text/event-stream"),
    );
    header_map.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );

    let client = reqwest::Client::builder()
        .default_headers(header_map)
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("build probe client: {}", e))?;

    let initialize_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "refact-auth-probe", "version": "0.1"}
        }
    });
    let response = client
        .post(url)
        .json(&initialize_body)
        .send()
        .await
        .map_err(|e| format!("probe request to {}: {}", url, e))?;

    let status = response.status();
    let needs_auth =
        status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN;
    let mut www_authenticate: Option<WwwAuthenticateInfo> = None;
    for value in response
        .headers()
        .get_all(reqwest::header::WWW_AUTHENTICATE)
        .iter()
    {
        let Ok(value_str) = value.to_str() else {
            continue;
        };
        if let Some(bearer_part) = find_bearer_challenge(value_str) {
            if let Some(parsed) = parse_www_authenticate(bearer_part) {
                www_authenticate = Some(parsed);
                break;
            }
        }
    }
    if www_authenticate.is_none() {
        www_authenticate = response
            .headers()
            .get_all(reqwest::header::WWW_AUTHENTICATE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(parse_www_authenticate);
    }

    if !needs_auth {
        return Ok(MCPAuthProbeResult {
            needs_auth: false,
            oauth_available: false,
            suggested_scopes: vec![],
            www_authenticate,
            http_status: Some(status.as_u16()),
        });
    }

    let discovery = tokio::time::timeout(Duration::from_secs(10), async {
        match AuthorizationManager::new(url).await {
            Ok(manager) => match manager.discover_metadata().await {
                Ok(metadata) => {
                    let scopes = metadata.scopes_supported.clone().unwrap_or_default();
                    (true, scopes)
                }
                Err(_) => (false, vec![]),
            },
            Err(_) => (false, vec![]),
        }
    })
    .await;
    let (oauth_available, suggested_scopes) = discovery.unwrap_or((false, vec![]));
    let suggested_scopes = if suggested_scopes.is_empty() {
        www_authenticate
            .as_ref()
            .and_then(|w| w.scope.as_ref())
            .map(|s| {
                s.split_whitespace()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        suggested_scopes
    };

    Ok(MCPAuthProbeResult {
        needs_auth: true,
        oauth_available,
        suggested_scopes,
        www_authenticate,
        http_status: Some(status.as_u16()),
    })
}

pub async fn ensure_oauth2_pkce_auth_type(config_path: &str) -> Result<bool, String> {
    let path = PathBuf::from(config_path);
    let existing = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read config {}: {}", config_path, e))?;
    let mapping: serde_yaml::Mapping = serde_yaml::from_str(&existing)
        .map_err(|e| format!("Failed to parse config YAML {}: {}", config_path, e))?;
    let auth_type_key = serde_yaml::Value::String("auth_type".to_string());
    let current = mapping
        .get(&auth_type_key)
        .and_then(|v| v.as_str())
        .unwrap_or("none");
    if current == "oauth2_pkce" {
        return Ok(false);
    }

    // Line-level edit to preserve comments (e.g. marketplace markers) and formatting.
    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
    let mut replaced = false;
    for line in lines.iter_mut() {
        if line.starts_with("auth_type:") {
            *line = "auth_type: oauth2_pkce".to_string();
            replaced = true;
            break;
        }
    }
    if !replaced {
        lines.push("auth_type: oauth2_pkce".to_string());
    }
    let mut yaml_str = lines.join("\n");
    if existing.ends_with('\n') || !replaced {
        yaml_str.push('\n');
    }
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, &yaml_str)
        .await
        .map_err(|e| format!("write {:?}: {}", tmp, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).await;
    }
    #[cfg(target_os = "windows")]
    if path.exists() {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|e| format!("remove {:?}: {}", path, e))?;
    }
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| format!("rename {:?} -> {:?}: {}", tmp, path, e))?;
    Ok(true)
}

struct TokenState {
    access_token: String,
    expires_at: Option<Instant>,
}

pub struct MCPTokenManager {
    settings: MCPAuthSettings,
    token_cache: Arc<AMutex<Option<TokenState>>>,
}

impl MCPTokenManager {
    pub fn new(settings: MCPAuthSettings) -> Self {
        Self {
            settings,
            token_cache: Arc::new(AMutex::new(None)),
        }
    }

    pub async fn get_token(&self) -> Result<String, String> {
        match self.settings.auth_type {
            AuthType::None => Err("No auth configured".to_string()),
            AuthType::Bearer => {
                if self.settings.bearer_token.is_empty() {
                    return Err("Bearer token is empty".to_string());
                }
                Ok(self.settings.bearer_token.clone())
            }
            AuthType::Oauth2ClientCredentials => self.get_oauth2_token().await,
            AuthType::Oauth2Pkce => {
                if let Some(tokens) = &self.settings.oauth_tokens {
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64;
                    if tokens.expires_at > 0 && tokens.expires_at > now_ms + 30_000 {
                        return Ok(tokens.access_token.clone());
                    }
                }
                Err("OAuth2 PKCE token expired or not set; re-authentication required".to_string())
            }
        }
    }

    async fn get_oauth2_token(&self) -> Result<String, String> {
        {
            let cache = self.token_cache.lock().await;
            if let Some(state) = cache.as_ref() {
                // When expires_at is None (server omitted expires_in), treat as expired so
                // we always fetch a fresh token rather than caching indefinitely.
                let still_valid = state
                    .expires_at
                    .map_or(false, |exp| exp > Instant::now() + Duration::from_secs(30));
                if still_valid {
                    return Ok(state.access_token.clone());
                }
            }
        }

        if self.settings.oauth2_token_url.is_empty() {
            return Err("oauth2_token_url is empty".to_string());
        }
        if self.settings.oauth2_client_id.is_empty() {
            return Err("oauth2_client_id is empty".to_string());
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
        let mut params = vec![
            ("grant_type", "client_credentials".to_string()),
            ("client_id", self.settings.oauth2_client_id.clone()),
            ("client_secret", self.settings.oauth2_client_secret.clone()),
        ];
        if !self.settings.oauth2_scopes.is_empty() {
            params.push(("scope", self.settings.oauth2_scopes.join(" ")));
        }

        let resp = client
            .post(&self.settings.oauth2_token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("OAuth2 token request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "OAuth2 token endpoint returned HTTP {}",
                resp.status()
            ));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse OAuth2 response: {}", e))?;

        let access_token = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "OAuth2 response missing access_token".to_string())?
            .to_string();

        let expires_at = body
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .map(|secs| Instant::now() + Duration::from_secs(secs));

        {
            let mut cache = self.token_cache.lock().await;
            *cache = Some(TokenState {
                access_token: access_token.clone(),
                expires_at,
            });
        }

        Ok(access_token)
    }

    pub async fn apply_auth(&self, headers: &mut HashMap<String, String>) -> Result<(), String> {
        match self.settings.auth_type {
            AuthType::None => Ok(()),
            AuthType::Bearer | AuthType::Oauth2ClientCredentials | AuthType::Oauth2Pkce => {
                let token = self.get_token().await?;
                headers.insert("Authorization".to_string(), format!("Bearer {}", token));
                Ok(())
            }
        }
    }
}

fn reconstruct_token_response(
    tokens: &MCPOAuthTokens,
) -> Result<StandardTokenResponse<VendorExtraTokenFields, BasicTokenType>, String> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let mut token_json = serde_json::json!({
        "access_token": tokens.access_token,
        "token_type": "Bearer",
    });
    if !tokens.refresh_token.is_empty() {
        token_json["refresh_token"] = serde_json::Value::String(tokens.refresh_token.clone());
    }
    if tokens.expires_at > 0 {
        let remaining_ms = tokens.expires_at - now_ms;
        let expires_in_secs = if remaining_ms <= 0 {
            0i64
        } else {
            (remaining_ms + 999) / 1000
        };
        token_json["expires_in"] = serde_json::Value::Number(expires_in_secs.into());
    }
    serde_json::from_value(token_json)
        .map_err(|e| format!("Failed to reconstruct token response: {}", e))
}

pub async fn create_auth_manager_from_tokens(
    mcp_url: &str,
    tokens: &MCPOAuthTokens,
) -> Result<AuthorizationManager, String> {
    if let Some(bound) = tokens.bound_url.as_deref() {
        if bound != mcp_url {
            return Err(format!(
                "stored OAuth tokens are bound to '{}' but the config now points at '{}'; re-authentication required",
                bound, mcp_url
            ));
        }
    }
    let mut state = OAuthState::new(mcp_url, None)
        .await
        .map_err(|e| format!("create OAuth state: {}", e))?;
    let token_response = reconstruct_token_response(tokens)?;
    state
        .set_credentials(&tokens.client_id, token_response)
        .await
        .map_err(|e| format!("set OAuth credentials: {}", e))?;
    state
        .into_authorization_manager()
        .ok_or_else(|| "Failed to extract AuthorizationManager after set_credentials".to_string())
}

const REFRESH_BEFORE_EXPIRY_MS: i64 = 5 * 60 * 1000;

pub fn needs_refresh(tokens: &MCPOAuthTokens) -> bool {
    if tokens.expires_at <= 0 {
        return false;
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    tokens.expires_at - now_ms < REFRESH_BEFORE_EXPIRY_MS
}

fn tokens_from_response(
    client_id: String,
    old_refresh_token: &str,
    response: &StandardTokenResponse<VendorExtraTokenFields, BasicTokenType>,
    old_scopes: &[String],
    bound_url: Option<String>,
) -> MCPOAuthTokens {
    let access_token = response.access_token().secret().to_string();
    let refresh_token = response
        .refresh_token()
        .map(|r| r.secret().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| old_refresh_token.to_string());
    let expires_at = response
        .expires_in()
        .map(|d| {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            now_ms + d.as_millis() as i64
        })
        .unwrap_or(0);
    let response_scopes: Vec<String> = response
        .scopes()
        .map(|scopes| scopes.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();
    let scopes = if response_scopes.is_empty() {
        old_scopes.to_vec()
    } else {
        response_scopes
    };
    MCPOAuthTokens {
        access_token,
        refresh_token,
        expires_at,
        client_id,
        client_secret: None,
        scopes,
        bound_url,
    }
}

pub async fn mcp_oauth_refresh_task(
    session_arc: Arc<AMutex<Box<dyn IntegrationSession>>>,
    config_path: String,
) {
    use super::session_mcp::{SessionMCP, MCPAuthStatus};

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;

        let auth_manager_arc = {
            let mut session_locked = session_arc.lock().await;
            let mcp_session = match session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
                Some(s) => s,
                None => return,
            };
            mcp_session.auth_manager.clone()
        };

        let auth_manager_arc = match auth_manager_arc {
            Some(am) => am,
            // auth_manager not yet set (session still starting up); wait for next cycle
            None => continue,
        };

        let tokens = match load_tokens_from_config(&config_path).await {
            Some(t) if !t.access_token.is_empty() => t,
            _ => {
                warn!("OAuth refresh task: no tokens in config {}", config_path);
                let mut session_locked = session_arc.lock().await;
                if let Some(mcp_session) = session_locked.as_any_mut().downcast_mut::<SessionMCP>()
                {
                    mcp_session.auth_status = MCPAuthStatus::NeedsLogin;
                }
                // No tokens on disk — user must re-authenticate; keep looping in case
                // tokens appear later (e.g., user completes OAuth flow in another tab).
                continue;
            }
        };

        if !needs_refresh(&tokens) {
            continue;
        }

        {
            let mut session_locked = session_arc.lock().await;
            if let Some(mcp_session) = session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
                mcp_session.auth_status = MCPAuthStatus::Refreshing;
            }
        }

        let refresh_result = {
            let am = auth_manager_arc.lock().await;
            am.refresh_token().await
        };

        match refresh_result {
            Ok(token_response) => {
                let new_tokens = tokens_from_response(
                    tokens.client_id.clone(),
                    &tokens.refresh_token,
                    &token_response,
                    &tokens.scopes,
                    tokens.bound_url.clone(),
                );
                if let Err(e) = save_tokens_to_config(&config_path, &new_tokens).await {
                    warn!(
                        "OAuth refresh task: failed to persist tokens for {}: {}",
                        config_path, e
                    );
                }
                let mut session_locked = session_arc.lock().await;
                if let Some(mcp_session) = session_locked.as_any_mut().downcast_mut::<SessionMCP>()
                {
                    mcp_session.auth_status = MCPAuthStatus::Authenticated;
                }
            }
            Err(e) => {
                warn!("MCP OAuth refresh failed for {}: {}", config_path, e);
                let mut session_locked = session_arc.lock().await;
                if let Some(mcp_session) = session_locked.as_any_mut().downcast_mut::<SessionMCP>()
                {
                    mcp_session.auth_status = MCPAuthStatus::NeedsReauth;
                }
                // Keep looping — this may be a transient network error; next cycle will retry.
                // If the refresh token itself is invalid the server will keep returning errors,
                // but auth_status=NeedsReauth surfaces the problem to the user.
            }
        }
    }
}

struct PendingOAuthSession {
    oauth_state: Arc<AMutex<OAuthState>>,
    config_path: String,
    mcp_url: String,
    created_at: SystemTime,
    state_param: String,
    scopes: Vec<String>,
}

static PENDING_SESSIONS: OnceLock<AMutex<HashMap<String, PendingOAuthSession>>> = OnceLock::new();
static STATE_INDEX: OnceLock<AMutex<HashMap<String, String>>> = OnceLock::new();

fn pending_sessions() -> &'static AMutex<HashMap<String, PendingOAuthSession>> {
    PENDING_SESSIONS.get_or_init(|| AMutex::new(HashMap::new()))
}

fn state_index() -> &'static AMutex<HashMap<String, String>> {
    STATE_INDEX.get_or_init(|| AMutex::new(HashMap::new()))
}

fn extract_state_from_url(auth_url: &str) -> Result<String, String> {
    let parsed =
        url::Url::parse(auth_url).map_err(|_| "Failed to parse authorization URL".to_string())?;
    let state = parsed
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string())
        .ok_or_else(|| "Authorization URL missing state parameter".to_string())?;
    if state.is_empty() {
        return Err("Authorization URL has empty state parameter".to_string());
    }
    Ok(state)
}

pub struct MCPOAuthSessionManager;

impl MCPOAuthSessionManager {
    pub async fn start_oauth_flow(
        mcp_url: &str,
        config_path: &str,
        scopes: &[&str],
        redirect_uri: &str,
    ) -> Result<(String, String), String> {
        Self::cleanup_expired_sessions().await;

        let mut state = OAuthState::new(mcp_url, None)
            .await
            .map_err(|e| format!("create OAuth state: {}", e))?;
        state
            .start_authorization(scopes, redirect_uri, None)
            .await
            .map_err(|e| format!("start OAuth authorization: {}", e))?;
        let auth_url = state
            .get_authorization_url()
            .await
            .map_err(|e| format!("get authorization URL: {}", e))?;
        let state_param = extract_state_from_url(&auth_url)?;
        let session_id = Uuid::new_v4().to_string();
        pending_sessions().lock().await.insert(
            session_id.clone(),
            PendingOAuthSession {
                oauth_state: Arc::new(AMutex::new(state)),
                config_path: config_path.to_string(),
                mcp_url: mcp_url.to_string(),
                created_at: SystemTime::now(),
                state_param: state_param.clone(),
                scopes: scopes.iter().map(|s| s.to_string()).collect(),
            },
        );
        state_index()
            .lock()
            .await
            .insert(state_param, session_id.clone());
        Ok((session_id, auth_url))
    }

    pub async fn exchange_code(
        session_id: &str,
        code: &str,
    ) -> Result<(MCPOAuthTokens, String), String> {
        let (oauth_state_arc, config_path, mcp_url, state_param, old_scopes) = {
            let sessions = pending_sessions().lock().await;
            let session = sessions
                .get(session_id)
                .ok_or_else(|| format!("No pending OAuth session: {}", session_id))?;
            (
                session.oauth_state.clone(),
                session.config_path.clone(),
                session.mcp_url.clone(),
                session.state_param.clone(),
                session.scopes.clone(),
            )
        };

        let mut oauth_state = oauth_state_arc.lock().await;
        oauth_state
            .handle_callback(code, &state_param)
            .await
            .map_err(|e| format!("OAuth callback: {}", e))?;
        let (client_id, creds_opt) = oauth_state
            .get_credentials()
            .await
            .map_err(|e| format!("get OAuth credentials: {}", e))?;
        drop(oauth_state);

        let token_response =
            creds_opt.ok_or_else(|| "No credentials after callback".to_string())?;
        let token_json = serde_json::to_value(&token_response)
            .map_err(|e| format!("serialize token response: {}", e))?;
        let access_token = token_json
            .get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let refresh_token = token_json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let expires_at = token_json
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .map(|secs| {
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                now_ms + secs as i64 * 1000
            })
            .unwrap_or(0);

        pending_sessions().lock().await.remove(session_id);
        if !state_param.is_empty() {
            state_index().lock().await.remove(&state_param);
        }

        let scopes_from_response: Vec<String> = token_json
            .get("scope")
            .and_then(|v| v.as_str())
            .map(|s| {
                s.split_whitespace()
                    .map(|p| p.to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let scopes = if scopes_from_response.is_empty() {
            old_scopes
        } else {
            scopes_from_response
        };

        Ok((
            MCPOAuthTokens {
                access_token,
                refresh_token,
                expires_at,
                client_id,
                client_secret: None,
                scopes,
                bound_url: Some(mcp_url),
            },
            config_path,
        ))
    }

    pub async fn find_session_id_by_state(state: &str) -> Option<String> {
        state_index().lock().await.get(state).cloned()
    }

    pub async fn cleanup_expired_sessions() {
        let expiry = Duration::from_secs(600);
        let mut removed_states: Vec<String> = Vec::new();
        {
            let mut sessions = pending_sessions().lock().await;
            sessions.retain(|id, session| {
                let keep = session
                    .created_at
                    .elapsed()
                    .map(|age| age < expiry)
                    .unwrap_or(false);
                if !keep {
                    warn!("MCPOAuthSessionManager: removing expired session {}", id);
                    removed_states.push(session.state_param.clone());
                }
                keep
            });
        }
        let mut si = state_index().lock().await;
        for state in removed_states {
            si.remove(&state);
        }
    }

    pub async fn cancel_oauth_flow(session_id: &str) -> bool {
        let removed = pending_sessions().lock().await.remove(session_id);
        if let Some(session) = removed {
            if !session.state_param.is_empty() {
                state_index().lock().await.remove(&session.state_param);
            }
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[test]
    fn test_auth_settings_default() {
        let s: MCPAuthSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.auth_type, AuthType::None);
        assert!(s.bearer_token.is_empty());
    }

    #[test]
    fn test_auth_type_enum_roundtrip() {
        for (variant, expected_str) in [
            (AuthType::None, "\"none\""),
            (AuthType::Bearer, "\"bearer\""),
            (
                AuthType::Oauth2ClientCredentials,
                "\"oauth2_client_credentials\"",
            ),
            (AuthType::Oauth2Pkce, "\"oauth2_pkce\""),
        ] {
            let serialized = serde_json::to_string(&variant).unwrap();
            assert_eq!(serialized, expected_str);
            let deserialized: AuthType = serde_json::from_str(&serialized).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn test_auth_type_oauth2_alias() {
        let json = serde_json::json!({"auth_type": "oauth2"});
        let settings: MCPAuthSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.auth_type, AuthType::Oauth2ClientCredentials);
    }

    #[test]
    fn test_auth_type_backward_compat_oauth2_alias() {
        let json = serde_json::json!({"auth_type": "oauth2"});
        let settings: MCPAuthSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.auth_type, AuthType::Oauth2ClientCredentials);
    }

    #[test]
    fn test_auth_type_oauth2_client_credentials_unchanged() {
        let json = serde_json::json!({"auth_type": "oauth2_client_credentials"});
        let settings: MCPAuthSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.auth_type, AuthType::Oauth2ClientCredentials);
    }

    #[test]
    fn test_auth_type_oauth2_pkce_deserialized() {
        let json = serde_json::json!({"auth_type": "oauth2_pkce"});
        let settings: MCPAuthSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.auth_type, AuthType::Oauth2Pkce);
    }

    #[test]
    fn test_auth_settings_serialization_roundtrip() {
        let settings = MCPAuthSettings {
            auth_type: AuthType::Bearer,
            bearer_token: "tok123".to_string(),
            oauth2_client_id: "".to_string(),
            oauth2_client_secret: "".to_string(),
            oauth2_token_url: "".to_string(),
            oauth2_scopes: vec![],
            oauth_tokens: None,
        };
        let json = serde_json::to_value(&settings).unwrap();
        let roundtrip: MCPAuthSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings, roundtrip);
    }

    #[test]
    fn test_mcp_oauth_tokens_serialization_roundtrip_json() {
        let tokens = MCPOAuthTokens {
            access_token: "access_abc".to_string(),
            refresh_token: "refresh_xyz".to_string(),
            expires_at: 1700000000000,
            client_id: "client_123".to_string(),
            client_secret: Some("secret_456".to_string()),
            scopes: vec!["read".to_string(), "write".to_string()],
            bound_url: Some("https://mcp.example.com/mcp".to_string()),
        };
        let json = serde_json::to_value(&tokens).unwrap();
        let roundtrip: MCPOAuthTokens = serde_json::from_value(json).unwrap();
        assert_eq!(tokens, roundtrip);
    }

    #[test]
    fn test_mcp_oauth_tokens_serialization_roundtrip_yaml() {
        let tokens = MCPOAuthTokens {
            access_token: "access_abc".to_string(),
            refresh_token: "refresh_xyz".to_string(),
            expires_at: 1700000000000,
            client_id: "client_123".to_string(),
            client_secret: None,
            scopes: vec!["openid".to_string()],
            bound_url: None,
        };
        let yaml = serde_yaml::to_string(&tokens).unwrap();
        let roundtrip: MCPOAuthTokens = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(tokens, roundtrip);
    }

    #[tokio::test]
    async fn test_token_persistence_merge_with_existing_config() {
        let mut tmp = NamedTempFile::new().unwrap();
        let existing_yaml = "url: https://example.com/mcp\nauth_type: oauth2_pkce\n";
        tmp.write_all(existing_yaml.as_bytes()).unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let tokens = MCPOAuthTokens {
            access_token: "my_access_token".to_string(),
            refresh_token: "my_refresh_token".to_string(),
            expires_at: 1700000000000,
            client_id: "my_client".to_string(),
            client_secret: None,
            scopes: vec!["mcp".to_string()],
            bound_url: None,
        };

        save_tokens_to_config(&path, &tokens).await.unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            content.contains("url: https://example.com/mcp"),
            "original fields preserved"
        );
        assert!(
            content.contains("auth_type: oauth2_pkce"),
            "original fields preserved"
        );
        assert!(content.contains("oauth_tokens"), "oauth_tokens key added");
        assert!(content.contains("my_access_token"), "access token present");

        let loaded = load_tokens_from_config(&path).await.unwrap();
        assert_eq!(loaded.access_token, tokens.access_token);
        assert_eq!(loaded.refresh_token, tokens.refresh_token);
        assert_eq!(loaded.expires_at, tokens.expires_at);
        assert_eq!(loaded.client_id, tokens.client_id);
    }

    #[tokio::test]
    async fn test_token_persistence_overwrites_existing_tokens() {
        let mut tmp = NamedTempFile::new().unwrap();
        let existing_yaml = "url: https://example.com/mcp\noauth_tokens:\n  access_token: old_token\n  refresh_token: old_refresh\n  expires_at: 0\n  client_id: old_client\n";
        tmp.write_all(existing_yaml.as_bytes()).unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let new_tokens = MCPOAuthTokens {
            access_token: "new_access_token".to_string(),
            refresh_token: "new_refresh_token".to_string(),
            expires_at: 1800000000000,
            client_id: "new_client".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };

        save_tokens_to_config(&path, &new_tokens).await.unwrap();

        let loaded = load_tokens_from_config(&path).await.unwrap();
        assert_eq!(loaded.access_token, "new_access_token");
        assert_eq!(loaded.client_id, "new_client");
    }

    #[tokio::test]
    async fn test_pending_session_expiry_cleanup() {
        let old_id = format!("test-stale-{}", Uuid::new_v4());
        let fresh_id = format!("test-fresh-{}", Uuid::new_v4());

        let old_state = OAuthState::new("http://localhost", None).await.unwrap();
        {
            let mut sessions = pending_sessions().lock().await;
            sessions.insert(
                old_id.clone(),
                PendingOAuthSession {
                    oauth_state: Arc::new(AMutex::new(old_state)),
                    config_path: "/tmp/test.yaml".to_string(),
                    mcp_url: "http://127.0.0.1:1/mcp".to_string(),
                    created_at: SystemTime::now() - Duration::from_secs(700),
                    state_param: String::new(),
                    scopes: vec![],
                },
            );
        }

        let fresh_state = OAuthState::new("http://localhost", None).await.unwrap();
        {
            let mut sessions = pending_sessions().lock().await;
            sessions.insert(
                fresh_id.clone(),
                PendingOAuthSession {
                    oauth_state: Arc::new(AMutex::new(fresh_state)),
                    config_path: "/tmp/test.yaml".to_string(),
                    mcp_url: "http://127.0.0.1:1/mcp".to_string(),
                    created_at: SystemTime::now(),
                    state_param: String::new(),
                    scopes: vec![],
                },
            );
        }

        MCPOAuthSessionManager::cleanup_expired_sessions().await;

        {
            let sessions = pending_sessions().lock().await;
            assert!(
                !sessions.contains_key(&old_id),
                "stale session should be removed"
            );
            assert!(
                sessions.contains_key(&fresh_id),
                "fresh session should remain"
            );
        }

        pending_sessions().lock().await.remove(&fresh_id);
    }

    #[tokio::test]
    async fn test_bearer_token_injection() {
        let settings = MCPAuthSettings {
            auth_type: AuthType::Bearer,
            bearer_token: "my-secret-token".to_string(),
            ..Default::default()
        };
        let manager = MCPTokenManager::new(settings);
        let mut headers = HashMap::new();
        manager.apply_auth(&mut headers).await.unwrap();
        assert_eq!(
            headers.get("Authorization").unwrap(),
            "Bearer my-secret-token"
        );
    }

    #[tokio::test]
    async fn test_none_auth_does_not_inject_headers() {
        let settings = MCPAuthSettings {
            auth_type: AuthType::None,
            ..Default::default()
        };
        let manager = MCPTokenManager::new(settings);
        let mut headers = HashMap::new();
        let result = manager.apply_auth(&mut headers).await;
        assert!(result.is_ok());
        assert!(headers.is_empty());
    }

    #[tokio::test]
    async fn test_bearer_empty_token_returns_error() {
        let settings = MCPAuthSettings {
            auth_type: AuthType::Bearer,
            bearer_token: "".to_string(),
            ..Default::default()
        };
        let manager = MCPTokenManager::new(settings);
        let mut headers = HashMap::new();
        let result = manager.apply_auth(&mut headers).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Bearer token is empty"));
    }

    #[tokio::test]
    async fn test_oauth2_client_credentials_missing_token_url_returns_error() {
        let settings = MCPAuthSettings {
            auth_type: AuthType::Oauth2ClientCredentials,
            oauth2_client_id: "client123".to_string(),
            oauth2_token_url: "".to_string(),
            ..Default::default()
        };
        let manager = MCPTokenManager::new(settings);
        let mut headers = HashMap::new();
        let result = manager.apply_auth(&mut headers).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("oauth2_token_url is empty"));
    }

    #[tokio::test]
    async fn test_oauth2_client_credentials_missing_client_id_returns_error() {
        let settings = MCPAuthSettings {
            auth_type: AuthType::Oauth2ClientCredentials,
            oauth2_client_id: "".to_string(),
            oauth2_token_url: "https://example.com/token".to_string(),
            ..Default::default()
        };
        let manager = MCPTokenManager::new(settings);
        let mut headers = HashMap::new();
        let result = manager.apply_auth(&mut headers).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("oauth2_client_id is empty"));
    }

    #[test]
    fn test_reconstruct_token_response_access_token() {
        use oauth2::TokenResponse;
        let tokens = MCPOAuthTokens {
            access_token: "access_abc123".to_string(),
            refresh_token: "refresh_xyz".to_string(),
            expires_at: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64)
                + 3_600_000,
            client_id: "client_id_1".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };
        let response = reconstruct_token_response(&tokens).unwrap();
        assert_eq!(response.access_token().secret(), "access_abc123");
    }

    #[test]
    fn test_reconstruct_token_response_expired_has_zero_expires_in() {
        use oauth2::TokenResponse;
        let tokens = MCPOAuthTokens {
            access_token: "access_expired".to_string(),
            refresh_token: "refresh_xyz".to_string(),
            expires_at: 1_000_000,
            client_id: "client_id_1".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };
        let response = reconstruct_token_response(&tokens).unwrap();
        assert_eq!(response.access_token().secret(), "access_expired");
        let expires_in = response
            .expires_in()
            .expect("expires_in should be present for expired token");
        assert_eq!(expires_in.as_secs(), 0);
    }

    #[test]
    fn test_reconstruct_token_response_no_refresh() {
        use oauth2::TokenResponse;
        let tokens = MCPOAuthTokens {
            access_token: "access_only".to_string(),
            refresh_token: "".to_string(),
            expires_at: 0,
            client_id: "client".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };
        let response = reconstruct_token_response(&tokens).unwrap();
        assert_eq!(response.access_token().secret(), "access_only");
        assert!(response.refresh_token().is_none());
    }

    #[tokio::test]
    async fn test_no_tokens_in_config_returns_none() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let result = load_tokens_from_config(&path).await;
        assert!(
            result.is_none(),
            "Empty config should return None for tokens"
        );
    }

    #[tokio::test]
    async fn test_persisted_tokens_loadable_for_reconstruction() {
        use std::io::Write;
        use oauth2::TokenResponse;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        let yaml = "auth_type: oauth2_pkce\n";
        tmp.write_all(yaml.as_bytes()).unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let tokens = MCPOAuthTokens {
            access_token: "test_access".to_string(),
            refresh_token: "test_refresh".to_string(),
            expires_at: (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64)
                + 3_600_000,
            client_id: "client_123".to_string(),
            client_secret: None,
            scopes: vec!["mcp".to_string()],
            bound_url: None,
        };
        save_tokens_to_config(&path, &tokens).await.unwrap();

        let loaded = load_tokens_from_config(&path).await.unwrap();
        assert_eq!(loaded.access_token, "test_access");

        let response = reconstruct_token_response(&loaded).unwrap();
        assert_eq!(response.access_token().secret(), "test_access");
    }

    #[test]
    fn test_needs_refresh_no_expiry() {
        let tokens = MCPOAuthTokens {
            access_token: "tok".to_string(),
            refresh_token: "ref".to_string(),
            expires_at: 0,
            client_id: "client".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };
        assert!(
            !needs_refresh(&tokens),
            "No expiry (0) should not trigger refresh"
        );
    }

    #[test]
    fn test_needs_refresh_expires_soon() {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let tokens = MCPOAuthTokens {
            access_token: "tok".to_string(),
            refresh_token: "ref".to_string(),
            expires_at: now_ms + 2 * 60 * 1000,
            client_id: "client".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };
        assert!(
            needs_refresh(&tokens),
            "Expiry in 2 minutes should trigger refresh"
        );
    }

    #[test]
    fn test_needs_refresh_expires_later() {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let tokens = MCPOAuthTokens {
            access_token: "tok".to_string(),
            refresh_token: "ref".to_string(),
            expires_at: now_ms + 60 * 60 * 1000,
            client_id: "client".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };
        assert!(
            !needs_refresh(&tokens),
            "Expiry in 1 hour should not trigger refresh"
        );
    }

    #[test]
    fn test_needs_refresh_already_expired() {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let tokens = MCPOAuthTokens {
            access_token: "tok".to_string(),
            refresh_token: "ref".to_string(),
            expires_at: now_ms - 1000,
            client_id: "client".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };
        assert!(
            needs_refresh(&tokens),
            "Already expired token should trigger refresh"
        );
    }

    #[test]
    fn test_tokens_from_response_fallback_refresh_token() {
        use oauth2::TokenResponse;
        let tokens = MCPOAuthTokens {
            access_token: "old_access".to_string(),
            refresh_token: "old_refresh".to_string(),
            expires_at: 0,
            client_id: "client".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };
        let response = reconstruct_token_response(&tokens).unwrap();
        let new_tokens = tokens_from_response(
            "client".to_string(),
            "old_refresh",
            &response,
            &[],
            Some("https://mcp.example.com/mcp".to_string()),
        );
        assert_eq!(
            new_tokens.bound_url.as_deref(),
            Some("https://mcp.example.com/mcp"),
            "refresh must preserve the URL binding"
        );
        assert_eq!(new_tokens.access_token, "old_access");
        assert_eq!(
            new_tokens.refresh_token, "old_refresh",
            "Should fall back to old refresh token"
        );
        assert_eq!(response.access_token().secret(), "old_access");
    }

    #[test]
    fn test_unknown_auth_type_fails_deserialization() {
        let json = serde_json::json!({"auth_type": "digest"});
        let result: Result<MCPAuthSettings, _> = serde_json::from_value(json);
        assert!(
            result.is_err(),
            "Unknown auth_type string should fail deserialization"
        );
    }

    #[test]
    fn test_start_flow_empty_state_rejected() {
        let url_no_state =
            "https://example.com/authorize?code_challenge=abc&code_challenge_method=S256";
        assert!(
            extract_state_from_url(url_no_state).is_err(),
            "URL missing state should fail"
        );

        let url_empty_state = "https://example.com/authorize?state=&code_challenge=abc";
        assert!(
            extract_state_from_url(url_empty_state).is_err(),
            "URL with empty state should fail"
        );

        let url_with_state = "https://example.com/authorize?state=abc123&code_challenge=xyz";
        let result = extract_state_from_url(url_with_state);
        assert!(result.is_ok(), "URL with valid state should succeed");
        assert_eq!(result.unwrap(), "abc123");
    }

    #[tokio::test]
    async fn test_find_session_by_state_o1() {
        let session_id = format!("test-state-o1-{}", Uuid::new_v4());
        let state_val = format!("test-state-{}", Uuid::new_v4());

        state_index()
            .lock()
            .await
            .insert(state_val.clone(), session_id.clone());

        let found = MCPOAuthSessionManager::find_session_id_by_state(&state_val).await;
        assert_eq!(found, Some(session_id.clone()));

        let not_found =
            MCPOAuthSessionManager::find_session_id_by_state("nonexistent_state_xyz_unique").await;
        assert!(not_found.is_none());

        state_index().lock().await.remove(&state_val);
    }

    #[tokio::test]
    async fn test_cleanup_called_on_start() {
        let stale_id = format!("test-cleanup-stale-{}", Uuid::new_v4());
        let stale_state = format!("test-state-stale-{}", Uuid::new_v4());

        let old_state = OAuthState::new("http://localhost", None).await.unwrap();
        {
            let mut sessions = pending_sessions().lock().await;
            sessions.insert(
                stale_id.clone(),
                PendingOAuthSession {
                    oauth_state: Arc::new(AMutex::new(old_state)),
                    config_path: "/tmp/test.yaml".to_string(),
                    mcp_url: "http://127.0.0.1:1/mcp".to_string(),
                    created_at: SystemTime::now() - Duration::from_secs(700),
                    state_param: stale_state.clone(),
                    scopes: vec![],
                },
            );
        }
        state_index()
            .lock()
            .await
            .insert(stale_state.clone(), stale_id.clone());

        MCPOAuthSessionManager::cleanup_expired_sessions().await;

        assert!(
            !pending_sessions().lock().await.contains_key(&stale_id),
            "stale session should be removed by cleanup"
        );
        assert!(
            !state_index().lock().await.contains_key(&stale_state),
            "stale state should be removed from state_index by cleanup"
        );
    }

    #[tokio::test]
    async fn test_save_tokens_fails_on_invalid_yaml() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"{{{{invalid yaml").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let original_content = std::fs::read_to_string(&path).unwrap();

        let tokens = MCPOAuthTokens {
            access_token: "tok".to_string(),
            ..Default::default()
        };
        let result = save_tokens_to_config(&path, &tokens).await;
        assert!(result.is_err(), "Should fail on invalid YAML");

        let after_content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            original_content, after_content,
            "File should be unchanged on error"
        );
    }

    #[tokio::test]
    async fn test_clear_tokens_fails_on_nonexistent_file() {
        let result =
            clear_tokens_from_config("/tmp/nonexistent_mcp_test_file_xyz_12345.yaml").await;
        assert!(result.is_err(), "Should fail on nonexistent file");
    }

    #[test]
    fn test_scopes_deserialize_from_string() {
        let json = serde_json::json!({"auth_type": "bearer", "oauth2_scopes": "read write"});
        let settings: MCPAuthSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.oauth2_scopes, vec!["read", "write"]);
    }

    #[test]
    fn test_scopes_deserialize_from_array() {
        let json = serde_json::json!({"auth_type": "bearer", "oauth2_scopes": ["read", "write"]});
        let settings: MCPAuthSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.oauth2_scopes, vec!["read", "write"]);
    }

    #[test]
    fn test_scopes_deserialize_empty_string() {
        let json = serde_json::json!({"auth_type": "bearer", "oauth2_scopes": ""});
        let settings: MCPAuthSettings = serde_json::from_value(json).unwrap();
        assert!(settings.oauth2_scopes.is_empty());
    }

    #[test]
    fn test_reconstruct_near_expiry_produces_expires_in() {
        use oauth2::TokenResponse;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let tokens = MCPOAuthTokens {
            access_token: "tok".to_string(),
            refresh_token: String::new(),
            expires_at: now_ms + 500,
            client_id: "client".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };
        let response = reconstruct_token_response(&tokens).unwrap();
        let expires_in = response
            .expires_in()
            .expect("expires_in should be present for near-expiry token");
        assert_eq!(
            expires_in.as_secs(),
            1,
            "500ms remaining should ceil to 1 second"
        );
    }

    #[test]
    fn test_reconstruct_expired_produces_zero_expires_in() {
        use oauth2::TokenResponse;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let tokens = MCPOAuthTokens {
            access_token: "tok".to_string(),
            refresh_token: String::new(),
            expires_at: now_ms - 1,
            client_id: "client".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };
        let response = reconstruct_token_response(&tokens).unwrap();
        let expires_in = response
            .expires_in()
            .expect("expires_in should be present for expired token");
        assert_eq!(
            expires_in.as_secs(),
            0,
            "expired token should have expires_in = 0"
        );
    }

    #[test]
    fn test_reconstruct_no_expiry_omits_expires_in() {
        use oauth2::TokenResponse;
        let tokens = MCPOAuthTokens {
            access_token: "tok".to_string(),
            refresh_token: String::new(),
            expires_at: 0,
            client_id: "client".to_string(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        };
        let response = reconstruct_token_response(&tokens).unwrap();
        assert!(
            response.expires_in().is_none(),
            "non-expiring token (expires_at=0) should omit expires_in"
        );
    }

    #[tokio::test]
    async fn test_cancel_removes_session() {
        let session_id = format!("test-cancel-{}", Uuid::new_v4());
        let state_val = format!("test-cancel-state-{}", Uuid::new_v4());

        let oauth_state = OAuthState::new("http://localhost", None).await.unwrap();
        pending_sessions().lock().await.insert(
            session_id.clone(),
            PendingOAuthSession {
                oauth_state: Arc::new(AMutex::new(oauth_state)),
                config_path: "/tmp/test.yaml".to_string(),
                mcp_url: "http://127.0.0.1:1/mcp".to_string(),
                created_at: SystemTime::now(),
                state_param: state_val.clone(),
                scopes: vec![],
            },
        );
        state_index()
            .lock()
            .await
            .insert(state_val.clone(), session_id.clone());

        let removed = MCPOAuthSessionManager::cancel_oauth_flow(&session_id).await;
        assert!(removed, "cancel should return true for existing session");

        assert!(
            !pending_sessions().lock().await.contains_key(&session_id),
            "session should be removed after cancel"
        );
        assert!(
            !state_index().lock().await.contains_key(&state_val),
            "state index should be cleaned up after cancel"
        );

        let double_cancel = MCPOAuthSessionManager::cancel_oauth_flow(&session_id).await;
        assert!(
            !double_cancel,
            "cancel of already-removed session should return false"
        );
    }

    #[tokio::test]
    async fn test_exchange_failure_keeps_session() {
        let session_id = format!("test-exchange-fail-{}", Uuid::new_v4());
        let state_val = format!("test-efk-state-{}", Uuid::new_v4());

        let oauth_state = OAuthState::new("http://localhost", None).await.unwrap();
        pending_sessions().lock().await.insert(
            session_id.clone(),
            PendingOAuthSession {
                oauth_state: Arc::new(AMutex::new(oauth_state)),
                config_path: "/tmp/test.yaml".to_string(),
                mcp_url: "http://127.0.0.1:1/mcp".to_string(),
                created_at: SystemTime::now(),
                state_param: state_val.clone(),
                scopes: vec![],
            },
        );
        state_index()
            .lock()
            .await
            .insert(state_val.clone(), session_id.clone());

        let result = MCPOAuthSessionManager::exchange_code(&session_id, "fake_code").await;
        assert!(
            result.is_err(),
            "exchange with uninitialized OAuth state should fail"
        );

        assert!(
            pending_sessions().lock().await.contains_key(&session_id),
            "session should remain after failed exchange (for retry)"
        );

        MCPOAuthSessionManager::cancel_oauth_flow(&session_id).await;
    }

    #[test]
    fn test_parse_www_authenticate_bearer_with_params() {
        let info = parse_www_authenticate(
            r#"Bearer realm="mcp", error="insufficient_scope", scope="read write", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource""#,
        )
        .unwrap();
        assert_eq!(info.scheme, "Bearer");
        assert_eq!(info.realm.as_deref(), Some("mcp"));
        assert_eq!(info.error.as_deref(), Some("insufficient_scope"));
        assert_eq!(info.scope.as_deref(), Some("read write"));
        assert_eq!(
            info.resource_metadata.as_deref(),
            Some("https://mcp.example.com/.well-known/oauth-protected-resource")
        );
    }

    #[test]
    fn test_parse_www_authenticate_unquoted_and_bare() {
        let info = parse_www_authenticate("Bearer realm=api, scope=mcp").unwrap();
        assert_eq!(info.scheme, "Bearer");
        assert_eq!(info.realm.as_deref(), Some("api"));
        assert_eq!(info.scope.as_deref(), Some("mcp"));

        let bare = parse_www_authenticate("Bearer").unwrap();
        assert_eq!(bare.scheme, "Bearer");
        assert!(bare.realm.is_none());
        assert!(bare.scope.is_none());
    }

    #[test]
    fn test_parse_www_authenticate_escaped_quotes_and_garbage() {
        let info = parse_www_authenticate(r#"Bearer realm="a \"b\" c""#).unwrap();
        assert_eq!(info.realm.as_deref(), Some(r#"a "b" c"#));

        assert!(parse_www_authenticate("").is_none());
        assert!(parse_www_authenticate("   ").is_none());

        let basic = parse_www_authenticate(r#"Basic realm="host""#).unwrap();
        assert_eq!(basic.scheme, "Basic");
        assert_eq!(basic.realm.as_deref(), Some("host"));
    }

    #[tokio::test]
    async fn test_ensure_oauth2_pkce_auth_type_upgrades_and_preserves() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "url: https://mcp.example.com/mcp\nauth_type: none\nbearer_token: \"\"\ninit_timeout: '60'"
        )
        .unwrap();
        let path = file.path().to_string_lossy().to_string();

        let changed = ensure_oauth2_pkce_auth_type(&path).await.unwrap();
        assert!(changed, "should report a change from none to oauth2_pkce");

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let value: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert_eq!(
            value.get("auth_type").and_then(|v| v.as_str()),
            Some("oauth2_pkce")
        );
        assert_eq!(
            value.get("url").and_then(|v| v.as_str()),
            Some("https://mcp.example.com/mcp"),
            "other fields must be preserved"
        );
        assert_eq!(
            value.get("init_timeout").and_then(|v| v.as_str()),
            Some("60")
        );

        let changed_again = ensure_oauth2_pkce_auth_type(&path).await.unwrap();
        assert!(!changed_again, "already oauth2_pkce; no rewrite");
    }

    async fn spawn_probe_server(
        status: axum::http::StatusCode,
        www_authenticate: &'static [&'static str],
        with_discovery: bool,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::routing::{get, post};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{}", addr);
        let base_for_meta = base.clone();

        let mcp = post(move || async move {
            let mut headers = axum::http::HeaderMap::new();
            for w in www_authenticate {
                headers.append("WWW-Authenticate", axum::http::HeaderValue::from_static(w));
            }
            (status, headers, "{}")
        });
        let mut app = axum::Router::new().route("/mcp", mcp);
        if with_discovery {
            app = app.route(
                "/.well-known/oauth-authorization-server",
                get(move || {
                    let base = base_for_meta.clone();
                    async move {
                        axum::Json(serde_json::json!({
                            "issuer": base,
                            "authorization_endpoint": format!("{}/authorize", base),
                            "token_endpoint": format!("{}/token", base),
                            "scopes_supported": ["mcp.read", "mcp.write"],
                            "response_types_supported": ["code"],
                            "code_challenge_methods_supported": ["S256"]
                        }))
                    }
                }),
            );
        }

        listener.set_nonblocking(true).unwrap();
        let handle = tokio::spawn(async move {
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(app.into_make_service())
                .await
                .unwrap();
        });
        (format!("{}/mcp", base), handle)
    }

    #[tokio::test]
    async fn test_probe_mcp_auth_200_no_auth_needed() {
        let (url, handle) = spawn_probe_server(axum::http::StatusCode::OK, &[], false).await;
        let result = probe_mcp_auth(&url, &HashMap::new(), &MCPAuthSettings::default())
            .await
            .unwrap();
        assert!(!result.needs_auth);
        assert!(!result.oauth_available);
        handle.abort();
    }

    #[tokio::test]
    async fn test_probe_mcp_auth_401_without_discovery() {
        let (url, handle) = spawn_probe_server(
            axum::http::StatusCode::UNAUTHORIZED,
            &[r#"Bearer realm="mcp", scope="tools.read""#],
            false,
        )
        .await;
        let result = probe_mcp_auth(&url, &HashMap::new(), &MCPAuthSettings::default())
            .await
            .unwrap();
        assert!(result.needs_auth);
        assert!(!result.oauth_available);
        assert_eq!(result.suggested_scopes, vec!["tools.read".to_string()]);
        let www = result.www_authenticate.unwrap();
        assert_eq!(www.scheme, "Bearer");
        assert_eq!(www.scope.as_deref(), Some("tools.read"));
        handle.abort();
    }

    #[tokio::test]
    async fn test_probe_mcp_auth_401_with_discovery() {
        let (url, handle) =
            spawn_probe_server(axum::http::StatusCode::UNAUTHORIZED, &["Bearer"], true).await;
        let result = probe_mcp_auth(&url, &HashMap::new(), &MCPAuthSettings::default())
            .await
            .unwrap();
        assert!(result.needs_auth);
        assert!(
            result.oauth_available,
            "discovery metadata served; oauth should be available"
        );
        assert_eq!(
            result.suggested_scopes,
            vec!["mcp.read".to_string(), "mcp.write".to_string()]
        );
        handle.abort();
    }

    #[tokio::test]
    async fn test_probe_mcp_auth_forbidden_counts_as_needs_auth() {
        let (url, handle) = spawn_probe_server(
            axum::http::StatusCode::FORBIDDEN,
            &[r#"Bearer error="insufficient_scope", scope="a b""#],
            false,
        )
        .await;
        let result = probe_mcp_auth(&url, &HashMap::new(), &MCPAuthSettings::default())
            .await
            .unwrap();
        assert!(result.needs_auth);
        assert_eq!(
            result.suggested_scopes,
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(result
            .www_authenticate
            .as_ref()
            .map(|w| w.error.as_deref() == Some("insufficient_scope"))
            .unwrap_or(false));
        handle.abort();
    }

    #[test]
    fn test_find_bearer_challenge_multi_scheme() {
        let multi = r#"Basic realm="host", Bearer scope="mcp.read", error="invalid_token""#;
        let bearer = find_bearer_challenge(multi).unwrap();
        assert!(bearer.starts_with("Bearer"));
        let info = parse_www_authenticate(bearer).unwrap();
        assert_eq!(info.scheme, "Bearer");
        assert_eq!(info.scope.as_deref(), Some("mcp.read"));
        assert_eq!(info.error.as_deref(), Some("invalid_token"));

        assert!(find_bearer_challenge(r#"Basic realm="host""#).is_none());
        assert!(
            find_bearer_challenge("bearer scope=x").is_some(),
            "case-insensitive"
        );
        assert!(
            find_bearer_challenge("Bearerlike realm=x").is_none(),
            "no match inside longer token"
        );
    }

    #[tokio::test]
    async fn test_probe_mcp_auth_multiple_www_authenticate_headers() {
        let (url, handle) = spawn_probe_server(
            axum::http::StatusCode::UNAUTHORIZED,
            &[r#"Basic realm="host""#, r#"Bearer scope="multi.scope""#],
            false,
        )
        .await;
        let result = probe_mcp_auth(&url, &HashMap::new(), &MCPAuthSettings::default())
            .await
            .unwrap();
        assert!(result.needs_auth);
        let www = result.www_authenticate.unwrap();
        assert_eq!(www.scheme, "Bearer", "Bearer challenge must win over Basic");
        assert_eq!(result.suggested_scopes, vec!["multi.scope".to_string()]);
        handle.abort();
    }

    #[tokio::test]
    async fn test_ensure_oauth2_pkce_auth_type_preserves_comments() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "# mcp_marketplace_source: community\n# mcp_marketplace_server: github\nurl: https://mcp.example.com/mcp\nauth_type: none\nheaders:\n  Accept: application/json"
        )
        .unwrap();
        let path = file.path().to_string_lossy().to_string();

        let changed = ensure_oauth2_pkce_auth_type(&path).await.unwrap();
        assert!(changed);

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            content.contains("# mcp_marketplace_source: community"),
            "marketplace source comment must survive the rewrite, got: {}",
            content
        );
        assert!(content.contains("# mcp_marketplace_server: github"));
        assert!(content.contains("auth_type: oauth2_pkce"));
        assert!(!content.contains("auth_type: none"));
        assert!(
            content.contains("  Accept: application/json"),
            "nested formatting preserved"
        );
    }

    #[tokio::test]
    async fn test_ensure_oauth2_pkce_auth_type_appends_when_missing() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "# header comment\nurl: https://mcp.example.com/mcp").unwrap();
        let path = file.path().to_string_lossy().to_string();

        let changed = ensure_oauth2_pkce_auth_type(&path).await.unwrap();
        assert!(changed);

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("# header comment"));
        assert!(content.contains("auth_type: oauth2_pkce"));
        let value: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert_eq!(
            value.get("auth_type").and_then(|v| v.as_str()),
            Some("oauth2_pkce")
        );
    }

    #[test]
    fn test_remote_mcp_schemas_keep_manual_auth_in_advanced_configuration() {
        for schema in [
            include_str!("mcp_http_schema.yaml"),
            include_str!("mcp_sse_schema.yaml"),
        ] {
            let value: serde_yaml::Value = serde_yaml::from_str(schema).unwrap();
            let fields = value.get("fields").and_then(|v| v.as_mapping()).unwrap();

            for (field_name, field) in fields {
                let field_name = field_name.as_str().unwrap();
                let field = field.as_mapping().unwrap();
                let is_advanced = field
                    .get(serde_yaml::Value::String("f_extra".to_string()))
                    .is_some();
                assert_eq!(
                    is_advanced,
                    field_name != "url",
                    "only url should be visible before advanced configuration is opened"
                );
            }
        }
    }
}
