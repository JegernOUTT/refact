use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeRole {
    Accent,
    Muted,
    Text,
    Warning,
    Error,
    Success,
    Highlight,
    Border,
}

impl ThemeRole {
    pub fn name(self) -> &'static str {
        match self {
            Self::Accent => "accent",
            Self::Muted => "muted",
            Self::Text => "text",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Success => "success",
            Self::Highlight => "highlight",
            Self::Border => "border",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiTheme {
    name: String,
    accent: Style,
    muted: Style,
    text: Style,
    warning: Style,
    error: Style,
    success: Style,
    highlight: Style,
    border: Style,
}

impl Default for TuiTheme {
    fn default() -> Self {
        Self::dark()
    }
}

impl TuiTheme {
    pub fn dark() -> Self {
        Self {
            name: "dark".to_string(),
            accent: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            muted: Style::default().fg(Color::DarkGray),
            text: Style::default().fg(Color::White),
            warning: Style::default().fg(Color::Yellow),
            error: Style::default().fg(Color::Red),
            success: Style::default().fg(Color::Green),
            highlight: Style::default().fg(Color::Cyan),
            border: Style::default().fg(Color::DarkGray),
        }
    }

    pub fn light() -> Self {
        Self {
            name: "light".to_string(),
            accent: Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            muted: Style::default().fg(Color::Gray),
            text: Style::default().fg(Color::Black),
            warning: Style::default().fg(Color::Yellow),
            error: Style::default().fg(Color::Red),
            success: Style::default().fg(Color::Green),
            highlight: Style::default().fg(Color::Blue),
            border: Style::default().fg(Color::Gray),
        }
    }

    pub fn plain() -> Self {
        Self {
            name: "plain".to_string(),
            accent: Style::default(),
            muted: Style::default(),
            text: Style::default(),
            warning: Style::default(),
            error: Style::default(),
            success: Style::default(),
            highlight: Style::default(),
            border: Style::default(),
        }
    }

    pub fn builtin_names() -> &'static [&'static str] {
        &["dark", "light", "plain"]
    }

    pub fn named(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(Self::dark()),
            "light" => Some(Self::light()),
            "plain" => Some(Self::plain()),
            _ => None,
        }
    }

    pub fn from_toml_str(content: &str) -> Result<Self, String> {
        let config: ThemeFileConfig = toml::from_str(content).map_err(|error| error.to_string())?;
        Ok(Self::from_config(config))
    }

    pub fn from_config_file_content(content: Option<&str>) -> Result<Self, String> {
        match content {
            Some(content) if !content.trim().is_empty() => Self::from_toml_str(content),
            _ => Ok(Self::default()),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn style(&self, role: ThemeRole) -> Style {
        match role {
            ThemeRole::Accent => self.accent,
            ThemeRole::Muted => self.muted,
            ThemeRole::Text => self.text,
            ThemeRole::Warning => self.warning,
            ThemeRole::Error => self.error,
            ThemeRole::Success => self.success,
            ThemeRole::Highlight => self.highlight,
            ThemeRole::Border => self.border,
        }
    }

    fn from_config(config: ThemeFileConfig) -> Self {
        let mut theme = config
            .theme
            .as_ref()
            .and_then(|section| section.name.as_deref())
            .and_then(Self::named)
            .unwrap_or_default();
        if let Some(section) = config.theme {
            theme.apply_overrides(section);
        }
        theme
    }

    fn apply_overrides(&mut self, section: ThemeSection) {
        if let Some(value) = section.name {
            self.name = value;
        }
        if let Some(value) = section.accent.and_then(parse_style) {
            self.accent = value;
        }
        if let Some(value) = section.muted.and_then(parse_style) {
            self.muted = value;
        }
        if let Some(value) = section.text.and_then(parse_style) {
            self.text = value;
        }
        if let Some(value) = section.warning.and_then(parse_style) {
            self.warning = value;
        }
        if let Some(value) = section.error.and_then(parse_style) {
            self.error = value;
        }
        if let Some(value) = section.success.and_then(parse_style) {
            self.success = value;
        }
        if let Some(value) = section.highlight.and_then(parse_style) {
            self.highlight = value;
        }
        if let Some(value) = section.border.and_then(parse_style) {
            self.border = value;
        }
    }
}

#[derive(Debug, Deserialize)]
struct ThemeFileConfig {
    #[serde(default)]
    theme: Option<ThemeSection>,
}

#[derive(Debug, Deserialize)]
struct ThemeSection {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    accent: Option<String>,
    #[serde(default)]
    muted: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    warning: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    success: Option<String>,
    #[serde(default)]
    highlight: Option<String>,
    #[serde(default)]
    border: Option<String>,
}

fn parse_style(value: String) -> Option<Style> {
    parse_color(&value).map(|color| Style::default().fg(color))
}

fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "dark-gray" | "darkgrey" | "dark-grey" => Some(Color::DarkGray),
        "white" => Some(Color::White),
        "reset" | "plain" | "default" => Some(Color::Reset),
        value if value.starts_with('#') && value.len() == 7 => {
            let red = u8::from_str_radix(&value[1..3], 16).ok()?;
            let green = u8::from_str_radix(&value[3..5], 16).ok()?;
            let blue = u8::from_str_radix(&value[5..7], 16).ok()?;
            Some(Color::Rgb(red, green, blue))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_builtin_and_override_theme() {
        let theme = TuiTheme::from_toml_str(
            r##"
[theme]
name = "light"
accent = "#112233"
muted = "dark-gray"
"##,
        )
        .unwrap();

        assert_eq!(theme.name(), "light");
        assert_eq!(
            theme.style(ThemeRole::Accent).fg,
            Some(Color::Rgb(17, 34, 51))
        );
        assert_eq!(theme.style(ThemeRole::Muted).fg, Some(Color::DarkGray));
    }
}
