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
        if roots.classify_root(Path::new(path)).is_some() {
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
    if paths.is_empty() {
        return;
    }

    let roots = memory_plane_roots(gcx.clone()).await;
    let (memory_paths, code_paths) = partition_paths(paths, &roots);

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
        assert!(!memory_paths.contains(&task_memory));
        assert!(!memory_paths.contains(&non_refact_task_trajectory));
        assert!(!memory_paths.contains(&task_meta));
        assert!(code_paths.contains(&task_memory));
        assert!(code_paths.contains(&non_refact_task_trajectory));
        assert!(code_paths.contains(&task_meta));
        assert!(code_paths.contains(&code));
    }
}
