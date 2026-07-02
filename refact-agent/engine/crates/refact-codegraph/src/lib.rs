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

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AMutex;

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

pub struct CodeGraphService {
    store: AMutex<Store>,
    queue: StdMutex<VecDeque<String>>,
    db_path: PathBuf,
    initial_index_done: AtomicBool,
}

impl CodeGraphService {
    pub fn open(db_path: PathBuf) -> Result<Self, String> {
        let store = Store::open(&db_path)?;
        Ok(Self {
            store: AMutex::new(store),
            queue: StdMutex::new(VecDeque::new()),
            db_path,
            initial_index_done: AtomicBool::new(false),
        })
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let store = Store::open_in_memory()?;
        Ok(Self {
            store: AMutex::new(store),
            queue: StdMutex::new(VecDeque::new()),
            db_path: PathBuf::from(":memory:"),
            initial_index_done: AtomicBool::new(true),
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
        let mut queue = self.queue.lock().unwrap();
        for path in paths {
            queue.push_back(path.clone());
        }
    }

    pub fn drain_batch(&self, max: usize) -> Vec<String> {
        let mut queue = self.queue.lock().unwrap();
        let take = max.min(queue.len());
        queue.drain(..take).collect()
    }

    pub fn queue_len(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    pub async fn index_file(&self, path: &str, text: &str, lang: &str) -> Result<(), String> {
        let store = self.store.lock().await;
        store.index_file_graph(path, text, lang).map(|_| ())
    }

    pub async fn remove_path(&self, path: &str) -> Result<(), String> {
        let store = self.store.lock().await;
        store.remove_path(path)
    }

    pub async fn counts(&self) -> Result<Counts, String> {
        let store = self.store.lock().await;
        store.counts()
    }

    pub async fn connect_usages(&self) -> Result<(), String> {
        let store = self.store.lock().await;
        store.connect_usages()
    }

    pub async fn has_dirty_usage_paths(&self) -> Result<bool, String> {
        let store = self.store.lock().await;
        store.has_dirty_paths()
    }

    pub async fn doc_usages(&self, cpath: &str) -> Result<Vec<(usize, String)>, String> {
        let store = self.store.lock().await;
        store.doc_usages(cpath)
    }

    pub async fn overview(&self, top_n: usize) -> Result<analytics::GraphOverview, String> {
        let store = self.store.lock().await;
        analytics::compute_overview(&store, top_n)
    }

    pub async fn all_files_with_text(&self) -> Result<Vec<(String, String)>, String> {
        let store = self.store.lock().await;
        store.all_files_with_text()
    }

    pub async fn graph_nodes(&self) -> Result<Vec<(i64, String, String)>, String> {
        let store = self.store.lock().await;
        store.node_names()
    }

    pub async fn graph_edges(&self) -> Result<Vec<(i64, i64, String)>, String> {
        let store = self.store.lock().await;
        store.graph_edges()
    }

    pub async fn per_file_centrality(
        &self,
        top_n: usize,
    ) -> Result<crate::analytics::FileCentrality, String> {
        let store = self.store.lock().await;
        analytics::per_file_centrality(&store, top_n)
    }

    pub async fn communities(&self) -> Result<Vec<communities::Community>, String> {
        let store = self.store.lock().await;
        communities::detect_communities(&store)
    }

    pub async fn execution_flows(
        &self,
        max_flows: usize,
    ) -> Result<Vec<communities::ExecFlow>, String> {
        let store = self.store.lock().await;
        communities::execution_flows(&store, max_flows)
    }

    pub async fn dead_code(&self) -> Result<Vec<dead_code::DeadSymbol>, String> {
        let store = self.store.lock().await;
        dead_code::dead_code(&store)
    }

    pub async fn type_hierarchy(&self, subtree_of: &str) -> Result<String, String> {
        let store = self.store.lock().await;
        facade::type_hierarchy(&store, subtree_of)
    }

    pub async fn search_hybrid(&self, query: &str, limit: usize) -> Result<Vec<CodeHit>, String> {
        let store = self.store.lock().await;
        retrieval::search_hybrid(&store, query, limit)
    }

    pub async fn doc_defs(
        &self,
        cpath: &str,
    ) -> Result<Vec<std::sync::Arc<refact_core::ast_types::AstDefinition>>, String> {
        let store = self.store.lock().await;
        facade::doc_defs(&store, cpath)
    }

    pub async fn definitions(
        &self,
        double_colon_path: &str,
    ) -> Result<Vec<std::sync::Arc<refact_core::ast_types::AstDefinition>>, String> {
        let store = self.store.lock().await;
        facade::definitions(&store, double_colon_path)
    }

    pub async fn definition_paths_fuzzy(
        &self,
        pattern: &str,
        top_n: usize,
    ) -> Result<Vec<String>, String> {
        let store = self.store.lock().await;
        facade::definition_paths_fuzzy(&store, pattern, top_n)
    }

    pub async fn fetch_counters(&self) -> Result<refact_core::ast_types::AstCounters, String> {
        let store = self.store.lock().await;
        facade::fetch_counters(&store)
    }
}
