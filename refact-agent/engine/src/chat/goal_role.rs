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
