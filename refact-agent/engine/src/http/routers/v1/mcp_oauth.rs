use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Extension;
use axum::extract::Query;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use tracing::warn;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::integrations::mcp::mcp_auth::{
    MCPOAuthSessionManager, clear_tokens_from_config, load_tokens_from_config,
    save_tokens_to_config,
};

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn json_response(
    status: StatusCode,
    body: &impl Serialize,
) -> Result<Response<Body>, ScratchError> {
    let json = serde_json::to_string(body).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("JSON: {}", e))
    })?;
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(json))
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Response build failed: {}", e),
            )
        })
}

fn html_response(
    title: &str,
    heading: &str,
    heading_color: &str,
    message: &str,
) -> Result<Response<Body>, ScratchError> {
    let html = format!(
        r#"<!DOCTYPE html>
<html><head><title>{title}</title></head>
<body style="font-family: system-ui; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; background: #1a1a2e; color: #e0e0e0;">
<div style="text-align: center;">
<h1 style="color: {heading_color};">{heading}</h1>
<p>{message}</p>
</div>
</body></html>"#,
        title = html_escape(title),
        heading = html_escape(heading),
        heading_color = heading_color,
        message = html_escape(message),
    );
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html")
        .header(
            "Content-Security-Policy",
            "default-src 'none'; style-src 'unsafe-inline'",
        )
        .body(Body::from(html))
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Response build failed: {}", e),
            )
        })
}

fn reject_path_traversal(config_path: &str) -> Result<(), ScratchError> {
    if config_path.contains("..") {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid config_path: path traversal not allowed".to_string(),
        ));
    }
    Ok(())
}

async fn validate_mcp_config_path(
    gcx: &Arc<ARwLock<GlobalContext>>,
    config_path: &str,
) -> Result<String, ScratchError> {
    reject_path_traversal(config_path)?;
    let exists = gcx
        .read()
        .await
        .integration_sessions
        .contains_key(config_path);
    if !exists {
        return Err(ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("No active MCP session for config: {}", config_path),
        ));
    }
    Ok(config_path.to_string())
}

async fn reload_mcp_integration(gcx: Arc<ARwLock<GlobalContext>>, config_path: &str) {
    let config_filename = std::path::Path::new(config_path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let _ = crate::integrations::running_integrations::load_integrations(
        gcx,
        &[format!("**/integrations.d/{}", config_filename)],
    )
    .await;
}

#[derive(Deserialize)]
pub struct McpOAuthStartRequest {
    pub config_path: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Serialize)]
struct McpOAuthStartResponse {
    session_id: String,
    authorize_url: String,
}

pub async fn handle_v1_mcp_oauth_start(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: McpOAuthStartRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    validate_mcp_config_path(&gcx, &req.config_path).await?;

    let http_port = gcx.read().await.cmdline.http_port;
    let redirect_uri = format!("http://127.0.0.1:{}/v1/mcp/oauth/callback", http_port);

    let config_content = tokio::fs::read_to_string(&req.config_path)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Read config {}: {}", req.config_path, e),
            )
        })?;
    let config_yaml: serde_yaml::Value = serde_yaml::from_str(&config_content).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Parse config: {}", e),
        )
    })?;
    let mcp_url = config_yaml
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::BAD_REQUEST,
                "Config has no 'url' field; stdio transport does not support OAuth".to_string(),
            )
        })?
        .to_string();

    let scopes: Vec<&str> = req.scopes.iter().map(|s| s.as_str()).collect();
    let (session_id, authorize_url) = MCPOAuthSessionManager::start_oauth_flow(
        &mcp_url,
        &req.config_path,
        &scopes,
        &redirect_uri,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::BAD_GATEWAY, e))?;

    json_response(
        StatusCode::OK,
        &McpOAuthStartResponse {
            session_id,
            authorize_url,
        },
    )
}

#[derive(Deserialize)]
pub struct McpOAuthExchangeRequest {
    pub session_id: String,
    pub code: String,
}

pub async fn handle_v1_mcp_oauth_exchange(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: McpOAuthExchangeRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    let (tokens, config_path) = MCPOAuthSessionManager::exchange_code(&req.session_id, &req.code)
        .await
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    save_tokens_to_config(&config_path, &tokens)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Save tokens: {}", e),
            )
        })?;

    reload_mcp_integration(gcx, &config_path).await;

    json_response(StatusCode::OK, &serde_json::json!({"success": true}))
}

#[derive(Deserialize)]
pub struct McpOAuthCallbackQuery {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

pub async fn handle_v1_mcp_oauth_callback(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(query): Query<McpOAuthCallbackQuery>,
) -> Result<Response<Body>, ScratchError> {
    if let Some(err) = &query.error {
        let desc = query
            .error_description
            .as_deref()
            .unwrap_or("Unknown error");
        warn!("MCP OAuth error from server: {} — {}", err, desc);
        return html_response(
            "Authentication Failed",
            "✗ Authentication Failed",
            "#ef4444",
            &format!("{}: {}", err, desc),
        );
    }

    let code = match &query.code {
        Some(c) if !c.is_empty() => c.clone(),
        _ => {
            return html_response(
                "Authentication Failed",
                "✗ Authentication Failed",
                "#ef4444",
                "No authorization code received. Please try again.",
            );
        }
    };

    let state = match &query.state {
        Some(s) if !s.is_empty() => s.clone(),
        _ => {
            return html_response(
                "Authentication Failed",
                "✗ Authentication Failed",
                "#ef4444",
                "Missing state parameter. Please start the OAuth flow again.",
            );
        }
    };

    let session_id = match MCPOAuthSessionManager::find_session_id_by_state(&state).await {
        Some(id) => id,
        None => {
            return html_response(
                "Authentication Failed",
                "✗ Authentication Failed",
                "#ef4444",
                "OAuth session expired or not found. Please start the OAuth flow again.",
            );
        }
    };

    let (tokens, config_path) =
        match MCPOAuthSessionManager::exchange_code(&session_id, &code).await {
            Ok(result) => result,
            Err(e) => {
                warn!("MCP OAuth exchange failed: {}", e);
                return html_response(
                    "Authentication Failed",
                    "✗ Authentication Failed",
                    "#ef4444",
                    &format!("Token exchange failed: {}", e),
                );
            }
        };

    if let Err(e) = save_tokens_to_config(&config_path, &tokens).await {
        warn!("MCP OAuth: failed to save tokens: {}", e);
        return html_response(
            "Authentication Failed",
            "✗ Authentication Failed",
            "#ef4444",
            "Tokens received but failed to save. Please try again.",
        );
    }

    reload_mcp_integration(gcx, &config_path).await;

    html_response(
        "Authentication Successful",
        "✓ Authentication Successful",
        "#4ade80",
        "You can close this window and return to the application.",
    )
}

#[derive(Deserialize)]
pub struct McpOAuthLogoutRequest {
    pub config_path: String,
}

pub async fn handle_v1_mcp_oauth_logout(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: McpOAuthLogoutRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    validate_mcp_config_path(&gcx, &req.config_path).await?;

    clear_tokens_from_config(&req.config_path)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Clear tokens: {}", e),
            )
        })?;

    reload_mcp_integration(gcx, &req.config_path).await;

    json_response(StatusCode::OK, &serde_json::json!({"success": true}))
}

#[derive(Deserialize)]
pub struct McpOAuthCancelRequest {
    pub session_id: String,
}

pub async fn handle_v1_mcp_oauth_cancel(
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: McpOAuthCancelRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;
    let cancelled = MCPOAuthSessionManager::cancel_oauth_flow(&req.session_id).await;
    json_response(StatusCode::OK, &serde_json::json!({"cancelled": cancelled}))
}

#[derive(Deserialize)]
pub struct McpOAuthStatusQuery {
    pub config_path: String,
}

#[derive(Serialize)]
struct McpOAuthStatusResponse {
    auth_type: String,
    authenticated: bool,
    expires_at: i64,
    scopes: Vec<String>,
}

pub async fn handle_v1_mcp_oauth_status(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(query): Query<McpOAuthStatusQuery>,
) -> Result<Response<Body>, ScratchError> {
    validate_mcp_config_path(&gcx, &query.config_path).await?;

    let config_content = tokio::fs::read_to_string(&query.config_path)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("Read config {}: {}", query.config_path, e),
            )
        })?;
    let config_yaml: serde_yaml::Value = serde_yaml::from_str(&config_content).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Parse config: {}", e),
        )
    })?;
    let auth_type = config_yaml
        .get("auth_type")
        .and_then(|v| v.as_str())
        .unwrap_or("none")
        .to_string();

    let tokens = load_tokens_from_config(&query.config_path).await;
    let (authenticated, expires_at, scopes) = match &tokens {
        Some(t) => {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            let authenticated =
                !t.access_token.is_empty() && (t.expires_at == 0 || t.expires_at > now_ms);
            (authenticated, t.expires_at, t.scopes.clone())
        }
        None => (false, 0, vec![]),
    };

    json_response(
        StatusCode::OK,
        &McpOAuthStatusResponse {
            auth_type,
            authenticated,
            expires_at,
            scopes,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use crate::integrations::mcp::mcp_auth::{
        MCPOAuthTokens, save_tokens_to_config, load_tokens_from_config, clear_tokens_from_config,
    };

    #[test]
    fn test_html_escape_script_tags() {
        let result = html_escape("<script>alert('xss')</script>");
        assert_eq!(
            result,
            "&lt;script&gt;alert(&#x27;xss&#x27;)&lt;/script&gt;"
        );
        assert!(!result.contains('<'));
        assert!(!result.contains('>'));
        assert!(!result.contains('\''));
    }

    #[test]
    fn test_html_response_contains_csp_header() {
        let response = html_response("Title", "Heading", "#4ade80", "Message").unwrap();
        let csp = response.headers().get("Content-Security-Policy").unwrap();
        let csp_str = csp.to_str().unwrap();
        assert!(csp_str.contains("default-src 'none'"));
        assert!(csp_str.contains("style-src 'unsafe-inline'"));
    }

    #[test]
    fn test_config_path_traversal_rejected() {
        assert!(reject_path_traversal("../../etc/passwd").is_err());
        assert!(reject_path_traversal("/tmp/../etc/passwd").is_err());
        assert!(reject_path_traversal("foo/../bar").is_err());
        assert!(reject_path_traversal("/safe/path/config.yaml").is_ok());
        assert!(reject_path_traversal(
            "/home/user/.config/refact/integrations.d/mcp_http_myserver.yaml"
        )
        .is_ok());
    }

    #[tokio::test]
    async fn test_oauth_start_fails_gracefully_when_server_unreachable() {
        use crate::integrations::mcp::mcp_auth::MCPOAuthSessionManager;
        let result = MCPOAuthSessionManager::start_oauth_flow(
            "http://127.0.0.1:1",
            "/tmp/test_mcp_oauth.yaml",
            &[],
            "http://127.0.0.1:8001/v1/mcp/oauth/callback",
        )
        .await;
        assert!(
            result.is_err(),
            "start_oauth_flow should fail when server is unreachable"
        );
        let err = result.unwrap_err();
        assert!(!err.is_empty(), "error message should not be empty");
    }

    #[tokio::test]
    async fn test_exchange_code_rejects_unknown_session_id() {
        use crate::integrations::mcp::mcp_auth::MCPOAuthSessionManager;
        let result =
            MCPOAuthSessionManager::exchange_code("unknown-session-id-12345", "some_code").await;
        assert!(result.is_err(), "exchange with unknown session should fail");
        assert!(
            result.unwrap_err().contains("No pending OAuth session"),
            "should say session not found"
        );
    }

    #[tokio::test]
    async fn test_expired_sessions_are_rejected() {
        use crate::integrations::mcp::mcp_auth::MCPOAuthSessionManager;
        MCPOAuthSessionManager::cleanup_expired_sessions().await;
        let result = MCPOAuthSessionManager::exchange_code("nonexistent-session-xyz", "code").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No pending OAuth session"));
    }

    #[tokio::test]
    async fn test_logout_clears_tokens_from_config() {
        let mut tmp = NamedTempFile::new().unwrap();
        let existing = "url: https://example.com\nauth_type: oauth2_pkce\n";
        tmp.write_all(existing.as_bytes()).unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let tokens = MCPOAuthTokens {
            access_token: "tok".to_string(),
            refresh_token: "ref".to_string(),
            expires_at: 9999999999000,
            client_id: "cid".to_string(),
            client_secret: None,
            scopes: vec!["read".to_string()],
        };
        save_tokens_to_config(&path, &tokens).await.unwrap();
        assert!(load_tokens_from_config(&path).await.is_some());

        clear_tokens_from_config(&path).await.unwrap();
        assert!(
            load_tokens_from_config(&path).await.is_none(),
            "tokens should be cleared"
        );
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            content.contains("url: https://example.com"),
            "other fields preserved"
        );
    }

    #[tokio::test]
    async fn test_status_returns_authenticated_when_valid_token() {
        let mut tmp = NamedTempFile::new().unwrap();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let future_expiry = now_ms + 3_600_000;
        let yaml = format!(
            "url: https://example.com\nauth_type: oauth2_pkce\noauth_tokens:\n  access_token: live_token\n  refresh_token: ref\n  expires_at: {}\n  client_id: cid\n  scopes:\n    - read\n",
            future_expiry
        );
        tmp.write_all(yaml.as_bytes()).unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let tokens = load_tokens_from_config(&path).await;
        assert!(tokens.is_some());
        let t = tokens.unwrap();
        assert_eq!(t.access_token, "live_token");
        let authenticated =
            !t.access_token.is_empty() && (t.expires_at == 0 || t.expires_at > now_ms);
        assert!(authenticated, "token should be valid");
    }

    #[tokio::test]
    async fn test_status_returns_not_authenticated_when_expired() {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let past_expiry = now_ms - 1_000;
        let mut tmp = NamedTempFile::new().unwrap();
        let yaml = format!(
            "auth_type: oauth2_pkce\noauth_tokens:\n  access_token: expired_token\n  refresh_token: ref\n  expires_at: {}\n  client_id: cid\n  scopes: []\n",
            past_expiry
        );
        tmp.write_all(yaml.as_bytes()).unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let tokens = load_tokens_from_config(&path).await;
        assert!(tokens.is_some());
        let t = tokens.unwrap();
        let authenticated =
            !t.access_token.is_empty() && (t.expires_at == 0 || t.expires_at > now_ms);
        assert!(!authenticated, "expired token should not be authenticated");
    }
}
