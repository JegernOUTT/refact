pub use refact_chat_api::goal_role::*;

impl refact_chat_api::goal_role::GoalRoleSession for crate::chat::types::ChatSession {
    fn goal_role_messages(&self) -> &[crate::call_validation::ChatMessage] {
        &self.messages
    }

    fn add_goal_role_message(&mut self, message: crate::call_validation::ChatMessage) {
        self.add_message(message);
    }
}
