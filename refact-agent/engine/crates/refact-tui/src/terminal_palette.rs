use crate::color::perceptual_distance;
use ratatui::style::Color;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StdoutColorLevel {
    TrueColor,
    Ansi256,
    Ansi16,
    Unknown,
}

pub fn stdout_color_level() -> StdoutColorLevel {
    match supports_color::on_cached(supports_color::Stream::Stdout) {
        Some(level) if level.has_16m => StdoutColorLevel::TrueColor,
        Some(level) if level.has_256 => StdoutColorLevel::Ansi256,
        Some(_) => StdoutColorLevel::Ansi16,
        None => StdoutColorLevel::Unknown,
    }
}

pub fn rgb_color((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

pub fn indexed_color(index: u8) -> Color {
    Color::Indexed(index)
}

pub fn best_color(target: (u8, u8, u8)) -> Color {
    best_color_for_color_level(target, effective_stdout_color_level())
}

pub fn best_color_for_level(target: (u8, u8, u8), color_level: StdoutColorLevel) -> Color {
    best_color_for_color_level(target, color_level)
}

fn effective_stdout_color_level() -> StdoutColorLevel {
    stdout_color_level_for_terminal(stdout_color_level(), TerminalColorEnv::from_env())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TerminalColorEnv {
    term_mentions_truecolor: bool,
    colorterm_mentions_truecolor: bool,
    term_program_is_windows_terminal: bool,
    has_wt_session: bool,
    has_force_color_override: bool,
}

impl TerminalColorEnv {
    fn from_env() -> Self {
        Self::from_values(
            std::env::var_os("TERM").as_deref(),
            std::env::var_os("WT_SESSION").as_deref(),
            std::env::var_os("COLORTERM").as_deref(),
            std::env::var_os("TERM_PROGRAM").as_deref(),
            std::env::var_os("FORCE_COLOR").as_deref(),
        )
    }

    fn from_values(
        term: Option<&std::ffi::OsStr>,
        wt_session: Option<&std::ffi::OsStr>,
        colorterm: Option<&std::ffi::OsStr>,
        term_program: Option<&std::ffi::OsStr>,
        force_color: Option<&std::ffi::OsStr>,
    ) -> Self {
        let term = term.and_then(|value| value.to_str()).unwrap_or_default();
        let colorterm = colorterm
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let term_program = term_program
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        Self {
            term_mentions_truecolor: mentions_truecolor(term),
            colorterm_mentions_truecolor: mentions_truecolor(colorterm),
            term_program_is_windows_terminal: term_program.eq_ignore_ascii_case("Windows Terminal"),
            has_wt_session: wt_session.is_some(),
            has_force_color_override: force_color.is_some(),
        }
    }
}

fn mentions_truecolor(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("truecolor") || value.contains("24bit") || value.contains("direct")
}

fn stdout_color_level_for_terminal(
    stdout_level: StdoutColorLevel,
    env: TerminalColorEnv,
) -> StdoutColorLevel {
    if !env.has_force_color_override
        && (env.has_wt_session
            || env.term_program_is_windows_terminal
            || env.colorterm_mentions_truecolor
            || env.term_mentions_truecolor)
    {
        return StdoutColorLevel::TrueColor;
    }

    stdout_level
}

fn best_color_for_color_level(target: (u8, u8, u8), color_level: StdoutColorLevel) -> Color {
    match color_level {
        StdoutColorLevel::TrueColor => rgb_color(target),
        StdoutColorLevel::Ansi256 => xterm_fixed_colors()
            .min_by(|(_, a), (_, b)| {
                perceptual_distance(*a, target)
                    .partial_cmp(&perceptual_distance(*b, target))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map_or_else(Color::default, |(i, _)| indexed_color(i as u8)),
        StdoutColorLevel::Ansi16 | StdoutColorLevel::Unknown => Color::default(),
    }
}

pub fn requery_default_colors() {
    imp::requery_default_colors();
}

#[derive(Clone, Copy)]
pub struct DefaultColors {
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
}

pub fn default_colors() -> Option<DefaultColors> {
    imp::default_colors()
}

pub fn default_fg() -> Option<(u8, u8, u8)> {
    default_colors().map(|c| c.fg)
}

pub fn default_bg() -> Option<(u8, u8, u8)> {
    #[cfg(test)]
    if let Some(bg) = default_bg_override_for_test() {
        return bg;
    }

    default_colors().map(|c| c.bg)
}

#[cfg(test)]
std::thread_local! {
    static DEFAULT_BG_OVERRIDE: std::cell::Cell<Option<Option<(u8, u8, u8)>>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn default_bg_override_for_test() -> Option<Option<(u8, u8, u8)>> {
    DEFAULT_BG_OVERRIDE.with(|override_value| override_value.get())
}

#[cfg(test)]
pub(crate) struct DefaultBgOverrideGuard {
    previous: Option<Option<(u8, u8, u8)>>,
}

#[cfg(test)]
impl Drop for DefaultBgOverrideGuard {
    fn drop(&mut self) {
        DEFAULT_BG_OVERRIDE.with(|override_value| override_value.set(self.previous));
    }
}

#[cfg(test)]
pub(crate) fn override_default_bg_for_test(bg: Option<(u8, u8, u8)>) -> DefaultBgOverrideGuard {
    let previous = DEFAULT_BG_OVERRIDE.with(|override_value| override_value.replace(Some(bg)));
    DefaultBgOverrideGuard { previous }
}

mod imp {
    use super::DefaultColors;
    use std::sync::Mutex;
    use std::sync::OnceLock;

    struct Cache<T> {
        attempted: bool,
        value: Option<T>,
    }

    impl<T> Default for Cache<T> {
        fn default() -> Self {
            Self {
                attempted: false,
                value: None,
            }
        }
    }

    impl<T: Copy> Cache<T> {
        fn get_or_init_with(&mut self, init: impl FnOnce() -> Option<T>) -> Option<T> {
            if !self.attempted {
                self.value = init();
                self.attempted = true;
            }
            self.value
        }
    }

    fn default_colors_cache() -> &'static Mutex<Cache<DefaultColors>> {
        static CACHE: OnceLock<Mutex<Cache<DefaultColors>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(Cache::default()))
    }

    pub(super) fn default_colors() -> Option<DefaultColors> {
        let cache = default_colors_cache();
        let mut cache = cache.lock().ok()?;
        cache.get_or_init_with(query_default_colors)
    }

    pub(super) fn requery_default_colors() {
        if let Ok(mut cache) = default_colors_cache().lock() {
            cache.value = query_default_colors();
            cache.attempted = true;
        }
    }

    fn query_default_colors() -> Option<DefaultColors> {
        None
    }
}

fn xterm_fixed_colors() -> impl Iterator<Item = (usize, (u8, u8, u8))> {
    XTERM_COLORS.into_iter().enumerate().skip(16)
}

pub const XTERM_COLORS: [(u8, u8, u8); 256] = build_xterm_colors();

const fn build_xterm_colors() -> [(u8, u8, u8); 256] {
    let mut colors = [(0, 0, 0); 256];
    let ansi16 = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    let mut i = 0;
    while i < 16 {
        colors[i] = ansi16[i];
        i += 1;
    }

    let steps = [0, 95, 135, 175, 215, 255];
    let mut red = 0;
    while red < 6 {
        let mut green = 0;
        while green < 6 {
            let mut blue = 0;
            while blue < 6 {
                let index = 16 + red * 36 + green * 6 + blue;
                colors[index] = (steps[red], steps[green], steps[blue]);
                blue += 1;
            }
            green += 1;
        }
        red += 1;
    }

    let mut gray = 0;
    while gray < 24 {
        let value = 8 + gray as u8 * 10;
        colors[232 + gray] = (value, value, value);
        gray += 1;
    }

    colors
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn env(
        term: Option<&str>,
        wt_session: Option<&str>,
        colorterm: Option<&str>,
        term_program: Option<&str>,
        force_color: Option<&str>,
    ) -> TerminalColorEnv {
        TerminalColorEnv::from_values(
            term.map(OsStr::new),
            wt_session.map(OsStr::new),
            colorterm.map(OsStr::new),
            term_program.map(OsStr::new),
            force_color.map(OsStr::new),
        )
    }

    #[test]
    fn best_color_uses_truecolor_without_quantization() {
        assert_eq!(
            best_color_for_level((12, 34, 56), StdoutColorLevel::TrueColor),
            rgb_color((12, 34, 56))
        );
    }

    #[test]
    fn best_color_quantizes_to_ansi256_index() {
        assert_eq!(
            best_color_for_level((95, 0, 0), StdoutColorLevel::Ansi256),
            indexed_color(52)
        );
        assert_eq!(
            best_color_for_level((238, 238, 238), StdoutColorLevel::Ansi256),
            indexed_color(255)
        );
    }

    #[test]
    fn best_color_resets_for_ansi16() {
        assert_eq!(
            best_color_for_level((12, 34, 56), StdoutColorLevel::Ansi16),
            Color::Reset
        );
    }

    #[test]
    fn terminal_color_level_wt_session_promotes_to_truecolor() {
        assert_eq!(
            stdout_color_level_for_terminal(
                StdoutColorLevel::Ansi16,
                env(None, Some("1"), None, None, None),
            ),
            StdoutColorLevel::TrueColor
        );
    }

    #[test]
    fn terminal_color_level_windows_terminal_name_promotes_to_truecolor() {
        assert_eq!(
            stdout_color_level_for_terminal(
                StdoutColorLevel::Ansi16,
                env(None, None, None, Some("Windows Terminal"), None),
            ),
            StdoutColorLevel::TrueColor
        );
    }

    #[test]
    fn terminal_color_level_colorterm_truecolor_promotes_to_truecolor() {
        assert_eq!(
            stdout_color_level_for_terminal(
                StdoutColorLevel::Unknown,
                env(None, None, Some("truecolor"), None, None),
            ),
            StdoutColorLevel::TrueColor
        );
    }

    #[test]
    fn terminal_color_level_force_color_keeps_reported_stdout_level() {
        assert_eq!(
            stdout_color_level_for_terminal(
                StdoutColorLevel::Ansi16,
                env(
                    Some("xterm-direct"),
                    Some("1"),
                    Some("truecolor"),
                    Some("Windows Terminal"),
                    Some("1"),
                ),
            ),
            StdoutColorLevel::Ansi16
        );
    }

    #[test]
    fn default_color_accessors_return_none_when_probe_unavailable() {
        requery_default_colors();
        assert_eq!(default_fg(), None);
        assert_eq!(default_bg(), None);
    }
}
