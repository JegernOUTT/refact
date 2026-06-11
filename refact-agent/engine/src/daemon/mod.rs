use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub mod auth;
pub mod client;
pub mod config;
pub mod cron_clock;
pub mod events;
pub mod lock;
pub mod mdns;
pub mod paths;
pub mod ports;
pub mod projects;
pub mod server;
pub mod state;
pub mod supervisor;

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
    fn in_dir(path: &Path) -> Self {
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
    let serve_result = server::serve(listener, state.clone(), actual_addr.port()).await;

    if let Some(mdns) = mdns_advertisement {
        mdns.stop();
    }

    if let Some(signal_task) = signal_task {
        signal_task.abort();
    }
    cron_clock_task.abort();
    if let Err(error) = serve_result {
        tracing::error!("{error}");
    }
    state.supervisor.stop_all().await;
    let _ = state
        .events
        .emit("daemon_stopped", None, serde_json::json!({}))
        .await;
    if let Err(error) = state::remove_daemon_info(&paths.daemon_json_path).await {
        tracing::warn!("{error}");
    }
    0
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

async fn forward_signals(state: Arc<state::DaemonState>) {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            state.request_shutdown("signal".to_string());
        }
        _ = wait_for_sigterm() => {
            state.request_shutdown("signal".to_string());
        }
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
}
