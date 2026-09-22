use sha2::{Digest, Sha256};

use refact_buddy_core::operator_types::{
    NativeSourceIdentity, ProjectIdentity, ResourceRef, WorkItem, WorkItemState, WorkItemSeverity,
    OPERATOR_SCHEMA_VERSION_V1,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectKey {
    pub project_id: String,
    pub resource_kind: String,
    pub resource_id: String,
    pub scope_fingerprint: String,
    pub native_revision: String,
}

impl SubjectKey {
    pub fn new(
        project: &ProjectIdentity,
        resource_kind: impl Into<String>,
        resource_id: impl Into<String>,
        scope_fingerprint: impl Into<String>,
        native_revision: impl Into<String>,
    ) -> Result<Self, WorkItemBuildError> {
        let key = Self {
            project_id: project.project_id.trim().to_string(),
            resource_kind: resource_kind.into().trim().to_string(),
            resource_id: resource_id.into().trim().to_string(),
            scope_fingerprint: scope_fingerprint.into().trim().to_string(),
            native_revision: native_revision.into().trim().to_string(),
        };
        if key.project_id.is_empty()
            || key.resource_kind.is_empty()
            || key.resource_id.is_empty()
            || key.scope_fingerprint.is_empty()
            || key.native_revision.is_empty()
        {
            return Err(WorkItemBuildError::MissingSubjectIdentity);
        }
        Ok(key)
    }

    pub fn stable_hash(&self) -> String {
        stable_hash(&[
            &self.project_id,
            &self.resource_kind,
            &self.resource_id,
            &self.scope_fingerprint,
            &self.native_revision,
        ])
    }

    pub fn work_item_id(&self) -> String {
        format!("work:{}", self.stable_hash())
    }

    pub fn artifact_id(&self) -> String {
        format!("artifact:{}", self.stable_hash())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkItemDraft {
    pub project: ProjectIdentity,
    pub subject: SubjectKey,
    pub kind: String,
    pub title: String,
    pub state: WorkItemState,
    pub native_sources: Vec<NativeSourceIdentity>,
    pub last_material_change_at: String,
    pub severity: Option<WorkItemSeverity>,
    pub problem_summary: Option<String>,
    pub scope: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkItemBuildError {
    MissingSubjectIdentity,
    MissingField(&'static str),
}

impl std::fmt::Display for WorkItemBuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSubjectIdentity => formatter.write_str("subject identity is incomplete"),
            Self::MissingField(field) => write!(formatter, "missing work item {field}"),
        }
    }
}

impl std::error::Error for WorkItemBuildError {}

impl WorkItemDraft {
    pub fn build(mut self) -> Result<WorkItem, WorkItemBuildError> {
        if self.project.project_id != self.subject.project_id {
            return Err(WorkItemBuildError::MissingSubjectIdentity);
        }
        self.kind = self.kind.trim().to_string();
        self.title = self.title.trim().to_string();
        self.last_material_change_at = self.last_material_change_at.trim().to_string();
        self.problem_summary = self
            .problem_summary
            .as_deref()
            .and_then(problem_summary_from_title);
        self.scope = self
            .scope
            .as_deref()
            .map(str::trim)
            .filter(|scope| !scope.is_empty())
            .map(ToOwned::to_owned);
        if self.kind.is_empty() {
            return Err(WorkItemBuildError::MissingField("kind"));
        }
        if self.title.is_empty() {
            return Err(WorkItemBuildError::MissingField("title"));
        }
        if self.last_material_change_at.is_empty() {
            return Err(WorkItemBuildError::MissingField("last_material_change_at"));
        }
        self.native_sources.sort_by(|left, right| {
            (&left.source_id, &left.source_version).cmp(&(&right.source_id, &right.source_version))
        });
        self.native_sources.dedup();
        Ok(WorkItem {
            schema_version: OPERATOR_SCHEMA_VERSION_V1,
            id: self.subject.work_item_id(),
            project: self.project,
            subject: ResourceRef {
                resource_kind: self.subject.resource_kind,
                resource_id: self.subject.resource_id,
                version: self.subject.native_revision,
            },
            kind: self.kind,
            title: self.title,
            state: self.state,
            native_sources: self.native_sources,
            current_artifact_id: None,
            current_artifact_revision: None,
            last_material_change_at: self.last_material_change_at,
            severity: self.severity,
            problem_summary: self.problem_summary,
            scope: self.scope,
        })
    }
}

pub fn problem_summary_from_title(title: &str) -> Option<String> {
    let summary = title
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ");
    (!summary.is_empty()).then_some(summary)
}

pub fn is_cosmetic_helper_outcome(
    outcome_work_id: &str,
    parent_work_id: Option<&str>,
) -> Option<String> {
    let parent = parent_work_id?.trim();
    (!parent.is_empty() && outcome_work_id.starts_with("helper:")).then(|| parent.to_string())
}

pub(crate) fn stable_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_buddy_core::operator_types::{NativeSourceKind, ProjectIdentity};

    fn project() -> ProjectIdentity {
        ProjectIdentity {
            project_id: "project".to_string(),
            canonical_root: "/tmp/project".to_string(),
        }
    }

    #[test]
    fn subject_identity_changes_for_scope_or_native_revision() {
        let base = SubjectKey::new(&project(), "check", "lint", "src", "main@a").unwrap();
        let narrower = SubjectKey::new(&project(), "check", "lint", "src/lib", "main@a").unwrap();
        let branch = SubjectKey::new(&project(), "check", "lint", "src", "feature@a").unwrap();
        assert_ne!(base.work_item_id(), narrower.work_item_id());
        assert_ne!(base.artifact_id(), branch.artifact_id());
    }

    #[test]
    fn draft_uses_deterministic_subject_and_keeps_source_identity() {
        let subject = SubjectKey::new(&project(), "check", "cargo", "engine", "main@a").unwrap();
        let item = WorkItemDraft {
            project: project(),
            subject: subject.clone(),
            kind: "check".to_string(),
            title: "Cargo check".to_string(),
            state: WorkItemState::NeedsAttention,
            native_sources: vec![NativeSourceIdentity {
                source_kind: NativeSourceKind::Check,
                source_id: "cargo".to_string(),
                source_version: "main@a".to_string(),
            }],
            last_material_change_at: "2026-09-05T00:00:00Z".to_string(),
            severity: Some(WorkItemSeverity::Unknown),
            problem_summary: problem_summary_from_title("Cargo check"),
            scope: Some("engine".to_string()),
        }
        .build()
        .unwrap();
        assert_eq!(item.id, subject.work_item_id());
        assert_eq!(item.subject.version, "main@a");
    }

    #[test]
    fn rows_expose_distinct_summaries() {
        let summaries = [
            ("provider", "Repair provider authentication"),
            ("workspace", "Review uncommitted workspace changes"),
            ("instructions", "Update missing AGENTS instructions"),
            ("modes", "Resolve overlapping mode prompts"),
            ("dependencies", "Check dependency manifest changes"),
        ]
        .into_iter()
        .map(|(kind, title)| {
            WorkItemDraft {
                project: project(),
                subject: SubjectKey::new(&project(), kind, kind, kind, "main@a").unwrap(),
                kind: kind.to_string(),
                title: title.to_string(),
                state: WorkItemState::NeedsAttention,
                native_sources: vec![],
                last_material_change_at: "2026-09-05T00:00:00Z".to_string(),
                severity: Some(WorkItemSeverity::Unknown),
                problem_summary: problem_summary_from_title(title),
                scope: None,
            }
            .build()
            .unwrap()
            .problem_summary
        })
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(summaries.len(), 5);
    }

    #[test]
    fn missing_severity_or_scope_is_representable() {
        let item = WorkItemDraft {
            project: project(),
            subject: SubjectKey::new(&project(), "check", "lint", "src", "main@a").unwrap(),
            kind: "check".to_string(),
            title: "Lint source".to_string(),
            state: WorkItemState::NeedsAttention,
            native_sources: vec![],
            last_material_change_at: "2026-09-05T00:00:00Z".to_string(),
            severity: None,
            problem_summary: None,
            scope: None,
        }
        .build()
        .unwrap();
        let wire = serde_json::to_string(&item).unwrap();
        assert!(!wire.contains("severity"));
        let round_trip: WorkItem = serde_json::from_str(&wire).unwrap();
        assert_eq!(round_trip.severity, None);
        assert_eq!(round_trip.scope, None);
    }
}
