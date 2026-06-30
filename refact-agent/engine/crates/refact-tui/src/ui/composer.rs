use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, SessionState};
use crate::key_hint;
use crate::style::user_message_style;
use crate::ui_consts::{FOOTER_INDENT_COLS, LIVE_PREFIX_COLS};
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

const MAX_COMPOSER_ROWS: u16 = 8;
const FOOTER_ROWS: u16 = 1;

pub(crate) fn render_composer(frame: &mut Frame<'_>, app: &App, area: Rect) {
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
    if input_area.height == 0 || input_area.width == 0 {
        return;
    }

    frame.render_widget(Block::default().style(user_message_style()), input_area);

    let status = composer_status(app);
    let footer_height = FOOTER_ROWS.min(input_area.height.saturating_sub(1));
    let editor_area = editor_area(input_area, footer_height);
    let text_width = editor_area.width.max(1);
    let max_rows = editor_area.height.min(MAX_COMPOSER_ROWS).max(1);
    let view = app.composer_state().view(text_width, max_rows);
    let paste_placeholders = app.composer_state().pending_paste_placeholders();
    let lines = if app.composer().is_empty() {
        vec![Line::from(Span::styled(
            "Ask Refact…",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ))]
    } else {
        view.lines
            .into_iter()
            .map(|line| line_with_paste_placeholders(line, &paste_placeholders))
            .collect()
    };
    if editor_area.height > 0 && editor_area.width > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(prompt_span(app))),
            prompt_area(input_area),
        );
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            editor_area,
        );
        if !app.composer().is_empty() {
            let x = editor_area
                .x
                .saturating_add(view.cursor_col.min(text_width.saturating_sub(1)));
            let y = editor_area
                .y
                .saturating_add(view.cursor_row.min(editor_area.height.saturating_sub(1)));
            frame.set_cursor_position((x, y));
        }
    }

    if footer_height > 0 {
        let footer_area = Rect {
            x: input_area.x,
            y: input_area
                .y
                .saturating_add(input_area.height.saturating_sub(1)),
            width: input_area.width,
            height: 1,
        };
        let footer = truncate_line_with_ellipsis_if_overflow(
            composer_footer_line(app, status),
            footer_area.width as usize,
        );
        frame.render_widget(Paragraph::new(footer), footer_area);
    }
}

fn line_with_paste_placeholders(line: String, placeholders: &[String]) -> Line<'static> {
    if placeholders.is_empty() {
        return Line::from(line);
    }
    let mut spans = Vec::new();
    let mut rest = line.as_str();
    while !rest.is_empty() {
        let Some((start, placeholder)) = placeholders
            .iter()
            .filter_map(|placeholder| rest.find(placeholder).map(|start| (start, placeholder)))
            .min_by_key(|(start, _)| *start)
        else {
            spans.push(Span::raw(rest.to_string()));
            break;
        };
        if start > 0 {
            spans.push(Span::raw(rest[..start].to_string()));
        }
        spans.push(Span::styled(
            placeholder.clone(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
        rest = &rest[start + placeholder.len()..];
    }
    Line::from(spans)
}

fn editor_area(area: Rect, footer_height: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(LIVE_PREFIX_COLS),
        y: area.y,
        width: area.width.saturating_sub(LIVE_PREFIX_COLS + 1),
        height: area.height.saturating_sub(footer_height),
    }
}

fn prompt_area(area: Rect) -> Rect {
    Rect {
        x: area.x,
        y: area.y,
        width: LIVE_PREFIX_COLS,
        height: 1,
    }
}

fn prompt_span(app: &App) -> Span<'static> {
    let prompt = if app.session_state() == SessionState::WaitingUserInput {
        "?"
    } else {
        "›"
    };
    Span::styled(prompt, Style::default().add_modifier(Modifier::BOLD))
}

fn composer_status(app: &App) -> Option<String> {
    match app.session_state() {
        _ if app.composer_history_search().is_some() => Some(history_search_title(app)),
        SessionState::Generating | SessionState::ExecutingTools => {
            Some("generating · Enter queues · Esc cancels".to_string())
        }
        SessionState::Paused => Some("approval pending".to_string()),
        SessionState::WaitingUserInput => Some("waiting for input".to_string()),
        _ if app.vim_enabled() => Some(format!("vim {}", app.vim_mode().label())),
        _ => None,
    }
}

fn composer_footer_line(app: &App, status: Option<String>) -> Line<'static> {
    let mut spans = vec![Span::raw(" ".repeat(FOOTER_INDENT_COLS))];
    if let Some(status) = status {
        spans.push(Span::styled(status, Style::default().fg(Color::DarkGray)));
        spans.push(Span::raw("   "));
    }
    spans.extend(key_hint::pair("Enter", "send").spans);
    spans.push(Span::raw("   "));
    let newline = app
        .keymap()
        .binding_label(
            crate::keymap::KeyContext::Main,
            crate::keymap::KeyAction::InsertNewline,
        )
        .and_then(|label| label.split('/').next().map(str::to_string))
        .unwrap_or_else(|| "Ctrl-J".to_string());
    spans.extend(key_hint::pair(newline, "newline").spans);
    if matches!(
        app.session_state(),
        SessionState::Generating | SessionState::ExecutingTools
    ) {
        spans.push(Span::raw("   "));
        spans.extend(key_hint::pair("Enter", "queue").spans);
    }
    Line::from(spans).dim()
}

pub(crate) fn history_search_title(app: &App) -> String {
    let Some(search) = app.composer_history_search() else {
        return "history search".to_string();
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
    format!("history search: {query} · {status} · Enter accept · Esc cancel")
}

pub(crate) fn render_queue_preview(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let local_len = app.input_queue().len();
    let mut spans = vec![Span::styled(
        format!(" queued ({local_len}) "),
        Style::default().fg(Color::DarkGray),
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
    let line =
        truncate_line_with_ellipsis_if_overflow(Line::from(spans).dim(), area.width as usize);
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ChatEvent, OpenProjectResponse};
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

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn composer_renders_deboxed_placeholder_and_footer_hints() {
        let app = App::new(project());
        let mut terminal = Terminal::new(TestBackend::new(64, 3)).unwrap();

        terminal
            .draw(|frame| render_composer(frame, &app, frame.area()))
            .unwrap();
        let text = buffer_text(&terminal);
        let placeholder = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .find(|cell| cell.symbol() == "A")
            .expect("placeholder rendered");

        assert!(text.contains("› Ask Refact…"));
        assert!(text.contains("Enter send"));
        assert!(text.contains("newline"));
        assert!(!text.contains("┌"));
        assert!(!text.contains("message"));
        assert!(placeholder.style().add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn composer_status_line_replaces_border_title_when_generating() {
        let mut app = App::new(project());
        app.apply_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "runtime_updated".to_string(),
            raw: serde_json::json!({"state": "generating"}),
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 3)).unwrap();

        terminal
            .draw(|frame| render_composer(frame, &app, frame.area()))
            .unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("generating · Enter queues · Esc cancels"));
        assert!(text.contains("Enter queue"));
        assert!(!text.contains("message (Enter queues"));
        assert!(!text.contains("┌"));
    }

    #[test]
    fn composer_cursor_accounts_for_removed_border_and_prompt() {
        let mut app = App::new(project());
        app.test_set_composer_text("hello");
        let mut terminal = Terminal::new(TestBackend::new(40, 2)).unwrap();

        terminal
            .draw(|frame| render_composer(frame, &app, frame.area()))
            .unwrap();

        terminal.backend_mut().assert_cursor_position((7, 0));
    }

    #[test]
    fn composer_large_paste_placeholder_is_dim() {
        let mut app = App::new(project());
        app.test_insert_paste(&"x".repeat(crate::composer::LARGE_PASTE_CHAR_THRESHOLD + 1));
        let mut terminal = Terminal::new(TestBackend::new(80, 2)).unwrap();

        terminal
            .draw(|frame| render_composer(frame, &app, frame.area()))
            .unwrap();
        let text = buffer_text(&terminal);
        let pasted_cell = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .find(|cell| cell.symbol() == "P")
            .expect("paste placeholder rendered");

        assert!(text.contains("[Pasted 1001 chars]"));
        assert!(pasted_cell.style().add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn queue_preview_renders_dim_without_box() {
        let mut app = App::new(project());
        app.apply_chat_event(ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "queue_updated".to_string(),
            raw: serde_json::json!({
                "queue_size": 1,
                "queued_items": [{"preview": "server-side"}]
            }),
        });
        let mut terminal = Terminal::new(TestBackend::new(64, 4)).unwrap();

        terminal
            .draw(|frame| render_composer(frame, &app, frame.area()))
            .unwrap();
        let text = buffer_text(&terminal);
        let queue_cell = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .find(|cell| cell.symbol() == "q")
            .expect("queue preview rendered");

        assert!(text.contains("server queued (1): server-side"));
        assert!(!text.contains("┌"));
        assert!(queue_cell.style().add_modifier.contains(Modifier::DIM));
    }
}
