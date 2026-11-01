use std::collections::HashMap;
use std::borrow::BorrowMut;
use std::sync::Arc;

use super::operator_store::{ExecutionClaim, OperatorStore, Stored};
use refact_buddy_core::operator_types::{
    ActionAvailability, ActionExecution, ActionExecutionState, ActionSpec, AuthorityClass,
    DecisionReference, NativeSourceIdentity, RecoveryRef, ResourceEffect, WorkerIdentity,
};
use serde::{Serialize, Deserialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRequest {
    pub execution_id: String,
    pub idempotency_key: String,
    pub spec: ActionSpec,
    pub bound_parameter_ref: Option<String>,
    pub requester: NativeSourceIdentity,
    pub decision: Option<DecisionReference>,
    pub worker: WorkerIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedAction {
    pub spec: ActionSpec,
    pub bound_parameter_ref: Option<String>,
    pub spec_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub execution: ActionExecution,
    pub effects: Vec<ResourceEffect>,
    pub verification_evidence: Vec<String>,
    pub recovery: Option<RecoveryRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionResult {
    Rejected(ActionRejection),
    Duplicate(ExecutionRecord),
    Finished(ExecutionRecord),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionRejection {
    AdapterNotRegistered,
    KindNotSupported,
    BoundParametersDoNotMatchSpec,
    Unavailable,
    Stale,
    Unsupported,
    AuthorizationDenied,
    VersionMismatch,
    IdempotencyConflict,
    RepositoryFailure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdapterVersion {
    Matches,
    Mismatch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyOutcome {
    CompletedWithRecovery {
        effects: Vec<ResourceEffect>,
        verification_evidence: Vec<String>,
        recovery: Option<RecoveryRef>,
    },
    Completed {
        effects: Vec<ResourceEffect>,
        verification_evidence: Vec<String>,
    },
    Partial {
        effects: Vec<ResourceEffect>,
        verification_evidence: Vec<String>,
        recovery: Option<RecoveryRef>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyFailure {
    pub effects_may_have_occurred: bool,
}

pub trait ActionAdapter: Send + Sync {
    fn adapter_id(&self) -> &str;
    fn supports_kind(&self, kind: &str) -> bool;
    fn expected_version(&self, spec: &ActionSpec) -> Result<AdapterVersion, ()>;
    fn preview(&self, prepared: &PreparedAction) -> Result<(), ()>;
    fn apply(&self, prepared: &PreparedAction) -> Result<ApplyOutcome, ApplyFailure>;
}

pub trait Authorizer: Send + Sync {
    fn authorize(
        &self,
        spec: &ActionSpec,
        requester: &NativeSourceIdentity,
        decision: Option<&DecisionReference>,
    ) -> bool;
}

pub trait ActionRepository {
    fn lookup(&mut self, _key: &str) -> Result<Option<ExecutionRecord>, ()> {
        Ok(None)
    }
    fn claim(&mut self, record: ExecutionRecord) -> Result<ClaimResult, ()>;
    fn transition(
        &mut self,
        execution_id: &str,
        expected_status: ActionExecutionState,
        status: ActionExecutionState,
    ) -> Result<(), ()>;
    fn finish(&mut self, record: ExecutionRecord) -> Result<(), ()>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimResult {
    Claimed,
    Existing(ExecutionRecord),
}

pub struct OperatorStoreRepository<S = OperatorStore> {
    store: S,
}

impl<S> OperatorStoreRepository<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    pub fn into_store(self) -> S {
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

impl<S: BorrowMut<OperatorStore>> ActionRepository for OperatorStoreRepository<S> {
    fn lookup(&mut self, key: &str) -> Result<Option<ExecutionRecord>, ()> {
        self.store
            .borrow_mut()
            .get_execution_by_idempotency_key(key)
            .map(|v| v.map(Self::record))
            .map_err(|_| ())
    }
    fn claim(&mut self, record: ExecutionRecord) -> Result<ClaimResult, ()> {
        match self
            .store
            .borrow_mut()
            .claim_execution(&record.execution)
            .map_err(|_| ())?
        {
            ExecutionClaim::Claimed(_) => Ok(ClaimResult::Claimed),
            ExecutionClaim::Existing(stored) => Ok(ClaimResult::Existing(Self::record(stored))),
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
            .borrow_mut()
            .get_execution(execution_id)
            .map_err(|_| ())?
            .ok_or(())?;
        if stored.value.status != expected_status {
            return Err(());
        }
        let mut execution = stored.value;
        execution.status = status;
        self.store
            .borrow_mut()
            .replace_execution(&execution, stored.revision)
            .map_err(|_| ())?;
        Ok(())
    }

    fn finish(&mut self, record: ExecutionRecord) -> Result<(), ()> {
        let stored = self
            .store
            .borrow_mut()
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
            .borrow_mut()
            .replace_execution(&execution, stored.revision)
            .map_err(|_| ())?;
        Ok(())
    }
}

pub struct ActionService<R> {
    repository: R,
    adapters: HashMap<String, Arc<dyn ActionAdapter>>,
    async_adapters: HashMap<String, Arc<dyn AsyncActionAdapter>>,
    authorizer: Arc<dyn Authorizer>,
}

impl<R: ActionRepository> ActionService<R> {
    pub fn new(repository: R, authorizer: Arc<dyn Authorizer>) -> Self {
        Self {
            repository,
            adapters: HashMap::new(),
            async_adapters: HashMap::new(),
            authorizer,
        }
    }

    pub fn register_adapter(&mut self, adapter: Arc<dyn ActionAdapter>) -> Result<(), ()> {
        let id = adapter.adapter_id().to_owned();
        if id.trim().is_empty() || self.adapters.contains_key(&id) {
            return Err(());
        }
        self.adapters.insert(id, adapter);
        Ok(())
    }

    pub fn prepare(&self, request: &ActionRequest) -> Result<PreparedAction, ActionRejection> {
        if request.spec.parameter_ref != request.bound_parameter_ref {
            return Err(ActionRejection::BoundParametersDoNotMatchSpec);
        }
        let adapter = self
            .adapters
            .get(&request.spec.adapter)
            .ok_or(ActionRejection::AdapterNotRegistered)?;
        if !adapter.supports_kind(&request.spec.kind) {
            return Err(ActionRejection::KindNotSupported);
        }
        Ok(PreparedAction {
            spec: request.spec.clone(),
            bound_parameter_ref: request.bound_parameter_ref.clone(),
            spec_hash: action_spec_hash(&request.spec, &request.bound_parameter_ref),
        })
    }

    pub fn preview(&self, request: &ActionRequest) -> ActionResult {
        let prepared = match self.prepare(request) {
            Ok(prepared) => prepared,
            Err(rejection) => return ActionResult::Rejected(rejection),
        };
        if let Some(rejection) = declared_unavailable(&prepared.spec) {
            return ActionResult::Rejected(rejection);
        }
        if !prepared.spec.capabilities.can_preview {
            return ActionResult::Rejected(ActionRejection::Unsupported);
        }
        if !self.authorizer.authorize(
            &prepared.spec,
            &request.requester,
            request.decision.as_ref(),
        ) {
            return ActionResult::Rejected(ActionRejection::AuthorizationDenied);
        }
        let adapter = match self.adapters.get(&prepared.spec.adapter) {
            Some(adapter) => adapter,
            None => return ActionResult::Rejected(ActionRejection::AdapterNotRegistered),
        };
        if adapter.expected_version(&prepared.spec) != Ok(AdapterVersion::Matches) {
            return ActionResult::Rejected(ActionRejection::VersionMismatch);
        }
        if adapter.preview(&prepared).is_err() {
            return ActionResult::Rejected(ActionRejection::RepositoryFailure);
        }
        ActionResult::Finished(ExecutionRecord {
            execution: execution_for(request, &prepared, ActionExecutionState::Prepared),
            effects: Vec::new(),
            verification_evidence: Vec::new(),
            recovery: None,
        })
    }

    pub fn execute(&mut self, request: &ActionRequest) -> ActionResult {
        let prepared = match self.prepare(request) {
            Ok(prepared) => prepared,
            Err(rejection) => return ActionResult::Rejected(rejection),
        };
        if let Some(rejection) = declared_unavailable(&prepared.spec) {
            return ActionResult::Rejected(rejection);
        }
        if !prepared.spec.capabilities.can_execute {
            return ActionResult::Rejected(ActionRejection::Unsupported);
        }
        if !self.authorizer.authorize(
            &prepared.spec,
            &request.requester,
            request.decision.as_ref(),
        ) {
            return ActionResult::Rejected(ActionRejection::AuthorizationDenied);
        }
        let adapter = match self.adapters.get(&prepared.spec.adapter) {
            Some(adapter) => Arc::clone(adapter),
            None => return ActionResult::Rejected(ActionRejection::AdapterNotRegistered),
        };
        match self.repository.lookup(&request.idempotency_key) {
            Ok(Some(existing)) => return duplicate_result(existing, request, &prepared),
            Ok(None) => {}
            Err(()) => return ActionResult::Rejected(ActionRejection::RepositoryFailure),
        }
        if adapter.expected_version(&prepared.spec) != Ok(AdapterVersion::Matches) {
            return ActionResult::Rejected(ActionRejection::VersionMismatch);
        }
        let claimed = ExecutionRecord {
            execution: execution_for(request, &prepared, ActionExecutionState::Claimed),
            effects: Vec::new(),
            verification_evidence: Vec::new(),
            recovery: None,
        };
        match self.repository.claim(claimed.clone()) {
            Ok(ClaimResult::Existing(existing)) => {
                return duplicate_result(existing, request, &prepared)
            }
            Ok(ClaimResult::Claimed) => {}
            Err(()) => return ActionResult::Rejected(ActionRejection::IdempotencyConflict),
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
            return ActionResult::Rejected(ActionRejection::RepositoryFailure);
        }
        let mut finished = claimed;
        match adapter.apply(&prepared) {
            Ok(ApplyOutcome::CompletedWithRecovery {
                effects,
                verification_evidence,
                recovery,
            }) => {
                finished.execution.status = ActionExecutionState::Completed;
                finished.effects = effects;
                finished.verification_evidence = verification_evidence;
                finished.recovery = recovery;
            }
            Ok(ApplyOutcome::Completed {
                effects,
                verification_evidence,
            }) => {
                finished.execution.status = ActionExecutionState::Completed;
                finished.effects = effects;
                finished.verification_evidence = verification_evidence;
            }
            Ok(ApplyOutcome::Partial {
                effects,
                verification_evidence,
                recovery,
            }) => {
                finished.execution.status = ActionExecutionState::Partial;
                finished.effects = effects;
                finished.verification_evidence = verification_evidence;
                finished.recovery = recovery;
            }
            Err(ApplyFailure {
                effects_may_have_occurred: true,
            }) => {
                finished.execution.status = ActionExecutionState::OutcomeUnknown;
            }
            Err(ApplyFailure {
                effects_may_have_occurred: false,
            }) => {
                finished.execution.status = ActionExecutionState::Failed;
            }
        }
        finished.execution.ended_at = Some(chrono::Utc::now().to_rfc3339());
        finished.execution.effects = finished.effects.clone();
        finished.execution.verification_evidence = finished.verification_evidence.clone();
        finished.execution.recovery = finished.recovery.clone();
        if self.repository.finish(finished.clone()).is_err() {
            return ActionResult::Rejected(ActionRejection::RepositoryFailure);
        }
        ActionResult::Finished(finished)
    }

    pub fn into_repository(self) -> R {
        self.repository
    }
}

pub(crate) fn declared_unavailable(spec: &ActionSpec) -> Option<ActionRejection> {
    match &spec.availability {
        ActionAvailability::Available => None,
        ActionAvailability::Unavailable => Some(ActionRejection::Unavailable),
        ActionAvailability::Stale => Some(ActionRejection::Stale),
        ActionAvailability::Unsupported => Some(ActionRejection::Unsupported),
    }
}

pub(crate) fn execution_for(
    request: &ActionRequest,
    prepared: &PreparedAction,
    status: ActionExecutionState,
) -> ActionExecution {
    ActionExecution {
        schema_version: request.spec.schema_version,
        execution_id: request.execution_id.clone(),
        project: request.spec.project.clone(),
        worker: request.worker.clone(),
        idempotency_key: request.idempotency_key.clone(),
        action_id: request.spec.action_id.clone(),
        spec_hash: prepared.spec_hash.clone(),
        requester: request.requester.clone(),
        decision: request.decision.clone(),
        status: status.clone(),
        started_at: (status == ActionExecutionState::Claimed)
            .then(|| chrono::Utc::now().to_rfc3339()),
        ended_at: None,
        effects: Vec::new(),
        verification_evidence: Vec::new(),
        recovery: None,
    }
}

pub fn action_spec_hash(spec: &ActionSpec, bound_parameter_ref: &Option<String>) -> String {
    #[derive(Serialize)]
    struct Immutable<'a> {
        schema_version: u16,
        action_id: &'a str,
        kind: &'a str,
        adapter: &'a str,
        project: &'a refact_buddy_core::operator_types::ProjectIdentity,
        resource: &'a refact_buddy_core::operator_types::ResourceRef,
        expected_version: &'a str,
        parameter_ref: &'a Option<String>,
        bound_parameter_ref: &'a Option<String>,
        confirmation: &'a refact_buddy_core::operator_types::ConfirmationClass,
        authority_class: &'a AuthorityClass,
        capabilities: &'a refact_buddy_core::operator_types::ActionCapabilities,
    }
    let immutable = Immutable {
        schema_version: spec.schema_version,
        action_id: &spec.action_id,
        kind: &spec.kind,
        adapter: &spec.adapter,
        project: &spec.project,
        resource: &spec.resource,
        expected_version: &spec.expected_version,
        parameter_ref: &spec.parameter_ref,
        bound_parameter_ref,
        confirmation: &spec.confirmation,
        authority_class: &spec.authority_class,
        capabilities: &spec.capabilities,
    };
    let bytes = serde_json::to_vec(&immutable).unwrap_or_default();
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_buddy_core::operator_types::{
        ActionCapabilities, ConfirmationClass, EffectState, NativeSourceKind, ProjectIdentity,
        ResourceRef, OPERATOR_SCHEMA_VERSION_V1,
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
                Some(existing) if existing.execution.spec_hash == record.execution.spec_hash => {
                    Ok(ClaimResult::Existing(existing.clone()))
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
            execution_id: &str,
            expected_status: ActionExecutionState,
            status: ActionExecutionState,
        ) -> Result<(), ()> {
            let record = self
                .records
                .values_mut()
                .find(|record| record.execution.execution_id == execution_id)
                .ok_or(())?;
            if record.execution.status != expected_status {
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

    struct TestAuthorizer(bool);
    impl Authorizer for TestAuthorizer {
        fn authorize(
            &self,
            _: &ActionSpec,
            _: &NativeSourceIdentity,
            _: Option<&DecisionReference>,
        ) -> bool {
            self.0
        }
    }

    #[derive(Clone, Copy)]
    enum AdapterMode {
        Complete,
        Partial,
        Unknown,
    }

    struct TestAdapter {
        mode: AdapterMode,
        version: AdapterVersion,
        applies: AtomicUsize,
        previews: AtomicUsize,
    }
    impl TestAdapter {
        fn new(mode: AdapterMode, version: AdapterVersion) -> Self {
            Self {
                mode,
                version,
                applies: AtomicUsize::new(0),
                previews: AtomicUsize::new(0),
            }
        }
    }
    impl ActionAdapter for TestAdapter {
        fn adapter_id(&self) -> &str {
            "test"
        }
        fn supports_kind(&self, kind: &str) -> bool {
            kind == "test_kind"
        }
        fn expected_version(&self, _: &ActionSpec) -> Result<AdapterVersion, ()> {
            Ok(self.version.clone())
        }
        fn preview(&self, _: &PreparedAction) -> Result<(), ()> {
            self.previews.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn apply(&self, _: &PreparedAction) -> Result<ApplyOutcome, ApplyFailure> {
            self.applies.fetch_add(1, Ordering::SeqCst);
            let effect = ResourceEffect {
                resource: resource(),
                state: EffectState::Applied,
                evidence_refs: vec![],
            };
            match self.mode {
                AdapterMode::Complete => Ok(ApplyOutcome::Completed {
                    effects: vec![effect],
                    verification_evidence: vec!["verified".into()],
                }),
                AdapterMode::Partial => Ok(ApplyOutcome::Partial {
                    effects: vec![effect],
                    verification_evidence: vec![],
                    recovery: Some(RecoveryRef {
                        recovery_id: "recovery".into(),
                        action_id: "action".into(),
                        expected_post_version: "2".into(),
                        available: true,
                    }),
                }),
                AdapterMode::Unknown => Err(ApplyFailure {
                    effects_may_have_occurred: true,
                }),
            }
        }
    }

    fn project() -> ProjectIdentity {
        ProjectIdentity {
            project_id: "project".into(),
            canonical_root: "/project".into(),
        }
    }
    fn resource() -> ResourceRef {
        ResourceRef {
            resource_kind: "test".into(),
            resource_id: "resource".into(),
            version: "1".into(),
        }
    }
    fn source() -> NativeSourceIdentity {
        NativeSourceIdentity {
            source_kind: NativeSourceKind::Chat,
            source_id: "chat".into(),
            source_version: "1".into(),
        }
    }
    fn request() -> ActionRequest {
        ActionRequest {
            execution_id: "execution".into(),
            idempotency_key: "key".into(),
            spec: ActionSpec {
                schema_version: OPERATOR_SCHEMA_VERSION_V1,
                action_id: "action".into(),
                kind: "test_kind".into(),
                adapter: "test".into(),
                project: project(),
                resource: resource(),
                expected_version: "1".into(),
                parameter_ref: Some("params".into()),
                availability: ActionAvailability::Available,
                confirmation: ConfirmationClass::None,
                authority_class: AuthorityClass::Mutating,
                capabilities: ActionCapabilities {
                    can_preview: true,
                    can_execute: true,
                    can_verify: true,
                    can_recover: true,
                    can_cancel: false,
                },
            },
            bound_parameter_ref: Some("params".into()),
            requester: source(),
            decision: None,
            worker: WorkerIdentity {
                worker_id: "worker".into(),
                engine_instance_id: "engine".into(),
                epoch: 1,
            },
        }
    }
    fn store_project(root: &std::path::Path) -> ProjectIdentity {
        ProjectIdentity {
            project_id: "project".into(),
            canonical_root: dunce::canonicalize(root)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        }
    }

    fn store_request(project: ProjectIdentity) -> ActionRequest {
        let mut request = request();
        request.spec.project = project;
        request
    }

    fn service(adapter: Arc<TestAdapter>, authorized: bool) -> ActionService<MemoryRepository> {
        let mut service = ActionService::new(
            MemoryRepository::default(),
            Arc::new(TestAuthorizer(authorized)),
        );
        assert!(service.register_adapter(adapter).is_ok());
        service
    }

    #[test]
    fn preview_has_no_effects() {
        let adapter = Arc::new(TestAdapter::new(
            AdapterMode::Complete,
            AdapterVersion::Matches,
        ));
        let result = service(Arc::clone(&adapter), true).preview(&request());
        assert!(matches!(result, ActionResult::Finished(record) if record.effects.is_empty()));
        assert_eq!(adapter.applies.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.previews.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn disabled_capabilities_reject_before_adapter_or_claim() {
        let adapter = Arc::new(TestAdapter::new(
            AdapterMode::Complete,
            AdapterVersion::Matches,
        ));
        let mut service = service(adapter.clone(), true);
        let mut disabled = request();
        disabled.spec.capabilities.can_preview = false;
        disabled.spec.capabilities.can_execute = false;
        assert!(matches!(
            service.preview(&disabled),
            ActionResult::Rejected(ActionRejection::Unsupported)
        ));
        assert!(matches!(
            service.execute(&disabled),
            ActionResult::Rejected(ActionRejection::Unsupported)
        ));
        assert_eq!(adapter.applies.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.previews.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unavailable_authorization_and_stale_version_prevent_apply() {
        let adapter = Arc::new(TestAdapter::new(
            AdapterMode::Complete,
            AdapterVersion::Matches,
        ));
        let mut unavailable = request();
        unavailable.spec.availability = ActionAvailability::Unavailable;
        assert!(matches!(
            service(Arc::clone(&adapter), true).execute(&unavailable),
            ActionResult::Rejected(ActionRejection::Unavailable)
        ));
        assert!(matches!(
            service(Arc::clone(&adapter), false).execute(&request()),
            ActionResult::Rejected(ActionRejection::AuthorizationDenied)
        ));
        let stale_adapter = Arc::new(TestAdapter::new(
            AdapterMode::Complete,
            AdapterVersion::Mismatch,
        ));
        assert!(matches!(
            service(Arc::clone(&stale_adapter), true).execute(&request()),
            ActionResult::Rejected(ActionRejection::VersionMismatch)
        ));
        assert_eq!(adapter.applies.load(Ordering::SeqCst), 0);
        assert_eq!(stale_adapter.applies.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn duplicate_key_runs_adapter_once_and_hash_mismatch_rejects() {
        let adapter = Arc::new(TestAdapter::new(
            AdapterMode::Complete,
            AdapterVersion::Matches,
        ));
        let mut service = service(Arc::clone(&adapter), true);
        assert!(matches!(
            service.execute(&request()),
            ActionResult::Finished(_)
        ));
        assert!(matches!(
            service.execute(&request()),
            ActionResult::Duplicate(_)
        ));
        let mut changed = request();
        changed.bound_parameter_ref = Some("other".into());
        changed.spec.parameter_ref = Some("other".into());
        assert!(matches!(
            service.execute(&changed),
            ActionResult::Rejected(ActionRejection::IdempotencyConflict)
        ));
        assert_eq!(adapter.applies.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn real_store_persists_terminal_outcome_and_duplicate() {
        let temp = TempDir::new().unwrap();
        let project = store_project(temp.path());
        let adapter = Arc::new(TestAdapter::new(
            AdapterMode::Partial,
            AdapterVersion::Matches,
        ));
        let repository =
            OperatorStoreRepository::new(OperatorStore::open(temp.path(), &project).unwrap());
        let mut service = ActionService::new(repository, Arc::new(TestAuthorizer(true)));
        assert!(service.register_adapter(adapter.clone()).is_ok());
        let request = store_request(project.clone());
        assert!(
            matches!(service.execute(&request), ActionResult::Finished(record) if record.execution.status == ActionExecutionState::Partial)
        );
        assert!(
            matches!(service.execute(&request), ActionResult::Duplicate(record) if record.execution.status == ActionExecutionState::Partial && record.recovery.is_some())
        );
        assert_eq!(adapter.applies.load(Ordering::SeqCst), 1);
        let store = service.into_repository().into_store();
        let stored = store.get_execution("execution").unwrap().unwrap();
        assert_eq!(stored.value.status, ActionExecutionState::Partial);
        assert_eq!(stored.value.effects.len(), 1);
        assert!(stored.value.recovery.is_some());
        drop(store);
        let reopened = OperatorStore::open(temp.path(), &project).unwrap();
        assert_eq!(
            reopened
                .get_execution("execution")
                .unwrap()
                .unwrap()
                .value
                .status,
            ActionExecutionState::Partial
        );
    }

    #[test]
    fn real_store_retains_unknown_outcome_without_retry() {
        let temp = TempDir::new().unwrap();
        let project = store_project(temp.path());
        let adapter = Arc::new(TestAdapter::new(
            AdapterMode::Unknown,
            AdapterVersion::Matches,
        ));
        let repository =
            OperatorStoreRepository::new(OperatorStore::open(temp.path(), &project).unwrap());
        let mut service = ActionService::new(repository, Arc::new(TestAuthorizer(true)));
        assert!(service.register_adapter(adapter.clone()).is_ok());
        let request = store_request(project);
        assert!(
            matches!(service.execute(&request), ActionResult::Finished(record) if record.execution.status == ActionExecutionState::OutcomeUnknown)
        );
        assert!(
            matches!(service.execute(&request), ActionResult::Duplicate(record) if record.execution.status == ActionExecutionState::OutcomeUnknown)
        );
        assert_eq!(adapter.applies.load(Ordering::SeqCst), 1);
    }
}

fn duplicate_result(
    existing: ExecutionRecord,
    request: &ActionRequest,
    prepared: &PreparedAction,
) -> ActionResult {
    if existing.execution.spec_hash != prepared.spec_hash
        || existing.execution.project != prepared.spec.project
        || existing.execution.requester != request.requester
        || existing.execution.decision != request.decision
    {
        ActionResult::Rejected(ActionRejection::IdempotencyConflict)
    } else {
        ActionResult::Duplicate(existing)
    }
}

#[async_trait::async_trait]
pub trait AsyncActionAdapter: Send + Sync {
    fn adapter_id(&self) -> &str;
    fn supports_kind(&self, kind: &str) -> bool;
    async fn expected_version(&self, spec: &ActionSpec) -> Result<AdapterVersion, ()>;
    async fn preview(&self, prepared: &PreparedAction) -> Result<(), ()>;
    async fn apply(&self, prepared: &PreparedAction) -> Result<ApplyOutcome, ApplyFailure>;
    async fn apply_execution(
        &self,
        prepared: &PreparedAction,
        _execution_id: &str,
    ) -> Result<ApplyOutcome, ApplyFailure> {
        self.apply(prepared).await
    }
}

#[async_trait::async_trait]
pub trait AsyncActionRepository: Send + Sync {
    async fn lookup(&self, key: &str) -> Result<Option<ExecutionRecord>, ()>;
    async fn prepare(&self, prepared: PreparedAction) -> Result<(), ()>;
    async fn claim(&self, record: ExecutionRecord) -> Result<ClaimResult, ()>;
    async fn claim_prepared(
        &self,
        record: ExecutionRecord,
        _prepared: &PreparedAction,
    ) -> Result<ClaimResult, ()> {
        self.claim(record).await
    }
    async fn transition(
        &self,
        id: &str,
        expected: ActionExecutionState,
        status: ActionExecutionState,
    ) -> Result<(), ()>;
    async fn finish(&self, record: ExecutionRecord) -> Result<(), ()>;
}

impl<R: AsyncActionRepository> ActionService<R> {
    pub fn new_async(repository: R, authorizer: Arc<dyn Authorizer>) -> Self {
        Self {
            repository,
            adapters: HashMap::new(),
            async_adapters: HashMap::new(),
            authorizer,
        }
    }

    pub fn register_async_adapter(
        &mut self,
        adapter: Arc<dyn AsyncActionAdapter>,
    ) -> Result<(), ()> {
        let id = adapter.adapter_id().to_owned();
        if id.trim().is_empty() || self.async_adapters.contains_key(&id) {
            return Err(());
        }
        self.async_adapters.insert(id, adapter);
        Ok(())
    }

    pub async fn execute_async(&self, request: &ActionRequest) -> ActionResult {
        self.run_async(request, false).await
    }

    pub async fn preview_async(&self, request: &ActionRequest) -> ActionResult {
        self.run_async(request, true).await
    }

    async fn run_async(&self, request: &ActionRequest, preview: bool) -> ActionResult {
        if request.spec.parameter_ref != request.bound_parameter_ref {
            return ActionResult::Rejected(ActionRejection::BoundParametersDoNotMatchSpec);
        }
        let Some(adapter) = self.async_adapters.get(&request.spec.adapter) else {
            return ActionResult::Rejected(ActionRejection::AdapterNotRegistered);
        };
        if !adapter.supports_kind(&request.spec.kind) {
            return ActionResult::Rejected(ActionRejection::KindNotSupported);
        }
        let prepared = PreparedAction {
            spec: request.spec.clone(),
            bound_parameter_ref: request.bound_parameter_ref.clone(),
            spec_hash: action_spec_hash(&request.spec, &request.bound_parameter_ref),
        };
        if !self.authorizer.authorize(
            &prepared.spec,
            &request.requester,
            request.decision.as_ref(),
        ) {
            return ActionResult::Rejected(ActionRejection::AuthorizationDenied);
        }
        if !preview {
            match self.repository.lookup(&request.idempotency_key).await {
                Ok(Some(existing)) => return duplicate_result(existing, request, &prepared),
                Ok(None) => {}
                Err(()) => return ActionResult::Rejected(ActionRejection::RepositoryFailure),
            }
        }
        if let Some(rejection) = declared_unavailable(&prepared.spec) {
            return ActionResult::Rejected(rejection);
        }
        if (preview && !prepared.spec.capabilities.can_preview)
            || (!preview && !prepared.spec.capabilities.can_execute)
        {
            return ActionResult::Rejected(ActionRejection::Unsupported);
        }
        if adapter.expected_version(&prepared.spec).await != Ok(AdapterVersion::Matches) {
            return ActionResult::Rejected(ActionRejection::VersionMismatch);
        }
        if self.repository.prepare(prepared.clone()).await.is_err() {
            return ActionResult::Rejected(ActionRejection::RepositoryFailure);
        }
        if preview {
            if adapter.preview(&prepared).await.is_err() {
                return ActionResult::Rejected(ActionRejection::RepositoryFailure);
            }
            return ActionResult::Finished(ExecutionRecord {
                execution: execution_for(request, &prepared, ActionExecutionState::Prepared),
                effects: vec![],
                verification_evidence: vec![],
                recovery: None,
            });
        }
        let mut record = ExecutionRecord {
            execution: execution_for(request, &prepared, ActionExecutionState::Claimed),
            effects: vec![],
            verification_evidence: vec![],
            recovery: None,
        };
        match self
            .repository
            .claim_prepared(record.clone(), &prepared)
            .await
        {
            Ok(ClaimResult::Existing(existing)) => {
                return duplicate_result(existing, request, &prepared)
            }
            Ok(ClaimResult::Claimed) => {}
            Err(()) => return ActionResult::Rejected(ActionRejection::IdempotencyConflict),
        }
        if self
            .repository
            .transition(
                &request.execution_id,
                ActionExecutionState::Claimed,
                ActionExecutionState::Running,
            )
            .await
            .is_err()
        {
            return ActionResult::Rejected(ActionRejection::RepositoryFailure);
        }
        match adapter
            .apply_execution(&prepared, &record.execution.execution_id)
            .await
        {
            Ok(ApplyOutcome::CompletedWithRecovery {
                effects,
                verification_evidence,
                recovery,
            }) => {
                record.execution.status = ActionExecutionState::Completed;
                record.effects = effects;
                record.verification_evidence = verification_evidence;
                record.recovery = recovery;
            }
            Ok(ApplyOutcome::Completed {
                effects,
                verification_evidence,
            }) => {
                record.execution.status = ActionExecutionState::Completed;
                record.effects = effects;
                record.verification_evidence = verification_evidence;
            }
            Ok(ApplyOutcome::Partial {
                effects,
                verification_evidence,
                recovery,
            }) => {
                record.execution.status = ActionExecutionState::Partial;
                record.effects = effects;
                record.verification_evidence = verification_evidence;
                record.recovery = recovery;
            }
            Err(failure) => {
                record.execution.status = if failure.effects_may_have_occurred {
                    ActionExecutionState::OutcomeUnknown
                } else {
                    ActionExecutionState::Failed
                }
            }
        }
        record.execution.ended_at = Some(chrono::Utc::now().to_rfc3339());
        record.execution.effects = record.effects.clone();
        record.execution.verification_evidence = record.verification_evidence.clone();
        record.execution.recovery = record.recovery.clone();
        if self.repository.finish(record.clone()).await.is_err() {
            return ActionResult::Rejected(ActionRejection::RepositoryFailure);
        }
        ActionResult::Finished(record)
    }
}
