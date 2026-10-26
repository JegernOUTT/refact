use std::sync::atomic::Ordering;
use std::sync::Arc;

use refact_runtime_api::{ChatSessionFacade, SessionState};
use serde_json::json;
use tokio::sync::Mutex;

use crate::app_state::EngineChatSessionFacade;
use crate::chat::types::{ChatSession, TaskMeta};
use crate::tasks::storage;
use crate::tasks::types::{CardOutcome, OutcomeFragment};
use crate::tools::task_tool_helpers::CANCEL_AGENT_STOP_TIMEOUT;
use crate::tools::tool_agent_lifecycle::tests::{
    args, output_text, planner_ccx, test_card, write_task, MockChatFacade,
};
use crate::tools::tool_agent_lifecycle::ToolCancelAgent;
use crate::tools::tools_description::Tool;

async fn executing_session(
    gcx: &Arc<crate::global_context::GlobalContext>,
) -> Arc<Mutex<ChatSession>> {
    let mut session = ChatSession::new("agent-chat-1".into());
    session.thread.task_meta = Some(TaskMeta {
        task_id: "task-1".into(),
        role: "agents".into(),
        agent_id: Some("agent-1".into()),
        card_id: Some("T-39".into()),
        planner_chat_id: Some("planner-chat".into()),
    });
    session.start_stream().unwrap();
    session.set_runtime_state(SessionState::ExecutingTools, None);
    session
        .queue_processor_running
        .store(true, Ordering::SeqCst);
    let session = Arc::new(Mutex::new(session));
    gcx.chat_sessions
        .write()
        .await
        .insert("agent-chat-1".into(), session.clone());
    session
}

#[tokio::test]
async fn cancel_agent_aborts_an_executing_session_promptly() {
    let root = tempfile::tempdir().unwrap();
    let gcx = write_task(
        root.path(),
        test_card("doing", Some("agent-chat-1".into()), None),
    )
    .await;
    let session = executing_session(&gcx).await;
    let facade = Arc::new(EngineChatSessionFacade::new(gcx.clone()));
    let ccx = planner_ccx(gcx.clone(), facade, "planner").await;

    let output = output_text(
        tokio::time::timeout(
            CANCEL_AGENT_STOP_TIMEOUT + std::time::Duration::from_secs(1),
            ToolCancelAgent::new().tool_execute(
                ccx,
                &"cancel".into(),
                &args(&[("card_id", json!("T-39")), ("reason", json!("stop now"))]),
            ),
        )
        .await
        .expect("cancel must finish within the stop bound")
        .unwrap(),
    );

    let session = session.lock().await;
    assert!(
        session.abort_flag.load(Ordering::SeqCst),
        "priority abort must set the live turn abort flag"
    );
    assert!(session.user_interrupt_flag.load(Ordering::SeqCst));
    assert_eq!(session.runtime.state, SessionState::Idle);
    assert!(session.command_queue.is_empty());
    assert!(output.contains("Agent stopped"), "{output}");
    assert!(!output.contains("command queued"), "{output}");
    drop(session);
    assert_eq!(
        storage::load_board(gcx, "task-1").await.unwrap().cards[0].column,
        "failed"
    );
}

#[tokio::test]
async fn cancel_agent_on_failed_card_stops_its_live_session() {
    for column in ["failed", "done"] {
        for cleared_chat_id in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let chat_id = (!cleared_chat_id).then(|| "agent-chat-1".into());
            let mut card = test_card(column, chat_id, None);
            card.final_report = Some("Original final report".into());
            let gcx = write_task(root.path(), card).await;
            if cleared_chat_id {
                storage::update_board_atomic(gcx.clone(), "task-1", |board| {
                    board.card_outcomes.insert(
                        "T-39".into(),
                        CardOutcome {
                            fragments: vec![OutcomeFragment::new(
                                "summary",
                                "Completed attempt",
                                Some("agent-chat-1".into()),
                            )],
                            ..Default::default()
                        },
                    );
                    Ok(())
                })
                .await
                .unwrap();
            }
            let before = storage::load_board(gcx.clone(), "task-1").await.unwrap();
            let session = executing_session(&gcx).await;
            let old_attempt = Arc::new(Mutex::new(ChatSession::new("old-attempt".into())));
            {
                let task_meta = session.lock().await.thread.task_meta.clone();
                let mut old = old_attempt.lock().await;
                old.thread.task_meta = task_meta;
                old.start_stream().unwrap();
            }
            gcx.chat_sessions
                .write()
                .await
                .insert("old-attempt".into(), old_attempt.clone());
            let unrelated = Arc::new(Mutex::new(ChatSession::new("unrelated".into())));
            {
                let mut other = unrelated.lock().await;
                other.thread.task_meta = Some(TaskMeta {
                    task_id: "another-task".into(),
                    role: "agents".into(),
                    card_id: Some("T-39".into()),
                    ..Default::default()
                });
                other.start_stream().unwrap();
            }
            gcx.chat_sessions
                .write()
                .await
                .insert("unrelated".into(), unrelated.clone());
            let facade = Arc::new(EngineChatSessionFacade::new(gcx.clone()));
            let ccx = planner_ccx(gcx.clone(), facade, "planner").await;

            let output = output_text(
                ToolCancelAgent::new()
                    .tool_execute(
                        ccx,
                        &"cancel-terminal".into(),
                        &args(&[
                            ("card_id", json!("T-39")),
                            ("reason", json!("stop stray turn")),
                        ]),
                    )
                    .await
                    .unwrap(),
            );

            assert!(session.lock().await.abort_flag.load(Ordering::SeqCst));
            assert_eq!(session.lock().await.runtime.state, SessionState::Idle);
            assert!(output.contains("Agent stopped"), "{output}");
            assert!(
                output.contains("agent-chat-1: Agent stopped; prior agent state: executing_tools"),
                "{output}"
            );
            assert!(
                output.contains("old-attempt: Agent stopped; prior agent state: generating"),
                "{output}"
            );
            assert!(old_attempt.lock().await.abort_flag.load(Ordering::SeqCst));
            assert!(!unrelated.lock().await.abort_flag.load(Ordering::SeqCst));
            let after = storage::load_board(gcx, "task-1").await.unwrap();
            assert_eq!(after.cards[0].column, column);
            assert_eq!(
                serde_json::to_value(after).unwrap(),
                serde_json::to_value(before).unwrap()
            );
        }
    }
}

#[tokio::test]
async fn cancel_agent_reports_honestly_when_abort_unconfirmed() {
    let root = tempfile::tempdir().unwrap();
    let gcx = write_task(
        root.path(),
        test_card("doing", Some("agent-chat-1".into()), None),
    )
    .await;
    let mut mock = MockChatFacade::new(Some(SessionState::ExecutingTools));
    mock.ignore_abort = true;
    let facade = Arc::new(mock);
    let ccx = planner_ccx(gcx, facade.clone(), "planner").await;

    let output = output_text(
        ToolCancelAgent::new()
            .tool_execute(
                ccx,
                &"cancel-ignored".into(),
                &args(&[("card_id", json!("T-39")), ("reason", json!("stop now"))]),
            )
            .await
            .unwrap(),
    );

    assert!(
        output.contains("abort requested; agent still running after 10 s"),
        "{output}"
    );
    assert!(!output.contains("Agent stopped"), "{output}");
    assert_eq!(
        facade.session_state("agent-chat-1").await.unwrap(),
        Some(SessionState::ExecutingTools)
    );
}
