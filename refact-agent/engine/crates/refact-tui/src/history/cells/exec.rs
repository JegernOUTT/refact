use super::*;
use crate::text_formatting::truncate_text;
use crate::text_safety::sanitize_tool_text;

const TOOL_CALL_MAX_LINES: usize = 5;
const USER_SHELL_TOOL_CALL_MAX_LINES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolCallCell {
    card: ToolCard,
    selected: bool,
}

impl ToolCallCell {
    pub fn new(card: ToolCard, selected: bool) -> Self {
        Self { card, selected }
    }

    pub fn card(&self) -> &ToolCard {
        &self.card
    }
}

impl HistoryCell for ToolCallCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Tool
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let label = if self.selected {
            "tool selected"
        } else {
            "tool"
        };
        let mut lines = vec![role_line(
            label,
            Style::default().fg(if self.selected {
                Color::Cyan
            } else {
                Color::Yellow
            }),
        )];
        lines.extend(self.card.render_lines(width));
        finish(lines)
    }

    fn is_final(&self) -> bool {
        self.card.status != ToolStatus::Running
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.card, self.selected))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubchatCell {
    card: ToolCard,
}

impl SubchatCell {
    pub fn new(card: ToolCard) -> Self {
        Self { card }
    }

    pub fn render_inline(&self, width: usize) -> Vec<Line<'static>> {
        self.card.render_subchat_lines(width)
    }
}

impl HistoryCell for SubchatCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Subchat
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        finish(self.render_inline(width))
    }

    fn is_final(&self) -> bool {
        !self.card.subchat_active
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.card))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecToolCell {
    card: ToolCard,
    selected: bool,
}

impl ExecToolCell {
    pub fn new(card: ToolCard, selected: bool) -> Self {
        Self { card, selected }
    }
}

impl HistoryCell for ExecToolCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Exec
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = vec![role_line(
            if self.selected {
                "exec selected"
            } else {
                "exec"
            },
            Style::default().fg(Color::Cyan),
        )];
        let mut meta = Vec::new();
        if let Some(exit_code) = exit_code_from_result(&self.card.result) {
            meta.push(format!("exit {exit_code}"));
        }
        if let Some(duration_ms) = self.card.duration_ms {
            meta.push(format_duration(duration_ms));
        }
        lines.push(tool_summary_line(
            &self.card,
            exec_command_label(&self.card),
            meta.join(" · "),
        ));
        lines.extend(subchat_lines(&self.card, width));
        if !self.card.result.is_empty() {
            lines.extend(exec_output_lines(&self.card, width));
        }
        finish(lines)
    }

    fn is_final(&self) -> bool {
        self.card.status != ToolStatus::Running
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.card, self.selected))
    }
}

fn exec_command_label(card: &ToolCard) -> String {
    argument_value(card, &["command", "cmd"])
        .map(|command| format!("$ {}", display_shell_command(&command)))
        .unwrap_or_else(|| command_label(card))
}

fn display_shell_command(command: &str) -> String {
    let parts = match shell_words::split(command) {
        Ok(parts) => parts,
        Err(_) => return command.to_string(),
    };
    if parts.len() >= 3
        && matches!(parts.first().map(String::as_str), Some("bash" | "sh"))
        && parts.get(1).is_some_and(|arg| arg == "-lc" || arg == "-c")
    {
        parts[2].clone()
    } else {
        command.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecOutputLine {
    text: String,
    stderr: bool,
}

fn exec_output_lines(card: &ToolCard, width: usize) -> Vec<Line<'static>> {
    let source = collect_exec_output_lines(&card.result);
    let failed = failed_exit(card);
    let only_err = failed && source.iter().any(|line| line.stderr);
    let max_lines = if card.name == "shell" {
        USER_SHELL_TOOL_CALL_MAX_LINES
    } else {
        TOOL_CALL_MAX_LINES
    };

    if only_err {
        return render_tail_lines(
            source.iter().filter(|line| line.stderr).cloned().collect(),
            width,
            max_lines,
        );
    }

    render_head_tail_lines(source, width, max_lines)
}

fn failed_exit(card: &ToolCard) -> bool {
    card.status == ToolStatus::Error
        || exit_code_from_result(&card.result)
            .is_some_and(|code| code.trim() != "0" && code.trim() != "<none>")
}

fn collect_exec_output_lines(result: &str) -> Vec<ExecOutputLine> {
    let sanitized = sanitize_tool_text(result);
    let mut lines = Vec::new();
    let mut stderr = false;
    let mut in_fence = false;

    for raw in sanitized.lines() {
        let trimmed = raw.trim();
        let lower = trimmed.to_ascii_lowercase();
        if is_status_line(trimmed) {
            continue;
        }
        if lower == "stdout" || lower == "stdout:" || lower == "output" || lower == "output:" {
            stderr = false;
            in_fence = false;
            continue;
        }
        if lower == "stderr" || lower == "stderr:" {
            stderr = true;
            in_fence = false;
            continue;
        }
        if trimmed == "```" {
            in_fence = !in_fence;
            continue;
        }
        if let Some(text) = inline_section_line(raw, "stdout:") {
            if !text.is_empty() {
                lines.push(ExecOutputLine {
                    text,
                    stderr: false,
                });
            }
            stderr = false;
            continue;
        }
        if let Some(text) = inline_section_line(raw, "output:") {
            if !text.is_empty() {
                lines.push(ExecOutputLine {
                    text,
                    stderr: false,
                });
            }
            stderr = false;
            continue;
        }
        if let Some(text) = inline_section_line(raw, "stderr:") {
            if !text.is_empty() {
                lines.push(ExecOutputLine { text, stderr: true });
            }
            stderr = true;
            continue;
        }

        let mut text = raw.to_string();
        if let Some(before) = text.strip_suffix("```") {
            text = before.to_string();
            in_fence = false;
        }
        if text.is_empty() && !in_fence {
            continue;
        }
        lines.push(ExecOutputLine { text, stderr });
    }

    lines
}

fn inline_section_line(raw: &str, prefix: &str) -> Option<String> {
    let trimmed = raw.trim_start();
    let head = trimmed.get(..prefix.len())?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let rest = trimmed[prefix.len()..].trim_start();
    (!rest.is_empty()).then(|| rest.trim_end_matches("```").to_string())
}

fn is_status_line(line: &str) -> bool {
    line.starts_with("The command was running ")
        || line.starts_with("⚠️ The command timed out ")
        || line.starts_with("⚠️ The command failed ")
        || line.starts_with("⚠️ The command was interrupted ")
        || line.starts_with("⚠️ The command did not reach ")
}

fn render_tail_lines(
    source: Vec<ExecOutputLine>,
    width: usize,
    max_lines: usize,
) -> Vec<Line<'static>> {
    if source.is_empty() {
        return Vec::new();
    }
    let omitted = source.len().saturating_sub(max_lines);
    let mut lines = Vec::new();
    if omitted > 0 {
        lines.push(tree_line(
            true,
            format!("… +{omitted} lines"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
            width,
        ));
    }
    let tail_start = source.len().saturating_sub(max_lines);
    for line in source.into_iter().skip(tail_start) {
        lines.push(tree_line(
            lines.is_empty(),
            line.text,
            output_line_style(line.stderr),
            width,
        ));
    }
    lines
}

fn render_head_tail_lines(
    source: Vec<ExecOutputLine>,
    width: usize,
    max_lines: usize,
) -> Vec<Line<'static>> {
    if source.is_empty() {
        return vec![tree_line(
            true,
            "(no output)".to_string(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
            width,
        )];
    }

    let total = source.len();
    let mut lines = Vec::new();
    let head_end = total.min(max_lines);
    for line in source.iter().take(head_end) {
        lines.push(tree_line(
            lines.is_empty(),
            line.text.clone(),
            output_line_style(line.stderr),
            width,
        ));
    }

    if total > max_lines * 2 {
        let omitted = total - max_lines * 2;
        lines.push(tree_line(
            false,
            format!("… +{omitted} lines"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
            width,
        ));
        for line in source.iter().skip(total - max_lines) {
            lines.push(tree_line(
                false,
                line.text.clone(),
                output_line_style(line.stderr),
                width,
            ));
        }
    } else {
        for line in source.iter().skip(head_end) {
            lines.push(tree_line(
                false,
                line.text.clone(),
                output_line_style(line.stderr),
                width,
            ));
        }
    }

    lines
}

fn tree_line(first: bool, text: String, style: Style, width: usize) -> Line<'static> {
    let prefix = if first { "  └ " } else { "    " };
    let text_width = width.saturating_sub(4).max(8);
    Line::from(vec![
        Span::styled(
            prefix,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
        Span::styled(truncate_text(&text, text_width), style),
    ])
}

fn output_line_style(stderr: bool) -> Style {
    let color = if stderr { Color::Red } else { Color::DarkGray };
    Style::default().fg(color).add_modifier(Modifier::DIM)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::{text, tool_card};
    use serde_json::json;

    #[test]
    fn unknown_tool_cell_keeps_generic_card() {
        let card = tool_card("totally_unknown", json!({"x": 1}), "line 1");
        let cell = ToolCallCell::new(card, true);
        assert!(cell.is_final());
        assert!(text(&cell.render(80)).contains("tool selected\n▾ ✅ totally_unknown"));
        assert!(text(&cell.render(80)).contains("line 1"));
    }

    #[test]
    fn exec_cell_snapshot_extracts_command_exit_and_output() {
        let card = tool_card(
            "shell",
            json!({"command": "echo hi"}),
            "stdout:\nhi\n\nThe command was running 0.120s, finished with exit code 0",
        );
        let cell = cell_from_tool_card(card, true);
        assert_eq!(cell.kind(), HistoryCellKind::Exec);
        let rendered = text(&cell.render(80));
        assert!(rendered.contains("exec selected\n▾ ✅ $ echo hi · exit 0 · 1.2s"));
        assert!(rendered.contains("  └ hi"));
        assert!(!rendered.contains("The command was running"));
    }

    #[test]
    fn exec_cell_strips_bash_lc_command_wrapper() {
        let card = tool_card(
            "shell",
            json!({"command": "bash -lc 'printf hi'"}),
            "hi\n\nThe command was running 0.120s, finished with exit code 0",
        );
        let rendered = text(&ExecToolCell::new(card, false).render(80));
        assert!(rendered.contains("$ printf hi · exit 0"));
        assert!(!rendered.contains("bash -lc"));
    }

    #[test]
    fn exec_cell_truncates_output_with_tree_ellipsis_and_tail() {
        let mut card = tool_card(
            "process_read",
            json!({"process_id": "exec_1"}),
            &(0..15)
                .map(|idx| format!("line {idx}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        card.expanded = false;
        let rendered = text(&ExecToolCell::new(card, false).render(80));
        assert!(rendered.contains("  └ line 0"));
        assert!(rendered.contains("    … +5 lines"));
        assert!(rendered.contains("    line 14"));
        assert!(!rendered.contains("line 5"));
    }

    #[test]
    fn failed_exec_cell_collapsed_output_prefers_stderr_tail() {
        let stderr = (0..60)
            .map(|idx| format!("err {idx}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = format!(
            "STDOUT\n```\nbuild ok\n```\n\nSTDERR\n```\n{stderr}\n```\n\nThe command was running 0.120s, finished with exit code 2"
        );
        let mut card = tool_card("shell", json!({"command": "cargo test"}), &result);
        card.status = ToolStatus::Error;
        card.expanded = false;
        let rendered_lines = ExecToolCell::new(card, false).render(80);
        let rendered = text(&rendered_lines);
        assert!(rendered.contains("▸ ❌ $ cargo test · exit 2 · 1.2s"));
        assert!(rendered.contains("  └ … +10 lines"));
        assert!(rendered.contains("    err 59"));
        assert!(!rendered.contains("build ok"));
        assert!(!rendered.contains("err 0"));
        let err_line = rendered_lines
            .iter()
            .find(|line| line.spans.iter().any(|span| span.content == "err 59"))
            .unwrap();
        assert_eq!(err_line.spans[1].style.fg, Some(Color::Red));
    }

    #[test]
    fn exec_cell_output_escape_sequences_render_inert() {
        let card = tool_card(
            "shell",
            json!({"command": "echo hi"}),
            "ok\x1b]0;pwned\x07\x1b[2Jdone\n\nThe command was running 0.120s, finished with exit code 0",
        );
        let rendered = text(&ExecToolCell::new(card, false).render(80));
        assert!(!rendered.contains('\x1b'));
        assert!(!rendered.contains('\x07'));
        assert!(!rendered.contains("pwned"));
        assert!(rendered.contains("okdone"));
    }
}
