use refact_core::chat_types::ChatMessage;

use crate::GoalBudget;
use crate::internal_roles::{self, EVENT_ROLE, GOAL_ROLE};
use crate::role_projection::{
    checkpoint_provenance, delta_revision, project_role_with_prefold, ApprovedBase, ProjectedDelta,
    ProjectedRole, ProjectionLimits, RoleProjection,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalInstallReport {
    pub version: u32,
    pub supersedes: Option<String>,
}

pub trait GoalRoleSession {
    fn goal_role_messages(&self) -> &[ChatMessage];
    fn add_goal_role_message(&mut self, message: ChatMessage);
}

impl<T> GoalRoleSession for T
where
    T: std::ops::DerefMut,
    T::Target: GoalRoleSession,
{
    fn goal_role_messages(&self) -> &[ChatMessage] {
        self.deref().goal_role_messages()
    }

    fn add_goal_role_message(&mut self, message: ChatMessage) {
        self.deref_mut().add_goal_role_message(message);
    }
}

pub fn install_goal<S: GoalRoleSession + ?Sized>(
    session: &mut S,
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
    session.add_goal_role_message(message);
    GoalInstallReport {
        version,
        supersedes,
    }
}

pub fn current_goal<S: GoalRoleSession + ?Sized>(session: &S) -> Option<&ChatMessage> {
    versioned_goals(session)
        .max_by_key(|(index, version, _)| (*version, *index))
        .map(|(_, _, message)| message)
}

pub fn current_base_goal<S: GoalRoleSession + ?Sized>(session: &S) -> Option<&ChatMessage> {
    current_goal(session)
}

pub fn goal_delta_events<S: GoalRoleSession + ?Sized>(session: &S) -> Vec<&ChatMessage> {
    session
        .goal_role_messages()
        .iter()
        .filter(|message| {
            let base = current_base_goal(session);
            let belongs = base.map_or(true, |base| {
                message
                    .extra
                    .get("event")
                    .unwrap_or(&serde_json::Value::Null)["payload"]["goal_id"]
                    .as_str()
                    .map_or(true, |id| id == base.message_id)
                    && session
                        .goal_role_messages()
                        .iter()
                        .position(|m| std::ptr::eq(m, *message))
                        .unwrap_or(0)
                        > session
                            .goal_role_messages()
                            .iter()
                            .position(|m| std::ptr::eq(m, base))
                            .unwrap_or(0)
            });
            belongs
                && message.role == EVENT_ROLE
                && message
                    .extra
                    .get("event")
                    .and_then(|event| event.get("subkind"))
                    .and_then(|subkind| subkind.as_str())
                    == Some("goal_delta")
                && !is_plan_delta(message)
        })
        .collect()
}

pub fn synthesize_current_goal<S: GoalRoleSession + ?Sized>(session: &S) -> Option<String> {
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

/// The complete append-only audit trail: every delta note with its revision,
/// in order. Checkpointing never removes an entry from this list.
pub fn goal_delta_history<S: GoalRoleSession + ?Sized>(session: &S) -> Vec<(u64, String)> {
    let base_id = current_base_goal(session).map(|base| base.message_id.as_str());
    let folded = base_id
        .and_then(|base_id| {
            goal_delta_events(session)
                .into_iter()
                .filter_map(checkpoint_provenance)
                .find(|checkpoint| checkpoint.approved_message_id == base_id)
        })
        .map_or(0, |checkpoint| checkpoint.folded_deltas);
    let mut revision_index = folded;
    goal_delta_events(session)
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

pub fn effective_goal_delta_count<S: GoalRoleSession + ?Sized>(session: &S) -> usize {
    let folded = current_base_goal(session)
        .and_then(|base| {
            goal_delta_events(session)
                .into_iter()
                .filter_map(checkpoint_provenance)
                .find(|checkpoint| checkpoint.approved_message_id == base.message_id)
        })
        .map_or(0, |checkpoint| checkpoint.folded_deltas);
    folded + goal_delta_history(session).len()
}

/// Bounded, deterministic projection of the approved base plus its deltas.
pub fn project_current_goal<S: GoalRoleSession + ?Sized>(
    session: &S,
    limits: ProjectionLimits,
) -> Option<RoleProjection> {
    let base = current_base_goal(session)?;
    let body = base.content.content_text_only();
    let meta = base.extra.get("goal");
    let approved = ApprovedBase {
        message_id: &base.message_id,
        mode: meta
            .and_then(|meta| meta.get("mode"))
            .and_then(|mode| mode.as_str())
            .unwrap_or_default(),
        version: goal_version(base).unwrap_or(1),
        created_at_ms: meta
            .and_then(|meta| meta.get("created_at_ms"))
            .and_then(|created| created.as_u64())
            .unwrap_or(0),
        body: &body,
    };
    let history = goal_delta_history(session);
    let deltas: Vec<ProjectedDelta<'_>> = history
        .iter()
        .map(|(revision, note)| ProjectedDelta {
            seq: *revision,
            note,
        })
        .collect();
    let prefold = goal_delta_events(session)
        .into_iter()
        .filter_map(checkpoint_provenance)
        .find(|checkpoint| checkpoint.approved_message_id == base.message_id);
    Some(project_role_with_prefold(
        ProjectedRole::Goal,
        &approved,
        prefold.as_ref(),
        &deltas,
        limits,
    ))
}

pub fn goal_history<S: GoalRoleSession + ?Sized>(session: &S) -> Vec<&ChatMessage> {
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

pub fn versioned_goals<'a, S: GoalRoleSession + ?Sized>(
    session: &'a S,
) -> impl Iterator<Item = (usize, u32, &'a ChatMessage)> + 'a {
    session
        .goal_role_messages()
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

pub fn is_plan_delta(message: &ChatMessage) -> bool {
    message
        .extra
        .get("event")
        .and_then(|v| v.get("payload"))
        .and_then(|v| v.get("target"))
        .and_then(|v| v.as_str())
        == Some("plan")
}

struct MessageGoals<'a>(&'a [ChatMessage]);
impl GoalRoleSession for MessageGoals<'_> {
    fn goal_role_messages(&self) -> &[ChatMessage] {
        self.0
    }
    fn add_goal_role_message(&mut self, _: ChatMessage) {
        unreachable!("read-only projection")
    }
}
impl crate::plan_role::PlanRoleSession for MessageGoals<'_> {
    fn plan_role_messages(&self) -> &[ChatMessage] {
        self.0
    }
    fn add_plan_role_message(&mut self, _: ChatMessage) {
        unreachable!("read-only projection")
    }
}

pub fn import_legacy_plan(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let view = MessageGoals(messages);
    let base = current_base_goal(&view);
    if base.is_some_and(|base| base.extra["goal"].get("plan").is_some())
        || messages.iter().any(|m| {
            is_plan_delta(m)
                && m.extra.get("event").unwrap_or(&serde_json::Value::Null)["payload"]
                    .get("plan")
                    .is_some()
                && base.is_some_and(|base| {
                    m.extra.get("event").unwrap_or(&serde_json::Value::Null)["payload"]["goal_id"]
                        .as_str()
                        .map_or(true, |id| id == base.message_id)
                })
        })
    {
        return messages.to_vec();
    }
    let Some(plan) = crate::plan_role::current_base_plan(&view) else {
        return messages.to_vec();
    };
    let mut out = messages.to_vec();
    let goal_id = match base {
        Some(base) => base.message_id.clone(),
        None => {
            let mut goal = internal_roles::goal(
                "legacy_import",
                1,
                "Implement the approved plan.",
                None,
                true,
                GoalBudget::default(),
            );
            goal.extra["goal"]["status"] = serde_json::json!("paused");
            goal.message_id = format!("legacy-plan-goal-{}", plan.message_id);
            goal.extra["goal"]["created_at_ms"] = plan.extra["plan"]["created_at_ms"].clone();
            let id = goal.message_id.clone();
            out.push(goal);
            id
        }
    };
    let mut imported = serde_json::json!({
        "content": plan.content.content_text_only(),
        "mode": plan.extra["plan"]["mode"],
        "version": plan.extra["plan"]["version"],
        "created_at_ms": plan.extra["plan"]["created_at_ms"],
        "message_id": plan.message_id,
        "legacy_source": plan.message_id,
    });
    if let Some(checkpoint) = crate::plan_role::plan_delta_events(&view)
        .into_iter()
        .filter_map(crate::role_projection::checkpoint_provenance)
        .last()
    {
        imported["checkpoint"] = serde_json::to_value(checkpoint).unwrap_or_default();
    }
    let mut install = internal_roles::goal_delta(
        "goal.legacy_import",
        serde_json::json!({
            "target":"plan", "goal_id":goal_id, "plan":imported, "seq":0
        }),
        "Legacy plan imported",
    );
    install.message_id = format!("legacy-plan-install-{}-{}", goal_id, plan.message_id);
    out.push(install);
    for (seq, content) in crate::plan_role::plan_delta_history(&view) {
        let mut delta = internal_roles::goal_delta(
            "goal.legacy_import",
            serde_json::json!({
                "target":"plan", "goal_id":goal_id, "seq":seq
            }),
            content,
        );
        delta.message_id = format!("legacy-plan-note-{}-{}-{}", goal_id, plan.message_id, seq);
        out.push(delta);
    }
    out
}

pub fn goal_revision_from_messages(messages: &[ChatMessage]) -> u64 {
    let view = MessageGoals(messages);
    let Some(base) = current_base_goal(&view) else {
        return 0;
    };
    let start = messages
        .iter()
        .position(|m| std::ptr::eq(m, base))
        .unwrap_or(0);
    let base_revision = base.extra["goal"]["revision"].as_u64().unwrap_or(1);
    base_revision
        + messages
            .iter()
            .skip(start + 1)
            .filter(|m| {
                m.extra.get("event").unwrap_or(&serde_json::Value::Null)["subkind"] == "goal_delta"
                    && m.extra.get("event").unwrap_or(&serde_json::Value::Null)["payload"]
                        ["goal_id"]
                        .as_str()
                        .map_or(true, |id| id == base.message_id)
            })
            .count() as u64
}

pub fn current_goal_plan(
    messages: &[ChatMessage],
    limits: ProjectionLimits,
    include_history: bool,
) -> Option<crate::GoalPlanSnapshot> {
    let messages = import_legacy_plan(messages);
    let view = MessageGoals(&messages);
    let base = current_base_goal(&view)?;
    let mut plan = base.extra["goal"].get("plan").cloned().or_else(|| {
        messages
            .iter()
            .filter(|m| is_plan_delta(m))
            .filter(|m| {
                m.extra.get("event").unwrap_or(&serde_json::Value::Null)["payload"]["goal_id"]
                    .as_str()
                    .map_or(true, |id| id == base.message_id)
            })
            .find_map(|m| {
                m.extra.get("event").unwrap_or(&serde_json::Value::Null)["payload"]
                    .get("plan")
                    .cloned()
            })
    });
    let mut history = Vec::new();
    for message in &messages {
        if !is_plan_delta(message) {
            continue;
        }
        let payload = &message
            .extra
            .get("event")
            .unwrap_or(&serde_json::Value::Null)["payload"];
        if payload["goal_id"]
            .as_str()
            .is_some_and(|id| id != base.message_id)
        {
            continue;
        }
        if let Some(installed) = payload.get("plan") {
            if plan.is_none() {
                plan = Some(installed.clone());
            }
        } else if plan.is_some() {
            history.push(crate::GoalPlanHistoryEntry {
                seq: delta_revision(message, history.len()),
                content: message.content.content_text_only(),
            });
        }
    }
    let plan = plan?;
    let body = plan["content"].as_str()?;
    let mode = plan["mode"].as_str().unwrap_or_default();
    let version = plan["version"].as_u64().unwrap_or(1) as u32;
    let created_at_ms = plan["created_at_ms"].as_u64().unwrap_or_default();
    let message_id = plan["message_id"].as_str().unwrap_or(&base.message_id);
    let prefold = plan
        .get("checkpoint")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let deltas = history
        .iter()
        .map(|h| ProjectedDelta {
            seq: h.seq,
            note: &h.content,
        })
        .collect::<Vec<_>>();
    let projected = project_role_with_prefold(
        ProjectedRole::Plan,
        &ApprovedBase {
            message_id,
            mode,
            version,
            created_at_ms,
            body,
        },
        prefold.as_ref(),
        &deltas,
        limits,
    );
    Some(crate::GoalPlanSnapshot {
        content: projected.text,
        mode: mode.into(),
        version,
        created_at_ms,
        delta_count: projected.delta_count,
        checkpoint: projected.checkpoint,
        history: if include_history { history } else { Vec::new() },
    })
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
            self.add_goal_role_message(message);
        }
    }

    impl GoalRoleSession for TestSession {
        fn goal_role_messages(&self) -> &[ChatMessage] {
            &self.messages
        }

        fn add_goal_role_message(&mut self, mut message: ChatMessage) {
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
            max_turns: Some(3),
            max_minutes: Some(4),
            max_tokens: Some(5),
            max_cost_cents: None,
            cooldown_ms: 6,
            no_progress_token_threshold: 7,
            no_progress_turns: Some(8),
            explicit: false,
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

        let legacy = internal_roles::plan("agent", 1, "Keep approved plan", None);
        session.add_message(legacy.clone());
        let imported = import_legacy_plan(&session.messages);
        assert_eq!(
            serde_json::to_value(import_legacy_plan(&imported)).unwrap(),
            serde_json::to_value(&imported).unwrap()
        );
        assert!(imported
            .iter()
            .any(|m| serde_json::to_value(m).unwrap() == serde_json::to_value(&legacy).unwrap()));
        let plan = current_goal_plan(&imported, ProjectionLimits::default(), true).unwrap();
        assert_eq!(plan.content, "Keep approved plan");
        assert_eq!(plan.delta_count, 0);
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
        let report = install_goal(&mut session, "agent", "one", true, GoalBudget::default());

        assert_eq!(report.version, 1);
        let json = session.events.remove(0);
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
