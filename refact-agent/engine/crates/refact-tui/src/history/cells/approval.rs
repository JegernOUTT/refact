use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApprovalOutcome {
    ApprovedOnce,
    ApprovedForChat,
    Denied,
}

impl ApprovalOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::ApprovedOnce => "approved once",
            Self::ApprovedForChat => "approved for chat",
            Self::Denied => "denied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalCell {
    state: ApprovalModalState,
    outcome: Option<ApprovalOutcome>,
}

impl ApprovalCell {
    pub fn new(state: ApprovalModalState, outcome: Option<ApprovalOutcome>) -> Self {
        Self { state, outcome }
    }

    pub fn set_outcome(&mut self, outcome: ApprovalOutcome) {
        self.outcome = Some(outcome);
    }
}

impl HistoryCell for ApprovalCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Approval
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = render_modal_lines(&self.state, width);
        if let Some(outcome) = self.outcome {
            lines.push(Line::from(Span::styled(
                format!("approval {}", outcome.label()),
                Style::default().fg(Color::DarkGray),
            )));
        }
        finish(lines)
    }

    fn is_final(&self) -> bool {
        self.outcome.is_some()
    }

    fn revision(&self) -> u64 {
        let reasons = self
            .state
            .reasons()
            .iter()
            .map(|reason| {
                (
                    reason.reason_type.as_str(),
                    reason.tool_name.as_str(),
                    reason.command.as_str(),
                    reason.rule.as_str(),
                    reason.tool_call_id.as_str(),
                    reason.integr_config_path.as_deref().unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>();
        revision(&(
            self.kind(),
            self.state.full_args(),
            self.state.pending_after(),
            reasons,
            self.outcome,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::{approval_state, text};

    #[test]
    fn approval_cell_snapshot() {
        let mut cell = ApprovalCell::new(approval_state(), None);
        assert!(!cell.is_final());
        assert!(text(&cell.render(80)).contains("Approval required"));
        cell.set_outcome(ApprovalOutcome::ApprovedOnce);
        assert!(cell.is_final());
        assert!(text(&cell.render(80)).contains("approval approved once"));
    }
}
