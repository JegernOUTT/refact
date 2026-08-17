use std::fmt;

use serde::Deserialize;

use crate::tools::review_types::{ReviewFinding, ReviewSeverity, VerificationStatus};

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateFinding {
    pub file: String,
    pub line1: u32,
    pub line2: u32,
    pub category: String,
    pub severity: ReviewSeverity,
    pub confidence: f32,
    pub claim: String,
    pub rationale: String,
}

impl CandidateFinding {
    pub(crate) fn into_review_parts(self) -> (ReviewFinding, String) {
        (
            ReviewFinding {
                id: String::new(),
                category: self.category,
                severity: self.severity,
                confidence: self.confidence,
                verification_status: VerificationStatus::Unverified,
                rank_tier: Default::default(),
                sources: Vec::new(),
                file: self.file,
                line1: self.line1,
                line2: self.line2,
                claim: self.claim,
                evidence: Vec::new(),
                impact: None,
                remediation: None,
                checks_performed: Vec::new(),
            },
            self.rationale,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateParseError {
    MissingJsonBlock,
    InvalidJson(String),
    NoValidCandidates(Vec<String>),
}

impl fmt::Display for CandidateParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingJsonBlock => write!(formatter, "missing fenced json block"),
            Self::InvalidJson(reason) => write!(formatter, "invalid candidate json: {reason}"),
            Self::NoValidCandidates(reasons) => {
                write!(formatter, "no valid candidates: {}", reasons.join("; "))
            }
        }
    }
}

impl std::error::Error for CandidateParseError {}

#[derive(Debug, PartialEq)]
pub(crate) struct ParsedCandidates {
    pub summary: String,
    pub candidates: Vec<CandidateFinding>,
    pub skipped_reasons: Vec<String>,
}

#[derive(Deserialize)]
struct CandidateEnvelope {
    summary: String,
    candidates: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct RawCandidateFinding {
    file: String,
    line1: u32,
    line2: u32,
    category: String,
    severity: String,
    confidence: f32,
    claim: String,
    rationale: String,
}

pub(crate) fn extract_last_json_block(text: &str) -> Result<&str, CandidateParseError> {
    let lowercase = text.to_ascii_lowercase();
    let start = lowercase
        .rfind("```json")
        .ok_or(CandidateParseError::MissingJsonBlock)?;
    let content_start = start + "```json".len();
    let remaining = &text[content_start..];
    let end = remaining
        .find("```")
        .ok_or(CandidateParseError::MissingJsonBlock)?;
    Ok(remaining[..end].trim())
}

fn normalize_category(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "correctness"
            | "consistency"
            | "security"
            | "tests"
            | "maintainability"
            | "performance"
            | "spec_compliance"
    )
    .then_some(normalized)
}

fn normalize_severity(value: &str) -> Option<ReviewSeverity> {
    match value.trim().to_ascii_lowercase().as_str() {
        "low" => Some(ReviewSeverity::Low),
        "medium" => Some(ReviewSeverity::Medium),
        "high" => Some(ReviewSeverity::High),
        "critical" => Some(ReviewSeverity::Critical),
        _ => None,
    }
}

fn validate_candidate(index: usize, value: serde_json::Value) -> Result<CandidateFinding, String> {
    let raw: RawCandidateFinding = serde_json::from_value(value)
        .map_err(|error| format!("candidate {}: {error}", index + 1))?;
    let file = raw.file.trim().to_string();
    if file.is_empty() {
        return Err(format!("candidate {}: file is empty", index + 1));
    }
    let claim = raw.claim.trim().to_string();
    if claim.is_empty() {
        return Err(format!("candidate {}: claim is empty", index + 1));
    }
    if raw.line1 > raw.line2 {
        return Err(format!(
            "candidate {}: line1 {} exceeds line2 {}",
            index + 1,
            raw.line1,
            raw.line2
        ));
    }
    let category = normalize_category(&raw.category).ok_or_else(|| {
        format!(
            "candidate {}: unknown category {:?}",
            index + 1,
            raw.category
        )
    })?;
    let severity = normalize_severity(&raw.severity).ok_or_else(|| {
        format!(
            "candidate {}: unknown severity {:?}",
            index + 1,
            raw.severity
        )
    })?;

    Ok(CandidateFinding {
        file,
        line1: raw.line1,
        line2: raw.line2,
        category,
        severity,
        confidence: raw.confidence.clamp(0.0, 1.0),
        claim,
        rationale: raw.rationale.trim().to_string(),
    })
}

pub(crate) fn parse_candidates_with_reasons(
    text: &str,
) -> Result<ParsedCandidates, CandidateParseError> {
    let json = extract_last_json_block(text)?;
    let envelope: CandidateEnvelope = serde_json::from_str(json)
        .map_err(|error| CandidateParseError::InvalidJson(error.to_string()))?;
    let input_count = envelope.candidates.len();
    let mut candidates = Vec::with_capacity(input_count);
    let mut skipped_reasons = Vec::new();

    for (index, value) in envelope.candidates.into_iter().enumerate() {
        match validate_candidate(index, value) {
            Ok(candidate) => candidates.push(candidate),
            Err(reason) => skipped_reasons.push(reason),
        }
    }

    if input_count > 0 && candidates.is_empty() {
        return Err(CandidateParseError::NoValidCandidates(skipped_reasons));
    }

    for reason in &skipped_reasons {
        tracing::warn!("review: {reason}");
    }

    Ok(ParsedCandidates {
        summary: envelope.summary.trim().to_string(),
        candidates,
        skipped_reasons,
    })
}

pub fn parse_candidates(
    text: &str,
) -> Result<(String, Vec<CandidateFinding>), CandidateParseError> {
    let parsed = parse_candidates_with_reasons(text)?;
    Ok((parsed.summary, parsed.candidates))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate_json(overrides: &str) -> String {
        format!(
            r#"{{"file":"src/lib.rs","line1":1,"line2":10,"category":"correctness","severity":"high","confidence":0.8,"claim":"The branch drops errors.","rationale":"The error arm returns success."{overrides}}}"#
        )
    }

    fn response(candidates: &str) -> String {
        format!(
            "```json\n{{\"summary\":\"Found plausible issues.\",\"candidates\":[{candidates}]}}\n```"
        )
    }

    #[test]
    fn tool_review_parse_candidates_accepts_well_formed_output() {
        let (summary, candidates) = parse_candidates(&response(&candidate_json(""))).unwrap();

        assert_eq!(summary, "Found plausible issues.");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].severity, ReviewSeverity::High);
    }

    #[test]
    fn tool_review_parse_candidates_ignores_surrounding_prose() {
        let text = format!("analysis before\n{}\nafter", response(&candidate_json("")));
        let (_, candidates) = parse_candidates(&text).unwrap();

        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn tool_review_parse_candidates_uses_last_json_block() {
        let first = response("");
        let second = response(&candidate_json(""));
        let (summary, candidates) = parse_candidates(&format!("{first}\n{second}")).unwrap();

        assert_eq!(summary, "Found plausible issues.");
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn tool_review_parse_candidates_skips_invalid_entries_with_reasons() {
        let valid = candidate_json("");
        let invalid_lines = candidate_json(",\"line1\":20");
        let invalid_category = candidate_json(",\"category\":\"style\"");
        let parsed = parse_candidates_with_reasons(&response(&format!(
            "{valid},{invalid_lines},{invalid_category}"
        )))
        .unwrap();

        assert_eq!(parsed.candidates.len(), 1);
        assert_eq!(parsed.skipped_reasons.len(), 2);
        assert!(parsed.skipped_reasons[0].contains("line1 20 exceeds line2 10"));
        assert!(parsed.skipped_reasons[1].contains("unknown category"));
    }

    #[test]
    fn tool_review_parse_candidates_rejects_malformed_output() {
        assert_eq!(
            parse_candidates("not json").unwrap_err(),
            CandidateParseError::MissingJsonBlock
        );
        assert!(matches!(
            parse_candidates("```json\n{broken}\n```").unwrap_err(),
            CandidateParseError::InvalidJson(_)
        ));
    }

    #[test]
    fn tool_review_parse_candidates_accepts_empty_candidates() {
        let (summary, candidates) = parse_candidates(&response("")).unwrap();

        assert_eq!(summary, "Found plausible issues.");
        assert!(candidates.is_empty());
    }

    #[test]
    fn tool_review_parse_candidates_normalizes_and_clamps_values() {
        let input = candidate_json(
            ",\"category\":\" SECURITY \",\"severity\":\" CRITICAL \",\"confidence\":4.2",
        );
        let (_, candidates) = parse_candidates(&response(&input)).unwrap();

        assert_eq!(candidates[0].category, "security");
        assert_eq!(candidates[0].severity, ReviewSeverity::Critical);
        assert_eq!(candidates[0].confidence, 1.0);
    }

    #[test]
    fn tool_review_candidate_rationale_is_separate_from_review_finding() {
        let (_, mut candidates) = parse_candidates(&response(&candidate_json(""))).unwrap();
        let (finding, rationale) = candidates.remove(0).into_review_parts();
        let serialized = serde_json::to_string(&finding).unwrap();

        assert_eq!(rationale, "The error arm returns success.");
        assert!(!serialized.contains("rationale"));
        assert!(!serialized.contains("error arm returns success"));
        assert_eq!(finding.verification_status, VerificationStatus::Unverified);
        assert!(finding.id.is_empty());
        assert!(finding.evidence.is_empty());
    }
}
