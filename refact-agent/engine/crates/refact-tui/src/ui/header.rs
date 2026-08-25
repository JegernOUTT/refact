use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::keymap::{KeyAction, KeyContext};
use crate::theme::ThemeRole;
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

pub(crate) fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let line = truncate_line_with_ellipsis_if_overflow(header_line(app), area.width as usize);
    frame.render_widget(Paragraph::new(line), area);
}

fn header_line(app: &App) -> Line<'static> {
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
        .unwrap_or_else(|| "Alt-M".to_string());
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
    let worktree = app.worktree_meta().map(|meta| {
        let branch = meta
            .branch
            .as_deref()
            .filter(|branch| !branch.is_empty())
            .unwrap_or("unknown");
        let id = meta
            .id
            .as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or("worktree");
        format!(" · wt {id} {branch}")
    });
    let accent = app.theme().style(ThemeRole::Accent);
    let muted = app.theme().style(ThemeRole::Muted);
    let mut spans = vec![
        Span::styled("refact", accent),
        Span::raw(" "),
        Span::styled(project.to_string(), accent),
    ];
    if let Some(worktree) = worktree {
        spans.push(Span::styled(worktree, muted));
    }
    spans.push(Span::styled(" | ", muted));
    append_header_action(&mut spans, &new, "new", accent, muted);
    spans.push(Span::styled(" · ", muted));
    append_header_action(&mut spans, &projects, "projects", accent, muted);
    spans.push(Span::styled(" · ", muted));
    append_header_action(&mut spans, &model, "model", accent, muted);
    spans.push(Span::styled(" · ", muted));
    append_header_action(&mut spans, &mode, "mode", accent, muted);
    spans.push(Span::styled(" · ", muted));
    append_header_action(&mut spans, &help, "help", accent, muted);
    if app.task_board_discoverable() {
        spans.push(Span::styled(" · ", muted));
        spans.push(Span::styled("/board", accent));
        spans.push(Span::raw(" "));
        spans.push(Span::styled("tasks", muted));
    }
    if !vim.is_empty() {
        spans.push(Span::styled(vim, muted));
    }
    Line::from(spans)
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
            pinned: Some(false),
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

    #[test]
    fn header_uses_the_footer_ellipsis_convention() {
        let app = App::new(project());
        let width = 24;
        let header = truncate_line_with_ellipsis_if_overflow(header_line(&app), width);
        let footer = crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow(
            crate::ui::footer::footer_line(&crate::ui::footer::FooterData::from_app(&app)),
            width,
        );

        for line in [&header, &footer] {
            assert!(crate::vendored::line_truncation::line_width(line) <= width);
            assert_eq!(
                line.spans.last().map(|span| span.content.as_ref()),
                Some("…")
            );
        }
    }

    #[test]
    fn task_modes_advertise_the_board_when_surfaces_are_enabled() {
        let previous = std::env::var_os("REFACT_TUI_SURFACES");
        std::env::set_var("REFACT_TUI_SURFACES", "1");
        let mut app = App::new(project());
        app.test_set_task_context(Some("task-1".to_string()), Some("task_planner".to_string()));

        let text = header_line(&app)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("/board tasks"));
        match previous {
            Some(value) => std::env::set_var("REFACT_TUI_SURFACES", value),
            None => std::env::remove_var("REFACT_TUI_SURFACES"),
        }
    }

    #[test]
    fn header_shows_worktree_identity_only_for_worktree_backed_chat() {
        let mut app = App::new(project());
        let initial = header_line(&app)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(!initial.contains("wt wt-1"));

        app.apply_chat_event(crate::client::ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: serde_json::json!({
                "thread": {
                    "worktree": {
                        "id": "wt-1",
                        "branch": "refact/task/T-59",
                        "root": "/tmp/wt"
                    }
                },
                "runtime": {"state": "idle"},
                "messages": []
            }),
        });

        let header = header_line(&app)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(header.contains("wt wt-1 refact/task/T-59"));
    }
}
