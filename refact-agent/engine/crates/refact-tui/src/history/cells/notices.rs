use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NoticeCell {
    text: String,
}

impl NoticeCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl HistoryCell for NoticeCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Notice
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        finish(match notice_kind(&self.text) {
            NoticeKind::Info => vec![info_line(&self.text)],
            NoticeKind::Warning => warning_lines(&self.text, width),
            NoticeKind::Error => vec![error_line(&self.text)],
        })
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InfoCell {
    lines: Vec<String>,
}

impl InfoCell {
    pub fn new(lines: Vec<String>) -> Self {
        Self { lines }
    }
}

impl HistoryCell for InfoCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Info
    }

    fn render(&self, _width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some((first, rest)) = self.lines.split_first() {
            lines.push(info_line(first));
            lines.extend(rest.iter().map(|text| {
                Line::from(Span::styled(
                    text.clone(),
                    Style::default().fg(Color::DarkGray),
                ))
            }));
        }
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.lines))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventCellData {
    pub subkind: String,
    pub source: String,
    pub content: String,
}

impl EventCellData {
    pub fn new(
        subkind: impl Into<String>,
        source: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            subkind: subkind.into(),
            source: source.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventCell {
    data: EventCellData,
}

impl EventCell {
    pub fn new(data: EventCellData) -> Self {
        Self { data }
    }
}

impl HistoryCell for EventCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Event
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let renderer = MarkdownRenderer::new(Some(width));
        let mut lines = vec![event_header_line(&self.data)];
        lines.extend(renderer.render(&self.data.content));
        finish(lines)
    }

    fn render_with_links(&self, width: usize) -> Vec<HyperlinkLine> {
        let renderer = MarkdownRenderer::new(Some(width));
        let mut lines = vec![HyperlinkLine::new(event_header_line(&self.data))];
        lines.extend(renderer.render_with_links(&self.data.content));
        finish_links(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.data))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeKind {
    Info,
    Warning,
    Error,
}

fn notice_kind(text: &str) -> NoticeKind {
    let lowered = text.to_ascii_lowercase();
    if lowered.contains("warning") || lowered.starts_with("warn") || lowered.contains(" warn") {
        NoticeKind::Warning
    } else if [
        "failed",
        "failure",
        "error",
        "denied",
        "invalid",
        "unavailable",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
    {
        NoticeKind::Error
    } else {
        NoticeKind::Info
    }
}

fn dim_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn warning_style() -> Style {
    Style::default().fg(Color::Yellow)
}

fn info_line(text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("• ", dim_style()),
        Span::raw(text.to_string()),
    ])
}

fn warning_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    adaptive_wrap_lines(
        [Line::from(Span::styled(text.to_string(), warning_style()))],
        RtOptions::new(width.max(1))
            .initial_indent(Line::from(Span::styled("⚠ ", warning_style())))
            .subsequent_indent(Line::from("  ")),
    )
}

fn error_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("■ {text}"),
        Style::default().fg(Color::Red),
    ))
}

fn event_header_line(data: &EventCellData) -> Line<'static> {
    Line::from(vec![
        Span::styled("• ", dim_style()),
        Span::styled(
            format!("event · {} · {}", data.subkind, data.source),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::text;

    #[test]
    fn notice_cell_snapshot() {
        assert_eq!(
            text(&NoticeCell::new("SSE disconnected").render(40)),
            "• SSE disconnected\n"
        );
        assert_eq!(
            text(&NoticeCell::new("Failed to load config").render(40)),
            "■ Failed to load config\n"
        );
        assert_eq!(
            text(&NoticeCell::new("TUI keymap warning: duplicate binding").render(18)),
            "⚠ TUI keymap\n  warning:\n  duplicate\n  binding\n"
        );
    }

    #[test]
    fn info_cell_snapshot() {
        let cell = InfoCell::new(vec![
            "TUI debug config".to_string(),
            "Config: /tmp/refact.toml".to_string(),
            "Theme: dark".to_string(),
        ]);
        let lines = cell.render(80);
        assert_eq!(
            text(&lines),
            "• TUI debug config\nConfig: /tmp/refact.toml\nTheme: dark\n"
        );
        assert_eq!(lines[1].spans[0].style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn event_cell_snapshot() {
        let cell = EventCell::new(EventCellData::new(
            "process_completed",
            "exec.registry",
            "Process exited with code 0",
        ));
        assert_eq!(
            text(&cell.render(80)),
            "• event · process_completed · exec.registry\nProcess exited with code 0\n"
        );
    }
}

#[derive(Debug, Clone)]
pub struct StatusCell {
    snapshot: crate::commands::session::StatusSnapshot,
    theme: crate::theme::TuiTheme,
}

impl StatusCell {
    pub fn new(
        snapshot: crate::commands::session::StatusSnapshot,
        theme: crate::theme::TuiTheme,
    ) -> Self {
        Self { snapshot, theme }
    }
}

impl HistoryCell for StatusCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Info
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        finish(crate::ui::status_card::render_lines(
            width,
            &self.snapshot,
            &self.theme,
        ))
    }

    fn revision(&self) -> u64 {
        revision(&format!("{:?}|{}", self.snapshot, self.theme.name()))
    }
}
