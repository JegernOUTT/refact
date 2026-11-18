use std::sync::Weak;

use crate::global_context::GlobalContext;

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

pub async fn remove_indexed_resources(gcx_weak: Weak<GlobalContext>, config_path: String) {
    let gcx = match gcx_weak.upgrade() {
        Some(g) => g,
        None => return,
    };

    let cache_dir = gcx.cache_dir.clone();

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
