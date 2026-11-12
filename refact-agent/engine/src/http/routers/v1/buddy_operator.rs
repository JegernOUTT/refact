use axum::extract::{Path, Query, State};
use axum::response::Result;
use hyper::StatusCode;
use refact_buddy_core::operator_types::{
    ActionExecution, ArtifactRevision, ProjectIdentity, ReadState, RunOutcome, WorkItem,
    WorkSnooze, OPERATOR_SCHEMA_VERSION_V1,
};
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::buddy::actor::{OperatorOpenFailureKind, OperatorUnavailableStatus};
use crate::buddy::operator_store::{OperatorStoreError, Page, Stored};
use crate::buddy::outcome_runtime::OutcomeRuntime;
use crate::custom_error::ScratchError;

const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: usize = 200;

#[derive(Serialize)]
pub struct OperatorStatus {
    schema_version: u16,
    mode: &'static str,
    features: crate::buddy::operator_runtime::OperatorFeatures,
    project: Option<ProjectIdentity>,
    worker: Option<refact_buddy_core::operator_types::WorkerIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unavailable_reason: Option<OperatorUnavailableReason>,
}

#[derive(Serialize)]
struct OperatorUnavailableReason {
    kind: &'static str,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_in_seconds: Option<u64>,
}

fn unavailable_reason(status: OperatorUnavailableStatus) -> OperatorUnavailableReason {
    OperatorUnavailableReason {
        kind: status.kind.as_str(),
        detail: status.detail,
        retry_in_seconds: status.retry_in_seconds,
    }
}

fn not_attempted_reason(detail: impl Into<String>) -> OperatorUnavailableReason {
    OperatorUnavailableReason {
        kind: OperatorOpenFailureKind::NotAttempted.as_str(),
        detail: detail.into(),
        retry_in_seconds: None,
    }
}

/// Read-only rollout discovery: an unavailable store must never enable legacy writers.
/// `mode` is a pure function of (settings.operator_enabled, runtime presence, project
/// scope): `disabled` only when the user turned the operator off, `enabled` only when
/// the durable store is open for the single active project root, `unavailable`
/// otherwise — including a service that has not started yet, a store that failed to
/// open, and a multi-root or mismatched workspace scope.
pub async fn handle_operator_status(
    State(app): State<AppState>,
) -> Result<axum::Json<OperatorStatus>, ScratchError> {
    let roots = crate::files_correction::get_unscoped_project_dirs(app.gcx.clone()).await;
    let buddy = app.buddy.buddy.lock().await;
    let service = buddy.as_ref();
    let features = service.map(|s| s.operator_features).unwrap_or_default();
    let current = service.filter(|s| roots.len() == 1 && roots[0] == s.project_root);
    let runtime = current.and_then(|s| s.outcome_runtime.as_ref());
    let mode = match service {
        Some(service) if !service.operator_features.storage => "disabled",
        Some(_) if runtime.is_some() => "enabled",
        _ => "unavailable",
    };
    let unavailable_reason = (mode == "unavailable").then(|| {
        current
            .map(|service| unavailable_reason(service.operator_unavailable_status()))
            .unwrap_or_else(|| {
                not_attempted_reason(
                    "Buddy needs one active project before it can open saved work.",
                )
            })
    });
    Ok(axum::Json(OperatorStatus {
        schema_version: 1,
        mode,
        features,
        project: runtime.map(OutcomeRuntime::project),
        worker: runtime.map(OutcomeRuntime::worker),
        unavailable_reason,
    }))
}

pub async fn handle_operator_reopen(
    State(app): State<AppState>,
) -> Result<axum::Json<OperatorStatus>, ScratchError> {
    crate::buddy::actor::reopen_operator_now(&app.buddy.buddy).await;
    handle_operator_status(State(app)).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationScope {
    pub project: ProjectIdentity,
    pub worker: refact_buddy_core::operator_types::WorkerIdentity,
}

/// Recovery exposes identifiers only, never retained report bodies. Both calls
/// are authenticated and bound to the live project and worker, not a stale tab.
pub async fn handle_pending_publications(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(scope): axum::Json<PublicationScope>,
) -> Result<
    axum::Json<OperatorEnvelope<Vec<crate::buddy::outcome_runtime::PendingPublicationInfo>>>,
    ScratchError,
> {
    authenticated_operator_request(&app, &headers).map_err(action_error)?;
    let runtime = gated_runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(action_error)?;
    let buddy = app.buddy.buddy.lock().await;
    validate_publication_owner(
        &runtime,
        buddy.as_ref().and_then(|s| s.outcome_runtime.as_ref()),
        &scope,
    )?;
    Ok(envelope(
        runtime.project(),
        runtime.pending_publications().map_err(action_error)?,
        None,
    ))
}

fn validate_publication_owner(
    runtime: &OutcomeRuntime,
    current: Option<&OutcomeRuntime>,
    scope: &PublicationScope,
) -> Result<(), ScratchError> {
    if runtime.is_closed()
        || current.is_none_or(|current| !runtime.same_owner(current))
        || scope.project != runtime.project()
        || scope.worker != runtime.worker()
    {
        return Err(action_error(
            "publication project or runtime owner changed; reload Buddy",
        ));
    }
    Ok(())
}

pub async fn handle_retry_publication(
    State(app): State<AppState>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
    axum::Json(scope): axum::Json<PublicationScope>,
) -> Result<
    axum::Json<OperatorEnvelope<Vec<crate::buddy::outcome_runtime::PendingPublicationInfo>>>,
    ScratchError,
> {
    authenticated_operator_request(&app, &headers).map_err(action_error)?;
    let runtime = gated_runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(action_error)?;
    // Pin the service owner until publication finishes. This path never invokes
    // a workflow, chat, tools, or housekeeping accounting.
    let buddy = app.buddy.buddy.lock().await;
    validate_publication_owner(
        &runtime,
        buddy.as_ref().and_then(|s| s.outcome_runtime.as_ref()),
        &scope,
    )?;
    runtime.retry_pending_publication(&run_id).await.map_err(|_| action_error("Retained publication could not be saved. It remains memory-only; resolve storage availability and retry before restarting."))?;
    Ok(envelope(
        runtime.project(),
        runtime.pending_publications().map_err(action_error)?,
        None,
    ))
}

use crate::buddy::resource_execution as resources;
use crate::buddy::action_execution::{
    ActionRequest, ActionResult, PreparedAction, AsyncActionRepository, Authorizer,
};
use crate::buddy::operator_runtime::{
    authenticated_operator_request, runtime as gated_runtime, OperatorCapability,
};
use refact_buddy_core::operator_types::{
    ActionSpec, DecisionReference, NativeSourceIdentity, NativeSourceKind,
};
use axum::http::HeaderMap;

fn action_error(error: impl std::fmt::Display) -> ScratchError {
    ScratchError::new(StatusCode::CONFLICT, error.to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareResourceRequest {
    pub spec: ActionSpec,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceDraftRequest {
    pub spec: ActionSpec,
    pub draft: serde_json::Value,
}

/// Drafts are immutable proposals, never permissions. A new revision needs a new reference.
pub async fn handle_resource_draft(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ResourceDraftRequest>,
) -> Result<axum::Json<OperatorEnvelope<crate::buddy::resources::ResourcePreview>>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(action_error)?;
    let _gate = resources::execution_gate().lock().await;
    let runtime = gated_runtime(&app, OperatorCapability::Actions)
        .await
        .map_err(action_error)?;
    if request.spec.project != runtime.project() || request.spec.schema_version != 1 {
        return Err(action_error("project/schema mismatch"));
    }
    let reference = request
        .spec
        .parameter_ref
        .clone()
        .ok_or_else(|| action_error("draft reference required"))?;
    // Decode before persistence to reject wrong families and strip no fields silently.
    macro_rules! typed {
        ($ty:ty) => {{
            let draft: $ty = serde_json::from_value(request.draft.clone()).map_err(action_error)?;
            let normalized = serde_json::to_value(&draft).map_err(action_error)?;
            if normalized != request.draft {
                return Err(action_error("draft must use the exact typed schema"));
            }
            resources::persist_draft(&runtime, reference, draft)
                .await
                .map_err(operator_error)?;
        }};
    }
    use crate::buddy::resources::*;
    match request.spec.adapter.as_str() {
        retention::ADAPTER => {
            if !retention::is_policy_action(&request.spec) {
                return Err(action_error("exact policy action identity required"));
            }
            let draft: retention::PolicyDraft =
                serde_json::from_value(request.draft.clone()).map_err(action_error)?;
            if serde_json::to_value(&draft).map_err(action_error)? != request.draft {
                return Err(action_error("exact policy draft schema required"));
            }
            resources::persist_policy_draft(&runtime, &request.spec, draft)
                .await
                .map_err(operator_error)?;
        }

        "settings" => typed!(settings::SettingsDraft),
        "buddy_settings" => typed!(settings::BuddySettingsDraft),
        "default_models" => typed!(default_models::DefaultModelDraft),
        "providers" => typed!(providers::ProviderDraft),
        "memory" => typed!(memory::MemoryDraft),
        "history" => typed!(history::HistoryDraft),
        "project_code" => typed!(project_code::ProjectCodeDraft),
        "candidate_installation" => {
            typed!(crate::buddy::improvement::installation::InstallationDraft)
        }
        "candidate_landing" => typed!(crate::buddy::improvement::activation::LandingDraft),
        "candidate_artifact" => typed!(crate::buddy::improvement::artifacts::ArtifactDraft),
        "candidate_artifact_cleanup" => {
            typed!(crate::buddy::improvement::artifacts::ArtifactCleanupDraft)
        }
        "hooks" => typed!(hooks::HookDraft),
        "integrations" => typed!(integrations::McpDraft),
        "worktree_housekeeping" => {
            typed!(worktree_housekeeping::WorktreeHousekeepingDraft)
        }
        "extensions" => typed!(extensions::MarkdownDraft),
        customization::ADAPTER_ID => typed!(customization::CustomizationDraft),
        _ => return Err(action_error("unregistered resource adapter")),
    }
    let catalog = resources::registered_catalog(app, &runtime, &request.spec)
        .await
        .map_err(|e| action_error(format!("{e:?}")))?;
    let preview = catalog
        .preview(&request.spec)
        .await
        .map_err(|e| action_error(format!("{e:?}")))?;
    Ok(envelope(runtime.project(), preview, None))
}

pub async fn handle_resource_rights(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<PrepareResourceRequest>,
) -> Result<axum::Json<OperatorEnvelope<Option<(String, serde_json::Value)>>>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(action_error)?;
    let runtime = gated_runtime(&app, OperatorCapability::Actions)
        .await
        .map_err(action_error)?;
    let catalog = resources::registered_catalog(app, &runtime, &request.spec)
        .await
        .map_err(|e| action_error(format!("{e:?}")))?;
    catalog
        .preview(&request.spec)
        .await
        .map_err(|e| action_error(format!("{e:?}")))?;
    let rights = resources::native_rights(&runtime, &request.spec)
        .await
        .map_err(|e| action_error(format!("{e:?}")))?;
    Ok(envelope(runtime.project(), rights, None))
}

pub async fn handle_resource_preview(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<PrepareResourceRequest>,
) -> Result<axum::Json<OperatorEnvelope<crate::buddy::resources::ResourcePreview>>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(action_error)?;
    let runtime = gated_runtime(&app, OperatorCapability::Actions)
        .await
        .map_err(action_error)?;
    let catalog = resources::registered_catalog(app, &runtime, &request.spec)
        .await
        .map_err(|e| action_error(format!("{e:?}")))?;
    let preview = catalog
        .preview(&request.spec)
        .await
        .map_err(|e| action_error(format!("{e:?}")))?;
    Ok(envelope(runtime.project(), preview, None))
}

pub async fn handle_resource_prepare(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<PrepareResourceRequest>,
) -> Result<axum::Json<OperatorEnvelope<PreparedAction>>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(action_error)?;
    let _gate = resources::execution_gate().lock().await;
    let runtime = gated_runtime(&app, OperatorCapability::Actions)
        .await
        .map_err(action_error)?;
    let catalog = resources::registered_catalog(app, &runtime, &request.spec)
        .await
        .map_err(|e| action_error(format!("{e:?}")))?;
    catalog
        .preview(&request.spec)
        .await
        .map_err(|e| action_error(format!("{e:?}")))?;
    let prepared = PreparedAction {
        spec_hash: crate::buddy::action_execution::action_spec_hash(
            &request.spec,
            &request.spec.parameter_ref,
        ),
        bound_parameter_ref: request.spec.parameter_ref.clone(),
        spec: request.spec,
    };
    resources::bind_prepared_draft(&runtime, &prepared)
        .await
        .map_err(operator_error)?;
    resources::RuntimeActionRepository(runtime.clone())
        .prepare(prepared.clone())
        .await
        .map_err(|_| action_error("preparation persistence failed"))?;
    Ok(envelope(runtime.project(), prepared, None))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantResourceRequest {
    pub project_id: String,
    pub spec_hash: String,
    pub expires_in_seconds: u32,
    pub recovery_execution_id: Option<String>,
    pub native_rights: Option<(String, serde_json::Value)>,
}

pub async fn handle_resource_grant(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<GrantResourceRequest>,
) -> Result<axum::Json<OperatorEnvelope<resources::ExactActionGrant>>, ScratchError> {
    let requester = authenticated_operator_request(&app, &headers).map_err(action_error)?;
    let _gate = resources::execution_gate().lock().await;
    let runtime = gated_runtime(&app, OperatorCapability::Actions)
        .await
        .map_err(action_error)?;
    if request.project_id != runtime.project().project_id
        || !(1..=3600).contains(&request.expires_in_seconds)
    {
        return Err(action_error("invalid project or grant duration"));
    }
    let prepared = resources::load_prepared(&runtime, request.spec_hash.clone())
        .await
        .map_err(action_error)?;
    if let Some(id) = &request.recovery_execution_id {
        let original = runtime
            .get_execution(id.clone())
            .await
            .map_err(operator_error)?
            .ok_or_else(|| action_error("original execution missing"))?
            .value;
        if original.spec_hash != prepared.spec_hash
            || original.recovery.as_ref().is_none_or(|r| !r.available)
        {
            return Err(action_error("original recovery unavailable"));
        }
    } else {
        resources::validate_prepared_draft(&runtime, &prepared)
            .await
            .map_err(action_error)?;
        let rights = resources::native_rights(&runtime, &prepared.spec)
            .await
            .map_err(|e| action_error(format!("{e:?}")))?;
        if rights != request.native_rights {
            return Err(action_error(
                "exact native command/tools/install rights must be confirmed",
            ));
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let grant = resources::ExactActionGrant {
        native_rights: request.native_rights,
        recovery_execution_id: request.recovery_execution_id,
        spec_hash: request.spec_hash,
        requester,
        decision: Some(DecisionReference {
            project: runtime.project(),
            worker: runtime.worker(),
            native: NativeSourceIdentity {
                source_kind: NativeSourceKind::External,
                source_id: id.clone(),
                source_version: "resource-grant-v1".into(),
            },
        }),
        expires_at: (chrono::Utc::now()
            + chrono::Duration::seconds(request.expires_in_seconds.into()))
        .to_rfc3339(),
        revoked: false,
    };
    resources::persist_grant(&runtime, id, grant.clone())
        .await
        .map_err(operator_error)?;
    Ok(envelope(runtime.project(), grant, None))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteResourceRequest {
    pub project_id: String,
    pub spec_hash: String,
    pub grant_id: String,
    pub execution_id: String,
    pub idempotency_key: String,
}

pub async fn handle_resource_execute(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ExecuteResourceRequest>,
) -> Result<axum::Json<OperatorEnvelope<ActionResult>>, ScratchError> {
    let requester = authenticated_operator_request(&app, &headers).map_err(action_error)?;
    let runtime = gated_runtime(&app, OperatorCapability::Actions)
        .await
        .map_err(action_error)?;
    if request.project_id != runtime.project().project_id {
        return Err(action_error("project changed"));
    }
    let prepared = resources::load_prepared(&runtime, request.spec_hash)
        .await
        .map_err(action_error)?;
    let grant = resources::load_grant(&runtime, request.grant_id)
        .await
        .map_err(operator_error)?
        .ok_or_else(|| action_error("grant missing"))?;
    let action = ActionRequest {
        execution_id: request.execution_id,
        idempotency_key: request.idempotency_key,
        spec: prepared.spec,
        bound_parameter_ref: prepared.bound_parameter_ref,
        requester,
        decision: grant.decision,
        worker: runtime.worker(),
    };
    let result = resources::execute_registered(app, runtime.clone(), action)
        .await
        .map_err(action_error)?;
    Ok(envelope(runtime.project(), result, None))
}

pub async fn handle_resource_grant_revoke(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ProjectQuery>,
) -> Result<axum::Json<OperatorEnvelope<resources::ExactActionGrant>>, ScratchError> {
    let requester = authenticated_operator_request(&app, &headers).map_err(action_error)?;
    let _gate = resources::execution_gate().lock().await;
    let runtime = gated_runtime(&app, OperatorCapability::Actions)
        .await
        .map_err(action_error)?;
    if request.project_id.as_deref() != Some(runtime.project().project_id.as_str()) {
        return Err(action_error("project changed"));
    }
    let grant = runtime
        .with_store(move |store| {
            let stored = store
                .get_document::<resources::ResourceDocument<resources::ExactActionGrant>>(
                    "action_grant",
                    &id,
                )?
                .ok_or(OperatorStoreError::Conflict("grant missing"))?;
            let mut value = stored.value;
            if value.payload.requester != requester {
                return Err(OperatorStoreError::Conflict("grant owner mismatch"));
            }
            value.payload.revoked = true;
            store.put_document("action_grant", &id, &value, Some(stored.revision))?;
            Ok(value.payload)
        })
        .await
        .map_err(operator_error)?;
    Ok(envelope(runtime.project(), grant, None))
}

pub async fn handle_resource_recover(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ExecuteResourceRequest>,
) -> Result<
    axum::Json<OperatorEnvelope<crate::buddy::action_execution::ExecutionRecord>>,
    ScratchError,
> {
    let requester = authenticated_operator_request(&app, &headers).map_err(action_error)?;
    let gate = resources::execution_gate().lock().await;
    let runtime = gated_runtime(&app, OperatorCapability::Actions)
        .await
        .map_err(action_error)?;
    if request.project_id != runtime.project().project_id {
        return Err(action_error("project changed"));
    }
    let original = runtime
        .get_execution(id.clone())
        .await
        .map_err(operator_error)?
        .ok_or_else(|| action_error("execution missing"))?
        .value;
    if original.spec_hash != request.spec_hash {
        return Err(action_error("original action mismatch"));
    }
    let prepared = resources::load_prepared(&runtime, request.spec_hash)
        .await
        .map_err(action_error)?;
    let grant = resources::load_grant(&runtime, request.grant_id)
        .await
        .map_err(operator_error)?
        .ok_or_else(|| action_error("grant missing"))?;
    if grant.recovery_execution_id.as_deref() != Some(id.as_str())
        || !grant.authorize(&prepared.spec, &requester, grant.decision.as_ref())
    {
        return Err(action_error("recovery grant denied"));
    }
    let recovery = original
        .recovery
        .ok_or_else(|| action_error("recovery unavailable"))?;
    let catalog = resources::registered_catalog(app, &runtime, &prepared.spec)
        .await
        .map_err(|e| action_error(format!("{e:?}")))?;
    let recovery_request = crate::buddy::action_recovery::RecoveryRequest {
        execution_id: request.execution_id,
        idempotency_key: request.idempotency_key,
        original_execution_id: id,
        original_spec: prepared.spec,
        original_bound_parameter_ref: prepared.bound_parameter_ref,
        recovery_id: recovery.recovery_id,
        expected_post_version: recovery.expected_post_version,
        requester,
        decision: grant.decision.clone(),
        worker: runtime.worker(),
    };
    let _gate = if recovery_request.original_spec.adapter
        == crate::buddy::improvement::activation::ADAPTER
    {
        drop(gate);
        None
    } else {
        Some(gate)
    };
    match catalog
        .recover(
            runtime.clone(),
            std::sync::Arc::new(grant),
            &recovery_request,
        )
        .await
    {
        crate::buddy::action_recovery::RecoveryResult::Finished(receipt)
        | crate::buddy::action_recovery::RecoveryResult::Duplicate(receipt) => {
            Ok(envelope(runtime.project(), receipt.receipt, None))
        }
        crate::buddy::action_recovery::RecoveryResult::Rejected(reason) => {
            Err(action_error(format!("{reason:?}")))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ProjectQuery {
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WorkListQuery {
    pub project_id: Option<String>,
    pub cursor: Option<u64>,
    pub limit: Option<usize>,
    pub view: Option<String>,
    pub search: Option<String>,
    pub attention: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ArtifactQuery {
    pub project_id: Option<String>,
    pub revision: Option<u32>,
}

#[derive(Serialize)]
pub struct OperatorEnvelope<T> {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub data: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<u64>,
}

fn page_size(limit: Option<usize>) -> Result<usize, ScratchError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_SIZE);
    if !(1..=MAX_PAGE_SIZE).contains(&limit) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "limit must be between 1 and 200".to_string(),
        ));
    }
    Ok(limit)
}

fn operator_error(error: OperatorStoreError) -> ScratchError {
    let status = match error {
        OperatorStoreError::InvalidInput(_) | OperatorStoreError::ProjectMismatch => {
            StatusCode::BAD_REQUEST
        }
        OperatorStoreError::SchemaTooNew(_) | OperatorStoreError::CorruptSchema(_) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        OperatorStoreError::AlreadyOwned(_) => StatusCode::SERVICE_UNAVAILABLE,
        OperatorStoreError::Conflict(_)
        | OperatorStoreError::StaleRevision {
            ..
        } => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ScratchError::new(status, error.to_string())
}

async fn operator_runtime(
    app: &AppState,
    requested_project_id: Option<&str>,
) -> Result<(OutcomeRuntime, ProjectIdentity), ScratchError> {
    let runtime = {
        let service = app.buddy.buddy.lock().await;
        service
            .as_ref()
            .and_then(|service| service.outcome_runtime.clone())
    }
    .ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy operator runtime is unavailable".to_string(),
        )
    })?;
    let project = runtime.project();
    if let Some(requested) = requested_project_id {
        if requested != project.project_id {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                "requested project does not match the active operator runtime".to_string(),
            ));
        }
    }
    Ok((runtime, project))
}

fn envelope<T>(
    project: ProjectIdentity,
    data: T,
    cursor: Option<u64>,
) -> axum::Json<OperatorEnvelope<T>> {
    axum::Json(OperatorEnvelope {
        schema_version: OPERATOR_SCHEMA_VERSION_V1,
        project,
        data,
        next_cursor: cursor,
    })
}

pub async fn handle_v1_buddy_operator_work_list(
    State(app): State<AppState>,
    Query(query): Query<WorkListQuery>,
) -> Result<axum::Json<OperatorEnvelope<Vec<Stored<WorkItem>>>>, ScratchError> {
    let limit = page_size(query.limit)?;
    let (runtime, project) = operator_runtime(&app, query.project_id.as_deref()).await?;
    let Page {
        entries,
        next_after,
    } = runtime
        .search_work_with_attention(
            query.cursor,
            limit,
            query.view,
            query.search,
            query.attention,
        )
        .await
        .map_err(operator_error)?;
    Ok(envelope(project, entries, next_after))
}

pub async fn handle_v1_buddy_operator_work_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Result<axum::Json<OperatorEnvelope<Stored<WorkItem>>>, ScratchError> {
    let (runtime, project) = operator_runtime(&app, query.project_id.as_deref()).await?;
    let item = runtime
        .get_work_item(id)
        .await
        .map_err(operator_error)?
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                "operator work item not found".to_string(),
            )
        })?;
    Ok(envelope(project, item, None))
}

/// Explicit legacy editor route adapter. No editor-session lookup or proposal creation.
pub async fn handle_legacy_draft_work(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Result<axum::Json<OperatorEnvelope<Stored<WorkItem>>>, ScratchError> {
    let runtime = gated_runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(action_error)?;
    let project = runtime.project();
    if query.project_id.as_deref() != Some(project.project_id.as_str()) {
        return Err(action_error("legacy proposal project mismatch"));
    }
    let subject = crate::buddy::work_items::SubjectKey::new(
        &project,
        "legacy_draft",
        &id,
        "legacy-import-v1",
        "1",
    )
    .map_err(|_| action_error("invalid legacy editor identity"))?;
    let item = runtime
        .get_work_item(subject.work_item_id())
        .await
        .map_err(operator_error)?
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                "No saved proposal is linked to this legacy editor".into(),
            )
        })?;
    Ok(envelope(project, item, None))
}

pub async fn handle_v1_buddy_operator_artifact_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ArtifactQuery>,
) -> Result<axum::Json<OperatorEnvelope<Stored<ArtifactRevision>>>, ScratchError> {
    let (runtime, project) = operator_runtime(&app, query.project_id.as_deref()).await?;
    let artifact = runtime
        .get_artifact_revision(id, query.revision)
        .await
        .map_err(operator_error)?
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                "operator artifact revision not found".to_string(),
            )
        })?;
    Ok(envelope(project, artifact, None))
}

pub async fn handle_v1_buddy_operator_execution_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Result<axum::Json<OperatorEnvelope<Stored<ActionExecution>>>, ScratchError> {
    let (runtime, project) = operator_runtime(&app, query.project_id.as_deref()).await?;
    let execution = runtime
        .get_execution(id)
        .await
        .map_err(operator_error)?
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                "operator execution not found".to_string(),
            )
        })?;
    Ok(envelope(project, execution, None))
}

pub async fn handle_v1_buddy_operator_outcome_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Result<axum::Json<OperatorEnvelope<Stored<RunOutcome>>>, ScratchError> {
    let (runtime, project) = operator_runtime(&app, query.project_id.as_deref()).await?;
    let outcome = runtime
        .get_run_outcome(id)
        .await
        .map_err(operator_error)?
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, "recorded run is unavailable".into())
        })?;
    Ok(envelope(project, outcome, None))
}

pub async fn handle_v1_buddy_operator_artifact_history(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WorkListQuery>,
) -> Result<axum::Json<OperatorEnvelope<Vec<Stored<ArtifactRevision>>>>, ScratchError> {
    let (runtime, project) = operator_runtime(&app, query.project_id.as_deref()).await?;
    let history = runtime
        .list_artifact_revisions(id, query.cursor, page_size(query.limit)?)
        .await
        .map_err(operator_error)?;
    Ok(envelope(project, history.entries, history.next_after))
}

pub async fn handle_v1_buddy_operator_work_outcomes(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WorkListQuery>,
) -> Result<axum::Json<OperatorEnvelope<Vec<Stored<RunOutcome>>>>, ScratchError> {
    let (runtime, project) = operator_runtime(&app, query.project_id.as_deref()).await?;
    let history = runtime
        .list_work_outcomes(id, query.cursor, page_size(query.limit)?)
        .await
        .map_err(operator_error)?;
    Ok(envelope(project, history.entries, history.next_after))
}

pub async fn handle_v1_buddy_operator_actions(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Result<axum::Json<OperatorEnvelope<Vec<crate::buddy::desk_actions::DeskAction>>>, ScratchError>
{
    let (runtime, project) = operator_runtime(&app, query.project_id.as_deref()).await?;
    let actions = runtime.desk_actions(id).await.map_err(operator_error)?;
    Ok(envelope(project, actions, None))
}

pub async fn handle_v1_buddy_operator_actions_batch(
    State(app): State<AppState>,
    Query(query): Query<crate::buddy::desk_actions_batch::WorkActionsBatchQuery>,
) -> Result<
    axum::Json<OperatorEnvelope<Vec<crate::buddy::desk_actions_batch::WorkDeskActions>>>,
    ScratchError,
> {
    let (runtime, project) = operator_runtime(&app, query.project_id.as_deref()).await?;
    let ids = crate::buddy::desk_actions_batch::parse_work_ids(query.ids.as_deref())
        .map_err(|message| ScratchError::new(StatusCode::BAD_REQUEST, message.to_string()))?;
    let actions = runtime
        .desk_actions_batch(ids)
        .await
        .map_err(operator_error)?;
    Ok(envelope(project, actions, None))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachResourceRequest {
    pub project_id: String,
    pub work_revision: u64,
    pub artifact_id: String,
    pub artifact_revision: u32,
    pub spec_hash: String,
}

pub async fn handle_resource_attach(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<AttachResourceRequest>,
) -> Result<axum::Json<OperatorEnvelope<serde_json::Value>>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(action_error)?;
    let runtime = gated_runtime(&app, OperatorCapability::Actions)
        .await
        .map_err(action_error)?;
    let project = runtime.project();
    if project.project_id != request.project_id {
        return Err(action_error("project changed"));
    }
    let origin = crate::buddy::desk_actions::WorkActionOrigin {
        schema_version: 1,
        project: project.clone(),
        work_id: id,
        work_revision: request.work_revision,
        artifact_id: request.artifact_id,
        artifact_revision: request.artifact_revision,
        spec_hash: request.spec_hash,
    };
    runtime
        .with_store(move |store| crate::buddy::desk_actions::attach_resource_action(store, origin))
        .await
        .map_err(action_error)?;
    Ok(envelope(
        project,
        serde_json::json!({"attached":true,"permission_granted":false}),
        None,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkReadRequest {
    pub project_id: String,
    pub artifact_id: String,
    pub artifact_revision: u32,
}

pub async fn handle_v1_buddy_operator_read_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Result<axum::Json<OperatorEnvelope<Option<ReadState>>>, ScratchError> {
    let (runtime, project) = operator_runtime(&app, query.project_id.as_deref()).await?;
    let state = runtime.get_read_state(id).await.map_err(operator_error)?;
    Ok(envelope(project, state, None))
}

pub async fn handle_v1_buddy_operator_read_mark(
    State(app): State<AppState>,
    Path(id): Path<String>,
    axum::Json(request): axum::Json<MarkReadRequest>,
) -> Result<axum::Json<OperatorEnvelope<ReadState>>, ScratchError> {
    let (runtime, project) = operator_runtime(&app, Some(&request.project_id)).await?;
    let state = runtime
        .mark_work_read(id, request.artifact_id, request.artifact_revision)
        .await
        .map_err(operator_error)?;
    Ok(envelope(project, state, None))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnoozeWorkRequest {
    pub project_id: String,
    pub canonical_root: String,
    pub artifact_id: String,
    pub artifact_revision: u32,
    pub minutes: Option<u32>,
}

pub async fn handle_v1_buddy_operator_snooze_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ProjectQuery>,
) -> Result<axum::Json<OperatorEnvelope<Option<WorkSnooze>>>, ScratchError> {
    let (runtime, project) = operator_runtime(&app, query.project_id.as_deref()).await?;
    let state = runtime.get_work_snooze(id).await.map_err(operator_error)?;
    Ok(envelope(project, state, None))
}

pub async fn handle_v1_buddy_operator_snooze_set(
    State(app): State<AppState>,
    Path(id): Path<String>,
    axum::Json(request): axum::Json<SnoozeWorkRequest>,
) -> Result<axum::Json<OperatorEnvelope<Option<WorkSnooze>>>, ScratchError> {
    let (runtime, project) = operator_runtime(&app, Some(&request.project_id)).await?;
    if project.canonical_root != request.canonical_root {
        return Err(ScratchError::new(
            StatusCode::CONFLICT,
            "work project changed".into(),
        ));
    }
    let state = runtime
        .snooze_work(
            id,
            request.artifact_id,
            request.artifact_revision,
            request.minutes,
        )
        .await
        .map_err(operator_error)?;
    Ok(envelope(project, state, None))
}

pub async fn handle_v1_buddy_operator_action_execute(
    State(app): State<AppState>,
    Path((id, action_id)): Path<(String, String)>,
    axum::Json(request): axum::Json<crate::buddy::desk_actions::DeskActionRequest>,
) -> Result<axum::Json<OperatorEnvelope<Stored<ActionExecution>>>, ScratchError> {
    let (runtime, project) = operator_runtime(&app, Some(&request.project_id)).await?;
    let execution = runtime
        .execute_desk_action(id, action_id, request)
        .await
        .map_err(operator_error)?;
    Ok(envelope(project, execution, None))
}

#[cfg(test)]
mod resource_http_tests {
    use super::*;

    #[tokio::test]
    async fn pending_publication_owner_change_is_refused_and_metadata_is_bounded() {
        use crate::buddy::outcome_runtime::{PendingWorkflowPublication, WorkflowTerminal};
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let runtime = crate::buddy::operator_runtime::open_project_runtime(&root).unwrap();
        let scope = PublicationScope {
            project: runtime.project(),
            worker: runtime.worker(),
        };
        validate_publication_owner(&runtime, Some(&runtime), &scope).unwrap();
        let mut wrong = PublicationScope {
            project: runtime.project(),
            worker: runtime.worker(),
        };
        wrong.worker.epoch += 1;
        assert!(validate_publication_owner(&runtime, Some(&runtime), &wrong).is_err());
        assert!(validate_publication_owner(&runtime, None, &scope).is_err());
        let start = runtime.start_workflow("commit_message", chrono::Utc::now().to_rfc3339());
        let run_id = start.run_id.clone();
        runtime
            .retain_pending_publication(PendingWorkflowPublication::Report {
                start,
                title: "Retained".into(),
                body: "private report bytes".into(),
                evidence: vec![],
                terminal: WorkflowTerminal::Completed,
            })
            .unwrap();
        let metadata = serde_json::to_string(&runtime.pending_publications().unwrap()).unwrap();
        assert!(!metadata.contains("private report bytes"));
        runtime.retry_pending_publication(&run_id).await.unwrap();
        assert!(runtime.pending_publications().unwrap().is_empty());
        runtime.shutdown().await;
        let reopened = crate::buddy::operator_runtime::open_project_runtime(&root).unwrap();
        assert!(validate_publication_owner(&runtime, Some(&reopened), &scope).is_err());
        assert!(validate_publication_owner(&reopened, Some(&reopened), &scope).is_err());
        reopened.shutdown().await;
    }

    #[tokio::test]
    async fn unavailable_status_reports_the_owner_and_manual_reopen_recovers() {
        let root = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(root.path()).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.join("cache"),
            root.join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let app = AppState::from_gcx(gcx).await;
        let holder = crate::buddy::operator_runtime::open_project_runtime(&root).unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let mut service = crate::buddy::actor::BuddyService::new(
            root.clone(),
            crate::buddy::state::default_buddy_state(),
            crate::buddy::settings::BuddySettings::default(),
            vec![],
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        service.enable_operator_from_settings();
        *app.buddy.buddy.lock().await = Some(service);

        let unavailable = handle_operator_status(State(app.clone())).await.unwrap().0;
        assert_eq!(unavailable.mode, "unavailable");
        let reason = unavailable.unavailable_reason.unwrap();
        assert_eq!(reason.kind, "already_owned");
        // Windows byte-range locks prevent a competing handle from reading
        // owner metadata. Exclusion still holds; the None-owner status must
        // not invent a PID or turn contention into an I/O error.
        #[cfg(windows)]
        assert_eq!(
            reason.detail,
            "operator store is already owned by another writer"
        );
        #[cfg(not(windows))]
        assert!(reason
            .detail
            .contains(&format!("pid {} ", std::process::id())));
        assert!(reason.retry_in_seconds.is_some());

        holder.shutdown().await;
        let recovered = handle_operator_reopen(State(app.clone())).await.unwrap().0;
        assert_eq!(recovered.mode, "enabled");
        assert!(recovered.unavailable_reason.is_none());
        app.buddy
            .buddy
            .lock()
            .await
            .as_ref()
            .unwrap()
            .outcome_runtime
            .clone()
            .unwrap()
            .shutdown()
            .await;
    }

    #[tokio::test]
    async fn status_never_treats_missing_service_or_missing_store_as_disabled() {
        let root = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.path().join("cache"),
            root.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        assert_eq!(
            handle_operator_status(State(app.clone()))
                .await
                .unwrap()
                .0
                .mode,
            "unavailable"
        );
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let mut settings = crate::buddy::settings::BuddySettings::default();
        settings.operator_enabled = false;
        let service = crate::buddy::actor::BuddyService::new(
            root.path().to_path_buf(),
            crate::buddy::state::default_buddy_state(),
            settings,
            vec![],
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        *app.buddy.buddy.lock().await = Some(service);
        let status = handle_operator_status(State(app.clone())).await.unwrap().0;
        assert_eq!(status.mode, "disabled");
        assert!(!status.features.storage);
        assert!(!status.features.actions);
        app.buddy
            .buddy
            .lock()
            .await
            .as_mut()
            .unwrap()
            .operator_features
            .storage = true;
        assert_eq!(
            handle_operator_status(State(app)).await.unwrap().0.mode,
            "unavailable"
        );
    }

    #[tokio::test]
    async fn default_settings_enable_the_operator_and_store_failure_reports_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let canonical = dunce::canonicalize(root.path()).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            canonical.join("cache"),
            canonical.join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![canonical.clone()];
        let app = AppState::from_gcx(gcx).await;
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let mut service = crate::buddy::actor::BuddyService::new(
            canonical.clone(),
            crate::buddy::state::default_buddy_state(),
            crate::buddy::settings::BuddySettings::default(),
            vec![],
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        service.enable_operator_from_settings();
        assert!(service.operator_features.storage);
        assert!(service.operator_features.actions);
        assert!(service.operator_features.supervision);
        assert!(service.operator_features.improvement);
        assert!(service.operator_features.editorial);
        let runtime = service.outcome_runtime.clone().unwrap();
        let project = runtime.project();
        *app.buddy.buddy.lock().await = Some(service);
        let status = handle_operator_status(State(app.clone())).await.unwrap().0;
        assert_eq!(status.mode, "enabled");
        assert!(status.features.storage);
        assert_eq!(status.project.unwrap(), project);
        // A second holder cannot take the same store; the degraded path reports
        // unavailable, never enabled, and never rewrites the existing database.
        let contended = tempfile::tempdir().unwrap();
        let contended = dunce::canonicalize(contended.path()).unwrap();
        let holder = crate::buddy::operator_runtime::open_project_runtime(&contended).unwrap();
        let database = contended.join(".refact/buddy/operator.sqlite");
        let before = std::fs::read(&database).unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let mut blocked = crate::buddy::actor::BuddyService::new(
            contended.clone(),
            crate::buddy::state::default_buddy_state(),
            crate::buddy::settings::BuddySettings::default(),
            vec![],
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        blocked.enable_operator_from_settings();
        assert!(blocked.operator_features.storage);
        assert!(blocked.outcome_runtime.is_none());
        *app.buddy.buddy.lock().await = Some(blocked);
        assert_eq!(
            handle_operator_status(State(app)).await.unwrap().0.mode,
            "unavailable"
        );
        assert_eq!(std::fs::read(&database).unwrap(), before);
        holder.shutdown().await;
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn operator_opt_out_reports_disabled_and_writes_no_operator_store() {
        let root = tempfile::tempdir().unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let mut settings = crate::buddy::settings::BuddySettings::default();
        settings.operator_enabled = false;
        let mut service = crate::buddy::actor::BuddyService::new(
            canonical.clone(),
            crate::buddy::state::default_buddy_state(),
            settings,
            vec![],
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        service.enable_operator_from_settings();
        assert!(!service.operator_features.storage);
        assert!(!service.operator_features.actions);
        assert!(!service.operator_features.supervision);
        assert!(!service.operator_features.improvement);
        assert!(!service.operator_features.editorial);
        assert!(service.outcome_runtime.is_none());
        assert!(!canonical.join(".refact/buddy/operator.sqlite").exists());
        assert!(!canonical.join(".refact/buddy/operator.lock").exists());
        assert!(service.add_opportunity_with_cooldown(legacy_opportunity(), 60));
        assert_eq!(service.snapshot().opportunities.len(), 1);
    }

    fn legacy_opportunity() -> crate::buddy::types::BuddyOpportunity {
        let now = chrono::Utc::now();
        crate::buddy::types::BuddyOpportunity {
            id: "legacy".into(),
            kind: crate::buddy::types::BuddyOpportunityKind::JobFinding,
            summary: "Legacy path still works when the operator is off".into(),
            priority: crate::buddy::types::BuddyPriority::Normal,
            confidence: 0.5,
            fact_keys: vec![],
            cooldown_key: "legacy:opt-out".into(),
            cooldown_secs: 60,
            status: crate::buddy::types::OpportunityStatus::New,
            proposed_actions: vec![],
            humor: None,
            humor_allowed: false,
            related: Default::default(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
            resolved_at: None,
        }
    }

    #[tokio::test]
    async fn connected_markdown_http_effect_retry_recovery_and_restart_are_durable() {
        use crate::buddy::{actor::BuddyService, action_execution::ActionRejection};
        use refact_buddy_core::operator_types::*;
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let project = ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: root.to_string_lossy().into_owned(),
        };
        let worker = WorkerIdentity {
            worker_id: "worker".into(),
            engine_instance_id: "fixture".into(),
            epoch: 1,
        };
        let runtime = OutcomeRuntime::open(&root, project.clone(), worker.clone()).unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.join("cache"),
            root.join("config"),
        )
        .await;
        std::sync::Arc::get_mut(&mut gcx)
            .unwrap()
            .cmdline
            .daemon_auth_token = Some("fixture-secret".into());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let app = AppState::from_gcx(gcx).await;
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let mut service = BuddyService::new(
            root.clone(),
            crate::buddy::state::default_buddy_state(),
            Default::default(),
            vec![],
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        service.operator_features.storage = true;
        service.operator_features.actions = true;
        service.outcome_runtime = Some(runtime.clone());
        *app.buddy.buddy.lock().await = Some(service);
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer fixture-secret".parse().unwrap());
        let spec = ActionSpec {
            schema_version: 1,
            action_id: "fixture-command".into(),
            kind: "markdown_project_apply".into(),
            adapter: "extensions".into(),
            project: project.clone(),
            resource: ResourceRef {
                resource_kind: "native_markdown".into(),
                resource_id: "commands/fixture".into(),
                version: "absent".into(),
            },
            expected_version: "absent".into(),
            parameter_ref: Some("fixture-draft".into()),
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
        let draft = serde_json::json!({"project":project,"kind":"command","name":"fixture","description":"Fixture","body":"Never execute: fixture text only"});
        let _ = handle_resource_draft(
            State(app.clone()),
            headers.clone(),
            axum::Json(ResourceDraftRequest {
                spec: spec.clone(),
                draft,
            }),
        )
        .await
        .unwrap();
        assert!(!root.join(".refact/commands/fixture.md").exists());
        let prepared = handle_resource_prepare(
            State(app.clone()),
            headers.clone(),
            axum::Json(PrepareResourceRequest {
                spec: spec.clone(),
            }),
        )
        .await
        .unwrap()
        .0
        .data;
        let rights = handle_resource_rights(
            State(app.clone()),
            headers.clone(),
            axum::Json(PrepareResourceRequest {
                spec,
            }),
        )
        .await
        .unwrap()
        .0
        .data;
        let grant = handle_resource_grant(
            State(app.clone()),
            headers.clone(),
            axum::Json(GrantResourceRequest {
                project_id: project.project_id.clone(),
                spec_hash: prepared.spec_hash.clone(),
                expires_in_seconds: 3600,
                native_rights: rights,
                recovery_execution_id: None,
            }),
        )
        .await
        .unwrap()
        .0
        .data;
        let grant_id = grant.decision.unwrap().native.source_id;
        let execute = || ExecuteResourceRequest {
            project_id: project.project_id.clone(),
            spec_hash: prepared.spec_hash.clone(),
            grant_id: grant_id.clone(),
            execution_id: "apply".into(),
            idempotency_key: "apply-key".into(),
        };
        let result =
            handle_resource_execute(State(app.clone()), headers.clone(), axum::Json(execute()))
                .await
                .unwrap()
                .0
                .data;
        assert!(
            matches!(result, ActionResult::Finished(ref record) if record.execution.status == ActionExecutionState::Partial && record.recovery.is_some())
        );
        let bytes = std::fs::read(root.join(".refact/commands/fixture.md")).unwrap();
        assert!(String::from_utf8(bytes.clone())
            .unwrap()
            .contains("fixture text only"));
        assert!(matches!(
            handle_resource_execute(State(app.clone()), headers.clone(), axum::Json(execute()))
                .await
                .unwrap()
                .0
                .data,
            ActionResult::Duplicate(_)
        ));
        assert_eq!(
            std::fs::read(root.join(".refact/commands/fixture.md")).unwrap(),
            bytes
        );
        let mut conflict = execute();
        conflict.spec_hash = "not-prepared".into();
        assert!(
            handle_resource_execute(State(app.clone()), headers.clone(), axum::Json(conflict))
                .await
                .is_err()
        );
        let recovery_grant = handle_resource_grant(
            State(app.clone()),
            headers.clone(),
            axum::Json(GrantResourceRequest {
                project_id: project.project_id.clone(),
                spec_hash: prepared.spec_hash.clone(),
                expires_in_seconds: 3600,
                native_rights: None,
                recovery_execution_id: Some("apply".into()),
            }),
        )
        .await
        .unwrap()
        .0
        .data;
        let recovery_grant_id = recovery_grant.decision.unwrap().native.source_id;
        let recover = || ExecuteResourceRequest {
            project_id: project.project_id.clone(),
            spec_hash: prepared.spec_hash.clone(),
            grant_id: recovery_grant_id.clone(),
            execution_id: "recover".into(),
            idempotency_key: "recover-key".into(),
        };
        assert!(handle_resource_execute(
            State(app.clone()),
            headers.clone(),
            axum::Json(recover())
        )
        .await
        .is_err());
        let restored = handle_resource_recover(
            State(app.clone()),
            Path("apply".into()),
            headers.clone(),
            axum::Json(recover()),
        )
        .await
        .unwrap()
        .0
        .data;
        assert_eq!(restored.execution.status, ActionExecutionState::Partial);
        assert!(!root.join(".refact/commands/fixture.md").exists());
        runtime.shutdown().await;
        let reopened = OutcomeRuntime::open(&root, project.clone(), worker).unwrap();
        app.buddy
            .buddy
            .lock()
            .await
            .as_mut()
            .unwrap()
            .outcome_runtime = Some(reopened.clone());
        assert!(matches!(
            handle_resource_execute(State(app.clone()), headers.clone(), axum::Json(execute()))
                .await
                .unwrap()
                .0
                .data,
            ActionResult::Duplicate(_)
        ));
        let duplicate = handle_resource_recover(
            State(app.clone()),
            Path("apply".into()),
            headers,
            axum::Json(recover()),
        )
        .await
        .unwrap()
        .0
        .data;
        assert_eq!(duplicate, restored);
        assert!(!root.join(".refact/commands/fixture.md").exists());
        assert!(!matches!(
            result,
            ActionResult::Rejected(ActionRejection::RepositoryFailure)
        ));
        reopened.shutdown().await;
    }
}
