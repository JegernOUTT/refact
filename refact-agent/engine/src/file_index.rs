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
const MIN_WALK_GAP_MS: u64 = 250;
const WALK_DUTY_FACTOR: u64 = 10;
const INCOMPLETE_WALK_BACKOFF_MS: [u64; 4] = [5_000, 15_000, 60_000, 300_000];
const CAP_LIMITED_WALK_INTERVAL_MS: u64 = 15 * 60_000;
const IMMEDIATE_RETRY_MAX_WALK_MS: u64 = 1_000;
const MAX_REREAD_PATHS_BEFORE_PUBLISH: usize = 256;
const MAX_PENDING_SUBTREE_SCANS: usize = 64;
const SUBTREE_SCAN_WAIT_STEP_MS: u64 = 50;
const MAX_SUBTREE_SCAN_PAUSE_MS: u64 = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileIndexWalkOutcome {
    Complete,
    CapLimited,
    WalkError,
    Invalidated,
    WatcherDrops,
    Shutdown,
}

impl FileIndexWalkOutcome {
    fn retriable(self) -> bool {
        matches!(
            self,
            Self::WalkError | Self::Invalidated | Self::WatcherDrops
        )
    }
    fn as_u8(self) -> u8 {
        match self {
            Self::Complete => 1,
            Self::CapLimited => 2,
            Self::WalkError => 3,
            Self::Invalidated => 4,
            Self::WatcherDrops => 5,
            Self::Shutdown => 6,
        }
    }
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Complete),
            2 => Some(Self::CapLimited),
            3 => Some(Self::WalkError),
            4 => Some(Self::Invalidated),
            5 => Some(Self::WatcherDrops),
            6 => Some(Self::Shutdown),
            _ => None,
        }
    }
}

struct LastWalk {
    ended_ms: u64,
    duration_ms: u64,
    roots: Vec<PathBuf>,
    outcome: FileIndexWalkOutcome,
}

#[derive(Default)]
struct SubtreeScanQueue {
    pending: Vec<(PathBuf, Vec<PathBuf>)>,
    worker: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum SubtreeScanEnqueue {
    Rejected,
    Covered,
    Queued { start_worker: bool },
    Overflow,
}

#[cfg(test)]
pub(crate) type SubtreeScanHook = Arc<dyn Fn(&Path) + Send + Sync>;

#[derive(Default)]
struct TouchedDuringWalk {
    paths: std::collections::HashSet<PathBuf>,
    overflow: bool,
}

#[derive(Default)]
struct WalkSchedule {
    running: bool,
    wanted: Option<Vec<PathBuf>>,
    last: Option<LastWalk>,
    consecutive_incomplete: u32,
}

impl WalkSchedule {
    fn not_before_ms(&self) -> u64 {
        let Some(last) = &self.last else {
            return 0;
        };
        let mut gap = last
            .duration_ms
            .saturating_mul(WALK_DUTY_FACTOR)
            .max(MIN_WALK_GAP_MS);
        if last.outcome.retriable() && self.consecutive_incomplete > 0 {
            let step = (self.consecutive_incomplete as usize - 1)
                .min(INCOMPLETE_WALK_BACKOFF_MS.len() - 1);
            gap = gap.max(INCOMPLETE_WALK_BACKOFF_MS[step]);
        }
        if last.outcome == FileIndexWalkOutcome::CapLimited {
            gap = gap.max(CAP_LIMITED_WALK_INTERVAL_MS);
        }
        last.ended_ms.saturating_add(gap)
    }

    fn due(&self, roots: &[PathBuf], now_ms: u64) -> bool {
        let Some(last) = &self.last else {
            return true;
        };
        let covered = roots.iter().all(|root| last.roots.contains(root));
        !covered || now_ms < last.ended_ms || now_ms >= self.not_before_ms()
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileIndexIncompleteReason {
    NeverBuilt,
    Building,
    CapLimited,
    WalkError,
    Invalidated,
    WatcherDrops,
    Shutdown,
    ChangedSinceWalk,
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
    pub last_walk_outcome: Option<FileIndexWalkOutcome>,
    pub incomplete_reason: Option<FileIndexIncompleteReason>,
    pub last_walk_duration_ms: u64,
    pub consecutive_incomplete_walks: u32,
    pub walk_running: bool,
    pub rebuild_pending: bool,
    pub next_walk_not_before_ms: u64,
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
    schedule: std::sync::Mutex<WalkSchedule>,
    last_attempt_outcome: AtomicU8,
    touched_during_walk: std::sync::Mutex<Option<TouchedDuringWalk>>,
    subtree_scans: std::sync::Mutex<SubtreeScanQueue>,
    #[cfg(test)]
    subtree_scan_hook: std::sync::Mutex<Option<SubtreeScanHook>>,
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
            schedule: std::sync::Mutex::new(WalkSchedule::default()),
            last_attempt_outcome: AtomicU8::new(0),
            touched_during_walk: std::sync::Mutex::new(None),
            subtree_scans: std::sync::Mutex::new(SubtreeScanQueue::default()),
            #[cfg(test)]
            subtree_scan_hook: std::sync::Mutex::new(None),
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
        self.record_touched(std::slice::from_ref(&path.to_path_buf()));
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

    /// Applies one watcher event and reports whether the inventory now wants a full walk.
    pub fn observe_event(&self, paths: &[PathBuf], structural: bool) -> bool {
        for path in paths {
            self.refresh(path);
        }
        if structural {
            self.mark_degraded();
            return true;
        }
        self.state() == FileIndexState::Degraded
    }

    /// Inventories a directory created or moved in, without waiting for a full walk.
    /// Returns false when the subtree could not be fully stored (walk error, cap or shutdown).
    pub fn inventory_subtree(&self, dir: &Path, roots: &[PathBuf], shutdown: &AtomicBool) -> bool {
        let Some((dir, root)) = subtree_scan_root(dir, roots) else {
            return true;
        };
        let mut complete = true;
        let walk = walkdir::WalkDir::new(&dir)
            .follow_links(false)
            .follow_root_links(false);
        for entry in walk {
            if shutdown.load(Ordering::Relaxed) {
                return false;
            }
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            if entry.file_type().is_dir() {
                if inventory_directory_visible(&root, entry.path())
                    && !self.upsert_entry(entry.path(), None)
                {
                    return false;
                }
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            match facts_for(entry.path()) {
                Some(facts) => {
                    if !self.upsert(entry.path(), facts) {
                        return false;
                    }
                }
                None => complete = false,
            }
        }
        #[cfg(test)]
        if let Some(hook) = self
            .subtree_scan_hook
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            hook(&dir);
        }
        complete
    }

    fn enqueue_subtree_scan(&self, dir: PathBuf, roots: Vec<PathBuf>) -> SubtreeScanEnqueue {
        let Some((dir, _)) = subtree_scan_root(&dir, &roots) else {
            return SubtreeScanEnqueue::Rejected;
        };
        let mut queue = self.subtree_scans.lock().unwrap_or_else(|p| p.into_inner());
        if queue
            .pending
            .iter()
            .any(|(ancestor, _)| dir.starts_with(ancestor))
        {
            return SubtreeScanEnqueue::Covered;
        }
        let mut position = None;
        let mut kept = 0;
        queue.pending.retain(|(pending, _)| {
            if pending.starts_with(&dir) {
                position.get_or_insert(kept);
                return false;
            }
            kept += 1;
            true
        });
        if queue.pending.len() >= MAX_PENDING_SUBTREE_SCANS {
            queue.pending.clear();
            return SubtreeScanEnqueue::Overflow;
        }
        let position = position.unwrap_or(queue.pending.len());
        queue.pending.insert(position, (dir, roots));
        let start_worker = !queue.worker;
        queue.worker = true;
        SubtreeScanEnqueue::Queued {
            start_worker,
        }
    }

    fn next_subtree_scan(&self, shutdown: bool) -> Option<(PathBuf, Vec<PathBuf>)> {
        let mut queue = self.subtree_scans.lock().unwrap_or_else(|p| p.into_inner());
        if shutdown {
            queue.pending.clear();
        }
        if queue.pending.is_empty() {
            queue.worker = false;
            return None;
        }
        Some(queue.pending.remove(0))
    }

    fn want_full_walk(&self, roots: Vec<PathBuf>) {
        if roots.is_empty() {
            return;
        }
        self.lock_schedule().wanted.get_or_insert(roots);
    }

    #[cfg(test)]
    pub(crate) fn set_subtree_scan_hook(&self, hook: Option<SubtreeScanHook>) {
        *self
            .subtree_scan_hook
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = hook;
    }

    fn record_touched(&self, paths: &[PathBuf]) {
        let mut touched = self
            .touched_during_walk
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Some(touched) = touched.as_mut() else {
            return;
        };
        for path in paths {
            if touched.paths.contains(path) {
                continue;
            }
            if touched.paths.len() >= MAX_REREAD_PATHS_BEFORE_PUBLISH {
                touched.overflow = true;
                return;
            }
            touched.paths.insert(path.clone());
        }
    }

    fn set_touched_tracking(&self, active: bool) -> Option<TouchedDuringWalk> {
        let mut touched = self
            .touched_during_walk
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        std::mem::replace(&mut *touched, active.then(TouchedDuringWalk::default))
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

    fn end_attempt(&self, outcome: FileIndexWalkOutcome) {
        self.set_touched_tracking(false);
        self.last_attempt_outcome
            .store(outcome.as_u8(), Ordering::Relaxed);
        self.finished_at_ms.store(now_ms(), Ordering::Relaxed);
    }

    fn last_attempt_outcome(&self) -> FileIndexWalkOutcome {
        FileIndexWalkOutcome::from_u8(self.last_attempt_outcome.load(Ordering::Relaxed))
            .unwrap_or(FileIndexWalkOutcome::WalkError)
    }

    fn lock_schedule(&self) -> std::sync::MutexGuard<'_, WalkSchedule> {
        self.schedule.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Single walk scheduler: records the wish and returns the roots to walk now, if due.
    pub(crate) fn request_walk_at(&self, roots: Vec<PathBuf>, now_ms: u64) -> Option<Vec<PathBuf>> {
        if roots.is_empty() {
            return None;
        }
        let mut schedule = self.lock_schedule();
        schedule.wanted = Some(roots);
        Self::take_due_walk(&mut schedule, now_ms)
    }

    pub(crate) fn poll_walk_at(&self, now_ms: u64) -> Option<Vec<PathBuf>> {
        Self::take_due_walk(&mut self.lock_schedule(), now_ms)
    }

    fn take_due_walk(schedule: &mut WalkSchedule, now_ms: u64) -> Option<Vec<PathBuf>> {
        if schedule.running {
            return None;
        }
        let due = schedule
            .wanted
            .as_deref()
            .is_some_and(|roots| schedule.due(roots, now_ms));
        if !due {
            return None;
        }
        schedule.running = true;
        schedule.wanted.take()
    }

    pub(crate) fn complete_walk_at(
        &self,
        roots: Vec<PathBuf>,
        ended_ms: u64,
        duration_ms: u64,
        outcome: FileIndexWalkOutcome,
        shutdown: bool,
    ) -> Option<Vec<PathBuf>> {
        let mut schedule = self.lock_schedule();
        match outcome {
            FileIndexWalkOutcome::Complete => schedule.consecutive_incomplete = 0,
            outcome if outcome.retriable() => {
                schedule.consecutive_incomplete = schedule.consecutive_incomplete.saturating_add(1);
                if schedule.wanted.is_none() {
                    schedule.wanted = Some(roots.clone());
                }
            }
            _ => {}
        }
        schedule.last = Some(LastWalk {
            ended_ms,
            duration_ms,
            roots,
            outcome,
        });
        schedule.running = false;
        if shutdown {
            return None;
        }
        Self::take_due_walk(&mut schedule, ended_ms)
    }

    pub fn status(&self) -> FileIndexStatus {
        let skipped = self.skipped_over_cap.load(Ordering::Relaxed);
        let state = self.state();
        let complete = skipped == 0 && state == FileIndexState::Ready;
        let (last_outcome, last_duration, consecutive, running, pending, not_before) = {
            let schedule = self.lock_schedule();
            (
                schedule.last.as_ref().map(|last| last.outcome),
                schedule.last.as_ref().map_or(0, |last| last.duration_ms),
                schedule.consecutive_incomplete,
                schedule.running,
                schedule.wanted.is_some(),
                schedule.not_before_ms(),
            )
        };
        let incomplete_reason = if complete {
            None
        } else if running || state == FileIndexState::Building {
            Some(FileIndexIncompleteReason::Building)
        } else if skipped > 0 || last_outcome == Some(FileIndexWalkOutcome::CapLimited) {
            Some(FileIndexIncompleteReason::CapLimited)
        } else {
            Some(match last_outcome {
                None => FileIndexIncompleteReason::NeverBuilt,
                Some(FileIndexWalkOutcome::Complete) | Some(FileIndexWalkOutcome::CapLimited) => {
                    FileIndexIncompleteReason::ChangedSinceWalk
                }
                Some(FileIndexWalkOutcome::WalkError) => FileIndexIncompleteReason::WalkError,
                Some(FileIndexWalkOutcome::Invalidated) => FileIndexIncompleteReason::Invalidated,
                Some(FileIndexWalkOutcome::WatcherDrops) => FileIndexIncompleteReason::WatcherDrops,
                Some(FileIndexWalkOutcome::Shutdown) => FileIndexIncompleteReason::Shutdown,
            })
        };
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
            complete,
            started_at_ms: self.started_at_ms.load(Ordering::Relaxed),
            finished_at_ms: self.finished_at_ms.load(Ordering::Relaxed),
            last_walk_outcome: last_outcome,
            incomplete_reason,
            last_walk_duration_ms: last_duration,
            consecutive_incomplete_walks: consecutive,
            walk_running: running,
            rebuild_pending: pending,
            next_walk_not_before_ms: not_before,
        }
    }
}

/// A subtree scan runs only on a real directory whose canonical path equals the requested path
/// with `.` and `..` removed, under a canonical workspace root; returns both paths.
/// An ancestor replaced by a symlink while a scan is running is not detected.
fn subtree_scan_root(dir: &Path, roots: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    let mut requested = PathBuf::new();
    for component in dir.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                requested.pop();
            }
            component => requested.push(component),
        }
    }
    let canonical = std::fs::canonicalize(dir).ok()?;
    let canonical = dunce::simplified(&canonical).to_path_buf();
    if canonical != requested
        || !std::fs::symlink_metadata(&canonical).is_ok_and(|metadata| metadata.is_dir())
    {
        return None;
    }
    let root = roots
        .iter()
        .filter_map(|root| {
            std::fs::canonicalize(root)
                .ok()
                .map(|root| dunce::simplified(&root).to_path_buf())
        })
        .filter(|root| canonical.starts_with(root))
        .max_by_key(|root| root.components().count())?;
    Some((canonical, root))
}

fn inventory_directory_visible(root: &Path, path: &Path) -> bool {
    // Only Explorer-visible directories need inventory slots; regular
    // files remain unfiltered for privacy hardlink identity discovery.
    path.strip_prefix(root).is_ok_and(|relative| {
        relative.components().all(|component| {
            let name = component.as_os_str().to_string_lossy();
            !name.starts_with('.') && !matches!(name.as_ref(), "target" | "node_modules")
        })
    })
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

#[cfg(test)]
pub(crate) fn tests_now_ms() -> u64 {
    now_ms()
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
    let started = std::time::Instant::now();
    build_observed_timed(
        index,
        roots,
        shutdown,
        || {},
        || {},
        || started.elapsed().as_millis() as u64,
    );
}

#[cfg(test)]
fn build_with_observer(
    index: &FileIndex,
    roots: &[PathBuf],
    shutdown: &AtomicBool,
    started: impl FnOnce(),
) {
    build_observed(index, roots, shutdown, started, || {});
}

#[cfg(test)]
fn build_observed(
    index: &FileIndex,
    roots: &[PathBuf],
    shutdown: &AtomicBool,
    started: impl FnOnce(),
    before_publish: impl FnOnce(),
) {
    build_observed_timed(index, roots, shutdown, started, before_publish, || 0);
}

fn build_observed_timed(
    index: &FileIndex,
    roots: &[PathBuf],
    shutdown: &AtomicBool,
    started: impl FnOnce(),
    before_publish: impl FnOnce(),
    elapsed_ms: impl Fn() -> u64,
) {
    let mut started = Some(started);
    let mut before_publish = Some(before_publish);
    build_retrying_timed(
        || build_attempt(index, roots, shutdown, &mut started, &mut before_publish),
        shutdown,
        elapsed_ms,
    );
}

#[cfg(test)]
fn build_retrying(attempt: impl FnMut() -> bool, shutdown: &AtomicBool) {
    build_retrying_timed(attempt, shutdown, || 0);
}

fn build_retrying_timed(
    mut attempt: impl FnMut() -> bool,
    shutdown: &AtomicBool,
    elapsed_ms: impl Fn() -> u64,
) {
    for _ in 0..3 {
        let attempt_started = elapsed_ms();
        if !attempt() || shutdown.load(Ordering::Relaxed) {
            return;
        }
        if elapsed_ms().saturating_sub(attempt_started) > IMMEDIATE_RETRY_MAX_WALK_MS {
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
    index.set_touched_tracking(true);
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
    let mut walk_error = canonical_roots.len() != roots.len();
    let mut pending: Vec<PathBuf> = Vec::with_capacity(MERGE_BATCH);
    for root in &canonical_roots {
        for entry in walkdir::WalkDir::new(root).follow_links(false).into_iter() {
            if shutdown.load(Ordering::Relaxed) {
                index.mark_degraded();
                index.end_attempt(FileIndexWalkOutcome::Shutdown);
                return false;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    walk_error = true;
                    continue;
                }
            };
            if entry.file_type().is_dir() && inventory_directory_visible(root, entry.path()) {
                staged.upsert_entry(entry.path(), None);
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
    let invalidated = |index: &FileIndex| {
        if index.pending_watcher_drops.load(Ordering::Relaxed) != watcher_drops {
            FileIndexWalkOutcome::WatcherDrops
        } else {
            FileIndexWalkOutcome::Invalidated
        }
    };
    let cap_limited = staged.skipped_over_cap.load(Ordering::Relaxed) > 0;
    if walk_error || staged.state() == FileIndexState::Degraded {
        let mut entries = index.entries.write().unwrap_or_else(|p| p.into_inner());
        if generation != index.generation() {
            index.set_state(FileIndexState::Degraded);
            index.end_attempt(if cap_limited {
                FileIndexWalkOutcome::CapLimited
            } else {
                invalidated(index)
            });
            return !cap_limited && !shutdown.load(Ordering::Relaxed);
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
        drop(entries);
        index.end_attempt(
            if cap_limited || index.skipped_over_cap.load(Ordering::Relaxed) > 0 {
                FileIndexWalkOutcome::CapLimited
            } else if walk_error {
                FileIndexWalkOutcome::WalkError
            } else {
                FileIndexWalkOutcome::Invalidated
            },
        );
        return false;
    }
    if generation != index.generation() {
        // Keeping the previous identities is conservative for privacy and avoids
        // replacing them with an incomplete map after errors or concurrent changes.
        index.mark_degraded();
        index.end_attempt(invalidated(index));
        return !shutdown.load(Ordering::Relaxed);
    }
    let mut entries = index.entries.write().unwrap_or_else(|p| p.into_inner());
    let mut identities = index.identities.write().unwrap_or_else(|p| p.into_inner());
    let touched = index.set_touched_tracking(false).unwrap_or_default();
    if generation != index.generation() || touched.overflow {
        index.set_state(FileIndexState::Degraded);
        drop(identities);
        drop(entries);
        index.end_attempt(invalidated(index));
        return !shutdown.load(Ordering::Relaxed);
    }
    for path in &touched.paths {
        staged.refresh(path);
    }
    if staged.skipped_over_cap.load(Ordering::Relaxed) > 0 {
        index.set_state(FileIndexState::Degraded);
        drop(identities);
        drop(entries);
        index.end_attempt(FileIndexWalkOutcome::CapLimited);
        return false;
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
    index.end_attempt(if index.skipped_over_cap.load(Ordering::Relaxed) > 0 {
        FileIndexWalkOutcome::CapLimited
    } else {
        FileIndexWalkOutcome::Complete
    });
    false
}

/// Requests a full inventory walk through the scheduler; returns a worker only if one starts now.
pub fn spawn_build(
    index: Arc<FileIndex>,
    roots: Vec<PathBuf>,
    shutdown: Arc<AtomicBool>,
) -> Option<tokio::task::JoinHandle<()>> {
    if shutdown.load(Ordering::Relaxed) {
        return None;
    }
    let roots = index.request_walk_at(roots, now_ms())?;
    Some(spawn_walk_worker(index, roots, shutdown))
}

/// Starts a previously deferred walk once it is due; called from the watcher recovery tick.
pub fn spawn_due_build(
    index: Arc<FileIndex>,
    shutdown: Arc<AtomicBool>,
) -> Option<tokio::task::JoinHandle<()>> {
    if shutdown.load(Ordering::Relaxed) {
        return None;
    }
    let roots = index.poll_walk_at(now_ms())?;
    Some(spawn_walk_worker(index, roots, shutdown))
}

/// Queues an immediate inventory of a directory created or moved in. Never waits for the scan.
pub fn request_subtree_scan(
    index: Arc<FileIndex>,
    dir: PathBuf,
    roots: Vec<PathBuf>,
    shutdown: Arc<AtomicBool>,
) -> Option<tokio::task::JoinHandle<()>> {
    if shutdown.load(Ordering::Relaxed) {
        return None;
    }
    match index.enqueue_subtree_scan(dir, roots.clone()) {
        SubtreeScanEnqueue::Rejected
        | SubtreeScanEnqueue::Covered
        | SubtreeScanEnqueue::Queued {
            start_worker: false,
        } => None,
        SubtreeScanEnqueue::Queued {
            start_worker: true,
        } => Some(tokio::spawn(run_subtree_scan_worker(index, shutdown))),
        SubtreeScanEnqueue::Overflow => {
            index.mark_degraded();
            let roots = index.request_walk_at(roots, now_ms())?;
            Some(spawn_walk_worker(index, roots, shutdown))
        }
    }
}

async fn run_subtree_scan_worker(index: Arc<FileIndex>, shutdown: Arc<AtomicBool>) {
    while let Some((dir, roots)) = index.next_subtree_scan(shutdown.load(Ordering::Relaxed)) {
        let started = tokio::time::Instant::now();
        let scan_index = index.clone();
        let scan_shutdown = shutdown.clone();
        let scan_roots = roots.clone();
        let complete = tokio::task::spawn_blocking(move || {
            scan_index.inventory_subtree(&dir, &scan_roots, &scan_shutdown)
        })
        .await
        .unwrap_or(false);
        if !complete && !shutdown.load(Ordering::Relaxed) {
            index.mark_degraded();
            index.want_full_walk(roots);
        }
        pause_after_subtree_scan(started.elapsed(), &shutdown).await;
    }
}

async fn pause_after_subtree_scan(elapsed: std::time::Duration, shutdown: &AtomicBool) {
    let mut remaining = elapsed.min(std::time::Duration::from_millis(MAX_SUBTREE_SCAN_PAUSE_MS));
    while !remaining.is_zero() && !shutdown.load(Ordering::Relaxed) {
        let step = remaining.min(std::time::Duration::from_millis(SUBTREE_SCAN_WAIT_STEP_MS));
        tokio::time::sleep(step).await;
        remaining -= step;
    }
}

fn spawn_walk_worker(
    index: Arc<FileIndex>,
    roots: Vec<PathBuf>,
    shutdown: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let mut roots = roots;
        loop {
            let started = std::time::Instant::now();
            let outcome = if shutdown.load(Ordering::Relaxed) {
                FileIndexWalkOutcome::Shutdown
            } else {
                build_blocking(&index, &roots, &shutdown);
                index.last_attempt_outcome()
            };
            let duration_ms = started.elapsed().as_millis() as u64;
            match index.complete_walk_at(
                roots,
                now_ms(),
                duration_ms,
                outcome,
                shutdown.load(Ordering::Relaxed),
            ) {
                Some(next) => roots = next,
                None => return,
            }
        }
    })
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

    fn over_cap_workspace() -> (tempfile::TempDir, FileIndex) {
        let temp = tempfile::tempdir().unwrap();
        for i in 0..8 {
            std::fs::write(temp.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        (temp, FileIndex::new(1))
    }

    fn run_walk(index: &FileIndex, roots: Vec<PathBuf>, ended_ms: u64, duration_ms: u64) {
        let shutdown = AtomicBool::new(false);
        build_blocking(index, &roots, &shutdown);
        let outcome = index.last_attempt_outcome();
        assert!(index
            .complete_walk_at(roots, ended_ms, duration_ms, outcome, false)
            .is_none());
    }

    #[test]
    fn over_cap_inventory_stops_rebuilding_on_event_bursts() {
        let (temp, index) = over_cap_workspace();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let roots = vec![root.clone()];
        let t0 = 1_000_000;
        assert_eq!(
            index.request_walk_at(roots.clone(), t0),
            Some(roots.clone())
        );
        run_walk(&index, roots.clone(), t0 + 10, 10);
        let status = index.status();
        assert!(
            status.finished_at_ms > 0,
            "an over-cap attempt must record its end"
        );
        assert_eq!(
            status.last_walk_outcome,
            Some(FileIndexWalkOutcome::CapLimited)
        );
        assert_eq!(
            status.incomplete_reason,
            Some(FileIndexIncompleteReason::CapLimited)
        );
        assert_eq!(index.state(), FileIndexState::Degraded);
        for i in 0..50u64 {
            let path = root.join(format!("f{}.txt", i % 8));
            let wants_walk = index.observe_event(&[path], i % 3 == 0);
            if wants_walk {
                assert_eq!(
                    index.request_walk_at(roots.clone(), t0 + 1_000 + i * 1_000),
                    None,
                    "event {i} must not start another walk of an over-cap inventory"
                );
            }
        }
        assert_eq!(index.poll_walk_at(t0 + 60_000), None);
        assert!(index.status().rebuild_pending);
        assert!(index.status().next_walk_not_before_ms >= t0 + 10 + CAP_LIMITED_WALK_INTERVAL_MS);
        let other = tempfile::tempdir().unwrap();
        let uncovered = vec![root.clone(), std::fs::canonicalize(other.path()).unwrap()];
        assert_eq!(
            index.request_walk_at(uncovered.clone(), t0 + 61_000),
            Some(uncovered.clone()),
            "uncovered roots are never delayed"
        );
        run_walk(&index, uncovered.clone(), t0 + 62_000, 10);
        assert_eq!(index.request_walk_at(uncovered.clone(), t0 + 63_000), None);
        assert_eq!(
            index.poll_walk_at(t0 + 62_000 + CAP_LIMITED_WALK_INTERVAL_MS),
            Some(uncovered),
            "after the long interval one more walk is allowed"
        );
    }

    #[test]
    fn duty_cycle_defers_requests_and_tick_serves_them_without_new_events() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.txt"), "a").unwrap();
        let index = FileIndex::default();
        let roots = vec![std::fs::canonicalize(temp.path()).unwrap()];
        let t0 = 5_000_000;
        let duration = 2_000;
        assert!(index.request_walk_at(roots.clone(), t0).is_some());
        assert_eq!(
            index.request_walk_at(roots.clone(), t0 + 1),
            None,
            "one walk at a time"
        );
        run_walk(&index, roots.clone(), t0 + duration, duration);
        assert!(index.status().complete);
        let gap = duration * WALK_DUTY_FACTOR;
        assert!(index.observe_event(&[roots[0].join("b.txt")], true));
        assert_eq!(index.state(), FileIndexState::Degraded);
        assert_eq!(
            index.request_walk_at(roots.clone(), t0 + duration + gap - 1),
            None,
            "a request inside the duty-cycle gap must be deferred"
        );
        assert_eq!(index.poll_walk_at(t0 + duration + gap - 1), None);
        assert_eq!(
            index.poll_walk_at(t0 + duration + gap),
            Some(roots.clone()),
            "the tick must start the deferred walk once due"
        );
        run_walk(&index, roots.clone(), t0 + duration + gap + 5, 5);
        assert_eq!(index.state(), FileIndexState::Ready);
        let next = t0 + duration + gap + 5 + MIN_WALK_GAP_MS;
        assert_eq!(index.request_walk_at(roots.clone(), next - 1), None);
        assert_eq!(index.poll_walk_at(next), Some(roots));
    }

    #[test]
    fn incomplete_walk_backoff_grows_and_resets_after_complete_build() {
        let index = FileIndex::default();
        let roots = vec![PathBuf::from("/root-a")];
        let mut now = 10_000_000;
        assert!(index.request_walk_at(roots.clone(), now).is_some());
        for expected in INCOMPLETE_WALK_BACKOFF_MS.iter().chain([&300_000u64]) {
            now += 1;
            let next = index.complete_walk_at(
                roots.clone(),
                now,
                1,
                FileIndexWalkOutcome::Invalidated,
                false,
            );
            assert_eq!(
                next, None,
                "an invalidated walk is not repeated immediately"
            );
            assert!(index.status().rebuild_pending, "the rebuild stays wanted");
            assert_eq!(index.status().next_walk_not_before_ms, now + expected);
            assert_eq!(index.poll_walk_at(now + expected - 1), None);
            now += expected;
            assert_eq!(index.poll_walk_at(now), Some(roots.clone()));
        }
        assert_eq!(index.status().consecutive_incomplete_walks, 5);
        now += 1;
        assert_eq!(
            index.complete_walk_at(roots.clone(), now, 1, FileIndexWalkOutcome::Complete, false),
            None
        );
        assert_eq!(index.status().consecutive_incomplete_walks, 0);
        assert_eq!(
            index.request_walk_at(roots.clone(), now + MIN_WALK_GAP_MS),
            Some(roots)
        );
    }

    #[test]
    fn structural_event_degrades_ready_index_and_later_walk_restores_ready() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.txt"), "a").unwrap();
        let index = FileIndex::default();
        let roots = vec![std::fs::canonicalize(temp.path()).unwrap()];
        let t0 = 20_000_000;
        assert!(index.request_walk_at(roots.clone(), t0).is_some());
        run_walk(&index, roots.clone(), t0 + 100, 100);
        assert_eq!(index.state(), FileIndexState::Ready);
        let created = roots[0].join("b.txt");
        std::fs::write(&created, "b").unwrap();
        assert!(index.observe_event(&[created.clone()], true));
        assert_eq!(index.state(), FileIndexState::Degraded);
        assert!(!index.status().complete);
        assert_eq!(index.request_walk_at(roots.clone(), t0 + 200), None);
        assert_eq!(index.poll_walk_at(t0 + 100 + 1_000), Some(roots.clone()));
        run_walk(&index, roots, t0 + 1_200, 100);
        assert_eq!(index.state(), FileIndexState::Ready);
        assert!(index.get(&created).is_some());
    }

    #[test]
    fn non_structural_event_on_degraded_index_does_not_bump_generation() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.txt");
        std::fs::write(&file, "a").unwrap();
        let index = FileIndex::default();
        index.refresh(&file);
        index.mark_degraded();
        let generation = index.generation();
        assert!(index.observe_event(&[file], false));
        assert_eq!(index.generation(), generation);
    }

    #[test]
    fn never_built_or_uncovered_requests_are_immediate() {
        let index = FileIndex::default();
        let a = vec![PathBuf::from("/root-a")];
        assert_eq!(index.request_walk_at(a.clone(), 1), Some(a.clone()));
        assert!(index
            .complete_walk_at(a.clone(), 2, 60_000, FileIndexWalkOutcome::WalkError, false)
            .is_none());
        let b = vec![PathBuf::from("/root-b")];
        assert_eq!(index.request_walk_at(b.clone(), 3), Some(b));
    }

    #[test]
    fn long_walk_is_not_retried_immediately() {
        let shutdown = AtomicBool::new(false);
        let clock = std::cell::Cell::new(0u64);
        let mut attempts = 0;
        build_retrying_timed(
            || {
                attempts += 1;
                clock.set(clock.get() + IMMEDIATE_RETRY_MAX_WALK_MS + 1);
                true
            },
            &shutdown,
            || clock.get(),
        );
        assert_eq!(attempts, 1);
    }

    #[test]
    fn cap_limited_attempt_is_not_retried_inside_build_retrying() {
        let (temp, index) = over_cap_workspace();
        let shutdown = AtomicBool::new(false);
        let mut attempts = 0;
        build_retrying(
            || {
                attempts += 1;
                let mut started = Some(|| {});
                let mut before_publish = Some(|| index.mark_degraded());
                build_attempt(
                    &index,
                    &[temp.path().into()],
                    &shutdown,
                    &mut started,
                    &mut before_publish,
                )
            },
            &shutdown,
        );
        assert_eq!(attempts, 1);
        assert_eq!(
            index.last_attempt_outcome(),
            FileIndexWalkOutcome::CapLimited
        );
    }

    #[test]
    fn walk_reading_transient_facts_cannot_publish_them_after_aba() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("flapping.txt");
        std::fs::write(&file, "a").unwrap();
        let file = std::fs::canonicalize(&file).unwrap();
        let original_mtime = std::fs::metadata(&file).unwrap().modified().unwrap();
        let index = FileIndex::default();
        let shutdown = AtomicBool::new(false);
        build_blocking(&index, &[temp.path().into()], &shutdown);
        let live = index.get(&file).unwrap();
        build_observed(
            &index,
            &[temp.path().into()],
            &shutdown,
            || std::fs::write(&file, "bb").unwrap(),
            || {
                std::fs::write(&file, "a").unwrap();
                std::fs::File::options()
                    .write(true)
                    .open(&file)
                    .unwrap()
                    .set_modified(original_mtime)
                    .unwrap();
                assert_eq!(facts_for(&file), Some(live));
                index.refresh(&file);
                index.refresh(&file);
            },
        );
        assert_eq!(
            index.get(&file),
            Some(live),
            "a path touched during the walk must be re-read before publication"
        );
        assert!(index.status().complete);
    }

    #[test]
    fn too_many_touched_paths_invalidate_publication() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.txt"), "a").unwrap();
        let index = FileIndex::default();
        let shutdown = AtomicBool::new(false);
        let mut started = Some(|| {});
        let mut before_publish = Some(|| {
            let paths: Vec<_> = (0..=MAX_REREAD_PATHS_BEFORE_PUBLISH)
                .map(|i| temp.path().join(format!("missing-{i}")))
                .collect();
            index.record_touched(&paths);
        });
        let retry = build_attempt(
            &index,
            &[temp.path().into()],
            &shutdown,
            &mut started,
            &mut before_publish,
        );
        assert!(retry);
        assert_eq!(
            index.last_attempt_outcome(),
            FileIndexWalkOutcome::Invalidated
        );
        assert!(!index.status().complete);
    }

    #[test]
    fn immediate_retry_threshold_is_inclusive() {
        let shutdown = AtomicBool::new(false);
        for (duration, expected) in [
            (IMMEDIATE_RETRY_MAX_WALK_MS, 3),
            (IMMEDIATE_RETRY_MAX_WALK_MS + 1, 1),
        ] {
            let clock = std::cell::Cell::new(0u64);
            let mut attempts = 0;
            build_retrying_timed(
                || {
                    attempts += 1;
                    clock.set(clock.get() + duration);
                    true
                },
                &shutdown,
                || clock.get(),
            );
            assert_eq!(attempts, expected, "walk of {duration} ms");
        }
    }

    #[test]
    fn repeated_touch_of_recorded_path_does_not_overflow() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.txt"), "a").unwrap();
        let index = FileIndex::default();
        let mut started = Some(|| {});
        let mut before_publish = Some(|| {
            let paths: Vec<_> = (0..MAX_REREAD_PATHS_BEFORE_PUBLISH)
                .map(|i| temp.path().join(format!("missing-{i}")))
                .collect();
            index.record_touched(&paths);
            index.record_touched(&paths[..1]);
        });
        let retry = build_attempt(
            &index,
            &[temp.path().into()],
            &AtomicBool::new(false),
            &mut started,
            &mut before_publish,
        );
        assert!(!retry);
        assert_eq!(
            index.last_attempt_outcome(),
            FileIndexWalkOutcome::Complete,
            "a repeat is not a new path"
        );
        assert!(index.status().complete);
    }

    #[cfg(unix)]
    #[test]
    fn subtree_scan_never_follows_a_linked_root() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        std::fs::write(external.path().join("outside.txt"), "x").unwrap();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        let link = root.join("linked");
        std::os::unix::fs::symlink(external.path(), &link).unwrap();
        let index = FileIndex::default();
        assert!(index.inventory_subtree(&link, &[root.clone()], &AtomicBool::new(false)));
        assert!(
            index
                .snapshot()
                .iter()
                .all(|(path, _)| !path.starts_with(&link)),
            "a linked root must not be inventoried under workspace paths"
        );
    }

    fn canonical_tempdir() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::simplified(&std::fs::canonicalize(temp.path()).unwrap()).to_path_buf();
        (temp, root)
    }

    #[test]
    fn nested_subtree_requests_coalesce_against_pending_ancestors_only() {
        let (_temp, root) = canonical_tempdir();
        for dir in ["a/b/c", "a/d", "a/e", "x", "y"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let index = FileIndex::default();
        let roots = vec![root.clone()];
        let enqueue = |dir: &str| index.enqueue_subtree_scan(root.join(dir), roots.clone());
        assert_eq!(
            enqueue("a/b/c"),
            SubtreeScanEnqueue::Queued {
                start_worker: true
            }
        );
        assert!(matches!(enqueue("x"), SubtreeScanEnqueue::Queued { .. }));
        assert!(matches!(enqueue("a/b"), SubtreeScanEnqueue::Queued { .. }));
        assert!(matches!(enqueue("y"), SubtreeScanEnqueue::Queued { .. }));
        assert_eq!(
            enqueue("a"),
            SubtreeScanEnqueue::Queued {
                start_worker: false
            }
        );
        assert_eq!(enqueue("a/d"), SubtreeScanEnqueue::Covered);
        let next = || index.next_subtree_scan(false).map(|(dir, _)| dir);
        assert_eq!(
            next(),
            Some(root.join("a")),
            "a replacing request takes the earliest position of the requests it covers"
        );
        assert_eq!(
            enqueue("a/e"),
            SubtreeScanEnqueue::Queued {
                start_worker: false
            },
            "a running ancestor does not cover a new request"
        );
        let next = || index.next_subtree_scan(false).map(|(dir, _)| dir);
        assert_eq!(next(), Some(root.join("x")));
        assert_eq!(next(), Some(root.join("y")));
        assert_eq!(next(), Some(root.join("a/e")));
        assert_eq!(next(), None, "a finished scan is never requeued");
    }

    #[test]
    fn subtree_requests_coalesce_on_the_canonical_path() {
        let (_temp, root) = canonical_tempdir();
        for dir in ["a", "b"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let index = FileIndex::default();
        let roots = vec![root.clone()];
        assert!(matches!(
            index.enqueue_subtree_scan(root.join("a"), roots.clone()),
            SubtreeScanEnqueue::Queued { .. }
        ));
        assert!(
            matches!(
                index.enqueue_subtree_scan(root.join("a/../b"), roots),
                SubtreeScanEnqueue::Queued { .. }
            ),
            "a/../b is not covered by a"
        );
        let next = || index.next_subtree_scan(false).map(|(dir, _)| dir);
        assert_eq!(next(), Some(root.join("a")));
        assert_eq!(next(), Some(root.join("b")));
        assert_eq!(next(), None);
    }

    #[cfg(unix)]
    #[test]
    fn subtree_scan_never_leaves_the_workspace_through_a_linked_ancestor() {
        let (_workspace, root) = canonical_tempdir();
        let external = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(external.path().join("dir/inner")).unwrap();
        std::fs::write(external.path().join("dir/outside.txt"), "x").unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(external.path(), &link).unwrap();
        let index = FileIndex::default();
        let roots = vec![root.clone()];
        assert!(index.inventory_subtree(&link.join("dir"), &roots, &AtomicBool::new(false)));
        assert!(
            index
                .snapshot()
                .iter()
                .all(|(path, _)| !path.starts_with(&link)),
            "a directory beneath a linked ancestor must not be inventoried under workspace paths"
        );
        assert!(
            !matches!(
                index.enqueue_subtree_scan(link.join("dir"), roots),
                SubtreeScanEnqueue::Queued { .. }
            ),
            "a directory beneath a linked ancestor is never queued"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn directory_moved_under_a_running_scan_is_inventoried_by_a_follow_up_scan() {
        let (_temp, root) = canonical_tempdir();
        let scanned = root.join("a");
        std::fs::create_dir_all(scanned.join("parent")).unwrap();
        std::fs::create_dir_all(root.join("staging/new/deep")).unwrap();
        std::fs::write(root.join("staging/new/deep/file.txt"), "x").unwrap();
        let index = Arc::new(FileIndex::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let (passed_tx, passed_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = Arc::new(std::sync::Mutex::new(release_rx));
        let barrier_dir = scanned.clone();
        index.set_subtree_scan_hook(Some(Arc::new(move |dir: &Path| {
            if dir == barrier_dir {
                let _ = passed_tx.send(());
                let _ = release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(10));
            }
        })));
        let worker = request_subtree_scan(
            index.clone(),
            scanned.clone(),
            vec![root.clone()],
            shutdown.clone(),
        )
        .expect("the first request starts the worker");
        tokio::task::spawn_blocking(move || {
            passed_rx.recv_timeout(std::time::Duration::from_secs(10))
        })
        .await
        .unwrap()
        .expect("the scan must have passed every entry of a");
        assert!(index
            .snapshot()
            .iter()
            .any(|(path, _)| path == &scanned.join("parent")));

        let moved = scanned.join("parent/new");
        std::fs::rename(root.join("staging/new"), &moved).unwrap();
        assert!(
            request_subtree_scan(index.clone(), moved.clone(), vec![root.clone()], shutdown)
                .is_none()
        );
        release_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(20), worker)
            .await
            .expect("the scan queue must drain")
            .unwrap();

        assert!(
            index.get(&moved.join("deep/file.txt")).is_some(),
            "the moved-in directory must be inventoried by a follow-up scan"
        );
        let status = index.status();
        assert!(!status.walk_running && !status.rebuild_pending);
        assert_eq!(status.last_walk_outcome, None, "no full walk ran");
    }

    #[tokio::test(start_paused = true)]
    async fn pause_after_a_slow_subtree_scan_is_capped() {
        let shutdown = AtomicBool::new(false);
        let started = tokio::time::Instant::now();
        pause_after_subtree_scan(std::time::Duration::from_secs(600), &shutdown).await;
        assert_eq!(
            started.elapsed(),
            std::time::Duration::from_millis(MAX_SUBTREE_SCAN_PAUSE_MS)
        );
        let started = tokio::time::Instant::now();
        pause_after_subtree_scan(std::time::Duration::from_millis(120), &shutdown).await;
        assert_eq!(started.elapsed(), std::time::Duration::from_millis(120));
    }

    #[tokio::test]
    async fn subtree_queue_overflow_falls_back_to_a_scheduled_full_walk() {
        let (_temp, root) = canonical_tempdir();
        std::fs::create_dir(root.join("overflow")).unwrap();
        let index = Arc::new(FileIndex::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        for i in 0..MAX_PENDING_SUBTREE_SCANS {
            std::fs::create_dir(root.join(format!("d{i}"))).unwrap();
            assert!(matches!(
                index.enqueue_subtree_scan(root.join(format!("d{i}")), vec![root.clone()]),
                SubtreeScanEnqueue::Queued { .. }
            ));
        }
        let walk = request_subtree_scan(
            index.clone(),
            root.join("overflow"),
            vec![root.clone()],
            shutdown,
        )
        .expect("overflow must request a full walk");
        assert_eq!(
            index.next_subtree_scan(false),
            None,
            "pending scans are dropped"
        );
        walk.await.unwrap();
        assert!(index.covers_roots(&[root]));
        assert!(index.status().complete);
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
