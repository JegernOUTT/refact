use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityFinding {
    pub rule: String,
    pub severity: Severity,
    pub line: usize,
    pub snippet: String,
}

pub fn sink_names() -> &'static [&'static str] {
    &["execute", "query", "exec", "raw"]
}

pub fn secret_key_names() -> &'static [&'static str] {
    &[
        "password",
        "passwd",
        "secret",
        "api_key",
        "apikey",
        "access_key",
        "aws_access_key",
        "aws_key",
        "token",
        "private_key",
        "aws_secret",
    ]
}

pub fn scan(lang: &str, text: &str) -> Vec<SecurityFinding> {
    let mut findings = Vec::new();
    // Keep byte offsets intact so the same masked source can also be used with
    // tree-sitter node spans below.
    let executable_text = mask_non_executable_source(lang, text);
    let comment_masked_text = mask_comments(lang, text);
    debug_assert_eq!(
        executable_text.len(),
        text.len(),
        "security source masking must preserve byte offsets"
    );
    debug_assert_eq!(comment_masked_text.len(), text.len());

    for (idx, ((line, executable_line), comment_masked_line)) in text
        .lines()
        .zip(executable_text.lines())
        .zip(comment_masked_text.lines())
        .enumerate()
    {
        let line_no = idx + 1;
        let trimmed = line.trim();
        let executable = executable_line.trim();
        let executable_lower = executable.to_ascii_lowercase();

        if has_tls_verify_disabled(executable, &executable_lower) {
            push_finding(
                &mut findings,
                "tls_verify_disabled",
                Severity::High,
                line_no,
                trimmed,
            );
        }

        if let Some((key, value)) = assigned_string_literal_in_executable(line, executable_line) {
            if contains_any(&key.to_ascii_lowercase(), secret_key_names())
                && is_real_secret_literal(&value)
            {
                push_finding(
                    &mut findings,
                    "hardcoded_secret",
                    Severity::Critical,
                    line_no,
                    trimmed,
                );
            }
        }

        if line_has_sql_injection(comment_masked_line, executable, &executable_lower) {
            push_finding(
                &mut findings,
                "sql_injection",
                Severity::High,
                line_no,
                trimmed,
            );
        }

        if line_has_command_injection(lang, executable, &executable_lower) {
            push_finding(
                &mut findings,
                "command_injection",
                Severity::High,
                line_no,
                trimmed,
            );
        }

        if line_has_dangerous_eval(lang, executable, &executable_lower) {
            push_finding(
                &mut findings,
                "dangerous_eval",
                Severity::High,
                line_no,
                trimmed,
            );
        }

        if line_has_weak_crypto(executable, &executable_lower) {
            push_finding(
                &mut findings,
                "weak_crypto",
                Severity::Medium,
                line_no,
                trimmed,
            );
        }

        if line_has_insecure_random(executable, &executable_lower) {
            push_finding(
                &mut findings,
                "insecure_random",
                Severity::Low,
                line_no,
                trimmed,
            );
        }
    }

    if let Some(tree) = refact_codegraph_parsers::parse_tree(lang, text) {
        let root = tree.root_node();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            let kind = node.kind();
            if kind.contains("call") {
                if let Ok(call_text) = node.utf8_text(text.as_bytes()) {
                    let byte_range = node.byte_range();
                    let masked_call_text = executable_text.get(byte_range.clone());
                    debug_assert!(
                        masked_call_text.is_some(),
                        "masked source must be sliceable by every AST byte span"
                    );
                    // Invalid parser spans must not silently suppress a call. Re-mask the
                    // call itself as a conservative, byte-preserving fallback.
                    let fallback;
                    let masked_call_text = if let Some(masked) = masked_call_text {
                        masked
                    } else {
                        fallback = mask_non_executable_source(lang, call_text);
                        fallback.as_str()
                    };
                    let line_no = node.start_position().row + 1;
                    let trimmed = call_text.trim();
                    let masked_trimmed = masked_call_text.trim();
                    let comment_masked_call = mask_comments(lang, call_text);
                    let comment_masked_trimmed = comment_masked_call.trim();
                    let lower = masked_trimmed.to_ascii_lowercase();
                    let callee = callee_name(masked_trimmed);

                    if is_sql_sink(&callee)
                        && call_contains_dynamic_sql(
                            comment_masked_trimmed,
                            masked_trimmed,
                            &comment_masked_trimmed.to_ascii_lowercase(),
                        )
                    {
                        push_finding(
                            &mut findings,
                            "sql_injection",
                            Severity::High,
                            line_no,
                            trimmed,
                        );
                    }

                    let command_call_text =
                        if lang.eq_ignore_ascii_case("rust") || lang.eq_ignore_ascii_case("rs") {
                            masked_trimmed
                        } else {
                            comment_masked_trimmed
                        };
                    if call_has_command_injection(
                        lang,
                        &callee,
                        command_call_text,
                        &command_call_text.to_ascii_lowercase(),
                    ) {
                        push_finding(
                            &mut findings,
                            "command_injection",
                            Severity::High,
                            line_no,
                            trimmed,
                        );
                    }

                    if call_has_dangerous_eval(lang, &callee, masked_trimmed, &lower) {
                        push_finding(
                            &mut findings,
                            "dangerous_eval",
                            Severity::High,
                            line_no,
                            trimmed,
                        );
                    }

                    if is_weak_crypto_callee(&callee, &lower) {
                        push_finding(
                            &mut findings,
                            "weak_crypto",
                            Severity::Medium,
                            line_no,
                            trimmed,
                        );
                    }
                }
            }

            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
    }

    findings.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then(a.rule.cmp(&b.rule))
            .then(a.snippet.len().cmp(&b.snippet.len()))
            .then(a.snippet.cmp(&b.snippet))
    });
    findings.dedup_by(|a, b| a.rule == b.rule && a.line == b.line);
    findings
}

fn push_finding(
    findings: &mut Vec<SecurityFinding>,
    rule: &str,
    severity: Severity,
    line: usize,
    snippet: &str,
) {
    findings.push(SecurityFinding {
        rule: rule.to_string(),
        severity,
        line,
        snippet: trim_snippet(snippet),
    });
}

fn trim_snippet(s: &str) -> String {
    s.trim().chars().take(160).collect()
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn has_tls_verify_disabled(line: &str, lower: &str) -> bool {
    lower.contains("verify=false")
        || lower.contains("verify = false")
        || lower.contains("rejectunauthorized: false")
        || lower.contains("rejectunauthorized:false")
        || line.contains("InsecureSkipVerify: true")
        || line.contains("InsecureSkipVerify:true")
}

fn assigned_string_literal_in_executable(
    original_line: &str,
    executable_line: &str,
) -> Option<(String, String)> {
    let eq = find_assignment_equals(executable_line)?;
    let key = last_identifier(executable_line[..eq].trim_end())?;
    let value = first_string_literal(original_line[eq + 1..].trim_start())?;
    Some((key, value))
}

fn find_assignment_equals(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'=' {
            let prev = if i > 0 { bytes[i - 1] } else { b' ' };
            let next = if i + 1 < bytes.len() {
                bytes[i + 1]
            } else {
                b' '
            };
            if prev != b'='
                && prev != b'!'
                && prev != b'<'
                && prev != b'>'
                && next != b'='
                && next != b'>'
            {
                return Some(i);
            }
        }
    }
    None
}

fn last_identifier(s: &str) -> Option<String> {
    let mut end = None;
    for (idx, ch) in s.char_indices().rev() {
        if is_ident_char(ch) {
            end.get_or_insert(idx + ch.len_utf8());
        } else if let Some(end_idx) = end {
            return Some(s[idx + ch.len_utf8()..end_idx].to_string());
        }
    }
    end.map(|end_idx| s[..end_idx].to_string())
}

fn is_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn first_string_literal(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut start = 0;
    while start < bytes.len()
        && (bytes[start] == b'r'
            || bytes[start] == b'u'
            || bytes[start] == b'b'
            || bytes[start] == b'f')
    {
        start += 1;
    }
    if start >= bytes.len() || (bytes[start] != b'\'' && bytes[start] != b'"') {
        return None;
    }
    let quote = bytes[start];
    let mut escaped = false;
    let mut out = String::new();
    for &b in &bytes[start + 1..] {
        if escaped {
            out.push(b as char);
            escaped = false;
        } else if b == b'\\' {
            escaped = true;
        } else if b == quote {
            return Some(out);
        } else {
            out.push(b as char);
        }
    }
    None
}

fn is_real_secret_literal(value: &str) -> bool {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    !trimmed.is_empty()
        && lower != "xxx"
        && lower != "xxxx"
        && lower != "changeme"
        && lower != "change_me"
        && lower != "placeholder"
        && lower != "todo"
        && !lower.contains("${")
        && !lower.contains("process.env")
        && !lower.contains("os.environ")
        && !lower.contains("getenv")
        && !lower.starts_with("env:")
}

fn line_has_sql_injection(original: &str, executable: &str, executable_lower: &str) -> bool {
    has_sink_call(executable_lower)
        && has_sql_keyword(&original.to_ascii_lowercase())
        && (executable.contains('+') || has_interpolation(original))
        && has_identifier_outside_strings(original)
}

fn has_sink_call(lower: &str) -> bool {
    sink_names()
        .iter()
        .any(|name| lower.contains(&format!("{}(", name)) || lower.contains(&format!(".{}(", name)))
}

fn has_sql_keyword(lower: &str) -> bool {
    [
        "select ", "insert ", "update ", "delete ", "where ", " from ",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

fn has_interpolation(line: &str) -> bool {
    line.contains("${")
        || (line.contains('{') && line.contains('}'))
        || line.contains("%s")
        || line.contains("%(")
}

fn has_identifier_outside_strings(line: &str) -> bool {
    let mut in_quote = None;
    let mut escaped = false;
    let mut current = String::new();
    for ch in line.chars() {
        if let Some(q) = in_quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                in_quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            in_quote = Some(ch);
            current.clear();
            continue;
        }
        if is_ident_char(ch) {
            current.push(ch);
        } else {
            if is_external_identifier(&current) {
                return true;
            }
            current.clear();
        }
    }
    is_external_identifier(&current)
}

fn is_external_identifier(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !lower.is_empty()
        && !matches!(
            lower.as_str(),
            "select"
                | "insert"
                | "update"
                | "delete"
                | "where"
                | "from"
                | "execute"
                | "query"
                | "exec"
                | "raw"
                | "true"
                | "false"
                | "none"
                | "null"
        )
        && lower.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn line_has_command_injection(lang: &str, line: &str, lower: &str) -> bool {
    if supports_backtick_execution(lang) && line_has_dynamic_command_backticks(line) {
        return true;
    }
    if lang.eq_ignore_ascii_case("rust") || lang.eq_ignore_ascii_case("rs") {
        return rust_process_call_has_dynamic_argument(line)
            || (rust_has_known_command_sink(lower) && call_has_non_literal_argument(line));
    }
    line_has_command_execution_call(lower) && call_has_non_literal_argument(line)
}

fn line_has_dangerous_eval(lang: &str, line: &str, lower: &str) -> bool {
    let dangerous_call = has_bare_call(lower, "eval")
        || (has_bare_call(lower, "exec") && !has_command_execution_context("", lower))
        || (is_javascript_like_language(lang) && has_global_function_call(line))
        || lower.contains("pickle.loads(")
        || lower.contains("marshal.load(")
        || lower.contains("deserialize(")
        || (lower.contains("yaml.load(") && !lower.contains("safeloader"));
    dangerous_call && call_has_external_input(line)
}

fn line_has_weak_crypto(_line: &str, lower: &str) -> bool {
    has_identifier_call(lower, "md5")
        || has_identifier_call(lower, "sha1")
        || has_identifier_call(lower, "des")
        || has_identifier_member(lower, "des")
        || has_ecb_mode(lower)
        || has_qualified_call(lower, "math.random")
}

fn line_has_insecure_random(line: &str, lower: &str) -> bool {
    let target = find_assignment_equals(line).and_then(|idx| last_identifier(&line[..idx]));
    if let Some(target) = target {
        let t = target.to_ascii_lowercase();
        let security_target = ["token", "key", "nonce", "secret", "password"]
            .iter()
            .any(|name| t.contains(name));
        security_target
            && (lower.contains("random(")
                || lower.contains("rand(")
                || lower.contains("random.")
                || lower.contains("rand."))
    } else {
        false
    }
}

fn callee_name(call_text: &str) -> String {
    let before_paren = call_text.split('(').next().unwrap_or(call_text).trim_end();
    let mut chars = before_paren.chars().rev();
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        if ch == '.' || ch == ':' || is_ident_char(ch) {
            out.push(ch);
        } else if !out.is_empty() {
            break;
        }
    }
    out.chars()
        .rev()
        .collect::<String>()
        .trim_matches('.')
        .to_ascii_lowercase()
}

fn is_sql_sink(callee: &str) -> bool {
    let last = callee.rsplit('.').next().unwrap_or(callee);
    sink_names().contains(&last)
}

fn call_has_command_injection(lang: &str, callee: &str, call_text: &str, lower: &str) -> bool {
    if lang.eq_ignore_ascii_case("rust") || lang.eq_ignore_ascii_case("rs") {
        return rust_process_call_has_dynamic_argument(call_text)
            || (has_command_execution_context(callee, lower)
                && call_has_non_literal_argument(call_text));
    }
    has_command_execution_context(callee, lower) && call_has_non_literal_argument(call_text)
}

fn call_contains_dynamic_sql(comment_masked: &str, executable: &str, lower: &str) -> bool {
    has_sql_keyword(lower)
        && (executable.contains('+') || has_interpolation(comment_masked))
        && has_identifier_outside_strings(comment_masked)
}

fn call_has_non_literal_argument(call_text: &str) -> bool {
    if let Some(args) = between_parens(call_text) {
        let arg = args.trim();
        if arg.is_empty() {
            return false;
        }
        if is_single_literal(arg) {
            return false;
        }
        arg.contains('+') || has_interpolation(arg) || has_identifier_outside_strings(arg)
    } else {
        false
    }
}

fn call_has_external_input(call_text: &str) -> bool {
    if let Some(args) = between_parens(call_text) {
        let lower = args.to_ascii_lowercase();
        !is_single_literal(args.trim())
            && (has_identifier_outside_strings(args)
                || lower.contains("input")
                || lower.contains("request")
                || lower.contains("user")
                || lower.contains("body")
                || lower.contains("params"))
    } else {
        false
    }
}

fn between_parens(s: &str) -> Option<&str> {
    let start = s.find('(')?;
    let end = s.rfind(')')?;
    if end > start {
        Some(&s[start + 1..end])
    } else {
        None
    }
}

fn is_single_literal(arg: &str) -> bool {
    let arg = arg.trim();
    first_string_literal(arg).map_or(false, |value| {
        let prefix_len = arg.find(|ch| ch == '\'' || ch == '"').unwrap_or(0);
        let quoted_len = prefix_len + value.len() + 2;
        arg.len() <= quoted_len || arg[quoted_len..].trim().is_empty()
    }) || arg
        .chars()
        .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == '-' || ch.is_whitespace())
}

fn call_has_dangerous_eval(lang: &str, callee: &str, call_text: &str, lower: &str) -> bool {
    let last = callee.rsplit('.').next().unwrap_or(callee);
    ((last == "eval")
        || (last == "exec" && !callee.contains('.') && !callee.contains(':'))
        || (is_javascript_like_language(lang) && is_global_function_callee(callee))
        || last == "deserialize"
        || callee.ends_with("pickle.loads")
        || callee.ends_with("marshal.load")
        || (callee.ends_with("yaml.load") && !lower.contains("safeloader")))
        && !has_command_execution_context(callee, lower)
        && call_has_external_input(call_text)
}

fn line_has_dynamic_command_backticks(line: &str) -> bool {
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('`') else {
            return false;
        };
        let command = &after_start[..end];
        if command.contains("${") || command.contains("#{") || contains_shell_variable(command) {
            return true;
        }
        rest = &after_start[end + 1..];
    }
    false
}

fn contains_shell_variable(command: &str) -> bool {
    let bytes = command.as_bytes();
    (0..bytes.len()).any(|index| {
        bytes[index] == b'$'
            && bytes.get(index + 1).is_some_and(|next| {
                *next == b'_'
                    || next.is_ascii_alphabetic()
                    || next.is_ascii_digit()
                    || matches!(*next, b'@' | b'*' | b'?' | b'$' | b'#' | b'-' | b'!')
            })
    })
}

fn rust_process_call_has_dynamic_argument(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let mut constructor_offset = 0;
    while let Some(command_start) = find_rust_command_constructor(&lower, constructor_offset) {
        let command_open = command_start + "command::new".len();
        if parenthesized_argument_at(line, command_open).is_some_and(is_dynamic_argument) {
            return true;
        }

        constructor_offset = command_open + 1;
        let chain_end =
            find_rust_command_constructor(&lower, constructor_offset).unwrap_or(lower.len());
        for method in [".arg(", ".args("] {
            let mut offset = constructor_offset;
            while let Some(relative) = lower[offset..chain_end].find(method) {
                let open = offset + relative + method.len() - 1;
                if parenthesized_argument_at(line, open).is_some_and(is_dynamic_argument) {
                    return true;
                }
                offset = open + 1;
            }
        }
    }
    false
}

fn find_rust_command_constructor(lower: &str, mut offset: usize) -> Option<usize> {
    let pattern = "command::new(";
    while let Some(relative) = lower[offset..].find(pattern) {
        let start = offset + relative;
        let identifier_boundary = lower[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_ident_char(ch));
        if identifier_boundary {
            return Some(start);
        }
        offset = start + pattern.len();
    }
    None
}

fn parenthesized_argument_at(value: &str, open: usize) -> Option<&str> {
    if value.as_bytes().get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0usize;
    for (relative, ch) in value[open..].char_indices() {
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(&value[open + 1..open + relative]);
            }
        }
    }
    None
}

fn is_dynamic_argument(argument: &str) -> bool {
    let argument = argument.trim();
    !argument.is_empty()
        && !is_single_literal(argument)
        && !argument.strip_prefix('&').is_some_and(is_single_literal)
        && has_identifier_outside_strings(argument)
}

#[derive(Clone, Copy)]
enum MaskState {
    Code,
    LineComment,
    SlashBlockComment(usize),
    HaskellBlockComment(usize),
    Quote { delimiter: u8, escaped: bool },
    TripleQuote { delimiter: u8, escaped: bool },
    ExecutableBacktick { escaped: bool },
    RustRaw { hashes: usize },
}

fn mask_non_executable_source(lang: &str, source: &str) -> String {
    mask_source(lang, source, true)
}

fn mask_comments(lang: &str, source: &str) -> String {
    mask_source(lang, source, false)
}

fn mask_source(lang: &str, source: &str, mask_strings: bool) -> String {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let language = lang.to_ascii_lowercase();
    let rust = matches!(language.as_str(), "rust" | "rs");
    let javascript = is_javascript_like_language(lang);
    let shell_like = supports_backtick_execution(lang);
    let ruby = matches!(language.as_str(), "ruby" | "rb");
    let python = matches!(language.as_str(), "python" | "py");
    let php = language == "php";
    let haskell = matches!(language.as_str(), "haskell" | "hs");
    let nested_slash_blocks = rust || matches!(language.as_str(), "swift");
    let bash = matches!(
        language.as_str(),
        "sh" | "bash" | "zsh" | "ksh" | "shell" | "shellscript"
    );
    let slash_comments = matches!(
        language.as_str(),
        "rust"
            | "rs"
            | "javascript"
            | "js"
            | "jsx"
            | "typescript"
            | "ts"
            | "tsx"
            | "java"
            | "c"
            | "cpp"
            | "c++"
            | "cc"
            | "cxx"
            | "h"
            | "hpp"
            | "csharp"
            | "c#"
            | "cs"
            | "go"
            | "golang"
            | "swift"
            | "scala"
            | "php"
    );
    let hash_comments = python || ruby || php || bash;
    let mut state = MaskState::Code;
    let mut index = 0;

    while index < bytes.len() {
        match state {
            MaskState::LineComment => {
                if bytes[index] == b'\n' {
                    state = MaskState::Code;
                } else {
                    masked[index] = b' ';
                }
                index += 1;
            }
            MaskState::SlashBlockComment(depth) => {
                if bytes[index] == b'\n' {
                    index += 1;
                } else if nested_slash_blocks && bytes[index..].starts_with(b"/*") {
                    masked[index..index + 2].fill(b' ');
                    state = MaskState::SlashBlockComment(depth + 1);
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    masked[index..index + 2].fill(b' ');
                    state = if depth == 1 {
                        MaskState::Code
                    } else {
                        MaskState::SlashBlockComment(depth - 1)
                    };
                    index += 2;
                } else {
                    masked[index] = b' ';
                    index += 1;
                }
            }
            MaskState::HaskellBlockComment(depth) => {
                if bytes[index] == b'\n' {
                    index += 1;
                } else if bytes[index..].starts_with(b"{-") {
                    masked[index..index + 2].fill(b' ');
                    state = MaskState::HaskellBlockComment(depth + 1);
                    index += 2;
                } else if bytes[index..].starts_with(b"-}") {
                    masked[index..index + 2].fill(b' ');
                    state = if depth == 1 {
                        MaskState::Code
                    } else {
                        MaskState::HaskellBlockComment(depth - 1)
                    };
                    index += 2;
                } else {
                    masked[index] = b' ';
                    index += 1;
                }
            }
            MaskState::Quote {
                delimiter,
                mut escaped,
            } => {
                if mask_strings && bytes[index] != b'\n' {
                    masked[index] = b' ';
                }
                if escaped {
                    escaped = false;
                } else if bytes[index] == b'\\' {
                    escaped = true;
                } else if bytes[index] == delimiter {
                    state = MaskState::Code;
                    index += 1;
                    continue;
                }
                state = MaskState::Quote { delimiter, escaped };
                index += 1;
            }
            MaskState::TripleQuote {
                delimiter,
                mut escaped,
            } => {
                if bytes[index..].starts_with(&[delimiter, delimiter, delimiter]) && !escaped {
                    if mask_strings {
                        masked[index..index + 3].fill(b' ');
                    }
                    index += 3;
                    state = MaskState::Code;
                } else {
                    if mask_strings && bytes[index] != b'\n' {
                        masked[index] = b' ';
                    }
                    escaped = !escaped && bytes[index] == b'\\';
                    state = MaskState::TripleQuote { delimiter, escaped };
                    index += 1;
                }
            }
            MaskState::RustRaw { hashes } => {
                if bytes[index] == b'"'
                    && bytes
                        .get(index + 1..index + 1 + hashes)
                        .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
                {
                    if mask_strings {
                        masked[index..index + 1 + hashes].fill(b' ');
                    }
                    index += 1 + hashes;
                    state = MaskState::Code;
                } else {
                    if mask_strings && bytes[index] != b'\n' {
                        masked[index] = b' ';
                    }
                    index += 1;
                }
            }
            MaskState::ExecutableBacktick { mut escaped } => {
                if escaped {
                    escaped = false;
                } else if bytes[index] == b'\\' {
                    escaped = true;
                } else if bytes[index] == b'`' {
                    state = MaskState::Code;
                    index += 1;
                    continue;
                }
                state = MaskState::ExecutableBacktick { escaped };
                index += 1;
            }
            MaskState::Code => {
                if slash_comments && bytes[index..].starts_with(b"//") {
                    masked[index..index + 2].fill(b' ');
                    index += 2;
                    state = MaskState::LineComment;
                } else if slash_comments && bytes[index..].starts_with(b"/*") {
                    masked[index..index + 2].fill(b' ');
                    index += 2;
                    state = MaskState::SlashBlockComment(1);
                } else if haskell && bytes[index..].starts_with(b"--") {
                    masked[index..index + 2].fill(b' ');
                    index += 2;
                    state = MaskState::LineComment;
                } else if haskell && bytes[index..].starts_with(b"{-") {
                    masked[index..index + 2].fill(b' ');
                    index += 2;
                    state = MaskState::HaskellBlockComment(1);
                } else if hash_comments && bytes[index] == b'#' {
                    masked[index] = b' ';
                    index += 1;
                    state = MaskState::LineComment;
                } else if shell_like && bytes[index] == b'`' {
                    index += 1;
                    state = MaskState::ExecutableBacktick { escaped: false };
                } else if rust {
                    if let Some((quote, hashes)) = rust_raw_string_start(bytes, index) {
                        if mask_strings {
                            masked[index..quote + 1].fill(b' ');
                        }
                        index = quote + 1;
                        state = MaskState::RustRaw { hashes };
                    } else if bytes[index] == b'"' {
                        if mask_strings {
                            masked[index] = b' ';
                        }
                        index += 1;
                        state = MaskState::Quote {
                            delimiter: b'"',
                            escaped: false,
                        };
                    } else if let Some(end) = (bytes[index] == b'\'')
                        .then(|| rust_char_literal_end(bytes, index))
                        .flatten()
                    {
                        if mask_strings {
                            masked[index..=end].fill(b' ');
                        }
                        index = end + 1;
                    } else {
                        index += 1;
                    }
                } else if (python || ruby)
                    && (bytes[index..].starts_with(b"\"\"\"") || bytes[index..].starts_with(b"'''"))
                {
                    let delimiter = bytes[index];
                    if mask_strings {
                        masked[index..index + 3].fill(b' ');
                    }
                    index += 3;
                    state = MaskState::TripleQuote {
                        delimiter,
                        escaped: false,
                    };
                } else if bytes[index] == b'\''
                    || bytes[index] == b'"'
                    || (javascript && bytes[index] == b'`')
                {
                    let delimiter = bytes[index];
                    if mask_strings {
                        masked[index] = b' ';
                    }
                    index += 1;
                    state = MaskState::Quote {
                        delimiter,
                        escaped: false,
                    };
                } else {
                    index += 1;
                }
            }
        }
    }
    String::from_utf8(masked).expect("masking preserves UTF-8 byte boundaries")
}

fn rust_raw_string_start(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
    let mut marker = if bytes.get(index) == Some(&b'r') {
        index + 1
    } else if bytes.get(index..index + 2) == Some(b"br") {
        index + 2
    } else {
        return None;
    };
    let hash_start = marker;
    while bytes.get(marker) == Some(&b'#') {
        marker += 1;
    }
    (bytes.get(marker) == Some(&b'"')).then_some((marker, marker - hash_start))
}

fn rust_char_literal_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    if bytes.get(index) == Some(&b'\\') {
        index += 2;
    } else {
        let ch = std::str::from_utf8(bytes.get(index..)?)
            .ok()?
            .chars()
            .next()?;
        index += ch.len_utf8();
    }
    (bytes.get(index) == Some(&b'\'')).then_some(index)
}

fn supports_backtick_execution(lang: &str) -> bool {
    matches!(
        lang.to_ascii_lowercase().as_str(),
        "sh" | "bash" | "zsh" | "ksh" | "shell" | "shellscript" | "ruby" | "rb" | "php"
    )
}

fn is_javascript_like_language(lang: &str) -> bool {
    matches!(
        lang.to_ascii_lowercase().as_str(),
        "javascript" | "js" | "jsx" | "typescript" | "ts" | "tsx"
    )
}

fn line_has_command_execution_call(lower: &str) -> bool {
    has_command_execution_context("", lower)
        || ["system", "popen", "spawn", "shell_exec", "passthru"]
            .iter()
            .any(|name| has_bare_call(lower, name))
}

fn has_command_execution_context(callee: &str, lower: &str) -> bool {
    let compact = lower
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let callee = callee.trim_matches('.').to_ascii_lowercase();
    let last = callee.rsplit(['.', ':']).next().unwrap_or(&callee);
    matches!(
        last,
        "system" | "popen" | "spawn" | "shell_exec" | "passthru"
    ) || callee.contains("subprocess")
        || callee.contains("child_process")
        || callee.contains("runtime.getruntime")
        || compact.contains("os.system(")
        || compact.contains("os.popen(")
        || compact.contains("subprocess.")
        || compact.contains("child_process.")
        || compact.contains("runtime.getruntime().exec(")
        || compact.contains("runtime.exec(")
        || compact.contains("processbuilder(")
}

fn has_bare_call(lower: &str, name: &str) -> bool {
    let pattern = format!("{name}(");
    let mut offset = 0;
    while let Some(index) = lower[offset..].find(&pattern) {
        let absolute = offset + index;
        let bare = lower[..absolute]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_ident_char(ch) && ch != '.' && ch != ':');
        if bare {
            return true;
        }
        offset = absolute + pattern.len();
    }
    false
}

fn rust_has_known_command_sink(lower: &str) -> bool {
    ["system", "popen", "spawn", "exec", "execl", "execv"]
        .iter()
        .any(|name| has_identifier_call(lower, name))
}

fn has_global_function_call(line: &str) -> bool {
    let mut offset = 0;
    while let Some(index) = line[offset..].find("Function") {
        let absolute = offset + index;
        let before = line[..absolute].trim_end();
        let boundary_before = before
            .chars()
            .next_back()
            .is_none_or(|ch| !is_ident_char(ch) && ch != ':' && ch != '.');
        let qualified_global = before.strip_suffix('.').is_some_and(|prefix| {
            let qualifier = trailing_identifier_path(prefix).to_ascii_lowercase();
            matches!(
                qualifier.as_str(),
                "globalthis" | "window" | "self" | "global"
            )
        });
        let constructor = before
            .split_whitespace()
            .next_back()
            .is_some_and(|word| word == "new");
        let after = line[absolute + "Function".len()..].trim_start();
        let declaration = before
            .split_whitespace()
            .next_back()
            .is_some_and(|word| word == "function");
        if (boundary_before || qualified_global || constructor)
            && after.starts_with('(')
            && !declaration
        {
            return true;
        }
        offset = absolute + "Function".len();
    }
    false
}

fn trailing_identifier_path(value: &str) -> &str {
    let value = value.trim_end();
    let start = value
        .char_indices()
        .rev()
        .find_map(|(index, ch)| (!is_ident_char(ch) && ch != '.').then_some(index + ch.len_utf8()))
        .unwrap_or(0);
    &value[start..]
}

fn is_global_function_callee(callee: &str) -> bool {
    let mut parts = callee.rsplit('.');
    if parts.next() != Some("function") {
        return false;
    }
    match parts.next() {
        None => true,
        Some(qualifier) => {
            parts.next().is_none()
                && matches!(qualifier, "globalthis" | "window" | "self" | "global")
        }
    }
}

fn has_identifier_call(lower: &str, name: &str) -> bool {
    has_identifier_followed_by(lower, name, '(')
}

fn has_identifier_member(lower: &str, name: &str) -> bool {
    has_identifier_followed_by(lower, name, '.')
}

fn has_identifier_followed_by(lower: &str, name: &str, following: char) -> bool {
    let mut offset = 0;
    while let Some(index) = lower[offset..].find(name) {
        let absolute = offset + index;
        let boundary_before = lower[..absolute]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_ident_char(ch));
        let after = lower[absolute + name.len()..].trim_start();
        if boundary_before && after.starts_with(following) {
            return true;
        }
        offset = absolute + name.len();
    }
    false
}

fn has_qualified_call(lower: &str, name: &str) -> bool {
    let mut offset = 0;
    while let Some(index) = lower[offset..].find(name) {
        let absolute = offset + index;
        let boundary_before = lower[..absolute]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_ident_char(ch));
        let after = lower[absolute + name.len()..].trim_start();
        if boundary_before && after.starts_with('(') {
            return true;
        }
        offset = absolute + name.len();
    }
    false
}

fn has_identifier_token(lower: &str, token: &str) -> bool {
    let mut offset = 0;
    while let Some(index) = lower[offset..].find(token) {
        let absolute = offset + index;
        let boundary_before = lower[..absolute]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_ident_char(ch));
        let boundary_after = lower[absolute + token.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !is_ident_char(ch));
        if boundary_before && boundary_after {
            return true;
        }
        offset = absolute + token.len();
    }
    false
}

fn has_ecb_mode(lower: &str) -> bool {
    has_identifier_token(lower, "ecb")
        || has_identifier_token(lower, "mode_ecb")
        || has_identifier_token(lower, "ecb_mode")
}

fn is_weak_crypto_callee(callee: &str, lower: &str) -> bool {
    let last = callee.rsplit('.').next().unwrap_or(callee);
    matches!(last, "md5" | "sha1" | "des")
        || has_qualified_call(lower, "math.random")
        || has_ecb_mode(lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_rule(findings: &[SecurityFinding], rule: &str) -> bool {
        findings.iter().any(|finding| finding.rule == rule)
    }

    #[test]
    fn detects_hardcoded_secret() {
        let findings = scan("python", "password = \"hunter2\"\n");
        let finding = findings
            .iter()
            .find(|finding| finding.rule == "hardcoded_secret")
            .unwrap();
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.line, 1);
    }

    #[test]
    fn detects_sql_injection() {
        let findings = scan(
            "python",
            "cursor.execute(\"SELECT * FROM t WHERE x=\" + user)\n",
        );
        assert!(has_rule(&findings, "sql_injection"));
    }

    #[test]
    fn detects_dangerous_eval() {
        let findings = scan("python", "eval(user_input)\n");
        assert!(has_rule(&findings, "dangerous_eval"));
    }

    #[test]
    fn eval_reports_single_rule() {
        let findings = scan("python", "eval(payload)\n");
        assert_eq!(findings.len(), 1, "got {findings:?}");
        assert_eq!(findings[0].rule, "dangerous_eval");
        assert_eq!(findings[0].line, 1);
    }

    #[test]
    fn subprocess_still_command_injection() {
        let findings = scan("python", "subprocess.run(user_command, shell=True)\n");
        let finding = findings
            .iter()
            .find(|finding| finding.rule == "command_injection")
            .expect("subprocess must stay command injection");
        assert_eq!(finding.severity, Severity::High);
        assert_eq!(finding.line, 1);
        assert!(!has_rule(&findings, "dangerous_eval"));
    }

    #[test]
    fn detects_dynamic_child_process_template_execution() {
        let findings = scan("typescript", "child_process.exec(`echo ${userInput}`);\n");
        assert!(has_rule(&findings, "command_injection"));
    }

    #[test]
    fn detects_dynamic_backtick_execution_in_supported_languages() {
        for (lang, source) in [
            ("ruby", "result = `echo #{user_input}`\n"),
            ("php", "$result = trim(`id $username`);\n"),
            ("bash", "result=`cat ${filename}`\n"),
        ] {
            let findings = scan(lang, source);
            assert!(
                has_rule(&findings, "command_injection"),
                "expected {lang} backtick finding for {source:?}: {findings:?}"
            );
        }
    }

    #[test]
    fn detects_shell_special_parameters_in_executable_backticks() {
        for parameter in ["$1", "$9", "$@", "$*", "$?", "$$", "$#", "$-", "$!", "$0"] {
            for (lang, source) in [
                ("ruby", format!("result = `printf '%s' {parameter}`\n")),
                ("php", format!("$result = `printf '%s' {parameter}`;\n")),
                ("bash", format!("result=`printf '%s' {parameter}`\n")),
            ] {
                let findings = scan(lang, &source);
                let command_findings = findings
                    .iter()
                    .filter(|finding| finding.rule == "command_injection")
                    .count();
                assert_eq!(
                    command_findings, 1,
                    "expected one {lang} finding for {parameter:?}: {findings:?}"
                );
            }
        }
    }

    #[test]
    fn backtick_dollar_without_a_shell_parameter_is_not_dynamic() {
        for (lang, source) in [
            ("ruby", "result = `printf '$:'`\n"),
            ("php", "$result = `printf '$:'`;\n"),
            ("bash", "result=`printf '$:'`\n"),
        ] {
            let findings = scan(lang, source);
            assert!(
                !has_rule(&findings, "command_injection"),
                "unexpected {lang} finding for {source:?}: {findings:?}"
            );
        }
    }

    #[test]
    fn comments_and_quoted_backticks_are_not_command_injection() {
        for (lang, source) in [
            (
                "ruby",
                "# example: `echo #{user_input}`\nmessage = \"`echo #{user_input}`\"\n",
            ),
            (
                "php",
                "// example: `id $username`\n$message = '`id $username`';\n",
            ),
            (
                "bash",
                "# example: `cat ${filename}`\nmessage='`cat ${filename}`'\n",
            ),
        ] {
            let findings = scan(lang, source);
            assert!(
                !has_rule(&findings, "command_injection"),
                "unexpected {lang} quoted/comment finding: {findings:?}"
            );
        }
    }

    #[test]
    fn executable_backticks_remain_visible_after_masking() {
        for (lang, source) in [
            ("ruby", "message = 'safe'; result = `echo #{user_input}`\n"),
            ("php", "$message = 'safe'; $result = `id $username`;\n"),
            ("bash", "message='safe'; result=`cat ${filename}`\n"),
        ] {
            let findings = scan(lang, source);
            assert!(
                has_rule(&findings, "command_injection"),
                "{lang}: {findings:?}"
            );
        }
    }

    #[test]
    fn static_backticks_and_non_executing_templates_are_not_command_injection() {
        for (lang, source) in [
            ("ruby", "result = `date`\n"),
            ("php", "$result = `whoami`;\n"),
            ("typescript", "const message = `hello ${userInput}`;\n"),
            (
                "rust",
                "let source = r#\"child_process.exec(`echo ${user}`)\"#;\n",
            ),
        ] {
            let findings = scan(lang, source);
            assert!(
                !has_rule(&findings, "command_injection"),
                "unexpected {lang} finding for {source:?}: {findings:?}"
            );
        }
    }

    #[test]
    fn detects_dynamic_rust_process_command_arguments() {
        for source in [
            "std::process::Command::new(user_input).status();\n",
            "std::process::Command::new(\"sh\").arg(user_input).status();\n",
            "Command::new(\"sh\").args(command_args).status();\n",
            "/* ignored Command::new(commented)\n*/ Command::new(user_input).status();\n",
            "let source = \"Command::new(embedded)\"; Command::new(user_input).status();\n",
        ] {
            let findings = scan("rust", source);
            assert!(
                has_rule(&findings, "command_injection"),
                "expected Rust process finding for {source:?}: {findings:?}"
            );
        }
    }

    #[test]
    fn detects_dynamic_arguments_in_later_rust_command_constructors() {
        for source in [
            "Command::new(\"date\").status(); Command::new(user_input).status();\n",
            "Command::new(\"date\").status(); Command::new(\"sh\").arg(user_input).status();\n",
            "Command::new(\"date\").status(); Command::new(\"sh\").args(command_args).status();\n",
        ] {
            let findings = scan("rust", source);
            assert!(
                has_rule(&findings, "command_injection"),
                "expected later Rust process finding for {source:?}: {findings:?}"
            );
        }
    }

    #[test]
    fn rust_process_literals_and_embedded_source_are_not_command_injection() {
        for source in [
            "std::process::Command::new(\"date\").arg(\"+%s\").status();\n",
            "std::process::Command::new(&\"date\").arg(&\"+%s\").status();\n",
            "let source = r#\"std::process::Command::new(user_input).arg(user_input)\"#;\n",
            "let source = \"Command::new(user_input) with `echo $1`\";\n",
            "log_message(\"Command::new(user_input).arg(user_input)\");\n",
            "log::debug!(\"Command::new(user_input).arg(user_input)\");\n",
            "// std::process::Command::new(user_input).status();\n",
            "/* std::process::Command::new(user_input).status(); */\n",
            "/* start\nCommand::new(user_input).arg(user_input).status();\nend */\n",
            "/* outer /* nested\nCommand::new(user_input).status();\n*/ end */\n",
        ] {
            let findings = scan("rust", source);
            assert!(
                !has_rule(&findings, "command_injection"),
                "unexpected Rust finding for {source:?}: {findings:?}"
            );
        }
    }

    #[test]
    fn rust_multiline_comments_and_strings_stay_masked() {
        for source in [
            "/* ignored\nCommand::new(user_input).status();\n*/\n",
            "let example = r#\"ignored\nCommand::new(user_input).status();\nlibc::system(user_input);\n\"#;\n",
            "let example = \"ignored\\\nCommand::new(user_input).status()\";\n",
        ] {
            let findings = scan("rust", source);
            assert!(
                !has_rule(&findings, "command_injection"),
                "unexpected multiline Rust finding for {source:?}: {findings:?}"
            );
        }
    }

    #[test]
    fn rust_command_and_known_sinks_are_combined() {
        for source in [
            "libc::system(user_input);\n",
            "unsafe { popen(command, mode) };\n",
            "Command::new(\"sh\").arg(user_input).status();\n",
            "fn run<'a>(command: &'a str) { libc::system(command); }\n",
        ] {
            let findings = scan("rust", source);
            assert!(
                has_rule(&findings, "command_injection"),
                "expected Rust sink for {source:?}: {findings:?}"
            );
        }
    }

    #[test]
    fn template_literal_json_is_not_command_injection() {
        for lang in ["typescript", "tsx"] {
            let findings = scan(lang, "`payload: ${JSON.stringify(value)}`;\n");
            assert!(
                !has_rule(&findings, "command_injection"),
                "unexpected {lang} findings: {findings:?}"
            );
        }
    }

    #[test]
    fn planted_secret_and_injection_lines_still_report() {
        let findings = scan(
            "python",
            "# planted\naws_key = \"AKIA1234567890ABCDEF\"\npassword = \"hunter2\"\nsubprocess.run(cmd, shell=True)\n",
        );

        assert!(findings.iter().any(|finding| {
            finding.rule == "hardcoded_secret"
                && finding.severity == Severity::Critical
                && finding.line == 2
        }));
        assert!(findings.iter().any(|finding| {
            finding.rule == "hardcoded_secret" && finding.severity == Severity::Critical
        }));
        assert!(findings.iter().any(|finding| {
            finding.rule == "command_injection"
                && finding.severity == Severity::High
                && finding.line == 4
        }));
    }

    #[test]
    fn ignores_literal_eval() {
        let findings = scan("python", "eval(\"2 + 2\")\n");
        assert!(!has_rule(&findings, "dangerous_eval"));
    }

    #[test]
    fn detects_weak_crypto() {
        let findings = scan("python", "hashlib.md5(data).hexdigest()\n");
        assert!(has_rule(&findings, "weak_crypto"));
    }

    #[test]
    fn detects_des_with_identifier_boundaries() {
        for source in ["DES(data)\n", "cipher.des(data)\n", "DES.new(key)\n"] {
            let findings = scan("python", source);
            assert!(has_rule(&findings, "weak_crypto"), "source: {source}");
        }
    }

    #[test]
    fn preserves_other_weak_crypto_findings() {
        for (lang, source) in [
            ("python", "hashlib.sha1(data)\n"),
            ("python", "AES.new(key, AES.MODE_ECB)\n"),
            ("javascript", "Math.random()\n"),
        ] {
            let findings = scan(lang, source);
            assert!(has_rule(&findings, "weak_crypto"), "source: {source}");
        }
    }

    #[test]
    fn includes_and_description_are_not_weak_crypto() {
        for lang in ["typescript", "tsx"] {
            let findings = scan(lang, "zone.send_to.includes(\"*\")\n");
            assert!(
                !has_rule(&findings, "weak_crypto"),
                "unexpected {lang} findings: {findings:?}"
            );
        }
        let findings = scan("typescript", "description.includes(type);\n");
        assert!(!has_rule(&findings, "weak_crypto"));
    }

    #[test]
    fn named_function_calls_and_declarations_are_not_dangerous_eval() {
        let findings = scan(
            "typescript",
            "function namedFunction(pathValue) {}\nnamedFunction(current)\nfunction buildFunction(value) {}\nbuildFunction(current)\n",
        );
        assert!(!has_rule(&findings, "dangerous_eval"));
    }

    #[test]
    fn detects_global_function_with_external_input() {
        let findings = scan("javascript", "const factory = Function(userInput);\n");
        assert!(has_rule(&findings, "dangerous_eval"));
    }

    #[test]
    fn detects_javascript_global_function_constructors() {
        for source in [
            "const factory = new Function(userInput);\n",
            "const factory = globalThis.Function(userInput);\n",
            "const factory = window.Function(userInput);\n",
            "const factory = self.Function(userInput);\n",
        ] {
            let findings = scan("typescript", source);
            assert!(
                has_rule(&findings, "dangerous_eval"),
                "expected Function finding for {source:?}: {findings:?}"
            );
        }
    }

    #[test]
    fn javascript_comments_and_strings_do_not_create_function_findings() {
        for (lang, source) in [
            (
                "javascript",
                "// Function(userInput)\nconst example = \"Function(userInput)\";\n",
            ),
            (
                "typescript",
                "/* Function(userInput) */\nconst example = 'Function(userInput)';\n",
            ),
            ("tsx", "const example = `Function(userInput)`;\n"),
        ] {
            let findings = scan(lang, source);
            assert!(
                !has_rule(&findings, "dangerous_eval"),
                "unexpected {lang} Function finding: {findings:?}"
            );
        }
    }

    #[test]
    fn ordinary_or_non_global_function_members_are_not_dangerous_eval() {
        for source in [
            "namedFunction(userInput);\n",
            "object.Function(userInput);\n",
            "function Function(userInput) {}\n",
            "function namedFunction(userInput) {}\n",
        ] {
            let findings = scan("typescript", source);
            assert!(
                !has_rule(&findings, "dangerous_eval"),
                "unexpected Function finding for {source:?}: {findings:?}"
            );
        }
    }

    #[test]
    fn embedded_javascript_in_rust_is_not_weak_crypto() {
        let findings = scan("rust", "let source = r#\"zone.send_to.includes(type)\"#;\n");
        assert!(!has_rule(&findings, "weak_crypto"));
    }

    #[test]
    fn detects_tls_verify_disabled() {
        let findings = scan("python", "requests.get(url, verify=False)\n");
        assert!(has_rule(&findings, "tls_verify_disabled"));
    }

    #[test]
    fn comments_cannot_create_line_heuristic_findings() {
        for (lang, source) in [
            (
                "python",
                "# password = \"hunter2\"\n# eval(user_input)\n# cursor.execute(\"SELECT * FROM t WHERE x=\" + user)\n# requests.get(url, verify=False)\n",
            ),
            (
                "ruby",
                "# password = \"hunter2\"; eval(user_input); query(\"SELECT x FROM t\" + user); verify=false\n",
            ),
            (
                "bash",
                "# password=\"hunter2\"; eval(user_input); query(\"SELECT x FROM t\" + user); verify=false\n",
            ),
            (
                "javascript",
                "// token = \"secret-value\"\n/* eval(userInput); db.query(\"SELECT * FROM t WHERE x=\" + user); */\n// fetch(url, { rejectUnauthorized: false });\n",
            ),
            (
                "java",
                "// password = \"hunter2\";\n/* runtime.exec(userInput); MessageDigest.getInstance(\"MD5\"); */\n",
            ),
            (
                "php",
                "# $password = \"hunter2\";\n// eval($userInput);\n/* $db->query(\"SELECT * FROM t WHERE x=\" . $user); */\n",
            ),
            (
                "haskell",
                "-- password = \"hunter2\"\n{- eval(user_input)\nrequests.get(url, verify=False) -}\n",
            ),
        ] {
            let findings = scan(lang, source);
            assert!(findings.is_empty(), "unexpected {lang} findings: {findings:?}");
        }
    }

    #[test]
    fn slash_comment_masking_covers_supported_language_aliases() {
        for lang in [
            "rust",
            "javascript",
            "typescript",
            "java",
            "c",
            "cpp",
            "csharp",
            "go",
            "swift",
            "scala",
            "php",
        ] {
            let findings = scan(
                lang,
                "// eval(user_input); password = \"hunter2\"; verify=false; md5(data)\n/* query(\"SELECT x FROM t\" + user) */\n",
            );
            assert!(
                findings.is_empty(),
                "unexpected {lang} findings: {findings:?}"
            );
        }
    }

    #[test]
    fn string_literals_cannot_create_line_heuristic_findings() {
        for (lang, source) in [
            (
                "python",
                "example = \"eval(user_input); requests.get(url, verify=False); hashlib.md5(data)\"\n",
            ),
            (
                "typescript",
                "const example = 'token assignment; db.query(SELECT + user); Math.random()';\n",
            ),
            (
                "rust",
                "let example = r#\"libc::system(user_input); InsecureSkipVerify: true; sha1(data)\"#;\n",
            ),
        ] {
            let findings = scan(lang, source);
            assert!(findings.is_empty(), "unexpected {lang} findings: {findings:?}");
        }
    }

    #[test]
    fn executable_findings_keep_original_snippets_and_real_sinks() {
        for (lang, source, rule) in [
            (
                "python",
                "requests.get(url, verify=False)\n",
                "tls_verify_disabled",
            ),
            ("python", "password = \"hunter2\"\n", "hardcoded_secret"),
            ("python", "eval(user_input)\n", "dangerous_eval"),
            ("javascript", "new Function(userInput)\n", "dangerous_eval"),
            (
                "ruby",
                "result = `echo #{user_input}`\n",
                "command_injection",
            ),
            (
                "rust",
                "Command::new(user_input).status();\n",
                "command_injection",
            ),
            ("python", "hashlib.md5(data)\n", "weak_crypto"),
        ] {
            let findings = scan(lang, source);
            let finding = findings
                .iter()
                .find(|finding| finding.rule == rule)
                .unwrap_or_else(|| panic!("missing {lang} {rule}: {findings:?}"));
            if lang == "rust" {
                assert_eq!(finding.snippet, "Command::new(user_input)");
            } else {
                assert_eq!(finding.snippet, source.trim());
            }
        }
    }

    #[test]
    fn clean_file_has_no_findings() {
        let text = "def add(a, b):\n    return a + b\n";
        assert!(scan("python", text).is_empty());
    }
}
