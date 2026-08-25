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
    entries: StdRwLock<HashMap<PathBuf, FileFacts>>,
    identities: StdRwLock<HashMap<(u64, u64), Vec<PathBuf>>>,
    bytes: AtomicU64,
    cap_bytes: AtomicU64,
    state: AtomicU8,
    scanned: AtomicUsize,
    skipped_over_cap: AtomicUsize,
    started_at_ms: AtomicU64,
    finished_at_ms: AtomicU64,
    build_running: AtomicBool,
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
        let cost = entry_cost(path);
        let outcome = {
            let mut guard = self
                .entries
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(slot) = guard.get_mut(path) {
                let previous = *slot;
                *slot = facts;
                Some(Some(previous))
            } else if self.bytes.load(Ordering::Relaxed) + cost
                > self.cap_bytes.load(Ordering::Relaxed)
            {
                None
            } else {
                guard.insert(path.to_path_buf(), facts);
                self.bytes.fetch_add(cost, Ordering::Relaxed);
                Some(None)
            }
        };
        match outcome {
            None => {
                self.skipped_over_cap.fetch_add(1, Ordering::Relaxed);
                self.set_state(FileIndexState::Degraded);
                false
            }
            Some(previous) => {
                if let Some(previous) = previous {
                    if previous.dev != facts.dev || previous.ino != facts.ino {
                        self.identity_forget(path, &previous);
                    }
                }
                self.identity_record(path, &facts);
                true
            }
        }
    }

    pub fn remove(&self, path: &Path) {
        let removed = {
            let mut guard = self
                .entries
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.remove(path)
        };
        if let Some(facts) = removed {
            let cost = entry_cost(path).min(self.bytes.load(Ordering::Relaxed));
            self.bytes.fetch_sub(cost, Ordering::Relaxed);
            self.identity_forget(path, &facts);
        }
    }

    pub fn refresh(&self, path: &Path) {
        match facts_for(path) {
            Some(facts) => {
                self.upsert(path, facts);
            }
            None => self.remove(path),
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
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn begin_build(&self) {
        self.set_state(FileIndexState::Building);
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
    use rayon::prelude::*;

    fn flush(index: &FileIndex, pending: &mut Vec<PathBuf>) {
        if pending.is_empty() {
            return;
        }
        let batch: Vec<(PathBuf, FileFacts)> = std::mem::take(pending)
            .into_par_iter()
            .filter_map(|path| facts_for(&path).map(|facts| (path, facts)))
            .collect();
        index.scanned.fetch_add(batch.len(), Ordering::Relaxed);
        index.merge_batch(batch);
    }

    index.begin_build();
    let mut pending: Vec<PathBuf> = Vec::with_capacity(MERGE_BATCH);
    for root in roots {
        for entry in walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if shutdown.load(Ordering::Relaxed) {
                index.finish_build();
                return;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            pending.push(entry.into_path());
            if pending.len() >= MERGE_BATCH {
                flush(index, &mut pending);
            }
        }
    }
    flush(index, &mut pending);
    index.finish_build();
}

pub fn spawn_build(
    index: Arc<FileIndex>,
    roots: Vec<PathBuf>,
    shutdown: Arc<AtomicBool>,
) -> Option<tokio::task::JoinHandle<()>> {
    if roots.is_empty() || index.build_running.swap(true, Ordering::SeqCst) {
        return None;
    }
    Some(tokio::task::spawn_blocking(move || {
        build_blocking(&index, &roots, &shutdown);
        index.build_running.store(false, Ordering::SeqCst);
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_falls_back_to_a_single_stat_and_caches() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.txt");
        std::fs::write(&file, "hello").unwrap();
        let index = FileIndex::default();

        assert!(index.get(&file).is_none());
        let facts = index.lookup_or_stat(&file).expect("stat fallback");

        assert_eq!(facts.size, 5);
        assert!(index.get(&file).is_some());
    }

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

    #[test]
    fn delete_invalidates_only_that_entry() {
        let temp = tempfile::tempdir().unwrap();
        let kept = temp.path().join("kept.txt");
        let gone = temp.path().join("gone.txt");
        std::fs::write(&kept, "a").unwrap();
        std::fs::write(&gone, "b").unwrap();
        let index = FileIndex::default();
        index.refresh(&kept);
        index.refresh(&gone);
        assert_eq!(index.len(), 2);

        std::fs::remove_file(&gone).unwrap();
        index.refresh(&gone);

        assert_eq!(index.len(), 1);
        assert!(index.get(&kept).is_some());
        assert!(index.get(&gone).is_none());
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

    #[test]
    fn build_populates_and_reports_ready() {
        let temp = tempfile::tempdir().unwrap();
        for i in 0..10 {
            std::fs::write(temp.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let index = FileIndex::default();
        let shutdown = AtomicBool::new(false);

        build_blocking(&index, &[temp.path().to_path_buf()], &shutdown);

        let status = index.status();
        assert_eq!(status.state, FileIndexState::Ready);
        assert_eq!(status.entries, 10);
        assert!(status.complete);
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
