use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, SessionState};
use crate::key_hint;
use crate::theme::ThemeRole;
use crate::ui_consts::{FOOTER_INDENT_COLS, LIVE_PREFIX_COLS};
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

const MAX_COMPOSER_ROWS: u16 = 8;
const HINT_ROWS: u16 = 1;
const BORDER_ROWS: u16 = 2;
const BORDER_COLS: u16 = 2;
const FRAMED_MIN_FRAME_HEIGHT: u16 = 16;
const HINT_MIN_FRAME_HEIGHT: u16 = 11;

pub(crate) fn desired_height(app: &App, width: u16, frame_height: u16) -> u16 {
    let framed = frame_height >= FRAMED_MIN_FRAME_HEIGHT;
    let text_width = if framed {
        text_width_for(width)
    } else {
        width.saturating_sub(LIVE_PREFIX_COLS + 1).max(1)
    };
    let text_rows = app.composer_state().height(text_width, MAX_COMPOSER_ROWS);
    let chrome = if framed {
        BORDER_ROWS + HINT_ROWS
    } else if frame_height >= HINT_MIN_FRAME_HEIGHT {
        HINT_ROWS
    } else {
        0
    };
    text_rows
        .saturating_add(chrome)
        .saturating_add(app.queue_preview_height())
}

fn text_width_for(width: u16) -> u16 {
    width
        .saturating_sub(BORDER_COLS + LIVE_PREFIX_COLS + 1)
        .max(1)
}

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

    let text_rows = app
        .composer_state()
        .height(text_width_for(input_area.width), MAX_COMPOSER_ROWS)
        .max(1);
    let framed = input_area.height >= text_rows + BORDER_ROWS
        && input_area.width > BORDER_COLS + LIVE_PREFIX_COLS + 1;
    let chrome_rows = if framed { BORDER_ROWS } else { 0 };
    let hint_height = if input_area.height > text_rows + chrome_rows {
        HINT_ROWS
    } else {
        0
    };
    let box_area = Rect {
        height: input_area.height.saturating_sub(hint_height),
        ..input_area
    };
    let status = composer_status(app);
    let inner = if framed {
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(app.theme().style(ThemeRole::Border));
        if let Some(status) = status.as_deref() {
            block = block.title(Line::from(Span::styled(
                format!(" {status} "),
                Style::default().fg(Color::DarkGray),
            )));
        }
        let inner = block.inner(box_area);
        frame.render_widget(block, box_area);
        Rect {
            x: inner.x.saturating_add(1),
            width: inner.width.saturating_sub(1),
            ..inner
        }
    } else {
        box_area
    };
    let editor_area = Rect {
        x: inner.x.saturating_add(LIVE_PREFIX_COLS),
        y: inner.y,
        width: inner.width.saturating_sub(LIVE_PREFIX_COLS),
        height: inner.height,
    };
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
            Rect {
                x: inner.x,
                y: inner.y,
                width: LIVE_PREFIX_COLS.min(inner.width),
                height: 1,
            },
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

    if hint_height > 0 {
        let hint_area = Rect {
            x: input_area.x,
            y: box_area.bottom(),
            width: input_area.width,
            height: hint_height,
        };
        let hints = truncate_line_with_ellipsis_if_overflow(
            composer_hint_line(app, if framed { None } else { status }),
            hint_area.width as usize,
        );
        frame.render_widget(Paragraph::new(hints), hint_area);
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
        _ if app.ctrl_c_quit_armed().is_some() => Some("Ctrl-C again to exit".to_string()),
        SessionState::Generating => Some("generating · Enter queues · Esc cancels".to_string()),
        SessionState::ExecutingTools => {
            Some("running tools · Enter queues · Esc cancels".to_string())
        }
        SessionState::Paused => Some("approval pending · Enter queues · Esc cancels".to_string()),
        SessionState::WaitingIde => {
            Some("waiting for IDE… · Enter queues · Esc aborts".to_string())
        }
        SessionState::WaitingUserInput => Some("waiting for input".to_string()),
        SessionState::Completed => Some("completed".to_string()),
        SessionState::Error => Some("error".to_string()),
        _ if app.vim_enabled() => Some(format!("vim {}", app.vim_mode().label())),
        _ => None,
    }
}

fn composer_hint_line(app: &App, status: Option<String>) -> Line<'static> {
    let mut spans = vec![Span::raw(" ".repeat(FOOTER_INDENT_COLS))];
    if let Some(status) = status {
        spans.push(Span::styled(status, Style::default().fg(Color::DarkGray)));
        spans.push(Span::raw("   "));
    }
    let busy = matches!(
        app.session_state(),
        SessionState::Generating
            | SessionState::ExecutingTools
            | SessionState::Paused
            | SessionState::WaitingIde
    );
    spans.extend(key_hint::pair("Enter", if busy { "queue" } else { "send" }).spans);
    spans.push(Span::raw("  ·  "));
    spans.extend(key_hint::pair(newline_hint(app), "newline").spans);
    if busy {
        spans.push(Span::raw("  ·  "));
        spans.extend(key_hint::pair("Esc", "interrupt").spans);
    }
    spans.push(Span::raw("  ·  "));
    let help = app
        .keymap()
        .binding_label(
            crate::keymap::KeyContext::Main,
            crate::keymap::KeyAction::ShowHelp,
        )
        .and_then(|label| label.split('/').next().map(str::to_string))
        .unwrap_or_else(|| "?".to_string());
    spans.extend(key_hint::pair(help, "shortcuts").spans);
    Line::from(spans).dim()
}

fn newline_hint(app: &App) -> String {
    if app.enhanced_keys_supported() {
        "Shift-Enter".to_string()
    } else {
        "\\+Enter".to_string()
    }
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
            pinned: Some(false),
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
    fn composer_renders_framed_placeholder_and_footer_hints() {
        let app = App::new(project());
        let mut terminal = Terminal::new(TestBackend::new(64, 4)).unwrap();

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
        assert!(text.contains("╭"));
        assert!(text.contains("╰"));
        assert!(text.contains("│"));
        assert!(!text.contains("message"));
        assert!(placeholder.style().add_modifier.contains(Modifier::DIM));
        assert_eq!(desired_height(&app, 64, 40), 4);
        assert_eq!(desired_height(&app, 64, 15), 2);
        assert_eq!(desired_height(&app, 64, 10), 1);
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
        assert!(text.contains("│"));
    }

    #[test]
    fn composer_cursor_accounts_for_border_and_prompt() {
        let mut app = App::new(project());
        app.test_set_composer_text("hello");
        let mut terminal = Terminal::new(TestBackend::new(40, 4)).unwrap();

        terminal
            .draw(|frame| render_composer(frame, &app, frame.area()))
            .unwrap();

        terminal.backend_mut().assert_cursor_position((9, 1));
    }

    #[test]
    fn composer_degrades_to_borderless_layout_in_two_rows() {
        let mut app = App::new(project());
        app.test_set_composer_text("hello");
        let mut terminal = Terminal::new(TestBackend::new(40, 2)).unwrap();

        terminal
            .draw(|frame| render_composer(frame, &app, frame.area()))
            .unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("› hello"));
        assert!(!text.contains("╭"));
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
