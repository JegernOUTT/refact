use super::service::*;
use super::types::*;
use super::scope::WorkCaller;
use crate::app_state::AppState;
use crate::chat::types::ChatSession;
use std::sync::Arc;
use serial_test::serial;
use tokio::sync::Mutex;

async fn fixture() -> (AppState, tempfile::TempDir, WorkCaller) {
    let dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
    let app = AppState::from_gcx(gcx).await;
    let caller = WorkCaller {
        source_root: dir.path().into(),
        checkout_root: dir.path().into(),
        root_chat_id: Some("caller".into()),
        principal: "caller".into(),
        model: Some(crate::caps::BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        }),
    };
    for id in ["a", "b", "c"] {
        let mut session = ChatSession::new(id.into());
        session.work_project_root = Some(dir.path().into());
        session.thread.title = format!("title {id}");
        app.chat
            .sessions
            .write()
            .await
            .insert(id.into(), Arc::new(Mutex::new(session)));
    }
    (app, dir, caller)
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn shared_projection_paginates_without_gaps_and_rejects_changed_state() {
    let (app, _dir, caller) = fixture().await;
    let request = WorkOverviewRequest {
        rows: Some(1),
        ..Default::default()
    };
    let first = overview(&app, &caller, request.clone()).await.unwrap();
    let retry = overview(&app, &caller, request.clone()).await.unwrap();
    assert_eq!(first.snapshot_id, retry.snapshot_id);
    assert_eq!(first.next_cursor, retry.next_cursor);
    assert_eq!(first.total, 3);
    let second_request = WorkOverviewRequest {
        cursor: first.next_cursor.clone(),
        ..request.clone()
    };
    let second = overview(&app, &caller, second_request.clone())
        .await
        .unwrap();
    assert_ne!(first.rows[0].key, second.rows[0].key);
    let third = overview(
        &app,
        &caller,
        WorkOverviewRequest {
            cursor: second.next_cursor.clone(),
            ..request.clone()
        },
    )
    .await
    .unwrap();
    assert_eq!(third.returned, 1);
    assert!(third.next_cursor.is_none());
    let session = app.chat.sessions.read().await.get("b").cloned().unwrap();
    session.lock().await.thread.title = "new title".into();
    assert_eq!(
        overview(&app, &caller, second_request)
            .await
            .unwrap_err()
            .code,
        "cursor_expired"
    );
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn denied_titles_paths_counts_and_changes_do_not_affect_visible_pages() {
    use refact_privacy::{PrivacyPolicy, Zone, ShellBehavior, Attribution, FileRecord};
    let (app, dir, caller) = fixture().await;
    let secret = dir.path().join("secret");
    std::fs::write(&secret, "private").unwrap();
    app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
        zones: vec![Zone {
            name: "secret".into(),
            patterns: vec![secret.to_string_lossy().into()],
            send_to: vec!["other".into()],
            on_shell_read: ShellBehavior::Withhold,
        }],
        ..Default::default()
    });
    let private = app.chat.sessions.read().await.get("b").cloned().unwrap();
    let mut message =
        crate::call_validation::ChatMessage::new("user".into(), "private content".into());
    crate::privacy::records::merge_records(
        &mut message,
        [FileRecord {
            path: secret.to_string_lossy().into(),
            zone: "secret".into(),
            attribution: Attribution::Declared,
        }],
    );
    private.lock().await.add_message(message);
    let first = overview(
        &app,
        &caller,
        WorkOverviewRequest {
            rows: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(first.total, 2);
    assert!(!serde_json::to_string(&first).unwrap().contains("title b"));
    private.lock().await.thread.title = "another private title".into();
    let second = overview(
        &app,
        &caller,
        WorkOverviewRequest {
            rows: Some(1),
            cursor: first.next_cursor,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(second.seq, first.seq);
    let denied = inspect(
        &app,
        &caller,
        InspectRequest {
            selector: InspectSelector::Work {
                work_id: "b".into(),
            },
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap_err();
    let absent = inspect(
        &app,
        &caller,
        InspectRequest {
            selector: InspectSelector::Work {
                work_id: "missing".into(),
            },
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(denied.code, absent.code);
    assert_eq!(denied.message, absent.message);
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn changed_destination_invalidates_cursor_and_policy_failure_is_not_empty_success() {
    let (app, _dir, mut caller) = fixture().await;
    let first = overview(
        &app,
        &caller,
        WorkOverviewRequest {
            rows: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    caller.model.as_mut().unwrap().id = "other/model".into();
    let error = overview(
        &app,
        &caller,
        WorkOverviewRequest {
            rows: Some(1),
            cursor: first.next_cursor,
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "invalid_cursor");
    app.gcx.privacy_policy_load.write().unwrap().error = Some("cannot read private config".into());
    let error = overview(&app, &caller, WorkOverviewRequest::default())
        .await
        .unwrap_err();
    assert_eq!(error.code, "policy_unavailable");
    assert!(!error.to_string().contains("private config"));
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn inspector_scopes_files_and_pages_worktree_inventory() {
    let (app, dir, caller) = fixture().await;
    let inventory = inspect(
        &app,
        &caller,
        InspectRequest {
            selector: InspectSelector::ProjectWorktrees {},
            cursor: None,
            limit: Some(1),
        },
    )
    .await
    .unwrap();
    assert_eq!(inventory.page.page_unit, "worktrees");
    assert_eq!(inventory.page.returned, inventory.page.roots.len());
    for paths in [vec![], vec!["../outside".into()]] {
        assert!(inspect(
            &app,
            &caller,
            InspectRequest {
                selector: InspectSelector::Files { paths },
                cursor: None,
                limit: Some(1)
            }
        )
        .await
        .is_err());
    }
    let path = dir.path().join("read.rs");
    std::fs::write(&path, "hello").unwrap();
    crate::file_activity::record_read(Some("a"), &path, "hello", 1, 1, 1, None);
    let files = inspect(
        &app,
        &caller,
        InspectRequest {
            selector: InspectSelector::Files {
                paths: vec!["read.rs".into()],
            },
            cursor: None,
            limit: Some(1),
        },
    )
    .await
    .unwrap();
    assert_eq!(files.page.rows.len(), 1);
    assert_eq!(files.page.rows[0].chat_id.as_deref(), Some("a"));
    assert!(files.integration_overlaps.is_empty());
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn same_relative_path_in_a_separate_checkout_is_an_advisory_overlap_only() {
    let (app, dir, caller) = fixture().await;
    let worktree = dir.path().join("cache/worktrees/peer");
    std::fs::create_dir_all(worktree.join("src")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(worktree.join("src/lib.rs"), "peer").unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "mine").unwrap();
    let mut peer = ChatSession::new("peer".into());
    peer.work_project_root = Some(dir.path().into());
    peer.thread.title = "peer".into();
    peer.thread.worktree = Some(crate::worktrees::types::WorktreeMeta {
        id: "peer-checkout".into(),
        kind: "chat".into(),
        root: worktree.clone(),
        source_workspace_root: dir.path().into(),
        repo_root: dir.path().into(),
        branch: Some("peer".into()),
        base_branch: None,
        base_commit: None,
        task_id: None,
        card_id: None,
        agent_id: None,
        enforce: false,
    });
    app.chat
        .sessions
        .write()
        .await
        .insert("peer".into(), Arc::new(Mutex::new(peer)));
    crate::file_activity::record_read(
        Some("peer"),
        &worktree.join("src/lib.rs"),
        "peer",
        1,
        1,
        1,
        None,
    );
    crate::file_activity::record_read(
        Some("a"),
        &dir.path().join("src/lib.rs"),
        "mine",
        1,
        1,
        1,
        None,
    );
    let files = inspect(
        &app,
        &caller,
        InspectRequest {
            selector: InspectSelector::Files {
                paths: vec!["src/lib.rs".into()],
            },
            cursor: None,
            limit: Some(10),
        },
    )
    .await
    .unwrap();
    let mut ids: Vec<String> = files
        .page
        .rows
        .iter()
        .filter_map(|row| row.chat_id.clone())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["a".to_string(), "peer".to_string()]);
    assert_eq!(
        files.integration_overlaps,
        vec!["trajectory:peer".to_string()]
    );
    let physical = inspect(
        &app,
        &caller,
        InspectRequest {
            selector: InspectSelector::Files {
                paths: vec!["cache/worktrees/peer/src/lib.rs".into()],
            },
            cursor: None,
            limit: Some(10),
        },
    )
    .await
    .unwrap();
    assert_eq!(physical.page.rows.len(), 1);
    assert_eq!(physical.page.rows[0].chat_id.as_deref(), Some("peer"));
    assert!(physical.integration_overlaps.is_empty());
    let unrelated = inspect(
        &app,
        &caller,
        InspectRequest {
            selector: InspectSelector::Files {
                paths: vec!["src/other.rs".into()],
            },
            cursor: None,
            limit: Some(10),
        },
    )
    .await
    .unwrap();
    assert!(unrelated.page.rows.is_empty());
    assert!(unrelated.integration_overlaps.is_empty());
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn board_catalog_bootstraps_once_and_guarded_writes_and_deletion_update_it() {
    let (app, dir, mut caller) = fixture().await;
    caller.model = None;
    let task = crate::tasks::storage::create_task(app.gcx.clone(), "Catalog task")
        .await
        .unwrap();
    let mut board = super::board_rows::tests::board();
    crate::tasks::storage::save_board_checked(app.gcx.clone(), &task.id, 0, &board)
        .await
        .unwrap();
    let path = dir
        .path()
        .join(".refact/tasks")
        .join(&task.id)
        .join("board.yaml");
    let saved = tokio::fs::read(&path).await.unwrap();
    let first = overview(&app, &caller, WorkOverviewRequest::default())
        .await
        .unwrap();
    let row = first
        .rows
        .iter()
        .find(|row| row.chat_id.as_deref() == Some("agent-a"))
        .unwrap();
    assert_eq!(row.metadata.task_id.as_deref(), Some(task.id.as_str()));
    assert_eq!(row.state, WorkStateTag::Interrupted);
    assert!(row.stale);
    let restarted = super::board_index::WorkBoardIndex::default();
    let (restored, unavailable) = restarted.rows(dir.path()).await;
    assert!(!unavailable);
    assert_eq!(restored.len(), 1);
    tokio::fs::write(&path, "invalid: [yaml").await.unwrap();
    for _ in 0..3 {
        assert!(overview(&app, &caller, WorkOverviewRequest::default())
            .await
            .unwrap()
            .rows
            .iter()
            .any(|row| row.title.as_deref() == Some("Board title")));
        assert_eq!(restarted.rows(dir.path()).await.0.len(), 1);
    }
    tokio::fs::write(&path, saved).await.unwrap();
    board.rev += 1;
    board.cards[0].title = "Changed through guarded save".into();
    crate::tasks::storage::save_board_checked(app.gcx.clone(), &task.id, 1, &board)
        .await
        .unwrap();
    let detail = inspect(
        &app,
        &caller,
        InspectRequest {
            selector: InspectSelector::Work {
                work_id: "agent-a".into(),
            },
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        detail.row.unwrap().title.as_deref(),
        Some("Changed through guarded save")
    );
    crate::tasks::storage::delete_task(app.gcx.clone(), &task.id)
        .await
        .unwrap();
    assert!(!overview(&app, &caller, WorkOverviewRequest::default())
        .await
        .unwrap()
        .rows
        .iter()
        .any(|row| row.chat_id.as_deref() == Some("agent-a")));
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn board_model_disclosure_carries_writer_provenance_and_denied_counts_stay_hidden() {
    let (app, dir, caller) = fixture().await;
    let secret = dir.path().join("secret.rs");
    tokio::fs::write(&secret, "source").await.unwrap();
    app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
        zones: vec![refact_privacy::Zone {
            name: "guarded".into(),
            patterns: vec![secret.to_string_lossy().into()],
            send_to: vec!["trusted".into()],
            on_shell_read: refact_privacy::ShellBehavior::Withhold,
        }],
        ..Default::default()
    });
    let writer = app.chat.sessions.read().await.get("a").cloned().unwrap();
    let mut message = crate::call_validation::ChatMessage::new("user".into(), "source".into());
    crate::privacy::records::merge_records(
        &mut message,
        [refact_privacy::FileRecord {
            path: secret.to_string_lossy().into(),
            zone: "guarded".into(),
            attribution: refact_privacy::Attribution::Declared,
        }],
    );
    writer.lock().await.add_message(message);
    let task = crate::tasks::storage::create_task(app.gcx.clone(), "Task")
        .await
        .unwrap();
    let board = super::board_rows::tests::board();
    crate::file_activity::TOOL_ACTOR
        .scope(
            "a".into(),
            crate::tasks::storage::save_board_checked(app.gcx.clone(), &task.id, 0, &board),
        )
        .await
        .unwrap();
    let visible = overview(&app, &caller, WorkOverviewRequest::default())
        .await
        .unwrap();
    let row = visible
        .rows
        .iter()
        .find(|row| row.chat_id.as_deref() == Some("agent-a"))
        .unwrap();
    assert!(row.provenance_complete);
    assert!(row
        .source_records
        .iter()
        .any(|record| record.zone == "guarded"));
    let child = format!("board-provenance-{}", uuid::Uuid::new_v4());
    crate::tasks::admission::reserve(&app, &child, "project", "root", 0)
        .await
        .unwrap();
    let gcx = app.gcx.clone();
    let task_id = task.id.clone();
    let (sent, received) = tokio::sync::oneshot::channel();
    crate::file_activity::TOOL_ACTOR
        .scope(
            "a".into(),
            crate::tasks::admission::spawn_tracked(&child, async move {
                let result = crate::tasks::storage::update_board_atomic(gcx, &task_id, |board| {
                    board.cards[0].column = "done".into();
                    Ok(())
                })
                .await;
                let _ = sent.send(result);
            }),
        )
        .await
        .unwrap();
    received.await.unwrap().unwrap();
    crate::tasks::admission::end(&child).await;
    crate::tasks::admission::shutdown(&app).await;
    let continued = inspect(
        &app,
        &caller,
        InspectRequest {
            selector: InspectSelector::Work {
                work_id: "agent-a".into(),
            },
            cursor: None,
            limit: None,
        },
    )
    .await
    .unwrap()
    .row
    .unwrap();
    assert!(continued.provenance_complete);
    assert_eq!(continued.state, WorkStateTag::Completed);
    let mut incompatible = caller;
    incompatible.model.as_mut().unwrap().id = "other/model".into();
    let hidden = overview(&app, &incompatible, WorkOverviewRequest::default())
        .await
        .unwrap();
    assert_eq!(hidden.total, 2);
    assert!(!serde_json::to_string(&hidden)
        .unwrap()
        .contains("Board title"));
    assert_eq!(hidden.withheld_rows, 0);
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn inspection_keeps_operator_degradation_but_models_cannot_probe_hidden_source_failures() {
    let (app, dir, mut caller) = fixture().await;
    let task_dir = dir.path().join(".refact/tasks/task-corrupt");
    tokio::fs::create_dir_all(&task_dir).await.unwrap();
    tokio::fs::write(task_dir.join("board.yaml"), "invalid: [yaml")
        .await
        .unwrap();
    let request = InspectRequest {
        selector: InspectSelector::ProjectWorktrees {},
        cursor: None,
        limit: None,
    };
    let hidden = inspect(&app, &caller, request.clone()).await.unwrap();
    assert!(hidden.page.degraded.is_empty());
    caller.model = None;
    let operator = inspect(&app, &caller, request).await.unwrap();
    assert!(operator
        .page
        .degraded
        .iter()
        .any(|source| source == "board index unavailable"));
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn known_board_writer_waits_for_provenance_before_taking_the_board_lock() {
    let (app, _dir, _caller) = fixture().await;
    let writer = app.chat.sessions.read().await.get("a").cloned().unwrap();
    let held = writer.lock().await;
    let mut source = Box::pin(
        crate::file_activity::TOOL_ACTOR
            .scope("a".into(), super::board_rows::actor_provenance(&app.gcx)),
    );
    assert!(matches!(
        futures::poll!(source.as_mut()),
        std::task::Poll::Pending
    ));
    drop(held);
    assert!(source.await.unwrap().unwrap().1);
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn missing_or_timed_out_board_writer_never_changes_persisted_board() {
    let (app, dir, _caller) = fixture().await;
    let task = crate::tasks::storage::create_task(app.gcx.clone(), "Task")
        .await
        .unwrap();
    let path = dir
        .path()
        .join(".refact/tasks")
        .join(&task.id)
        .join("board.yaml");
    let before = tokio::fs::read(&path).await.unwrap();
    let attempted = super::board_rows::tests::board();
    let error = crate::file_activity::TOOL_ACTOR
        .scope(
            "missing".into(),
            crate::tasks::storage::save_board_checked(app.gcx.clone(), &task.id, 0, &attempted),
        )
        .await
        .unwrap_err();
    assert!(error.message().contains("writer session unavailable"));
    assert_eq!(tokio::fs::read(&path).await.unwrap(), before);
    let writer = app.chat.sessions.read().await.get("a").cloned().unwrap();
    let held = writer.lock().await;
    tokio::time::pause();
    let mut write = Box::pin(crate::file_activity::TOOL_ACTOR.scope(
        "a".into(),
        crate::tasks::storage::save_board_checked(app.gcx.clone(), &task.id, 0, &attempted),
    ));
    assert!(matches!(
        futures::poll!(write.as_mut()),
        std::task::Poll::Pending
    ));
    tokio::time::advance(std::time::Duration::from_secs(
        crate::file_activity::settings().lock_timeout_secs + 1,
    ))
    .await;
    assert!(write
        .await
        .unwrap_err()
        .message()
        .contains("temporarily unavailable"));
    tokio::time::resume();
    drop(held);
    assert_eq!(tokio::fs::read(&path).await.unwrap(), before);
}
