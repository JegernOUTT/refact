use super::*;
use crate::tools::ToolStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalCell {
    state: ApprovalModalState,
    status: Option<ToolStatus>,
}

impl ApprovalCell {
    pub fn new(state: ApprovalModalState, status: Option<ToolStatus>) -> Self {
        Self { state, status }
    }

    pub fn set_status(&mut self, status: ToolStatus) {
        self.status = Some(status);
    }
}

impl HistoryCell for ApprovalCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Approval
    }

    fn render_raw(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = render_modal_lines(&self.state, width);
        if let Some(status) = self.status {
            lines.push(Line::from(Span::styled(
                format!("approval {}", status.visual()),
                default_theme_style(ThemeRole::Muted),
            )));
        }
        finish(lines)
    }

    fn is_final(&self) -> bool {
        self.status.is_some()
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
            self.status,
        ))
    }
}
