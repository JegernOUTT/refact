use refact_chat_api::GoalBudget;

use crate::call_validation::ChatMessage;
use crate::chat::internal_roles::{self, EVENT_ROLE, GOAL_ROLE};
use crate::chat::types::ChatSession;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalInstallReport {
    pub version: u32,
    pub supersedes: Option<String>,
}

pub fn install_goal(
    session: &mut ChatSession,
    mode: &str,
    body: &str,
    active: bool,
    budget: GoalBudget,
) -> GoalInstallReport {
    let previous = current_goal(session);
    let version = previous
        .and_then(goal_version)
        .map_or(1, |version| version + 1);
    let supersedes = previous.map(|message| message.message_id.clone());
    let message = internal_roles::goal(mode, version, body, supersedes.clone(), active, budget);
    session.add_message(message);
    GoalInstallReport {
        version,
        supersedes,
    }
}

pub fn current_goal(session: &ChatSession) -> Option<&ChatMessage> {
    versioned_goals(session)
        .max_by_key(|(index, version, _)| (*version, *index))
        .map(|(_, _, message)| message)
}

pub fn current_base_goal(session: &ChatSession) -> Option<&ChatMessage> {
    current_goal(session)
}

pub fn goal_delta_events(session: &ChatSession) -> Vec<&ChatMessage> {
    session
        .messages
        .iter()
        .filter(|message| {
            message.role == EVENT_ROLE
                && message
                    .extra
                    .get("event")
                    .and_then(|event| event.get("subkind"))
                    .and_then(|subkind| subkind.as_str())
                    == Some("goal_delta")
        })
        .collect()
}

pub fn synthesize_current_goal(session: &ChatSession) -> Option<String> {
    let base = current_base_goal(session)?.content.content_text_only();
    let deltas = goal_delta_events(session);
    if deltas.is_empty() {
        return Some(base);
    }
    let notes = deltas
        .into_iter()
        .map(|message| message.content.content_text_only())
        .collect::<Vec<_>>()
        .join("\n\n");
    Some(format!("{base}\n\n---\n\n## Goal updates\n\n{notes}"))
}

pub fn goal_history(session: &ChatSession) -> Vec<&ChatMessage> {
    let mut goals: Vec<_> = versioned_goals(session).collect();
    goals.sort_by(
        |(left_index, left_version, _), (right_index, right_version, _)| {
            right_version
                .cmp(left_version)
                .then_with(|| right_index.cmp(left_index))
        },
    );
    goals.into_iter().map(|(_, _, message)| message).collect()
}

pub fn versioned_goals(session: &ChatSession) -> impl Iterator<Item = (usize, u32, &ChatMessage)> {
    session
        .messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            goal_version(message).map(|version| (index, version, message))
        })
}

pub fn goal_version(message: &ChatMessage) -> Option<u32> {
    if message.role != GOAL_ROLE {
        return None;
    }
    message
        .extra
        .get("goal")?
        .get("version")?
        .as_u64()
        .and_then(|version| u32::try_from(version).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::{ChatEvent, EventEnvelope};

    fn make_session() -> ChatSession {
        ChatSession::new("test-chat".to_string())
    }

    #[test]
    fn install_three_goals_increments_version() {
        let mut session = make_session();

        let first = install_goal(&mut session, "agent", "one", true, GoalBudget::default());
        let second = install_goal(&mut session, "agent", "two", true, GoalBudget::default());
        let third = install_goal(&mut session, "agent", "three", true, GoalBudget::default());

        assert_eq!([first.version, second.version, third.version], [1, 2, 3]);
        let versions: Vec<u32> = session.messages.iter().filter_map(goal_version).collect();
        assert_eq!(versions, vec![1, 2, 3]);
    }

    #[test]
    fn supersedes_points_to_prior_message_id() {
        let mut session = make_session();

        install_goal(&mut session, "agent", "one", true, GoalBudget::default());
        let first_id = session.messages[0].message_id.clone();
        let second = install_goal(&mut session, "agent", "two", true, GoalBudget::default());

        assert_eq!(second.supersedes.as_deref(), Some(first_id.as_str()));
        assert_eq!(
            session.messages[1].extra["goal"]["supersedes"].as_str(),
            Some(first_id.as_str())
        );
    }

    #[test]
    fn current_goal_returns_highest_version() {
        let mut session = make_session();

        install_goal(&mut session, "agent", "one", true, GoalBudget::default());
        install_goal(&mut session, "agent", "two", true, GoalBudget::default());
        let third = install_goal(&mut session, "agent", "three", true, GoalBudget::default());

        let current = current_goal(&session).unwrap();
        assert_eq!(goal_version(current), Some(third.version));
        assert_eq!(current.message_id, session.messages[2].message_id);
    }

    #[test]
    fn current_base_goal_returns_single_goal() {
        let mut session = make_session();
        install_goal(&mut session, "agent", "base", true, GoalBudget::default());

        let current = current_base_goal(&session).unwrap();

        assert_eq!(current.role, GOAL_ROLE);
        assert_eq!(current.content.content_text_only(), "base");
    }

    #[test]
    fn install_goal_records_active_and_budget() {
        let mut session = make_session();
        let budget = GoalBudget {
            max_turns: 3,
            max_minutes: 4,
            max_tokens: 5,
            cooldown_ms: 6,
            no_progress_token_threshold: 7,
            no_progress_turns: 8,
        };

        install_goal(&mut session, "agent", "base", false, budget.clone());

        let goal_meta = &current_goal(&session).unwrap().extra["goal"];
        assert_eq!(goal_meta["active"], serde_json::json!(false));
        assert_eq!(goal_meta["budget"], serde_json::json!(budget));
    }

    #[test]
    fn goal_delta_events_in_order() {
        let mut session = make_session();
        session.add_message(internal_roles::goal_delta(
            "tool.set_goal",
            serde_json::json!({"seq": 1}),
            "first",
        ));
        session.add_message(internal_roles::event(
            internal_roles::EventSubkind::SystemNotice,
            "system",
            serde_json::json!({}),
            "ignore",
        ));
        session.add_message(internal_roles::goal_delta(
            "tool.set_goal",
            serde_json::json!({"seq": 2}),
            "second",
        ));

        let deltas = goal_delta_events(&session);

        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0].content.content_text_only(), "first");
        assert_eq!(deltas[1].content.content_text_only(), "second");
    }

    #[test]
    fn synthesize_current_goal_concats_base_and_deltas() {
        let mut session = make_session();
        install_goal(
            &mut session,
            "agent",
            "base goal",
            true,
            GoalBudget::default(),
        );
        session.add_message(internal_roles::goal_delta(
            "tool.set_goal",
            serde_json::json!({"seq": 1}),
            "first update",
        ));
        session.add_message(internal_roles::goal_delta(
            "tool.set_goal",
            serde_json::json!({"seq": 2}),
            "second update",
        ));

        let synthesized = synthesize_current_goal(&session).unwrap();

        assert_eq!(
            synthesized,
            "base goal\n\n---\n\n## Goal updates\n\nfirst update\n\nsecond update"
        );
    }

    #[test]
    fn synthesize_current_goal_uses_truncated_delta_content() {
        let mut session = make_session();
        install_goal(
            &mut session,
            "agent",
            "base goal",
            true,
            GoalBudget::default(),
        );
        let oversized = "x".repeat(internal_roles::MAX_GOAL_DELTA_CHARS + 100);
        session.add_message(internal_roles::goal_delta(
            "tool.update_goal",
            serde_json::json!({"seq": 1}),
            oversized,
        ));

        let delta = goal_delta_events(&session)[0];
        let delta_content = delta.content.content_text_only();
        let synthesized = synthesize_current_goal(&session).unwrap();

        assert!(delta_content.chars().count() <= internal_roles::MAX_GOAL_DELTA_CHARS);
        assert!(delta_content.contains("[truncated:"));
        assert!(synthesized.ends_with(&delta_content));
        assert!(synthesized.chars().count() < internal_roles::MAX_GOAL_DELTA_CHARS + 200);
    }

    #[test]
    fn goal_history_desc_by_version() {
        let mut session = make_session();

        install_goal(&mut session, "agent", "one", true, GoalBudget::default());
        install_goal(&mut session, "agent", "two", true, GoalBudget::default());
        install_goal(&mut session, "agent", "three", true, GoalBudget::default());

        let versions: Vec<u32> = goal_history(&session)
            .into_iter()
            .filter_map(goal_version)
            .collect();
        assert_eq!(versions, vec![3, 2, 1]);
    }

    #[test]
    fn oversized_goal_body_is_truncated() {
        let oversized = "x".repeat(internal_roles::MAX_GOAL_BODY_CHARS + 100);
        let mut session = make_session();
        install_goal(
            &mut session,
            "agent",
            &oversized,
            true,
            GoalBudget::default(),
        );
        let msg = current_goal(&session).unwrap();
        let body = match &msg.content {
            crate::call_validation::ChatContent::SimpleText(s) => s.as_str(),
            _ => panic!("expected SimpleText"),
        };
        assert!(
            body.chars().count() < oversized.chars().count(),
            "body should be shorter than original"
        );
        assert!(
            body.contains("[truncated:"),
            "truncation marker must be present"
        );
    }

    #[test]
    fn goal_truncation_preserves_utf8_boundary() {
        let oversized: String = "✓".repeat(internal_roles::MAX_GOAL_BODY_CHARS + 100);
        let mut session = make_session();
        install_goal(
            &mut session,
            "agent",
            &oversized,
            true,
            GoalBudget::default(),
        );
        let msg = current_goal(&session).unwrap();
        let body = match &msg.content {
            crate::call_validation::ChatContent::SimpleText(s) => s.clone(),
            _ => panic!("expected SimpleText"),
        };
        assert!(!body.is_empty());
        assert!(
            std::str::from_utf8(body.as_bytes()).is_ok(),
            "body must be valid UTF-8"
        );
        assert!(
            body.contains("[truncated:"),
            "truncation marker must be present"
        );
    }

    #[test]
    fn goal_metadata_records_truncation() {
        let oversized = "y".repeat(internal_roles::MAX_GOAL_BODY_CHARS + 50);
        let original_len = oversized.chars().count();
        let mut session = make_session();
        install_goal(
            &mut session,
            "agent",
            &oversized,
            true,
            GoalBudget::default(),
        );
        let msg = current_goal(&session).unwrap();
        let goal_meta = msg.extra.get("goal").unwrap();
        assert_eq!(goal_meta["truncated"], serde_json::json!(true));
        assert_eq!(goal_meta["original_chars"], serde_json::json!(original_len));
    }

    #[test]
    fn install_emits_message_added_event() {
        let mut session = make_session();
        let mut rx = session.subscribe();

        let report = install_goal(&mut session, "agent", "one", true, GoalBudget::default());

        assert_eq!(report.version, 1);
        let json = rx.try_recv().unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::MessageAdded { message, index } => {
                assert_eq!(index, 0);
                assert_eq!(message.role, GOAL_ROLE);
                assert_eq!(goal_version(&message), Some(1));
            }
            other => panic!("expected MessageAdded, got {:?}", other),
        }
    }
}
