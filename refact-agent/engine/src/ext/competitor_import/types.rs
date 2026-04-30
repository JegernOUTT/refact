use std::collections::BTreeMap;
use std::path::PathBuf;

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
pub struct ImportSummary {
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
        Self {
            discovered_scopes,
            ..Self::default()
        }
    }

    pub fn record_candidate(&mut self, candidate: &ImportCandidate) {
        self.candidates
            .push(ImportCandidateSummary::from(candidate));
    }

    pub fn record_status(&mut self, status: ImportStatus) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn summary_change_flags_only_track_created_or_updated_outputs() {
        let candidate = ImportCandidateSummary {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Command,
            scope: ImportScope::Global,
            source_root: PathBuf::from("/source"),
            source_path: PathBuf::from("/source/review.md"),
            dest_name: "review".to_string(),
            destination_path: PathBuf::from("/dest/review.md"),
            metadata: Value::Null,
        };
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
}
