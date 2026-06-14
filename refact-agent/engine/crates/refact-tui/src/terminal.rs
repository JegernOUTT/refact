use std::io::{self, Write};
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

pub type RefactTerminal = Terminal<CrosstermBackend<io::Stdout>>;

const INLINE_VIEWPORT_HEIGHT: u16 = 12;
const FALLBACK_ENV: &str = "REFACT_TUI_ALT_SCREEN";

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

pub struct TerminalSession {
    terminal: RefactTerminal,
    guard: TerminalRestoreGuard<CrosstermTerminalOps<io::Stdout>>,
}

impl TerminalSession {
    pub fn start() -> io::Result<Self> {
        Self::start_with_mode(TerminalMode::from_env())
    }

    pub fn start_with_mode(mode: TerminalMode) -> io::Result<Self> {
        let mut guard = TerminalRestoreGuard::new(CrosstermTerminalOps::new(io::stdout()), mode);
        guard.initialize()?;
        install_panic_restore_hook(mode);
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
        Ok(Self { terminal, guard })
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

    pub fn suspend(&mut self) {
        self.guard.restore();
    }

    pub fn resume(&mut self) -> io::Result<()> {
        self.guard.resume()?;
        self.terminal.clear()
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.guard.restore();
    }
}

fn install_panic_restore_hook(mode: TerminalMode) {
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let mut ops = CrosstermTerminalOps::new(io::stdout());
        restore_terminal_state(&mut ops, RestoreState::started(mode));
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
}

impl RestoreState {
    fn started(mode: TerminalMode) -> Self {
        Self {
            raw_mode: true,
            alternate_screen: mode == TerminalMode::AlternateScreen,
            mouse_capture: true,
            bracketed_paste: true,
            cursor_hidden: true,
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
}

trait TerminalOps {
    fn apply(&mut self, step: TerminalStep) -> io::Result<()>;
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
        }
    }
}

struct TerminalRestoreGuard<O: TerminalOps> {
    ops: O,
    mode: TerminalMode,
    active: bool,
    state: RestoreState,
}

impl<O: TerminalOps> TerminalRestoreGuard<O> {
    fn new(ops: O, mode: TerminalMode) -> Self {
        Self {
            ops,
            mode,
            active: true,
            state: RestoreState::default(),
        }
    }

    fn initialize(&mut self) -> io::Result<()> {
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
            TerminalStep::ShowCursor
            | TerminalStep::DisableBracketedPaste
            | TerminalStep::DisableMouseCapture
            | TerminalStep::LeaveAlternateScreen
            | TerminalStep::DisableRawMode => {}
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
}
