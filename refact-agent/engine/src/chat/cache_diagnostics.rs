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
        .or_else(|| body.get("instructions"))
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
