use regex::Regex;
use std::sync::OnceLock;

pub fn redact_sensitive(text: &str) -> String {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        vec![
            (
                Regex::new(r#"(?i)Bearer\s+[^\s"',]+"#).unwrap(),
                "Bearer [REDACTED]",
            ),
            (
                Regex::new(r"sk-[A-Za-z0-9]{8,}").unwrap(),
                "[REDACTED_SK_TOKEN]",
            ),
            (
                Regex::new(r#"(?i)\bghp_[A-Za-z0-9]{10,}\b"#).unwrap(),
                "[REDACTED_GH_TOKEN]",
            ),
            (
                Regex::new(r#"(?i)\bglpat-[A-Za-z0-9_-]{10,}\b"#).unwrap(),
                "[REDACTED_GL_TOKEN]",
            ),
            (
                Regex::new(
                    r#"(?i)\b(api[_-]?key|apikey|token|secret|password)\s*[:=]\s*[^\s"',;]+"#,
                )
                .unwrap(),
                "$1=[REDACTED]",
            ),
            (
                Regex::new(r#"(?i)Authorization:\s*[^\s"',]+"#).unwrap(),
                "Authorization: [REDACTED]",
            ),
            (
                Regex::new(r#"(?i)(https?://[^\s?#]+)\?[^\s)\]]+"#).unwrap(),
                "$1?[REDACTED]",
            ),
            (
                Regex::new(r#"file://[^\s)\]]+"#).unwrap(),
                "file://[REDACTED_PATH]",
            ),
            (
                Regex::new(r#"[A-Za-z]:\\[^\s)\]]+"#).unwrap(),
                "[REDACTED_PATH]",
            ),
            (
                Regex::new(r#"/(?:Users|home)/[^\s)]+"#).unwrap(),
                "[REDACTED_PATH]",
            ),
        ]
    });

    let mut out = text.to_string();
    for (re, replacement) in patterns {
        out = re.replace_all(&out, *replacement).into_owned();
    }
    out
}

pub fn safe_truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    let mut end = max_len.min(s.len());
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    &s[..end]
}

pub fn is_redaction_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            ',' | ';' | ')' | ']' | '}' | '"' | '\'' | '`' | '<' | '>'
        )
}

/// Returns a prefix window of `text` suitable for redaction scanning, backed off to the
/// last redaction boundary so secret-like tokens are never split mid-token at the window
/// edge. The second tuple element reports whether the input was truncated.
pub fn bounded_redaction_window(text: &str, scan_cap: usize) -> (&str, bool) {
    if text.len() <= scan_cap {
        return (text, false);
    }

    let prefix = safe_truncate(text, scan_cap);
    if prefix
        .chars()
        .last()
        .map(is_redaction_boundary)
        .unwrap_or(true)
        || text[prefix.len()..]
            .chars()
            .next()
            .map(is_redaction_boundary)
            .unwrap_or(false)
    {
        return (prefix, true);
    }

    let end = prefix
        .char_indices()
        .rev()
        .find(|(_, ch)| is_redaction_boundary(*ch))
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);

    (&prefix[..end], true)
}
