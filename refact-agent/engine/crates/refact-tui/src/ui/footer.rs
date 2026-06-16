use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, SessionState, SubscriptionStatus, UsageSummary};
use crate::client::{worker_state_label, WorkerInfo};
use crate::style::user_message_style;
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

const INTERRUPT_KEY: &str = "Esc";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FooterRuntimeState {
    Idle,
    Generating,
    Waking,
    Offline,
}

impl FooterRuntimeState {
    fn from_app(app: &App) -> Self {
        if !app.daemon_online() || app.subscription_status() == SubscriptionStatus::Offline {
            return Self::Offline;
        }
        if app.subscription_status() == SubscriptionStatus::Waking
            || worker_is_waking(app.current_worker())
        {
            return Self::Waking;
        }
        if matches!(
            app.session_state(),
            SessionState::Generating | SessionState::ExecutingTools | SessionState::Paused
        ) {
            return Self::Generating;
        }
        Self::Idle
    }

    fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Generating => "generating",
            Self::Waking => "waking",
            Self::Offline => "offline",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Idle => "●",
            Self::Generating => "◆",
            Self::Waking => "◐",
            Self::Offline => "○",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Idle => Color::Green,
            Self::Generating => Color::Cyan,
            Self::Waking => Color::Yellow,
            Self::Offline => Color::Red,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterData {
    pub project: String,
    pub model: String,
    pub mode: String,
    pub reasoning: String,
    pub runtime_state: FooterRuntimeState,
    pub worker: String,
    pub usage: Option<UsageSummary>,
    pub context_window_tokens: Option<u64>,
    pub retry_hint: Option<String>,
}

impl FooterData {
    pub fn from_app(app: &App) -> Self {
        Self {
            project: app
                .current_project()
                .map(|project| project.slug.clone())
                .unwrap_or_else(|| "-".to_string()),
            model: app.model().unwrap_or("default").to_string(),
            mode: app.mode().unwrap_or("agent").to_string(),
            reasoning: app.reasoning_effort_label().to_string(),
            runtime_state: FooterRuntimeState::from_app(app),
            worker: app
                .current_worker()
                .map(|worker| worker_state_label(Some(worker)))
                .unwrap_or_else(|| "unknown".to_string()),
            usage: app.usage(),
            context_window_tokens: app.context_window_tokens(),
            retry_hint: app.retry_hint().map(str::to_string),
        }
    }

    fn daemon_label(&self) -> &'static str {
        match self.runtime_state {
            FooterRuntimeState::Offline => "offline",
            _ => "online",
        }
    }
}

pub fn render(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let line = truncate_line_with_ellipsis_if_overflow(
        footer_line(&FooterData::from_app(app)),
        area.width as usize,
    );
    frame.render_widget(Paragraph::new(line).style(footer_surface_style()), area);
}

pub fn desired_height(_width: u16) -> u16 {
    1
}

pub fn footer_line(data: &FooterData) -> Line<'static> {
    let mut spans = Vec::new();
    spans.push(Span::raw(" "));
    if let Some(usage) = usage_label(data.usage, data.context_window_tokens) {
        spans.push(Span::styled(usage, Style::default().fg(Color::White)));
        spans.push(separator());
    }
    spans.push(Span::raw(data.project.clone()));
    spans.push(separator());
    spans.push(Span::raw(data.model.clone()));
    spans.push(separator());
    spans.push(Span::raw(data.mode.clone()));
    spans.push(separator());
    spans.push(Span::raw(format!("reason:{}", data.reasoning)));
    spans.push(separator());
    spans.extend(runtime_spans(data.runtime_state));
    spans.push(separator());
    spans.push(Span::raw(format!("daemon {}", data.daemon_label())));
    spans.push(separator());
    spans.push(Span::raw(format!("worker {}", data.worker)));
    if let Some(retry_hint) = &data.retry_hint {
        spans.push(separator());
        spans.push(Span::styled(
            retry_hint.clone(),
            Style::default().fg(Color::Yellow),
        ));
    }
    spans.push(Span::raw(" "));
    let mut line = Line::from(spans);
    line.style = dim_style();
    line
}

pub fn footer_text(data: &FooterData) -> String {
    footer_line(data)
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

pub fn usage_label(
    usage: Option<UsageSummary>,
    context_window_tokens: Option<u64>,
) -> Option<String> {
    let usage = usage?;
    let used = usage.tokens_used();
    match context_window_tokens.filter(|tokens| *tokens > 0) {
        Some(window) => Some(format!(
            "{}% context left ({} used)",
            context_left_percent(used, window),
            format_token_count(used)
        )),
        None => Some(format!("{} used", format_token_count(used))),
    }
}

pub fn format_token_count(tokens: u64) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }

    let tokens = tokens as f64;
    let (scaled, suffix) = if tokens >= 1_000_000_000_000.0 {
        (tokens / 1_000_000_000_000.0, "T")
    } else if tokens >= 1_000_000_000.0 {
        (tokens / 1_000_000_000.0, "B")
    } else if tokens >= 1_000_000.0 {
        (tokens / 1_000_000.0, "M")
    } else {
        (tokens / 1_000.0, "K")
    };

    let decimals = if scaled < 10.0 {
        2
    } else if scaled < 100.0 {
        1
    } else {
        0
    };
    let mut formatted = format!("{scaled:.decimals$}");
    if formatted.contains('.') {
        while formatted.ends_with('0') {
            formatted.pop();
        }
        if formatted.ends_with('.') {
            formatted.pop();
        }
    }
    format!("{formatted}{suffix}")
}

fn context_left_percent(used: u64, window: u64) -> u64 {
    if window == 0 {
        return 0;
    }
    let remaining = window.saturating_sub(used);
    (((remaining as u128 * 100) + (window as u128 / 2)) / window as u128) as u64
}

fn runtime_spans(state: FooterRuntimeState) -> Vec<Span<'static>> {
    let mut spans = vec![
        Span::styled(state.icon(), Style::default().fg(state.color())),
        Span::raw(" "),
        Span::styled(state.label(), Style::default().fg(state.color())),
    ];
    if state == FooterRuntimeState::Generating {
        spans.push(separator());
        spans.push(Span::styled(
            INTERRUPT_KEY,
            Style::default().fg(Color::Yellow),
        ));
        spans.push(Span::raw(" to interrupt"));
    }
    spans
}

fn separator() -> Span<'static> {
    Span::styled(" · ", dim_style())
}

fn dim_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn footer_surface_style() -> Style {
    user_message_style()
}

fn worker_is_waking(worker: Option<&WorkerInfo>) -> bool {
    worker
        .map(|worker| worker_state_label(Some(worker)).to_ascii_lowercase())
        .is_some_and(|state| state == "starting")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(runtime_state: FooterRuntimeState) -> FooterData {
        FooterData {
            project: "demo".to_string(),
            model: "model".to_string(),
            mode: "agent".to_string(),
            reasoning: "off".to_string(),
            runtime_state,
            worker: "ready".to_string(),
            usage: Some(UsageSummary {
                prompt_tokens: 10,
                completion_tokens: 0,
                total_tokens: 10,
            }),
            context_window_tokens: Some(100),
            retry_hint: None,
        }
    }

    #[test]
    fn usage_math_formats_context_left_and_compact_tokens() {
        let usage = Some(UsageSummary {
            prompt_tokens: 12_000,
            completion_tokens: 345,
            total_tokens: 12_345,
        });

        assert_eq!(
            usage_label(usage, Some(100_000)).as_deref(),
            Some("88% context left (12.3K used)")
        );
        assert_eq!(format_token_count(999), "999");
        assert_eq!(format_token_count(1_000), "1K");
        assert_eq!(format_token_count(1_550), "1.55K");
        assert_eq!(format_token_count(12_345), "12.3K");
        assert_eq!(format_token_count(123_456), "123K");
        assert_eq!(format_token_count(1_200_000), "1.2M");
        assert_eq!(format_token_count(1_234_567_890), "1.23B");
        assert_eq!(format_token_count(1_234_567_890_123), "1.23T");
    }

    #[test]
    fn footer_uses_one_soft_dimmed_surface_row() {
        let line = footer_line(&data(FooterRuntimeState::Idle));

        assert_eq!(desired_height(80), 1);
        assert_eq!(footer_surface_style(), crate::style::user_message_style());
        assert!(line.style.add_modifier.contains(Modifier::DIM));
        assert!(line
            .spans
            .iter()
            .all(|span| span.style.fg != Some(Color::DarkGray)));
    }

    #[test]
    fn footer_line_truncates_to_one_terminal_row() {
        let line = truncate_line_with_ellipsis_if_overflow(
            footer_line(&data(FooterRuntimeState::Idle)),
            24,
        );

        assert!(crate::vendored::line_truncation::line_width(&line) <= 24);
        assert!(line
            .spans
            .last()
            .is_some_and(|span| span.content.as_ref() == "…"));
    }

    #[test]
    fn state_transition_footer_snapshot_is_visually_distinct() {
        let snapshot = [
            FooterRuntimeState::Idle,
            FooterRuntimeState::Generating,
            FooterRuntimeState::Waking,
            FooterRuntimeState::Offline,
        ]
        .into_iter()
        .map(|state| footer_text(&data(state)))
        .collect::<Vec<_>>()
        .join("\n");

        assert_eq!(
            snapshot,
            " 90% context left (10 used) · demo · model · agent · reason:off · ● idle · daemon online · worker ready \n 90% context left (10 used) · demo · model · agent · reason:off · ◆ generating · Esc to interrupt · daemon online · worker ready \n 90% context left (10 used) · demo · model · agent · reason:off · ◐ waking · daemon online · worker ready \n 90% context left (10 used) · demo · model · agent · reason:off · ○ offline · daemon offline · worker ready "
        );

        let colors = [
            FooterRuntimeState::Idle,
            FooterRuntimeState::Generating,
            FooterRuntimeState::Waking,
            FooterRuntimeState::Offline,
        ]
        .into_iter()
        .map(|state| runtime_spans(state)[0].style.fg)
        .collect::<Vec<_>>();
        assert_eq!(
            colors,
            vec![
                Some(Color::Green),
                Some(Color::Cyan),
                Some(Color::Yellow),
                Some(Color::Red)
            ]
        );
    }

    #[test]
    fn generating_runtime_segment_matches_interrupt_hint_shape() {
        let spans = runtime_spans(FooterRuntimeState::Generating);
        let rendered = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(
            rendered,
            vec!["◆", " ", "generating", " · ", "Esc", " to interrupt"]
        );
    }
}
