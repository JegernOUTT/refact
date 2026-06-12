use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

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

pub(crate) async fn check<B>(token: Option<String>, req: Request<B>, next: Next<B>) -> Response
where
    B: Send + 'static,
{
    if let Some(expected) = token {
        // `/daemon/v1/status` stays public and read-only so local discovery and health checks can
        // verify daemon liveness before they have loaded credentials from daemon.json.
        if !(req.method() == Method::GET && req.uri().path() == "/daemon/v1/status") {
            let authorized = req
                .headers()
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(|t| token_matches(t, &expected))
                .unwrap_or(false);
            if !authorized {
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
}
