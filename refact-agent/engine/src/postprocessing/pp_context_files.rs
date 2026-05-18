use std::sync::Arc;
use tokenizers::Tokenizer;

pub use refact_postprocessing::pp_context_files::{
    FileLine, PPFile, pp_color_lines, DEBUG, MAX_LINE_LENGTH, RESERVE_FOR_QUESTION_AND_FOLLOWUP,
};
use refact_core::chat_types::{ContextFile, PostprocessSettings};

use crate::global_context::GlobalContext;
use super::gcx_pp_context::GcxPPContext;

pub async fn postprocess_context_files(
    gcx: Arc<GlobalContext>,
    context_file_vec: &mut Vec<ContextFile>,
    tokenizer: Option<Arc<Tokenizer>>,
    tokens_limit: usize,
    single_file_mode: bool,
    settings: &PostprocessSettings,
) -> (Vec<ContextFile>, Vec<String>) {
    refact_postprocessing::pp_context_files::postprocess_context_files(
        Arc::new(GcxPPContext(gcx)),
        context_file_vec,
        tokenizer,
        tokens_limit,
        single_file_mode,
        settings,
    )
    .await
}
