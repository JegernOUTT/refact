// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use ratatui::style::Style;
use ratatui::text::Span;

pub fn spans(text: &str, tick: u64, base: Style, highlight: Style) -> Vec<Span<'static>> {
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
