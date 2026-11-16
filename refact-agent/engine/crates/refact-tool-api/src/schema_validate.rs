use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaViolation {
    pub path: String,
    pub message: String,
}

/// Schema checks allowed per validation, on top of a few per value node.
const WORK_BUDGET: usize = 1_000_000;
const WORK_PER_NODE: usize = 8;
pub const TOO_COMPLEX: &str = "schema too complex to validate";

struct Work {
    left: usize,
    exceeded: bool,
}

fn nodes(value: &Value) -> usize {
    1 + match value {
        Value::Array(items) => items.iter().map(nodes).sum(),
        Value::Object(map) => map.values().map(nodes).sum(),
        _ => 0,
    }
}

pub fn validate(schema: &Value, value: &Value) -> Result<(), Vec<SchemaViolation>> {
    let mut errors = Vec::new();
    let mut work = Work {
        left: WORK_BUDGET.saturating_add(nodes(value).saturating_mul(WORK_PER_NODE)),
        exceeded: false,
    };
    check(schema, schema, value, "", 0, &[], &mut work, &mut errors);
    if work.exceeded {
        return Err(vec![SchemaViolation {
            path: String::new(),
            message: TOO_COMPLEX.into(),
        }]);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn violation(errors: &mut Vec<SchemaViolation>, path: &str, message: impl AsRef<str>) {
    let name = if path.is_empty() { "value" } else { path };
    errors.push(SchemaViolation {
        path: path.to_owned(),
        message: format!("'{name}' {}", message.as_ref()),
    });
}

fn property_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn matches_type(kind: &str, value: &Value) -> bool {
    match kind {
        "integer" => value.as_number().is_some_and(|n| {
            n.is_i64() || n.is_u64() || n.as_f64().is_some_and(|v| v.fract() == 0.0)
        }),
        "number" => value.is_number(),
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    }
}

fn json_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            let integer = |n: &serde_json::Number| {
                n.as_i64()
                    .map(i128::from)
                    .or_else(|| n.as_u64().map(i128::from))
            };
            match (integer(a), integer(b)) {
                (Some(a), Some(b)) => a == b,
                (Some(i), None) | (None, Some(i)) => {
                    let f = if integer(a).is_none() { a } else { b }.as_f64().unwrap();
                    f.fract() == 0.0 && f as i128 == i
                }
                (None, None) => a.as_f64() == b.as_f64(),
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| json_equal(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| json_equal(a, b)))
        }
        _ => left == right,
    }
}

#[allow(clippy::too_many_arguments)]
fn check(
    root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
    depth: usize,
    refs: &[&str],
    work: &mut Work,
    errors: &mut Vec<SchemaViolation>,
) {
    if work.exceeded {
        return;
    }
    if work.left == 0 {
        work.exceeded = true;
        return;
    }
    work.left -= 1;
    if depth > 128 {
        violation(errors, path, "exceeds schema recursion limit");
        return;
    }
    if schema == &Value::Bool(false) {
        violation(errors, path, "is forbidden by schema");
        return;
    }
    if !schema.is_object() || (value.is_null() && schema["nullable"] == true) {
        return;
    }
    if let Some(reference) = schema["$ref"].as_str() {
        let resolved =
            if reference.starts_with("#/$defs/") || reference.starts_with("#/definitions/") {
                root.pointer(&reference[1..])
            } else {
                None
            };
        if refs.contains(&reference) {
            violation(
                errors,
                path,
                "exceeds schema recursion limit: a $ref cycle consumes no data",
            );
        } else if let Some(target) = resolved {
            let mut chain = refs.to_vec();
            chain.push(reference);
            check(root, target, value, path, depth + 1, &chain, work, errors);
        } else {
            violation(
                errors,
                path,
                format!("has unresolved schema reference {reference}"),
            );
        }
    }
    if let Some(kind) = schema.get("type") {
        let (matches, expected) = if let Some(kind) = kind.as_str() {
            (matches_type(kind, value), kind.to_owned())
        } else if let Some(kinds) = kind.as_array() {
            let names: Vec<_> = kinds.iter().filter_map(Value::as_str).collect();
            (
                names.iter().any(|kind| matches_type(kind, value)),
                names.join(" or "),
            )
        } else {
            (true, String::new())
        };
        if !matches {
            violation(
                errors,
                path,
                format!("must be {expected}, got {}", value_type(value)),
            );
        }
    }
    if schema
        .get("const")
        .is_some_and(|expected| !json_equal(expected, value))
    {
        violation(errors, path, "does not match const");
    }
    if schema["enum"]
        .as_array()
        .is_some_and(|options| !options.iter().any(|option| json_equal(option, value)))
    {
        violation(errors, path, "must be one of the enum values");
    }
    for keyword in ["anyOf", "oneOf", "allOf"] {
        if let Some(variants) = schema[keyword].as_array() {
            let results: Vec<_> = variants
                .iter()
                .map(|variant| {
                    let mut branch = Vec::new();
                    check(
                        root,
                        variant,
                        value,
                        path,
                        depth + 1,
                        refs,
                        work,
                        &mut branch,
                    );
                    branch
                })
                .collect();
            let matches = results.iter().filter(|branch| branch.is_empty()).count();
            if keyword == "allOf" {
                errors.extend(results.into_iter().flatten());
            } else if matches == 0 || (keyword == "oneOf" && matches != 1) {
                violation(
                    errors,
                    path,
                    format!(
                        "must match {} {keyword} variant (matched {matches})",
                        if keyword == "oneOf" {
                            "exactly one"
                        } else {
                            "at least one"
                        }
                    ),
                );
                if matches == 0 {
                    errors.extend(results.into_iter().flatten());
                }
            }
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema["required"].as_array() {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    violation(errors, &property_path(path, key), "is required");
                }
            }
        }
        let properties = schema["properties"].as_object();
        for (key, value) in object {
            let child_path = property_path(path, key);
            if let Some(child) = properties.and_then(|properties| properties.get(key)) {
                check(
                    root,
                    child,
                    value,
                    &child_path,
                    depth + 1,
                    &[],
                    work,
                    errors,
                );
            } else if let Some(additional) = schema.get("additionalProperties") {
                if additional == &Value::Bool(false) {
                    violation(errors, &child_path, "is not an allowed property");
                } else if additional.is_object() {
                    check(
                        root,
                        additional,
                        value,
                        &child_path,
                        depth + 1,
                        &[],
                        work,
                        errors,
                    );
                }
            }
        }
    }
    if let Some(array) = value.as_array() {
        check_size(schema, "minItems", "maxItems", array.len(), path, errors);
        if schema["uniqueItems"] == true
            && array.iter().enumerate().any(|(index, item)| {
                array[..index]
                    .iter()
                    .any(|previous| json_equal(previous, item))
            })
        {
            violation(errors, path, "must have unique items");
        }
        if let Some(items) = schema.get("items") {
            for (index, value) in array.iter().enumerate() {
                let item_schema = if let Some(tuple) = items.as_array() {
                    tuple.get(index)
                } else {
                    Some(items)
                };
                if let Some(item_schema) = item_schema {
                    check(
                        root,
                        item_schema,
                        value,
                        &format!("{path}[{index}]"),
                        depth + 1,
                        &[],
                        work,
                        errors,
                    );
                }
            }
        }
    }
    if let Some(text) = value.as_str() {
        check_size(
            schema,
            "minLength",
            "maxLength",
            text.chars().count(),
            path,
            errors,
        );
        if let Some(pattern) = schema["pattern"].as_str() {
            match regex::Regex::new(pattern) {
                Ok(regex) if !regex.is_match(text) => {
                    violation(errors, path, format!("must match pattern {pattern}"))
                }
                Err(_) => violation(
                    errors,
                    path,
                    format!("has invalid schema pattern {pattern}"),
                ),
                _ => {}
            }
        }
    }
    if let Some(number) = value.as_f64() {
        for (keyword, upper, exclusive) in [
            ("minimum", false, false),
            ("maximum", true, false),
            ("exclusiveMinimum", false, true),
            ("exclusiveMaximum", true, true),
        ] {
            if let Some(bound) = schema[keyword].as_f64() {
                if (exclusive && number == bound)
                    || if upper {
                        number > bound
                    } else {
                        number < bound
                    }
                {
                    violation(errors, path, format!("violates {keyword} {bound}"));
                }
            }
        }
    }
}

fn check_size(
    schema: &Value,
    min: &str,
    max: &str,
    size: usize,
    path: &str,
    errors: &mut Vec<SchemaViolation>,
) {
    for (keyword, upper) in [(min, false), (max, true)] {
        if let Some(bound) = schema[keyword].as_u64() {
            if if upper {
                size as u64 > bound
            } else {
                (size as u64) < bound
            } {
                violation(errors, path, format!("violates {keyword} {bound}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate;
    use serde_json::json;

    #[test]
    fn enforces_exclusive_numeric_bounds() {
        let minimum = json!({"type":"number","exclusiveMinimum":0});
        assert!(validate(&minimum, &json!(0)).is_err());
        assert!(validate(&minimum, &json!(-0.5)).is_err());
        assert!(validate(&minimum, &json!(0.5)).is_ok());
        let maximum = json!({"type":"number","exclusiveMaximum":1});
        assert!(validate(&maximum, &json!(1)).is_err());
        assert!(validate(&maximum, &json!(1.5)).is_err());
        assert!(validate(&maximum, &json!(0.5)).is_ok());
    }

    #[test]
    fn unique_items_uses_json_semantic_equality() {
        let schema = json!({"type":"array","uniqueItems":true});
        for duplicate in [
            json!(["x", "x"]),
            json!([1, 1.0]),
            json!([{"a":[1]}, {"a":[1.0]}]),
        ] {
            assert!(validate(&schema, &duplicate).is_err(), "{duplicate}");
        }
        assert!(validate(&schema, &json!(["x", "y"])).is_ok());
        assert!(validate(&schema, &json!([1, true, "1"])).is_ok());
        assert!(validate(&json!({"uniqueItems":false}), &json!([1, 1])).is_ok());
    }

    #[test]
    fn const_and_enum_use_recursive_json_semantic_equality() {
        for expected in [json!(1), json!({"a":[1,{"b":2}]})] {
            let equivalent = if expected.is_number() {
                json!(1.0)
            } else {
                json!({"a":[1.0,{"b":2.0}]})
            };
            for schema in [json!({"const":expected}), json!({"enum":[expected]})] {
                assert!(validate(&schema, &equivalent).is_ok(), "{schema}");
                assert!(validate(&schema, &json!(true)).is_err());
                assert!(validate(&schema, &json!({"a":[1,{"b":3}]})).is_err());
            }
        }
        for (integer, rounded) in [
            (json!(9007199254740993_u64), json!(9007199254740992.0)),
            (json!(u64::MAX), json!(18446744073709551616.0)),
        ] {
            assert!(validate(&json!({"const":integer}), &rounded).is_err());
        }
    }

    #[test]
    fn reports_missing_required_and_type_mismatch() {
        let errors = validate(&json!({"type":"object","properties":{"timeout":{"type":"integer"}},"required":["command"]}), &json!({"timeout":"5"})).unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.path == "command" && e.message.contains("required")));
        assert!(errors
            .iter()
            .any(|e| e.message == "'timeout' must be integer, got string"));
    }

    #[test]
    fn resolves_local_defs_refs() {
        for key in ["$defs", "definitions"] {
            let schema = json!({key:{"X":{"type":"integer"}},"$ref":format!("#/{key}/X")});
            assert!(validate(&schema, &json!(7)).is_ok());
            assert!(validate(&schema, &json!("7")).is_err());
        }
    }

    #[test]
    fn any_of_and_one_of_semantics() {
        let variants = json!([{"type":"integer"},{"type":"number"}]);
        assert!(validate(&json!({"anyOf":variants}), &json!(1)).is_ok());
        assert!(validate(&json!({"anyOf":variants}), &json!(false)).is_err());
        assert!(validate(&json!({"oneOf":variants}), &json!(1)).is_err());
        assert!(validate(&json!({"oneOf":variants}), &json!(1.5)).is_ok());
        assert!(validate(&json!({"oneOf":variants}), &json!(false)).is_err());
    }

    #[test]
    fn collects_nested_constraints_and_supports_nullable_types() {
        let schema = json!({"type":"object","properties":{
            "a":{"type":"array","minItems":1,"maxItems":1,"items":{"type":"object","properties":{"c":{"type":"string","minLength":2,"maxLength":3,"pattern":"^x"}},"additionalProperties":{"type":"boolean"}}},
            "n":{"allOf":[{"minimum":2},{"maximum":3}]},
            "optional":{"type":["integer","null"]},
            "legacy":{"type":"string","nullable":true}
        },"additionalProperties":false});
        let errors = validate(
            &schema,
            &json!({"a":[{"c":"q","flag":3},{}],"n":4,"optional":null,"legacy":null,"extra":0}),
        )
        .unwrap_err();
        for path in ["a", "a[0].c", "a[0].flag", "n", "extra"] {
            assert!(errors.iter().any(|e| e.path == path), "{path}: {errors:?}");
        }
        assert!(!errors
            .iter()
            .any(|e| e.path == "optional" || e.path == "legacy"));
        assert!(validate(&json!({"type":"integer"}), &json!(1.0)).is_ok());
        assert!(validate(&json!({"type":"integer"}), &json!(1.5)).is_err());
        assert!(validate(
            &json!({"type":"string","minLength":1,"maxLength":1}),
            &json!("é")
        )
        .is_ok());
        assert!(validate(&json!({"enum":[null,"x"]}), &json!(null)).is_ok());
        assert!(validate(&json!({"enum":[null,"x"]}), &json!(false)).is_err());
    }

    #[test]
    fn bounds_reference_recursion_and_reports_missing_refs() {
        assert!(validate(
            &json!({"$defs":{"X":{"$ref":"#/$defs/X"}},"$ref":"#/$defs/X"}),
            &json!(0)
        )
        .unwrap_err()
        .iter()
        .any(|e| e.message.contains("recursion limit")));
        assert!(validate(&json!({"$ref":"#/$defs/missing"}), &json!(0)).is_err());
        assert!(validate(
            &json!({"$defs":{"a/b":{"type":"string"}},"$ref":"#/$defs/a~1b"}),
            &json!("ok")
        )
        .is_ok());
    }

    #[test]
    fn accepts_existing_registered_input_schema_shapes() {
        let flat = crate::json_schema_from_params(
            &[
                ("command", "string", "Command"),
                ("timeout", "integer", "Seconds"),
            ],
            &["command"],
        );
        assert!(validate(&flat, &json!({"command":"echo ok","timeout":5})).is_ok());
        let variants = json!([
            {"type":"object","properties":{"kind":{"const":"duration"},"duration_ms":{"type":"integer"}},"required":["kind","duration_ms"],"additionalProperties":false},
            {"type":"object","properties":{"kind":{"const":"process"},"process_id":{"type":"string"}},"required":["kind","process_id"],"additionalProperties":false}
        ]);
        let tagged = json!({"type":"object","properties":{"target":{"oneOf":variants}},"required":["target"]});
        assert!(validate(
            &tagged,
            &json!({"target":{"kind":"duration","duration_ms":100}})
        )
        .is_ok());
        assert!(validate(
            &json!({"anyOf":variants}),
            &json!({"kind":"process","process_id":"p"})
        )
        .is_ok());
        assert!(validate(
            &json!({"anyOf":variants}),
            &json!({"kind":"process","process_id":"p","extra":true})
        )
        .is_err());
    }

    #[test]
    fn hostile_recursive_schemas_end_quickly_with_a_violation() {
        let branches: Vec<_> = (0..8).map(|_| json!({"$ref":"#/$defs/X"})).collect();
        let hostile = json!({"$defs":{"X":{"anyOf":branches}},"$ref":"#/$defs/X"});
        let started = std::time::Instant::now();
        let errors = validate(&hostile, &json!({"a":1})).unwrap_err();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("schema too complex to validate")
                    || e.message.contains("recursion limit")),
            "{errors:?}"
        );
        let wide: Vec<_> = (0..64)
            .map(|_| json!({"anyOf":[{"$ref":"#/$defs/Y"},{"$ref":"#/$defs/Y"}]}))
            .collect();
        let fanout =
            json!({"$defs":{"Y":{"type":"array","items":{"anyOf":wide}}},"$ref":"#/$defs/Y"});
        let mut deep = json!(1);
        for _ in 0..12 {
            deep = json!([deep, deep]);
        }
        let started = std::time::Instant::now();
        let errors = validate(&fanout, &deep).unwrap_err();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("schema too complex to validate")),
            "{errors:?}"
        );
    }

    #[test]
    fn ordinary_recursive_tree_schemas_still_validate() {
        let tree = json!({"$defs":{"Node":{"type":"object","required":["name"],"properties":{
            "name":{"type":"string"},
            "children":{"type":"array","items":{"$ref":"#/$defs/Node"}}
        },"additionalProperties":false}},"$ref":"#/$defs/Node"});
        let mut node = json!({"name":"leaf"});
        for depth in 0..20 {
            node = json!({"name":format!("n{depth}"),"children":[node, {"name":"x"}]});
        }
        assert!(validate(&tree, &node).is_ok());
        let bad = json!({"name":"root","children":[{"name":1}]});
        let errors = validate(&tree, &bad).unwrap_err();
        assert!(
            errors.iter().any(|e| e.path == "children[0].name"),
            "{errors:?}"
        );
    }
}
