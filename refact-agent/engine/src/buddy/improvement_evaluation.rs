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
