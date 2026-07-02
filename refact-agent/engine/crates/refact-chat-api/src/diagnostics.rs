use std::sync::OnceLock;

use serde_json::{json, Value};
use uuid::Uuid;

use refact_core::retry_policy::{classify_user_error, user_error_info};
use refact_core::string_utils::{redact_sensitive, safe_truncate};

use refact_core::chat_types::{ChatContent, ChatMessage};

const UI_ONLY_MARKER: &str = "_ui_only";
pub const SAFE_PROVIDER_ERROR_DIAGNOSTIC_MAX_CHARS: usize = 2_000;
pub const SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED: &str = "\n...[truncated]";
const SAFE_PROVIDER_ERROR_DIAGNOSTIC_REDACTION_LOOKAHEAD_CHARS: usize = 512;

pub fn is_ui_only_message(msg: &ChatMessage) -> bool {
    msg.extra.get(UI_ONLY_MARKER).and_then(|v| v.as_bool()) == Some(true)
}

pub fn filter_ui_only_messages(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    messages
        .into_iter()
        .filter(|message| !is_ui_only_message(message))
        .collect()
}

fn mark_ui_only(extra: &mut serde_json::Map<String, Value>) {
    extra.insert(UI_ONLY_MARKER.to_string(), Value::Bool(true));
}

pub fn safe_provider_error_diagnostic(error: &str) -> String {
    safe_provider_error_diagnostic_with_limit(error, SAFE_PROVIDER_ERROR_DIAGNOSTIC_MAX_CHARS)
}

pub fn safe_provider_error_diagnostic_with_limit(error: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let (window, source_truncated) = provider_error_diagnostic_window(error, max_chars);
    let redacted = redact_provider_error_diagnostic(window);
    let needs_truncation = source_truncated || redacted.len() > max_chars;
    cap_provider_error_diagnostic(&redacted, max_chars, needs_truncation)
}

fn provider_error_diagnostic_window(error: &str, max_chars: usize) -> (&str, bool) {
    let scan_cap =
        max_chars.saturating_add(SAFE_PROVIDER_ERROR_DIAGNOSTIC_REDACTION_LOOKAHEAD_CHARS);
    let window = safe_truncate(error, scan_cap);
    let truncated = window.len() < error.len();
    if truncated {
        (trim_incomplete_provider_error_token(window), true)
    } else {
        (window, false)
    }
}

fn trim_incomplete_provider_error_token(window: &str) -> &str {
    if window
        .chars()
        .last()
        .map(is_provider_error_token_boundary)
        .unwrap_or(true)
    {
        return window;
    }
    let end = window
        .char_indices()
        .rev()
        .find(|(_, ch)| is_provider_error_token_boundary(*ch))
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    &window[..end]
}

fn is_provider_error_token_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            ',' | ';' | ':' | ')' | ']' | '}' | '"' | '\'' | '`' | '<' | '>'
        )
}

fn redact_provider_error_diagnostic(text: &str) -> String {
    static SK_PLACEHOLDER: OnceLock<regex::Regex> = OnceLock::new();
    let redacted = redact_sensitive(text);
    SK_PLACEHOLDER
        .get_or_init(|| regex::Regex::new(r"sk-[A-Za-z0-9_-]{8,}").unwrap())
        .replace_all(&redacted, "[REDACTED_SK_TOKEN]")
        .into_owned()
}

fn cap_provider_error_diagnostic(text: &str, max_chars: usize, with_marker: bool) -> String {
    let marked;
    let source = if with_marker {
        marked = format!(
            "{}{}",
            text.trim_end(),
            SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED
        );
        marked.as_str()
    } else {
        text
    };
    if source.len() <= max_chars {
        return source.to_string();
    }
    if max_chars <= SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED.len() {
        return safe_truncate(SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED, max_chars).to_string();
    }
    let keep = max_chars - SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED.len();
    format!(
        "{}{}",
        safe_truncate(source, keep).trim_end(),
        SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED
    )
}

pub fn make_ui_only_error_message(error: &str) -> ChatMessage {
    let category = classify_user_error(error);
    let info = user_error_info(category);
    let redacted_error = safe_provider_error_diagnostic(error);
    let mut extra = json!({
        "error_info": {
            "category": format!("{:?}", info.category),
            "title": info.title,
            "explanation": info.explanation,
            "suggested_action": info.suggested_action,
            "is_retryable": info.is_retryable,
            "raw_error": redacted_error.clone(),
        }
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    mark_ui_only(&mut extra);

    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "error".to_string(),
        content: ChatContent::SimpleText(redacted_error),
        extra,
        ..Default::default()
    }
}

pub fn make_ui_only_retry_status_message(
    error: &str,
    attempt: usize,
    max_attempts: usize,
    delay_secs: u64,
) -> ChatMessage {
    let category = classify_user_error(error);
    let base_info = user_error_info(category);
    let redacted_error = safe_provider_error_diagnostic(error);
    let title = format!(
        "Retrying — {} (attempt {}/{})",
        base_info.title, attempt, max_attempts
    );
    let explanation = format!("{} Next retry in {}s.", base_info.explanation, delay_secs);
    let summary = format!(
        "{} — retrying in {}s (attempt {}/{}).",
        base_info.title, delay_secs, attempt, max_attempts,
    );
    let mut extra = json!({
        "error_info": {
            "category": format!("{:?}", base_info.category),
            "title": title,
            "explanation": explanation,
            "suggested_action": base_info.suggested_action,
            "is_retryable": true,
            "raw_error": redacted_error,
        },
        "retry_status": {
            "attempt": attempt,
            "max_attempts": max_attempts,
            "delay_secs": delay_secs,
            "in_progress": true,
        },
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    mark_ui_only(&mut extra);

    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "error".to_string(),
        content: ChatContent::SimpleText(summary),
        extra,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_info_raw_error(message: &ChatMessage) -> &str {
        message
            .extra
            .get("error_info")
            .and_then(|info| info.get("raw_error"))
            .and_then(|raw_error| raw_error.as_str())
            .unwrap_or_default()
    }

    #[test]
    fn ui_only_marker_is_detected_and_filtered() {
        let visible = ChatMessage::new("user".to_string(), "visible".to_string());
        let hidden = make_ui_only_error_message("context_length_exceeded");

        assert!(is_ui_only_message(&hidden));
        let filtered = filter_ui_only_messages(vec![visible.clone(), hidden]);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].content.content_text_only(), "visible");
    }

    #[test]
    fn error_message_contains_structured_error_info() {
        let message = make_ui_only_error_message("context_length_exceeded: input too large");

        assert!(is_ui_only_message(&message));
        assert_eq!(message.role, "error");
        assert_eq!(
            message
                .extra
                .get("error_info")
                .and_then(|info| info.get("category"))
                .and_then(|category| category.as_str()),
            Some("ContextTooLarge")
        );
    }

    #[test]
    fn error_message_redacts_secret_from_content_and_raw_error() {
        let message = make_ui_only_error_message(
            "provider failed with Authorization: Bearer sk-abcdefgh12345678",
        );

        let content = message.content.content_text_only();
        assert!(!content.contains("sk-abcdefgh12345678"));
        assert!(content.contains("[REDACTED"));

        let raw_error = message
            .extra
            .get("error_info")
            .and_then(|info| info.get("raw_error"))
            .and_then(|raw_error| raw_error.as_str())
            .unwrap_or_default();
        assert!(!raw_error.contains("sk-abcdefgh12345678"));
        assert!(raw_error.contains("[REDACTED"));
    }

    #[test]
    fn error_message_bounds_huge_provider_error_before_persisting() {
        let far_tail = "FAR_TAIL_MARKER";
        let error = format!(
            "context_length_exceeded: Authorization: Bearer sk-test-secret {} {}",
            "x".repeat(200_000),
            far_tail,
        );

        let message = make_ui_only_error_message(&error);
        let content = message.content.content_text_only();
        let raw_error = error_info_raw_error(&message);

        assert!(content.len() <= SAFE_PROVIDER_ERROR_DIAGNOSTIC_MAX_CHARS);
        assert!(raw_error.len() <= SAFE_PROVIDER_ERROR_DIAGNOSTIC_MAX_CHARS);
        assert!(content.contains(SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED));
        assert!(raw_error.contains(SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED));
        assert!(!content.contains("sk-test-secret"));
        assert!(!raw_error.contains("sk-test-secret"));
        assert!(!content.contains("Authorization: Bearer sk-test-secret"));
        assert!(!raw_error.contains("Authorization: Bearer sk-test-secret"));
        assert!(!content.contains(far_tail));
        assert!(!raw_error.contains(far_tail));
    }

    #[test]
    fn error_message_redacts_bare_sk_token_near_boundary() {
        let max = SAFE_PROVIDER_ERROR_DIAGNOSTIC_MAX_CHARS;
        let secret = "sk-boundarysecretvalue";
        let prefix = "x".repeat(max.saturating_sub(20));
        let error = format!(
            "context_length_exceeded: {} {} {}",
            prefix,
            secret,
            "y".repeat(2_000),
        );

        let message = make_ui_only_error_message(&error);
        let content = message.content.content_text_only();
        let raw_error = error_info_raw_error(&message);

        assert!(content.len() <= max);
        assert!(raw_error.len() <= max);
        assert!(content.contains(SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED));
        assert!(raw_error.contains(SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED));
        assert!(!content.contains(secret));
        assert!(!raw_error.contains(secret));
        assert!(!content.contains("sk-boundary"));
        assert!(!raw_error.contains("sk-boundary"));
    }

    #[test]
    fn retry_status_message_carries_attempt_and_delay() {
        let message = make_ui_only_retry_status_message(
            "LLM error (429 Too Many Requests): rate limit",
            2,
            5,
            15,
        );

        assert!(is_ui_only_message(&message));
        assert_eq!(message.role, "error");
        let content = message.content.content_text_only();
        assert!(content.contains("attempt 2/5"));
        assert!(content.contains("15s"));

        let info = message.extra.get("error_info").expect("error_info present");
        assert_eq!(
            info.get("category").and_then(|c| c.as_str()),
            Some("ProviderRateLimit"),
        );
        assert_eq!(
            info.get("is_retryable").and_then(|b| b.as_bool()),
            Some(true),
        );
        assert!(info
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .contains("attempt 2/5"));

        let retry_status = message
            .extra
            .get("retry_status")
            .expect("retry_status present");
        assert_eq!(
            retry_status.get("attempt").and_then(|v| v.as_u64()),
            Some(2),
        );
        assert_eq!(
            retry_status.get("max_attempts").and_then(|v| v.as_u64()),
            Some(5),
        );
        assert_eq!(
            retry_status.get("delay_secs").and_then(|v| v.as_u64()),
            Some(15),
        );
        assert_eq!(
            retry_status.get("in_progress").and_then(|v| v.as_bool()),
            Some(true),
        );
    }

    #[test]
    fn retry_status_message_redacts_secret_from_content_and_raw_error() {
        let message =
            make_ui_only_retry_status_message("LLM error: token=sk-retrysecret12345678", 3, 5, 30);

        let content = message.content.content_text_only();
        assert!(!content.contains("sk-retrysecret12345678"));

        let raw_error = message
            .extra
            .get("error_info")
            .and_then(|info| info.get("raw_error"))
            .and_then(|raw_error| raw_error.as_str())
            .unwrap_or_default();
        assert!(!raw_error.contains("sk-retrysecret12345678"));
        assert!(raw_error.contains("[REDACTED"));
    }

    #[test]
    fn retry_status_raw_error_is_bounded_and_redacted() {
        let error = format!(
            "LLM error (503 Service Unavailable): Authorization: Bearer sk-test-secret {}",
            "retry-tail ".repeat(40_000),
        );

        let message = make_ui_only_retry_status_message(&error, 3, 5, 30);
        let content = message.content.content_text_only();
        let raw_error = error_info_raw_error(&message);

        assert!(raw_error.len() <= SAFE_PROVIDER_ERROR_DIAGNOSTIC_MAX_CHARS);
        assert!(raw_error.contains(SAFE_PROVIDER_ERROR_DIAGNOSTIC_TRUNCATED));
        assert!(!raw_error.contains("sk-test-secret"));
        assert!(!raw_error.contains("Authorization: Bearer sk-test-secret"));
        assert!(raw_error.contains("[REDACTED"));
        assert!(content.contains("retrying in 30s"));
        assert!(content.contains("attempt 3/5"));
        assert!(!content.contains("sk-test-secret"));
        assert!(!content.contains("Authorization: Bearer"));
    }
}
