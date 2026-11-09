//! Real daemon/worker E2E suite.
//!
//! Run with:
//! `REFACT_SKIP_GUI_BUILD=1 cargo build && REFACT_DAEMON_E2E=1 REFACT_SKIP_GUI_BUILD=1 cargo test --test it -- daemon_e2e:: --nocapture`
//!
//! Without `REFACT_DAEMON_E2E=1`, every test prints `skipped: set REFACT_DAEMON_E2E=1` and
//! returns before spawning the daemon. The tests set `REFACT_DAEMON_DIR` and `HOME` per process so
//! daemon state, config, worker logs, and engine user dirs stay inside a temp directory.

use std::time::Duration;

use chrono::{Datelike, Timelike, Utc};
use crate::e2e_helpers::{e2e_enabled, make_project, print_skip, wait_for, DaemonProcess, E2eDirs};
use serde_json::{json, Value};

fn future_minute_cron(offset_secs: i64) -> (String, u64) {
    let tz = refact_lsp::scheduler::scheduler_timezone();
    let target = Utc::now().with_timezone(&tz) + chrono::Duration::seconds(offset_secs);
    let target = target
        .with_second(0)
        .and_then(|dt| dt.with_nanosecond(0))
        .unwrap()
        + chrono::Duration::minutes(1);
    let cron = format!(
        "{} {} {} {} *",
        target.minute(),
        target.hour(),
        target.day(),
        target.month()
    );
    (cron, target.timestamp_millis() as u64)
}

fn past_minute_cron() -> (String, u64, u64) {
    let tz = refact_lsp::scheduler::scheduler_timezone();
    let target = (Utc::now().with_timezone(&tz) - chrono::Duration::minutes(1))
        .with_second(0)
        .and_then(|dt| dt.with_nanosecond(0))
        .unwrap();
    let created = target - chrono::Duration::minutes(1);
    let cron = format!(
        "{} {} {} {} *",
        target.minute(),
        target.hour(),
        target.day(),
        target.month()
    );
    (
        cron,
        created.timestamp_millis() as u64,
        target.timestamp_millis() as u64,
    )
}

fn fake_worker_cmd() -> Option<String> {
    let python = std::env::var("PYTHON3").unwrap_or_else(|_| "python3".to_string());
    if std::process::Command::new(&python)
        .arg("--version")
        .output()
        .is_err()
    {
        return None;
    }
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fake_worker.py");
    Some(shell_words::join([
        python.as_str(),
        script.to_string_lossy().as_ref(),
    ]))
}

fn durable_task(cron: &str, created_at_ms: u64, chat_id: &str) -> Value {
    json!({
        "id": "cron_e2e_once",
        "cron": cron,
        "prompt": "durable cron e2e prompt",
        "description": "durable cron e2e",
        "recurring": false,
        "durable": true,
        "created_at_ms": created_at_ms,
        "chat_id": chat_id,
        "mode": "agent",
        "last_fired_at_ms": null,
        "fire_count": 0,
        "auto_expire_after_ms": 0
    })
}

fn write_minimal_trajectory(project_root: &std::path::Path, chat_id: &str) {
    let dir = project_root.join(".refact").join("trajectories");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{chat_id}.json")),
        serde_json::to_vec_pretty(&json!({
            "id": chat_id,
            "title": "Cron E2E",
            "model": "",
            "mode": "agent",
            "tool_use": "agent",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "messages": [
                {"role": "user", "content": "seed cron session"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_durable_task(project_root: &std::path::Path, task: &Value) -> std::path::PathBuf {
    let path = project_root.join(".refact").join("scheduled_tasks.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec_pretty(&json!([task])).unwrap()).unwrap();
    path
}

async fn open_project(daemon: &DaemonProcess, root: &std::path::Path) -> Value {
    let response = daemon
        .post_json(
            "/daemon/v1/projects/open",
            json!({
                "root": root.to_string_lossy(),
                "client_kind": "e2e",
                "settings": {
                    "ast": false,
                    "vecdb": false,
                    "ast_max_files": 0,
                    "vecdb_max_files": 0
                }
            }),
        )
        .await;
    assert!(
        response.status().is_success(),
        "open returned {}",
        response.status()
    );
    response.json::<Value>().await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_worker_smoke_lifecycle_through_daemon() {
    if !e2e_enabled() {
        print_skip();
        return;
    }
    let dirs = E2eDirs::new();
    dirs.write_daemon_config(1800);
    let project = make_project("daemon-smoke");
    let daemon = DaemonProcess::start(&dirs).await;

    let opened = open_project(&daemon, project.path()).await;
    let project_id = opened["project_id"].as_str().unwrap().to_string();
    assert_eq!(opened["worker"]["state"], "ready");

    let ping = daemon.get(&format!("/p/{project_id}/v1/ping")).await;
    assert!(
        ping.status().is_success(),
        "ping returned {}",
        ping.status()
    );
    assert!(!ping.text().await.unwrap().trim().is_empty());

    let status = daemon.status().await;
    assert_eq!(status["workers"], 1);

    let stopped = daemon
        .post_json(&format!("/daemon/v1/projects/{project_id}/stop"), json!({}))
        .await;
    assert!(
        stopped.status().is_success(),
        "stop returned {}",
        stopped.status()
    );
    let status = wait_for(Duration::from_secs(30), || {
        let daemon = &daemon;
        async move {
            let status = daemon.status().await;
            (status["workers"].as_u64() == Some(0)).then_some(status)
        }
    })
    .await;
    assert_eq!(status["workers"], 0);

    daemon.shutdown().await;
}

// Only control-plane reads are allowed until the restarted daemon proves cron wake.
async fn wait_for_cold_cron_worker(
    daemon: &DaemonProcess,
    project_id: &str,
    old_pid: u64,
    task_id: &str,
    timeout: Duration,
) {
    wait_for(timeout, || async {
        let response = daemon.get("/daemon/v1/events").await;
        let text = response.text().await.ok()?;
        // A restarted daemon has a fresh in-memory event bus; disk history is not replayed.
        let woke = text
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .any(|event| {
                event["kind"] == "cron_wake"
                    && event["project_id"] == project_id
                    && event["payload"]["task_id"] == task_id
            });
        if !woke {
            return None;
        }
        let rows = daemon
            .get("/daemon/v1/workers")
            .await
            .json::<Vec<Value>>()
            .await
            .ok()?;
        rows.iter()
            .any(|row| {
                row["project_id"] == project_id
                    && row["state"] == "ready"
                    && row["pid"].as_u64().is_some_and(|pid| pid != old_pid)
            })
            .then_some(())
    })
    .await;
    let project = daemon
        .get(&format!("/daemon/v1/projects/{project_id}"))
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(project["automation_suspended"], false);
}

async fn ready_worker_cron_rows(daemon: &DaemonProcess, project_id: &str) -> Vec<Value> {
    // This public projection opens the durable store, not a strict read-only observer.
    // Call only while readiness is already independently proven; never use it to wake.
    let response = daemon
        .get(&format!("/p/{project_id}/v1/scheduler/cron"))
        .await;
    assert!(response.status().is_success());
    response.json().await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn durable_cron_fires_without_clients_attached() {
    if !e2e_enabled() {
        print_skip();
        return;
    }
    let dirs = E2eDirs::new();
    dirs.write_daemon_config(1800);
    let project = make_project("daemon-cron");
    let chat_id = "cron-e2e-chat";
    write_minimal_trajectory(project.path(), chat_id);
    let (cron, target_ms) = future_minute_cron(75);
    let created_at_ms = (Utc::now() - chrono::Duration::minutes(1)).timestamp_millis() as u64;
    let tasks_path =
        write_durable_task(project.path(), &durable_task(&cron, created_at_ms, chat_id));
    let fixture_bytes = std::fs::read(&tasks_path).unwrap();
    let daemon = DaemonProcess::start(&dirs).await;
    let opened = open_project(&daemon, project.path()).await;
    assert_eq!(opened["worker"]["state"], "ready");
    let project_id = opened["project_id"].as_str().unwrap().to_string();
    let old_pid = opened["worker"]["pid"].as_u64().unwrap();
    let baseline = ready_worker_cron_rows(&daemon, &project_id).await;
    let baseline = baseline
        .iter()
        .find(|row| row["id"] == "cron_e2e_once")
        .expect("known migrated fixture must exist before firing");
    assert_eq!(baseline["fire_count"], 0);
    assert!(baseline["last_fired_at_ms"].is_null());
    let backup = tasks_path.with_file_name("scheduled_tasks.migrated.json");
    assert!(
        !tasks_path.exists(),
        "real worker retires legacy JSON authority"
    );
    assert_eq!(std::fs::read(&backup).unwrap(), fixture_bytes);

    // Daemon shutdown kills its workers but does NOT persist an explicit project-stop fence.
    daemon.shutdown().await;
    assert!(
        (Utc::now().timestamp_millis() as u64) < target_ms,
        "setup missed scheduled occurrence"
    );
    let daemon = DaemonProcess::start(&dirs).await;
    wait_for_cold_cron_worker(
        &daemon,
        &project_id,
        old_pid,
        "automation",
        Duration::from_secs(120),
    )
    .await;

    // One-shot rows are removed on fire. Empty schedules alone are NEVER evidence of firing:
    // require the exact delivered cron event from the persisted fixture trajectory as well.
    let trajectory = project
        .path()
        .join(".refact/trajectories")
        .join(format!("{chat_id}.json"));
    wait_for(Duration::from_secs(150), || async {
        let bytes = tokio::fs::read(&trajectory).await.ok()?;
        let saved: Value = serde_json::from_slice(&bytes).ok()?;
        let delivered = saved["messages"].as_array()?.iter().any(|message| {
            let event = &message["event"];
            event["subkind"] == "cron_fire"
                && event["payload"]["task_id"] == "cron_e2e_once"
                && event["payload"]["recurring"] == false
        });
        (delivered && Utc::now().timestamp_millis() as u64 >= target_ms).then_some(())
    })
    .await;
    let rows = ready_worker_cron_rows(&daemon, &project_id).await;
    assert!(
        rows.iter()
            .find(|row| row["id"] == "cron_e2e_once")
            .map(|row| row["fire_count"].as_u64().unwrap_or_default() >= 1)
            .unwrap_or(true),
        "known job must have fired or been removed after exact delivery"
    );
    assert_eq!(std::fs::read(&backup).unwrap(), fixture_bytes);
    daemon.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn past_due_durable_cron_wakes_dead_worker_without_clients() {
    if !e2e_enabled() {
        print_skip();
        return;
    }
    let Some(worker_cmd) = fake_worker_cmd() else {
        print_skip();
        return;
    };
    let dirs = E2eDirs::new();
    dirs.write_daemon_config(1800);
    let project = make_project("daemon-cron-past-due");
    let chat_id = "cron-e2e-past-due-chat";
    write_minimal_trajectory(project.path(), chat_id);
    let (cron, created_at_ms, target_ms) = past_minute_cron();
    assert!(target_ms < Utc::now().timestamp_millis() as u64);
    let tasks_path =
        write_durable_task(project.path(), &durable_task(&cron, created_at_ms, chat_id));
    let fixture_bytes = std::fs::read(&tasks_path).unwrap();
    let mtime_before = std::fs::metadata(&tasks_path).unwrap().modified().unwrap();
    let env = [
        ("REFACT_DAEMON_WORKER_CMD", worker_cmd.as_str()),
        ("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS", "1"),
    ];
    let daemon = DaemonProcess::start_with_extra_env(&dirs, &env).await;
    let opened = open_project(&daemon, project.path()).await;
    assert_eq!(opened["worker"]["state"], "ready");
    let project_id = opened["project_id"].as_str().unwrap().to_string();
    let old_pid = opened["worker"]["pid"].as_u64().unwrap();
    daemon.shutdown().await;
    let daemon = DaemonProcess::start_with_extra_env(&dirs, &env).await;
    wait_for_cold_cron_worker(
        &daemon,
        &project_id,
        old_pid,
        "cron_e2e_once",
        Duration::from_secs(75),
    )
    .await;
    let status = daemon.status().await;
    assert_eq!(status["workers"], 1);
    assert!(status["cron_pending"].get(&project_id).is_some());
    assert_eq!(
        std::fs::read(&tasks_path).unwrap(),
        fixture_bytes,
        "fake-worker e2e proves wake only; real worker owns migration and firing"
    );
    assert_eq!(
        std::fs::metadata(&tasks_path).unwrap().modified().unwrap(),
        mtime_before
    );
    let rows: Vec<Value> = serde_json::from_slice(&fixture_bytes).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "cron_e2e_once");
    assert_eq!(rows[0]["fire_count"], 0);
    daemon.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_lsp_tcp_accepts_two_clients_concurrently() {
    if !e2e_enabled() {
        print_skip();
        return;
    }
    let dirs = E2eDirs::new();
    dirs.write_daemon_config(1800);
    let project = make_project("daemon-lsp");
    let daemon = DaemonProcess::start(&dirs).await;
    let opened = open_project(&daemon, project.path()).await;
    let lsp_port = opened["worker"]["lsp_port"].as_u64().unwrap();

    let (first, second) = tokio::join!(
        crate::e2e_helpers::lsp_initialize(lsp_port, 1),
        crate::e2e_helpers::lsp_initialize(lsp_port, 2)
    );
    assert_eq!(first["id"], 1);
    assert!(first.get("result").is_some(), "first response: {first}");
    assert_eq!(second["id"], 2);
    assert!(second.get("result").is_some(), "second response: {second}");

    daemon.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_stop_then_proxy_rewakes_worker() {
    if !e2e_enabled() {
        print_skip();
        return;
    }
    let dirs = E2eDirs::new();
    dirs.write_daemon_config(5);
    let project = make_project("daemon-idle");
    let daemon = DaemonProcess::start(&dirs).await;
    let opened = open_project(&daemon, project.path()).await;
    let project_id = opened["project_id"].as_str().unwrap().to_string();
    let first_pid = opened["worker"]["pid"].as_u64();

    wait_for(Duration::from_secs(90), || {
        let daemon = &daemon;
        async move {
            let status = daemon.status().await;
            (status["workers"].as_u64() == Some(0)).then_some(())
        }
    })
    .await;

    let ping = daemon.get(&format!("/p/{project_id}/v1/ping")).await;
    assert!(
        ping.status().is_success(),
        "ping returned {}",
        ping.status()
    );
    assert!(!ping.text().await.unwrap().trim().is_empty());

    let status = wait_for(Duration::from_secs(30), || {
        let daemon = &daemon;
        let project_id = project_id.clone();
        async move {
            let project = daemon
                .get(&format!("/daemon/v1/projects/{project_id}"))
                .await
                .json::<Value>()
                .await
                .ok()?;
            let status = daemon.status().await;
            (status["workers"].as_u64() == Some(1)).then_some(project)
        }
    })
    .await;
    assert_eq!(status["id"], project_id);
    let worker = daemon.get(&format!("/p/{project_id}/v1/ping")).await;
    assert!(worker.status().is_success());
    let current_status = daemon.status().await;
    assert_eq!(current_status["workers"], 1);
    if let Some(first_pid) = first_pid {
        let opened_again = open_project(&daemon, project.path()).await;
        assert_ne!(opened_again["worker"]["pid"].as_u64(), Some(first_pid));
    }

    daemon.shutdown().await;
}
