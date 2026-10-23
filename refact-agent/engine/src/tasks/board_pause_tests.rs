use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::agents::seal::{seal_reason, SealReason};
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatToolCall, ChatToolFunction};
use crate::chat::task_agent_monitor::{check_for_stuck_agents, tests::setup_monitor_case};
use crate::chat::types::{ChatSession, SessionState};
use crate::tasks::{storage, types::TaskBoard};
use crate::tools::tool_agent_lifecycle::{ToolCancelAgent, ToolPauseAgent, ToolResumeAgent};
use crate::tools::tools_description::Tool;
use refact_chat_api::ChatCommand;
use refact_core::chat_types::{DeliveryOutcome, PendingDelivery, PushMode};

pub(crate) struct Fixture {
    _temp: tempfile::TempDir,
    pub(crate) app: AppState,
    task_id: String,
    agent_chat_id: String,
    pub(crate) agent: Arc<Mutex<ChatSession>>,
    planner: Arc<Mutex<ChatSession>>,
    ccx: Arc<Mutex<AtCommandsContext>>,
}

pub(crate) async fn fixture(state: SessionState) -> Fixture {
    let (_temp, app, task_id, agent_chat_id, agent, planner) =
        setup_monitor_case("doing", state, Duration::from_secs(3600), vec![]).await;
    let (planner_id, meta) = {
        let session = planner.lock().await;
        (session.chat_id.clone(), session.thread.task_meta.clone())
    };
    let ccx = Arc::new(Mutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            vec![],
            planner_id,
            None,
            "test-model".to_string(),
            meta,
            None,
        )
        .await,
    ));
    Fixture {
        _temp,
        app,
        task_id,
        agent_chat_id,
        agent,
        planner,
        ccx,
    }
}

fn args() -> HashMap<String, Value> {
    HashMap::from([
        ("card_id".to_string(), json!("T-1")),
        ("reason".to_string(), json!("review checkpoint")),
    ])
}

pub(crate) async fn pause(f: &Fixture) {
    ToolPauseAgent::new()
        .tool_execute(f.ccx.clone(), &"pause-call".to_string(), &args())
        .await
        .unwrap();
}

pub(crate) async fn resume(f: &Fixture) {
    ToolResumeAgent::new()
        .tool_execute(f.ccx.clone(), &"resume-call".to_string(), &args())
        .await
        .unwrap();
}

fn pending_tool_message() -> ChatMessage {
    ChatMessage {
        role: "assistant".to_string(),
        tool_calls: Some(vec![ChatToolCall {
            id: "pending-cat".to_string(),
            index: None,
            function: ChatToolFunction {
                name: "cat".to_string(),
                arguments: json!({"paths": "Cargo.toml"}).to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        }]),
        ..Default::default()
    }
}

#[tokio::test]
async fn planner_pause_sets_runtime_gate_and_persists_paused_at() {
    let f = fixture(SessionState::ExecutingTools).await;
    let meta = f.agent.lock().await.thread.task_meta.clone().unwrap();
    let mut sibling = ChatSession::new("card-sibling".to_string());
    sibling.thread.task_meta = Some(meta.clone());
    let sibling = Arc::new(Mutex::new(sibling));
    f.app
        .chat
        .sessions
        .write()
        .await
        .insert("card-sibling".to_string(), sibling.clone());
    let mut unrelated = ChatSession::new("other-card".to_string());
    unrelated.thread.task_meta = Some(refact_chat_api::TaskMeta {
        card_id: Some("T-2".to_string()),
        ..meta
    });
    let unrelated = Arc::new(Mutex::new(unrelated));
    f.app
        .chat
        .sessions
        .write()
        .await
        .insert("other-card".to_string(), unrelated.clone());
    storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
        board.get_card_mut("T-1").unwrap().agent_chat_id = Some("stale-agent-id".to_string());
        Ok(())
    })
    .await
    .unwrap();

    pause(&f).await;

    let board = storage::load_board(f.app.gcx.clone(), &f.task_id)
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    let paused_at = card
        .paused_at
        .as_deref()
        .expect("pause timestamp must be persisted");
    assert!(chrono::DateTime::parse_from_rfc3339(paused_at).is_ok());
    assert_eq!(card.paused_by.as_deref(), Some("planner-test"));
    assert_eq!(card.column, "doing");
    let session = f.agent.lock().await;
    assert!(
        session.planner_paused(),
        "planner pause must set a runtime gate, not just send text"
    );
    assert_eq!(session.planner_pause.as_ref().unwrap().since, paused_at);
    assert_eq!(
        session.planner_pause.as_ref().unwrap().by.as_deref(),
        Some("planner-test")
    );
    assert_eq!(session.runtime.state, SessionState::ExecutingTools);
    assert!(!session.abort_flag.load(Ordering::SeqCst));
    assert!(session.runtime.pause_reasons.is_empty());
    drop(session);
    assert!(sibling.lock().await.planner_paused());
    assert!(!unrelated.lock().await.planner_paused());
    assert!(!f.planner.lock().await.planner_paused());
}

#[tokio::test]
async fn monitor_does_not_flag_a_paused_card_after_the_stuck_timeout() {
    let f = fixture(SessionState::Idle).await;
    let old = (Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
    storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
        let card = board.get_card_mut("T-1").unwrap();
        card.started_at = Some(old.clone());
        card.created_at = old.clone();
        card.last_heartbeat_at = Some(old.clone());
        card.paused_at = Some(old.clone());
        card.paused_by = Some("planner-test".to_string());
        Ok(())
    })
    .await
    .unwrap();
    for state in [
        SessionState::Idle,
        SessionState::Completed,
        SessionState::Generating,
        SessionState::ExecutingTools,
        SessionState::Error,
    ] {
        {
            let mut session = f.agent.lock().await;
            session.runtime.state = state;
            session.runtime.error =
                Some("LLM error (401 Unauthorized): invalid api key".to_string());
        }
        check_for_stuck_agents(f.app.clone()).await.unwrap();
    }
    let task_meta = f.agent.lock().await.thread.task_meta.clone().unwrap();
    for error in [
        "LLM error (503 Service Unavailable): overloaded",
        "LLM error (401 Unauthorized): invalid api key",
    ] {
        crate::chat::task_agent_monitor::handle_agent_streaming_error(
            f.app.clone(),
            &task_meta,
            error,
        )
        .await;
    }
    f.app.chat.sessions.write().await.remove(&f.agent_chat_id);
    check_for_stuck_agents(f.app.clone()).await.unwrap();
    storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
        board.get_card_mut("T-1").unwrap().agent_chat_id = None;
        Ok(())
    })
    .await
    .unwrap();
    check_for_stuck_agents(f.app.clone()).await.unwrap();

    let board = storage::load_board(f.app.gcx.clone(), &f.task_id)
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert_eq!(card.column, "doing");
    assert!(card.attention.is_none());
    assert!(card.final_report.is_none());
    assert!(card.status_updates.is_empty());
    assert!(f.planner.lock().await.pending_deliveries.is_empty());
}

#[tokio::test]
async fn planner_resume_clears_gate_and_field() {
    let f = fixture(SessionState::Idle).await;
    pause(&f).await;
    assert!(f.agent.lock().await.planner_paused());

    resume(&f).await;

    let board = storage::load_board(f.app.gcx.clone(), &f.task_id)
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert!(card.paused_at.is_none());
    assert!(card.paused_by.is_none());
    assert!(!f.agent.lock().await.planner_paused());
    assert!(card
        .status_updates
        .iter()
        .any(|update| update.message == "Resumed by planner"));
}

#[test]
fn old_board_without_paused_at_loads() {
    let board: TaskBoard = serde_yaml::from_str(
        "cards:\n  - id: T-1\n    title: Legacy\n    column: doing\n    created_at: '2026-01-01T00:00:00Z'\n",
    )
    .unwrap();
    assert!(board.cards[0].paused_at.is_none());
    assert!(board.cards[0].paused_by.is_none());
}

#[tokio::test]
async fn deliveries_wait_without_preemption_and_resume_wakes_pending_work_once() {
    let f = fixture(SessionState::Idle).await;
    pause(&f).await;
    let messages = f.agent.lock().await.messages.len();
    for push in [PushMode::Preempt, PushMode::Append, PushMode::WhenIdle] {
        let outcome = crate::chat::delivery::deliver_to_chat(
            f.app.clone(),
            &f.agent_chat_id,
            PendingDelivery::new(
                vec![ChatMessage {
                    role: "user".to_string(),
                    content: crate::call_validation::ChatContent::SimpleText(format!(
                        "steer {push:?}"
                    )),
                    ..Default::default()
                }],
                push,
                "test.steer",
                true,
            ),
        )
        .await
        .unwrap();
        assert_eq!(outcome, DeliveryOutcome::Queued);
    }
    {
        let mut session = f.agent.lock().await;
        assert_eq!(session.messages.len(), messages);
        assert_eq!(session.pending_deliveries.len(), 4);
        assert!(session.delivery_wake_sources.is_empty());
        assert_eq!(session.drain_pending_deliveries(), (vec![], false));
        assert!(!session.abort_flag.load(Ordering::SeqCst));
    }

    resume(&f).await;

    let mut session = f.agent.lock().await;
    let (delivered, wake) = session.drain_pending_deliveries();
    assert_eq!(delivered.len(), 4);
    assert!(wake);
    assert!(session.pending_deliveries.is_empty());
    assert!(session.command_queue.is_empty());
    assert_eq!(session.drain_pending_deliveries(), (vec![], false));
}

#[tokio::test]
async fn priority_abort_and_cancel_still_stop_paused_agents_and_leave_them_sealed() {
    let f = fixture(SessionState::Generating).await;
    pause(&f).await;
    f.app
        .chat
        .facade
        .push_priority_command(&f.agent_chat_id, ChatCommand::Abort {})
        .await
        .unwrap();
    {
        let session = f.agent.lock().await;
        assert!(session.abort_flag.load(Ordering::SeqCst));
        assert!(session.user_interrupt_flag.load(Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert!(session.planner_paused());
    }
    ToolCancelAgent::new()
        .tool_execute(f.ccx.clone(), &"cancel".to_string(), &args())
        .await
        .unwrap();
    assert_eq!(
        seal_reason(&f.app, &f.agent_chat_id).await,
        Some(SealReason::TaskCardLeftDoing {
            task_id: f.task_id.clone(),
            card_id: "T-1".to_string(),
            column: "failed".to_string(),
        })
    );
    assert!(ToolResumeAgent::new()
        .tool_execute(f.ccx.clone(), &"late-resume".to_string(), &args())
        .await
        .is_err());
    crate::chat::start_generation(f.app.clone(), f.agent.clone()).await;
    assert!(f.agent.lock().await.draft_message.is_none());
}

#[tokio::test]
async fn planner_pause_gates_generation_and_new_tools_at_safe_boundaries() {
    let f = fixture(SessionState::Idle).await;
    pause(&f).await;
    crate::chat::start_generation(f.app.clone(), f.agent.clone()).await;
    assert!(f.agent.lock().await.draft_message.is_none());
    {
        let mut session = f.agent.lock().await;
        session.add_message(pending_tool_message());
        assert!(session.start_stream().is_none());
    }
    let outcome =
        crate::chat::tools::process_tool_calls_once(f.app.clone(), f.agent.clone(), "agent", None)
            .await;
    assert!(matches!(outcome, crate::chat::tools::ToolStepOutcome::Stop));
    let session = f.agent.lock().await;
    assert_eq!(session.runtime.state, SessionState::Idle);
    assert!(session.planner_pause_pending_tools);
    assert_eq!(session.messages.last().unwrap().role, "assistant");
}

#[tokio::test]
async fn resume_queues_one_continuation_for_a_parked_tool_batch() {
    let f = fixture(SessionState::Idle).await;
    f.agent.lock().await.add_message(pending_tool_message());
    pause(&f).await;
    crate::chat::tools::process_tool_calls_once(f.app.clone(), f.agent.clone(), "agent", None)
        .await;

    resume(&f).await;

    let session = f.agent.lock().await;
    assert!(!session.planner_paused());
    assert!(session.planner_pause_pending_tools);
    assert_eq!(session.command_queue.len(), 1);
    assert!(matches!(
        session.command_queue[0].command,
        ChatCommand::Regenerate {}
    ));
    assert!(session.delivery_wake_sources.is_empty());
    assert_eq!(session.pending_deliveries.len(), 2);
}

#[tokio::test]
async fn loaded_agent_restores_planner_pause_from_the_card() {
    let f = fixture(SessionState::Idle).await;
    pause(&f).await;
    crate::chat::trajectories::maybe_save_trajectory(f.app.clone(), f.agent.clone()).await;
    f.app.chat.sessions.write().await.remove(&f.agent_chat_id);

    let restored = crate::chat::get_or_create_session_with_trajectory(
        f.app.clone(),
        &f.app.chat.sessions,
        &f.agent_chat_id,
    )
    .await;

    let session = restored.lock().await;
    assert!(session.planner_paused());
    assert_eq!(
        session.planner_pause.as_ref().unwrap().by.as_deref(),
        Some("planner-test")
    );
}

#[tokio::test]
async fn pause_refuses_every_card_outside_doing_without_runtime_mutation() {
    let f = fixture(SessionState::Idle).await;
    for column in ["planned", "done", "failed", "regressed", "merged"] {
        storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
            board.get_card_mut("T-1").unwrap().column = column.to_string();
            Ok(())
        })
        .await
        .unwrap();
        let error = ToolPauseAgent::new()
            .tool_execute(f.ccx.clone(), &"pause".to_string(), &args())
            .await
            .unwrap_err();
        assert!(error.contains("Cannot pause card T-1"), "{error}");
        assert!(!f.agent.lock().await.planner_paused());
        assert!(storage::load_board(f.app.gcx.clone(), &f.task_id)
            .await
            .unwrap()
            .cards[0]
            .paused_at
            .is_none());
    }
}

#[tokio::test]
async fn replacing_or_finishing_a_run_clears_its_persisted_pause() {
    for replace in [true, false] {
        let f = fixture(SessionState::Idle).await;
        pause(&f).await;
        let (board, ()) = storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
            let card = board.get_card_mut("T-1").unwrap();
            if replace {
                card.agent_chat_id = Some("replacement-chat".to_string());
            } else {
                card.column = "failed".to_string();
            }
            Ok(())
        })
        .await
        .unwrap();
        assert!(board.cards[0].paused_at.is_none());
        assert!(board.cards[0].paused_by.is_none());
    }
}
