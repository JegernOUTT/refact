pub use refact_chat_api::plan_role::*;

impl refact_chat_api::plan_role::PlanRoleSession for crate::chat::types::ChatSession {
    fn plan_role_messages(&self) -> &[crate::call_validation::ChatMessage] {
        &self.messages
    }

    fn add_plan_role_message(&mut self, message: crate::call_validation::ChatMessage) {
        self.add_message(message);
    }
}

pub fn current_base_plan(
    session: &crate::chat::types::ChatSession,
) -> Option<&crate::call_validation::ChatMessage> {
    session
        .accepted_control_messages()
        .filter(|message| message.role == "plan")
        .filter_map(|message| {
            let version = message.extra.get("plan")?.get("version")?.as_u64()?;
            u32::try_from(version)
                .ok()
                .map(|version| (version, message))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, message)| message)
}

pub fn install_plan(
    session: &mut crate::chat::types::ChatSession,
    mode: &str,
    body: &str,
) -> PlanInstallReport {
    if let Some(message) = session
        .post_tool_side_effects
        .iter()
        .chain(
            session
                .pending_deliveries
                .iter()
                .flat_map(|delivery| delivery.messages.iter()),
        )
        .find(|message| message.role == "plan")
    {
        return PlanInstallReport {
            version: message.extra["plan"]["version"].as_u64().unwrap_or(1) as u32,
            supersedes: message.extra["plan"]["supersedes"]
                .as_str()
                .map(str::to_owned),
        };
    }
    refact_chat_api::plan_role::install_plan(session, mode, body)
}
