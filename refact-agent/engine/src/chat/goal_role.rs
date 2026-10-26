pub use refact_chat_api::goal_role::*;

impl refact_chat_api::goal_role::GoalRoleSession for crate::chat::types::ChatSession {
    fn goal_role_messages(&self) -> &[crate::call_validation::ChatMessage] {
        &self.messages
    }

    fn add_goal_role_message(&mut self, message: crate::call_validation::ChatMessage) {
        self.add_message(message);
    }
}

pub fn try_current_base_goal(
    session: &crate::chat::types::ChatSession,
) -> Result<Option<crate::call_validation::ChatMessage>, String> {
    let projection = session.try_accepted_control_projection()?;
    Ok(refact_chat_api::goal_role::current_base_goal(&projection).cloned())
}

pub fn current_base_goal(
    session: &crate::chat::types::ChatSession,
) -> Option<crate::call_validation::ChatMessage> {
    try_current_base_goal(session).ok().flatten()
}

pub fn synthesize_current_goal(session: &crate::chat::types::ChatSession) -> Option<String> {
    try_synthesize_current_goal(session).ok().flatten()
}

pub fn try_synthesize_current_goal(
    session: &crate::chat::types::ChatSession,
) -> Result<Option<String>, String> {
    let projection = session.try_accepted_control_projection()?;
    Ok(refact_chat_api::goal_role::synthesize_current_goal(
        &projection,
    ))
}

pub fn goal_delta_events(
    session: &crate::chat::types::ChatSession,
) -> Vec<crate::call_validation::ChatMessage> {
    session
        .accepted_control_messages()
        .filter(|message| {
            message.role == "event"
                && message
                    .extra
                    .get("event")
                    .and_then(|event| event.get("subkind"))
                    .and_then(serde_json::Value::as_str)
                    == Some("goal_delta")
        })
        .collect()
}

pub fn install_goal(
    session: &mut crate::chat::types::ChatSession,
    mode: &str,
    body: &str,
    active: bool,
    budget: refact_chat_api::GoalBudget,
) -> GoalInstallReport {
    let mut projection = match session.try_accepted_control_projection() {
        Ok(projection) => projection,
        Err(error) => {
            tracing::warn!(%error, "Goal installation blocked by invalid active context");
            return GoalInstallReport {
                version: 0,
                supersedes: None,
            };
        }
    };
    if let Some(message) = session
        .post_tool_side_effects
        .iter()
        .chain(
            session
                .pending_deliveries
                .iter()
                .flat_map(|delivery| delivery.messages.iter()),
        )
        .find(|message| message.role == "goal")
    {
        return GoalInstallReport {
            version: message.extra["goal"]["version"].as_u64().unwrap_or(1) as u32,
            supersedes: message.extra["goal"]["supersedes"]
                .as_str()
                .map(str::to_owned),
        };
    }
    let before = projection.messages.len();
    let report =
        refact_chat_api::goal_role::install_goal(&mut projection, mode, body, active, budget);
    for message in projection.messages.into_iter().skip(before) {
        session.add_message(message);
    }
    report
}

pub fn prepare_goal_update(
    session: &crate::chat::types::ChatSession,
    note: Option<&str>,
    plan: Option<&refact_chat_api::GoalPlanMutation>,
    source: &str,
) -> Result<Vec<crate::call_validation::ChatMessage>, String> {
    use refact_chat_api::GoalPlanMutation;
    use serde_json::json;
    if note.is_none() && plan.is_none() {
        return Err("provide at least one of note or plan".into());
    }
    let projection = session.try_accepted_control_projection()?;
    let base = refact_chat_api::goal_role::current_base_goal(&projection)
        .ok_or("no goal to update; call set_goal first")?;
    let active = refact_core::active_context::active_context(&session.messages)
        .map_err(|error| error.to_string())?;
    let known = session
        .messages
        .iter()
        .chain(active.messages.iter())
        .chain(session.task_goal_messages.iter().flatten())
        .chain(session.post_tool_side_effects.iter())
        .chain(
            session
                .pending_deliveries
                .iter()
                .flat_map(|d| d.messages.iter()),
        )
        .map(|m| m.message_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut messages = projection
        .messages
        .iter()
        .filter(|m| {
            m.message_id.starts_with("legacy-plan-") && !known.contains(m.message_id.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    if let Some(note) = note {
        if note.trim().is_empty() {
            return Err("argument `note` must be non-empty".into());
        }
        messages.push(crate::chat::internal_roles::goal_delta(
            source,
            json!({
                "seq": effective_goal_delta_count(&projection) + 1,
                "goal_id": base.message_id,
            }),
            note,
        ));
    }
    if let Some(plan) = plan {
        let existing = current_goal_plan(
            &projection.messages,
            crate::runtime_settings::plan_projection_limits(),
            true,
        );
        let seq = existing.as_ref().map_or(0, |p| p.delta_count) + 1;
        let message = match plan {
            GoalPlanMutation::Install(plan) => {
                if existing.is_some() {
                    return Err("goal plan already exists; append a plan note instead".into());
                }
                let meta = goal_plan_metadata(&plan.content, &session.thread.mode)?;
                crate::chat::internal_roles::goal_delta(
                    source,
                    json!({"target":"plan", "goal_id":base.message_id,"seq":0,"plan":meta}),
                    &plan.content,
                )
            }
            GoalPlanMutation::Append(plan) => {
                if existing.is_none() {
                    return Err("no goal plan to update; install plan content first".into());
                }
                if plan.note.trim().is_empty() {
                    return Err("argument `plan.note` must be non-empty".into());
                }
                crate::chat::internal_roles::goal_delta(
                    source,
                    json!({"target":"plan","goal_id":base.message_id,"seq":seq}),
                    &plan.note,
                )
            }
        };
        messages.push(message);
    }
    Ok(messages)
}

pub fn goal_plan_metadata(content: &str, mode: &str) -> Result<serde_json::Value, String> {
    if content.trim().is_empty() {
        return Err("argument `plan.content` must be non-empty".into());
    }
    let message = crate::chat::internal_roles::plan(mode, 1, content, None);
    let mut meta = message.extra["plan"].clone();
    meta["content"] = serde_json::json!(message.content.content_text_only());
    meta["message_id"] = serde_json::json!(message.message_id);
    Ok(meta)
}

pub fn goal_revision(session: &crate::chat::types::ChatSession) -> u64 {
    session
        .try_accepted_control_projection()
        .map(|p| goal_revision_from_messages(&p.messages))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::ChatSession;
    use refact_core::active_context::{make_reconstruction_report, ReconstructionMetadata};

    #[test]
    fn rebuilt_legacy_goal_update_does_not_replay_imported_controls() {
        let mut session = ChatSession::new("planner-update".into());
        let plan = crate::chat::internal_roles::plan("task_planner", 1, "approved plan", None);
        let controls = import_legacy_plan(&[plan]);
        assert!(controls.iter().any(|message| message.role == "goal"));
        session.messages =
            vec![
                make_reconstruction_report(controls.clone(), ReconstructionMetadata::default())
                    .unwrap(),
            ];
        session.task_goal_messages = Some(controls);
        let updates =
            prepare_goal_update(&session, Some("new outcome"), None, "tool.update_goal").unwrap();
        assert_eq!(
            updates.len(),
            1,
            "existing canonical controls were replayed"
        );
        assert_eq!(updates[0].content.content_text_only(), "new outcome");
        assert_eq!(updates[0].extra["event"]["subkind"], "goal_delta");
    }
}
