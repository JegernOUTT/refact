use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;

use crate::app::{App, ComposerMode};

mod approval;
mod ask;
mod composer;
pub mod events;
pub mod footer;
mod header;
mod help;
pub(crate) mod menu;
mod overlay;
pub mod picker;
pub mod status_card;
pub mod status_indicator;
mod transcript;

pub fn render(frame: &mut Frame<'_>, app: &mut App) {
    app.begin_frame_render();
    let area = frame.area();
    let status_height = status_indicator::height(app, area.width);
    let footer_height = footer::desired_height(area.width);
    let main_constraints = if app.events_pane().open {
        vec![
            Constraint::Length(1),
            Constraint::Percentage(62),
            Constraint::Percentage(38),
            Constraint::Length(status_height),
            Constraint::Length(app.composer_height(area.width)),
            Constraint::Length(footer_height),
        ]
    } else {
        vec![
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(status_height),
            Constraint::Length(app.composer_height(area.width)),
            Constraint::Length(footer_height),
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(main_constraints)
        .split(area);

    header::render_header(frame, app, chunks[0]);
    transcript::render_transcript(frame, app, chunks[1]);
    let composer_area = if app.events_pane().open {
        chunks[4]
    } else {
        chunks[3]
    };
    if app.events_pane().open {
        events::render_events_pane(frame, app, chunks[2]);
        status_indicator::render(frame, app, chunks[3]);
        composer::render_composer(frame, app, composer_area);
        footer::render(frame, app, chunks[5]);
    } else {
        status_indicator::render(frame, app, chunks[2]);
        composer::render_composer(frame, app, composer_area);
        footer::render(frame, app, chunks[4]);
    }
    if matches!(app.composer_mode(), ComposerMode::ProjectPicker) {
        picker::render_project_picker(frame, app.project_picker(), area);
    }
    if let Some(picker) = app.modal_picker() {
        picker::render_modal_picker(frame, picker, area, composer_area);
    }
    if let Some(form) = app.ask_questions_form() {
        ask::render_ask_form(frame, form, area);
    }
    if app.transcript_overlay().is_some() {
        app.set_transcript_overlay_visible_height(overlay::transcript_overlay_body_height(area));
    }
    if let Some(overlay) = app.transcript_overlay() {
        overlay::render_transcript_overlay(frame, overlay, area);
    }
    if let Some(modal) = app.approval_modal() {
        approval::render_approval_modal(frame, modal, area);
    }
    if app.help_open() {
        help::render_help(frame, app, area);
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
            pinned: false,
            worker: None,
            cron_pending: None,
        }
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
        assert!(text.contains("Ctrl-X"));
        assert!(!text.contains("Ctrl-N"));
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
        assert!(text.contains("-old"));
        assert!(text.contains("+new"));
    }
}
