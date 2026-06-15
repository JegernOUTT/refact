use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

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
    bearer_token(req)
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
    bearer_token(req)
        .map(|token| token_matches(token, expected))
        .unwrap_or(false)
        || req
            .headers()
            .get(REFACT_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(|token| token_matches(token, expected))
            .unwrap_or(false)
}

fn bearer_token<B>(req: &Request<B>) -> Option<&str> {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
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
    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| token_matches(token, expected))
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

pub(crate) fn redact_daemon_token(value: &str) -> String {
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

pub(crate) async fn check_with_hooks<B>(
    token: Option<String>,
    hook_token: Option<String>,
    req: Request<B>,
    next: Next<B>,
) -> Response
where
    B: Send + 'static,
{
    if is_hook_request(&req) {
        let expected = hook_token
            .as_deref()
            .filter(|token| !token.is_empty())
            .or_else(|| token.as_deref().filter(|token| !token.is_empty()));
        if !expected
            .map(|expected| hook_request_authorized(&req, expected))
            .unwrap_or(false)
        {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            )
                .into_response();
        }
        return next.run(req).await;
    }
    if let Some(expected) = token {
        if !is_public_request(&req) {
            if !request_authorized(&req, &expected) {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Unauthorized"})),
                )
                    .into_response();
            }
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    fn request(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
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
    fn redact_daemon_token_hides_query_values() {
        let redacted =
            redact_daemon_token("GET http://x/p/a/v1?daemon_token=secret-token&chat=1 failed");
        assert!(!redacted.contains("secret-token"));
        assert!(redacted.contains("daemon_token=<redacted>&chat=1"));

        let redacted =
            redact_daemon_token("GET http://x/p/a/v1?d%61emon_token=secret-token&chat=1 failed");
        assert!(!redacted.contains("secret-token"));
        assert!(redacted.contains("d%61emon_token=<redacted>&chat=1"));
    }
}
