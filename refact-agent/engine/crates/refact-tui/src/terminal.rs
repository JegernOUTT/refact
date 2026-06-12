use std::io::{self, Write};
use std::panic;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode as crossterm_disable_raw_mode, enable_raw_mode as crossterm_enable_raw_mode,
    EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

pub type RefactTerminal = Terminal<CrosstermBackend<io::Stdout>>;

pub struct TerminalSession {
    terminal: RefactTerminal,
    guard: TerminalRestoreGuard<CrosstermTerminalOps<io::Stdout>>,
}

impl TerminalSession {
    pub fn start() -> io::Result<Self> {
        let mut guard = TerminalRestoreGuard::new(CrosstermTerminalOps::new(io::stdout()));
        guard.initialize()?;
        let previous_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            let mut ops = CrosstermTerminalOps::new(io::stdout());
            restore_terminal_state(&mut ops, true, true, true, true);
            previous_hook(panic_info);
        }));
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal, guard })
    }

    pub fn terminal_mut(&mut self) -> &mut RefactTerminal {
        &mut self.terminal
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.guard.restore();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalStep {
    EnableRawMode,
    EnterAlternateScreen,
    EnableMouseCapture,
    HideCursor,
    ShowCursor,
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
            TerminalStep::HideCursor => execute!(self.writer, Hide),
            TerminalStep::ShowCursor => execute!(self.writer, Show),
            TerminalStep::DisableMouseCapture => execute!(self.writer, DisableMouseCapture),
            TerminalStep::LeaveAlternateScreen => execute!(self.writer, LeaveAlternateScreen),
            TerminalStep::DisableRawMode => crossterm_disable_raw_mode(),
        }
    }
}

struct TerminalRestoreGuard<O: TerminalOps> {
    ops: O,
    active: bool,
    raw_mode: bool,
    alternate_screen: bool,
    mouse_capture: bool,
    cursor_hidden: bool,
}

impl<O: TerminalOps> TerminalRestoreGuard<O> {
    fn new(ops: O) -> Self {
        Self {
            ops,
            active: true,
            raw_mode: false,
            alternate_screen: false,
            mouse_capture: false,
            cursor_hidden: false,
        }
    }

    fn initialize(&mut self) -> io::Result<()> {
        self.apply_start_step(TerminalStep::EnableRawMode)?;
        self.apply_start_step(TerminalStep::EnterAlternateScreen)?;
        self.apply_start_step(TerminalStep::EnableMouseCapture)?;
        self.apply_start_step(TerminalStep::HideCursor)
    }

    fn apply_start_step(&mut self, step: TerminalStep) -> io::Result<()> {
        self.ops.apply(step)?;
        match step {
            TerminalStep::EnableRawMode => self.raw_mode = true,
            TerminalStep::EnterAlternateScreen => self.alternate_screen = true,
            TerminalStep::EnableMouseCapture => self.mouse_capture = true,
            TerminalStep::HideCursor => self.cursor_hidden = true,
            TerminalStep::ShowCursor
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
        restore_terminal_state(
            &mut self.ops,
            self.raw_mode,
            self.alternate_screen,
            self.mouse_capture,
            self.cursor_hidden,
        );
        self.raw_mode = false;
        self.alternate_screen = false;
        self.mouse_capture = false;
        self.cursor_hidden = false;
    }
}

impl<O: TerminalOps> Drop for TerminalRestoreGuard<O> {
    fn drop(&mut self) {
        self.restore();
    }
}

fn restore_terminal_state<O: TerminalOps>(
    ops: &mut O,
    raw_mode: bool,
    alternate_screen: bool,
    mouse_capture: bool,
    cursor_hidden: bool,
) {
    if cursor_hidden {
        let _ = ops.apply(TerminalStep::ShowCursor);
    }
    if mouse_capture {
        let _ = ops.apply(TerminalStep::DisableMouseCapture);
    }
    if alternate_screen {
        let _ = ops.apply(TerminalStep::LeaveAlternateScreen);
    }
    if raw_mode {
        let _ = ops.apply(TerminalStep::DisableRawMode);
    }
}

pub fn restore_terminal<W: Write>(writer: &mut W) -> io::Result<()> {
    execute!(writer, Show, DisableMouseCapture, LeaveAlternateScreen)
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
            let mut guard = TerminalRestoreGuard::new(ops);
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
}
