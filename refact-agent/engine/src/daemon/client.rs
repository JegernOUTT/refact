use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::daemon::state::DaemonInfo;

const PING_TIMEOUT: Duration = Duration::from_secs(2);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

pub async fn read_daemon_json() -> Option<DaemonInfo> {
    crate::daemon::state::read_daemon_info(&crate::daemon::paths::daemon_json_path())
        .await
        .ok()
        .flatten()
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
    let url = format!(
        "http://{}:{}/daemon/v1/status",
        connect_host(&info.bind),
        info.port
    );
    let client = match reqwest::Client::builder().timeout(PING_TIMEOUT).build() {
        Ok(client) => client,
        Err(_) => return false,
    };
    match client.get(url).send().await {
        Ok(response) if response.status().is_success() => true,
        _ => false,
    }
}

pub async fn ensure_daemon_running() -> Result<DaemonInfo, String> {
    if let Some(info) = shutdown_for_upgrade_if_older(env!("CARGO_PKG_VERSION")).await? {
        return Ok(info);
    }
    ensure_daemon_running_with_starter(spawn_detached_daemon).await
}

pub async fn shutdown_for_upgrade_if_older(my_version: &str) -> Result<Option<DaemonInfo>, String> {
    let Some(info) = read_daemon_json().await else {
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
        .map_err(|error| DaemonClientError::Http(format!("failed to contact daemon: {error}")))?;
    decode_json_response(response).await
}

pub async fn get_text(info: &DaemonInfo, path: &str) -> Result<String, DaemonClientError> {
    let response = daemon_request(info, reqwest::Method::GET, path)
        .send()
        .await
        .map_err(|error| DaemonClientError::Http(format!("failed to contact daemon: {error}")))?;
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
        .map_err(|error| DaemonClientError::Http(format!("failed to contact daemon: {error}")))?;
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
        .map_err(|error| DaemonClientError::Http(format!("failed to contact daemon: {error}")))?;
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
    let client = reqwest::Client::new();
    let request = client.request(method, url);
    match &info.auth_token {
        Some(token) => request.bearer_auth(token),
        None => request,
    }
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
        DaemonClientError::Http(format!("failed to read daemon response: {error}"))
    })
}

async fn response_status_error(response: reqwest::Response) -> DaemonClientError {
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| error.to_string());
    DaemonClientError::Status { status, body }
}

async fn ensure_daemon_running_with_starter<F>(starter: F) -> Result<DaemonInfo, String>
where
    F: FnOnce() -> Result<(), String>,
{
    if let Some(info) = read_daemon_json().await {
        if ping_daemon(&info).await {
            return Ok(info);
        }
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
        if let Some(info) = read_daemon_json().await {
            if ping_daemon(&info).await {
                return Ok(info);
            }
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
    let url = format!(
        "http://{}:{}/daemon/v1/shutdown",
        connect_host(&info.bind),
        info.port
    );
    let client = reqwest::Client::builder()
        .timeout(PING_TIMEOUT)
        .build()
        .map_err(|error| format!("failed to create daemon client: {error}"))?;
    let request = client
        .post(url)
        .json(&serde_json::json!({"reason": reason}));
    let request = match &info.auth_token {
        Some(token) => request.bearer_auth(token),
        None => request,
    };
    let response = request
        .send()
        .await
        .map_err(|error| format!("failed to request daemon shutdown: {error}"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "daemon shutdown failed with status {}",
            response.status()
        ))
    }
}

fn connect_host(bind: &str) -> String {
    match bind {
        "0.0.0.0" | "::" => "127.0.0.1".to_string(),
        other => other.to_string(),
    }
}

fn version_is_older(current: &str, mine: &str) -> bool {
    parse_version(current) < parse_version(mine)
}

fn parse_version(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split(|ch: char| !ch.is_ascii_digit() && ch != '.');
    let first = parts.next().unwrap_or(version);
    let mut nums = first
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0));
    (
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
    )
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
        assert!(version_is_older("1.2.3", "1.2.4"));
        assert!(version_is_older("1.2.3", "1.3.0"));
        assert!(!version_is_older("2.0.0", "1.9.9"));
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
