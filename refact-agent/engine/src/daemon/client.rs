use std::cmp::Ordering;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::daemon::state::DaemonInfo;

const CONTROL_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const CONTROL_TOTAL_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonInfoReadError {
    Corrupt { path: PathBuf, message: String },
    Io { path: PathBuf, message: String },
}

impl std::fmt::Display for DaemonInfoReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonInfoReadError::Corrupt { path, message } => {
                write!(f, "daemon info corrupt at {}: {message}", path.display())
            }
            DaemonInfoReadError::Io { path, message } => {
                write!(
                    f,
                    "failed to read daemon info at {}: {message}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for DaemonInfoReadError {}

pub async fn read_daemon_json() -> Result<Option<DaemonInfo>, DaemonInfoReadError> {
    read_daemon_json_path(&crate::daemon::paths::daemon_json_path()).await
}

async fn read_daemon_json_path(path: &Path) -> Result<Option<DaemonInfo>, DaemonInfoReadError> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => {
            serde_json::from_str(&content)
                .map(Some)
                .map_err(|error| DaemonInfoReadError::Corrupt {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                })
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DaemonInfoReadError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        }),
    }
}

#[derive(Debug)]
pub enum DaemonClientError {
    Http(String),
    Status { status: u16, body: String },
    Json(String),
}

impl std::fmt::Display for DaemonClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonClientError::Http(message) => write!(f, "{message}"),
            DaemonClientError::Status { status, body } => {
                write!(f, "daemon request failed with status {status}: {body}")
            }
            DaemonClientError::Json(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for DaemonClientError {}

pub async fn ping_daemon(info: &DaemonInfo) -> bool {
    get_json::<Value>(info, "/daemon/v1/status").await.is_ok()
}

pub async fn ensure_daemon_running() -> Result<DaemonInfo, String> {
    if let Some(info) = shutdown_for_upgrade_if_older(env!("CARGO_PKG_VERSION")).await? {
        return Ok(info);
    }
    ensure_daemon_running_with_starter(spawn_detached_daemon).await
}

pub async fn shutdown_for_upgrade_if_older(my_version: &str) -> Result<Option<DaemonInfo>, String> {
    let Some(info) = read_daemon_json()
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    if !version_is_older(&info.version, my_version) {
        return Ok(None);
    }
    if ping_daemon(&info).await {
        post_shutdown(&info, "upgrade").await?;
        wait_until_dead(&info, STARTUP_TIMEOUT).await?;
    }
    let upgraded = ensure_daemon_running_with_starter(spawn_detached_daemon).await?;
    Ok(Some(upgraded))
}

pub async fn get_json<T: DeserializeOwned>(
    info: &DaemonInfo,
    path: &str,
) -> Result<T, DaemonClientError> {
    let response = daemon_request(info, reqwest::Method::GET, path)
        .send()
        .await
        .map_err(|error| {
            DaemonClientError::Http(crate::daemon::auth::redact_daemon_token(&format!(
                "failed to contact daemon: {error}"
            )))
        })?;
    decode_json_response(response).await
}

pub async fn get_text(info: &DaemonInfo, path: &str) -> Result<String, DaemonClientError> {
    let response = daemon_request(info, reqwest::Method::GET, path)
        .send()
        .await
        .map_err(|error| {
            DaemonClientError::Http(crate::daemon::auth::redact_daemon_token(&format!(
                "failed to contact daemon: {error}"
            )))
        })?;
    decode_text_response(response).await
}

pub async fn post_json<B: Serialize + ?Sized, T: DeserializeOwned>(
    info: &DaemonInfo,
    path: &str,
    body: &B,
) -> Result<T, DaemonClientError> {
    let response = daemon_request(info, reqwest::Method::POST, path)
        .json(body)
        .send()
        .await
        .map_err(|error| {
            DaemonClientError::Http(crate::daemon::auth::redact_daemon_token(&format!(
                "failed to contact daemon: {error}"
            )))
        })?;
    decode_json_response(response).await
}

pub async fn post_empty_json<T: DeserializeOwned>(
    info: &DaemonInfo,
    path: &str,
) -> Result<T, DaemonClientError> {
    post_json(info, path, &Value::Object(Default::default())).await
}

pub async fn delete_json<T: DeserializeOwned>(
    info: &DaemonInfo,
    path: &str,
) -> Result<T, DaemonClientError> {
    let response = daemon_request(info, reqwest::Method::DELETE, path)
        .send()
        .await
        .map_err(|error| {
            DaemonClientError::Http(crate::daemon::auth::redact_daemon_token(&format!(
                "failed to contact daemon: {error}"
            )))
        })?;
    decode_json_response(response).await
}

pub async fn shutdown_daemon(info: &DaemonInfo, reason: &str) -> Result<(), String> {
    post_shutdown(info, reason).await
}

pub async fn wait_for_daemon_stop(info: &DaemonInfo, timeout: Duration) -> Result<(), String> {
    wait_until_dead(info, timeout).await
}

fn daemon_request(
    info: &DaemonInfo,
    method: reqwest::Method,
    path: &str,
) -> reqwest::RequestBuilder {
    let url = format!(
        "{}{}",
        crate::daemon::chat_client::daemon_base_url(info),
        path
    );
    let request = control_client().request(method, url);
    match &info.auth_token {
        Some(token) => request.bearer_auth(token),
        None => request,
    }
}

fn control_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| build_control_client().expect("failed to build daemon control client"))
}

fn build_control_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(CONTROL_CONNECT_TIMEOUT)
        .timeout(CONTROL_TOTAL_TIMEOUT)
        .build()
}

async fn decode_json_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, DaemonClientError> {
    if !response.status().is_success() {
        return Err(response_status_error(response).await);
    }
    response
        .json::<T>()
        .await
        .map_err(|error| DaemonClientError::Json(format!("invalid daemon JSON: {error}")))
}

async fn decode_text_response(response: reqwest::Response) -> Result<String, DaemonClientError> {
    if !response.status().is_success() {
        return Err(response_status_error(response).await);
    }
    response.text().await.map_err(|error| {
        DaemonClientError::Http(crate::daemon::auth::redact_daemon_token(&format!(
            "failed to read daemon response: {error}"
        )))
    })
}

async fn response_status_error(response: reqwest::Response) -> DaemonClientError {
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| error.to_string());
    DaemonClientError::Status {
        status,
        body: crate::daemon::auth::redact_daemon_token(&body),
    }
}

async fn ensure_daemon_running_with_starter<F>(starter: F) -> Result<DaemonInfo, String>
where
    F: FnOnce() -> Result<(), String>,
{
    match read_daemon_json().await {
        Ok(Some(info)) => {
            if ping_daemon(&info).await {
                return Ok(info);
            }
        }
        Ok(None) => {}
        Err(error) => return Err(error.to_string()),
    }
    starter()?;
    poll_daemon(STARTUP_TIMEOUT).await
}

fn spawn_detached_daemon() -> Result<(), String> {
    let exe =
        std::env::current_exe().map_err(|error| format!("failed to find current exe: {error}"))?;
    let mut command = Command::new(exe);
    command
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_detached(&mut command);
    command
        .spawn()
        .map_err(|error| format!("failed to spawn daemon: {error}"))?;
    Ok(())
}

#[cfg(unix)]
fn configure_detached(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
fn configure_detached(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn configure_detached(_: &mut Command) {}

async fn poll_daemon(timeout: Duration) -> Result<DaemonInfo, String> {
    let started = Instant::now();
    loop {
        match read_daemon_json().await {
            Ok(Some(info)) => {
                if ping_daemon(&info).await {
                    return Ok(info);
                }
            }
            Ok(None) => {}
            Err(error) => return Err(error.to_string()),
        }
        if started.elapsed() >= timeout {
            return Err("daemon did not become ready before timeout".to_string());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_until_dead(info: &DaemonInfo, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        if !ping_daemon(info).await {
            return Ok(());
        }
        if started.elapsed() >= timeout {
            return Err("daemon did not shut down before timeout".to_string());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn post_shutdown(info: &DaemonInfo, reason: &str) -> Result<(), String> {
    post_json::<_, Value>(
        info,
        "/daemon/v1/shutdown",
        &serde_json::json!({"reason": reason}),
    )
    .await
    .map(|_| ())
    .map_err(|error| match error {
        DaemonClientError::Http(message) => format!("failed to request daemon shutdown: {message}"),
        DaemonClientError::Status { status, .. } => {
            format!("daemon shutdown failed with status {status}")
        }
        DaemonClientError::Json(message) => message,
    })
}

fn version_is_older(current: &str, mine: &str) -> bool {
    compare_versions(current, mine) == Ordering::Less
}

pub(crate) fn compare_versions(left: &str, right: &str) -> Ordering {
    match (parse_semver(left), parse_semver(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemverParts {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<String>,
}

impl Ord for SemverParts {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| compare_prerelease(self.pre.as_deref(), other.pre.as_deref()))
    }
}

impl PartialOrd for SemverParts {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_semver(version: &str) -> Option<SemverParts> {
    let without_prefix = version.trim().strip_prefix('v').unwrap_or(version.trim());
    let without_build = without_prefix
        .split_once('+')
        .map(|(base, _)| base)
        .unwrap_or(without_prefix);
    let (core, pre) = without_build
        .split_once('-')
        .map(|(core, pre)| (core, Some(pre)))
        .unwrap_or((without_build, None));
    let parts = core.split('.').collect::<Vec<_>>();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let parse_part = |part: &str| {
        if part.chars().all(|ch| ch.is_ascii_digit()) {
            part.parse::<u64>().ok()
        } else {
            None
        }
    };
    let pre = match pre {
        Some("") => return None,
        Some(pre) => Some(pre.to_string()),
        None => None,
    };
    Some(SemverParts {
        major: parse_part(parts[0])?,
        minor: parse_part(parts[1])?,
        patch: parse_part(parts[2])?,
        pre,
    })
}

fn compare_prerelease(left: Option<&str>, right: Option<&str>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => compare_prerelease_identifiers(left, right),
    }
}

fn compare_prerelease_identifiers(left: &str, right: &str) -> Ordering {
    let mut left_parts = left.split('.');
    let mut right_parts = right.split('.');
    loop {
        match (left_parts.next(), right_parts.next()) {
            (Some(left), Some(right)) => {
                let ordering = compare_prerelease_identifier(left, right);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        }
    }
}

fn compare_prerelease_identifier(left: &str, right: &str) -> Ordering {
    let left_numeric = parse_numeric_identifier(left);
    let right_numeric = parse_numeric_identifier(right);
    match (left_numeric, right_numeric) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.cmp(right),
    }
}

fn parse_numeric_identifier(value: &str) -> Option<u64> {
    if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use serial_test::serial;

    struct EnvGuard {
        cache: Option<String>,
        config: Option<String>,
    }

    impl EnvGuard {
        fn set(cache: &std::path::Path, config: &std::path::Path) -> Self {
            let guard = Self {
                cache: std::env::var("REFACT_DAEMON_CACHE_DIR").ok(),
                config: std::env::var("REFACT_DAEMON_CONFIG_DIR").ok(),
            };
            std::env::set_var("REFACT_DAEMON_CACHE_DIR", cache);
            std::env::set_var("REFACT_DAEMON_CONFIG_DIR", config);
            guard
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(cache) = &self.cache {
                std::env::set_var("REFACT_DAEMON_CACHE_DIR", cache);
            } else {
                std::env::remove_var("REFACT_DAEMON_CACHE_DIR");
            }
            if let Some(config) = &self.config {
                std::env::set_var("REFACT_DAEMON_CONFIG_DIR", config);
            } else {
                std::env::remove_var("REFACT_DAEMON_CONFIG_DIR");
            }
        }
    }

    #[test]
    fn daemon_version_compare_handles_triples() {
        assert!(version_is_older("0.9.9", "0.10.0"));
        assert!(version_is_older("1.2.3", "1.2.4"));
        assert!(version_is_older("1.2.3", "1.3.0"));
        assert!(!version_is_older("2.0.0", "1.9.9"));
        assert!(!version_is_older("1.2.3", "1.2.3"));
    }

    #[test]
    fn daemon_version_compare_handles_prereleases() {
        assert!(version_is_older("1.2.3-alpha", "1.2.3"));
        assert!(version_is_older("1.2.3-alpha.1", "1.2.3-alpha.2"));
        assert!(version_is_older("1.2.3-alpha.9", "1.2.3-alpha.10"));
        assert!(!version_is_older("1.2.3", "1.2.3-alpha"));
        assert!(!version_is_older("1.2.3-alpha.2", "1.2.3-alpha.2"));
    }

    #[test]
    fn daemon_version_compare_falls_back_to_string_for_invalid_versions() {
        assert_eq!(
            compare_versions("dev-b", "dev-a"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("dev-a", "dev-a"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn control_client_builds_with_bounded_timeouts() {
        assert_eq!(CONTROL_CONNECT_TIMEOUT, Duration::from_secs(2));
        assert_eq!(CONTROL_TOTAL_TIMEOUT, Duration::from_secs(10));
        let _ = build_control_client().unwrap();
        assert!(std::ptr::eq(control_client(), control_client()));
    }

    #[tokio::test]
    async fn read_daemon_json_path_reports_corrupt_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.json");
        tokio::fs::write(&path, b"not json").await.unwrap();
        let error = read_daemon_json_path(&path).await.unwrap_err();
        assert!(matches!(error, DaemonInfoReadError::Corrupt { .. }));
        assert!(error.to_string().contains("daemon info corrupt at"));
        assert!(error.to_string().contains(&path.display().to_string()));
    }

    #[tokio::test]
    async fn read_daemon_json_path_distinguishes_absent_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.json");
        assert!(read_daemon_json_path(&path).await.unwrap().is_none());
    }

    fn daemon_info(port: u16, auth_token: Option<&str>) -> DaemonInfo {
        DaemonInfo {
            pid: 1,
            port,
            bind: "127.0.0.1".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            auth_token: auth_token.map(str::to_string),
            started_at_ms: 0,
            hostname_local: "test.local".to_string(),
            urls: crate::daemon::state::DaemonUrls {
                loopback: format!("http://127.0.0.1:{port}"),
                mdns: String::new(),
            },
        }
    }

    #[tokio::test]
    async fn ping_daemon_sends_bearer_token() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = axum::Router::new().route(
            "/daemon/v1/status",
            axum::routing::get(|headers: axum::http::HeaderMap| async move {
                if headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer secret")
                {
                    (
                        axum::http::StatusCode::OK,
                        axum::Json(serde_json::json!({"ok": true})),
                    )
                } else {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        axum::Json(serde_json::json!({"error": "Unauthorized"})),
                    )
                }
            }),
        );
        let server = axum::Server::from_tcp(listener.into_std().unwrap())
            .unwrap()
            .serve(app.into_make_service());
        let task = tokio::spawn(server);

        assert!(ping_daemon(&daemon_info(port, Some("secret"))).await);
        assert!(!ping_daemon(&daemon_info(port, Some("wrong"))).await);
        task.abort();
    }

    #[tokio::test]
    #[serial]
    async fn ensure_daemon_running_finds_foreground_task_variant() {
        let cache_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set(cache_dir.path(), config_dir.path());
        let runtime_paths =
            crate::daemon::RuntimePaths::in_dir(&crate::daemon::paths::daemon_dir());
        let handle_slot = Arc::new(Mutex::new(None));
        let starter_paths = runtime_paths.clone();
        let handle_slot_for_start = handle_slot.clone();
        let info = ensure_daemon_running_with_starter(move || {
            let config = crate::daemon::config::DaemonConfig {
                bind: "127.0.0.1".to_string(),
                port: 0,
                ..crate::daemon::config::DaemonConfig::default()
            };
            let handle = tokio::spawn(async move {
                crate::daemon::run_daemon_entry_with_paths(config, starter_paths, false, false)
                    .await
            });
            *handle_slot_for_start.lock().unwrap() = Some(handle);
            Ok(())
        })
        .await
        .unwrap();
        assert!(ping_daemon(&info).await);
        post_shutdown(&info, "test").await.unwrap();
        let handle = handle_slot.lock().unwrap().take().unwrap();
        assert_eq!(handle.await.unwrap(), 0);
    }
}
