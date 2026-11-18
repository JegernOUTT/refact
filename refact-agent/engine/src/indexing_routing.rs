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
        if crate::python_tool::workspace::is_python_workspace_path(Path::new(path)) {
            continue;
        }
        if roots.classify_file(Path::new(path)).is_some() {
            memory_paths.push(path.clone());
        } else {
            code_paths.push(path.clone());
        }
    }
    (memory_paths, code_paths)
}

pub async fn route_index_enqueue(gcx: Arc<GlobalContext>, paths: &[String]) {
    let paths: Vec<String> = paths
        .iter()
        .filter(|path| !crate::file_filter::is_transient_tmp_path(Path::new(path.as_str())))
        .cloned()
        .collect();
    if paths.is_empty() {
        return;
    }

    let roots = memory_plane_roots(gcx.clone()).await;
    let (_memory_paths, code_paths) = partition_paths(&paths, &roots);

    if !code_paths.is_empty() {
        let codegraph = gcx.codegraph.lock().await.clone();
        match codegraph {
            Some(service) => service.enqueue_files(&code_paths),
            None => {
                tracing::warn!(
                    "codegraph unavailable; skipping {} code file(s) (memory artifacts never enter the code index)",
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
        let buddy_conversation = project_root
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join("chat.json")
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
                buddy_conversation.clone(),
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
        assert!(!memory_paths.contains(&buddy_conversation));
        assert!(code_paths.contains(&knowledge_source));
        assert!(code_paths.contains(&task_trajectory_tool));
        assert!(code_paths.contains(&task_memory));
        assert!(code_paths.contains(&non_refact_task_trajectory));
        assert!(code_paths.contains(&task_meta));
        assert!(code_paths.contains(&buddy_conversation));
        assert!(code_paths.contains(&code));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn routes_only_code_under_memory_roots_to_codegraph() {
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

        route_index_enqueue(gcx.clone(), &paths).await;
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

        route_index_enqueue(gcx, &paths).await;

        assert!(codegraph.drain_batch(10).is_empty());
    }
}
