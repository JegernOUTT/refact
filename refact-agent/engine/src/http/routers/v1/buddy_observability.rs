//! On-demand diagnostics and policy-control previews. Policy mutation is owned by
//! the daemon's distinct user-authenticated transport, never a worker bearer.
//! Previews are ephemeral and cannot themselves confer authority.
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use crate::app_state::AppState;
use crate::buddy::operator_runtime::{authenticated_operator_request, runtime, OperatorCapability};
use crate::custom_error::ScratchError;

use crate::buddy::operator_observability::{DocumentGrowthPolicy, PolicySnapshot};
use crate::buddy::operator_store::{OperatorStore, OperatorStoreError};
use crate::buddy::resources::retention::{PolicyControlAudit, PolicyControlRecord};
use refact_buddy_core::operator_types::{NativeSourceIdentity, ProjectIdentity, WorkerIdentity};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyProposal {
    pub project_id: String,
    pub policy: Option<DocumentGrowthPolicy>,
}

/// Ephemeral exact consent material, not a grant and never stored by preview.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConsent {
    pub project: ProjectIdentity,
    pub worker: WorkerIdentity,
    pub before: PolicySnapshot,
    pub proposed_policy: Option<DocumentGrowthPolicy>,
    pub current_charge: usize,
    pub authority_class: String,
    pub confirmation_class: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyControlPreview {
    pub consent: PolicyConsent,
    pub digest: String,
    pub budget_formula: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfirmation {
    pub consent: PolicyConsent,
    pub expected_digest: String,
    pub idempotency_key: String,
    pub confirm: bool,
}

fn consent_digest(consent: &PolicyConsent) -> Result<String, OperatorStoreError> {
    use sha2::{Digest, Sha256};
    let encoded = serde_json::to_vec(consent)
        .map_err(|_| OperatorStoreError::InvalidInput("invalid policy consent"))?;
    let mut digest = Sha256::new();
    digest.update(b"refact-native-policy-consent-v1\0");
    digest.update(encoded);
    Ok(format!("{:x}", digest.finalize()))
}

pub(crate) fn preview_policy_control(
    store: &OperatorStore,
    project: ProjectIdentity,
    worker: WorkerIdentity,
    proposal: PolicyProposal,
) -> Result<PolicyControlPreview, OperatorStoreError> {
    preview_policy_control_from_snapshot(
        store.policy_snapshot()?,
        store.document_policy_charge()?,
        project,
        worker,
        proposal,
    )
}

pub(crate) fn preview_policy_control_from_snapshot(
    before: PolicySnapshot,
    current_charge: usize,
    project: ProjectIdentity,
    worker: WorkerIdentity,
    proposal: PolicyProposal,
) -> Result<PolicyControlPreview, OperatorStoreError> {
    if proposal.project_id != project.project_id {
        return Err(OperatorStoreError::ProjectMismatch);
    }
    if proposal
        .policy
        .as_ref()
        .is_some_and(|p| !(1..=100_000).contains(&p.max_rows_per_namespace))
    {
        return Err(OperatorStoreError::InvalidInput("invalid policy capacity"));
    }
    let consent = PolicyConsent {
        project,
        worker,
        before,
        proposed_policy: proposal.policy,
        current_charge,
        authority_class: "mutating".into(),
        confirmation_class: "user_confirmation_required".into(),
    };
    Ok(PolicyControlPreview {
        digest: consent_digest(&consent)?,
        consent,
        budget_formula: format!(
            "immutable baseline + {} * max_rows_per_namespace; retained documents, outstanding terminal obligations, reservation ledger and policy history/audit are charged; no evidence deletion or hard SQLite byte bound",
            crate::buddy::operator_observability::GLOBAL_DOCUMENT_BUDGET_MULTIPLIER,
        ),
    })
}

/// Called only after the daemon's separate Basic-user check and stopped-worker
/// fence. The worker bearer route deliberately cannot call this mutation helper.
pub(crate) fn confirm_policy_control(
    store: &mut OperatorStore,
    project: &ProjectIdentity,
    worker: &WorkerIdentity,
    requester: NativeSourceIdentity,
    request: PolicyConfirmation,
) -> Result<PolicyControlRecord, OperatorStoreError> {
    let consent = &request.consent;
    if !request.confirm
        || consent.project != *project
        || consent.worker != *worker
        || consent.authority_class != "mutating"
        || consent.confirmation_class != "user_confirmation_required"
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 128
        || request.expected_digest != consent_digest(consent)?
    {
        return Err(OperatorStoreError::InvalidInput(
            "exact user policy confirmation required",
        ));
    }
    let next = consent
        .before
        .epoch
        .checked_add(1)
        .ok_or(OperatorStoreError::Conflict("policy epoch exhausted"))?;
    let receipt_id = format!("document-policy:{next}");
    // Check the immutable before snapshot even for a response-lost duplicate.
    let before =
        match store.get_document::<PolicyControlRecord>("policy_control_audit", &receipt_id)? {
            Some(saved) => saved.value.before,
            None => store.policy_snapshot()?,
        };
    if before != consent.before {
        return Err(OperatorStoreError::Conflict(
            "policy changed; preview and confirm again",
        ));
    }
    let audit = PolicyControlAudit {
        project: project.clone(),
        worker: worker.clone(),
        requester,
        spec_hash: request.expected_digest,
        idempotency_key: request.idempotency_key,
        expected_charge: consent.current_charge,
    };
    store.apply_document_policy_with_audit(
        &consent.before.version(),
        consent.proposed_policy.clone(),
        &audit,
    )?;
    store
        .get_document::<PolicyControlRecord>("policy_control_audit", &receipt_id)?
        .map(|saved| saved.value)
        .ok_or(OperatorStoreError::Conflict("policy receipt unavailable"))
}

pub async fn handle_policy_preview(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(proposal): axum::Json<PolicyProposal>,
) -> Result<axum::Json<PolicyControlPreview>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(rejected)?;
    let owner = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(rejected)?;
    let project = owner.project();
    let worker = owner.worker();
    owner
        .with_store(move |store| preview_policy_control(store, project, worker, proposal))
        .await
        .map(axum::Json)
        .map_err(rejected)
}

pub async fn handle_policy_confirm() -> Result<axum::Json<PolicyControlRecord>, ScratchError> {
    Err(ScratchError::new(
        hyper::StatusCode::FORBIDDEN,
        "policy_confirmation_requires_daemon_user_auth_and_stopped_workers".into(),
    ))
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn response_lost_policy_replay_is_bound_to_requester_and_current_charge() {
        let temp = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "scratch".into(),
            canonical_root: temp
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let worker = WorkerIdentity {
            worker_id: "native".into(),
            engine_instance_id: "daemon-1".into(),
            epoch: 1,
        };
        let user = NativeSourceIdentity {
            source_kind: refact_buddy_core::operator_types::NativeSourceKind::External,
            source_id: "fixture-user".into(),
            source_version: "daemon-basic-user-v1".into(),
        };
        let mut store = OperatorStore::open(temp.path(), &project).unwrap();
        let proposal = PolicyProposal {
            project_id: project.project_id.clone(),
            policy: Some(DocumentGrowthPolicy::default()),
        };
        let preview =
            preview_policy_control(&store, project.clone(), worker.clone(), proposal.clone())
                .unwrap();
        let request = PolicyConfirmation {
            consent: preview.consent,
            expected_digest: preview.digest,
            idempotency_key: "native-policy-confirmation".into(),
            confirm: true,
        };
        let first =
            confirm_policy_control(&mut store, &project, &worker, user.clone(), request.clone())
                .unwrap();
        let charge = store.document_policy_charge().unwrap();
        let seq = store.retention_preview().unwrap().sequence_high_watermark;
        let mut foreign = user.clone();
        foreign.source_id = "other-user".into();
        assert!(
            confirm_policy_control(&mut store, &project, &worker, foreign, request.clone())
                .is_err()
        );
        assert_eq!(store.document_policy_charge().unwrap(), charge);
        assert_eq!(
            store.retention_preview().unwrap().sequence_high_watermark,
            seq
        );
        drop(store);
        let mut store = OperatorStore::open(temp.path(), &project).unwrap();
        assert_eq!(
            confirm_policy_control(&mut store, &project, &worker, user.clone(), request.clone())
                .unwrap(),
            first
        );
        assert_eq!(store.document_policy_charge().unwrap(), charge);
        assert_eq!(
            store.retention_preview().unwrap().sequence_high_watermark,
            seq
        );
        let preview =
            preview_policy_control(&store, project.clone(), worker.clone(), proposal.clone())
                .unwrap();
        let reused = PolicyConfirmation {
            consent: preview.consent.clone(),
            expected_digest: preview.digest.clone(),
            ..request.clone()
        };
        assert!(
            confirm_policy_control(&mut store, &project, &worker, user.clone(), reused).is_err()
        );
        let fresh = PolicyConfirmation {
            consent: preview.consent,
            expected_digest: preview.digest,
            idempotency_key: "next-native-confirmation".into(),
            confirm: true,
        };
        let value = serde_json::json!({"schema_version":1,"project":project,"observed":true});
        store
            .put_document("independent_observation", "new", &value, None)
            .unwrap();
        let new_charge = store.document_policy_charge().unwrap();
        let new_seq = store.retention_preview().unwrap().sequence_high_watermark;
        assert!(
            confirm_policy_control(&mut store, &project, &worker, user.clone(), fresh).is_err()
        );
        assert_eq!(store.document_policy_charge().unwrap(), new_charge);
        assert_eq!(
            store.retention_preview().unwrap().sequence_high_watermark,
            new_seq
        );
        let preview =
            preview_policy_control(&store, project.clone(), worker.clone(), proposal).unwrap();
        confirm_policy_control(
            &mut store,
            &project,
            &worker,
            user,
            PolicyConfirmation {
                consent: preview.consent,
                expected_digest: preview.digest,
                idempotency_key: "next-native-confirmation".into(),
                confirm: true,
            },
        )
        .unwrap();
        assert_eq!(
            store.policy_snapshot().unwrap().epoch,
            first.after.epoch + 1
        );
    }

    #[tokio::test]
    async fn worker_bearer_policy_confirmation_route_never_mints_user_authority() {
        use tower::ServiceExt;
        let router =
            axum::Router::new().route("/confirm", axum::routing::post(handle_policy_confirm));
        for bearer in ["model", "worker", "authenticated-user"] {
            let response = router
                .clone()
                .oneshot(
                    hyper::Request::builder()
                        .method("POST")
                        .uri("/confirm")
                        .header("authorization", format!("Bearer {bearer}"))
                        .body(hyper::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), hyper::StatusCode::FORBIDDEN);
        }
    }

    #[test]
    fn daemon_restart_requires_new_preview_without_writing_a_grant() {
        let temp = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "scratch".into(),
            canonical_root: temp
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let mut worker = WorkerIdentity {
            worker_id: "native".into(),
            engine_instance_id: "daemon-1".into(),
            epoch: 1,
        };
        let mut store = OperatorStore::open(temp.path(), &project).unwrap();
        let preview = preview_policy_control(
            &store,
            project.clone(),
            worker.clone(),
            PolicyProposal {
                project_id: project.project_id.clone(),
                policy: Some(DocumentGrowthPolicy::default()),
            },
        )
        .unwrap();
        let charge = store.document_policy_charge().unwrap();
        worker.epoch += 1;
        worker.engine_instance_id = "daemon-2".into();
        assert!(confirm_policy_control(
            &mut store,
            &project,
            &worker,
            NativeSourceIdentity {
                source_kind: refact_buddy_core::operator_types::NativeSourceKind::External,
                source_id: "fixture-user".into(),
                source_version: "daemon-basic-user-v1".into()
            },
            PolicyConfirmation {
                consent: preview.consent,
                expected_digest: preview.digest,
                idempotency_key: "restart".into(),
                confirm: true
            }
        )
        .is_err());
        assert_eq!(store.policy_snapshot().unwrap().epoch, 0);
        assert_eq!(store.document_policy_charge().unwrap(), charge);
    }

    #[test]
    fn ephemeral_digest_binds_worker_epoch_charge_proposal_and_authority() {
        let project = ProjectIdentity {
            project_id: "scratch".into(),
            canonical_root: "/scratch".into(),
        };
        let worker = WorkerIdentity {
            worker_id: "native".into(),
            engine_instance_id: "daemon-1".into(),
            epoch: 1,
        };
        let preview = preview_policy_control_from_snapshot(
            PolicySnapshot {
                epoch: 4,
                policy: None,
            },
            64,
            project,
            worker,
            PolicyProposal {
                project_id: "scratch".into(),
                policy: Some(DocumentGrowthPolicy {
                    max_rows_per_namespace: 8,
                    replace_editorial_aggregate: false,
                }),
            },
        )
        .unwrap();
        let mutations: [fn(&mut PolicyConsent); 6] = [
            |c| c.project.project_id.push('x'),
            |c| c.worker.epoch += 1,
            |c| c.before.epoch += 1,
            |c| c.current_charge += 1,
            |c| c.proposed_policy = None,
            |c| c.authority_class = "readonly".into(),
        ];
        for change in mutations {
            let mut changed = preview.consent.clone();
            change(&mut changed);
            assert_ne!(consent_digest(&changed).unwrap(), preview.digest);
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub project_id: String,
}

fn rejected(_: impl std::fmt::Display) -> ScratchError {
    // SQLite and IO errors can carry local paths. Never send them as diagnostics.
    ScratchError::new(
        hyper::StatusCode::CONFLICT,
        "operator_diagnostics_unavailable".into(),
    )
}

pub async fn handle_summary(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(request): Query<Request>,
) -> Result<axum::Json<crate::buddy::operator_observability::StoreSummary>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(rejected)?;
    let owner = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(rejected)?;
    if owner.project().project_id != request.project_id {
        return Err(rejected("project changed"));
    }
    let (summary, warnings) = owner
        .with_store(|store| store.diagnostic_summary_with_growth_warnings())
        .await
        .map_err(rejected)?;
    if !warnings.is_empty() {
        let mut service = app.buddy.buddy.lock().await;
        if let Some(service) = service.as_mut() {
            crate::buddy::actor::publish_operator_store_growth_warnings(service, warnings);
        }
    }
    Ok(axum::Json(summary))
}

pub async fn handle_retention_preview(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(request): Query<Request>,
) -> Result<axum::Json<crate::buddy::operator_observability::RetentionPreview>, ScratchError> {
    authenticated_operator_request(&app, &headers).map_err(rejected)?;
    let owner = runtime(&app, OperatorCapability::Storage)
        .await
        .map_err(rejected)?;
    if owner.project().project_id != request.project_id {
        return Err(rejected("project changed"));
    }
    owner
        .with_store(|store| store.retention_preview())
        .await
        .map(axum::Json)
        .map_err(rejected)
}
