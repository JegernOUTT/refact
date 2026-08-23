use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph, Widget};
use ratatui::Frame;

use crate::approvals::render_modal_lines;
use crate::render::wrapping::wrap_line;
use crate::ui::menu;
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

struct ApprovalLayout {
    header: Option<Rect>,
    subject: Option<Rect>,
    body: Rect,
    footer: Option<Rect>,
}

struct ApprovalBody {
    lines: Vec<Line<'static>>,
    remaining: usize,
    scrollable: bool,
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
    let body = visible_approval_body_lines(
        &lines,
        modal,
        layout.body.width as usize,
        layout.body.height as usize,
    );
    if let Some(header) = layout.header {
        if let Some(line) = lines.first().cloned() {
            truncate_line_with_ellipsis_if_overflow(line, header.width as usize)
                .render(header, frame.buffer_mut());
        }
    }
    if let Some(subject) = layout.subject {
        if let Some(line) = lines.get(2).cloned() {
            truncate_line_with_ellipsis_if_overflow(line, subject.width as usize)
                .render(subject, frame.buffer_mut());
        }
    }
    let footer_line = layout
        .footer
        .map(|_| approval_footer_line(&lines, &body, layout.body.width as usize).dim());
    if layout.body.height > 0 && layout.body.width > 0 {
        frame.render_widget(Paragraph::new(body.lines), layout.body);
    }
    if let (Some(footer), Some(line)) = (layout.footer, footer_line) {
        line.render(footer, frame.buffer_mut());
    }
}

fn approval_layout(area: Rect) -> ApprovalLayout {
    let header_height = u16::from(area.height > 0);
    let subject_height = u16::from(area.height > header_height);
    let remaining_height = area.height.saturating_sub(header_height + subject_height);
    let footer_height = u16::from(remaining_height > 1);
    let body_height = remaining_height.saturating_sub(footer_height);
    ApprovalLayout {
        header: (header_height > 0).then_some(Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        }),
        subject: (subject_height > 0).then_some(Rect {
            x: area.x,
            y: area.y.saturating_add(header_height),
            width: area.width,
            height: 1,
        }),
        body: Rect {
            x: area.x,
            y: area.y.saturating_add(header_height + subject_height),
            width: area.width,
            height: body_height,
        },
        footer: (footer_height > 0).then_some(Rect {
            x: area.x,
            y: area
                .y
                .saturating_add(header_height + subject_height + body_height),
            width: area.width,
            height: 1,
        }),
    }
}

fn visible_approval_body_lines(
    lines: &[Line<'static>],
    modal: &crate::approvals::ApprovalModalState,
    width: usize,
    height: usize,
) -> ApprovalBody {
    let body = lines
        .get(3..)
        .unwrap_or_default()
        .iter()
        .flat_map(|line| wrap_line(line.clone(), Some(width)))
        .collect::<Vec<_>>();
    let max_start = body.len().saturating_sub(height);
    let start = modal.detail_scroll().min(max_start);
    let lines = body
        .iter()
        .skip(start)
        .take(height)
        .cloned()
        .collect::<Vec<_>>();
    let remaining = body.len().saturating_sub(start + lines.len());
    ApprovalBody {
        lines,
        remaining,
        scrollable: max_start > 0,
    }
}

fn approval_footer_line(
    lines: &[Line<'static>],
    body: &ApprovalBody,
    width: usize,
) -> Line<'static> {
    if body.scrollable {
        if body.remaining == 0 {
            return Line::from("↑/↓ scroll · end");
        }
        let full_hint = format!("… {} more · ↑/↓ scroll", body.remaining);
        if full_hint.chars().count() <= width {
            return Line::from(full_hint);
        }
        return Line::from(format!("… {} more · ↑/↓", body.remaining));
    }
    lines.get(1).cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{ApprovalModalState, PauseReason};
    use crate::render::wrapping::line_to_plain;
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

    fn long_reason() -> PauseReason {
        PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "apply_patch".to_string(),
            command: "apply patch to review-target.rs".to_string(),
            rule: "ask".to_string(),
            tool_call_id: "call-patch".to_string(),
            integr_config_path: None,
            args: Some("{\n  \"path\": \"review-target.rs\"\n}".to_string()),
            diff: Some(
                (0..40)
                    .map(|idx| format!("+ changed line {idx}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
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

    #[test]
    fn approval_subject_remains_visible_across_supported_heights() {
        let mut modal = ApprovalModalState::new(vec![long_reason()]);
        modal.toggle_details();

        for height in 6..=40 {
            let backend = TestBackend::new(100, height);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| render_approval_modal(frame, &modal, frame.area()))
                .unwrap();

            let text = buffer_text(terminal.backend().buffer());
            assert!(
                text.contains("apply_patch  apply patch to review-target.rs"),
                "subject missing at height {height}: {text:?}"
            );
        }
    }

    #[test]
    fn overflowing_approval_reports_exact_remaining_lines_and_scroll_hint() {
        let mut modal = ApprovalModalState::new(vec![long_reason()]);
        modal.toggle_details();
        let lines = render_modal_lines(&modal, 80);
        let expected_remaining = lines[3..]
            .iter()
            .flat_map(|line| wrap_line(line.clone(), Some(80)))
            .count()
            .saturating_sub(2);
        let body = visible_approval_body_lines(&lines, &modal, 80, 2);

        assert_eq!(body.remaining, expected_remaining);
        assert!(body.scrollable);
        assert_eq!(
            line_to_plain(&approval_footer_line(&lines, &body, 80)),
            format!("… {expected_remaining} more · ↑/↓ scroll")
        );
    }

    #[test]
    fn summary_scrolling_reaches_the_final_line() {
        let mut modal =
            ApprovalModalState::new(vec![reason("call-1"), reason("call-2"), reason("call-3")]);
        let lines = render_modal_lines(&modal, 80);
        let expected_last = lines[3..]
            .iter()
            .flat_map(|line| wrap_line(line.clone(), Some(80)))
            .last()
            .unwrap();

        modal.scroll_details_down(usize::MAX);
        let body = visible_approval_body_lines(&lines, &modal, 80, 1);

        assert_eq!(body.remaining, 0);
        assert_eq!(body.lines, vec![expected_last]);
        assert_eq!(
            line_to_plain(&approval_footer_line(&lines, &body, 80)),
            "↑/↓ scroll · end"
        );
    }

    #[test]
    fn detail_scrolling_reaches_the_final_line() {
        let mut modal = ApprovalModalState::new(vec![long_reason()]);
        modal.toggle_details();
        let lines = render_modal_lines(&modal, 80);
        let expected_last = lines[3..]
            .iter()
            .flat_map(|line| wrap_line(line.clone(), Some(80)))
            .last()
            .unwrap();

        modal.scroll_details_down(usize::MAX);
        let body = visible_approval_body_lines(&lines, &modal, 80, 1);

        assert_eq!(body.remaining, 0);
        assert_eq!(body.lines, vec![expected_last]);
    }

    #[test]
    fn compact_overflow_hint_fits_the_minimum_popup_width() {
        let mut modal = ApprovalModalState::new(vec![long_reason()]);
        modal.toggle_details();
        let lines = render_modal_lines(&modal, 20);
        let body = visible_approval_body_lines(&lines, &modal, 20, 1);
        let footer = line_to_plain(&approval_footer_line(&lines, &body, 20));

        assert_eq!(footer, format!("… {} more · ↑/↓", body.remaining));
        assert!(footer.chars().count() <= 20);
    }
}
