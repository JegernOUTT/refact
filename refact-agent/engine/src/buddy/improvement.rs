use refact_buddy_core::operator_types::{
    ActionSpec, EvidenceRef, ImprovementCandidate, NativeSourceIdentity, OperatorContract,
    ProjectIdentity,
};
use serde::{Deserialize, Serialize};

use super::improvement_evaluation::{
    digest, valid_digest, validate_proof, EvaluationProof, EvaluationSpec, PROOF_NAMESPACE,
    SPEC_NAMESPACE,
};
use super::operator_store::{OperatorStore, OperatorStoreError, Stored};

pub const CANDIDATE_NAMESPACE: &str = "improvement_candidates";
pub const ATTEMPT_NAMESPACE: &str = "improvement_attempts";

#[path = "improvement_activation.rs"]
pub mod activation;

#[path = "improvement_artifacts.rs"]
pub mod artifacts;

#[path = "improvement_installation.rs"]
pub mod installation;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentTarget {
    Configuration,
    SourceCandidate,
    ReviewedCommit,
    EngineBuild,
    GuiBuild,
    PluginPackage,
    InstalledVersion,
    RunningHost,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CandidateRecord {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub candidate: ImprovementCandidate,
    pub target: DeploymentTarget,
    pub target_files: Vec<String>,
    pub content_digest: String,
    pub evaluation_digest: String,
    pub source_identity: String,
    pub commit_identity: Option<String>,
    pub engine_build: Option<String>,
    pub gui_build: Option<String>,
    pub plugin_package: Option<String>,
    pub installed_version: Option<String>,
    pub running_host: Option<String>,
    pub original_run_id: String,
    pub original_problem: Vec<EvidenceRef>,
    pub activation_action: Option<ActionSpec>,
    pub activation_draft_digest: Option<String>,
    pub submitted_by: NativeSourceIdentity,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptState {
    SpawnClaimed,
    WorkerStarted,
    ActivationBlocked,
    ActivationClaimed,
    OutcomeUnknown,
    Applied,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImprovementAttempt {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub id: String,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub state: AttemptState,
    pub native_id: Option<String>,
    pub detail: String,
    pub recovery: String,
}

pub fn register_candidate(
    store: &mut OperatorStore,
    record: &CandidateRecord,
    operator: &NativeSourceIdentity,
) -> Result<Stored<CandidateRecord>, OperatorStoreError> {
    record
        .candidate
        .validate()
        .map_err(|_| OperatorStoreError::InvalidInput("invalid candidate contract"))?;
    if record.schema_version != 1
        || &record.project != store.identity()
        || record.candidate.project != record.project
        || &record.submitted_by != operator
        || !valid_digest(&record.content_digest)
        || !valid_digest(&record.evaluation_digest)
        || record.source_identity.trim().is_empty()
        || record.original_problem.is_empty()
        || record.original_problem.len() > 12
        || record.candidate.activation_decision.is_some()
        || !record.candidate.proof_refs.is_empty()
    {
        return Err(OperatorStoreError::InvalidInput(
            "invalid candidate identity or candidate-granted authority",
        ));
    }
    let artifact_identity = match record.target {
        DeploymentTarget::EngineBuild => Some(record.engine_build.as_deref()),
        DeploymentTarget::GuiBuild => Some(record.gui_build.as_deref()),
        DeploymentTarget::PluginPackage => Some(record.plugin_package.as_deref()),
        _ => None,
    };
    if let Some(identity) = artifact_identity {
        if !identity.is_some_and(valid_digest)
            || record.candidate.build_identity.as_deref() != identity
        {
            return Err(OperatorStoreError::InvalidInput(
                "artifact target requires its exact SHA-256 build identity; source patch is not a built package",
            ));
        }
    }
    let spec: Stored<EvaluationSpec> = store
        .get_document(SPEC_NAMESPACE, &record.candidate.evaluation_spec_version)?
        .ok_or(OperatorStoreError::Conflict(
            "fixed evaluation specification unavailable",
        ))?;
    if digest(&spec.value)? != record.evaluation_digest {
        return Err(OperatorStoreError::Conflict(
            "evaluation specification digest changed",
        ));
    }
    if record.target_files.is_empty()
        || record.target_files.len() > 100
        || record.target_files.iter().any(|path| {
            path.is_empty()
                || path.contains('\\')
                || std::path::Path::new(path)
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
        })
    {
        return Err(OperatorStoreError::InvalidInput(
            "candidate requires bounded project-relative target files",
        ));
    }
    let original =
        store
            .get_run_outcome(&record.original_run_id)?
            .ok_or(OperatorStoreError::Conflict(
                "original problem evidence unavailable",
            ))?;
    if !record
        .original_problem
        .iter()
        .all(|evidence| original.value.evidence.contains(evidence))
    {
        return Err(OperatorStoreError::Conflict(
            "original problem evidence changed",
        ));
    }
    if let Some(action) = &record.activation_action {
        action
            .validate()
            .map_err(|_| OperatorStoreError::InvalidInput("invalid activation action"))?;
        if action.project != record.project {
            return Err(OperatorStoreError::Conflict(
                "activation action belongs to another project",
            ));
        }
        activation::validate_action_binding(store, record, action)?;
    }
    if record.target != DeploymentTarget::Configuration
        && record.candidate.patch_digest.as_deref() != Some(&record.content_digest)
    {
        return Err(OperatorStoreError::Conflict(
            "patch digest does not match candidate content",
        ));
    }
    if record.target == DeploymentTarget::Configuration
        && record
            .candidate
            .config_draft_ref
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(OperatorStoreError::InvalidInput(
            "configuration candidates require an exact native draft reference",
        ));
    }
    if record.target != DeploymentTarget::Configuration
        && !spec
            .value
            .criteria
            .iter()
            .any(|criterion| criterion.external_review)
    {
        return Err(OperatorStoreError::InvalidInput(
            "engineering candidates require independent external review proof",
        ));
    }
    store.put_document(
        CANDIDATE_NAMESPACE,
        &record.candidate.candidate_id,
        record,
        None,
    )
}

pub fn record_proof(
    store: &mut OperatorStore,
    candidate_id: &str,
    proof: &EvaluationProof,
    operator: &NativeSourceIdentity,
) -> Result<Stored<EvaluationProof>, OperatorStoreError> {
    if proof.authority.source_id == "buddy-fixed-evaluator" || proof.id.starts_with("fixed:") {
        return Err(OperatorStoreError::Conflict(
            "runner proofs cannot be uploaded",
        ));
    }
    let candidate = candidate(store, candidate_id)?;
    let spec: Stored<EvaluationSpec> = store
        .get_document(
            SPEC_NAMESPACE,
            &candidate.value.candidate.evaluation_spec_version,
        )?
        .ok_or(OperatorStoreError::Conflict(
            "evaluation specification unavailable",
        ))?;
    if operator != &spec.value.authority || proof.authority != *operator {
        return Err(OperatorStoreError::Conflict(
            "only the pre-bound independent evaluator may attest results",
        ));
    }
    validate_proof(
        store,
        &spec.value,
        proof,
        &digest(&candidate.value)?,
        chrono::Utc::now(),
        false,
    )?;
    store.put_document(PROOF_NAMESPACE, &proof.id, proof, None)
}

pub fn candidate(
    store: &OperatorStore,
    id: &str,
) -> Result<Stored<CandidateRecord>, OperatorStoreError> {
    store
        .get_document(CANDIDATE_NAMESPACE, id)?
        .ok_or(OperatorStoreError::Conflict("candidate unavailable"))
}

pub fn activation_gate(
    store: &OperatorStore,
    id: &str,
    expected_digest: &str,
    proof_id: &str,
) -> Result<CandidateRecord, OperatorStoreError> {
    let record = candidate(store, id)?.value;
    if digest(&record)? != expected_digest {
        return Err(OperatorStoreError::Conflict(
            "candidate changed after evaluation",
        ));
    }
    let spec: Stored<EvaluationSpec> = store
        .get_document(SPEC_NAMESPACE, &record.candidate.evaluation_spec_version)?
        .ok_or(OperatorStoreError::Conflict(
            "evaluation specification unavailable",
        ))?;
    if digest(&spec.value)? != record.evaluation_digest {
        return Err(OperatorStoreError::Conflict(
            "evaluation criteria or authority changed",
        ));
    }
    let proof: Stored<EvaluationProof> = store
        .get_document(PROOF_NAMESPACE, proof_id)?
        .ok_or(OperatorStoreError::Conflict("evaluation proof unavailable"))?;
    validate_proof(
        store,
        &spec.value,
        &proof.value,
        expected_digest,
        chrono::Utc::now(),
        true,
    )?;
    Ok(record)
}

pub fn validate_draft_binding(
    store: &OperatorStore,
    record: &CandidateRecord,
    action: &ActionSpec,
) -> Result<(), OperatorStoreError> {
    let draft_id = action
        .parameter_ref
        .as_deref()
        .ok_or(OperatorStoreError::Conflict("activation draft unavailable"))?;
    let draft: Stored<serde_json::Value> =
        store
            .get_document("resource_draft", draft_id)?
            .ok_or(OperatorStoreError::Conflict(
                "native activation draft unavailable",
            ))?;
    if record.activation_draft_digest.as_deref() != Some(digest(&draft.value)?.as_str())
        || (record.target == DeploymentTarget::Configuration
            && record.activation_draft_digest.as_deref() != Some(&record.content_digest))
        || (record.target == DeploymentTarget::Configuration
            && record.candidate.config_draft_ref.as_deref() != Some(draft_id))
    {
        return Err(OperatorStoreError::Conflict(
            "candidate content differs from immutable native draft",
        ));
    }
    Ok(())
}

pub fn recovery_description(record: &CandidateRecord) -> String {
    match record.target {
        DeploymentTarget::Configuration => "Use the owning resource execution's version-bound recovery under a fresh explicit grant. No automatic rollback or replay.".into(),
        DeploymentTarget::SourceCandidate | DeploymentTarget::ReviewedCommit => "Retain the isolated worktree. A separate registered recovery grant may revert the exact native squash landing only while its clean target post-image and known-good parent remain unchanged. Never reset, overwrite intervening edits or replay an unknown outcome.".into(),
        _ => format!("Binary/host activation blocked: release installer does not bind this candidate's package digest, host compatibility, drain/restart and rollback authority. Known-good reference: {}. Use the owning installer/host manually; installed and running identities remain unverified.", record.candidate.previous_known_good_identity.as_deref().unwrap_or("unavailable")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::improvement_evaluation::{
        Criterion, CheckObservation, CheckResult, register_spec,
    };

    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
        ))
        .unwrap()
    }

    fn setup() -> (
        tempfile::TempDir,
        OperatorStore,
        CandidateRecord,
        EvaluationProof,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "improvement-test".into(),
            canonical_root: std::fs::canonicalize(dir.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        let mut original: refact_buddy_core::operator_types::RunOutcome =
            serde_json::from_value(fixture()["run_outcome"].clone()).unwrap();
        original.project = project.clone();
        original.run_id = "original-problem".into();
        store.append_run_outcome(&original).unwrap();
        let operator = NativeSourceIdentity {
            source_kind: refact_buddy_core::operator_types::NativeSourceKind::Tool,
            source_id: "authenticated-independent-operator".into(),
            source_version: "1".into(),
        };
        let spec = EvaluationSpec {
            schema_version: 1,
            project: project.clone(),
            id: "fixed-evaluation".into(),
            criteria: vec![Criterion {
                id: "review".into(),
                command_or_protocol: "external independent review protocol v1".into(),
                expected_result: "all specified checks pass".into(),
                external_review: true,
            }],
            corpus_digest: "a".repeat(64),
            privacy_scope: "isolated fixture only".into(),
            authority: operator.clone(),
            max_age_seconds: 3600,
            execution: None,
        };
        register_spec(&mut store, &spec, &operator).unwrap();
        let candidate = CandidateRecord {
            schema_version: 1,
            project: project.clone(),
            candidate: ImprovementCandidate {
                schema_version: 1,
                candidate_id: "candidate".into(),
                project: project.clone(),
                target: "source_candidate".into(),
                worktree_ref: Some("isolated-worktree".into()),
                config_draft_ref: None,
                patch_digest: Some("b".repeat(64)),
                build_identity: None,
                evaluation_spec_version: spec.id.clone(),
                proof_refs: vec![],
                activation_decision: None,
                previous_known_good_identity: Some("known-good".into()),
            },
            target: DeploymentTarget::SourceCandidate,
            target_files: vec!["src/candidate.rs".into()],
            content_digest: "b".repeat(64),
            evaluation_digest: digest(&spec).unwrap(),
            source_identity: "source-tree".into(),
            commit_identity: Some("commit".into()),
            engine_build: None,
            gui_build: None,
            plugin_package: None,
            installed_version: None,
            running_host: None,
            original_run_id: original.run_id.clone(),
            original_problem: original.evidence.clone(),
            activation_action: None,
            activation_draft_digest: None,
            submitted_by: operator.clone(),
        };
        register_candidate(&mut store, &candidate, &operator).unwrap();
        let mut evaluation = original;
        evaluation.run_id = "independent-result".into();
        evaluation.input_fingerprint = digest(&candidate).unwrap();
        evaluation.status = refact_buddy_core::operator_types::RunStatus::Completed;
        evaluation.evidence[0].digest = Some("c".repeat(64));
        evaluation.evidence[0].observed_at = chrono::Utc::now().to_rfc3339();
        evaluation.evidence[0].source = operator.clone();
        store.append_run_outcome(&evaluation).unwrap();
        let proof = EvaluationProof {
            schema_version: 1,
            project,
            id: "proof".into(),
            candidate_digest: digest(&candidate).unwrap(),
            evaluation_digest: digest(&spec).unwrap(),
            authority: operator,
            observed_at: chrono::Utc::now().to_rfc3339(),
            checks: vec![CheckObservation {
                criterion_id: "review".into(),
                command_or_protocol: spec.criteria[0].command_or_protocol.clone(),
                result: CheckResult::Passed,
                observed_result: "fixture attestation, not live review".into(),
                evidence: evaluation.evidence[0].clone(),
                run_id: evaluation.run_id,
            }],
        };
        (dir, store, candidate, proof)
    }

    #[test]
    fn artifact_targets_require_distinct_exact_build_not_source_identity() {
        let (_dir, mut store, candidate, _proof) = setup();
        for (id, target) in [
            ("engine", DeploymentTarget::EngineBuild),
            ("gui", DeploymentTarget::GuiBuild),
            ("plugin", DeploymentTarget::PluginPackage),
        ] {
            let mut artifact = candidate.clone();
            artifact.candidate.candidate_id = id.into();
            artifact.target = target;
            assert!(register_candidate(&mut store, &artifact, &artifact.submitted_by).is_err());
            let exact = "d".repeat(64);
            match artifact.target {
                DeploymentTarget::EngineBuild => artifact.engine_build = Some(exact.clone()),
                DeploymentTarget::GuiBuild => artifact.gui_build = Some(exact.clone()),
                _ => artifact.plugin_package = Some(exact.clone()),
            }
            artifact.candidate.build_identity = Some("source-is-not-package".into());
            assert!(register_candidate(&mut store, &artifact, &artifact.submitted_by).is_err());
            artifact.candidate.build_identity = Some(exact);
            assert!(register_candidate(&mut store, &artifact, &artifact.submitted_by).is_ok());
            assert!(artifact.installed_version.is_none());
            assert!(artifact.running_host.is_none());
        }
    }

    #[test]
    fn a18_candidate_patch_build_eval_and_authority_changes_invalidate_proof() {
        let (_dir, mut store, candidate, proof) = setup();
        record_proof(&mut store, "candidate", &proof, &proof.authority).unwrap();
        assert!(activation_gate(&store, "candidate", &proof.candidate_digest, "proof").is_ok());
        for mutation in ["patch", "build", "eval", "authority"] {
            let mut changed = candidate.clone();
            match mutation {
                "patch" => changed.content_digest = "d".repeat(64),
                "build" => changed.engine_build = Some("different-build".into()),
                "eval" => changed.evaluation_digest = "e".repeat(64),
                _ => changed.submitted_by.source_version = "2".into(),
            }
            assert_ne!(digest(&changed).unwrap(), proof.candidate_digest);
            assert!(register_candidate(&mut store, &changed, &changed.submitted_by).is_err());
            assert!(
                activation_gate(&store, "candidate", &digest(&changed).unwrap(), "proof").is_err()
            );
        }
        let mut unavailable = proof.clone();
        unavailable.id = "unavailable-proof".into();
        unavailable.checks[0].result = CheckResult::Unavailable;
        record_proof(
            &mut store,
            "candidate",
            &unavailable,
            &unavailable.authority,
        )
        .unwrap();
        assert!(activation_gate(
            &store,
            "candidate",
            &proof.candidate_digest,
            &unavailable.id
        )
        .is_err());
        let mut stale = proof.clone();
        stale.observed_at = "2000-01-01T00:00:00Z".into();
        assert!(record_proof(&mut store, "candidate", &stale, &stale.authority).is_err());
        let mut foreign = proof.authority.clone();
        foreign.source_id = "candidate-worker".into();
        assert!(record_proof(&mut store, "candidate", &proof, &foreign).is_err());
    }

    #[test]
    fn a19_failed_activation_keeps_history_and_offers_only_supported_recovery() {
        let (dir, mut store, candidate, proof) = setup();
        record_proof(&mut store, "candidate", &proof, &proof.authority).unwrap();
        let attempt = ImprovementAttempt {
            schema_version: 1,
            project: candidate.project.clone(),
            id: "failed-activation".into(),
            candidate_id: "candidate".into(),
            candidate_digest: proof.candidate_digest.clone(),
            state: AttemptState::OutcomeUnknown,
            native_id: Some("native-execution".into()),
            detail: "effect may have happened; never replay".into(),
            recovery: recovery_description(&candidate),
        };
        store
            .put_document(ATTEMPT_NAMESPACE, &attempt.id, &attempt, None)
            .unwrap();
        drop(store);
        let store = OperatorStore::open(dir.path(), &candidate.project).unwrap();
        assert_eq!(
            store
                .get_document::<ImprovementAttempt>(ATTEMPT_NAMESPACE, &attempt.id)
                .unwrap()
                .unwrap()
                .value,
            attempt
        );
        assert!(store
            .get_document::<EvaluationProof>(PROOF_NAMESPACE, &proof.id)
            .unwrap()
            .is_some());
        for target in [
            DeploymentTarget::EngineBuild,
            DeploymentTarget::GuiBuild,
            DeploymentTarget::PluginPackage,
            DeploymentTarget::InstalledVersion,
            DeploymentTarget::RunningHost,
        ] {
            let mut binary = candidate.clone();
            binary.target = target;
            let recovery = recovery_description(&binary);
            assert!(recovery.contains("activation blocked"));
            assert!(recovery.contains("known-good"));
            assert!(recovery.contains("unverified"));
        }
        let request = spawn_request(&candidate, "parent".into(), "explicit-model".into()).unwrap();
        assert!(matches!(
            request.worktree_mode,
            crate::agents::spawn::SpawnWorktreeMode::Isolated { auto_merge: false }
        ));
        assert!(!request.tools.unwrap().iter().any(|tool| [
            "shell",
            "review",
            "subagent",
            "worktree_merge"
        ]
        .contains(&tool.as_str())));
        assert!(spawn_request(&candidate, "parent".into(), "".into()).is_err());
    }
}

pub fn spawn_request(
    record: &CandidateRecord,
    parent_chat_id: String,
    model: String,
) -> Result<crate::agents::spawn::SpawnRequest, OperatorStoreError> {
    if parent_chat_id.trim().is_empty() || model.trim().is_empty() {
        return Err(OperatorStoreError::InvalidInput(
            "explicit parent and model are required; fallback is prohibited",
        ));
    }
    Ok(crate::agents::spawn::SpawnRequest {
        completion_push: Default::default(),
        kind: crate::agents::types::BgAgentKind::Subagent,
        parent_chat_id,
        parent_root_chat_id: None,
        parent_tool_call_id: None,
        config_name: "subagent".into(),
        title: format!("Candidate {}", record.candidate.candidate_id),
        prompt: format!("Investigate immutable candidate {} (digest {}). Original evidence: {}. Produce an isolated successor proposal; changes require a new candidate identity and new independent evaluation. Do not modify acceptance criteria, grant authority, merge, deploy, or claim evaluation success.", record.candidate.candidate_id, digest(record)?, serde_json::to_string(&record.original_problem)?),
        tools: Some(vec!["cat".into(), "tree".into(), "search_pattern".into(), "glob".into(), "create_textdoc".into(), "update_textdoc".into(), "apply_patch".into()]),
        target_files: record.target_files.clone(),
        max_steps: 32,
        model,
        model_type: None,
        goal: None,
        plan: None,
        worktree_mode: crate::agents::spawn::SpawnWorktreeMode::Isolated { auto_merge: false },
        parent_subchat_tx: None,
        parent_worktree: None,
        parent_task_meta: None,
        native_supervision_guard: None,
        subchat_depth: 0,
        notify_parent: crate::agents::spawn::NotifyParent::Auto,
    })
}
