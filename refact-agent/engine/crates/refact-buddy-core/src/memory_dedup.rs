use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use sha2::{Digest, Sha256};

pub const NEAR_DUPLICATE_JACCARD: f64 = 0.85;
pub const MIN_TOKENS_FOR_NEAR_DUPLICATE: usize = 8;

fn iso_datetime_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\d{4}-\d{2}-\d{2}[Tt _]?(\d{2}:\d{2}(:\d{2})?(\.\d+)?([Zz]|[+-]\d{2}:?\d{2})?)?",
        )
        .unwrap()
    })
}

fn time_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b\d{1,2}:\d{2}(:\d{2})?\b").unwrap())
}

fn uuid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b").unwrap()
    })
}

fn hex_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b[0-9a-f]{8,}\b").unwrap())
}

fn number_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\d+([.,]\d+)*").unwrap())
}

pub fn normalize_memory_text(text: &str) -> String {
    let lowered = text.to_lowercase();
    let step = iso_datetime_re().replace_all(&lowered, " ");
    let step = time_re().replace_all(&step, " ");
    let step = uuid_re().replace_all(&step, " ");
    let step = hex_id_re().replace_all(&step, |caps: &regex::Captures| {
        if caps[0].chars().any(|c| c.is_ascii_digit()) {
            " ".to_string()
        } else {
            caps[0].to_string()
        }
    });
    let step = number_re().replace_all(&step, "#");
    step.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn content_signature(text: &str) -> String {
    let normalized = normalize_memory_text(text);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn token_set(text: &str) -> HashSet<String> {
    normalize_memory_text(text)
        .split(|c: char| !c.is_alphanumeric() && c != '#')
        .filter(|token| token.len() >= 2)
        .map(|token| token.to_string())
        .collect()
}

pub fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;
    if union == 0 {
        return 1.0;
    }
    intersection as f64 / union as f64
}

pub fn near_duplicate(a: &str, b: &str) -> bool {
    let tokens_a = token_set(a);
    let tokens_b = token_set(b);
    if tokens_a.len() < MIN_TOKENS_FOR_NEAR_DUPLICATE
        || tokens_b.len() < MIN_TOKENS_FOR_NEAR_DUPLICATE
    {
        return normalize_memory_text(a) == normalize_memory_text(b);
    }
    jaccard_similarity(&tokens_a, &tokens_b) >= NEAR_DUPLICATE_JACCARD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_stable_across_volatile_details() {
        let sig_a = content_signature("Signal hash fd0347ef: 43 repeated llm_errors at 12:30:01");
        let sig_b = content_signature("signal hash ab12cd34: 17 repeated llm_errors at 09:15:59");
        assert_eq!(sig_a, sig_b);
    }

    #[test]
    fn near_duplicate_detects_rephrased_repeats() {
        let a = "Pattern observed: 42 llm_error diagnostics clustered at chat generation, \
                 mostly caps-registry drift where the default model id is no longer served \
                 by the server. Fix is in user config, update default models.";
        let b = "Pattern observed: 47 llm_error diagnostics clustered at chat generation, \
                 mostly caps-registry drift where the default model id is no longer served \
                 by the server. Fix is in user config, update the default models slot.";
        assert!(near_duplicate(a, b));
    }

    #[test]
    fn near_duplicate_rejects_distinct_insights() {
        let a = "The compaction loop retries eight times before surfacing the context error \
                 to the user with a pointer at manual trimming tools.";
        let b = "Voice rendering caches the pulse one-liner for five minutes which replays \
                 identical lines when the pulse is quiet.";
        assert!(!near_duplicate(a, b));
    }
}
