use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

use crate::render::MarkdownRenderer;
use crate::history::cells::tool_display_name;
use crate::text_safety::{
    compact_tool_preview, sanitize_json_strings, sanitize_tool_inline, sanitize_tool_text,
};

const MAX_RESULT_LINES: usize = 200;
pub const MAX_SUBCHAT_DEPTH: usize = 5;
pub const MAX_SUBCHAT_ATTACHED_FILES: usize = 12;
pub const MAX_SUBCHAT_PROGRESS_CHARS: usize = 2000;
const COLLAPSED_SUBCHAT_LINES: usize = 2;
const EXPANDED_SUBCHAT_LINES: usize = 8;
const SUBCHAT_TREE_INITIAL: &str = "  └ ";
const SUBCHAT_TREE_CONTINUATION: &str = "    ";
const SUBCHAT_TREE_WIDTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolStatus {
    Queued,
    AwaitingApproval,
    ApprovedOnce,
    ApprovedForChat,
    Running,
    Succeeded,
    Failed,
    Denied,
    Cancelled,
}

impl ToolStatus {
    pub const ALL: [Self; 9] = [
        Self::Queued,
        Self::AwaitingApproval,
        Self::ApprovedOnce,
        Self::ApprovedForChat,
        Self::Running,
        Self::Succeeded,
        Self::Failed,
        Self::Denied,
        Self::Cancelled,
    ];

    pub fn icon(self) -> &'static str {
        match self {
            Self::Queued => "○",
            Self::AwaitingApproval => "?",
            Self::ApprovedOnce => "✓",
            Self::ApprovedForChat => "✓✓",
            Self::Running => "⏳",
            Self::Succeeded => "✅",
            Self::Failed => "❌",
            Self::Denied => "⊘",
            Self::Cancelled => "⊗",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::AwaitingApproval => "awaiting approval",
            Self::ApprovedOnce => "approved once",
            Self::ApprovedForChat => "approved for chat",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Denied => "denied",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn visual(self) -> String {
        format!("{} {}", self.icon(), self.label())
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::AwaitingApproval
                | Self::ApprovedOnce
                | Self::ApprovedForChat
                | Self::Running
        )
    }

    pub fn is_final(self) -> bool {
        !self.is_active()
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
    exit_code: Option<Option<i32>>,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
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
        let started_at_ms = timestamp_field(value, "started_at_ms");
        let completed_at_ms = timestamp_field(value, "completed_at_ms");
        Self {
            id,
            name,
            args: raw_args.clone(),
            args_preview: compact_preview(&raw_args, 96),
            result: String::new(),
            status: tool_status_from_value(value),
            duration_ms: structured_duration_ms(value)
                .flatten()
                .or_else(|| duration_from_timestamps(started_at_ms, completed_at_ms)),
            exit_code: structured_exit_code(value),
            started_at_ms,
            completed_at_ms,
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

    pub fn apply_result_metadata(&mut self, extra: &serde_json::Map<String, Value>) {
        if let Some(started_at_ms) = timestamp_field_from_extra(extra, "started_at_ms") {
            self.started_at_ms = Some(started_at_ms);
        }
        if let Some(completed_at_ms) = timestamp_field_from_extra(extra, "completed_at_ms") {
            self.completed_at_ms = Some(completed_at_ms);
        }
        if let Some(duration_ms) = structured_duration_ms_from_extra(extra) {
            self.duration_ms = duration_ms;
        } else if self.duration_ms.is_none() {
            self.duration_ms = duration_from_timestamps(self.started_at_ms, self.completed_at_ms);
        }
        if let Some(exit_code) = structured_exit_code_from_extra(extra) {
            self.exit_code = Some(exit_code);
        }
    }

    pub fn reported_exit_code(&self) -> Option<Option<i32>> {
        self.exit_code
    }

    pub fn reported_duration_ms(&self) -> Option<u64> {
        self.duration_ms.filter(|duration_ms| *duration_ms > 0)
    }

    pub fn update_from_tool_call(&mut self, update: ToolCard) {
        self.name = update.name;
        self.args = update.args;
        self.args_preview = update.args_preview;
        if update.duration_ms.is_some() {
            self.duration_ms = update.duration_ms;
        }
        if update.started_at_ms.is_some() {
            self.started_at_ms = update.started_at_ms;
        }
        if update.completed_at_ms.is_some() {
            self.completed_at_ms = update.completed_at_ms;
        }
        if update.exit_code.is_some() {
            self.exit_code = update.exit_code;
        }
        self.subchat_log = update.subchat_log;
        self.attached_files = update.attached_files;
        self.subchat_depth = update.subchat_depth;
        self.subchat_active = update.subchat_active;
        self.subchat_updates = update.subchat_updates;
        self.subchat_truncated = update.subchat_truncated;
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
            .reported_duration_ms()
            .map(format_duration)
            .unwrap_or_default();
        let mut parts = vec![format!(
            "{} {}",
            self.status.visual(),
            tool_display_name(&self.name)
        )];
        if let Some(summary) = tool_argument_summary(&self.args, 72) {
            parts.push(summary);
        }
        if !duration.is_empty() {
            parts.push(duration);
        }
        parts.join(" · ")
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
            lines.push(tool_section_label("Arguments"));
            lines.extend(render_tool_arguments(&self.args, width));
            lines.push(tool_section_label("Result"));
            lines.extend(render_tool_result(&self.result, width));
        } else if !self.result.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  └ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    tool_result_summary(&self.result, width.saturating_sub(4).max(8)),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
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
        let header_style = subchat_header_style();
        let mut lines = vec![Line::from(vec![
            Span::styled("  ↳ ", header_style),
            Span::styled(meta.join(" · "), header_style),
        ])];
        let mut body = Vec::new();
        if self.expanded {
            let latest = self.subchat_log.last().cloned().unwrap_or_default();
            if !latest.is_empty() {
                body.extend(subchat_output_lines(&latest, width, EXPANDED_SUBCHAT_LINES));
            }
            if let Some(line) = subchat_files_line(&self.attached_files, width) {
                body.push(line);
            }
        } else if let Some(latest) = self.subchat_log.last() {
            body.extend(subchat_output_lines(latest, width, COLLAPSED_SUBCHAT_LINES));
        } else if let Some(line) = subchat_files_line(&self.attached_files, width) {
            body.push(line);
        }
        lines.extend(prefix_subchat_body(body));
        lines
    }
}

fn tool_section_label(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        label.to_string(),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    ))
}

fn subchat_output_lines(text: &str, width: usize, max_lines: usize) -> Vec<Line<'static>> {
    let all_lines = text.lines().collect::<Vec<_>>();
    let source = if all_lines.is_empty() {
        vec![text]
    } else {
        all_lines
    };
    let shown = source.len().min(max_lines);
    let text_width = width.saturating_sub(SUBCHAT_TREE_WIDTH).max(8);
    let mut lines = source
        .iter()
        .take(shown)
        .map(|line| {
            Line::from(Span::styled(
                compact_preview(line, text_width),
                subchat_detail_style(),
            ))
        })
        .collect::<Vec<_>>();
    if source.len() > shown {
        lines.push(Line::from(Span::styled(
            format!("… +{} lines", source.len() - shown),
            subchat_detail_style(),
        )));
    }
    lines
}

fn subchat_files_line(files: &[String], width: usize) -> Option<Line<'static>> {
    let mut names = Vec::<&str>::new();
    for file in files.iter().take(MAX_SUBCHAT_ATTACHED_FILES) {
        let name = file.as_str();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        return None;
    }
    let joined = names.join(", ");
    let text_width = width
        .saturating_sub(SUBCHAT_TREE_WIDTH)
        .saturating_sub("Read ".len())
        .max(8);
    Some(Line::from(vec![
        Span::styled("Read ", subchat_action_style()),
        Span::styled(compact_preview(&joined, text_width), subchat_detail_style()),
    ]))
}

fn prefix_subchat_body(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let mut spans = vec![Span::styled(
                if index == 0 {
                    SUBCHAT_TREE_INITIAL
                } else {
                    SUBCHAT_TREE_CONTINUATION
                },
                subchat_detail_style(),
            )];
            spans.extend(line.spans);
            Line {
                spans,
                style: line.style,
                alignment: line.alignment,
            }
        })
        .collect()
}

fn subchat_header_style() -> Style {
    Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::DIM)
}

fn subchat_action_style() -> Style {
    Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM)
}

fn subchat_detail_style() -> Style {
    Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::DIM)
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
    log.pop().into_iter().collect()
}

fn tool_status_from_value(value: &Value) -> ToolStatus {
    let status = value
        .get("status")
        .or_else(|| value.get("state"))
        .and_then(Value::as_str)
        .map(|status| status.to_ascii_lowercase().replace(['_', '-'], " "));
    match status.as_deref() {
        Some("queued") | Some("pending") => ToolStatus::Queued,
        Some("awaiting approval") | Some("approval required") => ToolStatus::AwaitingApproval,
        Some("approved once") => ToolStatus::ApprovedOnce,
        Some("approved for chat") => ToolStatus::ApprovedForChat,
        Some("succeeded") | Some("success") | Some("completed") => ToolStatus::Succeeded,
        Some("failed") | Some("error") => ToolStatus::Failed,
        Some("denied") | Some("rejected") => ToolStatus::Denied,
        Some("cancelled") | Some("canceled") | Some("aborted") => ToolStatus::Cancelled,
        _ => ToolStatus::Running,
    }
}

fn timestamp_field(value: &Value, key: &str) -> Option<u64> {
    structured_field(value, &[key]).and_then(Value::as_u64)
}

fn timestamp_field_from_extra(extra: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    structured_field_from_extra(extra, &[key]).and_then(Value::as_u64)
}

fn duration_from_timestamps(
    started_at_ms: Option<u64>,
    completed_at_ms: Option<u64>,
) -> Option<u64> {
    completed_at_ms
        .zip(started_at_ms)
        .and_then(|(end, start)| end.checked_sub(start))
}

fn structured_duration_ms(value: &Value) -> Option<Option<u64>> {
    structured_field(value, &["duration_ms"]).map(Value::as_u64)
}

fn structured_exit_code(value: &Value) -> Option<Option<i32>> {
    structured_field(value, &["exit_code", "exitCode"])
        .map(|value| value.as_u64().and_then(|code| i32::try_from(code).ok()))
}

fn structured_duration_ms_from_extra(
    extra: &serde_json::Map<String, Value>,
) -> Option<Option<u64>> {
    structured_field_from_extra(extra, &["duration_ms"]).map(Value::as_u64)
}

fn structured_exit_code_from_extra(extra: &serde_json::Map<String, Value>) -> Option<Option<i32>> {
    structured_field_from_extra(extra, &["exit_code", "exitCode"])
        .map(|value| value.as_u64().and_then(|code| i32::try_from(code).ok()))
}

fn structured_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    value
        .get("exec")
        .and_then(|exec| field(exec, keys))
        .or_else(|| field(value, keys))
}

fn structured_field_from_extra<'a>(
    extra: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a Value> {
    extra
        .get("exec")
        .and_then(|exec| field(exec, keys))
        .or_else(|| keys.iter().find_map(|key| extra.get(*key)))
}

fn field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| value.get(*key))
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
    let source = format_tool_payload(result);
    let mut lines = if source.is_empty() {
        Vec::new()
    } else {
        MarkdownRenderer::new(Some(width.max(8))).render(&source)
    };
    let omitted = lines.len().saturating_sub(MAX_RESULT_LINES);
    lines.truncate(MAX_RESULT_LINES);
    if omitted > 0 {
        lines.push(Line::from(Span::styled(
            format!("… {omitted} more"),
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

pub fn render_tool_arguments(args: &str, width: usize) -> Vec<Line<'static>> {
    let source = format_tool_payload(args);
    if source.is_empty() {
        return vec![Line::from(Span::styled(
            "(no arguments)",
            Style::default().fg(Color::DarkGray),
        ))];
    }
    MarkdownRenderer::new(Some(width.max(8))).render(&source)
}

pub fn tool_argument_summary(args: &str, max_graphemes: usize) -> Option<String> {
    let value = serde_json::from_str::<Value>(args).ok();
    let summary = match value {
        Some(Value::Object(values)) => {
            const SALIENT_KEYS: &[&str] = &[
                "description",
                "summary",
                "title",
                "command",
                "cmd",
                "query",
                "search_key",
                "pattern",
                "path",
                "url",
                "message",
                "prompt",
                "question",
                "name",
                "id",
            ];
            SALIENT_KEYS
                .iter()
                .find_map(|key| {
                    values
                        .get(*key)
                        .map(|value| format!("{key}: {}", summary_value(value)))
                })
                .or_else(|| {
                    values.iter().find_map(|(key, value)| {
                        let value = summary_value(value);
                        (!value.is_empty()).then(|| format!("{key}: {value}"))
                    })
                })
                .or_else(|| (!values.is_empty()).then(|| format!("{} arguments", values.len())))
        }
        Some(value) => Some(summary_value(&value)),
        None => {
            let args = sanitize_tool_inline(args);
            if args.starts_with('{') || args.starts_with('[') {
                Some("arguments unavailable".to_string())
            } else {
                (!args.is_empty()).then_some(args)
            }
        }
    }?;
    Some(truncate_summary_at_boundary(&summary, max_graphemes))
}

pub fn tool_result_summary(result: &str, max_graphemes: usize) -> String {
    let source = sanitize_tool_text(result);
    let summary = serde_json::from_str::<Value>(&source)
        .ok()
        .and_then(|value| match value {
            Value::Object(values) => ["summary", "message", "detail", "output"]
                .iter()
                .find_map(|key| values.get(*key).map(summary_value))
                .or_else(|| (!values.is_empty()).then(|| "Result available".to_string())),
            Value::String(value) => Some(sanitize_tool_inline(value)),
            value => Some(summary_value(&value)),
        })
        .or_else(|| {
            source
                .lines()
                .map(sanitize_tool_inline)
                .find(|line| !line.is_empty())
        })
        .unwrap_or_else(|| "(no output)".to_string());
    truncate_summary_at_boundary(&summary, max_graphemes)
}

fn format_tool_payload(text: &str) -> String {
    let text = sanitize_tool_text(text);
    serde_json::from_str::<Value>(&text)
        .ok()
        .map(|value| sanitize_json_strings(&value))
        .and_then(|value| {
            serde_json::to_string_pretty(&value)
                .ok()
                .map(|value| format!("```json\n{value}\n```"))
        })
        .unwrap_or(text)
}

fn summary_value(value: &Value) -> String {
    match sanitize_json_strings(value) {
        Value::String(value) => sanitize_tool_inline(value),
        Value::Array(values) => format!("{} items", values.len()),
        Value::Object(values) => format!("{} fields", values.len()),
        Value::Null => "null".to_string(),
        value => value.to_string(),
    }
}

fn truncate_summary_at_boundary(text: &str, max_graphemes: usize) -> String {
    let text = sanitize_tool_inline(text);
    if text.graphemes(true).count() <= max_graphemes {
        return text;
    }
    if max_graphemes == 0 {
        return String::new();
    }

    let limit = max_graphemes.saturating_sub(1);
    let mut summary = String::new();
    for word in text.split_whitespace() {
        let separator = (!summary.is_empty()).then_some(" ").unwrap_or_default();
        if summary
            .graphemes(true)
            .count()
            .saturating_add(separator.graphemes(true).count())
            .saturating_add(word.graphemes(true).count())
            > limit
        {
            break;
        }
        summary.push_str(separator);
        summary.push_str(word);
    }
    if !summary.is_empty() {
        summary.push(' ');
    }
    summary.push('…');
    summary
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
    fn malformed_or_negative_structured_exit_codes_prevent_legacy_fallback() {
        for exit_code in [json!(-1), json!("7"), json!(i32::MAX as u64 + 1)] {
            let card = ToolCard::from_tool_call(&json!({
                "name": "shell",
                "exec": {"exit_code": exit_code},
            }));
            assert_eq!(card.reported_exit_code(), Some(None));
        }
    }
}
