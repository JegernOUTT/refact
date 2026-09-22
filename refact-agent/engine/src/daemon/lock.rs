use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fd_lock::{RwLock, RwLockWriteGuard};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub fn open_lock(path: &Path) -> io::Result<RwLock<File>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)?;
    Ok(RwLock::new(file))
}

pub fn try_lock(lock: &mut RwLock<File>) -> io::Result<RwLockWriteGuard<'_, File>> {
    lock.try_write()
}

pub fn lock_blocking(lock: &mut RwLock<File>) -> io::Result<RwLockWriteGuard<'_, File>> {
    lock.write()
}

pub fn is_already_locked(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
}

pub const WORKSPACE_BUSY_EXIT_CODE: i32 = 75;
pub const WORKSPACE_BUSY_MARKER: &str = "workspace busy";
pub const ENGINE_LOCKS_DIR_ENV: &str = "REFACT_ENGINE_LOCKS_DIR";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLeaseInfo {
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub http_port: u16,
    #[serde(default)]
    pub lsp_port: u16,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub started_at_ms: u64,
    #[serde(default)]
    pub command: String,
}

impl WorkspaceLeaseInfo {
    pub fn for_current_process(http_port: u16, lsp_port: u16, command: &str) -> Self {
        Self {
            pid: std::process::id(),
            http_port,
            lsp_port,
            version: env!("CARGO_PKG_VERSION").to_string(),
            started_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            command: command.to_string(),
        }
    }

    pub fn describe(&self) -> String {
        let mut parts = vec![format!("pid {}", self.pid)];
        if self.http_port != 0 {
            parts.push(format!("http port {}", self.http_port));
        }
        if self.lsp_port != 0 {
            parts.push(format!("lsp port {}", self.lsp_port));
        }
        if !self.command.is_empty() {
            parts.push(format!("command {}", self.command));
        }
        if !self.version.is_empty() {
            parts.push(format!("version {}", self.version));
        }
        parts.join(", ")
    }
}

pub fn describe_workspace_lease_holder(holder: Option<&WorkspaceLeaseInfo>) -> String {
    match holder {
        Some(info) => info.describe(),
        None => "unknown holder".to_string(),
    }
}

pub fn engine_locks_dir() -> PathBuf {
    if let Some(value) = std::env::var_os(ENGINE_LOCKS_DIR_ENV) {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    crate::daemon::paths::cache_root().join("engine-locks")
}

/// The lease key identifies the *index* an engine owns, not the directory it was launched from.
///
/// Callers must pass a root that already went through [`normalize_workspace_roots`], because that
/// is what makes the key agree with the codegraph store key: `codegraph::cg_highlev::codegraph_db_path`
/// derives `~/.cache/refact/codegraph/<project_hash>/` from
/// `files_correction::get_project_dirs` -> `project_dirs_for_unscoped_paths`, i.e. exactly the same
/// normalization this module uses. Two paths that normalize to the same root therefore share one
/// sqlite store, and the store only takes a *shared* advisory lease
/// (`refact_codegraph::store::try_acquire_shared_lease`), so it cannot stop two engines from
/// writing the same database. The workspace lease is what makes that pairing single-writer.
pub fn workspace_lease_key(root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root.to_string_lossy().as_bytes());
    hex::encode(hasher.finalize())
}

pub fn workspace_lease_path(root: &Path) -> PathBuf {
    engine_locks_dir().join(format!("{}.lock", workspace_lease_key(root)))
}

/// Holder details live beside the lock, never inside it.
///
/// `fd_lock` maps to `LockFileEx` on Windows, which is a *mandatory* byte-range lock: while the
/// lease is held, every other handle to that file fails to read it. Storing the JSON in the locked
/// file therefore made the holder unreadable on Windows exactly when a conflict needed reporting,
/// so a busy workspace could only ever be blamed on an "unknown holder".
pub fn workspace_lease_info_path(root: &Path) -> PathBuf {
    engine_locks_dir().join(format!("{}.json", workspace_lease_key(root)))
}

/// Collapses every alias of a workspace root onto the one path that owns its index.
///
/// Trailing slashes, `..` segments and symlinks collapse the obvious way. A registered managed
/// worktree (`~/.cache/refact/worktrees/<project hash>/<uuid>`) **deliberately** collapses onto its
/// project's `source_workspace_root`, so all worktrees of one project share a single lease key and
/// therefore a single engine. This is not an accident of path handling:
///
/// * The daemon never spawns a worker for a worktree. `ProjectRegistry::open` is only reached from
///   `POST /daemon/v1/projects/open`, whose `root` is the editor's workspace folder, and
///   `supervisor::worker_args` passes that same project root as `--workspace-folder`. Worktree
///   scoping happens *inside* the worker via `ExecutionScope`, which rewrites tool paths but never
///   asks for another engine.
/// * The codegraph store is keyed on the normalized root (see [`workspace_lease_key`]), so giving a
///   worktree its own lease would license a second engine to open and write the project's single
///   sqlite database concurrently, which the store's shared lease does not prevent.
///
/// Two genuinely different roots — including two worktrees belonging to *different* projects — still
/// produce different keys, because the collapse only applies to registry-backed worktree roots.
pub fn normalize_workspace_roots(cache_dir: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut normalized = crate::files_correction::project_dirs_for_unscoped_paths(cache_dir, roots);
    normalized.sort();
    normalized.dedup();
    normalized
}

pub fn workspace_lease_root_for(root: &Path) -> Option<PathBuf> {
    normalize_workspace_roots(&crate::daemon::paths::cache_root(), &[root.to_path_buf()])
        .into_iter()
        .next()
}

pub fn read_workspace_lease_holder(path: &Path) -> Option<WorkspaceLeaseInfo> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<WorkspaceLeaseInfo>(content.trim()).ok()
}

#[derive(Debug, Clone)]
pub struct WorkspaceLeaseConflict {
    pub root: PathBuf,
    pub path: PathBuf,
    pub holder: Option<WorkspaceLeaseInfo>,
}

impl WorkspaceLeaseConflict {
    pub fn holder_description(&self) -> String {
        describe_workspace_lease_holder(self.holder.as_ref())
    }
}

impl std::fmt::Display for WorkspaceLeaseConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{WORKSPACE_BUSY_MARKER}: another Refact engine already serves {} ({}); lease file {}",
            self.root.display(),
            self.holder_description(),
            self.path.display()
        )
    }
}

#[derive(Debug)]
pub enum WorkspaceLeaseError {
    Busy(WorkspaceLeaseConflict),
    Io { path: PathBuf, error: io::Error },
}

impl std::fmt::Display for WorkspaceLeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy(conflict) => write!(f, "{conflict}"),
            Self::Io { path, error } => {
                write!(
                    f,
                    "failed to take workspace lease {}: {error}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for WorkspaceLeaseError {}

pub struct WorkspaceLease {
    root: PathBuf,
    path: PathBuf,
    info_path: PathBuf,
    // The flock stays held because the descriptor stays open; dropping this frees it.
    _file: RwLock<File>,
}

impl WorkspaceLease {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn info_path(&self) -> &Path {
        &self.info_path
    }
}

impl std::fmt::Debug for WorkspaceLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceLease")
            .field("root", &self.root)
            .field("path", &self.path)
            .finish()
    }
}

fn write_lease_info(path: &Path, info: &WorkspaceLeaseInfo) -> io::Result<()> {
    let payload = serde_json::to_string(info)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Rename in so a reader racing the write never parses half a record.
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let mut file = File::create(&tmp)?;
    file.write_all(payload.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;
    drop(file);
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&tmp);
            Err(error)
        }
    }
}

pub fn try_acquire_workspace_lease(
    root: &Path,
    info: &WorkspaceLeaseInfo,
) -> Result<WorkspaceLease, WorkspaceLeaseError> {
    let path = workspace_lease_path(root);
    let info_path = workspace_lease_info_path(root);
    let mut lock = open_lock(&path).map_err(|error| WorkspaceLeaseError::Io {
        path: path.clone(),
        error,
    })?;
    let guard = match try_lock(&mut lock) {
        Ok(guard) => guard,
        Err(error) if is_already_locked(&error) => {
            return Err(WorkspaceLeaseError::Busy(WorkspaceLeaseConflict {
                root: root.to_path_buf(),
                holder: read_workspace_lease_holder(&info_path),
                path,
            }));
        }
        Err(error) => {
            return Err(WorkspaceLeaseError::Io {
                path: path.clone(),
                error,
            })
        }
    };
    if let Err(error) = write_lease_info(&info_path, info) {
        return Err(WorkspaceLeaseError::Io {
            path: info_path,
            error,
        });
    }
    // Forgetting the guard skips funlock; the flock stays held until the File descriptor closes,
    // i.e. until this WorkspaceLease is dropped.
    std::mem::forget(guard);
    Ok(WorkspaceLease {
        root: root.to_path_buf(),
        path,
        info_path,
        _file: lock,
    })
}

pub fn probe_workspace_lease(root: &Path) -> Option<WorkspaceLeaseConflict> {
    let path = workspace_lease_path(root);
    let file = OpenOptions::new().read(true).write(true).open(&path).ok()?;
    let mut lock = RwLock::new(file);
    let busy = match lock.try_write() {
        Ok(guard) => {
            drop(guard);
            false
        }
        Err(error) => is_already_locked(&error),
    };
    busy.then(|| WorkspaceLeaseConflict {
        root: root.to_path_buf(),
        holder: read_workspace_lease_holder(&workspace_lease_info_path(root)),
        path,
    })
}

#[derive(Debug, Default)]
pub struct WorkspaceLeaseSet {
    leases: Vec<WorkspaceLease>,
}

impl WorkspaceLeaseSet {
    pub fn roots(&self) -> Vec<PathBuf> {
        self.leases.iter().map(|lease| lease.root.clone()).collect()
    }

    pub fn holds(&self, root: &Path) -> bool {
        self.leases.iter().any(|lease| lease.root == root)
    }

    pub fn insert(&mut self, lease: WorkspaceLease) {
        if !self.holds(&lease.root) {
            self.leases.push(lease);
        }
    }

    pub fn release(&mut self, root: &Path) -> bool {
        let before = self.leases.len();
        self.leases.retain(|lease| lease.root != root);
        self.leases.len() != before
    }

    pub fn len(&self) -> usize {
        self.leases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }
}

pub fn acquire_workspace_leases(
    roots: &[PathBuf],
    info: &WorkspaceLeaseInfo,
) -> Result<WorkspaceLeaseSet, WorkspaceLeaseError> {
    let mut set = WorkspaceLeaseSet::default();
    for root in roots {
        match try_acquire_workspace_lease(root, info) {
            Ok(lease) => set.insert(lease),
            Err(error) => return Err(error),
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_lock_prevents_double_acquire() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.lock");
        let mut first = open_lock(&path).unwrap();
        let mut second = open_lock(&path).unwrap();
        let guard = try_lock(&mut first).unwrap();
        let error = try_lock(&mut second).unwrap_err();
        assert!(is_already_locked(&error));
        drop(guard);
        let _second_guard = try_lock(&mut second).unwrap();
    }
    struct LocksDirGuard {
        previous: Option<std::ffi::OsString>,
        _dir: tempfile::TempDir,
    }

    impl LocksDirGuard {
        fn set() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let previous = std::env::var_os(ENGINE_LOCKS_DIR_ENV);
            std::env::set_var(ENGINE_LOCKS_DIR_ENV, dir.path());
            Self {
                previous,
                _dir: dir,
            }
        }
    }

    impl Drop for LocksDirGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(ENGINE_LOCKS_DIR_ENV, value),
                None => std::env::remove_var(ENGINE_LOCKS_DIR_ENV),
            }
        }
    }

    const LEASE_HOLDER_ROOT_ENV: &str = "REFACT_TEST_LEASE_HOLDER_ROOT";

    fn test_info() -> WorkspaceLeaseInfo {
        WorkspaceLeaseInfo::for_current_process(8001, 8002, "refact-lsp")
    }

    #[test]
    #[ignore]
    fn workspace_lease_holder_child() {
        let Ok(root) = std::env::var(LEASE_HOLDER_ROOT_ENV) else {
            return;
        };
        let _lease = try_acquire_workspace_lease(Path::new(&root), &test_info()).unwrap();
        println!("lease taken");
        std::thread::sleep(std::time::Duration::from_secs(120));
    }

    #[test]
    #[serial_test::serial]
    fn workspace_lease_acquires_and_releases() {
        let _guard = LocksDirGuard::set();
        let root = tempfile::tempdir().unwrap();
        let lease = try_acquire_workspace_lease(root.path(), &test_info()).unwrap();
        assert!(lease.path().exists());
        let holder = read_workspace_lease_holder(lease.info_path()).unwrap();
        assert_eq!(holder.pid, std::process::id());
        assert_eq!(holder.http_port, 8001);
        let lease_path = lease.path().to_path_buf();
        drop(lease);
        let reacquired = try_acquire_workspace_lease(root.path(), &test_info()).unwrap();
        assert_eq!(reacquired.path(), lease_path.as_path());
    }

    #[test]
    #[serial_test::serial]
    fn workspace_lease_second_acquire_reports_holder() {
        let _guard = LocksDirGuard::set();
        let root = tempfile::tempdir().unwrap();
        let _first = try_acquire_workspace_lease(root.path(), &test_info()).unwrap();
        let error = try_acquire_workspace_lease(root.path(), &test_info()).unwrap_err();
        let WorkspaceLeaseError::Busy(conflict) = error else {
            panic!("expected busy error");
        };
        let holder = conflict
            .holder
            .clone()
            .expect("holder info must be readable");
        assert_eq!(holder.pid, std::process::id());
        assert_eq!(holder.http_port, 8001);
        assert!(conflict.to_string().contains(WORKSPACE_BUSY_MARKER));
        assert!(conflict.holder_description().contains("http port 8001"));
    }

    #[test]
    #[serial_test::serial]
    fn workspace_lease_holder_unreadable_when_file_is_malformed() {
        let _guard = LocksDirGuard::set();
        let root = tempfile::tempdir().unwrap();
        let path = workspace_lease_info_path(root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not json at all").unwrap();
        assert!(read_workspace_lease_holder(&path).is_none());
        assert_eq!(
            describe_workspace_lease_holder(None),
            "unknown holder".to_string()
        );
    }

    #[test]
    #[serial_test::serial]
    fn workspace_lease_probe_matches_lock_state() {
        let _guard = LocksDirGuard::set();
        let root = tempfile::tempdir().unwrap();
        assert!(probe_workspace_lease(root.path()).is_none());
        let lease = try_acquire_workspace_lease(root.path(), &test_info()).unwrap();
        let conflict = probe_workspace_lease(root.path()).expect("probe must see the holder");
        assert_eq!(
            conflict.holder.map(|info| info.pid),
            Some(std::process::id())
        );
        drop(lease);
        assert!(probe_workspace_lease(root.path()).is_none());
    }

    #[test]
    #[serial_test::serial]
    fn workspace_lease_survives_holder_crash() {
        let _guard = LocksDirGuard::set();
        let root = tempfile::tempdir().unwrap();
        let locks_dir = engine_locks_dir();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "daemon::lock::tests::workspace_lease_holder_child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(ENGINE_LOCKS_DIR_ENV, &locks_dir)
            .env(LEASE_HOLDER_ROOT_ENV, root.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if probe_workspace_lease(root.path()).is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child never took the workspace lease"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(try_acquire_workspace_lease(root.path(), &test_info()).is_err());
        child.kill().unwrap();
        let _ = child.wait().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Ok(lease) = try_acquire_workspace_lease(root.path(), &test_info()) {
                drop(lease);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "lease was not released after the holder was killed"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    #[test]
    #[serial_test::serial]
    fn workspace_lease_key_is_alias_insensitive() {
        let root = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let canonical = dunce::canonicalize(root.path()).unwrap();
        let with_slash = PathBuf::from(format!("{}/", canonical.display()));
        let normalized = normalize_workspace_roots(
            cache_dir.path(),
            &[canonical.clone(), with_slash, canonical.clone()],
        );
        assert_eq!(normalized, vec![canonical.clone()]);
        assert_eq!(
            workspace_lease_key(&normalized[0]),
            workspace_lease_key(&canonical)
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn workspace_lease_key_follows_symlinks() {
        let parent = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let real = parent.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = parent.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let normalized = normalize_workspace_roots(cache_dir.path(), &[link]);
        assert_eq!(normalized.len(), 1);
        assert_eq!(
            workspace_lease_key(&normalized[0]),
            workspace_lease_key(&dunce::canonicalize(&real).unwrap())
        );
    }

    /// Builds a real on-disk worktree registry so `normalize_workspace_roots` exercises the same
    /// `registered_worktree_path_mappings_uncached` validation the daemon runs in production.
    fn write_worktree_registry(
        cache_dir: &Path,
        source_root: &Path,
        worktree_ids: &[&str],
    ) -> Vec<PathBuf> {
        let project_hash = refact_worktrees::service::project_hash_for_path(source_root);
        let registry_dir = cache_dir.join("worktrees").join(&project_hash);
        std::fs::create_dir_all(&registry_dir).unwrap();
        let mut roots = Vec::new();
        let mut records = Vec::new();
        for id in worktree_ids {
            let root = registry_dir.join(id);
            std::fs::create_dir_all(&root).unwrap();
            let root = dunce::canonicalize(&root).unwrap();
            roots.push(root.clone());
            records.push(refact_worktrees::types::WorktreeRegistryRecord {
                meta: refact_worktrees::types::WorktreeMeta {
                    id: (*id).to_string(),
                    kind: "chat".to_string(),
                    root,
                    source_workspace_root: source_root.to_path_buf(),
                    repo_root: source_root.to_path_buf(),
                    branch: None,
                    base_branch: None,
                    base_commit: None,
                    task_id: None,
                    card_id: None,
                    agent_id: None,
                    enforce: true,
                },
                created_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
                last_seen_at: None,
                references: Vec::new(),
                last_known_status: None,
            });
        }
        let registry = refact_worktrees::types::WorktreeRegistry {
            schema_version: 1,
            source_workspace_root: source_root.to_path_buf(),
            project_hash,
            records,
        };
        std::fs::write(
            registry_dir.join("index.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        roots
    }

    /// Two managed worktrees of one project intentionally share a single lease key.
    ///
    /// They share `~/.cache/refact/codegraph/<project hash>/codegraph.sqlite`, and the store only
    /// takes a shared advisory lease, so a per-worktree lease would let two engines write one
    /// database. See the `normalize_workspace_roots` doc comment for the full argument.
    #[test]
    #[serial_test::serial]
    fn worktrees_of_one_project_intentionally_share_a_lease_key() {
        let cache_dir = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let source_root = dunce::canonicalize(source.path()).unwrap();
        let worktrees = write_worktree_registry(
            cache_dir.path(),
            &source_root,
            &[
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222",
            ],
        );

        let normalized = normalize_workspace_roots(cache_dir.path(), &worktrees);

        assert_eq!(
            normalized,
            vec![source_root.clone()],
            "both worktrees must normalize onto the project's source root"
        );
        assert_eq!(
            workspace_lease_key(&normalized[0]),
            workspace_lease_key(&source_root)
        );
        // Spelled out per root so a regression that keeps only one of the two is still caught.
        let first = normalize_workspace_roots(cache_dir.path(), &worktrees[..1]);
        let second = normalize_workspace_roots(cache_dir.path(), &worktrees[1..]);
        assert_eq!(
            workspace_lease_key(&first[0]),
            workspace_lease_key(&second[0]),
            "one engine per project: sibling worktrees share the codegraph store, so they \
             must share the lease that makes it single-writer"
        );
    }

    /// The collapse must not swallow genuinely different projects: worktrees of two different
    /// source roots still get two distinct lease keys, so two projects can run two engines.
    #[test]
    #[serial_test::serial]
    fn worktrees_of_different_projects_keep_distinct_lease_keys() {
        let cache_dir = tempfile::tempdir().unwrap();
        let first_source = tempfile::tempdir().unwrap();
        let second_source = tempfile::tempdir().unwrap();
        let first_root = dunce::canonicalize(first_source.path()).unwrap();
        let second_root = dunce::canonicalize(second_source.path()).unwrap();
        let first_worktrees = write_worktree_registry(
            cache_dir.path(),
            &first_root,
            &["33333333-3333-4333-8333-333333333333"],
        );
        let second_worktrees = write_worktree_registry(
            cache_dir.path(),
            &second_root,
            &["44444444-4444-4444-8444-444444444444"],
        );

        let normalized = normalize_workspace_roots(
            cache_dir.path(),
            &[first_worktrees[0].clone(), second_worktrees[0].clone()],
        );

        assert_eq!(normalized.len(), 2, "two projects must stay two roots");
        assert_ne!(
            workspace_lease_key(&normalized[0]),
            workspace_lease_key(&normalized[1])
        );
        assert!(normalized.contains(&first_root));
        assert!(normalized.contains(&second_root));
    }

    #[test]
    #[serial_test::serial]
    fn workspace_lease_set_rolls_back_on_conflict() {
        let _guard = LocksDirGuard::set();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let blocker = try_acquire_workspace_lease(second.path(), &test_info()).unwrap();
        let roots = vec![first.path().to_path_buf(), second.path().to_path_buf()];
        let error = acquire_workspace_leases(&roots, &test_info()).unwrap_err();
        assert!(matches!(error, WorkspaceLeaseError::Busy(_)));
        let recovered = try_acquire_workspace_lease(first.path(), &test_info());
        assert!(
            recovered.is_ok(),
            "first root must be released when a later root is busy"
        );
        drop(blocker);
    }

    #[test]
    #[serial_test::serial]
    fn workspace_lease_set_acquires_every_root() {
        let _guard = LocksDirGuard::set();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let roots = vec![first.path().to_path_buf(), second.path().to_path_buf()];
        let set = acquire_workspace_leases(&roots, &test_info()).unwrap();
        assert_eq!(set.len(), 2);
        assert!(set.holds(first.path()));
        assert!(probe_workspace_lease(second.path()).is_some());
        drop(set);
        assert!(probe_workspace_lease(first.path()).is_none());
        assert!(probe_workspace_lease(second.path()).is_none());
    }
}
