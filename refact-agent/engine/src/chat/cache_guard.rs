use std::sync::Arc;

use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::call_validation::ChatMessage;
use crate::chat::internal_roles::{event, EventSubkind};
use similar::{Algorithm, TextDiff};
use tokio::sync::{Mutex as AMutex};

use crate::app_state::AppState;
use crate::chat::types::{ChatSession, TaskMeta};
use crate::tokens::{cached_tokenizer, count_text_tokens_with_fallback};

const CACHE_GUARD_TOOL_NAME: &str = "cache_guard";
const CACHE_GUARD_TOOL_CALL_ID: &str = "cacheguard_force";
const CACHE_GUARD_RULE: &str = "Prompt cache prefix violation";
const CACHE_GUARD_PREVIEW_STRING_MAX_CHARS: usize = 240;
const CACHE_GUARD_DIFF_MAX_CHARS: usize = 32 * 1024;
const CACHE_GUARD_DIFF_TRUNCATION_NOTICE: &str = concat!(
    "\n...[cache guard diff truncated: omitted {omitted} chars; ",
    "sanitized provider request body preview exceeded limit]\n"
);

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
    let supports_cache_control =
        crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
            .await
            .ok()
            .and_then(|caps| crate::caps::resolve_chat_model(caps, model_id).ok())
            .map(|record| record.base.supports_cache_control)
            .unwrap_or(false);
    if supports_cache_control {
        return true;
    }

    crate::providers::pricing::lookup_model_pricing(&app.gcx, model_id)
        .await
        .is_some_and(|pricing| pricing.cache_read.is_some() || pricing.cache_creation.is_some())
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
            // The "tools" array is part of the prompt prefix — any change (including
            // appending a new tool) invalidates the LLM cache. Require strict equality.
            if parent_key == Some("tools") {
                return a == b;
            }
            if a.len() > b.len() {
                return false;
            }
            let is_strict_array_item = matches!(parent_key, Some("messages" | "input"));
            a.iter().zip(b.iter()).all(|(old_item, new_item)| {
                is_append_only_prefix_inner(old_item, new_item, is_strict_array_item, None)
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
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| preview_value_for_cache_guard_diff(value, parent_key))
                .collect(),
        ),
        Value::String(text) => {
            Value::String(redact_and_truncate_cache_guard_preview(text, parent_key))
        }
        scalar => scalar.clone(),
    }
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
        return Ok(CacheGuardOutcome::Pass(Some(sanitized)));
    };

    let diff = cap_cache_guard_diff(&unified_json_diff(
        &preview_body_for_cache_guard_diff(&previous),
        &preview_body_for_cache_guard_diff(&sanitized),
    ));
    let estimated_extra_usd = estimate_extra_cache_miss_usd(app.clone(), model_id, &previous).await;

    let reason = {
        let mut session = session_arc.lock().await;
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

pub async fn commit_cache_guard_snapshot(
    session_arc: Arc<AMutex<crate::chat::types::ChatSession>>,
    sanitized_body: Value,
) {
    let mut session = session_arc.lock().await;
    session.cache_guard_snapshot = Some(sanitized_body);
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
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let mut model_caps = std::collections::HashMap::new();
        model_caps.insert(
            model_id.to_string(),
            crate::caps::model_caps::ModelCapabilities {
                n_ctx: 128_000,
                max_output_tokens: 16_384,
                pricing: Some(crate::providers::traits::ModelPricing {
                    prompt: 2.5,
                    generated: 10.0,
                    cache_read: Some(1.25),
                    cache_creation: None,
                    context_over_200k: None,
                }),
                ..Default::default()
            },
        );
        {
            let mut caps = app.model.caps.write().await;
            caps.caps = Some(std::sync::Arc::new(crate::caps::CodeAssistantCaps {
                model_caps: std::sync::Arc::new(model_caps),
                ..Default::default()
            }));
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

    #[tokio::test]
    async fn cache_guard_enabled_for_normal_cache_priced_chat() {
        let app = app_with_cache_priced_model("test/model-with-cache").await;
        let session = session_with_task_role(None);

        assert!(is_guard_enabled(app, "test/model-with-cache", &session).await);
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
        let uncapped_diff = unified_json_diff(
            &preview_body_for_cache_guard_diff(&sanitize_body_for_cache_guard(&prev_body)),
            &preview_body_for_cache_guard_diff(&sanitize_body_for_cache_guard(&next_body)),
        );
        assert!(uncapped_diff.chars().count() > CACHE_GUARD_DIFF_MAX_CHARS);
        assert!(reason.chars().count() < CACHE_GUARD_DIFF_MAX_CHARS + 1_000);
        assert!(reason.contains("cache guard diff truncated"));
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
