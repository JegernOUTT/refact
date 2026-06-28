use std::time::Duration;

use ratatui::layout::Position;

pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_millis(100);

#[cfg(unix)]
pub(crate) fn cursor_position(timeout: Duration) -> std::io::Result<Option<Position>> {
    imp::cursor_position(timeout)
}

#[cfg(not(unix))]
pub(crate) fn cursor_position(_timeout: Duration) -> std::io::Result<Option<Position>> {
    use ratatui::backend::Backend as _;
    let mut backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    Ok(Some(backend.get_cursor_position()?))
}

#[cfg_attr(not(unix), allow(dead_code))]
fn parse_cursor_position(buffer: &[u8]) -> Option<Position> {
    let mut search_start = 0;
    while let Some(rel) = find_subslice(&buffer[search_start..], b"\x1b[") {
        let start = search_start + rel;
        let rest = &buffer[start + 2..];
        if let Some(end) = rest.iter().position(|byte| *byte == b'R') {
            if let Ok(payload) = std::str::from_utf8(&rest[..end]) {
                if let Some((row, col)) = payload.split_once(';') {
                    if let (Ok(row), Ok(col)) = (row.parse::<u16>(), col.parse::<u16>()) {
                        return Some(Position {
                            x: col.saturating_sub(1),
                            y: row.saturating_sub(1),
                        });
                    }
                }
            }
        }
        search_start = start + 2;
    }
    None
}

#[cfg_attr(not(unix), allow(dead_code))]
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|&start| &haystack[start..start + needle.len()] == needle)
}

#[cfg(unix)]
mod imp {
    use std::fs::File;
    use std::fs::OpenOptions;
    use std::io;
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::fd::FromRawFd;
    use std::time::Duration;
    use std::time::Instant;

    use ratatui::layout::Position;

    struct Tty {
        reader: File,
        writer: File,
        original_flags: libc::c_int,
    }

    impl Tty {
        fn open() -> io::Result<Self> {
            let stdio_reader = dup_file(libc::STDIN_FILENO);
            let stdio_writer = dup_file(libc::STDOUT_FILENO);
            match (stdio_reader, stdio_writer) {
                (Ok(reader), Ok(writer)) => Self::new(reader, writer),
                _ => {
                    let reader = OpenOptions::new().read(true).open("/dev/tty")?;
                    let writer = OpenOptions::new().write(true).open("/dev/tty")?;
                    Self::new(reader, writer)
                }
            }
        }

        fn new(reader: File, writer: File) -> io::Result<Self> {
            let fd = reader.as_raw_fd();
            let original_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if original_flags == -1 {
                return Err(io::Error::last_os_error());
            }
            if unsafe { libc::fcntl(fd, libc::F_SETFL, original_flags | libc::O_NONBLOCK) } == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                reader,
                writer,
                original_flags,
            })
        }

        fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.writer.write_all(bytes)?;
            self.writer.flush()
        }

        fn read_available(&mut self, buffer: &mut Vec<u8>) -> io::Result<()> {
            let mut chunk = [0_u8; 256];
            loop {
                let count = unsafe {
                    libc::read(
                        self.reader.as_raw_fd(),
                        chunk.as_mut_ptr().cast::<libc::c_void>(),
                        chunk.len(),
                    )
                };
                if count > 0 {
                    buffer.extend_from_slice(&chunk[..count as usize]);
                    continue;
                }
                if count == 0 {
                    return Ok(());
                }
                let err = io::Error::last_os_error();
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    return Ok(());
                }
                return Err(err);
            }
        }

        fn poll_readable(&self, timeout: Duration) -> io::Result<bool> {
            let mut fd = libc::pollfd {
                fd: self.reader.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let deadline = Instant::now() + timeout;
            loop {
                let now = Instant::now();
                if now >= deadline {
                    return Ok(false);
                }
                let timeout_ms = deadline
                    .saturating_duration_since(now)
                    .as_millis()
                    .min(libc::c_int::MAX as u128) as libc::c_int;
                let result = unsafe { libc::poll(&mut fd, 1, timeout_ms) };
                if result > 0 {
                    return Ok((fd.revents & libc::POLLIN) != 0);
                }
                if result == 0 {
                    return Ok(false);
                }
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::Interrupted {
                    return Err(err);
                }
            }
        }
    }

    impl Drop for Tty {
        fn drop(&mut self) {
            let _ =
                unsafe { libc::fcntl(self.reader.as_raw_fd(), libc::F_SETFL, self.original_flags) };
        }
    }

    fn dup_file(fd: libc::c_int) -> io::Result<File> {
        let duplicated = unsafe { libc::dup(fd) };
        if duplicated == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(duplicated) })
    }

    pub(super) fn cursor_position(timeout: Duration) -> io::Result<Option<Position>> {
        let mut tty = Tty::open()?;
        tty.write_all(b"\x1b[6n")?;
        read_until(&mut tty, timeout, super::parse_cursor_position)
    }

    fn read_until<T>(
        tty: &mut Tty,
        timeout: Duration,
        mut parse: impl FnMut(&[u8]) -> Option<T>,
    ) -> io::Result<Option<T>> {
        let deadline = Instant::now() + timeout;
        let mut buffer = Vec::new();
        loop {
            tty.read_available(&mut buffer)?;
            if let Some(value) = parse(&buffer) {
                return Ok(Some(value));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            if !tty.poll_readable(deadline.saturating_duration_since(now))? {
                return Ok(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_well_formed_cursor_report() {
        assert_eq!(
            parse_cursor_position(b"\x1b[12;34R"),
            Some(Position { x: 33, y: 11 })
        );
    }

    #[test]
    fn parses_report_with_leading_and_trailing_noise() {
        assert_eq!(
            parse_cursor_position(b"noise\x1b[1;1Rmore"),
            Some(Position { x: 0, y: 0 })
        );
    }

    #[test]
    fn ignores_unterminated_or_invalid_reports() {
        assert_eq!(parse_cursor_position(b""), None);
        assert_eq!(parse_cursor_position(b"\x1b[12;34"), None);
        assert_eq!(parse_cursor_position(b"\x1b[abc;defR"), None);
    }

    #[test]
    fn skips_malformed_escape_before_valid_report() {
        assert_eq!(
            parse_cursor_position(b"\x1b[bad\x1b[2;5R"),
            Some(Position { x: 4, y: 1 })
        );
    }
}
