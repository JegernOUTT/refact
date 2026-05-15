use std::sync::Arc;
use tokenizers::Tokenizer;

use crate::caps::EmbeddingModelRecord;

pub use refact_core::vecdb_types::{
    SearchResult, SimpleTextHashVector, SplitResult, VecDbStatus, VecdbRecord, VecdbSearch,
};

#[derive(Debug, Clone)]
pub struct VecdbConstants {
    pub embedding_model: EmbeddingModelRecord,
    pub tokenizer: Option<Arc<Tokenizer>>,
    pub splitter_window_size: usize,
    pub vecdb_max_files: usize,
}
