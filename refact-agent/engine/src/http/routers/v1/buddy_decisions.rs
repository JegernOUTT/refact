use axum::extract::{Path, Query, State};
use hyper::StatusCode;
use refact_chat_api::{PendingQuestionSet, PendingQuestionStatus};
use serde::{Deserialize, Serialize};

use crate::{app_state::AppState, custom_error::ScratchError};

#[derive(Serialize)]
pub struct NativeQuestionView {
    project_root: String,
    chat_id: String,
    question: PendingQuestionSet,
}

#[derive(Serialize)]
pub struct NativeDecisionSnapshot {
    schema_version: u16,
    project_root: String,
    coverage: &'static str,
    truncated: bool,
    questions: Vec<NativeQuestionView>,
}

pub(crate) async fn decision_root(app: &AppState) -> Result<String, ScratchError> {
    let roots = crate::files_correction::get_unscoped_project_dirs(app.gcx.clone()).await;
    if roots.len() != 1 {
        return Err(ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "native question aggregation requires one unambiguous project root".into(),
        ));
    }
    let root = tokio::fs::canonicalize(&roots[0]).await.map_err(|_| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Native project root is unavailable".into(),
        )
    })?;
    Ok(root.to_string_lossy().into_owned())
}

pub async fn handle_current_decisions(
    State(app): State<AppState>,
) -> Result<axum::Json<NativeDecisionSnapshot>, ScratchError> {
    let root = decision_root(&app).await?;
    let mut sessions: Vec<_> = app
        .chat
        .sessions
        .read()
        .await
        .iter()
        .map(|(id, session)| (id.clone(), session.clone()))
        .collect();
    sessions.sort_by(|a, b| a.0.cmp(&b.0));
    let mut truncated = sessions.len() > 200;
    sessions.truncate(200);
    let mut questions = Vec::new();
    'sessions: for (chat_id, session) in sessions {
        let session = tokio::time::timeout(std::time::Duration::from_millis(100), session.lock())
            .await
            .map_err(|_| {
                ScratchError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "native decisions are busy; retry the snapshot".into(),
                )
            })?;
        for question in &session.pending_questions {
            if question.status != PendingQuestionStatus::Pending {
                continue;
            }
            if questions.len() == 100 {
                truncated = true;
                break 'sessions;
            }
            questions.push(NativeQuestionView {
                project_root: root.clone(),
                chat_id: chat_id.clone(),
                question: question.clone(),
            });
        }
    }
    Ok(axum::Json(NativeDecisionSnapshot {
        schema_version: 1,
        project_root: root,
        coverage: "loaded_native_sessions",
        truncated,
        questions,
    }))
}

#[derive(Deserialize)]
pub struct DecisionProjectQuery {
    project_root: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionAnswerRequest {
    project_root: String,
    asked_at_ms: u64,
    client_request_id: String,
    #[serde(default)]
    answers: Vec<refact_chat_api::QuestionAnswer>,
    #[serde(default)]
    response: Option<refact_chat_api::QuestionResponse>,
}

pub async fn handle_question_answer(
    State(app): State<AppState>,
    Path((chat_id, tool_call_id)): Path<(String, String)>,
    axum::Json(request): axum::Json<DecisionAnswerRequest>,
) -> Result<hyper::Response<hyper::Body>, ScratchError> {
    if request.response.is_some() && !request.answers.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "provide either answers or a structured response, not both".into(),
        ));
    }
    if decision_root(&app).await? != request.project_root {
        return Err(ScratchError::new(
            StatusCode::CONFLICT,
            "native decision project changed".into(),
        ));
    }
    if request.client_request_id.is_empty() || request.client_request_id.len() > 256 {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "invalid answer request identity".into(),
        ));
    }
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&chat_id)
        .cloned()
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, "native session is not loaded".into())
        })?;
    let payload_hash = crate::buddy::decision_receipts::digest(
        &serde_json::to_value(&request)
            .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e.to_string()))?,
    );
    {
        let mut session = session.lock().await;
        if crate::coordination::scope::session_source_root(&session).is_none_or(|source| {
            crate::coordination::roots::project_identity(&source)
                != crate::coordination::roots::project_identity(std::path::Path::new(
                    &request.project_root,
                ))
        }) {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                "Question source belongs to another project".into(),
            ));
        }
        if let Some(old) =
            crate::buddy::decision_receipts::find(&session, &request.client_request_id)
                .map_err(|e| ScratchError::new(StatusCode::CONFLICT, e))?
        {
            if old.payload_hash != payload_hash {
                return Err(ScratchError::new(
                    StatusCode::CONFLICT,
                    "Answer identity payload conflict".into(),
                ));
            }
            let old = crate::buddy::decision_receipts::visible(
                &session,
                old,
                super::buddy_native_approvals::native_engine_identity(),
            );
            return Ok(hyper::Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(hyper::Body::from(
                    serde_json::to_string(&old).unwrap_or_default(),
                ))
                .unwrap());
        }
        if !session.pending_questions.iter().any(|question| {
            question.tool_call_id == tool_call_id
                && question.asked_at_ms == request.asked_at_ms
                && question.status == PendingQuestionStatus::Pending
        }) {
            return Err(ScratchError::new(
                StatusCode::CONFLICT,
                "native question is no longer pending".into(),
            ));
        }
        let request_identity = crate::buddy::decision_receipts::digest(&serde_json::json!([
            session.chat_id,
            session.created_at,
            tool_call_id,
            request.asked_at_ms
        ]));
        crate::buddy::decision_receipts::reserve(
            &mut session,
            crate::buddy::decision_receipts::DecisionReceipt {
                source_user_id: None,
                schema_version: 1,
                project_root: request.project_root.clone(),
                engine_identity: super::buddy_native_approvals::native_engine_identity().into(),
                chat_id: chat_id.clone(),
                client_request_id: request.client_request_id.clone(),
                target_id: tool_call_id.clone(),
                accepted: None,
                request_identity,
                source_version: request.asked_at_ms.to_string(),
                payload_hash,
                status: "queued".into(),
                followup: "not_requested".into(),
                reason: None,
                message_id: None,
                decision_version: 0,
                version: 0,
            },
        )
        .map_err(|e| ScratchError::new(StatusCode::CONFLICT, e))?;
    }
    if let Err(error) =
        crate::chat::trajectories::flush_trajectory_for_session(app.clone(), session.clone()).await
    {
        crate::buddy::decision_receipts::transition(
            &mut *session.lock().await,
            &request.client_request_id,
            "unknown",
            Some("Decision admission durability is uncertain; no command was dispatched".into()),
        );
        let _ =
            crate::chat::trajectories::flush_trajectory_for_session(app.clone(), session.clone())
                .await;
        return Err(ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, error));
    }
    let command = if let Some(response) = request.response {
        serde_json::json!({
            "type": "submit_question_response", "tool_call_id": tool_call_id,
            "response": response, "client_request_id": request.client_request_id,
            "asked_at_ms": request.asked_at_ms,
        })
    } else {
        serde_json::json!({
            "type": "answer_questions", "tool_call_id": tool_call_id,
            "answers": request.answers, "client_request_id": request.client_request_id,
            "asked_at_ms": request.asked_at_ms,
        })
    };
    let receipt_request_id = request.client_request_id.clone();
    let result = crate::chat::handle_v1_chat_command(
        State(app.clone()),
        Path(chat_id),
        hyper::body::Bytes::from(command.to_string()),
    )
    .await;
    if result
        .as_ref()
        .map_or(true, |response| !response.status().is_success())
    {
        crate::buddy::decision_receipts::transition(
            &mut *session.lock().await,
            &receipt_request_id,
            "unknown",
            Some(
                "Native command admission was not acknowledged; not automatically replayed".into(),
            ),
        );
        let _ = crate::chat::trajectories::flush_trajectory_for_session(app, session).await;
    }
    result
}

pub async fn handle_question_state(
    State(app): State<AppState>,
    Path((chat_id, tool_call_id)): Path<(String, String)>,
    Query(query): Query<DecisionProjectQuery>,
) -> Result<axum::Json<NativeQuestionView>, ScratchError> {
    let root = decision_root(&app).await?;
    if root != query.project_root {
        return Err(ScratchError::new(
            StatusCode::CONFLICT,
            "native decision project changed".into(),
        ));
    }
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&chat_id)
        .cloned()
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, "native session is not loaded".into())
        })?;
    let session = session.lock().await;
    let question = session
        .pending_questions
        .iter()
        .find(|question| question.tool_call_id == tool_call_id)
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                "native decision is unavailable".into(),
            )
        })?;
    if question.status != PendingQuestionStatus::Pending && session.trajectory_dirty {
        return Err(ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "native decision is awaiting durable acknowledgment".into(),
        ));
    }
    Ok(axum::Json(NativeQuestionView {
        project_root: root,
        chat_id,
        question: question.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::ChatSession;
    use refact_chat_api::{PendingQuestion, QuestionAnswer};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn native_projection_preserves_identity_and_waits_for_durable_resolution() {
        let root = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.path().join("cache"),
            root.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        let mut session = ChatSession::new("native-chat".into());
        session.work_project_root = Some(root.path().canonicalize().unwrap());
        session
            .register_pending_questions(PendingQuestionSet {
                tool_call_id: "native-question".into(),
                asked_at_ms: 7,
                questions: vec![PendingQuestion {
                    id: "ok".into(),
                    question_type: "yes_no".into(),
                    text: "Continue?".into(),
                    options: None,
                }],
                status: PendingQuestionStatus::Pending,
                answers: vec![],
                resolution_id: None,
                response_protocol_version: refact_chat_api::QUESTION_RESPONSE_PROTOCOL_VERSION,
                response: None,
            })
            .unwrap();
        let session = Arc::new(Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert("native-chat".into(), session.clone());
        let projection = handle_current_decisions(State(app.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(projection.coverage, "loaded_native_sessions");
        assert_eq!(projection.questions.len(), 1);
        assert_eq!(projection.questions[0].chat_id, "native-chat");
        assert_eq!(
            projection.questions[0].question.tool_call_id,
            "native-question"
        );
        let invalid = handle_question_answer(
            State(app.clone()),
            Path(("native-chat".into(), "native-question".into())),
            axum::Json(DecisionAnswerRequest {
                project_root: "foreign".into(),
                asked_at_ms: 7,
                client_request_id: "request".into(),
                answers: vec![],
                response: None,
            }),
        )
        .await;
        assert!(invalid.is_err());
        assert!(session.lock().await.command_queue.is_empty());
        session
            .lock()
            .await
            .queue_processor_running
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let queued = handle_question_answer(
            State(app.clone()),
            Path(("native-chat".into(), "native-question".into())),
            axum::Json(DecisionAnswerRequest {
                project_root: root.path().to_string_lossy().into_owned(),
                asked_at_ms: 7,
                client_request_id: "native-forwarded".into(),
                answers: vec![QuestionAnswer {
                    id: "ok".into(),
                    answer: serde_json::json!(true),
                }],
                response: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(queued.status(), StatusCode::ACCEPTED);
        {
            let mut locked = session.lock().await;
            let command = locked.command_queue.pop_front().unwrap();
            assert_eq!(command.client_request_id, "native-forwarded");
            assert!(
                matches!(command.command, refact_chat_api::ChatCommand::AnswerQuestions { tool_call_id, answers, asked_at_ms }
                if tool_call_id == "native-question" && answers.len() == 1 && answers[0].answer == serde_json::json!(true) && asked_at_ms == Some(7))
            );
            assert_eq!(
                locked.pending_questions[0].status,
                PendingQuestionStatus::Pending
            );
            locked
                .queue_processor_running
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
        assert!(handle_question_state(
            State(app.clone()),
            Path(("native-chat".into(), "foreign".into())),
            Query(DecisionProjectQuery {
                project_root: root.path().to_string_lossy().into_owned()
            })
        )
        .await
        .is_err());
        session
            .lock()
            .await
            .answer_pending_questions(
                "native-question",
                vec![QuestionAnswer {
                    id: "ok".into(),
                    answer: serde_json::json!(true),
                }],
                "request".into(),
            )
            .unwrap();
        assert!(handle_question_state(
            State(app.clone()),
            Path(("native-chat".into(), "native-question".into())),
            Query(DecisionProjectQuery {
                project_root: root.path().to_string_lossy().into_owned()
            })
        )
        .await
        .is_err());
        session.lock().await.trajectory_dirty = false;
        let state = handle_question_state(
            State(app.clone()),
            Path(("native-chat".into(), "native-question".into())),
            Query(DecisionProjectQuery {
                project_root: root.path().to_string_lossy().into_owned(),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(state.question.status, PendingQuestionStatus::Answered);
        assert_eq!(state.question.resolution_id.as_deref(), Some("request"));
        assert!(handle_current_decisions(State(app))
            .await
            .unwrap()
            .0
            .questions
            .is_empty());
    }
}
