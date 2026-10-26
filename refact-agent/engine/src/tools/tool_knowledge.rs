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
use crate::memories::memo_record_visible_to_memory_reader;
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

        let (gcx, execution_scope, memory_reader) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                cgcx.execution_scope.clone(),
                crate::tools::tool_task_memory::related_memory_access_context(
                    cgcx.task_meta.as_ref(),
                ),
            )
        };

        let search_key = match args.get("search_key") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `search_key` is not a string: {:?}", v)),
            None => return Err("argument `search_key` is missing".to_string()),
        };

        let memories = memories_search(gcx.clone(), &search_key, 5, 0, None).await?;

        let mut seen_memids = HashSet::new();
        let mut unique_memories = Vec::new();
        for memory in memories {
            if seen_memids.insert(memory.memid.clone())
                && memo_record_visible_to_memory_reader(&memory, &memory_reader).await
            {
                unique_memories.push(memory);
            }
        }

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
                    if doc.frontmatter.is_active()
                        && !seen_memids.contains(&id)
                        && crate::tools::tool_task_memory::memory_tag_set_visible_to_reader(
                            doc.frontmatter.tags.iter().map(String::as_str),
                            doc.frontmatter
                                .extra
                                .get("namespace")
                                .and_then(serde_yaml::Value::as_str),
                            &memory_reader,
                        )
                    {
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
                format_related_memories_section(&cards, &memory_reader)
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
    use crate::app_state::AppState;
    use crate::at_commands::at_commands::AtCommandsContext;
    use crate::call_validation::ChatContent;

    async fn run_knowledge(
        gcx: Arc<crate::global_context::GlobalContext>,
        agent_id: Option<&str>,
    ) -> String {
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                "knowledge-test-chat".to_string(),
                None,
                "test-model".to_string(),
                Some(crate::chat::types::TaskMeta {
                    task_id: "task-1".to_string(),
                    role: "agents".to_string(),
                    agent_id: agent_id.map(str::to_string),
                    ..Default::default()
                }),
                None,
            )
            .await,
        ));
        let mut tool = ToolGetKnowledge {
            config_path: String::new(),
        };
        let (_, results) = tool
            .tool_execute(
                ccx,
                &"knowledge-call".to_string(),
                &HashMap::from([(
                    "search_key".to_string(),
                    Value::String("needle".to_string()),
                )]),
            )
            .await
            .unwrap();
        results
            .into_iter()
            .filter_map(|result| match result {
                ContextEnum::ChatMessage(message) => match message.content {
                    ChatContent::SimpleText(text) => Some(text),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn knowledge_results_and_expansion_hide_foreign_agent_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(crate::file_filter::KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        tokio::fs::write(
            knowledge_dir.join("private.md"),
            "---\ntitle: Agent Secret\ntags: [needle]\nnamespace: agent:A\nstatus: active\n---\n\nprivate needle body",
        )
        .await
        .unwrap();
        tokio::fs::write(
            knowledge_dir.join("shared.md"),
            "---\ntitle: Shared Knowledge\ntags: [needle]\nstatus: active\n---\n\nshared needle body",
        )
        .await
        .unwrap();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        {
            let text = tokio::fs::read_to_string(knowledge_dir.join("private.md"))
                .await
                .unwrap();
            let (frontmatter, content_start) =
                crate::knowledge_graph::kg_structs::KnowledgeFrontmatter::parse(&text);
            let mut index = gcx.knowledge_index.lock().await;
            crate::knowledge_index::add_knowledge_frontmatter_to_index(
                &mut index,
                knowledge_dir.join("private.md"),
                &frontmatter,
                Some(&text[content_start..]),
            );
            index.add_from_frontmatter(
                knowledge_dir.join("shared.md"),
                &crate::knowledge_graph::kg_structs::KnowledgeFrontmatter {
                    id: Some("shared".to_string()),
                    title: Some("Shared Knowledge".to_string()),
                    tags: vec!["needle".to_string()],
                    status: Some("active".to_string()),
                    ..Default::default()
                },
                Some("shared needle body"),
            );
            index.mark_ready();
        }

        let foreign_result = run_knowledge(gcx.clone(), Some("B")).await;
        assert!(foreign_result.contains("Shared Knowledge"));
        assert!(!foreign_result.contains("Agent Secret"), "{foreign_result}");
        assert!(!foreign_result.contains("private.md"), "{foreign_result}");
        assert!(
            !foreign_result.contains("private needle body"),
            "{foreign_result}"
        );

        let owner_result = run_knowledge(gcx, Some("A")).await;
        assert!(owner_result.contains("Agent Secret"));
        assert!(owner_result.contains("private.md"));
    }
}
