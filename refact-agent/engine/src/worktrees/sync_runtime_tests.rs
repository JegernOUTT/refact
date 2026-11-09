use std::sync::Arc;
use tokio::sync::Mutex;
use serde_json::{json, Value};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::worktrees::{sync_authority, service::WorktreeService};
use crate::worktrees::sync_runtime::status_readiness;
use crate::chat::types::ChatSession;
use refact_runtime_api::SessionState;

struct Fixture {
    _temp: tempfile::TempDir,
    source: std::path::PathBuf,
    worktree: crate::worktrees::types::WorktreeMeta,
    session: Arc<Mutex<ChatSession>>,
    ccx: Arc<Mutex<AtCommandsContext>>,
}

async fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("repo");
    std::fs::create_dir(&source).unwrap();
    let source = dunce::canonicalize(&source).unwrap();
    let git =
        |root: &std::path::Path, args: &[&str]| crate::worktrees::git::run_git(root, args).unwrap();
    git(&source, &["init"]);
    git(&source, &["checkout", "-b", "main"]);
    git(&source, &["config", "user.email", "test@example.invalid"]);
    git(&source, &["config", "user.name", "Test"]);
    std::fs::write(source.join("file"), "base\n").unwrap();
    git(&source, &["add", "."]);
    git(&source, &["commit", "-m", "base"]);
    let cache = temp.path().join("cache");
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        cache.clone(),
        temp.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.chat_models.insert(
        "test/model".into(),
        Arc::new(crate::caps::ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                id: "test/model".into(),
                n_ctx: 200_000,
                ..Default::default()
            },
            supports_tools: true,
            supports_agent: true,
            ..Default::default()
        }),
    );
    let (project_root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
    {
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.loaded_project_root = Some(project_root);
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let service = WorktreeService::new(cache, source.clone()).unwrap();
    let worktree = service
        .create_chat_worktree("sync-root")
        .await
        .unwrap()
        .worktree
        .meta;
    let mut session = ChatSession::new("sync-root".into());
    session.thread.model = "test/model".into();
    session.work_project_root = Some(source.clone());
    session.thread.worktree = Some(worktree.clone());
    session.set_runtime_state(SessionState::ExecutingTools, None);
    let abort = session.abort_flag.clone();
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert("sync-root".into(), session.clone());
    let ccx = AtCommandsContext::new_with_abort(
        app,
        4096,
        10,
        false,
        vec![],
        "sync-root".into(),
        None,
        "test/model".into(),
        None,
        Some(worktree.clone()),
        Some(abort),
    )
    .await;
    Fixture {
        _temp: temp,
        source,
        worktree,
        session,
        ccx: Arc::new(Mutex::new(ccx)),
    }
}

async fn http(f: &Fixture, request: Value) -> Result<Value, String> {
    use tower::ServiceExt;
    let app = f.ccx.lock().await.app.clone();
    let router = crate::http::routers::v1::make_v1_router(app.clone()).with_state(app);
    let response = router
        .oneshot(
            hyper::Request::builder()
                .method("POST")
                .uri(format!("/worktrees/{}/sync", f.worktree.id))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    if status.is_success() {
        Ok(value)
    } else {
        Err(value.to_string())
    }
}

#[tokio::test]
async fn readiness_tracks_real_writers_and_bounds_private_diagnostics() {
    let f = fixture().await;
    let app = f.ccx.lock().await.app.clone();
    let service = WorktreeService::new(app.gcx.cache_dir.clone(), f.source.clone()).unwrap();
    let own = status_readiness(&app, &service, &f.worktree.id, Some("chat:sync-root"))
        .await
        .unwrap();
    assert!(own.ready);
    let (release, pending) = tokio::sync::oneshot::channel::<()>();
    app.agent_executions
        .spawn_owned(
            "finishing-writer".into(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Arc::new(tokio::sync::Notify::new()),
            async {
                let _ = pending.await;
            },
        )
        .unwrap();
    let refs = vec![crate::worktrees::types::WorktreeReference {
        chat_id: Some("finishing-writer".into()),
        ..Default::default()
    }];
    assert!(app.agent_executions.task_active("finishing-writer"));
    let finishing =
        sync_authority::source_writer_readiness(&app, &f.worktree, &refs, Some("chat:sync-root"))
            .await
            .unwrap();
    assert!(!finishing.ready);
    assert!(matches!(
        finishing.blockers[0].kind,
        sync_authority::WriterBlockerKind::ExecutionActive
    ));
    assert!(sync_authority::ensure_source_quiescent(
        &app,
        &f.worktree,
        &refs,
        Some("chat:sync-root")
    )
    .await
    .is_err());
    release.send(()).unwrap();
    app.agent_executions.shutdown().await;
    let user = http(&f, json!({"action":"status"})).await.unwrap();
    assert_eq!(user["status"], "idle");
    assert_eq!(user["writer_readiness"]["ready"], false);
    assert_eq!(
        user["writer_readiness"]["blockers"][0]["kind"],
        "chat_active"
    );
    assert_eq!(
        user["writer_readiness"]["blockers"][0]["chat_id"],
        "sync-root"
    );
    for i in 0..35 {
        let mut writer = ChatSession::new(format!("foreign-{i:02}"));
        writer.thread.worktree = Some(f.worktree.clone());
        writer.set_runtime_state(SessionState::Generating, None);
        app.chat
            .sessions
            .write()
            .await
            .insert(writer.chat_id.clone(), Arc::new(Mutex::new(writer)));
    }
    let status = http(&f, json!({"action":"status"})).await.unwrap();
    let readiness = &status["writer_readiness"];
    assert_eq!(readiness["ready"], false);
    assert_eq!(readiness["truncated"], true);
    assert_eq!(readiness["blockers"].as_array().unwrap().len(), 32);
    assert!(sync_authority::ensure_source_quiescent(
        &app,
        &f.worktree,
        &[],
        Some("chat:sync-root")
    )
    .await
    .is_err());
}

#[tokio::test]
async fn http_start_refuses_busy_writer_and_continues_user_conflict() {
    let f = fixture().await;
    conflict(&f);
    let status = http(&f, json!({"action":"status"})).await.unwrap();
    let request = json!({"action":"start","expected_source_head":status["source_head"],"expected_target_head":status["target_head"]});
    let before =
        crate::worktrees::git::run_git(&f.worktree.root, &["status", "--porcelain=v1"]).unwrap();
    let error = http(&f, request.clone()).await.unwrap_err();
    assert!(error.contains("writer"), "{error}");
    assert_eq!(
        before,
        crate::worktrees::git::run_git(&f.worktree.root, &["status", "--porcelain=v1"]).unwrap()
    );
    assert!(http(&f, json!({"action":"status"})).await.unwrap()["operation"].is_null());
    f.session
        .lock()
        .await
        .set_runtime_state(SessionState::Idle, None);
    let started = http(&f, request).await.unwrap();
    assert_eq!(started["status"], "conflicts", "{started}");
    let op = &started["operation"];
    let resolved = http(&f, json!({"action":"resolve","operation_id":op["operation_id"],"expected_revision":op["revision"],"resolutions":[{"path":"file","choice":"source"}]})).await.unwrap();
    let op = &resolved["operation"];
    let done = http(&f, json!({"action":"continue","operation_id":op["operation_id"],"expected_revision":op["revision"]})).await.unwrap();
    assert_eq!(done["status"], "completed", "{done}");
}

#[tokio::test]
async fn http_foreign_attached_writer_refuses_and_missing_root_recovers() {
    let f = fixture().await;
    conflict(&f);
    f.session
        .lock()
        .await
        .set_runtime_state(SessionState::Idle, None);
    let app = f.ccx.lock().await.app.clone();
    let mut foreign = ChatSession::new("foreign".into());
    foreign.thread.worktree = Some(f.worktree.clone());
    foreign.set_runtime_state(SessionState::Generating, None);
    app.chat
        .sessions
        .write()
        .await
        .insert("foreign".into(), Arc::new(Mutex::new(foreign)));
    let status = http(&f, json!({"action":"status"})).await.unwrap();
    let request = json!({"action":"start","expected_source_head":status["source_head"],"expected_target_head":status["target_head"]});
    assert!(http(&f, request.clone())
        .await
        .unwrap_err()
        .contains("writer"));
    app.chat.sessions.write().await.remove("foreign");
    let started = http(&f, request).await.unwrap();
    assert_eq!(started["status"], "conflicts");
    let held = f.worktree.root.with_extension("held");
    std::fs::rename(&f.worktree.root, &held).unwrap();
    let status = http(&f, json!({"action":"status"})).await.unwrap();
    assert_eq!(status["status"], "recovery_required", "{status}");
    let recovered = http(
        &f,
        json!({"action":"recover","expected_journal_digest":status["recovery_journal_digest"]}),
    )
    .await
    .unwrap();
    assert_eq!(recovered["status"], "aborted", "{recovered}");
    assert!(held.join("file").exists());
}

fn conflict(fixture: &Fixture) {
    let git =
        |root: &std::path::Path, args: &[&str]| crate::worktrees::git::run_git(root, args).unwrap();
    std::fs::write(fixture.worktree.root.join("file"), "source\n").unwrap();
    git(&fixture.worktree.root, &["commit", "-am", "source"]);
    std::fs::write(fixture.source.join("file"), "target\n").unwrap();
    git(&fixture.source, &["commit", "-am", "target"]);
}
