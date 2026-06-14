use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, SessionState};
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::vendored::{motion, shimmer};

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusIndicatorData {
    pub state: SessionState,
    pub elapsed_ms: u64,
    pub tick: u64,
    pub detail: Option<String>,
    pub reduced_motion: bool,
}

impl StatusIndicatorData {
    pub fn from_app(app: &App) -> Self {
        Self {
            state: app.session_state(),
            elapsed_ms: app.working_elapsed_ms(),
            tick: app.working_tick(),
            detail: app.working_detail().map(str::to_string),
            reduced_motion: motion::reduced_motion_from_env(),
        }
    }

    pub fn visible(&self) -> bool {
        self.state.shows_working_indicator()
    }
}

pub fn height(app: &App) -> u16 {
    u16::from(app.session_state().shows_working_indicator())
}

pub fn render(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let data = StatusIndicatorData::from_app(app);
    let Some(line) = status_indicator_line(&data) else {
        return;
    };
    let line = truncate_line_with_ellipsis_if_overflow(line, area.width as usize);
    frame.render_widget(Paragraph::new(line), area);
}

pub fn status_indicator_line(data: &StatusIndicatorData) -> Option<Line<'static>> {
    if !data.visible() {
        return None;
    }

    let elapsed = format_elapsed(data.elapsed_ms);
    let mut spans = Vec::new();
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        spinner(data),
        style(data, Color::Cyan).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" "));
    if data.reduced_motion {
        spans.push(Span::styled(
            "Working",
            style(data, Color::White).add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.extend(shimmer::spans(
            "Working",
            data.tick,
            style(data, Color::White),
            style(data, Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(
        format!(" {elapsed}"),
        style(data, Color::White),
    ));
    spans.push(separator(data));
    spans.push(Span::styled("Esc to interrupt", style(data, Color::Yellow)));
    if let Some(detail) = data.detail.as_deref().filter(|detail| !detail.is_empty()) {
        spans.push(separator(data));
        spans.push(Span::styled(
            detail.to_string(),
            style(data, Color::DarkGray),
        ));
    }
    Some(Line::from(spans))
}

pub fn format_elapsed(elapsed_ms: u64) -> String {
    let seconds = elapsed_ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn spinner(data: &StatusIndicatorData) -> &'static str {
    if data.reduced_motion {
        "•"
    } else {
        motion::frame(SPINNER_FRAMES, data.tick)
    }
}

fn separator(data: &StatusIndicatorData) -> Span<'static> {
    Span::styled(" · ", style(data, Color::DarkGray))
}

fn style(data: &StatusIndicatorData, color: Color) -> Style {
    if data.reduced_motion {
        Style::default()
    } else {
        Style::default().fg(color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn data(state: SessionState, reduced_motion: bool) -> StatusIndicatorData {
        StatusIndicatorData {
            state,
            elapsed_ms: 65_000,
            tick: 0,
            detail: Some("shell({\"cmd\":\"echo hi\"})".to_string()),
            reduced_motion,
        }
    }

    #[test]
    fn elapsed_formatter_is_compact() {
        assert_eq!(format_elapsed(0), "0:00");
        assert_eq!(format_elapsed(999), "0:00");
        assert_eq!(format_elapsed(65_000), "1:05");
        assert_eq!(format_elapsed(3_600_000), "60:00");
    }

    #[test]
    fn idle_is_hidden_and_busy_is_visible() {
        assert!(status_indicator_line(&data(SessionState::Idle, false)).is_none());
        assert!(status_indicator_line(&data(SessionState::ExecutingTools, false)).is_some());
    }

    #[test]
    fn animated_widget_snapshot_includes_spinner_elapsed_hint_and_detail() {
        let rendered = text(status_indicator_line(&data(SessionState::Generating, false)).unwrap());
        assert_eq!(
            rendered,
            " ⠋ Working 1:05 · Esc to interrupt · shell({\"cmd\":\"echo hi\"})"
        );
    }

    #[test]
    fn reduced_motion_widget_snapshot_is_static() {
        let mut reduced = data(SessionState::Generating, true);
        let first = text(status_indicator_line(&reduced).unwrap());
        reduced.tick = 7;
        let second = text(status_indicator_line(&reduced).unwrap());
        assert_eq!(
            first,
            " • Working 1:05 · Esc to interrupt · shell({\"cmd\":\"echo hi\"})"
        );
        assert_eq!(first, second);
    }
}
