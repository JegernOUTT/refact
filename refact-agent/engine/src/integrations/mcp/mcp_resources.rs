use std::path::PathBuf;
use std::sync::{Arc, Weak};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tokio::time::{timeout, Duration};
use rmcp::model::{Resource as McpResource, ReadResourceRequestParam, ResourceContents};
use rmcp::service::Peer;
use rmcp::RoleClient;

use crate::global_context::GlobalContext;

const MAX_RESOURCES_TO_INDEX: usize = 100;
const MAX_RESOURCE_SIZE_BYTES: usize = 50 * 1024 * 1024;
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
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
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
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
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
        tracing::error!("mcp_resources: failed to create dir {:?}: {}", resources_dir, e);
        return;
    }

    let limited: Vec<_> = resources.into_iter().take(MAX_RESOURCES_TO_INDEX).collect();
    let mut indexed_paths: Vec<String> = Vec::new();

    for resource in &limited {
        if resource.uri.contains('{') {
            continue;
        }

        let param = ReadResourceRequestParam { uri: resource.uri.clone() };
        let result = match timeout(
            Duration::from_secs(REQUEST_TIMEOUT_SECS),
            peer.read_resource(param),
        ).await {
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
                ResourceContents::TextResourceContents { uri, mime_type, text } => {
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
                    match tokio::fs::write(&file_path, &full_content).await {
                        Ok(_) => {
                            indexed_paths.push(file_path.to_string_lossy().to_string());
                        }
                        Err(e) => {
                            tracing::error!("mcp_resources: failed to write {:?}: {}", file_path, e);
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

    let msg = format!("mcp_resources: indexing {} text resources for {}", indexed_paths.len(), server_name);
    tracing::info!("{}", msg);
    super::session_mcp::add_log_entry(logs.clone(), msg).await;

    let vec_db_locked = vec_db.lock().await;
    if let Some(ref db) = *vec_db_locked {
        db.vectorizer_enqueue_files(&indexed_paths, false).await;
    }
}

pub async fn remove_indexed_resources(
    gcx_weak: Weak<ARwLock<GlobalContext>>,
    config_path: &str,
) {
    let gcx = match gcx_weak.upgrade() {
        Some(g) => g,
        None => return,
    };

    let (cache_dir, vec_db) = {
        let gcx_locked = gcx.read().await;
        (gcx_locked.cache_dir.clone(), gcx_locked.vec_db.clone())
    };

    let server_name = server_name_for_path(config_path);
    let resources_dir = cache_dir.join("mcp_resources").join(&server_name);

    if !resources_dir.exists() {
        return;
    }

    let mut entries = match tokio::fs::read_dir(&resources_dir).await {
        Ok(e) => e,
        Err(_) => return,
    };

    let vec_db_locked = vec_db.lock().await;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            if let Some(ref db) = *vec_db_locked {
                let _ = db.remove_file(&path).await;
            }
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
}

pub fn resources_cache_dir(cache_dir: &PathBuf, config_path: &str) -> PathBuf {
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
        assert_eq!(server_name_for_path("/home/user/.refact/integrations.d/mcp_stdio_myserver.yaml"), "mcp_stdio_myserver");
        assert_eq!(server_name_for_path("/tmp/test-server.yaml"), "test_server");
    }

    #[test]
    fn test_resources_cache_dir() {
        let cache_dir = PathBuf::from("/home/user/.cache/refact");
        let dir = resources_cache_dir(&cache_dir, "/path/to/mcp_stdio_myserver.yaml");
        assert_eq!(dir, PathBuf::from("/home/user/.cache/refact/mcp_resources/mcp_stdio_myserver"));
    }
}
