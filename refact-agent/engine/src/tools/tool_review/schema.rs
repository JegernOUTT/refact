use serde_json::{json, Value};
use super::ReviewToolKind;

#[cfg(test)]
#[test]
fn integer_bounds_match_runtime_fields() {
    assert!(validate(&line(), &json!(u32::MAX)).is_ok());
    assert!(validate(&line(), &json!(u64::from(u32::MAX) + 1)).is_err());
    assert!(validate(&line(), &json!(0)).is_err());
    assert!(validate(&timestamp(), &json!(i64::MIN)).is_ok());
    assert!(validate(&timestamp(), &json!(i64::MAX)).is_ok());
    assert!(validate(&timestamp(), &json!(u64::MAX)).is_err());
    assert!(validate(&n(), &json!(u64::MAX)).is_ok());
    assert!(validate(&n(), &json!(-1)).is_err());
}

fn s() -> Value {
    json!({"type":"string"})
}
fn n() -> Value {
    json!({"type":"integer","minimum":0,"maximum":u64::MAX})
}
fn line() -> Value {
    json!({"type":"integer","minimum":1,"maximum":u32::MAX})
}
fn timestamp() -> Value {
    json!({"type":"integer","minimum":i64::MIN,"maximum":i64::MAX})
}
fn boolean() -> Value {
    json!({"type":"boolean"})
}
fn nullable(v: Value) -> Value {
    json!({"anyOf":[v,{"type":"null"}]})
}
fn en(values: &[&str]) -> Value {
    json!({"type":"string","enum":values})
}
fn arr(v: Value) -> Value {
    json!({"type":"array","items":v})
}
fn obj(fields: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let properties: serde_json::Map<String, Value> =
        fields.into_iter().map(|(k, v)| (k.into(), v)).collect();
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn position() -> Value {
    obj(
        vec![
            ("candidate_id", s()),
            ("path", s()),
            ("side", en(&["base", "head"])),
            ("start_line", line()),
            ("end_line", line()),
            ("content_hash", s()),
            ("context_before", arr(s())),
            ("context_after", arr(s())),
        ],
        &[
            "candidate_id",
            "path",
            "side",
            "start_line",
            "end_line",
            "content_hash",
            "context_before",
            "context_after",
        ],
    )
}
fn anchor() -> Value {
    obj(
        vec![
            ("position", position()),
            ("status", en(&["current", "stalled"])),
            ("stalled_reason", nullable(s())),
            ("history", arr(position())),
        ],
        &["position", "status", "history"],
    )
}
fn blocks() -> Value {
    let variants = vec![
        obj(
            vec![("kind", en(&["markdown"])), ("body", s())],
            &["kind", "body"],
        ),
        obj(
            vec![
                ("kind", en(&["html"])),
                ("html", s()),
                ("static_fallback", nullable(s())),
                ("anchors", arr(anchor())),
            ],
            &["kind", "html"],
        ),
        obj(
            vec![("kind", en(&["code"])), ("anchor", anchor())],
            &["kind", "anchor"],
        ),
        obj(
            vec![
                ("kind", en(&["diff"])),
                ("candidate_id", s()),
                ("path", s()),
                ("anchors", arr(anchor())),
            ],
            &["kind", "candidate_id", "path"],
        ),
        obj(
            vec![
                ("kind", en(&["sources"])),
                ("evidence_ids", arr(s())),
                ("relation", en(&["consulted", "supports"])),
            ],
            &["kind", "evidence_ids", "relation"],
        ),
        obj(
            vec![("kind", en(&["evidence"])), ("evidence_ids", arr(s()))],
            &["kind", "evidence_ids"],
        ),
    ];
    arr(obj(
        vec![
            ("id", s()),
            ("revision", n()),
            ("content", json!({"oneOf":variants})),
        ],
        &["id", "revision", "content"],
    ))
}
fn discussion_target() -> Value {
    obj(
        vec![
            ("candidate_id", s()),
            ("step_id", s()),
            ("version", n()),
            ("block_id", nullable(s())),
            ("block_revision", nullable(n())),
        ],
        &["candidate_id", "step_id", "version"],
    )
}
fn version() -> Value {
    obj(
        vec![
            ("version", n()),
            ("title", s()),
            ("body", s()),
            ("html", nullable(s())),
            ("anchors", arr(anchor())),
            ("blocks", blocks()),
            ("candidate_id", nullable(s())),
        ],
        &["version", "title", "body", "anchors"],
    )
}
fn applicability() -> Value {
    en(&[
        "current",
        "needs_revalidation",
        "historical",
        "no_longer_applicable",
        "unknown",
    ])
}
fn contribution() -> Value {
    let actor = obj(
        vec![("kind", en(&["user", "agent", "system"])), ("id", s())],
        &["kind", "id"],
    );
    let record = obj(
        vec![
            ("path", s()),
            ("zone", s()),
            ("attribution", en(&["declared", "observed", "heuristic"])),
        ],
        &["path", "zone", "attribution"],
    );
    let context = obj(
        vec![
            ("candidate_id", nullable(s())),
            ("run_id", nullable(s())),
            ("applicability", applicability()),
            ("privacy", arr(record)),
        ],
        &["applicability", "privacy"],
    );
    obj(
        vec![
            ("id", s()),
            ("thread_id", s()),
            ("actor", actor),
            ("body", s()),
            ("created_at_ms", timestamp()),
            ("chat_message_id", nullable(s())),
            ("blocks", blocks()),
            ("context", nullable(context)),
        ],
        &["id", "thread_id", "actor", "body", "created_at_ms"],
    )
}
fn operation() -> Value {
    let step = obj(
        vec![
            ("id", s()),
            ("current", version()),
            ("history", arr(version())),
        ],
        &["id", "current", "history"],
    );
    let annotation = obj(
        vec![
            ("id", s()),
            ("kind", en(&["explanation", "highlight", "finding"])),
            ("title", s()),
            ("body", s()),
            ("anchors", arr(anchor())),
            (
                "severity",
                en(&["info", "low", "medium", "high", "critical"]),
            ),
            ("confidence", en(&["unknown", "low", "medium", "high"])),
            (
                "assessment",
                en(&["unassessed", "supported", "disputed", "refuted"]),
            ),
            (
                "disposition",
                en(&[
                    "open",
                    "repair_proposed",
                    "fixed",
                    "risk_accepted",
                    "dismissed",
                ]),
            ),
            ("applicability", applicability()),
            ("evidence_ids", arr(s())),
            ("run_id", nullable(s())),
        ],
        &[
            "id",
            "kind",
            "title",
            "body",
            "anchors",
            "severity",
            "confidence",
            "assessment",
            "disposition",
            "applicability",
            "evidence_ids",
        ],
    );
    let thread = obj(
        vec![
            ("id", s()),
            ("anchors", arr(anchor())),
            ("annotation_id", nullable(s())),
            ("resolved", boolean()),
            ("contributions", arr(contribution())),
            ("target", nullable(discussion_target())),
        ],
        &["id", "anchors", "resolved", "contributions"],
    );
    let source = json!({"oneOf":[obj(vec![("kind",en(&["source"])),("path",s()),("anchor",nullable(anchor()))], &["kind","path"]),obj(vec![("kind",en(&["external"])),("url",s())], &["kind","url"])]});
    let evidence = obj(
        vec![
            ("id", s()),
            ("candidate_id", s()),
            (
                "material_id",
                json!({"type":"null","description":"Runtime-owned provenance. Omit or set null for source/external evidence; execution material is recorded only by review_record_evidence."}),
            ),
            (
                "run_id",
                json!({"type":"null","description":"Runtime-owned attribution. Omit or set null; the server assigns the review run."}),
            ),
            ("source", source),
            ("summary", s()),
            ("applicability", applicability()),
            ("created_at_ms", timestamp()),
        ],
        &[
            "id",
            "candidate_id",
            "source",
            "summary",
            "applicability",
            "created_at_ms",
        ],
    );
    let mut alternatives = Vec::new();
    for (op, key, value) in [
        ("rename", "title", s()),
        ("set_target_reference", "target_reference", s()),
        ("put_tour_step", "step", step),
        ("put_annotation", "annotation", annotation),
        ("put_thread", "thread", thread),
        ("add_evidence", "evidence", evidence),
    ] {
        alternatives.push(obj(vec![("op", en(&[op])), (key, value)], &["op", key]));
    }
    json!({"oneOf":alternatives,"description":"Use exact objects from review_get for updates; preserve all history and contributions. put_thread updates existing threads only. Obtain anchors from review_anchor. Executed evidence is server-owned.","examples":[{"op":"rename","title":"Review title"},{"op":"put_tour_step","step":{"id":"orientation","current":{"version":1,"title":"Orientation","body":"Explanation","anchors":[]},"history":[]}}]})
}
fn discovery() -> Value {
    let source = obj(
        vec![
            ("kind", en(&["provided_manifest", "graphite_cli"])),
            ("reference", nullable(s())),
        ],
        &["kind"],
    );
    let pr = obj(
        vec![("host", s()), ("repository", s()), ("number", n())],
        &["host", "repository", "number"],
    );
    let member = obj(
        vec![
            ("key", s()),
            ("branch_ref", s()),
            ("pr_identity", nullable(pr)),
            ("parent", nullable(s())),
        ],
        &["key", "branch_ref"],
    );
    obj(
        vec![
            ("title", s()),
            ("source", source),
            ("selected_member", s()),
            ("members", arr(member)),
            (
                "comparison_mode",
                en(&["live_chain", "recorded_comparisons"]),
            ),
        ],
        &[
            "title",
            "source",
            "selected_member",
            "members",
            "comparison_mode",
        ],
    )
}
fn acquisition() -> Value {
    let comparison = obj(
        vec![("member", s()), ("base_ref", s()), ("head_ref", s())],
        &["member", "base_ref", "head_ref"],
    );
    obj(
        vec![
            ("preparation_token", s()),
            ("scope_receipt_tool_call_id", s()),
            ("request_key", s()),
            ("root_base_ref", nullable(s())),
            ("comparisons", arr(comparison)),
        ],
        &[
            "preparation_token",
            "scope_receipt_tool_call_id",
            "request_key",
            "comparisons",
        ],
    )
}
pub fn advertised(kind: ReviewToolKind, base: Value) -> Value {
    let mut schema = schema(kind, base);
    if matches!(kind, ReviewToolKind::Context) {
        schema.as_object_mut().unwrap().remove("oneOf");
        schema["additionalProperties"] = json!(false);
        schema["properties"]["action"] = en(&[
            "stack_prepare",
            "stack_import",
            "stack_inspect",
            "stack_member",
            "stack_capture",
            "stack_cumulative",
            "stack_refresh",
        ]);
        schema["description"] = json!("Without action provide repository, base_ref, candidate_ref and request_key. Gather context with ordinary tools. Stack actions preserve explicit confirmed scope. Fields from other actions are rejected.");
    }
    schema
}

pub fn schema(kind: ReviewToolKind, base: Value) -> Value {
    if matches!(kind, ReviewToolKind::Context) {
        let policy = obj(
            vec![
                ("staged", boolean()),
                ("working", boolean()),
                ("untracked", boolean()),
            ],
            &["staged", "working", "untracked"],
        );
        let mut variants = vec![obj(
            vec![
                ("repository", s()),
                ("base_ref", s()),
                ("candidate_ref", s()),
                ("policy", policy),
                ("target_reference", s()),
                ("title", s()),
                ("review_id", nullable(s())),
                ("expected_revision", nullable(n())),
                ("request_key", s()),
            ],
            &["repository", "base_ref", "candidate_ref", "request_key"],
        )];
        for (action, field, shape, required) in [
            ("stack_prepare", "discovery", discovery(), true),
            ("stack_import", "acquisition", acquisition(), true),
            ("stack_inspect", "", s(), false),
            ("stack_member", "member_id", s(), true),
            ("stack_capture", "member_id", s(), true),
            ("stack_cumulative", "member_id", s(), true),
            ("stack_refresh", "expected_revision", n(), true),
        ] {
            let mut fields = vec![("action", en(&[action]))];
            let mut req = vec!["action"];
            if !field.is_empty() {
                fields.push((field, shape));
                if required {
                    req.push(field);
                }
            }
            if action == "stack_capture" || action == "stack_refresh" {
                fields.push(("request_key", s()));
                req.push("request_key");
            }
            variants.push(obj(fields, &req));
        }
        let mut properties = serde_json::Map::new();
        for variant in &variants {
            if let Some(fields) = variant["properties"].as_object() {
                for (key, value) in fields {
                    properties
                        .entry(key.clone())
                        .or_insert_with(|| value.clone());
                }
            }
        }
        properties.insert(
            "action".into(),
            en(&[
                "stack_prepare",
                "stack_import",
                "stack_inspect",
                "stack_member",
                "stack_capture",
                "stack_cumulative",
                "stack_refresh",
            ]),
        );
        return json!({"type":"object","properties":properties,"oneOf":variants});
    }
    let mut result = base;
    if matches!(kind, ReviewToolKind::Update) {
        result["properties"]["operation"] = operation();
    }
    if matches!(kind, ReviewToolKind::Reply) {
        result["properties"]["blocks"] = blocks();
    }
    if matches!(kind, ReviewToolKind::Get | ReviewToolKind::Anchor) {
        result["properties"]["start_line"] = line();
        result["properties"]["end_line"] = line();
    }
    let optional: &[&str] = match kind {
        ReviewToolKind::Get => &[
            "review_id",
            "candidate_id",
            "object_id",
            "object_kind",
            "path",
            "side",
            "start_line",
            "end_line",
        ],
        ReviewToolKind::Anchor => &["review_id", "candidate_id"],
        ReviewToolKind::Update | ReviewToolKind::Reply => &["review_id"],
        ReviewToolKind::RecordEvidence => &["review_id", "prepared_material_id"],
        _ => &[],
    };
    for key in optional {
        let old = result["properties"][*key].clone();
        result["properties"][*key] = nullable(old);
    }
    result
}

// Deliberately bounded and value-free: errors must never echo potentially secret inputs.
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    fn walk(schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
        if errors.len() >= 32 {
            return;
        }
        if let Some(options) = schema.get("anyOf").and_then(Value::as_array) {
            if value.is_null() && options.iter().any(|x| x["type"] == "null") {
                return;
            }
            if let Some(first) = options.first() {
                walk(first, value, path, errors);
            }
            return;
        }
        if let Some(options) = schema.get("oneOf").and_then(Value::as_array) {
            if !value.is_object() {
                errors.push(format!("{path}: expected object"));
                return;
            }
            let key = if options.iter().any(|x| x["properties"].get("op").is_some()) {
                "op"
            } else if options
                .iter()
                .any(|x| x["properties"].get("kind").is_some())
            {
                "kind"
            } else {
                "action"
            };
            let discriminator = value.get(key);
            let selected = options.iter().find(|x| match discriminator {
                Some(d) => x["properties"][key]["enum"]
                    .as_array()
                    .map_or(false, |a| a.contains(d)),
                None => x["properties"].get(key).is_none(),
            });
            if let Some(x) = selected {
                walk(x, value, path, errors)
            } else {
                let allowed = options
                    .iter()
                    .filter_map(|x| x["properties"][key]["enum"].as_array())
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                errors.push(format!("{path}.{key}: required; expected one of {allowed}"));
            }
            return;
        }
        let ty = schema.get("type").and_then(Value::as_str).unwrap_or("");
        let valid = match ty {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "null" => value.is_null(),
            _ => true,
        };
        if !valid {
            errors.push(format!("{path}: expected {ty}"));
            return;
        }
        if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            if !values.contains(value) {
                errors.push(format!(
                    "{path}: expected one of {}",
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        if schema
            .get("minimum")
            .and_then(Value::as_i64)
            .is_some_and(|m| value.as_i64().is_some_and(|v| v < m))
        {
            errors.push(format!("{path}: below minimum"));
        }
        if schema
            .get("maximum")
            .is_some_and(|maximum| match (maximum.as_i64(), value.as_i64()) {
                (Some(maximum), Some(value)) => value > maximum,
                _ => maximum
                    .as_u64()
                    .zip(value.as_u64())
                    .is_some_and(|(maximum, value)| value > maximum),
            })
        {
            errors.push(format!("{path}: above maximum"));
        }
        if let Some(map) = value.as_object() {
            let props = &schema["properties"];
            if let Some(req) = schema.get("required").and_then(Value::as_array) {
                for key in req.iter().filter_map(Value::as_str) {
                    if !map.contains_key(key) && errors.len() < 32 {
                        errors.push(format!("{path}.{key}: required"));
                    }
                }
            }
            for (key, v) in map {
                if errors.len() >= 32 {
                    break;
                }
                if let Some(sub) = props.get(key) {
                    walk(sub, v, &format!("{path}.{key}"), errors)
                } else if schema["additionalProperties"] == false {
                    errors.push(format!("{path}: unknown field"));
                }
            }
        }
        if let (Some(items), Some(values)) = (schema.get("items"), value.as_array()) {
            for (i, v) in values.iter().enumerate() {
                if errors.len() >= 32 {
                    break;
                }
                walk(items, v, &format!("{path}[{i}]"), errors);
            }
        }
    }
    let mut errors = vec![];
    walk(schema, value, "$", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        if errors.len() >= 32 {
            errors.push("Further errors omitted".into());
        }
        Err(format!("Invalid review arguments: {}", errors.join("; ")))
    }
}
