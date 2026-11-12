use super::{store::*, types::*};
#[test]
fn organization_setting_patch_preserves_other_background_permissions() {
    let mut settings = crate::buddy::settings::BuddySettings::default();
    let request: crate::http::routers::v1::buddy::BuddySettingsRequest =
        serde_json::from_value(serde_json::json!({"organization_enabled": false})).unwrap();
    request.apply_to_settings(&mut settings);
    assert!(!settings.organization_enabled);
    assert!(settings.autonomous_chats_enabled);
    assert!(settings.housekeeping_enabled);
}

#[tokio::test]
async fn service_rejects_foreign_children_and_clears_derived_taxonomy_provenance() {
    use crate::{app_state::AppState, coordination::scope::WorkCaller, caps::BaseModelRecord};
    let dir = tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let foreign = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
    let app = AppState::from_gcx(gcx).await;
    for (id, root, parent) in [
        ("root", dir.path(), None),
        ("child", dir.path(), Some("root")),
        ("foreign", foreign.path(), None),
    ] {
        let mut session = crate::chat::types::ChatSession::new(id.into());
        session.work_project_root = Some(root.into());
        session.thread.root_chat_id = parent.map(str::to_string);
        session.thread.parent_id = parent.map(str::to_string);
        app.chat.sessions.write().await.insert(
            id.into(),
            std::sync::Arc::new(tokio::sync::Mutex::new(session)),
        );
    }
    let caller = WorkCaller {
        source_root: dir.path().into(),
        checkout_root: dir.path().into(),
        root_chat_id: None,
        principal: "operator".into(),
        model: None,
    };
    let lookup = |id: &str| ItemsRequest {
        items: vec![Item {
            kind: ItemKind::Chat,
            id: id.into(),
        }],
    };
    assert!(super::service::items(&app, &caller, lookup("root"))
        .await
        .is_ok());
    assert!(super::service::items(&app, &caller, lookup("child"))
        .await
        .is_err());
    assert!(super::service::items(&app, &caller, lookup("foreign"))
        .await
        .is_err());
    let path = dir.path().join("guarded.rs");
    std::fs::write(&path, "guarded").unwrap();
    app.gcx.privacy_policy_load.write().unwrap().policy =
        std::sync::Arc::new(refact_privacy::PrivacyPolicy {
            blocked: vec![path.to_string_lossy().to_string()],
            ..Default::default()
        });
    let records = crate::privacy::records::declared_file_records(&app.gcx, [path]).unwrap();
    merge_provenance(&Store::open(dir.path()).unwrap().conn, &records).unwrap();
    assert!(
        super::service::taxonomy(&app, &caller, TaxonomyKind::Topics, 200, None)
            .await
            .is_ok()
    );
    let model = WorkCaller {
        model: Some(BaseModelRecord {
            id: "other/model".into(),
            ..Default::default()
        }),
        ..caller
    };
    assert!(
        super::service::taxonomy(&app, &model, TaxonomyKind::Topics, 200, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn http_contract_checks_auth_limits_and_durable_revision() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let mut gcx = crate::global_context::tests::make_test_gcx().await;
    std::sync::Arc::get_mut(&mut gcx)
        .unwrap()
        .cmdline
        .daemon_auth_token = Some("organization-token".into());
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let router = crate::http::routers::v1::make_v1_router(app.clone()).with_state(app);
    let request = Request::builder()
        .uri("/organization/taxonomy?kind=topics")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.clone().oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let request=Request::builder().method("POST").uri("/organization/apply").header("authorization","Bearer organization-token").header("content-type","application/json").body(Body::from(r#"{"expected_revision":0,"request_id":"http","operations":[{"op":"create_topic","name":"Shipping"}]}"#)).unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&hyper::body::to_bytes(response.into_body()).await.unwrap())
            .unwrap();
    assert_eq!(body["revision"], 1);
    assert!(body["results"][0]["id"].is_string());
    let request = Request::builder()
        .uri("/organization/taxonomy?kind=topics&limit=201")
        .header("authorization", "Bearer organization-token")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.clone().oneshot(request).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    let request = Request::builder()
        .method("POST")
        .uri("/organization/apply")
        .header("authorization", "Bearer organization-token")
        .header("content-type", "application/json")
        .body(Body::from("x".repeat(128 * 1024 + 1)))
        .unwrap();
    assert_eq!(
        router.oneshot(request).await.unwrap().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[test]
fn cancelled_apply_rolls_back_revision_request_and_taxonomy() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let req = request(
        0,
        "cancelled",
        vec![Op::CreateTopic {
            name: "Cancelled".into(),
        }],
    );
    assert!(store
        .apply_guarded(&req, Authority::User, &[], || Err(Error::conflict(
            "cancelled"
        )))
        .is_err());
    assert_eq!(store.revision().unwrap(), 0);
    assert!(store
        .taxonomy(TaxonomyKind::Topics, 200, None)
        .unwrap()
        .items
        .is_empty());
    assert_eq!(store.apply(&req, Authority::User).unwrap().revision, 1);
}

#[tokio::test]
async fn registered_worktree_and_source_share_organization_revision() {
    use crate::{coordination::scope::WorkCaller, worktrees::service::WorktreeService};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let repo = git2::Repository::init(&source).unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
        .unwrap();
    drop(tree);
    drop(repo);
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let service = WorktreeService::new_async(app.gcx.cache_dir.clone(), source.clone())
        .await
        .unwrap();
    let created = service.create_chat_worktree("fixture-root").await.unwrap();
    let caller = WorkCaller {
        source_root: created.worktree.meta.root.clone(),
        checkout_root: created.worktree.meta.root.clone(),
        root_chat_id: None,
        principal: "operator".into(),
        model: None,
    };
    super::service::apply(
        &app,
        &caller,
        request(
            0,
            "shared",
            vec![Op::CreateTopic {
                name: "Shared project".into(),
            }],
        ),
    )
    .await
    .unwrap();
    assert!(!Store::path(&created.worktree.meta.root).exists());
    let source_caller = WorkCaller {
        source_root: source.clone(),
        checkout_root: source,
        ..caller
    };
    let result = super::service::taxonomy(&app, &source_caller, TaxonomyKind::Topics, 200, None)
        .await
        .unwrap();
    assert_eq!(result.revision, 1);
    assert_eq!(result.items[0].name, "Shared project");
}

fn request(revision: u64, id: &str, operations: Vec<Op>) -> ApplyRequest {
    ApplyRequest {
        expected_revision: revision,
        request_id: id.into(),
        operations,
    }
}
fn item() -> Item {
    Item {
        kind: ItemKind::Chat,
        id: "root".into(),
    }
}
#[test]
fn durable_replay_cas_and_atomic_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let create = request(
        0,
        "create",
        vec![
            Op::CreateTopic {
                name: "  Rust   project  ".into(),
            },
            Op::CreateTag {
                name: "Backend".into(),
            },
        ],
    );
    let response = store.apply(&create, Authority::User).unwrap();
    assert_eq!(response.revision, 1);
    let topic = response.results[0].id.clone().unwrap();
    let tag = response.results[1].id.clone().unwrap();
    drop(store);
    let mut store = Store::open(dir.path()).unwrap();
    assert_eq!(store.apply(&create, Authority::User).unwrap(), response);
    assert_eq!(
        store
            .taxonomy(TaxonomyKind::Topics, 200, None)
            .unwrap()
            .items[0]
            .name,
        "Rust project"
    );
    let mut conflict = create.clone();
    conflict.operations = vec![Op::CreateTag {
        name: "other".into(),
    }];
    assert_eq!(
        store.apply(&conflict, Authority::User).unwrap_err().code,
        "conflict"
    );
    assert_eq!(
        store
            .apply(
                &request(
                    0,
                    "stale",
                    vec![Op::CreateTag {
                        name: "other".into()
                    }]
                ),
                Authority::User
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    let bad = request(
        1,
        "bad",
        vec![
            Op::CreateTag {
                name: "rollback".into(),
            },
            Op::AssignItem {
                item: item(),
                topic_id: Some("absent".into()),
                tag_ids: vec![],
                topic_locked: None,
                tags_locked: None,
            },
        ],
    );
    assert!(store.apply(&bad, Authority::User).is_err());
    assert_eq!(store.revision().unwrap(), 1);
    assert_eq!(
        store
            .taxonomy(TaxonomyKind::Tags, 200, None)
            .unwrap()
            .items
            .len(),
        1
    );
    store
        .apply(
            &request(
                1,
                "assign",
                vec![Op::AssignItem {
                    item: item(),
                    topic_id: Some(topic.clone()),
                    tag_ids: vec![tag],
                    topic_locked: Some(true),
                    tags_locked: Some(true),
                }],
            ),
            Authority::User,
        )
        .unwrap();
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    let assignment = assignment(&store.conn, &item()).unwrap();
    assert_eq!(assignment.topic_id, Some(topic));
    assert!(assignment.topic_locked && assignment.tags_locked);
}
#[test]
fn manual_names_locks_archive_and_merge_are_protected() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let response = store
        .apply(
            &request(
                0,
                "create",
                vec![
                    Op::CreateTopic {
                        name: "manual".into(),
                    },
                    Op::CreateTopic {
                        name: "target".into(),
                    },
                ],
            ),
            Authority::User,
        )
        .unwrap();
    let topic = response.results[0].id.clone().unwrap();
    let target = response.results[1].id.clone().unwrap();
    assert!(store
        .apply(
            &request(
                1,
                "rename",
                vec![Op::RenameTopic {
                    id: topic.clone(),
                    name: "automatic".into()
                }]
            ),
            Authority::Model
        )
        .is_err());
    store
        .apply(
            &request(
                1,
                "lock",
                vec![Op::AssignItem {
                    item: item(),
                    topic_id: Some(topic.clone()),
                    tag_ids: vec![],
                    topic_locked: Some(true),
                    tags_locked: Some(true),
                }],
            ),
            Authority::User,
        )
        .unwrap();
    assert!(store
        .apply(
            &request(
                2,
                "replace",
                vec![Op::AssignItem {
                    item: item(),
                    topic_id: None,
                    tag_ids: vec![],
                    topic_locked: None,
                    tags_locked: None
                }]
            ),
            Authority::Model
        )
        .is_err());
    assert!(store
        .apply(
            &request(
                2,
                "merge",
                vec![Op::MergeTopic {
                    from_id: topic.clone(),
                    into_id: target
                }]
            ),
            Authority::User
        )
        .is_err());
    store
        .apply(
            &request(
                2,
                "archive",
                vec![Op::ArchiveTopic {
                    id: topic.clone(),
                }],
            ),
            Authority::User,
        )
        .unwrap();
    assert_eq!(
        assignment(&store.conn, &item()).unwrap().topic_id,
        Some(topic.clone())
    );
    assert!(store
        .apply(
            &request(
                3,
                "new",
                vec![Op::AssignItem {
                    item: Item {
                        kind: ItemKind::Task,
                        id: "task".into()
                    },
                    topic_id: Some(topic),
                    tag_ids: vec![],
                    topic_locked: None,
                    tags_locked: None
                }]
            ),
            Authority::User
        )
        .is_err());
}
#[test]
fn bounded_pages_bind_kind_project_and_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    store
        .apply(
            &request(
                0,
                "seed",
                vec![
                    Op::CreateTag {
                        name: "one".into(),
                    },
                    Op::CreateTag {
                        name: "two".into(),
                    },
                ],
            ),
            Authority::Model,
        )
        .unwrap();
    let first = store.taxonomy(TaxonomyKind::Tags, 1, None).unwrap();
    let cursor = first.next_cursor.unwrap();
    let second = store
        .taxonomy(TaxonomyKind::Tags, 1, Some(&cursor))
        .unwrap();
    assert_ne!(first.items[0].id, second.items[0].id);
    assert!(second.next_cursor.is_none());
    assert!(store
        .taxonomy(TaxonomyKind::Topics, 1, Some(&cursor))
        .is_err());
    let other = tempfile::tempdir().unwrap();
    assert!(Store::open(other.path())
        .unwrap()
        .taxonomy(TaxonomyKind::Tags, 1, Some(&cursor))
        .is_err());
    store
        .apply(
            &request(
                1,
                "change",
                vec![Op::CreateTag {
                    name: "third".into(),
                }],
            ),
            Authority::Model,
        )
        .unwrap();
    assert_eq!(
        store
            .taxonomy(TaxonomyKind::Tags, 1, Some(&cursor))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert!(store.taxonomy(TaxonomyKind::Tags, 201, None).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_and_hardlink_storage_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let foreign = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(foreign.path(), dir.path().join(".refact")).unwrap();
    assert!(Store::open(dir.path()).is_err());
    assert!(!foreign.path().join("organization.sqlite").exists());
    std::fs::remove_file(dir.path().join(".refact")).unwrap();
    drop(Store::open(dir.path()).unwrap());
    let db = Store::path(dir.path());
    std::fs::hard_link(&db, foreign.path().join("linked.sqlite")).unwrap();
    assert!(Store::open(dir.path()).is_err());
}
