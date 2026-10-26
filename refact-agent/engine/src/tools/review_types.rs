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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_chat_id: Option<String>,
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
            trace_chat_id: None,
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

    pub fn with_trace_chat_id(mut self, trace_chat_id: Option<String>) -> Self {
        self.trace_chat_id = trace_chat_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self
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
    #[serde(default)]
    pub dropped_files: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewOutcome {
    Reviewed,
    Partial,
    Inconclusive,
}

impl Default for ReviewOutcome {
    fn default() -> Self {
        Self::Inconclusive
    }
}

impl ReviewOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Reviewed => "reviewed",
            Self::Partial => "partial",
            Self::Inconclusive => "inconclusive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReviewReport {
    pub depth: String,
    #[serde(default)]
    pub outcome: ReviewOutcome,
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

    pub fn completed_stages(&self) -> usize {
        self.stages
            .iter()
            .filter(|stage| stage.is_complete())
            .count()
    }

    pub fn derive_outcome(&self) -> ReviewOutcome {
        let attempted: Vec<&StageRun> = self
            .stages
            .iter()
            .filter(|stage| Self::counts_for_outcome(stage))
            .collect();
        let completed = attempted.iter().filter(|stage| stage.is_complete()).count();
        if completed == 0 {
            return ReviewOutcome::Inconclusive;
        }
        if completed < attempted.len() {
            return ReviewOutcome::Partial;
        }
        ReviewOutcome::Reviewed
    }

    fn counts_for_outcome(stage: &StageRun) -> bool {
        match stage.status {
            StageStatusKind::NotRun => stage
                .reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with("review cancelled")),
            _ => true,
        }
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

    fn report_with_stages(stages: Vec<StageRun>) -> ReviewReport {
        ReviewReport {
            depth: "normal".to_string(),
            outcome: ReviewOutcome::Inconclusive,
            scope: ReviewScopeSummary::default(),
            diff: ReviewDiffSummary::default(),
            stages,
            findings: vec![],
            duration_ms: 0,
            duplicates_merged: 0,
            scratch_dir: None,
        }
    }

    #[test]
    fn review_outcome_is_inconclusive_when_no_stage_reached_ok() {
        let all_dead = report_with_stages(vec![
            StageRun::timed_out("diff", None, 1, "stage_budget"),
            StageRun::failed("spec", None, 2, "output_contract"),
            StageRun::not_run("browser", "applies_when"),
        ]);
        let partial = report_with_stages(vec![
            StageRun::ok("diff", None, 1),
            StageRun::timed_out("spec", None, 2, "stage_budget"),
        ]);
        let reviewed = report_with_stages(vec![
            StageRun::ok("diff", None, 1),
            StageRun::ok("spec", None, 2),
        ]);

        assert_eq!(all_dead.derive_outcome(), ReviewOutcome::Inconclusive);
        assert_eq!(partial.derive_outcome(), ReviewOutcome::Partial);
        assert_eq!(reviewed.derive_outcome(), ReviewOutcome::Reviewed);
    }

    #[test]
    fn review_outcome_ignores_deliberate_skips_but_counts_starved_stages() {
        let default_depth_run = report_with_stages(vec![
            StageRun::ok("diff", None, 1),
            StageRun::ok("spec", None, 2),
            StageRun::not_run("tests", "depth normal"),
            StageRun::not_run("concurrency", "opt-in stage"),
        ]);
        let cancelled = report_with_stages(vec![
            StageRun::ok("diff", None, 1),
            StageRun::not_run("spec", "review cancelled before the stage started"),
        ]);

        assert_eq!(default_depth_run.derive_outcome(), ReviewOutcome::Reviewed);
        assert_eq!(cancelled.derive_outcome(), ReviewOutcome::Partial);
        assert_eq!(
            report_with_stages(vec![]).derive_outcome(),
            ReviewOutcome::Inconclusive
        );
    }

    #[test]
    fn review_outcome_defaults_to_inconclusive_for_payloads_without_the_field() {
        let value = serde_json::json!({
            "depth": "normal",
            "scope": {"mode": "broad", "requested_files": 0, "reviewed_files": 0},
            "diff": {"changed_files": 0, "hunks": 0},
            "stages": [],
            "findings": [],
            "duration_ms": 0
        });

        let report: ReviewReport = serde_json::from_value(value).unwrap();

        assert_eq!(report.outcome, ReviewOutcome::Inconclusive);
        assert!(report.scope.dropped_files.is_empty());
    }
}
