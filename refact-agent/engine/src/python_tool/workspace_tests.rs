use super::workspace::*;
use sha2::{Digest, Sha256};
use tokio::fs;

fn age_run_dir(path: &std::path::Path) {
    filetime::set_file_mtime(path, filetime::FileTime::from_unix_time(1, 0)).unwrap();
}

#[tokio::test]
async fn quota_prunes_orphans_but_not_active_or_recent_directories() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    ws.ensure_dirs().await.unwrap();
    for id in ["missing", "invalid", "active", "recent"] {
        fs::create_dir_all(ws.run_dir(id)).await.unwrap();
        fs::write(ws.run_dir(id).join("capture"), "data")
            .await
            .unwrap();
        if id == "invalid" {
            fs::write(ws.run_dir(id).join("run.json"), "invalid")
                .await
                .unwrap();
        }
        if id != "recent" {
            age_run_dir(&ws.run_dir(id));
        }
    }
    let _active = ActiveRunPath::new(ws.run_dir("active"));
    enforce_project_quota(&temp.path().join("trajectories"), 0)
        .await
        .unwrap();
    assert!(!ws.run_dir("missing").exists());
    assert!(!ws.run_dir("invalid").exists());
    assert!(ws.run_dir("active").exists());
    assert!(ws.run_dir("recent").exists());
}

#[tokio::test]
async fn failed_run_guard_records_abandoned_run_and_preserves_terminal_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    ws.ensure_dirs().await.unwrap();
    fs::create_dir_all(ws.run_dir("abandoned")).await.unwrap();
    // Error returns and future cancellation both drop this guard.
    drop(FailedRunGuard::new(
        ws.run_dir("abandoned"),
        "abandoned".into(),
        42,
        false,
    ));
    let data: serde_json::Value = serde_json::from_slice(
        &fs::read(ws.run_dir("abandoned").join("run.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(data["status"], "failed");
    assert_eq!(data["started_at_ms"], 42);
    assert!(data["finished_at_ms"].as_u64().unwrap() >= 42);
    let mut guard = FailedRunGuard::new(ws.run_dir("abandoned"), "abandoned".into(), 42, false);
    fs::write(
        ws.run_dir("abandoned").join("run.json"),
        br#"{"status":"completed"}"#,
    )
    .await
    .unwrap();
    guard.disarm();
    drop(guard);
    let data: serde_json::Value = serde_json::from_slice(
        &fs::read(ws.run_dir("abandoned").join("run.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(data["status"], "completed");
}

fn workspace(root: &std::path::Path, id: &str) -> PythonWorkspace {
    PythonWorkspace::from_trajectory(
        &root.join("trajectories/root").join(format!("{id}.json")),
        id,
        root.join("config/python_state.key"),
    )
}
async fn snapshot(ws: &PythonWorkspace) -> std::path::PathBuf {
    record(ws, vec![], false).await
}
async fn record(
    ws: &PythonWorkspace,
    taint: Vec<serde_json::Value>,
    variables_truncated: bool,
) -> std::path::PathBuf {
    ws.ensure_dirs().await.unwrap();
    let generation = ws.new_generation_path();
    fs::write(&generation, b"trusted dump").await.unwrap();
    ws.record_snapshot(
        NewState {
            generation: generation.clone(),
            sha256: format!("{:x}", Sha256::digest(b"trusted dump")),
            bytes: 12,
            last_run_id: "run".into(),
            taint,
            variables: vec![],
            variables_truncated,
            variables_total: None,
            variables_unknown: false,
        },
        1024,
    )
    .await
    .unwrap();
    generation
}
#[test]
fn workspace_dir_is_inside_chat_trajectory_folder() {
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        workspace(temp.path(), "child").dir(),
        temp.path().join("trajectories/root/python/child")
    );
}
#[tokio::test]
async fn state_meta_round_trip_with_valid_hmac() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    snapshot(&ws).await;
    assert_eq!(ws.read_state_meta().await.unwrap().schema, 1);
    assert!(ws.verify_state().await.is_ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(temp.path().join("config/python_state.key"))
                .await
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[tokio::test]
async fn tampered_state_is_rejected_with_reset_reason() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    let generation = snapshot(&ws).await;
    fs::write(generation, b"malicious bytes").await.unwrap();
    assert!(ws
        .verify_state()
        .await
        .unwrap_err()
        .to_string()
        .contains("digest"));
}
#[tokio::test]
async fn state_for_other_chat_id_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    let generation = snapshot(&ws).await;
    let other = workspace(temp.path(), "other");
    other.ensure_dirs().await.unwrap();
    fs::copy(
        &generation,
        other.dir().join(generation.file_name().unwrap()),
    )
    .await
    .unwrap();
    fs::copy(ws.dir().join("state.json"), other.dir().join("state.json"))
        .await
        .unwrap();
    assert!(other
        .verify_state()
        .await
        .unwrap_err()
        .to_string()
        .contains("authentication"));
}
#[tokio::test]
async fn oversized_state_is_not_persisted() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    let saved = snapshot(&ws).await;
    let generation = ws.new_generation_path();
    fs::write(&generation, b"trusted dump").await.unwrap();
    let out = ws
        .record_snapshot(
            NewState {
                generation: generation.clone(),
                sha256: "unused".into(),
                bytes: 12,
                last_run_id: "run".into(),
                taint: vec![],
                variables: vec![],
                variables_truncated: false,
                variables_total: None,
                variables_unknown: false,
            },
            1,
        )
        .await
        .unwrap();
    assert!(!out.persisted);
    assert!(!generation.exists());
    assert!(saved.exists());
    assert_eq!(
        ws.read_state_meta().await.unwrap().generation,
        saved.file_name().unwrap().to_string_lossy()
    );
    assert!(ws.verify_state().await.is_ok());
}
#[tokio::test]
async fn clear_state_keeps_artifacts_and_runs() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    snapshot(&ws).await;
    fs::write(ws.artifacts_dir().join("output"), b"data")
        .await
        .unwrap();
    fs::create_dir_all(ws.run_dir("run")).await.unwrap();
    ws.clear_state().await.unwrap();
    assert!(ws.artifacts_dir().join("output").exists());
    assert!(ws.run_dir("run").exists());
}
#[tokio::test]
async fn fork_copies_state_and_artifacts_and_resigns_hmac() {
    let temp = tempfile::tempdir().unwrap();
    let src = workspace(temp.path(), "src");
    snapshot(&src).await;
    fs::write(src.artifacts_dir().join("a"), b"artifact")
        .await
        .unwrap();
    fs::write(src.dir().join("privacy_ledger.json"), b"[]")
        .await
        .unwrap();
    fs::create_dir_all(src.run_dir("run")).await.unwrap();
    let dst = workspace(temp.path(), "dst");
    src.fork_to(&dst).await.unwrap();
    assert!(dst.verify_state().await.is_ok());
    assert_ne!(
        src.read_state_meta().await.unwrap().hmac,
        dst.read_state_meta().await.unwrap().hmac
    );
    assert!(dst.artifacts_dir().join("a").exists());
    assert!(dst.dir().join("privacy_ledger.json").exists());
    assert!(!dst.run_dir("run").exists());
}
#[tokio::test]
async fn delete_for_chat_removes_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    snapshot(&ws).await;
    delete_for_trajectory(&temp.path().join("trajectories/root/chat.json"))
        .await
        .unwrap();
    assert!(!ws.dir().exists());
}
#[tokio::test]
async fn quota_prunes_oldest_runs_but_keeps_state_and_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    let generation = snapshot(&ws).await;
    fs::write(ws.artifacts_dir().join("a"), b"artifact")
        .await
        .unwrap();
    fs::create_dir_all(ws.run_dir("old")).await.unwrap();
    fs::write(ws.run_dir("old").join("stdout"), vec![0; 1024])
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    fs::create_dir_all(ws.run_dir("new")).await.unwrap();
    fs::write(ws.run_dir("new").join("stdout"), vec![0; 1024])
        .await
        .unwrap();
    fs::write(
        ws.run_dir("old").join("run.json"),
        br#"{"status":"completed","finished_at_ms":1}"#,
    )
    .await
    .unwrap();
    fs::write(
        ws.run_dir("new").join("run.json"),
        br#"{"status":"completed","finished_at_ms":2}"#,
    )
    .await
    .unwrap();
    let size = ws.disk_usage().await.unwrap();
    enforce_project_quota(&temp.path().join("trajectories"), size - 1024)
        .await
        .unwrap();
    assert!(!ws.run_dir("old").exists());
    assert!(ws.run_dir("new").exists());
    assert!(generation.exists());
    assert!(ws.artifacts_dir().join("a").exists());
}
#[test]
fn python_workspace_paths_are_excluded_from_indexing() {
    let path =
        std::path::Path::new("/project/.refact/trajectories/root/python/chat/runs/run/code.py");
    assert!(is_python_workspace_path(path));
    let roots =
        refact_core::memory_plane::MemoryPlaneRoots::new(vec!["/project".into()], None, None);
    let (memory, code) =
        crate::indexing_routing::partition_paths(&[path.to_string_lossy().into_owned()], &roots);
    assert!(memory.is_empty() && code.is_empty());
    assert!(!is_python_workspace_path(std::path::Path::new(
        "/project/python/code.py"
    )));
}

#[tokio::test]
async fn internal_trace_prune_removes_python_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "trace");
    snapshot(&ws).await;
    let folder = temp.path().join("trajectories/root");
    fs::write(
        folder.join("trace.json"),
        br#"{"id":"trace","link_type":"internal:test"}"#,
    )
    .await
    .unwrap();
    assert_eq!(
        crate::chat::trajectories::prune_internal_traces_in_folder(&folder, 0)
            .await
            .unwrap(),
        1
    );
    assert!(!ws.dir().exists());
}
#[tokio::test]
async fn trajectory_index_rebuild_ignores_python_workspace_files() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    ws.ensure_dirs().await.unwrap();
    fs::write(ws.dir().join("fake.json"), br#"{"id":"fake","title":"not a trajectory","messages":[],"created_at":"2025-01-01T00:00:00Z","updated_at":"2025-01-01T00:00:00Z","model":"","mode":"AGENT"}"#).await.unwrap();
    let index = crate::chat::trajectory_index::rebuild_trajectory_index_from_disk(
        &temp.path().join("trajectories/root/python"),
        None,
    )
    .await
    .unwrap();
    assert!(index.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn fork_refuses_symlinked_artifacts_root() {
    let temp = tempfile::tempdir().unwrap();
    let src = workspace(temp.path(), "src");
    src.ensure_dirs().await.unwrap();
    fs::remove_dir(src.artifacts_dir()).await.unwrap();
    std::os::unix::fs::symlink(temp.path(), src.artifacts_dir()).unwrap();
    assert!(src.fork_to(&workspace(temp.path(), "dst")).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn loading_key_tightens_loose_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    snapshot(&ws).await;
    let key = temp.path().join("config/python_state.key");
    fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644))
        .await
        .unwrap();
    ws.verify_state().await.unwrap();
    assert_eq!(
        fs::metadata(key).await.unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[tokio::test]
async fn quota_never_deletes_active_or_recent_runs() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    snapshot(&ws).await;
    for run in ["active", "recent"] {
        fs::create_dir_all(ws.run_dir(run)).await.unwrap();
        fs::write(ws.run_dir(run).join("capture"), "data")
            .await
            .unwrap();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    fs::write(
        ws.run_dir("recent").join("run.json"),
        serde_json::json!({"status":"completed","finished_at_ms":now}).to_string(),
    )
    .await
    .unwrap();
    fs::write(
        ws.run_dir("active").join("run.json"),
        br#"{"status":"completed","finished_at_ms":1}"#,
    )
    .await
    .unwrap();
    let _active = ActiveRunPath::new(ws.run_dir("active"));
    enforce_project_quota(&temp.path().join("trajectories"), 0)
        .await
        .unwrap();
    assert!(ws.run_dir("active").exists());
    assert!(ws.run_dir("recent").exists());
}

#[tokio::test]
async fn tampered_taint_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    snapshot(&ws).await;
    let mut meta = ws.read_state_meta().await.unwrap();
    meta.taint = vec![serde_json::json!({"files":[]})];
    fs::write(
        ws.dir().join("state.json"),
        serde_json::to_vec(&meta).unwrap(),
    )
    .await
    .unwrap();
    assert!(ws.verify_state().await.is_err());
}

#[tokio::test]
async fn quota_discovers_only_owned_storage_roots() {
    let temp = tempfile::tempdir().unwrap();
    let owned = [
        ".refact/trajectories",
        ".refact/trajectories/root",
        ".refact/tasks/task/trajectories/agents/agent",
        ".refact/buddy/chats/conversations/root",
        "trajectories/root",
        "tasks/task/trajectories/planner",
    ];
    let unrelated = [
        "unrelated",
        ".refact/plugins/example",
        "source/trajectories",
        ".refact/buddy/chats",
        "conversations/root",
    ];
    for parent in owned.iter().chain(unrelated.iter()) {
        let ws = PythonWorkspace::from_trajectory(
            &temp.path().join(parent).join("chat.json"),
            "chat",
            temp.path().join("key"),
        );
        ws.ensure_dirs().await.unwrap();
        fs::create_dir_all(ws.run_dir("old")).await.unwrap();
        fs::write(
            ws.run_dir("old").join("run.json"),
            br#"{"status":"completed","finished_at_ms":1}"#,
        )
        .await
        .unwrap();
    }
    enforce_project_quota(temp.path(), 0).await.unwrap();
    for parent in owned {
        assert!(
            !temp
                .path()
                .join(parent)
                .join("python/chat/runs/old")
                .exists(),
            "{parent}"
        );
    }
    for parent in unrelated {
        assert!(
            temp.path()
                .join(parent)
                .join("python/chat/runs/old")
                .exists(),
            "{parent}"
        );
    }
}

#[tokio::test]
async fn fork_carries_signed_taint_and_remaps_artifact_ledger() {
    let temp = tempfile::tempdir().unwrap();
    let src = workspace(temp.path(), "src");
    let taint = vec![
        serde_json::json!({"files":[{"path":"secret","zone":"blocked","attribution":"observed"}]}),
    ];
    record(&src, taint.clone(), false).await;
    let artifact = src.artifacts_dir().join("a");
    fs::write(&artifact, "derived").await.unwrap();
    // DerivedZone's wire representation is the persisted ledger contract.
    let ledger = serde_json::json!([[artifact, {"zone":"blocked", "origin":"secret"}]]);
    fs::write(
        src.dir().join("privacy_ledger.json"),
        serde_json::to_vec(&ledger).unwrap(),
    )
    .await
    .unwrap();
    let dst = workspace(temp.path(), "dst");
    src.fork_to(&dst).await.unwrap();
    assert!(dst.verify_state().await.is_ok());
    assert_eq!(dst.read_state_meta().await.unwrap().taint, taint);
    let copied: serde_json::Value = serde_json::from_slice(
        &fs::read(dst.dir().join("privacy_ledger.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        copied[0][0],
        dst.artifacts_dir().join("a").to_string_lossy().as_ref()
    );
}

#[tokio::test]
async fn variable_truncation_metadata_survives_persistence() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    record(&ws, vec![], true).await;
    assert!(ws.read_state_meta().await.unwrap().variables_truncated);
    assert!(ws.verify_state().await.is_ok());
}

#[tokio::test]
async fn workspace_path_matches_later_saved_trajectory_folder() {
    let (_dir, app, _) = crate::tools::tool_python::tests::fixture().await;
    let before = PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    let session = app.chat.sessions.read().await["python-test"].clone();
    crate::chat::trajectories::maybe_save_trajectory_with_intent(
        app.clone(),
        session,
        crate::chat::types::TrajectoryCommitIntent::Required,
    )
    .await;
    let after = PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    assert_eq!(before.dir(), after.dir());
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[tokio::test]
async fn quota_keeps_running_runs_of_other_engines_until_they_are_stale() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    ws.ensure_dirs().await.unwrap();
    let max_run_ms = refact_work_core::coordination_settings().python_max_run_ms;
    let now = now_ms();
    for (id, started) in [
        ("live", now - 20 * 60_000),
        ("crashed", now - max_run_ms - 3_600_000 - 60_000),
    ] {
        fs::create_dir_all(ws.run_dir(id)).await.unwrap();
        fs::write(ws.run_dir(id).join("capture"), "data")
            .await
            .unwrap();
        fs::write(
            ws.run_dir(id).join("run.json"),
            serde_json::json!({"schema":1,"run_id":id,"status":"running","started_at_ms":started})
                .to_string(),
        )
        .await
        .unwrap();
        age_run_dir(&ws.run_dir(id));
    }
    enforce_project_quota(&temp.path().join("trajectories"), 0)
        .await
        .unwrap();
    assert!(ws.run_dir("live").exists());
    assert!(!ws.run_dir("crashed").exists());
}

#[tokio::test]
async fn running_metadata_is_written_atomically_and_guard_replaces_it() {
    let temp = tempfile::tempdir().unwrap();
    let ws = workspace(temp.path(), "chat");
    ws.ensure_dirs().await.unwrap();
    fs::create_dir_all(ws.run_dir("run")).await.unwrap();
    let labels = serde_json::json!({"description":"Count TODO markers","code_preview":"x = 1"});
    write_running_run_json(&ws.run_dir("run"), "run", 42, true, &labels)
        .await
        .unwrap();
    let data: serde_json::Value =
        serde_json::from_slice(&fs::read(ws.run_dir("run").join("run.json")).await.unwrap())
            .unwrap();
    assert_eq!(data["status"], "running");
    assert_eq!(data["schema"], 1);
    assert_eq!(data["background"], true);
    assert_eq!(data["description"], "Count TODO markers");
    assert_eq!(data["code_preview"], "x = 1");
    drop(FailedRunGuard::new(ws.run_dir("run"), "run".into(), 42, false).with_labels(labels));
    let data: serde_json::Value =
        serde_json::from_slice(&fs::read(ws.run_dir("run").join("run.json")).await.unwrap())
            .unwrap();
    assert_eq!(data["status"], "failed");
    assert_eq!(data["description"], "Count TODO markers");
    assert_eq!(data["code_preview"], "x = 1");
}

#[cfg(unix)]
#[tokio::test]
async fn quota_and_failed_guard_work_under_a_symlinked_ancestor() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    std::fs::create_dir_all(&real).unwrap();
    let link = temp.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let ws = workspace(&link, "chat");
    // Trajectory storage itself refuses symlinked ancestors outside macOS /tmp and /var,
    // so lay the tree out directly to exercise only quota and the abandoned-run guard.
    std::fs::create_dir_all(ws.artifacts_dir()).unwrap();
    fs::create_dir_all(ws.run_dir("old")).await.unwrap();
    fs::write(
        ws.run_dir("old").join("run.json"),
        br#"{"status":"completed","finished_at_ms":1}"#,
    )
    .await
    .unwrap();
    fs::create_dir_all(ws.run_dir("active")).await.unwrap();
    fs::write(
        ws.run_dir("active").join("run.json"),
        br#"{"status":"completed","finished_at_ms":1}"#,
    )
    .await
    .unwrap();
    let _active = ActiveRunPath::new(ws.run_dir("active"));
    enforce_project_quota(&link.join("trajectories"), 0)
        .await
        .unwrap();
    assert!(!ws.run_dir("old").exists());
    assert!(ws.run_dir("active").exists());
    fs::create_dir_all(ws.run_dir("abandoned")).await.unwrap();
    drop(FailedRunGuard::new(
        ws.run_dir("abandoned"),
        "abandoned".into(),
        1,
        false,
    ));
    assert!(ws.run_dir("abandoned").join("run.json").exists());
    std::fs::remove_dir_all(ws.dir().join("runs")).unwrap();
    std::os::unix::fs::symlink(temp.path(), ws.dir().join("runs")).unwrap();
    std::fs::create_dir_all(temp.path().join("victim")).unwrap();
    drop(FailedRunGuard::new(
        ws.dir().join("runs/victim"),
        "victim".into(),
        1,
        false,
    ));
    assert!(!temp.path().join("victim/run.json").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_fork_of_a_busy_source_leaves_out_artifacts_the_run_has_not_labelled_yet() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    std::fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(&real, temp.path().join("alias")).unwrap();
    let src = workspace(&temp.path().join("alias"), "src");
    std::fs::create_dir_all(src.artifacts_dir()).unwrap();
    std::fs::create_dir_all(src.runs_dir()).unwrap();
    let canonical = std::fs::canonicalize(src.artifacts_dir()).unwrap();
    assert_ne!(canonical, src.artifacts_dir());
    let old = src.artifacts_dir().join("old.txt");
    std::fs::write(&old, "written before the run").unwrap();
    filetime::set_file_mtime(&old, filetime::FileTime::from_unix_time(1, 0)).unwrap();
    let _busy = ActiveRunPath::new(src.run_dir("busy"));
    std::fs::write(src.artifacts_dir().join("new.txt"), "written by the run").unwrap();
    std::fs::write(src.artifacts_dir().join("labelled.txt"), "labelled").unwrap();
    let ledger = serde_json::json!([[canonical.join("labelled.txt"), {"zone":"blocked", "origin":"secret"}]]);
    std::fs::write(src.dir().join("privacy_ledger.json"), ledger.to_string()).unwrap();
    let dst = workspace(temp.path(), "dst");
    let outcome = src.fork_to(&dst).await.unwrap();
    assert_eq!(outcome.artifacts_skipped, 1);
    assert!(dst.artifacts_dir().join("old.txt").exists());
    assert!(dst.artifacts_dir().join("labelled.txt").exists());
    assert!(!dst.artifacts_dir().join("new.txt").exists());
    let copied: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dst.dir().join("privacy_ledger.json")).unwrap())
            .unwrap();
    assert_eq!(
        copied[0][0],
        dst.artifacts_dir()
            .join("labelled.txt")
            .to_string_lossy()
            .as_ref()
    );
}
