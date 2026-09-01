use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use tokio::sync::Mutex as AMutex;
use tracing::{info, warn};

pub use refact_browser::*;

use crate::http::routers::v1::browser_settings;

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

pub async fn register_browser_runtime(
    app: crate::app_state::AppState,
    runtime: BrowserRuntime,
) -> String {
    let mut runtime = runtime;
    runtime.set_allowed_roots(workspace_roots(&app));
    let runtime_id = runtime.runtime_id.clone();
    let arc = Arc::new(AMutex::new(runtime));
    app.integrations
        .browser_runtimes
        .lock()
        .await
        .insert(runtime_id.clone(), arc);
    runtime_id
}

pub const PROCESS_TERM_GRACE: Duration = Duration::from_millis(1500);

pub fn pid_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
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

fn process_cmdline(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        Some(
            raw.split(|byte| *byte == 0)
                .map(String::from_utf8_lossy)
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

fn process_owns_profile(pid: u32, profile_dir: &Path) -> bool {
    let Some(cmdline) = process_cmdline(pid) else {
        return false;
    };
    cmdline.contains(&*profile_dir.to_string_lossy())
}

fn shutdown_runtime_process(pid: Option<u32>, profile_dir: &Path, delete_profile: bool) {
    if let Some(pid) = pid.filter(|pid| pid_is_alive(*pid)) {
        terminate_process(pid);
    }
    if let Some(lock_pid) = profile_lock_is_live(profile_dir) {
        terminate_process(lock_pid);
    }
    if !delete_profile {
        return;
    }
    if profile_lock_is_live(profile_dir).is_some() {
        warn!(
            "Not deleting browser profile {}: it is still locked by a live process",
            profile_dir.display()
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

async fn take_browser_runtime(
    app: &crate::app_state::AppState,
    runtime_id: &str,
    delete_profile: bool,
) -> Option<Arc<AMutex<BrowserRuntime>>> {
    let removed = app
        .integrations
        .browser_runtimes
        .lock()
        .await
        .remove(runtime_id)?;
    let (pid, profile_dir) = {
        let rt = removed.lock().await;
        (rt.browser.get_process_id(), rt.profile_dir.clone())
    };
    let runtime_id = runtime_id.to_string();
    if let Err(error) = tokio::task::spawn_blocking(move || {
        shutdown_runtime_process(pid, &profile_dir, delete_profile)
    })
    .await
    {
        warn!("Browser shutdown task for runtime {runtime_id} failed: {error}");
    }
    Some(removed)
}

pub async fn remove_browser_runtime(
    app: crate::app_state::AppState,
    runtime_id: &str,
) -> Option<Arc<AMutex<BrowserRuntime>>> {
    take_browser_runtime(&app, runtime_id, false).await
}

pub async fn discard_browser_runtime(
    app: crate::app_state::AppState,
    runtime_id: &str,
) -> Option<Arc<AMutex<BrowserRuntime>>> {
    take_browser_runtime(&app, runtime_id, true).await
}

pub const RUNTIME_RELEASE_TIMEOUT: Duration = Duration::from_secs(10);

async fn release_runtime_and_wait(removed: Option<Arc<AMutex<BrowserRuntime>>>, runtime_id: &str) {
    let Some(removed) = removed else {
        return;
    };
    let deadline = std::time::Instant::now() + RUNTIME_RELEASE_TIMEOUT;
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
    tokio::task::block_in_place(move || drop(removed));
}

fn profile_lock_pid(profile_dir: &Path) -> Option<u32> {
    let target = std::fs::read_link(profile_dir.join("SingletonLock")).ok()?;
    let target = target.to_string_lossy();
    let (_, pid) = target.rsplit_once('-')?;
    pid.parse::<u32>().ok()
}

pub fn profile_lock_is_live(profile_dir: &Path) -> Option<u32> {
    let pid = profile_lock_pid(profile_dir)?;
    if pid == 0 || !pid_is_alive(pid) {
        return None;
    }
    process_owns_profile(pid, profile_dir).then_some(pid)
}

pub const STALE_PROFILE_MIN_AGE: Duration = Duration::from_secs(600);
pub const STALE_PROFILE_SWEEP_LIMIT: usize = 256;

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
            let Some(cmdline) = process_cmdline(pid) else {
                continue;
            };
            for arg in cmdline.split_whitespace() {
                if let Some(dir) = arg.strip_prefix("--user-data-dir=") {
                    dirs.push(dir.to_string());
                }
            }
        }
        Some(dirs)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

pub fn sweep_stale_browser_profiles(cache_dir: &Path) -> usize {
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
        if !name.starts_with("subchat-") || !path.is_dir() {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        if live_dirs.iter().any(|dir| dir == &path_str) {
            continue;
        }
        if profile_lock_is_live(&path).is_some() {
            continue;
        }
        let recently_used = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .map(|modified| {
                modified
                    .elapsed()
                    .map(|age| age < STALE_PROFILE_MIN_AGE)
                    .unwrap_or(true)
            })
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

    fn owned_by_ancestor(&self, owner: &str) -> bool {
        self.ordered_ids().iter().any(|id| id == owner)
            || (!owner.is_empty() && self.chat_id.starts_with(owner))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOwner {
    pub runtime_id: String,
    pub attached_chat_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSelection {
    pub runtime_id: String,
    pub adopted: bool,
}

pub fn select_runtime_for_chat(
    owners: &[RuntimeOwner],
    lookup: &ChatRuntimeLookup,
) -> Option<RuntimeSelection> {
    for id in lookup.ordered_ids() {
        if let Some(owner) = owners
            .iter()
            .find(|owner| owner.attached_chat_id.as_deref() == Some(id.as_str()))
        {
            return Some(RuntimeSelection {
                runtime_id: owner.runtime_id.clone(),
                adopted: id != lookup.chat_id,
            });
        }
    }
    let only = match owners {
        [only] => only,
        _ => return None,
    };
    let adoptable = match only.attached_chat_id.as_deref() {
        None => true,
        Some(owner) => owner.is_empty() || lookup.owned_by_ancestor(owner),
    };
    adoptable.then(|| RuntimeSelection {
        runtime_id: only.runtime_id.clone(),
        adopted: true,
    })
}

pub async fn find_runtime_for_chat(
    app: crate::app_state::AppState,
    lookup: &ChatRuntimeLookup,
) -> Option<(String, Arc<AMutex<BrowserRuntime>>)> {
    let runtime_arcs: Vec<(String, Arc<AMutex<BrowserRuntime>>)> = {
        let browser_runtimes = app.integrations.browser_runtimes.clone();
        let browser_runtimes = browser_runtimes.lock().await;
        browser_runtimes
            .iter()
            .map(|(rid, arc)| (rid.clone(), arc.clone()))
            .collect()
    };
    let mut owners = Vec::with_capacity(runtime_arcs.len());
    for (rid, arc) in &runtime_arcs {
        let rt = arc.lock().await;
        owners.push(RuntimeOwner {
            runtime_id: rid.clone(),
            attached_chat_id: rt.attached_chat_id.clone(),
        });
    }
    let selection = select_runtime_for_chat(&owners, lookup)?;
    let (rid, arc) = runtime_arcs
        .into_iter()
        .find(|(rid, _)| *rid == selection.runtime_id)?;
    if selection.adopted {
        arc.lock().await.reattach(&lookup.chat_id);
    }
    Some((rid, arc))
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

pub fn relaunch_resume_warning(resume_index: usize) -> String {
    format!(
        "browser session was dead; relaunched and resumed from step {resume_index} — open tabs were lost, cookies/storage persist via profile"
    )
}

static RELAUNCH_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<AMutex<()>>>>> = OnceLock::new();

fn relaunch_lock_for_chat(chat_id: &str) -> Arc<AMutex<()>> {
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

pub async fn relaunch_runtime_for_chat(
    app: crate::app_state::AppState,
    chat_id: &str,
    profile_dir: PathBuf,
    launch_options: BrowserLaunchOptions,
    window_bounds: Option<refact_chat_api::WindowBounds>,
) -> Result<String, String> {
    let options = BrowserLaunchOptions {
        window_bounds: window_bounds.or(launch_options.window_bounds.clone()),
        ..launch_options
    };

    let relaunch_lock = relaunch_lock_for_chat(chat_id);
    let _relaunch_guard = relaunch_lock.lock().await;

    let previous_emitter_active = match find_runtime_by_chat_id(app.clone(), chat_id).await {
        Some((runtime_id, runtime_arc)) => {
            let (emitter_active, usable) = {
                let mut rt = runtime_arc.lock().await;
                let same_mode = rt.launch_options.headless == options.headless;
                let usable = same_mode && tokio::task::block_in_place(|| rt.check_connection());
                (rt.frame_emitter_active, usable)
            };
            if usable {
                info!(
                    "BrowserRuntime {} is already live for chat {}, reusing it instead of relaunching",
                    runtime_id, chat_id
                );
                return Ok(runtime_id);
            }
            drop(runtime_arc);
            let removed = remove_browser_runtime(app.clone(), &runtime_id).await;
            release_runtime_and_wait(removed, &runtime_id).await;
            tokio::time::sleep(browser_settings::current().relaunch_settle()).await;
            emitter_active
        }
        None => false,
    };

    let mode = options.mode_label();
    let mut runtime = BrowserRuntime::launch(profile_dir.clone(), options).map_err(|e| {
        format!(
            "Failed to relaunch browser in {} mode: {}",
            mode,
            describe_launch_failure(&e, &profile_dir)
        )
    })?;
    runtime.reattach(chat_id);
    let runtime_id = register_browser_runtime(app.clone(), runtime).await;

    let runtime_arc = {
        let browser_runtimes = app.integrations.browser_runtimes.clone();
        let browser_runtimes = browser_runtimes.lock().await;
        browser_runtimes.get(&runtime_id).cloned()
    };
    if let Some(runtime_arc) = runtime_arc {
        let mut rt = runtime_arc.lock().await;
        if let Err(e) = setup_recording_for_runtime(&mut rt) {
            warn!(
                "Browser recording setup failed after {} relaunch (non-fatal): {}",
                mode, e
            );
        }
    }

    if previous_emitter_active {
        ensure_frame_emitter(app, chat_id, &runtime_id).await;
    }

    info!(
        "BrowserRuntime {} relaunched ({}) for chat {}",
        runtime_id, mode, chat_id
    );
    Ok(runtime_id)
}

pub async fn browser_snapshot_for_chat(
    app: crate::app_state::AppState,
    chat_id: &str,
) -> Option<crate::chat::types::BrowserSnapshot> {
    let (runtime_id, runtime_arc) = find_runtime_by_chat_id(app, chat_id).await?;
    let mut rt = runtime_arc.lock().await;
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

struct RuntimeHealth {
    chat_id: Option<String>,
    was_connected: bool,
    still_connected: bool,
    idle_expired: bool,
    idle_timeout: Duration,
    profile_dir: PathBuf,
    launch_options: BrowserLaunchOptions,
}

pub async fn browser_monitor_background_task(app: crate::app_state::AppState) {
    let cache_dir = app.gcx.cache_dir.clone();
    match tokio::task::spawn_blocking(move || sweep_stale_browser_profiles(&cache_dir)).await {
        Ok(removed) if removed > 0 => info!("Swept {removed} stale browser profile dirs"),
        Ok(_) => {}
        Err(error) => warn!("Stale browser profile sweep failed: {error}"),
    }
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

        let mut to_remove: Vec<(String, bool)> = Vec::new();
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

            let RuntimeHealth {
                chat_id,
                was_connected,
                still_connected,
                idle_expired,
                idle_timeout,
                profile_dir,
                launch_options,
            } = {
                let mut rt = runtime_arc.lock().await;
                tokio::task::block_in_place(|| {
                    refact_browser::adopt_new_tabs(&mut rt, None);
                    let was_connected = rt.is_connected;
                    let still_connected = rt.check_connection();
                    RuntimeHealth {
                        chat_id: rt.attached_chat_id.clone(),
                        was_connected,
                        still_connected,
                        idle_expired: rt.is_idle_expired(),
                        idle_timeout: rt.idle_timeout,
                        profile_dir: rt.profile_dir.clone(),
                        launch_options: rt.launch_options.clone(),
                    }
                })
            };

            if was_connected && !still_connected {
                info!(
                    "BrowserRuntime {} (chat {:?}) lost connection",
                    rid, chat_id
                );
            }

            let evict_idle =
                idle_expired && (chat_id.is_none() || launch_options.evict_idle_attached);
            if evict_idle {
                warn!(
                    "BrowserRuntime {} idle timeout ({:?}) for chat {:?}",
                    rid, idle_timeout, chat_id
                );
                to_remove.push((rid.clone(), true));
                continue;
            }

            if !still_connected {
                match chat_id {
                    Some(chat_id) => to_relaunch.push(RuntimeRecoveryPlan {
                        runtime_id: rid.clone(),
                        chat_id,
                        profile_dir,
                        launch_options,
                    }),
                    None => to_remove.push((rid.clone(), true)),
                }
            }
        }

        for (rid, delete_profile) in to_remove {
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
            if let Err(error) = relaunch_runtime_for_chat(
                app.clone(),
                &plan.chat_id,
                plan.profile_dir.clone(),
                plan.launch_options.clone(),
                None,
            )
            .await
            {
                warn!(
                    "Failed to relaunch dead BrowserRuntime {} for chat {}: {}",
                    plan.runtime_id, plan.chat_id, error
                );
                remove_browser_runtime(app.clone(), &plan.runtime_id).await;
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
    fn monitor_evicts_dead_runtimes_regardless_of_chat_attachment() {
        let source = include_str!("browser_runtime.rs");
        let monitor = source
            .split_once("pub async fn browser_monitor_background_task(")
            .unwrap()
            .1
            .split_once("\n#[cfg(test)]")
            .unwrap()
            .0;

        assert!(
            !monitor.contains("if !still_connected && rt.attached_chat_id.is_none()"),
            "dead runtimes attached to a chat are still leaked forever"
        );
        assert!(monitor.contains("if !still_connected {"));
        assert!(monitor.contains("Some(chat_id) => to_relaunch.push(RuntimeRecoveryPlan {"));
        assert!(monitor.contains("None => to_remove.push((rid.clone(), true)),"));
        assert!(monitor.contains("relaunch_runtime_for_chat("));
        assert!(
            monitor.contains("tokio::task::block_in_place(|| {"),
            "sync CDP calls still run on a worker thread while holding the runtime mutex"
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

    #[test]
    fn relaunch_reuses_a_live_runtime_in_the_requested_mode() {
        let helper = include_str!("browser_runtime.rs")
            .split_once("pub async fn relaunch_runtime_for_chat(")
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0;

        for step in [
            "let relaunch_lock = relaunch_lock_for_chat(chat_id);",
            "let _relaunch_guard = relaunch_lock.lock().await;",
            "let same_mode = rt.launch_options.headless == options.headless;",
            "tokio::task::block_in_place(|| rt.check_connection())",
            "return Ok(runtime_id);",
        ] {
            assert!(helper.contains(step), "relaunch guard lost step: {step}");
        }
        assert!(
            helper.find("let _relaunch_guard").unwrap()
                < helper.find("remove_browser_runtime(").unwrap(),
            "the per-chat guard must be held across remove, launch and register"
        );
    }

    #[test]
    fn resume_warning_names_the_step_the_retry_starts_from() {
        let warning = relaunch_resume_warning(2);
        assert!(warning.contains("relaunched and resumed from step 2"));
        assert!(warning.contains("open tabs were lost"));
        assert!(warning.contains("cookies/storage persist via profile"));
    }

    #[test]
    fn relaunch_sequence_lives_only_in_the_shared_helper() {
        let helper = include_str!("browser_runtime.rs")
            .split_once("pub async fn relaunch_runtime_for_chat(")
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0;

        for step in [
            "remove_browser_runtime(app.clone(), &runtime_id)",
            "tokio::time::sleep(browser_settings::current().relaunch_settle())",
            "BrowserRuntime::launch(",
            "runtime.reattach(chat_id)",
            "register_browser_runtime(app.clone(), runtime)",
            "setup_recording_for_runtime(&mut rt)",
            "ensure_frame_emitter(app, chat_id, &runtime_id)",
        ] {
            assert!(helper.contains(step), "relaunch helper lost step: {step}");
        }

        let router = include_str!("../http/routers/v1/v1_browser.rs");
        assert!(router.contains("relaunch_runtime_for_chat("));
        assert!(
            !router.contains("Failed to relaunch browser in {} mode"),
            "router still duplicates the relaunch sequence"
        );

        let controller = include_str!("browser_controller.rs");
        assert!(controller.contains("relaunch_runtime_for_chat("));
        assert!(!controller.contains("BrowserRuntime::launch("));
    }

    #[test]
    fn relaunch_preserves_profile_launch_options_and_window_bounds() {
        let helper = include_str!("browser_runtime.rs")
            .split_once("pub async fn relaunch_runtime_for_chat(")
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0;

        assert!(helper
            .contains("window_bounds: window_bounds.or(launch_options.window_bounds.clone())"));
        assert!(helper.contains("..launch_options"));
        assert!(RELAUNCH_WARNING.contains("relaunched and retried"));
        assert!(RELAUNCH_WARNING.contains("open tabs lost"));
    }

    #[test]
    fn setup_chrome_session_cannot_hand_back_the_dead_runtime() {
        let session = include_str!("../tools/tool_chrome.rs")
            .split_once("async fn setup_chrome_session(")
            .unwrap()
            .1;
        let unhealthy = session
            .split_once("if runtime_healthy {")
            .unwrap()
            .1
            .split_once("find_runtime_by_chat_id(")
            .unwrap()
            .0;

        assert!(
            unhealthy.contains("remove_browser_runtime("),
            "dead runtime is not evicted before re-resolving it by chat id"
        );
    }

    fn owner(runtime_id: &str, attached: Option<&str>) -> RuntimeOwner {
        RuntimeOwner {
            runtime_id: runtime_id.to_string(),
            attached_chat_id: attached.map(str::to_string),
        }
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
    fn a_sole_unclaimed_runtime_is_adopted() {
        let owners = vec![owner("rt-free", None)];
        let lookup = ChatRuntimeLookup::for_chat("subchat-new");
        let selection = select_runtime_for_chat(&owners, &lookup).unwrap();
        assert_eq!(selection.runtime_id, "rt-free");
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
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            format!("hostname-{dead_pid}"),
            dir.path().join("SingletonLock"),
        )
        .unwrap();
        assert_eq!(profile_lock_pid(dir.path()), Some(dead_pid));
        assert_eq!(profile_lock_is_live(dir.path()), None);
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
            format!("hostname-{own_pid}"),
            dir.path().join("SingletonLock"),
        )
        .unwrap();
        assert_eq!(profile_lock_is_live(dir.path()), None);
    }

    #[test]
    fn launch_failure_text_never_blames_ports_alone() {
        let dir = tempfile::tempdir().unwrap();
        let described =
            describe_launch_failure("no available ports between 8000 and 9000", dir.path());
        assert!(described.contains("Chrome exited before reporting a debugging"));
        assert!(described.contains("ports are not exhausted"));
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
    fn runtime_removal_kills_chrome_and_can_delete_the_profile() {
        let source = include_str!("browser_runtime.rs");
        let helper = source
            .split_once("fn shutdown_runtime_process(")
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0;
        assert!(helper.contains("terminate_process(pid)"));
        assert!(helper.contains("std::fs::remove_dir_all(profile_dir)"));

        let terminate = source
            .split_once("fn terminate_process(")
            .unwrap()
            .1
            .split_once("\n}\n")
            .unwrap()
            .0;
        assert!(terminate.contains("libc::SIGTERM"));
        assert!(terminate.contains("libc::SIGKILL"));
    }

    #[test]
    fn test_get_browser_profile_dir_different_threads() {
        let cache_dir = PathBuf::from("/home/user/.cache/refact");
        let p1 = get_browser_profile_dir(&cache_dir, "thread-1");
        let p2 = get_browser_profile_dir(&cache_dir, "thread-2");
        assert_ne!(p1, p2);
        assert!(p1.to_str().unwrap().contains("thread-1"));
        assert!(p2.to_str().unwrap().contains("thread-2"));
    }
}
