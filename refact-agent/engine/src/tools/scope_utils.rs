use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::at_commands::at_file::{file_repair_candidates, return_one_candidate_or_a_good_error};
use crate::files_correction::{correct_to_nearest_dir_path, get_project_dirs};
use crate::global_context::GlobalContext;

async fn get_workspace_files(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<PathBuf> {
    gcx.read()
        .await
        .documents_state
        .workspace_files
        .lock()
        .unwrap()
        .clone()
}

pub async fn resolve_scope(
    gcx: Arc<ARwLock<GlobalContext>>,
    scope: &str,
) -> Result<Vec<String>, String> {
    if scope == "workspace" {
        return Ok(get_workspace_files(gcx)
            .await
            .into_iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect());
    }

    let project_dirs = get_project_dirs(gcx.clone()).await;
    let scope_string = scope.to_string();
    let scope_is_dir = scope.ends_with('/') || scope.ends_with('\\');

    if scope_is_dir {
        let dir_path = return_one_candidate_or_a_good_error(
            gcx.clone(),
            &scope_string,
            &correct_to_nearest_dir_path(gcx.clone(), &scope_string, false, 10).await,
            &project_dirs,
            true,
        )
        .await?;

        let dir_path_with_sep = if dir_path.ends_with(std::path::MAIN_SEPARATOR) {
            dir_path.clone()
        } else {
            format!("{}{}", dir_path, std::path::MAIN_SEPARATOR)
        };
        return Ok(get_workspace_files(gcx)
            .await
            .into_iter()
            .filter(|f| {
                f.to_string_lossy().starts_with(&dir_path_with_sep)
                    || f.to_string_lossy() == dir_path
            })
            .map(|f| f.to_string_lossy().to_string())
            .collect());
    }

    match return_one_candidate_or_a_good_error(
        gcx.clone(),
        &scope_string,
        &file_repair_candidates(gcx.clone(), &scope_string, 10, false).await,
        &project_dirs,
        false,
    )
    .await
    {
        Ok(file_path) => Ok(vec![file_path]),
        Err(file_err) => {
            match return_one_candidate_or_a_good_error(
                gcx.clone(),
                &scope_string,
                &correct_to_nearest_dir_path(gcx.clone(), &scope_string, false, 10).await,
                &project_dirs,
                true,
            )
            .await
            {
                Ok(dir_path) => {
                    let dir_path_with_sep = if dir_path.ends_with(std::path::MAIN_SEPARATOR) {
                        dir_path.clone()
                    } else {
                        format!("{}{}", dir_path, std::path::MAIN_SEPARATOR)
                    };
                    Ok(get_workspace_files(gcx)
                        .await
                        .into_iter()
                        .filter(|f| {
                            f.to_string_lossy().starts_with(&dir_path_with_sep)
                                || f.to_string_lossy() == dir_path
                        })
                        .map(|f| f.to_string_lossy().to_string())
                        .collect())
                }
                Err(_) => Err(file_err),
            }
        }
    }
}

pub async fn create_scope_filter(
    gcx: Arc<ARwLock<GlobalContext>>,
    scope: &str,
) -> Result<Option<String>, String> {
    if scope == "workspace" {
        return Ok(None);
    }

    let project_dirs = get_project_dirs(gcx.clone()).await;
    let scope_string = scope.to_string();
    let scope_is_dir = scope.ends_with('/') || scope.ends_with('\\');

    if scope_is_dir {
        let dir_path = return_one_candidate_or_a_good_error(
            gcx.clone(),
            &scope_string,
            &correct_to_nearest_dir_path(gcx.clone(), &scope_string, false, 10).await,
            &project_dirs,
            true,
        )
        .await?;

        let dir_path_with_sep = if dir_path.ends_with(std::path::MAIN_SEPARATOR) {
            dir_path.clone()
        } else {
            format!("{}{}", dir_path, std::path::MAIN_SEPARATOR)
        };
        return Ok(Some(format!("(scope LIKE '{}%')", dir_path_with_sep)));
    }

    match return_one_candidate_or_a_good_error(
        gcx.clone(),
        &scope_string,
        &file_repair_candidates(gcx.clone(), &scope_string, 10, false).await,
        &project_dirs,
        false,
    )
    .await
    {
        Ok(file_path) => Ok(Some(format!("(scope = \"{}\")", file_path))),
        Err(file_err) => {
            match return_one_candidate_or_a_good_error(
                gcx.clone(),
                &scope_string,
                &correct_to_nearest_dir_path(gcx.clone(), &scope_string, false, 10).await,
                &project_dirs,
                true,
            )
            .await
            {
                Ok(dir_path) => {
                    let dir_path_with_sep = if dir_path.ends_with(std::path::MAIN_SEPARATOR) {
                        dir_path.clone()
                    } else {
                        format!("{}{}", dir_path, std::path::MAIN_SEPARATOR)
                    };
                    Ok(Some(format!("(scope LIKE '{}%')", dir_path_with_sep)))
                }
                Err(_) => Err(file_err),
            }
        }
    }
}

pub fn validate_scope_files(files: Vec<String>, scope: &str) -> Result<Vec<String>, String> {
    if files.is_empty() {
        Err(format!(
            "⚠️ No files found in scope '{}'. 💡 Use 'workspace' for all files, 'dir/' (trailing slash) for directories, or check path exists",
            scope
        ))
    } else {
        Ok(files)
    }
}
