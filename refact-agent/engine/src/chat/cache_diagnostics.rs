use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let sorted: BTreeMap<_, _> = object
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize_json(value)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        scalar => scalar.clone(),
    }
}

pub fn sha256_hex(value: &Value) -> String {
    let canonical = canonicalize_json(value);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    hex::encode(Sha256::digest(bytes))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRequestHashes {
    pub tools_sha256: Option<String>,
    pub system_sha256: Option<String>,
    pub messages_sha256: Option<String>,
    pub full_body_sha256: String,
}

pub fn compute_provider_request_hashes(body: &Value) -> ProviderRequestHashes {
    let messages = body.get("messages").or_else(|| body.get("input"));
    let system = body
        .get("system")
        .or_else(|| first_system_message(messages));

    ProviderRequestHashes {
        tools_sha256: body.get("tools").map(sha256_hex),
        system_sha256: system.map(sha256_hex),
        messages_sha256: messages.map(sha256_hex),
        full_body_sha256: sha256_hex(body),
    }
}

pub fn log_provider_request_hashes(chat_id: Option<&str>, model: &str, body: &Value) {
    if !tracing::enabled!(target: "cache_diag", tracing::Level::INFO) {
        return;
    }

    let hashes = compute_provider_request_hashes(body);
    let tools_sha = hashes.tools_sha256.as_deref();
    let system_sha = hashes.system_sha256.as_deref();
    let messages_sha = hashes.messages_sha256.as_deref();

    tracing::info!(
        target: "cache_diag",
        chat_id = ?chat_id,
        model = ?model,
        tools = ?tools_sha,
        system = ?system_sha,
        messages = ?messages_sha,
        full = ?hashes.full_body_sha256,
        "provider_request_hashes"
    );
}

fn first_system_message(messages: Option<&Value>) -> Option<&Value> {
    messages?
        .as_array()?
        .iter()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("system"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_hash_is_key_order_independent() {
        let first = json!({"b": 1, "a": 2});
        let second = json!({"a": 2, "b": 1});

        assert_eq!(sha256_hex(&first), sha256_hex(&second));
        assert_eq!(canonicalize_json(&first), canonicalize_json(&second));
    }

    #[test]
    fn array_order_changes_hash() {
        let first = json!([1, 2, 3]);
        let second = json!([3, 2, 1]);

        assert_ne!(sha256_hex(&first), sha256_hex(&second));
    }

    #[test]
    fn anthropic_body_hashes_sections() {
        let body = json!({
            "model": "claude-sonnet-4",
            "tools": [{"name": "search", "input_schema": {"type": "object"}}],
            "system": [{"type": "text", "text": "You are helpful"}],
            "messages": [{"role": "user", "content": "hello"}],
        });

        let hashes = compute_provider_request_hashes(&body);
        let hashes_again = compute_provider_request_hashes(&body);

        assert_eq!(hashes.tools_sha256, Some(sha256_hex(&body["tools"])));
        assert_eq!(hashes.system_sha256, Some(sha256_hex(&body["system"])));
        assert_eq!(hashes.messages_sha256, Some(sha256_hex(&body["messages"])));
        assert_eq!(hashes.full_body_sha256, sha256_hex(&body));
        assert_eq!(hashes.full_body_sha256, hashes_again.full_body_sha256);
    }

    #[test]
    fn body_without_tools_has_no_tools_hash() {
        let body = json!({
            "model": "gpt-5",
            "messages": [{"role": "user", "content": "hello"}],
        });

        let hashes = compute_provider_request_hashes(&body);

        assert_eq!(hashes.tools_sha256, None);
        assert_eq!(hashes.messages_sha256, Some(sha256_hex(&body["messages"])));
    }

    #[test]
    fn system_hash_falls_back_to_openai_system_message() {
        let body = json!({
            "model": "gpt-5",
            "messages": [
                {"role": "user", "content": "before"},
                {"role": "system", "content": "rules"},
                {"role": "system", "content": "ignored"}
            ],
        });

        let hashes = compute_provider_request_hashes(&body);

        assert_eq!(hashes.system_sha256, Some(sha256_hex(&body["messages"][1])));
    }

    #[test]
    fn responses_input_is_used_for_messages_hash_and_system_fallback() {
        let body = json!({
            "model": "gpt-5",
            "input": [
                {"role": "system", "content": "rules"},
                {"role": "user", "content": "hello"}
            ],
        });

        let hashes = compute_provider_request_hashes(&body);

        assert_eq!(hashes.messages_sha256, Some(sha256_hex(&body["input"])));
        assert_eq!(hashes.system_sha256, Some(sha256_hex(&body["input"][0])));
    }
}
