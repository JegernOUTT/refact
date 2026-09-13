//! Exact-action admission policy control. No evidence cleanup or automatic activation.
use std::sync::Arc;

use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};

use super::*;
use crate::buddy::operator_observability::{DocumentGrowthPolicy, PolicySnapshot};
use crate::buddy::outcome_runtime::OutcomeRuntime;

pub const ADAPTER: &str = "operator_retention";
pub const ACTION: &str = "document_growth_policy_apply";
pub const RESOURCE: &str = "document-growth-policy";
pub const DRAFT_NAMESPACE: &str = "policy_control_draft";
pub const BOUND_NAMESPACE: &str = "policy_control_prepared";

pub fn is_policy_action(spec: &ActionSpec) -> bool {
    spec.schema_version == 1
        && spec.adapter == ADAPTER
        && spec.kind == ACTION
        && spec.resource.resource_kind == ADAPTER
        && spec.resource.resource_id == RESOURCE
        && spec.resource.version == spec.expected_version
        && spec.authority_class == refact_buddy_core::operator_types::AuthorityClass::Mutating
        && spec.confirmation
            == refact_buddy_core::operator_types::ConfirmationClass::UserConfirmationRequired
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyDraft {
    pub project: ProjectIdentity,
    pub policy: Option<DocumentGrowthPolicy>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyControlAudit {
    pub project: ProjectIdentity,
    pub requester: refact_buddy_core::operator_types::NativeSourceIdentity,
    pub worker: refact_buddy_core::operator_types::WorkerIdentity,
    pub spec_hash: String,
    pub idempotency_key: String,
    pub expected_charge: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyControlRecord {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub audit: PolicyControlAudit,
    pub before: PolicySnapshot,
    pub after: PolicySnapshot,
}

pub struct PolicyAdapter {
    runtime: OutcomeRuntime,
    drafts: Arc<dyn DraftRegistry<PolicyDraft>>,
}

impl PolicyAdapter {
    pub fn new(runtime: OutcomeRuntime, drafts: Arc<dyn DraftRegistry<PolicyDraft>>) -> Self {
        Self { runtime, drafts }
    }

    fn draft(&self, spec: &ActionSpec) -> Result<PolicyDraft, ResourceError> {
        if !is_policy_action(spec) {
            return Err(ResourceError::UnsupportedResource);
        }
        if spec.project != self.runtime.project() {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        if draft.project != spec.project {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        if draft
            .policy
            .as_ref()
            .is_some_and(|policy| !(1..=100_000).contains(&policy.max_rows_per_namespace))
        {
            return Err(ResourceError::ValidationFailed(
                "capacity must be 1..=100000".into(),
            ));
        }
        Ok(draft)
    }

    async fn snapshot(&self) -> Result<PolicySnapshot, ResourceError> {
        self.runtime
            .with_store(|store| store.policy_snapshot())
            .await
            .map_err(|_| ResourceError::MaterialUnavailable)
    }
}

fn epoch(version: &str) -> Result<u64, ResourceError> {
    version
        .strip_prefix("document-policy:")
        .and_then(|value| value.parse().ok())
        .ok_or(ResourceError::VersionMismatch)
}

fn recovery_epochs(
    spec: &ActionSpec,
    recovery: &RecoveryDescriptor,
) -> Result<(u64, u64, u64), ResourceError> {
    let original = epoch(&spec.expected_version)?;
    if recovery.previous_material_reference != format!("document-policy-history:{original}") {
        return Err(ResourceError::VersionMismatch);
    }
    let applied = epoch(&recovery.expected_post_version)?;
    if applied
        != original
            .checked_add(1)
            .ok_or(ResourceError::VersionMismatch)?
    {
        return Err(ResourceError::VersionMismatch);
    }
    let recovered = applied
        .checked_add(1)
        .ok_or(ResourceError::VersionMismatch)?;
    Ok((original, applied, recovered))
}

#[async_trait::async_trait]
impl AsyncResourceAdapter for PolicyAdapter {
    fn adapter_id(&self) -> &str {
        ADAPTER
    }

    fn capabilities(&self) -> ResourceCapabilities {
        ResourceCapabilities {
            preview: true,
            apply: true,
            verify: true,
            recover: true,
        }
    }

    async fn preview(&self, spec: &ActionSpec) -> Result<ResourcePreview, ResourceError> {
        let draft = self.draft(spec)?;
        let before = self.snapshot().await?;
        if before.version() != spec.expected_version {
            return Err(ResourceError::VersionMismatch);
        }
        let next = before
            .epoch
            .checked_add(1)
            .ok_or(ResourceError::VersionMismatch)?;
        Ok(ResourcePreview {
            resource_id: RESOURCE.into(),
            expected_version: before.version(),
            proposed_version: format!("document-policy:{next}"),
            reload_needed: false,
            diff: Some(format!(
                "Admission policy {:?} -> {:?}. All document namespaces share 64 times the namespace capacity plus immutable grandfather obligations. Retain all evidence; native completion reserves are charged; this is not a hard SQLite byte quota.",
                before.policy, draft.policy
            )),
        })
    }

    async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
        let draft = self.draft(spec)?;
        let before = epoch(&spec.expected_version)?;
        let expected = spec.expected_version.clone();
        let after = self
            .runtime
            .with_store(move |store| store.compare_set_document_policy(&expected, draft.policy))
            .await
            .map_err(|_| ResourceError::VersionMismatch)?;
        Ok(ResourceApplyReceipt {
            resource_id: RESOURCE.into(),
            previous_material_reference: format!("document-policy-history:{before}"),
            expected_post_version: after.version(),
            state: ResourceApplyState::Applied,
        })
    }

    async fn verify(&self, spec: &ActionSpec, version: &str) -> Result<bool, ResourceError> {
        self.draft(spec)?;
        Ok(self.snapshot().await?.version() == version)
    }

    async fn recover(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<ResourceApplyState, ResourceError> {
        self.draft(spec)?;
        let (original, _, _) = recovery_epochs(spec, recovery)?;
        let expected = recovery.expected_post_version.clone();
        self.runtime
            .with_store(move |store| {
                let policy = store.historical_document_policy(original)?;
                store.compare_set_document_policy(&expected, policy)
            })
            .await
            .map_err(|_| ResourceError::VersionMismatch)?;
        Ok(ResourceApplyState::Applied)
    }

    async fn recovered_version(
        &self,
        spec: &ActionSpec,
        recovery: &RecoveryDescriptor,
    ) -> Result<String, ResourceError> {
        self.draft(spec)?;
        let (original, _, recovered) = recovery_epochs(spec, recovery)?;
        let expected = recovery.expected_post_version.clone();
        self.runtime
            .with_store(move |store| {
                if store.policy_snapshot()?.version() != expected {
                    return Err(crate::buddy::operator_store::OperatorStoreError::Conflict(
                        "policy changed; prepare and grant again",
                    ));
                }
                store.historical_document_policy(original).map(|_| ())
            })
            .await
            .map_err(|_| ResourceError::VersionMismatch)?;
        Ok(format!("document-policy:{recovered}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::action_execution::{
        action_spec_hash, ActionRejection, ActionRequest, ActionResult, AsyncActionRepository,
        Authorizer, PreparedAction,
    };
    use crate::buddy::action_recovery::{RecoveryRequest, RecoveryResult};
    use crate::buddy::resource_execution::{BoundDraft, ExactActionGrant, ResourceCatalog};
    use refact_buddy_core::operator_types::*;

    struct Deny;

    impl Authorizer for Deny {
        fn authorize(
            &self,
            _: &ActionSpec,
            _: &NativeSourceIdentity,
            _: Option<&DecisionReference>,
        ) -> bool {
            false
        }
    }

    fn request(project: ProjectIdentity, worker: WorkerIdentity) -> ActionRequest {
        ActionRequest {
            execution_id: "policy-apply".into(),
            idempotency_key: "policy-apply-key".into(),
            bound_parameter_ref: Some("policy-draft".into()),
            requester: NativeSourceIdentity {
                source_kind: NativeSourceKind::Chat,
                source_id: "requester".into(),
                source_version: "1".into(),
            },
            decision: None,
            worker,
            spec: ActionSpec {
                schema_version: 1,
                action_id: "policy-action".into(),
                kind: ACTION.into(),
                adapter: ADAPTER.into(),
                project: project.clone(),
                resource: ResourceRef {
                    resource_kind: ADAPTER.into(),
                    resource_id: RESOURCE.into(),
                    version: "document-policy:0".into(),
                },
                expected_version: "document-policy:0".into(),
                parameter_ref: Some("policy-draft".into()),
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
            },
        }
    }

    fn grant(
        request: &mut ActionRequest,
        recovery_execution_id: Option<String>,
    ) -> ExactActionGrant {
        let id = if recovery_execution_id.is_some() {
            "policy-recovery-grant"
        } else {
            "policy-apply-grant"
        };
        let decision = DecisionReference {
            project: request.spec.project.clone(),
            worker: request.worker.clone(),
            native: NativeSourceIdentity {
                source_kind: NativeSourceKind::External,
                source_id: id.into(),
                source_version: "resource-grant-v1".into(),
            },
        };
        request.decision = Some(decision.clone());
        ExactActionGrant {
            native_rights: None,
            recovery_execution_id,
            spec_hash: action_spec_hash(&request.spec, &request.bound_parameter_ref),
            requester: request.requester.clone(),
            decision: Some(decision),
            expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            revoked: false,
        }
    }

    #[test]
    fn policy_identity_is_exactly_typed() {
        let project = ProjectIdentity {
            project_id: "project".into(),
            canonical_root: "/project".into(),
        };
        let worker = WorkerIdentity {
            worker_id: "worker".into(),
            engine_instance_id: "engine".into(),
            epoch: 1,
        };
        let request = request(project, worker);
        assert!(is_policy_action(&request.spec));
        let mutations: [fn(&mut ActionSpec); 7] = [
            |spec: &mut ActionSpec| spec.schema_version = 2,
            |spec: &mut ActionSpec| spec.kind = "other".into(),
            |spec: &mut ActionSpec| spec.resource.resource_id = "other".into(),
            |spec: &mut ActionSpec| spec.resource.resource_kind = "other".into(),
            |spec: &mut ActionSpec| spec.resource.version = "document-policy:1".into(),
            |spec: &mut ActionSpec| spec.authority_class = AuthorityClass::ReadOnly,
            |spec: &mut ActionSpec| spec.confirmation = ConfirmationClass::None,
        ];
        for changed in mutations {
            let mut spec = request.spec.clone();
            changed(&mut spec);
            assert!(!is_policy_action(&spec));
        }
    }

    #[tokio::test]
    async fn inactive_policy_requires_grant_and_recovery_advances_epoch_after_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let project = ProjectIdentity {
            project_id: "project".into(),
            canonical_root: root.to_string_lossy().into_owned(),
        };
        let worker = WorkerIdentity {
            worker_id: "worker".into(),
            engine_instance_id: "engine".into(),
            epoch: 1,
        };
        let runtime = OutcomeRuntime::open(&root, project.clone(), worker.clone()).unwrap();
        assert_eq!(
            runtime
                .with_store(|store| store.policy_snapshot())
                .await
                .unwrap(),
            PolicySnapshot {
                epoch: 0,
                policy: None
            }
        );
        let policy = DocumentGrowthPolicy {
            max_rows_per_namespace: 32,
            replace_editorial_aggregate: false,
        };
        let draft = Arc::new(BoundDraft::new(
            "policy-draft".into(),
            PolicyDraft {
                project: project.clone(),
                policy: Some(policy.clone()),
            },
        ));
        let mut catalog = ResourceCatalog::default();
        catalog
            .register(
                ACTION.into(),
                Arc::new(PolicyAdapter::new(runtime.clone(), draft.clone())),
            )
            .unwrap();
        let mut apply_request = request(project.clone(), worker.clone());
        let denied = catalog
            .service(runtime.clone(), Arc::new(Deny))
            .unwrap()
            .execute_async(&apply_request)
            .await;
        assert!(matches!(
            denied,
            ActionResult::Rejected(ActionRejection::AuthorizationDenied)
        ));
        assert_eq!(
            runtime
                .with_store(|store| store.policy_snapshot())
                .await
                .unwrap()
                .epoch,
            0
        );
        let apply_grant = Arc::new(grant(&mut apply_request, None));
        let prepared = PreparedAction {
            spec_hash: action_spec_hash(&apply_request.spec, &apply_request.bound_parameter_ref),
            bound_parameter_ref: apply_request.bound_parameter_ref.clone(),
            spec: apply_request.spec.clone(),
        };
        crate::buddy::resource_execution::RuntimeActionRepository(runtime.clone())
            .prepare(prepared)
            .await
            .unwrap();
        assert_eq!(
            catalog
                .preview(&apply_request.spec)
                .await
                .unwrap()
                .proposed_version,
            "document-policy:1"
        );
        let applied = match catalog
            .service(runtime.clone(), apply_grant.clone())
            .unwrap()
            .execute_async(&apply_request)
            .await
        {
            ActionResult::Finished(record) => record,
            other => panic!("{other:?}"),
        };
        assert_eq!(applied.execution.status, ActionExecutionState::Completed);
        assert_eq!(applied.effects[0].resource.version, "document-policy:1");
        assert_eq!(
            runtime
                .with_store(|store| store.policy_snapshot())
                .await
                .unwrap(),
            PolicySnapshot {
                epoch: 1,
                policy: Some(policy)
            }
        );
        runtime.shutdown().await;

        let reopened = OutcomeRuntime::open(&root, project.clone(), worker.clone()).unwrap();
        assert_eq!(
            reopened
                .with_store(|store| store.policy_snapshot())
                .await
                .unwrap()
                .epoch,
            1
        );
        let mut reopened_catalog = ResourceCatalog::default();
        reopened_catalog
            .register(
                ACTION.into(),
                Arc::new(PolicyAdapter::new(reopened.clone(), draft)),
            )
            .unwrap();
        let mut recovery_grant_request = apply_request.clone();
        let recovery_grant = Arc::new(grant(
            &mut recovery_grant_request,
            Some(apply_request.execution_id.clone()),
        ));
        assert!(apply_grant.recovery_execution_id.is_none());
        assert_eq!(
            recovery_grant.recovery_execution_id.as_deref(),
            Some(apply_request.execution_id.as_str())
        );
        assert_ne!(apply_grant.decision, recovery_grant.decision);
        let recovery = RecoveryRequest {
            execution_id: "policy-recover".into(),
            idempotency_key: "policy-recover-key".into(),
            original_execution_id: apply_request.execution_id.clone(),
            original_spec: apply_request.spec.clone(),
            original_bound_parameter_ref: apply_request.bound_parameter_ref.clone(),
            recovery_id: apply_request.execution_id.clone(),
            expected_post_version: "document-policy:1".into(),
            requester: apply_request.requester.clone(),
            decision: recovery_grant.decision.clone(),
            worker,
        };
        let recovered = match reopened_catalog
            .recover(reopened.clone(), recovery_grant, &recovery)
            .await
        {
            RecoveryResult::Finished(receipt) => receipt,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            recovered.restored_version.as_deref(),
            Some("document-policy:2")
        );
        assert_eq!(
            recovered.receipt.execution.status,
            ActionExecutionState::Completed
        );
        assert_eq!(
            reopened
                .with_store(|store| store.policy_snapshot())
                .await
                .unwrap(),
            PolicySnapshot {
                epoch: 2,
                policy: None
            }
        );
        assert!(matches!(
            reopened_catalog
                .recover(
                    reopened.clone(),
                    Arc::new(Deny),
                    &RecoveryRequest {
                        execution_id: "second-recover".into(),
                        idempotency_key: "second-recover-key".into(),
                        ..recovery
                    }
                )
                .await,
            RecoveryResult::Rejected(_)
        ));
        assert_eq!(
            reopened
                .with_store(|store| store.policy_snapshot())
                .await
                .unwrap()
                .epoch,
            2
        );
        reopened.shutdown().await;
    }
}
