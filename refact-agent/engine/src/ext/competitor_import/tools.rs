use std::collections::HashSet;

use super::types::ToolPolicy;

pub const READ_ONLY_TOOLS: &[&str] = &["tree", "cat", "glob", "search_pattern"];

const ALL_CANONICAL_TOOLS: &[&str] = &[
    "tree",
    "cat",
    "glob",
    "search_pattern",
    "shell",
    "apply_patch",
    "tasks_set",
    "web",
    "web_search",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolMappingResult {
    pub tools: Vec<String>,
    pub unknown: Vec<String>,
    pub denied: Vec<String>,
    pub used_default: bool,
}

pub fn map_known_tool_alias(input: &str) -> Option<&'static str> {
    match normalized_tool_key(input).as_str() {
        "cat" | "read" | "read_file" => Some("cat"),
        "tree" | "list" | "ls" | "list_files" => Some("tree"),
        "glob" => Some("glob"),
        "search_pattern" | "grep" | "search" | "search_files" => Some("search_pattern"),
        "shell" | "bash" | "execute_command" => Some("shell"),
        "apply_patch" | "write" | "edit" | "multiedit" | "patch" => Some("apply_patch"),
        "tasks_set" | "todowrite" | "todo_write" => Some("tasks_set"),
        "web" | "webfetch" => Some("web"),
        "web_search" | "websearch" => Some("web_search"),
        _ => None,
    }
}

pub fn map_allowed_tools(input: &[String]) -> ToolMappingResult {
    let mut result = ToolMappingResult::default();
    let mut seen = HashSet::new();
    for raw in input {
        if is_broad_tool_alias(raw) {
            push_read_only_tools(&mut result.tools, &mut seen);
            continue;
        }
        match map_known_tool_alias(raw) {
            Some(mapped) => push_tool(&mut result.tools, &mut seen, mapped),
            None => push_unknown(&mut result.unknown, raw),
        }
    }
    result
}

pub fn resolve_subagent_tools(policy: &ToolPolicy) -> ToolMappingResult {
    let mut result = match &policy.allowed {
        Some(allowed) if !allowed.is_empty() => map_allowed_tools(allowed),
        _ => read_only_default(),
    };

    if result.tools.is_empty() {
        let unknown = result.unknown;
        result = read_only_default();
        result.unknown = unknown;
    }

    let mut denied_seen = HashSet::new();
    for raw in &policy.denied {
        if is_broad_tool_alias(raw) {
            for tool in ALL_CANONICAL_TOOLS {
                push_tool(&mut result.denied, &mut denied_seen, tool);
            }
            continue;
        }
        match map_known_tool_alias(raw) {
            Some(mapped) => push_tool(&mut result.denied, &mut denied_seen, mapped),
            None => push_unknown(&mut result.unknown, raw),
        }
    }

    if !result.denied.is_empty() {
        let denied = result.denied.iter().cloned().collect::<HashSet<_>>();
        result.tools.retain(|tool| !denied.contains(tool));
    }

    result
}

fn read_only_default() -> ToolMappingResult {
    ToolMappingResult {
        tools: READ_ONLY_TOOLS
            .iter()
            .map(|tool| (*tool).to_string())
            .collect(),
        unknown: Vec::new(),
        denied: Vec::new(),
        used_default: true,
    }
}

fn push_read_only_tools(out: &mut Vec<String>, seen: &mut HashSet<String>) {
    for tool in READ_ONLY_TOOLS {
        push_tool(out, seen, tool);
    }
}

fn push_tool(out: &mut Vec<String>, seen: &mut HashSet<String>, tool: &str) {
    if seen.insert(tool.to_string()) {
        out.push(tool.to_string());
    }
}

fn push_unknown(out: &mut Vec<String>, raw: &str) {
    let trimmed = raw.trim();
    if !trimmed.is_empty() && !out.iter().any(|item| item == trimmed) {
        out.push(trimmed.to_string());
    }
}

fn is_broad_tool_alias(input: &str) -> bool {
    matches!(
        normalized_tool_key(input).as_str(),
        "*" | "all" | "all_tools" | "all-tools"
    )
}

fn normalized_tool_key(input: &str) -> String {
    let trimmed = input
        .trim()
        .trim_start_matches('!')
        .trim_start_matches('-')
        .trim();
    let before_args = trimmed.split(['(', ':']).next().unwrap_or(trimmed).trim();
    before_args.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }
}
