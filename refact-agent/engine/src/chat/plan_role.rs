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
) -> Option<crate::call_validation::ChatMessage> {
    try_current_base_plan(session).ok().flatten()
}

pub fn install_plan(
    session: &mut crate::chat::types::ChatSession,
    mode: &str,
    body: &str,
) -> PlanInstallReport {
    let projection = match session.try_accepted_control_projection() {
        Ok(projection) => projection,
        Err(error) => {
            session.set_runtime_state(crate::chat::types::SessionState::Error, Some(error));
            return PlanInstallReport {
                version: 0,
                supersedes: None,
            };
        }
    };
    if let Some(message) = refact_chat_api::plan_role::current_base_plan(&projection) {
        return PlanInstallReport {
            version: message.extra["plan"]["version"].as_u64().unwrap_or(1) as u32,
            supersedes: message.extra["plan"]["supersedes"]
                .as_str()
                .map(str::to_owned),
        };
    }
    session.add_message(crate::chat::internal_roles::plan(mode, 1, body, None));
    PlanInstallReport {
        version: 1,
        supersedes: None,
    }
}

pub fn try_current_base_plan(
    session: &crate::chat::types::ChatSession,
) -> Result<Option<crate::call_validation::ChatMessage>, String> {
    // Queued accepted controls are not yet part of stored history, so append them
    // after projection instead of treating archived controls as current.
    let mut messages = refact_core::active_context::active_context(&session.messages)
        .map_err(|error| error.to_string())?
        .messages;
    messages.extend(session.post_tool_side_effects.iter().cloned());
    messages.extend(
        session
            .pending_deliveries
            .iter()
            .flat_map(|delivery| delivery.messages.iter().cloned()),
    );
    Ok(messages
        .into_iter()
        .filter(|message| message.role == "plan")
        .filter_map(|message| {
            let version = message.extra.get("plan")?.get("version")?.as_u64()?;
            u32::try_from(version)
                .ok()
                .map(|version| (version, message))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, message)| message))
}
