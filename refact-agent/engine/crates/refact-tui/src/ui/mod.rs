use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use crate::app::{App, ComposerMode};

mod approval;
mod ask;
mod composer;
pub mod events;
pub mod footer;
pub(crate) mod goal_dock;
mod header;
mod help;
mod history;
pub(crate) mod menu;
mod overlay;
pub mod picker;
pub mod session_tabs;
mod settings;
pub mod status_card;
pub mod status_indicator;
mod transcript;
mod worktree;

const ASCII_FRAME_MAX_WIDTH: u16 = 60;
const BORDERLESS_MODAL_MAX_WIDTH: u16 = 39;
const COMPACT_MAX_WIDTH: u16 = 30;
const COMPACT_MAX_HEIGHT: u16 = 10;
const COMPACT_MIN_TRANSCRIPT_HEIGHT: u16 = 2;

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    crate::vendored::terminal_hyperlinks::clear_buffer_hyperlinks();
    app.begin_frame_render();
    let area = frame.area();
    if app.history_surface().is_some() {
        history::render_history_surface(frame, app, area);
        degrade_frames(frame, area);
        return;
    }
    let session_tabs_height = session_tabs::height(app);
    let status_height = status_indicator::height(app, area.width);
    let goal_dock_height = goal_dock::height(app);
    let footer_height = footer::desired_height(area.width);
    let composer_height = app
        .ask_questions_form()
        .map(|form| ask::desired_height(form, area.height))
        .unwrap_or_else(|| app.composer_height(area.width));
    let compact = area.width <= COMPACT_MAX_WIDTH && area.height <= COMPACT_MAX_HEIGHT;
    let events_open = app.events_pane().open && !compact;
    let main_constraints = if compact {
        compact_constraints(
            area.height,
            session_tabs_height,
            status_height,
            goal_dock_height,
            composer_height,
            footer_height,
        )
    } else if events_open {
        vec![
            Constraint::Length(1),
            Constraint::Length(session_tabs_height),
            Constraint::Percentage(62),
            Constraint::Percentage(38),
            Constraint::Length(status_height),
            Constraint::Length(goal_dock_height),
            Constraint::Length(composer_height),
            Constraint::Length(footer_height),
        ]
    } else {
        vec![
            Constraint::Length(1),
            Constraint::Length(session_tabs_height),
            Constraint::Min(1),
            Constraint::Length(status_height),
            Constraint::Length(goal_dock_height),
            Constraint::Length(composer_height),
            Constraint::Length(footer_height),
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(main_constraints)
        .split(area);

    header::render_header(frame, app, chunks[0]);
    if session_tabs_height > 0 {
        session_tabs::render(frame, app, chunks[1]);
    }
    transcript::render_transcript(frame, app, chunks[2]);
    let composer_area = if events_open { chunks[6] } else { chunks[5] };
    if events_open {
        events::render_events_pane(frame, app, chunks[3]);
        status_indicator::render(frame, app, chunks[4]);
        goal_dock::render(frame, app, chunks[5]);
        render_composer_region(frame, app, chunks[6]);
        footer::render(frame, app, chunks[7]);
    } else {
        status_indicator::render(frame, app, chunks[3]);
        goal_dock::render(frame, app, chunks[4]);
        render_composer_region(frame, app, chunks[5]);
        footer::render(frame, app, chunks[6]);
    }
    if compact {
        render_compact_truncation_indicator(frame, chunks[2]);
    }
    if matches!(app.composer_mode(), ComposerMode::ProjectPicker) {
        picker::render_project_picker(frame, app.project_picker(), area);
    }
    if let Some(picker) = app.modal_picker() {
        picker::render_modal_picker(frame, picker, area, composer_area);
    }
    if app.settings_surface_open() {
        settings::render_settings(frame, app, area);
    }
    if app.transcript_overlay().is_some() {
        app.set_transcript_overlay_visible_height(overlay::transcript_overlay_body_height(area));
    }
    if let Some(overlay) = app.transcript_overlay() {
        overlay::render_transcript_overlay(frame, overlay, area);
    }
    if let Some(board) = app.task_board_surface() {
        crate::app::surfaces::board::render_task_board(frame, board, area);
        degrade_frames(frame, area);
        if compact {
            render_compact_truncation_indicator(frame, area);
        }
        return;
    }
    if app.browser_surface().is_some() {
        let state = app.browser_state().clone();
        let browser = app.browser_surface().expect("browser surface is open");
        crate::app::surfaces::browser::render_browser_surface(frame, &state, browser, area);
        degrade_frames(frame, area);
        if compact {
            render_compact_truncation_indicator(frame, area);
        }
        return;
    }
    if app.goal_overlay_open() {
        goal_dock::render_overlay(frame, app, area);
    }
    if let Some(modal) = app.approval_modal() {
        approval::render_approval_modal(frame, modal, area);
    }
    worktree::render_worktree_merge_confirmation(frame, app, area);
    if app.help_open() {
        help::render_help(frame, app, area);
    }
    degrade_frames(frame, area);
}

fn compact_constraints(
    height: u16,
    session_tabs_height: u16,
    requested_status_height: u16,
    goal_dock_height: u16,
    requested_composer_height: u16,
    footer_height: u16,
) -> Vec<Constraint> {
    let body_height =
        height.saturating_sub(1 + session_tabs_height + footer_height + goal_dock_height);
    let composer_height =
        requested_composer_height.min(body_height.saturating_sub(COMPACT_MIN_TRANSCRIPT_HEIGHT));
    let status_height = requested_status_height
        .min(body_height.saturating_sub(composer_height + COMPACT_MIN_TRANSCRIPT_HEIGHT));
    let remaining_height = body_height.saturating_sub(composer_height + status_height);
    vec![
        Constraint::Length(1),
        Constraint::Length(session_tabs_height),
        Constraint::Length(remaining_height),
        Constraint::Length(status_height),
        Constraint::Length(goal_dock_height),
        Constraint::Length(composer_height),
        Constraint::Length(footer_height),
    ]
}

fn render_compact_truncation_indicator(frame: &mut Frame<'_>, area: Rect) {
    if area.is_empty() {
        return;
    }
    let indicator = Rect {
        x: area.x,
        y: area.y.saturating_add(area.height.saturating_sub(1)),
        width: area.width,
        height: 1,
    };
    Paragraph::new("… content truncated").render(indicator, frame.buffer_mut());
}

fn degrade_frames(frame: &mut Frame<'_>, area: Rect) {
    let buffer = frame.buffer_mut();
    if area.width <= BORDERLESS_MODAL_MAX_WIDTH {
        remove_frame_borders(buffer, area);
    }
    if area.width < ASCII_FRAME_MAX_WIDTH {
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let symbol = buffer[(x, y)].symbol();
                let replacement = match symbol {
                    "┌" | "┐" | "└" | "┘" | "├" | "┤" | "┬" | "┴" | "┼" => {
                        Some("+")
                    }
                    "─" => Some("-"),
                    "│" => Some("|"),
                    _ => None,
                };
                if let Some(replacement) = replacement {
                    buffer[(x, y)].set_symbol(replacement);
                }
            }
        }
    }
}

fn remove_frame_borders(buffer: &mut ratatui::buffer::Buffer, area: Rect) {
    let mut borders = Vec::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if buffer[(x, y)].symbol() != "┌" {
                continue;
            }
            let Some(right) =
                ((x + 1)..area.right()).find(|right| buffer[(*right, y)].symbol() == "┐")
            else {
                continue;
            };
            let Some(bottom) = ((y + 1)..area.bottom()).find(|bottom| {
                buffer[(x, *bottom)].symbol() == "└" && buffer[(right, *bottom)].symbol() == "┘"
            }) else {
                continue;
            };
            borders.push((x, y, right, bottom));
        }
    }
    for (left, top, right, bottom) in borders {
        for x in left..=right {
            buffer[(x, top)].set_symbol(" ");
            buffer[(x, bottom)].set_symbol(" ");
        }
        for y in top..=bottom {
            buffer[(left, y)].set_symbol(" ");
            buffer[(right, y)].set_symbol(" ");
        }
    }
}

fn render_composer_region(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if let Some(form) = app.ask_questions_form() {
        ask::render_ask_form(frame, form, area);
    } else {
        composer::render_composer(frame, app, area);
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn popup_anchored_above(bounds: Rect, anchor_y: u16, width: u16, height: u16) -> Rect {
    let width = width.min(bounds.width);
    let height = height.min(bounds.height);
    let max_y = bounds
        .y
        .saturating_add(bounds.height.saturating_sub(height));
    let y = anchor_y.saturating_sub(height).clamp(bounds.y, max_y);
    Rect {
        x: bounds
            .x
            .saturating_add(bounds.width.saturating_sub(width) / 2),
        y,
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
            pinned: Some(false),
            worker: None,
            cron_pending: None,
        }
    }

    fn assert_rect_inside(rect: Rect, bounds: Rect) {
        assert!(rect.x >= bounds.x);
        assert!(rect.y >= bounds.y);
        assert!(rect.x.saturating_add(rect.width) <= bounds.x.saturating_add(bounds.width));
        assert!(rect.y.saturating_add(rect.height) <= bounds.y.saturating_add(bounds.height));
    }

    #[test]
    fn popup_anchored_above_clamps_height_to_bounds() {
        let bounds = Rect::new(2, 4, 20, 5);
        let rect = popup_anchored_above(bounds, 8, 12, 99);

        assert_eq!(rect.height, bounds.height);
        assert_eq!(rect.y, bounds.y);
        assert_rect_inside(rect, bounds);
    }

    #[test]
    fn popup_anchored_above_clamps_anchor_above_bounds() {
        let bounds = Rect::new(5, 10, 30, 12);
        let rect = popup_anchored_above(bounds, 3, 10, 4);

        assert_eq!(rect.y, bounds.y);
        assert_rect_inside(rect, bounds);
    }

    #[test]
    fn popup_anchored_above_handles_empty_and_one_row_bounds() {
        let empty = Rect::new(4, 6, 0, 0);
        let empty_rect = popup_anchored_above(empty, 6, 20, 5);
        assert_eq!(empty_rect, Rect::new(4, 6, 0, 0));
        assert_rect_inside(empty_rect, empty);

        let one_row = Rect::new(4, 6, 7, 1);
        let one_row_rect = popup_anchored_above(one_row, 6, 20, 5);
        assert_eq!(one_row_rect, Rect::new(4, 6, 7, 1));
        assert_rect_inside(one_row_rect, one_row);
    }

    #[test]
    fn popup_anchored_above_uses_room_above_anchor() {
        let bounds = Rect::new(0, 0, 80, 24);
        let rect = popup_anchored_above(bounds, 20, 30, 6);

        assert_eq!(rect, Rect::new(25, 14, 30, 6));
        assert_rect_inside(rect, bounds);
    }

    #[test]
    fn help_rows_are_generated_from_active_keymap() {
        let mut app = App::new(project());
        let keymap = crate::keymap::KeymapRegistry::from_toml_str(
            r#"
[bindings]
new-chat = "ctrl-x"
help = "f1"
"#,
        )
        .unwrap();
        app.test_set_keymap(keymap);
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| help::render_help(frame, &app, frame.area()))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("show generated keymap help"));
        assert!(app
            .keymap_help_rows()
            .iter()
            .any(|row| row.action == Some(crate::keymap::KeyAction::NewChat)
                && row.bindings == "Ctrl-X"));
    }

    #[test]
    fn help_lists_every_key_context() {
        let app = App::new(project());
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| help::render_help(frame, &app, frame.area()))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        for context in crate::keymap::KeyContext::ALL {
            assert!(text.contains(context.label()), "{}", context.label());
        }
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
    fn render_busy_status_row_above_composer() {
        let mut app = App::new(project());
        app.set_native_scrollback(false);
        app.apply_chat_event(crate::client::ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "runtime_updated".to_string(),
            raw: serde_json::json!({"state": "generating"}),
        });
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let rows = terminal
            .backend()
            .buffer()
            .content()
            .chunks(80)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>());
        let rows = rows.collect::<Vec<_>>();
        let status_row = rows
            .iter()
            .position(|row| row.contains("Working") && row.contains("Esc to interrupt"))
            .unwrap();
        let composer_row = rows
            .iter()
            .position(|row| row.contains("Ask Refact"))
            .unwrap();
        assert!(status_row < composer_row);
    }

    #[test]
    fn goal_dock_is_gated_and_absent_without_a_goal() {
        let _surface_lock = goal_dock::test_surface_lock();
        let mut app = App::new(project());
        app.set_native_scrollback(false);
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!text.contains("Goal ACTIVE"));

        std::env::set_var("REFACT_TUI_SURFACES", "1");
        app.apply_chat_event(crate::client::ChatEvent {
            chat_id: Some(app.chat_id().to_string()),
            seq: None,
            kind: "snapshot".to_string(),
            raw: serde_json::json!({
                "runtime": {"state": "idle"},
                "messages": [{
                    "role": "goal",
                    "content": "Ship it",
                    "extra": {"goal": {
                        "version": 1,
                        "status": "active",
                        "active": true,
                        "progress": {"turns_used": 2, "tokens_used": 1200}
                    }}
                }]
            }),
        });
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Goal ACTIVE"));
        assert!(text.contains("2 turns"));
        std::env::remove_var("REFACT_TUI_SURFACES");
    }

    #[test]
    fn ask_form_replaces_composer_in_bottom_pane() {
        let mut app = App::new(project());
        app.set_native_scrollback(false);
        let request = crate::ask_questions::AskQuestionsRequest::from_tool_content(
            &serde_json::json!({
                "type": "ask_questions",
                "tool_call_id": "call-ask",
                "questions": [
                    {"id":"confirm","type":"yes_no","text":"Proceed?"}
                ],
            })
            .to_string(),
            None,
        )
        .unwrap();
        app.test_set_ask_questions_form(crate::ask_questions::AskQuestionsForm::new(request));
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let rows = terminal
            .backend()
            .buffer()
            .content()
            .chunks(80)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>();
        let question_row = rows
            .iter()
            .position(|row| row.contains("Proceed?"))
            .unwrap();
        let footer_row = rows.iter().rposition(|row| row.contains("daemon")).unwrap();

        assert!(!rows.join("\n").contains("Ask Refact"));
        assert!(question_row < footer_row);
        assert!(question_row >= footer_row.saturating_sub(8));
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
                .with_result("+ok", ToolStatus::Succeeded),
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
        assert!(text.contains("-old"));
        assert!(text.contains("+new"));
    }
}
