use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use tokio::sync::Mutex as AMutex;
use tracing::{info, warn};

pub use refact_browser::*;

use crate::http::routers::v1::browser_settings;

/// Bound cancellable waits only: never use this around a launch worker or publication.
pub(crate) async fn before_deadline<T>(
    deadline: Instant,
    future: impl std::future::Future<Output = T>,
) -> Result<T, String> {
    check_deadline(deadline)?;
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future)
        .await
        .map_err(|_| {
            "Browser request deadline exceeded; no further action was dispatched".to_string()
        })
}

fn check_deadline(deadline: Instant) -> Result<(), String> {
    if Instant::now() >= deadline {
        Err("Browser request deadline exceeded; no further action was dispatched".into())
    } else {
        Ok(())
    }
}

fn command_before<T>(deadline: Instant, execute: impl FnOnce() -> T) -> T {
    tokio::task::block_in_place(|| {
        let _guard = headless_chrome::browser::transport::CommandDeadlineGuard::new(deadline);
        execute()
    })
}

pub fn get_browser_profile_dir(gcx_cache_dir: &PathBuf, thread_id: &str) -> PathBuf {
    gcx_cache_dir.join("browser_profiles").join(thread_id)
}

fn workspace_roots(app: &crate::app_state::AppState) -> Vec<PathBuf> {
    app.gcx
        .documents_state
        .workspace_folders
        .lock()
        .map(|folders| folders.clone())
        .unwrap_or_default()
}

/// Ownership records of every registered runtime, readable without touching any runtime's
/// execution mutex. Lookups by chat id, status reads and health probes go through this index
/// so a browser wedged inside one CDP call cannot stall unrelated chats. Entries are weak:
/// the runtime owns its record, so dropping a runtime still closes its Chrome.
static RUNTIME_OWNERS: OnceLock<StdMutex<HashMap<String, Weak<RuntimeOwnership>>>> =
    OnceLock::new();

fn owners_index() -> &'static StdMutex<HashMap<String, Weak<RuntimeOwnership>>> {
    RUNTIME_OWNERS.get_or_init(Default::default)
}

fn index_runtime_owner(ownership: &Arc<RuntimeOwnership>) {
    let mut index = owners_index()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    index.retain(|_, record| record.strong_count() > 0);
    index.insert(
        ownership.runtime_id().to_string(),
        Arc::downgrade(ownership),
    );
}

fn unindex_runtime_owner(runtime_id: &str) -> Option<Arc<RuntimeOwnership>> {
    owners_index()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(runtime_id)
        .and_then(|record| record.upgrade())
}

pub fn runtime_owner_record(runtime_id: &str) -> Option<Arc<RuntimeOwnership>> {
    owners_index()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(runtime_id)
        .and_then(Weak::upgrade)
}

fn indexed_runtime_owners() -> Vec<Arc<RuntimeOwnership>> {
    owners_index()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .values()
        .filter_map(Weak::upgrade)
        .collect()
}

async fn registered_runtime_ids(app: &crate::app_state::AppState) -> HashSet<String> {
    app.integrations
        .browser_runtimes
        .lock()
        .await
        .keys()
        .cloned()
        .collect()
}

/// Ownership records for the runtimes currently in the registry.
pub async fn registered_runtime_owners(
    app: &crate::app_state::AppState,
) -> Vec<Arc<RuntimeOwnership>> {
    let ids = registered_runtime_ids(app).await;
    indexed_runtime_owners()
        .into_iter()
        .filter(|ownership| ids.contains(ownership.runtime_id()))
        .collect()
}

/// Registers a runtime and keeps exactly one generation per owning chat: any other runtime
/// still registered for the same chat is shut down first, so lookups can never pick a stale
/// generation while the chrome tool drives a fresh one.
pub async fn register_browser_runtime(
    app: crate::app_state::AppState,
    runtime: BrowserRuntime,
) -> String {
    let mut runtime = runtime;
    runtime.set_allowed_roots(workspace_roots(&app));
    let runtime_id = runtime.runtime_id.clone();
    let ownership = runtime.ownership.clone();
    if let Some(chat_id) = ownership.attached_chat_id().filter(|id| !id.is_empty()) {
        let stale: Vec<String> = registered_runtime_owners(&app)
            .await
            .into_iter()
            .filter(|other| {
                other.runtime_id() != runtime_id
                    && other.attached_chat_id().as_deref() == Some(chat_id.as_str())
            })
            .map(|other| other.runtime_id().to_string())
            .collect();
        for stale_id in stale {
            warn!(
                "BrowserRuntime {stale_id} was still registered for chat {chat_id}; \
                 replacing it with {runtime_id}"
            );
            take_browser_runtime(&app, &stale_id, false).await;
        }
    }
    index_runtime_owner(&ownership);
    drop(ownership);
    app.integrations
        .browser_runtimes
        .lock()
        .await
        .insert(runtime_id.clone(), Arc::new(AMutex::new(runtime)));
    runtime_id
}

pub const PROCESS_TERM_GRACE: Duration = Duration::from_millis(1500);

/// A zombie (`Z`) or dead (`X`) process still answers `kill(pid, 0)` but owns nothing any
/// more: its profile lock is stale and there is no point signalling it again.
fn process_is_defunct(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit(')')
                    .next()
                    .and_then(|rest| rest.trim_start().chars().next())
            })
            .is_some_and(|state| matches!(state, 'Z' | 'X'))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

pub fn pid_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
            return !process_is_defunct(pid);
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        type Handle = *mut std::ffi::c_void;
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const STILL_ACTIVE: u32 = 259;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
            fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
            fn CloseHandle(handle: Handle) -> i32;
        }

        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return false;
        }
        let mut exit_code = 0;
        let is_alive = unsafe { GetExitCodeProcess(process, &mut exit_code) != 0 }
            && exit_code == STILL_ACTIVE;
        unsafe { CloseHandle(process) };
        is_alive
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

fn terminate_process(pid: u32) {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        let deadline = std::time::Instant::now() + PROCESS_TERM_GRACE;
        while std::time::Instant::now() < deadline {
            if !pid_is_alive(pid) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}

fn process_args(pid: u32) -> Option<Vec<String>> {
    #[cfg(target_os = "linux")]
    {
        let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        Some(
            raw.split(|byte| *byte == 0)
                .filter(|arg| !arg.is_empty())
                .map(|arg| String::from_utf8_lossy(arg).into_owned())
                .collect(),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

fn user_data_dir_arg(arg: &str) -> Option<&Path> {
    arg.strip_prefix("--user-data-dir=").map(Path::new)
}

/// A process owns a profile only when one of its arguments is exactly `--user-data-dir=<profile>`;
/// a sibling profile whose path merely starts with this one (`…/abc-sub`) is someone else's.
fn process_owns_profile(pid: u32, profile_dir: &Path) -> bool {
    let Some(args) = process_args(pid) else {
        return false;
    };
    args.iter()
        .filter_map(|arg| user_data_dir_arg(arg))
        .any(|dir| dir == profile_dir)
}

/// Which process a removed runtime is allowed to terminate. Only the generation being removed
/// dies: a live profile-lock owner that is not that process is a replacement launched by
/// someone else and must be left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownTarget {
    Own(u32),
    LeaveReplacement { own: Option<u32>, lock_owner: u32 },
    Nothing,
}

pub fn shutdown_target(own_pid: Option<u32>, lock_owner: Option<u32>) -> ShutdownTarget {
    match (own_pid, lock_owner) {
        (Some(own), _) if pid_is_alive(own) => ShutdownTarget::Own(own),
        (own, Some(lock_owner)) if own != Some(lock_owner) => ShutdownTarget::LeaveReplacement {
            own,
            lock_owner,
        },
        _ => ShutdownTarget::Nothing,
    }
}

const PROFILE_SINGLETON_FILES: [&str; 3] = ["SingletonLock", "SingletonSocket", "SingletonCookie"];

/// The `SingletonLock` symlink target Chrome writes: `<hostname>-<pid>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileLock {
    pub host: String,
    pub pid: u32,
}

pub fn parse_profile_lock(target: &str) -> Option<ProfileLock> {
    let (host, pid) = target.rsplit_once('-')?;
    let pid = pid.parse::<u32>().ok().filter(|pid| *pid > 0)?;
    Some(ProfileLock {
        host: host.to_string(),
        pid,
    })
}

fn read_profile_lock(profile_dir: &Path) -> Option<Result<ProfileLock, String>> {
    let target = std::fs::read_link(profile_dir.join("SingletonLock")).ok()?;
    let target = target.to_string_lossy().into_owned();
    Some(parse_profile_lock(&target).ok_or(target))
}

pub fn local_hostname() -> String {
    hostname::get()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileLockState {
    Absent,
    Unparseable(String),
    Stale(ProfileLock),
    Live(ProfileLock),
    Foreign(ProfileLock),
}

impl ProfileLockState {
    pub fn describe(&self) -> String {
        match self {
            Self::Absent => "no profile lock".to_string(),
            Self::Unparseable(target) => format!("unparseable profile lock -> {target}"),
            Self::Stale(lock) => format!("stale lock of dead pid {} on {}", lock.pid, lock.host),
            Self::Live(lock) => format!("locked by live Chrome pid {} on {}", lock.pid, lock.host),
            Self::Foreign(lock) => format!(
                "locked by pid {} on another host {}; it cannot be checked or cleared from here",
                lock.pid, lock.host
            ),
        }
    }
}

pub fn profile_lock_state(profile_dir: &Path) -> ProfileLockState {
    match read_profile_lock(profile_dir) {
        None => ProfileLockState::Absent,
        Some(Err(target)) => ProfileLockState::Unparseable(target),
        Some(Ok(lock)) if lock.host != local_hostname() => ProfileLockState::Foreign(lock),
        Some(Ok(lock)) if pid_is_alive(lock.pid) && process_owns_profile(lock.pid, profile_dir) => {
            ProfileLockState::Live(lock)
        }
        Some(Ok(lock)) => ProfileLockState::Stale(lock),
    }
}

/// Removes singleton artifacts left by a dead Chrome on this host. Never touches a profile
/// whose lock is held by a live process here or by any process on another host.
pub fn clear_stale_profile_lock(profile_dir: &Path) -> Result<(), String> {
    let lock = match profile_lock_state(profile_dir) {
        ProfileLockState::Absent | ProfileLockState::Unparseable(_) => return Ok(()),
        ProfileLockState::Stale(lock) => lock,
        ProfileLockState::Live(lock) => {
            return Err(format!(
                "profile {} is locked by live Chrome process {} on {}",
                profile_dir.display(),
                lock.pid,
                lock.host
            ));
        }
        ProfileLockState::Foreign(lock) => {
            return Err(format!(
                "profile {} is locked by process {} on another host `{}` (this host is `{}`); \
                 close that browser or remove {}/SingletonLock by hand if that host is gone",
                profile_dir.display(),
                lock.pid,
                lock.host,
                local_hostname(),
                profile_dir.display()
            ));
        }
    };
    let pid = lock.pid;
    for name in PROFILE_SINGLETON_FILES {
        let path = profile_dir.join(name);
        if std::fs::symlink_metadata(&path).is_err() {
            continue;
        }
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "Failed to remove stale {} of dead Chrome {pid} in {}: {error}",
                    name,
                    profile_dir.display()
                );
            }
        }
    }
    info!(
        "Cleared stale singleton lock of dead Chrome {pid} in {}",
        profile_dir.display()
    );
    Ok(())
}

fn shutdown_runtime_process(pid: Option<u32>, profile_dir: &Path, delete_profile: bool) {
    match shutdown_target(pid, profile_lock_is_live(profile_dir)) {
        ShutdownTarget::Own(own) => terminate_process(own),
        ShutdownTarget::LeaveReplacement {
            own,
            lock_owner,
        } => warn!(
            "Profile {} is now locked by live Chrome {lock_owner}, not by the removed \
             runtime's process {own:?}; leaving the replacement alive",
            profile_dir.display()
        ),
        ShutdownTarget::Nothing => {}
    }
    if !profile_dir.as_os_str().is_empty() {
        let _ = clear_stale_profile_lock(profile_dir);
    }
    if !delete_profile {
        return;
    }
    let lock_state = profile_lock_state(profile_dir);
    if matches!(
        lock_state,
        ProfileLockState::Live(_) | ProfileLockState::Foreign(_)
    ) {
        warn!(
            "Not deleting browser profile {}: {}",
            profile_dir.display(),
            lock_state.describe()
        );
        return;
    }
    if let Err(error) = std::fs::remove_dir_all(profile_dir) {
        if error.kind() != std::io::ErrorKind::NotFound {
            warn!(
                "Failed to delete browser profile {}: {}",
                profile_dir.display(),
                error
            );
        }
    }
}

/// How long removal waits for a runtime's execution mutex before falling back to the
/// ownership record; a wedged CDP call must never block eviction.
pub const RUNTIME_LOCK_GRACE: Duration = Duration::from_secs(2);

async fn take_browser_runtime(
    app: &crate::app_state::AppState,
    runtime_id: &str,
    delete_profile: bool,
) -> Option<Arc<AMutex<BrowserRuntime>>> {
    take_browser_runtime_before_deadline(
        app,
        runtime_id,
        delete_profile,
        Instant::now() + Duration::from_secs(300),
    )
    .await
    .ok()
    .flatten()
}

async fn take_browser_runtime_before_deadline(
    app: &crate::app_state::AppState,
    runtime_id: &str,
    delete_profile: bool,
    deadline: Instant,
) -> Result<Option<Arc<AMutex<BrowserRuntime>>>, String> {
    let removed = before_deadline(deadline, app.integrations.browser_runtimes.lock())
        .await?
        .remove(runtime_id);
    let Some(removed) = removed else {
        return Ok(None);
    };
    let (pid, profile_dir) = match unindex_runtime_owner(runtime_id) {
        Some(ownership) => {
            ownership.invalidate();
            (
                ownership.process_id(),
                ownership.profile_dir().to_path_buf(),
            )
        }
        None => match tokio::time::timeout(
            RUNTIME_LOCK_GRACE.min(deadline.saturating_duration_since(Instant::now())),
            removed.lock(),
        )
        .await
        {
            Ok(rt) => {
                rt.ownership.invalidate();
                (rt.browser.get_process_id(), rt.profile_dir.clone())
            }
            Err(_) => {
                warn!(
                    "BrowserRuntime {runtime_id} is busy and has no ownership record; \
                     removing it without terminating its process"
                );
                (None, PathBuf::new())
            }
        },
    };
    let runtime_id = runtime_id.to_string();
    if let Err(error) = tokio::task::spawn_blocking(move || {
        shutdown_runtime_process(pid, &profile_dir, delete_profile)
    })
    .await
    {
        warn!("Browser shutdown task for runtime {runtime_id} failed: {error}");
    }
    Ok(Some(removed))
}

/// Dropping the last handle of a runtime makes headless_chrome `wait()` for its Chrome child;
/// a Chrome whose exit is stuck in the kernel (a `Zl` zombie with one thread in D state)
/// would then block the dropping thread forever. Runtimes are therefore always released on
/// a detached OS thread, never on an executor thread or inside a relaunch.
fn release_runtime_off_thread(removed: Arc<AMutex<BrowserRuntime>>, runtime_id: &str) {
    let name = format!(
        "browser-runtime-drop-{}",
        &runtime_id[..runtime_id.len().min(8)]
    );
    if let Err(error) = std::thread::Builder::new()
        .name(name)
        .spawn(move || drop(removed))
    {
        warn!("Failed to release BrowserRuntime {runtime_id} on a worker thread: {error}");
    }
}

pub async fn remove_browser_runtime(app: crate::app_state::AppState, runtime_id: &str) -> bool {
    match take_browser_runtime(&app, runtime_id, false).await {
        Some(removed) => {
            release_runtime_off_thread(removed, runtime_id);
            true
        }
        None => false,
    }
}

pub async fn discard_browser_runtime(app: crate::app_state::AppState, runtime_id: &str) -> bool {
    match take_browser_runtime(&app, runtime_id, true).await {
        Some(removed) => {
            release_runtime_off_thread(removed, runtime_id);
            true
        }
        None => false,
    }
}

pub const RUNTIME_RELEASE_TIMEOUT: Duration = Duration::from_secs(10);

async fn release_runtime_and_wait_before_deadline(
    removed: Option<Arc<AMutex<BrowserRuntime>>>,
    runtime_id: &str,
    request_deadline: Instant,
) {
    let Some(removed) = removed else {
        return;
    };
    let deadline = request_deadline.min(std::time::Instant::now() + RUNTIME_RELEASE_TIMEOUT);
    while Arc::strong_count(&removed) > 1 {
        if std::time::Instant::now() >= deadline {
            warn!(
                "BrowserRuntime {} still has {} live references after {:?}; \
                 relaunching while the old Chrome may still hold the profile lock",
                runtime_id,
                Arc::strong_count(&removed),
                RUNTIME_RELEASE_TIMEOUT
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    release_runtime_off_thread(removed, runtime_id);
}

pub fn profile_lock_is_live(profile_dir: &Path) -> Option<u32> {
    match profile_lock_state(profile_dir) {
        ProfileLockState::Live(lock) => Some(lock.pid),
        _ => None,
    }
}

pub const STALE_PROFILE_MIN_AGE: Duration = Duration::from_secs(600);
pub const STALE_REGULAR_PROFILE_MIN_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const STALE_PROFILE_SWEEP_LIMIT: usize = 256;

fn profile_min_age(name: &str) -> Duration {
    if name.starts_with("subchat-") {
        STALE_PROFILE_MIN_AGE
    } else {
        STALE_REGULAR_PROFILE_MIN_AGE
    }
}

fn live_chrome_profile_dirs() -> Option<Vec<String>> {
    #[cfg(target_os = "linux")]
    {
        let mut dirs = Vec::new();
        for entry in std::fs::read_dir("/proc").ok()? {
            let Ok(entry) = entry else { continue };
            let Some(pid) = entry
                .file_name()
                .to_string_lossy()
                .parse::<u32>()
                .ok()
                .filter(|pid| *pid > 0)
            else {
                continue;
            };
            let Some(args) = process_args(pid) else {
                continue;
            };
            dirs.extend(
                args.iter()
                    .filter_map(|arg| user_data_dir_arg(arg))
                    .map(|dir| dir.to_string_lossy().into_owned()),
            );
        }
        Some(dirs)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

pub fn sweep_stale_browser_profiles(cache_dir: &Path) -> usize {
    sweep_stale_browser_profiles_excluding(cache_dir, &[])
}

pub fn sweep_stale_browser_profiles_excluding(
    cache_dir: &Path,
    registered_profiles: &[PathBuf],
) -> usize {
    let Some(live_dirs) = live_chrome_profile_dirs() else {
        return 0;
    };
    let root = cache_dir.join("browser_profiles");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return 0;
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        if removed >= STALE_PROFILE_SWEEP_LIMIT {
            break;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !path.is_dir() {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        if registered_profiles
            .iter()
            .any(|registered| registered == &path)
        {
            continue;
        }
        if live_dirs.iter().any(|dir| dir == &path_str) {
            continue;
        }
        if matches!(
            profile_lock_state(&path),
            ProfileLockState::Live(_) | ProfileLockState::Foreign(_)
        ) {
            continue;
        }
        let min_age = profile_min_age(&name);
        let recently_used = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .map(|modified| modified.elapsed().map(|age| age < min_age).unwrap_or(true))
            .unwrap_or(true);
        if recently_used {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                removed += 1;
                info!("Removed stale browser profile {}", path.display());
            }
            Err(error) => warn!(
                "Failed to remove stale browser profile {}: {}",
                path.display(),
                error
            ),
        }
    }
    removed
}

pub async fn sweep_registered_browser_profiles(app: &crate::app_state::AppState) -> usize {
    let registered: Vec<PathBuf> = registered_runtime_owners(app)
        .await
        .into_iter()
        .map(|ownership| ownership.profile_dir().to_path_buf())
        .collect();
    let cache_dir = app.gcx.cache_dir.clone();
    tokio::task::spawn_blocking(move || {
        sweep_stale_browser_profiles_excluding(&cache_dir, &registered)
    })
    .await
    .unwrap_or(0)
}

pub fn describe_launch_failure(error: &str, profile_dir: &Path) -> String {
    match profile_lock_is_live(profile_dir) {
        Some(pid) => format!(
            "{error} — the real blocker is that profile {} is still locked by live Chrome \
             process {pid}; the previous browser never exited (any port-exhaustion wording \
             comes from the Chrome launcher and is misleading)",
            profile_dir.display()
        ),
        None if error.contains("no available ports") => format!(
            "{error} — the real blocker is that Chrome exited before reporting a debugging \
             URL after the launcher retried; ports are not exhausted (profile {})",
            profile_dir.display()
        ),
        None => format!("{error} (profile {})", profile_dir.display()),
    }
}

pub const PROFILE_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);
pub const LAUNCH_MAX_ATTEMPTS: u32 = 2;

/// Chrome launch failures that mean the new process died or handed off before it reported a
/// debugging URL, which a stale profile lock or a still-exiting predecessor explains.
pub const STARTUP_DEATH_MARKERS: [&str; 3] = [
    "no available ports",
    "didn't give us a websocket url",
    "debugging port is already in use",
];

/// Whether a failed launch attempt is retried: only once, only when the new Chrome died during
/// startup or its transport closed, and never while a live or foreign process holds the profile.
pub fn should_retry_launch(error: &str, attempt: u32, lock: &ProfileLockState) -> bool {
    let lowered = error.to_ascii_lowercase();
    let startup_death = is_transport_dead_error(error)
        || STARTUP_DEATH_MARKERS
            .iter()
            .any(|marker| lowered.contains(marker));
    attempt < LAUNCH_MAX_ATTEMPTS
        && startup_death
        && !matches!(
            lock,
            ProfileLockState::Live(_) | ProfileLockState::Foreign(_)
        )
}

/// Waits until `previous_pid` (if any) has exited and no live process on this host holds the
/// profile lock, bounded by `timeout`. Returns the lock state seen last.
pub async fn wait_for_profile_release(
    profile_dir: &Path,
    previous_pid: Option<u32>,
    timeout: Duration,
) -> ProfileLockState {
    let deadline = Instant::now() + timeout;
    loop {
        let state = profile_lock_state(profile_dir);
        let previous_alive = previous_pid.is_some_and(pid_is_alive);
        if (!previous_alive && !matches!(state, ProfileLockState::Live(_)))
            || Instant::now() >= deadline
        {
            if previous_alive {
                warn!(
                    "Previous Chrome {previous_pid:?} for {} is still alive after {timeout:?}",
                    profile_dir.display()
                );
            }
            return state;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The one launch path for an owned Chrome: waits for the previous process to release the
/// profile, clears a stale lock, launches, and on a startup death re-checks the lock and retries
/// exactly once after the settle delay.
pub async fn launch_with_lock_recovery(
    profile_dir: PathBuf,
    options: BrowserLaunchOptions,
    previous_pid: Option<u32>,
) -> Result<BrowserRuntime, String> {
    launch_with_lock_recovery_before_deadline(
        profile_dir,
        options,
        previous_pid,
        Instant::now() + Duration::from_secs(300),
    )
    .await
}

pub async fn launch_with_lock_recovery_before_deadline(
    profile_dir: PathBuf,
    options: BrowserLaunchOptions,
    previous_pid: Option<u32>,
    deadline: Instant,
) -> Result<BrowserRuntime, String> {
    check_deadline(deadline)?;
    wait_for_profile_release(
        &profile_dir,
        previous_pid,
        PROFILE_RELEASE_TIMEOUT.min(deadline.saturating_duration_since(Instant::now())),
    )
    .await;
    check_deadline(deadline)?;
    clear_stale_profile_lock(&profile_dir)
        .map_err(|error| format!("{error}; the previous browser never exited"))?;
    let mut attempt = 1;
    loop {
        check_deadline(deadline)?;
        let launch_dir = profile_dir.clone();
        let launch_options = options.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let _guard = headless_chrome::browser::transport::CommandDeadlineGuard::new(deadline);
            check_deadline(deadline)?;
            let runtime = BrowserRuntime::launch(launch_dir, launch_options)?;
            // The worker retains ownership through cleanup; no detached launch can publish later.
            if let Err(error) = check_deadline(deadline) {
                drop(runtime);
                return Err(error);
            }
            Ok(runtime)
        })
        .await
        .map_err(|error| format!("browser launch task failed: {error}"))
        .and_then(|outcome| outcome);
        let error = match outcome {
            Ok(runtime) => return Ok(runtime),
            Err(error) => error,
        };
        check_deadline(deadline)?;
        let lock = profile_lock_state(&profile_dir);
        if !should_retry_launch(&error, attempt, &lock) {
            return Err(format!(
                "{} after {attempt} attempt(s); {}",
                describe_launch_failure(&error, &profile_dir),
                lock.describe()
            ));
        }
        warn!(
            "Chrome died while starting on {} ({error}; {}); retrying once",
            profile_dir.display(),
            lock.describe()
        );
        if let ProfileLockState::Stale(_) = lock {
            clear_stale_profile_lock(&profile_dir)?;
        }
        before_deadline(
            deadline,
            tokio::time::sleep(browser_settings::current().relaunch_settle()),
        )
        .await?;
        attempt += 1;
    }
}

/// Says whether a dead runtime's Chrome actually exited or only its CDP socket closed, with the
/// transport error variant, so a relaunch warning is diagnosable.
pub fn transport_death_note(pid: Option<u32>, error: Option<&str>) -> String {
    let process = match pid {
        Some(pid) if pid_is_alive(pid) => {
            format!("Chrome process {pid} is still alive, so only the CDP socket closed")
        }
        Some(pid) => format!("Chrome process {pid} had exited (crash or external kill)"),
        None => "the Chrome process id is unknown".to_string(),
    };
    let variant = match error {
        Some(error) => match transport_dead_marker(error) {
            Some(marker) => format!("transport error `{marker}`"),
            None => format!("error `{}`", bounded_cdp_text(error)),
        },
        None => "the connection probe failed before the batch ran".to_string(),
    };
    format!("browser session died: {process}; {variant}")
}

#[derive(Debug, Clone, Default)]
pub struct ChatRuntimeLookup {
    pub chat_id: String,
    pub root_chat_id: Option<String>,
    pub background_agent_id: Option<String>,
}

impl ChatRuntimeLookup {
    pub fn for_chat(chat_id: &str) -> Self {
        Self {
            chat_id: chat_id.to_string(),
            ..Default::default()
        }
    }

    fn ordered_ids(&self) -> Vec<String> {
        let mut ids = vec![self.chat_id.clone()];
        for extra in [
            self.root_chat_id.as_deref(),
            self.background_agent_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if !extra.is_empty() && !ids.iter().any(|id| id == extra) {
                ids.push(extra.to_string());
            }
        }
        ids
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOwner {
    pub runtime_id: String,
    pub attached_chat_id: Option<String>,
    pub connected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSelection {
    pub runtime_id: String,
    pub adopted: bool,
}

/// Picks the runtime for a chat from ownership facts alone. Exact ids (the chat, then its
/// root chat, then its background agent) win, a connected generation beats a dead one, and
/// only a runtime nobody owns can be adopted; a lexical prefix is never ancestry.
pub fn select_runtime_for_chat(
    owners: &[RuntimeOwner],
    lookup: &ChatRuntimeLookup,
) -> Option<RuntimeSelection> {
    for id in lookup.ordered_ids() {
        let candidates = owners
            .iter()
            .filter(|owner| owner.attached_chat_id.as_deref() == Some(id.as_str()))
            .collect::<Vec<_>>();
        if let Some(owner) = candidates
            .iter()
            .find(|owner| owner.connected)
            .or_else(|| candidates.first())
        {
            return Some(RuntimeSelection {
                runtime_id: owner.runtime_id.clone(),
                adopted: id != lookup.chat_id,
            });
        }
    }
    let unowned = owners
        .iter()
        .filter(|owner| owner.attached_chat_id.as_deref().is_none_or(str::is_empty))
        .collect::<Vec<_>>();
    match unowned.as_slice() {
        [only] => Some(RuntimeSelection {
            runtime_id: only.runtime_id.clone(),
            adopted: true,
        }),
        _ => None,
    }
}

pub async fn find_runtime_for_chat(
    app: crate::app_state::AppState,
    lookup: &ChatRuntimeLookup,
) -> Option<(String, Arc<AMutex<BrowserRuntime>>)> {
    let records = registered_runtime_owners(&app).await;
    let owners = records
        .iter()
        .map(|record| {
            let state = record.snapshot();
            RuntimeOwner {
                runtime_id: record.runtime_id().to_string(),
                attached_chat_id: state.attached_chat_id,
                connected: state.connected,
            }
        })
        .collect::<Vec<_>>();
    let selection = select_runtime_for_chat(&owners, lookup)?;
    let record = records
        .iter()
        .find(|record| record.runtime_id() == selection.runtime_id)?;
    if selection.adopted
        && record
            .attached_chat_id()
            .is_none_or(|owner| owner.is_empty())
    {
        if !record.claim_if_unowned(&lookup.chat_id) {
            return None;
        }
        info!(
            "BrowserRuntime {} adopted by chat {}",
            selection.runtime_id, lookup.chat_id
        );
    }
    let arc = app
        .integrations
        .browser_runtimes
        .lock()
        .await
        .get(&selection.runtime_id)
        .cloned()?;
    Some((selection.runtime_id, arc))
}

pub async fn find_runtime_by_chat_id(
    app: crate::app_state::AppState,
    chat_id: &str,
) -> Option<(String, Arc<AMutex<BrowserRuntime>>)> {
    find_runtime_for_chat(app, &ChatRuntimeLookup::for_chat(chat_id)).await
}

#[allow(dead_code)]
pub const RELAUNCH_SETTLE: Duration = Duration::from_millis(800);

pub const RELAUNCH_WARNING: &str =
    "browser session was dead; relaunched and retried (open tabs lost, cookies/localStorage kept)";

pub const RELAUNCH_FENCE: &str = "the browser was relaunched after its previous process died, so every tab and all page state from before are gone and the active tab is about:blank";

pub fn recovery_fence_error(reason: &str) -> String {
    format!(
        "{reason}; this step needs a live document, so it was refused — start the batch with navigate, open_tab or set_content and retry"
    )
}

pub fn relaunch_resume_warning(resume_index: usize) -> String {
    format!(
        "browser session was dead; relaunched and resumed from step {resume_index} — open tabs were lost, cookies/storage persist via profile"
    )
}

pub fn relaunch_replay_warning(
    replayed_index: usize,
    resume_index: usize,
    replayed_summary: &str,
) -> String {
    let unreplayed = if replayed_index + 1 < resume_index {
        format!(
            "; steps {}..={} were NOT replayed, so page state produced by them may be lost",
            replayed_index + 1,
            resume_index - 1
        )
    } else {
        String::new()
    };
    format!(
        "browser session was dead; relaunched, replayed step {replayed_index} ({replayed_summary}) to restore the document, then resumed from step {resume_index} — open tabs were lost, cookies/storage persist via profile{unreplayed}"
    )
}

pub fn relaunch_url_restore_warning(url: &str, resume_index: usize) -> String {
    format!(
        "browser session was dead; relaunched, reopened the last committed document {url} (reached through earlier clicks or an earlier batch), then resumed from step {resume_index} — open tabs and in-page state were lost, cookies/storage persist via profile"
    )
}

pub fn relaunch_without_page_context_warning(resume_index: usize) -> String {
    format!(
        "browser session was dead and was relaunched, but no navigate/open_tab/set_content step precedes step {resume_index} in this batch and the last document URL is not restorable, so the page could not be restored and the relaunched browser is at about:blank; steps {resume_index}+ were not run — navigate again and retry them"
    )
}

static RELAUNCH_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<AMutex<()>>>>> = OnceLock::new();

/// One guard per chat shared by every create/relaunch path, so a monitor relaunch and a tool
/// launch for the same chat serialize instead of racing for one profile.
pub fn relaunch_lock_for_chat(chat_id: &str) -> Arc<AMutex<()>> {
    let locks = RELAUNCH_LOCKS.get_or_init(Default::default);
    let mut locks = locks.lock().unwrap_or_else(|error| error.into_inner());
    locks.retain(|key, lock| key == chat_id || Arc::strong_count(lock) > 1);
    locks.entry(chat_id.to_string()).or_default().clone()
}

#[derive(Debug, Clone)]
pub struct RuntimeRecoveryPlan {
    pub runtime_id: String,
    pub chat_id: String,
    pub profile_dir: PathBuf,
    pub launch_options: BrowserLaunchOptions,
}

pub async fn ensure_frame_emitter(
    app: crate::app_state::AppState,
    chat_id: &str,
    runtime_id: &str,
) {
    let runtime_arc = {
        let browser_runtimes = app.integrations.browser_runtimes.clone();
        let browser_runtimes = browser_runtimes.lock().await;
        browser_runtimes.get(runtime_id).cloned()
    };
    let Some(runtime_arc) = runtime_arc else {
        return;
    };
    let should_spawn = {
        let mut rt = runtime_arc.lock().await;
        if rt.frame_emitter_active {
            false
        } else {
            rt.frame_emitter_active = true;
            true
        }
    };
    if should_spawn {
        tokio::spawn(
            crate::http::routers::v1::v1_browser::browser_frame_emission_task(
                app.gcx.clone(),
                chat_id.to_string(),
                runtime_id.to_string(),
            ),
        );
    }
}

/// Preflight-only recovery. Never resumes a dispatched batch or replaces a live process.
pub async fn reconnect_runtime_for_chat(
    app: crate::app_state::AppState,
    chat_id: &str,
) -> Result<ReconnectOutcome, String> {
    reconnect_runtime_for_chat_before_deadline(
        app,
        chat_id,
        Instant::now() + Duration::from_secs(300),
    )
    .await
}

pub async fn reconnect_runtime_for_chat_before_deadline(
    app: crate::app_state::AppState,
    chat_id: &str,
    deadline: Instant,
) -> Result<ReconnectOutcome, String> {
    let lock = relaunch_lock_for_chat(chat_id);
    let _guard = before_deadline(deadline, lock.lock()).await?;
    let (id, runtime) = before_deadline(deadline, find_runtime_by_chat_id(app, chat_id))
        .await?
        .ok_or("No browser runtime to reconnect")?;
    let owner = runtime_owner_record(&id).ok_or("No browser ownership record")?;
    if !owner.is_current_generation()
        || !owner.owns_browser_process()
        || !owner.process_id().is_some_and(pid_is_alive)
    {
        return Err("Reconnect refused: original owned process is not alive".into());
    }
    let mut runtime = tokio::time::timeout(
        RUNTIME_LOCK_GRACE.min(deadline.saturating_duration_since(Instant::now())),
        runtime.lock(),
    )
    .await
    .map_err(|_| "Reconnect refused: runtime busy; live browser retained")?;
    command_before(deadline, || runtime.reconnect_same_process())
}

pub async fn relaunch_runtime_for_chat(
    app: crate::app_state::AppState,
    chat_id: &str,
    profile_dir: PathBuf,
    launch_options: BrowserLaunchOptions,
    window_bounds: Option<refact_chat_api::WindowBounds>,
) -> Result<String, String> {
    relaunch_runtime_for_chat_before_deadline(
        app,
        chat_id,
        profile_dir,
        launch_options,
        window_bounds,
        Instant::now() + Duration::from_secs(300),
    )
    .await
}

pub async fn relaunch_runtime_for_chat_before_deadline(
    app: crate::app_state::AppState,
    chat_id: &str,
    profile_dir: PathBuf,
    launch_options: BrowserLaunchOptions,
    window_bounds: Option<refact_chat_api::WindowBounds>,
    deadline: Instant,
) -> Result<String, String> {
    check_deadline(deadline)?;
    let options = BrowserLaunchOptions {
        window_bounds: window_bounds.or(launch_options.window_bounds.clone()),
        ..launch_options
    };

    let relaunch_lock = relaunch_lock_for_chat(chat_id);
    let _relaunch_guard = before_deadline(deadline, relaunch_lock.lock()).await?;

    let mut previous_pid = None;
    let previous_emitter_active = match before_deadline(
        deadline,
        find_runtime_by_chat_id(app.clone(), chat_id),
    )
    .await?
    {
        Some((runtime_id, runtime_arc)) => {
            // Capture ownership before waiting for the execution lock. Socket failure alone
            // must never authorize killing a live owned process.
            let owner = runtime_owner_record(&runtime_id);
            let original_alive = owner
                .as_ref()
                .is_some_and(|o| o.process_id().is_some_and(pid_is_alive));
            let (emitter_active, usable) = match tokio::time::timeout(
                RUNTIME_LOCK_GRACE.min(deadline.saturating_duration_since(Instant::now())),
                runtime_arc.lock(),
            )
            .await
            {
                Ok(mut rt) => {
                    let same_mode = rt.launch_options.headless == options.headless;
                    let process_alive = rt.ownership.process_id().is_none_or(pid_is_alive);
                    let usable = same_mode
                        && process_alive
                        && command_before(deadline, || rt.check_connection());
                    if !usable && original_alive && same_mode {
                        command_before(deadline, || rt.reconnect_same_process())?;
                        return Ok(runtime_id);
                    }
                    (rt.frame_emitter_active, usable)
                }
                Err(_) => {
                    if original_alive {
                        return Err(
                            "Browser runtime busy; live owned browser retained, not relaunched"
                                .into(),
                        );
                    }
                    warn!(
                        "BrowserRuntime {} for chat {} did not release its lock within {:?}; \
                             replacing it without waiting",
                        runtime_id, chat_id, RUNTIME_LOCK_GRACE
                    );
                    (false, false)
                }
            };
            if usable {
                info!(
                    "BrowserRuntime {} is already live for chat {}, reusing it instead of relaunching",
                    runtime_id, chat_id
                );
                return Ok(runtime_id);
            }
            drop(runtime_arc);
            previous_pid = runtime_owner_record(&runtime_id).and_then(|owner| owner.process_id());
            let removed =
                take_browser_runtime_before_deadline(&app, &runtime_id, false, deadline).await?;
            release_runtime_and_wait_before_deadline(removed, &runtime_id, deadline).await;
            before_deadline(
                deadline,
                tokio::time::sleep(browser_settings::current().relaunch_settle()),
            )
            .await?;
            emitter_active
        }
        None => false,
    };

    let mode = options.mode_label();
    let mut runtime =
        launch_with_lock_recovery_before_deadline(profile_dir, options, previous_pid, deadline)
            .await
            .map_err(|error| format!("Failed to relaunch browser in {mode} mode: {error}"))?;
    runtime.reattach(chat_id);
    runtime.fence_after_relaunch(RELAUNCH_FENCE);
    // Configure before publication, retaining ownership on any deadline failure.
    runtime.set_allowed_roots(workspace_roots(&app));
    if let Err(error) = check_deadline(deadline) {
        command_before(deadline, || drop(runtime));
        return Err(error);
    }
    if let Err(error) = command_before(deadline, || setup_recording_for_runtime(&mut runtime)) {
        warn!("Browser recording setup failed after {mode} relaunch (non-fatal): {error}");
    }
    let runtime_id = runtime.runtime_id.clone();
    let mut runtimes =
        match before_deadline(deadline, app.integrations.browser_runtimes.lock()).await {
            Ok(runtimes) => runtimes,
            Err(error) => {
                command_before(deadline, || drop(runtime));
                return Err(error);
            }
        };
    if let Err(error) = check_deadline(deadline) {
        drop(runtimes);
        command_before(deadline, || drop(runtime));
        return Err(error);
    }
    index_runtime_owner(&runtime.ownership);
    if previous_emitter_active {
        runtime.frame_emitter_active = true;
    }
    runtimes.insert(runtime_id.clone(), Arc::new(AMutex::new(runtime)));
    drop(runtimes);
    if previous_emitter_active {
        tokio::spawn(
            crate::http::routers::v1::v1_browser::browser_frame_emission_task(
                app.gcx.clone(),
                chat_id.to_string(),
                runtime_id.clone(),
            ),
        );
    }

    info!(
        "BrowserRuntime {} relaunched ({}) for chat {}",
        runtime_id, mode, chat_id
    );
    Ok(runtime_id)
}

/// How long a state snapshot waits for a busy runtime before reporting ownership facts only.
pub const SNAPSHOT_LOCK_GRACE: Duration = Duration::from_millis(250);

pub async fn browser_snapshot_for_chat(
    app: crate::app_state::AppState,
    chat_id: &str,
) -> Option<crate::chat::types::BrowserSnapshot> {
    let (runtime_id, runtime_arc) = find_runtime_by_chat_id(app, chat_id).await?;
    let Ok(mut rt) = tokio::time::timeout(SNAPSHOT_LOCK_GRACE, runtime_arc.lock()).await else {
        let connected = runtime_owner_record(&runtime_id)
            .map(|record| record.is_connected())
            .unwrap_or(false);
        return Some(crate::chat::types::BrowserSnapshot {
            runtime_id,
            connected,
            active_tab: None,
            url: None,
            title: None,
            tabs: Vec::new(),
        });
    };
    refact_browser::adopt_new_tabs(&mut rt, None);
    let tabs = rt
        .list_tab_infos()
        .into_iter()
        .map(|t| crate::chat::types::BrowserTabInfo {
            tab_id: t.id,
            url: t.url,
            title: t.title,
        })
        .collect::<Vec<_>>();
    let (url, title) = match rt.get_active_tab() {
        Some(tab) => (
            Some(tab.get_url()).filter(|s| !s.is_empty()),
            Some(tab.get_title().unwrap_or_default()).filter(|s| !s.is_empty()),
        ),
        None => (None, None),
    };
    Some(crate::chat::types::BrowserSnapshot {
        runtime_id,
        connected: rt.is_connected,
        active_tab: rt.active_tab_target_id().map(|s| s.to_string()),
        url,
        title,
        tabs,
    })
}

pub const MONITOR_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
pub const HEALTH_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_UNRESPONSIVE_PROBES: u32 = 3;
pub const RELAUNCH_MAX_FAILURES: u32 = 3;
pub const RELAUNCH_BACKOFF_BASE: Duration = Duration::from_secs(5);
pub const RELAUNCH_BACKOFF_MAX: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelaunchBudget {
    pub failures: u32,
    pub next_allowed: Option<Instant>,
}

impl RelaunchBudget {
    pub fn exhausted(&self) -> bool {
        self.failures >= RELAUNCH_MAX_FAILURES
    }

    pub fn may_attempt(&self, now: Instant) -> bool {
        !self.exhausted() && self.next_allowed.is_none_or(|allowed| now >= allowed)
    }

    pub fn record_failure(&mut self, now: Instant) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let exponent = self.failures.saturating_sub(1).min(16);
        let delay = RELAUNCH_BACKOFF_BASE
            .saturating_mul(1u32 << exponent)
            .min(RELAUNCH_BACKOFF_MAX);
        self.next_allowed = Some(now + delay);
        delay
    }

    pub fn record_success(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryDecision {
    Relaunch,
    Wait,
    Retire(String),
}

/// A dead runtime is relaunched only while someone can use it right away: the chat is busy
/// generating or executing tools, or Browser Mode is streaming its frames. Idle, finished and
/// unloaded chats get their browser back on demand from the next tool call instead of the
/// monitor respawning Chrome for nobody.
pub fn decide_recovery(
    chat_active: Option<bool>,
    frame_emitter_active: bool,
    budget: &RelaunchBudget,
    now: Instant,
) -> RecoveryDecision {
    let wanted = frame_emitter_active || chat_active == Some(true);
    if !wanted {
        return RecoveryDecision::Retire(match chat_active {
            None => "its chat is not loaded".to_string(),
            _ => "its chat is idle; the next chrome call relaunches it on demand".to_string(),
        });
    }
    if budget.exhausted() {
        return RecoveryDecision::Retire(format!(
            "relaunch failed {} times in a row; the next chrome call retries on demand",
            budget.failures
        ));
    }
    if !budget.may_attempt(now) {
        return RecoveryDecision::Wait;
    }
    RecoveryDecision::Relaunch
}

pub fn chat_is_disposable(chat_id: &str) -> bool {
    chat_id.starts_with("subchat-")
}

async fn chat_is_active(app: &crate::app_state::AppState, chat_id: &str) -> Option<bool> {
    let session = app.chat.sessions.read().await.get(chat_id).cloned()?;
    let active = match tokio::time::timeout(Duration::from_millis(100), session.lock()).await {
        Ok(session) => matches!(
            session.runtime.state,
            refact_chat_api::SessionState::Generating
                | refact_chat_api::SessionState::ExecutingTools
        ),
        Err(_) => true,
    };
    Some(active)
}

struct RuntimeHealth {
    chat_id: Option<String>,
    was_connected: bool,
    still_connected: bool,
    idle_expired: bool,
    idle_timeout: Duration,
    profile_dir: PathBuf,
    launch_options: BrowserLaunchOptions,
    frame_emitter_active: bool,
}

enum RuntimeCheck {
    Healthy,
    Inspected(RuntimeHealth),
}

/// Health of a runtime whose execution mutex is busy: probe through the ownership record on
/// a blocking thread with a hard deadline, and only after repeated silence treat the browser
/// as wedged so its process gets replaced.
async fn check_busy_runtime(runtime_id: &str, ownership: Arc<RuntimeOwnership>) -> RuntimeCheck {
    let probe = ownership.clone();
    let connected = tokio::task::spawn_blocking(move || probe.probe_connection())
        .await
        .ok()
        .flatten();
    match connected {
        Some(true) => return RuntimeCheck::Healthy,
        Some(false) => {
            warn!("BrowserRuntime {runtime_id} is busy and its transport is closed");
        }
        None => {
            let silent = ownership.record_unresponsive_probe();
            if silent < MAX_UNRESPONSIVE_PROBES {
                warn!(
                    "BrowserRuntime {runtime_id} did not answer a health probe within {:?} \
                     ({silent}/{MAX_UNRESPONSIVE_PROBES})",
                    HEALTH_PROBE_TIMEOUT
                );
                return RuntimeCheck::Healthy;
            }
            warn!(
                "BrowserRuntime {runtime_id} ignored {silent} consecutive health probes; \
                 treating it as wedged"
            );
        }
    }
    let state = ownership.snapshot();
    RuntimeCheck::Inspected(RuntimeHealth {
        chat_id: state.attached_chat_id,
        was_connected: state.connected,
        still_connected: false,
        idle_expired: false,
        idle_timeout: ownership.launch_options().idle_timeout_or_default(),
        profile_dir: ownership.profile_dir().to_path_buf(),
        launch_options: ownership.launch_options().clone(),
        frame_emitter_active: false,
    })
}

pub async fn browser_monitor_background_task(app: crate::app_state::AppState) {
    let cache_dir = app.gcx.cache_dir.clone();
    match tokio::task::spawn_blocking(move || sweep_stale_browser_profiles(&cache_dir)).await {
        Ok(removed) if removed > 0 => info!("Swept {removed} stale browser profile dirs"),
        Ok(_) => {}
        Err(error) => warn!("Stale browser profile sweep failed: {error}"),
    }
    let mut budgets: HashMap<String, RelaunchBudget> = HashMap::new();
    loop {
        let monitor_interval = browser_settings::current().monitor_interval();
        let shutdown_flag = app.runtime.shutdown_flag.clone();
        tokio::select! {
            _ = tokio::time::sleep(monitor_interval) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            } => {
                return;
            }
        }

        let runtime_ids: Vec<String> = {
            let browser_runtimes = app.integrations.browser_runtimes.clone();
            let browser_runtimes = browser_runtimes.lock().await;
            browser_runtimes.keys().cloned().collect()
        };

        let mut to_remove: Vec<(String, bool, String)> = Vec::new();
        let mut to_relaunch: Vec<RuntimeRecoveryPlan> = Vec::new();
        for rid in &runtime_ids {
            let runtime_arc = {
                let browser_runtimes = app.integrations.browser_runtimes.clone();
                let browser_runtimes = browser_runtimes.lock().await;
                match browser_runtimes.get(rid) {
                    Some(arc) => arc.clone(),
                    None => continue,
                }
            };

            let check = match tokio::time::timeout(MONITOR_LOCK_TIMEOUT, runtime_arc.lock()).await {
                Ok(mut rt) => RuntimeCheck::Inspected(tokio::task::block_in_place(|| {
                    let _deadline = headless_chrome::browser::transport::CommandDeadlineGuard::new(
                        Instant::now() + Duration::from_secs(5),
                    );
                    if let Err(error) = rt.maintain_artifacts() {
                        warn!("Browser artifact maintenance failed: {error}");
                    }
                    refact_browser::adopt_new_tabs(&mut rt, None);
                    let was_connected = rt.is_connected;
                    let still_connected = rt.check_connection();
                    RuntimeHealth {
                        chat_id: rt.attached_chat_id(),
                        was_connected,
                        still_connected,
                        idle_expired: rt.is_idle_expired(),
                        idle_timeout: rt.idle_timeout,
                        profile_dir: rt.profile_dir.clone(),
                        launch_options: rt.launch_options.clone(),
                        frame_emitter_active: rt.frame_emitter_active,
                    }
                })),
                Err(_) => match runtime_owner_record(rid) {
                    Some(ownership) => check_busy_runtime(rid, ownership).await,
                    None => continue,
                },
            };
            let RuntimeHealth {
                chat_id,
                was_connected,
                still_connected,
                idle_expired,
                idle_timeout,
                profile_dir,
                launch_options,
                frame_emitter_active,
            } = match check {
                RuntimeCheck::Healthy => continue,
                RuntimeCheck::Inspected(health) => health,
            };

            if was_connected && !still_connected {
                info!(
                    "BrowserRuntime {} (chat {:?}) lost connection",
                    rid, chat_id
                );
            }

            let chat_active = match &chat_id {
                Some(chat_id) => chat_is_active(&app, chat_id).await,
                None => None,
            };
            let disposable_idle = chat_id
                .as_deref()
                .is_some_and(|chat_id| chat_is_disposable(chat_id) && chat_active != Some(true));
            let evict_idle = idle_expired
                && (chat_id.is_none() || launch_options.evict_idle_attached || disposable_idle);
            if evict_idle {
                warn!(
                    "BrowserRuntime {} idle timeout ({:?}) for chat {:?}",
                    rid, idle_timeout, chat_id
                );
                to_remove.push((rid.clone(), true, "idle timeout".to_string()));
                continue;
            }

            if still_connected {
                if let Some(chat_id) = &chat_id {
                    budgets.remove(chat_id);
                }
                continue;
            }

            // Disconnected live browsers are retained for qualified preflight reconnect.
            // Monitor retirement/relaunch is never permission to destroy in-page state.
            if runtime_owner_record(rid)
                .is_some_and(|owner| owner.process_id().is_some_and(pid_is_alive))
            {
                continue;
            }

            match chat_id {
                Some(chat_id) if !profile_dir.as_os_str().is_empty() => {
                    let budget = budgets.entry(chat_id.clone()).or_default();
                    match decide_recovery(chat_active, frame_emitter_active, budget, Instant::now())
                    {
                        RecoveryDecision::Relaunch => to_relaunch.push(RuntimeRecoveryPlan {
                            runtime_id: rid.clone(),
                            chat_id,
                            profile_dir,
                            launch_options,
                        }),
                        RecoveryDecision::Wait => {}
                        RecoveryDecision::Retire(reason) => {
                            budgets.remove(&chat_id);
                            to_remove.push((rid.clone(), false, reason));
                        }
                    }
                }
                Some(_) => to_remove.push((
                    rid.clone(),
                    false,
                    "a connected (not launched) runtime cannot be relaunched".to_string(),
                )),
                None => to_remove.push((rid.clone(), true, "no chat owns it".to_string())),
            }
        }

        for (rid, delete_profile, reason) in to_remove {
            info!("Retiring BrowserRuntime {rid}: {reason}");
            if delete_profile {
                discard_browser_runtime(app.clone(), &rid).await;
            } else {
                remove_browser_runtime(app.clone(), &rid).await;
            }
        }

        for plan in to_relaunch {
            warn!(
                "BrowserRuntime {} is dead while attached to chat {}, relaunching",
                plan.runtime_id, plan.chat_id
            );
            match relaunch_runtime_for_chat(
                app.clone(),
                &plan.chat_id,
                plan.profile_dir.clone(),
                plan.launch_options.clone(),
                None,
            )
            .await
            {
                Ok(_) => {
                    budgets.remove(&plan.chat_id);
                }
                Err(error) => {
                    let budget = budgets.entry(plan.chat_id.clone()).or_default();
                    let delay = budget.record_failure(Instant::now());
                    warn!(
                        "Failed to relaunch dead BrowserRuntime {} for chat {} (attempt {}/{}, next in {:?}): {}",
                        plan.runtime_id,
                        plan.chat_id,
                        budget.failures,
                        RELAUNCH_MAX_FAILURES,
                        delay,
                        error
                    );
                    if !runtime_owner_record(&plan.runtime_id)
                        .is_some_and(|owner| owner.process_id().is_some_and(pid_is_alive))
                    {
                        remove_browser_runtime(app.clone(), &plan.runtime_id).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_get_browser_profile_dir() {
        let cache_dir = PathBuf::from("/tmp/refact-cache");
        let profile = get_browser_profile_dir(&cache_dir, "thread-abc-123");
        assert_eq!(
            profile,
            PathBuf::from("/tmp/refact-cache/browser_profiles/thread-abc-123")
        );
    }

    #[test]
    fn concurrent_relaunches_for_one_chat_share_a_single_guard() {
        let first = relaunch_lock_for_chat("chat-relaunch-guard");
        let second = relaunch_lock_for_chat("chat-relaunch-guard");
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(
            &first,
            &relaunch_lock_for_chat("other-chat-relaunch-guard")
        ));

        let guard = first.try_lock().expect("first relaunch takes the guard");
        assert!(
            second.try_lock().is_err(),
            "a second relaunch for the same chat must wait instead of launching another browser"
        );
        drop(guard);
        assert!(second.try_lock().is_ok());
    }

    fn owner(runtime_id: &str, attached: Option<&str>) -> RuntimeOwner {
        RuntimeOwner {
            runtime_id: runtime_id.to_string(),
            attached_chat_id: attached.map(str::to_string),
            connected: true,
        }
    }

    #[test]
    fn lexical_prefix_is_not_ancestry() {
        let owners = vec![owner("rt-one", Some("chat-1"))];
        let lookup = ChatRuntimeLookup::for_chat("chat-10");
        assert_eq!(select_runtime_for_chat(&owners, &lookup), None);

        let owners = vec![owner("rt-root", Some("root"))];
        let lookup = ChatRuntimeLookup::for_chat("root-child");
        assert_eq!(select_runtime_for_chat(&owners, &lookup), None);
    }

    #[test]
    fn a_connected_generation_beats_a_dead_one_for_the_same_owner() {
        let owners = vec![
            RuntimeOwner {
                runtime_id: "rt-dead".to_string(),
                attached_chat_id: Some("chat".to_string()),
                connected: false,
            },
            owner("rt-live", Some("chat")),
        ];
        let selection =
            select_runtime_for_chat(&owners, &ChatRuntimeLookup::for_chat("chat")).unwrap();
        assert_eq!(selection.runtime_id, "rt-live");
        assert!(!selection.adopted);

        let only_dead = vec![RuntimeOwner {
            runtime_id: "rt-dead".to_string(),
            attached_chat_id: Some("chat".to_string()),
            connected: false,
        }];
        let selection =
            select_runtime_for_chat(&only_dead, &ChatRuntimeLookup::for_chat("chat")).unwrap();
        assert_eq!(selection.runtime_id, "rt-dead");
    }

    #[test]
    fn a_lone_unowned_runtime_is_adopted_even_beside_owned_ones() {
        let owners = vec![
            owner("rt-a", Some("chat-a")),
            owner("rt-free", None),
            owner("rt-b", Some("chat-b")),
        ];
        let selection =
            select_runtime_for_chat(&owners, &ChatRuntimeLookup::for_chat("chat-new")).unwrap();
        assert_eq!(selection.runtime_id, "rt-free");
        assert!(selection.adopted);
    }

    #[test]
    fn stale_cleanup_cannot_kill_replacement() {
        let own = std::process::id();
        let dead = 4_294_000_002u32;
        assert_eq!(
            shutdown_target(Some(own), Some(own)),
            ShutdownTarget::Own(own)
        );
        assert_eq!(shutdown_target(Some(own), None), ShutdownTarget::Own(own));
        assert_eq!(
            shutdown_target(Some(dead), Some(own)),
            ShutdownTarget::LeaveReplacement {
                own: Some(dead),
                lock_owner: own
            }
        );
        assert_eq!(
            shutdown_target(None, Some(own)),
            ShutdownTarget::LeaveReplacement {
                own: None,
                lock_owner: own
            }
        );
        assert_eq!(shutdown_target(Some(dead), None), ShutdownTarget::Nothing);
        assert_eq!(shutdown_target(None, None), ShutdownTarget::Nothing);
    }

    #[test]
    fn dead_singleton_locks_are_cleared_before_launch() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(clear_stale_profile_lock(dir.path()), Ok(()));
        #[cfg(unix)]
        {
            let dead_pid = 4_294_000_003u32;
            let lock = dir.path().join("SingletonLock");
            std::os::unix::fs::symlink(format!("{}-{dead_pid}", local_hostname()), &lock).unwrap();
            std::fs::write(dir.path().join("SingletonCookie"), b"cookie").unwrap();
            assert_eq!(clear_stale_profile_lock(dir.path()), Ok(()));
            assert!(std::fs::symlink_metadata(&lock).is_err());
            assert!(!dir.path().join("SingletonCookie").exists());

            let foreign_host = format!("not-{}", local_hostname());
            std::os::unix::fs::symlink(format!("{foreign_host}-{dead_pid}"), &lock).unwrap();
            let error = clear_stale_profile_lock(dir.path())
                .expect_err("a lock written on another host must never be cleared from here");
            assert!(error.contains(&foreign_host), "{error}");
            assert!(error.contains(&dead_pid.to_string()), "{error}");
            assert!(std::fs::symlink_metadata(&lock).is_ok());
        }
    }

    #[test]
    fn relaunch_backoff_is_bounded() {
        let now = Instant::now();
        let mut budget = RelaunchBudget::default();
        assert!(budget.may_attempt(now));
        assert_eq!(budget.record_failure(now), Duration::from_secs(5));
        assert!(!budget.may_attempt(now));
        assert!(budget.may_attempt(now + Duration::from_secs(5)));
        assert_eq!(
            budget.record_failure(now + Duration::from_secs(5)),
            Duration::from_secs(10)
        );
        assert_eq!(
            budget.record_failure(now + Duration::from_secs(15)),
            Duration::from_secs(20)
        );
        assert!(budget.exhausted());
        assert!(!budget.may_attempt(now + Duration::from_secs(3600)));
        budget.record_success();
        assert_eq!(budget, RelaunchBudget::default());

        let mut many = RelaunchBudget::default();
        for _ in 0..40 {
            many.failures = 0;
            let _ = many.record_failure(now);
        }
        many.failures = 20;
        assert!(many.record_failure(now) <= RELAUNCH_BACKOFF_MAX);
    }

    #[test]
    fn finished_chat_is_retired_without_relaunch() {
        let now = Instant::now();
        let fresh = RelaunchBudget::default();
        assert!(matches!(
            decide_recovery(Some(false), false, &fresh, now),
            RecoveryDecision::Retire(reason) if reason.contains("idle")
        ));
        assert!(matches!(
            decide_recovery(None, false, &fresh, now),
            RecoveryDecision::Retire(reason) if reason.contains("not loaded")
        ));
        assert_eq!(
            decide_recovery(Some(true), false, &fresh, now),
            RecoveryDecision::Relaunch
        );
        assert_eq!(
            decide_recovery(Some(false), true, &fresh, now),
            RecoveryDecision::Relaunch
        );

        let mut waiting = RelaunchBudget::default();
        waiting.record_failure(now);
        assert_eq!(
            decide_recovery(Some(true), false, &waiting, now),
            RecoveryDecision::Wait
        );
        assert_eq!(
            decide_recovery(Some(true), false, &waiting, now + Duration::from_secs(5)),
            RecoveryDecision::Relaunch
        );

        let mut exhausted = RelaunchBudget::default();
        for _ in 0..RELAUNCH_MAX_FAILURES {
            exhausted.record_failure(now);
        }
        assert!(matches!(
            decide_recovery(Some(true), true, &exhausted, now + Duration::from_secs(3600)),
            RecoveryDecision::Retire(reason) if reason.contains("3 times")
        ));
        assert!(chat_is_disposable("subchat-abc"));
        assert!(!chat_is_disposable("chat-abc"));
    }

    #[test]
    fn exact_chat_id_match_wins_over_every_fallback() {
        let owners = vec![
            owner("rt-root", Some("root-chat")),
            owner("rt-exact", Some("subchat-1")),
        ];
        let lookup = ChatRuntimeLookup {
            chat_id: "subchat-1".to_string(),
            root_chat_id: Some("root-chat".to_string()),
            background_agent_id: Some("bgagent-1".to_string()),
        };
        let selection = select_runtime_for_chat(&owners, &lookup).unwrap();
        assert_eq!(selection.runtime_id, "rt-exact");
        assert!(!selection.adopted);
    }

    #[test]
    fn root_chat_id_is_tried_before_background_agent_id() {
        let owners = vec![
            owner("rt-agent", Some("bgagent-1")),
            owner("rt-root", Some("root-chat")),
        ];
        let lookup = ChatRuntimeLookup {
            chat_id: "subchat-new".to_string(),
            root_chat_id: Some("root-chat".to_string()),
            background_agent_id: Some("bgagent-1".to_string()),
        };
        let selection = select_runtime_for_chat(&owners, &lookup).unwrap();
        assert_eq!(selection.runtime_id, "rt-root");
        assert!(selection.adopted);
    }

    #[test]
    fn background_agent_id_is_the_last_named_fallback() {
        let owners = vec![owner("rt-agent", Some("bgagent-1"))];
        let lookup = ChatRuntimeLookup {
            chat_id: "subchat-new".to_string(),
            root_chat_id: Some("root-chat".to_string()),
            background_agent_id: Some("bgagent-1".to_string()),
        };
        let selection = select_runtime_for_chat(&owners, &lookup).unwrap();
        assert_eq!(selection.runtime_id, "rt-agent");
        assert!(selection.adopted);
    }

    #[test]
    fn a_sole_runtime_owned_by_a_stranger_is_not_adopted() {
        let owners = vec![owner("rt-other", Some("some-other-chat"))];
        let lookup = ChatRuntimeLookup::for_chat("subchat-new");
        assert_eq!(select_runtime_for_chat(&owners, &lookup), None);
    }

    #[test]
    fn ambiguous_runtimes_are_never_adopted() {
        let owners = vec![owner("rt-a", None), owner("rt-b", None)];
        let lookup = ChatRuntimeLookup {
            chat_id: "subchat-new".to_string(),
            root_chat_id: Some("root-chat".to_string()),
            background_agent_id: Some("bgagent-1".to_string()),
        };
        assert_eq!(select_runtime_for_chat(&owners, &lookup), None);
    }

    #[test]
    fn profile_lock_is_not_live_for_a_dead_pid() {
        let dir = tempfile::tempdir().unwrap();
        let dead_pid = 4_294_000_001u32;
        assert!(!pid_is_alive(dead_pid));
        // Chrome only writes the SingletonLock symlink on unix, so the lock-derived assertions
        // cannot run elsewhere; liveness itself is checked on every platform above.
        #[cfg(unix)]
        {
            let host = local_hostname();
            std::os::unix::fs::symlink(
                format!("{host}-{dead_pid}"),
                dir.path().join("SingletonLock"),
            )
            .unwrap();
            assert_eq!(
                profile_lock_state(dir.path()),
                ProfileLockState::Stale(ProfileLock {
                    host,
                    pid: dead_pid
                })
            );
            assert_eq!(profile_lock_is_live(dir.path()), None);
        }
    }

    #[test]
    fn profile_lock_is_not_live_for_an_unparseable_lock() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("garbage", dir.path().join("SingletonLock")).unwrap();
        assert_eq!(profile_lock_is_live(dir.path()), None);
    }

    #[test]
    fn profile_lock_is_not_live_when_the_pid_is_not_the_profile_owner() {
        let dir = tempfile::tempdir().unwrap();
        let own_pid = std::process::id();
        assert!(pid_is_alive(own_pid));
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            format!("{}-{own_pid}", local_hostname()),
            dir.path().join("SingletonLock"),
        )
        .unwrap();
        assert_eq!(profile_lock_is_live(dir.path()), None);

        #[cfg(target_os = "linux")]
        {
            struct Holder(std::process::Child);
            impl Drop for Holder {
                fn drop(&mut self) {
                    let _ = self.0.kill();
                    let _ = self.0.wait();
                }
            }
            let profile = dir.path().join("abc");
            std::fs::create_dir(&profile).unwrap();
            let holder = |user_data_dir: String, ready: &Path| {
                Holder(
                    std::process::Command::new("sh")
                        .args(["-c", "printf ready > \"$1\"; read reply", "sh"])
                        .arg(ready)
                        .arg(format!("--user-data-dir={user_data_dir}"))
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                        .unwrap(),
                )
            };
            let lock = profile.join("SingletonLock");
            for (user_data_dir, owns) in [
                (format!("{}-sub", profile.display()), false),
                (profile.display().to_string(), true),
            ] {
                let ready = dir.path().join(format!("holder-ready-{owns}"));
                let mut child = holder(user_data_dir.clone(), &ready);
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while std::fs::read_to_string(&ready).ok().as_deref() != Some("ready") {
                    assert!(child.0.try_wait().unwrap().is_none());
                    assert!(
                        std::time::Instant::now() < deadline,
                        "profile holder not ready"
                    );
                    std::thread::yield_now();
                }
                let pid = child.0.id();
                let _ = std::fs::remove_file(&lock);
                std::os::unix::fs::symlink(format!("{}-{pid}", local_hostname()), &lock).unwrap();
                let live = profile_lock_is_live(&profile);
                assert_eq!(live, owns.then_some(pid), "--user-data-dir={user_data_dir}");
            }
        }
    }

    #[test]
    fn a_launch_is_retried_once_only_for_a_startup_death_on_a_free_profile() {
        let stale = ProfileLockState::Stale(ProfileLock {
            host: "here".to_string(),
            pid: 7,
        });
        let live = ProfileLockState::Live(ProfileLock {
            host: "here".to_string(),
            pid: 7,
        });
        let foreign = ProfileLockState::Foreign(ProfileLock {
            host: "there".to_string(),
            pid: 7,
        });
        let closed = "Unable to make method calls because underlying connection is closed";
        let no_port = "There are no available ports between 8000 and 9000 for debugging";
        for error in [closed, no_port] {
            assert!(should_retry_launch(error, 1, &stale), "{error}");
            assert!(
                should_retry_launch(error, 1, &ProfileLockState::Absent),
                "{error}"
            );
            assert!(!should_retry_launch(error, 2, &stale), "{error}");
            assert!(!should_retry_launch(error, 1, &live), "{error}");
            assert!(!should_retry_launch(error, 1, &foreign), "{error}");
        }
        assert!(!should_retry_launch(
            "Failed to create profile dir: permission denied",
            1,
            &stale
        ));
    }

    #[test]
    fn stale_sweep_keeps_recent_and_non_subchat_profiles() {
        let cache = tempfile::tempdir().unwrap();
        let root = cache.path().join("browser_profiles");
        let recent = root.join("subchat-recent");
        let keep = root.join("chat-keep");
        std::fs::create_dir_all(&recent).unwrap();
        std::fs::create_dir_all(&keep).unwrap();
        assert_eq!(sweep_stale_browser_profiles(cache.path()), 0);
        assert!(recent.exists());
        assert!(keep.exists());
    }

    #[test]
    fn regular_profiles_have_seven_day_retention() {
        assert_eq!(profile_min_age("subchat-one"), Duration::from_secs(600));
        assert_eq!(
            profile_min_age("chat-one"),
            Duration::from_secs(7 * 24 * 60 * 60)
        );
    }
}
