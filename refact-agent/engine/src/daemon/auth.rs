use axum::http::{header, Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub(crate) const DAEMON_AUTH_COOKIE: &str = "refact_daemon_auth";
pub(crate) const DAEMON_AUTH_QUERY: &str = "daemon_token";

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
        || matching_daemon_query_token(req.uri().query(), expected).is_some()
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
            .map(|value| daemon_cookie_token_matches(value, expected))
            .unwrap_or(false)
    })
}

fn daemon_cookie_token_matches(value: &str, expected: &str) -> bool {
    value.split(';').any(|cookie| {
        let Some((name, token)) = cookie.trim().split_once('=') else {
            return false;
        };
        name.trim() == DAEMON_AUTH_COOKIE && token_matches(token.trim(), expected)
    })
}

pub(crate) fn matching_daemon_query_token(query: Option<&str>, expected: &str) -> Option<String> {
    query.and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(name, token)| name == DAEMON_AUTH_QUERY && token_matches(token, expected))
            .map(|(_, token)| token.into_owned())
    })
}

pub(crate) async fn check<B>(token: Option<String>, req: Request<B>, next: Next<B>) -> Response
where
    B: Send + 'static,
{
    if let Some(expected) = token {
        if !(req.method() == Method::GET && req.uri().path() == "/daemon/v1/status") {
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
    fn auth_middleware_accepts_cookie() {
        let request = Request::builder()
            .uri("/daemon/v1/projects")
            .header(
                header::COOKIE,
                "theme=dark; refact_daemon_auth=secret-token",
            )
            .body(Body::empty())
            .unwrap();

        assert!(request_authorized(&request, "secret-token"));
    }

    #[test]
    fn auth_middleware_accepts_query_token() {
        assert!(request_authorized(
            &request("/daemon/v1/projects?daemon_token=secret-token"),
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
            .uri("/daemon/v1/projects?daemon_token=secret-token")
            .header(header::AUTHORIZATION, "Bearer wrong-bearer")
            .header(header::COOKIE, "refact_daemon_auth=secret-token")
            .body(Body::empty())
            .unwrap();

        assert!(request_authorized(&request, "secret-token"));
    }
}
