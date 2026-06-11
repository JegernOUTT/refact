use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::RwLock as SyncRwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, RwLock};

use crate::daemon::config::DaemonConfig;
use crate::daemon::events::EventBus;
use crate::daemon_link::WorkerStatusReport;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonUrls {
    pub loopback: String,
    pub mdns: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub port: u16,
    pub bind: String,
    pub version: String,
    pub auth_token: Option<String>,
    pub started_at_ms: u64,
    pub hostname_local: String,
    pub urls: DaemonUrls,
}

pub struct DaemonState {
    pub config: DaemonConfig,
    pub auth_token: Option<String>,
    pub started_at_ms: u64,
    pub version: String,
    pub projects: RwLock<crate::daemon::projects::ProjectRegistry>,
    pub worker_statuses: RwLock<HashMap<String, WorkerStatusReport>>,
    pub supervisor: Arc<crate::daemon::supervisor::Supervisor>,
    pub events: EventBus,
    cron_pending: Arc<SyncRwLock<HashMap<String, u64>>>,
    shutdown_tx: broadcast::Sender<String>,
}

impl DaemonState {
    pub fn new(config: DaemonConfig, events: EventBus, auth_token: Option<String>) -> Arc<Self> {
        Self::new_with_daemon_dir(
            config,
            events,
            auth_token,
            crate::daemon::paths::daemon_dir(),
            0,
        )
    }

    pub fn new_with_daemon_dir(
        config: DaemonConfig,
        events: EventBus,
        auth_token: Option<String>,
        daemon_dir: PathBuf,
        daemon_port: u16,
    ) -> Arc<Self> {
        let (shutdown_tx, _) = broadcast::channel(16);
        let cron_pending = Arc::new(SyncRwLock::new(HashMap::new()));
        let supervisor = crate::daemon::supervisor::Supervisor::new_with_cron_pending(
            events.clone(),
            daemon_dir,
            daemon_port,
            cron_pending.clone(),
        );
        Arc::new(Self {
            config,
            auth_token,
            started_at_ms: now_ms(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            projects: RwLock::new(crate::daemon::projects::ProjectRegistry::empty(
                crate::daemon::paths::projects_json_path(),
            )),
            worker_statuses: RwLock::new(HashMap::new()),
            supervisor,
            events,
            cron_pending,
            shutdown_tx,
        })
    }

    pub fn shutdown_receiver(&self) -> broadcast::Receiver<String> {
        self.shutdown_tx.subscribe()
    }

    pub fn request_shutdown(&self, reason: String) {
        let _ = self.shutdown_tx.send(reason);
    }

    pub(crate) async fn set_cron_pending(&self, project_id: &str, next_fire_ms: Option<u64>) {
        match next_fire_ms {
            Some(next_fire_ms) => {
                self.cron_pending
                    .write()
                    .insert(project_id.to_string(), next_fire_ms);
            }
            None => {
                self.cron_pending.write().remove(project_id);
            }
        }
    }

    pub async fn cron_pending(&self, project_id: &str) -> Option<u64> {
        self.cron_pending.read().get(project_id).copied()
    }

    pub(crate) async fn cron_pending_snapshot(&self) -> HashMap<String, u64> {
        self.cron_pending.read().clone()
    }

    pub(crate) async fn retain_cron_pending(&self, project_ids: &HashSet<String>) {
        self.cron_pending
            .write()
            .retain(|project_id, _| project_ids.contains(project_id));
    }

    pub async fn latest_worker_status(&self, project_id: &str) -> Option<WorkerStatusReport> {
        self.worker_statuses.read().await.get(project_id).cloned()
    }

    pub async fn store_worker_status(&self, report: WorkerStatusReport) -> bool {
        let project_id = report.project_id.clone();
        let changed = {
            let mut statuses = self.worker_statuses.write().await;
            let changed = statuses
                .get(&project_id)
                .map(|previous| worker_status_event_fields_changed(previous, &report))
                .unwrap_or(true);
            statuses.insert(project_id.clone(), report.clone());
            changed
        };
        if changed {
            let payload = serde_json::to_value(&report).unwrap_or(Value::Null);
            let _ = self
                .events
                .emit("worker_status", Some(project_id), payload)
                .await;
        }
        changed
    }

    pub async fn load_projects(&self, path: PathBuf) {
        let registry = crate::daemon::projects::ProjectRegistry::load(path).await;
        *self.projects.write().await = registry;
    }

    pub fn daemon_info(&self, port: u16, bind: String) -> DaemonInfo {
        let host_local = hostname_local();
        DaemonInfo {
            pid: std::process::id(),
            port,
            bind,
            version: self.version.clone(),
            auth_token: self.auth_token.clone(),
            started_at_ms: self.started_at_ms,
            hostname_local: host_local.clone(),
            urls: DaemonUrls {
                loopback: format!("http://127.0.0.1:{port}/"),
                mdns: format!("http://{}:{port}/", host_local),
            },
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn worker_status_event_fields_changed(
    previous: &WorkerStatusReport,
    next: &WorkerStatusReport,
) -> bool {
    previous.busy_chats != next.busy_chats || previous.lsp_clients != next.lsp_clients
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

    fn status_report(
        lsp_clients: usize,
        busy_chats: usize,
        exec_running: usize,
    ) -> WorkerStatusReport {
        WorkerStatusReport {
            project_id: "project".to_string(),
            pid: 7,
            lsp_clients,
            busy_chats,
            exec_running,
            last_activity_ts: 11,
        }
    }

    #[test]
    fn worker_status_change_filter_only_checks_busy_and_lsp() {
        assert!(!worker_status_event_fields_changed(
            &status_report(1, 2, 3),
            &status_report(1, 2, 99)
        ));
        assert!(worker_status_event_fields_changed(
            &status_report(1, 2, 3),
            &status_report(2, 2, 3)
        ));
        assert!(worker_status_event_fields_changed(
            &status_report(1, 2, 3),
            &status_report(1, 3, 3)
        ));
    }

    #[tokio::test]
    async fn cron_pending_set_snapshot_and_retain() {
        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("e.jsonl")),
            None,
        );

        state.set_cron_pending("project-a", Some(100)).await;
        state.set_cron_pending("project-b", Some(200)).await;
        assert_eq!(state.cron_pending("project-a").await, Some(100));
        assert_eq!(state.supervisor.cron_pending("project-a"), Some(100));
        assert_eq!(state.cron_pending_snapshot().await.len(), 2);

        state
            .retain_cron_pending(&HashSet::from(["project-a".to_string()]))
            .await;
        assert_eq!(state.cron_pending("project-a").await, Some(100));
        assert_eq!(state.cron_pending("project-b").await, None);

        state.set_cron_pending("project-a", None).await;
        assert_eq!(state.cron_pending("project-a").await, None);
    }

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
            urls: DaemonUrls {
                loopback: "http://127.0.0.1:8488/".to_string(),
                mdns: "http://host.local:8488/".to_string(),
            },
        };
        write_daemon_info_atomic(&path, &info).await.unwrap();
        assert_eq!(read_daemon_info(&path).await.unwrap(), Some(info));
    }

    #[tokio::test]
    async fn daemon_info_urls_contain_port_and_host() {
        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("e.jsonl")),
            None,
        );
        let info = state.daemon_info(9000, "0.0.0.0".to_string());
        assert!(info.urls.loopback.contains("127.0.0.1:9000"));
        assert!(info.urls.mdns.contains(".local:9000"));
        assert!(info.urls.loopback.starts_with("http://"));
        assert!(info.urls.mdns.starts_with("http://"));
    }
}
