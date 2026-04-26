use std::path::{Path, PathBuf};
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
pub const RECENT_COMMITS_DURATION: Duration = Duration::from_secs(3 * SECONDS_PER_DAY); // 3 days
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

        let shadow_git_dirs: [PathBuf; 2] = [
            cache_dir.join("shadow_git"),
            cache_dir.join("shadow_git").join("nested"),
        ];
        let existing_shadow_git_dirs: Vec<&PathBuf> =
            shadow_git_dirs.iter().filter(|dir| dir.exists()).collect();

        for dir in &existing_shadow_git_dirs {
            match cleanup_inactive_shadow_repositories(dir, &workspace_folder_hashes).await {
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

        for dir in &existing_shadow_git_dirs {
            match collect_shadow_repo_dirs(dir).await {
                Ok(repo_dirs) => {
                    for repo_dir in repo_dirs {
                        cleanup_active_shadow_repository(&repo_dir).await;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Git shadow cleanup: failed to list repos in {}: {}",
                        dir.display(),
                        e
                    );
                }
            }
        }

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

async fn collect_shadow_repo_dirs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| format!("Failed to read {}: {}", dir.display(), e))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read directory entry: {}", e))?
    {
        let path = entry.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        // skip dirs already queued for removal
        if path.is_dir() && path.join(".git").exists() && !name.ends_with("_to_remove") {
            result.push(path);
        }
    }
    Ok(result)
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

async fn cleanup_active_shadow_repository(repo_dir: &Path) {
    match prune_old_refact_branches(repo_dir) {
        Ok(deleted) if deleted > 0 => {
            if let Err(e) = run_git_gc(repo_dir).await {
                tracing::warn!(
                    "Git shadow cleanup: git gc failed in {}: {}",
                    repo_dir.display(),
                    e
                );
            }
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(
                "Git shadow cleanup: failed to prune branches in {}: {}",
                repo_dir.display(),
                e
            );
        }
    }
}

fn prune_old_refact_branches(repo_dir: &Path) -> Result<usize, String> {
    let repo = git2::Repository::open(repo_dir).map_err_to_string()?;
    let cutoff_secs = SystemTime::now()
        .checked_sub(RECENT_COMMITS_DURATION)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut branches_to_delete: Vec<String> = Vec::new();
    let branches = repo
        .branches(Some(git2::BranchType::Local))
        .map_err_to_string()?;
    for branch_result in branches {
        let (branch, _) = branch_result.map_err_to_string()?;
        let name = match branch.name().map_err_to_string()? {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !name.starts_with("refact-") {
            continue;
        }
        if let Ok(commit) = branch.get().peel_to_commit() {
            if commit.time().seconds() < cutoff_secs {
                branches_to_delete.push(name);
            }
        }
    }

    let head_branch = repo
        .head()
        .ok()
        .filter(|h| h.is_branch())
        .and_then(|h| h.shorthand().map(|s| s.to_string()));
    if let Some(ref head) = head_branch {
        if branches_to_delete.iter().any(|b| b == head) {
            if let Ok(commit) = repo.head().and_then(|h| h.peel_to_commit()) {
                let _ = repo.set_head_detached(commit.id());
            }
        }
    }

    let mut deleted_count = 0usize;
    for branch_name in &branches_to_delete {
        match repo.find_branch(branch_name, git2::BranchType::Local) {
            Ok(mut branch) => match branch.delete() {
                Ok(()) => deleted_count += 1,
                Err(e) => tracing::warn!(
                    "Git shadow cleanup: failed to delete branch {}: {}",
                    branch_name,
                    e
                ),
            },
            Err(e) => tracing::warn!(
                "Git shadow cleanup: failed to find branch {}: {}",
                branch_name,
                e
            ),
        }
    }

    if deleted_count > 0 {
        tracing::info!(
            "Git shadow cleanup: pruned {}/{} old refact-* branches in {}",
            deleted_count,
            branches_to_delete.len(),
            repo_dir.display()
        );
    }
    Ok(deleted_count)
}

async fn run_git_gc(repo_dir: &Path) -> Result<(), String> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["gc", "--prune=now", "--quiet"])
        .output()
        .await
        .map_err_to_string()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git gc exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }
    Ok(())
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
