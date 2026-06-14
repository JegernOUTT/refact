use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::wrapping::wrap_line;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiffKind {
    File,
    Hunk,
    Add,
    Delete,
    Context,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiffToken {
    text: String,
    word_index: Option<usize>,
}

pub fn is_unified_diff(text: &str) -> bool {
    let mut has_file = false;
    let mut has_hunk = false;
    for line in text.lines() {
        if line.starts_with("diff --git ") || line.starts_with("--- ") || line.starts_with("+++ ") {
            has_file = true;
        }
        if line.starts_with("@@") {
            has_hunk = true;
        }
    }
    has_file && has_hunk
}

pub fn render_unified_diff(
    text: &str,
    width: Option<usize>,
    color_enabled: bool,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let lines = text.lines().collect::<Vec<_>>();
    let mut idx = 0usize;
    while idx < lines.len() {
        let line = lines[idx];
        let kind = classify(line);
        if kind == DiffKind::Delete
            && idx + 1 < lines.len()
            && classify(lines[idx + 1]) == DiffKind::Add
        {
            out.extend(render_changed_pair(
                line,
                lines[idx + 1],
                width,
                color_enabled,
            ));
            idx += 2;
            continue;
        }
        out.extend(render_diff_line(line, kind, width, color_enabled));
        idx += 1;
    }
    if out.is_empty() {
        out.push(Line::default());
    }
    out
}

fn render_changed_pair(
    removed: &str,
    added: &str,
    width: Option<usize>,
    color_enabled: bool,
) -> Vec<Line<'static>> {
    let delete_style = style_for(DiffKind::Delete, color_enabled);
    let add_style = style_for(DiffKind::Add, color_enabled);
    let removed_content = removed.strip_prefix('-').unwrap_or(removed);
    let added_content = added.strip_prefix('+').unwrap_or(added);
    let (removed_spans, added_spans) =
        word_diff_spans(removed_content, added_content, delete_style, add_style);
    let mut out = Vec::new();
    out.extend(wrap_line(
        line_from_parts(prefix_for(DiffKind::Delete), delete_style, removed_spans),
        width,
    ));
    out.extend(wrap_line(
        line_from_parts(prefix_for(DiffKind::Add), add_style, added_spans),
        width,
    ));
    out
}

fn render_diff_line(
    line: &str,
    kind: DiffKind,
    width: Option<usize>,
    color_enabled: bool,
) -> Vec<Line<'static>> {
    let style = style_for(kind, color_enabled);
    let prefix = prefix_for(kind);
    let content = if matches!(kind, DiffKind::Add | DiffKind::Delete) && line.len() > 1 {
        &line[1..]
    } else {
        line
    };
    wrap_line(
        Line::from(vec![
            Span::styled(prefix.to_string(), style),
            Span::styled(content.to_string(), style),
        ]),
        width,
    )
}

fn line_from_parts(
    prefix: &str,
    prefix_style: Style,
    content_spans: Vec<Span<'static>>,
) -> Line<'static> {
    let mut spans = vec![Span::styled(prefix.to_string(), prefix_style)];
    spans.extend(content_spans);
    Line::from(spans)
}

fn word_diff_spans(
    removed: &str,
    added: &str,
    delete_style: Style,
    add_style: Style,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let (removed_tokens, removed_words) = tokenize_words(removed);
    let (added_tokens, added_words) = tokenize_words(added);
    let (removed_unchanged, added_unchanged) = unchanged_word_indices(&removed_words, &added_words);
    (
        spans_for_tokens(&removed_tokens, &removed_unchanged, delete_style),
        spans_for_tokens(&added_tokens, &added_unchanged, add_style),
    )
}

fn tokenize_words(text: &str) -> (Vec<DiffToken>, Vec<String>) {
    let mut tokens = Vec::new();
    let mut words = Vec::new();
    let mut current = String::new();
    let mut current_space = None::<bool>;
    for ch in text.chars() {
        let is_space = ch.is_whitespace();
        if current_space.is_some_and(|space| space != is_space) {
            push_token(
                &mut tokens,
                &mut words,
                std::mem::take(&mut current),
                current_space.unwrap(),
            );
        }
        current_space = Some(is_space);
        current.push(ch);
    }
    if let Some(is_space) = current_space {
        push_token(&mut tokens, &mut words, current, is_space);
    }
    (tokens, words)
}

fn push_token(tokens: &mut Vec<DiffToken>, words: &mut Vec<String>, text: String, is_space: bool) {
    if text.is_empty() {
        return;
    }
    let word_index = if is_space {
        None
    } else {
        let idx = words.len();
        words.push(text.clone());
        Some(idx)
    };
    tokens.push(DiffToken { text, word_index });
}

fn unchanged_word_indices(removed: &[String], added: &[String]) -> (Vec<bool>, Vec<bool>) {
    let mut dp = vec![vec![0usize; added.len() + 1]; removed.len() + 1];
    for i in (0..removed.len()).rev() {
        for j in (0..added.len()).rev() {
            dp[i][j] = if removed[i] == added[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut removed_unchanged = vec![false; removed.len()];
    let mut added_unchanged = vec![false; added.len()];
    let mut i = 0usize;
    let mut j = 0usize;
    while i < removed.len() && j < added.len() {
        if removed[i] == added[j] {
            removed_unchanged[i] = true;
            added_unchanged[j] = true;
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    (removed_unchanged, added_unchanged)
}

fn spans_for_tokens(
    tokens: &[DiffToken],
    unchanged: &[bool],
    base_style: Style,
) -> Vec<Span<'static>> {
    tokens
        .iter()
        .map(|token| {
            let style = token
                .word_index
                .filter(|idx| !unchanged.get(*idx).copied().unwrap_or(false))
                .map(|_| changed_span_style(base_style))
                .unwrap_or(base_style);
            Span::styled(token.text.clone(), style)
        })
        .collect()
}

fn changed_span_style(style: Style) -> Style {
    style
        .add_modifier(Modifier::BOLD)
        .add_modifier(Modifier::REVERSED)
}

fn classify(line: &str) -> DiffKind {
    if line.starts_with("@@") {
        DiffKind::Hunk
    } else if line.starts_with("diff --git ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
    {
        DiffKind::File
    } else if line.starts_with('+') {
        DiffKind::Add
    } else if line.starts_with('-') {
        DiffKind::Delete
    } else {
        DiffKind::Context
    }
}

fn prefix_for(kind: DiffKind) -> &'static str {
    match kind {
        DiffKind::Add => "+ ",
        DiffKind::Delete => "- ",
        DiffKind::Hunk => "@ ",
        DiffKind::File => "  ",
        DiffKind::Context => "  ",
    }
}

fn style_for(kind: DiffKind, color_enabled: bool) -> Style {
    if !color_enabled {
        return match kind {
            DiffKind::File | DiffKind::Hunk => Style::default().add_modifier(Modifier::BOLD),
            _ => Style::default(),
        };
    }
    match kind {
        DiffKind::File => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        DiffKind::Hunk => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        DiffKind::Add => Style::default().fg(Color::Green),
        DiffKind::Delete => Style::default().fg(Color::Red),
        DiffKind::Context => Style::default().fg(Color::DarkGray),
    }
}

#[cfg(test)]
mod tests {
    use super::super::wrapping::line_to_plain;
    use super::*;

    fn plain(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(line_to_plain).collect()
    }

    #[test]
    fn detects_unified_diff() {
        assert!(is_unified_diff("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new"));
        assert!(!is_unified_diff("+not enough"));
    }

    #[test]
    fn renders_diff_prefixes_and_styles() {
        let lines = render_unified_diff(
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n same",
            Some(80),
            true,
        );
        let rendered = plain(&lines);
        assert!(rendered.iter().any(|line| line == "- old"));
        assert!(rendered.iter().any(|line| line == "+ new"));
        let add = lines
            .iter()
            .find(|line| line_to_plain(line) == "+ new")
            .unwrap();
        assert_eq!(add.spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn highlights_changed_words_in_adjacent_delete_add_pair() {
        let lines = render_unified_diff(
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-let status = \"slow\";\n+let status = \"fast\";",
            Some(80),
            true,
        );
        let removed = lines
            .iter()
            .find(|line| line_to_plain(line) == "- let status = \"slow\";")
            .unwrap();
        let added = lines
            .iter()
            .find(|line| line_to_plain(line) == "+ let status = \"fast\";")
            .unwrap();
        let removed_changed = removed
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "\"slow\";")
            .unwrap();
        let added_changed = added
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "\"fast\";")
            .unwrap();
        assert!(removed_changed.style.add_modifier.contains(Modifier::BOLD));
        assert!(removed_changed
            .style
            .add_modifier
            .contains(Modifier::REVERSED));
        assert!(added_changed.style.add_modifier.contains(Modifier::BOLD));
        assert!(added_changed
            .style
            .add_modifier
            .contains(Modifier::REVERSED));
    }

    #[test]
    fn no_color_diff_keeps_prefixes() {
        let lines = render_unified_diff("@@ -1 +1 @@\n-old\n+new", Some(80), false);
        assert!(plain(&lines).contains(&"- old".to_string()));
        assert!(plain(&lines).contains(&"+ new".to_string()));
    }
}
