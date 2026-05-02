use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;
use chrono::Local;

use crate::buddy::memory_lifecycle::parse_memory_lifecycle_status;
use crate::custom_error::ScratchError;
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::global_context::GlobalContext;
use crate::knowledge_graph::{KnowledgeFrontmatter, build_knowledge_graph};
use crate::memories::{normalize_memory_tags, rewrite_memory_document};

pub const AUTO_LINK_MAX_LINKS: usize = 5;
pub const AUTO_LINK_MIN_SCORE: f64 = 3.0;
pub const AUTO_LINK_MAX_TOTAL: usize = 10;
const VALID_STATUSES: &[&str] = &["active", "proposed", "pinned", "archived", "deprecated"];

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

fn apply_lifecycle_status(frontmatter: &mut KnowledgeFrontmatter, status: &str) {
    frontmatter.status = Some(status.to_string());
    if matches!(status, "archived" | "deprecated") {
        if frontmatter.deprecated_at.is_none() {
            frontmatter.deprecated_at = Some(Local::now().format("%Y-%m-%d").to_string());
        }
    } else {
        frontmatter.deprecated_at = None;
        frontmatter.superseded_by = None;
    }
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

async fn get_knowledge_root(gcx: &Arc<ARwLock<GlobalContext>>) -> Result<PathBuf, ScratchError> {
    let workspace_folders = gcx.read().await.documents_state.workspace_folders.clone();
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
    let root_metadata = tokio::fs::symlink_metadata(workspace_root)
        .await
        .map_err(|_| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cannot access workspace".to_string(),
            )
        })?;
    if root_metadata.file_type().is_symlink() {
        return Err(ScratchError::new(
            StatusCode::FORBIDDEN,
            "Knowledge directory cannot be a symlink".to_string(),
        ));
    }
    if !root_metadata.is_dir() {
        return Err(ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Knowledge directory is not a directory".to_string(),
        ));
    }

    let file_metadata = tokio::fs::symlink_metadata(file_path)
        .await
        .map_err(|_| ScratchError::new(StatusCode::NOT_FOUND, "File not found".to_string()))?;
    if file_metadata.file_type().is_symlink() {
        return Err(ScratchError::new(
            StatusCode::FORBIDDEN,
            "Memory file cannot be a symlink".to_string(),
        ));
    }
    if !file_metadata.is_file() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Memory path must be a file".to_string(),
        ));
    }

    let canonical = tokio::fs::canonicalize(file_path)
        .await
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|_| ScratchError::new(StatusCode::NOT_FOUND, "File not found".to_string()))?;

    let root_canonical = tokio::fs::canonicalize(workspace_root).await.map_err(|_| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Cannot access workspace".to_string(),
        )
    })?;
    let root_canonical = dunce::simplified(&root_canonical).to_path_buf();

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

    let knowledge_root = get_knowledge_root(&gcx).await?;
    let file_path = validate_knowledge_path(Path::new(&post.file_path), &knowledge_root).await?;

    let existing_text = tokio::fs::read_to_string(&file_path).await.map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read memory file: {}", e),
        )
    })?;

    let (mut frontmatter, content_start) = KnowledgeFrontmatter::parse(&existing_text);

    if let Some(title) = post.title {
        frontmatter.title = Some(sanitize_string(&title));
    }
    if let Some(tags) = post.tags {
        let tags = sanitize_and_dedupe_strings(tags);
        frontmatter.tags = normalize_memory_tags(&tags, 16);
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
        let Some(status) = parse_memory_lifecycle_status(&status) else {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                format!(
                    "Invalid status '{}'. Must be one of: {}",
                    status,
                    VALID_STATUSES.join(", ")
                ),
            ));
        };
        apply_lifecycle_status(&mut frontmatter, &status);
    }
    frontmatter.updated = Some(Local::now().format("%Y-%m-%d").to_string());

    let existing_body = existing_text.get(content_start..).unwrap_or("").to_string();
    let content_to_write = post.content.unwrap_or(existing_body);

    let auto_link_enabled = post.auto_link.unwrap_or(true);
    if auto_link_enabled && frontmatter.is_active() {
        if let Err(e) =
            auto_link_memory(gcx.clone(), &mut frontmatter, &content_to_write, &file_path).await
        {
            tracing::warn!("Auto-linking failed: {}", e);
        }
    }

    rewrite_memory_document(gcx.clone(), &file_path, &frontmatter, &content_to_write)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Extension;
    use hyper::body::Bytes;
    use serde_json::json;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    async fn test_gcx_with_workspace(dir: &Path) -> Arc<ARwLock<GlobalContext>> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let gcx_lock = gcx.read().await;
            *gcx_lock.documents_state.workspace_folders.lock().unwrap() = vec![dir.to_path_buf()];
        }
        gcx
    }

    fn active_frontmatter(id: &str) -> KnowledgeFrontmatter {
        KnowledgeFrontmatter {
            id: Some(id.to_string()),
            title: Some(id.to_string()),
            tags: strings(&["http-test"]),
            status: Some("active".to_string()),
            kind: Some("domain".to_string()),
            ..Default::default()
        }
    }

    async fn write_memory(path: &Path, id: &str, body: &str) {
        write_memory_frontmatter(path, active_frontmatter(id), body).await;
    }

    async fn write_memory_frontmatter(path: &Path, frontmatter: KnowledgeFrontmatter, body: &str) {
        tokio::fs::write(path, format!("{}\n\n{}", frontmatter.to_yaml(), body))
            .await
            .unwrap();
    }

    async fn read_frontmatter_body(path: &Path) -> (KnowledgeFrontmatter, String) {
        let text = tokio::fs::read_to_string(path).await.unwrap();
        let (frontmatter, content_start) = KnowledgeFrontmatter::parse(&text);
        (frontmatter, text[content_start..].trim().to_string())
    }

    fn create_file_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            return std::os::unix::fs::symlink(target, link).is_ok();
        }
        #[cfg(windows)]
        {
            return std::os::windows::fs::symlink_file(target, link).is_ok();
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            false
        }
    }

    fn create_dir_symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            return std::os::unix::fs::symlink(target, link).is_ok();
        }
        #[cfg(windows)]
        {
            return std::os::windows::fs::symlink_dir(target, link).is_ok();
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            false
        }
    }

    async fn update_status(
        gcx: Arc<ARwLock<GlobalContext>>,
        path: &Path,
        status: &str,
    ) -> Result<Response<Body>, ScratchError> {
        handle_v1_knowledge_update_memory(
            Extension(gcx),
            Bytes::from(
                json!({
                    "file_path": path.to_string_lossy(),
                    "status": status,
                    "auto_link": false,
                })
                .to_string(),
            ),
        )
        .await
    }

    async fn delete_memory(
        gcx: Arc<ARwLock<GlobalContext>>,
        path: &Path,
        archive: bool,
    ) -> Result<Response<Body>, ScratchError> {
        handle_v1_knowledge_delete_memory(
            Extension(gcx),
            Bytes::from(
                json!({
                    "file_path": path.to_string_lossy(),
                    "archive": archive,
                })
                .to_string(),
            ),
        )
        .await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_status_accepts_aliases_with_canonical_frontmatter() {
        let cases = [
            ("needs-review", "proposed"),
            ("stale", "deprecated"),
            ("obsolete", "deprecated"),
            ("inactive", "archived"),
            ("archive", "archived"),
        ];

        for (alias, expected) in cases {
            let dir = tempfile::tempdir().unwrap();
            let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
            tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
            let path = knowledge_dir.join(format!("{}.md", alias.replace('-', "_")));
            let body = format!("Body for {alias}");
            write_memory(&path, alias, &body).await;
            let gcx = test_gcx_with_workspace(dir.path()).await;

            let response = update_status(gcx, &path, alias).await.unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let (frontmatter, updated_body) = read_frontmatter_body(&path).await;
            assert_eq!(frontmatter.status.as_deref(), Some(expected));
            assert_eq!(updated_body, body);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_status_rejects_invalid_status() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let path = knowledge_dir.join("invalid-status.md");
        write_memory(&path, "invalid-status", "Invalid status body").await;
        let gcx = test_gcx_with_workspace(dir.path()).await;

        let err = update_status(gcx.clone(), &path, "unknown-status")
            .await
            .unwrap_err();

        assert_eq!(err.status_code, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Invalid status"));
        let empty_err = update_status(gcx, &path, "").await.unwrap_err();
        assert_eq!(empty_err.status_code, StatusCode::BAD_REQUEST);
        let (frontmatter, body) = read_frontmatter_body(&path).await;
        assert_eq!(frontmatter.status.as_deref(), Some("active"));
        assert_eq!(body, "Invalid status body");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_rejects_symlinked_knowledge_root() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let real_knowledge = dir.path().join("real-knowledge");
        tokio::fs::create_dir_all(workspace.join(".refact"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(&real_knowledge).await.unwrap();
        let real_path = real_knowledge.join("memory.md");
        write_memory(&real_path, "memory", "Body").await;
        if !create_dir_symlink(&real_knowledge, &workspace.join(KNOWLEDGE_FOLDER_NAME)) {
            return;
        }
        let gcx = test_gcx_with_workspace(&workspace).await;
        let symlink_path = workspace.join(KNOWLEDGE_FOLDER_NAME).join("memory.md");

        let err = update_status(gcx, &symlink_path, "pinned")
            .await
            .unwrap_err();

        assert_eq!(err.status_code, StatusCode::FORBIDDEN);
        assert!(err.message.contains("symlink"));
        let (frontmatter, body) = read_frontmatter_body(&real_path).await;
        assert_eq!(frontmatter.status.as_deref(), Some("active"));
        assert_eq!(body, "Body");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn archive_delete_rejects_symlinked_knowledge_root() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let real_knowledge = dir.path().join("real-knowledge");
        tokio::fs::create_dir_all(workspace.join(".refact"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(&real_knowledge).await.unwrap();
        let real_path = real_knowledge.join("memory.md");
        write_memory(&real_path, "memory", "Body").await;
        if !create_dir_symlink(&real_knowledge, &workspace.join(KNOWLEDGE_FOLDER_NAME)) {
            return;
        }
        let gcx = test_gcx_with_workspace(&workspace).await;
        let symlink_path = workspace.join(KNOWLEDGE_FOLDER_NAME).join("memory.md");

        let err = delete_memory(gcx, &symlink_path, true).await.unwrap_err();

        assert_eq!(err.status_code, StatusCode::FORBIDDEN);
        assert!(err.message.contains("symlink"));
        let (frontmatter, body) = read_frontmatter_body(&real_path).await;
        assert_eq!(frontmatter.status.as_deref(), Some("active"));
        assert_eq!(body, "Body");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_rejects_symlink_file_inside_knowledge_root() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let outside_path = dir.path().join("outside.md");
        write_memory(&outside_path, "outside", "Outside body").await;
        let link = knowledge_dir.join("link.md");
        if !create_file_symlink(&outside_path, &link) {
            return;
        }
        let gcx = test_gcx_with_workspace(dir.path()).await;

        let err = update_status(gcx, &link, "pinned").await.unwrap_err();

        assert_eq!(err.status_code, StatusCode::FORBIDDEN);
        assert!(err.message.contains("symlink"));
        let (frontmatter, body) = read_frontmatter_body(&outside_path).await;
        assert_eq!(frontmatter.status.as_deref(), Some("active"));
        assert_eq!(body, "Outside body");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hard_delete_rejects_symlink_file_inside_knowledge_root() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let outside_path = dir.path().join("outside.md");
        write_memory(&outside_path, "outside", "Outside body").await;
        let link = knowledge_dir.join("link.md");
        if !create_file_symlink(&outside_path, &link) {
            return;
        }
        let gcx = test_gcx_with_workspace(dir.path()).await;

        let err = delete_memory(gcx, &link, false).await.unwrap_err();

        assert_eq!(err.status_code, StatusCode::FORBIDDEN);
        assert!(err.message.contains("symlink"));
        assert!(outside_path.exists());
        assert!(link.exists());
        let (frontmatter, body) = read_frontmatter_body(&outside_path).await;
        assert_eq!(frontmatter.status.as_deref(), Some("active"));
        assert_eq!(body, "Outside body");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reactivating_memory_clears_archive_metadata() {
        let cases = ["active", "proposed", "pinned"];

        for status in cases {
            let dir = tempfile::tempdir().unwrap();
            let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
            tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
            let path = knowledge_dir.join(format!("reactivate-{status}.md"));
            let mut frontmatter = active_frontmatter(status);
            frontmatter.status = Some("deprecated".to_string());
            frontmatter.deprecated_at = Some("2026-05-01".to_string());
            frontmatter.superseded_by = Some("new-memory".to_string());
            write_memory_frontmatter(&path, frontmatter, "Reactivated body").await;
            let gcx = test_gcx_with_workspace(dir.path()).await;

            let response = update_status(gcx, &path, status).await.unwrap();

            assert_eq!(response.status(), StatusCode::OK);
            let (frontmatter, body) = read_frontmatter_body(&path).await;
            assert_eq!(frontmatter.status.as_deref(), Some(status));
            assert_eq!(frontmatter.deprecated_at, None);
            assert_eq!(frontmatter.superseded_by, None);
            assert_eq!(body, "Reactivated body");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_pinned_preserves_file_and_body() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let path = knowledge_dir.join("pinned.md");
        let body = "# Pinned\n\nOriginal body";
        write_memory(&path, "pinned", body).await;
        let gcx = test_gcx_with_workspace(dir.path()).await;

        let response = update_status(gcx, &path, "pinned").await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(path.exists());
        let (frontmatter, updated_body) = read_frontmatter_body(&path).await;
        assert_eq!(frontmatter.status.as_deref(), Some("pinned"));
        assert!(frontmatter.is_active());
        assert_eq!(updated_body, body);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_proposed_preserves_file_and_body() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let path = knowledge_dir.join("proposed.md");
        let body = "# Proposed\n\nOriginal body";
        write_memory(&path, "proposed", body).await;
        let gcx = test_gcx_with_workspace(dir.path()).await;

        let response = update_status(gcx, &path, "needs-review").await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(path.exists());
        let (frontmatter, updated_body) = read_frontmatter_body(&path).await;
        assert_eq!(frontmatter.status.as_deref(), Some("proposed"));
        assert!(frontmatter.is_active());
        assert_eq!(updated_body, body);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_archived_preserves_body_and_makes_memory_inactive() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let path = knowledge_dir.join("archived.md");
        let body = "Archived body";
        write_memory(&path, "archived", body).await;
        let gcx = test_gcx_with_workspace(dir.path()).await;

        let response = update_status(gcx.clone(), &path, "archived").await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(path.exists());
        let (frontmatter, updated_body) = read_frontmatter_body(&path).await;
        assert_eq!(frontmatter.status.as_deref(), Some("archived"));
        assert!(!frontmatter.is_active());
        assert_eq!(updated_body, body);
        let kg = build_knowledge_graph(gcx.clone()).await;
        assert!(kg.active_docs().all(|doc| doc.path != path));
        let found = crate::memories::load_memories_by_tags(gcx, &["http-test"], 10)
            .await
            .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_deprecated_preserves_body_and_makes_memory_inactive() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let path = knowledge_dir.join("deprecated.md");
        let body = "Deprecated body";
        write_memory(&path, "deprecated", body).await;
        let gcx = test_gcx_with_workspace(dir.path()).await;

        let response = update_status(gcx.clone(), &path, "stale").await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(path.exists());
        let (frontmatter, updated_body) = read_frontmatter_body(&path).await;
        assert_eq!(frontmatter.status.as_deref(), Some("deprecated"));
        assert!(!frontmatter.is_active());
        assert_eq!(updated_body, body);
        let kg = build_knowledge_graph(gcx.clone()).await;
        assert!(kg.active_docs().all(|doc| doc.path != path));
        let found = crate::memories::load_memories_by_tags(gcx, &["http-test"], 10)
            .await
            .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn archive_delete_endpoint_is_non_destructive() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let path = knowledge_dir.join("archive-delete.md");
        let body = "Archive delete body";
        write_memory(&path, "archive-delete", body).await;
        let gcx = test_gcx_with_workspace(dir.path()).await;

        let response = delete_memory(gcx.clone(), &path, true).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(path.exists());
        let (frontmatter, updated_body) = read_frontmatter_body(&path).await;
        assert_eq!(frontmatter.status.as_deref(), Some("archived"));
        assert!(!frontmatter.is_active());
        assert_eq!(updated_body, body);
        let kg = build_knowledge_graph(gcx.clone()).await;
        assert!(kg.active_docs().all(|doc| doc.path != path));
        let similar = kg.find_similar_docs(&["http-test".to_string()], &[], &[]);
        assert!(similar.iter().all(|(id, _)| id != "archive-delete"));
        let found = crate::memories::load_memories_by_tags(gcx, &["http-test"], 10)
            .await
            .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hard_delete_endpoint_removes_file_only_without_archive_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        let archived_path = knowledge_dir.join("archive-delete.md");
        let hard_delete_path = knowledge_dir.join("hard-delete.md");
        write_memory(&archived_path, "archive-delete", "Archive delete body").await;
        write_memory(&hard_delete_path, "hard-delete", "Hard delete body").await;
        let gcx = test_gcx_with_workspace(dir.path()).await;

        let archive_response = delete_memory(gcx.clone(), &archived_path, true)
            .await
            .unwrap();
        let hard_delete_response = delete_memory(gcx, &hard_delete_path, false).await.unwrap();

        assert_eq!(archive_response.status(), StatusCode::OK);
        assert_eq!(hard_delete_response.status(), StatusCode::OK);
        assert!(archived_path.exists());
        assert!(!hard_delete_path.exists());
    }
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

    let knowledge_root = get_knowledge_root(&gcx).await?;
    let file_path = validate_knowledge_path(Path::new(&post.file_path), &knowledge_root).await?;

    if post.archive {
        crate::memories::update_memory_document_frontmatter(
            gcx.clone(),
            &file_path,
            |frontmatter| {
                if frontmatter.is_archived() || frontmatter.is_deprecated() {
                    return Ok(false);
                }
                apply_lifecycle_status(frontmatter, "archived");
                frontmatter.updated = Some(Local::now().format("%Y-%m-%d").to_string());
                Ok(true)
            },
        )
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to archive memory: {}", e),
            )
        })?;
        tracing::info!("Archived memory: {}", file_path.display());
    } else {
        crate::memories::delete_document_from_disk(gcx.clone(), &file_path)
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        tracing::info!("Deleted memory: {}", file_path.display());
    }

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
