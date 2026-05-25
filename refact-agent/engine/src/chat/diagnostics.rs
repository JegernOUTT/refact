use serde_json::{json, Value};
use uuid::Uuid;

use refact_chat_history::retry_policy::{classify_user_error, user_error_info};

use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::history_limit::Tier0CompactReport;

const UI_ONLY_MARKER: &str = "_ui_only";

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

pub fn make_ui_only_error_message(error: &str) -> ChatMessage {
    let category = classify_user_error(error);
    let info = user_error_info(category);
    let mut extra = json!({
        "error_info": {
            "category": format!("{:?}", info.category),
            "title": info.title,
            "explanation": info.explanation,
            "suggested_action": info.suggested_action,
            "is_retryable": info.is_retryable,
            "raw_error": error,
        }
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    mark_ui_only(&mut extra);

    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "error".to_string(),
        content: ChatContent::SimpleText(error.to_string()),
        extra,
        ..Default::default()
    }
}

pub fn format_tier0_compaction_report(report: &Tier0CompactReport, attempt: usize) -> String {
    format!(
        "{}\n\n{}\n\n{}\n{}\n{}\n{}",
        "## Reactive compaction report",
        "Context limit was reached, so Refact compacted the conversation before retrying.",
        format!("- Attempt: {}", attempt),
        format!(
            "- Context file entries deduplicated: {}",
            report.context_files_deduped,
        ),
        format!(
            "- Tool outputs truncated: {}",
            report.tool_outputs_truncated
        ),
        format!("- Estimated tokens saved: {}", report.tokens_saved_estimate),
    )
}

pub fn make_ui_only_compaction_report_message(
    report: &Tier0CompactReport,
    attempt: usize,
    affected_range: Option<(usize, usize)>,
) -> ChatMessage {
    let mut extra = serde_json::Map::new();
    mark_ui_only(&mut extra);
    extra.insert("compaction_report".to_string(), Value::Bool(true));

    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "summarization".to_string(),
        content: ChatContent::SimpleText(format_tier0_compaction_report(report, attempt)),
        summarization_tier: Some("tier2_reactive".to_string()),
        summarized_range: affected_range,
        summarized_token_estimate: Some(report.tokens_saved_estimate),
        extra,
        ..Default::default()
    }
}

pub fn append_ui_only_reactive_compaction_diagnostics(
    messages: &mut Vec<ChatMessage>,
    error: &str,
    report: &Tier0CompactReport,
    attempt: usize,
) {
    let range = if messages.is_empty() {
        None
    } else {
        Some((0usize, messages.len().saturating_sub(1)))
    };
    messages.push(make_ui_only_error_message(error));
    messages.push(make_ui_only_compaction_report_message(
        report, attempt, range,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn compaction_report_contains_tier0_details() {
        let report = Tier0CompactReport {
            context_files_deduped: 2,
            tool_outputs_truncated: 3,
            tokens_saved_estimate: 456,
        };
        let message = make_ui_only_compaction_report_message(&report, 2, Some((0, 9)));

        assert!(is_ui_only_message(&message));
        assert_eq!(message.role, "summarization");
        assert_eq!(
            message.summarization_tier.as_deref(),
            Some("tier2_reactive")
        );
        assert_eq!(message.summarized_token_estimate, Some(456));
        assert_eq!(
            message.extra.get("compaction_report"),
            Some(&Value::Bool(true))
        );
        let content = message.content.content_text_only();
        assert!(content.contains("Attempt: 2"));
        assert!(content.contains("Context file entries deduplicated: 2"));
        assert!(content.contains("Tool outputs truncated: 3"));
        assert!(content.contains("Estimated tokens saved: 456"));
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
}
