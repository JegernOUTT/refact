use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::io::Write;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use tokio::fs;
use chrono::Local;
use tempfile::NamedTempFile;

use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;
use crate::knowledge_graph::KnowledgeFrontmatter;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;

#[derive(Deserialize)]
pub struct UpdateMemoryPost {
    pub file_path: String,
    #[serde(default)]
    pub title: Option<String>,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub filenames: Vec<String>,
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

fn get_knowledge_root(gcx: &Arc<ARwLock<GlobalContext>>) -> Result<PathBuf, ScratchError> {
    let workspace_folders = gcx.blocking_read().documents_state.workspace_folders.clone();
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
    let canonical = tokio::fs::canonicalize(file_path).await.map_err(|_| {
        ScratchError::new(
            StatusCode::NOT_FOUND,
            "File not found".to_string(),
        )
    })?;
    
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
    
    if canonical.extension().map(|e| e != "md").unwrap_or(true) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Only .md files allowed".to_string(),
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
    let file_path = validate_knowledge_path(
        Path::new(&post.file_path),
        &knowledge_root,
    ).await?;

    let existing_text = get_file_text_from_memory_or_disk(gcx.clone(), &file_path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let (mut frontmatter, _) = KnowledgeFrontmatter::parse(&existing_text);

    if let Some(title) = post.title {
        frontmatter.title = Some(title);
    }
    if !post.tags.is_empty() {
        frontmatter.tags = post.tags;
    }
    if let Some(kind) = post.kind {
        frontmatter.kind = Some(kind);
    }
    if !post.filenames.is_empty() {
        frontmatter.filenames = post.filenames;
    }
    frontmatter.updated = Some(Local::now().format("%Y-%m-%d").to_string());

    let new_content = format!("{}\n\n{}", frontmatter.to_yaml(), post.content.trim());

    let dir = file_path.parent().ok_or_else(|| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid file path: no parent directory".to_string(),
        )
    })?.to_path_buf();

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

    if let Some(vecdb) = gcx.read().await.vec_db.lock().await.as_ref() {
        vecdb
            .vectorizer_enqueue_files(&vec![file_path.to_string_lossy().to_string()], true)
            .await;
    }

    gcx.write().await.documents_state.memory_document_map.remove(&file_path);

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
    let file_path = validate_knowledge_path(
        Path::new(&post.file_path),
        &knowledge_root,
    ).await?;

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
        fs::remove_file(&file_path)
            .await
            .map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to delete memory file: {}", e),
                )
            })?;
        tracing::info!("Deleted memory: {}", file_path.display());
    }

    if let Some(vecdb) = gcx.read().await.vec_db.lock().await.as_ref() {
        vecdb
            .vectorizer_enqueue_files(&vec![file_path.to_string_lossy().to_string()], true)
            .await;
    }

    gcx.write().await.documents_state.memory_document_map.remove(&file_path);

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
