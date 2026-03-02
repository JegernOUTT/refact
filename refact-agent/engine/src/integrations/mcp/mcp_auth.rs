use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AMutex;
use tracing::warn;
use uuid::Uuid;

use oauth2::{StandardTokenResponse, EmptyExtraTokenFields, basic::BasicTokenType};
use rmcp::transport::auth::{OAuthState, AuthorizationManager};

fn default_auth_type() -> String {
    "none".to_string()
}

fn deserialize_auth_type<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let s = String::deserialize(d)?;
    if s.as_str() == "oauth2" {
        Ok("oauth2_client_credentials".to_string())
    } else {
        Ok(s)
    }
}

#[derive(Deserialize, Serialize, Clone, Default, Debug, PartialEq)]
pub struct MCPAuthSettings {
    #[serde(default = "default_auth_type", deserialize_with = "deserialize_auth_type")]
    pub auth_type: String,
    #[serde(default)]
    pub bearer_token: String,
    #[serde(default)]
    pub oauth2_client_id: String,
    #[serde(default)]
    pub oauth2_client_secret: String,
    #[serde(default)]
    pub oauth2_token_url: String,
    #[serde(default)]
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
}

pub async fn save_tokens_to_config(config_path: &str, tokens: &MCPOAuthTokens) -> Result<(), String> {
    let path = PathBuf::from(config_path);
    let existing = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    let mut mapping: serde_yaml::Mapping = serde_yaml::from_str(&existing).unwrap_or_default();
    let tokens_value = serde_yaml::to_value(tokens)
        .map_err(|e| format!("serialize tokens: {}", e))?;
    mapping.insert(serde_yaml::Value::String("oauth_tokens".to_string()), tokens_value);
    let yaml_str = serde_yaml::to_string(&serde_yaml::Value::Mapping(mapping))
        .map_err(|e| format!("serialize yaml: {}", e))?;
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, &yaml_str).await
        .map_err(|e| format!("write {:?}: {}", tmp, e))?;
    #[cfg(target_os = "windows")]
    if path.exists() {
        tokio::fs::remove_file(&path).await
            .map_err(|e| format!("remove {:?}: {}", path, e))?;
    }
    tokio::fs::rename(&tmp, &path).await
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
    let existing = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    let mut mapping: serde_yaml::Mapping = serde_yaml::from_str(&existing).unwrap_or_default();
    mapping.remove(serde_yaml::Value::String("oauth_tokens".to_string()));
    let yaml_str = serde_yaml::to_string(&serde_yaml::Value::Mapping(mapping))
        .map_err(|e| format!("serialize yaml: {}", e))?;
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, &yaml_str).await
        .map_err(|e| format!("write {:?}: {}", tmp, e))?;
    #[cfg(target_os = "windows")]
    if path.exists() {
        tokio::fs::remove_file(&path).await
            .map_err(|e| format!("remove {:?}: {}", path, e))?;
    }
    tokio::fs::rename(&tmp, &path).await
        .map_err(|e| format!("rename {:?} -> {:?}: {}", tmp, path, e))?;
    Ok(())
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
        match self.settings.auth_type.as_str() {
            "none" => Err("No auth configured".to_string()),
            "bearer" => {
                if self.settings.bearer_token.is_empty() {
                    return Err("Bearer token is empty".to_string());
                }
                Ok(self.settings.bearer_token.clone())
            }
            "oauth2_client_credentials" => self.get_oauth2_token().await,
            "oauth2_pkce" => {
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
            other => Err(format!("Unknown auth_type: {}", other)),
        }
    }

    async fn get_oauth2_token(&self) -> Result<String, String> {
        {
            let cache = self.token_cache.lock().await;
            if let Some(state) = cache.as_ref() {
                let still_valid = state
                    .expires_at
                    .map_or(true, |exp| exp > Instant::now() + Duration::from_secs(30));
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

        let client = reqwest::Client::new();
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
            return Err(format!("OAuth2 token endpoint returned HTTP {}", resp.status()));
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
        match self.settings.auth_type.as_str() {
            "none" => Ok(()),
            "bearer" | "oauth2_client_credentials" | "oauth2_pkce" => {
                let token = self.get_token().await?;
                headers.insert("Authorization".to_string(), format!("Bearer {}", token));
                Ok(())
            }
            other => Err(format!("Unknown auth_type: {}", other)),
        }
    }
}

fn reconstruct_token_response(
    tokens: &MCPOAuthTokens,
) -> Result<StandardTokenResponse<EmptyExtraTokenFields, BasicTokenType>, String> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let expires_in_secs = if tokens.expires_at > now_ms {
        (tokens.expires_at - now_ms) / 1000
    } else {
        0
    };

    let mut token_json = serde_json::json!({
        "access_token": tokens.access_token,
        "token_type": "Bearer",
    });
    if !tokens.refresh_token.is_empty() {
        token_json["refresh_token"] = serde_json::Value::String(tokens.refresh_token.clone());
    }
    if expires_in_secs > 0 {
        token_json["expires_in"] = serde_json::Value::Number(expires_in_secs.into());
    }
    serde_json::from_value(token_json)
        .map_err(|e| format!("Failed to reconstruct token response: {}", e))
}

pub async fn create_auth_manager_from_tokens(
    mcp_url: &str,
    tokens: &MCPOAuthTokens,
) -> Result<AuthorizationManager, String> {
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

struct PendingOAuthSession {
    oauth_state: Arc<AMutex<OAuthState>>,
    config_path: String,
    created_at: SystemTime,
    state_param: String,
}

static PENDING_SESSIONS: OnceLock<AMutex<HashMap<String, PendingOAuthSession>>> = OnceLock::new();

fn pending_sessions() -> &'static AMutex<HashMap<String, PendingOAuthSession>> {
    PENDING_SESSIONS.get_or_init(|| AMutex::new(HashMap::new()))
}

pub struct MCPOAuthSessionManager;

impl MCPOAuthSessionManager {
    pub async fn start_oauth_flow(
        mcp_url: &str,
        config_path: &str,
        scopes: &[&str],
        redirect_uri: &str,
    ) -> Result<(String, String), String> {
        let mut state = OAuthState::new(mcp_url, None)
            .await
            .map_err(|e| format!("create OAuth state: {}", e))?;
        state.start_authorization(scopes, redirect_uri)
            .await
            .map_err(|e| format!("start OAuth authorization: {}", e))?;
        let auth_url = state.get_authorization_url()
            .await
            .map_err(|e| format!("get authorization URL: {}", e))?;
        let state_param = url::Url::parse(&auth_url)
            .ok()
            .and_then(|u| u.query_pairs().find(|(k, _)| k == "state").map(|(_, v)| v.to_string()))
            .unwrap_or_default();
        let session_id = Uuid::new_v4().to_string();
        pending_sessions().lock().await.insert(session_id.clone(), PendingOAuthSession {
            oauth_state: Arc::new(AMutex::new(state)),
            config_path: config_path.to_string(),
            created_at: SystemTime::now(),
            state_param,
        });
        Ok((session_id, auth_url))
    }

    pub async fn exchange_code(session_id: &str, code: &str) -> Result<(MCPOAuthTokens, String), String> {
        let session = pending_sessions().lock().await.remove(session_id)
            .ok_or_else(|| format!("No pending OAuth session: {}", session_id))?;
        let config_path = session.config_path.clone();
        let mut oauth_state = session.oauth_state.lock().await;
        oauth_state.handle_callback(code)
            .await
            .map_err(|e| format!("OAuth callback: {}", e))?;
        let (client_id, creds_opt) = oauth_state.get_credentials()
            .await
            .map_err(|e| format!("get OAuth credentials: {}", e))?;
        let token_response = creds_opt.ok_or_else(|| "No credentials after callback".to_string())?;
        let token_json = serde_json::to_value(&token_response)
            .map_err(|e| format!("serialize token response: {}", e))?;
        let access_token = token_json.get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let refresh_token = token_json.get("refresh_token")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let expires_at = token_json.get("expires_in")
            .and_then(|v| v.as_u64())
            .map(|secs| {
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                now_ms + secs as i64 * 1000
            })
            .unwrap_or(0);
        Ok((MCPOAuthTokens {
            access_token,
            refresh_token,
            expires_at,
            client_id,
            client_secret: None,
            scopes: vec![],
        }, config_path))
    }

    pub async fn find_session_id_by_state(state: &str) -> Option<String> {
        let sessions = pending_sessions().lock().await;
        for (id, session) in sessions.iter() {
            if session.state_param == state {
                return Some(id.clone());
            }
        }
        None
    }

    pub async fn cleanup_expired_sessions() {
        let expiry = Duration::from_secs(600);
        let mut sessions = pending_sessions().lock().await;
        sessions.retain(|id, session| {
            let keep = session.created_at.elapsed().map(|age| age < expiry).unwrap_or(false);
            if !keep {
                warn!("MCPOAuthSessionManager: removing expired session {}", id);
            }
            keep
        });
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
        assert_eq!(s.auth_type, "none");
        assert!(s.bearer_token.is_empty());
    }

    #[test]
    fn test_auth_type_backward_compat_oauth2_alias() {
        let json = serde_json::json!({"auth_type": "oauth2"});
        let settings: MCPAuthSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.auth_type, "oauth2_client_credentials");
    }

    #[test]
    fn test_auth_type_oauth2_client_credentials_unchanged() {
        let json = serde_json::json!({"auth_type": "oauth2_client_credentials"});
        let settings: MCPAuthSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.auth_type, "oauth2_client_credentials");
    }

    #[test]
    fn test_auth_type_oauth2_pkce_deserialized() {
        let json = serde_json::json!({"auth_type": "oauth2_pkce"});
        let settings: MCPAuthSettings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.auth_type, "oauth2_pkce");
    }

    #[test]
    fn test_auth_settings_serialization_roundtrip() {
        let settings = MCPAuthSettings {
            auth_type: "bearer".to_string(),
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
        };

        save_tokens_to_config(&path, &tokens).await.unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("url: https://example.com/mcp"), "original fields preserved");
        assert!(content.contains("auth_type: oauth2_pkce"), "original fields preserved");
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
            sessions.insert(old_id.clone(), PendingOAuthSession {
                oauth_state: Arc::new(AMutex::new(old_state)),
                config_path: "/tmp/test.yaml".to_string(),
                created_at: SystemTime::now() - Duration::from_secs(700),
                state_param: String::new(),
            });
        }

        let fresh_state = OAuthState::new("http://localhost", None).await.unwrap();
        {
            let mut sessions = pending_sessions().lock().await;
            sessions.insert(fresh_id.clone(), PendingOAuthSession {
                oauth_state: Arc::new(AMutex::new(fresh_state)),
                config_path: "/tmp/test.yaml".to_string(),
                created_at: SystemTime::now(),
                state_param: String::new(),
            });
        }

        MCPOAuthSessionManager::cleanup_expired_sessions().await;

        {
            let sessions = pending_sessions().lock().await;
            assert!(!sessions.contains_key(&old_id), "stale session should be removed");
            assert!(sessions.contains_key(&fresh_id), "fresh session should remain");
        }

        pending_sessions().lock().await.remove(&fresh_id);
    }

    #[tokio::test]
    async fn test_bearer_token_injection() {
        let settings = MCPAuthSettings {
            auth_type: "bearer".to_string(),
            bearer_token: "my-secret-token".to_string(),
            ..Default::default()
        };
        let manager = MCPTokenManager::new(settings);
        let mut headers = HashMap::new();
        manager.apply_auth(&mut headers).await.unwrap();
        assert_eq!(headers.get("Authorization").unwrap(), "Bearer my-secret-token");
    }

    #[tokio::test]
    async fn test_none_auth_does_not_inject_headers() {
        let settings = MCPAuthSettings {
            auth_type: "none".to_string(),
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
            auth_type: "bearer".to_string(),
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
            auth_type: "oauth2_client_credentials".to_string(),
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
            auth_type: "oauth2_client_credentials".to_string(),
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
        };
        let response = reconstruct_token_response(&tokens).unwrap();
        assert_eq!(response.access_token().secret(), "access_abc123");
    }

    #[test]
    fn test_reconstruct_token_response_expired_no_expires_in() {
        use oauth2::TokenResponse;
        let tokens = MCPOAuthTokens {
            access_token: "access_expired".to_string(),
            refresh_token: "refresh_xyz".to_string(),
            expires_at: 1_000_000,
            client_id: "client_id_1".to_string(),
            client_secret: None,
            scopes: vec![],
        };
        let response = reconstruct_token_response(&tokens).unwrap();
        assert_eq!(response.access_token().secret(), "access_expired");
        assert!(response.expires_in().is_none());
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
        assert!(result.is_none(), "Empty config should return None for tokens");
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
        };
        save_tokens_to_config(&path, &tokens).await.unwrap();

        let loaded = load_tokens_from_config(&path).await.unwrap();
        assert_eq!(loaded.access_token, "test_access");

        let response = reconstruct_token_response(&loaded).unwrap();
        assert_eq!(response.access_token().secret(), "test_access");
    }

    #[tokio::test]
    async fn test_unknown_auth_type_returns_error() {
        let settings = MCPAuthSettings {
            auth_type: "digest".to_string(),
            ..Default::default()
        };
        let manager = MCPTokenManager::new(settings);
        let mut headers = HashMap::new();
        let result = manager.apply_auth(&mut headers).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown auth_type"));
    }
}
