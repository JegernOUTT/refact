pub mod analytics;
pub mod communities;
pub mod config;
pub mod dead_code;
pub mod extract;
pub mod facade;
pub mod pr_blast;
pub mod retrieval;
pub mod schema;
pub mod security_scan;
pub mod store;
pub mod symbols_fmt;

pub use retrieval::CodeHit;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AMutex;
use tokio::sync::Notify;
use tracing::debug;

pub use store::{
    Counts, IndexedFile, ParseFailure, Store, StoreIdentity, WalCheckpointMode, WalCheckpointResult,
};

pub const PARSE_FAILURE_REPORT_LIMIT: i64 = 50;

pub fn lang_from_path(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
    {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => "cpp",
        "go" => "go",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "scala" => "scala",
        "sh" | "bash" => "bash",
        "ex" | "exs" => "elixir",
        "ml" | "mli" => "ocaml",
        "hs" => "haskell",
        _ => "",
    }
}

#[derive(Debug, Clone)]
pub struct CachedGraphAnalytics {
    pub generation: u64,
    pub counts: Counts,
    pub data: analytics::GraphData,
    pub analytics: analytics::GraphAnalytics,
    pub communities: Vec<communities::Community>,
    pub dead_code: dead_code::DeadCodeResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScopedAnalyticsKey {
    generation: u64,
    allowed_paths: Vec<String>,
}

#[derive(Debug, Clone)]
struct ScopedAnalyticsCache {
    entries: VecDeque<ScopedAnalyticsCacheEntry>,
}

#[derive(Debug, Clone)]
struct ScopedAnalyticsCacheEntry {
    key: ScopedAnalyticsKey,
    value: Arc<CachedGraphAnalytics>,
}

const SCOPED_ANALYTICS_CACHE_CAPACITY: usize = 4;
const SCOPED_ANALYTICS_MAX_REBUILD_ATTEMPTS: usize = 2;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexReadiness {
    pub queued: usize,
    pub dirty_paths: i64,
    pub pending_refs: i64,
    pub cross_file_edges: i64,
    pub cross_file_ready: bool,
    pub parse_failures: i64,
    pub parse_failure_paths: Vec<String>,
}

pub const DB_UNAVAILABLE_ERROR: &str = "codegraph database is unavailable";

fn unavailable_error() -> String {
    format!("{DB_UNAVAILABLE_ERROR}: waiting for the store to be reopened")
}

/// Minimum spacing between reopen attempts after another process replaced the database, so a
/// permanently failing reopen degrades to the idle cadence instead of spinning.
const REOPEN_RETRY_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseRecovery {
    /// The connections still point at the file on disk.
    NotNeeded,
    /// Both connections were reopened on the current file.
    Recovered,
    /// A replacement was seen but the retry interval has not elapsed yet.
    Deferred,
    /// The database is unavailable; the next attempt happens on the idle cadence.
    Failed(String),
}

pub struct CodeGraphService {
    store: AMutex<Option<Store>>,
    read_store: Option<AMutex<Option<Store>>>,
    queue: StdMutex<PendingQueue>,
    throughput: StdMutex<ThroughputWindow>,
    queue_notify: Notify,
    db_path: PathBuf,
    initial_index_done: AtomicBool,
    graph_generation: AtomicU64,
    analytics_cache: AMutex<Option<Arc<CachedGraphAnalytics>>>,
    scoped_analytics_cache: AMutex<ScopedAnalyticsCache>,
    analytics_rebuild_count: AtomicUsize,
    scoped_analytics_rebuild_count: AtomicUsize,
    run_parse_failures: AtomicUsize,
    store_generation: AtomicU64,
    reopen_attempts: AtomicUsize,
    last_reopen_attempt: StdMutex<Option<Instant>>,
    writer_identity: StdMutex<Option<Arc<StoreIdentity>>>,
    store_available: AtomicBool,
}

fn normalize_indexed_path(path: &str) -> String {
    let mut normalized = refact_git_intel::paths::normalize_separators(path);
    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped.to_string();
    }
    normalized
}

fn matches_indexed_path(requested: &str, indexed: &str) -> bool {
    requested == indexed
        || matches_repo_relative_suffix(requested, indexed)
        || matches_repo_relative_suffix(indexed, requested)
}

fn matches_repo_relative_suffix(path: &str, suffix: &str) -> bool {
    let Some(root) = root_for_suffix_match(path, suffix) else {
        return false;
    };
    let repo_root = if root.is_empty() && path.starts_with('/') {
        "/"
    } else {
        root
    };
    refact_git_intel::paths::repo_relative(path, repo_root)
        .is_some_and(|relative| refact_git_intel::paths::normalize_separators(relative) == suffix)
}

fn root_for_suffix_match<'a>(path: &'a str, suffix: &str) -> Option<&'a str> {
    let prefix_len = path.len().checked_sub(suffix.len() + 1)?;
    if path.get(prefix_len..prefix_len + 1) != Some("/") {
        return None;
    }
    if path.get(prefix_len + 1..) != Some(suffix) {
        return None;
    }
    Some(&path[..prefix_len])
}

fn resolve_indexed_paths(requested: &[String], indexed: &[String]) -> Vec<String> {
    requested
        .iter()
        .map(|path| {
            let normalized = normalize_indexed_path(path);
            if let Some(indexed_path) = indexed
                .iter()
                .find(|indexed_path| normalize_indexed_path(indexed_path) == normalized)
            {
                return indexed_path.clone();
            }

            let candidates = indexed
                .iter()
                .filter(|indexed_path| {
                    let indexed_normalized = normalize_indexed_path(indexed_path);
                    matches_indexed_path(&normalized, &indexed_normalized)
                })
                .collect::<Vec<_>>();
            if candidates.len() == 1 {
                candidates[0].clone()
            } else {
                normalized
            }
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

enum AnalyticsRebuildPause {
    None,
    #[cfg(test)]
    Gate(std::sync::Arc<AnalyticsRebuildGate>),
}

enum ScopedAnalyticsRebuildPause {
    None,
    #[cfg(test)]
    Gate(Option<std::sync::Arc<AnalyticsRebuildGate>>),
    #[cfg(test)]
    AdvanceGeneration,
}

impl ScopedAnalyticsRebuildPause {
    async fn after_rebuild(&mut self, _service: &CodeGraphService) {
        match self {
            ScopedAnalyticsRebuildPause::None => {}
            #[cfg(test)]
            ScopedAnalyticsRebuildPause::Gate(gate) => {
                if let Some(gate) = gate.take() {
                    gate.pause().await;
                }
            }
            #[cfg(test)]
            ScopedAnalyticsRebuildPause::AdvanceGeneration => _service.bump_graph_generation(),
        }
    }
}

impl AnalyticsRebuildPause {
    async fn wait(self) {
        match self {
            AnalyticsRebuildPause::None => {}
            #[cfg(test)]
            AnalyticsRebuildPause::Gate(gate) => gate.pause().await,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedPath {
    pub store_path: String,
    pub read_path: String,
}

impl QueuedPath {
    pub fn new(store_path: String, read_path: String) -> Self {
        Self {
            store_path,
            read_path,
        }
    }
}

#[cfg(test)]
struct AnalyticsRebuildGate {
    after_rebuild: tokio::sync::Barrier,
    resume: tokio::sync::Semaphore,
}

#[cfg(test)]
impl AnalyticsRebuildGate {
    fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            after_rebuild: tokio::sync::Barrier::new(2),
            resume: tokio::sync::Semaphore::new(0),
        })
    }

    async fn pause(&self) {
        self.after_rebuild.wait().await;
        self.resume
            .acquire()
            .await
            .expect("gate remains open")
            .forget();
    }

    async fn wait_paused(&self) {
        self.after_rebuild.wait().await;
    }

    fn resume(&self) {
        self.resume.add_permits(1);
    }
}
#[derive(Default)]
struct PendingQueue {
    order: VecDeque<String>,
    entries: HashMap<String, QueuedPath>,
}

const THROUGHPUT_WINDOW: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
struct ThroughputWindow {
    completions: VecDeque<(Instant, usize)>,
}

impl ThroughputWindow {
    fn record(&mut self, now: Instant, count: usize) {
        if count == 0 {
            return;
        }
        self.completions.push_back((now, count));
        self.prune(now);
    }

    fn files_per_min(&mut self, now: Instant) -> f64 {
        self.prune(now);
        let count = self
            .completions
            .iter()
            .map(|(_, count)| *count)
            .sum::<usize>();
        if count == 0 {
            0.0
        } else {
            count as f64 / (THROUGHPUT_WINDOW.as_secs_f64() / 60.0)
        }
    }

    fn prune(&mut self, now: Instant) {
        while self
            .completions
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > THROUGHPUT_WINDOW)
        {
            self.completions.pop_front();
        }
    }
}

impl CodeGraphService {
    pub fn open(db_path: PathBuf) -> Result<Self, String> {
        let store = Store::open(&db_path)?;
        let read_store = Store::open_readonly(&db_path)?;
        let identity = store.identity_handle();
        Ok(Self {
            store: AMutex::new(Some(store)),
            read_store: Some(AMutex::new(Some(read_store))),
            queue: StdMutex::new(PendingQueue::default()),
            throughput: StdMutex::new(ThroughputWindow::default()),
            queue_notify: Notify::new(),
            db_path,
            initial_index_done: AtomicBool::new(false),
            graph_generation: AtomicU64::new(0),
            analytics_cache: AMutex::new(None),
            scoped_analytics_cache: AMutex::new(ScopedAnalyticsCache {
                entries: VecDeque::new(),
            }),
            analytics_rebuild_count: AtomicUsize::new(0),
            scoped_analytics_rebuild_count: AtomicUsize::new(0),
            run_parse_failures: AtomicUsize::new(0),
            store_generation: AtomicU64::new(0),
            reopen_attempts: AtomicUsize::new(0),
            last_reopen_attempt: StdMutex::new(None),
            writer_identity: StdMutex::new(Some(identity)),
            store_available: AtomicBool::new(true),
        })
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let store = Store::open_in_memory()?;
        Ok(Self {
            store: AMutex::new(Some(store)),
            read_store: None,
            queue: StdMutex::new(PendingQueue::default()),
            throughput: StdMutex::new(ThroughputWindow::default()),
            queue_notify: Notify::new(),
            db_path: PathBuf::from(":memory:"),
            initial_index_done: AtomicBool::new(true),
            graph_generation: AtomicU64::new(0),
            analytics_cache: AMutex::new(None),
            scoped_analytics_cache: AMutex::new(ScopedAnalyticsCache {
                entries: VecDeque::new(),
            }),
            analytics_rebuild_count: AtomicUsize::new(0),
            scoped_analytics_rebuild_count: AtomicUsize::new(0),
            run_parse_failures: AtomicUsize::new(0),
            store_generation: AtomicU64::new(0),
            reopen_attempts: AtomicUsize::new(0),
            last_reopen_attempt: StdMutex::new(None),
            writer_identity: StdMutex::new(None),
            store_available: AtomicBool::new(true),
        })
    }

    pub fn mark_initial_index_done(&self) {
        self.initial_index_done.store(true, Ordering::Relaxed);
    }

    pub fn is_initial_index_done(&self) -> bool {
        self.initial_index_done.load(Ordering::Relaxed)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn enqueue_files(&self, paths: &[String]) {
        if paths.is_empty() {
            return;
        }
        let entries = paths
            .iter()
            .map(|path| QueuedPath::new(path.clone(), path.clone()))
            .collect::<Vec<_>>();
        self.enqueue_paths_with_read_paths(&entries);
    }

    pub fn enqueue_paths_with_read_paths(&self, paths: &[QueuedPath]) {
        if paths.is_empty() {
            return;
        }
        let mut queue = self.queue.lock().unwrap();
        let mut added = false;
        for path in paths {
            if queue.entries.contains_key(&path.store_path) {
                queue.entries.insert(path.store_path.clone(), path.clone());
            } else {
                queue.order.push_back(path.store_path.clone());
                queue.entries.insert(path.store_path.clone(), path.clone());
                added = true;
            }
        }
        drop(queue);
        if added {
            self.queue_notify.notify_one();
        }
    }

    pub fn drain_batch(&self, max: usize) -> Vec<String> {
        self.drain_batch_entries(max)
            .into_iter()
            .map(|path| path.store_path)
            .collect()
    }

    pub fn drain_batch_entries(&self, max: usize) -> Vec<QueuedPath> {
        let mut queue = self.queue.lock().unwrap();
        let take = max.min(queue.order.len());
        let mut drained = Vec::with_capacity(take);
        for _ in 0..take {
            if let Some(path) = queue.order.pop_front() {
                if let Some(entry) = queue.entries.remove(&path) {
                    drained.push(entry);
                }
            }
        }
        drained
    }

    pub fn queue_len(&self) -> usize {
        self.queue.lock().unwrap().entries.len()
    }

    pub fn record_index_completions(&self, count: usize) {
        self.throughput
            .lock()
            .unwrap()
            .record(Instant::now(), count);
    }

    pub fn throughput_files_per_min(&self) -> f64 {
        self.throughput
            .lock()
            .unwrap()
            .files_per_min(Instant::now())
    }

    pub async fn wait_for_enqueue(&self) {
        self.queue_notify.notified().await;
    }

    async fn with_read_store<T>(
        &self,
        f: impl FnOnce(&Store) -> Result<T, String>,
    ) -> Result<T, String> {
        if let Some(store) = &self.read_store {
            let store = store.lock().await;
            f(store.as_ref().ok_or_else(unavailable_error)?)
        } else {
            let store = self.store.lock().await;
            f(store.as_ref().ok_or_else(unavailable_error)?)
        }
    }

    async fn with_write_store<T>(
        &self,
        f: impl FnOnce(&Store) -> Result<T, String>,
    ) -> Result<T, String> {
        let store = self.store.lock().await;
        f(store.as_ref().ok_or_else(unavailable_error)?)
    }

    /// Generation of the underlying sqlite files. It changes whenever the connections are
    /// reopened after another process replaced the database, so results computed against the
    /// previous files can be discarded instead of written back.
    pub fn store_generation(&self) -> u64 {
        self.store_generation.load(Ordering::Acquire)
    }

    pub fn reopen_attempts(&self) -> usize {
        self.reopen_attempts.load(Ordering::Relaxed)
    }

    /// True when the writer connection no longer points at the file living at `db_path`.
    ///
    /// This runs on the hot loop before every queue drain, so it deliberately reads the retained
    /// descriptor instead of locking the writer connection the indexer holds across batches.
    pub fn database_replaced(&self) -> bool {
        match self.writer_identity.lock().unwrap().as_ref() {
            Some(identity) => identity.replaced(),
            None => !self.store_available.load(Ordering::Acquire),
        }
    }

    pub fn is_available(&self) -> bool {
        self.store_available.load(Ordering::Acquire)
    }

    /// Reopen both connections on the file that now lives at `db_path`, dropping every cache
    /// derived from the old files. The pending queue is deliberately preserved: those paths still
    /// need indexing, and the fresh database has none of their content.
    pub async fn recover_replaced_database(&self) -> DatabaseRecovery {
        if self.read_store.is_none() {
            return DatabaseRecovery::NotNeeded;
        }
        if !self.database_replaced() && self.is_available() {
            return DatabaseRecovery::NotNeeded;
        }
        {
            let mut last = self.last_reopen_attempt.lock().unwrap();
            let now = Instant::now();
            if let Some(previous) = *last {
                if now.saturating_duration_since(previous) < REOPEN_RETRY_INTERVAL {
                    return DatabaseRecovery::Deferred;
                }
            }
            *last = Some(now);
        }
        self.reopen_attempts.fetch_add(1, Ordering::Relaxed);

        let mut writer = self.store.lock().await;
        let Some(read_slot) = self.read_store.as_ref() else {
            return DatabaseRecovery::NotNeeded;
        };
        let mut reader = read_slot.lock().await;
        *writer = None;
        *reader = None;

        let db_path = self.db_path.clone();
        let reopened = Store::open(&db_path).and_then(|store| {
            let read_store = Store::open_readonly(&db_path)?;
            Ok((store, read_store))
        });
        match reopened {
            Ok((store, read_store)) => {
                *self.writer_identity.lock().unwrap() = Some(store.identity_handle());
                *writer = Some(store);
                *reader = Some(read_store);
                drop(reader);
                drop(writer);
                self.store_available.store(true, Ordering::Release);
                self.store_generation.fetch_add(1, Ordering::AcqRel);
                self.bump_graph_generation();
                self.reset_caches_after_reopen().await;
                debug!("codegraph: reopened both connections after the database was replaced");
                DatabaseRecovery::Recovered
            }
            Err(err) => {
                *self.writer_identity.lock().unwrap() = None;
                drop(reader);
                drop(writer);
                self.store_available.store(false, Ordering::Release);
                self.store_generation.fetch_add(1, Ordering::AcqRel);
                DatabaseRecovery::Failed(format!("{DB_UNAVAILABLE_ERROR}: {err}"))
            }
        }
    }

    async fn reset_caches_after_reopen(&self) {
        *self.analytics_cache.lock().await = None;
        self.scoped_analytics_cache.lock().await.entries.clear();
        *self.throughput.lock().unwrap() = ThroughputWindow::default();
        self.run_parse_failures.store(0, Ordering::Relaxed);
        self.initial_index_done.store(false, Ordering::Relaxed);
    }

    pub fn graph_generation(&self) -> u64 {
        self.graph_generation.load(Ordering::Acquire)
    }

    fn bump_graph_generation(&self) {
        self.graph_generation.fetch_add(1, Ordering::AcqRel);
    }

    #[cfg(test)]
    pub fn analytics_rebuild_count(&self) -> usize {
        self.analytics_rebuild_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn scoped_analytics_rebuild_count(&self) -> usize {
        self.scoped_analytics_rebuild_count.load(Ordering::Relaxed)
    }

    pub async fn index_file(
        &self,
        path: &str,
        text: &str,
        lang: &str,
    ) -> Result<IndexedFile, String> {
        let indexed = self
            .with_write_store(|store| store.index_file_graph(path, text, lang))
            .await?;
        self.record_run_parse_failures(std::slice::from_ref(&indexed));
        if indexed.changed {
            self.bump_graph_generation();
        }
        Ok(indexed)
    }

    pub async fn index_files_batch(
        &self,
        entries: &[(String, String, String)],
    ) -> Result<Vec<IndexedFile>, String> {
        let results = self
            .with_write_store(|store| store.index_files_batch(entries))
            .await?;
        let changed = results.iter().any(|indexed| indexed.changed);
        self.record_run_parse_failures(&results);
        if changed {
            self.bump_graph_generation();
        }
        Ok(results)
    }

    fn checkpoint_wal_failure_tolerant(store: &Store, mode: WalCheckpointMode) {
        match store.checkpoint_wal(mode) {
            Ok(result) if result.busy != 0 => {
                debug!(
                    ?mode,
                    ?result,
                    "codegraph WAL checkpoint blocked; will retry later"
                );
            }
            Ok(result) => {
                debug!(?mode, ?result, "codegraph WAL checkpoint complete");
            }
            Err(err) => {
                debug!(?mode, %err, "codegraph WAL checkpoint failed; will retry later");
            }
        }
    }

    /// Checkpoint the WAL without allowing a busy reader or SQLite error to disrupt indexing.
    /// Use `Passive` after routine batches and `Truncate` only at a genuine idle boundary.
    pub async fn checkpoint_wal(&self, mode: WalCheckpointMode) {
        let store = self.store.lock().await;
        let Some(store) = store.as_ref() else {
            return;
        };
        Self::checkpoint_wal_failure_tolerant(store, mode);
    }

    pub async fn remove_path(&self, path: &str) -> Result<(), String> {
        let changed = self
            .with_write_store(|store| store.remove_path(path))
            .await?;
        if changed {
            self.bump_graph_generation();
        }
        Ok(())
    }

    pub async fn counts(&self) -> Result<Counts, String> {
        self.with_read_store(|store| store.counts()).await
    }

    pub async fn connect_usages(&self) -> Result<(), String> {
        let changed = self
            .with_write_store(|store| store.connect_usages())
            .await?;
        if changed {
            self.bump_graph_generation();
        }
        Ok(())
    }

    pub async fn has_dirty_usage_paths(&self) -> Result<bool, String> {
        self.with_read_store(|store| store.has_dirty_paths()).await
    }

    pub async fn index_readiness(&self) -> Result<IndexReadiness, String> {
        let queued = self.queue_len();
        let (dirty_paths, pending_refs, cross_file_edges, parse_failures, parse_failure_paths) =
            self.with_read_store(|store| {
                Ok((
                    store.dirty_path_count()? as i64,
                    store.pending_ref_count()?,
                    store.cross_file_edge_count()?,
                    store.parse_failure_count()?,
                    store
                        .parse_failures(PARSE_FAILURE_REPORT_LIMIT)?
                        .into_iter()
                        .map(|failure| failure.path)
                        .collect::<Vec<_>>(),
                ))
            })
            .await?;
        Ok(IndexReadiness {
            queued,
            dirty_paths,
            pending_refs,
            cross_file_edges,
            cross_file_ready: queued == 0 && dirty_paths == 0,
            parse_failures,
            parse_failure_paths,
        })
    }

    pub async fn parse_failure_count(&self) -> Result<i64, String> {
        self.with_read_store(|store| store.parse_failure_count())
            .await
    }

    pub async fn parse_failures(&self, limit: i64) -> Result<Vec<ParseFailure>, String> {
        self.with_read_store(|store| store.parse_failures(limit))
            .await
    }

    pub fn run_parse_failures(&self) -> usize {
        self.run_parse_failures.load(Ordering::Relaxed)
    }

    pub fn reset_run_parse_failures(&self) {
        self.run_parse_failures.store(0, Ordering::Relaxed);
    }

    fn record_run_parse_failures(&self, indexed: &[IndexedFile]) {
        let failures = indexed
            .iter()
            .filter(|entry| entry.parse_failure.is_some())
            .count();
        if failures > 0 {
            self.run_parse_failures
                .fetch_add(failures, Ordering::Relaxed);
        }
    }

    pub async fn meta_get(&self, key: &str) -> Result<Option<String>, String> {
        self.with_read_store(|store| store.meta_get(key)).await
    }

    pub async fn meta_set(&self, key: &str, value: &str) -> Result<(), String> {
        self.with_write_store(|store| store.meta_set(key, value))
            .await
    }

    pub async fn dirty_usage_path_count(&self) -> Result<usize, String> {
        self.with_read_store(|store| store.dirty_path_count()).await
    }

    pub async fn doc_usages(&self, cpath: &str) -> Result<Vec<(usize, String)>, String> {
        self.with_read_store(|store| store.doc_usages(cpath)).await
    }

    pub async fn cached_graph_analytics(&self) -> Result<Arc<CachedGraphAnalytics>, String> {
        self.cached_graph_analytics_with_pause(AnalyticsRebuildPause::None)
            .await
    }

    #[cfg(test)]
    async fn cached_graph_analytics_with_gate(
        &self,
        gate: std::sync::Arc<AnalyticsRebuildGate>,
    ) -> Result<Arc<CachedGraphAnalytics>, String> {
        self.cached_graph_analytics_with_pause(AnalyticsRebuildPause::Gate(gate))
            .await
    }

    async fn cached_graph_analytics_with_pause(
        &self,
        pause: AnalyticsRebuildPause,
    ) -> Result<Arc<CachedGraphAnalytics>, String> {
        let mut pause = Some(pause);
        loop {
            {
                let cache = self.analytics_cache.lock().await;
                let generation = self.graph_generation.load(Ordering::Acquire);
                if let Some(cached) = cache
                    .as_ref()
                    .filter(|cached| cached.generation == generation)
                {
                    return Ok(cached.clone());
                }
            }

            let generation = self.graph_generation.load(Ordering::Acquire);
            let rebuilt = self.rebuild_graph_analytics(generation).await?;
            self.analytics_rebuild_count.fetch_add(1, Ordering::Relaxed);
            if let Some(pause) = pause.take() {
                pause.wait().await;
            }

            let mut cache = self.analytics_cache.lock().await;
            let current_generation = self.graph_generation.load(Ordering::Acquire);
            if let Some(cached) = cache
                .as_ref()
                .filter(|cached| cached.generation == current_generation)
            {
                return Ok(cached.clone());
            }
            if current_generation == rebuilt.generation {
                *cache = Some(rebuilt.clone());
                return Ok(rebuilt);
            }
        }
    }

    async fn rebuild_graph_analytics(
        &self,
        generation: u64,
    ) -> Result<Arc<CachedGraphAnalytics>, String> {
        let (counts, data, symbols, dcp_pairs) = self
            .with_read_store(|store| {
                store.read_snapshot(|store| {
                    Ok((
                        store.counts()?,
                        analytics::GraphData::from_store(store)?,
                        store.symbol_records()?,
                        store.all_symbols()?,
                    ))
                })
            })
            .await?;
        let compute = move || {
            let analytics = analytics::compute_graph_analytics_from_data(&data);
            let communities = communities::detect_communities_from_data(&data)?;
            let dead_code = dead_code::dead_code_from_parts(symbols, dcp_pairs, &data.edges);
            Ok(Arc::new(CachedGraphAnalytics {
                generation,
                counts,
                data,
                analytics,
                communities,
                dead_code,
            }))
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle
                .spawn_blocking(compute)
                .await
                .map_err(|e| format!("codegraph analytics rebuild join: {e}"))?,
            Err(_) => compute(),
        }
    }

    pub async fn overview(&self, top_n: usize) -> Result<analytics::GraphOverview, String> {
        Ok(self
            .cached_graph_analytics()
            .await?
            .analytics
            .overview
            .truncated(top_n))
    }

    pub async fn all_files_with_text(&self) -> Result<Vec<(String, String)>, String> {
        self.with_read_store(|store| store.all_files_with_text())
            .await
    }

    pub async fn all_paths(&self) -> Result<Vec<String>, String> {
        self.with_read_store(|store| store.all_paths()).await
    }

    async fn filtered_graph_data(
        &self,
        allowed_paths: &HashSet<String>,
    ) -> Result<analytics::GraphData, String> {
        self.with_read_store(|store| {
            store.read_snapshot(|store| Self::filtered_graph_data_from_store(store, allowed_paths))
        })
        .await
    }

    fn filtered_graph_data_from_store(
        store: &Store,
        allowed_paths: &HashSet<String>,
    ) -> Result<analytics::GraphData, String> {
        let nodes = store
            .node_names()?
            .into_iter()
            .filter(|(_, _, path)| allowed_paths.contains(path))
            .collect::<Vec<_>>();
        let ids = nodes.iter().map(|(id, _, _)| *id).collect::<HashSet<_>>();
        let edges = store
            .graph_edges()?
            .into_iter()
            .filter(|(src, dst, _)| ids.contains(src) && ids.contains(dst))
            .collect();
        Ok(analytics::GraphData { nodes, edges })
    }

    pub async fn graph_nodes(&self) -> Result<Vec<analytics::GraphNode>, String> {
        Ok(self.cached_graph_analytics().await?.data.nodes.clone())
    }

    pub async fn graph_node_records(
        &self,
    ) -> Result<Vec<(i64, String, String, String, Option<String>)>, String> {
        self.with_read_store(|store| store.node_records()).await
    }

    pub async fn graph_edges(&self) -> Result<Vec<analytics::GraphEdge>, String> {
        Ok(self.cached_graph_analytics().await?.data.edges.clone())
    }

    pub async fn per_file_centrality(
        &self,
        top_n: usize,
    ) -> Result<crate::analytics::FileCentrality, String> {
        Ok(self
            .cached_graph_analytics()
            .await?
            .analytics
            .file_centrality
            .truncated(top_n))
    }

    pub async fn communities(&self) -> Result<Vec<communities::Community>, String> {
        Ok(self.cached_graph_analytics().await?.communities.clone())
    }

    pub async fn execution_flows(
        &self,
        max_flows: usize,
    ) -> Result<Vec<communities::ExecFlow>, String> {
        let cached = self.cached_graph_analytics().await?;
        communities::execution_flows_from_data(&cached.data, max_flows)
    }

    pub async fn dead_code(&self) -> Result<dead_code::DeadCodeResult, String> {
        Ok(self.cached_graph_analytics().await?.dead_code.clone())
    }

    pub async fn scoped_graph_analytics(
        &self,
        allowed_paths: &HashSet<String>,
    ) -> Result<Arc<CachedGraphAnalytics>, String> {
        self.scoped_graph_analytics_with_pause(allowed_paths, ScopedAnalyticsRebuildPause::None)
            .await
    }

    #[cfg(test)]
    async fn scoped_graph_analytics_with_gate(
        &self,
        allowed_paths: &HashSet<String>,
        gate: std::sync::Arc<AnalyticsRebuildGate>,
    ) -> Result<Arc<CachedGraphAnalytics>, String> {
        self.scoped_graph_analytics_with_pause(
            allowed_paths,
            ScopedAnalyticsRebuildPause::Gate(Some(gate)),
        )
        .await
    }

    async fn scoped_graph_analytics_with_pause(
        &self,
        allowed_paths: &HashSet<String>,
        mut pause: ScopedAnalyticsRebuildPause,
    ) -> Result<Arc<CachedGraphAnalytics>, String> {
        let allowed_paths = allowed_paths.clone();
        let mut stable_paths = allowed_paths.iter().cloned().collect::<Vec<_>>();
        stable_paths.sort();
        for attempt in 0..SCOPED_ANALYTICS_MAX_REBUILD_ATTEMPTS {
            let (generation, key) = {
                let mut cache = self.scoped_analytics_cache.lock().await;
                let generation = self.graph_generation();
                let key = ScopedAnalyticsKey {
                    generation,
                    allowed_paths: stable_paths.clone(),
                };
                cache
                    .entries
                    .retain(|entry| entry.key.generation == generation);
                if let Some(position) = cache.entries.iter().position(|entry| entry.key == key) {
                    let cached = cache.entries.remove(position).expect("cache entry exists");
                    let value = cached.value.clone();
                    cache.entries.push_back(cached);
                    return Ok(value);
                }
                (generation, key)
            };

            let rebuilt = self
                .rebuild_scoped_graph_analytics(generation, &allowed_paths)
                .await?;
            pause.after_rebuild(self).await;

            let mut cache = self.scoped_analytics_cache.lock().await;
            let current_generation = self.graph_generation();
            cache
                .entries
                .retain(|entry| entry.key.generation == current_generation);
            if let Some(cached) = cache.entries.iter().find(|entry| {
                entry.key.generation == current_generation
                    && entry.key.allowed_paths == stable_paths
            }) {
                return Ok(cached.value.clone());
            }
            if current_generation == generation {
                cache.entries.push_back(ScopedAnalyticsCacheEntry {
                    key,
                    value: rebuilt.clone(),
                });
                while cache.entries.len() > SCOPED_ANALYTICS_CACHE_CAPACITY {
                    cache.entries.pop_front();
                }
                return Ok(rebuilt);
            }
            drop(cache);
            if attempt + 1 == SCOPED_ANALYTICS_MAX_REBUILD_ATTEMPTS {
                return Err(format!(
                    "codegraph generation changed while rebuilding scoped analytics (rebuilt {}, current {})",
                    rebuilt.generation,
                    self.graph_generation()
                ));
            }
        }
        unreachable!("scoped analytics rebuild attempts are non-zero")
    }

    async fn rebuild_scoped_graph_analytics(
        &self,
        generation: u64,
        allowed_paths: &HashSet<String>,
    ) -> Result<Arc<CachedGraphAnalytics>, String> {
        let (data, symbols, dcp_pairs, fts_docs) = self
            .with_read_store(|store| {
                store.read_snapshot(|store| {
                    let data = Self::filtered_graph_data_from_store(store, allowed_paths)?;
                    let ids = data
                        .nodes
                        .iter()
                        .map(|(id, _, _)| *id)
                        .collect::<HashSet<_>>();
                    let symbols = store
                        .symbol_records()?
                        .into_iter()
                        .filter(|symbol| ids.contains(&symbol.node_id))
                        .collect();
                    let dcp_pairs = store
                        .all_symbols()?
                        .into_iter()
                        .filter(|(_, id)| ids.contains(id))
                        .collect();
                    let fts_docs = store
                        .all_paths()?
                        .into_iter()
                        .filter(|path| allowed_paths.contains(path))
                        .count() as i64;
                    Ok((data, symbols, dcp_pairs, fts_docs))
                })
            })
            .await?;
        self.scoped_analytics_rebuild_count
            .fetch_add(1, Ordering::Relaxed);
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let files = data
                .nodes
                .iter()
                .map(|(_, _, path)| path)
                .collect::<HashSet<_>>()
                .len();
            let counts = Counts {
                nodes: data.nodes.len() as i64,
                edges: data.edges.len() as i64,
                files: files as i64,
                fts_docs,
            };
            let analytics = analytics::compute_graph_analytics_from_data(&data);
            let communities = communities::detect_communities_from_data(&data)?;
            let dead_code = dead_code::dead_code_from_parts(symbols, dcp_pairs, &data.edges);
            Ok(Arc::new(CachedGraphAnalytics {
                generation,
                counts,
                data,
                analytics,
                communities,
                dead_code,
            }))
        })
        .await
        .map_err(|e| format!("scoped codegraph analytics join: {e}"))?
    }

    pub async fn security_scan(
        &self,
        _path: &str,
        lang: &str,
        text: &str,
    ) -> Result<Vec<security_scan::SecurityFinding>, String> {
        Ok(security_scan::scan(lang, text))
    }

    pub async fn pr_blast(
        &self,
        changed_files: &[String],
        max_depth: usize,
    ) -> Result<pr_blast::BlastReport, String> {
        if changed_files.is_empty() || max_depth == 0 {
            return Ok(pr_blast::empty_report(changed_files));
        }
        self.with_read_store(|store| {
            let indexed_paths = store.all_paths()?;
            let changed_files = resolve_indexed_paths(changed_files, &indexed_paths);
            pr_blast::blast_radius(store, &changed_files, max_depth)
        })
        .await
    }

    pub async fn pr_blast_scoped(
        &self,
        changed_files: &[String],
        max_depth: usize,
        allowed_paths: &HashSet<String>,
    ) -> Result<pr_blast::BlastReport, String> {
        if changed_files.is_empty() || max_depth == 0 {
            return Ok(pr_blast::empty_report(changed_files));
        }
        let mut indexed_paths = allowed_paths.iter().cloned().collect::<Vec<_>>();
        indexed_paths.sort();
        let changed_files = resolve_indexed_paths(changed_files, &indexed_paths);
        let data = self.filtered_graph_data(allowed_paths).await?;
        Ok(pr_blast::blast_radius_from_data(
            &data,
            &changed_files,
            max_depth,
        ))
    }

    pub async fn type_hierarchy(&self, subtree_of: &str) -> Result<String, String> {
        self.with_read_store(|store| facade::type_hierarchy(store, subtree_of))
            .await
    }

    pub async fn search_hybrid(&self, query: &str, limit: usize) -> Result<Vec<CodeHit>, String> {
        self.with_read_store(|store| retrieval::search_hybrid(store, query, limit))
            .await
    }

    pub async fn doc_defs(
        &self,
        cpath: &str,
    ) -> Result<Vec<std::sync::Arc<refact_core::ast_types::AstDefinition>>, String> {
        self.with_read_store(|store| facade::doc_defs(store, cpath))
            .await
    }

    pub async fn definitions(
        &self,
        double_colon_path: &str,
    ) -> Result<Vec<std::sync::Arc<refact_core::ast_types::AstDefinition>>, String> {
        self.with_read_store(|store| facade::definitions(store, double_colon_path))
            .await
    }

    pub async fn definition_paths_fuzzy(
        &self,
        pattern: &str,
        top_n: usize,
    ) -> Result<Vec<String>, String> {
        self.with_read_store(|store| facade::definition_paths_fuzzy(store, pattern, top_n))
            .await
    }

    pub async fn fetch_counters(&self) -> Result<refact_core::ast_types::AstCounters, String> {
        self.with_read_store(facade::fetch_counters).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    const BENCH_INDEX_CORPUS: &[(&str, &str)] = &[
        ("../gui/src/__tests__/buddy.test.ts", "typescript"),
        ("../gui/src/__tests__/buddy_home.test.tsx", "typescript"),
        (
            "../gui/src/features/Tasks/TaskWorkspace.test.tsx",
            "typescript",
        ),
        ("../gui/src/features/Chat/Thread/reducer.ts", "typescript"),
        ("../gui/src/features/Tasks/TaskWorkspace.tsx", "typescript"),
        ("../gui/src/features/Buddy/canvas/animLoop.ts", "typescript"),
        ("../gui/src/app/middleware.ts", "typescript"),
        ("../gui/src/features/Buddy/BuddyWorld.tsx", "typescript"),
        (
            "../gui/src/components/ChatContent/ToolsContent.tsx",
            "typescript",
        ),
        ("../gui/src/__tests__/buddy_showcase.test.ts", "typescript"),
        ("../gui/src/services/refact/types.ts", "typescript"),
        ("../gui/src/services/refact/providers.ts", "typescript"),
        (
            "../gui/src/features/Chat/Thread/reducer.test.ts",
            "typescript",
        ),
        (
            "../gui/src/__tests__/buddy_world_draw.test.ts",
            "typescript",
        ),
        (
            "../gui/src/features/Buddy/BuddyChatCompanion.tsx",
            "typescript",
        ),
        (
            "../gui/src/features/Buddy/buddyWorldDrawAtmosphere.ts",
            "typescript",
        ),
        ("../gui/src/features/Buddy/buddyWorldModel.ts", "typescript"),
        ("../gui/src/__tests__/chatSSEProtocol.test.ts", "typescript"),
        (
            "../gui/src/features/Buddy/buddyWorldDrawActor.ts",
            "typescript",
        ),
        ("../gui/src/features/Chat/Thread/selectors.ts", "typescript"),
        ("src/chat/trajectories.rs", "rust"),
        ("crates/refact-tui/src/app.rs", "rust"),
        ("src/buddy/tests.rs", "rust"),
        ("src/chat/summarization.rs", "rust"),
        ("src/chat/session.rs", "rust"),
        ("src/buddy/jobs/autonomous_chats.rs", "rust"),
        ("crates/refact-worktrees/src/service.rs", "rust"),
        ("crates/refact-llm/src/adapters/anthropic.rs", "rust"),
        ("src/providers/http.rs", "rust"),
        ("src/chat/queue.rs", "rust"),
        ("src/chat/generation.rs", "rust"),
        ("src/buddy/memory_lifecycle.rs", "rust"),
        ("src/chat/stream_core.rs", "rust"),
        ("crates/refact-exec/src/registry.rs", "rust"),
        ("src/files_in_workspace.rs", "rust"),
        ("src/scheduler/runner.rs", "rust"),
        ("src/subchat.rs", "rust"),
        ("crates/refact-chat-history/src/trajectory_ops.rs", "rust"),
        ("src/tools/tool_task_memory.rs", "rust"),
        ("src/tools/tool_process.rs", "rust"),
        ("src/tools/tool_task_merge_agent.rs", "rust"),
        ("crates/refact-agentic/src/mode_transition.rs", "rust"),
        ("src/buddy/actor.rs", "rust"),
        ("crates/refact-llm/src/adapters/openai_responses.rs", "rust"),
        ("src/chat/task_agent_monitor.rs", "rust"),
        ("src/daemon/cli.rs", "rust"),
        ("src/http/routers/v1/tasks.rs", "rust"),
        ("src/tools/tool_compress_chat.rs", "rust"),
        ("src/memories.rs", "rust"),
        ("src/http/routers/v1/mcp_marketplace.rs", "rust"),
    ];

    #[tokio::test]
    #[ignore = "manual dev-profile indexing benchmark"]
    async fn bench_index() {
        let engine_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let service = CodeGraphService::open_in_memory().unwrap();
        let mut bytes = 0usize;
        let started = Instant::now();

        for (rel_path, lang) in BENCH_INDEX_CORPUS {
            let full_path = engine_root.join(rel_path);
            let text = std::fs::read_to_string(&full_path)
                .unwrap_or_else(|err| panic!("read {}: {err}", full_path.display()));
            bytes += text.len();
            service.index_file(rel_path, &text, lang).await.unwrap();
        }

        let elapsed = started.elapsed();
        let counts = service.counts().await.unwrap();
        assert_eq!(counts.files, BENCH_INDEX_CORPUS.len() as i64);
        println!(
            "bench_index files={} bytes={} nodes={} edges={} fts_docs={} elapsed_ms={:.3} ms_per_file={:.3}",
            BENCH_INDEX_CORPUS.len(),
            bytes,
            counts.nodes,
            counts.edges,
            counts.fts_docs,
            elapsed.as_secs_f64() * 1000.0,
            elapsed.as_secs_f64() * 1000.0 / BENCH_INDEX_CORPUS.len() as f64
        );
    }

    #[test]
    fn enqueue_files_deduplicates_pending_paths_fifo() {
        let service = CodeGraphService::open_in_memory().unwrap();

        service.enqueue_files(&[
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
            "src/a.rs".to_string(),
        ]);
        service.enqueue_files(&["src/b.rs".to_string(), "src/c.rs".to_string()]);

        assert_eq!(service.queue_len(), 3);
        assert_eq!(
            service.drain_batch(10),
            vec![
                "src/a.rs".to_string(),
                "src/b.rs".to_string(),
                "src/c.rs".to_string()
            ]
        );

        service.enqueue_files(&["src/a.rs".to_string()]);
        assert_eq!(service.drain_batch(10), vec!["src/a.rs".to_string()]);
    }

    #[test]
    fn enqueue_files_requeues_drained_paths_but_not_pending_paths() {
        let service = CodeGraphService::open_in_memory().unwrap();

        service.enqueue_files(&["src/a.rs".to_string(), "src/b.rs".to_string()]);
        assert_eq!(service.drain_batch(1), vec!["src/a.rs".to_string()]);

        service.enqueue_files(&["src/a.rs".to_string(), "src/b.rs".to_string()]);

        assert_eq!(service.queue_len(), 2);
        assert_eq!(
            service.drain_batch(10),
            vec!["src/b.rs".to_string(), "src/a.rs".to_string()]
        );
    }

    #[test]
    fn enqueue_paths_updates_pending_read_path_without_reordering() {
        let service = CodeGraphService::open_in_memory().unwrap();

        service.enqueue_paths_with_read_paths(&[
            QueuedPath::new("src/a.rs".to_string(), "worktree/a.rs".to_string()),
            QueuedPath::new("src/b.rs".to_string(), "worktree/b.rs".to_string()),
        ]);
        service.enqueue_paths_with_read_paths(&[QueuedPath::new(
            "src/a.rs".to_string(),
            "worktree/a-new.rs".to_string(),
        )]);

        assert_eq!(
            service.drain_batch_entries(10),
            vec![
                QueuedPath::new("src/a.rs".to_string(), "worktree/a-new.rs".to_string()),
                QueuedPath::new("src/b.rs".to_string(), "worktree/b.rs".to_string()),
            ]
        );
    }

    #[test]
    fn enqueue_files_bulk_dedupes_without_quadratic_scan() {
        let service = CodeGraphService::open_in_memory().unwrap();
        let mut paths = Vec::with_capacity(100_000);
        for idx in 0..50_000 {
            paths.push(format!("src/{idx}.rs"));
        }
        for idx in 0..50_000 {
            paths.push(format!("src/{idx}.rs"));
        }

        let started = Instant::now();
        service.enqueue_files(&paths);
        let elapsed = started.elapsed();

        assert_eq!(service.queue_len(), 50_000);
        assert!(
            elapsed < Duration::from_secs(2),
            "bulk enqueue took {:?}",
            elapsed
        );
        let drained = service.drain_batch(usize::MAX);
        assert_eq!(drained.len(), 50_000);
        assert_eq!(drained.first(), Some(&"src/0.rs".to_string()));
        assert_eq!(drained.last(), Some(&"src/49999.rs".to_string()));
    }

    #[tokio::test]
    async fn wait_for_enqueue_wakes_after_new_pending_path() {
        let service = CodeGraphService::open_in_memory().unwrap();
        let notified = service.wait_for_enqueue();

        service.enqueue_files(&["src/a.rs".to_string()]);

        tokio::time::timeout(Duration::from_millis(100), notified)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn overview_served_from_cache_second_call_analytics() {
        let service = CodeGraphService::open_in_memory().unwrap();
        service
            .index_file("src/a.rs", "pub fn helper() {}\n", "rust")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();

        let first = service.overview(10).await.unwrap();
        assert_eq!(service.analytics_rebuild_count(), 1);
        let second = service.overview(10).await.unwrap();
        assert_eq!(service.analytics_rebuild_count(), 1);
        assert_eq!(first, second);

        service
            .index_file("src/b.rs", "fn run() { helper(); }\n", "rust")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();
        let third = service.overview(10).await.unwrap();

        assert_eq!(service.analytics_rebuild_count(), 2);
        assert_ne!(first.node_count, third.node_count);
    }

    #[test]
    fn resolve_indexed_paths_behavior_unchanged() {
        let indexed = vec![
            "/repo/src/core.rs".to_string(),
            "/repo/src/caller.rs".to_string(),
            "/repo/examples/core.rs".to_string(),
        ];

        assert_eq!(
            resolve_indexed_paths(&[" .\\src\\caller.rs ".to_string()], &indexed),
            vec!["/repo/src/caller.rs".to_string()]
        );
        assert_eq!(
            resolve_indexed_paths(&["src/core.rs".to_string()], &indexed),
            vec!["/repo/src/core.rs".to_string()]
        );
        assert_eq!(
            resolve_indexed_paths(&["core.rs".to_string()], &indexed),
            vec!["core.rs".to_string()]
        );
        assert_eq!(
            resolve_indexed_paths(&["missing.rs".to_string()], &indexed),
            vec!["missing.rs".to_string()]
        );
    }

    #[tokio::test]
    async fn indexer_continues_past_parse_failure_and_reports_the_count() {
        let service = CodeGraphService::open_in_memory().unwrap();
        let entries = vec![
            (
                "src/good.rs".to_string(),
                "pub fn good() {}\n".to_string(),
                "rust".to_string(),
            ),
            (
                "src/broken.rs".to_string(),
                "fn broken( { ; ) } !!!\n".to_string(),
                "rust".to_string(),
            ),
            (
                "src/also_good.rs".to_string(),
                "pub fn also_good() {}\n".to_string(),
                "rust".to_string(),
            ),
        ];

        let results = service.index_files_batch(&entries).await.unwrap();

        assert_eq!(results.len(), 3, "one bad file must not abort the batch");
        assert_eq!(service.run_parse_failures(), 1);
        assert_eq!(service.parse_failure_count().await.unwrap(), 1);
        let readiness = service.index_readiness().await.unwrap();
        assert_eq!(readiness.parse_failures, 1);
        assert_eq!(
            readiness.parse_failure_paths,
            vec!["src/broken.rs".to_string()]
        );
        assert_eq!(service.all_paths().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn empty_file_and_failed_file_are_not_conflated_by_the_service() {
        let service = CodeGraphService::open_in_memory().unwrap();

        let empty = service
            .index_file("src/empty.rs", "// nothing\n", "rust")
            .await
            .unwrap();
        assert!(empty.parse_failure.is_none());
        assert_eq!(service.parse_failure_count().await.unwrap(), 0);

        let broken = service
            .index_file("src/broken.rs", "fn broken( { ; ) } !!!\n", "rust")
            .await
            .unwrap();
        assert!(broken.parse_failure.is_some());
        assert_eq!(service.parse_failure_count().await.unwrap(), 1);
        assert_eq!(service.run_parse_failures(), 1);

        service.reset_run_parse_failures();
        assert_eq!(service.run_parse_failures(), 0);
        assert_eq!(
            service.parse_failure_count().await.unwrap(),
            1,
            "resetting the per-run counter must not erase the persisted record"
        );
    }

    #[tokio::test]
    async fn index_readiness_reports_cross_file_state() {
        let service = CodeGraphService::open_in_memory().unwrap();

        service
            .index_file("src/a.rs", "pub fn helper() {}\n", "rust")
            .await
            .unwrap();
        let dirty = service.index_readiness().await.unwrap();
        assert_eq!(dirty.queued, 0);
        assert!(dirty.dirty_paths > 0);
        assert!(dirty.pending_refs >= 0);
        assert_eq!(dirty.cross_file_edges, 0);
        assert!(!dirty.cross_file_ready);

        service
            .index_file("src/b.rs", "fn run() { helper(); }\n", "rust")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();
        service.enqueue_files(&["src/c.rs".to_string()]);

        let queued = service.index_readiness().await.unwrap();
        assert_eq!(queued.queued, 1);
        assert_eq!(queued.dirty_paths, 0);
        assert!(queued.cross_file_edges > 0);
        assert!(!queued.cross_file_ready);

        service.drain_batch(1);
        let ready = service.index_readiness().await.unwrap();
        assert_eq!(ready.queued, 0);
        assert_eq!(ready.dirty_paths, 0);
        assert!(ready.cross_file_edges > 0);
        assert!(ready.cross_file_ready);
    }

    #[tokio::test]
    async fn meta_kv_roundtrip_and_reserved_key_untouched() {
        let service = CodeGraphService::open_in_memory().unwrap();
        let schema_version = service.meta_get("schema_version").await.unwrap();

        assert_eq!(service.meta_get("user_snapshot").await.unwrap(), None);
        service
            .meta_set("user_snapshot", "generation=1")
            .await
            .unwrap();
        assert_eq!(
            service.meta_get("user_snapshot").await.unwrap(),
            Some("generation=1".to_string())
        );
        service
            .meta_set("user_snapshot", "generation=2")
            .await
            .unwrap();
        assert_eq!(
            service.meta_get("user_snapshot").await.unwrap(),
            Some("generation=2".to_string())
        );

        assert!(service.meta_set("schema_version", "999").await.is_err());
        assert_eq!(
            service.meta_get("schema_version").await.unwrap(),
            schema_version
        );
    }

    #[tokio::test]
    async fn pr_blast_resolves_noncanonical_changed_paths() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let indexed_core = repo.join("src/core.rs").to_string_lossy().to_string();
        let indexed_caller = repo.join("src/caller.rs").to_string_lossy().to_string();
        let service = CodeGraphService::open_in_memory().unwrap();
        service
            .index_file(&indexed_core, "pub fn core() {}\n", "rust")
            .await
            .unwrap();
        service
            .index_file(&indexed_caller, "fn caller() { core(); }\n", "rust")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();

        let report = service
            .pr_blast(&[" .\\src\\core.rs ".to_string()], 2)
            .await
            .unwrap();

        assert_eq!(report.changed_files, vec![indexed_core]);
        assert!(report
            .directly_impacted
            .iter()
            .any(|impact| impact.path == indexed_caller && impact.symbol == "caller"));
    }

    #[tokio::test]
    async fn pr_blast_keeps_unresolvable_changed_paths_empty() {
        let service = CodeGraphService::open_in_memory().unwrap();
        service
            .index_file("src/core.rs", "pub fn core() {}\n", "rust")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();

        let report = service
            .pr_blast(&["missing.rs".to_string()], 2)
            .await
            .unwrap();

        assert_eq!(report.changed_files, vec!["missing.rs".to_string()]);
        assert!(report.directly_impacted.is_empty());
        assert!(report.transitively_impacted.is_empty());
        assert_eq!(report.risk_score, 0.0);
    }

    #[tokio::test]
    async fn pr_blast_early_return_does_not_wait_for_store() {
        let service = CodeGraphService::open_in_memory().unwrap();
        let store_guard = service.store.lock().await;

        let empty = tokio::time::timeout(Duration::from_millis(50), service.pr_blast(&[], 2))
            .await
            .expect("empty blast must not read the store")
            .unwrap();
        let zero_depth = tokio::time::timeout(
            Duration::from_millis(50),
            service.pr_blast(&["src/a.rs".to_string()], 0),
        )
        .await
        .expect("zero-depth blast must not read the store")
        .unwrap();
        let scoped = tokio::time::timeout(
            Duration::from_millis(50),
            service.pr_blast_scoped(
                &["src/a.rs".to_string()],
                0,
                &HashSet::from(["src/a.rs".to_string()]),
            ),
        )
        .await
        .expect("zero-depth scoped blast must not read the store")
        .unwrap();
        drop(store_guard);

        assert!(empty.changed_files.is_empty());
        assert_eq!(zero_depth.changed_files, vec!["src/a.rs".to_string()]);
        assert_eq!(zero_depth.impacted_file_count, 0);
        assert_eq!(scoped.changed_files, vec!["src/a.rs".to_string()]);
    }

    #[tokio::test]
    async fn scoped_pr_blast_resolves_basename_only_within_allowed_paths() {
        let service = CodeGraphService::open_in_memory().unwrap();
        service
            .index_file("src/core.rs", "pub fn scoped_core() {}\n", "rust")
            .await
            .unwrap();
        service
            .index_file(
                "src/caller.rs",
                "fn scoped_caller() { scoped_core(); }\n",
                "rust",
            )
            .await
            .unwrap();
        service
            .index_file("examples/core.rs", "pub fn other_core() {}\n", "rust")
            .await
            .unwrap();
        service.connect_usages().await.unwrap();

        let allowed = HashSet::from(["src/core.rs".to_string(), "src/caller.rs".to_string()]);
        let report = service
            .pr_blast_scoped(&["core.rs".to_string()], 1, &allowed)
            .await
            .unwrap();

        assert_eq!(report.changed_files, vec!["src/core.rs".to_string()]);
        assert!(report
            .directly_impacted
            .iter()
            .any(|impact| impact.path == "src/caller.rs" && impact.symbol == "scoped_caller"));
        assert!(report
            .directly_impacted
            .iter()
            .all(|impact| impact.path != "examples/core.rs"));
    }

    #[tokio::test]
    async fn scoped_analytics_recomputes_dead_code_and_pr_blast() {
        let service = CodeGraphService::open_in_memory().unwrap();
        service
            .index_file("src/core.rs", "fn main() {}\nfn orphan() {}\n", "rust")
            .await
            .unwrap();
        service
            .index_file(
                "dist/generated.rs",
                "fn main() { orphan(); }\nfn generated_caller() { orphan(); }\n",
                "rust",
            )
            .await
            .unwrap();
        service.connect_usages().await.unwrap();
        assert!(!service
            .dead_code()
            .await
            .unwrap()
            .iter()
            .any(|symbol| symbol.name == "orphan"));

        let allowed = HashSet::from(["src/core.rs".to_string()]);
        let scoped = service.scoped_graph_analytics(&allowed).await.unwrap();
        assert!(scoped
            .dead_code
            .iter()
            .any(|symbol| symbol.name == "orphan"));
        assert!(scoped
            .analytics
            .file_centrality
            .top_pagerank
            .iter()
            .all(|(path, _)| path == "src/core.rs"));

        let report = service
            .pr_blast_scoped(&["src/core.rs".to_string()], 2, &allowed)
            .await
            .unwrap();
        assert!(report.directly_impacted.is_empty());
        assert_eq!(report.impacted_file_count, 0);
        assert_eq!(report.risk_score, 0.0);
    }

    #[tokio::test]
    async fn scoped_analytics_cache_reuses_set_and_invalidates_on_generation() {
        let service = CodeGraphService::open_in_memory().unwrap();
        service
            .index_file("src/a.rs", "pub fn a() {}\n", "rust")
            .await
            .unwrap();
        let allowed = HashSet::from(["src/a.rs".to_string()]);

        let first = service.scoped_graph_analytics(&allowed).await.unwrap();
        let second = service.scoped_graph_analytics(&allowed).await.unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(service.scoped_analytics_rebuild_count(), 1);

        service
            .index_file("src/b.rs", "pub fn b() {}\n", "rust")
            .await
            .unwrap();
        let third = service.scoped_graph_analytics(&allowed).await.unwrap();
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!(service.scoped_analytics_rebuild_count(), 2);
        assert_eq!(third.generation, service.graph_generation());
    }

    #[tokio::test]
    async fn scoped_analytics_counts_match_scoped_graph_and_fts_documents() {
        let service = CodeGraphService::open_in_memory().unwrap();
        service
            .index_file("src/a.rs", "pub fn a() {}\n", "rust")
            .await
            .unwrap();
        service
            .index_file("excluded/b.rs", "pub fn b() {}\n", "rust")
            .await
            .unwrap();
        let allowed = HashSet::from(["src/a.rs".to_string()]);

        let scoped = service.scoped_graph_analytics(&allowed).await.unwrap();
        let scoped_files = scoped
            .data
            .nodes
            .iter()
            .map(|(_, _, path)| path)
            .collect::<HashSet<_>>()
            .len() as i64;

        assert_eq!(scoped.counts.nodes, scoped.data.nodes.len() as i64);
        assert_eq!(scoped.counts.edges, scoped.data.edges.len() as i64);
        assert_eq!(scoped.counts.files, scoped_files);
        assert_eq!(scoped.counts.files, 1);
        assert_eq!(scoped.counts.fts_docs, 1);
        assert_eq!(service.counts().await.unwrap().fts_docs, 2);
    }

    #[tokio::test]
    async fn scoped_analytics_cache_keeps_multiple_scopes_and_evicts_oldest() {
        let service = CodeGraphService::open_in_memory().unwrap();
        for index in 0..=SCOPED_ANALYTICS_CACHE_CAPACITY {
            service
                .index_file(
                    &format!("src/{index}.rs"),
                    &format!("pub fn item_{index}() {{}}\n"),
                    "rust",
                )
                .await
                .unwrap();
        }

        let scopes = (0..=SCOPED_ANALYTICS_CACHE_CAPACITY)
            .map(|index| HashSet::from([format!("src/{index}.rs")]))
            .collect::<Vec<_>>();
        let mut snapshots = Vec::new();
        for scope in &scopes[..SCOPED_ANALYTICS_CACHE_CAPACITY] {
            snapshots.push(service.scoped_graph_analytics(scope).await.unwrap());
        }
        for (scope, snapshot) in scopes[..SCOPED_ANALYTICS_CACHE_CAPACITY]
            .iter()
            .zip(&snapshots)
        {
            let cached = service.scoped_graph_analytics(scope).await.unwrap();
            assert!(Arc::ptr_eq(snapshot, &cached));
        }
        assert_eq!(
            service.scoped_analytics_rebuild_count(),
            SCOPED_ANALYTICS_CACHE_CAPACITY
        );

        service
            .scoped_graph_analytics(&scopes[SCOPED_ANALYTICS_CACHE_CAPACITY])
            .await
            .unwrap();
        let cache = service.scoped_analytics_cache.lock().await;
        assert_eq!(cache.entries.len(), SCOPED_ANALYTICS_CACHE_CAPACITY);
        assert!(cache
            .entries
            .iter()
            .all(|entry| entry.key.allowed_paths != vec!["src/0.rs".to_string()]));
        drop(cache);

        service.scoped_graph_analytics(&scopes[0]).await.unwrap();
        assert_eq!(
            service.scoped_analytics_rebuild_count(),
            SCOPED_ANALYTICS_CACHE_CAPACITY + 2
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scoped_analytics_rebuild_does_not_hold_cache_mutex() {
        let service = Arc::new(CodeGraphService::open_in_memory().unwrap());
        service
            .index_file("src/a.rs", "pub fn a() {}\n", "rust")
            .await
            .unwrap();
        let allowed = HashSet::from(["src/a.rs".to_string()]);
        let gate = AnalyticsRebuildGate::new();
        let rebuild = {
            let service = service.clone();
            let allowed = allowed.clone();
            let gate = gate.clone();
            tokio::spawn(async move {
                service
                    .scoped_graph_analytics_with_gate(&allowed, gate)
                    .await
            })
        };
        gate.wait_paused().await;

        assert!(service.scoped_analytics_cache.try_lock().is_ok());
        let concurrent = service.scoped_graph_analytics(&allowed).await.unwrap();
        gate.resume();
        let paused = rebuild.await.unwrap().unwrap();
        assert!(Arc::ptr_eq(&concurrent, &paused));
    }

    #[tokio::test]
    async fn scoped_analytics_generation_churn_returns_explicit_error() {
        let service = CodeGraphService::open_in_memory().unwrap();
        service
            .index_file("src/a.rs", "pub fn a() {}\n", "rust")
            .await
            .unwrap();
        let allowed = HashSet::from(["src/a.rs".to_string()]);

        let error = service
            .scoped_graph_analytics_with_pause(
                &allowed,
                ScopedAnalyticsRebuildPause::AdvanceGeneration,
            )
            .await
            .unwrap_err();

        assert_eq!(
            service.scoped_analytics_rebuild_count(),
            SCOPED_ANALYTICS_MAX_REBUILD_ATTEMPTS
        );
        assert!(error.contains("generation changed"), "{error}");
        assert!(service
            .scoped_analytics_cache
            .lock()
            .await
            .entries
            .is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn analytics_rebuild_in_flight_does_not_publish_or_block_stale_generation() {
        let service = Arc::new(CodeGraphService::open_in_memory().unwrap());
        service
            .index_file("src/a.rs", "pub fn helper() {}\n", "rust")
            .await
            .unwrap();

        let generation_before = service.graph_generation.load(Ordering::Acquire);
        let gate = AnalyticsRebuildGate::new();
        let first = {
            let service = service.clone();
            let gate = gate.clone();
            tokio::spawn(async move { service.cached_graph_analytics_with_gate(gate).await })
        };
        gate.wait_paused().await;

        service
            .index_file("src/b.rs", "pub fn added() {}\n", "rust")
            .await
            .unwrap();
        let generation_after = service.graph_generation.load(Ordering::Acquire);
        assert!(generation_after > generation_before);

        let second =
            tokio::time::timeout(Duration::from_millis(500), service.cached_graph_analytics())
                .await
                .expect("analytics cache lock must not be held while another rebuild is paused")
                .unwrap();
        assert_eq!(second.generation, generation_after);
        assert!(second.data.nodes.iter().any(|(_, name, _)| name == "added"));

        gate.resume();
        let first = first.await.unwrap().unwrap();
        assert_eq!(first.generation, generation_after);
        assert!(first.data.nodes.iter().any(|(_, name, _)| name == "added"));

        let cached = service.analytics_cache.lock().await.clone().unwrap();
        assert_eq!(cached.generation, generation_after);
        assert!(cached.data.nodes.iter().any(|(_, name, _)| name == "added"));
    }

    #[tokio::test]
    async fn file_backed_doc_defs_reads_while_writer_mutex_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let service = CodeGraphService::open(dir.path().join("codegraph.sqlite")).unwrap();
        service
            .index_file("src/a.rs", "pub fn ready() {}\n", "rust")
            .await
            .unwrap();

        let writer_guard = service.store.lock().await;
        let defs = tokio::time::timeout(Duration::from_millis(200), service.doc_defs("src/a.rs"))
            .await
            .expect("doc_defs should use the read connection instead of waiting for writer mutex")
            .unwrap();
        drop(writer_guard);

        assert!(defs.iter().any(|def| def.name() == "ready"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_backed_definitions_read_during_uncommitted_writer_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let service =
            Arc::new(CodeGraphService::open(dir.path().join("codegraph.sqlite")).unwrap());
        service
            .index_file("src/a.rs", "pub fn ready() {}\n", "rust")
            .await
            .unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let writer_path = service.db_path().to_path_buf();
        let writer_barrier = barrier.clone();
        let writer = thread::spawn(move || {
            let conn = rusqlite::Connection::open(&writer_path).unwrap();
            conn.pragma_update(None, "journal_mode", "WAL").unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            tx.execute(
                "INSERT INTO nodes(kind, path, name, lang, line1, line2) \
                 VALUES('file', 'src/held.rs', 'held.rs', 'rust', 1, 1)",
                [],
            )
            .unwrap();
            writer_barrier.wait();
            std::thread::sleep(Duration::from_millis(300));
            tx.rollback().unwrap();
        });
        tokio::task::spawn_blocking(move || barrier.wait())
            .await
            .unwrap();

        let defs = tokio::time::timeout(Duration::from_millis(200), service.definitions("ready"))
            .await
            .expect("definitions should read the last committed snapshot while a writer is active")
            .unwrap();
        writer.join().unwrap();

        assert!(defs.iter().any(|def| def.name() == "ready"));
    }

    // Recreating the file under a live connection is only possible where unlinking an open file
    // is: SQLite holds the database without FILE_SHARE_DELETE on Windows.
    #[cfg(unix)]
    #[tokio::test]
    async fn recovery_reopens_both_stores_and_keeps_the_queue() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("codegraph.sqlite");
        let service = CodeGraphService::open(db_path.clone()).unwrap();
        service.enqueue_files(&["src/a.rs".to_string(), "src/b.rs".to_string()]);

        assert!(!service.database_replaced());
        assert_eq!(
            service.recover_replaced_database().await,
            DatabaseRecovery::NotNeeded
        );

        Store::remove_sqlite_files(&db_path).unwrap();
        drop(Store::open(&db_path).unwrap());
        assert!(
            service.database_replaced(),
            "a recreated file at the same path must be detected"
        );

        let generation_before = service.store_generation();
        assert_eq!(
            service.recover_replaced_database().await,
            DatabaseRecovery::Recovered
        );

        assert!(service.is_available());
        assert!(
            !service.database_replaced(),
            "the reopened writer must track the fresh inode"
        );
        assert!(service.store_generation() > generation_before);
        assert_eq!(
            service.queue_len(),
            2,
            "recovery must preserve the pending queue, the fresh db has none of that content"
        );
        service
            .index_file("src/a.rs", "fn a() {}\n", "rust")
            .await
            .expect("the writer connection must work after recovery");
        assert!(
            !service.definitions("a").await.unwrap().is_empty(),
            "the reader connection must see the fresh database too"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_reopen_reports_unavailable_and_does_not_spin() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("codegraph.sqlite");
        let service = CodeGraphService::open(db_path.clone()).unwrap();
        service.enqueue_files(&["src/a.rs".to_string()]);

        // A directory at the database path is a different inode than the retained handle, and no
        // reopen attempt can ever succeed against it.
        Store::remove_sqlite_files(&db_path).unwrap();
        std::fs::create_dir(&db_path).unwrap();

        assert!(service.database_replaced());
        match service.recover_replaced_database().await {
            DatabaseRecovery::Failed(err) => {
                assert!(err.contains(DB_UNAVAILABLE_ERROR), "got {err:?}")
            }
            other => panic!("a broken path must fail the reopen, got {other:?}"),
        }
        assert!(!service.is_available());
        assert_eq!(service.reopen_attempts(), 1);

        for _ in 0..5 {
            assert_eq!(
                service.recover_replaced_database().await,
                DatabaseRecovery::Deferred,
                "retries inside the interval must be deferred, not attempted"
            );
        }
        assert_eq!(
            service.reopen_attempts(),
            1,
            "a failing reopen must not spin: retries wait for the idle cadence"
        );
        assert_eq!(
            service.queue_len(),
            1,
            "an unavailable database must not drop the queue"
        );
        assert!(service.counts().await.is_err());
    }
}
