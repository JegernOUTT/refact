use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use serde::Deserialize;

pub type HeaderResolver = Arc<
    dyn Fn(String) -> BoxFuture<'static, Result<Option<HashMap<String, String>>, String>>
        + Send
        + Sync,
>;

const MAX_EXPIRES_IN_SECS: u64 = 366 * 24 * 3600;
const MAX_TOKEN_LEN: usize = 16 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

pub struct ScopedSender {
    gateway_origin: url::Origin,
    resolver: HeaderResolver,
    client: reqwest::Client,
}

impl ScopedSender {
    pub fn new(gateway_url: &str, resolver: HeaderResolver) -> Result<Self, String> {
        let parsed =
            url::Url::parse(gateway_url).map_err(|_| "Invalid gateway MCP URL".to_string())?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| "Cannot build gateway OAuth HTTP client".to_string())?;
        Ok(Self {
            gateway_origin: parsed.origin(),
            resolver,
            client,
        })
    }

    pub async fn headers_for(&self, target: &url::Url) -> Result<HashMap<String, String>, String> {
        if target.origin() != self.gateway_origin {
            return Ok(HashMap::new());
        }
        match (self.resolver)(target.to_string()).await? {
            Some(headers) => Ok(headers),
            None => Err("Managed LiteLLM connection is no longer active".to_string()),
        }
    }

    pub async fn send(
        &self,
        method: reqwest::Method,
        target: &url::Url,
        form: Option<&[(&str, String)]>,
        json: Option<&serde_json::Value>,
    ) -> Result<reqwest::Response, String> {
        if !matches!(target.scheme(), "https" | "http")
            || !target.username().is_empty()
            || target.password().is_some()
        {
            return Err("Unsafe OAuth endpoint URL".to_string());
        }
        if target.scheme() == "http"
            && target.origin() != self.gateway_origin
            && !is_loopback(target)
        {
            return Err("OAuth endpoint must use HTTPS".to_string());
        }
        let mut request = self
            .client
            .request(method, target.clone())
            .header(reqwest::header::ACCEPT, "application/json");
        for (name, value) in self.headers_for(target).await? {
            request = request.header(name, value);
        }
        if let Some(form) = form {
            request = request.form(form);
        }
        if let Some(json) = json {
            request = request.json(json);
        }
        request
            .send()
            .await
            .map_err(|_| "OAuth request failed".to_string())
    }
}

fn is_loopback(target: &url::Url) -> bool {
    matches!(
        target.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("[::1]")
    )
}

pub const MAX_BODY_BYTES: usize = 256 * 1024;

pub async fn read_json_bounded<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T, String> {
    if response
        .content_length()
        .map(|n| n as usize > MAX_BODY_BYTES)
        .unwrap_or(false)
    {
        return Err("OAuth response too large".to_string());
    }
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "OAuth response read failed".to_string())?
    {
        if buf.len() + chunk.len() > MAX_BODY_BYTES {
            return Err("OAuth response too large".to_string());
        }
        buf.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&buf).map_err(|_| "Invalid OAuth response".to_string())
}

const KNOWN_ERROR_CODES: &[&str] = &[
    "invalid_request",
    "invalid_client",
    "invalid_grant",
    "unauthorized_client",
    "unsupported_grant_type",
    "invalid_scope",
    "invalid_token",
    "access_denied",
    "invalid_redirect_uri",
    "invalid_client_metadata",
    "server_error",
    "temporarily_unavailable",
];

pub async fn redacted_failure(what: &str, response: reqwest::Response) -> String {
    let status = response.status().as_u16();
    let code = read_json_bounded::<serde_json::Value>(response)
        .await
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .and_then(|c| {
            KNOWN_ERROR_CODES
                .iter()
                .find(|k| **k == c)
                .map(|k| k.to_string())
        });
    match code {
        Some(code) => format!("{} failed: HTTP {} ({})", what, status, code),
        None => format!("{} failed: HTTP {}", what, status),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthServerMetadata {
    #[serde(default)]
    pub issuer: Option<String>,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Option<Vec<String>>,
    #[serde(default)]
    pub code_challenge_methods_supported: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ProtectedResourceMetadata {
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Option<Vec<String>>,
}

fn ingress_and_name(resource: &url::Url) -> (String, String) {
    let path = resource.path().trim_end_matches('/');
    let segments: Vec<&str> = path.split('/').collect();
    match segments.as_slice() {
        [head @ .., name, "mcp"] if !name.is_empty() => (head.join("/"), (*name).to_string()),
        _ => (String::new(), String::new()),
    }
}

fn routes(base: &url::Url, paths: Vec<String>) -> Vec<url::Url> {
    let mut out = Vec::new();
    for candidate in paths {
        let mut u = base.clone();
        u.set_query(None);
        u.set_fragment(None);
        u.set_path(&candidate);
        if !out.contains(&u) {
            out.push(u);
        }
    }
    out
}

fn resource_metadata_routes(resource: &url::Url) -> Vec<url::Url> {
    let (prefix, name) = ingress_and_name(resource);
    let path = resource.path().trim_end_matches('/');
    let kind = "oauth-protected-resource";
    routes(
        resource,
        vec![
            format!("/.well-known/{kind}{path}"),
            format!("{prefix}/.well-known/{kind}/{name}/mcp"),
            format!("{prefix}/.well-known/{kind}"),
        ],
    )
}

fn issuer_metadata_routes(resource: &url::Url, issuer: &url::Url, kind: &str) -> Vec<url::Url> {
    let (prefix, _) = ingress_and_name(resource);
    let issuer_path = issuer.path().trim_end_matches('/');
    let rel = issuer_path.strip_prefix(prefix.as_str()).unwrap_or("");
    routes(
        issuer,
        vec![
            format!("/.well-known/{kind}{issuer_path}"),
            format!("{prefix}/.well-known/{kind}{rel}"),
        ],
    )
}

fn issuer_in_scope(resource: &url::Url, issuer: &url::Url) -> bool {
    let (prefix, name) = ingress_and_name(resource);
    let path = issuer.path().trim_end_matches('/');
    path == prefix
        || path == format!("{prefix}/mcp")
        || (!name.is_empty() && path == format!("{prefix}/{name}"))
}

async fn fetch_json<T: serde::de::DeserializeOwned>(
    sender: &ScopedSender,
    target: &url::Url,
) -> Result<Option<T>, String> {
    let response = sender
        .send(reqwest::Method::GET, target, None, None)
        .await
        .map_err(|_| {
            "Managed OAuth metadata request failed; check the gateway connection".to_string()
        })?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!(
            "Managed OAuth metadata request failed: HTTP {}",
            response.status().as_u16()
        ));
    }
    read_json_bounded::<T>(response).await.map(Some)
}

fn same_resource(a: &str, b: &url::Url) -> bool {
    url::Url::parse(a)
        .map(|u| {
            u.origin() == b.origin()
                && u.path().trim_end_matches('/') == b.path().trim_end_matches('/')
        })
        .unwrap_or(false)
}

pub async fn discover(
    sender: &ScopedSender,
    mcp_url: &url::Url,
) -> Result<AuthServerMetadata, String> {
    let gateway = mcp_url.origin();
    let (prefix, _) = ingress_and_name(mcp_url);
    let mut issuer = mcp_url.clone();
    issuer.set_path(if prefix.is_empty() { "/" } else { &prefix });
    issuer.set_query(None);
    issuer.set_fragment(None);
    let mut resource_scopes = None;
    let mut discovery_error = None;
    for candidate in resource_metadata_routes(mcp_url) {
        let metadata = match fetch_json::<ProtectedResourceMetadata>(sender, &candidate).await {
            Ok(metadata) => metadata,
            Err(error) => {
                discovery_error.get_or_insert(error);
                continue;
            }
        };
        if let Some(prm) = metadata {
            if let Some(resource) = prm.resource.as_deref() {
                if !same_resource(resource, mcp_url) {
                    return Err(
                        "Resource metadata does not match the managed MCP server; configure MCP Gateway URL to use the gateway's canonical MCP host".to_string()
                    );
                }
            }
            if let Some(first) = prm.authorization_servers.first() {
                issuer = url::Url::parse(first)
                    .map_err(|_| "Invalid authorization server in resource metadata".to_string())?;
            }
            resource_scopes = prm.scopes_supported;
            break;
        }
    }
    if issuer.origin() != gateway {
        return Err("Managed MCP authorization server is not on the approved gateway".to_string());
    }
    if !issuer_in_scope(mcp_url, &issuer) {
        return Err(
            "Managed MCP authorization server is outside the approved gateway ingress".to_string(),
        );
    }
    issuer.set_query(None);
    issuer.set_fragment(None);
    for candidate in issuer_metadata_routes(mcp_url, &issuer, "oauth-authorization-server")
        .into_iter()
        .chain(issuer_metadata_routes(
            mcp_url,
            &issuer,
            "openid-configuration",
        ))
    {
        let metadata = match fetch_json::<AuthServerMetadata>(sender, &candidate).await {
            Ok(metadata) => metadata,
            Err(error) => {
                discovery_error.get_or_insert(error);
                continue;
            }
        };
        if let Some(mut metadata) = metadata {
            validate_endpoints(&metadata, &gateway)?;
            if let Some(claimed) = metadata.issuer.as_deref() {
                let claimed = url::Url::parse(claimed)
                    .map_err(|_| "Invalid issuer in metadata".to_string())?;
                if claimed.origin() != gateway {
                    return Err(
                        "Authorization server issuer is not the approved gateway".to_string()
                    );
                }
            }
            if metadata.scopes_supported.as_ref().is_none_or(Vec::is_empty) {
                metadata.scopes_supported = resource_scopes;
            }
            return Ok(metadata);
        }
    }
    Err(discovery_error
        .unwrap_or_else(|| "OAuth metadata not found for managed MCP server".to_string()))
}

fn validate_endpoints(metadata: &AuthServerMetadata, gateway: &url::Origin) -> Result<(), String> {
    let parse = |e: &str| -> Result<url::Url, String> {
        let u = url::Url::parse(e).map_err(|_| "Invalid OAuth endpoint in metadata".to_string())?;
        if !matches!(u.scheme(), "https" | "http")
            || !u.username().is_empty()
            || u.password().is_some()
        {
            return Err("Unsafe OAuth endpoint in metadata".to_string());
        }
        Ok(u)
    };
    parse(&metadata.authorization_endpoint)?;
    for endpoint in
        std::iter::once(&metadata.token_endpoint).chain(metadata.registration_endpoint.as_ref())
    {
        if &parse(endpoint)?.origin() != gateway {
            return Err(
                "Managed OAuth token/registration endpoint is not on the approved gateway"
                    .to_string(),
            );
        }
    }
    if let Some(methods) = &metadata.code_challenge_methods_supported {
        if !methods.iter().any(|m| m == "S256") {
            return Err("Authorization server does not support PKCE S256".to_string());
        }
    }
    Ok(())
}

pub async fn register(
    sender: &ScopedSender,
    metadata: &AuthServerMetadata,
    redirect_uri: &str,
    scopes: &[String],
) -> Result<(String, Option<String>), String> {
    let endpoint = metadata.registration_endpoint.as_deref().ok_or_else(|| {
        "Authorization server does not support dynamic client registration".to_string()
    })?;
    let endpoint =
        url::Url::parse(endpoint).map_err(|_| "Invalid registration endpoint".to_string())?;
    let mut body = serde_json::json!({
        "client_name": "Refact",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    if !scopes.is_empty() {
        body["scope"] = serde_json::Value::String(scopes.join(" "));
    }
    let response = sender
        .send(reqwest::Method::POST, &endpoint, None, Some(&body))
        .await?;
    if !response.status().is_success() {
        return Err(redacted_failure("Client registration", response).await);
    }
    let value: serde_json::Value = read_json_bounded(response)
        .await
        .map_err(|_| "Invalid registration response".to_string())?;
    let field = |k: &str| {
        value
            .get(k)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty() && s.len() <= MAX_TOKEN_LEN)
            .map(str::to_string)
    };
    let client_id =
        field("client_id").ok_or_else(|| "Registration response has no client_id".to_string())?;
    Ok((client_id, field("client_secret")))
}

pub fn authorization_url(
    metadata: &AuthServerMetadata,
    client_id: &str,
    redirect_uri: &str,
    resource: &str,
    scopes: &[String],
) -> Result<(String, String, String), String> {
    let (challenge, verifier) = oauth2::PkceCodeChallenge::new_random_sha256();
    let state = oauth2::CsrfToken::new_random();
    let mut u = url::Url::parse(&metadata.authorization_endpoint)
        .map_err(|_| "Invalid authorization endpoint".to_string())?;
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state.secret())
            .append_pair("code_challenge", challenge.as_str())
            .append_pair("code_challenge_method", "S256")
            .append_pair("resource", resource);
        if !scopes.is_empty() {
            q.append_pair("scope", &scopes.join(" "));
        }
    }
    Ok((
        u.to_string(),
        state.secret().to_string(),
        verifier.secret().to_string(),
    ))
}

#[derive(Deserialize)]
struct TokenBody {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
}

async fn token_request(
    sender: &ScopedSender,
    token_endpoint: &str,
    params: &[(&str, String)],
    what: &str,
) -> Result<TokenBody, String> {
    let endpoint =
        url::Url::parse(token_endpoint).map_err(|_| "Invalid token endpoint".to_string())?;
    let response = sender
        .send(reqwest::Method::POST, &endpoint, Some(params), None)
        .await?;
    if !response.status().is_success() {
        return Err(redacted_failure(what, response).await);
    }
    let body: TokenBody = read_json_bounded(response)
        .await
        .map_err(|_| format!("{} returned an invalid token response", what))?;
    if body.access_token.is_empty()
        || body.access_token.len() > MAX_TOKEN_LEN
        || body
            .refresh_token
            .as_ref()
            .map(|r| r.len() > MAX_TOKEN_LEN)
            .unwrap_or(false)
    {
        return Err(format!("{} returned an invalid token response", what));
    }
    Ok(body)
}

fn into_tokens(
    body: TokenBody,
    client_id: &str,
    old_refresh: &str,
    old_scopes: &[String],
    bound_url: &str,
) -> super::MCPOAuthTokens {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let scopes: Vec<String> = body
        .scope
        .as_deref()
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| old_scopes.to_vec());
    super::MCPOAuthTokens {
        access_token: body.access_token,
        refresh_token: body
            .refresh_token
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| old_refresh.to_string()),
        expires_at: body
            .expires_in
            .filter(|s| *s > 0 && *s <= MAX_EXPIRES_IN_SECS)
            .map(|s| now_ms.saturating_add(s as i64 * 1000))
            .unwrap_or(0),
        client_id: client_id.to_string(),
        client_secret: None,
        scopes,
        bound_url: Some(bound_url.to_string()),
    }
}

pub struct CodeExchange<'a> {
    pub token_endpoint: &'a str,
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub redirect_uri: &'a str,
    pub resource: &'a str,
    pub code: &'a str,
    pub verifier: &'a str,
    pub scopes: &'a [String],
}

pub async fn exchange_code(
    sender: &ScopedSender,
    req: CodeExchange<'_>,
) -> Result<super::MCPOAuthTokens, String> {
    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", req.code.to_string()),
        ("redirect_uri", req.redirect_uri.to_string()),
        ("client_id", req.client_id.to_string()),
        ("code_verifier", req.verifier.to_string()),
        ("resource", req.resource.to_string()),
    ];
    if let Some(secret) = req.client_secret {
        params.push(("client_secret", secret.to_string()));
    }
    let body = token_request(sender, req.token_endpoint, &params, "Token exchange").await?;
    let mut tokens = into_tokens(body, req.client_id, "", req.scopes, req.resource);
    tokens.client_secret = req.client_secret.map(str::to_string);
    Ok(tokens)
}

pub async fn refresh(
    sender: &ScopedSender,
    mcp_url: &str,
    tokens: &super::MCPOAuthTokens,
) -> Result<super::MCPOAuthTokens, String> {
    if tokens.refresh_token.is_empty() {
        return Err("No refresh token; re-authentication required".to_string());
    }
    let resource = url::Url::parse(mcp_url).map_err(|_| "Invalid gateway MCP URL".to_string())?;
    let metadata = discover(sender, &resource).await?;
    let mut params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", tokens.refresh_token.clone()),
        ("client_id", tokens.client_id.clone()),
        ("resource", mcp_url.to_string()),
    ];
    if !tokens.scopes.is_empty() {
        params.push(("scope", tokens.scopes.join(" ")));
    }
    if let Some(secret) = tokens.client_secret.as_deref() {
        params.push(("client_secret", secret.to_string()));
    }
    let body = token_request(sender, &metadata.token_endpoint, &params, "Token refresh").await?;
    let mut refreshed = into_tokens(
        body,
        &tokens.client_id,
        &tokens.refresh_token,
        &tokens.scopes,
        mcp_url,
    );
    refreshed.client_secret = tokens.client_secret.clone();
    Ok(refreshed)
}

use std::sync::OnceLock;
use std::time::SystemTime;
use tokio::sync::Mutex as AMutex;

const PENDING_TTL: Duration = Duration::from_secs(600);

pub fn registry_resolver(config_path: &str) -> HeaderResolver {
    let config_path = config_path.to_string();
    Arc::new(move |target: String| {
        let config_path = config_path.clone();
        Box::pin(async move {
            crate::providers::litellm_mcp::gateway_headers(&config_path, &target).await
        })
    })
}

pub async fn managed_identity(config_path: &str) -> Result<Option<String>, String> {
    crate::providers::litellm_mcp::gateway_identity(config_path).await
}

pub async fn revalidate_binding(
    config_path: &str,
    expected_identity: &str,
    mcp_url: &str,
) -> Result<(), String> {
    match managed_identity(config_path).await? {
        Some(current) if current == expected_identity => {}
        Some(_) => return Err("LiteLLM identity changed during sign-in; start again".to_string()),
        None => return Err("Managed LiteLLM connection is no longer active".to_string()),
    }
    match crate::providers::litellm_mcp::gateway_transport_url(config_path).await? {
        Some(current_resource) if current_resource == mcp_url => Ok(()),
        _ => Err("Managed MCP server changed during sign-in; start again".to_string()),
    }
}

struct PendingGatewayFlow {
    config_path: String,
    mcp_url: String,
    identity: String,
    client_id: String,
    client_secret: Option<String>,
    token_endpoint: String,
    redirect_uri: String,
    verifier: String,
    scopes: Vec<String>,
    created_at: SystemTime,
}

fn pending() -> &'static AMutex<HashMap<String, PendingGatewayFlow>> {
    static P: OnceLock<AMutex<HashMap<String, PendingGatewayFlow>>> = OnceLock::new();
    P.get_or_init(|| AMutex::new(HashMap::new()))
}

fn by_state() -> &'static AMutex<HashMap<String, String>> {
    static S: OnceLock<AMutex<HashMap<String, String>>> = OnceLock::new();
    S.get_or_init(|| AMutex::new(HashMap::new()))
}

async fn purge_expired() {
    let mut sessions = pending().lock().await;
    sessions.retain(|_, f| {
        f.created_at
            .elapsed()
            .map(|a| a < PENDING_TTL)
            .unwrap_or(false)
    });
    let live: std::collections::HashSet<String> = sessions.keys().cloned().collect();
    drop(sessions);
    by_state().lock().await.retain(|_, id| live.contains(id));
}

pub async fn start_flow(
    config_path: &str,
    identity: &str,
    mcp_url: &str,
    scopes: &[String],
    redirect_uri: &str,
) -> Result<(String, String), String> {
    start_flow_with(
        config_path,
        identity,
        mcp_url,
        scopes,
        redirect_uri,
        registry_resolver(config_path),
    )
    .await
}

pub(crate) async fn start_flow_with(
    config_path: &str,
    identity: &str,
    mcp_url: &str,
    scopes: &[String],
    redirect_uri: &str,
    resolver: HeaderResolver,
) -> Result<(String, String), String> {
    purge_expired().await;
    let resource = url::Url::parse(mcp_url).map_err(|_| "Invalid gateway MCP URL".to_string())?;
    let sender = ScopedSender::new(mcp_url, resolver)?;
    let metadata = discover(&sender, &resource).await?;
    let effective_scopes = if scopes.is_empty() {
        metadata.scopes_supported.clone().unwrap_or_default()
    } else {
        scopes.to_vec()
    };
    let (client_id, client_secret) =
        register(&sender, &metadata, redirect_uri, &effective_scopes).await?;
    let (auth_url, state, verifier) = authorization_url(
        &metadata,
        &client_id,
        redirect_uri,
        mcp_url,
        &effective_scopes,
    )?;
    let session_id = uuid::Uuid::new_v4().to_string();
    pending().lock().await.insert(
        session_id.clone(),
        PendingGatewayFlow {
            config_path: config_path.to_string(),
            mcp_url: mcp_url.to_string(),
            identity: identity.to_string(),
            client_id,
            client_secret,
            token_endpoint: metadata.token_endpoint,
            redirect_uri: redirect_uri.to_string(),
            verifier,
            scopes: effective_scopes,
            created_at: SystemTime::now(),
        },
    );
    by_state().lock().await.insert(state, session_id.clone());
    Ok((session_id, auth_url))
}

pub async fn find_session_by_state(state: &str) -> Option<String> {
    by_state().lock().await.get(state).cloned()
}

pub async fn has_session(session_id: &str) -> bool {
    pending().lock().await.contains_key(session_id)
}

pub async fn cancel(session_id: &str) -> bool {
    let removed = pending().lock().await.remove(session_id).is_some();
    by_state().lock().await.retain(|_, id| id != session_id);
    removed
}

pub async fn cancel_for_config(config_path: &str) {
    let mut sessions = pending().lock().await;
    sessions.retain(|_, flow| flow.config_path != config_path);
    let live: std::collections::HashSet<String> = sessions.keys().cloned().collect();
    drop(sessions);
    by_state().lock().await.retain(|_, id| live.contains(id));
}

pub async fn complete_flow(
    session_id: &str,
    code: &str,
) -> Result<(super::MCPOAuthTokens, String), String> {
    complete_flow_with(session_id, code, None).await
}

pub(crate) async fn complete_flow_with(
    session_id: &str,
    code: &str,
    test_resolver: Option<HeaderResolver>,
) -> Result<(super::MCPOAuthTokens, String), String> {
    let flow = pending()
        .lock()
        .await
        .remove(session_id)
        .ok_or_else(|| "OAuth session expired or not found".to_string())?;
    by_state().lock().await.retain(|_, id| id != session_id);
    if flow
        .created_at
        .elapsed()
        .map(|a| a >= PENDING_TTL)
        .unwrap_or(true)
    {
        return Err("OAuth session expired; start again".to_string());
    }
    let is_test = test_resolver.is_some();
    if !is_test {
        revalidate_binding(&flow.config_path, &flow.identity, &flow.mcp_url).await?;
    }
    let resolver = test_resolver.unwrap_or_else(|| registry_resolver(&flow.config_path));
    let sender = ScopedSender::new(&flow.mcp_url, resolver)?;
    let tokens = exchange_code(
        &sender,
        CodeExchange {
            token_endpoint: &flow.token_endpoint,
            client_id: &flow.client_id,
            client_secret: flow.client_secret.as_deref(),
            redirect_uri: &flow.redirect_uri,
            resource: &flow.mcp_url,
            code,
            verifier: &flow.verifier,
            scopes: &flow.scopes,
        },
    )
    .await?;
    if !is_test {
        crate::providers::litellm_mcp::persist_gateway_tokens(
            &flow.config_path,
            &flow.identity,
            &flow.mcp_url,
            &tokens,
        )
        .await?;
    }
    Ok((tokens, flow.config_path))
}

fn refresh_locks() -> &'static AMutex<HashMap<String, Arc<AMutex<()>>>> {
    static L: OnceLock<AMutex<HashMap<String, Arc<AMutex<()>>>>> = OnceLock::new();
    L.get_or_init(|| AMutex::new(HashMap::new()))
}

pub async fn current_gateway_access_token(
    config_path: &str,
    url: &str,
) -> Result<Option<String>, String> {
    let Some(identity) = managed_identity(config_path).await? else {
        return Ok(None);
    };
    let Some(tokens) = super::load_tokens_from_config(config_path)
        .await
        .filter(|t| !t.access_token.is_empty())
    else {
        return Ok(None);
    };
    let transport_url = crate::providers::litellm_mcp::gateway_transport_url(config_path)
        .await?
        .ok_or_else(|| "Managed LiteLLM connection is no longer active".to_string())?;
    if tokens.bound_url.as_deref() != Some(transport_url.as_str()) || url != transport_url {
        return Err(
            "Stored OAuth session belongs to a different MCP resource; sign in again".to_string(),
        );
    }
    if !super::needs_refresh(&tokens) {
        return Ok(Some(tokens.access_token));
    }
    let lock = refresh_locks()
        .lock()
        .await
        .entry(config_path.to_string())
        .or_default()
        .clone();
    let _guard = lock.lock().await;
    let tokens = super::load_tokens_from_config(config_path)
        .await
        .filter(|t| !t.access_token.is_empty())
        .ok_or_else(|| "OAuth session was signed out".to_string())?;
    if !super::needs_refresh(&tokens) {
        return Ok(Some(tokens.access_token));
    }
    if tokens.refresh_token.is_empty() {
        return if super::is_token_expired(&tokens) {
            Err("OAuth session expired; sign in again".to_string())
        } else {
            Ok(Some(tokens.access_token))
        };
    }
    let sender = ScopedSender::new(&transport_url, registry_resolver(config_path))?;
    match refresh(&sender, &transport_url, &tokens).await {
        Ok(new_tokens) => {
            crate::providers::litellm_mcp::persist_gateway_tokens(
                config_path,
                &identity,
                &transport_url,
                &new_tokens,
            )
            .await?;
            Ok(Some(new_tokens.access_token))
        }
        Err(e) if !super::is_token_expired(&tokens) && !super::is_permanent_oauth_error(&e) => {
            Ok(Some(tokens.access_token))
        }
        Err(e) => Err(e),
    }
}

pub async fn probe_gateway_auth(
    config_path: &str,
    url: &str,
) -> Result<Option<super::MCPAuthProbeResult>, String> {
    if managed_identity(config_path).await?.is_none() {
        return Ok(None);
    }
    probe_with(url, registry_resolver(config_path))
        .await
        .map(Some)
}

pub(crate) async fn probe_with(
    url: &str,
    resolver: HeaderResolver,
) -> Result<super::MCPAuthProbeResult, String> {
    let target = url::Url::parse(url).map_err(|_| "Invalid gateway MCP URL".to_string())?;
    let sender = ScopedSender::new(url, resolver)?;
    let mut request = sender
        .client
        .post(target.clone())
        .header(reqwest::header::ACCEPT, "application/json, text/event-stream")
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "refact-auth-probe", "version": "0.1"}}
        }));
    for (name, value) in sender.headers_for(&target).await? {
        request = request.header(name, value);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "Managed MCP probe failed".to_string())?;
    let status = response.status();
    let needs_auth =
        status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN;
    let www_authenticate = response
        .headers()
        .get_all(reqwest::header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(super::parse_www_authenticate);
    drop(response);
    let (oauth_available, suggested_scopes, discovery_error) = if needs_auth {
        match discover(&sender, &target).await {
            Ok(m) => (true, m.scopes_supported.unwrap_or_default(), None),
            Err(error) => (false, vec![], Some(error)),
        }
    } else {
        (false, vec![], None)
    };
    Ok(super::MCPAuthProbeResult {
        needs_auth,
        oauth_available,
        suggested_scopes,
        www_authenticate,
        http_status: Some(status.as_u16()),
        discovery_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State as AxState;
    use axum::http::{HeaderMap, Uri};
    use axum::Router;
    use std::sync::Mutex as SMutex;

    type Seen = Arc<SMutex<Vec<(String, Option<String>)>>>;

    struct Fake {
        base: String,
        seen: Seen,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn serve(handler: fn(&str, &str) -> (u16, String)) -> Fake {
        let seen: Seen = Arc::new(SMutex::new(Vec::new()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let base_for = base.clone();
        let app = Router::new()
            .fallback(
                move |AxState(seen): AxState<Seen>, uri: Uri, headers: HeaderMap| {
                    let base = base_for.clone();
                    async move {
                        let key = headers
                            .get("x-litellm-api-key")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string);
                        seen.lock().unwrap().push((uri.path().to_string(), key));
                        let (status, body) = handler(&base, uri.path());
                        axum::response::Response::builder()
                            .status(status)
                            .header("content-type", "application/json")
                            .body(axum::body::Body::from(body))
                            .unwrap()
                    }
                },
            )
            .with_state(seen.clone());
        let task = tokio::spawn(async move {
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(app.into_make_service())
                .await
                .unwrap();
        });
        Fake {
            base,
            seen,
            task,
        }
    }

    fn resolver_for(key: &'static str) -> HeaderResolver {
        Arc::new(move |_url: String| {
            Box::pin(async move {
                Ok(Some(HashMap::from([(
                    "x-litellm-api-key".to_string(),
                    format!("Bearer {key}"),
                )])))
            })
        })
    }

    fn gateway(base: &str, path: &str) -> (u16, String) {
        match path {
            "/.well-known/oauth-protected-resource/srv/mcp" => (200, format!(r#"{{"resource":"{base}/srv/mcp","authorization_servers":["{base}"]}}"#)),
            "/.well-known/oauth-authorization-server" => (200, format!(
                r#"{{"issuer":"{base}","authorization_endpoint":"https://idp.example/authorize","token_endpoint":"{base}/srv/token","registration_endpoint":"{base}/srv/register","code_challenge_methods_supported":["S256"],"scopes_supported":["mcp"]}}"#
            )),
            "/srv/register" => (201, r#"{"client_id":"cid-1"}"#.to_string()),
            "/srv/token" => (200, r#"{"access_token":"at-1","refresh_token":"rt-1","expires_in":18446744073709551615,"token_type":"Bearer"}"#.to_string()),
            _ => (404, "{}".to_string()),
        }
    }

    fn idp(_base: &str, _path: &str) -> (u16, String) {
        (200, r#"{"ok":true}"#.to_string())
    }

    fn mcp_issuer_gateway(base: &str, path: &str) -> (u16, String) {
        let prefix = if path.contains("/proxy/") {
            "/proxy"
        } else {
            ""
        };
        if path == format!("{prefix}/srv/mcp") {
            return (401, "{}".into());
        }
        if path == format!("/.well-known/oauth-protected-resource{prefix}/srv/mcp") {
            return (
                200,
                format!(
                    r#"{{"resource":"{base}{prefix}/srv/mcp","authorization_servers":["{base}{prefix}/mcp"],"scopes_supported":["openid","offline"]}}"#
                ),
            );
        }
        if path == format!("/.well-known/oauth-authorization-server{prefix}/mcp") {
            return (
                200,
                format!(
                    r#"{{"issuer":"{base}{prefix}/mcp","authorization_endpoint":"{base}{prefix}/authorize/mcp-session","token_endpoint":"{base}{prefix}/token","registration_endpoint":"{base}{prefix}/register","code_challenge_methods_supported":["S256"],"scopes_supported":[]}}"#
                ),
            );
        }
        match path {
            "/register" | "/proxy/register" => (201, r#"{"client_id":"gateway-client"}"#.into()),
            "/token" | "/proxy/token" => (200, r#"{"access_token":"gateway-access","refresh_token":"gateway-refresh","expires_in":3600}"#.into()),
            _ => (404, "{}".into()),
        }
    }

    #[tokio::test]
    async fn managed_mcp_issuer_supports_probe_login_and_exchange_with_ingress() {
        for prefix in ["", "/proxy"] {
            let gw = serve(mcp_issuer_gateway).await;
            let mcp_url = format!("{}{prefix}/srv/mcp", gw.base);
            let probe = probe_with(&mcp_url, resolver_for("gw-secret"))
                .await
                .unwrap();
            assert_eq!(probe.http_status, Some(401));
            assert!(probe.needs_auth && probe.oauth_available);
            assert_eq!(probe.discovery_error, None);
            assert_eq!(probe.suggested_scopes, ["openid", "offline"]);
            let (session, auth_url) = start_flow_with(
                "/tmp/managed-mcp-issuer.yaml",
                "identity",
                &mcp_url,
                &[],
                "http://127.0.0.1:1/callback",
                resolver_for("gw-secret"),
            )
            .await
            .unwrap();
            let auth_url = url::Url::parse(&auth_url).unwrap();
            assert_eq!(auth_url.path(), format!("{prefix}/authorize/mcp-session"));
            let query: HashMap<_, _> = auth_url.query_pairs().into_owned().collect();
            assert_eq!(query["resource"], mcp_url);
            assert_eq!(query["scope"], "openid offline");
            assert_eq!(query["code_challenge_method"], "S256");
            let (tokens, _) = complete_flow_with(&session, "code", Some(resolver_for("gw-secret")))
                .await
                .unwrap();
            assert_eq!(tokens.access_token, "gateway-access");
            assert_eq!(tokens.bound_url.as_deref(), Some(mcp_url.as_str()));
            let seen = gw.seen.lock().unwrap().clone();
            for expected in [
                format!("/.well-known/oauth-authorization-server{prefix}/mcp"),
                format!("{prefix}/register"),
                format!("{prefix}/token"),
            ] {
                assert!(
                    seen.iter().any(|(path, key)| path == &expected
                        && key.as_deref() == Some("Bearer gw-secret")),
                    "{expected}"
                );
            }
        }
    }

    #[tokio::test]
    async fn managed_probe_preserves_resource_mismatch_without_reflecting_metadata() {
        let gw = serve(|_, path| match path {
            "/srv/mcp" => (401, "{}".into()),
            "/.well-known/oauth-protected-resource/srv/mcp" => (200, r#"{"resource":"https://untrusted.example/secret-value/srv/mcp","authorization_servers":["https://untrusted.example/mcp"]}"#.into()),
            _ => panic!("unexpected metadata request: {path}"),
        }).await;
        let probe = probe_with(&format!("{}/srv/mcp", gw.base), resolver_for("gw-secret"))
            .await
            .unwrap();
        assert!(probe.needs_auth && !probe.oauth_available);
        let expected = "Resource metadata does not match the managed MCP server; configure MCP Gateway URL to use the gateway's canonical MCP host";
        assert_eq!(probe.discovery_error.as_deref(), Some(expected));
        assert_eq!(
            probe.failure_message(),
            format!(
                "Server requires authentication (HTTP 401/403); OAuth discovery failed: {expected}"
            )
        );
        assert_eq!(gw.seen.lock().unwrap().len(), 2);
        assert!(!serde_json::to_string(&probe)
            .unwrap()
            .contains("untrusted.example"));
    }

    #[tokio::test]
    async fn managed_probe_distinguishes_invalid_metadata_http_errors_and_missing_metadata() {
        for (handler, expected) in [
            (
                (|_, path| {
                    if path == "/srv/mcp" {
                        (401, "{}".into())
                    } else {
                        (200, "not json secret-value".into())
                    }
                }) as fn(&str, &str) -> (u16, String),
                "Invalid OAuth response",
            ),
            (
                (|_, path| {
                    if path == "/srv/mcp" {
                        (401, "{}".into())
                    } else {
                        (503, "secret-value".into())
                    }
                }),
                "Managed OAuth metadata request failed: HTTP 503",
            ),
            (
                (|_, path| {
                    if path == "/srv/mcp" {
                        (401, "{}".into())
                    } else {
                        (404, "{}".into())
                    }
                }),
                "OAuth metadata not found for managed MCP server",
            ),
        ] {
            let gw = serve(handler).await;
            let probe = probe_with(&format!("{}/srv/mcp", gw.base), resolver_for("gw-secret"))
                .await
                .unwrap();
            assert!(probe.needs_auth && !probe.oauth_available);
            assert_eq!(probe.discovery_error.as_deref(), Some(expected));
            assert!(!probe.failure_message().contains("secret-value"));
            assert!(!probe.failure_message().contains("configure a bearer token"));
        }
    }

    #[tokio::test]
    async fn managed_discovery_tries_ingress_fallback_after_failed_candidate() {
        let gw = serve(|base, path| match path {
            "/.well-known/oauth-protected-resource/proxy/srv/mcp" => (401, "{}".into()),
            "/proxy/.well-known/oauth-protected-resource/srv/mcp" => (200, format!(r#"{{"resource":"{base}/proxy/srv/mcp","authorization_servers":["{base}/proxy/mcp"]}}"#)),
            "/.well-known/oauth-authorization-server/proxy/mcp" => (200, "not json".into()),
            "/proxy/.well-known/oauth-authorization-server/mcp" => (200, format!(r#"{{"issuer":"{base}/proxy/mcp","authorization_endpoint":"{base}/proxy/authorize/mcp-session","token_endpoint":"{base}/proxy/token","code_challenge_methods_supported":["S256"]}}"#)),
            _ => (404, "{}".into()),
        }).await;
        let resource = url::Url::parse(&format!("{}/proxy/srv/mcp", gw.base)).unwrap();
        let sender = ScopedSender::new(resource.as_str(), resolver_for("gw-secret")).unwrap();
        let metadata = discover(&sender, &resource).await.unwrap();
        assert_eq!(metadata.token_endpoint, format!("{}/proxy/token", gw.base));
        let paths: Vec<_> = gw
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|(path, _)| path.clone())
            .collect();
        assert_eq!(
            paths,
            [
                "/.well-known/oauth-protected-resource/proxy/srv/mcp",
                "/proxy/.well-known/oauth-protected-resource/srv/mcp",
                "/.well-known/oauth-authorization-server/proxy/mcp",
                "/proxy/.well-known/oauth-authorization-server/mcp",
            ]
        );
    }

    #[tokio::test]
    async fn managed_flow_sends_gateway_key_only_to_gateway() {
        let gw = serve(gateway).await;
        let mcp_url = format!("{}/srv/mcp", gw.base);
        let (session_id, auth_url) = start_flow_with(
            "/tmp/managed.yaml",
            "ident",
            &mcp_url,
            &[],
            "http://127.0.0.1:1/cb",
            resolver_for("gw-secret"),
        )
        .await
        .unwrap();
        assert!(auth_url.starts_with("https://idp.example/authorize?"));
        assert!(auth_url.contains("code_challenge_method=S256"));
        assert!(!auth_url.contains("gw-secret"));
        let state = url::Url::parse(&auth_url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .to_string();
        assert_eq!(
            find_session_by_state(&state).await.as_deref(),
            Some(session_id.as_str())
        );
        let (tokens, path) =
            complete_flow_with(&session_id, "code-1", Some(resolver_for("gw-secret")))
                .await
                .unwrap();
        assert_eq!(path, "/tmp/managed.yaml");
        assert_eq!(tokens.access_token, "at-1");
        assert_eq!(tokens.bound_url.as_deref(), Some(mcp_url.as_str()));
        assert_eq!(
            tokens.expires_at, 0,
            "absurd expires_in must not overflow or be trusted"
        );
        assert!(
            find_session_by_state(&state).await.is_none(),
            "state is one-time"
        );
        assert!(
            complete_flow_with(&session_id, "code-1", Some(resolver_for("gw-secret")))
                .await
                .is_err()
        );
        let seen = gw.seen.lock().unwrap().clone();
        for p in ["/srv/register", "/srv/token"] {
            assert!(
                seen.iter()
                    .any(|(path, key)| path == p && key.as_deref() == Some("Bearer gw-secret")),
                "{p} missing key"
            );
        }
    }

    #[tokio::test]
    async fn scoped_sender_never_sends_key_off_origin() {
        let gw = serve(gateway).await;
        let ext = serve(idp).await;
        let sender =
            ScopedSender::new(&format!("{}/srv/mcp", gw.base), resolver_for("gw-secret")).unwrap();
        let target = url::Url::parse(&format!("{}/token", ext.base)).unwrap();
        sender
            .send(
                reqwest::Method::POST,
                &target,
                Some(&[("a", "b".to_string())]),
                None,
            )
            .await
            .unwrap();
        let seen = ext.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].1, None);
    }

    #[tokio::test]
    async fn discovery_rejects_external_authorization_server_and_foreign_resource() {
        let ext = serve(idp).await;
        let ext_base = ext.base.clone();
        let gw = serve(|base, path| match path {
            "/.well-known/oauth-protected-resource/srv/mcp" => (200, format!(r#"{{"resource":"{base}/srv/mcp","authorization_servers":["http://127.0.0.2:9"]}}"#)),
            _ => (404, "{}".to_string()),
        })
        .await;
        let sender =
            ScopedSender::new(&format!("{}/srv/mcp", gw.base), resolver_for("gw-secret")).unwrap();
        let err = discover(
            &sender,
            &url::Url::parse(&format!("{}/srv/mcp", gw.base)).unwrap(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("not on the approved gateway"));
        let foreign = serve(|_b, path| match path {
            "/.well-known/oauth-protected-resource/srv/mcp" => (
                200,
                r#"{"resource":"https://other.example/mcp"}"#.to_string(),
            ),
            _ => (404, "{}".to_string()),
        })
        .await;
        let sender =
            ScopedSender::new(&format!("{}/srv/mcp", foreign.base), resolver_for("k")).unwrap();
        assert!(discover(
            &sender,
            &url::Url::parse(&format!("{}/srv/mcp", foreign.base)).unwrap()
        )
        .await
        .is_err());
        assert!(
            ext.seen.lock().unwrap().is_empty(),
            "external IdP {ext_base} must not be contacted"
        );
    }

    #[tokio::test]
    async fn token_errors_are_redacted_and_bodies_bounded() {
        let gw = serve(|_b, path| match path {
            "/t" => (
                400,
                r#"{"error":"secret-shaped-value-abc123","error_description":"leak"}"#.to_string(),
            ),
            "/big" => (
                200,
                format!(
                    r#"{{"access_token":"{}"}}"#,
                    "a".repeat(MAX_BODY_BYTES + 10)
                ),
            ),
            _ => (404, "{}".to_string()),
        })
        .await;
        let sender = ScopedSender::new(&gw.base, resolver_for("k")).unwrap();
        let err = token_request(&sender, &format!("{}/t", gw.base), &[], "Token exchange")
            .await
            .err()
            .unwrap();
        assert_eq!(err, "Token exchange failed: HTTP 400");
        assert!(
            token_request(&sender, &format!("{}/big", gw.base), &[], "Token exchange")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn stale_binding_blocks_requests() {
        let gw = serve(gateway).await;
        let stale: HeaderResolver =
            Arc::new(|_u: String| Box::pin(async { Err("LiteLLM identity changed".to_string()) }));
        let sender = ScopedSender::new(&gw.base, stale).unwrap();
        let target = url::Url::parse(&format!("{}/srv/token", gw.base)).unwrap();
        assert!(sender
            .send(reqwest::Method::POST, &target, None, None)
            .await
            .is_err());
        assert!(gw.seen.lock().unwrap().is_empty());
    }
}
