use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use refact_chat_api::{ChatCommand, PendingQuestionStatus, QuestionAnswer, QuestionState};
use tokio::sync::Mutex;

use crate::app_state::AppState;
use crate::chat::types::ChatSession;
use crate::http::routers::v1::sidebar::NotificationEvent;
use crate::tools::tools_description::Tool;

const WAIT_BOUND: Duration = Duration::from_secs(30);

struct Fixture {
    app: AppState,
    sessions: [Arc<Mutex<ChatSession>>; 2],
    notifications: tokio::sync::broadcast::Receiver<NotificationEvent>,
    workspace: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let notifications = gcx.notification_events_tx.as_ref().unwrap().subscribe();
        let app = AppState::from_gcx(gcx).await;
        let sessions = ["chat-a", "chat-b"].map(|chat_id| {
            let mut session = ChatSession::new(chat_id.into());
            session.set_title("Attention leave test".into(), true);
            Arc::new(Mutex::new(session))
        });
        for session in &sessions {
            let chat_id = session.lock().await.chat_id.clone();
            app.chat
                .sessions
                .write()
                .await
                .insert(chat_id, session.clone());
        }
        Self {
            app,
            sessions,
            notifications,
            workspace,
        }
    }

    async fn ask(&self, index: usize, default: Option<&str>) -> String {
        let chat_id = self.sessions[index].lock().await.chat_id.clone();
        let context = crate::at_commands::at_commands::AtCommandsContext::new_from_app(
            self.app.clone(),
            4096,
            20,
            false,
            vec![],
            chat_id.clone(),
            None,
            "test-model".into(),
            None,
            None,
        )
        .await;
        let mut question = serde_json::json!({
            "id": "q1",
            "type": "free_text",
            "text": "Should we continue?",
        });
        if let Some(default) = default {
            question["priority"] = serde_json::json!("low");
            question["default"] = serde_json::json!(default);
        }
        let mut tool = crate::tools::tool_ask_questions::ToolAskQuestions {
            config_path: String::new(),
        };
        tool.tool_execute(
            Arc::new(Mutex::new(context)),
            &"ask-call".to_string(),
            &HashMap::from([("questions".into(), serde_json::json!([question]))]),
        )
        .await
        .unwrap();
        let ids = self
            .app
            .attention()
            .questions_for_waiter(&format!("{chat_id}:ask-call"));
        assert_eq!(ids.len(), 1);
        ids[0].clone()
    }

    async fn command(&self, index: usize, command: ChatCommand, request_id: &str) {
        let chat_id = self.sessions[index].lock().await.chat_id.clone();
        crate::app_state::push_chat_command_with_request_id(
            self.app.gcx.clone(),
            &chat_id,
            command,
            request_id.into(),
        )
        .await
        .unwrap();
    }

    async fn barrier(&self, index: usize) {
        self.command(
            index,
            ChatCommand::SetParams {
                patch: serde_json::json!({"temperature": 0.25}),
            },
            "leave-test-barrier",
        )
        .await;
        tokio::time::timeout(WAIT_BOUND, async {
            loop {
                if self.sessions[index].lock().await.thread.temperature == Some(0.25) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queue did not reach the command after the question resolution");
    }

    async fn cancel(&self, index: usize) {
        self.command(index, ChatCommand::Abort {}, "cancel-questions")
            .await;
        self.barrier(index).await;
        assert_eq!(
            self.sessions[index].lock().await.pending_questions[0].status,
            PendingQuestionStatus::Cancelled
        );
    }

    fn after_window(&self, question_id: &str) -> DateTime<Utc> {
        self.app.questions().get(question_id).unwrap().asked_at
            + ChronoDuration::milliseconds(self.app.attention().limits().batch_window_ms as i64 + 1)
    }

    async fn close(&self) {
        for session in &self.sessions {
            let mut session = session.lock().await;
            session.closed = true;
            session.queue_notify.notify_one();
        }
        tokio::time::timeout(WAIT_BOUND, async {
            for session in &self.sessions {
                loop {
                    if !session
                        .lock()
                        .await
                        .queue_processor_running
                        .load(Ordering::SeqCst)
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            }
        })
        .await
        .expect("question test queue did not stop");
    }
}

#[tokio::test]
async fn a_cancelled_waiter_leaves_and_an_orphaned_shared_prompt_is_not_delivered() {
    let mut fixture = Fixture::new().await;
    let id = fixture.ask(0, None).await;
    assert_eq!(fixture.ask(1, None).await, id);
    assert_eq!(fixture.app.attention().blocked_runs(&id), 2);

    fixture.cancel(0).await;
    fixture.cancel(1).await;

    assert_eq!(fixture.app.attention().blocked_runs(&id), 0);
    assert_eq!(
        fixture.app.questions().get(&id).unwrap().state,
        QuestionState::Cancelled
    );
    assert_eq!(
        crate::attention_delivery::deliver_due(&fixture.app, fixture.after_window(&id)).await,
        0
    );
    assert!(fixture.notifications.try_recv().is_err());
    fixture.close().await;
}

#[tokio::test]
async fn a_remaining_waiter_still_receives_the_shared_prompt() {
    let mut fixture = Fixture::new().await;
    let id = fixture.ask(0, None).await;
    assert_eq!(fixture.ask(1, None).await, id);

    fixture.cancel(0).await;

    assert_eq!(fixture.app.attention().blocked_runs(&id), 1);
    assert_eq!(
        fixture.app.questions().get(&id).unwrap().state,
        QuestionState::Pending
    );
    assert_eq!(
        crate::attention_delivery::deliver_due(&fixture.app, fixture.after_window(&id)).await,
        1
    );
    assert!(matches!(
        fixture.notifications.try_recv().unwrap(),
        NotificationEvent::AskQuestions { chat_id, tool_call_id, questions, .. }
            if chat_id == "chat-b" && tool_call_id == "ask-call" && questions[0].id == "q1"
    ));
    assert!(
        fixture.notifications.try_recv().is_err(),
        "a cancelled chat must not be notified"
    );
    fixture.close().await;
}

#[tokio::test]
async fn a_user_answer_leaves_the_budget_and_never_becomes_a_timeout_default() {
    let fixture = Fixture::new().await;
    let id = fixture.ask(0, Some("no")).await;
    let contract = fixture.app.questions().get(&id).unwrap();
    let due = contract.expires_at.unwrap() + ChronoDuration::milliseconds(1);
    let asked_at_ms = {
        let session = fixture.sessions[0].lock().await;
        session.user_interrupt_flag.store(true, Ordering::SeqCst);
        session.pending_questions[0].asked_at_ms
    };
    fixture
        .command(
            0,
            ChatCommand::AnswerQuestions {
                tool_call_id: "ask-call".into(),
                answers: vec![QuestionAnswer {
                    id: "q1".into(),
                    answer: serde_json::json!("yes"),
                }],
                asked_at_ms: Some(asked_at_ms),
            },
            "user-answer-request",
        )
        .await;
    fixture.barrier(0).await;

    {
        let session = fixture.sessions[0].lock().await;
        assert_eq!(
            session.pending_questions[0].status,
            PendingQuestionStatus::Answered
        );
        assert_eq!(
            session.pending_questions[0].answers[0].answer,
            serde_json::json!("yes")
        );
    }
    assert_ne!(
        fixture.app.questions().get(&id).unwrap().state,
        QuestionState::Pending,
        "a user-answered question must no longer be pending in the question service"
    );
    assert_eq!(
        crate::attention_delivery::apply_timeout_defaults(&fixture.app, due).await,
        0
    );
    assert!(fixture
        .app
        .attention()
        .take_expired_defaults(due)
        .is_empty());
    assert_ne!(
        fixture.app.questions().get(&id).unwrap().state,
        QuestionState::Defaulted
    );
    assert_eq!(fixture.app.attention().blocked_runs(&id), 0);
    fixture.close().await;
}

#[tokio::test]
async fn a_new_user_message_also_releases_its_cancelled_questions() {
    let mut fixture = Fixture::new().await;
    let id = fixture.ask(0, None).await;
    fixture.sessions[0]
        .lock()
        .await
        .user_interrupt_flag
        .store(true, Ordering::SeqCst);
    fixture
        .command(
            0,
            ChatCommand::UserMessage {
                content: serde_json::json!("Never mind the question"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: true,
                client_message_id: None,
            },
            "new-user-message",
        )
        .await;
    fixture.barrier(0).await;

    assert_eq!(
        fixture.sessions[0].lock().await.pending_questions[0].status,
        PendingQuestionStatus::Cancelled
    );
    assert_eq!(fixture.app.attention().blocked_runs(&id), 0);
    assert_eq!(
        crate::attention_delivery::deliver_due(&fixture.app, fixture.after_window(&id)).await,
        0
    );
    assert!(fixture.notifications.try_recv().is_err());
    fixture.close().await;
}

#[tokio::test]
async fn a_rolled_back_user_answer_keeps_its_attention_waiter() {
    let fixture = Fixture::new().await;
    let id = fixture.ask(0, Some("no")).await;
    let trajectories = fixture.workspace.path().join(".refact/trajectories");
    std::fs::remove_dir_all(&trajectories).unwrap();
    std::fs::write(&trajectories, "not a directory").unwrap();
    let (asked_at_ms, mut events) = {
        let session = fixture.sessions[0].lock().await;
        (
            session.pending_questions[0].asked_at_ms,
            session.subscribe(),
        )
    };
    fixture
        .command(
            0,
            ChatCommand::AnswerQuestions {
                tool_call_id: "ask-call".into(),
                answers: vec![QuestionAnswer {
                    id: "q1".into(),
                    answer: serde_json::json!("yes"),
                }],
                asked_at_ms: Some(asked_at_ms),
            },
            "failed-user-answer",
        )
        .await;
    tokio::time::timeout(WAIT_BOUND, async {
        loop {
            let event: serde_json::Value =
                serde_json::from_str(&events.recv().await.unwrap()).unwrap();
            if event["client_request_id"] == "failed-user-answer" && event["accepted"] == false {
                assert!(event["result"]["error"]
                    .as_str()
                    .unwrap()
                    .contains("failed to persist question answers"));
                break;
            }
        }
    })
    .await
    .expect("failed persistence did not reject the answer");

    assert_eq!(
        fixture.sessions[0].lock().await.pending_questions[0].status,
        PendingQuestionStatus::Pending
    );
    assert_eq!(
        fixture.app.questions().get(&id).unwrap().state,
        QuestionState::Pending
    );
    assert_eq!(fixture.app.attention().blocked_runs(&id), 1);
    fixture.close().await;
}
