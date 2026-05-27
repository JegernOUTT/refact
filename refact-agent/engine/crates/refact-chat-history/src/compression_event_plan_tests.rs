use serde_json::json;

use crate::compression_exemption::event_subkind;
use crate::history_limit::{tier0_deterministic_compact_with, CompactAggression, CompressionStrength};
use refact_core::chat_types::{ChatContent, ChatMessage};

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

fn plan(text: &str) -> ChatMessage {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "plan".to_string(),
        json!({
            "mode": "agent",
            "version": 1,
            "created_at_ms": 123,
            "supersedes": null,
        }),
    );
    ChatMessage {
        message_id: "plan-id".to_string(),
        role: "plan".to_string(),
        content: ChatContent::SimpleText(text.to_string()),
        extra,
        ..Default::default()
    }
}

fn event(subkind: &str, source: &str, text: &str) -> ChatMessage {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "event".to_string(),
        json!({
            "subkind": subkind,
            "source": source,
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

fn aggression_for_strength(strength: &CompressionStrength) -> Option<CompactAggression> {
    match strength {
        CompressionStrength::Absent => None,
        CompressionStrength::Low | CompressionStrength::Medium => Some(CompactAggression::Standard),
        CompressionStrength::High => Some(CompactAggression::Aggressive),
    }
}

#[test]
fn plan_is_never_compressed() {
    let plan = plan("PLAN: keep this byte-for-byte");
    let plan_json = serde_json::to_string(&plan).unwrap();
    let strengths = [
        CompressionStrength::Absent,
        CompressionStrength::Low,
        CompressionStrength::Medium,
        CompressionStrength::High,
    ];

    for strength in strengths {
        let mut messages = Vec::with_capacity(2001);
        messages.push(plan.clone());
        for i in 0..1000 {
            messages.push(user(&format!("user turn {i}")));
            messages.push(assistant(&format!("assistant turn {i}")));
        }

        if let Some(aggression) = aggression_for_strength(&strength) {
            tier0_deterministic_compact_with(&mut messages, 4, aggression);
        }

        let plans: Vec<&ChatMessage> = messages
            .iter()
            .filter(|message| message.role == "plan")
            .collect();
        assert_eq!(plans.len(), 1, "strength {strength:?}");
        assert_eq!(serde_json::to_string(plans[0]).unwrap(), plan_json);
    }
}

#[test]
fn tick_dropped_outside_window() {
    let mut messages = Vec::new();
    for i in 0..50 {
        messages.push(event("tick", "clock", &format!("tick {i}")));
    }
    for i in 0..5 {
        messages.push(user(&format!("turn {i}")));
    }

    tier0_deterministic_compact_with(&mut messages, 3, CompactAggression::Standard);

    assert!(!messages
        .iter()
        .any(|message| message.role == "event" && event_subkind(message) == Some("tick")));
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.role == "user")
            .count(),
        5
    );
}

#[test]
fn process_completed_keeps_recent_n() {
    let mut messages = Vec::new();
    for i in 0..10 {
        messages.push(event(
            "process_completed",
            "exec:build",
            &format!("process completed {i}"),
        ));
    }

    tier0_deterministic_compact_with(&mut messages, 0, CompactAggression::Standard);

    let process_events: Vec<&ChatMessage> = messages
        .iter()
        .filter(|message| event_subkind(message) == Some("process_completed"))
        .collect();
    let summary_events: Vec<&ChatMessage> = messages
        .iter()
        .filter(|message| event_subkind(message) == Some("summarization_marker"))
        .collect();

    assert_eq!(process_events.len(), 3);
    assert_eq!(summary_events.len(), 1);
    assert!(summary_events[0]
        .content
        .content_text_only()
        .contains("7 earlier process_completed events"));
    assert!(summary_events[0]
        .content
        .content_text_only()
        .contains("source=\"exec:build\""));
    assert_eq!(
        process_events[0].content.content_text_only(),
        "process completed 7"
    );
    assert_eq!(
        process_events[1].content.content_text_only(),
        "process completed 8"
    );
    assert_eq!(
        process_events[2].content.content_text_only(),
        "process completed 9"
    );
}

#[test]
fn anchor_preserved_under_aggressive() {
    let notice = event("system_notice", "system", "do not drop this notice");
    let notice_json = serde_json::to_string(&notice).unwrap();
    let mut messages = vec![notice];
    for i in 0..10 {
        messages.push(user(&format!("turn {i}")));
    }

    tier0_deterministic_compact_with(&mut messages, 2, CompactAggression::Aggressive);

    let notices: Vec<&ChatMessage> = messages
        .iter()
        .filter(|message| event_subkind(message) == Some("system_notice"))
        .collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(serde_json::to_string(notices[0]).unwrap(), notice_json);
}
