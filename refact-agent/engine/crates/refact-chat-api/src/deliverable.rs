use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DELIVERABLE_CHECK_STRUCTURAL: &str = "structural";
const MAX_SCHEMA_DEPTH: usize = 16;
const RESULT_BASE_TOKENS: usize = 64;
const RESULT_SECTION_TOKENS: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverableContract {
    #[serde(default)]
    pub required_sections: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub evidence_required: bool,
}

/// What the child could not be asked to produce, and where the full artifact is
/// still reachable. Emitted instead of a truncated section, never alongside a
/// met contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverableOmission {
    pub reason: String,
    pub reference: String,
    pub required_tokens: usize,
    pub available_tokens: usize,
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
    /// Correction attempts the child actually consumed. A contract met with a
    /// non-zero count was not met first try.
    #[serde(default)]
    pub corrections_used: u32,
    /// Every permitted correction was spent and the contract is still unmet.
    #[serde(default)]
    pub corrections_exhausted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omission: Option<DeliverableOmission>,
}

/// Settings-derived bound on correction work. `max_attempts == 0` keeps the
/// pre-correction behaviour exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrectionPolicy {
    pub max_attempts: u32,
    pub result_budget_tokens: usize,
}

impl Default for CorrectionPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            result_budget_tokens: 1024,
        }
    }
}

impl CorrectionPolicy {
    pub fn disabled() -> Self {
        Self {
            max_attempts: 0,
            result_budget_tokens: 0,
        }
    }
}

/// Per-run context facts: what is left to write the deliverable into, and the
/// recoverable pointer to the child's full output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultHeadroom {
    pub available_tokens: usize,
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectionRequest {
    pub attempt: u32,
    pub prompt: String,
}

impl DeliverableStatus {
    pub fn met_after_correction(&self) -> bool {
        self.met && self.corrections_used > 0
    }

    pub fn after_correction(mut self, attempt: u32) -> Self {
        self.corrections_used = attempt;
        self
    }
}

/// The single decision point for correction retries. Pure: the caller performs
/// the model call and folds the re-evaluated status back in through
/// [`DeliverableStatus::after_correction`].
///
/// `corrections_used` strictly increases on every returned request, so a caller
/// looping until `None` terminates in at most `policy.max_attempts` rounds.
pub fn plan_correction(
    contract: &DeliverableContract,
    status: DeliverableStatus,
    policy: &CorrectionPolicy,
    headroom: &ResultHeadroom,
) -> (DeliverableStatus, Option<CorrectionRequest>) {
    if policy.max_attempts == 0 {
        return (status, None);
    }
    let required = contract
        .estimated_result_tokens()
        .max(policy.result_budget_tokens);
    if headroom.available_tokens < required {
        let mut status = status;
        status.met = false;
        status.omission = Some(DeliverableOmission {
            reason: format!(
                "remaining context holds {} tokens and the declared deliverable needs {required}; \
                 the full deliverable was left unrequested rather than truncated",
                headroom.available_tokens
            ),
            reference: headroom.reference.clone(),
            required_tokens: required,
            available_tokens: headroom.available_tokens,
        });
        return (status, None);
    }
    if status.met {
        return (status, None);
    }
    if status.corrections_used >= policy.max_attempts {
        let mut status = status;
        status.corrections_exhausted = true;
        return (status, None);
    }
    let attempt = status.corrections_used + 1;
    let prompt = contract.correction_prompt(&status, attempt, policy.max_attempts);
    (status, Some(CorrectionRequest { attempt, prompt }))
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
            corrections_used: 0,
            corrections_exhausted: false,
            omission: None,
        }
    }

    /// Lower bound on the tokens a complete answer to this contract occupies.
    /// Deliberately coarse: it decides whether to ask at all, not how to write.
    pub fn estimated_result_tokens(&self) -> usize {
        let mut tokens = RESULT_BASE_TOKENS + self.required_sections.len() * RESULT_SECTION_TOKENS;
        if let Some(schema) = &self.output_schema {
            let bytes = serde_json::to_vec(schema)
                .map(|bytes| bytes.len())
                .unwrap_or(0);
            tokens += RESULT_SECTION_TOKENS + bytes / 4;
        }
        if self.evidence_required {
            tokens += RESULT_SECTION_TOKENS;
        }
        tokens
    }

    /// Names every element the previous message missed. Never an open-ended
    /// "try again".
    pub fn correction_prompt(
        &self,
        status: &DeliverableStatus,
        attempt: u32,
        max_attempts: u32,
    ) -> String {
        let mut lines = vec![format!(
            "# Deliverable correction {attempt} of {max_attempts}"
        )];
        lines.push(
            "Your previous final message did not satisfy the declared deliverable contract. Fix \
             exactly the elements listed below and post the complete final message again."
                .into(),
        );
        for section in &status.missing_sections {
            lines.push(format!(
                "- Missing required section `# {}`: add the heading followed by substantive content.",
                section.trim()
            ));
        }
        if let Some(error) = status.schema_error.as_ref() {
            lines.push(format!("- JSON block rejected: {error}"));
        }
        if status.evidence_missing {
            lines.push(
                "- Missing `# Evidence`: cite the exact commands, tool results, or files backing \
                 each claim."
                    .into(),
            );
        }
        lines.push(format!(
            "At most {max_attempts} correction attempts are allowed; after that the contract is \
             reported unmet. Satisfying this structural check means the output is well-formed, not \
             that the work is correct."
        ));
        lines.join("\n")
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

    const UNMET: &str = "# Findings\nx\n";
    const SATISFIED: &str = "# Findings\nx\n# Changes\ny\n```json\n{\"status\":\"done\",\"files\":[\"a.rs\"],\"count\":2}\n```\n# Evidence\ncargo test exit 0\n";

    fn ample() -> usize {
        contract().estimated_result_tokens() + 4096
    }

    fn drive(
        policy: &CorrectionPolicy,
        headroom_tokens: usize,
        first: &str,
        mut child: impl FnMut(u32) -> String,
    ) -> (DeliverableStatus, Vec<String>) {
        let contract = contract();
        let mut prompts = Vec::new();
        let mut status = contract.evaluate(first);
        loop {
            let headroom = ResultHeadroom {
                available_tokens: headroom_tokens,
                reference: "child-chat-1".into(),
            };
            let (next, request) = plan_correction(&contract, status, policy, &headroom);
            status = next;
            let Some(request) = request else { break };
            prompts.push(request.prompt.clone());
            status = contract
                .evaluate(&child(request.attempt))
                .after_correction(request.attempt);
        }
        (status, prompts)
    }

    #[test]
    fn an_unmet_contract_spends_exactly_one_correction_then_reports_unmet() {
        let (status, prompts) = drive(&CorrectionPolicy::default(), ample(), UNMET, |_| {
            UNMET.to_string()
        });
        assert_eq!(prompts.len(), 1);
        assert!(!status.met);
        assert_eq!(status.corrections_used, 1);
        assert!(status.corrections_exhausted);
        assert!(status.omission.is_none());
        assert!(!status.met_after_correction());
    }

    #[test]
    fn a_contract_met_on_the_second_attempt_is_recorded_as_met_after_correction() {
        let (status, prompts) = drive(&CorrectionPolicy::default(), ample(), UNMET, |_| {
            SATISFIED.to_string()
        });
        assert_eq!(prompts.len(), 1);
        assert!(status.met);
        assert_eq!(status.corrections_used, 1);
        assert!(status.met_after_correction());
        assert!(!status.corrections_exhausted);
        let first_try = contract().evaluate(SATISFIED);
        assert!(first_try.met);
        assert!(!first_try.met_after_correction());
    }

    #[test]
    fn zero_configured_attempts_reproduce_the_uncorrected_status() {
        for text in [UNMET, SATISFIED] {
            let (status, prompts) = drive(&CorrectionPolicy::disabled(), 0, text, |_| {
                panic!("a disabled policy must never call the child again")
            });
            assert!(prompts.is_empty());
            assert_eq!(status, contract().evaluate(text));
        }
    }

    #[test]
    fn insufficient_headroom_reports_a_recoverable_omission_instead_of_a_met_section() {
        let required = contract()
            .estimated_result_tokens()
            .max(CorrectionPolicy::default().result_budget_tokens);
        let (status, prompts) = drive(
            &CorrectionPolicy::default(),
            required - 1,
            SATISFIED,
            |_| panic!("no correction can be requested without headroom for the answer"),
        );
        assert!(prompts.is_empty());
        assert!(
            !status.met,
            "a truncated deliverable must never present as met"
        );
        let omission = status.omission.expect("recoverable omission reference");
        assert_eq!(omission.reference, "child-chat-1");
        assert_eq!(omission.required_tokens, required);
        assert_eq!(omission.available_tokens, required - 1);
        assert!(omission.reason.contains("rather than truncated"));
    }

    #[test]
    fn a_child_that_never_satisfies_the_contract_stops_at_the_configured_bound() {
        let policy = CorrectionPolicy {
            max_attempts: 3,
            ..CorrectionPolicy::default()
        };
        let calls = std::cell::Cell::new(0u32);
        let (status, prompts) = drive(&policy, ample(), UNMET, |attempt| {
            calls.set(calls.get() + 1);
            assert_eq!(attempt, calls.get());
            UNMET.to_string()
        });
        assert_eq!(calls.get(), 3);
        assert_eq!(prompts.len(), 3);
        assert_eq!(status.corrections_used, 3);
        assert!(!status.met);
        assert!(status.corrections_exhausted);
        assert!(prompts[0].contains("correction 1 of 3"));
        assert!(prompts[2].contains("correction 3 of 3"));
    }
}
