use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImportArtifact {
    FileContent { destination: PathBuf, content: String },
    DirectoryCopy { source: PathBuf, destination: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportCandidate {
    pub competitor: Competitor,
    pub kind: ImportKind,
    pub scope: ImportScope,
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    pub artifact: ImportArtifact,
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
    pub candidates: Vec<ImportCandidate>,
    pub issues: Vec<ImportIssue>,
    pub status_counts: BTreeMap<ImportStatus, usize>,
}

impl ImportSummary {
    pub fn from_scopes(discovered_scopes: Vec<ImportScope>) -> Self {
        Self {
            discovered_scopes,
            ..Self::default()
        }
    }

    pub fn record_status(&mut self, status: ImportStatus) {
        *self.status_counts.entry(status).or_insert(0) += 1;
    }

    pub fn add_issue(&mut self, issue: ImportIssue) {
        self.record_status(issue.status.clone());
        self.issues.push(issue);
    }

    pub fn merge(&mut self, other: ImportSummary) {
        self.discovered_scopes.extend(other.discovered_scopes);
        self.discovered_sources.extend(other.discovered_sources);
        self.candidates.extend(other.candidates);
        self.issues.extend(other.issues);
        for (status, count) in other.status_counts {
            *self.status_counts.entry(status).or_insert(0) += count;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.discovered_scopes.is_empty()
            && self.discovered_sources.is_empty()
            && self.candidates.is_empty()
            && self.issues.is_empty()
            && self.status_counts.is_empty()
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
}
