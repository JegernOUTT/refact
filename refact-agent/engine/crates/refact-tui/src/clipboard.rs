use std::io::{self, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

pub const MAX_OSC52_PAYLOAD_BYTES: usize = 100 * 1024;
pub const MAX_OSC52_COPY_BYTES: usize = (MAX_OSC52_PAYLOAD_BYTES / 4) * 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardCopyReport {
    pub original_bytes: usize,
    pub copied_bytes: usize,
    pub truncated: bool,
}

pub fn tmux_passthrough_enabled_from_env() -> bool {
    std::env::var_os("TMUX").is_some_and(|value| !value.is_empty())
}

pub fn osc52_sequence(text: &str, tmux_passthrough: bool) -> (Vec<u8>, ClipboardCopyReport) {
    let original_bytes = text.len();
    let copied = truncate_to_osc52_limit(text);
    let copied_bytes = copied.len();
    let encoded = STANDARD.encode(copied.as_bytes());
    let osc = format!("\x1b]52;c;{encoded}\x07");
    let bytes = if tmux_passthrough {
        format!("\x1bPtmux;{}\x1b\\", osc.replacen('\x1b', "\x1b\x1b", 1)).into_bytes()
    } else {
        osc.into_bytes()
    };
    (
        bytes,
        ClipboardCopyReport {
            original_bytes,
            copied_bytes,
            truncated: copied_bytes < original_bytes,
        },
    )
}

pub fn write_osc52_copy<W: Write>(
    writer: &mut W,
    text: &str,
    tmux_passthrough: bool,
) -> io::Result<ClipboardCopyReport> {
    let (bytes, report) = osc52_sequence(text, tmux_passthrough);
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(report)
}

fn truncate_to_osc52_limit(text: &str) -> &str {
    if text.len() <= MAX_OSC52_COPY_BYTES {
        return text;
    }
    let mut end = MAX_OSC52_COPY_BYTES;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_sequence_encodes_base64_payload() {
        let (bytes, report) = osc52_sequence("hi", false);

        assert_eq!(bytes, b"\x1b]52;c;aGk=\x07");
        assert_eq!(
            report,
            ClipboardCopyReport {
                original_bytes: 2,
                copied_bytes: 2,
                truncated: false,
            }
        );
    }

    #[test]
    fn osc52_sequence_wraps_for_tmux_passthrough() {
        let (bytes, report) = osc52_sequence("hi", true);

        assert_eq!(bytes, b"\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\");
        assert!(!report.truncated);
    }

    #[test]
    fn osc52_sequence_truncates_at_utf8_boundary() {
        let input = format!("{}é", "a".repeat(MAX_OSC52_COPY_BYTES - 1));
        let (bytes, report) = osc52_sequence(&input, false);

        assert!(report.truncated);
        assert_eq!(report.original_bytes, MAX_OSC52_COPY_BYTES + 1);
        assert_eq!(report.copied_bytes, MAX_OSC52_COPY_BYTES - 1);
        let copied = "a".repeat(MAX_OSC52_COPY_BYTES - 1);
        let expected = format!("\x1b]52;c;{}\x07", STANDARD.encode(copied.as_bytes()));
        assert_eq!(bytes, expected.as_bytes());
    }
}
