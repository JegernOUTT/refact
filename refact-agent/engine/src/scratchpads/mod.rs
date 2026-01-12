use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};

pub mod code_completion_fim;
mod completon_rag;
pub mod multimodality;
pub mod scratchpad_utils;

pub use crate::chat::history_limit as chat_utils_limit_history;
pub use crate::chat::prompts as chat_utils_prompts;

use crate::ast::ast_indexer_thread::AstIndexService;
use crate::call_validation::CodeCompletionPost;
use crate::caps::CompletionModelRecord;
use crate::global_context::GlobalContext;
use crate::scratchpad_abstract::ScratchpadAbstract;
use crate::completion_cache;
use crate::telemetry::telemetry_structs;

fn verify_has_send<T: Send>(_x: &T) {}

pub async fn create_code_completion_scratchpad(
    global_context: Arc<ARwLock<GlobalContext>>,
    model_rec: &CompletionModelRecord,
    post: &CodeCompletionPost,
    cache_arc: Arc<StdRwLock<completion_cache::CompletionCache>>,
    tele_storage: Arc<StdRwLock<telemetry_structs::Storage>>,
    ast_module: Option<Arc<AMutex<AstIndexService>>>,
) -> Result<Box<dyn ScratchpadAbstract>, String> {
    let tokenizer_arc =
        crate::tokens::cached_tokenizer(global_context.clone(), &model_rec.base).await?;
    let mut result: Box<dyn ScratchpadAbstract> = if model_rec.scratchpad == "FIM-PSM" {
        Box::new(code_completion_fim::FillInTheMiddleScratchpad::new(
            tokenizer_arc,
            &post,
            "PSM".to_string(),
            cache_arc,
            tele_storage,
            ast_module,
            global_context.clone(),
        ))
    } else if model_rec.scratchpad == "FIM-SPM" {
        Box::new(code_completion_fim::FillInTheMiddleScratchpad::new(
            tokenizer_arc,
            &post,
            "SPM".to_string(),
            cache_arc,
            tele_storage,
            ast_module,
            global_context.clone(),
        ))
    } else {
        return Err(format!(
            "Unsupported completion scratchpad '{}'. Only FIM-PSM and FIM-SPM are supported.",
            model_rec.scratchpad
        ));
    };
    result
        .apply_model_adaptation_patch(&model_rec.scratchpad_patch)
        .await?;
    verify_has_send(&result);
    Ok(result)
}
