use super::*;
use serde_json::json;

#[tokio::test]
async fn worker_http_accepts_large_state_without_weakening_view_origin_or_cas() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let created = execute(
        &app,
        Caller::Host,
        Command::Create {
            slug: "large-http".into(),
            name: "Large HTTP".into(),
        },
    )
    .await
    .unwrap();
    let id = serde_json::from_value(created["manifest"]["app_id"].clone()).unwrap();
    execute(
        &app,
        Caller::Host,
        Command::Publish {
            app_id: id,
            expected_revision: created["revision"].as_str().unwrap().into(),
        },
    )
    .await
    .unwrap();
    let view = execute(
        &app,
        Caller::Host,
        Command::ViewRegister {
            app_id: id,
            chat_id: None,
            binding_id: None,
        },
    )
    .await
    .unwrap();
    let value = json!({"history": "metric-42;".repeat(2 * 1024 * 1024)});
    let command = json!({
        "action": "state_set", "app_id": id, "scope": {"type": "standalone"},
        "view_id": view["view_id"], "view_token": view["view_token"],
        "expected_revision": 0, "value": value,
    });
    let bytes = serde_json::to_vec(&command).unwrap();
    assert!(bytes.len() > 15 * 1024 * 1024);
    let router = crate::http::routers::make_refact_http_server(app.clone());
    let request = |bytes: Vec<u8>, origin: &str| {
        Request::builder()
            .method("POST")
            .uri("/v1/apps/commands")
            .header("content-type", "application/json")
            .header("host", "localhost:8001")
            .header("origin", origin)
            .body(Body::from(bytes))
            .unwrap()
    };
    let response = router
        .clone()
        .oneshot(request(bytes.clone(), "http://localhost:8001"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let result: serde_json::Value =
        serde_json::from_slice(&hyper::body::to_bytes(response.into_body()).await.unwrap())
            .unwrap();
    assert_eq!(result, json!({"revision": 1, "value": value}));

    let conflict = router
        .clone()
        .oneshot(request(bytes.clone(), "http://localhost:8001"))
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let denied = router
        .clone()
        .oneshot(request(bytes, "null"))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let mut wrong_view = command;
    wrong_view["expected_revision"] = json!(1);
    wrong_view["view_token"] = json!("wrong");
    let denied = router
        .oneshot(request(
            serde_json::to_vec(&wrong_view).unwrap(),
            "http://localhost:8001",
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);
    let saved = runtime(&app, &Caller::Host)
        .await
        .unwrap()
        .store
        .read_state("large-http", refact_apps_core::StateScope::Standalone)
        .await
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.value, value);
    app.gcx.apps.shutdown().await;
}

#[tokio::test]
async fn shutdown_refuses_new_runtime_admission() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let host = gcx
        .apps
        .get(first.path().into(), gcx.clone())
        .await
        .unwrap();
    gcx.shutdown_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);
    gcx.apps.shutdown().await;
    let result = gcx.apps.get(second.path().into(), gcx.clone()).await;
    let refused = matches!(&result, Err(error) if error.contains("shut"));
    gcx.apps.shutdown().await;
    drop(host);
    assert!(refused, "registry admitted a runtime after shutdown");
}

#[tokio::test]
async fn published_document_and_view_state_use_immutable_bundle_and_exact_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let created = execute(
        &app,
        Caller::Host,
        Command::Create {
            slug: "demo".into(),
            name: "Demo".into(),
        },
    )
    .await
    .unwrap();
    let id = serde_json::from_value(created["manifest"]["app_id"].clone()).unwrap();
    execute(
        &app,
        Caller::Host,
        Command::Publish {
            app_id: id,
            expected_revision: created["revision"].as_str().unwrap().into(),
        },
    )
    .await
    .unwrap();
    execute(
        &app,
        Caller::Host,
        Command::WriteFile {
            app_id: id,
            path: "ui/index.html".into(),
            content: "draft only".into(),
            expected_revision: created["revision"].as_str().unwrap().into(),
        },
    )
    .await
    .unwrap();
    let doc = document(&app, id, None, None).await.unwrap();
    assert!(!doc["html"].as_str().unwrap().contains("draft only"));
    let view = execute(
        &app,
        Caller::Host,
        Command::ViewRegister {
            app_id: id,
            chat_id: None,
            binding_id: None,
        },
    )
    .await
    .unwrap();
    let view_id = serde_json::from_value(view["view_id"].clone()).unwrap();
    let command = |token: String| Command::StateSet {
        app_id: id,
        scope: refact_apps_core::StateScope::Standalone,
        chat_id: None,
        view_id: Some(view_id),
        view_token: Some(token),
        expected_revision: 0,
        value: json!({"ok":true}),
    };
    assert!(execute(&app, Caller::Host, command("wrong".into()))
        .await
        .is_err());
    let state = execute(
        &app,
        Caller::Host,
        command(view["view_token"].as_str().unwrap().into()),
    )
    .await
    .unwrap();
    assert_eq!(state["revision"], 1);
    assert!(execute(
        &app,
        Caller::Host,
        command(view["view_token"].as_str().unwrap().into())
    )
    .await
    .unwrap_err()
    .contains("conflict"));
    app.gcx.apps.shutdown().await;
}

#[tokio::test]
async fn registered_worktree_uses_project_catalog_and_binding() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let source = source.canonicalize().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
    let (root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.chat_models.insert(
        "test/model".into(),
        Arc::new(crate::caps::ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                id: "test/model".into(),
                ..Default::default()
            },
            ..Default::default()
        }),
    );
    {
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.loaded_project_root = Some(root);
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let service = crate::worktrees::service::WorktreeService::new_async(
        app.gcx.cache_dir.clone(),
        source.clone(),
    )
    .await
    .unwrap();
    let old = service.load_registry().await.unwrap();
    let mut registry = old.clone();
    let checkout = service.worktree_path_for_id("test-wt").unwrap();
    std::fs::create_dir_all(&checkout).unwrap();
    let meta = crate::worktrees::types::WorktreeMeta {
        id: "test-wt".into(),
        kind: "chat".into(),
        root: checkout.clone(),
        source_workspace_root: source.clone(),
        repo_root: source.clone(),
        branch: None,
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
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            last_seen_at: None,
            references: vec![],
            last_known_status: None,
        });
    service
        .compare_and_save_registry(&old, &registry)
        .await
        .unwrap();
    let created = execute(
        &app,
        Caller::Host,
        Command::Create {
            slug: "shared".into(),
            name: "Shared".into(),
        },
    )
    .await
    .unwrap();
    let id: uuid::Uuid = serde_json::from_value(created["manifest"]["app_id"].clone()).unwrap();
    execute(
        &app,
        Caller::Host,
        Command::Publish {
            app_id: id,
            expected_revision: created["revision"].as_str().unwrap().into(),
        },
    )
    .await
    .unwrap();
    let binding = uuid::Uuid::new_v4();
    let mut session = crate::chat::types::ChatSession::new("worktree-chat".into());
    session.thread.model = "test/model".into();
    session.work_project_root = Some(source.clone());
    session.thread.worktree = Some(meta);
    session.thread.app_workspace = Some(serde_json::from_value(json!({"schema_version":1,"revision":1,"bindings":[{"binding_id":binding,"app_id":id}],"layout":{"kind":"leaf","tabs":[{"kind":"chat","ref":"self"},{"kind":"app","binding_id":binding}],"active":0}})).unwrap());
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert("worktree-chat".into(), session.clone());
    let host = runtime(&app, &Caller::Host).await.unwrap();
    let caller = Caller::HostChat {
        chat_id: "worktree-chat".into(),
    };
    let worktree = runtime(&app, &caller).await.unwrap();
    assert!(Arc::ptr_eq(&host, &worktree));
    assert_eq!(
        document(&app, id, Some("worktree-chat".into()), Some(binding))
            .await
            .unwrap()["app_id"],
        json!(id)
    );
    let view = execute(
        &app,
        caller,
        Command::ViewRegister {
            app_id: id,
            chat_id: Some("worktree-chat".into()),
            binding_id: Some(binding),
        },
    )
    .await
    .unwrap();
    let view_id = serde_json::from_value(view["view_id"].clone()).unwrap();
    let set = || Command::StateSet {
        app_id: id,
        scope: refact_apps_core::StateScope::Binding {
            binding_id: binding,
        },
        chat_id: Some("worktree-chat".into()),
        view_id: Some(view_id),
        view_token: Some(view["view_token"].as_str().unwrap().into()),
        expected_revision: 0,
        value: json!({"shared": 7}),
    };
    assert_eq!(
        execute(&app, Caller::Host, set()).await.unwrap()["revision"],
        1
    );
    assert!(execute(&app, Caller::Host, set())
        .await
        .unwrap_err()
        .contains("conflict"));
    assert_eq!(
        document(&app, id, Some("worktree-chat".into()), Some(binding))
            .await
            .unwrap()["state"]["value"],
        json!({"shared": 7})
    );
    let details = execute(
        &app,
        Caller::Host,
        Command::Details {
            app_id: id,
        },
    )
    .await
    .unwrap();
    assert!(details["chats"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["chat_id"] == "worktree-chat" && v["binding_ids"] == json!([binding])));
    let listed = execute(
        &app,
        Caller::HostChat {
            chat_id: "worktree-chat".into(),
        },
        Command::List {},
    )
    .await
    .unwrap();
    assert!(listed["apps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["manifest"]["app_id"] == json!(id)));

    // The process adapter pins the registered worktree, not the project root.
    let process = app
        .gcx
        .exec_registry
        .register(
            crate::exec::ExecProcessMeta::new(crate::exec::ExecMode::Background, "test".into())
                .with_chat_id("worktree-chat")
                .with_workspace(checkout.clone()),
            1024,
        )
        .await;
    let process_def = refact_apps_core::SourceDefinition {
        source_id: "process".into(),
        channel: "logs".into(),
        driver: refact_apps_core::SourceDriver::Process {
            process_id: process.meta.process_id.to_string(),
            chat_id: Some("worktree-chat".into()),
        },
        permissions: vec![refact_apps_core::Permission::Process],
    };
    host.automation
        .configure(
            &id.to_string(),
            "process-revision".into(),
            vec![process_def.clone()],
        )
        .await
        .unwrap();
    host.process_sources
        .sync(
            &id.to_string(),
            "process-revision",
            &[process_def.clone()],
            "worktree-chat",
        )
        .await
        .unwrap();
    {
        let mut owner = session.lock().await;
        owner.thread.worktree = None;
    }
    // Same project but different checkout: old process cannot be reauthorized.
    assert!(host
        .process_sources
        .sync(
            &id.to_string(),
            "process-revision",
            &[process_def],
            "worktree-chat"
        )
        .await
        .is_err());
    session.lock().await.thread.worktree = Some(registry.records[0].meta.clone());

    // A host-approved source and feed survive lookup through another checkout.
    let def = refact_apps_core::SourceDefinition {
        source_id: "input".into(),
        channel: "display".into(),
        driver: refact_apps_core::SourceDriver::Internal {
            topic: "metrics".into(),
        },
        permissions: vec![refact_apps_core::Permission::Internal {
            topic: "metrics".into(),
        }],
    };
    host.automation
        .configure(&id.to_string(), "test-revision".into(), vec![def])
        .await
        .unwrap();
    host.automation
        .set_grant(&id.to_string(), "input", true)
        .await
        .unwrap();
    host.automation
        .set_demand(&id.to_string(), "input", "test", true)
        .await
        .unwrap();
    host.automation
        .publish_internal("metrics", json!({"reading": 42}))
        .await
        .unwrap();
    let feed = execute(
        &app,
        Caller::HostChat {
            chat_id: "worktree-chat".into(),
        },
        Command::SourceRead {
            app_id: id,
            source_id: "input".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(feed["data"], json!({"reading":42}));
    assert_eq!(
        host.automation.snapshot(&id.to_string()).await[0].data,
        Some(json!({"reading":42}))
    );

    // The actual source-project app path, not the checkout-local path, controls model egress.
    let tool = Caller::Tool {
        chat_id: "worktree-chat".into(),
        model: Some("test/model".into()),
    };
    let records = preflight_tool_records(&app, &tool).await.unwrap();
    assert!(records.iter().any(|record| record
        .path
        .starts_with(source.join(".refact/apps/shared").to_str().unwrap())));
    app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
        zones: vec![refact_privacy::Zone {
            name: "private-app".into(),
            patterns: vec![format!("{}/**", source.join(".refact/apps").display())],
            send_to: vec!["other".into()],
            on_shell_read: refact_privacy::ShellBehavior::Withhold,
        }],
        ..Default::default()
    });
    assert!(
        execute(&app, tool, Command::List {}).await.is_err(),
        "private source project catalog reached model"
    );

    // A different open project cannot use this project's binding even with the exact UUIDs.
    let foreign = dir.path().join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    app.gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .push(foreign.clone());
    {
        let mut session = session.lock().await;
        session.thread.worktree = None;
        session.work_project_root = Some(foreign);
    }
    assert!(
        validate_binding(&app, &host, id, Some("worktree-chat"), Some(binding))
            .await
            .is_err()
    );
    {
        let mut session = session.lock().await;
        session.thread.worktree = Some(registry.records[0].meta.clone());
        session.work_project_root = Some(source.clone());
    }
    assert!(document(
        &app,
        id,
        Some("worktree-chat".into()),
        Some(uuid::Uuid::new_v4())
    )
    .await
    .is_err());
    let mut removed = registry.clone();
    removed.records.clear();
    service
        .compare_and_save_registry(&registry, &removed)
        .await
        .unwrap();
    assert!(
        document(&app, id, Some("worktree-chat".into()), Some(binding))
            .await
            .is_err()
    );
    app.gcx.apps.shutdown().await;
}
