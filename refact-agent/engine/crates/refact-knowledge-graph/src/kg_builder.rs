use std::collections::HashSet;
use std::path::PathBuf;
use regex::Regex;
use tracing::info;
use walkdir::WalkDir;

use super::kg_structs::{KnowledgeDoc, KnowledgeFrontmatter, KnowledgeGraph};

fn path_has_component(path: &std::path::Path, component: &str) -> bool {
    path.components().any(|c| c.as_os_str() == component)
}

fn extract_entities(content: &str) -> Vec<String> {
    let backtick_re =
        Regex::new(r"`([a-zA-Z_][a-zA-Z0-9_:]*(?:::[a-zA-Z_][a-zA-Z0-9_]*)*)`").unwrap();
    let mut entities: HashSet<String> = HashSet::new();

    for caps in backtick_re.captures_iter(content) {
        let entity = caps.get(1).unwrap().as_str().to_string();
        if entity.len() >= 3 && entity.len() <= 100 {
            entities.insert(entity);
        }
    }

    entities.into_iter().collect()
}

pub async fn build_knowledge_graph(
    knowledge_dirs: Vec<PathBuf>,
    workspace_files: HashSet<String>,
) -> KnowledgeGraph {
    let mut graph = KnowledgeGraph::new();

    if knowledge_dirs.is_empty() {
        info!("knowledge_graph: no .refact/knowledge directories found");
        return graph;
    }

    let mut doc_count = 0;

    for knowledge_dir in knowledge_dirs {
        for entry in WalkDir::new(&knowledge_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "md" && ext != "mdx" {
                continue;
            }

            if path_has_component(path, "archive") {
                continue;
            }

            let path_buf = path.to_path_buf();
            let text = match tokio::fs::read_to_string(&path_buf).await {
                Ok(t) => t,
                Err(_) => continue,
            };

            let (frontmatter, content_start) = KnowledgeFrontmatter::parse(&text);
            if frontmatter.is_archived() || frontmatter.is_deprecated() {
                continue;
            }
            let content = text[content_start..].to_string();
            let entities = extract_entities(&content);

            let mut validated_filenames = Vec::new();
            for filename in &frontmatter.filenames {
                let exists = workspace_files.contains(filename);
                if exists {
                    validated_filenames.push(filename.clone());
                }
                graph.get_or_create_file(filename, exists);
            }

            let doc = KnowledgeDoc {
                path: path_buf,
                frontmatter: KnowledgeFrontmatter {
                    filenames: validated_filenames,
                    ..frontmatter
                },
                content,
                entities,
            };

            graph.add_doc(doc);
            doc_count += 1;
        }
    }

    graph.link_docs();

    let active_count = graph
        .docs
        .values()
        .filter(|d| d.frontmatter.is_active())
        .count();
    let deprecated_count = graph
        .docs
        .values()
        .filter(|d| d.frontmatter.is_deprecated())
        .count();
    let trajectory_count = graph
        .docs
        .values()
        .filter(|d| d.frontmatter.kind.as_deref() == Some("trajectory"))
        .count();
    let code_count = graph
        .docs
        .values()
        .filter(|d| d.frontmatter.kind.as_deref() == Some("code"))
        .count();

    info!("knowledge_graph: built successfully");
    info!(
        "  Documents: {} total ({} active, {} deprecated, {} trajectories, {} code)",
        doc_count, active_count, deprecated_count, trajectory_count, code_count
    );
    info!(
        "  Tags: {}, Files: {}, Entities: {}",
        graph.tag_index.len(),
        graph.file_index.len(),
        graph.entity_index.len()
    );
    info!("  Graph edges: {}", graph.graph.edge_count());

    graph
}
