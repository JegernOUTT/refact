use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

pub const DEFAULT_FILE_INDEX_CAP_BYTES: u64 = 64 * 1024 * 1024;
const MERGE_BATCH: usize = 2048;
const ENTRY_OVERHEAD_BYTES: u64 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct FileFacts {
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
    pub size: u64,
    pub mtime_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileIndexState {
    Cold,
    Building,
    Ready,
    Degraded,
}

impl FileIndexState {
    fn as_u8(self) -> u8 {
        match self {
            Self::Cold => 0,
            Self::Building => 1,
            Self::Ready => 2,
            Self::Degraded => 3,
        }
    }
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Building,
            2 => Self::Ready,
            3 => Self::Degraded,
            _ => Self::Cold,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FileIndexStatus {
    pub state: FileIndexState,
    pub entries: usize,
    pub hard_linked_identities: usize,
    pub bytes: u64,
    pub cap_bytes: u64,
    pub scanned: usize,
    pub skipped_over_cap: usize,
    pub complete: bool,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
}

pub struct FileIndex {
    entries: StdRwLock<HashMap<PathBuf, Option<FileFacts>>>,
    identities: StdRwLock<HashMap<(u64, u64), Vec<PathBuf>>>,
    bytes: AtomicU64,
    cap_bytes: AtomicU64,
    state: AtomicU8,
    scanned: AtomicUsize,
    skipped_over_cap: AtomicUsize,
    started_at_ms: AtomicU64,
    finished_at_ms: AtomicU64,
    build_running: AtomicBool,
    build_requests: std::sync::Mutex<Option<Vec<PathBuf>>>,
    generation: AtomicU64,
    built_roots: StdRwLock<Vec<PathBuf>>,
    pending_watcher_drops: AtomicU64,
    processed_watcher_drops: AtomicU64,
}

impl Default for FileIndex {
    fn default() -> Self {
        Self::new(DEFAULT_FILE_INDEX_CAP_BYTES)
    }
}

impl FileIndex {
    pub fn new(cap_bytes: u64) -> Self {
        Self {
            entries: StdRwLock::new(HashMap::new()),
            identities: StdRwLock::new(HashMap::new()),
            bytes: AtomicU64::new(0),
            cap_bytes: AtomicU64::new(cap_bytes),
            state: AtomicU8::new(FileIndexState::Cold.as_u8()),
            scanned: AtomicUsize::new(0),
            skipped_over_cap: AtomicUsize::new(0),
            started_at_ms: AtomicU64::new(0),
            finished_at_ms: AtomicU64::new(0),
            build_running: AtomicBool::new(false),
            build_requests: std::sync::Mutex::new(None),
            generation: AtomicU64::new(0),
            built_roots: StdRwLock::new(Vec::new()),
            pending_watcher_drops: AtomicU64::new(0),
            processed_watcher_drops: AtomicU64::new(0),
        }
    }

    pub fn state(&self) -> FileIndexState {
        FileIndexState::from_u8(self.state.load(Ordering::Relaxed))
    }

    fn set_state(&self, state: FileIndexState) {
        self.state.store(state.as_u8(), Ordering::Relaxed);
    }

    pub fn get(&self, path: &Path) -> Option<FileFacts> {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(path)
            .copied()
            .flatten()
    }

    pub fn lookup_or_stat(&self, path: &Path) -> Option<FileFacts> {
        if let Some(facts) = self.get(path) {
            return Some(facts);
        }
        let facts = facts_for(path)?;
        self.upsert(path, facts);
        Some(facts)
    }

    pub fn aliases_of(&self, facts: &FileFacts, exclude: &Path) -> Vec<PathBuf> {
        // Synchronize identity reads with atomic generation publication.
        let _entries = self.entries.read().unwrap_or_else(|p| p.into_inner());
        if facts.nlink <= 1 {
            return Vec::new();
        }
        self.identities
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&(facts.dev, facts.ino))
            .map(|paths| {
                paths
                    .iter()
                    .filter(|candidate| candidate.as_path() != exclude)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn upsert(&self, path: &Path, facts: FileFacts) -> bool {
        self.upsert_entry(path, Some(facts))
    }

    fn upsert_entry(&self, path: &Path, facts: Option<FileFacts>) -> bool {
        let cost = entry_cost(path);
        let mut guard = self.entries.write().unwrap_or_else(|p| p.into_inner());
        if guard.get(path) == Some(&facts) {
            return true;
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        let previous = if let Some(slot) = guard.get_mut(path) {
            Some(std::mem::replace(slot, facts))
        } else if self.bytes.load(Ordering::Relaxed) + cost > self.cap_bytes.load(Ordering::Relaxed)
        {
            self.skipped_over_cap.fetch_add(1, Ordering::Relaxed);
            self.set_state(FileIndexState::Degraded);
            return false;
        } else {
            guard.insert(path.to_path_buf(), facts);
            self.bytes.fetch_add(cost, Ordering::Relaxed);
            None
        };
        if let Some(Some(previous)) = previous {
            self.identity_forget(path, &previous);
        }
        if let Some(facts) = facts {
            self.identity_record(path, &facts);
        }
        true
    }

    pub fn remove(&self, path: &Path) {
        let mut guard = self.entries.write().unwrap_or_else(|p| p.into_inner());
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(facts) = guard.remove(path) {
            let cost = entry_cost(path).min(self.bytes.load(Ordering::Relaxed));
            self.bytes.fetch_sub(cost, Ordering::Relaxed);
            if let Some(facts) = facts {
                self.identity_forget(path, &facts);
            }
        }
    }

    /// Metadata inventory only: None denotes a real directory, never a symlink.
    pub fn snapshot(&self) -> Vec<(PathBuf, Option<FileFacts>)> {
        self.entries
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(path, facts)| (path.clone(), *facts))
            .collect()
    }

    pub fn remove_subtree(&self, path: &Path) {
        for (known, _) in self.snapshot() {
            if known.starts_with(path) {
                self.remove(&known);
            }
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn covers_roots(&self, roots: &[PathBuf]) -> bool {
        let built = self.built_roots.read().unwrap_or_else(|p| p.into_inner());
        roots.iter().all(|root| built.contains(root))
    }

    pub fn mark_degraded(&self) {
        let _entries = self.entries.write().unwrap_or_else(|p| p.into_inner());
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.set_state(FileIndexState::Degraded);
    }

    pub fn observe_watcher_drops(&self, dropped: u64) -> bool {
        let _entries = self.entries.write().unwrap_or_else(|p| p.into_inner());
        if dropped <= self.pending_watcher_drops.load(Ordering::Relaxed) {
            return false;
        }
        self.pending_watcher_drops.store(dropped, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.set_state(FileIndexState::Degraded);
        true
    }

    pub fn refresh(&self, path: &Path) {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => {
                self.upsert(path, facts_from_metadata(&metadata));
            }
            Ok(metadata) if metadata.is_dir() => {
                self.upsert_entry(path, None);
            }
            _ => {
                let directory = self
                    .entries
                    .read()
                    .unwrap_or_else(|p| p.into_inner())
                    .get(path)
                    .is_some_and(|facts| facts.is_none());
                if directory {
                    self.remove_subtree(path);
                } else {
                    self.remove(path);
                }
            }
        }
    }

    fn identity_record(&self, path: &Path, facts: &FileFacts) {
        if facts.nlink <= 1 {
            return;
        }
        let mut guard = self
            .identities
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let slot = guard.entry((facts.dev, facts.ino)).or_default();
        if !slot.iter().any(|known| known.as_path() == path) {
            slot.push(path.to_path_buf());
        }
    }

    fn identity_forget(&self, path: &Path, facts: &FileFacts) {
        let mut guard = self
            .identities
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let key = (facts.dev, facts.ino);
        if let Some(slot) = guard.get_mut(&key) {
            slot.retain(|known| known.as_path() != path);
            if slot.is_empty() {
                guard.remove(&key);
            }
        }
    }

    pub fn merge_batch(&self, batch: Vec<(PathBuf, FileFacts)>) {
        for (path, facts) in batch {
            self.upsert(&path, facts);
        }
    }

    pub fn len(&self) -> usize {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .filter(|facts| facts.is_some())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn begin_build(&self) {
        let _entries = self.entries.write().unwrap_or_else(|p| p.into_inner());
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.set_state(FileIndexState::Building);
        self.skipped_over_cap.store(0, Ordering::Relaxed);
        self.scanned.store(0, Ordering::Relaxed);
        self.started_at_ms.store(now_ms(), Ordering::Relaxed);
        self.finished_at_ms.store(0, Ordering::Relaxed);
    }

    pub fn finish_build(&self) {
        if self.skipped_over_cap.load(Ordering::Relaxed) > 0 {
            self.set_state(FileIndexState::Degraded);
        } else {
            self.set_state(FileIndexState::Ready);
        }
        self.finished_at_ms.store(now_ms(), Ordering::Relaxed);
    }

    pub fn status(&self) -> FileIndexStatus {
        let skipped = self.skipped_over_cap.load(Ordering::Relaxed);
        let state = self.state();
        FileIndexStatus {
            state,
            entries: self.len(),
            hard_linked_identities: self
                .identities
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            bytes: self.bytes.load(Ordering::Relaxed),
            cap_bytes: self.cap_bytes.load(Ordering::Relaxed),
            scanned: self.scanned.load(Ordering::Relaxed),
            skipped_over_cap: skipped,
            complete: skipped == 0 && state == FileIndexState::Ready,
            started_at_ms: self.started_at_ms.load(Ordering::Relaxed),
            finished_at_ms: self.finished_at_ms.load(Ordering::Relaxed),
        }
    }
}

fn entry_cost(path: &Path) -> u64 {
    path.as_os_str().len() as u64 + std::mem::size_of::<FileFacts>() as u64 + ENTRY_OVERHEAD_BYTES
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

pub fn facts_for(path: &Path) -> Option<FileFacts> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    Some(facts_from_metadata(&metadata))
}

#[cfg(unix)]
fn facts_from_metadata(metadata: &std::fs::Metadata) -> FileFacts {
    use std::os::unix::fs::MetadataExt;
    FileFacts {
        dev: metadata.dev(),
        ino: metadata.ino(),
        nlink: metadata.nlink(),
        size: metadata.size(),
        mtime_ms: metadata.mtime() * 1000 + i64::from(metadata.mtime_nsec()) / 1_000_000,
    }
}

#[cfg(not(unix))]
fn facts_from_metadata(metadata: &std::fs::Metadata) -> FileFacts {
    let mtime_ms = metadata
        .modified()
        .ok()
        .and_then(|stamp| stamp.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0);
    FileFacts {
        dev: 0,
        ino: 0,
        nlink: 1,
        size: metadata.len(),
        mtime_ms,
    }
}

pub fn build_blocking(index: &FileIndex, roots: &[PathBuf], shutdown: &AtomicBool) {
    build_with_observer(index, roots, shutdown, || {});
}

fn build_with_observer(
    index: &FileIndex,
    roots: &[PathBuf],
    shutdown: &AtomicBool,
    started: impl FnOnce(),
) {
    build_observed(index, roots, shutdown, started, || {});
}

fn build_observed(
    index: &FileIndex,
    roots: &[PathBuf],
    shutdown: &AtomicBool,
    started: impl FnOnce(),
    before_publish: impl FnOnce(),
) {
    let mut started = Some(started);
    let mut before_publish = Some(before_publish);
    build_retrying(
        || build_attempt(index, roots, shutdown, &mut started, &mut before_publish),
        shutdown,
    );
}

fn build_retrying(mut attempt: impl FnMut() -> bool, shutdown: &AtomicBool) {
    for _ in 0..3 {
        if !attempt() || shutdown.load(Ordering::Relaxed) {
            return;
        }
        for _ in 0..5 {
            if shutdown.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

fn build_attempt(
    index: &FileIndex,
    roots: &[PathBuf],
    shutdown: &AtomicBool,
    started: &mut Option<impl FnOnce()>,
    before_publish: &mut Option<impl FnOnce()>,
) -> bool {
    use rayon::prelude::*;

    fn flush(index: &FileIndex, pending: &mut Vec<PathBuf>) {
        if pending.is_empty() {
            return;
        }
        let expected = pending.len();
        let batch: Vec<(PathBuf, FileFacts)> = std::mem::take(pending)
            .into_par_iter()
            .filter_map(|path| facts_for(&path).map(|facts| (path, facts)))
            .collect();
        if batch.len() != expected {
            index.mark_degraded();
        }
        index.scanned.fetch_add(batch.len(), Ordering::Relaxed);
        index.merge_batch(batch);
    }

    index.begin_build();
    let generation = index.generation();
    let watcher_drops = index.pending_watcher_drops.load(Ordering::Relaxed);
    // Never clear the live privacy/hardlink inventory while rebuilding. Scan into a
    // private bounded generation and publish only after a stable complete traversal.
    if let Some(started) = started.take() {
        started();
    }
    let staged = FileIndex::new(index.cap_bytes.load(Ordering::Relaxed));
    let canonical_roots: Vec<_> = roots
        .iter()
        .filter_map(|root| {
            std::fs::canonicalize(root)
                .ok()
                .map(|root| dunce::simplified(&root).to_path_buf())
        })
        .collect();
    let previous_roots = index
        .built_roots
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    for (path, facts) in index.snapshot() {
        if !previous_roots.iter().any(|root| path.starts_with(root))
            && !canonical_roots.iter().any(|root| path.starts_with(root))
        {
            staged.upsert_entry(&path, facts);
        }
    }
    let mut uncertain = canonical_roots.len() != roots.len();
    let mut pending: Vec<PathBuf> = Vec::with_capacity(MERGE_BATCH);
    for root in &canonical_roots {
        for entry in walkdir::WalkDir::new(root).follow_links(false).into_iter() {
            if shutdown.load(Ordering::Relaxed) {
                index.mark_degraded();
                return false;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    uncertain = true;
                    continue;
                }
            };
            if entry.file_type().is_dir() {
                // Only Explorer-visible directories need inventory slots; regular
                // files remain unfiltered for privacy hardlink identity discovery.
                if entry.path().strip_prefix(root).is_ok_and(|relative| {
                    relative.components().all(|component| {
                        let name = component.as_os_str().to_string_lossy();
                        !name.starts_with('.')
                            && !matches!(name.as_ref(), "target" | "node_modules")
                    })
                }) {
                    staged.upsert_entry(entry.path(), None);
                }
            }
            if !entry.file_type().is_file() {
                continue;
            }
            pending.push(entry.into_path());
            if pending.len() >= MERGE_BATCH {
                flush(&staged, &mut pending);
            }
        }
    }
    flush(&staged, &mut pending);
    if let Some(before_publish) = before_publish.take() {
        before_publish();
    }
    uncertain |= staged.state() == FileIndexState::Degraded;
    if uncertain {
        let mut entries = index.entries.write().unwrap_or_else(|p| p.into_inner());
        if generation != index.generation() {
            index.set_state(FileIndexState::Degraded);
            return !shutdown.load(Ordering::Relaxed);
        }
        for (path, facts) in staged.snapshot() {
            if entries.contains_key(&path) {
                continue;
            }
            let cost = entry_cost(&path);
            if index.bytes.load(Ordering::Relaxed) + cost > index.cap_bytes.load(Ordering::Relaxed)
            {
                index.skipped_over_cap.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            entries.insert(path.clone(), facts);
            index.bytes.fetch_add(cost, Ordering::Relaxed);
            if let Some(facts) = facts {
                index.identity_record(&path, &facts);
            }
        }
        index.generation.fetch_add(1, Ordering::SeqCst);
        index.set_state(FileIndexState::Degraded);
        return false;
    }
    if generation != index.generation() {
        // Keeping the previous identities is conservative for privacy and avoids
        // replacing them with an incomplete map after errors or concurrent changes.
        index.mark_degraded();
        return !shutdown.load(Ordering::Relaxed);
    }
    let mut entries = index.entries.write().unwrap_or_else(|p| p.into_inner());
    let mut identities = index.identities.write().unwrap_or_else(|p| p.into_inner());
    if generation != index.generation() {
        index.set_state(FileIndexState::Degraded);
        return !shutdown.load(Ordering::Relaxed);
    }
    *entries = staged
        .entries
        .into_inner()
        .unwrap_or_else(|p| p.into_inner());
    *identities = staged
        .identities
        .into_inner()
        .unwrap_or_else(|p| p.into_inner());
    index
        .bytes
        .store(staged.bytes.load(Ordering::Relaxed), Ordering::Relaxed);
    index
        .scanned
        .store(staged.scanned.load(Ordering::Relaxed), Ordering::Relaxed);
    *index.built_roots.write().unwrap_or_else(|p| p.into_inner()) = canonical_roots;
    index
        .processed_watcher_drops
        .store(watcher_drops, Ordering::Relaxed);
    index.finish_build();
    false
}

pub fn spawn_build(
    index: Arc<FileIndex>,
    roots: Vec<PathBuf>,
    shutdown: Arc<AtomicBool>,
) -> Option<tokio::task::JoinHandle<()>> {
    if roots.is_empty() {
        return None;
    }
    let mut requests = index
        .build_requests
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    *requests = Some(roots);
    if index.build_running.swap(true, Ordering::SeqCst) {
        return None;
    }
    drop(requests);
    Some(tokio::task::spawn_blocking(move || loop {
        let roots = {
            let mut requests = index
                .build_requests
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            match requests.take() {
                Some(roots) if !shutdown.load(Ordering::Relaxed) => roots,
                _ => {
                    index.build_running.store(false, Ordering::SeqCst);
                    return;
                }
            }
        };
        build_blocking(&index, &roots, &shutdown);
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_is_reported_instead_of_silently_dropping_and_lookup_still_works() {
        let temp = tempfile::tempdir().unwrap();
        let index = FileIndex::new(0);
        let file = temp.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let facts = facts_for(&file).unwrap();

        assert!(!index.upsert(&file, facts));

        let status = index.status();
        assert_eq!(status.state, FileIndexState::Degraded);
        assert_eq!(status.skipped_over_cap, 1);
        assert!(!status.complete);
        assert!(index.lookup_or_stat(&file).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_aliases_are_tracked_incrementally_without_any_walk() {
        let temp = tempfile::tempdir().unwrap();
        let secret = temp.path().join(".env");
        let alias = temp.path().join("innocent.txt");
        let plain = temp.path().join("plain.txt");
        std::fs::write(&secret, "s").unwrap();
        std::fs::hard_link(&secret, &alias).unwrap();
        std::fs::write(&plain, "p").unwrap();
        let index = FileIndex::default();
        index.refresh(&secret);
        index.refresh(&alias);
        index.refresh(&plain);

        let alias_facts = index.get(&alias).unwrap();
        let found = index.aliases_of(&alias_facts, &alias);
        assert_eq!(found, vec![secret.clone()]);

        let plain_facts = index.get(&plain).unwrap();
        assert!(index.aliases_of(&plain_facts, &plain).is_empty());

        std::fs::remove_file(&secret).unwrap();
        index.refresh(&secret);
        assert!(index.aliases_of(&alias_facts, &alias).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn rebuild_keeps_live_hardlink_aliases_until_atomic_publication() {
        let temp = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let secret = external.path().join(".env");
        let alias = temp.path().join("plain.txt");
        std::fs::write(&secret, "private").unwrap();
        std::fs::hard_link(&secret, &alias).unwrap();
        let index = FileIndex::default();
        index.refresh(&secret);
        index.refresh(&alias);
        let facts = index.get(&alias).unwrap();
        assert_eq!(index.aliases_of(&facts, &alias), vec![secret.clone()]);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let index_ref = &index;
            scope.spawn(move || {
                build_with_observer(
                    index_ref,
                    &[temp.path().into()],
                    &AtomicBool::new(false),
                    || {
                        started_tx.send(()).unwrap();
                        release_rx
                            .recv_timeout(std::time::Duration::from_secs(10))
                            .unwrap();
                    },
                )
            });
            started_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            let during = index.aliases_of(&facts, &alias);
            release_tx.send(()).unwrap();
            assert_eq!(during, vec![secret.clone()]);
        });
        assert_eq!(index.aliases_of(&facts, &alias), vec![secret]);
        assert!(index.status().complete);
    }

    #[test]
    fn directory_inventory_rebuild_prunes_deleted_subtrees_and_keeps_empty_folders() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("no_extension"), "x").unwrap();
        let index = FileIndex::default();
        let shutdown = AtomicBool::new(false);
        build_blocking(&index, &[temp.path().into()], &shutdown);
        assert!(index
            .snapshot()
            .iter()
            .any(|(path, facts)| path == &old && facts.is_none()));
        std::fs::rename(&old, temp.path().join("new")).unwrap();
        index.refresh(&old);
        assert!(!index
            .snapshot()
            .iter()
            .any(|(path, _)| path.starts_with(&old)));
        std::fs::create_dir(temp.path().join("empty")).unwrap();
        build_blocking(&index, &[temp.path().into()], &shutdown);
        let snapshot = index.snapshot();
        assert!(snapshot
            .iter()
            .any(|(path, facts)| path == &temp.path().join("empty") && facts.is_none()));
        assert!(index.get(&temp.path().join("new/no_extension")).is_some());
        assert!(index.status().complete);
    }

    #[test]
    fn cold_build_recovers_from_modify_during_staging() {
        let temp = tempfile::tempdir().unwrap();
        let changed = temp.path().join("changed.txt");
        let untouched = temp.path().join("untouched.txt");
        std::fs::write(&changed, "old").unwrap();
        std::fs::write(&untouched, "retained").unwrap();
        let index = FileIndex::default();
        build_with_observer(
            &index,
            &[temp.path().into()],
            &AtomicBool::new(false),
            || {
                std::fs::write(&changed, "changed bytes").unwrap();
                index.refresh(&changed);
            },
        );
        assert!(
            index.status().complete,
            "ordinary Modify must not strand a cold inventory"
        );
        assert_eq!(index.get(&untouched).unwrap().size, 8);
        assert_eq!(index.get(&changed).unwrap().size, 13);
    }

    #[cfg(unix)]
    #[test]
    fn racing_external_lookup_alias_survives_staged_publication() {
        let temp = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let alias = temp.path().join("plain.txt");
        let secret = external.path().join(".env");
        std::fs::write(&secret, "private").unwrap();
        std::fs::hard_link(&secret, &alias).unwrap();
        let index = FileIndex::default();
        build_observed(
            &index,
            &[temp.path().into()],
            &AtomicBool::new(false),
            || {},
            || {
                assert!(index.lookup_or_stat(&secret).is_some());
            },
        );
        let facts = index.get(&alias).unwrap();
        assert_eq!(index.aliases_of(&facts, &alias), vec![secret]);
        assert!(index.status().complete);
    }

    #[test]
    fn racing_remove_cannot_be_resurrected_by_staged_publication() {
        let temp = tempfile::tempdir().unwrap();
        let removed = temp.path().join("removed.txt");
        std::fs::write(&removed, "exists").unwrap();
        let index = FileIndex::default();
        build_observed(
            &index,
            &[temp.path().into()],
            &AtomicBool::new(false),
            || {},
            || {
                std::fs::remove_file(&removed).unwrap();
                index.remove(&removed);
            },
        );
        assert!(
            index.get(&removed).is_none(),
            "staging must not resurrect a concurrent removal"
        );
        assert!(index.status().complete);
    }

    #[test]
    fn watcher_drop_watermark_coalesces_and_only_success_acknowledges() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("present.txt"), "present").unwrap();
        let index = FileIndex::default();
        assert!(index.observe_watcher_drops(1));
        let generation = index.generation();
        assert!(!index.observe_watcher_drops(1));
        assert_eq!(index.generation(), generation);
        build_blocking(
            &index,
            &[temp.path().join("missing")],
            &AtomicBool::new(false),
        );
        assert!(!index.status().complete);
        assert_eq!(index.processed_watcher_drops.load(Ordering::Relaxed), 0);
        build_observed(
            &index,
            &[temp.path().into()],
            &AtomicBool::new(false),
            || {},
            || {
                assert!(!index.observe_watcher_drops(1));
            },
        );
        assert!(index.status().complete);
        assert_eq!(index.processed_watcher_drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn repeated_build_invalidation_is_bounded_and_later_build_recovers() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("changing.txt");
        std::fs::write(&file, "initial").unwrap();
        let index = FileIndex::default();
        let shutdown = AtomicBool::new(false);
        let mut attempts = 0;
        build_retrying(
            || {
                attempts += 1;
                let mut started = Some(|| {});
                let mut before_publish = Some(|| {
                    std::fs::write(&file, "x".repeat(attempts)).unwrap();
                    index.refresh(&file);
                });
                let retry = build_attempt(
                    &index,
                    &[temp.path().into()],
                    &shutdown,
                    &mut started,
                    &mut before_publish,
                );
                assert!(
                    attempts <= 3,
                    "one worker must bound invalidated full scans"
                );
                retry
            },
            &shutdown,
        );
        assert_eq!(attempts, 3);
        assert!(!index.status().complete);
        build_blocking(&index, &[temp.path().into()], &shutdown);
        assert!(index.status().complete);
        assert_eq!(index.get(&file).unwrap().size, 3);
    }

    #[test]
    fn build_stops_on_shutdown() {
        let temp = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(temp.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let index = FileIndex::default();
        let shutdown = AtomicBool::new(true);

        build_blocking(&index, &[temp.path().to_path_buf()], &shutdown);

        assert_eq!(index.len(), 0);
    }
}
