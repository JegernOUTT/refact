//! Real daemon/worker E2E suite.
//!
//! Run with:
//! `REFACT_SKIP_GUI_BUILD=1 cargo build && REFACT_DAEMON_E2E=1 REFACT_SKIP_GUI_BUILD=1 cargo test --test daemon_e2e -- --nocapture`
//!
//! Without `REFACT_DAEMON_E2E=1`, every test prints `skipped: set REFACT_DAEMON_E2E=1` and
//! returns before spawning the daemon. The tests set `REFACT_DAEMON_DIR` and `HOME` per process so
//! daemon state, config, worker logs, and engine user dirs stay inside a temp directory.

mod e2e_helpers;

use std::time::{Duration, SystemTime};

use chrono::{Datelike, Timelike, Utc};
use e2e_helpers::{e2e_enabled, make_project, print_skip, wait_for, DaemonProcess, E2eDirs};
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
    let mtime_before = std::fs::metadata(&tasks_path).unwrap().modified().unwrap();

    let daemon = DaemonProcess::start(&dirs).await;
    let opened = open_project(&daemon, project.path()).await;
    let project_id = opened["project_id"].as_str().unwrap().to_string();
    let stop = daemon
        .post_json(&format!("/daemon/v1/projects/{project_id}/stop"), json!({}))
        .await;
    assert!(
        stop.status().is_success(),
        "stop returned {}",
        stop.status()
    );

    wait_for(Duration::from_secs(30), || {
        let daemon = &daemon;
        async move {
            let status = daemon.status().await;
            (status["workers"].as_u64() == Some(0)).then_some(())
        }
    })
    .await;
    assert_eq!(
        std::fs::metadata(&tasks_path).unwrap().modified().unwrap(),
        mtime_before
    );

    let worker_started = wait_for(Duration::from_secs(120), || {
        let daemon = &daemon;
        async move {
            let status = daemon.status().await;
            (status["workers"].as_u64().unwrap_or_default() > 0).then_some(status)
        }
    })
    .await;
    assert!(
        worker_started["cron_pending"].get(&project_id).is_some()
            || Utc::now().timestamp_millis() as u64 >= target_ms
    );

    wait_for(Duration::from_secs(150), || {
        let tasks_path = tasks_path.clone();
        async move {
            let content = tokio::fs::read_to_string(&tasks_path).await.ok()?;
            let tasks: Vec<Value> = serde_json::from_str(&content).ok()?;
            if tasks.is_empty()
                || tasks.iter().any(|task| {
                    task["id"] == "cron_e2e_once"
                        && task["fire_count"].as_u64().unwrap_or_default() >= 1
                })
            {
                return Some(());
            }
            None
        }
    })
    .await;
    assert!(std::fs::metadata(&tasks_path).unwrap().modified().unwrap() > mtime_before);

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
        e2e_helpers::lsp_initialize(lsp_port, 1),
        e2e_helpers::lsp_initialize(lsp_port, 2)
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

#[test]
fn default_suite_skips_without_env() {
    if e2e_enabled() {
        return;
    }
    print_skip();
    let now = SystemTime::now();
    assert!(now.elapsed().unwrap() < Duration::from_secs(1));
}
