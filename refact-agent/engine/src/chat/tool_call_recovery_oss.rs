use std::collections::HashSet;

use serde_json::{json, Value};
use tracing::info;

fn build_tool_call(name: &str, arguments: &str) -> Value {
    json!({
        "type": "function",
        "id": format!("call_{}", uuid::Uuid::new_v4().to_string().replace("-", "")),
        "function": {
            "name": name,
            "arguments": arguments,
        }
    })
}

fn extract_balanced_json_object(s: &str) -> Option<(String, usize, usize)> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for i in start..bytes.len() {
        let ch = bytes[i] as char;

        if escape_next {
            escape_next = false;
            continue;
        }
        if in_string && ch == '\\' {
            escape_next = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }

        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((s[start..=i].to_string(), start, i + 1));
                }
            }
            _ => {}
        }
    }

    None
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn recover_function_style(
    content: &str,
    allowed: &HashSet<String>,
) -> Option<(String, Vec<Value>, &'static str)> {
    // Collect all valid matches, then pick the one at the earliest position.
    // This avoids nondeterminism from HashSet iteration order when multiple
    // tools are in the allowed set.
    let mut best: Option<(usize, String, Vec<Value>)> = None;

    for name in allowed {
        let needle = format!("{}(", name);
        let Some(start) = content.find(&needle) else {
            continue;
        };

        // Word-boundary check: char immediately before the tool name must not
        // be alphanumeric or underscore. Prevents `notshell(` matching `shell(`.
        if start > 0 {
            let prev_char = content[..start].chars().next_back().unwrap_or(' ');
            if is_word_char(prev_char) {
                continue;
            }
        }

        let after = &content[start + name.len()..];
        let Some((json_args, _, end_rel)) = extract_balanced_json_object(after) else {
            continue;
        };
        let parsed: Value = match serde_json::from_str(&json_args) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if !parsed.is_object() {
            continue;
        }
        let tail = &after[end_rel..];
        let tail = tail.trim_start();
        let tail = tail.strip_prefix(')').unwrap_or(tail);
        let clean = format!("{}{}", &content[..start], tail).trim().to_string();

        // Prose guard: if there is a large amount of surrounding text after
        // removing the tool call, this is likely an explanation or example,
        // not an actual invocation. Reject if non-whitespace chars exceed 120.
        let prose_len = clean.chars().filter(|c| !c.is_whitespace()).count();
        if prose_len > 120 {
            continue;
        }

        if best.as_ref().map_or(true, |(pos, _, _)| start < *pos) {
            best = Some((start, clean, vec![build_tool_call(name, &json_args)]));
        }
    }

    best.map(|(_, clean, calls)| (clean, calls, "function_text_recovery"))
}

fn recover_fenced_json(
    content: &str,
    allowed: &HashSet<String>,
) -> Option<(String, Vec<Value>, &'static str)> {
    let fence = content.find("```")?;
    let rest = &content[fence + 3..];
    let end = rest.find("```")?;
    let block = rest[..end].trim();
    let json_start = block.find('{')?;
    let candidate = &block[json_start..];
    let parsed: Value = serde_json::from_str(candidate).ok()?;

    let name = parsed
        .get("tool")
        .or_else(|| parsed.get("name"))
        .and_then(|v| v.as_str())?;
    if !allowed.contains(name) {
        return None;
    }
    let arguments = parsed
        .get("arguments")
        .or_else(|| parsed.get("parameters"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !arguments.is_object() {
        return None;
    }
    let arguments_str = serde_json::to_string(&arguments).ok()?;
    let clean = format!("{}{}", &content[..fence], &rest[end + 3..])
        .trim()
        .to_string();
    Some((
        clean,
        vec![build_tool_call(name, &arguments_str)],
        "json_recovery",
    ))
}

fn recover_xml_tool(
    content: &str,
    allowed: &HashSet<String>,
) -> Option<(String, Vec<Value>, &'static str)> {
    let tag_start = content.find("<tool")?;
    let tag_end = content[tag_start..].find('>')? + tag_start;
    let header = &content[tag_start..=tag_end];
    let name_key = "name=\"";
    let name_pos = header.find(name_key)? + name_key.len();
    let name_end = header[name_pos..].find('"')? + name_pos;
    let name = &header[name_pos..name_end];
    if !allowed.contains(name) {
        return None;
    }
    let close = content[tag_end + 1..].find("</tool>")? + tag_end + 1;
    let body = content[tag_end + 1..close].trim();
    let parsed: Value = serde_json::from_str(body).ok()?;
    if !parsed.is_object() {
        return None;
    }
    let clean = format!("{}{}", &content[..tag_start], &content[close + 7..])
        .trim()
        .to_string();
    Some((clean, vec![build_tool_call(name, body)], "xml_recovery"))
}

pub fn recover_tool_calls_from_oss_text(
    content: &str,
    allowed: &HashSet<String>,
) -> Option<(String, Vec<Value>, &'static str)> {
    if content.is_empty() || allowed.is_empty() {
        return None;
    }

    let result = recover_function_style(content, allowed)
        .or_else(|| recover_fenced_json(content, allowed))
        .or_else(|| recover_xml_tool(content, allowed));

    if let Some((_, calls, source)) = &result {
        info!(
            "tool_call_recovery_oss: recovered {} tool call(s) via {}",
            calls.len(),
            source
        );
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_allowed(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_recover_function_style() {
        let allowed = make_allowed(&["shell"]);
        let content = r#"I'll do it now. shell({"command":"ls","workdir":"/tmp"})"#;
        let (clean, calls, source) = recover_tool_calls_from_oss_text(content, &allowed).unwrap();
        assert_eq!(source, "function_text_recovery");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "shell");
        assert!(!clean.contains("shell({"));
        assert!(!clean.contains(')'));
    }

    #[test]
    fn test_recover_function_style_checks_all_allowed_tools() {
        let allowed = make_allowed(&["shell", "apply_patch"]);
        let content = r#"apply_patch({"patch":"*** Begin Patch"})"#;
        let (_, calls, source) = recover_tool_calls_from_oss_text(content, &allowed).unwrap();
        assert_eq!(source, "function_text_recovery");
        assert_eq!(calls[0]["function"]["name"], "apply_patch");
    }

    #[test]
    fn test_recover_fenced_json() {
        let allowed = make_allowed(&["apply_patch"]);
        let content = "```json\n{\"tool\":\"apply_patch\",\"arguments\":{\"patch\":\"*** Begin Patch\"}}\n```";
        let (_, calls, source) = recover_tool_calls_from_oss_text(content, &allowed).unwrap();
        assert_eq!(source, "json_recovery");
        assert_eq!(calls[0]["function"]["name"], "apply_patch");
    }

    #[test]
    fn test_recover_xml_tool() {
        let allowed = make_allowed(&["search_pattern"]);
        let content = r#"<tool name="search_pattern">{"pattern":"abc","scope":"src/"}</tool>"#;
        let (_, calls, source) = recover_tool_calls_from_oss_text(content, &allowed).unwrap();
        assert_eq!(source, "xml_recovery");
        assert_eq!(calls[0]["function"]["name"], "search_pattern");
    }

    #[test]
    fn test_reject_unknown_tool() {
        let allowed = make_allowed(&["shell"]);
        let content = r#"dangerous_tool({"x":1})"#;
        assert!(recover_tool_calls_from_oss_text(content, &allowed).is_none());
    }

    #[test]
    fn test_no_match_inside_larger_identifier() {
        // "notshell(" contains "shell(" as a substring — must NOT match
        let allowed = make_allowed(&["shell"]);
        let content = r#"notshell({"command":"ls"})"#;
        assert!(recover_tool_calls_from_oss_text(content, &allowed).is_none());
    }

    #[test]
    fn test_earliest_match_wins_regardless_of_set_iteration_order() {
        // Content has apply_patch first, then shell.
        // Whichever name happens to be iterated first from the HashSet,
        // the result must always be apply_patch (the earlier one in text).
        let allowed = make_allowed(&["shell", "apply_patch"]);
        let content = r#"apply_patch({"patch":"x"}) and shell({"command":"ls","workdir":"/"})"#;
        let (_, calls, _) = recover_tool_calls_from_oss_text(content, &allowed).unwrap();
        assert_eq!(calls[0]["function"]["name"], "apply_patch");
    }

    #[test]
    fn test_prose_guard_rejects_heavy_surrounding_text() {
        // The tool call is embedded in a long explanation — should NOT recover
        let allowed = make_allowed(&["shell"]);
        let content = "If you want to list the files in the temporary directory, \
            you can use shell({\"command\":\"ls\",\"workdir\":\"/tmp\"}) to do so. \
            This is a very common operation that you might want to perform when \
            debugging or inspecting the state of the system.";
        assert!(recover_tool_calls_from_oss_text(content, &allowed).is_none());
    }

    #[test]
    fn test_prose_guard_allows_short_prefix() {
        // Small prefix like "Running:" is fine (< 120 non-ws chars after removal)
        let allowed = make_allowed(&["shell"]);
        let content = r#"Running: shell({"command":"ls","workdir":"/tmp"})"#;
        let result = recover_tool_calls_from_oss_text(content, &allowed);
        assert!(result.is_some());
    }

    #[test]
    fn test_clean_content_has_no_stray_paren() {
        let allowed = make_allowed(&["shell"]);
        let content = r#"shell({"command":"ls","workdir":"/tmp"})"#;
        let (clean, _, _) = recover_tool_calls_from_oss_text(content, &allowed).unwrap();
        assert!(
            !clean.contains(')'),
            "clean content must not have stray closing paren"
        );
    }
}
