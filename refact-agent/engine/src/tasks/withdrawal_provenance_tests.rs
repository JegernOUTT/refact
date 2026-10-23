use std::sync::Arc;

use refact_chat_api::QuestionState;
use refact_privacy::{PrivacyPolicy, ShellBehavior, Zone};

use crate::caps::BaseModelRecord;
use crate::chat::questions_closure_tests::{ask, fixture, Fixture};
use crate::coordination::board_rows::{rows_from_board, BoardProvenance};
use crate::coordination::privacy_guard::clear_rows;
use crate::file_activity::TOOL_ACTOR;
use crate::tasks::types::TaskBoard;

use super::storage;

async fn persisted(f: &Fixture) -> (TaskBoard, BoardProvenance) {
    let dir = storage::find_task_dir(f.app.gcx.clone(), &f.task_id)
        .await
        .unwrap();
    let body = tokio::fs::read_to_string(dir.join("board.yaml"))
        .await
        .unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(&body).unwrap();
    let board = serde_yaml::from_value(value.clone()).unwrap();
    let provenance = serde_yaml::from_value(value["work_provenance"].clone()).unwrap();
    (board, provenance)
}

async fn provenanced_fixture() -> Fixture {
    let f = fixture().await;
    let board = storage::load_board(f.app.gcx.clone(), &f.task_id)
        .await
        .unwrap();
    let dir = storage::find_task_dir(f.app.gcx.clone(), &f.task_id)
        .await
        .unwrap();
    tokio::fs::remove_file(dir.join("board.yaml"))
        .await
        .unwrap();
    TOOL_ACTOR
        .scope(
            f.agent_id.clone(),
            storage::create_board(f.app.gcx.clone(), &f.task_id, &board),
        )
        .await
        .unwrap();
    let (board, provenance) = persisted(&f).await;
    assert!(provenance.valid_for(&board, &crate::file_activity::settings()));
    f
}

async fn withdraw(f: &Fixture) -> String {
    let question = ask(f).await;
    let (before, provenance) = persisted(f).await;
    assert_eq!(before.get_card("T-1").unwrap().column, "doing");
    assert!(provenance.valid_for(&before, &crate::file_activity::settings()));
    assert_eq!(
        f.app.questions().get(&question).unwrap().state,
        QuestionState::Pending
    );
    TOOL_ACTOR
        .scope(
            f.agent_id.clone(),
            storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
                board.get_card_mut("T-1").unwrap().column = "done".into();
                Ok(())
            }),
        )
        .await
        .unwrap();
    question
}

#[tokio::test]
async fn withdrawal_write_keeps_board_provenance_valid() {
    let f = provenanced_fixture().await;
    let question = withdraw(&f).await;
    let (board, provenance) = persisted(&f).await;
    assert!(board
        .get_card("T-1")
        .unwrap()
        .status_updates
        .iter()
        .any(|update| {
            update.message
                == format!("[WITHDRAWN:{question}] Questions withdrawn when the agent finished")
        }));
    assert_eq!(
        f.app.questions().get(&question).unwrap().state,
        QuestionState::Withdrawn
    );
    assert!(
        provenance.valid_for(&board, &crate::file_activity::settings()),
        "withdrawal marker must be included in the persisted board provenance digest"
    );
}

#[tokio::test]
async fn withdrawn_board_rows_stay_visible_to_a_cleared_model() {
    let f = provenanced_fixture().await;
    withdraw(&f).await;
    let (board, provenance) = persisted(&f).await;
    let roots = crate::files_correction::get_project_dirs(f.app.gcx.clone()).await;
    let root = &roots[0];
    f.app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
        zones: vec![Zone {
            name: "project".into(),
            patterns: vec![format!("{}/**", root.display()), root.display().to_string()],
            send_to: vec!["trusted".into()],
            on_shell_read: ShellBehavior::Withhold,
        }],
        ..Default::default()
    });
    let rows = rows_from_board(
        root,
        &f.task_id,
        &board,
        &provenance,
        &crate::file_activity::settings(),
    )
    .unwrap();
    assert_eq!(rows.len(), board.cards.len());
    let denied = BaseModelRecord {
        id: "other/model".into(),
        ..Default::default()
    };
    assert!(clear_rows(&f.app.gcx, rows.clone(), &denied)
        .unwrap()
        .is_empty());
    let cleared = BaseModelRecord {
        id: "trusted/model".into(),
        ..Default::default()
    };
    let visible = clear_rows(&f.app.gcx, rows, &cleared).unwrap();
    assert_eq!(visible.len(), board.cards.len());
    assert_eq!(
        visible[0].metadata.task_id.as_deref(),
        Some(f.task_id.as_str())
    );
    assert_eq!(visible[0].metadata.card_id.as_deref(), Some("T-1"));
}

#[tokio::test]
async fn provenance_stays_complete_on_the_next_write() {
    let f = provenanced_fixture().await;
    withdraw(&f).await;
    let (before, _) = persisted(&f).await;
    storage::update_board_atomic(f.app.gcx.clone(), &f.task_id, |board| {
        board
            .get_card_mut("T-1")
            .unwrap()
            .instructions
            .push_str(" Updated.");
        Ok(())
    })
    .await
    .unwrap();
    let (board, provenance) = persisted(&f).await;
    assert_eq!(board.rev, before.rev + 1);
    assert!(
        provenance.complete,
        "withdrawal must not poison later provenance"
    );
    assert!(provenance.valid_for(&board, &crate::file_activity::settings()));
}
