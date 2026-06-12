use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::render::{color_enabled_from_env, is_unified_diff, render_unified_diff};

const MAX_RESULT_LINES: usize = 200;

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
    pub args_preview: String,
    pub result: String,
    pub status: ToolStatus,
    pub duration_ms: Option<u64>,
    pub started_at_ms: u64,
    pub expanded: bool,
}

impl ToolCard {
    pub fn from_tool_call(value: &Value) -> Self {
        let id = value
            .get("id")
            .or_else(|| value.get("tool_call_id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let name = value
            .get("function")
            .and_then(|function| function.get("name"))
            .or_else(|| value.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_string();
        let raw_args = value
            .get("function")
            .and_then(|function| function.get("arguments"))
            .or_else(|| value.get("arguments"))
            .or_else(|| value.get("args"))
            .or_else(|| value.get("input"))
            .map(value_to_display)
            .unwrap_or_default();
        Self {
            id,
            name,
            args_preview: compact_preview(&raw_args, 96),
            result: String::new(),
            status: ToolStatus::Running,
            duration_ms: None,
            started_at_ms: now_ms(),
            expanded: false,
        }
    }

    pub fn with_result(mut self, result: impl Into<String>, status: ToolStatus) -> Self {
        self.result = result.into();
        self.status = status;
        self
    }

    pub fn update_from_tool_call(&mut self, update: ToolCard) {
        self.name = update.name;
        self.args_preview = update.args_preview;
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
        if self.expanded {
            lines.extend(render_tool_result(&self.result, width));
        }
        lines
    }
}

pub fn render_tool_result(result: &str, width: usize) -> Vec<Line<'static>> {
    if is_unified_diff(result) {
        return render_unified_diff(
            result,
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
    match value {
        Value::String(value) => value.clone(),
        value => serde_json::to_string(value).unwrap_or_else(|_| value.to_string()),
    }
}

fn compact_preview(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut out = compact.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
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
