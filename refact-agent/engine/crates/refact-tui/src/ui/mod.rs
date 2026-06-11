use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ComposerMode, ProjectPickerState, SessionState, TranscriptItem};
use crate::client::worker_state_label;
use crate::render::MarkdownRenderer;
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(app.composer_height(area.width)),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, app, chunks[0]);
    render_transcript(frame, app, chunks[1]);
    render_composer(frame, app, chunks[2]);
    render_status(frame, app, chunks[3]);
    if matches!(app.composer_mode(), ComposerMode::ProjectPicker) {
        render_project_picker(frame, app.project_picker(), area);
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
            "  Ctrl-N new · Ctrl-P projects · Ctrl-Q quit",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_transcript(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let renderer = MarkdownRenderer::new(Some(area.width.saturating_sub(2) as usize));
    let mut lines = Vec::new();
    for item in app.visible_transcript() {
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
            TranscriptItem::Tool(text) => lines.push(Line::from(Span::styled(
                format!("→ {text}"),
                Style::default().fg(Color::Yellow),
            ))),
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
    let state = app.session_state().as_str();
    let daemon_dot = if app.daemon_online() { "●" } else { "○" };
    let worker = app
        .current_worker()
        .map(|worker| worker_state_label(Some(worker)))
        .unwrap_or_else(|| "unknown".to_string());
    let status = format!(" {project} · {model} · {state} · daemon {daemon_dot} · worker {worker} ");
    let line = truncate_line_with_ellipsis_if_overflow(Line::from(status), area.width as usize);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

fn render_project_picker(frame: &mut Frame<'_>, picker: &ProjectPickerState, area: Rect) {
    let width = area.width.saturating_sub(8).min(80);
    let height = area.height.saturating_sub(6).min(20);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::client::OpenProjectResponse;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    #[test]
    fn render_smoke_draws_header_and_composer() {
        let project = OpenProjectResponse {
            project_id: "p1".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: false,
            worker: None,
            cron_pending: None,
        };
        let app = App::new(project);
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
}
