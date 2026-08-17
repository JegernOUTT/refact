use glob::Pattern;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::command_classify::{
    executable_basename, extract_command_segments, segment_command, CommandSegments,
};

pub fn command_should_be_confirmed_by_user(
    command: &String,
    commands_need_confirmation_rules: &Vec<String>,
) -> (bool, String) {
    if let Some(rule) = commands_need_confirmation_rules.iter().find(|glob| {
        let pattern = match Pattern::new(glob) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Invalid glob pattern '{}': {}", glob, e);
                return false;
            }
        };
        pattern.matches(&command)
    }) {
        return (true, rule.clone());
    }
    (false, "".to_string())
}

pub fn command_should_be_confirmed_by_user_segment_aware(
    command: &String,
    commands_need_confirmation_rules: &Vec<String>,
) -> (bool, String) {
    command_matches_rules_segment_aware(command, commands_need_confirmation_rules)
}

pub fn command_should_be_denied(
    command: &String,
    commands_deny_rules: &Vec<String>,
) -> (bool, String) {
    if let Some(rule) = commands_deny_rules.iter().find(|glob| {
        let pattern = match Pattern::new(glob) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Invalid glob pattern '{}': {}", glob, e);
                return false;
            }
        };
        pattern.matches(&command)
    }) {
        return (true, rule.clone());
    }
    (false, "".to_string())
}

pub fn command_should_be_denied_segment_aware(
    command: &String,
    commands_deny_rules: &Vec<String>,
) -> (bool, String) {
    command_matches_rules_segment_aware(command, commands_deny_rules)
}

fn command_matches_rules_segment_aware(command: &String, rules: &Vec<String>) -> (bool, String) {
    let segments = extract_command_segments(command);
    if let Some(rule) = rules.iter().find(|glob| {
        let pattern = match Pattern::new(glob) {
            Ok(pattern) => pattern,
            Err(error) => {
                tracing::warn!("Invalid glob pattern '{}': {}", glob, error);
                return false;
            }
        };
        !segments.parse_ok || command_matches_pattern(command, &segments, &pattern)
    }) {
        return (true, rule.clone());
    }
    (false, String::new())
}

fn command_matches_pattern(command: &str, segments: &CommandSegments, pattern: &Pattern) -> bool {
    pattern.matches(command)
        || segments.segments.iter().any(|segment| {
            pattern.matches(&segment_command(segment))
                || executable_basename(segment).is_some_and(|name| pattern.matches(name))
        })
}

#[derive(Clone, Debug, PartialEq)]
pub enum MatchConfirmDenyResult {
    PASS,
    CONFIRMATION,
    DENY,
}

#[derive(Clone, Debug)]
pub struct MatchConfirmDeny {
    pub result: MatchConfirmDenyResult,
    pub command: String,
    pub rule: String,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ToolGroupCategory {
    Builtin,
    Integration,
    MCP,
    ConfigSubagent,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ToolSourceType {
    Builtin,
    Integration,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ToolSource {
    pub source_type: ToolSourceType,
    pub config_path: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ToolDesc {
    pub name: String,
    #[serde(default)]
    pub experimental: bool,
    #[serde(default)]
    pub allow_parallel: bool,
    pub description: String,
    /// Full JSON Schema for tool input parameters.
    /// Must be `{"type": "object", "properties": {...}, "required": [...]}`.
    /// For tools with no parameters, use `{"type": "object", "properties": {}}`.
    pub input_schema: serde_json::Value,
    /// Optional JSON Schema for structured output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    /// MCP-style tool annotations (readOnlyHint, destructiveHint, idempotentHint, openWorldHint, title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<serde_json::Value>,
    pub display_name: String,
    pub source: ToolSource,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
pub struct ToolConfig {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_parallel: Option<bool>,
}

impl Default for ToolConfig {
    fn default() -> Self {
        ToolConfig {
            enabled: true,
            allow_parallel: None,
        }
    }
}

/// Helper to build a simple input schema from flat parameter definitions.
/// Useful for builtin tools that have simple string/boolean/integer params.
pub fn json_schema_from_params(params: &[(&str, &str, &str)], required: &[&str]) -> Value {
    let mut properties = serde_json::Map::new();
    for (name, param_type, description) in params {
        properties.insert(
            name.to_string(),
            json!({
                "type": param_type,
                "description": description
            }),
        );
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required
    })
}

pub fn is_strict_compatible(schema: &Value) -> bool {
    let Some(obj) = schema.as_object() else {
        return true;
    };
    if obj.get("type") != Some(&json!("object")) {
        return true;
    }
    if obj.get("additionalProperties") == Some(&json!(true)) {
        return false;
    }
    let Some(props) = obj.get("properties").and_then(|p| p.as_object()) else {
        return false;
    };
    if props.is_empty() {
        return true;
    }
    let required_set: std::collections::HashSet<&str> = obj
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    for (key, val) in props {
        if !required_set.contains(key.as_str()) {
            return false;
        }
        if val.get("type") == Some(&json!("object")) && !is_strict_compatible(val) {
            return false;
        }
        if let Some(items) = val.get("items") {
            if items.get("type") == Some(&json!("object")) && !is_strict_compatible(items) {
                return false;
            }
        }
    }
    true
}

fn apply_strict_schema(schema: Value) -> Value {
    let Value::Object(mut map) = schema else {
        return schema;
    };
    if map.get("type") == Some(&json!("object")) {
        if !map.contains_key("additionalProperties") {
            map.insert("additionalProperties".to_string(), json!(false));
        }
        if let Some(Value::Object(props)) = map.remove("properties") {
            let new_props: serde_json::Map<String, Value> = props
                .into_iter()
                .map(|(k, v)| {
                    let new_v = if v.get("type") == Some(&json!("object")) {
                        apply_strict_schema(v)
                    } else if v.get("type") == Some(&json!("array")) {
                        let Value::Object(mut arr_map) = v else {
                            unreachable!()
                        };
                        if let Some(items) = arr_map.remove("items") {
                            arr_map.insert("items".to_string(), apply_strict_schema(items));
                        }
                        Value::Object(arr_map)
                    } else {
                        v
                    };
                    (k, new_v)
                })
                .collect();
            map.insert("properties".to_string(), Value::Object(new_props));
        }
    }
    Value::Object(map)
}

pub fn make_openai_tool_value(
    name: String,
    description: String,
    input_schema: Value,
    strict: bool,
) -> Value {
    let mut parameters_schema = input_schema;
    let effective_strict = strict && is_strict_compatible(&parameters_schema);
    if effective_strict {
        parameters_schema = apply_strict_schema(parameters_schema);
    }
    let mut function_obj = json!({
        "name": name,
        "description": description,
        "parameters": parameters_schema
    });
    if effective_strict {
        function_obj["strict"] = json!(true);
    }
    json!({
        "type": "function",
        "function": function_obj
    })
}

impl ToolDesc {
    pub fn into_openai_style(self, strict: bool) -> Value {
        make_openai_tool_value(self.name, self.description, self.input_schema, strict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_json_schema_from_params_basic() {
        let schema = json_schema_from_params(
            &[
                ("path", "string", "File path"),
                ("content", "string", "File content"),
            ],
            &["path"],
        );
        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["properties"]["path"]["type"], json!("string"));
        assert_eq!(
            schema["properties"]["path"]["description"],
            json!("File path")
        );
        assert_eq!(schema["properties"]["content"]["type"], json!("string"));
        assert_eq!(schema["required"], json!(["path"]));
    }

    #[test]
    fn test_json_schema_from_params_no_params() {
        let schema = json_schema_from_params(&[], &[]);
        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["properties"], json!({}));
        assert_eq!(schema["required"], json!([]));
    }

    #[test]
    fn test_make_openai_tool_value_not_strict() {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"}
            },
            "required": ["query"]
        });
        let result = make_openai_tool_value(
            "search".to_string(),
            "Search the web".to_string(),
            schema,
            false,
        );
        assert_eq!(result["type"], json!("function"));
        assert_eq!(result["function"]["name"], json!("search"));
        assert_eq!(result["function"]["description"], json!("Search the web"));
        assert_eq!(result["function"]["parameters"]["type"], json!("object"));
        assert!(result["function"]["strict"].is_null());
        assert!(result["function"]["parameters"]["additionalProperties"].is_null());
    }

    #[test]
    fn test_make_openai_tool_value_strict() {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"}
            },
            "required": ["query"]
        });
        let result = make_openai_tool_value(
            "search".to_string(),
            "Search the web".to_string(),
            schema,
            true,
        );
        assert_eq!(result["function"]["strict"], json!(true));
        assert_eq!(
            result["function"]["parameters"]["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn test_make_openai_tool_value_strict_preserves_existing_additional_properties() {
        let schema = json!({
            "type": "object",
            "properties": {},
            "additionalProperties": true
        });
        let result = make_openai_tool_value("tool".to_string(), "A tool".to_string(), schema, true);
        assert_eq!(
            result["function"]["parameters"]["additionalProperties"],
            json!(true)
        );
    }

    #[test]
    fn test_make_openai_tool_value_complex_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of items"
                },
                "config": {
                    "type": "object",
                    "properties": {
                        "verbose": {"type": "boolean"}
                    }
                },
                "mode": {
                    "type": "string",
                    "enum": ["fast", "slow"]
                }
            },
            "required": ["items"]
        });
        let result = make_openai_tool_value(
            "process".to_string(),
            "Process items".to_string(),
            schema,
            false,
        );
        assert_eq!(
            result["function"]["parameters"]["properties"]["items"]["type"],
            json!("array")
        );
        assert_eq!(
            result["function"]["parameters"]["properties"]["mode"]["enum"],
            json!(["fast", "slow"])
        );
    }

    #[test]
    fn test_invalid_glob_does_not_panic() {
        let (confirmed, _) = command_should_be_confirmed_by_user(
            &"some command".to_string(),
            &vec!["[invalid".to_string()],
        );
        assert!(!confirmed);

        let (denied, _) =
            command_should_be_denied(&"some command".to_string(), &vec!["[invalid".to_string()]);
        assert!(!denied);
    }

    #[test]
    fn test_segment_aware_matching_catches_shell_evasions() {
        let rules = vec!["sudo*".to_string()];
        for command in [
            "bash -c 'sudo rm -rf /'",
            "echo hi && sudo id",
            "true; sudo id",
            "$(sudo id)",
            "`sudo id`",
            "sh -c \"bash -c 'sudo id'\"",
        ] {
            let (matched, rule) =
                command_should_be_denied_segment_aware(&command.to_string(), &rules);
            assert!(matched, "{command:?}");
            assert_eq!(rule, "sudo*");
        }
    }

    #[test]
    fn test_segment_aware_matching_uses_executable_basename_only() {
        let exact_rules = vec!["sudo".to_string()];
        assert!(
            command_should_be_denied_segment_aware(&"/usr/bin/sudo id".to_string(), &exact_rules).0
        );
        for command in ["echo sudo", "cat sudoku.txt"] {
            assert!(!command_should_be_denied_segment_aware(&command.to_string(), &exact_rules).0);
        }
        let prefix_rules = vec!["sudo*".to_string()];
        for command in ["echo sudo", "cat sudoku.txt"] {
            assert!(!command_should_be_denied_segment_aware(&command.to_string(), &prefix_rules).0);
        }
    }

    #[test]
    fn test_segment_parse_failure_fails_closed_when_rules_exist() {
        let command = "echo 'unterminated sudo".to_string();
        assert!(command_should_be_denied_segment_aware(&command, &vec!["*sudo".to_string()]).0);
        assert!(command_should_be_denied_segment_aware(&command, &vec!["sudo*".to_string()]).0);
    }

    #[test]
    fn test_segment_aware_matching_catches_forwarding_wrappers() {
        let rules = vec!["sudo*".to_string()];
        for command in [
            "command sudo id",
            "builtin sudo id",
            "exec sudo id",
            "env X=1 sudo id",
            "nohup sudo id",
            "nice -n 5 sudo id",
            "ionice -c 3 sudo id",
            "time sudo id",
            "timeout 5 sudo id",
            "stdbuf -o L sudo id",
            "setsid sudo id",
            "xargs sudo id",
            "sudo -- sudo id",
            "doas -n sudo id",
            "find . -exec sudo id \\;",
            "eval 'sudo id'",
            "sh <<< 'sudo id'",
            "sh <<'EOF'\nsudo id\nEOF",
        ] {
            assert!(
                command_should_be_denied_segment_aware(&command.to_string(), &rules).0,
                "{command:?}"
            );
        }
    }

    #[test]
    fn test_common_safe_commands_do_not_match_sudo_rule() {
        let rules = vec!["sudo*".to_string()];
        for command in ["ls -la", "cargo test", "git status", "npm run build"] {
            assert!(
                !command_should_be_denied_segment_aware(&command.to_string(), &rules).0,
                "{command:?}"
            );
            let extracted = extract_command_segments(command);
            assert!(extracted.parse_ok, "{command:?}");
            assert!(
                crate::command_classify::structural_flags(&extracted).is_empty(),
                "{command:?}"
            );
        }
    }

    #[test]
    fn test_all_reported_bypasses_are_denied_or_structurally_confirmed() {
        let rules = vec!["sudo*".to_string()];
        let mut deeply_nested = "sudo id".to_string();
        for _ in 0..=4 {
            deeply_nested = shell_words::join(["sh", "-c", deeply_nested.as_str()]);
        }
        for command in [
            "command sudo id",
            "exec sudo id",
            "env X=1 sudo id",
            "X=sudo; $X id",
            "eval \"sudo id\"",
            "printf 'sudo id\\n' | sh",
            "sh <<< 'sudo id'",
            "find . -exec sudo id \\;",
            "xargs sudo id",
            "printf 'c3VkbyBpZAo=' | base64 -d | sh",
            deeply_nested.as_str(),
            "cmd /C \"sudo id\"",
            "powershell -Command \"bash -c 'sudo id'\"",
        ] {
            let denied = command_should_be_denied_segment_aware(&command.to_string(), &rules).0;
            let structurally_confirmed =
                !crate::command_classify::structural_flags(&extract_command_segments(command))
                    .is_empty();
            assert!(denied || structurally_confirmed, "{command:?}");
        }
    }

    #[test]
    fn test_into_openai_style_roundtrip() {
        let input_schema = json!({
            "type": "object",
            "properties": {
                "filename": {"type": "string", "description": "The filename"}
            },
            "required": ["filename"]
        });
        let desc = ToolDesc {
            name: "cat".to_string(),
            experimental: false,
            allow_parallel: true,
            description: "Read a file".to_string(),
            input_schema: input_schema.clone(),
            output_schema: None,
            annotations: None,
            display_name: "Cat".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: "".to_string(),
            },
        };
        let result = desc.into_openai_style(false);
        assert_eq!(result["function"]["name"], json!("cat"));
        assert_eq!(
            result["function"]["parameters"]["properties"]["filename"]["type"],
            json!("string")
        );
    }

    #[test]
    fn test_is_strict_compatible_all_required() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        });
        assert!(is_strict_compatible(&schema));
    }

    #[test]
    fn test_is_strict_compatible_optional_param() {
        let schema = json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "timeout": {"type": "string"}
            },
            "required": ["command"]
        });
        assert!(!is_strict_compatible(&schema));
    }

    #[test]
    fn test_is_strict_compatible_no_params() {
        let schema = json!({"type": "object", "properties": {}, "required": []});
        assert!(is_strict_compatible(&schema));
    }

    #[test]
    fn test_is_strict_compatible_unstructured_nested_object() {
        let schema = json!({
            "type": "object",
            "properties": {
                "url": {"type": "string"},
                "options": {"type": "object"}
            },
            "required": ["url", "options"]
        });
        assert!(!is_strict_compatible(&schema));
    }

    #[test]
    fn test_is_strict_compatible_nested_array_of_objects_all_required() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "status": {"type": "string"}
                        },
                        "required": ["id", "status"]
                    }
                }
            },
            "required": ["tasks"]
        });
        assert!(is_strict_compatible(&schema));
    }

    #[test]
    fn test_is_strict_compatible_nested_array_of_objects_optional_field() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "options": {"type": "string"}
                        },
                        "required": ["id"]
                    }
                }
            },
            "required": ["tasks"]
        });
        assert!(!is_strict_compatible(&schema));
    }

    #[test]
    fn test_apply_strict_schema_top_level() {
        let schema = json!({
            "type": "object",
            "properties": {"x": {"type": "string"}},
            "required": ["x"]
        });
        let result = apply_strict_schema(schema);
        assert_eq!(result["additionalProperties"], json!(false));
    }

    #[test]
    fn test_apply_strict_schema_recursive_nested_object() {
        let schema = json!({
            "type": "object",
            "properties": {
                "config": {
                    "type": "object",
                    "properties": {"verbose": {"type": "boolean"}},
                    "required": ["verbose"]
                }
            },
            "required": ["config"]
        });
        let result = apply_strict_schema(schema);
        assert_eq!(result["additionalProperties"], json!(false));
        assert_eq!(
            result["properties"]["config"]["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn test_apply_strict_schema_recursive_array_items() {
        let schema = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"id": {"type": "string"}},
                        "required": ["id"]
                    }
                }
            },
            "required": ["items"]
        });
        let result = apply_strict_schema(schema);
        assert_eq!(result["additionalProperties"], json!(false));
        assert_eq!(
            result["properties"]["items"]["items"]["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn test_make_openai_tool_value_strict_skipped_for_optional_params() {
        let schema = json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "timeout": {"type": "string"}
            },
            "required": ["command"]
        });
        let result = make_openai_tool_value("shell".to_string(), "Run".to_string(), schema, true);
        assert!(result["function"]["strict"].is_null());
        assert!(result["function"]["parameters"]["additionalProperties"].is_null());
    }

    #[test]
    fn test_make_openai_tool_value_strict_applied_recursively() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string"},
                            "status": {"type": "string"}
                        },
                        "required": ["id", "status"]
                    }
                }
            },
            "required": ["tasks"]
        });
        let result = make_openai_tool_value(
            "set_tasks".to_string(),
            "Set tasks".to_string(),
            schema,
            true,
        );
        assert_eq!(result["function"]["strict"], json!(true));
        assert_eq!(
            result["function"]["parameters"]["additionalProperties"],
            json!(false)
        );
        assert_eq!(
            result["function"]["parameters"]["properties"]["tasks"]["items"]
                ["additionalProperties"],
            json!(false)
        );
    }
}
