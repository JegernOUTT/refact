use std::borrow::Cow;
use std::ops::Range;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use textwrap::core::{display_width, Word};
use textwrap::{Options, WordSeparator};
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

#[derive(Debug, Clone)]
pub struct RtOptions<'a> {
    pub width: usize,
    pub line_ending: textwrap::LineEnding,
    pub initial_indent: Line<'a>,
    pub subsequent_indent: Line<'a>,
    pub break_words: bool,
    pub wrap_algorithm: textwrap::WrapAlgorithm,
    pub word_separator: textwrap::WordSeparator,
    pub word_splitter: textwrap::WordSplitter,
}

impl From<usize> for RtOptions<'_> {
    fn from(width: usize) -> Self {
        Self::new(width)
    }
}

impl<'a> RtOptions<'a> {
    pub fn new(width: usize) -> Self {
        Self {
            width,
            line_ending: textwrap::LineEnding::LF,
            initial_indent: Line::default(),
            subsequent_indent: Line::default(),
            break_words: true,
            wrap_algorithm: textwrap::WrapAlgorithm::FirstFit,
            word_separator: textwrap::WordSeparator::new(),
            word_splitter: textwrap::WordSplitter::HyphenSplitter,
        }
    }

    pub fn line_ending(self, line_ending: textwrap::LineEnding) -> Self {
        Self {
            line_ending,
            ..self
        }
    }

    pub fn width(self, width: usize) -> Self {
        Self { width, ..self }
    }

    pub fn initial_indent(self, initial_indent: Line<'a>) -> Self {
        Self {
            initial_indent,
            ..self
        }
    }

    pub fn subsequent_indent(self, subsequent_indent: Line<'a>) -> Self {
        Self {
            subsequent_indent,
            ..self
        }
    }

    pub fn break_words(self, break_words: bool) -> Self {
        Self {
            break_words,
            ..self
        }
    }

    pub fn wrap_algorithm(self, wrap_algorithm: textwrap::WrapAlgorithm) -> Self {
        Self {
            wrap_algorithm,
            ..self
        }
    }

    pub fn word_separator(self, word_separator: textwrap::WordSeparator) -> Self {
        Self {
            word_separator,
            ..self
        }
    }

    pub fn word_splitter(self, word_splitter: textwrap::WordSplitter) -> Self {
        Self {
            word_splitter,
            ..self
        }
    }
}

pub trait IntoLineInput<'a> {
    fn into_line_input(self) -> Line<'a>;
}

impl<'a> IntoLineInput<'a> for &'a Line<'a> {
    fn into_line_input(self) -> Line<'a> {
        self.clone()
    }
}

impl<'a> IntoLineInput<'a> for &'a mut Line<'a> {
    fn into_line_input(self) -> Line<'a> {
        self.clone()
    }
}

impl<'a> IntoLineInput<'a> for Line<'a> {
    fn into_line_input(self) -> Line<'a> {
        self
    }
}

impl<'a> IntoLineInput<'a> for String {
    fn into_line_input(self) -> Line<'a> {
        Line::from(self)
    }
}

impl<'a> IntoLineInput<'a> for &'a str {
    fn into_line_input(self) -> Line<'a> {
        Line::from(self)
    }
}

impl<'a> IntoLineInput<'a> for Cow<'a, str> {
    fn into_line_input(self) -> Line<'a> {
        Line::from(self)
    }
}

impl<'a> IntoLineInput<'a> for Span<'a> {
    fn into_line_input(self) -> Line<'a> {
        Line::from(self)
    }
}

impl<'a> IntoLineInput<'a> for Vec<Span<'a>> {
    fn into_line_input(self) -> Line<'a> {
        Line::from(self)
    }
}

pub fn wrap_line(line: Line<'static>, width: Option<usize>) -> Vec<Line<'static>> {
    let Some(width) = width.filter(|width| *width > 0) else {
        return vec![line];
    };
    if line_width(&line) <= width {
        return vec![line];
    }
    let chars = flatten_line_chars(&line);
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

pub fn wrap_ranges<'a, O>(text: &str, width_or_options: O) -> Vec<Range<usize>>
where
    O: Into<Options<'a>>,
{
    let opts = width_or_options.into();
    let mut lines = Vec::new();
    let mut cursor = 0usize;
    let wrapped = textwrap::wrap(text, &opts);
    for (line_index, line) in wrapped.iter().enumerate() {
        let synthetic_prefix = if line_index == 0 {
            opts.initial_indent
        } else {
            opts.subsequent_indent
        };
        let range = match line {
            Cow::Borrowed(slice) => borrowed_slice_range(text, slice).unwrap_or_else(|| {
                map_owned_wrapped_line_to_range(text, cursor, slice, synthetic_prefix)
            }),
            Cow::Owned(slice) => {
                map_owned_wrapped_line_to_range(text, cursor, slice, synthetic_prefix)
            }
        };
        let trailing_spaces = trailing_ascii_spaces_len(&text[range.end..]);
        lines.push(range.start..range.end + trailing_spaces + 1);
        cursor = range.end + trailing_spaces;
    }
    lines
}

pub fn wrap_ranges_trim<'a, O>(text: &str, width_or_options: O) -> Vec<Range<usize>>
where
    O: Into<Options<'a>>,
{
    let opts = width_or_options.into();
    let mut lines = Vec::new();
    let mut cursor = 0usize;
    let wrapped = textwrap::wrap(text, &opts);
    for (line_index, line) in wrapped.iter().enumerate() {
        let synthetic_prefix = if line_index == 0 {
            opts.initial_indent
        } else {
            opts.subsequent_indent
        };
        let range = match line {
            Cow::Borrowed(slice) => borrowed_slice_range(text, slice).unwrap_or_else(|| {
                map_owned_wrapped_line_to_range(text, cursor, slice, synthetic_prefix)
            }),
            Cow::Owned(slice) => {
                map_owned_wrapped_line_to_range(text, cursor, slice, synthetic_prefix)
            }
        };
        cursor = range.end;
        lines.push(range);
    }
    lines
}

pub fn line_contains_url_like(line: &Line<'_>) -> bool {
    text_contains_url_like(&line_to_plain(line))
}

pub fn line_has_mixed_url_and_non_url_tokens(line: &Line<'_>) -> bool {
    text_has_mixed_url_and_non_url_tokens(&line_to_plain(line))
}

pub fn text_contains_url_like(text: &str) -> bool {
    text.split_ascii_whitespace().any(is_url_like_token)
}

pub fn url_preserving_wrap_options<'a>(opts: RtOptions<'a>) -> RtOptions<'a> {
    opts.word_separator(textwrap::WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::NoHyphenation)
        .break_words(false)
}

pub fn adaptive_wrap_line<'a, O>(line: &'a Line<'a>, width_or_options: O) -> Vec<Line<'a>>
where
    O: Into<RtOptions<'a>>,
{
    let base = width_or_options.into();
    if !line_contains_url_like(line) {
        return word_wrap_line(line, base);
    }

    if line_has_mixed_url_and_non_url_tokens(line) {
        mixed_url_wrap_line(line, base)
    } else {
        word_wrap_line(line, url_preserving_wrap_options(base))
    }
}

pub fn adaptive_wrap_lines<'a, I, O, L>(lines: I, width_or_options: O) -> Vec<Line<'static>>
where
    I: IntoIterator<Item = L>,
    L: IntoLineInput<'a>,
    O: Into<RtOptions<'a>>,
{
    let base_opts = width_or_options.into();
    let mut out = Vec::new();

    for (idx, line) in lines.into_iter().enumerate() {
        let line = line.into_line_input();
        let opts = if idx == 0 {
            base_opts.clone()
        } else {
            base_opts
                .clone()
                .initial_indent(base_opts.subsequent_indent.clone())
        };
        let wrapped = adaptive_wrap_line(&line, opts);
        out.extend(wrapped.iter().map(line_to_owned));
    }

    out
}

pub fn word_wrap_line<'a, O>(line: &'a Line<'a>, width_or_options: O) -> Vec<Line<'a>>
where
    O: Into<RtOptions<'a>>,
{
    let (flat, span_bounds) = flatten_line_text(line);
    let rt_opts = width_or_options.into();
    let opts = Options::new(rt_opts.width.max(1))
        .line_ending(rt_opts.line_ending)
        .break_words(rt_opts.break_words)
        .wrap_algorithm(rt_opts.wrap_algorithm)
        .word_separator(rt_opts.word_separator)
        .word_splitter(rt_opts.word_splitter);

    if flat.is_empty() {
        return vec![styled_indent(
            rt_opts.initial_indent.clone(),
            line.style,
            line.alignment,
        )];
    }

    let initial_width = opts
        .width
        .saturating_sub(line_width(&rt_opts.initial_indent))
        .max(1);
    let initial_wrapped = wrap_ranges_trim(&flat, opts.clone().width(initial_width));
    let Some(first_line_range) = initial_wrapped.first() else {
        return vec![styled_indent(
            rt_opts.initial_indent.clone(),
            line.style,
            line.alignment,
        )];
    };

    let mut out = Vec::new();
    let mut first_line = styled_indent(rt_opts.initial_indent.clone(), line.style, line.alignment);
    append_sliced_spans(&mut first_line, line, &span_bounds, first_line_range);
    out.push(first_line);

    let base = first_line_range.end;
    let base = base + trailing_ascii_spaces_len(&flat[base..]);
    let subsequent_width = opts
        .width
        .saturating_sub(line_width(&rt_opts.subsequent_indent))
        .max(1);
    let remaining_wrapped = wrap_ranges_trim(&flat[base..], opts.width(subsequent_width));
    for range in remaining_wrapped {
        if range.is_empty() {
            continue;
        }
        let offset_range = range.start + base..range.end + base;
        let mut subsequent_line = styled_indent(
            rt_opts.subsequent_indent.clone(),
            line.style,
            line.alignment,
        );
        append_sliced_spans(&mut subsequent_line, line, &span_bounds, &offset_range);
        out.push(subsequent_line);
    }

    out
}

pub fn word_wrap_lines<'a, I, O, L>(lines: I, width_or_options: O) -> Vec<Line<'static>>
where
    I: IntoIterator<Item = L>,
    L: IntoLineInput<'a>,
    O: Into<RtOptions<'a>>,
{
    let base_opts = width_or_options.into();
    let mut out = Vec::new();

    for (idx, line) in lines.into_iter().enumerate() {
        let line = line.into_line_input();
        let opts = if idx == 0 {
            base_opts.clone()
        } else {
            base_opts
                .clone()
                .initial_indent(base_opts.subsequent_indent.clone())
        };
        let wrapped = word_wrap_line(&line, opts);
        out.extend(wrapped.iter().map(line_to_owned));
    }

    out
}

pub fn take_prefix_by_width(text: &str, max_cols: usize) -> (String, &str, usize) {
    if max_cols == 0 || text.is_empty() {
        return (String::new(), text, 0);
    }
    let mut cols = 0usize;
    let mut end_idx = 0usize;
    for (idx, ch) in text.char_indices() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if cols.saturating_add(ch_width) > max_cols {
            break;
        }
        cols += ch_width;
        end_idx = idx + ch.len_utf8();
        if cols == max_cols {
            break;
        }
    }
    (text[..end_idx].to_string(), &text[end_idx..], cols)
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

fn flatten_line_chars(line: &Line<'_>) -> Vec<StyledChar> {
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

fn borrowed_slice_range(text: &str, slice: &str) -> Option<Range<usize>> {
    let text_start = text.as_ptr() as usize;
    let text_end = text_start.checked_add(text.len())?;
    let slice_start = slice.as_ptr() as usize;
    let slice_end = slice_start.checked_add(slice.len())?;

    if slice_start < text_start || slice_end > text_end {
        return None;
    }

    Some(slice_start - text_start..slice_end - text_start)
}

fn map_owned_wrapped_line_to_range(
    text: &str,
    cursor: usize,
    wrapped: &str,
    synthetic_prefix: &str,
) -> Range<usize> {
    let wrapped = if synthetic_prefix.is_empty() {
        wrapped
    } else {
        wrapped.strip_prefix(synthetic_prefix).unwrap_or(wrapped)
    };

    let mut start = cursor;
    while start < text.len() && !wrapped.starts_with(' ') {
        let Some(ch) = text[start..].chars().next() else {
            break;
        };
        if ch != ' ' {
            break;
        }
        start += ch.len_utf8();
    }

    let mut end = start;
    let mut saw_source_char = false;
    let mut chars = wrapped.chars().peekable();
    while let Some(ch) = chars.next() {
        if end < text.len() {
            let Some(src) = text[end..].chars().next() else {
                break;
            };
            if ch == src {
                end += src.len_utf8();
                saw_source_char = true;
                continue;
            }
        }

        if ch == '-' && chars.peek().is_none() {
            continue;
        }

        if !saw_source_char {
            continue;
        }

        tracing::warn!(
            wrapped = %wrapped,
            cursor,
            end,
            "wrap_ranges could not fully map owned line"
        );
        break;
    }

    start..end
}

fn trailing_ascii_spaces_len(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .take_while(|byte| **byte == b' ')
        .count()
}

fn text_has_mixed_url_and_non_url_tokens(text: &str) -> bool {
    let mut saw_url = false;
    let mut saw_non_url = false;

    for raw_token in text.split_ascii_whitespace() {
        if is_url_like_token(raw_token) {
            saw_url = true;
        } else if is_substantive_non_url_token(raw_token) {
            saw_non_url = true;
        }

        if saw_url && saw_non_url {
            return true;
        }
    }

    false
}

fn is_url_like_token(raw_token: &str) -> bool {
    let token = trim_url_token(raw_token);
    !token.is_empty() && (is_absolute_url_like(token) || is_bare_url_like(token))
}

fn is_substantive_non_url_token(raw_token: &str) -> bool {
    let token = trim_url_token(raw_token);
    if token.is_empty() || is_decorative_marker_token(raw_token, token) {
        return false;
    }

    token.chars().any(char::is_alphanumeric)
}

fn is_decorative_marker_token(raw_token: &str, token: &str) -> bool {
    let raw = raw_token.trim();
    matches!(
        raw,
        "-" | "*"
            | "+"
            | "•"
            | "◦"
            | "▪"
            | ">"
            | "|"
            | "│"
            | "┆"
            | "└"
            | "├"
            | "┌"
            | "┐"
            | "┘"
            | "┼"
    ) || is_ordered_list_marker(raw, token)
}

fn is_ordered_list_marker(raw_token: &str, token: &str) -> bool {
    token.chars().all(|ch| ch.is_ascii_digit())
        && (raw_token.ends_with('.') || raw_token.ends_with(')'))
}

fn trim_url_token(token: &str) -> &str {
    token.trim_matches(|ch: char| {
        matches!(
            ch,
            '(' | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | ','
                | '.'
                | ';'
                | ':'
                | '!'
                | '\''
                | '"'
        )
    })
}

fn is_absolute_url_like(token: &str) -> bool {
    if !token.contains("://") {
        return false;
    }

    if let Ok(url) = url::Url::parse(token) {
        let scheme = url.scheme().to_ascii_lowercase();
        if matches!(
            scheme.as_str(),
            "http" | "https" | "ftp" | "ftps" | "ws" | "wss"
        ) {
            return url.host_str().is_some();
        }
        return true;
    }

    has_valid_scheme_prefix(token)
}

fn has_valid_scheme_prefix(token: &str) -> bool {
    let Some((scheme, rest)) = token.split_once("://") else {
        return false;
    };
    if scheme.is_empty() || rest.is_empty() {
        return false;
    }

    let mut chars = scheme.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '+' || ch == '-' || ch == '.')
}

fn is_bare_url_like(token: &str) -> bool {
    let (host_port, has_trailer) = split_host_port_and_trailer(token);
    if host_port.is_empty() {
        return false;
    }

    if !has_trailer && !host_port.to_ascii_lowercase().starts_with("www.") {
        return false;
    }

    let (host, port) = split_host_and_port(host_port);
    if host.is_empty() {
        return false;
    }
    if let Some(port) = port {
        if !is_valid_port(port) {
            return false;
        }
    }

    host.eq_ignore_ascii_case("localhost") || is_ipv4(host) || is_domain_name(host)
}

fn split_host_port_and_trailer(token: &str) -> (&str, bool) {
    if let Some(idx) = token.find(['/', '?', '#']) {
        (&token[..idx], true)
    } else {
        (token, false)
    }
}

fn split_host_and_port(host_port: &str) -> (&str, Option<&str>) {
    if host_port.starts_with('[') {
        return (host_port, None);
    }

    if let Some((host, port)) = host_port.rsplit_once(':') {
        if !host.is_empty() && !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()) {
            return (host, Some(port));
        }
    }

    (host_port, None)
}

fn is_valid_port(port: &str) -> bool {
    if port.is_empty() || port.len() > 5 || !port.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }

    port.parse::<u16>().is_ok()
}

fn is_ipv4(host: &str) -> bool {
    let parts = host.split('.').collect::<Vec<_>>();
    if parts.len() != 4 {
        return false;
    }

    parts
        .iter()
        .all(|part| !part.is_empty() && part.parse::<u8>().is_ok())
}

fn is_domain_name(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if !host.contains('.') {
        return false;
    }

    let mut labels = host.split('.');
    let Some(tld) = labels.next_back() else {
        return false;
    };
    if !is_tld(tld) {
        return false;
    }

    labels.all(is_domain_label)
}

fn is_tld(label: &str) -> bool {
    (2..=63).contains(&label.len()) && label.chars().all(|ch| ch.is_ascii_alphabetic())
}

fn is_domain_label(label: &str) -> bool {
    if label.is_empty() || label.len() > 63 {
        return false;
    }

    let mut chars = label.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let Some(last) = label.chars().next_back() else {
        return false;
    };

    first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && label
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

#[derive(Clone, Debug)]
struct MixedUrlWord {
    range: Range<usize>,
    is_url: bool,
}

impl MixedUrlWord {
    fn width(&self, text: &str) -> usize {
        display_width(&text[self.range.clone()])
    }
}

fn mixed_url_wrap_line<'a>(line: &'a Line<'a>, rt_opts: RtOptions<'a>) -> Vec<Line<'a>> {
    let (flat, span_bounds) = flatten_line_text(line);
    let initial_width = rt_opts
        .width
        .saturating_sub(line_width(&rt_opts.initial_indent))
        .max(1);
    let subsequent_width = rt_opts
        .width
        .saturating_sub(line_width(&rt_opts.subsequent_indent))
        .max(1);
    let ranges = mixed_url_wrap_ranges(&flat, initial_width, subsequent_width);

    let mut out = Vec::new();
    for (idx, range) in ranges.iter().enumerate() {
        let mut wrapped_line = if idx == 0 {
            rt_opts.initial_indent.clone()
        } else {
            rt_opts.subsequent_indent.clone()
        };
        wrapped_line = styled_indent(wrapped_line, line.style, line.alignment);
        append_sliced_spans(&mut wrapped_line, line, &span_bounds, range);
        out.push(wrapped_line);
    }

    if out.is_empty() {
        vec![styled_indent(
            rt_opts.initial_indent.clone(),
            line.style,
            line.alignment,
        )]
    } else {
        out
    }
}

fn mixed_url_wrap_ranges(
    text: &str,
    initial_width: usize,
    subsequent_width: usize,
) -> Vec<Range<usize>> {
    let leading_space_width = text.chars().take_while(|ch| *ch == ' ').count();
    let mut words = Vec::new();
    let mut cursor = 0usize;
    for word in WordSeparator::AsciiSpace.find_words(text) {
        let word_start = cursor;
        let word_end = word_start + word.word.len();
        let trailing_space_end = word_end + word.whitespace.len();
        if !word.word.is_empty() {
            words.push(MixedUrlWord {
                range: word_start..word_end,
                is_url: is_url_like_token(word.word),
            });
        }
        cursor = trailing_space_end;
    }

    let mut lines = Vec::new();
    let mut line_start = None;
    let mut line_end = 0usize;
    let mut line_width = 0usize;
    let mut line_limit = initial_width.max(1);

    for word in words {
        let mut pending = split_mixed_url_word(text, word, line_limit);
        let mut pending_idx = 0usize;

        while let Some(piece) = pending.get(pending_idx).cloned() {
            let empty_line_prefix_width = if line_start.is_none() && lines.is_empty() {
                leading_space_width
            } else {
                0
            };
            let empty_line_piece_limit = line_limit.saturating_sub(empty_line_prefix_width).max(1);
            if line_start.is_none() && !piece.is_url && piece.width(text) > empty_line_piece_limit {
                pending.splice(
                    pending_idx..=pending_idx,
                    split_mixed_url_word(text, piece, empty_line_piece_limit),
                );
                continue;
            }

            let piece_width = piece.width(text);
            let inter_word_space = line_start
                .map(|_| text[line_end..piece.range.start].len())
                .unwrap_or(0);
            let fits = if line_start.is_none() {
                piece.is_url
                    || empty_line_prefix_width + piece_width <= line_limit
                    || empty_line_prefix_width >= line_limit
            } else {
                line_width + inter_word_space + piece_width <= line_limit
            };

            if fits {
                if line_start.is_none() {
                    let is_first_output_line = lines.is_empty();
                    let start = if is_first_output_line {
                        0
                    } else {
                        piece.range.start
                    };
                    line_start = Some(start);
                    line_width = if is_first_output_line {
                        leading_space_width + piece_width
                    } else {
                        piece_width
                    };
                } else {
                    line_width += inter_word_space + piece_width;
                }
                line_end = piece.range.end;
                pending_idx += 1;
                continue;
            }

            if let Some(start) = line_start.take() {
                lines.push(start..line_end);
            }
            line_end = 0;
            line_width = 0;
            line_limit = subsequent_width.max(1);
        }
    }

    if let Some(start) = line_start {
        lines.push(start..line_end);
    }

    lines
}

fn split_mixed_url_word(text: &str, word: MixedUrlWord, line_limit: usize) -> Vec<MixedUrlWord> {
    if word.is_url || word.width(text) <= line_limit {
        return vec![word];
    }

    let source = Word::from(&text[word.range.clone()]);
    let mut offset = word.range.start;
    let mut pieces = Vec::new();
    for piece in source.break_apart(line_limit.max(1)) {
        let end = offset + piece.word.len();
        pieces.push(MixedUrlWord {
            range: offset..end,
            is_url: false,
        });
        offset = end;
    }
    pieces
}

fn flatten_line_text(line: &Line<'_>) -> (String, Vec<(Range<usize>, Style)>) {
    let mut flat = String::new();
    let mut span_bounds = Vec::new();
    let mut acc = 0usize;
    for span in &line.spans {
        let text = span.content.as_ref();
        let start = acc;
        flat.push_str(text);
        acc += text.len();
        span_bounds.push((start..acc, span.style));
    }
    (flat, span_bounds)
}

fn styled_indent<'a>(
    mut line: Line<'a>,
    style: Style,
    alignment: Option<ratatui::layout::Alignment>,
) -> Line<'a> {
    line.style = style;
    line.alignment = alignment;
    line
}

fn append_sliced_spans<'a>(
    target: &mut Line<'a>,
    original: &'a Line<'a>,
    span_bounds: &[(Range<usize>, Style)],
    range: &Range<usize>,
) {
    target.spans.extend(
        slice_line_spans(original, span_bounds, range)
            .spans
            .into_iter()
            .map(|span| span.patch_style(original.style)),
    );
}

fn slice_line_spans<'a>(
    original: &'a Line<'a>,
    span_bounds: &[(Range<usize>, Style)],
    range: &Range<usize>,
) -> Line<'a> {
    let start_byte = range.start;
    let end_byte = range.end;
    let mut spans = Vec::new();
    for (idx, (span_range, style)) in span_bounds.iter().enumerate() {
        let start = span_range.start;
        let end = span_range.end;
        if end <= start_byte {
            continue;
        }
        if start >= end_byte {
            break;
        }
        let seg_start = start_byte.max(start);
        let seg_end = end_byte.min(end);
        if seg_end > seg_start {
            let local_start = seg_start - start;
            let local_end = seg_end - start;
            let content = original.spans[idx].content.as_ref();
            spans.push(Span {
                style: *style,
                content: Cow::Borrowed(&content[local_start..local_end]),
            });
        }
        if end >= end_byte {
            break;
        }
    }
    Line {
        style: original.style,
        alignment: original.alignment,
        spans,
    }
}

fn line_to_owned(line: &Line<'_>) -> Line<'static> {
    Line {
        style: line.style,
        alignment: line.alignment,
        spans: line
            .spans
            .iter()
            .map(|span| Span::styled(span.content.as_ref().to_owned(), span.style))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn plain(lines: Vec<Line<'static>>) -> Vec<String> {
        lines.iter().map(line_to_plain).collect()
    }

    fn borrowed_plain(lines: &[Line<'_>]) -> Vec<String> {
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
            Style::default().fg(Color::Green),
        )]);
        let lines = wrap_line(line, Some(5));
        assert_eq!(line_to_plain(&lines[0]), "alpha");
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn word_wrap_line_applies_initial_and_subsequent_indents() {
        let opts = RtOptions::new(8)
            .initial_indent(Line::from("- "))
            .subsequent_indent(Line::from("  "));
        let line = Line::from("hello world foo");
        let out = word_wrap_line(&line, opts);
        assert_eq!(borrowed_plain(&out), vec!["- hello", "  world", "  foo"]);
    }

    #[test]
    fn word_wrap_line_preserves_styles_across_wrapped_spans() {
        let line = Line::from(vec![
            Span::styled("hello ", Style::default().fg(Color::Red)),
            Span::raw("world"),
        ]);
        let out = word_wrap_line(&line, 6);
        assert_eq!(borrowed_plain(&out), vec!["hello", "world"]);
        assert_eq!(out[0].spans[0].style.fg, Some(Color::Red));
        assert_eq!(out[1].spans[0].style.fg, None);
    }

    #[test]
    fn word_wrap_lines_applies_initial_indent_once() {
        let opts = RtOptions::new(8)
            .initial_indent(Line::from("- "))
            .subsequent_indent(Line::from("  "));
        let out = word_wrap_lines(
            vec![Line::from("hello world"), Line::from("foo bar baz")],
            opts,
        );
        assert_eq!(
            plain(out),
            vec!["- hello", "  world", "  foo", "  bar", "  baz"]
        );
    }

    #[test]
    fn adaptive_wrap_line_keeps_long_url_like_token_intact() {
        let line = Line::from("example.test/a-very-long-path-with-many-segments-and-query?x=1&y=2");
        let out = adaptive_wrap_line(&line, RtOptions::new(20));
        assert_eq!(
            borrowed_plain(&out),
            vec!["example.test/a-very-long-path-with-many-segments-and-query?x=1&y=2"]
        );
    }

    #[test]
    fn adaptive_wrap_line_preserves_default_behavior_for_non_url_tokens() {
        let line = Line::from("a_very_long_token_without_spaces_to_force_wrapping");
        let out = adaptive_wrap_line(&line, RtOptions::new(20));
        assert!(out.len() > 1);
    }

    #[test]
    fn adaptive_wrap_line_mixed_line_keeps_regular_words_intact() {
        let line = Line::from(
            "see https://example.com/path and keep strikethrough intact while wrapping prose",
        );
        let out = adaptive_wrap_line(&line, RtOptions::new(36));
        assert_eq!(
            borrowed_plain(&out),
            vec![
                "see https://example.com/path and",
                "keep strikethrough intact while",
                "wrapping prose",
            ]
        );
    }

    #[test]
    fn adaptive_wrap_lines_uses_subsequent_indent_after_first_input_line() {
        let opts = RtOptions::new(8)
            .initial_indent(Line::from("- "))
            .subsequent_indent(Line::from("  "));
        let out = adaptive_wrap_lines(["hello world", "see https://x.co/path"], opts);
        assert_eq!(
            plain(out),
            vec!["- hello", "  world", "  see", "  https://x.co/path"]
        );
    }

    #[test]
    fn url_detection_matches_expected_tokens() {
        assert!(text_contains_url_like("https://example.com/a/b"));
        assert!(text_contains_url_like("www.example.com/path?x=1"));
        assert!(text_contains_url_like("localhost:3000/api"));
        assert!(!text_contains_url_like("src/main.rs"));
        assert!(!text_contains_url_like("hello.world"));
    }

    #[test]
    fn wrap_ranges_trim_rebuilds_unspaced_wrapped_source() {
        let text = "a_very_long_token_without_spaces";
        let ranges = wrap_ranges_trim(text, Options::new(8));
        let rebuilt = ranges
            .iter()
            .map(|range| &text[range.clone()])
            .collect::<String>();
        assert_eq!(rebuilt, text);
        assert!(ranges.len() > 1);
    }

    #[test]
    fn take_prefix_by_width_counts_wide_characters() {
        let (prefix, suffix, width) = take_prefix_by_width("😀😀 你好", 6);
        assert_eq!(prefix, "😀😀 ");
        assert_eq!(suffix, "你好");
        assert_eq!(width, 5);
    }
}
