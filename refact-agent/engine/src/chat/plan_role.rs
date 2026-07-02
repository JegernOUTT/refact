pub use refact_chat_api::plan_role::*;

impl refact_chat_api::plan_role::PlanRoleSession for crate::chat::types::ChatSession {
    fn plan_role_messages(&self) -> &[crate::call_validation::ChatMessage] {
        &self.messages
    }

    fn add_plan_role_message(&mut self, message: crate::call_validation::ChatMessage) {
        self.add_message(message);
    }
}
