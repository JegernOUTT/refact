use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use git2::{DiffOptions, Repository, Signature, StashFlags, Status, StatusOptions, StatusShow};
use uuid::Uuid;

use crate::types::{DirtyBaseline, WorktreeDiffFile, WorktreeDiffStats, WorktreeStatus};

pub struct WorktreeCreateResult {
    pub branch_was_created: bool,
    pub repo_root: PathBuf,
    pub base_branch: Option<String>,
    pub base_commit: String,
    pub dirty_source: bool,
}

pub struct WorktreeDiffParts {
    pub resolved_base_commit: Option<String>,
    pub files: Vec<WorktreeDiffFile>,
    pub stats: WorktreeDiffStats,
    pub patch: String,
    pub patch_truncated: bool,
    pub patch_shown_bytes: usize,
    pub patch_total_bytes: usize,
}

pub struct PreflightMergeResult {
    pub conflicts: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GitWorktreeEntry {
    pub root: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
}

fn status_options(show: StatusShow) -> StatusOptions {
    let mut options = StatusOptions::new();
    options
        .disable_pathspec_match(true)
        .include_ignored(false)
        .include_unmodified(false)
        .include_unreadable(false)
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show(show);
    options
}

fn is_index_changed(status: Status) -> bool {
    status.intersects(
        Status::INDEX_NEW
            | Status::INDEX_MODIFIED
            | Status::INDEX_DELETED
            | Status::INDEX_RENAMED
            | Status::INDEX_TYPECHANGE,
    )
}

fn is_workdir_changed(status: Status) -> bool {
    status.intersects(
        Status::WT_NEW
            | Status::WT_MODIFIED
            | Status::WT_DELETED
            | Status::WT_RENAMED
            | Status::WT_TYPECHANGE,
    )
}

pub fn discover_repo(path: &Path) -> Result<Repository, String> {
    Repository::discover(path).map_err(|e| {
        format!(
            "Workspace is not a git repository '{}': {}",
            path.display(),
            e
        )
    })
}

pub fn repo_root(repo: &Repository) -> Result<PathBuf, String> {
    repo.workdir()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "Repository has no working tree".to_string())
}

pub fn current_branch(repo: &Repository) -> Option<String> {
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    head.shorthand().map(|s| s.to_string())
}

pub fn local_branches(repo: &Repository) -> Vec<String> {
    let mut branches = Vec::new();
    let Ok(iter) = repo.branches(Some(git2::BranchType::Local)) else {
        return branches;
    };
    for item in iter.flatten() {
        if let Ok(Some(name)) = item.0.name() {
            branches.push(name.to_string());
        }
    }
    branches.sort();
    branches.dedup();
    branches
}

pub fn head_commit(repo: &Repository) -> Result<String, String> {
    let head = repo
        .head()
        .map_err(|e| format!("Failed to get HEAD: {}", e))?;
    let commit = head
        .peel_to_commit()
        .map_err(|e| format!("Failed to get HEAD commit: {}", e))?;
    Ok(commit.id().to_string())
}

pub fn commit_for_ref(repo: &Repository, refish: &str) -> Result<String, String> {
    let object = repo
        .revparse_single(refish)
        .map_err(|e| format!("Failed to resolve base '{}': {}", refish, e))?;
    let commit = object
        .peel_to_commit()
        .map_err(|e| format!("Base '{}' is not a commit: {}", refish, e))?;
    Ok(commit.id().to_string())
}

pub fn commit_for_branch(repo: &Repository, branch_name: &str) -> Result<String, String> {
    let branch = repo
        .find_branch(branch_name, git2::BranchType::Local)
        .map_err(|e| {
            if e.code() == git2::ErrorCode::NotFound {
                format!("Base branch '{}' not found", branch_name)
            } else {
                format!("Failed to resolve base branch '{}': {}", branch_name, e)
            }
        })?;
    let commit = branch
        .into_reference()
        .peel_to_commit()
        .map_err(|e| format!("Base branch '{}' is not a commit: {}", branch_name, e))?;
    Ok(commit.id().to_string())
}

pub fn has_uncommitted_changes(repo: &Repository) -> Result<bool, String> {
    let statuses = repo
        .statuses(Some(&mut status_options(StatusShow::IndexAndWorkdir)))
        .map_err(|e| format!("Failed to get git status: {}", e))?;
    Ok(statuses.iter().any(|entry| {
        let status = entry.status();
        is_index_changed(status) || is_workdir_changed(status)
    }))
}

pub fn create_worktree(
    source_root: &Path,
    worktree_path: &Path,
    worktree_name: &str,
    branch_name: &str,
    base_branch_ref: Option<&str>,
    base_commit_ref: Option<&str>,
) -> Result<WorktreeCreateResult, String> {
    let repo = discover_repo(source_root)?;
    let repo_root = repo_root(&repo)?;
    let base_branch = base_branch_ref
        .map(|s| s.to_string())
        .or_else(|| current_branch(&repo));
    if let Some(branch) = base_branch_ref {
        commit_for_branch(&repo, branch)?;
    }
    let base_commit = match (base_commit_ref, base_branch_ref) {
        (Some(commit), _) => commit_for_ref(&repo, commit)?,
        (None, Some(branch)) => commit_for_branch(&repo, branch)?,
        (None, None) => head_commit(&repo)?,
    };
    let dirty_source = has_porcelain_changes(&repo_root).unwrap_or(false);

    let commit_oid =
        git2::Oid::from_str(&base_commit).map_err(|e| format!("Invalid commit OID: {}", e))?;
    let commit = repo
        .find_commit(commit_oid)
        .map_err(|e| format!("Failed to find base commit: {}", e))?;

    let ref_name = format!("refs/heads/{}", branch_name);
    let branch_was_created = match repo.find_reference(&ref_name) {
        Ok(_) => {
            return Err(format!(
                "Branch '{}' already exists; choose a new worktree branch name or attach the existing worktree",
                branch_name
            ));
        }
        Err(e) if e.code() == git2::ErrorCode::NotFound => {
            repo.branch(branch_name, &commit, false)
                .map_err(|e| format!("Failed to create branch '{}': {}", branch_name, e))?;
            true
        }
        Err(e) => return Err(format!("Failed to look up branch '{}': {}", branch_name, e)),
    };

    if let Err(e) = add_worktree_with_git(source_root, worktree_path, branch_name) {
        if branch_was_created {
            let _ = repo
                .find_branch(branch_name, git2::BranchType::Local)
                .and_then(|mut branch| branch.delete());
        }
        return Err(format!(
            "Failed to create worktree '{}': {}",
            worktree_name, e
        ));
    }

    Ok(WorktreeCreateResult {
        branch_was_created,
        repo_root,
        base_branch,
        base_commit,
        dirty_source,
    })
}

fn add_worktree_with_git(
    source_root: &Path,
    worktree_path: &Path,
    branch_name: &str,
) -> Result<(), String> {
    let output = Command::new("git")
        .args(["worktree", "add"])
        .arg(worktree_path)
        .arg(branch_name)
        .current_dir(source_root)
        .output()
        .map_err(|e| format!("Failed to run git worktree add: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

pub fn remove_worktree(
    source_root: &Path,
    worktree_name: &str,
    worktree_path: &Path,
) -> Vec<String> {
    let mut warnings = Vec::new();
    match discover_repo(source_root) {
        Ok(repo) => match repo.find_worktree(worktree_name) {
            Ok(worktree) => {
                let mut options = git2::WorktreePruneOptions::new();
                options.valid(true);
                options.locked(true);
                options.working_tree(true);
                if let Err(e) = worktree.prune(Some(&mut options)) {
                    warnings.push(format!(
                        "Failed to prune worktree '{}': {}",
                        worktree_name, e
                    ));
                }
            }
            Err(e) => warnings.push(format!(
                "Could not find worktree '{}' in git metadata: {}",
                worktree_name, e
            )),
        },
        Err(e) => warnings.push(e),
    }

    if worktree_path.exists() {
        if let Err(e) = std::fs::remove_dir_all(worktree_path) {
            warnings.push(format!(
                "Failed to remove worktree directory '{}': {}",
                worktree_path.display(),
                e
            ));
        }
    }
    warnings
}

pub fn delete_branch(source_root: &Path, branch_name: &str) -> Result<bool, String> {
    let repo = discover_repo(source_root)?;
    let result = match repo.find_branch(branch_name, git2::BranchType::Local) {
        Ok(mut branch) => {
            branch
                .delete()
                .map_err(|e| format!("Failed to delete branch '{}': {}", branch_name, e))?;
            Ok(true)
        }
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(false),
        Err(e) => Err(format!("Failed to find branch '{}': {}", branch_name, e)),
    };
    result
}

pub fn status_for_path(path: &Path) -> WorktreeStatus {
    if !path.exists() {
        return WorktreeStatus {
            path_exists: false,
            is_git_worktree: false,
            dirty: false,
            conflicted: false,
            staged_count: 0,
            unstaged_count: 0,
            untracked_count: 0,
            branch: None,
            head_commit: None,
            error: None,
        };
    }

    match discover_repo(path) {
        Ok(repo) => status_for_repo(&repo),
        Err(e) => WorktreeStatus {
            path_exists: true,
            is_git_worktree: false,
            dirty: false,
            conflicted: false,
            staged_count: 0,
            unstaged_count: 0,
            untracked_count: 0,
            branch: None,
            head_commit: None,
            error: Some(e),
        },
    }
}

pub fn status_for_repo(repo: &Repository) -> WorktreeStatus {
    let mut status = WorktreeStatus {
        path_exists: true,
        is_git_worktree: true,
        dirty: false,
        conflicted: false,
        staged_count: 0,
        unstaged_count: 0,
        untracked_count: 0,
        branch: current_branch(repo),
        head_commit: head_commit(repo).ok(),
        error: None,
    };
    match repo.statuses(Some(&mut status_options(StatusShow::IndexAndWorkdir))) {
        Ok(statuses) => {
            for entry in statuses.iter() {
                let entry_status = entry.status();
                if is_index_changed(entry_status) {
                    status.staged_count += 1;
                }
                if entry_status.is_conflicted() {
                    status.conflicted = true;
                }
                if entry_status.is_wt_new() && !is_index_changed(entry_status) {
                    status.untracked_count += 1;
                } else if is_workdir_changed(entry_status) {
                    status.unstaged_count += 1;
                }
            }
            status.dirty =
                status.staged_count > 0 || status.unstaged_count > 0 || status.untracked_count > 0;
        }
        Err(e) => {
            status.error = Some(format!("Failed to read git status: {}", e));
        }
    }
    status
}

pub fn canonical_common_dir(repo: &Repository) -> Result<PathBuf, String> {
    repo.commondir().canonicalize().map_err(|e| {
        format!(
            "Failed to canonicalize git common dir '{}': {}",
            repo.commondir().display(),
            e
        )
    })
}

pub fn run_git(path: &Path, args: &[&str]) -> Result<String, String> {
    run_git_observed(path, args, || Ok(()))
}

fn run_git_observed(
    path: &Path,
    args: &[&str],
    succeeded: impl FnOnce() -> Result<(), String>,
) -> Result<String, String> {
    run_git_observed_with_env(path, args, &[], succeeded)
}

fn run_git_observed_with_env(
    path: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    succeeded: impl FnOnce() -> Result<(), String>,
) -> Result<String, String> {
    crate::locking::checkpoint_git_phase()?;
    #[cfg(test)]
    commit_window_fault::before(path, args)?;
    let output = Command::new("git")
        .args(args)
        .envs(envs.iter().copied())
        .current_dir(path)
        .output()
        .map_err(|e| format!("Failed to run git {:?}: {}", args, e))?;
    #[cfg(test)]
    if output.status.success() {
        commit_window_fault::hit(path, args)?;
    }
    if output.status.success() {
        succeeded()?;
    }
    crate::locking::checkpoint_git_phase()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stderr.is_empty() {
            Err(stdout.into_owned())
        } else {
            Err(stderr.into_owned())
        }
    }
}

#[cfg(test)]
pub(crate) mod commit_window_fault {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    enum Fault {
        After,
        Before,
        Trigger(Arc<AtomicBool>),
    }

    static FAULTS: Mutex<Vec<(PathBuf, &'static str, Fault)>> = Mutex::new(Vec::new());

    fn key(root: &Path) -> PathBuf {
        dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
    }

    pub(crate) fn install(root: &Path, token: &'static str) {
        FAULTS
            .lock()
            .unwrap()
            .push((key(root), token, Fault::After));
    }

    pub(crate) fn install_before(root: &Path, token: &'static str) {
        FAULTS
            .lock()
            .unwrap()
            .push((key(root), token, Fault::Before));
    }

    pub(crate) fn install_trigger(root: &Path, token: &'static str, flag: Arc<AtomicBool>) {
        FAULTS
            .lock()
            .unwrap()
            .push((key(root), token, Fault::Trigger(flag)));
    }

    fn take(root: &Path, args: &[&str], after: bool) -> Option<Fault> {
        let root = key(root);
        let mut faults = FAULTS.lock().unwrap();
        let index = faults.iter().position(|(path, token, fault)| {
            *path == root && args.contains(token) && matches!(fault, Fault::After) == after
        })?;
        Some(faults.remove(index).2)
    }

    pub(super) fn before(root: &Path, args: &[&str]) -> Result<(), String> {
        match take(root, args, false) {
            None => Ok(()),
            Some(Fault::Trigger(flag)) => {
                flag.store(true, Ordering::SeqCst);
                Ok(())
            }
            Some(_) => Err(format!("injected failure before git {args:?}")),
        }
    }

    pub(super) fn hit(root: &Path, args: &[&str]) -> Result<(), String> {
        match take(root, args, true) {
            None => Ok(()),
            Some(_) => Err("injected authority loss after git exited successfully".into()),
        }
    }
}

pub const MERGE_COMMITTER_NAME: &str = "Refact Agent";
pub const MERGE_COMMITTER_EMAIL: &str = "agent@refact.ai";

#[derive(Clone, Copy)]
pub enum Landing<'a> {
    Merge { source: &'a str },
    Squash { tree: &'a str },
    SquashOperation { tree: &'a str, marker: &'a str },
}

fn landing_matches(root: &Path, commit: &str, pre_merge_head: &str, landing: Landing<'_>) -> bool {
    if commit == pre_merge_head {
        return false;
    }
    let rev = |spec: String| {
        run_git(root, &["rev-parse", "--verify", "-q", &spec])
            .ok()
            .map(|out| out.trim().to_string())
    };
    if rev(format!("{commit}^1")).as_deref() != Some(pre_merge_head) {
        return false;
    }
    match landing {
        Landing::Merge {
            source,
        } => rev(format!("{commit}^2")).as_deref() == Some(source),
        Landing::Squash {
            tree,
        }
        | Landing::SquashOperation {
            tree, ..
        } => {
            if rev(format!("{commit}^2")).is_some() {
                return false;
            }
            (!tree.is_empty() && rev(format!("{commit}^{{tree}}")).as_deref() == Some(tree))
                || matches!(landing, Landing::SquashOperation { marker, .. }
                    if !marker.is_empty() && run_git(root, &["log", "-1", "--format=%B", commit])
                        .is_ok_and(|message| message.trim_end().rsplit_once("\n\n").is_some_and(|(_, trailer)| trailer == marker)))
        }
    }
}

pub fn landed_commit(root: &Path, pre_merge_head: &str, landing: Landing<'_>) -> Option<String> {
    crate::locking::with_git_phase(None, || {
        let head = head_rev(root).ok()?;
        landing_matches(root, &head, pre_merge_head, landing).then_some(head)
    })
}

pub fn landed_commit_below_head(
    root: &Path,
    pre_merge_head: &str,
    landing: Landing<'_>,
) -> Option<String> {
    crate::locking::with_git_phase(None, || {
        let commits = run_git(
            root,
            &[
                "rev-list",
                "--first-parent",
                "--reverse",
                &format!("{pre_merge_head}..HEAD"),
            ],
        )
        .ok()?;
        let first = commits.lines().next()?.trim().to_string();
        landing_matches(root, &first, pre_merge_head, landing).then_some(first)
    })
}

pub fn find_landed_commit(
    root: &Path,
    pre_merge_head: &str,
    landing: Landing<'_>,
) -> Option<String> {
    landed_commit(root, pre_merge_head, landing)
        .or_else(|| landed_commit_below_head(root, pre_merge_head, landing))
}

pub fn reapply_failure_summary(
    error: &str,
    rolled_back: bool,
    stash_restored: bool,
    foreign_head: Option<&str>,
    stash_location: &str,
) -> String {
    let restore = if stash_restored {
        "your edits were restored".to_string()
    } else {
        format!(
            "restoring your uncommitted edits failed; they are saved in {}",
            stash_location
        )
    };
    match (foreign_head, rolled_back, stash_restored) {
        (Some(head), _, _) => format!(
            "The target HEAD moved to {} during the merge; nothing was rolled back; {}: {}",
            head, restore, error
        ),
        (None, true, true) => format!(
            "Nothing was merged: your uncommitted target edits could not be re-applied on top of the incoming changes, so the merge was rolled back and your edits were restored in place: {}",
            error
        ),
        (None, false, true) => format!(
            "Nothing was merged: the merge was rolled back with warnings; your edits were restored: {}",
            error
        ),
        (None, _, false) => format!(
            "Nothing was merged: the merge was rolled back, but restoring your uncommitted edits failed; they are saved in {}: {}",
            stash_location, error
        ),
    }
}

pub fn kept_after_reapply_failure(
    error: &str,
    stash_restored: bool,
    foreign_head: Option<&str>,
    stash_location: &str,
) -> String {
    format!(
        "The merge commit landed and was kept because {}; re-applying your uncommitted target edits failed: {}; {}",
        match foreign_head {
            Some(head) => format!("the target HEAD moved to {} afterwards", head),
            None => "rolling it back failed".to_string(),
        },
        error,
        if stash_restored {
            "they were restored afterwards".to_string()
        } else {
            format!("they are saved in {}", stash_location)
        }
    )
}

pub fn commit_retained(root: &Path, commit: &str) -> bool {
    crate::locking::with_git_phase(None, || {
        git_probe(root, &["merge-base", "--is-ancestor", commit, "HEAD"]).unwrap_or(true)
    })
}

pub fn run_git_with_refact_author(path: &Path, args: &[&str]) -> Result<String, String> {
    run_git_with_refact_author_observed(path, args, || Ok(()))
}

pub(crate) fn run_git_with_refact_author_observed(
    path: &Path,
    args: &[&str],
    succeeded: impl FnOnce() -> Result<(), String>,
) -> Result<String, String> {
    let mut full_args = vec![
        "-c",
        "user.name=Refact Agent",
        "-c",
        "user.email=agent@refact.ai",
    ];
    full_args.extend_from_slice(args);
    run_git_observed_with_env(
        path,
        &full_args,
        &[
            ("GIT_COMMITTER_NAME", MERGE_COMMITTER_NAME),
            ("GIT_COMMITTER_EMAIL", MERGE_COMMITTER_EMAIL),
        ],
        succeeded,
    )
}

pub fn run_git_lossy(path: &Path, args: &[&str]) -> String {
    run_git(path, args).unwrap_or_default()
}

pub fn remove_worktree_path(source_root: &Path, worktree_path: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    let path_arg = worktree_path.to_string_lossy().to_string();
    if let Err(e) = run_git(source_root, &["worktree", "remove", "--force", &path_arg]) {
        warnings.push(format!(
            "Failed to remove worktree '{}': {}",
            worktree_path.display(),
            e
        ));
    }
    if worktree_path.exists() {
        if let Err(e) = std::fs::remove_dir_all(worktree_path) {
            warnings.push(format!(
                "Failed to remove worktree directory '{}': {}",
                worktree_path.display(),
                e
            ));
        }
    }
    warnings
}

pub fn branch_merged_into(root: &Path, branch: &str, base: &str) -> bool {
    branch == base || run_git(root, &["merge-base", "--is-ancestor", branch, base]).is_ok()
}

pub fn list_git_worktrees(source_root: &Path) -> Vec<GitWorktreeEntry> {
    list_git_worktrees_checked(source_root).unwrap_or_default()
}

pub(crate) fn list_git_worktrees_checked(
    source_root: &Path,
) -> Result<Vec<GitWorktreeEntry>, String> {
    let output = run_git(source_root, &["worktree", "list", "--porcelain"])?;
    let mut entries = Vec::new();
    let mut root: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut head: Option<String> = None;
    for line in output.lines().chain(std::iter::once("")) {
        if line.trim().is_empty() {
            if let Some(root) = root.take() {
                entries.push(GitWorktreeEntry {
                    root,
                    branch: branch.take(),
                    head: head.take(),
                });
            }
            branch = None;
            head = None;
            continue;
        }
        if let Some(value) = line.strip_prefix("worktree ") {
            root = Some(PathBuf::from(value));
        } else if let Some(value) = line.strip_prefix("HEAD ") {
            head = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("branch ") {
            branch = Some(
                value
                    .strip_prefix("refs/heads/")
                    .unwrap_or(value)
                    .to_string(),
            );
        }
    }
    Ok(entries)
}

pub fn prune_worktrees(source_root: &Path) -> Result<(), String> {
    run_git(source_root, &["worktree", "prune"]).map(|_| ())
}

#[derive(Debug)]
pub struct MergeCheckout {
    pub root: PathBuf,
    source_root: PathBuf,
    temporary: Option<tempfile::TempDir>,
    observed_branch: Option<String>,
    observed_head: String,
    target_head: String,
    target_branch: String,
}

impl Drop for MergeCheckout {
    fn drop(&mut self) {
        if let Some(temporary) = self.temporary.take() {
            if self.root.exists() {
                temporary.keep();
            }
        }
    }
}

pub fn prepare_merge_checkout(source_root: &Path, target: &str) -> Result<MergeCheckout, String> {
    let target_head = run_git(source_root, &["rev-parse", "--verify", target])?
        .trim()
        .to_string();
    let mut targets = list_git_worktrees_checked(source_root)?
        .into_iter()
        .filter(|entry| entry.branch.as_deref() == Some(target));
    if let Some(entry) = targets.next() {
        if targets.next().is_some() {
            return Err(format!(
                "Target branch '{}' is checked out more than once",
                target
            ));
        }
        ensure_checkout_on_branch(&entry.root, "Merge target", target)?;
        return Ok(MergeCheckout {
            root: dunce::canonicalize(&entry.root).map_err(|error| error.to_string())?,
            source_root: source_root.to_path_buf(),
            temporary: None,
            observed_branch: current_branch(&discover_repo(&entry.root)?),
            observed_head: head_rev(&entry.root)?,
            target_head,
            target_branch: target.to_string(),
        });
    }
    if ensure_clean_worktree(source_root, "Source workspace").is_ok() {
        return Ok(MergeCheckout {
            root: source_root.to_path_buf(),
            source_root: source_root.to_path_buf(),
            temporary: None,
            observed_branch: current_branch(&discover_repo(source_root)?),
            observed_head: head_rev(source_root)?,
            target_head,
            target_branch: target.to_string(),
        });
    }
    let temporary = tempfile::Builder::new()
        .prefix("refact-merge-target-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    let root = dunce::canonicalize(temporary.path())
        .map_err(|error| error.to_string())?
        .join("worktree");
    Ok(MergeCheckout {
        root,
        source_root: source_root.to_path_buf(),
        temporary: Some(temporary),
        observed_branch: None,
        observed_head: String::new(),
        target_head,
        target_branch: target.to_string(),
    })
}

impl MergeCheckout {
    pub(crate) fn is_temporary(&self) -> bool {
        self.temporary.is_some()
    }

    pub fn target_branch(&self) -> &str {
        &self.target_branch
    }

    pub fn cleanup(&self) -> Result<(), String> {
        if self.temporary.is_some() && self.root.exists() {
            ensure_checkout_on_branch(&self.root, "Temporary merge target", &self.target_branch)?;
            ensure_no_merge_in_progress(&self.root, "Temporary merge target")?;
            ensure_clean_worktree(&self.root, "Temporary merge target")?;
            run_git(
                &self.source_root,
                &["worktree", "remove", &self.root.to_string_lossy()],
            )?;
        }
        Ok(())
    }

    pub fn materialize(&self) -> Result<(), String> {
        if run_git(
            &self.source_root,
            &["rev-parse", "--verify", &self.target_branch],
        )?
        .trim()
            != self.target_head
        {
            return Err(
                "Merge target HEAD changed after resolution; resolve again before retrying".into(),
            );
        }
        if self.temporary.is_some() {
            if !self.root.exists() {
                run_git(
                    &self.source_root,
                    &[
                        "worktree",
                        "add",
                        &self.root.to_string_lossy(),
                        &self.target_branch,
                    ],
                )?;
            }
        } else if head_rev(&self.root)? != self.observed_head
            || current_branch(&discover_repo(&self.root)?) != self.observed_branch
        {
            return Err(
                "Merge target HEAD or branch changed after resolution; no writes performed".into(),
            );
        }
        Ok(())
    }
}

pub fn validate_merge_checkout(root: &Path, target: &str) -> Result<(), String> {
    for entry in list_git_worktrees(root) {
        if entry.branch.as_deref() == Some(target)
            && dunce::canonicalize(&entry.root).map_err(|error| error.to_string())?
                != dunce::canonicalize(root).map_err(|error| error.to_string())?
        {
            return Err(format!(
                "Merge target '{}' moved to another checkout",
                target
            ));
        }
    }
    let repo = discover_repo(root)?;
    if current_branch(&repo).as_deref() != Some(target) {
        ensure_clean_worktree(root, "Merge target before branch switch")?;
    }
    Ok(())
}

fn parse_numstat(output: &str) -> HashMap<String, (Option<usize>, Option<usize>)> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 3 {
                return None;
            }
            let additions = parts[0].parse::<usize>().ok();
            let deletions = parts[1].parse::<usize>().ok();
            let path = parts.last().unwrap_or(&parts[2]).to_string();
            Some((path, (additions, deletions)))
        })
        .collect()
}

fn parse_name_status(output: &str, numstat: &str, source: &str) -> Vec<WorktreeDiffFile> {
    let stats = parse_numstat(numstat);
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 2 {
                return None;
            }
            let status = parts[0].chars().next().unwrap_or('M').to_string();
            let path = parts.last().unwrap_or(&parts[1]).to_string();
            let (additions, deletions) = stats.get(&path).cloned().unwrap_or((None, None));
            Some(WorktreeDiffFile {
                old_path: if matches!(status.as_str(), "R" | "C") {
                    Some(parts[1].to_string())
                } else {
                    None
                },
                path,
                status,
                source: source.to_string(),
                additions,
                deletions,
            })
        })
        .collect()
}

fn count_file_lines(path: &Path) -> Option<usize> {
    const MAX_LINE_COUNT_BYTES: u64 = 1_000_000;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    if metadata.len() > MAX_LINE_COUNT_BYTES {
        return None;
    }
    std::fs::read_to_string(path)
        .ok()
        .map(|content| content.lines().count())
}

fn list_untracked(path: &Path) -> Vec<WorktreeDiffFile> {
    run_git_lossy(path, &["ls-files", "--others", "--exclude-standard"])
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| WorktreeDiffFile {
            path: line.to_string(),
            old_path: None,
            status: "A".to_string(),
            source: "untracked".to_string(),
            additions: count_file_lines(&path.join(line)),
            deletions: Some(0),
        })
        .collect()
}

struct PatchAccumulator {
    text: String,
    truncated: bool,
    total_bytes: usize,
    max_bytes: usize,
}

impl PatchAccumulator {
    fn new(max_bytes: usize) -> Self {
        Self {
            text: String::new(),
            truncated: false,
            total_bytes: 0,
            max_bytes,
        }
    }

    fn push(&mut self, text: &str) {
        self.total_bytes = self.total_bytes.saturating_add(text.len());
        if self.truncated || self.text.len() >= self.max_bytes {
            self.truncated = true;
            return;
        }
        let remaining = self.max_bytes - self.text.len();
        if text.len() <= remaining {
            self.text.push_str(text);
            return;
        }
        let mut end = remaining;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&text[..end]);
        self.text.push_str("\n");
        self.truncated = true;
    }

    fn account_unread(&mut self, bytes: usize) {
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        if bytes > 0 {
            self.truncated = true;
        }
    }

    fn shown_bytes(&self) -> usize {
        self.text.len()
    }
}

fn append_patch_section(patch: &mut PatchAccumulator, title: &str, body: &str) {
    if body.trim().is_empty() {
        return;
    }
    patch.push(&format!("\n## {}\n", title));
    patch.push(body);
    if !body.ends_with('\n') {
        patch.push("\n");
    }
}

fn append_untracked_patch(root: &Path, files: &[WorktreeDiffFile], patch: &mut PatchAccumulator) {
    const MAX_UNTRACKED_PREVIEW_BYTES: usize = 64_000;
    for file in files {
        let file_path = root.join(&file.path);
        let Ok(metadata) = std::fs::symlink_metadata(&file_path) else {
            continue;
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            let Ok(target) = std::fs::read_link(&file_path) else {
                continue;
            };
            append_patch_section(
                patch,
                &format!("untracked symlink {}", file.path),
                &format!(
                    "diff --git a/{} b/{}\nnew file mode 120000\n--- /dev/null\n+++ b/{}\n@@\n+{}\n",
                    file.path,
                    file.path,
                    file.path,
                    target.to_string_lossy()
                ),
            );
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Ok(mut source) = std::fs::File::open(&file_path) else {
            continue;
        };
        append_patch_section(
            patch,
            &format!("untracked {}", file.path),
            &format!(
                "diff --git a/{} b/{}\nnew file mode 100644\n--- /dev/null\n+++ b/{}\n@@\n",
                file.path, file.path, file.path
            ),
        );
        let read_cap = MAX_UNTRACKED_PREVIEW_BYTES;
        let mut buffer = Vec::new();
        let mut reader = source.by_ref().take(read_cap as u64 + 1);
        if reader.read_to_end(&mut buffer).is_err() {
            continue;
        }
        let over_limit = buffer.len() > read_cap;
        if over_limit {
            buffer.truncate(read_cap);
        }
        let content = String::from_utf8_lossy(&buffer);
        for line in content.lines() {
            patch.push("+");
            patch.push(line);
            patch.push("\n");
        }
        if over_limit {
            let unread = (metadata.len() as usize).saturating_sub(read_cap);
            patch.account_unread(unread);
        }
    }
}

fn resolve_diff_base(
    root: &Path,
    base_commit: Option<&str>,
    base_branch: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(base) = base_commit {
        let refish = format!("{}^{{commit}}", base);
        let resolved = run_git(root, &["rev-parse", "--verify", &refish])?;
        return Ok(Some(resolved.trim().to_string()));
    }
    if let Some(base) = base_branch {
        let resolved = run_git(root, &["merge-base", base, "HEAD"])
            .map_err(|e| format!("Failed to resolve merge-base for '{}': {}", base, e))?;
        return Ok(Some(resolved.trim().to_string()));
    }
    Ok(None)
}

pub fn diff_for_path(
    root: &Path,
    base_commit: Option<&str>,
    base_branch: Option<&str>,
    max_patch_bytes: usize,
) -> Result<WorktreeDiffParts, String> {
    discover_repo(root)?;
    let mut files = Vec::new();
    let mut stats = WorktreeDiffStats::default();
    let mut patch = PatchAccumulator::new(max_patch_bytes);

    let committed_base = resolve_diff_base(root, base_commit, base_branch)?;
    if let Some(base) = &committed_base {
        let range = format!("{}..HEAD", base);
        let committed_name_status = run_git(root, &["diff", "--name-status", &range])?;
        let committed_numstat = run_git(root, &["diff", "--numstat", &range])?;
        let committed = parse_name_status(&committed_name_status, &committed_numstat, "committed");
        stats.committed_files = committed.len();
        files.extend(committed);
        let committed_patch = run_git(root, &["diff", "--no-ext-diff", &range])?;
        append_patch_section(&mut patch, "committed", &committed_patch);
    }

    let staged_name_status = run_git_lossy(root, &["diff", "--cached", "--name-status"]);
    let staged_numstat = run_git_lossy(root, &["diff", "--cached", "--numstat"]);
    let staged = parse_name_status(&staged_name_status, &staged_numstat, "staged");
    stats.staged_files = staged.len();
    files.extend(staged);
    let staged_patch = run_git_lossy(root, &["diff", "--no-ext-diff", "--cached"]);
    append_patch_section(&mut patch, "staged", &staged_patch);

    let unstaged_name_status = run_git_lossy(root, &["diff", "--name-status"]);
    let unstaged_numstat = run_git_lossy(root, &["diff", "--numstat"]);
    let unstaged = parse_name_status(&unstaged_name_status, &unstaged_numstat, "unstaged");
    stats.unstaged_files = unstaged.len();
    files.extend(unstaged);
    let unstaged_patch = run_git_lossy(root, &["diff", "--no-ext-diff"]);
    append_patch_section(&mut patch, "unstaged", &unstaged_patch);

    let untracked = list_untracked(root);
    stats.untracked_files = untracked.len();
    append_untracked_patch(root, &untracked, &mut patch);
    files.extend(untracked);
    stats.files_changed = files.len();
    stats.additions = files.iter().filter_map(|file| file.additions).sum();
    stats.deletions = files.iter().filter_map(|file| file.deletions).sum();

    let patch_truncated = patch.truncated;
    let patch_shown_bytes = patch.shown_bytes();
    let patch_total_bytes = patch.total_bytes.max(patch_shown_bytes);
    let mut patch_text = patch.text;
    if patch_truncated {
        patch_text.push_str(&format!(
            "\n[patch truncated: showing {} of {} bytes]\n",
            patch_shown_bytes, patch_total_bytes
        ));
    }

    Ok(WorktreeDiffParts {
        resolved_base_commit: committed_base,
        files,
        stats,
        patch: patch_text,
        patch_truncated,
        patch_shown_bytes,
        patch_total_bytes,
    })
}

pub fn branch_exists(root: &Path, branch: &str) -> Result<bool, String> {
    let reference = format!("refs/heads/{}", branch);
    Ok(run_git(root, &["rev-parse", "--verify", &reference]).is_ok())
}

pub fn ensure_no_merge_in_progress(root: &Path, label: &str) -> Result<(), String> {
    discover_repo(root)?;
    if has_merge_in_progress(root) {
        return Err(format!("{} has a merge in progress", label));
    }
    Ok(())
}

pub fn has_merge_in_progress(root: &Path) -> bool {
    run_git(root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok()
}

pub fn ensure_clean_worktree(root: &Path, label: &str) -> Result<(), String> {
    ensure_no_merge_in_progress(root, label)?;
    if has_porcelain_changes(root)? {
        Err(format!("{} has uncommitted changes", label))
    } else {
        Ok(())
    }
}

pub fn has_porcelain_changes(root: &Path) -> Result<bool, String> {
    Ok(!run_git(root, &["status", "--porcelain"])?.trim().is_empty())
}

/// Files listed in a dirty baseline before the rest are counted only.
pub const DIRTY_BASELINE_MAX_FILES: usize = 50;

/// Uncommitted work standing in a checkout right now, or `None` when it is
/// clean. Reads only; nothing is stashed, staged or reset here.
pub fn dirty_baseline(root: &Path) -> Result<Option<DirtyBaseline>, String> {
    let repo = discover_repo(root)?;
    let porcelain = run_git(root, &["status", "--porcelain"])?;
    let mut baseline = DirtyBaseline {
        branch: current_branch(&repo),
        ..DirtyBaseline::default()
    };
    for line in porcelain.lines() {
        if line.len() < 4 {
            continue;
        }
        let staged = line.as_bytes()[0] as char;
        let unstaged = line.as_bytes()[1] as char;
        if staged == '?' {
            baseline.untracked_count += 1;
        } else {
            if staged != ' ' {
                baseline.staged_count += 1;
            }
            if unstaged != ' ' {
                baseline.unstaged_count += 1;
            }
        }
        if baseline.files.len() < DIRTY_BASELINE_MAX_FILES {
            baseline.files.push(line[3..].to_string());
        } else {
            baseline.files_truncated = true;
        }
    }
    Ok((!baseline.files.is_empty()).then_some(baseline))
}

pub fn ensure_dirty_worktree_on_branch(
    root: &Path,
    label: &str,
    expected_branch: &str,
) -> Result<(), String> {
    ensure_no_merge_in_progress(root, label)?;
    if !has_porcelain_changes(root)? {
        return Ok(());
    }
    ensure_checkout_on_branch(root, label, expected_branch)
}

pub fn ensure_checkout_on_branch(
    root: &Path,
    label: &str,
    expected_branch: &str,
) -> Result<(), String> {
    let repo = discover_repo(root)?;
    ensure_repo_on_branch(&repo, label, expected_branch)
}

pub fn ensure_repo_on_branch(
    repo: &Repository,
    label: &str,
    expected_branch: &str,
) -> Result<(), String> {
    match current_branch(repo).as_deref() {
        Some(current_branch) if current_branch == expected_branch => Ok(()),
        Some(current_branch) => Err(format!(
            "{} is checked out on a different branch '{}'; expected branch '{}'",
            label, current_branch, expected_branch
        )),
        None => Err(format!(
            "{} is checked out on detached HEAD {}; expected branch '{}'",
            label,
            head_commit(repo)?,
            expected_branch
        )),
    }
}

#[derive(Debug, Clone)]
pub struct DirtyWorktreeStash {
    pub original_branch: Option<String>,
    pub original_head: String,
    stash_marker: String,
    stash_oid: String,
}

#[derive(Debug, Clone)]
pub struct RestoreStashedChangesResult {
    pub critical_ok: bool,
    pub warnings: Vec<String>,
    pub rolled_back: bool,
    pub stash_restored: bool,
    pub foreign_head: Option<String>,
}

impl DirtyWorktreeStash {
    pub fn stash_oid(&self) -> &str {
        &self.stash_oid
    }

    pub fn describe(&self) -> String {
        format!(
            "git stash {} (\"{}\"); recover with `git stash apply --index {}`",
            self.stash_oid, self.stash_marker, self.stash_oid
        )
    }
}

pub fn stash_dirty_worktree(
    root: &Path,
    label: &str,
    expected_branch: Option<&str>,
) -> Result<Option<DirtyWorktreeStash>, String> {
    match stash_dirty_worktree_retaining(root, label, expected_branch) {
        Ok(stash) => Ok(stash),
        Err((error, None)) => Err(error),
        Err((error, Some(stash))) => {
            let restore = rollback_merge_and_restore_stash(root, None, Some(&stash));
            Err(format!("{}; {}", error, restore_details(&stash, &restore)))
        }
    }
}

pub fn stash_dirty_worktree_retaining(
    root: &Path,
    label: &str,
    expected_branch: Option<&str>,
) -> Result<Option<DirtyWorktreeStash>, (String, Option<DirtyWorktreeStash>)> {
    let before = || -> Result<Option<(Repository, Option<String>, String)>, String> {
        ensure_no_merge_in_progress(root, label)?;
        let status = run_git(root, &["status", "--porcelain"])?;
        if status.trim().is_empty() {
            return Ok(None);
        }
        let repo = discover_repo(root)?;
        let original_branch = current_branch(&repo);
        let original_head = head_rev(root)?;
        if let Some(expected_branch) = expected_branch {
            ensure_checkout_on_branch(root, label, expected_branch)?;
        }
        crate::locking::checkpoint_git_phase()?;
        Ok(Some((repo, original_branch, original_head)))
    };
    let Some((mut repo, original_branch, original_head)) = before().map_err(|e| (e, None))? else {
        return Ok(None);
    };
    let stash_marker = format!("Refact worktree merge target workspace {}", Uuid::new_v4());
    let stasher = Signature::now("Refact Agent", "agent@refact.ai")
        .map_err(|e| (format!("Failed to create stash author: {}", e), None))?;
    let stash_oid = repo
        .stash_save(&stasher, &stash_marker, Some(StashFlags::INCLUDE_UNTRACKED))
        .map_err(|e| {
            (
                format!("Failed to stash target workspace changes: {}", e),
                None,
            )
        })?
        .to_string();
    let stash = DirtyWorktreeStash {
        original_branch,
        original_head,
        stash_marker,
        stash_oid,
    };
    if let Err(e) = crate::locking::checkpoint_git_phase() {
        return Err((e, Some(stash)));
    }
    if let Err(e) = ensure_clean_worktree(root, label) {
        return Err((e, Some(stash)));
    }
    Ok(Some(stash))
}

pub fn restore_details(stash: &DirtyWorktreeStash, result: &RestoreStashedChangesResult) -> String {
    if result.critical_ok && result.warnings.is_empty() {
        "target workspace changes were restored".to_string()
    } else if result.critical_ok {
        format!(
            "target workspace changes were restored with warnings: {}",
            result.warnings.join("; ")
        )
    } else if result.stash_restored {
        format!(
            "target workspace changes were restored, but the rollback finished with warnings: {}",
            result.warnings.join("; ")
        )
    } else {
        format!(
            "failed to restore target workspace changes; the edits are preserved in {}: {}",
            stash.describe(),
            result.warnings.join("; ")
        )
    }
}

pub fn rollback_merge_and_restore_stash(
    root: &Path,
    pre_merge_head: Option<&str>,
    stash: Option<&DirtyWorktreeStash>,
) -> RestoreStashedChangesResult {
    rollback_merge_owned(root, pre_merge_head, None, stash)
}

pub fn rollback_merge_owned(
    root: &Path,
    pre_merge_head: Option<&str>,
    own_commit: Option<&str>,
    stash: Option<&DirtyWorktreeStash>,
) -> RestoreStashedChangesResult {
    crate::locking::with_git_phase(None, || {
        let baseline = pre_merge_head.or_else(|| stash.map(|stash| stash.original_head.as_str()));
        let head = match (baseline, head_rev(root)) {
            (Some(_), Ok(head)) => head,
            (_, head) => {
                return RestoreStashedChangesResult {
                    critical_ok: false,
                    warnings: vec![format!("Rollback ownership could not be established; nothing was rolled back; {}; {}", head.err().unwrap_or_else(|| "missing pre-merge baseline".into()), stash.map(|stash| stash.describe()).unwrap_or_default())],
                    rolled_back: false,
                    stash_restored: stash.is_none(),
                    foreign_head: None,
                };
            }
        };
        if let Some(stash) = stash {
            if discover_repo(root).map(|repo| current_branch(&repo)).ok()
                != Some(stash.original_branch.clone())
            {
                return RestoreStashedChangesResult {
                    critical_ok: false,
                    warnings: vec![format!("Target checkout branch changed; nothing was rolled back; edits are saved in {}", stash.describe())],
                    rolled_back: false,
                    stash_restored: false,
                    foreign_head: Some(head),
                };
            }
        }
        if Some(head.as_str()) != baseline && Some(head.as_str()) != own_commit {
            return restore_over_foreign_head(root, head, stash);
        }
        let mut warnings = cleanup_failed_merge(root);
        let mut rolled_back = warnings.is_empty();
        if let Some(head) = pre_merge_head {
            let reset = reset_keeping_local_changes(root, head);
            rolled_back &= reset.is_empty();
            warnings.extend(reset);
        }
        let mut critical_ok = rolled_back;
        let mut stash_restored = true;
        if let Some(stash) = stash {
            let restored = restore_stashed_changes(root, stash);
            critical_ok &= restored.critical_ok;
            rolled_back &= restored.rolled_back;
            stash_restored = restored.stash_restored;
            warnings.extend(restored.warnings);
        }
        critical_ok &= conflict_files_for_path(root).is_empty() && !has_merge_in_progress(root);
        for name in ["MERGE_MSG", "SQUASH_MSG"] {
            if let Ok(path) = run_git(root, &["rev-parse", "--git-path", name]) {
                let path = PathBuf::from(path.trim());
                let path = if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                };
                if path.exists() {
                    if let Err(e) = std::fs::remove_file(&path) {
                        critical_ok = false;
                        warnings.push(format!("Failed to remove {}: {}", name, e));
                    }
                }
            }
        }
        RestoreStashedChangesResult {
            critical_ok,
            warnings,
            rolled_back,
            stash_restored,
            foreign_head: None,
        }
    })
}

fn restore_over_foreign_head(
    root: &Path,
    head: String,
    stash: Option<&DirtyWorktreeStash>,
) -> RestoreStashedChangesResult {
    let mut leftovers = Vec::new();
    if has_merge_in_progress(root) {
        leftovers.push("MERGE_HEAD".to_string());
    }
    if let Ok(path) = run_git(root, &["rev-parse", "--git-path", "SQUASH_MSG"]) {
        let path = PathBuf::from(path.trim());
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        if path.exists() {
            leftovers.push("SQUASH_MSG".to_string());
        }
    }
    let unmerged = conflict_files_for_path(root);
    if !unmerged.is_empty() {
        leftovers.push(format!("unmerged index entries ({})", unmerged.join(", ")));
    }
    let mut warnings = vec![format!(
        "The target HEAD moved to {} during the merge; nothing was rolled back{}",
        head,
        if leftovers.is_empty() {
            String::new()
        } else {
            format!(
                "; leftover merge state was left untouched for inspection: {}",
                leftovers.join(", ")
            )
        }
    )];
    if !unmerged.is_empty()
        || leftovers
            .iter()
            .any(|item| item == "MERGE_HEAD" || item == "SQUASH_MSG")
    {
        if let Some(stash) = stash {
            warnings.push(format!(
                "Your uncommitted target edits were not re-applied over the leftover merge state; they are saved in {}",
                stash.describe()
            ));
        }
        return RestoreStashedChangesResult {
            critical_ok: false,
            warnings,
            rolled_back: false,
            stash_restored: stash.is_none(),
            foreign_head: Some(head),
        };
    }
    let stash_restored = match stash {
        None => true,
        Some(stash) => match apply_and_drop_stash(root, stash) {
            Ok(drop_warnings) => {
                warnings.extend(drop_warnings);
                true
            }
            Err(e) => {
                warnings.push(format!(
                    "Restoring your uncommitted target edits failed; they are saved in {}: {}",
                    stash.describe(),
                    e
                ));
                false
            }
        },
    };
    RestoreStashedChangesResult {
        critical_ok: false,
        warnings,
        rolled_back: false,
        stash_restored,
        foreign_head: Some(head),
    }
}

pub struct TargetStashGuard {
    root: PathBuf,
    pre_merge_head: Option<String>,
    own_commit: Option<String>,
    stash: Option<DirtyWorktreeStash>,
}

impl TargetStashGuard {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            pre_merge_head: None,
            own_commit: None,
            stash: None,
        }
    }

    pub fn set_own_commit(&mut self, commit: Option<String>) {
        self.own_commit = commit;
    }

    pub fn hold(&mut self, stash: Option<DirtyWorktreeStash>) {
        self.stash = stash;
    }

    pub fn set_pre_merge_head(&mut self, head: String) {
        self.pre_merge_head = Some(head);
    }

    pub fn holds_stash(&self) -> bool {
        self.stash.is_some()
    }

    pub fn release(&mut self) -> Option<DirtyWorktreeStash> {
        self.stash.take()
    }

    pub fn recover(&mut self) -> Option<(DirtyWorktreeStash, RestoreStashedChangesResult)> {
        let stash = self.stash.take()?;
        let result = rollback_merge_owned(
            &self.root,
            self.pre_merge_head.as_deref(),
            self.own_commit.as_deref(),
            Some(&stash),
        );
        Some((stash, result))
    }

    pub fn fail(&mut self, error: String) -> String {
        match self.recover() {
            None => error,
            Some((_, result)) if result.foreign_head.is_some() => {
                format!("{}; {}", error, result.warnings.join("; "))
            }
            Some((stash, result)) if result.rolled_back => format!(
                "{}; the target was rolled back to its pre-merge HEAD and {}",
                error,
                restore_details(&stash, &result)
            ),
            Some((stash, result)) => format!(
                "{}; rolling the target back to its pre-merge HEAD did not finish cleanly and {}",
                error,
                restore_details(&stash, &result)
            ),
        }
    }
}

impl Drop for TargetStashGuard {
    fn drop(&mut self) {
        let _ = self.recover();
    }
}

fn tree_paths(root: &Path, rev: &str) -> Result<std::collections::BTreeSet<String>, String> {
    Ok(run_git(root, &["ls-tree", "-r", "-z", "--name-only", rev])?
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect())
}

fn git_probe(root: &Path, args: &[&str]) -> Result<bool, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("Failed to run git {:?}: {}", args, e))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

fn untracked_and_ignored(root: &Path, path: &str) -> Result<bool, String> {
    Ok(!git_probe(
        root,
        &[
            "--literal-pathspecs",
            "ls-files",
            "--error-unmatch",
            "--",
            path,
        ],
    )? && git_probe(
        root,
        &[
            "check-ignore",
            "-q",
            "--no-index",
            "--",
            &format!("./{path}"),
        ],
    )?)
}

fn ignored_untracked_under(root: &Path, path: &str) -> Result<Vec<String>, String> {
    Ok(run_git(
        root,
        &[
            "--literal-pathspecs",
            "ls-files",
            "-z",
            "-o",
            "-i",
            "--exclude-standard",
            "--",
            path,
        ],
    )?
    .split('\0')
    .filter(|path| !path.is_empty())
    .map(str::to_string)
    .collect())
}

pub fn ignored_paths_colliding_with_incoming(
    root: &Path,
    source: &str,
) -> Result<Vec<String>, String> {
    let base = run_git(root, &["merge-base", "HEAD", source])?
        .trim()
        .to_string();
    let changed = run_git(
        root,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=AMT",
            &base,
            source,
        ],
    )?;
    let head = tree_paths(root, "HEAD")?;
    let mut head_dirs = std::collections::BTreeSet::new();
    for path in &head {
        let mut prefix = PathBuf::new();
        let components: Vec<_> = Path::new(path).components().collect();
        for component in &components[..components.len().saturating_sub(1)] {
            prefix.push(component);
            head_dirs.insert(prefix.to_string_lossy().to_string());
        }
    }
    let tracked_in_head = |path: &str| head.contains(path) || head_dirs.contains(path);
    let mut collisions = std::collections::BTreeSet::new();
    for path in changed.split('\0').filter(|path| !path.is_empty()) {
        if head_dirs.contains(path) {
            collisions.extend(ignored_untracked_under(root, path)?);
            continue;
        }
        if tracked_in_head(path) {
            continue;
        }
        let mut prefix = PathBuf::new();
        let components: Vec<_> = Path::new(path).components().collect();
        let mut blocked_by_parent = false;
        for component in &components[..components.len().saturating_sub(1)] {
            prefix.push(component);
            let parent = prefix.to_string_lossy().to_string();
            if tracked_in_head(&parent) {
                continue;
            }
            match root.join(&prefix).symlink_metadata() {
                Ok(meta) if meta.is_dir() => {}
                Ok(_) => {
                    if untracked_and_ignored(root, &parent)? {
                        collisions.insert(parent);
                    }
                    blocked_by_parent = true;
                    break;
                }
                Err(_) => {
                    blocked_by_parent = true;
                    break;
                }
            }
        }
        if blocked_by_parent {
            continue;
        }
        let Ok(meta) = root.join(path).symlink_metadata() else {
            continue;
        };
        if meta.is_dir() {
            collisions.extend(ignored_untracked_under(root, path)?);
        } else if untracked_and_ignored(root, path)? {
            collisions.insert(path.to_string());
        }
    }
    Ok(collisions.into_iter().collect())
}

fn stash_entry_for_marker(root: &Path, marker: &str) -> Result<Option<(String, String)>, String> {
    Ok(
        run_git(root, &["stash", "list", "--format=%gd%x00%H%x00%s"])?
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\0');
                let stash_ref = parts.next()?;
                let oid = parts.next()?;
                let subject = parts.next()?;
                subject
                    .contains(marker)
                    .then(|| (stash_ref.to_string(), oid.to_string()))
            })
            .find(|(_, oid)| !oid.trim().is_empty()),
    )
}

pub fn apply_and_drop_stash(
    root: &Path,
    stash: &DirtyWorktreeStash,
) -> Result<Vec<String>, String> {
    let untracked_before = stash_untracked_paths(root, stash)
        .into_iter()
        .filter(|path| root.join(path).symlink_metadata().is_err())
        .collect::<Vec<_>>();
    if let Err(e) = run_git(root, &["stash", "apply", "--index", &stash.stash_oid]) {
        let removed = remove_untracked_created_by_failed_apply(root, stash, &untracked_before);
        return Err(format!(
            "Failed to reapply target workspace changes after merge: {}; changes remain saved in git stash {}{}",
            e,
            stash.stash_oid,
            if removed.is_empty() {
                String::new()
            } else {
                format!(
                    "; removed {} untracked file(s) the failed apply had restored so a retry can succeed",
                    removed.len()
                )
            }
        ));
    }

    let mut warnings = Vec::new();
    match stash_entry_for_marker(root, &stash.stash_marker) {
        Ok(Some(stash_ref)) if stash_ref.1 == stash.stash_oid => {
            crate::locking::checkpoint_git_phase()?;
            if let Err(e) = run_git(root, &["stash", "drop", &stash_ref.0]) {
                warnings.push(format!(
                    "Target workspace changes were reapplied, but dropping git stash {} failed: {}",
                    stash_ref.0, e
                ));
            }
        }
        Ok(_) => warnings.push(format!(
            "Target workspace changes were reapplied, but exact owned git stash {} was not found for cleanup",
            stash.stash_oid
        )),
        Err(e) => warnings.push(format!(
            "Target workspace changes were reapplied, but locating git stash {} for cleanup failed: {}",
            stash.stash_oid, e
        )),
    }
    Ok(warnings)
}

fn stash_untracked_paths(root: &Path, stash: &DirtyWorktreeStash) -> Vec<String> {
    let untracked = format!("{}^3", stash.stash_oid);
    if run_git(
        root,
        &[
            "rev-parse",
            "-q",
            "--verify",
            &format!("{untracked}^{{tree}}"),
        ],
    )
    .is_err()
    {
        return Vec::new();
    }
    tree_paths(root, &untracked)
        .map(|paths| paths.into_iter().collect())
        .unwrap_or_default()
}

fn symlink_target_bytes(path: &Path) -> Option<Vec<u8>> {
    let target = std::fs::read_link(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(target.as_os_str().as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        Some(target.to_string_lossy().replace('\\', "/").into_bytes())
    }
}

fn remove_untracked_created_by_failed_apply(
    root: &Path,
    stash: &DirtyWorktreeStash,
    absent_before: &[String],
) -> Vec<String> {
    let mut removed = Vec::new();
    for path in absent_before {
        let on_disk = root.join(path);
        let Ok(meta) = on_disk.symlink_metadata() else {
            continue;
        };
        let is_symlink = meta.file_type().is_symlink();
        if !meta.is_file() && !is_symlink {
            continue;
        }
        let Ok(tracked) = run_git(root, &["ls-files", "--", path]) else {
            continue;
        };
        if !tracked.trim().is_empty() {
            continue;
        }
        let Ok(entry) = run_git(
            root,
            &["ls-tree", &format!("{}^3", stash.stash_oid), "--", path],
        ) else {
            continue;
        };
        let stash_symlink = entry.starts_with("120000 ");
        if entry.trim().is_empty() || stash_symlink != is_symlink {
            continue;
        }
        let Ok(expected) = std::process::Command::new("git")
            .args([
                "cat-file",
                "blob",
                &format!("{}^3:{}", stash.stash_oid, path),
            ])
            .current_dir(root)
            .output()
        else {
            continue;
        };
        if !expected.status.success() {
            continue;
        }
        let actual = if is_symlink {
            symlink_target_bytes(&on_disk)
        } else {
            std::fs::read(&on_disk).ok()
        };
        if actual.as_deref() == Some(expected.stdout.as_slice())
            && std::fs::remove_file(&on_disk).is_ok()
        {
            removed.push(path.clone());
        }
    }
    removed
}

pub fn reset_keeping_local_changes(root: &Path, rev: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Err(e) = run_git(root, &["reset", "--merge", "HEAD"]) {
        warnings.push(format!(
            "Failed to clear unfinished merge state in target workspace: {}",
            e
        ));
        return warnings;
    }
    if rev != "HEAD" {
        if let Err(e) = run_git(root, &["reset", "--keep", rev]) {
            warnings.push(format!(
                "Failed to reset target workspace to {}: {}",
                rev, e
            ));
        }
    }
    warnings
}

pub fn restore_stashed_changes(
    root: &Path,
    stash: &DirtyWorktreeStash,
) -> RestoreStashedChangesResult {
    let mut warnings = Vec::new();
    let mut critical_ok = true;
    let reset_warnings = reset_keeping_local_changes(root, "HEAD");
    if !reset_warnings.is_empty() {
        critical_ok = false;
    }
    let mut rolled_back = critical_ok;
    warnings.extend(reset_warnings);
    let checkout_result = match stash.original_branch.as_deref() {
        Some(branch) => checkout_branch(root, branch),
        None => run_git(root, &["checkout", &stash.original_head]).map(|_| ()),
    };
    if let Err(e) = checkout_result {
        warnings.push(format!(
            "Failed to restore original target workspace checkout: {}",
            e
        ));
        return RestoreStashedChangesResult {
            critical_ok: false,
            warnings,
            rolled_back: false,
            stash_restored: false,
            foreign_head: None,
        };
    }
    let reset_warnings = reset_keeping_local_changes(root, &stash.original_head);
    if !reset_warnings.is_empty() {
        critical_ok = false;
        rolled_back = false;
    }
    warnings.extend(reset_warnings);
    let stash_restored = match apply_and_drop_stash(root, stash) {
        Ok(drop_warnings) => {
            warnings.extend(drop_warnings);
            true
        }
        Err(e) => {
            critical_ok = false;
            warnings.push(format!(
                "Failed to restore target workspace changes after rolling back merge: {}",
                e
            ));
            false
        }
    };
    RestoreStashedChangesResult {
        critical_ok,
        warnings,
        rolled_back,
        stash_restored,
        foreign_head: None,
    }
}

pub fn commits_ahead(root: &Path, base: &str, branch: &str) -> Result<u32, String> {
    let range = format!("{}..{}", base, branch);
    let output = run_git(root, &["rev-list", "--count", &range])?;
    output
        .trim()
        .parse::<u32>()
        .map_err(|e| format!("Failed to parse commits ahead count: {}", e))
}

pub fn base_relative_ahead_behind(
    root: &Path,
    base_commit: Option<&str>,
    base_branch: Option<&str>,
) -> Option<(usize, usize)> {
    let repo = discover_repo(root).ok()?;
    let head = repo.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    let local_oid = head.peel_to_commit().ok()?.id();
    let base_oid = base_commit
        .and_then(|commit| commit_for_ref(&repo, commit).ok())
        .or_else(|| base_branch.and_then(|branch| commit_for_ref(&repo, branch).ok()))
        .and_then(|commit| git2::Oid::from_str(&commit).ok())?;
    repo.graph_ahead_behind(local_oid, base_oid).ok()
}

pub fn head_rev(root: &Path) -> Result<String, String> {
    Ok(run_git(root, &["rev-parse", "HEAD"])?.trim().to_string())
}

pub fn checkout_branch(root: &Path, branch: &str) -> Result<(), String> {
    run_git(root, &["checkout", branch]).map(|_| ())
}

pub fn diff_between(root: &Path, base: &str, branch: &str) -> String {
    let range = format!("{}...{}", base, branch);
    run_git_lossy(root, &["diff", &range])
}

pub fn commit_all(root: &Path, message: &str) -> Result<Option<String>, String> {
    ensure_no_merge_in_progress(root, "Source worktree")?;
    let status = run_git(root, &["status", "--porcelain"])?;
    if status.trim().is_empty() {
        return Ok(None);
    }
    let staged_patch = run_git(root, &["diff", "--cached", "--binary"])?;
    if let Err(e) = run_git(root, &["add", "-A"]) {
        let restore_warnings = restore_index_from_patch(root, &staged_patch);
        return Err(format_commit_failure_with_restore(
            "Failed to stage source worktree changes before auto-commit",
            &e,
            restore_warnings,
        ));
    }
    let commit_result = run_git(
        root,
        &[
            "-c",
            "user.name=Refact Agent",
            "-c",
            "user.email=agent@refact.ai",
            "commit",
            "-m",
            message,
            "--no-gpg-sign",
        ],
    );
    match commit_result {
        Ok(_) => Ok(Some(head_rev(root)?)),
        Err(e) if e.contains("nothing to commit") => {
            let restore_warnings = restore_index_from_patch(root, &staged_patch);
            if restore_warnings.is_empty() {
                Ok(None)
            } else {
                Err(format_commit_failure_with_restore(
                    "Auto-commit found nothing to commit",
                    &e,
                    restore_warnings,
                ))
            }
        }
        Err(e) => {
            let restore_warnings = restore_index_from_patch(root, &staged_patch);
            Err(format_commit_failure_with_restore(
                "Failed to auto-commit source worktree changes",
                &e,
                restore_warnings,
            ))
        }
    }
}

fn restore_index_from_patch(root: &Path, staged_patch: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Err(e) = run_git(root, &["reset", "--mixed", "HEAD"]) {
        warnings.push(format!(
            "Failed to restore source worktree index after auto-commit failure: {}",
            e
        ));
        return warnings;
    }
    if staged_patch.trim().is_empty() {
        return warnings;
    }

    let mut patch_file = match tempfile::NamedTempFile::new() {
        Ok(file) => file,
        Err(e) => {
            warnings.push(format!(
                "Failed to create temporary staged patch for source index restoration: {}",
                e
            ));
            return warnings;
        }
    };
    if let Err(e) = std::io::Write::write_all(&mut patch_file, staged_patch.as_bytes()) {
        warnings.push(format!(
            "Failed to write temporary staged patch for source index restoration: {}",
            e
        ));
        return warnings;
    }
    let patch_path = patch_file.path().to_string_lossy().to_string();
    if let Err(e) = run_git(
        root,
        &[
            "apply",
            "--cached",
            "--binary",
            "--whitespace=nowarn",
            &patch_path,
        ],
    ) {
        warnings.push(format!(
            "Failed to restore source worktree staged changes after auto-commit failure: {}",
            e
        ));
    }
    warnings
}

fn format_commit_failure_with_restore(
    context: &str,
    error: &str,
    restore_warnings: Vec<String>,
) -> String {
    if restore_warnings.is_empty() {
        format!("{}: {}; source worktree index was restored", context, error)
    } else {
        format!(
            "{}: {}; source worktree index restoration warnings: {}",
            context,
            error,
            restore_warnings.join("; ")
        )
    }
}

pub fn conflict_files_for_path(root: &Path) -> Vec<String> {
    run_git(root, &["diff", "--name-only", "--diff-filter=U"])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|file| !file.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub fn cleanup_failed_merge(root: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    if has_merge_in_progress(root) {
        if let Err(e) = run_git(root, &["merge", "--abort"]) {
            warnings.push(format!("Failed to abort merge: {}", e));
        }
    } else {
        warnings.extend(reset_keeping_local_changes(root, "HEAD"));
    }
    warnings
}

pub fn preflight_merge_conflicts(
    source_root: &Path,
    target_branch: &str,
    source_branch: &str,
    strategy: &str,
) -> Result<PreflightMergeResult, String> {
    crate::locking::checkpoint_git_phase()?;
    let temp = tempfile::Builder::new()
        .prefix("refact-merge-preflight-")
        .tempdir()
        .map_err(|e| format!("Failed to create merge preflight directory: {}", e))?;
    let canonical_temp = dunce::canonicalize(temp.path())
        .map_err(|e| format!("Failed to canonicalize merge preflight directory: {}", e))?;
    let _ = temp.keep();
    let temp = canonical_temp;
    let preflight_path = temp.join("worktree");
    let retained = |reason: String| {
        format!(
            "Merge preflight stopped; inspect retained directory '{}' (worktree '{}'); do not replay: {}",
            temp.display(),
            preflight_path.display(),
            reason
        )
    };
    let preflight_str = preflight_path
        .to_str()
        .ok_or_else(|| retained("Merge preflight path is not valid UTF-8".to_string()))?;
    run_git(
        source_root,
        &["worktree", "add", "--detach", preflight_str, target_branch],
    )
    .map_err(&retained)?;
    crate::locking::checkpoint_git_phase().map_err(&retained)?;
    let merge_args = if strategy == "squash" {
        vec!["merge", "--squash", source_branch]
    } else {
        vec!["merge", "--no-commit", "--no-ff", source_branch]
    };
    let merge_output = Command::new("git")
        .args(&merge_args)
        .current_dir(&preflight_path)
        .output()
        .map_err(|e| retained(format!("Merge preflight Git outcome unknown: {}", e)))?;
    crate::locking::checkpoint_git_phase().map_err(&retained)?;
    if !merge_output.status.success() && merge_output.status.code() != Some(1) {
        return Err(retained(format!(
            "Merge preflight did not complete normally: {}",
            merge_output.status
        )));
    }
    let conflicts = run_git(&preflight_path, &["diff", "--name-only", "--diff-filter=U"])
        .map_err(&retained)?
        .lines()
        .map(str::trim)
        .filter(|file| !file.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if !merge_output.status.success() && conflicts.is_empty() {
        return Err(retained(format!(
            "Merge preflight failed without reported conflicts: {}",
            String::from_utf8_lossy(&merge_output.stderr)
        )));
    }
    run_git(
        source_root,
        &["worktree", "remove", "--force", preflight_str],
    )
    .map_err(&retained)?;
    crate::locking::checkpoint_git_phase().map_err(&retained)?;
    std::fs::remove_dir(&temp)
        .map_err(|e| retained(format!("Failed to remove empty preflight directory: {}", e)))?;
    Ok(PreflightMergeResult {
        conflicts: if merge_output.status.success() {
            Vec::new()
        } else {
            conflicts
        },
        warnings: Vec::new(),
    })
}

#[allow(dead_code)]
pub fn diff_head_to_workdir_as_string(
    repository: &Repository,
    max_size: usize,
) -> Result<String, String> {
    let mut diff_options = DiffOptions::new();
    diff_options.include_untracked(true);
    diff_options.recurse_untracked_dirs(true);
    let head = repository
        .head()
        .and_then(|head_ref| head_ref.peel_to_tree())
        .map_err(|e| format!("Failed to get HEAD tree: {}", e))?;
    let diff = repository
        .diff_tree_to_workdir(Some(&head), Some(&mut diff_options))
        .map_err(|e| format!("Failed to generate diff: {}", e))?;
    let mut diff_str = String::new();
    diff.print(git2::DiffFormat::Patch, |_, _, line| {
        let line_content = std::str::from_utf8(line.content()).unwrap_or("");
        if diff_str.len() + line_content.len() < max_size {
            diff_str.push(line.origin());
            diff_str.push_str(line_content);
        }
        true
    })
    .map_err(|e| format!("Failed to print diff: {}", e))?;
    Ok(diff_str)
}

#[cfg(test)]
mod diff_metadata_tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) -> String {
        run_git(root, args).unwrap()
    }

    #[test]
    fn temporary_merge_checkout_keeps_canonical_identity_after_materialization() {
        let temp = tempfile::tempdir().unwrap();
        let source = dunce::canonicalize(temp.path()).unwrap();
        git(&source, &["init", "-b", "main"]);
        git(&source, &["config", "core.autocrlf", "false"]);
        git(&source, &["config", "user.email", "merge@example.test"]);
        git(&source, &["config", "user.name", "Merge Test"]);
        std::fs::write(source.join("file.txt"), "base\n").unwrap();
        git(&source, &["add", "file.txt"]);
        git(&source, &["commit", "-m", "base"]);
        git(&source, &["branch", "target"]);
        std::fs::write(source.join("file.txt"), "user changes\n").unwrap();

        let checkout = prepare_merge_checkout(&source, "target").unwrap();
        assert!(checkout.is_temporary());
        assert!(!checkout.root.exists());
        assert_eq!(
            checkout.root,
            dunce::canonicalize(checkout.root.parent().unwrap())
                .unwrap()
                .join("worktree")
        );
        checkout.materialize().unwrap();
        assert_eq!(checkout.root, dunce::canonicalize(&checkout.root).unwrap());
        let target = list_git_worktrees_checked(&source)
            .unwrap()
            .into_iter()
            .find(|entry| entry.branch.as_deref() == Some("target"))
            .unwrap();
        assert_eq!(dunce::canonicalize(target.root).unwrap(), checkout.root);
        checkout.cleanup().unwrap();
        assert!(!checkout.root.exists());
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "user changes\n"
        );
    }

    #[test]
    fn detailed_diff_preserves_rename_and_resolved_fallback_base() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init"]);
        git(root, &["config", "user.email", "diff@example.test"]);
        git(root, &["config", "user.name", "Diff Test"]);
        std::fs::write(root.join("old name.txt"), "original content\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "base"]);
        git(root, &["branch", "diff-base"]);
        let base = git(root, &["rev-parse", "HEAD"]).trim().to_string();
        git(root, &["mv", "old name.txt", "new name.txt"]);
        git(root, &["commit", "-m", "rename"]);
        std::fs::write(root.join("new name.txt"), "staged content\n").unwrap();
        git(root, &["add", "."]);
        std::fs::write(root.join("new name.txt"), "unstaged content\n").unwrap();
        std::fs::write(root.join("untracked.txt"), "new\n").unwrap();
        let diff = diff_for_path(root, None, Some("diff-base"), 50_000).unwrap();
        assert_eq!(diff.resolved_base_commit.as_deref(), Some(base.as_str()));
        let renamed = diff
            .files
            .iter()
            .find(|file| file.source == "committed")
            .unwrap();
        assert_eq!(renamed.path, "new name.txt");
        assert_eq!(renamed.old_path.as_deref(), Some("old name.txt"));
        assert_eq!(renamed.status, "R");
        assert_eq!(
            diff.files
                .iter()
                .map(|file| (file.source.as_str(), file.path.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("committed", "new name.txt"),
                ("staged", "new name.txt"),
                ("unstaged", "new name.txt"),
                ("untracked", "untracked.txt")
            ]
        );
        assert!(diff_for_path(root, Some("unknown-base"), None, 100).is_err());
    }
}

#[cfg(test)]
mod rollback_tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) -> String {
        run_git(root, args).unwrap()
    }

    fn read(root: &Path, name: &str) -> String {
        std::fs::read_to_string(root.join(name)).unwrap()
    }

    fn repo_with_conflicting_branch() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init"]);
        git(root, &["checkout", "-b", "main"]);
        git(root, &["config", "core.autocrlf", "false"]);
        git(root, &["config", "user.email", "rollback@example.test"]);
        git(root, &["config", "user.name", "Rollback Test"]);
        for name in ["merged.txt", "clean.txt", "other.txt"] {
            std::fs::write(root.join(name), "base\n").unwrap();
        }
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "base"]);
        git(root, &["checkout", "-b", "feature"]);
        std::fs::write(root.join("merged.txt"), "feature\n").unwrap();
        std::fs::write(root.join("clean.txt"), "feature\n").unwrap();
        git(root, &["commit", "-am", "feature"]);
        git(root, &["checkout", "main"]);
        std::fs::write(root.join("merged.txt"), "main\n").unwrap();
        git(root, &["commit", "-am", "main"]);
        temp
    }

    #[test]
    fn failed_squash_cleanup_keeps_edits_the_merge_did_not_touch() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        std::fs::write(root.join("other.txt"), "another chat\n").unwrap();
        assert!(run_git(root, &["merge", "--squash", "feature"]).is_err());
        assert!(!has_merge_in_progress(root));
        assert_eq!(read(root, "clean.txt"), "feature\n");

        assert_eq!(cleanup_failed_merge(root), Vec::<String>::new());
        assert_eq!(read(root, "other.txt"), "another chat\n");
        assert_eq!(read(root, "merged.txt"), "main\n");
        assert_eq!(read(root, "clean.txt"), "base\n");
        assert_eq!(git(root, &["status", "--porcelain"]).trim(), "M other.txt");
    }

    #[test]
    fn failed_squash_cleanup_refuses_to_overwrite_an_edit_on_a_merged_file() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        assert!(run_git(root, &["merge", "--squash", "feature"]).is_err());
        std::fs::write(root.join("clean.txt"), "another chat\n").unwrap();

        let warnings = cleanup_failed_merge(root);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(read(root, "clean.txt"), "another chat\n");
    }

    #[test]
    fn undoing_a_merge_commit_keeps_unrelated_edits_and_stops_on_a_collision() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        let before = git(root, &["rev-parse", "HEAD"]).trim().to_string();
        std::fs::write(root.join("clean.txt"), "merged\n").unwrap();
        git(root, &["commit", "-am", "merge commit stand-in"]);
        std::fs::write(root.join("other.txt"), "another chat\n").unwrap();

        assert_eq!(
            reset_keeping_local_changes(root, &before),
            Vec::<String>::new()
        );
        assert_eq!(git(root, &["rev-parse", "HEAD"]).trim(), before);
        assert_eq!(read(root, "clean.txt"), "base\n");
        assert_eq!(read(root, "other.txt"), "another chat\n");

        std::fs::write(root.join("clean.txt"), "merged again\n").unwrap();
        git(root, &["add", "clean.txt"]);
        git(root, &["commit", "-m", "second stand-in"]);
        let merged = git(root, &["rev-parse", "HEAD"]).trim().to_string();
        std::fs::write(root.join("clean.txt"), "another chat\n").unwrap();

        let warnings = reset_keeping_local_changes(root, &before);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(git(root, &["rev-parse", "HEAD"]).trim(), merged);
        assert_eq!(read(root, "clean.txt"), "another chat\n");
        assert_eq!(read(root, "other.txt"), "another chat\n");
    }

    fn dirty_user_edits(root: &Path) {
        std::fs::write(root.join("merged.txt"), "staged user\n").unwrap();
        git(root, &["add", "merged.txt"]);
        std::fs::write(root.join("merged.txt"), "unstaged user\n").unwrap();
        std::fs::write(root.join("notes.txt"), "untracked user\n").unwrap();
    }

    fn assert_user_edits(root: &Path, head: &str) {
        assert_eq!(git(root, &["rev-parse", "HEAD"]).trim(), head);
        assert_eq!(git(root, &["show", ":merged.txt"]), "staged user\n");
        assert_eq!(read(root, "merged.txt"), "unstaged user\n");
        assert_eq!(read(root, "notes.txt"), "untracked user\n");
        assert!(git(root, &["stash", "list"]).trim().is_empty());
        assert!(!has_merge_in_progress(root));
    }

    #[test]
    fn stash_post_validation_failure_returns_the_stash_handle() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        dirty_user_edits(root);
        let head = git(root, &["rev-parse", "HEAD"]).trim().to_string();
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let lose_after = std::sync::Arc::new(AtomicUsize::new(usize::MAX));
        let (seen, limit) = (calls.clone(), lose_after.clone());
        let authority = crate::locking::MergeAuthority(std::sync::Arc::new(move || {
            if seen.fetch_add(1, Ordering::SeqCst) >= limit.load(Ordering::SeqCst) {
                Err("lease revoked".into())
            } else {
                Ok(())
            }
        }));
        let probe = crate::locking::with_git_phase(Some(authority.clone()), || {
            stash_dirty_worktree_retaining(root, "Target", Some("main"))
        })
        .unwrap()
        .unwrap();
        let total = calls.swap(0, Ordering::SeqCst);
        assert!(rollback_merge_and_restore_stash(root, None, Some(&probe)).critical_ok);
        assert_user_edits(root, &head);

        lose_after.store(total - 1, Ordering::SeqCst);
        let (error, stash) = crate::locking::with_git_phase(Some(authority), || {
            stash_dirty_worktree_retaining(root, "Target", Some("main"))
        })
        .unwrap_err();
        assert!(error.contains("lease revoked"), "{error}");
        let stash = stash.expect("a created stash must be returned with the error");
        assert!(!git(root, &["stash", "list"]).trim().is_empty());

        let revoked = crate::locking::MergeAuthority(std::sync::Arc::new(|| Err("revoked".into())));
        let restored = crate::locking::with_git_phase(Some(revoked), || {
            rollback_merge_and_restore_stash(root, Some(&head), Some(&stash))
        });
        assert!(restored.critical_ok, "{:?}", restored.warnings);
        assert_user_edits(root, &head);
    }

    #[test]
    fn rollback_clears_squash_and_merge_state_then_restores_edits() {
        for strategy in [
            &["merge", "--squash", "feature"][..],
            &["merge", "--no-ff", "--no-commit", "feature"][..],
        ] {
            let temp = repo_with_conflicting_branch();
            let root = temp.path();
            git(root, &["checkout", "-b", "clean-base", "HEAD~1"]);
            dirty_user_edits(root);
            let head = git(root, &["rev-parse", "HEAD"]).trim().to_string();
            let stash = stash_dirty_worktree(root, "Target", None).unwrap().unwrap();
            git(root, strategy);
            assert_eq!(read(root, "clean.txt"), "feature\n");

            let restored = rollback_merge_and_restore_stash(root, Some(&head), Some(&stash));

            assert!(
                restored.critical_ok,
                "{strategy:?}: {:?}",
                restored.warnings
            );
            assert_user_edits(root, &head);
            assert_eq!(read(root, "clean.txt"), "base\n");
            for name in ["MERGE_HEAD", "MERGE_MSG", "SQUASH_MSG"] {
                let path = git(root, &["rev-parse", "--git-path", name]);
                assert!(!root.join(path.trim()).exists(), "{strategy:?}: {name}");
            }
        }
    }

    #[test]
    fn failed_apply_removes_only_untracked_files_it_restored_so_retry_succeeds() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        let pre = git(root, &["rev-parse", "HEAD"]).trim().to_string();
        std::fs::write(root.join("merged.txt"), "user\n").unwrap();
        std::fs::write(root.join("notes.txt"), "untracked user\n").unwrap();
        let stash = stash_dirty_worktree(root, "Target", Some("main"))
            .unwrap()
            .unwrap();
        std::fs::write(root.join("merged.txt"), "incoming\n").unwrap();
        git(root, &["commit", "-am", "incoming overlap"]);

        let error = apply_and_drop_stash(root, &stash).unwrap_err();

        assert!(error.contains("removed 1 untracked file"), "{error}");
        assert!(!root.join("notes.txt").exists());
        let warnings = reset_keeping_local_changes(root, &pre);
        assert!(warnings.is_empty(), "{warnings:?}");
        apply_and_drop_stash(root, &stash).unwrap();
        assert_eq!(read(root, "merged.txt"), "user\n");
        assert_eq!(read(root, "notes.txt"), "untracked user\n");
        assert!(git(root, &["stash", "list"]).trim().is_empty());
    }

    #[test]
    fn failed_apply_keeps_untracked_files_that_existed_or_differ() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        std::fs::write(root.join("merged.txt"), "user\n").unwrap();
        std::fs::write(root.join("notes.txt"), "untracked user\n").unwrap();
        let stash = stash_dirty_worktree(root, "Target", Some("main"))
            .unwrap()
            .unwrap();
        std::fs::write(root.join("merged.txt"), "incoming\n").unwrap();
        git(root, &["commit", "-am", "incoming overlap"]);
        std::fs::write(root.join("notes.txt"), "someone else\n").unwrap();

        assert!(apply_and_drop_stash(root, &stash).is_err());

        assert_eq!(read(root, "notes.txt"), "someone else\n");
        assert!(!git(root, &["stash", "list"]).trim().is_empty());
    }

    #[test]
    fn ignored_collision_probe_names_files_and_blocking_parents() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        std::fs::write(root.join(".git/info/exclude"), "*.log\nout\n").unwrap();
        git(root, &["checkout", "feature"]);
        std::fs::create_dir(root.join("out")).unwrap();
        std::fs::write(root.join("build.log"), "tracked\n").unwrap();
        std::fs::write(root.join("out/result.txt"), "tracked\n").unwrap();
        git(root, &["add", "-f", "build.log", "out/result.txt"]);
        git(root, &["commit", "-m", "tracked outputs"]);
        git(root, &["checkout", "main"]);
        assert!(!root.join("out").exists());
        assert!(ignored_paths_colliding_with_incoming(root, "feature")
            .unwrap()
            .is_empty());
        std::fs::write(root.join("build.log"), "precious\n").unwrap();
        std::fs::write(root.join("out"), "precious file\n").unwrap();

        assert_eq!(
            ignored_paths_colliding_with_incoming(root, "feature").unwrap(),
            vec!["build.log".to_string(), "out".to_string()]
        );
    }

    fn simple_repo() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        git(root, &["init"]);
        git(root, &["checkout", "-b", "main"]);
        git(root, &["config", "core.autocrlf", "false"]);
        git(root, &["config", "user.email", "probe@example.test"]);
        git(root, &["config", "user.name", "Probe Test"]);
        temp
    }

    fn commit_all_files(root: &Path, message: &str) {
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", message]);
    }

    #[test]
    fn ignored_collision_probe_ignores_tracked_dir_replaced_by_file() {
        let temp = simple_repo();
        let root = temp.path();
        std::fs::create_dir(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/x.md"), "x\n").unwrap();
        commit_all_files(root, "base");
        git(root, &["checkout", "-b", "feature"]);
        git(root, &["rm", "-r", "-q", "docs"]);
        std::fs::write(root.join("docs"), "now a file\n").unwrap();
        commit_all_files(root, "dir to file");
        git(root, &["checkout", "main"]);

        assert!(ignored_paths_colliding_with_incoming(root, "feature")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn ignored_collision_probe_flags_ignored_contents_of_tracked_dir_replaced_by_file() {
        let temp = simple_repo();
        let root = temp.path();
        std::fs::create_dir(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/x.md"), "x\n").unwrap();
        std::fs::write(root.join(".gitignore"), "*.tmp\n").unwrap();
        commit_all_files(root, "base");
        git(root, &["checkout", "-b", "feature"]);
        git(root, &["rm", "-r", "-q", "docs"]);
        std::fs::write(root.join("docs"), "now a file\n").unwrap();
        commit_all_files(root, "dir to file");
        git(root, &["checkout", "main"]);
        std::fs::write(root.join("docs/cache.tmp"), "precious\n").unwrap();

        assert_eq!(
            ignored_paths_colliding_with_incoming(root, "feature").unwrap(),
            vec!["docs/cache.tmp".to_string()]
        );
    }

    #[test]
    fn ignored_collision_probe_treats_glob_characters_literally() {
        let temp = simple_repo();
        let root = temp.path();
        std::fs::create_dir(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/i.tsx"), "tracked\n").unwrap();
        std::fs::write(root.join(".gitignore"), "pages/\\[id\\].tsx\n").unwrap();
        commit_all_files(root, "base");
        git(root, &["checkout", "-b", "feature"]);
        std::fs::write(root.join("pages/[id].tsx"), "incoming\n").unwrap();
        git(root, &["add", "-f", "pages/[id].tsx"]);
        git(root, &["commit", "-m", "incoming"]);
        git(root, &["checkout", "main"]);
        std::fs::write(root.join("pages/[id].tsx"), "precious\n").unwrap();

        assert_eq!(
            ignored_paths_colliding_with_incoming(root, "feature").unwrap(),
            vec!["pages/[id].tsx".to_string()]
        );
    }

    #[test]
    fn ignored_collision_probe_fails_closed_when_git_fails() {
        let temp = simple_repo();
        let root = temp.path();
        assert!(untracked_and_ignored(&root.join("missing"), "x").is_err());
    }

    #[test]
    fn landed_commit_recognises_only_this_merges_commit() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        git(root, &["checkout", "-q", "-b", "clean-base", "HEAD~1"]);
        let before = head_rev(root).unwrap();
        let source = git(root, &["rev-parse", "feature"]).trim().to_string();
        assert_eq!(
            landed_commit(
                root,
                &before,
                Landing::Merge {
                    source: &source
                }
            ),
            None
        );

        std::fs::write(root.join("extra.txt"), "x\n").unwrap();
        git(root, &["add", "extra.txt"]);
        git(root, &["commit", "-m", "unrelated child"]);
        let tree = git(root, &["rev-parse", "HEAD^{tree}"]).trim().to_string();
        assert_eq!(
            landed_commit(
                root,
                &before,
                Landing::Merge {
                    source: &source
                }
            ),
            None
        );
        git(root, &["reset", "-q", "--hard", &before]);
        git(root, &["merge", "-q", "--squash", "feature"]);
        let squash_tree = git(root, &["write-tree"]).trim().to_string();
        assert_ne!(tree, squash_tree);
        git(root, &["reset", "-q", "--hard", &before]);
        std::fs::write(root.join("extra.txt"), "x\n").unwrap();
        git(root, &["add", "extra.txt"]);
        git(root, &["commit", "-m", "unrelated child"]);
        assert_eq!(
            landed_commit(
                root,
                &before,
                Landing::Squash {
                    tree: &squash_tree
                }
            ),
            None
        );

        git(root, &["reset", "-q", "--hard", &before]);
        git(root, &["merge", "-q", "--squash", "feature"]);
        git(root, &["commit", "-q", "-m", "squash"]);
        let squashed = head_rev(root).unwrap();
        assert_eq!(
            landed_commit(
                root,
                &before,
                Landing::Squash {
                    tree: &squash_tree
                }
            ),
            Some(squashed)
        );

        git(root, &["reset", "-q", "--hard", &before]);
        git(root, &["merge", "-q", "--no-ff", "--no-edit", "feature"]);
        let merged = head_rev(root).unwrap();
        assert_eq!(
            landed_commit(
                root,
                &before,
                Landing::Merge {
                    source: &source
                }
            ),
            Some(merged)
        );
        assert_eq!(
            landed_commit(
                root,
                &before,
                Landing::Merge {
                    source: &before
                }
            ),
            None
        );
    }

    #[test]
    fn squash_identity_accepts_a_restaged_tree_only_from_this_operation() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        git(root, &["checkout", "-q", "-b", "clean-base", "HEAD~1"]);
        let before = head_rev(root).unwrap();
        git(root, &["merge", "-q", "--squash", "feature"]);
        let tree = git(root, &["write-tree"]).trim().to_string();
        std::fs::write(root.join("other.txt"), "formatted\n").unwrap();
        git(root, &["add", "other.txt"]);
        git(root, &["commit", "-q", "-m", "user squash"]);
        let marker = format!("Refact-Merge-Operation: {}", Uuid::new_v4());
        let landing = Landing::SquashOperation {
            tree: &tree,
            marker: &marker,
        };
        assert_eq!(landed_commit(root, &before, landing), None);

        git(root, &["reset", "-q", "--hard", &before]);
        git(root, &["merge", "-q", "--squash", "feature"]);
        std::fs::write(root.join("other.txt"), "formatted\n").unwrap();
        git(root, &["add", "other.txt"]);
        run_git_with_refact_author(root, &["commit", "-q", "-m", "agent squash"]).unwrap();
        assert_eq!(landed_commit(root, &before, landing), None);
        let message = format!("agent squash\n\n{marker}");
        run_git_with_refact_author(root, &["commit", "--amend", "-q", "-m", &message]).unwrap();
        let own = head_rev(root).unwrap();
        assert_eq!(landed_commit(root, &before, landing), Some(own.clone()));

        git(root, &["commit", "-q", "--allow-empty", "-m", "hook"]);
        assert_eq!(landed_commit(root, &before, landing), None);
        assert_eq!(landed_commit_below_head(root, &before, landing), Some(own));
    }

    #[test]
    fn rollback_over_a_foreign_head_leaves_merge_state_alone_and_names_it() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        let pre = head_rev(root).unwrap();
        dirty_user_edits(root);
        let stash = stash_dirty_worktree(root, "Target", Some("main"))
            .unwrap()
            .unwrap();
        std::fs::write(root.join("foreign.txt"), "other writer\n").unwrap();
        git(root, &["add", "foreign.txt"]);
        git(root, &["commit", "-m", "foreign"]);
        let foreign = head_rev(root).unwrap();
        assert!(run_git(root, &["merge", "--no-ff", "feature"]).is_err());
        assert!(has_merge_in_progress(root));

        let restored = rollback_merge_owned(root, Some(&pre), None, Some(&stash));

        assert_eq!(head_rev(root).unwrap(), foreign);
        assert!(has_merge_in_progress(root));
        assert_eq!(
            conflict_files_for_path(root),
            vec!["merged.txt".to_string()]
        );
        assert!(!restored.stash_restored);
        assert!(!restored.critical_ok);
        let first = &restored.warnings[0];
        assert!(first.contains("leftover merge state"), "{first}");
        assert!(first.contains("MERGE_HEAD"), "{first}");
        assert!(
            first.contains("unmerged index entries (merged.txt)"),
            "{first}"
        );
        assert!(
            restored
                .warnings
                .iter()
                .any(|warning| warning.contains(stash.stash_oid())),
            "{:?}",
            restored.warnings
        );
        assert!(!git(root, &["stash", "list"]).trim().is_empty());
    }

    #[test]
    fn refact_author_commits_force_the_merge_committer_over_config_overrides() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        git(root, &["config", "committer.email", "dev@example.test"]);
        git(root, &["config", "committer.name", "Dev Override"]);

        run_git_with_refact_author(root, &["commit", "--allow-empty", "-q", "-m", "identity"])
            .unwrap();

        assert_eq!(
            git(root, &["log", "-1", "--format=%ce"]).trim(),
            MERGE_COMMITTER_EMAIL
        );
        assert_eq!(
            git(root, &["log", "-1", "--format=%cn"]).trim(),
            MERGE_COMMITTER_NAME
        );
    }

    #[test]
    fn rollback_over_a_foreign_head_keeps_a_staged_squash_and_names_the_stash() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        git(root, &["checkout", "-b", "side"]);
        std::fs::write(root.join("side.txt"), "side\n").unwrap();
        git(root, &["add", "side.txt"]);
        git(root, &["commit", "-m", "side"]);
        git(root, &["checkout", "main"]);
        let pre = head_rev(root).unwrap();
        dirty_user_edits(root);
        let stash = stash_dirty_worktree(root, "Target", Some("main"))
            .unwrap()
            .unwrap();
        run_git(root, &["merge", "--squash", "side"]).unwrap();
        let tree = git(root, &["rev-parse", "HEAD^{tree}"]);
        let foreign = git(
            root,
            &["commit-tree", tree.trim(), "-p", &pre, "-m", "foreign"],
        )
        .trim()
        .to_string();
        git(root, &["update-ref", "HEAD", &foreign]);

        let restored = rollback_merge_owned(root, Some(&pre), None, Some(&stash));

        assert_eq!(head_rev(root).unwrap(), foreign);
        assert!(!restored.stash_restored);
        assert!(
            restored.warnings[0].contains("SQUASH_MSG"),
            "{:?}",
            restored.warnings
        );
        assert!(
            restored
                .warnings
                .iter()
                .any(|warning| warning.contains(stash.stash_oid())),
            "{:?}",
            restored.warnings
        );
        assert!(git(root, &["diff", "--cached", "--name-only"]).contains("side.txt"));
        assert!(!git(root, &["stash", "list"]).trim().is_empty());
    }

    #[test]
    fn safety_rollback_retains_stash_when_head_is_unreadable_or_branch_changed() {
        for unreadable in [false, true] {
            let temp = repo_with_conflicting_branch();
            let root = temp.path();
            dirty_user_edits(root);
            let stash = stash_dirty_worktree(root, "Target", Some("main"))
                .unwrap()
                .unwrap();
            if unreadable {
                std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/missing\n").unwrap();
            } else {
                git(root, &["checkout", "-b", "other-branch"]);
            }
            let result = rollback_merge_owned(root, None, None, Some(&stash));
            assert!(!result.critical_ok, "{result:?}");
            assert!(!result.rolled_back, "{result:?}");
            assert!(!result.stash_restored, "{result:?}");
            assert!(git(root, &["stash", "list"]).contains("Refact worktree merge target"));
            if unreadable {
                assert_eq!(
                    std::fs::read_to_string(root.join(".git/HEAD")).unwrap(),
                    "ref: refs/heads/missing\n"
                );
            } else {
                assert_eq!(
                    git(root, &["branch", "--show-current"]).trim(),
                    "other-branch"
                );
            }
        }
    }

    #[test]
    fn safety_rollback_without_explicit_baseline_preserves_foreign_head() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        dirty_user_edits(root);
        let stash = stash_dirty_worktree(root, "Target", Some("main"))
            .unwrap()
            .unwrap();
        git(root, &["commit", "--allow-empty", "-m", "foreign"]);
        let foreign = head_rev(root).unwrap();
        let result = rollback_merge_owned(root, None, None, Some(&stash));
        assert_eq!(head_rev(root).unwrap(), foreign);
        assert!(!result.rolled_back, "{result:?}");
        assert_eq!(result.foreign_head.as_deref(), Some(foreign.as_str()));
        assert_user_edits(root, &foreign);
    }

    #[test]
    fn rollback_never_moves_head_away_from_a_foreign_commit() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        let pre = head_rev(root).unwrap();
        dirty_user_edits(root);
        let stash = stash_dirty_worktree(root, "Target", Some("main"))
            .unwrap()
            .unwrap();
        std::fs::write(root.join("foreign.txt"), "other writer\n").unwrap();
        git(root, &["add", "foreign.txt"]);
        git(root, &["commit", "-m", "foreign"]);
        let foreign = head_rev(root).unwrap();

        let restored = rollback_merge_owned(root, Some(&pre), Some(&pre), Some(&stash));

        assert_eq!(head_rev(root).unwrap(), foreign);
        assert_eq!(restored.foreign_head.as_deref(), Some(foreign.as_str()));
        assert!(!restored.rolled_back);
        assert!(restored.stash_restored, "{:?}", restored.warnings);
        assert!(restored
            .warnings
            .iter()
            .any(|warning| warning.contains(&format!("moved to {foreign}"))
                && warning.contains("nothing was rolled back")));
        assert_eq!(read(root, "foreign.txt"), "other writer\n");
        assert_eq!(git(root, &["show", ":merged.txt"]), "staged user\n");
        assert_eq!(read(root, "merged.txt"), "unstaged user\n");
        assert_eq!(read(root, "notes.txt"), "untracked user\n");
        assert!(git(root, &["stash", "list"]).trim().is_empty());
    }

    #[test]
    fn ignored_collision_probe_handles_paths_starting_with_a_colon() {
        let temp = simple_repo();
        let root = temp.path();
        std::fs::write(root.join("file.txt"), "base\n").unwrap();
        std::fs::write(root.join(".gitignore"), ":precious.dat\n*.log\n").unwrap();
        commit_all_files(root, "base");
        git(root, &["checkout", "-b", "feature"]);
        for path in [":precious.dat", ":(x).log"] {
            std::fs::write(root.join(path), "incoming\n").unwrap();
            git(root, &["add", "-f", "--", &format!("./{path}")]);
        }
        git(root, &["commit", "-m", "incoming"]);
        git(root, &["checkout", "main"]);
        for path in [":precious.dat", ":(x).log"] {
            std::fs::write(root.join(path), "precious\n").unwrap();
        }

        assert_eq!(
            ignored_paths_colliding_with_incoming(root, "feature").unwrap(),
            vec![":(x).log".to_string(), ":precious.dat".to_string()]
        );
    }

    #[test]
    fn ignored_collision_probe_ignores_tracked_file_replaced_by_dir() {
        let temp = simple_repo();
        let root = temp.path();
        std::fs::write(root.join("docs"), "a file\n").unwrap();
        commit_all_files(root, "base");
        git(root, &["checkout", "-b", "feature"]);
        git(root, &["rm", "-q", "docs"]);
        std::fs::create_dir(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/a.md"), "a\n").unwrap();
        commit_all_files(root, "file to dir");
        git(root, &["checkout", "main"]);

        assert!(ignored_paths_colliding_with_incoming(root, "feature")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn ignored_collision_probe_ignores_untouched_path_the_target_stopped_tracking() {
        let temp = simple_repo();
        let root = temp.path();
        std::fs::write(root.join("config.local"), "base config\n").unwrap();
        std::fs::write(root.join("file.txt"), "base\n").unwrap();
        commit_all_files(root, "base");
        git(root, &["branch", "feature"]);
        git(root, &["rm", "-q", "--cached", "config.local"]);
        std::fs::write(root.join(".gitignore"), "config.local\n").unwrap();
        commit_all_files(root, "stop tracking config.local");
        std::fs::write(root.join("config.local"), "user local config\n").unwrap();
        git(root, &["checkout", "-q", "feature"]);
        std::fs::write(root.join("file.txt"), "feature\n").unwrap();
        git(root, &["commit", "-qam", "feature change"]);
        git(root, &["checkout", "-q", "main"]);
        std::fs::write(root.join("config.local"), "user local config\n").unwrap();

        assert!(ignored_paths_colliding_with_incoming(root, "feature")
            .unwrap()
            .is_empty());
        git(root, &["merge", "--no-ff", "--no-edit", "feature"]);
        assert_eq!(read(root, "config.local"), "user local config\n");
    }

    #[test]
    fn ignored_collision_probe_skips_unignored_occupants_and_flags_ignored_dir_contents() {
        let temp = simple_repo();
        let root = temp.path();
        std::fs::write(root.join("file.txt"), "base\n").unwrap();
        std::fs::write(root.join(".gitignore"), "cache/\n").unwrap();
        commit_all_files(root, "base");
        git(root, &["checkout", "-b", "feature"]);
        std::fs::write(root.join("plain.txt"), "incoming\n").unwrap();
        std::fs::write(root.join("cache"), "incoming file at an ignored dir\n").unwrap();
        git(root, &["add", "-f", "plain.txt", "cache"]);
        git(root, &["commit", "-m", "incoming"]);
        git(root, &["checkout", "main"]);
        std::fs::write(root.join("plain.txt"), "untracked, not ignored\n").unwrap();
        std::fs::create_dir(root.join("cache")).unwrap();
        std::fs::write(root.join("cache/blob.bin"), "precious\n").unwrap();

        assert_eq!(
            ignored_paths_colliding_with_incoming(root, "feature").unwrap(),
            vec!["cache/blob.bin".to_string()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_apply_removes_restored_untracked_symlink_so_retry_succeeds() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        let pre = git(root, &["rev-parse", "HEAD"]).trim().to_string();
        std::fs::write(root.join("merged.txt"), "user\n").unwrap();
        std::os::unix::fs::symlink("merged.txt", root.join("link")).unwrap();
        let stash = stash_dirty_worktree(root, "Target", Some("main"))
            .unwrap()
            .unwrap();
        assert!(root.join("link").symlink_metadata().is_err());
        std::fs::write(root.join("merged.txt"), "incoming\n").unwrap();
        git(root, &["commit", "-am", "incoming overlap"]);

        let error = apply_and_drop_stash(root, &stash).unwrap_err();

        assert!(error.contains("removed 1 untracked file"), "{error}");
        assert!(root.join("link").symlink_metadata().is_err());
        assert!(reset_keeping_local_changes(root, &pre).is_empty());
        apply_and_drop_stash(root, &stash).unwrap();
        assert_eq!(
            std::fs::read_link(root.join("link")).unwrap(),
            PathBuf::from("merged.txt")
        );
        assert!(git(root, &["stash", "list"]).trim().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn failed_apply_keeps_symlink_with_different_target() {
        let temp = repo_with_conflicting_branch();
        let root = temp.path();
        std::fs::write(root.join("merged.txt"), "user\n").unwrap();
        std::os::unix::fs::symlink("merged.txt", root.join("link")).unwrap();
        let stash = stash_dirty_worktree(root, "Target", Some("main"))
            .unwrap()
            .unwrap();
        std::fs::write(root.join("merged.txt"), "incoming\n").unwrap();
        git(root, &["commit", "-am", "incoming overlap"]);
        let before: Vec<String> = Vec::new();
        std::os::unix::fs::symlink("other.txt", root.join("link")).unwrap();
        let removed = remove_untracked_created_by_failed_apply(root, &stash, &["link".to_string()]);
        assert!(removed.is_empty());
        assert_eq!(
            std::fs::read_link(root.join("link")).unwrap(),
            PathBuf::from("other.txt")
        );
        let _ = before;
    }
}
