use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, RwLock};

use crate::daemon::config::DaemonConfig;
use crate::daemon::events::EventBus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub port: u16,
    pub bind: String,
    pub version: String,
    pub auth_token: Option<String>,
    pub started_at_ms: u64,
    pub hostname_local: String,
}

pub struct DaemonState {
    pub config: DaemonConfig,
    pub started_at_ms: u64,
    pub version: String,
    pub projects: RwLock<HashMap<String, Value>>,
    pub events: EventBus,
    shutdown_tx: broadcast::Sender<String>,
}

impl DaemonState {
    pub fn new(config: DaemonConfig, events: EventBus) -> Arc<Self> {
        let (shutdown_tx, _) = broadcast::channel(16);
        Arc::new(Self {
            config,
            started_at_ms: now_ms(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            projects: RwLock::new(HashMap::new()),
            events,
            shutdown_tx,
        })
    }

    pub fn shutdown_receiver(&self) -> broadcast::Receiver<String> {
        self.shutdown_tx.subscribe()
    }

    pub fn request_shutdown(&self, reason: String) {
        let _ = self.shutdown_tx.send(reason);
    }

    pub fn daemon_info(&self, port: u16, bind: String) -> DaemonInfo {
        DaemonInfo {
            pid: std::process::id(),
            port,
            bind,
            version: self.version.clone(),
            auth_token: None,
            started_at_ms: self.started_at_ms,
            hostname_local: hostname_local(),
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn hostname_local() -> String {
    let hostname = hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .map(|value| sanitize_hostname_label(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "refact".to_string());
    format!("{hostname}.local")
}

fn sanitize_hostname_label(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub async fn read_daemon_info(path: &Path) -> Result<Option<DaemonInfo>, String> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => serde_json::from_str(&content)
            .map(Some)
            .map_err(|error| format!("failed to parse {}: {error}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

pub async fn write_daemon_info_atomic(path: &Path, info: &DaemonInfo) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let content = serde_json::to_vec_pretty(info)
        .map_err(|error| format!("failed to encode daemon info: {error}"))?;
    tokio::fs::write(&tmp, content)
        .await
        .map_err(|error| format!("failed to write {}: {error}", tmp.display()))?;
    #[cfg(windows)]
    match tokio::fs::remove_file(path).await {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(format!("failed to replace {}: {error}", path.display())),
    }
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|error| format!("failed to publish {}: {error}", path.display()))?;
    Ok(())
}

pub async fn remove_daemon_info(path: &Path) -> Result<(), String> {
    match tokio::fs::remove_file(path).await {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove {}: {error}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn daemon_json_atomic_write_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.json");
        let info = DaemonInfo {
            pid: 42,
            port: 8488,
            bind: "127.0.0.1".to_string(),
            version: "1.2.3".to_string(),
            auth_token: None,
            started_at_ms: 100,
            hostname_local: "host.local".to_string(),
        };
        write_daemon_info_atomic(&path, &info).await.unwrap();
        assert_eq!(read_daemon_info(&path).await.unwrap(), Some(info));
    }
}
