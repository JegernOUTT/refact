use refact_core::chat_types::ChatMessage;

use crate::internal_roles::{self, EVENT_ROLE, PLAN_ROLE};
use crate::role_projection::{
    checkpoint_provenance, delta_revision, project_role_with_prefold, ApprovedBase, ProjectedDelta,
    ProjectedRole, ProjectionLimits, RoleProjection,
};

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

/// The complete append-only audit trail: every delta note with its revision,
/// in order. Checkpointing never removes an entry from this list.
pub fn plan_delta_history<S: PlanRoleSession + ?Sized>(session: &S) -> Vec<(u64, String)> {
    let base_id = current_base_plan(session).map(|base| base.message_id.as_str());
    let folded = base_id
        .and_then(|base_id| {
            plan_delta_events(session)
                .into_iter()
                .filter_map(checkpoint_provenance)
                .find(|checkpoint| checkpoint.approved_message_id == base_id)
        })
        .map_or(0, |checkpoint| checkpoint.folded_deltas);
    let mut revision_index = folded;
    plan_delta_events(session)
        .into_iter()
        .filter_map(|message| {
            if checkpoint_provenance(message).is_some() {
                return None;
            }
            let revision = delta_revision(message, revision_index);
            revision_index += 1;
            Some((revision, message.content.content_text_only()))
        })
        .collect()
}

pub fn effective_plan_delta_count<S: PlanRoleSession + ?Sized>(session: &S) -> usize {
    let folded = current_base_plan(session)
        .and_then(|base| {
            plan_delta_events(session)
                .into_iter()
                .filter_map(checkpoint_provenance)
                .find(|checkpoint| checkpoint.approved_message_id == base.message_id)
        })
        .map_or(0, |checkpoint| checkpoint.folded_deltas);
    folded + plan_delta_history(session).len()
}

/// Bounded, deterministic projection of the approved base plus its deltas.
pub fn project_current_plan<S: PlanRoleSession + ?Sized>(
    session: &S,
    limits: ProjectionLimits,
) -> Option<RoleProjection> {
    let base = current_base_plan(session)?;
    let body = base.content.content_text_only();
    let meta = base.extra.get("plan");
    let approved = ApprovedBase {
        message_id: &base.message_id,
        mode: meta
            .and_then(|meta| meta.get("mode"))
            .and_then(|mode| mode.as_str())
            .unwrap_or_default(),
        version: plan_version(base).unwrap_or(1),
        created_at_ms: meta
            .and_then(|meta| meta.get("created_at_ms"))
            .and_then(|created| created.as_u64())
            .unwrap_or(0),
        body: &body,
    };
    let history = plan_delta_history(session);
    let deltas: Vec<ProjectedDelta<'_>> = history
        .iter()
        .map(|(revision, note)| ProjectedDelta {
            seq: *revision,
            note,
        })
        .collect();
    let prefold = plan_delta_events(session)
        .into_iter()
        .filter_map(checkpoint_provenance)
        .find(|checkpoint| checkpoint.approved_message_id == base.message_id);
    Some(project_role_with_prefold(
        ProjectedRole::Plan,
        &approved,
        prefold.as_ref(),
        &deltas,
        limits,
    ))
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

/// Active control state is owned because its plan may live inside a report payload.
/// Invalid/legacy boundaries are errors, never an absent plan or an archive fallback.
pub fn try_current_plan<S: PlanRoleSession + ?Sized>(
    session: &S,
) -> Result<Option<ChatMessage>, String> {
    let messages = refact_core::active_context::active_context(session.plan_role_messages())
        .map_err(|error| error.to_string())?
        .messages;
    Ok(messages
        .into_iter()
        .enumerate()
        .filter_map(|(index, message)| {
            plan_version(&message).map(|version| (index, version, message))
        })
        .max_by_key(|(index, version, _)| (*version, *index))
        .map(|(_, _, message)| message))
}

pub fn try_current_base_plan<S: PlanRoleSession + ?Sized>(
    session: &S,
) -> Result<Option<ChatMessage>, String> {
    try_current_plan(session)
}

pub fn try_plan_delta_events<S: PlanRoleSession + ?Sized>(
    session: &S,
) -> Result<Vec<ChatMessage>, String> {
    let messages = refact_core::active_context::active_context(session.plan_role_messages())
        .map_err(|error| error.to_string())?
        .messages;
    Ok(messages
        .into_iter()
        .filter(|message| {
            message.role == EVENT_ROLE
                && message
                    .extra
                    .get("event")
                    .and_then(|event| event.get("subkind"))
                    .and_then(|v| v.as_str())
                    == Some("plan_delta")
        })
        .collect())
}

pub fn try_synthesize_current_plan<S: PlanRoleSession + ?Sized>(
    session: &S,
) -> Result<Option<String>, String> {
    let Some(plan) = try_current_base_plan(session)? else {
        return Ok(None);
    };
    let base = plan.content.content_text_only();
    let deltas = try_plan_delta_events(session)?;
    if deltas.is_empty() {
        return Ok(Some(base));
    }
    let notes = deltas
        .into_iter()
        .map(|message| message.content.content_text_only())
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(Some(format!("{base}\n\n---\n\n## Plan updates\n\n{notes}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatEvent, EventEnvelope};

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
                event: ChatEvent::MessageAdded {
                    message,
                    index,
                },
            };
            self.events.push(serde_json::to_string(&envelope).unwrap());
        }
    }

    fn make_session() -> TestSession {
        TestSession::new("test-chat".to_string())
    }

    #[test]
    fn active_plan_and_deltas_exclude_archive() {
        let mut session = make_session();
        install_plan(&mut session, "agent", "ARCHIVED_PLAN");
        session.add_message(internal_roles::plan_delta(
            "test",
            serde_json::json!({}),
            "ARCHIVED_DELTA",
        ));
        let mut active_plan = internal_roles::plan("agent", 1, "ACTIVE_PLAN", None);
        active_plan.message_id = "active-plan".into();
        let mut delta = internal_roles::plan_delta("test", serde_json::json!({}), "ACTIVE_DELTA");
        delta.message_id = "active-delta".into();
        let report = refact_core::active_context::make_reconstruction_report(
            vec![active_plan, delta],
            Default::default(),
        )
        .unwrap();
        session.add_message(report);
        assert_eq!(
            try_current_plan(&session)
                .unwrap()
                .unwrap()
                .content
                .content_text_only(),
            "ACTIVE_PLAN"
        );
        let text = try_synthesize_current_plan(&session).unwrap().unwrap();
        assert!(text.contains("ACTIVE_DELTA"));
        assert!(!text.contains("ARCHIVED"));
        assert_eq!(
            plan_history(&session).len(),
            1,
            "archive display remains available"
        );
    }

    #[test]
    fn active_plan_errors_are_not_missing_plans() {
        let mut session = make_session();
        let mut legacy = ChatMessage::new("assistant".into(), "legacy".into());
        legacy.summarized_range = Some((0, 0));
        session.add_message(legacy);
        assert!(try_current_plan(&session).is_err());
        assert!(try_plan_delta_events(&session).is_err());
        assert!(try_synthesize_current_plan(&session).is_err());
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
}
