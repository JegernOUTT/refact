use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use refact_core::chat_types::ChatMessage;

pub fn canonical_source_value(message: &ChatMessage) -> Value {
    let mut value = serde_json::to_value(message).unwrap_or_else(|_| json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.remove("message_id");
        obj.remove("id");
    }
    value
}

pub fn source_hash_for_messages(messages: &[ChatMessage]) -> String {
    let canonical: Vec<Value> = messages.iter().map(canonical_source_value).collect();
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}
