use std::collections::VecDeque;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::client::ToolDecision;
use crate::render::wrapping::wrap_line;
use crate::render::{color_enabled_from_env, render_unified_diff};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PauseReason {
    pub reason_type: String,
    pub tool_name: String,
    pub command: String,
    pub rule: String,
    pub tool_call_id: String,
    pub integr_config_path: Option<String>,
    pub args: Option<String>,
    pub diff: Option<String>,
}

impl PauseReason {
    pub fn from_value(value: &Value) -> Option<Self> {
        let command = value
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
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
            command: command.clone(),
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
            args: extract_args(value).or_else(|| pretty_json_str(&command)),
            diff: extract_diff(value),
        })
    }
}

fn extract_args(value: &Value) -> Option<String> {
    raw_args_value(value).and_then(pretty_json_value)
}

fn raw_args_value(value: &Value) -> Option<&Value> {
    ["args", "arguments", "tool_args", "input"]
        .iter()
        .find_map(|key| value.get(*key))
}

fn pretty_json_value(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => pretty_jsonish_str(text),
        other => serde_json::to_string_pretty(other).ok(),
    }
}

fn pretty_json_str(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
}

fn pretty_jsonish_str(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    pretty_json_str(trimmed).or_else(|| Some(trimmed.to_string()))
}

fn extract_diff(value: &Value) -> Option<String> {
    ["diff", "preview_diff", "unified_diff", "patch"]
        .iter()
        .find_map(|key| string_field(value, key))
        .or_else(|| raw_args_value(value).and_then(find_diff_field))
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn find_diff_field(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => ["diff", "preview_diff", "unified_diff", "patch"]
            .iter()
            .find_map(|key| {
                map.get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
            })
            .or_else(|| map.values().find_map(find_diff_field)),
        Value::Array(values) => values.iter().find_map(find_diff_field),
        _ => None,
    }
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
    details_open: bool,
    detail_scroll: usize,
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
            details_open: false,
            detail_scroll: 0,
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
        self.details_open
    }

    pub fn details_open(&self) -> bool {
        self.details_open
    }

    pub fn detail_scroll(&self) -> usize {
        self.detail_scroll
    }

    pub fn pending_after(&self) -> usize {
        self.pending_after
    }

    pub fn queue_len(&self) -> usize {
        self.pending_after.saturating_add(1)
    }

    pub fn queue_position(&self) -> usize {
        1
    }

    pub fn queue_label(&self) -> String {
        format!("approval {} of {}", self.queue_position(), self.queue_len())
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

    pub fn toggle_details(&mut self) {
        self.details_open = !self.details_open;
        self.detail_scroll = 0;
    }

    pub fn back_from_details(&mut self) {
        if self.details_open {
            self.details_open = false;
            self.detail_scroll = 0;
        }
    }

    pub fn scroll_details_up(&mut self, amount: usize) {
        if self.details_open {
            self.detail_scroll = self.detail_scroll.saturating_sub(amount);
        }
    }

    pub fn scroll_details_down(&mut self, amount: usize) {
        if self.details_open {
            self.detail_scroll = self.detail_scroll.saturating_add(amount);
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
    let width = width.max(8);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "Approval required",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" · {}", state.queue_label()),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(Span::styled(
            approval_help_text(state.details_open()),
            Style::default().fg(Color::DarkGray),
        )),
    ];
    if state.details_open() {
        render_detail_lines(state, width, &mut lines);
    } else {
        render_summary_lines(state, width, &mut lines);
    }
    lines
}

fn approval_help_text(details_open: bool) -> &'static str {
    if details_open {
        "y approve once · a approve for chat · n reject · v summary · Esc back · ↑/↓ scroll"
    } else {
        "y approve once · a approve for chat · n reject · v details · Esc back"
    }
}

fn render_summary_lines(state: &ApprovalModalState, width: usize, lines: &mut Vec<Line<'static>>) {
    if state.pending_after() > 0 {
        lines.push(Line::from(Span::styled(
            format!("{} more pending in queue", state.pending_after()),
            Style::default().fg(Color::DarkGray),
        )));
    }
    for (idx, reason) in state.reasons().iter().enumerate() {
        let command = reason
            .command
            .is_empty()
            .then(|| args_preview(reason.args.as_deref()))
            .flatten()
            .unwrap_or_else(|| preview_command(&reason.command, width.saturating_sub(12).min(140)));
        let prefix = if state.reasons().len() > 1 {
            format!("• {}/{} ", idx + 1, state.reasons().len())
        } else {
            "• ".to_string()
        };
        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(Color::Cyan)),
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
}

fn args_preview(args: Option<&str>) -> Option<String> {
    args.and_then(|args| {
        args.lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| preview_command(line, 120))
    })
}

fn render_detail_lines(state: &ApprovalModalState, width: usize, lines: &mut Vec<Line<'static>>) {
    lines.push(Line::default());
    for (idx, reason) in state.reasons().iter().enumerate() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("tool {}/{}", idx + 1, state.reasons().len()),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" · {}", reason.tool_name),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        if !reason.tool_call_id.is_empty() {
            lines.push(meta_line("id", &reason.tool_call_id));
        }
        if !reason.rule.is_empty() {
            lines.push(meta_line("rule", &reason.rule));
        }
        if let Some(path) = &reason.integr_config_path {
            lines.push(meta_line("config", path));
        }
        if !reason.command.is_empty() {
            let label = if is_shell_like(&reason.tool_name) {
                "shell command"
            } else {
                "command"
            };
            lines.push(section_line(label));
            push_wrapped_block(
                lines,
                &reason.command,
                width,
                Style::default().fg(Color::White),
            );
        }
        if let Some(args) = &reason.args {
            lines.push(section_line("args"));
            push_wrapped_block(lines, args, width, Style::default().fg(Color::White));
        }
        if let Some(diff) = &reason.diff {
            lines.push(section_line("diff"));
            lines.extend(render_unified_diff(
                diff,
                Some(width.saturating_sub(2).max(8)),
                color_enabled_from_env(),
            ));
        }
        if idx + 1 < state.reasons().len() {
            lines.push(Line::default());
        }
    }
}

fn meta_line(label: &str, value: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {label}: {value}"),
        Style::default().fg(Color::DarkGray),
    ))
}

fn section_line(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {label}:"),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))
}

fn push_wrapped_block(lines: &mut Vec<Line<'static>>, text: &str, width: usize, style: Style) {
    if text.is_empty() {
        lines.push(Line::from("  "));
        return;
    }
    let block_width = width.saturating_sub(2).max(8);
    for raw_line in text.lines() {
        let line = Line::from(vec![
            Span::styled("  ", Style::default().fg(Color::DarkGray)),
            Span::styled(raw_line.to_string(), style),
        ]);
        lines.extend(wrap_line(line, Some(block_width)));
    }
}

fn is_shell_like(tool_name: &str) -> bool {
    matches!(tool_name, "shell" | "bash" | "command" | "run_command")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::wrapping::line_to_plain;
    use serde_json::json;

    fn reason(id: &str) -> PauseReason {
        PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "shell".to_string(),
            command: format!("echo {id}"),
            rule: "*".to_string(),
            tool_call_id: id.to_string(),
            integr_config_path: None,
            args: None,
            diff: None,
        }
    }

    fn text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(line_to_plain)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn detail_toggle_back_and_scroll_state_machine() {
        let mut modal = ApprovalModalState::new(vec![reason("call-1")]);
        assert!(!modal.details_open());
        assert_eq!(modal.detail_scroll(), 0);
        modal.toggle_details();
        assert!(modal.details_open());
        modal.scroll_details_down(1);
        assert_eq!(modal.detail_scroll(), 1);
        modal.back_from_details();
        assert!(!modal.details_open());
        assert_eq!(modal.detail_scroll(), 0);
        modal.back_from_details();
        assert!(!modal.details_open());
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
        assert_eq!(first.queue_label(), "approval 1 of 2");
        assert_eq!(
            queue.pop_front().unwrap().reasons()[0].tool_call_id,
            "call-1"
        );
        let second = queue.front().unwrap();
        assert_eq!(second.reasons()[0].tool_call_id, "call-2");
        assert_eq!(second.pending_after(), 0);
        assert_eq!(second.queue_label(), "approval 1 of 1");
    }

    #[test]
    fn render_modal_lines_reports_queue_count() {
        let mut queue = ApprovalQueue::new();
        queue.push(ApprovalModalState::with_scope(
            "chat-1",
            vec![reason("call-1")],
        ));
        queue.push(ApprovalModalState::with_scope(
            "chat-1",
            vec![reason("call-2")],
        ));

        let rendered = text(&render_modal_lines(queue.front().unwrap(), 80));
        assert!(rendered.contains("approval 1 of 2"));
        assert!(rendered.contains("1 more pending in queue"));
    }

    #[test]
    fn detail_view_renders_pretty_args_command_and_diff() {
        let mut modal = ApprovalModalState::from_event(&json!({
            "reasons": [{
                "type": "confirmation",
                "tool_name": "shell",
                "command": "printf hi",
                "rule": "ask",
                "tool_call_id": "call-1",
                "args": {"command": "printf hi", "cwd": "/tmp/demo"},
                "diff": "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new"
            }]
        }))
        .unwrap();
        modal.toggle_details();

        let rendered = text(&render_modal_lines(&modal, 100));
        assert!(rendered.contains("shell command"));
        assert!(rendered.contains("printf hi"));
        assert!(rendered.contains("args"));
        assert!(rendered.contains("\"command\": \"printf hi\""));
        assert!(rendered.contains("diff"));
        assert!(rendered.contains("- old"));
        assert!(rendered.contains("+ new"));
    }
}
