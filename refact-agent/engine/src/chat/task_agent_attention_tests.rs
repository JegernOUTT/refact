use super::task_agent_monitor::{
    check_for_stuck_agents, handle_agent_streaming_error, tests::setup_monitor_case,
};
use crate::chat::task_agent_monitor::{AgentFailureKind, agent_failure_attention_kind};
use crate::chat::types::SessionState;
use crate::tasks::storage;
use crate::tools::tools_description::Tool;
use std::time::Duration;

#[tokio::test]
async fn unknown_error_sets_attention_and_keeps_doing() {
    let (_temp, app, task_id, _agent_chat_id, agent, planner) = setup_monitor_case(
        "doing",
        SessionState::Error,
        Duration::from_secs(5 * 60),
        vec![],
    )
    .await;
    agent.lock().await.runtime.error = Some("Unknown error".to_string());
    let task_meta = agent.lock().await.thread.task_meta.clone().unwrap();
    handle_agent_streaming_error(app.clone(), &task_meta, "Unknown error").await;

    let board = storage::load_board(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert_eq!(card.column, "doing");
    assert_eq!(
        card.attention
            .as_ref()
            .map(|attention| attention.kind.as_str()),
        Some("unknown")
    );
    assert_eq!(card.status_updates.len(), 1);
    assert!(card.status_updates[0]
        .message
        .starts_with("Needs attention:"));
    assert_eq!(planner.lock().await.pending_deliveries.len(), 1);
}

#[tokio::test]
async fn agent_finish_succeeds_after_attention() {
    let (_temp, app, task_id, agent_chat_id, agent, _planner) = setup_monitor_case(
        "doing",
        SessionState::Error,
        Duration::from_secs(90),
        vec![],
    )
    .await;
    agent.lock().await.runtime.error = Some("Unknown error".to_string());
    let task_meta = agent.lock().await.thread.task_meta.clone().unwrap();
    handle_agent_streaming_error(app.clone(), &task_meta, "Unknown error").await;

    let ccx = crate::at_commands::at_commands::AtCommandsContext::new_from_app(
        app.clone(),
        4096,
        20,
        false,
        vec![],
        agent_chat_id,
        None,
        "test-model".to_string(),
        Some(task_meta),
        None,
    )
    .await;
    let mut tool = crate::tools::tool_task_agent_finish::ToolTaskAgentFinish::new();
    let args = std::collections::HashMap::from([
        ("success".to_string(), serde_json::json!(true)),
        (
            "report".to_string(),
            serde_json::json!("Finished after recovery"),
        ),
    ]);
    tool.tool_execute(
        std::sync::Arc::new(tokio::sync::Mutex::new(ccx)),
        &"finish-call".to_string(),
        &args,
    )
    .await
    .unwrap();

    let board = storage::load_board(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert_eq!(card.column, "done");
    assert!(card.attention.is_none());
}

#[tokio::test]
async fn stall_sets_attention_not_failed() {
    let (_temp, app, task_id, _agent_chat_id, _agent, _planner) = setup_monitor_case(
        "doing",
        SessionState::Generating,
        Duration::from_secs(4 * 60 + 10),
        vec![],
    )
    .await;
    check_for_stuck_agents(app.clone()).await.unwrap();
    let board = storage::load_board(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert_eq!(card.column, "doing");
    assert_eq!(
        card.attention
            .as_ref()
            .map(|attention| attention.kind.as_str()),
        Some("stall")
    );
}

#[tokio::test]
async fn unloaded_session_with_recent_trajectory_is_not_flagged() {
    let (_temp, app, task_id, agent_chat_id, agent, _planner) =
        setup_monitor_case("doing", SessionState::Idle, Duration::from_secs(90), vec![]).await;
    {
        let mut sessions = app.chat.sessions.write().await;
        sessions.remove(&agent_chat_id);
    }
    let snapshot = {
        let session = agent.lock().await;
        super::trajectories::trajectory_snapshot_from_session(&session)
    };
    super::trajectories::save_trajectory_snapshot(app.gcx.clone(), snapshot)
        .await
        .unwrap();
    storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
        board
            .get_card_mut("T-1")
            .unwrap()
            .status_updates
            .push(crate::tasks::types::StatusUpdate {
                timestamp: (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339(),
                message: "Old card activity".to_string(),
            });
        Ok(())
    })
    .await
    .unwrap();
    check_for_stuck_agents(app.clone()).await.unwrap();
    let board = storage::load_board(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert_eq!(card.column, "doing");
    assert!(card.attention.is_none());
}

#[tokio::test]
async fn permanent_error_still_fails() {
    let (_temp, app, task_id, _agent_chat_id, agent, _planner) = setup_monitor_case(
        "doing",
        SessionState::Error,
        Duration::from_secs(90),
        vec![],
    )
    .await;
    let task_meta = agent.lock().await.thread.task_meta.clone().unwrap();
    handle_agent_streaming_error(
        app.clone(),
        &task_meta,
        "LLM error (401 Unauthorized): invalid api key",
    )
    .await;
    let board = storage::load_board(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert_eq!(card.column, "failed");
}

#[tokio::test]
async fn attention_clears_on_progress() {
    let (_temp, app, task_id, _agent_chat_id, agent, _planner) = setup_monitor_case(
        "doing",
        SessionState::Error,
        Duration::from_secs(90),
        vec![],
    )
    .await;
    let task_meta = agent.lock().await.thread.task_meta.clone().unwrap();
    handle_agent_streaming_error(app.clone(), &task_meta, "Unknown error").await;
    let ccx = crate::at_commands::at_commands::AtCommandsContext::new_from_app(
        app.clone(),
        4096,
        20,
        false,
        vec![],
        "agent-T-1".to_string(),
        None,
        "test-model".to_string(),
        Some(task_meta),
        None,
    )
    .await;
    let mut tool = crate::tools::tool_task_agent::ToolTaskAgentUpdate::new();
    let args = std::collections::HashMap::from([
        ("card_id".to_string(), serde_json::json!("T-1")),
        (
            "message".to_string(),
            serde_json::json!("Recovered and resumed work"),
        ),
    ]);
    tool.tool_execute(
        std::sync::Arc::new(tokio::sync::Mutex::new(ccx)),
        &"update-call".to_string(),
        &args,
    )
    .await
    .unwrap();
    let board = storage::load_board(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert!(card.attention.is_none());
}

#[test]
fn recoverable_failures_are_attention_only_negative_control() {
    let kind = AgentFailureKind::from_error("Unknown error");
    assert_eq!(agent_failure_attention_kind(kind), Some("unknown"));
    assert_eq!(
        agent_failure_attention_kind(AgentFailureKind::Permanent),
        None
    );
}
