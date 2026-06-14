use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ComposerMode, ProjectPickerState, SessionState};
use crate::approvals::render_modal_lines;
use crate::events_pane::{render_event_lines, render_worker_lines};
use crate::keymap::{HelpRow, KeyAction, KeyContext};
use crate::overlay::PagerMode;
use crate::pickers::PickerState;
use crate::theme::ThemeRole;
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::vendored::terminal_hyperlinks::{
    hyperlinks_enabled_from_env, mark_buffer_hyperlinks, visible_lines, HyperlinkLine,
};

pub mod footer;

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    app.begin_frame_render();
    let area = frame.area();
    let main_constraints = if app.events_pane().open {
        vec![
            Constraint::Length(1),
            Constraint::Percentage(62),
            Constraint::Percentage(38),
            Constraint::Length(app.composer_height(area.width)),
            Constraint::Length(1),
        ]
    } else {
        vec![
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(app.composer_height(area.width)),
            Constraint::Length(1),
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(main_constraints)
        .split(area);

    render_header(frame, app, chunks[0]);
    render_transcript(frame, app, chunks[1]);
    let composer_area = if app.events_pane().open {
        chunks[3]
    } else {
        chunks[2]
    };
    if app.events_pane().open {
        render_events_pane(frame, app, chunks[2]);
        render_composer(frame, app, composer_area);
        footer::render(frame, app, chunks[4]);
    } else {
        render_composer(frame, app, composer_area);
        footer::render(frame, app, chunks[3]);
    }
    if matches!(app.composer_mode(), ComposerMode::ProjectPicker) {
        render_project_picker(frame, app.project_picker(), area);
    }
    if let Some(picker) = app.modal_picker() {
        render_modal_picker(frame, picker, area, composer_area);
    }
    if app.transcript_overlay().is_some() {
        let overlay_height = area.height.saturating_sub(4).max(8);
        app.set_transcript_overlay_visible_height(overlay_height.saturating_sub(3) as usize);
    }
    if let Some(overlay) = app.transcript_overlay() {
        render_transcript_overlay(frame, overlay, area);
    }
    if let Some(modal) = app.approval_modal() {
        render_approval_modal(frame, modal, area);
    }
    if app.help_open() {
        render_help(frame, app, area);
    }
}

fn render_transcript_overlay(
    frame: &mut Frame<'_>,
    overlay: &crate::overlay::PagerOverlay,
    area: Rect,
) {
    let width = area.width.saturating_sub(6).max(24);
    let height = area.height.saturating_sub(4).max(8);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    let mode = match overlay.mode() {
        PagerMode::Rendered => "rendered",
        PagerMode::Raw => "copy/raw",
    };
    let block =
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} · {} ", overlay.title(), mode));
    let inner_height = popup.height.saturating_sub(3) as usize;
    let body = overlay
        .visible_lines(inner_height)
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(body).block(block).wrap(Wrap { trim: false }),
        popup,
    );
    let status_area = Rect {
        x: popup.x.saturating_add(1),
        y: popup.y + popup.height.saturating_sub(2),
        width: popup.width.saturating_sub(2),
        height: 1,
    };
    let status = truncate_line_with_ellipsis_if_overflow(
        Line::from(Span::styled(
            overlay.status(),
            Style::default().fg(Color::DarkGray),
        )),
        status_area.width as usize,
    );
    frame.render_widget(Paragraph::new(status), status_area);
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
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
    let line = Line::from(vec![
        Span::styled("refact ", app.theme().style(ThemeRole::Accent)),
        Span::raw(project),
        Span::styled(
            format!("  {new} new · {projects} projects · {model} model · {mode} mode · {help} help{vim}"),
            app.theme().style(ThemeRole::Muted),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_transcript(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if app.native_scrollback() {
        render_live_transcript(frame, app, area);
    } else {
        render_full_transcript(frame, app, area);
    }
}

fn render_live_transcript(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let mut lines = Vec::<HyperlinkLine>::new();
    for (idx, item) in app.visible_transcript().iter().enumerate() {
        lines.extend(crate::history::render_transcript_item_hyperlink_lines(
            item,
            area.width.saturating_sub(2) as usize,
            app.transcript_item_selected(idx, item),
        ));
    }
    app.note_rendered_messages(app.visible_transcript().len());
    if lines.is_empty() {
        lines.push(HyperlinkLine::new(Line::from(Span::styled(
            "History is in native scrollback. Start typing below.",
            Style::default().fg(Color::DarkGray),
        ))));
    }
    let visible = visible_lines(lines.clone());
    frame.render_widget(
        Paragraph::new(visible)
            .block(Block::default().borders(Borders::BOTTOM))
            .wrap(Wrap { trim: false }),
        area,
    );
    mark_buffer_hyperlinks(
        frame.buffer_mut(),
        area,
        &lines,
        hyperlinks_enabled_from_env(),
    );
}

fn render_full_transcript(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let mut lines = Vec::<HyperlinkLine>::new();
    for (idx, item) in app.visible_transcript().iter().enumerate() {
        lines.extend(crate::history::render_transcript_item_hyperlink_lines(
            item,
            area.width.saturating_sub(2) as usize,
            app.transcript_item_selected(idx, item),
        ));
    }
    app.note_rendered_messages(app.visible_transcript().len());
    if lines.is_empty() {
        lines.push(HyperlinkLine::new(Line::from(Span::styled(
            "Start typing. Enter sends, Shift-Enter inserts a newline.",
            Style::default().fg(Color::DarkGray),
        ))));
    }
    let total = lines.len();
    let height = area.height as usize;
    let start = total
        .saturating_sub(height)
        .saturating_sub(app.scroll_offset());
    let end = total.saturating_sub(app.scroll_offset()).min(total);
    let view_links = lines[start..end].to_vec();
    let view = visible_lines(view_links.clone());
    frame.render_widget(
        Paragraph::new(view)
            .block(Block::default().borders(Borders::BOTTOM))
            .wrap(Wrap { trim: false }),
        area,
    );
    mark_buffer_hyperlinks(
        frame.buffer_mut(),
        area,
        &view_links,
        hyperlinks_enabled_from_env(),
    );
}

fn render_composer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let queue_height = app.queue_preview_height().min(area.height);
    let input_area = if queue_height > 0 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(queue_height), Constraint::Min(1)])
            .split(area);
        render_queue_preview(frame, app, chunks[0]);
        chunks[1]
    } else {
        area
    };
    let title = match app.session_state() {
        _ if app.composer_history_search().is_some() => history_search_title(app),
        SessionState::Generating => " message (Enter queues · Esc cancels) ".to_string(),
        SessionState::Paused => " approval pending ".to_string(),
        SessionState::WaitingUserInput => " waiting for input ".to_string(),
        _ if app.vim_enabled() => format!(" message · vim {} ", app.vim_mode().label()),
        _ => " message ".to_string(),
    };
    let inner_width = input_area.width.saturating_sub(2).max(1);
    let max_rows = input_area.height.saturating_sub(2).max(1);
    let view = app.composer_state().view(inner_width, max_rows);
    let lines = if app.composer().is_empty() {
        vec![Line::from(Span::styled(
            "Ask Refact…",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        view.lines.into_iter().map(Line::from).collect()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title.as_str()))
            .wrap(Wrap { trim: false }),
        input_area,
    );
    if !app.composer().is_empty() {
        let x = input_area
            .x
            .saturating_add(1)
            .saturating_add(view.cursor_col.min(inner_width.saturating_sub(1)));
        let y = input_area
            .y
            .saturating_add(1)
            .saturating_add(view.cursor_row.min(max_rows.saturating_sub(1)));
        frame.set_cursor_position((x, y));
    }
}

fn history_search_title(app: &App) -> String {
    let Some(search) = app.composer_history_search() else {
        return " history search ".to_string();
    };
    let status = if search.total == 0 {
        "no matches".to_string()
    } else {
        format!("{}/{}", search.selected, search.total)
    };
    let query = if search.query.is_empty() {
        "type to filter".to_string()
    } else {
        search.query
    };
    format!(" history search: {query} · {status} · Enter accept · Esc cancel ")
}

fn render_queue_preview(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let local_len = app.input_queue().len();
    let mut spans = vec![Span::styled(
        format!(" queued ({local_len}) "),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )];
    for (idx, item) in app.input_queue().items().iter().enumerate().take(3) {
        let selected = app.input_queue().selected_index() == Some(idx);
        let editing = app.input_queue().editing_index() == Some(idx);
        let marker = if editing {
            "✎"
        } else if selected {
            "›"
        } else {
            "•"
        };
        let text = item.text.replace('\n', " ⏎ ");
        spans.push(Span::styled(
            format!("{marker}{} ", idx + 1),
            Style::default().fg(if selected || editing {
                Color::Cyan
            } else {
                Color::DarkGray
            }),
        ));
        spans.push(Span::raw(text));
        spans.push(Span::styled("  ", Style::default().fg(Color::DarkGray)));
    }
    if local_len > 3 {
        spans.push(Span::styled(
            format!("+{} more  ", local_len - 3),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if app.server_queue_size() > 0 {
        let preview = app
            .server_queue_previews()
            .first()
            .map(|value| format!(": {}", value.replace('\n', " ⏎ ")))
            .unwrap_or_default();
        spans.push(Span::styled(
            format!(" server queued ({}){preview} ", app.server_queue_size()),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let line = truncate_line_with_ellipsis_if_overflow(Line::from(spans), area.width as usize);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_project_picker(frame: &mut Frame<'_>, picker: &ProjectPickerState, area: Rect) {
    let width = area.width.saturating_sub(8).min(80);
    let height = area.height.saturating_sub(6).min(20);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" projects: {} ", picker.filter));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let items: Vec<ListItem<'_>> = picker
        .filtered_projects()
        .iter()
        .enumerate()
        .map(|(idx, project)| {
            let marker = if idx == picker.selected { "›" } else { " " };
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::Cyan)),
                Span::raw(" "),
                Span::styled(
                    project.slug.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", project.root.display()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();
    let list = if items.is_empty() {
        List::new(vec![ListItem::new("No projects match")])
    } else {
        List::new(items)
    };
    frame.render_widget(list, inner);
}
fn render_modal_picker(frame: &mut Frame<'_>, picker: &PickerState, area: Rect, composer: Rect) {
    let width = area.width.saturating_sub(8).min(86);
    let filtered = picker.filtered_items();
    let rows = filtered.len().max(1).min(8) as u16;
    let height = rows
        .saturating_add(2)
        .min(area.height.saturating_sub(2).max(1));
    let max_y = area.y.saturating_add(area.height.saturating_sub(height));
    let wanted_y = composer.y.saturating_sub(height);
    let y = wanted_y.min(max_y);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let title = if picker.is_multi() {
        format!(
            " {}: {} selected · {} ",
            picker.title(),
            picker.selected_count(),
            picker.filter
        )
    } else {
        format!(" {}: {} ", picker.title(), picker.filter)
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let items: Vec<ListItem<'_>> = filtered
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let cursor = if idx == picker.selected { "›" } else { " " };
            let checked = if picker.is_multi() {
                if picker.is_selected(&item.id) {
                    "☑"
                } else {
                    "☐"
                }
            } else {
                ""
            };
            let title = if checked.is_empty() {
                item.title.clone()
            } else {
                format!("{checked} {}", item.title)
            };
            ListItem::new(Line::from(vec![
                Span::styled(cursor, Style::default().fg(Color::Cyan)),
                Span::raw(" "),
                Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("  {}", item.description),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();
    let list = if items.is_empty() {
        List::new(vec![ListItem::new("No entries match")])
    } else {
        List::new(items)
    };
    frame.render_widget(list, inner);
}

fn render_approval_modal(
    frame: &mut Frame<'_>,
    modal: &crate::approvals::ApprovalModalState,
    area: Rect,
) {
    let width = area.width.saturating_sub(6).min(96);
    let max_height = if modal.details_open() { 28 } else { 16 };
    let height = area.height.saturating_sub(6).min(max_height);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL).title(" approval ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(visible_approval_lines(
            modal,
            inner.width as usize,
            inner.height as usize,
        ))
        .wrap(Wrap { trim: false }),
        inner,
    );
}

fn visible_approval_lines(
    modal: &crate::approvals::ApprovalModalState,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    let lines = render_modal_lines(modal, width);
    if !modal.details_open() || height == 0 {
        return lines;
    }
    let fixed = lines.len().min(2);
    let body_height = height.saturating_sub(fixed);
    let body_start = fixed + modal.detail_scroll().min(lines.len().saturating_sub(fixed));
    lines[..fixed]
        .iter()
        .cloned()
        .chain(lines[body_start..].iter().take(body_height).cloned())
        .collect()
}

fn render_events_pane(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    frame.render_widget(
        Paragraph::new(render_event_lines(app.events_pane().events()))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" daemon events "),
            )
            .wrap(Wrap { trim: false }),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(render_worker_lines(app.events_pane().workers()))
            .block(Block::default().borders(Borders::ALL).title(" workers "))
            .wrap(Wrap { trim: false }),
        columns[1],
    );
}

fn render_help(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let rows = app.keymap_help_rows();
    let row_count = rows.len().min(24) as u16;
    let popup = centered(
        area,
        area.width.saturating_sub(8).min(92),
        row_count.saturating_add(4).min(area.height),
    );
    frame.render_widget(Clear, popup);
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("Theme ", app.theme().style(ThemeRole::Muted)),
        Span::raw(app.theme().name().to_string()),
        Span::styled(" · vim ", app.theme().style(ThemeRole::Muted)),
        Span::raw(if app.vim_enabled() {
            app.vim_mode().label().to_string()
        } else {
            "off".to_string()
        }),
    ]));
    lines.push(Line::from(""));
    for row in rows.into_iter().take(24) {
        lines.push(help_row_line(row, app));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" help "))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn help_row_line(row: HelpRow, app: &App) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:>12} ", row.context.label()),
            app.theme().style(ThemeRole::Muted),
        ),
        Span::styled(
            format!("{:<20}", row.bindings),
            app.theme().style(ThemeRole::Highlight),
        ),
        Span::raw(row.description.to_string()),
    ])
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::app::TranscriptItem;
    use crate::approvals::{ApprovalModalState, PauseReason};
    use crate::client::OpenProjectResponse;
    use crate::tools::{ToolCard, ToolStatus};
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

    #[test]
    fn help_rows_are_generated_from_active_keymap() {
        let app = App::new(project());
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| render_help(frame, &app, frame.area()))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("show generated keymap help"));
        assert!(text.contains("Ctrl-N"));
    }

    #[test]
    fn render_smoke_draws_header_and_composer() {
        let mut app = App::new(project());
        app.set_native_scrollback(false);
        let backend = TestBackend::new(64, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("refact"));
        assert!(text.contains("Ask Refact"));
        assert!(text.contains("demo"));
    }

    #[test]
    fn native_scrollback_frame_renders_only_live_tail() {
        let mut app = App::new(project());
        app.set_native_scrollback(true);
        for idx in 0..5_000 {
            app.test_push_history_item(TranscriptItem::Notice(format!("history {idx}")));
        }
        app.test_push_tool(ToolCard::from_tool_call(
            &serde_json::json!({"id": "call-1", "name": "shell"}),
        ));
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        assert_eq!(app.rendered_message_count(), 1);
    }

    #[test]
    fn render_smoke_draws_modal_and_tool_card() {
        let mut app = App::new(project());
        app.set_native_scrollback(false);
        app.test_push_tool(
            ToolCard::from_tool_call(&serde_json::json!({"id": "call-1", "name": "shell"}))
                .with_result("+ok", ToolStatus::Success),
        );
        app.test_set_approval(ApprovalModalState::new(vec![PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "shell".to_string(),
            command: "echo hi".to_string(),
            rule: "default".to_string(),
            tool_call_id: "call-1".to_string(),
            integr_config_path: None,
            args: None,
            diff: None,
        }]));
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Approval required"));
        assert!(text.contains("shell"));
    }

    #[test]
    fn render_approval_detail_snapshot_with_diff_and_shell_command() {
        let mut app = App::new(project());
        let mut modal = ApprovalModalState::from_event(&serde_json::json!({
            "reasons": [
                {
                    "type": "confirmation",
                    "tool_name": "shell",
                    "command": "printf 'hi' && git diff",
                    "rule": "ask",
                    "tool_call_id": "call-shell",
                    "args": {"command": "printf 'hi' && git diff", "cwd": "/tmp/demo"}
                },
                {
                    "type": "confirmation",
                    "tool_name": "apply_patch",
                    "command": "apply patch",
                    "rule": "ask",
                    "tool_call_id": "call-patch",
                    "diff": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new"
                }
            ]
        }))
        .unwrap();
        modal.toggle_details();
        app.test_set_approval(modal);

        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("approval 1 of 1"));
        assert!(text.contains("shell command"));
        assert!(text.contains("printf 'hi' && git diff"));
        assert!(text.contains(r#""cwd": "/tmp/demo""#));
        assert!(text.contains("apply_patch"));
        assert!(text.contains("- old"));
        assert!(text.contains("+ new"));
    }
}
