use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use chrono::{DateTime, TimeZone, Utc};
use git2::{Branch, DiffOptions, Oid, Repository};
use tracing::error;
use url::Url;
use refact_core::custom_error::MapErrToString;
use refact_core::string_utils::{redact_sensitive, safe_truncate};
use crate::{FileChange, FileChangeStatus};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_STAGE_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_GIT_HISTORY_COMMITS: usize = 500;
pub const MAX_GIT_HISTORY_FILES_PER_COMMIT: usize = 200;
pub const MAX_GIT_HISTORY_COCHANGE_PAIRS: usize = 2_000;
pub const MAX_GIT_HISTORY_HOTSPOTS: usize = 100;
const MAX_GIT_HISTORY_COCHANGE_FILES_PER_COMMIT: usize = 40;
const MAX_GIT_HISTORY_COCHANGE_COMMITS: usize = 10;
const MAX_GIT_HISTORY_MESSAGE_CHARS: usize = 320;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitBranchInfo {
    pub name: String,
    pub is_head: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHistoryOptions {
    pub max_commits: usize,
    pub since: Option<DateTime<Utc>>,
    pub max_files_per_commit: usize,
    pub cochange_threshold: usize,
    pub max_cochange_pairs: usize,
    pub max_hotspots: usize,
}

impl Default for GitHistoryOptions {
    fn default() -> Self {
        Self {
            max_commits: 200,
            since: None,
            max_files_per_commit: MAX_GIT_HISTORY_FILES_PER_COMMIT,
            cochange_threshold: 3,
            max_cochange_pairs: MAX_GIT_HISTORY_COCHANGE_PAIRS,
            max_hotspots: MAX_GIT_HISTORY_HOTSPOTS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GitCommitClassification {
    Bugfix,
    Revert,
    Migration,
    Decision,
    Rationale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GitFileChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    TypeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommitFileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: GitFileChangeStatus,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommitSummary {
    pub oid: String,
    pub short_oid: String,
    pub time: DateTime<Utc>,
    pub parent_oids: Vec<String>,
    pub message: String,
    pub classifications: Vec<GitCommitClassification>,
    pub changes: Vec<GitCommitFileChange>,
    pub file_cap_hit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCoChangePair {
    pub path_a: String,
    pub path_b: String,
    pub count: usize,
    pub commits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHotspot {
    pub path: String,
    pub edit_count: usize,
    pub additions: usize,
    pub deletions: usize,
    pub score: u64,
    pub latest_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHistoryReport {
    pub commits: Vec<GitCommitSummary>,
    pub cochanges: Vec<GitCoChangePair>,
    pub hotspots: Vec<GitHotspot>,
    pub commit_cap_hit: bool,
}

fn canonical_path(path: PathBuf) -> PathBuf {
    dunce::simplified(&path.canonicalize().unwrap_or_else(|_| path.clone())).to_path_buf()
}

fn status_options(include_unmodified: bool, show: git2::StatusShow) -> git2::StatusOptions {
    let mut options = git2::StatusOptions::new();
    options
        .disable_pathspec_match(true)
        .include_ignored(false)
        .include_unmodified(include_unmodified)
        .include_unreadable(false)
        .include_untracked(true)
        .recurse_ignored_dirs(false)
        .recurse_untracked_dirs(true)
        .rename_threshold(100)
        .update_index(true)
        .show(show);
    options
}

#[allow(dead_code)]
pub fn get_git_remotes(repository_path: &Path) -> Result<Vec<(String, String)>, String> {
    let repository = Repository::discover(repository_path)
        .map_err(|e| format!("Failed to open repository: {}", e))?;
    let remotes = repository
        .remotes()
        .map_err(|e| format!("Failed to get remotes: {}", e))?;
    let mut result = Vec::new();
    for name in remotes.iter().flatten() {
        if let Ok(remote) = repository.find_remote(name) {
            if let Some(url) = remote.url() {
                if let Ok(mut parsed_url) = Url::parse(url) {
                    parsed_url.set_username("").ok();
                    parsed_url.set_password(None).ok();
                    result.push((name.to_string(), parsed_url.to_string()));
                } else {
                    result.push((name.to_string(), url.to_string()));
                }
            }
        }
    }
    Ok(result)
}

pub fn git_ls_files(repository_path: &PathBuf) -> Option<Vec<PathBuf>> {
    let repository = Repository::open(repository_path)
        .map_err(|e| error!("Failed to open repository: {}", e))
        .ok()?;

    let statuses = repository
        .statuses(Some(&mut status_options(
            true,
            git2::StatusShow::IndexAndWorkdir,
        )))
        .map_err(|e| error!("Failed to get statuses: {}", e))
        .ok()?;

    let mut files = Vec::new();
    for entry in statuses.iter() {
        let path = String::from_utf8_lossy(entry.path_bytes()).to_string();
        files.push(repository_path.join(path));
    }
    if !files.is_empty() {
        Some(files)
    } else {
        None
    }
}

pub fn get_or_create_branch<'repo>(
    repository: &'repo Repository,
    branch_name: &str,
) -> Result<Branch<'repo>, String> {
    match repository.find_branch(branch_name, git2::BranchType::Local) {
        Ok(branch) => Ok(branch),
        Err(_) => {
            let head_commit = repository
                .head()
                .and_then(|h| h.peel_to_commit())
                .map_err_with_prefix("Failed to get HEAD commit:")?;
            repository
                .branch(branch_name, &head_commit, false)
                .map_err_with_prefix("Failed to create branch:")
        }
    }
}

pub fn list_branches(repository: &Repository) -> Result<Vec<GitBranchInfo>, String> {
    let branches = repository
        .branches(Some(git2::BranchType::Local))
        .map_err_with_prefix("Failed to list branches:")?;
    let mut result = Vec::new();
    for branch_result in branches {
        let (branch, _) = branch_result.map_err_with_prefix("Failed to read branch:")?;
        let Some(name) = branch
            .name()
            .map_err_with_prefix("Failed to read branch name:")?
        else {
            continue;
        };
        let upstream = branch
            .upstream()
            .ok()
            .and_then(|upstream| upstream.name().ok().flatten().map(ToString::to_string));
        result.push(GitBranchInfo {
            name: name.to_string(),
            is_head: branch.is_head(),
            upstream,
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

fn is_changed_in_wt(status: git2::Status) -> bool {
    status.intersects(
        git2::Status::WT_NEW
            | git2::Status::WT_MODIFIED
            | git2::Status::WT_DELETED
            | git2::Status::WT_RENAMED
            | git2::Status::WT_TYPECHANGE,
    )
}

fn is_changed_in_index(status: git2::Status) -> bool {
    status.intersects(
        git2::Status::INDEX_NEW
            | git2::Status::INDEX_MODIFIED
            | git2::Status::INDEX_DELETED
            | git2::Status::INDEX_RENAMED
            | git2::Status::INDEX_TYPECHANGE,
    )
}

pub fn get_diff_statuses(
    show_opt: git2::StatusShow,
    repo: &Repository,
    include_abs_paths: bool,
) -> Result<(Vec<FileChange>, Vec<FileChange>), String> {
    let repo_workdir = repo
        .workdir()
        .ok_or("Failed to get workdir from repository".to_string())?;

    let mut staged_changes = Vec::new();
    let mut unstaged_changes = Vec::new();
    let statuses = repo
        .statuses(Some(&mut status_options(false, show_opt)))
        .map_err_with_prefix("Failed to get statuses:")?;

    for entry in statuses.iter() {
        let status = entry.status();
        let relative_path = PathBuf::from(String::from_utf8_lossy(entry.path_bytes()).to_string());

        if entry.path_bytes().last() == Some(&b'/')
            && repo_workdir.join(&relative_path).join(".git").exists()
        {
            continue;
        }

        let should_not_be_present = match show_opt {
            git2::StatusShow::Index => is_changed_in_wt(status) || status.is_index_renamed(),
            git2::StatusShow::Workdir => is_changed_in_index(status) || status.is_wt_renamed(),
            git2::StatusShow::IndexAndWorkdir => {
                status.is_index_renamed() || status.is_wt_renamed()
            }
        };
        if should_not_be_present {
            tracing::error!("File status is {:?} for file {:?}, which should not be present due to status options.", status, relative_path);
            continue;
        }

        let absolute_path =
            if include_abs_paths && (is_changed_in_index(status) || is_changed_in_wt(status)) {
                canonical_path(repo_workdir.join(&relative_path))
            } else {
                PathBuf::new()
            };

        if is_changed_in_index(status) {
            staged_changes.push(FileChange {
                status: match status {
                    s if s.is_index_new() => FileChangeStatus::ADDED,
                    s if s.is_index_deleted() => FileChangeStatus::DELETED,
                    _ => FileChangeStatus::MODIFIED,
                },
                absolute_path: absolute_path.clone(),
                relative_path: relative_path.clone(),
            });
        }

        if is_changed_in_wt(status) {
            unstaged_changes.push(FileChange {
                status: match status {
                    s if s.is_wt_new() => FileChangeStatus::ADDED,
                    s if s.is_wt_deleted() => FileChangeStatus::DELETED,
                    _ => FileChangeStatus::MODIFIED,
                },
                absolute_path,
                relative_path,
            });
        }
    }

    Ok((staged_changes, unstaged_changes))
}

pub fn get_diff_statuses_index_to_commit(
    repository: &Repository,
    commit_oid: &git2::Oid,
    include_abs_paths: bool,
) -> Result<Vec<FileChange>, String> {
    let head = repository
        .head()
        .map_err_with_prefix("Failed to get HEAD:")?;
    let original_head_ref = head
        .is_branch()
        .then(|| head.name().map(ToString::to_string))
        .flatten();
    let original_head_oid = head.target();

    repository
        .set_head_detached(commit_oid.clone())
        .map_err_with_prefix("Failed to set HEAD:")?;

    let result = get_diff_statuses(git2::StatusShow::Index, repository, include_abs_paths);

    let restore_result = match (&original_head_ref, original_head_oid) {
        (Some(head_ref), _) => repository.set_head(head_ref),
        (None, Some(oid)) => repository.set_head_detached(oid),
        (None, None) => Ok(()),
    };

    if let Err(restore_err) = restore_result {
        let prev_err = result.as_ref().err().cloned().unwrap_or_default();
        return Err(format!(
            "{}\nFailed to restore head: {}",
            prev_err, restore_err
        ));
    }

    result.map(|(staged_changes, _unstaged_changes)| staged_changes)
}

pub fn stage_changes(
    repository: &Repository,
    file_changes: &[FileChange],
    abort_flag: &Arc<AtomicBool>,
) -> Result<usize, String> {
    let workdir = repository.workdir().map(|p| p.to_path_buf());
    let mut index = repository
        .index()
        .map_err_with_prefix("Failed to get index:")?;
    let mut skipped = 0usize;

    for file_change in file_changes {
        if abort_flag.load(Ordering::SeqCst) {
            return Err("stage_changes aborted".to_string());
        }
        match file_change.status {
            FileChangeStatus::ADDED | FileChangeStatus::MODIFIED => {
                if let Some(ref wd) = workdir {
                    if let Ok(meta) = std::fs::metadata(wd.join(&file_change.relative_path)) {
                        if meta.len() > MAX_STAGE_FILE_SIZE_BYTES {
                            tracing::warn!(
                                "shadow git: skipping large file {} ({:.1} MB)",
                                file_change.relative_path.display(),
                                meta.len() as f64 / 1_048_576.0
                            );
                            skipped += 1;
                            continue;
                        }
                    }
                }
                index
                    .add_path(&file_change.relative_path)
                    .map_err_with_prefix("Failed to add file to index:")?;
            }
            FileChangeStatus::DELETED => {
                index
                    .remove_path(&file_change.relative_path)
                    .map_err_with_prefix("Failed to remove file from index:")?;
            }
        }
    }

    index
        .write()
        .map_err_with_prefix("Failed to write index:")?;
    Ok(skipped)
}

pub fn unstage_changes(repository: &Repository, paths: &[PathBuf]) -> Result<usize, String> {
    if paths.is_empty() {
        return Ok(0);
    }

    match repository
        .head()
        .and_then(|head| head.peel(git2::ObjectType::Commit))
    {
        Ok(head_object) => repository
            .reset_default(Some(&head_object), paths.iter())
            .map_err_with_prefix("Failed to unstage changes:")?,
        Err(error)
            if matches!(
                error.code(),
                git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
            ) =>
        {
            let mut index = repository
                .index()
                .map_err_with_prefix("Failed to get index:")?;
            for path in paths {
                match index.remove_path(path) {
                    Ok(()) => {}
                    Err(error) if error.code() == git2::ErrorCode::NotFound => {}
                    Err(error) => return Err(format!("Failed to unstage changes: {}", error)),
                }
            }
            index
                .write()
                .map_err_with_prefix("Failed to write index:")?;
        }
        Err(error) => return Err(format!("Failed to resolve HEAD: {}", error)),
    }

    Ok(paths.len())
}

pub fn get_configured_author_email_and_name(
    repository: &Repository,
) -> Result<(String, String), String> {
    let config = repository
        .config()
        .map_err_with_prefix("Failed to get repository config:")?;
    let author_email = config
        .get_string("user.email")
        .map_err_with_prefix("Failed to get author email:")?;
    let author_name = config
        .get_string("user.name")
        .map_err_with_prefix("Failed to get author name:")?;
    Ok((author_email, author_name))
}

pub fn commit(
    repository: &Repository,
    branch: &Branch,
    message: &str,
    author_name: &str,
    author_email: &str,
) -> Result<Oid, String> {
    let mut index = repository
        .index()
        .map_err_with_prefix("Failed to get index:")?;
    let tree_id = index
        .write_tree()
        .map_err_with_prefix("Failed to write tree:")?;
    let tree = repository
        .find_tree(tree_id)
        .map_err_with_prefix("Failed to find tree:")?;

    let signature = git2::Signature::now(author_name, author_email)
        .map_err_with_prefix("Failed to create signature:")?;
    let branch_ref_name = branch
        .get()
        .name()
        .ok_or("Invalid branch name".to_string())?;

    let parent_commit = if let Some(target) = branch.get().target() {
        repository
            .find_commit(target)
            .map_err(|e| format!("Failed to find branch commit: {}", e))?
    } else {
        return Err("No parent commits found".to_string());
    };

    let commit = repository
        .commit(
            Some(branch_ref_name),
            &signature,
            &signature,
            message,
            &tree,
            &[&parent_commit],
        )
        .map_err(|e| format!("Failed to create commit: {}", e))?;

    repository
        .set_head(branch_ref_name)
        .map_err_with_prefix("Failed to set branch as head:")?;

    Ok(commit)
}

pub fn open_or_init_repo(path: &Path) -> Result<Repository, String> {
    let repo = match Repository::open(path) {
        Ok(repo) => Ok(repo),
        Err(e) if e.code() == git2::ErrorCode::NotFound => {
            Repository::init(path).map_err_to_string()
        }
        Err(e) => Err(e.to_string()),
    }?;
    if let Ok(mut config) = repo.config() {
        if let Err(err) = config.set_bool("core.autocrlf", false) {
            tracing::warn!(
                "Failed to disable autocrlf for shadow git repo '{}': {}",
                path.display(),
                err
            );
        }
    }
    Ok(repo)
}

pub fn mine_git_history(
    repository_path: &Path,
    options: GitHistoryOptions,
) -> Result<GitHistoryReport, String> {
    let repository = Repository::discover(repository_path)
        .map_err(|e| format!("Failed to open repository: {}", e))?;
    mine_git_history_from_repo(&repository, options)
}

pub fn mine_git_history_from_repo(
    repository: &Repository,
    options: GitHistoryOptions,
) -> Result<GitHistoryReport, String> {
    let max_commits = options.max_commits.min(MAX_GIT_HISTORY_COMMITS);
    if max_commits == 0 {
        return Ok(GitHistoryReport {
            commits: Vec::new(),
            cochanges: Vec::new(),
            hotspots: Vec::new(),
            commit_cap_hit: false,
        });
    }

    let head_oid = repository
        .head()
        .and_then(|head| head.peel_to_commit())
        .map(|commit| commit.id())
        .map_err_with_prefix("Failed to get HEAD commit:")?;
    let commits = recent_git_commits_from(repository, head_oid, &options)?;
    let commit_cap_hit = commits.len() == max_commits;
    let cochanges = compute_git_cochanges(&commits, &options);
    let hotspots = compute_git_hotspots(&commits, &options);
    Ok(GitHistoryReport {
        commits,
        cochanges,
        hotspots,
        commit_cap_hit,
    })
}

#[allow(dead_code)]
pub fn recent_git_commits(
    repository_path: &Path,
    max_commits: usize,
    since: Option<DateTime<Utc>>,
) -> Result<Vec<GitCommitSummary>, String> {
    let repository = Repository::discover(repository_path)
        .map_err(|e| format!("Failed to open repository: {}", e))?;
    let head_oid = repository
        .head()
        .and_then(|head| head.peel_to_commit())
        .map(|commit| commit.id())
        .map_err_with_prefix("Failed to get HEAD commit:")?;
    recent_git_commits_from(
        &repository,
        head_oid,
        &GitHistoryOptions {
            max_commits,
            since,
            ..GitHistoryOptions::default()
        },
    )
}

fn recent_git_commits_from(
    repository: &Repository,
    head_oid: Oid,
    options: &GitHistoryOptions,
) -> Result<Vec<GitCommitSummary>, String> {
    let max_commits = options.max_commits.min(MAX_GIT_HISTORY_COMMITS);
    let mut revwalk = repository
        .revwalk()
        .map_err_with_prefix("Failed to create revwalk:")?;
    revwalk
        .set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
        .map_err_with_prefix("Failed to configure revwalk:")?;
    revwalk
        .push(head_oid)
        .map_err_with_prefix("Failed to walk HEAD:")?;

    let mut commits = Vec::new();
    for oid_result in revwalk {
        if commits.len() >= max_commits {
            break;
        }
        let oid = match oid_result {
            Ok(oid) => oid,
            Err(err) => {
                tracing::debug!("git history: skipping unreadable object: {}", err);
                continue;
            }
        };
        let commit = match repository.find_commit(oid) {
            Ok(commit) => commit,
            Err(err) => {
                tracing::debug!("git history: skipping missing commit {}: {}", oid, err);
                continue;
            }
        };
        let Some(time) = git_time_to_utc(commit.time().seconds()) else {
            continue;
        };
        if options.since.map(|since| time < since).unwrap_or(false) {
            break;
        }
        commits.push(commit_summary(repository, &commit, time, options)?);
    }
    Ok(commits)
}

fn commit_summary(
    repository: &Repository,
    commit: &git2::Commit,
    time: DateTime<Utc>,
    options: &GitHistoryOptions,
) -> Result<GitCommitSummary, String> {
    let parent_oids = commit
        .parent_ids()
        .map(|oid| oid.to_string())
        .collect::<Vec<_>>();
    let raw_message = commit.message().unwrap_or_default();
    let message = cap_git_text(raw_message, MAX_GIT_HISTORY_MESSAGE_CHARS);
    let classifications = classify_commit_message(&message);
    let (changes, file_cap_hit) = changed_files_for_commit(repository, commit, options)?;
    let oid = commit.id().to_string();
    Ok(GitCommitSummary {
        short_oid: short_oid(&oid),
        oid,
        time,
        parent_oids,
        message,
        classifications,
        changes,
        file_cap_hit,
    })
}

#[allow(dead_code)]
pub fn changed_files_for_commit_oid(
    repository: &Repository,
    oid: Oid,
    max_files: usize,
) -> Result<Vec<GitCommitFileChange>, String> {
    let commit = repository
        .find_commit(oid)
        .map_err_with_prefix("Failed to find commit:")?;
    let (changes, _) = changed_files_for_commit(
        repository,
        &commit,
        &GitHistoryOptions {
            max_files_per_commit: max_files,
            ..GitHistoryOptions::default()
        },
    )?;
    Ok(changes)
}

fn changed_files_for_commit(
    repository: &Repository,
    commit: &git2::Commit,
    options: &GitHistoryOptions,
) -> Result<(Vec<GitCommitFileChange>, bool), String> {
    let new_tree = commit
        .tree()
        .map_err_with_prefix("Failed to read commit tree:")?;
    let old_tree = if commit.parent_count() == 0 {
        None
    } else {
        commit.parent(0).ok().and_then(|parent| parent.tree().ok())
    };

    let mut diff_options = DiffOptions::new();
    diff_options
        .include_typechange(true)
        .skip_binary_check(false)
        .max_size(1_000_000);
    let mut diff = repository
        .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut diff_options))
        .map_err_with_prefix("Failed to diff commit:")?;
    let mut find_options = git2::DiffFindOptions::new();
    find_options
        .renames(true)
        .rename_threshold(50)
        .rename_limit(200);
    if let Err(err) = diff.find_similar(Some(&mut find_options)) {
        tracing::debug!("git history: rename detection skipped: {}", err);
    }

    let max_files = options
        .max_files_per_commit
        .min(MAX_GIT_HISTORY_FILES_PER_COMMIT);
    let line_counts = diff_line_counts(&diff);
    let mut changes = diff
        .deltas()
        .filter_map(|delta| diff_delta_to_change(delta, &line_counts))
        .collect::<Vec<_>>();
    changes.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.old_path.cmp(&b.old_path))
            .then_with(|| a.status.cmp(&b.status))
    });
    let file_cap_hit = changes.len() > max_files;
    changes.truncate(max_files);
    Ok((changes, file_cap_hit))
}

fn diff_line_counts(diff: &git2::Diff) -> BTreeMap<String, (usize, usize)> {
    let mut counts = BTreeMap::<String, (usize, usize)>::new();
    let mut line_cb = |delta: git2::DiffDelta<'_>,
                       _hunk: Option<git2::DiffHunk<'_>>,
                       line: git2::DiffLine<'_>| {
        let Some(path) = diff_delta_path_for_counts(delta) else {
            return true;
        };
        let entry = counts.entry(path).or_insert((0, 0));
        match line.origin() {
            '+' => entry.0 = entry.0.saturating_add(1),
            '-' => entry.1 = entry.1.saturating_add(1),
            _ => {}
        }
        true
    };
    if diff
        .foreach(&mut |_, _| true, None, None, Some(&mut line_cb))
        .is_err()
    {
        return BTreeMap::new();
    }
    counts
}

fn diff_delta_path_for_counts(delta: git2::DiffDelta<'_>) -> Option<String> {
    match delta.status() {
        git2::Delta::Deleted => diff_path(delta.old_file().path()),
        _ => diff_path(delta.new_file().path()).or_else(|| diff_path(delta.old_file().path())),
    }
}

fn diff_delta_to_change(
    delta: git2::DiffDelta<'_>,
    line_counts: &BTreeMap<String, (usize, usize)>,
) -> Option<GitCommitFileChange> {
    let status = match delta.status() {
        git2::Delta::Added | git2::Delta::Copied => GitFileChangeStatus::Added,
        git2::Delta::Deleted => GitFileChangeStatus::Deleted,
        git2::Delta::Modified => GitFileChangeStatus::Modified,
        git2::Delta::Renamed => GitFileChangeStatus::Renamed,
        git2::Delta::Typechange => GitFileChangeStatus::TypeChanged,
        _ => return None,
    };
    let path = match status {
        GitFileChangeStatus::Deleted => diff_path(delta.old_file().path())?,
        _ => diff_path(delta.new_file().path()).or_else(|| diff_path(delta.old_file().path()))?,
    };
    let old_path = if status == GitFileChangeStatus::Renamed {
        diff_path(delta.old_file().path())
    } else {
        None
    };
    let (additions, deletions) = line_counts
        .get(&path)
        .copied()
        .filter(|(additions, deletions)| *additions > 0 || *deletions > 0)
        .unwrap_or_else(|| delta_line_counts(&delta));
    Some(GitCommitFileChange {
        path,
        old_path,
        status,
        additions,
        deletions,
        binary: delta.old_file().is_binary() || delta.new_file().is_binary(),
    })
}

fn delta_line_counts(delta: &git2::DiffDelta<'_>) -> (usize, usize) {
    match delta.status() {
        git2::Delta::Added | git2::Delta::Copied => (approx_file_lines(delta.new_file().size()), 0),
        git2::Delta::Deleted => (0, approx_file_lines(delta.old_file().size())),
        git2::Delta::Modified | git2::Delta::Renamed | git2::Delta::Typechange => {
            let old_size = delta.old_file().size();
            let new_size = delta.new_file().size();
            if new_size >= old_size {
                (approx_file_lines(new_size - old_size), 0)
            } else {
                (0, approx_file_lines(old_size - new_size))
            }
        }
        _ => (0, 0),
    }
}

fn approx_file_lines(bytes: u64) -> usize {
    if bytes == 0 {
        0
    } else {
        (bytes as usize / 80).saturating_add(1)
    }
}

fn diff_path(path: Option<&Path>) -> Option<String> {
    normalize_history_path(&path?.to_string_lossy())
}

pub fn classify_commit_message(message: &str) -> Vec<GitCommitClassification> {
    let lower = message.to_lowercase();
    let mut classes = BTreeSet::new();
    if contains_keyword(
        &lower,
        &["fix", "fixed", "bug", "bugfix", "hotfix", "regression"],
    ) {
        classes.insert(GitCommitClassification::Bugfix);
    }
    if contains_keyword(&lower, &["revert", "rollback", "back out", "backout"]) {
        classes.insert(GitCommitClassification::Revert);
    }
    if contains_keyword(&lower, &["migrate", "migration", "schema", "upgrade"]) {
        classes.insert(GitCommitClassification::Migration);
    }
    if contains_keyword(&lower, &["decision", "decide", "decided", "adr"]) {
        classes.insert(GitCommitClassification::Decision);
    }
    if contains_keyword(&lower, &["because", "why", "rationale", "reason"])
        || lower.contains(" so that ")
    {
        classes.insert(GitCommitClassification::Rationale);
    }
    classes.into_iter().collect()
}

fn contains_keyword(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| {
        text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .any(|word| word == *needle)
            || text.contains(needle)
    })
}

fn compute_git_cochanges(
    commits: &[GitCommitSummary],
    options: &GitHistoryOptions,
) -> Vec<GitCoChangePair> {
    if options.cochange_threshold == 0 || options.max_cochange_pairs == 0 {
        return Vec::new();
    }
    let mut counts: BTreeMap<(String, String), (usize, Vec<String>)> = BTreeMap::new();
    for commit in commits {
        let mut files = commit
            .changes
            .iter()
            .filter(|change| !change.binary)
            .map(|change| change.path.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        files.truncate(MAX_GIT_HISTORY_COCHANGE_FILES_PER_COMMIT);
        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                if counts.len() >= MAX_GIT_HISTORY_COCHANGE_PAIRS
                    && !counts.contains_key(&(files[i].clone(), files[j].clone()))
                {
                    continue;
                }
                let entry = counts
                    .entry((files[i].clone(), files[j].clone()))
                    .or_insert_with(|| (0, Vec::new()));
                entry.0 = entry.0.saturating_add(1);
                if entry.1.len() < MAX_GIT_HISTORY_COCHANGE_COMMITS {
                    entry.1.push(commit.short_oid.clone());
                }
            }
        }
    }
    let mut pairs = counts
        .into_iter()
        .filter(|(_, (count, _))| *count >= options.cochange_threshold)
        .map(|((path_a, path_b), (count, commits))| GitCoChangePair {
            path_a,
            path_b,
            count,
            commits,
        })
        .collect::<Vec<_>>();
    pairs.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.path_a.cmp(&b.path_a))
            .then_with(|| a.path_b.cmp(&b.path_b))
    });
    pairs.truncate(
        options
            .max_cochange_pairs
            .min(MAX_GIT_HISTORY_COCHANGE_PAIRS),
    );
    pairs
}

fn compute_git_hotspots(
    commits: &[GitCommitSummary],
    options: &GitHistoryOptions,
) -> Vec<GitHotspot> {
    if options.max_hotspots == 0 {
        return Vec::new();
    }
    let mut stats: BTreeMap<String, GitHotspot> = BTreeMap::new();
    for commit in commits {
        for change in &commit.changes {
            if change.binary {
                continue;
            }
            let entry = stats
                .entry(change.path.clone())
                .or_insert_with(|| GitHotspot {
                    path: change.path.clone(),
                    edit_count: 0,
                    additions: 0,
                    deletions: 0,
                    score: 0,
                    latest_commit: commit.short_oid.clone(),
                });
            entry.edit_count = entry.edit_count.saturating_add(1);
            entry.additions = entry.additions.saturating_add(change.additions);
            entry.deletions = entry.deletions.saturating_add(change.deletions);
        }
    }
    let mut hotspots = stats
        .into_values()
        .map(|mut hotspot| {
            let churn = hotspot.additions.saturating_add(hotspot.deletions) as u64;
            hotspot.score = churn.saturating_add((hotspot.edit_count as u64).saturating_mul(25));
            hotspot
        })
        .collect::<Vec<_>>();
    hotspots.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.edit_count.cmp(&a.edit_count))
            .then_with(|| a.path.cmp(&b.path))
    });
    hotspots.truncate(options.max_hotspots.min(MAX_GIT_HISTORY_HOTSPOTS));
    hotspots
}

fn normalize_history_path(path: &str) -> Option<String> {
    let path = path.trim().replace('\\', "/");
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    if parts.is_empty() || parts.iter().any(|part| *part == "..") {
        return None;
    }
    Some(parts.join("/"))
}

fn cap_git_text(text: &str, max_chars: usize) -> String {
    let redacted = redact_sensitive(text);
    let single = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    safe_truncate(&single, max_chars).trim().to_string()
}

fn git_time_to_utc(seconds: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(seconds, 0).single()
}

fn short_oid(oid: &str) -> String {
    oid.chars().take(12).collect()
}

pub fn get_commit_datetime(
    repository: &Repository,
    commit_oid: &Oid,
) -> Result<DateTime<Utc>, String> {
    let commit = repository
        .find_commit(commit_oid.clone())
        .map_err_to_string()?;

    Utc.timestamp_opt(commit.time().seconds(), 0)
        .single()
        .ok_or_else(|| "Failed to get commit datetime".to_string())
}

pub fn git_diff_head_to_workdir<'repo>(
    repository: &'repo Repository,
) -> Result<git2::Diff<'repo>, String> {
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

    Ok(diff)
}

pub fn git_diff_head_to_workdir_as_string(
    repository: &Repository,
    max_size: usize,
) -> Result<String, String> {
    let diff = git_diff_head_to_workdir(repository)?;

    let mut diff_str = String::new();
    diff.print(git2::DiffFormat::Patch, |_, _, line| {
        let line_content = std::str::from_utf8(line.content()).unwrap_or("");
        if diff_str.len() + line_content.len() < max_size {
            diff_str.push(line.origin());
            diff_str.push_str(line_content);
            if diff_str.len() > max_size {
                diff_str.truncate(max_size - 4);
                diff_str.push_str("...\n");
            }
        }
        true
    })
    .map_err(|e| format!("Failed to print diff: {}", e))?;

    Ok(diff_str)
}

pub fn checkout_head_and_branch_to_commit(
    repo: &Repository,
    branch_name: &str,
    commit_oid: &Oid,
) -> Result<(), String> {
    let commit = repo
        .find_commit(commit_oid.clone())
        .map_err_with_prefix("Failed to find commit:")?;

    let mut branch_ref = repo
        .find_branch(branch_name, git2::BranchType::Local)
        .map_err_with_prefix("Failed to get branch:")?
        .into_reference();
    branch_ref
        .set_target(commit.id(), "Restoring checkpoint")
        .map_err_with_prefix("Failed to update branch reference:")?;

    repo.set_head(&format!("refs/heads/{}", branch_name))
        .map_err_with_prefix("Failed to set HEAD:")?;

    let mut checkout_opts = git2::build::CheckoutBuilder::new();
    checkout_opts.force().update_index(true);
    repo.checkout_head(Some(&mut checkout_opts))
        .map_err_with_prefix("Failed to checkout HEAD:")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn init_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        (dir, repo)
    }

    fn signature() -> git2::Signature<'static> {
        git2::Signature::now("test", "test@example.com").unwrap()
    }

    fn commit_file(
        repo: &Repository,
        root: &Path,
        path: &str,
        content: &str,
        message: &str,
    ) -> Oid {
        let file_path = root.join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&file_path, content).unwrap();
        commit_paths(repo, &[path], message)
    }

    fn commit_paths(repo: &Repository, paths: &[&str], message: &str) -> Oid {
        let mut index = repo.index().unwrap();
        for path in paths {
            index.add_path(Path::new(path)).unwrap();
        }
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = signature();
        let parents = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| repo.find_commit(oid).ok())
            .into_iter()
            .collect::<Vec<_>>();
        let parent_refs = parents.iter().collect::<Vec<_>>();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .unwrap()
    }

    fn rename_and_commit(
        repo: &Repository,
        root: &Path,
        old_path: &str,
        new_path: &str,
        message: &str,
    ) -> Oid {
        fs::rename(root.join(old_path), root.join(new_path)).unwrap();
        let mut index = repo.index().unwrap();
        index.remove_path(Path::new(old_path)).unwrap();
        index.add_path(Path::new(new_path)).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = signature();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .unwrap()
    }

    fn mine(repo: &Repository, max_commits: usize) -> GitHistoryReport {
        mine_git_history_from_repo(
            repo,
            GitHistoryOptions {
                max_commits,
                cochange_threshold: 3,
                max_hotspots: 20,
                max_cochange_pairs: 50,
                ..GitHistoryOptions::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn root_commit_history_is_handled() {
        let (dir, repo) = init_repo();
        let oid = commit_file(
            &repo,
            dir.path(),
            "src/lib.rs",
            "fn main() {}\n",
            "initial decision because setup",
        );

        let report = mine(&repo, 10);

        assert_eq!(report.commits.len(), 1);
        assert_eq!(report.commits[0].oid, oid.to_string());
        assert_eq!(report.commits[0].parent_oids.len(), 0);
        assert_eq!(
            report.commits[0].changes[0].status,
            GitFileChangeStatus::Added
        );
        assert!(report.commits[0]
            .classifications
            .contains(&GitCommitClassification::Decision));
    }

    #[test]
    fn rename_detection_records_old_and_new_path() {
        let (dir, repo) = init_repo();
        commit_file(
            &repo,
            dir.path(),
            "old.rs",
            "line one\nline two\n",
            "initial",
        );
        rename_and_commit(
            &repo,
            dir.path(),
            "old.rs",
            "new.rs",
            "rename old to new because layout",
        );

        let report = mine(&repo, 10);
        let changes = report
            .commits
            .iter()
            .flat_map(|commit| commit.changes.iter())
            .map(|change| (change.status, change.old_path.clone(), change.path.clone()))
            .collect::<Vec<_>>();

        assert!(changes.iter().any(|(status, old, path)| {
            *status == GitFileChangeStatus::Renamed
                && old.as_deref() == Some("old.rs")
                && path == "new.rs"
        }));
    }

    #[test]
    fn repeated_cochange_threshold_creates_deterministic_pair() {
        let (dir, repo) = init_repo();
        for idx in 0..4 {
            fs::write(dir.path().join("b.rs"), format!("b {idx}\n")).unwrap();
            fs::write(dir.path().join("a.rs"), format!("a {idx}\n")).unwrap();
            commit_paths(&repo, &["a.rs", "b.rs"], "fix pair because bug");
        }

        let first = mine(&repo, 20);
        let second = mine(&repo, 20);

        assert_eq!(first.cochanges, second.cochanges);
        assert!(first
            .cochanges
            .iter()
            .any(|pair| pair.path_a == "a.rs" && pair.path_b == "b.rs" && pair.count >= 3));
    }

    #[test]
    fn hotspot_scoring_is_deterministic_and_capped() {
        let (dir, repo) = init_repo();
        for idx in 0..8 {
            commit_file(
                &repo,
                dir.path(),
                &format!("file_{idx}.rs"),
                &format!("{idx}\n"),
                "initial",
            );
        }
        for idx in 0..6 {
            commit_file(
                &repo,
                dir.path(),
                "hot.rs",
                &format!("{}\n", "x".repeat(idx + 1)),
                "fix hot",
            );
        }

        let report = mine_git_history_from_repo(
            &repo,
            GitHistoryOptions {
                max_commits: 50,
                max_hotspots: 3,
                ..GitHistoryOptions::default()
            },
        )
        .unwrap();
        let again = mine_git_history_from_repo(
            &repo,
            GitHistoryOptions {
                max_commits: 50,
                max_hotspots: 3,
                ..GitHistoryOptions::default()
            },
        )
        .unwrap();

        assert_eq!(report.hotspots, again.hotspots);
        assert_eq!(report.hotspots.len(), 3);
        assert_eq!(report.hotspots[0].path, "hot.rs");
    }

    #[test]
    fn bugfix_commit_message_creates_classified_source_sha() {
        let (dir, repo) = init_repo();
        let oid = commit_file(
            &repo,
            dir.path(),
            "bug.rs",
            "fixed\n",
            "fix parser bug because crash",
        );

        let report = mine(&repo, 10);
        let commit = report
            .commits
            .iter()
            .find(|commit| commit.oid == oid.to_string())
            .unwrap();

        assert!(commit
            .classifications
            .contains(&GitCommitClassification::Bugfix));
        assert!(commit
            .classifications
            .contains(&GitCommitClassification::Rationale));
        assert_eq!(
            commit.short_oid,
            oid.to_string().chars().take(12).collect::<String>()
        );
    }

    #[test]
    fn detached_head_history_does_not_fail_or_mutate_repo() {
        let (dir, repo) = init_repo();
        let first = commit_file(&repo, dir.path(), "a.rs", "a\n", "initial");
        commit_file(&repo, dir.path(), "b.rs", "b\n", "fix b bug");
        repo.set_head_detached(first).unwrap();

        let before = repo.head().unwrap().target();
        let report = mine(&repo, 10);
        let after = repo.head().unwrap().target();

        assert_eq!(before, after);
        assert_eq!(report.commits[0].oid, first.to_string());
    }

    #[test]
    fn traversal_cap_is_respected() {
        let (dir, repo) = init_repo();
        for idx in 0..6 {
            commit_file(
                &repo,
                dir.path(),
                "cap.rs",
                &format!("{idx}\n"),
                &format!("fix {idx}"),
            );
        }

        let report = mine(&repo, 3);

        assert_eq!(report.commits.len(), 3);
        assert!(report.commit_cap_hit);
    }

    #[test]
    fn list_branches_marks_current_branch() {
        let (dir, repo) = init_repo();
        let oid = commit_file(&repo, dir.path(), "file.txt", "base\n", "initial");
        let commit = repo.find_commit(oid).unwrap();
        repo.branch("feature", &commit, false).unwrap();

        let branches = list_branches(&repo).unwrap();

        assert_eq!(branches.len(), 2);
        assert!(branches
            .iter()
            .any(|branch| branch.name == "feature" && !branch.is_head));
        assert_eq!(branches.iter().filter(|branch| branch.is_head).count(), 1);
    }

    #[test]
    fn unstage_changes_moves_staged_modification_to_workdir() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "file.txt", "base\n", "initial");
        fs::write(dir.path().join("file.txt"), "changed\n").unwrap();
        let change = FileChange {
            relative_path: PathBuf::from("file.txt"),
            absolute_path: dir.path().join("file.txt"),
            status: FileChangeStatus::MODIFIED,
        };
        stage_changes(&repo, &[change], &Arc::new(AtomicBool::new(false))).unwrap();
        let (staged, unstaged) =
            get_diff_statuses(git2::StatusShow::IndexAndWorkdir, &repo, false).unwrap();
        assert_eq!(staged.len(), 1);
        assert!(unstaged.is_empty());

        assert_eq!(
            unstage_changes(&repo, &[PathBuf::from("file.txt")]).unwrap(),
            1
        );

        let (staged, unstaged) =
            get_diff_statuses(git2::StatusShow::IndexAndWorkdir, &repo, false).unwrap();
        assert!(staged.is_empty());
        assert_eq!(unstaged.len(), 1);
        assert_eq!(unstaged[0].relative_path, PathBuf::from("file.txt"));
    }
}
