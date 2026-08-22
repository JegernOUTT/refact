use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokenizers::Tokenizer;

pub use refact_core::vecdb_types::{
    EmbeddingModelConfig, SearchResult, SimpleTextHashVector, SplitResult, VecDbStatus,
    VecdbRecord, VecdbSearch,
};

pub type EmbeddingCredentialResolver = Arc<
    dyn Fn(Option<String>) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct VecdbConstants {
    pub embedding_model: EmbeddingModelConfig,
    pub embedding_credential_resolver: Option<EmbeddingCredentialResolver>,
    pub tokenizer: Option<Arc<Tokenizer>>,
    pub splitter_window_size: usize,
    pub vecdb_max_files: usize,
}

impl std::fmt::Debug for VecdbConstants {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VecdbConstants")
            .field("embedding_model", &self.embedding_model)
            .field(
                "embedding_credential_resolver",
                &self
                    .embedding_credential_resolver
                    .as_ref()
                    .map(|_| "<resolver>"),
            )
            .field("tokenizer", &self.tokenizer)
            .field("splitter_window_size", &self.splitter_window_size)
            .field("vecdb_max_files", &self.vecdb_max_files)
            .finish()
    }
}
