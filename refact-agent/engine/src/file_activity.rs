use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{RwLock as AsyncRwLock, OwnedRwLockReadGuard, OwnedRwLockWriteGuard};

tokio::task_local! {
    pub static TOOL_ACTOR: String;
}

pub fn current_actor() -> Option<String> {
    TOOL_ACTOR
        .try_with(Clone::clone)
        .ok()
        .filter(|actor| !actor.is_empty())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WriteIntent {
    Create,
    Overwrite { authorized: bool },
    Contextual,
    Delete,
    Rename,
    Tree,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteKind {
    Created,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReadObservation {
    pub actor: String,
    pub key: String,
    pub path: String,
    pub alias: Option<String>,
    pub line1: usize,
    pub line2: usize,
    pub total_lines: usize,
    pub whole: bool,
    pub partial_reason: Option<String>,
    pub bytes: u64,
    pub digest: String,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct WriteObservation {
    pub actor: Option<String>,
    pub key: String,
    pub path: String,
    pub alias: Option<String>,
    pub kind: WriteKind,
    pub tool: String,
    pub attributed: bool,
    pub bytes_after: Option<u64>,
    pub digest_after: Option<String>,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Observations {
    pub reads: Vec<ReadObservation>,
    pub writes: Vec<WriteObservation>,
    pub reads_dropped: u64,
    pub writes_dropped: u64,
}

#[derive(Default)]
struct Registry {
    alias_to_key: HashMap<String, String>,
    key_to_alias: HashMap<String, String>,
    key_to_path: HashMap<String, String>,
    reads: Vec<ReadObservation>,
    writes: Vec<WriteObservation>,
    reads_dropped: u64,
    writes_dropped: u64,
}

fn registry() -> &'static RwLock<Registry> {
    static SLOT: OnceLock<RwLock<Registry>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(Registry::default()))
}

static OBSERVATIONS_REVISION: AtomicU64 = AtomicU64::new(0);

pub fn observations_revision() -> u64 {
    OBSERVATIONS_REVISION.load(Ordering::Acquire)
}

fn read_registry<R>(f: impl FnOnce(&Registry) -> R) -> R {
    let guard = registry()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&guard)
}

fn write_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    let mut guard = registry()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

pub fn settings() -> refact_work_core::CoordinationSettings {
    refact_work_core::coordination_settings()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn digest_of(content: &str) -> String {
    digest_bytes(content.as_bytes())
}

pub fn digest_bytes(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    let out = hasher.finalize();
    hex::encode(&out[..16])
}

pub fn short_digest(digest: &str) -> String {
    digest.chars().take(12).collect()
}

fn canonical_string(path: &Path) -> String {
    let resolved = crate::coordination::scope::canonical_candidate(path)
        .unwrap_or_else(|_| path.to_path_buf());
    resolved.to_string_lossy().to_string()
}

fn file_alias(path: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(md) = std::fs::metadata(path) {
            return Some(format!("id:{}:{}", md.dev(), md.ino()));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    None
}

/// Collapses every spelling and hard link of one physical file onto a single
/// stable key: the first canonical path observed for that file identity.
pub fn resolve_key(path: &Path) -> (String, Option<String>) {
    let canonical = canonical_string(path);
    let alias = file_alias(path);
    let Some(alias) = alias else {
        return (canonical, None);
    };
    write_registry(|reg| {
        if let Some(existing) = reg.alias_to_key.get(&alias).cloned() {
            if file_alias(Path::new(&existing)).as_ref() != Some(&alias) {
                reg.alias_to_key.remove(&alias);
            }
        }
        let cap = settings()
            .max_read_stamps
            .saturating_add(settings().max_write_records);
        if reg.alias_to_key.len() >= cap {
            let retained: std::collections::HashSet<_> = reg
                .reads
                .iter()
                .map(|read| read.key.clone())
                .chain(reg.writes.iter().map(|write| write.key.clone()))
                .collect();
            reg.alias_to_key.retain(|_, key| retained.contains(key));
            reg.key_to_alias.retain(|key, _| retained.contains(key));
            reg.key_to_path.retain(|key, _| retained.contains(key));
        }
        let key = match reg.alias_to_key.get(&alias) {
            Some(existing) => existing.clone(),
            None => {
                reg.alias_to_key.insert(alias.clone(), canonical.clone());
                canonical.clone()
            }
        };
        reg.key_to_alias.insert(key.clone(), alias.clone());
        reg.key_to_path.insert(key.clone(), canonical.clone());
        (key, Some(alias))
    })
}

fn current_state(path: &Path) -> Result<Option<(String, u64)>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some((digest_bytes(&bytes), bytes.len() as u64))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Failed to inspect {:?}: {}", path, e)),
    }
}
static FILE_FSYNCS: AtomicU64 = AtomicU64::new(0);
static DIR_FSYNCS: AtomicU64 = AtomicU64::new(0);
static DURABLE_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

const MAX_SYMLINK_HOPS: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DurabilityCounters {
    pub file_fsyncs: u64,
    pub dir_fsyncs: u64,
}

/// Observable seam: every durable file and directory fsync is counted here, so
/// a test can assert the syscall actually happened instead of trusting a claim.
pub fn durability_counters() -> DurabilityCounters {
    DurabilityCounters {
        file_fsyncs: FILE_FSYNCS.load(Ordering::Relaxed),
        dir_fsyncs: DIR_FSYNCS.load(Ordering::Relaxed),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplaceMode {
    /// The file must not exist, so a racing creator still loses the create.
    CreateNew,
    /// Replace existing content, creating the file when it is absent.
    Replace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplacementKind {
    Created,
    Renamed,
    InPlaceHardLinked,
}

/// Test-only record of which exact paths were fsynced, so a test can assert
/// the syscall happened for its own file and directory instead of watching a
/// global counter that every other parallel test also moves.
#[cfg(test)]
pub(crate) mod durable_observed {
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    fn slot() -> &'static Mutex<(Vec<String>, Vec<String>)> {
        static SLOT: OnceLock<Mutex<(Vec<String>, Vec<String>)>> = OnceLock::new();
        SLOT.get_or_init(|| Mutex::new((Vec::new(), Vec::new())))
    }

    pub fn record_file(path: &Path) {
        slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0
            .push(path.to_string_lossy().to_string());
    }

    pub fn record_dir(path: &Path) {
        slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .1
            .push(path.to_string_lossy().to_string());
    }

    pub fn file_fsyncs_under(prefix: &Path) -> usize {
        let prefix = prefix.to_string_lossy().to_string();
        let guard = slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .0
            .iter()
            .filter(|path| path.starts_with(&prefix))
            .count()
    }

    pub fn dir_fsyncs_for(dir: &Path) -> usize {
        let dir = dir.to_string_lossy().to_string();
        let guard = slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.1.iter().filter(|path| *path == &dir).count()
    }
}

/// Test-only crash simulation. It stops a replacement at exactly the two
/// points that matter — after the temp file is durable but before the rename,
/// and after the rename but before the directory fsync — optionally only for
/// paths matching a needle, so a multi-file apply can be failed at one file.
#[cfg(test)]
pub(crate) mod durable_fault {
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    pub const AFTER_TEMP_WRITE: u8 = 1;
    pub const AFTER_RENAME: u8 = 2;

    type Fault = Option<(u8, String)>;

    fn slot() -> &'static Mutex<Fault> {
        static SLOT: OnceLock<Mutex<Fault>> = OnceLock::new();
        SLOT.get_or_init(|| Mutex::new(None))
    }

    fn set(fault: Fault) {
        *slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = fault;
    }

    pub struct Armed;

    impl Drop for Armed {
        fn drop(&mut self) {
            set(None);
        }
    }

    /// Arming is always scoped to a path needle so an armed fault can never
    /// reach a file belonging to another test running in parallel. Tests that
    /// arm must also share one `#[serial]` key, because only one fault is held
    /// at a time.
    pub fn arm_for(point: u8, needle: impl Into<String>) -> Armed {
        set(Some((point, needle.into())));
        Armed
    }

    pub fn triggered(point: u8, path: &Path) -> bool {
        match &*slot()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            Some((armed, needle)) => {
                *armed == point && path.to_string_lossy().contains(needle.as_str())
            }
            None => false,
        }
    }
}

fn temp_path_for(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let unique = DURABLE_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(
        ".{}.{}.{}.durable-tmp",
        name,
        std::process::id(),
        unique
    ))
}

fn hard_link_count(metadata: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.nlink()
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        1
    }
}

/// Resolves the inode that should receive the bytes. `open()` follows
/// symlinks, so replacing the link's target (rather than the link itself)
/// reproduces the behaviour an in-place write already had. This picks a write
/// destination only; file identity for locking still comes solely from
/// `resolve_key`.
async fn resolve_symlink_target(path: &Path) -> Result<PathBuf, String> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_SYMLINK_HOPS {
        let metadata = match tokio::fs::symlink_metadata(&current).await {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(e) => return Err(format!("Failed to inspect {:?}: {}", current, e)),
        };
        if !metadata.file_type().is_symlink() {
            return Ok(current);
        }
        let link = tokio::fs::read_link(&current)
            .await
            .map_err(|e| format!("Failed to read link {:?}: {}", current, e))?;
        current = if link.is_absolute() {
            link
        } else {
            current
                .parent()
                .map(|parent| parent.join(&link))
                .unwrap_or(link)
        };
    }
    Err(format!(
        "{:?} exceeds {} symlink hops",
        path, MAX_SYMLINK_HOPS
    ))
}

async fn sync_file(file: &tokio::fs::File, path: &Path) -> Result<(), String> {
    file.sync_all()
        .await
        .map_err(|e| format!("Failed to fsync {:?}: {}", path, e))?;
    FILE_FSYNCS.fetch_add(1, Ordering::Relaxed);
    #[cfg(test)]
    durable_observed::record_file(path);
    Ok(())
}

/// Forces the directory entry itself to disk. Without this the rename that
/// published the new file can be lost even though the file contents survived.
pub async fn sync_dir(dir: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        let owned = dir.to_path_buf();
        tokio::task::spawn_blocking(move || std::fs::File::open(&owned)?.sync_all())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("Failed to fsync directory {:?}: {}", dir, e))?;
        DIR_FSYNCS.fetch_add(1, Ordering::Relaxed);
        #[cfg(test)]
        durable_observed::record_dir(dir);
        Ok(())
    }
    // Windows has no directory-fsync equivalent: a directory handle cannot be
    // flushed, so only the file contents above are forced to disk.
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

async fn create_exclusively(target: &Path, contents: &[u8]) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .await
        .map_err(|e| format!("Failed to create {:?}: {}", target, e))?;
    let written = async {
        file.write_all(contents)
            .await
            .map_err(|e| format!("Failed to write {:?}: {}", target, e))?;
        file.flush()
            .await
            .map_err(|e| format!("Failed to flush {:?}: {}", target, e))?;
        sync_file(&file, target).await
    }
    .await;
    drop(file);
    if let Err(error) = written {
        let _ = tokio::fs::remove_file(target).await;
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) struct DurableRenameBarrier {
    pub target: PathBuf,
    pub staged: Arc<tokio::sync::Notify>,
    pub release: Arc<tokio::sync::Notify>,
    pub fail: bool,
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static BEFORE_DURABLE_RENAME: DurableRenameBarrier;
}

pub(crate) async fn write_temp_then_rename(
    target: &Path,
    contents: &[u8],
    permissions: Option<std::fs::Permissions>,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let tmp = temp_path_for(target);
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .await
            .map_err(|e| format!("Failed to create {:?}: {}", tmp, e))?;
        let written = async {
            file.write_all(contents)
                .await
                .map_err(|e| format!("Failed to write {:?}: {}", tmp, e))?;
            file.flush()
                .await
                .map_err(|e| format!("Failed to flush {:?}: {}", tmp, e))?;
            sync_file(&file, &tmp).await
        }
        .await;
        drop(file);
        written?;
        if let Some(permissions) = permissions {
            tokio::fs::set_permissions(&tmp, permissions)
                .await
                .map_err(|e| format!("Failed to set permissions on {:?}: {}", tmp, e))?;
        }
        #[cfg(test)]
        if let Ok(Some((staged, release, fail))) = BEFORE_DURABLE_RENAME.try_with(|barrier| {
            (barrier.target == target).then(|| {
                (
                    barrier.staged.clone(),
                    barrier.release.clone(),
                    barrier.fail,
                )
            })
        }) {
            staged.notify_one();
            release.notified().await;
            if fail {
                return Err("injected failure before durable rename".to_string());
            }
        }
        #[cfg(test)]
        if durable_fault::triggered(durable_fault::AFTER_TEMP_WRITE, target) {
            return Err("injected failure after temp write".to_string());
        }
        // A plain rename replaces the destination on both unix and Windows.
        // The target is never removed first, so it is never briefly absent.
        tokio::fs::rename(&tmp, target)
            .await
            .map_err(|e| format!("Failed to rename {:?} to {:?}: {}", tmp, target, e))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

async fn write_in_place(target: &Path, contents: &[u8]) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(target)
        .await
        .map_err(|e| format!("Failed to open {:?}: {}", target, e))?;
    let written = async {
        file.write_all(contents)
            .await
            .map_err(|e| format!("Failed to write {:?}: {}", target, e))?;
        file.flush()
            .await
            .map_err(|e| format!("Failed to flush {:?}: {}", target, e))?;
        sync_file(&file, target).await
    }
    .await;
    drop(file);
    written
}

/// Crash-durable whole-file write. What is actually guaranteed, by outcome:
///
/// * `Created` — the file did not exist. It is created exclusively, written,
///   fsynced, and the parent directory is fsynced. A crash part-way can leave
///   a short new file, but no previous content can be lost.
/// * `Renamed` — a temp file in the same directory is written, fsynced, given
///   the target's permissions and renamed over it, then the parent directory
///   is fsynced. A crash leaves either the whole old file or the whole new
///   file, never a truncated one.
/// * `InPlaceHardLinked` — the target has more than one hard link. Renaming
///   would break that link and split one file into two, so the content is
///   written in place and fsynced. This is durable but NOT atomic: a crash can
///   leave partial content. It is the only outcome that can.
///
/// A symlinked target is resolved first: the link is left alone and the file
/// it points at is replaced. Permissions are preserved; ownership is preserved
/// only in the in-place case, because the renamed replacement is a new inode
/// owned by this process.
///
/// On Windows the rename still replaces the destination without removing it
/// first, but there is no directory fsync, so only file contents are forced.
pub async fn write_file_durably(
    path: &Path,
    contents: &[u8],
    mode: ReplaceMode,
) -> Result<ReplacementKind, String> {
    let target = resolve_symlink_target(path).await?;
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{:?} has no parent directory", target))?;

    let existing = match tokio::fs::metadata(&target).await {
        Ok(metadata) => Some(metadata),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Failed to inspect {:?}: {}", target, e)),
    };

    if matches!(mode, ReplaceMode::CreateNew) || existing.is_none() {
        create_exclusively(&target, contents).await?;
        sync_dir(&parent).await?;
        return Ok(ReplacementKind::Created);
    }

    let metadata = existing.ok_or_else(|| format!("{:?} disappeared", target))?;
    if !metadata.is_file() {
        return Err(format!("{:?} is not a regular file", target));
    }
    if hard_link_count(&metadata) > 1 {
        write_in_place(&target, contents).await?;
        return Ok(ReplacementKind::InPlaceHardLinked);
    }
    write_temp_then_rename(&target, contents, Some(metadata.permissions())).await?;
    #[cfg(test)]
    if durable_fault::triggered(durable_fault::AFTER_RENAME, &target) {
        return Err("injected failure after rename".to_string());
    }
    sync_dir(&parent).await?;
    Ok(ReplacementKind::Renamed)
}

fn lock_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("REFACT_FILE_ACTIVITY_LOCK_DIR") {
        if !custom.trim().is_empty() {
            return PathBuf::from(custom);
        }
    }
    std::env::temp_dir().join("refact-file-activity-locks")
}

fn lock_path_for(key: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let digest = hex::encode(&hasher.finalize()[..16]);
    lock_dir().join(format!("{}.lock", digest))
}

fn process_locks() -> &'static std::sync::Mutex<HashMap<String, std::sync::Weak<AsyncRwLock<()>>>> {
    static SLOT: OnceLock<std::sync::Mutex<HashMap<String, std::sync::Weak<AsyncRwLock<()>>>>> =
        OnceLock::new();
    SLOT.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn process_lock_for(key: &str) -> Arc<AsyncRwLock<()>> {
    let mut locks = process_locks()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(key).and_then(std::sync::Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(AsyncRwLock::new(()));
    locks.insert(key.into(), Arc::downgrade(&lock));
    lock
}

fn open_lock_file(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    Ok(file)
}

async fn acquire_os_lock(
    path: PathBuf,
    deadline: Instant,
    poll: Duration,
    shared: bool,
) -> Result<File, String> {
    let mut file = tokio::task::spawn_blocking(move || open_lock_file(&path))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    loop {
        let (next, result) = tokio::task::spawn_blocking(move || {
            let result = if shared {
                file.try_lock_shared()
            } else {
                file.try_lock()
            };
            (file, result)
        })
        .await
        .map_err(|error| error.to_string())?;
        file = next;
        match result {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(error) => return Err(error.to_string()),
        }
        if Instant::now() >= deadline {
            return Err("Timed out waiting for cooperative file lock".into());
        }
        tokio::time::sleep(poll.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
}

struct HeldLock {
    _process_read: Option<OwnedRwLockReadGuard<()>>,
    _process_write: Option<OwnedRwLockWriteGuard<()>>,
    _file: File,
}

async fn hold(
    key: &str,
    shared: bool,
    deadline: Instant,
    poll: Duration,
) -> Result<HeldLock, String> {
    let process_lock = process_lock_for(key);
    let (read, write) = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
        if shared {
            (Some(process_lock.read_owned().await), None)
        } else {
            (None, Some(process_lock.write_owned().await))
        }
    })
    .await
    .map_err(|_| format!("Timed out waiting for the in-process file lock on {}", key))?;
    let file = acquire_os_lock(lock_path_for(key), deadline, poll, shared).await?;
    Ok(HeldLock {
        _process_read: read,
        _process_write: write,
        _file: file,
    })
}

pub struct WriteGuard {
    actor: Option<String>,
    key: String,
    alias: Option<String>,
    path: PathBuf,
    tool: String,
    digest_before: Option<String>,
    committed: bool,
    _locks: Arc<Vec<HeldLock>>,
}

impl WriteGuard {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn commit(self, kind: WriteKind, after: Option<&str>) {
        self.commit_bytes(kind, after.map(str::as_bytes));
    }

    pub fn commit_bytes(mut self, kind: WriteKind, after: Option<&[u8]>) {
        self.record_bytes(kind, after);
    }

    pub fn record_bytes(&mut self, kind: WriteKind, after: Option<&[u8]>) {
        self.committed = true;
        let at_ms = now_ms();
        let digest_after = after.map(digest_bytes);
        let bytes_after = after.map(|c| c.len() as u64);
        let observation = WriteObservation {
            actor: self.actor.clone(),
            key: self.key.clone(),
            path: self.path.to_string_lossy().to_string(),
            alias: self.alias.clone(),
            kind,
            tool: self.tool.clone(),
            attributed: self.actor.is_some(),
            bytes_after,
            digest_after: digest_after.clone(),
            at_ms,
        };
        let max_writes = settings().max_write_records.max(1);
        crate::coordination::file_provenance::observe(
            crate::coordination::file_provenance::ObservedActivity {
                file_key: self.key.clone(),
                file_alias: self.alias.clone(),
                path: observation.path.clone(),
                actor: self.actor.clone(),
                tool: self.tool.clone(),
                access: refact_work_core::AccessKind::Write,
                digest_before: self.digest_before.clone(),
                digest_after,
                at_ms: at_ms as i64,
            },
        );
        write_registry(move |reg| {
            reg.writes.push(observation);
            while reg.writes.len() > max_writes {
                reg.writes.remove(0);
                reg.writes_dropped = reg.writes_dropped.saturating_add(1);
            }
            OBSERVATIONS_REVISION.fetch_add(1, Ordering::Release);
        });
    }
}

pub async fn begin_write(
    actor: Option<&str>,
    path: &Path,
    intent: WriteIntent,
    tool: &str,
) -> Result<WriteGuard, String> {
    begin_writes(actor, &[(path.to_path_buf(), intent)], tool)
        .await?
        .pop()
        .ok_or_else(|| "missing file write guard".into())
}

pub async fn begin_writes(
    actor: Option<&str>,
    paths: &[(PathBuf, WriteIntent)],
    tool: &str,
) -> Result<Vec<WriteGuard>, String> {
    let cfg = settings();
    if paths.is_empty() || paths.len() > cfg.max_write_records {
        return Err("file transaction path count exceeds configured limit".into());
    }
    for (path, _) in paths {
        crate::coordination::scope::canonical_candidate(path).map_err(|error| error.to_string())?;
    }
    crate::coordination::file_provenance::flush().await;
    let deadline = Instant::now() + Duration::from_secs(cfg.lock_timeout_secs.max(1));
    let poll = Duration::from_millis(cfg.lock_poll_interval_ms.clamp(1, 5_000));

    let lock_keys = || {
        let mut keys = std::collections::BTreeMap::new();
        for (path, intent) in paths {
            let canonical = PathBuf::from(canonical_string(path));
            keys.insert(format!("path:{}", canonical.display()), false);
            for parent in canonical.ancestors().skip(1) {
                keys.entry(format!("directory:{}", parent.display()))
                    .or_insert(true);
            }
            if matches!(intent, WriteIntent::Tree) {
                keys.insert(format!("directory:{}", canonical.display()), false);
            }
            if let Some(alias) = file_alias(path) {
                keys.insert(alias, false);
            }
        }
        keys
    };
    let locks = loop {
        let keys = lock_keys();
        let mut locks = Vec::with_capacity(keys.len());
        for (key, shared) in &keys {
            locks.push(hold(key, *shared, deadline, poll).await?);
        }
        if keys == lock_keys() {
            break locks;
        }
        drop(locks);
        if Instant::now() >= deadline {
            return Err("file identity changed while acquiring cooperative locks".into());
        }
    };

    let locks = Arc::new(locks);
    let mut guards = Vec::with_capacity(paths.len());
    for (path, intent) in paths {
        guards.push(validate_write(
            actor,
            path,
            *intent,
            tool,
            locks.clone(),
            &cfg,
        )?);
    }
    Ok(guards)
}

fn validate_write(
    actor: Option<&str>,
    path: &Path,
    intent: WriteIntent,
    tool: &str,
    locks: Arc<Vec<HeldLock>>,
    cfg: &refact_work_core::CoordinationSettings,
) -> Result<WriteGuard, String> {
    let (key, alias) = resolve_key(path);
    let before = match intent {
        WriteIntent::Tree => None,
        _ => current_state(path)?,
    };
    let guard = WriteGuard {
        actor: actor.map(|a| a.to_string()),
        key: key.clone(),
        alias,
        path: path.to_path_buf(),
        tool: tool.to_string(),
        digest_before: before.as_ref().map(|(digest, _)| digest.clone()),
        committed: false,
        _locks: locks,
    };

    if !cfg.enabled {
        return Ok(guard);
    }

    if matches!(intent, WriteIntent::Tree) {
        if path.exists() && !path.is_dir() {
            return Err("directory transaction requires a directory"
                .to_string()
                .into());
        }
        return Ok(guard);
    }

    match intent {
        WriteIntent::Create if before.is_some() => {
            return Err(format!(
                "⚠️ {:?} already exists. 💡 cat() it first, then edit it instead of creating it.",
                path
            ));
        }
        WriteIntent::Delete | WriteIntent::Rename if before.is_none() => {
            return Err(format!("⚠️ {:?} does not exist.", path));
        }
        _ => {}
    }
    Ok(guard)
}

pub fn record_visible_read(
    actor: &str,
    file: &refact_core::chat_types::ContextFile,
    text: &str,
) -> ReadObservation {
    let expected: Vec<_> = text.lines().collect();
    let total = expected.len();
    let shown: Vec<_> = file
        .file_content
        .lines()
        .filter_map(|line| {
            let (number, content) = line.split_once(" | ")?;
            Some((number.trim().parse::<usize>().ok()?, content))
        })
        .collect();
    let whole = file.line1 <= 1
        && file.line2 >= total
        && shown.len() == total
        && shown
            .iter()
            .enumerate()
            .all(|(index, (line, content))| *line == index + 1 && *content == expected[index]);
    record_read(
        Some(actor),
        Path::new(&file.file_name),
        text,
        file.line1,
        file.line2,
        total,
        (!whole).then(|| "only the rendered range survived output processing".into()),
    )
}

/// Records what a chat actually saw. `content` must be the full authoritative
/// text that was loaded, even when only part of it was rendered.
pub fn record_read(
    actor: Option<&str>,
    path: &Path,
    content: &str,
    line1: usize,
    line2: usize,
    total_lines: usize,
    partial_reason: Option<String>,
) -> ReadObservation {
    let (key, alias) = resolve_key(path);
    let digest = digest_of(content);
    let at_ms = now_ms();
    let whole = partial_reason.is_none() && line1 <= 1 && line2 >= total_lines;
    let observation = ReadObservation {
        actor: actor.unwrap_or_default().to_string(),
        key: key.clone(),
        path: path.to_string_lossy().to_string(),
        alias,
        line1,
        line2,
        total_lines,
        whole,
        partial_reason: partial_reason.clone(),
        bytes: content.len() as u64,
        digest: digest.clone(),
        at_ms,
    };
    let max_reads = settings().max_read_stamps.max(1);
    let stored = observation.clone();
    let actor_owned = actor.map(|a| a.to_string());
    crate::coordination::file_provenance::observe(
        crate::coordination::file_provenance::ObservedActivity {
            file_key: key.clone(),
            file_alias: observation.alias.clone(),
            path: observation.path.clone(),
            actor: actor_owned.clone(),
            tool: "read".to_string(),
            access: refact_work_core::AccessKind::Read,
            digest_before: None,
            digest_after: Some(digest.clone()),
            at_ms: at_ms as i64,
        },
    );
    write_registry(move |reg| {
        reg.reads.push(stored);
        while reg.reads.len() > max_reads {
            reg.reads.remove(0);
            reg.reads_dropped = reg.reads_dropped.saturating_add(1);
        }
        OBSERVATIONS_REVISION.fetch_add(1, Ordering::Release);
    });
    observation
}

pub fn render_stamp(observation: &ReadObservation) -> String {
    if observation.whole {
        format!(
            "{} — full read, {} lines, {} bytes, content {}",
            observation.path,
            observation.total_lines,
            observation.bytes,
            short_digest(&observation.digest)
        )
    } else {
        let reason = observation
            .partial_reason
            .clone()
            .unwrap_or_else(|| format!("lines {}-{} shown", observation.line1, observation.line2));
        format!(
            "{} — partial read ({} of {} lines: {}), content {}",
            observation.path,
            observation.line2.saturating_sub(observation.line1) + 1,
            observation.total_lines,
            reason,
            short_digest(&observation.digest)
        )
    }
}

pub fn render_stamp_section(observations: &[ReadObservation]) -> String {
    if observations.is_empty() {
        return String::new();
    }
    let mut out = String::from("Read stamps:\n");
    for observation in observations {
        out.push_str(&render_stamp(observation));
        out.push('\n');
    }
    out.push('\n');
    out
}

/// Query surface for the work catalogue. `project` filters by path prefix.
pub fn observations(project: Option<&str>) -> Observations {
    read_registry(|reg| {
        let keep = |path: &str| match project {
            Some(prefix) if !prefix.is_empty() => Path::new(path).starts_with(Path::new(prefix)),
            _ => true,
        };
        Observations {
            reads: reg
                .reads
                .iter()
                .filter(|r| keep(&r.path) || keep(&r.key))
                .cloned()
                .collect(),
            writes: reg
                .writes
                .iter()
                .filter(|w| keep(&w.path) || keep(&w.key))
                .cloned()
                .collect(),
            reads_dropped: reg.reads_dropped,
            writes_dropped: reg.writes_dropped,
        }
    })
}

pub fn reset_for_tests() {
    write_registry(|reg| {
        *reg = Registry::default();
        OBSERVATIONS_REVISION.fetch_add(1, Ordering::Release);
    });
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn read_and_write_observations_publish_revisions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("observed.txt");
        std::fs::write(&path, "fixture").unwrap();
        let before = observations_revision();
        record_read(Some("revision-reader"), &path, "fixture", 1, 1, 1, None);
        assert!(observations_revision() > before);
        let guard = begin_write(None, &path, WriteIntent::Contextual, "revision-writer")
            .await
            .unwrap();
        let before_write = observations_revision();
        guard.commit(WriteKind::Modified, Some("fixture"));
        assert!(observations_revision() > before_write);
    }

    #[tokio::test]
    async fn directory_mutation_excludes_descendant_writes_but_siblings_can_proceed() {
        let dir = tempfile::tempdir().unwrap();
        let subtree = dir.path().join("tree");
        std::fs::create_dir(&subtree).unwrap();
        let file = subtree.join("file");
        let sibling = dir.path().join("sibling");
        std::fs::write(&file, "before").unwrap();
        std::fs::write(&sibling, "sibling").unwrap();
        let held = begin_write(None, &file, WriteIntent::Contextual, "writer")
            .await
            .unwrap();
        let mut deletion = Box::pin(begin_write(None, &subtree, WriteIntent::Tree, "delete"));
        assert!(futures::poll!(deletion.as_mut()).is_pending());
        let other = begin_write(None, &sibling, WriteIntent::Contextual, "sibling")
            .await
            .unwrap();
        drop(other);
        drop(held);
        let directory = deletion.await.unwrap();
        let mut writer = Box::pin(begin_write(None, &file, WriteIntent::Contextual, "writer"));
        assert!(futures::poll!(writer.as_mut()).is_pending());
        drop(directory);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn directory_mutation_accepts_changed_descendant_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        std::fs::write(&path, "before").unwrap();
        record_read(Some("tree-writer"), &path, "before", 1, 1, 1, None);
        std::fs::write(&path, "changed").unwrap();
        assert!(
            begin_write(Some("tree-writer"), dir.path(), WriteIntent::Tree, "delete")
                .await
                .is_ok()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "changed");
    }
    use super::*;

    fn count_temp_files(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".durable-tmp"))
            .count()
    }

    #[tokio::test]
    async fn replacement_is_atomic_and_fsyncs_the_file_and_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, "old\r\ncontent\r\n").unwrap();
        let before = durability_counters();

        let kind = write_file_durably(&path, b"new\r\ncontent\r\n", ReplaceMode::Replace)
            .await
            .unwrap();

        assert_eq!(kind, ReplacementKind::Renamed);
        assert_eq!(std::fs::read(&path).unwrap(), b"new\r\ncontent\r\n");
        assert!(
            durable_observed::file_fsyncs_under(dir.path()) >= 1,
            "the replacement contents were never fsynced"
        );
        #[cfg(unix)]
        assert_eq!(
            durable_observed::dir_fsyncs_for(dir.path()),
            1,
            "parent directory was not fsynced, so the rename may not survive power loss"
        );
        assert!(durability_counters().file_fsyncs > before.file_fsyncs);
        assert_eq!(count_temp_files(dir.path()), 0);
    }

    #[tokio::test]
    async fn replacement_preserves_permissions_and_arbitrary_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.bin");
        std::fs::write(&path, [0x00, 0xff, 0xfe, b'\r', b'\n']).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        }

        write_file_durably(&path, &[0xff, 0x00, b'\n'], ReplaceMode::Replace)
            .await
            .unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), [0xff, 0x00, b'\n']);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o640, "permissions were not preserved");
        }
    }

    #[tokio::test]
    #[serial_test::serial(durable_fault)]
    async fn failure_before_the_rename_leaves_the_original_byte_identical_and_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, "original").unwrap();
        let armed = durable_fault::arm_for(
            durable_fault::AFTER_TEMP_WRITE,
            path.to_string_lossy().to_string(),
        );

        let error = write_file_durably(&path, b"replacement", ReplaceMode::Replace)
            .await
            .unwrap_err();
        drop(armed);

        assert!(
            error.contains("injected failure after temp write"),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(
            count_temp_files(dir.path()),
            0,
            "temp file was not cleaned up"
        );
    }

    #[tokio::test]
    #[serial_test::serial(durable_fault)]
    async fn failure_after_the_rename_leaves_the_new_content_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, "original").unwrap();
        let armed = durable_fault::arm_for(
            durable_fault::AFTER_RENAME,
            path.to_string_lossy().to_string(),
        );

        let error = write_file_durably(&path, b"replacement", ReplaceMode::Replace)
            .await
            .unwrap_err();
        drop(armed);

        assert!(error.contains("injected failure after rename"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "replacement");
        assert_eq!(count_temp_files(dir.path()), 0);
    }

    #[tokio::test]
    async fn create_new_refuses_an_existing_file_and_never_overwrites_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, "existing").unwrap();

        assert!(
            write_file_durably(&path, b"clobber", ReplaceMode::CreateNew)
                .await
                .is_err()
        );

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_symlinked_target_keeps_the_link_and_replaces_the_file_it_points_at() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        std::fs::write(&target, "old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let kind = write_file_durably(&link, b"new", ReplaceMode::Replace)
            .await
            .unwrap();

        assert_eq!(kind, ReplacementKind::Renamed);
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the symlink was replaced by a regular file"
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_hardlinked_target_is_written_in_place_so_both_names_stay_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        let alias = dir.path().join("alias.txt");
        std::fs::write(&path, "old").unwrap();
        std::fs::hard_link(&path, &alias).unwrap();

        let kind = write_file_durably(&path, b"new", ReplaceMode::Replace)
            .await
            .unwrap();

        assert_eq!(kind, ReplacementKind::InPlaceHardLinked);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(
            std::fs::read_to_string(&alias).unwrap(),
            "new",
            "the hard link was broken instead of being written through"
        );
        assert!(
            durable_observed::file_fsyncs_under(dir.path()) >= 1,
            "the in-place write was not fsynced"
        );
        assert_eq!(count_temp_files(dir.path()), 0);
    }

    #[test]
    fn byte_digests_distinguish_invalid_utf8_sequences() {
        assert_ne!(digest_bytes(&[0xff]), digest_bytes(&[0xfe]));
        assert_eq!(digest_bytes(b"same"), digest_of("same"));
    }

    #[tokio::test]
    async fn opposite_path_order_cannot_deadlock_and_partial_guard_drop_keeps_lock_set() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let mut guards = begin_writes(
            None,
            &[
                (a.clone(), WriteIntent::Contextual),
                (b.clone(), WriteIntent::Contextual),
            ],
            "first",
        )
        .await
        .unwrap();
        let reverse = [
            (b.clone(), WriteIntent::Contextual),
            (a.clone(), WriteIntent::Contextual),
        ];
        let mut next = Box::pin(begin_writes(None, &reverse, "second"));
        assert!(futures::poll!(next.as_mut()).is_pending());
        guards.pop().unwrap().commit(WriteKind::Modified, Some("b"));
        assert!(futures::poll!(next.as_mut()).is_pending());
        drop(guards);
        assert_eq!(next.await.unwrap().len(), 2);
    }

    #[test]
    fn subprocess_file_writer() {
        let Ok(root) = std::env::var("REFACT_TEST_FILE_RACE_ROOT") else {
            return;
        };
        let id = std::env::var("REFACT_TEST_FILE_RACE_ID").unwrap();
        let root = PathBuf::from(root);
        let path = root.join("shared");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(install_race_provenance(&id));
        record_read(Some(&id), &path, "before", 1, 1, 1, None);
        std::fs::write(root.join(format!("ready-{id}")), "ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !root.join("go").exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let result = runtime.block_on(async {
            let mut guard =
                begin_write(Some(&id), &path, WriteIntent::Contextual, "subprocess-test").await?;
            tokio::fs::write(&path, &id)
                .await
                .map_err(|error| error.to_string())?;
            guard.record_bytes(WriteKind::Modified, Some(id.as_bytes()));
            crate::coordination::file_provenance::flush().await;
            drop(guard);
            Ok::<_, String>(())
        });
        std::fs::write(
            root.join(format!("result-{id}")),
            match result {
                Ok(()) => "won".to_string(),
                Err(error) => format!("stale:{error}"),
            },
        )
        .unwrap();
    }

    /// A subprocess that never reaches the barrier has to say why. Without its
    /// exit status and its own output a crashed writer is indistinguishable
    /// from a slow one, and the only symptom left is the barrier timeout.
    fn subprocess_report(root: &Path, child: &mut std::process::Child, id: &str) -> String {
        let status = match child.try_wait() {
            Ok(Some(status)) => format!("exited with {status}"),
            Ok(None) => "is still running".to_string(),
            Err(error) => format!("has no reportable status: {error}"),
        };
        let log = std::fs::read_to_string(root.join(format!("log-{id}")))
            .unwrap_or_else(|error| format!("<log unreadable: {error}>"));
        format!("subprocess {id} {status}; its output was:\n{log}")
    }

    async fn install_race_provenance(id: &str) {
        let Ok(store_path) = std::env::var("REFACT_TEST_FILE_RACE_STORE") else {
            return;
        };
        let store = std::sync::Arc::new(
            refact_work_core::CoordinationStore::open(
                Path::new(&store_path),
                &refact_work_core::CoordinationSettings::default(),
            )
            .await
            .unwrap(),
        );
        crate::coordination::file_provenance::install_sink(
            store,
            refact_work_core::ProjectKey::new("race"),
            format!("engine-{id}"),
            None,
        );
    }

    #[serial_test::serial(file_provenance)]
    #[test]
    fn cooperating_processes_record_both_writes() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("coordination.sqlite");
        std::fs::write(dir.path().join("shared"), "before").unwrap();
        let spawn = |id: &str| {
            let log = File::create(dir.path().join(format!("log-{id}"))).unwrap();
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "file_activity::tests::subprocess_file_writer",
                    "--nocapture",
                ])
                .env("REFACT_TEST_FILE_RACE_ROOT", dir.path())
                .env("REFACT_TEST_FILE_RACE_ID", id)
                .env("REFACT_TEST_FILE_RACE_STORE", &store_path)
                .stdout(std::process::Stdio::from(log.try_clone().unwrap()))
                .stderr(std::process::Stdio::from(log))
                .spawn()
                .unwrap()
        };
        let mut a = spawn("a");
        let mut b = spawn("b");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !dir.path().join("ready-a").exists() || !dir.path().join("ready-b").exists() {
            if Instant::now() >= deadline {
                let report = format!(
                    "{}\n{}",
                    subprocess_report(dir.path(), &mut a, "a"),
                    subprocess_report(dir.path(), &mut b, "b")
                );
                let _ = a.kill();
                let _ = b.kill();
                let _ = a.wait();
                let _ = b.wait();
                panic!("cooperating subprocesses did not reach barrier\n{report}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::write(dir.path().join("go"), "go").unwrap();
        assert!(
            a.wait().unwrap().success(),
            "{}",
            subprocess_report(dir.path(), &mut a, "a")
        );
        assert!(
            b.wait().unwrap().success(),
            "{}",
            subprocess_report(dir.path(), &mut b, "b")
        );
        let a = std::fs::read_to_string(dir.path().join("result-a")).unwrap();
        let b = std::fs::read_to_string(dir.path().join("result-b")).unwrap();
        assert_eq!(a, "won");
        assert_eq!(b, "won");
        let winner = std::fs::read_to_string(dir.path().join("shared")).unwrap();
        assert!(matches!(winner.as_str(), "a" | "b"));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let key = resolve_key(&dir.path().join("shared")).0;
        let writes = runtime.block_on(async {
            let store = Arc::new(
                refact_work_core::CoordinationStore::open(
                    &store_path,
                    &refact_work_core::CoordinationSettings::default(),
                )
                .await
                .unwrap(),
            );
            let access = store.authorize(refact_work_core::ProjectKey::new("race"));
            store.list_activity_observations(&access).await.unwrap()
        });
        let writes: Vec<_> = writes
            .into_iter()
            .filter(|record| {
                record.file_key == key && record.access == refact_work_core::AccessKind::Write
            })
            .collect();
        assert_eq!(writes.len(), 2);
        for actor in ["a", "b"] {
            let record = writes
                .iter()
                .find(|record| record.work.id == actor)
                .unwrap_or_else(|| panic!("missing write from {actor}: {writes:?}"));
            assert_eq!(record.tool_call.as_deref(), Some("subprocess-test"));
            assert_eq!(
                record.content_hash.as_deref(),
                Some(digest_of(actor).as_str())
            );
        }
    }
}
