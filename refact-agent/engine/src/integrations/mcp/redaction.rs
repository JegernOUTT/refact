use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

pub const REDACTED: &str = "<REDACTED>";

const SECRET_NAME: &str = "token|key|secret|password|passwd|auth|credential|sig|signature|bearer";

/// Export predicate: field names whose scalar values are secrets. `headers` and `env`
/// maps are recursed into and redacted per key, so imports keep their shape.
pub fn is_secret_field(key: &str) -> bool {
    let key = key.to_lowercase();
    [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "authorization",
        "bearer",
        "cookie",
        "credential",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

/// Stricter predicate for chat tool output: every header and env value is secret.
pub fn is_tool_secret_field(key: &str) -> bool {
    let lower = key.to_lowercase();
    is_secret_field(key) || lower.contains("headers") || lower == "env" || lower.contains("_env")
}

fn is_secret_name(name: &str) -> bool {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(&format!("(?i){SECRET_NAME}")).unwrap());
    RE.is_match(name)
}

pub fn redact_secrets(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if is_secret_field(key) && !child.is_object() && !child.is_array() {
                    *child = Value::String(REDACTED.into());
                } else {
                    redact_secrets(child);
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                redact_secrets(child);
            }
        }
        _ => {}
    }
}

/// Tool-output redaction: everything under a secret-looking key, plus values that follow
/// secret-looking flags in arrays.
pub fn redact_tool_secrets(value: &mut Value) {
    redact_tool(value, false)
}

fn redact_tool(value: &mut Value, inherited: bool) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                redact_tool(child, inherited || is_tool_secret_field(key));
            }
        }
        Value::Array(items) => {
            redact_secret_flag_values(items);
            for child in items {
                redact_tool(child, inherited);
            }
        }
        Value::Null => {}
        _ if inherited => *value = Value::String(REDACTED.into()),
        _ => {}
    }
}

/// Redacts the entry following a secret-looking flag, e.g. `["--api-key", "sk-1"]`.
fn redact_secret_flag_values(items: &mut [Value]) {
    for i in 1..items.len() {
        let follows_secret_flag = items[i - 1].as_str().is_some_and(|flag| {
            flag.starts_with('-') && !flag.contains('=') && is_secret_name(flag)
        });
        if follows_secret_flag && items[i].as_str().is_some_and(|v| !v.starts_with('-')) {
            items[i] = Value::String(REDACTED.into());
        }
    }
}

static SCRUB_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r#"(?i)(Bearer\s+)[^\s"',;]+"#.to_string(),
        r"(?i)(Authorization:\s*)[^\r\n]+".to_string(),
        r"(?i)(https?://)[^/@\s]+:[^/@\s]+@".to_string(),
        format!(r#"(?i)([?&][^=&\s"']*(?:{SECRET_NAME})[^=&\s"']*=)[^&\s#"']+"#),
        format!(r#"(?i)((?:^|[\s"'\[,(])-{{0,2}}[A-Za-z0-9_.-]*(?:{SECRET_NAME})[A-Za-z0-9_.-]*=)[^\s&"',;]+"#),
        format!(r#"(?i)((?:^|[\s"'\[,(])-{{1,2}}[A-Za-z0-9_.-]*(?:{SECRET_NAME})[A-Za-z0-9_.-]*\s+)[^\s"',;-][^\s"',;]*"#),
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap())
    .collect()
});

pub fn scrub_text(text: &str, known_values: &[String]) -> String {
    let mut text = text.to_owned();
    let mut values = known_values.to_vec();
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    for value in values.iter().filter(|v| !v.is_empty()) {
        text = text.replace(value, "***");
    }
    for pattern in SCRUB_PATTERNS.iter() {
        text = pattern.replace_all(&text, "${1}***").into_owned();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scrub_text_covers_flags_env_assignments_and_query_params() {
        for (input, secret) in [
            ("srv --api-key sk-flag1 --verbose", "sk-flag1"),
            ("srv --token=tok-eq --x", "tok-eq"),
            ("X_TOKEN=abc-env srv", "abc-env"),
            ("MY_SECRET=s3cr3t ./run", "s3cr3t"),
            ("https://h/p?key=qk1&x=1", "qk1"),
            ("https://h/p?a=1&apikey=qk2", "qk2"),
            ("https://h/p?sig=qk3#frag", "qk3"),
            ("https://h/p?password=qk4", "qk4"),
            ("https://h/p?client_secret=qk5", "qk5"),
            ("token=legacy1 api_key=legacy2", "legacy1"),
            ("token=legacy1 api_key=legacy2", "legacy2"),
        ] {
            let out = scrub_text(input, &[]);
            assert!(!out.contains(secret), "{input} -> {out}");
        }
        assert_eq!(
            scrub_text("https://h/p?x=1&key=v", &[]),
            "https://h/p?x=1&key=***"
        );
        assert_eq!(
            scrub_text("srv --port 8080 --verbose", &[]),
            "srv --port 8080 --verbose"
        );
    }

    #[test]
    fn tool_redaction_scrubs_secret_flag_args_and_env_maps() {
        let mut value = json!({
            "command": "npx",
            "args": ["-y", "srv", "--api-key", "sk-arg", "--port", "80", "--token", "--next"],
            "env": {"PATH": "/bin"},
            "headers": {"X-Custom": "c"},
        });
        redact_tool_secrets(&mut value);
        assert_eq!(
            value["args"],
            json!([
                "-y",
                "srv",
                "--api-key",
                REDACTED,
                "--port",
                "80",
                "--token",
                "--next"
            ])
        );
        assert_eq!(value["env"]["PATH"], REDACTED);
        assert_eq!(value["headers"]["X-Custom"], REDACTED);
    }

    #[test]
    fn export_redaction_keeps_header_and_env_maps() {
        let mut value = json!({
            "env": {"GITHUB_TOKEN": "ghp", "PATH": "/bin"},
            "headers": {"Authorization": "Bearer x", "Accept": "application/json"},
        });
        assert!(!is_secret_field("env") && !is_secret_field("headers"));
        redact_secrets(&mut value);
        assert_eq!(
            value["env"],
            json!({"GITHUB_TOKEN": REDACTED, "PATH": "/bin"})
        );
        assert_eq!(
            value["headers"],
            json!({"Authorization": REDACTED, "Accept": "application/json"})
        );
    }
}
