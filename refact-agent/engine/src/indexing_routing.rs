use std::path::Path;
use std::sync::Arc;

use refact_core::memory_plane::MemoryPlaneRoots;

use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;

pub async fn memory_plane_roots(gcx: Arc<GlobalContext>) -> MemoryPlaneRoots {
    MemoryPlaneRoots::new(
        get_project_dirs(gcx.clone()).await,
        Some(crate::memories::get_global_knowledge_dir(gcx.clone()).await),
        Some(crate::chat::trajectories::get_global_trajectories_dir(gcx).await),
    )
}

pub fn partition_paths(paths: &[String], roots: &MemoryPlaneRoots) -> (Vec<String>, Vec<String>) {
    let mut memory_paths = Vec::new();
    let mut code_paths = Vec::new();
    for path in paths {
        if roots.classify_file(Path::new(path)).is_some() {
            memory_paths.push(path.clone());
        } else {
            code_paths.push(path.clone());
        }
    }
    (memory_paths, code_paths)
}

pub async fn route_index_enqueue(
    gcx: Arc<GlobalContext>,
    paths: &[String],
    process_immediately: bool,
    vecdb_only: bool,
) {
    let paths: Vec<String> = paths
        .iter()
        .filter(|path| !crate::file_filter::is_transient_tmp_path(Path::new(path.as_str())))
        .cloned()
        .collect();
    if paths.is_empty() {
        return;
    }

    let roots = memory_plane_roots(gcx.clone()).await;
    let (memory_paths, code_paths) = partition_paths(&paths, &roots);

    let vec_db = gcx.vec_db.clone();

    if !memory_paths.is_empty() {
        if let Some(ref mut db) = *vec_db.lock().await {
            db.vectorizer_enqueue_files(&memory_paths, process_immediately, roots.clone())
                .await;
        }
    }

    if vecdb_only {
        return;
    }

    if !code_paths.is_empty() {
        let codegraph = gcx.codegraph.lock().await.clone();
        match codegraph {
            Some(service) => service.enqueue_files(&code_paths),
            None => {
                tracing::warn!(
                    "codegraph unavailable; skipping {} code file(s) (memory-plane vec_db never receives code)",
                    code_paths.len()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vecdb::vdb_structs::{
        EmbeddingModelConfig, SearchResult, VecDbStatus, VecdbRecord, VecdbSearch,
    };
    use async_trait::async_trait;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct RecordingVecdb {
        enqueue_calls: Arc<AtomicUsize>,
        documents: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl VecdbSearch for RecordingVecdb {
        async fn vecdb_search(
            &self,
            query: String,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<SearchResult, String> {
            Ok(SearchResult {
                query_text: query,
                results: vec![],
            })
        }

        async fn get_status(&self) -> Result<VecDbStatus, String> {
            Ok(VecDbStatus {
                files_unprocessed: 0,
                files_total: 0,
                requests_made_since_start: 0,
                vectors_made_since_start: 0,
                db_size: 0,
                db_cache_size: 0,
                state: "done".to_string(),
                queue_additions: false,
                vecdb_max_files_hit: false,
                vecdb_errors: Default::default(),
            })
        }

        async fn remove_file(&self, _file_path: &PathBuf) -> Result<(), String> {
            Ok(())
        }

        async fn vectorizer_enqueue_files(
            &self,
            documents: &[String],
            _process_immediately: bool,
            _roots: refact_core::memory_plane::MemoryPlaneRoots,
        ) {
            self.enqueue_calls
                .fetch_add(documents.len(), Ordering::SeqCst);
            self.documents.lock().unwrap().extend(documents.to_vec());
        }

        fn current_constants(&self) -> (EmbeddingModelConfig, usize) {
            (
                EmbeddingModelConfig {
                    endpoint: String::new(),
                    endpoint_style: String::new(),
                    embedding_endpoint_style: String::new(),
                    api_key: String::new(),
                    model_name: String::new(),
                    embedding_size: 0,
                    dimensions: None,
                    query_prefix: String::new(),
                    document_prefix: String::new(),
                    rejection_threshold: 0.0,
                    embedding_batch: 1,
                    n_ctx: 0,
                },
                0,
            )
        }

        async fn embed_query(&self, _query: &str) -> Result<Vec<f32>, String> {
            Ok(vec![])
        }

        async fn vecdb_search_with_embedding(
            &self,
            _embedding: &Vec<f32>,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<Vec<VecdbRecord>, String> {
            Ok(vec![])
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn partitions_memory_and_code_paths() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }

        let project_root = get_project_dirs(gcx.clone())
            .await
            .into_iter()
            .next()
            .expect("test workspace must resolve to a project dir");
        let roots = memory_plane_roots(gcx.clone()).await;

        let knowledge = project_root
            .join(".refact")
            .join("knowledge")
            .join("note.md")
            .to_string_lossy()
            .to_string();
        let trajectory = project_root
            .join(".refact")
            .join("trajectories")
            .join("abc.json")
            .to_string_lossy()
            .to_string();
        let task_trajectory = project_root
            .join(".refact")
            .join("tasks")
            .join("T-1")
            .join("trajectories")
            .join("planner")
            .join("chat.json")
            .to_string_lossy()
            .to_string();
        let knowledge_source = project_root
            .join(".refact")
            .join("knowledge")
            .join("source.rs")
            .to_string_lossy()
            .to_string();
        let task_trajectory_tool = project_root
            .join(".refact")
            .join("tasks")
            .join("T-1")
            .join("trajectories")
            .join("tool.py")
            .to_string_lossy()
            .to_string();
        let task_memory = project_root
            .join(".refact")
            .join("tasks")
            .join("T-1")
            .join("memories")
            .join("note.md")
            .to_string_lossy()
            .to_string();
        let non_refact_task_trajectory = project_root
            .join("src")
            .join("tasks")
            .join("T-1")
            .join("trajectories")
            .join("chat.json")
            .to_string_lossy()
            .to_string();
        let task_meta = project_root
            .join(".refact")
            .join("tasks")
            .join("T-1")
            .join("meta.yaml")
            .to_string_lossy()
            .to_string();
        let code = project_root
            .join("src")
            .join("main.rs")
            .to_string_lossy()
            .to_string();

        let (memory_paths, code_paths) = partition_paths(
            &[
                knowledge.clone(),
                trajectory.clone(),
                task_trajectory.clone(),
                knowledge_source.clone(),
                task_trajectory_tool.clone(),
                task_memory.clone(),
                non_refact_task_trajectory.clone(),
                task_meta.clone(),
                code.clone(),
            ],
            &roots,
        );

        assert!(memory_paths.contains(&knowledge));
        assert!(memory_paths.contains(&trajectory));
        assert!(memory_paths.contains(&task_trajectory));
        assert!(!memory_paths.contains(&knowledge_source));
        assert!(!memory_paths.contains(&task_trajectory_tool));
        assert!(!memory_paths.contains(&task_memory));
        assert!(!memory_paths.contains(&non_refact_task_trajectory));
        assert!(!memory_paths.contains(&task_meta));
        assert!(code_paths.contains(&knowledge_source));
        assert!(code_paths.contains(&task_trajectory_tool));
        assert!(code_paths.contains(&task_memory));
        assert!(code_paths.contains(&non_refact_task_trajectory));
        assert!(code_paths.contains(&task_meta));
        assert!(code_paths.contains(&code));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn routes_memory_artifacts_to_vecdb_and_code_under_memory_roots_to_codegraph() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }
        let project_root = get_project_dirs(gcx.clone())
            .await
            .into_iter()
            .next()
            .expect("test workspace must resolve to a project dir");
        let enqueue_calls = Arc::new(AtomicUsize::new(0));
        let vecdb_documents = Arc::new(Mutex::new(Vec::new()));
        *gcx.vec_db.lock().await = Some(Arc::new(RecordingVecdb {
            enqueue_calls: enqueue_calls.clone(),
            documents: vecdb_documents.clone(),
        }));
        let codegraph = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(codegraph.clone());

        let knowledge = project_root
            .join(".refact")
            .join("knowledge")
            .join("note.md");
        let trajectory = project_root
            .join(".refact")
            .join("tasks")
            .join("T-1")
            .join("trajectories")
            .join("run.json");
        let knowledge_source = project_root
            .join(".refact")
            .join("knowledge")
            .join("source.rs");
        let task_trajectory_tool = project_root
            .join(".refact")
            .join("tasks")
            .join("T-1")
            .join("trajectories")
            .join("tool.py");
        let paths = vec![
            knowledge.to_string_lossy().to_string(),
            trajectory.to_string_lossy().to_string(),
            knowledge_source.to_string_lossy().to_string(),
            task_trajectory_tool.to_string_lossy().to_string(),
        ];

        route_index_enqueue(gcx, &paths, false, false).await;

        assert_eq!(enqueue_calls.load(Ordering::SeqCst), 2);
        let vecdb_documents = vecdb_documents.lock().unwrap();
        assert!(vecdb_documents.contains(&knowledge.to_string_lossy().to_string()));
        assert!(vecdb_documents.contains(&trajectory.to_string_lossy().to_string()));
        assert!(!vecdb_documents.contains(&knowledge_source.to_string_lossy().to_string()));
        assert!(!vecdb_documents.contains(&task_trajectory_tool.to_string_lossy().to_string()));
        drop(vecdb_documents);
        let queued = codegraph.drain_batch(10);
        assert_eq!(queued.len(), 2);
        assert!(queued.contains(&knowledge_source.to_string_lossy().to_string()));
        assert!(queued.contains(&task_trajectory_tool.to_string_lossy().to_string()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn transient_tmp_paths_are_never_enqueued() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }
        let project_root = get_project_dirs(gcx.clone())
            .await
            .into_iter()
            .next()
            .expect("test workspace must resolve to a project dir");
        let enqueue_calls = Arc::new(AtomicUsize::new(0));
        let vecdb_documents = Arc::new(Mutex::new(Vec::new()));
        *gcx.vec_db.lock().await = Some(Arc::new(RecordingVecdb {
            enqueue_calls: enqueue_calls.clone(),
            documents: vecdb_documents.clone(),
        }));
        let codegraph = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(codegraph.clone());

        let buddy_state_tmp = project_root
            .join(".refact")
            .join("buddy")
            .join("state.json.tmp");
        let knowledge_note_tmp = project_root
            .join(".refact")
            .join("knowledge")
            .join("note.md.tmp");
        let index_write_tmp = project_root
            .join(".refact")
            .join("trajectories")
            .join(".index.json.tmp-0a1b2c3d");
        let paths = vec![
            buddy_state_tmp.to_string_lossy().to_string(),
            knowledge_note_tmp.to_string_lossy().to_string(),
            index_write_tmp.to_string_lossy().to_string(),
        ];

        route_index_enqueue(gcx, &paths, false, false).await;

        assert_eq!(enqueue_calls.load(Ordering::SeqCst), 0);
        assert!(vecdb_documents.lock().unwrap().is_empty());
        assert!(codegraph.drain_batch(10).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn code_under_memory_roots_routes_to_codegraph_with_zero_vecdb_enqueue() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }
        let project_root = get_project_dirs(gcx.clone())
            .await
            .into_iter()
            .next()
            .expect("test workspace must resolve to a project dir");
        let enqueue_calls = Arc::new(AtomicUsize::new(0));
        let vecdb_documents = Arc::new(Mutex::new(Vec::new()));
        *gcx.vec_db.lock().await = Some(Arc::new(RecordingVecdb {
            enqueue_calls: enqueue_calls.clone(),
            documents: vecdb_documents.clone(),
        }));
        let codegraph = Arc::new(crate::codegraph::CodeGraphService::open_in_memory().unwrap());
        *gcx.codegraph.lock().await = Some(codegraph.clone());

        let knowledge_source = project_root
            .join(".refact")
            .join("knowledge")
            .join("source.rs");
        let task_trajectory_tool = project_root
            .join(".refact")
            .join("tasks")
            .join("T-1")
            .join("trajectories")
            .join("tool.py");
        let paths = vec![
            knowledge_source.to_string_lossy().to_string(),
            task_trajectory_tool.to_string_lossy().to_string(),
        ];

        route_index_enqueue(gcx, &paths, false, false).await;

        assert_eq!(enqueue_calls.load(Ordering::SeqCst), 0);
        assert!(vecdb_documents.lock().unwrap().is_empty());
        assert_eq!(codegraph.drain_batch(10), paths);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn codegraph_absent_does_not_route_code_to_vecdb() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        }
        let enqueue_calls = Arc::new(AtomicUsize::new(0));
        let vecdb_documents = Arc::new(Mutex::new(Vec::new()));
        *gcx.vec_db.lock().await = Some(Arc::new(RecordingVecdb {
            enqueue_calls: enqueue_calls.clone(),
            documents: vecdb_documents,
        }));
        *gcx.codegraph.lock().await = None;
        let code_path = dir.path().join("src").join("main.rs");

        route_index_enqueue(
            gcx,
            &[code_path.to_string_lossy().to_string()],
            false,
            false,
        )
        .await;

        assert_eq!(enqueue_calls.load(Ordering::SeqCst), 0);
    }
}
