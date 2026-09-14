use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DELIVERABLE_CHECK_STRUCTURAL: &str = "structural";
const MAX_SCHEMA_DEPTH: usize = 16;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverableContract {
    #[serde(default)]
    pub required_sections: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub evidence_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverableStatus {
    pub check: String,
    pub met: bool,
    #[serde(default)]
    pub missing_sections: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_error: Option<String>,
    #[serde(default)]
    pub evidence_missing: bool,
}

impl DeliverableContract {
    pub fn is_empty(&self) -> bool {
        self.required_sections.is_empty() && self.output_schema.is_none() && !self.evidence_required
    }

    pub fn validate(&self, max_sections: usize, max_schema_bytes: usize) -> Result<(), String> {
        if self.required_sections.len() > max_sections {
            return Err(format!(
                "deliverable.required_sections exceeds {max_sections} entries"
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for section in &self.required_sections {
            let trimmed = section.trim();
            if trimmed.is_empty() || trimmed.chars().count() > 64 || trimmed.contains('\n') {
                return Err(
                    "deliverable.required_sections entries must be 1-64 single-line characters"
                        .into(),
                );
            }
            if !seen.insert(trimmed.to_ascii_lowercase()) {
                return Err(format!("deliverable.required_sections repeats `{trimmed}`"));
            }
        }
        if let Some(schema) = &self.output_schema {
            if !schema.is_object() {
                return Err("deliverable.output_schema must be a JSON schema object".into());
            }
            if serde_json::to_vec(schema)
                .map(|bytes| bytes.len())
                .unwrap_or(usize::MAX)
                > max_schema_bytes
            {
                return Err(format!(
                    "deliverable.output_schema exceeds {max_schema_bytes} bytes"
                ));
            }
            check_schema_depth(schema, 0)?;
        }
        Ok(())
    }

    pub fn render_instructions(&self) -> String {
        let mut lines = vec!["# Deliverable contract".to_string()];
        if !self.required_sections.is_empty() {
            lines.push("Your final message MUST contain each of these Markdown section headings, in this order or any order, each followed by substantive content:".into());
            for section in &self.required_sections {
                lines.push(format!("- `# {}`", section.trim()));
            }
        }
        if let Some(schema) = &self.output_schema {
            lines.push("Your final message MUST include exactly one ```json fenced block that validates against this schema (unknown keywords are ignored; type/required/properties/items/enum/bounds are enforced):".into());
            lines.push(format!(
                "```json\n{}\n```",
                serde_json::to_string_pretty(schema).unwrap_or_default()
            ));
        }
        if self.evidence_required {
            lines.push("Your final message MUST contain a `# Evidence` section citing the exact commands, tool results, or files that back each claim. Unbacked claims are reported as unverified, not as success.".into());
        }
        lines.push("A missing section, invalid JSON block, or missing evidence marks the deliverable contract unmet; the parent sees this alongside your text.".into());
        lines.join("\n")
    }

    pub fn evaluate(&self, final_text: &str) -> DeliverableStatus {
        let missing_sections = self
            .required_sections
            .iter()
            .filter(|section| section_body(final_text, section).is_none())
            .cloned()
            .collect::<Vec<_>>();
        let schema_error = self.output_schema.as_ref().and_then(|schema| {
            let blocks = json_blocks(final_text);
            match blocks.as_slice() {
                [] => Some("no ```json block in the final message".to_string()),
                [only] => match serde_json::from_str::<Value>(only) {
                    Ok(value) => validate_subset(schema, &value, "$", 0).err(),
                    Err(error) => Some(format!("final ```json block is not valid JSON: {error}")),
                },
                _ => Some(format!(
                    "expected exactly one ```json block, found {}",
                    blocks.len()
                )),
            }
        });
        let evidence_missing = self.evidence_required
            && section_body(final_text, "Evidence")
                .map(|body| body.trim().is_empty())
                .unwrap_or(true);
        DeliverableStatus {
            check: DELIVERABLE_CHECK_STRUCTURAL.into(),
            met: missing_sections.is_empty() && schema_error.is_none() && !evidence_missing,
            missing_sections,
            schema_error,
            evidence_missing,
        }
    }
}

fn check_schema_depth(schema: &Value, depth: usize) -> Result<(), String> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err("deliverable.output_schema nests too deeply".into());
    }
    match schema {
        Value::Object(map) => map
            .values()
            .try_for_each(|value| check_schema_depth(value, depth + 1)),
        Value::Array(items) => items
            .iter()
            .try_for_each(|value| check_schema_depth(value, depth + 1)),
        _ => Ok(()),
    }
}

fn heading_matches(line: &str, section: &str) -> bool {
    let trimmed = line.trim();
    let stripped = trimmed.trim_start_matches('#');
    if stripped.len() == trimmed.len() {
        return trimmed
            .strip_suffix(':')
            .is_some_and(|name| name.trim().eq_ignore_ascii_case(section.trim()));
    }
    let name = stripped.trim().trim_end_matches(':').trim();
    name.eq_ignore_ascii_case(section.trim())
}

fn section_body(text: &str, section: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|line| heading_matches(line, section))?;
    let body = lines[start + 1..]
        .iter()
        .take_while(|line| !line.trim_start().starts_with('#'))
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    Some(body)
}

fn json_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        match &mut current {
            Some(block) => {
                if trimmed.starts_with("```") {
                    blocks.push(std::mem::take(block));
                    current = None;
                } else {
                    block.push_str(line);
                    block.push('\n');
                }
            }
            None => {
                if trimmed.eq_ignore_ascii_case("```json") {
                    current = Some(String::new());
                }
            }
        }
    }
    blocks
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn type_allows(expected: &str, value: &Value) -> bool {
    let actual = type_name(value);
    expected == actual || (expected == "number" && actual == "integer")
}

pub fn validate_subset(
    schema: &Value,
    value: &Value,
    path: &str,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(format!("{path}: schema nests too deeply"));
    }
    let Some(schema) = schema.as_object() else {
        return Err(format!("{path}: schema is not an object"));
    };
    match schema.get("type") {
        Some(Value::String(expected)) if !type_allows(expected, value) => {
            return Err(format!(
                "{path}: expected {expected}, found {}",
                type_name(value)
            ));
        }
        Some(Value::Array(options)) => {
            if !options.iter().any(|option| {
                option
                    .as_str()
                    .is_some_and(|expected| type_allows(expected, value))
            }) {
                return Err(format!(
                    "{path}: type {} is not permitted",
                    type_name(value)
                ));
            }
        }
        _ => {}
    }
    if let Some(Value::Array(options)) = schema.get("enum") {
        if !options.contains(value) {
            return Err(format!(
                "{path}: value is not one of the permitted enum values"
            ));
        }
    }
    if let Some(number) = value.as_f64() {
        if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
            if number < minimum {
                return Err(format!("{path}: {number} is below minimum {minimum}"));
            }
        }
        if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64) {
            if number > maximum {
                return Err(format!("{path}: {number} is above maximum {maximum}"));
            }
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count();
        if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
            if (length as u64) < min {
                return Err(format!("{path}: string shorter than {min}"));
            }
        }
        if let Some(max) = schema.get("maxLength").and_then(Value::as_u64) {
            if (length as u64) > max {
                return Err(format!("{path}: string longer than {max}"));
            }
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(Value::Array(required)) = schema.get("required") {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(format!("{path}: missing required property `{key}`"));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, child) in object {
            match properties.and_then(|properties| properties.get(key)) {
                Some(child_schema) => {
                    validate_subset(child_schema, child, &format!("{path}.{key}"), depth + 1)?
                }
                None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                    return Err(format!("{path}: unexpected property `{key}`"));
                }
                None => {}
            }
        }
    }
    if let (Some(items), Some(child_schema)) = (value.as_array(), schema.get("items")) {
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
            if (items.len() as u64) < min {
                return Err(format!("{path}: fewer than {min} items"));
            }
        }
        for (index, item) in items.iter().enumerate() {
            validate_subset(child_schema, item, &format!("{path}[{index}]"), depth + 1)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn contract() -> DeliverableContract {
        DeliverableContract {
            required_sections: vec!["Findings".into(), "Changes".into()],
            output_schema: Some(json!({
                "type": "object",
                "required": ["status", "files"],
                "properties": {
                    "status": {"type": "string", "enum": ["done", "blocked"]},
                    "files": {"type": "array", "items": {"type": "string"}},
                    "count": {"type": "integer", "minimum": 0}
                },
                "additionalProperties": false
            })),
            evidence_required: true,
        }
    }

    #[test]
    fn structural_evaluation_reports_each_unmet_element_without_claiming_success() {
        let status = contract().evaluate("# Findings\nnone\n\n# Evidence\n");
        assert_eq!(status.check, DELIVERABLE_CHECK_STRUCTURAL);
        assert!(!status.met);
        assert_eq!(status.missing_sections, vec!["Changes"]);
        assert!(status.evidence_missing);
        assert!(status
            .schema_error
            .as_deref()
            .unwrap()
            .contains("no ```json block"));
        let met = contract().evaluate(
            "## findings:\nx\n# Changes\ny\n```json\n{\"status\":\"done\",\"files\":[\"a.rs\"],\"count\":2}\n```\n# Evidence\ncargo test exit 0\n",
        );
        assert!(met.met, "{met:?}");
        let bad_schema = contract().evaluate(
            "# Findings\nx\n# Changes\ny\n```json\n{\"status\":\"maybe\",\"files\":[1]}\n```\n# Evidence\nok\n",
        );
        assert!(bad_schema
            .schema_error
            .as_deref()
            .unwrap()
            .contains("$.status"));
        let extra_property = contract().evaluate(
            "# Findings\nx\n# Changes\ny\n```json\n{\"status\":\"done\",\"files\":[],\"other\":1}\n```\n# Evidence\nok\n",
        );
        assert!(extra_property
            .schema_error
            .as_deref()
            .unwrap()
            .contains("unexpected property"));
        let two_blocks = contract().evaluate(
            "# Findings\nx\n# Changes\ny\n```json\n{}\n```\n```json\n{}\n```\n# Evidence\nok\n",
        );
        assert!(two_blocks
            .schema_error
            .as_deref()
            .unwrap()
            .contains("exactly one"));
    }

    #[test]
    fn contract_validation_bounds_sections_and_schema() {
        let mut invalid = contract();
        invalid.required_sections.push("findings".into());
        assert!(invalid.validate(16, 4096).is_err());
        let mut long = contract();
        long.required_sections = vec!["x".repeat(65)];
        assert!(long.validate(16, 4096).is_err());
        assert!(contract().validate(1, 4096).is_err());
        assert!(contract().validate(16, 8).is_err());
        let mut scalar = contract();
        scalar.output_schema = Some(json!("string"));
        assert!(scalar.validate(16, 4096).is_err());
        let mut deep = json!({});
        for _ in 0..20 {
            deep = json!({"properties": {"a": deep}});
        }
        let mut nested = contract();
        nested.output_schema = Some(deep);
        assert!(nested.validate(16, 1 << 20).is_err());
        assert!(contract().validate(16, 4096).is_ok());
        assert!(DeliverableContract::default().is_empty());
    }

    #[test]
    fn rendered_instructions_and_json_round_trip_are_stable() {
        let text = contract().render_instructions();
        assert!(text.contains("# Deliverable contract"));
        assert!(text.contains("`# Findings`"));
        assert!(text.contains("# Evidence"));
        let value = serde_json::to_value(contract()).unwrap();
        assert_eq!(
            serde_json::from_value::<DeliverableContract>(value).unwrap(),
            contract()
        );
        let legacy: DeliverableContract = serde_json::from_str("{}").unwrap();
        assert!(legacy.is_empty());
    }
}
