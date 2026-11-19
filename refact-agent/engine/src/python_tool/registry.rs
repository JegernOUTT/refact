use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use refact_python_kernel::client::{ClientEvent, ClientOptions, KernelClient, WorkerCommand};
use refact_python_kernel::protocol::{KernelCommand, KernelEvent, VariableSize};
use serde_json::Value;
use tokio::sync::{oneshot, Mutex as AsyncMutex, OwnedMutexGuard};

use crate::app_state::AppState;
use super::workspace::{
    interrupted, pause_point, NewState, PersistOutcome, Pin, PinnedState, PythonWorkspace,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PythonBusy {
    pub job_id: String,
    pub tool_call_id: String,
}
impl std::fmt::Display for PythonBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Python job {} (tool call {}) is still running; its result arrives in this chat by itself and starts your next turn, so do not poll for it. Use tool_job only to cancel it",
            self.job_id, self.tool_call_id
        )
    }
}
impl std::error::Error for PythonBusy {}

#[derive(Debug)]
pub struct SessionStart {
    pub restored: bool,
    pub reset_reason: Option<String>,
    /// What this start tells about the state that is not a reset: variables a fork did not
    /// copy, a closed session's unsaved changes, a saved state that could not be read.
    pub notes: Vec<String>,
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
    last_bytes: Option<u64>,
    /// A run reached the worker since the last save.
    dirty: bool,
    /// State size at the last large-state note.
    noted_bytes: Option<u64>,
    /// Notes the next start reports.
    notes: Vec<String>,
    /// The worker started empty because the saved state could not be read: it serves one run
    /// and is never saved over that state.
    keep_saved: bool,
}
impl Session {
    /// The fields that belong to the generation the worker was loaded from.
    fn forget_generation(&mut self) {
        self.taint.clear();
        self.last_run_id.clear();
        self.last_bytes = None;
        self.noted_bytes = None;
    }
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
                        last_bytes: None,
                        dirty: false,
                        noted_bytes: None,
                        notes: vec![],
                        keep_saved: false,
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
    pub fn has_worker(&self) -> bool {
        self.session.as_ref().unwrap().client.is_some()
    }
    /// A note the next start of this session reports; it is not a reset.
    pub fn note(&mut self, note: String) {
        self.session.as_mut().unwrap().notes.push(note);
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
        let s = self.session.as_mut().unwrap();
        s.dirty = true;
        s.client
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

    /// The engine-wide start lock covers only counting live sessions, eviction and the spawn;
    /// the new worker's Ready wait and restore run under this chat's own slot.
    pub async fn ensure_ready_at(
        &mut self,
        app: &AppState,
        restart: bool,
        root: PathBuf,
    ) -> Result<SessionStart, String> {
        let workspace = PythonWorkspace::for_chat(app, &self.chat_id).await?;
        workspace.ensure_dirs().await?;
        let settings = crate::runtime_settings::coordination_for_project(&root).await?;
        let s = self.session.as_mut().unwrap();
        s.workspace = Some(workspace.clone());
        s.max_state_bytes = settings.python_max_state_bytes;
        s.project_root = root.clone();
        if restart {
            workspace.clear_state().await?;
            s.forget_generation();
            s.dirty = false;
        }
        if s.keep_saved {
            retire(s, &self.slot).await;
        }
        if s.client.is_some() {
            if restart {
                reset(s.client.as_mut().unwrap()).await?;
            }
            return Ok(SessionStart {
                restored: false,
                reset_reason: s.reset_reason.take(),
                notes: std::mem::take(&mut s.notes),
            });
        }
        let backend = self.registry.0.backend;
        let spawn = || spawn_worker(backend, &root, settings.python_max_memory_bytes);
        {
            let _startup = self.registry.0.startup.lock().await;
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
                if victim.dirty {
                    let unsaved = match persist_session(&mut victim, &victim_slot).await {
                        Err(reason) => Some(reason),
                        Ok(outcome) if outcome.not_saved.is_some() => {
                            Some(over_limit(outcome.bytes, victim.max_state_bytes))
                        }
                        Ok(_) => None,
                    };
                    if let Some(reason) = unsaved {
                        let fallback = match &victim.workspace {
                            Some(saved)
                                if matches!(saved.pin(false, false).await, Pin::Pinned(_)) =>
                            {
                                "variables come from the last successful save"
                            }
                            _ => "the session starts empty",
                        };
                        victim.notes.push(format!(
                            "the session was closed to free a worker and its last changes were not saved ({reason}); {fallback}"
                        ));
                    }
                }
                if let Some(mut client) = victim.client.take() {
                    let _ = client.kill().await;
                }
                victim_slot
                    .live
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                live -= 1;
            }
            let client = spawn()?;
            *self.slot.kill.lock().unwrap() = client.kill_handle();
            s.client = Some(client);
            s.dirty = false;
            s.keep_saved = false;
            self.slot
                .live
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let (restored, note) = match start_worker(s, &self.slot, &workspace, restart, &spawn).await
        {
            Ok(started) => started,
            Err(e) => {
                if let Some(mut client) = s.client.take() {
                    let _ = client.kill().await;
                }
                self.slot
                    .live
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                return Err(e);
            }
        };
        let start = SessionStart {
            restored,
            reset_reason: s.reset_reason.take(),
            notes: std::mem::take(&mut s.notes)
                .into_iter()
                .chain(note)
                .collect(),
        };
        *self.slot.last_reset.lock().unwrap() = start.reset_reason.clone();
        Ok(start)
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
        persist_session(s, &self.slot).await
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
fn spawn_worker(
    backend: WorkerBackend,
    root: &Path,
    memory_limit_bytes: u64,
) -> Result<KernelClient, String> {
    let options = ClientOptions {
        memory_limit_bytes,
        memory_poll_ms: 250,
    };
    match backend {
        WorkerBackend::Process => KernelClient::spawn(
            WorkerCommand {
                program: std::env::current_exe().map_err(|e| e.to_string())?,
                args: vec![crate::self_exec::PYTHON_WORKER_ARG.into()],
                env: worker_env(),
                cwd: Some(root.to_path_buf()),
            },
            options,
        ),
        WorkerBackend::InThread => KernelClient::spawn_in_thread(
            Box::new(refact_python_kernel::hostfs::HostFs::new(Default::default())),
            options,
        ),
    }
    .map_err(|e| e.to_string())
}
async fn event(client: &mut KernelClient) -> Result<KernelEvent, String> {
    reply(client, Duration::from_secs(5)).await
}
async fn reply(client: &mut KernelClient, wait: Duration) -> Result<KernelEvent, String> {
    match tokio::time::timeout(wait, client.next_event())
        .await
        .map_err(|_| format!("Python worker gave no answer within {} s", wait.as_secs()))?
    {
        ClientEvent::Kernel(event) => Ok(event),
        other => Err(format!("Python worker failure: {other:?}")),
    }
}
async fn ready(client: &mut KernelClient, dir: &Path) -> Result<(), String> {
    if interrupted(dir, "ready_timeout") {
        return Err("Python worker gave no answer within 5 s".into());
    }
    match event(client).await? {
        KernelEvent::Ready {
            ..
        } => Ok(()),
        _ => Err("Python worker did not send Ready".into()),
    }
}
/// Writing or reading a whole snapshot gets time in proportion to its size: 100 ms per MiB,
/// at least 10 s and at most 2 minutes; an unknown size gets the most.
fn snapshot_timeout(bytes: Option<u64>) -> Duration {
    bytes.map_or(Duration::from_secs(120), |bytes| {
        Duration::from_millis((bytes >> 20).saturating_mul(100))
            .clamp(Duration::from_secs(10), Duration::from_secs(120))
    })
}
/// Waits for a new worker and loads the published state into it, so its taint and run belong
/// to exactly the bytes that were loaded. Returns whether the state was restored and, when it
/// could not be read, the note that it is kept for the next run.
async fn start_worker(
    s: &mut Session,
    slot: &Slot,
    workspace: &PythonWorkspace,
    restart: bool,
    spawn: &(dyn Fn() -> Result<KernelClient, String> + Sync),
) -> Result<(bool, Option<String>), String> {
    let client = s.client.as_mut().ok_or("Python worker not initialized")?;
    ready(client, workspace.dir()).await?;
    if restart {
        return Ok((false, None));
    }
    for _ in 0..3 {
        let pinned = match workspace.pin(true, true).await {
            Pin::Nothing => {
                s.forget_generation();
                return Ok((false, None));
            }
            Pin::Unavailable(reason) => return Ok((false, Some(keep_saved(s, &reason)))),
            Pin::Invalid(reason, manifest) => {
                if invalidate(s, workspace, reason.0, &manifest).await? {
                    return Ok((false, None));
                }
                continue;
            }
            Pin::Pinned(pinned) => pinned,
        };
        pause_point(workspace.dir(), "restore_pinned").await;
        let client = s.client.as_mut().ok_or("Python worker not initialized")?;
        match load(client, &pinned, s.max_state_bytes).await {
            Load::Loaded => {
                s.taint = pinned.meta.taint;
                s.last_run_id = pinned.meta.last_run_id;
                s.last_bytes = Some(pinned.meta.bytes);
                return Ok((true, None));
            }
            Load::Refused(message) => match tokio::fs::symlink_metadata(&pinned.path).await {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let note = keep_saved(s, "its snapshot vanished while it was read");
                    return Ok((false, Some(note)));
                }
                Err(e) => {
                    let note = keep_saved(s, &format!("its snapshot is unreadable: {e}"));
                    return Ok((false, Some(note)));
                }
                Ok(_) => {
                    let reason = format!("snapshot load failed: {message}");
                    if invalidate(s, workspace, reason, &pinned.manifest).await? {
                        return Ok((false, None));
                    }
                }
            },
            Load::WorkerFailed(reason) => {
                if let Some(mut client) = s.client.take() {
                    let _ = client.kill().await;
                }
                let client = spawn()?;
                *slot.kill.lock().unwrap() = client.kill_handle();
                ready(s.client.insert(client), workspace.dir()).await?;
                return Ok((false, Some(keep_saved(s, &reason))));
            }
        }
    }
    let note = keep_saved(s, "the saved state changed while it was read");
    Ok((false, Some(note)))
}
enum Load {
    Loaded,
    Refused(String),
    WorkerFailed(String),
}
async fn load(client: &mut KernelClient, pinned: &PinnedState, max_state_bytes: u64) -> Load {
    if let Err(e) = client
        .send(KernelCommand::LoadSnapshot {
            max_state_bytes,
            path: pinned.path.to_string_lossy().into(),
            expected_sha256: pinned.meta.sha256.clone(),
        })
        .await
    {
        return Load::WorkerFailed(e.to_string());
    }
    let wait = snapshot_timeout(Some(pinned.meta.bytes));
    match tokio::time::timeout(wait, client.next_event()).await {
        Ok(ClientEvent::Kernel(KernelEvent::Loaded)) => Load::Loaded,
        Ok(ClientEvent::Kernel(KernelEvent::LoadFailed {
            message,
        })) => Load::Refused(message),
        Ok(other) => Load::WorkerFailed(format!("{other:?}")),
        Err(_) => Load::WorkerFailed(format!(
            "the worker gave no answer within {} s",
            wait.as_secs()
        )),
    }
}
/// The worker serves this run empty; the state it could not read stays for the next start.
fn keep_saved(s: &mut Session, reason: &str) -> String {
    s.keep_saved = true;
    s.forget_generation();
    format!("saved state could not be read ({reason}); it is kept for the next run")
}
/// Clears an invalid state while `manifest` is still the current one; false when it changed
/// meanwhile. The reason is stored first, so it is reported even when clearing fails.
async fn invalidate(
    s: &mut Session,
    workspace: &PythonWorkspace,
    reason: String,
    manifest: &[u8],
) -> Result<bool, String> {
    let pending = s.reset_reason.replace(reason);
    pause_point(workspace.dir(), "before_clear").await;
    let cleared = if interrupted(workspace.dir(), "clear_state") {
        Err("Cannot clear the Python state: injected failure".into())
    } else {
        workspace.clear_if_current(manifest).await
    };
    if !cleared? {
        s.reset_reason = pending;
        return Ok(false);
    }
    s.forget_generation();
    reset(s.client.as_mut().ok_or("Python worker not initialized")?).await?;
    Ok(true)
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
/// Kills the worker and waits briefly for it to exit, so nothing it still writes outlives it.
async fn retire(s: &mut Session, slot: &Slot) {
    if let Some(mut client) = s.client.take() {
        let _ = client.kill().await;
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while !matches!(client.next_event().await, ClientEvent::Exited { .. }) {}
        })
        .await;
    }
    slot.live.store(false, std::sync::atomic::Ordering::SeqCst);
}
/// A save that fails or gets no answer in time retires the worker, so a late answer can never
/// be read as the reply to a later command, and removes the generation it was writing; the next
/// start loads the last published state. A worker that started empty is never saved.
async fn persist_session(s: &mut Session, slot: &Slot) -> Result<PersistOutcome, String> {
    if s.keep_saved {
        retire(s, slot).await;
        return Ok(PersistOutcome::default());
    }
    let workspace = s
        .workspace
        .clone()
        .ok_or("Python workspace not initialized")?;
    let generation = workspace.new_generation_path();
    let saved = save(s, slot, &workspace, &generation).await;
    if saved.is_err() {
        retire(s, slot).await;
        workspace.discard_generation(&generation).await;
    }
    saved
}
async fn save(
    s: &mut Session,
    slot: &Slot,
    workspace: &PythonWorkspace,
    generation: &Path,
) -> Result<PersistOutcome, String> {
    let client = s.client.as_mut().ok_or("Python worker not initialized")?;
    client
        .send(KernelCommand::Snapshot {
            path: generation.to_string_lossy().into(),
        })
        .await
        .map_err(|e| e.to_string())?;
    let estimate = [s.last_bytes, client.rss_bytes()]
        .into_iter()
        .flatten()
        .max();
    let wait = snapshot_timeout(estimate);
    let answer = if interrupted(workspace.dir(), "snapshot_timeout") {
        Err(format!(
            "Python worker gave no answer within {} s",
            wait.as_secs()
        ))
    } else {
        reply(client, wait).await
    };
    let (bytes, sha256) = match answer? {
        KernelEvent::SnapshotDone {
            bytes,
            sha256,
        } => (bytes, sha256),
        other => return Err(format!("Python snapshot failed: {other:?}")),
    };
    if interrupted(workspace.dir(), "snapshot_done") {
        return Err("interrupted".into());
    }
    if bytes > s.max_state_bytes {
        workspace.discard_generation(generation).await;
        return Ok(PersistOutcome {
            persisted: false,
            bytes,
            not_saved: Some(format!(
                "not saved ({}); variables stay in memory until this session is restarted or evicted; set large variables to None and the next run saves again",
                over_limit(bytes, s.max_state_bytes)
            )),
            ..Default::default()
        });
    }
    if interrupted(workspace.dir(), "generation_cut") {
        std::fs::OpenOptions::new()
            .write(true)
            .open(generation)
            .and_then(|file| file.set_len(bytes / 2))
            .map_err(|e| e.to_string())?;
    }
    let client = s.client.as_mut().ok_or("Python worker not initialized")?;
    client
        .send(KernelCommand::Inspect)
        .await
        .map_err(|e| e.to_string())?;
    let (variables, variables_truncated, variables_total, variables_unknown, largest, unmeasured) =
        match event(client).await? {
            KernelEvent::Variables {
                items,
                truncated,
                total,
                unknown,
                largest,
                unmeasured,
                ..
            } => (items, truncated, total, unknown, largest, unmeasured),
            other => return Err(format!("Python inspection failed: {other:?}")),
        };
    if interrupted(workspace.dir(), "inspected") {
        return Err("interrupted".into());
    }
    if slot.deleted.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("chat deleted".into());
    }
    let outcome = workspace
        .record_snapshot(
            NewState {
                generation: generation.to_path_buf(),
                sha256,
                bytes,
                last_run_id: s.last_run_id.clone(),
                taint: s.taint.clone(),
                variables,
                variables_truncated,
                variables_total,
                variables_unknown,
            },
            s.max_state_bytes,
        )
        .await?;
    s.last_bytes = Some(bytes);
    s.dirty = false;
    if bytes < LARGE_STATE_BYTES {
        s.noted_bytes = None;
    }
    let due = bytes >= LARGE_STATE_BYTES
        && s.noted_bytes
            .is_none_or(|noted| bytes as f64 >= noted as f64 * 1.5);
    if due {
        s.noted_bytes = Some(bytes);
    }
    Ok(PersistOutcome {
        note: due.then(|| large_state_note(bytes, variables_unknown, &largest, unmeasured)),
        ..outcome
    })
}

/// From this size on a run tells what holds the state, again each time it grows by half.
const LARGE_STATE_BYTES: u64 = 16 * 1024 * 1024;
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}
fn over_limit(bytes: u64, max_state_bytes: u64) -> String {
    format!(
        "state is {}, over the limit {}",
        megabytes(bytes),
        megabytes(max_state_bytes)
    )
}
/// `unmeasured_total` counts every variable whose size is unknown, also those `largest` leaves out.
fn large_state_note(
    bytes: u64,
    unknown: bool,
    largest: &[VariableSize],
    unmeasured_total: usize,
) -> String {
    let mut note = format!("the session holds {} of variables", megabytes(bytes));
    if unknown {
        note.push_str("; they could not be measured");
    }
    let measured: Vec<String> = largest
        .iter()
        .filter(|v| v.approx_bytes.is_some_and(|bytes| bytes >= 1_000_000))
        .filter_map(|v| {
            let size = megabytes(v.approx_bytes?);
            Some(match v.len {
                Some(len) => format!("{} {}[{len}] ~{size}", v.name, v.type_name),
                None => format!("{} {} ~{size}", v.name, v.type_name),
            })
        })
        .collect();
    if !measured.is_empty() {
        note.push_str(&format!(". Largest: {}", measured.join(", ")));
    }
    let unmeasured: Vec<String> = largest
        .iter()
        .filter(|v| v.approx_bytes.is_none())
        .map(|v| format!("{} ({})", v.name, v.type_name))
        .collect();
    let more = unmeasured_total.saturating_sub(unmeasured.len());
    if !unmeasured.is_empty() {
        note.push_str(&format!("; size unknown: {}", unmeasured.join(", ")));
        if more > 0 {
            note.push_str(&format!(" and {more} more of unknown size"));
        }
    } else if more > 0 {
        note.push_str(&format!("; {more} variables of unknown size"));
    }
    note.push_str(". Set variables you no longer need to None (del is not supported), or start over with restart=true.");
    note
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
