use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::events_pane::{render_event_lines, render_worker_lines};
use crate::theme::{ThemeRole, TuiTheme};
use crate::ui::menu;

const TWO_COLUMN_MIN_WIDTH: u16 = 60;

pub fn render_events_pane(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let inner = menu::render_menu_surface(area, frame.buffer_mut());
    render_events_pane_content(frame.buffer_mut(), app, inner);
}

fn render_events_pane_content(buf: &mut Buffer, app: &App, area: Rect) {
    if area.is_empty() {
        return;
    }
    let (events_area, workers_area) = if area.width < TWO_COLUMN_MIN_WIDTH {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);
        (rows[0], rows[1])
    } else {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(57),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .split(area);
        render_column_divider(buf, columns[1], app.theme());
        (columns[0], columns[2])
    };
    render_section(
        buf,
        events_area,
        app.theme(),
        "daemon events",
        render_event_lines(app.events_pane().events(), app.theme(), events_area.width),
    );
    render_section(
        buf,
        workers_area,
        app.theme(),
        "workers",
        render_worker_lines(app.events_pane().workers(), app.theme(), workers_area.width),
    );
}

fn render_column_divider(buf: &mut Buffer, area: Rect, theme: &TuiTheme) {
    if area.is_empty() {
        return;
    }
    for y in area.top()..area.bottom() {
        Span::styled("│", theme.style(ThemeRole::Muted)).render(
            Rect {
                x: area.x,
                y,
                width: 1,
                height: 1,
            },
            buf,
        );
    }
}

fn render_section(
    buf: &mut Buffer,
    area: Rect,
    theme: &TuiTheme,
    title: &'static str,
    lines: Vec<Line<'static>>,
) {
    if area.is_empty() {
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Fill(1)])
        .split(area);
    let header_area = chunks[0];
    let body_area = chunks[1];
    Line::from(Span::styled(
        title,
        theme.style(ThemeRole::Muted).add_modifier(Modifier::BOLD),
    ))
    .render(header_area, buf);
    Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: false })
        .render(body_area, buf);
}
