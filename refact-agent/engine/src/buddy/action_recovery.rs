//! Explicit recovery and verification for completed or partial action effects.
//!
//! This is deliberately not a generic undo facility.  A recovery request is bound to the
//! original `ActionSpec` hash, resource, authority, and a resource-kind-specific adapter.
//! Recovery material is referenced, never serialized here; a separate provider owns any
//! short-lived preimage or credential-bearing token.

use std::collections::HashMap;
use std::sync::Arc;

use refact_buddy_core::operator_types::{
    ActionExecution, ActionExecutionState, ActionSpec, DecisionReference, NativeSourceIdentity,
    RecoveryRef, WorkerIdentity,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::action_execution::{
    action_spec_hash, ActionRepository, Authorizer, ClaimResult, ExecutionRecord,
};
use super::operator_store::{OperatorStore, Stored};

/// A request must carry the original reviewed spec.  The service verifies that it is exactly
/// the spec (including immutable bound parameter reference) used for the original execution;
/// callers never provide a new target or authority class for recovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryRequest {
    pub execution_id: String,
    pub idempotency_key: String,
    pub original_execution_id: String,
    pub original_spec: ActionSpec,
    pub original_bound_parameter_ref: Option<String>,
    pub recovery_id: String,
    pub expected_post_version: String,
    pub requester: NativeSourceIdentity,
    pub decision: Option<DecisionReference>,
    pub worker: WorkerIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryReceipt {
    pub receipt: ExecutionRecord,
    pub original_execution_id: String,
    pub recovery_id: String,
    pub restored_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryResult {
    Rejected(RecoveryRejection),
    Duplicate(RecoveryReceipt),
    Finished(RecoveryReceipt),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryRejection {
    OriginalNotFound,
    OriginalNotRecoverable,
    OriginalSpecDoesNotMatch,
    RecoveryIdentityDoesNotMatch,
    RecoveryUnavailable { reason: String },
    AdapterNotRegistered,
    ResourceKindNotSupported,
    AuthorizationDenied,
    CurrentVersionMismatch,
    RecoveryFailed { effects_may_have_occurred: bool },
    VerificationFailed,
    IdempotencyConflict,
    RepositoryFailure,
}

/// Material expiry is intentionally observable.  It never deletes or alters the original
/// execution record, and the service never invents a preimage representation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryMaterial {
    Available { reference: String },
    Unavailable { reason: String },
}

pub trait RecoveryMaterialProvider: Send + Sync {
    fn material_for(&self, original: &ActionExecution, recovery: &RecoveryRef) -> RecoveryMaterial;
}

pub trait RecoveryAdapter: Send + Sync {
    fn adapter_id(&self) -> &str;
    fn supports_resource_kind(&self, resource_kind: &str) -> bool;
    fn inspect_current_version(&self, spec: &ActionSpec) -> Result<String, ()>;
    fn recover(
        &self,
        original: &ActionSpec,
        recovery: &RecoveryRef,
        material_reference: &str,
    ) -> Result<(), RecoveryApplyFailure>;
    fn verify_restored_version(
        &self,
        spec: &ActionSpec,
        expected_version: &str,
    ) -> Result<bool, ()>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryApplyFailure {
    /// If true, recovery may have changed the target.  The receipt is retained as
    /// `outcome_unknown`, rather than claiming an undo completed.
    pub effects_may_have_occurred: bool,
}

/// Reads the original execution and writes recovery receipts.  Implementations must make
/// `claim` durable before an adapter call; it is an intent receipt, not exactly-once effects.
pub trait RecoveryRepository: ActionRepository {
    fn original(&self, execution_id: &str) -> Result<Option<ExecutionRecord>, ()>;
}

/// Store adapter for recovery.  It intentionally uses a fresh `ActionExecution` receipt; the
/// original is never overwritten or removed.
pub struct OperatorStoreRecoveryRepository {
    store: OperatorStore,
}

impl OperatorStoreRecoveryRepository {
    pub fn new(store: OperatorStore) -> Self {
        Self {
            store,
        }
    }

    pub fn into_store(self) -> OperatorStore {
        self.store
    }

    fn record(stored: Stored<ActionExecution>) -> ExecutionRecord {
        ExecutionRecord {
            effects: stored.value.effects.clone(),
            verification_evidence: stored.value.verification_evidence.clone(),
            recovery: stored.value.recovery.clone(),
            execution: stored.value,
        }
    }
}

impl ActionRepository for OperatorStoreRecoveryRepository {
    fn lookup(&mut self, key: &str) -> Result<Option<ExecutionRecord>, ()> {
        self.store
            .get_execution_by_idempotency_key(key)
            .map(|v| v.map(Self::record))
            .map_err(|_| ())
    }
    fn claim(&mut self, record: ExecutionRecord) -> Result<ClaimResult, ()> {
        match self
            .store
            .claim_completion_execution(&record.execution)
            .map_err(|_| ())?
        {
            super::operator_store::ExecutionClaim::Claimed(_) => Ok(ClaimResult::Claimed),
            super::operator_store::ExecutionClaim::Existing(existing) => {
                Ok(ClaimResult::Existing(Self::record(existing)))
            }
        }
    }

    fn transition(
        &mut self,
        execution_id: &str,
        expected_status: ActionExecutionState,
        status: ActionExecutionState,
    ) -> Result<(), ()> {
        let stored = self
            .store
            .get_execution(execution_id)
            .map_err(|_| ())?
            .ok_or(())?;
        if stored.value.status != expected_status {
            return Err(());
        }
        let mut value = stored.value;
        value.status = status;
        self.store
            .replace_execution(&value, stored.revision)
            .map_err(|_| ())?;
        Ok(())
    }

    fn finish(&mut self, record: ExecutionRecord) -> Result<(), ()> {
        let stored = self
            .store
            .get_execution(&record.execution.execution_id)
            .map_err(|_| ())?
            .ok_or(())?;
        if stored.value.status != ActionExecutionState::Running {
            return Err(());
        }
        let mut execution = record.execution;
        execution.effects = record.effects;
        execution.verification_evidence = record.verification_evidence;
        execution.recovery = record.recovery;
        self.store
            .replace_execution(&execution, stored.revision)
            .map_err(|_| ())?;
        Ok(())
    }
}

impl RecoveryRepository for OperatorStoreRecoveryRepository {
    fn original(&self, execution_id: &str) -> Result<Option<ExecutionRecord>, ()> {
        Ok(self
            .store
            .get_execution(execution_id)
            .map_err(|_| ())?
            .map(Self::record))
    }
}

pub struct RecoveryService<R> {
    repository: R,
    adapters: HashMap<String, Arc<dyn RecoveryAdapter>>,
    authorizer: Arc<dyn Authorizer>,
    material_provider: Arc<dyn RecoveryMaterialProvider>,
}

impl<R: RecoveryRepository> RecoveryService<R> {
    pub fn new(
        repository: R,
        authorizer: Arc<dyn Authorizer>,
        material_provider: Arc<dyn RecoveryMaterialProvider>,
    ) -> Self {
        Self {
            repository,
            adapters: HashMap::new(),
            authorizer,
            material_provider,
        }
    }

    pub fn register_adapter(&mut self, adapter: Arc<dyn RecoveryAdapter>) -> Result<(), ()> {
        let id = adapter.adapter_id().to_owned();
        if id.trim().is_empty() || self.adapters.contains_key(&id) {
            return Err(());
        }
        self.adapters.insert(id, adapter);
        Ok(())
    }

    pub fn recover(&mut self, request: &RecoveryRequest) -> RecoveryResult {
        let original = match self.repository.original(&request.original_execution_id) {
            Ok(Some(original)) => original,
            Ok(None) => return RecoveryResult::Rejected(RecoveryRejection::OriginalNotFound),
            Err(()) => return RecoveryResult::Rejected(RecoveryRejection::RepositoryFailure),
        };
        let recovery = match verified_recovery(&original.execution, request) {
            Ok(recovery) => recovery,
            Err(rejection) => return RecoveryResult::Rejected(rejection),
        };
        if action_spec_hash(
            &request.original_spec,
            &request.original_bound_parameter_ref,
        ) != original.execution.spec_hash
            || request.original_spec.parameter_ref != request.original_bound_parameter_ref
            || request.original_spec.action_id != original.execution.action_id
            || request.original_spec.project != original.execution.project
        {
            return RecoveryResult::Rejected(RecoveryRejection::OriginalSpecDoesNotMatch);
        }
        if !self.authorizer.authorize(
            &request.original_spec,
            &request.requester,
            request.decision.as_ref(),
        ) {
            return RecoveryResult::Rejected(RecoveryRejection::AuthorizationDenied);
        }
        let adapter = match self.adapters.get(&request.original_spec.adapter) {
            Some(adapter)
                if adapter
                    .supports_resource_kind(&request.original_spec.resource.resource_kind) =>
            {
                Arc::clone(adapter)
            }
            Some(_) => {
                return RecoveryResult::Rejected(RecoveryRejection::ResourceKindNotSupported)
            }
            None => return RecoveryResult::Rejected(RecoveryRejection::AdapterNotRegistered),
        };
        match self.repository.lookup(&request.idempotency_key) {
            Ok(Some(existing)) => {
                if existing.execution.spec_hash
                    != recovery_request_hash(request, &original.execution.spec_hash)
                {
                    return RecoveryResult::Rejected(RecoveryRejection::IdempotencyConflict);
                }
                let restored = (existing.execution.status == ActionExecutionState::Completed)
                    .then(|| request.original_spec.expected_version.clone());
                return RecoveryResult::Duplicate(receipt(existing, request, restored));
            }
            Ok(None) => {}
            Err(()) => return RecoveryResult::Rejected(RecoveryRejection::RepositoryFailure),
        }
        let material = match self
            .material_provider
            .material_for(&original.execution, recovery)
        {
            RecoveryMaterial::Available {
                reference,
            } if !reference.trim().is_empty() => reference,
            RecoveryMaterial::Available {
                ..
            } => {
                return RecoveryResult::Rejected(RecoveryRejection::RecoveryUnavailable {
                    reason: "recovery material reference is empty".into(),
                })
            }
            RecoveryMaterial::Unavailable {
                reason,
            } => {
                return RecoveryResult::Rejected(RecoveryRejection::RecoveryUnavailable {
                    reason,
                })
            }
        };
        match adapter.inspect_current_version(&request.original_spec) {
            Ok(version) if version == recovery.expected_post_version => {}
            Ok(_) => return RecoveryResult::Rejected(RecoveryRejection::CurrentVersionMismatch),
            Err(()) => return RecoveryResult::Rejected(RecoveryRejection::RepositoryFailure),
        }

        let claimed = ExecutionRecord {
            execution: recovery_execution(request, &original.execution),
            effects: Vec::new(),
            verification_evidence: Vec::new(),
            recovery: None,
        };
        match self.repository.claim(claimed.clone()) {
            Ok(ClaimResult::Existing(existing)) => {
                return RecoveryResult::Duplicate(receipt(existing, request, None))
            }
            Ok(ClaimResult::Claimed) => {}
            Err(()) => return RecoveryResult::Rejected(RecoveryRejection::IdempotencyConflict),
        }
        if self
            .repository
            .transition(
                &claimed.execution.execution_id,
                ActionExecutionState::Claimed,
                ActionExecutionState::Running,
            )
            .is_err()
        {
            return RecoveryResult::Rejected(RecoveryRejection::RepositoryFailure);
        }

        let mut finished = claimed;
        let outcome = match adapter.recover(&request.original_spec, recovery, &material) {
            Ok(()) => match adapter.verify_restored_version(
                &request.original_spec,
                &request.original_spec.expected_version,
            ) {
                Ok(true) => {
                    finished.execution.status = ActionExecutionState::Completed;
                    finished
                        .verification_evidence
                        .push("recovery_restored_version_verified".into());
                    Ok(Some(request.original_spec.expected_version.clone()))
                }
                Ok(false) | Err(()) => {
                    finished.execution.status = ActionExecutionState::Partial;
                    finished
                        .verification_evidence
                        .push("recovery_restore_not_verified".into());
                    Err(RecoveryRejection::VerificationFailed)
                }
            },
            Err(failure) => {
                finished.execution.status = if failure.effects_may_have_occurred {
                    ActionExecutionState::OutcomeUnknown
                } else {
                    ActionExecutionState::Failed
                };
                Err(RecoveryRejection::RecoveryFailed {
                    effects_may_have_occurred: failure.effects_may_have_occurred,
                })
            }
        };
        finished.execution.ended_at = Some(chrono::Utc::now().to_rfc3339());
        if outcome.is_ok() {
            let mut resource = request.original_spec.resource.clone();
            resource.version = request.original_spec.expected_version.clone();
            finished
                .effects
                .push(refact_buddy_core::operator_types::ResourceEffect {
                    resource,
                    state: refact_buddy_core::operator_types::EffectState::Applied,
                    evidence_refs: finished.verification_evidence.clone(),
                });
        }
        finished.execution.effects = finished.effects.clone();
        finished.execution.verification_evidence = finished.verification_evidence.clone();
        finished.execution.recovery = finished.recovery.clone();
        // Always persist the terminal receipt.  The return value remains truthful even if the
        // adapter failed or verification was inconclusive.
        if self.repository.finish(finished.clone()).is_err() {
            return RecoveryResult::Rejected(RecoveryRejection::RepositoryFailure);
        }
        // Like action execution, a terminal receipt is returned even when the effect failed or
        // verification was inconclusive. Its state is the source of truth; `Rejected` is only
        // for requests that were never durably claimed.
        RecoveryResult::Finished(receipt(finished, request, outcome.ok().flatten()))
    }

    pub fn into_repository(self) -> R {
        self.repository
    }
}

pub(crate) fn verified_recovery<'a>(
    original: &'a ActionExecution,
    request: &RecoveryRequest,
) -> Result<&'a RecoveryRef, RecoveryRejection> {
    if !matches!(
        original.status,
        ActionExecutionState::Completed | ActionExecutionState::Partial
    ) {
        return Err(RecoveryRejection::OriginalNotRecoverable);
    }
    let recovery = original
        .recovery
        .as_ref()
        .ok_or(RecoveryRejection::OriginalNotRecoverable)?;
    if !recovery.available {
        return Err(RecoveryRejection::RecoveryUnavailable {
            reason: "original recovery is unavailable".into(),
        });
    }
    if recovery.recovery_id != request.recovery_id
        || recovery.action_id != original.action_id
        || recovery.expected_post_version != request.expected_post_version
    {
        return Err(RecoveryRejection::RecoveryIdentityDoesNotMatch);
    }
    Ok(recovery)
}

pub(crate) fn recovery_execution(
    request: &RecoveryRequest,
    original: &ActionExecution,
) -> ActionExecution {
    ActionExecution {
        schema_version: original.schema_version,
        execution_id: request.execution_id.clone(),
        project: original.project.clone(),
        worker: request.worker.clone(),
        idempotency_key: request.idempotency_key.clone(),
        action_id: format!("recovery:{}:{}", original.execution_id, request.recovery_id),
        spec_hash: recovery_request_hash(request, &original.spec_hash),
        requester: request.requester.clone(),
        decision: request.decision.clone(),
        status: ActionExecutionState::Claimed,
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        ended_at: None,
        effects: Vec::new(),
        verification_evidence: Vec::new(),
        recovery: None,
    }
}

fn receipt(
    record: ExecutionRecord,
    request: &RecoveryRequest,
    restored_version: Option<String>,
) -> RecoveryReceipt {
    RecoveryReceipt {
        receipt: record,
        original_execution_id: request.original_execution_id.clone(),
        recovery_id: request.recovery_id.clone(),
        restored_version,
    }
}

pub(crate) fn recovery_request_hash(request: &RecoveryRequest, original_spec_hash: &str) -> String {
    #[derive(Serialize)]
    struct Immutable<'a> {
        original_execution_id: &'a str,
        original_spec_hash: &'a str,
        recovery_id: &'a str,
        expected_post_version: &'a str,
        requester: &'a NativeSourceIdentity,
        decision: &'a Option<DecisionReference>,
    }
    let immutable = Immutable {
        original_execution_id: &request.original_execution_id,
        original_spec_hash,
        recovery_id: &request.recovery_id,
        expected_post_version: &request.expected_post_version,
        requester: &request.requester,
        decision: &request.decision,
    };
    Sha256::digest(serde_json::to_vec(&immutable).unwrap_or_default())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_buddy_core::operator_types::{
        ActionAvailability, ActionCapabilities, AuthorityClass, ConfirmationClass,
        NativeSourceKind, ProjectIdentity, ResourceRef, OPERATOR_SCHEMA_VERSION_V1,
    };
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    #[derive(Default)]
    struct MemoryRepository {
        records: HashMap<String, ExecutionRecord>,
    }
    impl ActionRepository for MemoryRepository {
        fn lookup(&mut self, key: &str) -> Result<Option<ExecutionRecord>, ()> {
            Ok(self.records.get(key).cloned())
        }
        fn claim(&mut self, record: ExecutionRecord) -> Result<ClaimResult, ()> {
            match self.records.get(&record.execution.idempotency_key) {
                Some(old) if old.execution.spec_hash == record.execution.spec_hash => {
                    Ok(ClaimResult::Existing(old.clone()))
                }
                Some(_) => Err(()),
                None => {
                    self.records
                        .insert(record.execution.idempotency_key.clone(), record);
                    Ok(ClaimResult::Claimed)
                }
            }
        }
        fn transition(
            &mut self,
            id: &str,
            expected: ActionExecutionState,
            status: ActionExecutionState,
        ) -> Result<(), ()> {
            let record = self
                .records
                .values_mut()
                .find(|r| r.execution.execution_id == id)
                .ok_or(())?;
            if record.execution.status != expected {
                return Err(());
            }
            record.execution.status = status;
            Ok(())
        }
        fn finish(&mut self, record: ExecutionRecord) -> Result<(), ()> {
            self.records
                .insert(record.execution.idempotency_key.clone(), record);
            Ok(())
        }
    }
    impl RecoveryRepository for MemoryRepository {
        fn original(&self, id: &str) -> Result<Option<ExecutionRecord>, ()> {
            Ok(self
                .records
                .values()
                .find(|r| r.execution.execution_id == id)
                .cloned())
        }
    }
    struct Allow(bool);
    impl Authorizer for Allow {
        fn authorize(
            &self,
            _: &ActionSpec,
            _: &NativeSourceIdentity,
            _: Option<&DecisionReference>,
        ) -> bool {
            self.0
        }
    }
    struct Material(bool);
    impl RecoveryMaterialProvider for Material {
        fn material_for(&self, _: &ActionExecution, _: &RecoveryRef) -> RecoveryMaterial {
            if self.0 {
                RecoveryMaterial::Available {
                    reference: "sealed-ref".into(),
                }
            } else {
                RecoveryMaterial::Unavailable {
                    reason: "expired".into(),
                }
            }
        }
    }
    struct LocalAdapter {
        version: String,
        recoveries: AtomicUsize,
        verifies: bool,
        fails_unknown: bool,
    }
    impl RecoveryAdapter for LocalAdapter {
        fn adapter_id(&self) -> &str {
            "local"
        }
        fn supports_resource_kind(&self, kind: &str) -> bool {
            kind == "local"
        }
        fn inspect_current_version(&self, _: &ActionSpec) -> Result<String, ()> {
            Ok(self.version.clone())
        }
        fn recover(
            &self,
            _: &ActionSpec,
            _: &RecoveryRef,
            _: &str,
        ) -> Result<(), RecoveryApplyFailure> {
            self.recoveries.fetch_add(1, Ordering::SeqCst);
            if self.fails_unknown {
                Err(RecoveryApplyFailure {
                    effects_may_have_occurred: true,
                })
            } else {
                Ok(())
            }
        }
        fn verify_restored_version(&self, _: &ActionSpec, _: &str) -> Result<bool, ()> {
            Ok(self.verifies)
        }
    }
    fn project(root: Option<&std::path::Path>) -> ProjectIdentity {
        ProjectIdentity {
            project_id: "p".into(),
            canonical_root: root
                .map(|r| {
                    dunce::canonicalize(r)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .unwrap_or_else(|| "/p".into()),
        }
    }
    fn source() -> NativeSourceIdentity {
        NativeSourceIdentity {
            source_kind: NativeSourceKind::Chat,
            source_id: "chat".into(),
            source_version: "1".into(),
        }
    }
    fn spec(project: ProjectIdentity) -> ActionSpec {
        ActionSpec {
            schema_version: OPERATOR_SCHEMA_VERSION_V1,
            action_id: "act".into(),
            kind: "local-change".into(),
            adapter: "local".into(),
            project,
            resource: ResourceRef {
                resource_kind: "local".into(),
                resource_id: "target".into(),
                version: "1".into(),
            },
            expected_version: "1".into(),
            parameter_ref: Some("immutable-params".into()),
            availability: ActionAvailability::Available,
            confirmation: ConfirmationClass::UserConfirmationRequired,
            authority_class: AuthorityClass::Mutating,
            capabilities: ActionCapabilities {
                can_preview: true,
                can_execute: true,
                can_verify: true,
                can_recover: true,
                can_cancel: false,
            },
        }
    }
    fn original(
        project: ProjectIdentity,
        status: ActionExecutionState,
    ) -> (ExecutionRecord, ActionSpec) {
        let spec = spec(project);
        let hash = action_spec_hash(&spec, &spec.parameter_ref);
        (
            ExecutionRecord {
                execution: ActionExecution {
                    schema_version: OPERATOR_SCHEMA_VERSION_V1,
                    execution_id: "original".into(),
                    project: spec.project.clone(),
                    worker: WorkerIdentity {
                        worker_id: "w".into(),
                        engine_instance_id: "e".into(),
                        epoch: 1,
                    },
                    idempotency_key: "original-key".into(),
                    action_id: spec.action_id.clone(),
                    spec_hash: hash,
                    requester: source(),
                    decision: None,
                    status,
                    started_at: None,
                    ended_at: None,
                    effects: vec![],
                    verification_evidence: vec![],
                    recovery: Some(RecoveryRef {
                        recovery_id: "recover".into(),
                        action_id: "act".into(),
                        expected_post_version: "2".into(),
                        available: true,
                    }),
                },
                effects: vec![],
                verification_evidence: vec![],
                recovery: Some(RecoveryRef {
                    recovery_id: "recover".into(),
                    action_id: "act".into(),
                    expected_post_version: "2".into(),
                    available: true,
                }),
            },
            spec,
        )
    }
    fn request(spec: ActionSpec) -> RecoveryRequest {
        RecoveryRequest {
            execution_id: "recovery-execution".into(),
            idempotency_key: "recovery-key".into(),
            original_execution_id: "original".into(),
            original_bound_parameter_ref: spec.parameter_ref.clone(),
            original_spec: spec,
            recovery_id: "recover".into(),
            expected_post_version: "2".into(),
            requester: source(),
            decision: None,
            worker: WorkerIdentity {
                worker_id: "w2".into(),
                engine_instance_id: "e".into(),
                epoch: 2,
            },
        }
    }
    fn memory_service(
        original: ExecutionRecord,
        adapter: Arc<LocalAdapter>,
        material: bool,
    ) -> RecoveryService<MemoryRepository> {
        let mut records = HashMap::new();
        records.insert(original.execution.idempotency_key.clone(), original);
        let mut service = RecoveryService::new(
            MemoryRepository {
                records,
            },
            Arc::new(Allow(true)),
            Arc::new(Material(material)),
        );
        service.register_adapter(adapter).unwrap();
        service
    }
    #[test]
    fn recovery_is_bound_to_original_spec_and_exact_post_version() {
        let (original, spec) = original(project(None), ActionExecutionState::Partial);
        let adapter = Arc::new(LocalAdapter {
            version: "2".into(),
            recoveries: AtomicUsize::new(0),
            verifies: true,
            fails_unknown: false,
        });
        let mut service = memory_service(original, Arc::clone(&adapter), true);
        let mut bad = request(spec.clone());
        bad.original_spec.resource.resource_id = "other".into();
        assert!(matches!(
            service.recover(&bad),
            RecoveryResult::Rejected(RecoveryRejection::OriginalSpecDoesNotMatch)
        ));
        let mut bad_version = request(spec);
        bad_version.expected_post_version = "3".into();
        assert!(matches!(
            service.recover(&bad_version),
            RecoveryResult::Rejected(RecoveryRejection::RecoveryIdentityDoesNotMatch)
        ));
        assert_eq!(adapter.recoveries.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn expired_material_preserves_original_and_runs_no_effect() {
        let (original, spec) = original(project(None), ActionExecutionState::Completed);
        let adapter = Arc::new(LocalAdapter {
            version: "2".into(),
            recoveries: AtomicUsize::new(0),
            verifies: true,
            fails_unknown: false,
        });
        let mut service = memory_service(original, Arc::clone(&adapter), false);
        assert!(
            matches!(service.recover(&request(spec)), RecoveryResult::Rejected(RecoveryRejection::RecoveryUnavailable { reason }) if reason == "expired")
        );
        assert!(
            matches!(service.into_repository().original("original"), Ok(Some(record)) if record.execution.status == ActionExecutionState::Completed)
        );
        assert_eq!(adapter.recoveries.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn failure_that_may_have_effects_is_durable_unknown_not_false_undo() {
        let (original, spec) = original(project(None), ActionExecutionState::Partial);
        let adapter = Arc::new(LocalAdapter {
            version: "2".into(),
            recoveries: AtomicUsize::new(0),
            verifies: true,
            fails_unknown: true,
        });
        let mut service = memory_service(original, adapter, true);
        assert!(matches!(
            service.recover(&request(spec)),
            RecoveryResult::Finished(receipt)
                if receipt.receipt.execution.status == ActionExecutionState::OutcomeUnknown
        ));
        let repository = service.into_repository();
        assert!(
            matches!(repository.records.get("recovery-key"), Some(record) if record.execution.status == ActionExecutionState::OutcomeUnknown)
        );
        assert!(
            matches!(repository.original("original"), Ok(Some(record)) if record.execution.status == ActionExecutionState::Partial)
        );
    }
    #[test]
    fn real_store_receipt_is_idempotent_and_original_history_remains() {
        let temp = TempDir::new().unwrap();
        let project = project(Some(temp.path()));
        let (original, spec) = original(project.clone(), ActionExecutionState::Partial);
        let mut store = OperatorStore::open(temp.path(), &project).unwrap();
        store.claim_execution(&original.execution).unwrap();
        let adapter = Arc::new(LocalAdapter {
            version: "2".into(),
            recoveries: AtomicUsize::new(0),
            verifies: true,
            fails_unknown: false,
        });
        let mut service = RecoveryService::new(
            OperatorStoreRecoveryRepository::new(store),
            Arc::new(Allow(true)),
            Arc::new(Material(true)),
        );
        let recovery_adapter: Arc<dyn RecoveryAdapter> = adapter.clone();
        service.register_adapter(recovery_adapter).unwrap();
        let request = request(spec);
        assert!(
            matches!(service.recover(&request), RecoveryResult::Finished(receipt) if receipt.receipt.execution.status == ActionExecutionState::Completed)
        );
        assert!(
            matches!(service.recover(&request), RecoveryResult::Duplicate(receipt) if receipt.receipt.execution.status == ActionExecutionState::Completed)
        );
        assert_eq!(adapter.recoveries.load(Ordering::SeqCst), 1);
        let store = service.into_repository().into_store();
        assert_eq!(
            store
                .get_execution("original")
                .unwrap()
                .unwrap()
                .value
                .status,
            ActionExecutionState::Partial
        );
        assert_eq!(
            store
                .get_execution("recovery-execution")
                .unwrap()
                .unwrap()
                .value
                .status,
            ActionExecutionState::Completed
        );
    }
}
