use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyHint<'a> {
    pub key: &'a str,
    pub label: &'a str,
}

impl<'a> KeyHint<'a> {
    pub const fn new(key: &'a str, label: &'a str) -> Self {
        Self { key, label }
    }
}

pub fn plain(key_label: impl Into<String>) -> Span<'static> {
    Span::styled(key_label.into(), key_style())
}

pub fn key(key_label: impl Into<String>) -> Span<'static> {
    plain(key_label)
}

pub fn label(label: impl Into<String>) -> Span<'static> {
    Span::styled(label.into(), label_style())
}

pub fn pair(key_label: impl Into<String>, description: impl Into<String>) -> Line<'static> {
    let key_label = key_label.into();
    let description = description.into();
    Line::from(vec![key(key_label), Span::raw(" "), label(description)])
}

pub fn pairs<'a>(pairs: impl IntoIterator<Item = KeyHint<'a>>) -> Line<'static> {
    let mut spans = Vec::new();
    for hint in pairs {
        if !spans.is_empty() {
            spans.push(Span::raw("   "));
        }
        spans.push(key(hint.key.to_string()));
        spans.push(Span::raw(" "));
        spans.push(label(hint.label.to_string()));
    }
    Line::from(spans)
}

pub fn joined(pairs: &[(&str, &str)]) -> Line<'static> {
    pairs
        .iter()
        .map(|(key, label)| KeyHint::new(key, label))
        .collect()
}

impl<'a> FromIterator<KeyHint<'a>> for Line<'static> {
    fn from_iter<T: IntoIterator<Item = KeyHint<'a>>>(iter: T) -> Self {
        pairs(iter)
    }
}

fn key_style() -> Style {
    crate::theme::TuiTheme::dark()
        .style(crate::theme::ThemeRole::Highlight)
        .add_modifier(Modifier::BOLD)
}

fn label_style() -> Style {
    crate::theme::TuiTheme::dark().style(crate::theme::ThemeRole::Muted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn renders_hint_line_with_key_chips_and_labels() {
        let line = pairs([
            KeyHint::new("Enter", "send"),
            KeyHint::new("Ctrl-J", "newline"),
        ]);

        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text, "Enter send   Ctrl-J newline");
        assert_eq!(line.spans[0].style.fg, Some(Color::Cyan));
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line.spans[2].style.fg, Some(Color::DarkGray));
        assert_eq!(line.spans[3].content.as_ref(), "   ");
    }
}
