use std::collections::{HashMap, HashSet};
use std::io::{ErrorKind, Read};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::RwLock as SyncRwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::daemon::config::DaemonConfig;
use crate::daemon::events::EventBus;
use crate::daemon::idle::{WorkerIdleSnapshot, STATUS_FRESH_MS};
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
    pub executable_sha256: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerRow {
    pub project_id: String,
    pub slug: String,
    pub root: PathBuf,
    #[serde(default = "worker_row_root_exists_default")]
    pub root_exists: bool,
    pub pinned: bool,
    pub last_active_ms: u64,
    pub state: WorkerState,
    pub pid: Option<u32>,
    pub rss_bytes: Option<u64>,
    pub cpu_percent: Option<f32>,
    pub uptime_secs: Option<u64>,
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
    pub log_path: String,
}

fn worker_row_root_exists_default() -> bool {
    true
}

pub struct DaemonState {
    pub config: DaemonConfig,
    pub auth_token: Option<String>,
    pub started_at_ms: u64,
    pub version: String,
    relaunch_executable: Result<PathBuf, String>,
    pub projects: RwLock<crate::daemon::projects::ProjectRegistry>,
    worker_statuses: RwLock<HashMap<String, StoredWorkerStatus>>,
    buddy_observations: Mutex<Option<crate::daemon::buddy_reconciliation::ReconciliationStore>>,
    buddy_wakes: Mutex<Option<crate::daemon::buddy_wake::WakeStore>>,
    buddy_wake_epochs: SyncRwLock<HashMap<String, (String, u64)>>,
    pub proxy_activity: Arc<SyncRwLock<HashMap<String, ProxyActivity>>>,
    pub supervisor: Arc<crate::daemon::supervisor::Supervisor>,
    pub proxy_client: reqwest::Client,
    pub proxy_stream_client: reqwest::Client,
    pub(crate) console: crate::daemon::console::Console,
    pub events: EventBus,
    pub daemon_dir: PathBuf,
    cron_pending: Arc<SyncRwLock<HashMap<String, u64>>>,
    pub update_state: Mutex<DaemonUpdateState>,
    pub update_check_cache:
        Mutex<Option<(std::time::Instant, refact_self_update::UpdateCheckInfo)>>,
    pub update_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    shutdown_tx: broadcast::Sender<String>,
    shutdown_requested: AtomicBool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Idle,
    Checking,
    Downloading,
    Restarting,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonUpdateState {
    pub phase: UpdatePhase,
    pub detail: Option<String>,
    pub target_version: Option<String>,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
}

impl Default for DaemonUpdateState {
    fn default() -> Self {
        Self {
            phase: UpdatePhase::Idle,
            detail: None,
            target_version: None,
            started_at_ms: None,
            finished_at_ms: None,
        }
    }
}

pub(crate) fn resolve_current_executable() -> Result<PathBuf, String> {
    let path = std::env::current_exe()
        .map_err(|error| format!("cannot resolve current executable: {error}"))?;
    Ok(stable_executable_path(path))
}

fn stable_executable_path(path: PathBuf) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        if let Some(path_bytes) = path.as_os_str().as_bytes().strip_suffix(b" (deleted)") {
            let replacement = PathBuf::from(std::ffi::OsString::from_vec(path_bytes.to_vec()));
            if replacement.is_file() {
                return replacement.canonicalize().unwrap_or(replacement);
            }
        }
    }
    path
}

pub fn executable_sha256_for_path(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("failed to open executable {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to read executable {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

static CURRENT_EXECUTABLE_SHA256: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

pub fn current_executable_sha256() -> Option<String> {
    CURRENT_EXECUTABLE_SHA256
        .get_or_init(|| {
            let path = std::env::current_exe().ok()?;
            executable_sha256_for_path(&path).ok()
        })
        .clone()
}

pub fn current_executable_sha256_if_computed() -> Option<String> {
    CURRENT_EXECUTABLE_SHA256.get().cloned().flatten()
}

pub fn warm_current_executable_sha256() {
    if CURRENT_EXECUTABLE_SHA256.get().is_some() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("refact-exe-sha256".to_string())
        .spawn(|| {
            let _ = current_executable_sha256();
        });
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
        Self::new_with_daemon_dir_and_executable(
            config,
            events,
            auth_token,
            daemon_dir,
            daemon_port,
            resolve_current_executable(),
        )
    }

    fn new_with_daemon_dir_and_executable(
        config: DaemonConfig,
        events: EventBus,
        auth_token: Option<String>,
        daemon_dir: PathBuf,
        daemon_port: u16,
        relaunch_executable: Result<PathBuf, String>,
    ) -> Arc<Self> {
        let (shutdown_tx, _) = broadcast::channel(16);
        let cron_pending = Arc::new(SyncRwLock::new(HashMap::new()));
        let proxy_activity = Arc::new(SyncRwLock::new(HashMap::new()));
        let worker_auth_token = auth_token.clone().or_else(|| {
            config
                .hooks
                .enabled
                .then(|| config.hooks.token.clone())
                .flatten()
        });
        let supervisor = crate::daemon::supervisor::Supervisor::new_with_cron_pending(
            events.clone(),
            daemon_dir.clone(),
            daemon_port,
            cron_pending.clone(),
            proxy_activity.clone(),
            config.idle_timeout_secs,
            worker_auth_token,
            relaunch_executable.clone(),
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
        warm_current_executable_sha256();
        Arc::new(Self {
            config,
            auth_token,
            started_at_ms: now_ms(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            relaunch_executable,
            projects: RwLock::new(crate::daemon::projects::ProjectRegistry::empty(
                daemon_dir.join("projects.json"),
            )),
            worker_statuses: RwLock::new(HashMap::new()),
            buddy_observations: Mutex::new(None),
            buddy_wakes: Mutex::new(None),
            buddy_wake_epochs: SyncRwLock::new(HashMap::new()),
            proxy_activity,
            supervisor,
            proxy_client,
            proxy_stream_client,
            console: crate::daemon::console::Console::new(),
            events,
            daemon_dir,

            cron_pending,
            update_state: Mutex::new(DaemonUpdateState::default()),
            update_check_cache: Mutex::new(None),
            update_task: Mutex::new(None),
            shutdown_tx,
            shutdown_requested: AtomicBool::new(false),
        })
    }

    /// Scratch-only constructor; production never accepts an installation target from transport.
    #[cfg(test)]
    pub(crate) fn with_test_installation_target(
        config: DaemonConfig,
        daemon_dir: PathBuf,
        target: PathBuf,
    ) -> Arc<Self> {
        Self::new_with_daemon_dir_and_executable(
            config,
            EventBus::new(daemon_dir.join("events.jsonl")),
            Some("worker-bearer".into()),
            daemon_dir,
            0,
            Ok(target),
        )
    }

    pub(crate) fn relaunch_executable(&self) -> Result<&Path, String> {
        self.relaunch_executable
            .as_deref()
            .map_err(|error| error.clone())
    }

    pub fn shutdown_receiver(&self) -> broadcast::Receiver<String> {
        self.shutdown_tx.subscribe()
    }

    pub fn request_shutdown(&self, reason: String) {
        self.shutdown_requested.store(true, Ordering::Relaxed);
        let _ = self.shutdown_tx.send(reason.clone());
        self.supervisor.request_shutdown(reason);
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

    pub(crate) async fn store_validated_worker_status(
        &self,
        report: WorkerStatusReport,
    ) -> Result<bool, String> {
        let project_id = report.project_id.clone();
        if self.projects.read().await.get(&project_id).is_none() {
            return Err("project not found".to_string());
        }
        if !self
            .supervisor
            .worker_identity_matches(&project_id, report.pid, &report.instance_token)
            .await
        {
            return Err("worker status does not match the current worker".to_string());
        }
        Ok(self.store_worker_status(report).await)
    }

    pub async fn load_projects(&self, path: PathBuf) {
        // The store is colocated with native registry metadata, never in a
        // project's Buddy database. Loading observations performs no wakes.
        let observations_path = path.with_file_name("buddy-observations.json");
        match crate::daemon::buddy_reconciliation::ReconciliationStore::load(
            observations_path,
            now_ms(),
        )
        .await
        {
            Ok(store) => *self.buddy_observations.lock().await = Some(store),
            Err(error) => {
                *self.buddy_observations.lock().await = None;
                tracing::warn!("Buddy daemon observations unavailable: {error}");
            }
        }
        // Only future explicit timer slots are eligible after restart; never catch up.
        match crate::daemon::buddy_wake::WakeStore::load(
            path.with_file_name("buddy-wake-grants.json"),
            now_ms(),
        )
        .await
        {
            Ok(store) => {
                for grant in store.all() {
                    self.buddy_wake_epochs.write().insert(
                        grant.project_id.clone(),
                        (
                            grant.grant_id.clone(),
                            self.supervisor.buddy_wake_fence(&grant.project_id),
                        ),
                    );
                }
                *self.buddy_wakes.lock().await = Some(store);
            }
            Err(error) => {
                *self.buddy_wakes.lock().await = None;
                tracing::warn!("Buddy wake grants unavailable: {error}");
            }
        }
        let registry = crate::daemon::projects::ProjectRegistry::load(path).await;
        let mut projects = self.projects.write().await;
        for entry in registry.list() {
            // Only registry restoration and explicit routes change desired state;
            // general liveness refreshes may carry stale entries.
            self.supervisor
                .set_automation_suspended(&entry.id, entry.automation_suspended);
            self.sync_project_liveness(&entry).await;
        }
        *projects = registry;
    }

    pub(crate) async fn buddy_wake_grant(
        &self,
        id: &str,
    ) -> Result<Option<crate::daemon::buddy_wake::WakeGrant>, String> {
        Ok(self
            .buddy_wakes
            .lock()
            .await
            .as_ref()
            .ok_or("wake store unavailable")?
            .get(id))
    }

    pub(crate) async fn issue_buddy_wake_grant(
        &self,
        id: &str,
        request: crate::daemon::buddy_wake::WakeGrantRequest,
        author: String,
    ) -> Result<crate::daemon::buddy_wake::WakeGrant, String> {
        let registry = self.projects.read().await;
        let entry = registry.get(id).ok_or("project not found")?;
        let mut guard = self.buddy_wakes.lock().await;
        self.supervisor.invalidate_buddy_wake(id);
        let epoch = self.supervisor.buddy_wake_fence(id);
        let grant = guard
            .as_mut()
            .ok_or("wake store unavailable")?
            .issue(entry, request, author, now_ms())
            .await?;
        self.buddy_wake_epochs
            .write()
            .insert(id.into(), (grant.grant_id.clone(), epoch));
        Ok(grant)
    }

    pub(crate) async fn revoke_buddy_wake_grant(&self, id: &str) -> Result<(), String> {
        // Revoke fence before first await, including when the disk writer is busy.
        self.supervisor.invalidate_buddy_wake(id);
        self.buddy_wakes
            .lock()
            .await
            .as_mut()
            .ok_or("wake store unavailable")?
            .revoke(id)
            .await
    }

    pub(crate) async fn tick_buddy_wakes(&self, now: u64) {
        let tick_started = std::time::Instant::now();
        // Missing configured user authentication disables starts, not GET/revoke.
        if self.is_shutting_down()
            || !self.config.auth.enabled
            || self
                .config
                .auth
                .password
                .as_ref()
                .is_none_or(|s| s.is_empty())
        {
            return;
        }
        let grants = self
            .buddy_wakes
            .lock()
            .await
            .as_mut()
            .map(|s| s.batch(now))
            .unwrap_or_default();
        for grant in grants {
            if self.config.auth.username.as_ref() != Some(&grant.authored_by) {
                continue;
            }
            let epoch = self
                .buddy_wake_epochs
                .read()
                .get(&grant.project_id)
                .cloned();
            let Some((id, epoch)) = epoch.filter(|(id, _)| id == &grant.grant_id) else {
                continue;
            };
            let registry = self.projects.read().await;
            let Some(entry) = registry.get(&grant.project_id) else {
                continue;
            };
            let outcome = self
                .supervisor
                .buddy_wake(entry, epoch, grant.request.expires_at_ms, || async {
                    if self.is_shutting_down()
                        || tokio::fs::canonicalize(&entry.root)
                            .await
                            .map_err(|e| e.to_string())?
                            != entry.root
                    {
                        return Ok(false);
                    }
                    self.buddy_wakes
                        .lock()
                        .await
                        .as_mut()
                        .ok_or("wake store unavailable")?
                        .consume(
                            entry,
                            &grant,
                            now.saturating_add(tick_started.elapsed().as_millis() as u64),
                        )
                        .await
                })
                .await;
            let message = match outcome {
                Ok(None) => continue,
                Ok(Some(info)) => format!(
                    "worker_observed_{:?}; no execution authority restored",
                    info.state
                ),
                Err(_) => "start_failed_or_unconfirmed; no budget refund".into(),
            };
            if let Some(store) = self.buddy_wakes.lock().await.as_mut() {
                if let Err(error) = store.outcome(&grant.project_id, &id, message).await {
                    tracing::warn!("Buddy wake outcome unavailable: {error}");
                }
            }
        }
    }

    pub(crate) async fn accept_buddy_projection(
        &self,
        report: crate::daemon::buddy_reconciliation::WorkerProjection,
    ) -> Result<bool, String> {
        let registry = self.projects.read().await;
        let entry = registry
            .get(&report.project_id)
            .ok_or("project not found")?;
        let token = report.instance_token.clone();
        self.supervisor
            .with_worker_identity(&entry.id, report.pid, &token, |identity| async {
                self.buddy_observations
                    .lock()
                    .await
                    .as_mut()
                    .ok_or("daemon observations unavailable")?
                    .accept(entry, identity, report, now_ms())
                    .await
            })
            .await
    }

    pub(crate) async fn buddy_projection(
        &self,
        id: &str,
    ) -> Result<Option<crate::daemon::buddy_reconciliation::ProjectObservation>, String> {
        self.reconcile_buddy_project(id, now_ms()).await?;
        Ok(self
            .buddy_observations
            .lock()
            .await
            .as_ref()
            .ok_or("daemon observations unavailable")?
            .get(id))
    }

    pub(crate) async fn reconcile_buddy_project(&self, id: &str, now: u64) -> Result<(), String> {
        let registry = self.projects.read().await;
        self.supervisor
            .with_observed_worker_identity(id, |identity| async move {
                if let Some(store) = self.buddy_observations.lock().await.as_mut() {
                    store
                        .reconcile(id, registry.get(id), identity.as_ref(), now)
                        .await?;
                }
                Ok(())
            })
            .await
    }

    pub(crate) async fn reconcile_buddy_batch(&self, now: u64) {
        let ids = self
            .buddy_observations
            .lock()
            .await
            .as_mut()
            .map(|s| s.batch())
            .unwrap_or_default();
        for id in ids {
            if let Err(error) = self.reconcile_buddy_project(&id, now).await {
                tracing::warn!("Buddy daemon reconciliation unavailable: {error}");
            }
        }
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
        self.supervisor
            .with_project_op_lock(project_id, || async {
                self.supervisor.note_project_activity(project_id, now).await;
                self.record_proxy_activity(project_id, now);
            })
            .await;
    }

    pub async fn refresh_proxy_activity(&self, project_id: &str) {
        let now = now_ms();
        self.supervisor.note_project_activity(project_id, now).await;
        self.record_proxy_activity(project_id, now);
    }

    fn record_proxy_activity(&self, project_id: &str, activity_ms: u64) {
        let mut activity = self.proxy_activity.write();
        let activity = activity.entry(project_id.to_string()).or_default();
        activity.last_proxy_activity_ms = activity.last_proxy_activity_ms.max(activity_ms);
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
        activity.last_proxy_activity_ms = now_ms();
        activity.live_proxy_streams = activity.live_proxy_streams.saturating_sub(1);
    }

    async fn stored_status_matches_current_worker(
        &self,
        project_id: &str,
        worker: Option<&WorkerInfo>,
        status: Option<&StoredWorkerStatus>,
    ) -> bool {
        let Some(worker) = worker else {
            return false;
        };
        let Some(status) = status else {
            return false;
        };
        if worker.pid != Some(status.report.pid) {
            return false;
        }
        self.supervisor
            .worker_identity_matches(project_id, status.report.pid, &status.report.instance_token)
            .await
    }

    pub(crate) async fn worker_idle_snapshot(
        &self,
        project_id: &str,
    ) -> Option<WorkerIdleSnapshot> {
        let entry = {
            let registry = self.projects.read().await;
            registry.get(project_id).cloned()
        }?;
        let status = self.worker_statuses.read().await.get(project_id).cloned();
        let activity = self
            .proxy_activity
            .read()
            .get(project_id)
            .cloned()
            .unwrap_or_default();
        let cron_next_fire_ms = self.cron_pending(project_id).await;
        let info = self.supervisor.worker_info(project_id).await?;
        let status = if self
            .stored_status_matches_current_worker(project_id, Some(&info), status.as_ref())
            .await
        {
            status.as_ref()
        } else {
            None
        };
        let mut snapshot =
            worker_idle_snapshot_from(&entry, &info, status, &activity, cron_next_fire_ms);
        snapshot.automation_background_active =
            self.automation_background_active(&entry.root).await;
        Some(snapshot)
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
            let status = if self
                .stored_status_matches_current_worker(&entry.id, Some(&info), status)
                .await
            {
                status
            } else {
                None
            };
            let activity = activity.get(&entry.id).cloned().unwrap_or_default();
            let mut snapshot = worker_idle_snapshot_from(
                &entry,
                &info,
                status,
                &activity,
                cron_pending.get(&entry.id).copied(),
            );
            snapshot.automation_background_active =
                self.automation_background_active(&entry.root).await;
            snapshots.push((entry.id.clone(), snapshot));
        }
        snapshots
    }

    async fn automation_background_active(&self, root: &Path) -> Option<bool> {
        if !crate::scheduler::runner::runner_enabled(&self.config.scheduler) {
            return Some(false);
        }
        // Re-observe in the stop predicate, not just on the coarse wake tick.
        // Live residency includes session-only leases; restart eligibility does not.
        // Unknown observation keeps a live worker but is never wake authority.
        crate::daemon::cron_clock::observe_automation(root, now_ms())
            .await
            .map(|observation| {
                observation.is_some_and(|observation| observation.summary.background_active)
            })
            .ok()
    }

    pub(crate) async fn stop_worker_if_idle(
        &self,
        project_id: &str,
    ) -> Result<Option<WorkerInfo>, String> {
        self.supervisor
            .stop_worker_if(project_id, || async {
                self.worker_idle_snapshot(project_id)
                    .await
                    .map(|snapshot| {
                        crate::daemon::idle::idle_decision(now_ms(), &snapshot, &self.config)
                            == crate::daemon::idle::IdleDecision::Stop
                    })
                    .unwrap_or(false)
            })
            .await
    }

    pub(crate) async fn purge_project_runtime(&self, project_id: &str) -> Result<(), String> {
        self.supervisor.invalidate_buddy_wake(project_id);
        if let Some(store) = self.buddy_wakes.lock().await.as_mut() {
            store.revoke(project_id).await?;
        }
        self.supervisor.forget_worker(project_id).await?;
        self.worker_statuses.write().await.remove(project_id);
        self.proxy_activity.write().remove(project_id);
        self.set_cron_pending(project_id, None).await;
        Ok(())
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
            let status = if self
                .stored_status_matches_current_worker(&entry.id, worker.as_ref(), status)
                .await
            {
                status
            } else {
                None
            };
            let activity = activity.get(&entry.id).cloned().unwrap_or_default();
            let last_activity_ms = entry
                .last_active_ms
                .max(activity.last_proxy_activity_ms)
                .max(
                    status
                        .map(|status| status.report.last_activity_ts)
                        .unwrap_or(0),
                );
            let mut idle_deadline_ms = idle_deadline_ms(
                now,
                &entry,
                worker.as_ref(),
                status,
                &activity,
                cron_pending.get(&entry.id).copied(),
                self.config.idle_timeout_secs,
            );
            if self.automation_background_active(&entry.root).await != Some(false) {
                idle_deadline_ms = None;
            }
            rows.push(WorkerRow {
                project_id: entry.id.clone(),
                slug: entry.slug.clone(),
                root: entry.root.clone(),
                root_exists: entry.root.exists(),
                pinned: entry.pinned,
                last_active_ms: last_activity_ms,
                state: worker
                    .as_ref()
                    .map(|worker| worker.state.clone())
                    .unwrap_or(WorkerState::Stopped),
                pid: worker.as_ref().and_then(|worker| worker.pid),
                rss_bytes: None,
                cpu_percent: None,
                uptime_secs: None,
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
                log_path: self
                    .daemon_dir
                    .join("logs")
                    .join(format!("worker-{}.log", entry.slug))
                    .to_string_lossy()
                    .to_string(),
            });
        }
        let pids = rows.iter().filter_map(|row| row.pid).collect::<Vec<_>>();
        let resources =
            tokio::task::spawn_blocking(move || crate::daemon::resources::worker_resources(&pids))
                .await
                .unwrap_or_default();
        for row in &mut rows {
            let Some(sample) = row.pid.and_then(|pid| resources.get(&pid)) else {
                continue;
            };
            row.rss_bytes = Some(sample.rss_bytes);
            row.cpu_percent = Some(sample.cpu_percent);
            row.uptime_secs = Some(sample.uptime_secs);
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
            executable_sha256: current_executable_sha256_if_computed(),
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

fn worker_idle_snapshot_from(
    entry: &crate::daemon::projects::ProjectEntry,
    info: &WorkerInfo,
    status: Option<&StoredWorkerStatus>,
    activity: &ProxyActivity,
    cron_next_fire_ms: Option<u64>,
) -> WorkerIdleSnapshot {
    WorkerIdleSnapshot {
        state: info.state.clone(),
        pinned: entry.pinned,
        live_proxy_streams: activity.live_proxy_streams.min(u64::from(u32::MAX)) as u32,
        last_proxy_activity_ms: activity
            .last_proxy_activity_ms
            .max(entry.last_active_ms)
            .max(
                status
                    .map(|status| status.report.last_activity_ts)
                    .unwrap_or(0),
            ),
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
        cron_next_fire_ms,
        automation_background_active: Some(false),
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
    let Some(status) = status else {
        return None;
    };
    if now.saturating_sub(status.received_ms) >= STATUS_FRESH_MS {
        return None;
    }
    if status.report.lsp_clients > 0
        || status.report.busy_chats > 0
        || status.report.exec_running > 0
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
        .max(status.report.last_activity_ts);
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
    publish_daemon_state_tmp(&tmp, path).await?;
    Ok(())
}

#[cfg(not(windows))]
async fn publish_daemon_state_tmp(tmp: &Path, path: &Path) -> Result<(), String> {
    tokio::fs::rename(tmp, path)
        .await
        .map_err(|error| format!("failed to publish {}: {error}", path.display()))
}

#[cfg(windows)]
async fn publish_daemon_state_tmp(tmp: &Path, path: &Path) -> Result<(), String> {
    match tokio::fs::remove_file(path).await {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(format!("failed to replace {}: {error}", path.display())),
    }
    tokio::fs::rename(tmp, path)
        .await
        .map_err(|error| format!("failed to publish {}: {error}", path.display()))
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
    use crate::daemon::projects::{ProjectEntry, ProjectSettings};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    #[test]
    fn daemon_state_keeps_executable_path_across_atomic_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("refact-agent");
        let replacement = dir.path().join("refact-agent.new");
        std::fs::write(&executable, b"old").unwrap();
        std::fs::write(&replacement, b"new").unwrap();
        let state = DaemonState::new_with_daemon_dir_and_executable(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
            dir.path().join("daemon"),
            0,
            Ok(executable.canonicalize().unwrap()),
        );

        std::fs::rename(&replacement, &executable).unwrap();

        assert_eq!(
            std::fs::read(state.relaunch_executable().unwrap()).unwrap(),
            b"new"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stable_executable_path_recovers_replaced_linux_executable() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("refact-agent");
        std::fs::write(&executable, b"new").unwrap();
        let mut deleted_path = executable.as_os_str().as_bytes().to_vec();
        deleted_path.extend_from_slice(b" (deleted)");

        assert_eq!(
            stable_executable_path(PathBuf::from(std::ffi::OsString::from_vec(deleted_path))),
            executable.canonicalize().unwrap()
        );
    }

    fn test_daemon_info() -> DaemonInfo {
        DaemonInfo {
            pid: 42,
            port: 8488,
            bind: "127.0.0.1".to_string(),
            version: "1.2.3".to_string(),
            executable_sha256: None,
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
            instance_token: "token".to_string(),
            lsp_clients,
            busy_chats,
            exec_running,
            last_activity_ts: 11,
        }
    }

    fn test_project_entry() -> ProjectEntry {
        ProjectEntry {
            id: "project".to_string(),
            slug: "project".to_string(),
            root: PathBuf::from("/tmp/project"),
            pinned: false,
            last_active_ms: 100,
            automation_suspended: false,
            settings: ProjectSettings::default(),
        }
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

    #[test]
    fn new_uses_hook_token_for_worker_link_when_daemon_auth_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let config = DaemonConfig {
            hooks: crate::daemon::config::HooksConfig {
                enabled: true,
                token: Some("hook-secret".to_string()),
                ..Default::default()
            },
            ..DaemonConfig::default()
        };
        let state = DaemonState::new(config, EventBus::new(dir.path().join("e.jsonl")), None);

        assert_eq!(
            state.supervisor.test_daemon_auth_token().as_deref(),
            Some("hook-secret")
        );
    }

    #[tokio::test]
    async fn daemon_json_atomic_write_replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.json");
        let mut info = test_daemon_info();
        write_daemon_info_atomic(&path, &info).await.unwrap();

        info.port = 9999;
        write_daemon_info_atomic(&path, &info).await.unwrap();

        assert_eq!(read_daemon_info(&path).await.unwrap(), Some(info));
    }

    #[tokio::test]
    async fn idle_snapshot_ignores_status_from_previous_pid() {
        let dir = tempfile::tempdir().unwrap();
        let state = DaemonState::new_with_daemon_dir(
            DaemonConfig::default(),
            EventBus::new(dir.path().join("events.jsonl")),
            None,
            dir.path().join("daemon"),
            0,
        );
        let project_root = dir.path().join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let entry = {
            let mut registry = state.projects.write().await;
            registry.open(project_root).await.unwrap()
        };
        state
            .supervisor
            .set_test_worker_info(&entry.id, 8, WorkerState::Ready, "current-token")
            .await;
        let mut stale = status_report(5, 6, 7);
        stale.project_id = entry.id.clone();
        stale.pid = 7;
        stale.instance_token = "old-token".to_string();
        stale.last_activity_ts = 999_999;
        state.store_worker_status(stale).await;

        let snapshot = state.worker_idle_snapshot(&entry.id).await.unwrap();

        assert_eq!(snapshot.lsp_clients, 0);
        assert_eq!(snapshot.busy_chats, 0);
        assert_eq!(snapshot.exec_running, 0);
        assert_eq!(snapshot.last_status_report_ms, 0);
        assert_eq!(snapshot.last_proxy_activity_ms, entry.last_active_ms);
        let rows = state.worker_rows().await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].busy_chats, 0);
        assert_eq!(rows[0].last_status_report_ms, None);
        assert_eq!(rows[0].idle_deadline_ms, None);
    }

    #[test]
    fn idle_deadline_requires_fresh_status() {
        let entry = test_project_entry();
        let worker = WorkerInfo {
            project_id: entry.id.clone(),
            pid: Some(7),
            http_port: 1,
            lsp_port: 2,
            state: WorkerState::Ready,
            last_error: None,
        };
        let activity = ProxyActivity {
            last_proxy_activity_ms: 100,
            live_proxy_streams: 0,
        };
        let status = StoredWorkerStatus {
            report: status_report(0, 0, 0),
            received_ms: 1,
        };

        assert_eq!(
            idle_deadline_ms(
                STATUS_FRESH_MS + 1,
                &entry,
                Some(&worker),
                Some(&status),
                &activity,
                None,
                2,
            ),
            None
        );
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
}
