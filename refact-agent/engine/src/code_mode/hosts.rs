//! The declared, schema-described set of host operations orchestration code may
//! call. The catalog is a closed constant: an operation that is not listed here
//! does not exist for code mode, and every listed operation names the host tool
//! whose authority it borrows.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostValueType {
    String,
    Integer,
    Boolean,
    StringArray,
}

impl HostValueType {
    pub fn json_type(self) -> &'static str {
        match self {
            HostValueType::String => "string",
            HostValueType::Integer => "integer",
            HostValueType::Boolean => "boolean",
            HostValueType::StringArray => "array",
        }
    }

    fn schema(self) -> Value {
        match self {
            HostValueType::StringArray => json!({"type": "array", "items": {"type": "string"}}),
            other => json!({"type": other.json_type()}),
        }
    }

    fn accepts(self, value: &Value) -> bool {
        match self {
            HostValueType::String => value.is_string(),
            HostValueType::Integer => value.is_u64() || value.is_i64(),
            HostValueType::Boolean => value.is_boolean(),
            HostValueType::StringArray => value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HostArgument {
    pub name: &'static str,
    pub value_type: HostValueType,
    pub required: bool,
}

const fn required(name: &'static str, value_type: HostValueType) -> HostArgument {
    HostArgument {
        name,
        value_type,
        required: true,
    }
}

const fn optional(name: &'static str, value_type: HostValueType) -> HostArgument {
    HostArgument {
        name,
        value_type,
        required: false,
    }
}

/// One callable host operation. `tool` is the host tool the operation performs,
/// and is the name checked against the assignment authority ceiling — code mode
/// borrows existing tool authority instead of minting its own.
#[derive(Debug, Clone, Copy)]
pub struct HostOperation {
    pub name: &'static str,
    pub tool: &'static str,
    pub summary: &'static str,
    pub arguments: &'static [HostArgument],
}

static SPAWN_ARGUMENTS: &[HostArgument] = &[
    required("task", HostValueType::String),
    required("expected_result", HostValueType::String),
    optional("target_files", HostValueType::StringArray),
];

static CLAIM_ARGUMENTS: &[HostArgument] = &[
    required("card_id", HostValueType::String),
    optional("expected_rev", HostValueType::Integer),
];

static WAIT_ARGUMENTS: &[HostArgument] = &[
    required("agent_id", HostValueType::String),
    optional("timeout_ms", HostValueType::Integer),
];

static REPORT_ARGUMENTS: &[HostArgument] = &[required("status_line", HostValueType::String)];

pub static HOST_OPERATIONS: &[HostOperation] = &[
    HostOperation {
        name: "spawn",
        tool: "subagent",
        summary: "Start one background child for a described slice of work.",
        arguments: SPAWN_ARGUMENTS,
    },
    HostOperation {
        name: "claim",
        tool: "board_claim",
        summary: "Claim one board card before working it.",
        arguments: CLAIM_ARGUMENTS,
    },
    HostOperation {
        name: "wait",
        tool: "wait",
        summary: "Wait for one child to reach a terminal state.",
        arguments: WAIT_ARGUMENTS,
    },
    HostOperation {
        name: "report",
        tool: "progress_report",
        summary: "Publish one progress line for the orchestrating run.",
        arguments: REPORT_ARGUMENTS,
    },
];

pub fn operation(name: &str) -> Option<&'static HostOperation> {
    HOST_OPERATIONS.iter().find(|op| op.name == name)
}

pub fn declared_names() -> String {
    HOST_OPERATIONS
        .iter()
        .map(|op| op.name)
        .collect::<Vec<_>>()
        .join(", ")
}

impl HostOperation {
    pub fn schema(&self) -> Value {
        let mut properties = Map::new();
        let mut required_names = Vec::new();
        for argument in self.arguments {
            properties.insert(argument.name.to_string(), argument.value_type.schema());
            if argument.required {
                required_names.push(Value::String(argument.name.to_string()));
            }
        }
        json!({
            "op": self.name,
            "tool": self.tool,
            "description": self.summary,
            "arguments": {
                "type": "object",
                "properties": Value::Object(properties),
                "required": required_names,
                "additionalProperties": false,
            },
        })
    }

    /// Deny by default: an argument the schema does not declare is a refusal,
    /// not an ignored extra.
    pub fn validate(&self, arguments: &Value) -> Result<(), String> {
        let object = arguments
            .as_object()
            .ok_or_else(|| format!("host operation `{}` needs an arguments object", self.name))?;
        for argument in self.arguments {
            match object.get(argument.name) {
                None if argument.required => {
                    return Err(format!(
                        "host operation `{}` requires argument `{}`",
                        self.name, argument.name
                    ));
                }
                None => {}
                Some(value) if argument.value_type.accepts(value) => {}
                Some(_) => {
                    return Err(format!(
                        "host operation `{}` argument `{}` must be {}",
                        self.name,
                        argument.name,
                        argument.value_type.json_type()
                    ));
                }
            }
        }
        if let Some(unknown) = object
            .keys()
            .find(|name| !self.arguments.iter().any(|argument| &argument.name == name))
        {
            return Err(format!(
                "host operation `{}` has no argument `{unknown}`",
                self.name
            ));
        }
        Ok(())
    }
}

/// One line of the host-call program a confined worker emits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostCall {
    pub op: String,
    #[serde(default)]
    pub arguments: Value,
}

impl HostCall {
    pub fn new(op: impl Into<String>, arguments: Value) -> Self {
        Self {
            op: op.into(),
            arguments,
        }
    }

    pub fn parse(line: &str) -> Result<Self, String> {
        serde_json::from_str(line)
            .map_err(|error| format!("worker emitted a line that is not a host call: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_mode_declares_a_closed_catalog_that_names_its_backing_tool() {
        assert_eq!(
            HOST_OPERATIONS
                .iter()
                .map(|op| (op.name, op.tool))
                .collect::<Vec<_>>(),
            vec![
                ("spawn", "subagent"),
                ("claim", "board_claim"),
                ("wait", "wait"),
                ("report", "progress_report"),
            ]
        );
        assert!(operation("shell").is_none());
        assert!(operation("apply").is_none());
        assert!(operation("spawn").is_some());
    }

    #[test]
    fn the_published_schema_forbids_undeclared_arguments() {
        let spawn = operation("spawn").unwrap();
        let schema = spawn.schema();

        assert_eq!(schema["arguments"]["additionalProperties"], json!(false));
        assert_eq!(
            schema["arguments"]["required"],
            json!(["task", "expected_result"])
        );
        assert_eq!(
            schema["arguments"]["properties"]["target_files"],
            json!({"type": "array", "items": {"type": "string"}})
        );
        assert_eq!(schema["tool"], json!("subagent"));
    }

    #[test]
    fn arguments_are_checked_against_the_schema_and_never_widened() {
        let spawn = operation("spawn").unwrap();

        spawn
            .validate(&json!({"task": "t", "expected_result": "r"}))
            .unwrap();
        spawn
            .validate(&json!({"task": "t", "expected_result": "r", "target_files": ["a.rs"]}))
            .unwrap();
        assert!(spawn
            .validate(&json!({"task": "t"}))
            .unwrap_err()
            .contains("requires argument `expected_result`"));
        assert!(spawn
            .validate(&json!({"task": 1, "expected_result": "r"}))
            .unwrap_err()
            .contains("must be string"));
        assert!(spawn
            .validate(&json!({"task": "t", "expected_result": "r", "tools": "all"}))
            .unwrap_err()
            .contains("has no argument `tools`"));
        assert!(spawn
            .validate(&json!({"task": "t", "expected_result": "r", "target_files": [1]}))
            .unwrap_err()
            .contains("must be array"));
        assert!(spawn
            .validate(&json!("spawn"))
            .unwrap_err()
            .contains("needs"));
    }

    #[test]
    fn a_host_call_is_parsed_from_one_line_or_refused() {
        let call = HostCall::parse(r#"{"op":"report","arguments":{"status_line":"hi"}}"#).unwrap();

        assert_eq!(call.op, "report");
        assert_eq!(call.arguments["status_line"], json!("hi"));
        assert_eq!(
            HostCall::parse(r#"{"op":"report"}"#).unwrap().arguments,
            Value::Null
        );
        assert!(HostCall::parse("print('hello')")
            .unwrap_err()
            .contains("not a host call"));
    }
}
