use std::path::PathBuf;
use std::sync::Arc;
use async_trait::async_trait;
use refact_core::ast_types::AstDefinition;

#[async_trait]
pub trait PPContextTrait: Send + Sync {
    async fn read_file(&self, path: &PathBuf) -> Result<String, String>;
    async fn correct_to_nearest_filename(&self, path: &str, limit: usize) -> Vec<String>;
    async fn shortify_paths(&self, paths: &[String]) -> Vec<String>;
    async fn doc_defs_for_path(&self, path: &str) -> Vec<Arc<AstDefinition>>;
    fn canonical_path(&self, path: &str) -> PathBuf;
}
