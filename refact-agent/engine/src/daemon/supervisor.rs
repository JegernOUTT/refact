//! Worker supervisor for daemon-managed projects.
//!
//! Integration tests and downstream cards can set `REFACT_DAEMON_WORKER_CMD` to a fake worker command.
//! The supervisor appends the normal worker flags to that command, so `tests/fake_worker.py` can
//! exercise readiness, graceful shutdown, and crash-loop handling without launching the full engine.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock as SyncRwLock;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Child;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::daemon::events::EventBus;
use crate::daemon::ports::PortPair;
use crate::daemon::projects::{ProjectEntry, ProjectSettings};
use crate::daemon::state::now_ms;

const READINESS_POLL: Duration = Duration::from_millis(250);
const READINESS_TIMEOUT: Duration = Duration::from_secs(120);
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(10);
const KILL_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const CRASH_WINDOW_MS: u64 = 10 * 60 * 1000;
const MAX_PORT_BUSY_RETRIES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Stopped,
    Starting,
    Ready,
    Stopping,
    Crashed,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub project_id: String,
    pub pid: Option<u32>,
    pub http_port: u16,
    pub lsp_port: u16,
    pub state: WorkerState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct WorkerLaunchSpec {
    project_id: String,
    slug: String,
    root: PathBuf,
    settings: ProjectSettings,
}

impl From<&ProjectEntry> for WorkerLaunchSpec {
    fn from(entry: &ProjectEntry) -> Self {
        Self {
            project_id: entry.id.clone(),
            slug: entry.slug.clone(),
            root: entry.root.clone(),
            settings: entry.settings.clone(),
        }
    }
}

struct WorkerRecord {
    info: WorkerInfo,
    child: Option<Child>,
    generation: u64,
    crash_history: VecDeque<u64>,
    monitor_task: Option<JoinHandle<()>>,
}

struct WorkerSlot {
    op_lock: Mutex<()>,
    record: Mutex<WorkerRecord>,
}

pub struct Supervisor {
    workers: RwLock<HashMap<String, Arc<WorkerSlot>>>,
    events: EventBus,
    daemon_dir: PathBuf,
    daemon_port: RwLock<u16>,
    cron_pending: Arc<SyncRwLock<HashMap<String, u64>>>,
    client: reqwest::Client,
}

impl Supervisor {
    pub fn new(events: EventBus, daemon_dir: PathBuf, daemon_port: u16) -> Arc<Self> {
        Self::new_with_cron_pending(
            events,
            daemon_dir,
            daemon_port,
            Arc::new(SyncRwLock::new(HashMap::new())),
        )
    }

    pub(crate) fn new_with_cron_pending(
        events: EventBus,
        daemon_dir: PathBuf,
        daemon_port: u16,
        cron_pending: Arc<SyncRwLock<HashMap<String, u64>>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            workers: RwLock::new(HashMap::new()),
            events,
            daemon_dir,
            daemon_port: RwLock::new(daemon_port),
            cron_pending,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .expect("failed to build daemon supervisor http client"),
        })
    }

    pub async fn set_daemon_port(&self, port: u16) {
        *self.daemon_port.write().await = port;
    }

    pub async fn ensure_worker(
        self: &Arc<Self>,
        entry: &ProjectEntry,
    ) -> Result<WorkerInfo, String> {
        let spec = WorkerLaunchSpec::from(entry);
        let slot = self.slot_for(&spec.project_id).await;
        let _guard = slot.op_lock.lock().await;
        if let Some(info) = self.reusable_info(&slot).await {
            return Ok(info);
        }
        self.spawn_worker_locked(slot.clone(), spec, "ensure").await
    }

    pub async fn restart_worker(
        self: &Arc<Self>,
        entry: &ProjectEntry,
    ) -> Result<WorkerInfo, String> {
        let spec = WorkerLaunchSpec::from(entry);
        let slot = self.slot_for(&spec.project_id).await;
        let _guard = slot.op_lock.lock().await;
        self.stop_slot_locked(&slot, &spec.project_id, false)
            .await?;
        {
            let mut record = slot.record.lock().await;
            record.crash_history.clear();
        }
        self.spawn_worker_locked(slot.clone(), spec, "manual_restart")
            .await
    }

    pub async fn stop_worker(&self, project_id: &str) -> Result<Option<WorkerInfo>, String> {
        let Some(slot) = self.get_slot(project_id).await else {
            return Ok(None);
        };
        let _guard = slot.op_lock.lock().await;
        let info = self.stop_slot_locked(&slot, project_id, true).await?;
        Ok(Some(info))
    }

    pub async fn stop_all(&self) {
        let ids: Vec<String> = self.workers.read().await.keys().cloned().collect();
        for id in ids {
            let _ = self.stop_worker(&id).await;
        }
    }

    pub async fn worker_info(&self, project_id: &str) -> Option<WorkerInfo> {
        let slot = self.get_slot(project_id).await?;
        let info = slot.record.lock().await.info.clone();
        Some(info)
    }

    pub async fn worker_count(&self) -> u64 {
        let slots: Vec<Arc<WorkerSlot>> = self.workers.read().await.values().cloned().collect();
        let mut count = 0;
        for slot in slots {
            let state = slot.record.lock().await.info.state.clone();
            if !matches!(state, WorkerState::Stopped) {
                count += 1;
            }
        }
        count
    }

    pub fn cron_pending(&self, project_id: &str) -> Option<u64> {
        self.cron_pending.read().get(project_id).copied()
    }

    fn wants_alive(&self, project_id: &str) -> bool {
        self.cron_pending(project_id)
            .map(|next_fire_ms| {
                crate::daemon::cron_clock::cron_pending_blocks_idle_stop(next_fire_ms, now_ms())
            })
            .unwrap_or(true)
    }

    async fn slot_for(&self, project_id: &str) -> Arc<WorkerSlot> {
        if let Some(slot) = self.workers.read().await.get(project_id).cloned() {
            return slot;
        }
        let mut workers = self.workers.write().await;
        workers
            .entry(project_id.to_string())
            .or_insert_with(|| Arc::new(WorkerSlot::new(project_id.to_string())))
            .clone()
    }

    async fn get_slot(&self, project_id: &str) -> Option<Arc<WorkerSlot>> {
        self.workers.read().await.get(project_id).cloned()
    }

    async fn reusable_info(&self, slot: &Arc<WorkerSlot>) -> Option<WorkerInfo> {
        let mut record = slot.record.lock().await;
        match record.info.state.clone() {
            WorkerState::Ready => {
                if child_is_alive(&mut record.child) {
                    return Some(record.info.clone());
                }
                record.child = None;
                record.info.pid = None;
                None
            }
            WorkerState::Starting | WorkerState::Stopping | WorkerState::Crashed => {
                Some(record.info.clone())
            }
            WorkerState::Stopped | WorkerState::Failed { .. } => None,
        }
    }

    async fn spawn_worker_locked(
        self: &Arc<Self>,
        slot: Arc<WorkerSlot>,
        spec: WorkerLaunchSpec,
        reason: &str,
    ) -> Result<WorkerInfo, String> {
        for attempt in 1..=MAX_PORT_BUSY_RETRIES {
            let ports = crate::daemon::ports::allocate_port_pair()?;
            let nonce = uuid::Uuid::new_v4().to_string();
            let child = match self.spawn_child(&spec, ports, &nonce).await {
                Ok(child) => child,
                Err(error) => {
                    self.mark_failed(&slot, &spec.project_id, error.clone())
                        .await;
                    return Err(format!("failed to spawn worker: {error}"));
                }
            };
            let pid = child.id();
            let generation = {
                let mut record = slot.record.lock().await;
                abort_task(&mut record.monitor_task);
                record.generation = record.generation.saturating_add(1);
                record.child = Some(child);
                record.info = WorkerInfo {
                    project_id: spec.project_id.clone(),
                    pid,
                    http_port: ports.http_port,
                    lsp_port: ports.lsp_port,
                    state: WorkerState::Starting,
                    last_error: None,
                };
                record.generation
            };
            let _ = self
                .events
                .emit(
                    "worker_starting",
                    Some(spec.project_id.clone()),
                    json!({
                        "pid": pid,
                        "http_port": ports.http_port,
                        "lsp_port": ports.lsp_port,
                        "reason": reason,
                    }),
                )
                .await;

            match self
                .wait_until_ready_or_exit(&slot, generation, ports.http_port, &nonce)
                .await?
            {
                ReadinessOutcome::Ready => {
                    let monitor = self.monitor_handle(slot.clone(), spec.clone(), generation);
                    let info = {
                        let mut record = slot.record.lock().await;
                        if record.generation == generation {
                            abort_task(&mut record.monitor_task);
                            record.monitor_task = Some(monitor);
                            record.info.state = WorkerState::Ready;
                            record.info.last_error = None;
                            record.info.clone()
                        } else {
                            monitor.abort();
                            record.info.clone()
                        }
                    };
                    let _ = self
                        .events
                        .emit(
                            "worker_ready",
                            Some(spec.project_id.clone()),
                            json!({
                                "pid": info.pid,
                                "http_port": info.http_port,
                                "lsp_port": info.lsp_port,
                            }),
                        )
                        .await;
                    return Ok(info);
                }
                ReadinessOutcome::Exited(exit_code)
                    if is_port_busy_exit(exit_code) && attempt < MAX_PORT_BUSY_RETRIES =>
                {
                    let _ = self
                        .events
                        .emit(
                            "worker_exited",
                            Some(spec.project_id.clone()),
                            json!({
                                "exit_code": exit_code,
                                "during_startup": true,
                                "retrying_ports": true,
                                "attempt": attempt,
                            }),
                        )
                        .await;
                    continue;
                }
                ReadinessOutcome::Exited(exit_code) => {
                    let (info, delay) = self
                        .record_unexpected_exit(
                            &slot,
                            &spec.project_id,
                            generation,
                            exit_code,
                            "worker exited before readiness".to_string(),
                        )
                        .await;
                    self.emit_exit_or_crash(&info, exit_code, true, delay).await;
                    if let Some(delay) = delay {
                        let monitor = self.delayed_restart_handle(
                            slot.clone(),
                            spec.clone(),
                            generation,
                            delay,
                        );
                        let mut record = slot.record.lock().await;
                        if record.generation == generation {
                            abort_task(&mut record.monitor_task);
                            record.monitor_task = Some(monitor);
                        } else {
                            monitor.abort();
                        }
                    }
                    return Ok(info);
                }
                ReadinessOutcome::Timeout => {
                    self.kill_generation_child(&slot, generation).await;
                    let info = self
                        .mark_failed(
                            &slot,
                            &spec.project_id,
                            "worker readiness timed out".to_string(),
                        )
                        .await;
                    let _ = self
                        .events
                        .emit(
                            "worker_exited",
                            Some(spec.project_id.clone()),
                            json!({"reason": "readiness_timeout", "will_restart": false}),
                        )
                        .await;
                    return Ok(info);
                }
            }
        }
        let info = self
            .mark_failed(
                &slot,
                &spec.project_id,
                "worker port allocation retry limit reached".to_string(),
            )
            .await;
        Ok(info)
    }

    async fn spawn_child(
        &self,
        spec: &WorkerLaunchSpec,
        ports: PortPair,
        nonce: &str,
    ) -> Result<Child, String> {
        let mut command = self.worker_command_base()?;
        command.args(worker_args(
            spec,
            ports,
            nonce,
            *self.daemon_port.read().await,
            self.daemon_dir.clone(),
        ));
        command.current_dir(&spec.root);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        command.kill_on_drop(true);
        command
            .spawn()
            .map_err(|error| format!("failed to spawn worker command: {error}"))
    }

    fn worker_command_base(&self) -> Result<tokio::process::Command, String> {
        if let Ok(command) = std::env::var("REFACT_DAEMON_WORKER_CMD") {
            let parts = shell_words::split(&command)
                .map_err(|error| format!("failed to parse REFACT_DAEMON_WORKER_CMD: {error}"))?;
            let Some((program, args)) = parts.split_first() else {
                return Err("REFACT_DAEMON_WORKER_CMD is empty".to_string());
            };
            let mut cmd = tokio::process::Command::new(program);
            cmd.args(args);
            return Ok(cmd);
        }
        let exe = std::env::current_exe()
            .map_err(|error| format!("failed to resolve current executable: {error}"))?;
        let mut cmd = tokio::process::Command::new(exe);
        cmd.arg("worker");
        Ok(cmd)
    }

    async fn wait_until_ready_or_exit(
        &self,
        slot: &Arc<WorkerSlot>,
        generation: u64,
        http_port: u16,
        nonce: &str,
    ) -> Result<ReadinessOutcome, String> {
        let deadline = Instant::now() + READINESS_TIMEOUT;
        let url = format!("http://127.0.0.1:{http_port}/v1/ping");
        loop {
            if let Some(exit_code) = take_exited_child(slot, generation).await? {
                return Ok(ReadinessOutcome::Exited(exit_code));
            }
            if Instant::now() >= deadline {
                return Ok(ReadinessOutcome::Timeout);
            }
            if let Ok(response) = self.client.get(&url).send().await {
                if let Ok(body) = response.text().await {
                    if body.trim() == nonce {
                        return Ok(ReadinessOutcome::Ready);
                    }
                }
            }
            tokio::time::sleep(READINESS_POLL).await;
        }
    }

    fn monitor_handle(
        self: &Arc<Self>,
        slot: Arc<WorkerSlot>,
        spec: WorkerLaunchSpec,
        generation: u64,
    ) -> JoinHandle<()> {
        let supervisor = self.clone();
        tokio::spawn(async move {
            supervisor.monitor_worker(slot, spec, generation).await;
        })
    }

    fn delayed_restart_handle(
        self: &Arc<Self>,
        slot: Arc<WorkerSlot>,
        spec: WorkerLaunchSpec,
        generation: u64,
        delay: Duration,
    ) -> JoinHandle<()> {
        let supervisor = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            supervisor.restart_after_delay(slot, spec, generation).await;
        })
    }

    async fn restart_after_delay(
        self: Arc<Self>,
        slot: Arc<WorkerSlot>,
        spec: WorkerLaunchSpec,
        generation: u64,
    ) {
        let _guard = slot.op_lock.lock().await;
        {
            let mut record = slot.record.lock().await;
            if record.generation != generation
                || !matches!(record.info.state.clone(), WorkerState::Starting)
                || record.child.is_some()
            {
                return;
            }
            record.monitor_task = None;
        }
        let _ = self
            .spawn_worker_locked(slot.clone(), spec.clone(), "auto_restart")
            .await;
    }

    async fn monitor_worker(
        self: Arc<Self>,
        slot: Arc<WorkerSlot>,
        spec: WorkerLaunchSpec,
        generation: u64,
    ) {
        loop {
            tokio::time::sleep(READINESS_POLL).await;
            let exit_code = match take_exited_child(&slot, generation).await {
                Ok(Some(exit_code)) => exit_code,
                Ok(None) => {
                    if !self.generation_is_active(&slot, generation).await {
                        return;
                    }
                    continue;
                }
                Err(error) => {
                    tracing::warn!("worker monitor failed for {}: {error}", spec.project_id);
                    return;
                }
            };
            if self
                .record_stopped_if_requested(&slot, &spec.project_id, generation)
                .await
            {
                return;
            }
            let (info, delay) = self
                .record_unexpected_exit(
                    &slot,
                    &spec.project_id,
                    generation,
                    exit_code,
                    "worker exited".to_string(),
                )
                .await;
            self.emit_exit_or_crash(&info, exit_code, false, delay)
                .await;
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
                self.restart_after_delay(slot.clone(), spec.clone(), generation)
                    .await;
            }
            return;
        }
    }

    async fn generation_is_active(&self, slot: &Arc<WorkerSlot>, generation: u64) -> bool {
        let record = slot.record.lock().await;
        record.generation == generation && record.child.is_some()
    }

    async fn record_stopped_if_requested(
        &self,
        slot: &Arc<WorkerSlot>,
        project_id: &str,
        generation: u64,
    ) -> bool {
        let info = {
            let mut record = slot.record.lock().await;
            if record.generation != generation
                || !matches!(record.info.state.clone(), WorkerState::Stopping)
            {
                return false;
            }
            record.info.pid = None;
            record.info.state = WorkerState::Stopped;
            record.info.last_error = None;
            record.info.clone()
        };
        let _ = self
            .events
            .emit(
                "worker_stopped",
                Some(project_id.to_string()),
                json!({"pid": info.pid}),
            )
            .await;
        true
    }

    async fn record_unexpected_exit(
        &self,
        slot: &Arc<WorkerSlot>,
        project_id: &str,
        generation: u64,
        exit_code: Option<i32>,
        reason: String,
    ) -> (WorkerInfo, Option<Duration>) {
        let now = now_ms();
        let mut record = slot.record.lock().await;
        if record.generation != generation {
            return (record.info.clone(), None);
        }
        push_crash(&mut record.crash_history, now);
        let delay =
            next_restart_delay_from_window(&record.crash_history, now).map(runtime_restart_delay);
        record.child = None;
        record.info.pid = None;
        record.info.last_error = Some(match exit_code {
            Some(code) => format!("{reason} (exit code {code})"),
            None => format!("{reason} (signal)"),
        });
        if delay.is_some() && self.wants_alive(project_id) {
            record.info.state = WorkerState::Starting;
        } else if delay.is_none() {
            record.info.state = WorkerState::Crashed;
        } else {
            record.info.state = WorkerState::Stopped;
        }
        (
            record.info.clone(),
            delay.filter(|_| self.wants_alive(project_id)),
        )
    }

    async fn emit_exit_or_crash(
        &self,
        info: &WorkerInfo,
        exit_code: Option<i32>,
        during_startup: bool,
        delay: Option<Duration>,
    ) {
        let will_restart = delay.is_some();
        let _ = self
            .events
            .emit(
                "worker_exited",
                Some(info.project_id.clone()),
                json!({
                    "exit_code": exit_code,
                    "during_startup": during_startup,
                    "will_restart": will_restart,
                    "restart_delay_ms": delay.map(|d| d.as_millis() as u64),
                }),
            )
            .await;
        if matches!(info.state.clone(), WorkerState::Crashed) {
            let _ = self
                .events
                .emit(
                    "worker_crashed",
                    Some(info.project_id.clone()),
                    json!({"last_error": info.last_error}),
                )
                .await;
        }
        if matches!(info.state.clone(), WorkerState::Crashed)
            && self.cron_pending(&info.project_id).is_some()
        {
            let _ = self
                .events
                .emit(
                    "cron_worker_crashed",
                    Some(info.project_id.clone()),
                    json!({"next_fire_ms": self.cron_pending(&info.project_id)}),
                )
                .await;
        }
    }

    async fn stop_slot_locked(
        &self,
        slot: &Arc<WorkerSlot>,
        project_id: &str,
        emit_event: bool,
    ) -> Result<WorkerInfo, String> {
        let generation = {
            let mut record = slot.record.lock().await;
            abort_task(&mut record.monitor_task);
            record.generation = record.generation.saturating_add(1);
            record.info.state = if record.child.is_some() {
                WorkerState::Stopping
            } else {
                WorkerState::Stopped
            };
            record.generation
        };
        let (http_port, had_child) = {
            let record = slot.record.lock().await;
            (record.info.http_port, record.child.is_some())
        };
        if had_child {
            let _ = self
                .client
                .post(format!("http://127.0.0.1:{http_port}/v1/graceful-shutdown"))
                .send()
                .await;
            if self
                .wait_for_generation_exit(slot, generation, GRACEFUL_STOP_TIMEOUT)
                .await?
                .is_none()
            {
                self.kill_generation_child(slot, generation).await;
            }
        }
        let info = {
            let mut record = slot.record.lock().await;
            record.child = None;
            record.info.pid = None;
            record.info.state = WorkerState::Stopped;
            record.info.last_error = None;
            record.info.clone()
        };
        if emit_event {
            let _ = self
                .events
                .emit("worker_stopped", Some(project_id.to_string()), json!({}))
                .await;
        }
        Ok(info)
    }

    async fn wait_for_generation_exit(
        &self,
        slot: &Arc<WorkerSlot>,
        generation: u64,
        timeout: Duration,
    ) -> Result<Option<Option<i32>>, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(exit_code) = take_exited_child(slot, generation).await? {
                return Ok(Some(exit_code));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(READINESS_POLL).await;
        }
    }

    async fn kill_generation_child(&self, slot: &Arc<WorkerSlot>, generation: u64) {
        {
            let mut record = slot.record.lock().await;
            if record.generation == generation {
                if let Some(child) = record.child.as_mut() {
                    let _ = child.start_kill();
                }
            }
        }
        let _ = self
            .wait_for_generation_exit(slot, generation, KILL_WAIT_TIMEOUT)
            .await;
    }

    async fn mark_failed(
        &self,
        slot: &Arc<WorkerSlot>,
        project_id: &str,
        reason: String,
    ) -> WorkerInfo {
        let mut record = slot.record.lock().await;
        record.child = None;
        record.info = WorkerInfo {
            project_id: project_id.to_string(),
            pid: None,
            http_port: record.info.http_port,
            lsp_port: record.info.lsp_port,
            state: WorkerState::Failed {
                reason: reason.clone(),
            },
            last_error: Some(reason),
        };
        record.info.clone()
    }
}

impl WorkerSlot {
    fn new(project_id: String) -> Self {
        Self {
            op_lock: Mutex::new(()),
            record: Mutex::new(WorkerRecord {
                info: WorkerInfo {
                    project_id,
                    pid: None,
                    http_port: 0,
                    lsp_port: 0,
                    state: WorkerState::Stopped,
                    last_error: None,
                },
                child: None,
                generation: 0,
                crash_history: VecDeque::new(),
                monitor_task: None,
            }),
        }
    }
}

enum ReadinessOutcome {
    Ready,
    Exited(Option<i32>),
    Timeout,
}

fn worker_args(
    spec: &WorkerLaunchSpec,
    ports: PortPair,
    nonce: &str,
    daemon_port: u16,
    daemon_dir: PathBuf,
) -> Vec<String> {
    let mut args = vec![
        "--workspace-folder".to_string(),
        spec.root.to_string_lossy().to_string(),
        "--http-port".to_string(),
        ports.http_port.to_string(),
        "--http-host".to_string(),
        "127.0.0.1".to_string(),
        "--lsp-port".to_string(),
        ports.lsp_port.to_string(),
        "--ping-message".to_string(),
        nonce.to_string(),
        "--project-id".to_string(),
        spec.project_id.clone(),
        "--daemon-endpoint".to_string(),
        format!("http://127.0.0.1:{daemon_port}"),
        "--logs-to-file".to_string(),
        daemon_dir
            .join("logs")
            .join(format!("worker-{}.log", spec.slug))
            .to_string_lossy()
            .to_string(),
    ];
    if spec.settings.ast {
        args.push("--ast".to_string());
        args.push("--ast-max-files".to_string());
        args.push(spec.settings.ast_max_files.to_string());
    }
    if spec.settings.vecdb {
        args.push("--vecdb".to_string());
        args.push("--vecdb-max-files".to_string());
        args.push(spec.settings.vecdb_max_files.to_string());
    }
    args
}

fn child_is_alive(child: &mut Option<Child>) -> bool {
    let alive = match child.as_mut() {
        Some(child) => matches!(child.try_wait(), Ok(None)),
        None => false,
    };
    if !alive {
        *child = None;
    }
    alive
}

async fn take_exited_child(
    slot: &Arc<WorkerSlot>,
    generation: u64,
) -> Result<Option<Option<i32>>, String> {
    let mut record = slot.record.lock().await;
    if record.generation != generation {
        return Ok(None);
    }
    let Some(child) = record.child.as_mut() else {
        return Ok(None);
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            let code = status.code();
            record.child = None;
            record.info.pid = None;
            Ok(Some(code))
        }
        Ok(None) => Ok(None),
        Err(error) => Err(format!("failed to poll worker child: {error}")),
    }
}

fn abort_task(task: &mut Option<JoinHandle<()>>) {
    if let Some(task) = task.take() {
        task.abort();
    }
}

fn is_port_busy_exit(exit_code: Option<i32>) -> bool {
    matches!(exit_code, Some(0) | Some(48) | Some(98) | Some(10048))
}

fn push_crash(history: &mut VecDeque<u64>, now: u64) {
    while history
        .front()
        .map(|ts| now.saturating_sub(*ts) > CRASH_WINDOW_MS)
        .unwrap_or(false)
    {
        history.pop_front();
    }
    history.push_back(now);
}

pub fn next_restart_delay(crash_history: &[u64], now: u64) -> Option<Duration> {
    let recent = crash_history
        .iter()
        .filter(|ts| now.saturating_sub(**ts) <= CRASH_WINDOW_MS)
        .count();
    if recent > 5 {
        return None;
    }
    Some(match recent {
        0 => Duration::from_secs(1),
        1 => Duration::from_secs(1),
        2 => Duration::from_secs(5),
        _ => Duration::from_secs(30),
    })
}

fn next_restart_delay_from_window(history: &VecDeque<u64>, now: u64) -> Option<Duration> {
    let values: Vec<u64> = history.iter().copied().collect();
    next_restart_delay(&values, now)
}

fn runtime_restart_delay(delay: Duration) -> Duration {
    std::env::var("REFACT_DAEMON_SUPERVISOR_BACKOFF_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_delay_follows_backoff_schedule() {
        assert_eq!(next_restart_delay(&[], 100), Some(Duration::from_secs(1)));
        assert_eq!(
            next_restart_delay(&[100], 100),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            next_restart_delay(&[100, 200], 200),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            next_restart_delay(&[100, 200, 300, 400, 500], 500),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            next_restart_delay(&[100, 200, 300, 400, 500, 600], 600),
            None
        );
    }

    #[test]
    fn restart_delay_ignores_old_crashes() {
        let now = CRASH_WINDOW_MS + 10;
        assert_eq!(
            next_restart_delay(&[1, now - 2, now - 1], now),
            Some(Duration::from_secs(5))
        );
    }

    #[test]
    fn push_crash_prunes_window() {
        let mut history = VecDeque::from(vec![1, 2]);
        push_crash(&mut history, CRASH_WINDOW_MS + 2);
        assert_eq!(history, VecDeque::from(vec![2, CRASH_WINDOW_MS + 2]));
    }
}
