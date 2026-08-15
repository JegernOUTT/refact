use std::collections::HashSet;

lazy_static::lazy_static! {
    static ref INVALID_REFRESH_TOKENS: std::sync::Mutex<HashSet<String>> =
        std::sync::Mutex::new(HashSet::new());
}

pub fn is_permanent_refresh_error(error: &str) -> bool {
    if let Some(value) = extract_json_object(error) {
        if json_contains_invalid_grant(&value) {
            return true;
        }
    }
    error.to_ascii_lowercase().contains("invalid_grant")
}

fn extract_json_object(text: &str) -> Option<serde_json::Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

fn json_contains_invalid_grant(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => text.eq_ignore_ascii_case("invalid_grant"),
        serde_json::Value::Array(values) => values.iter().any(json_contains_invalid_grant),
        serde_json::Value::Object(map) => map.values().any(json_contains_invalid_grant),
        _ => false,
    }
}

pub fn mark_invalid_refresh_token(provider_name: &str, refresh_token: &str) {
    if refresh_token.is_empty() {
        return;
    }
    if let Ok(mut tokens) = INVALID_REFRESH_TOKENS.lock() {
        tokens.insert(refresh_token_key(provider_name, refresh_token));
    }
}

pub fn is_invalid_refresh_token(provider_name: &str, refresh_token: &str) -> bool {
    if refresh_token.is_empty() {
        return false;
    }
    INVALID_REFRESH_TOKENS
        .lock()
        .map(|tokens| tokens.contains(&refresh_token_key(provider_name, refresh_token)))
        .unwrap_or(false)
}

pub fn clear_invalid_refresh_token(provider_name: &str, refresh_token: &str) {
    if refresh_token.is_empty() {
        return;
    }
    if let Ok(mut tokens) = INVALID_REFRESH_TOKENS.lock() {
        tokens.remove(&refresh_token_key(provider_name, refresh_token));
    }
}

fn refresh_token_key(provider_name: &str, refresh_token: &str) -> String {
    format!("{provider_name}:{refresh_token}")
}
