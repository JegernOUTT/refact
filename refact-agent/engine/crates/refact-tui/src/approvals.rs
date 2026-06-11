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
pub struct ApprovalModalState {
    reasons: Vec<PauseReason>,
    full_args: bool,
}

impl ApprovalModalState {
    pub fn new(reasons: Vec<PauseReason>) -> Self {
        Self {
            reasons,
            full_args: false,
        }
    }

    pub fn from_event(raw: &Value) -> Option<Self> {
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
            Some(Self::new(reasons))
        }
    }

    pub fn reasons(&self) -> &[PauseReason] {
        &self.reasons
    }

    pub fn full_args(&self) -> bool {
        self.full_args
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
        let modal = ApprovalModalState::new(vec![PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "shell".to_string(),
            command: "echo hi".to_string(),
            rule: "*".to_string(),
            tool_call_id: "call-1".to_string(),
            integr_config_path: None,
        }]);
        assert_eq!(
            modal.decisions(true),
            vec![ToolDecision {
                tool_call_id: "call-1".to_string(),
                accepted: true,
            }]
        );
    }
}
