use unicode_segmentation::UnicodeSegmentation;

use crate::render::wrapping::take_prefix_by_width;

pub(super) fn indices(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.grapheme_indices(true)
}

pub(super) fn clamp_boundary(text: &str, target: usize) -> usize {
    if target >= text.len() {
        return text.len();
    }
    indices(text)
        .take_while(|(index, _)| *index <= target)
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0)
}

pub(super) fn previous_boundary(text: &str, cursor: usize) -> Option<usize> {
    let cursor = clamp_boundary(text, cursor);
    (cursor > 0).then(|| indices(&text[..cursor]).last().map(|(index, _)| index))?
}

pub(super) fn next_boundary(text: &str, cursor: usize) -> Option<usize> {
    let cursor = clamp_boundary(text, cursor);
    if cursor >= text.len() {
        return None;
    }
    indices(&text[cursor..])
        .nth(1)
        .map(|(index, _)| cursor + index)
        .or(Some(text.len()))
}

pub(super) fn line_and_column(text: &str, cursor: usize) -> (usize, usize) {
    let cursor = clamp_boundary(text, cursor);
    let before = &text[..cursor];
    let line = before.bytes().filter(|byte| *byte == b'\n').count();
    let start = before.rfind('\n').map_or(0, |index| index + 1);
    (line, display_width(&before[start..]))
}

pub(super) fn line_count(text: &str) -> usize {
    text.bytes().filter(|byte| *byte == b'\n').count() + 1
}

pub(super) fn offset_for_line_column(
    text: &str,
    target_line: usize,
    target_column: usize,
) -> usize {
    let Some(start) = line_start(text, target_line) else {
        return text.len();
    };
    let end = text[start..]
        .find('\n')
        .map_or(text.len(), |index| start + index);
    let mut column = 0usize;
    for (index, grapheme) in indices(&text[start..end]) {
        let width = grapheme_width(grapheme);
        if column.saturating_add(width) > target_column {
            return start + index;
        }
        column = column.saturating_add(width);
        if column == target_column {
            return start + index + grapheme.len();
        }
    }
    end
}

pub(super) fn display_width(text: &str) -> usize {
    text.graphemes(true)
        .map(grapheme_width)
        .fold(0, usize::saturating_add)
}

pub(super) fn grapheme_width(grapheme: &str) -> usize {
    take_prefix_by_width(grapheme, usize::MAX).2
}

fn line_start(text: &str, target_line: usize) -> Option<usize> {
    let mut start = 0;
    for _ in 0..target_line {
        start += text[start..].find('\n')? + 1;
    }
    Some(start)
}
