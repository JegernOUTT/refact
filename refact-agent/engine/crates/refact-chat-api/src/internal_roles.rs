use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use refact_core::chat_types::{ChatContent, ChatMessage};

use crate::GoalBudget;

pub const EVENT_ROLE: &str = "event";
pub const PLAN_ROLE: &str = "plan";
pub const GOAL_ROLE: &str = "goal";
pub const MAX_PLAN_BODY_CHARS: usize = 96 * 1024;
pub const MAX_PLAN_DELTA_CHARS: usize = 16 * 1024;
pub const MAX_GOAL_BODY_CHARS: usize = 96 * 1024;
pub const MAX_GOAL_DELTA_CHARS: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanDeltaTruncation {
    pub original_chars: usize,
    pub kept_chars: usize,
}

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
    PlanDelta,
    GoalDelta,
    GoalPursuit,
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

pub fn plan_delta(
    source: impl Into<String>,
    payload: serde_json::Value,
    content: impl Into<String>,
) -> ChatMessage {
    plan_delta_with_truncation(source, payload, content).0
}

pub fn plan_delta_with_truncation(
    source: impl Into<String>,
    payload: serde_json::Value,
    content: impl Into<String>,
) -> (ChatMessage, Option<PlanDeltaTruncation>) {
    let (content, truncation) = bounded_delta_note(content, MAX_PLAN_DELTA_CHARS);
    let payload = plan_delta_payload_with_truncation(payload, truncation);
    (
        event(EventSubkind::PlanDelta, source, payload, content),
        truncation,
    )
}

pub fn bounded_plan_delta_note(
    content: impl Into<String>,
) -> (String, Option<PlanDeltaTruncation>) {
    bounded_delta_note(content, MAX_PLAN_DELTA_CHARS)
}

pub fn goal_delta(
    source: impl Into<String>,
    payload: serde_json::Value,
    content: impl Into<String>,
) -> ChatMessage {
    goal_delta_with_truncation(source, payload, content).0
}

pub fn goal_delta_with_truncation(
    source: impl Into<String>,
    payload: serde_json::Value,
    content: impl Into<String>,
) -> (ChatMessage, Option<PlanDeltaTruncation>) {
    let (content, truncation) = bounded_goal_delta_note(content);
    let payload = plan_delta_payload_with_truncation(payload, truncation);
    (
        event(EventSubkind::GoalDelta, source, payload, content),
        truncation,
    )
}

pub fn bounded_goal_delta_note(
    content: impl Into<String>,
) -> (String, Option<PlanDeltaTruncation>) {
    bounded_delta_note(content, MAX_GOAL_DELTA_CHARS)
}

fn bounded_delta_note(
    content: impl Into<String>,
    max_chars: usize,
) -> (String, Option<PlanDeltaTruncation>) {
    let content_str: String = content.into();
    let original_chars = content_str.chars().count();
    if original_chars <= max_chars {
        return (content_str, None);
    }

    let mut kept_chars = max_chars;
    loop {
        let marker = truncation_marker(kept_chars, original_chars);
        let total_chars = kept_chars + marker.chars().count();
        if total_chars <= max_chars || kept_chars == 0 {
            let kept: String = content_str.chars().take(kept_chars).collect();
            return (
                kept + &marker,
                Some(PlanDeltaTruncation {
                    original_chars,
                    kept_chars,
                }),
            );
        }
        kept_chars = kept_chars.saturating_sub(total_chars - max_chars);
    }
}

fn plan_delta_payload_with_truncation(
    payload: serde_json::Value,
    truncation: Option<PlanDeltaTruncation>,
) -> serde_json::Value {
    let Some(truncation) = truncation else {
        return payload;
    };
    let mut payload = match payload {
        serde_json::Value::Object(map) => map,
        value => serde_json::Map::from_iter([("value".to_string(), value)]),
    };
    payload.insert("truncated".to_string(), json!(true));
    payload.insert(
        "original_chars".to_string(),
        json!(truncation.original_chars),
    );
    payload.insert("kept_chars".to_string(), json!(truncation.kept_chars));
    serde_json::Value::Object(payload)
}

fn truncation_marker(kept_chars: usize, original_chars: usize) -> String {
    format!("\n\n[truncated: kept {kept_chars} of {original_chars} chars]")
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

pub fn goal(
    mode: impl Into<String>,
    version: u32,
    content: impl Into<String>,
    supersedes: Option<String>,
    active: bool,
    budget: GoalBudget,
) -> ChatMessage {
    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let content_str: String = content.into();
    let original_chars = content_str.chars().count();
    let (body, truncated) = if original_chars > MAX_GOAL_BODY_CHARS {
        let kept: String = content_str.chars().take(MAX_GOAL_BODY_CHARS).collect();
        let marker = format!(
            "\n\n[truncated: kept {} of {} chars]",
            MAX_GOAL_BODY_CHARS, original_chars
        );
        (kept + &marker, true)
    } else {
        (content_str, false)
    };
    let mut goal_meta = json!({
        "mode": mode.into(),
        "version": version,
        "created_at_ms": created_at_ms,
        "supersedes": supersedes,
        "active": active,
        "budget": budget,
    });
    if truncated {
        goal_meta["truncated"] = json!(true);
        goal_meta["original_chars"] = json!(original_chars);
    }
    let mut extra = serde_json::Map::new();
    extra.insert("goal".to_string(), goal_meta);
    ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: GOAL_ROLE.to_string(),
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
    fn test_goal_helper_produces_correct_role_and_extra() {
        let budget = GoalBudget {
            max_turns: Some(3),
            max_minutes: Some(4),
            max_tokens: Some(5),
            cooldown_ms: 6,
            no_progress_token_threshold: 7,
            no_progress_turns: Some(8),
            explicit: false,
        };
        let msg = goal(
            "agent",
            1,
            "goal content",
            Some("prev-goal-id".to_string()),
            true,
            budget.clone(),
        );
        assert_eq!(msg.role, GOAL_ROLE);
        let goal_meta = msg.extra.get("goal").expect("extra.goal missing");
        assert_eq!(goal_meta["mode"], json!("agent"));
        assert_eq!(goal_meta["version"], json!(1));
        assert!(goal_meta["created_at_ms"].as_u64().unwrap_or(0) > 0);
        assert_eq!(goal_meta["supersedes"], json!("prev-goal-id"));
        assert_eq!(goal_meta["active"], json!(true));
        assert_eq!(goal_meta["budget"], json!(budget));
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
    fn test_goal_roundtrip() {
        let msg = goal(
            "task_agent",
            2,
            "detailed goal",
            None,
            true,
            GoalBudget::default(),
        );
        let serialized = serde_json::to_string(&msg).unwrap();
        let deserialized: ChatMessage = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.role, GOAL_ROLE);
        let goal_meta = deserialized.extra.get("goal").expect("extra.goal missing");
        assert_eq!(goal_meta["mode"], json!("task_agent"));
        assert_eq!(goal_meta["version"], json!(2));
        assert_eq!(goal_meta["active"], json!(true));
        assert!(goal_meta["supersedes"].is_null());
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
    fn test_goal_truncates_oversized_body() {
        let oversized = "a".repeat(MAX_GOAL_BODY_CHARS + 100);
        let msg = goal("agent", 1, &oversized, None, true, GoalBudget::default());
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
            _ => panic!("expected SimpleText"),
        };
        assert!(
            body.contains("[truncated:"),
            "truncation marker should be present"
        );
        let goal_meta = msg.extra.get("goal").unwrap();
        assert_eq!(goal_meta["truncated"], json!(true));
        assert_eq!(
            goal_meta["original_chars"].as_u64().unwrap(),
            (MAX_GOAL_BODY_CHARS + 100) as u64
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
        assert_eq!(
            serde_json::to_value(EventSubkind::PlanDelta).unwrap(),
            json!("plan_delta")
        );
        assert_eq!(
            serde_json::to_value(EventSubkind::GoalDelta).unwrap(),
            json!("goal_delta")
        );
        assert_eq!(
            serde_json::to_value(EventSubkind::GoalPursuit).unwrap(),
            json!("goal_pursuit")
        );
    }

    #[test]
    fn plan_delta_helper_produces_correct_role_and_subkind() {
        let msg = plan_delta("tool.set_plan", json!({"seq": 7}), "append note");

        assert_eq!(msg.role, EVENT_ROLE);
        let event_meta = msg.extra.get("event").expect("extra.event missing");
        assert_eq!(event_meta["subkind"], json!("plan_delta"));
        assert_eq!(event_meta["source"], json!("tool.set_plan"));
        assert_eq!(event_meta["payload"]["seq"], json!(7));
    }

    #[test]
    fn plan_delta_helper_truncates_oversized_content() {
        let oversized = "a".repeat(MAX_PLAN_DELTA_CHARS + 100);
        let msg = plan_delta("tool.update_plan", json!({"seq": 1}), oversized);
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
            _ => panic!("expected SimpleText"),
        };

        assert!(body.chars().count() <= MAX_PLAN_DELTA_CHARS);
        assert!(body.contains("[truncated:"));
        let payload = &msg.extra["event"]["payload"];
        assert_eq!(payload["seq"], json!(1));
        assert_eq!(payload["truncated"], json!(true));
        assert_eq!(payload["original_chars"], json!(MAX_PLAN_DELTA_CHARS + 100));
        assert!(payload["kept_chars"].as_u64().unwrap() < MAX_PLAN_DELTA_CHARS as u64);
    }

    #[test]
    fn plan_delta_with_truncation_returns_message_and_metadata() {
        let oversized = "a".repeat(MAX_PLAN_DELTA_CHARS + 100);
        let (msg, truncation) =
            plan_delta_with_truncation("tool.update_plan", json!({"seq": 1}), oversized);
        let truncation = truncation.expect("expected truncation metadata");
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
            _ => panic!("expected SimpleText"),
        };

        assert!(body.chars().count() <= MAX_PLAN_DELTA_CHARS);
        assert_eq!(truncation.original_chars, MAX_PLAN_DELTA_CHARS + 100);
        assert!(truncation.kept_chars < MAX_PLAN_DELTA_CHARS);
        let payload = &msg.extra["event"]["payload"];
        assert_eq!(payload["original_chars"], json!(truncation.original_chars));
        assert_eq!(payload["kept_chars"], json!(truncation.kept_chars));
    }

    #[test]
    fn plan_delta_helper_leaves_small_content_unchanged() {
        let msg = plan_delta("tool.update_plan", json!({"seq": 1}), "small note");
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
            _ => panic!("expected SimpleText"),
        };

        assert_eq!(body, "small note");
        assert!(msg.extra["event"]["payload"].get("truncated").is_none());
    }

    #[test]
    fn plan_delta_with_truncation_leaves_small_content_unchanged() {
        let (msg, truncation) =
            plan_delta_with_truncation("tool.update_plan", json!({"seq": 1}), "small note");
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
            _ => panic!("expected SimpleText"),
        };

        assert_eq!(body, "small note");
        assert_eq!(truncation, None);
        assert!(msg.extra["event"]["payload"].get("truncated").is_none());
    }

    #[test]
    fn goal_delta_helper_produces_correct_role_and_subkind() {
        let msg = goal_delta("tool.set_goal", json!({"seq": 7}), "append note");

        assert_eq!(msg.role, EVENT_ROLE);
        let event_meta = msg.extra.get("event").expect("extra.event missing");
        assert_eq!(event_meta["subkind"], json!("goal_delta"));
        assert_eq!(event_meta["source"], json!("tool.set_goal"));
        assert_eq!(event_meta["payload"]["seq"], json!(7));
    }

    #[test]
    fn goal_delta_helper_truncates_oversized_content() {
        let oversized = "a".repeat(MAX_GOAL_DELTA_CHARS + 100);
        let msg = goal_delta("tool.update_goal", json!({"seq": 1}), oversized);
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
            _ => panic!("expected SimpleText"),
        };

        assert!(body.chars().count() <= MAX_GOAL_DELTA_CHARS);
        assert!(body.contains("[truncated:"));
        let payload = &msg.extra["event"]["payload"];
        assert_eq!(payload["seq"], json!(1));
        assert_eq!(payload["truncated"], json!(true));
        assert_eq!(payload["original_chars"], json!(MAX_GOAL_DELTA_CHARS + 100));
        assert!(payload["kept_chars"].as_u64().unwrap() < MAX_GOAL_DELTA_CHARS as u64);
    }

    #[test]
    fn goal_delta_with_truncation_returns_message_and_metadata() {
        let oversized = "a".repeat(MAX_GOAL_DELTA_CHARS + 100);
        let (msg, truncation) =
            goal_delta_with_truncation("tool.update_goal", json!({"seq": 1}), oversized);
        let truncation = truncation.expect("expected truncation metadata");
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
            _ => panic!("expected SimpleText"),
        };

        assert!(body.chars().count() <= MAX_GOAL_DELTA_CHARS);
        assert_eq!(truncation.original_chars, MAX_GOAL_DELTA_CHARS + 100);
        assert!(truncation.kept_chars < MAX_GOAL_DELTA_CHARS);
        let payload = &msg.extra["event"]["payload"];
        assert_eq!(payload["original_chars"], json!(truncation.original_chars));
        assert_eq!(payload["kept_chars"], json!(truncation.kept_chars));
    }

    #[test]
    fn goal_delta_helper_leaves_small_content_unchanged() {
        let msg = goal_delta("tool.update_goal", json!({"seq": 1}), "small note");
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
            _ => panic!("expected SimpleText"),
        };

        assert_eq!(body, "small note");
        assert!(msg.extra["event"]["payload"].get("truncated").is_none());
    }

    #[test]
    fn goal_delta_with_truncation_leaves_small_content_unchanged() {
        let (msg, truncation) =
            goal_delta_with_truncation("tool.update_goal", json!({"seq": 1}), "small note");
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
            _ => panic!("expected SimpleText"),
        };

        assert_eq!(body, "small note");
        assert_eq!(truncation, None);
        assert!(msg.extra["event"]["payload"].get("truncated").is_none());
    }
}
