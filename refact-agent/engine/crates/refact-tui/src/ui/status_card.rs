// Adapted from openai/codex codex-rs/tui/src/status/{card.rs,format.rs,helpers.rs}, Apache-2.0.

use std::collections::BTreeSet;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::commands::session::{PermissionPolicy, StatusSnapshot};
use crate::text_formatting::{center_truncate_path, format_tokens_compact};
use crate::theme::{ThemeRole, TuiTheme};

#[derive(Debug, Clone)]
struct FieldFormatter {
    indent: &'static str,
    label_width: usize,
    value_offset: usize,
    label_style: Style,
}

impl FieldFormatter {
    const INDENT: &'static str = " ";

    fn from_labels<'a>(labels: impl IntoIterator<Item = &'a str>, theme: &TuiTheme) -> Self {
        let label_width = labels
            .into_iter()
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0);
        let indent_width = UnicodeWidthStr::width(Self::INDENT);
        let value_offset = indent_width + label_width + 1 + 3;

        Self {
            indent: Self::INDENT,
            label_width,
            value_offset,
            label_style: theme.style(ThemeRole::Muted),
        }
    }

    fn line(&self, label: &'static str, value_spans: Vec<Span<'static>>) -> Line<'static> {
        Line::from(self.full_spans(label, value_spans))
    }

    fn value_width(&self, available_inner_width: usize) -> usize {
        available_inner_width.saturating_sub(self.value_offset)
    }

    fn full_spans(&self, label: &str, mut value_spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
        let mut spans = Vec::with_capacity(value_spans.len() + 1);
        spans.push(self.label_span(label));
        spans.append(&mut value_spans);
        spans
    }

    fn label_span(&self, label: &str) -> Span<'static> {
        let mut text = String::with_capacity(self.value_offset);
        text.push_str(self.indent);
        text.push_str(label);
        text.push(':');
        let label_width = UnicodeWidthStr::width(label);
        let padding = 3 + self.label_width.saturating_sub(label_width);
        text.push_str(" ".repeat(padding).as_str());
        Span::styled(text, self.label_style)
    }
}

pub fn render(width: u16, snapshot: &StatusSnapshot, theme: &TuiTheme) -> Paragraph<'static> {
    Paragraph::new(render_lines(width as usize, snapshot, theme)).wrap(Wrap { trim: false })
}

pub(crate) fn render_lines(
    width: usize,
    snapshot: &StatusSnapshot,
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    let available_inner_width = width.saturating_sub(4);
    if available_inner_width == 0 {
        return Vec::new();
    }

    let labels = labels(snapshot);
    let formatter = FieldFormatter::from_labels(labels.iter().map(String::as_str), theme);
    let value_width = formatter.value_width(available_inner_width).max(1);
    let mut lines = vec![header_line(theme), Line::default()];

    lines.push(formatter.line("Daemon", value(snapshot.daemon_label())));
    lines.push(formatter.line(
        "Daemon URL source",
        value(snapshot.daemon_url_source_label().to_string()),
    ));
    lines.push(formatter.line("Worker", value(snapshot.worker.clone())));
    lines.push(formatter.line("Model", value(snapshot.model.clone())));
    lines.push(formatter.line("Mode", value(snapshot.mode.clone())));
    lines.push(formatter.line("Reasoning", value(snapshot.reasoning.clone())));
    lines.push(formatter.line(
        "Terminal background",
        value(crate::commands::session::terminal_background_status()),
    ));
    lines.push(formatter.line("Directory", value(directory_label(snapshot, value_width))));
    lines.push(formatter.line(
        "Permissions",
        value(permissions_label(snapshot.permission_policy)),
    ));
    lines.push(formatter.line("Token usage", token_usage_spans(snapshot, theme)));
    lines.push(formatter.line("Context window", context_window_spans(snapshot, theme)));

    if let Some(retry_hint) = snapshot.retry_hint.as_ref().filter(|hint| !hint.is_empty()) {
        lines.push(formatter.line(
            "Retry hint",
            vec![Span::styled(
                retry_hint.clone(),
                theme.style(ThemeRole::Warning),
            )],
        ));
    }

    let inner_width = lines
        .iter()
        .map(line_display_width)
        .max()
        .unwrap_or(0)
        .min(available_inner_width);
    let lines = lines
        .into_iter()
        .map(|line| truncate_line_to_width(line, inner_width))
        .collect::<Vec<_>>();
    with_border_with_inner_width(lines, inner_width, theme)
}

fn labels(snapshot: &StatusSnapshot) -> Vec<String> {
    let mut labels = Vec::new();
    let mut seen = BTreeSet::new();
    for label in [
        "Daemon",
        "Daemon URL source",
        "Worker",
        "Model",
        "Mode",
        "Reasoning",
        "Terminal background",
        "Directory",
        "Permissions",
        "Token usage",
        "Context window",
    ] {
        push_label(&mut labels, &mut seen, label);
    }
    if snapshot
        .retry_hint
        .as_ref()
        .is_some_and(|hint| !hint.is_empty())
    {
        push_label(&mut labels, &mut seen, "Retry hint");
    }
    labels
}

fn push_label(labels: &mut Vec<String>, seen: &mut BTreeSet<String>, label: &str) {
    if seen.insert(label.to_string()) {
        labels.push(label.to_string());
    }
}

fn header_line(theme: &TuiTheme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            FieldFormatter::INDENT.to_string(),
            theme.style(ThemeRole::Muted),
        ),
        Span::styled("refact".to_string(), theme.style(ThemeRole::Accent)),
        Span::styled(
            format!(" (v{})", env!("CARGO_PKG_VERSION")),
            theme.style(ThemeRole::Muted),
        ),
    ])
}

fn directory_label(snapshot: &StatusSnapshot, max_width: usize) -> String {
    match snapshot
        .project_root
        .as_ref()
        .filter(|root| !root.is_empty())
    {
        Some(root) => center_truncate_path(root, max_width),
        None => snapshot.project.clone(),
    }
}

fn permissions_label(policy: PermissionPolicy) -> String {
    format!(
        "auto_approve_editing_tools={} · auto_approve_dangerous_commands={}",
        policy.auto_approve_editing_tools, policy.auto_approve_dangerous_commands
    )
}

fn value(text: String) -> Vec<Span<'static>> {
    vec![Span::from(text)]
}

fn token_usage_spans(snapshot: &StatusSnapshot, theme: &TuiTheme) -> Vec<Span<'static>> {
    let Some(usage) = snapshot.usage.as_ref() else {
        return muted_value("not reported", theme);
    };
    let components = [
        usage.prompt_tokens.map(|tokens| (tokens, "input")),
        usage.completion_tokens.map(|tokens| (tokens, "output")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if usage.total_tokens.is_none() && components.is_empty() {
        return muted_value("not reported", theme);
    }
    let has_components = !components.is_empty();
    let mut spans = Vec::new();
    if let Some(total) = usage.total_tokens {
        spans.push(Span::from(format_tokens_compact(total)));
        spans.push(Span::from(" total"));
        if has_components {
            spans.push(Span::styled(" (", theme.style(ThemeRole::Muted)));
        }
    }
    for (index, (tokens, label)) in components.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" + ", theme.style(ThemeRole::Muted)));
        }
        spans.push(Span::styled(
            format!("{} {label}", format_tokens_compact(tokens)),
            theme.style(ThemeRole::Muted),
        ));
    }
    if usage.total_tokens.is_some() && has_components {
        spans.push(Span::styled(")", theme.style(ThemeRole::Muted)));
    }
    spans
}

fn context_window_spans(snapshot: &StatusSnapshot, theme: &TuiTheme) -> Vec<Span<'static>> {
    let Some(usage) = snapshot.usage.as_ref() else {
        return muted_value("not reported", theme);
    };
    let Some(total) = usage.total_tokens else {
        return muted_value("not reported", theme);
    };
    let Some(window) = usage.context_window_tokens.filter(|window| *window > 0) else {
        return muted_value("not reported", theme);
    };
    vec![
        Span::from(format!("{}% left", context_left_percent(total, window))),
        Span::styled(" (", theme.style(ThemeRole::Muted)),
        Span::styled(format_tokens_compact(total), theme.style(ThemeRole::Muted)),
        Span::styled("/", theme.style(ThemeRole::Muted)),
        Span::styled(format_tokens_compact(window), theme.style(ThemeRole::Muted)),
        Span::styled(")", theme.style(ThemeRole::Muted)),
    ]
}

fn muted_value(text: &str, theme: &TuiTheme) -> Vec<Span<'static>> {
    vec![Span::styled(
        text.to_string(),
        theme.style(ThemeRole::Muted),
    )]
}

fn context_left_percent(used: u64, window: u64) -> u64 {
    let remaining = window.saturating_sub(used);
    (((remaining as u128 * 100) + (window as u128 / 2)) / window as u128) as u64
}

fn with_border_with_inner_width(
    lines: Vec<Line<'static>>,
    inner_width: usize,
    theme: &TuiTheme,
) -> Vec<Line<'static>> {
    let content_width = inner_width.max(lines.iter().map(line_display_width).max().unwrap_or(0));
    let mut out = Vec::with_capacity(lines.len() + 2);
    let border_inner_width = content_width + 2;
    let border_style = theme.style(ThemeRole::Muted);
    out.push(Line::from(Span::styled(
        format!("╭{}╮", "─".repeat(border_inner_width)),
        border_style,
    )));

    for line in lines {
        let used_width = line_display_width(&line);
        let mut spans = Vec::with_capacity(line.spans.len() + 4);
        spans.push(Span::styled("│ ".to_string(), border_style));
        spans.extend(line.spans);
        if used_width < content_width {
            spans.push(Span::styled(
                " ".repeat(content_width - used_width),
                border_style,
            ));
        }
        spans.push(Span::styled(" │".to_string(), border_style));
        out.push(Line::from(spans));
    }

    out.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(border_inner_width)),
        border_style,
    )));
    out
}

fn line_display_width(line: &Line<'static>) -> usize {
    line.spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn truncate_line_to_width(line: Line<'static>, max_width: usize) -> Line<'static> {
    if max_width == 0 {
        return Line::from(Vec::<Span<'static>>::new());
    }

    let mut used = 0usize;
    let mut spans_out = Vec::<Span<'static>>::new();
    for span in line.spans {
        let text = span.content.into_owned();
        let style = span.style;
        let span_width = UnicodeWidthStr::width(text.as_str());
        if span_width == 0 {
            spans_out.push(Span::styled(text, style));
            continue;
        }
        if used >= max_width {
            break;
        }
        if used + span_width <= max_width {
            used += span_width;
            spans_out.push(Span::styled(text, style));
            continue;
        }

        let mut truncated = String::new();
        for ch in text.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + ch_width > max_width {
                break;
            }
            truncated.push(ch);
            used += ch_width;
        }
        if !truncated.is_empty() {
            spans_out.push(Span::styled(truncated, style));
        }
        break;
    }
    Line::from(spans_out)
}

trait StatusSnapshotExt {
    fn daemon_label(&self) -> String;
    fn daemon_url_source_label(&self) -> &'static str;
}

impl StatusSnapshotExt for StatusSnapshot {
    fn daemon_label(&self) -> String {
        if !self.daemon_online {
            return "offline".to_string();
        }
        match (&self.daemon_version, self.daemon_port) {
            (Some(version), Some(port)) => format!("v{version} on port {port}"),
            (Some(version), None) => format!("v{version}"),
            (None, Some(port)) => format!("online on port {port}"),
            (None, None) => self
                .daemon_base_url
                .as_ref()
                .map(|url| format!("online at {url}"))
                .unwrap_or_else(|| "online, details loading".to_string()),
        }
    }

    fn daemon_url_source_label(&self) -> &'static str {
        self.daemon_url_source
            .map(crate::client::DaemonUrlSource::as_str)
            .unwrap_or("not reported")
    }
}
