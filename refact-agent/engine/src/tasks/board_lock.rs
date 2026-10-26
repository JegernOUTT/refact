use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::{Arc, OnceLock};
#[cfg(test)]
use std::sync::RwLock as SyncRwLock;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex as AMutex, OwnedMutexGuard};

pub const BOARD_LOCK_FILE: &str = "board.lock";
pub const PLANNER_IDS_LOCK_FILE: &str = "planner_ids.lock";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardLockSettings {
    pub wait_timeout_ms: u64,
    pub poll_interval_ms: u64,
}

impl Default for BoardLockSettings {
    fn default() -> Self {
        Self::from_coordination_settings()
    }
}

impl BoardLockSettings {
    pub fn from_coordination_settings() -> Self {
        let coordination = refact_work_core::coordination_settings();
        Self {
            wait_timeout_ms: coordination.lock_timeout_secs.saturating_mul(1_000),
            poll_interval_ms: coordination.lock_poll_interval_ms,
        }
        .sanitized()
    }

    fn sanitized(self) -> Self {
        Self {
            wait_timeout_ms: self.wait_timeout_ms.clamp(1, 600_000),
            poll_interval_ms: self.poll_interval_ms.clamp(1, 5_000),
        }
    }
}

#[cfg(test)]
static SETTINGS_OVERRIDES: OnceLock<SyncRwLock<HashMap<PathBuf, BoardLockSettings>>> =
    OnceLock::new();

#[cfg(test)]
fn settings_override_cell() -> &'static SyncRwLock<HashMap<PathBuf, BoardLockSettings>> {
    SETTINGS_OVERRIDES.get_or_init(|| SyncRwLock::new(HashMap::new()))
}

#[cfg(test)]
#[must_use]
struct BoardLockSettingsOverride {
    dir: PathBuf,
    previous: Option<BoardLockSettings>,
}

#[cfg(test)]
impl Drop for BoardLockSettingsOverride {
    fn drop(&mut self) {
        let mut overrides = settings_override_cell()
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match self.previous {
            Some(settings) => {
                overrides.insert(self.dir.clone(), settings);
            }
            None => {
                overrides.remove(&self.dir);
            }
        }
    }
}

#[cfg(test)]
fn set_board_lock_settings_override(
    dir: &Path,
    settings: BoardLockSettings,
) -> BoardLockSettingsOverride {
    let dir = dunce::canonicalize(dir).unwrap();
    let previous = settings_override_cell()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(dir.clone(), settings.sanitized());
    BoardLockSettingsOverride { dir, previous }
}

#[cfg(test)]
fn board_lock_settings_for(dir: &Path) -> BoardLockSettings {
    let canonical = dunce::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    settings_override_cell()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&canonical)
        .copied()
        .unwrap_or_else(board_lock_settings)
}

pub fn board_lock_settings() -> BoardLockSettings {
    BoardLockSettings::from_coordination_settings()
}
static PROCESS_LOCKS: OnceLock<AMutex<HashMap<String, Arc<AMutex<()>>>>> = OnceLock::new();

fn process_locks() -> &'static AMutex<HashMap<String, Arc<AMutex<()>>>> {
    PROCESS_LOCKS.get_or_init(|| AMutex::new(HashMap::new()))
}

async fn process_lock_for(key: &str) -> Arc<AMutex<()>> {
    let mut locks = process_locks().lock().await;
    locks
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(AMutex::new(())))
        .clone()
}

#[derive(Debug)]
pub struct BoardLockGuard {
    key: String,
    dir: PathBuf,
    lock_file: String,
    _process_guard: OwnedMutexGuard<()>,
    _release: Sender<()>,
}

impl BoardLockGuard {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn guards(&self, lock_file: &str) -> bool {
        self.lock_file == lock_file
    }
}

fn open_lock_file(path: &Path) -> io::Result<fd_lock::RwLock<File>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    Ok(fd_lock::RwLock::new(file))
}

pub async fn canonical_dir(dir: &Path) -> Result<PathBuf, String> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        dunce::canonicalize(&dir)
            .or_else(|_| std::fs::canonicalize(&dir))
            .unwrap_or(dir)
    })
    .await
    .map_err(|e| format!("Failed to canonicalize task dir: {}", e))
}

fn lock_key(canonical_dir: &Path, lock_file: &str) -> String {
    format!("{}|{}", canonical_dir.to_string_lossy(), lock_file)
}

fn lock_path(canonical_dir: &Path, lock_file: &str) -> Result<PathBuf, String> {
    let parent = canonical_dir
        .parent()
        .ok_or("task directory has no parent")?;
    let key = crate::file_activity::digest_of(&canonical_dir.to_string_lossy());
    Ok(parent.join(".locks").join(format!("{key}.{lock_file}")))
}

async fn acquire_os_lock(
    path: PathBuf,
    deadline: Instant,
    poll: Duration,
) -> Result<Sender<()>, String> {
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (acquired_tx, acquired_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let blocking_path = path.clone();

    tokio::task::spawn_blocking(move || {
        let mut acquired_tx = Some(acquired_tx);
        let mut lock = match open_lock_file(&blocking_path) {
            Ok(lock) => lock,
            Err(e) => {
                if let Some(tx) = acquired_tx.take() {
                    let _ = tx.send(Err(format!(
                        "Failed to open lock file {}: {}",
                        blocking_path.display(),
                        e
                    )));
                }
                return;
            }
        };
        loop {
            match lock.try_write() {
                Ok(guard) => {
                    if let Some(tx) = acquired_tx.take() {
                        if tx.send(Ok(())).is_err() {
                            drop(guard);
                            return;
                        }
                    }
                    let _ = release_rx.recv();
                    drop(guard);
                    return;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    if let Some(tx) = acquired_tx.take() {
                        let _ = tx.send(Err(format!(
                            "Failed to lock {}: {}",
                            blocking_path.display(),
                            e
                        )));
                    }
                    return;
                }
            }
            if matches!(release_rx.try_recv(), Err(TryRecvError::Disconnected)) {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                if let Some(tx) = acquired_tx.take() {
                    let _ = tx.send(Err(format!(
                        "Timed out waiting for lock {}",
                        blocking_path.display()
                    )));
                }
                return;
            }
            std::thread::sleep(poll.min(deadline.saturating_duration_since(now)));
        }
    });

    match acquired_rx.await {
        Ok(Ok(())) => Ok(release_tx),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(format!(
            "Lock worker for {} stopped unexpectedly",
            path.display()
        )),
    }
}

pub async fn acquire(dir: &Path, lock_file: &str) -> Result<BoardLockGuard, String> {
    #[cfg(not(test))]
    let settings = board_lock_settings();
    #[cfg(test)]
    let settings = board_lock_settings_for(dir);
    let deadline = Instant::now() + Duration::from_millis(settings.wait_timeout_ms);
    let poll = Duration::from_millis(settings.poll_interval_ms);

    let canonical = canonical_dir(dir).await?;
    let key = lock_key(&canonical, lock_file);

    let process_lock = process_lock_for(&key).await;
    let process_guard = tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        process_lock.lock_owned(),
    )
    .await
    .map_err(|_| format!("Timed out waiting for in-process lock {}", key))?;

    let release = acquire_os_lock(lock_path(&canonical, lock_file)?, deadline, poll).await?;
    if !canonical.is_dir() {
        return Err("Task directory no longer exists".into());
    }

    Ok(BoardLockGuard {
        key,
        dir: canonical,
        lock_file: lock_file.to_string(),
        _process_guard: process_guard,
        _release: release,
    })
}

pub async fn acquire_board(dir: &Path) -> Result<BoardLockGuard, String> {
    acquire(dir, BOARD_LOCK_FILE).await
}

pub async fn acquire_planner_ids(dir: &Path) -> Result<BoardLockGuard, String> {
    acquire(dir, PLANNER_IDS_LOCK_FILE).await
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ForeignHolder {
        _thread: std::thread::JoinHandle<()>,
        release: Sender<()>,
        acquired: std::sync::mpsc::Receiver<()>,
    }

    fn hold_from_foreign_handle(path: PathBuf) -> ForeignHolder {
        let (release, release_rx) = mpsc::channel::<()>();
        let (acquired_tx, acquired) = mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            let mut lock = open_lock_file(&path).unwrap();
            let guard = lock.try_write().unwrap();
            acquired_tx.send(()).unwrap();
            let _ = release_rx.recv();
            drop(guard);
        });
        acquired.recv().unwrap();
        ForeignHolder {
            _thread: thread,
            release,
            acquired,
        }
    }

    fn short_settings(dir: &Path) -> BoardLockSettingsOverride {
        set_board_lock_settings_override(
            dir,
            BoardLockSettings {
                wait_timeout_ms: 300,
                poll_interval_ms: 5,
            },
        )
    }

    #[tokio::test]
    async fn foreign_holder_blocks_acquisition_until_released() {
        let dir = tempfile::tempdir().unwrap();
        let _settings = short_settings(dir.path());
        let canonical = canonical_dir(dir.path()).await.unwrap();
        let holder = hold_from_foreign_handle(lock_path(&canonical, BOARD_LOCK_FILE).unwrap());

        let denied = acquire_board(dir.path()).await;
        assert!(denied.is_err(), "lock must not be granted while held");
        assert!(denied.unwrap_err().contains("Timed out"));

        holder.release.send(()).unwrap();
        drop(holder.acquired);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let granted = acquire_board(dir.path()).await;
        assert!(granted.is_ok(), "lock must be granted after release");
    }

    #[tokio::test]
    async fn same_process_waiters_are_serialized() {
        let dir = tempfile::tempdir().unwrap();
        let _settings = set_board_lock_settings_override(
            dir.path(),
            BoardLockSettings {
                wait_timeout_ms: 5_000,
                poll_interval_ms: 5,
            },
        );
        let path = dir.path().to_path_buf();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let mut handles = vec![];
        for _ in 0..8 {
            let path = path.clone();
            let counter = counter.clone();
            let peak = peak.clone();
            handles.push(tokio::spawn(async move {
                let guard = acquire_board(&path).await.unwrap();
                let live = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                peak.fetch_max(live, std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                counter.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                drop(guard);
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }
        assert_eq!(peak.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_alias_maps_to_the_same_lock() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let _settings = short_settings(&alias);
        assert_eq!(board_lock_settings_for(&real).wait_timeout_ms, 300);

        let held = acquire_board(&real).await.unwrap();
        let denied = acquire_board(&alias).await;

        assert!(
            denied.is_err(),
            "aliased path must resolve to the same canonical lock"
        );
        drop(held);
        assert!(acquire_board(&alias).await.is_ok());
    }

    #[tokio::test]
    async fn distinct_lock_files_do_not_block_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let _settings = short_settings(dir.path());
        let board = acquire_board(dir.path()).await.unwrap();
        let planner = acquire_planner_ids(dir.path()).await;
        assert!(planner.is_ok());
        assert!(board.guards(BOARD_LOCK_FILE));
        assert!(!board.guards(PLANNER_IDS_LOCK_FILE));
    }

    #[tokio::test]
    async fn deleting_a_task_never_deletes_or_recreates_its_lock_domain() {
        let root = tempfile::tempdir().unwrap();
        let task = root.path().join("task");
        std::fs::create_dir(&task).unwrap();
        let guard = acquire_board(&task).await.unwrap();
        let path = lock_path(guard.dir(), BOARD_LOCK_FILE).unwrap();
        std::fs::remove_dir(&task).unwrap();
        assert!(path.is_file());
        drop(guard);
        assert!(acquire_board(&task).await.is_err());
        assert!(!task.exists());
        std::fs::create_dir(&task).unwrap();
        let replacement = acquire_board(&task).await.unwrap();
        assert_eq!(path, lock_path(replacement.dir(), BOARD_LOCK_FILE).unwrap());
    }
}
