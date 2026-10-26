use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;

use crate::app_state::AppState;
use crate::buddy::operator_runtime::{authenticated_operator_request, runtime, OperatorCapability};
use crate::buddy::supervision::{
    self, CandidateAssessmentRequest, ControlRequest, GoalIntake, MessageRequest,
};
use crate::buddy::supervision_authority::{self as authority, MandateRequest};
use crate::custom_error::ScratchError;

fn error(message: String) -> ScratchError {
    ScratchError::new(StatusCode::CONFLICT, message)
}

fn validate_binding(
    headers: &HeaderMap,
    project: &refact_buddy_core::operator_types::ProjectIdentity,
    worker: &refact_buddy_core::operator_types::WorkerIdentity,
) -> Result<(), String> {
    for (name, expected) in [
        ("x-refact-project-id", project.project_id.clone()),
        ("x-refact-worker-id", worker.worker_id.clone()),
        (
            "x-refact-engine-instance",
            worker.engine_instance_id.clone(),
        ),
        ("x-refact-worker-epoch", worker.epoch.to_string()),
    ] {
        let mut values = headers.get_all(name).iter();
        if values.next().and_then(|v| v.to_str().ok()) != Some(expected.as_str())
            || values.next().is_some()
        {
            return Err(format!("missing, duplicate, or stale {name} binding"));
        }
    }
    Ok(())
}

async fn authorized_runtime(
    app: &AppState,
    headers: &HeaderMap,
) -> Result<crate::buddy::outcome_runtime::OutcomeRuntime, ScratchError> {
    authenticated_operator_request(app, headers)
        .map_err(|e| ScratchError::new(StatusCode::UNAUTHORIZED, e))?;
    let runtime = runtime(app, OperatorCapability::Supervision)
        .await
        .map_err(|e| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
    validate_binding(headers, &runtime.project(), &runtime.worker()).map_err(error)?;
    Ok(runtime)
}

pub async fn context(State(app): State<AppState>, headers: HeaderMap) -> Json<serde_json::Value> {
    let authenticated_owner = authenticated_operator_request(&app, &headers).ok();
    let roots = crate::files_correction::get_unscoped_project_dirs(app.gcx.clone()).await;
    let (features, project, worker) = {
        let buddy = app.buddy.buddy.lock().await;
        match buddy
            .as_ref()
            .filter(|service| roots.len() == 1 && roots[0] == service.project_root)
        {
            Some(service) => match service.outcome_runtime.as_ref().filter(|runtime| {
                runtime.project().canonical_root == service.project_root.to_string_lossy()
            }) {
                Some(runtime) => (
                    service.operator_features,
                    Some(runtime.project()),
                    Some(runtime.worker()),
                ),
                None => (Default::default(), None, None),
            },
            None => (Default::default(), None, None),
        }
    };
    Json(serde_json::json!({
        "schema_version": 1,
        "features": features,
        "project": project,
        "worker": worker,
        "authenticated_owner": authenticated_owner,
    }))
}

fn bound_response(
    value: impl serde::Serialize,
    runtime: &crate::buddy::outcome_runtime::OutcomeRuntime,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let mut value = serde_json::to_value(value).map_err(|e| error(e.to_string()))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| error("invalid supervision response".into()))?;
    object.insert("worker".into(), serde_json::json!(runtime.worker()));
    object.insert("project".into(), serde_json::json!(runtime.project()));
    Ok(Json(value))
}

pub async fn create_mandate(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MandateRequest>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let requester = authenticated_operator_request(&app, &headers)
        .map_err(|e| ScratchError::new(StatusCode::UNAUTHORIZED, e))?;
    let runtime = authorized_runtime(&app, &headers).await?;
    let result = authority::create(&runtime, requester, request)
        .await
        .map_err(error)?;
    Ok(Json(serde_json::json!(result)))
}

pub async fn get_mandate(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let runtime = authorized_runtime(&app, &headers).await?;
    let result = authority::load(&runtime, id).await.map_err(error)?;
    Ok(Json(serde_json::json!(result)))
}

pub async fn revoke_mandate(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let runtime = authorized_runtime(&app, &headers).await?;
    if supervision::load_link(&runtime, id.clone()).await.is_ok() {
        return bound_response(
            supervision::stop(&app, &runtime, id).await.map_err(error)?,
            &runtime,
        );
    }
    Ok(Json(serde_json::json!(authority::revoke(&runtime, id)
        .await
        .map_err(error)?)))
}

pub async fn intake_goal(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<GoalIntake>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let runtime = authorized_runtime(&app, &headers).await?;
    bound_response(
        supervision::intake(&app, &runtime, id, request)
            .await
            .map_err(error)?,
        &runtime,
    )
}

pub async fn spawn_worker(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let runtime = authorized_runtime(&app, &headers).await?;
    bound_response(
        supervision::spawn_worker(&app, &runtime, id)
            .await
            .map_err(error)?,
        &runtime,
    )
}

pub async fn get_status(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let runtime = authorized_runtime(&app, &headers).await?;
    bound_response(
        supervision::status(&app, &runtime, id)
            .await
            .map_err(error)?,
        &runtime,
    )
}

pub async fn reconcile(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let runtime = authorized_runtime(&app, &headers).await?;
    bound_response(
        supervision::reconcile(&app, &runtime, id)
            .await
            .map_err(error)?,
        &runtime,
    )
}

pub async fn routine_question(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let runtime = authorized_runtime(&app, &headers).await?;
    bound_response(
        supervision::answer_routine_question(&app, &runtime, id)
            .await
            .map_err(error)?,
        &runtime,
    )
}

/// Typed steering: only a mandate-listed `note_id`, never request text.
pub async fn message_worker(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<MessageRequest>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let requester = authenticated_operator_request(&app, &headers)
        .map_err(|e| ScratchError::new(StatusCode::UNAUTHORIZED, e))?;
    let runtime = authorized_runtime(&app, &headers).await?;
    bound_response(
        supervision::message_worker(&app, &runtime, id, requester, request)
            .await
            .map_err(error)?,
        &runtime,
    )
}

pub async fn pause_worker(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<ControlRequest>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let requester = authenticated_operator_request(&app, &headers)
        .map_err(|e| ScratchError::new(StatusCode::UNAUTHORIZED, e))?;
    let runtime = authorized_runtime(&app, &headers).await?;
    bound_response(
        supervision::pause_worker(&app, &runtime, id, requester, request)
            .await
            .map_err(error)?,
        &runtime,
    )
}

pub async fn resume_worker(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<ControlRequest>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let requester = authenticated_operator_request(&app, &headers)
        .map_err(|e| ScratchError::new(StatusCode::UNAUTHORIZED, e))?;
    let runtime = authorized_runtime(&app, &headers).await?;
    bound_response(
        supervision::resume_worker(&app, &runtime, id, requester, request)
            .await
            .map_err(error)?,
        &runtime,
    )
}

pub async fn cancel_worker(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<ControlRequest>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let requester = authenticated_operator_request(&app, &headers)
        .map_err(|e| ScratchError::new(StatusCode::UNAUTHORIZED, e))?;
    let runtime = authorized_runtime(&app, &headers).await?;
    bound_response(
        supervision::cancel_worker(&app, &runtime, id, requester, request)
            .await
            .map_err(error)?,
        &runtime,
    )
}

pub async fn assess_candidate(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<CandidateAssessmentRequest>,
) -> Result<Json<serde_json::Value>, ScratchError> {
    let requester = authenticated_operator_request(&app, &headers)
        .map_err(|e| ScratchError::new(StatusCode::UNAUTHORIZED, e))?;
    let runtime = authorized_runtime(&app, &headers).await?;
    bound_response(
        supervision::assess_candidate(&app, &runtime, id, requester, request)
            .await
            .map_err(error)?,
        &runtime,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_identity_header_is_required_exact_and_unique() {
        let project = refact_buddy_core::operator_types::ProjectIdentity {
            project_id: "p".into(),
            canonical_root: "/p".into(),
        };
        let worker = refact_buddy_core::operator_types::WorkerIdentity {
            worker_id: "w".into(),
            engine_instance_id: "e".into(),
            epoch: 2,
        };
        let mut headers = HeaderMap::new();
        for (key, value) in [
            ("x-refact-project-id", "p"),
            ("x-refact-worker-id", "w"),
            ("x-refact-engine-instance", "e"),
            ("x-refact-worker-epoch", "2"),
        ] {
            headers.insert(key, value.parse().unwrap());
        }
        assert!(validate_binding(&headers, &project, &worker).is_ok());
        for key in [
            "x-refact-project-id",
            "x-refact-worker-id",
            "x-refact-engine-instance",
            "x-refact-worker-epoch",
        ] {
            let mut missing = headers.clone();
            missing.remove(key);
            assert!(validate_binding(&missing, &project, &worker).is_err());
            let mut stale = headers.clone();
            stale.insert(key, "stale".parse().unwrap());
            assert!(validate_binding(&stale, &project, &worker).is_err());
            let mut duplicate = headers.clone();
            duplicate.append(key, headers[key].clone());
            assert!(validate_binding(&duplicate, &project, &worker).is_err());
        }
    }

    #[test]
    fn control_bodies_reject_free_text_and_unknown_fields() {
        assert!(serde_json::from_value::<MessageRequest>(
            serde_json::json!({"control_id":"c1","note_id":"focus"})
        )
        .is_ok());
        assert!(serde_json::from_value::<MessageRequest>(
            serde_json::json!({"control_id":"c1","note_id":"focus","text":"do this instead"})
        )
        .is_err());
        assert!(serde_json::from_value::<MessageRequest>(
            serde_json::json!({"control_id":"c1","text":"free text"})
        )
        .is_err());
        assert!(serde_json::from_value::<ControlRequest>(
            serde_json::json!({"control_id":"c1","action":"stop"})
        )
        .is_err());
        assert!(serde_json::from_value::<ControlRequest>(
            serde_json::json!({"control_id":"c1","requested_by":{"source_kind":"chat","source_id":"x","source_version":"1"}})
        )
        .is_err());
    }

    #[test]
    fn candidate_assessment_body_is_exact() {
        assert!(serde_json::from_value::<CandidateAssessmentRequest>(
            serde_json::json!({"control_id":"c1","workflow_id":"security_whisperer"})
        )
        .is_ok());
        assert!(serde_json::from_value::<CandidateAssessmentRequest>(
            serde_json::json!({"control_id":"c1","workflow_id":"security_whisperer","chat_id":"forged"})
        )
        .is_err());
    }
}
