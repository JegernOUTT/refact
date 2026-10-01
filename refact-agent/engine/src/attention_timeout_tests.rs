use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use refact_chat_api::{ChatCommand, PendingQuestionStatus, QuestionAnswer, QuestionKind};

use crate::attention::{
    AttentionPriority, AttentionRequest, AttentionScope, PromptDelivery, PromptDeliveryQuestion,
};

use crate::tools::tools_description::Tool;

fn timestamp(seconds: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(seconds, 0).unwrap()
}

fn answer_request(default: &str, delivery: PromptDelivery) -> AttentionRequest {
    AttentionRequest::new(
        AttentionScope::new("/project", "AllowToSendAnywhere"),
        QuestionKind::UserInput,
        "chat:timeout-chat",
        "user",
        "Should we continue?",
    )
    .with_priority(AttentionPriority::Low)
    .with_default_answer(Some(default.to_string()))
    .with_delivery(Some(delivery))
}

fn delivery(chat_id: &str, tool_call_id: &str, asked_at_ms: u64) -> PromptDelivery {
    PromptDelivery {
        chat_id: chat_id.into(),
        tool_call_id: tool_call_id.into(),
        asked_at_ms,
        question: PromptDeliveryQuestion {
            id: "q1".into(),
            question_type: "free_text".into(),
            text: "Should we continue?".into(),
            options: None,
        },
    }
}

async fn fixture() -> (
    crate::app_state::AppState,
    Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    tempfile::TempDir,
) {
    let workspace = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().to_path_buf()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let session = Arc::new(tokio::sync::Mutex::new(
        crate::chat::types::ChatSession::new("timeout-chat".into()),
    ));
    app.chat
        .sessions
        .write()
        .await
        .insert("timeout-chat".into(), session.clone());
    (app, session, workspace)
}

async fn ask(app: &crate::app_state::AppState, default: &str) -> String {
    let chat_id = "timeout-chat";
    let tool_call_id = "ask-call";
    let context = crate::at_commands::at_commands::AtCommandsContext::new_from_app(
        app.clone(),
        4096,
        20,
        false,
        vec![],
        chat_id.to_string(),
        None,
        "test-model".into(),
        None,
        None,
    )
    .await;
    let mut tool = crate::tools::tool_ask_questions::ToolAskQuestions {
        config_path: String::new(),
    };
    let args = HashMap::from([(
        "questions".to_string(),
        serde_json::json!([{
            "id":"q1",
            "type":"free_text",
            "text":"Should we continue?",
            "priority":"low",
            "default": default,
        }]),
    )]);
    tool.tool_execute(
        Arc::new(tokio::sync::Mutex::new(context)),
        &tool_call_id.to_string(),
        &args,
    )
    .await
    .unwrap();
    tool_call_id.to_string()
}

async fn wait_until_answered(session: &Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let session = session.lock().await;
            if session.pending_questions[0].status == PendingQuestionStatus::Answered {
                return;
            }
            drop(session);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_expired_low_priority_question_is_answered_with_its_default_once() {
    let (app, session, _workspace) = fixture().await;
    let _tool_call_id = ask(&app, "no").await;
    let question_id = app
        .attention()
        .queue(Utc::now())
        .waiting
        .first()
        .unwrap()
        .question_id
        .clone();
    let asked_at_ms = session.lock().await.pending_questions[0].asked_at_ms;
    let due = app
        .questions()
        .get(&question_id)
        .unwrap()
        .expires_at
        .unwrap()
        + Duration::milliseconds(1);
    assert_eq!(crate::attention_delivery::deliver_due(&app, due).await, 0);
    assert_eq!(
        crate::attention_delivery::apply_timeout_defaults(&app, due).await,
        1
    );
    assert_eq!(
        crate::attention_delivery::apply_timeout_defaults(&app, due).await,
        0
    );
    assert_eq!(
        crate::attention_delivery::apply_timeout_defaults(&app, due).await,
        0
    );
    assert_eq!(
        crate::attention_delivery::apply_timeout_defaults(&app, due + Duration::seconds(1)).await,
        0
    );
    wait_until_answered(&session).await;

    let session = session.lock().await;
    let set = &session.pending_questions[0];
    assert_eq!(set.status, PendingQuestionStatus::Answered);
    assert_eq!(set.answers[0].answer, serde_json::json!("no"));
    assert_eq!(
        set.resolution_id.as_deref(),
        Some(format!("attention-timeout:{question_id}").as_str())
    );
    assert_eq!(set.asked_at_ms, asked_at_ms);
    assert_eq!(session.runtime.state, refact_chat_api::SessionState::Idle);
    assert!(session.messages.iter().any(|message| {
        message.role == "event"
            && message.content.content_text_only()
                == "No answer before the deadline; applied the documented default: no"
    }));
}

#[tokio::test]
async fn a_user_answer_before_expiry_wins_and_the_default_is_never_applied() {
    let (app, session, _workspace) = fixture().await;
    let tool_call_id = ask(&app, "no").await;
    let asked_at_ms = session.lock().await.pending_questions[0].asked_at_ms;
    crate::app_state::push_chat_command_with_request_id(
        app.gcx.clone(),
        "timeout-chat",
        ChatCommand::AnswerQuestions {
            tool_call_id,
            answers: vec![QuestionAnswer {
                id: "q1".into(),
                answer: serde_json::json!("yes"),
            }],
            asked_at_ms: Some(asked_at_ms),
        },
        "user-answer-request".into(),
    )
    .await
    .unwrap();
    wait_until_answered(&session).await;
    let due = app
        .questions()
        .snapshot()
        .into_iter()
        .find(|question| {
            question.asker == "chat:timeout-chat"
                && question.default_answer.as_deref() == Some("no")
        })
        .and_then(|question| question.expires_at)
        .unwrap()
        + Duration::milliseconds(1);
    assert_eq!(crate::attention_delivery::deliver_due(&app, due).await, 0);
    assert_eq!(
        crate::attention_delivery::apply_timeout_defaults(&app, due).await,
        0
    );
    assert_eq!(
        crate::attention_delivery::apply_timeout_defaults(&app, due).await,
        0
    );

    let session = session.lock().await;
    let set = &session.pending_questions[0];
    assert_eq!(set.answers[0].answer, serde_json::json!("yes"));
    assert_eq!(set.resolution_id.as_deref(), Some("user-answer-request"));
    assert!(!session.messages.iter().any(|message| {
        message
            .content
            .content_text_only()
            .contains("applied the documented default")
    }));
}

#[tokio::test]
async fn a_blocking_question_can_never_reach_the_default_path() {
    let (app, _, _workspace) = fixture().await;
    let now = timestamp(1_700_000_000);
    let admission = app.attention().join(
        AttentionRequest::new(
            AttentionScope::new("/project", "AllowToSendAnywhere"),
            QuestionKind::UserInput,
            "agent",
            "user",
            "Should we continue?",
        )
        .with_priority(AttentionPriority::Blocking)
        .with_default_answer(Some("no".into())),
        "blocked",
        now,
    );
    assert!(admission.is_err());

    let id = app
        .attention()
        .join(
            AttentionRequest::new(
                AttentionScope::new("/project", "AllowToSendAnywhere"),
                QuestionKind::UserInput,
                "agent",
                "user",
                "Blocking?",
            )
            .with_priority(AttentionPriority::Blocking)
            .with_delivery(Some(delivery(
                "timeout-chat",
                "blocking-call",
                1_700_000_000_000,
            ))),
            "blocking-waiter",
            now,
        )
        .unwrap()
        .question_id;
    app.questions().expire(timestamp(2_000_000_000));
    assert!(app
        .attention()
        .take_expired_defaults(timestamp(2_000_000_000))
        .is_empty());
    assert_eq!(app.questions().get(&id).unwrap().default_answer, None);
}

#[tokio::test]
async fn expired_defaults_are_claimed_once() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let now = timestamp(1_700_000_000);
    let admission = app
        .attention()
        .join(
            answer_request(
                "no",
                delivery("timeout-chat", "ask", now.timestamp_millis() as u64),
            ),
            "timeout-chat:ask",
            now,
        )
        .unwrap();
    let expires_at = app
        .questions()
        .get(&admission.question_id)
        .unwrap()
        .expires_at
        .unwrap();
    let due = expires_at + Duration::milliseconds(1);
    let first = app.attention().take_expired_defaults(due);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].question_id, admission.question_id);
    assert!(app.attention().take_expired_defaults(due).is_empty());
}
