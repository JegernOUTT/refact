use std::time::Duration;

use crate::agents::seal::{is_sealed, seal_reason, SealReason};
use crate::chat::task_agent_monitor::tests::setup_monitor_case;
use crate::chat::types::SessionState;
use crate::tasks::storage;

#[tokio::test]
async fn card_leaving_doing_is_sealed_after_the_next_commit() {
    let (_workspace, app, task_id, chat_id, _session, _planner) =
        setup_monitor_case("doing", SessionState::Idle, Duration::ZERO, vec![]).await;
    assert!(!is_sealed(&app, &chat_id).await);
    storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
        board.get_card_mut("T-1").unwrap().column = "done".into();
        Ok(())
    })
    .await
    .unwrap();
    let before = app.gcx.task_card_states.parse_count();
    assert!(is_sealed(&app, &chat_id).await);
    assert_eq!(app.gcx.task_card_states.parse_count(), before);

    storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
        board.get_card_mut("T-1").unwrap().column = "doing".into();
        Ok(())
    })
    .await
    .unwrap();
    assert!(!is_sealed(&app, &chat_id).await);
    storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
        board.get_card_mut("T-1").unwrap().agent_chat_id = Some("replacement".into());
        Ok(())
    })
    .await
    .unwrap();
    assert!(is_sealed(&app, &chat_id).await);
    storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
        board.get_card_mut("T-1").unwrap().agent_chat_id = None;
        Ok(())
    })
    .await
    .unwrap();
    assert!(!is_sealed(&app, &chat_id).await);
    storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
        board.cards.clear();
        Ok(())
    })
    .await
    .unwrap();
    assert!(matches!(seal_reason(&app, &chat_id).await,
        Some(SealReason::TaskCardLeftDoing { column, .. }) if column == "deleted"));
}

#[tokio::test]
async fn another_worker_commit_cannot_leave_a_stale_seal_or_owner() {
    let (_workspace, app, task_id, chat_id, _session, _planner) =
        setup_monitor_case("done", SessionState::Idle, Duration::ZERO, vec![]).await;
    assert!(is_sealed(&app, &chat_id).await);
    let other = crate::global_context::tests::make_test_gcx().await;
    *other.documents_state.workspace_folders.lock().unwrap() = app
        .gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    storage::update_board_atomic(other.clone(), &task_id, |board| {
        board.get_card_mut("T-1").unwrap().column = "doing".into();
        Ok(())
    })
    .await
    .unwrap();
    assert!(!is_sealed(&app, &chat_id).await);
    storage::update_board_atomic(other.clone(), &task_id, |board| {
        board.get_card_mut("T-1").unwrap().agent_chat_id = Some("other-worker-owner".into());
        Ok(())
    })
    .await
    .unwrap();
    assert!(is_sealed(&app, &chat_id).await);
    storage::update_board_atomic(other, &task_id, |board| {
        board.get_card_mut("T-1").unwrap().agent_chat_id = Some(chat_id.clone());
        Ok(())
    })
    .await
    .unwrap();
    assert!(!is_sealed(&app, &chat_id).await);
}

#[tokio::test]
async fn rejected_commit_and_read_failure_never_publish_a_seal() {
    let (_workspace, app, task_id, chat_id, _session, _planner) =
        setup_monitor_case("doing", SessionState::Idle, Duration::ZERO, vec![]).await;
    let mut board = storage::load_board(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    board.cards[0].column = "done".into();
    assert!(
        storage::save_board_checked(app.gcx.clone(), &task_id, board.rev + 1, &board)
            .await
            .is_err()
    );
    assert!(!is_sealed(&app, &chat_id).await);
    storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
        board.cards[0].column = "done".into();
        Ok(())
    })
    .await
    .unwrap();
    assert!(is_sealed(&app, &chat_id).await);
    let dir = storage::find_task_dir(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    tokio::fs::write(dir.join("board.yaml"), "cards: [invalid")
        .await
        .unwrap();
    assert!(!is_sealed(&app, &chat_id).await);
}
