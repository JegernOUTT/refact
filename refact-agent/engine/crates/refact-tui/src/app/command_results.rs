use super::*;
use crate::commands::{CommandAvailability, CommandContext, CommandDef};

#[derive(Debug, Clone)]
pub(super) struct CommandOrigin {
    pub(super) project_id: String,
    pub(super) chat_id: String,
}

impl CommandOrigin {
    fn is_current(&self, app: &App) -> bool {
        app.current_project_id() == Some(self.project_id.as_str()) && app.chat_id() == self.chat_id
    }
}

impl App {
    pub(super) fn apply_daemon_status(
        &mut self,
        status: DaemonStatus,
        base_url: String,
        source: crate::client::DaemonUrlSource,
    ) {
        self.daemon_online = true;
        self.daemon_status = Some(status);
        self.daemon_base_url = Some(base_url);
        self.daemon_url_source = Some(source);
        self.show_status_card();
    }

    pub(super) fn record_daemon_status_error(&mut self, error: &str) {
        self.daemon_online = false;
        self.retry_hint = retry_hint_from_message(error);
        self.add_notice(format!("Failed to load daemon status: {error}"));
    }

    pub(super) fn command_available(&mut self, command: CommandDef, name: &str) -> bool {
        if command.available(CommandContext {
            active_turn: self.is_chat_active(),
        }) {
            return true;
        }
        let notice = match command.availability {
            CommandAvailability::IdleOnly => {
                format!("/{name} is available between turns only; an active turn is running")
            }
            CommandAvailability::ActiveTurnOnly => {
                format!("/{name} is only available while a response is running")
            }
            CommandAvailability::Always => return true,
        };
        self.add_notice(notice);
        false
    }

    pub(super) fn command_origin(&self) -> CommandOrigin {
        CommandOrigin {
            project_id: self.current_project_id().unwrap_or_default().to_string(),
            chat_id: self.chat_id().to_string(),
        }
    }

    pub(super) fn handle_command_finished(
        &mut self,
        context: CommandContextTag,
        result: Result<(), String>,
    ) -> AppAction {
        let origin = match &context {
            CommandContextTag::Abort { origin }
            | CommandContextTag::BrowserContextDecision { origin, .. }
            | CommandContextTag::RetryFromIndex { origin, .. }
            | CommandContextTag::ToolDecisions { origin, .. }
            | CommandContextTag::Rename { origin, .. }
            | CommandContextTag::Fork { origin, .. }
            | CommandContextTag::Archive { origin, .. } => Some(origin),
            _ => None,
        };
        if origin.is_some_and(|origin| !origin.is_current(self)) {
            return AppAction::None;
        }
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
            CommandContextTag::Abort { .. } => {
                if !self.abort_in_flight {
                    return AppAction::None;
                }
                self.abort_in_flight = false;
                self.set_session_state(SessionState::Idle);
                self.clear_approvals();
                self.clear_active_ask_questions();
                self.dispatch_next_queued_input()
            }
            CommandContextTag::BrowserContextDecision { .. } => AppAction::None,
            CommandContextTag::RetryFromIndex { .. } => {
                self.pending_backtrack_rollback = None;
                AppAction::None
            }
            CommandContextTag::ToolDecisions {
                client_request_id, ..
            } => {
                let _ = client_request_id;
                AppAction::None
            }
            CommandContextTag::Rename { title, .. } => {
                self.apply_renamed_chat(title);
                AppAction::None
            }
            CommandContextTag::Fork {
                target_chat_id,
                title,
                ..
            } => self.open_forked_chat(target_chat_id, title),
            CommandContextTag::Archive { chat_id, .. } => self.apply_archived_chat(chat_id),
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
            CommandContextTag::Abort { .. } => {
                self.retry_hint = retry_hint_from_message(&error);
                self.abort_in_flight = false;
                self.add_notice(format!("Abort failed: {error}"));
                AppAction::None
            }
            CommandContextTag::BrowserContextDecision { prompt, .. } => {
                self.browser_state.restore_context_prompt(prompt);
                self.notice_command_failure("Browser context decision", error)
            }
            CommandContextTag::RetryFromIndex { rollback, .. } => {
                if let Some(rollback) = self.pending_backtrack_rollback.take().or(rollback) {
                    self.restore_backtrack_rollback(rollback);
                }
                self.retry_hint = retry_hint_from_message(&error);
                self.add_notice(format!("Retry failed: {error}"));
                AppAction::None
            }
            CommandContextTag::ToolDecisions {
                client_request_id,
                rollback,
                ..
            } => {
                let _ = client_request_id;
                self.restore_tool_decision_rollback(rollback);
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

    fn restore_tool_decision_rollback(&mut self, rollback: ToolDecisionRollback) {
        let scope = rollback.approval.scope().to_string();
        self.pending_approval_clears
            .retain(|pending| pending.scope != scope);
        self.approval_queue.remove_scope(&scope);
        self.approval_queue.push_front(rollback.approval);
        for (tool_call_id, status) in rollback.tool_statuses {
            self.set_tool_statuses(&[tool_call_id], status);
        }
        let history_changed = self.history.remove_approval_scope(&scope);
        if let Some(index) = self.transcript.iter().rposition(|item| {
            matches!(item, TranscriptItem::Approval(approval, Some(_)) if approval.scope() == scope)
        }) {
            self.transcript.remove(index);
        }
        if history_changed && self.native_scrollback && self.history.inserted_cell_count() > 0 {
            self.resize_reflow.schedule_immediate();
        }
    }

    pub(super) fn tool_statuses(&self, tool_call_ids: &[String]) -> Vec<(String, ToolStatus)> {
        let mut seen = HashSet::new();
        let mut statuses = self
            .transcript
            .iter()
            .filter_map(|item| match item {
                TranscriptItem::Tool(card) if tool_call_ids.iter().any(|id| id == &card.id) => seen
                    .insert(card.id.clone())
                    .then_some((card.id.clone(), card.status)),
                _ => None,
            })
            .collect::<Vec<_>>();
        statuses.extend(
            self.history
                .tool_statuses(tool_call_ids)
                .into_iter()
                .filter(|(tool_call_id, _)| seen.insert(tool_call_id.clone())),
        );
        statuses
    }

    #[cfg(test)]
    pub(super) fn history_tool_status_for_test(&self, tool_call_id: &str) -> Option<ToolStatus> {
        self.history
            .tool_statuses(&[tool_call_id.to_string()])
            .into_iter()
            .find_map(|(id, status)| (id == tool_call_id).then_some(status))
    }
}
