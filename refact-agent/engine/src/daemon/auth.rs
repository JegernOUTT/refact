use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use serde_json::json;

use crate::daemon::config::DaemonConfig;

const BASIC_AUTH_REALM: &str = "Refact daemon";

#[derive(Clone, Default)]
pub(crate) struct DaemonAuthPolicy {
    pub token: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub hook_token: Option<String>,
    pub open_hooks_allowed: bool,
}

impl DaemonAuthPolicy {
    fn basic_credentials(&self) -> Option<(&str, &str)> {
        match (self.username.as_deref(), self.password.as_deref()) {
            (Some(username), Some(password)) if !username.is_empty() && !password.is_empty() => {
                Some((username, password))
            }
            _ => None,
        }
    }

    fn requires_auth(&self) -> bool {
        self.token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty())
            || self.basic_credentials().is_some()
    }
}

pub(crate) fn peer_is_loopback<B>(req: &Request<B>) -> bool {
    match req.extensions().get::<ConnectInfo<SocketAddr>>() {
        Some(ConnectInfo(addr)) => addr.ip().is_loopback(),
        None => true,
    }
}

pub(crate) fn origin_is_loopback(origin: &str) -> bool {
    let host = origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin);
    let host = host.split('/').next().unwrap_or(host).trim();
    let host_no_port = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host.rsplit_once(':').map(|(left, _)| left).unwrap_or(host)
    };
    host_no_port.eq_ignore_ascii_case("localhost")
        || host_no_port
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

pub(crate) fn origin_is_trusted_ide_webview(origin: &str) -> bool {
    let origin = origin.trim();
    if origin.eq_ignore_ascii_case("http://refactai")
        || origin.eq_ignore_ascii_case("https://refactai")
    {
        return true;
    }
    origin
        .split_once("://")
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("vscode-webview"))
        .map(|(_, host)| {
            !host.is_empty()
                && host
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        })
        .unwrap_or(false)
}

pub(crate) fn origin_allowed_for_loopback_or_ide(origin: &str) -> bool {
    origin_is_loopback(origin) || origin_is_trusted_ide_webview(origin)
}

fn loopback_trust_allowed<B>(req: &Request<B>) -> bool {
    if !peer_is_loopback(req) {
        return false;
    }
    match req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        None => true,
        Some(origin) if origin.eq_ignore_ascii_case("null") => false,
        Some(origin) => origin_allowed_for_loopback_or_ide(origin),
    }
}

fn basic_credentials_from_headers(headers: &HeaderMap) -> Option<(String, String)> {
    if has_multiple_authorization_headers(headers) {
        return None;
    }
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())?
        .trim();
    let mut parts = value.splitn(2, |ch: char| ch.is_ascii_whitespace());
    let scheme = parts.next()?;
    if !scheme.eq_ignore_ascii_case("Basic") {
        return None;
    }
    let encoded = parts.next()?.trim();
    let decoded = base64::prelude::BASE64_STANDARD.decode(encoded).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (username, password) = decoded.split_once(':')?;
    Some((username.to_string(), password.to_string()))
}

fn basic_auth_matches(req_headers: &HeaderMap, expected_user: &str, expected_pass: &str) -> bool {
    match basic_credentials_from_headers(req_headers) {
        Some((username, password)) => {
            token_matches(&username, expected_user) && token_matches(&password, expected_pass)
        }
        None => false,
    }
}

fn remote_authorized<B>(req: &Request<B>, policy: &DaemonAuthPolicy) -> bool {
    if let Some((username, password)) = policy.basic_credentials() {
        if basic_auth_matches(req.headers(), username, password) {
            return true;
        }
    }
    if let Some(token) = policy
        .token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
    {
        return request_authorized(req, token);
    }
    false
}

fn unauthorized_basic() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(
            header::WWW_AUTHENTICATE,
            format!("Basic realm=\"{BASIC_AUTH_REALM}\""),
        )],
        Json(json!({"error": "Unauthorized"})),
    )
        .into_response()
}

pub(crate) const DAEMON_AUTH_COOKIE: &str = "refact_daemon_auth";
pub(crate) const DAEMON_AUTH_QUERY: &str = "daemon_token";
const PROJECT_COOKIE_PREFIX: &str = "project:";
const REDACTED_DAEMON_TOKEN: &str = "<redacted>";
const REFACT_TOKEN_HEADER: &str = "x-refact-token";

pub(crate) fn generate_token() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) fn resolve_token(config_token: Option<&str>) -> String {
    config_token
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(generate_token)
}

pub(crate) fn token_matches(provided: &str, expected: &str) -> bool {
    let provided = provided.as_bytes();
    let expected = expected.as_bytes();
    let mut diff = provided.len() ^ expected.len();
    for (index, right) in expected.iter().copied().enumerate() {
        let left = provided.get(index).copied().unwrap_or(0);
        diff |= usize::from(left ^ right);
    }
    diff == 0
}

fn request_authorized<B>(req: &Request<B>, expected: &str) -> bool {
    if has_multiple_authorization_headers(req.headers()) {
        return false;
    }
    bearer_token_from_headers(req.headers())
        .map(|token| token_matches(token, expected))
        .unwrap_or(false)
        || cookie_token_matches(req, expected)
        || (query_token_allowed(req)
            && matching_daemon_query_token(req.uri().query(), expected).is_some())
}

fn hook_request_authorized<B>(req: &Request<B>, expected: &str) -> bool {
    if query_contains_daemon_token(req.uri().query()) {
        return false;
    }
    if has_multiple_authorization_headers(req.headers()) {
        return false;
    }
    bearer_token_from_headers(req.headers())
        .map(|token| token_matches(token, expected))
        .unwrap_or(false)
        || req
            .headers()
            .get(REFACT_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(|token| token_matches(token.trim(), expected))
            .unwrap_or(false)
}

pub(crate) fn bearer_token_from_headers(headers: &HeaderMap) -> Option<&str> {
    if has_multiple_authorization_headers(headers) {
        return None;
    }
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())?
        .trim();
    let mut parts = value.splitn(2, |ch: char| ch.is_ascii_whitespace());
    let scheme = parts.next()?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    let token = parts.next()?.trim();
    (!token.is_empty()).then_some(token)
}

fn has_multiple_authorization_headers(headers: &HeaderMap) -> bool {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    values.next().is_some() && values.next().is_some()
}

fn cookie_token_matches<B>(req: &Request<B>, expected: &str) -> bool {
    req.headers().get_all(header::COOKIE).iter().any(|value| {
        value
            .to_str()
            .map(|value| daemon_cookie_token_matches(value, expected, req.uri().path()))
            .unwrap_or(false)
    })
}

fn daemon_cookie_token_matches(value: &str, expected: &str, path: &str) -> bool {
    value.split(';').any(|cookie| {
        let Some((name, token)) = cookie.trim().split_once('=') else {
            return false;
        };
        name.trim() == DAEMON_AUTH_COOKIE && cookie_value_authorizes(token.trim(), expected, path)
    })
}

fn cookie_value_authorizes(value: &str, expected: &str, path: &str) -> bool {
    if token_matches(value, expected) {
        return !is_project_api_path(path);
    }
    let Some((project_id, token)) = project_cookie_parts(value) else {
        return false;
    };
    token_matches(token, expected) && project_cookie_authorizes_path(project_id, path)
}

fn project_cookie_parts(value: &str) -> Option<(&str, &str)> {
    value
        .strip_prefix(PROJECT_COOKIE_PREFIX)
        .and_then(|rest| rest.split_once(':'))
        .filter(|(project_id, token)| !project_id.is_empty() && !token.is_empty())
}

fn project_cookie_authorizes_path(project_id: &str, path: &str) -> bool {
    requested_project_id(path)
        .map(|requested| requested == project_id)
        .unwrap_or_else(|| is_static_asset_path(path))
}

fn query_token_allowed<B>(req: &Request<B>) -> bool {
    req.method() == Method::GET
        && (req.uri().path() == "/" || is_project_index_path(req.uri().path()))
}

fn is_public_request<B>(req: &Request<B>) -> bool {
    (req.method() == Method::GET && req.uri().path() == "/daemon/v1/status")
        || (req.method() == Method::GET && is_static_asset_path(req.uri().path()))
}

fn is_hook_request<B>(req: &Request<B>) -> bool {
    let path = req.uri().path();
    path == "/hooks" || path == "/hooks/" || path.starts_with("/hooks/")
}

fn is_static_asset_path(path: &str) -> bool {
    path.starts_with("/dist/chat/")
}

fn is_project_index_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/p/") else {
        return false;
    };
    let rest = rest.trim_end_matches('/');
    !rest.is_empty() && !rest.contains('/')
}

fn is_project_api_path(path: &str) -> bool {
    requested_project_id(path)
        .map(|project_id| {
            let prefix = format!("/p/{project_id}/v1");
            path == prefix || path.starts_with(&format!("{prefix}/"))
        })
        .unwrap_or(false)
}

pub(crate) fn project_cookie_value(project_id: &str, token: &str) -> String {
    format!("{PROJECT_COOKIE_PREFIX}{project_id}:{token}")
}

pub(crate) fn project_cookie_from_headers(
    headers: &HeaderMap,
    project_id: &str,
    expected: &str,
) -> Option<String> {
    if has_multiple_authorization_headers(headers) {
        return None;
    }
    if let Some(token) =
        bearer_token_from_headers(headers).filter(|token| token_matches(token, expected))
    {
        return Some(project_cookie_value(project_id, token));
    }
    headers.get_all(header::COOKIE).iter().find_map(|value| {
        value.to_str().ok().and_then(|value| {
            value.split(';').find_map(|cookie| {
                let (name, token) = cookie.trim().split_once('=')?;
                if name.trim() != DAEMON_AUTH_COOKIE {
                    return None;
                }
                let token = token.trim();
                if token_matches(token, expected) {
                    return Some(project_cookie_value(project_id, token));
                }
                let (cookie_project_id, project_token) = project_cookie_parts(token)?;
                (cookie_project_id == project_id && token_matches(project_token, expected))
                    .then(|| project_cookie_value(project_id, project_token))
            })
        })
    })
}

pub(crate) fn requested_project_id(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/p/")?;
    let project_id = rest.split('/').next().unwrap_or_default();
    (!project_id.is_empty()).then_some(project_id)
}

pub(crate) fn matching_daemon_query_token(query: Option<&str>, expected: &str) -> Option<String> {
    query.and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(name, token)| name == DAEMON_AUTH_QUERY && token_matches(token, expected))
            .map(|(_, token)| token.into_owned())
    })
}

fn query_contains_daemon_token(query: Option<&str>) -> bool {
    query
        .map(|query| {
            url::form_urlencoded::parse(query.as_bytes()).any(|(name, _)| name == DAEMON_AUTH_QUERY)
        })
        .unwrap_or(false)
}

pub(crate) fn query_without_daemon_token(query: Option<&str>) -> Option<String> {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    let mut kept = false;
    for (name, value) in url::form_urlencoded::parse(query?.as_bytes()) {
        if name == DAEMON_AUTH_QUERY {
            continue;
        }
        kept = true;
        serializer.append_pair(&name, &value);
    }
    kept.then(|| serializer.finish())
}

pub(crate) fn redact_daemon_query_token(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut cursor = 0;
    while let Some((value_start, value_end)) = next_daemon_token_value(value, cursor) {
        out.push_str(&value[cursor..value_start]);
        out.push_str(REDACTED_DAEMON_TOKEN);
        cursor = value_end;
    }
    if cursor == 0 {
        return value.to_string();
    }
    out.push_str(&value[cursor..]);
    out
}

pub(crate) fn daemon_lan_auth_ok(config: &DaemonConfig) -> bool {
    if !config.auth.enabled {
        return false;
    }
    let has_token = config
        .auth
        .token
        .as_deref()
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false);
    let has_basic = config
        .auth
        .username
        .as_deref()
        .map(|value| !value.is_empty())
        .unwrap_or(false)
        && config
            .auth
            .password
            .as_deref()
            .map(|value| !value.is_empty())
            .unwrap_or(false);
    has_token || has_basic
}

pub(crate) fn hooks_unauthenticated_allowed_for_bind(bind: &str) -> bool {
    bind.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

pub(crate) fn validate_hooks_auth_policy(
    config: &DaemonConfig,
    bind_ip: IpAddr,
) -> Result<(), String> {
    if !config.hooks.enabled || config.auth.enabled || bind_ip.is_loopback() {
        return Ok(());
    }
    if config
        .hooks
        .token
        .as_deref()
        .is_some_and(|token| !token.trim().is_empty())
    {
        return Ok(());
    }
    Err("daemon hooks without hooks.token or daemon auth are only allowed on loopback binds; set hooks.token, enable auth, or bind to 127.0.0.1/::1".to_string())
}

fn next_daemon_token_value(value: &str, from: usize) -> Option<(usize, usize)> {
    for (offset, ch) in value[from..].char_indices() {
        if ch != '=' {
            continue;
        }
        let eq = from + offset;
        let name_start = param_name_start(value, eq);
        let name = &value[name_start..eq];
        if decoded_param_name_matches(name) {
            let value_start = eq + ch.len_utf8();
            let value_end = value[value_start..]
                .find(is_token_delimiter)
                .map(|end| value_start + end)
                .unwrap_or(value.len());
            return Some((value_start, value_end));
        }
    }
    None
}

fn param_name_start(value: &str, eq: usize) -> usize {
    value[..eq]
        .char_indices()
        .rev()
        .find(|(_, ch)| is_param_start_delimiter(*ch))
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0)
}

fn decoded_param_name_matches(name: &str) -> bool {
    if name == DAEMON_AUTH_QUERY {
        return true;
    }
    let pair = format!("{name}=x");
    url::form_urlencoded::parse(pair.as_bytes())
        .next()
        .map(|(name, _)| name == DAEMON_AUTH_QUERY)
        .unwrap_or(false)
}

fn is_param_start_delimiter(ch: char) -> bool {
    matches!(
        ch,
        '?' | '&' | ' ' | '\t' | '\r' | '\n' | '"' | '\'' | '<' | '>' | '(' | '{' | '['
    )
}

fn is_token_delimiter(ch: char) -> bool {
    matches!(
        ch,
        '&' | '#' | ' ' | '\t' | '\r' | '\n' | '"' | '\'' | '<' | '>' | ')' | '}' | ']'
    )
}

pub(crate) async fn enforce<B>(policy: DaemonAuthPolicy, req: Request<B>, next: Next<B>) -> Response
where
    B: Send + 'static,
{
    let peer_loopback = loopback_trust_allowed(&req);
    if is_hook_request(&req) {
        if query_contains_daemon_token(req.uri().query()) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            )
                .into_response();
        }
        let expected = policy
            .hook_token
            .as_deref()
            .filter(|token| !token.trim().is_empty())
            .or_else(|| {
                policy
                    .token
                    .as_deref()
                    .filter(|token| !token.trim().is_empty())
            });
        match expected {
            Some(expected) => {
                if !hook_request_authorized(&req, expected) {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error": "Unauthorized"})),
                    )
                        .into_response();
                }
            }
            None if policy.open_hooks_allowed => {}
            None => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"error": "hooks require hooks.token or daemon auth on non-loopback binds"})),
                )
                    .into_response();
            }
        }
        return next.run(req).await;
    }
    if peer_loopback || is_public_request(&req) {
        return next.run(req).await;
    }
    if !policy.requires_auth() {
        return unauthorized_basic();
    }
    if !remote_authorized(&req, &policy) {
        return unauthorized_basic();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::routing::get;
    use axum::Router;
    use std::net::SocketAddr;

    fn request(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    fn request_from(peer: SocketAddr, origin: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri("/p/project/v1/ping");
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        let mut req = builder.body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(peer));
        req
    }

    fn loopback_request(origin: Option<&str>) -> Request<Body> {
        request_from(SocketAddr::from(([127, 0, 0, 1], 40000)), origin)
    }

    async fn router_response(peer: SocketAddr, origin: Option<&str>) -> StatusCode {
        use tower::ServiceExt;

        let policy = DaemonAuthPolicy {
            token: Some("secret".to_string()),
            ..Default::default()
        };
        let app = Router::new()
            .route("/p/project/v1/ping", get(|| async { StatusCode::OK }))
            .layer(axum::middleware::from_fn(move |req, next| {
                enforce(policy.clone(), req, next)
            }));
        app.oneshot(request_from(peer, origin))
            .await
            .unwrap()
            .status()
    }

    #[test]
    fn resolve_token_uses_config_when_set() {
        assert_eq!(resolve_token(Some("my-token")), "my-token");
    }

    #[test]
    fn resolve_token_generates_uuid_when_none() {
        let t = resolve_token(None);
        assert_eq!(t.len(), 36);
    }

    #[test]
    fn resolve_token_generates_when_blank() {
        let t = resolve_token(Some(""));
        assert!(!t.is_empty());
        assert_ne!(t, "");
    }

    #[test]
    fn token_matches_accepts_equal_tokens() {
        assert!(token_matches("secret-token", "secret-token"));
    }

    #[test]
    fn token_matches_rejects_unequal_tokens() {
        assert!(!token_matches("secret-token", "secret-taken"));
    }

    #[test]
    fn token_matches_rejects_length_mismatch() {
        assert!(!token_matches("secret-token", "secret-token-extra"));
        assert!(!token_matches("secret-token-extra", "secret-token"));
    }

    #[test]
    fn auth_middleware_accepts_bearer_token() {
        let request = Request::builder()
            .uri("/daemon/v1/projects")
            .header(header::AUTHORIZATION, "Bearer secret-token")
            .body(Body::empty())
            .unwrap();

        assert!(request_authorized(&request, "secret-token"));
    }

    #[test]
    fn auth_middleware_accepts_ows_and_case_insensitive_bearer_scheme() {
        let request = Request::builder()
            .uri("/daemon/v1/projects")
            .header(header::AUTHORIZATION, "  bEaReR \t secret-token  ")
            .body(Body::empty())
            .unwrap();

        assert!(request_authorized(&request, "secret-token"));
    }

    #[test]
    fn auth_middleware_rejects_multiple_authorization_headers() {
        let mut request = Request::builder()
            .uri("/daemon/v1/projects")
            .header(header::AUTHORIZATION, "Bearer secret-token")
            .header(header::COOKIE, "refact_daemon_auth=secret-token")
            .body(Body::empty())
            .unwrap();
        request.headers_mut().append(
            header::AUTHORIZATION,
            "Bearer secret-token".parse().unwrap(),
        );

        assert!(!request_authorized(&request, "secret-token"));
    }

    #[test]
    fn auth_middleware_accepts_global_cookie_for_picker_and_control_plane() {
        let request = Request::builder()
            .uri("/")
            .header(
                header::COOKIE,
                "theme=dark; refact_daemon_auth=secret-token",
            )
            .body(Body::empty())
            .unwrap();

        assert!(request_authorized(&request, "secret-token"));

        let control = Request::builder()
            .uri("/daemon/v1/projects")
            .header(header::COOKIE, "refact_daemon_auth=secret-token")
            .body(Body::empty())
            .unwrap();
        let project_api = Request::builder()
            .uri("/p/project/v1/chats")
            .header(header::COOKIE, "refact_daemon_auth=secret-token")
            .body(Body::empty())
            .unwrap();

        assert!(request_authorized(&control, "secret-token"));
        assert!(!request_authorized(&project_api, "secret-token"));
    }

    #[test]
    fn auth_middleware_accepts_query_token_for_bootstrap_only() {
        assert!(request_authorized(
            &request("/?daemon_token=secret-token"),
            "secret-token"
        ));
        assert!(request_authorized(
            &request("/p/project/?daemon_token=secret-token"),
            "secret-token"
        ));
        assert!(!request_authorized(
            &request("/daemon/v1/projects?daemon_token=secret-token"),
            "secret-token"
        ));
        assert!(!request_authorized(
            &request("/p/project/v1/chats?daemon_token=secret-token"),
            "secret-token"
        ));
    }

    #[test]
    fn auth_middleware_rejects_wrong_token_from_all_sources() {
        let request = Request::builder()
            .uri("/daemon/v1/projects?daemon_token=wrong-query")
            .header(header::AUTHORIZATION, "Bearer wrong-bearer")
            .header(header::COOKIE, "refact_daemon_auth=wrong-cookie")
            .body(Body::empty())
            .unwrap();

        assert!(!request_authorized(&request, "secret-token"));
    }

    #[test]
    fn auth_middleware_accepts_later_matching_source() {
        let request = Request::builder()
            .uri("/daemon/v1/projects?daemon_token=wrong-query")
            .header(header::AUTHORIZATION, "Bearer wrong-bearer")
            .header(header::COOKIE, "refact_daemon_auth=secret-token")
            .body(Body::empty())
            .unwrap();

        assert!(request_authorized(&request, "secret-token"));
    }

    #[test]
    fn project_cookie_authorizes_only_matching_project_and_static_assets() {
        let cookie = format!(
            "refact_daemon_auth={}",
            project_cookie_value("project-a", "secret-token")
        );
        let same_project = Request::builder()
            .uri("/p/project-a/v1/chats")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        let other_project = Request::builder()
            .uri("/p/project-b/v1/chats")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        let control_plane = Request::builder()
            .uri("/daemon/v1/projects")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        let asset = Request::builder()
            .uri("/dist/chat/style.css")
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap();

        assert!(request_authorized(&same_project, "secret-token"));
        assert!(!request_authorized(&other_project, "secret-token"));
        assert!(!request_authorized(&control_plane, "secret-token"));
        assert!(request_authorized(&asset, "secret-token"));
    }

    #[test]
    fn project_cookie_from_headers_converts_global_cookie_to_project_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "refact_daemon_auth=secret-token".parse().unwrap(),
        );

        assert_eq!(
            project_cookie_from_headers(&headers, "project-a", "secret-token").as_deref(),
            Some("project:project-a:secret-token")
        );
        assert_eq!(
            project_cookie_from_headers(&headers, "project-a", "wrong"),
            None
        );
    }

    #[test]
    fn project_cookie_from_headers_rejects_multiple_authorization_headers() {
        let mut headers = HeaderMap::new();
        headers.append(
            header::AUTHORIZATION,
            "Bearer secret-token".parse().unwrap(),
        );
        headers.append(
            header::AUTHORIZATION,
            "Bearer secret-token".parse().unwrap(),
        );

        assert_eq!(
            project_cookie_from_headers(&headers, "project-a", "secret-token"),
            None
        );
    }

    #[test]
    fn hooks_auth_policy_allows_no_token_on_loopback_only() {
        let loopback = DaemonConfig {
            bind: "127.0.0.1".to_string(),
            hooks: crate::daemon::config::HooksConfig {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let wildcard = DaemonConfig {
            hooks: crate::daemon::config::HooksConfig {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let tokenized = DaemonConfig {
            hooks: crate::daemon::config::HooksConfig {
                enabled: true,
                token: Some("hook-secret".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(hooks_unauthenticated_allowed_for_bind("127.0.0.1"));
        assert!(hooks_unauthenticated_allowed_for_bind("::1"));
        assert!(!hooks_unauthenticated_allowed_for_bind("0.0.0.0"));
        assert!(validate_hooks_auth_policy(&loopback, "127.0.0.1".parse().unwrap()).is_ok());
        assert!(validate_hooks_auth_policy(&tokenized, "0.0.0.0".parse().unwrap()).is_ok());
        assert!(validate_hooks_auth_policy(&wildcard, "0.0.0.0".parse().unwrap()).is_err());
    }

    #[test]
    fn query_without_daemon_token_removes_only_daemon_token() {
        assert_eq!(
            query_without_daemon_token(Some("a=1&daemon_token=secret&b=2")),
            Some("a=1&b=2".to_string())
        );
        assert_eq!(
            query_without_daemon_token(Some("a=1&d%61emon_token=secret&b=2")),
            Some("a=1&b=2".to_string())
        );
        assert_eq!(
            query_without_daemon_token(Some("daemon_token=secret")),
            None
        );
        assert_eq!(query_without_daemon_token(None), None);
    }

    #[test]
    fn redact_daemon_query_token_hides_query_values() {
        let redacted = redact_daemon_query_token(
            "GET http://x/p/a/v1?daemon_token=secret-token&chat=1 failed",
        );
        assert!(!redacted.contains("secret-token"));
        assert!(redacted.contains("daemon_token=<redacted>&chat=1"));

        let redacted = redact_daemon_query_token(
            "GET http://x/p/a/v1?d%61emon_token=secret-token&chat=1 failed",
        );
        assert!(!redacted.contains("secret-token"));
        assert!(redacted.contains("d%61emon_token=<redacted>&chat=1"));
    }

    #[test]
    fn origin_is_trusted_ide_webview_accepts_known_webview_origins() {
        assert!(origin_is_trusted_ide_webview("http://refactai"));
        assert!(origin_is_trusted_ide_webview("https://refactai"));
        assert!(origin_is_trusted_ide_webview("HTTP://RefactAI"));
        assert!(origin_is_trusted_ide_webview(
            "vscode-webview://11112222-3333-4444-5555-666677778888"
        ));
        assert!(origin_is_trusted_ide_webview("vscode-webview://deadBEEF"));
        assert!(origin_is_trusted_ide_webview(
            "vscode-webview://0ddd89pf9tgcs7jvdpe0qs2lctosmhsfsbhj2q7ncq6jv6cq2us4"
        ));
    }

    #[test]
    fn origin_is_trusted_ide_webview_rejects_unknown_origins() {
        assert!(!origin_is_trusted_ide_webview("http://evil.example"));
        assert!(!origin_is_trusted_ide_webview("http://refactai.evil.com"));
        assert!(!origin_is_trusted_ide_webview("vscode-webview://"));
        assert!(!origin_is_trusted_ide_webview("vscode-webview://not_hex"));
        assert!(!origin_is_trusted_ide_webview("vscode-webview://abc/../x"));
        assert!(!origin_is_trusted_ide_webview("evil-webview://abcdef"));
        assert!(!origin_is_trusted_ide_webview("null"));
        assert!(!origin_is_trusted_ide_webview("http://127.0.0.1:8488"));
    }

    #[test]
    fn origin_allowed_for_loopback_or_ide_composes_loopback_and_webview() {
        assert!(origin_allowed_for_loopback_or_ide("http://127.0.0.1:8488"));
        assert!(origin_allowed_for_loopback_or_ide("http://localhost:3000"));
        assert!(origin_allowed_for_loopback_or_ide("http://refactai"));
        assert!(origin_allowed_for_loopback_or_ide(
            "vscode-webview://11112222-3333-4444-5555-666677778888"
        ));
        assert!(!origin_allowed_for_loopback_or_ide("http://evil.example"));
        assert!(!origin_allowed_for_loopback_or_ide("null"));
    }

    #[test]
    fn loopback_trust_allows_ide_webview_origins_from_loopback_peer() {
        assert!(loopback_trust_allowed(&loopback_request(None)));
        assert!(loopback_trust_allowed(&loopback_request(Some(
            "http://refactai"
        ))));
        assert!(loopback_trust_allowed(&loopback_request(Some(
            "vscode-webview://11112222-3333-4444-5555-666677778888"
        ))));
        assert!(loopback_trust_allowed(&loopback_request(Some(
            "http://127.0.0.1:8488"
        ))));
    }

    #[test]
    fn loopback_trust_rejects_untrusted_origins_and_remote_peers() {
        assert!(!loopback_trust_allowed(&loopback_request(Some(
            "http://evil.example"
        ))));
        assert!(!loopback_trust_allowed(&loopback_request(Some("null"))));
        assert!(!loopback_trust_allowed(&request_from(
            SocketAddr::from(([192, 168, 1, 50], 40000)),
            Some("http://refactai"),
        )));
    }

    #[tokio::test]
    async fn enforce_allows_ide_webview_origins_from_loopback_peer() {
        assert_eq!(
            router_response(
                SocketAddr::from(([127, 0, 0, 1], 40000)),
                Some("http://refactai"),
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(
            router_response(
                SocketAddr::from(([127, 0, 0, 1], 40000)),
                Some("vscode-webview://11112222-3333-4444-5555-666677778888"),
            )
            .await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn enforce_rejects_untrusted_loopback_origins_and_remote_ide_origin() {
        assert_eq!(
            router_response(
                SocketAddr::from(([127, 0, 0, 1], 40000)),
                Some("http://evil.example"),
            )
            .await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            router_response(SocketAddr::from(([127, 0, 0, 1], 40000)), Some("null"),).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            router_response(
                SocketAddr::from(([192, 168, 1, 50], 40000)),
                Some("http://refactai"),
            )
            .await,
            StatusCode::UNAUTHORIZED
        );
    }
}
