use std::sync::{Arc, Weak};
use tokio::sync::Mutex as AMutex;
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
    let hash = refact_core::chunk_utils::official_text_hashing_function(uri);
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
    gcx_weak: Weak<GlobalContext>,
    config_path: String,
    peer: Peer<RoleClient>,
    resources: Vec<McpResource>,
    logs: Arc<AMutex<Vec<String>>>,
) {
    let gcx = match gcx_weak.upgrade() {
        Some(g) => g,
        None => return,
    };

    let (cache_dir, vec_db) = { (gcx.cache_dir.clone(), gcx.vec_db.clone()) };

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
                ResourceContents::BlobResourceContents {
                    ..
                } => {}
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

    let roots = crate::indexing_routing::memory_plane_roots(gcx.clone()).await;
    let db = vec_db.lock().await.clone();
    if let Some(db) = db {
        db.vectorizer_enqueue_files(&indexed_paths, false, roots)
            .await;
    }
}

pub async fn remove_indexed_resources(gcx_weak: Weak<GlobalContext>, config_path: String) {
    let gcx = match gcx_weak.upgrade() {
        Some(g) => g,
        None => return,
    };

    let (cache_dir, vec_db) = { (gcx.cache_dir.clone(), gcx.vec_db.clone()) };

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
            let db = vec_db.lock().await.clone();
            if let Some(db) = db {
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
