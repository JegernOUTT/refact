use std::io::{self, IsTerminal, Write};
use std::panic;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode as crossterm_disable_raw_mode, enable_raw_mode as crossterm_enable_raw_mode,
    EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};
use serde::Deserialize;

pub type RefactTerminal = Terminal<CrosstermBackend<io::Stdout>>;

const INLINE_VIEWPORT_HEIGHT: u16 = 12;
const FALLBACK_ENV: &str = "REFACT_TUI_ALT_SCREEN";
const TITLE_ENV: &str = "REFACT_TUI_TERMINAL_TITLE";
const DEFAULT_TERMINAL_TITLE: &str = "refact";
const MAX_TERMINAL_TITLE_CHARS: usize = 80;
const PUSH_TITLE_SEQUENCE: &[u8] = b"\x1b[22;0t";
const POP_TITLE_SEQUENCE: &[u8] = b"\x1b[23;0t";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalMode {
    Inline,
    AlternateScreen,
}

impl TerminalMode {
    pub fn from_env() -> Self {
        if std::env::var(FALLBACK_ENV).is_ok_and(|value| is_truthy(&value)) {
            Self::AlternateScreen
        } else {
            Self::Inline
        }
    }
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn is_falsey(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalTitleConfig {
    enabled: bool,
    is_tty: bool,
}

impl TerminalTitleConfig {
    pub fn from_config_content(content: Option<&str>, is_tty: bool) -> Self {
        Self {
            enabled: content
                .and_then(title_enabled_from_config_content)
                .unwrap_or(true),
            is_tty,
        }
    }

    pub fn from_env(content: Option<&str>) -> Self {
        let mut config = Self::from_config_content(content, io::stdout().is_terminal());
        if let Some(enabled) = std::env::var(TITLE_ENV)
            .ok()
            .and_then(|value| title_enabled_from_value(&value))
        {
            config.enabled = enabled;
        }
        config
    }

    #[cfg(test)]
    fn disabled() -> Self {
        Self {
            enabled: false,
            is_tty: false,
        }
    }

    fn active(self) -> bool {
        self.enabled && self.is_tty
    }
}

#[derive(Debug, Deserialize)]
struct TerminalTitleFileConfig {
    #[serde(default)]
    terminal_title: Option<bool>,
    #[serde(default)]
    terminal: Option<TerminalTitleSection>,
}

#[derive(Debug, Deserialize)]
struct TerminalTitleSection {
    #[serde(default)]
    title: Option<bool>,
    #[serde(default)]
    terminal_title: Option<bool>,
}

fn title_enabled_from_config_content(content: &str) -> Option<bool> {
    let config: TerminalTitleFileConfig = toml::from_str(content).ok()?;
    config
        .terminal
        .and_then(|section| section.title.or(section.terminal_title))
        .or(config.terminal_title)
}

fn title_enabled_from_value(value: &str) -> Option<bool> {
    if is_truthy(value) {
        Some(true)
    } else if is_falsey(value) {
        Some(false)
    } else {
        None
    }
}

pub fn terminal_title(project: Option<&str>, status: &str) -> String {
    let project = project
        .map(clean_title_part)
        .filter(|project| !project.is_empty())
        .unwrap_or_else(|| "no project".to_string());
    let status = clean_title_part(status);
    truncate_title(&format!("refact · {project} · {status}"))
}

fn clean_title_part(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_title(value: &str) -> String {
    if value.chars().count() <= MAX_TERMINAL_TITLE_CHARS {
        return value.to_string();
    }
    let mut title = value
        .chars()
        .take(MAX_TERMINAL_TITLE_CHARS.saturating_sub(1))
        .collect::<String>();
    title.push('…');
    title
}

fn osc2_title_sequence(title: &str) -> Vec<u8> {
    format!("\x1b]2;{}\x07", clean_title_part(title)).into_bytes()
}

pub struct TerminalSession {
    terminal: RefactTerminal,
    guard: TerminalRestoreGuard<CrosstermTerminalOps<io::Stdout>>,
    last_title: Option<String>,
}

impl TerminalSession {
    pub fn start() -> io::Result<Self> {
        Self::start_with_title_config(TerminalTitleConfig::from_env(None))
    }

    pub fn start_with_mode(mode: TerminalMode) -> io::Result<Self> {
        Self::start_with_mode_and_title_config(mode, TerminalTitleConfig::from_env(None))
    }

    pub fn start_with_title_config(title_config: TerminalTitleConfig) -> io::Result<Self> {
        Self::start_with_mode_and_title_config(TerminalMode::from_env(), title_config)
    }

    pub fn start_with_mode_and_title_config(
        mode: TerminalMode,
        title_config: TerminalTitleConfig,
    ) -> io::Result<Self> {
        let mut guard = TerminalRestoreGuard::new_with_title_config(
            CrosstermTerminalOps::new(io::stdout()),
            mode,
            title_config,
        );
        guard.initialize()?;
        install_panic_restore_hook(mode, title_config);
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = match mode {
            TerminalMode::Inline => Terminal::with_options(
                backend,
                TerminalOptions {
                    viewport: Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
                },
            )?,
            TerminalMode::AlternateScreen => Terminal::new(backend)?,
        };
        Ok(Self {
            terminal,
            guard,
            last_title: None,
        })
    }

    pub fn terminal_mut(&mut self) -> &mut RefactTerminal {
        &mut self.terminal
    }

    pub fn write_clipboard(
        &mut self,
        text: &str,
    ) -> io::Result<crate::clipboard::ClipboardCopyReport> {
        crate::clipboard::write_osc52_copy(
            self.terminal.backend_mut(),
            text,
            crate::clipboard::tmux_passthrough_enabled_from_env(),
        )
    }

    pub fn mode(&self) -> TerminalMode {
        self.guard.mode
    }

    pub fn set_title(&mut self, title: &str) -> io::Result<()> {
        let title = clean_title_part(title);
        if self.last_title.as_deref() == Some(title.as_str()) {
            return Ok(());
        }
        self.guard.set_title(&title)?;
        if self.guard.title_active() {
            self.last_title = Some(title);
        }
        Ok(())
    }

    pub fn suspend(&mut self) {
        self.guard.restore();
        self.last_title = None;
    }

    pub fn resume(&mut self) -> io::Result<()> {
        self.guard.resume()?;
        self.last_title = None;
        self.terminal.clear()
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.guard.restore();
    }
}

fn install_panic_restore_hook(mode: TerminalMode, title_config: TerminalTitleConfig) {
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let mut ops = CrosstermTerminalOps::new(io::stdout());
        restore_terminal_state(&mut ops, RestoreState::started(mode, title_config.active()));
        previous_hook(panic_info);
    }));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct RestoreState {
    raw_mode: bool,
    alternate_screen: bool,
    mouse_capture: bool,
    bracketed_paste: bool,
    cursor_hidden: bool,
    title_pushed: bool,
}

impl RestoreState {
    fn started(mode: TerminalMode, title_pushed: bool) -> Self {
        Self {
            raw_mode: true,
            alternate_screen: mode == TerminalMode::AlternateScreen,
            mouse_capture: true,
            bracketed_paste: true,
            cursor_hidden: true,
            title_pushed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalStep {
    EnableRawMode,
    EnterAlternateScreen,
    EnableMouseCapture,
    EnableBracketedPaste,
    HideCursor,
    ShowCursor,
    DisableBracketedPaste,
    DisableMouseCapture,
    LeaveAlternateScreen,
    DisableRawMode,
    PushTitle,
    PopTitle,
}

trait TerminalOps {
    fn apply(&mut self, step: TerminalStep) -> io::Result<()>;
    fn set_title(&mut self, title: &str) -> io::Result<()>;
}

struct CrosstermTerminalOps<W: Write> {
    writer: W,
}

impl<W: Write> CrosstermTerminalOps<W> {
    fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W: Write> TerminalOps for CrosstermTerminalOps<W> {
    fn apply(&mut self, step: TerminalStep) -> io::Result<()> {
        match step {
            TerminalStep::EnableRawMode => crossterm_enable_raw_mode(),
            TerminalStep::EnterAlternateScreen => execute!(self.writer, EnterAlternateScreen),
            TerminalStep::EnableMouseCapture => execute!(self.writer, EnableMouseCapture),
            TerminalStep::EnableBracketedPaste => execute!(self.writer, EnableBracketedPaste),
            TerminalStep::HideCursor => execute!(self.writer, Hide),
            TerminalStep::ShowCursor => execute!(self.writer, Show),
            TerminalStep::DisableBracketedPaste => execute!(self.writer, DisableBracketedPaste),
            TerminalStep::DisableMouseCapture => execute!(self.writer, DisableMouseCapture),
            TerminalStep::LeaveAlternateScreen => execute!(self.writer, LeaveAlternateScreen),
            TerminalStep::DisableRawMode => crossterm_disable_raw_mode(),
            TerminalStep::PushTitle => {
                self.writer.write_all(PUSH_TITLE_SEQUENCE)?;
                self.writer.flush()
            }
            TerminalStep::PopTitle => {
                self.writer.write_all(POP_TITLE_SEQUENCE)?;
                self.writer.flush()
            }
        }
    }

    fn set_title(&mut self, title: &str) -> io::Result<()> {
        self.writer.write_all(&osc2_title_sequence(title))?;
        self.writer.flush()
    }
}

struct TerminalRestoreGuard<O: TerminalOps> {
    ops: O,
    mode: TerminalMode,
    active: bool,
    state: RestoreState,
    title_config: TerminalTitleConfig,
}

impl<O: TerminalOps> TerminalRestoreGuard<O> {
    #[cfg(test)]
    fn new(ops: O, mode: TerminalMode) -> Self {
        Self::new_with_title_config(ops, mode, TerminalTitleConfig::disabled())
    }

    fn new_with_title_config(
        ops: O,
        mode: TerminalMode,
        title_config: TerminalTitleConfig,
    ) -> Self {
        Self {
            ops,
            mode,
            active: true,
            state: RestoreState::default(),
            title_config,
        }
    }

    fn initialize(&mut self) -> io::Result<()> {
        if self.title_active() {
            let _ = self.apply_start_step(TerminalStep::PushTitle);
        }
        self.apply_start_step(TerminalStep::EnableRawMode)?;
        if self.mode == TerminalMode::AlternateScreen {
            self.apply_start_step(TerminalStep::EnterAlternateScreen)?;
        }
        self.apply_start_step(TerminalStep::EnableMouseCapture)?;
        self.apply_start_step(TerminalStep::EnableBracketedPaste)?;
        self.apply_start_step(TerminalStep::HideCursor)
    }

    fn apply_start_step(&mut self, step: TerminalStep) -> io::Result<()> {
        self.ops.apply(step)?;
        match step {
            TerminalStep::EnableRawMode => self.state.raw_mode = true,
            TerminalStep::EnterAlternateScreen => self.state.alternate_screen = true,
            TerminalStep::EnableMouseCapture => self.state.mouse_capture = true,
            TerminalStep::EnableBracketedPaste => self.state.bracketed_paste = true,
            TerminalStep::HideCursor => self.state.cursor_hidden = true,
            TerminalStep::PushTitle => self.state.title_pushed = true,
            TerminalStep::ShowCursor
            | TerminalStep::DisableBracketedPaste
            | TerminalStep::DisableMouseCapture
            | TerminalStep::LeaveAlternateScreen
            | TerminalStep::DisableRawMode
            | TerminalStep::PopTitle => {}
        }
        Ok(())
    }

    fn restore(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        restore_terminal_state(&mut self.ops, self.state);
        self.state = RestoreState::default();
    }

    fn resume(&mut self) -> io::Result<()> {
        if self.active {
            return Ok(());
        }
        self.active = true;
        self.initialize()
    }

    fn title_active(&self) -> bool {
        self.title_config.active()
    }

    fn set_title(&mut self, title: &str) -> io::Result<()> {
        if !self.active || !self.title_active() {
            return Ok(());
        }
        self.ops.set_title(title)
    }
}

impl<O: TerminalOps> Drop for TerminalRestoreGuard<O> {
    fn drop(&mut self) {
        self.restore();
    }
}

fn restore_terminal_state<O: TerminalOps>(ops: &mut O, state: RestoreState) {
    if state.cursor_hidden {
        let _ = ops.apply(TerminalStep::ShowCursor);
    }
    if state.bracketed_paste {
        let _ = ops.apply(TerminalStep::DisableBracketedPaste);
    }
    if state.mouse_capture {
        let _ = ops.apply(TerminalStep::DisableMouseCapture);
    }
    if state.alternate_screen {
        let _ = ops.apply(TerminalStep::LeaveAlternateScreen);
    }
    if state.raw_mode {
        let _ = ops.apply(TerminalStep::DisableRawMode);
    }
    if state.title_pushed {
        let _ = ops.set_title(DEFAULT_TERMINAL_TITLE);
        let _ = ops.apply(TerminalStep::PopTitle);
    }
}

pub fn restore_terminal<W: Write>(writer: &mut W) -> io::Result<()> {
    execute!(
        writer,
        Show,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::AssertUnwindSafe;
    use std::sync::{Arc, Mutex};

    struct TestGuard<'a> {
        output: &'a mut Vec<u8>,
    }

    impl Drop for TestGuard<'_> {
        fn drop(&mut self) {
            restore_terminal(self.output).unwrap();
        }
    }

    #[derive(Clone)]
    struct FakeTerminalOps {
        calls: Arc<Mutex<Vec<TerminalStep>>>,
        fail_on: Option<TerminalStep>,
    }

    impl TerminalOps for FakeTerminalOps {
        fn apply(&mut self, step: TerminalStep) -> io::Result<()> {
            self.calls.lock().unwrap().push(step);
            if self.fail_on == Some(step) {
                Err(io::Error::new(io::ErrorKind::Other, "terminal step failed"))
            } else {
                Ok(())
            }
        }

        fn set_title(&mut self, _title: &str) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn terminal_guard_restores_on_panic() {
        let mut output = Vec::new();
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = TestGuard {
                output: &mut output,
            };
            panic!("boom");
        }));
        assert!(result.is_err());
        let rendered = String::from_utf8_lossy(&output);
        assert!(rendered.contains("?1049l"));
        assert!(rendered.contains("?2004l"));
        assert!(rendered.contains("?1000l") || rendered.contains("?1002l"));
    }

    #[test]
    fn partial_init_failure_after_raw_mode_restores_raw_mode() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        {
            let ops = FakeTerminalOps {
                calls: calls.clone(),
                fail_on: Some(TerminalStep::EnterAlternateScreen),
            };
            let mut guard = TerminalRestoreGuard::new(ops, TerminalMode::AlternateScreen);
            assert!(guard.initialize().is_err());
        }
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                TerminalStep::EnableRawMode,
                TerminalStep::EnterAlternateScreen,
                TerminalStep::DisableRawMode,
            ]
        );
    }

    #[test]
    fn partial_init_failure_after_bracketed_paste_disables_it() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        {
            let ops = FakeTerminalOps {
                calls: calls.clone(),
                fail_on: Some(TerminalStep::HideCursor),
            };
            let mut guard = TerminalRestoreGuard::new(ops, TerminalMode::Inline);
            assert!(guard.initialize().is_err());
        }
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                TerminalStep::EnableRawMode,
                TerminalStep::EnableMouseCapture,
                TerminalStep::EnableBracketedPaste,
                TerminalStep::HideCursor,
                TerminalStep::DisableBracketedPaste,
                TerminalStep::DisableMouseCapture,
                TerminalStep::DisableRawMode,
            ]
        );
    }

    #[test]
    fn alternate_mode_enters_and_leaves_alt_screen() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        {
            let ops = FakeTerminalOps {
                calls: calls.clone(),
                fail_on: None,
            };
            let mut guard = TerminalRestoreGuard::new(ops, TerminalMode::AlternateScreen);
            guard.initialize().unwrap();
        }
        let calls = calls.lock().unwrap().clone();
        assert!(calls.contains(&TerminalStep::EnterAlternateScreen));
        assert!(calls.contains(&TerminalStep::LeaveAlternateScreen));
        assert!(calls.contains(&TerminalStep::EnableBracketedPaste));
        assert!(calls.contains(&TerminalStep::DisableBracketedPaste));
    }

    #[test]
    fn inline_mode_does_not_enter_or_leave_alternate_screen() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        {
            let ops = FakeTerminalOps {
                calls: calls.clone(),
                fail_on: None,
            };
            let mut guard = TerminalRestoreGuard::new(ops, TerminalMode::Inline);
            guard.initialize().unwrap();
        }
        let calls = calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            vec![
                TerminalStep::EnableRawMode,
                TerminalStep::EnableMouseCapture,
                TerminalStep::EnableBracketedPaste,
                TerminalStep::HideCursor,
                TerminalStep::ShowCursor,
                TerminalStep::DisableBracketedPaste,
                TerminalStep::DisableMouseCapture,
                TerminalStep::DisableRawMode,
            ]
        );
    }

    #[test]
    fn terminal_mode_env_fallback_is_truthy_only() {
        assert!(is_truthy("1"));
        assert!(is_truthy("true"));
        assert!(!is_truthy("0"));
        assert!(!is_truthy("false"));
    }

    #[test]
    fn terminal_title_formats_project_and_status() {
        assert_eq!(
            terminal_title(Some("demo"), "generating"),
            "refact · demo · generating"
        );
        assert_eq!(
            terminal_title(Some(" demo\nproject "), " idle\tready "),
            "refact · demo project · idle ready"
        );
        assert_eq!(terminal_title(None, "idle"), "refact · no project · idle");
    }

    #[test]
    fn terminal_title_truncates_long_project_at_char_boundary() {
        let title = terminal_title(Some(&format!("{}é", "a".repeat(120))), "generating");
        assert_eq!(title.chars().count(), MAX_TERMINAL_TITLE_CHARS);
        assert!(title.ends_with('…'));
        assert!(title.starts_with("refact · "));
    }

    #[test]
    fn terminal_title_config_defaults_on_and_respects_config_gate() {
        assert!(TerminalTitleConfig::from_config_content(None, true).active());
        assert!(!TerminalTitleConfig::from_config_content(None, false).active());
        assert!(
            !TerminalTitleConfig::from_config_content(Some("terminal_title = false"), true)
                .active()
        );
        assert!(
            TerminalTitleConfig::from_config_content(Some("[terminal]\ntitle = true"), true)
                .active()
        );
        assert!(
            !TerminalTitleConfig::from_config_content(Some("[terminal]\ntitle = false"), true)
                .active()
        );
    }

    #[test]
    fn osc2_title_sequence_removes_control_bytes() {
        assert_eq!(
            osc2_title_sequence("demo\nproject"),
            b"\x1b]2;demo project\x07"
        );
    }

    #[test]
    fn title_guard_pushes_and_restores_title_when_enabled() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        {
            let ops = FakeTerminalOps {
                calls: calls.clone(),
                fail_on: None,
            };
            let mut guard = TerminalRestoreGuard::new_with_title_config(
                ops,
                TerminalMode::Inline,
                TerminalTitleConfig::from_config_content(None, true),
            );
            guard.initialize().unwrap();
        }
        let calls = calls.lock().unwrap().clone();
        assert_eq!(calls.first(), Some(&TerminalStep::PushTitle));
        assert!(calls.contains(&TerminalStep::PopTitle));
    }
}
