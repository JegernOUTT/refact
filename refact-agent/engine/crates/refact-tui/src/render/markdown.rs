use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use regex_lite::Regex;
use unicode_width::UnicodeWidthStr;
use url::Url;

use super::color_enabled_from_env;
use super::diff::{is_unified_diff, render_unified_diff};
use super::highlight::highlight_code_to_lines;
use super::line_utils::line_to_static;
use super::markdown_table::{render_table, TableRenderStyles, TableState};
use super::wrapping::{adaptive_wrap_line, RtOptions};
use crate::vendored::decoded_text_merge::DecodedTextMerge;
use crate::vendored::terminal_hyperlinks::{
    annotate_web_urls_in_line, plain_hyperlink_lines, remap_wrapped_line, visible_lines,
    web_destination, HyperlinkLine,
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
    let cwd = std::env::current_dir().ok();
    let mut writer = Writer::new(options, cwd, source);
    for (event, range) in DecodedTextMerge::new(parser) {
        writer.handle_event(event, range);
    }
    writer.finish_with_links()
}

pub fn render_markdown_with_options(source: &str, options: RenderOptions) -> Vec<Line<'static>> {
    if is_unified_diff(source) {
        return render_unified_diff(source, options.width, options.color_enabled);
    }

    visible_lines(render_markdown_hyperlink_lines_with_options(
        source, options,
    ))
}

#[derive(Clone, Debug)]
struct MarkdownStyles {
    h1: Style,
    h2: Style,
    h3: Style,
    h4: Style,
    h5: Style,
    h6: Style,
    code: Style,
    emphasis: Style,
    strong: Style,
    strikethrough: Style,
    ordered_list_marker: Style,
    unordered_list_marker: Style,
    link: Style,
    blockquote: Style,
    muted: Style,
    table_separator: Style,
}

impl MarkdownStyles {
    fn new(color_enabled: bool) -> Self {
        let color = |color| {
            if color_enabled {
                Style::default().fg(color)
            } else {
                Style::default()
            }
        };
        Self {
            h1: Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            h2: Style::default().add_modifier(Modifier::BOLD),
            h3: Style::default().add_modifier(Modifier::BOLD | Modifier::ITALIC),
            h4: Style::default().add_modifier(Modifier::ITALIC),
            h5: Style::default().add_modifier(Modifier::ITALIC),
            h6: Style::default().add_modifier(Modifier::ITALIC),
            code: color(Color::Cyan),
            emphasis: Style::default().add_modifier(Modifier::ITALIC),
            strong: Style::default().add_modifier(Modifier::BOLD),
            strikethrough: Style::default().add_modifier(Modifier::CROSSED_OUT),
            ordered_list_marker: color(Color::LightBlue),
            unordered_list_marker: Style::default(),
            link: color(Color::Cyan).add_modifier(Modifier::UNDERLINED),
            blockquote: color(Color::Green),
            muted: color(Color::DarkGray),
            table_separator: color(Color::DarkGray),
        }
    }

    fn heading(&self, level: HeadingLevel) -> Style {
        match level {
            HeadingLevel::H1 => self.h1,
            HeadingLevel::H2 => self.h2,
            HeadingLevel::H3 => self.h3,
            HeadingLevel::H4 => self.h4,
            HeadingLevel::H5 => self.h5,
            HeadingLevel::H6 => self.h6,
        }
    }
}

#[derive(Clone, Debug)]
struct IndentContext {
    prefix: Vec<Span<'static>>,
    marker: Option<Vec<Span<'static>>>,
    is_list: bool,
}

impl IndentContext {
    fn new(prefix: Vec<Span<'static>>, marker: Option<Vec<Span<'static>>>, is_list: bool) -> Self {
        Self {
            prefix,
            marker,
            is_list,
        }
    }
}

#[derive(Clone, Debug)]
struct LinkState {
    destination: String,
    show_destination: bool,
    local_target_display: Option<String>,
}

struct Writer {
    options: RenderOptions,
    out: Vec<HyperlinkLine>,
    source: String,
    styles: MarkdownStyles,
    inline_styles: Vec<Style>,
    indent_stack: Vec<IndentContext>,
    list_indices: Vec<Option<u64>>,
    list_needs_blank_before_next_item: Vec<bool>,
    list_item_start_line_counts: Vec<usize>,
    link: Option<LinkState>,
    needs_newline: bool,
    pending_marker_line: bool,
    code_block: Option<CodeBlockState>,
    table: Option<TableState>,
    current_line_content: Option<HyperlinkLine>,
    current_initial_indent: Vec<Span<'static>>,
    current_subsequent_indent: Vec<Span<'static>>,
    current_line_style: Style,
    line_ends_with_local_link_target: bool,
    pending_local_link_soft_break: bool,
    cwd: Option<PathBuf>,
}

impl Writer {
    fn new(options: RenderOptions, cwd: Option<PathBuf>, source: &str) -> Self {
        Self {
            options,
            out: Vec::new(),
            source: source.to_string(),
            styles: MarkdownStyles::new(options.color_enabled),
            inline_styles: Vec::new(),
            indent_stack: Vec::new(),
            list_indices: Vec::new(),
            list_needs_blank_before_next_item: Vec::new(),
            list_item_start_line_counts: Vec::new(),
            link: None,
            needs_newline: false,
            pending_marker_line: false,
            code_block: None,
            table: None,
            current_line_content: None,
            current_initial_indent: Vec::new(),
            current_subsequent_indent: Vec::new(),
            current_line_style: Style::default(),
            line_ends_with_local_link_target: false,
            pending_local_link_soft_break: false,
            cwd,
        }
    }

    fn finish_with_links(mut self) -> Vec<HyperlinkLine> {
        if self.code_block.is_some() {
            self.flush_code_block();
        }
        if self.table.is_some() {
            self.flush_table();
        }
        self.flush_current_line();
        self.out
    }

    fn handle_event(&mut self, event: Event<'_>, range: Range<usize>) {
        if self.code_block.is_some() {
            self.handle_code_event(event);
            return;
        }
        self.prepare_for_event(&event);
        match event {
            Event::Start(tag) => self.start_tag(tag, range),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.text(text.as_ref()),
            Event::Code(code) => self.code(code.to_string()),
            Event::SoftBreak => self.soft_break(),
            Event::HardBreak => self.hard_break(),
            Event::Rule => self.rule(),
            Event::Html(html) | Event::InlineHtml(html) => self.html(html.as_ref()),
            _ => {}
        }
    }

    fn prepare_for_event(&mut self, event: &Event<'_>) {
        if !self.pending_local_link_soft_break {
            return;
        }
        if matches!(event, Event::Text(text) if text.trim_start().starts_with(':')) {
            self.pending_local_link_soft_break = false;
            return;
        }
        self.pending_local_link_soft_break = false;
        self.push_line(Line::default());
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

    fn start_tag(&mut self, tag: Tag<'_>, range: Range<usize>) {
        match tag {
            Tag::Paragraph => self.start_paragraph(),
            Tag::Heading { level, .. } => self.start_heading(level),
            Tag::Emphasis => self.push_inline_style(self.styles.emphasis),
            Tag::Strong => self.push_inline_style(self.styles.strong),
            Tag::Strikethrough => self.push_inline_style(self.styles.strikethrough),
            Tag::CodeBlock(kind) => self.start_code_block(kind),
            Tag::BlockQuote => self.start_blockquote(),
            Tag::List(start) => self.start_list(start),
            Tag::Item => self.start_item(),
            Tag::Link { dest_url, .. } => self.push_link(dest_url.to_string()),
            Tag::Table(alignments) => self.start_table(alignments),
            Tag::TableHead => {
                if let Some(table) = &mut self.table {
                    table.start_header();
                }
            }
            Tag::TableRow => {
                let has_table_pipe_syntax = self.has_table_row_boundary_pipe(range);
                if let Some(table) = &mut self.table {
                    table.start_row(has_table_pipe_syntax);
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
            TagEnd::Paragraph => self.end_paragraph(),
            TagEnd::Heading(_) => self.end_heading(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_inline_style(),
            TagEnd::Link => self.pop_link(),
            TagEnd::BlockQuote => self.end_blockquote(),
            TagEnd::List(_) => self.end_list(),
            TagEnd::Item => self.end_item(),
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

    fn start_paragraph(&mut self) {
        if self.in_table_cell() {
            return;
        }
        if self.needs_newline {
            self.push_blank_line();
            self.needs_newline = false;
        }
        self.push_line(Line::default());
    }

    fn end_paragraph(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.flush_current_line();
        self.needs_newline = true;
        self.pending_marker_line = false;
    }

    fn start_heading(&mut self, level: HeadingLevel) {
        if self.in_table_cell() {
            return;
        }
        self.flush_current_line();
        if self.needs_newline && !self.out.is_empty() {
            self.push_blank_line();
        }
        let style = self.styles.heading(level);
        self.push_line(Line::from(Span::styled(
            format!("{} ", "#".repeat(level as usize)),
            style,
        )));
        self.push_inline_style(style);
        self.needs_newline = false;
    }

    fn end_heading(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.pop_inline_style();
        self.flush_current_line();
        self.needs_newline = true;
    }

    fn start_blockquote(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.flush_current_line();
        if self.needs_newline && !self.out.is_empty() {
            self.push_blank_line();
        }
        self.indent_stack.push(IndentContext::new(
            vec![Span::styled("> ", self.styles.blockquote)],
            None,
            false,
        ));
        self.needs_newline = false;
    }

    fn end_blockquote(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.flush_current_line();
        self.indent_stack.pop();
        self.needs_newline = true;
    }

    fn start_list(&mut self, start: Option<u64>) {
        self.flush_current_line();
        if self.list_indices.is_empty() && self.needs_newline && !self.out.is_empty() {
            self.push_blank_line();
        }
        self.list_indices.push(start);
        self.list_needs_blank_before_next_item.push(false);
        self.needs_newline = false;
    }

    fn end_list(&mut self) {
        self.flush_current_line();
        self.list_indices.pop();
        self.list_needs_blank_before_next_item.pop();
        self.needs_newline = true;
    }

    fn start_item(&mut self) {
        if self
            .list_needs_blank_before_next_item
            .last_mut()
            .map(std::mem::take)
            .unwrap_or(false)
        {
            self.push_blank_line();
        }
        self.flush_current_line();
        self.list_item_start_line_counts.push(self.out.len());
        self.pending_marker_line = true;
        let depth = self.list_indices.len();
        let width = depth.saturating_mul(4).saturating_sub(3).max(1);
        let is_ordered = self
            .list_indices
            .last()
            .map(Option::is_some)
            .unwrap_or(false);
        let marker = self.list_indices.last_mut().map(|last| match last {
            None => vec![Span::styled(
                format!("{}- ", " ".repeat(width.saturating_sub(1))),
                self.styles.unordered_list_marker,
            )],
            Some(index) => {
                let marker = format!("{:width$}. ", *index, width = width);
                *index += 1;
                vec![Span::styled(marker, self.styles.ordered_list_marker)]
            }
        });
        let indent_len = if is_ordered { width + 2 } else { width + 1 };
        self.indent_stack.push(IndentContext::new(
            vec![Span::raw(" ".repeat(indent_len))],
            marker,
            true,
        ));
        self.needs_newline = false;
    }

    fn end_item(&mut self) {
        self.flush_current_line();
        let start_line_count = self.list_item_start_line_counts.pop().unwrap_or_default();
        if self.out.len().saturating_sub(start_line_count) > 1 {
            if let Some(needs_blank) = self.list_needs_blank_before_next_item.last_mut() {
                *needs_blank = true;
            }
        }
        self.indent_stack.pop();
        self.pending_marker_line = false;
    }

    fn start_code_block(&mut self, kind: CodeBlockKind<'_>) {
        self.flush_current_line();
        if self.needs_newline && !self.out.is_empty() {
            self.push_blank_line();
        }
        self.code_block = Some(CodeBlockState {
            lang: code_block_lang(kind),
            source: String::new(),
        });
        self.needs_newline = false;
    }

    fn start_table(&mut self, alignments: Vec<Alignment>) {
        self.flush_current_line();
        if self.needs_newline && !self.out.is_empty() {
            self.push_blank_line();
        }
        self.table = Some(TableState::new(alignments));
        self.needs_newline = false;
    }

    fn text(&mut self, text: &str) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell() {
            self.push_text_to_table_cell(text);
            return;
        }
        if self.pending_marker_line {
            self.push_line(Line::default());
        }
        self.pending_marker_line = false;
        let style = self.current_inline_style();
        for (idx, line) in text.lines().enumerate() {
            if idx > 0 {
                self.push_line(Line::default());
            }
            self.push_text_spans(line, style);
        }
        self.needs_newline = false;
    }

    fn code(&mut self, code: String) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        let span = Span::styled(code, self.current_inline_style().patch(self.styles.code));
        if self.in_table_cell() {
            self.push_span_to_table_cell(span);
            return;
        }
        if self.pending_marker_line {
            self.push_line(Line::default());
            self.pending_marker_line = false;
        }
        self.push_span(span);
    }

    fn html(&mut self, html: &str) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        let style = self.current_inline_style().patch(self.styles.muted);
        for (idx, line) in html.lines().enumerate() {
            if idx > 0 {
                self.push_line(Line::default());
            }
            if self.in_table_cell() {
                self.push_span_to_table_cell(Span::styled(line.to_string(), style));
            } else {
                self.push_span(Span::styled(line.to_string(), style));
            }
        }
    }

    fn soft_break(&mut self) {
        if self.suppressing_local_link_label() {
            return;
        }
        if self.in_table_cell() {
            self.push_span_to_table_cell(Span::styled(" ", self.current_inline_style()));
            return;
        }
        if self.line_ends_with_local_link_target {
            self.pending_local_link_soft_break = true;
            self.line_ends_with_local_link_target = false;
            return;
        }
        self.line_ends_with_local_link_target = false;
        self.push_line(Line::default());
    }

    fn hard_break(&mut self) {
        if self.suppressing_local_link_label() {
            return;
        }
        self.line_ends_with_local_link_target = false;
        if self.in_table_cell() {
            self.push_table_cell_hard_break();
            return;
        }
        self.push_line(Line::default());
    }

    fn rule(&mut self) {
        self.flush_current_line();
        if self.needs_newline && !self.out.is_empty() {
            self.push_blank_line();
        }
        self.push_prewrapped_line(HyperlinkLine::new(Line::from(Span::styled(
            "———",
            self.styles.muted,
        ))));
        self.needs_newline = true;
    }

    fn flush_code_block(&mut self) {
        let Some(code_block) = self.code_block.take() else {
            return;
        };
        let code_lines = if self.options.color_enabled {
            highlight_code_to_lines(&code_block.source, &code_block.lang)
        } else {
            plain_code_lines(&code_block.source)
        };
        let mut pending_marker_line = self.pending_marker_line;
        for line in code_lines {
            self.push_code_line(HyperlinkLine::new(line), pending_marker_line);
            pending_marker_line = false;
        }
        self.pending_marker_line = false;
        self.needs_newline = true;
    }

    fn flush_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        let rendered = render_table(
            table,
            self.available_table_width(),
            TableRenderStyles {
                header: self.styles.strong,
                separator: self.styles.table_separator,
            },
        );
        let mut pending_marker_line = self.pending_marker_line;
        for line in rendered.table_lines {
            if pending_marker_line {
                if rendered.table_lines_prewrapped {
                    self.push_prewrapped_line_with_marker(line, true);
                } else {
                    self.push_ordinary_hyperlink_line_with_marker(line, true);
                }
                pending_marker_line = false;
            } else if rendered.table_lines_prewrapped {
                self.push_prewrapped_line(line);
            } else {
                self.push_ordinary_hyperlink_line(line);
            }
        }
        for line in rendered.spillover_lines {
            self.push_ordinary_hyperlink_line(line);
        }
        self.pending_marker_line = false;
        self.needs_newline = true;
    }

    fn push_link(&mut self, destination: String) {
        self.push_inline_style(self.styles.link);
        self.link = Some(LinkState {
            show_destination: should_render_link_destination(&destination),
            local_target_display: if is_local_path_like_link(&destination) {
                render_local_link_target(&destination, self.cwd.as_deref())
            } else {
                None
            },
            destination,
        });
    }

    fn pop_link(&mut self) {
        self.pop_inline_style();
        let Some(link) = self.link.take() else {
            return;
        };
        if link.show_destination {
            if self.in_table_cell() {
                self.push_span_to_table_cell(Span::raw(" ("));
                let mut destination = HyperlinkLine::new(Line::default());
                destination.push_span(
                    Span::styled(link.destination.clone(), self.styles.link),
                    web_destination(&link.destination).as_deref(),
                );
                self.push_annotated_to_table_cell(destination);
                self.push_span_to_table_cell(Span::raw(")"));
            } else {
                self.push_span(Span::raw(" ("));
                let mut destination = HyperlinkLine::new(Line::default());
                destination.push_span(
                    Span::styled(link.destination.clone(), self.styles.link),
                    web_destination(&link.destination).as_deref(),
                );
                self.push_annotated(destination);
                self.push_span(Span::raw(")"));
            }
        } else if let Some(local_target_display) = link.local_target_display {
            let style = self.current_inline_style().patch(self.styles.code);
            let span = Span::styled(local_target_display, style);
            if self.in_table_cell() {
                self.push_span_to_table_cell(span);
            } else {
                if self.pending_marker_line {
                    self.push_line(Line::default());
                }
                self.push_span(span);
                self.line_ends_with_local_link_target = true;
            }
        }
    }

    fn suppressing_local_link_label(&self) -> bool {
        self.link
            .as_ref()
            .and_then(|link| link.local_target_display.as_ref())
            .is_some()
    }

    fn push_inline_style(&mut self, style: Style) {
        let current = self.current_inline_style();
        self.inline_styles.push(current.patch(style));
    }

    fn pop_inline_style(&mut self) {
        self.inline_styles.pop();
    }

    fn current_inline_style(&self) -> Style {
        self.inline_styles.last().copied().unwrap_or_default()
    }

    fn push_line(&mut self, line: Line<'static>) {
        self.flush_current_line();
        let was_pending = self.pending_marker_line;
        self.current_initial_indent = self.prefix_spans(was_pending);
        self.current_subsequent_indent = self.prefix_spans(false);
        self.current_line_style = if self.is_blockquote_active() {
            self.styles.blockquote.patch(line.style)
        } else {
            line.style
        };
        self.current_line_content = Some(HyperlinkLine::new(line));
        self.line_ends_with_local_link_target = false;
        self.pending_marker_line = false;
    }

    fn push_span(&mut self, span: Span<'static>) {
        if self.current_line_content.is_none() {
            self.push_line(Line::default());
        }
        if let Some(line) = self.current_line_content.as_mut() {
            line.line.spans.push(span);
        }
    }

    fn push_annotated(&mut self, mut appended: HyperlinkLine) {
        if self.current_line_content.is_none() {
            self.push_line(Line::default());
        }
        if let Some(line) = self.current_line_content.as_mut() {
            let shift = line.width();
            line.line.spans.append(&mut appended.line.spans);
            line.hyperlinks
                .extend(appended.hyperlinks.into_iter().map(|mut link| {
                    link.columns = link.columns.start + shift..link.columns.end + shift;
                    link
                }));
        }
    }

    fn push_text_spans(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        let span = Span::styled(text.to_string(), style);
        let destination = self
            .link
            .as_ref()
            .and_then(|link| web_destination(&link.destination));
        let annotated = if let Some(destination) = destination {
            let mut annotated = HyperlinkLine::new(Line::default());
            annotated.push_span(span, Some(&destination));
            annotated
        } else if self.link.is_some() {
            HyperlinkLine::new(Line::from(span))
        } else {
            annotate_web_urls_in_line(Line::from(span))
        };
        self.push_annotated(annotated);
    }

    fn flush_current_line(&mut self) {
        let Some(mut line) = self.current_line_content.take() else {
            return;
        };
        if line.line.spans.is_empty() && self.current_initial_indent.is_empty() {
            self.current_subsequent_indent.clear();
            self.line_ends_with_local_link_target = false;
            return;
        }
        let style = self.current_line_style;
        if let Some(width) = self.options.width.filter(|width| *width > 0) {
            let opts = RtOptions::new(width)
                .initial_indent(Line::from(self.current_initial_indent.clone()))
                .subsequent_indent(Line::from(self.current_subsequent_indent.clone()));
            let wrapped = adaptive_wrap_line(&line.line, opts)
                .into_iter()
                .map(|wrapped| line_to_static(&wrapped))
                .collect();
            for wrapped in remap_wrapped_line(&line, wrapped) {
                self.push_output_line(wrapped.style(style));
            }
            self.current_initial_indent.clear();
            self.current_subsequent_indent.clear();
            self.line_ends_with_local_link_target = false;
            return;
        }
        let mut spans = self.current_initial_indent.clone();
        let shift = spans
            .iter()
            .map(|span| span.content.as_ref().width())
            .sum::<usize>();
        spans.append(&mut line.line.spans);
        for hyperlink in &mut line.hyperlinks {
            hyperlink.columns = hyperlink.columns.start + shift..hyperlink.columns.end + shift;
        }
        line.line = Line::from(spans);
        self.push_output_line(line.style(style));
        self.current_initial_indent.clear();
        self.current_subsequent_indent.clear();
        self.line_ends_with_local_link_target = false;
    }

    fn push_code_line(&mut self, mut line: HyperlinkLine, pending_marker_line: bool) {
        self.flush_current_line();
        let mut spans = self.prefix_spans(pending_marker_line);
        let shift = spans
            .iter()
            .map(|span| span.content.as_ref().width())
            .sum::<usize>();
        spans.append(&mut line.line.spans);
        for hyperlink in &mut line.hyperlinks {
            hyperlink.columns = hyperlink.columns.start + shift..hyperlink.columns.end + shift;
        }
        line.line = Line::from(spans);
        self.push_output_line(line);
    }

    fn push_prewrapped_line(&mut self, line: HyperlinkLine) {
        self.push_prewrapped_line_with_marker(line, false);
    }

    fn push_prewrapped_line_with_marker(
        &mut self,
        mut line: HyperlinkLine,
        pending_marker_line: bool,
    ) {
        self.flush_current_line();
        let style = if self.is_blockquote_active() {
            self.styles.blockquote.patch(line.line.style)
        } else {
            line.line.style
        };
        let mut spans = self.prefix_spans(pending_marker_line);
        let shift = spans
            .iter()
            .map(|span| span.content.as_ref().width())
            .sum::<usize>();
        spans.append(&mut line.line.spans);
        for hyperlink in &mut line.hyperlinks {
            hyperlink.columns = hyperlink.columns.start + shift..hyperlink.columns.end + shift;
        }
        line.line = Line::from(spans);
        self.push_output_line(line.style(style));
    }

    fn push_ordinary_hyperlink_line(&mut self, line: HyperlinkLine) {
        self.push_ordinary_hyperlink_line_with_marker(line, false);
    }

    fn push_ordinary_hyperlink_line_with_marker(
        &mut self,
        line: HyperlinkLine,
        pending_marker_line: bool,
    ) {
        self.flush_current_line();
        self.current_initial_indent = self.prefix_spans(pending_marker_line);
        self.current_subsequent_indent = self.prefix_spans(false);
        self.current_line_style = if self.is_blockquote_active() {
            self.styles.blockquote.patch(line.line.style)
        } else {
            line.line.style
        };
        self.current_line_content = Some(line);
        self.flush_current_line();
    }

    fn push_blank_line(&mut self) {
        self.flush_current_line();
        self.push_output_line(HyperlinkLine::new(Line::default()));
    }

    fn push_output_line(&mut self, line: HyperlinkLine) {
        self.out.push(line);
    }

    fn is_blockquote_active(&self) -> bool {
        self.indent_stack
            .iter()
            .any(|context| context.prefix.iter().any(|span| span.content.contains('>')))
    }

    fn prefix_spans(&self, pending_marker_line: bool) -> Vec<Span<'static>> {
        let mut prefix = Vec::new();
        let last_marker_index = if pending_marker_line {
            self.indent_stack
                .iter()
                .enumerate()
                .rev()
                .find_map(|(idx, context)| context.marker.is_some().then_some(idx))
        } else {
            None
        };
        let last_list_index = self
            .indent_stack
            .iter()
            .rposition(|context| context.is_list);

        for (idx, context) in self.indent_stack.iter().enumerate() {
            if pending_marker_line {
                if Some(idx) == last_marker_index {
                    if let Some(marker) = &context.marker {
                        prefix.extend(marker.iter().cloned());
                        continue;
                    }
                }
                if context.is_list && last_marker_index.is_some_and(|marker_idx| marker_idx > idx) {
                    continue;
                }
            } else if context.is_list && Some(idx) != last_list_index {
                continue;
            }
            prefix.extend(context.prefix.iter().cloned());
        }
        prefix
    }

    fn in_table_cell(&self) -> bool {
        self.table
            .as_ref()
            .is_some_and(TableState::has_current_cell)
    }

    fn push_span_to_table_cell(&mut self, span: Span<'static>) {
        if let Some(table) = &mut self.table {
            table.push_span_to_current_cell(span);
        }
    }

    fn push_annotated_to_table_cell(&mut self, line: HyperlinkLine) {
        if let Some(table) = &mut self.table {
            table.push_annotated_to_current_cell(line);
        }
    }

    fn push_table_cell_hard_break(&mut self) {
        if let Some(table) = &mut self.table {
            table.hard_break_current_cell();
        }
    }

    fn push_text_to_table_cell(&mut self, text: &str) {
        let style = self.current_inline_style();
        for (idx, line) in text.lines().enumerate() {
            if idx > 0 {
                self.push_table_cell_hard_break();
            }
            let span = Span::styled(line.to_string(), style);
            let destination = self
                .link
                .as_ref()
                .and_then(|link| web_destination(&link.destination));
            let annotated = if let Some(destination) = destination {
                let mut annotated = HyperlinkLine::new(Line::default());
                annotated.push_span(span, Some(&destination));
                annotated
            } else if self.link.is_some() {
                HyperlinkLine::new(Line::from(span))
            } else {
                annotate_web_urls_in_line(Line::from(span))
            };
            self.push_annotated_to_table_cell(annotated);
        }
    }

    fn available_table_width(&self) -> Option<usize> {
        self.options.width.map(|width| {
            let prefix_width = self
                .prefix_spans(self.pending_marker_line)
                .iter()
                .map(|span| span.content.as_ref().width())
                .sum::<usize>();
            width.saturating_sub(prefix_width)
        })
    }

    fn has_table_row_boundary_pipe(&self, range: Range<usize>) -> bool {
        let Some(source) = self.source.get(range) else {
            return false;
        };
        let source = source.trim();
        source.starts_with('|') || source.ends_with('|')
    }
}

struct CodeBlockState {
    lang: String,
    source: String,
}

fn should_render_link_destination(dest_url: &str) -> bool {
    !is_local_path_like_link(dest_url)
}

fn is_local_path_like_link(dest_url: &str) -> bool {
    dest_url.starts_with("file://")
        || dest_url.starts_with('/')
        || dest_url.starts_with("~/")
        || dest_url.starts_with("./")
        || dest_url.starts_with("../")
        || dest_url.starts_with("\\\\")
        || matches!(
            dest_url.as_bytes(),
            [drive, b':', separator, ..]
                if drive.is_ascii_alphabetic() && matches!(separator, b'/' | b'\\')
        )
}

static COLON_LOCATION_SUFFIX_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r":\d+(?::\d+)?(?:[-–]\d+(?::\d+)?)?$").expect("valid location regex")
});

static HASH_LOCATION_SUFFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^L\d+(?:C\d+)?(?:-L\d+(?:C\d+)?)?$").expect("valid hash regex"));

fn render_local_link_target(dest_url: &str, cwd: Option<&Path>) -> Option<String> {
    let (path_text, location_suffix) = parse_local_link_target(dest_url)?;
    let mut rendered = display_local_link_path(&path_text, cwd);
    if let Some(location_suffix) = location_suffix {
        rendered.push_str(&location_suffix);
    }
    Some(rendered)
}

fn parse_local_link_target(dest_url: &str) -> Option<(String, Option<String>)> {
    if dest_url.starts_with("file://") {
        let url = Url::parse(dest_url).ok()?;
        let path_text = file_url_to_local_path_text(&url)?;
        let location_suffix = url
            .fragment()
            .and_then(normalize_hash_location_suffix_fragment);
        return Some((path_text, location_suffix));
    }

    let mut path_text = dest_url;
    let mut location_suffix = None;
    if let Some((candidate_path, fragment)) = dest_url.rsplit_once('#') {
        if let Some(normalized) = normalize_hash_location_suffix_fragment(fragment) {
            path_text = candidate_path;
            location_suffix = Some(normalized);
        }
    }
    if location_suffix.is_none() {
        if let Some(suffix) = extract_colon_location_suffix(path_text) {
            let path_len = path_text.len().saturating_sub(suffix.len());
            path_text = &path_text[..path_len];
            location_suffix = Some(suffix);
        }
    }

    Some((expand_local_link_path(path_text), location_suffix))
}

fn normalize_hash_location_suffix_fragment(fragment: &str) -> Option<String> {
    HASH_LOCATION_SUFFIX_RE
        .is_match(fragment)
        .then(|| normalize_markdown_hash_location_suffix(&format!("#{fragment}")))
        .flatten()
}

fn normalize_markdown_hash_location_suffix(suffix: &str) -> Option<String> {
    let fragment = suffix.strip_prefix('#')?;
    let (start, end) = match fragment.split_once('-') {
        Some((start, end)) => (start, Some(end)),
        None => (fragment, None),
    };
    let (start_line, start_column) = parse_markdown_hash_location_point(start)?;
    let mut normalized = String::from(":");
    normalized.push_str(start_line);
    if let Some(column) = start_column {
        normalized.push(':');
        normalized.push_str(column);
    }
    if let Some(end) = end {
        let (end_line, end_column) = parse_markdown_hash_location_point(end)?;
        normalized.push('-');
        normalized.push_str(end_line);
        if let Some(column) = end_column {
            normalized.push(':');
            normalized.push_str(column);
        }
    }
    Some(normalized)
}

fn parse_markdown_hash_location_point(point: &str) -> Option<(&str, Option<&str>)> {
    let point = point.strip_prefix('L')?;
    match point.split_once('C') {
        Some((line, column)) => Some((line, Some(column))),
        None => Some((point, None)),
    }
}

fn extract_colon_location_suffix(path_text: &str) -> Option<String> {
    COLON_LOCATION_SUFFIX_RE
        .find(path_text)
        .filter(|matched| matched.end() == path_text.len())
        .map(|matched| matched.as_str().to_string())
}

fn expand_local_link_path(path_text: &str) -> String {
    if let Some(rest) = path_text.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return normalize_local_link_path_text(&home.join(rest).to_string_lossy());
        }
    }
    normalize_local_link_path_text(path_text)
}

fn file_url_to_local_path_text(url: &Url) -> Option<String> {
    if let Ok(path) = url.to_file_path() {
        return Some(normalize_local_link_path_text(&path.to_string_lossy()));
    }

    let mut path_text = url.path().to_string();
    if let Some(host) = url.host_str() {
        if !host.is_empty() && host != "localhost" {
            path_text = format!("//{host}{path_text}");
        } else if matches!(
            path_text.as_bytes(),
            [b'/', drive, b':', b'/', ..] if drive.is_ascii_alphabetic()
        ) {
            path_text.remove(0);
        }
    }
    Some(normalize_local_link_path_text(&path_text))
}

fn normalize_local_link_path_text(path_text: &str) -> String {
    if let Some(rest) = path_text.strip_prefix("\\\\") {
        format!("//{}", rest.replace('\\', "/").trim_start_matches('/'))
    } else {
        path_text.replace('\\', "/")
    }
}

fn is_absolute_local_link_path(path_text: &str) -> bool {
    path_text.starts_with('/')
        || path_text.starts_with("//")
        || matches!(
            path_text.as_bytes(),
            [drive, b':', b'/', ..] if drive.is_ascii_alphabetic()
        )
}

fn trim_trailing_local_path_separator(path_text: &str) -> &str {
    if path_text == "/" || path_text == "//" {
        return path_text;
    }
    if matches!(path_text.as_bytes(), [drive, b':', b'/'] if drive.is_ascii_alphabetic()) {
        return path_text;
    }
    path_text.trim_end_matches('/')
}

fn strip_local_path_prefix<'a>(path_text: &'a str, cwd_text: &str) -> Option<&'a str> {
    let path_text = trim_trailing_local_path_separator(path_text);
    let cwd_text = trim_trailing_local_path_separator(cwd_text);
    if path_text == cwd_text {
        return None;
    }
    if cwd_text == "/" || cwd_text == "//" {
        return path_text.strip_prefix('/');
    }
    path_text
        .strip_prefix(cwd_text)
        .and_then(|rest| rest.strip_prefix('/'))
}

fn display_local_link_path(path_text: &str, cwd: Option<&Path>) -> String {
    let path_text = normalize_local_link_path_text(path_text);
    if !is_absolute_local_link_path(&path_text) {
        return path_text;
    }
    if let Some(cwd) = cwd {
        let cwd_text = normalize_local_link_path_text(&cwd.to_string_lossy());
        if let Some(stripped) = strip_local_path_prefix(&path_text, &cwd_text) {
            return stripped.to_string();
        }
    }
    path_text
}

fn plain_code_lines(code: &str) -> Vec<Line<'static>> {
    let mut lines = code
        .lines()
        .map(|line| Line::from(line.to_string()))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(Line::from(String::new()));
    }
    lines
}

fn code_block_lang(kind: CodeBlockKind<'_>) -> String {
    match kind {
        CodeBlockKind::Fenced(lang) => lang
            .split(|ch: char| ch.is_whitespace() || ch == ',')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string(),
        CodeBlockKind::Indented => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::wrapping::line_to_plain;
    use ratatui::style::Color;

    fn text(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(line_to_plain).collect()
    }

    #[test]
    fn renders_graduated_heading_styles_and_inline_styles() {
        let lines = render_markdown_with_options(
            "# H1\n\n## H2\n\n### H3\n\n#### H4\n\n**bold** *em* ~~gone~~ `code`",
            RenderOptions {
                width: Some(80),
                color_enabled: true,
            },
        );
        let rendered = text(&lines);
        assert!(rendered.iter().any(|line| line == "# H1"));
        assert!(rendered.iter().any(|line| line == "## H2"));
        assert!(rendered.iter().any(|line| line == "### H3"));
        assert!(rendered.iter().any(|line| line == "#### H4"));
        let h1 = lines
            .iter()
            .find(|line| line_to_plain(line) == "# H1")
            .unwrap();
        assert!(h1.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert!(h1.spans[0]
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));
        let h3 = lines
            .iter()
            .find(|line| line_to_plain(line) == "### H3")
            .unwrap();
        assert!(h3.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert!(h3.spans[0].style.add_modifier.contains(Modifier::ITALIC));
        let h4 = lines
            .iter()
            .find(|line| line_to_plain(line) == "#### H4")
            .unwrap();
        assert!(h4.spans[0].style.add_modifier.contains(Modifier::ITALIC));
        let inline = lines.last().unwrap();
        assert!(
            inline
                .spans
                .iter()
                .any(|span| span.content == "bold"
                    && span.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(
            inline
                .spans
                .iter()
                .any(|span| span.content == "em"
                    && span.style.add_modifier.contains(Modifier::ITALIC))
        );
        assert!(inline.spans.iter().any(|span| span.content == "gone"
            && span.style.add_modifier.contains(Modifier::CROSSED_OUT)));
        assert!(inline
            .spans
            .iter()
            .any(|span| span.content == "code" && span.style.fg == Some(Color::Cyan)));
    }

    #[test]
    fn nested_lists_indent_four_wide_and_wrap_with_continuations() {
        let lines = render_markdown_with_options(
            "- outer item with several words to wrap\n  - inner item that also needs wrapping",
            RenderOptions::plain(Some(20)),
        );
        assert_eq!(
            text(&lines),
            vec![
                "- outer item with".to_string(),
                "  several words to".to_string(),
                "  wrap".to_string(),
                "    - inner item".to_string(),
                "      that also".to_string(),
                "      needs wrapping".to_string(),
            ]
        );
    }

    #[test]
    fn ordered_lists_and_blockquotes_wrap_with_prefixes() {
        let lines = render_markdown_with_options(
            "1. ordered item contains many words for wrapping\n\n> block quote with content that should wrap nicely",
            RenderOptions::plain(Some(22)),
        );
        let rendered = text(&lines);
        assert!(rendered.contains(&"1. ordered item".to_string()));
        assert!(rendered.contains(&"   contains many words".to_string()));
        assert!(rendered.contains(&"> block quote with".to_string()));
        assert!(rendered.contains(&"> content that should".to_string()));
        assert!(rendered.iter().all(|line| line.width() <= 22));
    }

    #[test]
    fn markdown_link_renders_label_destination_and_osc8_ranges() {
        let lines = render_markdown_hyperlink_lines_with_options(
            "Read [docs](https://example.com/docs) today",
            RenderOptions {
                width: Some(80),
                color_enabled: true,
            },
        );
        assert_eq!(
            line_to_plain(&lines[0].line),
            "Read docs (https://example.com/docs) today"
        );
        assert_eq!(lines[0].hyperlinks.len(), 2);
        assert!(lines[0]
            .hyperlinks
            .iter()
            .all(|link| link.destination == "https://example.com/docs"));

        let enabled = crate::vendored::terminal_hyperlinks::line_with_osc8(&lines[0], true);
        let enabled_text = line_to_plain(&enabled);
        assert_eq!(
            crate::vendored::terminal_hyperlinks::strip_osc8(&enabled_text),
            "Read docs (https://example.com/docs) today"
        );
        assert!(enabled_text.contains("\x1b]8;;https://example.com/docs\x1b\\docs\x1b]8;;\x1b\\"));
    }

    #[test]
    fn local_file_links_render_target_with_normalized_suffix() {
        let cwd = std::env::current_dir().unwrap();
        let target = cwd.join("src/render/markdown.rs");
        let markdown = format!("Open [ignored]({}#L12C3-L14C9).", target.display());
        let lines = render_markdown_with_options(&markdown, RenderOptions::plain(Some(120)));
        assert_eq!(
            text(&lines),
            vec!["Open src/render/markdown.rs:12:3-14:9.".to_string()]
        );
    }

    #[test]
    fn code_blocks_highlight_without_label() {
        let highlighted = render_markdown_with_options(
            "```rust\nfn main() {}\n```",
            RenderOptions {
                width: Some(80),
                color_enabled: true,
            },
        );
        let rendered = text(&highlighted);
        assert!(!rendered.iter().any(|line| line.contains("code ·")));
        assert_eq!(rendered, vec!["fn main() {}".to_string()]);
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
    fn renders_aligned_table_snapshot() {
        let source = "| Name | Count | Note |\n| --- | ---: | :---: |\n| α | 2 | small |\n| longer | 12 | wrapped words here |";
        let lines = render_markdown_with_options(source, RenderOptions::plain(Some(32)));
        let rendered = text(&lines).join("\n");
        let expected = " Name      Count       Note\n━━━━━━━━  ━━━━━━━  ━━━━━━━━━━━━━\n α             2       small\n────────  ───────  ─────────────\n longer       12      wrapped\n                    words here";
        assert_eq!(rendered, expected);
        assert!(rendered.lines().all(|line| line.width() <= 32));
    }

    #[test]
    fn narrow_table_falls_back_to_key_value_records() {
        let source = "| Path | Summary | Count |\n| --- | --- | ---: |\n| src/render/markdown_table.rs | this row contains enough narrative words to stop scanning as a grid | 7 |";
        let lines = render_markdown_with_options(source, RenderOptions::plain(Some(24)));
        assert_eq!(
            text(&lines),
            vec![
                " Path".to_string(),
                "  src/render/".to_string(),
                "  markdown_table.rs".to_string(),
                " Summary".to_string(),
                "  this row contains".to_string(),
                "  enough narrative words".to_string(),
                "  to stop scanning as a".to_string(),
                "  grid".to_string(),
                " Count".to_string(),
                "  7".to_string(),
            ]
        );
    }

    #[test]
    fn table_spillover_rows_render_as_prose() {
        let source = "| Name | Value |\n| --- | --- |\n| ok | yes |\ntrailing paragraph that should not become a sparse table row";
        let lines = render_markdown_with_options(source, RenderOptions::plain(Some(30)));
        let rendered = text(&lines);
        assert!(rendered.contains(&" ok      yes".to_string()));
        assert!(rendered.contains(&"trailing paragraph that should".to_string()));
        assert!(rendered.contains(&"not become a sparse table row".to_string()));
    }

    #[test]
    fn table_hyperlinks_survive_wrapping_and_records() {
        let destination = "https://example.com/a/very/long/path";
        let source = format!("| URL | Count |\n| --- | --- |\n| {destination} | 1 |");
        let lines = render_markdown_hyperlink_lines_with_options(
            &source,
            RenderOptions {
                width: Some(18),
                color_enabled: false,
            },
        );
        let destinations = lines
            .iter()
            .flat_map(|line| line.hyperlinks.iter().map(|link| link.destination.as_str()))
            .collect::<Vec<_>>();
        assert!(!destinations.is_empty());
        assert!(destinations.iter().all(|value| *value == destination));
    }

    #[test]
    fn no_color_table_and_code_are_legible() {
        let lines = render_markdown_with_options(
            "| A | B |\n|---|---|\n| one | two |\n\n```rust\nfn main() {}\n```",
            RenderOptions::plain(Some(40)),
        );
        let rendered = text(&lines).join("\n");
        assert!(rendered.contains(" A      B"));
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
        assert!(rendered.contains(&"1 -old".to_string()));
        assert!(rendered.contains(&"1 +new".to_string()));
        let add = lines
            .iter()
            .find(|line| line_to_plain(line) == "1 +new")
            .unwrap();
        assert_eq!(add.spans[1].style.fg, Some(Color::Green));
    }
}
