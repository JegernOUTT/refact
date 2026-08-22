use std::sync::Arc;
use tokenizers::Tokenizer;

pub use refact_postprocessing::pp_tool_results::ToolBudget;
use refact_core::chat_types::{ChatMessage, ContextFile, PostprocessSettings};

use crate::global_context::GlobalContext;
use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome};
use super::gcx_pp_context::GcxPPContext;

fn postprocess_item_count(tool_messages: &[ChatMessage], context_files: &[ContextFile]) -> u64 {
    tool_messages.len().saturating_add(context_files.len()) as u64
}

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
    let item_count = postprocess_item_count(&tool_messages, &context_files);
    let span = perf_diagnostics::span(PerfComponent::ToolResultPostprocess, None, None);
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
    span.finish_tool(PerfOutcome::Success, 1, item_count, None);
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postprocess_item_count_covers_large_tool_batches() {
        let tool_messages = (0..10_000)
            .map(|_| ChatMessage::new("tool".to_string(), "result".to_string()))
            .collect::<Vec<_>>();

        assert_eq!(postprocess_item_count(&tool_messages, &[]), 10_000);
    }
}
