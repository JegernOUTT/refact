use refact_core::chat_types::ChatMessage;

use crate::internal_roles::{self, EVENT_ROLE, PLAN_ROLE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanInstallReport {
    pub version: u32,
    pub supersedes: Option<String>,
}

pub trait PlanRoleSession {
    fn plan_role_messages(&self) -> &[ChatMessage];
    fn add_plan_role_message(&mut self, message: ChatMessage);
}

impl<T> PlanRoleSession for T
where
    T: std::ops::DerefMut,
    T::Target: PlanRoleSession,
{
    fn plan_role_messages(&self) -> &[ChatMessage] {
        self.deref().plan_role_messages()
    }

    fn add_plan_role_message(&mut self, message: ChatMessage) {
        self.deref_mut().add_plan_role_message(message);
    }
}

pub fn install_plan<S: PlanRoleSession + ?Sized>(
    session: &mut S,
    mode: &str,
    body: &str,
) -> PlanInstallReport {
    let previous = current_plan(session);
    let version = previous
        .and_then(plan_version)
        .map_or(1, |version| version + 1);
    let supersedes = previous.map(|message| message.message_id.clone());
    let message = internal_roles::plan(mode, version, body, supersedes.clone());
    session.add_plan_role_message(message);
    PlanInstallReport {
        version,
        supersedes,
    }
}

pub fn current_plan<S: PlanRoleSession + ?Sized>(session: &S) -> Option<&ChatMessage> {
    versioned_plans(session)
        .max_by_key(|(index, version, _)| (*version, *index))
        .map(|(_, _, message)| message)
}

pub fn current_base_plan<S: PlanRoleSession + ?Sized>(session: &S) -> Option<&ChatMessage> {
    current_plan(session)
}

pub fn plan_delta_events<S: PlanRoleSession + ?Sized>(session: &S) -> Vec<&ChatMessage> {
    session
        .plan_role_messages()
        .iter()
        .filter(|message| {
            message.role == EVENT_ROLE
                && message
                    .extra
                    .get("event")
                    .and_then(|event| event.get("subkind"))
                    .and_then(|subkind| subkind.as_str())
                    == Some("plan_delta")
        })
        .collect()
}

pub fn synthesize_current_plan<S: PlanRoleSession + ?Sized>(session: &S) -> Option<String> {
    let base = current_base_plan(session)?.content.content_text_only();
    let deltas = plan_delta_events(session);
    if deltas.is_empty() {
        return Some(base);
    }
    let notes = deltas
        .into_iter()
        .map(|message| message.content.content_text_only())
        .collect::<Vec<_>>()
        .join("\n\n");
    Some(format!("{base}\n\n---\n\n## Plan updates\n\n{notes}"))
}

pub fn plan_history<S: PlanRoleSession + ?Sized>(session: &S) -> Vec<&ChatMessage> {
    let mut plans: Vec<_> = versioned_plans(session).collect();
    plans.sort_by(
        |(left_index, left_version, _), (right_index, right_version, _)| {
            right_version
                .cmp(left_version)
                .then_with(|| right_index.cmp(left_index))
        },
    );
    plans.into_iter().map(|(_, _, message)| message).collect()
}

fn versioned_plans<'a, S: PlanRoleSession + ?Sized>(
    session: &'a S,
) -> impl Iterator<Item = (usize, u32, &'a ChatMessage)> + 'a {
    session
        .plan_role_messages()
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            plan_version(message).map(|version| (index, version, message))
        })
}

fn plan_version(message: &ChatMessage) -> Option<u32> {
    if message.role != PLAN_ROLE {
        return None;
    }
    message
        .extra
        .get("plan")?
        .get("version")?
        .as_u64()
        .and_then(|version| u32::try_from(version).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatEvent, EventEnvelope};
    use refact_core::chat_types::ChatContent;

    #[derive(Default)]
    struct TestSession {
        chat_id: String,
        messages: Vec<ChatMessage>,
        event_seq: u64,
        events: Vec<String>,
    }

    impl TestSession {
        fn new(chat_id: String) -> Self {
            Self {
                chat_id,
                ..Default::default()
            }
        }

        fn add_message(&mut self, message: ChatMessage) {
            self.add_plan_role_message(message);
        }

        fn install_plan(&mut self, mode: &str, body: &str) -> PlanInstallReport {
            install_plan(self, mode, body)
        }
    }

    impl PlanRoleSession for TestSession {
        fn plan_role_messages(&self) -> &[ChatMessage] {
            &self.messages
        }

        fn add_plan_role_message(&mut self, mut message: ChatMessage) {
            if message.message_id.is_empty() {
                message.message_id = uuid::Uuid::new_v4().to_string();
            }
            let index = self.messages.len();
            self.messages.push(message.clone());
            self.event_seq += 1;
            let envelope = EventEnvelope {
                chat_id: self.chat_id.clone(),
                seq: self.event_seq,
                event: ChatEvent::MessageAdded { message, index },
            };
            self.events.push(serde_json::to_string(&envelope).unwrap());
        }
    }

    fn make_session() -> TestSession {
        TestSession::new("test-chat".to_string())
    }

    #[test]
    fn install_three_plans_increments_version() {
        let mut session = make_session();

        let first = install_plan(&mut session, "agent", "one");
        let second = install_plan(&mut session, "agent", "two");
        let third = install_plan(&mut session, "agent", "three");

        assert_eq!([first.version, second.version, third.version], [1, 2, 3]);
        let versions: Vec<u32> = session.messages.iter().filter_map(plan_version).collect();
        assert_eq!(versions, vec![1, 2, 3]);
    }

    #[test]
    fn supersedes_points_to_prior_message_id() {
        let mut session = make_session();

        install_plan(&mut session, "agent", "one");
        let first_id = session.messages[0].message_id.clone();
        let second = install_plan(&mut session, "agent", "two");

        assert_eq!(second.supersedes.as_deref(), Some(first_id.as_str()));
        assert_eq!(
            session.messages[1].extra["plan"]["supersedes"].as_str(),
            Some(first_id.as_str())
        );
    }

    #[test]
    fn current_plan_returns_highest_version() {
        let mut session = make_session();

        install_plan(&mut session, "agent", "one");
        install_plan(&mut session, "agent", "two");
        let third = install_plan(&mut session, "agent", "three");

        let current = current_plan(&session).unwrap();
        assert_eq!(plan_version(current), Some(third.version));
        assert_eq!(current.message_id, session.messages[2].message_id);
    }

    #[test]
    fn current_base_plan_returns_single_plan() {
        let mut session = make_session();
        install_plan(&mut session, "agent", "base");

        let current = current_base_plan(&session).unwrap();

        assert_eq!(current.role, PLAN_ROLE);
        assert_eq!(current.content.content_text_only(), "base");
    }

    #[test]
    fn plan_delta_events_in_order() {
        let mut session = make_session();
        session.add_message(internal_roles::plan_delta(
            "tool.set_plan",
            serde_json::json!({"seq": 1}),
            "first",
        ));
        session.add_message(internal_roles::event(
            internal_roles::EventSubkind::SystemNotice,
            "system",
            serde_json::json!({}),
            "ignore",
        ));
        session.add_message(internal_roles::plan_delta(
            "tool.set_plan",
            serde_json::json!({"seq": 2}),
            "second",
        ));

        let deltas = plan_delta_events(&session);

        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0].content.content_text_only(), "first");
        assert_eq!(deltas[1].content.content_text_only(), "second");
    }

    #[test]
    fn synthesize_current_plan_concats_base_and_deltas() {
        let mut session = make_session();
        install_plan(&mut session, "agent", "base plan");
        session.add_message(internal_roles::plan_delta(
            "tool.set_plan",
            serde_json::json!({"seq": 1}),
            "first update",
        ));
        session.add_message(internal_roles::plan_delta(
            "tool.set_plan",
            serde_json::json!({"seq": 2}),
            "second update",
        ));

        let synthesized = synthesize_current_plan(&session).unwrap();

        assert_eq!(
            synthesized,
            "base plan\n\n---\n\n## Plan updates\n\nfirst update\n\nsecond update"
        );
    }

    #[test]
    fn synthesize_current_plan_uses_truncated_delta_content() {
        let mut session = make_session();
        install_plan(&mut session, "agent", "base plan");
        let oversized = "x".repeat(internal_roles::MAX_PLAN_DELTA_CHARS + 100);
        session.add_message(internal_roles::plan_delta(
            "tool.update_plan",
            serde_json::json!({"seq": 1}),
            oversized,
        ));

        let delta = plan_delta_events(&session)[0];
        let delta_content = delta.content.content_text_only();
        let synthesized = synthesize_current_plan(&session).unwrap();

        assert!(delta_content.chars().count() <= internal_roles::MAX_PLAN_DELTA_CHARS);
        assert!(delta_content.contains("[truncated:"));
        assert!(synthesized.ends_with(&delta_content));
        assert!(synthesized.chars().count() < internal_roles::MAX_PLAN_DELTA_CHARS + 200);
    }

    #[test]
    fn plan_history_desc_by_version() {
        let mut session = make_session();

        install_plan(&mut session, "agent", "one");
        install_plan(&mut session, "agent", "two");
        install_plan(&mut session, "agent", "three");

        let versions: Vec<u32> = plan_history(&session)
            .into_iter()
            .filter_map(plan_version)
            .collect();
        assert_eq!(versions, vec![3, 2, 1]);
    }

    #[test]
    fn oversized_plan_body_is_truncated() {
        let oversized = "x".repeat(internal_roles::MAX_PLAN_BODY_CHARS + 100);
        let mut session = make_session();
        install_plan(&mut session, "agent", &oversized);
        let msg = current_plan(&session).unwrap();
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.as_str(),
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
    fn plan_truncation_preserves_utf8_boundary() {
        let oversized: String = "✓".repeat(internal_roles::MAX_PLAN_BODY_CHARS + 100);
        let mut session = make_session();
        install_plan(&mut session, "agent", &oversized);
        let msg = current_plan(&session).unwrap();
        let body = match &msg.content {
            ChatContent::SimpleText(s) => s.clone(),
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
    fn plan_metadata_records_truncation() {
        let oversized = "y".repeat(internal_roles::MAX_PLAN_BODY_CHARS + 50);
        let original_len = oversized.chars().count();
        let mut session = make_session();
        install_plan(&mut session, "agent", &oversized);
        let msg = current_plan(&session).unwrap();
        let plan_meta = msg.extra.get("plan").unwrap();
        assert_eq!(plan_meta["truncated"], serde_json::json!(true));
        assert_eq!(plan_meta["original_chars"], serde_json::json!(original_len));
    }

    #[test]
    fn install_emits_message_added_event() {
        let mut session = make_session();
        let report = session.install_plan("agent", "one");

        assert_eq!(report.version, 1);
        let json = session.events.remove(0);
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::MessageAdded { message, index } => {
                assert_eq!(index, 0);
                assert_eq!(message.role, PLAN_ROLE);
                assert_eq!(plan_version(&message), Some(1));
            }
            other => panic!("expected MessageAdded, got {:?}", other),
        }
    }
}
