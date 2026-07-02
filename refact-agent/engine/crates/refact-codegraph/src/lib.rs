pub mod analytics;
pub mod communities;
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

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AMutex;
use tokio::sync::Notify;

pub use store::{Counts, Store};

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
    pub data: analytics::GraphData,
    pub analytics: analytics::GraphAnalytics,
    pub communities: Vec<communities::Community>,
}

pub struct CodeGraphService {
    store: AMutex<Store>,
    read_store: Option<AMutex<Store>>,
    queue: StdMutex<PendingQueue>,
    queue_notify: Notify,
    db_path: PathBuf,
    initial_index_done: AtomicBool,
    graph_generation: AtomicU64,
    analytics_cache: AMutex<Option<CachedGraphAnalytics>>,
    analytics_rebuild_count: AtomicUsize,
}

enum AnalyticsRebuildPause {
    None,
    #[cfg(test)]
    Gate(std::sync::Arc<AnalyticsRebuildGate>),
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
    resume: Notify,
}

#[cfg(test)]
impl AnalyticsRebuildGate {
    fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            after_rebuild: tokio::sync::Barrier::new(2),
            resume: Notify::new(),
        })
    }

    async fn pause(&self) {
        self.after_rebuild.wait().await;
        self.resume.notified().await;
    }

    async fn wait_paused(&self) {
        self.after_rebuild.wait().await;
    }

    fn resume(&self) {
        self.resume.notify_waiters();
    }
}
#[derive(Default)]
struct PendingQueue {
    order: VecDeque<String>,
    entries: HashMap<String, QueuedPath>,
}

impl CodeGraphService {
    pub fn open(db_path: PathBuf) -> Result<Self, String> {
        let store = Store::open(&db_path)?;
        let read_store = Store::open_readonly(&db_path)?;
        Ok(Self {
            store: AMutex::new(store),
            read_store: Some(AMutex::new(read_store)),
            queue: StdMutex::new(PendingQueue::default()),
            queue_notify: Notify::new(),
            db_path,
            initial_index_done: AtomicBool::new(false),
            graph_generation: AtomicU64::new(0),
            analytics_cache: AMutex::new(None),
            analytics_rebuild_count: AtomicUsize::new(0),
        })
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let store = Store::open_in_memory()?;
        Ok(Self {
            store: AMutex::new(store),
            read_store: None,
            queue: StdMutex::new(PendingQueue::default()),
            queue_notify: Notify::new(),
            db_path: PathBuf::from(":memory:"),
            initial_index_done: AtomicBool::new(true),
            graph_generation: AtomicU64::new(0),
            analytics_cache: AMutex::new(None),
            analytics_rebuild_count: AtomicUsize::new(0),
        })
    }

    pub fn mark_initial_index_done(&self) {
        self.initial_index_done.store(true, Ordering::Relaxed);
    }

    pub fn is_initial_index_done(&self) -> bool {
        self.initial_index_done.load(Ordering::Relaxed)
    }

    pub async fn wait_until_ready(&self, timeout_ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if self.is_initial_index_done() && self.queue_len() == 0 {
                return true;
            }
            if Instant::now() >= deadline {
                return self.is_initial_index_done();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
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

    pub async fn wait_for_enqueue(&self) {
        self.queue_notify.notified().await;
    }

    async fn with_read_store<T>(
        &self,
        f: impl FnOnce(&Store) -> Result<T, String>,
    ) -> Result<T, String> {
        if let Some(store) = &self.read_store {
            let store = store.lock().await;
            f(&store)
        } else {
            let store = self.store.lock().await;
            f(&store)
        }
    }

    fn bump_graph_generation(&self) {
        self.graph_generation.fetch_add(1, Ordering::AcqRel);
    }

    #[cfg(test)]
    pub fn analytics_rebuild_count(&self) -> usize {
        self.analytics_rebuild_count.load(Ordering::Relaxed)
    }

    pub async fn index_file(&self, path: &str, text: &str, lang: &str) -> Result<(), String> {
        let store = self.store.lock().await;
        let (_file_id, changed) = store.index_file_graph(path, text, lang)?;
        drop(store);
        if changed {
            self.bump_graph_generation();
        }
        Ok(())
    }

    pub async fn remove_path(&self, path: &str) -> Result<(), String> {
        let store = self.store.lock().await;
        let changed = store.remove_path(path)?;
        drop(store);
        if changed {
            self.bump_graph_generation();
        }
        Ok(())
    }

    pub async fn counts(&self) -> Result<Counts, String> {
        self.with_read_store(|store| store.counts()).await
    }

    pub async fn connect_usages(&self) -> Result<(), String> {
        let store = self.store.lock().await;
        let changed = store.connect_usages()?;
        drop(store);
        if changed {
            self.bump_graph_generation();
        }
        Ok(())
    }

    pub async fn has_dirty_usage_paths(&self) -> Result<bool, String> {
        self.with_read_store(|store| store.has_dirty_paths()).await
    }

    pub async fn doc_usages(&self, cpath: &str) -> Result<Vec<(usize, String)>, String> {
        self.with_read_store(|store| store.doc_usages(cpath)).await
    }

    pub async fn cached_graph_analytics(&self) -> Result<CachedGraphAnalytics, String> {
        self.cached_graph_analytics_with_pause(AnalyticsRebuildPause::None)
            .await
    }

    #[cfg(test)]
    async fn cached_graph_analytics_with_gate(
        &self,
        gate: std::sync::Arc<AnalyticsRebuildGate>,
    ) -> Result<CachedGraphAnalytics, String> {
        self.cached_graph_analytics_with_pause(AnalyticsRebuildPause::Gate(gate))
            .await
    }

    async fn cached_graph_analytics_with_pause(
        &self,
        pause: AnalyticsRebuildPause,
    ) -> Result<CachedGraphAnalytics, String> {
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
    ) -> Result<CachedGraphAnalytics, String> {
        self.with_read_store(|store| {
            store.read_snapshot(|store| {
                let data = analytics::GraphData::from_store(store)?;
                let analytics = analytics::compute_graph_analytics_from_data(&data);
                let communities = communities::detect_communities_from_data(&data)?;
                Ok(CachedGraphAnalytics {
                    generation,
                    data,
                    analytics,
                    communities,
                })
            })
        })
        .await
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

    pub async fn graph_nodes(&self) -> Result<Vec<analytics::GraphNode>, String> {
        Ok(self.cached_graph_analytics().await?.data.nodes)
    }

    pub async fn graph_edges(&self) -> Result<Vec<analytics::GraphEdge>, String> {
        Ok(self.cached_graph_analytics().await?.data.edges)
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
        Ok(self.cached_graph_analytics().await?.communities)
    }

    pub async fn execution_flows(
        &self,
        max_flows: usize,
    ) -> Result<Vec<communities::ExecFlow>, String> {
        let cached = self.cached_graph_analytics().await?;
        communities::execution_flows_from_data(&cached.data, max_flows)
    }

    pub async fn dead_code(&self) -> Result<Vec<dead_code::DeadSymbol>, String> {
        self.with_read_store(dead_code::dead_code).await
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
    async fn overview_reuses_cached_graph_analytics_until_index_changes() {
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
}
