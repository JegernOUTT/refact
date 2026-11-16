use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use refact_python_kernel::client::{ClientEvent, ClientOptions, KernelClient, WorkerCommand};
use refact_python_kernel::protocol::{KernelCommand, KernelEvent};
use serde_json::Value;
use tokio::sync::{oneshot, Mutex as AsyncMutex, OwnedMutexGuard};

use crate::app_state::AppState;
use super::workspace::{PersistOutcome, PythonWorkspace};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PythonBusy {
    pub job_id: String,
    pub tool_call_id: String,
}
impl std::fmt::Display for PythonBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Python job {} (tool call {}) is still running; use tool_job to wait for or cancel it",
            self.job_id, self.tool_call_id
        )
    }
}
impl std::error::Error for PythonBusy {}

#[derive(Debug)]
pub struct SessionStart {
    pub restored: bool,
    pub reset_reason: Option<String>,
}

#[derive(Clone, Copy, Default)]
pub enum WorkerBackend {
    #[default]
    Process,
    InThread,
}

struct Queue {
    active: Option<u64>,
    next: u64,
    background: Option<PythonBusy>,
    waiting: VecDeque<(u64, oneshot::Sender<Result<(), PythonBusy>>)>,
    used: Instant,
}
struct Session {
    client: Option<KernelClient>,
    workspace: Option<PythonWorkspace>,
    max_state_bytes: u64,
    last_run_id: String,
    taint: Vec<Value>,
    reset_reason: Option<String>,
    project_root: PathBuf,
}
struct Slot {
    deleted: std::sync::atomic::AtomicBool,
    live: std::sync::atomic::AtomicBool,
    kill: Mutex<Option<tokio::sync::mpsc::UnboundedSender<()>>>,
    queue: Mutex<Queue>,
    session: Arc<AsyncMutex<Session>>,
    last_reset: Mutex<Option<String>>,
    last_restored: std::sync::atomic::AtomicBool,
}
struct Inner {
    slots: Mutex<HashMap<String, Arc<Slot>>>,
    startup: AsyncMutex<()>,
    backend: WorkerBackend,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SessionStatus {
    pub live: bool,
    pub running: bool,
    pub background: bool,
}

#[derive(Clone)]
pub struct KernelRegistry(Arc<Inner>);
impl Default for KernelRegistry {
    fn default() -> Self {
        Self::new(WorkerBackend::Process)
    }
}
impl KernelRegistry {
    pub fn new(backend: WorkerBackend) -> Self {
        Self(Arc::new(Inner {
            slots: Mutex::new(HashMap::new()),
            startup: AsyncMutex::new(()),
            backend,
        }))
    }
    pub fn reserve_turn(&self, chat_id: &str, tool_call_id: &str) -> TurnTicket {
        let slot = self
            .0
            .slots
            .lock()
            .unwrap()
            .entry(chat_id.into())
            .or_insert_with(|| {
                Arc::new(Slot {
                    deleted: std::sync::atomic::AtomicBool::new(false),
                    live: std::sync::atomic::AtomicBool::new(false),
                    kill: Mutex::new(None),
                    queue: Mutex::new(Queue {
                        active: None,
                        next: 0,
                        background: None,
                        waiting: VecDeque::new(),
                        used: Instant::now(),
                    }),
                    last_reset: Mutex::new(None),
                    last_restored: std::sync::atomic::AtomicBool::new(false),
                    session: Arc::new(AsyncMutex::new(Session {
                        client: None,
                        workspace: None,
                        max_state_bytes: 0,
                        last_run_id: String::new(),
                        taint: vec![],
                        reset_reason: None,
                        project_root: PathBuf::new(),
                    })),
                })
            })
            .clone();
        let (tx, rx) = oneshot::channel();
        let id;
        {
            let mut q = slot.queue.lock().unwrap();
            id = q.next;
            q.next += 1;
            if let Some(busy) = &q.background {
                let _ = tx.send(Err(busy.clone()));
            } else if q.active.is_none() {
                q.active = Some(id);
                let _ = tx.send(Ok(()));
            } else {
                q.waiting.push_back((id, tx));
            }
        }
        TurnTicket {
            registry: self.clone(),
            slot,
            chat_id: chat_id.into(),
            tool_call_id: tool_call_id.into(),
            id,
            rx: Some(rx),
            transferred: false,
        }
    }
    pub async fn reset_reason(&self, chat_id: &str) -> Option<String> {
        let slot = self.0.slots.lock().unwrap().get(chat_id).cloned()?;
        let reason = slot.last_reset.lock().unwrap().clone();
        reason
    }
    pub fn last_run_restored(&self, chat_id: &str) -> bool {
        self.0
            .slots
            .lock()
            .unwrap()
            .get(chat_id)
            .is_some_and(|slot| slot.last_restored.load(std::sync::atomic::Ordering::SeqCst))
    }
    pub fn session_status(&self, chat_id: &str) -> SessionStatus {
        let slot = self.0.slots.lock().unwrap().get(chat_id).cloned();
        let Some(slot) = slot else {
            return SessionStatus::default();
        };
        let q = slot.queue.lock().unwrap();
        let live = slot.live.load(std::sync::atomic::Ordering::SeqCst)
            && slot
                .kill
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|kill| !kill.is_closed());
        SessionStatus {
            live,
            running: q.active.is_some(),
            background: q.background.is_some(),
        }
    }

    #[cfg(test)]
    pub(crate) async fn lose_worker_for_test(&self, chat: &str) {
        let slot = self.0.slots.lock().unwrap().get(chat).cloned().unwrap();
        if let Some(client) = slot.session.lock().await.client.as_mut() {
            let _ = client.kill().await;
        };
    }
    pub async fn drop_chat(&self, chat_id: &str) {
        let slot = self.0.slots.lock().unwrap().remove(chat_id);
        let Some(slot) = slot else {
            return;
        };
        slot.deleted
            .store(true, std::sync::atomic::Ordering::SeqCst);
        slot.deleted
            .store(true, std::sync::atomic::Ordering::SeqCst);
        slot.live.store(false, std::sync::atomic::Ordering::SeqCst);
        if let Some(kill) = slot.kill.lock().unwrap().take() {
            let _ = kill.send(());
        }
        {
            let mut queue = slot.queue.lock().unwrap();
            queue.active = None;
            for (_, sender) in queue.waiting.drain(..) {
                let _ = sender.send(Err(PythonBusy {
                    job_id: "chat deleted".into(),
                    tool_call_id: String::new(),
                }));
            }
        }
        if let Ok(mut session) = slot.session.clone().try_lock_owned() {
            if let Some(mut client) = session.client.take() {
                let _ = client.kill().await;
            }
        }
    }
    pub async fn shutdown_all(&self) {
        let slots: Vec<_> = self.0.slots.lock().unwrap().values().cloned().collect();
        let shutdown = async {
            for slot in slots {
                slot.live.store(false, std::sync::atomic::Ordering::SeqCst);
                if let Some(kill) = slot.kill.lock().unwrap().take() {
                    let _ = kill.send(());
                }
                if let Ok(mut s) = slot.session.clone().try_lock_owned() {
                    if let Some(mut client) = s.client.take() {
                        let _ = client.kill().await;
                    }
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(5), shutdown).await;
    }
}
fn release(slot: &Slot, id: u64) {
    let mut q = slot.queue.lock().unwrap();
    if q.active != Some(id) {
        q.waiting.retain(|(n, _)| *n != id);
        return;
    }
    q.active = None;
    q.background = None;
    q.used = Instant::now();
    while let Some((next, tx)) = q.waiting.pop_front() {
        if tx.send(Ok(())).is_ok() {
            q.active = Some(next);
            break;
        }
    }
}
pub struct TurnTicket {
    registry: KernelRegistry,
    slot: Arc<Slot>,
    chat_id: String,
    tool_call_id: String,
    id: u64,
    rx: Option<oneshot::Receiver<Result<(), PythonBusy>>>,
    transferred: bool,
}
impl TurnTicket {
    pub async fn acquire(mut self) -> Result<SessionLease, PythonBusy> {
        self.rx.take().unwrap().await.unwrap_or_else(|_| {
            Err(PythonBusy {
                job_id: "unavailable".into(),
                tool_call_id: self.tool_call_id.clone(),
            })
        })?;
        let session = self.slot.session.clone().lock_owned().await;
        if self.slot.deleted.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(PythonBusy {
                job_id: "chat deleted".into(),
                tool_call_id: self.tool_call_id.clone(),
            });
        }
        let background = self.slot.queue.lock().unwrap().background.clone();
        if let Some(busy) = background {
            return Err(busy);
        }
        self.transferred = true;
        Ok(SessionLease {
            registry: self.registry.clone(),
            slot: self.slot.clone(),
            chat_id: self.chat_id.clone(),
            tool_call_id: self.tool_call_id.clone(),
            id: self.id,
            session: Some(session),
            feed_active: false,
        })
    }
}
impl Drop for TurnTicket {
    fn drop(&mut self) {
        if !self.transferred {
            release(&self.slot, self.id);
        }
    }
}
pub struct SessionLease {
    registry: KernelRegistry,
    slot: Arc<Slot>,
    chat_id: String,
    tool_call_id: String,
    id: u64,
    session: Option<OwnedMutexGuard<Session>>,
    feed_active: bool,
}
impl Drop for SessionLease {
    fn drop(&mut self) {
        if self.feed_active {
            if let Some(session) = self.session.as_mut() {
                session.client.take();
                session.reset_reason = Some("run exited before cleanup; session lost".into());
                *self.slot.last_reset.lock().unwrap() = session.reset_reason.clone();
                self.slot
                    .live
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }
        }
        self.session.take();
        release(&self.slot, self.id);
    }
}
impl SessionLease {
    pub fn remember_reset_reason(&self, reason: Option<String>) {
        *self.slot.last_reset.lock().unwrap() = reason;
    }
    pub fn remember_restored(&self, restored: bool) {
        self.slot
            .last_restored
            .store(restored, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn set_feed_active(&mut self, active: bool) {
        self.feed_active = active;
    }
    pub fn taint(&self) -> Vec<Value> {
        self.session.as_ref().unwrap().taint.clone()
    }
    pub fn mark_background(&self, job_id: &str) {
        let mut q = self.slot.queue.lock().unwrap();
        let busy = PythonBusy {
            job_id: job_id.into(),
            tool_call_id: self.tool_call_id.clone(),
        };
        q.background = Some(busy.clone());
        for (_, tx) in q.waiting.drain(..) {
            let _ = tx.send(Err(busy.clone()));
        }
    }
    pub fn client(&mut self) -> &mut KernelClient {
        self.session
            .as_mut()
            .unwrap()
            .client
            .as_mut()
            .expect("ensure_ready must succeed before client")
    }
    pub async fn ensure_ready(
        &mut self,
        app: &AppState,
        restart: bool,
    ) -> Result<SessionStart, String> {
        let root = crate::ext::hooks_runner::get_project_dir_string(app.clone()).await;
        self.ensure_ready_at(app, restart, PathBuf::from(root))
            .await
    }
    pub async fn ensure_ready_at(
        &mut self,
        app: &AppState,
        restart: bool,
        root: PathBuf,
    ) -> Result<SessionStart, String> {
        let workspace = PythonWorkspace::for_chat(app, &self.chat_id).await?;
        workspace.ensure_dirs().await?;
        let settings = crate::runtime_settings::coordination_for_project(&root).await?;
        let _startup = self.registry.0.startup.lock().await;
        let s = self.session.as_mut().unwrap();
        s.workspace = Some(workspace.clone());
        s.max_state_bytes = settings.python_max_state_bytes;
        s.project_root = root.clone();
        if restart {
            workspace.clear_state().await?;
            s.taint.clear();
            s.last_run_id.clear();
        }
        if s.client.is_none() {
            let mut idle: Vec<_> = self
                .registry
                .0
                .slots
                .lock()
                .unwrap()
                .values()
                .filter(|slot| !Arc::ptr_eq(slot, &self.slot))
                .cloned()
                .collect();
            idle.sort_by_key(|slot| slot.queue.lock().unwrap().used);
            let mut live = 0;
            let mut candidates = vec![];
            for slot in idle {
                if let Ok(session) = slot.session.clone().try_lock_owned() {
                    if session.client.is_some() {
                        live += 1;
                        if slot.queue.lock().unwrap().active.is_none() {
                            candidates.push((slot.clone(), session));
                        }
                    }
                } else if slot.live.load(std::sync::atomic::Ordering::SeqCst) {
                    live += 1;
                }
            }
            while live >= settings.python_max_live_sessions {
                if candidates.is_empty() {
                    return Err("Python live-session limit reached: every worker is busy".into());
                }
                let (victim_slot, mut victim) = candidates.remove(0);
                persist_session(&mut victim).await?;
                if let Some(mut client) = victim.client.take() {
                    let _ = client.kill().await;
                }
                victim_slot
                    .live
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                live -= 1;
            }
            let options = ClientOptions {
                memory_limit_bytes: settings.python_max_memory_bytes,
                memory_poll_ms: 250,
            };
            let client = match self.registry.0.backend {
                WorkerBackend::Process => KernelClient::spawn(
                    WorkerCommand {
                        program: std::env::current_exe().map_err(|e| e.to_string())?,
                        args: vec!["--python-worker".into()],
                        env: worker_env(),
                        cwd: Some(root),
                    },
                    options,
                ),
                WorkerBackend::InThread => KernelClient::spawn_in_thread(
                    Box::new(refact_python_kernel::hostfs::HostFs::new(Default::default())),
                    options,
                ),
            }
            .map_err(|e| e.to_string())?;
            *self.slot.kill.lock().unwrap() = client.kill_handle();
            s.client = Some(client);
            self.slot
                .live
                .store(true, std::sync::atomic::Ordering::SeqCst);
            if !matches!(
                event(s.client.as_mut().unwrap()).await?,
                KernelEvent::Ready { .. }
            ) {
                s.client.take();
                return Err("Python worker did not send Ready".into());
            }
            let mut start = SessionStart {
                restored: false,
                reset_reason: s.reset_reason.take(),
            };
            if !restart
                && (workspace.state_path().exists() || workspace.read_state_meta().await.is_some())
            {
                match workspace.verify_state().await {
                    Ok(state) => {
                        s.client
                            .as_ref()
                            .unwrap()
                            .send(KernelCommand::LoadSnapshot {
                                max_state_bytes: settings.python_max_state_bytes,
                                path: state.path.to_string_lossy().into(),
                                expected_sha256: state.sha256,
                            })
                            .await
                            .map_err(|e| e.to_string())?;
                        match event(s.client.as_mut().unwrap()).await {
                            Ok(KernelEvent::Loaded) => {
                                start.restored = true;
                                if let Some(meta) = workspace.read_state_meta().await {
                                    s.taint = meta.taint;
                                    s.last_run_id = meta.last_run_id;
                                }
                            }
                            other => {
                                start.reset_reason =
                                    Some(format!("snapshot load failed: {other:?}"))
                            }
                        }
                    }
                    Err(reason) => start.reset_reason = Some(reason.to_string()),
                }
                if !start.restored {
                    s.taint.clear();
                    s.last_run_id.clear();
                    workspace.clear_state().await?;
                    reset(s.client.as_mut().unwrap()).await?;
                }
            }
            *self.slot.last_reset.lock().unwrap() = start.reset_reason.clone();
            return Ok(start);
        }
        if restart {
            reset(s.client.as_mut().unwrap()).await?;
            s.taint.clear();
        }
        Ok(SessionStart {
            restored: false,
            reset_reason: s.reset_reason.take(),
        })
    }
    pub async fn persist(
        &mut self,
        _app: &AppState,
        run_id: &str,
        taint: Vec<Value>,
    ) -> Result<PersistOutcome, String> {
        let s = self.session.as_mut().unwrap();
        s.last_run_id = run_id.into();
        s.taint = taint;
        persist_session(s).await
    }
    /// The run that hit the limit reports `reason` itself; the next start restores the
    /// saved snapshot without repeating it.
    pub async fn discard_worker_for_rollback(&mut self, reason: &str) {
        self.discard_worker(reason).await;
        self.session.as_mut().unwrap().reset_reason = None;
    }
    pub async fn discard_worker(&mut self, reason: &str) {
        let s = self.session.as_mut().unwrap();
        if let Some(mut client) = s.client.take() {
            let _ = client.kill().await;
        }
        self.slot
            .live
            .store(false, std::sync::atomic::Ordering::SeqCst);
        s.reset_reason = Some(reason.into());
        *self.slot.last_reset.lock().unwrap() = s.reset_reason.clone();
    }
}
fn worker_env() -> HashMap<String, String> {
    let mut env = HashMap::new();
    if cfg!(windows) {
        for key in ["SystemRoot", "WINDIR", "SYSTEMDRIVE", "TEMP", "TMP"] {
            if let Ok(value) = std::env::var(key) {
                env.insert(key.into(), value);
            }
        }
    }
    env
}
async fn event(client: &mut KernelClient) -> Result<KernelEvent, String> {
    match tokio::time::timeout(Duration::from_secs(5), client.next_event())
        .await
        .map_err(|_| "Python worker response timed out")?
    {
        ClientEvent::Kernel(event) => Ok(event),
        other => Err(format!("Python worker failure: {other:?}")),
    }
}
async fn reset(client: &mut KernelClient) -> Result<(), String> {
    client
        .send(KernelCommand::Reset)
        .await
        .map_err(|e| e.to_string())?;
    match event(client).await? {
        KernelEvent::ResetDone => Ok(()),
        other => Err(format!("Python reset failed: {other:?}")),
    }
}
async fn persist_session(s: &mut Session) -> Result<PersistOutcome, String> {
    let workspace = s
        .workspace
        .as_ref()
        .ok_or("Python workspace not initialized")?;
    let client = s.client.as_mut().ok_or("Python worker not initialized")?;
    client
        .send(KernelCommand::Snapshot {
            path: workspace.state_path().to_string_lossy().into(),
        })
        .await
        .map_err(|e| e.to_string())?;
    let (bytes, sha256) = match event(client).await? {
        KernelEvent::SnapshotDone {
            bytes,
            sha256,
        } => (bytes, sha256),
        other => return Err(format!("Python snapshot failed: {other:?}")),
    };
    client
        .send(KernelCommand::Inspect)
        .await
        .map_err(|e| e.to_string())?;
    let (variables, variables_truncated, variables_total) = match event(client).await? {
        KernelEvent::Variables {
            items,
            truncated,
            total,
            ..
        } => (items, truncated, total),
        other => return Err(format!("Python inspection failed: {other:?}")),
    };
    let outcome = workspace
        .record_snapshot(
            sha256,
            bytes,
            s.last_run_id.clone(),
            variables,
            s.taint.clone(),
            s.max_state_bytes,
        )
        .await?;
    workspace
        .set_variables_info(variables_truncated, variables_total)
        .await?;
    Ok(outcome)
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
