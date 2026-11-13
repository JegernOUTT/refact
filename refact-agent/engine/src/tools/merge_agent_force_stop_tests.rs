use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use refact_runtime_api::{ChatSessionFacade, SessionState};
use serde_json::json;
use tokio::sync::Mutex;

use crate::app_state::EngineChatSessionFacade;
use crate::chat::types::{ChatSession, TaskMeta};
use crate::tasks::storage;

use super::merge_agent_target_checkout_tests::{git, Fixture, TARGET};
use super::tool_agent_lifecycle::tests::{args, planner_ccx, MockChatFacade};
use super::tool_task_merge_agent::merge_agent_with_git_probe;

async fn doing_fixture(legacy: bool) -> Fixture {
    let fixture = Fixture::new(true).await;
    fixture.write_board(legacy);
    storage::update_board_atomic(fixture.gcx.clone(), "task-1", |board| {
        let card = board.get_card_mut("T-1").unwrap();
        card.column = "doing".into();
        card.final_report = None;
        card.completed_at = None;
        card.assignee = Some("stale-agent".into());
        card.agent_chat_id = Some("stale-chat".into());
        Ok(())
    })
    .await
    .unwrap();
    fixture
}

async fn live_session(
    fixture: &Fixture,
    id: &str,
    task_id: &str,
    card_id: &str,
    role: &str,
    state: SessionState,
) -> Arc<Mutex<ChatSession>> {
    let mut session = ChatSession::new(id.into());
    session.thread.task_meta = Some(TaskMeta {
        task_id: task_id.into(),
        card_id: Some(card_id.into()),
        role: role.into(),
        ..Default::default()
    });
    session.start_stream().unwrap();
    session.set_runtime_state(state, None);
    let session = Arc::new(Mutex::new(session));
    fixture
        .gcx
        .chat_sessions
        .write()
        .await
        .insert(id.into(), session.clone());
    session
}

fn snapshot(root: &Path) -> Vec<String> {
    [
        vec!["show-ref"],
        vec!["status", "--porcelain"],
        vec!["diff", "--cached"],
        vec!["diff"],
        vec!["worktree", "list", "--porcelain"],
    ]
    .map(|command| git(root, &command))
    .to_vec()
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn force_merge_of_doing_card_stops_live_agent_before_touching_worktree() {
    for legacy in [false, true] {
        let fixture = doing_fixture(legacy).await;
        let executing = live_session(
            &fixture,
            "live-agent",
            "task-1",
            "T-1",
            "agents",
            SessionState::ExecutingTools,
        )
        .await;
        let generating = live_session(
            &fixture,
            "old-attempt",
            "task-1",
            "T-1",
            "agents",
            SessionState::Generating,
        )
        .await;
        let mut unrelated = Vec::new();
        for (id, task, card, role) in [
            ("other-task", "task-2", "T-1", "agents"),
            ("other-card", "task-1", "T-2", "agents"),
            ("planner", "task-1", "T-1", "planner"),
        ] {
            unrelated
                .push(live_session(&fixture, id, task, card, role, SessionState::Generating).await);
        }
        let facade = Arc::new(EngineChatSessionFacade::new(fixture.gcx.clone()));
        let ccx = planner_ccx(fixture.gcx.clone(), facade, "planner").await;
        let probed = Arc::new(AtomicBool::new(false));
        let observed = probed.clone();
        merge_agent_with_git_probe(
            ccx,
            "force-merge",
            &args(&[("card_id", json!("T-1"))]),
            move |root| async move {
                for session in [executing, generating] {
                    let session = session.lock().await;
                    assert!(
                        session.abort_flag.load(Ordering::SeqCst),
                        "abort must be observed before the first git/worktree operation"
                    );
                    assert_eq!(session.runtime.state, SessionState::Idle);
                    assert!(session.command_queue.is_empty());
                }
                observed.store(true, Ordering::SeqCst);
                tokio::process::Command::new("git")
                    .args(["rev-parse", "--is-inside-work-tree"])
                    .current_dir(root)
                    .output()
                    .await
                    .unwrap()
                    .status
                    .success()
            },
        )
        .await
        .unwrap();
        assert!(probed.load(Ordering::SeqCst));
        for session in unrelated {
            assert!(!session.lock().await.abort_flag.load(Ordering::SeqCst));
        }
        assert_merged(&fixture).await;
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn force_merge_refuses_when_stop_unconfirmed() {
    for legacy in [false, true] {
        let fixture = doing_fixture(legacy).await;
        live_session(
            &fixture,
            "live-agent",
            "task-1",
            "T-1",
            "agents",
            SessionState::ExecutingTools,
        )
        .await;
        std::fs::write(fixture.agent.join("pending.txt"), "not committed\n").unwrap();
        let roots = [&fixture.source, &fixture.planner, &fixture.agent];
        let before = roots.map(|root| snapshot(root));
        let registry = std::fs::read(fixture.service.registry_path()).unwrap();
        let board = storage::load_board(fixture.gcx.clone(), "task-1")
            .await
            .unwrap();
        let mut mock = MockChatFacade::new(Some(SessionState::ExecutingTools));
        mock.ignore_abort = true;
        let facade = Arc::new(mock);
        let ccx = planner_ccx(fixture.gcx.clone(), facade.clone(), "planner").await;
        let error = merge_agent_with_git_probe(
            ccx,
            "refused-merge",
            &args(&[("card_id", json!("T-1"))]),
            |_| async {
                panic!("unconfirmed stop must not reach the first git/worktree operation")
            },
        )
        .await
        .unwrap_err();
        assert!(
            error.contains("agent still running; cancel it first"),
            "{error}"
        );
        assert!(error.contains("still running after 10 s"), "{error}");
        assert_eq!(
            facade.session_state("live-agent").await.unwrap(),
            Some(SessionState::ExecutingTools)
        );
        assert_eq!(before, roots.map(|root| snapshot(root)));
        assert_eq!(
            registry,
            std::fs::read(fixture.service.registry_path()).unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(fixture.agent.join("pending.txt")).unwrap(),
            "not committed\n"
        );
        let after = storage::load_board(fixture.gcx.clone(), "task-1")
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(board).unwrap(),
            serde_json::to_value(after).unwrap()
        );
    }
}

async fn assert_merged(fixture: &Fixture) {
    assert_eq!(
        std::fs::read_to_string(fixture.planner.join("file.txt")).unwrap(),
        "merged\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.source.join("file.txt")).unwrap(),
        "base\n"
    );
    assert_eq!(
        git(&fixture.source, &["branch", "--show-current"]).trim(),
        "main"
    );
    assert_eq!(
        git(&fixture.planner, &["branch", "--show-current"]).trim(),
        TARGET
    );
    assert!(!fixture.agent.exists());
    let board = storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert_eq!(card.column, "done");
    assert!(card.assignee.is_none());
    assert!(card.agent_chat_id.is_none());
    assert!(card.agent_worktree.is_none());
    assert!(card.agent_branch.is_none());
}

fn merge_markers(card: &crate::tasks::types::BoardCard) -> Vec<String> {
    card.status_updates
        .iter()
        .filter(|update| {
            update
                .message
                .starts_with(super::tool_task_merge_agent::MERGE_AGENT_STATUS_PREFIX)
        })
        .map(|update| update.message.clone())
        .collect()
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn every_applied_merge_records_exactly_one_merge_marker() {
    for legacy in [false, true] {
        for doing in [false, true] {
            let fixture = if doing {
                doing_fixture(legacy).await
            } else {
                let fixture = Fixture::new(true).await;
                fixture.write_board(legacy);
                fixture
            };
            let started = chrono::Utc::now();
            let facade = Arc::new(EngineChatSessionFacade::new(fixture.gcx.clone()));
            let ccx = planner_ccx(fixture.gcx.clone(), facade, "planner").await;
            merge_agent_with_git_probe(
                ccx,
                "marker-merge",
                &args(&[("card_id", json!("T-1"))]),
                |_| async { true },
            )
            .await
            .unwrap();
            assert_merged(&fixture).await;
            let board = storage::load_board(fixture.gcx.clone(), "task-1")
                .await
                .unwrap();
            let card = board.get_card("T-1").unwrap();
            let markers = merge_markers(card);
            assert_eq!(
                markers.len(),
                1,
                "legacy={legacy} doing={doing}: {markers:?}"
            );
            let marker = card
                .status_updates
                .iter()
                .find(|update| update.message == markers[0])
                .unwrap();
            assert!(
                chrono::DateTime::parse_from_rfc3339(&marker.timestamp).unwrap() >= started,
                "marker must carry a fresh timestamp"
            );
            assert!(markers[0].contains(TARGET), "{markers:?}");
            if doing {
                assert!(card.status_updates.iter().any(|update| update
                    .message
                    .contains("marked done for manual review")
                    || update
                        .message
                        .contains("Marked done by planner for manual review")));
            }
        }
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn registered_lease_loss_after_applied_merge_is_merged_with_warning_and_one_marker() {
    let fixture = Fixture::new(true).await;
    fixture.write_board(false);
    std::fs::write(fixture.planner.join("notes.txt"), "user notes\n").unwrap();
    crate::coordination::merge_gate::post_apply_fault::install(&fixture.planner);
    let facade = Arc::new(EngineChatSessionFacade::new(fixture.gcx.clone()));
    let ccx = planner_ccx(fixture.gcx.clone(), facade, "planner").await;

    let result = merge_agent_with_git_probe(
        ccx,
        "late-loss",
        &args(&[("card_id", json!("T-1"))]),
        |_| async { true },
    )
    .await
    .expect("an applied merge is never reported as an error");

    let crate::call_validation::ContextEnum::ChatMessage(message) = &result.1[0] else {
        panic!("expected chat message")
    };
    let text = message.content.content_text_only();
    assert!(text.contains("Agent Work Merged"), "{text}");
    assert!(text.contains("injected lease loss"), "{text}");
    assert_eq!(
        std::fs::read_to_string(fixture.planner.join("file.txt")).unwrap(),
        "merged\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.planner.join("notes.txt")).unwrap(),
        "user notes\n"
    );
    assert!(git(&fixture.planner, &["stash", "list"]).trim().is_empty());
    let board = storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap();
    assert_eq!(merge_markers(board.get_card("T-1").unwrap()).len(), 1);
}
