use std::ops::Range;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy)]
struct StyledChar {
    ch: char,
    style: Style,
}

#[derive(Clone)]
enum Piece {
    Range(Range<usize>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Space,
    Word,
}

#[derive(Clone)]
struct Token {
    kind: TokenKind,
    range: Range<usize>,
}

pub fn wrap_line(line: Line<'static>, width: Option<usize>) -> Vec<Line<'static>> {
    let Some(width) = width.filter(|width| *width > 0) else {
        return vec![line];
    };
    if line_width(&line) <= width {
        return vec![line];
    }
    let chars = flatten_line(&line);
    if chars.is_empty() {
        return vec![Line::default()];
    }
    let tokens = tokens(&chars);
    let mut out = Vec::new();
    let mut current = Vec::<Piece>::new();
    let mut current_width = 0usize;

    let mut pending_space = None::<Range<usize>>;
    for token in tokens {
        match token.kind {
            TokenKind::Space if current.is_empty() => append_range(
                &chars,
                token.range,
                width,
                &mut out,
                &mut current,
                &mut current_width,
            ),
            TokenKind::Space => pending_space = Some(token.range),
            TokenKind::Word => {
                if let Some(space_range) = pending_space.take() {
                    append_space_and_word(
                        &chars,
                        space_range,
                        token.range,
                        width,
                        &mut out,
                        &mut current,
                        &mut current_width,
                    );
                } else {
                    append_range(
                        &chars,
                        token.range,
                        width,
                        &mut out,
                        &mut current,
                        &mut current_width,
                    );
                }
            }
        }
    }
    if let Some(space_range) = pending_space {
        append_range(
            &chars,
            space_range,
            width,
            &mut out,
            &mut current,
            &mut current_width,
        );
    }
    push_current(&chars, &mut out, &mut current, &mut current_width);
    if out.is_empty() {
        out.push(Line::default());
    }
    out
}

fn append_space_and_word(
    chars: &[StyledChar],
    space_range: Range<usize>,
    word_range: Range<usize>,
    width: usize,
    out: &mut Vec<Line<'static>>,
    current: &mut Vec<Piece>,
    current_width: &mut usize,
) {
    let space_width = range_width(chars, space_range.clone());
    let word_width = range_width(chars, word_range.clone());
    if *current_width + space_width + word_width <= width {
        current.push(Piece::Range(space_range));
        current.push(Piece::Range(word_range));
        *current_width += space_width + word_width;
        return;
    }

    push_current(chars, out, current, current_width);
    if space_width > 1 {
        append_range(chars, space_range, width, out, current, current_width);
    }
    append_range(chars, word_range, width, out, current, current_width);
}

fn append_range(
    chars: &[StyledChar],
    range: Range<usize>,
    width: usize,
    out: &mut Vec<Line<'static>>,
    current: &mut Vec<Piece>,
    current_width: &mut usize,
) {
    let token_width = range_width(chars, range.clone());
    if token_width > width {
        if !current.is_empty() {
            push_current(chars, out, current, current_width);
        }
        push_hard_wrapped_range(chars, range, width, out);
    } else if *current_width + token_width <= width {
        current.push(Piece::Range(range));
        *current_width += token_width;
    } else {
        push_current(chars, out, current, current_width);
        current.push(Piece::Range(range));
        *current_width = token_width;
    }
}

pub fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    wrap_line(Line::from(text.to_string()), Some(width))
        .into_iter()
        .map(|line| line_to_plain(&line))
        .collect()
}

pub fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

pub fn line_to_plain(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

pub fn pad_line(mut line: Line<'static>, width: usize) -> Line<'static> {
    let current = line_width(&line);
    if current < width {
        line.spans.push(Span::raw(" ".repeat(width - current)));
    }
    line
}

pub fn recolor_line(line: Line<'static>, style: Style) -> Line<'static> {
    Line::from(
        line.spans
            .into_iter()
            .map(|span| Span::styled(span.content.into_owned(), span.style.patch(style)))
            .collect::<Vec<_>>(),
    )
}

fn flatten_line(line: &Line<'_>) -> Vec<StyledChar> {
    line.spans
        .iter()
        .flat_map(|span| {
            span.content.chars().map(move |ch| StyledChar {
                ch,
                style: span.style,
            })
        })
        .collect()
}

fn tokens(chars: &[StyledChar]) -> Vec<Token> {
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let kind = if chars[start].ch.is_whitespace() {
            TokenKind::Space
        } else {
            TokenKind::Word
        };
        let mut end = start + 1;
        while end < chars.len() && chars[end].ch.is_whitespace() == (kind == TokenKind::Space) {
            end += 1;
        }
        out.push(Token {
            kind,
            range: start..end,
        });
        start = end;
    }
    out
}

fn push_current(
    chars: &[StyledChar],
    out: &mut Vec<Line<'static>>,
    current: &mut Vec<Piece>,
    current_width: &mut usize,
) {
    if current.is_empty() {
        *current_width = 0;
        return;
    }
    out.push(line_from_pieces(chars, current));
    current.clear();
    *current_width = 0;
}

fn push_hard_wrapped_range(
    chars: &[StyledChar],
    range: Range<usize>,
    width: usize,
    out: &mut Vec<Line<'static>>,
) {
    let mut start = range.start;
    let mut cursor = range.start;
    let mut current_width = 0usize;
    while cursor < range.end {
        let ch_width = char_width(chars[cursor].ch).max(1);
        if current_width > 0 && current_width + ch_width > width {
            out.push(line_from_range(chars, start..cursor));
            start = cursor;
            current_width = 0;
        }
        current_width += ch_width;
        cursor += 1;
    }
    if start < range.end {
        out.push(line_from_range(chars, start..range.end));
    }
}

fn line_from_pieces(chars: &[StyledChar], pieces: &[Piece]) -> Line<'static> {
    let mut spans = Vec::<Span<'static>>::new();
    for piece in pieces {
        match piece {
            Piece::Range(range) => push_range_spans(chars, range.clone(), &mut spans),
        }
    }
    Line::from(spans)
}

fn line_from_range(chars: &[StyledChar], range: Range<usize>) -> Line<'static> {
    let mut spans = Vec::<Span<'static>>::new();
    push_range_spans(chars, range, &mut spans);
    Line::from(spans)
}

fn push_range_spans(chars: &[StyledChar], range: Range<usize>, spans: &mut Vec<Span<'static>>) {
    for idx in range {
        push_span(spans, chars[idx].ch, chars[idx].style);
    }
}

fn push_span(spans: &mut Vec<Span<'static>>, ch: char, style: Style) {
    if let Some(last) = spans.last_mut() {
        if last.style == style {
            last.content.to_mut().push(ch);
            return;
        }
    }
    spans.push(Span::styled(ch.to_string(), style));
}

fn range_width(chars: &[StyledChar], range: Range<usize>) -> usize {
    chars[range].iter().map(|value| char_width(value.ch)).sum()
}

fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: Vec<Line<'static>>) -> Vec<String> {
        lines.iter().map(line_to_plain).collect()
    }

    #[test]
    fn url_token_stays_intact_when_it_fits() {
        let url = "https://example.com/a-b-c";
        let lines = plain(wrap_line(
            Line::from(format!("see {url} after")),
            Some(url.width()),
        ));
        assert!(lines.iter().any(|line| line == url));
        assert!(!lines.iter().any(|line| line == "https://example.com/a-"));
    }

    #[test]
    fn long_url_hard_breaks_only_when_it_cannot_fit() {
        let lines = plain(wrap_line(
            Line::from("https://example.com/super/long/path"),
            Some(12),
        ));
        assert!(lines.len() > 1);
        assert_eq!(lines[0], "https://exam");
    }

    #[test]
    fn preserves_indentation_and_multi_space_runs_when_wrapping() {
        let lines = plain(wrap_line(Line::from("    key  =  value"), Some(12)));
        assert_eq!(lines, vec!["    key  =", "  value"]);
    }

    #[test]
    fn preserves_span_style_when_wrapping() {
        let line = Line::from(vec![Span::styled(
            "alpha beta",
            Style::default().fg(ratatui::style::Color::Green),
        )]);
        let lines = wrap_line(line, Some(5));
        assert_eq!(line_to_plain(&lines[0]), "alpha");
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(ratatui::style::Color::Green)
        );
    }
}
