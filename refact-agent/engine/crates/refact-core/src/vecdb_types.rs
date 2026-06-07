use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use indexmap::IndexMap;
use async_trait::async_trait;

use crate::llm_types::EmbeddingModelRecord;

pub type FileReader = Arc<
    dyn Fn(PathBuf) -> Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

#[async_trait]
pub trait VecdbSearch: Send + Sync {
    async fn vecdb_search(
        &self,
        query: String,
        top_n: usize,
        filter_mb: Option<String>,
    ) -> Result<SearchResult, String>;
    async fn get_status(&self) -> Result<VecDbStatus, String>;
    async fn remove_file(&self, file_path: &PathBuf) -> Result<(), String>;
    async fn vectorizer_enqueue_files(&self, documents: &[String], process_immediately: bool);
    fn current_constants(&self) -> (EmbeddingModelConfig, usize);
    async fn embed_query(&self, query: &str) -> Result<Vec<f32>, String>;
    async fn vecdb_search_with_embedding(
        &self,
        embedding: &Vec<f32>,
        top_n: usize,
        filter_mb: Option<String>,
    ) -> Result<Vec<VecdbRecord>, String>;
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct EmbeddingModelConfig {
    pub endpoint: String,
    pub endpoint_style: String,
    pub embedding_endpoint_style: String,
    pub api_key: String,
    pub auth_token: String,
    pub extra_headers: std::collections::HashMap<String, String>,
    pub model_name: String,
    pub embedding_size: i32,
    pub dimensions: Option<usize>,
    pub query_prefix: String,
    pub document_prefix: String,
    pub rejection_threshold: f32,
    pub embedding_batch: usize,
    pub n_ctx: usize,
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

impl From<&EmbeddingModelRecord> for EmbeddingModelConfig {
    fn from(model: &EmbeddingModelRecord) -> Self {
        Self {
            endpoint: model.base.endpoint.clone(),
            endpoint_style: model.base.endpoint_style.clone(),
            embedding_endpoint_style: model.base.embedding_endpoint_style.clone(),
            api_key: model.base.api_key.clone(),
            auth_token: model.base.auth_token.clone(),
            extra_headers: model.base.extra_headers.clone(),
            model_name: model.base.name.clone(),
            embedding_size: model.embedding_size,
            dimensions: model.dimensions,
            query_prefix: model.query_prefix.clone(),
            document_prefix: model.document_prefix.clone(),
            rejection_threshold: model.rejection_threshold,
            embedding_batch: model.embedding_batch,
            n_ctx: model.base.n_ctx,
        }
    }
}
