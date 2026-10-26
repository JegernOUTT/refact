use serde_json::json;

use crate::compression_exemption::{event_subkind, exemption_for, CompressionExemption};
use refact_core::chat_types::{ChatContent, ChatMessage};

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
fn event_exemptions_match_hidden_role_contract() {
    let tick = event("tick", "tool.sleep", "tick");
    let process = event("process_completed", "exec.registry", "done");
    let notice = event("system_notice", "system", "notice");
    let summary = event("summarization_marker", "chat.summarizer", "summary");
    let plan_delta = event("plan_delta", "tool.set_plan", "append-only note");
    let goal_delta = event("goal_delta", "tool.update_goal", "append-only note");
    let goal_pursuit = event("goal_pursuit", "chat.goal", "pursuit turn");

    assert_eq!(event_subkind(&tick), Some("tick"));
    assert_eq!(exemption_for(&tick), CompressionExemption::DropOnAge);
    assert_eq!(exemption_for(&process), CompressionExemption::KeepRecentN);
    assert_eq!(exemption_for(&notice), CompressionExemption::PreserveAnchor);
    assert_eq!(
        exemption_for(&summary),
        CompressionExemption::PreserveAnchor
    );
    assert_eq!(exemption_for(&plan_delta), CompressionExemption::Never);
    assert_eq!(exemption_for(&goal_delta), CompressionExemption::Never);
    assert_eq!(
        exemption_for(&goal_pursuit),
        CompressionExemption::PreserveAnchor
    );
}
