use serde::{Deserialize, Serialize};

/// Severity assigned to a code review finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Outcome of verifying a code review finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Unverified,
    Verified,
    Downgraded,
    Rejected,
    NeedsHumanValidation,
}

/// Evidence supporting a code review finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewEvidence {
    pub kind: String,
    pub path: Option<String>,
    pub line1: Option<u32>,
    pub line2: Option<u32>,
    pub content: String,
}

/// Structured code review finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewFinding {
    pub id: String,
    pub category: String,
    pub severity: ReviewSeverity,
    pub confidence: f32,
    pub verification_status: VerificationStatus,
    pub file: String,
    pub line1: u32,
    pub line2: u32,
    pub claim: String,
    pub evidence: Vec<ReviewEvidence>,
    pub impact: Option<String>,
    pub remediation: Option<String>,
    pub checks_performed: Vec<String>,
}

/// Files and optional constraints covered by a code review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewScopeSummary {
    pub files_reviewed: Vec<String>,
    pub focus: Option<String>,
    pub diff_base: Option<String>,
}

/// Structured result of the code review pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewReport {
    pub scope: ReviewScopeSummary,
    pub findings: Vec<ReviewFinding>,
    pub checks_performed: Vec<String>,
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_code_review_report_serde_roundtrip_covers_all_enum_variants() {
        let severities = [
            ReviewSeverity::Low,
            ReviewSeverity::Medium,
            ReviewSeverity::High,
            ReviewSeverity::Critical,
            ReviewSeverity::Critical,
        ];
        let statuses = [
            VerificationStatus::Unverified,
            VerificationStatus::Verified,
            VerificationStatus::Downgraded,
            VerificationStatus::Rejected,
            VerificationStatus::NeedsHumanValidation,
        ];
        let report = ReviewReport {
            scope: ReviewScopeSummary {
                files_reviewed: vec!["src/lib.rs".to_string()],
                focus: Some("security".to_string()),
                diff_base: Some("main".to_string()),
            },
            findings: severities
                .into_iter()
                .zip(statuses)
                .enumerate()
                .map(|(index, (severity, verification_status))| ReviewFinding {
                    id: format!("finding-{index}"),
                    category: "correctness".to_string(),
                    severity,
                    confidence: 0.9,
                    verification_status,
                    file: "src/lib.rs".to_string(),
                    line1: 10,
                    line2: 12,
                    claim: "A claim".to_string(),
                    evidence: vec![ReviewEvidence {
                        kind: "excerpt".to_string(),
                        path: Some("src/lib.rs".to_string()),
                        line1: Some(10),
                        line2: Some(12),
                        content: "let value = 1;".to_string(),
                    }],
                    impact: Some("An impact".to_string()),
                    remediation: Some("A remediation".to_string()),
                    checks_performed: vec!["cargo check".to_string()],
                })
                .collect(),
            checks_performed: vec!["cargo check".to_string()],
            summary: "Review summary".to_string(),
        };

        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["findings"][0]["severity"], "low");
        assert_eq!(value["findings"][1]["severity"], "medium");
        assert_eq!(value["findings"][2]["severity"], "high");
        assert_eq!(value["findings"][3]["severity"], "critical");
        assert_eq!(value["findings"][0]["verification_status"], "unverified");
        assert_eq!(value["findings"][1]["verification_status"], "verified");
        assert_eq!(value["findings"][2]["verification_status"], "downgraded");
        assert_eq!(value["findings"][3]["verification_status"], "rejected");
        assert_eq!(
            value["findings"][4]["verification_status"],
            "needs_human_validation"
        );
        assert_eq!(
            serde_json::from_value::<ReviewReport>(value).unwrap(),
            report
        );
    }
}
