// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use ratatui::style::{Color, Modifier, Style};

use crate::terminal_palette;

const LIGHT_BG_ACCENT_RGB: (u8, u8, u8) = (0, 95, 135);
const TABLE_SEPARATOR_FG_ALPHA: f32 = 0.20;

pub fn user_message_style() -> Style {
    user_message_style_for(default_terminal_bg())
}

pub fn proposed_plan_style() -> Style {
    proposed_plan_style_for(default_terminal_bg())
}

pub(crate) fn table_separator_style() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

pub(crate) fn accent_style() -> Style {
    accent_style_for(default_terminal_bg())
}

pub fn user_message_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    match terminal_bg {
        Some(bg) => Style::default().bg(user_message_bg(bg)),
        None => Style::default(),
    }
}

pub fn proposed_plan_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    match terminal_bg {
        Some(bg) => Style::default().bg(proposed_plan_bg(bg)),
        None => Style::default(),
    }
}

pub(crate) fn accent_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    let fg = if terminal_bg.is_some_and(is_light) {
        best_color(LIGHT_BG_ACCENT_RGB)
    } else {
        Color::Cyan
    };
    Style::default().fg(fg).add_modifier(Modifier::BOLD)
}

pub fn user_message_bg(terminal_bg: (u8, u8, u8)) -> Color {
    let (top, alpha) = if is_light(terminal_bg) {
        ((0, 0, 0), 0.04)
    } else {
        ((255, 255, 255), 0.12)
    };
    best_color(blend(top, terminal_bg, alpha))
}

pub fn proposed_plan_bg(terminal_bg: (u8, u8, u8)) -> Color {
    user_message_bg(terminal_bg)
}

pub(crate) fn default_terminal_bg() -> Option<(u8, u8, u8)> {
    if !terminal_background_color_enabled() {
        return None;
    }
    terminal_palette::default_bg()
}

fn terminal_background_color_enabled() -> bool {
    #[cfg(test)]
    if let Some(enabled) = color_enabled_override_for_test() {
        return enabled;
    }

    terminal_background_color_enabled_for_env(
        std::env::var_os("NO_COLOR").is_some(),
        std::env::var("TERM").ok().as_deref(),
    )
}

fn terminal_background_color_enabled_for_env(no_color: bool, term: Option<&str>) -> bool {
    !no_color && term.map(|term| term != "dumb").unwrap_or(true)
}

#[cfg(test)]
std::thread_local! {
    static COLOR_ENABLED_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn color_enabled_override_for_test() -> Option<bool> {
    COLOR_ENABLED_OVERRIDE.with(|override_value| override_value.get())
}

#[cfg(test)]
struct ColorEnabledOverrideGuard {
    previous: Option<bool>,
}

#[cfg(test)]
impl Drop for ColorEnabledOverrideGuard {
    fn drop(&mut self) {
        COLOR_ENABLED_OVERRIDE.with(|override_value| override_value.set(self.previous));
    }
}

#[cfg(test)]
fn override_color_enabled_for_test(enabled: bool) -> ColorEnabledOverrideGuard {
    let previous =
        COLOR_ENABLED_OVERRIDE.with(|override_value| override_value.replace(Some(enabled)));
    ColorEnabledOverrideGuard { previous }
}

fn table_separator_style_for(
    terminal_fg: Option<(u8, u8, u8)>,
    terminal_bg: Option<(u8, u8, u8)>,
) -> Style {
    let (Some(fg), Some(bg)) = (terminal_fg, terminal_bg) else {
        return table_separator_style();
    };
    Style::default().fg(best_color(blend(fg, bg, TABLE_SEPARATOR_FG_ALPHA)))
}

fn is_light(bg: (u8, u8, u8)) -> bool {
    let (red, green, blue) = bg;
    let luminance = 0.299 * red as f32 + 0.587 * green as f32 + 0.114 * blue as f32;
    luminance > 128.0
}

fn blend(fg: (u8, u8, u8), bg: (u8, u8, u8), alpha: f32) -> (u8, u8, u8) {
    let red = (fg.0 as f32 * alpha + bg.0 as f32 * (1.0 - alpha)) as u8;
    let green = (fg.1 as f32 * alpha + bg.1 as f32 * (1.0 - alpha)) as u8;
    let blue = (fg.2 as f32 * alpha + bg.2 as f32 * (1.0 - alpha)) as u8;
    (red, green, blue)
}

fn best_color((red, green, blue): (u8, u8, u8)) -> Color {
    Color::Rgb(red, green, blue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_style_is_bold() {
        let style = accent_style();

        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn default_terminal_bg_uses_probe_when_color_enabled() {
        let _color = override_color_enabled_for_test(true);
        let _bg = terminal_palette::override_default_bg_for_test(Some((250, 250, 250)));

        assert_eq!(default_terminal_bg(), Some((250, 250, 250)));
        assert_eq!(accent_style().fg, Some(Color::Rgb(0, 95, 135)));
        assert_eq!(user_message_style().bg, Some(Color::Rgb(240, 240, 240)));
    }

    #[test]
    fn default_terminal_bg_falls_back_when_probe_unavailable() {
        let _color = override_color_enabled_for_test(true);
        let _bg = terminal_palette::override_default_bg_for_test(None);

        assert_eq!(default_terminal_bg(), None);
        assert_eq!(accent_style().fg, Some(Color::Cyan));
        assert_eq!(user_message_style(), Style::default());
    }

    #[test]
    fn default_terminal_bg_respects_disabled_color() {
        let _color = override_color_enabled_for_test(false);
        let _bg = terminal_palette::override_default_bg_for_test(Some((250, 250, 250)));

        assert_eq!(default_terminal_bg(), None);
        assert_eq!(accent_style().fg, Some(Color::Cyan));
        assert_eq!(user_message_style(), Style::default());
    }

    #[test]
    fn terminal_background_color_enabled_follows_env_contract() {
        assert!(!terminal_background_color_enabled_for_env(
            true,
            Some("xterm-256color")
        ));
        assert!(!terminal_background_color_enabled_for_env(
            false,
            Some("dumb")
        ));
        assert!(terminal_background_color_enabled_for_env(
            false,
            Some("xterm-256color")
        ));
        assert!(terminal_background_color_enabled_for_env(false, None));
    }

    #[test]
    fn user_message_style_returns_a_style() {
        let style = user_message_style();

        assert_eq!(style, Style::default());
    }

    #[test]
    fn user_message_style_uses_background_when_available() {
        let style = user_message_style_for(Some((0, 0, 0)));

        assert_eq!(style.bg, Some(Color::Rgb(30, 30, 30)));
    }

    #[test]
    fn proposed_plan_bg_matches_user_message_bg() {
        let terminal_bg = (255, 255, 255);

        assert_eq!(proposed_plan_bg(terminal_bg), user_message_bg(terminal_bg));
    }

    #[test]
    fn table_separator_style_dims_without_terminal_colors() {
        let style = table_separator_style_for(None, Some((0, 0, 0)));

        assert!(style.add_modifier.contains(Modifier::DIM));
    }
}
