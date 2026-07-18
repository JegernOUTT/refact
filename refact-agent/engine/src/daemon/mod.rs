use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

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
const DEFAULT_LOG_FILTER_LEVEL: &str = "info";
const NOISY_LOG_TARGETS: &[&str] = &[
    "hyper",
    "hyper_util",
    "h2",
    "tower",
    "tower_http",
    "rustls",
    "mio",
    "tokio_util",
    "reqwest",
];
const LOG_FILE_MAX_BYTES: u64 = 64 * 1024 * 1024;
const LOG_DIR_MAX_BYTES: u64 = 300 * 1024 * 1024;
const LOG_RETENTION_SECS: u64 = 14 * 24 * 60 * 60;
const ROTATED_LOG_FILES_TO_KEEP: usize = 3;

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
    let relaunch_wait = std::env::var("REFACT_DAEMON_RELAUNCH")
        .map(|value| value == "1")
        .unwrap_or(false);
    let lock_deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let _lock_guard = loop {
        match lock::try_lock(&mut lock_file) {
            Ok(guard) => break guard,
            Err(error) if lock::is_already_locked(&error) => {
                if relaunch_wait && std::time::Instant::now() < lock_deadline {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
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

    let mdns_advertisement =
        mdns::MdnsAdvertisement::start(&config, actual_addr.ip(), actual_addr.port());

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
    if let Some(update_task) = state.update_task.lock().await.take() {
        update_task.abort();
        let _ = update_task.await;
    }
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
    let (writer, guard) = non_blocking_bounded_log_writer_with_prune(path, true);
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_filter(log_env_filter());
    let _ = tracing_subscriber::registry().with(layer).try_init();
    guard
}

pub(crate) fn default_log_filter_directives() -> String {
    log_filter_directives(DEFAULT_LOG_FILTER_LEVEL)
}

fn log_filter_directives(default_level: &str) -> String {
    std::iter::once(default_level.to_string())
        .chain(
            NOISY_LOG_TARGETS
                .iter()
                .map(|target| format!("{target}=warn")),
        )
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn rust_log_is_set() -> bool {
    std::env::var_os("RUST_LOG").is_some_and(|value| !value.is_empty())
}

pub(crate) fn log_env_filter() -> EnvFilter {
    log_env_filter_with_default_level(DEFAULT_LOG_FILTER_LEVEL)
}

pub(crate) fn log_env_filter_with_default_level(default_level: &str) -> EnvFilter {
    let directives = std::env::var("RUST_LOG")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| log_filter_directives(default_level));
    EnvFilter::new(directives)
}

pub(crate) fn non_blocking_bounded_log_writer(
    path: &Path,
) -> (
    tracing_appender::non_blocking::NonBlocking,
    tracing_appender::non_blocking::WorkerGuard,
) {
    non_blocking_bounded_log_writer_with_prune(path, false)
}

fn non_blocking_bounded_log_writer_with_prune(
    path: &Path,
    prune_logs: bool,
) -> (
    tracing_appender::non_blocking::NonBlocking,
    tracing_appender::non_blocking::WorkerGuard,
) {
    match BoundedLogWriter::new(path, prune_logs) {
        Ok(writer) => tracing_appender::non_blocking(writer),
        Err(error) => {
            eprintln!("failed to open log file {}: {error}", path.display());
            tracing_appender::non_blocking(io::stderr())
        }
    }
}

struct BoundedLogWriter {
    path: PathBuf,
    file: Option<File>,
    bytes_written: u64,
}

impl BoundedLogWriter {
    fn new(path: &Path, prune_logs: bool) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            rotate_log_file_if_oversized(path, LOG_FILE_MAX_BYTES)?;
            if prune_logs {
                prune_log_dir(parent, path)?;
            }
        }
        let file = open_log_file(path)?;
        let bytes_written = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(file),
            bytes_written,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        if let Some(mut file) = self.file.take() {
            file.flush()?;
        }
        rotate_log_file_sequence(&self.path, ROTATED_LOG_FILES_TO_KEEP)?;
        self.file = Some(open_log_file(&self.path)?);
        self.bytes_written = 0;
        Ok(())
    }
}

impl Write for BoundedLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.bytes_written > 0
            && self.bytes_written.saturating_add(buf.len() as u64) > LOG_FILE_MAX_BYTES
        {
            self.rotate()?;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "log file is closed"))?;
        let written = file.write(buf)?;
        self.bytes_written = self.bytes_written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            file.flush()
        } else {
            Ok(())
        }
    }
}

fn open_log_file(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

fn rotate_log_file_if_oversized(path: &Path, max_bytes: u64) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_file() && metadata.len() > max_bytes {
        rotate_log_file_sequence(path, ROTATED_LOG_FILES_TO_KEEP)?;
    }
    Ok(())
}

fn rotate_log_file_sequence(path: &Path, backups: usize) -> io::Result<()> {
    if backups == 0 {
        return remove_log_file(path);
    }
    remove_log_file(&numbered_log_path(path, backups))?;
    for index in (1..backups).rev() {
        rename_log_file_if_exists(
            &numbered_log_path(path, index),
            &numbered_log_path(path, index + 1),
        )?;
    }
    rename_log_file_if_exists(path, &numbered_log_path(path, 1))?;
    Ok(())
}

fn numbered_log_path(path: &Path, index: usize) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    path.with_file_name(format!("{file_name}.{index}"))
}

fn rename_log_file_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn prune_log_dir(logs_dir: &Path, active_path: &Path) -> io::Result<()> {
    prune_log_dir_with_limits(
        logs_dir,
        active_path,
        Duration::from_secs(LOG_RETENTION_SECS),
        LOG_DIR_MAX_BYTES,
        ROTATED_LOG_FILES_TO_KEEP,
    )
}

fn prune_log_dir_with_limits(
    logs_dir: &Path,
    active_path: &Path,
    max_age: Duration,
    max_total_bytes: u64,
    rotated_daemon_files_to_keep: usize,
) -> io::Result<()> {
    let mut candidates = collect_log_candidates(logs_dir, active_path)?;
    let now = SystemTime::now();
    let mut delete_paths = std::collections::HashSet::new();

    for candidate in &candidates {
        let too_old = now
            .duration_since(candidate.modified)
            .map(|age| age > max_age)
            .unwrap_or(false);
        if too_old || is_unittest_worker_log_name(&candidate.file_name) {
            delete_paths.insert(candidate.path.clone());
        }
    }

    let mut daemon_rotations = candidates
        .iter()
        .filter(|candidate| is_daemon_rotation_log_name(&candidate.file_name))
        .collect::<Vec<_>>();
    daemon_rotations.sort_by(|a, b| b.modified.cmp(&a.modified));
    for candidate in daemon_rotations
        .into_iter()
        .skip(rotated_daemon_files_to_keep)
    {
        delete_paths.insert(candidate.path.clone());
    }

    for path in delete_paths.iter() {
        remove_log_file(path)?;
    }
    candidates.retain(|candidate| !delete_paths.contains(&candidate.path));

    let active_len = fs::metadata(active_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let mut total_bytes = active_len.saturating_add(candidates.iter().map(|c| c.len).sum::<u64>());
    candidates.sort_by(|a, b| a.modified.cmp(&b.modified));
    for candidate in candidates {
        if total_bytes <= max_total_bytes {
            break;
        }
        remove_log_file(&candidate.path)?;
        total_bytes = total_bytes.saturating_sub(candidate.len);
    }
    Ok(())
}

#[derive(Clone)]
struct LogCandidate {
    path: PathBuf,
    file_name: String,
    len: u64,
    modified: SystemTime,
}

fn collect_log_candidates(logs_dir: &Path, active_path: &Path) -> io::Result<Vec<LogCandidate>> {
    let mut candidates = Vec::new();
    let active_file_name = active_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string());
    let entries = match fs::read_dir(logs_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidates),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path == active_path {
            continue;
        }
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name == "daemon.log" || active_file_name.as_deref() == Some(file_name.as_str()) {
            continue;
        }
        if !is_daemon_or_worker_log_name(&file_name) {
            continue;
        }
        let metadata = entry.metadata()?;
        candidates.push(LogCandidate {
            path,
            file_name,
            len: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    Ok(candidates)
}

fn is_daemon_or_worker_log_name(name: &str) -> bool {
    name == "daemon.log"
        || is_daemon_rotation_log_name(name)
        || (name.starts_with("worker-") && (name.ends_with(".log") || name.contains(".log.")))
}

fn is_daemon_rotation_log_name(name: &str) -> bool {
    name.starts_with("daemon.log.")
}

fn is_unittest_worker_log_name(name: &str) -> bool {
    name.starts_with("worker-unittest-") && (name.ends_with(".log") || name.contains(".log."))
}

fn remove_log_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
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
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedWriter {
        type Writer = SharedWriter;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    struct RustLogGuard(Option<String>);

    impl RustLogGuard {
        fn unset() -> Self {
            let previous = std::env::var("RUST_LOG").ok();
            std::env::remove_var("RUST_LOG");
            Self(previous)
        }

        fn set(value: &str) -> Self {
            let previous = std::env::var("RUST_LOG").ok();
            std::env::set_var("RUST_LOG", value);
            Self(previous)
        }
    }

    impl Drop for RustLogGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("RUST_LOG", value),
                None => std::env::remove_var("RUST_LOG"),
            }
        }
    }

    fn capture_filtered_logs(filter: EnvFilter) -> String {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let layer = tracing_subscriber::fmt::layer()
            .without_time()
            .with_ansi(false)
            .with_writer(SharedWriter(logs.clone()))
            .with_filter(filter);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target: "hyper::proto::h2", "hyper trace noise");
            tracing::debug!(target: "h2::hpack::decoder", "h2 debug noise");
            tracing::info!(target: "refact_lsp::daemon", "daemon info kept");
        });
        let bytes = logs.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    fn write_log(path: &Path, size: usize, age_secs: u64) {
        fs::write(path, vec![b'x'; size]).unwrap();
        let mtime =
            filetime::FileTime::from_system_time(SystemTime::now() - Duration::from_secs(age_secs));
        filetime::set_file_mtime(path, mtime).unwrap();
    }

    async fn wait_for_info(path: &Path) -> state::DaemonInfo {
        for _ in 0..100 {
            if let Some(info) = state::read_daemon_info(path).await.unwrap() {
                return info;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("daemon info not written");
    }

    #[test]
    #[serial_test::serial]
    fn default_log_filter_keeps_app_info_and_drops_http_trace_noise() {
        let _guard = RustLogGuard::unset();

        let directives = default_log_filter_directives();
        let output = capture_filtered_logs(log_env_filter());

        assert!(directives.contains("info"));
        for target in NOISY_LOG_TARGETS {
            assert!(directives.contains(&format!("{target}=warn")));
        }
        assert!(output.contains("daemon info kept"));
        assert!(!output.contains("hyper trace noise"));
        assert!(!output.contains("h2 debug noise"));
    }

    #[test]
    #[serial_test::serial]
    fn rust_log_override_allows_noisy_debugging_when_requested() {
        let _guard = RustLogGuard::set("hyper=trace");

        let output = capture_filtered_logs(log_env_filter());

        assert!(output.contains("hyper trace noise"));
    }

    #[test]
    fn oversized_log_file_rotates_before_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.log");
        fs::write(&path, b"large").unwrap();
        fs::write(numbered_log_path(&path, 1), b"previous").unwrap();

        rotate_log_file_if_oversized(&path, 3).unwrap();

        assert!(!path.exists());
        assert_eq!(fs::read(numbered_log_path(&path, 1)).unwrap(), b"large");
        assert_eq!(fs::read(numbered_log_path(&path, 2)).unwrap(), b"previous");
    }

    #[test]
    fn log_prune_deletes_old_unittest_and_budget_excess_without_active() {
        let dir = tempfile::tempdir().unwrap();
        let active = dir.path().join("daemon.log");
        write_log(&active, 100, 1);
        write_log(&dir.path().join("daemon.log.1"), 1, 3);
        write_log(&dir.path().join("daemon.log.2"), 1, 4);
        write_log(&dir.path().join("daemon.log.3"), 1, 5);
        write_log(&dir.path().join("worker-unittest-card.log"), 10, 1);
        write_log(&dir.path().join("worker-stale.log"), 10, 120);
        write_log(&dir.path().join("worker-budget-old.log"), 40, 20);
        write_log(&dir.path().join("worker-budget-new.log"), 40, 10);
        write_log(&dir.path().join("unrelated.txt"), 10, 120);

        prune_log_dir_with_limits(dir.path(), &active, Duration::from_secs(60), 155, 2).unwrap();

        assert!(active.exists());
        assert!(dir.path().join("daemon.log.1").exists());
        assert!(dir.path().join("daemon.log.2").exists());
        assert!(!dir.path().join("daemon.log.3").exists());
        assert!(!dir.path().join("worker-unittest-card.log").exists());
        assert!(!dir.path().join("worker-stale.log").exists());
        assert!(!dir.path().join("worker-budget-old.log").exists());
        assert!(dir.path().join("worker-budget-new.log").exists());
        assert!(dir.path().join("unrelated.txt").exists());
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
