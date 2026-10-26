// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use std::sync::OnceLock;
use std::time::Instant;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use super::shimmer;

const ACTIVITY_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
static SHIMMER_START: OnceLock<Instant> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionMode {
    Animated,
    Reduced,
}

impl MotionMode {
    pub fn from_animations_enabled(animations_enabled: bool) -> Self {
        if animations_enabled {
            Self::Animated
        } else {
            Self::Reduced
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReducedMotionIndicator {
    Hidden,
    StaticBullet,
}

pub fn reduced_motion_from_env() -> bool {
    std::env::var_os("REFACT_TUI_REDUCED_MOTION").is_some()
        || std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM")
            .map(|term| term == "dumb")
            .unwrap_or(false)
}

pub fn frame<'a>(frames: &'a [&'a str], tick: u64) -> &'a str {
    if frames.is_empty() {
        return "";
    }
    frames[tick as usize % frames.len()]
}

pub fn activity_indicator(
    start: Option<Instant>,
    mode: MotionMode,
    indicator: ReducedMotionIndicator,
) -> Option<Span<'static>> {
    match mode {
        MotionMode::Animated => Some(Span::styled(
            frame(ACTIVITY_FRAMES, activity_tick(start)),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        MotionMode::Reduced => match indicator {
            ReducedMotionIndicator::Hidden => None,
            ReducedMotionIndicator::StaticBullet => Some(Span::raw("•")),
        },
    }
}

pub fn shimmer_text(text: &str, mode: MotionMode) -> Vec<Span<'static>> {
    match mode {
        MotionMode::Animated => shimmer::spans(
            text,
            shimmer_tick(),
            Style::default().fg(Color::White),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        MotionMode::Reduced => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![Span::raw(text.to_string())]
            }
        }
    }
}

fn activity_tick(start: Option<Instant>) -> u64 {
    start
        .map(|start| start.elapsed().as_millis() as u64 / 100)
        .unwrap_or_default()
}

fn shimmer_tick() -> u64 {
    let start = SHIMMER_START.get_or_init(Instant::now);
    start.elapsed().as_millis() as u64 / 100
}
