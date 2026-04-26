use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::io::Write;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use tokio::fs;
use chrono::Local;
use tempfile::NamedTempFile;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::knowledge_graph::{KnowledgeFrontmatter, build_knowledge_graph};
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;

pub const AUTO_LINK_MAX_LINKS: usize = 5;
pub const AUTO_LINK_MIN_SCORE: f64 = 3.0;
pub const AUTO_LINK_MAX_TOTAL: usize = 10;
const VALID_STATUSES: &[&str] = &["active", "deprecated", "archived"];

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

fn sanitize_string(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '\n' && *c != '\r')
        .collect::<String>()
        .trim()
        .to_string()
}

fn sanitize_and_dedupe_strings(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .map(|s| sanitize_string(&s))
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}

#[derive(Deserialize)]
pub struct UpdateMemoryPost {
    pub file_path: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub filenames: Option<Vec<String>>,
    #[serde(default)]
    pub links: Option<Vec<String>>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub auto_link: Option<bool>,
}

#[derive(Deserialize)]
pub struct DeleteMemoryPost {
    pub file_path: String,
    #[serde(default)]
    pub archive: bool,
}

#[derive(Serialize)]
pub struct MemoryOperationResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn auto_link_memory(
    gcx: Arc<ARwLock<GlobalContext>>,
    frontmatter: &mut KnowledgeFrontmatter,
    content: &str,
    doc_path: &Path,
) -> Result<(), String> {
    let entities = extract_entities(content);
    let kg = build_knowledge_graph(gcx.clone()).await;
    let similar_docs = kg.find_similar_docs(&frontmatter.tags, &frontmatter.filenames, &entities);

    let doc_id = frontmatter
        .id
        .clone()
        .unwrap_or_else(|| doc_path.to_string_lossy().to_string());

    let suggested_links: Vec<String> = similar_docs
        .into_iter()
        .filter(|(id, score)| {
            if *score < AUTO_LINK_MIN_SCORE || *id == doc_id {
                return false;
            }
            if let Some(doc) = kg.docs.get(id) {
                if !doc.frontmatter.is_active() {
                    return false;
                }
            }
            true
        })
        .take(AUTO_LINK_MAX_LINKS)
        .map(|(id, _)| id)
        .collect();

    for link in suggested_links {
        if frontmatter.links.len() >= AUTO_LINK_MAX_TOTAL {
            break;
        }
        if !frontmatter.links.contains(&link) {
            frontmatter.links.push(link);
        }
    }

    frontmatter.links.retain(|link| link != &doc_id);

    Ok(())
}

fn get_knowledge_root(gcx: &Arc<ARwLock<GlobalContext>>) -> Result<PathBuf, ScratchError> {
    let workspace_folders = gcx
        .blocking_read()
        .documents_state
        .workspace_folders
        .clone();
    let folders = workspace_folders.lock().unwrap();

    if folders.is_empty() {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No workspace folder configured".to_string(),
        ));
    }

    Ok(folders[0].join(KNOWLEDGE_FOLDER_NAME))
}

async fn validate_knowledge_path(
    file_path: &Path,
    workspace_root: &Path,
) -> Result<PathBuf, ScratchError> {
    let canonical = tokio::fs::canonicalize(file_path)
        .await
        .map_err(|_| ScratchError::new(StatusCode::NOT_FOUND, "File not found".to_string()))?;

    let root_canonical = tokio::fs::canonicalize(workspace_root).await.map_err(|_| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot access workspace".to_string(),
        )
    })?;

    if !canonical.starts_with(&root_canonical) {
        return Err(ScratchError::new(
            StatusCode::FORBIDDEN,
            "Path outside knowledge directory".to_string(),
        ));
    }

    let ext = canonical.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext != "md" && ext != "mdx" {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Only .md and .mdx files allowed".to_string(),
        ));
    }

    Ok(canonical)
}

pub async fn handle_v1_knowledge_update_memory(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<UpdateMemoryPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let knowledge_root = get_knowledge_root(&gcx)?;
    let file_path = validate_knowledge_path(Path::new(&post.file_path), &knowledge_root).await?;

    let existing_text = get_file_text_from_memory_or_disk(gcx.clone(), &file_path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let (mut frontmatter, content_start) = KnowledgeFrontmatter::parse(&existing_text);

    if let Some(title) = post.title {
        frontmatter.title = Some(sanitize_string(&title));
    }
    if let Some(tags) = post.tags {
        frontmatter.tags = sanitize_and_dedupe_strings(tags);
    }
    if let Some(kind) = post.kind {
        let kind = sanitize_string(&kind);
        if !kind.is_empty() {
            frontmatter.kind = Some(kind);
        }
    }
    if let Some(filenames) = post.filenames {
        frontmatter.filenames = sanitize_and_dedupe_strings(filenames);
    }
    if let Some(links) = post.links {
        frontmatter.links = sanitize_and_dedupe_strings(links);
    }
    if let Some(status) = post.status {
        let status = sanitize_string(&status);
        if !status.is_empty() {
            if !VALID_STATUSES.contains(&status.as_str()) {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "Invalid status '{}'. Must be one of: {}",
                        status,
                        VALID_STATUSES.join(", ")
                    ),
                ));
            }
            frontmatter.status = Some(status);
        }
    }
    frontmatter.updated = Some(Local::now().format("%Y-%m-%d").to_string());

    let existing_body = existing_text.get(content_start..).unwrap_or("").to_string();
    let content_to_write = post.content.unwrap_or(existing_body);

    let auto_link_enabled = post.auto_link.unwrap_or(true);
    if auto_link_enabled {
        if let Err(e) =
            auto_link_memory(gcx.clone(), &mut frontmatter, &content_to_write, &file_path).await
        {
            tracing::warn!("Auto-linking failed: {}", e);
        }
    }

    let new_content = format!("{}\n\n{}", frontmatter.to_yaml(), content_to_write.trim());

    let dir = file_path
        .parent()
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::BAD_REQUEST,
                "Invalid file path: no parent directory".to_string(),
            )
        })?
        .to_path_buf();

    let file_path_clone = file_path.clone();
    tokio::task::spawn_blocking(move || {
        let mut tmp_file = NamedTempFile::new_in(&dir).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create temporary file: {}", e),
            )
        })?;

        tmp_file.write_all(new_content.as_bytes()).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to write temporary file: {}", e),
            )
        })?;

        tmp_file.flush().map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to flush temporary file: {}", e),
            )
        })?;

        tmp_file.persist(&file_path_clone).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update memory file: {}", e),
            )
        })?;

        Ok::<(), ScratchError>(())
    })
    .await
    .map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task join error: {}", e),
        )
    })??;

    let vec_db = gcx.read().await.vec_db.clone();
    if let Some(vecdb) = vec_db.lock().await.as_ref() {
        vecdb
            .vectorizer_enqueue_files(&vec![file_path.to_string_lossy().to_string()], true)
            .await;
    }

    gcx.write()
        .await
        .documents_state
        .memory_document_map
        .remove(&file_path);

    tracing::info!("Updated memory: {}", file_path.display());

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::to_string(&MemoryOperationResponse {
                success: true,
                error: None,
            })
            .unwrap(),
        ))
        .unwrap())
}

pub async fn handle_v1_knowledge_delete_memory(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<DeleteMemoryPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let knowledge_root = get_knowledge_root(&gcx)?;
    let file_path = validate_knowledge_path(Path::new(&post.file_path), &knowledge_root).await?;

    if post.archive {
        crate::memories::archive_document(gcx.clone(), &file_path)
            .await
            .map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to archive memory: {}", e),
                )
            })?;
        tracing::info!("Archived memory: {}", file_path.display());
    } else {
        fs::remove_file(&file_path).await.map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to delete memory file: {}", e),
            )
        })?;
        tracing::info!("Deleted memory: {}", file_path.display());
    }

    let vec_db = gcx.read().await.vec_db.clone();
    if let Some(vecdb) = vec_db.lock().await.as_ref() {
        vecdb
            .vectorizer_enqueue_files(&vec![file_path.to_string_lossy().to_string()], true)
            .await;
    }

    gcx.write()
        .await
        .documents_state
        .memory_document_map
        .remove(&file_path);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::to_string(&MemoryOperationResponse {
                success: true,
                error: None,
            })
            .unwrap(),
        ))
        .unwrap())
}
