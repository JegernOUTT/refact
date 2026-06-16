// Adapted from openai/codex codex-rs/tui terminal_hyperlinks.rs, Apache-2.0.

use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use url::Url;

use crate::render::line_utils::line_to_static;
use crate::render::wrapping::{adaptive_wrap_line, RtOptions};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalHyperlink {
    pub columns: Range<usize>,
    pub destination: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HyperlinkLine {
    pub line: Line<'static>,
    pub hyperlinks: Vec<TerminalHyperlink>,
}

impl HyperlinkLine {
    pub fn new(line: Line<'static>) -> Self {
        Self {
            line,
            hyperlinks: Vec::new(),
        }
    }

    pub fn width(&self) -> usize {
        self.line
            .spans
            .iter()
            .map(|span| span.content.as_ref().width())
            .sum()
    }

    pub fn push_span(&mut self, span: Span<'static>, destination: Option<&str>) {
        let start = self.width();
        let end = start + span.content.as_ref().width();
        self.line.spans.push(span);
        if end > start {
            if let Some(destination) = destination.and_then(web_destination) {
                self.hyperlinks.push(TerminalHyperlink {
                    columns: start..end,
                    destination,
                });
            }
        }
    }

    pub fn style(mut self, style: ratatui::style::Style) -> Self {
        self.line = self.line.style(style);
        self
    }
}

impl From<Line<'static>> for HyperlinkLine {
    fn from(line: Line<'static>) -> Self {
        Self::new(line)
    }
}

impl From<String> for HyperlinkLine {
    fn from(text: String) -> Self {
        Self::new(Line::from(text))
    }
}

impl From<&'static str> for HyperlinkLine {
    fn from(text: &'static str) -> Self {
        Self::new(Line::from(text))
    }
}

pub fn visible_lines(lines: Vec<HyperlinkLine>) -> Vec<Line<'static>> {
    lines.into_iter().map(|line| line.line).collect()
}

pub fn plain_hyperlink_lines(lines: Vec<Line<'static>>) -> Vec<HyperlinkLine> {
    lines.into_iter().map(annotate_web_urls_in_line).collect()
}

pub fn prefix_hyperlink_lines(
    lines: Vec<HyperlinkLine>,
    initial_prefix: Span<'static>,
    subsequent_prefix: Span<'static>,
) -> Vec<HyperlinkLine> {
    lines
        .into_iter()
        .enumerate()
        .map(|(index, mut line)| {
            let prefix = if index == 0 {
                initial_prefix.clone()
            } else {
                subsequent_prefix.clone()
            };
            let shift = prefix.content.as_ref().width();
            let mut spans = Vec::with_capacity(line.line.spans.len() + 1);
            spans.push(prefix);
            spans.extend(line.line.spans);
            line.line = Line::from(spans).style(line.line.style);
            for hyperlink in &mut line.hyperlinks {
                hyperlink.columns = hyperlink.columns.start + shift..hyperlink.columns.end + shift;
            }
            line
        })
        .collect()
}

pub fn adaptive_wrap_hyperlink_lines(
    lines: &[HyperlinkLine],
    options: RtOptions<'static>,
) -> Vec<HyperlinkLine> {
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let options = if index == 0 {
            options.clone()
        } else {
            options
                .clone()
                .initial_indent(options.subsequent_indent.clone())
        };
        out.extend(remap_wrapped_line(
            line,
            adaptive_wrap_line(&line.line, options)
                .iter()
                .map(line_to_static)
                .collect(),
        ));
    }
    out
}

pub fn annotate_web_urls(lines: Vec<Line<'static>>) -> Vec<HyperlinkLine> {
    lines.into_iter().map(annotate_web_urls_in_line).collect()
}

pub fn annotate_web_urls_in_line(line: Line<'static>) -> HyperlinkLine {
    let text = line_text(&line);
    let mut out = HyperlinkLine::new(line);
    out.hyperlinks = web_links_in_text(&text);
    out
}

pub fn remap_wrapped_line(
    source: &HyperlinkLine,
    wrapped: Vec<Line<'static>>,
) -> Vec<HyperlinkLine> {
    let mut out = plain_hyperlink_lines(wrapped);
    let source_text = line_text(&source.line);
    let mut source_byte = 0usize;
    let mut source_column = 0usize;
    for (index, line) in out.iter_mut().enumerate() {
        if index > 0 {
            let trimmed = source_text[source_byte..].trim_start_matches(char::is_whitespace);
            let skipped = source_text[source_byte..].len() - trimmed.len();
            source_column += source_text[source_byte..source_byte + skipped].width();
            source_byte += skipped;
        }

        let rendered = line_text(&line.line);
        let remaining = &source_text[source_byte..];
        let Some(rendered_start) = longest_suffix_matching_prefix(&rendered, remaining) else {
            continue;
        };
        let mapped = &rendered[rendered_start..];
        let mut output_column = rendered[..rendered_start].width();
        line.hyperlinks.clear();
        for ch in mapped.chars() {
            let width = ch.width().unwrap_or(0);
            if let Some(link) = source
                .hyperlinks
                .iter()
                .find(|link| link.columns.contains(&source_column))
            {
                push_link_range(
                    line,
                    output_column..output_column + width,
                    &link.destination,
                );
            }
            source_column += width;
            output_column += width;
        }
        source_byte += mapped.len();
    }
    out
}

pub fn line_with_osc8(line: &HyperlinkLine, enabled: bool) -> Line<'static> {
    if !enabled || line.hyperlinks.is_empty() {
        return line.line.clone();
    }
    Line {
        style: line.line.style,
        alignment: line.line.alignment,
        spans: decorate_spans(line),
    }
}

pub fn lines_with_osc8(lines: &[HyperlinkLine], enabled: bool) -> Vec<Line<'static>> {
    lines
        .iter()
        .map(|line| line_with_osc8(line, enabled))
        .collect()
}

pub fn mark_buffer_hyperlinks(
    buffer: &mut Buffer,
    area: Rect,
    lines: &[HyperlinkLine],
    enabled: bool,
) {
    if !enabled || area.width == 0 {
        return;
    }
    for (row, line) in lines.iter().enumerate().take(area.height as usize) {
        for link in &line.hyperlinks {
            let Some(destination) = web_destination(&link.destination) else {
                continue;
            };
            let mut cells = Vec::new();
            for column in link.columns.clone() {
                if column >= area.width as usize {
                    continue;
                }
                let x = area.x + column as u16;
                let y = area.y + row as u16;
                let cell = &buffer[(x, y)];
                if !cell.skip && !cell.symbol().trim().is_empty() {
                    cells.push((x, y));
                }
            }
            let Some(first) = cells.first().copied() else {
                continue;
            };
            let last = cells.last().copied().unwrap_or(first);
            let start = osc8_start(&destination);
            let end = osc8_end();
            if first == last {
                let symbol = buffer[first].symbol().to_string();
                buffer[first].set_symbol(&format!("{start}{symbol}{end}"));
            } else {
                let symbol = buffer[first].symbol().to_string();
                buffer[first].set_symbol(&format!("{start}{symbol}"));
                let symbol = buffer[last].symbol().to_string();
                buffer[last].set_symbol(&format!("{symbol}{end}"));
            }
        }
    }
}

pub fn hyperlinks_enabled_from_env() -> bool {
    let term = std::env::var("TERM").ok();
    let term_program = std::env::var("TERM_PROGRAM").ok();
    let force = std::env::var("REFACT_TUI_HYPERLINKS")
        .ok()
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        });
    hyperlinks_enabled_from_probe(EnvProbe {
        no_color: std::env::var_os("NO_COLOR").is_some(),
        term: term.as_deref(),
        term_program: term_program.as_deref(),
        vte_version: std::env::var_os("VTE_VERSION").is_some(),
        wt_session: std::env::var_os("WT_SESSION").is_some(),
        wezterm: std::env::var_os("WEZTERM_EXECUTABLE").is_some(),
        kitty: std::env::var_os("KITTY_WINDOW_ID").is_some(),
        force,
    })
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EnvProbe<'a> {
    pub no_color: bool,
    pub term: Option<&'a str>,
    pub term_program: Option<&'a str>,
    pub vte_version: bool,
    pub wt_session: bool,
    pub wezterm: bool,
    pub kitty: bool,
    pub force: Option<bool>,
}

pub fn hyperlinks_enabled_from_probe(probe: EnvProbe<'_>) -> bool {
    if probe.no_color || probe.term == Some("dumb") {
        return false;
    }
    if let Some(force) = probe.force {
        return force;
    }
    if probe.vte_version || probe.wt_session || probe.wezterm || probe.kitty {
        return true;
    }
    if let Some(term) = probe.term {
        if term.contains("xterm-kitty") || term.contains("wezterm") {
            return true;
        }
    }
    matches!(
        probe.term_program,
        Some("iTerm.app" | "WezTerm" | "vscode" | "WarpTerminal" | "Apple_Terminal")
    )
}

pub fn osc8_hyperlink(destination: &str, text: &str) -> String {
    let Some(destination) = web_destination(destination) else {
        return text.to_string();
    };
    format!("{}{}{}", osc8_start(&destination), text, osc8_end())
}

pub fn strip_osc8(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut stripped = String::with_capacity(text.len());
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index..].starts_with(b"\x1b]8;;") {
            index += 5;
            while index < bytes.len() {
                if bytes[index] == b'\x07' {
                    index += 1;
                    break;
                }
                if index + 1 < bytes.len() && bytes[index] == b'\x1b' && bytes[index + 1] == b'\\' {
                    index += 2;
                    break;
                }
                index += 1;
            }
            continue;
        }
        let ch = text[index..]
            .chars()
            .next()
            .expect("current byte index starts a character");
        stripped.push(ch);
        index += ch.len_utf8();
    }

    stripped
}

fn decorate_spans(line: &HyperlinkLine) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut column = 0usize;
    let mut link_index = 0usize;
    let mut active_link_index = None;
    for span in &line.line.spans {
        for ch in span.content.chars() {
            let width = ch.width().unwrap_or(0);
            while line
                .hyperlinks
                .get(link_index)
                .is_some_and(|link| link.columns.end <= column)
            {
                link_index += 1;
            }
            let selected_link_index = line
                .hyperlinks
                .get(link_index)
                .and_then(|link| link.columns.contains(&column).then_some(link_index));
            if active_link_index != selected_link_index {
                if active_link_index.is_some() {
                    append_to_last_span(&mut out, &osc8_end());
                }
                if let Some(destination) = selected_link_index
                    .and_then(|index| web_destination(&line.hyperlinks[index].destination))
                {
                    push_styled_content(&mut out, &osc8_start(&destination), span.style);
                }
                active_link_index = selected_link_index;
            }
            push_styled_content(&mut out, &ch.to_string(), span.style);
            column += width;
        }
    }
    if active_link_index.is_some() {
        append_to_last_span(&mut out, &osc8_end());
    }
    out
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn longest_suffix_matching_prefix(rendered: &str, source: &str) -> Option<usize> {
    rendered
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(rendered.len()))
        .find(|index| source.starts_with(&rendered[*index..]) && *index < rendered.len())
}

fn push_link_range(line: &mut HyperlinkLine, range: Range<usize>, destination: &str) {
    if range.is_empty() {
        return;
    }
    if let Some(previous) = line.hyperlinks.last_mut() {
        if previous.destination == destination && previous.columns.end == range.start {
            previous.columns.end = range.end;
            return;
        }
    }
    line.hyperlinks.push(TerminalHyperlink {
        columns: range,
        destination: destination.to_string(),
    });
}

fn web_links_in_text(text: &str) -> Vec<TerminalHyperlink> {
    let mut links = Vec::new();
    let mut search_from = 0usize;
    for raw_token in text.split_ascii_whitespace() {
        let Some(relative_start) = text[search_from..].find(raw_token) else {
            continue;
        };
        let raw_start = search_from + relative_start;
        search_from = raw_start + raw_token.len();
        let trimmed_start = raw_token
            .find(|ch: char| !is_leading_punctuation(ch))
            .unwrap_or(raw_token.len());
        let trimmed_end = trailing_url_end(&raw_token[trimmed_start..]) + trimmed_start;
        if trimmed_start >= trimmed_end {
            continue;
        }
        let candidate = &raw_token[trimmed_start..trimmed_end];
        let Some(destination) = web_destination(candidate) else {
            continue;
        };
        let start = text[..raw_start + trimmed_start].width();
        let end = start + candidate.width();
        links.push(TerminalHyperlink {
            columns: start..end,
            destination,
        });
    }
    links
}

pub fn web_destination(destination: &str) -> Option<String> {
    let safe_destination = destination
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>();
    let parsed = Url::parse(&safe_destination).ok()?;
    matches!(parsed.scheme(), "http" | "https")
        .then(|| parsed.host_str())
        .flatten()?;
    Some(safe_destination)
}

fn is_leading_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | '.' | ';' | '!' | '\'' | '"'
    )
}

fn trailing_url_end(candidate: &str) -> usize {
    let mut end = candidate.len();
    while end > 0 {
        let remaining = &candidate[..end];
        let Some(ch) = remaining.chars().next_back() else {
            break;
        };
        let trim = matches!(ch, ',' | '.' | ';' | '!' | '\'' | '"')
            || matches!(ch, ')' | ']' | '}' | '>')
                && has_unmatched_closing_delimiter(remaining, ch);
        if !trim {
            break;
        }
        end -= ch.len_utf8();
    }
    end
}

fn has_unmatched_closing_delimiter(candidate: &str, closing: char) -> bool {
    let opening = match closing {
        ')' => '(',
        ']' => '[',
        '}' => '{',
        '>' => '<',
        _ => return false,
    };
    candidate.chars().filter(|ch| *ch == closing).count()
        > candidate.chars().filter(|ch| *ch == opening).count()
}

fn osc8_start(destination: &str) -> String {
    format!("\x1b]8;;{destination}\x1b\\")
}

fn osc8_end() -> String {
    "\x1b]8;;\x1b\\".to_string()
}

fn push_styled_content(out: &mut Vec<Span<'static>>, content: &str, style: ratatui::style::Style) {
    if let Some(last) = out.last_mut() {
        if last.style == style {
            last.content.to_mut().push_str(content);
            return;
        }
    }
    out.push(Span::styled(content.to_string(), style));
}

fn append_to_last_span(out: &mut [Span<'static>], content: &str) {
    if let Some(last) = out.last_mut() {
        last.content.to_mut().push_str(content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::wrapping::line_to_plain;

    fn plain(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn osc8_wraps_and_strips_with_st_terminator() {
        let linked = osc8_hyperlink("https://example.com", "site");
        assert_eq!(
            linked,
            "\x1b]8;;https://example.com\x1b\\site\x1b]8;;\x1b\\"
        );
        assert_eq!(strip_osc8(&linked), "site");
    }

    #[test]
    fn decorates_markdown_link_range_without_changing_visible_text() {
        let mut line = HyperlinkLine::new(Line::default());
        line.push_span(Span::raw("read "), None);
        line.push_span(Span::raw("site"), Some("https://example.com"));
        let decorated = line_with_osc8(&line, true);
        let snapshot = plain(&decorated);
        assert_eq!(strip_osc8(&snapshot), "read site");
        assert!(snapshot.contains("\x1b]8;;https://example.com\x1b\\site\x1b]8;;\x1b\\"));
        assert_eq!(plain(&line_with_osc8(&line, false)), "read site");
    }

    #[test]
    fn annotates_punctuated_web_urls() {
        assert_eq!(
            annotate_web_urls_in_line(Line::from("See (https://example.com/a).")),
            HyperlinkLine {
                line: Line::from("See (https://example.com/a)."),
                hyperlinks: vec![TerminalHyperlink {
                    columns: 5..26,
                    destination: "https://example.com/a".to_string(),
                }],
            }
        );
    }

    #[test]
    fn prefix_hyperlink_lines_shifts_existing_ranges() {
        let line = annotate_web_urls_in_line(Line::from("Read https://example.com/docs"));

        let lines = prefix_hyperlink_lines(vec![line], Span::raw("• "), Span::raw("  "));

        assert_eq!(
            line_to_plain(&lines[0].line),
            "• Read https://example.com/docs"
        );
        assert_eq!(
            lines[0].hyperlinks,
            vec![TerminalHyperlink {
                columns: 7..31,
                destination: "https://example.com/docs".to_string(),
            }]
        );
    }

    #[test]
    fn adaptive_wrap_hyperlink_lines_preserves_wrapped_url_destination() {
        let source =
            annotate_web_urls_in_line(Line::from("Read https://example.com/docs and keep going"));

        let lines = adaptive_wrap_hyperlink_lines(&[source], RtOptions::new(18));

        assert_eq!(
            lines
                .iter()
                .map(|line| line_to_plain(&line.line))
                .collect::<Vec<_>>(),
            vec!["Read", "https://example.com/docs", "and keep going"]
        );
        assert_eq!(
            lines[1].hyperlinks,
            vec![TerminalHyperlink {
                columns: 0..24,
                destination: "https://example.com/docs".to_string(),
            }]
        );
    }

    #[test]
    fn env_probe_respects_no_color_and_known_terminals() {
        assert!(!hyperlinks_enabled_from_probe(EnvProbe {
            no_color: true,
            term_program: Some("iTerm.app"),
            ..EnvProbe::default()
        }));
        assert!(hyperlinks_enabled_from_probe(EnvProbe {
            term_program: Some("iTerm.app"),
            ..EnvProbe::default()
        }));
        assert!(hyperlinks_enabled_from_probe(EnvProbe {
            force: Some(true),
            ..EnvProbe::default()
        }));
        assert!(!hyperlinks_enabled_from_probe(EnvProbe {
            force: Some(false),
            term_program: Some("iTerm.app"),
            ..EnvProbe::default()
        }));
    }
}
