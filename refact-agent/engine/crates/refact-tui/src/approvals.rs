use std::collections::VecDeque;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::client::ToolDecision;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PauseReason {
    pub reason_type: String,
    pub tool_name: String,
    pub command: String,
    pub rule: String,
    pub tool_call_id: String,
    pub integr_config_path: Option<String>,
}

impl PauseReason {
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            reason_type: value
                .get("type")
                .or_else(|| value.get("raw_type"))
                .and_then(Value::as_str)
                .unwrap_or("confirmation")
                .to_string(),
            tool_name: value
                .get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string(),
            command: value
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            rule: value
                .get("rule")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            tool_call_id: value
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            integr_config_path: value
                .get("integr_config_path")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalKeyAction {
    None,
    ApproveOnce,
    ApproveForChat,
    Deny,
    ToggleFullArgs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovalQueueKey {
    scope: String,
    tool_call_ids: Vec<String>,
}

impl ApprovalQueueKey {
    fn from_reasons(scope: impl Into<String>, reasons: &[PauseReason]) -> Self {
        let mut tool_call_ids = reasons
            .iter()
            .filter(|reason| !reason.tool_call_id.is_empty())
            .map(|reason| reason.tool_call_id.clone())
            .collect::<Vec<_>>();
        if tool_call_ids.is_empty() {
            tool_call_ids = reasons
                .iter()
                .map(|reason| format!("{}:{}:{}", reason.tool_name, reason.command, reason.rule))
                .collect();
        }
        Self {
            scope: scope.into(),
            tool_call_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalModalState {
    key: ApprovalQueueKey,
    reasons: Vec<PauseReason>,
    full_args: bool,
    pending_after: usize,
}

impl ApprovalModalState {
    pub fn new(reasons: Vec<PauseReason>) -> Self {
        Self::with_scope(String::new(), reasons)
    }

    pub fn with_scope(scope: impl Into<String>, reasons: Vec<PauseReason>) -> Self {
        let scope = scope.into();
        Self {
            key: ApprovalQueueKey::from_reasons(scope, &reasons),
            reasons,
            full_args: false,
            pending_after: 0,
        }
    }

    pub fn from_event(raw: &Value) -> Option<Self> {
        Self::from_event_in_scope(String::new(), raw)
    }

    pub fn from_event_in_scope(scope: impl Into<String>, raw: &Value) -> Option<Self> {
        let reasons = raw
            .get("reasons")
            .or_else(|| raw.get("pause_reasons"))
            .and_then(Value::as_array)?
            .iter()
            .filter_map(PauseReason::from_value)
            .collect::<Vec<_>>();
        if reasons.is_empty() {
            None
        } else {
            Some(Self::with_scope(scope, reasons))
        }
    }

    pub fn reasons(&self) -> &[PauseReason] {
        &self.reasons
    }

    pub fn full_args(&self) -> bool {
        self.full_args
    }

    pub fn pending_after(&self) -> usize {
        self.pending_after
    }

    pub fn is_empty(&self) -> bool {
        self.reasons.is_empty()
    }

    pub fn decisions(&self, accepted: bool) -> Vec<ToolDecision> {
        self.reasons
            .iter()
            .filter(|reason| !reason.tool_call_id.is_empty())
            .map(|reason| ToolDecision {
                tool_call_id: reason.tool_call_id.clone(),
                accepted,
            })
            .collect()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ApprovalKeyAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => ApprovalKeyAction::ApproveOnce,
            KeyCode::Char('a') | KeyCode::Char('A') => ApprovalKeyAction::ApproveForChat,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => ApprovalKeyAction::Deny,
            KeyCode::Char('v') | KeyCode::Char('V') => {
                self.full_args = !self.full_args;
                ApprovalKeyAction::ToggleFullArgs
            }
            KeyCode::Enter => ApprovalKeyAction::None,
            _ => ApprovalKeyAction::None,
        }
    }

    fn set_pending_after(&mut self, pending_after: usize) {
        self.pending_after = pending_after;
    }

    fn same_key(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApprovalQueue {
    pending: VecDeque<ApprovalModalState>,
}

impl ApprovalQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn front(&self) -> Option<&ApprovalModalState> {
        self.pending.front()
    }

    pub fn front_mut(&mut self) -> Option<&mut ApprovalModalState> {
        self.pending.front_mut()
    }

    pub fn push(&mut self, mut modal: ApprovalModalState) -> bool {
        if self.pending.iter().any(|queued| queued.same_key(&modal)) {
            self.refresh_pending_counts();
            return false;
        }
        modal.set_pending_after(0);
        self.pending.push_back(modal);
        self.refresh_pending_counts();
        true
    }

    pub fn pop_front(&mut self) -> Option<ApprovalModalState> {
        let modal = self.pending.pop_front();
        self.refresh_pending_counts();
        modal
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    fn refresh_pending_counts(&mut self) {
        let len = self.pending.len();
        for (idx, modal) in self.pending.iter_mut().enumerate() {
            modal.set_pending_after(len.saturating_sub(idx + 1));
        }
    }
}

pub fn preview_command(command: &str, max_chars: usize) -> String {
    let compact = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut out = compact.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}

pub fn render_modal_lines(state: &ApprovalModalState, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "Approval required",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "y approve once · a approve for chat · n/Esc deny · v toggle args",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    if state.pending_after() > 0 {
        lines.push(Line::from(Span::styled(
            format!("{} more pending", state.pending_after()),
            Style::default().fg(Color::DarkGray),
        )));
    }
    for reason in state.reasons() {
        let command = if state.full_args() {
            reason.command.clone()
        } else {
            preview_command(&reason.command, width.saturating_sub(8).min(140))
        };
        lines.push(Line::from(vec![
            Span::styled("• ", Style::default().fg(Color::Cyan)),
            Span::styled(
                reason.tool_name.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(command, Style::default().fg(Color::White)),
        ]));
        if !reason.rule.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  rule: {}", reason.rule),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn reason(id: &str) -> PauseReason {
        PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "shell".to_string(),
            command: format!("echo {id}"),
            rule: "*".to_string(),
            tool_call_id: id.to_string(),
            integr_config_path: None,
        }
    }

    #[test]
    fn approval_keymap_maps_decisions_without_enter_default() {
        let mut modal = ApprovalModalState::new(vec![]);
        assert_eq!(
            modal.handle_key(key(KeyCode::Char('y'))),
            ApprovalKeyAction::ApproveOnce
        );
        assert_eq!(
            modal.handle_key(key(KeyCode::Char('a'))),
            ApprovalKeyAction::ApproveForChat
        );
        assert_eq!(
            modal.handle_key(key(KeyCode::Char('n'))),
            ApprovalKeyAction::Deny
        );
        assert_eq!(modal.handle_key(key(KeyCode::Esc)), ApprovalKeyAction::Deny);
        assert_eq!(
            modal.handle_key(key(KeyCode::Enter)),
            ApprovalKeyAction::None
        );
        assert_eq!(
            modal.handle_key(key(KeyCode::Char('v'))),
            ApprovalKeyAction::ToggleFullArgs
        );
        assert!(modal.full_args());
    }

    #[test]
    fn approval_decisions_preserve_tool_ids() {
        let modal = ApprovalModalState::new(vec![reason("call-1")]);
        assert_eq!(
            modal.decisions(true),
            vec![ToolDecision {
                tool_call_id: "call-1".to_string(),
                accepted: true,
            }]
        );
    }

    #[test]
    fn approval_queue_preserves_fifo_and_pending_count() {
        let mut queue = ApprovalQueue::new();
        assert!(queue.push(ApprovalModalState::with_scope(
            "chat-1",
            vec![reason("call-1")]
        )));
        assert!(queue.push(ApprovalModalState::with_scope(
            "chat-1",
            vec![reason("call-2")]
        )));
        assert!(!queue.push(ApprovalModalState::with_scope(
            "chat-1",
            vec![reason("call-2")]
        )));

        let first = queue.front().unwrap();
        assert_eq!(first.reasons()[0].tool_call_id, "call-1");
        assert_eq!(first.pending_after(), 1);
        assert_eq!(
            queue.pop_front().unwrap().reasons()[0].tool_call_id,
            "call-1"
        );
        let second = queue.front().unwrap();
        assert_eq!(second.reasons()[0].tool_call_id, "call-2");
        assert_eq!(second.pending_after(), 0);
    }

    #[test]
    fn render_modal_lines_reports_more_pending() {
        let mut queue = ApprovalQueue::new();
        queue.push(ApprovalModalState::with_scope(
            "chat-1",
            vec![reason("call-1")],
        ));
        queue.push(ApprovalModalState::with_scope(
            "chat-1",
            vec![reason("call-2")],
        ));

        let text = render_modal_lines(queue.front().unwrap(), 80)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("1 more pending"));
    }
}
