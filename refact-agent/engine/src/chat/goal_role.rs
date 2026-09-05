pub use refact_chat_api::goal_role::*;

impl refact_chat_api::goal_role::GoalRoleSession for crate::chat::types::ChatSession {
    fn goal_role_messages(&self) -> &[crate::call_validation::ChatMessage] {
        &self.messages
    }

    fn add_goal_role_message(&mut self, message: crate::call_validation::ChatMessage) {
        self.add_message(message);
    }
}

pub fn current_base_goal(
    session: &crate::chat::types::ChatSession,
) -> Option<&crate::call_validation::ChatMessage> {
    session
        .accepted_control_messages()
        .filter(|message| message.role == "goal")
        .filter_map(|message| {
            let version = message.extra.get("goal")?.get("version")?.as_u64()?;
            u32::try_from(version)
                .ok()
                .map(|version| (version, message))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, message)| message)
}

pub fn install_goal(
    session: &mut crate::chat::types::ChatSession,
    mode: &str,
    body: &str,
    active: bool,
    budget: refact_chat_api::GoalBudget,
) -> GoalInstallReport {
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
    refact_chat_api::goal_role::install_goal(session, mode, body, active, budget)
}
