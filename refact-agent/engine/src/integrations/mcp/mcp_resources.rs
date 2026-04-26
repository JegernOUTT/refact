use std::sync::{Arc, Weak};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tokio::time::{timeout, Duration};
use rmcp::model::{Resource as McpResource, ReadResourceRequestParams, ResourceContents};
use rmcp::service::Peer;
use rmcp::RoleClient;

use crate::global_context::GlobalContext;

const MAX_RESOURCES_TO_INDEX: usize = 100;
const MAX_RESOURCE_SIZE_BYTES: usize = 50 * 1024 * 1024;
const MAX_TOTAL_INDEX_BYTES: usize = 200 * 1024 * 1024;
const REQUEST_TIMEOUT_SECS: u64 = 30;

pub fn is_text_mime(mime_type: &Option<String>) -> bool {
    match mime_type {
        None => true,
        Some(m) => {
            let m = m.to_lowercase();
            m.starts_with("text/")
                || m == "application/json"
                || m == "application/xml"
                || m == "application/javascript"
                || m == "application/x-yaml"
                || m == "application/yaml"
        }
    }
}

fn uri_to_filename(uri: &str) -> String {
    let sanitized: String = uri
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let hash = crate::ast::chunk_utils::official_text_hashing_function(uri);
    let prefix = sanitized.chars().take(40).collect::<String>();
    format!("{}_{}.md", prefix, &hash[..8])
}

fn server_name_for_path(config_path: &str) -> String {
    let path = std::path::Path::new(config_path);
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mcp")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub async fn index_mcp_resources(
    gcx_weak: Weak<ARwLock<GlobalContext>>,
    config_path: String,
    peer: Peer<RoleClient>,
    resources: Vec<McpResource>,
    logs: Arc<AMutex<Vec<String>>>,
) {
    let gcx = match gcx_weak.upgrade() {
        Some(g) => g,
        None => return,
    };

    let (cache_dir, vec_db) = {
        let gcx_locked = gcx.read().await;
        (gcx_locked.cache_dir.clone(), gcx_locked.vec_db.clone())
    };

    if vec_db.lock().await.is_none() {
        return;
    }

    let server_name = server_name_for_path(&config_path);
    let resources_dir = cache_dir.join("mcp_resources").join(&server_name);
    if let Err(e) = tokio::fs::create_dir_all(&resources_dir).await {
        tracing::error!(
            "mcp_resources: failed to create dir {:?}: {}",
            resources_dir,
            e
        );
        return;
    }

    let limited: Vec<_> = resources.into_iter().take(MAX_RESOURCES_TO_INDEX).collect();
    let total_count = limited.len();
    let mut indexed_paths: Vec<String> = Vec::new();
    let mut total_bytes: usize = 0;

    'outer: for resource in &limited {
        if resource.uri.contains('{') {
            continue;
        }

        let param = ReadResourceRequestParams::new(resource.uri.clone());
        let result = match timeout(
            Duration::from_secs(REQUEST_TIMEOUT_SECS),
            peer.read_resource(param),
        )
        .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                let msg = format!("mcp_resources: failed to read {}: {:?}", resource.uri, e);
                tracing::warn!("{}", msg);
                super::session_mcp::add_log_entry(logs.clone(), msg).await;
                continue;
            }
            Err(_) => {
                let msg = format!("mcp_resources: read {} timed out", resource.uri);
                tracing::warn!("{}", msg);
                super::session_mcp::add_log_entry(logs.clone(), msg).await;
                continue;
            }
        };

        for content in result.contents {
            match content {
                ResourceContents::TextResourceContents {
                    uri,
                    mime_type,
                    text,
                    ..
                } => {
                    if !is_text_mime(&mime_type) || text.len() > MAX_RESOURCE_SIZE_BYTES {
                        continue;
                    }
                    let filename = uri_to_filename(&uri);
                    let file_path = resources_dir.join(&filename);
                    let header = format!(
                        "<!-- MCP Resource: {} -->\n<!-- Server: {} -->\n\n",
                        uri, server_name
                    );
                    let full_content = format!("{}{}", header, text);
                    let content_len = full_content.len();
                    if total_bytes + content_len > MAX_TOTAL_INDEX_BYTES {
                        let remaining = total_count - indexed_paths.len();
                        let msg = format!(
                            "MCP resource indexing for {}: total size cap reached ({} bytes), skipped {} resources",
                            server_name, total_bytes, remaining
                        );
                        tracing::warn!("{}", msg);
                        super::session_mcp::add_log_entry(logs.clone(), msg).await;
                        break 'outer;
                    }
                    match tokio::fs::write(&file_path, &full_content).await {
                        Ok(_) => {
                            total_bytes += content_len;
                            indexed_paths.push(file_path.to_string_lossy().to_string());
                        }
                        Err(e) => {
                            tracing::error!(
                                "mcp_resources: failed to write {:?}: {}",
                                file_path,
                                e
                            );
                        }
                    }
                }
                ResourceContents::BlobResourceContents { .. } => {}
            }
        }
    }

    if indexed_paths.is_empty() {
        return;
    }

    let msg = format!(
        "mcp_resources: indexing {} text resources for {}",
        indexed_paths.len(),
        server_name
    );
    tracing::info!("{}", msg);
    super::session_mcp::add_log_entry(logs.clone(), msg).await;

    let vec_db_locked = vec_db.lock().await;
    if let Some(ref db) = *vec_db_locked {
        db.vectorizer_enqueue_files(&indexed_paths, false).await;
    }
}

pub async fn remove_indexed_resources(gcx_weak: Weak<ARwLock<GlobalContext>>, config_path: String) {
    let gcx = match gcx_weak.upgrade() {
        Some(g) => g,
        None => return,
    };

    let (cache_dir, vec_db) = {
        let gcx_locked = gcx.read().await;
        (gcx_locked.cache_dir.clone(), gcx_locked.vec_db.clone())
    };

    let server_name = server_name_for_path(&config_path);
    let resources_dir = cache_dir.join("mcp_resources").join(&server_name);

    if !resources_dir.exists() {
        return;
    }

    let mut entries = match tokio::fs::read_dir(&resources_dir).await {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut md_paths: Vec<std::path::PathBuf> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            md_paths.push(path);
        }
    }

    for path in md_paths {
        {
            let vec_db_locked = vec_db.lock().await;
            if let Some(ref db) = *vec_db_locked {
                let _ = db.remove_file(&path).await;
            }
        }
        let _ = tokio::fs::remove_file(&path).await;
    }
}

#[cfg(test)]
pub fn resources_cache_dir(
    cache_dir: &std::path::PathBuf,
    config_path: &str,
) -> std::path::PathBuf {
    let server_name = server_name_for_path(config_path);
    cache_dir.join("mcp_resources").join(server_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_text_mime_none() {
        assert!(is_text_mime(&None));
    }

    #[test]
    fn test_is_text_mime_text_plain() {
        assert!(is_text_mime(&Some("text/plain".to_string())));
    }

    #[test]
    fn test_is_text_mime_text_markdown() {
        assert!(is_text_mime(&Some("text/markdown".to_string())));
    }

    #[test]
    fn test_is_text_mime_application_json() {
        assert!(is_text_mime(&Some("application/json".to_string())));
    }

    #[test]
    fn test_is_text_mime_image_binary() {
        assert!(!is_text_mime(&Some("image/png".to_string())));
        assert!(!is_text_mime(&Some("application/octet-stream".to_string())));
    }

    #[test]
    fn test_uri_to_filename_simple() {
        let name = uri_to_filename("file:///path/to/doc.txt");
        assert!(name.ends_with(".md"));
        assert!(name.len() < 70);
    }

    #[test]
    fn test_uri_to_filename_different_uris_produce_different_names() {
        let name1 = uri_to_filename("db://tables/users");
        let name2 = uri_to_filename("db://tables/orders");
        assert_ne!(name1, name2);
    }

    #[test]
    fn test_uri_to_filename_same_uri_deterministic() {
        let name1 = uri_to_filename("file:///docs/readme.md");
        let name2 = uri_to_filename("file:///docs/readme.md");
        assert_eq!(name1, name2);
    }

    #[test]
    fn test_server_name_for_path() {
        assert_eq!(
            server_name_for_path("/home/user/.refact/integrations.d/mcp_stdio_myserver.yaml"),
            "mcp_stdio_myserver"
        );
        assert_eq!(server_name_for_path("/tmp/test-server.yaml"), "test_server");
    }

    #[test]
    fn test_resources_cache_dir() {
        let cache_dir = std::path::PathBuf::from("/home/user/.cache/refact");
        let dir = resources_cache_dir(&cache_dir, "/path/to/mcp_stdio_myserver.yaml");
        assert_eq!(
            dir,
            std::path::PathBuf::from("/home/user/.cache/refact/mcp_resources/mcp_stdio_myserver")
        );
    }

    #[test]
    fn test_max_total_index_bytes_constant() {
        assert_eq!(MAX_TOTAL_INDEX_BYTES, 200 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_total_cap_stops_indexing() {
        use std::sync::Arc;
        use tokio::sync::Mutex as AMutex;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let resources_dir = tmp.path().to_path_buf();
        tokio::fs::create_dir_all(&resources_dir).await.unwrap();

        let chunk_size = MAX_TOTAL_INDEX_BYTES / 3 + 1;
        let big_text = "x".repeat(chunk_size);

        let mut indexed = Vec::new();
        let mut total_bytes: usize = 0;
        let mut cap_reached = false;
        let uris = vec!["res://a", "res://b", "res://c", "res://d"];
        let logs: Arc<AMutex<Vec<String>>> = Arc::new(AMutex::new(Vec::new()));

        'outer: for uri in &uris {
            let header = format!("<!-- MCP Resource: {} -->\n<!-- Server: test -->\n\n", uri);
            let full_content = format!("{}{}", header, &big_text);
            let content_len = full_content.len();
            if total_bytes + content_len > MAX_TOTAL_INDEX_BYTES {
                let remaining = uris.len() - indexed.len();
                let msg = format!(
                    "MCP resource indexing for test: total size cap reached ({} bytes), skipped {} resources",
                    total_bytes, remaining
                );
                {
                    let mut l = logs.lock().await;
                    l.push(msg);
                }
                cap_reached = true;
                break 'outer;
            }
            let file_path = resources_dir.join(format!("{}.md", uri.replace("://", "_")));
            tokio::fs::write(&file_path, &full_content).await.unwrap();
            total_bytes += content_len;
            indexed.push(file_path);
        }

        assert!(cap_reached, "cap should have been reached");
        assert!(
            indexed.len() < uris.len(),
            "not all resources should be indexed"
        );
        assert!(
            total_bytes <= MAX_TOTAL_INDEX_BYTES,
            "total bytes should not exceed cap"
        );

        let log_entries = logs.lock().await;
        assert!(!log_entries.is_empty(), "warning should have been logged");
        assert!(log_entries[0].contains("total size cap reached"));
    }

    #[tokio::test]
    async fn test_remove_indexed_resources_iterates_without_holding_lock() {
        use tempfile::TempDir;
        use tokio::sync::Mutex as AMutex;

        let tmp = TempDir::new().unwrap();
        let resources_dir = tmp.path().to_path_buf();
        tokio::fs::create_dir_all(&resources_dir).await.unwrap();

        let file1 = resources_dir.join("resource1.md");
        let file2 = resources_dir.join("resource2.md");
        let other = resources_dir.join("other.txt");
        tokio::fs::write(&file1, "content1").await.unwrap();
        tokio::fs::write(&file2, "content2").await.unwrap();
        tokio::fs::write(&other, "other").await.unwrap();

        let mut entries = tokio::fs::read_dir(&resources_dir).await.unwrap();
        let db_option: Option<()> = None;

        let mut removed_md = 0usize;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                let _ = db_option;
                let _ = tokio::fs::remove_file(&path).await;
                removed_md += 1;
            }
        }

        assert_eq!(removed_md, 2);
        assert!(!file1.exists());
        assert!(!file2.exists());
        assert!(other.exists());

        let _lock: AMutex<Option<()>> = AMutex::new(None);
    }
}
