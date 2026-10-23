use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde_json::{json, Value};

use crate::{storage, BackgroundAgent, BackgroundAgentRegistry, BgAgentKind, CreateAgentRequest};

async fn healthy_record() -> BackgroundAgent {
    let temp = tempfile::tempdir().unwrap();
    let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .unwrap();
    let (record, _, _) = registry
        .create(CreateAgentRequest {
            parent_chat_id: "parent".into(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "test".into(),
            title: "Healthy".into(),
            prompt: "Continue".into(),
            target_files: Vec::new(),
            model: "test-model".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap();
    registry
        .mark_failed(&record.agent_id, "done".into())
        .await
        .unwrap()
}

fn empty_legacy_answer(healthy: &BackgroundAgent) -> Value {
    let mut bad = serde_json::to_value(healthy).unwrap();
    bad["schema_version"] = json!(1);
    bad["agent_id"] = json!("bgagent-bad");
    bad["unknown_metadata"] = json!({"preserve": [true, 42, "exact"]});
    bad["questions"] = json!([{
        "id": "empty-answer", "text": "Continue?", "asked_at": Utc::now(),
        "answer": "", "answered_at": Utc::now()
    }]);
    bad
}

async fn write_snapshot(root: &Path, snapshot: &Value) {
    tokio::fs::write(
        root.join("records.json"),
        serde_json::to_vec(snapshot).unwrap(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn one_bad_record_is_quarantined_and_the_rest_load() {
    let (logs, _subscriber) = capture_logs();
    let temp = tempfile::tempdir().unwrap();
    let healthy = healthy_record().await;
    let bad = empty_legacy_answer(&healthy);
    write_snapshot(temp.path(), &json!([healthy, bad])).await;

    let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .unwrap_or_else(|error| panic!("registry must isolate the unreadable record: {error}"));

    assert_eq!(registry.list_all().await, vec![healthy.clone()]);
    assert!(registry.get_any("bgagent-bad").await.is_err());
    let report = registry.quarantine_report().unwrap();
    assert_eq!(report.record_count, Some(1));
    assert!(!report.first_reason.is_empty());
    let name = report.path.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("records.unreadable-"));
    assert!(name.ends_with(".json"));
    chrono::NaiveDateTime::parse_from_str(
        &name["records.unreadable-".len()..name.len() - ".json".len()],
        "%Y%m%dT%H%M%SZ",
    )
    .unwrap();
    let sidecar: Value =
        serde_json::from_slice(&tokio::fs::read(&report.path).await.unwrap()).unwrap();
    assert_eq!(
        sidecar,
        json!([{"reason": report.first_reason, "record": bad}])
    );
    assert_eq!(storage::load_all(temp.path()).await.unwrap().len(), 1);
    assert_eq!(
        serde_json::from_slice::<Value>(
            &tokio::fs::read(temp.path().join("records.json"))
                .await
                .unwrap()
        )
        .unwrap(),
        json!([healthy])
    );
    let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logged.lines().count(), 1, "{logged}");
    assert!(logged.contains("ERROR"), "{logged}");
    assert!(
        logged.contains(&report.path.display().to_string()),
        "{logged}"
    );
    assert!(logged.contains("Record count: 1"), "{logged}");
}

#[tokio::test]
async fn unparseable_file_is_renamed_and_registry_starts_empty() {
    for bytes in [b"[{invalid\xFF".as_slice(), b"{}", b"", b"null"] {
        let temp = tempfile::tempdir().unwrap();
        tokio::fs::write(temp.path().join("records.json"), bytes)
            .await
            .unwrap();
        let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
            .await
            .unwrap();
        assert!(registry.list_all().await.is_empty());
        let report = registry.quarantine_report().unwrap();
        assert_eq!(report.record_count, None);
        assert!(report
            .path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("records.json.unreadable-"));
        assert_eq!(tokio::fs::read(&report.path).await.unwrap(), bytes);
        assert!(!temp.path().join("records.json").exists());
        let restarted = BackgroundAgentRegistry::new(temp.path().to_path_buf())
            .await
            .unwrap();
        assert!(restarted.list_all().await.is_empty());
        assert_eq!(tokio::fs::read(&report.path).await.unwrap(), bytes);
    }
}

#[tokio::test]
async fn saves_after_quarantine_never_touch_the_quarantined_data() {
    let healthy = healthy_record().await;
    for snapshot in [
        serde_json::to_vec(&json!([healthy, empty_legacy_answer(&healthy)])).unwrap(),
        b"not json".to_vec(),
    ] {
        let temp = tempfile::tempdir().unwrap();
        tokio::fs::write(temp.path().join("records.json"), snapshot)
            .await
            .unwrap();
        let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
            .await
            .unwrap();
        let report = registry.quarantine_report().unwrap();
        let original = tokio::fs::read(&report.path).await.unwrap();
        let modified = tokio::fs::metadata(&report.path)
            .await
            .unwrap()
            .modified()
            .unwrap();
        for _ in 0..3 {
            storage::save_all(temp.path(), vec![healthy.clone()])
                .await
                .unwrap();
            registry.flush_pending_writes_for_test().await.unwrap();
            assert_eq!(tokio::fs::read(&report.path).await.unwrap(), original);
            assert_eq!(
                tokio::fs::metadata(&report.path)
                    .await
                    .unwrap()
                    .modified()
                    .unwrap(),
                modified
            );
        }
        write_snapshot(
            temp.path(),
            &json!([healthy, empty_legacy_answer(&healthy)]),
        )
        .await;
        let second = BackgroundAgentRegistry::new(temp.path().to_path_buf())
            .await
            .unwrap();
        assert_ne!(second.quarantine_report().unwrap().path, report.path);
        assert_eq!(tokio::fs::read(&report.path).await.unwrap(), original);
    }
}

#[tokio::test]
async fn valid_records_file_is_never_quarantined() {
    let temp = tempfile::tempdir().unwrap();
    let healthy = healthy_record().await;
    for snapshot in [json!([]), json!([healthy])] {
        write_snapshot(temp.path(), &snapshot).await;
        let path = temp.path().join("records.json");
        let before = tokio::fs::read(&path).await.unwrap();
        let modified = tokio::fs::metadata(&path)
            .await
            .unwrap()
            .modified()
            .unwrap();
        let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
            .await
            .unwrap();
        assert!(registry.quarantine_report().is_none());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), before);
        assert_eq!(
            tokio::fs::metadata(&path)
                .await
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }
}

#[tokio::test]
async fn infrastructure_errors_still_fail_registry_initialization() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("not-a-directory");
    tokio::fs::write(&root, "occupied").await.unwrap();
    let error = BackgroundAgentRegistry::new(root).await.err().unwrap();
    assert!(error.contains("Failed to create background agent registry directory"));
    tokio::fs::create_dir(temp.path().join("records.json"))
        .await
        .unwrap();
    let error = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .err()
        .unwrap();
    assert!(error.contains("Failed to read background agents file"));
}

#[cfg(unix)]
#[tokio::test]
async fn quarantine_io_failure_preserves_the_original_and_fails_closed() {
    use std::os::unix::fs::PermissionsExt;

    let healthy = healthy_record().await;
    for snapshot in [
        serde_json::to_vec(&json!([healthy, empty_legacy_answer(&healthy)])).unwrap(),
        b"not json".to_vec(),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("records.json");
        tokio::fs::write(&path, &snapshot).await.unwrap();
        let original_permissions = std::fs::metadata(temp.path()).unwrap().permissions();
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = BackgroundAgentRegistry::new(temp.path().to_path_buf()).await;
        std::fs::set_permissions(temp.path(), original_permissions).unwrap();
        let error = result
            .err()
            .expect("quarantine I/O must still fail initialization");
        assert!(error.contains("quarantine"), "{error}");
        assert_eq!(tokio::fs::read(&path).await.unwrap(), snapshot);
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capture_logs() -> (LogBuffer, tracing::subscriber::DefaultGuard) {
    let logs = LogBuffer::default();
    let writer = logs.clone();
    let guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::ERROR)
            .with_writer(move || writer.clone())
            .finish(),
    );
    (logs, guard)
}

#[tokio::test]
async fn debounced_save_error_is_logged_and_retried() {
    let (logs, _subscriber) = capture_logs();
    let temp = tempfile::tempdir().unwrap();
    let healthy = healthy_record().await;
    write_snapshot(temp.path(), &json!([healthy])).await;
    let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .unwrap();
    let (record, _, _) = registry
        .create(CreateAgentRequest {
            parent_chat_id: "parent".into(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "test".into(),
            title: "Retry".into(),
            prompt: "Continue".into(),
            target_files: vec![],
            model: "model".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap();
    registry
        .update_progress(&record.agent_id, "latest".into(), 7)
        .await
        .unwrap();
    let records_path = temp.path().join("records.json");
    tokio::fs::remove_file(&records_path).await.unwrap();
    tokio::fs::create_dir(&records_path).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if String::from_utf8_lossy(&logs.0.lock().unwrap())
                .contains(&records_path.display().to_string())
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let logged = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("ERROR"), "{logged}");
    assert!(
        logged.contains(&format!(
            "Failed to save background agents at {}:",
            records_path.display()
        )),
        "{logged}"
    );
    assert!(logged.contains("retrying in 10 seconds"), "{logged}");
    tokio::fs::remove_dir(&records_path).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if records_path.is_file() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let saved = storage::load_all(temp.path()).await.unwrap();
    assert_eq!(saved[&record.agent_id].progress.as_deref(), Some("latest"));
    assert_eq!(saved[&record.agent_id].step_count, 7);
}
