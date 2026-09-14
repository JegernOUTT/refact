use std::collections::BTreeSet;

use refact_buddy_core::operator_types::{EvidenceRef, NativeSourceIdentity, ProjectIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::operator_store::{OperatorStore, OperatorStoreError, Stored};

pub const SPEC_NAMESPACE: &str = "improvement_specs";
pub const PROOF_NAMESPACE: &str = "improvement_proofs";

/// Preparation-only records. Admission counts every retained revision: no proof,
/// known-good dependency, activation decision or pin is selected for deletion.
/// Proofs/attempts/fixed-run terminal writes deliberately require separate reservations.
pub const ADMISSION_NAMESPACES: &[&str] = &[SPEC_NAMESPACE, "improvement_candidates"];

pub fn digest<T: Serialize>(value: &T) -> Result<String, OperatorStoreError> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

pub fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EvaluationSpec {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub id: String,
    pub criteria: Vec<Criterion>,
    pub corpus_digest: String,
    pub privacy_scope: String,
    pub authority: NativeSourceIdentity,
    pub max_age_seconds: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<super::improvement_runner::FixedExecution>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub id: String,
    pub command_or_protocol: String,
    pub expected_result: String,
    pub external_review: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckResult {
    Passed,
    Failed,
    Unavailable,
    Inconclusive,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CheckObservation {
    pub criterion_id: String,
    pub command_or_protocol: String,
    pub result: CheckResult,
    pub observed_result: String,
    pub evidence: EvidenceRef,
    pub run_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EvaluationProof {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub id: String,
    pub candidate_digest: String,
    pub evaluation_digest: String,
    pub authority: NativeSourceIdentity,
    pub observed_at: String,
    pub checks: Vec<CheckObservation>,
}

pub fn register_spec(
    store: &mut OperatorStore,
    spec: &EvaluationSpec,
    operator: &NativeSourceIdentity,
) -> Result<Stored<EvaluationSpec>, OperatorStoreError> {
    if spec.schema_version != 1
        || &spec.project != store.identity()
        || &spec.authority != operator
        || spec.id.trim().is_empty()
        || spec.criteria.is_empty()
        || spec.criteria.len() > 32
        || !(1..=604800).contains(&spec.max_age_seconds)
        || !valid_digest(&spec.corpus_digest)
        || spec.privacy_scope.trim().is_empty()
    {
        return Err(OperatorStoreError::InvalidInput(
            "invalid fixed evaluation specification",
        ));
    }
    let mut ids = BTreeSet::new();
    for criterion in &spec.criteria {
        if criterion.id.trim().is_empty()
            || !ids.insert(&criterion.id)
            || criterion.command_or_protocol.trim().is_empty()
            || criterion.expected_result.trim().is_empty()
        {
            return Err(OperatorStoreError::InvalidInput(
                "invalid or duplicate evaluation criterion",
            ));
        }
    }
    if let Some(execution) = &spec.execution {
        execution.validate(spec)?;
    }
    store.put_document(SPEC_NAMESPACE, &spec.id, spec, None)
}

pub fn validate_proof(
    store: &OperatorStore,
    spec: &EvaluationSpec,
    proof: &EvaluationProof,
    candidate_digest: &str,
    now: chrono::DateTime<chrono::Utc>,
    require_pass: bool,
) -> Result<(), OperatorStoreError> {
    let runner_owned =
        proof.authority.source_id == "buddy-fixed-evaluator" && spec.execution.is_some();
    if runner_owned {
        let id = proof
            .id
            .strip_prefix("fixed:")
            .ok_or(OperatorStoreError::Conflict(
                "runner proof identity invalid",
            ))?;
        let run: Stored<super::improvement_runner::FixedRun> = store
            .get_document(super::improvement_runner::RUN_NAMESPACE, id)?
            .ok_or(OperatorStoreError::Conflict("trusted runner record absent"))?;
        if run.value.grant.candidate_digest != proof.candidate_digest
            || run.value.grant.evaluation_digest != proof.evaluation_digest
            || run.value.evaluator != proof.authority
            || run.value.state != "finished_no_activation"
        {
            return Err(OperatorStoreError::Conflict(
                "runner evidence binding invalid",
            ));
        }
    }
    let observed = chrono::DateTime::parse_from_rfc3339(&proof.observed_at)
        .map_err(|_| OperatorStoreError::InvalidInput("invalid evaluation observation time"))?;
    if proof.schema_version != 1
        || proof.project != spec.project
        || &proof.project != store.identity()
        || (!runner_owned && proof.authority != spec.authority)
        || proof.candidate_digest != candidate_digest
        || proof.evaluation_digest != digest(spec)?
        || observed > now
        || now.signed_duration_since(observed).num_seconds() > i64::from(spec.max_age_seconds)
        || proof.checks.len() != spec.criteria.len()
    {
        return Err(OperatorStoreError::Conflict(
            "stale or mismatched candidate evaluation proof",
        ));
    }
    let mut seen = BTreeSet::new();
    for check in &proof.checks {
        let criterion = spec
            .criteria
            .iter()
            .find(|criterion| criterion.id == check.criterion_id)
            .ok_or(OperatorStoreError::Conflict("evaluation criterion changed"))?;
        if !seen.insert(&check.criterion_id)
            || check.command_or_protocol != criterion.command_or_protocol
            || (require_pass && check.result != CheckResult::Passed)
            || check.observed_result.trim().is_empty()
            || !check.evidence.digest.as_deref().is_some_and(valid_digest)
        {
            return Err(OperatorStoreError::Conflict(
                "check failed, unavailable, duplicated or changed",
            ));
        }
        let evidence_time = chrono::DateTime::parse_from_rfc3339(&check.evidence.observed_at)
            .map_err(|_| OperatorStoreError::InvalidInput("invalid native evidence time"))?;
        if evidence_time > observed
            || now.signed_duration_since(evidence_time).num_seconds()
                > i64::from(spec.max_age_seconds)
            || check.evidence.source != proof.authority
        {
            return Err(OperatorStoreError::Conflict(
                "native evidence is stale or not from the fixed evaluator",
            ));
        }
        let run = store
            .get_run_outcome(&check.run_id)?
            .ok_or(OperatorStoreError::Conflict(
                "evaluation native evidence unavailable",
            ))?;
        if run.value.project != proof.project
            || !run.value.evidence.contains(&check.evidence)
            || run.value.input_fingerprint != candidate_digest
            || run.value.status != refact_buddy_core::operator_types::RunStatus::Completed
        {
            return Err(OperatorStoreError::Conflict(
                "native result does not bind this immutable candidate",
            ));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use refact_buddy_core::operator_types::{NativeSourceKind, RunOutcome, RunStatus};

    fn operator() -> NativeSourceIdentity {
        NativeSourceIdentity {
            source_kind: NativeSourceKind::Tool,
            source_id: "authenticated-independent-operator".into(),
            source_version: "1".into(),
        }
    }

    fn runner() -> NativeSourceIdentity {
        NativeSourceIdentity {
            source_kind: NativeSourceKind::Check,
            source_id: "buddy-fixed-evaluator".into(),
            source_version: "engine:worker:1:v1".into(),
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        store: OperatorStore,
        spec: EvaluationSpec,
        proof: EvaluationProof,
        candidate_digest: String,
        now: chrono::DateTime<chrono::Utc>,
    }

    impl Fixture {
        fn accepts(&self, proof: &EvaluationProof) -> Result<(), OperatorStoreError> {
            self.with_spec(&self.spec, proof, true)
        }

        fn with_spec(
            &self,
            spec: &EvaluationSpec,
            proof: &EvaluationProof,
            require_pass: bool,
        ) -> Result<(), OperatorStoreError> {
            validate_proof(
                &self.store,
                spec,
                proof,
                &self.candidate_digest,
                self.now,
                require_pass,
            )
        }
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "evaluation-proof".into(),
            canonical_root: std::fs::canonicalize(dir.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
        ))
        .unwrap();
        let now = chrono::Utc::now();
        let observed = now - chrono::Duration::seconds(30);
        let candidate_digest = "b".repeat(64);
        let spec = EvaluationSpec {
            schema_version: 1,
            project: project.clone(),
            id: "fixed-evaluation".into(),
            criteria: vec![
                Criterion {
                    id: "review".into(),
                    command_or_protocol: "external independent review protocol v1".into(),
                    expected_result: "all specified checks pass".into(),
                    external_review: true,
                },
                Criterion {
                    id: "build".into(),
                    command_or_protocol: "/usr/bin/cargo check".into(),
                    expected_result: "exit status 0".into(),
                    external_review: false,
                },
            ],
            corpus_digest: "a".repeat(64),
            privacy_scope: "isolated fixture only".into(),
            authority: operator(),
            max_age_seconds: 3600,
            execution: None,
        };
        register_spec(&mut store, &spec, &operator()).unwrap();
        let mut run: RunOutcome = serde_json::from_value(contract["run_outcome"].clone()).unwrap();
        run.project = project.clone();
        run.run_id = "independent-result".into();
        run.input_fingerprint = candidate_digest.clone();
        run.status = RunStatus::Completed;
        run.effects.clear();
        let mut review = run.evidence[0].clone();
        review.evidence_id = "evidence-review".into();
        review.source = operator();
        review.digest = Some("c".repeat(64));
        review.observed_at = (observed - chrono::Duration::seconds(60)).to_rfc3339();
        let mut build = review.clone();
        build.evidence_id = "evidence-build".into();
        build.digest = Some("d".repeat(64));
        run.evidence = vec![review.clone(), build.clone()];
        store.append_run_outcome(&run).unwrap();
        let proof = EvaluationProof {
            schema_version: 1,
            project,
            id: "proof".into(),
            candidate_digest: candidate_digest.clone(),
            evaluation_digest: digest(&spec).unwrap(),
            authority: operator(),
            observed_at: observed.to_rfc3339(),
            checks: vec![
                CheckObservation {
                    criterion_id: "review".into(),
                    command_or_protocol: spec.criteria[0].command_or_protocol.clone(),
                    result: CheckResult::Passed,
                    observed_result: "independent review attestation".into(),
                    evidence: review,
                    run_id: run.run_id.clone(),
                },
                CheckObservation {
                    criterion_id: "build".into(),
                    command_or_protocol: spec.criteria[1].command_or_protocol.clone(),
                    result: CheckResult::Passed,
                    observed_result: "exit status 0".into(),
                    evidence: build,
                    run_id: run.run_id.clone(),
                },
            ],
        };
        Fixture {
            _dir: dir,
            store,
            spec,
            proof,
            candidate_digest,
            now,
        }
    }

    #[test]
    fn a_complete_proof_bound_to_retained_native_evidence_is_accepted() {
        let fixture = fixture();
        fixture.accepts(&fixture.proof).unwrap();
        fixture
            .with_spec(&fixture.spec, &fixture.proof, false)
            .unwrap();
        assert_eq!(
            fixture.proof.evaluation_digest,
            digest(&fixture.spec).unwrap()
        );
    }

    #[test]
    fn evaluation_criteria_or_authority_change_invalidates_the_recorded_proof() {
        let fixture = fixture();
        for mutation in ["criterion", "protocol", "authority", "max_age", "privacy"] {
            let mut spec = fixture.spec.clone();
            match mutation {
                "criterion" => spec.criteria[1].expected_result = "any exit status".into(),
                "protocol" => spec.criteria[1].command_or_protocol = "/usr/bin/true".into(),
                "authority" => spec.authority.source_version = "2".into(),
                "max_age" => spec.max_age_seconds = 7200,
                _ => spec.privacy_scope = "share with anyone".into(),
            }
            assert_ne!(
                digest(&spec).unwrap(),
                fixture.proof.evaluation_digest,
                "{mutation}"
            );
            assert!(
                fixture.with_spec(&spec, &fixture.proof, true).is_err(),
                "{mutation}"
            );
        }
        let mut spec = fixture.spec.clone();
        spec.authority.source_id = "another-operator".into();
        let mut proof = fixture.proof.clone();
        proof.evaluation_digest = digest(&spec).unwrap();
        assert!(fixture.with_spec(&spec, &proof, true).is_err());
        let mut forged = fixture.proof.clone();
        forged.evaluation_digest = "f".repeat(64);
        assert!(fixture.accepts(&forged).is_err());
    }

    #[test]
    fn proof_schema_version_must_be_exactly_the_supported_one() {
        let fixture = fixture();
        for version in [0u16, 2, 65535] {
            let mut proof = fixture.proof.clone();
            proof.schema_version = version;
            assert!(fixture.accepts(&proof).is_err(), "{version}");
        }
        let wire = serde_json::to_value(&fixture.proof).unwrap();
        let mut missing = wire.clone();
        missing.as_object_mut().unwrap().remove("schema_version");
        assert!(serde_json::from_value::<EvaluationProof>(missing).is_err());
        let mut unknown = wire;
        unknown["authority_override"] = serde_json::json!("untrusted");
        assert!(serde_json::from_value::<EvaluationProof>(unknown).is_err());
    }

    #[test]
    fn proof_bound_to_another_project_or_evaluator_is_refused() {
        let fixture = fixture();
        for mutation in ["project_id", "canonical_root"] {
            let mut proof = fixture.proof.clone();
            match mutation {
                "project_id" => proof.project.project_id = "another-project".into(),
                _ => proof.project.canonical_root = "/somewhere/else".into(),
            }
            assert!(fixture.accepts(&proof).is_err(), "{mutation}");
        }
        let mut spec = fixture.spec.clone();
        spec.project.project_id = "another-project".into();
        let mut agreed = fixture.proof.clone();
        agreed.project = spec.project.clone();
        agreed.evaluation_digest = digest(&spec).unwrap();
        assert!(fixture.with_spec(&spec, &agreed, true).is_err());
        for mutation in ["source_id", "source_version", "source_kind"] {
            let mut proof = fixture.proof.clone();
            match mutation {
                "source_id" => proof.authority.source_id = "candidate-worker".into(),
                "source_version" => proof.authority.source_version = "2".into(),
                _ => proof.authority.source_kind = NativeSourceKind::Chat,
            }
            assert!(fixture.accepts(&proof).is_err(), "{mutation}");
        }
        let mut foreign_evidence = fixture.proof.clone();
        foreign_evidence.checks[0].evidence.source.source_id = "candidate-worker".into();
        assert!(fixture.accepts(&foreign_evidence).is_err());
    }

    #[test]
    fn forged_or_truncated_proofs_never_pass_the_activation_gate() {
        let fixture = fixture();
        let mut truncated = fixture.proof.clone();
        truncated.checks.pop();
        assert!(fixture.accepts(&truncated).is_err());
        let mut padded = fixture.proof.clone();
        padded.checks.push(padded.checks[0].clone());
        assert!(fixture.accepts(&padded).is_err());
        let mut duplicated = fixture.proof.clone();
        duplicated.checks[1] = duplicated.checks[0].clone();
        assert!(fixture.accepts(&duplicated).is_err());
        let mut renamed = fixture.proof.clone();
        renamed.checks[1].criterion_id = "unknown-criterion".into();
        assert!(fixture.accepts(&renamed).is_err());
        let mut rewritten = fixture.proof.clone();
        rewritten.checks[1].command_or_protocol = "/usr/bin/true".into();
        assert!(fixture.accepts(&rewritten).is_err());
        let mut silent = fixture.proof.clone();
        silent.checks[1].observed_result = "   ".into();
        assert!(fixture.accepts(&silent).is_err());
        for digest_value in [None, Some("not-a-digest".to_string()), Some("f".repeat(64))] {
            let mut proof = fixture.proof.clone();
            proof.checks[1].evidence.digest = digest_value.clone();
            assert!(fixture.accepts(&proof).is_err(), "{digest_value:?}");
        }
        let mut unknown_run = fixture.proof.clone();
        unknown_run.checks[1].run_id = "never-executed".into();
        assert!(fixture.accepts(&unknown_run).is_err());
        let mut other_candidate = fixture.proof.clone();
        other_candidate.candidate_digest = "0".repeat(64);
        assert!(fixture.accepts(&other_candidate).is_err());
    }

    #[test]
    fn activation_requires_passing_checks_while_recording_still_accepts_results() {
        let fixture = fixture();
        for result in [
            CheckResult::Failed,
            CheckResult::Unavailable,
            CheckResult::Inconclusive,
        ] {
            let mut proof = fixture.proof.clone();
            proof.checks[1].result = result.clone();
            assert!(fixture.accepts(&proof).is_err(), "{result:?}");
            fixture.with_spec(&fixture.spec, &proof, false).unwrap();
        }
    }

    #[test]
    fn stale_future_or_unparsable_observation_times_are_refused() {
        let fixture = fixture();
        let mut future = fixture.proof.clone();
        future.observed_at = (fixture.now + chrono::Duration::seconds(5)).to_rfc3339();
        assert!(fixture.accepts(&future).is_err());
        let mut stale = fixture.proof.clone();
        stale.observed_at = (fixture.now
            - chrono::Duration::seconds(i64::from(fixture.spec.max_age_seconds) + 1))
        .to_rfc3339();
        assert!(fixture.accepts(&stale).is_err());
        let mut unparsable = fixture.proof.clone();
        unparsable.observed_at = "yesterday".into();
        assert!(fixture.accepts(&unparsable).is_err());
        let mut later_evidence = fixture.proof.clone();
        later_evidence.checks[0].evidence.observed_at = fixture.now.to_rfc3339();
        assert!(fixture.accepts(&later_evidence).is_err());
    }

    #[test]
    fn runner_authority_requires_a_retained_trusted_runner_record() {
        let fixture = fixture();
        let mut impostor = fixture.proof.clone();
        impostor.authority = runner();
        impostor.checks[0].evidence.source = runner();
        impostor.checks[1].evidence.source = runner();
        assert!(fixture.accepts(&impostor).is_err());
        let mut spec = fixture.spec.clone();
        spec.execution = Some(crate::buddy::improvement_runner::FixedExecution {
            commands: vec![],
            offline: None,
            output_limit_bytes: 4096,
            total_timeout_seconds: 60,
        });
        let mut claimed = impostor.clone();
        claimed.evaluation_digest = digest(&spec).unwrap();
        claimed.id = "fixed:never-claimed".into();
        assert!(fixture.with_spec(&spec, &claimed, true).is_err());
        let mut unprefixed = claimed;
        unprefixed.id = "proof".into();
        assert!(fixture.with_spec(&spec, &unprefixed, true).is_err());
    }

    #[test]
    fn native_evidence_must_bind_a_completed_run_for_this_candidate() {
        let mut fixture = fixture();
        for mutation in ["status", "fingerprint"] {
            let mut run: RunOutcome = fixture
                .store
                .get_run_outcome("independent-result")
                .unwrap()
                .unwrap()
                .value;
            run.run_id = format!("{mutation}-run");
            match mutation {
                "status" => run.status = RunStatus::Partial,
                _ => run.input_fingerprint = "0".repeat(64),
            }
            fixture.store.append_run_outcome(&run).unwrap();
            let mut proof = fixture.proof.clone();
            proof.checks[0].run_id = run.run_id.clone();
            proof.checks[1].run_id = run.run_id;
            assert!(fixture.accepts(&proof).is_err(), "{mutation}");
        }
    }
}
