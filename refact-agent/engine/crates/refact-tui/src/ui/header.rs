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
