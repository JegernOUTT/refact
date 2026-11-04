use std::sync::Arc;
use tokio::sync::Mutex;

use crate::app_state::AppState;
use crate::attention::{AttentionPriority, AttentionRequest, PromptDelivery, PromptDeliveryQuestion};
use crate::chat::types::{ChatSession, PendingQuestion, PendingQuestionSet, PendingQuestionStatus};
use refact_chat_api::{QuestionKind, QuestionSettlement};
use refact_work_core::PendingApproval;

const USER_APPROVAL_PREFIX: &str = "agent-approval:";

pub(crate) fn is_user_approval(tool_call_id: &str) -> bool {
    tool_call_id.starts_with(USER_APPROVAL_PREFIX)
}

pub(crate) async fn apply_user_answer(
    app: &AppState,
    session: &Arc<Mutex<ChatSession>>,
    tool_call_id: &str,
    answers: Vec<crate::chat::types::QuestionAnswer>,
    asked_at_ms: Option<u64>,
    resolution_id: &str,
    response: Option<refact_chat_api::QuestionResponse>,
) -> Result<(), String> {
    let changed = {
        let mut session = session.lock().await;
        match response {
            Some(response) => session.submit_question_response(
                tool_call_id,
                asked_at_ms.ok_or("question timestamp required")?,
                response,
                resolution_id.into(),
            )?,
            None => session.answer_pending_questions_at(
                tool_call_id,
                answers,
                resolution_id.into(),
                asked_at_ms,
            )?,
        }
    };
    if !changed {
        let mut session = session.lock().await;
        let snapshot = session.snapshot();
        session.emit(snapshot);
        return Ok(());
    }
    if let Err(error) = crate::chat::trajectories::try_save_trajectory_with_intent(
        app.clone(),
        session.clone(),
        crate::chat::types::TrajectoryCommitIntent::Required,
    )
    .await
    {
        session
            .lock()
            .await
            .rollback_answered_questions(tool_call_id, resolution_id, None);
        return Err(format!("failed to persist user approval answer: {error}"));
    }
    if let Err(error) = answer_from_user(app, session, tool_call_id, resolution_id).await {
        session
            .lock()
            .await
            .rollback_answered_questions(tool_call_id, resolution_id, None);
        return Err(error);
    }
    let mut session = session.lock().await;
    let snapshot = session.snapshot();
    session.emit(snapshot);
    Ok(())
}

pub(super) fn parent_can_answer(session: &ChatSession) -> bool {
    let allows = |name: &str| super::super::tool_authority::canonical(name) == "agent_approval";
    if !session.active_command.allowed_tools.is_empty()
        && !session
            .active_command
            .allowed_tools
            .iter()
            .any(|name| allows(name))
    {
        return false;
    }
    if let Some(tools) = session
        .thread
        .frozen_request_prefix
        .as_ref()
        .and_then(|prefix| prefix.tools_canonical.as_ref())
        .and_then(|tools| tools.as_array())
    {
        if !tools.iter().any(|tool| {
            tool.get("function")
                .and_then(|function| function.get("name"))
                .and_then(|name| name.as_str())
                .is_some_and(allows)
        }) {
            return false;
        }
    }
    session
        .tool_catalog
        .as_ref()
        .is_none_or(|catalog| catalog.index.tools.iter().any(|tool| allows(&tool.name)))
}

pub(super) async fn show_waiting(
    app: &AppState,
    target: &super::super::approval_target::ApprovalTarget,
    escalated: bool,
) -> Result<(), String> {
    let text = if escalated {
        "Waiting for user command approval (parent cannot answer: agent_approval unavailable)"
            .into()
    } else {
        format!("Waiting for parent {} command approval", target.parent_id)
    };
    if !target.work_id.starts_with("board:") {
        let record = app.agents.get_any(&target.work_id).await?;
        let updated = app
            .agents
            .update_progress(&target.work_id, text.clone(), record.step_count)
            .await?;
        super::super::spawn::emit_background_agent_update(app.clone(), &updated).await;
    }
    let child = app
        .chat
        .sessions
        .read()
        .await
        .get(&target.chat_id)
        .cloned()
        .ok_or("child session unavailable for approval wait")?;
    child
        .lock()
        .await
        .add_message(crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            crate::agents::push::NOTICE_APPROVAL,
            serde_json::json!({"waiting_for": if escalated { "user" } else { &target.parent_id }}),
            text,
        ));
    Ok(())
}

pub(super) async fn ask(
    app: &AppState,
    pending: &PendingApproval,
    asker: &str,
    notice: &crate::call_validation::ChatMessage,
) -> Result<String, String> {
    let scope = crate::attention::scope_for_chat(app, &pending.parent_chat_id).await;
    let text = notice.content.content_text_only();
    let tool_call_id = format!("{USER_APPROVAL_PREFIX}{}", pending.id);
    let options = Some(vec!["approve".into(), "reject".into()]);
    let admission = app
        .attention()
        .join(
            AttentionRequest::new(
                scope,
                QuestionKind::PermissionEscalation,
                asker,
                "user",
                &text,
            )
            .with_options(options.clone())
            .with_priority(AttentionPriority::Blocking)
            .with_delivery(Some(PromptDelivery {
                chat_id: pending.parent_chat_id.clone(),
                tool_call_id: tool_call_id.clone(),
                asked_at_ms: pending.created_at,
                question: PromptDeliveryQuestion {
                    id: pending.id.clone(),
                    question_type: "single_select".into(),
                    text: text.clone(),
                    options: options.clone(),
                },
            })),
            asker,
            chrono::Utc::now(),
        )
        .map_err(|error| format!("user approval escalation: {error}"))?;
    let result = async {
        let parent = app
            .chat
            .sessions
            .read()
            .await
            .get(&pending.parent_chat_id)
            .cloned()
            .ok_or("parent session unavailable for user approval")?;
        parent
            .lock()
            .await
            .register_pending_questions(PendingQuestionSet {
                tool_call_id: tool_call_id.clone(),
                questions: vec![PendingQuestion {
                    id: pending.id.clone(),
                    question_type: "single_select".into(),
                    text,
                    options,
                }],
                asked_at_ms: pending.created_at,
                status: PendingQuestionStatus::Pending,
                answers: vec![],
                resolution_id: None,
                response_protocol_version: refact_chat_api::QUESTION_RESPONSE_PROTOCOL_VERSION,
                response: None,
            })?;
        if let Err(error) = crate::chat::trajectories::try_save_trajectory_with_intent(
            app.clone(),
            parent.clone(),
            crate::chat::types::TrajectoryCommitIntent::Required,
        )
        .await
        {
            parent
                .lock()
                .await
                .rollback_pending_question_registration(&tool_call_id);
            return Err(format!("failed to persist user approval question: {error}"));
        }
        let mut parent = parent.lock().await;
        let snapshot = parent.snapshot();
        parent.emit(snapshot);
        Ok(())
    }
    .await;
    if let Err(error) = result {
        let _ = app
            .attention()
            .leave(&admission.question_id, asker, chrono::Utc::now());
        return Err(error);
    }
    crate::attention_delivery::deliver_due(app, chrono::Utc::now()).await;
    Ok(admission.question_id)
}

pub(crate) async fn answer_from_user(
    app: &AppState,
    session: &Arc<Mutex<ChatSession>>,
    tool_call_id: &str,
    resolution_id: &str,
) -> Result<bool, String> {
    let Some(id) = tool_call_id.strip_prefix(USER_APPROVAL_PREFIX) else {
        return Ok(false);
    };
    let (chat_id, question, answer, explicit_consent) = {
        let session = session.lock().await;
        let set = session
            .pending_questions
            .iter()
            .find(|set| {
                set.tool_call_id == tool_call_id
                    && set.status == PendingQuestionStatus::Answered
                    && set.resolution_id.as_deref() == Some(resolution_id)
            })
            .ok_or("user approval answer is not committed")?;
        if set.questions.len() != 1 || set.answers.len() != 1 || set.questions[0].id != id {
            return Err("user approval question binding changed".into());
        }
        (
            session.chat_id.clone(),
            set.questions[0].clone(),
            set.answers[0].answer.clone(),
            set.response.as_ref().is_none_or(|response| {
                matches!(
                    response.answers.first().map(|item| &item.value),
                    Some(refact_chat_api::QuestionResponseValue::Answered { .. })
                )
            }),
        )
    };
    let pending = app
        .agent_approvals
        .get(&chat_id, id, super::now_ms())
        .map_err(|error| format!("user approval request: {error:?}"))?;
    let contract = app
        .questions()
        .pending_for_asker(&pending.child_work_id, chrono::Utc::now())
        .into_iter()
        .find(|contract| {
            contract.kind == QuestionKind::PermissionEscalation
                && contract.answerer == "user"
                && contract.text == question.text
                && contract.options == question.options
        })
        .ok_or("user approval question is no longer pending")?;
    if !explicit_consent {
        let response = {
            let session = session.lock().await;
            session
                .pending_questions
                .iter()
                .find(|set| set.tool_call_id == tool_call_id)
                .and_then(|set| set.response.clone())
                .ok_or("structured approval response unavailable")?
        };
        let notice = crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            crate::agents::push::NOTICE_APPROVAL,
            serde_json::json!({"request_id":id,"response":response,"permission_granted":false}),
            format!("User responded without granting command permission. Their response follows; decide how to proceed without running the unapproved command: {}", serde_json::to_string(&response).map_err(|error| error.to_string())?),
        );
        let target =
            super::super::approval_target::ApprovalTarget::resolve(app, &pending.child_work_id)
                .await?;
        let child = app.chat.sessions.read().await.get(&target.chat_id).cloned();
        if let Some(child) = child {
            child.lock().await.add_message(notice.clone());
            crate::chat::trajectories::try_save_trajectory_with_intent(
                app.clone(),
                child,
                crate::chat::types::TrajectoryCommitIntent::Required,
            )
            .await
            .map_err(|error| format!("failed to persist non-consent context: {error}"))?;
        }
        session.lock().await.add_message(notice);
        crate::chat::trajectories::try_save_trajectory_with_intent(
            app.clone(),
            session.clone(),
            crate::chat::types::TrajectoryCommitIntent::Required,
        )
        .await
        .map_err(|error| format!("failed to persist non-consent notice: {error}"))?;
        app.questions()
            .withdraw(&contract.id, &contract.asker, chrono::Utc::now())
            .map_err(|error| format!("user approval withdrawal: {error}"))?;
        return Ok(true);
    }
    let answer = answer
        .as_str()
        .filter(|answer| matches!(*answer, "approve" | "reject"))
        .ok_or("user approval requires approve or reject")?;
    let target =
        super::super::approval_target::ApprovalTarget::resolve(app, &pending.child_work_id)
            .await
            .and_then(|target| target.check(&pending));
    if let Err(reason) = target {
        let now = chrono::Utc::now();
        let refusal =
            "approval no longer pending: the child run ended or was replaced; nothing was approved";
        app.questions()
            .withdraw(&contract.id, &contract.asker, now)
            .map_err(|error| format!("user approval withdrawal: {error}"))?;
        let _ = app
            .attention()
            .leave(&contract.id, &pending.child_work_id, now);
        {
            let mut session = session.lock().await;
            if let Some(set) = session.pending_questions.iter_mut().find(|set| {
                set.tool_call_id == tool_call_id
                    && set.resolution_id.as_deref() == Some(resolution_id)
            }) {
                set.status = PendingQuestionStatus::Cancelled;
                set.answers.clear();
                set.response = None;
                set.resolution_id = None;
                session.increment_version();
            }
            session.add_message(crate::chat::internal_roles::event(
                crate::chat::internal_roles::EventSubkind::SystemNotice,
                crate::agents::push::NOTICE_APPROVAL,
                serde_json::json!({"request_id": id, "state": "withdrawn", "reason": reason}),
                refusal,
            ));
        }
        if let Err(error) = crate::chat::trajectories::try_save_trajectory_with_intent(
            app.clone(),
            session.clone(),
            crate::chat::types::TrajectoryCommitIntent::Required,
        )
        .await
        {
            tracing::warn!(%error, request_id = id, "failed to persist stale approval withdrawal");
        }
        let mut session = session.lock().await;
        let snapshot = session.snapshot();
        session.emit(snapshot);
        return Err(refusal.into());
    }
    app.questions()
        .answer(&contract.id, "user", answer.into(), chrono::Utc::now())
        .map_err(|error| format!("user approval answer: {error}"))?;
    Ok(true)
}

pub(super) async fn wait(
    app: &AppState,
    pending: &PendingApproval,
    question_id: &str,
) -> Result<(), String> {
    let decision = tokio::select! {
        question = app.questions().wait(question_id, chrono::Utc::now()) => {
            matches!(question, Ok(QuestionSettlement::Answered(ref answer)) if answer == "approve")
        }
        _ = app.agent_approvals.wait(&pending.id, super::now_ms()) => return Ok(()),
    };
    let context = crate::at_commands::at_commands::AtCommandsContext::new_from_app(
        app.clone(),
        4096,
        1,
        false,
        vec![],
        pending.parent_chat_id.clone(),
        None,
        String::new(),
        None,
        None,
    )
    .await;
    let wrapper_id = format!("{USER_APPROVAL_PREFIX}{}", pending.id);
    if decision {
        app.agent_approvals
            .grant_user_once(
                &pending.parent_chat_id,
                &pending.id,
                pending.run_generation,
                &wrapper_id,
                super::now_ms(),
            )
            .map_err(|error| format!("user approval grant: {error:?}"))?;
    }
    let mut tool = crate::tools::tool_agent_approval::ToolAgentApproval {
        config_path: String::new(),
    };
    use crate::tools::tools_description::Tool;
    tool.tool_execute(
        Arc::new(Mutex::new(context)),
        &wrapper_id,
        &std::collections::HashMap::from([
            (
                "action".into(),
                serde_json::json!(if decision { "approve" } else { "reject" }),
            ),
            ("request_id".into(), serde_json::json!(pending.id)),
        ]),
    )
    .await?;
    Ok(())
}
