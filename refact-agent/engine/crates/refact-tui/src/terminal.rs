use std::io::{self, IsTerminal, Write};
use std::panic;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};
#[cfg(unix)]
use std::sync::OnceLock;
#[cfg(unix)]
use std::thread;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode as crossterm_disable_raw_mode, enable_raw_mode as crossterm_enable_raw_mode,
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::{Terminal, TerminalOptions, Viewport};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

pub type RefactTerminal = Terminal<StdoutBackend>;

pub struct StdoutBackend<W: Write = io::Stdout> {
    inner: CrosstermBackend<W>,
    cursor_position: Position,
}

impl StdoutBackend<io::Stdout> {
    fn new(cursor_position: Position) -> Self {
        Self::with_writer(io::stdout(), cursor_position)
    }
}

impl<W: Write> StdoutBackend<W> {
    fn with_writer(writer: W, cursor_position: Position) -> Self {
        Self {
            inner: CrosstermBackend::new(writer),
            cursor_position,
        }
    }
}

impl<W: Write> Write for StdoutBackend<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Write::flush(&mut self.inner)
    }
}

impl<W: Write> Backend for StdoutBackend<W> {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let result = (|| {
            let mut pending = Vec::new();
            let mut open_destination = None::<String>;
            for (x, y, cell) in content {
                let destination =
                    crate::vendored::terminal_hyperlinks::buffer_hyperlink_destination(cell)
                        .and_then(|destination| {
                            crate::vendored::terminal_hyperlinks::web_destination(&destination)
                        });
                if destination != open_destination {
                    self.inner.draw(pending.drain(..))?;
                    if open_destination.is_some() {
                        self.inner.write_all(b"\x1b]8;;\x1b\\")?;
                    }
                    if let Some(destination) = destination.as_deref() {
                        self.inner
                            .write_all(format!("\x1b]8;;{destination}\x1b\\").as_bytes())?;
                    }
                    open_destination = destination;
                }
                pending.push((x, y, cell));
            }
            self.inner.draw(pending.drain(..))?;
            if open_destination.is_some() {
                self.inner.write_all(b"\x1b]8;;\x1b\\")?;
            }
            Ok(())
        })();
        crate::vendored::terminal_hyperlinks::clear_buffer_hyperlinks();
        result
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.cursor_position)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let position = position.into();
        self.cursor_position = position;
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ratatui::backend::ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}

pub const TARGET_FRAME_INTERVAL: Duration = MIN_FRAME_INTERVAL;

const MIN_FRAME_INTERVAL: Duration = Duration::from_nanos(8_333_334);
const INLINE_VIEWPORT_HEIGHT: u16 = 12;
const FALLBACK_ENV: &str = "REFACT_TUI_ALT_SCREEN";
const TITLE_ENV: &str = "REFACT_TUI_TERMINAL_TITLE";
const DEFAULT_TERMINAL_TITLE: &str = "refact";
const MAX_TERMINAL_TITLE_CHARS: usize = 80;
const PUSH_TITLE_SEQUENCE: &[u8] = b"\x1b[22;0t";
const POP_TITLE_SEQUENCE: &[u8] = b"\x1b[23;0t";

#[derive(Clone, Debug)]
pub struct FrameRequester {
    frame_schedule_tx: mpsc::UnboundedSender<Instant>,
}

impl FrameRequester {
    pub fn new() -> (Self, mpsc::Receiver<()>) {
        let (schedule_tx, schedule_rx) = mpsc::unbounded_channel();
        let (draw_tx, draw_rx) = mpsc::channel(1);
        tokio::spawn(FrameScheduler::new(schedule_rx, draw_tx).run());
        (
            Self {
                frame_schedule_tx: schedule_tx,
            },
            draw_rx,
        )
    }

    pub fn schedule_frame(&self) {
        let _ = self.frame_schedule_tx.send(Instant::now());
    }

    pub fn schedule_frame_in(&self, delay: Duration) {
        let _ = self.frame_schedule_tx.send(Instant::now() + delay);
    }
}

#[derive(Debug, Default)]
struct FrameRateLimiter {
    last_emitted_at: Option<Instant>,
}

impl FrameRateLimiter {
    fn clamp_deadline(&self, requested: Instant) -> Instant {
        let Some(last_emitted_at) = self.last_emitted_at else {
            return requested;
        };
        let min_allowed = last_emitted_at
            .checked_add(MIN_FRAME_INTERVAL)
            .unwrap_or(last_emitted_at);
        requested.max(min_allowed)
    }

    fn mark_emitted(&mut self, emitted_at: Instant) {
        self.last_emitted_at = Some(emitted_at);
    }
}

struct FrameScheduler {
    schedule_rx: mpsc::UnboundedReceiver<Instant>,
    draw_tx: mpsc::Sender<()>,
    rate_limiter: FrameRateLimiter,
}

impl FrameScheduler {
    fn new(schedule_rx: mpsc::UnboundedReceiver<Instant>, draw_tx: mpsc::Sender<()>) -> Self {
        Self {
            schedule_rx,
            draw_tx,
            rate_limiter: FrameRateLimiter::default(),
        }
    }

    async fn run(mut self) {
        const IDLE_SLEEP: Duration = Duration::from_secs(60 * 60 * 24 * 365);
        let mut next_deadline: Option<Instant> = None;
        loop {
            let target = next_deadline.unwrap_or_else(|| Instant::now() + IDLE_SLEEP);
            let sleep = tokio::time::sleep_until(target.into());
            tokio::pin!(sleep);
            tokio::select! {
                draw_at = self.schedule_rx.recv() => {
                    let Some(draw_at) = draw_at else {
                        break;
                    };
                    self.note_request(draw_at, &mut next_deadline);
                    while let Ok(draw_at) = self.schedule_rx.try_recv() {
                        self.note_request(draw_at, &mut next_deadline);
                    }
                }
                _ = &mut sleep => {
                    if next_deadline.is_some() {
                        next_deadline = None;
                        self.rate_limiter.mark_emitted(Instant::now());
                        match self.draw_tx.try_send(()) {
                            Ok(()) | Err(TrySendError::Full(_)) => {}
                            Err(TrySendError::Closed(_)) => break,
                        }
                    }
                }
            }
        }
    }

    fn note_request(&mut self, draw_at: Instant, next_deadline: &mut Option<Instant>) {
        let draw_at = self.rate_limiter.clamp_deadline(draw_at);
        *next_deadline = Some(next_deadline.map_or(draw_at, |current| current.min(draw_at)));
    }
}

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
        install_signal_restore_handler()?;
        let mut guard = TerminalRestoreGuard::new_with_title_config(
            CrosstermTerminalOps::new(io::stdout()),
            mode,
            title_config,
        );
        guard.initialize()?;
        install_panic_restore_hook(mode, title_config);
        let terminal = build_terminal(mode)?;
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

    pub fn write_notification(&mut self, bytes: &[u8]) -> io::Result<()> {
        let writer = self.terminal.backend_mut();
        writer.write_all(bytes)?;
        Write::flush(writer)
    }

    pub fn write_inline_images(
        &mut self,
        images: &[crate::terminal_image::InlineImage],
    ) -> io::Result<()> {
        let restore_position = self.terminal.get_cursor_position()?;
        crate::terminal_image::render_inline_images(
            self.terminal.backend_mut(),
            images,
            restore_position,
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

    pub fn clear_for_resize_reflow(&mut self) -> io::Result<()> {
        execute!(self.terminal.backend_mut(), Clear(ClearType::Purge))?;
        self.terminal = build_terminal(self.guard.mode)?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.guard.restore();
    }
}

fn install_panic_restore_hook(mode: TerminalMode, title_config: TerminalTitleConfig) {
    if !terminal_panic_hook_installed() {
        return;
    }
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let mut ops = CrosstermTerminalOps::new(io::stdout());
        restore_terminal_state(&mut ops, RestoreState::started(mode, title_config.active()));
        previous_hook(panic_info);
    }));
}

fn terminal_panic_hook_installed() -> bool {
    static PANIC_RESTORE_HOOK_INSTALLED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    !PANIC_RESTORE_HOOK_INSTALLED.swap(true, std::sync::atomic::Ordering::AcqRel)
}

#[cfg(unix)]
static SIGNAL_RESTORE_WRITE_FD: AtomicI32 = AtomicI32::new(-1);

#[cfg(unix)]
struct SignalRestoreHandler {
    _thread: thread::JoinHandle<()>,
}

#[cfg(unix)]
fn install_signal_restore_handler() -> io::Result<()> {
    static SIGNAL_RESTORE_HANDLER: OnceLock<Result<SignalRestoreHandler, String>> = OnceLock::new();

    match SIGNAL_RESTORE_HANDLER.get_or_init(install_unix_signal_restore_handler) {
        Ok(_) => Ok(()),
        Err(error) => Err(io::Error::other(error.clone())),
    }
}

#[cfg(not(unix))]
fn install_signal_restore_handler() -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn install_unix_signal_restore_handler() -> Result<SignalRestoreHandler, String> {
    let mut pipe_fds = [-1; 2];
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error().to_string());
    }
    let read_fd = pipe_fds[0];
    let write_fd = pipe_fds[1];

    if let Err(error) = set_nonblocking(write_fd) {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return Err(error.to_string());
    }

    let thread = match thread::Builder::new()
        .name("refact-tui-signal-restore".to_string())
        .spawn(move || wait_for_terminal_signal(read_fd))
    {
        Ok(thread) => thread,
        Err(error) => {
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
            return Err(error.to_string());
        }
    };

    if let Err(error) = install_unix_signal_handlers() {
        unsafe {
            libc::close(write_fd);
        }
        return Err(error.to_string());
    }

    SIGNAL_RESTORE_WRITE_FD.store(write_fd, Ordering::Release);
    Ok(SignalRestoreHandler { _thread: thread })
}

#[cfg(unix)]
fn set_nonblocking(fd: libc::c_int) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn install_unix_signal_handlers() -> io::Result<()> {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        if unsafe {
            libc::signal(
                signal,
                terminal_signal_handler as *const () as libc::sighandler_t,
            )
        } == libc::SIG_ERR
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(unix)]
extern "C" fn terminal_signal_handler(signal: libc::c_int) {
    let _ = write_terminal_signal(SIGNAL_RESTORE_WRITE_FD.load(Ordering::Relaxed), signal);
}

#[cfg(unix)]
fn write_terminal_signal(fd: libc::c_int, signal: libc::c_int) -> Option<libc::ssize_t> {
    if fd < 0 {
        return None;
    }

    let signal = signal as u8;
    unsafe { Some(libc::write(fd, std::ptr::addr_of!(signal).cast(), 1)) }
}

#[cfg(unix)]
fn wait_for_terminal_signal(read_fd: libc::c_int) {
    let mut ops = CrosstermTerminalOps::new(io::stdout());
    let Some(signal) = wait_for_terminal_signal_with_ops(read_fd, &mut ops) else {
        return;
    };

    unsafe {
        libc::_exit(128 + i32::from(signal));
    }
}

#[cfg(unix)]
fn wait_for_terminal_signal_with_ops<O: TerminalOps>(
    read_fd: libc::c_int,
    ops: &mut O,
) -> Option<u8> {
    let mut signal = 0u8;
    loop {
        let bytes_read = unsafe { libc::read(read_fd, std::ptr::addr_of_mut!(signal).cast(), 1) };
        if bytes_read == 1 {
            break;
        }
        if bytes_read == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        unsafe {
            libc::close(read_fd);
        }
        return None;
    }
    unsafe {
        libc::close(read_fd);
    }

    let _ = restore_terminal_ops(ops);
    Some(signal)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct RestoreState {
    raw_mode: bool,
    alternate_screen: bool,
    mouse_capture: bool,
    focus_change: bool,
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
            focus_change: true,
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
    EnableFocusChange,
    EnableBracketedPaste,
    HideCursor,
    ShowCursor,
    DisableBracketedPaste,
    DisableFocusChange,
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
            TerminalStep::EnableFocusChange => execute!(self.writer, EnableFocusChange),
            TerminalStep::EnableBracketedPaste => execute!(self.writer, EnableBracketedPaste),
            TerminalStep::HideCursor => execute!(self.writer, Hide),
            TerminalStep::ShowCursor => execute!(self.writer, Show),
            TerminalStep::DisableBracketedPaste => execute!(self.writer, DisableBracketedPaste),
            TerminalStep::DisableFocusChange => execute!(self.writer, DisableFocusChange),
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
        self.apply_start_step(TerminalStep::EnableFocusChange)?;
        self.apply_start_step(TerminalStep::EnableBracketedPaste)?;
        self.apply_start_step(TerminalStep::HideCursor)
    }

    fn apply_start_step(&mut self, step: TerminalStep) -> io::Result<()> {
        self.ops.apply(step)?;
        match step {
            TerminalStep::EnableRawMode => self.state.raw_mode = true,
            TerminalStep::EnterAlternateScreen => self.state.alternate_screen = true,
            TerminalStep::EnableMouseCapture => self.state.mouse_capture = true,
            TerminalStep::EnableFocusChange => self.state.focus_change = true,
            TerminalStep::EnableBracketedPaste => self.state.bracketed_paste = true,
            TerminalStep::HideCursor => self.state.cursor_hidden = true,
            TerminalStep::PushTitle => self.state.title_pushed = true,
            TerminalStep::ShowCursor
            | TerminalStep::DisableBracketedPaste
            | TerminalStep::DisableFocusChange
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
    if state.focus_change {
        let _ = ops.apply(TerminalStep::DisableFocusChange);
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
    restore_terminal_ops(&mut CrosstermTerminalOps::new(writer))
}

fn restore_terminal_ops<O: TerminalOps>(ops: &mut O) -> io::Result<()> {
    let mut first_error = None;
    for step in [
        TerminalStep::ShowCursor,
        TerminalStep::DisableBracketedPaste,
        TerminalStep::DisableFocusChange,
        TerminalStep::DisableMouseCapture,
        TerminalStep::LeaveAlternateScreen,
        TerminalStep::DisableRawMode,
    ] {
        if let Err(error) = ops.apply(step) {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn build_terminal(mode: TerminalMode) -> io::Result<RefactTerminal> {
    let cursor_position = match mode {
        TerminalMode::Inline => probe_startup_cursor_position(),
        TerminalMode::AlternateScreen => Position { x: 0, y: 0 },
    };
    build_terminal_with_cursor(mode, cursor_position)
}

fn build_terminal_with_cursor(
    mode: TerminalMode,
    cursor_position: Position,
) -> io::Result<RefactTerminal> {
    match mode {
        TerminalMode::Inline => Terminal::with_options(
            StdoutBackend::new(cursor_position),
            TerminalOptions {
                viewport: Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
            },
        ),
        TerminalMode::AlternateScreen => Terminal::new(StdoutBackend::new(cursor_position)),
    }
}

fn probe_startup_cursor_position() -> Position {
    match crate::terminal_probe::cursor_position(crate::terminal_probe::DEFAULT_TIMEOUT) {
        Ok(Some(position)) => position,
        Ok(None) => {
            tracing::warn!(
                "initial cursor position probe timed out after {}ms; defaulting to origin",
                crate::terminal_probe::DEFAULT_TIMEOUT.as_millis()
            );
            Position { x: 0, y: 0 }
        }
        Err(err) => {
            tracing::warn!("initial cursor position probe failed ({err}); defaulting to origin");
            Position { x: 0, y: 0 }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use std::panic::AssertUnwindSafe;
    use std::sync::{Arc, Mutex};

    struct TestGuard<'a> {
        output: &'a mut Vec<u8>,
    }

    impl Drop for TestGuard<'_> {
        fn drop(&mut self) {
            let _ = restore_terminal(self.output);
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

    #[cfg(unix)]
    fn unix_pipe() -> [libc::c_int; 2] {
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        fds
    }

    #[cfg(unix)]
    #[test]
    fn signal_handler_write_emits_one_signal_byte() {
        let [read_fd, write_fd] = unix_pipe();

        assert_eq!(write_terminal_signal(write_fd, libc::SIGTERM), Some(1));

        let mut signal = 0u8;
        assert_eq!(
            unsafe { libc::read(read_fd, std::ptr::addr_of_mut!(signal).cast(), 1) },
            1
        );
        assert_eq!(signal, libc::SIGTERM as u8);

        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
    }

    #[cfg(unix)]
    #[test]
    fn signal_handler_write_skips_negative_fd() {
        assert_eq!(write_terminal_signal(-1, libc::SIGINT), None);
    }

    #[cfg(unix)]
    #[test]
    fn signal_handler_write_drops_signal_when_pipe_is_full() {
        let [read_fd, write_fd] = unix_pipe();
        set_nonblocking(write_fd).unwrap();
        let fill = [0u8; 4096];

        loop {
            if unsafe { libc::write(write_fd, fill.as_ptr().cast(), fill.len()) } == -1 {
                assert_eq!(
                    io::Error::last_os_error().raw_os_error(),
                    Some(libc::EAGAIN)
                );
                break;
            }
        }

        assert_eq!(write_terminal_signal(write_fd, libc::SIGINT), Some(-1));
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::EAGAIN)
        );

        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
    }

    #[cfg(unix)]
    #[test]
    fn signal_waiter_restores_terminal_with_mock_ops() {
        let [read_fd, write_fd] = unix_pipe();
        assert_eq!(write_terminal_signal(write_fd, libc::SIGINT), Some(1));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let waiter_calls = calls.clone();

        let signal = std::thread::spawn(move || {
            let mut ops = FakeTerminalOps {
                calls: waiter_calls,
                fail_on: None,
            };
            wait_for_terminal_signal_with_ops(read_fd, &mut ops)
        })
        .join()
        .unwrap();

        assert_eq!(signal, Some(libc::SIGINT as u8));
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                TerminalStep::ShowCursor,
                TerminalStep::DisableBracketedPaste,
                TerminalStep::DisableFocusChange,
                TerminalStep::DisableMouseCapture,
                TerminalStep::LeaveAlternateScreen,
                TerminalStep::DisableRawMode,
            ]
        );

        unsafe {
            libc::close(write_fd);
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
        assert!(rendered.contains("?1004l"));
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
    fn partial_init_failure_after_focus_change_disables_it() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        {
            let ops = FakeTerminalOps {
                calls: calls.clone(),
                fail_on: Some(TerminalStep::EnableBracketedPaste),
            };
            let mut guard = TerminalRestoreGuard::new(ops, TerminalMode::Inline);
            assert!(guard.initialize().is_err());
        }
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                TerminalStep::EnableRawMode,
                TerminalStep::EnableMouseCapture,
                TerminalStep::EnableFocusChange,
                TerminalStep::EnableBracketedPaste,
                TerminalStep::DisableFocusChange,
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
        assert!(calls.contains(&TerminalStep::EnableFocusChange));
        assert!(calls.contains(&TerminalStep::DisableFocusChange));
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
                TerminalStep::EnableFocusChange,
                TerminalStep::EnableBracketedPaste,
                TerminalStep::HideCursor,
                TerminalStep::ShowCursor,
                TerminalStep::DisableBracketedPaste,
                TerminalStep::DisableFocusChange,
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
    fn stdout_backend_uses_bounded_cursor_position() {
        let mut backend = StdoutBackend::new(Position { x: 4, y: 9 });
        assert_eq!(
            Backend::get_cursor_position(&mut backend).unwrap(),
            Position { x: 4, y: 9 }
        );
        assert_eq!(
            Backend::get_cursor_position(&mut backend).unwrap(),
            Position { x: 4, y: 9 }
        );
    }

    #[test]
    fn stdout_backend_emits_osc8_without_mutating_cell_symbols() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        buffer.set_string(0, 0, "link", Style::default());
        let line = crate::vendored::terminal_hyperlinks::HyperlinkLine {
            line: Line::from("link"),
            hyperlinks: vec![crate::vendored::terminal_hyperlinks::TerminalHyperlink {
                columns: 0..4,
                destination: "https://example.com".to_string(),
            }],
        };
        let area = buffer.area;
        crate::vendored::terminal_hyperlinks::mark_buffer_hyperlinks(&buffer, area, &[line], true);

        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut backend = StdoutBackend::with_writer(TestWriter(output.clone()), Position::ORIGIN);
        backend
            .draw(
                buffer
                    .content()
                    .iter()
                    .enumerate()
                    .map(|(index, cell)| (index as u16, 0, cell)),
            )
            .unwrap();

        assert!(buffer
            .content()
            .iter()
            .all(|cell| !cell.symbol().contains('\x1b')));
        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(output.contains("\x1b]8;;https://example.com\x1b\\"));
        assert!(output.contains("\x1b]8;;\x1b\\"));
        assert!(output.contains("link"));
    }

    #[test]
    fn inline_image_protocols_are_written_without_cell_symbols() {
        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut backend = StdoutBackend::with_writer(TestWriter(output.clone()), Position::ORIGIN);
        let image = crate::terminal_image::InlineImage::new(
            crate::terminal_probe::ImageProtocol::Kitty,
            one_pixel_png(),
            "image/png".to_string(),
            Position::ORIGIN,
        );

        crate::terminal_image::render_inline_images(&mut backend, &[image], Position::ORIGIN)
            .unwrap();

        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(output.contains("\x1b_G"));
        assert!(!output.contains("[image:"));
    }

    fn one_pixel_png() -> Vec<u8> {
        let image = image::DynamicImage::new_rgba8(1, 1);
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    struct TestWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for TestWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn resize_rebuild_uses_the_bounded_cursor_position() {
        let cursor_position = Position { x: 4, y: 9 };
        let mut terminal =
            build_terminal_with_cursor(TerminalMode::Inline, cursor_position).unwrap();

        assert_eq!(
            Backend::get_cursor_position(terminal.backend_mut()).unwrap(),
            cursor_position
        );
    }

    #[test]
    fn restore_terminal_disables_raw_mode() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut ops = FakeTerminalOps {
            calls: calls.clone(),
            fail_on: None,
        };

        restore_terminal_ops(&mut ops).unwrap();

        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                TerminalStep::ShowCursor,
                TerminalStep::DisableBracketedPaste,
                TerminalStep::DisableFocusChange,
                TerminalStep::DisableMouseCapture,
                TerminalStep::LeaveAlternateScreen,
                TerminalStep::DisableRawMode,
            ]
        );
    }

    #[test]
    fn restore_terminal_is_idempotent() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut ops = FakeTerminalOps {
            calls: calls.clone(),
            fail_on: None,
        };

        restore_terminal_ops(&mut ops).unwrap();
        restore_terminal_ops(&mut ops).unwrap();

        assert_eq!(calls.lock().unwrap().len(), 12);
        assert_eq!(
            calls.lock().unwrap().last(),
            Some(&TerminalStep::DisableRawMode)
        );
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

    #[test]
    fn frame_rate_limiter_clamps_to_min_interval() {
        let t0 = Instant::now();
        let mut limiter = FrameRateLimiter::default();

        assert_eq!(limiter.clamp_deadline(t0), t0);
        limiter.mark_emitted(t0);

        assert_eq!(
            limiter.clamp_deadline(t0 + Duration::from_millis(1)),
            t0 + MIN_FRAME_INTERVAL
        );
    }

    #[tokio::test]
    async fn frame_requester_coalesces_immediate_requests() {
        let (requester, mut rx) = FrameRequester::new();

        requester.schedule_frame();
        requester.schedule_frame();

        assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .is_some());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn frame_requester_waits_for_delayed_request() {
        let (requester, mut rx) = FrameRequester::new();

        requester.schedule_frame_in(Duration::from_millis(20));

        assert!(tokio::time::timeout(Duration::from_millis(5), rx.recv())
            .await
            .is_err());
        assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .is_some());
    }
}
