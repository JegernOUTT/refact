use std::collections::HashMap;
use std::sync::Arc;
use refact_buddy_core::operator_types::{
    ActionSpec, ActionExecutionState, ProjectIdentity, ResourceEffect, EffectState, RecoveryRef,
};
use serde::{Serialize, Deserialize};
use super::action_execution::{
    ActionService, AsyncActionAdapter, AsyncActionRepository, AdapterVersion, ApplyOutcome,
    ApplyFailure, PreparedAction, ExecutionRecord, ClaimResult, ActionRepository,
    OperatorStoreRepository, Authorizer,
};
use super::outcome_runtime::OutcomeRuntime;
use super::operator_store::OperatorStoreError;
use super::resources::{
    AsyncResourceAdapter, ResourceApplyReceipt, ResourceApplyState, ResourcePreview, ResourceError,
};

/// Serializes grants/revocations and connected effects; database claims remain authoritative
/// across restart. Never hold the store transaction across an adapter await.
pub fn execution_gate() -> &'static tokio::sync::Mutex<()> {
    static GATE: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub async fn load_prepared(
    runtime: &OutcomeRuntime,
    hash: String,
) -> Result<PreparedAction, String> {
    runtime
        .with_store(move |store| {
            let prepared = store
                .get_document::<ResourceDocument<PreparedAction>>("prepared_action", &hash)?
                .ok_or(OperatorStoreError::Conflict("prepared action missing"))?
                .value
                .payload;
            if prepared.spec_hash != hash
                || super::action_execution::action_spec_hash(
                    &prepared.spec,
                    &prepared.bound_parameter_ref,
                ) != hash
            {
                return Err(OperatorStoreError::Conflict("prepared action changed"));
            }
            Ok(prepared)
        })
        .await
        .map_err(|e| e.to_string())
}

fn draft_namespaces(spec: &ActionSpec) -> (&'static str, &'static str) {
    if super::resources::retention::is_policy_action(spec) {
        (
            super::resources::retention::DRAFT_NAMESPACE,
            super::resources::retention::BOUND_NAMESPACE,
        )
    } else {
        ("resource_draft", "prepared_draft")
    }
}

pub async fn persist_policy_draft(
    runtime: &OutcomeRuntime,
    spec: &ActionSpec,
    draft: super::resources::retention::PolicyDraft,
) -> Result<(), OperatorStoreError> {
    if draft_namespaces(spec).0 != "policy_control_draft"
        || draft.project != runtime.project()
        || spec.project != runtime.project()
    {
        return Err(OperatorStoreError::ProjectMismatch);
    }
    let id = spec
        .parameter_ref
        .clone()
        .ok_or(OperatorStoreError::InvalidInput(
            "policy draft reference missing",
        ))?;
    let value = ResourceDocument {
        schema_version: 1,
        project: runtime.project(),
        payload: draft,
    };
    runtime
        .with_store(move |s| {
            s.put_document("policy_control_draft", &id, &value, None)
                .map(|_| ())
        })
        .await
}

async fn bound_draft<D: Serialize + serde::de::DeserializeOwned + Clone + Send + Sync + 'static>(
    runtime: &OutcomeRuntime,
    spec: &ActionSpec,
) -> Result<Arc<BoundDraft<D>>, ResourceError> {
    let reference = spec
        .parameter_ref
        .clone()
        .ok_or(ResourceError::ParameterReferenceMissing)?;
    let hash = super::action_execution::action_spec_hash(spec, &spec.parameter_ref);
    let key = reference.clone();
    let (draft_namespace, prepared_namespace) = draft_namespaces(spec);
    let draft = runtime
        .with_store(move |store| {
            let bound = store.get_document::<ResourceDocument<D>>(prepared_namespace, &hash)?;
            let document = match bound {
                Some(bound) => Some(bound),
                None => store.get_document::<ResourceDocument<D>>(draft_namespace, &key)?,
            };
            Ok(document.map(|d| d.value.payload))
        })
        .await
        .map_err(|_| ResourceError::MaterialUnavailable)?
        .ok_or(ResourceError::ParameterReferenceMissing)?;
    Ok(Arc::new(BoundDraft::new(reference, draft)))
}

/// Resolve only the server-owned adapter/kind pairs, with a typed persisted draft.
pub async fn registered_catalog(
    app: crate::app_state::AppState,
    runtime: &OutcomeRuntime,
    spec: &ActionSpec,
) -> Result<ResourceCatalog, ResourceError> {
    registered_catalog_with_grant(app, runtime, spec, None).await
}

async fn registered_catalog_with_grant(
    app: crate::app_state::AppState,
    runtime: &OutcomeRuntime,
    spec: &ActionSpec,
    grant: Option<ExactActionGrant>,
) -> Result<ResourceCatalog, ResourceError> {
    use super::resources::*;
    if spec.project != runtime.project() || spec.schema_version != 1 {
        return Err(ResourceError::ProjectScopeMismatch);
    }
    let project = runtime.project();
    if spec.confirmation
        != refact_buddy_core::operator_types::ConfirmationClass::UserConfirmationRequired
        || spec.authority_class != refact_buddy_core::operator_types::AuthorityClass::Mutating
    {
        return Err(ResourceError::ValidationFailed(
            "resource actions require explicit mutation confirmation".into(),
        ));
    }
    let materials = Arc::new(LazyRecoveryMaterials(project.clone()));
    let rights: Arc<dyn crate::ext::operator_service::ResourceRights> = match grant {
        Some(grant) => Arc::new(GrantedResourceRights(grant)),
        None => Arc::new(crate::ext::operator_service::DenyResourceRights),
    };
    let (kind, adapter): (&str, Arc<dyn AsyncResourceAdapter>) = match spec.adapter.as_str() {
        retention::ADAPTER => (
            retention::ACTION,
            Arc::new(retention::PolicyAdapter::new(
                runtime.clone(),
                bound_draft(runtime, spec).await?,
            )),
        ),

        "settings" => (
            "settings_trajectory_patch",
            Arc::new(settings::SettingsAdapter::new(
                app,
                project,
                bound_draft(runtime, spec).await?,
                materials,
            )),
        ),
        "buddy_settings" => (
            "buddy_settings_patch",
            Arc::new(settings::BuddySettingsAdapter::new(
                app,
                project,
                bound_draft(runtime, spec).await?,
                materials,
            )),
        ),
        "default_models" => (
            "default_model_select",
            Arc::new(default_models::DefaultModelsAdapter::new(
                app,
                project,
                bound_draft(runtime, spec).await?,
                materials,
            )),
        ),
        "providers" => (
            "provider_enable_custom_model",
            Arc::new(providers::ProvidersAdapter::new(
                app,
                project,
                bound_draft(runtime, spec).await?,
                materials,
            )),
        ),
        "memory" => (
            "memory_lifecycle",
            Arc::new(memory::MemoryAdapter::new(
                app,
                project,
                bound_draft(runtime, spec).await?,
                materials,
            )),
        ),
        "history" => (
            "export_backup",
            Arc::new(history::HistoryAdapter::new(
                app,
                runtime.clone(),
                bound_draft(runtime, spec).await?,
                materials,
            )),
        ),
        "hooks" => (
            "hook_project_apply",
            Arc::new(hooks::HooksAdapter::new(
                hooks::HookPreparation::new(project, bound_draft(runtime, spec).await?),
                materials,
                rights,
            )),
        ),
        // Inert disabled-config preparation and connected native MCP lifecycle are distinct
        // action kinds on one adapter identity; neither inherits the other's authority.
        "integrations" if spec.kind == "mcp_project_apply" => (
            "mcp_project_apply",
            Arc::new(integrations::IntegrationsAdapter::new(
                integrations::McpPreparation::new(project, bound_draft(runtime, spec).await?),
                materials,
                rights,
            )),
        ),
        "integrations"
            if matches!(
                spec.kind.as_str(),
                "mcp_capability_probe" | "mcp_enabled_reload"
            ) =>
        {
            let kind: &'static str = if spec.kind == "mcp_capability_probe" {
                "mcp_capability_probe"
            } else {
                "mcp_enabled_reload"
            };
            (
                kind,
                Arc::new(integrations::McpRuntimeAdapter::new(
                    project,
                    bound_draft(runtime, spec).await?,
                    rights,
                    app.gcx,
                    materials,
                )),
            )
        }
        "worktree_housekeeping" => {
            let worktrees = crate::worktrees::service::WorktreeService::new_async(
                app.paths.cache_dir.clone(),
                project.canonical_root.clone().into(),
            )
            .await
            .map_err(ResourceError::ValidationFailed)?;
            (
                "archive",
                Arc::new(worktree_housekeeping::WorktreeHousekeepingAdapter::new(
                    app,
                    runtime.clone(),
                    worktrees,
                    bound_draft(runtime, spec).await?,
                    materials,
                )?),
            )
        }
        "extensions" => (
            "markdown_project_apply",
            Arc::new(extensions::ExtensionsAdapter::new(
                extensions::MarkdownPreparation::new(project, bound_draft(runtime, spec).await?),
                materials,
                rights,
            )),
        ),
        super::improvement::installation::ADAPTER => {
            let owner = super::operator_runtime::runtime(
                &app,
                super::operator_runtime::OperatorCapability::Improvement,
            )
            .await
            .map_err(ResourceError::ValidationFailed)?;
            if owner.project() != runtime.project() || owner.worker() != runtime.worker() {
                return Err(ResourceError::ProjectScopeMismatch);
            }
            (
                super::improvement::installation::ACTION,
                Arc::new(super::improvement::installation::InstallationAdapter::new(
                    runtime.clone(),
                    bound_draft(runtime, spec).await?,
                )),
            )
        }
        super::improvement::activation::ADAPTER => {
            let owner = super::operator_runtime::runtime(
                &app,
                super::operator_runtime::OperatorCapability::Improvement,
            )
            .await
            .map_err(ResourceError::ValidationFailed)?;
            if owner.project() != runtime.project() || owner.worker() != runtime.worker() {
                return Err(ResourceError::ProjectScopeMismatch);
            }
            let worktrees = crate::worktrees::service::WorktreeService::new_async(
                app.paths.cache_dir.clone(),
                project.canonical_root.clone().into(),
            )
            .await
            .map_err(ResourceError::ValidationFailed)?;
            (
                super::improvement::activation::ACTION,
                Arc::new(super::improvement::activation::LandingAdapter::new(
                    app,
                    runtime.clone(),
                    worktrees,
                    bound_draft(runtime, spec).await?,
                    materials,
                )),
            )
        }
        super::improvement::artifacts::CLEANUP_ADAPTER => {
            let owner = super::operator_runtime::runtime(
                &app,
                super::operator_runtime::OperatorCapability::Improvement,
            )
            .await
            .map_err(ResourceError::ValidationFailed)?;
            if owner.project() != runtime.project() || owner.worker() != runtime.worker() {
                return Err(ResourceError::ProjectScopeMismatch);
            }
            (
                super::improvement::artifacts::CLEANUP_ACTION,
                Arc::new(super::improvement::artifacts::ArtifactCleanupAdapter::new(
                    runtime.clone(),
                    bound_draft(runtime, spec).await?,
                )),
            )
        }
        super::improvement::artifacts::ADAPTER => {
            let owner = super::operator_runtime::runtime(
                &app,
                super::operator_runtime::OperatorCapability::Improvement,
            )
            .await
            .map_err(ResourceError::ValidationFailed)?;
            if owner.project() != runtime.project() || owner.worker() != runtime.worker() {
                return Err(ResourceError::ProjectScopeMismatch);
            }
            let worktrees = crate::worktrees::service::WorktreeService::new_async(
                app.paths.cache_dir.clone(),
                project.canonical_root.clone().into(),
            )
            .await
            .map_err(ResourceError::ValidationFailed)?;
            (
                super::improvement::artifacts::ACTION,
                Arc::new(super::improvement::artifacts::ArtifactAdapter::new(
                    runtime.clone(),
                    worktrees,
                    bound_draft(runtime, spec).await?,
                )),
            )
        }
        "project_code" => {
            let worktrees = crate::worktrees::service::WorktreeService::new(
                app.paths.cache_dir.clone(),
                project.canonical_root.clone().into(),
            )
            .map_err(ResourceError::ValidationFailed)?;
            (
                "checkpoint_isolated_candidate",
                Arc::new(project_code::ProjectCodeAdapter::new(
                    app.gcx,
                    project,
                    worktrees,
                    bound_draft(runtime, spec).await?,
                    materials,
                )?),
            )
        }
        customization::ADAPTER_ID => {
            let files = customization::CustomizationFileAdapter::new(
                project,
                bound_draft::<customization::CustomizationDraft>(runtime, spec).await?,
                materials,
            )?;
            (
                customization::ACTION_KIND,
                Arc::new(customization::ActiveCustomizationAdapter::new(
                    files, app.gcx,
                )),
            )
        }
        _ => return Err(ResourceError::UnsupportedResource),
    };
    if spec.kind != kind {
        return Err(ResourceError::UnsupportedResource);
    }
    let caps = adapter.capabilities();
    if !spec.capabilities.can_preview
        || (spec.capabilities.can_execute && !caps.apply)
        || (spec.capabilities.can_verify && !caps.verify)
        || (spec.capabilities.can_recover && !caps.recover)
        || spec.capabilities.can_cancel
    {
        return Err(ResourceError::UnsupportedResource);
    }
    let mut catalog = ResourceCatalog::default();
    catalog.register(kind.into(), adapter)?;
    Ok(catalog)
}

/// Persist the exact draft bytes alongside preparation; a later draft revision never
/// inherits consent merely because its opaque reference stayed the same.
pub async fn bind_prepared_draft(
    runtime: &OutcomeRuntime,
    prepared: &PreparedAction,
) -> Result<(), OperatorStoreError> {
    let prepared = prepared.clone();
    let (draft_namespace, prepared_namespace) = draft_namespaces(&prepared.spec);
    runtime
        .with_store(move |store| {
            let reference = prepared
                .bound_parameter_ref
                .as_deref()
                .ok_or(OperatorStoreError::Conflict("draft missing"))?;
            let draft = store
                .get_document::<ResourceDocument<serde_json::Value>>(draft_namespace, reference)?
                .ok_or(OperatorStoreError::Conflict("draft missing"))?
                .value;
            store.put_document(prepared_namespace, &prepared.spec_hash, &draft, None)?;
            Ok(())
        })
        .await
}

pub async fn validate_prepared_draft(
    runtime: &OutcomeRuntime,
    prepared: &PreparedAction,
) -> Result<(), String> {
    let prepared = prepared.clone();
    let (draft_namespace, prepared_namespace) = draft_namespaces(&prepared.spec);
    runtime
        .with_store(move |store| {
            let reference = prepared
                .bound_parameter_ref
                .as_deref()
                .ok_or(OperatorStoreError::Conflict("draft missing"))?;
            let current = store
                .get_document::<ResourceDocument<serde_json::Value>>(draft_namespace, reference)?;
            let bound = store.get_document::<ResourceDocument<serde_json::Value>>(
                prepared_namespace,
                &prepared.spec_hash,
            )?;
            match (current, bound) {
                (Some(current), Some(bound)) if current.value.payload == bound.value.payload => {
                    Ok(())
                }
                _ => Err(OperatorStoreError::Conflict(
                    "prepared draft changed; prepare and confirm again",
                )),
            }
        })
        .await
        .map_err(|e| e.to_string())
}

pub async fn execute_registered(
    app: crate::app_state::AppState,
    runtime: OutcomeRuntime,
    request: super::action_execution::ActionRequest,
) -> Result<super::action_execution::ActionResult, String> {
    let gate = execution_gate().lock().await;
    let active = super::operator_runtime::runtime(
        &app,
        super::operator_runtime::OperatorCapability::Actions,
    )
    .await?;
    if active.project() != runtime.project()
        || active.worker() != runtime.worker()
        || (request.spec.adapter == super::improvement::activation::ADAPTER
            && !active.same_owner(&runtime))
        || request.worker != runtime.worker()
    {
        return Err("operator project/worker changed".into());
    }
    let hash =
        super::action_execution::action_spec_hash(&request.spec, &request.bound_parameter_ref);
    let prepared = load_prepared(&runtime, hash).await?;
    if prepared.spec != request.spec || prepared.bound_parameter_ref != request.bound_parameter_ref
    {
        return Err("prepared action changed".into());
    }
    let decision = request.decision.as_ref().ok_or("exact grant required")?;
    let grant = load_grant(&runtime, decision.native.source_id.clone())
        .await
        .map_err(|e| e.to_string())?
        .ok_or("grant missing")?;
    if grant.recovery_execution_id.is_some()
        || !grant.authorize(&request.spec, &request.requester, request.decision.as_ref())
    {
        return Err("grant denied".into());
    }
    // A response-lost retry must not re-open resources or require their old preimage.
    if let Some(existing) = RuntimeActionRepository(runtime.clone())
        .lookup(&request.idempotency_key)
        .await
        .map_err(|_| "execution lookup failed")?
    {
        return Ok(
            if existing.execution.spec_hash == prepared.spec_hash
                && existing.execution.project == request.spec.project
                && existing.execution.requester == request.requester
                && existing.execution.decision == request.decision
            {
                super::action_execution::ActionResult::Duplicate(existing)
            } else {
                super::action_execution::ActionResult::Rejected(
                    super::action_execution::ActionRejection::IdempotencyConflict,
                )
            },
        );
    }
    validate_prepared_draft(&runtime, &prepared).await?;
    let origin_hash = prepared.spec_hash.clone();
    runtime
        .with_store(move |store| super::desk_actions::validate_resource_origin(store, &origin_hash))
        .await
        .map_err(|error| error.to_string())?;
    let catalog = registered_catalog_with_grant(app, &runtime, &request.spec, Some(grant.clone()))
        .await
        .map_err(|e| format!("{e:?}"))?;
    let _gate = if request.spec.adapter == super::improvement::activation::ADAPTER {
        drop(gate);
        None
    } else {
        Some(gate)
    };
    Ok(catalog
        .service(runtime, Arc::new(grant))
        .map_err(|_| "catalog registration failed")?
        .execute_async(&request)
        .await)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceDocument<T> {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub payload: T,
}

#[derive(Clone)]
pub(crate) struct ResourceExecutionContext {
    pub runtime: OutcomeRuntime,
    pub execution:
        super::operator_store::Stored<refact_buddy_core::operator_types::ActionExecution>,
    pub recovery: Option<super::action_recovery::RecoveryRequest>,
}

tokio::task_local! {
    static RESOURCE_EXECUTION: ResourceExecutionContext;
}

impl ResourceExecutionContext {
    pub(crate) async fn scope<F: std::future::Future>(self, operation: F) -> F::Output {
        RESOURCE_EXECUTION.scope(self, operation).await
    }

    pub(crate) async fn publish_receipt(
        &self,
        spec: &ActionSpec,
        receipt: &ResourceApplyReceipt,
    ) -> Result<(), OperatorStoreError> {
        let execution = self.execution.clone();
        let spec = spec.clone();
        let value = ResourceDocument {
            schema_version: 1,
            project: spec.project.clone(),
            payload: receipt.clone(),
        };
        self.runtime
            .with_store(move |store| {
                let id = &execution.value.execution_id;
                if execution.value.status != ActionExecutionState::Running
                    || execution.value.project != spec.project
                    || execution.value.action_id != spec.action_id
                    || execution.value.spec_hash
                        != super::action_execution::action_spec_hash(&spec, &spec.parameter_ref)
                    || store.identity() != &spec.project
                    || store.get_execution(id)?.as_ref() != Some(&execution)
                    || value.payload.resource_id != spec.resource.resource_id
                {
                    return Err(OperatorStoreError::Conflict(
                        "receipt execution binding changed",
                    ));
                }
                store
                    .put_reserved_document(
                        "resource_apply",
                        id,
                        "resource_receipt",
                        id,
                        &value,
                        None,
                    )
                    .map(|_| ())
            })
            .await
    }

    async fn receipt_is_published(
        &self,
        spec: &ActionSpec,
        receipt: &ResourceApplyReceipt,
    ) -> bool {
        let execution = self.execution.clone();
        let spec = spec.clone();
        let receipt = receipt.clone();
        self.runtime
            .with_store(move |store| {
                let stored = store.get_document::<ResourceDocument<ResourceApplyReceipt>>(
                    "resource_receipt",
                    &execution.value.execution_id,
                )?;
                Ok(store.identity() == &spec.project
                    && execution.value.project == spec.project
                    && execution.value.action_id == spec.action_id
                    && execution.value.spec_hash
                        == super::action_execution::action_spec_hash(&spec, &spec.parameter_ref)
                    && store.get_execution(&execution.value.execution_id)?.as_ref()
                        == Some(&execution)
                    && receipt.resource_id == spec.resource.resource_id
                    && stored.is_some_and(|stored| {
                        stored.value.schema_version == 1
                            && stored.value.project == spec.project
                            && stored.value.payload == receipt
                    }))
            })
            .await
            .unwrap_or(false)
    }
}

pub(crate) fn current_resource_execution() -> Result<ResourceExecutionContext, ResourceError> {
    RESOURCE_EXECUTION.try_with(Clone::clone).map_err(|_| {
        ResourceError::ValidationFailed("current resource execution authority unavailable".into())
    })
}

pub struct RuntimeActionRepository(pub OutcomeRuntime);

#[async_trait::async_trait]
impl AsyncActionRepository for RuntimeActionRepository {
    async fn lookup(&self, key: &str) -> Result<Option<ExecutionRecord>, ()> {
        let key = key.to_owned();
        self.0
            .with_store(move |store| {
                Ok(store
                    .get_execution_by_idempotency_key(&key)?
                    .map(|stored| record(stored.value)))
            })
            .await
            .map_err(|_| ())
    }
    async fn prepare(&self, prepared: PreparedAction) -> Result<(), ()> {
        let id = prepared.spec_hash.clone();
        let value = ResourceDocument {
            schema_version: 1,
            project: prepared.spec.project.clone(),
            payload: prepared,
        };
        self.0
            .with_store(move |store| {
                // Preserve owner-only envelope provenance on an exact retry. The
                // generic repository must never erase or mint daemon preparation.
                if let Some(existing) = store
                    .get_document::<ResourceDocument<PreparedAction>>("prepared_action", &id)?
                {
                    if existing.value.project != value.project
                        || existing.value.payload != value.payload
                    {
                        return Err(OperatorStoreError::Conflict("prepared action changed"));
                    }
                    return Ok(());
                }
                store
                    .put_document("prepared_action", &id, &value, None)
                    .map(|_| ())
            })
            .await
            .map_err(|_| ())
    }
    async fn claim(&self, record: ExecutionRecord) -> Result<ClaimResult, ()> {
        self.0
            .with_store(move |store| {
                OperatorStoreRepository::new(store)
                    .claim(record)
                    .map_err(|_| OperatorStoreError::Conflict("action claim rejected"))
            })
            .await
            .map_err(|_| ())
    }
    async fn claim_prepared(
        &self,
        execution: ExecutionRecord,
        prepared: &PreparedAction,
    ) -> Result<ClaimResult, ()> {
        let prepared = prepared.clone();
        self.0
            .with_store(move |store| {
                let stored = store
                    .get_document::<ResourceDocument<PreparedAction>>(
                        "prepared_action",
                        &prepared.spec_hash,
                    )?
                    .ok_or(OperatorStoreError::Conflict("resource preparation missing"))?;
                if stored.value.payload != prepared
                    || prepared.spec.project != *store.identity()
                    || execution.execution.spec_hash != prepared.spec_hash
                    || execution.execution.action_id != prepared.spec.action_id
                {
                    return Err(OperatorStoreError::Conflict(
                        "resource execution binding changed",
                    ));
                }
                match store.claim_resource_execution(&execution.execution, &prepared)? {
                    super::operator_store::ExecutionClaim::Claimed(_) => Ok(ClaimResult::Claimed),
                    super::operator_store::ExecutionClaim::Existing(stored) => {
                        Ok(ClaimResult::Existing(record(stored.value)))
                    }
                }
            })
            .await
            .map_err(|_| ())
    }
    async fn transition(
        &self,
        id: &str,
        expected: ActionExecutionState,
        status: ActionExecutionState,
    ) -> Result<(), ()> {
        let id = id.to_owned();
        self.0
            .with_store(move |store| {
                OperatorStoreRepository::new(store)
                    .transition(&id, expected, status)
                    .map_err(|_| OperatorStoreError::Conflict("action transition rejected"))
            })
            .await
            .map_err(|_| ())
    }
    async fn finish(&self, record: ExecutionRecord) -> Result<(), ()> {
        self.0
            .with_store(move |store| {
                OperatorStoreRepository::new(store)
                    .finish(record)
                    .map_err(|_| OperatorStoreError::Conflict("action finish rejected"))
            })
            .await
            .map_err(|_| ())
    }
}

fn record(execution: refact_buddy_core::operator_types::ActionExecution) -> ExecutionRecord {
    ExecutionRecord {
        effects: execution.effects.clone(),
        verification_evidence: execution.verification_evidence.clone(),
        recovery: execution.recovery.clone(),
        execution,
    }
}

#[derive(Default)]
pub struct ResourceCatalog {
    adapters: HashMap<String, (String, Arc<dyn AsyncResourceAdapter>)>,
}

impl ResourceCatalog {
    pub fn register(
        &mut self,
        action_kind: String,
        adapter: Arc<dyn AsyncResourceAdapter>,
    ) -> Result<(), ResourceError> {
        let id = adapter.adapter_id().to_owned();
        if id.trim().is_empty() || action_kind.trim().is_empty() || self.adapters.contains_key(&id)
        {
            return Err(ResourceError::UnsupportedResource);
        }
        self.adapters.insert(id, (action_kind, adapter));
        Ok(())
    }
    pub async fn preview(&self, spec: &ActionSpec) -> Result<ResourcePreview, ResourceError> {
        let (kind, adapter) = self
            .adapters
            .get(&spec.adapter)
            .ok_or(ResourceError::UnsupportedResource)?;
        if kind != &spec.kind || !adapter.capabilities().preview {
            return Err(ResourceError::UnsupportedResource);
        }
        adapter.preview(spec).await
    }
    pub fn service(
        &self,
        runtime: OutcomeRuntime,
        authorizer: Arc<dyn Authorizer>,
    ) -> Result<ActionService<RuntimeActionRepository>, ()> {
        let mut service =
            ActionService::new_async(RuntimeActionRepository(runtime.clone()), authorizer);
        for (kind, adapter) in self.adapters.values() {
            service.register_async_adapter(Arc::new(ResourceActionAdapter {
                kind: kind.clone(),
                adapter: adapter.clone(),
                runtime: runtime.clone(),
            }))?;
        }
        Ok(service)
    }
}

pub struct ResourceActionAdapter {
    kind: String,
    adapter: Arc<dyn AsyncResourceAdapter>,
    runtime: OutcomeRuntime,
}

#[async_trait::async_trait]
impl AsyncActionAdapter for ResourceActionAdapter {
    fn adapter_id(&self) -> &str {
        self.adapter.adapter_id()
    }
    fn supports_kind(&self, kind: &str) -> bool {
        kind == self.kind
    }
    async fn expected_version(&self, spec: &ActionSpec) -> Result<AdapterVersion, ()> {
        match self.adapter.preview(spec).await {
            Ok(preview) if preview.expected_version == spec.expected_version => {
                Ok(AdapterVersion::Matches)
            }
            Ok(_) | Err(ResourceError::VersionMismatch) => Ok(AdapterVersion::Mismatch),
            Err(_) => Err(()),
        }
    }
    async fn preview(&self, prepared: &PreparedAction) -> Result<(), ()> {
        if !self.adapter.capabilities().preview {
            return Err(());
        }
        self.adapter
            .preview(&prepared.spec)
            .await
            .map(|_| ())
            .map_err(|_| ())
    }
    async fn apply(&self, _prepared: &PreparedAction) -> Result<ApplyOutcome, ApplyFailure> {
        Err(ApplyFailure {
            effects_may_have_occurred: false,
        })
    }
    async fn apply_execution(
        &self,
        prepared: &PreparedAction,
        execution_id: &str,
    ) -> Result<ApplyOutcome, ApplyFailure> {
        if !self.adapter.capabilities().apply {
            return Err(ApplyFailure {
                effects_may_have_occurred: false,
            });
        }
        let id = execution_id.to_owned();
        let binding = prepared.clone();
        let execution = self
            .runtime
            .with_store(move |store| {
                let execution = store
                    .get_execution(&id)?
                    .ok_or(OperatorStoreError::Conflict("execution claim missing"))?;
                if execution.value.status != ActionExecutionState::Running
                    || execution.value.spec_hash != binding.spec_hash
                    || execution.value.action_id != binding.spec.action_id
                    || execution.value.project != binding.spec.project
                {
                    return Err(OperatorStoreError::Conflict("execution claim changed"));
                }
                let slots = [("resource_receipt", id.clone(), 1)];
                if super::resources::retention::is_policy_action(&binding.spec) {
                    store.reserve_policy_terminal_documents(&binding.spec, &id, &slots)?;
                } else {
                    store.reserve_terminal_documents("resource_apply", &id, &slots)?;
                }
                Ok(execution)
            })
            .await
            .map_err(|_| ApplyFailure {
                effects_may_have_occurred: false,
            })?;
        let context = ResourceExecutionContext {
            runtime: self.runtime.clone(),
            execution,
            recovery: None,
        };
        let (receipt, mut partial_reason) = match context
            .clone()
            .scope(self.adapter.apply(&prepared.spec))
            .await
        {
            Ok(receipt) => (receipt, None),
            Err(ResourceError::KnownPartial { receipt, reason }) => (receipt, Some(reason)),
            Err(ResourceError::NotApplied(_)) => {
                return Err(ApplyFailure {
                    effects_may_have_occurred: false,
                })
            }
            Err(_) => {
                return Err(ApplyFailure {
                    effects_may_have_occurred: true,
                })
            }
        };
        if receipt.resource_id != prepared.spec.resource.resource_id {
            return Err(ApplyFailure {
                effects_may_have_occurred: true,
            });
        }
        let published = if partial_reason.is_some() {
            context.receipt_is_published(&prepared.spec, &receipt).await
        } else {
            context
                .publish_receipt(&prepared.spec, &receipt)
                .await
                .is_ok()
        };
        if !published && partial_reason.is_none() {
            if prepared.spec.adapter == super::improvement::activation::ADAPTER {
                partial_reason = Some(
                    "known landing receipt publication unavailable; inspect without replay".into(),
                );
            } else {
                return Err(ApplyFailure {
                    effects_may_have_occurred: true,
                });
            }
        }
        if receipt.state == ResourceApplyState::NotApplied {
            return Err(ApplyFailure {
                effects_may_have_occurred: false,
            });
        }
        let verified = partial_reason.is_none()
            && receipt.state == ResourceApplyState::Applied
            && self.adapter.capabilities().verify
            && prepared.spec.capabilities.can_verify
            && self
                .adapter
                .verify(&prepared.spec, &receipt.expected_post_version)
                .await
                .unwrap_or(false);
        let mut resource = prepared.spec.resource.clone();
        resource.version = receipt.expected_post_version.clone();
        let evidence = if verified {
            vec![format!(
                "resource_receipt:{}:{}",
                execution_id, receipt.expected_post_version
            )]
        } else {
            vec![]
        };
        let mut effect_evidence = evidence.clone();
        if let Some(reason) = partial_reason {
            effect_evidence.push(format!(
                "known_partial:{}:{}",
                execution_id,
                reason.chars().take(512).collect::<String>()
            ));
            effect_evidence.push(format!(
                "resource_material:{}:{}",
                execution_id, receipt.previous_material_reference
            ));
            if !published {
                effect_evidence.push("resource_receipt_publication_unavailable".into());
            }
        }
        let effects = vec![ResourceEffect {
            resource,
            state: EffectState::Applied,
            evidence_refs: effect_evidence,
        }];
        let recovery = (self.adapter.capabilities().recover
            && prepared.spec.capabilities.can_recover
            && !receipt.previous_material_reference.is_empty())
        .then(|| RecoveryRef {
            recovery_id: execution_id.to_owned(),
            action_id: prepared.spec.action_id.clone(),
            expected_post_version: receipt.expected_post_version,
            available: published,
        });
        if verified {
            Ok(ApplyOutcome::CompletedWithRecovery {
                effects,
                verification_evidence: evidence,
                recovery,
            })
        } else {
            Ok(ApplyOutcome::Partial {
                effects,
                verification_evidence: evidence,
                recovery,
            })
        }
    }
}

pub async fn load_resource_receipt(
    runtime: &OutcomeRuntime,
    execution_id: String,
) -> Result<Option<ResourceApplyReceipt>, OperatorStoreError> {
    runtime
        .with_store(move |store| {
            Ok(store
                .get_document::<ResourceDocument<ResourceApplyReceipt>>(
                    "resource_receipt",
                    &execution_id,
                )?
                .map(|stored| stored.value.payload))
        })
        .await
}

impl ResourceCatalog {
    pub async fn recover(
        &self,
        runtime: OutcomeRuntime,
        authorizer: Arc<dyn Authorizer>,
        request: &super::action_recovery::RecoveryRequest,
    ) -> super::action_recovery::RecoveryResult {
        use super::action_recovery::{
            RecoveryResult, RecoveryRejection, RecoveryReceipt, verified_recovery,
            recovery_execution,
        };
        let original = match runtime
            .get_execution(request.original_execution_id.clone())
            .await
        {
            Ok(Some(value)) => value.value,
            Ok(None) => return RecoveryResult::Rejected(RecoveryRejection::OriginalNotFound),
            Err(_) => return RecoveryResult::Rejected(RecoveryRejection::RepositoryFailure),
        };
        let recovery = match verified_recovery(&original, request) {
            Ok(value) => value.clone(),
            Err(error) => return RecoveryResult::Rejected(error),
        };
        if super::action_execution::action_spec_hash(
            &request.original_spec,
            &request.original_bound_parameter_ref,
        ) != original.spec_hash
            || request.original_spec.parameter_ref != request.original_bound_parameter_ref
            || request.original_spec.project != original.project
        {
            return RecoveryResult::Rejected(RecoveryRejection::OriginalSpecDoesNotMatch);
        }
        if !authorizer.authorize(
            &request.original_spec,
            &request.requester,
            request.decision.as_ref(),
        ) {
            return RecoveryResult::Rejected(RecoveryRejection::AuthorizationDenied);
        }
        let receipt_for = |record: ExecutionRecord| {
            let restored_version = (record.execution.status == ActionExecutionState::Completed)
                .then(|| {
                    record
                        .effects
                        .first()
                        .map(|e| e.resource.version.clone())
                        .unwrap_or_else(|| request.original_spec.expected_version.clone())
                });
            RecoveryReceipt {
                receipt: record,
                original_execution_id: original.execution_id.clone(),
                recovery_id: recovery.recovery_id.clone(),
                restored_version,
            }
        };
        let repository = RuntimeActionRepository(runtime.clone());
        let mut finished = record(recovery_execution(request, &original));
        match repository.lookup(&request.idempotency_key).await {
            Ok(Some(existing)) if existing.execution.spec_hash == finished.execution.spec_hash => {
                return RecoveryResult::Duplicate(receipt_for(existing))
            }
            Ok(Some(_)) => return RecoveryResult::Rejected(RecoveryRejection::IdempotencyConflict),
            Err(_) => return RecoveryResult::Rejected(RecoveryRejection::RepositoryFailure),
            Ok(None) => {}
        }
        let Some((kind, adapter)) = self.adapters.get(&request.original_spec.adapter) else {
            return RecoveryResult::Rejected(RecoveryRejection::AdapterNotRegistered);
        };
        if kind != &request.original_spec.kind
            || !adapter.capabilities().recover
            || !request.original_spec.capabilities.can_recover
        {
            return RecoveryResult::Rejected(RecoveryRejection::OriginalNotRecoverable);
        }
        let receipt_id = if recovery.recovery_id == original.execution_id {
            original.execution_id.clone()
        } else if recovery.recovery_id == original.spec_hash {
            let original_id = original.execution_id.clone();
            if !matches!(
                runtime
                    .with_store(move |store| store.is_legacy_resource_execution(&original_id))
                    .await,
                Ok(true)
            ) {
                return RecoveryResult::Rejected(RecoveryRejection::RecoveryIdentityDoesNotMatch);
            }
            original.spec_hash.clone()
        } else {
            return RecoveryResult::Rejected(RecoveryRejection::RecoveryIdentityDoesNotMatch);
        };
        let material = match load_resource_receipt(&runtime, receipt_id).await {
            Ok(Some(receipt))
                if receipt.expected_post_version == recovery.expected_post_version
                    && receipt.resource_id == request.original_spec.resource.resource_id =>
            {
                receipt
            }
            _ => {
                return RecoveryResult::Rejected(RecoveryRejection::RecoveryUnavailable {
                    reason: "durable resource receipt unavailable".into(),
                })
            }
        };
        if adapter
            .verify(&request.original_spec, &recovery.expected_post_version)
            .await
            != Ok(true)
        {
            return RecoveryResult::Rejected(RecoveryRejection::CurrentVersionMismatch);
        }
        let descriptor = super::resources::RecoveryDescriptor {
            previous_material_reference: material.previous_material_reference,
            expected_post_version: material.expected_post_version,
        };
        let restored_version = match adapter
            .recovered_version(&request.original_spec, &descriptor)
            .await
        {
            Ok(version) => version,
            Err(_) => return RecoveryResult::Rejected(RecoveryRejection::CurrentVersionMismatch),
        };
        let claim = finished.clone();
        let original_id = original.execution_id.clone();
        match runtime
            .with_store(move |store| {
                match store.claim_resource_recovery_execution(&claim.execution, &original_id)? {
                    super::operator_store::ExecutionClaim::Claimed(_) => Ok(ClaimResult::Claimed),
                    super::operator_store::ExecutionClaim::Existing(stored) => {
                        Ok(ClaimResult::Existing(record(stored.value)))
                    }
                }
            })
            .await
        {
            Ok(ClaimResult::Existing(existing)) => {
                return RecoveryResult::Duplicate(receipt_for(existing))
            }
            Ok(ClaimResult::Claimed) => {}
            Err(_) => return RecoveryResult::Rejected(RecoveryRejection::IdempotencyConflict),
        }
        if repository
            .transition(
                &request.execution_id,
                ActionExecutionState::Claimed,
                ActionExecutionState::Running,
            )
            .await
            .is_err()
        {
            return RecoveryResult::Rejected(RecoveryRejection::RepositoryFailure);
        }
        let result = if request.original_spec.adapter == super::improvement::activation::ADAPTER {
            let execution = match runtime.get_execution(request.execution_id.clone()).await {
                Ok(Some(execution)) => execution,
                _ => return RecoveryResult::Rejected(RecoveryRejection::RepositoryFailure),
            };
            ResourceExecutionContext {
                runtime: runtime.clone(),
                execution,
                recovery: Some(request.clone()),
            }
            .scope(adapter.recover(&request.original_spec, &descriptor))
            .await
        } else {
            adapter.recover(&request.original_spec, &descriptor).await
        };
        match result {
            Ok(state) => {
                let document = ResourceDocument {
                    schema_version: 1,
                    project: original.project.clone(),
                    payload: ResourceApplyReceipt {
                        resource_id: request.original_spec.resource.resource_id.clone(),
                        previous_material_reference: String::new(),
                        expected_post_version: restored_version.clone(),
                        state: state.clone(),
                    },
                };
                let id = request.execution_id.clone();
                if runtime
                    .with_store(move |store| {
                        store.put_reserved_document(
                            "resource_apply",
                            &id,
                            "resource_receipt",
                            &id,
                            &document,
                            None,
                        )
                    })
                    .await
                    .is_err()
                {
                    finished.execution.status = ActionExecutionState::OutcomeUnknown;
                    finished.execution.ended_at = Some(chrono::Utc::now().to_rfc3339());
                    if repository.finish(finished.clone()).await.is_err() {
                        return RecoveryResult::Rejected(RecoveryRejection::RepositoryFailure);
                    }
                    return RecoveryResult::Finished(receipt_for(finished));
                }
                let verified = state == ResourceApplyState::Applied
                    && adapter
                        .verify(&request.original_spec, &restored_version)
                        .await
                        == Ok(true);
                finished.execution.status = if verified {
                    ActionExecutionState::Completed
                } else if state == ResourceApplyState::NotApplied {
                    ActionExecutionState::Failed
                } else {
                    ActionExecutionState::Partial
                };
                if state != ResourceApplyState::NotApplied {
                    let mut resource = request.original_spec.resource.clone();
                    resource.version = restored_version.clone();
                    if verified {
                        finished.verification_evidence.push(format!(
                            "restored:{}:{}",
                            original.spec_hash, resource.version
                        ));
                    }
                    finished.effects.push(ResourceEffect {
                        resource,
                        state: EffectState::Applied,
                        evidence_refs: finished.verification_evidence.clone(),
                    });
                }
            }
            Err(_) => finished.execution.status = ActionExecutionState::OutcomeUnknown,
        }
        finished.execution.ended_at = Some(chrono::Utc::now().to_rfc3339());
        finished.execution.effects = finished.effects.clone();
        finished.execution.verification_evidence = finished.verification_evidence.clone();
        if repository.finish(finished.clone()).await.is_err() {
            return RecoveryResult::Rejected(RecoveryRejection::RepositoryFailure);
        }
        RecoveryResult::Finished(receipt_for(finished))
    }
}

pub async fn persist_draft<D: Serialize + serde::de::DeserializeOwned + Clone + Send + 'static>(
    runtime: &OutcomeRuntime,
    reference: String,
    draft: D,
) -> Result<(), OperatorStoreError> {
    let value = ResourceDocument {
        schema_version: 1,
        project: runtime.project(),
        payload: draft,
    };
    runtime
        .with_store(move |store| {
            store
                .put_document("resource_draft", &reference, &value, None)
                .map(|_| ())
        })
        .await
}

pub async fn load_draft<D: Serialize + serde::de::DeserializeOwned + Clone + Send + 'static>(
    runtime: &OutcomeRuntime,
    reference: String,
) -> Result<Option<D>, OperatorStoreError> {
    runtime
        .with_store(move |store| {
            Ok(store
                .get_document::<ResourceDocument<D>>("resource_draft", &reference)?
                .map(|value| value.value.payload))
        })
        .await
}

pub struct BoundDraft<D> {
    reference: String,
    draft: D,
}
impl<D> BoundDraft<D> {
    pub fn new(reference: String, draft: D) -> Self {
        Self { reference, draft }
    }
}
impl<D: Clone + Send + Sync> super::resources::DraftRegistry<D> for BoundDraft<D> {
    fn get(&self, reference: &str) -> Result<D, ResourceError> {
        if self.reference == reference {
            Ok(self.draft.clone())
        } else {
            Err(ResourceError::ParameterReferenceMissing)
        }
    }
}

pub struct PrivateRecoveryMaterials {
    root: std::path::PathBuf,
}
struct LazyRecoveryMaterials(ProjectIdentity);
impl super::resources::RecoveryMaterialStore for LazyRecoveryMaterials {
    fn put(&self, bytes: &[u8]) -> Result<String, ResourceError> {
        super::resources::RecoveryMaterialStore::put(
            &PrivateRecoveryMaterials::open(&self.0)?,
            bytes,
        )
    }
    fn get(&self, reference: &str) -> Result<Vec<u8>, ResourceError> {
        super::resources::RecoveryMaterialStore::get(
            &PrivateRecoveryMaterials::open(&self.0)?,
            reference,
        )
    }
}
impl PrivateRecoveryMaterials {
    pub fn open(project: &ProjectIdentity) -> Result<Self, ResourceError> {
        let project_root = std::fs::canonicalize(&project.canonical_root)?;
        if project_root.to_string_lossy() != project.canonical_root {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        let mut root = project_root;
        for component in [".refact", "operator-recovery"] {
            root.push(component);
            match std::fs::symlink_metadata(&root) {
                Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                    return Err(ResourceError::SymlinkNotAllowed)
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Self { root })
    }
    fn path(&self, reference: &str) -> Result<std::path::PathBuf, ResourceError> {
        if reference.len() != 64 || !reference.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ResourceError::MaterialUnavailable);
        }
        if std::fs::symlink_metadata(&self.root)?
            .file_type()
            .is_symlink()
        {
            return Err(ResourceError::SymlinkNotAllowed);
        }
        let path = self.root.join(reference);
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(ResourceError::SymlinkNotAllowed);
            }
        }
        Ok(path)
    }
}
impl super::resources::RecoveryMaterialStore for PrivateRecoveryMaterials {
    fn put(&self, bytes: &[u8]) -> Result<String, ResourceError> {
        use std::io::Write;
        // Creation is deferred to the authorized effect path: constructing a catalog
        // for preview must not create resource directories.
        let parent = self
            .root
            .parent()
            .ok_or(ResourceError::PathEscapesProject)?;
        for directory in [parent, self.root.as_path()] {
            match std::fs::create_dir(directory) {
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(
                            directory,
                            std::fs::Permissions::from_mode(0o700),
                        )?;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = std::fs::symlink_metadata(directory)?;
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(ResourceError::SymlinkNotAllowed);
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        let reference = super::resources::customization::digest(bytes);
        let path = self.path(&reference)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::fs::read(&path)? != bytes {
                    return Err(ResourceError::MaterialUnavailable);
                }
            }
            Err(error) => return Err(error.into()),
        }
        Ok(reference)
    }
    fn get(&self, reference: &str) -> Result<Vec<u8>, ResourceError> {
        let bytes = std::fs::read(self.path(reference)?)?;
        if super::resources::customization::digest(&bytes) != reference {
            return Err(ResourceError::MaterialUnavailable);
        }
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExactActionGrant {
    #[serde(default)]
    pub native_rights: Option<(String, serde_json::Value)>,
    pub spec_hash: String,
    pub requester: refact_buddy_core::operator_types::NativeSourceIdentity,
    pub decision: Option<refact_buddy_core::operator_types::DecisionReference>,
    pub expires_at: String,
    pub revoked: bool,
    /// Consent to restore this original execution is never consent to apply it again.
    #[serde(default)]
    pub recovery_execution_id: Option<String>,
}

impl Authorizer for ExactActionGrant {
    fn authorize(
        &self,
        spec: &ActionSpec,
        requester: &refact_buddy_core::operator_types::NativeSourceIdentity,
        decision: Option<&refact_buddy_core::operator_types::DecisionReference>,
    ) -> bool {
        !self.revoked
            && &self.requester == requester
            && self.decision.as_ref() == decision
            && self.spec_hash
                == super::action_execution::action_spec_hash(spec, &spec.parameter_ref)
            && chrono::DateTime::parse_from_rfc3339(&self.expires_at)
                .is_ok_and(|expiry| expiry > chrono::Utc::now())
    }
}

struct GrantedResourceRights(ExactActionGrant);
impl crate::ext::operator_service::ResourceRights for GrantedResourceRights {
    fn authorize(
        &self,
        spec: &ActionSpec,
        proposed_version: &str,
        requirement: &serde_json::Value,
    ) -> Result<(), ResourceError> {
        if self
            .0
            .authorize(spec, &self.0.requester, self.0.decision.as_ref())
            && self.0.recovery_execution_id.is_none()
            && self
                .0
                .native_rights
                .as_ref()
                .is_some_and(|(post, rights)| post == proposed_version && rights == requirement)
        {
            Ok(())
        } else {
            Err(ResourceError::ValidationFailed(
                "exact native resource grant denied".into(),
            ))
        }
    }
}

pub async fn native_rights(
    runtime: &OutcomeRuntime,
    spec: &ActionSpec,
) -> Result<Option<(String, serde_json::Value)>, ResourceError> {
    use super::resources::{extensions::*, hooks::*, integrations::*};
    // Connected MCP kinds are authorized against the probe's own exact requirement, not
    // against inert disabled-config preparation.
    if spec.adapter == "integrations"
        && matches!(
            spec.kind.as_str(),
            "mcp_capability_probe" | "mcp_enabled_reload"
        )
    {
        let draft: Arc<BoundDraft<McpDraft>> = bound_draft(runtime, spec).await?;
        let spec = spec.clone();
        return tokio::task::spawn_blocking(move || {
            let draft = super::resources::DraftRegistry::get(
                draft.as_ref(),
                spec.parameter_ref
                    .as_deref()
                    .ok_or(ResourceError::ParameterReferenceMissing)?,
            )?;
            let prepared = crate::integrations::operator_probe::prepare(&spec, &draft)?;
            Ok(Some((prepared.post, prepared.requirement)))
        })
        .await
        .map_err(|e| ResourceError::Io(e.to_string()))?;
    }
    let preparation: Box<dyn ResourcePreparation> = match spec.adapter.as_str() {
        "hooks" => Box::new(HookPreparation::new(
            runtime.project(),
            bound_draft(runtime, spec).await?,
        )),
        "integrations" => Box::new(McpPreparation::new(
            runtime.project(),
            bound_draft(runtime, spec).await?,
        )),
        "extensions" => Box::new(MarkdownPreparation::new(
            runtime.project(),
            bound_draft(runtime, spec).await?,
        )),
        _ => return Ok(None),
    };
    let spec = spec.clone();
    tokio::task::spawn_blocking(move || {
        let prepared = preparation.prepare(&spec)?;
        Ok(Some((
            crate::ext::operator_service::version(&Some(prepared.after)),
            prepared.rights,
        )))
    })
    .await
    .map_err(|e| ResourceError::Io(e.to_string()))?
}

pub async fn persist_grant(
    runtime: &OutcomeRuntime,
    id: String,
    grant: ExactActionGrant,
) -> Result<(), OperatorStoreError> {
    let value = ResourceDocument {
        schema_version: 1,
        project: runtime.project(),
        payload: grant,
    };
    runtime
        .with_store(move |store| {
            if value.payload.recovery_execution_id.is_some() {
                return store.put_recovery_grant(&id, &value);
            }
            store
                .put_document("action_grant", &id, &value, None)
                .map(|_| ())
        })
        .await
}

pub async fn load_grant(
    runtime: &OutcomeRuntime,
    id: String,
) -> Result<Option<ExactActionGrant>, OperatorStoreError> {
    runtime
        .with_store(move |store| {
            Ok(store
                .get_document::<ResourceDocument<ExactActionGrant>>("action_grant", &id)?
                .map(|value| value.value.payload))
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::action_execution::{ActionRequest, ActionResult};
    use super::super::resources::{ResourceCapabilities, RecoveryDescriptor};
    use refact_buddy_core::operator_types::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Adapter {
        version: std::sync::Mutex<String>,
        applied: AtomicUsize,
        fail_apply: bool,
        state: ResourceApplyState,
        reject_recovery: std::sync::atomic::AtomicBool,
    }
    #[async_trait::async_trait]
    impl AsyncResourceAdapter for Adapter {
        fn adapter_id(&self) -> &str {
            "test"
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
            tokio::task::yield_now().await;
            if *self.version.lock().unwrap() != spec.expected_version {
                return Err(ResourceError::VersionMismatch);
            }
            Ok(ResourcePreview {
                resource_id: spec.resource.resource_id.clone(),
                expected_version: spec.expected_version.clone(),
                proposed_version: "2".into(),
                reload_needed: false,
                diff: Some("1 -> 2".into()),
            })
        }
        async fn apply(&self, spec: &ActionSpec) -> Result<ResourceApplyReceipt, ResourceError> {
            tokio::task::yield_now().await;
            self.applied.fetch_add(1, Ordering::SeqCst);
            if self.fail_apply {
                return Err(ResourceError::Io("effect outcome unknown".into()));
            }
            if self.state == ResourceApplyState::NotApplied {
                return Ok(ResourceApplyReceipt {
                    resource_id: spec.resource.resource_id.clone(),
                    previous_material_reference: String::new(),
                    expected_post_version: spec.expected_version.clone(),
                    state: self.state.clone(),
                });
            }
            *self.version.lock().unwrap() = "2".into();
            Ok(ResourceApplyReceipt {
                resource_id: spec.resource.resource_id.clone(),
                previous_material_reference: "sealed".into(),
                expected_post_version: "2".into(),
                state: ResourceApplyState::Applied,
            })
        }
        async fn verify(&self, _: &ActionSpec, version: &str) -> Result<bool, ResourceError> {
            Ok(*self.version.lock().unwrap() == version)
        }
        async fn recovered_version(
            &self,
            spec: &ActionSpec,
            _: &RecoveryDescriptor,
        ) -> Result<String, ResourceError> {
            if self.reject_recovery.load(Ordering::SeqCst) {
                Err(ResourceError::VersionMismatch)
            } else {
                Ok(spec.expected_version.clone())
            }
        }
        async fn recover(
            &self,
            _: &ActionSpec,
            _: &RecoveryDescriptor,
        ) -> Result<ResourceApplyState, ResourceError> {
            *self.version.lock().unwrap() = "1".into();
            Ok(ResourceApplyState::Applied)
        }
    }
    fn request(project: ProjectIdentity) -> ActionRequest {
        ActionRequest {
            execution_id: "execution".into(),
            idempotency_key: "key".into(),
            bound_parameter_ref: Some("draft".into()),
            requester: NativeSourceIdentity {
                source_kind: NativeSourceKind::Chat,
                source_id: "requester".into(),
                source_version: "1".into(),
            },
            decision: None,
            worker: WorkerIdentity {
                worker_id: "worker".into(),
                engine_instance_id: "engine".into(),
                epoch: 1,
            },
            spec: ActionSpec {
                schema_version: 1,
                action_id: "action".into(),
                kind: "change".into(),
                adapter: "test".into(),
                project,
                resource: ResourceRef {
                    resource_kind: "test".into(),
                    resource_id: "resource".into(),
                    version: "1".into(),
                },
                expected_version: "1".into(),
                parameter_ref: Some("draft".into()),
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
    #[tokio::test]
    async fn real_catalog_preview_has_no_writes_and_draft_revision_cannot_inherit_preparation() {
        let root = tempfile::tempdir().unwrap();
        let project_root = root.path().join("project");
        std::fs::create_dir(&project_root).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.path().join("cache"),
            root.path().join("config"),
        )
        .await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let project = ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: std::fs::canonicalize(&project_root)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let mut req = request(project.clone());
        req.spec.adapter = "extensions".into();
        req.spec.kind = "markdown_project_apply".into();
        req.spec.resource = ResourceRef {
            resource_kind: "native_markdown".into(),
            resource_id: "commands/fixture".into(),
            version: "absent".into(),
        };
        req.spec.expected_version = "absent".into();
        let runtime =
            OutcomeRuntime::open(&project_root, project.clone(), req.worker.clone()).unwrap();
        let draft = serde_json::json!({"project":project,"kind":"command","name":"fixture","description":"fixture command","body":"Only fixture text"});
        persist_draft(&runtime, "draft".into(), draft.clone())
            .await
            .unwrap();
        let catalog = registered_catalog(app, &runtime, &req.spec).await.unwrap();
        let preview = catalog.preview(&req.spec).await.unwrap();
        assert_eq!(preview.expected_version, "absent");
        assert!(!project_root.join(".refact/commands/fixture.md").exists());
        assert!(!project_root.join(".refact/operator-recovery").exists());
        let prepared = PreparedAction {
            spec_hash: super::super::action_execution::action_spec_hash(
                &req.spec,
                &req.bound_parameter_ref,
            ),
            spec: req.spec,
            bound_parameter_ref: req.bound_parameter_ref,
        };
        bind_prepared_draft(&runtime, &prepared).await.unwrap();
        validate_prepared_draft(&runtime, &prepared).await.unwrap();
        runtime
            .with_store(move |store| {
                let stored = store
                    .get_document::<ResourceDocument<serde_json::Value>>("resource_draft", "draft")?
                    .unwrap();
                let mut value = stored.value;
                value.payload["body"] = serde_json::json!("Changed after review");
                store.put_document("resource_draft", "draft", &value, Some(stored.revision))?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(validate_prepared_draft(&runtime, &prepared).await.is_err());
        runtime.shutdown().await;
    }
    #[tokio::test]
    async fn changed_post_version_retry_and_recovery_reopen_retain_full_receipts() {
        let temp = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "project".into(),
            canonical_root: std::fs::canonicalize(temp.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let request = request(project.clone());
        let runtime =
            OutcomeRuntime::open(temp.path(), project.clone(), request.worker.clone()).unwrap();
        let adapter = Arc::new(Adapter {
            version: std::sync::Mutex::new("1".into()),
            applied: AtomicUsize::new(0),
            fail_apply: false,
            state: ResourceApplyState::Applied,
            reject_recovery: std::sync::atomic::AtomicBool::new(false),
        });
        let mut catalog = ResourceCatalog::default();
        catalog.register("change".into(), adapter.clone()).unwrap();
        let grant = Arc::new(ExactActionGrant {
            native_rights: None,
            recovery_execution_id: None,
            spec_hash: super::super::action_execution::action_spec_hash(
                &request.spec,
                &request.bound_parameter_ref,
            ),
            requester: request.requester.clone(),
            decision: None,
            expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            revoked: false,
        });
        let service = catalog.service(runtime.clone(), grant.clone()).unwrap();
        let first = match service.execute_async(&request).await {
            ActionResult::Finished(value) => value,
            other => panic!("{other:?}"),
        };
        assert_eq!(first.execution.status, ActionExecutionState::Completed);
        assert!(first.recovery.is_some());
        assert_eq!(first.execution.effects, first.effects);
        let second = service.execute_async(&request).await;
        assert!(matches!(second, ActionResult::Duplicate(value) if value == first));
        assert_eq!(adapter.applied.load(Ordering::SeqCst), 1);
        assert_eq!(
            load_resource_receipt(&runtime, request.execution_id.clone())
                .await
                .unwrap()
                .unwrap()
                .expected_post_version,
            "2"
        );
        assert!(
            load_resource_receipt(&runtime, first.execution.spec_hash.clone())
                .await
                .unwrap()
                .is_none()
        );
        runtime
            .with_store(|store| {
                store.set_document_growth_policy(Some(
                    super::super::operator_observability::DocumentGrowthPolicy {
                        max_rows_per_namespace: 1,
                        replace_editorial_aggregate: false,
                    },
                ))?;
                store.set_capacity_policy(Some(
                    super::super::operator_observability::CapacityPolicy {
                        max_rows_per_table: 1,
                        max_database_bytes: 1,
                    },
                ))
            })
            .await
            .unwrap();
        let recovery = super::super::action_recovery::RecoveryRequest {
            execution_id: "restore".into(),
            idempotency_key: "restore-key".into(),
            original_execution_id: request.execution_id.clone(),
            original_spec: request.spec.clone(),
            original_bound_parameter_ref: request.bound_parameter_ref.clone(),
            recovery_id: first.recovery.unwrap().recovery_id,
            expected_post_version: "2".into(),
            requester: request.requester.clone(),
            decision: None,
            worker: request.worker.clone(),
        };
        adapter.reject_recovery.store(true, Ordering::SeqCst);
        assert!(matches!(
            catalog
                .recover(runtime.clone(), grant.clone(), &recovery)
                .await,
            super::super::action_recovery::RecoveryResult::Rejected(
                super::super::action_recovery::RecoveryRejection::CurrentVersionMismatch
            )
        ));
        assert!(runtime
            .get_execution(recovery.execution_id.clone())
            .await
            .unwrap()
            .is_none());
        adapter.reject_recovery.store(false, Ordering::SeqCst);
        let restored = match catalog
            .recover(runtime.clone(), grant.clone(), &recovery)
            .await
        {
            super::super::action_recovery::RecoveryResult::Finished(value) => value,
            other => panic!("{other:?}"),
        };
        assert!(restored.receipt.execution.ended_at.is_some());
        assert_eq!(restored.receipt.effects.len(), 1);
        assert!(!restored.receipt.verification_evidence.is_empty());
        assert_eq!(
            load_resource_receipt(&runtime, recovery.execution_id.clone())
                .await
                .unwrap()
                .unwrap()
                .expected_post_version,
            "1"
        );
        let summary = runtime
            .with_store(|store| store.diagnostic_summary())
            .await
            .unwrap();
        assert_eq!(
            summary
                .document_growth
                .terminal_reservations
                .retained_entries,
            2
        );
        assert_eq!(
            summary
                .document_growth
                .terminal_reservations
                .outstanding_slots,
            0
        );
        runtime.shutdown().await;
        let reopened = OutcomeRuntime::open(temp.path(), project, request.worker.clone()).unwrap();
        assert!(
            matches!(catalog.recover(reopened.clone(), grant, &recovery).await, super::super::action_recovery::RecoveryResult::Duplicate(value) if value == restored)
        );
        reopened.shutdown().await;
    }

    fn git(root: &std::path::Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Drive the registered worktree_housekeeping adapter through the ACTUAL ActionService:
    /// saved typed draft -> registered_catalog -> preview -> exact grant -> execute_async ->
    /// durable receipt -> recover. Also proves the parent store's self-reference exceptions
    /// let an action's own draft/prepared/receipt rows through instead of self-blocking.
    #[tokio::test]
    async fn registered_worktree_archive_runs_through_real_action_service_without_self_blocking() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-b", "main"]);
        git(&root, &["config", "user.name", "Scratch"]);
        git(&root, &["config", "user.email", "scratch@example.invalid"]);
        std::fs::write(root.join("tracked"), "retained\n").unwrap();
        git(&root, &["add", "tracked"]);
        git(&root, &["commit", "-m", "initial"]);
        let root = root.canonicalize().unwrap();
        let project = ProjectIdentity {
            project_id: "scratch-registered-archive".into(),
            canonical_root: root.to_string_lossy().into_owned(),
        };

        // The catalog builds its own WorktreeService from app.paths.cache_dir, so the gcx cache
        // dir must be the same one the owner registry uses: this is the real registration path.
        let cache_dir = scratch.path().join("cache");
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache_dir.clone(),
            scratch.path().join("config"),
        )
        .await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let owner = crate::worktrees::service::WorktreeService::new(
            app.paths.cache_dir.clone(),
            root.clone(),
        )
        .unwrap();
        let created = owner
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/registered-archive".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = created.worktree.meta.id.clone();
        let checkout = created.worktree.meta.root.clone();

        let worker = WorkerIdentity {
            worker_id: "worker".into(),
            engine_instance_id: "engine".into(),
            epoch: 1,
        };
        let runtime = OutcomeRuntime::open(&root, project.clone(), worker.clone()).unwrap();
        let version = owner.preview_worktree_archive(&id).await.unwrap().version;

        let spec = ActionSpec {
            schema_version: 1,
            action_id: "archive-registered".into(),
            kind: "archive".into(),
            adapter: "worktree_housekeeping".into(),
            project: project.clone(),
            resource: ResourceRef {
                resource_kind: "registered_worktree".into(),
                resource_id: id.clone(),
                version: version.clone(),
            },
            expected_version: version.clone(),
            parameter_ref: Some("archive-draft".into()),
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
        };

        // Saved TYPED draft, exactly as the HTTP draft route persists it.
        let draft = super::super::resources::worktree_housekeeping::WorktreeHousekeepingDraft {
            operation:
                super::super::resources::worktree_housekeeping::WorktreeHousekeepingOperation::Archive,
            project: project.clone(),
            worktree_id: id.clone(),
        };
        persist_draft(&runtime, "archive-draft".into(), draft)
            .await
            .unwrap();

        // Unsupported operations must not become phantom-available on this adapter.
        let mut wrong_kind = spec.clone();
        wrong_kind.kind = "delete".into();
        assert!(registered_catalog(app.clone(), &runtime, &wrong_kind)
            .await
            .is_err());
        let mut unsupported_cancel = spec.clone();
        unsupported_cancel.capabilities.can_cancel = true;
        let sequence = runtime
            .with_store(|store| Ok(store.retention_preview()?.sequence_high_watermark))
            .await
            .unwrap();
        assert!(matches!(
            registered_catalog(app.clone(), &runtime, &unsupported_cancel).await,
            Err(ResourceError::UnsupportedResource)
        ));
        assert_eq!(
            runtime
                .with_store(|store| Ok(store.retention_preview()?.sequence_high_watermark))
                .await
                .unwrap(),
            sequence
        );
        assert!(checkout.exists());
        let mut wrong_resource = spec.clone();
        wrong_resource.resource.resource_kind = "directory".into();
        assert!(registered_catalog(app.clone(), &runtime, &wrong_resource)
            .await
            .unwrap()
            .preview(&wrong_resource)
            .await
            .is_err());

        // Preview through the real catalog: no archive, no registry mutation, exact versions.
        let registry_before = std::fs::read(owner.registry_path()).unwrap();
        let catalog = registered_catalog(app.clone(), &runtime, &spec)
            .await
            .unwrap();
        let preview = catalog.preview(&spec).await.unwrap();
        assert_eq!(preview.expected_version, version);
        assert_eq!(preview.proposed_version, format!("archived:{version}"));
        assert!(preview.diff.unwrap().contains("NO deletion"));
        assert_eq!(
            registry_before,
            std::fs::read(owner.registry_path()).unwrap()
        );
        assert!(checkout.exists());

        // Gates stay closed without an exact grant.
        let prepared = PreparedAction {
            spec_hash: super::super::action_execution::action_spec_hash(&spec, &spec.parameter_ref),
            spec: spec.clone(),
            bound_parameter_ref: spec.parameter_ref.clone(),
        };
        bind_prepared_draft(&runtime, &prepared).await.unwrap();
        RuntimeActionRepository(runtime.clone())
            .prepare(prepared.clone())
            .await
            .unwrap();
        validate_prepared_draft(&runtime, &prepared).await.unwrap();

        let requester = NativeSourceIdentity {
            source_kind: NativeSourceKind::Chat,
            source_id: "operator".into(),
            source_version: "1".into(),
        };
        let request = ActionRequest {
            execution_id: "archive-execution".into(),
            idempotency_key: "archive-key".into(),
            bound_parameter_ref: spec.parameter_ref.clone(),
            requester: requester.clone(),
            decision: None,
            worker: worker.clone(),
            spec: spec.clone(),
        };
        let revoked = Arc::new(ExactActionGrant {
            native_rights: None,
            recovery_execution_id: None,
            spec_hash: prepared.spec_hash.clone(),
            requester: requester.clone(),
            decision: None,
            expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            revoked: true,
        });
        assert!(matches!(
            catalog
                .service(runtime.clone(), revoked)
                .unwrap()
                .execute_async(&request)
                .await,
            ActionResult::Rejected(
                super::super::action_execution::ActionRejection::AuthorizationDenied
            )
        ));
        assert!(checkout.exists(), "denied grant must not archive anything");
        assert_eq!(
            registry_before,
            std::fs::read(owner.registry_path()).unwrap()
        );

        // Exact grant: the actual ActionService applies, verifies and records a receipt.
        let grant = test_grant(&request);
        let finished = match catalog
            .service(runtime.clone(), grant.clone())
            .unwrap()
            .execute_async(&request)
            .await
        {
            ActionResult::Finished(record) => record,
            other => panic!("{other:?}"),
        };
        assert_eq!(finished.execution.status, ActionExecutionState::Completed);
        assert!(!checkout.exists(), "checkout moved into owner quarantine");
        assert!(
            crate::worktrees::git::branch_exists(&root, "refact/chat/registered-archive").unwrap()
        );
        let recovery_ref = finished.recovery.clone().expect("recoverable archive");
        assert_eq!(
            recovery_ref.expected_post_version,
            format!("archived:{version}")
        );
        let receipt = load_resource_receipt(&runtime, request.execution_id.clone())
            .await
            .unwrap()
            .expect("durable receipt");
        assert_eq!(receipt.resource_id, id);
        assert_eq!(receipt.state, ResourceApplyState::Applied);

        // Own draft/prepared/receipt rows are exempt from the store's reference scan, so a
        // second authorized attempt fails on native state, never on self-reference.
        let self_id = id.clone();
        let self_root = created.worktree.meta.root.to_string_lossy().into_owned();
        let self_branch = created.worktree.meta.branch.clone();
        assert!(
            !runtime
                .with_store(move |store| store.worktree_is_referenced(
                    &self_id,
                    &self_root,
                    self_branch.as_deref()
                ))
                .await
                .unwrap(),
            "exact own draft/prepared/receipt rows must not self-block archiving"
        );

        // Recovery restores the exact checkout through the same catalog. Consent to restore is
        // a DISTINCT grant: it carries its own decision and recovery_execution_id, and the
        // apply path refuses it, so restore consent is never re-apply consent.
        let restore_decision = DecisionReference {
            project: project.clone(),
            worker: worker.clone(),
            native: NativeSourceIdentity {
                source_kind: NativeSourceKind::Chat,
                source_id: "archive-restore-decision".into(),
                source_version: "1".into(),
            },
        };
        let recovery = super::super::action_recovery::RecoveryRequest {
            execution_id: "archive-restore".into(),
            idempotency_key: "archive-restore-key".into(),
            original_execution_id: request.execution_id.clone(),
            original_spec: spec.clone(),
            original_bound_parameter_ref: request.bound_parameter_ref.clone(),
            recovery_id: recovery_ref.recovery_id,
            expected_post_version: recovery_ref.expected_post_version,
            requester: requester.clone(),
            decision: Some(restore_decision.clone()),
            worker: worker.clone(),
        };
        let recovery_grant = Arc::new(ExactActionGrant {
            native_rights: None,
            recovery_execution_id: Some(request.execution_id.clone()),
            spec_hash: prepared.spec_hash.clone(),
            requester: requester.clone(),
            decision: Some(restore_decision),
            expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            revoked: false,
        });
        // The original apply grant does not authorize restore: its decision does not match.
        assert!(matches!(
            catalog.recover(runtime.clone(), grant, &recovery).await,
            super::super::action_recovery::RecoveryResult::Rejected(
                super::super::action_recovery::RecoveryRejection::AuthorizationDenied
            )
        ));
        let restored = match catalog
            .recover(runtime.clone(), recovery_grant, &recovery)
            .await
        {
            super::super::action_recovery::RecoveryResult::Finished(value) => value,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            restored.receipt.execution.status,
            ActionExecutionState::Completed
        );
        assert_eq!(restored.restored_version.as_deref(), Some(version.as_str()));
        assert_eq!(restored.receipt.effects.len(), 1);
        assert_eq!(
            restored.receipt.effects[0].resource.version, version,
            "a completed recovery receipt records the restored native version"
        );
        assert!(!restored.receipt.verification_evidence.is_empty());
        let recovery_receipt = load_resource_receipt(&runtime, recovery.execution_id.clone())
            .await
            .unwrap()
            .expect("durable recovery receipt");
        assert_eq!(recovery_receipt.resource_id, id);
        assert_eq!(recovery_receipt.state, ResourceApplyState::Applied);
        assert_eq!(
            recovery_receipt.expected_post_version,
            restored.restored_version.unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(checkout.join("tracked")).unwrap(),
            "retained\n"
        );
        assert!(owner.get_worktree(&id).await.is_ok());
        runtime.shutdown().await;
    }

    fn test_grant(request: &ActionRequest) -> Arc<ExactActionGrant> {
        Arc::new(ExactActionGrant {
            native_rights: None,
            recovery_execution_id: None,
            spec_hash: super::super::action_execution::action_spec_hash(
                &request.spec,
                &request.bound_parameter_ref,
            ),
            requester: request.requester.clone(),
            decision: request.decision.clone(),
            expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            revoked: false,
        })
    }

    #[tokio::test]
    async fn separately_authorized_same_spec_owns_distinct_execution_receipts() {
        let temp = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "project".into(),
            canonical_root: std::fs::canonicalize(temp.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let first = request(project.clone());
        let runtime = OutcomeRuntime::open(temp.path(), project, first.worker.clone()).unwrap();
        let adapter = Arc::new(Adapter {
            version: std::sync::Mutex::new("1".into()),
            applied: AtomicUsize::new(0),
            fail_apply: false,
            state: ResourceApplyState::Applied,
            reject_recovery: std::sync::atomic::AtomicBool::new(false),
        });
        let mut catalog = ResourceCatalog::default();
        catalog.register("change".into(), adapter.clone()).unwrap();
        assert!(matches!(
            catalog.service(runtime.clone(), test_grant(&first)).unwrap()
                .execute_async(&first).await,
            ActionResult::Finished(record) if record.execution.status == ActionExecutionState::Completed
        ));
        *adapter.version.lock().unwrap() = "1".into();
        let mut second = first.clone();
        second.execution_id = "execution-2".into();
        second.idempotency_key = "key-2".into();
        second.decision = Some(DecisionReference {
            project: second.spec.project.clone(),
            worker: second.worker.clone(),
            native: NativeSourceIdentity {
                source_kind: NativeSourceKind::Chat,
                source_id: "second-grant".into(),
                source_version: "1".into(),
            },
        });
        let service = catalog
            .service(runtime.clone(), test_grant(&second))
            .unwrap();
        let applied = match service.execute_async(&second).await {
            ActionResult::Finished(record) => record,
            other => panic!("{other:?}"),
        };
        assert_eq!(applied.execution.status, ActionExecutionState::Completed);
        assert_eq!(applied.recovery.unwrap().recovery_id, "execution-2");
        assert!(matches!(
            service.execute_async(&second).await,
            ActionResult::Duplicate(_)
        ));
        assert_eq!(adapter.applied.load(Ordering::SeqCst), 2);
        assert!(load_resource_receipt(&runtime, first.execution_id)
            .await
            .unwrap()
            .is_some());
        assert!(load_resource_receipt(&runtime, second.execution_id)
            .await
            .unwrap()
            .is_some());
        let reservations = runtime
            .with_store(|store| {
                Ok(store
                    .diagnostic_summary()?
                    .document_growth
                    .terminal_reservations)
            })
            .await
            .unwrap();
        assert_eq!(reservations.retained_entries, 2);
        assert_eq!(reservations.outstanding_slots, 0);
        runtime.shutdown().await;
    }
}
