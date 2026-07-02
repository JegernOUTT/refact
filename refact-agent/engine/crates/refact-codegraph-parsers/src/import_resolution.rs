use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathMappings {
    pub base_url: String,
    pub paths: Vec<(String, String)>,
}

pub fn parse_tsconfig(json_text: &str) -> PathMappings {
    let text = strip_json_comments(json_text);
    let compiler = find_object_for_key(&text, "compilerOptions").unwrap_or_else(|| text.clone());
    let base_url = find_string_for_key(&compiler, "baseUrl").unwrap_or_default();
    let mut paths = Vec::new();

    if let Some(paths_object) = find_object_for_key(&compiler, "paths") {
        let mut i = 0;
        while i < paths_object.len() {
            if let Some((key, after_key)) = parse_next_json_string(&paths_object, i) {
                i = skip_ws(&paths_object, after_key);
                if !matches!(paths_object.as_bytes().get(i), Some(b':')) {
                    i = after_key;
                    continue;
                }
                i = skip_ws(&paths_object, i + 1);
                if let Some((value, next)) = parse_first_string_value(&paths_object, i) {
                    paths.push((key, value));
                    i = next;
                } else {
                    i += 1;
                }
            } else {
                break;
            }
        }
    }

    PathMappings { base_url, paths }
}

pub fn resolve_ts_import(spec: &str, mappings: &PathMappings) -> Option<String> {
    if spec.starts_with("./") || spec.starts_with("../") {
        return None;
    }

    for (pattern, target) in &mappings.paths {
        if let Some(captured) = match_ts_pattern(spec, pattern) {
            let mapped = apply_ts_target(target, captured);
            return Some(apply_ts_base_url(&mappings.base_url, &mapped));
        }
    }

    if mappings.base_url.is_empty() {
        Some(spec.to_string())
    } else {
        Some(join_path(&mappings.base_url, spec))
    }
}

pub fn parse_go_mod(text: &str) -> Option<String> {
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("module")
            && line["module".len()..]
                .chars()
                .next()
                .map_or(true, char::is_whitespace)
        {
            let value = line["module".len()..].trim();
            if !value.is_empty() {
                return Some(value.split_whitespace().next().unwrap_or(value).to_string());
            }
        }
    }
    None
}

pub fn resolve_go_import(spec: &str, module_path: &str) -> Option<String> {
    if spec == module_path {
        return Some(String::new());
    }
    let prefix = format!("{}/", module_path.trim_end_matches('/'));
    spec.strip_prefix(&prefix).map(|s| s.to_string())
}

pub fn parse_cargo_toml(text: &str) -> (Option<String>, Vec<String>) {
    let mut section = String::new();
    let mut package_name = None;
    let mut members = Vec::new();
    let mut in_members = false;
    let mut members_buf = String::new();

    for raw in text.lines() {
        let line = strip_toml_inline_comment(raw).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if in_members {
            members_buf.push(' ');
            members_buf.push_str(&line);
            if line.contains(']') {
                members.extend(parse_toml_string_array(&members_buf));
                in_members = false;
                members_buf.clear();
            }
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line.trim_matches(&['[', ']'][..]).trim().to_string();
            continue;
        }
        if section == "package" && package_name.is_none() {
            if let Some(value) = parse_assignment_string(&line, "name") {
                package_name = Some(value);
            }
        } else if section == "workspace" && starts_assignment(&line, "members") {
            if let Some(start) = line.find('[') {
                members_buf.push_str(&line[start..]);
                if line[start..].contains(']') {
                    members.extend(parse_toml_string_array(&members_buf));
                    members_buf.clear();
                } else {
                    in_members = true;
                }
            }
        }
    }

    (package_name, members)
}

pub fn parse_package_json(text: &str) -> (Option<String>, Option<String>) {
    let text = strip_json_comments(text);
    let name = find_string_for_key(&text, "name");
    let entry = find_string_for_key(&text, "main").or_else(|| find_string_for_key(&text, "module"));
    (name, entry)
}

pub fn resolve_barrel(
    spec: &str,
    barrel_reexports: &HashMap<String, String>,
    max_hops: usize,
) -> String {
    let limit = if max_hops == 0 { 5 } else { max_hops };
    let mut current = spec.to_string();
    let mut seen = HashSet::new();

    for _ in 0..limit {
        if !seen.insert(current.clone()) {
            break;
        }
        if let Some(next) = barrel_reexports.get(&current) {
            current = next.clone();
        } else {
            break;
        }
    }

    current
}

fn strip_json_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for n in chars.by_ref() {
                if n == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = '\0';
            for n in chars.by_ref() {
                if prev == '*' && n == '/' {
                    break;
                }
                prev = n;
            }
        } else {
            out.push(c);
        }
    }

    out
}

fn find_object_for_key(text: &str, key: &str) -> Option<String> {
    let mut i = 0;
    while let Some((found_key, after_key)) = parse_next_json_string(text, i) {
        i = skip_ws(text, after_key);
        if found_key == key && matches!(text.as_bytes().get(i), Some(b':')) {
            i = skip_ws(text, i + 1);
            if matches!(text.as_bytes().get(i), Some(b'{')) {
                let end = find_matching(text, i, b'{', b'}')?;
                return Some(text[i + 1..end].to_string());
            }
        } else {
            i = after_key;
        }
    }
    None
}

fn find_string_for_key(text: &str, key: &str) -> Option<String> {
    let mut i = 0;
    while let Some((found_key, after_key)) = parse_next_json_string(text, i) {
        i = skip_ws(text, after_key);
        if found_key == key && matches!(text.as_bytes().get(i), Some(b':')) {
            i = skip_ws(text, i + 1);
            if let Some((value, _)) = parse_json_string_at(text, i) {
                return Some(value);
            }
        } else {
            i = after_key;
        }
    }
    None
}

fn parse_first_string_value(text: &str, i: usize) -> Option<(String, usize)> {
    let i = skip_ws(text, i);
    if matches!(text.as_bytes().get(i), Some(b'[')) {
        let mut j = i + 1;
        loop {
            j = skip_ws(text, j);
            match text.as_bytes().get(j) {
                Some(b'"') => return parse_json_string_at(text, j),
                Some(b']') | None => return None,
                _ => j += 1,
            }
        }
    }
    parse_json_string_at(text, i)
}

fn parse_next_json_string(text: &str, start: usize) -> Option<(String, usize)> {
    let mut i = start;
    while i < text.len() {
        if matches!(text.as_bytes().get(i), Some(b'"')) {
            return parse_json_string_at(text, i);
        }
        i += 1;
    }
    None
}

fn parse_json_string_at(text: &str, start: usize) -> Option<(String, usize)> {
    if !matches!(text.as_bytes().get(start), Some(b'"')) {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    let mut i = start + 1;

    while i < text.len() {
        let c = text[i..].chars().next()?;
        i += c.len_utf8();
        if escaped {
            match c {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'b' => out.push('\u{0008}'),
                'f' => out.push('\u{000c}'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    if i + 4 <= text.len() {
                        if let Ok(value) = u16::from_str_radix(&text[i..i + 4], 16) {
                            if let Some(ch) = char::from_u32(value as u32) {
                                out.push(ch);
                            }
                        }
                        i += 4;
                    }
                }
                _ => out.push(c),
            }
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Some((out, i));
        } else {
            out.push(c);
        }
    }
    None
}

fn skip_ws(text: &str, mut i: usize) -> usize {
    while i < text.len() && text.as_bytes()[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn find_matching(text: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start;
    let mut in_string = false;
    let mut escaped = false;

    while i < text.len() {
        let b = text.as_bytes()[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
        } else if b == b'"' {
            in_string = true;
        } else if b == open {
            depth += 1;
        } else if b == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn match_ts_pattern<'a>(spec: &'a str, pattern: &str) -> Option<Option<&'a str>> {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        if spec == prefix {
            return Some(Some(""));
        }
        return spec.strip_prefix(&format!("{}/", prefix)).map(Some);
    }
    if let Some(star) = pattern.find('*') {
        let prefix = &pattern[..star];
        let suffix = &pattern[star + 1..];
        if spec.starts_with(prefix)
            && spec.ends_with(suffix)
            && spec.len() >= prefix.len() + suffix.len()
        {
            return Some(Some(&spec[prefix.len()..spec.len() - suffix.len()]));
        }
        return None;
    }
    (spec == pattern).then_some(None)
}

fn apply_ts_target(target: &str, captured: Option<&str>) -> String {
    match captured {
        Some(value) => {
            if target.contains('*') {
                target.replacen('*', value, 1)
            } else if value.is_empty() {
                target.trim_end_matches('/').to_string()
            } else {
                join_path(target.trim_end_matches("/*").trim_end_matches('/'), value)
            }
        }
        None => target.to_string(),
    }
}

fn apply_ts_base_url(base_url: &str, target: &str) -> String {
    let target = target.strip_prefix("./").unwrap_or(target);
    if base_url.is_empty() || target.starts_with('/') {
        target.to_string()
    } else {
        join_path(base_url, target)
    }
}

fn join_path(base: &str, tail: &str) -> String {
    if base.is_empty() {
        tail.trim_start_matches('/').to_string()
    } else if tail.is_empty() {
        base.trim_end_matches('/').to_string()
    } else {
        format!(
            "{}/{}",
            base.trim_end_matches('/'),
            tail.trim_start_matches('/')
        )
    }
}

fn strip_toml_inline_comment(line: &str) -> String {
    let mut out = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for c in line.chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '#' {
            break;
        } else {
            out.push(c);
        }
    }
    out
}

fn starts_assignment(line: &str, key: &str) -> bool {
    line.strip_prefix(key)
        .map(|rest| rest.trim_start().starts_with('='))
        .unwrap_or(false)
}

fn parse_assignment_string(line: &str, key: &str) -> Option<String> {
    if !starts_assignment(line, key) {
        return None;
    }
    let value = line.split_once('=')?.1.trim();
    parse_json_string_at(value, 0).map(|(s, _)| s)
}

fn parse_toml_string_array(text: &str) -> Vec<String> {
    let start = match text.find('[') {
        Some(pos) => pos,
        None => return Vec::new(),
    };
    let end = match text.rfind(']') {
        Some(pos) if pos > start => pos,
        _ => text.len(),
    };
    let body = &text[start + 1..end];
    let mut values = Vec::new();
    let mut i = 0;
    while let Some((value, next)) = parse_next_json_string(body, i) {
        values.push(value);
        i = next;
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_resolves_tsconfig() {
        let mappings = parse_tsconfig(
            r#"{"compilerOptions":{"baseUrl":"src","paths":{"@app/*":["app/*"],"@lib":["lib/index.ts"]}}}"#,
        );
        assert_eq!(mappings.base_url, "src");
        assert_eq!(mappings.paths.len(), 2);
        assert_eq!(
            resolve_ts_import("@app/user/service", &mappings).as_deref(),
            Some("src/app/user/service")
        );
        assert_eq!(
            resolve_ts_import("@lib", &mappings).as_deref(),
            Some("src/lib/index.ts")
        );
    }

    #[test]
    fn resolves_ts_paths_without_base_url() {
        let mappings = parse_tsconfig(
            r#"{"compilerOptions":{"paths":{"@app/*":["app/*"],"@lib":["lib/index.ts"]}}}"#,
        );
        assert_eq!(
            resolve_ts_import("@app/user/service", &mappings).as_deref(),
            Some("app/user/service")
        );
        assert_eq!(
            resolve_ts_import("@lib", &mappings).as_deref(),
            Some("lib/index.ts")
        );
    }

    #[test]
    fn parses_and_resolves_go_module_imports() {
        let module = parse_go_mod("module github.com/acme/proj\n\ngo 1.21");
        assert_eq!(module.as_deref(), Some("github.com/acme/proj"));
        assert_eq!(
            resolve_go_import("github.com/acme/proj/pkg/db", "github.com/acme/proj").as_deref(),
            Some("pkg/db")
        );
    }

    #[test]
    fn parses_cargo_package_and_workspace_members() {
        let text = r#"
[package]
name = "mycrate"

[workspace]
members = ["crates/a", "crates/b"]
"#;
        let (name, members) = parse_cargo_toml(text);
        assert_eq!(name.as_deref(), Some("mycrate"));
        assert_eq!(
            members,
            vec!["crates/a".to_string(), "crates/b".to_string()]
        );
    }

    #[test]
    fn parses_package_json_name_and_entry() {
        let (name, entry) =
            parse_package_json(r#"{"name":"pkg","module":"dist/index.mjs","main":"index.js"}"#);
        assert_eq!(name.as_deref(), Some("pkg"));
        assert_eq!(entry.as_deref(), Some("index.js"));
    }

    #[test]
    fn follows_barrel_reexports() {
        let mut reexports = HashMap::new();
        reexports.insert("a".to_string(), "b".to_string());
        reexports.insert("b".to_string(), "c".to_string());
        assert_eq!(resolve_barrel("a", &reexports, 5), "c");
    }
}
