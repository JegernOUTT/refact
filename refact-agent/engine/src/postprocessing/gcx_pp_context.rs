use std::path::PathBuf;
use std::sync::Arc;
use async_trait::async_trait;
use refact_core::ast_types::AstDefinition;
use refact_postprocessing::pp_context_provider::PPContextTrait;

use crate::global_context::GlobalContext;
use crate::files_correction::{canonical_path, correct_to_nearest_filename, shortify_paths};
use crate::files_in_workspace::get_file_text_from_memory_or_disk;

pub struct GcxPPContext(pub Arc<GlobalContext>);

#[async_trait]
impl PPContextTrait for GcxPPContext {
    async fn read_file(&self, path: &PathBuf) -> Result<String, String> {
        get_file_text_from_memory_or_disk(self.0.clone(), path).await
    }

    async fn correct_to_nearest_filename(&self, path: &str, limit: usize) -> Vec<String> {
        correct_to_nearest_filename(self.0.clone(), &path.to_string(), false, limit).await
    }

    async fn shortify_paths(&self, paths: &[String]) -> Vec<String> {
        shortify_paths(self.0.clone(), &paths.to_vec()).await
    }

    async fn doc_defs_for_path(&self, path: &str) -> Vec<Arc<AstDefinition>> {
        let codegraph_opt = self.0.codegraph.lock().await.clone();
        match codegraph_opt {
            Some(service) => service.doc_defs(path).await.unwrap_or_default(),
            None => vec![],
        }
    }

    fn canonical_path(&self, path: &str) -> PathBuf {
        canonical_path(path)
    }
}
