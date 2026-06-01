use std::sync::Arc;

use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::call_validation::ChatMessage;
use crate::chat::internal_roles::{event, EventSubkind};
use similar::{Algorithm, TextDiff};
use tokio::sync::{Mutex as AMutex};

use crate::app_state::AppState;
use crate::caps::ChatModelRecord;
use crate::chat::types::{ChatSession, TaskMeta};
use crate::tokens::{cached_tokenizer, count_text_tokens_with_fallback};

const CACHE_GUARD_TOOL_NAME: &str = "cache_guard";
const CACHE_GUARD_TOOL_CALL_ID: &str = "cacheguard_force";
const CACHE_GUARD_RULE: &str = "Prompt cache prefix violation";
const CACHE_GUARD_PREVIEW_STRING_MAX_CHARS: usize = 240;
const CACHE_GUARD_PREVIEW_ARRAY_MAX_ITEMS: usize = 24;
const CACHE_GUARD_DIFF_MAX_CHARS: usize = 32 * 1024;
const CACHE_GUARD_DIFF_TRUNCATION_NOTICE: &str = concat!(
    "\n...[cache guard diff truncated: omitted {omitted} chars; ",
    "sanitized provider request body preview exceeded limit]\n"
);
const CACHE_GUARD_DIFF_PREVIEW_NOTICE: &str = concat!(
    "\n[cache guard diff generated from bounded structural preview: ",
    "large arrays use count summaries and text payloads are redacted/truncated]\n"
);

const RECURSIVELY_IGNORED_KEYS: &[&str] = &[
    "cache_control",
    "request_attempt_id",
    "usage",
    "finish_reason",
    "checkpoints",
];

const TOP_LEVEL_IGNORED_KEYS: &[&str] = &[
    "temperature",
    "max_tokens",
    "max_completion_tokens",
    "max_output_tokens",
    "frequency_penalty",
    "presence_penalty",
    "stop",
    "stop_sequences",
    "n",
];

pub enum CacheGuardOutcome {
    Pass(Option<serde_json::Value>),
    Paused { reason: String },
    Error(String),
}

pub fn is_cache_guard_pause_id(tool_call_id: &str) -> bool {
    tool_call_id.starts_with("cacheguard_")
}

pub fn is_cache_guard_pause_reason(reason: &crate::chat::types::PauseReason) -> bool {
    reason.tool_name == CACHE_GUARD_TOOL_NAME || is_cache_guard_pause_id(&reason.tool_call_id)
}

pub fn cache_guard_event_message(payload: Value, summary: impl Into<String>) -> ChatMessage {
    event(
        EventSubkind::SystemNotice,
        "chat.cache_guard",
        payload,
        summary,
    )
}

pub async fn is_guard_enabled(app: AppState, model_id: &str, session: &ChatSession) -> bool {
    is_guard_enabled_for_task_meta(app, model_id, session.thread.task_meta.as_ref()).await
}

async fn is_guard_enabled_for_task_meta(
    app: AppState,
    model_id: &str,
    task_meta: Option<&TaskMeta>,
) -> bool {
    if task_meta.is_some_and(|meta| is_task_management_role(&meta.role)) {
        return false;
    }

    model_supports_cache_guard(app, model_id).await
}

fn is_task_management_role(role: &str) -> bool {
    matches!(role, "planner" | "agent" | "agents" | "task_agent")
}

async fn model_supports_cache_guard(app: AppState, model_id: &str) -> bool {
    crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
        .await
        .ok()
        .and_then(|caps| crate::caps::resolve_chat_model(caps, model_id).ok())
        .is_some_and(|record| chat_model_supports_cache_guard(&record))
}

fn chat_model_supports_cache_guard(record: &ChatModelRecord) -> bool {
    record.base.supports_cache_control
}

pub fn sanitize_body_for_cache_guard(value: &Value) -> Value {
    sanitize_value(value, true)
}

fn body_for_cache_guard(value: &Value) -> Value {
    let sanitized = sanitize_body_for_cache_guard(value);
    if has_anthropic_explicit_cache_markers(value) {
        truncate_body_to_anthropic_cache_prefix(value, sanitized)
    } else {
        sanitized
    }
}

pub fn is_append_only_prefix(prev: &Value, next: &Value) -> bool {
    is_append_only_prefix_inner(prev, next, None, true)
}

fn has_anthropic_explicit_cache_markers(value: &Value) -> bool {
    count_anthropic_cache_control_markers(value) > 0
}

fn count_anthropic_cache_control_markers(value: &Value) -> usize {
    count_cache_control_markers(value.get("system"))
        + count_cache_control_markers(value.get("messages"))
        + count_cache_control_markers(value.get("tools"))
}

fn count_cache_control_markers(value: Option<&Value>) -> usize {
    let Some(value) = value else {
        return 0;
    };
    match value {
        Value::Array(values) => values
            .iter()
            .map(|item| {
                item.get("cache_control").map_or(0, |_| 1)
                    + count_cache_control_markers(item.get("content"))
            })
            .sum(),
        _ => 0,
    }
}

fn truncate_body_to_anthropic_cache_prefix(raw: &Value, mut sanitized: Value) -> Value {
    if let Some((message_index, content_index)) = raw
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| last_message_cache_marker_index(messages))
    {
        truncate_messages_at(&mut sanitized, message_index, content_index);
        return sanitized;
    }

    if let Some(index) = raw
        .get("system")
        .and_then(Value::as_array)
        .and_then(|values| last_direct_cache_marker_index(values))
    {
        truncate_top_level_array_at(&mut sanitized, "system", index);
        remove_top_level_key(&mut sanitized, "messages");
        return sanitized;
    }

    if let Some(index) = raw
        .get("tools")
        .and_then(Value::as_array)
        .and_then(|values| last_direct_cache_marker_index(values))
    {
        truncate_top_level_array_at(&mut sanitized, "tools", index);
        remove_top_level_key(&mut sanitized, "system");
        remove_top_level_key(&mut sanitized, "messages");
    }

    sanitized
}

fn truncate_top_level_array_at(sanitized: &mut Value, key: &str, index: usize) {
    if let Some(values) = sanitized.get_mut(key).and_then(Value::as_array_mut) {
        values.truncate(index + 1);
    }
}

fn truncate_messages_at(sanitized: &mut Value, message_index: usize, content_index: usize) {
    if let Some(messages) = sanitized.get_mut("messages").and_then(Value::as_array_mut) {
        messages.truncate(message_index + 1);
        if let Some(content) = messages
            .get_mut(message_index)
            .and_then(|message| message.get_mut("content"))
            .and_then(Value::as_array_mut)
        {
            content.truncate(content_index + 1);
        }
    }
}

fn remove_top_level_key(sanitized: &mut Value, key: &str) {
    if let Some(object) = sanitized.as_object_mut() {
        object.remove(key);
    }
}

fn last_direct_cache_marker_index(values: &[Value]) -> Option<usize> {
    values
        .iter()
        .rposition(|value| value.get("cache_control").is_some())
}

fn last_message_cache_marker_index(messages: &[Value]) -> Option<(usize, usize)> {
    messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, message)| {
            let content_index = message
                .get("content")
                .and_then(Value::as_array)
                .and_then(|content| last_direct_cache_marker_index(content))?;
            Some((index, content_index))
        })
}

fn is_append_only_prefix_inner(
    prev: &Value,
    next: &Value,
    parent_key: Option<&str>,
    top_level: bool,
) -> bool {
    match (prev, next) {
        (Value::Null, Value::Null)
        | (Value::Bool(_), Value::Bool(_))
        | (Value::Number(_), Value::Number(_))
        | (Value::String(_), Value::String(_)) => prev == next,
        (Value::Array(a), Value::Array(b)) => {
            if matches!(parent_key, Some("messages" | "input")) {
                return a.len() <= b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(old_item, new_item)| old_item == new_item);
            }
            a == b
        }
        (Value::Object(a), Value::Object(b)) => {
            let a_keys = a
                .keys()
                .filter(|key| !is_ignored_key(key, top_level))
                .count();
            let b_keys = b
                .keys()
                .filter(|key| !is_ignored_key(key, top_level))
                .count();
            if a_keys != b_keys {
                return false;
            }
            a.iter()
                .filter(|(key, _)| !is_ignored_key(key, top_level))
                .all(|(key, old_v)| {
                    b.get(key)
                        .map(|new_v| is_append_only_prefix_inner(old_v, new_v, Some(key), false))
                        .unwrap_or(false)
                })
        }
        _ => false,
    }
}

fn is_anthropic_cache_prefix_compatible(prev: &Value, next: &Value) -> bool {
    is_anthropic_cache_prefix_inner(prev, next, None, true, false)
}

fn is_anthropic_cache_prefix_inner(
    prev: &Value,
    next: &Value,
    parent_key: Option<&str>,
    top_level: bool,
    content_append_ok: bool,
) -> bool {
    match (prev, next) {
        (Value::Null, Value::Null)
        | (Value::Bool(_), Value::Bool(_))
        | (Value::Number(_), Value::Number(_))
        | (Value::String(_), Value::String(_)) => prev == next,
        (Value::Array(a), Value::Array(b)) => {
            if matches!(parent_key, Some("messages")) || content_append_ok {
                return a.len() <= b.len()
                    && a.iter().zip(b.iter()).all(|(old_item, new_item)| {
                        is_anthropic_cache_prefix_inner(old_item, new_item, None, false, false)
                    });
            }
            a == b
        }
        (Value::Object(a), Value::Object(b)) => {
            let a_keys = a
                .keys()
                .filter(|key| !is_ignored_key(key, top_level))
                .count();
            let b_keys = b
                .keys()
                .filter(|key| !is_ignored_key(key, top_level))
                .count();
            if a_keys != b_keys {
                return false;
            }
            a.iter()
                .filter(|(key, _)| !is_ignored_key(key, top_level))
                .all(|(key, old_v)| {
                    let allow_content_append = key == "content" && a.get("role") == b.get("role");
                    b.get(key)
                        .map(|new_v| {
                            is_anthropic_cache_prefix_inner(
                                old_v,
                                new_v,
                                Some(key),
                                false,
                                allow_content_append,
                            )
                        })
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

fn cap_cache_guard_diff(diff: &str) -> String {
    let char_count = diff.chars().count();
    if char_count <= CACHE_GUARD_DIFF_MAX_CHARS {
        return diff.to_string();
    }

    let kept: String = diff.chars().take(CACHE_GUARD_DIFF_MAX_CHARS).collect();
    let omitted = char_count - CACHE_GUARD_DIFF_MAX_CHARS;
    let notice = CACHE_GUARD_DIFF_TRUNCATION_NOTICE.replace("{omitted}", &omitted.to_string());
    format!("{kept}{notice}")
}

fn preview_body_for_cache_guard_diff(value: &Value) -> Value {
    preview_value_for_cache_guard_diff(value, None)
}

fn preview_value_for_cache_guard_diff(value: &Value, parent_key: Option<&str>) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                out.insert(
                    key.clone(),
                    preview_value_for_cache_guard_diff(value, Some(key.as_str())),
                );
            }
            Value::Object(out)
        }
        Value::Array(values) => {
            let omitted = values
                .len()
                .saturating_sub(CACHE_GUARD_PREVIEW_ARRAY_MAX_ITEMS);
            let mut preview_values: Vec<Value> = values
                .iter()
                .take(CACHE_GUARD_PREVIEW_ARRAY_MAX_ITEMS)
                .map(|value| preview_value_for_cache_guard_diff(value, parent_key))
                .collect();
            if omitted > 0 {
                preview_values.push(json_summary_cache_guard_preview(format!(
                    "[cache guard preview truncated: total_items={} retained_items={} omitted_items={}]",
                    values.len(),
                    values.len() - omitted,
                    omitted
                )));
            }
            Value::Array(preview_values)
        }
        Value::String(text) => {
            Value::String(redact_and_truncate_cache_guard_preview(text, parent_key))
        }
        scalar => scalar.clone(),
    }
}

fn json_summary_cache_guard_preview(summary: String) -> Value {
    let mut out = Map::new();
    out.insert(
        "__cache_guard_preview__".to_string(),
        Value::String(summary),
    );
    Value::Object(out)
}

fn redact_and_truncate_cache_guard_preview(text: &str, parent_key: Option<&str>) -> String {
    let redacted = if parent_key.is_some_and(is_sensitive_key) {
        "[REDACTED]".to_string()
    } else {
        redact_common_secret_patterns(text)
    };
    if parent_key.is_some_and(is_payload_text_key) {
        return fingerprint_cache_guard_payload(&redacted);
    }
    truncate_cache_guard_preview(&redacted)
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    normalized.contains("api_key")
        || normalized.contains("apikey")
        || normalized.contains("auth_token")
        || normalized.contains("authorization")
        || normalized.contains("bearer")
        || normalized.contains("password")
        || normalized.contains("secret")
        || normalized == "token"
        || normalized.ends_with("_token")
}

fn is_payload_text_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "arguments"
            | "content"
            | "description"
            | "instructions"
            | "m_content"
            | "output"
            | "prompt"
            | "system"
            | "text"
    )
}

fn fingerprint_cache_guard_payload(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let digest_hex = hex::encode(digest);
    format!(
        "[redacted text chars={} sha256={}]",
        text.chars().count(),
        &digest_hex[..16]
    )
}

fn redact_common_secret_patterns(text: &str) -> String {
    let patterns = [
        (r"(?i)Bearer\s+[A-Za-z0-9._~+/=-]+", "Bearer [REDACTED]"),
        (r"sk-[A-Za-z0-9_-]{6,}", "[REDACTED_SECRET]"),
        (
            r#"(?i)([A-Za-z0-9_-]*api[_-]?key\s*[:=]\s*["']?)[^\s&"'`]+"#,
            "${1}[REDACTED]",
        ),
    ];
    let mut redacted = text.to_string();
    for (pattern, replacement) in patterns {
        if let Ok(regex) = Regex::new(pattern) {
            redacted = regex.replace_all(&redacted, replacement).to_string();
        }
    }
    redacted
}

fn truncate_cache_guard_preview(text: &str) -> String {
    let char_count = text.chars().count();
    if char_count <= CACHE_GUARD_PREVIEW_STRING_MAX_CHARS {
        return text.to_string();
    }
    let kept: String = text
        .chars()
        .take(CACHE_GUARD_PREVIEW_STRING_MAX_CHARS)
        .collect();
    format!(
        "{kept}…[truncated {} chars]",
        char_count - CACHE_GUARD_PREVIEW_STRING_MAX_CHARS
    )
}

pub async fn estimate_extra_cache_miss_usd(
    app: AppState,
    model_id: &str,
    previous_sanitized: &Value,
) -> Option<f64> {
    let pricing = crate::providers::pricing::lookup_model_pricing(&app.gcx, model_id).await?;
    let cache_read_rate = pricing.cache_read?;
    if pricing.prompt <= cache_read_rate {
        return Some(0.0);
    }

    let previous_pretty = serde_json::to_string_pretty(previous_sanitized).ok()?;
    let model_rec = {
        let caps = crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
            .await
            .ok()?;
        crate::caps::resolve_chat_model(caps, model_id).ok()?
    };
    let tokenizer = cached_tokenizer(app.gcx, &model_rec.base)
        .await
        .ok()
        .flatten();
    let cached_tokens = count_text_tokens_with_fallback(tokenizer, &previous_pretty);
    let delta_rate = pricing.prompt - cache_read_rate;
    Some((cached_tokens as f64) * delta_rate / 1_000_000.0)
}

pub async fn check_or_pause_cache_guard(
    app: AppState,
    session_arc: Arc<AMutex<crate::chat::types::ChatSession>>,
    model_id: &str,
    request_body: &Value,
) -> Result<CacheGuardOutcome, String> {
    let task_meta = {
        let session = session_arc.lock().await;
        session.thread.task_meta.clone()
    };
    if !is_guard_enabled_for_task_meta(app.clone(), model_id, task_meta.as_ref()).await {
        return Ok(CacheGuardOutcome::Pass(None));
    }

    // OpenAI Responses API stateful mode: when previous_response_id is present,
    // the server handles caching via response chaining. The request body intentionally
    // sends only tail items (not the full conversation), so the append-only prefix
    // check does not apply.
    if request_body
        .get("previous_response_id")
        .is_some_and(|v| !v.is_null())
    {
        return Ok(CacheGuardOutcome::Pass(None));
    }

    let sanitized = body_for_cache_guard(request_body);

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
        return Ok(CacheGuardOutcome::Pass(Some(sanitized)));
    };

    if has_anthropic_explicit_cache_markers(request_body)
        && is_anthropic_cache_prefix_compatible(&previous, &sanitized)
    {
        return Ok(CacheGuardOutcome::Pass(Some(sanitized)));
    }

    let mut diff = unified_json_diff(
        &preview_body_for_cache_guard_diff(&previous),
        &preview_body_for_cache_guard_diff(&sanitized),
    );
    diff.push_str(CACHE_GUARD_DIFF_PREVIEW_NOTICE);
    let diff = cap_cache_guard_diff(&diff);
    let estimated_extra_usd = estimate_extra_cache_miss_usd(app.clone(), model_id, &previous).await;

    let reason = {
        let mut session = session_arc.lock().await;
        if let Some(outcome) = cache_guard_outcome_if_snapshot_changed(
            session.cache_guard_snapshot.as_ref(),
            &previous,
            &sanitized,
        ) {
            return Ok(outcome);
        }
        session.discard_draft_for_pause();
        session
            .abort_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
        session.abort_notify.notify_waiters();

        let mut summary =
            format!("Prompt cache append-only prefix check failed for model `{model_id}`.\n\n");
        if let Some(extra) = estimated_extra_usd {
            summary.push_str(&format!(
                "Estimated extra cost if cache miss occurs: `${extra:.6}` USD.\n\n"
            ));
        }
        summary.push_str("Unified diff (sanitized provider request body):\n\n");
        summary.push_str("```diff\n");
        summary.push_str(&diff);
        summary.push_str("\n```\n");

        let reasons = vec![crate::chat::types::PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: CACHE_GUARD_TOOL_NAME.to_string(),
            command: summary.clone(),
            rule: CACHE_GUARD_RULE.to_string(),
            tool_call_id: CACHE_GUARD_TOOL_CALL_ID.to_string(),
            integr_config_path: None,
        }];
        session.set_paused_with_reasons_and_auto_approved(reasons, Vec::new(), None);
        summary
    };

    Ok(CacheGuardOutcome::Paused { reason })
}

fn cache_guard_outcome_if_snapshot_changed(
    current: Option<&Value>,
    captured_previous: &Value,
    sanitized: &Value,
) -> Option<CacheGuardOutcome> {
    if current == Some(captured_previous) {
        return None;
    }
    let Some(current) = current else {
        return Some(CacheGuardOutcome::Pass(Some(sanitized.clone())));
    };
    if is_append_only_prefix(current, sanitized) {
        Some(CacheGuardOutcome::Pass(Some(sanitized.clone())))
    } else {
        Some(CacheGuardOutcome::Pass(None))
    }
}

// `cache_guard_snapshot` is an in-memory-only canonical provider request body for
// cache-prefix comparison; trajectory persistence intentionally omits it.
pub async fn commit_cache_guard_snapshot(
    session_arc: Arc<AMutex<crate::chat::types::ChatSession>>,
    sanitized_body: Value,
) {
    let mut session = session_arc.lock().await;
    session.cache_guard_snapshot = Some(sanitized_body);
}

fn is_ignored_key(key: &str, top_level: bool) -> bool {
    RECURSIVELY_IGNORED_KEYS.contains(&key) || top_level && TOP_LEVEL_IGNORED_KEYS.contains(&key)
}

fn sanitize_value(value: &Value, top_level: bool) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                if is_ignored_key(key, top_level) {
                    continue;
                }
                out.insert(key.clone(), sanitize_value(value, false));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|value| sanitize_value(value, false))
                .collect(),
        ),
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
                {"role": "assistant", "content": "ok", "provider_specific_fields": {"x": 1, "cache_control": {"type": "ephemeral"}}}
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
        assert_eq!(out["messages"][1]["provider_specific_fields"]["x"], 1);
        assert!(out["messages"][1]["provider_specific_fields"]
            .get("cache_control")
            .is_none());
    }

    #[test]
    fn test_top_level_generation_options_are_ignored() {
        let prev = sanitize_body_for_cache_guard(&json!({
            "messages": [{"role": "user", "content": "hi"}],
            "temperature": 0.1,
            "max_tokens": 1024,
            "stop": ["old"],
            "presence_penalty": 0.0
        }));
        let next = sanitize_body_for_cache_guard(&json!({
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"}
            ],
            "temperature": 0.9,
            "max_tokens": 2048,
            "stop": ["new"],
            "presence_penalty": 1.0
        }));

        assert!(prev.get("temperature").is_none());
        assert!(prev.get("max_tokens").is_none());
        assert!(prev.get("stop").is_none());
        assert!(prev.get("presence_penalty").is_none());
        assert!(is_append_only_prefix(&prev, &next));
    }

    #[test]
    fn test_nested_generation_named_keys_remain_semantic() {
        let prev = sanitize_body_for_cache_guard(&json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "configure",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "max_tokens": {"type": "integer"},
                            "stop": {"type": "string"}
                        }
                    }
                }
            }],
            "provider_specific_fields": {
                "generation_limits": {"max_tokens": 100, "stop": "END"}
            }
        }));
        let next = sanitize_body_for_cache_guard(&json!({
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"}
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "configure",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "max_tokens": {"type": "string"},
                            "stop": {"type": "boolean"}
                        }
                    }
                }
            }],
            "provider_specific_fields": {
                "generation_limits": {"max_tokens": 200, "stop": "DONE"}
            }
        }));

        assert!(prev["tools"][0]["function"]["parameters"]["properties"]
            .get("max_tokens")
            .is_some());
        assert!(prev["tools"][0]["function"]["parameters"]["properties"]
            .get("stop")
            .is_some());
        assert_eq!(
            prev["provider_specific_fields"]["generation_limits"]["max_tokens"],
            100
        );
        assert_eq!(
            prev["provider_specific_fields"]["generation_limits"]["stop"],
            "END"
        );
        assert!(!is_append_only_prefix(&prev, &next));
    }

    #[test]
    fn test_provider_specific_fields_semantic_mutation_breaks_prefix() {
        let prev = sanitize_body_for_cache_guard(&json!({
            "messages": [{"role": "user", "content": "hi"}],
            "provider_specific_fields": {
                "reasoning": {"effort": "medium"},
                "cache_control": {"type": "ephemeral"}
            }
        }));
        let next = sanitize_body_for_cache_guard(&json!({
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"}
            ],
            "provider_specific_fields": {
                "reasoning": {"effort": "high"},
                "cache_control": {"type": "ephemeral"}
            }
        }));

        assert!(prev["provider_specific_fields"]
            .get("cache_control")
            .is_none());
        assert!(!is_append_only_prefix(&prev, &next));
    }

    #[test]
    fn test_append_only_prefix_objects_and_arrays() {
        let prev = json!({"messages": [1, 2], "meta": {"a": 1}});
        let next = json!({"messages": [1, 2, 3], "meta": {"a": 1}});
        assert!(is_append_only_prefix(&prev, &next));

        let bad_message = json!({"messages": [1, 99, 3], "meta": {"a": 1}});
        assert!(!is_append_only_prefix(&prev, &bad_message));

        let bad_meta = json!({"messages": [1, 2, 3], "meta": {"a": 1, "b": 2}});
        assert!(!is_append_only_prefix(&prev, &bad_meta));
    }

    #[test]
    fn test_append_only_prefix_strict_top_level_keys() {
        let prev = json!({"messages": [1, 2], "meta": {"a": 1}});
        let next = json!({"messages": [1, 2, 3], "meta": {"a": 1}, "extra": true});
        assert!(!is_append_only_prefix(&prev, &next));
    }

    #[test]
    fn test_tools_array_strict_equality() {
        let tool_a =
            json!({"type": "function", "function": {"name": "tool_a", "description": "A"}});
        let tool_b =
            json!({"type": "function", "function": {"name": "tool_b", "description": "B"}});

        // Identical tools → OK
        let prev = json!({"messages": [1], "tools": [tool_a.clone()]});
        let next = json!({"messages": [1, 2], "tools": [tool_a.clone()]});
        assert!(is_append_only_prefix(&prev, &next));

        // New tool appended mid-session → violation (breaks LLM cache prefix)
        let next_extra = json!({"messages": [1, 2], "tools": [tool_a.clone(), tool_b.clone()]});
        assert!(!is_append_only_prefix(&prev, &next_extra));

        // Tool removed mid-session → violation
        let prev2 = json!({"messages": [1], "tools": [tool_a.clone(), tool_b.clone()]});
        let next_removed = json!({"messages": [1, 2], "tools": [tool_a.clone()]});
        assert!(!is_append_only_prefix(&prev2, &next_removed));

        // Tool description changed mid-session → violation
        let tool_a_changed =
            json!({"type": "function", "function": {"name": "tool_a", "description": "Changed"}});
        let next_changed = json!({"messages": [1, 2], "tools": [tool_a_changed]});
        assert!(!is_append_only_prefix(&prev, &next_changed));
    }

    #[test]
    fn test_non_conversation_arrays_are_strict() {
        let prev = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "system": [{"type": "text", "text": "stable"}]
        });
        let appended = json!({
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"}
            ],
            "system": [
                {"type": "text", "text": "stable"},
                {"type": "text", "text": "new prefix"}
            ]
        });
        let nested_key = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "system": [{"type": "text", "text": "stable", "name": "new"}]
        });

        assert!(!is_append_only_prefix(&prev, &appended));
        assert!(!is_append_only_prefix(&prev, &nested_key));
    }

    #[test]
    fn test_non_conversation_scalars_are_strict() {
        let prev = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "instructions": "stable prefix"
        });
        let next = json!({
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"}
            ],
            "instructions": "changed prefix"
        });

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
    fn detects_anthropic_explicit_cache_markers() {
        let body = json!({
            "system": [{"type": "text", "text": "stable", "cache_control": {"type": "ephemeral"}}],
            "tools": [{"name": "search", "input_schema": {"type": "object"}}],
            "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
        });

        assert!(has_anthropic_explicit_cache_markers(&body));
        assert_eq!(count_anthropic_cache_control_markers(&body), 1);
    }

    #[test]
    fn anthropic_cache_guard_body_keeps_only_explicit_cache_prefix() {
        let body = json!({
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "cached", "cache_control": {"type": "ephemeral"}}]},
                {"role": "assistant", "content": [{"type": "text", "text": "uncached suffix"}]}
            ]
        });

        let sanitized = body_for_cache_guard(&body);

        assert_eq!(sanitized["messages"].as_array().unwrap().len(), 1);
        assert_eq!(sanitized["messages"][0]["role"], "user");
        assert!(sanitized.to_string().contains("cached"));
        assert!(!sanitized.to_string().contains("uncached suffix"));
        assert!(!sanitized.to_string().contains("cache_control"));
    }

    #[test]
    fn comparable_anthropic_prefix_allows_marker_moving_forward() {
        let prev = sanitize_body_for_cache_guard(&json!({
            "messages": [{
                "role": "user",
                "content": [{"type": "text", "text": "hello", "cache_control": {"type": "ephemeral"}}]
            }]
        }));
        let next = sanitize_body_for_cache_guard(&json!({
            "messages": [
                {
                    "role": "user",
                    "content": [{"type": "text", "text": "hello"}]
                },
                {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "changed suffix"}]
                },
                {
                    "role": "user",
                    "content": [{"type": "text", "text": "continue", "cache_control": {"type": "ephemeral"}}]
                }
            ]
        }));

        assert!(is_anthropic_cache_prefix_compatible(&prev, &next));
    }

    #[tokio::test]
    async fn cache_guard_allows_anthropic_suffix_changes_after_explicit_cache_marker() {
        let app = app_with_cache_priced_model("test/model-with-cache").await;
        let session = session_with_task_role(None);
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));
        let prev_body = json!({
            "model": "test",
            "messages": [{
                "role": "user",
                "content": [{"type": "text", "text": "hello", "cache_control": {"type": "ephemeral"}}]
            }]
        });
        {
            let mut session = session_arc.lock().await;
            session.cache_guard_snapshot = Some(body_for_cache_guard(&prev_body));
        }

        let next_body = json!({
            "model": "test",
            "messages": [
                {
                    "role": "user",
                    "content": [{"type": "text", "text": "hello"}]
                },
                {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "changed suffix"}]
                },
                {
                    "role": "user",
                    "content": [{"type": "text", "text": "continue", "cache_control": {"type": "ephemeral"}}]
                }
            ]
        });
        let outcome = check_or_pause_cache_guard(
            app,
            session_arc.clone(),
            "test/model-with-cache",
            &next_body,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, CacheGuardOutcome::Pass(Some(_))));
        let session = session_arc.lock().await;
        assert!(session.runtime.pause_reasons.is_empty());
    }

    #[tokio::test]
    async fn cache_guard_pauses_when_anthropic_explicit_cache_prefix_changes() {
        let app = app_with_cache_priced_model("test/model-with-cache").await;
        let session = session_with_task_role(None);
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));
        let prev_body = json!({
            "model": "test",
            "messages": [{
                "role": "user",
                "content": [{"type": "text", "text": "hello", "cache_control": {"type": "ephemeral"}}]
            }]
        });
        {
            let mut session = session_arc.lock().await;
            session.cache_guard_snapshot = Some(body_for_cache_guard(&prev_body));
        }

        let next_body = json!({
            "model": "test",
            "messages": [{
                "role": "user",
                "content": [{"type": "text", "text": "changed", "cache_control": {"type": "ephemeral"}}]
            }]
        });
        let outcome = check_or_pause_cache_guard(
            app,
            session_arc.clone(),
            "test/model-with-cache",
            &next_body,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, CacheGuardOutcome::Paused { .. }));
        let session = session_arc.lock().await;
        assert_eq!(session.runtime.pause_reasons.len(), 1);
    }

    #[test]
    fn test_append_only_prefix_input_keys_strict() {
        let prev = json!({
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}
            ]
        });
        let next = json!({
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}], "extra": true}
            ]
        });
        assert!(!is_append_only_prefix(&prev, &next));
    }

    #[test]
    fn test_append_only_prefix_messages_nested_content_keys_strict() {
        let prev = json!({
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "hi"}]}
            ]
        });
        let next = json!({
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "hi", "extra": true}]}
            ]
        });
        assert!(!is_append_only_prefix(&prev, &next));
    }

    #[test]
    fn test_append_only_prefix_input_nested_content_keys_strict() {
        let prev = json!({
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}
            ]
        });
        let next = json!({
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi", "extra": true}]}
            ]
        });
        assert!(!is_append_only_prefix(&prev, &next));
    }

    #[test]
    fn test_append_only_prefix_allows_appended_messages_and_input_items() {
        let prev = json!({
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "hi"}]}
            ],
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}
            ]
        });
        let next = json!({
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "hi"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "hello"}]}
            ],
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "hello"}]}
            ]
        });
        assert!(is_append_only_prefix(&prev, &next));
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

    #[test]
    fn cache_guard_diff_preview_redacts_secrets_and_truncates_large_strings() {
        let body = json!({
            "messages": [{
                "role": "user",
                "content": format!("Bearer sk-secret-123 api_key=abcd {}", "x".repeat(400))
            }],
            "metadata": {
                "auth_token": "raw-token-value",
                "safe_preview": "x".repeat(400)
            }
        });

        let preview = preview_body_for_cache_guard_diff(&body);
        let preview_text = serde_json::to_string(&preview).unwrap();

        assert!(!preview_text.contains("sk-secret-123"));
        assert!(!preview_text.contains("api_key=abcd"));
        assert!(!preview_text.contains("raw-token-value"));
        assert!(preview_text.contains("[redacted text chars="));
        assert!(preview_text.contains("sha256="));
        assert!(preview_text.contains("[truncated"));
    }

    #[test]
    fn cache_guard_diff_preview_omitted_arrays_use_count_summary() {
        let secret_tail = (0..64)
            .map(|i| json!({"api_key": format!("sk-omitted-secret-{i}")}))
            .collect::<Vec<_>>();
        let raw_digest = {
            let digest = Sha256::digest(serde_json::to_vec(&secret_tail).unwrap());
            let digest_hex = hex::encode(digest);
            digest_hex[..16].to_string()
        };
        let mut values = vec![json!({"safe": "kept"}); CACHE_GUARD_PREVIEW_ARRAY_MAX_ITEMS];
        values.extend(secret_tail);
        let body = json!({"metadata": values});

        let preview = preview_body_for_cache_guard_diff(&body);
        let preview_text = serde_json::to_string(&preview).unwrap();

        assert!(preview_text.contains("total_items=88"));
        assert!(preview_text.contains("retained_items=24"));
        assert!(preview_text.contains("omitted_items=64"));
        assert!(!preview_text.contains("omitted_sha256"));
        assert!(!preview_text.contains(&raw_digest));
        assert!(!preview_text.contains("sk-omitted-secret"));
    }

    #[test]
    fn cache_guard_diff_preview_huge_omitted_arrays_are_bounded() {
        let mut values = vec![json!({"safe": "kept"}); CACHE_GUARD_PREVIEW_ARRAY_MAX_ITEMS];
        values.extend((0..10_000).map(
            |i| json!({"secret": format!("sk-huge-omitted-secret-{i}-{}", "x".repeat(1024))}),
        ));
        let body = json!({"metadata": values});
        let started = std::time::Instant::now();

        let preview = preview_body_for_cache_guard_diff(&body);
        let elapsed = started.elapsed();
        let preview_text = serde_json::to_string(&preview).unwrap();

        assert!(elapsed < std::time::Duration::from_secs(2));
        assert!(preview_text.contains("total_items=10024"));
        assert!(preview_text.contains("omitted_items=10000"));
        assert!(!preview_text.contains("sk-huge-omitted-secret"));
        assert!(preview_text.len() < 2_000);
    }

    #[test]
    fn cache_guard_emits_event_not_user_message() {
        let message = cache_guard_event_message(
            json!({"model": "test/model", "reason": "prefix_changed"}),
            "cache guard probe",
        );

        assert_eq!(message.role, "event");
        assert_ne!(message.role, "user");
        let event = message.extra.get("event").unwrap();
        assert_eq!(event["subkind"], "system_notice");
        assert_eq!(event["source"], "chat.cache_guard");
        assert_eq!(event["payload"]["reason"], "prefix_changed");
        assert_eq!(message.content.content_text_only(), "cache guard probe");
    }

    async fn app_with_cache_priced_model(model_id: &str) -> AppState {
        app_with_cache_priced_model_support(model_id, true).await
    }

    async fn app_with_cache_priced_model_without_cache_support(model_id: &str) -> AppState {
        app_with_cache_priced_model_support(model_id, false).await
    }

    async fn app_with_cache_priced_model_support(
        model_id: &str,
        supports_cache_control: bool,
    ) -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let pricing = crate::providers::traits::ModelPricing {
            prompt: 2.5,
            generated: 10.0,
            cache_read: Some(1.25),
            cache_creation: None,
            context_over_200k: None,
        };
        let mut model_caps = std::collections::HashMap::new();
        model_caps.insert(
            model_id.to_string(),
            crate::caps::model_caps::ModelCapabilities {
                n_ctx: 128_000,
                max_output_tokens: 16_384,
                supports_cache_control: true,
                pricing: Some(pricing),
                ..Default::default()
            },
        );
        let mut code_caps = crate::caps::CodeAssistantCaps {
            model_caps: std::sync::Arc::new(model_caps),
            ..Default::default()
        };
        code_caps.chat_models.insert(
            model_id.to_string(),
            std::sync::Arc::new(ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: model_id.to_string(),
                    name: model_id.to_string(),
                    n_ctx: 128_000,
                    supports_cache_control,
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
        {
            let mut caps = app.model.caps.write().await;
            caps.caps = Some(std::sync::Arc::new(code_caps));
            caps.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        app
    }

    fn task_meta(role: &str) -> TaskMeta {
        TaskMeta {
            task_id: "task-1".to_string(),
            role: role.to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: None,
        }
    }

    fn session_with_task_role(role: Option<&str>) -> crate::chat::types::ChatSession {
        let mut session = crate::chat::types::ChatSession::new("test-cache-guard".to_string());
        session.thread.task_meta = role.map(task_meta);
        session
    }

    #[test]
    fn cache_guard_stale_snapshot_recheck_passes_without_pause() {
        let captured_previous = sanitize_body_for_cache_guard(&json!({
            "messages": [{"role": "user", "content": "old"}],
            "model": "test"
        }));
        let current = sanitize_body_for_cache_guard(&json!({
            "messages": [{"role": "user", "content": "new"}],
            "model": "test"
        }));
        let sanitized = sanitize_body_for_cache_guard(&json!({
            "messages": [
                {"role": "user", "content": "new"},
                {"role": "assistant", "content": "hello"}
            ],
            "model": "test"
        }));

        let outcome =
            cache_guard_outcome_if_snapshot_changed(Some(&current), &captured_previous, &sanitized);

        assert!(matches!(outcome, Some(CacheGuardOutcome::Pass(Some(_)))));
    }

    #[test]
    fn cache_guard_stale_snapshot_recheck_does_not_commit_over_new_violation() {
        let captured_previous = sanitize_body_for_cache_guard(&json!({
            "messages": [{"role": "user", "content": "old"}],
            "model": "test"
        }));
        let current = sanitize_body_for_cache_guard(&json!({
            "messages": [{"role": "user", "content": "new"}],
            "model": "test"
        }));
        let sanitized = sanitize_body_for_cache_guard(&json!({
            "messages": [{"role": "assistant", "content": "different"}],
            "model": "test"
        }));

        let outcome =
            cache_guard_outcome_if_snapshot_changed(Some(&current), &captured_previous, &sanitized);

        assert!(matches!(outcome, Some(CacheGuardOutcome::Pass(None))));
    }

    #[test]
    fn cache_guard_stale_snapshot_recheck_noops_when_snapshot_unchanged() {
        let captured_previous = sanitize_body_for_cache_guard(&json!({
            "messages": [{"role": "user", "content": "old"}],
            "model": "test"
        }));
        let sanitized = sanitize_body_for_cache_guard(&json!({
            "messages": [{"role": "assistant", "content": "different"}],
            "model": "test"
        }));

        let outcome = cache_guard_outcome_if_snapshot_changed(
            Some(&captured_previous),
            &captured_previous,
            &sanitized,
        );

        assert!(outcome.is_none());
    }

    #[tokio::test]
    async fn cache_guard_enabled_for_normal_cache_supported_chat() {
        let app = app_with_cache_priced_model("test/model-with-cache").await;
        let session = session_with_task_role(None);

        assert!(is_guard_enabled(app, "test/model-with-cache", &session).await);
    }

    #[test]
    fn cache_guard_disabled_for_cache_priced_model_without_cache_support() {
        let record = ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                supports_cache_control: false,
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(
            !chat_model_supports_cache_guard(&record),
            "cache pricing metadata must not enable cache guard without model cache support"
        );
    }

    #[tokio::test]
    async fn cache_guard_enablement_ignores_cache_pricing_without_model_cache_support() {
        let app =
            app_with_cache_priced_model_without_cache_support("test/model-with-cache-price").await;
        let session = session_with_task_role(None);

        assert!(
            !is_guard_enabled(app, "test/model-with-cache-price", &session).await,
            "cache pricing metadata must not enable cache guard without model cache support"
        );
    }

    #[tokio::test]
    async fn cache_guard_disabled_for_task_management_roles() {
        for role in ["planner", "agents", "agent", "task_agent"] {
            let app = app_with_cache_priced_model("test/model-with-cache").await;
            let session = session_with_task_role(Some(role));

            assert!(
                !is_guard_enabled(app, "test/model-with-cache", &session).await,
                "cache guard should be disabled for task role {role}"
            );
        }
    }

    #[tokio::test]
    async fn cache_guard_pass_returns_pass_outcome() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let session = crate::chat::types::ChatSession::new("test-pass".to_string());
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));
        let body = json!({"messages": [{"role": "user", "content": "hi"}]});
        let outcome = check_or_pause_cache_guard(app, session_arc, "some/model", &body)
            .await
            .unwrap();
        assert!(matches!(outcome, CacheGuardOutcome::Pass(_)));
    }

    #[tokio::test]
    async fn cache_guard_violation_in_normal_chat_pauses() {
        let app = app_with_cache_priced_model("test/model-with-cache").await;
        let session = session_with_task_role(None);
        let prev_body =
            json!({"messages": [{"role": "user", "content": "hello"}], "model": "test"});
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));

        {
            let mut s = session_arc.lock().await;
            s.cache_guard_snapshot = Some(sanitize_body_for_cache_guard(&prev_body));
        }

        let next_body =
            json!({"messages": [{"role": "assistant", "content": "hi"}], "model": "test"});
        let outcome = check_or_pause_cache_guard(
            app,
            session_arc.clone(),
            "test/model-with-cache",
            &next_body,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, CacheGuardOutcome::Paused { .. }));
        let session = session_arc.lock().await;
        assert_eq!(session.runtime.pause_reasons.len(), 1);
        assert!(is_cache_guard_pause_reason(
            &session.runtime.pause_reasons[0]
        ));
    }

    #[tokio::test]
    async fn cache_guard_violation_pause_reason_redacts_secret_content() {
        let app = app_with_cache_priced_model("test/model-with-cache").await;
        let session = session_with_task_role(None);
        let prev_body = json!({
            "messages": [{"role": "user", "content": "Bearer sk-secret-123 api_key=abcd"}],
            "model": "test"
        });
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));

        {
            let mut s = session_arc.lock().await;
            s.cache_guard_snapshot = Some(sanitize_body_for_cache_guard(&prev_body));
        }

        let next_body = json!({
            "messages": [{"role": "assistant", "content": "changed Bearer sk-secret-123 api_key=abcd"}],
            "model": "test"
        });
        let outcome = check_or_pause_cache_guard(
            app,
            session_arc.clone(),
            "test/model-with-cache",
            &next_body,
        )
        .await
        .unwrap();

        let CacheGuardOutcome::Paused { reason } = outcome else {
            panic!("cache guard should pause on changed prefix");
        };
        assert!(reason.contains("Prompt cache append-only prefix check failed"));
        assert!(!reason.contains("sk-secret-123"));
        assert!(!reason.contains("api_key=abcd"));
        assert!(reason.contains("[redacted text chars="));
        assert!(reason.contains("sha256="));
        let session = session_arc.lock().await;
        assert_eq!(session.runtime.pause_reasons[0].command, reason);
    }

    #[tokio::test]
    async fn cache_guard_violation_pause_reason_caps_huge_redacted_diff() {
        let app = app_with_cache_priced_model("test/model-with-cache").await;
        let session = session_with_task_role(None);
        let prev_body = json!({
            "input": (0..900).map(|i| json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": format!("safe-prefix-{i}")}]
            })).collect::<Vec<_>>(),
            "metadata": {"auth_token": "raw-token-value"},
            "model": "test"
        });
        let mut next_body = prev_body.clone();
        for item in next_body["input"].as_array_mut().unwrap() {
            item["extra"] = json!("Bearer sk-secret-123 api_key=abcd");
        }
        next_body["metadata"]["auth_token"] = json!("changed-raw-token-value");
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));

        {
            let mut s = session_arc.lock().await;
            s.cache_guard_snapshot = Some(sanitize_body_for_cache_guard(&prev_body));
        }

        let outcome = check_or_pause_cache_guard(
            app,
            session_arc.clone(),
            "test/model-with-cache",
            &next_body,
        )
        .await
        .unwrap();

        let CacheGuardOutcome::Paused { reason } = outcome else {
            panic!("cache guard should pause on changed prefix");
        };
        assert!(reason.chars().count() < CACHE_GUARD_DIFF_MAX_CHARS / 2);
        assert!(reason.contains("bounded structural preview"));
        assert!(reason.contains("cache guard preview truncated"));
        assert!(!reason.contains("sk-secret-123"));
        assert!(!reason.contains("api_key=abcd"));
        assert!(!reason.contains("raw-token-value"));
        assert!(reason.contains("[REDACTED]"));
        let session = session_arc.lock().await;
        assert_eq!(session.runtime.pause_reasons[0].command, reason);
    }

    #[tokio::test]
    async fn cache_guard_violation_in_task_agent_chat_passes_without_pausing() {
        let app = app_with_cache_priced_model("test/model-with-cache").await;
        let session = session_with_task_role(Some("agents"));
        let prev_body =
            json!({"messages": [{"role": "user", "content": "hello"}], "model": "test"});
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));

        {
            let mut s = session_arc.lock().await;
            s.cache_guard_snapshot = Some(sanitize_body_for_cache_guard(&prev_body));
        }

        let next_body =
            json!({"messages": [{"role": "assistant", "content": "hi"}], "model": "test"});
        let outcome = check_or_pause_cache_guard(
            app,
            session_arc.clone(),
            "test/model-with-cache",
            &next_body,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, CacheGuardOutcome::Pass(None)));
        let session = session_arc.lock().await;
        assert!(session.runtime.pause_reasons.is_empty());
    }

    #[tokio::test]
    async fn cache_guard_force_next_allows_one_intentional_reset() {
        let app = app_with_cache_priced_model("test/model-with-cache").await;
        let session = session_with_task_role(None);
        let prev_body =
            json!({"messages": [{"role": "user", "content": "hello"}], "model": "test"});
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));

        {
            let mut s = session_arc.lock().await;
            s.cache_guard_snapshot = Some(sanitize_body_for_cache_guard(&prev_body));
            s.cache_guard_force_next = true;
        }

        let next_body =
            json!({"messages": [{"role": "assistant", "content": "hi"}], "model": "test"});
        let outcome = check_or_pause_cache_guard(
            app,
            session_arc.clone(),
            "test/model-with-cache",
            &next_body,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, CacheGuardOutcome::Pass(Some(_))));
        let session = session_arc.lock().await;
        assert!(!session.cache_guard_force_next);
        assert!(session.runtime.pause_reasons.is_empty());
    }

    #[tokio::test]
    async fn cache_guard_skips_openai_previous_response_id() {
        // When previous_response_id is present, the request body uses tail-only mode
        // (only new messages after last assistant), so the full-body comparison is invalid.
        // The cache guard should skip validation in this case.
        let prev = json!({
            "model": "gpt-4.1",
            "instructions": "You are helpful",
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]}
            ],
            "store": true
        });
        let next_stateful = json!({
            "model": "gpt-4.1",
            "instructions": "You are helpful",
            "input": [
                {"type": "function_call_output", "call_id": "call_1", "output": "result"}
            ],
            "store": true,
            "previous_response_id": "resp_abc123"
        });
        // This SHOULD fail the append-only check (different input, extra key)
        assert!(!is_append_only_prefix(
            &sanitize_body_for_cache_guard(&prev),
            &sanitize_body_for_cache_guard(&next_stateful),
        ));

        let app = app_with_cache_priced_model("test/model-with-cache").await;
        let session = session_with_task_role(None);
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));
        {
            let mut session = session_arc.lock().await;
            session.cache_guard_snapshot = Some(sanitize_body_for_cache_guard(&prev));
        }

        let outcome = check_or_pause_cache_guard(
            app,
            session_arc.clone(),
            "test/model-with-cache",
            &next_stateful,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, CacheGuardOutcome::Pass(None)));
        let session = session_arc.lock().await;
        assert!(session.runtime.pause_reasons.is_empty());
    }
}
