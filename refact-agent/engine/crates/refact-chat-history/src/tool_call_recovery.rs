use std::collections::HashSet;
use serde_json::{json, Value};
use tracing::{info, warn};

/// Build a set of allowed tool names from the LLM request tools.
/// Tools are expected to be OpenAI-style: `{"type":"function","function":{"name":"..."}}`
pub fn allowed_tool_names(tools: &Option<Vec<Value>>) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Some(tools) = tools {
        for tool in tools {
            if let Some(name) = tool
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
            {
                names.insert(name.to_string());
            }
            // Responses API format: {"type":"function","name":"..."}
            if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                names.insert(name.to_string());
            }
        }
    }
    names
}

/// Attempt to recover tool calls from garbled ChatML content.
///
/// GPT-5 Codex models occasionally leak tool calls into the text content instead of
/// emitting structured function_call events. The pattern looks like:
///
/// ```text
/// <clean preamble>...<garbled tokens>assistant to=functions.<tool_name> <garbled>json
/// {"command":"...","workdir":"..."}
/// ```
///
/// This function detects the `to=functions.{name}` pattern, extracts the JSON arguments,
/// validates against allowed tools, and returns synthetic tool call Values.
///
/// Returns `Some((cleaned_content, tool_calls))` if recovery succeeded, `None` otherwise.
pub fn recover_tool_calls_from_chatml_content(
    content: &str,
    allowed: &HashSet<String>,
) -> Option<(String, Vec<Value>)> {
    if content.is_empty() || allowed.is_empty() {
        return None;
    }

    // Pattern: to=functions.{name} or to=multi_tool_use.parallel
    let chatml_single = find_chatml_single_tool(content, allowed);
    let chatml_multi = find_chatml_multi_tool(content, allowed);

    if let Some((clean, calls)) = chatml_multi {
        if !calls.is_empty() {
            return Some((clean, calls));
        }
    }

    if let Some((clean, calls)) = chatml_single {
        if !calls.is_empty() {
            return Some((clean, calls));
        }
    }

    None
}

/// Detect `to=functions.{name}` followed by JSON arguments.
fn find_chatml_single_tool(
    content: &str,
    allowed: &HashSet<String>,
) -> Option<(String, Vec<Value>)> {
    let marker = "to=functions.";
    let marker_pos = content.find(marker)?;
    let after_marker = &content[marker_pos + marker.len()..];

    // Extract tool name (alphanumeric + underscore until whitespace or non-word char)
    let name_end = after_marker
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(after_marker.len());
    if name_end == 0 {
        return None;
    }
    let tool_name = &after_marker[..name_end];

    if !allowed.contains(tool_name) {
        warn!(
            "tool_call_recovery: leaked tool name '{}' not in allowed set, skipping",
            tool_name
        );
        return None;
    }

    // Find JSON arguments after the tool name
    let rest = &after_marker[name_end..];
    let json_args = extract_first_json_object(rest)?;

    // Validate JSON parses
    let parsed: Value = serde_json::from_str(&json_args).ok()?;
    if !parsed.is_object() {
        return None;
    }

    // Build synthetic tool call
    let call_id = format!("call_{}", uuid::Uuid::new_v4().to_string().replace("-", ""));
    let tool_call = json!({
        "type": "function",
        "id": call_id,
        "function": {
            "name": tool_name,
            "arguments": json_args,
        }
    });

    // Clean content: keep text before the garbled section
    let clean = extract_clean_preamble(content, marker_pos);

    info!(
        "tool_call_recovery: recovered '{}' tool call from ChatML-leaked content ({} chars args)",
        tool_name,
        json_args.len()
    );

    Some((clean, vec![tool_call]))
}

/// Detect `to=multi_tool_use.parallel` followed by `{"tool_uses":[...]}`.
fn find_chatml_multi_tool(
    content: &str,
    allowed: &HashSet<String>,
) -> Option<(String, Vec<Value>)> {
    let marker = "to=multi_tool_use.parallel";
    let marker_pos = content.find(marker)?;
    let rest = &content[marker_pos + marker.len()..];

    // Find the JSON object containing tool_uses
    let json_str = extract_first_json_object(rest)?;
    let parsed: Value = serde_json::from_str(&json_str).ok()?;

    let tool_uses = parsed.get("tool_uses").and_then(|v| v.as_array())?;

    if tool_uses.is_empty() {
        return None;
    }

    let mut calls = Vec::new();
    for (i, use_item) in tool_uses.iter().enumerate() {
        // Format: {"recipient_name":"functions.{name}","parameters":{...}}
        let recipient = use_item
            .get("recipient_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let tool_name = recipient.strip_prefix("functions.").unwrap_or(recipient);

        if tool_name.is_empty() || !allowed.contains(tool_name) {
            warn!(
                "tool_call_recovery: multi_tool_use inner tool '{}' not in allowed set, skipping",
                tool_name
            );
            continue;
        }

        let parameters = use_item.get("parameters").cloned().unwrap_or(json!({}));
        let args_str = if parameters.is_object() {
            serde_json::to_string(&parameters).unwrap_or_else(|_| "{}".to_string())
        } else if let Some(s) = parameters.as_str() {
            s.to_string()
        } else {
            "{}".to_string()
        };

        let call_id = format!(
            "call_{}_{}",
            uuid::Uuid::new_v4().to_string().replace("-", ""),
            i
        );
        calls.push(json!({
            "type": "function",
            "id": call_id,
            "index": i,
            "function": {
                "name": tool_name,
                "arguments": args_str,
            }
        }));
    }

    if calls.is_empty() {
        return None;
    }

    let clean = extract_clean_preamble(content, marker_pos);

    info!(
        "tool_call_recovery: recovered {} tool calls from multi_tool_use.parallel in ChatML-leaked content",
        calls.len()
    );

    Some((clean, calls))
}

/// Unwrap `multi_tool_use.parallel` wrapper tool calls in the structured tool_calls array.
///
/// Some OpenAI models (via Chat Completions API) emit a single tool call to
/// `multi_tool_use.parallel` whose arguments contain an array of individual tool calls.
/// This function expands them into separate tool call entries.
pub fn unwrap_multi_tool_use_parallel(
    tool_calls_raw: &[Value],
    allowed: &HashSet<String>,
) -> Vec<Value> {
    let mut result = Vec::new();
    let mut any_unwrapped = false;

    for tc in tool_calls_raw {
        let name = tc
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("");

        if name != "multi_tool_use.parallel" {
            result.push(tc.clone());
            continue;
        }

        let args_str = tc
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|a| a.as_str())
            .unwrap_or("{}");

        let args: Value = match serde_json::from_str(args_str) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    "tool_call_recovery: failed to parse multi_tool_use.parallel arguments: {}",
                    e
                );
                result.push(tc.clone());
                continue;
            }
        };

        // Accept both {"tool_uses":[...]} and direct array [...]
        let inner_calls = args
            .get("tool_uses")
            .and_then(|v| v.as_array())
            .or_else(|| args.as_array())
            .cloned();

        let inner_calls = match inner_calls {
            Some(c) if !c.is_empty() => c,
            _ => {
                warn!("tool_call_recovery: multi_tool_use.parallel has no parsable inner calls");
                result.push(tc.clone());
                continue;
            }
        };

        let wrapper_id = tc
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("call_multi");

        for (i, inner) in inner_calls.iter().enumerate() {
            // Format variants: {recipient_name:"functions.X", parameters:{...}}
            //                  {tool_name:"X", tool_input:{...}}
            //                  {name:"X", arguments:{...}}
            let tool_name = extract_inner_tool_name(inner);
            if tool_name.is_empty() {
                warn!(
                    "tool_call_recovery: multi_tool_use inner call {} has no tool name, skipping",
                    i
                );
                continue;
            }

            if !allowed.is_empty() && !allowed.contains(&tool_name) {
                warn!(
                    "tool_call_recovery: multi_tool_use inner tool '{}' not in allowed set, skipping",
                    tool_name
                );
                continue;
            }

            let args_value = extract_inner_tool_args(inner);
            let args_str = if args_value.is_object() {
                serde_json::to_string(&args_value).unwrap_or_else(|_| "{}".to_string())
            } else if let Some(s) = args_value.as_str() {
                s.to_string()
            } else {
                "{}".to_string()
            };

            let child_id = format!("{}__{}", wrapper_id, i);
            result.push(json!({
                "type": "function",
                "id": child_id,
                "index": result.len(),
                "function": {
                    "name": tool_name,
                    "arguments": args_str,
                }
            }));
        }

        any_unwrapped = true;
    }

    if any_unwrapped {
        // Re-index sequentially
        for (i, tc) in result.iter_mut().enumerate() {
            if let Some(obj) = tc.as_object_mut() {
                obj.insert("index".to_string(), json!(i));
            }
        }
        info!(
            "tool_call_recovery: unwrapped multi_tool_use.parallel into {} tool calls",
            result.len()
        );
    }

    result
}

/// Extract tool name from various inner-call formats.
fn extract_inner_tool_name(inner: &Value) -> String {
    // Format 1: {"recipient_name":"functions.X", ...}
    if let Some(recipient) = inner.get("recipient_name").and_then(|v| v.as_str()) {
        if let Some(name) = recipient.strip_prefix("functions.") {
            return name.to_string();
        }
        return recipient.to_string();
    }
    // Format 2: {"tool_name":"X", ...}
    if let Some(name) = inner.get("tool_name").and_then(|v| v.as_str()) {
        return name.to_string();
    }
    // Format 3: {"name":"X", ...}
    if let Some(name) = inner.get("name").and_then(|v| v.as_str()) {
        return name.to_string();
    }
    // Format 4: {"function":{"name":"X"}, ...}
    if let Some(name) = inner
        .get("function")
        .and_then(|f| f.get("name"))
        .and_then(|n| n.as_str())
    {
        return name.to_string();
    }
    String::new()
}

/// Extract tool arguments from various inner-call formats.
fn extract_inner_tool_args(inner: &Value) -> Value {
    // Format 1: {"parameters":{...}}
    if let Some(params) = inner.get("parameters") {
        return params.clone();
    }
    // Format 2: {"tool_input":{...}}
    if let Some(input) = inner.get("tool_input") {
        return input.clone();
    }
    // Format 3: {"arguments":"..." or {...}}
    if let Some(args) = inner.get("arguments") {
        return args.clone();
    }
    // Format 4: {"function":{"arguments":"..."}}
    if let Some(args) = inner.get("function").and_then(|f| f.get("arguments")) {
        return args.clone();
    }
    json!({})
}

/// Extract the first valid JSON object from a string, using bracket matching.
/// Handles nested objects and strings with escaped characters.
fn extract_first_json_object(s: &str) -> Option<String> {
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

        if ch == '\\' && in_string {
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
                    return Some(s[start..=i].to_string());
                }
            }
            _ => {}
        }
    }

    None
}

/// Extract clean text content from before the garbled ChatML section.
///
/// The garbled pattern typically looks like:
///   `<clean text>. <garbled> assistant to=functions.X <garbled> json\n{...}`
/// or:
///   `<clean text>. +#+#+#+#+#+assistant to=functions.X <garbled> json\n{...}`
///
/// We find `assistant` right before `to=` and strip from there (including any
/// preceding garbled/decorative tokens).
fn extract_clean_preamble(content: &str, marker_pos: usize) -> String {
    let before = &content[..marker_pos];

    // Find "assistant" that immediately precedes "to=" (with optional whitespace)
    let assistant_pos = before.rfind("assistant");
    let cut_pos = if let Some(pos) = assistant_pos {
        // Walk backwards from "assistant" to skip garbled non-ASCII and decorative tokens
        // like +#+#, CJK, Armenian that bridge clean text to the ChatML marker
        let pre_assistant = &before[..pos];
        let clean_end = pre_assistant
            .char_indices()
            .rev()
            .find(|(_, c)| {
                c.is_ascii_alphanumeric() && *c != '#'
                    || (*c == '.' || *c == '!' || *c == '?' || *c == ')' || *c == '"' || *c == '\'')
            })
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        clean_end
    } else {
        // No "assistant" found — fall back to stripping non-ASCII from the end
        before
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_ascii_alphanumeric() || *c == '.' || *c == '!' || *c == '?')
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0)
    };

    before[..cut_pos].trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_allowed(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn make_tools_json(names: &[&str]) -> Option<Vec<Value>> {
        Some(
            names
                .iter()
                .map(|name| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": name,
                            "description": "test tool",
                            "parameters": {"type": "object", "properties": {}}
                        }
                    })
                })
                .collect(),
        )
    }

    // ========== allowed_tool_names ==========

    #[test]
    fn test_allowed_tool_names_from_openai_format() {
        let tools = make_tools_json(&["shell", "cat", "apply_patch"]);
        let names = allowed_tool_names(&tools);
        assert!(names.contains("shell"));
        assert!(names.contains("cat"));
        assert!(names.contains("apply_patch"));
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn test_allowed_tool_names_empty() {
        let names = allowed_tool_names(&None);
        assert!(names.is_empty());
    }

    // ========== extract_first_json_object ==========

    #[test]
    fn test_extract_json_simple() {
        let s = r#"some garbage {"key":"value"} more"#;
        assert_eq!(extract_first_json_object(s).unwrap(), r#"{"key":"value"}"#);
    }

    #[test]
    fn test_extract_json_nested() {
        let s = r#"prefix {"outer":{"inner":42}} suffix"#;
        assert_eq!(
            extract_first_json_object(s).unwrap(),
            r#"{"outer":{"inner":42}}"#
        );
    }

    #[test]
    fn test_extract_json_with_escaped_braces_in_string() {
        let s = r#"x {"patch":"line1\nline2","b":1} y"#;
        assert_eq!(
            extract_first_json_object(s).unwrap(),
            r#"{"patch":"line1\nline2","b":1}"#
        );
    }

    #[test]
    fn test_extract_json_none_when_no_brace() {
        assert!(extract_first_json_object("no json here").is_none());
    }

    #[test]
    fn test_extract_json_none_when_unclosed() {
        assert!(extract_first_json_object("start {\"key\":\"val").is_none());
    }

    // ========== extract_clean_preamble ==========

    #[test]
    fn test_clean_preamble_strips_garbled() {
        let content = "I'll run the test.րցassistant to=functions.shell rest";
        let marker_pos = content.find("to=functions.shell").unwrap();
        let clean = extract_clean_preamble(content, marker_pos);
        assert_eq!(clean, "I'll run the test.");
    }

    #[test]
    fn test_clean_preamble_with_plus_signs() {
        let content = "I'll patch that safely. +#+#+#+#+#+assistant to=functions.apply_patch rest";
        let marker_pos = content.find("to=functions.apply_patch").unwrap();
        let clean = extract_clean_preamble(content, marker_pos);
        assert_eq!(clean, "I'll patch that safely.");
    }

    // ========== recover_tool_calls_from_chatml_content ==========

    #[test]
    fn test_recover_single_shell_tool() {
        let content = concat!(
            "I'll run the new stress test and then type-check.",
            "րցassistant to=functions.shell մեկusage 天天中彩票 JSON\n",
            r#"{"command":"npm run test","workdir":"/home/user/project","timeout":"600"}"#
        );
        let allowed = make_allowed(&["shell", "cat"]);
        let result = recover_tool_calls_from_chatml_content(content, &allowed);
        assert!(result.is_some());
        let (clean, calls) = result.unwrap();
        assert_eq!(calls.len(), 1);

        let tc = &calls[0];
        assert_eq!(tc["function"]["name"], "shell");
        let args: Value =
            serde_json::from_str(tc["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["command"], "npm run test");
        assert_eq!(args["workdir"], "/home/user/project");

        assert!(clean.contains("type-check."));
        assert!(!clean.contains("մեկ"));
    }

    #[test]
    fn test_recover_apply_patch_tool() {
        let content = concat!(
            "I'll patch this. +#+#+#+#+#+assistant to=functions.apply_patch մեdata json\n",
            r#"{"patch": "*** Begin Patch\n*** Update File: /src/main.rs\n@@\n-old\n+new\n"}"#
        );
        let allowed = make_allowed(&["apply_patch", "shell"]);
        let result = recover_tool_calls_from_chatml_content(content, &allowed);
        assert!(result.is_some());
        let (_, calls) = result.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "apply_patch");
    }

    #[test]
    fn test_recover_rejects_unknown_tool() {
        let content = "text assistant to=functions.dangerous_tool garble json\n{\"x\":1}";
        let allowed = make_allowed(&["shell", "cat"]);
        let result = recover_tool_calls_from_chatml_content(content, &allowed);
        assert!(result.is_none());
    }

    #[test]
    fn test_recover_rejects_no_json_args() {
        let content = "text assistant to=functions.shell garble without json";
        let allowed = make_allowed(&["shell"]);
        let result = recover_tool_calls_from_chatml_content(content, &allowed);
        assert!(result.is_none());
    }

    #[test]
    fn test_recover_empty_content() {
        let allowed = make_allowed(&["shell"]);
        assert!(recover_tool_calls_from_chatml_content("", &allowed).is_none());
    }

    #[test]
    fn test_recover_normal_content_no_false_positive() {
        let content = "This is a normal assistant response about shell commands and functions.";
        let allowed = make_allowed(&["shell", "cat"]);
        let result = recover_tool_calls_from_chatml_content(content, &allowed);
        assert!(result.is_none());
    }

    // ========== multi_tool_use.parallel in content ==========

    #[test]
    fn test_recover_multi_tool_from_content() {
        let content = concat!(
            "I'll search now. +#+#+assistant to=multi_tool_use.parallel garble json\n",
            r#"{"tool_uses":[{"recipient_name":"functions.search_pattern","parameters":{"pattern":"test","scope":"src/"}},{"recipient_name":"functions.tree","parameters":{"path":"/src"}}]}"#
        );
        let allowed = make_allowed(&["search_pattern", "tree", "cat"]);
        let result = recover_tool_calls_from_chatml_content(content, &allowed);
        assert!(result.is_some());
        let (clean, calls) = result.unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["function"]["name"], "search_pattern");
        assert_eq!(calls[1]["function"]["name"], "tree");
        assert!(clean.contains("search now."));
    }

    // ========== unwrap_multi_tool_use_parallel (structured) ==========

    #[test]
    fn test_unwrap_parallel_recipient_name_format() {
        let wrapper = json!({
            "type": "function",
            "id": "call_abc123",
            "function": {
                "name": "multi_tool_use.parallel",
                "arguments": r#"{"tool_uses":[{"recipient_name":"functions.cat","parameters":{"path":"/file.rs"}},{"recipient_name":"functions.tree","parameters":{"path":"/src"}}]}"#
            }
        });
        let allowed = make_allowed(&["cat", "tree"]);
        let result = unwrap_multi_tool_use_parallel(&[wrapper], &allowed);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["function"]["name"], "cat");
        assert_eq!(result[1]["function"]["name"], "tree");
        assert_eq!(result[0]["id"], "call_abc123__0");
        assert_eq!(result[1]["id"], "call_abc123__1");
        assert_eq!(result[0]["index"], 0);
        assert_eq!(result[1]["index"], 1);
    }

    #[test]
    fn test_unwrap_parallel_tool_name_format() {
        let wrapper = json!({
            "type": "function",
            "id": "call_xyz",
            "function": {
                "name": "multi_tool_use.parallel",
                "arguments": r#"{"tool_uses":[{"tool_name":"shell","tool_input":{"command":"ls"}},{"tool_name":"cat","tool_input":{"path":"/a.rs"}}]}"#
            }
        });
        let allowed = make_allowed(&["shell", "cat"]);
        let result = unwrap_multi_tool_use_parallel(&[wrapper], &allowed);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["function"]["name"], "shell");
        assert_eq!(result[1]["function"]["name"], "cat");
    }

    #[test]
    fn test_unwrap_parallel_mixed_with_normal() {
        let normal = json!({
            "type": "function",
            "id": "call_normal",
            "function": {"name": "shell", "arguments": r#"{"command":"echo hi"}"#}
        });
        let wrapper = json!({
            "type": "function",
            "id": "call_wrap",
            "function": {
                "name": "multi_tool_use.parallel",
                "arguments": r#"{"tool_uses":[{"recipient_name":"functions.cat","parameters":{"path":"/f"}}]}"#
            }
        });
        let allowed = make_allowed(&["shell", "cat"]);
        let result = unwrap_multi_tool_use_parallel(&[normal, wrapper], &allowed);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["function"]["name"], "shell");
        assert_eq!(result[1]["function"]["name"], "cat");
        // Re-indexed
        assert_eq!(result[0]["index"], 0);
        assert_eq!(result[1]["index"], 1);
    }

    #[test]
    fn test_unwrap_parallel_invalid_json() {
        let wrapper = json!({
            "type": "function",
            "id": "call_bad",
            "function": {
                "name": "multi_tool_use.parallel",
                "arguments": "{broken json"
            }
        });
        let allowed = make_allowed(&["shell"]);
        let result = unwrap_multi_tool_use_parallel(&[wrapper], &allowed);
        // Keeps wrapper as-is
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["function"]["name"], "multi_tool_use.parallel");
    }

    #[test]
    fn test_unwrap_parallel_rejects_unknown_tools() {
        let wrapper = json!({
            "type": "function",
            "id": "call_unk",
            "function": {
                "name": "multi_tool_use.parallel",
                "arguments": r#"{"tool_uses":[{"recipient_name":"functions.dangerous","parameters":{}}]}"#
            }
        });
        let allowed = make_allowed(&["shell", "cat"]);
        let result = unwrap_multi_tool_use_parallel(&[wrapper], &allowed);
        // No valid inner calls -> wrapper remains
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_unwrap_no_parallel_passthrough() {
        let normal1 =
            json!({"type":"function","id":"c1","function":{"name":"cat","arguments":"{}"}});
        let normal2 =
            json!({"type":"function","id":"c2","function":{"name":"shell","arguments":"{}"}});
        let allowed = make_allowed(&["cat", "shell"]);
        let result = unwrap_multi_tool_use_parallel(&[normal1.clone(), normal2.clone()], &allowed);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["id"], "c1");
        assert_eq!(result[1]["id"], "c2");
    }

    // ========== Real-world test cases from responses.jsonl ==========

    #[test]
    fn test_real_world_shell_leak() {
        let content = "I'll run the new stress test and then type-check to make sure it integrates cleanly.\u{0580}\u{0581}assistant to=functions.shell \u{0574}\u{0565}\u{056F}\u{0576}\u{0561}\u{0562}\u{0561}\u{0576}\u{0578}\u{0582}\u{0569}\u{0575}\u{0578}\u{0582}\u{0576}  \u{5929}\u{5929}\u{4E2D}\u{5F69}\u{7968}\u{4E0A} JSON\n{\"command\":\"npm run test:stress && npm run types\",\"workdir\":\"/home/svakhreev/projects/smc/refact/refact-agent/gui\",\"output_filter\":\"(FAIL|PASS|Test Files|Tests|error TS|Duration|RUN|✓|✗)\",\"output_limit\":\"240\",\"timeout\":\"600\"}";
        let allowed = make_allowed(&["shell", "apply_patch", "cat", "tree"]);
        let result = recover_tool_calls_from_chatml_content(content, &allowed);
        assert!(
            result.is_some(),
            "Failed to recover tool call from real-world garbled content"
        );
        let (clean, calls) = result.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "shell");

        let args: Value =
            serde_json::from_str(calls[0]["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["command"], "npm run test:stress && npm run types");
        assert_eq!(
            args["workdir"],
            "/home/svakhreev/projects/smc/refact/refact-agent/gui"
        );
        assert_eq!(args["timeout"], "600");

        assert!(clean.ends_with("cleanly."));
        assert!(!clean.contains('\u{0580}'));
    }

    #[test]
    fn test_real_world_apply_patch_leak() {
        let content = "One more cleanup. +#+#+#+#+#+assistant to=functions.apply_patch \u{0574}\u{0565}\u{056F}\u{0576}\u{0561}\u{0562}\u{0561}\u{0576}\u{0578}\u{0582}\u{0569}\u{0575}\u{0578}\u{0582}\u{0576} \u{FF3F}\u{4FC3}\u{53BB}\u{4E5F}json code\n{\"patch\": \"*** Begin Patch\\n*** Update File: /src/main.rs\\n@@\\n-old\\n+new\\n\"}";
        let allowed = make_allowed(&["apply_patch", "shell"]);
        let result = recover_tool_calls_from_chatml_content(content, &allowed);
        assert!(result.is_some());
        let (clean, calls) = result.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "apply_patch");
        assert!(clean.ends_with("cleanup."));
    }
}
