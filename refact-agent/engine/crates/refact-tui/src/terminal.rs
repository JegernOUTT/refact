use std::io::{self, Write};
use std::panic;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

pub type RefactTerminal = Terminal<CrosstermBackend<io::Stdout>>;

pub struct TerminalSession {
    terminal: RefactTerminal,
    guard: TerminalRestoreGuard,
}

impl TerminalSession {
    pub fn start() -> io::Result<Self> {
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide)?;
        let previous_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            let mut stdout = io::stdout();
            let _ = restore_terminal(&mut stdout);
            let _ = disable_raw_mode();
            previous_hook(panic_info);
        }));
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            guard: TerminalRestoreGuard { active: true },
        })
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

pub struct TerminalRestoreGuard {
    active: bool,
}

impl TerminalRestoreGuard {
    fn restore(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = restore_terminal(&mut stdout);
    }
}

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

pub fn restore_terminal<W: Write>(writer: &mut W) -> io::Result<()> {
    execute!(writer, Show, DisableMouseCapture, LeaveAlternateScreen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::AssertUnwindSafe;

    struct TestGuard<'a> {
        output: &'a mut Vec<u8>,
    }

    impl Drop for TestGuard<'_> {
        fn drop(&mut self) {
            restore_terminal(self.output).unwrap();
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
}
