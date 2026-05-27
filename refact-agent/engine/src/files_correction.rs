use std::sync::Arc;
use std::time::Instant;
use std::path::{PathBuf, Path};
use tracing::info;

use crate::global_context::GlobalContext;
use crate::files_in_workspace::{detect_vcs_for_a_file_path, CacheCorrection};
use crate::fuzzy_search::fuzzy_search;
use crate::worktrees::scope::ExecutionScope;

pub use refact_files::path_utils::{
    preprocess_path_for_normalization, canonical_path, canonicalize_normalized_path,
    any_glob_matches_path, serialize_path, deserialize_path, CommandSimplifiedDirExt,
    shortify_paths_from_indexed,
};

pub async fn paths_from_anywhere(global_context: Arc<GlobalContext>) -> Vec<PathBuf> {
    let (file_paths_from_memory, paths_from_workspace, paths_from_jsonl) = {
        let documents_state = &global_context.documents_state; // somehow keeps lock until out of scope
        let file_paths_from_memory = documents_state
            .memory_document_map
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let paths_from_workspace = documents_state.workspace_files.lock().unwrap().clone();
        let paths_from_jsonl = documents_state.jsonl_files.lock().unwrap().clone();
        (
            file_paths_from_memory,
            paths_from_workspace,
            paths_from_jsonl,
        )
    };

    let paths_from_anywhere = file_paths_from_memory.into_iter().chain(
        paths_from_workspace
            .into_iter()
            .chain(paths_from_jsonl.into_iter()),
    );

    paths_from_anywhere.collect::<Vec<PathBuf>>()
}

pub async fn files_cache_rebuild_as_needed(
    global_context: Arc<GlobalContext>,
) -> Arc<CacheCorrection> {
    let cache_dirty_arc = global_context.documents_state.cache_dirty.clone();
    let mut cache_correction_arc = global_context
        .documents_state
        .cache_correction
        .lock()
        .unwrap()
        .clone();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let mut cache_dirty_ref = cache_dirty_arc.lock().await;
    if *cache_dirty_ref > 0.0 && now > *cache_dirty_ref {
        info!("rebuilding files cache...");
        // NOTE: we build cache on each add/delete file inside the workspace.
        // There should be a way to build cache once and then update it.
        let start_time = Instant::now();
        let paths_from_anywhere = paths_from_anywhere(global_context.clone()).await;
        let workspace_folders = get_project_dirs(global_context.clone()).await;
        let cache_correction = CacheCorrection::build(&paths_from_anywhere, &workspace_folders);

        info!(
            "rebuild completed in {:.3}s, over {}",
            start_time.elapsed().as_secs_f64(),
            paths_from_anywhere.len()
        );
        cache_correction_arc = Arc::new(cache_correction);
        {
            let cx = global_context.clone();
            *cx.documents_state.cache_correction.lock().unwrap() = cache_correction_arc.clone();
        }
        *cache_dirty_ref = 0.0;
    }

    cache_correction_arc
}

async fn complete_path_with_project_dir(
    gcx: Arc<GlobalContext>,
    correction_candidate: &String,
    is_dir: bool,
) -> Option<PathBuf> {
    fn path_exists(path: &PathBuf, is_dir: bool) -> bool {
        (is_dir && path.is_dir()) || (!is_dir && path.is_file())
    }
    let candidate_path = canonical_path(correction_candidate);
    let project_dirs = get_project_dirs(gcx.clone()).await;
    for p in project_dirs {
        if path_exists(&candidate_path, is_dir) && candidate_path.starts_with(&p) {
            return Some(candidate_path);
        }

        // This might save a roundtrip:
        // .../project1/project1/1.cpp
        // model likes to output only one "project1" of the two needed
        if candidate_path.starts_with(&p) {
            let last_component = p
                .components()
                .last()
                .map(|x| x.as_os_str().to_string_lossy().to_string())
                .unwrap_or("".to_string());
            let last_component_duplicated = p.join(&last_component).join(
                &candidate_path
                    .strip_prefix(&p)
                    .unwrap_or(candidate_path.as_path()),
            );
            if path_exists(&last_component_duplicated, is_dir) {
                info!(
                    "autocorrected by duplicating the project last component: {} -> {}",
                    p.to_string_lossy().to_string(),
                    last_component_duplicated.to_string_lossy().to_string()
                );
                return Some(last_component_duplicated);
            }
        }
    }
    None
}

async fn _correct_to_nearest(
    gcx: Arc<GlobalContext>,
    correction_candidate: &String,
    is_dir: bool,
    fuzzy: bool,
    top_n: usize,
) -> Vec<String> {
    if let Some(fixed) =
        complete_path_with_project_dir(gcx.clone(), correction_candidate, is_dir).await
    {
        return vec![fixed.to_string_lossy().to_string()];
    }

    let cache_correction_arc = files_cache_rebuild_as_needed(gcx.clone()).await;
    // it's dangerous to use cache_correction_arc without a mutex, but should be fine as long as it's read-only
    // (another thread never writes to the map itself, it can only replace the arc with a different map)

    // NOTE: do we need top_n here?
    let correction_cache = if is_dir {
        &cache_correction_arc.directories
    } else {
        &cache_correction_arc.filenames
    };
    let matches = correction_cache.find_matches(&PathBuf::from(correction_candidate));
    if matches.is_empty() {
        info!(
            "not found {:?} in cache_correction, is_dir={}",
            correction_candidate, is_dir
        );
    } else {
        return matches
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<String>>();
    }

    if fuzzy {
        info!(
            "fuzzy search {:?} is_dir={}, cache_fuzzy_arc.len={}",
            correction_candidate,
            is_dir,
            correction_cache.len()
        );
        return fuzzy_search(
            correction_candidate,
            correction_cache.short_paths_iter(),
            top_n,
            &['/', '\\'],
        );
    }

    vec![]
}

pub async fn correct_to_nearest_filename(
    gcx: Arc<GlobalContext>,
    correction_candidate: &String,
    fuzzy: bool,
    top_n: usize,
) -> Vec<String> {
    _correct_to_nearest(gcx, correction_candidate, false, fuzzy, top_n).await
}

pub async fn correct_to_nearest_dir_path(
    gcx: Arc<GlobalContext>,
    correction_candidate: &String,
    fuzzy: bool,
    top_n: usize,
) -> Vec<String> {
    _correct_to_nearest(gcx, correction_candidate, true, fuzzy, top_n).await
}

pub async fn get_project_dirs(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let workspace_folders = gcx.documents_state.workspace_folders.clone();
    let workspace_folders_locked = workspace_folders.lock().unwrap();
    workspace_folders_locked.iter().cloned().collect::<Vec<_>>()
}

#[allow(dead_code)]
pub async fn get_project_dirs_with_execution_scope(
    gcx: Arc<GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
) -> Vec<PathBuf> {
    if let Some(scope) = execution_scope {
        if scope.is_enforced() {
            return scope.effective_project_dirs();
        }
    }
    get_project_dirs(gcx).await
}

pub async fn get_active_project_path(gcx: Arc<GlobalContext>) -> Option<PathBuf> {
    let workspace_folders = get_project_dirs(gcx.clone()).await;
    if workspace_folders.is_empty() {
        return None;
    }

    let active_file = gcx.documents_state.active_file_path.lock().await.clone();
    // tracing::info!("get_active_project_path(), active_file={:?} workspace_folders={:?}", active_file, workspace_folders);

    let active_file_path = if let Some(active_file) = active_file {
        active_file
    } else {
        // tracing::info!("returning the first workspace folder: {:?}", workspace_folders[0]);
        return Some(workspace_folders[0].clone());
    };

    if let Some((path, _)) = detect_vcs_for_a_file_path(&active_file_path).await {
        // tracing::info!("found VCS path: {:?}", path);
        return Some(path);
    }

    // Without VCS, return one of workspace_folders that is a parent for active_file_path
    for f in workspace_folders {
        if active_file_path.starts_with(&f) {
            // tracing::info!("found that {:?} is the workspace folder", f);
            return Some(f);
        }
    }

    tracing::info!("no project is active");
    None
}

pub async fn get_active_workspace_folder(gcx: Arc<GlobalContext>) -> Option<PathBuf> {
    let workspace_folders = get_project_dirs(gcx.clone()).await;

    let active_file = gcx.documents_state.active_file_path.lock().await.clone();
    if let Some(active_file) = active_file {
        for f in &workspace_folders {
            if active_file.starts_with(f) {
                tracing::info!("found that {:?} is the workspace folder", f);
                return Some(f.clone());
            }
        }
    }

    if let Some(first_workspace_folder) = workspace_folders.first() {
        tracing::info!(
            "found that {:?} is the workspace folder",
            first_workspace_folder
        );
        Some(first_workspace_folder.clone())
    } else {
        None
    }
}

pub async fn shortify_paths(gcx: Arc<GlobalContext>, paths: &Vec<String>) -> Vec<String> {
    let cache_correction_arc = files_cache_rebuild_as_needed(gcx.clone()).await;
    shortify_paths_from_indexed(&cache_correction_arc, paths)
}

pub async fn check_if_its_inside_a_workspace_or_config(
    gcx: Arc<GlobalContext>,
    path: &Path,
) -> Result<(), String> {
    let workspace_folders = get_project_dirs(gcx.clone()).await;
    let config_dir = gcx.config_dir.clone();
    let path = canonicalize_normalized_path(path.to_path_buf());
    let workspace_folders_normalized = workspace_folders
        .iter()
        .map(|path| canonicalize_normalized_path(path.clone()))
        .collect::<Vec<_>>();
    let config_dir = canonicalize_normalized_path(config_dir);

    if workspace_folders_normalized
        .iter()
        .any(|dir| path.starts_with(dir))
        || path.starts_with(&config_dir)
    {
        Ok(())
    } else {
        Err(format!(
            "Path '{path:?}' is outside of project directories:\n{workspace_folders_normalized:?}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(all(target_arch = "aarch64", target_os = "linux")))]
    #[cfg(not(debug_assertions))]
    #[test]
    fn test_fuzzy_search_speed() {
        // Arrange
        let workspace_paths = vec![
            PathBuf::from("home").join("user").join("repo1"),
            PathBuf::from("home").join("user").join("repo2"),
            PathBuf::from("home").join("user").join("repo3"),
            PathBuf::from("home").join("user").join("repo4"),
        ];

        let mut paths = Vec::new();
        for i in 0..100000 {
            let path = workspace_paths[i % workspace_paths.len()]
                .join(format!("dir{}", i % 1000))
                .join(format!("dir{}", i / 1000))
                .join(format!("file{}.ext", i));
            paths.push(path);
        }
        let start_time = std::time::Instant::now();
        let paths_str = paths
            .iter()
            .map(|x| x.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let correction_candidate = PathBuf::from("file100000")
            .join("dir1000")
            .join("file100000.ext")
            .to_string_lossy()
            .to_string();

        // Act
        let results = fuzzy_search(&correction_candidate, paths_str, 10, &['/', '\\']);

        // Assert
        let time_spent = start_time.elapsed();
        println!("fuzzy_search took {} ms", time_spent.as_millis());
        assert_eq!(results.len(), 10, "The result should contain 10 paths");
        println!("{:?}", results);
    }
}
