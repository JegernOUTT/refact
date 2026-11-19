//! The default browser is one durable Chromium profile and one live runtime per trusted
//! project. Its runtime owner is `project-browser:<project id>`, never a chat id, so the
//! chat-keyed lookups cannot return it; callers resolve the project from trusted session
//! state first. Named and explicitly isolated sessions do not use this module.
use super::*;

const OWNER_PREFIX: &str = "project-browser:";
const PROJECT_PROFILES_DIR: &str = "project_browser_profiles";
const LEGACY_PROFILES_DIR: &str = "browser_profiles";
pub const PROJECT_ENSURE_TIMEOUT: Duration = Duration::from_secs(120);

/// Launch flags that would move the profile, open a debugging endpoint or make the session
/// ephemeral are never accepted for the shared project profile.
const FORBIDDEN_PROJECT_ARG_PREFIXES: &[&str] = &[
    "--user-data-dir",
    "--profile-directory",
    "--remote-debugging",
    "--incognito",
    "--guest",
    "--temp-profile",
    "--disk-cache-dir",
    "--no-restore-last-session",
    "--no-startup-window",
];

fn redact_profile_path(error: &str, profile_dir: &Path) -> String {
    error.replace(
        &profile_dir.display().to_string(),
        "<project browser profile>",
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBrowserScope {
    project_id: String,
    source_root: PathBuf,
}

impl ProjectBrowserScope {
    pub fn from_source_root(root: &Path) -> Result<Self, String> {
        let canonical = std::fs::canonicalize(root)
            .map_err(|error| format!("project root is not accessible: {error}"))?;
        let canonical = dunce::simplified(&canonical).to_path_buf();
        if !canonical.is_dir() {
            return Err("project root is not a directory".into());
        }
        Ok(Self {
            project_id: crate::coordination::roots::project_identity(&canonical),
            source_root: canonical,
        })
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn source_root(&self) -> &Path {
        &self.source_root
    }

    pub fn owner_id(&self) -> String {
        format!("{OWNER_PREFIX}{}", self.project_id)
    }

    pub fn profile_dir(&self, cache_dir: &Path) -> PathBuf {
        cache_dir.join(PROJECT_PROFILES_DIR).join(&self.project_id)
    }
}

pub fn is_project_owner_id(owner: &str) -> bool {
    owner.starts_with(OWNER_PREFIX)
}

/// The project of a live chat session: its worktree's source root, else the root the
/// session was bound to on creation or restore. An unknown chat id or a session without a
/// bound project is refused; there is no fallback to the currently selected workspace.
pub async fn project_scope_for_chat(
    app: &crate::app_state::AppState,
    chat_id: &str,
) -> Result<ProjectBrowserScope, String> {
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("project browser requires a live chat session")?;
    let root = {
        let session = session.lock().await;
        crate::coordination::scope::session_source_root(&session)
    }
    .ok_or("chat has no trusted project root")?;
    ProjectBrowserScope::from_source_root(&root)
}

/// The engine's currently selected project, resolved exactly as a new chat session binds
/// its `work_project_root`, so a fresh chat and this scope agree on the canonical root.
pub async fn current_project_scope(
    app: &crate::app_state::AppState,
) -> Result<ProjectBrowserScope, String> {
    let root =
        crate::coordination::roots::resolve_source_workspace_root(app.gcx.clone(), None).await?;
    ProjectBrowserScope::from_source_root(&root)
}

/// A daemon-managed worker serves exactly the project it was started for: its pinned CLI root.
/// `None` for direct workers.
pub fn pinned_project_scope(
    app: &crate::app_state::AppState,
) -> Option<Result<ProjectBrowserScope, String>> {
    let cmdline = &app.gcx.cmdline;
    let managed = !cmdline.daemon_endpoint.is_empty()
        && !cmdline.project_id.is_empty()
        && !cmdline.workspace_folder.is_empty();
    managed.then(|| ProjectBrowserScope::from_source_root(Path::new(&cmdline.workspace_folder)))
}

/// The source root a tool context is bound to. `Sessionless` is a context without an execution
/// scope whose chat and root chat have no live session (e.g. a tool run over HTTP outside any
/// chat); `Unbound` is a live session that recorded no project root.
enum ContextRoot {
    Sessionless,
    Unbound,
    Bound(PathBuf),
}

async fn context_root(
    app: &crate::app_state::AppState,
    execution_scope: Option<&crate::worktrees::scope::ExecutionScope>,
    chat_id: &str,
    root_chat_id: &str,
) -> ContextRoot {
    if let Some(scope) = execution_scope {
        return ContextRoot::Bound(scope.source_workspace_root().to_path_buf());
    }
    let mut live = false;
    for id in [chat_id, root_chat_id] {
        if id.is_empty() {
            continue;
        }
        let session = app.chat.sessions.read().await.get(id).cloned();
        if let Some(session) = session {
            live = true;
            let root = crate::coordination::scope::session_source_root(&*session.lock().await);
            if let Some(root) = root {
                return ContextRoot::Bound(root);
            }
        }
    }
    if live {
        ContextRoot::Unbound
    } else {
        ContextRoot::Sessionless
    }
}

/// Tool contexts: the execution scope's source root wins, then the chat, then its root chat.
/// `None` keeps the per-chat browser: always for sessionless contexts, and in direct workers for
/// a context without an accessible project root. A daemon-managed worker serves exactly its
/// pinned project: a live chat without a recorded root uses it, while a root that is foreign or
/// no longer accessible, or an inaccessible pin, is refused rather than given another browser.
pub async fn project_scope_for_context(
    app: &crate::app_state::AppState,
    execution_scope: Option<&crate::worktrees::scope::ExecutionScope>,
    chat_id: &str,
    root_chat_id: &str,
) -> Result<Option<ProjectBrowserScope>, String> {
    let Some(pinned) = pinned_project_scope(app) else {
        let scope = match execution_scope {
            Some(scope) => ProjectBrowserScope::from_source_root(scope.source_workspace_root()),
            None => match project_scope_for_chat(app, chat_id).await {
                Ok(scope) => Ok(scope),
                Err(error) if root_chat_id.is_empty() || root_chat_id == chat_id => Err(error),
                Err(_) => project_scope_for_chat(app, root_chat_id).await,
            },
        };
        return Ok(scope.ok());
    };
    let root = context_root(app, execution_scope, chat_id, root_chat_id).await;
    if matches!(root, ContextRoot::Sessionless) {
        return Ok(None);
    }
    let pinned = pinned.map_err(|_| {
        "the project browser is unavailable: this worker's project root is no longer accessible"
            .to_string()
    })?;
    match root {
        ContextRoot::Sessionless | ContextRoot::Unbound => Ok(Some(pinned)),
        ContextRoot::Bound(root) => match ProjectBrowserScope::from_source_root(&root) {
            Err(_) => Err(
                "the project browser is unavailable: this chat's project root is no longer accessible"
                    .into(),
            ),
            Ok(scope) if scope != pinned => Err(
                "the project browser is unavailable: this chat belongs to a different project than this worker serves; open the chat in its own project"
                    .into(),
            ),
            Ok(_) => Ok(Some(pinned)),
        },
    }
}

pub fn project_launch_options(options: BrowserLaunchOptions) -> BrowserLaunchOptions {
    BrowserLaunchOptions {
        headless: true,
        restore_last_session: true,
        extra_args: options
            .extra_args
            .into_iter()
            .filter(|arg| {
                let lowered = arg.trim().to_ascii_lowercase();
                !FORBIDDEN_PROJECT_ARG_PREFIXES
                    .iter()
                    .any(|prefix| lowered.starts_with(prefix))
            })
            .collect(),
        evict_idle_attached: false,
        ..options
    }
}

pub async fn find_project_runtime(
    app: &crate::app_state::AppState,
    scope: &ProjectBrowserScope,
) -> Option<(String, Arc<AMutex<BrowserRuntime>>)> {
    find_runtime_by_exact_owner(app, &scope.owner_id()).await
}

pub struct EnsuredProjectRuntime {
    pub runtime_id: String,
    pub runtime: Arc<AMutex<BrowserRuntime>>,
    pub launched: bool,
}

async fn runtime_arc(
    app: &crate::app_state::AppState,
    runtime_id: &str,
) -> Result<Arc<AMutex<BrowserRuntime>>, String> {
    app.integrations
        .browser_runtimes
        .lock()
        .await
        .get(runtime_id)
        .cloned()
        .ok_or_else(|| format!("project BrowserRuntime {runtime_id} disappeared"))
}

/// Returns the project's runtime, launching it on the durable project profile when none is
/// registered. A live disconnected runtime qualifies only for same-process reconnect;
/// a runtime whose process died is relaunched on the same profile. An unresolved profile is
/// resolved first by adopting the most recently used inactive earlier chat profile, if any.
pub async fn ensure_project_runtime(
    app: &crate::app_state::AppState,
    scope: &ProjectBrowserScope,
    options: BrowserLaunchOptions,
    deadline: Instant,
) -> Result<EnsuredProjectRuntime, String> {
    let owner = scope.owner_id();
    let lock = relaunch_lock_for_chat(&owner);
    let _guard = before_deadline(deadline, lock.lock()).await?;
    if let Some((existing_id, existing)) =
        before_deadline(deadline, find_project_runtime(app, scope)).await?
    {
        let record = runtime_owner_record(&existing_id)
            .ok_or_else(|| "project browser ownership record missing".to_string())?;
        if record.process_id().is_some_and(pid_is_alive) {
            if !record.is_connected() {
                reconnect_owned_runtime_before_deadline(&existing_id, &existing, &owner, deadline)
                    .await
                    .map_err(|error| redact_profile_path(&error, record.profile_dir()))?;
            }
            return Ok(EnsuredProjectRuntime {
                runtime_id: existing_id,
                runtime: existing,
                launched: false,
            });
        }
        drop(existing);
        let runtime_id = relaunch_owned_runtime_before_deadline(
            app.clone(),
            &owner,
            record.profile_dir().to_path_buf(),
            record.launch_options().clone(),
            None,
            deadline,
        )
        .await
        .map_err(|error| redact_profile_path(&error, record.profile_dir()))?;
        let runtime = runtime_arc(app, &runtime_id).await?;
        return Ok(EnsuredProjectRuntime {
            launched: runtime_id != existing_id,
            runtime_id,
            runtime,
        });
    }
    let cache_dir = app.gcx.cache_dir.clone();
    let profile_dir = scope.profile_dir(&cache_dir);
    if !profile_resolved(&profile_dir)? {
        resolve_project_profile(app, scope, deadline).await?;
    }
    let options = project_launch_options(options);
    let mode = options.mode_label();
    let mut runtime =
        launch_with_lock_recovery_before_deadline(profile_dir.clone(), options, None, deadline)
            .await
            .map_err(|error| {
                format!(
                    "Failed to start the project browser in {mode} mode: {}",
                    redact_profile_path(&error, &profile_dir)
                )
            })?;
    runtime.reattach(&owner);
    if let Err(error) = command_before(deadline, || setup_recording_for_runtime(&mut runtime)) {
        warn!("Project browser recording setup failed (non-fatal): {error}");
    }
    let runtime_id = register_browser_runtime(app.clone(), runtime).await;
    info!(
        "Project BrowserRuntime {runtime_id} started for project {}",
        scope.project_id()
    );
    let runtime = runtime_arc(app, &runtime_id).await?;
    Ok(EnsuredProjectRuntime {
        runtime_id,
        runtime,
        launched: true,
    })
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |age| age.as_millis() as u64)
}

/// Opens a tab in its own window, owned by `chat_id` when given; the active tab is kept.
pub fn open_project_tab(
    runtime: &mut BrowserRuntime,
    chat_id: Option<&str>,
) -> Result<Arc<headless_chrome::Tab>, String> {
    let tab = runtime.open_tab()?;
    if let Err(error) = setup_recording_for_tab(runtime, tab.clone()) {
        let _ = tab.close(false);
        return Err(error);
    }
    if let Some(chat_id) = chat_id {
        let id = tab.get_target_id().to_string();
        refact_browser::record_chat_tab_use(
            &mut runtime.tab_meta,
            chat_id,
            std::slice::from_ref(&id),
            Some(&id),
            None,
            now_ms(),
        );
    }
    Ok(tab)
}

/// The tab a shared-runtime call without an explicit target runs in for this chat. An
/// unowned active tab is claimed here, under the caller's runtime lock, so a concurrent call
/// from another chat cannot pick it too.
pub fn chat_default_target(
    runtime: &mut BrowserRuntime,
    chat_id: &str,
) -> Result<crate::integrations::browser_models::TabTarget, String> {
    use crate::integrations::browser_models::TabTarget;
    use refact_browser::ChatTabChoice;
    if runtime.recovery_fence.is_some() {
        return Ok(TabTarget::Active);
    }
    refact_browser::adopt_new_tabs(runtime, None);
    let live = runtime
        .live_tabs()
        .iter()
        .map(|tab| tab.get_target_id().to_string())
        .collect::<Vec<_>>();
    let active = runtime
        .get_active_tab()
        .map(|tab| tab.get_target_id().to_string());
    Ok(
        match refact_browser::choose_chat_tab(chat_id, &live, &runtime.tab_meta, active.as_deref())
        {
            ChatTabChoice::Own(id) => TabTarget::Id {
                id,
            },
            ChatTabChoice::ClaimActive(id) => {
                refact_browser::record_chat_tab_use(
                    &mut runtime.tab_meta,
                    chat_id,
                    &[],
                    Some(&id),
                    None,
                    now_ms(),
                );
                TabTarget::Id {
                    id,
                }
            }
            ChatTabChoice::CreateNew => TabTarget::Id {
                id: open_project_tab(runtime, Some(chat_id))?
                    .get_target_id()
                    .to_string(),
            },
        },
    )
}

/// Closes the project's Chrome; the durable profile is always kept.
pub async fn stop_project_runtime(
    app: &crate::app_state::AppState,
    scope: &ProjectBrowserScope,
) -> bool {
    let lock = relaunch_lock_for_chat(&scope.owner_id());
    let _guard = lock.lock().await;
    let mut stopped = false;
    while let Some((runtime_id, runtime)) = find_project_runtime(app, scope).await {
        drop(runtime);
        if !remove_browser_runtime(app.clone(), &runtime_id).await {
            break;
        }
        stopped = true;
    }
    stopped
}

/// Worker shutdown, after request admission is drained and the browser monitor is stopped.
pub async fn shutdown_project_runtimes(app: &crate::app_state::AppState) -> usize {
    let project_runtimes = registered_runtime_owners(app)
        .await
        .into_iter()
        .filter_map(|record| {
            record
                .attached_chat_id()
                .filter(|owner| is_project_owner_id(owner))
                .map(|owner| (record.runtime_id().to_string(), owner))
        })
        .collect::<Vec<_>>();
    futures::future::join_all(
        project_runtimes
            .iter()
            .map(|(runtime_id, owner)| shutdown_project_runtime_generation(app, runtime_id, owner)),
    )
    .await
    .into_iter()
    .filter(|removed| *removed)
    .count()
}

fn profile_resolved(profile_dir: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(profile_dir) {
        Ok(meta) if meta.is_dir() => Ok(true),
        Ok(_) => Err("project browser profile path is not a directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "cannot inspect the project browser profile: {error}"
        )),
    }
}

async fn registered_profile_dirs(app: &crate::app_state::AppState) -> Vec<PathBuf> {
    registered_runtime_owners(app)
        .await
        .into_iter()
        .map(|record| record.profile_dir().to_path_buf())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyProfileCandidate {
    pub chat_id: String,
    pub modified_ms: Option<u64>,
    pub active: bool,
}

fn valid_legacy_chat_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !chat_is_disposable(id)
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_real_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file())
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

#[derive(serde::Deserialize)]
struct TrajectoryProjectBinding {
    #[serde(default)]
    worktree: Option<TrajectoryWorktreeBinding>,
}

#[derive(serde::Deserialize)]
struct TrajectoryWorktreeBinding {
    #[serde(default)]
    source_workspace_root: Option<PathBuf>,
}

/// A trajectory that records its worktree's source root must name this project; one without
/// a worktree record is bound by its location alone.
fn trajectory_binds_project(path: &Path, root: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(binding) =
        serde_json::from_reader::<_, TrajectoryProjectBinding>(std::io::BufReader::new(file))
    else {
        return false;
    };
    match binding.worktree {
        None => true,
        Some(TrajectoryWorktreeBinding {
            source_workspace_root: Some(bound),
        }) => dunce::canonicalize(bound).is_ok_and(|bound| bound == root),
        Some(_) => false,
    }
}

/// Same-project membership is proven only by the chat's trajectory living under this
/// project's `.refact/trajectories` and not recording another project's worktree; nothing
/// inside the profile itself is read.
fn chat_belongs_to_project(source_root: &Path, chat_id: &str) -> bool {
    let Ok(root) = dunce::canonicalize(source_root) else {
        return false;
    };
    let refact_dir = root.join(".refact");
    let dir = refact_dir.join("trajectories");
    if !is_real_dir(&refact_dir) || !is_real_dir(&dir) {
        return false;
    }
    let file = format!("{chat_id}.json");
    let trusted_file = |path: &Path| {
        is_real_file(path)
            && dunce::canonicalize(path).is_ok_and(|canonical| canonical == path)
            && trajectory_binds_project(path, &root)
    };
    trusted_file(&dir.join(&file))
        || (is_real_dir(&dir.join(chat_id)) && trusted_file(&dir.join(chat_id).join(&file)))
}

fn profile_is_active(path: &Path, active_profiles: &[PathBuf]) -> bool {
    active_profiles.iter().any(|active| active == path) || profile_possibly_in_use(path)
}

pub fn list_legacy_candidates_in(
    cache_dir: &Path,
    source_root: &Path,
    active_profiles: &[PathBuf],
) -> Vec<LegacyProfileCandidate> {
    let Ok(entries) = std::fs::read_dir(cache_dir.join(LEGACY_PROFILES_DIR)) else {
        return Vec::new();
    };
    let mut candidates = entries
        .flatten()
        .filter_map(|entry| {
            let chat_id = entry.file_name().to_str()?.to_string();
            let path = entry.path();
            if !valid_legacy_chat_id(&chat_id)
                || !is_real_dir(&path)
                || !chat_belongs_to_project(source_root, &chat_id)
            {
                return None;
            }
            let modified_ms = std::fs::symlink_metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|age| age.as_millis() as u64);
            Some(LegacyProfileCandidate {
                active: profile_is_active(&path, active_profiles),
                chat_id,
                modified_ms,
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| a.chat_id.cmp(&b.chat_id))
    });
    candidates
}

/// Moves exactly one verified, inactive legacy chat profile into the unresolved project
/// profile slot with one same-filesystem rename. Nothing is copied, read or extracted.
fn adopt_legacy_profile_in(
    cache_dir: &Path,
    scope: &ProjectBrowserScope,
    chat_id: &str,
    active_profiles: &[PathBuf],
) -> Result<(), String> {
    if !valid_legacy_chat_id(chat_id) {
        return Err("invalid chat id for browser profile adoption".into());
    }
    let target = scope.profile_dir(cache_dir);
    if profile_resolved(&target)? {
        return Err("the project browser profile already exists; adoption is one-time".into());
    }
    let candidates = list_legacy_candidates_in(cache_dir, scope.source_root(), active_profiles);
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.chat_id == chat_id)
        .ok_or("no adoptable browser profile for that chat in this project")?;
    if candidate.active {
        return Err("that chat's browser is still running; close it before adopting".into());
    }
    let source = cache_dir.join(LEGACY_PROFILES_DIR).join(chat_id);
    clear_stale_profile_lock(&source)
        .map_err(|_| "that chat's browser is still running; close it before adopting")?;
    let parent = target
        .parent()
        .ok_or("project browser profile has no parent directory")?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create the project profile directory: {error}"))?;
    if profile_resolved(&target)? {
        return Err("the project browser profile already exists; adoption is one-time".into());
    }
    if !is_real_dir(&source) || !chat_belongs_to_project(scope.source_root(), chat_id) {
        return Err("no adoptable browser profile for that chat in this project".into());
    }
    if profile_is_active(&source, active_profiles) {
        return Err("that chat's browser is still running; close it before adopting".into());
    }
    rename_no_replace(&source, &target).map_err(|error| {
        if is_cross_device(&error) {
            "the chat profile is on a different filesystem; it was left untouched".to_string()
        } else if error.kind() == std::io::ErrorKind::AlreadyExists {
            "the project browser profile already exists; adoption is one-time".to_string()
        } else {
            format!("moving the chat browser profile failed; it was left in place: {error}")
        }
    })?;
    info!(
        "Adopted browser profile of chat {chat_id} as project profile {}",
        scope.project_id()
    );
    Ok(())
}

fn is_cross_device(error: &std::io::Error) -> bool {
    #[cfg(windows)]
    const CROSS_DEVICE: i32 = 17; // ERROR_NOT_SAME_DEVICE
    #[cfg(not(windows))]
    const CROSS_DEVICE: i32 = libc::EXDEV;
    error.raw_os_error() == Some(CROSS_DEVICE)
}

/// Moves a directory only if nothing exists at the destination, so an empty profile
/// created concurrently is never replaced. Where the filesystem has no atomic no-replace
/// rename, the destination is checked right before a plain rename under the owner lock.
#[cfg(not(windows))]
fn rename_no_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = |path: &Path| {
            std::ffi::CString::new(path.as_os_str().as_bytes())
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
        };
        let (from, to) = (path(source)?, path(target)?);
        #[cfg(target_os = "linux")]
        let renamed = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let renamed = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        if renamed == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if !matches!(
            error.raw_os_error(),
            Some(libc::EINVAL | libc::ENOSYS | libc::ENOTSUP)
        ) {
            return Err(error);
        }
    }
    if std::fs::symlink_metadata(target).is_ok() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    std::fs::rename(source, target)
}

/// `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` fails when the destination exists;
/// `std::fs::rename` may replace an empty directory on current Windows.
#[cfg(windows)]
fn rename_no_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<u16>>()
    };
    let (from, to) = (wide(source), wide(target));
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Resolves the project profile once: the most recently used inactive chat profile of this
/// project is adopted, otherwise an empty profile is created. A candidate that started or
/// vanished while its chat lock was awaited is skipped for the next one.
async fn resolve_project_profile(
    app: &crate::app_state::AppState,
    scope: &ProjectBrowserScope,
    deadline: Instant,
) -> Result<(), String> {
    let cache_dir = app.gcx.cache_dir.clone();
    let active = registered_profile_dirs(app).await;
    let candidates = list_legacy_candidates_in(&cache_dir, scope.source_root(), &active);
    for candidate in candidates.into_iter().filter(|candidate| !candidate.active) {
        let lock = relaunch_lock_for_chat(&candidate.chat_id);
        let _chat_guard = before_deadline(deadline, lock.lock()).await?;
        let active = registered_profile_dirs(app).await;
        let source = cache_dir.join(LEGACY_PROFILES_DIR).join(&candidate.chat_id);
        if !is_real_dir(&source) || profile_is_active(&source, &active) {
            continue;
        }
        return adopt_legacy_profile_in(&cache_dir, scope, &candidate.chat_id, &active);
    }
    std::fs::create_dir_all(scope.profile_dir(&cache_dir))
        .map_err(|error| format!("cannot create the project browser profile: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        cache: tempfile::TempDir,
        project: tempfile::TempDir,
        scope: ProjectBrowserScope,
    }

    fn fixture() -> Fixture {
        let cache = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let scope = ProjectBrowserScope::from_source_root(project.path()).unwrap();
        Fixture {
            cache,
            project,
            scope,
        }
    }

    fn legacy(fx: &Fixture, chat_id: &str, in_project: bool) -> PathBuf {
        let profile = fx.cache.path().join(LEGACY_PROFILES_DIR).join(chat_id);
        std::fs::create_dir_all(profile.join("Default")).unwrap();
        std::fs::write(profile.join("Default").join("Cookies"), b"synthetic").unwrap();
        if in_project {
            let dir = fx.project.path().join(".refact").join("trajectories");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{chat_id}.json")), b"{}").unwrap();
        }
        profile
    }

    #[test]
    fn project_identity_is_shared_by_root_not_chat_and_differs_across_projects() {
        let fx = fixture();
        let again = ProjectBrowserScope::from_source_root(fx.project.path()).unwrap();
        assert_eq!(fx.scope, again);
        let other = tempfile::tempdir().unwrap();
        let other = ProjectBrowserScope::from_source_root(other.path()).unwrap();
        assert_ne!(fx.scope.project_id(), other.project_id());
        assert_ne!(
            fx.scope.profile_dir(fx.cache.path()),
            other.profile_dir(fx.cache.path())
        );
        assert!(is_project_owner_id(&fx.scope.owner_id()));
        assert!(!chat_is_disposable(&fx.scope.owner_id()));
        assert!(ProjectBrowserScope::from_source_root(&fx.project.path().join("missing")).is_err());
    }

    #[test]
    fn chat_runtime_lookup_never_selects_a_project_runtime() {
        let fx = fixture();
        let owners = vec![RuntimeOwner {
            runtime_id: "rt-project".into(),
            attached_chat_id: Some(fx.scope.owner_id()),
            connected: true,
        }];
        let lookup = ChatRuntimeLookup {
            chat_id: "chat-a".into(),
            root_chat_id: Some("root".into()),
            background_agent_id: None,
        };
        assert_eq!(select_runtime_for_chat(&owners, &lookup), None);
    }

    #[test]
    fn project_launch_options_drop_profile_and_debugging_overrides() {
        let options = project_launch_options(BrowserLaunchOptions {
            extra_args: vec![
                "--user-data-dir=/tmp/x".into(),
                "--remote-debugging-port=9222".into(),
                " --Incognito".into(),
                "--profile-directory=Other".into(),
                "--lang=en".into(),
            ],
            evict_idle_attached: true,
            ..Default::default()
        });
        assert_eq!(options.extra_args, vec!["--lang=en".to_string()]);
        assert!(!options.evict_idle_attached);
    }

    #[test]
    fn only_same_project_real_chat_profiles_are_candidates() {
        let fx = fixture();
        legacy(&fx, "chat-mine", true);
        legacy(&fx, "chat-foreign", false);
        legacy(&fx, "subchat-x", true);
        let root = fx.cache.path().join(LEGACY_PROFILES_DIR);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("chat-mine"), root.join("chat-link")).unwrap();
            let dir = fx.project.path().join(".refact").join("trajectories");
            std::fs::write(dir.join("chat-link.json"), b"{}").unwrap();
        }
        let ids = list_legacy_candidates_in(fx.cache.path(), fx.project.path(), &[])
            .into_iter()
            .map(|c| c.chat_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["chat-mine".to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn only_real_project_trajectory_storage_proves_profile_provenance() {
        use std::os::unix::fs::symlink;

        let fx = fixture();
        let foreign = tempfile::tempdir().unwrap();
        let foreign_refact = foreign.path().join(".refact");
        let foreign_trajectories = foreign_refact.join("trajectories");
        std::fs::create_dir_all(&foreign_trajectories).unwrap();
        std::fs::write(foreign_trajectories.join("chat-foreign.json"), b"{}").unwrap();
        let source = legacy(&fx, "chat-foreign", false);
        let project_refact = fx.project.path().join(".refact");
        symlink(&foreign_refact, &project_refact).unwrap();
        assert!(list_legacy_candidates_in(fx.cache.path(), fx.scope.source_root(), &[]).is_empty());
        assert!(adopt_legacy_profile_in(fx.cache.path(), &fx.scope, "chat-foreign", &[]).is_err());
        assert!(source.exists() && !fx.scope.profile_dir(fx.cache.path()).exists());

        std::fs::remove_file(project_refact).unwrap();
        legacy(&fx, "chat-flat", true);
        legacy(&fx, "chat-nested", false);
        let nest = fx.project.path().join(".refact/trajectories/chat-nested");
        std::fs::create_dir_all(&nest).unwrap();
        std::fs::write(nest.join("chat-nested.json"), b"{}").unwrap();
        let mut ids = list_legacy_candidates_in(fx.cache.path(), fx.scope.source_root(), &[])
            .into_iter()
            .map(|candidate| candidate.chat_id)
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, ["chat-flat", "chat-nested"]);
    }

    #[test]
    fn adoption_moves_exactly_the_selected_profile_once() {
        let fx = fixture();
        let chosen = legacy(&fx, "chat-a", true);
        let other = legacy(&fx, "chat-b", true);
        let unrelated = legacy(&fx, "chat-c", false);
        let target = fx.scope.profile_dir(fx.cache.path());

        adopt_legacy_profile_in(fx.cache.path(), &fx.scope, "chat-a", &[]).unwrap();
        assert!(!chosen.exists());
        assert_eq!(
            std::fs::read(target.join("Default").join("Cookies")).unwrap(),
            b"synthetic"
        );
        assert!(other.exists() && unrelated.exists());

        let again = adopt_legacy_profile_in(fx.cache.path(), &fx.scope, "chat-b", &[]);
        assert!(again.unwrap_err().contains("already exists"));
        assert!(other.exists());
    }

    #[test]
    fn adoption_rejects_active_foreign_and_invalid_targets() {
        let fx = fixture();
        let busy = legacy(&fx, "chat-busy", true);
        legacy(&fx, "chat-foreign", false);
        let target = fx.scope.profile_dir(fx.cache.path());
        let active = vec![busy.clone()];
        let error =
            adopt_legacy_profile_in(fx.cache.path(), &fx.scope, "chat-busy", &active).unwrap_err();
        assert!(error.contains("still running"), "{error}");
        for id in ["chat-foreign", "../chat-busy", "subchat-1", ""] {
            assert!(
                adopt_legacy_profile_in(fx.cache.path(), &fx.scope, id, &[]).is_err(),
                "{id}"
            );
        }
        assert!(busy.exists() && !target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn adoption_rejects_a_profile_locked_from_another_host() {
        let fx = fixture();
        let locked = legacy(&fx, "chat-locked", true);
        std::os::unix::fs::symlink(
            format!("not-{}-4242", local_hostname()),
            locked.join("SingletonLock"),
        )
        .unwrap();
        let candidates = list_legacy_candidates_in(fx.cache.path(), fx.project.path(), &[]);
        assert!(candidates[0].active);
        assert!(adopt_legacy_profile_in(fx.cache.path(), &fx.scope, "chat-locked", &[]).is_err());
        assert!(locked.exists());
    }

    #[test]
    fn trajectories_recording_another_projects_worktree_are_not_candidates() {
        let fx = fixture();
        let other = tempfile::tempdir().unwrap();
        let dir = fx.project.path().join(".refact").join("trajectories");
        let write = |chat_id: &str, body: serde_json::Value| {
            legacy(&fx, chat_id, true);
            std::fs::write(dir.join(format!("{chat_id}.json")), body.to_string()).unwrap();
        };
        write(
            "chat-plain",
            serde_json::json!({"id": "chat-plain", "worktree": null}),
        );
        write(
            "chat-same",
            serde_json::json!({"worktree": {"id": "w1", "source_workspace_root": fx.project.path()}}),
        );
        write(
            "chat-other",
            serde_json::json!({"worktree": {"id": "w2", "source_workspace_root": other.path()}}),
        );
        write(
            "chat-rootless",
            serde_json::json!({"worktree": {"id": "w3"}}),
        );
        legacy(&fx, "chat-corrupt", true);
        std::fs::write(dir.join("chat-corrupt.json"), b"{not json").unwrap();
        let mut ids = list_legacy_candidates_in(fx.cache.path(), fx.scope.source_root(), &[])
            .into_iter()
            .map(|candidate| candidate.chat_id)
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, ["chat-plain", "chat-same"]);
        assert!(adopt_legacy_profile_in(fx.cache.path(), &fx.scope, "chat-other", &[]).is_err());
    }

    #[test]
    fn cross_device_moves_are_recognised() {
        #[cfg(not(windows))]
        let cross = std::io::Error::from_raw_os_error(libc::EXDEV);
        #[cfg(windows)]
        let cross = std::io::Error::from_raw_os_error(17);
        assert!(is_cross_device(&cross));
        assert!(!is_cross_device(&std::io::Error::from(
            std::io::ErrorKind::AlreadyExists
        )));
    }

    #[test]
    fn adoption_never_replaces_a_concurrently_created_profile() {
        let fx = fixture();
        let source = legacy(&fx, "chat-a", true);
        let target = fx.scope.profile_dir(fx.cache.path());
        std::fs::create_dir_all(&target).unwrap();
        let error = rename_no_replace(&source, &target).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(source.join("Default").join("Cookies").exists());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        std::fs::remove_dir(&target).unwrap();
        rename_no_replace(&source, &target).unwrap();
        assert!(target.join("Default").join("Cookies").exists() && !source.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unverifiable_profile_locks_count_as_in_use() {
        assert!(lock_owner_unverified(true, None));
        assert!(!lock_owner_unverified(true, Some(false)));
        assert!(!lock_owner_unverified(false, None));
        let fx = fixture();
        let garbled = legacy(&fx, "chat-garbled", true);
        std::os::unix::fs::symlink("no-pid-here", garbled.join("SingletonLock")).unwrap();
        let stale = legacy(&fx, "chat-stale", true);
        std::os::unix::fs::symlink(
            format!("{}-{}", local_hostname(), std::process::id()),
            stale.join("SingletonLock"),
        )
        .unwrap();
        assert!(profile_possibly_in_use(&garbled));
        assert!(!profile_possibly_in_use(&stale));
        let plain = legacy(&fx, "chat-plain-lock", true);
        std::fs::write(plain.join("SingletonLock"), b"").unwrap();
        assert!(profile_possibly_in_use(&plain));
        let unreadable = legacy(&fx, "chat-unreadable", true);
        let fresh = legacy(&fx, "chat-fresh", true);
        assert!(!profile_possibly_in_use(&fresh));
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
            let in_use = profile_possibly_in_use(&unreadable);
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
            let root_reads_everything = unsafe { libc::geteuid() } == 0;
            assert!(
                in_use || root_reads_everything,
                "an unreadable SingletonLock must count as in use"
            );
        }
        let mut ids = list_legacy_candidates_in(fx.cache.path(), fx.scope.source_root(), &[])
            .into_iter()
            .filter(|candidate| !candidate.active)
            .map(|candidate| candidate.chat_id)
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, ["chat-fresh", "chat-stale", "chat-unreadable"]);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_candidate_started_while_awaiting_its_lock_is_skipped_for_the_next() {
        let fx = fixture();
        let app = app_with_cache(&fx).await;
        let older = legacy_used_at(&fx, "chat-older", true, 200);
        let recent = legacy_used_at(&fx, "chat-recent", true, 100);
        let target = fx.scope.profile_dir(fx.cache.path());
        let lock = relaunch_lock_for_chat("chat-recent");
        let guard = lock.lock().await;
        let resolving = tokio::spawn({
            let app = app.clone();
            let scope = fx.scope.clone();
            async move {
                resolve_project_profile(&app, &scope, Instant::now() + Duration::from_secs(10))
                    .await
            }
        });
        while Arc::strong_count(&lock) < 3 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        std::os::unix::fs::symlink(
            format!("not-{}-4242", local_hostname()),
            recent.join("SingletonLock"),
        )
        .unwrap();
        drop(guard);
        resolving.await.unwrap().unwrap();
        assert!(recent.join("Default").join("Cookies").exists());
        assert!(!older.exists());
        assert!(target.join("Default").join("Cookies").exists());
    }

    #[tokio::test]
    async fn unknown_chat_and_unbound_session_have_no_project_scope() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let project = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().into()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        assert!(project_scope_for_chat(&app, "missing").await.is_err());
        let unbound = crate::chat::types::ChatSession::new("unbound".into());
        let mut bound_a = crate::chat::types::ChatSession::new("a".into());
        bound_a.work_project_root = Some(project.path().into());
        let mut bound_b = crate::chat::types::ChatSession::new("b".into());
        bound_b.work_project_root = Some(project.path().into());
        {
            let mut sessions = app.chat.sessions.write().await;
            for session in [unbound, bound_a, bound_b] {
                let id = session.chat_id.clone();
                sessions.insert(id, Arc::new(AMutex::new(session)));
            }
        }
        assert!(project_scope_for_chat(&app, "unbound").await.is_err());
        let a = project_scope_for_chat(&app, "a").await.unwrap();
        let b = project_scope_for_chat(&app, "b").await.unwrap();
        assert_eq!(a, b);
        assert_eq!(current_project_scope(&app).await.unwrap(), a);
        let via_root = project_scope_for_context(&app, None, "subchat-1", "a")
            .await
            .unwrap();
        assert_eq!(via_root, Some(a));
        assert_eq!(
            project_scope_for_context(&app, None, "unbound", "unbound")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn managed_worker_tools_use_only_the_pinned_project() {
        let project_a = tempfile::tempdir().unwrap();
        let project_b = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let cmdline = &mut Arc::get_mut(&mut gcx).unwrap().cmdline;
            cmdline.daemon_endpoint = "http://127.0.0.1:8488".into();
            cmdline.project_id = "project-a".into();
            cmdline.workspace_folder = project_a.path().to_str().unwrap().into();
        }
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        {
            let mut sessions = app.chat.sessions.write().await;
            for (id, root) in [("on-a", project_a.path()), ("on-b", project_b.path())] {
                let mut session = crate::chat::types::ChatSession::new(id.into());
                session.work_project_root = Some(root.into());
                sessions.insert(id.into(), Arc::new(AMutex::new(session)));
            }
        }
        let pinned = ProjectBrowserScope::from_source_root(project_a.path()).unwrap();
        assert_eq!(
            project_scope_for_context(&app, None, "on-a", "on-a")
                .await
                .unwrap(),
            Some(pinned.clone())
        );
        assert!(project_scope_for_context(&app, None, "on-b", "on-b")
            .await
            .is_err());
        assert!(project_scope_for_context(&app, None, "sub", "on-b")
            .await
            .is_err());
        let deleted = tempfile::tempdir().unwrap();
        {
            let mut sessions = app.chat.sessions.write().await;
            let mut on_deleted = crate::chat::types::ChatSession::new("on-deleted".into());
            on_deleted.work_project_root = Some(deleted.path().into());
            sessions.insert("on-deleted".into(), Arc::new(AMutex::new(on_deleted)));
            let unbound = crate::chat::types::ChatSession::new("unbound".into());
            sessions.insert("unbound".into(), Arc::new(AMutex::new(unbound)));
        }
        drop(deleted);
        let error = project_scope_for_context(&app, None, "on-deleted", "on-deleted")
            .await
            .unwrap_err();
        assert!(error.contains("no longer accessible"), "{error}");
        assert_eq!(
            project_scope_for_context(&app, None, "unbound", "unbound")
                .await
                .unwrap(),
            Some(pinned.clone())
        );
        assert_eq!(
            project_scope_for_context(&app, None, "sub", "unbound")
                .await
                .unwrap(),
            Some(pinned)
        );
        assert_eq!(
            project_scope_for_context(&app, None, "http-only", "")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn direct_worker_tools_follow_the_chat_project() {
        let project_b = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        assert!(gcx.cmdline.daemon_endpoint.is_empty());
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let mut session = crate::chat::types::ChatSession::new("on-b".into());
        session.work_project_root = Some(project_b.path().into());
        app.chat
            .sessions
            .write()
            .await
            .insert("on-b".into(), Arc::new(AMutex::new(session)));
        assert!(pinned_project_scope(&app).is_none());
        assert_eq!(
            project_scope_for_context(&app, None, "on-b", "on-b")
                .await
                .unwrap(),
            Some(ProjectBrowserScope::from_source_root(project_b.path()).unwrap())
        );
    }

    fn legacy_used_at(fx: &Fixture, chat_id: &str, in_project: bool, age_secs: u64) -> PathBuf {
        let profile = legacy(fx, chat_id, in_project);
        std::fs::File::open(&profile)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(age_secs))
            .unwrap();
        profile
    }

    async fn app_with_cache(fx: &Fixture) -> crate::app_state::AppState {
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx).unwrap().cache_dir = fx.cache.path().to_path_buf();
        crate::app_state::AppState::from_gcx(gcx).await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unresolved_profile_adopts_the_most_recent_inactive_project_profile_once() {
        let fx = fixture();
        let app = app_with_cache(&fx).await;
        let oldest = legacy_used_at(&fx, "chat-oldest", true, 300);
        let older = legacy_used_at(&fx, "chat-older", true, 200);
        let recent = legacy_used_at(&fx, "chat-recent", true, 100);
        std::fs::write(recent.join("Default").join("Cookies"), b"recent").unwrap();
        let locked = legacy_used_at(&fx, "chat-locked", true, 10);
        std::os::unix::fs::symlink(
            format!("not-{}-4242", local_hostname()),
            locked.join("SingletonLock"),
        )
        .unwrap();
        let foreign = legacy_used_at(&fx, "chat-foreign", false, 0);
        let target = fx.scope.profile_dir(fx.cache.path());
        let deadline = Instant::now() + Duration::from_secs(5);

        resolve_project_profile(&app, &fx.scope, deadline)
            .await
            .unwrap();
        assert!(!recent.exists());
        assert_eq!(
            std::fs::read(target.join("Default").join("Cookies")).unwrap(),
            b"recent"
        );
        assert!(oldest.exists() && older.exists() && locked.exists() && foreign.exists());
        assert!(profile_resolved(&target).unwrap());

        let error = resolve_project_profile(&app, &fx.scope, deadline)
            .await
            .unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert!(oldest.exists() && older.exists());
        assert_eq!(
            std::fs::read(target.join("Default").join("Cookies")).unwrap(),
            b"recent"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unresolved_profile_without_eligible_candidates_starts_fresh() {
        let fx = fixture();
        let app = app_with_cache(&fx).await;
        let locked = legacy(&fx, "chat-locked", true);
        std::os::unix::fs::symlink(
            format!("not-{}-4242", local_hostname()),
            locked.join("SingletonLock"),
        )
        .unwrap();
        let foreign = legacy(&fx, "chat-foreign", false);
        let target = fx.scope.profile_dir(fx.cache.path());

        resolve_project_profile(&app, &fx.scope, Instant::now() + Duration::from_secs(5))
            .await
            .unwrap();
        assert!(profile_resolved(&target).unwrap());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert!(locked.join("Default").join("Cookies").exists());
        assert!(foreign.join("Default").join("Cookies").exists());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worker_shutdown_closes_and_reaps_the_exact_owned_child() {
        use futures::{SinkExt, StreamExt};
        use std::os::unix::fs::PermissionsExt;
        use tokio::io::AsyncWriteExt;
        use tokio_tungstenite::{accept_async, tungstenite::Message};

        let fx = fixture();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let profile = fx.scope.profile_dir(fx.cache.path());
        std::fs::create_dir_all(&profile).unwrap();
        let persisted = profile.join("persisted");
        std::fs::write(&persisted, b"existing profile state").unwrap();
        let marker = profile.join("voluntary-exit");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let control = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let executable = fx.cache.path().join("chrome-fixture");
        std::fs::write(
            &executable,
            format!(
                "#!/usr/bin/env python3\nimport socket,sys\ns=socket.create_connection(('127.0.0.1',{}))\nprint('DevTools listening on ws://{}/devtools/browser/fixture',file=sys.stderr,flush=True)\nassert s.recv(1)==b'x'\nopen({:?},'w').write('saved')\n",
                control.local_addr().unwrap().port(),
                listener.local_addr().unwrap(),
                marker.to_str().unwrap(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut browser = accept_async(stream).await.unwrap();
            let call: serde_json::Value =
                serde_json::from_str(browser.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(call["method"], "Target.setDiscoverTargets");
            browser
                .send(Message::text(
                    serde_json::json!({"id": call["id"], "result": {}}).to_string(),
                ))
                .await
                .unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let _downloads = accept_async(stream).await.unwrap();
            let call: serde_json::Value =
                serde_json::from_str(browser.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(call["method"], "Browser.close");
            drop(browser);
            let (mut child, _) = control.accept().await.unwrap();
            child.write_all(b"x").await.unwrap();
        });
        let mut runtime = tokio::task::spawn_blocking(move || {
            let _deadline = headless_chrome::browser::transport::CommandDeadlineGuard::new(
                Instant::now() + Duration::from_secs(5),
            );
            BrowserRuntime::launch(
                profile,
                BrowserLaunchOptions {
                    chrome_path: Some(executable),
                    chromium_sandbox: false,
                    ..BrowserLaunchOptions::headless(true)
                },
            )
        })
        .await
        .unwrap()
        .unwrap();
        runtime.reattach(&fx.scope.owner_id());
        let ownership = runtime.ownership.clone();
        let pid = ownership.process_id().unwrap();
        let id = register_browser_runtime(app.clone(), runtime).await;
        let held_runtime = runtime_arc(&app, &id).await.unwrap();
        assert!(pid_is_alive(pid) && !marker.exists());

        assert!(!shutdown_project_runtime_generation(&app, &id, "project-browser:other").await);
        assert!(ownership.is_current_generation());
        let (first, second) = tokio::join!(
            shutdown_project_runtimes(&app),
            shutdown_project_runtimes(&app),
        );
        assert_eq!(first + second, 1);
        assert!(!ownership.is_current_generation());
        assert!(find_project_runtime(&app, &fx.scope).await.is_none());
        // A zombie still answers signal 0 on Unix, so this also checks the child was reaped.
        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        assert_eq!(std::fs::read(&marker).unwrap(), b"saved");
        assert_eq!(
            std::fs::read(&persisted).unwrap(),
            b"existing profile state"
        );
        drop(held_runtime);
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }
}
