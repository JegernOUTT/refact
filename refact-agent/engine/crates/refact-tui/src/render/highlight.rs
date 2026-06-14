use std::sync::OnceLock;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<Theme> = OnceLock::new();

pub fn highlight_code(code: &str, lang: &str, color_enabled: bool) -> Vec<Line<'static>> {
    if !color_enabled {
        return plain_code_lines(code);
    }
    highlight_code_with_syntect(code, lang).unwrap_or_else(|| fallback_highlight_code(code, lang))
}

fn highlight_code_with_syntect(code: &str, lang: &str) -> Option<Vec<Line<'static>>> {
    let syntax_set = SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines);
    let theme = THEME.get_or_init(load_theme);
    let syntax = syntax_set
        .find_syntax_by_token(language_name(lang))
        .or_else(|| syntax_set.find_syntax_by_name(language_name(lang)))
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text());
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut out = Vec::new();
    for line in code.lines() {
        let highlighted = highlighter.highlight_line(line, syntax_set).ok()?;
        let spans = highlighted
            .into_iter()
            .map(|(style, text)| Span::styled(text.to_string(), style_from_syntect(style)))
            .collect::<Vec<_>>();
        out.push(Line::from(spans));
    }
    if code.ends_with('\n') {
        out.push(Line::default());
    }
    if out.is_empty() {
        out.push(Line::default());
    }
    Some(out)
}

fn load_theme() -> Theme {
    let theme_set = ThemeSet::load_defaults();
    theme_set
        .themes
        .get("base16-ocean.dark")
        .or_else(|| theme_set.themes.values().next())
        .cloned()
        .unwrap_or_default()
}

fn style_from_syntect(style: syntect::highlighting::Style) -> Style {
    Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ))
}

fn language_name(lang: &str) -> &str {
    lang.split(|ch: char| ch.is_whitespace() || ch == ',')
        .next()
        .unwrap_or_default()
        .trim()
}

fn plain_code_lines(code: &str) -> Vec<Line<'static>> {
    let mut out = code
        .lines()
        .map(|line| Line::from(Span::raw(line.to_string())))
        .collect::<Vec<_>>();
    if code.ends_with('\n') {
        out.push(Line::default());
    }
    if out.is_empty() {
        out.push(Line::default());
    }
    out
}

fn fallback_highlight_code(code: &str, lang: &str) -> Vec<Line<'static>> {
    let fallback = Style::default().fg(Color::Yellow);
    let mut out = Vec::new();
    for line in code.lines() {
        out.push(Line::from(highlight_fallback_line(
            line,
            language_name(lang),
            fallback,
        )));
    }
    if code.ends_with('\n') {
        out.push(Line::default());
    }
    if out.is_empty() {
        out.push(Line::default());
    }
    out
}

fn highlight_fallback_line(line: &str, lang: &str, fallback: Style) -> Vec<Span<'static>> {
    match lang {
        "rust" | "rs" => highlight_with_keywords(line, RUST_KEYWORDS, fallback),
        "json" => highlight_with_keywords(line, JSON_KEYWORDS, fallback),
        "toml" => highlight_with_keywords(line, TOML_KEYWORDS, fallback),
        "bash" | "sh" | "shell" => highlight_with_keywords(line, SHELL_KEYWORDS, fallback),
        _ => vec![Span::styled(line.to_string(), fallback)],
    }
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "else", "enum", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "type", "unsafe", "use", "where",
    "while",
];
const JSON_KEYWORDS: &[&str] = &["true", "false", "null"];
const TOML_KEYWORDS: &[&str] = &["true", "false"];
const SHELL_KEYWORDS: &[&str] = &[
    "case", "do", "done", "elif", "else", "esac", "fi", "for", "function", "if", "in", "then",
    "while",
];

fn highlight_with_keywords(line: &str, keywords: &[&str], fallback: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut token = String::new();
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if !in_string && ch == '/' && chars.peek() == Some(&'/') {
            flush_keyword_or_token(&mut spans, &mut token, keywords, fallback);
            let mut comment = String::from(ch);
            comment.push(chars.next().unwrap_or('/'));
            comment.extend(chars);
            spans.push(Span::styled(comment, Style::default().fg(Color::DarkGray)));
            return spans;
        }

        if ch == '"' {
            if in_string && !escaped {
                token.push(ch);
                flush_token(&mut spans, &mut token, Style::default().fg(Color::Green));
                in_string = false;
                continue;
            }
            if !in_string {
                flush_keyword_or_token(&mut spans, &mut token, keywords, fallback);
                in_string = true;
            }
        }

        if in_string {
            token.push(ch);
            escaped = ch == '\\' && !escaped;
            if ch != '\\' {
                escaped = false;
            }
            continue;
        }

        if ch.is_ascii_alphanumeric() || ch == '_' {
            token.push(ch);
            continue;
        }

        flush_keyword_or_token(&mut spans, &mut token, keywords, fallback);
        spans.push(Span::styled(ch.to_string(), fallback));
    }

    if in_string {
        flush_token(&mut spans, &mut token, Style::default().fg(Color::Green));
    } else {
        flush_keyword_or_token(&mut spans, &mut token, keywords, fallback);
    }
    spans
}

fn flush_keyword_or_token(
    spans: &mut Vec<Span<'static>>,
    token: &mut String,
    keywords: &[&str],
    fallback: Style,
) {
    if token.is_empty() {
        return;
    }
    let style = if keywords.contains(&token.as_str()) {
        Style::default().fg(Color::Cyan)
    } else if token.chars().all(|ch| ch.is_ascii_digit()) {
        Style::default().fg(Color::Magenta)
    } else {
        fallback
    };
    flush_token(spans, token, style);
}

fn flush_token(spans: &mut Vec<Span<'static>>, token: &mut String, style: Style) {
    if token.is_empty() {
        return;
    }
    spans.push(Span::styled(std::mem::take(token), style));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_highlighting_uses_multiple_spans() {
        let lines = highlight_code("fn main() {}", "rust", true);
        assert!(lines[0].spans.len() > 1);
    }

    #[test]
    fn unknown_language_still_renders() {
        let lines = highlight_code("plain", "made-up", true);
        assert_eq!(lines[0].spans[0].content.as_ref(), "plain");
    }

    #[test]
    fn no_color_returns_plain_spans() {
        let lines = highlight_code("fn main() {}", "rust", false);
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].style, Style::default());
    }
}
