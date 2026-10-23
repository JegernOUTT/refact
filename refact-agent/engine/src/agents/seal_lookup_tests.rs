use std::time::{Duration, Instant};

use crate::agents::seal::{is_sealed, seal_reason, SealReason};
use crate::agents::types::{AgentCompletion, BgAgentKind, CreateAgentRequest};
use crate::app_state::AppState;
use crate::chat::task_agent_monitor::tests::setup_monitor_case;
use crate::chat::types::SessionState;
use crate::tasks::storage;

#[tokio::test]
async fn is_sealed_does_not_reparse_the_board_per_call() {
    let (_workspace, app, _task_id, chat_id, session, _planner) =
        setup_monitor_case("doing", SessionState::Idle, Duration::ZERO, vec![]).await;
    let before = app.gcx.task_card_states.parse_count();
    for _ in 0..1000 {
        assert!(!is_sealed(&app, &chat_id).await);
    }
    assert_eq!(
        app.gcx.task_card_states.parse_count() - before,
        0,
        "warm seal checks must not parse the board"
    );

    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = app
        .gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    let cold = AppState::from_gcx(gcx).await;
    cold.chat
        .sessions
        .write()
        .await
        .insert(chat_id.clone(), session);
    let before = cold.gcx.task_card_states.parse_count();
    let checks = (0..1000).map(|_| is_sealed(&cold, &chat_id));
    assert!(futures::future::join_all(checks)
        .await
        .into_iter()
        .all(|sealed| !sealed));
    let parses = cold.gcx.task_card_states.parse_count() - before;
    assert!(
        parses <= 1,
        "1000 seal checks parsed the board {parses} times"
    );
}

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

#[tokio::test]
async fn background_child_lookup_does_not_clone_all_records() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let app = AppState::from_gcx(gcx).await;
    let (record, _, _) = app
        .agents
        .create(CreateAgentRequest {
            parent_chat_id: "parent".into(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "test".into(),
            title: "lookup".into(),
            prompt: "x".repeat(1_000_000),
            target_files: vec![],
            model: "test".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap();
    app.agents
        .mark_running(&record.agent_id, "lookup-child".into())
        .await
        .unwrap();
    app.agents
        .mark_completed(
            &record.agent_id,
            AgentCompletion {
                result_summary: "done".into(),
                edited_files: vec![],
                diff_summary: None,
                conflict_summary: None,
                child_chat_id: None,
                deliverable_status: None,
                handoff: None,
            },
        )
        .await
        .unwrap();
    let before = app.agents.list_all_calls_for_test();
    for _ in 0..1000 {
        assert!(is_sealed(&app, "lookup-child").await);
        assert!(!is_sealed(&app, "unknown-child").await);
    }
    assert_eq!(app.agents.list_all_calls_for_test(), before);
}

#[tokio::test]
#[ignore]
async fn five_mb_board_seal_timing() {
    let (_workspace, app, task_id, chat_id, _session, _planner) =
        setup_monitor_case("doing", SessionState::Idle, Duration::ZERO, vec![]).await;
    storage::update_board_atomic(app.gcx.clone(), &task_id, |board| {
        board.cards[0].instructions = "x".repeat(5_000_000);
        Ok(())
    })
    .await
    .unwrap();
    let dir = storage::find_task_dir(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    let bytes = tokio::fs::metadata(dir.join("board.yaml"))
        .await
        .unwrap()
        .len();
    let start = Instant::now();
    for _ in 0..20 {
        let board = storage::load_board(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        assert_eq!(board.get_card("T-1").unwrap().column, "doing");
    }
    let before = start.elapsed();
    assert!(!is_sealed(&app, &chat_id).await);
    let parses = app.gcx.task_card_states.parse_count();
    let start = Instant::now();
    for _ in 0..1000 {
        assert!(!is_sealed(&app, &chat_id).await);
    }
    let after = start.elapsed();
    assert_eq!(app.gcx.task_card_states.parse_count(), parses);
    println!("H-11 synthetic board: {bytes} bytes; before: 20 full-board lookups in {before:?} ({:?}/call); after: 1000 is_sealed calls in {after:?} ({:?}/call), 0 board parses", before / 20, after / 1000);
}
