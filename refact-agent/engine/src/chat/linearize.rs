use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::diagnostics::is_ui_only_message;
use refact_chat_history::compression_exemption::{exemption_for, CompressionExemption};
use std::collections::{HashMap, HashSet};

fn is_authoritative_summary(msg: &ChatMessage) -> bool {
    if msg.role != "summarization" || is_ui_only_message(msg) {
        return false;
    }
    matches!(
        msg.summarization_tier.as_deref(),
        Some("tier0_deterministic") | Some("tier1_llm") | Some("tier1_merged")
    )
}

pub fn apply_summarization_linearize(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    if !messages.iter().any(|m| m.role == "summarization") {
        return messages;
    }

    let summaries: Vec<(usize, usize, String)> = messages
        .iter()
        .filter(|m| is_authoritative_summary(m))
        .filter_map(|m| {
            let (start, end) = m.summarized_range?;
            Some((start, end, m.content.content_text_only()))
        })
        .collect();

    let mut suppressed: HashSet<usize> = HashSet::new();
    for (start, end, _) in &summaries {
        for i in *start..=*end {
            if messages
                .get(i)
                .map(|msg| exemption_for(msg) == CompressionExemption::Never)
                .unwrap_or(false)
            {
                continue;
            }
            suppressed.insert(i);
        }
    }

    let mut summary_by_start: HashMap<usize, Vec<(usize, String)>> = HashMap::new();
    for (start, end, content) in summaries {
        let Some(insert_at) = (start..=end).find(|idx| {
            messages
                .get(*idx)
                .map(|msg| exemption_for(msg) != CompressionExemption::Never)
                .unwrap_or(false)
        }) else {
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
        if msg.role == "summarization" {
            continue;
        }
        if suppressed.contains(&i) {
            if let Some(entries) = summary_by_start.remove(&i) {
                for (_, content) in entries {
                    result.push(ChatMessage {
                        role: "user".to_string(),
                        content: ChatContent::SimpleText(content),
                        ..Default::default()
                    });
                }
            }
            continue;
        }
        result.push(msg.clone());
    }

    result
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

    fn summarization(content: &str, range: (usize, usize)) -> ChatMessage {
        ChatMessage {
            role: "summarization".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            summarized_range: Some(range),
            summarization_tier: Some("tier0_deterministic".to_string()),
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
    fn test_linearize_summarization_replaces_range() {
        let messages = vec![
            user("hello"),          // 0
            assistant("response1"), // 1
            user("follow up"),      // 2
            assistant("response2"), // 3
            user("new question"),   // 4
            summarization("Summary of messages 1-3", (1, 3)),
            assistant("final"), // 6
        ];
        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "user", "user", "assistant"]);
        assert_eq!(result[0].content.content_text_only(), "hello");
        assert_eq!(
            result[1].content.content_text_only(),
            "Summary of messages 1-3"
        );
        assert_eq!(result[2].content.content_text_only(), "new question");
        assert_eq!(result[3].content.content_text_only(), "final");
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
            summarization_tier: Some("tier2_reactive".to_string()),
            extra,
            ..Default::default()
        }
    }

    #[test]
    fn test_linearize_ignores_ui_only_tier2_reactive_summaries() {
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
            summarization("stale summary", (10, 11)),
        ];
        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "user"]);
        assert_eq!(result[2].content.content_text_only(), "current question");
    }

    #[test]
    fn test_linearize_messages_after_summarized_range_preserved() {
        let messages = vec![
            user("msg0"),      // 0
            assistant("msg1"), // 1
            user("msg2"),      // 2 - in range
            summarization("sum", (2, 2)),
            user("msg3"), // 4
        ];
        let result = apply_summarization_linearize(messages);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].content.content_text_only(), "msg0");
        assert_eq!(result[1].content.content_text_only(), "msg1");
        assert_eq!(result[2].content.content_text_only(), "sum");
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
    fn linearize_keeps_plan_when_summary_range_overlaps_it() {
        let messages = vec![
            user("old"),
            plan("sacred plan"),
            user("new"),
            summarization("sum", (0, 2)),
        ];
        let result = apply_summarization_linearize(messages);
        let roles: Vec<&str> = result.iter().map(|message| message.role.as_str()).collect();

        assert_eq!(roles, vec!["user", "plan"]);
        assert_eq!(result[0].content.content_text_only(), "sum");
        assert_eq!(result[1].content.content_text_only(), "sacred plan");
    }
}
