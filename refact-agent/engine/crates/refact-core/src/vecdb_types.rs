use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use indexmap::IndexMap;
use async_trait::async_trait;

#[async_trait]
pub trait VecdbSearch: Send {
    async fn vecdb_search(
        &self,
        query: String,
        top_n: usize,
        filter_mb: Option<String>,
    ) -> Result<SearchResult, String>;
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VecDbStatus {
    pub files_unprocessed: usize,
    pub files_total: usize,
    pub requests_made_since_start: usize,
    pub vectors_made_since_start: usize,
    pub db_size: usize,
    pub db_cache_size: usize,
    pub state: String,
    pub queue_additions: bool,
    pub vecdb_max_files_hit: bool,
    pub vecdb_errors: IndexMap<String, usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct VecdbRecord {
    pub vector: Option<Vec<f32>>,
    pub file_path: PathBuf,
    pub start_line: u64,
    pub end_line: u64,
    pub distance: f32,
    pub usefulness: f32,
}

#[derive(Debug, Clone)]
pub struct SplitResult {
    pub file_path: PathBuf,
    pub window_text: String,
    pub window_text_hash: String,
    pub start_line: u64,
    pub end_line: u64,
    pub symbol_path: String,
}

#[derive(Clone)]
pub struct SimpleTextHashVector {
    pub window_text: String,
    pub window_text_hash: String,
    pub vector: Option<Vec<f32>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchResult {
    pub query_text: String,
    pub results: Vec<VecdbRecord>,
}
