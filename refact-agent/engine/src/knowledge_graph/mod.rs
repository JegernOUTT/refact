pub use refact_knowledge_graph::kg_structs;
pub use refact_knowledge_graph::kg_structs::KnowledgeFrontmatter;
pub use refact_knowledge_graph::kg_cleanup::{KgFileDeleter, KgGraphBuilder};

pub mod kg_subchat;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use refact_knowledge_graph::kg_structs::KnowledgeGraph;
use tokio::sync::Mutex as AMutex;

type CachedKnowledgeGraph = (Arc<AtomicU64>, u64, Arc<KnowledgeGraph>);
type KnowledgeGraphCache = AMutex<Option<CachedKnowledgeGraph>>;

static KNOWLEDGE_GRAPH_CACHE: OnceLock<KnowledgeGraphCache> = OnceLock::new();

fn knowledge_graph_cache() -> &'static KnowledgeGraphCache {
    KNOWLEDGE_GRAPH_CACHE.get_or_init(|| AMutex::new(None))
}

pub async fn build_knowledge_graph(gcx: Arc<GlobalContext>) -> Arc<KnowledgeGraph> {
    build_knowledge_graph_with_cache(gcx, knowledge_graph_cache()).await
}

async fn build_knowledge_graph_with_cache(
    gcx: Arc<GlobalContext>,
    cache: &KnowledgeGraphCache,
) -> Arc<KnowledgeGraph> {
    // The lock is intentionally held across the build await: it makes the rebuild a
    // singleflight so concurrent callers cannot stampede into parsing every knowledge doc.
    let mut cache = cache.lock().await;
    let counter = gcx.knowledge_generation.clone();
    let generation = counter.load(Ordering::SeqCst);
    if let Some((cached_counter, cached_generation, graph)) = cache.as_ref() {
        if Arc::ptr_eq(cached_counter, &counter) && *cached_generation == generation {
            return graph.clone();
        }
    }
    let graph = Arc::new(build_knowledge_graph_uncached(gcx).await);
    *cache = Some((counter, generation, graph.clone()));
    graph
}

async fn build_knowledge_graph_uncached(gcx: Arc<GlobalContext>) -> KnowledgeGraph {
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
        Box::pin(async move { build_knowledge_graph_uncached(gcx).await })
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
        Box::pin(async move { build_knowledge_graph_uncached(gcx).await })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_context::tests::make_test_gcx;

    async fn gcx_with_one_knowledge_doc() -> (tempfile::TempDir, Arc<GlobalContext>) {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        tokio::fs::write(
            knowledge_dir.join("cached.md"),
            "---\nid: \"cached-doc\"\nstatus: active\ntags: [\"kg-cache\"]\n---\n\nCached body",
        )
        .await
        .unwrap();
        let gcx = make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }
        (dir, gcx)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn knowledge_graph_cache_returns_same_arc_without_mutation() {
        let (_dir, gcx) = gcx_with_one_knowledge_doc().await;
        let cache: KnowledgeGraphCache = AMutex::new(None);

        let first = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;
        let second = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;

        assert!(Arc::ptr_eq(&first, &second));
        assert!(first.docs.contains_key("cached-doc"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn knowledge_graph_cache_rebuilds_after_generation_bump() {
        let (_dir, gcx) = gcx_with_one_knowledge_doc().await;
        let cache: KnowledgeGraphCache = AMutex::new(None);

        let first = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;
        gcx.knowledge_generation.fetch_add(1, Ordering::SeqCst);
        let second = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;

        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(first.docs.len(), second.docs.len());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn knowledge_graph_cache_ignores_unrelated_enrichment_bumps() {
        let (_dir, gcx) = gcx_with_one_knowledge_doc().await;
        let cache: KnowledgeGraphCache = AMutex::new(None);

        let first = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;
        gcx.enrichment_generation.fetch_add(1, Ordering::SeqCst);
        let second = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;
        assert!(Arc::ptr_eq(&first, &second));

        gcx.knowledge_generation.fetch_add(1, Ordering::SeqCst);
        let third = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;
        assert!(!Arc::ptr_eq(&second, &third));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn knowledge_graph_cache_rebuilds_after_memories_add() {
        let (dir, gcx) = gcx_with_one_knowledge_doc().await;
        let cache: KnowledgeGraphCache = AMutex::new(None);

        let first = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;
        let mut frontmatter = KnowledgeFrontmatter::default();
        frontmatter.id = Some("added-doc".to_string());
        frontmatter.status = Some("active".to_string());
        frontmatter.tags = vec!["kg-cache".to_string()];
        crate::memories::memories_add(gcx.clone(), &frontmatter, "Added by memories_add")
            .await
            .unwrap();
        let second = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;

        assert!(!Arc::ptr_eq(&first, &second));
        assert!(second.docs.contains_key("added-doc"));
        drop(dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn knowledge_graph_cache_does_not_leak_across_generations() {
        let (_dir, gcx) = gcx_with_one_knowledge_doc().await;
        let cache: KnowledgeGraphCache = AMutex::new(None);

        let first = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;
        assert_eq!(cache.lock().await.as_ref().unwrap().1, 0);
        gcx.knowledge_generation.fetch_add(1, Ordering::SeqCst);
        let second = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;
        assert_eq!(cache.lock().await.as_ref().unwrap().1, 1);
        let third = build_knowledge_graph_with_cache(gcx.clone(), &cache).await;

        assert!(!Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&second, &third));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn knowledge_graph_cache_does_not_leak_across_global_contexts() {
        let (_first_dir, first_gcx) = gcx_with_one_knowledge_doc().await;
        let (_second_dir, second_gcx) = gcx_with_one_knowledge_doc().await;
        let cache: KnowledgeGraphCache = AMutex::new(None);

        let first = build_knowledge_graph_with_cache(first_gcx.clone(), &cache).await;
        let second = build_knowledge_graph_with_cache(second_gcx.clone(), &cache).await;

        assert!(!Arc::ptr_eq(&first, &second));
    }
}
