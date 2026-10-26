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
