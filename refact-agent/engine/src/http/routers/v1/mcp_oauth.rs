use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::Query;
use axum::http::{Response, StatusCode};
use axum::extract::State;
use hyper::Body;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use crate::custom_error::ScratchError;
use crate::integrations::mcp::mcp_auth::gateway_oauth;
use crate::integrations::mcp::mcp_auth::{
    MCPOAuthSessionManager, clear_tokens_from_config, ensure_oauth2_pkce_auth_type,
    load_tokens_from_config, save_tokens_to_config,
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
    gcx: &Arc<GlobalContext>,
    config_path: &str,
) -> Result<String, ScratchError> {
    reject_path_traversal(config_path)?;
    let integration_sessions = gcx.integration_sessions.clone();
    let exists = integration_sessions.lock().await.contains_key(config_path);
    if !exists {
        return Err(ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("No active MCP session for config: {}", config_path),
        ));
    }
    Ok(config_path.to_string())
}

async fn reload_mcp_integration(gcx: Arc<GlobalContext>, config_path: &str) {
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
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let req: McpOAuthStartRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    validate_mcp_config_path(&gcx, &req.config_path).await?;
    let managed_identity = gateway_oauth::managed_identity(&req.config_path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::CONFLICT, e))?;

    let http_port = gcx.cmdline.http_port;
    let redirect_uri = match managed_identity {
        Some(_) => crate::providers::litellm_mcp::gateway_callback_url(&req.config_path, http_port)
            .await
            .map_err(|e| ScratchError::new(StatusCode::CONFLICT, e))?
            .ok_or_else(|| {
                ScratchError::new(
                    StatusCode::CONFLICT,
                    "Managed LiteLLM connection is no longer active".to_string(),
                )
            })?,
        None => format!("http://127.0.0.1:{}/v1/mcp/oauth/callback", http_port),
    };

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

    let probe_scopes: Vec<String> = {
        let session_opt = {
            let sessions = gcx.integration_sessions.lock().await;
            sessions.get(&req.config_path).cloned()
        };
        match session_opt {
            Some(session_arc) => {
                let mut session_locked = session_arc.lock().await;
                session_locked
                    .as_any_mut()
                    .downcast_mut::<crate::integrations::mcp::session_mcp::SessionMCP>()
                    .and_then(|s| s.oauth_probe.as_ref().map(|p| p.suggested_scopes.clone()))
                    .unwrap_or_default()
            }
            None => vec![],
        }
    };

    let config_auth_type = config_yaml
        .get("auth_type")
        .and_then(|v| v.as_str())
        .unwrap_or("none")
        .to_string();
    if config_auth_type != "oauth2_pkce" {
        ensure_oauth2_pkce_auth_type(&req.config_path)
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    }

    let effective_scopes: Vec<String> = if !req.scopes.is_empty() {
        req.scopes.clone()
    } else {
        let config_scopes = serde_yaml::from_value::<
            crate::integrations::mcp::mcp_auth::MCPAuthSettings,
        >(config_yaml.clone())
        .map(|a| a.oauth2_scopes)
        .unwrap_or_default();
        if !config_scopes.is_empty() {
            config_scopes
        } else {
            probe_scopes
        }
    };

    if let Some(identity) = managed_identity {
        gateway_oauth::revalidate_binding(&req.config_path, &identity, &mcp_url)
            .await
            .map_err(|e| ScratchError::new(StatusCode::CONFLICT, e))?;
        let (session_id, authorize_url) = gateway_oauth::start_flow(
            &req.config_path,
            &identity,
            &mcp_url,
            &effective_scopes,
            &redirect_uri,
        )
        .await
        .map_err(|e| ScratchError::new(StatusCode::BAD_GATEWAY, e))?;
        return json_response(
            StatusCode::OK,
            &McpOAuthStartResponse {
                session_id,
                authorize_url,
            },
        );
    }

    let scopes: Vec<&str> = effective_scopes.iter().map(|s| s.as_str()).collect();
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
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let req: McpOAuthExchangeRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    if gateway_oauth::has_session(&req.session_id).await {
        let (_, config_path) = gateway_oauth::complete_flow(&req.session_id, &req.code)
            .await
            .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;
        reload_mcp_integration(gcx, &config_path).await;
        return json_response(StatusCode::OK, &serde_json::json!({"success": true}));
    }

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

const SAFE_OAUTH_ERROR_CODES: &[&str] = &[
    "access_denied",
    "invalid_request",
    "unauthorized_client",
    "unsupported_response_type",
    "invalid_scope",
    "server_error",
    "temporarily_unavailable",
    "login_required",
    "consent_required",
    "interaction_required",
];

fn sanitized_oauth_error_code(err: &str) -> &'static str {
    SAFE_OAUTH_ERROR_CODES
        .iter()
        .copied()
        .find(|code| *code == err)
        .unwrap_or("unknown_error")
}

async fn callback_provider_error(
    state: Option<&str>,
    err: &str,
) -> Result<Response<Body>, ScratchError> {
    let code = sanitized_oauth_error_code(err);
    let managed_session = match state.filter(|s| !s.is_empty()) {
        Some(state) => gateway_oauth::find_session_by_state(state).await,
        None => None,
    };
    match managed_session {
        Some(session_id) => {
            gateway_oauth::cancel(&session_id).await;
            warn!("Managed LiteLLM MCP OAuth sign-in failed at the authorization server ({code}); pending sign-in cancelled");
        }
        None => warn!("MCP OAuth sign-in failed at the authorization server ({code})"),
    }
    html_response(
        "Authentication Failed",
        "✗ Authentication Failed",
        "#ef4444",
        &format!(
            "The authorization server did not complete sign-in ({code}). Please start the OAuth flow again."
        ),
    )
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
    State(app): State<AppState>,
    Query(query): Query<McpOAuthCallbackQuery>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    if let Some(err) = &query.error {
        return callback_provider_error(query.state.as_deref(), err).await;
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

    let managed_session = gateway_oauth::find_session_by_state(&state).await;
    let session_id = match managed_session
        .clone()
        .or(MCPOAuthSessionManager::find_session_id_by_state(&state).await)
    {
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

    let exchange = if managed_session.is_some() {
        gateway_oauth::complete_flow(&session_id, &code).await
    } else {
        MCPOAuthSessionManager::exchange_code(&session_id, &code).await
    };
    let (tokens, config_path) = match exchange {
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

    if managed_session.is_none() {
        if let Err(e) = save_tokens_to_config(&config_path, &tokens).await {
            warn!("MCP OAuth: failed to save tokens: {}", e);
            return html_response(
                "Authentication Failed",
                "✗ Authentication Failed",
                "#ef4444",
                "Tokens received but failed to save. Please try again.",
            );
        }
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
    State(app): State<AppState>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let req: McpOAuthLogoutRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid JSON: {}", e),
        )
    })?;

    validate_mcp_config_path(&gcx, &req.config_path).await?;

    let managed = crate::providers::litellm_mcp::logout_gateway(&req.config_path)
        .await
        .map_err(|_| {
            ScratchError::new(
                StatusCode::CONFLICT,
                "Cannot sign out managed MCP session".to_string(),
            )
        })?;
    if !managed {
        clear_tokens_from_config(&req.config_path)
            .await
            .map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Clear tokens: {}", e),
                )
            })?;
    }

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
    let cancelled = gateway_oauth::cancel(&req.session_id).await
        | MCPOAuthSessionManager::cancel_oauth_flow(&req.session_id).await;
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
    needs_login: bool,
    oauth_available: bool,
    suggested_scopes: Vec<String>,
}

pub async fn handle_v1_mcp_oauth_status(
    State(app): State<AppState>,
    Query(query): Query<McpOAuthStatusQuery>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
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
    let config_url = config_yaml
        .get("url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let tokens = load_tokens_from_config(&query.config_path).await;
    let (authenticated, expires_at, scopes) = match &tokens {
        Some(t) => {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            let url_bound_ok = match (t.bound_url.as_deref(), config_url.as_deref()) {
                (Some(bound), Some(cfg)) => bound == cfg,
                _ => true,
            };
            let authenticated = !t.access_token.is_empty()
                && (t.expires_at == 0 || t.expires_at > now_ms)
                && url_bound_ok;
            (authenticated, t.expires_at, t.scopes.clone())
        }
        None => (false, 0, vec![]),
    };

    let (session_needs_login, probe_oauth_available, probe_scopes) = {
        let session_opt = {
            let sessions = gcx.integration_sessions.lock().await;
            sessions.get(&query.config_path).cloned()
        };
        match session_opt {
            Some(session_arc) => {
                let mut session_locked = session_arc.lock().await;
                match session_locked
                    .as_any_mut()
                    .downcast_mut::<crate::integrations::mcp::session_mcp::SessionMCP>()
                {
                    Some(s) => {
                        let needs = matches!(
                            s.auth_status,
                            crate::integrations::mcp::session_mcp::MCPAuthStatus::NeedsLogin
                                | crate::integrations::mcp::session_mcp::MCPAuthStatus::NeedsReauth
                        );
                        let (avail, pscopes) = match s.oauth_probe.as_ref() {
                            Some(p) => (p.oauth_available, p.suggested_scopes.clone()),
                            None => (false, vec![]),
                        };
                        (needs, avail, pscopes)
                    }
                    None => (false, false, vec![]),
                }
            }
            None => (false, false, vec![]),
        }
    };

    let needs_login = session_needs_login || (auth_type == "oauth2_pkce" && !authenticated);
    let oauth_available = auth_type == "oauth2_pkce" || probe_oauth_available;
    let suggested_scopes = if !scopes.is_empty() {
        scopes.clone()
    } else {
        probe_scopes
    };

    json_response(
        StatusCode::OK,
        &McpOAuthStatusResponse {
            auth_type,
            authenticated,
            expires_at,
            scopes,
            needs_login,
            oauth_available,
            suggested_scopes,
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

    async fn body_text(response: Response<Body>) -> String {
        String::from_utf8(
            hyper::body::to_bytes(response.into_body())
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    async fn fake_gateway() -> String {
        use axum::http::Uri;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let base_for = base.clone();
        let app = axum::Router::new().fallback(move |uri: Uri| {
            let base = base_for.clone();
            async move {
                let (status, body) = match uri.path() {
                    "/.well-known/oauth-protected-resource/srv/mcp" => (200, format!(
                        r#"{{"resource":"{base}/srv/mcp","authorization_servers":["{base}"]}}"#
                    )),
                    "/.well-known/oauth-authorization-server" => (200, format!(
                        r#"{{"issuer":"{base}","authorization_endpoint":"https://idp.example/authorize","token_endpoint":"{base}/srv/token","registration_endpoint":"{base}/srv/register","code_challenge_methods_supported":["S256"],"scopes_supported":["mcp"]}}"#
                    )),
                    "/srv/register" => (201, r#"{"client_id":"cid-1"}"#.to_string()),
                    _ => (404, "{}".to_string()),
                };
                axum::response::Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap()
            }
        });
        tokio::spawn(
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(app.into_make_service()),
        );
        base
    }

    #[tokio::test]
    async fn test_managed_callback_error_cancels_flow_without_reflection() {
        let base = fake_gateway().await;
        let resolver: gateway_oauth::HeaderResolver =
            Arc::new(|_url: String| Box::pin(async { Ok(Some(std::collections::HashMap::new())) }));
        let (session_id, auth_url) = gateway_oauth::start_flow_with(
            "/tmp/managed-callback-error.yaml",
            "ident",
            &format!("{base}/srv/mcp"),
            &[],
            "https://refact.example.com/v1/mcp/oauth/callback",
            resolver,
        )
        .await
        .unwrap();
        let state = url::Url::parse(&auth_url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .to_string();
        assert!(gateway_oauth::has_session(&session_id).await);

        let response = callback_provider_error(Some(&state), "<script>evil-code</script>")
            .await
            .unwrap();
        let body = body_text(response).await;
        assert!(body.contains("unknown_error"));
        assert!(!body.contains("evil-code") && !body.contains("script&gt;"));
        assert!(!gateway_oauth::has_session(&session_id).await);
        assert!(gateway_oauth::find_session_by_state(&state).await.is_none());
    }

    #[tokio::test]
    async fn test_generic_callback_error_shows_only_known_codes() {
        let body = body_text(
            callback_provider_error(Some("unknown-state"), "access_denied")
                .await
                .unwrap(),
        )
        .await;
        assert!(body.contains("(access_denied)"));
        let body = body_text(
            callback_provider_error(None, "token=leaked-secret")
                .await
                .unwrap(),
        )
        .await;
        assert!(body.contains("(unknown_error)"));
        assert!(!body.contains("leaked-secret"));
        assert_eq!(sanitized_oauth_error_code("access_denied"), "access_denied");
        assert_eq!(sanitized_oauth_error_code("ACCESS_DENIED"), "unknown_error");
    }

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
            bound_url: None,
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
}
