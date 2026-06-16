// Adapted from openai/codex codex-rs/tui/src/bottom_pane/selection_popup_common.rs, Apache-2.0.

use std::borrow::Cow;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Widget};

use crate::key_hint;
use crate::render::{Insets, RectExt};
use crate::render::wrapping::{line_width, word_wrap_line, RtOptions};
use crate::style::{accent_style, user_message_style};
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

pub(crate) const MAX_POPUP_ROWS: usize = 8;

const MENU_SURFACE_INSET_V: u16 = 1;
const MENU_SURFACE_INSET_H: u16 = 2;
const FIXED_LEFT_COLUMN_NUMERATOR: usize = 3;
const FIXED_LEFT_COLUMN_DENOMINATOR: usize = 10;

#[derive(Default)]
pub(crate) struct GenericDisplayRow {
    pub(crate) name: String,
    pub(crate) name_style: Option<Style>,
    pub(crate) name_prefix_spans: Vec<Span<'static>>,
    pub(crate) key_label: Option<String>,
    pub(crate) match_indices: Option<Vec<usize>>,
    pub(crate) description: Option<String>,
    pub(crate) category_tag: Option<String>,
    pub(crate) disabled_reason: Option<String>,
    pub(crate) is_disabled: bool,
    pub(crate) wrap_indent: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ColumnWidthMode {
    #[default]
    AutoVisible,
    AutoAllRows,
    Fixed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ColumnWidthConfig {
    pub(crate) mode: ColumnWidthMode,
    pub(crate) name_column_width: Option<usize>,
}

impl ColumnWidthConfig {
    pub(crate) const fn new(mode: ColumnWidthMode, name_column_width: Option<usize>) -> Self {
        Self {
            mode,
            name_column_width,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScrollState {
    pub(crate) selected_idx: Option<usize>,
    pub(crate) scroll_top: usize,
}

impl ScrollState {
    pub(crate) fn new() -> Self {
        Self {
            selected_idx: None,
            scroll_top: 0,
        }
    }

    pub(crate) fn clamp_selection(&mut self, len: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        self.selected_idx = Some(self.selected_idx.unwrap_or(0).min(len - 1));
    }

    pub(crate) fn move_up_wrap(&mut self, len: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        self.selected_idx = Some(match self.selected_idx {
            Some(idx) if idx > 0 => idx - 1,
            Some(_) => len - 1,
            None => 0,
        });
    }

    pub(crate) fn move_down_wrap(&mut self, len: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        self.selected_idx = Some(match self.selected_idx {
            Some(idx) if idx + 1 < len => idx + 1,
            _ => 0,
        });
    }

    pub(crate) fn page_up_clamped(&mut self, len: usize, visible_rows: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        let step = visible_rows.max(1);
        let current = self.selected_idx.unwrap_or(0).min(len - 1);
        self.selected_idx = Some(current.saturating_sub(step));
        self.ensure_visible(len, visible_rows);
    }

    pub(crate) fn page_down_clamped(&mut self, len: usize, visible_rows: usize) {
        if self.clear_if_empty(len) {
            return;
        }
        let step = visible_rows.max(1);
        let current = self.selected_idx.unwrap_or(0).min(len - 1);
        self.selected_idx = Some(current.saturating_add(step).min(len - 1));
        self.ensure_visible(len, visible_rows);
    }

    pub(crate) fn ensure_visible(&mut self, len: usize, visible_rows: usize) {
        if len == 0 || visible_rows == 0 {
            self.scroll_top = 0;
            return;
        }
        if let Some(selected) = self.selected_idx {
            if selected < self.scroll_top {
                self.scroll_top = selected;
            } else {
                let bottom = self.scroll_top + visible_rows - 1;
                if selected > bottom {
                    self.scroll_top = selected + 1 - visible_rows;
                }
            }
        } else {
            self.scroll_top = 0;
        }
    }

    fn clear_if_empty(&mut self, len: usize) -> bool {
        if len != 0 {
            return false;
        }
        self.selected_idx = None;
        self.scroll_top = 0;
        true
    }
}

pub(crate) fn render_menu_surface(area: Rect, buf: &mut Buffer) -> Rect {
    if area.is_empty() {
        return area;
    }
    Block::default()
        .style(user_message_style())
        .render(area, buf);
    area.inset(Insets::vh(MENU_SURFACE_INSET_V, MENU_SURFACE_INSET_H))
}

pub(crate) fn standard_popup_hint_line() -> Line<'static> {
    accept_cancel_hint_line(Some("Enter"), "to confirm", Some("Esc"), "to go back")
}

pub(crate) fn accept_cancel_hint_line(
    accept: Option<&str>,
    accept_label: &'static str,
    cancel: Option<&str>,
    cancel_label: &'static str,
) -> Line<'static> {
    match (accept, cancel) {
        (Some(accept), Some(cancel)) => Line::from(vec![
            "Press ".into(),
            key_hint::plain(accept.to_string()),
            format!(" {accept_label} or ").into(),
            key_hint::plain(cancel.to_string()),
            format!(" {cancel_label}").into(),
        ]),
        (Some(accept), None) => Line::from(vec![
            "Press ".into(),
            key_hint::plain(accept.to_string()),
            format!(" {accept_label}").into(),
        ]),
        (None, Some(cancel)) => Line::from(vec![
            "Press ".into(),
            key_hint::plain(cancel.to_string()),
            format!(" {cancel_label}").into(),
        ]),
        (None, None) => Line::from(""),
    }
}

pub(crate) fn render_rows(
    area: Rect,
    buf: &mut Buffer,
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    empty_message: &str,
) -> u16 {
    render_rows_inner(
        area,
        buf,
        rows_all,
        state,
        max_results,
        empty_message,
        ColumnWidthConfig::default(),
    )
}

pub(crate) fn render_rows_single_line(
    area: Rect,
    buf: &mut Buffer,
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    empty_message: &str,
) -> u16 {
    render_rows_single_line_with_config(
        area,
        buf,
        rows_all,
        state,
        max_results,
        empty_message,
        ColumnWidthConfig::default(),
    )
}

pub(crate) fn render_rows_single_line_with_config(
    area: Rect,
    buf: &mut Buffer,
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    empty_message: &str,
    column_width: ColumnWidthConfig,
) -> u16 {
    if rows_all.is_empty() {
        render_empty_message(area, buf, empty_message);
        return u16::from(area.height > 0);
    }

    let visible_items = max_results
        .min(rows_all.len())
        .min(area.height.max(1) as usize);
    if visible_items == 0 {
        return 0;
    }

    let start_idx = window_start(rows_all.len(), state, visible_items);
    let desc_col = compute_desc_col(rows_all, start_idx, visible_items, area.width, column_width);
    let mut rendered_lines = 0u16;
    let mut y = area.y;

    for (idx, row) in rows_all
        .iter()
        .enumerate()
        .skip(start_idx)
        .take(visible_items)
    {
        if y >= area.y.saturating_add(area.height) {
            break;
        }
        let mut line = build_full_line(row, desc_col);
        apply_row_state_style(
            std::slice::from_mut(&mut line),
            Some(idx) == state.selected_idx && !row.is_disabled,
            row.is_disabled,
        );
        let line = truncate_line_with_ellipsis_if_overflow(line, area.width as usize);
        line.render(
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
            buf,
        );
        y = y.saturating_add(1);
        rendered_lines = rendered_lines.saturating_add(1);
    }

    rendered_lines
}

pub(crate) fn measure_rows_height(
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    width: u16,
) -> u16 {
    measure_rows_height_with_config(
        rows_all,
        state,
        max_results,
        width,
        ColumnWidthConfig::default(),
    )
}

pub(crate) fn measure_rows_height_with_config(
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    width: u16,
    column_width: ColumnWidthConfig,
) -> u16 {
    if rows_all.is_empty() {
        return 1;
    }
    let visible_items = max_results.min(rows_all.len());
    if visible_items == 0 {
        return 0;
    }
    let start_idx = window_start(rows_all.len(), state, visible_items);
    let content_width = width.max(1);
    let desc_col = compute_desc_col(
        rows_all,
        start_idx,
        visible_items,
        content_width,
        column_width,
    );
    rows_all
        .iter()
        .skip(start_idx)
        .take(visible_items)
        .map(|row| wrap_row_lines(row, desc_col, content_width).len() as u16)
        .fold(0u16, u16::saturating_add)
        .max(1)
}

fn render_rows_inner(
    area: Rect,
    buf: &mut Buffer,
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    empty_message: &str,
    column_width: ColumnWidthConfig,
) -> u16 {
    if rows_all.is_empty() {
        render_empty_message(area, buf, empty_message);
        return u16::from(area.height > 0);
    }
    let max_items = max_results.min(rows_all.len());
    if max_items == 0 {
        return 0;
    }
    let start_idx = window_start(rows_all.len(), state, max_items);
    let desc_col = compute_desc_col(rows_all, start_idx, max_items, area.width, column_width);
    let mut rendered_lines = 0u16;
    let mut y = area.y;

    for (idx, row) in rows_all.iter().enumerate().skip(start_idx).take(max_items) {
        if y >= area.y.saturating_add(area.height) {
            break;
        }
        let mut wrapped = wrap_row_lines(row, desc_col, area.width);
        apply_row_state_style(
            &mut wrapped,
            Some(idx) == state.selected_idx && !row.is_disabled,
            row.is_disabled,
        );
        for line in wrapped {
            if y >= area.y.saturating_add(area.height) {
                break;
            }
            line.render(
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
            y = y.saturating_add(1);
            rendered_lines = rendered_lines.saturating_add(1);
        }
    }

    rendered_lines
}

fn render_empty_message(area: Rect, buf: &mut Buffer, empty_message: &str) {
    if area.height == 0 {
        return;
    }
    Line::from(Span::styled(
        empty_message.to_string(),
        Style::default()
            .add_modifier(Modifier::DIM)
            .add_modifier(Modifier::ITALIC),
    ))
    .render(area, buf);
}

fn window_start(len: usize, state: &ScrollState, visible_items: usize) -> usize {
    if len == 0 || visible_items == 0 {
        return 0;
    }
    let mut start_idx = state.scroll_top.min(len.saturating_sub(1));
    if let Some(selected) = state.selected_idx {
        if selected < start_idx {
            start_idx = selected;
        } else {
            let bottom = start_idx.saturating_add(visible_items.saturating_sub(1));
            if selected > bottom {
                start_idx = selected + 1 - visible_items;
            }
        }
    }
    start_idx
}

fn compute_desc_col(
    rows_all: &[GenericDisplayRow],
    start_idx: usize,
    visible_items: usize,
    content_width: u16,
    column_width: ColumnWidthConfig,
) -> usize {
    if content_width <= 1 {
        return 0;
    }
    let max_desc_col = content_width.saturating_sub(1) as usize;
    match column_width.mode {
        ColumnWidthMode::Fixed => ((content_width as usize * FIXED_LEFT_COLUMN_NUMERATOR)
            / FIXED_LEFT_COLUMN_DENOMINATOR)
            .clamp(1, max_desc_col),
        ColumnWidthMode::AutoVisible | ColumnWidthMode::AutoAllRows => {
            let rows = match column_width.mode {
                ColumnWidthMode::AutoVisible => rows_all
                    .iter()
                    .skip(start_idx)
                    .take(visible_items)
                    .collect::<Vec<_>>(),
                ColumnWidthMode::AutoAllRows => rows_all.iter().collect::<Vec<_>>(),
                ColumnWidthMode::Fixed => Vec::new(),
            };
            let measured = rows
                .iter()
                .map(|row| name_line_width(row))
                .max()
                .unwrap_or(0);
            column_width
                .name_column_width
                .map(|width| width.max(measured))
                .unwrap_or(measured)
                .saturating_add(2)
                .min(max_desc_col)
        }
    }
}

fn name_line_width(row: &GenericDisplayRow) -> usize {
    let mut spans = row.name_prefix_spans.clone();
    spans.extend(name_spans(row));
    if let Some(key_label) = &row.key_label {
        spans.push(Span::raw(" ("));
        spans.push(key_hint::plain(key_label.clone()));
        spans.push(Span::raw(")"));
    }
    if row.disabled_reason.is_some() {
        spans.push(Span::styled(
            " (disabled)",
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    line_width(&Line::from(spans))
}

fn build_full_line(row: &GenericDisplayRow, desc_col: usize) -> Line<'static> {
    let mut spans = row.name_prefix_spans.clone();
    spans.extend(name_spans(row));
    if let Some(key_label) = &row.key_label {
        spans.push(Span::raw(" ("));
        spans.push(key_hint::plain(key_label.clone()));
        spans.push(Span::raw(")"));
    }
    if row.disabled_reason.is_some() {
        spans.push(Span::styled(
            " (disabled)",
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    let name_width = line_width(&Line::from(spans.clone()));
    if let Some(description) = combined_description(row) {
        let gap = desc_col.saturating_sub(name_width).max(2);
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(
            description,
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    if let Some(tag) = row.category_tag.as_ref().filter(|tag| !tag.is_empty()) {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            tag.clone(),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    Line::from(spans)
}

fn name_spans(row: &GenericDisplayRow) -> Vec<Span<'static>> {
    let base = row.name_style.unwrap_or_default();
    let Some(match_indices) = row.match_indices.as_ref() else {
        return vec![Span::styled(row.name.clone(), base)];
    };
    row.name
        .chars()
        .enumerate()
        .map(|(idx, ch)| {
            let text = ch.to_string();
            if match_indices.contains(&idx) {
                Span::styled(text, base.add_modifier(Modifier::BOLD))
            } else {
                Span::styled(text, base)
            }
        })
        .collect()
}

fn combined_description(row: &GenericDisplayRow) -> Option<String> {
    match (&row.description, &row.disabled_reason) {
        (Some(description), Some(reason)) => Some(format!("{description} (disabled: {reason})")),
        (Some(description), None) => Some(description.clone()),
        (None, Some(reason)) => Some(format!("disabled: {reason}")),
        (None, None) => None,
    }
}

fn wrap_row_lines(row: &GenericDisplayRow, desc_col: usize, width: u16) -> Vec<Line<'static>> {
    let full_line = build_full_line(row, desc_col);
    let continuation_indent = row
        .wrap_indent
        .unwrap_or_else(|| {
            if row.description.is_some() {
                desc_col
            } else {
                0
            }
        })
        .min(width.saturating_sub(1) as usize);
    let options = RtOptions::new(width.max(1) as usize)
        .initial_indent(Line::from(""))
        .subsequent_indent(Line::from(" ".repeat(continuation_indent)));
    word_wrap_line(&full_line, options)
        .into_iter()
        .map(line_to_owned)
        .collect()
}

fn line_to_owned(line: Line<'_>) -> Line<'static> {
    Line {
        style: line.style,
        alignment: line.alignment,
        spans: line
            .spans
            .into_iter()
            .map(|span| Span {
                style: span.style,
                content: Cow::Owned(span.content.into_owned()),
            })
            .collect(),
    }
}

fn apply_row_state_style(lines: &mut [Line<'static>], selected: bool, is_disabled: bool) {
    if selected {
        for line in lines.iter_mut() {
            for span in &mut line.spans {
                span.style = accent_style();
            }
        }
    }
    if is_disabled {
        for line in lines.iter_mut() {
            for span in &mut line.spans {
                span.style = span.style.dim();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Color;

    #[test]
    fn menu_surface_returns_codex_inset() {
        let area = Rect::new(2, 3, 20, 10);
        let mut buffer = Buffer::empty(area);

        let inner = render_menu_surface(area, &mut buffer);

        assert_eq!(inner, Rect::new(4, 4, 16, 8));
    }

    #[test]
    fn scroll_state_wraps_and_pages() {
        let mut state = ScrollState::new();
        let len = 10;
        let visible = 4;

        state.clamp_selection(len);
        assert_eq!(state.selected_idx, Some(0));
        state.move_up_wrap(len);
        state.ensure_visible(len, visible);
        assert_eq!(state.selected_idx, Some(9));
        assert_eq!(state.scroll_top, 6);
        state.move_down_wrap(len);
        state.ensure_visible(len, visible);
        assert_eq!(state.selected_idx, Some(0));
        assert_eq!(state.scroll_top, 0);
        state.page_down_clamped(len, visible);
        assert_eq!(state.selected_idx, Some(4));
        assert_eq!(state.scroll_top, 1);
        state.page_up_clamped(len, visible);
        assert_eq!(state.selected_idx, Some(0));
        assert_eq!(state.scroll_top, 0);
    }

    #[test]
    fn selected_rows_use_accent_style() {
        let rows = vec![GenericDisplayRow {
            name: "Alpha".to_string(),
            description: Some("fast".to_string()),
            ..Default::default()
        }];
        let state = ScrollState {
            selected_idx: Some(0),
            scroll_top: 0,
        };
        let area = Rect::new(0, 0, 20, 1);
        let mut buffer = Buffer::empty(area);

        render_rows_single_line(area, &mut buffer, &rows, &state, 1, "no rows");

        let style = buffer[(0, 0)].style();
        assert_eq!(style.fg, Some(Color::Cyan));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn descriptions_are_dim_when_not_selected() {
        let rows = vec![GenericDisplayRow {
            name: "Alpha".to_string(),
            description: Some("fast".to_string()),
            ..Default::default()
        }];
        let state = ScrollState {
            selected_idx: None,
            scroll_top: 0,
        };
        let area = Rect::new(0, 0, 20, 1);
        let mut buffer = Buffer::empty(area);

        render_rows_single_line(area, &mut buffer, &rows, &state, 1, "no rows");

        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        let desc_x = text.find("fast").expect("description rendered") as u16;
        assert!(buffer[(desc_x, 0)]
            .style()
            .add_modifier
            .contains(Modifier::DIM));
    }

    #[test]
    fn measure_rows_height_accounts_for_wrapping() {
        let rows = vec![GenericDisplayRow {
            name: "Alpha".to_string(),
            description: Some("one two three four five six".to_string()),
            ..Default::default()
        }];
        let state = ScrollState {
            selected_idx: Some(0),
            scroll_top: 0,
        };

        assert!(measure_rows_height(&rows, &state, 1, 12) > 1);
    }

    #[test]
    fn explicit_column_modes_are_available_for_callers() {
        let rows = vec![GenericDisplayRow {
            name: "Alpha".to_string(),
            description: Some("desc".to_string()),
            ..Default::default()
        }];
        let state = ScrollState {
            selected_idx: Some(0),
            scroll_top: 0,
        };
        let area = Rect::new(0, 0, 30, 1);
        let mut buffer = Buffer::empty(area);

        let rendered = render_rows_single_line_with_config(
            area,
            &mut buffer,
            &rows,
            &state,
            1,
            "no rows",
            ColumnWidthConfig::new(ColumnWidthMode::Fixed, None),
        );
        let stable_rendered = render_rows_single_line_with_config(
            area,
            &mut buffer,
            &rows,
            &state,
            1,
            "no rows",
            ColumnWidthConfig::new(ColumnWidthMode::AutoAllRows, None),
        );

        assert_eq!(rendered, 1);
        assert_eq!(stable_rendered, 1);
    }
}
