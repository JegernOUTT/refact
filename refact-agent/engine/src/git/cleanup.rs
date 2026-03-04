use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::time::Duration;
use tokio::sync::RwLock as ARwLock;

use crate::ast::chunk_utils::official_text_hashing_function;
use crate::custom_error::{trace_and_default, MapErrToString};
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;

const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
const MAX_INACTIVE_REPO_DURATION: Duration = Duration::from_secs(7 * SECONDS_PER_DAY); // 1 week
pub const RECENT_COMMITS_DURATION: Duration = Duration::from_secs(7 * SECONDS_PER_DAY); // 1 week
const CLEANUP_INTERVAL_DURATION: Duration = Duration::from_secs(SECONDS_PER_DAY); // 1 day

pub async fn git_shadow_cleanup_background_task(gcx: Arc<ARwLock<GlobalContext>>) {
    loop {
        let shutdown_flag = gcx.read().await.shutdown_flag.clone();
        // wait 2 mins before cleanup; lower priority than other startup tasks
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(2 * 60)) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                }
            } => {
                tracing::info!("Git shadow cleanup: shutdown detected, stopping");
                return;
            }
        }

        let cache_dir = {
            let gcx_locked = gcx.read().await;
            gcx_locked.cache_dir.clone()
        };
        let workspace_folders = get_project_dirs(gcx.clone()).await;
        let workspace_folder_hashes: Vec<_> = workspace_folders
            .into_iter()
            .map(|f| official_text_hashing_function(&f.to_string_lossy()))
            .collect();

        let dirs_to_check: Vec<_> = [
            cache_dir.join("shadow_git"),
            cache_dir.join("shadow_git").join("nested"),
        ]
        .into_iter()
        .filter(|dir| dir.exists())
        .collect();

        for dir in dirs_to_check {
            match cleanup_inactive_shadow_repositories(&dir, &workspace_folder_hashes).await {
                Ok(cleanup_count) => {
                    if cleanup_count > 0 {
                        tracing::info!(
                            "Git shadow cleanup: removed {} old repositories",
                            cleanup_count
                        );
                    }
                }
                Err(e) => {
                    tracing::error!("Git shadow cleanup failed: {}", e);
                }
            }
        }

        // NOTE: We intentionally do NOT perform object-level cleanup within active repositories.
        // The previous approach of deleting "old" objects was dangerous because "old" commits
        // can still be reachable from branches, and deleting their trees/blobs corrupts the repo.
        // Instead, we rely on full repository cleanup (above) for inactive repos.
        // If disk space becomes a concern, consider using `git gc` externally or implementing
        // proper reachability-based pruning.

        tokio::select! {
            _ = tokio::time::sleep(CLEANUP_INTERVAL_DURATION) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                }
            } => {
                tracing::info!("Git shadow cleanup: shutdown detected, stopping");
                return;
            }
        }
    }
}

async fn cleanup_inactive_shadow_repositories(
    dir: &Path,
    workspace_folder_hashes: &[String],
) -> Result<usize, String> {
    let mut inactive_repos = Vec::new();

    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| format!("Failed to read shadow_git directory: {}", e))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read directory entry: {}", e))?
    {
        let path = entry.path();
        let dir_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if !path.is_dir()
            || !path.join(".git").exists()
            || workspace_folder_hashes.contains(&dir_name)
        {
            continue;
        }

        if repo_is_inactive(&path)
            .await
            .unwrap_or_else(trace_and_default)
        {
            inactive_repos.push(path);
        }
    }

    let mut repos_to_remove = Vec::new();
    for repo_path in inactive_repos {
        let dir_name = repo_path.file_name().unwrap_or_default().to_string_lossy();
        if !dir_name.ends_with("_to_remove") {
            let mut new_path = repo_path.clone();
            new_path.set_file_name(format!("{dir_name}_to_remove"));
            match tokio::fs::rename(&repo_path, &new_path).await {
                Ok(()) => repos_to_remove.push(new_path),
                Err(e) => {
                    tracing::warn!("Failed to rename repo {}: {}", repo_path.display(), e);
                    continue;
                }
            }
        } else {
            repos_to_remove.push(repo_path);
        }
    }

    let mut cleanup_count = 0;
    for repo in repos_to_remove {
        match tokio::fs::remove_dir_all(&repo).await {
            Ok(()) => {
                tracing::info!("Removed old shadow git repository: {}", repo.display());
                cleanup_count += 1;
            }
            Err(e) => tracing::warn!(
                "Failed to remove shadow git repository {}: {}",
                repo.display(),
                e
            ),
        }
    }

    Ok(cleanup_count)
}

async fn repo_is_inactive(repo_dir: &Path) -> Result<bool, String> {
    let metadata = tokio::fs::metadata(repo_dir)
        .await
        .map_err_with_prefix(format!(
            "Failed to get metadata for {}:",
            repo_dir.display()
        ))?;

    let mtime = metadata.modified().map_err_with_prefix(format!(
        "Failed to get modified time for {}:",
        repo_dir.display()
    ))?;

    let duration_since_mtime = SystemTime::now()
        .duration_since(mtime)
        .map_err_with_prefix(format!(
            "Failed to calculate age for {}:",
            repo_dir.display()
        ))?;

    Ok(duration_since_mtime > MAX_INACTIVE_REPO_DURATION)
}


