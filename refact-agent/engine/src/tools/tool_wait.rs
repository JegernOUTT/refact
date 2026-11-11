use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolWait {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolWait {
    fn tool_description(&self) -> ToolDesc {
        let task_schema = super::tool_task_check_agents::agent_status_input_schema();
        let mut task_properties = task_schema["properties"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        task_properties.insert("kind".into(), json!({"const":"task_agents"}));
        let mut push_schema = refact_core::chat_types::PushMode::schema();
        push_schema["description"] = json!("Duration targets only. When tick notices should land: preempt (interrupt the current answer), append (default, at the next safe message boundary), when_idle (after the whole turn finishes).");
        ToolDesc {
            name:"wait".into(), display_name:"Wait".into(),
            source:ToolSource {source_type:ToolSourceType::Builtin,config_path:self.config_path.clone()},
            experimental:false, allow_parallel:true,
            description:"Wait for a duration, process, any one authorized child agent, or park the task planner awaiting matching agents. Duration targets require a nonblank target.description (at most 80 trimmed characters); the legacy top-level description remains accepted for duration only. Omit description and push for non-duration targets. push belongs at the top level, never inside target. Interruption never cancels the target. Duration ticks are enqueued after the wait tool result. Task agent timeouts are optional, at least 30 seconds, in whole-second milliseconds.".into(),
            input_schema:json!({"type":"object","additionalProperties":false,"properties":{
                "target":{"oneOf":[
                    {"type":"object","additionalProperties":false,"properties":{"kind":{"const":"duration"},"duration_ms":{"type":"integer","minimum":100},"tick_interval_ms":{"type":"integer","minimum":5000},"description":{"type":"string","minLength":1,"maxLength":80,"pattern":"\\S","description":"Required nonblank reason for this duration wait, at most 80 trimmed characters."}},"required":["kind","duration_ms","description"]},
                    {"type":"object","additionalProperties":false,"properties":{"kind":{"const":"process"},"process_id":{"type":"string","minLength":1}},"required":["kind","process_id"]},
                    {"type":"object","additionalProperties":false,"properties":{"kind":{"const":"agents"},"agent_ids":{"type":"array","minItems":1,"items":{"type":"string","minLength":1}}},"required":["kind","agent_ids"]},
                    {"type":"object","additionalProperties":false,"properties":task_properties,"required":["kind"]}
                ]},
                "timeout_ms":{"type":"integer","minimum":0,"description":"Optional timeout for non-duration targets only. Task agents require at least 30000 ms in whole seconds."},
                "push":push_schema
            },"required":["target"]}), output_schema:None,annotations:None,
        }
    }
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (kind, mut operation) = parse_wait_request(args)?;
        match kind {
            "duration" => {
                super::tool_sleep::execute_duration_wait(ccx, tool_call_id, &operation).await
            }
            "process" => {
                super::tool_process::execute_process_wait(ccx, tool_call_id, &operation).await
            }
            "agents" => {
                super::tool_background_agents::execute_agents_wait(ccx, tool_call_id, &operation)
                    .await
            }
            "task_agents" => {
                if let Some(value) = operation.remove("timeout_ms") {
                    let ms = value.as_u64().ok_or("timeout_ms must be an integer")?;
                    if ms < 30_000 {
                        return Err("task_agents timeout_ms must be at least 30000".into());
                    }
                    if ms % 1000 != 0 {
                        return Err("task_agents timeout_ms must use whole seconds".into());
                    }
                    operation.insert("wake_up_after_secs".into(), json!(ms / 1000));
                }
                super::tool_task_wait_for_agents::execute_task_agents_wait(
                    ccx,
                    tool_call_id,
                    &operation,
                )
                .await
            }
            _ => unreachable!(),
        }
    }
    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

fn parse_wait_request(
    args: &HashMap<String, Value>,
) -> Result<(&str, HashMap<String, Value>), String> {
    for key in args.keys() {
        if !matches!(
            key.as_str(),
            "target" | "description" | "timeout_ms" | "push"
        ) {
            return Err(format!("Unknown argument '{key}'"));
        }
    }
    let target = args
        .get("target")
        .and_then(Value::as_object)
        .ok_or("target must be an object")?;
    let kind = target
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("target.kind is required")?;
    let fields: &[&str] = match kind {
        "duration" => &["kind", "duration_ms", "tick_interval_ms", "description"],
        "process" => &["kind", "process_id"],
        "agents" => &["kind", "agent_ids"],
        "task_agents" => &[
            "kind",
            "task_id",
            "card_ids",
            "status_filter",
            "format",
            "priority_filter",
            "min_age_minutes",
            "sort",
            "limit",
            "offset",
            "since_seq",
            "group_by",
        ],
        _ => return Err("target.kind must be duration, process, agents, or task_agents".into()),
    };
    for key in target.keys() {
        if !fields.contains(&key.as_str()) {
            if key == "push" && kind == "duration" {
                return Err(format!(
                    "target.push is not supported; place push at the top level for duration only. Example: {}",
                    json!({"target":{"kind":"duration","duration_ms":100,"description":"Wait briefly"},"push":"append"})
                ));
            }
            if key == "push" || key == "description" {
                return Err(format!(
                    "target.{key} is not applicable to {kind}; description and push apply only to duration (push at the top level). Example: {}",
                    minimal_wait_example(kind)
                ));
            }
            return Err(format!("'{key}' is not applicable to {kind}"));
        }
    }
    if kind == "duration" && args.contains_key("timeout_ms") {
        return Err("timeout_ms is not applicable to duration".into());
    }
    if kind != "duration" && (args.contains_key("description") || args.contains_key("push")) {
        return Err(format!(
            "description and push apply only to duration; omit both for {kind}. Example: {}",
            minimal_wait_example(kind)
        ));
    }
    let mut operation: HashMap<String, Value> = target
        .iter()
        .filter(|(k, _)| k.as_str() != "kind")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if kind == "duration" {
        let nested = target.get("description");
        let legacy = args.get("description");
        for value in [nested, legacy].into_iter().flatten() {
            if !value.is_string() {
                return Err("description must be a string".into());
            }
        }
        if let (Some(nested), Some(legacy)) = (nested, legacy) {
            if nested.as_str().map(str::trim) != legacy.as_str().map(str::trim) {
                return Err("Conflicting target.description and top-level description".into());
            }
        }
        let description = nested
            .or(legacy)
            .and_then(Value::as_str)
            .ok_or("Missing required argument 'description'")?
            .trim();
        if description.is_empty() {
            return Err("description must be a non-empty string".into());
        }
        if description.chars().count() > 80 {
            return Err(format!(
                "target.description must be at most 80 trimmed characters; shorten the reason. Example: {}",
                minimal_wait_example("duration")
            ));
        }
        operation.insert("description".into(), json!(description));
    }
    for key in ["push", "timeout_ms"] {
        if let Some(value) = args.get(key) {
            operation.insert(key.into(), value.clone());
        }
    }
    Ok((kind, operation))
}

fn minimal_wait_example(kind: &str) -> Value {
    let target = match kind {
        "duration" => json!({"kind":"duration","duration_ms":100,"description":"Wait briefly"}),
        "process" => json!({"kind":"process","process_id":"exec_..."}),
        "agents" => json!({"kind":"agents","agent_ids":["bgagent-..."]}),
        "task_agents" => json!({"kind":"task_agents"}),
        _ => unreachable!(),
    };
    json!({"target": target})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_example_parses(error: &str) {
        let example = error
            .split("Example: ")
            .nth(1)
            .expect("actionable JSON example");
        let args: HashMap<String, Value> = serde_json::from_str(example).unwrap();
        parse_wait_request(&args).unwrap();
    }

    #[test]
    fn wait_rejects_misplaced_fields_with_valid_examples() {
        for kind in ["process", "agents", "task_agents"] {
            for field in ["description", "push"] {
                for nested in [false, true] {
                    let mut request = minimal_wait_example(kind);
                    let value = if field == "push" {
                        json!("append")
                    } else {
                        json!("Wait briefly")
                    };
                    if nested {
                        request["target"][field] = value;
                    } else {
                        request[field] = value;
                    }
                    let args = serde_json::from_value(request).unwrap();
                    let error = parse_wait_request(&args).unwrap_err();
                    assert!(error.contains("only to duration"));
                    assert_example_parses(&error);
                }
            }
        }
        let mut request = minimal_wait_example("duration");
        request["target"]["push"] = json!("append");
        let error = parse_wait_request(&serde_json::from_value(request).unwrap()).unwrap_err();
        assert!(error.contains("place push at the top level"));
        assert_example_parses(&error);
    }

    #[test]
    fn wait_duration_description_limit_counts_trimmed_characters() {
        for legacy in [false, true] {
            for (length, valid) in [(80, true), (81, false)] {
                let mut request = minimal_wait_example("duration");
                let value = json!(format!(" {} ", "é".repeat(length)));
                if legacy {
                    request["target"]
                        .as_object_mut()
                        .unwrap()
                        .remove("description");
                    request["description"] = value;
                } else {
                    request["target"]["description"] = value;
                }
                let args = serde_json::from_value(request).unwrap();
                let result = parse_wait_request(&args);
                if valid {
                    assert_eq!(result.unwrap().1["description"], "é".repeat(length));
                } else {
                    let error = result.unwrap_err();
                    assert!(error.contains("at most 80 trimmed characters"));
                    assert_example_parses(&error);
                }
            }
        }
    }

    #[test]
    fn parse_wait_request_contract() {
        let schema = ToolWait {
            config_path: String::new(),
        }
        .tool_description()
        .input_schema;
        assert_eq!(schema["required"], json!(["target"]));
        assert!(schema["properties"].get("description").is_none());
        let variants = schema["properties"]["target"]["oneOf"].as_array().unwrap();
        assert_eq!(
            variants[0]["required"],
            json!(["kind", "duration_ms", "description"])
        );
        let description = &variants[0]["properties"]["description"];
        assert_eq!(description["pattern"], "\\S");
        assert_eq!(description["minLength"], 1);
        assert_eq!(description["maxLength"], 80);
        let nonblank = regex::Regex::new(description["pattern"].as_str().unwrap()).unwrap();
        for (value, valid) in [("", false), (" \t\n", false), ("wait", true)] {
            assert_eq!(nonblank.is_match(value), valid);
        }
        for variant in &variants[1..] {
            assert!(variant["properties"].get("description").is_none());
            assert_eq!(variant["additionalProperties"], false);
        }
        for (nested, legacy, expected) in [
            (Some(json!(" wait ")), None, Ok("wait")),
            (None, Some(json!(" wait ")), Ok("wait")),
            (Some(json!(" wait ")), Some(json!("wait")), Ok("wait")),
            (None, None, Err("Missing required argument 'description'")),
            (
                Some(json!("")),
                None,
                Err("description must be a non-empty string"),
            ),
            (
                Some(json!(" \t\n")),
                None,
                Err("description must be a non-empty string"),
            ),
            (
                None,
                Some(json!(" \t\n")),
                Err("description must be a non-empty string"),
            ),
            (Some(json!(7)), None, Err("description must be a string")),
            (
                None,
                Some(json!(false)),
                Err("description must be a string"),
            ),
            (
                Some(json!("wait")),
                Some(json!(null)),
                Err("description must be a string"),
            ),
            (
                Some(json!("wait")),
                Some(json!("other")),
                Err("Conflicting target.description and top-level description"),
            ),
        ] {
            let mut target = json!({"kind":"duration", "duration_ms":100});
            if let Some(nested) = nested {
                target["description"] = nested;
            }
            let mut args = HashMap::from([("target".into(), target)]);
            if let Some(legacy) = legacy {
                args.insert("description".into(), legacy);
            }
            let actual = parse_wait_request(&args).map(|(kind, operation)| {
                assert_eq!(kind, "duration");
                operation["description"].as_str().unwrap().to_string()
            });
            assert_eq!(
                actual.as_ref().map(String::as_str).map_err(String::as_str),
                expected
            );
        }
        for target in [
            json!({"kind":"process", "process_id":"exec_test"}),
            json!({"kind":"agents", "agent_ids":["child"]}),
            json!({"kind":"task_agents"}),
        ] {
            let mut args = HashMap::from([("target".into(), target)]);
            args.insert("timeout_ms".into(), json!(30_000));
            let (_, operation) = parse_wait_request(&args).unwrap();
            assert!(!operation.contains_key("description"));
            assert_eq!(operation["timeout_ms"], 30_000);
            args.insert("push".into(), json!("append"));
            assert!(parse_wait_request(&args).is_err());
            args.remove("push");
            args.insert("description".into(), json!("wait"));
            assert!(parse_wait_request(&args).is_err());
            args.remove("description");
            args.get_mut("target").unwrap()["description"] = json!("wait");
            assert!(parse_wait_request(&args).is_err());
        }
        let mut args = HashMap::from([
            (
                "target".into(),
                json!({"kind":"duration", "duration_ms":100, "tick_interval_ms":5000, "description":"wait"}),
            ),
            ("push".into(), json!("when_idle")),
        ]);
        let (_, operation) = parse_wait_request(&args).unwrap();
        assert_eq!(operation["tick_interval_ms"], 5000);
        assert_eq!(operation["push"], "when_idle");
        args.insert("timeout_ms".into(), json!(100));
        assert!(parse_wait_request(&args).is_err());
    }
}
