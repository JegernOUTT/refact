//! The default browser is one durable Chromium profile and one live runtime per trusted
//! project. Its runtime owner is `project-browser:<project id>`, never a chat id, so the
//! chat-keyed lookups cannot return it; callers resolve the project from trusted session
//! state first. Named and explicitly isolated sessions do not use this module.
use super::*;
use serde::Serialize;

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

/// Tool contexts: the execution scope's source root wins, then the chat, then its root chat.
pub async fn project_scope_for_context(
    app: &crate::app_state::AppState,
    execution_scope: Option<&crate::worktrees::scope::ExecutionScope>,
    chat_id: &str,
    root_chat_id: &str,
) -> Result<ProjectBrowserScope, String> {
    if let Some(scope) = execution_scope {
        return ProjectBrowserScope::from_source_root(scope.source_workspace_root());
    }
    match project_scope_for_chat(app, chat_id).await {
        Ok(scope) => Ok(scope),
        Err(error) if root_chat_id.is_empty() || root_chat_id == chat_id => Err(error),
        Err(_) => project_scope_for_chat(app, root_chat_id).await,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnsureError {
    DecisionRequired { candidate_count: usize },
    Failed(String),
}

impl std::fmt::Display for EnsureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DecisionRequired {
                candidate_count,
            } => write!(
                f,
                "the project browser profile needs a decision first: {candidate_count} earlier \
                 chat browser profile(s) can be adopted or skipped in the Browser tab"
            ),
            Self::Failed(error) => write!(f, "{error}"),
        }
    }
}

impl From<String> for EnsureError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
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
/// a runtime whose process died is relaunched on the same profile. An unresolved profile with
/// adoptable earlier chat profiles needs an explicit adopt/skip decision first.
pub async fn ensure_project_runtime(
    app: &crate::app_state::AppState,
    scope: &ProjectBrowserScope,
    options: BrowserLaunchOptions,
    deadline: Instant,
) -> Result<EnsuredProjectRuntime, EnsureError> {
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
        let active = registered_profile_dirs(app).await;
        let candidates = list_legacy_candidates_in(&cache_dir, scope.source_root(), &active);
        if !candidates.is_empty() {
            return Err(EnsureError::DecisionRequired {
                candidate_count: candidates.len(),
            });
        }
        std::fs::create_dir_all(&profile_dir)
            .map_err(|error| format!("cannot create the project browser profile: {error}"))?;
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

/// Engine shutdown: closes every project browser with SIGTERM so Chrome flushes cookies and
/// session state into the durable profiles, which are kept.
pub async fn shutdown_project_runtimes(app: &crate::app_state::AppState) -> usize {
    let project_runtimes = registered_runtime_owners(app)
        .await
        .into_iter()
        .filter(|record| {
            record
                .attached_chat_id()
                .is_some_and(|owner| is_project_owner_id(&owner))
        })
        .map(|record| record.runtime_id().to_string())
        .collect::<Vec<_>>();
    let mut stopped = 0;
    for runtime_id in project_runtimes {
        if remove_browser_runtime(app.clone(), &runtime_id).await {
            stopped += 1;
        }
    }
    stopped
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LegacyProfileCandidate {
    pub chat_id: String,
    pub modified_ms: Option<u64>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProjectProfileState {
    pub project_id: String,
    pub resolved: bool,
    pub runtime_id: Option<String>,
    pub runtime_live: bool,
    pub candidates: Vec<LegacyProfileCandidate>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AdoptOutcome {
    pub project_id: String,
    pub adopted_chat_id: String,
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

/// Same-project membership is proven only by the chat's trajectory living under this
/// project's `.refact/trajectories`; nothing inside the profile itself is read.
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
        is_real_file(path) && dunce::canonicalize(path).is_ok_and(|canonical| canonical == path)
    };
    trusted_file(&dir.join(&file))
        || (is_real_dir(&dir.join(chat_id)) && trusted_file(&dir.join(chat_id).join(&file)))
}

fn profile_is_active(path: &Path, active_profiles: &[PathBuf]) -> bool {
    active_profiles.iter().any(|active| active == path)
        || matches!(
            profile_lock_state(path),
            ProfileLockState::Live(_) | ProfileLockState::Foreign(_)
        )
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
pub fn adopt_legacy_profile_in(
    cache_dir: &Path,
    scope: &ProjectBrowserScope,
    chat_id: &str,
    confirmed: bool,
    active_profiles: &[PathBuf],
    project_runtime_registered: bool,
) -> Result<AdoptOutcome, String> {
    if !confirmed {
        return Err("adopting a chat browser profile requires explicit confirmation".into());
    }
    if !valid_legacy_chat_id(chat_id) {
        return Err("invalid chat id for browser profile adoption".into());
    }
    if project_runtime_registered {
        return Err("the project browser is running; stop it before adopting a profile".into());
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
    clear_stale_profile_lock(&source)?;
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
    std::fs::rename(&source, &target).map_err(|error| {
        if error.raw_os_error() == Some(libc::EXDEV) {
            "the chat profile is on a different filesystem; it was left untouched".to_string()
        } else {
            format!("moving the chat browser profile failed; it was left in place: {error}")
        }
    })?;
    info!(
        "Adopted browser profile of chat {chat_id} as project profile {}",
        scope.project_id()
    );
    Ok(AdoptOutcome {
        project_id: scope.project_id().to_string(),
        adopted_chat_id: chat_id.to_string(),
    })
}

pub async fn project_profile_state(
    app: &crate::app_state::AppState,
    scope: &ProjectBrowserScope,
) -> Result<ProjectProfileState, String> {
    let cache_dir = app.gcx.cache_dir.clone();
    let resolved = profile_resolved(&scope.profile_dir(&cache_dir))?;
    let runtime = find_project_runtime(app, scope).await;
    let runtime_live = runtime
        .as_ref()
        .and_then(|(id, _)| runtime_owner_record(id))
        .is_some_and(|record| record.is_connected());
    let candidates = if resolved {
        Vec::new()
    } else {
        let active = registered_profile_dirs(app).await;
        list_legacy_candidates_in(&cache_dir, scope.source_root(), &active)
    };
    Ok(ProjectProfileState {
        project_id: scope.project_id().to_string(),
        resolved,
        runtime_id: runtime.map(|(id, _)| id),
        runtime_live,
        candidates,
    })
}

pub async fn adopt_legacy_profile(
    app: &crate::app_state::AppState,
    scope: &ProjectBrowserScope,
    chat_id: &str,
    confirmed: bool,
) -> Result<AdoptOutcome, String> {
    if !valid_legacy_chat_id(chat_id) {
        return Err("invalid chat id for browser profile adoption".into());
    }
    let owner_guard = relaunch_lock_for_chat(&scope.owner_id()).lock_owned().await;
    let chat_guard = relaunch_lock_for_chat(chat_id).lock_owned().await;
    let registered = find_project_runtime(app, scope).await.is_some();
    let active = registered_profile_dirs(app).await;
    let cache_dir = app.gcx.cache_dir.clone();
    let scope = scope.clone();
    let chat_id = chat_id.to_string();
    tokio::task::spawn_blocking(move || {
        let (_owner, _chat) = (owner_guard, chat_guard);
        adopt_legacy_profile_in(&cache_dir, &scope, &chat_id, confirmed, &active, registered)
    })
    .await
    .map_err(|error| format!("profile adoption task failed: {error}"))?
}

/// Records the decision to start the project with a fresh profile.
pub async fn skip_legacy_adoption(
    app: &crate::app_state::AppState,
    scope: &ProjectBrowserScope,
) -> Result<ProjectProfileState, String> {
    {
        let lock = relaunch_lock_for_chat(&scope.owner_id());
        let _guard = lock.lock().await;
        let profile_dir = scope.profile_dir(&app.gcx.cache_dir);
        if !profile_resolved(&profile_dir)? {
            std::fs::create_dir_all(&profile_dir)
                .map_err(|error| format!("cannot create the project browser profile: {error}"))?;
        }
    }
    project_profile_state(app, scope).await
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
        assert!(adopt_legacy_profile_in(
            fx.cache.path(),
            &fx.scope,
            "chat-foreign",
            true,
            &[],
            false
        )
        .is_err());
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

        let refused =
            adopt_legacy_profile_in(fx.cache.path(), &fx.scope, "chat-a", false, &[], false);
        assert!(refused.unwrap_err().contains("confirmation"));
        assert!(chosen.exists() && !target.exists());

        let outcome =
            adopt_legacy_profile_in(fx.cache.path(), &fx.scope, "chat-a", true, &[], false)
                .unwrap();
        assert_eq!(outcome.adopted_chat_id, "chat-a");
        assert!(!chosen.exists());
        assert_eq!(
            std::fs::read(target.join("Default").join("Cookies")).unwrap(),
            b"synthetic"
        );
        assert!(other.exists() && unrelated.exists());

        let again = adopt_legacy_profile_in(fx.cache.path(), &fx.scope, "chat-b", true, &[], false);
        assert!(again.unwrap_err().contains("already exists"));
        assert!(other.exists());
    }

    #[test]
    fn adoption_rejects_active_foreign_invalid_and_live_targets() {
        let fx = fixture();
        let busy = legacy(&fx, "chat-busy", true);
        legacy(&fx, "chat-foreign", false);
        let target = fx.scope.profile_dir(fx.cache.path());
        let active = vec![busy.clone()];
        let error = adopt_legacy_profile_in(
            fx.cache.path(),
            &fx.scope,
            "chat-busy",
            true,
            &active,
            false,
        )
        .unwrap_err();
        assert!(error.contains("still running"), "{error}");
        let error =
            adopt_legacy_profile_in(fx.cache.path(), &fx.scope, "chat-busy", true, &[], true)
                .unwrap_err();
        assert!(error.contains("running"), "{error}");
        for id in ["chat-foreign", "../chat-busy", "subchat-1", ""] {
            assert!(
                adopt_legacy_profile_in(fx.cache.path(), &fx.scope, id, true, &[], false).is_err(),
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
        assert!(adopt_legacy_profile_in(
            fx.cache.path(),
            &fx.scope,
            "chat-locked",
            true,
            &[],
            false
        )
        .is_err());
        assert!(locked.exists());
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
        assert_eq!(via_root, a);
    }

    #[tokio::test]
    async fn unresolved_profile_with_candidates_requires_a_decision_and_skip_resolves_it() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let project = tempfile::tempdir().unwrap();
        let scope = ProjectBrowserScope::from_source_root(project.path()).unwrap();
        let legacy_dir = gcx.cache_dir.join(LEGACY_PROFILES_DIR).join("chat-old");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let trajectories = project.path().join(".refact").join("trajectories");
        std::fs::create_dir_all(&trajectories).unwrap();
        std::fs::write(trajectories.join("chat-old.json"), b"{}").unwrap();

        let state = project_profile_state(&app, &scope).await.unwrap();
        assert!(!state.resolved);
        assert_eq!(state.candidates.len(), 1);
        let result = ensure_project_runtime(
            &app,
            &scope,
            BrowserLaunchOptions::default(),
            Instant::now() + Duration::from_secs(5),
        )
        .await;
        assert_eq!(
            result.err(),
            Some(EnsureError::DecisionRequired {
                candidate_count: 1
            })
        );
        assert!(!scope.profile_dir(&gcx.cache_dir).exists());

        let state = skip_legacy_adoption(&app, &scope).await.unwrap();
        assert!(state.resolved && state.candidates.is_empty());
        assert!(legacy_dir.exists());
        let error = adopt_legacy_profile(&app, &scope, "chat-old", true)
            .await
            .unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert!(legacy_dir.exists());
    }
}
