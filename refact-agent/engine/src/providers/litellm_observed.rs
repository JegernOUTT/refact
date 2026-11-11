use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, TimeDelta, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

const SAMPLE_TTL_SECONDS: i64 = 300;
const SAMPLE_CAPACITY: usize = 100;

#[derive(Clone, Debug, Serialize)]
pub struct ObservedHeadroom {
    pub observed_at: String,
    pub expires_at: String,
    pub scope: &'static str,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requests_remaining: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_remaining: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requests_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requests_reset_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_reset_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_spend: Option<f64>,
}

struct Sample {
    identity: String,
    observed_at: i64,
    value: ObservedHeadroom,
}

#[derive(Default)]
struct Samples {
    entries: HashMap<(std::path::PathBuf, String), Sample>,
}

fn samples() -> &'static Mutex<Samples> {
    static SAMPLES: OnceLock<Mutex<Samples>> = OnceLock::new();
    SAMPLES.get_or_init(|| Mutex::new(Samples::default()))
}

pub fn identity(base: &str, bearer: &str, headers: &HashMap<String, String>) -> String {
    let mut hash = Sha256::new();
    hash.update(base.trim_end_matches('/').as_bytes());
    hash.update([0]);
    hash.update(bearer.as_bytes());
    let mut headers = headers.iter().collect::<Vec<_>>();
    headers.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (name, value) in headers {
        hash.update([0]);
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update(value.as_bytes());
    }
    hex::encode(hash.finalize())
}

fn reset_at(text: &str, now: DateTime<Utc>) -> Option<String> {
    let text = text.trim();
    if text.len() > 128 || text.is_empty() {
        return None;
    }
    if let Ok(date) = DateTime::parse_from_rfc3339(text) {
        return Some(date.with_timezone(&Utc).to_rfc3339());
    }
    if let Ok(seconds) = text.parse::<u64>() {
        let seconds = i64::try_from(seconds).ok()?;
        if seconds >= 1_000_000_000 {
            return DateTime::from_timestamp(seconds, 0).map(|date| date.to_rfc3339());
        }
        if seconds > 31_536_000 {
            return None;
        }
        return now
            .checked_add_signed(TimeDelta::seconds(seconds))
            .map(|date| date.to_rfc3339());
    }
    let mut rest = text;
    let mut milliseconds = 0_f64;
    while !rest.is_empty() {
        let end = rest.find(|c: char| !c.is_ascii_digit() && c != '.')?;
        let value = rest[..end].parse::<f64>().ok()?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        rest = &rest[end..];
        let (unit, factor) = if rest.starts_with("ms") {
            (2, 1.0)
        } else if rest.starts_with('s') {
            (1, 1_000.0)
        } else if rest.starts_with('m') {
            (1, 60_000.0)
        } else if rest.starts_with('h') {
            (1, 3_600_000.0)
        } else if rest.starts_with('d') {
            (1, 86_400_000.0)
        } else {
            return None;
        };
        milliseconds += value * factor;
        if !milliseconds.is_finite() || milliseconds > 31_536_000_000.0 {
            return None;
        }
        rest = &rest[unit..];
    }
    now.checked_add_signed(TimeDelta::milliseconds(milliseconds.ceil() as i64))
        .map(|date| date.to_rfc3339())
}

fn parse(
    headers: &reqwest::header::HeaderMap,
    model: &str,
    now: DateTime<Utc>,
) -> Option<ObservedHeadroom> {
    let text = |name: &str| headers.get(name)?.to_str().ok();
    let count = |name: &str| text(name)?.trim().parse::<u64>().ok();
    let money = |name: &str| {
        text(name)?
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value >= 0.0)
    };
    let requests_reset_at = text("x-ratelimit-reset-requests").and_then(|text| reset_at(text, now));
    let tokens_reset_at = text("x-ratelimit-reset-tokens").and_then(|text| reset_at(text, now));
    let expires_at = requests_reset_at
        .iter()
        .chain(tokens_reset_at.iter())
        .filter_map(|text| {
            DateTime::parse_from_rfc3339(text)
                .ok()
                .map(|date| date.with_timezone(&Utc))
        })
        .fold(
            now + TimeDelta::seconds(SAMPLE_TTL_SECONDS),
            |expiry, reset| expiry.min(reset),
        );
    let value = ObservedHeadroom {
        observed_at: now.to_rfc3339(),
        expires_at: expires_at.to_rfc3339(),
        scope: "unknown",
        model: model.chars().take(256).collect(),
        requests_remaining: count("x-ratelimit-remaining-requests"),
        tokens_remaining: count("x-ratelimit-remaining-tokens"),
        requests_limit: count("x-ratelimit-limit-requests"),
        tokens_limit: count("x-ratelimit-limit-tokens"),
        requests_reset_at,
        tokens_reset_at,
        response_cost: money("x-litellm-response-cost"),
        key_spend: money("x-litellm-key-spend"),
    };
    (value.requests_remaining.is_some()
        || value.tokens_remaining.is_some()
        || value.requests_limit.is_some()
        || value.tokens_limit.is_some()
        || value.response_cost.is_some()
        || value.key_spend.is_some())
    .then_some(value)
}

pub fn capture(
    config_dir: &Path,
    provider: &str,
    identity: String,
    model: &str,
    headers: &reqwest::header::HeaderMap,
) {
    let now = Utc::now();
    let Some(value) = parse(headers, model, now) else {
        return;
    };
    let Ok(mut samples) = samples().lock() else {
        return;
    };
    samples
        .entries
        .retain(|_, entry| now.timestamp().saturating_sub(entry.observed_at) < SAMPLE_TTL_SECONDS);
    let key = (config_dir.to_path_buf(), provider.to_string());
    if !samples.entries.contains_key(&key) && samples.entries.len() >= SAMPLE_CAPACITY {
        if let Some(oldest) = samples
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.observed_at)
            .map(|(key, _)| key.clone())
        {
            samples.entries.remove(&oldest);
        }
    }
    samples.entries.insert(
        key,
        Sample {
            identity,
            observed_at: now.timestamp(),
            value,
        },
    );
}

pub fn get(config_dir: &Path, provider: &str, identity: &str) -> Option<ObservedHeadroom> {
    let samples = samples().lock().ok()?;
    let sample = samples
        .entries
        .get(&(config_dir.to_path_buf(), provider.to_string()))?;
    (sample.identity == identity
        && Utc::now().timestamp().saturating_sub(sample.observed_at) < SAMPLE_TTL_SECONDS
        && DateTime::parse_from_rfc3339(&sample.value.expires_at)
            .is_ok_and(|expiry| expiry > Utc::now()))
    .then(|| sample.value.clone())
}

pub fn invalidate(config_dir: &Path, provider: &str) {
    if let Ok(mut samples) = samples().lock() {
        samples
            .entries
            .remove(&(config_dir.to_path_buf(), provider.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_sample_keeps_only_numeric_allowlisted_values_and_unknown_scope() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-ratelimit-remaining-requests", "0".parse().unwrap());
        headers.insert("x-ratelimit-limit-tokens", "1000".parse().unwrap());
        headers.insert("x-ratelimit-reset-requests", "1m2.5s".parse().unwrap());
        headers.insert("x-litellm-response-cost", "0.000001".parse().unwrap());
        headers.insert("authorization", "Bearer must-not-escape".parse().unwrap());
        headers.insert("x-litellm-key-spend", "NaN".parse().unwrap());
        let sample = parse(&headers, "alias", now).unwrap();
        assert_eq!(sample.requests_remaining, Some(0));
        assert_eq!(sample.tokens_limit, Some(1000));
        assert_eq!(
            sample.requests_reset_at,
            Some((now + TimeDelta::milliseconds(62_500)).to_rfc3339())
        );
        assert_eq!(sample.response_cost, Some(0.000001));
        assert_eq!(sample.key_spend, None);
        assert_eq!(sample.scope, "unknown");
        assert!(!serde_json::to_string(&sample)
            .unwrap()
            .contains("must-not-escape"));
    }

    #[test]
    fn reset_formats_are_bounded_and_deterministic() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        assert_eq!(
            reset_at("1700000060", now),
            Some((now + TimeDelta::seconds(60)).to_rfc3339())
        );
        assert_eq!(
            reset_at("60", now),
            Some((now + TimeDelta::seconds(60)).to_rfc3339())
        );
        assert_eq!(
            reset_at("2023-11-14T22:14:20Z", now),
            Some((now + TimeDelta::seconds(60)).to_rfc3339())
        );
        for value in [
            "-1s",
            "NaN",
            "1e99s",
            "31536001s",
            "31536001",
            "18446744073709551615",
            "secret",
        ] {
            assert_eq!(reset_at(value, now), None, "{value}");
        }
    }

    #[test]
    fn cache_cannot_cross_provider_identity_or_configuration_directory() {
        let first = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-ratelimit-limit-requests", "10".parse().unwrap());
        let fingerprint = identity("https://proxy", "first-key", &HashMap::new());
        capture(
            first.path(),
            "litellm",
            fingerprint.clone(),
            "model",
            &headers,
        );
        assert_eq!(
            get(first.path(), "litellm", &fingerprint)
                .unwrap()
                .requests_limit,
            Some(10)
        );
        assert!(get(
            first.path(),
            "litellm",
            &identity("https://proxy", "second-key", &HashMap::new())
        )
        .is_none());
        assert!(get(other.path(), "litellm", &fingerprint).is_none());
        invalidate(first.path(), "litellm");
        assert!(get(first.path(), "litellm", &fingerprint).is_none());
    }

    #[test]
    fn reset_expiry_and_auth_header_changes_invalidate_observations() {
        let dir = tempfile::tempdir().unwrap();
        let headers = HashMap::from([("x-tenant".to_string(), "one".to_string())]);
        let fingerprint = identity("https://proxy", "key", &headers);
        let changed = identity(
            "https://proxy",
            "key",
            &HashMap::from([("x-tenant".to_string(), "two".to_string())]),
        );
        assert_ne!(fingerprint, changed);
        let mut response = reqwest::header::HeaderMap::new();
        response.insert("x-ratelimit-remaining-requests", "0".parse().unwrap());
        response.insert(
            "x-ratelimit-reset-requests",
            "2000-01-01T00:00:00Z".parse().unwrap(),
        );
        capture(
            dir.path(),
            "litellm",
            fingerprint.clone(),
            "model",
            &response,
        );
        assert!(get(dir.path(), "litellm", &fingerprint).is_none());
    }
}
