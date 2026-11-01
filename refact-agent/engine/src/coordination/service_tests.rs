use super::service::*;
use super::types::*;
use super::scope::WorkCaller;
use crate::app_state::AppState;
use crate::chat::types::ChatSession;
use std::sync::Arc;
use serial_test::serial;
use tokio::sync::Mutex;

async fn fixture() -> (AppState, tempfile::TempDir, WorkCaller) {
    let dir = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
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
    let expected_registry = registry.clone();
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
            creator_chat_id: None,
            meta: meta.clone(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            last_seen_at: None,
            references: Vec::new(),
            last_known_status: None,
        });
    service
        .compare_and_save_registry(&expected_registry, &registry)
        .await
        .unwrap();

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
    let inventory_request = InspectRequest {
        selector: InspectSelector::ProjectWorktrees {},
        cursor: None,
        limit: Some(1),
    };
    let frozen_inventory = inspect(&app, &caller, inventory_request.clone())
        .await
        .unwrap();

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
    let continued = inspect(
        &app,
        &caller,
        InspectRequest {
            cursor: frozen_inventory.page.next_cursor.clone(),
            ..inventory_request
        },
    )
    .await
    .unwrap();
    assert_eq!(
        continued.page.snapshot_id,
        frozen_inventory.page.snapshot_id
    );
    assert!(continued.page.snapshot_stale);
    assert!(continued
        .page
        .roots
        .iter()
        .all(|root| root.presence == RootPresence::Present));

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
async fn ordinary_activity_preserves_immutable_page_rows_and_fresh_view_advances() {
    let (app, _dir, caller) = fixture().await;
    let request = WorkOverviewRequest {
        rows: Some(1),
        ..Default::default()
    };
    let first = overview(&app, &caller, request.clone()).await.unwrap();
    assert_eq!(first.total, 3);
    let continuation = WorkOverviewRequest {
        cursor: Some(first.next_cursor.clone().unwrap()),
        ..request.clone()
    };
    let second = overview(&app, &caller, continuation.clone()).await.unwrap();
    assert_ne!(first.rows[0].key, second.rows[0].key);
    let peer = app.chat.sessions.read().await.get("b").cloned().unwrap();
    {
        let mut peer = peer.lock().await;
        peer.thread.title = "new title".into();
        peer.runtime.state = crate::chat::types::SessionState::ExecutingTools;
        peer.last_activity = std::time::Instant::now() + std::time::Duration::from_secs(1);
    }
    let own = app.chat.sessions.read().await.get("a").cloned().unwrap();
    own.lock().await.runtime.queue_size = 2;
    let frozen = overview(&app, &caller, continuation.clone()).await.unwrap();
    assert_eq!(frozen.rows, second.rows);
    assert_eq!(frozen.next_cursor, second.next_cursor);
    assert_eq!(
        frozen.snapshot_observed_at_ms,
        first.snapshot_observed_at_ms
    );
    assert!(frozen.snapshot_stale);
    assert!(frozen.generated_at_ms >= frozen.snapshot_observed_at_ms);
    let fresh = overview(&app, &caller, request).await.unwrap();
    assert_eq!(fresh.snapshot_id, first.snapshot_id);
    assert!(fresh.seq > first.seq);
    assert!(!fresh.snapshot_stale);
    assert_eq!(fresh.rows[0].title.as_deref(), Some("new title"));
    let retry = overview(&app, &caller, continuation).await.unwrap();
    assert_eq!(retry.rows, second.rows);
    assert_eq!(retry.next_cursor, second.next_cursor);
}

#[serial(coordination_work_overview)]
#[serial(runtime_settings)]
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
    let files = inspect(
        &app,
        &caller,
        InspectRequest {
            selector: InspectSelector::Files {
                paths: vec!["secret".into()],
            },
            cursor: None,
            limit: Some(1),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(files.code, absent.code);
    assert!(!serde_json::to_string(&files).unwrap().contains("secret"));

    for id in ["a", "c"] {
        app.chat.sessions.write().await.remove(id);
        let id = format!("discovery-cap-visible-{id}");
        let mut session = ChatSession::new(id.clone());
        session.work_project_root = Some(dir.path().into());
        app.chat
            .sessions
            .write()
            .await
            .insert(id, Arc::new(Mutex::new(session)));
    }
    let _settings = crate::tools::settings_guard::SettingsGuard::install(|settings| {
        settings.coordination.max_work_history_rows = 3;
    });
    let request = WorkOverviewRequest {
        rows: Some(1),
        ..Default::default()
    };
    let error = overview(&app, &caller, request.clone()).await.unwrap_err();
    assert_eq!(error.code, "unavailable");
    assert_eq!(
        error.cause,
        Some(super::error::WorkFailureCause::SnapshotRowCapacity)
    );
    let public = serde_json::to_value(&error).unwrap();
    assert_eq!(public["cause"], "snapshot_row_capacity");
    assert!(public["retryable"].as_bool().unwrap());
    assert!(uuid::Uuid::parse_str(public["request_id"].as_str().unwrap()).is_ok());
    assert_eq!(public.as_object().unwrap().len(), 5);
    assert!(error
        .message
        .contains("root scope, states or an authorized path"));
    let c = app
        .chat
        .sessions
        .read()
        .await
        .get("discovery-cap-visible-c")
        .cloned()
        .unwrap();
    c.lock().await.thread.root_chat_id = Some("caller".into());
    private.lock().await.thread.root_chat_id = Some("caller".into());
    let filtered_request = WorkOverviewRequest {
        scope: OverviewScope::Root,
        ..request.clone()
    };
    let filtered = overview(&app, &caller, filtered_request.clone())
        .await
        .unwrap();
    assert_eq!(filtered.total, 1);
    assert_eq!(
        filtered.rows[0].chat_id.as_deref(),
        Some("discovery-cap-visible-c")
    );
    let operator = super::scope::operator_caller(&app).await.unwrap();
    let operator_error = overview(&app, &operator, request.clone())
        .await
        .unwrap_err();
    assert_eq!(operator_error.cause, error.cause);
    assert_eq!(operator_error.message, error.message);
    app.chat.sessions.write().await.remove("b");
    let without_denied = overview(&app, &caller, request.clone()).await.unwrap_err();
    let mut before = serde_json::to_value(&error).unwrap();
    let mut after = serde_json::to_value(&without_denied).unwrap();
    before.as_object_mut().unwrap().remove("request_id");
    after.as_object_mut().unwrap().remove("request_id");
    assert_eq!(before, after);
    for private in [
        "title b",
        "another private title",
        "secret",
        dir.path().to_str().unwrap(),
    ] {
        assert!(!before.to_string().contains(private));
        assert!(!operator_error.to_string().contains(private));
    }
    assert_eq!(
        overview(&app, &operator, request.clone())
            .await
            .unwrap()
            .total,
        2
    );
    let filtered_without_denied = overview(&app, &caller, filtered_request).await.unwrap();
    assert_eq!(filtered_without_denied.total, filtered.total);
    assert_eq!(filtered_without_denied.rows, filtered.rows);
}

#[cfg(unix)]
#[serial(coordination_work_overview)]
#[tokio::test]
async fn frozen_source_paths_are_reclassified_even_when_policy_and_recorded_zones_are_unchanged() {
    let (app, dir, caller) = fixture().await;
    let allowed = dir.path().join("allowed.rs");
    let denied = dir.path().join("private.rs");
    let alias = dir.path().join("alias.rs");
    std::fs::write(&allowed, "public").unwrap();
    std::fs::write(&denied, "private").unwrap();
    std::os::unix::fs::symlink(&allowed, &alias).unwrap();
    app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
        zones: vec![refact_privacy::Zone {
            name: "private".into(),
            patterns: vec![denied.to_string_lossy().into_owned()],
            send_to: vec!["other".into()],
            on_shell_read: refact_privacy::ShellBehavior::Withhold,
        }],
        ..Default::default()
    });
    let mut message = crate::call_validation::ChatMessage::new("user".into(), "source".into());
    crate::privacy::records::merge_records(
        &mut message,
        [refact_privacy::FileRecord {
            path: alias.to_string_lossy().into_owned(),
            zone: "normal".into(),
            attribution: refact_privacy::Attribution::Declared,
        }],
    );
    app.chat
        .sessions
        .read()
        .await
        .get("b")
        .cloned()
        .unwrap()
        .lock()
        .await
        .add_message(message);
    let request = WorkOverviewRequest {
        rows: Some(1),
        ..Default::default()
    };
    let first = overview(&app, &caller, request.clone()).await.unwrap();
    assert_eq!(first.total, 3);
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&denied, &alias).unwrap();
    let error = overview(
        &app,
        &caller,
        WorkOverviewRequest {
            cursor: first.next_cursor,
            ..request
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "cursor_expired");
    assert!(!serde_json::to_string(&error).unwrap().contains("alias.rs"));
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
    assert_eq!(
        error.cause,
        Some(super::error::WorkFailureCause::PolicyUnavailable)
    );
    assert!(error
        .message
        .contains("restore a valid policy and model selection"));
    assert!(!error.to_string().contains("private config"));
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn frozen_pages_revoke_removed_unknown_or_newly_private_sources_without_counts() {
    for change in ["removed", "unknown", "private", "lost_provenance", "policy"] {
        let (app, dir, caller) = fixture().await;
        let secret = dir.path().join("secret.rs");
        std::fs::write(&secret, "source").unwrap();
        let session = app.chat.sessions.read().await.get("b").cloned().unwrap();
        let mut source = crate::call_validation::ChatMessage::new("user".into(), "source".into());
        source.message_id = "source".into();
        crate::privacy::records::merge_records(
            &mut source,
            [refact_privacy::FileRecord {
                path: secret.to_string_lossy().into_owned(),
                zone: "normal".into(),
                attribution: refact_privacy::Attribution::Declared,
            }],
        );
        if change == "lost_provenance" {
            session.lock().await.add_message(source.clone());
        }
        let request = WorkOverviewRequest {
            rows: Some(1),
            ..Default::default()
        };
        let first = overview(&app, &caller, request.clone()).await.unwrap();
        match change {
            "removed" => {
                app.chat.sessions.write().await.remove("b");
            }
            "unknown" => {
                source
                    .extra
                    .insert("privacy".into(), serde_json::json!({"files":false}));
                session.lock().await.add_message(source);
            }
            "lost_provenance" => {
                session.lock().await.remove_message("source");
            }
            _ => {
                app.gcx.privacy_policy_load.write().unwrap().policy =
                    Arc::new(refact_privacy::PrivacyPolicy {
                        zones: vec![refact_privacy::Zone {
                            name: "secret".into(),
                            patterns: vec![secret.to_string_lossy().into_owned()],
                            send_to: vec!["other".into()],
                            on_shell_read: refact_privacy::ShellBehavior::Withhold,
                        }],
                        ..Default::default()
                    });
                if change == "private" {
                    session.lock().await.add_message(source);
                }
            }
        }
        let error = overview(
            &app,
            &caller,
            WorkOverviewRequest {
                cursor: first.next_cursor,
                ..request
            },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error.code, "cursor_expired" | "invalid_cursor"),
            "{change}: {error}"
        );
        let json = serde_json::to_string(&error).unwrap();
        for value in ["title b", "secret.rs", "total", "rows", "source"] {
            assert!(!json.contains(value), "{change}: {json}");
        }
    }
}

#[serial(coordination_work_overview)]
#[tokio::test]
async fn inspection_freezes_detail_and_file_matches_and_carries_selected_path_provenance() {
    let (app, dir, caller) = fixture().await;
    let a = app.chat.sessions.read().await.get("a").cloned().unwrap();
    let b = app.chat.sessions.read().await.get("b").cloned().unwrap();
    let c = app.chat.sessions.read().await.get("c").cloned().unwrap();
    b.lock().await.thread.parent_id = Some("a".into());
    c.lock().await.thread.parent_id = Some("a".into());
    let request = InspectRequest {
        selector: InspectSelector::Work {
            work_id: "a".into(),
        },
        cursor: None,
        limit: Some(1),
    };
    let first = inspect(&app, &caller, request.clone()).await.unwrap();
    a.lock().await.thread.title = "changed parent".into();
    let next = inspect(
        &app,
        &caller,
        InspectRequest {
            cursor: first.page.next_cursor.clone(),
            ..request.clone()
        },
    )
    .await
    .unwrap();
    assert_eq!(next.row, first.row);
    assert!(next.page.snapshot_stale);
    assert_ne!(next.page.rows[0].key, first.page.rows[0].key);
    assert_eq!(
        inspect(&app, &caller, request)
            .await
            .unwrap()
            .row
            .unwrap()
            .title
            .as_deref(),
        Some("changed parent")
    );
    let path = dir.path().join("read.rs");
    std::fs::write(&path, "source").unwrap();
    for id in ["a", "b"] {
        crate::file_activity::record_read(Some(id), &path, "source", 1, 1, 1, None);
    }
    let request = InspectRequest {
        selector: InspectSelector::Files {
            paths: vec!["read.rs".into()],
        },
        cursor: None,
        limit: Some(1),
    };
    let files = inspect(&app, &caller, request.clone()).await.unwrap();
    crate::file_activity::record_read(Some("c"), &path, "source", 1, 1, 1, None);
    let next = inspect(
        &app,
        &caller,
        InspectRequest {
            cursor: files.page.next_cursor.clone(),
            ..request
        },
    )
    .await
    .unwrap();
    assert_eq!(next.page.total, 2);
    assert!(next.page.snapshot_stale);
    assert_ne!(next.page.rows[0].chat_id.as_deref(), Some("c"));
    let mut message = crate::call_validation::ChatMessage::new("tool".into(), "files".into());
    attach_provenance(&app, &mut message, &next.page, None).unwrap();
    assert!(serde_json::to_string(&message.extra)
        .unwrap()
        .contains("read.rs"));
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
    let operator = inspect(&app, &caller, request.clone()).await.unwrap();
    assert!(operator
        .page
        .degraded
        .iter()
        .any(|source| source == "board index unavailable"));
    app.gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clear();
    let operator_error = inspect(&app, &caller, request.clone()).await.unwrap_err();
    caller.model = Some(crate::caps::BaseModelRecord {
        id: "trusted/model".into(),
        ..Default::default()
    });
    let model_error = inspect(&app, &caller, request.clone()).await.unwrap_err();
    assert_eq!(
        operator_error.cause,
        Some(super::error::WorkFailureCause::ScopedSourceUnavailable)
    );
    assert_eq!(
        model_error.cause,
        Some(super::error::WorkFailureCause::ProjectionUnavailable)
    );
    assert!(operator_error
        .message
        .contains("project is still open and accessible"));
    assert!(model_error.message.contains("narrow scope or filters"));
    let public = serde_json::to_value(&model_error).unwrap();
    assert_eq!(public["cause"], "projection_unavailable");
    assert_eq!(public.as_object().unwrap().len(), 5);
    for private in ["task-corrupt", "board.yaml", dir.path().to_str().unwrap()] {
        assert!(!operator_error.to_string().contains(private));
        assert!(!model_error.to_string().contains(private));
    }
    tokio::fs::remove_file(task_dir.join("board.yaml"))
        .await
        .unwrap();
    let absent = inspect(&app, &caller, request).await.unwrap_err();
    assert_eq!(absent.cause, model_error.cause);
    assert_eq!(absent.code, model_error.code);
    assert_eq!(absent.message, model_error.message);
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
