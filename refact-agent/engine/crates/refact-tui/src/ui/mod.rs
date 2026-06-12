use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ComposerMode, ProjectPickerState, SessionState, TranscriptItem};
use crate::approvals::render_modal_lines;
use crate::client::worker_state_label;
use crate::events_pane::{render_event_lines, render_worker_lines};
use crate::pickers::{PickerKind, PickerState};
use crate::render::MarkdownRenderer;
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

pub fn render(frame: &mut Frame<'_>, app: &App) {
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
    if app.events_pane().open {
        render_events_pane(frame, app, chunks[2]);
        render_composer(frame, app, chunks[3]);
        render_status(frame, app, chunks[4]);
    } else {
        render_composer(frame, app, chunks[2]);
        render_status(frame, app, chunks[3]);
    }
    if matches!(app.composer_mode(), ComposerMode::ProjectPicker) {
        render_project_picker(frame, app.project_picker(), area);
    }
    if let Some(picker) = app.modal_picker() {
        render_modal_picker(frame, picker, area);
    }
    if let Some(modal) = app.approval_modal() {
        render_approval_modal(frame, modal, area);
    }
    if app.help_open() {
        render_help(frame, area);
    }
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let project = app
        .current_project()
        .map(|project| project.slug.as_str())
        .unwrap_or("no project");
    let line = Line::from(vec![
        Span::styled(
            "refact ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(project),
        Span::styled(
            "  Ctrl-N new · Ctrl-P projects · Ctrl-M model · Ctrl-O mode · F2 events · ? help",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_transcript(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let renderer = MarkdownRenderer::new(Some(area.width.saturating_sub(2) as usize));
    let mut lines = Vec::new();
    for (idx, item) in app.visible_transcript().iter().enumerate() {
        match item {
            TranscriptItem::User(text) => {
                lines.push(Line::from(Span::styled(
                    "you",
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.extend(renderer.render(text));
            }
            TranscriptItem::Assistant(text) => {
                lines.push(Line::from(Span::styled(
                    "assistant",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.extend(renderer.render(text));
            }
            TranscriptItem::Reasoning(text, collapsed) => {
                let label = if *collapsed {
                    "reasoning collapsed"
                } else {
                    "reasoning"
                };
                lines.push(Line::from(Span::styled(
                    label,
                    Style::default().fg(Color::DarkGray),
                )));
                if !collapsed {
                    lines.extend(renderer.render(text));
                }
            }
            TranscriptItem::Tool(card) => {
                let selected = app.selected_tool_index() == Some(idx);
                let label = if selected { "tool selected" } else { "tool" };
                lines.push(Line::from(Span::styled(
                    label,
                    Style::default().fg(if selected { Color::Cyan } else { Color::Yellow }),
                )));
                lines.extend(card.render_lines(area.width.saturating_sub(2) as usize));
            }
            TranscriptItem::Notice(text) => lines.push(Line::from(Span::styled(
                text.clone(),
                Style::default().fg(Color::DarkGray),
            ))),
        }
        lines.push(Line::default());
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "Start typing. Enter sends, Shift-Enter inserts a newline.",
            Style::default().fg(Color::DarkGray),
        )));
    }
    let total = lines.len();
    let height = area.height as usize;
    let start = total
        .saturating_sub(height)
        .saturating_sub(app.scroll_offset());
    let end = total.saturating_sub(app.scroll_offset()).min(total);
    let view = lines[start..end].to_vec();
    frame.render_widget(
        Paragraph::new(view)
            .block(Block::default().borders(Borders::BOTTOM))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_composer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let title = match app.session_state() {
        SessionState::Generating => " message (Esc cancels) ",
        SessionState::Paused => " approval pending ",
        _ => " message ",
    };
    let text = if app.composer().is_empty() {
        Line::from(Span::styled(
            "Ask Refact…",
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(app.composer().to_string())
    };
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let project = app
        .current_project()
        .map(|project| project.slug.as_str())
        .unwrap_or("-");
    let model = app.model().unwrap_or("default");
    let mode = app.mode().unwrap_or("agent");
    let state = app.session_state().as_str();
    let daemon_dot = if app.daemon_online() { "●" } else { "○" };
    let worker = app
        .current_worker()
        .map(|worker| worker_state_label(Some(worker)))
        .unwrap_or_else(|| "unknown".to_string());
    let usage = app
        .usage()
        .map(|usage| format!(" · usage {}", usage.display()))
        .unwrap_or_default();
    let status = format!(
        " {project} · {model} · {mode} · {state} · daemon {daemon_dot} · worker {worker}{usage} "
    );
    let line = truncate_line_with_ellipsis_if_overflow(Line::from(status), area.width as usize);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().fg(Color::DarkGray)),
        area,
    );
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

fn render_modal_picker(frame: &mut Frame<'_>, picker: &PickerState, area: Rect) {
    let width = area.width.saturating_sub(8).min(86);
    let height = area.height.saturating_sub(6).min(22);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    let title = match picker.kind {
        PickerKind::Model => " models ",
        PickerKind::Mode => " modes ",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("{title}{} ", picker.filter));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let items: Vec<ListItem<'_>> = picker
        .filtered_items()
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let marker = if idx == picker.selected { "›" } else { " " };
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::Cyan)),
                Span::raw(" "),
                Span::styled(
                    item.title.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", item.id),
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
    let height = area.height.saturating_sub(6).min(16);
    let popup = centered(area, width, height);
    frame.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL).title(" approval ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(render_modal_lines(modal, inner.width as usize)).wrap(Wrap { trim: false }),
        inner,
    );
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

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    let popup = centered(
        area,
        area.width.saturating_sub(8).min(78),
        15.min(area.height),
    );
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from("Enter send · Shift-Enter newline · Esc cancel/close"),
        Line::from("Ctrl-N new chat · Ctrl-P projects · Ctrl-M models · Ctrl-O modes"),
        Line::from("F2 daemon events/workers · Tab select next tool · Enter/Space expand tool"),
        Line::from("Approvals: y approve once · a approve for chat · n/Esc deny · v full args"),
        Line::from("Ctrl-R toggle reasoning · PageUp/PageDown scroll · Ctrl-Q quit"),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" help "))
            .wrap(Wrap { trim: false }),
        popup,
    );
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
    fn render_smoke_draws_header_and_composer() {
        let app = App::new(project());
        let backend = TestBackend::new(64, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
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
    fn render_smoke_draws_modal_and_tool_card() {
        let mut app = App::new(project());
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
        }]));
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
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
}
