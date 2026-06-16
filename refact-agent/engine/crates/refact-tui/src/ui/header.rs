use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::keymap::{KeyAction, KeyContext};
use crate::theme::ThemeRole;

pub(crate) fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let project = app
        .current_project()
        .map(|project| project.slug.as_str())
        .unwrap_or("no project");
    let new = app
        .keymap()
        .binding_label(KeyContext::Main, KeyAction::NewChat)
        .unwrap_or_else(|| "Ctrl-N".to_string());
    let projects = app
        .keymap()
        .binding_label(KeyContext::Main, KeyAction::OpenProjects)
        .unwrap_or_else(|| "Ctrl-P".to_string());
    let model = app
        .keymap()
        .binding_label(KeyContext::Main, KeyAction::OpenModels)
        .unwrap_or_else(|| "Ctrl-M".to_string());
    let mode = app
        .keymap()
        .binding_label(KeyContext::Main, KeyAction::OpenModes)
        .unwrap_or_else(|| "Ctrl-O".to_string());
    let help = app
        .keymap()
        .binding_label(KeyContext::Main, KeyAction::ShowHelp)
        .unwrap_or_else(|| "?".to_string());
    let vim = if app.vim_enabled() {
        format!(" · vim {}", app.vim_mode().label())
    } else {
        String::new()
    };
    let accent = app.theme().style(ThemeRole::Accent);
    let muted = app.theme().style(ThemeRole::Muted);
    let mut spans = vec![
        Span::styled("refact", accent),
        Span::raw(" "),
        Span::styled(project.to_string(), accent),
        Span::styled(" | ", muted),
    ];
    append_header_action(&mut spans, &new, "new", accent, muted);
    spans.push(Span::styled(" · ", muted));
    append_header_action(&mut spans, &projects, "projects", accent, muted);
    spans.push(Span::styled(" · ", muted));
    append_header_action(&mut spans, &model, "model", accent, muted);
    spans.push(Span::styled(" · ", muted));
    append_header_action(&mut spans, &mode, "mode", accent, muted);
    spans.push(Span::styled(" · ", muted));
    append_header_action(&mut spans, &help, "help", accent, muted);
    if !vim.is_empty() {
        spans.push(Span::styled(vim, muted));
    }
    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

fn append_header_action(
    spans: &mut Vec<Span<'static>>,
    key: &str,
    label: &'static str,
    accent: Style,
    muted: Style,
) {
    spans.push(Span::styled(key.to_string(), accent));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(label, muted));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::client::OpenProjectResponse;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;
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

    #[test]
    fn header_uses_accent_and_muted_styles_without_changing_labels() {
        let app = App::new(project());
        let backend = TestBackend::new(100, 1);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_header(frame, &app, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("refact demo |"));
        assert!(text.contains("new ·"));
        assert!(text.contains("projects ·"));
        assert!(text.contains("model ·"));
        assert!(text.contains("mode ·"));
        assert!(text.contains("help"));
        assert_eq!(
            buffer[(0, 0)].style().fg,
            app.theme().style(ThemeRole::Accent).fg
        );
        assert!(buffer[(0, 0)].style().add_modifier.contains(Modifier::BOLD));
        assert_eq!(
            buffer[(7, 0)].style().fg,
            app.theme().style(ThemeRole::Accent).fg
        );
        assert_eq!(
            buffer[(11, 0)].style().fg,
            app.theme().style(ThemeRole::Muted).fg
        );
    }
}
