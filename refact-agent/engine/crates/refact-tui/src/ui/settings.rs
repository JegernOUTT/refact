use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::key_hint;
use crate::ui::menu::{self, GenericDisplayRow, ScrollState};

pub(super) fn render_settings(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let popup = popup_area(area);
    frame.render_widget(Clear, popup);
    let inner = menu::render_menu_surface(popup, frame.buffer_mut());
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let rows = app.settings_rows();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Settings · this chat only"),
            Line::from("Unavailable controls remain listed with their reason."),
        ]),
        header,
    );
    let display = rows
        .iter()
        .map(|row| GenericDisplayRow {
            name: format!("{}: {}", row.title, row.value),
            name_prefix_spans: vec![Span::raw(if row.unavailable_reason.is_some() {
                "  "
            } else {
                "› "
            })],
            description: row
                .unavailable_reason
                .clone()
                .or_else(|| Some(row.section.to_string())),
            category_tag: Some(row.section.to_string()),
            disabled_reason: row.unavailable_reason.clone(),
            is_disabled: row.unavailable_reason.is_some(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    menu::render_rows(
        body,
        frame.buffer_mut(),
        &display,
        &ScrollState {
            selected_idx: Some(app.settings_selected()),
            scroll_top: 0,
        },
        body.height as usize,
        "No settings available",
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            key_hint::plain("Enter"),
            Span::raw(" edit/toggle  "),
            key_hint::plain("Esc"),
            Span::raw(" close"),
        ])),
        footer,
    );
}

fn popup_area(area: Rect) -> Rect {
    let width = area.width.saturating_sub(6).max(24).min(area.width);
    let height = area.height.saturating_sub(2).max(8).min(area.height);
    super::centered(area, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::client::OpenProjectResponse;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    #[test]
    fn settings_render_dangerous_toggles_with_textual_off_markers() {
        let mut app = App::new(OpenProjectResponse {
            project_id: "p".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: None,
            worker: None,
            cron_pending: None,
        });
        app.test_open_settings_surface();
        app.test_set_thread_params(serde_json::json!({
            "auto_approve_editing_tools": false,
            "auto_approve_dangerous_commands": false,
        }));
        app.test_open_settings_surface();
        let backend = TestBackend::new(120, 50);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_settings(frame, &app, frame.area()))
            .unwrap();

        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("! auto-approve editing tools: OFF"));
        assert!(text.contains("! auto-approve dangerous commands: OFF"));
    }
}
