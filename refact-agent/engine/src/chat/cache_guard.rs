use std::sync::Arc;

use serde_json::{Map, Value};
use similar::{Algorithm, TextDiff};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};

use crate::global_context::GlobalContext;
use crate::providers::traits::ModelPricing;
use crate::tokens::{cached_tokenizer, count_text_tokens_with_fallback};

const CACHE_GUARD_TOOL_NAME: &str = "cache_guard";
const CACHE_GUARD_TOOL_CALL_ID: &str = "cacheguard_force";
const CACHE_GUARD_RULE: &str = "Prompt cache prefix violation";

const IGNORED_KEYS: &[&str] = &[
    "cache_control",
    "request_attempt_id",
    "temperature",
    "max_tokens",
    "max_completion_tokens",
    "max_output_tokens",
    "frequency_penalty",
    "stop",
    "stop_sequences",
    "n",
    "usage",
    "finish_reason",
    "checkpoints",
    "provider_specific_fields",
];

pub fn is_cache_guard_pause_id(tool_call_id: &str) -> bool {
    tool_call_id.starts_with("cacheguard_")
}

pub fn is_cache_guard_pause_reason(reason: &crate::chat::types::PauseReason) -> bool {
    reason.tool_name == CACHE_GUARD_TOOL_NAME || is_cache_guard_pause_id(&reason.tool_call_id)
}

pub async fn is_guard_enabled_for_model(
    gcx: Arc<ARwLock<GlobalContext>>,
    model_id: &str,
) -> bool {
    let Some(pricing) = get_model_pricing(&gcx, model_id).await else {
        return false;
    };
    pricing.cache_read.is_some() || pricing.cache_creation.is_some()
}

pub fn sanitize_body_for_cache_guard(value: &Value) -> Value {
    sanitize_value(value)
}

pub fn is_append_only_prefix(prev: &Value, next: &Value) -> bool {
    is_append_only_prefix_inner(prev, next, true, None)
}

fn is_append_only_prefix_inner(
    prev: &Value,
    next: &Value,
    strict_object: bool,
    parent_key: Option<&str>,
) -> bool {
    match (prev, next) {
        (Value::Null, Value::Null)
        | (Value::Bool(_), Value::Bool(_))
        | (Value::Number(_), Value::Number(_))
        | (Value::String(_), Value::String(_)) => prev == next,
        (Value::Array(a), Value::Array(b)) => {
            if a.len() > b.len() {
                return false;
            }
            let is_messages_array = parent_key == Some("messages");
            a.iter().zip(b.iter()).all(|(old_item, new_item)| {
                is_append_only_prefix_inner(old_item, new_item, is_messages_array, None)
            })
        }
        (Value::Object(a), Value::Object(b)) => {
            if strict_object {
                if a.len() != b.len() {
                    return false;
                }
                if !a.keys().all(|k| b.contains_key(k)) {
                    return false;
                }
            }
            a.iter().all(|(k, old_v)| {
                b.get(k)
                    .map(|new_v| is_append_only_prefix_inner(old_v, new_v, false, Some(k)))
                    .unwrap_or(false)
            })
        }
        _ => false,
    }
}

pub fn unified_json_diff(prev: &Value, next: &Value) -> String {
    let prev_pretty = serde_json::to_string_pretty(prev).unwrap_or_else(|_| prev.to_string());
    let next_pretty = serde_json::to_string_pretty(next).unwrap_or_else(|_| next.to_string());

    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .diff_lines(&prev_pretty, &next_pretty);

    diff.unified_diff()
        .context_radius(6)
        .header("previous", "current")
        .to_string()
}

pub async fn estimate_extra_cache_miss_usd(
    gcx: Arc<ARwLock<GlobalContext>>,
    model_id: &str,
    previous_sanitized: &Value,
) -> Option<f64> {
    let pricing = get_model_pricing(&gcx, model_id).await?;
    let cache_read_rate = pricing.cache_read?;
    if pricing.prompt <= cache_read_rate {
        return Some(0.0);
    }

    let previous_pretty = serde_json::to_string_pretty(previous_sanitized).ok()?;
    let model_rec = {
        let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
            .await
            .ok()?;
        crate::caps::resolve_chat_model(caps, model_id).ok()?
    };
    let tokenizer = cached_tokenizer(gcx, &model_rec.base).await.ok().flatten();
    let cached_tokens = count_text_tokens_with_fallback(tokenizer, &previous_pretty);
    let delta_rate = pricing.prompt - cache_read_rate;
    Some((cached_tokens as f64) * delta_rate / 1_000_000.0)
}

pub async fn check_or_pause_cache_guard(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<crate::chat::types::ChatSession>>,
    model_id: &str,
    request_body: &Value,
) -> Result<Option<Value>, String> {
    if !is_guard_enabled_for_model(gcx.clone(), model_id).await {
        return Ok(None);
    }

    let sanitized = sanitize_body_for_cache_guard(request_body);

    let maybe_violation_prev = {
        let mut session = session_arc.lock().await;
        if session.cache_guard_force_next {
            session.cache_guard_force_next = false;
            None
        } else if let Some(prev) = session.cache_guard_snapshot.as_ref() {
            if is_append_only_prefix(prev, &sanitized) {
                None
            } else {
                Some(prev.clone())
            }
        } else {
            None
        }
    };

    let Some(previous) = maybe_violation_prev else {
        return Ok(Some(sanitized));
    };

    let diff = unified_json_diff(&previous, &sanitized);
    let estimated_extra_usd = estimate_extra_cache_miss_usd(gcx.clone(), model_id, &previous).await;

    {
        let mut session = session_arc.lock().await;
        session.discard_draft_for_pause();
        session
            .abort_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let mut summary = format!(
            "Prompt cache append-only prefix check failed for model `{}`.\n\n",
            model_id
        );
        if let Some(extra) = estimated_extra_usd {
            summary.push_str(&format!(
                "Estimated extra cost if cache miss occurs: `${:.6}` USD.\n\n",
                extra
            ));
        }
        summary.push_str("Unified diff (sanitized provider request body):\n\n");
        summary.push_str("```diff\n");
        summary.push_str(&diff);
        summary.push_str("\n```\n");

        let reasons = vec![crate::chat::types::PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: CACHE_GUARD_TOOL_NAME.to_string(),
            command: summary,
            rule: CACHE_GUARD_RULE.to_string(),
            tool_call_id: CACHE_GUARD_TOOL_CALL_ID.to_string(),
            integr_config_path: None,
        }];
        session.set_paused_with_reasons_and_auto_approved(reasons, Vec::new(), None);
    }

    Err("Cache guard: request blocked due to prompt prefix violation".to_string())
}

pub async fn commit_cache_guard_snapshot(
    session_arc: Arc<AMutex<crate::chat::types::ChatSession>>,
    sanitized_body: Value,
) {
    let mut session = session_arc.lock().await;
    session.cache_guard_snapshot = Some(sanitized_body);
}

async fn get_model_pricing(
    gcx: &Arc<ARwLock<GlobalContext>>,
    model_id: &str,
) -> Option<ModelPricing> {
    let parts: Vec<&str> = model_id.splitn(2, '/').collect();
    if parts.len() != 2 {
        return None;
    }
    let provider_name = parts[0];
    let model_name = parts[1];

    let gcx_locked = gcx.read().await;
    let registry = gcx_locked.providers.read().await;
    registry
        .get(provider_name)
        .and_then(|provider| provider.model_pricing(model_name))
}

fn sanitize_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                if IGNORED_KEYS.contains(&k.as_str()) {
                    continue;
                }
                out.insert(k.clone(), sanitize_value(v));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sanitize_value).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_sanitize_removes_ignored_fields_recursively() {
        let input = json!({
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "hello", "cache_control": {"type": "ephemeral"}}]},
                {"role": "assistant", "content": "ok", "provider_specific_fields": {"x": 1}}
            ],
            "temperature": 0.3,
            "max_tokens": 1024,
            "meta": {"request_attempt_id": "abc", "chat_id": "x"},
            "reasoning_effort": "high"
        });
        let out = sanitize_body_for_cache_guard(&input);
        assert!(out.get("temperature").is_none());
        assert!(out.get("max_tokens").is_none());
        assert_eq!(out["reasoning_effort"], "high");
        assert!(out["meta"].get("request_attempt_id").is_none());
        assert_eq!(out["meta"]["chat_id"], "x");

        let first_content = out["messages"][0]["content"].as_array().unwrap();
        assert!(first_content[0].get("cache_control").is_none());
        assert!(out["messages"][1].get("provider_specific_fields").is_none());
    }

    #[test]
    fn test_append_only_prefix_objects_and_arrays() {
        let prev = json!({"messages": [1, 2], "meta": {"a": 1}});
        let next = json!({"messages": [1, 2, 3], "meta": {"a": 1, "b": 2}});
        assert!(is_append_only_prefix(&prev, &next));

        let bad = json!({"messages": [1, 99, 3], "meta": {"a": 1}});
        assert!(!is_append_only_prefix(&prev, &bad));
    }

    #[test]
    fn test_append_only_prefix_strict_top_level_keys() {
        let prev = json!({"messages": [1, 2], "meta": {"a": 1}});
        let next = json!({"messages": [1, 2, 3], "meta": {"a": 1}, "extra": true});
        assert!(!is_append_only_prefix(&prev, &next));
    }

    #[test]
    fn test_append_only_prefix_messages_keys_strict() {
        let prev = json!({
            "messages": [
                {"role": "user", "content": "hi"}
            ]
        });
        let next = json!({
            "messages": [
                {"role": "user", "content": "hi", "extra": true}
            ]
        });
        assert!(!is_append_only_prefix(&prev, &next));
    }

    #[test]
    fn test_cache_guard_pause_reason_detection() {
        let reason = crate::chat::types::PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "cache_guard".to_string(),
            command: String::new(),
            rule: String::new(),
            tool_call_id: "cacheguard_force".to_string(),
            integr_config_path: None,
        };
        assert!(is_cache_guard_pause_reason(&reason));
        assert!(is_cache_guard_pause_id("cacheguard_force_once"));
        assert!(!is_cache_guard_pause_id("call_123"));
    }
}
