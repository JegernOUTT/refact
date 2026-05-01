use std::sync::Arc;
use std::time::SystemTime;
use chrono::{DateTime, Utc};
use git2::{IndexAddOption, Oid, Repository};
use tokio::sync::RwLock as ARwLock;
use tokio::sync::Mutex as AMutex;
use tokio::time::Instant;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use serde::{Serialize, Deserialize};

use crate::ast::chunk_utils::official_text_hashing_function;
use crate::custom_error::MapErrToString;
use crate::files_blocklist::reload_indexing_everywhere_if_needed;
use crate::files_correction::{
    deserialize_path, get_active_workspace_folder, get_project_dirs, serialize_path,
};
use crate::global_context::GlobalContext;
use crate::git::{FileChange, FileChangeStatus, from_unix_glob_pattern_to_gitignore};
use crate::git::operations::{
    checkout_head_and_branch_to_commit, commit, get_commit_datetime, get_diff_statuses,
    get_diff_statuses_index_to_commit, get_or_create_branch, stage_changes, open_or_init_repo,
};
use crate::git::cleanup::RECENT_COMMITS_DURATION;

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub struct Checkpoint {
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub workspace_folder: PathBuf,
    pub commit_hash: String,
}

impl Checkpoint {
    pub fn workspace_hash(&self) -> String {
        workspace_folder_hash(&self.workspace_folder)
    }
}

async fn open_shadow_repo_and_nested_repos(
    gcx: Arc<ARwLock<GlobalContext>>,
    workspace_folder: &Path,
    allow_init_main_repo: bool,
) -> Result<(Repository, Vec<Repository>, String), String> {
    async fn open_repos(
        gcx: Arc<ARwLock<GlobalContext>>,
        paths: &[PathBuf],
        allow_init: bool,
        nested: bool,
        cache_dir: &Path,
    ) -> Result<Vec<Repository>, String> {
        let indexing_everywhere = reload_indexing_everywhere_if_needed(gcx).await;
        let mut result = Vec::new();
        for path in paths {
            let path = normalize_shadow_path(path);
            let indexing_for_path = indexing_everywhere.indexing_for_path(&path);
            let path_hash = shadow_repo_hash(&path);
            let git_dir_path = if nested {
                cache_dir.join("shadow_git").join("nested").join(&path_hash)
            } else {
                cache_dir.join("shadow_git").join(&path_hash)
            };
            let repo = if allow_init {
                open_or_init_repo(&git_dir_path).map_err_to_string()
            } else {
                Repository::open(&git_dir_path).map_err_to_string()
            }?;
            let filetime_now = filetime::FileTime::now();
            filetime::set_file_times(&git_dir_path, filetime_now, filetime_now)
                .map_err_to_string()?;
            repo.set_workdir(&path, false).map_err_to_string()?;
            for git_rule in [".git", ".git/**"] {
                if let Err(e) = repo.add_ignore_rule(git_rule) {
                    tracing::warn!(
                        "Failed to add ignore rule for {}: {}",
                        path.to_string_lossy(),
                        e
                    );
                }
            }
            for blocklisted_rule in indexing_for_path.blocklist {
                if let Err(e) =
                    repo.add_ignore_rule(&from_unix_glob_pattern_to_gitignore(&blocklisted_rule))
                {
                    tracing::warn!(
                        "Failed to add ignore rule for {}: {}",
                        path.to_string_lossy(),
                        e
                    );
                }
            }
            for additional_indexing_rule in indexing_for_path.additional_indexing_dirs {
                if let Err(e) = repo.add_ignore_rule(&format!(
                    "!{}",
                    from_unix_glob_pattern_to_gitignore(&additional_indexing_rule)
                )) {
                    tracing::warn!(
                        "Failed to add ignore rule for {}: {}",
                        path.to_string_lossy(),
                        e
                    );
                }
            }
            result.push(repo);
        }
        Ok(result)
    }

    let (cache_dir, vcs_roots) = {
        let gcx_locked = gcx.read().await;
        (
            gcx_locked.cache_dir.clone(),
            gcx_locked.documents_state.workspace_vcs_roots.clone(),
        )
    };
    let nested_vcs_roots: Vec<PathBuf> = {
        let vcs_roots_locked = vcs_roots.lock().unwrap();
        let workspace_key = normalize_shadow_path(workspace_folder);
        vcs_roots_locked
            .iter()
            .map(|vcs| normalize_shadow_path(vcs))
            .filter(|vcs| vcs.starts_with(&workspace_key) && *vcs != workspace_key)
            .collect()
    };
    let workspace_folder_hash = workspace_folder_hash(workspace_folder);

    let repo = open_repos(
        gcx.clone(),
        &[workspace_folder.to_path_buf()],
        allow_init_main_repo,
        false,
        &cache_dir,
    )
    .await?
    .into_iter()
    .next()
    .unwrap();
    let nested_repos = open_repos(gcx.clone(), &nested_vcs_roots, true, true, &cache_dir).await?;

    Ok((repo, nested_repos, workspace_folder_hash))
}

fn get_file_changes_from_nested_repos<'a>(
    parent_repo: &'a Repository,
    nested_repos: &'a [Repository],
    include_abs_paths: bool,
) -> Result<(Vec<(&'a Repository, Vec<FileChange>)>, Vec<FileChange>), String> {
    let repo_workdir = parent_repo
        .workdir()
        .ok_or("Failed to get workdir.".to_string())?;
    let mut file_changes_per_repo = Vec::new();
    let mut file_changes_flattened = Vec::new();

    for nested_repo in nested_repos {
        let (_, nested_repo_changes) =
            get_diff_statuses(git2::StatusShow::Workdir, nested_repo, include_abs_paths)?;
        let nested_repo_workdir = nested_repo
            .workdir()
            .ok_or("Failed to get nested repo workdir".to_string())?;
        let nested_repo_rel_path = nested_repo_workdir
            .strip_prefix(repo_workdir)
            .map_err_to_string()?;

        for change in &nested_repo_changes {
            file_changes_flattened.push(FileChange {
                relative_path: nested_repo_rel_path.join(&change.relative_path),
                absolute_path: change.absolute_path.clone(),
                status: change.status.clone(),
            });
        }
        file_changes_per_repo.push((nested_repo, nested_repo_changes));
    }

    Ok((file_changes_per_repo, file_changes_flattened))
}

fn resolve_checkpoint_workspace_folder(workspace_folder: &Path) -> Result<PathBuf, String> {
    let resolved = std::fs::canonicalize(workspace_folder).map_err(|e| {
        format!(
            "Checkpoint workspace root '{}' does not exist or cannot be resolved: {}",
            workspace_folder.display(),
            e
        )
    })?;
    if !resolved.is_dir() {
        return Err(format!(
            "Checkpoint workspace root '{}' is not a directory",
            resolved.display()
        ));
    }
    Ok(dunce::simplified(&resolved).to_path_buf())
}

fn workspace_folder_hash(workspace_folder: &Path) -> String {
    let normalized = normalize_shadow_path(workspace_folder);
    official_text_hashing_function(&normalized.to_string_lossy().to_string())
}

fn shadow_repo_hash(workspace_folder: &Path) -> String {
    let hash_path = std::fs::canonicalize(workspace_folder)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .unwrap_or_else(|_| normalize_shadow_path(workspace_folder));
    official_text_hashing_function(&hash_path.to_string_lossy().to_string())
}

fn normalize_shadow_path(path: &Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(canonical) => dunce::simplified(&canonical).to_path_buf(),
        Err(_) => dunce::simplified(path).to_path_buf(),
    }
}

fn repo_has_commits(repo: &Repository) -> bool {
    repo.head()
        .map(|head| head.target().is_some())
        .unwrap_or(false)
}

fn create_initial_shadow_commit(
    repo: &Repository,
    nested_repos: &[Repository],
    abort_flag: &Arc<AtomicBool>,
) -> Result<(Oid, usize), String> {
    let (_, mut file_changes) = get_diff_statuses(git2::StatusShow::Workdir, repo, false)?;
    let (nested_file_changes, all_nested_changes) =
        get_file_changes_from_nested_repos(repo, nested_repos, false)?;
    file_changes.extend(all_nested_changes);

    let mut skipped = stage_changes(repo, &file_changes, abort_flag)?;

    let mut index = repo.index().map_err_to_string()?;
    let tree_id = index.write_tree().map_err_to_string()?;
    let tree = repo.find_tree(tree_id).map_err_to_string()?;
    let signature = git2::Signature::now("Refact Agent", "agent@refact.ai").map_err_to_string()?;
    let commit = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "Initial commit",
            &tree,
            &[],
        )
        .map_err_to_string()?;

    for (nested_repo, changes) in nested_file_changes {
        skipped += stage_changes(nested_repo, &changes, abort_flag)?;
    }

    Ok((commit, skipped))
}

async fn initialize_shadow_repo_for_root_if_needed(
    gcx: Arc<ARwLock<GlobalContext>>,
    workspace_folder: &Path,
) -> Result<(), String> {
    let workspace_folder_str = workspace_folder.to_string_lossy().to_string();
    let abort_flag: Arc<AtomicBool> = gcx.read().await.git_operations_abort_flag.clone();
    let (repo, nested_repos, _) =
        open_shadow_repo_and_nested_repos(gcx.clone(), workspace_folder, true).await?;

    if repo_has_commits(&repo) {
        tracing::info!(
            "Shadow git repo for {} is already initialized.",
            workspace_folder_str
        );
        return Ok(());
    }

    let t0 = Instant::now();
    let (_, skipped) = create_initial_shadow_commit(&repo, &nested_repos, &abort_flag)?;
    if skipped > 0 {
        tracing::warn!(
            "initial commit for {workspace_folder_str}: {skipped} large file(s) not snapshotted"
        );
    }
    tracing::info!(
        "Shadow git repo for {} initialized in {:.2}s.",
        workspace_folder_str,
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}

pub async fn create_workspace_checkpoint_for_root(
    gcx: Arc<ARwLock<GlobalContext>>,
    workspace_folder: &Path,
    prev_checkpoint: Option<&Checkpoint>,
    chat_id: &str,
) -> Result<(Checkpoint, Repository), String> {
    let t0 = Instant::now();

    let workspace_folder = resolve_checkpoint_workspace_folder(workspace_folder)?;
    let workspace_folder_hash = workspace_folder_hash(&workspace_folder);
    if let Some(prev_checkpoint) = prev_checkpoint {
        if prev_checkpoint.workspace_hash() != workspace_folder_hash {
            return Err("Can not create checkpoint for different workspace folder".to_string());
        }
    }

    initialize_shadow_repo_for_root_if_needed(gcx.clone(), &workspace_folder).await?;

    let abort_flag: Arc<AtomicBool> = gcx.read().await.git_operations_abort_flag.clone();
    let (repo, nested_repos, _) =
        open_shadow_repo_and_nested_repos(gcx.clone(), &workspace_folder, false).await?;

    if !repo_has_commits(&repo) {
        return Err("No commits in shadow git repo.".to_string());
    }

    let checkpoint = {
        let branch = get_or_create_branch(&repo, &format!("refact-{chat_id}"))?;

        let (_, mut file_changes) = get_diff_statuses(git2::StatusShow::Workdir, &repo, false)?;

        let (nested_file_changes, flattened_nested_file_changes) =
            get_file_changes_from_nested_repos(&repo, &nested_repos, false)?;
        file_changes.extend(flattened_nested_file_changes);

        let mut skipped = stage_changes(&repo, &file_changes, &abort_flag)?;
        let commit_oid = commit(
            &repo,
            &branch,
            &format!("Auto commit for chat {chat_id}"),
            "Refact Agent",
            "agent@refact.ai",
        )?;

        for (nested_repo, changes) in nested_file_changes {
            skipped += stage_changes(nested_repo, &changes, &abort_flag)?;
        }
        if skipped > 0 {
            tracing::warn!(
                "checkpoint for chat {chat_id}: {skipped} large file(s) not snapshotted"
            );
        }

        Checkpoint {
            workspace_folder,
            commit_hash: commit_oid.to_string(),
        }
    };

    tracing::info!("Checkpoint created in {:.2}s", t0.elapsed().as_secs_f64());

    {
        let mut ev = crate::buddy::actor::make_runtime_event(
            "checkpoint_saved",
            &format!("Checkpoint saved for chat {}", chat_id),
            "git",
            &format!("checkpoint_{}", chat_id),
            "completed",
            None,
        );
        ev.chat_id = Some(chat_id.to_string());
        crate::buddy::actor::buddy_enqueue_event(gcx.clone(), ev).await;
    }

    Ok((checkpoint, repo))
}

pub async fn create_workspace_checkpoint(
    gcx: Arc<ARwLock<GlobalContext>>,
    prev_checkpoint: Option<&Checkpoint>,
    chat_id: &str,
) -> Result<(Checkpoint, Repository), String> {
    let workspace_folder = get_active_workspace_folder(gcx.clone())
        .await
        .ok_or_else(|| "No active workspace folder".to_string())?;
    create_workspace_checkpoint_for_root(gcx, &workspace_folder, prev_checkpoint, chat_id).await
}

pub async fn preview_changes_for_workspace_checkpoint_for_root(
    gcx: Arc<ARwLock<GlobalContext>>,
    workspace_folder: &Path,
    checkpoint_to_restore: &Checkpoint,
    chat_id: &str,
) -> Result<(Vec<FileChange>, DateTime<Utc>, Checkpoint), String> {
    let (checkpoint_for_undo, repo) = create_workspace_checkpoint_for_root(
        gcx.clone(),
        workspace_folder,
        Some(checkpoint_to_restore),
        chat_id,
    )
    .await?;

    let commit_to_restore_oid =
        Oid::from_str(&checkpoint_to_restore.commit_hash).map_err_to_string()?;
    let reverted_to = match get_commit_datetime(&repo, &commit_to_restore_oid) {
        Ok(dt) => dt,
        Err(_) => return Err(
            "This checkpoint has expired (checkpoints older than 3 days are removed automatically)"
                .to_string(),
        ),
    };

    let mut files_changed = match get_diff_statuses_index_to_commit(
        &repo,
        &commit_to_restore_oid,
        true,
    ) {
        Ok(files_changed) => files_changed,
        Err(e) => {
            let recent_cutoff_timestamp = SystemTime::now()
                .checked_sub(RECENT_COMMITS_DURATION)
                .unwrap()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            if reverted_to.timestamp() < recent_cutoff_timestamp as i64 {
                return Err("This checkpoint has expired (checkpoints older than 3 days are removed automatically)".to_string());
            } else {
                return Err(e);
            }
        }
    };

    for change in &mut files_changed {
        change.status = match change.status {
            FileChangeStatus::ADDED => FileChangeStatus::DELETED,
            FileChangeStatus::DELETED => FileChangeStatus::ADDED,
            FileChangeStatus::MODIFIED => FileChangeStatus::MODIFIED,
        };
    }

    Ok((files_changed, reverted_to, checkpoint_for_undo))
}

pub async fn preview_changes_for_workspace_checkpoint(
    gcx: Arc<ARwLock<GlobalContext>>,
    checkpoint_to_restore: &Checkpoint,
    chat_id: &str,
) -> Result<(Vec<FileChange>, DateTime<Utc>, Checkpoint), String> {
    let workspace_folder = get_active_workspace_folder(gcx.clone())
        .await
        .ok_or_else(|| "No active workspace folder".to_string())?;
    preview_changes_for_workspace_checkpoint_for_root(
        gcx,
        &workspace_folder,
        checkpoint_to_restore,
        chat_id,
    )
    .await
}

pub async fn restore_workspace_checkpoint_for_root(
    gcx: Arc<ARwLock<GlobalContext>>,
    workspace_folder: &Path,
    checkpoint_to_restore: &Checkpoint,
    chat_id: &str,
) -> Result<(), String> {
    let workspace_folder = resolve_checkpoint_workspace_folder(workspace_folder)?;
    let workspace_folder_hash = workspace_folder_hash(&workspace_folder);
    if checkpoint_to_restore.workspace_hash() != workspace_folder_hash {
        return Err("Can not restore checkpoint for different workspace folder".to_string());
    }

    let (repo, nested_repos, _) =
        open_shadow_repo_and_nested_repos(gcx.clone(), &workspace_folder, false).await?;

    let commit_to_restore_oid =
        Oid::from_str(&checkpoint_to_restore.commit_hash).map_err_to_string()?;

    checkout_head_and_branch_to_commit(
        &repo,
        &format!("refact-{chat_id}"),
        &commit_to_restore_oid,
    )?;

    for nested_repo in &nested_repos {
        let nested_workdir = match nested_repo.workdir() {
            Some(wd) => wd.to_path_buf(),
            None => {
                tracing::error!("Failed to get workdir for nested repo");
                continue;
            }
        };

        let reset_index_result = nested_repo.index().and_then(|mut index| {
            index.add_all(
                ["*"],
                IndexAddOption::DEFAULT,
                Some(&mut |path, _| {
                    let abs_path = nested_workdir.join(path);
                    if abs_path.is_dir() && abs_path.join(".git").exists() {
                        1
                    } else {
                        0
                    }
                }),
            )?;
            index.write()
        });
        if let Err(e) = reset_index_result {
            tracing::error!(
                "Failed to reset index for {}: {e}",
                nested_workdir.display()
            );
        }
    }

    Ok(())
}

pub async fn restore_workspace_checkpoint(
    gcx: Arc<ARwLock<GlobalContext>>,
    checkpoint_to_restore: &Checkpoint,
    chat_id: &str,
) -> Result<(), String> {
    let workspace_folder = get_active_workspace_folder(gcx.clone())
        .await
        .ok_or_else(|| "No active workspace folder".to_string())?;
    restore_workspace_checkpoint_for_root(gcx, &workspace_folder, checkpoint_to_restore, chat_id)
        .await
}

pub async fn init_shadow_repos_if_needed(gcx: Arc<ARwLock<GlobalContext>>) -> () {
    let init_shadow_repos_lock: Arc<AMutex<bool>> = gcx.read().await.init_shadow_repos_lock.clone();
    let _init_shadow_repos_lock = init_shadow_repos_lock.lock().await; // wait for previous init

    let workspace_folders = get_project_dirs(gcx.clone()).await;
    let abort_flag: Arc<AtomicBool> = gcx.read().await.git_operations_abort_flag.clone();

    for workspace_folder in workspace_folders {
        let workspace_folder_str = workspace_folder.to_string_lossy().to_string();

        let (repo, nested_repos) =
            match open_shadow_repo_and_nested_repos(gcx.clone(), &workspace_folder, true).await {
                Ok((repo, nested_repos, _)) => (repo, nested_repos),
                Err(e) => {
                    tracing::error!(
                        "Failed to open or init shadow repo for {workspace_folder_str}: {e}"
                    );
                    continue;
                }
            };

        let has_commits = repo
            .head()
            .map(|head| head.target().is_some())
            .unwrap_or(false);
        if has_commits {
            tracing::info!(
                "Shadow git repo for {} is already initialized.",
                workspace_folder_str
            );
            continue;
        }

        let t0 = Instant::now();

        let initial_commit_result: Result<(Oid, usize), String> = (|| {
            let (_, mut file_changes) = get_diff_statuses(git2::StatusShow::Workdir, &repo, false)?;
            let (nested_file_changes, all_nested_changes) =
                get_file_changes_from_nested_repos(&repo, &nested_repos, false)?;
            file_changes.extend(all_nested_changes);

            let mut skipped = stage_changes(&repo, &file_changes, &abort_flag)?;

            let mut index = repo.index().map_err_to_string()?;
            let tree_id = index.write_tree().map_err_to_string()?;
            let tree = repo.find_tree(tree_id).map_err_to_string()?;
            let signature =
                git2::Signature::now("Refact Agent", "agent@refact.ai").map_err_to_string()?;
            let commit = repo
                .commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    "Initial commit",
                    &tree,
                    &[],
                )
                .map_err_to_string()?;

            for (nested_repo, changes) in nested_file_changes {
                skipped += stage_changes(&nested_repo, &changes, &abort_flag)?;
            }
            Ok((commit, skipped))
        })();

        match initial_commit_result {
            Ok((_, skipped)) => {
                if skipped > 0 {
                    tracing::warn!("initial commit for {workspace_folder_str}: {skipped} large file(s) not snapshotted");
                }
                tracing::info!(
                    "Shadow git repo for {} initialized in {:.2}s.",
                    workspace_folder_str,
                    t0.elapsed().as_secs_f64()
                );
            }
            Err(e) => {
                tracing::error!("Initial commit for {workspace_folder_str} failed: {e}");
                continue;
            }
        }
    }
}

pub async fn enqueue_init_shadow_repos(gcx: Arc<ARwLock<GlobalContext>>) {
    let mut gcx_locked = gcx.write().await;
    // NOTE: potentially we can run init multiple times
    let gcx_cloned = gcx.clone();
    gcx_locked
        .init_shadow_repos_background_task_holder
        .push_back(tokio::spawn(async move {
            init_shadow_repos_if_needed(gcx_cloned).await;
        }));
}

pub async fn abort_init_shadow_repos(gcx: Arc<ARwLock<GlobalContext>>) {
    // NOTE: git2 operations are synchronous and can't be cancelled by tokio abort;
    // we set the abort flag and wait with a timeout to avoid hanging shutdown.
    let holder = {
        let mut gcx_locked = gcx.write().await;
        gcx_locked
            .git_operations_abort_flag
            .store(true, Ordering::SeqCst);
        std::mem::take(&mut gcx_locked.init_shadow_repos_background_task_holder)
    };
    // holder.abort() already has an internal timeout, but we call it here after releasing the lock
    let mut holder = holder;
    holder.abort().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::FileChangeStatus;
    use crate::worktrees::types::WorktreeMeta;
    use std::fs;
    use std::path::{Path, PathBuf};

    struct Fixture {
        _temp: tempfile::TempDir,
        source: PathBuf,
        worktree: PathBuf,
        gcx: Arc<ARwLock<GlobalContext>>,
        worktree_meta: WorktreeMeta,
    }

    fn write_file(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    async fn fixture() -> Fixture {
        let temp = tempfile::Builder::new()
            .prefix("refact-checkpoint-worktree")
            .tempdir()
            .unwrap();
        let source = temp.path().join("source");
        let worktree = temp.path().join("worktree");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::create_dir_all(worktree.join("src")).unwrap();
        write_file(&source.join("src").join("file.txt"), "source\n");
        write_file(&worktree.join("src").join("file.txt"), "before\n");
        let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
        let worktree = dunce::simplified(&fs::canonicalize(worktree).unwrap()).to_path_buf();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            let gcx_lock = gcx.read().await;
            *gcx_lock.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let worktree_meta = WorktreeMeta {
            id: "wt-checkpoint".to_string(),
            kind: "task_agent".to_string(),
            root: worktree.clone(),
            source_workspace_root: source.clone(),
            repo_root: source.clone(),
            branch: Some("refact/checkpoint".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: None,
            task_id: Some("task".to_string()),
            card_id: Some("card".to_string()),
            agent_id: Some("agent".to_string()),
            enforce: true,
        };
        Fixture {
            _temp: temp,
            source,
            worktree,
            gcx,
            worktree_meta,
        }
    }

    #[tokio::test]
    async fn checkpoint_worktree_create_uses_worktree_root() {
        let f = fixture().await;
        let (checkpoint, repo) = create_workspace_checkpoint_for_root(
            f.gcx.clone(),
            &f.worktree_meta.root,
            None,
            "checkpoint_worktree_create",
        )
        .await
        .unwrap();
        assert_eq!(checkpoint.workspace_folder, f.worktree);
        assert_eq!(repo.workdir().unwrap(), f.worktree.as_path());
        assert!(!checkpoint.commit_hash.is_empty());
    }

    #[tokio::test]
    async fn checkpoint_worktree_preview_reports_worktree_changes() {
        let f = fixture().await;
        let (checkpoint, _) = create_workspace_checkpoint_for_root(
            f.gcx.clone(),
            &f.worktree,
            None,
            "checkpoint_worktree_preview",
        )
        .await
        .unwrap();
        write_file(&f.worktree.join("src").join("file.txt"), "after\n");
        let (files_changed, _reverted_to, checkpoint_for_undo) =
            preview_changes_for_workspace_checkpoint_for_root(
                f.gcx.clone(),
                &f.worktree,
                &checkpoint,
                "checkpoint_worktree_preview",
            )
            .await
            .unwrap();
        assert_eq!(checkpoint_for_undo.workspace_folder, f.worktree);
        let changed_file = files_changed
            .iter()
            .find(|change| change.relative_path == PathBuf::from("src/file.txt"))
            .unwrap();
        assert_eq!(changed_file.status, FileChangeStatus::MODIFIED);
        assert_eq!(
            changed_file.absolute_path,
            f.worktree.join("src").join("file.txt")
        );
        assert!(files_changed
            .iter()
            .all(|change| change.absolute_path.starts_with(&f.worktree)));
    }

    #[tokio::test]
    async fn checkpoint_worktree_restore_reverts_only_worktree() {
        let f = fixture().await;
        let (checkpoint, _) = create_workspace_checkpoint_for_root(
            f.gcx.clone(),
            &f.worktree,
            None,
            "checkpoint_worktree_restore",
        )
        .await
        .unwrap();
        write_file(&f.worktree.join("src").join("file.txt"), "after\n");
        restore_workspace_checkpoint_for_root(
            f.gcx.clone(),
            &f.worktree,
            &checkpoint,
            "checkpoint_worktree_restore",
        )
        .await
        .unwrap();
        assert_eq!(
            fs::read_to_string(f.worktree.join("src").join("file.txt")).unwrap(),
            "before\n"
        );
        assert_eq!(
            fs::read_to_string(f.source.join("src").join("file.txt")).unwrap(),
            "source\n"
        );
    }

    #[tokio::test]
    async fn checkpoint_worktree_root_mismatch_is_rejected() {
        let f = fixture().await;
        let (checkpoint, _) = create_workspace_checkpoint_for_root(
            f.gcx.clone(),
            &f.worktree,
            None,
            "checkpoint_worktree_mismatch",
        )
        .await
        .unwrap();
        let other = f.source.parent().unwrap().join("other-worktree");
        fs::create_dir_all(&other).unwrap();
        let other = dunce::simplified(&fs::canonicalize(other).unwrap()).to_path_buf();
        let error = restore_workspace_checkpoint_for_root(
            f.gcx.clone(),
            &other,
            &checkpoint,
            "checkpoint_worktree_mismatch",
        )
        .await
        .unwrap_err();
        assert!(error.contains("different workspace folder"));
    }

    #[tokio::test]
    async fn checkpoint_worktree_stale_root_is_rejected() {
        let f = fixture().await;
        let error = match create_workspace_checkpoint_for_root(
            f.gcx.clone(),
            &f.worktree.join("deleted-root"),
            None,
            "checkpoint_worktree_stale",
        )
        .await
        {
            Ok(_) => panic!("stale worktree root unexpectedly created a checkpoint"),
            Err(e) => e,
        };
        assert!(error.contains("does not exist or cannot be resolved"));
    }

    #[tokio::test]
    async fn checkpoint_worktree_legacy_no_worktree_uses_active_workspace() {
        let f = fixture().await;
        write_file(&f.source.join("src").join("legacy.txt"), "legacy before\n");
        let (checkpoint, _) =
            create_workspace_checkpoint(f.gcx.clone(), None, "checkpoint_worktree_legacy")
                .await
                .unwrap();
        assert_eq!(checkpoint.workspace_folder, f.source);
        write_file(&f.source.join("src").join("legacy.txt"), "legacy after\n");
        restore_workspace_checkpoint(f.gcx.clone(), &checkpoint, "checkpoint_worktree_legacy")
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(f.source.join("src").join("legacy.txt")).unwrap(),
            "legacy before\n"
        );
    }
}
