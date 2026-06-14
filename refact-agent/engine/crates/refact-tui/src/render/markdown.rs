use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::color_enabled_from_env;
use super::diff::{is_unified_diff, render_unified_diff};
use super::highlight::highlight_code;
use super::wrapping::{line_to_plain, line_width, pad_line, wrap_line};
use crate::vendored::decoded_text_merge::DecodedTextMerge;
use crate::vendored::terminal_hyperlinks::{
    plain_hyperlink_lines, remap_wrapped_line, visible_lines, HyperlinkLine,
};

#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownRenderer {
    width: Option<usize>,
    color_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    pub width: Option<usize>,
    pub color_enabled: bool,
}

impl RenderOptions {
    pub fn new(width: Option<usize>) -> Self {
        Self {
            width,
            color_enabled: color_enabled_from_env(),
        }
    }

    pub fn plain(width: Option<usize>) -> Self {
        Self {
            width,
            color_enabled: false,
        }
    }
}

impl MarkdownRenderer {
    pub fn new(width: Option<usize>) -> Self {
        Self {
            width,
            color_enabled: color_enabled_from_env(),
        }
    }

    pub fn plain(width: Option<usize>) -> Self {
        Self {
            width,
            color_enabled: false,
        }
    }

    pub fn render(&self, source: &str) -> Vec<Line<'static>> {
        render_markdown_with_options(
            source,
            RenderOptions {
                width: self.width,
                color_enabled: self.color_enabled,
            },
        )
    }

    pub fn render_with_links(&self, source: &str) -> Vec<HyperlinkLine> {
        render_markdown_hyperlink_lines_with_options(
            source,
            RenderOptions {
                width: self.width,
                color_enabled: self.color_enabled,
            },
        )
    }
}

pub fn render_markdown(source: &str, width: Option<usize>) -> Vec<Line<'static>> {
    render_markdown_with_options(source, RenderOptions::new(width))
}

pub fn render_markdown_hyperlink_lines_with_options(
    source: &str,
    options: RenderOptions,
) -> Vec<HyperlinkLine> {
    if is_unified_diff(source) {
        return plain_hyperlink_lines(render_unified_diff(
            source,
            options.width,
            options.color_enabled,
        ));
    }

    let mut parser_options = Options::empty();
    parser_options.insert(Options::ENABLE_TABLES);
    parser_options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(source, parser_options).into_offset_iter();
    let mut writer = Writer::new(options);
    for (event, _) in DecodedTextMerge::new(parser) {
        writer.handle_event(event);
    }
    writer.finish_with_links()
}

pub fn render_markdown_with_options(source: &str, options: RenderOptions) -> Vec<Line<'static>> {
    if is_unified_diff(source) {
        return render_unified_diff(source, options.width, options.color_enabled);
    }

    let mut parser_options = Options::empty();
    parser_options.insert(Options::ENABLE_TABLES);
    parser_options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(source, parser_options).into_offset_iter();
    let mut writer = Writer::new(options);
    for (event, _) in DecodedTextMerge::new(parser) {
        writer.handle_event(event);
    }
    writer.finish()
}

struct Writer {
    options: RenderOptions,
    out: Vec<HyperlinkLine>,
    current: HyperlinkLine,
    link_stack: Vec<String>,
    style_stack: Vec<Style>,
    list_depth: usize,
    list_stack: Vec<Option<u64>>,
    table: Option<TableState>,
    code_block: Option<CodeBlockState>,
}

impl Writer {
    fn new(options: RenderOptions) -> Self {
        Self {
            options,
            out: Vec::new(),
            current: HyperlinkLine::new(Line::default()),
            link_stack: Vec::new(),
            style_stack: vec![Style::default()],
            list_depth: 0,
            list_stack: Vec::new(),
            table: None,
            code_block: None,
        }
    }

    fn finish(self) -> Vec<Line<'static>> {
        visible_lines(self.finish_with_links())
    }

    fn finish_with_links(mut self) -> Vec<HyperlinkLine> {
        if self.code_block.is_some() {
            self.flush_code_block();
        }
        if self.table.is_some() {
            self.flush_table();
        }
        self.flush_line();
        self.out
    }

    fn handle_event(&mut self, event: Event<'_>) {
        if self.code_block.is_some() {
            self.handle_code_event(event);
            return;
        }

        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.push_text(text.as_ref()),
            Event::Code(code) => self.push_span(Span::styled(
                code.to_string(),
                self.current_style().patch(self.style(MarkdownStyle::Code)),
            )),
            Event::SoftBreak => self.push_text(" "),
            Event::HardBreak => self.hard_break(),
            Event::Rule => {
                self.flush_line();
                self.out.push(HyperlinkLine::new(Line::from(Span::styled(
                    "─".repeat(self.options.width.unwrap_or(24).min(80)),
                    self.style(MarkdownStyle::Muted),
                ))));
            }
            Event::Html(html) | Event::InlineHtml(html) => self.push_span(Span::styled(
                html.to_string(),
                self.current_style().patch(self.style(MarkdownStyle::Muted)),
            )),
            _ => {}
        }
    }

    fn handle_code_event(&mut self, event: Event<'_>) {
        match event {
            Event::End(TagEnd::CodeBlock) => self.flush_code_block(),
            Event::Text(text) => {
                if let Some(code_block) = &mut self.code_block {
                    code_block.source.push_str(text.as_ref());
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(code_block) = &mut self.code_block {
                    code_block.source.push('\n');
                }
            }
            _ => {}
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush_line();
                let style = self.style(MarkdownStyle::Heading);
                self.style_stack.push(style);
                self.push_span(Span::styled(
                    format!("{} ", heading_marker(level as u8)),
                    style,
                ));
            }
            Tag::Emphasis => self.push_merged_style(Modifier::ITALIC, None),
            Tag::Strong => self.push_merged_style(Modifier::BOLD, None),
            Tag::Strikethrough => self.push_merged_style(Modifier::CROSSED_OUT, None),
            Tag::CodeBlock(kind) => {
                self.flush_line();
                self.code_block = Some(CodeBlockState {
                    lang: code_block_lang(kind),
                    source: String::new(),
                });
            }
            Tag::BlockQuote => {
                self.flush_line();
                self.push_span(Span::styled("▍ ", self.style(MarkdownStyle::Quote)));
                self.push_merged_style(Modifier::empty(), Some(Color::Green));
            }
            Tag::List(start) => {
                self.flush_line();
                self.list_depth = self.list_depth.saturating_add(1);
                self.list_stack.push(start);
            }
            Tag::Item => {
                self.flush_line();
                let indent = "  ".repeat(self.list_depth.saturating_sub(1));
                let marker = self.next_list_marker();
                self.push_span(Span::styled(
                    format!("{indent}{marker} "),
                    self.style(MarkdownStyle::ListMarker),
                ));
            }
            Tag::Link { dest_url, .. } => {
                self.link_stack.push(dest_url.to_string());
                self.push_merged_style(Modifier::UNDERLINED, Some(Color::Cyan));
            }
            Tag::Table(alignments) => {
                self.flush_line();
                self.table = Some(TableState::new(alignments));
            }
            Tag::TableHead => {
                if let Some(table) = &mut self.table {
                    table.in_header = true;
                }
            }
            Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.start_row();
                }
            }
            Tag::TableCell => {
                if let Some(table) = &mut self.table {
                    table.start_cell();
                }
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.style_stack.pop();
                self.flush_line();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.style_stack.pop();
            }
            TagEnd::Link => {
                self.style_stack.pop();
                self.link_stack.pop();
            }
            TagEnd::BlockQuote => {
                self.style_stack.pop();
                self.flush_line();
            }
            TagEnd::List(_) => {
                self.flush_line();
                self.list_depth = self.list_depth.saturating_sub(1);
                self.list_stack.pop();
            }
            TagEnd::Item => self.flush_line(),
            TagEnd::Paragraph => self.flush_line(),
            TagEnd::TableCell => {
                if let Some(table) = &mut self.table {
                    table.end_cell();
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = &mut self.table {
                    table.end_row();
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    table.end_header();
                }
            }
            TagEnd::Table => self.flush_table(),
            _ => {}
        }
    }

    fn push_text(&mut self, text: &str) {
        self.push_span(Span::styled(text.to_string(), self.current_style()));
    }

    fn push_span(&mut self, span: Span<'static>) {
        if let Some(table) = &mut self.table {
            table.push_span(span);
        } else {
            self.current
                .push_span(span, self.link_stack.last().map(String::as_str));
        }
    }

    fn hard_break(&mut self) {
        if let Some(table) = &mut self.table {
            table.hard_break();
        } else {
            self.flush_line();
        }
    }

    fn flush_line(&mut self) {
        if self.current.line.spans.is_empty() {
            return;
        }
        let source = std::mem::take(&mut self.current);
        let wrapped = wrap_line(source.line.clone(), self.options.width);
        self.out.extend(remap_wrapped_line(&source, wrapped));
    }

    fn flush_code_block(&mut self) {
        let Some(code_block) = self.code_block.take() else {
            return;
        };
        let label = if code_block.lang.is_empty() {
            "code".to_string()
        } else {
            format!("code · {}", code_block.lang)
        };
        self.out.push(HyperlinkLine::new(Line::from(Span::styled(
            label,
            self.style(MarkdownStyle::Muted),
        ))));
        for line in highlight_code(
            &code_block.source,
            &code_block.lang,
            self.options.color_enabled,
        ) {
            let mut spans = vec![Span::styled("  ", self.style(MarkdownStyle::Muted))];
            spans.extend(line.spans);
            self.out.extend(plain_hyperlink_lines(wrap_line(
                Line::from(spans),
                self.options.width,
            )));
        }
    }

    fn flush_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        self.out
            .extend(plain_hyperlink_lines(render_table(table, self.options)));
    }

    fn push_merged_style(&mut self, modifier: Modifier, color: Option<Color>) {
        let mut style = self.current_style().add_modifier(modifier);
        if self.options.color_enabled {
            if let Some(color) = color {
                style = style.fg(color);
            }
        }
        self.style_stack.push(style);
    }

    fn current_style(&self) -> Style {
        self.style_stack.last().copied().unwrap_or_default()
    }

    fn style(&self, style: MarkdownStyle) -> Style {
        let color_enabled = self.options.color_enabled;
        match style {
            MarkdownStyle::Heading => Style::default().add_modifier(Modifier::BOLD),
            MarkdownStyle::Code => {
                if color_enabled {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }
            }
            MarkdownStyle::Muted => {
                if color_enabled {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                }
            }
            MarkdownStyle::Quote => {
                if color_enabled {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                }
            }
            MarkdownStyle::ListMarker => {
                if color_enabled {
                    Style::default().fg(Color::Blue)
                } else {
                    Style::default()
                }
            }
        }
    }

    fn next_list_marker(&mut self) -> String {
        let Some(last) = self.list_stack.last_mut() else {
            return "•".to_string();
        };
        if let Some(next) = last {
            let marker = format!("{next}.");
            *next += 1;
            marker
        } else {
            "•".to_string()
        }
    }
}

#[derive(Clone, Copy)]
enum MarkdownStyle {
    Heading,
    Code,
    Muted,
    Quote,
    ListMarker,
}

struct CodeBlockState {
    lang: String,
    source: String,
}

#[derive(Default, Clone)]
struct TableCell {
    lines: Vec<Line<'static>>,
}

impl TableCell {
    fn push_span(&mut self, span: Span<'static>) {
        self.ensure_line();
        self.lines.last_mut().unwrap().spans.push(span);
    }

    fn hard_break(&mut self) {
        self.lines.push(Line::default());
    }

    fn ensure_line(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(Line::default());
        }
    }

    fn plain_text(&self) -> String {
        self.lines
            .iter()
            .map(line_to_plain)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

struct TableState {
    alignments: Vec<Alignment>,
    header: Vec<TableCell>,
    rows: Vec<Vec<TableCell>>,
    current_row: Vec<TableCell>,
    current_cell: Option<TableCell>,
    in_header: bool,
}

impl TableState {
    fn new(alignments: Vec<Alignment>) -> Self {
        Self {
            alignments,
            header: Vec::new(),
            rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: None,
            in_header: false,
        }
    }

    fn start_row(&mut self) {
        self.current_row.clear();
        self.current_cell = None;
    }

    fn end_row(&mut self) {
        if self.current_cell.is_some() {
            self.end_cell();
        }
        let row = std::mem::take(&mut self.current_row);
        if self.in_header {
            self.header = row;
        } else if !row.is_empty() {
            self.rows.push(row);
        }
    }

    fn end_header(&mut self) {
        if self.current_cell.is_some() {
            self.end_cell();
        }
        let row = std::mem::take(&mut self.current_row);
        if !row.is_empty() {
            self.header = row;
        }
        self.in_header = false;
    }

    fn start_cell(&mut self) {
        self.current_cell = Some(TableCell::default());
    }

    fn end_cell(&mut self) {
        if let Some(cell) = self.current_cell.take() {
            self.current_row.push(cell);
        }
    }

    fn push_span(&mut self, span: Span<'static>) {
        self.current_cell
            .get_or_insert_with(TableCell::default)
            .push_span(span);
    }

    fn hard_break(&mut self) {
        self.current_cell
            .get_or_insert_with(TableCell::default)
            .hard_break();
    }
}

fn render_table(table: TableState, options: RenderOptions) -> Vec<Line<'static>> {
    let column_count = table
        .alignments
        .len()
        .max(table.header.len())
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
    if column_count == 0 {
        return Vec::new();
    }

    let header = normalize_row(table.header, column_count);
    let rows = table
        .rows
        .into_iter()
        .map(|row| normalize_row(row, column_count))
        .collect::<Vec<_>>();
    let widths = table_widths(&header, &rows, options.width);
    let table_width = widths.iter().sum::<usize>() + (column_count.saturating_sub(1) * 3);
    let mut out = Vec::new();

    if !header.iter().all(|cell| cell.plain_text().is_empty()) {
        out.extend(render_table_row(
            &header,
            &widths,
            &table.alignments,
            options,
            true,
        ));
        out.push(Line::from(Span::styled(
            "━".repeat(table_width),
            table_separator_style(options, true),
        )));
    }

    for (idx, row) in rows.iter().enumerate() {
        out.extend(render_table_row(
            row,
            &widths,
            &table.alignments,
            options,
            false,
        ));
        if idx + 1 < rows.len() {
            out.push(Line::from(Span::styled(
                "─".repeat(table_width),
                table_separator_style(options, false),
            )));
        }
    }
    out
}

fn normalize_row(mut row: Vec<TableCell>, column_count: usize) -> Vec<TableCell> {
    row.resize_with(column_count, TableCell::default);
    row.truncate(column_count);
    row
}

fn table_widths(header: &[TableCell], rows: &[Vec<TableCell>], width: Option<usize>) -> Vec<usize> {
    let column_count = header.len();
    let mut widths = vec![3usize; column_count];
    for row in std::iter::once(header).chain(rows.iter().map(Vec::as_slice)) {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(natural_cell_width(cell));
        }
    }

    let Some(available) = width else {
        return widths;
    };
    let separators = column_count.saturating_sub(1) * 3;
    let min_total = column_count * 3 + separators;
    if available <= min_total {
        return vec![3; column_count];
    }
    let target = available.saturating_sub(separators);
    while widths.iter().sum::<usize>() > target {
        let Some((idx, value)) = widths
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, value)| *value > 3)
            .max_by_key(|(_, value)| *value)
        else {
            break;
        };
        widths[idx] = value - 1;
    }
    widths
}

fn render_table_row(
    row: &[TableCell],
    widths: &[usize],
    alignments: &[Alignment],
    options: RenderOptions,
    is_header: bool,
) -> Vec<Line<'static>> {
    let wrapped = row
        .iter()
        .zip(widths)
        .map(|(cell, width)| wrap_cell(cell, *width, options, is_header))
        .collect::<Vec<_>>();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    let mut out = Vec::new();
    for line_idx in 0..height {
        let mut spans = Vec::new();
        for col_idx in 0..row.len() {
            if col_idx > 0 {
                spans.push(Span::styled(" │ ", table_separator_style(options, false)));
            }
            let blank;
            let line = if let Some(line) = wrapped[col_idx].get(line_idx) {
                line.clone()
            } else {
                blank = Line::from(" ".repeat(widths[col_idx]));
                blank
            };
            let aligned = align_line(line, widths[col_idx], alignments.get(col_idx).copied());
            spans.extend(aligned.spans);
        }
        out.push(Line::from(spans));
    }
    out
}

fn wrap_cell(
    cell: &TableCell,
    width: usize,
    options: RenderOptions,
    is_header: bool,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let style = if is_header {
        let style = Style::default().add_modifier(Modifier::BOLD);
        if options.color_enabled {
            style.fg(Color::White)
        } else {
            style
        }
    } else {
        Style::default()
    };
    for source in &cell.lines {
        let line = Line::from(
            source
                .spans
                .iter()
                .map(|span| Span::styled(span.content.to_string(), span.style.patch(style)))
                .collect::<Vec<_>>(),
        );
        out.extend(wrap_line(line, Some(width)));
    }
    if out.is_empty() {
        out.push(Line::default());
    }
    out
}

fn align_line(line: Line<'static>, width: usize, alignment: Option<Alignment>) -> Line<'static> {
    let current = line_width(&line);
    if current >= width {
        return line;
    }
    let pad = width - current;
    match alignment.unwrap_or(Alignment::Left) {
        Alignment::Right => {
            let mut spans = vec![Span::raw(" ".repeat(pad))];
            spans.extend(line.spans);
            Line::from(spans)
        }
        Alignment::Center => {
            let left = pad / 2;
            let right = pad - left;
            let mut spans = vec![Span::raw(" ".repeat(left))];
            spans.extend(line.spans);
            spans.push(Span::raw(" ".repeat(right)));
            Line::from(spans)
        }
        Alignment::Left | Alignment::None => pad_line(line, width),
    }
}

fn natural_cell_width(cell: &TableCell) -> usize {
    cell.plain_text()
        .split_whitespace()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
        .max(cell.lines.iter().map(line_width).max().unwrap_or(0).min(24))
        .max(3)
}

fn table_separator_style(options: RenderOptions, header: bool) -> Style {
    if !options.color_enabled {
        return if header {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
    }
    let style = Style::default().fg(Color::DarkGray);
    if header {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn heading_marker(level: u8) -> &'static str {
    match level {
        1 => "#",
        2 => "##",
        3 => "###",
        4 => "####",
        5 => "#####",
        _ => "######",
    }
}

fn code_block_lang(kind: CodeBlockKind<'_>) -> String {
    match kind {
        CodeBlockKind::Fenced(lang) => lang.to_string(),
        CodeBlockKind::Indented => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn text(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(line_to_plain).collect()
    }

    #[test]
    fn renders_markdown_headings_and_code() {
        let lines = render_markdown_with_options(
            "# Title\nhello `world`\n\n```rust\nfn main() {}\n```",
            RenderOptions {
                width: Some(80),
                color_enabled: true,
            },
        );
        let rendered = text(&lines);
        assert!(rendered.iter().any(|line| line.contains("# Title")));
        assert!(rendered.iter().any(|line| line.contains("hello world")));
        assert!(rendered.iter().any(|line| line.contains("code · rust")));
        assert!(rendered.iter().any(|line| line.contains("fn main")));
    }

    #[test]
    fn wraps_long_markdown_without_breaking_fitting_url() {
        let url = "https://example.com/alpha-beta/gamma";
        let lines = render_markdown_with_options(
            &format!("Read {url} today please"),
            RenderOptions::plain(Some(url.width())),
        );
        let rendered = text(&lines);
        assert!(rendered.iter().any(|line| line == url));
    }

    #[test]
    fn markdown_link_fixture_has_enabled_and_disabled_osc8_snapshots() {
        let lines = render_markdown_hyperlink_lines_with_options(
            "Read [docs](https://example.com/docs) today",
            RenderOptions {
                width: Some(80),
                color_enabled: true,
            },
        );
        assert_eq!(line_to_plain(&lines[0].line), "Read docs today");
        assert_eq!(
            lines[0].hyperlinks[0].destination,
            "https://example.com/docs"
        );

        let enabled = crate::vendored::terminal_hyperlinks::line_with_osc8(&lines[0], true);
        let enabled_text = line_to_plain(&enabled);
        assert_eq!(
            crate::vendored::terminal_hyperlinks::strip_osc8(&enabled_text),
            "Read docs today"
        );
        assert!(enabled_text.contains("\x1b]8;;https://example.com/docs\x1b\\docs\x1b]8;;\x1b\\"));

        let disabled = crate::vendored::terminal_hyperlinks::line_with_osc8(&lines[0], false);
        assert_eq!(line_to_plain(&disabled), "Read docs today");
    }

    #[test]
    fn renders_aligned_table_snapshot() {
        let source = "| Name | Count | Note |\n| --- | ---: | :---: |\n| α | 2 | small |\n| longer | 12 | wrapped words here |";
        let lines = render_markdown_with_options(source, RenderOptions::plain(Some(32)));
        let rendered = text(&lines).join("\n");
        let expected = "Name   │ Count │      Note      \n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\nα      │     2 │      small     \n────────────────────────────────\nlonger │    12 │  wrapped words \n       │       │      here      ";
        assert_eq!(rendered, expected);
        assert!(rendered.lines().all(|line| line.width() <= 32));
    }

    #[test]
    fn code_block_highlights_rust_and_plain_unknown_language() {
        let highlighted = render_markdown_with_options(
            "```rust\nfn main() {}\n```",
            RenderOptions {
                width: Some(80),
                color_enabled: true,
            },
        );
        let code_line = highlighted
            .iter()
            .find(|line| line_to_plain(line).contains("fn main"))
            .unwrap();
        assert!(code_line.spans.iter().any(|span| span.style.fg.is_some()));

        let plain =
            render_markdown_with_options("```unknown\nhello\n```", RenderOptions::plain(Some(80)));
        let plain_line = plain
            .iter()
            .find(|line| line_to_plain(line).contains("hello"))
            .unwrap();
        assert!(plain_line.spans.iter().all(|span| span.style.fg.is_none()));
    }

    #[test]
    fn no_color_table_and_code_are_legible() {
        let lines = render_markdown_with_options(
            "| A | B |\n|---|---|\n| one | two |\n\n```rust\nfn main() {}\n```",
            RenderOptions::plain(Some(40)),
        );
        let rendered = text(&lines).join("\n");
        assert!(rendered.contains("A   │ B"));
        assert!(rendered.contains("fn main"));
        assert!(lines
            .iter()
            .flat_map(|line| &line.spans)
            .all(|span| span.style.fg != Some(Color::Green)));
    }

    #[test]
    fn unified_diff_uses_diff_renderer() {
        let lines = render_markdown_with_options(
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new",
            RenderOptions {
                width: Some(80),
                color_enabled: true,
            },
        );
        let rendered = text(&lines);
        assert!(rendered.contains(&"- old".to_string()));
        assert!(rendered.contains(&"+ new".to_string()));
        let add = lines
            .iter()
            .find(|line| line_to_plain(line) == "+ new")
            .unwrap();
        assert_eq!(add.spans[0].style.fg, Some(Color::Green));
    }
}
