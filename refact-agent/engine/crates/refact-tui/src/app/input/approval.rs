use super::super::*;

impl App {
    pub(super) fn handle_approval_key(&mut self, key: KeyEvent) -> AppAction {
        let dispatch = self.keymap.dispatch(KeyContext::Approval, key);
        match dispatch.action {
            Some(KeyAction::ApprovalApproveOnce) => {
                self.send_current_tool_decision(true, None, ToolStatus::ApprovedOnce)
            }
            Some(KeyAction::ApprovalApproveForChat) => {
                let patch = self.approval_modal().map(approval_patch);
                self.send_current_tool_decision(true, patch, ToolStatus::ApprovedForChat)
            }
            Some(KeyAction::ApprovalDeny) => {
                self.send_current_tool_decision(false, None, ToolStatus::Denied)
            }
            Some(KeyAction::ApprovalToggleDetails) => {
                if let Some(modal) = self.approval_queue.front_mut() {
                    modal.toggle_details();
                }
                AppAction::None
            }
            Some(KeyAction::Cancel) => {
                if let Some(modal) = self.approval_queue.front_mut() {
                    modal.back_from_details();
                }
                AppAction::None
            }
            Some(KeyAction::MoveUp) => {
                if let Some(modal) = self.approval_queue.front_mut() {
                    modal.scroll_details_up(1);
                }
                AppAction::None
            }
            Some(KeyAction::MoveDown) => {
                if let Some(modal) = self.approval_queue.front_mut() {
                    modal.scroll_details_down(1);
                }
                AppAction::None
            }
            Some(KeyAction::ScrollPageUp) => {
                if let Some(modal) = self.approval_queue.front_mut() {
                    modal.scroll_details_up(5);
                }
                AppAction::None
            }
            Some(KeyAction::ScrollPageDown) => {
                if let Some(modal) = self.approval_queue.front_mut() {
                    modal.scroll_details_down(5);
                }
                AppAction::None
            }
            _ => AppAction::None,
        }
    }

    pub(super) fn handle_approval_paste(&mut self, _text: &str) {}

    fn send_current_tool_decision(
        &mut self,
        accepted: bool,
        patch: Option<Value>,
        outcome: ToolStatus,
    ) -> AppAction {
        let rollback = ToolDecisionRollback {
            approval_queue: self.approval_queue.clone(),
            pending_approval_clears: self.pending_approval_clears.clone(),
            transcript: self.transcript.clone(),
            history: self.history.clone(),
        };
        let Some(modal) = self.pop_current_approval() else {
            return AppAction::None;
        };
        self.set_tool_statuses(modal.tool_call_ids(), outcome);
        self.push_history_item(TranscriptItem::Approval(modal.clone(), Some(outcome)));
        self.pending_tool_decision_rollback = Some(rollback);
        AppAction::SendToolDecisions {
            decisions: modal.decisions(accepted),
            patch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_patch_list_contains_patch() {
        assert!(PATCH_LIKE_FUNCTIONS.contains(&"patch"));
    }
}
