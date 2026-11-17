use std::collections::HashMap;

use serde_json::Value;

use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::tools_description::{ToolDesc, ToolSource, ToolSourceType};

pub mod integrations;
pub mod memory;
pub mod refact_engine;
pub mod surface;
pub mod user_prefs;

#[cfg(test)]
mod contract_tests;

fn source(config_path: &str) -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: config_path.to_string(),
    }
}

fn desc(
    config_path: &str,
    name: &str,
    display_name: &str,
    description: &str,
    input_schema: Value,
) -> ToolDesc {
    ToolDesc {
        name: name.to_string(),
        display_name: display_name.to_string(),
        source: source(config_path),
        experimental: false,
        allow_parallel: false,
        description: description.to_string(),
        input_schema,
        output_schema: None,
        annotations: None,
    }
}

fn result(tool_call_id: &String, text: impl Into<String>) -> (bool, Vec<ContextEnum>) {
    (
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text.into()),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })],
    )
}

fn string_arg<'a>(args: &'a HashMap<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("argument `{name}` is missing or not a non-empty string"))
}
