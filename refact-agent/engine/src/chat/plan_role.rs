use crate::call_validation::ChatMessage;
use crate::chat::internal_roles::{self, PLAN_ROLE};
use crate::chat::types::ChatSession;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanInstallReport {
    pub version: u32,
    pub supersedes: Option<String>,
}

pub fn install_plan(session: &mut ChatSession, mode: &str, body: &str) -> PlanInstallReport {
    let previous = current_plan(session);
    let version = previous
        .and_then(plan_version)
        .map_or(1, |version| version + 1);
    let supersedes = previous.map(|message| message.message_id.clone());
    let message = internal_roles::plan(mode, version, body, supersedes.clone());
    session.add_message(message);
    PlanInstallReport {
        version,
        supersedes,
    }
}

pub fn current_plan(session: &ChatSession) -> Option<&ChatMessage> {
    versioned_plans(session)
        .max_by_key(|(index, version, _)| (*version, *index))
        .map(|(_, _, message)| message)
}

pub fn plan_history(session: &ChatSession) -> Vec<&ChatMessage> {
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

fn versioned_plans(session: &ChatSession) -> impl Iterator<Item = (usize, u32, &ChatMessage)> {
    session
        .messages
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
    use crate::chat::types::{ChatEvent, EventEnvelope};

    fn make_session() -> ChatSession {
        ChatSession::new("test-chat".to_string())
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
    fn install_emits_message_added_event() {
        let mut session = make_session();
        let mut rx = session.subscribe();

        let report = session.install_plan("agent", "one");

        assert_eq!(report.version, 1);
        let json = rx.try_recv().unwrap();
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
