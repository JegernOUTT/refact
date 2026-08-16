use std::sync::Arc;
use tokenizers::Tokenizer;

pub use refact_postprocessing::pp_tool_results::ToolBudget;
use refact_core::chat_types::{ChatMessage, ContextFile, PostprocessSettings};

use crate::global_context::GlobalContext;
use super::gcx_pp_context::GcxPPContext;

pub async fn postprocess_tool_results(
    gcx: Arc<GlobalContext>,
    tokenizer: Option<Arc<Tokenizer>>,
    tool_messages: Vec<ChatMessage>,
    context_files: Vec<ContextFile>,
    budget: ToolBudget,
    pp_settings: PostprocessSettings,
    existing_messages: &[ChatMessage],
    truncation_exempt_tool_call_ids: &std::collections::HashSet<String>,
) -> Vec<ChatMessage> {
    let mut messages = refact_postprocessing::pp_tool_results::postprocess_tool_results(
        Arc::new(GcxPPContext(gcx.clone())),
        tokenizer,
        tool_messages,
        context_files,
        budget,
        pp_settings,
        existing_messages,
        truncation_exempt_tool_call_ids,
    )
    .await;
    if let Err(error) = crate::privacy::records::attach_declared_output_files(&gcx, &mut messages) {
        tracing::error!("failed to attach file privacy records: {error}");
    }
    messages
}
