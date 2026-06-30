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

pub fn render_events_pane(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let inner = menu::render_menu_surface(area, frame.buffer_mut());
    render_events_pane_content(frame.buffer_mut(), app, inner);
}

fn render_events_pane_content(buf: &mut Buffer, app: &App, area: Rect) {
    if area.is_empty() {
        return;
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    render_section(
        buf,
        columns[0],
        app.theme(),
        "daemon events",
        render_event_lines(app.events_pane().events(), app.theme()),
    );
    render_section(
        buf,
        columns[1],
        app.theme(),
        "workers",
        render_worker_lines(app.events_pane().workers(), app.theme()),
    );
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
    use crate::client::OpenProjectResponse;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn project() -> OpenProjectResponse {
        OpenProjectResponse {
            project_id: "p1".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: false,
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
    fn events_pane_renders_deboxed_headers() {
        let app = App::new(project());
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();

        terminal
            .draw(|frame| render_events_pane(frame, &app, frame.area()))
            .unwrap();
        let text = text_from_terminal(&terminal);

        assert!(text.contains("daemon events"));
        assert!(text.contains("workers"));
        assert!(!text.contains("┌"));
        assert!(!text.contains("│"));
        assert!(!text.contains("└"));
    }

    #[test]
    fn events_pane_renders_dim_empty_states() {
        let app = App::new(project());
        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();

        terminal
            .draw(|frame| render_events_pane(frame, &app, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = text_from_terminal(&terminal);
        let no_events = text.find("No daemon events yet").expect("empty event text") as u16;
        let no_workers = text.find("No workers").expect("empty worker text") as u16;

        assert!(buffer[(no_events % 60, no_events / 60)]
            .style()
            .add_modifier
            .contains(Modifier::ITALIC));
        assert!(buffer[(no_workers % 60, no_workers / 60)]
            .style()
            .add_modifier
            .contains(Modifier::ITALIC));
    }
}
