use serde_json::{Map, Value};
use std::collections::HashMap;

const MAX_COERCE_DEPTH: usize = 12;

pub fn coerce_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "t" | "yes" | "y" | "on" | "1" => Some(true),
            "false" | "f" | "no" | "n" | "off" | "0" => Some(false),
            _ => None,
        },
        Value::Number(n) => match n.as_f64() {
            Some(f) if f == 1.0 => Some(true),
            Some(f) if f == 0.0 => Some(false),
            _ => None,
        },
        _ => None,
    }
}

pub fn coerce_integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)),
        Value::String(s) => {
            let s = s.trim();
            s.parse::<i64>().ok().or_else(|| {
                s.parse::<f64>()
                    .ok()
                    .filter(|f| f.fract() == 0.0)
                    .map(|f| f as i64)
            })
        }
        _ => None,
    }
}

pub fn coerce_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    }
}

pub fn coerce_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub fn coerce_array(value: &Value) -> Option<Vec<Value>> {
    match value {
        Value::Array(a) => Some(a.clone()),
        Value::String(s) => {
            let trimmed = s.trim();
            if !trimmed.starts_with('[') {
                return None;
            }
            match serde_json::from_str::<Value>(trimmed) {
                Ok(Value::Array(a)) => Some(a),
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn coerce_object(value: &Value) -> Option<Map<String, Value>> {
    match value {
        Value::Object(o) => Some(o.clone()),
        Value::String(s) => {
            let trimmed = s.trim();
            if !trimmed.starts_with('{') {
                return None;
            }
            match serde_json::from_str::<Value>(trimmed) {
                Ok(Value::Object(o)) => Some(o),
                _ => None,
            }
        }
        _ => None,
    }
}

fn value_matches_type(value: &Value, ty: &str) -> bool {
    match ty {
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "integer" => match value {
            Value::Number(n) => {
                n.is_i64() || n.is_u64() || n.as_f64().is_some_and(|f| f.fract() == 0.0)
            }
            _ => false,
        },
        _ => true,
    }
}

fn declared_types(schema: &Value) -> Vec<String> {
    match schema.get("type") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

fn alternative_schemas(schema: &Value) -> Vec<&Value> {
    ["anyOf", "oneOf", "allOf"]
        .iter()
        .filter_map(|key| schema.get(*key))
        .filter_map(|v| v.as_array())
        .flatten()
        .collect()
}

fn coerce_to_type(value: &Value, ty: &str, schema: &Value) -> Option<Value> {
    match ty {
        "boolean" => coerce_bool(value).map(Value::Bool),
        "integer" => coerce_integer(value).map(|i| Value::Number(i.into())),
        "number" => coerce_number(value)
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number),
        "string" => coerce_string(value).map(Value::String),
        "array" => coerce_array(value)
            .or_else(|| wrap_scalar_in_array(value, schema))
            .map(Value::Array),
        "object" => coerce_object(value).map(Value::Object),
        _ => None,
    }
}

fn wrap_scalar_in_array(value: &Value, schema: &Value) -> Option<Vec<Value>> {
    if value.is_null() || value.is_array() || value.is_object() {
        return None;
    }
    let item_schema = schema.get("items");
    let item_types = item_schema.map(declared_types).unwrap_or_default();
    if item_types.is_empty() {
        return Some(vec![value.clone()]);
    }
    let item_schema = item_schema.unwrap_or(&Value::Null);
    for ty in &item_types {
        if value_matches_type(value, ty) {
            return Some(vec![value.clone()]);
        }
        if let Some(coerced) = coerce_to_type(value, ty, item_schema) {
            return Some(vec![coerced]);
        }
    }
    None
}

fn coerce_value(
    value: &mut Value,
    schema: &Value,
    path: &str,
    notes: &mut Vec<String>,
    depth: usize,
) {
    if depth > MAX_COERCE_DEPTH || value.is_null() {
        return;
    }

    let types = declared_types(schema);
    let matched = types.is_empty() || types.iter().any(|ty| value_matches_type(value, ty));

    if !matched {
        let before = json_type_name(value);
        for ty in &types {
            if ty == "null" {
                continue;
            }
            if let Some(coerced) = coerce_to_type(value, ty, schema) {
                notes.push(format!("{path}: {before} -> {ty}"));
                *value = coerced;
                break;
            }
        }
    }

    if types.is_empty() {
        for alt in alternative_schemas(schema) {
            let alt_types = declared_types(alt);
            if alt_types.is_empty() || alt_types.iter().any(|ty| value_matches_type(value, ty)) {
                coerce_value(value, alt, path, notes, depth + 1);
                return;
            }
        }
        for alt in alternative_schemas(schema) {
            let before = json_type_name(value);
            for ty in declared_types(alt) {
                if ty == "null" {
                    continue;
                }
                if let Some(coerced) = coerce_to_type(value, &ty, alt) {
                    notes.push(format!("{path}: {before} -> {ty}"));
                    *value = coerced;
                    return;
                }
            }
        }
        return;
    }

    match value {
        Value::Object(map) => {
            if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                for (key, child_schema) in props {
                    if let Some(child) = map.get_mut(key) {
                        coerce_value(
                            child,
                            child_schema,
                            &format!("{path}.{key}"),
                            notes,
                            depth + 1,
                        );
                    }
                }
            }
            if let Some(extra) = schema.get("additionalProperties").filter(|v| v.is_object()) {
                let declared: Vec<String> = schema
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .map(|p| p.keys().cloned().collect())
                    .unwrap_or_default();
                for (key, child) in map.iter_mut() {
                    if !declared.contains(key) {
                        coerce_value(child, extra, &format!("{path}.{key}"), notes, depth + 1);
                    }
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema.get("items").filter(|v| v.is_object()) {
                for (i, child) in items.iter_mut().enumerate() {
                    coerce_value(
                        child,
                        item_schema,
                        &format!("{path}[{i}]"),
                        notes,
                        depth + 1,
                    );
                }
            }
        }
        _ => {}
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) => {
            if n.is_f64() {
                "number"
            } else {
                "integer"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub fn coerce_args_to_schema(args: &mut Map<String, Value>, input_schema: &Value) -> Vec<String> {
    let mut notes = Vec::new();
    let Some(props) = input_schema.get("properties").and_then(|p| p.as_object()) else {
        return notes;
    };
    for (key, child_schema) in props {
        if let Some(value) = args.get_mut(key) {
            coerce_value(value, child_schema, key, &mut notes, 0);
        }
    }
    notes
}

pub fn coerce_hashmap_to_schema(
    args: &mut HashMap<String, Value>,
    input_schema: &Value,
) -> Vec<String> {
    let mut notes = Vec::new();
    let Some(props) = input_schema.get("properties").and_then(|p| p.as_object()) else {
        return notes;
    };
    for (key, child_schema) in props {
        if let Some(value) = args.get_mut(key) {
            coerce_value(value, child_schema, key, &mut notes, 0);
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(props: Value) -> Value {
        json!({"type": "object", "properties": props})
    }

    #[test]
    fn bool_from_common_model_spellings() {
        for raw in [
            json!("true"),
            json!("True"),
            json!("TRUE"),
            json!("1"),
            json!(1),
            json!("yes"),
            json!("on"),
            json!(" true "),
        ] {
            assert_eq!(coerce_bool(&raw), Some(true), "failed for {raw}");
        }
        for raw in [
            json!("false"),
            json!("False"),
            json!("0"),
            json!(0),
            json!("no"),
            json!("off"),
        ] {
            assert_eq!(coerce_bool(&raw), Some(false), "failed for {raw}");
        }
        assert_eq!(coerce_bool(&json!("maybe")), None);
        assert_eq!(coerce_bool(&json!(2)), None);
        assert_eq!(coerce_bool(&json!(null)), None);
    }

    #[test]
    fn schema_coercion_rewrites_declared_booleans() {
        let s = schema(json!({"wait": {"type": "boolean"}, "dry_run": {"type": "boolean"}}));
        let mut args: Map<String, Value> = json!({"wait": "True", "dry_run": 1})
            .as_object()
            .unwrap()
            .clone();
        let notes = coerce_args_to_schema(&mut args, &s);
        assert_eq!(args["wait"], json!(true));
        assert_eq!(args["dry_run"], json!(true));
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn string_params_are_never_touched() {
        let s = schema(json!({"query": {"type": "string"}}));
        let mut args: Map<String, Value> = json!({"query": "1"}).as_object().unwrap().clone();
        let notes = coerce_args_to_schema(&mut args, &s);
        assert_eq!(args["query"], json!("1"));
        assert!(notes.is_empty());
    }

    #[test]
    fn uncoercible_values_are_left_alone() {
        let s = schema(json!({"wait": {"type": "boolean"}}));
        let mut args: Map<String, Value> = json!({"wait": "maybe"}).as_object().unwrap().clone();
        let notes = coerce_args_to_schema(&mut args, &s);
        assert_eq!(args["wait"], json!("maybe"));
        assert!(notes.is_empty());
    }

    #[test]
    fn numbers_and_strings_swap_by_declared_type() {
        let s = schema(json!({
            "max_steps": {"type": "integer"},
            "temperature": {"type": "number"},
            "label": {"type": "string"}
        }));
        let mut args: Map<String, Value> =
            json!({"max_steps": "50", "temperature": "0.7", "label": 42})
                .as_object()
                .unwrap()
                .clone();
        coerce_args_to_schema(&mut args, &s);
        assert_eq!(args["max_steps"], json!(50));
        assert_eq!(args["temperature"], json!(0.7));
        assert_eq!(args["label"], json!("42"));
    }

    #[test]
    fn json_encoded_containers_are_parsed() {
        let s = schema(json!({
            "paths": {"type": "array", "items": {"type": "string"}},
            "options": {"type": "object"}
        }));
        let mut args: Map<String, Value> =
            json!({"paths": "[\"a.rs\", \"b.rs\"]", "options": "{\"deep\": true}"})
                .as_object()
                .unwrap()
                .clone();
        coerce_args_to_schema(&mut args, &s);
        assert_eq!(args["paths"], json!(["a.rs", "b.rs"]));
        assert_eq!(args["options"], json!({"deep": true}));
    }

    #[test]
    fn lone_scalar_is_wrapped_for_array_params() {
        let s = schema(json!({"paths": {"type": "array", "items": {"type": "string"}}}));
        let mut args: Map<String, Value> = json!({"paths": "a.rs"}).as_object().unwrap().clone();
        coerce_args_to_schema(&mut args, &s);
        assert_eq!(args["paths"], json!(["a.rs"]));
    }

    #[test]
    fn nested_object_and_array_booleans_are_coerced() {
        let s = schema(json!({
            "request": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {"verify": {"type": "boolean"}, "timeout_ms": {"type": "integer"}}
                        }
                    }
                }
            }
        }));
        let mut args: Map<String, Value> =
            json!({"request": {"steps": [{"verify": "false", "timeout_ms": "3000"}]}})
                .as_object()
                .unwrap()
                .clone();
        coerce_args_to_schema(&mut args, &s);
        assert_eq!(args["request"]["steps"][0]["verify"], json!(false));
        assert_eq!(args["request"]["steps"][0]["timeout_ms"], json!(3000));
    }

    #[test]
    fn nullable_and_anyof_schemas_are_supported() {
        let s = schema(json!({
            "flag": {"type": ["boolean", "null"]},
            "count": {"anyOf": [{"type": "integer"}, {"type": "null"}]}
        }));
        let mut args: Map<String, Value> = json!({"flag": "yes", "count": "7"})
            .as_object()
            .unwrap()
            .clone();
        coerce_args_to_schema(&mut args, &s);
        assert_eq!(args["flag"], json!(true));
        assert_eq!(args["count"], json!(7));
    }

    #[test]
    fn null_values_and_unknown_params_are_preserved() {
        let s = schema(json!({"flag": {"type": "boolean"}}));
        let mut args: Map<String, Value> = json!({"flag": null, "extra": "untouched"})
            .as_object()
            .unwrap()
            .clone();
        coerce_args_to_schema(&mut args, &s);
        assert_eq!(args["flag"], json!(null));
        assert_eq!(args["extra"], json!("untouched"));
    }

    #[test]
    fn missing_schema_is_a_no_op() {
        let mut args: Map<String, Value> = json!({"flag": "true"}).as_object().unwrap().clone();
        let notes = coerce_args_to_schema(&mut args, &json!({}));
        assert_eq!(args["flag"], json!("true"));
        assert!(notes.is_empty());
    }

    #[test]
    fn hashmap_variant_matches_map_variant() {
        let s = schema(json!({"tty": {"type": "boolean"}}));
        let mut args: HashMap<String, Value> = HashMap::new();
        args.insert("tty".to_string(), json!("True"));
        let notes = coerce_hashmap_to_schema(&mut args, &s);
        assert_eq!(args["tty"], json!(true));
        assert_eq!(notes.len(), 1);
    }
}
