use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;

pub use refact_ext::config_dirs::{
    collect_md_files_recursive, is_claude_dir, source_for_dir, CommandSource, ExtDirs,
};

pub async fn get_ext_dirs(gcx: Arc<ARwLock<GlobalContext>>) -> ExtDirs {
    let config_dir = gcx.read().await.config_dir.clone();
    let workspace_dirs = get_project_dirs(gcx.clone()).await;

    let mut global_dirs = Vec::new();
    if let Some(home) = home::home_dir() {
        global_dirs.push(home.join(".claude"));
    }
    global_dirs.push(config_dir.clone());

    let mut installed_dirs = Vec::new();
    let installed_root = config_dir.join("plugins").join("installed");
    if let Ok(mut entries) = tokio::fs::read_dir(&installed_root).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                installed_dirs.push(path);
            }
        }
    }
    installed_dirs.sort();

    let mut project_dirs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in &workspace_dirs {
        if seen.insert(dir.clone()) {
            project_dirs.push(dir.join(".claude"));
            project_dirs.push(dir.join(".refact"));
        }
    }

    ExtDirs {
        global_dirs,
        installed_dirs,
        project_dirs,
    }
}
