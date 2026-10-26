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

/// Register one worktree against the fixture project exactly as the worktree
/// service persists it, and bind a live session to it.
async fn registered_worktree(
    app: &AppState,
    source: &std::path::Path,
    chat_id: &str,
) -> (
    crate::worktrees::service::WorktreeService,
    crate::worktrees::types::WorktreeMeta,
) {
    let service = crate::worktrees::service::WorktreeService::new_async(
        app.gcx.cache_dir.clone(),
        source.to_path_buf(),
    )
    .await
    .unwrap();
    let mut registry = service.load_registry().await.unwrap();
    let id = "wt-removed";
    let root = service.worktree_path_for_id(id).unwrap();
    tokio::fs::create_dir_all(&root).await.unwrap();
    let meta = crate::worktrees::types::WorktreeMeta {
        id: id.to_string(),
        kind: "chat".to_string(),
        root: root.clone(),
        source_workspace_root: registry.source_workspace_root.clone(),
        repo_root: registry.source_workspace_root.clone(),
        branch: Some("feature".to_string()),
        base_branch: None,
        base_commit: None,
        task_id: None,
        card_id: None,
        agent_id: None,
        enforce: false,
    };
    registry
        .records
        .push(crate::worktrees::types::WorktreeRegistryRecord {
            meta: meta.clone(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            last_seen_at: None,
            references: Vec::new(),
            last_known_status: None,
        });
    service.save_registry(&registry).await.unwrap();

    let mut session = ChatSession::new(chat_id.to_string());
    session.work_project_root = Some(source.to_path_buf());
    session.thread.title = "work in the worktree".into();
    session.thread.worktree = Some(meta.clone());
    app.chat
        .sessions
        .write()
        .await
        .insert(chat_id.to_string(), Arc::new(Mutex::new(session)));
    (service, meta)
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn a_removed_worktree_stays_inspectable_but_stops_reading_as_an_active_location() {
    let (app, dir, caller) = fixture().await;
    let (service, meta) = registered_worktree(&app, dir.path(), "in-worktree").await;
    let touched = meta.root.join("touched.rs");
    std::fs::write(&touched, "hello").unwrap();
    crate::file_activity::record_read(Some("in-worktree"), &touched, "hello", 1, 1, 1, None);

    super::roots::reset_presence_cache_for_test();
    let live = overview(&app, &caller, WorkOverviewRequest::default())
        .await
        .unwrap();
    let live_row = live
        .rows
        .iter()
        .find(|row| row.chat_id.as_deref() == Some("in-worktree"))
        .unwrap();
    assert_eq!(live_row.root.presence, RootPresence::Present);
    assert!(live_row.root.presence.is_active_location());

    let registry_before = std::fs::read(service.registry_path()).unwrap();
    std::fs::remove_dir_all(&meta.root).unwrap();
    super::roots::reset_presence_cache_for_test();

    let gone = overview(&app, &caller, WorkOverviewRequest::default())
        .await
        .unwrap();
    let gone_row = gone
        .rows
        .iter()
        .find(|row| row.chat_id.as_deref() == Some("in-worktree"))
        .unwrap();
    assert_eq!(gone_row.root.presence, RootPresence::Missing);
    assert!(!gone_row.root.presence.is_active_location());
    assert!(gone
        .roots
        .iter()
        .any(|root| root.id == meta.id && root.presence == RootPresence::Missing));

    let inventory = inspect(
        &app,
        &caller,
        InspectRequest {
            selector: InspectSelector::ProjectWorktrees {},
            cursor: None,
            limit: Some(50),
        },
    )
    .await
    .unwrap();
    let listed = inventory
        .page
        .roots
        .iter()
        .find(|root| root.id == meta.id)
        .expect("a removed worktree stays an inspectable historical record");
    assert_eq!(listed.presence, RootPresence::Missing);

    // Reporting a missing root must never repair, prune or recreate anything.
    assert_eq!(
        std::fs::read(service.registry_path()).unwrap(),
        registry_before
    );
    assert!(!meta.root.exists());
    let reloaded = service.load_registry().await.unwrap();
    assert!(reloaded
        .records
        .iter()
        .any(|record| record.meta.id == meta.id));
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
