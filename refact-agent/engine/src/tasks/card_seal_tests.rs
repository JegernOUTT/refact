use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::Mutex;

use crate::agents::seal::{seal_reason, SealReason};
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::task_agent_monitor::tests::setup_monitor_case;
use crate::chat::types::{ChatSession, SessionState};
use crate::tasks::storage;
use crate::tasks::types::CardAttention;
use crate::tools::tool_task_agent_finish::ToolTaskAgentFinish;
use crate::tools::tools_description::Tool;

struct Fixture {
    _temp: tempfile::TempDir,
    app: AppState,
    task_id: String,
    agent_chat_id: String,
    agent: Arc<Mutex<ChatSession>>,
    planner: Arc<Mutex<ChatSession>>,
}

async fn fixture() -> Fixture {
    let (_temp, app, task_id, agent_chat_id, agent, planner) = setup_monitor_case(
        "doing",
        SessionState::ExecutingTools,
        Duration::ZERO,
        vec![],
    )
    .await;
    Fixture {
        _temp,
        app,
        task_id,
        agent_chat_id,
        agent,
        planner,
    }
}

fn expected(fixture: &Fixture, column: &str) -> Option<SealReason> {
    Some(SealReason::TaskCardLeftDoing {
        task_id: fixture.task_id.clone(),
        card_id: "T-1".to_string(),
        column: column.to_string(),
    })
}

async fn finish(fixture: &Fixture, success: bool) {
    let task_meta = fixture.agent.lock().await.thread.task_meta.clone();
    let ccx = AtCommandsContext::new_from_app(
        fixture.app.clone(),
        4096,
        20,
        false,
        vec![],
        fixture.agent_chat_id.clone(),
        None,
        "test-model".to_string(),
        task_meta,
        None,
    )
    .await;
    ToolTaskAgentFinish::new()
        .tool_execute(
            Arc::new(Mutex::new(ccx)),
            &"finish-call".to_string(),
            &HashMap::from([
                ("success".to_string(), json!(success)),
                ("report".to_string(), json!("Card seal lifecycle test")),
            ]),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn finished_card_chat_is_sealed() {
    let fixture = fixture().await;
    assert_eq!(
        seal_reason(&fixture.app, &fixture.agent_chat_id).await,
        None
    );

    finish(&fixture, true).await;

    let board = storage::load_board(fixture.app.gcx.clone(), &fixture.task_id)
        .await
        .unwrap();
    assert_eq!(board.get_card("T-1").unwrap().column, "done");
    assert_eq!(
        seal_reason(&fixture.app, &fixture.agent_chat_id).await,
        expected(&fixture, "done")
    );
}

#[tokio::test]
async fn failed_card_chat_is_sealed_even_after_agent_chat_id_is_cleared() {
    let fixture = fixture().await;

    finish(&fixture, false).await;

    let board = storage::load_board(fixture.app.gcx.clone(), &fixture.task_id)
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert_eq!(card.column, "failed");
    assert!(card.agent_chat_id.is_none());
    assert!(card.assignee.is_none());
    assert_eq!(
        seal_reason(&fixture.app, &fixture.agent_chat_id).await,
        expected(&fixture, "failed")
    );
}

#[tokio::test]
async fn replaced_chat_is_sealed_after_restart() {
    let fixture = fixture().await;
    finish(&fixture, false).await;
    let replacement_chat_id = "replacement-agent-chat";
    storage::update_board_atomic(fixture.app.gcx.clone(), &fixture.task_id, |board| {
        let card = board.get_card_mut("T-1").unwrap();
        card.column = "doing".to_string();
        card.assignee = Some("replacement-agent".to_string());
        card.agent_chat_id = Some(replacement_chat_id.to_string());
        Ok(())
    })
    .await
    .unwrap();
    let mut replacement = ChatSession::new(replacement_chat_id.to_string());
    let mut meta = fixture.agent.lock().await.thread.task_meta.clone().unwrap();
    meta.agent_id = Some("replacement-agent".to_string());
    replacement.thread.task_meta = Some(meta);
    fixture.app.chat.sessions.write().await.insert(
        replacement_chat_id.to_string(),
        Arc::new(Mutex::new(replacement)),
    );

    assert_eq!(
        seal_reason(&fixture.app, &fixture.agent_chat_id).await,
        expected(&fixture, "doing")
    );
    assert_eq!(seal_reason(&fixture.app, replacement_chat_id).await, None);
}

#[tokio::test]
async fn doing_card_with_attention_is_not_sealed() {
    let fixture = fixture().await;
    storage::update_board_atomic(fixture.app.gcx.clone(), &fixture.task_id, |board| {
        board.get_card_mut("T-1").unwrap().attention = Some(CardAttention {
            reason: "Recoverable stall".to_string(),
            kind: "stall".to_string(),
            since: chrono::Utc::now().to_rfc3339(),
            count: 1,
        });
        Ok(())
    })
    .await
    .unwrap();

    assert_eq!(
        seal_reason(&fixture.app, &fixture.agent_chat_id).await,
        None
    );
}

#[tokio::test]
async fn planner_chat_is_never_sealed() {
    let fixture = fixture().await;
    let planner_chat_id = {
        let mut planner = fixture.planner.lock().await;
        planner.thread.task_meta.as_mut().unwrap().card_id = Some("T-1".to_string());
        planner.thread.id.clone()
    };
    for column in ["doing", "done", "failed", "planned", "merged"] {
        storage::update_board_atomic(fixture.app.gcx.clone(), &fixture.task_id, |board| {
            board.get_card_mut("T-1").unwrap().column = column.to_string();
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(seal_reason(&fixture.app, &planner_chat_id).await, None);
    }
}

#[tokio::test]
async fn deleted_card_chat_is_sealed() {
    let fixture = fixture().await;
    storage::update_board_atomic(fixture.app.gcx.clone(), &fixture.task_id, |board| {
        board.cards.clear();
        Ok(())
    })
    .await
    .unwrap();

    assert_eq!(
        seal_reason(&fixture.app, &fixture.agent_chat_id).await,
        expected(&fixture, "deleted")
    );
}

#[tokio::test]
async fn every_column_outside_doing_seals_the_card_chat() {
    let fixture = fixture().await;
    for column in ["planned", "done", "failed", "cancelled", "review", "merged"] {
        storage::update_board_atomic(fixture.app.gcx.clone(), &fixture.task_id, |board| {
            let card = board.get_card_mut("T-1").unwrap();
            card.column = column.to_string();
            card.agent_chat_id = None;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            seal_reason(&fixture.app, &fixture.agent_chat_id).await,
            expected(&fixture, column)
        );
    }
}

#[tokio::test]
async fn queued_doing_card_with_unset_owner_is_not_sealed() {
    let fixture = fixture().await;
    storage::update_board_atomic(fixture.app.gcx.clone(), &fixture.task_id, |board| {
        board.get_card_mut("T-1").unwrap().agent_chat_id = None;
        Ok(())
    })
    .await
    .unwrap();
    crate::tasks::admission::reserve(
        &fixture.app,
        &fixture.agent_chat_id,
        &fixture.task_id,
        "planner-test",
        0,
    )
    .await
    .unwrap();

    let reason = seal_reason(&fixture.app, &fixture.agent_chat_id).await;
    crate::tasks::admission::end(&fixture.agent_chat_id).await;
    assert_eq!(reason, None);
}

#[tokio::test]
async fn board_read_failure_does_not_seal() {
    let fixture = fixture().await;
    let task_dir = storage::find_task_dir(fixture.app.gcx.clone(), &fixture.task_id)
        .await
        .unwrap();
    tokio::fs::write(task_dir.join("board.yaml"), "cards: [invalid")
        .await
        .unwrap();
    assert!(
        storage::load_board(fixture.app.gcx.clone(), &fixture.task_id)
            .await
            .is_err()
    );

    assert_eq!(
        seal_reason(&fixture.app, &fixture.agent_chat_id).await,
        None
    );
}

#[tokio::test]
async fn unloaded_chat_is_not_read_from_trajectory() {
    let fixture = fixture().await;
    let snapshot = {
        let session = fixture.agent.lock().await;
        crate::chat::trajectories::trajectory_snapshot_from_session(&session)
    };
    crate::chat::trajectories::save_trajectory_snapshot(fixture.app.gcx.clone(), snapshot)
        .await
        .unwrap();
    storage::update_board_atomic(fixture.app.gcx.clone(), &fixture.task_id, |board| {
        board.get_card_mut("T-1").unwrap().column = "failed".to_string();
        Ok(())
    })
    .await
    .unwrap();
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .remove(&fixture.agent_chat_id);

    assert_eq!(
        seal_reason(&fixture.app, &fixture.agent_chat_id).await,
        None
    );
}

#[tokio::test]
async fn other_roles_or_missing_task_card_metadata_are_not_sealed() {
    let fixture = fixture().await;
    storage::update_board_atomic(fixture.app.gcx.clone(), &fixture.task_id, |board| {
        board.get_card_mut("T-1").unwrap().column = "failed".to_string();
        Ok(())
    })
    .await
    .unwrap();
    for role in ["planner", "subchats", "task_agent"] {
        fixture
            .agent
            .lock()
            .await
            .thread
            .task_meta
            .as_mut()
            .unwrap()
            .role = role.to_string();
        assert_eq!(
            seal_reason(&fixture.app, &fixture.agent_chat_id).await,
            None
        );
    }
    {
        let mut agent = fixture.agent.lock().await;
        let meta = agent.thread.task_meta.as_mut().unwrap();
        meta.role = "agents".to_string();
        meta.card_id = None;
    }
    assert_eq!(
        seal_reason(&fixture.app, &fixture.agent_chat_id).await,
        None
    );
    fixture.agent.lock().await.thread.task_meta = None;
    assert_eq!(
        seal_reason(&fixture.app, &fixture.agent_chat_id).await,
        None
    );
}
