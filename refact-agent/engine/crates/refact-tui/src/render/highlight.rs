use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color as SyntectColor, FontStyle, Highlighter, Style as SyntectStyle, Theme, ThemeSet,
};
use syntect::parsing::{Scope, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;
use two_face::theme::{EmbeddedThemeName, LazyThemeSet};

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<RwLock<Theme>> = OnceLock::new();
static THEME_OVERRIDE: OnceLock<RwLock<Option<String>>> = OnceLock::new();
static THEME_HOME: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

const ANSI_ALPHA_INDEX: u8 = 0x00;
const ANSI_ALPHA_DEFAULT: u8 = 0x01;
const OPAQUE_ALPHA: u8 = 0xFF;
const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;
const MAX_HIGHLIGHT_LINES: usize = 10_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThemeEntry {
    pub name: String,
    pub is_custom: bool,
}

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(two_face::syntax::extra_newlines)
}

pub fn set_theme_override(name: Option<String>, theme_home: Option<PathBuf>) -> Option<String> {
    let warning = validate_theme_name(name.as_deref(), theme_home.as_deref());
    write_lock(theme_override_lock(), name.clone());
    write_lock(theme_home_lock(), theme_home.clone());
    let (theme, fallback_warning) = theme_or_builtin_fallback(resolve_theme_with_override(
        name.as_deref(),
        theme_home.as_deref(),
    ));
    set_syntax_theme(theme);
    warning.or(fallback_warning)
}

pub fn set_syntax_theme(theme: Theme) {
    let mut guard = match theme_lock().write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = theme;
}

pub fn current_syntax_theme() -> Theme {
    match theme_lock().read() {
        Ok(theme) => theme.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub fn validate_theme_name(name: Option<&str>, theme_home: Option<&Path>) -> Option<String> {
    let name = name?;
    if resolve_theme_by_name(name, theme_home).is_some() {
        return None;
    }
    Some(format!(
        "Theme \"{name}\" not found. Using the default syntax theme."
    ))
}

pub fn resolve_theme_by_name(name: &str, theme_home: Option<&Path>) -> Option<Theme> {
    if let Some(embedded) = parse_theme_name(name) {
        return embedded_theme(embedded);
    }
    custom_theme_path(name, theme_home).and_then(|path| ThemeSet::get_theme(path).ok())
}

pub fn adaptive_default_theme_name() -> &'static str {
    match crate::terminal_palette::default_bg() {
        Some(bg) if crate::color::is_light(bg) => "catppuccin-latte",
        _ => "catppuccin-mocha",
    }
}

pub fn configured_theme_name() -> String {
    let name = theme_override_name();
    let theme_home = configured_theme_home();
    if let Some(name) = name.as_deref() {
        if resolve_theme_by_name(name, theme_home.as_deref()).is_some() {
            return name.to_string();
        }
    }
    adaptive_default_theme_name().to_string()
}

pub fn list_available_themes(theme_home: Option<&Path>) -> Vec<ThemeEntry> {
    let mut entries = BUILTIN_THEME_NAMES
        .iter()
        .map(|name| ThemeEntry {
            name: (*name).to_string(),
            is_custom: false,
        })
        .collect::<Vec<_>>();

    if let Some(home) = theme_home {
        let themes_dir = home.join("themes");
        if let Ok(read_dir) = std::fs::read_dir(themes_dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("tmTheme")
                    || ThemeSet::get_theme(&path).is_err()
                {
                    continue;
                }
                let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
                    continue;
                };
                if !entries.iter().any(|entry| entry.name == name) {
                    entries.push(ThemeEntry {
                        name: name.to_string(),
                        is_custom: true,
                    });
                }
            }
        }
    }

    entries.sort_by_cached_key(|entry| entry.name.to_ascii_lowercase());
    entries
}

pub fn foreground_style_for_scopes(scope_names: &[&str]) -> Option<Style> {
    let theme = current_syntax_theme();
    foreground_style_for_scopes_with_theme(&theme, scope_names)
}

pub fn exceeds_highlight_limits(total_bytes: usize, total_lines: usize) -> bool {
    total_bytes > MAX_HIGHLIGHT_BYTES || total_lines > MAX_HIGHLIGHT_LINES
}

pub fn highlight_code_to_lines(code: &str, lang: &str) -> Vec<Line<'static>> {
    highlight_code_to_lines_with_color(code, lang, super::color_enabled_from_env())
}

pub(super) fn highlight_code_to_lines_with_color(
    code: &str,
    lang: &str,
    color_enabled: bool,
) -> Vec<Line<'static>> {
    highlight_code_to_styled_spans_with_color(code, lang, color_enabled)
        .map(|lines| lines.into_iter().map(Line::from).collect())
        .unwrap_or_else(|| plain_code_lines(code))
}

pub fn highlight_bash_to_lines(script: &str) -> Vec<Line<'static>> {
    highlight_code_to_lines(script, "bash")
}

pub fn highlight_code_to_styled_spans(code: &str, lang: &str) -> Option<Vec<Vec<Span<'static>>>> {
    highlight_code_to_styled_spans_with_color(code, lang, super::color_enabled_from_env())
}

pub(super) fn highlight_code_to_styled_spans_with_color(
    code: &str,
    lang: &str,
    color_enabled: bool,
) -> Option<Vec<Vec<Span<'static>>>> {
    if !color_enabled {
        return None;
    }
    let guard = match theme_lock().read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    highlight_to_line_spans_with_theme(code, lang, &guard)
}

fn theme_lock() -> &'static RwLock<Theme> {
    THEME.get_or_init(|| {
        let (theme, warning) = theme_or_builtin_fallback(resolve_theme_with_override(
            theme_override_name().as_deref(),
            configured_theme_home().as_deref(),
        ));
        if let Some(warning) = warning {
            tracing::warn!("{warning}");
        }
        RwLock::new(theme)
    })
}

fn theme_override_lock() -> &'static RwLock<Option<String>> {
    THEME_OVERRIDE.get_or_init(|| RwLock::new(None))
}

fn theme_home_lock() -> &'static RwLock<Option<PathBuf>> {
    THEME_HOME.get_or_init(|| RwLock::new(None))
}

fn theme_override_name() -> Option<String> {
    read_lock(theme_override_lock())
}

fn configured_theme_home() -> Option<PathBuf> {
    read_lock(theme_home_lock())
}

fn read_lock<T: Clone>(lock: &RwLock<T>) -> T {
    match lock.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn write_lock<T>(lock: &RwLock<T>, value: T) {
    let mut guard = match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = value;
}

fn resolve_theme_with_override(name: Option<&str>, theme_home: Option<&Path>) -> Option<Theme> {
    if let Some(name) = name {
        if let Some(theme) = resolve_theme_by_name(name, theme_home) {
            return Some(theme);
        }
    }
    resolve_theme_by_name(adaptive_default_theme_name(), None)
}

fn embedded_theme(name: EmbeddedThemeName) -> Option<Theme> {
    let theme_set = LazyThemeSet::from(two_face::theme::extra());
    embedded_theme_from_set(&theme_set, name)
}

fn embedded_theme_from_set(theme_set: &LazyThemeSet, name: EmbeddedThemeName) -> Option<Theme> {
    theme_set.get(name.as_name()).cloned()
}

fn theme_or_builtin_fallback(theme: Option<Theme>) -> (Theme, Option<String>) {
    match theme {
        Some(theme) => (theme, None),
        None => (
            embedded_theme(EmbeddedThemeName::Ansi).unwrap_or_default(),
            Some("Syntax theme unavailable. Using the ANSI fallback theme.".to_string()),
        ),
    }
}

fn custom_theme_path(name: &str, theme_home: Option<&Path>) -> Option<PathBuf> {
    theme_home.map(|home| home.join("themes").join(format!("{name}.tmTheme")))
}

fn parse_theme_name(name: &str) -> Option<EmbeddedThemeName> {
    match name {
        "ansi" => Some(EmbeddedThemeName::Ansi),
        "base16" => Some(EmbeddedThemeName::Base16),
        "base16-eighties-dark" => Some(EmbeddedThemeName::Base16EightiesDark),
        "base16-mocha-dark" => Some(EmbeddedThemeName::Base16MochaDark),
        "base16-ocean-dark" => Some(EmbeddedThemeName::Base16OceanDark),
        "base16-ocean-light" => Some(EmbeddedThemeName::Base16OceanLight),
        "base16-256" => Some(EmbeddedThemeName::Base16_256),
        "catppuccin-frappe" => Some(EmbeddedThemeName::CatppuccinFrappe),
        "catppuccin-latte" => Some(EmbeddedThemeName::CatppuccinLatte),
        "catppuccin-macchiato" => Some(EmbeddedThemeName::CatppuccinMacchiato),
        "catppuccin-mocha" => Some(EmbeddedThemeName::CatppuccinMocha),
        "coldark-cold" => Some(EmbeddedThemeName::ColdarkCold),
        "coldark-dark" => Some(EmbeddedThemeName::ColdarkDark),
        "dark-neon" => Some(EmbeddedThemeName::DarkNeon),
        "dracula" => Some(EmbeddedThemeName::Dracula),
        "github" => Some(EmbeddedThemeName::Github),
        "gruvbox-dark" => Some(EmbeddedThemeName::GruvboxDark),
        "gruvbox-light" => Some(EmbeddedThemeName::GruvboxLight),
        "inspired-github" => Some(EmbeddedThemeName::InspiredGithub),
        "1337" => Some(EmbeddedThemeName::Leet),
        "monokai-extended" => Some(EmbeddedThemeName::MonokaiExtended),
        "monokai-extended-bright" => Some(EmbeddedThemeName::MonokaiExtendedBright),
        "monokai-extended-light" => Some(EmbeddedThemeName::MonokaiExtendedLight),
        "monokai-extended-origin" => Some(EmbeddedThemeName::MonokaiExtendedOrigin),
        "nord" => Some(EmbeddedThemeName::Nord),
        "one-half-dark" => Some(EmbeddedThemeName::OneHalfDark),
        "one-half-light" => Some(EmbeddedThemeName::OneHalfLight),
        "solarized-dark" => Some(EmbeddedThemeName::SolarizedDark),
        "solarized-light" => Some(EmbeddedThemeName::SolarizedLight),
        "sublime-snazzy" => Some(EmbeddedThemeName::SublimeSnazzy),
        "two-dark" => Some(EmbeddedThemeName::TwoDark),
        "zenburn" => Some(EmbeddedThemeName::Zenburn),
        _ => None,
    }
}

const BUILTIN_THEME_NAMES: &[&str] = &[
    "1337",
    "ansi",
    "base16",
    "base16-256",
    "base16-eighties-dark",
    "base16-mocha-dark",
    "base16-ocean-dark",
    "base16-ocean-light",
    "catppuccin-frappe",
    "catppuccin-latte",
    "catppuccin-macchiato",
    "catppuccin-mocha",
    "coldark-cold",
    "coldark-dark",
    "dark-neon",
    "dracula",
    "github",
    "gruvbox-dark",
    "gruvbox-light",
    "inspired-github",
    "monokai-extended",
    "monokai-extended-bright",
    "monokai-extended-light",
    "monokai-extended-origin",
    "nord",
    "one-half-dark",
    "one-half-light",
    "solarized-dark",
    "solarized-light",
    "sublime-snazzy",
    "two-dark",
    "zenburn",
];

fn foreground_style_for_scopes_with_theme(theme: &Theme, scope_names: &[&str]) -> Option<Style> {
    let highlighter = Highlighter::new(theme);
    scope_names.iter().find_map(|scope_name| {
        let scope = Scope::new(scope_name).ok()?;
        let foreground = highlighter.style_mod_for_stack(&[scope]).foreground?;
        convert_syntect_color(foreground).map(|foreground| Style::default().fg(foreground))
    })
}

fn ansi_palette_color(index: u8) -> Color {
    match index {
        0x00 => Color::Black,
        0x01 => Color::Red,
        0x02 => Color::Green,
        0x03 => Color::Yellow,
        0x04 => Color::Blue,
        0x05 => Color::Magenta,
        0x06 => Color::Cyan,
        0x07 => Color::Gray,
        n => Color::Indexed(n),
    }
}

fn convert_syntect_color(color: SyntectColor) -> Option<Color> {
    match color.a {
        ANSI_ALPHA_INDEX => Some(ansi_palette_color(color.r)),
        ANSI_ALPHA_DEFAULT => None,
        OPAQUE_ALPHA => Some(Color::Rgb(color.r, color.g, color.b)),
        _ => None,
    }
}

fn convert_style(syntect_style: SyntectStyle) -> Style {
    let mut style = Style::default();
    if let Some(foreground) = convert_syntect_color(syntect_style.foreground) {
        style = style.fg(foreground);
    }
    if syntect_style.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

fn find_syntax(lang: &str) -> Option<&'static SyntaxReference> {
    let syntax_set = syntax_set();
    let lang = language_name(lang);
    let patched = match lang {
        "csharp" | "c-sharp" => "c#",
        "golang" => "go",
        "python3" => "python",
        "shell" => "bash",
        _ => lang,
    };

    syntax_set
        .find_syntax_by_token(patched)
        .or_else(|| syntax_set.find_syntax_by_name(patched))
        .or_else(|| {
            let lower = patched.to_ascii_lowercase();
            syntax_set
                .syntaxes()
                .iter()
                .find(|syntax| syntax.name.to_ascii_lowercase() == lower)
        })
        .or_else(|| syntax_set.find_syntax_by_extension(patched))
}

fn language_name(lang: &str) -> &str {
    lang.split(|ch: char| ch.is_whitespace() || ch == ',')
        .next()
        .unwrap_or_default()
        .trim()
}

fn highlight_to_line_spans_with_theme(
    code: &str,
    lang: &str,
    theme: &Theme,
) -> Option<Vec<Vec<Span<'static>>>> {
    if code.is_empty() || exceeds_highlight_limits(code.len(), code.lines().count()) {
        return None;
    }

    let syntax = find_syntax(lang)?;
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut lines = Vec::new();

    for line in LinesWithEndings::from(code) {
        let ranges = highlighter.highlight_line(line, syntax_set()).ok()?;
        let mut spans = Vec::new();
        for (style, text) in ranges {
            let text = text.trim_end_matches(['\n', '\r']);
            if !text.is_empty() {
                spans.push(Span::styled(text.to_string(), convert_style(style)));
            }
        }
        if spans.is_empty() {
            spans.push(Span::raw(String::new()));
        }
        lines.push(spans);
    }

    Some(lines)
}

fn plain_code_lines(code: &str) -> Vec<Line<'static>> {
    let mut lines = code
        .lines()
        .map(|line| Line::from(line.to_string()))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(Line::from(String::new()));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exceeds_highlight_limits_uses_byte_and_line_caps() {
        assert!(!exceeds_highlight_limits(
            MAX_HIGHLIGHT_BYTES,
            MAX_HIGHLIGHT_LINES
        ));
        assert!(exceeds_highlight_limits(
            MAX_HIGHLIGHT_BYTES + 1,
            MAX_HIGHLIGHT_LINES
        ));
        assert!(exceeds_highlight_limits(
            MAX_HIGHLIGHT_BYTES,
            MAX_HIGHLIGHT_LINES + 1
        ));
    }
}
