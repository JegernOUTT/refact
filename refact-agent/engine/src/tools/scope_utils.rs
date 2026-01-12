use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::at_commands::at_file::{file_repair_candidates, return_one_candidate_or_a_good_error};
use crate::files_blocklist::reload_indexing_everywhere_if_needed;
use crate::files_correction::{canonicalize_normalized_path, correct_to_nearest_dir_path, get_project_dirs_with_code_workdir};
use crate::files_in_workspace::ls_files;
use crate::global_context::GlobalContext;

fn normalize_scope(scope: &str) -> String {
    scope.trim().replace('\\', "/")
}

fn try_resolve_path_in_workdir(workdir: &PathBuf, scope: &str) -> Option<PathBuf> {
    let normalized = normalize_scope(scope);
    let scope_path = PathBuf::from(&normalized);

    let candidate = if scope_path.is_absolute() {
        scope_path
    } else {
        workdir.join(&normalized)
    };

    if !candidate.exists() {
        return None;
    }

    let workdir_canonical = canonicalize_normalized_path(workdir.clone());
    let candidate_canonical = canonicalize_normalized_path(candidate);

    if !candidate_canonical.starts_with(&workdir_canonical) {
        return None;
    }

    Some(candidate_canonical)
}

async fn list_files_in_dir(
    gcx: Arc<ARwLock<GlobalContext>>,
    code_workdir: &Option<PathBuf>,
    dir_path: &PathBuf,
) -> Vec<String> {
    let indexing_everywhere = reload_indexing_everywhere_if_needed(gcx.clone()).await;
    let search_root = code_workdir.as_ref().unwrap_or(dir_path);

    ls_files(&indexing_everywhere, search_root, true)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.starts_with(dir_path))
        .map(|f| f.to_string_lossy().to_string())
        .collect()
}

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
    code_workdir: &Option<PathBuf>,
    scope: &str,
) -> Result<Vec<String>, String> {
    if scope == "workspace" {
        if let Some(workdir) = code_workdir {
            if workdir.exists() {
                let indexing_everywhere = reload_indexing_everywhere_if_needed(gcx.clone()).await;
                let files = ls_files(&indexing_everywhere, workdir, true).unwrap_or_default();
                return Ok(files.into_iter().map(|f| f.to_string_lossy().to_string()).collect());
            }
        }
        return Ok(get_workspace_files(gcx).await
            .into_iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect());
    }

    if let Some(workdir) = code_workdir {
        if workdir.exists() {
            if let Some(resolved) = try_resolve_path_in_workdir(workdir, scope) {
                if resolved.is_file() {
                    return Ok(vec![resolved.to_string_lossy().to_string()]);
                }
                if resolved.is_dir() {
                    return Ok(list_files_in_dir(gcx, code_workdir, &resolved).await);
                }
            }
        }
    }

    let project_dirs = get_project_dirs_with_code_workdir(gcx.clone(), code_workdir).await;
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

        let dir_path_buf = PathBuf::from(&dir_path);
        if let Some(workdir) = code_workdir {
            if workdir.exists() {
                return Ok(list_files_in_dir(gcx, code_workdir, &dir_path_buf).await);
            }
        }

        let dir_path_with_sep = if dir_path.ends_with(std::path::MAIN_SEPARATOR) {
            dir_path.clone()
        } else {
            format!("{}{}", dir_path, std::path::MAIN_SEPARATOR)
        };
        return Ok(get_workspace_files(gcx).await
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
                    let dir_path_buf = PathBuf::from(&dir_path);
                    if let Some(workdir) = code_workdir {
                        if workdir.exists() {
                            return Ok(list_files_in_dir(gcx, code_workdir, &dir_path_buf).await);
                        }
                    }

                    let dir_path_with_sep = if dir_path.ends_with(std::path::MAIN_SEPARATOR) {
                        dir_path.clone()
                    } else {
                        format!("{}{}", dir_path, std::path::MAIN_SEPARATOR)
                    };
                    Ok(get_workspace_files(gcx).await
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
    code_workdir: &Option<PathBuf>,
    scope: &str,
) -> Result<Option<String>, String> {
    if scope == "workspace" {
        if let Some(workdir) = code_workdir {
            if workdir.exists() {
                let workdir_str = workdir.to_string_lossy();
                return Ok(Some(format!("(scope LIKE '{}%')", workdir_str)));
            }
        }
        return Ok(None);
    }

    if let Some(workdir) = code_workdir {
        if workdir.exists() {
            if let Some(resolved) = try_resolve_path_in_workdir(workdir, scope) {
                let resolved_str = resolved.to_string_lossy();
                if resolved.is_file() {
                    return Ok(Some(format!("(scope = \"{}\")", resolved_str)));
                }
                if resolved.is_dir() {
                    let dir_with_sep = if resolved_str.ends_with(std::path::MAIN_SEPARATOR) {
                        resolved_str.to_string()
                    } else {
                        format!("{}{}", resolved_str, std::path::MAIN_SEPARATOR)
                    };
                    return Ok(Some(format!("(scope LIKE '{}%')", dir_with_sep)));
                }
            }
        }
    }

    let project_dirs = get_project_dirs_with_code_workdir(gcx.clone(), code_workdir).await;
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
