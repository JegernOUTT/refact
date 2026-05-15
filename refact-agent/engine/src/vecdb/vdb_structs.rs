pub use refact_core::vecdb_types::{
    EmbeddingModelConfig, SearchResult, SimpleTextHashVector, SplitResult, VecDbStatus,
    VecdbRecord, VecdbSearch,
};
pub use refact_vecdb::vdb_structs::VecdbConstants;

use crate::caps::EmbeddingModelRecord;

impl From<&EmbeddingModelRecord> for EmbeddingModelConfig {
    fn from(model: &EmbeddingModelRecord) -> Self {
        Self {
            endpoint: model.base.endpoint.clone(),
            endpoint_style: model.base.endpoint_style.clone(),
            api_key: model.base.api_key.clone(),
            model_name: model.base.name.clone(),
            embedding_size: model.embedding_size,
            rejection_threshold: model.rejection_threshold,
            embedding_batch: model.embedding_batch,
            n_ctx: model.base.n_ctx,
        }
    }
}
