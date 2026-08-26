use std::sync::Arc;
use std::collections::HashSet;
use serde_json::Value;
use tracing::info;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;
use std::collections::HashMap;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::memories::memories_search;
use crate::knowledge_graph::build_knowledge_graph;
use crate::knowledge_index::format_related_memories_section;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::native_enrichment::{workspace_roots, NativeReferences};

pub struct ToolGetKnowledge {
    pub config_path: String,
}

fn add_knowledge_references(
    references: &mut NativeReferences,
    memories: &[crate::memories::MemoRecord],
    roots: &[std::path::PathBuf],
) {
    if memories.len() > 8 {
        references.mark_truncated();
    }
    for memory in memories.iter().take(8) {
        references.add_artifact(
            &memory.memid,
            memory.title.clone(),
            memory.kind.clone(),
            Some("knowledge".to_string()),
        );
        if let Some(path) = &memory.file_path {
            let (line1, line2) = memory
                .line_range
                .map(|(line1, line2)| (line1 as usize, line2 as usize))
                .unwrap_or((0, 0));
            references.add_path(
                path,
                roots,
                line1,
                line2,
                "memory",
                memory.score,
                "knowledge",
            );
        }
    }
}

#[async_trait]
impl Tool for ToolGetKnowledge {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "knowledge".to_string(),
            display_name: "Knowledge".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Searches project knowledge base for relevant information. Uses semantic search and knowledge graph expansion.".to_string(),
            input_schema: json_schema_from_params(&[("search_key", "string", "Search query for the knowledge database.")], &["search_key"]),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        info!("knowledge search {:?}", args);

        let (gcx, execution_scope) = {
            let cgcx = ccx.lock().await;
            (cgcx.app.gcx.clone(), cgcx.execution_scope.clone())
        };

        let search_key = match args.get("search_key") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `search_key` is not a string: {:?}", v)),
            None => return Err("argument `search_key` is missing".to_string()),
        };

        let memories = memories_search(gcx.clone(), &search_key, 5, 0, None).await?;

        let mut seen_memids = HashSet::new();
        let mut unique_memories: Vec<_> = memories
            .into_iter()
            .filter(|m| seen_memids.insert(m.memid.clone()))
            .collect();

        if !unique_memories.is_empty() {
            let kg = build_knowledge_graph(gcx.clone()).await;

            let initial_ids: Vec<String> = unique_memories
                .iter()
                .filter_map(|m| m.file_path.as_ref())
                .filter_map(|p| kg.get_doc_by_path(p))
                .filter_map(|d| d.frontmatter.id.clone())
                .collect();

            let expanded_ids = kg.expand_search_results(&initial_ids, 3);

            for id in expanded_ids {
                if let Some(doc) = kg.get_doc_by_id(&id) {
                    if doc.frontmatter.is_active() && !seen_memids.contains(&id) {
                        seen_memids.insert(id.clone());
                        let snippet: String = doc.content.chars().take(500).collect();
                        unique_memories.push(crate::memories::MemoRecord {
                            memid: id,
                            tags: doc.frontmatter.tags.clone(),
                            content: snippet,
                            file_path: Some(doc.path.clone()),
                            line_range: None,
                            title: doc.frontmatter.title.clone(),
                            created: doc.frontmatter.created.clone(),
                            kind: doc.frontmatter.kind.clone(),
                            score: None, // KG expansion doesn't have scores
                        });
                    }
                }
            }
        }

        unique_memories.sort_by(|a, b| {
            let a_is_traj = a.kind.as_deref() == Some("trajectory");
            let b_is_traj = b.kind.as_deref() == Some("trajectory");
            a_is_traj.cmp(&b_is_traj)
        });

        let roots = workspace_roots(&gcx, execution_scope.as_ref());
        let mut references = NativeReferences::new();
        references.add_query(
            &search_key,
            unique_memories.len(),
            if unique_memories.is_empty() {
                "no_matches"
            } else {
                "matched"
            },
            "knowledge",
        );
        add_knowledge_references(&mut references, &unique_memories, &roots);

        let memories_str = if unique_memories.is_empty() {
            "No relevant knowledge found.".to_string()
        } else {
            let mut body: String = unique_memories
                .iter()
                .take(8)
                .map(|m| {
                    let mut result = String::new();
                    if let Some(path) = &m.file_path {
                        result.push_str(&format!("📄 {}", path.display()));
                        if let Some((start, end)) = m.line_range {
                            result.push_str(&format!(":{}-{}", start, end));
                        }
                        result.push('\n');
                    }
                    if let Some(title) = &m.title {
                        result.push_str(&format!("📌 {}\n", title));
                    }
                    if let Some(kind) = &m.kind {
                        result.push_str(&format!("📦 {}\n", kind));
                    }
                    if !m.tags.is_empty() {
                        result.push_str(&format!("🏷️ {}\n", m.tags.join(", ")));
                    }
                    result.push_str(&m.content);
                    result.push_str("\n\n---\n");
                    result
                })
                .collect();

            // Add a short-form related memories section (fast, in-memory).
            let related = {
                let gcx_read = gcx.clone();
                let idx_guard = gcx_read.knowledge_index.lock().await;
                let mut tags: Vec<String> = unique_memories
                    .iter()
                    .flat_map(|m| m.tags.iter().cloned())
                    .collect();
                tags.sort();
                tags.dedup();
                let cards = idx_guard.related_for_tags(&tags, 8);
                format_related_memories_section(&cards, None)
            };

            body.push_str(&related);
            body.push_str("\n\nNote: the related memories section is heuristic; fetch full content if needed.");
            body
        };

        Ok((
            false,
            vec![ContextEnum::ChatMessage({
                let mut message = ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(memories_str),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    output_filter: Some(OutputFilter::no_limits()),
                    ..Default::default()
                };
                crate::privacy::load_privacy_if_needed(gcx.clone()).await;
                let records = crate::privacy::records::declared_file_records(
                    &gcx,
                    unique_memories
                        .iter()
                        .filter_map(|memory| memory.file_path.clone()),
                )?;
                crate::privacy::records::merge_records(&mut message, records);
                references.attach(&mut message);
                message
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec!["knowledge".to_string()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_references_use_native_query_artifact_and_workspace_path_metadata() {
        let memory = crate::memories::MemoRecord {
            memid: "memory-1".to_string(),
            file_path: Some(crate::test_paths::abs(
                "workspace/.refact/knowledge/memory.md",
            )),
            line_range: Some((3, 5)),
            title: Some("Memory title".to_string()),
            kind: Some("decision".to_string()),
            score: Some(0.8),
            ..Default::default()
        };
        let mut references = NativeReferences::new();
        references.add_query("remember this", 1, "matched", "knowledge");
        add_knowledge_references(
            &mut references,
            &[memory],
            &[crate::test_paths::abs("workspace")],
        );
        let mut message = ChatMessage::new("tool".to_string(), "raw knowledge".to_string());
        references.attach(&mut message);

        let enrichment = refact_chat_api::tool_enrichment_from_extra(&message.extra).unwrap();
        assert_eq!(enrichment.references.len(), 3);
        assert_eq!(enrichment.references[0].target, "remember this");
        assert_eq!(enrichment.references[1].target, "artifact:memory-1");
        assert_eq!(
            enrichment.references[2].target,
            ".refact/knowledge/memory.md"
        );
        assert_eq!(enrichment.references[2].line1, Some(3));
        assert_eq!(enrichment.references[2].line2, Some(5));
        assert_eq!(message.content.content_text_only(), "raw knowledge");
    }
}
