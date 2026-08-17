use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDepth {
    Quick,
    Standard,
    Deep,
}

impl ReviewDepth {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "quick" => Some(Self::Quick),
            "standard" => Some(Self::Standard),
            "deep" => Some(Self::Deep),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Standard => "standard",
            Self::Deep => "deep",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Low,
    Medium,
    High,
    Critical,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Unverified,
    Verified,
    Downgraded,
    Rejected,
    NeedsHumanValidation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RankTier {
    ExecutionReproduced,
    Corroborated,
    Verified,
    NeedsHumanValidation,
    #[default]
    Unverified,
    Downgraded,
}

impl RankTier {
    pub fn rank(&self) -> u8 {
        match self {
            Self::ExecutionReproduced => 5,
            Self::Corroborated => 4,
            Self::Verified => 3,
            Self::NeedsHumanValidation => 2,
            Self::Unverified => 1,
            Self::Downgraded => 0,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::ExecutionReproduced => "execution-reproduced",
            Self::Corroborated => "corroborated",
            Self::Verified => "verified",
            Self::NeedsHumanValidation => "needs human validation",
            Self::Unverified => "unverified",
            Self::Downgraded => "downgraded",
        }
    }
}
pub mod evidence_kinds {
    pub const EXCERPT: &str = "excerpt";
    pub const DIFF_HUNK: &str = "diff_hunk";
    pub const SYMBOL: &str = "symbol";
    pub const CHECK: &str = "check";
    pub const STATIC_FACT: &str = "static_fact";
    pub const COMMAND_OUTPUT: &str = "command_output";
    pub const EXECUTION_OUTPUT: &str = "execution_output";
    pub const MUTATION_PROBE: &str = "mutation_probe";
    pub const SCREENSHOT: &str = "screenshot";
    pub const CONSOLE_LOG: &str = "console_log";
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewEvidence {
    pub kind: String,
    pub path: Option<String>,
    pub line1: Option<u32>,
    pub line2: Option<u32>,
    pub content: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewFinding {
    pub id: String,
    pub category: String,
    pub severity: ReviewSeverity,
    pub confidence: f32,
    pub verification_status: VerificationStatus,
    #[serde(default)]
    pub rank_tier: RankTier,
    #[serde(default)]
    pub sources: Vec<String>,
    pub file: String,
    pub line1: u32,
    pub line2: u32,
    pub claim: String,
    pub evidence: Vec<ReviewEvidence>,
    pub impact: Option<String>,
    pub remediation: Option<String>,
    pub checks_performed: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewScopeSummary {
    pub files_reviewed: Vec<String>,
    pub focus: Option<String>,
    pub diff_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MechanicalCheck {
    pub name: String,
    pub command: Vec<String>,
    pub exit_status: i32,
    pub output_excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MechanicalResult {
    pub passed: bool,
    pub checks: Vec<MechanicalCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStageStatus {
    Skipped,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewStage {
    pub name: String,
    pub status: ReviewStageStatus,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Ran,
    Skipped,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentRunReport {
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub status: AgentRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub candidates: usize,
    pub survived: usize,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewPipelineMetadata {
    pub stages: Vec<ReviewStage>,
    pub stopped_reason: Option<String>,
    pub mechanical: Option<MechanicalResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentRunReport>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewReport {
    pub scope: ReviewScopeSummary,
    pub findings: Vec<ReviewFinding>,
    pub checks_performed: Vec<String>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assumed_intent: Option<String>,
    #[serde(default)]
    pub pipeline: ReviewPipelineMetadata,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_review_report_serde_roundtrip_covers_all_enum_variants() {
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
        let tiers = [
            RankTier::Unverified,
            RankTier::ExecutionReproduced,
            RankTier::Corroborated,
            RankTier::Verified,
            RankTier::Downgraded,
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
                .zip(tiers)
                .enumerate()
                .map(
                    |(index, ((severity, verification_status), rank_tier))| ReviewFinding {
                        id: format!("finding-{index}"),
                        category: "correctness".to_string(),
                        severity,
                        confidence: 0.9,
                        verification_status,
                        rank_tier,
                        sources: vec![
                            "l1_diff@thinking".to_string(),
                            "s4_test_integrity".to_string(),
                        ],
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
                    },
                )
                .collect(),
            checks_performed: vec!["cargo check".to_string()],
            summary: "Review summary".to_string(),
            assumed_intent: Some("Ship the parser".to_string()),
            pipeline: ReviewPipelineMetadata {
                stages: vec![ReviewStage {
                    name: "mechanical".to_string(),
                    status: ReviewStageStatus::Completed,
                    reason: None,
                }],
                stopped_reason: None,
                mechanical: Some(MechanicalResult {
                    passed: true,
                    checks: vec![MechanicalCheck {
                        name: "cargo check".to_string(),
                        command: vec!["cargo".to_string(), "check".to_string()],
                        exit_status: 0,
                        output_excerpt: "Finished".to_string(),
                    }],
                }),
                depth: Some("standard".to_string()),
                agents: vec![AgentRunReport {
                    agent: "l1_diff".to_string(),
                    model: Some("thinking-model".to_string()),
                    status: AgentRunStatus::Ran,
                    reason: None,
                    candidates: 3,
                    survived: 2,
                    duration_ms: 1200,
                    steps: Some(1),
                }],
            },
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
        assert_eq!(value["findings"][1]["rank_tier"], "execution_reproduced");
        assert_eq!(value["findings"][2]["rank_tier"], "corroborated");
        assert_eq!(value["findings"][0]["sources"][0], "l1_diff@thinking");
        assert_eq!(value["pipeline"]["stages"][0]["status"], "completed");
        assert_eq!(value["pipeline"]["depth"], "standard");
        assert_eq!(value["pipeline"]["agents"][0]["agent"], "l1_diff");
        assert_eq!(value["pipeline"]["agents"][0]["status"], "ran");
        assert_eq!(value["assumed_intent"], "Ship the parser");
        assert_eq!(
            value["pipeline"]["mechanical"]["checks"][0]["exit_status"],
            0
        );
        assert_eq!(
            serde_json::from_value::<ReviewReport>(value).unwrap(),
            report
        );
    }

    #[test]
    fn tool_review_finding_deserializes_legacy_json_without_new_fields() {
        let legacy = serde_json::json!({
            "id": "rf-1234abcd",
            "category": "correctness",
            "severity": "high",
            "confidence": 0.8,
            "verification_status": "verified",
            "file": "src/lib.rs",
            "line1": 1,
            "line2": 2,
            "claim": "A claim",
            "evidence": [],
            "impact": null,
            "remediation": null,
            "checks_performed": []
        });
        let finding: ReviewFinding = serde_json::from_value(legacy).unwrap();
        assert_eq!(finding.rank_tier, RankTier::Unverified);
        assert!(finding.sources.is_empty());
    }

    #[test]
    fn tool_review_depth_parses_known_values_only() {
        assert_eq!(ReviewDepth::parse("quick"), Some(ReviewDepth::Quick));
        assert_eq!(
            ReviewDepth::parse(" Standard "),
            Some(ReviewDepth::Standard)
        );
        assert_eq!(ReviewDepth::parse("DEEP"), Some(ReviewDepth::Deep));
        assert_eq!(ReviewDepth::parse("max"), None);
        assert!(ReviewDepth::Quick < ReviewDepth::Standard);
        assert!(ReviewDepth::Standard < ReviewDepth::Deep);
    }

    #[test]
    fn tool_review_rank_tier_ordering_matches_evidence_strength() {
        assert!(RankTier::ExecutionReproduced.rank() > RankTier::Corroborated.rank());
        assert!(RankTier::Corroborated.rank() > RankTier::Verified.rank());
        assert!(RankTier::Verified.rank() > RankTier::NeedsHumanValidation.rank());
        assert!(RankTier::NeedsHumanValidation.rank() > RankTier::Unverified.rank());
        assert!(RankTier::Unverified.rank() > RankTier::Downgraded.rank());
    }
}
