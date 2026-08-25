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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::client::{OpenProjectResponse, WorkerInfo};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn project() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "p1".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: Some(false),
            worker: None,
            cron_pending: None,
        }
    }

    fn text_from_terminal(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn events_pane_renders_framed_headers() {
        let app = App::new(project());
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();

        terminal
            .draw(|frame| render_events_pane(frame, &app, frame.area()))
            .unwrap();
        let text = text_from_terminal(&terminal);

        assert!(text.contains("daemon events"));
        assert!(text.contains("workers"));
        assert!(text.contains("┌"));
        assert!(text.contains("│"));
        assert!(text.contains("└"));
    }

    #[test]
    fn events_pane_renders_dim_empty_states() {
        let app = App::new(project());
        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();

        terminal
            .draw(|frame| render_events_pane(frame, &app, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let no_events = find_text_start(buffer, "No daemon events yet").expect("empty event text");
        let no_workers = find_text_start(buffer, "No workers").expect("empty worker text");

        assert!(buffer[no_events]
            .style()
            .add_modifier
            .contains(Modifier::ITALIC));
        assert!(buffer[no_workers]
            .style()
            .add_modifier
            .contains(Modifier::ITALIC));
    }

    #[test]
    fn events_pane_stacks_sections_at_40_columns() {
        let app = App::new(project());
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();

        terminal
            .draw(|frame| render_events_pane(frame, &app, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let events = find_text_start(buffer, "daemon events").expect("event header");
        let workers = find_text_start(buffer, "workers").expect("worker header");

        assert_eq!(events.0, workers.0);
        assert!(workers.1 > events.1);
    }

    #[test]
    fn events_pane_renders_populated_worker_list() {
        let workers = vec![WorkerInfo {
            project_id: "p1".to_string(),
            pid: Some(42),
            http_port: Some(9000),
            lsp_port: Some(9001),
            state: serde_json::json!("ready"),
            last_error: None,
            ..WorkerInfo::default()
        }];
        let lines = render_worker_lines(&workers, &TuiTheme::dark(), 80);

        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0].to_string(),
            "p1 pid=42 http=9000 lsp=9001 state=ready"
        );
    }

    fn find_text_start(buffer: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, u16)> {
        for y in buffer.area.top()..buffer.area.bottom() {
            let row = (buffer.area.left()..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>();
            if let Some(x) = row.find(needle) {
                return Some((buffer.area.x + x as u16, y));
            }
        }
        None
    }
}
