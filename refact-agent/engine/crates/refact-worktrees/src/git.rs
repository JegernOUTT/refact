use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use git2::{DiffOptions, Repository, Signature, StashFlags, Status, StatusOptions, StatusShow};
use uuid::Uuid;

use crate::types::{WorktreeDiffFile, WorktreeDiffStats, WorktreeStatus};

pub struct WorktreeCreateResult {
    pub branch_was_created: bool,
    pub repo_root: PathBuf,
    pub base_branch: Option<String>,
    pub base_commit: String,
    pub dirty_source: bool,
}

pub struct WorktreeDiffParts {
    pub files: Vec<WorktreeDiffFile>,
    pub stats: WorktreeDiffStats,
    pub patch: String,
    pub patch_truncated: bool,
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
    base_ref: Option<&str>,
) -> Result<WorktreeCreateResult, String> {
    let repo = discover_repo(source_root)?;
    let repo_root = repo_root(&repo)?;
    let base_branch = base_ref
        .map(|s| s.to_string())
        .or_else(|| current_branch(&repo));
    let base_commit = match base_ref {
        Some(base) => commit_for_branch(&repo, base)?,
        None => head_commit(&repo)?,
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
        Ok(repo) => {
            let mut status = WorktreeStatus {
                path_exists: true,
                is_git_worktree: true,
                dirty: false,
                conflicted: false,
                staged_count: 0,
                unstaged_count: 0,
                untracked_count: 0,
                branch: current_branch(&repo),
                head_commit: head_commit(&repo).ok(),
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
                    status.dirty = status.staged_count > 0
                        || status.unstaged_count > 0
                        || status.untracked_count > 0;
                }
                Err(e) => {
                    status.error = Some(format!("Failed to read git status: {}", e));
                }
            }
            status
        }
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

pub fn run_git(path: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .map_err(|e| format!("Failed to run git {:?}: {}", args, e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

pub fn run_git_with_refact_author(path: &Path, args: &[&str]) -> Result<String, String> {
    let mut full_args = vec![
        "-c",
        "user.name=Refact Agent",
        "-c",
        "user.email=agent@refact.ai",
    ];
    full_args.extend_from_slice(args);
    run_git(path, &full_args)
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
    let output = run_git_lossy(source_root, &["worktree", "list", "--porcelain"]);
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
    entries
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
            status: "A".to_string(),
            source: "untracked".to_string(),
            additions: count_file_lines(&path.join(line)),
            deletions: Some(0),
        })
        .collect()
}

fn push_bounded(target: &mut String, text: &str, max_bytes: usize, truncated: &mut bool) {
    if *truncated || target.len() >= max_bytes {
        *truncated = true;
        return;
    }
    let remaining = max_bytes - target.len();
    if text.len() <= remaining {
        target.push_str(text);
        return;
    }
    let mut end = remaining;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    target.push_str(&text[..end]);
    target.push_str("\n");
    *truncated = true;
}

fn append_patch_section(
    patch: &mut String,
    title: &str,
    body: &str,
    max_bytes: usize,
    truncated: &mut bool,
) {
    if body.trim().is_empty() {
        return;
    }
    push_bounded(patch, &format!("\n## {}\n", title), max_bytes, truncated);
    push_bounded(patch, body, max_bytes, truncated);
    if !body.ends_with('\n') {
        push_bounded(patch, "\n", max_bytes, truncated);
    }
}

fn append_untracked_patch(
    root: &Path,
    files: &[WorktreeDiffFile],
    patch: &mut String,
    max_bytes: usize,
    truncated: &mut bool,
) {
    const MAX_UNTRACKED_PREVIEW_BYTES: usize = 64_000;
    for file in files {
        if *truncated {
            return;
        }
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
                max_bytes,
                truncated,
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
            max_bytes,
            truncated,
        );
        if *truncated {
            return;
        }
        let remaining = max_bytes.saturating_sub(patch.len());
        let read_cap = remaining.min(MAX_UNTRACKED_PREVIEW_BYTES);
        if read_cap == 0 {
            *truncated = true;
            return;
        }
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
            push_bounded(patch, "+", max_bytes, truncated);
            push_bounded(patch, line, max_bytes, truncated);
            push_bounded(patch, "\n", max_bytes, truncated);
            if *truncated {
                return;
            }
        }
        if over_limit {
            *truncated = true;
            return;
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
    let mut patch = String::new();
    let mut patch_truncated = false;

    let committed_base = resolve_diff_base(root, base_commit, base_branch)?;
    if let Some(base) = committed_base {
        let range = format!("{}..HEAD", base);
        let committed_name_status = run_git(root, &["diff", "--name-status", &range])?;
        let committed_numstat = run_git(root, &["diff", "--numstat", &range])?;
        let committed = parse_name_status(&committed_name_status, &committed_numstat, "committed");
        stats.committed_files = committed.len();
        files.extend(committed);
        let committed_patch = run_git(root, &["diff", "--no-ext-diff", &range])?;
        append_patch_section(
            &mut patch,
            "committed",
            &committed_patch,
            max_patch_bytes,
            &mut patch_truncated,
        );
    }

    let staged_name_status = run_git_lossy(root, &["diff", "--cached", "--name-status"]);
    let staged_numstat = run_git_lossy(root, &["diff", "--cached", "--numstat"]);
    let staged = parse_name_status(&staged_name_status, &staged_numstat, "staged");
    stats.staged_files = staged.len();
    files.extend(staged);
    let staged_patch = run_git_lossy(root, &["diff", "--no-ext-diff", "--cached"]);
    append_patch_section(
        &mut patch,
        "staged",
        &staged_patch,
        max_patch_bytes,
        &mut patch_truncated,
    );

    let unstaged_name_status = run_git_lossy(root, &["diff", "--name-status"]);
    let unstaged_numstat = run_git_lossy(root, &["diff", "--numstat"]);
    let unstaged = parse_name_status(&unstaged_name_status, &unstaged_numstat, "unstaged");
    stats.unstaged_files = unstaged.len();
    files.extend(unstaged);
    let unstaged_patch = run_git_lossy(root, &["diff", "--no-ext-diff"]);
    append_patch_section(
        &mut patch,
        "unstaged",
        &unstaged_patch,
        max_patch_bytes,
        &mut patch_truncated,
    );

    let untracked = list_untracked(root);
    stats.untracked_files = untracked.len();
    append_untracked_patch(
        root,
        &untracked,
        &mut patch,
        max_patch_bytes,
        &mut patch_truncated,
    );
    files.extend(untracked);
    stats.files_changed = files.len();
    stats.additions = files.iter().filter_map(|file| file.additions).sum();
    stats.deletions = files.iter().filter_map(|file| file.deletions).sum();

    if patch_truncated {
        push_bounded(
            &mut patch,
            "\n[patch truncated]\n",
            max_patch_bytes.saturating_add(128),
            &mut false,
        );
    }

    Ok(WorktreeDiffParts {
        files,
        stats,
        patch,
        patch_truncated,
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
    match current_branch(&repo).as_deref() {
        Some(current_branch) if current_branch == expected_branch => Ok(()),
        Some(current_branch) => Err(format!(
            "{} is checked out on a different branch '{}'; expected branch '{}'",
            label, current_branch, expected_branch
        )),
        None => Err(format!(
            "{} is checked out on detached HEAD {}; expected branch '{}'",
            label,
            head_rev(root)?,
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
}

pub fn stash_dirty_worktree(
    root: &Path,
    label: &str,
    expected_branch: Option<&str>,
) -> Result<Option<DirtyWorktreeStash>, String> {
    ensure_no_merge_in_progress(root, label)?;
    let status = run_git(root, &["status", "--porcelain"])?;
    if status.trim().is_empty() {
        return Ok(None);
    }

    let mut repo = discover_repo(root)?;
    let original_branch = current_branch(&repo);
    let original_head = head_rev(root)?;
    if let Some(expected_branch) = expected_branch {
        ensure_checkout_on_branch(root, label, expected_branch)?;
    }
    let stash_marker = format!("Refact worktree merge target workspace {}", Uuid::new_v4());
    let stasher = Signature::now("Refact Agent", "agent@refact.ai")
        .map_err(|e| format!("Failed to create stash author: {}", e))?;
    let stash_oid = repo
        .stash_save(&stasher, &stash_marker, Some(StashFlags::INCLUDE_UNTRACKED))
        .map_err(|e| format!("Failed to stash target workspace changes: {}", e))?
        .to_string();
    let stash = DirtyWorktreeStash {
        original_branch,
        original_head,
        stash_marker,
        stash_oid,
    };
    if let Err(e) = ensure_clean_worktree(root, label) {
        let restore_result = restore_stashed_changes(root, &stash);
        let restore_details = if restore_result.critical_ok && restore_result.warnings.is_empty() {
            "target workspace changes were restored".to_string()
        } else if restore_result.critical_ok {
            format!(
                "target workspace changes were restored with warnings: {}",
                restore_result.warnings.join("; ")
            )
        } else {
            format!(
                "failed to restore target workspace changes; they remain saved in git stash {}: {}",
                stash.stash_oid,
                restore_result.warnings.join("; ")
            )
        };
        return Err(format!("{}; {}", e, restore_details));
    }

    Ok(Some(stash))
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
    run_git(root, &["stash", "apply", "--index", &stash.stash_oid]).map_err(|e| {
        format!(
            "Failed to reapply target workspace changes after merge: {}; changes remain saved in git stash {}",
            e, stash.stash_oid
        )
    })?;

    let mut warnings = Vec::new();
    match stash_entry_for_marker(root, &stash.stash_marker) {
        Ok(Some(stash_ref)) => {
            if let Err(e) = run_git(root, &["stash", "drop", &stash_ref.0]) {
                warnings.push(format!(
                    "Target workspace changes were reapplied, but dropping git stash {} failed: {}",
                    stash_ref.0, e
                ));
            }
        }
        Ok(None) => warnings.push(format!(
            "Target workspace changes were reapplied, but git stash {} was not found for cleanup",
            stash.stash_oid
        )),
        Err(e) => warnings.push(format!(
            "Target workspace changes were reapplied, but locating git stash {} for cleanup failed: {}",
            stash.stash_oid, e
        )),
    }
    Ok(warnings)
}

pub fn reset_hard(root: &Path, rev: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Err(e) = run_git(root, &["reset", "--hard", rev]) {
        warnings.push(format!(
            "Failed to reset target workspace to {}: {}",
            rev, e
        ));
    }
    warnings
}

pub fn restore_stashed_changes(
    root: &Path,
    stash: &DirtyWorktreeStash,
) -> RestoreStashedChangesResult {
    let mut warnings = Vec::new();
    let mut critical_ok = true;
    let reset_warnings = reset_hard(root, "HEAD");
    if !reset_warnings.is_empty() {
        critical_ok = false;
    }
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
        };
    }
    let reset_warnings = reset_hard(root, &stash.original_head);
    if !reset_warnings.is_empty() {
        critical_ok = false;
    }
    warnings.extend(reset_warnings);
    match apply_and_drop_stash(root, stash) {
        Ok(drop_warnings) => warnings.extend(drop_warnings),
        Err(e) => {
            critical_ok = false;
            warnings.push(format!(
                "Failed to restore target workspace changes after rolling back merge: {}",
                e
            ));
        }
    }
    RestoreStashedChangesResult {
        critical_ok,
        warnings,
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
        warnings.extend(reset_hard(root, "HEAD"));
    }
    warnings
}

pub fn preflight_merge_conflicts(
    source_root: &Path,
    target_branch: &str,
    source_branch: &str,
    strategy: &str,
) -> Result<PreflightMergeResult, String> {
    let temp = tempfile::Builder::new()
        .prefix("refact-merge-preflight-")
        .tempdir()
        .map_err(|e| format!("Failed to create merge preflight directory: {}", e))?;
    let preflight_path = temp.path().join("worktree");
    let preflight_str = preflight_path
        .to_str()
        .ok_or_else(|| "Merge preflight path is not valid UTF-8".to_string())?;
    run_git(
        source_root,
        &["worktree", "add", "--detach", preflight_str, target_branch],
    )?;
    let merge_result = if strategy == "squash" {
        run_git(&preflight_path, &["merge", "--squash", source_branch])
    } else {
        run_git(
            &preflight_path,
            &["merge", "--no-commit", "--no-ff", source_branch],
        )
    };
    let conflicts = conflict_files_for_path(&preflight_path);
    let mut warnings = Vec::new();
    if let Err(e) = run_git(
        source_root,
        &["worktree", "remove", "--force", preflight_str],
    ) {
        warnings.push(format!("Failed to remove merge preflight worktree: {}", e));
    }
    if preflight_path.exists() {
        if let Err(e) = std::fs::remove_dir_all(&preflight_path) {
            warnings.push(format!(
                "Failed to remove merge preflight directory '{}': {}",
                preflight_path.display(),
                e
            ));
        }
    }
    match merge_result {
        Ok(_) => Ok(PreflightMergeResult {
            conflicts: Vec::new(),
            warnings,
        }),
        Err(_e) if !conflicts.is_empty() => Ok(PreflightMergeResult {
            conflicts,
            warnings,
        }),
        Err(e) => {
            if warnings.is_empty() {
                Err(format!("Merge preflight failed: {}", e))
            } else {
                Err(format!(
                    "Merge preflight failed: {}; cleanup warnings: {}",
                    e,
                    warnings.join("; ")
                ))
            }
        }
    }
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
