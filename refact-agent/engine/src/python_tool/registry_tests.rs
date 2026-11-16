use super::*;
use crate::chat::types::ChatSession;
use refact_python_kernel::protocol::RunLimits;

async fn fixture(limit: usize) -> (tempfile::TempDir, AppState, KernelRegistry) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(root.join(".refact")).unwrap();
    std::fs::write(
        root.join(".refact/coordination.yaml"),
        format!("python_max_live_sessions: {limit}\n"),
    )
    .unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root];
    let app = AppState::from_gcx(gcx).await;
    for id in ["a", "b", "c"] {
        app.chat.sessions.write().await.insert(
            id.into(),
            Arc::new(AsyncMutex::new(ChatSession::new(id.into()))),
        );
    }
    (dir, app, KernelRegistry::new(WorkerBackend::InThread))
}
async fn feed(lease: &mut SessionLease, code: &str) -> Value {
    lease
        .client()
        .send(KernelCommand::Feed {
            run_id: "run".into(),
            code: code.into(),
            globals: vec![],
            limits: RunLimits::default(),
        })
        .await
        .unwrap();
    loop {
        match event(lease.client()).await.unwrap() {
            KernelEvent::Completed {
                value, ..
            } => return value,
            KernelEvent::Stdout {
                ..
            } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
}
#[tokio::test]
async fn same_message_runs_execute_in_call_order() {
    let registry = KernelRegistry::new(WorkerBackend::InThread);
    let first = registry.reserve_turn("a", "1");
    let second = registry.reserve_turn("a", "2");
    let third = registry.reserve_turn("a", "3");
    let mut second = Box::pin(second.acquire());
    assert!(futures::poll!(&mut second).is_pending());
    let first = first.acquire().await.unwrap();
    drop(first);
    let second = second.await.unwrap();
    let mut third = Box::pin(third.acquire());
    assert!(futures::poll!(&mut third).is_pending());
    drop(second);
    drop(third.await.unwrap());
}
#[tokio::test]
async fn second_run_is_refused_while_earlier_run_is_in_background() {
    let registry = KernelRegistry::default();
    let first = registry.reserve_turn("a", "1").acquire().await.unwrap();
    let second = registry.reserve_turn("a", "2");
    first.mark_background("job");
    let busy = second.acquire().await.err().unwrap();
    assert_eq!(busy.job_id, "job");
    assert!(busy.to_string().contains("tool_job"));
    assert!(registry.reserve_turn("a", "3").acquire().await.is_err());
    drop(first);
    assert!(registry.reserve_turn("a", "4").acquire().await.is_ok());
}
#[tokio::test]
async fn lru_evicts_idle_sessions_beyond_limit() {
    let (_dir, app, registry) = fixture(1).await;
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 42").await;
    drop(a);
    let mut b = registry.reserve_turn("b", "2").acquire().await.unwrap();
    b.ensure_ready(&app, false).await.unwrap();
    assert!(registry.0.slots.lock().unwrap()["a"]
        .session
        .try_lock()
        .unwrap()
        .client
        .is_none());
}
#[tokio::test]
async fn evicted_session_restores_from_snapshot() {
    let (_dir, app, registry) = fixture(1).await;
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 42").await;
    drop(a);
    let mut b = registry.reserve_turn("b", "2").acquire().await.unwrap();
    b.ensure_ready(&app, false).await.unwrap();
    drop(b);
    let mut a = registry.reserve_turn("a", "3").acquire().await.unwrap();
    assert!(a.ensure_ready(&app, false).await.unwrap().restored);
    assert_eq!(feed(&mut a, "x").await, 42);
}
#[tokio::test]
async fn tampered_snapshot_resets_session_with_reason() {
    let (_dir, app, registry) = fixture(2).await;
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 42").await;
    a.persist(&app, "run", vec![]).await.unwrap();
    let workspace = PythonWorkspace::for_chat(&app, "a").await.unwrap();
    tokio::fs::write(workspace.state_path(), b"bad")
        .await
        .unwrap();
    a.discard_worker("test").await;
    assert!(a
        .ensure_ready(&app, false)
        .await
        .unwrap()
        .reset_reason
        .is_some());
    assert_eq!(feed(&mut a, "'x' in locals()").await, false);
}
#[tokio::test]
async fn restart_clears_variables() {
    let (_dir, app, registry) = fixture(2).await;
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 42").await;
    a.persist(&app, "run", vec![]).await.unwrap();
    a.ensure_ready(&app, true).await.unwrap();
    assert_eq!(feed(&mut a, "'x' in locals()").await, false);
    assert!(!PythonWorkspace::for_chat(&app, "a")
        .await
        .unwrap()
        .state_path()
        .exists());
}
#[tokio::test]
async fn discarded_worker_restores_last_snapshot_on_next_run() {
    let (_dir, app, registry) = fixture(2).await;
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 42").await;
    a.persist(&app, "run", vec![]).await.unwrap();
    feed(&mut a, "x = 99").await;
    a.discard_worker("cancelled").await;
    assert!(a.ensure_ready(&app, false).await.unwrap().restored);
    assert_eq!(feed(&mut a, "x").await, 42);
}
#[tokio::test]
async fn shutdown_kills_all_sessions() {
    let (_dir, app, registry) = fixture(2).await;
    for id in ["a", "b"] {
        let mut lease = registry.reserve_turn(id, "1").acquire().await.unwrap();
        lease.ensure_ready(&app, false).await.unwrap();
    }
    registry.shutdown_all().await;
    for slot in registry.0.slots.lock().unwrap().values() {
        assert!(slot.session.try_lock().unwrap().client.is_none());
    }
}
#[tokio::test]
async fn dropped_ticket_does_not_block_successors() {
    let registry = KernelRegistry::default();
    let first = registry.reserve_turn("a", "1");
    let second = registry.reserve_turn("a", "2");
    drop(first);
    assert!(second.acquire().await.is_ok());
}

#[tokio::test]
async fn deleting_chat_drops_worker_and_slot() {
    let (_dir, app, registry) = fixture(2).await;
    let mut lease = registry.reserve_turn("a", "first").acquire().await.unwrap();
    lease.ensure_ready(&app, false).await.unwrap();
    let waiting = registry.reserve_turn("a", "waiting");
    registry.drop_chat("a").await;
    assert!(!registry.0.slots.lock().unwrap().contains_key("a"));
    assert!(waiting.acquire().await.is_err());
    assert!(!registry.session_status("a").live);
}
