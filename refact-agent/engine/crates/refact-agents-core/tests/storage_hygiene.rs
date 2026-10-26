use std::path::Path;
use std::time::{Duration, SystemTime};

use chrono::{TimeDelta, Utc};
use refact_agents_core::storage::{load_all, save_all};
use refact_agents_core::{
    BackgroundAgent, BackgroundAgentRegistry, BgAgentKind, BgAgentStatus, CreateAgentRequest,
};
use refact_work_core::{coordination_settings, install_coordination_settings, CoordinationSettings};

async fn seed_record(path: &Path) -> BackgroundAgent {
    let registry = BackgroundAgentRegistry::new(path.to_path_buf())
        .await
        .unwrap();
    registry
        .create(CreateAgentRequest {
            parent_chat_id: "parent".into(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "test".into(),
            title: "Storage hygiene".into(),
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
        .unwrap()
        .0
}

fn write_file(path: &Path, modified: SystemTime) {
    let mut file = std::fs::File::create(path).unwrap();
    std::io::Write::write_all(&mut file, b"keep or sweep").unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
}

#[tokio::test]
async fn stale_tmp_files_are_removed_on_load() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let now = SystemTime::now();
    let old = now - Duration::from_secs(7200);
    let foreign_pid = std::process::id().wrapping_add(1);
    let stale = root.join(format!("records.json.{foreign_pid}.1.tmp"));
    let fresh = root.join(format!("records.json.{foreign_pid}.2.tmp"));
    let current = root.join(format!("records.json.{}.3.tmp", std::process::id()));
    let unknown = root.join("records.json.unknown.4.tmp");
    let unknown_fresh = root.join("records.json.unknown.5.tmp");
    let quarantine = root.join("records.json.unreadable-20260101T000000Z");
    let partial_quarantine = root.join("records.unreadable-20260101T000000Z.json");
    let unrelated = root.join("unrelated.tmp");
    let other_staging = root.join(format!("other.json.{foreign_pid}.1.tmp"));
    let directory = root.join(format!("records.json.{foreign_pid}.directory.tmp"));
    for path in [
        &stale,
        &current,
        &unknown,
        &quarantine,
        &partial_quarantine,
        &unrelated,
        &other_staging,
    ] {
        write_file(path, old);
    }
    for path in [&fresh, &unknown_fresh] {
        write_file(path, now);
    }
    std::fs::create_dir(&directory).unwrap();
    #[cfg(unix)]
    {
        std::fs::File::open(&directory)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
    }
    #[cfg(unix)]
    let symlink = {
        let path = root.join(format!("records.json.{foreign_pid}.symlink.tmp"));
        std::os::unix::fs::symlink(&unrelated, &path).unwrap();
        path
    };

    let registry = BackgroundAgentRegistry::new(root.to_path_buf())
        .await
        .unwrap();

    assert!(!stale.exists(), "stale foreign-pid tmp was not removed");
    assert!(!unknown.exists(), "stale unknown-pid tmp was not removed");
    for path in [
        &fresh,
        &current,
        &unknown_fresh,
        &quarantine,
        &partial_quarantine,
        &unrelated,
        &other_staging,
    ] {
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"keep or sweep",
            "changed {path:?}"
        );
    }
    assert!(directory.is_dir());
    #[cfg(unix)]
    assert!(std::fs::symlink_metadata(symlink)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(registry.quarantine_report().is_none());
    assert!(!root.join("records.json").exists());
}

struct RestoreSettings(CoordinationSettings);

impl Drop for RestoreSettings {
    fn drop(&mut self) {
        install_coordination_settings(self.0.clone()).unwrap();
    }
}

#[tokio::test]
async fn retention_keeps_n_recent_terminal_records_and_all_live_ones() {
    let _restore = RestoreSettings(coordination_settings());
    for retained in [
        73,
        CoordinationSettings::default().agent_terminal_retained_max,
    ] {
        let mut settings = coordination_settings();
        settings.agent_terminal_retained_max = retained;
        install_coordination_settings(settings).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let seed = seed_record(temp.path()).await;
        let now = Utc::now();
        let mut records = Vec::new();
        let terminal_count = retained + 17;
        for index in 0..terminal_count {
            let mut record = seed.clone();
            record.agent_id = format!("terminal-{index}");
            record.status = [
                BgAgentStatus::Completed,
                BgAgentStatus::Failed,
                BgAgentStatus::Cancelled,
                BgAgentStatus::Interrupted,
                BgAgentStatus::Closed,
            ][index % 5];
            record.created_at = now - TimeDelta::minutes(index as i64);
            let finished = now - TimeDelta::days(40) + TimeDelta::seconds(index as i64);
            record.finished_at = (index % 2 == 0).then_some(finished);
            record.last_update_at = if record.finished_at.is_some() {
                now + TimeDelta::days(40)
            } else {
                finished
            };
            records.push(record);
        }
        let mut pending = records[0].clone();
        pending.agent_id = "pending".into();
        pending
            .pending_deliveries
            .push(refact_chat_api::PendingDelivery::new(
                Vec::new(),
                refact_chat_api::PushMode::Append,
                "test",
                false,
            ));
        records.push(pending.clone());
        let live_count = retained + 11;
        let live: Vec<_> = (0..live_count)
            .map(|index| {
                let mut record = seed.clone();
                record.agent_id = format!("live-{index}");
                record.status = [
                    BgAgentStatus::Queued,
                    BgAgentStatus::Running,
                    BgAgentStatus::WaitingForApproval,
                ][index % 3];
                record.created_at = now - TimeDelta::days(100);
                record.last_update_at = record.created_at;
                record
            })
            .collect();
        records.extend(live.iter().cloned());
        records.reverse();

        save_all(temp.path(), records).await.unwrap();
        let loaded = load_all(temp.path()).await.unwrap();

        assert_eq!(loaded.len(), retained + live_count + 1);
        for index in 0..terminal_count {
            assert_eq!(
                loaded.contains_key(&format!("terminal-{index}")),
                index >= terminal_count - retained,
                "incorrect terminal retention for {index} with N={retained}",
            );
        }
        for record in live {
            assert_eq!(loaded[&record.agent_id], record);
        }
        assert_eq!(loaded["pending"], pending);
    }
}
