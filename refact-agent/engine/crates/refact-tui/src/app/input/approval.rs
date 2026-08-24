use super::super::*;

impl App {
    pub(super) fn handle_approval_key(&mut self, key: KeyEvent) -> AppAction {
        let dispatch = self.keymap.dispatch(KeyContext::Approval, key);
        match dispatch.action {
            Some(KeyAction::ApprovalApproveOnce) => self
                .pop_current_approval()
                .map(|modal| {
                    self.set_tool_statuses(modal.tool_call_ids(), ToolStatus::ApprovedOnce);
                    self.push_history_item(TranscriptItem::Approval(
                        modal.clone(),
                        Some(ToolStatus::ApprovedOnce),
                    ));
                    AppAction::SendToolDecisions {
                        decisions: modal.decisions(true),
                        patch: None,
                    }
                })
                .unwrap_or(AppAction::None),
            Some(KeyAction::ApprovalApproveForChat) => self
                .pop_current_approval()
                .map(|modal| {
                    let patch = approval_patch(&modal);
                    self.set_tool_statuses(modal.tool_call_ids(), ToolStatus::ApprovedForChat);
                    self.push_history_item(TranscriptItem::Approval(
                        modal.clone(),
                        Some(ToolStatus::ApprovedForChat),
                    ));
                    AppAction::SendToolDecisions {
                        patch: Some(patch),
                        decisions: modal.decisions(true),
                    }
                })
                .unwrap_or(AppAction::None),
            Some(KeyAction::ApprovalDeny) => self
                .pop_current_approval()
                .map(|modal| {
                    self.set_tool_statuses(modal.tool_call_ids(), ToolStatus::Denied);
                    self.push_history_item(TranscriptItem::Approval(
                        modal.clone(),
                        Some(ToolStatus::Denied),
                    ));
                    AppAction::SendToolDecisions {
                        decisions: modal.decisions(false),
                        patch: None,
                    }
                })
                .unwrap_or(AppAction::None),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_patch_list_contains_patch() {
        assert!(PATCH_LIKE_FUNCTIONS.contains(&"patch"));
    }
}
