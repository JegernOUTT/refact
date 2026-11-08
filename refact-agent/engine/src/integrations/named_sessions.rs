//! Named sessions are deliberately absent from the default runtime map and owner index.
//! A launch worker owns its quota slot through publication or verified cleanup. Closing
//! invalidates execution first; an unverifiable exit retains both the slot and profile.
use super::*;
#[path = "named_profiles.rs"]
mod named_profiles;
use named_profiles::OwnedProfile;
use serde::Serialize;
use uuid::Uuid;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::global_context::GlobalContext;

#[derive(Clone)]
pub struct NamedSessionOwner {
    gcx: Weak<GlobalContext>,
    key: OwnerKey,
    cache_dir: PathBuf,
}
#[derive(Clone, Hash, PartialEq, Eq)]
struct OwnerKey {
    app: usize,
    chat: String,
    roots: Vec<PathBuf>,
}
impl NamedSessionOwner {
    pub fn from_context(ccx: &AtCommandsContext) -> Result<Self, String> {
        if ccx.chat_id.is_empty() {
            return Err("Named sessions require an exact chat owner".into());
        }
        let mut roots = ccx
            .execution_scope
            .as_ref()
            .map(|s| s.effective_project_dirs())
            .unwrap_or_else(|| workspace_roots(&ccx.app));
        roots.sort();
        roots.dedup();
        if roots.is_empty() {
            return Err("Named sessions require a trusted project root".into());
        }
        Ok(Self {
            gcx: Arc::downgrade(&ccx.app.gcx),
            key: OwnerKey {
                app: Arc::as_ptr(&ccx.app.gcx) as usize,
                chat: ccx.chat_id.clone(),
                roots,
            },
            cache_dir: ccx.app.gcx.cache_dir.clone(),
        })
    }
    fn check(&self) -> Result<(), String> {
        let gcx = self
            .gcx
            .upgrade()
            .ok_or("Named session application has stopped")?;
        if gcx.shutdown_flag.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Named session application is shutting down".into());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Debug)]
pub struct NamedSessionInfo {
    pub name: String,
    pub session_id: String,
    pub runtime_id: Option<String>,
    pub state: String,
}
#[derive(Clone)]
// A session UUID maps to exactly one runtime generation for its entire lifetime.
// There is deliberately no replacement or default-recovery API under this handle.
pub struct NamedRuntime {
    pub runtime: Arc<AMutex<BrowserRuntime>>,
    pub ownership: Arc<RuntimeOwnership>,
    pub session_id: String,
    pub runtime_id: String,
    // Registry copies are unleased; resolved request clones share this one lease.
    lease: Option<Arc<RequestLease>>,
}
struct RequestLease {
    reg: Registry,
    key: Key,
    id: String,
}
impl Drop for RequestLease {
    fn drop(&mut self) {
        let mut map = locked(&self.reg);
        if let Some(e) = map
            .get_mut(&self.key)
            .filter(|e| e.info.session_id == self.id)
        {
            e.active_requests = e.active_requests.saturating_sub(1);
            e.touched = Instant::now();
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NamedLifecycleStage {
    BeforeDispatch,
    AfterDispatchUnknown,
    AfterDispatchCleanupKnown,
}
#[derive(Debug, Serialize)]
pub struct NamedLifecycleError {
    pub stage: NamedLifecycleStage,
    pub session_id: Option<String>,
    pub message: String,
}
impl NamedLifecycleError {
    fn after(message: impl Into<String>, id: &str, known: bool) -> Self {
        Self {
            stage: if known {
                NamedLifecycleStage::AfterDispatchCleanupKnown
            } else {
                NamedLifecycleStage::AfterDispatchUnknown
            },
            session_id: Some(id.into()),
            message: message.into(),
        }
    }
}
impl From<String> for NamedLifecycleError {
    fn from(message: String) -> Self {
        Self {
            stage: NamedLifecycleStage::BeforeDispatch,
            session_id: None,
            message,
        }
    }
}
impl From<&str> for NamedLifecycleError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}
impl std::fmt::Display for NamedLifecycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl From<NamedLifecycleError> for String {
    fn from(error: NamedLifecycleError) -> Self {
        error.to_string()
    }
}
#[derive(Serialize, Debug)]
pub struct NamedSessionClose {
    pub name: String,
    pub session_id: String,
    pub runtime_id: Option<String>,
    pub closed_target_ids: Vec<String>,
    pub targets_complete: bool,
    pub process_exited: bool,
    pub profile_disposition: String,
}
struct Entry {
    owner: NamedSessionOwner,
    info: NamedSessionInfo,
    runtime: Option<NamedRuntime>,
    owned_process: Option<headless_chrome::browser::OwnedProcess>,
    failed_processes: Vec<headless_chrome::browser::OwnedProcess>,
    touched: Instant,
    active_requests: usize,
    quarantine: Option<OwnedProfile>,
    closing: bool,
    cleaning: bool,
}
type Key = (OwnerKey, String);
type Registry = Arc<StdMutex<HashMap<Key, Entry>>>;
static NAMED: OnceLock<Registry> = OnceLock::new();
struct Worker {
    owner: Weak<GlobalContext>,
    id: String,
    task: tokio::task::JoinHandle<()>,
}
static WORKERS: OnceLock<StdMutex<Vec<Worker>>> = OnceLock::new();
fn spawn_owned(owner: Weak<GlobalContext>, id: String, work: impl FnOnce() + Send + 'static) {
    // Register while holding the same lock used by shutdown's pending inspection.
    let mut workers = WORKERS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    workers.push(Worker {
        owner,
        id,
        task: tokio::task::spawn_blocking(work),
    });
}
async fn reap_workers(owner: &Weak<GlobalContext>, all: bool) {
    loop {
        let (finished, pending) = {
            let mut workers = WORKERS
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut done = Vec::new();
            let mut i = 0;
            while i < workers.len() {
                if Weak::ptr_eq(&workers[i].owner, owner) && workers[i].task.is_finished() {
                    done.push(workers.swap_remove(i));
                } else {
                    i += 1;
                }
            }
            (
                done,
                workers
                    .iter()
                    .filter(|w| Weak::ptr_eq(&w.owner, owner))
                    .count(),
            )
        };
        // Only finished handles leave the retained collection. Cancellation during
        // shutdown never loses a still-running launch or cleanup worker.
        for worker in finished {
            if let Err(error) = worker.task.await {
                warn!(
                    "Named browser lifecycle worker {} failed: {error}",
                    worker.id
                );
            }
        }
        if !all || pending == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
fn registry() -> Registry {
    NAMED.get_or_init(Default::default).clone()
}
fn locked(reg: &Registry) -> std::sync::MutexGuard<'_, HashMap<Key, Entry>> {
    reg.lock().unwrap_or_else(|e| e.into_inner())
}
use refact_browser::sessions::valid_name;
fn mark_closing(entry: &mut Entry) {
    entry.closing = true;
    entry.info.state = "closing".into();
    if let Some(rt) = &entry.runtime {
        rt.ownership.invalidate();
    }
}
// This guard and the worker serialize on the registry lock: cancellation cannot leave
// a late, usable publication. A closing entry remains quota-bearing until cleanup proves exit.
struct CreateGuard {
    reg: Registry,
    key: Key,
    id: String,
    armed: bool,
}
impl Drop for CreateGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        {
            let mut map = locked(&self.reg);
            if let Some(e) = map
                .get_mut(&self.key)
                .filter(|e| e.info.session_id == self.id)
            {
                mark_closing(e);
            }
        }
        schedule_cleanup(self.reg.clone(), self.key.clone(), self.id.clone());
    }
}
fn schedule_cleanup(reg: Registry, key: Key, id: String) {
    let mut workers = WORKERS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let owner = {
        let mut map = locked(&reg);
        match map.get_mut(&key).filter(|e| e.info.session_id == id) {
            Some(e) if e.runtime.is_some() && !e.cleaning => {
                e.cleaning = true;
                Some(e.owner.gcx.clone())
            }
            _ => None,
        }
    };
    if let Some(owner) = owner {
        let worker_id = id.clone();
        let task = tokio::task::spawn_blocking(move || {
            cleanup(&reg, &key, &id);
        });
        workers.push(Worker {
            owner,
            id: worker_id,
            task,
        });
    }
}

pub async fn create_named_session(
    owner: NamedSessionOwner,
    name: &str,
    options: BrowserLaunchOptions,
    deadline: Instant,
) -> Result<NamedSessionInfo, NamedLifecycleError> {
    create_named_session_with_launch(owner, name, options, deadline, BrowserRuntime::launch).await
}
async fn create_named_session_with_launch(
    owner: NamedSessionOwner,
    name: &str,
    options: BrowserLaunchOptions,
    deadline: Instant,
    launch: impl FnOnce(PathBuf, BrowserLaunchOptions) -> Result<BrowserRuntime, String>
        + Send
        + 'static,
) -> Result<NamedSessionInfo, NamedLifecycleError> {
    owner.check()?;
    check_deadline(deadline)?;
    reap_workers(&owner.gcx, false).await;
    if !valid_name(name) {
        return Err("Invalid named session identifier".into());
    }
    if owner.cache_dir.to_str().is_none() {
        return Err("Named browser profiles require a UTF-8 cache path".into());
    }
    if !options.extra_args.is_empty() || options.downloads_dir.is_some() {
        return Err(
            "Named sessions reject custom Chrome arguments and downloads directories".into(),
        );
    }
    let reg = registry();
    let key = (owner.key.clone(), name.to_string());
    let id = Uuid::new_v4().to_string();
    let (mut cancel, rx) = {
        // Shutdown cannot observe the reservation without also seeing its retained worker.
        let mut workers = WORKERS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            owner.check()?;
            let mut map = locked(&reg);
            if map.contains_key(&key) {
                return Err("Named session already exists or is closing".into());
            }
            if map.len() >= 32 || map.keys().filter(|(o, _)| o == &owner.key).count() >= 4 {
                return Err("Named session quota exceeded (4 per owner, 32 global)".into());
            }
            map.insert(
                key.clone(),
                Entry {
                    owner: owner.clone(),
                    info: NamedSessionInfo {
                        name: name.into(),
                        session_id: id.clone(),
                        runtime_id: None,
                        state: "creating".into(),
                    },
                    runtime: None,
                    owned_process: None,
                    failed_processes: Vec::new(),
                    touched: Instant::now(),
                    active_requests: 0,
                    quarantine: None,
                    closing: false,
                    cleaning: false,
                },
            );
        }
        let cancel = CreateGuard {
            reg: reg.clone(),
            key: key.clone(),
            id: id.clone(),
            armed: true,
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let worker_owner = owner.gcx.clone();
        let worker_id = id.clone();
        let task = tokio::task::spawn_blocking(move || {
            let _deadline =
                headless_chrome::browser::transport::CommandDeadlineGuard::new(deadline);
            let profile = match OwnedProfile::create(&owner.cache_dir) {
                Ok(profile) => profile,
                Err(error) => {
                    locked(&reg).remove(&key);
                    let _ = tx.send(Err(NamedLifecycleError::after(error, &id, true)));
                    return;
                }
            };
            let tracker = headless_chrome::browser::OwnedLaunchTracker::new();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                launch(profile.path().to_path_buf(), options)
            }))
            .unwrap_or_else(|_| {
                Err("Named browser launch worker panicked; original child cleanup retained".into())
            });
            let failed_processes = if result.is_err() {
                tracker.processes()
            } else {
                Vec::new()
            };
            drop(tracker);
            let failed_cleanup_known = result.is_err()
                && failed_processes
                    .iter()
                    .all(|child| child.terminate_before(deadline).unwrap_or(false));
            let failed_profile_removed =
                failed_cleanup_known && profile.cleanup(true, true).is_ok();
            let mut map = locked(&reg);
            match result {
                Err(error) => {
                    if failed_profile_removed {
                        map.remove(&key);
                    } else if let Some(e) = map.get_mut(&key) {
                        mark_closing(e);
                        e.quarantine = Some(profile);
                        e.failed_processes = failed_processes;
                        e.info.state = "quarantined".into();
                    }
                    let _ = tx.send(Err(NamedLifecycleError::after(
                        error,
                        &id,
                        failed_profile_removed,
                    )));
                }
                Ok(mut runtime) => {
                    runtime.set_allowed_roots(owner.key.roots.clone());
                    runtime.reattach(&owner.key.chat);
                    let owned_process = runtime.browser.owned_process();
                    let rt = NamedRuntime {
                        runtime_id: runtime.runtime_id.clone(),
                        session_id: id.clone(),
                        ownership: runtime.ownership.clone(),
                        runtime: Arc::new(AMutex::new(runtime)),
                        lease: None,
                    };
                    let e = map.get_mut(&key).expect("launch reservation retained");
                    e.quarantine = Some(profile);
                    e.owned_process = owned_process;
                    e.info.runtime_id = Some(rt.runtime_id.clone());
                    e.runtime = Some(rt);
                    e.info.state = "ready".into();
                    if e.closing
                        || owner.check().is_err()
                        || Instant::now() >= deadline
                        || tx.is_closed()
                    {
                        mark_closing(e);
                        e.cleaning = true;
                        drop(map);
                        cleanup(&reg, &key, &id);
                    } else {
                        let info = e.info.clone();
                        if tx.send(Ok(info)).is_err() {
                            mark_closing(e);
                            e.cleaning = true;
                            drop(map);
                            cleanup(&reg, &key, &id);
                        }
                    }
                }
            }
        });
        workers.push(Worker {
            owner: worker_owner,
            id: worker_id,
            task,
        });
        drop(workers);
        (cancel, rx)
    };
    let result = before_deadline(deadline, rx)
        .await
        .map_err(|e| NamedLifecycleError::after(e, &cancel.id, false))?
        .map_err(|_| {
            NamedLifecycleError::after("Named session launch worker stopped", &cancel.id, false)
        })?;
    if result.is_ok() {
        cancel.armed = false;
    }
    result
}
pub fn list_named_sessions(owner: &NamedSessionOwner) -> Result<Vec<NamedSessionInfo>, String> {
    owner.check()?;
    let reg = registry();
    let map = locked(&reg);
    let mut rows: Vec<_> = map
        .iter()
        .filter(|((o, _), e)| o == &owner.key && Weak::ptr_eq(&e.owner.gcx, &owner.gcx))
        .map(|(_, e)| e.info.clone())
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}
pub fn resolve_named_session(
    owner: &NamedSessionOwner,
    name: &str,
    expected_session_id: &str,
) -> Result<NamedRuntime, String> {
    owner.check()?;
    let reg = registry();
    let mut map = locked(&reg);
    let e = map
        .get_mut(&(owner.key.clone(), name.into()))
        .filter(|e| {
            Weak::ptr_eq(&e.owner.gcx, &owner.gcx)
                && e.info.session_id == expected_session_id
                && !e.closing
        })
        .ok_or("Named session not found, closing, or session UUID mismatch")?;
    let mut rt = e
        .runtime
        .as_ref()
        .filter(|rt| rt.ownership.is_current_generation())
        .ok_or("Named session not ready")?
        .clone();
    e.active_requests += 1;
    rt.lease = Some(Arc::new(RequestLease {
        reg: reg.clone(),
        key: (owner.key.clone(), name.into()),
        id: expected_session_id.into(),
    }));
    e.touched = Instant::now();
    Ok(rt)
}
fn cleanup(reg: &Registry, key: &Key, id: &str) -> Option<NamedSessionClose> {
    let _deadline = headless_chrome::browser::transport::CommandDeadlineGuard::new(
        Instant::now() + Duration::from_secs(5),
    );
    let (rt, owned_process, info, profile) = {
        let mut map = locked(reg);
        let e = map.get_mut(key).filter(|e| e.info.session_id == id)?;
        mark_closing(e);
        (
            e.runtime.clone()?,
            e.owned_process.clone(),
            e.info.clone(),
            e.quarantine.clone(),
        )
    };
    let inventory = rt.runtime.try_lock().ok().map(|r| {
        r.list_tab_infos()
            .into_iter()
            .map(|t| t.id)
            .collect::<Vec<_>>()
    });
    let targets_complete = inventory.is_some();
    let targets = inventory.unwrap_or_default();
    // Only the original owned Child handle can authorize termination and prove exit.
    // Missing handles, errors and timeouts retain quota for later inspection/retry.
    let exited = owned_process.as_ref().is_some_and(|child| {
        match child.terminate_before(Instant::now() + Duration::from_secs(5)) {
            Ok(exited) => exited,
            Err(error) => {
                warn!("Named browser child cleanup failed: {error}");
                false
            }
        }
    });
    let disposition = match profile {
        Some(profile) if exited => {
            let safe = matches!(
                profile_lock_state(profile.path()),
                ProfileLockState::Absent | ProfileLockState::Stale(_)
            );
            match profile.cleanup(true, safe) {
                Ok(()) => "removed".to_string(),
                Err(error) => format!("retained: {error}"),
            }
        }
        _ => "retained: process exit unverified or profile unavailable".to_string(),
    };
    let mut map = locked(reg);
    if disposition == "removed" {
        map.remove(key);
    } else if let Some(e) = map.get_mut(key) {
        e.cleaning = false;
    }
    drop(map);
    Some(NamedSessionClose {
        name: info.name,
        session_id: info.session_id,
        runtime_id: info.runtime_id,
        closed_target_ids: if exited { targets } else { Vec::new() },
        targets_complete: targets_complete && exited,
        process_exited: exited,
        profile_disposition: disposition,
    })
}
pub async fn close_named_session(
    owner: &NamedSessionOwner,
    name: &str,
    expected_session_id: &str,
    deadline: Instant,
) -> Result<NamedSessionClose, NamedLifecycleError> {
    owner.check()?;
    check_deadline(deadline)?;
    let rx = {
        let mut workers = WORKERS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let reg = registry();
        let key = (owner.key.clone(), name.into());
        {
            let mut map = locked(&reg);
            let e = map
                .get_mut(&key)
                .filter(|e| {
                    Weak::ptr_eq(&e.owner.gcx, &owner.gcx)
                        && e.info.session_id == expected_session_id
                })
                .ok_or("Named session not found or session UUID mismatch")?;
            mark_closing(e);
            if e.runtime.is_none() || e.cleaning {
                return Err(NamedLifecycleError::after(
                    "Named session closing; cleanup still pending and quota retained",
                    expected_session_id,
                    false,
                ));
            }
            e.cleaning = true;
        }
        let id = expected_session_id.to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let worker_id = id.clone();
        let task = tokio::task::spawn_blocking(move || {
            let _ = tx.send(cleanup(&reg, &key, &id));
        });
        workers.push(Worker {
            owner: owner.gcx.clone(),
            id: worker_id,
            task,
        });
        rx
    };
    before_deadline(deadline, rx)
        .await
        .map_err(|e| NamedLifecycleError::after(e, expected_session_id, false))?
        .map_err(|e| {
            NamedLifecycleError::after(
                format!("Named session cleanup worker failed: {e}"),
                expected_session_id,
                false,
            )
        })?
        .ok_or_else(|| {
            NamedLifecycleError::after(
                "Named session cleanup unavailable",
                expected_session_id,
                false,
            )
        })
}
fn idle_expired(entry: &Entry, timeout: Duration) -> bool {
    entry.active_requests == 0 && entry.touched.elapsed() >= timeout
}
fn reconcile_quarantines(reg: &Registry, owner: &Weak<GlobalContext>) {
    let pending: Vec<_> = {
        let map = locked(reg);
        map.iter()
            .filter(|(_, entry)| {
                Weak::ptr_eq(&entry.owner.gcx, owner)
                    && entry.closing
                    && entry.runtime.is_none()
                    && entry.quarantine.is_some()
            })
            .map(|(key, entry)| {
                (
                    key.clone(),
                    entry.info.session_id.clone(),
                    entry.failed_processes.clone(),
                    entry.quarantine.clone().unwrap(),
                )
            })
            .collect()
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    for (key, id, children, profile) in pending {
        let exited = children
            .iter()
            .all(|child| child.terminate_before(deadline).unwrap_or(false));
        if exited && profile.cleanup(true, true).is_ok() {
            let mut map = locked(reg);
            if map
                .get(&key)
                .is_some_and(|entry| entry.info.session_id == id)
            {
                map.remove(&key);
            }
        }
    }
}
/// Called by the existing application monitor, including its shutdown branch. No
/// named runtime participates in default adoption, reconnect, relaunch, or health probes.
pub(super) async fn maintain_named_sessions(app: &crate::app_state::AppState, shutdown: bool) {
    let reg = registry();
    let keys = {
        let mut map = locked(&reg);
        map.iter_mut()
            .filter_map(|(key, e)| {
                let ours = Weak::ptr_eq(&e.owner.gcx, &Arc::downgrade(&app.gcx));
                if !ours {
                    return None;
                }
                let expired = e.runtime.as_ref().is_some_and(|rt| {
                    idle_expired(e, rt.ownership.launch_options().idle_timeout_or_default())
                });
                if (ours && (shutdown || expired)) || e.owner.gcx.strong_count() == 0 {
                    mark_closing(e);
                }
                (e.closing && e.runtime.is_some() && !e.cleaning)
                    .then(|| (key.clone(), e.info.session_id.clone()))
            })
            .collect::<Vec<_>>()
    };
    for (key, id) in keys {
        schedule_cleanup(reg.clone(), key, id);
    }
    reconcile_quarantines(&reg, &Arc::downgrade(&app.gcx));
    reap_workers(&Arc::downgrade(&app.gcx), shutdown).await;
    // Launch workers may have completed into quarantine while shutdown waited.
    reconcile_quarantines(&reg, &Arc::downgrade(&app.gcx));
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn owner() -> (Arc<GlobalContext>, NamedSessionOwner, tempfile::TempDir) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let cache = tempfile::tempdir().unwrap();
        let owner = NamedSessionOwner {
            gcx: Arc::downgrade(&gcx),
            key: OwnerKey {
                app: Arc::as_ptr(&gcx) as usize,
                chat: Uuid::new_v4().to_string(),
                roots: vec![cache.path().into()],
            },
            cache_dir: cache.path().into(),
        };
        (gcx, owner, cache)
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn cancelled_reservation_stays_closing_and_wrong_uuid_cannot_close() {
        let (_gcx, owner, _cache) = owner().await;
        let reg: Registry = Default::default();
        let key = (owner.key.clone(), "cancelled".into());
        let id = Uuid::new_v4().to_string();
        locked(&reg).insert(
            key.clone(),
            Entry {
                owner,
                info: NamedSessionInfo {
                    name: "cancelled".into(),
                    session_id: id.clone(),
                    runtime_id: None,
                    state: "creating".into(),
                },
                runtime: None,
                owned_process: None,
                failed_processes: Vec::new(),
                touched: Instant::now(),
                active_requests: 0,
                quarantine: None,
                closing: false,
                cleaning: false,
            },
        );
        drop(CreateGuard {
            reg: reg.clone(),
            key: key.clone(),
            id: "stale-uuid".into(),
            armed: true,
        });
        assert!(!locked(&reg).get(&key).unwrap().closing);
        drop(CreateGuard {
            reg: reg.clone(),
            key: key.clone(),
            id,
            armed: true,
        });
        assert!(locked(&reg).get(&key).unwrap().closing);
        assert_eq!(
            locked(&reg).len(),
            1,
            "launch still owns quota until cleanup proves exit"
        );
        for invalid in ["", "../escape", "x/y", "shared_default"] {
            assert!(!valid_name(invalid));
        }
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn colliding_owner_key_does_not_authorize_another_application() {
        let (_other_gcx, mut other, _other_cache) = owner().await;
        let (_gcx, owner, _cache) = owner().await;
        other.key = owner.key.clone();
        let key = (owner.key.clone(), "collision".to_string());
        let id = Uuid::new_v4().to_string();
        let reg = registry();
        locked(&reg).insert(
            key.clone(),
            Entry {
                owner: owner.clone(),
                info: NamedSessionInfo {
                    name: "collision".into(),
                    session_id: id.clone(),
                    runtime_id: None,
                    state: "creating".into(),
                },
                runtime: None,
                owned_process: None,
                failed_processes: Vec::new(),
                touched: Instant::now(),
                active_requests: 0,
                quarantine: None,
                closing: false,
                cleaning: false,
            },
        );
        assert!(list_named_sessions(&other).unwrap().is_empty());
        assert!(resolve_named_session(&other, "collision", &id).is_err());
        assert!(close_named_session(
            &other,
            "collision",
            &id,
            Instant::now() + Duration::from_secs(1)
        )
        .await
        .is_err());
        assert!(!locked(&reg).get(&key).unwrap().closing);
        locked(&reg).remove(&key);
    }
    fn reservation(owner: NamedSessionOwner, name: &str, id: &str) -> Entry {
        Entry {
            owner,
            info: NamedSessionInfo {
                name: name.into(),
                session_id: id.into(),
                runtime_id: None,
                state: "creating".into(),
            },
            runtime: None,
            owned_process: None,
            failed_processes: Vec::new(),
            touched: Instant::now(),
            active_requests: 0,
            quarantine: None,
            closing: false,
            cleaning: false,
        }
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn active_request_and_shared_clones_prevent_idle_expiry_until_last_drop() {
        let (_gcx, owner, _cache) = owner().await;
        let reg: Registry = Default::default();
        let key = (owner.key.clone(), "lease".into());
        let mut entry = reservation(owner, "lease", "generation");
        entry.touched = Instant::now() - Duration::from_secs(60);
        entry.active_requests = 1;
        locked(&reg).insert(key.clone(), entry);
        let lease = Arc::new(RequestLease {
            reg: reg.clone(),
            key: key.clone(),
            id: "generation".into(),
        });
        let clone = lease.clone();
        assert!(!idle_expired(
            locked(&reg).get(&key).unwrap(),
            Duration::from_secs(1)
        ));
        drop(lease);
        assert_eq!(locked(&reg).get(&key).unwrap().active_requests, 1);
        drop(clone);
        let map = locked(&reg);
        let entry = map.get(&key).unwrap();
        assert_eq!(entry.active_requests, 0);
        assert!(
            !idle_expired(entry, Duration::from_secs(1)),
            "completion restarts the idle clock"
        );
        assert!(idle_expired(entry, Duration::ZERO));
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_reaps_only_own_workers_while_foreign_worker_is_blocked() {
        let (_gcx, owner, _cache) = owner().await;
        let (_foreign_gcx, foreign, _foreign_cache) = self::owner().await;
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        spawn_owned(foreign.gcx.clone(), "blocked".into(), move || {
            started.send(()).unwrap();
            blocked.recv().unwrap();
        });
        ready.await.unwrap();
        spawn_owned(owner.gcx.clone(), "ours".into(), || {});
        // A completion deadline is only a hang guard: the channel proves the foreign
        // worker cannot finish before this scoped shutdown returns.
        let result =
            tokio::time::timeout(Duration::from_secs(2), reap_workers(&owner.gcx, true)).await;
        release.send(()).unwrap();
        reap_workers(&foreign.gcx, true).await;
        assert!(
            result.is_ok(),
            "shutdown awaited a foreign application worker"
        );
    }
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn quarantined_failed_launch_releases_reservation_only_after_lock_clears() {
        let (_gcx, owner, cache) = owner().await;
        let (profile_tx, profile_rx) = tokio::sync::oneshot::channel();
        let error = create_named_session_with_launch(
            owner.clone(),
            "failed",
            BrowserLaunchOptions::default(),
            Instant::now() + Duration::from_secs(5),
            move |profile, _| {
                std::fs::create_dir_all(&profile).unwrap();
                std::os::unix::fs::symlink(
                    "definitely-foreign-host-12345",
                    profile.join("SingletonLock"),
                )
                .unwrap();
                profile_tx.send(profile).unwrap();
                Err("injected vendor launch failure".into())
            },
        )
        .await
        .unwrap_err();
        let profile = profile_rx.await.unwrap();
        assert_eq!(error.stage, NamedLifecycleStage::AfterDispatchUnknown);
        assert!(error.session_id.is_some());
        let reg = registry();
        let key = (owner.key.clone(), "failed".into());
        reconcile_quarantines(&reg, &owner.gcx);
        assert!(
            locked(&reg).contains_key(&key),
            "ambiguous foreign process retains quota"
        );
        std::fs::remove_file(profile.join("SingletonLock")).unwrap();
        reconcile_quarantines(&reg, &owner.gcx);
        assert!(
            !locked(&reg).contains_key(&key),
            "failed runtime-less launch must be reinspected"
        );
        reap_workers(&owner.gcx, true).await;
        drop(cache);
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn cancelled_dispatched_create_retains_worker_until_late_failure_releases_quota() {
        let (_gcx, owner, _cache) = owner().await;
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let mut future = Box::pin(create_named_session_with_launch(
            owner.clone(),
            "late",
            BrowserLaunchOptions::default(),
            Instant::now() + Duration::from_secs(5),
            move |_, _| {
                started.send(()).unwrap();
                blocked.recv().unwrap();
                Err("late launch failure".into())
            },
        ));
        tokio::select! { result = &mut future => panic!("unexpected early result: {result:?}"), _ = ready => {} }
        // Drop the actual create future, not a duplicate cancellation algorithm.
        drop(future);
        assert!(list_named_sessions(&owner)
            .unwrap()
            .iter()
            .all(|e| e.state == "closing"));
        release.send(()).unwrap();
        reap_workers(&owner.gcx, true).await;
        assert!(list_named_sessions(&owner).unwrap().is_empty());
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn create_preflight_error_is_typed_before_dispatch() {
        let (_gcx, owner, _cache) = owner().await;
        let error = create_named_session(
            owner,
            "../bad",
            BrowserLaunchOptions::default(),
            Instant::now() + Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(error.stage, NamedLifecycleStage::BeforeDispatch);
        assert!(error.session_id.is_none());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn launch_panic_without_a_child_releases_profile_and_reservation() {
        let (_gcx, owner, _cache) = owner().await;
        let error = create_named_session_with_launch(
            owner.clone(),
            "panic-fixture",
            BrowserLaunchOptions::default(),
            Instant::now() + Duration::from_secs(5),
            |_, _| panic!("controlled launch fixture"),
        )
        .await
        .unwrap_err();
        assert_eq!(error.stage, NamedLifecycleStage::AfterDispatchCleanupKnown);
        assert!(list_named_sessions(&owner).unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(owner.cache_dir.join("named_browser_profiles"))
                .unwrap()
                .count(),
            0
        );
        reap_workers(&owner.gcx, true).await;
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn non_utf8_cache_is_rejected_before_profile_allocation() {
        use std::os::unix::ffi::OsStringExt;
        let (_gcx, mut owner, cache) = owner().await;
        owner.cache_dir = cache.path().join(std::ffi::OsString::from_vec(vec![0xff]));
        let error = create_named_session_with_launch(
            owner.clone(),
            "invalid-cache",
            BrowserLaunchOptions::default(),
            Instant::now() + Duration::from_secs(5),
            |_, _| panic!("invalid path must never dispatch launch"),
        )
        .await
        .unwrap_err();
        assert_eq!(error.stage, NamedLifecycleStage::BeforeDispatch);
        assert!(!cache.path().join("named_browser_profiles").exists());
        assert!(list_named_sessions(&owner).unwrap().is_empty());
    }

    /// Explicit opt-in: this test launches only its own two local Chrome children.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "launches two owned Chrome processes; run explicitly"]
    async fn live_named_children_are_isolated_and_generation_fenced() {
        use std::io::{Read, Write};
        let (_gcx, owner, _cache) = owner().await;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = stop.clone();
        let server = std::thread::spawn(move || {
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok((mut s, _)) = listener.accept() {
                    let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
                    let mut b = [0; 4096];
                    let _ = s.read(&mut b);
                    let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\n<html>ok</html>");
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });
        struct LiveCleanup {
            children: Vec<headless_chrome::browser::OwnedProcess>,
            stop: Arc<std::sync::atomic::AtomicBool>,
            server: Option<std::thread::JoinHandle<()>>,
        }
        impl Drop for LiveCleanup {
            fn drop(&mut self) {
                for child in &self.children {
                    let _ = child.terminate_before(Instant::now() + Duration::from_secs(5));
                }
                self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
                if let Some(server) = self.server.take() {
                    let _ = server.join();
                }
            }
        }
        let mut cleanup_guard = LiveCleanup {
            children: Vec::new(),
            stop,
            server: Some(server),
        };
        let deadline = Instant::now() + Duration::from_secs(90);
        let options = BrowserLaunchOptions {
            headless: true,
            chromium_sandbox: false,
            ..Default::default()
        };
        let a = create_named_session(owner.clone(), "a", options.clone(), deadline)
            .await
            .unwrap();
        let ra = resolve_named_session(&owner, "a", &a.session_id).unwrap();
        cleanup_guard
            .children
            .push(ra.runtime.lock().await.browser.owned_process().unwrap());
        let b = create_named_session(owner.clone(), "b", options.clone(), deadline)
            .await
            .unwrap();
        let rb = resolve_named_session(&owner, "b", &b.session_id).unwrap();
        cleanup_guard
            .children
            .push(rb.runtime.lock().await.browser.owned_process().unwrap());
        assert_ne!(ra.runtime_id, rb.runtime_id);
        assert_ne!(ra.ownership.process_id(), rb.ownership.process_id());
        assert!(
            runtime_owner_record(&ra.runtime_id).is_none(),
            "named must never enter default adoption index"
        );
        let mut stranger = owner.clone();
        stranger.key.chat = "stranger".into();
        assert!(resolve_named_session(&stranger, "a", &a.session_id).is_err());
        assert!(close_named_session(&stranger, "a", &a.session_id, deadline)
            .await
            .is_err());
        assert!(resolve_named_session(&owner, "a", &b.session_id).is_err());
        tokio::task::block_in_place(|| {
            let _guard = headless_chrome::browser::transport::CommandDeadlineGuard::new(deadline);
            let ta = ra.runtime.blocking_lock().browser.new_tab().unwrap();
            let tb = rb.runtime.blocking_lock().browser.new_tab().unwrap();
            let url = format!("http://{address}/");
            ta.navigate_to(&url)
                .unwrap()
                .wait_until_navigated()
                .unwrap();
            tb.navigate_to(&url)
                .unwrap()
                .wait_until_navigated()
                .unwrap();
            ta.evaluate(
                "document.cookie='named=alpha';localStorage.setItem('named','alpha');true",
                false,
            )
            .unwrap();
            let isolated = tb
                .evaluate(
                    "document.cookie === '' && localStorage.getItem('named') === null",
                    false,
                )
                .unwrap();
            assert_eq!(isolated.value, Some(serde_json::json!(true)));
        });
        let old_pid = ra.ownership.process_id().unwrap();
        let closed = close_named_session(&owner, "a", &a.session_id, deadline)
            .await
            .unwrap();
        assert!(closed.process_exited);
        assert!(!pid_is_alive(old_pid));
        assert!(resolve_named_session(&owner, "a", &a.session_id).is_err());
        let recreated = create_named_session(owner.clone(), "a", options, deadline)
            .await
            .unwrap();
        let recreated_runtime = resolve_named_session(&owner, "a", &recreated.session_id).unwrap();
        cleanup_guard.children.push(
            recreated_runtime
                .runtime
                .lock()
                .await
                .browser
                .owned_process()
                .unwrap(),
        );
        assert_ne!(a.session_id, recreated.session_id);
        assert!(close_named_session(&owner, "a", &a.session_id, deadline)
            .await
            .is_err());
        assert!(
            close_named_session(&owner, "a", &recreated.session_id, deadline)
                .await
                .unwrap()
                .process_exited
        );
        assert!(
            close_named_session(&owner, "b", &b.session_id, deadline)
                .await
                .unwrap()
                .process_exited
        );
        drop(cleanup_guard);
        reap_workers(&owner.gcx, true).await;
    }
}
