use super::{
    now_ms, request,
    tests::{fixture, Fixture},
    user,
};
use crate::chat::types::{ChatSession, QuestionAnswer};
use refact_chat_api::{FrozenRequestPrefix, QuestionKind, QuestionState};
use refact_work_core::{ApprovalDecision, ApprovalError, PendingApproval};
use std::sync::Arc;
use tokio::sync::Mutex;

async fn parent(fixture: &Fixture, approval_tool: bool) -> Arc<Mutex<ChatSession>> {
    let parent = fixture
        .app
        .chat
        .sessions
        .read()
        .await
        .get("parent")
        .cloned()
        .unwrap();
    let catalog = fixture
        .app
        .tool_registry
        .acquire_tool_catalog("agent", Some("test/model"), None)
        .await;
    let mut session = parent.lock().await;
    session.tool_catalog = Some(catalog);
    let mut names = vec!["shell"];
    if approval_tool {
        names.push("agent_approval");
    }
    session.thread.frozen_request_prefix = Some(FrozenRequestPrefix {
        schema_version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        system_prompt: Some("frozen parent".into()),
        tools_canonical: Some(serde_json::json!(names.into_iter().map(|name| {
            serde_json::json!({"type":"function","function":{"name":name,"parameters":{"type":"object"}}})
        }).collect::<Vec<_>>())),
    });
    drop(session);
    parent
}

async fn ready<F: std::future::Future<Output = Result<bool, String>>>(
    fixture: &Fixture,
    future: &mut std::pin::Pin<Box<F>>,
    parent: &Arc<Mutex<ChatSession>>,
) -> PendingApproval {
    let pending = fixture.waiting(future.as_mut()).await;
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            assert!(futures::poll!(future.as_mut()).is_pending());
            let session = parent.lock().await;
            if session
                .messages
                .iter()
                .any(|message| message.content.content_text_only().contains(&pending.id))
            {
                break;
            }
            drop(session);
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("approval was not routed");
    pending
}

async fn user_answer(
    fixture: &Fixture,
    parent: &Arc<Mutex<ChatSession>>,
    pending: &PendingApproval,
    answer: &str,
) {
    use crate::chat::types::{ChatCommand, CommandRequest};
    use std::sync::atomic::Ordering;
    let running = {
        let mut session = parent.lock().await;
        let set = session
            .pending_questions
            .iter()
            .find(|set| set.questions[0].id == pending.id)
            .unwrap()
            .clone();
        let request = CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "user-approval-answer".into(),
            priority: true,
            command: ChatCommand::AnswerQuestions {
                tool_call_id: set.tool_call_id,
                answers: vec![QuestionAnswer {
                    id: pending.id.clone(),
                    answer: serde_json::json!(answer),
                }],
                asked_at_ms: Some(set.asked_at_ms),
            },
        };
        session.command_queue.push_back(request);
        session.set_runtime_state(crate::chat::types::SessionState::Idle, None);
        session.queue_processor_running.clone()
    };
    running.store(true, Ordering::SeqCst);
    let worker = tokio::spawn(crate::chat::process_command_queue(
        fixture.app.clone(),
        parent.clone(),
        running,
    ));
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if fixture
                .app
                .questions()
                .pending_for_asker(&fixture.agent_id, chrono::Utc::now())
                .is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("committed user answer did not settle approval question");
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn approval_for_parent_without_agent_approval_escalates_to_user_immediately() {
    let fixture = fixture().await;
    let parent = parent(&fixture, false).await;
    let mut notifications = fixture
        .app
        .gcx
        .notification_events_tx
        .as_ref()
        .unwrap()
        .subscribe();
    let call = fixture.call("shell");
    let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs approval"));
    let pending = ready(&fixture, &mut future, &parent).await;
    let questions = fixture
        .app
        .questions()
        .pending_for_asker(&fixture.agent_id, chrono::Utc::now());
    assert_eq!(questions.len(), 1);
    assert_eq!(
        questions[0].answerer, "user",
        "approval must not wait on an incapable parent"
    );
    assert_eq!(questions[0].kind, QuestionKind::PermissionEscalation);
    let queue = fixture.app.attention().queue(chrono::Utc::now());
    assert_eq!(queue.waiting.len(), 1);
    assert_eq!(
        queue.waiting[0].priority,
        crate::attention::AttentionPriority::Blocking
    );
    assert!(queue.waiting[0].presented);
    assert!(matches!(notifications.try_recv().unwrap(),
        crate::http::routers::v1::sidebar::NotificationEvent::AskQuestions { chat_id, questions, .. }
        if chat_id == "parent" && questions[0].id == pending.id
    ));
    assert!(parent.lock().await.messages.iter().any(|message| {
        message
            .content
            .content_text_only()
            .contains("Escalated to the user because the parent session cannot answer")
    }));
    assert!(fixture
        .app
        .agents
        .get_any(&fixture.agent_id)
        .await
        .unwrap()
        .progress
        .unwrap()
        .contains("Waiting for user"));
    assert!(fixture.ccx.lock().await.approved_executions.is_empty());
    assert!(fixture
        .app
        .questions()
        .answer(
            &questions[0].id,
            "parent",
            "approve".into(),
            chrono::Utc::now(),
        )
        .is_err());
    user_answer(&fixture, &parent, &pending, "reject").await;
    assert!(!future.await.unwrap());
    assert!(!fixture.dir.path().join("approval.txt").exists());
}

#[tokio::test]
async fn custom_approve_text_and_skips_never_grant_command_permission() {
    for disposition in ["custom", "skipped"] {
        let fixture = fixture().await;
        let parent = parent(&fixture, false).await;
        let call = fixture.call("shell");
        let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs approval"));
        let pending = ready(&fixture, &mut future, &parent).await;
        let set = parent.lock().await.pending_questions[0].clone();
        let mut item = serde_json::json!({"id":pending.id,"disposition":disposition});
        if disposition == "custom" {
            item["answer"] = serde_json::json!("approve");
        }
        let response = crate::chat::handle_v1_chat_command(
            axum::extract::State(fixture.app.clone()),
            axum::extract::Path("parent".into()),
            hyper::body::Bytes::from(serde_json::json!({
                "type":"submit_question_response", "client_request_id":format!("nonconsent-{disposition}"),
                "tool_call_id":set.tool_call_id, "asked_at_ms":set.asked_at_ms,
                "response":{"answers":[item],"overall_comments":"not granting permission"}
            }).to_string()),
        ).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert!(!future.await.unwrap());
        let target = crate::agents::approval_target::ApprovalTarget::resolve(
            &fixture.app,
            &pending.child_work_id,
        )
        .await
        .unwrap();
        let child = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get(&target.chat_id)
            .cloned()
            .unwrap();
        assert!(child.lock().await.messages.iter().any(|message| {
            message
                .content
                .content_text_only()
                .contains("not granting permission")
                && message.content.content_text_only().contains(disposition)
        }));
        assert!(fixture.ccx.lock().await.approved_executions.is_empty());
        assert!(!fixture.dir.path().join("approval.txt").exists());
        assert!(matches!(
            fixture.app.agent_approvals.consume_user_once(
                "parent",
                &pending.id,
                pending.run_generation,
                &set.tool_call_id,
                now_ms(),
            ),
            Err(ApprovalError::NotApproved | ApprovalError::AlreadyDecided)
        ));
    }
}

async fn assert_stale_user_answer(
    fixture: &Fixture,
    parent: &Arc<Mutex<ChatSession>>,
    pending: &PendingApproval,
) {
    let contract = fixture
        .app
        .questions()
        .pending_for_asker(&fixture.agent_id, chrono::Utc::now())
        .pop()
        .unwrap();
    let set = parent.lock().await.pending_questions[0].clone();
    let response = crate::chat::handle_v1_chat_command(
        axum::extract::State(fixture.app.clone()),
        axum::extract::Path("parent".into()),
        hyper::body::Bytes::from(
            serde_json::json!({
                "client_request_id":"stale-user-answer", "type":"answer_questions",
                "tool_call_id":set.tool_call_id, "asked_at_ms":set.asked_at_ms,
                "answers":[{"id":pending.id,"answer":"approve"}],
            })
            .to_string(),
        ),
    )
    .await
    .unwrap();
    let settled = fixture.app.questions().get(&contract.id).unwrap();
    assert_eq!(
        settled.state,
        QuestionState::Withdrawn,
        "stale child approval must be withdrawn before recording the user answer"
    );
    assert_eq!(settled.answer, None);
    assert_eq!(settled.answered_at, None);
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        body["error"],
        "approval no longer pending: the child run ended or was replaced; nothing was approved"
    );
    assert_eq!(
        fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
        Err(ApprovalError::NotApproved)
    );
    assert_eq!(
        fixture.app.agent_approvals.consume_user_once(
            "parent",
            &pending.id,
            pending.run_generation,
            &set.tool_call_id,
            now_ms(),
        ),
        Err(ApprovalError::NotApproved)
    );
    assert!(fixture.ccx.lock().await.approved_executions.is_empty());
    assert!(!fixture.dir.path().join("approval.txt").exists());
    assert_eq!(fixture.app.attention().blocked_runs(&contract.id), 0);
    assert!(fixture
        .app
        .attention()
        .questions_for_waiter(&fixture.agent_id)
        .is_empty());
    let session = parent.lock().await;
    let set = &session.pending_questions[0];
    assert_eq!(
        set.status,
        crate::chat::types::PendingQuestionStatus::Cancelled
    );
    assert!(set.answers.is_empty());
    assert_eq!(set.resolution_id, None);
    assert!(session.messages.iter().any(|message| {
        message.extra.get("event").is_some_and(|event| {
            event["payload"]["state"] == "withdrawn"
                && event["payload"]["reason"]
                    .as_str()
                    .is_some_and(|reason| !reason.is_empty())
        })
    }));
}

#[tokio::test]
async fn user_answer_after_child_terminal_is_refused_before_settling() {
    let fixture = fixture().await;
    let parent = parent(&fixture, false).await;
    let call = fixture.call("shell");
    let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs approval"));
    let pending = ready(&fixture, &mut future, &parent).await;
    fixture
        .app
        .agents
        .mark_failed(&fixture.agent_id, "child ended".into())
        .await
        .unwrap();
    assert_stale_user_answer(&fixture, &parent, &pending).await;
    assert!(future.await.is_err());
    assert!(fixture.ccx.lock().await.approved_executions.is_empty());
    assert_eq!(
        fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
        Ok(ApprovalDecision::Cancelled)
    );
}

#[tokio::test]
async fn user_answer_after_card_replacement_is_refused() {
    let fixture = super::tests::board_fixture().await;
    let parent = parent(&fixture, false).await;
    let child_id = fixture.ccx.lock().await.chat_id.clone();
    let call = fixture.call("shell");
    let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "board approval"));
    let pending = ready(&fixture, &mut future, &parent).await;
    crate::tasks::storage::update_board_atomic(fixture.app.gcx.clone(), "approval-task", |board| {
        board.get_card_mut("card").unwrap().agent_chat_id = Some("replacement".into());
        Ok(())
    })
    .await
    .unwrap();
    assert_stale_user_answer(&fixture, &parent, &pending).await;
    assert!(future.await.is_err());
    assert!(fixture.ccx.lock().await.approved_executions.is_empty());
    crate::tasks::admission::end(&child_id).await;
}

#[tokio::test]
async fn user_answer_after_child_generation_changes_is_refused() {
    let fixture = fixture().await;
    let parent = parent(&fixture, false).await;
    let call = fixture.call("shell");
    let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs approval"));
    let pending = ready(&fixture, &mut future, &parent).await;
    let terminal = fixture
        .app
        .agents
        .mark_failed(&fixture.agent_id, "old run ended".into())
        .await
        .unwrap();
    fixture
        .app
        .agents
        .reopen(&fixture.agent_id, "parent", terminal.change_seq, false)
        .await
        .unwrap();
    let current = fixture
        .app
        .agents
        .mark_running(&fixture.agent_id, "replacement-child-chat".into())
        .await
        .unwrap();
    assert_ne!(current.run_generation, pending.run_generation);
    assert_stale_user_answer(&fixture, &parent, &pending).await;
    assert!(future.await.is_err());
    assert!(fixture.ccx.lock().await.approved_executions.is_empty());
}

#[tokio::test]
async fn approval_for_parent_with_agent_approval_still_goes_to_parent() {
    let fixture = fixture().await;
    let parent = parent(&fixture, true).await;
    let call = fixture.call("shell");
    let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs approval"));
    let pending = ready(&fixture, &mut future, &parent).await;
    let question = fixture.app.questions().get(&pending.id).unwrap();
    assert_eq!(question.answerer, "parent");
    assert!(fixture
        .app
        .attention()
        .queue(chrono::Utc::now())
        .waiting
        .is_empty());
    assert!(parent.lock().await.pending_questions.is_empty());
    assert!(fixture
        .app
        .agents
        .get_any(&fixture.agent_id)
        .await
        .unwrap()
        .progress
        .unwrap()
        .contains("Waiting for parent parent"));
    fixture
        .app
        .agent_approvals
        .decide(
            "parent",
            pending.run_generation,
            &pending.id,
            false,
            now_ms(),
        )
        .unwrap();
    assert!(!future.await.unwrap());
}

#[tokio::test]
async fn escalated_approval_is_never_auto_approved() {
    for answer in ["approve", "expire"] {
        let fixture = fixture().await;
        let parent = parent(&fixture, false).await;
        let call = fixture.call("shell");
        let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs approval"));
        let pending = ready(&fixture, &mut future, &parent).await;
        let question = fixture
            .app
            .questions()
            .pending_for_asker(&fixture.agent_id, chrono::Utc::now())
            .pop()
            .unwrap();
        assert_eq!(question.default_answer, None);
        assert_eq!(
            fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
            Err(ApprovalError::NotApproved)
        );
        assert!(futures::poll!(future.as_mut()).is_pending());
        assert!(fixture.ccx.lock().await.approved_executions.is_empty());
        assert!(!fixture.dir.path().join("approval.txt").exists());
        if answer == "approve" {
            user_answer(&fixture, &parent, &pending, answer).await;
            assert!(future.await.unwrap());
            assert!(fixture
                .ccx
                .lock()
                .await
                .approved_executions
                .contains_key(&call.id));
        } else {
            fixture
                .app
                .questions()
                .expire(question.expires_at.unwrap() + chrono::Duration::seconds(1));
            assert!(!future.await.unwrap());
            assert_eq!(
                fixture.app.questions().get(&question.id).unwrap().state,
                QuestionState::Expired
            );
            assert_eq!(
                fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
                Ok(ApprovalDecision::Rejected)
            );
            assert!(fixture.ccx.lock().await.approved_executions.is_empty());
        }
        assert!(!fixture.dir.path().join("approval.txt").exists());
    }
}

#[tokio::test]
async fn capability_check_uses_frozen_tools_then_active_catalog_and_command_allowlist() {
    let fixture = fixture().await;
    let parent = parent(&fixture, false).await;
    let mut session = parent.lock().await;
    assert!(!user::parent_can_answer(&session));
    session.thread.frozen_request_prefix = None;
    assert!(user::parent_can_answer(&session));
    session.active_command.allowed_tools = vec!["shell".into()];
    assert!(!user::parent_can_answer(&session));
    session.active_command.allowed_tools.clear();
    let mut catalog = session.tool_catalog.as_ref().unwrap().as_ref().clone();
    catalog
        .index
        .tools
        .retain(|tool| tool.name != "agent_approval");
    session.tool_catalog = Some(Arc::new(catalog));
    assert!(!user::parent_can_answer(&session));
    session.tool_catalog = None;
    assert!(user::parent_can_answer(&session));
}

#[tokio::test]
async fn board_child_approval_reaches_user_and_preserves_run_binding() {
    let fixture = super::tests::board_fixture().await;
    let parent = parent(&fixture, false).await;
    let child_id = fixture.ccx.lock().await.chat_id.clone();
    let call = fixture.call("shell");
    let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "board approval"));
    let pending = ready(&fixture, &mut future, &parent).await;
    assert_eq!(pending.child_work_id, fixture.agent_id);
    assert_eq!(pending.run_generation, fixture.generation);
    let child = fixture
        .app
        .chat
        .sessions
        .read()
        .await
        .get(&child_id)
        .cloned()
        .unwrap();
    assert!(child.lock().await.messages.iter().any(|message| {
        message
            .content
            .content_text_only()
            .contains("Waiting for user command approval")
    }));
    user_answer(&fixture, &parent, &pending, "approve").await;
    assert!(future.await.unwrap());
    assert!(fixture
        .ccx
        .lock()
        .await
        .approved_executions
        .contains_key(&call.id));
    assert!(!fixture.dir.path().join("approval.txt").exists());
    crate::tasks::admission::end(&child_id).await;
}

#[tokio::test]
async fn escalated_user_approval_cannot_override_parent_tool_denial() {
    let fixture = fixture().await;
    let parent = parent(&fixture, false).await;
    parent.lock().await.active_command.allowed_tools = vec!["cat".into()];
    let call = fixture.call("shell");
    let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs approval"));
    let pending = ready(&fixture, &mut future, &parent).await;
    user_answer(&fixture, &parent, &pending, "approve").await;
    assert!(future.await.unwrap_err().contains("not authorized"));
    assert!(fixture.ccx.lock().await.approved_executions.is_empty());
    assert!(!fixture.dir.path().join("approval.txt").exists());
}

#[tokio::test]
async fn escalated_user_answer_bypasses_busy_parent_without_interrupting_it() {
    let fixture = fixture().await;
    let parent = parent(&fixture, false).await;
    let call = fixture.call("shell");
    let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs approval"));
    let pending = ready(&fixture, &mut future, &parent).await;
    let set = parent.lock().await.pending_questions[0].clone();
    assert_eq!(
        parent.lock().await.runtime.state,
        crate::chat::types::SessionState::Generating
    );
    let body = serde_json::json!({
        "client_request_id":"busy-parent-answer", "type":"answer_questions",
        "tool_call_id":set.tool_call_id, "asked_at_ms":set.asked_at_ms,
        "answers":[{"id":pending.id,"answer":"approve"}],
    });
    for invalid in ["answer", "instance"] {
        let mut invalid_body = body.clone();
        invalid_body["client_request_id"] = serde_json::json!(format!("invalid-{invalid}"));
        if invalid == "answer" {
            invalid_body["answers"][0]["answer"] = "yes".into();
        } else {
            invalid_body["asked_at_ms"] = serde_json::json!(set.asked_at_ms + 1);
        }
        let response = crate::chat::handle_v1_chat_command(
            axum::extract::State(fixture.app.clone()),
            axum::extract::Path("parent".into()),
            hyper::body::Bytes::from(invalid_body.to_string()),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(
            fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
            Err(ApprovalError::NotApproved)
        );
    }
    let response = crate::chat::handle_v1_chat_command(
        axum::extract::State(fixture.app.clone()),
        axum::extract::Path("parent".into()),
        hyper::body::Bytes::from(body.to_string()),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        parent.lock().await.runtime.state,
        crate::chat::types::SessionState::Generating
    );
    assert!(parent.lock().await.command_queue.is_empty());
    assert!(future.await.unwrap());
    assert!(fixture
        .ccx
        .lock()
        .await
        .approved_executions
        .contains_key(&call.id));
    assert!(!fixture.dir.path().join("approval.txt").exists());
}
