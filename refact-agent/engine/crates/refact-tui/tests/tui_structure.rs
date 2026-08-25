use std::fs;
use std::path::Path;

use regex_lite::Regex;

const APP_RS_PRODUCTION_LINE_CAP: usize = 1_510;
const STATE_RS_PRODUCTION_LINE_CAP: usize = 2_000;
const RATCHET_SCHEDULE: &str = "1,510 app.rs production lines → 2,000 state.rs production lines";
const INLINE_TEST_BOUNDARY: &str = "\n#[cfg(test)]\nmod tests {";
enum NonEngineTuiToolMatch {
    Exact(&'static str),
    Prefix(&'static str),
}

const NON_ENGINE_TUI_TOOL_MATCHES: &[NonEngineTuiToolMatch] = &[
    // Provider-native web search calls are resolved by the provider, not the engine registry.
    NonEngineTuiToolMatch::Exact("web_search_call"),
    // Provider-native file search calls are resolved by the provider, not the engine registry.
    NonEngineTuiToolMatch::Exact("file_search_call"),
    // Provider-native code interpreter calls are resolved by the provider, not the engine registry.
    NonEngineTuiToolMatch::Exact("code_interpreter_call"),
    // Provider-native local shell calls are resolved by the provider, not the engine registry.
    NonEngineTuiToolMatch::Exact("local_shell_call"),
    // Provider-native image generation calls are resolved by the provider, not the engine registry.
    NonEngineTuiToolMatch::Exact("image_generation_call"),
    // Provider-native computer-use calls are resolved by the provider, not the engine registry.
    NonEngineTuiToolMatch::Exact("computer_use_call"),
    // Provider-native web fetch calls are resolved by the provider, not the engine registry.
    NonEngineTuiToolMatch::Exact("web_fetch"),
    // Provider-native code execution calls are resolved by the provider, not the engine registry.
    NonEngineTuiToolMatch::Exact("code_execution"),
    // Server tool-use IDs identify provider-executed calls, not engine tool names.
    NonEngineTuiToolMatch::Prefix("srvtoolu_"),
];

#[test]
fn app_rs_stays_within_the_current_ratchet_cap() {
    assert_production_line_cap("src/app.rs", APP_RS_PRODUCTION_LINE_CAP);
}

#[test]
fn state_rs_stays_within_the_current_ratchet_cap() {
    assert_production_line_cap("src/app/state.rs", STATE_RS_PRODUCTION_LINE_CAP);
}

fn assert_production_line_cap(relative_path: &str, cap: usize) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    let production = source
        .split_once(INLINE_TEST_BOUNDARY)
        .map(|(production, _)| production)
        .unwrap_or(&source);
    let current_line_count = production.lines().count();

    assert!(
        current_line_count <= cap,
        "{relative_path} has {current_line_count} production lines; cap is {cap}. Ratchet schedule: {RATCHET_SCHEDULE}. The cap may only decrease."
    );
}

#[test]
fn tui_tool_literals_are_registered_or_explicitly_exempt() {
    let tui_source = read_tui_source("src/history/cells/tool_family.rs");
    let registered = engine_registered_tool_names();
    let exact_matches = string_literals_from_const(&tui_source, "TUI_TOOL_FAMILY_REGISTRY");
    let provider_native_matches =
        string_literals_from_const(&tui_source, "PROVIDER_NATIVE_TOOL_NAMES");
    let prefix_matches = string_literals_from_const(&tui_source, "PREFIX_TOOL_FAMILIES");
    let exempt_literals = NON_ENGINE_TUI_TOOL_MATCHES
        .iter()
        .filter_map(|entry| match entry {
            NonEngineTuiToolMatch::Exact(name) => Some((*name).to_string()),
            NonEngineTuiToolMatch::Prefix(_) => None,
        })
        .collect();
    let exempt_prefixes = NON_ENGINE_TUI_TOOL_MATCHES
        .iter()
        .filter_map(|entry| match entry {
            NonEngineTuiToolMatch::Exact(_) => None,
            NonEngineTuiToolMatch::Prefix(prefix) => Some(*prefix),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        provider_native_matches, exempt_literals,
        "provider-native TUI tool names must have an explicit engine-registry exemption"
    );
    assert!(
        provider_native_matches
            .iter()
            .all(|name| !registered.contains(name)),
        "provider-native exemption contains an engine-registered tool"
    );

    let missing = exact_matches
        .iter()
        .filter(|name| !registered.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "TUI exact tool matches absent from the engine registry: {missing:?}"
    );

    assert_eq!(exempt_prefixes, ["srvtoolu_"]);
    assert!(prefix_matches.contains("srvtoolu_"));
    assert!(prefix_matches.contains("process_"));
    let process_tools = registered
        .iter()
        .filter(|name| name.starts_with("process_"))
        .collect::<Vec<_>>();
    assert_eq!(
        process_tools.len(),
        7,
        "the TUI process_ matcher must continue to cover the seven engine process tools: {process_tools:?}"
    );
}

fn read_tui_source(relative_path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn engine_registered_tool_names() -> std::collections::HashSet<String> {
    let engine_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("refact-tui crate must have an engine root");
    let registry_source = read_source(&engine_root.join("src/tools/tools_list.rs"));
    let registry_source = registry_source
        .split("#[cfg(test)]")
        .next()
        .expect("engine registry source must have a production section");
    let constructor_re =
        Regex::new(r"Box::new\s*\(\s*crate::tools::(?:[A-Za-z0-9_]+::)+(Tool[A-Za-z0-9_]+)")
            .expect("valid tool constructor regex");
    let registered_types = constructor_re
        .captures_iter(registry_source)
        .map(|captures| captures[1].to_string())
        .collect::<std::collections::HashSet<_>>();
    registered_types
        .iter()
        .filter_map(|tool_type| tool_description_name(&engine_root.join("src/tools"), tool_type))
        .collect()
}

fn tool_description_name(directory: &Path, tool_type: &str) -> Option<String> {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()));
    entries.find_map(|entry| {
        let path = entry
            .unwrap_or_else(|error| panic!("failed to read tools entry: {error}"))
            .path();
        if path.is_dir() {
            tool_description_name(&path, tool_type)
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            let source = read_source(&path);
            tool_description_body(&source, tool_type)
                .and_then(|body| tool_name_from_description(&source, body))
        } else {
            None
        }
    })
}

fn read_source(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn tool_description_body<'a>(source: &'a str, tool_type: &str) -> Option<&'a str> {
    let implementation = source.find(&format!("impl Tool for {tool_type} {{"))?;
    let implementation = braced_body(&source[implementation..])?;
    let description = implementation.find("fn tool_description")?;
    function_body(&implementation[description..])
}

fn tool_name_from_description(source: &str, description: &str) -> Option<String> {
    let name_field = Regex::new(r#"name:\s*"([a-z][a-z0-9_]*)"\.to_string"#)
        .expect("valid tool name field regex");
    if let Some(captures) = name_field.captures(description) {
        return Some(captures[1].to_string());
    }

    for helper in ["tool_desc", "desc", "background_agent_tool_desc"] {
        if let Some(arguments) = function_call_arguments(description, helper) {
            let string_literal =
                Regex::new(r#""([a-z][a-z0-9_]*)""#).expect("valid descriptor name regex");
            if let Some(captures) = string_literal.captures(arguments) {
                return Some(captures[1].to_string());
            }
        }
    }

    let helper = Regex::new(r"([a-z][a-z0-9_]*_description)\s*\(\s*\)")
        .expect("valid descriptor helper regex")
        .captures(description)?[1]
        .to_string();
    let function = source.find(&format!("fn {helper}"))?;
    tool_name_from_description(source, function_body(&source[function..])?)
}

fn braced_body(source: &str) -> Option<&str> {
    let start = source.find('{')?;
    let mut depth = 0usize;
    for (offset, character) in source[start..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[start..=start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

fn function_body(source: &str) -> Option<&str> {
    let brace = source.find('{')?;
    braced_body(&source[brace..])
}

fn function_call_arguments<'a>(source: &'a str, function: &str) -> Option<&'a str> {
    let start = source.find(&format!("{function}("))? + function.len();
    let mut depth = 0usize;
    for (offset, character) in source[start..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[start + 1..start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

fn string_literals_from_const(source: &str, name: &str) -> std::collections::HashSet<String> {
    let declaration = source
        .find(&format!("const {name}"))
        .unwrap_or_else(|| panic!("missing {name} declaration"));
    let block = source[declaration..]
        .split_once("];")
        .unwrap_or_else(|| panic!("missing end of {name} declaration"))
        .0;
    string_literals(block)
}

fn string_literals(source: &str) -> std::collections::HashSet<String> {
    Regex::new(r#"\"([^\"]+)\""#)
        .expect("valid string literal regex")
        .captures_iter(source)
        .map(|captures| captures[1].to_string())
        .collect()
}
