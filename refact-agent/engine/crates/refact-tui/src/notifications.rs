use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Deserialize;

const NOTIFY_ENV: &str = "REFACT_TUI_NOTIFY";
const DEFAULT_DEBOUNCE_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationKind {
    TurnComplete,
    ApprovalNeeded,
}

impl NotificationKind {
    fn text(self) -> &'static str {
        match self {
            Self::TurnComplete => "Refact: response ready",
            Self::ApprovalNeeded => "Refact: approval needed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationConfig {
    enabled: bool,
    bell: bool,
    debounce: Duration,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self::new(true, true, Duration::from_millis(DEFAULT_DEBOUNCE_MS))
    }
}

impl NotificationConfig {
    pub fn new(enabled: bool, bell: bool, debounce: Duration) -> Self {
        Self {
            enabled,
            bell,
            debounce,
        }
    }

    pub fn from_config_file_content(content: Option<&str>) -> Result<Self, String> {
        Self::from_config_file_content_with_env(content, std::env::var(NOTIFY_ENV).ok().as_deref())
    }

    pub fn from_env() -> Self {
        let mut config = Self::default();
        config.apply_env_value(std::env::var(NOTIFY_ENV).ok().as_deref());
        config
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn bell(&self) -> bool {
        self.bell
    }

    fn from_config_file_content_with_env(
        content: Option<&str>,
        env_value: Option<&str>,
    ) -> Result<Self, String> {
        let mut config = Self::default();
        if let Some(content) = content.filter(|content| !content.trim().is_empty()) {
            let file: NotificationFileConfig =
                toml::from_str(content).map_err(|error| error.to_string())?;
            if let Some(setting) = file.notify {
                config.apply_setting(setting);
            }
            if let Some(setting) = file.notifications {
                config.apply_setting(NotificationSetting::Section(setting));
            }
        }
        config.apply_env_value(env_value);
        Ok(config)
    }

    fn apply_setting(&mut self, setting: NotificationSetting) {
        match setting {
            NotificationSetting::Flag(enabled) => self.enabled = enabled,
            NotificationSetting::Mode(mode) => self.apply_mode(&mode),
            NotificationSetting::Section(section) => self.apply_section(section),
        }
    }

    fn apply_section(&mut self, section: NotificationSection) {
        if let Some(enabled) = section.enabled {
            self.enabled = enabled;
        }
        if let Some(mode) = section.mode {
            self.apply_mode(&mode);
        }
        if let Some(bell) = section.bell {
            self.bell = bell;
        }
        if let Some(debounce_ms) = section.debounce_ms {
            self.debounce = Duration::from_millis(debounce_ms);
        }
    }

    fn apply_env_value(&mut self, value: Option<&str>) {
        if let Some(value) = value {
            self.apply_mode(value);
        }
    }

    fn apply_mode(&mut self, value: &str) {
        match value.trim().to_ascii_lowercase().as_str() {
            "0" | "false" | "no" | "off" | "quiet" | "silent" | "none" | "disabled" => {
                self.enabled = false;
            }
            "1" | "true" | "yes" | "on" | "notify" | "osc9" | "bell" => {
                self.enabled = true;
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusState {
    Unknown,
    Focused,
    Unfocused,
}

#[derive(Debug, Clone)]
pub struct NotificationManager {
    config: NotificationConfig,
    focus: FocusState,
    last_sent: HashMap<NotificationKind, Instant>,
    pending: Vec<Vec<u8>>,
}

impl Default for NotificationManager {
    fn default() -> Self {
        Self::new(NotificationConfig::from_env())
    }
}

impl NotificationManager {
    pub fn new(config: NotificationConfig) -> Self {
        Self {
            config,
            focus: FocusState::Unknown,
            last_sent: HashMap::new(),
            pending: Vec::new(),
        }
    }

    pub fn set_config(&mut self, config: NotificationConfig) {
        self.config = config;
        if !self.config.enabled {
            self.pending.clear();
        }
    }

    pub fn config(&self) -> &NotificationConfig {
        &self.config
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focus = if focused {
            FocusState::Focused
        } else {
            FocusState::Unfocused
        };
    }

    pub fn queue(&mut self, kind: NotificationKind) -> bool {
        self.queue_at(kind, Instant::now())
    }

    pub fn queue_at(&mut self, kind: NotificationKind, now: Instant) -> bool {
        if !self.config.enabled || self.focus == FocusState::Focused {
            return false;
        }
        if self
            .last_sent
            .get(&kind)
            .is_some_and(|last| now.saturating_duration_since(*last) < self.config.debounce)
        {
            return false;
        }
        self.last_sent.insert(kind, now);
        self.pending.push(osc9_sequence(
            kind.text(),
            self.config.bell,
            tmux_passthrough_enabled_from_env(),
        ));
        true
    }

    pub fn drain_pending(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.pending)
    }
}

pub fn osc9_sequence(text: &str, bell: bool, tmux_passthrough: bool) -> Vec<u8> {
    let osc = format!("\x1b]9;{}\x07", sanitize_text(text));
    let mut bytes = if tmux_passthrough {
        format!("\x1bPtmux;{}\x1b\\", osc.replacen('\x1b', "\x1b\x1b", 1)).into_bytes()
    } else {
        osc.into_bytes()
    };
    if bell {
        bytes.push(b'\x07');
    }
    bytes
}

fn sanitize_text(text: &str) -> String {
    let cleaned = text
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    let compact = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        "Refact".to_string()
    } else {
        compact
    }
}

fn tmux_passthrough_enabled_from_env() -> bool {
    std::env::var_os("TMUX").is_some_and(|value| !value.is_empty())
}

#[derive(Debug, Deserialize)]
struct NotificationFileConfig {
    #[serde(default)]
    notifications: Option<NotificationSection>,
    #[serde(default)]
    notify: Option<NotificationSetting>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NotificationSetting {
    Flag(bool),
    Mode(String),
    Section(NotificationSection),
}

#[derive(Debug, Deserialize)]
struct NotificationSection {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    bell: Option<bool>,
    #[serde(default)]
    debounce_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(debounce_ms: u64) -> NotificationConfig {
        NotificationConfig {
            enabled: true,
            bell: true,
            debounce: Duration::from_millis(debounce_ms),
        }
    }

    #[test]
    fn osc9_sequence_encodes_payload_and_bell() {
        assert_eq!(
            osc9_sequence("Refact: response ready", true, false),
            b"\x1b]9;Refact: response ready\x07\x07"
        );
        assert_eq!(
            osc9_sequence("Refact: response ready", false, false),
            b"\x1b]9;Refact: response ready\x07"
        );
    }

    #[test]
    fn osc9_sequence_escapes_control_text() {
        assert_eq!(
            osc9_sequence("hello\x07\x1bworld\nnext", false, false),
            b"\x1b]9;hello world next\x07"
        );
        assert_eq!(osc9_sequence("\x07\n", false, false), b"\x1b]9;Refact\x07");
    }

    #[test]
    fn osc9_sequence_wraps_for_tmux_passthrough() {
        assert_eq!(
            osc9_sequence("hi", true, true),
            b"\x1bPtmux;\x1b\x1b]9;hi\x07\x1b\\\x07"
        );
    }

    #[test]
    fn notification_manager_debounces_per_kind() {
        let mut manager = NotificationManager::new(config(1_000));
        let start = Instant::now();

        assert!(manager.queue_at(NotificationKind::TurnComplete, start));
        assert!(!manager.queue_at(
            NotificationKind::TurnComplete,
            start + Duration::from_millis(999)
        ));
        assert!(manager.queue_at(
            NotificationKind::ApprovalNeeded,
            start + Duration::from_millis(999)
        ));
        assert!(manager.queue_at(
            NotificationKind::TurnComplete,
            start + Duration::from_millis(1_000)
        ));
        assert_eq!(manager.drain_pending().len(), 3);
    }

    #[test]
    fn notification_manager_honors_focus_and_enabled_gate() {
        let start = Instant::now();
        let mut manager = NotificationManager::new(config(0));

        manager.set_focused(true);
        assert!(!manager.queue_at(NotificationKind::TurnComplete, start));
        manager.set_focused(false);
        assert!(manager.queue_at(NotificationKind::TurnComplete, start));

        let mut disabled = config(0);
        disabled.enabled = false;
        manager.set_config(disabled);
        assert!(!manager.queue_at(
            NotificationKind::ApprovalNeeded,
            start + Duration::from_secs(1)
        ));
    }

    #[test]
    fn notification_config_parses_aliases() {
        let config = NotificationConfig::from_config_file_content(Some(
            r#"
notify = "off"

[notifications]
enabled = true
bell = false
debounce_ms = 17
"#,
        ))
        .unwrap();

        assert!(config.enabled);
        assert!(!config.bell);
        assert_eq!(config.debounce, Duration::from_millis(17));
    }

    #[test]
    fn notification_config_parses_notify_table() {
        let config = NotificationConfig::from_config_file_content_with_env(
            Some(
                r#"
[notify]
mode = "on"
bell = false
"#,
            ),
            None,
        )
        .unwrap();

        assert!(config.enabled);
        assert!(!config.bell);
    }

    #[test]
    fn notification_config_env_overrides_file() {
        let config = NotificationConfig::from_config_file_content_with_env(
            Some(
                r#"
[notifications]
enabled = true
"#,
            ),
            Some("quiet"),
        )
        .unwrap();

        assert!(!config.enabled);
    }
}
