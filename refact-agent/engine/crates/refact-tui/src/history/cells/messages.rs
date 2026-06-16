use super::*;

use crate::render::line_utils::prefix_lines;
use crate::render::width::usable_content_width;
use crate::style::user_message_style;
use crate::ui_consts::LIVE_PREFIX_COLS;
use crate::vendored::terminal_hyperlinks::{prefix_hyperlink_lines, visible_lines};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserCell {
    text: String,
    selected: bool,
}

impl UserCell {
    pub fn new(text: impl Into<String>, selected: bool) -> Self {
        Self {
            text: text.into(),
            selected,
        }
    }

    fn wrapped_body(&self, width: usize) -> Vec<Line<'static>> {
        let source = self.text.trim_end_matches(['\r', '\n']);
        if source.is_empty() {
            return Vec::new();
        }
        let style = user_message_style();
        adaptive_wrap_lines(
            source
                .split('\n')
                .map(|line| user_line_with_mentions(line, style)),
            RtOptions::new(user_wrap_width(width)),
        )
    }
}

impl HistoryCell for UserCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::User
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        let style = user_message_style();
        let mut lines = vec![Line::from("").style(style)];
        lines.extend(prefix_lines(
            self.wrapped_body(width),
            Span::styled(
                "› ",
                Style::default().add_modifier(Modifier::BOLD | Modifier::DIM),
            ),
            Span::raw("  "),
        ));
        lines.push(Line::from("").style(style));
        lines
    }

    fn render_with_links(&self, width: usize) -> Vec<HyperlinkLine> {
        let style = user_message_style();
        let mut lines = vec![HyperlinkLine::new(Line::from("").style(style))];
        lines.extend(prefix_hyperlink_lines(
            plain_hyperlink_lines(self.wrapped_body(width)),
            Span::styled(
                "› ",
                Style::default().add_modifier(Modifier::BOLD | Modifier::DIM),
            ),
            Span::raw("  "),
        ));
        lines.push(HyperlinkLine::new(Line::from("").style(style)));
        lines
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text, self.selected))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AssistantCell {
    text: String,
}

impl AssistantCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl HistoryCell for AssistantCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Assistant
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        visible_lines(self.render_with_links(width))
    }

    fn render_with_links(&self, width: usize) -> Vec<HyperlinkLine> {
        let renderer = MarkdownRenderer::new(Some(prefixed_body_width(width)));
        prefix_hyperlink_lines(
            renderer.render_with_links(&self.text),
            Span::styled("• ", Style::default().add_modifier(Modifier::DIM)),
            Span::raw("  "),
        )
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantStreamCell {
    lines: Vec<HyperlinkLine>,
    first: bool,
}

impl AssistantStreamCell {
    pub fn new(text: impl AsRef<str>, first: bool) -> Self {
        let mut lines = text
            .as_ref()
            .split('\n')
            .map(|line| Line::from(line.to_string()))
            .collect::<Vec<_>>();
        if text.as_ref().ends_with('\n') {
            lines.pop();
        }
        Self {
            lines: plain_hyperlink_lines(lines),
            first,
        }
    }

    pub fn new_lines(lines: Vec<HyperlinkLine>, first: bool) -> Self {
        Self { lines, first }
    }
}

impl HistoryCell for AssistantStreamCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Assistant
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        visible_lines(self.render_with_links(width))
    }

    fn render_with_links(&self, _width: usize) -> Vec<HyperlinkLine> {
        let mut out = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            let prefix = if index == 0 && self.first {
                Span::styled("• ", Style::default().add_modifier(Modifier::DIM))
            } else {
                Span::raw("  ")
            };
            out.extend(prefix_hyperlink_lines(
                vec![line.clone()],
                prefix,
                Span::raw("  "),
            ));
        }
        out
    }

    fn is_stream_continuation(&self) -> bool {
        !self.first
    }

    fn is_final(&self) -> bool {
        false
    }

    fn revision(&self) -> u64 {
        let text = self
            .lines
            .iter()
            .map(|line| {
                line.line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        revision(&(self.kind(), text, self.first))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReasoningCell {
    text: String,
    collapsed: bool,
}

impl ReasoningCell {
    pub fn new(text: impl Into<String>, collapsed: bool) -> Self {
        Self {
            text: text.into(),
            collapsed,
        }
    }

    pub fn update_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    pub fn collapsed(&self) -> bool {
        self.collapsed
    }

    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }
}

impl HistoryCell for ReasoningCell {
    fn kind(&self) -> HistoryCellKind {
        HistoryCellKind::Reasoning
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        visible_lines(self.render_with_links(width))
    }

    fn render_with_links(&self, width: usize) -> Vec<HyperlinkLine> {
        let lines = if self.collapsed {
            vec![HyperlinkLine::new(reasoning_line("collapsed"))]
        } else {
            let renderer = MarkdownRenderer::new(Some(prefixed_body_width(width)));
            style_hyperlink_lines(renderer.render_with_links(&self.text), reasoning_style())
        };
        prefix_hyperlink_lines(
            lines,
            Span::styled("• ", Style::default().add_modifier(Modifier::DIM)),
            Span::raw("  "),
        )
    }

    fn revision(&self) -> u64 {
        revision(&(self.kind(), &self.text, self.collapsed))
    }
}
fn user_wrap_width(width: usize) -> usize {
    width.saturating_sub(LIVE_PREFIX_COLS as usize + 1).max(1)
}

fn prefixed_body_width(width: usize) -> usize {
    usable_content_width(width, 2).unwrap_or(1)
}

fn user_line_with_mentions(line: &str, style: Style) -> Line<'static> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = line[cursor..].find('@') {
        let start = cursor + relative_start;
        if start > cursor {
            spans.push(Span::raw(line[cursor..start].to_string()));
        }
        let end = line[start..]
            .char_indices()
            .find_map(|(index, ch)| (index > 0 && ch.is_whitespace()).then_some(start + index))
            .unwrap_or(line.len());
        spans.push(Span::styled(
            line[start..end].to_string(),
            style.patch(Style::default().fg(Color::Cyan)),
        ));
        cursor = end;
    }
    if cursor < line.len() {
        spans.push(Span::raw(line[cursor..].to_string()));
    }
    if spans.is_empty() {
        Line::from("").style(style)
    } else {
        Line::from(spans).style(style)
    }
}

fn reasoning_style() -> Style {
    Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::DIM | Modifier::ITALIC)
}

fn reasoning_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), reasoning_style()))
}

fn style_hyperlink_lines(lines: Vec<HyperlinkLine>, style: Style) -> Vec<HyperlinkLine> {
    lines
        .into_iter()
        .map(|mut line| {
            line.line.style = line.line.style.patch(style);
            line.line.spans = line
                .line
                .spans
                .into_iter()
                .map(|span| Span::styled(span.content.into_owned(), span.style.patch(style)))
                .collect();
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::cells::test_support::text;

    #[test]
    fn user_cell_snapshot() {
        let cell = UserCell::new("hello @there", false);
        assert_eq!(text(&cell.render(40)), "\n› hello @there\n");
        let style = user_message_style();
        assert!(cell.render(40).iter().all(|line| line.style == style));
        let mention = cell
            .render(40)
            .into_iter()
            .flat_map(|line| line.spans)
            .find(|span| span.content.as_ref() == "@there")
            .unwrap();
        assert_eq!(mention.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn user_cell_wraps_with_hanging_gutter_and_band() {
        let cell = UserCell::new("alpha beta gamma", false);
        let lines = cell.render(10);

        assert_eq!(text(&lines), "\n› alpha\n  beta\n  gamma\n");
        assert!(lines.iter().all(|line| line.style == user_message_style()));
    }

    #[test]
    fn assistant_cell_snapshot() {
        let cell = AssistantCell::new("| A | B |\n|---|---|\n| one | two |");
        assert_eq!(
            text(&cell.render(40)),
            "•  A      B\n  ━━━━━  ━━━━━\n   one    two"
        );
    }

    #[test]
    fn assistant_cell_rerenders_markdown_from_source_on_resize() {
        let cell = AssistantCell::new("alpha beta gamma delta");

        assert_eq!(text(&cell.render(80)), "• alpha beta gamma delta");
        assert_eq!(text(&cell.render(8)), "• alpha\n  beta\n  gamma\n  delta");
    }

    #[test]
    fn reasoning_cell_snapshot_and_update_preserves_collapse() {
        let mut cell = ReasoningCell::new("hidden plan", false);
        assert_eq!(text(&cell.render(40)), "• hidden plan");
        let reasoning_span = cell.render(40)[0].spans[1].clone();
        assert!(reasoning_span.style.add_modifier.contains(Modifier::DIM));
        assert!(reasoning_span.style.add_modifier.contains(Modifier::ITALIC));
        cell.update_text("updated plan");
        assert!(!cell.collapsed());
        cell.set_collapsed(true);
        assert_eq!(text(&cell.render(40)), "• collapsed");
    }
}
