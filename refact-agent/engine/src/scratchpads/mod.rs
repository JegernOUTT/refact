use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use tokio::sync::Mutex as AMutex;

pub mod multimodality;
pub mod scratchpad_utils;
pub use refact_scratchpads::code_completion_fim;

pub use crate::chat::history_limit as chat_utils_limit_history;
pub use crate::chat::prompts as chat_utils_prompts;

use crate::ast::ast_indexer_thread::AstIndexService;
use crate::call_validation::CodeCompletionPost;
use crate::caps::CompletionModelRecord;
use crate::global_context::GlobalContext;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::completion_cache;

pub async fn create_code_completion_scratchpad(
    global_context: Arc<GlobalContext>,
    model_rec: &CompletionModelRecord,
    post: &CodeCompletionPost,
    cache_arc: Arc<StdRwLock<completion_cache::CompletionCache>>,
    ast_module: Option<Arc<AMutex<AstIndexService>>>,
) -> Result<Box<dyn ScratchpadAbstract>, String> {
    let tokenizer =
        crate::tokens::cached_tokenizer(global_context.clone(), &model_rec.base).await?;
    let ast_index = match ast_module {
        Some(ast) => Some(ast.lock().await.ast_index.clone()),
        None => None,
    };
    let pp_context = Arc::new(crate::postprocessing::gcx_pp_context::GcxPPContext(
        global_context.clone(),
    )) as Arc<dyn refact_postprocessing::pp_context_provider::PPContextTrait>;
    let project_dirs = crate::files_correction::get_project_dirs(global_context.clone()).await;
    refact_scratchpads::create_code_completion_scratchpad(
        tokenizer,
        &model_rec.scratchpad,
        &model_rec.scratchpad_patch,
        post,
        cache_arc,
        ast_index,
        pp_context,
        project_dirs,
    )
    .await
}
