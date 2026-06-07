pub mod code_completion_fim;
pub mod completion_cache;
pub(crate) mod completon_rag;
pub mod multimodality;
pub mod scratchpad_abstract;
pub mod scratchpad_utils;

use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};
use serde_json::Value;
use tokenizers::Tokenizer;

use refact_ast::ast::ast_structs::AstDB;
use refact_core::chat_types::CodeCompletionPost;
use refact_postprocessing::pp_context_provider::PPContextTrait;

use scratchpad_abstract::ScratchpadAbstract;

pub async fn create_code_completion_scratchpad(
    tokenizer: Option<Arc<Tokenizer>>,
    scratchpad_name: &str,
    scratchpad_patch: &Value,
    post: &CodeCompletionPost,
    cache_arc: Arc<StdRwLock<completion_cache::CompletionCache>>,
    ast_index: Option<Arc<AstDB>>,
    pp_context: Arc<dyn PPContextTrait>,
    project_dirs: Vec<PathBuf>,
) -> Result<Box<dyn ScratchpadAbstract>, String> {
    let mut result: Box<dyn ScratchpadAbstract> = if scratchpad_name == "plain" {
        Box::new(code_completion_fim::FillInTheMiddleScratchpad::new(
            tokenizer,
            post,
            "plain".to_string(),
            cache_arc,
            ast_index,
            pp_context,
            project_dirs,
        ))
    } else if scratchpad_name == "FIM-PSM" {
        Box::new(code_completion_fim::FillInTheMiddleScratchpad::new(
            tokenizer,
            post,
            "PSM".to_string(),
            cache_arc,
            ast_index,
            pp_context,
            project_dirs,
        ))
    } else if scratchpad_name == "FIM-SPM" {
        Box::new(code_completion_fim::FillInTheMiddleScratchpad::new(
            tokenizer,
            post,
            "SPM".to_string(),
            cache_arc,
            ast_index,
            pp_context,
            project_dirs,
        ))
    } else {
        return Err(format!(
            "Unsupported completion scratchpad '{}'. Only plain, FIM-PSM and FIM-SPM are supported.",
            scratchpad_name
        ));
    };
    result
        .apply_model_adaptation_patch(scratchpad_patch)
        .await?;
    Ok(result)
}
