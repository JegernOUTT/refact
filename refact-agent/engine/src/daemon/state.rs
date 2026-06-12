use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::RwLock as SyncRwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::{broadcast, RwLock};

use crate::daemon::config::DaemonConfig;
use crate::daemon::events::EventBus;
use crate::daemon::idle::WorkerIdleSnapshot;
use crate::daemon::supervisor::{WorkerInfo, WorkerState};
use crate::daemon_link::WorkerStatusReport;

#[derive(Debug, Clone)]
struct StoredWorkerStatus {
    report: WorkerStatusReport,
    received_ms: u64,
}

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyActivity {
    pub last_proxy_activity_ms: u64,
    pub live_proxy_streams: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRow {
    pub project_id: String,
    pub slug: String,
    pub root: PathBuf,
    pub pinned: bool,
    pub last_active_ms: u64,
    pub state: WorkerState,
    pub pid: Option<u32>,
    pub http_port: Option<u16>,
    pub lsp_port: Option<u16>,
    pub lsp_clients: usize,
    pub busy_chats: usize,
    pub exec_running: usize,
    pub live_proxy_streams: u64,
    pub cron_next_fire_ms: Option<u64>,
    pub idle_deadline_ms: Option<u64>,
    pub last_status_report_ms: Option<u64>,
    pub last_error: Option<String>,
}

pub struct DaemonState {
    pub config: DaemonConfig,
    pub auth_token: Option<String>,
    pub started_at_ms: u64,
    pub version: String,
    pub projects: RwLock<crate::daemon::projects::ProjectRegistry>,
    worker_statuses: RwLock<HashMap<String, StoredWorkerStatus>>,
    pub proxy_activity: SyncRwLock<HashMap<String, ProxyActivity>>,
    pub supervisor: Arc<crate::daemon::supervisor::Supervisor>,
    pub proxy_client: reqwest::Client,
    pub proxy_stream_client: reqwest::Client,
    pub events: EventBus,
    pub daemon_dir: PathBuf,
    cron_pending: Arc<SyncRwLock<HashMap<String, u64>>>,
    shutdown_tx: broadcast::Sender<String>,
    shutdown_requested: AtomicBool,
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
            daemon_dir.clone(),
            daemon_port,
            cron_pending.clone(),
            config.idle_timeout_secs,
            auth_token.clone(),
        );
        let proxy_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(300))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("failed to build daemon proxy http client");
        let proxy_stream_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .expect("failed to build daemon proxy stream http client");
        Arc::new(Self {
            config,
            auth_token,
            started_at_ms: now_ms(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            projects: RwLock::new(crate::daemon::projects::ProjectRegistry::empty(
                daemon_dir.join("projects.json"),
            )),
            worker_statuses: RwLock::new(HashMap::new()),
            proxy_activity: SyncRwLock::new(HashMap::new()),
            supervisor,
            proxy_client,
            proxy_stream_client,
            events,
            daemon_dir,
            cron_pending,
            shutdown_tx,
            shutdown_requested: AtomicBool::new(false),
        })
    }

    pub fn shutdown_receiver(&self) -> broadcast::Receiver<String> {
        self.shutdown_tx.subscribe()
    }

    pub fn request_shutdown(&self, reason: String) {
        self.shutdown_requested.store(true, Ordering::Relaxed);
        let _ = self.shutdown_tx.send(reason);
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutdown_requested.load(Ordering::Relaxed)
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
        self.worker_statuses
            .read()
            .await
            .get(project_id)
            .map(|status| status.report.clone())
    }

    pub async fn store_worker_status(&self, report: WorkerStatusReport) -> bool {
        let project_id = report.project_id.clone();
        if report.last_activity_ts > 0 {
            self.supervisor
                .note_project_activity(&project_id, report.last_activity_ts)
                .await;
        }
        let changed = {
            let mut statuses = self.worker_statuses.write().await;
            let changed = statuses
                .get(&project_id)
                .map(|previous| worker_status_event_fields_changed(&previous.report, &report))
                .unwrap_or(true);
            statuses.insert(
                project_id.clone(),
                StoredWorkerStatus {
                    report: report.clone(),
                    received_ms: now_ms(),
                },
            );
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
        for entry in registry.list() {
            self.sync_project_liveness(&entry).await;
        }
        *self.projects.write().await = registry;
    }

    pub(crate) async fn sync_project_liveness(
        &self,
        entry: &crate::daemon::projects::ProjectEntry,
    ) {
        self.supervisor
            .set_project_liveness(&entry.id, entry.pinned, entry.last_active_ms)
            .await;
        let mut activity = self.proxy_activity.write();
        let activity = activity.entry(entry.id.clone()).or_default();
        activity.last_proxy_activity_ms = activity.last_proxy_activity_ms.max(entry.last_active_ms);
    }

    pub async fn update_proxy_activity(&self, project_id: &str) {
        let now = now_ms();
        self.supervisor.note_project_activity(project_id, now).await;
        let mut activity = self.proxy_activity.write();
        activity
            .entry(project_id.to_string())
            .or_default()
            .last_proxy_activity_ms = now;
    }

    pub async fn increment_live_proxy_stream(&self, project_id: &str) {
        let mut activity = self.proxy_activity.write();
        let activity = activity.entry(project_id.to_string()).or_default();
        activity.last_proxy_activity_ms = now_ms();
        activity.live_proxy_streams = activity.live_proxy_streams.saturating_add(1);
    }

    pub fn decrement_live_proxy_stream(&self, project_id: &str) {
        let mut activity = self.proxy_activity.write();
        let activity = activity.entry(project_id.to_string()).or_default();
        activity.live_proxy_streams = activity.live_proxy_streams.saturating_sub(1);
    }

    pub(crate) async fn worker_idle_snapshots(&self) -> Vec<(String, WorkerIdleSnapshot)> {
        let projects = {
            let registry = self.projects.read().await;
            registry.list()
        };
        let statuses = self.worker_statuses.read().await.clone();
        let activity = self.proxy_activity.read().clone();
        let cron_pending = self.cron_pending_snapshot().await;
        let mut snapshots = Vec::new();
        for entry in projects {
            let Some(info) = self.supervisor.worker_info(&entry.id).await else {
                continue;
            };
            let status = statuses.get(&entry.id);
            let activity = activity.get(&entry.id).cloned().unwrap_or_default();
            snapshots.push((
                entry.id.clone(),
                WorkerIdleSnapshot {
                    state: info.state,
                    pinned: entry.pinned,
                    live_proxy_streams: activity.live_proxy_streams.min(u64::from(u32::MAX)) as u32,
                    last_proxy_activity_ms: activity
                        .last_proxy_activity_ms
                        .max(entry.last_active_ms),
                    lsp_clients: status
                        .map(|status| status.report.lsp_clients.min(u32::MAX as usize) as u32)
                        .unwrap_or(0),
                    busy_chats: status
                        .map(|status| status.report.busy_chats.min(u32::MAX as usize) as u32)
                        .unwrap_or(0),
                    exec_running: status
                        .map(|status| status.report.exec_running.min(u32::MAX as usize) as u32)
                        .unwrap_or(0),
                    last_status_report_ms: status.map(|status| status.received_ms).unwrap_or(0),
                    cron_next_fire_ms: cron_pending.get(&entry.id).copied(),
                },
            ));
        }
        snapshots
    }

    pub async fn proxy_activity(&self, project_id: &str) -> ProxyActivity {
        self.proxy_activity
            .read()
            .get(project_id)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn worker_rows(&self) -> Vec<WorkerRow> {
        let projects = {
            let registry = self.projects.read().await;
            registry.list()
        };
        let statuses = self.worker_statuses.read().await.clone();
        let activity = self.proxy_activity.read().clone();
        let cron_pending = self.cron_pending_snapshot().await;
        let now = now_ms();
        let mut rows = Vec::new();
        for entry in projects {
            let worker = self.supervisor.worker_info(&entry.id).await;
            let status = statuses.get(&entry.id);
            let activity = activity.get(&entry.id).cloned().unwrap_or_default();
            let last_activity_ms = entry
                .last_active_ms
                .max(activity.last_proxy_activity_ms)
                .max(
                    status
                        .map(|status| status.report.last_activity_ts)
                        .unwrap_or(0),
                );
            let idle_deadline_ms = idle_deadline_ms(
                now,
                &entry,
                worker.as_ref(),
                status,
                &activity,
                cron_pending.get(&entry.id).copied(),
                self.config.idle_timeout_secs,
            );
            rows.push(WorkerRow {
                project_id: entry.id.clone(),
                slug: entry.slug.clone(),
                root: entry.root.clone(),
                pinned: entry.pinned,
                last_active_ms: last_activity_ms,
                state: worker
                    .as_ref()
                    .map(|worker| worker.state.clone())
                    .unwrap_or(WorkerState::Stopped),
                pid: worker.as_ref().and_then(|worker| worker.pid),
                http_port: worker.as_ref().map(|worker| worker.http_port),
                lsp_port: worker.as_ref().map(|worker| worker.lsp_port),
                lsp_clients: status.map(|status| status.report.lsp_clients).unwrap_or(0),
                busy_chats: status.map(|status| status.report.busy_chats).unwrap_or(0),
                exec_running: status.map(|status| status.report.exec_running).unwrap_or(0),
                live_proxy_streams: activity.live_proxy_streams,
                cron_next_fire_ms: cron_pending.get(&entry.id).copied(),
                idle_deadline_ms,
                last_status_report_ms: status.map(|status| status.received_ms),
                last_error: worker.as_ref().and_then(|worker| worker.last_error.clone()),
            });
        }
        rows.sort_by(|a, b| {
            a.slug
                .cmp(&b.slug)
                .then_with(|| a.project_id.cmp(&b.project_id))
        });
        rows
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

fn idle_deadline_ms(
    now: u64,
    entry: &crate::daemon::projects::ProjectEntry,
    worker: Option<&WorkerInfo>,
    status: Option<&StoredWorkerStatus>,
    activity: &ProxyActivity,
    cron_next_fire_ms: Option<u64>,
    idle_timeout_secs: u64,
) -> Option<u64> {
    if idle_timeout_secs == 0 || entry.pinned {
        return None;
    }
    if worker.map(|worker| &worker.state) != Some(&WorkerState::Ready) {
        return None;
    }
    if activity.live_proxy_streams > 0 {
        return None;
    }
    if status
        .map(|status| {
            status.report.lsp_clients > 0
                || status.report.busy_chats > 0
                || status.report.exec_running > 0
        })
        .unwrap_or(false)
    {
        return None;
    }
    if cron_next_fire_ms
        .map(|next_fire_ms| crate::daemon::idle::cron_pending_blocks_idle_stop(next_fire_ms, now))
        .unwrap_or(false)
    {
        return None;
    }
    let last_activity_ms = entry
        .last_active_ms
        .max(activity.last_proxy_activity_ms)
        .max(
            status
                .map(|status| status.report.last_activity_ts)
                .unwrap_or(0),
        );
    Some(last_activity_ms.saturating_add(idle_timeout_secs.saturating_mul(1000)))
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
    previous.busy_chats != next.busy_chats
        || previous.lsp_clients != next.lsp_clients
        || previous.exec_running != next.exec_running
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
    write_daemon_state_json_atomic(path, info, "daemon info").await
}

pub(crate) async fn write_daemon_state_json_atomic<T: Serialize>(
    path: &Path,
    value: &T,
    label: &str,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    match tokio::fs::remove_file(&tmp).await {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(format!("failed to replace {}: {error}", tmp.display())),
    }
    let content = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to encode {label}: {error}"))?;
    write_daemon_state_tmp(&tmp, &content, label).await?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|error| format!("failed to publish {}: {error}", path.display()))?;
    Ok(())
}

async fn write_daemon_state_tmp(path: &Path, content: &[u8], label: &str) -> Result<(), String> {
    #[cfg(unix)]
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .await
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    #[cfg(not(unix))]
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    #[cfg(unix)]
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .await
        .map_err(|error| format!("failed to chmod {}: {error}", path.display()))?;
    file.write_all(content)
        .await
        .map_err(|error| format!("failed to write {label}: {error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("failed to sync {}: {error}", path.display()))?;
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn test_daemon_info() -> DaemonInfo {
        DaemonInfo {
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
        }
    }

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
    fn worker_status_event_fields_changed_checks_event_fields() {
        let previous = status_report(1, 2, 0);
        let mut activity_only = status_report(1, 2, 0);
        activity_only.last_activity_ts = 99;

        assert!(!worker_status_event_fields_changed(
            &previous,
            &activity_only
        ));
        assert!(worker_status_event_fields_changed(
            &status_report(1, 2, 0),
            &status_report(2, 2, 0)
        ));
        assert!(worker_status_event_fields_changed(
            &status_report(1, 2, 0),
            &status_report(1, 3, 0)
        ));
        assert!(worker_status_event_fields_changed(
            &status_report(1, 2, 0),
            &status_report(1, 2, 1)
        ));
        assert!(worker_status_event_fields_changed(
            &status_report(1, 2, 1),
            &status_report(1, 2, 0)
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
    async fn new_with_daemon_dir_uses_daemon_dir_projects_json() {
        let dir = tempfile::tempdir().unwrap();
        let daemon_dir = dir.path().join("custom-daemon");
        let state = DaemonState::new_with_daemon_dir(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("e.jsonl")),
            None,
            daemon_dir.clone(),
            0,
        );

        let projects_path = state.projects.read().await.path().to_path_buf();
        assert_eq!(state.daemon_dir, daemon_dir);
        assert_eq!(projects_path, daemon_dir.join("projects.json"));
    }

    #[tokio::test]
    async fn daemon_json_atomic_write_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.json");
        let info = test_daemon_info();
        write_daemon_info_atomic(&path, &info).await.unwrap();
        assert_eq!(read_daemon_info(&path).await.unwrap(), Some(info));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn daemon_json_write_sets_permissions_on_create_and_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.json");
        let mut info = test_daemon_info();

        write_daemon_info_atomic(&path, &info).await.unwrap();
        assert_eq!(file_mode(&path), 0o600);

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        info.port = 9999;
        write_daemon_info_atomic(&path, &info).await.unwrap();

        assert_eq!(file_mode(&path), 0o600);
        assert_eq!(read_daemon_info(&path).await.unwrap(), Some(info));
    }

    #[cfg(unix)]
    fn file_mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
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
