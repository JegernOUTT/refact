use axum::Extension;
use axum::extract::Query;
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::Serialize;
use std::collections::HashMap;

use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;
use crate::knowledge_graph::build_knowledge_graph;

fn normalize_tag(tag: &str) -> String {
    tag.to_lowercase().trim().to_string()
}

#[derive(Serialize)]
struct KgNodeJson {
    id: String,
    node_type: String,
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
}

#[derive(Serialize)]
struct KgEdgeJson {
    source: String,
    target: String,
    edge_type: String,
}

#[derive(Serialize)]
struct KgStatsJson {
    doc_count: usize,
    tag_count: usize,
    file_count: usize,
    entity_count: usize,
    edge_count: usize,
    active_docs: usize,
    deprecated_docs: usize,
    trajectory_count: usize,
}

#[derive(Serialize)]
struct KnowledgeGraphJson {
    nodes: Vec<KgNodeJson>,
    edges: Vec<KgEdgeJson>,
    stats: KgStatsJson,
}

pub async fn handle_v1_knowledge_graph(
    Query(params): Query<HashMap<String, String>>,
    Extension(gcx): Extension<SharedGlobalContext>,
) -> Result<Response<Body>, ScratchError> {
    let include_content = params
        .get("include_content")
        .and_then(|v| v.parse::<u8>().ok())
        .map(|v| v != 0)
        .unwrap_or(false);

    let kg = build_knowledge_graph(gcx).await;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for (id, doc) in &kg.docs {
        let label = doc.frontmatter.title.clone().unwrap_or_else(|| {
            doc.path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| id.clone())
        });
        let node_type = match doc.frontmatter.status.as_deref() {
            Some("deprecated") => "doc_deprecated",
            Some("archived") => "doc_archived",
            _ => match doc.frontmatter.kind.as_deref() {
                Some("trajectory") => "doc_trajectory",
                Some("code") => "doc_code",
                Some("decision") => "doc_decision",
                _ => "doc",
            },
        };
        nodes.push(KgNodeJson {
            id: id.clone(),
            node_type: node_type.to_string(),
            label,
            title: doc.frontmatter.title.clone(),
            content: if include_content {
                Some(doc.content.clone())
            } else {
                None
            },
            tags: Some(doc.frontmatter.tags.clone()),
            created: doc.frontmatter.created.clone(),
            file_path: Some(doc.path.to_string_lossy().to_string()),
            kind: doc.frontmatter.kind.clone(),
        });
    }

    for (tag, _) in &kg.tag_index {
        let normalized = normalize_tag(tag);
        nodes.push(KgNodeJson {
            id: format!("tag:{}", normalized),
            node_type: "tag".to_string(),
            label: tag.clone(),
            title: None,
            content: None,
            tags: None,
            created: None,
            file_path: None,
            kind: None,
        });
    }

    for (file, _) in &kg.file_index {
        let label = std::path::Path::new(file)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| file.clone());
        nodes.push(KgNodeJson {
            id: format!("file:{}", file),
            node_type: "file".to_string(),
            label,
            title: None,
            content: None,
            tags: None,
            created: None,
            file_path: None,
            kind: None,
        });
    }

    for (entity, _) in &kg.entity_index {
        nodes.push(KgNodeJson {
            id: format!("entity:{}", entity),
            node_type: "entity".to_string(),
            label: entity.clone(),
            title: None,
            content: None,
            tags: None,
            created: None,
            file_path: None,
            kind: None,
        });
    }

    for (id, doc) in &kg.docs {
        for tag in &doc.frontmatter.tags {
            edges.push(KgEdgeJson {
                source: id.clone(),
                target: format!("tag:{}", normalize_tag(tag)),
                edge_type: "tagged_with".to_string(),
            });
        }
        for file in &doc.frontmatter.filenames {
            edges.push(KgEdgeJson {
                source: id.clone(),
                target: format!("file:{}", file),
                edge_type: "references_file".to_string(),
            });
        }
        for entity in &doc.entities {
            edges.push(KgEdgeJson {
                source: id.clone(),
                target: format!("entity:{}", entity),
                edge_type: "mentions".to_string(),
            });
        }
        for link in &doc.frontmatter.links {
            if kg.docs.contains_key(link) {
                edges.push(KgEdgeJson {
                    source: id.clone(),
                    target: link.clone(),
                    edge_type: "links_to".to_string(),
                });
            }
        }
        if let Some(superseded_by) = &doc.frontmatter.superseded_by {
            if kg.docs.contains_key(superseded_by) {
                edges.push(KgEdgeJson {
                    source: id.clone(),
                    target: superseded_by.clone(),
                    edge_type: "superseded_by".to_string(),
                });
            }
        }
    }

    let active_docs = kg
        .docs
        .values()
        .filter(|d| d.frontmatter.is_active())
        .count();
    let deprecated_docs = kg
        .docs
        .values()
        .filter(|d| d.frontmatter.is_deprecated())
        .count();
    let trajectory_count = kg
        .docs
        .values()
        .filter(|d| d.frontmatter.kind.as_deref() == Some("trajectory"))
        .count();

    let stats = KgStatsJson {
        doc_count: kg.docs.len(),
        tag_count: kg.tag_index.len(),
        file_count: kg.file_index.len(),
        entity_count: kg.entity_index.len(),
        edge_count: edges.len(),
        active_docs,
        deprecated_docs,
        trajectory_count,
    };

    let response = KnowledgeGraphJson {
        nodes,
        edges,
        stats,
    };

    let json_string = serde_json::to_string_pretty(&response).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("JSON serialization error: {}", e),
        )
    })?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json_string))
        .unwrap())
}
