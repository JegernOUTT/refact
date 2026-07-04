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

    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();

        if has_tls_verify_disabled(trimmed, &lower) {
            push_finding(
                &mut findings,
                "tls_verify_disabled",
                Severity::High,
                line_no,
                trimmed,
            );
        }

        if let Some((key, value)) = assigned_string_literal(trimmed) {
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

        if line_has_sql_injection(trimmed, &lower) {
            push_finding(
                &mut findings,
                "sql_injection",
                Severity::High,
                line_no,
                trimmed,
            );
        }

        if line_has_command_injection(trimmed, &lower) {
            push_finding(
                &mut findings,
                "command_injection",
                Severity::High,
                line_no,
                trimmed,
            );
        }

        if line_has_dangerous_eval(trimmed, &lower) {
            push_finding(
                &mut findings,
                "dangerous_eval",
                Severity::High,
                line_no,
                trimmed,
            );
        }

        if line_has_weak_crypto(trimmed, &lower) {
            push_finding(
                &mut findings,
                "weak_crypto",
                Severity::Medium,
                line_no,
                trimmed,
            );
        }

        if line_has_insecure_random(trimmed, &lower) {
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
                    let line_no = node.start_position().row + 1;
                    let trimmed = call_text.trim();
                    let lower = trimmed.to_ascii_lowercase();
                    let callee = callee_name(trimmed);

                    if is_sql_sink(&callee) && call_contains_dynamic_sql(trimmed, &lower) {
                        push_finding(
                            &mut findings,
                            "sql_injection",
                            Severity::High,
                            line_no,
                            trimmed,
                        );
                    }

                    if is_command_sink(&callee, &lower) && call_has_non_literal_argument(trimmed) {
                        push_finding(
                            &mut findings,
                            "command_injection",
                            Severity::High,
                            line_no,
                            trimmed,
                        );
                    }

                    if call_has_dangerous_eval(&callee, trimmed, &lower) {
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

fn assigned_string_literal(line: &str) -> Option<(String, String)> {
    let eq = find_assignment_equals(line)?;
    let left = line[..eq].trim_end();
    let right = line[eq + 1..].trim_start();
    let key = last_identifier(left)?;
    let value = first_string_literal(right)?;
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

fn line_has_sql_injection(line: &str, lower: &str) -> bool {
    has_sink_call(lower)
        && has_sql_keyword(lower)
        && (line.contains('+') || has_interpolation(line))
        && has_identifier_outside_strings(line)
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

fn line_has_command_injection(line: &str, lower: &str) -> bool {
    if line_has_shell_backticks(line) {
        return has_identifier_outside_strings(line) || has_interpolation(line);
    }
    line_has_command_execution_call(lower) && call_has_non_literal_argument(line)
}

fn line_has_dangerous_eval(line: &str, lower: &str) -> bool {
    let dangerous_call = has_bare_call(lower, "eval")
        || (has_bare_call(lower, "exec") && !has_command_execution_context("", lower))
        || line.contains("Function(")
        || lower.contains("pickle.loads(")
        || lower.contains("marshal.load(")
        || lower.contains("deserialize(")
        || (lower.contains("yaml.load(") && !lower.contains("safeloader"));
    dangerous_call && call_has_external_input(line)
}

fn line_has_weak_crypto(_line: &str, lower: &str) -> bool {
    lower.contains("md5(")
        || lower.contains(".md5(")
        || lower.contains("sha1(")
        || lower.contains(".sha1(")
        || lower.contains("des(")
        || lower.contains("des.")
        || lower.contains("ecb")
        || lower.contains("math.random(")
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

fn is_command_sink(callee: &str, lower: &str) -> bool {
    has_command_execution_context(callee, lower)
}

fn call_contains_dynamic_sql(call_text: &str, lower: &str) -> bool {
    has_sql_keyword(lower)
        && (call_text.contains('+') || has_interpolation(call_text))
        && has_identifier_outside_strings(call_text)
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

fn call_has_dangerous_eval(callee: &str, call_text: &str, lower: &str) -> bool {
    let last = callee.rsplit('.').next().unwrap_or(callee);
    ((last == "eval")
        || (last == "exec" && !callee.contains('.') && !callee.contains(':'))
        || matches!(last, "function" | "deserialize")
        || callee.ends_with("pickle.loads")
        || callee.ends_with("marshal.load")
        || (callee.ends_with("yaml.load") && !lower.contains("safeloader")))
        && !has_command_execution_context(callee, lower)
        && call_has_external_input(call_text)
}

fn line_has_shell_backticks(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('`') && trimmed[1..].contains('`')
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

fn is_weak_crypto_callee(callee: &str, lower: &str) -> bool {
    let last = callee.rsplit('.').next().unwrap_or(callee);
    matches!(last, "md5" | "sha1" | "des")
        || lower.contains("math.random(")
        || lower.contains("ecb")
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
    fn detects_tls_verify_disabled() {
        let findings = scan("python", "requests.get(url, verify=False)\n");
        assert!(has_rule(&findings, "tls_verify_disabled"));
    }

    #[test]
    fn clean_file_has_no_findings() {
        let text = "def add(a, b):\n    return a + b\n";
        assert!(scan("python", text).is_empty());
    }
}
