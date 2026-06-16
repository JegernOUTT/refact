use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget, Wrap};
use ratatui::Frame;

use crate::key_hint;
use crate::overlay::{PagerMode, PagerOverlay};
use crate::read_only_views::{ViewOverlayRow, ViewOverlaySurface};
use crate::ui::menu::{self, GenericDisplayRow, ScrollState};
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

const MIN_OVERLAY_WIDTH: u16 = 24;
const MIN_OVERLAY_HEIGHT: u16 = 8;
const SURFACE_INSET_V: u16 = 1;
const SURFACE_INSET_H: u16 = 2;

struct OverlayLayout {
    header: Option<Rect>,
    body: Rect,
    separator: Option<Rect>,
    hint: Option<Rect>,
}

pub(super) fn transcript_overlay_body_height(area: Rect) -> usize {
    let popup = transcript_popup_area(area);
    let inner = menu_surface_inner_area(popup);
    content_layout(inner).body.height as usize
}

pub(crate) fn render_transcript_overlay(
    frame: &mut Frame<'_>,
    overlay: &crate::overlay::PagerOverlay,
    area: Rect,
) {
    let popup = transcript_popup_area(area);
    frame.render_widget(Clear, popup);
    // Read-only view surfaces (/mcp, /skills, /memories) render as a menu-style list.
    if let Some(surface) = overlay.surface() {
        let mode = match overlay.mode() {
            PagerMode::Rendered => "rendered",
            PagerMode::Raw => "copy/raw",
        };
        render_surface_overlay(frame, overlay, surface, popup, mode);
        return;
    }
    let inner = menu::render_menu_surface(popup, frame.buffer_mut());
    let layout = content_layout(inner);

    if let Some(header) = layout.header {
        render_header(header, overlay, frame.buffer_mut());
    }
    if layout.body.height > 0 && layout.body.width > 0 {
        let body = overlay
            .visible_lines(layout.body.height as usize)
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), layout.body);
    }
    if let Some(separator) = layout.separator {
        render_separator(separator, frame.buffer_mut());
    }
    if let Some(hint) = layout.hint {
        let line =
            truncate_line_with_ellipsis_if_overflow(hint_bar_line(overlay), hint.width as usize);
        line.render(hint, frame.buffer_mut());
    }
}

fn transcript_popup_area(area: Rect) -> Rect {
    let width = area
        .width
        .saturating_sub(6)
        .max(MIN_OVERLAY_WIDTH)
        .min(area.width);
    let height = area
        .height
        .saturating_sub(4)
        .max(MIN_OVERLAY_HEIGHT)
        .min(area.height);
    super::centered(area, width, height)
}

fn menu_surface_inner_area(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(SURFACE_INSET_H),
        y: area.y.saturating_add(SURFACE_INSET_V),
        width: area.width.saturating_sub(SURFACE_INSET_H.saturating_mul(2)),
        height: area
            .height
            .saturating_sub(SURFACE_INSET_V.saturating_mul(2)),
    }
}

fn content_layout(area: Rect) -> OverlayLayout {
    let footer_height = match area.height {
        0 | 1 => 0,
        2 => 1,
        _ => 2,
    };
    let header_height = u16::from(area.height > footer_height);
    let body_height = area.height.saturating_sub(header_height + footer_height);
    let footer_y = area
        .y
        .saturating_add(area.height.saturating_sub(footer_height));
    OverlayLayout {
        header: (header_height > 0).then_some(Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        }),
        body: Rect {
            x: area.x,
            y: area.y.saturating_add(header_height),
            width: area.width,
            height: body_height,
        },
        separator: (footer_height == 2).then_some(Rect {
            x: area.x,
            y: footer_y,
            width: area.width,
            height: 1,
        }),
        hint: (footer_height > 0).then_some(Rect {
            x: area.x,
            y: footer_y.saturating_add(u16::from(footer_height == 2)),
            width: area.width,
            height: 1,
        }),
    }
}

fn render_header(area: Rect, overlay: &PagerOverlay, buf: &mut Buffer) {
    let mut text = format!("─ {} · {} ", overlay.title(), overlay.mode_label());
    let width = area.width as usize;
    let text_width = text.chars().count();
    if text_width < width {
        text.push_str(&"─".repeat(width - text_width));
    }
    let line =
        truncate_line_with_ellipsis_if_overflow(Line::from(Span::styled(text, dim_style())), width);
    line.render(area, buf);
}

fn render_separator(area: Rect, buf: &mut Buffer) {
    Line::from(Span::styled("─".repeat(area.width as usize), dim_style())).render(area, buf);
}

fn hint_bar_line(overlay: &PagerOverlay) -> Line<'static> {
    let copy_label = match overlay.mode() {
        PagerMode::Rendered => "raw",
        PagerMode::Raw => "rendered",
    };
    Line::from(vec![
        key_hint::label(overlay.mode_label()),
        Span::raw(" "),
        key_hint::label(overlay.search_label()),
        Span::raw("  "),
        key_hint::plain("↑/↓"),
        Span::raw(" "),
        key_hint::label("scroll"),
        Span::raw("  "),
        key_hint::plain("PgUp/PgDn"),
        Span::raw(" "),
        key_hint::label("page"),
        Span::raw("  "),
        key_hint::plain("/"),
        Span::raw(" "),
        key_hint::label("search"),
        Span::raw("  "),
        key_hint::plain("n/N"),
        Span::raw(" "),
        key_hint::label("match"),
        Span::raw("  "),
        key_hint::plain("c"),
        Span::raw(" "),
        key_hint::label(copy_label),
        Span::raw("  "),
        key_hint::plain("y"),
        Span::raw(" "),
        key_hint::label("yank"),
        Span::raw("  "),
        key_hint::plain("Esc/q"),
        Span::raw(" "),
        key_hint::label("close"),
    ])
}

fn dim_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn render_surface_overlay(
    frame: &mut Frame<'_>,
    overlay: &crate::overlay::PagerOverlay,
    surface: &ViewOverlaySurface,
    popup: Rect,
    mode: &str,
) {
    let inner = menu::render_menu_surface(popup, frame.buffer_mut());
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let [header_area, body_area, status_area] = Layout::vertical([
        Constraint::Length(surface_header_height(surface, inner.height)),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    render_surface_header(frame, overlay, surface, header_area, mode);
    render_surface_rows(frame, surface, body_area, overlay.scroll());
    let status = truncate_line_with_ellipsis_if_overflow(
        Line::from(Span::styled(
            overlay.status(),
            Style::default().fg(Color::DarkGray),
        )),
        status_area.width as usize,
    );
    frame.render_widget(Paragraph::new(status), status_area);
}

fn surface_header_height(surface: &ViewOverlaySurface, available_height: u16) -> u16 {
    surface
        .summary_lines
        .len()
        .saturating_add(2)
        .min(available_height.saturating_sub(1) as usize)
        .max(1) as u16
}

fn render_surface_header(
    frame: &mut Frame<'_>,
    overlay: &crate::overlay::PagerOverlay,
    surface: &ViewOverlaySurface,
    area: Rect,
    mode: &str,
) {
    if area.height == 0 {
        return;
    }
    let mut lines = vec![Line::from(vec![
        Span::styled(
            overlay.title().to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" · {mode}"),
            Style::default().add_modifier(Modifier::DIM),
        ),
    ])];
    lines.extend(surface.summary_lines.iter().map(|line| {
        truncate_line_with_ellipsis_if_overflow(
            Line::from(Span::styled(
                line.clone(),
                Style::default().add_modifier(Modifier::DIM),
            )),
            area.width as usize,
        )
    }));
    lines.truncate(area.height as usize);
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_surface_rows(
    frame: &mut Frame<'_>,
    surface: &ViewOverlaySurface,
    area: Rect,
    scroll: usize,
) {
    if area.height == 0 {
        return;
    }
    let rows = surface
        .rows
        .iter()
        .map(surface_row_to_display_row)
        .collect::<Vec<_>>();
    let selected_idx = surface
        .rows
        .iter()
        .enumerate()
        .skip(scroll)
        .find_map(|(idx, row)| (!row.is_disabled).then_some(idx));
    let state = ScrollState {
        selected_idx,
        scroll_top: scroll,
    };
    menu::render_rows(
        area,
        frame.buffer_mut(),
        &rows,
        &state,
        area.height as usize,
        &surface.empty_message,
    );
}

fn surface_row_to_display_row(row: &ViewOverlayRow) -> GenericDisplayRow {
    GenericDisplayRow {
        name: row.name.clone(),
        name_prefix_spans: vec![Span::raw(if row.is_disabled { "" } else { "› " })],
        description: row.description.clone(),
        category_tag: row.category_tag.clone(),
        disabled_reason: row.disabled_reason.clone(),
        is_disabled: row.is_disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::PagerOverlay;
    use crate::read_only_views::{ViewOverlayRow, ViewOverlaySurface};
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn transcript_overlay_renders_deboxed_surface_and_hint_bar() {
        let overlay = PagerOverlay::new(
            "Transcript",
            vec!["alpha".to_string(), "beta".to_string()],
            vec!["raw alpha".to_string()],
        );
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_transcript_overlay(frame, &overlay, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Transcript · Rendered"));
        assert!(text.contains("alpha"));
        assert!(text.contains("Rendered /"));
        assert!(text.contains("↑/↓ scroll"));
        assert!(text.contains("Esc/q close"));
        assert!(!text.contains("┌"));
        assert!(!text.contains("┐"));
        assert!(!text.contains("└"));
        assert!(!text.contains("┘"));
        assert!(!text.contains("│"));
    }

    #[test]
    fn transcript_overlay_hint_bar_tracks_raw_mode() {
        let overlay = PagerOverlay::raw(
            "Transcript raw",
            vec!["rendered".to_string()],
            vec!["raw one".to_string(), "raw two".to_string()],
        );
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_transcript_overlay(frame, &overlay, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Transcript raw · Raw"));
        assert!(text.contains("raw one"));
        assert!(text.contains("Raw /"));
        assert!(text.contains("c rendered"));
    }

    #[test]
    fn body_height_matches_rendered_content_window() {
        let area = Rect::new(0, 0, 80, 20);
        assert_eq!(transcript_overlay_body_height(area), 11);
    }

    #[test]
    fn read_only_views_surface_overlay_is_deboxed_and_accents_rows() {
        let overlay = PagerOverlay::new("Skills", vec!["Skills".to_string()], Vec::new())
            .with_surface(Some(ViewOverlaySurface {
                summary_lines: vec!["Available skills".to_string()],
                rows: vec![
                    ViewOverlayRow {
                        name: "Skills".to_string(),
                        description: None,
                        category_tag: None,
                        disabled_reason: None,
                        is_disabled: true,
                    },
                    ViewOverlayRow {
                        name: "/explain".to_string(),
                        description: Some("Explain code".to_string()),
                        category_tag: Some("project".to_string()),
                        disabled_reason: None,
                        is_disabled: false,
                    },
                ],
                empty_message: "No skills".to_string(),
            }));
        let backend = TestBackend::new(72, 18);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_transcript_overlay(frame, &overlay, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("Skills"));
        assert!(text.contains("/explain"));
        assert!(text.contains("Explain code"));
        assert!(text.contains("rendered"));
        assert!(!text.contains("┌"));
        let cursor = buffer
            .content()
            .iter()
            .find(|cell| cell.symbol() == "›")
            .expect("row cursor rendered");
        assert_eq!(cursor.style().fg, Some(Color::Cyan));
        assert!(cursor.style().add_modifier.contains(Modifier::BOLD));
    }
}
