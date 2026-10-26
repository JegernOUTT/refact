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

    fn render_raw(&self, width: usize) -> Vec<Line<'static>> {
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
    dim: bool,
}

impl InfoCell {
    pub fn new(lines: Vec<String>) -> Self {
        Self { lines, dim: false }
    }

    pub fn system_fact(lines: Vec<String>) -> Self {
        Self { lines, dim: true }
    }
}

impl HistoryCell for InfoCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Info
    }

    fn render_raw(&self, _width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some((first, rest)) = self.lines.split_first() {
            lines.push(if self.dim {
                system_fact_line(first)
            } else {
                info_line(first)
            });
            lines.extend(rest.iter().map(|text| {
                if self.dim {
                    system_fact_detail_line(text)
                } else {
                    Line::from(Span::styled(
                        text.clone(),
                        default_theme_style(ThemeRole::Muted),
                    ))
                }
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

    fn render_raw(&self, width: usize) -> Vec<Line<'static>> {
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
    default_theme_style(ThemeRole::Warning)
}

fn info_line(text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("• ", dim_style()),
        Span::raw(text.to_string()),
    ])
}

fn system_fact_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("• {text}"),
        default_theme_style(ThemeRole::Muted).add_modifier(Modifier::DIM),
    ))
}

fn system_fact_detail_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        default_theme_style(ThemeRole::Muted).add_modifier(Modifier::DIM),
    ))
}

fn warning_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    adaptive_wrap_lines(
        [Line::from(Span::styled(text.to_string(), warning_style()))],
        RtOptions::new(width.max(1))
            .initial_indent(Line::from(Span::styled("! ", warning_style())))
            .subsequent_indent(Line::from("  ")),
    )
}

fn error_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("■ {text}"),
        default_theme_style(ThemeRole::Error),
    ))
}

fn event_header_line(data: &EventCellData) -> Line<'static> {
    Line::from(vec![
        Span::styled("• ", dim_style()),
        Span::styled(
            format!("event · {} · {}", data.subkind, data.source),
            default_theme_style(ThemeRole::Muted),
        ),
    ])
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

    fn render_raw(&self, width: usize) -> Vec<Line<'static>> {
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
