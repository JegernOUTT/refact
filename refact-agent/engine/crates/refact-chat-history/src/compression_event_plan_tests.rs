use serde_json::json;

use crate::compression_exemption::{event_subkind, exemption_for, CompressionExemption};
use crate::history_limit::CompressionStrength;
use refact_core::chat_types::{ChatContent, ChatMessage};

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

#[test]
fn plan_is_never_compressed_for_all_strength_values() {
    let plan = plan("PLAN: keep this byte-for-byte");
    let plan_json = serde_json::to_string(&plan).unwrap();
    let strengths = [
        CompressionStrength::Absent,
        CompressionStrength::Low,
        CompressionStrength::Medium,
        CompressionStrength::High,
    ];

    for strength in strengths {
        assert_eq!(
            exemption_for(&plan),
            CompressionExemption::Never,
            "{strength:?}"
        );
        assert_eq!(serde_json::to_string(&plan).unwrap(), plan_json);
    }
}

#[test]
fn event_exemptions_match_hidden_role_contract() {
    let tick = event("tick", "tool.sleep", "tick");
    let process = event("process_completed", "exec.registry", "done");
    let notice = event("system_notice", "system", "notice");
    let summary = event("summarization_marker", "chat.summarizer", "summary");
    let plan_delta = event("plan_delta", "tool.set_plan", "append-only note");

    assert_eq!(event_subkind(&tick), Some("tick"));
    assert_eq!(exemption_for(&tick), CompressionExemption::DropOnAge);
    assert_eq!(exemption_for(&process), CompressionExemption::KeepRecentN);
    assert_eq!(exemption_for(&notice), CompressionExemption::PreserveAnchor);
    assert_eq!(
        exemption_for(&summary),
        CompressionExemption::PreserveAnchor
    );
    assert_eq!(exemption_for(&plan_delta), CompressionExemption::Never);
}
