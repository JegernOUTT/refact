use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::diagnostics::is_ui_only_message;
use refact_chat_history::compression_exemption::{exemption_for, CompressionExemption};
use refact_chat_history::trajectory_ops::COMPRESSION_REPORT_ROLE;
use std::collections::{HashMap, HashSet};

fn is_authoritative_summary(msg: &ChatMessage) -> bool {
    if is_ui_only_message(msg) {
        return false;
    }
    msg.role == "assistant"
        && msg
            .extra
            .get("compression")
            .and_then(|compression| compression.get("kind"))
            .and_then(|kind| kind.as_str())
            == Some("llm_segment_summary")
}

fn summary_content(msg: &ChatMessage) -> String {
    msg.extra
        .get("summary_text")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| msg.content.content_text_only())
}

fn is_legacy_range_summary(msg: &ChatMessage) -> bool {
    is_authoritative_summary(msg) && msg.summarized_range.is_some()
}

fn is_source_preserving_summary(msg: &ChatMessage) -> bool {
    is_authoritative_summary(msg)
        && msg
            .extra
            .get("compression")
            .and_then(|compression| compression.get("insert_mode"))
            .and_then(|mode| mode.as_str())
            == Some("source_preserving")
}

fn source_preserving_summary_id_sets(
    messages: &[ChatMessage],
) -> (HashSet<String>, HashSet<String>) {
    let mut suppressed = HashSet::new();
    let mut preserved = HashSet::new();
    for message in messages
        .iter()
        .filter(|message| is_source_preserving_summary(message))
    {
        let Some(compression) = message.extra.get("compression") else {
            continue;
        };
        // Suppression is membership-based on message ids. Engine-side compaction
        // (tool-output truncation, context-file dedup, history repair) legitimately
        // mutates source messages in place after summarization, so the stored
        // `source_hash` is diagnostic metadata and must never gate suppression:
        // a hash gate silently disables compression exactly when the context is
        // under pressure, which explodes the request and forces duplicate
        // re-summarization of the same segment.
        collect_compression_ids(
            compression,
            "summarized_source_message_ids",
            &mut suppressed,
        );
        collect_compression_ids(compression, "preserved_source_message_ids", &mut preserved);
    }
    (suppressed, preserved)
}

fn collect_compression_ids(
    compression: &serde_json::Value,
    field: &str,
    target: &mut HashSet<String>,
) {
    if let Some(ids) = compression.get(field).and_then(|ids| ids.as_array()) {
        target.extend(
            ids.iter()
                .filter_map(|id| id.as_str())
                .filter(|id| !id.is_empty())
                .map(ToString::to_string),
        );
    }
}

fn can_suppress_source_preserving_source(
    msg: &ChatMessage,
    suppressed: &HashSet<String>,
    preserved: &HashSet<String>,
) -> bool {
    !msg.message_id.is_empty()
        && suppressed.contains(&msg.message_id)
        && !preserved.contains(&msg.message_id)
        && !matches!(msg.role.as_str(), "user" | "system" | "plan" | "goal")
        && !is_authoritative_summary(msg)
        && exemption_for(msg) != CompressionExemption::Never
}

fn is_visual_compression_report(msg: &ChatMessage) -> bool {
    msg.role == COMPRESSION_REPORT_ROLE
}

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

fn is_linearization_only_message(msg: &ChatMessage) -> bool {
    msg.role == "summarization"
        || is_legacy_range_summary(msg)
        || is_visual_compression_report(msg)
        || is_wire_suppressed_goal_pursuit(msg)
}

fn legacy_summary_ranges(messages: &[ChatMessage]) -> Vec<(usize, usize, String)> {
    messages
        .iter()
        .filter(|msg| is_authoritative_summary(msg))
        .filter_map(|msg| {
            let (start, end) = msg.summarized_range?;
            if start > end || end >= messages.len() {
                return None;
            }
            Some((start, end, summary_content(msg)))
        })
        .collect()
}

pub fn apply_summarization_linearize(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let summaries = legacy_summary_ranges(&messages);
    let (source_preserving_suppressed_ids, source_preserving_preserved_ids) =
        source_preserving_summary_id_sets(&messages);
    if summaries.is_empty() {
        let mut result: Vec<ChatMessage> = messages
            .into_iter()
            .filter(|message| !is_linearization_only_message(message))
            .filter(|message| {
                !can_suppress_source_preserving_source(
                    message,
                    &source_preserving_suppressed_ids,
                    &source_preserving_preserved_ids,
                )
            })
            .collect();
        detach_orphaned_context_files(&mut result);
        return result;
    }

    let mut suppressed: HashSet<usize> = HashSet::new();
    for (start, end, _) in &summaries {
        for i in *start..=*end {
            if messages
                .get(i)
                .map(|msg| {
                    msg.role == "user"
                        || msg.role == "event"
                        || exemption_for(msg) == CompressionExemption::Never
                })
                .unwrap_or(false)
            {
                continue;
            }
            suppressed.insert(i);
        }
    }

    let mut summary_by_start: HashMap<usize, Vec<(usize, String)>> = HashMap::new();
    for (start, end, content) in summaries {
        let insert_at = if let Some(insert_at) = (start..=end).find(|idx| suppressed.contains(idx))
        {
            insert_at
        } else if start < end {
            (start + 1).min(messages.len())
        } else {
            continue;
        };
        summary_by_start
            .entry(insert_at)
            .or_default()
            .push((end, content));
    }
    for entries in summary_by_start.values_mut() {
        entries.sort_by_key(|(end, _)| *end);
    }

    let mut result = Vec::with_capacity(messages.len());

    for (i, msg) in messages.iter().enumerate() {
        if let Some(entries) = summary_by_start.remove(&i) {
            for (_, content) in entries {
                result.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: ChatContent::SimpleText(content),
                    ..Default::default()
                });
            }
        }
        if is_linearization_only_message(msg)
            || suppressed.contains(&i)
            || can_suppress_source_preserving_source(
                msg,
                &source_preserving_suppressed_ids,
                &source_preserving_preserved_ids,
            )
        {
            continue;
        }
        result.push(msg.clone());
    }

    detach_orphaned_context_files(&mut result);
    result
}

/// Preserved context files survive linearization while the assistant tool call
/// that produced them is suppressed by a summary. Clear the dangling
/// `tool_call_id` so downstream history repair does not drop them as orphaned
/// tool results.
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

    fn user(text: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn assistant(text: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn summarization(content: &str, range: Option<(usize, usize)>) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "compression".to_string(),
            serde_json::json!({
                "schema_version": 1,
                "kind": "llm_segment_summary",
                "source_hash": "hash",
                "source_message_ids": [],
                "created_at": "now",
                "summary_model": "test",
            }),
        );
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            summarized_range: range,
            summarization_tier: Some("llm_segment_summary".to_string()),
            extra,
            ..Default::default()
        }
    }

    fn compression_report(content: &str) -> ChatMessage {
        ChatMessage {
            role: COMPRESSION_REPORT_ROLE.to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            summarization_tier: Some("tier1_llm".to_string()),
            extra: serde_json::Map::from_iter([(
                "compression_report".to_string(),
                serde_json::json!({
                    "kind": "chat_compression_report",
                    "compression_kind": "llm_segment_summary",
                    "source_message_count": 1,
                }),
            )]),
            ..Default::default()
        }
    }

    fn source_preserving_summary(
        content: &str,
        summarized_ids: &[&str],
        preserved_ids: &[&str],
    ) -> ChatMessage {
        // No `source_hash` on purpose: hash-less summaries follow the legacy
        // trust-the-ids suppression path. Hash validation cases set it explicitly.
        let mut summary = summarization(content, None);
        summary.extra.insert(
            "compression".to_string(),
            serde_json::json!({
                "schema_version": 3,
                "kind": "llm_segment_summary",
                "insert_mode": "source_preserving",
                "source_message_ids": summarized_ids,
                "summarized_source_message_ids": summarized_ids,
                "preserved_source_message_ids": preserved_ids,
                "created_at": "now",
                "summary_model": "test",
            }),
        );
        summary
    }

    fn with_source_hash(mut summary: ChatMessage, source_hash: &str) -> ChatMessage {
        if let Some(compression) = summary.extra.get_mut("compression") {
            if let Some(obj) = compression.as_object_mut() {
                obj.insert(
                    "source_hash".to_string(),
                    serde_json::Value::String(source_hash.to_string()),
                );
            }
        }
        summary
    }

    #[test]
    fn keeps_summary_and_suppresses_sources_when_hash_matches() {
        let source = with_id(assistant("heavy old answer"), "src-1");
        let valid_hash =
            crate::chat::summarization::source_hash_for_messages(std::slice::from_ref(&source));
        let summary = with_id(
            with_source_hash(
                source_preserving_summary("compact summary", &["src-1"], &[]),
                &valid_hash,
            ),
            "summary-1",
        );
        let messages = vec![
            with_id(user("question"), "u-1"),
            source,
            summary,
            with_id(user("next"), "u-2"),
        ];

        let result = apply_summarization_linearize(messages);

        let ids: Vec<&str> = result
            .iter()
            .map(|message| message.message_id.as_str())
            .collect();
        assert!(!ids.contains(&"src-1"));
        assert!(ids.contains(&"summary-1"));
    }

    #[test]
    fn suppresses_sources_even_when_content_mutated_after_summarization() {
        let source = with_id(assistant("edited answer"), "src-1");
        let summary = with_id(
            with_source_hash(
                source_preserving_summary("compact summary", &["src-1"], &[]),
                "definitely-not-the-current-hash",
            ),
            "summary-1",
        );
        let messages = vec![
            with_id(user("question"), "u-1"),
            source,
            summary,
            with_id(user("next"), "u-2"),
        ];

        let result = apply_summarization_linearize(messages);

        // Deterministic compaction and history repair mutate sources in place;
        // a stale hash must not disable suppression or drop the summary.
        let ids: Vec<&str> = result
            .iter()
            .map(|message| message.message_id.as_str())
            .collect();
        assert!(!ids.contains(&"src-1"));
        assert!(ids.contains(&"summary-1"));
    }

    #[test]
    fn keeps_summary_visible_when_sources_are_missing() {
        let summary = with_id(
            with_source_hash(
                source_preserving_summary("carried summary", &["gone-src"], &[]),
                "some-hash",
            ),
            "summary-1",
        );
        let messages = vec![
            with_id(user("question"), "u-1"),
            summary,
            with_id(user("next"), "u-2"),
        ];

        let result = apply_summarization_linearize(messages);

        // A summary carried without its sources (handoff/branch) stays on the wire
        // as context; it simply suppresses nothing.
        let ids: Vec<&str> = result
            .iter()
            .map(|message| message.message_id.as_str())
            .collect();
        assert!(ids.contains(&"summary-1"));
        assert!(ids.contains(&"u-1"));
        assert!(ids.contains(&"u-2"));
    }

    fn with_id(mut message: ChatMessage, id: &str) -> ChatMessage {
        message.message_id = id.to_string();
        message
    }

    #[test]
    fn preserved_context_file_survives_and_detaches_from_suppressed_call() {
        let mut call = assistant("calling cat");
        call.tool_calls = Some(vec![crate::call_validation::ChatToolCall {
            id: "call-1".to_string(),
            function: crate::call_validation::ChatToolFunction {
                name: "cat".to_string(),
                arguments: "{}".to_string(),
            },
            tool_type: "function".to_string(),
            index: None,
            extra_content: None,
        }]);
        let call = with_id(call, "src-call");
        let mut tool_result = ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText("paths found".to_string()),
            tool_call_id: "call-1".to_string(),
            ..Default::default()
        };
        tool_result.message_id = "src-tool".to_string();
        let mut preserved_file = context_file("important file body");
        preserved_file.tool_call_id = "call-1".to_string();
        let preserved_file = with_id(preserved_file, "src-file");
        let summary = with_id(
            source_preserving_summary(
                "summary of the work",
                &["src-call", "src-tool", "src-file"],
                &["src-file"],
            ),
            "summary-1",
        );
        let messages = vec![
            with_id(user("question"), "u-1"),
            call,
            tool_result,
            preserved_file,
            summary,
            with_id(user("next"), "u-2"),
        ];

        let result = apply_summarization_linearize(messages);

        let ids: Vec<&str> = result
            .iter()
            .map(|message| message.message_id.as_str())
            .collect();
        assert!(!ids.contains(&"src-call"));
        assert!(!ids.contains(&"src-tool"));
        assert!(ids.contains(&"src-file"));
        assert!(ids.contains(&"summary-1"));
        let kept_file = result
            .iter()
            .find(|message| message.message_id == "src-file")
            .unwrap();
        assert!(kept_file.tool_call_id.is_empty());
    }

    fn context_file(text: &str) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn event(text: &str) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "event".to_string(),
            serde_json::json!({
                "subkind": "tool_decision",
                "source": "test",
                "payload": {},
            }),
        );
        ChatMessage {
            role: "event".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            extra,
            ..Default::default()
        }
    }

    fn plan(text: &str) -> ChatMessage {
        ChatMessage {
            role: "plan".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn goal(text: &str) -> ChatMessage {
        ChatMessage {
            role: "goal".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn system(text: &str) -> ChatMessage {
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn plan_delta_event(text: &str) -> ChatMessage {
        let mut message = event(text);
        message.extra.insert(
            "event".to_string(),
            serde_json::json!({
                "subkind": "plan_delta",
                "source": "test",
                "payload": {"seq": 1},
            }),
        );
        message
    }

    fn goal_delta_event(text: &str) -> ChatMessage {
        let mut message = event(text);
        message.extra.insert(
            "event".to_string(),
            serde_json::json!({
                "subkind": "goal_delta",
                "source": "test",
                "payload": {"seq": 1},
            }),
        );
        message
    }

    #[test]
    fn test_linearize_no_summarization_unchanged() {
        let messages = vec![user("hello"), assistant("hi"), user("world")];
        let result = apply_summarization_linearize(messages.clone());
        assert_eq!(result.len(), messages.len());
        assert_eq!(result[0].content.content_text_only(), "hello");
        assert_eq!(result[1].content.content_text_only(), "hi");
        assert_eq!(result[2].content.content_text_only(), "world");
    }

    #[test]
    fn test_linearize_summarization_replaces_non_user_range_members() {
        let messages = vec![
            user("hello"),
            assistant("response1"),
            user("follow up"),
            assistant("response2"),
            user("new question"),
            summarization("Summary of messages 1-3", Some((1, 3))),
            assistant("final"),
        ];
        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(
            roles,
            vec!["user", "assistant", "user", "user", "assistant"]
        );
        assert_eq!(result[0].content.content_text_only(), "hello");
        assert_eq!(
            result[1].content.content_text_only(),
            "Summary of messages 1-3"
        );
        assert_eq!(result[2].content.content_text_only(), "follow up");
        assert_eq!(result[3].content.content_text_only(), "new question");
        assert_eq!(result[4].content.content_text_only(), "final");
    }

    #[test]
    fn test_linearize_drops_summarization_without_known_tier() {
        let untyped = ChatMessage {
            role: "summarization".to_string(),
            content: ChatContent::SimpleText("untyped summary".to_string()),
            summarized_range: Some((0, 0)),
            summarization_tier: None,
            ..Default::default()
        };
        let messages = vec![user("hello"), untyped, assistant("hi")];
        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant"]);
    }

    fn ui_only_reactive_summary(content: &str, range: (usize, usize)) -> ChatMessage {
        let mut extra = serde_json::Map::new();
        extra.insert("_ui_only".to_string(), serde_json::Value::Bool(true));
        ChatMessage {
            role: "summarization".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            summarized_range: Some(range),
            summarization_tier: Some("legacy_reactive".to_string()),
            extra,
            ..Default::default()
        }
    }

    #[test]
    fn test_linearize_ignores_ui_only_legacy_reactive_summaries() {
        let messages = vec![
            user("hello"),
            assistant("hi"),
            user("real follow-up"),
            ui_only_reactive_summary("compaction diagnostic", (0, 2)),
            assistant("final"),
        ];
        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "user", "assistant"]);
        assert_eq!(result[0].content.content_text_only(), "hello");
        assert_eq!(result[1].content.content_text_only(), "hi");
        assert_eq!(result[2].content.content_text_only(), "real follow-up");
        assert_eq!(result[3].content.content_text_only(), "final");
    }

    #[test]
    fn test_linearize_drops_tail_summary_without_matching_range() {
        let messages = vec![
            user("old user"),
            assistant("old assistant"),
            user("current question"),
            summarization("stale summary", Some((10, 11))),
        ];
        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert_eq!(result[2].content.content_text_only(), "current question");
    }

    #[test]
    fn test_linearize_messages_after_summarized_range_preserved() {
        let messages = vec![
            user("msg0"),
            assistant("msg1"),
            user("msg2"),
            summarization("sum", Some((2, 2))),
            user("msg3"),
        ];
        let result = apply_summarization_linearize(messages);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].content.content_text_only(), "msg0");
        assert_eq!(result[1].content.content_text_only(), "msg1");
        assert_eq!(result[2].content.content_text_only(), "msg2");
        assert_eq!(result[3].content.content_text_only(), "msg3");
    }

    #[test]
    fn linearize_does_not_merge_event_with_user() {
        let messages = vec![user("before"), event("hidden event"), user("after")];
        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();

        assert_eq!(roles, vec!["user", "event", "user"]);
        assert_eq!(result[0].content.content_text_only(), "before");
        assert_eq!(result[1].content.content_text_only(), "hidden event");
        assert_eq!(result[2].content.content_text_only(), "after");
    }

    #[test]
    fn linearize_drops_archival_goal_pursuit_events_but_keeps_nudges() {
        fn pursuit_event(kind: &str) -> ChatMessage {
            crate::chat::internal_roles::event(
                crate::chat::internal_roles::EventSubkind::GoalPursuit,
                "chat.goal_monitor",
                serde_json::json!({"kind": kind, "at_ms": 1}),
                format!("pursuit {kind}"),
            )
        }

        let messages = vec![
            user("before"),
            pursuit_event("nudge"),
            pursuit_event("pursuit_quiescent"),
            pursuit_event("stopped"),
            pursuit_event("snoozed"),
            pursuit_event("verification_gaps"),
            user("after"),
        ];

        let result = apply_summarization_linearize(messages);
        let contents: Vec<String> = result
            .iter()
            .map(|message| message.content.content_text_only())
            .collect();

        assert!(contents.iter().any(|text| text == "pursuit nudge"));
        assert!(contents
            .iter()
            .any(|text| text == "pursuit verification_gaps"));
        assert!(!contents
            .iter()
            .any(|text| text == "pursuit pursuit_quiescent"));
        assert!(!contents.iter().any(|text| text == "pursuit stopped"));
        assert!(!contents.iter().any(|text| text == "pursuit snoozed"));
    }

    #[test]
    fn linearize_keeps_plan_when_summary_range_overlaps_it() {
        let messages = vec![
            user("old"),
            plan("sacred plan"),
            user("new"),
            summarization("sum", Some((0, 2))),
        ];
        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();

        assert_eq!(roles, vec!["user", "assistant", "plan", "user"]);
        assert_eq!(result[0].content.content_text_only(), "old");
        assert_eq!(result[1].content.content_text_only(), "sum");
        assert_eq!(result[2].content.content_text_only(), "sacred plan");
        assert_eq!(result[3].content.content_text_only(), "new");
    }

    #[test]
    fn linearize_keeps_goal_when_summary_range_overlaps_it() {
        let messages = vec![
            user("old"),
            goal("sacred goal"),
            user("new"),
            summarization("sum", Some((0, 2))),
        ];
        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();

        assert_eq!(roles, vec!["user", "assistant", "goal", "user"]);
        assert_eq!(result[0].content.content_text_only(), "old");
        assert_eq!(result[1].content.content_text_only(), "sum");
        assert_eq!(result[2].content.content_text_only(), "sacred goal");
        assert_eq!(result[3].content.content_text_only(), "new");
    }

    #[test]
    fn test_linearize_overlapping_summaries_keeps_both_anchor_summaries() {
        let messages = vec![
            user("msg0"),
            assistant("msg1"),
            user("msg2"),
            assistant("msg3"),
            summarization("summary-a", Some((0, 2))),
            summarization("summary-b", Some((1, 3))),
            user("tail"),
        ];

        let result = apply_summarization_linearize(messages);
        let text: Vec<String> = result
            .iter()
            .map(|msg| msg.content.content_text_only())
            .collect();

        assert_eq!(text, vec!["msg0", "summary-a", "summary-b", "msg2", "tail"]);
    }

    #[test]
    fn linearize_preserves_in_place_segment_summary_between_users() {
        let messages = vec![
            user("A exact bytes"),
            summarization("summary", None),
            user("B exact bytes"),
        ];

        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();

        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert_eq!(result[0].content.content_text_only(), "A exact bytes");
        assert_eq!(result[1].content.content_text_only(), "summary");
        assert_eq!(result[2].content.content_text_only(), "B exact bytes");
    }

    #[test]
    fn linearize_drops_compression_report_but_keeps_internal_segment_summary() {
        let messages = vec![
            user("before compression"),
            compression_report("visible report must not reach the model"),
            summarization("internal summary stays model-visible", None),
            user("after compression"),
        ];

        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();
        let text: Vec<String> = result
            .iter()
            .map(|message| message.content.content_text_only())
            .collect();

        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert_eq!(
            text,
            vec![
                "before compression",
                "internal summary stays model-visible",
                "after compression"
            ]
        );
    }

    #[test]
    fn linearize_drops_compression_report_when_legacy_range_summaries_exist() {
        let messages = vec![
            user("first"),
            assistant("old assistant"),
            compression_report("visible range report must not reach the model"),
            user("second"),
            summarization("legacy summary", Some((1, 1))),
        ];

        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();
        let text: Vec<String> = result
            .iter()
            .map(|message| message.content.content_text_only())
            .collect();

        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert_eq!(text, vec!["first", "legacy summary", "second"]);
    }

    #[test]
    fn linearize_source_preserving_summary_suppresses_original_source_messages() {
        let messages = vec![
            user("user anchor before source"),
            with_id(
                assistant("UNIQUE_ASSISTANT_SOURCE_OUTPUT"),
                "assistant-source",
            ),
            with_id(
                context_file("UNIQUE_CONTEXT_SOURCE_OUTPUT"),
                "context-source",
            ),
            compression_report("UNIQUE_VISIBLE_REPORT_OUTPUT"),
            source_preserving_summary(
                "UNIQUE_INTERNAL_SOURCE_PRESERVING_SUMMARY",
                &["assistant-source", "context-source"],
                &[],
            ),
            user("user anchor after source"),
        ];

        let result = apply_summarization_linearize(messages);
        let text = result
            .iter()
            .map(|message| message.content.content_text_only())
            .collect::<Vec<_>>()
            .join("\n");
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();

        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert!(!text.contains("UNIQUE_ASSISTANT_SOURCE_OUTPUT"));
        assert!(!text.contains("UNIQUE_CONTEXT_SOURCE_OUTPUT"));
        assert!(!text.contains("UNIQUE_VISIBLE_REPORT_OUTPUT"));
        assert!(text.contains("UNIQUE_INTERNAL_SOURCE_PRESERVING_SUMMARY"));
        assert!(text.contains("user anchor before source"));
        assert!(text.contains("user anchor after source"));
    }

    #[test]
    fn linearize_source_preserving_summary_keeps_original_user_messages() {
        let messages = vec![
            with_id(user("UNIQUE_USER_SOURCE_ANCHOR"), "user-source"),
            with_id(
                assistant("UNIQUE_ASSISTANT_SOURCE_TO_SUPPRESS"),
                "assistant-source",
            ),
            source_preserving_summary(
                "UNIQUE_SUMMARY_WITH_USER_SOURCE",
                &["user-source", "assistant-source"],
                &[],
            ),
            user("tail user"),
        ];

        let result = apply_summarization_linearize(messages);
        let text = result
            .iter()
            .map(|message| message.content.content_text_only())
            .collect::<Vec<_>>()
            .join("\n");
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();

        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert!(text.contains("UNIQUE_USER_SOURCE_ANCHOR"));
        assert!(text.contains("UNIQUE_SUMMARY_WITH_USER_SOURCE"));
        assert!(!text.contains("UNIQUE_ASSISTANT_SOURCE_TO_SUPPRESS"));
    }

    #[test]
    fn linearize_source_preserving_summary_keeps_preserved_context_file_ids() {
        let messages = vec![
            user("before preserved context"),
            with_id(
                context_file("UNIQUE_PRESERVED_CONTEXT_FILE"),
                "preserved-context",
            ),
            with_id(
                assistant("UNIQUE_SUPPRESSED_ASSISTANT_OUTPUT"),
                "assistant-source",
            ),
            source_preserving_summary(
                "UNIQUE_SUMMARY_WITH_PRESERVED_CONTEXT",
                &["preserved-context", "assistant-source"],
                &["preserved-context"],
            ),
            user("after preserved context"),
        ];

        let result = apply_summarization_linearize(messages);
        let text = result
            .iter()
            .map(|message| message.content.content_text_only())
            .collect::<Vec<_>>()
            .join("\n");
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();

        assert_eq!(roles, vec!["user", "context_file", "assistant", "user"]);
        assert!(text.contains("UNIQUE_PRESERVED_CONTEXT_FILE"));
        assert!(text.contains("UNIQUE_SUMMARY_WITH_PRESERVED_CONTEXT"));
        assert!(!text.contains("UNIQUE_SUPPRESSED_ASSISTANT_OUTPUT"));
    }

    #[test]
    fn linearize_drops_report_but_keeps_source_preserving_summary() {
        let messages = vec![
            user("before source-preserving report"),
            compression_report("UNIQUE_REPORT_SHOULD_NOT_LINEARIZE"),
            source_preserving_summary(
                "UNIQUE_SOURCE_PRESERVING_SUMMARY_STAYS",
                &["missing-source-id"],
                &[],
            ),
            user("after source-preserving report"),
        ];

        let result = apply_summarization_linearize(messages);
        let text = result
            .iter()
            .map(|message| message.content.content_text_only())
            .collect::<Vec<_>>()
            .join("\n");
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();

        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert!(!text.contains("UNIQUE_REPORT_SHOULD_NOT_LINEARIZE"));
        assert!(text.contains("UNIQUE_SOURCE_PRESERVING_SUMMARY_STAYS"));
    }

    #[test]
    fn linearize_source_preserving_summary_keeps_system_plan_and_never_anchors() {
        let messages = vec![
            with_id(system("UNIQUE_SYSTEM_SOURCE_ANCHOR"), "system-source"),
            with_id(plan("UNIQUE_PLAN_SOURCE_ANCHOR"), "plan-source"),
            with_id(goal("UNIQUE_GOAL_SOURCE_ANCHOR"), "goal-source"),
            with_id(
                plan_delta_event("UNIQUE_NEVER_EVENT_SOURCE_ANCHOR"),
                "never-event-source",
            ),
            with_id(
                goal_delta_event("UNIQUE_GOAL_DELTA_SOURCE_ANCHOR"),
                "goal-delta-source",
            ),
            with_id(
                assistant("UNIQUE_ASSISTANT_SOURCE_TO_SUPPRESS"),
                "assistant-source",
            ),
            source_preserving_summary(
                "UNIQUE_SUMMARY_WITH_ANCHORS",
                &[
                    "system-source",
                    "plan-source",
                    "goal-source",
                    "never-event-source",
                    "goal-delta-source",
                    "assistant-source",
                ],
                &[],
            ),
            user("tail user"),
        ];

        let result = apply_summarization_linearize(messages);
        let text = result
            .iter()
            .map(|message| message.content.content_text_only())
            .collect::<Vec<_>>()
            .join("\n");
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();

        assert_eq!(
            roles,
            vec![
                "system",
                "plan",
                "goal",
                "event",
                "event",
                "assistant",
                "user"
            ]
        );
        assert!(text.contains("UNIQUE_SYSTEM_SOURCE_ANCHOR"));
        assert!(text.contains("UNIQUE_PLAN_SOURCE_ANCHOR"));
        assert!(text.contains("UNIQUE_GOAL_SOURCE_ANCHOR"));
        assert!(text.contains("UNIQUE_NEVER_EVENT_SOURCE_ANCHOR"));
        assert!(text.contains("UNIQUE_GOAL_DELTA_SOURCE_ANCHOR"));
        assert!(text.contains("UNIQUE_SUMMARY_WITH_ANCHORS"));
        assert!(!text.contains("UNIQUE_ASSISTANT_SOURCE_TO_SUPPRESS"));
    }

    #[test]
    fn linearize_preserves_anchor_event_within_summarized_range() {
        let messages = vec![
            user("old"),
            assistant("old response"),
            event("preserve-anchor event"),
            user("new"),
            summarization("sum of old", Some((1, 2))),
        ];
        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();

        // event has PreserveAnchor exemption and must survive the summarized range
        assert_eq!(roles, vec!["user", "assistant", "event", "user"]);
        assert_eq!(result[1].content.content_text_only(), "sum of old");
        assert_eq!(
            result[2].content.content_text_only(),
            "preserve-anchor event"
        );
    }
}
