use std::collections::HashMap;

pub const LLM_HTTP_HEADER_RETRY_ENABLED_HEADER: &str =
    "x-refact-internal-llm-http-header-retry-enabled";
pub const LLM_HTTP_HEADER_RETRY_TIMEOUT_SECONDS_HEADER: &str =
    "x-refact-internal-llm-http-header-retry-timeout-seconds";
pub const LLM_HTTP_HEADER_RETRY_MAX_ATTEMPTS_HEADER: &str =
    "x-refact-internal-llm-http-header-retry-max-attempts";

pub const LLM_HTTP_HEADER_RETRY_TIMEOUT_SECONDS_DEFAULT: u64 = 120;
pub const LLM_HTTP_HEADER_RETRY_MAX_ATTEMPTS_DEFAULT: usize = 10;

pub fn insert_llm_http_header_retry_config(
    extra_headers: &mut HashMap<String, String>,
    enabled: bool,
    timeout_seconds: u64,
    max_attempts: usize,
) {
    extra_headers.insert(
        LLM_HTTP_HEADER_RETRY_ENABLED_HEADER.to_string(),
        enabled.to_string(),
    );
    extra_headers.insert(
        LLM_HTTP_HEADER_RETRY_TIMEOUT_SECONDS_HEADER.to_string(),
        timeout_seconds.to_string(),
    );
    extra_headers.insert(
        LLM_HTTP_HEADER_RETRY_MAX_ATTEMPTS_HEADER.to_string(),
        max_attempts.to_string(),
    );
}
