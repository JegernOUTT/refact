use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use hyper::StatusCode;
use serde::Deserialize;

use crate::app_state::AppState;
use crate::buddy::improvement::{
    self, CandidateRecord, ImprovementAttempt, AttemptState, ATTEMPT_NAMESPACE, CANDIDATE_NAMESPACE,
};
use crate::buddy::improvement_evaluation::{self, EvaluationSpec, EvaluationProof};
use crate::buddy::operator_runtime::{authenticated_operator_request, runtime, OperatorCapability};
use crate::buddy::operator_store::{OperatorStoreError, Stored};
use crate::custom_error::ScratchError;
use crate::buddy::improvement_runner::{self, EvaluationGrant, FixedRun};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectRequest {
    pub project_id: String,
    pub worktree_id: String,
    pub target_files: Vec<String>,
}

pub async fn handle_inspect(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<InspectRequest>,
) -> Result<axum::Json<improvement_runner::ExactIdentity>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(error)?;
    let runtime = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(error)?;
    if runtime.project().project_id != request.project_id {
        return Err(error("project changed"));
    }
    let service = crate::worktrees::service::WorktreeService::new_async(
        app.paths.cache_dir.clone(),
        runtime.project().canonical_root.into(),
    )
    .await
    .map_err(error)?;
    Ok(axum::Json(
        improvement_runner::inspect_source(&service, &request.worktree_id, &request.target_files)
            .await
            .map_err(error)?,
    ))
}

pub async fn handle_evaluate(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(grant): axum::Json<EvaluationGrant>,
) -> Result<axum::Json<EvaluationProof>, ScratchError> {
    let operator = authenticated_operator_request(&app, &headers).map_err(error)?;
    let runtime = runtime(&app, OperatorCapability::Improvement)
        .await
        .map_err(error)?;
    let service = crate::worktrees::service::WorktreeService::new_async(
        app.paths.cache_dir.clone(),
        runtime.project().canonical_root.into(),
    )
    .await
    .map_err(error)?;
    improvement_runner::evaluate_owned(&app, runtime, service, id, grant, operator)
        .await
        .map(axum::Json)
        .map_err(error)
}

pub async fn handle_evaluation_cancel(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<axum::Json<Stored<FixedRun>>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(error)?;
    let runtime = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(error)?;
    Ok(axum::Json(
        runtime
            .with_store(move |store| improvement_runner::cancel_run(store, &id))
            .await
            .map_err(error)?,
    ))
}

fn error(error: impl std::fmt::Display) -> ScratchError {
    ScratchError::new(StatusCode::CONFLICT, error.to_string())
}

fn artifact_error(_: crate::buddy::resources::ResourceError) -> ScratchError {
    // Resource errors can contain filesystem paths and parser input. Never serialize them.
    error("artifact_inspection_rejected")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactInspectRequest {
    pub spec_hash: String,
    pub grant_id: String,
}

pub async fn handle_artifact_inspect(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ArtifactInspectRequest>,
) -> Result<axum::Json<improvement::artifacts::ArtifactInspection>, ScratchError> {
    use crate::buddy::action_execution::Authorizer;
    use crate::buddy::resource_execution as resources;
    let operator = authenticated_operator_request(&app, &headers).map_err(error)?;
    let _gate = resources::execution_gate().lock().await;
    let runtime = runtime(&app, OperatorCapability::Improvement)
        .await
        .map_err(error)?;
    let prepared = resources::load_prepared(&runtime, request.spec_hash)
        .await
        .map_err(error)?;
    if prepared.spec.adapter != improvement::artifacts::ADAPTER
        || prepared.spec.kind != improvement::artifacts::ACTION
    {
        return Err(error("exact artifact action required"));
    }
    let grant = resources::load_grant(&runtime, request.grant_id)
        .await
        .map_err(error)?
        .ok_or_else(|| error("grant missing"))?;
    if grant.recovery_execution_id.is_some()
        || !grant.authorize(&prepared.spec, &operator, grant.decision.as_ref())
    {
        return Err(error("artifact inspection grant denied"));
    }
    resources::validate_prepared_draft(&runtime, &prepared)
        .await
        .map_err(error)?;
    let reference = prepared
        .spec
        .parameter_ref
        .clone()
        .ok_or_else(|| error("artifact draft missing"))?;
    let draft =
        resources::load_draft::<improvement::artifacts::ArtifactDraft>(&runtime, reference.clone())
            .await
            .map_err(error)?
            .ok_or_else(|| error("artifact draft missing"))?;
    let worktrees = crate::worktrees::service::WorktreeService::new_async(
        app.paths.cache_dir.clone(),
        runtime.project().canonical_root.into(),
    )
    .await
    .map_err(error)?;
    let adapter = improvement::artifacts::ArtifactAdapter::new(
        runtime,
        worktrees,
        std::sync::Arc::new(resources::BoundDraft::new(reference, draft)),
    );
    adapter
        .inspect(&prepared.spec)
        .await
        .map(axum::Json)
        .map_err(artifact_error)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationQuery {
    pub project_id: String,
}

pub async fn handle_evaluation_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<EvaluationQuery>,
    headers: HeaderMap,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(error)?;
    let runtime = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(error)?;
    if runtime.project().project_id != query.project_id {
        return Err(error("project changed"));
    }
    let status = runtime
        .with_store(move |store| improvement_runner::evaluation_status(store, &id))
        .await
        .map_err(error)?
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, "evaluation run unavailable".into())
        })?;
    Ok(axum::Json(
        serde_json::json!({"schema_version":1,"project":runtime.project(),"data":status}),
    ))
}

pub async fn handle_evaluations(
    State(app): State<AppState>,
    Query(query): Query<ListQuery>,
    headers: HeaderMap,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(error)?;
    let runtime = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(error)?;
    if runtime.project().project_id != query.project_id {
        return Err(error("project changed"));
    }
    if query.cursor.is_some_and(|cursor| cursor > i64::MAX as u64) {
        return Err(error("invalid evaluation cursor"));
    }
    let (data, next_cursor) = runtime
        .with_store(move |store| {
            let page = store.list_documents::<FixedRun>(
                improvement_runner::RUN_NAMESPACE,
                query.cursor,
                query.limit.unwrap_or(50),
            )?;
            let data = page
                .entries
                .into_iter()
                .map(|run| {
                    improvement_runner::evaluation_status(store, &run.value.grant.run_id)?.ok_or(
                        OperatorStoreError::InvalidInput("evaluation run unavailable"),
                    )
                })
                .collect::<Result<Vec<_>, OperatorStoreError>>()?;
            Ok((data, page.next_after))
        })
        .await
        .map_err(error)?;
    Ok(axum::Json(
        serde_json::json!({"schema_version":1,"project":runtime.project(),"data":data,"next_cursor":next_cursor}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub project_id: String,
    pub cursor: Option<u64>,
    pub limit: Option<usize>,
}

pub async fn handle_list(
    State(app): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let runtime = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(error)?;
    if runtime.project().project_id != query.project_id {
        return Err(error("project changed"));
    }
    let page = runtime
        .with_store(move |store| {
            store.list_documents::<CandidateRecord>(
                CANDIDATE_NAMESPACE,
                query.cursor,
                query.limit.unwrap_or(50),
            )
        })
        .await
        .map_err(error)?;
    Ok(axum::Json(
        serde_json::json!({"schema_version":1,"project":runtime.project(),"data":page.entries,"next_cursor":page.next_after}),
    ))
}

pub async fn handle_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<axum::Json<Stored<CandidateRecord>>, ScratchError> {
    let runtime = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(error)?;
    if runtime.project().project_id != query.project_id {
        return Err(error("project changed"));
    }
    Ok(axum::Json(
        runtime
            .with_store(move |store| improvement::candidate(store, &id))
            .await
            .map_err(error)?,
    ))
}

pub async fn handle_spec_create(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(spec): axum::Json<EvaluationSpec>,
) -> Result<axum::Json<Stored<EvaluationSpec>>, ScratchError> {
    let operator = authenticated_operator_request(&app, &headers).map_err(error)?;
    let runtime = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(error)?;
    Ok(axum::Json(
        runtime
            .with_store(move |store| improvement_evaluation::register_spec(store, &spec, &operator))
            .await
            .map_err(error)?,
    ))
}

pub async fn handle_create(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(candidate): axum::Json<CandidateRecord>,
) -> Result<axum::Json<Stored<CandidateRecord>>, ScratchError> {
    let operator = authenticated_operator_request(&app, &headers).map_err(error)?;
    let runtime = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(error)?;
    Ok(axum::Json(
        runtime
            .with_store(move |store| improvement::register_candidate(store, &candidate, &operator))
            .await
            .map_err(error)?,
    ))
}

pub async fn handle_proof(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(proof): axum::Json<EvaluationProof>,
) -> Result<axum::Json<Stored<EvaluationProof>>, ScratchError> {
    let operator = authenticated_operator_request(&app, &headers).map_err(error)?;
    let runtime = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(error)?;
    Ok(axum::Json(
        runtime
            .with_store(move |store| improvement::record_proof(store, &id, &proof, &operator))
            .await
            .map_err(error)?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnRequest {
    pub candidate_digest: String,
    pub parent_chat_id: String,
    pub model: String,
}

pub async fn handle_spawn(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<SpawnRequest>,
) -> Result<axum::Json<Stored<ImprovementAttempt>>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(error)?;
    let runtime = runtime(&app, OperatorCapability::Improvement)
        .await
        .map_err(error)?;
    let candidate = runtime
        .with_store(move |store| improvement::candidate(store, &id))
        .await
        .map_err(error)?
        .value;
    if improvement_evaluation::digest(&candidate).map_err(error)? != request.candidate_digest {
        return Err(error("candidate changed"));
    }
    let active_root = crate::files_correction::get_active_project_path(app.gcx.clone())
        .await
        .ok_or_else(|| error("active project unavailable"))?;
    if active_root.to_string_lossy() != candidate.project.canonical_root {
        return Err(error("active candidate project changed"));
    }
    if request.model.trim().is_empty() {
        return Err(error("explicit model required"));
    }
    crate::tools::task_tool_helpers::preflight_agent_model(app.gcx.clone(), &request.model)
        .await
        .map_err(error)?;
    let parent = app
        .chat
        .sessions
        .read()
        .await
        .get(&request.parent_chat_id)
        .cloned()
        .ok_or_else(|| error("native parent session unavailable"))?;
    {
        let parent = parent.lock().await;
        if parent.closed {
            return Err(error("native parent session closed"));
        }
    }
    let native = improvement::spawn_request(&candidate, request.parent_chat_id, request.model)
        .map_err(error)?;
    let attempt = ImprovementAttempt {
        schema_version: 1, project: candidate.project.clone(),
        id: format!("spawn:{}", request.candidate_digest),
        candidate_id: candidate.candidate.candidate_id.clone(), candidate_digest: request.candidate_digest,
        state: AttemptState::SpawnClaimed, native_id: None,
        detail: "Native worker spawn claimed; interrupted claims require native reconciliation and are never replayed.".into(),
        recovery: improvement::recovery_description(&candidate),
    };
    let claim = runtime
        .with_store(move |store| {
            if store
                .get_document::<ImprovementAttempt>(ATTEMPT_NAMESPACE, &attempt.id)?
                .is_some()
            {
                return Err(OperatorStoreError::Conflict(
                    "spawn already claimed; inspect attempt and native agents, do not replay",
                ));
            }
            store.put_document(ATTEMPT_NAMESPACE, &attempt.id, &attempt, None)
        })
        .await
        .map_err(error)?;
    let mut attempt = claim.value;
    match crate::agents::spawn::spawn_background_agent(app, native).await {
        Ok(handle) => {
            attempt.state = AttemptState::WorkerStarted;
            attempt.native_id = Some(handle.agent_id);
            attempt.detail = "Native isolated candidate worker started with auto_merge=false. Worker completion is not evaluation proof.".into();
        }
        Err(detail) => {
            attempt.state = AttemptState::Failed;
            attempt.detail = detail;
        }
    }
    Ok(axum::Json(
        runtime
            .with_store(move |store| {
                store.put_document(
                    ATTEMPT_NAMESPACE,
                    &attempt.id,
                    &attempt,
                    Some(claim.revision),
                )
            })
            .await
            .map_err(error)?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationRequest {
    pub candidate_digest: String,
    pub proof_id: String,
}

pub async fn handle_activation_check(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ActivationRequest>,
) -> Result<axum::Json<Stored<ImprovementAttempt>>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(error)?;
    let runtime = runtime(&app, OperatorCapability::Improvement)
        .await
        .map_err(error)?;
    let attempt = runtime.with_store(move |store| {
        let candidate = improvement::candidate(store, &id)?.value;
        let gate = improvement::activation_gate(store, &id, &request.candidate_digest, &request.proof_id);
        let detail = match gate {
            Err(error) => error.to_string(),
            Ok(_) => format!("Evaluation binding is current. Exact privately staged artifact verified: {}. Activation requires a separate exact resource action grant; binary/host deployment is unsupported.", improvement::artifacts::staged_readiness(store, &candidate).unwrap_or(false)),
        };
        let attempt = ImprovementAttempt {
            schema_version: 1, project: candidate.project.clone(), id: uuid::Uuid::new_v4().to_string(),
            candidate_id: id, candidate_digest: request.candidate_digest,
            state: AttemptState::ActivationBlocked, native_id: None, detail,
            recovery: improvement::recovery_description(&candidate),
        };
        store.put_document(ATTEMPT_NAMESPACE, &attempt.id, &attempt, None)
    }).await.map_err(error)?;
    Ok(axum::Json(attempt))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivateRequest {
    pub candidate_digest: String,
    pub proof_id: String,
    pub execution_id: String,
    pub idempotency_key: String,
    pub decision: refact_buddy_core::operator_types::DecisionReference,
}

pub async fn handle_activate(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ActivateRequest>,
) -> Result<axum::Json<Stored<ImprovementAttempt>>, ScratchError> {
    let operator = authenticated_operator_request(&app, &headers).map_err(error)?;
    let runtime = runtime(&app, OperatorCapability::Improvement)
        .await
        .map_err(error)?;
    let worker = runtime.worker();
    let (claim, action) = runtime.with_store(move |store| {
        let candidate = improvement::activation_gate(store, &id, &request.candidate_digest, &request.proof_id)?;
        if !matches!(candidate.target, improvement::DeploymentTarget::Configuration | improvement::DeploymentTarget::SourceCandidate | improvement::DeploymentTarget::ReviewedCommit) {
            return Err(OperatorStoreError::Conflict("binary and host activation unsupported; use activation-check for recovery details"));
        }
        let spec = candidate.activation_action.clone().ok_or(OperatorStoreError::Conflict("candidate has no owning resource activation action"))?;
        improvement::activation::validate_action_binding(store, &candidate, &spec)?;
        let attempt = ImprovementAttempt { schema_version:1, project:candidate.project.clone(), id:format!("activate:{}", request.candidate_digest), candidate_id:id, candidate_digest:request.candidate_digest, state:AttemptState::ActivationClaimed, native_id:Some(request.execution_id.clone()), detail:"Activation claimed before native effect; interrupted claims require reconciliation, never replay.".into(), recovery:improvement::recovery_description(&candidate) };
        if store.get_document::<ImprovementAttempt>(ATTEMPT_NAMESPACE, &attempt.id)?.is_some() {
            return Err(OperatorStoreError::Conflict("activation already claimed; inspect native execution, do not replay"));
        }
        let action = crate::buddy::action_execution::ActionRequest {
            execution_id:request.execution_id, idempotency_key:request.idempotency_key,
            bound_parameter_ref:spec.parameter_ref.clone(), spec,
            requester:operator, decision:Some(request.decision), worker,
        };
        Ok((store.put_document(ATTEMPT_NAMESPACE, &attempt.id, &attempt, None)?, action))
    }).await.map_err(error)?;
    let result =
        crate::buddy::resource_execution::execute_registered(app, runtime.clone(), action).await;
    let mut attempt = claim.value;
    match result {
        Ok(crate::buddy::action_execution::ActionResult::Finished(record))
        | Ok(crate::buddy::action_execution::ActionResult::Duplicate(record)) => {
            attempt.native_id = Some(record.execution.execution_id);
            attempt.state = match record.execution.status {
                refact_buddy_core::operator_types::ActionExecutionState::Completed => {
                    AttemptState::Applied
                }
                refact_buddy_core::operator_types::ActionExecutionState::Failed => {
                    AttemptState::Failed
                }
                _ => AttemptState::OutcomeUnknown,
            };
            attempt.detail = format!("Owning resource execution status: {:?}. This does not imply a new installed or running binary.", record.execution.status);
        }
        Ok(crate::buddy::action_execution::ActionResult::Rejected(rejection)) => {
            attempt.state = AttemptState::OutcomeUnknown;
            attempt.detail = format!("Owning action rejected: {rejection:?}. Inspect the native execution before retrying; no automatic recovery.");
        }
        Err(detail) => {
            attempt.state = AttemptState::OutcomeUnknown;
            attempt.detail = detail;
        }
    }
    Ok(axum::Json(
        runtime
            .with_store(move |store| {
                store.put_document(
                    ATTEMPT_NAMESPACE,
                    &attempt.id,
                    &attempt,
                    Some(claim.revision),
                )
            })
            .await
            .map_err(error)?,
    ))
}

pub async fn handle_attempts(
    State(app): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let runtime = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(error)?;
    if runtime.project().project_id != query.project_id {
        return Err(error("project changed"));
    }
    let page = runtime
        .with_store(move |store| {
            store.list_documents::<ImprovementAttempt>(
                ATTEMPT_NAMESPACE,
                query.cursor,
                query.limit.unwrap_or(50),
            )
        })
        .await
        .map_err(error)?;
    Ok(axum::Json(
        serde_json::json!({"schema_version":1,"data":page.entries,"next_cursor":page.next_after}),
    ))
}

#[cfg(test)]
mod evaluation_tests {
    use super::*;
    use crate::buddy::outcome_runtime::OutcomeRuntime;
    use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};
    use tower::ServiceExt;

    #[test]
    fn artifact_resource_errors_never_expose_payloads() {
        use crate::buddy::resources::ResourceError;
        for resource in [
            ResourceError::ValidationFailed("secret parser input".into()),
            ResourceError::Io("/private/token".into()),
            ResourceError::ProjectScopeMismatch,
        ] {
            let mapped = artifact_error(resource);
            assert_eq!(mapped.status_code, StatusCode::CONFLICT);
            assert_eq!(mapped.message, "artifact_inspection_rejected");
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn exact_pending_cleanup_uses_authenticated_prepare_grant_execution_and_retry() {
        use super::super::buddy_operator as api;
        use crate::buddy::improvement::artifacts::*;
        use refact_buddy_core::operator_types::*;
        use sha2::{Digest, Sha256};
        let (root, app, runtime) = fixture().await;
        {
            let mut owner = app.buddy.buddy.lock().await;
            let flags = &mut owner.as_mut().unwrap().operator_features;
            flags.actions = true;
            flags.improvement = true;
        }
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer fixture-secret".parse().unwrap());
        let artifact = "a".repeat(64);
        let dir = root.path().join(".refact/candidate-artifacts");
        std::fs::create_dir_all(&dir).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let pending = dir.join(format!("pending-{artifact}"));
        std::fs::write(&pending, b"partial scratch image").unwrap();
        let draft = ArtifactCleanupDraft {
            project: runtime.project(),
            files: std::collections::BTreeMap::from([(
                format!("pending-{artifact}"),
                hex::encode(Sha256::digest(b"partial scratch image")),
            )]),
        };
        let version = improvement_evaluation::digest(&draft).unwrap();
        let spec = ActionSpec {
            schema_version: 1,
            action_id: "cleanup".into(),
            kind: CLEANUP_ACTION.into(),
            adapter: CLEANUP_ADAPTER.into(),
            project: runtime.project(),
            resource: ResourceRef {
                resource_kind: CLEANUP_ADAPTER.into(),
                resource_id: "orphan_artifacts".into(),
                version: version.clone(),
            },
            expected_version: version,
            parameter_ref: Some("cleanup-draft".into()),
            availability: ActionAvailability::Available,
            confirmation: ConfirmationClass::UserConfirmationRequired,
            authority_class: AuthorityClass::Mutating,
            capabilities: ActionCapabilities {
                can_preview: true,
                can_execute: true,
                can_verify: true,
                can_recover: false,
                can_cancel: false,
            },
        };
        let proposal = || api::ResourceDraftRequest {
            spec: spec.clone(),
            draft: serde_json::to_value(&draft).unwrap(),
        };
        assert!(api::handle_resource_draft(
            State(app.clone()),
            HeaderMap::new(),
            axum::Json(proposal())
        )
        .await
        .is_err());
        let _ =
            api::handle_resource_draft(State(app.clone()), headers.clone(), axum::Json(proposal()))
                .await
                .unwrap();
        let prepared = api::handle_resource_prepare(
            State(app.clone()),
            headers.clone(),
            axum::Json(api::PrepareResourceRequest {
                spec: spec.clone(),
            }),
        )
        .await
        .unwrap();
        let value = serde_json::to_value(prepared.0).unwrap();
        let spec_hash = value["data"]["spec_hash"].as_str().unwrap().to_string();
        assert!(pending.exists());
        let grant_request = |hash: String| api::GrantResourceRequest {
            project_id: runtime.project().project_id,
            spec_hash: hash,
            expires_in_seconds: 60,
            recovery_execution_id: None,
            native_rights: None,
        };
        assert!(api::handle_resource_grant(
            State(app.clone()),
            headers.clone(),
            axum::Json(grant_request("wrong".into()))
        )
        .await
        .is_err());
        let grant = api::handle_resource_grant(
            State(app.clone()),
            headers.clone(),
            axum::Json(grant_request(spec_hash.clone())),
        )
        .await
        .unwrap();
        let value = serde_json::to_value(grant.0).unwrap();
        let grant_id = value["data"]["decision"]["native"]["source_id"]
            .as_str()
            .unwrap()
            .to_string();
        let execute = || api::ExecuteResourceRequest {
            project_id: runtime.project().project_id,
            spec_hash: spec_hash.clone(),
            grant_id: grant_id.clone(),
            execution_id: "cleanup-execution".into(),
            idempotency_key: "cleanup-key".into(),
        };
        let mut wrong = HeaderMap::new();
        wrong.insert("authorization", "Bearer wrong".parse().unwrap());
        assert!(
            api::handle_resource_execute(State(app.clone()), wrong, axum::Json(execute()))
                .await
                .is_err()
        );
        assert!(pending.exists());
        // A granted filename is not authority over subsequently changed bytes.
        std::fs::write(&pending, b"changed after grant").unwrap();
        let mut changed_request = execute();
        changed_request.execution_id = "changed-pending".into();
        changed_request.idempotency_key = "changed-pending-key".into();
        let changed_execute = || api::ExecuteResourceRequest {
            execution_id: "changed-pending".into(),
            idempotency_key: "changed-pending-key".into(),
            ..execute()
        };
        let rejected = api::handle_resource_execute(
            State(app.clone()),
            headers.clone(),
            axum::Json(changed_request),
        )
        .await
        .unwrap();
        let rejected = serde_json::to_value(rejected.0).unwrap();
        assert_eq!(
            rejected["data"]["Finished"]["execution"]["status"],
            "outcome_unknown"
        );
        assert!(pending.exists());
        assert_eq!(std::fs::read(&pending).unwrap(), b"changed after grant");
        std::fs::write(&pending, b"partial scratch image").unwrap();
        // Resource apply errors retain uncertainty even when this fixture saw no unlink.
        // Restoring valid bytes must not turn the same execution into a cleanup retry.
        let duplicate = api::handle_resource_execute(
            State(app.clone()),
            headers.clone(),
            axum::Json(changed_execute()),
        )
        .await
        .unwrap();
        let duplicate = serde_json::to_value(duplicate.0).unwrap();
        assert_eq!(duplicate["data"]["Duplicate"], rejected["data"]["Finished"]);
        assert_eq!(std::fs::read(&pending).unwrap(), b"partial scratch image");
        let result = api::handle_resource_execute(
            State(app.clone()),
            headers.clone(),
            axum::Json(execute()),
        )
        .await
        .unwrap();
        let result = serde_json::to_value(result.0).unwrap();
        assert!(result.to_string().contains("completed"), "{result}");
        assert!(!pending.exists());
        let retry =
            api::handle_resource_execute(State(app.clone()), headers, axum::Json(execute()))
                .await
                .unwrap();
        let retry = serde_json::to_value(retry.0).unwrap();
        assert!(retry["data"]["Duplicate"].is_object(), "{retry}");
        assert!(!dir.join(&artifact).exists());
        runtime.shutdown().await;
    }

    #[cfg(target_os = "linux")]
    async fn artifact_post(
        app: &AppState,
        uri: &str,
        token: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let router = super::super::make_v1_router(app.clone()).with_state(app.clone());
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn native_artifact_http_stage_attestation_partial_and_reopen_receipt() {
        use crate::buddy::improvement::artifacts::*;
        use crate::buddy::improvement_evaluation::{digest, Criterion, CheckObservation, CheckResult};
        use refact_buddy_core::operator_types::*;
        use sha2::{Digest, Sha256};
        use std::collections::BTreeMap;
        use std::os::unix::fs::PermissionsExt;
        let (root, app, runtime) = fixture().await;
        {
            let mut owner = app.buddy.buddy.lock().await;
            owner.as_mut().unwrap().operator_features.actions = true;
            owner.as_mut().unwrap().operator_features.improvement = true;
        }
        // Real scratch git repository and native registered isolated checkout.
        // This fixture does not invoke a compiler or claim a built/runnable binary.
        {
            let repo = git2::Repository::init(root.path()).unwrap();
            std::fs::write(
                root.path().join(".gitignore"),
                "target/\ncache/\nconfig/\n.refact/\n",
            )
            .unwrap();
            std::fs::write(root.path().join("source.rs"), "// frozen source fixture\n").unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(std::path::Path::new(".gitignore")).unwrap();
            index.add_path(std::path::Path::new("source.rs")).unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let signature = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                "frozen scratch fixture",
                &tree,
                &[],
            )
            .unwrap();
        }
        let worktrees = crate::worktrees::service::WorktreeService::new_async(
            app.paths.cache_dir.clone(),
            root.path().to_path_buf(),
        )
        .await
        .unwrap();
        let checkout = worktrees
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/artifact-fixture".into()),
                ..Default::default()
            })
            .await
            .unwrap()
            .worktree
            .meta;
        std::fs::write(
            checkout.root.join("source.rs"),
            "// explicitly evaluated candidate fixture\n",
        )
        .unwrap();
        let exact =
            improvement_runner::inspect_source(&worktrees, &checkout.id, &["source.rs".into()])
                .await
                .unwrap();
        // Structurally valid ELF header fixture only; never execute these bytes.
        let mut bytes = vec![0u8; 120];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        for (offset, value) in [
            (16, 2),
            (18, 62),
            (20, 1),
            (32, 64),
            (52, 64),
            (54, 56),
            (56, 1),
            (64, 1),
            (96, 120),
        ] {
            bytes[offset] = value;
        }
        let artifact_digest = hex::encode(Sha256::digest(&bytes));
        let output = checkout.root.join("target/fixture-engine");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(&output, &bytes).unwrap();
        assert_eq!(
            exact,
            improvement_runner::inspect_source(&worktrees, &checkout.id, &["source.rs".into()])
                .await
                .unwrap()
        );
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer fixture-secret".parse().unwrap());
        let authority = authenticated_operator_request(&app, &headers).unwrap();
        let protocol = "explicit operator attestation of frozen scratch source identity; NOT a build/test result";
        let evaluation = EvaluationSpec {
            schema_version: 1,
            project: runtime.project(),
            id: "artifact-attestation-spec".into(),
            criteria: vec![Criterion {
                id: "identity".into(),
                command_or_protocol: protocol.into(),
                expected_result: "source identity matches fixture".into(),
                external_review: false,
            }],
            corpus_digest: digest(&exact).unwrap(),
            privacy_scope: "scratch fixture only".into(),
            authority: authority.clone(),
            max_age_seconds: 3600,
            execution: None,
        };
        let candidate: CandidateRecord = serde_json::from_value(serde_json::json!({
            "schema_version":1,"project":runtime.project(),
            "candidate":{"schema_version":1,"candidate_id":"artifact-candidate","project":runtime.project(),"target":"engine_build","worktree_ref":checkout.id,"build_identity":artifact_digest,"evaluation_spec_version":evaluation.id,"proof_refs":[]},
            "target":"engine_build","target_files":["source.rs"],"content_digest":exact.patch_digest,"evaluation_digest":digest(&evaluation).unwrap(),"source_identity":exact.source_digest,"engine_build":artifact_digest,"original_run_id":"fixture-origin","original_problem":[],"submitted_by":authority
        })).unwrap();
        let candidate_digest = digest(&candidate).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let evidence = EvidenceRef {
            evidence_id: "attested-source".into(),
            source: authority.clone(),
            locator: "fixture://operator-attested-source-not-build-proof".into(),
            observed_at: now.clone(),
            digest: Some(digest(&exact).unwrap()),
        };
        let mut start = runtime.start_workflow("scratch_source_attestation", now.clone());
        start.run_id = "artifact-attestation".into();
        start.input_fingerprint = candidate_digest.clone();
        runtime.begin_workflow(start.clone()).await.unwrap();
        runtime
            .publish_report(
                start,
                "Fixture identity attestation".into(),
                protocol.into(),
                vec![evidence.clone()],
                crate::buddy::outcome_runtime::WorkflowTerminal::Completed,
            )
            .await
            .unwrap();
        let proof = EvaluationProof {
            schema_version: 1,
            project: runtime.project(),
            id: "artifact-attested-proof".into(),
            candidate_digest: candidate_digest.clone(),
            evaluation_digest: digest(&evaluation).unwrap(),
            authority: authority.clone(),
            observed_at: now,
            checks: vec![CheckObservation {
                criterion_id: "identity".into(),
                command_or_protocol: protocol.into(),
                result: CheckResult::Passed,
                observed_result:
                    "frozen source snapshot explicitly attested; build provenance unverified".into(),
                evidence,
                run_id: "artifact-attestation".into(),
            }],
        };
        // Seed immutable evaluated candidate fixture, not a fabricated native compiler run.
        runtime
            .with_store(move |store| {
                improvement_evaluation::register_spec(store, &evaluation, &authority)?;
                store.put_document(CANDIDATE_NAMESPACE, "artifact-candidate", &candidate, None)?;
                improvement::record_proof(store, "artifact-candidate", &proof, &authority)?;
                Ok(())
            })
            .await
            .unwrap();
        let draft = ArtifactDraft {
            project: runtime.project(),
            candidate_id: "artifact-candidate".into(),
            candidate_digest,
            proof_id: "artifact-attested-proof".into(),
            worktree_id: checkout.id.clone(),
            relative_path: "target/fixture-engine".into(),
            format: ArtifactFormat::LinuxElf64,
            artifact_digest: artifact_digest.clone(),
            manifest_digest: digest(&BTreeMap::from([("engine", &artifact_digest)])).unwrap(),
            attested_version: "scratch-fixture-not-built".into(),
            attested_os: "linux".into(),
            attested_arch: "x86_64".into(),
            attested_host_requirement: "operator attested Linux; runtime unverified".into(),
            retained_known_good: vec![],
        };
        let version = digest(&draft).unwrap();
        let spec = ActionSpec {
            schema_version: 1,
            action_id: "stage-fixture".into(),
            kind: ACTION.into(),
            adapter: ADAPTER.into(),
            project: runtime.project(),
            resource: ResourceRef {
                resource_kind: ADAPTER.into(),
                resource_id: draft.candidate_id.clone(),
                version: version.clone(),
            },
            expected_version: version,
            parameter_ref: Some("artifact-fixture-draft".into()),
            availability: ActionAvailability::Available,
            confirmation: ConfirmationClass::UserConfirmationRequired,
            authority_class: AuthorityClass::Mutating,
            capabilities: ActionCapabilities {
                can_preview: true,
                can_execute: true,
                can_verify: true,
                can_recover: false,
                can_cancel: false,
            },
        };
        let proposal = serde_json::json!({"spec":spec,"draft":draft});
        assert_eq!(
            artifact_post(&app, "/buddy/actions/draft", "wrong", proposal.clone())
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            artifact_post(&app, "/buddy/actions/draft", "fixture-secret", proposal)
                .await
                .0,
            StatusCode::OK
        );
        let (status, prepared) = artifact_post(
            &app,
            "/buddy/actions/prepare",
            "fixture-secret",
            serde_json::json!({"spec":spec}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{prepared}");
        let spec_hash = prepared["data"]["spec_hash"].as_str().unwrap();
        let grant_request = serde_json::json!({"project_id":runtime.project().project_id,"spec_hash":spec_hash,"expires_in_seconds":3600});
        let mut wrong_hash = grant_request.clone();
        wrong_hash["spec_hash"] = serde_json::json!("wrong");
        assert_eq!(
            artifact_post(&app, "/buddy/actions/grant", "fixture-secret", wrong_hash)
                .await
                .0,
            StatusCode::CONFLICT
        );
        let (status, grant) = artifact_post(
            &app,
            "/buddy/actions/grant",
            "fixture-secret",
            grant_request.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{grant}");
        let grant_id = grant["data"]["decision"]["native"]["source_id"]
            .as_str()
            .unwrap();
        let inspect = serde_json::json!({"spec_hash":spec_hash,"grant_id":grant_id});
        let mut mismatched_inspect = inspect.clone();
        mismatched_inspect["spec_hash"] = serde_json::json!("wrong");
        assert_eq!(
            artifact_post(
                &app,
                "/buddy/improvement/artifacts/inspect",
                "fixture-secret",
                mismatched_inspect
            )
            .await
            .0,
            StatusCode::CONFLICT
        );

        assert_eq!(
            artifact_post(
                &app,
                "/buddy/improvement/artifacts/inspect",
                "wrong",
                inspect.clone()
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        let (status, inspection) = artifact_post(
            &app,
            "/buddy/improvement/artifacts/inspect",
            "fixture-secret",
            inspect.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{inspection}");
        assert_eq!(inspection["artifact_digest"], artifact_digest);
        assert!(inspection["compatibility_status"]
            .as_str()
            .unwrap()
            .contains("not build proof"));
        let dir = root.path().join(".refact/candidate-artifacts");
        assert!(!dir.exists()); // draft, prepare, grant and inspect have no staging effect.
        for source_change in [false, true] {
            let path = if source_change {
                checkout.root.join("source.rs")
            } else {
                output.clone()
            };
            let original = std::fs::read(&path).unwrap();
            std::fs::write(&path, b"changed after grant").unwrap();
            assert_eq!(
                artifact_post(
                    &app,
                    "/buddy/improvement/artifacts/inspect",
                    "fixture-secret",
                    inspect.clone()
                )
                .await
                .0,
                StatusCode::CONFLICT
            );
            let id = if source_change {
                "changed-source"
            } else {
                "changed-content"
            };
            let changed_execute = serde_json::json!({"project_id":runtime.project().project_id,"spec_hash":spec_hash,"grant_id":grant_id,"execution_id":id,"idempotency_key":id});
            let (status, rejected) = artifact_post(
                &app,
                "/buddy/actions/execute",
                "fixture-secret",
                changed_execute.clone(),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{rejected}");
            assert_eq!(
                rejected["data"]["Finished"]["execution"]["status"], "outcome_unknown",
                "{rejected}"
            );
            assert!(!dir.exists());
            assert_eq!(std::fs::read(&path).unwrap(), b"changed after grant");
            std::fs::write(&path, &original).unwrap();
            // No staging was observed, but ResourceError is not a typed no-effect
            // receipt. The durable unknown execution cannot replay with repaired input.
            let (status, duplicate) = artifact_post(
                &app,
                "/buddy/actions/execute",
                "fixture-secret",
                changed_execute,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{duplicate}");
            assert_eq!(duplicate["data"]["Duplicate"], rejected["data"]["Finished"]);
            assert!(!dir.exists());
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let pending = dir.join(format!("pending-{artifact_digest}"));
        std::fs::write(&pending, &bytes[..10]).unwrap();
        let execute = serde_json::json!({"project_id":runtime.project().project_id,"spec_hash":spec_hash,"grant_id":grant_id,"execution_id":"partial-stage","idempotency_key":"partial-stage-key"});
        assert_eq!(
            artifact_post(&app, "/buddy/actions/execute", "wrong", execute.clone())
                .await
                .0,
            StatusCode::CONFLICT
        );
        let (status, partial) = artifact_post(
            &app,
            "/buddy/actions/execute",
            "fixture-secret",
            execute.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{partial}");
        assert_eq!(
            partial["data"]["Finished"]["execution"]["status"],
            "outcome_unknown"
        );
        assert_eq!(std::fs::read(&pending).unwrap(), &bytes[..10]);
        assert!(!dir.join(&artifact_digest).exists());
        runtime.shutdown().await;
        let reopened =
            OutcomeRuntime::open(root.path(), runtime.project(), runtime.worker()).unwrap();
        app.buddy
            .buddy
            .lock()
            .await
            .as_mut()
            .unwrap()
            .outcome_runtime = Some(reopened.clone());
        let (_, duplicate) = artifact_post(
            &app,
            "/buddy/actions/execute",
            "fixture-secret",
            execute.clone(),
        )
        .await;
        assert_eq!(
            duplicate["data"]["Duplicate"]["execution"]["status"],
            "outcome_unknown"
        );
        assert_eq!(duplicate["data"]["Duplicate"], partial["data"]["Finished"]);
        assert_eq!(std::fs::read(&pending).unwrap(), &bytes[..10]);
        // Separate crash point: complete fsynced pending bytes, before atomic publish.
        // No retry repairs partial bytes; this write is explicit fixture setup only.
        std::fs::write(&pending, &bytes).unwrap();
        std::fs::File::open(&pending).unwrap().sync_all().unwrap();
        let (status, duplicate) = artifact_post(
            &app,
            "/buddy/actions/execute",
            "fixture-secret",
            execute.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{duplicate}");
        assert_eq!(duplicate["data"]["Duplicate"], partial["data"]["Finished"]);
        assert_eq!(std::fs::read(&pending).unwrap(), bytes);
        assert!(!dir.join(&artifact_digest).exists());
        let (_, fresh) = artifact_post(
            &app,
            "/buddy/actions/grant",
            "fixture-secret",
            grant_request,
        )
        .await;
        let mut execute = execute;
        execute["grant_id"] = fresh["data"]["decision"]["native"]["source_id"].clone();
        execute["execution_id"] = serde_json::json!("complete-stage");
        execute["idempotency_key"] = serde_json::json!("complete-stage-key");
        let (status, staged) = artifact_post(
            &app,
            "/buddy/actions/execute",
            "fixture-secret",
            execute.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{staged}");
        assert_eq!(
            staged["data"]["Finished"]["execution"]["status"], "completed",
            "{staged}"
        );
        assert_eq!(std::fs::read(dir.join(&artifact_digest)).unwrap(), bytes);
        assert!(!pending.exists());
        let uri = "/buddy/executions/complete-stage?project_id=evaluation-fixture";
        let (status, receipt) = request(&app, "GET", uri, Some("fixture-secret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(receipt["data"]["value"]["status"], "completed");
        assert_eq!(
            receipt["data"]["value"]["effects"][0]["resource"]["version"],
            artifact_digest
        );
        assert_eq!(
            receipt["data"]["value"]["verification_evidence"][0],
            format!("resource_receipt:complete-stage:{artifact_digest}")
        );
        assert!(receipt["data"]["value"]["recovery"].is_null());

        reopened.shutdown().await;
        let reloaded =
            OutcomeRuntime::open(root.path(), reopened.project(), reopened.worker()).unwrap();
        app.buddy
            .buddy
            .lock()
            .await
            .as_mut()
            .unwrap()
            .outcome_runtime = Some(reloaded.clone());
        assert_eq!(
            request(&app, "GET", uri, Some("fixture-secret")).await.1,
            receipt
        );
        let (_, duplicate) =
            artifact_post(&app, "/buddy/actions/execute", "fixture-secret", execute).await;
        assert_eq!(
            duplicate["data"]["Duplicate"]["execution"]["status"],
            "completed"
        );
        assert_eq!(duplicate["data"]["Duplicate"], staged["data"]["Finished"]);
        assert_eq!(std::fs::read(dir.join(&artifact_digest)).unwrap(), bytes);
        assert_eq!(
            std::fs::read(root.path().join("source.rs")).unwrap(),
            b"// frozen source fixture\n"
        );
        assert!(!root.path().join("installed").exists());
        reloaded
            .with_store(|store| {
                let candidate = improvement::candidate(store, "artifact-candidate")?.value;
                assert!(staged_readiness(store, &candidate)?);
                Ok(())
            })
            .await
            .unwrap();
        reloaded.shutdown().await;
    }

    async fn fixture() -> (tempfile::TempDir, AppState, OutcomeRuntime) {
        let root =
            tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.path().join("cache"),
            root.path().join("config"),
        )
        .await;
        std::sync::Arc::get_mut(&mut gcx)
            .unwrap()
            .cmdline
            .daemon_auth_token = Some("fixture-secret".into());
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        let runtime = OutcomeRuntime::open(
            root.path(),
            ProjectIdentity {
                project_id: "evaluation-fixture".into(),
                canonical_root: root.path().to_string_lossy().into_owned(),
            },
            WorkerIdentity {
                worker_id: "worker".into(),
                engine_instance_id: "engine".into(),
                epoch: 1,
            },
        )
        .unwrap();
        let mut service = crate::buddy::actor::BuddyService::new(
            root.path().to_path_buf(),
            crate::buddy::state::default_buddy_state(),
            Default::default(),
            vec![],
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tokio::sync::broadcast::channel(64).0,
            None,
        );
        service.operator_features.storage = true;
        service.outcome_runtime = Some(runtime.clone());
        *app.buddy.buddy.lock().await = Some(service);
        (root, app, runtime)
    }

    async fn request(
        app: &AppState,
        method: &str,
        uri: &str,
        token: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let router = super::super::make_v1_router(app.clone()).with_state(app.clone());
        let mut request = axum::http::Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = router
            .oneshot(request.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    fn claim(runtime: &OutcomeRuntime, id: &str) -> FixedRun {
        FixedRun {
            schema_version: 1,
            project: runtime.project(),
            grant: EvaluationGrant {
                candidate_digest: "a".repeat(64),
                evaluation_digest: "b".repeat(64),
                run_id: id.into(),
                expires_at: "2099-01-01T00:00:00Z".into(),
            },
            requester: improvement_runner::runner_origin(&runtime.worker()),
            evaluator: improvement_runner::runner_origin(&runtime.worker()),
            candidate_id: "candidate".into(),
            evaluation_spec_id: None,
            state: "claimed_outcome_unknown_until_finalized".into(),
            process_id: Some("native-process".into()),
            cancel_requested: false,
            exact: None,
        }
    }

    #[tokio::test]
    async fn evaluation_schema_one_legacy_run_remains_readable_and_strict() {
        let (_root, app, runtime) = fixture().await;
        let mut legacy = serde_json::to_value(claim(&runtime, "legacy")).unwrap();
        legacy.as_object_mut().unwrap().remove("evaluation_spec_id");
        assert_eq!(legacy["schema_version"], 1);
        runtime
            .with_store(move |store| {
                let mut missing_version = legacy.clone();
                missing_version
                    .as_object_mut()
                    .unwrap()
                    .remove("schema_version");
                assert!(matches!(
                    store.put_document(
                        improvement_runner::RUN_NAMESPACE,
                        "missing-version",
                        &missing_version,
                        None
                    ),
                    Err(OperatorStoreError::InvalidInput(_))
                ));
                let mut future = legacy.clone();
                for version in [0, 2] {
                    future["schema_version"] = serde_json::json!(version);
                    assert!(matches!(
                        store.put_document(
                            improvement_runner::RUN_NAMESPACE,
                            "future",
                            &future,
                            None
                        ),
                        Err(OperatorStoreError::InvalidInput(_))
                    ));
                }
                let mut foreign = legacy.clone();
                foreign["project"]["canonical_root"] = serde_json::json!("/another-project");
                assert!(matches!(
                    store.put_document(
                        improvement_runner::RUN_NAMESPACE,
                        "foreign",
                        &foreign,
                        None
                    ),
                    Err(OperatorStoreError::ProjectMismatch)
                ));
                store.reserve_terminal_documents(
                    "fixed_evaluation",
                    "legacy",
                    &[
                        (improvement_runner::RUN_NAMESPACE, "legacy".into(), 16),
                        ("improvement_comparisons", "legacy".into(), 1),
                        ("improvement_proofs", "fixed:legacy".into(), 1),
                    ],
                )?;
                store.put_reserved_document(
                    "fixed_evaluation",
                    "legacy",
                    improvement_runner::RUN_NAMESPACE,
                    "legacy",
                    &legacy,
                    None,
                )?;
                let read = store
                    .get_document::<FixedRun>(improvement_runner::RUN_NAMESPACE, "legacy")?
                    .unwrap();
                assert_eq!(read.value.schema_version, 1);
                assert_eq!(&read.value.project, store.identity());
                assert!(read.value.evaluation_spec_id.is_none());
                let cancelled = improvement_runner::cancel_run(store, "legacy")?;
                assert!(cancelled.value.cancel_requested);
                assert_eq!(cancelled.value.schema_version, 1);
                Ok(())
            })
            .await
            .unwrap();
        let (status, body) = request(
            &app,
            "GET",
            "/buddy/improvement/evaluations/legacy?project_id=evaluation-fixture",
            Some("fixture-secret"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["run"]["value"]["schema_version"], 1);
        assert!(body["data"]["evaluation_spec_id"].is_null());
        assert!(body["data"]["proof"].is_null());
        runtime.shutdown().await;
    }

    async fn assert_persisted_comparison(runtime: &OutcomeRuntime, grant: &EvaluationGrant) {
        let grant = grant.clone();
        runtime
            .with_store(move |store| {
                let comparison = store
                    .get_document::<improvement_runner::EvaluationComparison>(
                        "improvement_comparisons",
                        &grant.run_id,
                    )?
                    .unwrap()
                    .value;
                assert_eq!(comparison.schema_version, 1);
                assert_eq!(&comparison.project, store.identity());
                assert_eq!(comparison.run_id, grant.run_id);
                assert_eq!(comparison.candidate_digest, grant.candidate_digest);
                assert_eq!(comparison.evaluation_digest, grant.evaluation_digest);
                assert_eq!(comparison.cases.len(), 1);
                assert!(comparison.cases[0].candidate_pass);
                let run = store
                    .get_document::<FixedRun>(improvement_runner::RUN_NAMESPACE, &grant.run_id)?
                    .unwrap()
                    .value;
                assert_eq!(run.schema_version, 1);
                assert_eq!(&run.project, store.identity());
                let draft = store
                    .get_document::<serde_json::Value>("resource_draft", "offline-draft")?
                    .unwrap()
                    .value;
                assert_eq!(draft["schema_version"], 1);
                let parsed: improvement_runner::OfflineProfileDraft =
                    serde_json::from_value(draft.clone())?;
                assert_eq!(&parsed.project, store.identity());
                let candidate = store
                    .get_document::<CandidateRecord>(CANDIDATE_NAMESPACE, &run.candidate_id)?
                    .unwrap()
                    .value;
                assert_eq!(
                    candidate.content_digest,
                    improvement_evaluation::digest(&draft)?
                );
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn evaluation_http_disconnect_after_real_claim_preserves_owned_proof() {
        let (_root, app, runtime) = fixture().await;
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer fixture-secret".parse().unwrap());
        let authority = authenticated_operator_request(&app, &headers).unwrap();
        let offline = improvement_runner::OfflineComparison {
            criterion_id: "corpus".into(),
            cases: vec![improvement_runner::CorpusCase {
                id: "quiet".into(),
                intent: "status".into(),
                quiet: true,
                expected_delivery: false,
                max_chars: 20,
                forbidden: vec![],
            }],
            baseline: improvement_runner::OfflineProfile {
                outputs: std::collections::BTreeMap::from([("status".into(), "Ready".into())]),
                respect_quiet: true,
            },
        };
        let draft = serde_json::to_value(improvement_runner::OfflineProfileDraft {
            schema_version: 1,
            project: runtime.project(),
            profile: offline.baseline.clone(),
        })
        .unwrap();
        let spec = EvaluationSpec {
            schema_version: 1,
            project: runtime.project(),
            id: "offline-spec".into(),
            criteria: vec![improvement_evaluation::Criterion {
                id: "corpus".into(),
                command_or_protocol: "quiet fixed corpus".into(),
                expected_result: "no quiet delivery".into(),
                external_review: false,
            }],
            corpus_digest: improvement_evaluation::digest(&offline.cases).unwrap(),
            privacy_scope: "scratch".into(),
            authority: authority.clone(),
            max_age_seconds: 3600,
            execution: Some(improvement_runner::FixedExecution {
                commands: vec![],
                offline: Some(offline),
                output_limit_bytes: 1024,
                total_timeout_seconds: 1,
            }),
        };
        let candidate: CandidateRecord = serde_json::from_value(serde_json::json!({
            "schema_version":1,"project":runtime.project(),
            "candidate":{"schema_version":1,"candidate_id":"offline-candidate","project":runtime.project(),"target":"configuration","config_draft_ref":"offline-draft","evaluation_spec_version":spec.id,"proof_refs":[]},
            "target":"configuration","target_files":[],"content_digest":improvement_evaluation::digest(&draft).unwrap(),"evaluation_digest":improvement_evaluation::digest(&spec).unwrap(),"source_identity":"offline-v1","original_run_id":"original","original_problem":[],"submitted_by":authority
        })).unwrap();
        let grant = EvaluationGrant {
            candidate_digest: improvement_evaluation::digest(&candidate).unwrap(),
            evaluation_digest: improvement_evaluation::digest(&spec).unwrap(),
            run_id: "http-disconnect".into(),
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339(),
        };
        runtime
            .with_store(move |store| {
                store.put_document(CANDIDATE_NAMESPACE, "offline-candidate", &candidate, None)?;
                improvement_evaluation::register_spec(store, &spec, &spec.authority)?;
                store.put_document("resource_draft", "offline-draft", &draft, None)?;
                Ok(())
            })
            .await
            .unwrap();
        app.buddy
            .buddy
            .lock()
            .await
            .as_mut()
            .unwrap()
            .operator_features
            .improvement = true;
        let (_gate, claimed, release, finished) =
            improvement_runner::gate_claim(runtime.project().canonical_root, grant.run_id.clone());
        let router = super::super::make_v1_router(app.clone()).with_state(app.clone());
        let mut response = tokio::spawn(
            router.oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/buddy/improvement/candidates/offline-candidate/evaluate")
                    .header("authorization", "Bearer fixture-secret")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&grant).unwrap()))
                    .unwrap(),
            ),
        );
        tokio::select! {
            result = claimed => result.unwrap(),
            result = &mut response => panic!("evaluation ended before durable claim: {result:?}"),
        }
        response.abort();
        assert!(response.await.unwrap_err().is_cancelled());
        let uri = "/buddy/improvement/evaluations/http-disconnect?project_id=evaluation-fixture";
        let (status, body) = request(&app, "GET", uri, Some("fixture-secret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["data"]["run"]["value"]["state"],
            "claimed_outcome_unknown_until_finalized"
        );
        assert!(body["data"]["proof"].is_null());
        release.send(()).unwrap();
        finished.await.unwrap();
        app.buddy
            .buddy
            .lock()
            .await
            .as_mut()
            .unwrap()
            .operator_features
            .improvement = false;
        let (status, body) = request(&app, "GET", uri, Some("fixture-secret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["data"]["run"]["value"]["state"],
            "finished_no_activation"
        );
        assert_eq!(body["data"]["proof_status"], "current_no_activation");
        assert_eq!(
            body["data"]["proof"]["value"]["candidate_digest"],
            grant.candidate_digest
        );
        assert_eq!(
            body["data"]["proof"]["value"]["evaluation_digest"],
            grant.evaluation_digest
        );
        assert_eq!(
            body["data"]["proof"]["value"]["checks"][0]["result"],
            "passed"
        );
        assert_eq!(
            body["data"]["outcome"]["value"]["run_id"],
            "http-disconnect"
        );
        assert_eq!(body["data"]["run"]["value"]["cancel_requested"], false);
        assert_persisted_comparison(&runtime, &grant).await;
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn disconnected_claim_is_readable_and_cancellable_with_activation_disabled() {
        let (_root, app, runtime) = fixture().await;
        let run = claim(&runtime, "disconnected");
        let (tx, rx) = tokio::sync::oneshot::channel();
        drop(rx);
        let owned = runtime.clone();
        tokio::spawn(async move {
            let stored = owned
                .with_store(move |store| {
                    // Native schema-7 runs reserve completion capacity before the
                    // durable claim, as improvement_runner::evaluate does. A fresh unreserved row
                    // is not a pre-migration run eligible for grandfathering.
                    store.reserve_terminal_documents(
                        "fixed_evaluation",
                        "disconnected",
                        &[
                            (improvement_runner::RUN_NAMESPACE, "disconnected".into(), 16),
                            ("improvement_comparisons", "disconnected".into(), 1),
                            ("improvement_proofs", "fixed:disconnected".into(), 1),
                        ],
                    )?;
                    store.put_reserved_document(
                        "fixed_evaluation",
                        "disconnected",
                        improvement_runner::RUN_NAMESPACE,
                        "disconnected",
                        &run,
                        None,
                    )
                })
                .await
                .unwrap();
            assert!(tx.send(stored).is_err());
        })
        .await
        .unwrap();
        let uri = "/buddy/improvement/evaluations/disconnected?project_id=evaluation-fixture";
        let (status, body) = request(&app, "GET", uri, Some("fixture-secret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["data"]["run"]["value"]["state"],
            "claimed_outcome_unknown_until_finalized"
        );
        assert_eq!(body["data"]["run"]["value"]["process_id"], "native-process");
        assert_eq!(
            body["data"]["run"]["value"]["grant"]["candidate_digest"],
            "a".repeat(64)
        );
        assert!(body["data"]["proof"].is_null());
        assert!(body["data"]["outcome"].is_null());
        assert_eq!(body["data"]["run"]["value"]["cancel_requested"], false);
        for token in [None, Some("wrong")] {
            assert_eq!(
                request(
                    &app,
                    "POST",
                    "/buddy/improvement/evaluations/disconnected/cancel",
                    token,
                )
                .await
                .0,
                StatusCode::CONFLICT
            );
            assert_eq!(
                request(&app, "GET", uri, Some("fixture-secret")).await.1,
                body
            );
        }
        assert_eq!(
            request(
                &app,
                "POST",
                "/buddy/improvement/evaluations/disconnected/cancel",
                Some("fixture-secret")
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            request(&app, "GET", uri, Some("fixture-secret")).await.1["data"]["run"]["value"]
                ["cancel_requested"],
            true
        );
        assert!(
            !app.buddy
                .buddy
                .lock()
                .await
                .as_ref()
                .unwrap()
                .operator_features
                .improvement
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn finished_identity_evidence_and_stale_proof_are_retrieved_honestly() {
        let (_root, app, runtime) = fixture().await;
        let mut run = claim(&runtime, "finished");
        let now = chrono::Utc::now().to_rfc3339();
        let spec = EvaluationSpec {
            schema_version: 1,
            project: runtime.project(),
            id: "frozen-spec".into(),
            criteria: vec![improvement_evaluation::Criterion {
                id: "check".into(),
                command_or_protocol: "fixed protocol".into(),
                expected_result: "pass".into(),
                external_review: false,
            }],
            corpus_digest: "c".repeat(64),
            privacy_scope: "scratch".into(),
            authority: run.requester.clone(),
            max_age_seconds: 3600,
            execution: Some(improvement_runner::FixedExecution {
                commands: vec![],
                offline: None,
                output_limit_bytes: 1024,
                total_timeout_seconds: 1,
            }),
        };
        let candidate: CandidateRecord = serde_json::from_value(serde_json::json!({
            "schema_version":1,"project":runtime.project(),
            "candidate":{"schema_version":1,"candidate_id":"candidate","project":runtime.project(),"target":"source_candidate","evaluation_spec_version":spec.id,"proof_refs":[]},
            "target":"source_candidate","target_files":["source.rs"],"content_digest":"a".repeat(64),"evaluation_digest":improvement_evaluation::digest(&spec).unwrap(),"source_identity":"b".repeat(64),"original_run_id":"original","original_problem":[],"submitted_by":run.requester
        })).unwrap();
        run.grant.candidate_digest = improvement_evaluation::digest(&candidate).unwrap();
        run.grant.evaluation_digest = improvement_evaluation::digest(&spec).unwrap();
        run.state = "finished_no_activation".into();
        run.process_id = None;
        let evidence = refact_buddy_core::operator_types::EvidenceRef {
            evidence_id: "finished:check".into(),
            source: run.evaluator.clone(),
            locator: "buddy-evaluation://finished".into(),
            observed_at: now.clone(),
            digest: Some("d".repeat(64)),
        };
        let proof = EvaluationProof {
            schema_version: 1,
            project: runtime.project(),
            id: "fixed:finished".into(),
            candidate_digest: run.grant.candidate_digest.clone(),
            evaluation_digest: run.grant.evaluation_digest.clone(),
            authority: run.evaluator.clone(),
            observed_at: now.clone(),
            checks: vec![improvement_evaluation::CheckObservation {
                criterion_id: "check".into(),
                command_or_protocol: "fixed protocol".into(),
                result: improvement_evaluation::CheckResult::Failed,
                observed_result: "fixed check failed".into(),
                evidence: evidence.clone(),
                run_id: "finished".into(),
            }],
        };
        let mut start = runtime.start_workflow("fixed_candidate_evaluation", now);
        start.run_id = "finished".into();
        start.input_fingerprint = run.grant.candidate_digest.clone();
        runtime.begin_workflow(start.clone()).await.unwrap();
        runtime
            .publish_report(
                start,
                "Fixed evaluation".into(),
                "Failure is not activation".into(),
                vec![evidence],
                crate::buddy::outcome_runtime::WorkflowTerminal::Completed,
            )
            .await
            .unwrap();
        let expected_candidate_digest = run.grant.candidate_digest.clone();
        let expected_spec_digest = run.grant.evaluation_digest.clone();
        runtime
            .with_store(move |store| {
                store.put_document(CANDIDATE_NAMESPACE, "candidate", &candidate, None)?;
                store.put_document(
                    improvement_evaluation::SPEC_NAMESPACE,
                    &spec.id,
                    &spec,
                    None,
                )?;
                store.put_document(improvement_runner::RUN_NAMESPACE, "finished", &run, None)?;
                store.put_document(
                    improvement_evaluation::PROOF_NAMESPACE,
                    &proof.id,
                    &proof,
                    None,
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let uri = "/buddy/improvement/evaluations/finished?project_id=evaluation-fixture";
        let (status, body) = request(&app, "GET", uri, Some("fixture-secret")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["evaluation_spec_id"], "frozen-spec");
        assert_eq!(body["data"]["proof_status"], "current_no_activation");
        assert_eq!(
            body["data"]["proof"]["value"]["candidate_digest"],
            expected_candidate_digest
        );
        assert_eq!(
            body["data"]["proof"]["value"]["evaluation_digest"],
            expected_spec_digest
        );
        assert_eq!(
            body["data"]["proof"]["value"]["checks"][0]["result"],
            "failed"
        );
        assert_eq!(
            body["data"]["outcome"]["value"]["evidence"][0]["evidence_id"],
            "finished:check"
        );
        runtime
            .with_store(|store| {
                let old = store
                    .get_document::<EvaluationProof>(
                        improvement_evaluation::PROOF_NAMESPACE,
                        "fixed:finished",
                    )?
                    .unwrap();
                let mut stale = old.value;
                stale.observed_at = "2000-01-01T00:00:00Z".into();
                store.put_document(
                    improvement_evaluation::PROOF_NAMESPACE,
                    "fixed:finished",
                    &stale,
                    Some(old.revision),
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let (_, body) = request(&app, "GET", uri, Some("fixture-secret")).await;
        assert_eq!(body["data"]["proof_status"], "withheld_stale_or_mismatched");
        assert!(body["data"]["proof"].is_null());
        assert_eq!(
            body["data"]["run"]["value"]["state"],
            "finished_no_activation"
        );
        runtime
            .with_store(|store| {
                let old = store
                    .get_document::<FixedRun>(improvement_runner::RUN_NAMESPACE, "finished")?
                    .unwrap();
                let mut stopped = old.value;
                stopped.state = "stopped_outcome_unknown_no_replay".into();
                store.put_document(
                    improvement_runner::RUN_NAMESPACE,
                    "finished",
                    &stopped,
                    Some(old.revision),
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let (_, body) = request(&app, "GET", uri, Some("fixture-secret")).await;
        assert_eq!(
            body["data"]["run"]["value"]["state"],
            "stopped_outcome_unknown_no_replay"
        );
        assert_eq!(body["data"]["proof_status"], "not_finalized");
        assert!(body["data"]["proof"].is_null());
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn evaluation_routes_enforce_auth_project_and_native_pagination_bounds() {
        let (_root, app, runtime) = fixture().await;
        for id in ["first", "second"] {
            let run = claim(&runtime, id);
            runtime
                .with_store(move |store| {
                    store.put_document(improvement_runner::RUN_NAMESPACE, id, &run, None)
                })
                .await
                .unwrap();
        }
        for uri in [
            "/buddy/improvement/evaluations?project_id=evaluation-fixture",
            "/buddy/improvement/evaluations/first?project_id=evaluation-fixture",
        ] {
            for token in [None, Some("wrong")] {
                assert_eq!(
                    request(&app, "GET", uri, token).await.0,
                    StatusCode::CONFLICT
                );
            }
            assert_eq!(
                request(
                    &app,
                    "GET",
                    &uri.replace("evaluation-fixture", "other"),
                    Some("fixture-secret")
                )
                .await
                .0,
                StatusCode::CONFLICT
            );
        }
        for suffix in ["limit=0", "limit=201", "cursor=18446744073709551615"] {
            assert_eq!(
                request(
                    &app,
                    "GET",
                    &format!(
                        "/buddy/improvement/evaluations?project_id=evaluation-fixture&{suffix}"
                    ),
                    Some("fixture-secret")
                )
                .await
                .0,
                StatusCode::CONFLICT
            );
        }
        let (_, first) = request(
            &app,
            "GET",
            "/buddy/improvement/evaluations?project_id=evaluation-fixture&limit=1",
            Some("fixture-secret"),
        )
        .await;
        assert_eq!(first["data"].as_array().unwrap().len(), 1);
        assert_eq!(first["data"][0]["run"]["value"]["grant"]["run_id"], "first");
        let cursor = first["next_cursor"].as_u64().unwrap();
        let (_, second) = request(&app, "GET", &format!("/buddy/improvement/evaluations?project_id=evaluation-fixture&limit=1&cursor={cursor}"), Some("fixture-secret")).await;
        assert_eq!(
            second["data"][0]["run"]["value"]["grant"]["run_id"],
            "second"
        );
        assert!(second["next_cursor"].is_null());
        assert_eq!(
            request(
                &app,
                "GET",
                "/buddy/improvement/evaluations/missing?project_id=evaluation-fixture",
                Some("fixture-secret")
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        *app.gcx.documents_state.workspace_folders.lock().unwrap() = vec![];
        assert_eq!(
            request(
                &app,
                "GET",
                "/buddy/improvement/evaluations?project_id=evaluation-fixture",
                Some("fixture-secret")
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        runtime.shutdown().await;
    }
}
