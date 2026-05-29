use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::call_validation::{ChatContent, ChatMessage};

pub const EVENT_ROLE: &str = "event";
pub const PLAN_ROLE: &str = "plan";
pub const MAX_PLAN_BODY_CHARS: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSubkind {
    ModeSwitch,
    ToolDecision,
    IdeCallback,
    ProcessCompleted,
    CronFire,
    Tick,
    SummarizationMarker,
    VerifierReport,
    CancellationNote,
    SystemNotice,
}

pub fn event(
    subkind: EventSubkind,
    source: impl Into<String>,
    payload: serde_json::Value,
    content: impl Into<String>,
) -> ChatMessage {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "event".to_string(),
        json!({
            "subkind": subkind,
            "source": source.into(),
            "payload": payload,
        }),
    );
    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: EVENT_ROLE.to_string(),
        content: ChatContent::SimpleText(content.into()),
        extra,
        ..Default::default()
    }
}

pub fn last_is_event(messages: &[ChatMessage]) -> bool {
    messages
        .last()
        .is_some_and(|message| message.role == EVENT_ROLE)
}

pub fn mode_switch_event(
    source: impl Into<String>,
    from: impl AsRef<str>,
    to: impl AsRef<str>,
    reason: Option<&str>,
) -> ChatMessage {
    let from = from.as_ref();
    let to = to.as_ref();
    event(
        EventSubkind::ModeSwitch,
        source,
        json!({ "from": from, "to": to, "reason": reason }),
        format!("Mode switched: {} → {}", from, to),
    )
}

pub fn plan(
    mode: impl Into<String>,
    version: u32,
    content: impl Into<String>,
    supersedes: Option<String>,
) -> ChatMessage {
    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let content_str: String = content.into();
    let original_chars = content_str.chars().count();
    let (body, truncated) = if original_chars > MAX_PLAN_BODY_CHARS {
        let kept: String = content_str.chars().take(MAX_PLAN_BODY_CHARS).collect();
        let marker = format!(
            "\n\n[truncated: kept {} of {} chars]",
            MAX_PLAN_BODY_CHARS, original_chars
        );
        (kept + &marker, true)
    } else {
        (content_str, false)
    };
    let mut plan_meta = json!({
        "mode": mode.into(),
        "version": version,
        "created_at_ms": created_at_ms,
        "supersedes": supersedes,
    });
    if truncated {
        plan_meta["truncated"] = json!(true);
        plan_meta["original_chars"] = json!(original_chars);
    }
    let mut extra = serde_json::Map::new();
    extra.insert("plan".to_string(), plan_meta);
    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: PLAN_ROLE.to_string(),
        content: ChatContent::SimpleText(body),
        extra,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_event_helper_produces_correct_role_and_extra() {
        let msg = event(
            EventSubkind::CronFire,
            "scheduler.cron",
            json!({"task": "daily_digest"}),
            "cron fired",
        );
        assert_eq!(msg.role, EVENT_ROLE);
        let event_meta = msg.extra.get("event").expect("extra.event missing");
        assert_eq!(event_meta["subkind"], json!("cron_fire"));
        assert_eq!(event_meta["source"], json!("scheduler.cron"));
        assert_eq!(event_meta["payload"]["task"], json!("daily_digest"));
    }

    #[test]
    fn test_plan_helper_produces_correct_role_and_extra() {
        let msg = plan("agent", 1, "plan content", Some("prev-plan-id".to_string()));
        assert_eq!(msg.role, PLAN_ROLE);
        let plan_meta = msg.extra.get("plan").expect("extra.plan missing");
        assert_eq!(plan_meta["mode"], json!("agent"));
        assert_eq!(plan_meta["version"], json!(1));
        assert!(plan_meta["created_at_ms"].as_u64().unwrap_or(0) > 0);
        assert_eq!(plan_meta["supersedes"], json!("prev-plan-id"));
    }

    #[test]
    fn test_event_roundtrip() {
        let msg = event(
            EventSubkind::ToolDecision,
            "tool.process_start",
            json!({"tool": "shell"}),
            "tool decision made",
        );
        let serialized = serde_json::to_string(&msg).unwrap();
        let deserialized: ChatMessage = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.role, EVENT_ROLE);
        let event_meta = deserialized
            .extra
            .get("event")
            .expect("extra.event missing");
        assert_eq!(event_meta["subkind"], json!("tool_decision"));
        assert_eq!(event_meta["source"], json!("tool.process_start"));
        assert_eq!(event_meta["payload"], json!({"tool": "shell"}));
    }

    #[test]
    fn test_plan_roundtrip() {
        let msg = plan("task_planner", 2, "detailed plan", None);
        let serialized = serde_json::to_string(&msg).unwrap();
        let deserialized: ChatMessage = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.role, PLAN_ROLE);
        let plan_meta = deserialized.extra.get("plan").expect("extra.plan missing");
        assert_eq!(plan_meta["mode"], json!("task_planner"));
        assert_eq!(plan_meta["version"], json!(2));
        assert!(plan_meta["supersedes"].is_null());
    }

    #[test]
    fn test_plan_truncates_oversized_body() {
        let oversized = "a".repeat(MAX_PLAN_BODY_CHARS + 100);
        let msg = plan("agent", 1, &oversized, None);
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
            _ => panic!("expected SimpleText"),
        };
        assert!(
            body.contains("[truncated:"),
            "truncation marker should be present"
        );
        let plan_meta = msg.extra.get("plan").unwrap();
        assert_eq!(plan_meta["truncated"], json!(true));
        assert_eq!(
            plan_meta["original_chars"].as_u64().unwrap(),
            (MAX_PLAN_BODY_CHARS + 100) as u64
        );
    }

    #[test]
    fn test_event_subkind_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_value(EventSubkind::ModeSwitch).unwrap(),
            json!("mode_switch")
        );
        assert_eq!(
            serde_json::to_value(EventSubkind::ToolDecision).unwrap(),
            json!("tool_decision")
        );
        assert_eq!(
            serde_json::to_value(EventSubkind::IdeCallback).unwrap(),
            json!("ide_callback")
        );
        assert_eq!(
            serde_json::to_value(EventSubkind::ProcessCompleted).unwrap(),
            json!("process_completed")
        );
        assert_eq!(
            serde_json::to_value(EventSubkind::CronFire).unwrap(),
            json!("cron_fire")
        );
        assert_eq!(
            serde_json::to_value(EventSubkind::Tick).unwrap(),
            json!("tick")
        );
        assert_eq!(
            serde_json::to_value(EventSubkind::SummarizationMarker).unwrap(),
            json!("summarization_marker")
        );
        assert_eq!(
            serde_json::to_value(EventSubkind::VerifierReport).unwrap(),
            json!("verifier_report")
        );
        assert_eq!(
            serde_json::to_value(EventSubkind::CancellationNote).unwrap(),
            json!("cancellation_note")
        );
        assert_eq!(
            serde_json::to_value(EventSubkind::SystemNotice).unwrap(),
            json!("system_notice")
        );
    }
}
