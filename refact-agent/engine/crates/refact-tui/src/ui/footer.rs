use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, SessionState, SubscriptionStatus, UsageSummary};
use crate::client::{worker_state_label, WorkerInfo};
use crate::keymap::{KeyAction, KeyContext, KeymapRegistry};
use crate::style::user_message_style;
use crate::text_formatting::format_tokens_compact;
use crate::vendored::line_truncation::truncate_line_with_ellipsis_if_overflow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FooterRuntimeState {
    Idle,
    Generating,
    ExecutingTools,
    Paused,
    WaitingIde,
    WaitingUserInput,
    Completed,
    Error,
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
        match app.session_state() {
            SessionState::Idle => Self::Idle,
            SessionState::Generating => Self::Generating,
            SessionState::ExecutingTools => Self::ExecutingTools,
            SessionState::Paused => Self::Paused,
            SessionState::WaitingIde => Self::WaitingIde,
            SessionState::WaitingUserInput => Self::WaitingUserInput,
            SessionState::Completed => Self::Completed,
            SessionState::Error => Self::Error,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Generating => "generating",
            Self::ExecutingTools => "running tools",
            Self::Paused => "approval pending",
            Self::WaitingIde => "waiting for IDE…",
            Self::WaitingUserInput => "waiting for input",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Waking => "waking",
            Self::Offline => "offline",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Idle => "●",
            Self::Generating => "◆",
            Self::ExecutingTools => "◆",
            Self::Paused => "◆",
            Self::WaitingIde => "◆",
            Self::WaitingUserInput => "◆",
            Self::Completed => "●",
            Self::Error => "●",
            Self::Waking => "◐",
            Self::Offline => "○",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Idle => Color::Green,
            Self::Generating => Color::Cyan,
            Self::ExecutingTools => Color::Cyan,
            Self::Paused => Color::Yellow,
            Self::WaitingIde => Color::Yellow,
            Self::WaitingUserInput => Color::Yellow,
            Self::Completed => Color::Green,
            Self::Error => Color::Red,
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
    pub interrupt_key: String,
    pub retry_key: String,
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
            interrupt_key: key_label(app.keymap(), KeyContext::Main, KeyAction::Cancel, "Esc"),
            retry_key: key_label(
                app.keymap(),
                KeyContext::Main,
                KeyAction::RetrySubscription,
                "Ctrl-Shift-R",
            ),
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
    spans.extend(runtime_spans(
        data.runtime_state,
        &data.interrupt_key,
        &data.retry_key,
    ));
    spans.push(separator());
    spans.push(Span::raw(format!("daemon {}", data.daemon_label())));
    spans.push(separator());
    spans.push(Span::raw(data.project.clone()));
    spans.push(separator());
    spans.push(Span::raw(data.model.clone()));
    spans.push(separator());
    spans.push(Span::raw(data.mode.clone()));
    spans.push(separator());
    spans.push(Span::raw(format!("reason:{}", data.reasoning)));
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
    let Some(used) = usage.tokens_used() else {
        return match (usage.prompt_tokens, usage.completion_tokens) {
            (Some(prompt), Some(completion)) => {
                Some(format!("{prompt} input · {completion} output reported"))
            }
            (Some(prompt), None) => Some(format!("{prompt} input reported")),
            (None, Some(completion)) => Some(format!("{completion} output reported")),
            (None, None) => None,
        };
    };
    match context_window_tokens.filter(|tokens| *tokens > 0) {
        Some(window) => Some(format!(
            "{}% context left ({} used)",
            context_left_percent(used, window),
            format_tokens_compact(used)
        )),
        None => Some(format!("{} used", format_tokens_compact(used))),
    }
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

fn context_left_percent(used: u64, window: u64) -> u64 {
    if window == 0 {
        return 0;
    }
    let remaining = window.saturating_sub(used);
    (((remaining as u128 * 100) + (window as u128 / 2)) / window as u128) as u64
}

fn runtime_spans(
    state: FooterRuntimeState,
    interrupt_key: &str,
    retry_key: &str,
) -> Vec<Span<'static>> {
    let mut spans = vec![
        Span::styled(state.icon(), Style::default().fg(state.color())),
        Span::raw(" "),
        Span::styled(state.label(), Style::default().fg(state.color())),
    ];
    let escape_hint = match state {
        FooterRuntimeState::WaitingIde => Some(" to abort"),
        FooterRuntimeState::Generating
        | FooterRuntimeState::ExecutingTools
        | FooterRuntimeState::Paused
        | FooterRuntimeState::WaitingUserInput => Some(" to interrupt"),
        _ => None,
    };
    if let Some(escape_hint) = escape_hint {
        spans.push(separator());
        spans.push(Span::styled(
            interrupt_key.to_string(),
            Style::default().fg(Color::Yellow),
        ));
        spans.push(Span::raw(escape_hint));
    } else if state == FooterRuntimeState::Offline {
        spans.push(separator());
        spans.push(Span::styled(
            retry_key.to_string(),
            Style::default().fg(Color::Yellow),
        ));
        spans.push(Span::raw(" to retry"));
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
