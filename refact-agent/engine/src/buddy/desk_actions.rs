use std::sync::Arc;

use refact_buddy_core::operator_types::{
    ActionAvailability, ActionCapabilities, ActionExecution, ActionSpec, ArtifactRevision,
    AuthorityClass, ConfirmationClass, DecisionReference, NativeSourceIdentity, NativeSourceKind,
    OperatorContract, ResourceRef, WorkerIdentity, OPERATOR_SCHEMA_VERSION_V1,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::action_execution::{
    action_spec_hash, ActionAdapter, ActionRequest, ActionResult, ActionService, AdapterVersion,
    ApplyFailure, ApplyOutcome, Authorizer, OperatorStoreRepository, PreparedAction,
};
use super::operator_store::{OperatorStore, OperatorStoreError, Stored};

pub const CHECK_SAVED_EVIDENCE: &str = "check_saved_evidence";
const ADAPTER_ID: &str = "operator.saved_evidence";
const MAX_LINKS: usize = 200;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeskActionRequest {
    pub project_id: String,
    pub work_revision: u64,
    pub artifact_id: String,
    pub artifact_revision: u32,
    pub spec_hash: String,
    pub idempotency_key: String,
}

#[derive(Serialize)]
pub struct DeskAction {
    pub work_id: String,
    pub spec: ActionSpec,
    pub spec_hash: String,
    pub work_revision: u64,
    pub artifact_id: String,
    pub artifact_revision: u32,
    pub label: String,
    pub description: String,
    pub latest_execution: Option<Stored<ActionExecution>>,
}

struct EvidenceLinksAdapter {
    spec: ActionSpec,
    complete: bool,
    observations: Vec<String>,
}

impl ActionAdapter for EvidenceLinksAdapter {
    fn adapter_id(&self) -> &str {
        ADAPTER_ID
    }
    fn supports_kind(&self, kind: &str) -> bool {
        kind == CHECK_SAVED_EVIDENCE
    }
    fn expected_version(&self, spec: &ActionSpec) -> Result<AdapterVersion, ()> {
        Ok(if spec == &self.spec {
            AdapterVersion::Matches
        } else {
            AdapterVersion::Mismatch
        })
    }
    fn preview(&self, prepared: &PreparedAction) -> Result<(), ()> {
        (prepared.spec == self.spec).then_some(()).ok_or(())
    }
    fn apply(&self, prepared: &PreparedAction) -> Result<ApplyOutcome, ApplyFailure> {
        if prepared.spec != self.spec {
            return Err(ApplyFailure {
                effects_may_have_occurred: false,
            });
        }
        if self.complete {
            Ok(ApplyOutcome::Completed {
                effects: vec![],
                verification_evidence: self.observations.clone(),
            })
        } else {
            Ok(ApplyOutcome::Partial {
                effects: vec![],
                verification_evidence: self.observations.clone(),
                recovery: None,
            })
        }
    }
}

struct EvidenceLinksAuthorizer;

fn requester() -> NativeSourceIdentity {
    NativeSourceIdentity {
        source_kind: NativeSourceKind::Check,
        source_id: "buddy-desk:saved-evidence".into(),
        source_version: "1".into(),
    }
}

impl Authorizer for EvidenceLinksAuthorizer {
    fn authorize(
        &self,
        spec: &ActionSpec,
        source: &NativeSourceIdentity,
        decision: Option<&DecisionReference>,
    ) -> bool {
        spec.adapter == ADAPTER_ID
            && spec.kind == CHECK_SAVED_EVIDENCE
            && spec.authority_class == AuthorityClass::ReadOnly
            && spec.confirmation == ConfirmationClass::None
            && spec.parameter_ref.is_none()
            && source == &requester()
            && decision.is_none()
    }
}

pub fn actions_for_work(
    store: &OperatorStore,
    work_id: &str,
) -> Result<Vec<DeskAction>, OperatorStoreError> {
    let work = store
        .get_work_item(work_id)?
        .ok_or(OperatorStoreError::InvalidInput("work item not found"))?;
    work.value
        .validate()
        .map_err(|_| OperatorStoreError::InvalidInput("invalid work item"))?;
    if &work.value.project != store.identity() {
        return Err(OperatorStoreError::ProjectMismatch);
    }
    let (Some(id), Some(revision)) = (
        &work.value.current_artifact_id,
        work.value.current_artifact_revision,
    ) else {
        return Ok(vec![]);
    };
    let artifact = store
        .get_artifact_revision(id, Some(revision))?
        .ok_or(OperatorStoreError::Conflict("current report is missing"))?
        .value;
    artifact
        .validate()
        .map_err(|_| OperatorStoreError::InvalidInput("invalid report"))?;
    if &artifact.project != store.identity() {
        return Err(OperatorStoreError::ProjectMismatch);
    }
    let version = hex::encode(Sha256::digest(serde_json::to_vec(&(
        work.revision,
        &artifact,
    ))?));
    let bounded =
        artifact.source_run_ids.len() <= MAX_LINKS && artifact.evidence.len() <= MAX_LINKS;
    let spec = ActionSpec {
        schema_version: OPERATOR_SCHEMA_VERSION_V1,
        action_id: CHECK_SAVED_EVIDENCE.into(),
        kind: CHECK_SAVED_EVIDENCE.into(),
        adapter: ADAPTER_ID.into(),
        project: store.identity().clone(),
        resource: ResourceRef {
            resource_kind: "saved_report".into(),
            resource_id: work_id.into(),
            version: version.clone(),
        },
        expected_version: version,
        parameter_ref: None,
        availability: if bounded {
            ActionAvailability::Available
        } else {
            ActionAvailability::Unavailable
        },
        confirmation: ConfirmationClass::None,
        authority_class: AuthorityClass::ReadOnly,
        capabilities: ActionCapabilities {
            can_preview: true,
            can_execute: bounded,
            can_verify: false,
            can_recover: false,
            can_cancel: false,
        },
    };
    let spec_hash = action_spec_hash(&spec, &None);
    let latest_execution = store.latest_execution_for_spec(&spec_hash)?;
    let mut actions = vec![DeskAction {
        work_id: work_id.into(),
        spec, spec_hash, work_revision: work.revision, artifact_id: id.clone(), artifact_revision: revision,
        label: "Check saved evidence links".into(),
        description: if bounded {
            "Checks retained source outcomes and evidence metadata. Does not open source files or URLs, verify findings, or apply fixes. Only the check receipt is saved."
        } else { "This report exceeds the bounded evidence-check limit; no check will run." }.into(),
        latest_execution,
    }];
    if let Some(registered) =
        store.get_document::<WorkActionBindings>("work_action_bindings", work_id)?
    {
        for hash in registered.value.spec_hashes {
            let Some(origin) =
                store.get_document::<WorkActionOrigin>("work_action_origin", &hash)?
            else {
                continue;
            };
            if origin.value.work_revision != work.revision
                || origin.value.artifact_id != *id
                || origin.value.artifact_revision != revision
            {
                continue;
            }
            let Some(prepared) = store.get_document::<super::resource_execution::ResourceDocument<
                PreparedAction,
            >>("prepared_action", &hash)?
            else {
                continue;
            };
            let prepared = prepared.value.payload;
            if prepared.spec_hash != hash || prepared.spec.project != work.value.project {
                return Err(OperatorStoreError::ProjectMismatch);
            }
            actions.push(DeskAction {
                work_id: work_id.into(), spec: prepared.spec.clone(), spec_hash: hash.clone(),
                work_revision: work.revision, artifact_id: id.clone(), artifact_revision: revision,
                label: format!("Preview {} change", prepared.spec.resource.resource_kind),
                description: "Review the registered resource scope and exact changes before granting permission. Report text does not authorize execution.".into(),
                latest_execution: store.latest_execution_for_spec(&hash)?,
            });
        }
    }
    Ok(actions)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkActionOrigin {
    pub schema_version: u16,
    pub project: refact_buddy_core::operator_types::ProjectIdentity,
    pub work_id: String,
    pub work_revision: u64,
    pub artifact_id: String,
    pub artifact_revision: u32,
    pub spec_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorkActionBindings {
    schema_version: u16,
    project: refact_buddy_core::operator_types::ProjectIdentity,
    spec_hashes: Vec<String>,
}

pub fn attach_resource_action(
    store: &mut OperatorStore,
    origin: WorkActionOrigin,
) -> Result<(), OperatorStoreError> {
    if origin.project != *store.identity() {
        return Err(OperatorStoreError::ProjectMismatch);
    }
    validate_action_origin(store, &origin)?;
    let prepared = store
        .get_document::<super::resource_execution::ResourceDocument<PreparedAction>>(
            "prepared_action",
            &origin.spec_hash,
        )?
        .ok_or(OperatorStoreError::Conflict(
            "prepared resource action unavailable",
        ))?
        .value
        .payload;
    if prepared.spec_hash != origin.spec_hash
        || prepared.spec.project != origin.project
        || prepared.spec.authority_class != AuthorityClass::Mutating
    {
        return Err(OperatorStoreError::Conflict(
            "prepared resource identity changed",
        ));
    }
    let current =
        store.get_document::<WorkActionBindings>("work_action_bindings", &origin.work_id)?;
    let mut bindings =
        current
            .as_ref()
            .map(|entry| entry.value.clone())
            .unwrap_or(WorkActionBindings {
                schema_version: 1,
                project: origin.project.clone(),
                spec_hashes: vec![],
            });
    if !bindings.spec_hashes.contains(&origin.spec_hash) {
        if bindings.spec_hashes.len() >= 32 {
            return Err(OperatorStoreError::Conflict("work action capacity reached"));
        }
        bindings.spec_hashes.push(origin.spec_hash.clone());
    }
    store.put_document("work_action_origin", &origin.spec_hash, &origin, None)?;
    store.put_document(
        "work_action_bindings",
        &origin.work_id,
        &bindings,
        current.map(|entry| entry.revision),
    )?;
    Ok(())
}

fn validate_action_origin(
    store: &OperatorStore,
    origin: &WorkActionOrigin,
) -> Result<(), OperatorStoreError> {
    let work = store
        .get_work_item(&origin.work_id)?
        .ok_or(OperatorStoreError::Conflict("work unavailable"))?;
    if work.revision != origin.work_revision
        || work.value.project != origin.project
        || work.value.current_artifact_id.as_deref() != Some(&origin.artifact_id)
        || work.value.current_artifact_revision != Some(origin.artifact_revision)
    {
        return Err(OperatorStoreError::Conflict(
            "work or report changed; prepare a new action",
        ));
    }
    Ok(())
}

pub fn validate_resource_origin(
    store: &OperatorStore,
    hash: &str,
) -> Result<(), OperatorStoreError> {
    if let Some(origin) = store.get_document::<WorkActionOrigin>("work_action_origin", hash)? {
        validate_action_origin(store, &origin.value)?;
    }
    Ok(())
}

#[cfg(test)]
mod resource_origin_tests {
    use super::*;

    #[test]
    fn registered_target_is_separate_from_work_and_stale_origin_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "origin-test".into(),
            canonical_root: dir
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
        ))
        .unwrap();
        let mut work: refact_buddy_core::operator_types::WorkItem =
            serde_json::from_value(fixture["work_item"].clone()).unwrap();
        let mut artifact: ArtifactRevision =
            serde_json::from_value(fixture["artifact_revision"].clone()).unwrap();
        work.project = project.clone();
        artifact.project = project.clone();
        work.current_artifact_id = Some(artifact.artifact_id.clone());
        work.current_artifact_revision = Some(artifact.revision);
        let stored = store.upsert_work_item(&work, None).unwrap();
        store.append_artifact_revision(&artifact).unwrap();
        let mut spec: ActionSpec = serde_json::from_value(fixture["action_spec"].clone()).unwrap();
        spec.project = project.clone();
        spec.resource.resource_id = "skills/scoped-change".into();
        spec.authority_class = AuthorityClass::Mutating;
        let hash = action_spec_hash(&spec, &spec.parameter_ref);
        let prepared = PreparedAction {
            spec: spec.clone(),
            spec_hash: hash.clone(),
            bound_parameter_ref: spec.parameter_ref.clone(),
        };
        let document = super::super::resource_execution::ResourceDocument {
            schema_version: 1,
            project: project.clone(),
            payload: prepared,
        };
        store
            .put_document("prepared_action", &hash, &document, None)
            .unwrap();
        let origin = WorkActionOrigin {
            schema_version: 1,
            project: project.clone(),
            work_id: work.id.clone(),
            work_revision: stored.revision,
            artifact_id: artifact.artifact_id.clone(),
            artifact_revision: artifact.revision,
            spec_hash: hash.clone(),
        };
        attach_resource_action(&mut store, origin.clone()).unwrap();
        attach_resource_action(&mut store, origin).unwrap();
        let actions = actions_for_work(&store, &work.id).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[1].work_id, work.id);
        assert_ne!(actions[1].spec.resource.resource_id, work.id);
        assert!(validate_resource_origin(&store, &hash).is_ok());
        work.title.push_str(" changed");
        store
            .upsert_work_item(&work, Some(stored.revision))
            .unwrap();
        assert!(validate_resource_origin(&store, &hash).is_err());
        assert_eq!(actions_for_work(&store, &work.id).unwrap().len(), 1);
    }
}

fn check_links(
    store: &OperatorStore,
    work_id: &str,
    artifact: &ArtifactRevision,
) -> Result<(bool, Vec<String>), OperatorStoreError> {
    let mut matched_entries = vec![false; artifact.evidence.len()];
    let mut retained = 0;
    for id in &artifact.source_run_ids {
        if let Some(run) = store.get_run_outcome(id)? {
            run.value
                .validate()
                .map_err(|_| OperatorStoreError::InvalidInput("invalid source outcome"))?;
            if run.value.project == artifact.project && run.value.work_id == work_id {
                retained += 1;
                for (index, entry) in artifact.evidence.iter().enumerate() {
                    matched_entries[index] |= run.value.evidence.contains(entry);
                }
            }
        }
    }
    let matched = matched_entries
        .into_iter()
        .filter(|matched| *matched)
        .count();
    let provenance_matches = artifact.provenance.source_run_ids == artifact.source_run_ids
        && artifact.provenance.evidence_refs
            == artifact
                .evidence
                .iter()
                .map(|entry| entry.evidence_id.clone())
                .collect::<Vec<_>>();
    let complete = retained > 0
        && retained == artifact.source_run_ids.len()
        && !artifact.evidence.is_empty()
        && matched == artifact.evidence.len()
        && provenance_matches;
    Ok((complete, vec![
        format!("Retained source outcomes: {retained}/{}.", artifact.source_run_ids.len()),
        format!("Evidence metadata matches: {matched}/{} (including recorded digests, not live file hashes).", artifact.evidence.len()),
        format!("Provenance references match: {}.", if provenance_matches { "yes" } else { "no" }),
        "Findings are not independently verified. Source files and URLs were not opened; no fix was applied.".into(),
    ]))
}

pub fn execute_action(
    store: &mut OperatorStore,
    worker: WorkerIdentity,
    work_id: &str,
    action_id: &str,
    request: DeskActionRequest,
) -> Result<Stored<ActionExecution>, OperatorStoreError> {
    if request.project_id != store.identity().project_id {
        return Err(OperatorStoreError::ProjectMismatch);
    }
    if request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 128
        || !request
            .idempotency_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(OperatorStoreError::InvalidInput("invalid idempotency key"));
    }
    let action = actions_for_work(store, work_id)?
        .into_iter()
        .find(|action| action.spec.action_id == action_id)
        .ok_or(OperatorStoreError::InvalidInput(
            "action is not registered for this work item",
        ))?;
    if action.work_revision != request.work_revision
        || action.artifact_id != request.artifact_id
        || action.artifact_revision != request.artifact_revision
        || action.spec_hash != request.spec_hash
    {
        return Err(OperatorStoreError::Conflict(
            "work or report changed; reload actions before executing",
        ));
    }
    if action.spec.availability != ActionAvailability::Available {
        return Err(OperatorStoreError::Conflict("action is unavailable"));
    }
    let artifact = store
        .get_artifact_revision(&action.artifact_id, Some(action.artifact_revision))?
        .ok_or(OperatorStoreError::Conflict("report is missing"))?
        .value;
    let (complete, observations) = check_links(store, work_id, &artifact)?;
    let adapter = EvidenceLinksAdapter {
        spec: action.spec.clone(),
        complete,
        observations,
    };
    let execution_id = format!(
        "desk:{}",
        hex::encode(Sha256::digest(request.idempotency_key.as_bytes()))
    );
    let request = ActionRequest {
        execution_id,
        idempotency_key: request.idempotency_key,
        spec: action.spec,
        bound_parameter_ref: None,
        requester: requester(),
        decision: None,
        worker,
    };
    let result = {
        let mut service = ActionService::new(
            OperatorStoreRepository::new(&mut *store),
            Arc::new(EvidenceLinksAuthorizer),
        );
        service
            .register_adapter(Arc::new(adapter))
            .map_err(|_| OperatorStoreError::Conflict("action registration failed"))?;
        service.execute(&request)
    };
    match result {
        ActionResult::Finished(record) | ActionResult::Duplicate(record) => {
            store.get_execution(&record.execution.execution_id)?.ok_or(
                OperatorStoreError::Conflict("execution result was not persisted"),
            )
        }
        ActionResult::Rejected(_) => Err(OperatorStoreError::Conflict(
            "action rejected; reload its execution status before retrying",
        )),
    }
}
