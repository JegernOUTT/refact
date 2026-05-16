use std::sync::Arc;
use std::collections::HashSet;
use tokenizers::Tokenizer;
use tokio::sync::RwLock as ARwLock;

pub use refact_postprocessing::pp_utils::{
    color_with_gradient_type, colorize_comments_up, colorize_if_more_useful, colorize_minus_one,
    colorize_parentof, downgrade_lines_if_subsymbol,
};
pub use refact_postprocessing::pp_context_files::PPFile;
use refact_core::chat_types::{ContextFile, PostprocessSettings};

use crate::global_context::GlobalContext;
use super::gcx_pp_context::GcxPPContext;

pub async fn pp_resolve_ctx_file_paths(
    gcx: Arc<ARwLock<GlobalContext>>,
    context_file_vec: &mut Vec<ContextFile>,
) -> Vec<(String, String)> {
    refact_postprocessing::pp_utils::pp_resolve_ctx_file_paths(
        Arc::new(GcxPPContext(gcx)),
        context_file_vec,
    )
    .await
}

pub async fn pp_ast_markup_files(
    gcx: Arc<ARwLock<GlobalContext>>,
    context_file_vec: &mut Vec<ContextFile>,
) -> Vec<Arc<PPFile>> {
    refact_postprocessing::pp_utils::pp_ast_markup_files(
        Arc::new(GcxPPContext(gcx)),
        context_file_vec,
    )
    .await
}

pub async fn pp_load_files_without_ast(
    gcx: Arc<ARwLock<GlobalContext>>,
    context_file_vec: &mut Vec<ContextFile>,
) -> Vec<Arc<PPFile>> {
    refact_postprocessing::pp_utils::pp_load_files_without_ast(
        Arc::new(GcxPPContext(gcx)),
        context_file_vec,
    )
    .await
}

pub async fn context_msgs_from_paths(
    gcx: Arc<ARwLock<GlobalContext>>,
    files_set: HashSet<String>,
) -> Vec<ContextFile> {
    refact_postprocessing::pp_utils::context_msgs_from_paths(Arc::new(GcxPPContext(gcx)), files_set)
        .await
}
