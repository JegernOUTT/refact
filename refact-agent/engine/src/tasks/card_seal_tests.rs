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
