use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDepth {
    Normal,
    Deep,
}

impl ReviewDepth {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "normal" | "quick" | "standard" => Some(Self::Normal),
            "deep" => Some(Self::Deep),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Deep => "deep",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Note,
    Low,
    Medium,
    High,
    Blocker,
}

impl ReviewSeverity {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "blocker" | "critical" | "fatal" => Some(Self::Blocker),
            "high" | "major" => Some(Self::High),
            "medium" | "moderate" => Some(Self::Medium),
            "low" | "minor" => Some(Self::Low),
            "note" | "info" | "nit" | "trivial" => Some(Self::Note),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blocker => "blocker",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Note => "note",
        }
    }

    pub fn rank(&self) -> u8 {
        match self {
            Self::Blocker => 4,
            Self::High => 3,
            Self::Medium => 2,
            Self::Low => 1,
            Self::Note => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeMode {
    Strict,
    Adjacent,
    #[default]
    Broad,
}

impl ScopeMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "strict" => Some(Self::Strict),
            "adjacent" => Some(Self::Adjacent),
            "broad" => Some(Self::Broad),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Adjacent => "adjacent",
            Self::Broad => "broad",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CommandRun {
    pub cmd: String,
    pub exit: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StageCoverage {
    #[serde(default)]
    pub files_read: Vec<String>,
    #[serde(default)]
    pub commands_run: Vec<CommandRun>,
    #[serde(default)]
    pub tools_unavailable: Vec<String>,
    #[serde(default)]
    pub stopped_early: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatusKind {
    Ok,
    TimedOut,
    Failed,
    NotRun,
}

impl StageStatusKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::TimedOut => "timed out",
            Self::Failed => "failed",
            Self::NotRun => "not run",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StageRun {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub status: StageStatusKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub duration_ms: u64,
    pub findings: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub coverage: StageCoverage,
}

impl StageRun {
    fn base(name: &str, status: StageStatusKind, reason: Option<String>) -> Self {
        Self {
            name: name.to_string(),
            model: None,
            status,
            reason,
            duration_ms: 0,
            findings: 0,
            summary: None,
            coverage: StageCoverage::default(),
        }
    }

    pub fn ok(name: &str, model: Option<String>, duration_ms: u64) -> Self {
        let mut run = Self::base(name, StageStatusKind::Ok, None);
        run.model = model;
        run.duration_ms = duration_ms;
        run
    }

    pub fn timed_out(name: &str, model: Option<String>, duration_ms: u64, reason: &str) -> Self {
        let mut run = Self::base(name, StageStatusKind::TimedOut, Some(reason.to_string()));
        run.model = model;
        run.duration_ms = duration_ms;
        run
    }

    pub fn failed(name: &str, model: Option<String>, duration_ms: u64, reason: &str) -> Self {
        let mut run = Self::base(name, StageStatusKind::Failed, Some(reason.to_string()));
        run.model = model;
        run.duration_ms = duration_ms;
        run
    }

    pub fn not_run(name: &str, reason: &str) -> Self {
        Self::base(name, StageStatusKind::NotRun, Some(reason.to_string()))
    }

    pub fn is_complete(&self) -> bool {
        self.status == StageStatusKind::Ok
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FindingLocation {
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Dispute {
    pub stage: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewFinding {
    pub id: String,
    pub stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub title: String,
    pub severity: ReviewSeverity,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    pub claim: String,
    pub evidence: String,
    pub evidence_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reproduction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    pub introduced_by_diff: bool,
    pub out_of_scope: bool,
    #[serde(default)]
    pub reported_by: Vec<String>,
    #[serde(default)]
    pub locations: Vec<FindingLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disputed: Option<Dispute>,
}

impl ReviewFinding {
    pub fn is_hypothesis(&self) -> bool {
        if self.disputed.is_some() {
            return true;
        }
        self.reproduction.is_none() && !self.evidence_present
    }

    pub fn has_reproduction(&self) -> bool {
        self.reproduction
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewScopeSummary {
    pub mode: String,
    pub requested_files: usize,
    pub reviewed_files: usize,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion: Option<String>,
    #[serde(default)]
    pub out_of_scope_findings: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewDiffSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    pub changed_files: usize,
    pub hunks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewReport {
    pub depth: String,
    pub scope: ReviewScopeSummary,
    pub diff: ReviewDiffSummary,
    pub stages: Vec<StageRun>,
    pub findings: Vec<ReviewFinding>,
    pub duration_ms: u64,
    #[serde(default)]
    pub duplicates_merged: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scratch_dir: Option<String>,
}

impl ReviewReport {
    pub fn facts(&self) -> Vec<&ReviewFinding> {
        self.findings
            .iter()
            .filter(|finding| !finding.is_hypothesis())
            .collect()
    }

    pub fn hypotheses(&self) -> Vec<&ReviewFinding> {
        self.findings
            .iter()
            .filter(|finding| finding.is_hypothesis())
            .collect()
    }

    pub fn reproduced(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| !finding.is_hypothesis() && finding.has_reproduction())
            .count()
    }

    pub fn pre_existing(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| !finding.introduced_by_diff)
            .count()
    }

    pub fn incomplete_stages(&self) -> Vec<&StageRun> {
        self.stages
            .iter()
            .filter(|stage| !stage.is_complete())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(id: &str) -> ReviewFinding {
        ReviewFinding {
            id: id.to_string(),
            stage: "diff".to_string(),
            model: Some("some-model".to_string()),
            title: "Error is dropped".to_string(),
            severity: ReviewSeverity::High,
            file: "src/lib.rs".to_string(),
            line_start: 10,
            line_end: 14,
            claim: "The error arm returns success.".to_string(),
            evidence: "return Ok(());".to_string(),
            evidence_present: true,
            reproduction: Some("cargo test -p thing failing_case".to_string()),
            fix: Some("Propagate the error.".to_string()),
            introduced_by_diff: true,
            out_of_scope: false,
            reported_by: vec!["diff".to_string()],
            locations: vec![],
            disputed: None,
        }
    }

    #[test]
    fn review_report_serde_roundtrip_keeps_every_field() {
        let mut hypothesis = finding("rf-2");
        hypothesis.reproduction = None;
        hypothesis.evidence_present = false;
        hypothesis.severity = ReviewSeverity::Note;
        let report = ReviewReport {
            depth: "deep".to_string(),
            scope: ReviewScopeSummary {
                mode: "strict".to_string(),
                requested_files: 12,
                reviewed_files: 14,
                files: vec!["src/lib.rs".to_string()],
                focus: Some("browser lifecycle".to_string()),
                expansion: Some("+2 dependency edges".to_string()),
                out_of_scope_findings: 1,
            },
            diff: ReviewDiffSummary {
                base: Some("1a2b3c".to_string()),
                head: Some("HEAD".to_string()),
                changed_files: 7,
                hunks: 31,
            },
            stages: vec![
                StageRun::ok("diff", Some("model-a".to_string()), 4010),
                StageRun::timed_out("dependencies", None, 360000, "stage_budget"),
                StageRun::failed("spec", None, 10, "output_contract"),
                StageRun::not_run("browser", "applies_when"),
            ],
            findings: vec![finding("rf-1"), hypothesis],
            duration_ms: 500000,
            duplicates_merged: 3,
            scratch_dir: Some(".refact/review_scratch/rv-1".to_string()),
        };

        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["findings"][0]["severity"], "high");
        assert_eq!(value["findings"][1]["severity"], "note");
        assert_eq!(value["stages"][0]["status"], "ok");
        assert_eq!(value["stages"][1]["status"], "timed_out");
        assert_eq!(value["stages"][1]["reason"], "stage_budget");
        assert_eq!(value["stages"][3]["status"], "not_run");
        assert_eq!(value["scope"]["mode"], "strict");
        assert!(value["findings"][0].get("confidence").is_none());
        assert!(value["findings"][0].get("rank_tier").is_none());
        assert!(value["findings"][0].get("verification_status").is_none());
        assert_eq!(
            serde_json::from_value::<ReviewReport>(value).unwrap(),
            report
        );
    }

    #[test]
    fn review_hypotheses_are_findings_without_reproduction_and_without_evidence() {
        let mut quoted_only = finding("rf-quote");
        quoted_only.reproduction = None;
        let mut repro_only = finding("rf-repro");
        repro_only.evidence_present = false;
        let mut neither = finding("rf-none");
        neither.reproduction = None;
        neither.evidence_present = false;
        let mut disputed = finding("rf-disputed");
        disputed.disputed = Some(Dispute {
            stage: "adversarial".to_string(),
            reason: "guarded by an earlier check".to_string(),
        });

        let report = ReviewReport {
            depth: "normal".to_string(),
            scope: ReviewScopeSummary::default(),
            diff: ReviewDiffSummary::default(),
            stages: vec![],
            findings: vec![quoted_only, repro_only, neither, disputed],
            duration_ms: 0,
            duplicates_merged: 0,
            scratch_dir: None,
        };

        let hypotheses: Vec<&str> = report
            .hypotheses()
            .into_iter()
            .map(|finding| finding.id.as_str())
            .collect();
        assert_eq!(hypotheses, ["rf-none", "rf-disputed"]);
        let facts: Vec<&str> = report
            .facts()
            .into_iter()
            .map(|finding| finding.id.as_str())
            .collect();
        assert_eq!(facts, ["rf-quote", "rf-repro"]);
        assert_eq!(report.reproduced(), 1);
    }

    #[test]
    fn review_severity_parses_current_and_legacy_words_and_rejects_junk() {
        assert_eq!(ReviewSeverity::parse("blocker"), Some(ReviewSeverity::Blocker));
        assert_eq!(
            ReviewSeverity::parse(" CRITICAL "),
            Some(ReviewSeverity::Blocker)
        );
        assert_eq!(ReviewSeverity::parse("nit"), Some(ReviewSeverity::Note));
        assert_eq!(ReviewSeverity::parse("showstopper"), None);
        assert!(ReviewSeverity::Blocker.rank() > ReviewSeverity::High.rank());
        assert!(ReviewSeverity::Low.rank() > ReviewSeverity::Note.rank());
    }

    #[test]
    fn review_stage_rows_record_incomplete_stages() {
        let report = ReviewReport {
            depth: "normal".to_string(),
            scope: ReviewScopeSummary::default(),
            diff: ReviewDiffSummary::default(),
            stages: vec![
                StageRun::ok("diff", None, 1),
                StageRun::timed_out("tests", None, 2, "stage_budget"),
                StageRun::not_run("browser", "applies_when"),
            ],
            findings: vec![],
            duration_ms: 0,
            duplicates_merged: 0,
            scratch_dir: None,
        };

        let incomplete: Vec<&str> = report
            .incomplete_stages()
            .into_iter()
            .map(|stage| stage.name.as_str())
            .collect();
        assert_eq!(incomplete, ["tests", "browser"]);
    }

    #[test]
    fn review_depth_parses_known_values_only() {
        assert_eq!(ReviewDepth::parse("normal"), Some(ReviewDepth::Normal));
        assert_eq!(ReviewDepth::parse("DEEP"), Some(ReviewDepth::Deep));
        assert_eq!(ReviewDepth::parse("max"), None);
        assert!(ReviewDepth::Normal < ReviewDepth::Deep);
    }
}
