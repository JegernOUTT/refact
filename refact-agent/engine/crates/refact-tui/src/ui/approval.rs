use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph, Widget, Wrap};
use ratatui::Frame;

use crate::approvals::render_modal_lines;
use crate::ui::menu;

struct ApprovalLayout {
    header: Option<Rect>,
    body: Rect,
    footer: Option<Rect>,
}

pub(crate) fn render_approval_modal(
    frame: &mut Frame<'_>,
    modal: &crate::approvals::ApprovalModalState,
    area: Rect,
) {
    if area.is_empty() {
        return;
    }
    let width = area.width.saturating_sub(6).min(96).max(24).min(area.width);
    let max_height = if modal.details_open() { 28 } else { 16 };
    let height = area
        .height
        .saturating_sub(6)
        .min(max_height)
        .max(8)
        .min(area.height);
    let popup = super::centered(area, width, height);
    frame.render_widget(Clear, popup);
    let inner = menu::render_menu_surface(popup, frame.buffer_mut());
    render_approval_content(frame, modal, inner);
}

fn render_approval_content(
    frame: &mut Frame<'_>,
    modal: &crate::approvals::ApprovalModalState,
    area: Rect,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let lines = render_modal_lines(modal, area.width as usize);
    let layout = approval_layout(area);
    if let Some(header) = layout.header {
        if let Some(line) = lines.first().cloned() {
            line.render(header, frame.buffer_mut());
        }
    }
    if layout.body.height > 0 && layout.body.width > 0 {
        frame.render_widget(
            Paragraph::new(visible_approval_body_lines(
                &lines,
                modal,
                layout.body.height as usize,
            ))
            .wrap(Wrap { trim: false }),
            layout.body,
        );
    }
    if let Some(footer) = layout.footer {
        if let Some(line) = lines.get(1).cloned() {
            line.dim().render(footer, frame.buffer_mut());
        }
    }
}

fn approval_layout(area: Rect) -> ApprovalLayout {
    let footer_height = u16::from(area.height > 1);
    let header_height = u16::from(area.height > footer_height);
    let body_height = area.height.saturating_sub(header_height + footer_height);
    ApprovalLayout {
        header: (header_height > 0).then_some(Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        }),
        body: Rect {
            x: area.x,
            y: area.y.saturating_add(header_height),
            width: area.width,
            height: body_height,
        },
        footer: (footer_height > 0).then_some(Rect {
            x: area.x,
            y: area.y.saturating_add(area.height.saturating_sub(1)),
            width: area.width,
            height: 1,
        }),
    }
}

fn visible_approval_body_lines(
    lines: &[Line<'static>],
    modal: &crate::approvals::ApprovalModalState,
    height: usize,
) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    let body = lines.get(2..).unwrap_or_default();
    let start = if modal.details_open() {
        modal.detail_scroll().min(body.len())
    } else {
        0
    };
    body.iter().skip(start).take(height).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{ApprovalModalState, PauseReason};
    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Modifier};
    use ratatui::Terminal;

    fn reason(id: &str) -> PauseReason {
        PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "shell".to_string(),
            command: format!("echo {id}"),
            rule: "ask".to_string(),
            tool_call_id: id.to_string(),
            integr_config_path: None,
            args: None,
            diff: None,
        }
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn approval_modal_renders_deboxed_surface_footer_and_accent_row() {
        let modal = ApprovalModalState::new(vec![reason("call-1")]);
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_approval_modal(frame, &modal, frame.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text = buffer_text(buffer);
        assert!(text.contains("Approval required"));
        assert!(text.contains("y approve"));
        assert!(text.contains("a approve for chat"));
        assert!(text.contains("v details"));
        assert!(!text.contains("┌"));
        assert!(!text.contains("┐"));
        assert!(!text.contains("└"));
        assert!(!text.contains("┘"));
        assert!(!text.contains("│"));
        let cursor = buffer
            .content()
            .iter()
            .find(|cell| cell.symbol() == "›")
            .expect("selected approval row rendered");
        assert_eq!(cursor.style().fg, Some(Color::Cyan));
        assert!(cursor.style().add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn approval_detail_modal_keeps_diff_and_summary_footer() {
        let mut modal = ApprovalModalState::from_event(&serde_json::json!({
            "reasons": [{
                "type": "confirmation",
                "tool_name": "apply_patch",
                "command": "apply patch",
                "rule": "ask",
                "tool_call_id": "call-patch",
                "diff": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new"
            }]
        }))
        .unwrap();
        modal.toggle_details();
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_approval_modal(frame, &modal, frame.area()))
            .unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("apply_patch"));
        assert!(text.contains("-old"));
        assert!(text.contains("+new"));
        assert!(text.contains("v summary"));
        assert!(!text.contains("┌"));
    }
}
