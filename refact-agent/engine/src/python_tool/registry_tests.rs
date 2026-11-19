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
    tokio::fs::write(generation_path(&app, "a").await, b"bad")
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
    let ws = PythonWorkspace::for_chat(&app, "a").await.unwrap();
    assert!(generation_files(ws.dir()).is_empty());
    assert!(ws.read_state_meta().await.is_none());
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

async fn generation_path(app: &AppState, chat: &str) -> PathBuf {
    let ws = PythonWorkspace::for_chat(app, chat).await.unwrap();
    let meta: Value =
        serde_json::from_slice(&tokio::fs::read(ws.dir().join("state.json")).await.unwrap())
            .unwrap();
    ws.dir()
        .join(meta["generation"].as_str().unwrap_or("state.monty"))
}
async fn restart_engine(app: &AppState, chat: &str) -> (SessionLease, SessionStart) {
    let fresh = KernelRegistry::new(WorkerBackend::InThread);
    let mut lease = fresh.reserve_turn(chat, "after").acquire().await.unwrap();
    let start = lease.ensure_ready(app, false).await.unwrap();
    (lease, start)
}

#[tokio::test]
async fn an_interrupted_save_leaves_the_old_or_the_new_whole_generation() {
    for step in [
        "snapshot_done",
        "inspected",
        "manifest_written",
        "published",
    ] {
        for first in [true, false] {
            let case = format!("{step}, first save: {first}");
            let (_dir, app, registry) = fixture(2).await;
            let old = vec![serde_json::json!({"files":[{"path":"old"}]})];
            let new = vec![serde_json::json!({"files":[{"path":"new"}]})];
            let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
            a.ensure_ready(&app, false).await.unwrap();
            if !first {
                feed(&mut a, "x = 1").await;
                a.persist(&app, "old", old.clone()).await.unwrap();
            }
            feed(&mut a, "x = 2").await;
            let ws = PythonWorkspace::for_chat(&app, "a").await.unwrap();
            super::super::workspace::arm_save_fault(ws.dir(), step);
            assert!(a.persist(&app, "new", new.clone()).await.is_err(), "{case}");
            drop(a);
            drop(registry);
            let (mut a, start) = restart_engine(&app, "a").await;
            assert_eq!(start.reset_reason, None, "{case}");
            let x = feed(&mut a, "x if 'x' in locals() else None").await;
            if step == "published" || first && step == "manifest_written" {
                assert!(start.restored, "{case}");
                assert_eq!((x, a.taint()), (serde_json::json!(2), new), "{case}");
            } else if first {
                assert!(!start.restored, "{case}");
                assert_eq!((x, a.taint()), (Value::Null, vec![]), "{case}");
            } else {
                assert!(start.restored, "{case}");
                assert_eq!((x, a.taint()), (serde_json::json!(1), old), "{case}");
            }
        }
    }
}

#[tokio::test]
async fn a_manifest_replace_that_was_not_atomic_is_completed_on_the_next_start() {
    let (_dir, app, registry) = fixture(2).await;
    let new = vec![serde_json::json!({"files":[{"path":"new"}]})];
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 1").await;
    a.persist(&app, "old", vec![]).await.unwrap();
    feed(&mut a, "x = 2").await;
    let ws = PythonWorkspace::for_chat(&app, "a").await.unwrap();
    super::super::workspace::arm_save_fault(ws.dir(), "manifest_written");
    assert!(a.persist(&app, "new", new.clone()).await.is_err());
    tokio::fs::remove_file(ws.dir().join("state.json"))
        .await
        .unwrap();
    drop(a);
    drop(registry);
    let (mut a, start) = restart_engine(&app, "a").await;
    assert_eq!((start.restored, start.reset_reason), (true, None));
    assert_eq!(feed(&mut a, "x").await, 2);
    assert_eq!(a.taint(), new);
    assert!(ws.dir().join("state.json").exists());
}

#[tokio::test]
async fn a_missing_cut_or_tampered_generation_resets_with_its_reason() {
    for damage in ["missing", "cut", "tampered"] {
        let (_dir, app, registry) = fixture(2).await;
        let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
        a.ensure_ready(&app, false).await.unwrap();
        feed(&mut a, "x = 'x' * 1000").await;
        a.persist(&app, "run", vec![]).await.unwrap();
        let path = generation_path(&app, "a").await;
        let mut bytes = tokio::fs::read(&path).await.unwrap();
        match damage {
            "missing" => tokio::fs::remove_file(&path).await.unwrap(),
            "cut" => tokio::fs::write(&path, &bytes[..bytes.len() / 2])
                .await
                .unwrap(),
            _ => {
                let at = bytes.len() / 2;
                bytes[at] ^= 1;
                tokio::fs::write(&path, &bytes).await.unwrap();
            }
        }
        drop(a);
        drop(registry);
        let (mut a, start) = restart_engine(&app, "a").await;
        let reason = start.reset_reason.unwrap_or_default();
        let expected = if damage == "missing" {
            "missing"
        } else {
            "mismatch"
        };
        assert!(reason.contains(expected), "{damage}: {reason}");
        assert!(!start.restored, "{damage}");
        assert_eq!(feed(&mut a, "'x' in locals()").await, false, "{damage}");
    }
}

fn generation_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "monty"))
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn each_save_is_a_new_generation_and_only_the_previous_one_is_kept() {
    let (_dir, app, registry) = fixture(2).await;
    let ws = PythonWorkspace::for_chat(&app, "a").await.unwrap();
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    let mut published = vec![];
    for i in 0..3 {
        feed(&mut a, &format!("x = {i}")).await;
        a.persist(&app, "run", vec![]).await.unwrap();
        published.push(generation_path(&app, "a").await);
    }
    let mut kept = published[1..].to_vec();
    kept.sort();
    assert_eq!(generation_files(ws.dir()), kept);
    assert_ne!(published[0], published[1]);
    assert_ne!(published[1], published[2]);
}

#[tokio::test]
async fn a_session_saved_in_the_old_layout_loads_and_moves_to_generations() {
    let (_dir, app, registry) = fixture(2).await;
    let ws = PythonWorkspace::for_chat(&app, "a").await.unwrap();
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 7").await;
    a.persist(&app, "run", vec![]).await.unwrap();
    let legacy = ws.dir().join("state.monty");
    let generation = generation_path(&app, "a").await;
    if generation != legacy {
        tokio::fs::rename(&generation, &legacy).await.unwrap();
    }
    let mut meta: Value =
        serde_json::from_slice(&tokio::fs::read(ws.dir().join("state.json")).await.unwrap())
            .unwrap();
    meta.as_object_mut().unwrap().remove("generation");
    tokio::fs::write(ws.dir().join("state.json"), meta.to_string())
        .await
        .unwrap();
    drop(a);
    drop(registry);
    let (mut a, start) = restart_engine(&app, "a").await;
    assert_eq!((start.restored, start.reset_reason), (true, None));
    assert_eq!(feed(&mut a, "x").await, 7);
    for i in 0..2 {
        feed(&mut a, &format!("x = {i}")).await;
        a.persist(&app, "run", vec![]).await.unwrap();
        assert_eq!(legacy.exists(), i == 0, "after save {i}");
    }
    assert_ne!(generation_path(&app, "a").await, legacy);
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

async fn fork_fixture() -> (tempfile::TempDir, AppState) {
    let (dir, mut app, _) = fixture(4).await;
    app.python_tool = Arc::new(crate::python_tool::PythonToolRuntime {
        registry: KernelRegistry::new(WorkerBackend::InThread),
        ..Default::default()
    });
    (dir, app)
}
async fn chat_lease(app: &AppState, chat: &str) -> SessionLease {
    app.python_tool
        .registry
        .reserve_turn(chat, "t")
        .acquire()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_fork_says_when_the_source_state_cannot_be_copied() {
    let (_dir, app) = fork_fixture().await;
    let mut a = chat_lease(&app, "a").await;
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 1").await;
    a.persist(&app, "run", vec![]).await.unwrap();
    let path = generation_path(&app, "a").await;
    let mut bytes = tokio::fs::read(&path).await.unwrap();
    let at = bytes.len() / 2;
    bytes[at] ^= 1;
    tokio::fs::write(&path, &bytes).await.unwrap();
    drop(a);
    PythonWorkspace::fork(&app, "a", "b").await.unwrap();
    let mut b = chat_lease(&app, "b").await;
    let start = b.ensure_ready(&app, false).await.unwrap();
    assert_eq!(start.reset_reason, None);
    let note = start.notes.join("\n");
    assert!(
        note.starts_with("variables were not copied from chat a:") && note.contains("mismatch"),
        "{note}"
    );
    assert!(!start.restored);
    assert_eq!(feed(&mut b, "'x' in locals()").await, false);
}

#[tokio::test]
async fn a_fork_leaves_a_destination_that_has_its_own_session_alone() {
    let (_dir, app) = fork_fixture().await;
    for (chat, code) in [("a", "x = 'source'"), ("b", "x = 'destination'")] {
        let mut lease = chat_lease(&app, chat).await;
        lease.ensure_ready(&app, false).await.unwrap();
        feed(&mut lease, code).await;
        lease.persist(&app, "run", vec![]).await.unwrap();
    }
    let before = generation_path(&app, "b").await;
    PythonWorkspace::fork(&app, "a", "b").await.unwrap();
    assert_eq!(generation_path(&app, "b").await, before);
    let mut b = chat_lease(&app, "b").await;
    let start = b.ensure_ready(&app, false).await.unwrap();
    assert_eq!(feed(&mut b, "x").await, "destination");
    assert_eq!(start.reset_reason, None);
    assert_eq!(
        start.notes,
        ["variables were not copied from chat a: this chat already has a Python session"]
    );
}

#[tokio::test]
async fn a_fork_waits_for_a_destination_run_and_copies_the_last_saved_source_state() {
    let (_dir, app) = fork_fixture().await;
    let taint = vec![serde_json::json!({"files":[{"path":"saved"}]})];
    let mut a = chat_lease(&app, "a").await;
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 1").await;
    a.persist(&app, "run", taint.clone()).await.unwrap();
    feed(&mut a, "x = 2").await;
    let b_run = chat_lease(&app, "b").await;
    let fork = tokio::spawn({
        let app = app.clone();
        async move { PythonWorkspace::fork(&app, "a", "b").await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let ws = PythonWorkspace::for_chat(&app, "b").await.unwrap();
    assert!(ws.read_state_meta().await.is_none());
    assert!(!fork.is_finished());
    drop(b_run);
    tokio::time::timeout(Duration::from_secs(10), fork)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(a);
    let mut b = chat_lease(&app, "b").await;
    let start = b.ensure_ready(&app, false).await.unwrap();
    assert_eq!((start.restored, start.reset_reason), (true, None));
    assert_eq!(feed(&mut b, "x").await, 1);
    assert_eq!(b.taint(), taint);
}

#[tokio::test]
async fn an_evicted_session_that_is_saved_is_not_saved_again() {
    let (_dir, app, registry) = fixture(1).await;
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 42").await;
    a.persist(&app, "run", vec![]).await.unwrap();
    drop(a);
    let ws = PythonWorkspace::for_chat(&app, "a").await.unwrap();
    let before = tokio::fs::read(ws.dir().join("state.json")).await.unwrap();
    let mut b = registry.reserve_turn("b", "2").acquire().await.unwrap();
    b.ensure_ready(&app, false).await.unwrap();
    assert_eq!(
        tokio::fs::read(ws.dir().join("state.json")).await.unwrap(),
        before
    );
    assert!(registry.0.slots.lock().unwrap()["a"]
        .session
        .try_lock()
        .unwrap()
        .client
        .is_none());
}

#[tokio::test]
async fn an_eviction_save_that_fails_still_starts_the_new_session() {
    let (_dir, app, registry) = fixture(1).await;
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 1").await;
    a.persist(&app, "run", vec![]).await.unwrap();
    feed(&mut a, "x = 2").await;
    drop(a);
    let ws = PythonWorkspace::for_chat(&app, "a").await.unwrap();
    super::super::workspace::arm_save_fault(ws.dir(), "snapshot_timeout");
    let mut b = registry.reserve_turn("b", "2").acquire().await.unwrap();
    b.ensure_ready(&app, false).await.unwrap();
    drop(b);
    let mut a = registry.reserve_turn("a", "3").acquire().await.unwrap();
    let start = a.ensure_ready(&app, false).await.unwrap();
    assert!(start.restored);
    assert_eq!(start.reset_reason, None);
    assert_eq!(
        start.notes,
        ["the session was closed to free a worker and its last changes were not saved (Python worker gave no answer within 10 s); variables come from the last successful save"]
    );
    assert_eq!(feed(&mut a, "x").await, 1);
}

#[test]
fn a_busy_session_tells_the_model_not_to_poll() {
    let text = PythonBusy {
        job_id: "job-1".into(),
        tool_call_id: "call-1".into(),
    }
    .to_string();
    assert_eq!(
        text,
        "Python job job-1 (tool call call-1) is still running; its result arrives in this chat by itself and starts your next turn, so do not poll for it. Use tool_job only to cancel it"
    );
    assert!(!text.contains("wait for"));
}

use super::super::workspace::{arm_pause, arm_save_fault, ForkOutcome};

async fn saved(app: &AppState, registry: &KernelRegistry, code: &str) -> (SessionLease, PathBuf) {
    let mut lease = registry.reserve_turn("a", "1").acquire().await.unwrap();
    lease.ensure_ready(app, false).await.unwrap();
    feed(&mut lease, code).await;
    lease.persist(app, "run", vec![]).await.unwrap();
    let dir = PythonWorkspace::for_chat(app, "a")
        .await
        .unwrap()
        .dir()
        .to_path_buf();
    (lease, dir)
}
fn pending_manifests(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("state.json.")
        })
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn a_start_whose_ready_wait_fails_retires_the_new_worker() {
    let (_dir, app, registry) = fixture(2).await;
    let (mut a, dir) = saved(&app, &registry, "x = 42").await;
    a.discard_worker_for_rollback("test").await;
    arm_save_fault(&dir, "ready_timeout");
    assert!(a.ensure_ready(&app, false).await.is_err());
    assert!(!a.has_worker());
    assert!(!registry.session_status("a").live);
    let start = a.ensure_ready(&app, false).await.unwrap();
    assert!(start.restored);
    assert_eq!(feed(&mut a, "x").await, 42);
}

#[tokio::test]
async fn a_start_that_cannot_clear_an_invalid_state_still_reports_its_reset() {
    let (_dir, app, registry) = fixture(2).await;
    let (mut a, dir) = saved(&app, &registry, "x = 42").await;
    tokio::fs::write(generation_path(&app, "a").await, b"bad")
        .await
        .unwrap();
    a.discard_worker_for_rollback("test").await;
    arm_save_fault(&dir, "clear_state");
    assert!(a.ensure_ready(&app, false).await.is_err());
    assert!(!a.has_worker());
    PythonWorkspace::for_chat(&app, "a")
        .await
        .unwrap()
        .clear_state()
        .await
        .unwrap();
    let start = a.ensure_ready(&app, false).await.unwrap();
    let reason = start.reset_reason.unwrap_or_default();
    assert!(reason.contains("mismatch"), "{reason}");
}

#[tokio::test]
async fn an_unreadable_manifest_keeps_the_state_and_starts_empty_for_one_run() {
    let (_dir, app, registry) = fixture(2).await;
    let taint = vec![serde_json::json!({"files":[{"path":"saved"}]})];
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 42").await;
    a.persist(&app, "run", taint.clone()).await.unwrap();
    a.discard_worker_for_rollback("test").await;
    let dir = PythonWorkspace::for_chat(&app, "a")
        .await
        .unwrap()
        .dir()
        .to_path_buf();
    let manifest = tokio::fs::read(dir.join("state.json")).await.unwrap();
    arm_save_fault(&dir, "manifest_read");
    let start = a.ensure_ready(&app, false).await.unwrap();
    assert_eq!((start.restored, start.reset_reason), (false, None));
    assert_eq!(
        start.notes,
        ["saved state could not be read (its metadata is unreadable: injected read failure); it is kept for the next run"]
    );
    assert_eq!(a.taint(), Vec::<Value>::new());
    assert_eq!(feed(&mut a, "'x' in locals()").await, false);
    feed(&mut a, "y = 1").await;
    assert!(!a.persist(&app, "empty", vec![]).await.unwrap().persisted);
    assert!(!a.has_worker());
    assert_eq!(
        tokio::fs::read(dir.join("state.json")).await.unwrap(),
        manifest
    );
    let start = a.ensure_ready(&app, false).await.unwrap();
    assert!(start.restored && start.notes.is_empty());
    assert_eq!(feed(&mut a, "x").await, 42);
    assert_eq!(a.taint(), taint);
}

#[tokio::test]
async fn a_manifest_replaced_before_it_is_cleared_is_read_again() {
    let (_dir, app, registry) = fixture(2).await;
    let (a, dir) = saved(&app, &registry, "x = 42").await;
    drop(a);
    drop(registry);
    let manifest = dir.join("state.json");
    let valid = tokio::fs::read(&manifest).await.unwrap();
    let mut meta: Value = serde_json::from_slice(&valid).unwrap();
    meta["hmac"] = serde_json::json!("0".repeat(64));
    tokio::fs::write(&manifest, meta.to_string()).await.unwrap();
    let pause = arm_pause(&dir, "before_clear");
    let start = tokio::spawn({
        let app = app.clone();
        async move { restart_engine(&app, "a").await }
    });
    pause.reached().await;
    tokio::fs::write(&manifest, &valid).await.unwrap();
    pause.release();
    let (mut a, start) = start.await.unwrap();
    assert_eq!((start.restored, start.reset_reason), (true, None));
    assert_eq!(feed(&mut a, "x").await, 42);
}

#[tokio::test]
async fn a_generation_that_vanishes_while_it_loads_is_not_a_reset() {
    let (_dir, app, registry) = fixture(2).await;
    let (a, dir) = saved(&app, &registry, "x = 42").await;
    drop(a);
    drop(registry);
    let generation = generation_path(&app, "a").await;
    let pause = arm_pause(&dir, "restore_pinned");
    let start = tokio::spawn({
        let app = app.clone();
        async move { restart_engine(&app, "a").await }
    });
    pause.reached().await;
    tokio::fs::remove_file(&generation).await.unwrap();
    pause.release();
    let (_a, start) = start.await.unwrap();
    assert_eq!((start.restored, start.reset_reason), (false, None));
    assert_eq!(
        start.notes,
        ["saved state could not be read (its snapshot vanished while it was read); it is kept for the next run"]
    );
    assert!(dir.join("state.json").exists());
}

#[tokio::test]
async fn a_valid_pending_manifest_stands_in_for_a_current_one_whose_snapshot_is_damaged() {
    let (_dir, app, registry) = fixture(2).await;
    let (mut a, dir) = saved(&app, &registry, "x = 1").await;
    let first = generation_path(&app, "a").await;
    feed(&mut a, "x = 2").await;
    arm_save_fault(&dir, "manifest_written");
    assert!(a.persist(&app, "new", vec![]).await.is_err());
    drop(a);
    drop(registry);
    let mut bytes = tokio::fs::read(&first).await.unwrap();
    let at = bytes.len() / 2;
    bytes[at] ^= 1;
    tokio::fs::write(&first, &bytes).await.unwrap();
    let (mut a, start) = restart_engine(&app, "a").await;
    assert_eq!((start.restored, start.reset_reason), (true, None));
    assert_eq!(feed(&mut a, "x").await, 2);
}

#[tokio::test]
async fn an_adopted_pending_manifest_survives_a_failed_install_and_a_torn_next_save() {
    let (_dir, app, registry) = fixture(2).await;
    let (mut a, dir) = saved(&app, &registry, "x = 1").await;
    feed(&mut a, "x = 2").await;
    arm_save_fault(&dir, "manifest_written");
    assert!(a.persist(&app, "new", vec![]).await.is_err());
    drop(a);
    drop(registry);
    tokio::fs::remove_file(dir.join("state.json"))
        .await
        .unwrap();
    arm_save_fault(&dir, "install");
    let (mut a, start) = restart_engine(&app, "a").await;
    assert_eq!((start.restored, start.reset_reason), (true, None));
    assert_eq!(feed(&mut a, "x").await, 2);
    assert!(!dir.join("state.json").exists());
    feed(&mut a, "x = 3").await;
    arm_save_fault(&dir, "manifest_write_cut");
    assert!(a.persist(&app, "torn", vec![]).await.is_err());
    drop(a);
    let (mut a, start) = restart_engine(&app, "a").await;
    assert_eq!((start.restored, start.reset_reason), (true, None));
    assert_eq!(feed(&mut a, "x").await, 2);
}

#[tokio::test]
async fn a_torn_pending_manifest_is_ignored_and_the_current_state_wins() {
    let (_dir, app, registry) = fixture(2).await;
    let (mut a, dir) = saved(&app, &registry, "x = 1").await;
    let first = generation_path(&app, "a").await;
    feed(&mut a, "x = 2").await;
    arm_save_fault(&dir, "manifest_write_cut");
    assert!(a.persist(&app, "torn", vec![]).await.is_err());
    assert_eq!(pending_manifests(&dir).len(), 1);
    assert_eq!(generation_files(&dir), vec![first.clone()]);
    drop(a);
    drop(registry);
    let (mut a, start) = restart_engine(&app, "a").await;
    assert_eq!((start.restored, start.reset_reason), (true, None));
    assert_eq!(feed(&mut a, "x").await, 1);
    assert_eq!(generation_files(&dir), vec![first]);
}

#[tokio::test]
async fn a_failed_rename_leaves_the_old_state_and_nothing_of_the_attempt() {
    let (_dir, app, registry) = fixture(2).await;
    let (mut a, dir) = saved(&app, &registry, "x = 1").await;
    let first = generation_path(&app, "a").await;
    feed(&mut a, "x = 2").await;
    arm_save_fault(&dir, "install");
    assert!(a.persist(&app, "new", vec![]).await.is_err());
    assert_eq!(pending_manifests(&dir), Vec::<PathBuf>::new());
    assert_eq!(generation_files(&dir), vec![first]);
    drop(a);
    drop(registry);
    let (mut a, start) = restart_engine(&app, "a").await;
    assert_eq!((start.restored, start.reset_reason), (true, None));
    assert_eq!(feed(&mut a, "x").await, 1);
}

#[tokio::test]
async fn a_partly_written_generation_is_never_published() {
    let (_dir, app, registry) = fixture(2).await;
    let (mut a, dir) = saved(&app, &registry, "x = 1").await;
    let first = generation_path(&app, "a").await;
    feed(&mut a, "x = 'y' * 1000").await;
    arm_save_fault(&dir, "generation_cut");
    let error = a.persist(&app, "cut", vec![]).await.unwrap_err();
    assert!(error.contains("does not match"), "{error}");
    assert_eq!(generation_files(&dir), vec![first]);
    drop(a);
    drop(registry);
    let (mut a, start) = restart_engine(&app, "a").await;
    assert_eq!((start.restored, start.reset_reason), (true, None));
    assert_eq!(feed(&mut a, "x").await, 1);
}

#[tokio::test]
async fn a_failed_save_removes_its_generation_and_unknown_ones_stay_until_they_are_old() {
    let (_dir, app, registry) = fixture(2).await;
    let (mut a, dir) = saved(&app, &registry, "x = 1").await;
    let first = generation_path(&app, "a").await;
    feed(&mut a, "x = 2").await;
    arm_save_fault(&dir, "snapshot_done");
    assert!(a.persist(&app, "failed", vec![]).await.is_err());
    assert_eq!(generation_files(&dir), vec![first]);
    a.ensure_ready(&app, false).await.unwrap();
    let young = dir.join(format!("state-{}.monty", "a".repeat(32)));
    let old = dir.join(format!("state-{}.monty", "b".repeat(32)));
    for path in [&young, &old] {
        std::fs::write(path, b"another engine's save").unwrap();
    }
    filetime::set_file_mtime(&old, filetime::FileTime::from_unix_time(1, 0)).unwrap();
    feed(&mut a, "x = 3").await;
    assert!(a.persist(&app, "run", vec![]).await.unwrap().persisted);
    assert!(young.exists());
    assert!(!old.exists());
}

#[tokio::test]
async fn a_late_save_reply_never_answers_the_next_command_of_the_same_registry() {
    let (_dir, app, registry) = fixture(2).await;
    let (mut a, dir) = saved(&app, &registry, "x = 1").await;
    feed(&mut a, "x = 2").await;
    arm_save_fault(&dir, "snapshot_timeout");
    assert!(a.persist(&app, "late", vec![]).await.is_err());
    let start = a.ensure_ready(&app, false).await.unwrap();
    assert!(start.restored);
    assert_eq!(feed(&mut a, "x").await, 1);
}

#[tokio::test]
async fn an_oversized_state_is_refused_and_the_saved_one_stays() {
    let (dir, app, registry) = fixture(1).await;
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        "python_max_live_sessions: 1\npython_max_state_bytes: 1048576\n",
    )
    .unwrap();
    let (mut a, ws) = saved(&app, &registry, "x = 1").await;
    let manifest = tokio::fs::read(ws.join("state.json")).await.unwrap();
    let first = generation_path(&app, "a").await;
    feed(&mut a, "big = 'b' * 2000000").await;
    let outcome = a.persist(&app, "big", vec![]).await.unwrap();
    assert!(!outcome.persisted);
    assert_eq!(
        outcome.not_saved.as_deref(),
        Some("not saved (state is 2.0 MB, over the limit 1.0 MB); variables stay in memory until this session is restarted or evicted; set large variables to None and the next run saves again")
    );
    assert_eq!(
        tokio::fs::read(ws.join("state.json")).await.unwrap(),
        manifest
    );
    assert_eq!(generation_files(&ws), vec![first]);
    assert!(a.has_worker());
    assert_eq!(feed(&mut a, "len(big)").await, 2000000);
    drop(a);
    let mut b = registry.reserve_turn("b", "2").acquire().await.unwrap();
    b.ensure_ready(&app, false).await.unwrap();
    drop(b);
    let mut a = registry.reserve_turn("a", "3").acquire().await.unwrap();
    let start = a.ensure_ready(&app, false).await.unwrap();
    assert_eq!(
        (start.restored, start.reset_reason.as_deref()),
        (true, None)
    );
    assert_eq!(
        start.notes,
        ["the session was closed to free a worker and its last changes were not saved (state is 2.0 MB, over the limit 1.0 MB); variables come from the last successful save"]
    );
    assert_eq!(feed(&mut a, "x").await, 1);
}

#[tokio::test]
async fn an_eviction_without_a_saved_state_says_the_session_starts_empty() {
    let (_dir, app, registry) = fixture(1).await;
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 1").await;
    drop(a);
    let dir = PythonWorkspace::for_chat(&app, "a")
        .await
        .unwrap()
        .dir()
        .to_path_buf();
    arm_save_fault(&dir, "snapshot_timeout");
    let mut b = registry.reserve_turn("b", "2").acquire().await.unwrap();
    b.ensure_ready(&app, false).await.unwrap();
    drop(b);
    let mut a = registry.reserve_turn("a", "3").acquire().await.unwrap();
    let start = a.ensure_ready(&app, false).await.unwrap();
    assert_eq!((start.restored, start.reset_reason), (false, None));
    assert_eq!(
        start.notes,
        ["the session was closed to free a worker and its last changes were not saved (Python worker gave no answer within 120 s); the session starts empty"]
    );
}

#[tokio::test]
async fn a_fork_never_writes_over_a_destination_manifest_even_one_that_does_not_parse() {
    let (_dir, app) = fork_fixture().await;
    for (chat, code) in [("a", "x = 'source'"), ("b", "x = 'destination'")] {
        let mut lease = chat_lease(&app, chat).await;
        lease.ensure_ready(&app, false).await.unwrap();
        feed(&mut lease, code).await;
        lease.persist(&app, "run", vec![]).await.unwrap();
        lease.discard_worker_for_rollback("test").await;
    }
    let manifest = PythonWorkspace::for_chat(&app, "b")
        .await
        .unwrap()
        .dir()
        .join("state.json");
    let refused = ForkOutcome {
        variables_not_copied: Some("this chat already has saved Python state".into()),
        ..Default::default()
    };
    let before = tokio::fs::read(&manifest).await.unwrap();
    assert_eq!(
        PythonWorkspace::fork(&app, "a", "b").await.unwrap(),
        refused
    );
    assert_eq!(tokio::fs::read(&manifest).await.unwrap(), before);
    let mut b = chat_lease(&app, "b").await;
    let start = b.ensure_ready(&app, false).await.unwrap();
    assert_eq!(feed(&mut b, "x").await, "destination");
    assert_eq!(
        start.notes,
        ["variables were not copied from chat a: this chat already has saved Python state"]
    );
    b.discard_worker_for_rollback("test").await;
    drop(b);
    tokio::fs::write(&manifest, b"{not json").await.unwrap();
    assert_eq!(
        PythonWorkspace::fork(&app, "a", "b").await.unwrap(),
        refused
    );
    assert_eq!(tokio::fs::read(&manifest).await.unwrap(), b"{not json");
}

#[tokio::test]
async fn a_fork_that_races_two_publishes_copies_one_whole_generation() {
    let (_dir, app) = fork_fixture().await;
    let taint = |n: u32| vec![serde_json::json!({"files":[{"path":format!("v{n}")}]})];
    let mut a = chat_lease(&app, "a").await;
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 1").await;
    a.persist(&app, "run", taint(1)).await.unwrap();
    let dir = PythonWorkspace::for_chat(&app, "a")
        .await
        .unwrap()
        .dir()
        .to_path_buf();
    let pause = arm_pause(&dir, "fork_pinned");
    let fork = tokio::spawn({
        let app = app.clone();
        async move { PythonWorkspace::fork(&app, "a", "b").await }
    });
    pause.reached().await;
    for n in [2, 3] {
        feed(&mut a, &format!("x = {n}")).await;
        a.persist(&app, "run", taint(n)).await.unwrap();
    }
    pause.release();
    assert_eq!(fork.await.unwrap().unwrap(), ForkOutcome::default());
    drop(a);
    let mut b = chat_lease(&app, "b").await;
    let start = b.ensure_ready(&app, false).await.unwrap();
    assert!(start.restored);
    assert_eq!(
        (feed(&mut b, "x").await, b.taint()),
        (serde_json::json!(3), taint(3))
    );
}

#[tokio::test]
async fn a_start_that_restores_nothing_forgets_the_taint_of_a_save_that_failed() {
    let (_dir, app, registry) = fixture(2).await;
    let mut a = registry.reserve_turn("a", "1").acquire().await.unwrap();
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 1").await;
    let dir = PythonWorkspace::for_chat(&app, "a")
        .await
        .unwrap()
        .dir()
        .to_path_buf();
    arm_save_fault(&dir, "snapshot_timeout");
    let taint = vec![serde_json::json!({"files":[{"path":"secret"}]})];
    assert!(a.persist(&app, "run", taint).await.is_err());
    let start = a.ensure_ready(&app, false).await.unwrap();
    assert!(!start.restored);
    assert_eq!(a.taint(), Vec::<Value>::new());
}

#[tokio::test]
async fn a_slow_restore_does_not_hold_up_the_start_of_another_chat() {
    let (_dir, app) = fork_fixture().await;
    let mut a = chat_lease(&app, "a").await;
    a.ensure_ready(&app, false).await.unwrap();
    feed(&mut a, "x = 42").await;
    a.persist(&app, "run", vec![]).await.unwrap();
    a.discard_worker_for_rollback("test").await;
    let dir = PythonWorkspace::for_chat(&app, "a")
        .await
        .unwrap()
        .dir()
        .to_path_buf();
    let pause = arm_pause(&dir, "restore_pinned");
    let slow = tokio::spawn({
        let app = app.clone();
        async move {
            let start = a
                .ensure_ready(&app, false)
                .await
                .map(|start| start.restored);
            (a, start)
        }
    });
    pause.reached().await;
    let mut b = chat_lease(&app, "b").await;
    tokio::time::timeout(Duration::from_secs(10), b.ensure_ready(&app, false))
        .await
        .expect("another chat's start waited for a restore")
        .unwrap();
    pause.release();
    let (mut a, restored) = slow.await.unwrap();
    assert_eq!(restored, Ok(true));
    assert_eq!(feed(&mut a, "x").await, 42);
}

#[cfg(unix)]
#[tokio::test]
async fn a_folder_sync_the_filesystem_cannot_do_is_only_a_warning() {
    let (_dir, app, registry) = fixture(2).await;
    let (mut a, dir) = saved(&app, &registry, "x = 1").await;
    feed(&mut a, "x = 2").await;
    arm_save_fault(&dir, "dir_sync_einval");
    assert!(a.persist(&app, "run", vec![]).await.unwrap().persisted);
    feed(&mut a, "x = 3").await;
    arm_save_fault(&dir, "dir_sync_eio");
    assert!(a.persist(&app, "run", vec![]).await.is_err());
    let start = a.ensure_ready(&app, false).await.unwrap();
    assert!(start.restored);
    assert_eq!(feed(&mut a, "x").await, 2);
}

#[tokio::test]
async fn the_large_state_note_is_due_again_after_a_restart_or_a_small_save() {
    let (_dir, app, registry) = fixture(2).await;
    let (mut a, _dir) = saved(&app, &registry, "x = 1").await;
    a.session.as_mut().unwrap().noted_bytes = Some(32 << 20);
    feed(&mut a, "x = 2").await;
    a.persist(&app, "run", vec![]).await.unwrap();
    assert_eq!(a.session.as_ref().unwrap().noted_bytes, None);
    a.session.as_mut().unwrap().noted_bytes = Some(32 << 20);
    a.ensure_ready(&app, true).await.unwrap();
    assert_eq!(a.session.as_ref().unwrap().noted_bytes, None);
}

#[test]
fn the_large_state_note_counts_the_unmeasured_variables_it_does_not_name() {
    let size = |name: &str, approx_bytes: Option<u64>| VariableSize {
        name: name.into(),
        type_name: "Index".into(),
        len: None,
        approx_bytes,
    };
    let largest = [
        size("big", Some(20_000_000)),
        size("a", None),
        size("b", None),
    ];
    assert_eq!(
        large_state_note(30_000_000, false, &largest, 5),
        "the session holds 30.0 MB of variables. Largest: big Index ~20.0 MB; size unknown: a (Index), b (Index) and 3 more of unknown size. Set variables you no longer need to None (del is not supported), or start over with restart=true."
    );
    assert!(!large_state_note(30_000_000, false, &largest, 2).contains("more of unknown size"));
    assert!(large_state_note(30_000_000, false, &largest[..1], 4)
        .contains("; 4 variables of unknown size."));
}
