use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{ThemeRole, TuiTheme};

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
    plain_with_theme(&TuiTheme::default(), key_label)
}

pub fn plain_with_theme(theme: &TuiTheme, key_label: impl Into<String>) -> Span<'static> {
    Span::styled(key_label.into(), key_style(theme))
}

pub fn key(key_label: impl Into<String>) -> Span<'static> {
    plain(key_label)
}

pub fn key_with_theme(theme: &TuiTheme, key_label: impl Into<String>) -> Span<'static> {
    plain_with_theme(theme, key_label)
}

pub fn label(label: impl Into<String>) -> Span<'static> {
    label_with_theme(&TuiTheme::default(), label)
}

pub fn label_with_theme(theme: &TuiTheme, label: impl Into<String>) -> Span<'static> {
    Span::styled(label.into(), label_style(theme))
}

pub fn pair(key_label: impl Into<String>, description: impl Into<String>) -> Line<'static> {
    pair_with_theme(&TuiTheme::default(), key_label, description)
}

pub fn pair_with_theme(
    theme: &TuiTheme,
    key_label: impl Into<String>,
    description: impl Into<String>,
) -> Line<'static> {
    let key_label = key_label.into();
    let description = description.into();
    Line::from(vec![
        key_with_theme(theme, key_label),
        Span::raw(" "),
        label_with_theme(theme, description),
    ])
}

pub fn pairs<'a>(pairs: impl IntoIterator<Item = KeyHint<'a>>) -> Line<'static> {
    pairs_with_theme(&TuiTheme::default(), pairs)
}

pub fn pairs_with_theme<'a>(
    theme: &TuiTheme,
    pairs: impl IntoIterator<Item = KeyHint<'a>>,
) -> Line<'static> {
    let mut spans = Vec::new();
    for hint in pairs {
        if !spans.is_empty() {
            spans.push(Span::raw("   "));
        }
        spans.push(key_with_theme(theme, hint.key.to_string()));
        spans.push(Span::raw(" "));
        spans.push(label_with_theme(theme, hint.label.to_string()));
    }
    Line::from(spans)
}

pub fn joined(hints: &[(&str, &str)]) -> Line<'static> {
    pairs(hints.iter().map(|(key, label)| KeyHint::new(key, label)))
}

pub fn joined_with_theme(theme: &TuiTheme, hints: &[(&str, &str)]) -> Line<'static> {
    pairs_with_theme(
        theme,
        hints.iter().map(|(key, label)| KeyHint::new(key, label)),
    )
}

impl<'a> FromIterator<KeyHint<'a>> for Line<'static> {
    fn from_iter<T: IntoIterator<Item = KeyHint<'a>>>(iter: T) -> Self {
        pairs(iter)
    }
}

fn key_style(theme: &TuiTheme) -> Style {
    theme
        .style(ThemeRole::Highlight)
        .add_modifier(Modifier::BOLD)
}

fn label_style(theme: &TuiTheme) -> Style {
    theme.style(ThemeRole::Muted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn renders_hint_line_with_key_chips_and_labels() {
        let theme = TuiTheme::dark();
        let line = pairs_with_theme(
            &theme,
            [
                KeyHint::new("Enter", "send"),
                KeyHint::new("Ctrl-J", "newline"),
            ],
        );

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

    #[test]
    fn renders_key_chips_with_the_passed_theme() {
        let theme = TuiTheme::light();
        let chip = key_with_theme(&theme, "Enter");
        let label = label_with_theme(&theme, "send");

        assert_eq!(chip.style.fg, Some(Color::Blue));
        assert!(chip.style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(label.style.fg, Some(Color::Gray));
    }
}
