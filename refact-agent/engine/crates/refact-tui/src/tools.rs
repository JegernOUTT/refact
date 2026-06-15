use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::render::{color_enabled_from_env, is_unified_diff, render_unified_diff};
use crate::text_safety::{
    compact_tool_preview, sanitize_json_strings, sanitize_tool_inline, sanitize_tool_text,
};

const MAX_RESULT_LINES: usize = 200;
pub const MAX_SUBCHAT_DEPTH: usize = 5;
pub const MAX_SUBCHAT_ATTACHED_FILES: usize = 12;
pub const MAX_SUBCHAT_PROGRESS_CHARS: usize = 2000;
const COLLAPSED_SUBCHAT_LINES: usize = 2;
const EXPANDED_SUBCHAT_LINES: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolStatus {
    Running,
    Success,
    Error,
}

impl ToolStatus {
    pub fn icon(self) -> &'static str {
        match self {
            ToolStatus::Running => "⏳",
            ToolStatus::Success => "✅",
            ToolStatus::Error => "❌",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolCard {
    pub id: String,
    pub name: String,
    pub args: String,
    pub args_preview: String,
    pub result: String,
    pub status: ToolStatus,
    pub duration_ms: Option<u64>,
    pub started_at_ms: u64,
    pub expanded: bool,
    pub subchat_log: Vec<String>,
    pub attached_files: Vec<String>,
    pub subchat_depth: usize,
    pub subchat_updates: usize,
    pub subchat_active: bool,
    pub subchat_truncated: bool,
}

impl ToolCard {
    pub fn from_tool_call(value: &Value) -> Self {
        let id = value
            .get("id")
            .or_else(|| value.get("tool_call_id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let name = sanitize_tool_inline(
            value
                .get("function")
                .and_then(|function| function.get("name"))
                .or_else(|| value.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("tool"),
        );
        let raw_args = value
            .get("function")
            .and_then(|function| function.get("arguments"))
            .or_else(|| value.get("arguments"))
            .or_else(|| value.get("args"))
            .or_else(|| value.get("input"))
            .map(value_to_display)
            .unwrap_or_default();
        let mut attached_files = string_array_field(value, "attached_files");
        let attached_files_truncated = attached_files.len() > MAX_SUBCHAT_ATTACHED_FILES;
        attached_files.truncate(MAX_SUBCHAT_ATTACHED_FILES);
        Self {
            id,
            name,
            args: raw_args.clone(),
            args_preview: compact_preview(&raw_args, 96),
            result: String::new(),
            status: ToolStatus::Running,
            duration_ms: None,
            started_at_ms: now_ms(),
            expanded: false,
            subchat_log: subchat_log_from_value(value),
            attached_files,
            subchat_depth: value
                .get("subchat_depth")
                .or_else(|| value.get("depth"))
                .and_then(Value::as_u64)
                .map(|depth| (depth as usize).clamp(1, MAX_SUBCHAT_DEPTH))
                .unwrap_or(1),
            subchat_active: value
                .get("subchat")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty()),
            subchat_updates: value
                .get("subchat_updates")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
            subchat_truncated: value
                .get("subchat_truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || attached_files_truncated,
        }
    }

    pub fn with_result(mut self, result: impl Into<String>, status: ToolStatus) -> Self {
        self.result = sanitize_tool_text(result.into());
        self.status = status;
        self
    }

    pub fn set_result(&mut self, result: impl AsRef<str>) {
        self.result = sanitize_tool_text(result);
    }

    pub fn update_from_tool_call(&mut self, update: ToolCard) {
        self.name = update.name;
        self.args = update.args;
        self.args_preview = update.args_preview;
        if !update.subchat_log.is_empty() {
            self.subchat_log = update.subchat_log;
        }
        if !update.attached_files.is_empty() {
            self.attached_files = update.attached_files;
        }
    }

    pub fn clear_subchat(&mut self) {
        self.subchat_log.clear();
        self.attached_files.clear();
        self.subchat_active = false;
        self.subchat_truncated = false;
        self.subchat_updates = 0;
        self.subchat_depth = 1;
    }

    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }

    pub fn summary(&self) -> String {
        let duration = self
            .duration_ms
            .map(format_duration)
            .unwrap_or_else(|| "".to_string());
        if duration.is_empty() {
            format!(
                "{} {}({})",
                self.status.icon(),
                self.name,
                self.args_preview
            )
        } else {
            format!(
                "{} {}({}) · {}",
                self.status.icon(),
                self.name,
                self.args_preview,
                duration
            )
        }
    }

    pub fn render_lines(&self, width: usize) -> Vec<Line<'static>> {
        let marker = if self.expanded { "▾" } else { "▸" };
        let mut lines = vec![Line::from(vec![
            Span::styled(marker, Style::default().fg(Color::Cyan)),
            Span::raw(" "),
            Span::styled(self.summary(), Style::default().fg(Color::Yellow)),
        ])];
        lines.extend(self.render_subchat_lines(width));
        if self.expanded {
            lines.extend(render_tool_result(&self.result, width));
        }
        lines
    }

    pub fn render_subchat_lines(&self, width: usize) -> Vec<Line<'static>> {
        if self.subchat_log.is_empty()
            && self.attached_files.is_empty()
            && self.subchat_updates == 0
        {
            return Vec::new();
        }
        let state = if self.subchat_active {
            "active"
        } else {
            "recent"
        };
        let mut meta = vec![format!("subagent {state}")];
        if self.subchat_depth > 1 {
            meta.push(format!("depth {}", self.subchat_depth));
        }
        if self.subchat_updates > 0 {
            meta.push(format!("{} updates", self.subchat_updates));
        }
        if !self.attached_files.is_empty() {
            meta.push(format!("{} files", self.attached_files.len()));
        }
        if self.subchat_truncated {
            meta.push("truncated".to_string());
        }
        let mut lines = vec![Line::from(vec![
            Span::styled("  ↳ ", Style::default().fg(Color::Magenta)),
            Span::styled(meta.join(" · "), Style::default().fg(Color::Magenta)),
        ])];
        if self.expanded {
            let latest = self.subchat_log.last().cloned().unwrap_or_default();
            if !latest.is_empty() {
                lines.extend(subchat_output_lines(&latest, width, EXPANDED_SUBCHAT_LINES));
            }
            if !self.attached_files.is_empty() {
                let shown = self.attached_files.len().min(MAX_SUBCHAT_ATTACHED_FILES);
                for file in self.attached_files.iter().take(shown) {
                    lines.push(Line::from(vec![
                        Span::styled("    file ", Style::default().fg(Color::DarkGray)),
                        Span::raw(compact_preview(file, width.saturating_sub(10).max(8))),
                    ]));
                }
            }
        } else if let Some(latest) = self.subchat_log.last() {
            lines.extend(subchat_output_lines(latest, width, COLLAPSED_SUBCHAT_LINES));
        }
        lines
    }
}

fn subchat_output_lines(text: &str, width: usize, max_lines: usize) -> Vec<Line<'static>> {
    let all_lines = text.lines().collect::<Vec<_>>();
    let source = if all_lines.is_empty() {
        vec![text]
    } else {
        all_lines
    };
    let shown = source.len().min(max_lines);
    let mut lines = source
        .iter()
        .take(shown)
        .map(|line| {
            Line::from(vec![
                Span::styled("    ", Style::default().fg(Color::DarkGray)),
                Span::raw(compact_preview(line, width.saturating_sub(6).max(8))),
            ])
        })
        .collect::<Vec<_>>();
    if source.len() > shown {
        lines.push(Line::from(Span::styled(
            format!("    … {} more subagent lines", source.len() - shown),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn subchat_log_from_value(value: &Value) -> Vec<String> {
    let mut log = string_array_field(value, "subchat_log");
    if log.is_empty() {
        if let Some(subchat) = value
            .get("subchat")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && !value.contains("/tool:"))
        {
            log.push(sanitize_tool_text(subchat));
        }
    }
    log.truncate(1);
    log
}

fn string_array_field(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(sanitize_tool_text)
                .collect()
        })
        .unwrap_or_default()
}

pub fn render_tool_result(result: &str, width: usize) -> Vec<Line<'static>> {
    let result = sanitize_tool_text(result);
    if is_unified_diff(&result) {
        return render_unified_diff(
            &result,
            Some(width.saturating_sub(2).max(8)),
            color_enabled_from_env(),
        )
        .into_iter()
        .take(MAX_RESULT_LINES + 1)
        .collect();
    }

    let mut lines = Vec::new();
    let all_lines = result.lines().collect::<Vec<_>>();
    let shown = all_lines.len().min(MAX_RESULT_LINES);
    for line in all_lines.iter().take(shown) {
        lines.push(Line::from(Span::styled(
            compact_preview(line, width.saturating_sub(4).max(8)),
            style_for_result_line(line),
        )));
    }
    if all_lines.len() > MAX_RESULT_LINES {
        lines.push(Line::from(Span::styled(
            format!("… {} more", all_lines.len() - MAX_RESULT_LINES),
            Style::default().fg(Color::DarkGray),
        )));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no output)",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

pub fn style_for_result_line(line: &str) -> Style {
    if line.starts_with('+') && !line.starts_with("+++") {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') && !line.starts_with("---") {
        Style::default().fg(Color::Red)
    } else if line.starts_with("@@") {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn value_to_display(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        if let Ok(parsed) = serde_json::from_str::<Value>(text) {
            return value_to_display(&parsed);
        }
        return sanitize_tool_text(text);
    }
    let sanitized = sanitize_json_strings(value);
    match sanitized {
        Value::String(value) => value,
        value => serde_json::to_string(&value).unwrap_or_else(|_| value.to_string()),
    }
}

fn compact_preview(value: &str, max_chars: usize) -> String {
    compact_tool_preview(value, max_chars)
}

fn format_duration(duration_ms: u64) -> String {
    if duration_ms < 1000 {
        format!("{duration_ms}ms")
    } else {
        format!("{:.1}s", duration_ms as f64 / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn card_collapse_expand_toggles_result_lines() {
        let mut card = ToolCard::from_tool_call(&json!({
            "id": "call-1",
            "function": {"name": "shell", "arguments": "{\"cmd\":\"echo hi\"}"}
        }))
        .with_result("+ok\n-no", ToolStatus::Success);
        assert_eq!(card.render_lines(80).len(), 1);
        card.toggle();
        let lines = card.render_lines(80);
        assert!(lines.len() > 1);
        assert!(format!("{:?}", lines).contains("+ok"));
    }

    #[test]
    fn subchat_progress_renders_collapsed_and_expanded() {
        let mut card = ToolCard::from_tool_call(&json!({
            "id": "call-1",
            "function": {"name": "tool_subagent", "arguments": "{}"},
            "subchat_log": ["one\ntwo\nthree"],
            "attached_files": ["src/lib.rs"],
            "subchat_updates": 2,
            "subchat_depth": 2
        }));
        let collapsed = format!("{:?}", card.render_subchat_lines(80));
        assert!(collapsed.contains("subagent"));
        assert!(collapsed.contains("one"));
        assert!(collapsed.contains("more subagent lines"));
        assert!(!collapsed.contains("src/lib.rs"));

        card.toggle();
        let expanded = format!("{:?}", card.render_subchat_lines(80));
        assert!(expanded.contains("src/lib.rs"));
        assert!(expanded.contains("three"));
    }

    #[test]
    fn result_truncates_after_limit() {
        let result = (0..205)
            .map(|idx| format!("line {idx}"))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = render_tool_result(&result, 80);
        assert_eq!(lines.len(), 201);
        assert!(format!("{:?}", lines.last().unwrap()).contains("5 more"));
    }

    #[test]
    fn tool_result_escape_sequences_render_inert() {
        let lines = render_tool_result("ok\x1b]0;pwned\x07\x1b[2Jdone", 80);
        let rendered = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(!rendered.contains('\x1b'));
        assert!(!rendered.contains('\x07'));
        assert!(!rendered.contains("pwned"));
        assert!(rendered.contains("okdone"));
    }

    #[test]
    fn tool_call_args_escape_sequences_render_inert() {
        let card = ToolCard::from_tool_call(&json!({
            "id": "call-1",
            "function": {
                "name": "shell\x1b[31m",
                "arguments": r#"{"command":"echo \u001b]0;pwned\u0007\u001b[2Jdone"}"#
            }
        }));

        let rendered = card.summary();

        assert!(!rendered.contains('\x1b'));
        assert!(!rendered.contains('\x07'));
        assert!(!rendered.contains("pwned"));
        assert!(rendered.contains("shell"));
        assert!(rendered.contains("echo done"));
    }

    #[test]
    fn compact_preview_truncates_on_grapheme_boundary() {
        let family = "👨‍👩‍👧‍👦";
        let preview = compact_preview(&format!("ab{family}cd"), 3);
        assert_eq!(preview, format!("ab{family}…"));
    }

    #[test]
    fn unified_diff_result_uses_diff_renderer() {
        let lines = render_tool_result("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new", 80);
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(rendered.contains(&"- old".to_string()));
        assert!(rendered.contains(&"+ new".to_string()));
    }
}
