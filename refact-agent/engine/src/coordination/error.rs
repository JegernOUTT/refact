use serde::Serialize;

/// Safe public errors: details from filesystem, policy or peer state are never
/// interpolated here. The same envelope is used by tools and HTTP.
#[derive(Debug, Clone, Serialize)]
pub struct WorkError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub request_id: String,
}

impl WorkError {
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            request_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_argument", message, false)
    }

    pub fn invalid_cursor() -> Self {
        Self::new(
            "invalid_cursor",
            "cursor does not match this request",
            false,
        )
    }

    pub fn cursor_expired() -> Self {
        Self::new(
            "cursor_expired",
            "projection changed or cursor expired; fetch a fresh snapshot",
            true,
        )
    }

    pub fn not_visible() -> Self {
        Self::new(
            "not_found_or_not_visible",
            "work not found or not visible",
            false,
        )
    }

    pub fn policy_unavailable() -> Self {
        Self::new(
            "policy_unavailable",
            "current destination privacy policy is unavailable",
            true,
        )
    }

    pub fn scope_denied() -> Self {
        Self::new("scope_denied", "requested scope is not authorized", false)
    }

    pub fn unavailable() -> Self {
        Self::new("unavailable", "work projection is unavailable", true)
    }
}

impl std::fmt::Display for WorkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&serde_json::to_string(self).map_err(|_| std::fmt::Error)?)
    }
}

impl std::error::Error for WorkError {}
