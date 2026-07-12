use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

pub const E2E_ENV: &str = "REFACT_DAEMON_E2E";

pub fn e2e_enabled() -> bool {
    std::env::var(E2E_ENV).ok().as_deref() == Some("1")
}

pub fn print_skip() {
    println!("skipped: set REFACT_DAEMON_E2E=1");
}

pub struct E2eDirs {
    _tmp: TempDir,
    pub daemon_dir: PathBuf,
    pub home_dir: PathBuf,
}

impl E2eDirs {
    pub fn new() -> Self {
        let tmp = tempdir().unwrap();
        let daemon_dir = tmp.path().join("daemon");
        let home_dir = tmp.path().join("home");
        std::fs::create_dir_all(&daemon_dir).unwrap();
        std::fs::create_dir_all(&home_dir).unwrap();
        Self {
            _tmp: tmp,
            daemon_dir,
            home_dir,
        }
    }

    pub fn write_daemon_config(&self, idle_timeout_secs: u64) {
        std::fs::write(
            self.daemon_dir.join("daemon.yaml"),
            format!("port: 0\nbind: 127.0.0.1\nidle_timeout_secs: {idle_timeout_secs}\n"),
        )
        .unwrap();
    }
}

pub struct DaemonProcess {
    child: Child,
    daemon_dir: PathBuf,
}

impl DaemonProcess {
    pub async fn start(dirs: &E2eDirs) -> Self {
        Self::start_with_extra_env(dirs, &[]).await
    }

    pub async fn start_with_extra_env(dirs: &E2eDirs, extra_env: &[(&str, &str)]) -> Self {
        let mut command = Command::new(refact_bin());
        command
            .arg("daemon")
            .arg("--foreground")
            .env("REFACT_DAEMON_DIR", &dirs.daemon_dir)
            .env("HOME", &dirs.home_dir)
            .env("NO_COLOR", "1")
            // Poll idle workers sub-second so idle-stop is detected quickly and
            // deterministically instead of on the coarse 60s production cadence.
            // Tests using long idle_timeout_secs never idle-stop regardless.
            .env("REFACT_DAEMON_IDLE_TICK_MS", "500")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let child = command.spawn().unwrap();
        let process = Self {
            child,
            daemon_dir: dirs.daemon_dir.clone(),
        };
        process.wait_for_status().await;
        process
    }

    pub async fn status(&self) -> Value {
        let info = self.daemon_info().await;
        get_json(&format!(
            "{}/daemon/v1/status",
            self.base_url_from_info(&info)
        ))
        .await
    }

    pub async fn post_json(&self, path: &str, body: Value) -> reqwest::Response {
        let info = self.daemon_info().await;
        reqwest::Client::new()
            .post(format!("{}{}", self.base_url_from_info(&info), path))
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    pub async fn get(&self, path: &str) -> reqwest::Response {
        let info = self.daemon_info().await;
        reqwest::Client::new()
            .get(format!("{}{}", self.base_url_from_info(&info), path))
            .send()
            .await
            .unwrap()
    }

    pub async fn daemon_info(&self) -> Value {
        let path = self.daemon_dir.join("daemon.json");
        wait_for(Duration::from_secs(30), || {
            let path = path.clone();
            async move {
                let content = tokio::fs::read_to_string(&path).await.ok()?;
                serde_json::from_str::<Value>(&content).ok()
            }
        })
        .await
    }

    pub async fn shutdown(mut self) {
        let _ = self
            .post_json("/daemon/v1/shutdown", json!({"reason": "test"}))
            .await;
        let _ = tokio::time::timeout(Duration::from_secs(10), self.child.wait()).await;
    }

    async fn wait_for_status(&self) {
        wait_for(Duration::from_secs(30), || async {
            let info = self.daemon_info().await;
            let url = format!("{}/daemon/v1/status", self.base_url_from_info(&info));
            match reqwest::get(url).await {
                Ok(response) if response.status().is_success() => Some(()),
                _ => None,
            }
        })
        .await;
    }

    fn base_url_from_info(&self, info: &Value) -> String {
        let port = info["port"].as_u64().unwrap();
        format!("http://127.0.0.1:{port}")
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

pub fn refact_bin() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_refact") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join(if cfg!(windows) {
            "refact.exe"
        } else {
            "refact"
        })
}

pub async fn wait_for<T, F, Fut>(timeout: Duration, mut f: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = f().await {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for condition");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

pub async fn get_json(url: &str) -> Value {
    let response = reqwest::get(url).await.unwrap();
    assert!(
        response.status().is_success(),
        "GET {url} returned {}",
        response.status()
    );
    response.json::<Value>().await.unwrap()
}

pub fn make_project(name: &str) -> TempDir {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), format!("# {name}\n")).unwrap();
    let _ = std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(dir.path())
        .status();
    dir
}

pub async fn lsp_initialize(port: u64, id: u64) -> Value {
    let mut stream = TcpStream::connect(("127.0.0.1", port as u16))
        .await
        .unwrap();
    write_lsp_message(
        &mut stream,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootUri": null,
                "capabilities": {},
                "workspaceFolders": null
            }
        }),
    )
    .await;
    read_lsp_response(&mut stream, id).await
}

async fn write_lsp_message(stream: &mut TcpStream, value: Value) {
    let body = value.to_string();
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    stream.write_all(frame.as_bytes()).await.unwrap();
}

async fn read_lsp_response(stream: &mut TcpStream, id: u64) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = read_lsp_message(stream).await;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                return message;
            }
        }
    })
    .await
    .unwrap()
}

async fn read_lsp_message(stream: &mut TcpStream) -> Value {
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        stream.read_exact(&mut byte).await.unwrap();
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let header = String::from_utf8(header).unwrap();
    let content_length = header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap();
    let mut body = vec![0_u8; content_length];
    stream.read_exact(&mut body).await.unwrap();
    serde_json::from_slice::<Value>(&body).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_dir_env_name_matches_runtime_contract() {
        assert_eq!(
            refact_lsp::daemon::paths::DAEMON_DIR_ENV,
            "REFACT_DAEMON_DIR"
        );
    }

    #[test]
    fn daemon_dir_env_redirects_runtime_paths() {
        let temp = tempdir().unwrap();
        let previous = std::env::var_os(refact_lsp::daemon::paths::DAEMON_DIR_ENV);
        std::env::set_var(refact_lsp::daemon::paths::DAEMON_DIR_ENV, temp.path());

        assert_eq!(refact_lsp::daemon::paths::daemon_dir(), temp.path());
        assert_eq!(
            refact_lsp::daemon::paths::daemon_config_path(),
            temp.path().join("daemon.yaml")
        );
        assert_eq!(
            refact_lsp::daemon::paths::daemon_json_path(),
            temp.path().join("daemon.json")
        );

        match previous {
            Some(value) => std::env::set_var(refact_lsp::daemon::paths::DAEMON_DIR_ENV, value),
            None => std::env::remove_var(refact_lsp::daemon::paths::DAEMON_DIR_ENV),
        }
    }

    #[test]
    fn refact_bin_falls_back_to_debug_binary() {
        let path = refact_bin();
        assert!(path.ends_with(if cfg!(windows) {
            "refact.exe"
        } else {
            "refact"
        }));
    }
}
