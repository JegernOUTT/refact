use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Competitor {
    ClaudeCode,
    OpenCode,
    KiloCode,
    ContinueDev,
}

impl Competitor {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::OpenCode => "opencode",
            Self::KiloCode => "kilo_code",
            Self::ContinueDev => "continue_dev",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind {
    Skill,
    Command,
    Subagent,
    UnsupportedRules,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImportScope {
    Global,
    Project { root: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ImportSourceRoot {
    pub competitor: Competitor,
    pub scope: ImportScope,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionContext {
    pub competitor: Competitor,
    pub scope: ImportScope,
    pub source_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionError {
    pub competitor: Competitor,
    pub kind: ImportKind,
    pub scope: ImportScope,
    pub path: PathBuf,
    pub message: String,
}

impl ConversionError {
    pub fn new(
        context: &ConversionContext,
        kind: ImportKind,
        path: PathBuf,
        message: impl Into<String>,
    ) -> Self {
        Self {
            competitor: context.competitor,
            kind,
            scope: context.scope.clone(),
            path,
            message: message.into(),
        }
    }

    pub fn into_issue(self) -> ImportIssue {
        ImportIssue {
            competitor: Some(self.competitor),
            kind: Some(self.kind),
            scope: Some(self.scope),
            path: Some(self.path),
            status: ImportStatus::Error,
            message: self.message,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolPolicy {
    pub allowed: Option<Vec<String>>,
    pub denied: Vec<String>,
}

impl ToolPolicy {
    pub fn missing() -> Self {
        Self {
            allowed: None,
            denied: Vec::new(),
        }
    }

    pub fn allow(allowed: Vec<String>) -> Self {
        Self {
            allowed: Some(allowed),
            denied: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedSubagent {
    pub id: String,
    pub title: String,
    pub description: String,
    pub prompt: String,
    pub tool_policy: ToolPolicy,
    pub max_steps: Option<usize>,
    pub model: Option<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportArtifact {
    FileContent { content: String },
    DirectoryCopy { source_dir: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCandidate {
    pub competitor: Competitor,
    pub kind: ImportKind,
    pub scope: ImportScope,
    pub source_root: PathBuf,
    pub source_path: PathBuf,
    pub dest_name: String,
    pub destination_path: PathBuf,
    pub artifact: ImportArtifact,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportCandidateSummary {
    pub competitor: Competitor,
    pub kind: ImportKind,
    pub scope: ImportScope,
    pub source_root: PathBuf,
    pub source_path: PathBuf,
    pub dest_name: String,
    pub destination_path: PathBuf,
    #[serde(default, skip_serializing)]
    pub metadata: Value,
}

impl From<&ImportCandidate> for ImportCandidateSummary {
    fn from(candidate: &ImportCandidate) -> Self {
        Self {
            competitor: candidate.competitor,
            kind: candidate.kind,
            scope: candidate.scope.clone(),
            source_root: candidate.source_root.clone(),
            source_path: candidate.source_path.clone(),
            dest_name: candidate.dest_name.clone(),
            destination_path: candidate.destination_path.clone(),
            metadata: candidate.metadata.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    Created,
    Updated,
    Unchanged,
    Conflict,
    UserModified,
    Unsupported,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportOutcome {
    pub candidate: ImportCandidateSummary,
    pub status: ImportStatus,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportIssue {
    pub competitor: Option<Competitor>,
    pub kind: Option<ImportKind>,
    pub scope: Option<ImportScope>,
    pub path: Option<PathBuf>,
    pub status: ImportStatus,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportReportCounts {
    pub discovered: usize,
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub conflicts: usize,
    pub user_modified: usize,
    pub unsupported: usize,
    pub errors: usize,
}

impl ImportReportCounts {
    fn add_status(&mut self, status: &ImportStatus) {
        match status {
            ImportStatus::Created => self.created += 1,
            ImportStatus::Updated => self.updated += 1,
            ImportStatus::Unchanged => self.unchanged += 1,
            ImportStatus::Conflict => self.conflicts += 1,
            ImportStatus::UserModified => self.user_modified += 1,
            ImportStatus::Unsupported => self.unsupported += 1,
            ImportStatus::Error => self.errors += 1,
        }
    }

    pub fn attention_items(&self) -> usize {
        self.conflicts + self.user_modified + self.errors
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReportIssue {
    pub competitor: Option<Competitor>,
    pub kind: Option<ImportKind>,
    pub scope: Option<ImportScope>,
    pub path: Option<PathBuf>,
    pub status: ImportStatus,
    pub message: String,
}

impl ImportReportIssue {
    fn from_issue(issue: &ImportIssue) -> Self {
        Self {
            competitor: issue.competitor,
            kind: issue.kind,
            scope: issue.scope.clone(),
            path: issue.path.clone(),
            status: issue.status.clone(),
            message: sanitize_report_message(&issue.message),
        }
    }

    fn from_outcome(outcome: &ImportOutcome) -> Self {
        Self {
            competitor: Some(outcome.candidate.competitor),
            kind: Some(outcome.candidate.kind),
            scope: Some(outcome.candidate.scope.clone()),
            path: Some(outcome.candidate.destination_path.clone()),
            status: outcome.status.clone(),
            message: sanitize_report_message(&outcome.message),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    pub discovered_scopes: Vec<ImportScope>,
    pub discovered_sources: Vec<ImportSourceRoot>,
    pub discovered_candidates: usize,
    pub status_counts: BTreeMap<ImportStatus, usize>,
    pub competitor_counts: BTreeMap<Competitor, ImportReportCounts>,
    pub kind_counts: BTreeMap<ImportKind, ImportReportCounts>,
    pub top_issues: Vec<ImportReportIssue>,
}

impl ImportReport {
    pub fn from_summary(summary: &ImportSummary) -> Self {
        Self::from_summary_with_scope(summary, None)
    }

    pub fn from_summary_for_scope(summary: &ImportSummary, scope: &ImportScope) -> Self {
        Self::from_summary_with_scope(summary, Some(scope))
    }

    pub fn status_count(&self, status: &ImportStatus) -> usize {
        self.status_counts.get(status).copied().unwrap_or(0)
    }

    pub fn attention_items(&self) -> usize {
        self.status_count(&ImportStatus::Conflict)
            + self.status_count(&ImportStatus::UserModified)
            + self.status_count(&ImportStatus::Error)
    }

    fn from_summary_with_scope(summary: &ImportSummary, scope: Option<&ImportScope>) -> Self {
        let mut report = Self {
            generated_at: summary.generated_at.clone(),
            completed_at: summary.completed_at.clone(),
            discovered_scopes: match scope {
                Some(scope) => vec![scope.clone()],
                None => summary.discovered_scopes.clone(),
            },
            discovered_sources: summary
                .discovered_sources
                .iter()
                .filter(|source| scope.map_or(true, |scope| &source.scope == scope))
                .cloned()
                .collect(),
            ..Self::default()
        };

        let candidates = summary
            .candidates
            .iter()
            .filter(|candidate| scope.map_or(true, |scope| &candidate.scope == scope))
            .collect::<Vec<_>>();
        let outcomes = summary
            .outcomes
            .iter()
            .filter(|outcome| scope.map_or(true, |scope| &outcome.candidate.scope == scope))
            .collect::<Vec<_>>();
        let issues = summary
            .issues
            .iter()
            .filter(|issue| match scope {
                Some(scope) => issue.scope.as_ref() == Some(scope),
                None => true,
            })
            .collect::<Vec<_>>();

        report.discovered_candidates = candidates.len();
        for candidate in candidates {
            report.add_discovered(candidate.competitor, candidate.kind);
        }
        for outcome in &outcomes {
            report.add_status(
                Some(outcome.candidate.competitor),
                Some(outcome.candidate.kind),
                &outcome.status,
            );
        }
        for issue in &issues {
            if issue_matches_outcome_refs(issue, &outcomes) {
                continue;
            }
            report.add_status(issue.competitor, issue.kind, &issue.status);
        }
        if report.status_counts.is_empty() && scope.is_none() {
            report.status_counts = summary.status_counts.clone();
        }
        report.top_issues = collect_top_issues(&outcomes, &issues);
        report
    }

    fn add_discovered(&mut self, competitor: Competitor, kind: ImportKind) {
        self.competitor_counts
            .entry(competitor)
            .or_default()
            .discovered += 1;
        self.kind_counts.entry(kind).or_default().discovered += 1;
    }

    fn add_status(
        &mut self,
        competitor: Option<Competitor>,
        kind: Option<ImportKind>,
        status: &ImportStatus,
    ) {
        *self.status_counts.entry(status.clone()).or_insert(0) += 1;
        if let Some(competitor) = competitor {
            self.competitor_counts
                .entry(competitor)
                .or_default()
                .add_status(status);
        }
        if let Some(kind) = kind {
            self.kind_counts.entry(kind).or_default().add_status(status);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    pub discovered_scopes: Vec<ImportScope>,
    pub discovered_sources: Vec<ImportSourceRoot>,
    pub candidates: Vec<ImportCandidateSummary>,
    pub outcomes: Vec<ImportOutcome>,
    pub issues: Vec<ImportIssue>,
    pub errors: Vec<ImportIssue>,
    pub status_counts: BTreeMap<ImportStatus, usize>,
}

impl ImportSummary {
    pub fn from_scopes(discovered_scopes: Vec<ImportScope>) -> Self {
        let mut summary = Self {
            discovered_scopes,
            ..Self::default()
        };
        summary.mark_generated();
        summary
    }

    pub fn mark_generated(&mut self) {
        if self.generated_at.is_none() {
            self.generated_at = Some(Utc::now().to_rfc3339());
        }
    }

    pub fn mark_completed(&mut self) {
        self.mark_generated();
        self.completed_at = Some(Utc::now().to_rfc3339());
    }

    pub fn record_candidate(&mut self, candidate: &ImportCandidate) {
        self.mark_generated();
        self.candidates
            .push(ImportCandidateSummary::from(candidate));
    }

    pub fn record_status(&mut self, status: ImportStatus) {
        self.mark_generated();
        *self.status_counts.entry(status).or_insert(0) += 1;
    }

    pub fn add_outcome(&mut self, outcome: ImportOutcome) {
        self.record_status(outcome.status.clone());
        self.outcomes.push(outcome);
    }

    pub fn add_issue(&mut self, issue: ImportIssue) {
        self.record_status(issue.status.clone());
        if issue.status == ImportStatus::Error {
            self.errors.push(issue.clone());
        }
        self.issues.push(issue);
    }

    pub fn merge(&mut self, other: ImportSummary) {
        if self.generated_at.is_none() {
            self.generated_at = other.generated_at.clone();
        }
        if let Some(completed_at) = &other.completed_at {
            if self
                .completed_at
                .as_ref()
                .map(|existing| existing < completed_at)
                .unwrap_or(true)
            {
                self.completed_at = Some(completed_at.clone());
            }
        }
        self.discovered_scopes.extend(other.discovered_scopes);
        self.discovered_sources.extend(other.discovered_sources);
        self.candidates.extend(other.candidates);
        self.outcomes.extend(other.outcomes);
        self.issues.extend(other.issues);
        self.errors.extend(other.errors);
        for (status, count) in other.status_counts {
            *self.status_counts.entry(status).or_insert(0) += count;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.discovered_scopes.is_empty()
            && self.discovered_sources.is_empty()
            && self.candidates.is_empty()
            && self.outcomes.is_empty()
            && self.issues.is_empty()
            && self.errors.is_empty()
            && self.status_counts.is_empty()
    }

    pub fn has_created_or_updated(&self, kinds: &[ImportKind]) -> bool {
        self.outcomes.iter().any(|outcome| {
            matches!(
                outcome.status,
                ImportStatus::Created | ImportStatus::Updated
            ) && kinds.contains(&outcome.candidate.kind)
        })
    }

    pub fn has_command_or_skill_changes(&self) -> bool {
        self.has_created_or_updated(&[ImportKind::Command, ImportKind::Skill])
    }

    pub fn has_subagent_changes(&self) -> bool {
        self.has_created_or_updated(&[ImportKind::Subagent])
    }

    pub fn has_imported_changes(&self) -> bool {
        self.has_command_or_skill_changes() || self.has_subagent_changes()
    }
}

fn collect_top_issues(
    outcomes: &[&ImportOutcome],
    issues: &[&ImportIssue],
) -> Vec<ImportReportIssue> {
    let mut top_issues = Vec::new();
    for outcome in outcomes {
        if is_top_issue_status(&outcome.status) {
            top_issues.push(ImportReportIssue::from_outcome(outcome));
        }
    }
    for issue in issues {
        if is_top_issue_status(&issue.status) && !issue_matches_outcome_refs(issue, outcomes) {
            top_issues.push(ImportReportIssue::from_issue(issue));
        }
    }
    top_issues.truncate(10);
    top_issues
}

fn is_top_issue_status(status: &ImportStatus) -> bool {
    matches!(
        status,
        ImportStatus::Conflict
            | ImportStatus::UserModified
            | ImportStatus::Unsupported
            | ImportStatus::Error
    )
}

fn issue_matches_outcome_refs(issue: &ImportIssue, outcomes: &[&ImportOutcome]) -> bool {
    outcomes.iter().any(|outcome| {
        issue.status == outcome.status
            && issue.kind == Some(outcome.candidate.kind)
            && issue.scope.as_ref() == Some(&outcome.candidate.scope)
            && issue.path.as_ref() == Some(&outcome.candidate.destination_path)
    })
}

fn sanitize_report_message(message: &str) -> String {
    let compact = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= 240 {
        return compact;
    }
    let mut truncated = compact.chars().take(240).collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate_summary() -> ImportCandidateSummary {
        ImportCandidateSummary {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Command,
            scope: ImportScope::Global,
            source_root: PathBuf::from("/source"),
            source_path: PathBuf::from("/source/review.md"),
            dest_name: "review".to_string(),
            destination_path: PathBuf::from("/dest/review.md"),
            metadata: Value::Null,
        }
    }

    #[test]
    fn summary_merge_combines_counts_and_items() {
        let mut left = ImportSummary::from_scopes(vec![ImportScope::Global]);
        left.record_status(ImportStatus::Created);
        left.record_status(ImportStatus::Unchanged);

        let mut right = ImportSummary::from_scopes(vec![ImportScope::Project {
            root: PathBuf::from("/repo"),
        }]);
        right.record_status(ImportStatus::Created);
        right.add_issue(ImportIssue {
            competitor: Some(Competitor::ClaudeCode),
            kind: Some(ImportKind::UnsupportedRules),
            scope: Some(ImportScope::Global),
            path: Some(PathBuf::from("/home/user/.claude/CLAUDE.md")),
            status: ImportStatus::Unsupported,
            message: "rules are report-only in v1".to_string(),
        });

        left.merge(right);

        assert_eq!(left.discovered_scopes.len(), 2);
        assert_eq!(left.issues.len(), 1);
        assert_eq!(left.status_counts.get(&ImportStatus::Created), Some(&2));
        assert_eq!(left.status_counts.get(&ImportStatus::Unchanged), Some(&1));
        assert_eq!(left.status_counts.get(&ImportStatus::Unsupported), Some(&1));
        assert!(left.generated_at.is_some());
    }

    #[test]
    fn summary_change_flags_only_track_created_or_updated_outputs() {
        let candidate = candidate_summary();
        let mut summary = ImportSummary::default();
        summary.add_outcome(ImportOutcome {
            candidate: candidate.clone(),
            status: ImportStatus::Unchanged,
            message: "unchanged".to_string(),
        });
        assert!(!summary.has_imported_changes());

        summary.add_outcome(ImportOutcome {
            candidate,
            status: ImportStatus::Updated,
            message: "updated".to_string(),
        });

        assert!(summary.has_command_or_skill_changes());
        assert!(!summary.has_subagent_changes());
        assert!(summary.has_imported_changes());
    }

    #[test]
    fn summary_serialization_omits_artifact_content() {
        let candidate = ImportCandidate {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Command,
            scope: ImportScope::Global,
            source_root: PathBuf::from("/source"),
            source_path: PathBuf::from("/source/secret.md"),
            dest_name: "secret".to_string(),
            destination_path: PathBuf::from("/dest/secret.md"),
            artifact: ImportArtifact::FileContent {
                content: "secret artifact content".to_string(),
            },
            metadata: serde_json::json!({"original_name": "secret"}),
        };
        let mut summary = ImportSummary::default();
        summary.record_candidate(&candidate);

        let json = serde_json::to_string(&summary).unwrap();

        assert!(!json.contains("secret artifact content"));
        assert!(json.contains("secret.md"));
    }

    #[test]
    fn report_serialization_has_timestamps_counts_and_no_artifact_content() {
        let candidate = ImportCandidate {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Command,
            scope: ImportScope::Global,
            source_root: PathBuf::from("/source"),
            source_path: PathBuf::from("/source/secret.md"),
            dest_name: "secret".to_string(),
            destination_path: PathBuf::from("/dest/secret.md"),
            artifact: ImportArtifact::FileContent {
                content: "secret artifact content".to_string(),
            },
            metadata: serde_json::json!({"original_description": "secret artifact content"}),
        };
        let mut summary = ImportSummary::from_scopes(vec![ImportScope::Global]);
        summary.record_candidate(&candidate);
        summary.add_outcome(ImportOutcome {
            candidate: ImportCandidateSummary::from(&candidate),
            status: ImportStatus::Created,
            message: "created generated destination".to_string(),
        });
        summary.mark_completed();

        let report = ImportReport::from_summary(&summary);
        let json = serde_json::to_string(&report).unwrap();

        assert!(report.generated_at.is_some());
        assert!(report.completed_at.is_some());
        assert_eq!(report.discovered_candidates, 1);
        assert_eq!(report.status_counts.get(&ImportStatus::Created), Some(&1));
        assert_eq!(report.competitor_counts[&Competitor::ClaudeCode].created, 1);
        assert_eq!(report.kind_counts[&ImportKind::Command].created, 1);
        assert!(!json.contains("secret artifact content"));
        assert!(!json.contains("original_description"));
    }

    #[test]
    fn report_keeps_top_attention_issues_without_duplicate_writer_errors() {
        let candidate = candidate_summary();
        let mut summary = ImportSummary::default();
        summary.add_outcome(ImportOutcome {
            candidate: candidate.clone(),
            status: ImportStatus::Error,
            message: "write failed".to_string(),
        });
        summary.issues.push(ImportIssue {
            competitor: Some(candidate.competitor),
            kind: Some(candidate.kind),
            scope: Some(candidate.scope.clone()),
            path: Some(candidate.destination_path.clone()),
            status: ImportStatus::Error,
            message: "write failed".to_string(),
        });

        let report = ImportReport::from_summary(&summary);

        assert_eq!(report.status_count(&ImportStatus::Error), 1);
        assert_eq!(report.top_issues.len(), 1);
    }
}
