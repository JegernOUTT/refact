use refact_chat_api::PythonEngineItem;
use serde_json::Value;
use std::sync::LazyLock;

pub const MASK: &str = "***";
pub const ITEM_VALUE_MAX_BYTES: usize = 16 * 1024;

/// Words that make a field name secret wherever they appear in it.
const SECRET_WORDS: &[&str] = &[
    "apikey",
    "secret",
    "secrets",
    "password",
    "passwords",
    "passwd",
    "authorization",
    "cookie",
    "cookies",
    "credential",
    "credentials",
    "bearer",
];

/// Words that name a secret only as the last word of a field name: `api_token`, `auth`,
/// `github_pat`, `db_dsn` are secrets; `max_tokens`, `token_count`, `auth_type`, `author` or
/// `pattern` are not.
const SECRET_LAST_WORDS: &[&str] = &["token", "auth", "pat", "dsn"];

/// Words before a final `key` that make it an ordinary key rather than a secret one.
const PLAIN_KEY_QUALIFIERS: &[&str] = &[
    "primary",
    "foreign",
    "sort",
    "partition",
    "hash",
    "cache",
    "map",
    "lookup",
    "group",
    "unique",
    "composite",
    "shortcut",
    "hot",
    "row",
    "idempotency",
    "dedup",
    "routing",
];

/// Lower-case words of a field name, split at separators and at camelCase boundaries.
fn name_words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut words = Vec::new();
    let mut word = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_ascii_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        let boundary = c.is_ascii_uppercase()
            && i > 0
            && (chars[i - 1].is_ascii_lowercase()
                || chars[i - 1].is_ascii_digit()
                || (chars[i - 1].is_ascii_uppercase()
                    && chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase())));
        if boundary && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.push(c.to_ascii_lowercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// A field name that looks like it holds a key, token, secret, password, authorization, cookie,
/// credential, personal access token or connection string. One rule for output fields, argument
/// fields, flags and preview item labels.
pub fn secret_looking(name: &str) -> bool {
    let words = name_words(name);
    let joined: String = words.concat();
    let last = words.last().map(String::as_str);
    words.iter().any(|word| {
        SECRET_WORDS.contains(&word.as_str())
            || word.ends_with("password")
            || word.ends_with("secret")
    }) || last.is_some_and(|word| SECRET_LAST_WORDS.contains(&word) || word.ends_with("token"))
        || (last == Some("key")
            && words.len() > 1
            && !PLAIN_KEY_QUALIFIERS.contains(&words[words.len() - 2].as_str()))
        || joined.contains("connectionstring")
        || joined.contains("connstr")
}

/// Credentials inside a URL of any scheme: `scheme://user:secret@host`, `redis://:secret@host`.
static URL_CREDENTIALS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b([a-z][a-z0-9+.-]*://)[^/@\s:]*:[^/@\s]+@").unwrap()
});

/// The shared text scrubber plus credentials in URLs of any scheme.
pub fn scrub_text(text: &str) -> String {
    let text = URL_CREDENTIALS.replace_all(text, "${1}***@");
    crate::integrations::mcp::redaction::scrub_text(&text, &[])
}

/// Byte ranges of `text` that the text scrubber masks, one per match that changes the text, in
/// the order of its passes; empty exactly when `scrub_text` leaves the text unchanged. Each pass
/// runs on the text the earlier passes left, and its matches are mapped back to `text`.
pub fn secret_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut current = text.to_string();
    let mut origin: Vec<usize> = (0..=text.len()).collect();
    let mut spans = Vec::new();
    let passes = std::iter::once((&*URL_CREDENTIALS, "@")).chain(
        crate::integrations::mcp::redaction::SCRUB_PATTERNS
            .iter()
            .map(|pattern| (pattern, "")),
    );
    for (pattern, tail) in passes {
        let mut next = String::with_capacity(current.len());
        let mut next_origin = Vec::with_capacity(origin.len());
        let mut last = 0;
        for captures in pattern.captures_iter(&current) {
            let whole = captures.get(0).expect("a match has its whole span");
            let start = captures.get(1).map_or(whole.start(), |kept| kept.end());
            let end = whole.end() - tail.len();
            if current[start..end] == *MASK {
                continue;
            }
            spans.push(origin[start]..origin[end]);
            next.push_str(&current[last..start]);
            next_origin.extend_from_slice(&origin[last..start]);
            next.push_str(MASK);
            next_origin.extend(std::iter::repeat_n(origin[start], MASK.len()));
            last = end;
        }
        next.push_str(&current[last..]);
        next_origin.extend_from_slice(&origin[last..]);
        current = next;
        origin = next_origin;
    }
    spans
}

/// Generic scrubber: string values under secret-looking field names become `***`, every other
/// string goes through the shared text scrubber (bearer tokens, URL credentials, secret flags).
/// Numbers and booleans stay, so counts such as `python_max_llm_tokens` remain readable.
pub fn scrub(value: &mut Value) {
    scrub_inner(value, false);
}

fn scrub_inner(value: &mut Value, secret: bool) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                scrub_inner(child, secret || secret_looking(key));
            }
        }
        Value::Array(items) => {
            for i in 1..items.len() {
                let follows_flag = items[i - 1].as_str().is_some_and(|flag| {
                    flag.starts_with('-')
                        && !flag.contains('=')
                        && secret_looking(flag.trim_start_matches('-'))
                });
                if follows_flag && items[i].as_str().is_some_and(|v| !v.starts_with('-')) {
                    items[i] = Value::String(MASK.into());
                }
            }
            for child in items.iter_mut() {
                scrub_inner(child, secret);
            }
        }
        Value::String(text) if secret && !text.is_empty() => *text = MASK.into(),
        Value::String(text) => {
            let scrubbed = scrub_text(text);
            if scrubbed != *text {
                *text = scrubbed;
            }
        }
        _ => {}
    }
}

fn scrubbed_under(label: &str, value: Value) -> Value {
    let mut wrapped = serde_json::json!({ label: value });
    scrub(&mut wrapped);
    wrapped[label].take()
}

/// The key a value is scrubbed under: the label when it is a field name, a neutral key when the
/// label is built from a name a user chose.
fn scrub_key(label: &str, label_is_name: bool) -> &str {
    if label_is_name {
        "value"
    } else {
        label
    }
}

/// One preview item. The old value is masked under its label and cut to 16 KB serialized; the
/// new value is kept as given, and the pipeline refuses the call when it would be masked or cut
/// (`new_value_refusal`), so a card never shows a value other than the one applied.
pub fn item(label: &str, old: Option<Value>, new: Option<Value>) -> PythonEngineItem {
    build(label, old, new, false)
}

/// An item whose label is built from a name a user chose (zone, provider, model, tool, hook
/// event, path) rather than a configuration field name: the secret-name rule is not applied to
/// the label, the values are still scrubbed for credentials inside them.
pub fn named_item(label: &str, old: Option<Value>, new: Option<Value>) -> PythonEngineItem {
    build(label, old, new, true)
}

fn build(
    label: &str,
    old: Option<Value>,
    new: Option<Value>,
    label_is_name: bool,
) -> PythonEngineItem {
    let mut truncated = false;
    let old = old.map(|value| {
        let value = scrubbed_under(scrub_key(label, label_is_name), value);
        let text = serde_json::to_string(&value).unwrap_or_default();
        if text.len() <= ITEM_VALUE_MAX_BYTES {
            value
        } else {
            truncated = true;
            let mut end = ITEM_VALUE_MAX_BYTES - 64;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            Value::String(format!("{}…", &text[..end]))
        }
    });
    PythonEngineItem {
        label: label.into(),
        old,
        new,
        truncated,
        label_is_name,
    }
}

/// Why the card cannot carry this item's new value: it looks like a secret (masking would change
/// it) or it is larger than one card value may be.
pub fn new_value_refusal(item: &PythonEngineItem) -> Option<String> {
    let new = item.new.as_ref()?;
    if scrubbed_under(scrub_key(&item.label, item.label_is_name), new.clone()) != *new {
        return Some(format!(
            "the change would set {} to a value that looks like a secret; scripts cannot set secrets, set it in Settings instead",
            item.label
        ));
    }
    let size = serde_json::to_string(new).map_or(0, |text| text.len());
    (size > ITEM_VALUE_MAX_BYTES).then(|| {
        format!(
            "the new value of {} is {size} bytes, too large to review on an approval card (limit {ITEM_VALUE_MAX_BYTES} bytes)",
            item.label
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_names_and_ordinary_names() {
        for name in [
            "token",
            "api_token",
            "access_token",
            "refresh_token",
            "bearer_token",
            "accessToken",
            "githubtoken",
            "API_TOKEN",
            "secret",
            "client_secret",
            "password",
            "db_password",
            "credential",
            "credentials",
            "authorization",
            "Authorization",
            "api_key",
            "apiKey",
            "apikey",
            "x-api-key",
            "private_key",
            "auth",
            "proxy_auth",
            "github_pat",
            "db_dsn",
            "cookie",
            "connection_string",
        ] {
            assert!(secret_looking(name), "{name} should be secret-looking");
        }
        for name in [
            "image_token_mode",
            "max_tokens",
            "tokens_in",
            "token_count",
            "token_budget",
            "tokenizer",
            "keyboard",
            "hotkey",
            "key_bindings",
            "primary_key",
            "key",
            "key_id",
            "author",
            "auth_type",
            "path",
            "pattern",
            "sid",
            "count_tokens",
        ] {
            assert!(!secret_looking(name), "{name} should not be secret-looking");
        }
    }

    #[test]
    fn secret_spans_cover_what_the_scrubber_masks_and_nothing_else() {
        for text in ["plain text about tokens and keys", "", "Bearer ***"] {
            assert_eq!(scrub_text(text), text, "{text}");
            assert!(secret_spans(text).is_empty(), "{text}");
        }
        for (text, secret) in [
            ("run X_TOKEN=abc-env srv", "abc-env"),
            ("use Bearer xyz123 now", "xyz123"),
            ("db postgres://u:pw@h/db here", "u:pw"),
            ("https://h/p?api_key=qk1&x=1", "qk1"),
            ("srv --api-key sk-flag1 --verbose", "sk-flag1"),
        ] {
            let spans = secret_spans(text);
            assert_eq!(spans.len(), 1, "{text}");
            let span = spans[0].clone();
            let at = text.find(secret).unwrap();
            assert!(
                span.start <= at && at + secret.len() <= span.end,
                "{text}: {span:?}"
            );
            assert!(!text[..span.start].contains(secret), "{text}");
        }
        let star = "token=*";
        assert_ne!(scrub_text(star), star);
        assert_eq!(secret_spans(star), vec![6..star.len()]);
        let two = "a TOKEN=x b TOKEN=y c";
        let spans = secret_spans(two);
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert_eq!(&two[spans[0].clone()], "x");
        assert_eq!(&two[spans[1].clone()], "y");
        let unicode = "é TOKEN=ж é";
        let span = secret_spans(unicode)[0].clone();
        assert!(unicode.is_char_boundary(span.start) && unicode.is_char_boundary(span.end));
        assert_eq!(&unicode[span], "ж");
    }

    /// T19: matches are reported one by one in the text they were found in, so an edit between
    /// two untouched secrets keeps both spans where they were.
    #[test]
    fn secret_spans_are_the_individual_matches_of_every_pass() {
        let text = "X_TOKEN=first\nkeep this sentence\nZ_TOKEN=second db postgres://u:pw@h/x";
        let spans = secret_spans(text);
        let masked: Vec<&str> = spans.iter().map(|span| &text[span.clone()]).collect();
        assert_eq!(masked, ["u:pw", "first", "second"], "{spans:?}");
        let middle = text.find("keep").unwrap();
        assert!(spans
            .iter()
            .all(|span| span.end <= middle || span.start >= middle + "keep this sentence".len()));
    }

    #[test]
    fn a_new_value_under_an_ordinary_name_is_carried_and_a_secret_one_is_refused() {
        let ordinary = item("image_token_mode", None, Some(serde_json::json!("auto")));
        assert_eq!(new_value_refusal(&ordinary), None);
        let secret = item("api_token", None, Some(serde_json::json!("abc")));
        assert!(new_value_refusal(&secret).unwrap().contains("api_token"));
    }

    #[test]
    fn a_label_built_from_a_name_does_not_make_the_value_secret_but_credentials_inside_still_do() {
        for label in [
            "secret.patterns",
            "zone_secret.patterns",
            "credentials",
            "api_token",
        ] {
            let named = named_item(
                label,
                Some(serde_json::json!("old")),
                Some(serde_json::json!("src/**")),
            );
            assert_eq!(named.old, Some(serde_json::json!("old")));
            assert_eq!(new_value_refusal(&named), None, "{label}");
        }
        let with_credentials = named_item(
            "credentials.send_to",
            Some(serde_json::json!("postgres://u:pw@h/db")),
            Some(serde_json::json!("postgres://u:pw@h/db")),
        );
        assert_eq!(
            with_credentials.old,
            Some(serde_json::json!("postgres://***@h/db"))
        );
        assert!(new_value_refusal(&with_credentials).is_some());
        let field = item(
            "zone_secret.patterns",
            None,
            Some(serde_json::json!("src/**")),
        );
        assert!(new_value_refusal(&field).is_some());
    }
}
