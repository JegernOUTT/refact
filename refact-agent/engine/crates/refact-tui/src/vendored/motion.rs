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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn content<'a>(span: &'a Span<'static>) -> &'a str {
        span.content.as_ref()
    }

    #[test]
    fn frame_wraps() {
        assert_eq!(frame(&["a", "b", "c"], 4), "b");
        assert_eq!(frame(&[], 4), "");
    }

    #[test]
    fn motion_mode_follows_animation_flag() {
        assert_eq!(
            MotionMode::from_animations_enabled(true),
            MotionMode::Animated
        );
        assert_eq!(
            MotionMode::from_animations_enabled(false),
            MotionMode::Reduced
        );
    }

    #[test]
    fn reduced_motion_activity_indicator_uses_explicit_fallback() {
        assert_eq!(
            activity_indicator(None, MotionMode::Reduced, ReducedMotionIndicator::Hidden),
            None
        );
        let bullet = activity_indicator(
            None,
            MotionMode::Reduced,
            ReducedMotionIndicator::StaticBullet,
        )
        .unwrap();
        assert_eq!(content(&bullet), "•");
    }

    #[test]
    fn animated_activity_indicator_uses_frame_primitive() {
        let start = Instant::now() - Duration::from_millis(400);
        let indicator = activity_indicator(
            Some(start),
            MotionMode::Animated,
            ReducedMotionIndicator::Hidden,
        )
        .unwrap();
        assert!(ACTIVITY_FRAMES.contains(&content(&indicator)));
        assert_eq!(indicator.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn reduced_motion_shimmer_text_is_plain_text() {
        assert_eq!(
            shimmer_text("Loading", MotionMode::Reduced),
            vec![Span::raw("Loading")]
        );
        assert_eq!(
            shimmer_text("", MotionMode::Reduced),
            Vec::<Span<'static>>::new()
        );
    }

    #[test]
    fn animated_shimmer_text_delegates_to_shimmer_spans() {
        let spans = shimmer_text("abc", MotionMode::Animated);
        assert_eq!(spans.len(), 3);
        assert_eq!(
            spans
                .iter()
                .filter(|span| span.style.fg == Some(Color::Cyan))
                .count(),
            1
        );
    }
}
