use std::fs;
use std::path::Path;

use regex_lite::Regex;

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

const RETIRED_TUI_TOOL_NAMES: &[&str] = &[
    "sleep",
    "get_plan",
    "set_plan",
    "update_plan",
    "pause_goal",
    "snooze_goal",
    "pause_agent",
    "resume_agent",
    "wait_agents",
    "agent_wait",
    "agent_resume",
];

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

    for name in RETIRED_TUI_TOOL_NAMES {
        assert!(
            exact_matches.contains(*name),
            "obsolete legacy exemption: {name}"
        );
        assert!(
            !registered.contains(*name),
            "legacy tool is registered again: {name}"
        );
    }
    let missing_tools = |names: &std::collections::HashSet<String>| {
        let mut missing = names
            .iter()
            .filter(|name| {
                !registered.contains(*name) && !RETIRED_TUI_TOOL_NAMES.contains(&name.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        missing.sort();
        missing
    };
    let mut negative_control = exact_matches.clone();
    negative_control.insert("bogus_unregistered_tui_tool".into());
    assert_eq!(
        missing_tools(&negative_control),
        ["bogus_unregistered_tui_tool"]
    );

    let missing = missing_tools(&exact_matches);
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
        6,
        "the TUI process_ matcher must continue to cover the six engine process tools: {process_tools:?}"
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
        .split_once(INLINE_TEST_BOUNDARY)
        .map(|(production, _)| production)
        .unwrap_or(&registry_source);
    let constructor_re =
        Regex::new(r"Box::new\s*\(\s*crate::tools::(?:[A-Za-z0-9_]+::)+(Tool[A-Za-z0-9_]+)")
            .expect("valid tool constructor regex");
    let registered_types = constructor_re
        .captures_iter(registry_source)
        .map(|captures| captures[1].to_string())
        .collect::<std::collections::HashSet<_>>();
    let mut registered = registered_types
        .iter()
        .map(|tool_type| {
            tool_description_name(&engine_root.join("src/tools"), tool_type)
                .unwrap_or_else(|| panic!("cannot resolve registered descriptor for {tool_type}"))
        })
        .collect::<std::collections::HashSet<_>>();
    registered.extend(exposed_default_subagent_tool_names(&engine_root));
    registered
}

fn exposed_default_subagent_tool_names(engine_root: &Path) -> std::collections::HashSet<String> {
    let directory = engine_root.join("crates/refact-yaml-configs/src/defaults/subagents");
    let id_re =
        Regex::new(r"(?m)^id:\s*([a-z][a-z0-9_]*)\s*(?:#.*)?$").expect("valid subagent id regex");
    let exposed_re =
        Regex::new(r"(?m)^expose_as_tool:\s*true\s*(?:#.*)?$").expect("valid expose-as-tool regex");
    fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("failed to read subagent entry: {error}"))
                .path()
        })
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("yaml"))
        .filter_map(|path| {
            let source = read_source(&path);
            if !exposed_re.is_match(&source) {
                return None;
            }
            Some(
                id_re
                    .captures(&source)
                    .unwrap_or_else(|| panic!("missing id in exposed subagent {}", path.display()))
                    [1]
                .to_string(),
            )
        })
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
    if !source.contains(tool_type) {
        return None;
    }
    let implementation_re = Regex::new(&format!(
        r"impl\s+(?:[A-Za-z0-9_]+::)*Tool\s+for\s+{tool_type}\s*\{{"
    ))
    .expect("valid Tool implementation regex");
    let implementation = implementation_re.find(source)?.start();
    let implementation = braced_body(&source[implementation..])?;
    let description = implementation.find("fn tool_description")?;
    function_body(&implementation[description..])
}

fn tool_name_from_description(source: &str, description: &str) -> Option<String> {
    let name_field =
        Regex::new(r#"(?:name:\s*|\.name\s*=\s*)"([a-z][a-z0-9_]*)"\.(?:to_string|into)"#)
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

    if description
        .trim_start_matches('{')
        .trim_start()
        .starts_with("description(")
    {
        let arguments = function_call_arguments(description, "description")?;
        let helper = source.find("fn description(")?;
        let body = function_body(&source[helper..])?;
        let conditional = Regex::new(
            r#"name:\s*if\s+resume\s*\{\s*"([a-z_]+)"\s*\}\s*else\s*\{\s*"([a-z_]+)"\s*\}"#,
        )
        .expect("valid conditional descriptor regex");
        let names = conditional.captures(body)?;
        return match arguments.rsplit(',').next()?.trim() {
            "true" => Some(names[1].to_string()),
            "false" => Some(names[2].to_string()),
            _ => None,
        };
    }

    let helper = Regex::new(r"([a-z][a-z0-9_]*_description)\s*\(")
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
