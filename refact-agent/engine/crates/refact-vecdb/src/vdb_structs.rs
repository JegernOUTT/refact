use std::sync::Arc;
use tokenizers::Tokenizer;

pub use refact_core::vecdb_types::{
    EmbeddingModelConfig, SearchResult, SimpleTextHashVector, SplitResult, VecDbStatus,
    VecdbRecord, VecdbSearch,
};

#[derive(Debug, Clone)]
pub struct VecdbConstants {
    pub embedding_model: EmbeddingModelConfig,
    pub tokenizer: Option<Arc<Tokenizer>>,
    pub splitter_window_size: usize,
    pub vecdb_max_files: usize,
}
