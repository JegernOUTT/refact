use std::path::PathBuf;
use std::sync::Arc;
use async_trait::async_trait;
use refact_core::ast_types::AstDefinition;
use refact_core::chat_types::ContextFile;

#[async_trait]
pub trait PPContextTrait: Send + Sync {
    async fn read_file(&self, path: &PathBuf) -> Result<String, String>;
    async fn read_file_for_dedup(&self, path: &PathBuf) -> Result<String, String> {
        self.read_file(path).await
    }
    async fn correct_to_nearest_filename(&self, path: &str, limit: usize) -> Vec<String>;
    async fn shortify_paths(&self, paths: &[String]) -> Vec<String>;
    async fn doc_defs_for_path(&self, path: &str) -> Vec<Arc<AstDefinition>>;
    fn canonical_path(&self, path: &str) -> PathBuf;
    fn record_deduplicated_read(&self, _file: &ContextFile, _current_text: &str) {}
}
