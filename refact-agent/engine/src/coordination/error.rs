use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkFailureCause {
    SnapshotRowCapacity,
    SnapshotByteCapacity,
    ScopedSourceUnavailable,
    PolicyUnavailable,
    SettingsUnavailable,
    ProvenanceCapacity,
    SnapshotEncodingUnavailable,
    ProjectionUnavailable,
}

/// Safe public errors: details from filesystem, policy or peer state are never
/// interpolated here. The same envelope is used by tools and HTTP.
#[derive(Debug, Clone, Serialize)]
pub struct WorkError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<WorkFailureCause>,
}

impl WorkError {
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            request_id: uuid::Uuid::new_v4().to_string(),
            cause: None,
        }
    }

    pub fn projection_failure(cause: WorkFailureCause) -> Self {
        let (code, message) = match cause {
            WorkFailureCause::SnapshotRowCapacity => (
                "unavailable",
                "requested visible snapshot exceeds its retained item capacity; narrow the request with root scope, states or an authorized path, then fetch a fresh snapshot. Reducing limit only changes page size, not whole-snapshot capacity",
            ),
            WorkFailureCause::SnapshotByteCapacity => (
                "unavailable",
                "requested visible snapshot exceeds its retained byte capacity; narrow the request with root scope, states or an authorized path, then fetch a fresh snapshot. Reducing limit only changes page size, not whole-snapshot capacity",
            ),
            WorkFailureCause::ScopedSourceUnavailable => (
                "unavailable",
                "the caller's scoped project source is unavailable; check that the project is still open and accessible, then retry with a fresh snapshot",
            ),
            WorkFailureCause::PolicyUnavailable => (
                "policy_unavailable",
                "current destination privacy policy is unavailable; restore a valid policy and model selection, then retry with a fresh snapshot",
            ),
            WorkFailureCause::SettingsUnavailable => (
                "unavailable",
                "work projection settings are unavailable or invalid; ask the local operator to validate coordination settings, then retry with a fresh snapshot",
            ),
            WorkFailureCause::SnapshotEncodingUnavailable => (
                "unavailable",
                "work snapshot could not be encoded; retry with a fresh snapshot and report request_id to the local operator if it persists",
            ),
            WorkFailureCause::ProvenanceCapacity => (
                "unavailable",
                "complete privacy/provenance cannot fit within the bounded response; provenance was not truncated or omitted. Reducing limit or repeating an unchanged request does not resolve this capacity failure; report request_id to the local operator",
            ),
            WorkFailureCause::ProjectionUnavailable => (
                "unavailable",
                "work projection is unavailable; retry with a fresh snapshot, narrow scope or filters, and report request_id to the local operator if it persists",
            ),
        };
        let mut error = Self::new(code, message, cause != WorkFailureCause::ProvenanceCapacity);
        error.cause = Some(cause);
        tracing::debug!(request_id = %error.request_id, cause = ?cause, "work projection failure");
        error
    }

    pub fn for_provenance_request(mut self, guidance: &str, attaching: bool) -> Self {
        if self.cause != Some(WorkFailureCause::ProvenanceCapacity) {
            return self;
        }
        let phase = if attaching {
            "while attaching response privacy provenance"
        } else {
            "while preparing the requested projection"
        };
        self.message = format!("Complete privacy/provenance cannot fit within the bounded response {phase}; provenance was not truncated or omitted. Reducing limit or repeating an unchanged request does not resolve this capacity failure. {guidance}");
        self.retryable = false;
        self
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
        Self::projection_failure(WorkFailureCause::PolicyUnavailable)
    }

    pub fn scope_denied() -> Self {
        Self::new("scope_denied", "requested scope is not authorized", false)
    }

    pub fn unavailable() -> Self {
        Self::projection_failure(WorkFailureCause::ProjectionUnavailable)
    }
}

impl std::fmt::Display for WorkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&serde_json::to_string(self).map_err(|_| std::fmt::Error)?)
    }
}

impl std::error::Error for WorkError {}
