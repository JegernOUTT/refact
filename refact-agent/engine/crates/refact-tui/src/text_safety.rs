use serde_json::{Map, Value};
use unicode_segmentation::UnicodeSegmentation;

pub fn sanitize_tool_text(text: impl AsRef<str>) -> String {
    let text = text.as_ref();
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == 0x1b {
            index = skip_escape(bytes, index + 1);
            continue;
        }

        let ch = text[index..]
            .chars()
            .next()
            .expect("index is always on a UTF-8 boundary");
        let next = index + ch.len_utf8();

        match ch {
            '\u{009b}' => {
                index = skip_csi(bytes, next);
            }
            '\u{0090}' | '\u{0098}' | '\u{009d}' | '\u{009e}' | '\u{009f}' => {
                index = skip_control_string(bytes, next);
            }
            '\u{009c}' => {
                index = next;
            }
            '\n' => {
                out.push('\n');
                index = next;
            }
            ch if ch.is_control() => {
                out.push(' ');
                index = next;
            }
            ch => {
                out.push(ch);
                index = next;
            }
        }
    }

    out
}

pub fn sanitize_tool_inline(text: impl AsRef<str>) -> String {
    sanitize_tool_text(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn compact_tool_preview(text: &str, max_graphemes: usize) -> String {
    let compact = sanitize_tool_inline(text);
    truncate_graphemes(&compact, max_graphemes).0
}

pub fn truncate_graphemes(text: &str, max_graphemes: usize) -> (String, bool) {
    let mut graphemes = text.graphemes(true);
    let mut out = String::new();
    for _ in 0..max_graphemes {
        let Some(grapheme) = graphemes.next() else {
            return (text.to_string(), false);
        };
        out.push_str(grapheme);
    }
    if graphemes.next().is_none() {
        return (text.to_string(), false);
    }
    out.push('…');
    (out, true)
}

pub fn sanitize_json_strings(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(sanitize_tool_text(text)),
        Value::Array(values) => Value::Array(values.iter().map(sanitize_json_strings).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (sanitize_tool_inline(key), sanitize_json_strings(value)))
                .collect::<Map<_, _>>(),
        ),
        value => value.clone(),
    }
}

fn skip_escape(bytes: &[u8], index: usize) -> usize {
    if index >= bytes.len() {
        return index;
    }
    match bytes[index] {
        b'[' => skip_csi(bytes, index + 1),
        b']' | b'P' | b'^' | b'_' | b'X' => skip_control_string(bytes, index + 1),
        0x20..=0x2f => (index + 2).min(bytes.len()),
        _ => (index + 1).min(bytes.len()),
    }
}

fn skip_csi(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        let byte = bytes[index];
        index += 1;
        if (0x40..=0x7e).contains(&byte) {
            break;
        }
    }
    index
}

fn skip_control_string(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            0x07 => return index + 1,
            0xc2 if bytes.get(index + 1) == Some(&0x9c) => return index + 2,
            0x1b if bytes.get(index + 1) == Some(&b'\\') => return index + 2,
            _ => index += 1,
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_terminal_escape_sequences_and_control_chars() {
        let input = "ok\x1b]0;pwned\x07\x1b[2J\u{009b}31mbad\u{009c}\rnext\nline";
        let output = sanitize_tool_text(input);
        assert!(!output.contains('\x1b'));
        assert!(!output.contains('\u{009b}'));
        assert!(!output.contains('\u{009c}'));
        assert_eq!(output, "okbad next\nline");
    }

    #[test]
    fn truncates_on_grapheme_boundaries() {
        let family = "👨‍👩‍👧‍👦";
        let (truncated, was_truncated) = truncate_graphemes(&format!("ab{family}cd"), 3);
        assert!(was_truncated);
        assert_eq!(truncated, format!("ab{family}…"));
    }

    #[test]
    fn sanitizes_json_string_values() {
        let value =
            sanitize_json_strings(&json!({"command\x1b[31m": "echo\x1b[2J", "items": ["a\x07b"]}));
        assert_eq!(value.get("command").unwrap(), "echo");
        assert_eq!(value["command"], "echo");
        assert_eq!(value["items"][0], "a b");
    }
}
