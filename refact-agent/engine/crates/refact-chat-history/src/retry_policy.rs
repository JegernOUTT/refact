use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry { reason: &'static str },
    ContextLimit { reason: &'static str },
    DoNotRetry { reason: &'static str },
    UserCancelled { reason: &'static str },
}

impl RetryDecision {
    pub fn reason(self) -> &'static str {
        match self {
            RetryDecision::Retry { reason }
            | RetryDecision::ContextLimit { reason }
            | RetryDecision::DoNotRetry { reason }
            | RetryDecision::UserCancelled { reason } => reason,
        }
    }

    pub fn is_retryable_transient(self) -> bool {
        matches!(self, RetryDecision::Retry { .. })
    }

    pub fn is_context_limit(self) -> bool {
        matches!(self, RetryDecision::ContextLimit { .. })
    }

    pub fn is_user_cancelled(self) -> bool {
        matches!(self, RetryDecision::UserCancelled { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserErrorCategory {
    ProviderTransient,
    ProviderRateLimit,
    ContextTooLarge,
    AuthenticationFailed,
    ModelUnavailable,
    BillingQuota,
    InvalidRequest,
    NetworkFailure,
    StreamCorrupted,
    ToolSchemaInvalid,
    ContentPolicy,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserErrorInfo {
    pub category: UserErrorCategory,
    pub title: &'static str,
    pub explanation: &'static str,
    pub suggested_action: &'static str,
    pub is_retryable: bool,
}

pub const MAX_LLM_RETRY_ATTEMPTS: usize = 5;
pub const LLM_RETRY_DELAYS: [Duration; MAX_LLM_RETRY_ATTEMPTS] = [
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(45),
    Duration::from_secs(120),
    Duration::from_secs(300),
];

const NON_RETRYABLE_STATUS_CODES: &[&str] = &[
    "400", "401", "402", "403", "404", "405", "409", "413", "415", "422", "423", "424", "426",
    "451",
];

const RETRYABLE_STATUS_CODES: &[&str] = &["408", "425", "429", "500", "502", "503", "504", "529"];
const PROVIDER_TRANSIENT_STATUS_CODES: &[&str] = &["408", "425", "500", "502", "503", "504", "529"];
const RATE_LIMIT_STATUS_CODES: &[&str] = &["429"];
const CONTEXT_LIMIT_STATUS_CODES: &[&str] = &["413"];
const AUTHENTICATION_STATUS_CODES: &[&str] = &["401", "403"];
const BILLING_STATUS_CODES: &[&str] = &["402"];
const MODEL_UNAVAILABLE_STATUS_CODES: &[&str] = &["404"];
const CONTENT_POLICY_STATUS_CODES: &[&str] = &["451"];
const INVALID_REQUEST_STATUS_CODES: &[&str] =
    &["400", "405", "409", "415", "422", "423", "424", "426"];

const USER_CANCELLED_PATTERNS: &[&str] = &["aborted", "cancelled", "canceled"];

const CONTEXT_LIMIT_PATTERNS: &[&str] = &[
    "context window",
    "context length",
    "context_length",
    "context-length",
    "context limit",
    "context_limit",
    "context_length_exceeded",
    "maximum context",
    "max context",
    "too many tokens",
    "token limit",
    "tokens exceed",
    "token count exceeds",
    "input token count exceeds",
    "prompt is too long",
    "input is too long",
    "request too large",
    "payload too large",
    "payload exceeds size limit",
    "exceeds the maximum number of tokens",
    "exceed the maximum number of tokens",
];

const AUTHENTICATION_PATTERNS: &[&str] = &[
    "invalid api key",
    "incorrect api key",
    "api key not valid",
    "invalid key",
    "invalid_api_key",
    "invalid x-api-key",
    "invalid api_key",
    "authentication",
    "unauthorized",
    "forbidden",
    "permission denied",
    "permission_denied",
    "permission_error",
    "insufficient_scope",
    "invalid_scope",
    "access_denied",
    "access denied",
    "invalid_grant",
    "expired_token",
    "token expired",
    "expired token",
    "invalidated oauth token",
    "codex login",
    "refact does not refresh codex cli-managed tokens",
    "openai codex provider settings",
    "no authorization code received",
    "missing state parameter",
    "api key",
];

const BILLING_QUOTA_PATTERNS: &[&str] = &[
    "billing",
    "insufficient credits",
    "insufficient credit",
    "credits_exhausted",
    "credit balance",
    "insufficient quota",
    "insufficient_quota",
    "quota exceeded",
    "quota_exceeded",
    "quota limit",
    "usage limit reached",
    "no credits remaining",
    "payment required",
    "spend_limit_exceeded",
    "hard_limit_reached",
    "exceeded your current quota",
    "prepaid credits",
];

const CONTENT_POLICY_PATTERNS: &[&str] = &[
    "safety",
    "policy",
    "content filter",
    "content_filter",
    "content policy",
    "moderation_required",
    "moderation required",
    "recitation",
    "blocked by safety",
    "responsibleaipolicyviolation",
];

const MODEL_UNAVAILABLE_PATTERNS: &[&str] = &[
    "model_not_found",
    "model not found",
    "model does not exist",
    "invalid model",
    "unknown model",
    "no such model",
    "model is not available",
    "model unavailable",
    "model is unavailable",
    "not available for model",
    "no endpoints found",
    "no endpoint found",
    "model is deprecated",
];

const RATE_LIMIT_PATTERNS: &[&str] = &[
    "rate limit",
    "rate_limit",
    "rate-limit",
    "ratelimit",
    "rate_limit_exceeded",
    "too many requests",
    "too_many_requests",
    "resource exhausted",
    "resource_exhausted",
    "throttl",
    "slow_down",
    "websocket_connection_limit_reached",
    "requests per minute",
    "tokens per minute",
];

const NETWORK_FAILURE_PATTERNS: &[&str] = &[
    "network",
    "connection reset",
    "connection closed",
    "connection refused",
    "connection aborted",
    "connection error",
    "connect error",
    "failed to connect",
    "error trying to connect",
    "tcp connect",
    "dns",
    "eof",
    "broken pipe",
    "socket",
    "tls",
    "certificate",
    "no route to host",
    "name or service not known",
    "failed to resolve",
    "failed to lookup",
    "host lookup failed",
    "no such host",
    "network unreachable",
    "proxy error",
];

const STREAM_CORRUPTED_PATTERNS: &[&str] = &[
    "decode response body",
    "decoding response body",
    "body decode",
    "decode body",
    "failed to decode",
    "stream ended unexpectedly",
    "ended unexpectedly",
    "response.failed",
    "error event",
    "invalid provider output",
    "failed to parse event",
    "invalid sse",
    "malformed sse",
    "incomplete chunk",
    "chunked encoding",
    "can't stream from",
    "cannot stream from",
    "stream interrupted after partial output",
    "all retry attempts failed",
];

const TOOL_SCHEMA_INVALID_PATTERNS: &[&str] = &[
    "tool_use ids were found without tool_result",
    "without tool_result",
    "missing tool_result",
    "tool_result blocks immediately after",
    "property keys should match pattern",
    "input_schema.properties",
    "custom.input_schema.properties",
    "invalid tool schema",
    "bad tool schema",
    "tool schema",
    "tool call schema",
    "messages with role `tool` must be a response",
    "messages with role 'tool' must be a response",
];

const PROVIDER_TRANSIENT_PATTERNS: &[&str] = &[
    "timeout",
    "timed out",
    "deadline exceeded",
    "temporarily unavailable",
    "try again",
    "overloaded",
    "overload",
    "overloaded_error",
    "capacity",
    "server error",
    "api_error",
    "internal error",
    "internal server error",
    "service unavailable",
    "unavailable",
    "bad gateway",
    "gateway timeout",
    "out of memory",
    "oom",
    "server is busy",
    "server busy",
    "upstream error",
    "upstream request timeout",
    "authorization_pending",
    "model runner process has terminated",
];

const INVALID_REQUEST_PATTERNS: &[&str] = &[
    "invalid request",
    "bad request",
    "malformed",
    "json problem",
    "serialize",
    "deserialize",
    "invalid_argument",
    "invalid argument",
    "unknown variant",
    "no endpoint configured",
    "invalid content-type header",
    "streaming with n > 1 is not supported",
    "unsupported",
    "does not support",
    "not supported",
    "schema validation failed",
    "validationexception",
    "unrecognized request argument",
    "missing required parameter",
    "field required",
    "invalid parameter",
    "invalid value",
    "must be positive",
    "no route",
];

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn contains_status(lower: &str, codes: &[&str]) -> bool {
    codes.iter().any(|code| contains_status_code(lower, code))
}

fn contains_status_code(lower: &str, code: &str) -> bool {
    let explicit_prefixes = [
        "http ",
        "http status ",
        "status ",
        "status: ",
        "status_code:",
        "status_code: ",
        "code:",
        "code: ",
        "code=",
        "error code ",
        "error code:",
        "error code: ",
        "api error ",
    ];
    if explicit_prefixes
        .iter()
        .any(|prefix| contains_code_token(lower, &format!("{prefix}{code}"), prefix.len()))
    {
        return true;
    }

    contains_parenthesized_status_code(lower, code)
        || contains_provider_prefixed_status_code(lower, code)
}

fn contains_code_token(lower: &str, pattern: &str, code_offset: usize) -> bool {
    let mut start = 0usize;
    while let Some(pos) = lower[start..].find(pattern) {
        let idx = start + pos + code_offset;
        let end = idx + pattern.len() - code_offset;
        if is_status_code_boundary(lower, idx, end) {
            return true;
        }
        start += pos + 1;
    }
    false
}

fn is_status_code_boundary(text: &str, start: usize, end: usize) -> bool {
    let before_ok = start == 0
        || text[..start]
            .chars()
            .next_back()
            .map(|c| !c.is_ascii_digit())
            .unwrap_or(true);
    let after_ok = end >= text.len()
        || text[end..]
            .chars()
            .next()
            .map(|c| !c.is_ascii_digit())
            .unwrap_or(true);
    before_ok && after_ok
}

fn contains_parenthesized_status_code(lower: &str, code: &str) -> bool {
    let pattern = format!("({code}");
    let mut start = 0usize;
    while let Some(pos) = lower[start..].find(&pattern) {
        let idx = start + pos + 1;
        let end = idx + code.len();
        if is_status_code_boundary(lower, idx, end) {
            let before = window_before(lower, start + pos, 40);
            if contains_any(before, &["api", "error", "http", "status"]) {
                return true;
            }
        }
        start += pos + 1;
    }
    false
}

fn contains_provider_prefixed_status_code(lower: &str, code: &str) -> bool {
    let provider_context = [
        "openai",
        "anthropic",
        "gemini",
        "ollama",
        "openrouter",
        "deepseek",
        "groq",
        "xai",
        "provider",
        "llm",
    ];
    let status_reasons = [
        ":",
        " internal server error",
        " service unavailable",
        " bad gateway",
        " gateway timeout",
        " rate",
        " unauthorized",
        " forbidden",
        " not found",
        " invalid",
        " bad request",
        " payment required",
        " too many",
        " payload",
        " request",
    ];
    let mut start = 0usize;
    while let Some(pos) = lower[start..].find(code) {
        let idx = start + pos;
        let end = idx + code.len();
        if is_status_code_boundary(lower, idx, end) {
            let before = window_before(lower, idx, 40);
            let after = window_after(lower, end, 32);
            if contains_any(before, &provider_context)
                && status_reasons
                    .iter()
                    .any(|reason| after.starts_with(reason))
            {
                return true;
            }
        }
        start += pos + 1;
    }
    false
}

fn contains_model_unavailable(lower: &str) -> bool {
    contains_any(lower, MODEL_UNAVAILABLE_PATTERNS) || contains_model_not_found(lower)
}

fn contains_model_not_found(lower: &str) -> bool {
    let mut start = 0usize;
    while let Some(pos) = lower[start..].find("not found") {
        let idx = start + pos;
        let after_start = idx + "not found".len();
        let before = window_before(lower, idx, 48);
        let after = window_after(lower, after_start, 48);
        if contains_any(before, &["model", "endpoint", "route"])
            || contains_any(after, &["model", "endpoint", "route"])
        {
            return true;
        }
        start += pos + 1;
    }
    false
}

fn window_before(text: &str, end: usize, max_chars: usize) -> &str {
    let mut start = 0usize;
    for (count, (idx, _)) in text[..end].char_indices().rev().enumerate() {
        if count + 1 == max_chars {
            start = idx;
            break;
        }
    }
    &text[start..end]
}

fn window_after(text: &str, start: usize, max_chars: usize) -> &str {
    let end = text[start..]
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| start + idx)
        .unwrap_or(text.len());
    &text[start..end]
}

fn contains_retryable_status(lower: &str) -> bool {
    contains_status(lower, RETRYABLE_STATUS_CODES)
}

fn contains_non_retryable_status(lower: &str) -> bool {
    contains_status(lower, NON_RETRYABLE_STATUS_CODES)
}

fn classify_user_error_from_lower(lower: &str) -> UserErrorCategory {
    if contains_any(lower, TOOL_SCHEMA_INVALID_PATTERNS) {
        return UserErrorCategory::ToolSchemaInvalid;
    }

    if contains_status(lower, CONTEXT_LIMIT_STATUS_CODES)
        || contains_any(lower, CONTEXT_LIMIT_PATTERNS)
    {
        return UserErrorCategory::ContextTooLarge;
    }

    if contains_status(lower, BILLING_STATUS_CODES) || contains_any(lower, BILLING_QUOTA_PATTERNS) {
        return UserErrorCategory::BillingQuota;
    }

    if contains_status(lower, AUTHENTICATION_STATUS_CODES)
        || contains_any(lower, AUTHENTICATION_PATTERNS)
    {
        return UserErrorCategory::AuthenticationFailed;
    }

    if contains_status(lower, CONTENT_POLICY_STATUS_CODES)
        || contains_any(lower, CONTENT_POLICY_PATTERNS)
    {
        return UserErrorCategory::ContentPolicy;
    }

    if contains_status(lower, MODEL_UNAVAILABLE_STATUS_CODES) || contains_model_unavailable(lower) {
        return UserErrorCategory::ModelUnavailable;
    }

    if contains_status(lower, RATE_LIMIT_STATUS_CODES) || contains_any(lower, RATE_LIMIT_PATTERNS) {
        return UserErrorCategory::ProviderRateLimit;
    }

    if contains_any(lower, NETWORK_FAILURE_PATTERNS) {
        return UserErrorCategory::NetworkFailure;
    }

    if contains_status(lower, PROVIDER_TRANSIENT_STATUS_CODES)
        || contains_any(lower, PROVIDER_TRANSIENT_PATTERNS)
    {
        return UserErrorCategory::ProviderTransient;
    }

    if contains_any(lower, STREAM_CORRUPTED_PATTERNS) {
        return UserErrorCategory::StreamCorrupted;
    }

    if contains_status(lower, INVALID_REQUEST_STATUS_CODES)
        || contains_non_retryable_status(lower)
        || contains_any(lower, INVALID_REQUEST_PATTERNS)
    {
        return UserErrorCategory::InvalidRequest;
    }

    UserErrorCategory::Unknown
}

pub fn classify_user_error(error: &str) -> UserErrorCategory {
    classify_user_error_from_lower(&error.to_lowercase())
}

pub fn user_error_info(category: UserErrorCategory) -> UserErrorInfo {
    match category {
        UserErrorCategory::ProviderTransient => UserErrorInfo {
            category,
            title: "Provider temporarily unavailable",
            explanation: "The model provider returned a temporary timeout, server, overload, or capacity error.",
            suggested_action: "retry",
            is_retryable: true,
        },
        UserErrorCategory::ProviderRateLimit => UserErrorInfo {
            category,
            title: "Rate limit reached",
            explanation: "The model provider is throttling requests or has asked clients to slow down.",
            suggested_action: "retry",
            is_retryable: true,
        },
        UserErrorCategory::ContextTooLarge => UserErrorInfo {
            category,
            title: "Context too large",
            explanation: "The prompt or request exceeds the model context window or payload size limit.",
            suggested_action: "compact",
            is_retryable: false,
        },
        UserErrorCategory::AuthenticationFailed => UserErrorInfo {
            category,
            title: "Authentication failed",
            explanation: "The provider rejected the configured credentials, token, or authorization scope.",
            suggested_action: "check_auth",
            is_retryable: false,
        },
        UserErrorCategory::ModelUnavailable => UserErrorInfo {
            category,
            title: "Model unavailable",
            explanation: "The requested model was not found, is unavailable, or is not supported by the provider route.",
            suggested_action: "switch_model",
            is_retryable: false,
        },
        UserErrorCategory::BillingQuota => UserErrorInfo {
            category,
            title: "Billing or quota limit reached",
            explanation: "The provider reports exhausted credits, quota, billing, or payment limits.",
            suggested_action: "check_billing",
            is_retryable: false,
        },
        UserErrorCategory::InvalidRequest => UserErrorInfo {
            category,
            title: "Invalid request",
            explanation: "The provider rejected the request shape, parameters, content type, or unsupported feature.",
            suggested_action: "none",
            is_retryable: false,
        },
        UserErrorCategory::NetworkFailure => UserErrorInfo {
            category,
            title: "Network failure",
            explanation: "The request failed because of connection, DNS, socket, TLS, or transport failure.",
            suggested_action: "retry",
            is_retryable: true,
        },
        UserErrorCategory::StreamCorrupted => UserErrorInfo {
            category,
            title: "Stream corrupted",
            explanation: "The provider stream ended unexpectedly or returned data that could not be decoded.",
            suggested_action: "retry",
            is_retryable: true,
        },
        UserErrorCategory::ToolSchemaInvalid => UserErrorInfo {
            category,
            title: "Tool schema invalid",
            explanation: "The provider rejected tool schema metadata or tool call and result ordering.",
            suggested_action: "none",
            is_retryable: false,
        },
        UserErrorCategory::ContentPolicy => UserErrorInfo {
            category,
            title: "Content policy blocked",
            explanation: "The provider blocked the request or response with a safety, moderation, or policy rule.",
            suggested_action: "none",
            is_retryable: false,
        },
        UserErrorCategory::Unknown => UserErrorInfo {
            category,
            title: "Unknown error",
            explanation: "The error did not match a known provider, network, request, billing, or policy pattern.",
            suggested_action: "none",
            is_retryable: false,
        },
    }
}

pub fn classify_llm_error_for_retry(error: &str) -> RetryDecision {
    let lower = error.to_lowercase();

    if contains_any(&lower, USER_CANCELLED_PATTERNS) {
        return RetryDecision::UserCancelled {
            reason: "cancelled",
        };
    }

    match classify_user_error_from_lower(&lower) {
        UserErrorCategory::ProviderTransient
        | UserErrorCategory::ProviderRateLimit
        | UserErrorCategory::NetworkFailure
        | UserErrorCategory::StreamCorrupted => RetryDecision::Retry {
            reason: if contains_retryable_status(&lower) {
                "retryable_http_status"
            } else {
                "transient_error"
            },
        },
        UserErrorCategory::ContextTooLarge => RetryDecision::ContextLimit {
            reason: "context_limit",
        },
        UserErrorCategory::Unknown => RetryDecision::DoNotRetry {
            reason: "unknown_error",
        },
        UserErrorCategory::AuthenticationFailed
        | UserErrorCategory::ModelUnavailable
        | UserErrorCategory::BillingQuota
        | UserErrorCategory::InvalidRequest
        | UserErrorCategory::ToolSchemaInvalid
        | UserErrorCategory::ContentPolicy => RetryDecision::DoNotRetry {
            reason: "non_retryable_error",
        },
    }
}

pub fn should_retry_llm_error(error: &str, retry_attempt: usize, abort_flag: &AtomicBool) -> bool {
    classify_llm_error_for_retry(error).is_retryable_transient()
        && retry_attempt < MAX_LLM_RETRY_ATTEMPTS
        && !abort_flag.load(Ordering::SeqCst)
}

pub fn retry_delay_for_attempt(retry_attempt: usize) -> Duration {
    LLM_RETRY_DELAYS
        .get(retry_attempt)
        .copied()
        .unwrap_or_else(|| *LLM_RETRY_DELAYS.last().unwrap())
}

pub async fn sleep_or_abort(delay: Duration, abort_flag: Arc<AtomicBool>) -> bool {
    let mut heartbeat = tokio::time::interval(Duration::from_millis(200));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let sleep = tokio::time::sleep(delay);
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            _ = &mut sleep => return false,
            _ = heartbeat.tick() => {
                if abort_flag.load(Ordering::SeqCst) {
                    return true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct ErrorCase {
        error: &'static str,
        decision: RetryDecision,
        category: UserErrorCategory,
    }

    fn retry_http() -> RetryDecision {
        RetryDecision::Retry {
            reason: "retryable_http_status",
        }
    }

    fn retry_transient() -> RetryDecision {
        RetryDecision::Retry {
            reason: "transient_error",
        }
    }

    fn context_limit() -> RetryDecision {
        RetryDecision::ContextLimit {
            reason: "context_limit",
        }
    }

    fn non_retryable() -> RetryDecision {
        RetryDecision::DoNotRetry {
            reason: "non_retryable_error",
        }
    }

    fn unknown_error() -> RetryDecision {
        RetryDecision::DoNotRetry {
            reason: "unknown_error",
        }
    }

    fn cancelled() -> RetryDecision {
        RetryDecision::UserCancelled {
            reason: "cancelled",
        }
    }

    #[test]
    fn classifies_real_provider_errors_table() {
        let cases: &[ErrorCase] = &[
            ErrorCase {
                error: "OpenAI error code: 429 - Rate limit reached for gpt-4o-mini in organization org on tokens per min",
                decision: retry_http(),
                category: UserErrorCategory::ProviderRateLimit,
            },
            ErrorCase {
                error: "OpenAI API error (429): rate_limit_exceeded: Please try again in 20s",
                decision: retry_http(),
                category: UserErrorCategory::ProviderRateLimit,
            },
            ErrorCase {
                error: "OpenAI error code: 429 - insufficient_quota: You exceeded your current quota",
                decision: non_retryable(),
                category: UserErrorCategory::BillingQuota,
            },
            ErrorCase {
                error: "OpenAI invalid_request_error: This model's maximum context length is 128000 tokens. However, your messages resulted in 129001 tokens",
                decision: context_limit(),
                category: UserErrorCategory::ContextTooLarge,
            },
            ErrorCase {
                error: "OpenAI BadRequestError: context_length_exceeded: Please reduce your prompt",
                decision: context_limit(),
                category: UserErrorCategory::ContextTooLarge,
            },
            ErrorCase {
                error: "OpenAI API error (401 Unauthorized): Incorrect API key provided",
                decision: non_retryable(),
                category: UserErrorCategory::AuthenticationFailed,
            },
            ErrorCase {
                error: "OpenAI API error (404): model_not_found: The model `gpt-4.5-preview` does not exist or you do not have access to it",
                decision: non_retryable(),
                category: UserErrorCategory::ModelUnavailable,
            },
            ErrorCase {
                error: "OpenAI API error (400): content_filter: This response was filtered due to the prompt triggering Azure OpenAI content management policy",
                decision: non_retryable(),
                category: UserErrorCategory::ContentPolicy,
            },
            ErrorCase {
                error: "OpenAI response.failed: Internal server error (code=server_error)",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "OpenAI API error (503 Service Unavailable): The server is overloaded or not ready yet",
                decision: retry_http(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "OpenAI API error (400 Bad Request): Unrecognized request argument supplied: max_completion_tokens",
                decision: non_retryable(),
                category: UserErrorCategory::InvalidRequest,
            },
            ErrorCase {
                error: "Anthropic API error 529: overloaded_error: Overloaded",
                decision: retry_http(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "Anthropic overloaded_error: Claude is currently overloaded, please try again later",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "Anthropic API error (429): rate_limit_error: Your account has hit a rate limit",
                decision: retry_http(),
                category: UserErrorCategory::ProviderRateLimit,
            },
            ErrorCase {
                error: "Anthropic invalid_request_error: prompt is too long: 201000 tokens > 200000 maximum context window",
                decision: context_limit(),
                category: UserErrorCategory::ContextTooLarge,
            },
            ErrorCase {
                error: "Anthropic messages API: tool_use ids were found without tool_result blocks immediately after: toolu_01ABC",
                decision: non_retryable(),
                category: UserErrorCategory::ToolSchemaInvalid,
            },
            ErrorCase {
                error: "Anthropic Bad Request: tools.47.custom.input_schema.properties: Property keys should match pattern ^[a-zA-Z0-9_-]{1,64}$",
                decision: non_retryable(),
                category: UserErrorCategory::ToolSchemaInvalid,
            },
            ErrorCase {
                error: "Anthropic invalid_request_error: unknown variant `web_search_20250305`, expected one of `auto`, `any`, `tool`",
                decision: non_retryable(),
                category: UserErrorCategory::InvalidRequest,
            },
            ErrorCase {
                error: "Anthropic OAuth: Encountered invalidated oauth token; run claude login again",
                decision: non_retryable(),
                category: UserErrorCategory::AuthenticationFailed,
            },
            ErrorCase {
                error: "Anthropic API error (400): messages.0.content.0: Field required",
                decision: non_retryable(),
                category: UserErrorCategory::InvalidRequest,
            },
            ErrorCase {
                error: "Anthropic billing_error: credit balance is too low to access Claude",
                decision: non_retryable(),
                category: UserErrorCategory::BillingQuota,
            },
            ErrorCase {
                error: "Gemini API error: RESOURCE_EXHAUSTED: Too many requests. Please try again later",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderRateLimit,
            },
            ErrorCase {
                error: "Gemini API error: RESOURCE_EXHAUSTED: Quota exceeded for quota metric generativelanguage.googleapis.com/generate_content_free_tier_requests",
                decision: non_retryable(),
                category: UserErrorCategory::BillingQuota,
            },
            ErrorCase {
                error: "Gemini API error: INVALID_ARGUMENT: Request contains an invalid argument",
                decision: non_retryable(),
                category: UserErrorCategory::InvalidRequest,
            },
            ErrorCase {
                error: "Gemini API error: PERMISSION_DENIED: API key not valid. Please pass a valid API key",
                decision: non_retryable(),
                category: UserErrorCategory::AuthenticationFailed,
            },
            ErrorCase {
                error: "Gemini response was blocked due to SAFETY: candidate.finish_reason=SAFETY",
                decision: non_retryable(),
                category: UserErrorCategory::ContentPolicy,
            },
            ErrorCase {
                error: "Gemini candidate was blocked because finish_reason was RECITATION",
                decision: non_retryable(),
                category: UserErrorCategory::ContentPolicy,
            },
            ErrorCase {
                error: "Gemini API error 400 INVALID_ARGUMENT: The input token count exceeds the maximum number of tokens allowed",
                decision: context_limit(),
                category: UserErrorCategory::ContextTooLarge,
            },
            ErrorCase {
                error: "Gemini API error 503 UNAVAILABLE: The model is overloaded. Please try again later",
                decision: retry_http(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "Ollama error: model 'llama3.2:latest' not found, try pulling it first",
                decision: non_retryable(),
                category: UserErrorCategory::ModelUnavailable,
            },
            ErrorCase {
                error: "error sending request for url http://localhost:11434/api/chat: connection refused",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "Ollama server error: llama runner process: cuda out of memory",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "EOF while reading from Ollama stream",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "Ollama 500 Internal Server Error: model runner process has terminated",
                decision: retry_http(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "Ollama host lookup failed: no such host ollama.local",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "OpenRouter error 402: credits_exhausted: Add credits to continue",
                decision: non_retryable(),
                category: UserErrorCategory::BillingQuota,
            },
            ErrorCase {
                error: "OpenRouter moderation_required: This prompt requires moderation before it can be sent",
                decision: non_retryable(),
                category: UserErrorCategory::ContentPolicy,
            },
            ErrorCase {
                error: "OpenRouter error 529: Provider is overloaded",
                decision: retry_http(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "OpenRouter 429: rate-limited upstream provider",
                decision: retry_http(),
                category: UserErrorCategory::ProviderRateLimit,
            },
            ErrorCase {
                error: "OpenRouter 404: No endpoints found that support this model",
                decision: non_retryable(),
                category: UserErrorCategory::ModelUnavailable,
            },
            ErrorCase {
                error: "OpenRouter Provider returned error: context length exceeded for model anthropic/claude-3.7-sonnet",
                decision: context_limit(),
                category: UserErrorCategory::ContextTooLarge,
            },
            ErrorCase {
                error: "DeepSeek API error: rate_limit_exceeded: Rate limit reached for deepseek-chat",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderRateLimit,
            },
            ErrorCase {
                error: "DeepSeek API error: This model's maximum context length is 65536 tokens",
                decision: context_limit(),
                category: UserErrorCategory::ContextTooLarge,
            },
            ErrorCase {
                error: "DeepSeek API server is busy, please try again later",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "Groq API error 429: Rate limit reached for model llama-3.1-70b-versatile in organization",
                decision: retry_http(),
                category: UserErrorCategory::ProviderRateLimit,
            },
            ErrorCase {
                error: "Groq invalid_request_error: Please reduce the length of the messages or completion because it exceeds the context window",
                decision: context_limit(),
                category: UserErrorCategory::ContextTooLarge,
            },
            ErrorCase {
                error: "xAI API error: too_many_requests: rate limit exceeded",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderRateLimit,
            },
            ErrorCase {
                error: "xAI API error: prompt token count exceeds context window for grok-2",
                decision: context_limit(),
                category: UserErrorCategory::ContextTooLarge,
            },
            ErrorCase {
                error: "xAI API error: insufficient credits in account",
                decision: non_retryable(),
                category: UserErrorCategory::BillingQuota,
            },
            ErrorCase {
                error: "LLM request failed: operation timed out after 60 seconds",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "LLM request failed: deadline exceeded while waiting for response headers",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "Stream error: connection reset by peer",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "Stream error: connection closed before message completed",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "Transport error: connection refused by provider host",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "DNS error: failed to lookup address information for api.openai.com",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "unexpected EOF while reading response body",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "write tcp 10.0.0.1: broken pipe",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "TLS handshake failed: certificate verify failed",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "error trying to connect: tcp connect error: Network is unreachable",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "failed to resolve host: Name or service not known",
                decision: retry_transient(),
                category: UserErrorCategory::NetworkFailure,
            },
            ErrorCase {
                error: "failed to decode response body from provider stream",
                decision: retry_transient(),
                category: UserErrorCategory::StreamCorrupted,
            },
            ErrorCase {
                error: "LLM stream ended unexpectedly without completion signal",
                decision: retry_transient(),
                category: UserErrorCategory::StreamCorrupted,
            },
            ErrorCase {
                error: "SSE error event: response.failed with code server_error",
                decision: retry_transient(),
                category: UserErrorCategory::StreamCorrupted,
            },
            ErrorCase {
                error: "InvalidProviderOutput: failed to parse event JSON from stream",
                decision: retry_transient(),
                category: UserErrorCategory::StreamCorrupted,
            },
            ErrorCase {
                error: "hyper error: incomplete chunked encoding from upstream",
                decision: retry_transient(),
                category: UserErrorCategory::StreamCorrupted,
            },
            ErrorCase {
                error: "No endpoint configured for provider custom-openai-compatible",
                decision: non_retryable(),
                category: UserErrorCategory::InvalidRequest,
            },
            ErrorCase {
                error: "Streaming with n > 1 is not supported",
                decision: non_retryable(),
                category: UserErrorCategory::InvalidRequest,
            },
            ErrorCase {
                error: "Invalid content-type header: text/html; expected application/json",
                decision: non_retryable(),
                category: UserErrorCategory::InvalidRequest,
            },
            ErrorCase {
                error: "Unsupported image input for this provider route",
                decision: non_retryable(),
                category: UserErrorCategory::InvalidRequest,
            },
            ErrorCase {
                error: "Refact does not refresh Codex CLI-managed tokens; run codex login",
                decision: non_retryable(),
                category: UserErrorCategory::AuthenticationFailed,
            },
            ErrorCase {
                error: "Codex OAuth slow_down: polling too quickly",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderRateLimit,
            },
            ErrorCase {
                error: "authorization_pending",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderTransient,
            },
            ErrorCase {
                error: "websocket_connection_limit_reached for ChatGPT backend",
                decision: retry_transient(),
                category: UserErrorCategory::ProviderRateLimit,
            },
            ErrorCase {
                error: "LLM error: unknown provider failure frobnicated the request",
                decision: unknown_error(),
                category: UserErrorCategory::Unknown,
            },
            ErrorCase {
                error: "User cancelled the operation",
                decision: cancelled(),
                category: UserErrorCategory::Unknown,
            },
        ];

        assert!(cases.len() >= 50);
        for case in cases {
            assert_eq!(
                classify_llm_error_for_retry(case.error),
                case.decision,
                "unexpected retry decision for {}",
                case.error
            );
            assert_eq!(
                classify_user_error(case.error),
                case.category,
                "unexpected user category for {}",
                case.error
            );
        }
    }

    #[test]
    fn test_partial_output_with_context_limit_classifies_correctly() {
        assert_eq!(
            classify_user_error("Stream interrupted after partial output and all retry attempts failed. Original error: LLM error: invalid_request_error: context_length_exceeded: Your input exceeds the context window"),
            UserErrorCategory::ContextTooLarge
        );
    }

    #[test]
    fn test_partial_output_alone_classifies_as_stream_corrupted() {
        assert_eq!(
            classify_user_error(
                "Stream interrupted after partial output and all retry attempts failed."
            ),
            UserErrorCategory::StreamCorrupted
        );
    }

    #[test]
    fn retryable_status_wins_over_generic_validation_words() {
        assert!(matches!(
            classify_llm_error_for_retry("LLM error (429): ValidationException: rate limited"),
            RetryDecision::Retry { .. }
        ));
        assert_eq!(
            classify_user_error("LLM error (429): ValidationException: rate limited"),
            UserErrorCategory::ProviderRateLimit
        );
    }

    #[test]
    fn billing_quota_wins_over_retryable_status() {
        assert_eq!(
            classify_llm_error_for_retry("LLM error (429): insufficient_quota"),
            RetryDecision::DoNotRetry {
                reason: "non_retryable_error",
            }
        );
        assert_eq!(
            classify_user_error("LLM error (429): insufficient_quota"),
            UserErrorCategory::BillingQuota
        );
    }

    #[test]
    fn classifier_avoids_broad_status_and_context_patterns() {
        assert_eq!(
            classify_llm_error_for_retry(
                "OpenAI invalid_request_error: max tokens must be positive"
            ),
            RetryDecision::DoNotRetry {
                reason: "non_retryable_error",
            }
        );
        assert_eq!(
            classify_user_error("OpenAI invalid_request_error: max tokens must be positive"),
            UserErrorCategory::InvalidRequest
        );

        assert_eq!(
            classify_user_error("File not found: src/main.rs"),
            UserErrorCategory::Unknown
        );
        assert_eq!(
            classify_user_error("operation failed after 500 attempts"),
            UserErrorCategory::Unknown
        );
    }

    #[test]
    fn does_not_retry_user_cancellation() {
        assert!(matches!(
            classify_llm_error_for_retry("Aborted"),
            RetryDecision::UserCancelled {
                reason: "cancelled"
            }
        ));
        assert_eq!(classify_user_error("Aborted"), UserErrorCategory::Unknown);
    }

    #[test]
    fn should_retry_obeys_attempt_limit_and_abort() {
        let flag = AtomicBool::new(false);
        assert!(should_retry_llm_error("timeout", 0, &flag));
        assert!(!should_retry_llm_error(
            "timeout",
            MAX_LLM_RETRY_ATTEMPTS,
            &flag
        ));
        flag.store(true, Ordering::SeqCst);
        assert!(!should_retry_llm_error("timeout", 0, &flag));
    }

    #[test]
    fn helper_methods_identify_classification_groups() {
        assert!(classify_llm_error_for_retry("timeout").is_retryable_transient());
        assert!(classify_llm_error_for_retry("context length exceeded").is_context_limit());
        assert!(classify_llm_error_for_retry("cancelled").is_user_cancelled());
        assert_eq!(
            classify_llm_error_for_retry("timeout").reason(),
            "transient_error"
        );
    }

    #[test]
    fn user_error_info_has_action_and_retryability_for_each_category() {
        let cases = [
            (UserErrorCategory::ProviderTransient, "retry", true),
            (UserErrorCategory::ProviderRateLimit, "retry", true),
            (UserErrorCategory::ContextTooLarge, "compact", false),
            (UserErrorCategory::AuthenticationFailed, "check_auth", false),
            (UserErrorCategory::ModelUnavailable, "switch_model", false),
            (UserErrorCategory::BillingQuota, "check_billing", false),
            (UserErrorCategory::InvalidRequest, "none", false),
            (UserErrorCategory::NetworkFailure, "retry", true),
            (UserErrorCategory::StreamCorrupted, "retry", true),
            (UserErrorCategory::ToolSchemaInvalid, "none", false),
            (UserErrorCategory::ContentPolicy, "none", false),
            (UserErrorCategory::Unknown, "none", false),
        ];

        for (category, suggested_action, is_retryable) in cases {
            let info = user_error_info(category);
            assert_eq!(info.category, category);
            assert!(!info.title.is_empty());
            assert!(!info.explanation.is_empty());
            assert_eq!(info.suggested_action, suggested_action);
            assert_eq!(info.is_retryable, is_retryable);
        }
    }
}
