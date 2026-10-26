use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, SessionState};
use crate::keymap::{KeyAction, KeyContext, KeymapRegistry};
use crate::render::wrapping::{line_width, RtOptions, word_wrap_lines};
use crate::style::user_message_style;
use crate::vendored::line_truncation::{
    truncate_line_to_width, truncate_line_with_ellipsis_if_overflow,
};
use crate::vendored::{motion, shimmer};

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const STATUS_DETAILS_DEFAULT_MAX_LINES: usize = 3;
const DETAILS_PREFIX: &str = "  └ ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusIndicatorData {
    pub state: SessionState,
    pub elapsed_ms: u64,
    pub tick: u64,
    pub detail: Option<String>,
    pub reduced_motion: bool,
    pub interrupt_key: String,
}

impl StatusIndicatorData {
    pub fn from_app(app: &App) -> Self {
        Self {
            state: app.session_state(),
            elapsed_ms: app.working_elapsed_ms(),
            tick: app.working_tick(),
            detail: app.working_detail().map(str::to_string),
            reduced_motion: motion::reduced_motion_from_env(),
            interrupt_key: key_label(app.keymap(), KeyContext::Main, KeyAction::Cancel, "Esc"),
        }
    }

    pub fn visible(&self) -> bool {
        self.state.shows_working_indicator()
    }
}

pub fn height(app: &App, width: u16) -> u16 {
    let data = StatusIndicatorData::from_app(app);
    status_indicator_lines(&data, width)
        .map(|lines| u16::try_from(lines.len()).unwrap_or(u16::MAX))
        .unwrap_or(0)
}

pub fn render(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let data = StatusIndicatorData::from_app(app);
    let Some(lines) = status_indicator_lines(&data, area.width) else {
        return;
    };
    let lines = lines
        .into_iter()
        .take(usize::from(area.height))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(user_message_style()),
        area,
    );
}

pub fn status_indicator_line(data: &StatusIndicatorData) -> Option<Line<'static>> {
    status_indicator_lines(data, u16::MAX).and_then(|mut lines| lines.drain(..1).next())
}

pub fn status_indicator_lines(
    data: &StatusIndicatorData,
    width: u16,
) -> Option<Vec<Line<'static>>> {
    if !data.visible() {
        return None;
    }

    let mut lines = vec![status_header_line(data, width)];
    lines.extend(status_detail_lines(data, width));
    Some(lines)
}

fn status_header_line(data: &StatusIndicatorData, width: u16) -> Line<'static> {
    let elapsed = format_elapsed(data.elapsed_ms);
    let mut spans = Vec::new();
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        spinner(data),
        indicator_style(data, Color::Cyan).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" "));
    if data.reduced_motion {
        spans.push(Span::styled(
            "Working",
            indicator_style(data, Color::White).add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.extend(shimmer::spans(
            "Working",
            data.tick,
            indicator_style(data, Color::White),
            indicator_style(data, Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(
        format!(" ({elapsed} • "),
        indicator_style(data, Color::DarkGray),
    ));
    spans.push(Span::styled(
        format!("{} to interrupt", data.interrupt_key),
        indicator_style(data, Color::Yellow),
    ));
    spans.push(Span::styled(")", indicator_style(data, Color::DarkGray)));
    truncate_line_with_ellipsis_if_overflow(Line::from(spans), usize::from(width))
}

fn key_label(
    keymap: &KeymapRegistry,
    context: KeyContext,
    action: KeyAction,
    fallback: &str,
) -> String {
    keymap
        .binding_label(context, action)
        .and_then(|label| label.split('/').next().map(str::to_string))
        .unwrap_or_else(|| fallback.to_string())
}

pub fn fmt_elapsed_compact(elapsed_secs: u64) -> String {
    if elapsed_secs < 60 {
        return format!("{elapsed_secs}s");
    }
    if elapsed_secs < 3600 {
        let minutes = elapsed_secs / 60;
        let seconds = elapsed_secs % 60;
        return format!("{minutes}m {seconds:02}s");
    }
    let hours = elapsed_secs / 3600;
    let minutes = (elapsed_secs % 3600) / 60;
    let seconds = elapsed_secs % 60;
    format!("{hours}h {minutes:02}m {seconds:02}s")
}

pub fn format_elapsed(elapsed_ms: u64) -> String {
    fmt_elapsed_compact(elapsed_ms / 1000)
}

fn status_detail_lines(data: &StatusIndicatorData, width: u16) -> Vec<Line<'static>> {
    let Some(detail) = data
        .detail
        .as_deref()
        .map(str::trim)
        .filter(|detail| !detail.is_empty())
    else {
        return Vec::new();
    };
    if width == 0 {
        return Vec::new();
    }

    let detail_style = detail_style(data);
    let prefix_width = line_width(&Line::from(DETAILS_PREFIX));
    let opts = RtOptions::new(usize::from(width))
        .initial_indent(Line::from(Span::styled(DETAILS_PREFIX, detail_style)))
        .subsequent_indent(Line::from(Span::styled(
            " ".repeat(prefix_width),
            detail_style,
        )))
        .break_words(true);
    let mut lines = word_wrap_lines(
        detail
            .lines()
            .map(|line| vec![Span::styled(line.to_string(), detail_style)]),
        opts,
    )
    .into_iter()
    .map(|line| truncate_line_with_ellipsis_if_overflow(line, usize::from(width)))
    .collect::<Vec<_>>();

    if lines.len() > STATUS_DETAILS_DEFAULT_MAX_LINES {
        lines.truncate(STATUS_DETAILS_DEFAULT_MAX_LINES);
        if let Some(last) = lines.pop() {
            lines.push(force_ellipsis(last, usize::from(width)));
        }
    }

    lines
}

fn force_ellipsis(line: Line<'static>, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }
    let Line {
        style,
        alignment,
        mut spans,
    } = truncate_line_to_width(line, width.saturating_sub(1));
    let ellipsis_style = spans.last().map(|span| span.style).unwrap_or(style);
    spans.push(Span::styled("…", ellipsis_style));
    Line {
        style,
        alignment,
        spans,
    }
}

fn spinner(data: &StatusIndicatorData) -> &'static str {
    if data.reduced_motion {
        "•"
    } else {
        motion::frame(SPINNER_FRAMES, data.tick)
    }
}

fn detail_style(data: &StatusIndicatorData) -> Style {
    indicator_style(data, Color::DarkGray).add_modifier(Modifier::DIM)
}

fn indicator_style(data: &StatusIndicatorData, color: Color) -> Style {
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

    fn line_texts(lines: Vec<Line<'static>>) -> Vec<String> {
        lines.into_iter().map(text).collect()
    }

    fn data(state: SessionState, reduced_motion: bool) -> StatusIndicatorData {
        StatusIndicatorData {
            state,
            elapsed_ms: 65_000,
            tick: 0,
            detail: Some("shell({\"cmd\":\"echo hi\"})".to_string()),
            reduced_motion,
            interrupt_key: "Esc".to_string(),
        }
    }

    #[test]
    fn reduced_motion_widget_snapshot_is_static() {
        let mut reduced = data(SessionState::Generating, true);
        let first = line_texts(status_indicator_lines(&reduced, 80).unwrap());
        reduced.tick = 7;
        let second = line_texts(status_indicator_lines(&reduced, 80).unwrap());
        assert_eq!(
            first,
            vec![
                " • Working (1m 05s • Esc to interrupt)",
                "  └ shell({\"cmd\":\"echo hi\"})",
            ]
        );
        assert_eq!(first, second);
    }
}
