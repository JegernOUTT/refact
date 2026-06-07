pub use refact_knowledge_graph::kg_structs;
pub use refact_knowledge_graph::kg_structs::KnowledgeFrontmatter;
pub use refact_knowledge_graph::kg_cleanup::{KgFileDeleter, KgGraphBuilder};

pub mod kg_subchat;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use refact_knowledge_graph::kg_structs::KnowledgeGraph;

pub async fn build_knowledge_graph(gcx: Arc<GlobalContext>) -> KnowledgeGraph {
    let project_dirs = get_project_dirs(gcx.clone()).await;
    let mut knowledge_dirs: Vec<PathBuf> = project_dirs
        .iter()
        .map(|d| d.join(KNOWLEDGE_FOLDER_NAME))
        .filter(|d| d.exists())
        .collect();
    let global_dir = crate::memories::get_global_knowledge_dir(gcx.clone()).await;
    if global_dir.exists() {
        knowledge_dirs.push(global_dir);
    }
    let workspace_files = collect_workspace_files(gcx).await;
    refact_knowledge_graph::kg_builder::build_knowledge_graph(knowledge_dirs, workspace_files).await
}

pub async fn knowledge_cleanup_background_task(gcx: Arc<GlobalContext>) {
    let shutdown_flag = gcx.shutdown_flag.clone();
    let cache_dir = gcx.cache_dir.clone();
    let gcx_for_build = gcx.clone();
    let build_graph: KgGraphBuilder = Arc::new(move || {
        let gcx = gcx_for_build.clone();
        Box::pin(async move { build_knowledge_graph(gcx).await })
    });
    let gcx_for_delete = gcx.clone();
    let delete_file: KgFileDeleter = Arc::new(move |path: PathBuf| {
        let gcx = gcx_for_delete.clone();
        Box::pin(async move { crate::memories::delete_document_from_disk(gcx, &path).await })
    });
    refact_knowledge_graph::kg_cleanup::knowledge_cleanup_background_task(
        shutdown_flag,
        cache_dir,
        build_graph,
        delete_file,
    )
    .await
}
pub async fn cleanup_inactive_memories_on_startup(gcx: Arc<GlobalContext>) {
    let gcx_for_build = gcx.clone();
    let build_graph: KgGraphBuilder = Arc::new(move || {
        let gcx = gcx_for_build.clone();
        Box::pin(async move { build_knowledge_graph(gcx).await })
    });
    let gcx_for_delete = gcx.clone();
    let delete_file: KgFileDeleter = Arc::new(move |path: PathBuf| {
        let gcx = gcx_for_delete.clone();
        Box::pin(async move { crate::memories::delete_document_from_disk(gcx, &path).await })
    });
    match refact_knowledge_graph::kg_cleanup::remove_inactive_memories(&build_graph, &delete_file)
        .await
    {
        Ok(0) => tracing::info!("startup memory cleanup: no inactive memories to remove"),
        Ok(deleted) => tracing::info!(
            "startup memory cleanup: removed {} inactive (non-active) memories",
            deleted
        ),
        Err(e) => tracing::warn!("startup memory cleanup failed: {}", e),
    }
}

async fn collect_workspace_files(gcx: Arc<GlobalContext>) -> HashSet<String> {
    let project_dirs = get_project_dirs(gcx.clone()).await;
    let mut files = HashSet::new();
    for dir in project_dirs {
        let indexing =
            crate::files_blocklist::reload_indexing_everywhere_if_needed(gcx.clone()).await;
        if let Ok(paths) = crate::files_in_workspace::ls_files(&*indexing, &dir, true) {
            for path in paths {
                if let Ok(rel) = path.strip_prefix(&dir) {
                    files.insert(rel.to_string_lossy().to_string());
                }
                files.insert(path.to_string_lossy().to_string());
            }
        }
    }
    files
}
