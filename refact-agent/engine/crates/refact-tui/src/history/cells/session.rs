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

    fn render(&self, width: usize) -> Vec<Line<'static>> {
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
                Span::styled("/model", Style::default().fg(Color::Cyan)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::text;

    fn subtitle() -> String {
        "model: gpt-demo · /model to change\ndirectory: /tmp/demo\nTips: type /help for shortcuts"
            .to_string()
    }

    #[test]
    fn session_cell_snapshot() {
        let cell = SessionCell::new("New chat", Some(subtitle()));
        assert_eq!(
            text(&cell.render(80)),
            format!(
                ">_ refact (v{})\n\nmodel: gpt-demo · /model to change\ndirectory: /tmp/demo\nTips: type /help for shortcuts\n",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn session_cell_renders_model_directory_and_tips() {
        let cell = SessionCell::new("New chat", Some(subtitle()));
        let rendered = text(&cell.render(80));
        assert!(rendered.contains("model: gpt-demo"));
        assert!(rendered.contains("/model to change"));
        assert!(rendered.contains("directory: /tmp/demo"));
        assert!(rendered.contains("Tips: type /help"));
    }

    #[test]
    fn session_cell_truncates_lines_to_width() {
        let cell = SessionCell::new("New chat", Some(subtitle()));
        let lines = cell.render(12);
        assert!(lines.iter().all(|line| line_width(line) <= 12));
        assert_eq!(text(&lines).lines().next(), Some(">_ refact (v"));
    }
}
