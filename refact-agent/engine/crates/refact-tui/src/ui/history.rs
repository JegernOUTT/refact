use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

pub(crate) fn render_history_surface(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(history) = app.history_surface() else {
        return;
    };
    frame.render_widget(Clear, area);
    if area.is_empty() {
        return;
    }
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let lines = history.render_lines(area.width as usize);
    let header_line = truncate_line_with_ellipsis_if_overflow(
        ratatui::text::Line::from(
            lines
                .first()
                .cloned()
                .unwrap_or_else(|| "History".to_string()),
        ),
        header.width as usize,
    );
    frame.render_widget(Paragraph::new(header_line), header);
    let footer_line = lines.last().cloned().unwrap_or_default();
    let body_lines = lines
        .iter()
        .skip(1)
        .take(lines.len().saturating_sub(2))
        .map(|line| {
            truncate_line_with_ellipsis_if_overflow(
                ratatui::text::Line::from(line.clone()),
                body.width as usize,
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(body_lines).wrap(Wrap { trim: false }), body);
    let footer_line = truncate_line_with_ellipsis_if_overflow(
        ratatui::text::Line::from(footer_line),
        footer.width as usize,
    );
    frame.render_widget(Paragraph::new(footer_line), footer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::OpenProjectResponse;
    use crate::sessions::TrajectoryMeta;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn app() -> App {
        let mut app = App::new(OpenProjectResponse {
            project_id: "project".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: None,
            worker: None,
            cron_pending: None,
        });
        app.test_open_history_surface(vec![TrajectoryMeta {
            id: "chat-history".to_string(),
            title: "History surface".to_string(),
            updated_at: "2026-08-25T10:00:00Z".to_string(),
            total_cost_usd: None,
            ..Default::default()
        }]);
        app
    }

    #[test]
    fn history_surface_keeps_primary_information_at_40_by_15() {
        let app = app();
        let mut terminal = Terminal::new(TestBackend::new(40, 15)).unwrap();
        terminal
            .draw(|frame| render_history_surface(frame, &app, frame.area()))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("History"));
        assert!(text.contains("surface"));
        assert!(text.contains("Enter"));
    }
}
