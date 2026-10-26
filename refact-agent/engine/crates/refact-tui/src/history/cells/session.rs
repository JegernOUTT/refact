use super::*;
use crate::render::wrapping::take_prefix_by_width;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionCell {
    title: String,
    subtitle: Option<String>,
}

impl SessionCell {
    pub fn new(title: impl Into<String>, subtitle: Option<String>) -> Self {
        Self {
            title: title.into(),
            subtitle,
        }
    }
}

impl HistoryCell for SessionCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Session
    }

    fn render_raw(&self, width: usize) -> Vec<Line<'static>> {
        let width = width.max(1);
        let mut lines = vec![clipped_line(session_title_spans(), width), Line::from("")];
        lines.extend(session_detail_lines(self.subtitle.as_deref(), width));
        finish(lines)
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.title, &self.subtitle))
    }
}

fn session_title_spans() -> Vec<Span<'static>> {
    let dim = Style::default().add_modifier(Modifier::DIM);
    vec![
        Span::styled(">_ ", dim),
        Span::styled("refact", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(format!(" (v{})", env!("CARGO_PKG_VERSION")), dim),
    ]
}

fn session_detail_lines(subtitle: Option<&str>, width: usize) -> Vec<Line<'static>> {
    subtitle
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| session_detail_line(line, width))
        .collect()
}

fn session_detail_line(text: &str, width: usize) -> Line<'static> {
    let dim = Style::default().add_modifier(Modifier::DIM);
    if let Some((before, after)) = text.split_once("/model") {
        return clipped_line(
            vec![
                Span::styled(before.to_string(), dim),
                Span::styled("/model", default_theme_style(ThemeRole::Highlight)),
                Span::styled(after.to_string(), dim),
            ],
            width,
        );
    }
    clipped_line(vec![Span::styled(text.to_string(), dim)], width)
}

fn clipped_line(spans: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for span in spans {
        if used >= width {
            break;
        }
        let content = span.content.into_owned();
        let (prefix, _, prefix_width) = take_prefix_by_width(&content, width - used);
        if !prefix.is_empty() {
            out.push(Span::styled(prefix, span.style));
        }
        used += prefix_width;
    }
    Line::from(out)
}
