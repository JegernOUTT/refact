use axum::{extract::State, http::HeaderMap, Json};
use serde::{Deserialize, Serialize};
use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};
use crate::{app_state::AppState, custom_error::ScratchError};
use crate::buddy::{
    editorial::{self, Counters, Feedback, Surface},
    operator_runtime::{self, OperatorCapability},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedbackRequest {
    pub project: ProjectIdentity,
    pub worker: WorkerIdentity,
    pub speech_id: String,
    pub feedback: Feedback,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimRequest {
    pub project: ProjectIdentity,
    pub worker: WorkerIdentity,
    pub speech_id: String,
    pub surface: Surface,
}
#[derive(Serialize)]
pub struct ClaimResponse {
    pub claimed: bool,
    pub permission_granted: bool,
}
#[derive(Serialize)]
pub struct FeedbackResponse {
    pub recorded: bool,
    pub permission_granted: bool,
}

fn error(status: hyper::StatusCode, message: &str) -> ScratchError {
    ScratchError::new(status, message.into())
}
fn authorize(app: &AppState, headers: &HeaderMap) -> Result<(), ScratchError> {
    operator_runtime::authenticated_operator_request(app, headers)
        .map(|_| ())
        .map_err(|_| {
            error(
                hyper::StatusCode::UNAUTHORIZED,
                "authenticated operator request required",
            )
        })
}
fn validate_binding(
    expected_project: &ProjectIdentity,
    expected_worker: &WorkerIdentity,
    project: &ProjectIdentity,
    worker: &WorkerIdentity,
) -> Result<(), ScratchError> {
    if project != expected_project || worker != expected_worker {
        return Err(error(
            hyper::StatusCode::CONFLICT,
            "editorial project or worker changed; refresh context",
        ));
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), ScratchError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':'))
    {
        return Err(error(
            hyper::StatusCode::BAD_REQUEST,
            "invalid speech identity",
        ));
    }
    Ok(())
}

pub async fn handle_feedback(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<FeedbackRequest>,
) -> Result<Json<FeedbackResponse>, ScratchError> {
    authorize(&app, &headers)?;
    validate_id(&request.speech_id)?;
    let runtime = operator_runtime::runtime(&app, OperatorCapability::Editorial)
        .await
        .map_err(|_| {
            error(
                hyper::StatusCode::SERVICE_UNAVAILABLE,
                "editorial capability unavailable",
            )
        })?;
    validate_binding(
        &runtime.project(),
        &runtime.worker(),
        &request.project,
        &request.worker,
    )?;
    runtime
        .with_store(move |store| {
            editorial::with_state(store, |state| {
                state.feedback(
                    &request.speech_id,
                    request.feedback,
                    chrono::Utc::now().timestamp(),
                )
            })
        })
        .await
        .map_err(|_| {
            error(
                hyper::StatusCode::CONFLICT,
                "delivered speech unavailable; refresh before feedback",
            )
        })?;
    Ok(Json(FeedbackResponse {
        recorded: true,
        permission_granted: false,
    }))
}

pub async fn handle_claim(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ClaimRequest>,
) -> Result<Json<ClaimResponse>, ScratchError> {
    authorize(&app, &headers)?;
    validate_id(&request.speech_id)?;
    if request.surface == Surface::Avatar {
        return Err(error(
            hyper::StatusCode::FORBIDDEN,
            "avatar editorial activation awaits human quality review",
        ));
    }
    let runtime = operator_runtime::runtime(&app, OperatorCapability::Editorial)
        .await
        .map_err(|_| {
            error(
                hyper::StatusCode::SERVICE_UNAVAILABLE,
                "editorial capability unavailable",
            )
        })?;
    validate_binding(
        &runtime.project(),
        &runtime.worker(),
        &request.project,
        &request.worker,
    )?;
    let claimed = runtime
        .with_store(move |store| {
            editorial::with_state(store, |state| {
                state.claim(
                    &request.speech_id,
                    request.surface,
                    chrono::Utc::now().timestamp(),
                )
            })
        })
        .await
        .map_err(|_| {
            error(
                hyper::StatusCode::CONFLICT,
                "published speech unavailable; refresh before delivery",
            )
        })?;
    Ok(Json(ClaimResponse {
        claimed,
        permission_granted: false,
    }))
}

pub async fn handle_metrics(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Counters>, ScratchError> {
    authorize(&app, &headers)?;
    let runtime = operator_runtime::runtime(&app, OperatorCapability::Editorial)
        .await
        .map_err(|_| {
            error(
                hyper::StatusCode::SERVICE_UNAVAILABLE,
                "editorial capability unavailable",
            )
        })?;
    let counters = runtime
        .with_store(|store| editorial::snapshot(store))
        .await
        .map_err(|_| {
            error(
                hyper::StatusCode::SERVICE_UNAVAILABLE,
                "editorial counters unavailable",
            )
        })?;
    Ok(Json(counters))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_project() -> ProjectIdentity {
        ProjectIdentity {
            project_id: "p".into(),
            canonical_root: "/fixture".into(),
        }
    }
    fn fixture_worker() -> WorkerIdentity {
        WorkerIdentity {
            worker_id: "worker".into(),
            engine_instance_id: "engine".into(),
            epoch: 1,
        }
    }
    #[test]
    fn binding_rejects_other_project_root_and_engine_epoch() {
        let project = fixture_project();
        let worker = fixture_worker();
        assert!(validate_binding(&project, &worker, &project, &worker).is_ok());
        let mut other = project.clone();
        other.project_id = "other".into();
        assert!(validate_binding(&project, &worker, &other, &worker).is_err());
        other = project.clone();
        other.canonical_root = "/other".into();
        assert!(validate_binding(&project, &worker, &other, &worker).is_err());
        let mut restarted = worker.clone();
        restarted.engine_instance_id = "restarted".into();
        assert!(validate_binding(&project, &worker, &project, &restarted).is_err());
        restarted = worker.clone();
        restarted.worker_id = "other".into();
        assert!(validate_binding(&project, &worker, &project, &restarted).is_err());
    }

    #[test]
    fn feedback_body_cannot_carry_text_authority_or_identity() {
        let valid = serde_json::json!({"project": fixture_project(), "worker": fixture_worker(), "speech_id":"s", "feedback":"useful"});
        assert!(serde_json::from_value::<FeedbackRequest>(valid.clone()).is_ok());
        for field in ["text", "principal", "permission", "project", "settings"] {
            let mut body = valid.clone();
            body[field] = serde_json::json!("injected");
            assert!(serde_json::from_value::<FeedbackRequest>(body).is_err());
        }
        assert!(serde_json::from_value::<FeedbackRequest>(
            serde_json::json!({"speech_id":"s", "feedback":"approve"})
        )
        .is_err());
        assert!(validate_id("token=private").is_err());
    }
    #[tokio::test]
    async fn authenticated_claim_rejects_stale_binding_without_consuming_delivery() {
        use crate::buddy::{actor::BuddyService, outcome_runtime::OutcomeRuntime};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let project = ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: root.to_string_lossy().into_owned(),
        };
        let worker = fixture_worker();
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
            root,
            crate::buddy::state::default_buddy_state(),
            Default::default(),
            vec![],
            crate::buddy::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        service.operator_features.storage = true;
        service.operator_features.editorial = true;
        service.outcome_runtime = Some(runtime.clone());
        *app.buddy.buddy.lock().await = Some(service);
        runtime
            .with_store(|store| {
                editorial::with_state(store, |state| {
                    let now = chrono::Utc::now().timestamp();
                    let candidate = editorial::Candidate::new("win", "fixture", "fixture", now);
                    assert_eq!(
                        state.reserve(candidate.clone(), now),
                        editorial::Decision::Eligible
                    );
                    state.publish(&candidate.id, "fixture-speech", now)
                })
            })
            .await
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer fixture-secret".parse().unwrap());
        for wrong_project in [true, false] {
            let mut requested_project = project.clone();
            let mut requested_worker = worker.clone();
            if wrong_project {
                requested_project.project_id = "foreign".into();
            } else {
                requested_worker.epoch += 1;
            }
            assert!(handle_claim(
                State(app.clone()),
                headers.clone(),
                Json(ClaimRequest {
                    project: requested_project,
                    worker: requested_worker,
                    speech_id: "fixture-speech".into(),
                    surface: Surface::Panel,
                })
            )
            .await
            .is_err());
        }
        let first = handle_claim(
            State(app.clone()),
            headers.clone(),
            Json(ClaimRequest {
                project: project.clone(),
                worker: worker.clone(),
                speech_id: "fixture-speech".into(),
                surface: Surface::Panel,
            }),
        )
        .await
        .unwrap();
        assert!(first.0.claimed);
        assert!(!first.0.permission_granted);
        let duplicate = handle_claim(
            State(app),
            headers,
            Json(ClaimRequest {
                project,
                worker,
                speech_id: "fixture-speech".into(),
                surface: Surface::Notification,
            }),
        )
        .await
        .unwrap();
        assert!(!duplicate.0.claimed);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn unauthenticated_feedback_claim_and_metrics_fail_without_creating_storage() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        assert!(handle_feedback(
            State(app.clone()),
            HeaderMap::new(),
            Json(FeedbackRequest {
                project: fixture_project(),
                worker: fixture_worker(),
                speech_id: "s".into(),
                feedback: Feedback::Useful
            })
        )
        .await
        .is_err());
        assert!(handle_claim(
            State(app.clone()),
            HeaderMap::new(),
            Json(ClaimRequest {
                project: fixture_project(),
                worker: fixture_worker(),
                speech_id: "s".into(),
                surface: Surface::Panel
            })
        )
        .await
        .is_err());
        assert!(handle_metrics(State(app), HeaderMap::new()).await.is_err());
        assert!(!dir.path().join(".refact/buddy/operator.sqlite").exists());
    }
}
