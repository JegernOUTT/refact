use std::path::PathBuf;
use std::time::{Duration, Instant};

use refact_lsp::daemon::config::DaemonConfig;
use refact_lsp::daemon::events::EventBus;
use refact_lsp::daemon::projects::{ProjectEntry, ProjectSettings};
use refact_lsp::daemon::state::DaemonState;
use refact_lsp::daemon::supervisor::{Supervisor, WorkerInfo, WorkerState};
use serial_test::serial;
use serde_json::Value;
use tempfile::tempdir;

struct EnvGuard {
    keys: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn set(fake_crash: bool) -> Option<Self> {
        Self::set_with_port_busy(fake_crash, None)
    }

    fn set_with_port_busy(fake_crash: bool, port_busy_exit: Option<&str>) -> Option<Self> {
        let python = std::env::var("PYTHON3").unwrap_or_else(|_| "python3".to_string());
        if std::process::Command::new(&python)
            .arg("--version")
            .output()
            .is_err()
        {
            return None;
        }
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake_worker.py");
        let previous = vec![
            (
                "REFACT_DAEMON_WORKER_CMD",
                std::env::var("REFACT_DAEMON_WORKER_CMD").ok(),
            ),
            (
                "REFACT_DAEMON_SUPERVISOR_BACKOFF_MS",
                std::env::var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS").ok(),
            ),
            ("FAKE_WORKER_CRASH", std::env::var("FAKE_WORKER_CRASH").ok()),
            (
                "FAKE_WORKER_PORT_BUSY_EXIT",
                std::env::var("FAKE_WORKER_PORT_BUSY_EXIT").ok(),
            ),
            (
                "FAKE_WORKER_PUSH_STATUS",
                std::env::var("FAKE_WORKER_PUSH_STATUS").ok(),
            ),
            (
                "FAKE_WORKER_SKIP_LSP",
                std::env::var("FAKE_WORKER_SKIP_LSP").ok(),
            ),
            (
                "FAKE_WORKER_DELAY_READY",
                std::env::var("FAKE_WORKER_DELAY_READY").ok(),
            ),
            (
                "FAKE_WORKER_GRACEFUL_DELAY",
                std::env::var("FAKE_WORKER_GRACEFUL_DELAY").ok(),
            ),
        ];
        std::env::set_var(
            "REFACT_DAEMON_WORKER_CMD",
            shell_words::join([python.as_str(), script.to_string_lossy().as_ref()]),
        );
        std::env::set_var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS", "1");
        if fake_crash {
            std::env::set_var("FAKE_WORKER_CRASH", "1");
        } else {
            std::env::remove_var("FAKE_WORKER_CRASH");
        }
        match port_busy_exit {
            Some(code) => std::env::set_var("FAKE_WORKER_PORT_BUSY_EXIT", code),
            None => std::env::remove_var("FAKE_WORKER_PORT_BUSY_EXIT"),
        }
        std::env::remove_var("FAKE_WORKER_PUSH_STATUS");
        std::env::remove_var("FAKE_WORKER_SKIP_LSP");
        std::env::remove_var("FAKE_WORKER_DELAY_READY");
        std::env::remove_var("FAKE_WORKER_GRACEFUL_DELAY");
        Some(Self { keys: previous })
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.keys.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn project_entry(root: PathBuf, id: &str) -> ProjectEntry {
    ProjectEntry {
        id: id.to_string(),
        slug: id.to_string(),
        root,
        pinned: false,
        last_active_ms: 0,
        settings: ProjectSettings::default(),
    }
}

fn supervisor(dir: &tempfile::TempDir) -> std::sync::Arc<Supervisor> {
    Supervisor::new(
        EventBus::new(dir.path().join("events.jsonl")),
        dir.path().join("daemon"),
        8488,
    )
}

async fn wait_for_ready_with_new_pid(
    supervisor: &Supervisor,
    project_id: &str,
    old_pid: u32,
) -> WorkerInfo {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(info) = supervisor.worker_info(project_id).await {
            if matches!(info.state, WorkerState::Ready) && info.pid != Some(old_pid) {
                return info;
            }
        }
        assert!(Instant::now() < deadline, "worker did not restart in time");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_state_with_timeout(
    supervisor: &Supervisor,
    project_id: &str,
    state: WorkerState,
    timeout: Duration,
) -> WorkerInfo {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(info) = supervisor.worker_info(project_id).await {
            if info.state == state.clone() {
                return info;
            }
        }
        assert!(
            Instant::now() < deadline,
            "worker state {:?} was not reached in time",
            state
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_pid_exit(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if !pid_is_running(pid) {
            return;
        }
        assert!(Instant::now() < deadline, "pid {pid} did not exit in time");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }

    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
    }
}

fn pid_is_running(pid: u32) -> bool {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, 0) == 0
    }

    #[cfg(windows)]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        output
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|stdout| stdout.contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn ensure_worker_reaches_ready() {
    let Some(_env) = EnvGuard::set(false) else {
        return;
    };
    std::env::set_var("FAKE_WORKER_PUSH_STATUS", "1");
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "ready-project");
    let supervisor = supervisor(&dir);

    let info = supervisor.ensure_worker(&entry).await.unwrap();
    assert_eq!(info.state, WorkerState::Ready);
    assert!(info.pid.is_some());
    assert!(info.http_port > 0);
    assert!(info.lsp_port > 0);

    supervisor.stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn readiness_requires_lsp_port_open() {
    let Some(_env) = EnvGuard::set(false) else {
        return;
    };
    std::env::set_var("FAKE_WORKER_SKIP_LSP", "1");
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "no-lsp-project");
    let supervisor = supervisor(&dir);
    let start = tokio::spawn({
        let supervisor = supervisor.clone();
        let entry = entry.clone();
        async move { supervisor.ensure_worker(&entry).await }
    });

    let starting = wait_for_state_with_timeout(
        &supervisor,
        &entry.id,
        WorkerState::Starting,
        Duration::from_secs(5),
    )
    .await;
    assert!(starting.lsp_port > 0);
    tokio::time::sleep(Duration::from_millis(700)).await;
    let info = supervisor.worker_info(&entry.id).await.unwrap();
    assert_eq!(info.state, WorkerState::Starting);
    supervisor.request_shutdown("test shutdown".to_string());
    let info = tokio::time::timeout(Duration::from_secs(10), start)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(info.state, WorkerState::Failed { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn concurrent_ensure_spawns_one_process() {
    let Some(_env) = EnvGuard::set(false) else {
        return;
    };
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "concurrent-project");
    let events = EventBus::new(dir.path().join("events.jsonl"));
    let supervisor = Supervisor::new(events.clone(), dir.path().join("daemon"), 8488);

    let (first, second) = tokio::join!(
        supervisor.ensure_worker(&entry),
        supervisor.ensure_worker(&entry)
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.state, WorkerState::Ready);
    assert_eq!(first.pid, second.pid);
    let starting = events
        .snapshot()
        .await
        .into_iter()
        .filter(|event| event.kind == "worker_starting")
        .count();
    assert_eq!(starting, 1);

    supervisor.stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn killed_worker_auto_restarts() {
    let Some(_env) = EnvGuard::set(false) else {
        return;
    };
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "restart-project");
    let supervisor = supervisor(&dir);

    let info = supervisor.ensure_worker(&entry).await.unwrap();
    let pid = info.pid.unwrap();
    kill_pid(pid);
    let restarted = wait_for_ready_with_new_pid(&supervisor, &entry.id, pid).await;
    assert_eq!(restarted.state, WorkerState::Ready);

    supervisor.stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn crash_loop_reaches_crashed() {
    let Some(_env) = EnvGuard::set(true) else {
        return;
    };
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "crash-project");
    let supervisor = supervisor(&dir);

    let _ = supervisor.ensure_worker(&entry).await.unwrap();
    let crashed = wait_for_state_with_timeout(
        &supervisor,
        &entry.id,
        WorkerState::Crashed,
        Duration::from_secs(20),
    )
    .await;
    assert_eq!(crashed.state, WorkerState::Crashed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn crashed_worker_recovers_on_subsequent_ensure() {
    let Some(_env) = EnvGuard::set(true) else {
        return;
    };
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "recover-crash-project");
    let supervisor = supervisor(&dir);

    let _ = supervisor.ensure_worker(&entry).await.unwrap();
    let crashed = wait_for_state_with_timeout(
        &supervisor,
        &entry.id,
        WorkerState::Crashed,
        Duration::from_secs(20),
    )
    .await;
    assert_eq!(crashed.state, WorkerState::Crashed);

    std::env::remove_var("FAKE_WORKER_CRASH");
    let recovered = supervisor.ensure_worker(&entry).await.unwrap();

    assert_eq!(recovered.state, WorkerState::Ready);
    assert!(recovered.pid.is_some());
    supervisor.stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn open_project_returns_bad_gateway_when_crash_loop_exhausts() {
    let Some(_env) = EnvGuard::set(true) else {
        return;
    };
    use axum::body::Body;
    use hyper::{Request, StatusCode};
    use tower::ServiceExt;

    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let state = DaemonState::new(
        DaemonConfig::default(),
        EventBus::new(dir.path().join("events.jsonl")),
        None,
    );
    state.load_projects(dir.path().join("projects.json")).await;
    let router = refact_lsp::daemon::server::make_router(state.clone(), 8488);
    let body = serde_json::to_vec(&serde_json::json!({
        "root": root.to_string_lossy(),
    }))
    .unwrap();

    let response = tokio::time::timeout(
        Duration::from_secs(15),
        router.oneshot(
            Request::builder()
                .method("POST")
                .uri("/daemon/v1/projects/open")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        ),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value["error"]
        .as_str()
        .unwrap_or_default()
        .contains("worker exited"));
    let entries = state.projects.read().await.list();
    assert_eq!(entries.len(), 1);
    let info = state.supervisor.worker_info(&entries[0].id).await.unwrap();
    assert_eq!(info.state, WorkerState::Crashed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn port_busy_exit_retries_ports_until_limit() {
    let Some(_env) = EnvGuard::set_with_port_busy(false, Some("98")) else {
        return;
    };
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "port-busy-project");
    let events = EventBus::new(dir.path().join("events.jsonl"));
    let supervisor = Supervisor::new(events.clone(), dir.path().join("daemon"), 8488);

    let info = supervisor.ensure_worker(&entry).await.unwrap();
    assert!(matches!(
        info.state,
        WorkerState::Failed { ref reason } if reason == "worker port allocation retry limit reached"
    ));
    let events = events.snapshot().await;
    let port_busy_events = events
        .iter()
        .filter(|event| event.kind == "worker_exited" && event.payload["exit_code"] == 98)
        .collect::<Vec<_>>();
    assert_eq!(port_busy_events.len(), 3);
    assert_eq!(port_busy_events[0].payload["retrying_ports"], true);
    assert_eq!(port_busy_events[1].payload["retrying_ports"], true);
    assert_eq!(port_busy_events[2].payload["retrying_ports"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn stop_worker_gracefully_exits() {
    let Some(_env) = EnvGuard::set(false) else {
        return;
    };
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "stop-project");
    let supervisor = supervisor(&dir);

    let info = supervisor.ensure_worker(&entry).await.unwrap();
    assert_eq!(info.state, WorkerState::Ready);
    let pid = info.pid.unwrap();
    let stopped = supervisor.stop_worker(&entry.id).await.unwrap().unwrap();
    assert_eq!(stopped.state, WorkerState::Stopped);
    assert_eq!(supervisor.worker_count().await, 0);
    wait_for_pid_exit(pid).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn forced_stop_reaps_starting_worker() {
    let Some(_env) = EnvGuard::set(false) else {
        return;
    };
    std::env::set_var("FAKE_WORKER_DELAY_READY", "30");
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "forced-stop-project");
    let supervisor = supervisor(&dir);
    let start = tokio::spawn({
        let supervisor = supervisor.clone();
        let entry = entry.clone();
        async move { supervisor.ensure_worker(&entry).await }
    });

    let starting = wait_for_state_with_timeout(
        &supervisor,
        &entry.id,
        WorkerState::Starting,
        Duration::from_secs(5),
    )
    .await;
    let pid = starting.pid.unwrap();
    let stopped = supervisor.stop_worker(&entry.id).await.unwrap().unwrap();
    assert_eq!(stopped.state, WorkerState::Stopped);
    let start_info = tokio::time::timeout(Duration::from_secs(10), start)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        start_info.state,
        WorkerState::Stopping | WorkerState::Stopped
    ));
    assert_eq!(supervisor.worker_count().await, 0);
    wait_for_pid_exit(pid).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn concurrent_proxy_failures_queue_one_restart_for_generation() {
    let Some(_env) = EnvGuard::set(false) else {
        return;
    };
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "proxy-restart-project");
    let events = EventBus::new(dir.path().join("events.jsonl"));
    let supervisor = Supervisor::new(events.clone(), dir.path().join("daemon"), 8488);
    let info = supervisor.ensure_worker(&entry).await.unwrap();
    let pid = info.pid.unwrap();
    std::env::set_var("FAKE_WORKER_DELAY_READY", "1");

    for _ in 0..10 {
        supervisor
            .notify_proxy_unreachable(entry.clone(), false, Some(pid))
            .await;
    }

    let restarted = wait_for_ready_with_new_pid(&supervisor, &entry.id, pid).await;
    assert_eq!(restarted.state, WorkerState::Ready);
    let starts = events
        .snapshot()
        .await
        .into_iter()
        .filter(|event| event.kind == "worker_starting")
        .count();
    assert_eq!(starts, 2);
    supervisor.stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn active_proxy_stream_keeps_killed_idle_worker_wanted_for_restart() {
    let Some(_env) = EnvGuard::set(false) else {
        return;
    };
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "active-stream-project");
    let config = DaemonConfig {
        idle_timeout_secs: 1,
        ..DaemonConfig::default()
    };
    let state = DaemonState::new_with_daemon_dir(
        config,
        EventBus::new(dir.path().join("events.jsonl")),
        None,
        dir.path().join("daemon"),
        0,
    );

    let info = state.supervisor.ensure_worker(&entry).await.unwrap();
    let pid = info.pid.unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    state.increment_live_proxy_stream(&entry.id).await;
    kill_pid(pid);

    let restarted = wait_for_ready_with_new_pid(&state.supervisor, &entry.id, pid).await;

    assert_eq!(restarted.state, WorkerState::Ready);
    state.decrement_live_proxy_stream(&entry.id);
    state.supervisor.stop_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn stop_all_stops_children_for_daemon_shutdown() {
    let Some(_env) = EnvGuard::set(false) else {
        return;
    };
    let dir = tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = project_entry(root, "shutdown-project");
    let supervisor = supervisor(&dir);

    let info = supervisor.ensure_worker(&entry).await.unwrap();
    assert_eq!(info.state, WorkerState::Ready);
    supervisor.stop_all().await;
    let stopped = supervisor.worker_info(&entry.id).await.unwrap();
    assert_eq!(stopped.state, WorkerState::Stopped);
    assert_eq!(supervisor.worker_count().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
#[cfg_attr(
    windows,
    ignore = "Windows artifact runners can starve fake worker shutdown"
)]
async fn stop_all_stops_workers_in_parallel() {
    let Some(_env) = EnvGuard::set(false) else {
        return;
    };
    std::env::set_var("FAKE_WORKER_GRACEFUL_DELAY", "2");
    let dir = tempdir().unwrap();
    let supervisor = supervisor(&dir);
    let entries = ["parallel-a", "parallel-b", "parallel-c"]
        .into_iter()
        .map(|id| {
            let root = dir.path().join(id);
            std::fs::create_dir_all(&root).unwrap();
            project_entry(root, id)
        })
        .collect::<Vec<_>>();

    for entry in &entries {
        let info = supervisor.ensure_worker(entry).await.unwrap();
        assert_eq!(info.state, WorkerState::Ready);
    }

    let start = Instant::now();
    supervisor.stop_all().await;

    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(supervisor.worker_count().await, 0);
    for entry in &entries {
        let stopped = supervisor.worker_info(&entry.id).await.unwrap();
        assert_eq!(stopped.state, WorkerState::Stopped);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn auth_enabled_daemon_accepts_fake_worker_status() {
    let Some(_env) = EnvGuard::set(false) else {
        return;
    };
    let dir = tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = DaemonState::new_with_daemon_dir(
        DaemonConfig::default(),
        EventBus::new(dir.path().join("events.jsonl")),
        Some("secret-token".to_string()),
        dir.path().join("daemon"),
        port,
    );
    let server = axum::Server::from_tcp(listener.into_std().unwrap())
        .unwrap()
        .serve(refact_lsp::daemon::server::make_router(state.clone(), port).into_make_service());
    let server_task = tokio::spawn(server);
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let entry = {
        let mut registry = state.projects.write().await;
        registry.open(root).await.unwrap()
    };

    let info = state.supervisor.ensure_worker(&entry).await.unwrap();
    assert_eq!(info.state, WorkerState::Ready);
    let accepted = {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(report) = state.latest_worker_status(&entry.id).await {
                assert_eq!(report.project_id, entry.id);
                assert_eq!(report.pid, info.pid.unwrap_or_default());
                break report;
            }
            assert!(
                Instant::now() < deadline,
                "worker status was not accepted in time"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://127.0.0.1:{port}/daemon/v1/worker-status");
    let mut wrong_token = accepted.clone();
    wrong_token.instance_token = "wrong-token".to_string();
    let response = client
        .post(&url)
        .bearer_auth("secret-token")
        .json(&wrong_token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

    let mut empty_token = accepted.clone();
    empty_token.instance_token.clear();
    let response = client
        .post(&url)
        .bearer_auth("secret-token")
        .json(&empty_token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

    let response = client
        .post(&url)
        .bearer_auth("secret-token")
        .json(&accepted)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let report = state.latest_worker_status(&entry.id).await.unwrap();
    assert_eq!(report.project_id, entry.id);
    assert_eq!(report.pid, info.pid.unwrap_or_default());
    assert_eq!(report.instance_token, accepted.instance_token);

    state.supervisor.stop_all().await;
    server_task.abort();
}
