use super::memory::ToolBuddyMemoryCreate;
use super::surface::{ToolBuddyRuntimeEvent, ToolBuddySpeak};
use crate::tools::tools_description::Tool;
use refact_yaml_configs::project_configs_bootstrap::embedded_defaults;
use serde_json::Value;

fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    let schema = schema.as_object().ok_or("schema is not an object")?;
    for key in schema.keys() {
        if ![
            "type",
            "properties",
            "required",
            "additionalProperties",
            "enum",
            "items",
            "maxLength",
            "minimum",
            "maximum",
            "default",
            "description",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("unsupported schema keyword {key}"));
        }
    }
    if let Some(kind) = schema.get("type").and_then(Value::as_str) {
        let valid = match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => return Err(format!("unsupported type {kind}")),
        };
        if !valid {
            return Err(format!("expected {kind}"));
        }
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if !values.contains(value) {
            return Err(format!("invalid enum value {value}"));
        }
    }
    if let Some(text) = value.as_str() {
        if schema
            .get("maxLength")
            .and_then(Value::as_u64)
            .is_some_and(|max| text.chars().count() as u64 > max)
        {
            return Err("string too long".into());
        }
    }
    if let Some(number) = value.as_f64() {
        if schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|min| number < min)
            || schema
                .get("maximum")
                .and_then(Value::as_f64)
                .is_some_and(|max| number > max)
        {
            return Err("number out of range".into());
        }
    }
    if let Some(array) = value.as_array() {
        if let Some(items) = schema.get("items") {
            for item in array {
                validate(items, item)?;
            }
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for field in required {
                let field = field.as_str().ok_or("invalid required field")?;
                if !object.contains_key(field) {
                    return Err(format!("missing {field}"));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, value) in object {
            if let Some(property) = properties.and_then(|p| p.get(key)) {
                validate(property, value)?;
            } else if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                return Err(format!("unknown property {key}"));
            }
        }
    }
    Ok(())
}

#[test]
fn autonomous_examples_match_live_tool_schemas() {
    let schemas = [
        ToolBuddyRuntimeEvent {
            config_path: String::new(),
        }
        .tool_description(),
        ToolBuddySpeak {
            config_path: String::new(),
        }
        .tool_description(),
        ToolBuddyMemoryCreate {
            config_path: String::new(),
        }
        .tool_description(),
    ];
    let defaults = embedded_defaults("subagents");
    assert!(!defaults.is_empty(), "embedded subagents must be present");
    let mut validated_examples = 0;
    let mut validated_autonomous = 0;
    for (filename, content) in defaults {
        let config: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        if config["subchat"]["autonomous_no_confirm"].as_bool() != Some(true) {
            continue;
        }
        let tools = config["tools"].as_sequence().expect("autonomous tools");
        if !schemas.iter().any(|desc| {
            tools
                .iter()
                .any(|tool| tool.as_str() == Some(desc.name.as_str()))
        }) {
            continue;
        }
        let prompt = config["messages"]["system_prompt"]
            .as_str()
            .expect("system prompt");
        assert!(
            prompt.contains("nonempty final assistant assessment"),
            "{filename}: autonomous completion must produce a saved assessment"
        );
        validated_autonomous += 1;
        for desc in &schemas {
            let prefix = format!("{}(", desc.name);
            let examples: Vec<_> = prompt
                .lines()
                .filter_map(|line| line.trim().strip_prefix(&prefix))
                .collect();
            let available = tools
                .iter()
                .any(|tool| tool.as_str() == Some(desc.name.as_str()));
            if !available {
                assert!(
                    examples.is_empty(),
                    "{filename}: example calls unavailable tool {}",
                    desc.name
                );
                continue;
            }
            assert!(
                !examples.is_empty(),
                "{filename}: missing {} example",
                desc.name
            );
            for example in examples {
                let json = example.strip_suffix(')').expect("example must end in )");
                let args: Value = serde_json::from_str(json).expect("example must contain JSON");
                validate(&desc.input_schema, &args)
                    .unwrap_or_else(|err| panic!("{filename}: {}: {err}", desc.name));
                validated_examples += 1;
            }
        }
    }
    assert!(
        validated_autonomous > 0,
        "autonomous callers must be checked"
    );
    assert!(validated_examples > 0, "tool examples must be validated");
}

#[test]
fn validator_rejects_the_old_missing_status_example() {
    let schema = ToolBuddyRuntimeEvent {
        config_path: String::new(),
    }
    .tool_description()
    .input_schema;
    assert!(validate(
        &schema,
        &serde_json::json!({"title":"Daily digest", "description":"Summary"})
    )
    .is_err());
}
