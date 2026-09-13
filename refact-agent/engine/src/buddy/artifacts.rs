use refact_buddy_core::operator_types::{
    ArtifactProvenance, ArtifactRevision, EvidenceRef, NativeSourceKind, RunOutcome,
    OPERATOR_SCHEMA_VERSION_V1,
};

use super::work_items::{stable_hash, SubjectKey};

pub const MAX_ARTIFACT_EVIDENCE: usize = 12;
pub const MAX_ARTIFACT_TEXT_BYTES: usize = 32_768;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactDraft {
    pub subject: SubjectKey,
    pub source_run_ids: Vec<String>,
    pub title: String,
    pub conclusion: String,
    pub delta: String,
    pub body: String,
    pub uncertainty: Option<String>,
    pub proposed_action_ids: Vec<String>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactBuildError {
    MissingField(&'static str),
    EvidenceLimitExceeded,
    EvidenceDoesNotBelongToRun,
    PrivacyGuard(&'static str),
}

impl std::fmt::Display for ArtifactBuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(field) => write!(formatter, "missing artifact {field}"),
            Self::EvidenceLimitExceeded => formatter.write_str("artifact evidence limit exceeded"),
            Self::EvidenceDoesNotBelongToRun => {
                formatter.write_str("artifact evidence is not present in source runs")
            }
            Self::PrivacyGuard(field) => {
                write!(formatter, "artifact {field} violates privacy guard")
            }
        }
    }
}

impl std::error::Error for ArtifactBuildError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactMaterialization {
    Folded { artifact_id: String, revision: u32 },
    Revision(ArtifactRevision),
}

impl ArtifactDraft {
    pub fn content_fingerprint(&self) -> String {
        let action_ids = canonical_ids(&self.proposed_action_ids);
        let evidence = canonical_evidence_identities(&self.evidence);
        stable_hash(&[
            &self.subject.stable_hash(),
            self.title.trim(),
            self.conclusion.trim(),
            self.delta.trim(),
            self.body.trim(),
            self.uncertainty.as_deref().map(str::trim).unwrap_or(""),
            &action_ids.join("\u{1f}"),
            &evidence.join("\u{1f}"),
        ])
    }

    pub fn materialize(
        mut self,
        source_runs: &[RunOutcome],
        current: Option<&ArtifactRevision>,
    ) -> Result<ArtifactMaterialization, ArtifactBuildError> {
        self.normalize_and_validate(source_runs)?;
        let artifact_id = self.subject.artifact_id();
        if let Some(current) = current {
            if current.artifact_id == artifact_id
                && artifact_content_fingerprint(current) == self.content_fingerprint()
            {
                return Ok(ArtifactMaterialization::Folded {
                    artifact_id,
                    revision: current.revision,
                });
            }
        }
        let revision = current
            .filter(|value| value.artifact_id == artifact_id)
            .map_or(1, |value| value.revision.saturating_add(1));
        let evidence_refs = self
            .evidence
            .iter()
            .map(|item| item.evidence_id.clone())
            .collect();
        let contains_untrusted_source = self
            .evidence
            .iter()
            .any(|item| matches!(item.source.source_kind, NativeSourceKind::External));
        Ok(ArtifactMaterialization::Revision(ArtifactRevision {
            schema_version: OPERATOR_SCHEMA_VERSION_V1,
            artifact_id,
            revision,
            project: source_runs
                .first()
                .ok_or(ArtifactBuildError::MissingField("source_runs"))?
                .project
                .clone(),
            source_run_ids: self.source_run_ids.clone(),
            title: self.title,
            conclusion: self.conclusion,
            delta: self.delta,
            body: self.body,
            uncertainty: self.uncertainty,
            proposed_action_ids: self.proposed_action_ids,
            evidence: self.evidence,
            provenance: ArtifactProvenance {
                source_run_ids: self.source_run_ids,
                evidence_refs,
                contains_untrusted_source,
            },
        }))
    }

    fn normalize_and_validate(
        &mut self,
        source_runs: &[RunOutcome],
    ) -> Result<(), ArtifactBuildError> {
        self.title = guarded_text(&self.title, "title")?;
        self.conclusion = guarded_text(&self.conclusion, "conclusion")?;
        self.delta = guarded_text(&self.delta, "delta")?;
        self.body = guarded_text(&self.body, "body")?;
        self.uncertainty = self
            .uncertainty
            .as_deref()
            .map(|value| guarded_text(value, "uncertainty"))
            .transpose()?;
        if self.title.is_empty() {
            return Err(ArtifactBuildError::MissingField("title"));
        }
        if self.conclusion.is_empty() {
            return Err(ArtifactBuildError::MissingField("conclusion"));
        }
        self.source_run_ids = canonical_ids(&self.source_run_ids);
        self.proposed_action_ids = canonical_ids(&self.proposed_action_ids);
        if self.source_run_ids.is_empty() {
            return Err(ArtifactBuildError::MissingField("source_run_ids"));
        }
        if self.evidence.len() > MAX_ARTIFACT_EVIDENCE {
            return Err(ArtifactBuildError::EvidenceLimitExceeded);
        }
        self.evidence
            .sort_by(|left, right| left.evidence_id.cmp(&right.evidence_id));
        self.evidence
            .dedup_by(|left, right| left.evidence_id == right.evidence_id);
        let mut source_evidence = Vec::new();
        for run in source_runs {
            if run.project.project_id != self.subject.project_id {
                return Err(ArtifactBuildError::EvidenceDoesNotBelongToRun);
            }
            if self.source_run_ids.contains(&run.run_id) {
                source_evidence.extend(run.evidence.iter());
            }
        }
        if !self
            .source_run_ids
            .iter()
            .all(|id| source_runs.iter().any(|run| &run.run_id == id))
            || !self
                .evidence
                .iter()
                .all(|item| source_evidence.iter().any(|original| *original == item))
        {
            return Err(ArtifactBuildError::EvidenceDoesNotBelongToRun);
        }
        Ok(())
    }
}

fn canonical_ids(values: &[String]) -> Vec<String> {
    let mut values = values
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn guarded_text(value: &str, field: &'static str) -> Result<String, ArtifactBuildError> {
    let value = value.trim();
    if value.len() > MAX_ARTIFACT_TEXT_BYTES || value.contains('\0') {
        return Err(ArtifactBuildError::PrivacyGuard(field));
    }
    Ok(value.to_string())
}

fn canonical_evidence_identities(evidence: &[EvidenceRef]) -> Vec<String> {
    let mut identities = evidence
        .iter()
        .map(|item| {
            stable_hash(&[
                &format!("{:?}", item.source.source_kind),
                item.source.source_id.trim(),
                item.source.source_version.trim(),
                item.locator.trim(),
                item.digest.as_deref().map(str::trim).unwrap_or(""),
            ])
        })
        .collect::<Vec<_>>();
    identities.sort();
    identities.dedup();
    identities
}

fn artifact_content_fingerprint(artifact: &ArtifactRevision) -> String {
    let subject_hash = artifact
        .artifact_id
        .strip_prefix("artifact:")
        .unwrap_or(&artifact.artifact_id);
    let action_ids = canonical_ids(&artifact.proposed_action_ids);
    let evidence = canonical_evidence_identities(&artifact.evidence);
    stable_hash(&[
        subject_hash,
        artifact.title.trim(),
        artifact.conclusion.trim(),
        artifact.delta.trim(),
        artifact.body.trim(),
        artifact.uncertainty.as_deref().map(str::trim).unwrap_or(""),
        &action_ids.join("\u{1f}"),
        &evidence.join("\u{1f}"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_buddy_core::operator_types::{
        EvidenceRef, NativeSourceIdentity, ProjectIdentity, RunStatus, WorkerIdentity,
    };

    fn project() -> ProjectIdentity {
        ProjectIdentity {
            project_id: "project".to_string(),
            canonical_root: "/tmp/project".to_string(),
        }
    }
    fn source() -> NativeSourceIdentity {
        NativeSourceIdentity {
            source_kind: NativeSourceKind::Check,
            source_id: "cargo".to_string(),
            source_version: "main@a".to_string(),
        }
    }
    fn run() -> RunOutcome {
        RunOutcome {
            schema_version: 1,
            run_id: "run-1".to_string(),
            work_id: "work".to_string(),
            parent_run_id: None,
            project: project(),
            worker: WorkerIdentity {
                worker_id: "worker".to_string(),
                engine_instance_id: "engine".to_string(),
                epoch: 1,
            },
            origin: "test".to_string(),
            workflow: None,
            input_fingerprint: "input".to_string(),
            coverage: "src".to_string(),
            started_at: "t1".to_string(),
            ended_at: Some("t2".to_string()),
            status: RunStatus::Completed,
            evidence: vec![EvidenceRef {
                evidence_id: "e1".to_string(),
                source: source(),
                locator: "check://cargo".to_string(),
                observed_at: "t2".to_string(),
                digest: Some("d".to_string()),
            }],
            effects: vec![],
            failure_class: None,
        }
    }
    fn draft() -> ArtifactDraft {
        ArtifactDraft {
            subject: SubjectKey::new(&project(), "check", "cargo", "src", "main@a").unwrap(),
            source_run_ids: vec!["run-1".to_string()],
            title: "Cargo".to_string(),
            conclusion: "failed".to_string(),
            delta: "one error".to_string(),
            body: "raw evidence retained".to_string(),
            uncertainty: None,
            proposed_action_ids: vec![],
            evidence: run().evidence,
        }
    }

    #[test]
    fn identical_material_content_folds_despite_run_timestamps() {
        let first = match draft().materialize(&[run()], None).unwrap() {
            ArtifactMaterialization::Revision(value) => value,
            _ => panic!(),
        };
        let mut recheck = run();
        recheck.run_id = "run-2".into();
        recheck.started_at = "later".into();
        recheck.evidence[0].evidence_id = "rechecked-evidence".into();
        recheck.evidence[0].observed_at = "later".into();
        let mut repeated = draft();
        repeated.source_run_ids = vec![recheck.run_id.clone()];
        repeated.evidence = recheck.evidence.clone();
        assert!(matches!(
            repeated.materialize(&[recheck], Some(&first)).unwrap(),
            ArtifactMaterialization::Folded { .. }
        ));
    }

    #[test]
    fn evidence_id_does_not_authorize_rewriting_its_source() {
        let mut invented = draft();
        invented.evidence[0].source.source_id = "different-source".into();
        assert_eq!(
            invented.materialize(&[run()], None),
            Err(ArtifactBuildError::EvidenceDoesNotBelongToRun)
        );
    }
    #[test]
    fn changed_scope_or_conclusion_creates_new_revision_not_fuzzy_merge() {
        let first = match draft().materialize(&[run()], None).unwrap() {
            ArtifactMaterialization::Revision(value) => value,
            _ => panic!(),
        };
        let mut changed = draft();
        changed.conclusion = "passed".to_string();
        assert!(
            matches!(changed.materialize(&[run()], Some(&first)).unwrap(), ArtifactMaterialization::Revision(value) if value.revision == 2)
        );
        let mut distinct = draft();
        distinct.subject =
            SubjectKey::new(&project(), "check", "cargo", "src/lib", "main@a").unwrap();
        assert!(
            matches!(distinct.materialize(&[run()], Some(&first)).unwrap(), ArtifactMaterialization::Revision(value) if value.revision == 1 && value.artifact_id != first.artifact_id)
        );
    }
}
