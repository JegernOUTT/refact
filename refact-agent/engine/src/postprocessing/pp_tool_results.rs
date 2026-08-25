use std::sync::Arc;
use tokenizers::Tokenizer;

pub use refact_postprocessing::pp_tool_results::ToolBudget;
use refact_core::chat_types::{ChatContent, ChatMessage, ContextFile, PostprocessSettings};
use refact_privacy::PrivacyRecord;
use refact_chat_api::{
    attach_tool_enrichment, redact_tool_enrichment, ToolEnrichment, ToolEnrichmentKind,
    ToolEnrichmentProvenance, ToolEnrichmentReference, ToolEnrichmentReferenceDetails,
};

use crate::call_validation::DiffChunk;
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
        if message.role != "tool" && message.role != "context_file" && message.role != "diff" {
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
        let truncated = message
            .extra
            .get("review_refs_truncated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        attach_tool_enrichment(
            message,
            ToolEnrichment {
                references,
                truncated,
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

fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn is_safe_mime(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'+' | b'.' | b'-'))
}

fn native_references(message: &ChatMessage) -> Vec<ToolEnrichmentReference> {
    let mut references: Vec<ToolEnrichmentReference> = Vec::new();
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
            reference.summary = result
                .get("source")
                .and_then(serde_json::Value::as_str)
                .filter(|source| is_safe_identifier(source))
                .map(|source| {
                    let span = result
                        .get("citation_span")
                        .and_then(serde_json::Value::as_u64)
                        .map(|span| format!(" · span: {span}"))
                        .unwrap_or_default();
                    format!("source: {source}{span}")
                });
            Some(reference)
        }));
    }
    if let Some(fetch) = message.extra.get("web_fetch") {
        let source = fetch.get("source").and_then(serde_json::Value::as_str);
        let status = fetch
            .get("status")
            .and_then(serde_json::Value::as_u64)
            .map(|status| status.to_string());
        let content_type = fetch
            .get("content_type")
            .and_then(serde_json::Value::as_str)
            .filter(|content_type| is_safe_mime(content_type));
        for (field, label) in [("requested_url", "requested"), ("final_url", "final")] {
            let Some(url) = fetch.get(field).and_then(serde_json::Value::as_str) else {
                continue;
            };
            let mut reference = reference(ToolEnrichmentKind::Url, url);
            reference.label = source
                .filter(|source| matches!(*source, "jina_reader" | "simple_fetch"))
                .map(|source| format!("{label} · {source}"));
            reference.status = status.clone();
            reference.summary = content_type.map(|content_type| content_type.to_string());
            references.push(reference);
        }
    }
    if let Some(browser) = message.extra.get("browser") {
        if let Some(url) = browser.get("page_url").and_then(serde_json::Value::as_str) {
            let mut reference = reference(ToolEnrichmentKind::Url, url);
            reference.label = Some("browser page".to_string());
            reference.status = browser
                .get("status")
                .and_then(serde_json::Value::as_u64)
                .map(|status| status.to_string());
            references.push(reference);
        }
        for (field, label) in [
            ("network_count", "network requests"),
            ("console_errors", "console errors"),
            ("console_warnings", "console warnings"),
        ] {
            let Some(count) = browser.get(field).and_then(serde_json::Value::as_u64) else {
                continue;
            };
            let mut reference = reference(
                ToolEnrichmentKind::Diagnostic,
                &format!("browser:{field}:{count}"),
            );
            reference.label = Some(label.to_string());
            references.push(reference);
        }
        if let Some(artifacts) = browser
            .get("artifacts")
            .and_then(serde_json::Value::as_array)
        {
            references.extend(artifacts.iter().filter_map(|artifact| {
                let id = artifact.get("id")?.as_str()?;
                let kind = artifact.get("kind")?.as_str()?;
                let mime = artifact.get("mime")?.as_str()?;
                let bytes = artifact.get("bytes")?.as_u64()?;
                if !is_safe_identifier(id) || !is_safe_identifier(kind) || !is_safe_mime(mime) {
                    return None;
                }
                let mut reference =
                    reference(ToolEnrichmentKind::Artifact, &format!("artifact:{id}"));
                reference.label = Some(kind.to_string());
                reference.summary = Some(format!("{mime} · {bytes} bytes"));
                Some(reference)
            }));
        }
    }
    if let Some(mcp) = message.extra.get("mcp") {
        let server = mcp.get("server").and_then(serde_json::Value::as_str);
        let tool = mcp.get("tool").and_then(serde_json::Value::as_str);
        let schema_hash = mcp.get("schema_hash").and_then(serde_json::Value::as_str);
        let result_type = mcp.get("result_type").and_then(serde_json::Value::as_str);
        let status = mcp.get("status").and_then(serde_json::Value::as_str);
        if let (Some(server), Some(tool), Some(schema_hash), Some(result_type), Some(status)) =
            (server, tool, schema_hash, result_type, status)
        {
            if !is_safe_identifier(server)
                || !is_safe_identifier(tool)
                || schema_hash.len() != 16
                || !schema_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                || !matches!(
                    result_type,
                    "json" | "text" | "multimodal" | "context_files"
                )
                || !matches!(status, "success" | "failed")
            {
                return references;
            }
            let mut reference = reference(
                ToolEnrichmentKind::Symbol,
                &format!("mcp::{server}::{tool}"),
            );
            reference.label = Some(format!("MCP {server}/{tool}"));
            reference.summary = Some(format!("schema {schema_hash} · {result_type}"));
            reference.status = Some(status.to_string());
            references.push(reference);
        }
    }
    for citation in &message.citations {
        let Some(url) = citation.get("url").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let mut reference = reference(ToolEnrichmentKind::Citation, url);
        reference.label = citation
            .get("title")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        if let (Some(start), Some(end)) = (
            citation
                .get("start_char_index")
                .and_then(serde_json::Value::as_u64),
            citation
                .get("end_char_index")
                .and_then(serde_json::Value::as_u64),
        ) {
            reference.summary = Some(format!("span: {start}-{end}"));
        }
        references.push(reference);
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
        let mut agent_reference = reference(ToolEnrichmentKind::Agent, agent_id);
        agent_reference.status = message
            .extra
            .get("background_agent_status")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        agent_reference.details = Some(ToolEnrichmentReferenceDetails {
            parent_chat_id: message
                .extra
                .get("background_agent_parent_chat_id")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string),
            child_chat_id: message
                .extra
                .get("child_chat_id")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string),
            result_available: message
                .extra
                .get("background_agent_result_available")
                .and_then(serde_json::Value::as_bool),
            conflict: message
                .extra
                .get("background_agent_conflict")
                .and_then(serde_json::Value::as_bool),
            ..Default::default()
        });
        references.push(agent_reference);
        for (key, status) in [("target_files", "target"), ("edited_files", "edited")] {
            if let Some(paths) = message.extra.get(key).and_then(serde_json::Value::as_array) {
                references.extend(paths.iter().filter_map(|path| {
                    let path = path.as_str()?;
                    let mut reference = reference(ToolEnrichmentKind::Path, path);
                    reference.label = Some(format!("agent {status}"));
                    reference.status = Some(status.to_string());
                    reference.details = Some(ToolEnrichmentReferenceDetails {
                        action: Some(status.to_string()),
                        ..Default::default()
                    });
                    Some(reference)
                }));
            }
        }
    }
    references.extend(diff_references(message));
    references.extend(git_references(message));
    references.extend(review_references(message));
    references
}

fn diff_references(message: &ChatMessage) -> Vec<ToolEnrichmentReference> {
    if message.role != "diff" {
        return Vec::new();
    }
    let ChatContent::SimpleText(content) = &message.content else {
        return Vec::new();
    };
    let Ok(chunks) = serde_json::from_str::<Vec<DiffChunk>>(content) else {
        return Vec::new();
    };
    let applied = message
        .extra
        .get("diff_state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let mut references: Vec<ToolEnrichmentReference> = Vec::new();
    for chunk in chunks {
        if let Some(reference) = references.iter_mut().find(|reference| {
            reference.kind == ToolEnrichmentKind::Diff && reference.target == chunk.file_name
        }) {
            if let Some(details) = reference.details.as_mut() {
                details.hunk_count = Some(details.hunk_count.unwrap_or(0).saturating_add(1));
            }
            reference.line1 = reference.line1.min(Some(chunk.line1));
            reference.line2 = reference.line2.max(Some(chunk.line2));
        } else {
            let mut reference = reference(ToolEnrichmentKind::Diff, &chunk.file_name);
            reference.status = Some(applied.to_string());
            reference.details = Some(ToolEnrichmentReferenceDetails {
                action: Some(chunk.file_action),
                rename_to: chunk.file_name_rename,
                hunk_count: Some(1),
                ..Default::default()
            });
            reference.line1 = Some(chunk.line1);
            reference.line2 = Some(chunk.line2);
            references.push(reference);
        }
    }
    references
}

fn git_references(message: &ChatMessage) -> Vec<ToolEnrichmentReference> {
    let Some(git) = message.extra.get("git") else {
        return Vec::new();
    };
    let Some(status) = git.get("status").and_then(serde_json::Value::as_str) else {
        return Vec::new();
    };
    let target = git
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("workspace");
    let mut reference = reference(ToolEnrichmentKind::Git, target);
    reference.status = Some(status.to_string());
    reference.details = Some(ToolEnrichmentReferenceDetails {
        short_sha: git
            .get("short_sha")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string),
        scope: git
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string),
        ..Default::default()
    });
    reference.truncated = git
        .get("truncated")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    vec![reference]
}

fn review_references(message: &ChatMessage) -> Vec<ToolEnrichmentReference> {
    let Some(references) = message
        .extra
        .get("review_refs")
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };
    references
        .iter()
        .filter_map(|value| {
            let path = value.get("path")?.as_str()?;
            let mut reference = reference(ToolEnrichmentKind::Review, path);
            reference.label = value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string);
            reference.status = value
                .get("severity")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string);
            reference.details = Some(ToolEnrichmentReferenceDetails {
                action: value
                    .get("evidence_kind")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string),
                scope: value
                    .get("scope")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string),
                ..Default::default()
            });
            reference.line1 = value
                .get("line1")
                .and_then(serde_json::Value::as_u64)
                .and_then(|line| usize::try_from(line).ok());
            reference.line2 = value
                .get("line2")
                .and_then(serde_json::Value::as_u64)
                .and_then(|line| usize::try_from(line).ok());
            Some(reference)
        })
        .collect()
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

    #[test]
    fn native_web_metadata_keeps_redirect_status_and_strips_url_secrets() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        message.extra.insert(
            "web_fetch".to_string(),
            serde_json::json!({
                "requested_url": "https://user:token@example.test/start?token=secret",
                "final_url": "https://example.test/final?api_key=secret",
                "status": 302,
                "content_type": "text/html",
                "source": "jina_reader",
            }),
        );

        enrich_tool_messages(std::slice::from_mut(&mut message));

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();
        assert_eq!(enrichment.references.len(), 2);
        assert_eq!(
            enrichment.references[0].target,
            "https://example.test/start"
        );
        assert_eq!(
            enrichment.references[1].target,
            "https://example.test/final"
        );
        assert!(enrichment.references.iter().all(|reference| {
            reference.status.as_deref() == Some("302")
                && reference.summary.as_deref() == Some("text/html")
        }));
        assert!(!serde_json::to_string(&enrichment)
            .unwrap()
            .contains("secret"));
    }

    #[test]
    fn native_citation_metadata_uses_url_and_span_without_cited_text() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        message.citations = vec![serde_json::json!({
            "url": "https://example.test/source?token=secret",
            "title": "Source",
            "cited_text": "secret citation body",
            "start_char_index": 4,
            "end_char_index": 12,
        })];

        enrich_tool_messages(std::slice::from_mut(&mut message));

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();
        assert_eq!(enrichment.references.len(), 1);
        assert_eq!(enrichment.references[0].kind, ToolEnrichmentKind::Citation);
        assert_eq!(
            enrichment.references[0].target,
            "https://example.test/source"
        );
        assert_eq!(
            enrichment.references[0].summary.as_deref(),
            Some("span: 4-12")
        );
        assert!(!serde_json::to_string(&enrichment)
            .unwrap()
            .contains("secret citation body"));
    }

    #[test]
    fn native_browser_metadata_uses_artifact_ids_and_never_paths_or_payloads() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        message.extra.insert(
            "browser".to_string(),
            serde_json::json!({
                "page_url": "https://example.test/dashboard?token=secret",
                "status": 200,
                "network_count": 3,
                "console_errors": 1,
                "console_warnings": 2,
                "artifacts": [{
                    "id": "image-1",
                    "kind": "pdf",
                    "mime": "application/pdf",
                    "bytes": 4096,
                    "path": "/tmp/private/report.pdf",
                    "data": "secret payload"
                }],
            }),
        );

        enrich_tool_messages(std::slice::from_mut(&mut message));

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();
        assert!(enrichment
            .references
            .iter()
            .any(|reference| reference.target == "artifact:image-1"));
        let serialized = serde_json::to_string(&enrichment).unwrap();
        assert!(!serialized.contains("/tmp/private/report.pdf"));
        assert!(!serialized.contains("secret payload"));
        assert!(!serialized.contains("token=secret"));
    }

    #[test]
    fn native_mcp_metadata_ignores_unknown_json_and_never_includes_arguments() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        message.extra.insert(
            "mcp".to_string(),
            serde_json::json!({
                "server": "github",
                "tool": "get_issue",
                "status": "success",
                "schema_hash": "0123456789abcdef",
                "result_type": "json",
                "args": {"token": "secret"},
                "response": {"password": "secret"}
            }),
        );

        enrich_tool_messages(std::slice::from_mut(&mut message));

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();
        assert_eq!(enrichment.references.len(), 1);
        assert_eq!(enrichment.references[0].target, "mcp::github::get_issue");
        assert!(!serde_json::to_string(&enrichment)
            .unwrap()
            .contains("secret"));

        message
            .extra
            .insert("mcp".to_string(), serde_json::json!({"args": "secret"}));
        message.extra.remove("tool_enrichment");
        enrich_tool_messages(std::slice::from_mut(&mut message));
        assert!(refact_chat_api::tool_enrichment_from_extra(&message.extra).is_none());
    }

    #[test]
    fn native_enrichment_keeps_diff_git_review_and_agent_metadata_thin() {
        let mut diff = ChatMessage::new(
            "diff".to_string(),
            serde_json::json!([
                {
                    "file_name": "src/old.rs",
                    "file_action": "rename",
                    "line1": 2,
                    "line2": 5,
                    "lines_remove": "private old content",
                    "lines_add": "private new content",
                    "file_name_rename": "src/new.rs",
                    "is_file": true,
                    "application_details": ""
                },
                {
                    "file_name": "src/old.rs",
                    "file_action": "rename",
                    "line1": 8,
                    "line2": 9,
                    "lines_remove": "more private content",
                    "lines_add": "more private content",
                    "file_name_rename": "src/new.rs",
                    "is_file": true,
                    "application_details": ""
                }
            ])
            .to_string(),
        );
        diff.extra
            .insert("diff_state".to_string(), serde_json::json!("applied"));
        let raw = diff.content.content_text_only();

        enrich_tool_messages(std::slice::from_mut(&mut diff));

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&diff.extra).unwrap();
        assert_eq!(enrichment.references.len(), 1);
        assert_eq!(enrichment.references[0].kind, ToolEnrichmentKind::Diff);
        assert_eq!(enrichment.references[0].status.as_deref(), Some("applied"));
        let details = enrichment.references[0].details.as_ref().unwrap();
        assert_eq!(details.action.as_deref(), Some("rename"));
        assert_eq!(details.rename_to.as_deref(), Some("src/new.rs"));
        assert_eq!(details.hunk_count, Some(2));
        assert!(!serde_json::to_string(&enrichment)
            .unwrap()
            .contains("private old content"));
        assert_eq!(diff.content.content_text_only(), raw);

        let mut metadata = ChatMessage::new("tool".to_string(), "raw result".to_string());
        metadata.extra.insert(
            "git".to_string(),
            serde_json::json!({
                "status": "available",
                "path": "workspace",
                "short_sha": "ABC1234",
                "scope": "card:T-54:stat:12",
                "diff": "must not be copied"
            }),
        );
        metadata.extra.insert(
            "review_refs".to_string(),
            serde_json::json!([{
                "id": "finding-1",
                "severity": "high",
                "path": "src/lib.rs",
                "line1": 3,
                "line2": 4,
                "evidence_kind": "excerpt",
                "scope": "abc1234",
                "content": "must not be copied"
            }]),
        );
        metadata.extra.insert(
            "background_agent_id".to_string(),
            serde_json::json!("bgagent-1"),
        );
        metadata.extra.insert(
            "background_agent_status".to_string(),
            serde_json::json!("completed"),
        );
        metadata.extra.insert(
            "background_agent_parent_chat_id".to_string(),
            serde_json::json!("parent-chat"),
        );
        metadata.extra.insert(
            "child_chat_id".to_string(),
            serde_json::json!("subchat-child"),
        );
        metadata.extra.insert(
            "background_agent_result_available".to_string(),
            serde_json::json!(true),
        );
        metadata.extra.insert(
            "background_agent_conflict".to_string(),
            serde_json::json!(false),
        );
        metadata.extra.insert(
            "target_files".to_string(),
            serde_json::json!(["src/target.rs"]),
        );
        metadata.extra.insert(
            "edited_files".to_string(),
            serde_json::json!(["src/edited.rs"]),
        );

        enrich_tool_messages(std::slice::from_mut(&mut metadata));

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&metadata.extra).unwrap();
        assert_eq!(enrichment.references.len(), 5);
        assert_eq!(enrichment.references[0].kind, ToolEnrichmentKind::Agent);
        assert_eq!(enrichment.references[1].status.as_deref(), Some("target"));
        assert_eq!(enrichment.references[2].status.as_deref(), Some("edited"));
        assert_eq!(enrichment.references[3].kind, ToolEnrichmentKind::Git);
        assert_eq!(enrichment.references[4].kind, ToolEnrichmentKind::Review);
        assert_eq!(
            enrichment.references[0]
                .details
                .as_ref()
                .unwrap()
                .parent_chat_id
                .as_deref(),
            Some("parent-chat")
        );
        assert_eq!(
            enrichment.references[0]
                .details
                .as_ref()
                .unwrap()
                .child_chat_id
                .as_deref(),
            Some("subchat-child")
        );
        assert_eq!(
            enrichment.references[3]
                .details
                .as_ref()
                .unwrap()
                .short_sha
                .as_deref(),
            Some("abc1234")
        );
        assert!(!serde_json::to_string(&enrichment)
            .unwrap()
            .contains("must not be copied"));
    }

    #[test]
    fn native_enrichment_marks_bounded_review_references_as_truncated() {
        let mut message = ChatMessage::new("tool".to_string(), "raw result".to_string());
        message.extra.insert(
            "review_refs".to_string(),
            serde_json::json!([{
                "id": "finding-1",
                "severity": "high",
                "path": "src/lib.rs",
                "line1": 3,
                "line2": 4,
                "evidence_kind": "excerpt"
            }]),
        );
        message
            .extra
            .insert("review_refs_truncated".to_string(), serde_json::json!(true));

        enrich_tool_messages(std::slice::from_mut(&mut message));

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();
        assert!(enrichment.truncated);
    }
}
