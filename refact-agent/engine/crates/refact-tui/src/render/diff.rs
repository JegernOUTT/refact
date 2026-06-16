use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use crate::diff_model::FileChange;
use crate::terminal_palette::{stdout_color_level, StdoutColorLevel};

use super::highlight::{exceeds_highlight_limits, highlight_code_to_styled_spans};
use super::wrapping::wrap_line;

const WORD_DIFF_MAX_CHARS: usize = 4096;
const WORD_DIFF_MAX_WORDS: usize = 512;
const WORD_DIFF_MAX_CELLS: usize = 65_536;
const TAB_WIDTH: usize = 4;
const ADD_BG_RGB: (u8, u8, u8) = (33, 58, 43);
const DEL_BG_RGB: (u8, u8, u8) = (74, 34, 29);
const ADD_BG_256: u8 = 22;
const DEL_BG_256: u8 = 52;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineType {
    Insert,
    Delete,
    Context,
}

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
    start: usize,
    end: usize,
}

#[derive(Clone)]
struct NumberedLine {
    line_number: usize,
    kind: DiffLineType,
    content: String,
    syntax_index: usize,
}

#[derive(Clone)]
enum HunkItem {
    Numbered(NumberedLine),
    Meta(String),
}

#[derive(Clone, Copy)]
struct HunkHeader {
    old_start: usize,
    new_start: usize,
}

#[derive(Clone, Copy)]
struct DiffStyleContext {
    color_enabled: bool,
    color_level: StdoutColorLevel,
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
    render_unified_diff_with_lang(text, width, color_enabled, detect_lang_from_diff(text))
}

pub fn create_diff_summary(
    changes: &HashMap<PathBuf, FileChange>,
    cwd: &Path,
    wrap_cols: usize,
) -> Vec<Line<'static>> {
    let mut rows = changes
        .iter()
        .map(|(path, change)| {
            let (added, removed) = match change {
                FileChange::Add { content } => (content.lines().count(), 0),
                FileChange::Delete { content } => (0, content.lines().count()),
                FileChange::Update { unified_diff, .. } => {
                    calculate_add_remove_from_diff(unified_diff)
                }
            };
            let move_path = match change {
                FileChange::Update {
                    move_path: Some(path),
                    ..
                } => Some(path.clone()),
                _ => None,
            };
            (path.clone(), move_path, added, removed, change.clone())
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|(path, _, _, _, _)| path.clone());

    let total_added = rows.iter().map(|(_, _, added, _, _)| *added).sum::<usize>();
    let total_removed = rows
        .iter()
        .map(|(_, _, _, removed, _)| *removed)
        .sum::<usize>();
    let mut out = Vec::new();
    let mut header = vec![Span::styled(
        "• ",
        Style::default().add_modifier(Modifier::DIM),
    )];
    if let [(path, move_path, added, removed, change)] = rows.as_slice() {
        let verb = match change {
            FileChange::Add { .. } => "Added",
            FileChange::Delete { .. } => "Deleted",
            FileChange::Update { .. } => "Edited",
        };
        header.push(Span::styled(
            verb.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        header.push(Span::raw(" "));
        header.extend(path_spans(path, move_path.as_ref(), cwd));
        header.push(Span::raw(" "));
        header.extend(line_count_summary_spans(*added, *removed));
    } else {
        let noun = if rows.len() == 1 { "file" } else { "files" };
        header.push(Span::styled(
            "Edited".to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        header.push(Span::raw(format!(" {} {} ", rows.len(), noun)));
        header.extend(line_count_summary_spans(total_added, total_removed));
    }
    out.push(Line::from(header));

    let color_enabled = super::color_enabled_from_env();
    let skip_file_header = rows.len() == 1;
    for (idx, (path, move_path, added, removed, change)) in rows.into_iter().enumerate() {
        if idx > 0 {
            out.push(Line::default());
        }
        if !skip_file_header {
            let mut file_header = vec![Span::styled(
                "  └ ",
                Style::default().add_modifier(Modifier::DIM),
            )];
            file_header.extend(path_spans(&path, move_path.as_ref(), cwd));
            file_header.push(Span::raw(" "));
            file_header.extend(line_count_summary_spans(added, removed));
            out.push(Line::from(file_header));
        }
        let lang_path = move_path.as_ref().unwrap_or(&path);
        let lang = detect_lang_for_path(lang_path);
        let rendered = render_file_change(
            &change,
            Some(wrap_cols.saturating_sub(4).max(8)),
            color_enabled,
            lang,
        );
        out.extend(indent_lines(rendered, "    "));
    }

    out
}

pub fn display_path_for(path: &Path, cwd: &Path) -> String {
    if path.is_relative() {
        return path.display().to_string();
    }
    if let Ok(stripped) = path.strip_prefix(cwd) {
        let rendered = stripped.display().to_string();
        return if rendered.is_empty() {
            ".".to_string()
        } else {
            rendered
        };
    }
    if let Some(stripped) = relativize_to_home(path) {
        let rendered = stripped.display().to_string();
        return if rendered.is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rendered)
        };
    }
    path.display().to_string()
}

pub fn calculate_add_remove_from_diff(diff: &str) -> (usize, usize) {
    diff.lines().fold((0, 0), |(added, removed), line| {
        if line.starts_with("+++") || line.starts_with("---") {
            (added, removed)
        } else if line.starts_with('+') {
            (added + 1, removed)
        } else if line.starts_with('-') {
            (added, removed + 1)
        } else {
            (added, removed)
        }
    })
}

fn render_unified_diff_with_lang(
    text: &str,
    width: Option<usize>,
    color_enabled: bool,
    lang: Option<String>,
) -> Vec<Line<'static>> {
    let lines = text.lines().collect::<Vec<_>>();
    let mut out = Vec::new();
    let style_context = DiffStyleContext {
        color_enabled,
        color_level: stdout_color_level(),
    };
    let gutter_width = line_number_width(max_line_number_in_diff(&lines));
    let mut idx = 0usize;
    let mut seen_hunk = false;
    while idx < lines.len() {
        let line = lines[idx];
        if let Some(header) = parse_hunk_header(line) {
            if seen_hunk {
                out.push(render_hunk_gap(gutter_width, style_context));
            }
            seen_hunk = true;
            out.extend(render_meta_line(line, DiffKind::Hunk, width, color_enabled));
            idx += 1;
            let hunk_start = idx;
            while idx < lines.len()
                && parse_hunk_header(lines[idx]).is_none()
                && !is_file_meta_line(lines[idx])
            {
                idx += 1;
            }
            out.extend(render_hunk_lines(
                &lines[hunk_start..idx],
                header,
                gutter_width,
                width,
                style_context,
                lang.as_deref(),
            ));
            continue;
        }
        if line.starts_with("diff --git ") {
            seen_hunk = false;
        }
        out.extend(render_meta_line(line, classify(line), width, color_enabled));
        idx += 1;
    }
    if out.is_empty() {
        out.push(Line::default());
    }
    out
}

fn render_file_change(
    change: &FileChange,
    width: Option<usize>,
    color_enabled: bool,
    lang: Option<String>,
) -> Vec<Line<'static>> {
    match change {
        FileChange::Add { content } => render_whole_file_diff(
            content,
            DiffLineType::Insert,
            width,
            color_enabled,
            lang.as_deref(),
        ),
        FileChange::Delete { content } => render_whole_file_diff(
            content,
            DiffLineType::Delete,
            width,
            color_enabled,
            lang.as_deref(),
        ),
        FileChange::Update { unified_diff, .. } => {
            render_unified_diff_with_lang(unified_diff, width, color_enabled, lang)
        }
    }
}

fn render_whole_file_diff(
    content: &str,
    kind: DiffLineType,
    width: Option<usize>,
    color_enabled: bool,
    lang: Option<&str>,
) -> Vec<Line<'static>> {
    let style_context = DiffStyleContext {
        color_enabled,
        color_level: stdout_color_level(),
    };
    let content_lines = content.lines().collect::<Vec<_>>();
    let syntax_lines = syntax_lines_for(&content_lines, lang, color_enabled);
    let gutter_width = line_number_width(content_lines.len());
    let mut out = Vec::new();
    for (idx, content) in content_lines.iter().enumerate() {
        let syntax_spans = syntax_lines.as_ref().and_then(|lines| lines.get(idx));
        out.extend(render_numbered_diff_line(
            idx + 1,
            kind,
            styled_content_spans(content, kind, syntax_spans, color_enabled),
            width,
            gutter_width,
            style_context,
        ));
    }
    if out.is_empty() {
        out.push(Line::default());
    }
    out
}

fn render_hunk_lines(
    raw_lines: &[&str],
    header: HunkHeader,
    gutter_width: usize,
    width: Option<usize>,
    style_context: DiffStyleContext,
    lang: Option<&str>,
) -> Vec<Line<'static>> {
    let mut old_ln = header.old_start;
    let mut new_ln = header.new_start;
    let mut syntax_index = 0usize;
    let mut items = Vec::new();
    let mut content_lines = Vec::new();

    for raw in raw_lines {
        let Some((kind, content)) = hunk_line_content(raw) else {
            items.push(HunkItem::Meta((*raw).to_string()));
            continue;
        };
        let line_number = match kind {
            DiffLineType::Insert => {
                let current = new_ln;
                new_ln += 1;
                current
            }
            DiffLineType::Delete => {
                let current = old_ln;
                old_ln += 1;
                current
            }
            DiffLineType::Context => {
                let current = new_ln;
                old_ln += 1;
                new_ln += 1;
                current
            }
        };
        content_lines.push(content.to_string());
        items.push(HunkItem::Numbered(NumberedLine {
            line_number,
            kind,
            content: content.to_string(),
            syntax_index,
        }));
        syntax_index += 1;
    }

    let syntax_lines = syntax_lines_for_refs(&content_lines, lang, style_context.color_enabled);
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < items.len() {
        match &items[idx] {
            HunkItem::Meta(text) => {
                out.extend(render_meta_line(
                    text,
                    DiffKind::Context,
                    width,
                    style_context.color_enabled,
                ));
                idx += 1;
            }
            HunkItem::Numbered(removed)
                if removed.kind == DiffLineType::Delete
                    && matches!(items.get(idx + 1), Some(HunkItem::Numbered(next)) if next.kind == DiffLineType::Insert) =>
            {
                let HunkItem::Numbered(added) = &items[idx + 1] else {
                    unreachable!();
                };
                let removed_syntax = syntax_lines
                    .as_ref()
                    .and_then(|lines| lines.get(removed.syntax_index));
                let added_syntax = syntax_lines
                    .as_ref()
                    .and_then(|lines| lines.get(added.syntax_index));
                let (removed_spans, added_spans) = word_diff_spans_with_syntax(
                    &removed.content,
                    &added.content,
                    content_style_for(DiffLineType::Delete, style_context.color_enabled),
                    content_style_for(DiffLineType::Insert, style_context.color_enabled),
                    removed_syntax,
                    added_syntax,
                );
                out.extend(render_numbered_diff_line(
                    removed.line_number,
                    DiffLineType::Delete,
                    removed_spans,
                    width,
                    gutter_width,
                    style_context,
                ));
                out.extend(render_numbered_diff_line(
                    added.line_number,
                    DiffLineType::Insert,
                    added_spans,
                    width,
                    gutter_width,
                    style_context,
                ));
                idx += 2;
            }
            HunkItem::Numbered(line) => {
                let syntax_spans = syntax_lines
                    .as_ref()
                    .and_then(|lines| lines.get(line.syntax_index));
                out.extend(render_numbered_diff_line(
                    line.line_number,
                    line.kind,
                    styled_content_spans(
                        &line.content,
                        line.kind,
                        syntax_spans,
                        style_context.color_enabled,
                    ),
                    width,
                    gutter_width,
                    style_context,
                ));
                idx += 1;
            }
        }
    }
    out
}

fn render_numbered_diff_line(
    line_number: usize,
    kind: DiffLineType,
    content_spans: Vec<Span<'static>>,
    width: Option<usize>,
    gutter_width: usize,
    style_context: DiffStyleContext,
) -> Vec<Line<'static>> {
    let prefix_width = gutter_width.max(1) + 2;
    let chunks = if let Some(width) = width.filter(|width| *width > prefix_width) {
        wrap_styled_spans(&content_spans, width.saturating_sub(prefix_width).max(1))
    } else {
        vec![content_spans]
    };
    let gutter_style = gutter_style(style_context.color_enabled);
    let sign_style = sign_style_for(kind, style_context.color_enabled);
    let line_style = line_style_for(kind, style_context);
    let sign = sign_for(kind).to_string();
    chunks
        .into_iter()
        .enumerate()
        .map(|(idx, chunk)| {
            let mut spans = Vec::new();
            if idx == 0 {
                spans.push(Span::styled(
                    format!("{line_number:>gutter_width$} "),
                    gutter_style,
                ));
                spans.push(Span::styled(sign.clone(), sign_style));
            } else {
                spans.push(Span::styled(
                    format!("{:gutter_width$}  ", ""),
                    gutter_style,
                ));
            }
            spans.extend(chunk);
            let mut line = Line::from(spans);
            line.style = line_style;
            line
        })
        .collect()
}

fn render_hunk_gap(gutter_width: usize, style_context: DiffStyleContext) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:gutter_width$} ", ""),
            gutter_style(style_context.color_enabled),
        ),
        Span::styled("⋮", Style::default().add_modifier(Modifier::DIM)),
    ])
}

fn render_meta_line(
    line: &str,
    kind: DiffKind,
    width: Option<usize>,
    color_enabled: bool,
) -> Vec<Line<'static>> {
    let style = style_for(kind, color_enabled);
    wrap_line(
        Line::from(vec![Span::styled(line.to_string(), style)]),
        width,
    )
}

fn line_count_summary_spans(added: usize, removed: usize) -> Vec<Span<'static>> {
    vec![
        Span::raw("("),
        Span::styled(format!("+{added}"), style_for(DiffKind::Add, true)),
        Span::raw(" "),
        Span::styled(format!("-{removed}"), style_for(DiffKind::Delete, true)),
        Span::raw(")"),
    ]
}

fn path_spans(path: &Path, move_path: Option<&PathBuf>, cwd: &Path) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw(display_path_for(path, cwd))];
    if let Some(move_path) = move_path {
        spans.push(Span::raw(format!(
            " → {}",
            display_path_for(move_path, cwd)
        )));
    }
    spans
}

fn indent_lines(lines: Vec<Line<'static>>, prefix: &str) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|mut line| {
            let mut spans = vec![Span::raw(prefix.to_string())];
            spans.extend(line.spans);
            line.spans = spans;
            line
        })
        .collect()
}

fn styled_content_spans(
    content: &str,
    kind: DiffLineType,
    syntax_spans: Option<&Vec<Span<'static>>>,
    color_enabled: bool,
) -> Vec<Span<'static>> {
    if let Some(syntax_spans) = syntax_spans.filter(|spans| spans_plain(spans) == content) {
        return syntax_spans
            .iter()
            .map(|span| {
                let style = if kind == DiffLineType::Delete {
                    span.style.add_modifier(Modifier::DIM)
                } else {
                    span.style
                };
                Span::styled(span.content.clone().into_owned(), style)
            })
            .collect();
    }
    vec![Span::styled(
        content.to_string(),
        content_style_for(kind, color_enabled),
    )]
}

fn word_diff_spans_with_syntax(
    removed: &str,
    added: &str,
    delete_style: Style,
    add_style: Style,
    removed_syntax: Option<&Vec<Span<'static>>>,
    added_syntax: Option<&Vec<Span<'static>>>,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    if removed.len() > WORD_DIFF_MAX_CHARS || added.len() > WORD_DIFF_MAX_CHARS {
        return line_level_spans(removed, added, delete_style, add_style);
    }
    let (removed_tokens, removed_words) = tokenize_words(removed);
    let (added_tokens, added_words) = tokenize_words(added);
    if removed_words.len() > WORD_DIFF_MAX_WORDS
        || added_words.len() > WORD_DIFF_MAX_WORDS
        || removed_words.len().saturating_mul(added_words.len()) > WORD_DIFF_MAX_CELLS
    {
        return line_level_spans(removed, added, delete_style, add_style);
    }
    let (removed_unchanged, added_unchanged) = unchanged_word_indices(&removed_words, &added_words);
    (
        spans_for_tokens(
            &removed_tokens,
            &removed_unchanged,
            delete_style,
            removed_syntax.filter(|spans| spans_plain(spans) == removed),
            true,
        ),
        spans_for_tokens(
            &added_tokens,
            &added_unchanged,
            add_style,
            added_syntax.filter(|spans| spans_plain(spans) == added),
            false,
        ),
    )
}

fn line_level_spans(
    removed: &str,
    added: &str,
    delete_style: Style,
    add_style: Style,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    (
        vec![Span::styled(removed.to_string(), delete_style)],
        vec![Span::styled(added.to_string(), add_style)],
    )
}

fn tokenize_words(text: &str) -> (Vec<DiffToken>, Vec<String>) {
    let mut tokens = Vec::new();
    let mut words = Vec::new();
    let mut current = String::new();
    let mut current_space = None::<bool>;
    let mut current_start = 0usize;
    for (idx, ch) in text.char_indices() {
        let is_space = ch.is_whitespace();
        if current_space.is_some_and(|space| space != is_space) {
            push_token(
                &mut tokens,
                &mut words,
                std::mem::take(&mut current),
                current_space.unwrap(),
                current_start,
                idx,
            );
            current_start = idx;
        }
        current_space = Some(is_space);
        current.push(ch);
    }
    if let Some(is_space) = current_space {
        push_token(
            &mut tokens,
            &mut words,
            current,
            is_space,
            current_start,
            text.len(),
        );
    }
    (tokens, words)
}

fn push_token(
    tokens: &mut Vec<DiffToken>,
    words: &mut Vec<String>,
    text: String,
    is_space: bool,
    start: usize,
    end: usize,
) {
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
    tokens.push(DiffToken {
        text,
        word_index,
        start,
        end,
    });
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
    syntax_spans: Option<&Vec<Span<'static>>>,
    dim_syntax: bool,
) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    for token in tokens {
        let changed = token
            .word_index
            .is_some_and(|idx| !unchanged.get(idx).copied().unwrap_or(false));
        let mut spans = if let Some(syntax_spans) = syntax_spans {
            spans_from_syntax_range(syntax_spans, token, base_style)
        } else {
            vec![Span::styled(token.text.clone(), base_style)]
        };
        for span in &mut spans {
            if dim_syntax && syntax_spans.is_some() {
                span.style = span.style.add_modifier(Modifier::DIM);
            }
            if changed {
                span.style = changed_span_style(span.style);
            }
        }
        out.extend(spans);
    }
    out
}

fn spans_from_syntax_range(
    syntax_spans: &[Span<'static>],
    token: &DiffToken,
    fallback_style: Style,
) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for span in syntax_spans {
        let text = span.content.as_ref();
        let span_start = cursor;
        let span_end = cursor + text.len();
        cursor = span_end;
        let start = token.start.max(span_start);
        let end = token.end.min(span_end);
        if start >= end {
            continue;
        }
        out.push(Span::styled(
            text[start - span_start..end - span_start].to_string(),
            span.style,
        ));
    }
    if out.is_empty() {
        out.push(Span::styled(token.text.clone(), fallback_style));
    }
    out
}

fn changed_span_style(style: Style) -> Style {
    style
        .add_modifier(Modifier::BOLD)
        .add_modifier(Modifier::REVERSED)
}

fn syntax_lines_for(
    lines: &[&str],
    lang: Option<&str>,
    color_enabled: bool,
) -> Option<Vec<Vec<Span<'static>>>> {
    let owned = lines
        .iter()
        .map(|line| (*line).to_string())
        .collect::<Vec<_>>();
    syntax_lines_for_refs(&owned, lang, color_enabled)
}

fn syntax_lines_for_refs(
    lines: &[String],
    lang: Option<&str>,
    color_enabled: bool,
) -> Option<Vec<Vec<Span<'static>>>> {
    if !color_enabled || lines.is_empty() {
        return None;
    }
    let lang = lang?;
    let total_bytes = lines.iter().map(String::len).sum::<usize>();
    if exceeds_highlight_limits(total_bytes, lines.len()) {
        return None;
    }
    let text = lines.join("\n");
    let syntax_lines = highlight_code_to_styled_spans(&text, lang)?;
    (syntax_lines.len() == lines.len()).then_some(syntax_lines)
}

fn spans_plain(spans: &[Span<'static>]) -> String {
    spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn wrap_styled_spans(spans: &[Span<'static>], max_cols: usize) -> Vec<Vec<Span<'static>>> {
    let max_cols = max_cols.max(1);
    let mut result = Vec::new();
    let mut current_line = Vec::new();
    let mut col = 0usize;

    for span in spans {
        let style = span.style;
        let mut remaining = span.content.as_ref();
        while !remaining.is_empty() {
            let mut byte_end = 0usize;
            let mut chars_col = 0usize;
            for ch in remaining.chars() {
                let width = ch.width().unwrap_or(if ch == '\t' { TAB_WIDTH } else { 0 });
                if col + chars_col + width > max_cols {
                    break;
                }
                byte_end += ch.len_utf8();
                chars_col += width;
            }
            if byte_end == 0 {
                if !current_line.is_empty() {
                    result.push(std::mem::take(&mut current_line));
                    col = 0;
                }
                let Some(ch) = remaining.chars().next() else {
                    break;
                };
                let ch_len = ch.len_utf8();
                current_line.push(Span::styled(remaining[..ch_len].to_string(), style));
                col = ch.width().unwrap_or(if ch == '\t' { TAB_WIDTH } else { 1 });
                remaining = &remaining[ch_len..];
                continue;
            }
            let (chunk, rest) = remaining.split_at(byte_end);
            current_line.push(Span::styled(chunk.to_string(), style));
            col += chars_col;
            remaining = rest;
            if col >= max_cols {
                result.push(std::mem::take(&mut current_line));
                col = 0;
            }
        }
    }

    if !current_line.is_empty() || result.is_empty() {
        result.push(current_line);
    }
    result
}

fn max_line_number_in_diff(lines: &[&str]) -> usize {
    let mut max_line_number = 0usize;
    let mut idx = 0usize;
    while idx < lines.len() {
        let Some(header) = parse_hunk_header(lines[idx]) else {
            idx += 1;
            continue;
        };
        let mut old_ln = header.old_start;
        let mut new_ln = header.new_start;
        idx += 1;
        while idx < lines.len()
            && parse_hunk_header(lines[idx]).is_none()
            && !is_file_meta_line(lines[idx])
        {
            if let Some((kind, _)) = hunk_line_content(lines[idx]) {
                match kind {
                    DiffLineType::Insert => {
                        max_line_number = max_line_number.max(new_ln);
                        new_ln += 1;
                    }
                    DiffLineType::Delete => {
                        max_line_number = max_line_number.max(old_ln);
                        old_ln += 1;
                    }
                    DiffLineType::Context => {
                        max_line_number = max_line_number.max(new_ln);
                        old_ln += 1;
                        new_ln += 1;
                    }
                }
            }
            idx += 1;
        }
    }
    max_line_number
}

fn line_number_width(max_line_number: usize) -> usize {
    if max_line_number == 0 {
        1
    } else {
        max_line_number.to_string().len()
    }
}

fn parse_hunk_header(line: &str) -> Option<HunkHeader> {
    if !line.starts_with("@@") {
        return None;
    }
    let end = line[2..].find("@@")? + 2;
    let header = &line[..end];
    let mut old_start = None;
    let mut new_start = None;
    for part in header.split_whitespace() {
        if let Some(rest) = part.strip_prefix('-') {
            old_start = parse_range_start(rest);
        } else if let Some(rest) = part.strip_prefix('+') {
            new_start = parse_range_start(rest);
        }
    }
    Some(HunkHeader {
        old_start: old_start?,
        new_start: new_start?,
    })
}

fn parse_range_start(text: &str) -> Option<usize> {
    let start = text.split(',').next()?;
    if start.chars().all(|ch| ch.is_ascii_digit()) {
        start.parse().ok()
    } else {
        None
    }
}

fn hunk_line_content(line: &str) -> Option<(DiffLineType, &str)> {
    if let Some(content) = line.strip_prefix('+') {
        Some((DiffLineType::Insert, content))
    } else if let Some(content) = line.strip_prefix('-') {
        Some((DiffLineType::Delete, content))
    } else if let Some(content) = line.strip_prefix(' ') {
        Some((DiffLineType::Context, content))
    } else if line.is_empty() {
        Some((DiffLineType::Context, ""))
    } else {
        None
    }
}

fn is_file_meta_line(line: &str) -> bool {
    line.starts_with("diff --git ") || line.starts_with("--- ") || line.starts_with("+++ ")
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

fn detect_lang_from_diff(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        path_from_file_header(line).and_then(|path| detect_lang_for_path(Path::new(path)))
    })
}

fn path_from_file_header(line: &str) -> Option<&str> {
    let path = line
        .strip_prefix("+++ ")
        .or_else(|| line.strip_prefix("--- "))?;
    let path = path.split_whitespace().next()?;
    if path == "/dev/null" {
        return None;
    }
    Some(
        path.strip_prefix("a/")
            .or_else(|| path.strip_prefix("b/"))
            .unwrap_or(path),
    )
}

fn detect_lang_for_path(path: &Path) -> Option<String> {
    path.extension()?.to_str().map(str::to_string)
}

fn relativize_to_home(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    path.strip_prefix(home).ok().map(Path::to_path_buf)
}

fn sign_for(kind: DiffLineType) -> char {
    match kind {
        DiffLineType::Insert => '+',
        DiffLineType::Delete => '-',
        DiffLineType::Context => ' ',
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

fn gutter_style(color_enabled: bool) -> Style {
    if color_enabled {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    }
}

fn sign_style_for(kind: DiffLineType, color_enabled: bool) -> Style {
    content_style_for(kind, color_enabled)
}

fn content_style_for(kind: DiffLineType, color_enabled: bool) -> Style {
    if !color_enabled {
        return Style::default();
    }
    match kind {
        DiffLineType::Insert => Style::default().fg(Color::Green),
        DiffLineType::Delete => Style::default().fg(Color::Red),
        DiffLineType::Context => Style::default(),
    }
}

fn line_style_for(kind: DiffLineType, context: DiffStyleContext) -> Style {
    if !context.color_enabled {
        return Style::default();
    }
    match (kind, context.color_level) {
        (DiffLineType::Insert, StdoutColorLevel::TrueColor) => {
            Style::default().bg(Color::Rgb(ADD_BG_RGB.0, ADD_BG_RGB.1, ADD_BG_RGB.2))
        }
        (DiffLineType::Delete, StdoutColorLevel::TrueColor) => {
            Style::default().bg(Color::Rgb(DEL_BG_RGB.0, DEL_BG_RGB.1, DEL_BG_RGB.2))
        }
        (DiffLineType::Insert, StdoutColorLevel::Ansi256) => {
            Style::default().bg(Color::Indexed(ADD_BG_256))
        }
        (DiffLineType::Delete, StdoutColorLevel::Ansi256) => {
            Style::default().bg(Color::Indexed(DEL_BG_256))
        }
        _ => Style::default(),
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
    fn renders_diff_gutter_signs_and_styles() {
        let lines = render_unified_diff(
            "--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-old\n+new\n same",
            Some(80),
            true,
        );
        let rendered = plain(&lines);
        assert!(rendered.iter().any(|line| line == "1 -old"));
        assert!(rendered.iter().any(|line| line == "1 +new"));
        assert!(rendered.iter().any(|line| line == "2  same"));
        let add = lines
            .iter()
            .find(|line| line_to_plain(line) == "1 +new")
            .unwrap();
        assert_eq!(add.spans[1].style.fg, Some(Color::Green));
    }

    #[test]
    fn renders_file_and_hunk_lines() {
        let lines = render_unified_diff("--- a/x\n+++ b/x\n@@ -1 +1 @@\n same", Some(80), false);
        let rendered = plain(&lines);
        assert!(rendered.contains(&"--- a/x".to_string()));
        assert!(rendered.contains(&"+++ b/x".to_string()));
        assert!(rendered.contains(&"@@ -1 +1 @@".to_string()));
        assert!(rendered.contains(&"1  same".to_string()));
        assert!(!rendered.iter().any(|line| line.starts_with("@ @@")));
    }

    #[test]
    fn renders_hunk_gap_ellipsis() {
        let lines = render_unified_diff(
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n@@ -10 +10 @@\n-old2\n+new2",
            Some(80),
            true,
        );
        assert!(plain(&lines).iter().any(|line| line.trim() == "⋮"));
    }

    #[test]
    fn hunk_header_allows_function_context_without_leaking_as_line() {
        let lines = render_unified_diff(
            "--- a/x.rs\n+++ b/x.rs\n@@ -20,1 +20,1 @@ fn main()\n-old\n+new",
            Some(80),
            false,
        );
        assert!(plain(&lines).contains(&"20 -old".to_string()));
        assert!(plain(&lines).contains(&"20 +new".to_string()));
    }

    #[test]
    fn highlights_changed_words_in_adjacent_delete_add_pair() {
        let lines = render_unified_diff(
            "--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-let status = \"slow\";\n+let status = \"fast\";",
            Some(80),
            true,
        );
        let removed = lines
            .iter()
            .find(|line| line_to_plain(line) == "1 -let status = \"slow\";")
            .unwrap();
        let added = lines
            .iter()
            .find(|line| line_to_plain(line) == "1 +let status = \"fast\";")
            .unwrap();
        let removed_changed = removed
            .spans
            .iter()
            .find(|span| span.content.as_ref().contains("slow"))
            .unwrap();
        let added_changed = added
            .spans
            .iter()
            .find(|span| span.content.as_ref().contains("fast"))
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
    fn delete_syntax_spans_are_dimmed() {
        let lines = render_unified_diff(
            "--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-let answer = 42;",
            Some(80),
            true,
        );
        let removed = lines
            .iter()
            .find(|line| line_to_plain(line) == "1 -let answer = 42;")
            .unwrap();
        assert!(removed
            .spans
            .iter()
            .skip(2)
            .any(|span| span.style.add_modifier.contains(Modifier::DIM)));
    }

    #[test]
    fn oversized_word_diff_uses_line_level_fallback() {
        let removed = format!("-{}", "slow ".repeat(WORD_DIFF_MAX_WORDS + 1));
        let added = format!("+{}", "fast ".repeat(WORD_DIFF_MAX_WORDS + 1));
        let diff = format!("--- a/x\n+++ b/x\n@@ -1 +1 @@\n{removed}\n{added}");
        let lines = render_unified_diff(&diff, None, true);
        let removed_line = lines
            .iter()
            .find(|line| line_to_plain(line).contains("-slow"))
            .unwrap();
        let added_line = lines
            .iter()
            .find(|line| line_to_plain(line).contains("+fast"))
            .unwrap();
        assert!(removed_line
            .spans
            .iter()
            .all(|span| !span.style.add_modifier.contains(Modifier::REVERSED)));
        assert!(added_line
            .spans
            .iter()
            .all(|span| !span.style.add_modifier.contains(Modifier::REVERSED)));
    }

    #[test]
    fn no_color_diff_keeps_gutter_signs() {
        let lines = render_unified_diff("@@ -1 +1 @@\n-old\n+new", Some(80), false);
        assert!(plain(&lines).contains(&"1 -old".to_string()));
        assert!(plain(&lines).contains(&"1 +new".to_string()));
    }

    #[test]
    fn calculates_add_remove_counts() {
        assert_eq!(
            calculate_add_remove_from_diff("--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-old\n+new\n same"),
            (1, 1)
        );
    }

    #[test]
    fn displays_paths_relative_to_cwd_and_home() {
        let cwd = PathBuf::from("/tmp/project");
        assert_eq!(
            display_path_for(Path::new("/tmp/project/src/main.rs"), &cwd),
            "src/main.rs"
        );
        assert_eq!(
            display_path_for(Path::new("src/lib.rs"), &cwd),
            "src/lib.rs"
        );
    }

    #[test]
    fn creates_diff_summary_for_multiple_files() {
        let mut changes = HashMap::new();
        changes.insert(
            PathBuf::from("src/a.rs"),
            FileChange::Add {
                content: "fn a() {}\n".to_string(),
            },
        );
        changes.insert(
            PathBuf::from("src/b.rs"),
            FileChange::Update {
                unified_diff: "--- a/src/b.rs\n+++ b/src/b.rs\n@@ -1 +1 @@\n-old\n+new".to_string(),
                move_path: Some(PathBuf::from("src/c.rs")),
            },
        );
        let rendered = plain(&create_diff_summary(
            &changes,
            Path::new("/tmp/project"),
            80,
        ));
        assert!(rendered[0].contains("Edited 2 files"));
        assert!(rendered
            .iter()
            .any(|line| line.contains("src/b.rs → src/c.rs")));
        assert!(rendered.iter().any(|line| line.contains("+new")));
    }
}
