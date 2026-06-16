use pulldown_cmark::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::line_utils::line_to_static;
use super::wrapping::word_wrap_line;
use crate::vendored::terminal_hyperlinks::{remap_wrapped_line, HyperlinkLine};

const TABLE_COLUMN_GAP: usize = 2;
const TABLE_CELL_PADDING: usize = 1;
const TABLE_HEADER_SEPARATOR_CHAR: char = '━';
const TABLE_BODY_SEPARATOR_CHAR: char = '─';

const FIELD_LEADING_PADDING: usize = 1;
const FIELD_GAP: usize = 2;
const MIN_VALUE_WIDTH: usize = 3;
const MIN_ALIGNED_COMPACT_VALUE_WIDTH: usize = 12;
const MIN_ALIGNED_EXPANSIVE_VALUE_WIDTH: usize = 24;
const MIN_SCANNABLE_NARRATIVE_WIDTH: usize = 12;
const MIN_SCANNABLE_TOKEN_HEAVY_WIDTH: usize = 12;
const CRAMPED_EXPANSIVE_CELL_LINES: usize = 4;
const CATASTROPHIC_NARRATIVE_CELL_LINES: usize = 7;
const STACKED_VALUE_INDENT: usize = 2;

#[derive(Clone, Debug, Default)]
pub(crate) struct TableCell {
    lines: Vec<HyperlinkLine>,
}

impl TableCell {
    fn ensure_line(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(HyperlinkLine::new(Line::default()));
        }
    }

    pub(crate) fn push_span(&mut self, span: Span<'static>) {
        self.ensure_line();
        if let Some(line) = self.lines.last_mut() {
            line.line.spans.push(span);
        }
    }

    pub(crate) fn push_annotated(&mut self, mut appended: HyperlinkLine) {
        self.ensure_line();
        if let Some(line) = self.lines.last_mut() {
            let shift = line.width();
            line.line.spans.append(&mut appended.line.spans);
            line.hyperlinks
                .extend(appended.hyperlinks.into_iter().map(|mut link| {
                    link.columns = link.columns.start + shift..link.columns.end + shift;
                    link
                }));
        }
    }

    pub(crate) fn hard_break(&mut self) {
        self.lines.push(HyperlinkLine::new(Line::default()));
    }

    fn plain_text(&self) -> String {
        self.lines
            .iter()
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug)]
struct TableBodyRow {
    cells: Vec<TableCell>,
    has_table_pipe_syntax: bool,
}

#[derive(Debug)]
pub(crate) struct TableState {
    alignments: Vec<Alignment>,
    header: Option<Vec<TableCell>>,
    rows: Vec<TableBodyRow>,
    current_row: Option<Vec<TableCell>>,
    current_row_has_table_pipe_syntax: bool,
    current_cell: Option<TableCell>,
    in_header: bool,
}

impl TableState {
    pub(crate) fn new(alignments: Vec<Alignment>) -> Self {
        Self {
            alignments,
            header: None,
            rows: Vec::new(),
            current_row: None,
            current_row_has_table_pipe_syntax: false,
            current_cell: None,
            in_header: false,
        }
    }

    pub(crate) fn start_header(&mut self) {
        self.in_header = true;
    }

    pub(crate) fn end_header(&mut self) {
        if self.current_cell.is_some() {
            self.end_cell();
        }
        if let Some(row) = self.current_row.take() {
            if !row.is_empty() {
                self.header = Some(row);
            }
        }
        self.in_header = false;
    }

    pub(crate) fn start_row(&mut self, has_table_pipe_syntax: bool) {
        self.current_row = Some(Vec::new());
        self.current_cell = None;
        self.current_row_has_table_pipe_syntax = has_table_pipe_syntax;
    }

    pub(crate) fn end_row(&mut self) {
        if self.current_cell.is_some() {
            self.end_cell();
        }
        let Some(row) = self.current_row.take() else {
            return;
        };
        if self.in_header {
            self.header = Some(row);
        } else if !row.is_empty() {
            self.rows.push(TableBodyRow {
                cells: row,
                has_table_pipe_syntax: self.current_row_has_table_pipe_syntax,
            });
        }
        self.current_row_has_table_pipe_syntax = false;
    }

    pub(crate) fn start_cell(&mut self) {
        self.current_cell = Some(TableCell::default());
    }

    pub(crate) fn end_cell(&mut self) {
        if let Some(cell) = self.current_cell.take() {
            self.current_row.get_or_insert_with(Vec::new).push(cell);
        }
    }

    pub(crate) fn has_current_cell(&self) -> bool {
        self.current_cell.is_some()
    }

    pub(crate) fn push_span_to_current_cell(&mut self, span: Span<'static>) {
        self.current_cell
            .get_or_insert_with(TableCell::default)
            .push_span(span);
    }

    pub(crate) fn push_annotated_to_current_cell(&mut self, line: HyperlinkLine) {
        self.current_cell
            .get_or_insert_with(TableCell::default)
            .push_annotated(line);
    }

    pub(crate) fn hard_break_current_cell(&mut self) {
        self.current_cell
            .get_or_insert_with(TableCell::default)
            .hard_break();
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TableRenderStyles {
    pub(crate) header: Style,
    pub(crate) separator: Style,
}

pub(crate) struct RenderedTable {
    pub(crate) table_lines: Vec<HyperlinkLine>,
    pub(crate) table_lines_prewrapped: bool,
    pub(crate) spillover_lines: Vec<HyperlinkLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TableColumnKind {
    Narrative,
    TokenHeavy,
    Compact,
}

#[derive(Clone, Debug)]
struct TableColumnMetrics {
    max_width: usize,
    header_token_width: usize,
    body_token_width: usize,
    kind: TableColumnKind,
}

pub(crate) fn render_table(
    mut table: TableState,
    available_width: Option<usize>,
    styles: TableRenderStyles,
) -> RenderedTable {
    let column_count = table.alignments.len();
    if column_count == 0 {
        return RenderedTable {
            table_lines: Vec::new(),
            table_lines_prewrapped: true,
            spillover_lines: Vec::new(),
        };
    }

    let mut spillover_rows = Vec::new();
    let mut rows = Vec::with_capacity(table.rows.len());
    for (idx, row) in table.rows.iter().enumerate() {
        let next_row = table.rows.get(idx + 1);
        if column_count > 1 && is_spillover_row(row, next_row) {
            if let Some(cell) = row.cells.first().cloned() {
                spillover_rows.push(cell);
            }
        } else {
            rows.push(row.cells.clone());
        }
    }

    let mut header = table
        .header
        .take()
        .unwrap_or_else(|| vec![TableCell::default(); column_count]);
    normalize_row(&mut header, column_count);
    for row in &mut rows {
        normalize_row(row, column_count);
    }

    let metrics = collect_table_column_metrics(&header, &rows, column_count);
    let column_widths = compute_column_widths(
        &metrics,
        column_count,
        available_table_width(column_count, available_width),
    );
    let spillover_lines = spillover_rows
        .into_iter()
        .flat_map(|cell| cell.lines)
        .collect::<Vec<_>>();

    let Some(column_widths) = column_widths else {
        if rows.is_empty() {
            return RenderedTable {
                table_lines: render_table_pipe_fallback(&header, &rows, &table.alignments),
                table_lines_prewrapped: false,
                spillover_lines,
            };
        }
        return RenderedTable {
            table_lines: render_records(
                &header,
                &rows,
                &metrics,
                available_width,
                styles.header,
                styles.separator,
            ),
            table_lines_prewrapped: true,
            spillover_lines,
        };
    };

    if should_render_records(&rows, &column_widths, &metrics) {
        return RenderedTable {
            table_lines: render_records(
                &header,
                &rows,
                &metrics,
                available_width,
                styles.header,
                styles.separator,
            ),
            table_lines_prewrapped: true,
            spillover_lines,
        };
    }

    let mut out = Vec::with_capacity(2 + rows.len() * 2);
    out.extend(render_table_row(
        &header,
        &column_widths,
        &table.alignments,
        styles.header,
    ));
    out.push(render_table_separator(
        &column_widths,
        TABLE_HEADER_SEPARATOR_CHAR,
        styles.separator,
    ));
    for (idx, row) in rows.iter().enumerate() {
        out.extend(render_table_row(
            row,
            &column_widths,
            &table.alignments,
            Style::default(),
        ));
        if idx + 1 < rows.len() {
            out.push(render_table_separator(
                &column_widths,
                TABLE_BODY_SEPARATOR_CHAR,
                styles.separator,
            ));
        }
    }

    RenderedTable {
        table_lines: out,
        table_lines_prewrapped: true,
        spillover_lines,
    }
}

fn available_table_width(column_count: usize, available_width: Option<usize>) -> Option<usize> {
    available_width.map(|width| {
        let reserved = column_count.saturating_sub(1) * TABLE_COLUMN_GAP
            + column_count * TABLE_CELL_PADDING * 2;
        width.saturating_sub(reserved)
    })
}

fn normalize_row(row: &mut Vec<TableCell>, column_count: usize) {
    row.truncate(column_count);
    row.resize_with(column_count, TableCell::default);
}

fn compute_column_widths(
    metrics: &[TableColumnMetrics],
    column_count: usize,
    available_width: Option<usize>,
) -> Option<Vec<usize>> {
    let min_column_width = 3usize;
    let mut widths = metrics
        .iter()
        .map(|col| col.max_width.max(min_column_width))
        .collect::<Vec<_>>();

    let Some(max_width) = available_width else {
        return Some(widths);
    };
    let minimum_total = column_count * min_column_width;
    if max_width < minimum_total {
        return None;
    }

    let mut floors = metrics
        .iter()
        .map(|col| preferred_column_floor(col, min_column_width))
        .collect::<Vec<_>>();
    let mut floor_total = floors.iter().sum::<usize>();
    while floor_total > max_width {
        let Some((idx, _)) = floors
            .iter()
            .enumerate()
            .filter(|(_, floor)| **floor > min_column_width)
            .min_by_key(|(idx, floor)| {
                (
                    column_shrink_priority(metrics[*idx].kind),
                    usize::MAX.saturating_sub(**floor),
                )
            })
        else {
            break;
        };
        floors[idx] -= 1;
        floor_total -= 1;
    }

    let mut total_width = widths.iter().sum::<usize>();
    while total_width > max_width {
        let Some(idx) = next_column_to_shrink(&widths, &floors, metrics) else {
            break;
        };
        widths[idx] -= 1;
        total_width -= 1;
    }

    (total_width <= max_width).then_some(widths)
}

fn collect_table_column_metrics(
    header: &[TableCell],
    rows: &[Vec<TableCell>],
    column_count: usize,
) -> Vec<TableColumnMetrics> {
    let mut metrics = Vec::with_capacity(column_count);
    for column in 0..column_count {
        let header_cell = &header[column];
        let header_plain = header_cell.plain_text();
        let header_token_width = longest_token_width(&header_plain);
        let mut max_width = cell_display_width(header_cell);
        let mut body_token_width = 0usize;
        let mut body_token_count = 0usize;
        let mut long_body_token_count = 0usize;
        let mut total_words = 0usize;
        let mut total_cells = 0usize;
        let mut total_cell_width = 0usize;

        for row in rows {
            let cell = &row[column];
            max_width = max_width.max(cell_display_width(cell));
            let plain = cell.plain_text();
            body_token_width = body_token_width.max(longest_token_width(&plain));
            let word_count = plain.split_whitespace().count();
            if word_count > 0 {
                body_token_count += word_count;
                long_body_token_count += plain
                    .split_whitespace()
                    .filter(|token| token.width() >= 20)
                    .count();
                total_words += word_count;
                total_cells += 1;
                total_cell_width += plain.width();
            }
        }

        let avg_words_per_cell = if total_cells == 0 {
            header_plain.split_whitespace().count() as f64
        } else {
            total_words as f64 / total_cells as f64
        };
        let avg_cell_width = if total_cells == 0 {
            header_plain.width() as f64
        } else {
            total_cell_width as f64 / total_cells as f64
        };
        let kind = if long_body_token_count > 0
            && long_body_token_count >= body_token_count.saturating_sub(long_body_token_count)
        {
            TableColumnKind::TokenHeavy
        } else if avg_words_per_cell >= 4.0 || avg_cell_width >= 28.0 {
            TableColumnKind::Narrative
        } else {
            TableColumnKind::Compact
        };

        metrics.push(TableColumnMetrics {
            max_width,
            header_token_width,
            body_token_width,
            kind,
        });
    }

    metrics
}

fn preferred_column_floor(metrics: &TableColumnMetrics, min_column_width: usize) -> usize {
    let token_target = match metrics.kind {
        TableColumnKind::Narrative | TableColumnKind::TokenHeavy => 16,
        TableColumnKind::Compact => metrics
            .header_token_width
            .max(metrics.body_token_width.min(16)),
    };
    token_target.max(min_column_width).min(metrics.max_width)
}

fn next_column_to_shrink(
    widths: &[usize],
    floors: &[usize],
    metrics: &[TableColumnMetrics],
) -> Option<usize> {
    widths
        .iter()
        .enumerate()
        .filter(|(idx, width)| **width > floors[*idx])
        .min_by_key(|(idx, width)| {
            let slack = width.saturating_sub(floors[*idx]);
            (
                column_shrink_priority(metrics[*idx].kind),
                usize::MAX.saturating_sub(slack),
            )
        })
        .map(|(idx, _)| idx)
}

fn column_shrink_priority(kind: TableColumnKind) -> usize {
    match kind {
        TableColumnKind::TokenHeavy => 0,
        TableColumnKind::Narrative => 1,
        TableColumnKind::Compact => 2,
    }
}

fn should_render_records(
    rows: &[Vec<TableCell>],
    column_widths: &[usize],
    metrics: &[TableColumnMetrics],
) -> bool {
    if rows.is_empty() {
        return false;
    }

    let affected_rows = rows
        .iter()
        .filter(|row| {
            let contains_fragmented_value =
                row.iter()
                    .zip(column_widths)
                    .zip(metrics)
                    .any(|((cell, width), metrics)| {
                        let has_fragmented_token = cell
                            .plain_text()
                            .split_whitespace()
                            .any(|token| token.width() > *width);
                        match metrics.kind {
                            TableColumnKind::Compact => has_fragmented_token,
                            TableColumnKind::TokenHeavy => {
                                *width < MIN_SCANNABLE_TOKEN_HEAVY_WIDTH && has_fragmented_token
                            }
                            TableColumnKind::Narrative => false,
                        }
                    });
            contains_fragmented_value || expansive_cells_are_starved(row, column_widths, metrics)
        })
        .count();
    let threshold = if rows.len() == 1 {
        1
    } else {
        2.max(rows.len().div_ceil(3))
    };

    affected_rows >= threshold
}

fn expansive_cells_are_starved(
    row: &[TableCell],
    column_widths: &[usize],
    metrics: &[TableColumnMetrics],
) -> bool {
    let expansive_cells = row
        .iter()
        .zip(column_widths)
        .zip(metrics)
        .filter(|((_cell, _width), metrics)| metrics.kind != TableColumnKind::Compact)
        .map(|((cell, width), metrics)| (metrics.kind, *width, wrap_cell(cell, *width).len()))
        .collect::<Vec<_>>();

    expansive_cells
        .iter()
        .filter(|(_, _, height)| *height >= CRAMPED_EXPANSIVE_CELL_LINES)
        .count()
        >= 2
        || expansive_cells.iter().any(|(kind, width, height)| {
            *kind == TableColumnKind::Narrative
                && *width < MIN_SCANNABLE_NARRATIVE_WIDTH
                && *height >= CATASTROPHIC_NARRATIVE_CELL_LINES
        })
}

fn render_records(
    headers: &[TableCell],
    rows: &[Vec<TableCell>],
    metrics: &[TableColumnMetrics],
    available_width: Option<usize>,
    label_style: Style,
    separator_style: Style,
) -> Vec<HyperlinkLine> {
    let label_width = headers
        .iter()
        .map(|header| header.plain_text().width())
        .max()
        .unwrap_or(0);
    let minimum_value_width = if metrics
        .iter()
        .any(|metrics| metrics.kind != TableColumnKind::Compact)
    {
        MIN_ALIGNED_EXPANSIVE_VALUE_WIDTH
    } else {
        MIN_ALIGNED_COMPACT_VALUE_WIDTH
    };
    let aligned_fields = match available_width {
        Some(width) => {
            FIELD_LEADING_PADDING + label_width + FIELD_GAP + minimum_value_width <= width
        }
        None => true,
    };
    let mut out = Vec::new();

    for (row_index, row) in rows.iter().enumerate() {
        for (header, value) in headers.iter().zip(row) {
            if aligned_fields {
                render_aligned_field(
                    &mut out,
                    header,
                    value,
                    label_width,
                    available_width,
                    label_style,
                );
            } else {
                render_stacked_field(&mut out, header, value, available_width, label_style);
            }
        }
        if row_index + 1 < rows.len() {
            let width = available_width.unwrap_or_else(|| widest_line_width(&out));
            out.push(HyperlinkLine::new(Line::from(Span::styled(
                TABLE_BODY_SEPARATOR_CHAR.to_string().repeat(width),
                separator_style,
            ))));
        }
    }

    out
}

fn render_aligned_field(
    out: &mut Vec<HyperlinkLine>,
    header: &TableCell,
    value: &TableCell,
    label_width: usize,
    available_width: Option<usize>,
    label_style: Style,
) {
    let value_indent = FIELD_LEADING_PADDING + label_width + FIELD_GAP;
    let value_width = available_width
        .map(|width| width.saturating_sub(value_indent).max(MIN_VALUE_WIDTH))
        .unwrap_or_else(|| cell_display_width(value).max(MIN_VALUE_WIDTH));
    let wrapped_value = wrap_cell(value, value_width);
    for (line_index, value_line) in wrapped_value.into_iter().enumerate() {
        let mut spans = Vec::new();
        if line_index == 0 {
            let label = header.plain_text();
            spans.push(Span::raw(" ".repeat(FIELD_LEADING_PADDING)));
            spans.push(Span::styled(label.clone(), label_style));
            spans.push(Span::raw(
                " ".repeat(label_width.saturating_sub(label.width()) + FIELD_GAP),
            ));
        } else {
            spans.push(Span::raw(" ".repeat(value_indent)));
        }
        push_prefixed_value_line(out, spans, value_line);
    }
}

fn render_stacked_field(
    out: &mut Vec<HyperlinkLine>,
    header: &TableCell,
    value: &TableCell,
    available_width: Option<usize>,
    label_style: Style,
) {
    let label_width = available_width
        .map(|width| width.saturating_sub(FIELD_LEADING_PADDING).max(1))
        .unwrap_or_else(|| header.plain_text().width().max(1));
    let label = Line::from(Span::styled(header.plain_text(), label_style));
    for label_line in word_wrap_line(&label, super::wrapping::RtOptions::new(label_width)) {
        let mut spans = vec![Span::raw(" ".repeat(FIELD_LEADING_PADDING))];
        spans.extend(line_to_static(&label_line).spans);
        out.push(HyperlinkLine::new(Line::from(spans)));
    }

    let value_width = available_width
        .map(|width| width.saturating_sub(STACKED_VALUE_INDENT).max(1))
        .unwrap_or_else(|| cell_display_width(value).max(1));
    for value_line in wrap_cell(value, value_width) {
        push_prefixed_value_line(
            out,
            vec![Span::raw(" ".repeat(STACKED_VALUE_INDENT))],
            value_line,
        );
    }
}

fn push_prefixed_value_line(
    out: &mut Vec<HyperlinkLine>,
    mut prefix: Vec<Span<'static>>,
    mut value_line: HyperlinkLine,
) {
    let shift = prefix
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum::<usize>();
    prefix.append(&mut value_line.line.spans);
    let mut output_line = HyperlinkLine::new(Line::from(prefix));
    output_line
        .hyperlinks
        .extend(value_line.hyperlinks.into_iter().map(|mut link| {
            link.columns = link.columns.start + shift..link.columns.end + shift;
            link
        }));
    out.push(output_line);
}

fn render_table_separator(
    column_widths: &[usize],
    separator_char: char,
    style: Style,
) -> HyperlinkLine {
    let segment_char = separator_char.to_string();
    let gap = " ".repeat(TABLE_COLUMN_GAP);
    let text = column_widths
        .iter()
        .map(|width| segment_char.repeat(*width + TABLE_CELL_PADDING * 2))
        .collect::<Vec<_>>()
        .join(&gap);
    HyperlinkLine::new(Line::from(Span::styled(text, style)))
}

fn render_table_row(
    row: &[TableCell],
    column_widths: &[usize],
    alignments: &[Alignment],
    row_style: Style,
) -> Vec<HyperlinkLine> {
    let wrapped_cells = row
        .iter()
        .zip(column_widths)
        .map(|(cell, width)| wrap_cell(cell, *width))
        .collect::<Vec<_>>();
    let row_height = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1);

    let mut out = Vec::with_capacity(row_height);
    for row_line in 0..row_height {
        let Some(last_visible_column) = wrapped_cells.iter().rposition(|lines| {
            lines
                .get(row_line)
                .is_some_and(|line| line_display_width(&line.line) > 0)
        }) else {
            out.push(HyperlinkLine::new(Line::default().style(row_style)));
            continue;
        };
        let mut spans = Vec::new();
        for (column, width) in column_widths
            .iter()
            .enumerate()
            .take(last_visible_column + 1)
        {
            spans.push(Span::raw(" ".repeat(TABLE_CELL_PADDING)));
            let mut line = wrapped_cells[column]
                .get(row_line)
                .cloned()
                .unwrap_or_default();
            let line_width = line_display_width(&line.line);
            let remaining = width.saturating_sub(line_width);
            let (left_padding, right_padding) = match alignments[column] {
                Alignment::Left | Alignment::None => (0, remaining),
                Alignment::Center => (remaining / 2, remaining - remaining / 2),
                Alignment::Right => (remaining, 0),
            };
            if left_padding > 0 {
                spans.push(Span::raw(" ".repeat(left_padding)));
            }
            spans.append(&mut line.line.spans);
            let is_last_column = column == last_visible_column;
            if right_padding > 0 && !is_last_column {
                spans.push(Span::raw(" ".repeat(right_padding)));
            }
            if !is_last_column {
                spans.push(Span::raw(" ".repeat(TABLE_CELL_PADDING)));
                spans.push(Span::raw(" ".repeat(TABLE_COLUMN_GAP)));
            }
        }
        let mut out_line = HyperlinkLine::new(Line::from(spans).style(row_style));
        let mut column_start = 0usize;
        for (column, width) in column_widths
            .iter()
            .enumerate()
            .take(last_visible_column + 1)
        {
            column_start += TABLE_CELL_PADDING;
            if let Some(line) = wrapped_cells[column].get(row_line) {
                let remaining = width.saturating_sub(line_display_width(&line.line));
                let left_padding = match alignments[column] {
                    Alignment::Left | Alignment::None => 0,
                    Alignment::Center => remaining / 2,
                    Alignment::Right => remaining,
                };
                out_line
                    .hyperlinks
                    .extend(line.hyperlinks.iter().cloned().map(|mut link| {
                        link.columns = link.columns.start + column_start + left_padding
                            ..link.columns.end + column_start + left_padding;
                        link
                    }));
            }
            column_start += *width + TABLE_CELL_PADDING + TABLE_COLUMN_GAP;
        }
        out.push(out_line);
    }
    out
}

fn render_table_pipe_fallback(
    header: &[TableCell],
    rows: &[Vec<TableCell>],
    alignments: &[Alignment],
) -> Vec<HyperlinkLine> {
    let mut out = Vec::new();
    out.push(row_to_pipe_line(header));
    out.push(HyperlinkLine::new(Line::from(
        alignments_to_pipe_delimiter(alignments),
    )));
    out.extend(rows.iter().map(|row| row_to_pipe_line(row)));
    out
}

fn row_to_pipe_line(row: &[TableCell]) -> HyperlinkLine {
    let mut out = HyperlinkLine::new(Line::default());
    out.push_span(Span::raw("|"), None);
    for cell in row {
        out.push_span(Span::raw(" "), None);
        for (index, line) in cell.lines.iter().enumerate() {
            if index > 0 {
                out.push_span(Span::raw(" "), None);
            }
            let text = line_to_plain(&line.line);
            let mut column = 0usize;
            let mut current_destination = None;
            let mut current_text = String::new();
            for ch in text.chars() {
                let destination = line
                    .hyperlinks
                    .iter()
                    .find(|link| link.columns.contains(&column))
                    .map(|link| link.destination.as_str());
                if destination != current_destination {
                    flush_pipe_text(&mut out, &mut current_text, current_destination);
                    current_destination = destination;
                }
                if ch == '|' {
                    current_text.push_str("\\|");
                } else {
                    current_text.push(ch);
                }
                column += UnicodeWidthChar::width(ch).unwrap_or(0);
            }
            flush_pipe_text(&mut out, &mut current_text, current_destination);
        }
        out.push_span(Span::raw(" |"), None);
    }
    out
}

fn flush_pipe_text(out: &mut HyperlinkLine, current_text: &mut String, destination: Option<&str>) {
    if !current_text.is_empty() {
        out.push_span(Span::raw(std::mem::take(current_text)), destination);
    }
}

fn alignments_to_pipe_delimiter(alignments: &[Alignment]) -> String {
    let mut out = String::from("|");
    for alignment in alignments {
        let segment = match alignment {
            Alignment::Left => ":---",
            Alignment::Center => ":---:",
            Alignment::Right => "---:",
            Alignment::None => "---",
        };
        out.push_str(segment);
        out.push('|');
    }
    out
}

fn wrap_cell(cell: &TableCell, width: usize) -> Vec<HyperlinkLine> {
    if cell.lines.is_empty() {
        return vec![HyperlinkLine::new(Line::default())];
    }

    let mut wrapped = Vec::new();
    for source_line in &cell.lines {
        let rendered = word_wrap_line(
            &source_line.line,
            super::wrapping::RtOptions::new(width.max(1)),
        )
        .into_iter()
        .map(|line| line_to_static(&line))
        .collect::<Vec<_>>();
        if rendered.is_empty() {
            wrapped.push(HyperlinkLine::new(Line::default()));
        } else {
            wrapped.extend(remap_wrapped_line(source_line, rendered));
        }
    }
    if wrapped.is_empty() {
        wrapped.push(HyperlinkLine::new(Line::default()));
    }
    wrapped
}

fn is_spillover_row(row: &TableBodyRow, next_row: Option<&TableBodyRow>) -> bool {
    let Some(first_text) = first_non_empty_only_text(&row.cells) else {
        return false;
    };

    if row.cells.len() == 1 && !row.has_table_pipe_syntax {
        return true;
    }

    if looks_like_html_content(&first_text) {
        return true;
    }

    if first_text.trim_end().ends_with(':') {
        if next_row
            .and_then(|row| first_non_empty_only_text(&row.cells))
            .is_some_and(|text| looks_like_html_content(&text))
        {
            return true;
        }

        if next_row.is_none() && looks_like_html_label_line(&first_text) {
            return true;
        }
    }

    false
}

fn first_non_empty_only_text(row: &[TableCell]) -> Option<String> {
    let first = row.first()?.plain_text();
    if first.trim().is_empty() {
        return None;
    }
    row[1..]
        .iter()
        .all(|cell| cell.plain_text().trim().is_empty())
        .then_some(first)
}

fn looks_like_html_content(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (idx, &byte) in bytes.iter().enumerate() {
        if byte != b'<' {
            continue;
        }
        let mut tag_start = idx + 1;
        if tag_start < bytes.len() && (bytes[tag_start] == b'/' || bytes[tag_start] == b'!') {
            tag_start += 1;
        }
        if bytes.get(tag_start).is_some_and(u8::is_ascii_alphabetic)
            && bytes
                .get(tag_start + 1..)
                .is_some_and(|suffix| suffix.contains(&b'>'))
        {
            return true;
        }
    }
    false
}

fn looks_like_html_label_line(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.ends_with(':') {
        return false;
    }
    trimmed
        .trim_end_matches(':')
        .trim()
        .split_whitespace()
        .any(|word| word.eq_ignore_ascii_case("html"))
}

fn line_to_plain(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn line_display_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.as_ref().width())
        .sum()
}

fn cell_display_width(cell: &TableCell) -> usize {
    cell.lines
        .iter()
        .map(|line| line_display_width(&line.line))
        .max()
        .unwrap_or(0)
}

fn longest_token_width(text: &str) -> usize {
    text.split_whitespace().map(str::width).max().unwrap_or(0)
}

fn widest_line_width(lines: &[HyperlinkLine]) -> usize {
    lines
        .iter()
        .map(|line| line_display_width(&line.line))
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    use ratatui::style::Modifier;

    fn make_cell(text: &str) -> TableCell {
        let mut cell = TableCell::default();
        cell.push_span(Span::raw(text.to_string()));
        cell
    }

    fn make_body_row(cells: Vec<TableCell>, has_table_pipe_syntax: bool) -> TableBodyRow {
        TableBodyRow {
            cells,
            has_table_pipe_syntax,
        }
    }

    fn render_source_table(source: &str) -> RenderedTable {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        let mut table = None;
        for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
            match event {
                Event::Start(Tag::Table(alignments)) => table = Some(TableState::new(alignments)),
                Event::Start(Tag::TableHead) => {
                    if let Some(table) = &mut table {
                        table.start_header();
                    }
                }
                Event::Start(Tag::TableRow) => {
                    let has_table_pipe_syntax = source
                        .get(range)
                        .map(|source| {
                            let source = source.trim();
                            source.starts_with('|') || source.ends_with('|')
                        })
                        .unwrap_or(false);
                    if let Some(table) = &mut table {
                        table.start_row(has_table_pipe_syntax);
                    }
                }
                Event::Start(Tag::TableCell) => {
                    if let Some(table) = &mut table {
                        table.start_cell();
                    }
                }
                Event::Text(text) => {
                    if let Some(table) = &mut table {
                        table.push_span_to_current_cell(Span::raw(text.to_string()));
                    }
                }
                Event::End(TagEnd::TableCell) => {
                    if let Some(table) = &mut table {
                        table.end_cell();
                    }
                }
                Event::End(TagEnd::TableRow) => {
                    if let Some(table) = &mut table {
                        table.end_row();
                    }
                }
                Event::End(TagEnd::TableHead) => {
                    if let Some(table) = &mut table {
                        table.end_header();
                    }
                }
                Event::End(TagEnd::Table) => break,
                _ => {}
            }
        }
        render_table(
            table.unwrap(),
            Some(24),
            TableRenderStyles {
                header: Style::default().add_modifier(Modifier::BOLD),
                separator: Style::default(),
            },
        )
    }

    #[test]
    fn column_classification_matches_table_shapes() {
        let header = vec![make_cell("ID"), make_cell("Description")];
        let rows = vec![
            vec![make_cell("1"), make_cell("a long description of the item")],
            vec![make_cell("2"), make_cell("another verbose body cell here")],
        ];
        let metrics = collect_table_column_metrics(&header, &rows, 2);
        assert_eq!(metrics[0].kind, TableColumnKind::Compact);
        assert_eq!(metrics[1].kind, TableColumnKind::Narrative);

        let header = vec![make_cell("URL")];
        let rows = vec![
            vec![make_cell("https://example.com/very/long/path")],
            vec![make_cell("https://another.example.org/deep")],
        ];
        let metrics = collect_table_column_metrics(&header, &rows, 1);
        assert_eq!(metrics[0].kind, TableColumnKind::TokenHeavy);
    }

    #[test]
    fn shrink_priority_prefers_token_heavy_then_narrative() {
        let widths = [20usize, 20, 20];
        let floors = [8usize, 8, 8];
        let metrics = [
            TableColumnMetrics {
                max_width: 30,
                header_token_width: 8,
                body_token_width: 6,
                kind: TableColumnKind::Narrative,
            },
            TableColumnMetrics {
                max_width: 30,
                header_token_width: 8,
                body_token_width: 28,
                kind: TableColumnKind::TokenHeavy,
            },
            TableColumnMetrics {
                max_width: 30,
                header_token_width: 8,
                body_token_width: 6,
                kind: TableColumnKind::Compact,
            },
        ];
        assert_eq!(next_column_to_shrink(&widths, &floors, &metrics), Some(1));
        let widths = [20usize, 8, 20];
        assert_eq!(next_column_to_shrink(&widths, &floors, &metrics), Some(0));
    }

    #[test]
    fn spillover_detects_parser_artifacts() {
        let row = make_body_row(vec![make_cell("some trailing text")], false);
        assert!(is_spillover_row(&row, None));
        let row = make_body_row(vec![make_cell("some sparse value"), make_cell("")], false);
        assert!(!is_spillover_row(&row, None));
        let row = make_body_row(vec![make_cell("some sparse value")], true);
        assert!(!is_spillover_row(&row, None));
        let row = make_body_row(
            vec![make_cell("HTML block:"), make_cell(""), make_cell("")],
            false,
        );
        let next = make_body_row(
            vec![make_cell("<div>x</div>"), make_cell(""), make_cell("")],
            false,
        );
        assert!(is_spillover_row(&row, Some(&next)));
    }

    #[test]
    fn renders_no_outer_pipe_multicell_rows_in_grid() {
        let rendered = render_source_table("a | b\n--- | ---\nc | d\ne | f");
        let text = rendered
            .table_lines
            .iter()
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>();
        assert!(rendered.spillover_lines.is_empty());
        assert_eq!(
            text,
            vec![
                " a      b",
                "━━━━━  ━━━━━",
                " c      d",
                "─────  ─────",
                " e      f",
            ]
        );
    }

    #[test]
    fn renders_grid_and_records() {
        let mut table = TableState::new(vec![Alignment::None, Alignment::Right]);
        table.start_header();
        table.start_row(true);
        table.start_cell();
        table.push_span_to_current_cell(Span::raw("Name"));
        table.end_cell();
        table.start_cell();
        table.push_span_to_current_cell(Span::raw("Count"));
        table.end_cell();
        table.end_row();
        table.end_header();
        table.start_row(true);
        table.start_cell();
        table.push_span_to_current_cell(Span::raw("frogs"));
        table.end_cell();
        table.start_cell();
        table.push_span_to_current_cell(Span::raw("12"));
        table.end_cell();
        table.end_row();
        let rendered = render_table(
            table,
            Some(32),
            TableRenderStyles {
                header: Style::default().add_modifier(Modifier::BOLD),
                separator: Style::default(),
            },
        );
        let text = rendered
            .table_lines
            .iter()
            .map(|line| line_to_plain(&line.line))
            .collect::<Vec<_>>();
        assert_eq!(
            text,
            vec![" Name     Count", "━━━━━━━  ━━━━━━━", " frogs       12"]
        );
    }
}
