use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub mod auth;
pub mod chat_client;
pub mod cli;
pub mod client;
pub mod config;
pub mod cron_clock;
pub mod events;
pub mod hooks;
pub mod idle;
pub mod lock;
pub mod mdns;
pub mod paths;
pub mod ports;
pub mod projects;
pub mod proxy;
pub mod run_cmd;
pub mod server;
pub mod state;
pub mod supervisor;
pub mod web;

#[cfg(not(test))]
const DAEMON_SHUTDOWN_DRAIN: Duration = Duration::from_secs(10);
#[cfg(test)]
const DAEMON_SHUTDOWN_DRAIN: Duration = Duration::from_millis(500);
#[cfg(not(test))]
const DAEMON_PROXY_DRAIN: Duration = Duration::from_secs(2);
#[cfg(test)]
const DAEMON_PROXY_DRAIN: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub(crate) struct RuntimePaths {
    lock_path: PathBuf,
    daemon_json_path: PathBuf,
    events_jsonl_path: PathBuf,
    daemon_log_path: PathBuf,
    projects_json_path: PathBuf,
    daemon_dir_path: PathBuf,
}

impl RuntimePaths {
    fn default() -> Self {
        Self {
            lock_path: paths::lock_path(),
            daemon_json_path: paths::daemon_json_path(),
            events_jsonl_path: paths::events_jsonl_path(),
            daemon_log_path: paths::daemon_log_path(),
            projects_json_path: paths::projects_json_path(),
            daemon_dir_path: paths::daemon_dir(),
        }
    }

    #[cfg(test)]
    pub(crate) fn in_dir(path: &Path) -> Self {
        Self {
            lock_path: path.join("daemon.lock"),
            daemon_json_path: path.join("daemon.json"),
            events_jsonl_path: path.join("events.jsonl"),
            daemon_log_path: path.join("logs").join("daemon.log"),
            projects_json_path: path.join("projects.json"),
            daemon_dir_path: path.to_path_buf(),
        }
    }
}

pub async fn run_daemon(foreground: bool) {
    let config = match config::load().await {
        Ok(config) => config,
        Err(error) => {
            eprintln!("failed to load daemon config: {error}");
            std::process::exit(1);
        }
    };
    let code = run_daemon_entry_with_paths(config, RuntimePaths::default(), foreground, true).await;
    if code != 0 {
        std::process::exit(code);
    }
}

pub(crate) async fn run_daemon_entry_with_paths(
    config: config::DaemonConfig,
    paths: RuntimePaths,
    foreground: bool,
    install_signal_handlers: bool,
) -> i32 {
    let _log_guard = setup_logging(&paths.daemon_log_path);
    let mut lock_file = match lock::open_lock(&paths.lock_path) {
        Ok(lock_file) => lock_file,
        Err(error) => {
            eprintln!(
                "failed to open daemon lock {}: {error}",
                paths.lock_path.display()
            );
            return 1;
        }
    };
    let _lock_guard = match lock::try_lock(&mut lock_file) {
        Ok(guard) => guard,
        Err(error) if lock::is_already_locked(&error) => {
            print_already_running(&paths.daemon_json_path).await;
            return 0;
        }
        Err(error) => {
            eprintln!(
                "failed to lock daemon {}: {error}",
                paths.lock_path.display()
            );
            return 1;
        }
    };

    let listener = match server::bind_listener(&config) {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!("{error}");
            eprintln!("{error}");
            return 1;
        }
    };
    let actual_addr = match listener.local_addr() {
        Ok(addr) => addr,
        Err(error) => {
            eprintln!("failed to read daemon listener address: {error}");
            return 1;
        }
    };

    let auth_token = if config.auth.enabled {
        Some(auth::resolve_token(config.auth.token.as_deref()))
    } else {
        None
    };

    let events = events::EventBus::new(paths.events_jsonl_path.clone());
    let state = state::DaemonState::new_with_daemon_dir(
        config.clone(),
        events,
        auth_token,
        paths.daemon_dir_path.clone(),
        actual_addr.port(),
    );
    state.load_projects(paths.projects_json_path.clone()).await;
    let info = state.daemon_info(actual_addr.port(), actual_addr.ip().to_string());
    if let Err(error) = state::write_daemon_info_atomic(&paths.daemon_json_path, &info).await {
        eprintln!("{error}");
        return 1;
    }
    tracing::info!("daemon listening on {}", actual_addr);
    if foreground {
        eprintln!("refact daemon listening on {}", actual_addr);
    }
    let signal_task = if install_signal_handlers {
        Some(tokio::spawn(forward_signals(state.clone())))
    } else {
        None
    };
    let _ = state
        .events
        .emit(
            "daemon_started",
            None,
            serde_json::json!({"port": actual_addr.port()}),
        )
        .await;

    let mdns_advertisement = mdns::MdnsAdvertisement::start(actual_addr.port());

    let cron_clock_task = cron_clock::spawn(state.clone());
    let idle_task = idle::spawn(state.clone());
    let mut shutdown_rx = state.shutdown_receiver();
    let mut server_task = tokio::spawn(server::serve(listener, state.clone(), actual_addr.port()));
    let mut workers_stopped = false;
    let mut exit_code = 0;
    let mut stopped_payload = serde_json::json!({});
    let mut failed_payload = None;

    tokio::select! {
        shutdown = shutdown_rx.recv() => {
            let reason = shutdown.unwrap_or_else(|_| "shutdown".to_string());
            let drain_timeout_ms = DAEMON_SHUTDOWN_DRAIN.as_millis() as u64;
            let _ = state
                .events
                .emit(
                    "daemon_draining",
                    None,
                    serde_json::json!({"reason": reason.clone(), "drain_timeout_ms": drain_timeout_ms}),
                )
                .await;
            drain_proxy_streams(&state, DAEMON_PROXY_DRAIN).await;
            state.supervisor.stop_all().await;
            workers_stopped = true;
            match tokio::time::timeout(DAEMON_SHUTDOWN_DRAIN, &mut server_task).await {
                Ok(joined) => match joined {
                    Ok(Ok(())) => {
                        stopped_payload = serde_json::json!({"reason": reason.clone()});
                    }
                    Ok(Err(error)) => {
                        tracing::error!("{error}");
                        exit_code = 1;
                        failed_payload = Some(serde_json::json!({"reason": reason.clone(), "error": error}));
                    }
                    Err(error) => {
                        tracing::error!("daemon server task failed: {error}");
                        exit_code = 1;
                        failed_payload = Some(serde_json::json!({"reason": reason.clone(), "error": error.to_string()}));
                    }
                },
                Err(_) => {
                    server_task.abort();
                    let _ = server_task.await;
                    let _ = state
                        .events
                        .emit(
                            "daemon_shutdown_forced",
                            None,
                            serde_json::json!({
                                "reason": reason.clone(),
                                "drain_timeout_ms": drain_timeout_ms,
                            }),
                        )
                        .await;
                    stopped_payload = serde_json::json!({
                        "reason": reason.clone(),
                        "forced": true,
                        "drain_timeout_ms": drain_timeout_ms,
                    });
                }
            }
        }
        joined = &mut server_task => {
            match joined {
                Ok(Ok(())) if state.is_shutting_down() => {
                    stopped_payload = serde_json::json!({"reason": "shutdown"});
                }
                Ok(Ok(())) => {
                    exit_code = 1;
                    failed_payload = Some(serde_json::json!({
                        "error": "daemon server exited unexpectedly",
                    }));
                    state.request_shutdown("server_exit".to_string());
                }
                Ok(Err(error)) => {
                    tracing::error!("{error}");
                    exit_code = 1;
                    failed_payload = Some(serde_json::json!({"error": error}));
                    state.request_shutdown("server_error".to_string());
                }
                Err(error) => {
                    tracing::error!("daemon server task failed: {error}");
                    exit_code = 1;
                    failed_payload = Some(serde_json::json!({"error": error.to_string()}));
                    state.request_shutdown("server_task_failed".to_string());
                }
            }
        }
    }

    if let Some(mdns) = mdns_advertisement {
        mdns.stop();
    }

    if let Some(signal_task) = signal_task {
        signal_task.abort();
    }
    cron_clock_task.abort();
    idle_task.abort();
    if !workers_stopped {
        state.supervisor.stop_all().await;
    }
    if let Some(payload) = failed_payload {
        let _ = state.events.emit("daemon_failed", None, payload).await;
    } else {
        let _ = state
            .events
            .emit("daemon_stopped", None, stopped_payload)
            .await;
    }
    if let Err(error) = state::remove_daemon_info(&paths.daemon_json_path).await {
        tracing::warn!("{error}");
    }
    exit_code
}

fn setup_logging(path: &Path) -> tracing_appender::non_blocking::WorkerGuard {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let appender = tracing_appender::rolling::RollingFileAppender::new(
        tracing_appender::rolling::Rotation::NEVER,
        path.parent().unwrap_or_else(|| Path::new(".")),
        path.file_name().unwrap_or_default(),
    );
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false);
    let _ = tracing_subscriber::registry().with(layer).try_init();
    guard
}

async fn print_already_running(path: &Path) {
    match state::read_daemon_info(path).await {
        Ok(Some(info)) => println!(
            "daemon already running (pid {}, port {})",
            info.pid, info.port
        ),
        _ => println!("daemon already running"),
    }
}

async fn drain_proxy_streams(state: &state::DaemonState, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if state
            .proxy_activity
            .read()
            .values()
            .all(|activity| activity.live_proxy_streams == 0)
        {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn forward_signals(state: Arc<state::DaemonState>) {
    wait_for_signal().await;
    state.request_shutdown("signal".to_string());
    wait_for_signal().await;
    let _ = state
        .events
        .emit(
            "daemon_shutdown_escalated",
            None,
            serde_json::json!({"reason": "second_signal"}),
        )
        .await;
    std::process::exit(130);
}

async fn wait_for_signal() {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = wait_for_sigterm() => {}
    }
}

#[cfg(unix)]
async fn wait_for_sigterm() {
    if let Ok(mut sigterm) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        let _ = sigterm.recv().await;
    }
}

#[cfg(not(unix))]
async fn wait_for_sigterm() {
    std::future::pending::<()>().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::{Body, Request, StatusCode};

    async fn wait_for_info(path: &Path) -> state::DaemonInfo {
        for _ in 0..100 {
            if let Some(info) = state::read_daemon_info(path).await.unwrap() {
                return info;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("daemon info not written");
    }

    #[tokio::test]
    async fn daemon_lifecycle_status_shutdown_removes_daemon_json() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::in_dir(dir.path());
        let config = config::DaemonConfig {
            bind: "127.0.0.1".to_string(),
            port: 0,
            ..config::DaemonConfig::default()
        };
        let task_paths = paths.clone();
        let task = tokio::spawn(async move {
            run_daemon_entry_with_paths(config, task_paths, false, false).await
        });
        let info = wait_for_info(&paths.daemon_json_path).await;
        let client = reqwest::Client::new();
        let status: serde_json::Value = client
            .get(format!("http://127.0.0.1:{}/daemon/v1/status", info.port))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(status["workers"], 0);
        let response = client
            .post(format!("http://127.0.0.1:{}/daemon/v1/shutdown", info.port))
            .json(&serde_json::json!({"reason": "test"}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        assert_eq!(task.await.unwrap(), 0);
        assert!(!paths.daemon_json_path.exists());
    }

    #[tokio::test]
    async fn daemon_lifecycle_daemon_json_has_urls() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::in_dir(dir.path());
        let config = config::DaemonConfig {
            bind: "127.0.0.1".to_string(),
            port: 0,
            ..config::DaemonConfig::default()
        };
        let task_paths = paths.clone();
        let task = tokio::spawn(async move {
            run_daemon_entry_with_paths(config, task_paths, false, false).await
        });
        let info = wait_for_info(&paths.daemon_json_path).await;
        assert!(info.urls.loopback.starts_with("http://127.0.0.1:"));
        assert!(info.urls.mdns.contains(".local:"));
        let client = reqwest::Client::new();
        let _ = client
            .post(format!("http://127.0.0.1:{}/daemon/v1/shutdown", info.port))
            .json(&serde_json::json!({"reason": "test"}))
            .send()
            .await
            .unwrap();
        let _ = task.await;
    }

    #[tokio::test]
    async fn daemon_router_shutdown_endpoint_accepts_reason() {
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let state = state::DaemonState::new(
            config::DaemonConfig::default(),
            events::EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        let mut shutdown_rx = state.shutdown_receiver();
        let response = server::make_router(state, 8488)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/daemon/v1/shutdown")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"reason":"test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(shutdown_rx.recv().await.unwrap(), "test");
    }

    #[tokio::test(start_paused = true)]
    async fn drain_proxy_streams_returns_after_timeout_with_live_stream() {
        let dir = tempfile::tempdir().unwrap();
        let state = state::DaemonState::new(
            config::DaemonConfig::default(),
            events::EventBus::new(dir.path().join("events.jsonl")),
            None,
        );
        state.increment_live_proxy_stream("project").await;
        let start = tokio::time::Instant::now();

        drain_proxy_streams(&state, Duration::from_secs(1)).await;

        assert!(tokio::time::Instant::now().duration_since(start) >= Duration::from_secs(1));
        assert_eq!(state.proxy_activity("project").await.live_proxy_streams, 1);
    }
}
