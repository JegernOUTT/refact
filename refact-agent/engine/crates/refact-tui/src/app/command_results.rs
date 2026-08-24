use super::*;

impl App {
    pub(super) fn handle_command_finished(
        &mut self,
        context: CommandContextTag,
        result: Result<(), String>,
    ) -> AppAction {
        match result {
            Ok(()) => self.handle_command_success(context),
            Err(error) => self.handle_command_failure(context, error),
        }
    }

    fn handle_command_success(&mut self, context: CommandContextTag) -> AppAction {
        match context {
            CommandContextTag::SendMessage { correlation, .. } => {
                self.clear_in_flight_send(&correlation.client_request_id);
                AppAction::None
            }
            CommandContextTag::Abort => {
                if !self.abort_in_flight {
                    return AppAction::None;
                }
                self.abort_in_flight = false;
                self.set_session_state(SessionState::Idle);
                self.clear_approvals();
                self.clear_active_ask_questions();
                self.dispatch_next_queued_input()
            }
            CommandContextTag::RetryFromIndex { .. } => {
                self.pending_backtrack_rollback = None;
                AppAction::None
            }
            CommandContextTag::ToolDecisions { .. } => {
                self.pending_tool_decision_rollback = None;
                AppAction::None
            }
            CommandContextTag::Rename { title } => {
                self.apply_renamed_chat(title);
                AppAction::None
            }
            CommandContextTag::Fork {
                target_chat_id,
                title,
            } => self.open_forked_chat(target_chat_id, title),
            CommandContextTag::Archive { chat_id } => self.apply_archived_chat(chat_id),
            CommandContextTag::Reasoning { .. } | CommandContextTag::Other => AppAction::None,
        }
    }

    fn handle_command_failure(&mut self, context: CommandContextTag, error: String) -> AppAction {
        match context {
            CommandContextTag::SendMessage {
                prompt,
                params,
                correlation,
            } => self.handle_send_message_failure(prompt, params, correlation, error),
            CommandContextTag::Abort => {
                self.retry_hint = retry_hint_from_message(&error);
                self.abort_in_flight = false;
                self.add_notice(format!("Abort failed: {error}"));
                AppAction::None
            }
            CommandContextTag::RetryFromIndex { rollback } => {
                if let Some(rollback) = self.pending_backtrack_rollback.take().or(rollback) {
                    self.restore_backtrack_rollback(rollback);
                }
                self.retry_hint = retry_hint_from_message(&error);
                self.add_notice(format!("Retry failed: {error}"));
                AppAction::None
            }
            CommandContextTag::ToolDecisions { rollback } => {
                if let Some(rollback) = self.pending_tool_decision_rollback.take().or(rollback) {
                    self.restore_tool_decision_rollback(rollback);
                }
                self.retry_hint = retry_hint_from_message(&error);
                self.add_notice(format!("Tool decision failed: {error}"));
                AppAction::None
            }
            CommandContextTag::Rename { .. } => self.notice_command_failure("Rename", error),
            CommandContextTag::Fork { .. } => self.notice_command_failure("Fork", error),
            CommandContextTag::Archive { .. } => self.notice_command_failure("Archive", error),
            CommandContextTag::Reasoning { previous } => {
                self.retry_hint = retry_hint_from_message(&error);
                self.restore_reasoning_snapshot(previous);
                self.add_notice(format!("/reasoning failed: {error}"));
                AppAction::None
            }
            CommandContextTag::Other => self.notice_command_failure("Command", error),
        }
    }

    fn notice_command_failure(&mut self, command: &str, error: String) -> AppAction {
        self.retry_hint = retry_hint_from_message(&error);
        self.add_notice(format!("{command} failed: {error}"));
        AppAction::None
    }
}
