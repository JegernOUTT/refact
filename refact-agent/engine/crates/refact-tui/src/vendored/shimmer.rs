// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use ratatui::style::Style;
use ratatui::text::Span;

pub fn spans(text: &'static str, tick: u64, base: Style, highlight: Style) -> Vec<Span<'static>> {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return Vec::new();
    }
    let focus = tick as usize % chars.len();
    chars
        .into_iter()
        .enumerate()
        .map(|(idx, ch)| Span::styled(ch.to_string(), if idx == focus { highlight } else { base }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn highlights_one_character() {
        let spans = spans(
            "abc",
            1,
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::White),
        );
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].style.fg, Some(Color::White));
    }
}
