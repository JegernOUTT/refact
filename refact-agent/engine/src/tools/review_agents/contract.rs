use std::fmt;

use serde::Deserialize;

use crate::tools::review_types::{ReviewSeverity, StageCoverage};

pub const REPAIR_PROMPT: &str = "Your last message did not contain the required JSON object. Re-emit JSON only: no prose, no explanation, no markdown outside a single fenced json block, matching the output contract you were given. Parse error:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    NoJson,
    InvalidJson(String),
    WrongShape(String),
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoJson => write!(formatter, "no JSON object found in the final answer"),
            Self::InvalidJson(reason) => write!(formatter, "invalid JSON: {reason}"),
            Self::WrongShape(reason) => {
                write!(formatter, "JSON does not match the contract: {reason}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFinding {
    pub title: String,
    pub severity: ReviewSeverity,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    pub claim: String,
    pub evidence: String,
    pub reproduction: Option<String>,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageOutput {
    pub summary: String,
    pub findings: Vec<RawFinding>,
    pub coverage: StageCoverage,
    pub rejected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub id: String,
    pub supported: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerdictOutput {
    pub summary: String,
    pub verdicts: Vec<Verdict>,
    pub coverage: StageCoverage,
    pub rejected: Vec<String>,
}

#[derive(Deserialize)]
struct RawEnvelope {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    findings: Vec<serde_json::Value>,
    #[serde(default)]
    coverage: Option<StageCoverage>,
}

#[derive(Deserialize)]
struct RawVerdictEnvelope {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    verdicts: Vec<serde_json::Value>,
    #[serde(default)]
    coverage: Option<StageCoverage>,
}

#[derive(Deserialize)]
struct RawFindingFields {
    #[serde(default)]
    title: String,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    file: String,
    #[serde(default)]
    line_start: u32,
    #[serde(default)]
    line_end: u32,
    #[serde(default)]
    claim: String,
    #[serde(default)]
    evidence: String,
    #[serde(default)]
    reproduction: Option<String>,
    #[serde(default)]
    fix: Option<String>,
}

#[derive(Deserialize)]
struct RawVerdictFields {
    #[serde(default)]
    id: String,
    #[serde(default)]
    verdict: String,
    #[serde(default)]
    reason: String,
}

fn optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty() && text.to_ascii_lowercase() != "null")
}

fn balanced_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for index in start..bytes.len() {
        let character = bytes[index] as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=index]);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn extract_json(text: &str) -> Result<&str, ContractError> {
    let lowercase = text.to_ascii_lowercase();
    if let Some(start) = lowercase.rfind("```json") {
        let remaining = &text[start + "```json".len()..];
        if let Some(end) = remaining.find("```") {
            return Ok(remaining[..end].trim());
        }
        return Ok(remaining.trim());
    }
    if let Some(start) = lowercase.rfind("```") {
        let remaining = &text[start + 3..];
        let body = remaining.split("```").next().unwrap_or(remaining).trim();
        if body.starts_with('{') {
            return Ok(body);
        }
    }
    balanced_object(text).ok_or(ContractError::NoJson)
}

fn parse_finding(index: usize, value: serde_json::Value) -> Result<RawFinding, String> {
    let raw: RawFindingFields =
        serde_json::from_value(value).map_err(|error| format!("finding {}: {error}", index + 1))?;
    let file = raw.file.trim().to_string();
    if file.is_empty() {
        return Err(format!("finding {}: file is empty", index + 1));
    }
    let claim = raw.claim.trim().to_string();
    if claim.is_empty() {
        return Err(format!("finding {}: claim is empty", index + 1));
    }
    let severity = ReviewSeverity::parse(&raw.severity).ok_or_else(|| {
        format!(
            "finding {}: unknown severity {:?}",
            index + 1,
            raw.severity.trim()
        )
    })?;
    let line_start = raw.line_start.max(1);
    let line_end = raw.line_end.max(line_start);
    let title = match raw.title.trim() {
        "" => claim.clone(),
        title => title.to_string(),
    };
    Ok(RawFinding {
        title,
        severity,
        file,
        line_start,
        line_end,
        claim,
        evidence: raw.evidence.trim().to_string(),
        reproduction: optional_text(raw.reproduction),
        fix: optional_text(raw.fix),
    })
}

pub fn parse_stage_output(text: &str) -> Result<StageOutput, ContractError> {
    let json = extract_json(text)?;
    let envelope: RawEnvelope = serde_json::from_str(json)
        .map_err(|error| ContractError::InvalidJson(error.to_string()))?;
    let mut findings = Vec::with_capacity(envelope.findings.len());
    let mut rejected = Vec::new();
    for (index, value) in envelope.findings.into_iter().enumerate() {
        match parse_finding(index, value) {
            Ok(finding) => findings.push(finding),
            Err(reason) => rejected.push(reason),
        }
    }
    Ok(StageOutput {
        summary: envelope.summary.trim().to_string(),
        findings,
        coverage: envelope.coverage.unwrap_or_default(),
        rejected,
    })
}

pub fn parse_verdict_output(text: &str) -> Result<VerdictOutput, ContractError> {
    let json = extract_json(text)?;
    let envelope: RawVerdictEnvelope = serde_json::from_str(json)
        .map_err(|error| ContractError::InvalidJson(error.to_string()))?;
    let mut verdicts = Vec::with_capacity(envelope.verdicts.len());
    let mut rejected = Vec::new();
    for (index, value) in envelope.verdicts.into_iter().enumerate() {
        let raw: RawVerdictFields = match serde_json::from_value(value) {
            Ok(raw) => raw,
            Err(error) => {
                rejected.push(format!("verdict {}: {error}", index + 1));
                continue;
            }
        };
        let id = raw.id.trim().to_string();
        if id.is_empty() {
            rejected.push(format!("verdict {}: id is empty", index + 1));
            continue;
        }
        let supported = match raw.verdict.trim().to_ascii_lowercase().as_str() {
            "supported" | "confirmed" | "valid" => true,
            "unsupported" | "refuted" | "invalid" | "disproved" => false,
            other => {
                rejected.push(format!("verdict {}: unknown verdict {other:?}", index + 1));
                continue;
            }
        };
        verdicts.push(Verdict {
            id,
            supported,
            reason: raw.reason.trim().to_string(),
        });
    }
    Ok(VerdictOutput {
        summary: envelope.summary.trim().to_string(),
        verdicts,
        coverage: envelope.coverage.unwrap_or_default(),
        rejected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FINDING: &str = r#"{"title":"Error dropped","severity":"blocker","file":"src/lib.rs","line_start":10,"line_end":14,"claim":"The error arm returns success.","evidence":"return Ok(());","reproduction":"cargo test -p thing","fix":"propagate"}"#;

    fn envelope(findings: &str) -> String {
        format!(
            r#"```json
{{"stage":"diff","model":"m","findings":[{findings}],"summary":"checked five files","coverage":{{"files_read":["src/lib.rs"],"commands_run":[{{"cmd":"cargo check","exit":0}}],"tools_unavailable":[],"stopped_early":null}}}}
```"#
        )
    }

    #[test]
    fn review_contract_parses_findings_coverage_and_commands() {
        let output = parse_stage_output(&envelope(FINDING)).unwrap();

        assert_eq!(output.findings.len(), 1);
        assert_eq!(output.findings[0].severity, ReviewSeverity::Blocker);
        assert_eq!(output.findings[0].line_end, 14);
        assert_eq!(
            output.findings[0].reproduction.as_deref(),
            Some("cargo test -p thing")
        );
        assert_eq!(output.summary, "checked five files");
        assert_eq!(output.coverage.files_read, ["src/lib.rs"]);
        assert_eq!(output.coverage.commands_run[0].cmd, "cargo check");
        assert_eq!(output.coverage.commands_run[0].exit, 0);
        assert!(output.rejected.is_empty());
    }

    #[test]
    fn review_contract_accepts_bare_json_and_prose_around_it() {
        let bare = format!(
            r#"Here is what I found.

{{"findings":[{FINDING}],"summary":"done"}}"#
        );

        let output = parse_stage_output(&bare).unwrap();

        assert_eq!(output.findings.len(), 1);
        assert_eq!(output.summary, "done");
    }

    #[test]
    fn review_contract_accepts_no_findings_as_a_valid_result() {
        let output = parse_stage_output(&envelope("")).unwrap();

        assert!(output.findings.is_empty());
        assert!(output.rejected.is_empty());
    }

    #[test]
    fn review_contract_rejects_bad_entries_without_losing_the_good_ones() {
        let bad_severity = FINDING.replace(r#""severity":"blocker""#, r#""severity":"spicy""#);
        let no_file = FINDING.replace(r#""file":"src/lib.rs""#, r#""file":"  ""#);
        let output =
            parse_stage_output(&envelope(&format!("{FINDING},{bad_severity},{no_file}"))).unwrap();

        assert_eq!(output.findings.len(), 1);
        assert_eq!(output.rejected.len(), 2);
        assert!(output.rejected[0].contains("unknown severity"));
        assert!(output.rejected[1].contains("file is empty"));
    }

    #[test]
    fn review_contract_normalizes_missing_titles_and_zero_lines() {
        let minimal = r#"{"findings":[{"severity":"low","file":"a.rs","line_start":0,"line_end":0,"claim":"A claim","evidence":"x","reproduction":null}],"summary":""}"#;

        let output = parse_stage_output(minimal).unwrap();

        assert_eq!(output.findings[0].title, "A claim");
        assert_eq!(output.findings[0].line_start, 1);
        assert_eq!(output.findings[0].line_end, 1);
        assert_eq!(output.findings[0].reproduction, None);
    }

    #[test]
    fn review_contract_reports_unparseable_output() {
        assert_eq!(
            parse_stage_output("no json here"),
            Err(ContractError::NoJson)
        );
        assert!(matches!(
            parse_stage_output("```json\n{oops}\n```"),
            Err(ContractError::InvalidJson(_))
        ));
    }

    #[test]
    fn review_contract_parses_adversarial_verdicts() {
        let text = r#"```json
{"stage":"adversarial","verdicts":[{"id":"rf-1","verdict":"unsupported","reason":"guarded at src/lib.rs:4"},{"id":"rf-2","verdict":"supported","reason":""},{"id":"rf-3","verdict":"maybe","reason":""}],"summary":"checked three"}
```"#;

        let output = parse_verdict_output(text).unwrap();

        assert_eq!(output.verdicts.len(), 2);
        assert!(!output.verdicts[0].supported);
        assert_eq!(output.verdicts[0].reason, "guarded at src/lib.rs:4");
        assert!(output.verdicts[1].supported);
        assert_eq!(output.rejected.len(), 1);
    }
}
