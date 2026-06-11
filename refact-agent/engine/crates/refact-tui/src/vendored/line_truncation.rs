// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use ratatui::text::Line;
use ratatui::text::Span;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

pub fn line_width(line: &Line<'_>) -> usize {
    line.iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

pub fn truncate_line_to_width(line: Line<'static>, max_width: usize) -> Line<'static> {
    if max_width == 0 {
        return Line::from(Vec::<Span<'static>>::new());
    }

    let Line {
        style,
        alignment,
        spans,
    } = line;
    let mut used = 0usize;
    let mut spans_out: Vec<Span<'static>> = Vec::with_capacity(spans.len());

    for span in spans {
        let span_width = UnicodeWidthStr::width(span.content.as_ref());

        if span_width == 0 {
            spans_out.push(span);
            continue;
        }

        if used >= max_width {
            break;
        }

        if used + span_width <= max_width {
            used += span_width;
            spans_out.push(span);
            continue;
        }

        let style = span.style;
        let text = span.content.as_ref();
        let mut end_idx = 0usize;
        for (idx, ch) in text.char_indices() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + ch_width > max_width {
                break;
            }
            end_idx = idx + ch.len_utf8();
            used += ch_width;
        }

        if end_idx > 0 {
            spans_out.push(Span::styled(text[..end_idx].to_string(), style));
        }

        break;
    }

    Line {
        style,
        alignment,
        spans: spans_out,
    }
}

pub fn truncate_line_with_ellipsis_if_overflow(
    line: Line<'static>,
    max_width: usize,
) -> Line<'static> {
    if max_width == 0 {
        return Line::from(Vec::<Span<'static>>::new());
    }

    if line_width(&line) <= max_width {
        return line;
    }

    let truncated = truncate_line_to_width(line, max_width.saturating_sub(1));
    let Line {
        style,
        alignment,
        mut spans,
    } = truncated;
    let ellipsis_style = spans.last().map(|span| span.style).unwrap_or_default();
    spans.push(Span::styled("…", ellipsis_style));
    Line {
        style,
        alignment,
        spans,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Stylize;

    #[test]
    fn truncates_wide_chars_without_overflow() {
        let line = Line::from(vec![Span::styled(
            "ab猫cd",
            ratatui::style::Style::new().bold(),
        )]);
        let out = truncate_line_with_ellipsis_if_overflow(line, 5);
        assert_eq!(line_width(&out), 5);
        assert_eq!(
            out.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>(),
            "ab猫…"
        );
        assert!(out.spans[0]
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD));
    }
}
