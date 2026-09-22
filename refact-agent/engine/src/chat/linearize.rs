use crate::call_validation::ChatMessage;
use crate::chat::diagnostics::is_ui_only_message;
use refact_core::active_context::active_context;
use std::collections::HashSet;

const WIRE_SUPPRESSED_PURSUIT_KINDS: &[&str] = &[
    "pursuit_quiescent",
    "budget_exhausted",
    "no_progress",
    "stopped",
    "paused",
    "snoozed",
    "resumed",
];

fn is_wire_suppressed_goal_pursuit(msg: &ChatMessage) -> bool {
    if msg.role != "event" {
        return false;
    }
    let Some(event) = msg.extra.get("event") else {
        return false;
    };
    if event.get("subkind").and_then(|value| value.as_str()) != Some("goal_pursuit") {
        return false;
    }
    event
        .get("payload")
        .and_then(|payload| payload.get("kind"))
        .and_then(|kind| kind.as_str())
        .is_some_and(|kind| WIRE_SUPPRESSED_PURSUIT_KINDS.contains(&kind))
}

/// Reconstruct first, while boundary metadata is intact. Legacy summaries are never
/// interpreted as suppression instructions; callers must surface the rebuild error.
pub fn apply_summarization_linearize(
    messages: Vec<ChatMessage>,
) -> Result<Vec<ChatMessage>, String> {
    let active = active_context(&messages).map_err(|error| error.to_string())?;
    let mut result: Vec<_> = active
        .messages
        .into_iter()
        .filter(|message| {
            !is_ui_only_message(message)
                && !matches!(
                    message.role.as_str(),
                    "summarization" | "compression_report" | "error"
                )
                && !is_wire_suppressed_goal_pursuit(message)
        })
        .collect();
    detach_orphaned_context_files(&mut result);
    Ok(result)
}

fn detach_orphaned_context_files(messages: &mut [ChatMessage]) {
    let live_call_ids: HashSet<String> = messages
        .iter()
        .filter_map(|message| message.tool_calls.as_ref())
        .flatten()
        .map(|tool_call| tool_call.id.clone())
        .collect();
    for message in messages.iter_mut() {
        if message.role == "context_file"
            && !message.tool_call_id.is_empty()
            && !live_call_ids.contains(&message.tool_call_id)
        {
            message.tool_call_id = String::new();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_core::chat_types::ChatContent;
    use serde_json::json;

    fn message(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: role.into(),
            content: ChatContent::SimpleText(text.into()),
            ..Default::default()
        }
    }

    #[test]
    fn archived_sentinel_never_reaches_wire() {
        let mut boundary = message("compression_report", "REPORT_SENTINEL");
        boundary.extra.insert("compression_report".into(), json!({
            "kind": "reconstructed_history", "schema_version": 1,
            "payload": {"messages": [message("user", "active question"), message("assistant", "active answer")]}
        }));
        let raw = vec![
            message("assistant", "ARCHIVED_SENTINEL"),
            boundary,
            message("user", "tail"),
        ];
        let output = apply_summarization_linearize(raw.clone()).unwrap();
        let text = serde_json::to_string(&output).unwrap();
        assert!(!text.contains("ARCHIVED_SENTINEL"));
        assert!(!text.contains("REPORT_SENTINEL"));
        assert!(text.contains("active question"));
        assert!(text.contains("tail"));
        assert_eq!(raw[0].content.content_text_only(), "ARCHIVED_SENTINEL");
    }

    #[test]
    fn legacy_is_an_error_not_a_migration() {
        let mut legacy = message("assistant", "summary");
        legacy.summarized_range = Some((0, 0));
        assert!(
            apply_summarization_linearize(vec![message("user", "original"), legacy])
                .unwrap_err()
                .contains("explicit rebuild")
        );
    }

    #[test]
    fn corrupt_boundary_fails_closed() {
        let mut boundary = message("compression_report", "bad");
        boundary.extra.insert("compression_report".into(), json!({"kind":"reconstructed_history", "schema_version":1,"payload":{"messages":"bad"}}));
        assert!(apply_summarization_linearize(vec![message("user", "archive"), boundary]).is_err());
    }

    #[test]
    fn ordinary_filters_and_context_detachment_remain() {
        let mut context = message("context_file", "file");
        context.tool_call_id = "missing-call".into();
        let mut event = message("event", "archival status");
        event.extra.insert(
            "event".into(),
            json!({"subkind":"goal_pursuit","payload":{"kind":"stopped"}}),
        );
        let output = apply_summarization_linearize(vec![
            message("user", "question"),
            context,
            event,
            message("error", "UI error"),
        ])
        .unwrap();
        assert_eq!(output.len(), 2);
        assert!(output[1].tool_call_id.is_empty());
    }
}
