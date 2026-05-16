use std::path::PathBuf;
use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::RwLock as ARwLock;
use refact_ast::ast::ast_structs::AstDefinition;
use refact_ast::ast::treesitter::parsers::get_ast_parser_by_filename;
use refact_postprocessing::pp_context_provider::PPContextTrait;

use crate::global_context::GlobalContext;
use crate::files_correction::{canonical_path, correct_to_nearest_filename, shortify_paths};
use crate::files_in_workspace::get_file_text_from_memory_or_disk;

pub struct GcxPPContext(pub Arc<ARwLock<GlobalContext>>);

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
        let path_buf = PathBuf::from(path);
        let ast_service = self.0.read().await.ast_service.clone();
        match ast_service {
            Some(ast) if get_ast_parser_by_filename(&path_buf).is_ok() => {
                let ast_index = ast.lock().await.ast_index.clone();
                crate::ast::ast_db::doc_defs(ast_index, &path.to_string())
            }
            _ => vec![],
        }
    }

    fn canonical_path(&self, path: &str) -> PathBuf {
        canonical_path(path)
    }
}
