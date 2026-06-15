use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

pub const DEFAULT_RETRY_MAX_ATTEMPTS: u32 = 3;
pub const DEFAULT_RETRY_BACKOFF_MS: [u64; 3] = [60_000, 120_000, 300_000];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryCategory {
    RateLimit,
    Overloaded,
    Network,
    Timeout,
    ServerError,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetryConfig {
    #[serde(default = "default_retry_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_retry_backoff_ms")]
    pub backoff_ms: Vec<u64>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_RETRY_MAX_ATTEMPTS,
            backoff_ms: default_retry_backoff_ms(),
        }
    }
}

pub fn classify(error: &str) -> Option<RetryCategory> {
    let error = error.trim();
    if error.is_empty() {
        return None;
    }
    if RATE_LIMIT_RE.is_match(error) {
        return Some(RetryCategory::RateLimit);
    }
    if OVERLOADED_RE.is_match(error) {
        return Some(RetryCategory::Overloaded);
    }
    if NETWORK_RE.is_match(error) {
        return Some(RetryCategory::Network);
    }
    if TIMEOUT_RE.is_match(error) {
        return Some(RetryCategory::Timeout);
    }
    if SERVER_CONTEXT_RE.is_match(error)
        || SERVER_5XX_PHRASE_RE.is_match(error)
        || SERVER_PHRASE_RE.is_match(error)
        || SERVER_5XX_RE.is_match(error)
        || SERVER_TERSE_RE.is_match(error)
    {
        return Some(RetryCategory::ServerError);
    }
    None
}

pub fn retry_delay_ms(config: &RetryConfig, completed_retry_attempts: u32) -> Option<u64> {
    if completed_retry_attempts >= config.max_attempts {
        return None;
    }
    let delay = config
        .backoff_ms
        .get(completed_retry_attempts as usize)
        .copied()
        .or_else(|| config.backoff_ms.last().copied())?;
    (delay > 0).then_some(delay)
}

fn default_retry_max_attempts() -> u32 {
    DEFAULT_RETRY_MAX_ATTEMPTS
}

fn default_retry_backoff_ms() -> Vec<u64> {
    DEFAULT_RETRY_BACKOFF_MS.to_vec()
}

static RATE_LIMIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(rate[_ ]limit|too many requests|429|resource has been exhausted|cloudflare|tokens per day)").unwrap()
});
static OVERLOADED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b529\b|\boverloaded(?:_error)?\b|high demand|temporar(?:ily|y) overloaded|capacity exceeded").unwrap()
});
static NETWORK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(network|fetch failed|socket|econnreset|econnrefused|eai_again|enetdown|ehostunreach|ehostdown|enetreset|enetunreach|epipe)").unwrap()
});
static TIMEOUT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(timeout|etimedout)").unwrap());
static SERVER_CONTEXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r###"(?i)\b(?:https?|status(?:[ _]code)?|response(?:[ _]code)?|http(?:[ _]status)?)\b[\s:=#"']{0,4}5\d{2}\b"###).unwrap()
});
static SERVER_5XX_PHRASE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b5\d{2}\b[\s:)\].,-]*(?:internal server error|server error|bad gateway|service unavailable|gateway time-?out)\b").unwrap()
});
static SERVER_PHRASE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(internal server error|bad gateway|service unavailable|gateway time-?out)\b")
        .unwrap()
});
static SERVER_5XX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b5xx\b").unwrap());
static SERVER_TERSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*5\d{2}\s*$").unwrap());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_positive_categories() {
        for (input, expected) in [
            ("rate limit exceeded", RetryCategory::RateLimit),
            ("429 Too Many Requests", RetryCategory::RateLimit),
            ("overloaded_error", RetryCategory::Overloaded),
            ("529", RetryCategory::Overloaded),
            ("ECONNRESET", RetryCategory::Network),
            ("fetch failed", RetryCategory::Network),
            ("request timeout", RetryCategory::Timeout),
            ("ETIMEDOUT", RetryCategory::Timeout),
            ("500 Internal Server Error", RetryCategory::ServerError),
            ("502 Bad Gateway", RetryCategory::ServerError),
            ("5xx", RetryCategory::ServerError),
            ("503", RetryCategory::ServerError),
        ] {
            assert_eq!(classify(input), Some(expected), "{input}");
        }
    }

    #[test]
    fn classify_does_not_treat_5xx_numbers_in_prose_as_server_errors() {
        for input in [
            "context limit 512 exceeded",
            "exited with 503 lines",
            "pid 511 killed",
            "connected to /tmp/x-540.sock",
            "",
            "unknown",
        ] {
            assert_eq!(classify(input), None, "{input}");
        }
    }

    #[test]
    fn server_error_requires_context_for_numeric_5xx() {
        for input in [
            "http 503",
            "status code: 504",
            "response_code=502",
            "HTTP status 500",
            "service unavailable",
        ] {
            assert_eq!(classify(input), Some(RetryCategory::ServerError), "{input}");
        }
    }

    #[test]
    fn retry_delay_is_bounded_and_reuses_last_delay() {
        let config = RetryConfig {
            max_attempts: 4,
            backoff_ms: vec![10, 20],
        };

        assert_eq!(retry_delay_ms(&config, 0), Some(10));
        assert_eq!(retry_delay_ms(&config, 1), Some(20));
        assert_eq!(retry_delay_ms(&config, 2), Some(20));
        assert_eq!(retry_delay_ms(&config, 3), Some(20));
        assert_eq!(retry_delay_ms(&config, 4), None);
    }
}
