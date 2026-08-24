use std::sync::Arc;
use tokenizers::Tokenizer;

pub use refact_postprocessing::pp_tool_results::ToolBudget;
use refact_core::chat_types::{ChatContent, ChatMessage, ContextFile, PostprocessSettings};
use refact_privacy::PrivacyRecord;
use refact_chat_api::{
    attach_tool_enrichment, redact_tool_enrichment, ToolEnrichment, ToolEnrichmentKind,
    ToolEnrichmentProvenance, ToolEnrichmentReference,
};

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
    let outcome = match crate::privacy::records::attach_declared_output_files(&gcx, &mut messages) {
        Ok(()) => PerfOutcome::Success,
        Err(error) => {
            tracing::error!("failed to attach file privacy records: {error}");
            PerfOutcome::Failure
        }
    };
    enrich_tool_messages(&mut messages);
    span.finish_tool(outcome, 1, item_count, None);
    messages
}

fn enrich_tool_messages(messages: &mut [ChatMessage]) {
    for message in messages {
        if message.role != "tool" && message.role != "context_file" {
            continue;
        }
        if tool_result_is_privacy_restricted(message) {
            redact_tool_enrichment(message);
            continue;
        }
        let references = native_references(message);
        if references.is_empty() {
            continue;
        }
        attach_tool_enrichment(
            message,
            ToolEnrichment {
                references,
                ..Default::default()
            },
        );
    }
}

fn tool_result_is_privacy_restricted(message: &ChatMessage) -> bool {
    if message.extra.contains_key("privacy_shell")
        || message.extra.contains_key("privacy_observation")
    {
        return true;
    }
    match message.extra.get("privacy") {
        Some(value) => serde_json::from_value::<PrivacyRecord>(value.clone())
            .map(|privacy| privacy.files.iter().any(|file| file.zone != "normal"))
            .unwrap_or(true),
        None => false,
    }
}

fn native_references(message: &ChatMessage) -> Vec<ToolEnrichmentReference> {
    let mut references = Vec::new();
    if let ChatContent::ContextFiles(files) = &message.content {
        for file in files {
            let mut path = reference(ToolEnrichmentKind::Path, &file.file_name);
            path.line1 = (file.line1 > 0).then_some(file.line1);
            path.line2 = (file.line2 > 0).then_some(file.line2);
            path.source = Some("context_file".to_string());
            references.push(path);
            references.extend(file.symbols.iter().map(|symbol| {
                let mut reference = reference(ToolEnrichmentKind::Symbol, symbol);
                reference.source = Some("context_file".to_string());
                reference
            }));
        }
    }
    if let Some(path_enrichment) = message.extra.get("path_enrichment") {
        if let Some(paths) = path_enrichment
            .get("references")
            .and_then(serde_json::Value::as_array)
        {
            references.extend(paths.iter().filter_map(|path| {
                let target = path.get("path")?.as_str()?;
                let mut reference = reference(ToolEnrichmentKind::Path, target);
                reference.line1 = path
                    .get("line1")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|line| usize::try_from(line).ok());
                reference.line2 = path
                    .get("line2")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|line| usize::try_from(line).ok());
                reference.source = path
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string);
                Some(reference)
            }));
        }
    }
    if let Some(results) = message
        .extra
        .get("search_results")
        .and_then(serde_json::Value::as_array)
    {
        references.extend(results.iter().filter_map(|result| {
            let url = result.get("url")?.as_str()?;
            let mut reference = reference(ToolEnrichmentKind::Url, url);
            reference.label = result
                .get("title")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string);
            Some(reference)
        }));
    }
    if let Some(exec) = message.extra.get("exec") {
        if let Some(process_id) = exec.get("process_id").and_then(serde_json::Value::as_str) {
            let mut reference = reference(ToolEnrichmentKind::Process, process_id);
            reference.label = exec
                .get("short_description")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string);
            reference.status = exec
                .get("status")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string);
            references.push(reference);
        }
    }
    if let Some(agent_id) = message
        .extra
        .get("background_agent_id")
        .and_then(serde_json::Value::as_str)
    {
        let mut reference = reference(ToolEnrichmentKind::Agent, agent_id);
        reference.status = message
            .extra
            .get("background_agent_status")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        references.push(reference);
    }
    references
}

fn reference(kind: ToolEnrichmentKind, target: &str) -> ToolEnrichmentReference {
    ToolEnrichmentReference {
        provenance: ToolEnrichmentProvenance::Native,
        ..ToolEnrichmentReference::new(kind, target)
    }
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

    #[test]
    fn native_enrichment_uses_existing_structured_fields_without_changing_raw_content() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        message.extra.insert(
            "search_results".to_string(),
            serde_json::json!([
                {"title": "Example", "url": "https://example.test/path?token=secret"}
            ]),
        );
        message.extra.insert(
            "exec".to_string(),
            serde_json::json!({
                "process_id": "process-1",
                "status": "running",
                "short_description": "Local server",
                "command": "must never be copied"
            }),
        );
        let raw = message.content.content_text_only();

        enrich_tool_messages(std::slice::from_mut(&mut message));

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();
        assert_eq!(enrichment.references.len(), 2);
        assert_eq!(enrichment.references[0].target, "https://example.test/path");
        assert_eq!(enrichment.references[1].target, "process-1");
        assert!(!serde_json::to_string(&enrichment)
            .unwrap()
            .contains("must never be copied"));
        assert_eq!(message.content.content_text_only(), raw);
    }

    #[test]
    fn native_enrichment_preserves_context_paths_and_symbols() {
        let mut message = ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: "src/lib.rs".to_string(),
                file_content: "large canonical payload".to_string(),
                line1: 1,
                line2: 2,
                file_rev: None,
                symbols: vec!["crate::entry".to_string()],
                gradient_type: -1,
                usefulness: 0.0,
                skip_pp: false,
            }]),
            ..Default::default()
        };

        enrich_tool_messages(std::slice::from_mut(&mut message));

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();
        assert_eq!(enrichment.references.len(), 2);
        assert_eq!(enrichment.references[0].kind, ToolEnrichmentKind::Path);
        assert_eq!(enrichment.references[0].target, "src/lib.rs");
        assert_eq!(enrichment.references[0].line1, Some(1));
        assert_eq!(enrichment.references[0].line2, Some(2));
        assert_eq!(enrichment.references[1].kind, ToolEnrichmentKind::Symbol);
        assert_eq!(enrichment.references[1].target, "crate::entry");
        assert!(!serde_json::to_string(&enrichment)
            .unwrap()
            .contains("large canonical payload"));
    }

    #[test]
    fn native_enrichment_preserves_structured_process_path_ranges() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        message.extra.insert(
            "path_enrichment".to_string(),
            serde_json::json!({
                "references": [{
                    "path": "src/lib.rs",
                    "line1": 4,
                    "line2": 8,
                    "source": "argv"
                }]
            }),
        );

        enrich_tool_messages(std::slice::from_mut(&mut message));

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();
        assert_eq!(enrichment.references.len(), 1);
        assert_eq!(enrichment.references[0].target, "src/lib.rs");
        assert_eq!(enrichment.references[0].line1, Some(4));
        assert_eq!(enrichment.references[0].line2, Some(8));
        assert_eq!(enrichment.references[0].source.as_deref(), Some("argv"));
        assert_eq!(message.content.content_text_only(), "raw result");
    }

    #[test]
    fn restricted_privacy_records_remove_existing_structured_metadata() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        message.extra.insert(
            "privacy".to_string(),
            serde_json::json!({
                "files": [{
                    "path": ".env",
                    "zone": "secrets",
                    "attribution": "declared"
                }]
            }),
        );
        attach_tool_enrichment(
            &mut message,
            ToolEnrichment {
                references: vec![reference(ToolEnrichmentKind::Path, "src/lib.rs")],
                ..Default::default()
            },
        );

        enrich_tool_messages(std::slice::from_mut(&mut message));

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();
        assert!(enrichment.references.is_empty());
        assert!(enrichment.privacy.restricted);
        assert_eq!(message.content.content_text_only(), "raw result");
    }
}
