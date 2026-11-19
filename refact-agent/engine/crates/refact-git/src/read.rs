use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};

use git2::{DiffOptions, Repository};
use serde::Serialize;

use crate::operations::{
    changed_files_for_commit_oid, GitFileChangeStatus, MAX_GIT_HISTORY_FILES_PER_COMMIT,
};
use crate::{FileChange, FileChangeStatus};

pub const MAX_PATCH_BYTES: usize = 512 * 1024;
pub const MAX_LOG_COMMITS: usize = 500;
pub const MAX_LOG_SCAN_COMMITS: usize = 50_000;
const MAX_CONTAINING_BRANCHES: usize = 50;
const MAX_CONTAINING_BRANCHES_EXAMINED: usize = 500;
pub const MAX_STATUS_ENTRIES: usize = 10_000;
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const BINARY_PROBE_BYTES: usize = 8000;

/// A read failure with the HTTP class the Git rail answers it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitReadError {
    BadRequest(String),
    NotFound(String),
    Internal(String),
}

impl GitReadError {
    pub fn message(&self) -> &str {
        match self {
            Self::BadRequest(message) | Self::NotFound(message) | Self::Internal(message) => {
                message
            }
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FileChangeJson {
    pub relative_path: String,
    pub absolute_path: String,
    pub status: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitStatusRoot {
    pub root: String,
    pub branch: Option<String>,
    pub head_detached: bool,
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    pub staged: Vec<FileChangeJson>,
    pub unstaged: Vec<FileChangeJson>,
    pub untracked_included: bool,
    pub head_oid: Option<String>,
    pub upstream: Option<String>,
    pub repo_state: String,
    pub truncated: bool,
    pub changes: Vec<GitChangeJson>,
    pub untracked: Vec<GitPathJson>,
    pub conflicted: Vec<GitConflictJson>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitChangeJson {
    pub relative_path: String,
    pub absolute_path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub staged: bool,
    pub unstaged: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitPathJson {
    pub relative_path: String,
    pub absolute_path: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitConflictJson {
    pub relative_path: String,
    pub absolute_path: String,
    pub ours: bool,
    pub theirs: bool,
    pub ancestor: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitDiffRoot {
    pub root: String,
    pub patch: String,
    pub truncated: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitLogRoot {
    pub root: String,
    pub commits: Vec<GitCommitJson>,
    pub head_oid: Option<String>,
    pub scan_truncated: bool,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct GitRefLabel {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitCommitJson {
    pub oid: String,
    pub short_oid: String,
    pub time_ms: i64,
    pub author_name: String,
    pub author_email: String,
    pub message_first_line: String,
    pub message: String,
    pub parents: Vec<String>,
    pub refs: Vec<GitRefLabel>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitBranchesRoot {
    pub root: String,
    pub current: Option<String>,
    pub branches: Vec<GitBranchJson>,
    pub head_detached: bool,
    pub head_oid: Option<String>,
    pub remotes: Vec<String>,
    pub remote_branches: Vec<GitRemoteBranchJson>,
    pub tags: Vec<GitTagJson>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitBranchJson {
    pub name: String,
    pub is_head: bool,
    pub upstream: Option<String>,
    pub tip_oid: Option<String>,
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    pub upstream_gone: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitRemoteBranchJson {
    pub name: String,
    pub remote: String,
    pub short_name: String,
    pub tip_oid: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitTagJson {
    pub name: String,
    pub target_oid: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitCommitFileJson {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitCommitDetails {
    pub root: String,
    pub oid: String,
    pub short_oid: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub author_time_ms: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_time_ms: i64,
    pub message: String,
    pub refs: Vec<GitRefLabel>,
    pub branches: Vec<String>,
    pub branches_truncated: bool,
    pub files: Vec<GitCommitFileJson>,
    pub files_truncated: bool,
}

pub fn repo_relative_paths(paths: Vec<String>) -> Result<Vec<PathBuf>, String> {
    let mut unique = BTreeSet::new();
    for raw_path in paths {
        let path = PathBuf::from(raw_path);
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err("Git paths must be non-empty paths relative to the repository".to_string());
        }
        unique.insert(path);
    }
    Ok(unique.into_iter().collect())
}

fn file_change_json(change: FileChange) -> FileChangeJson {
    FileChangeJson {
        relative_path: change.relative_path.to_string_lossy().to_string(),
        absolute_path: change.absolute_path.to_string_lossy().to_string(),
        status: match change.status {
            FileChangeStatus::ADDED => "ADDED",
            FileChangeStatus::MODIFIED => "MODIFIED",
            FileChangeStatus::DELETED => "DELETED",
        }
        .to_string(),
    }
}

pub fn repo_state_name(state: git2::RepositoryState) -> &'static str {
    use git2::RepositoryState as State;
    match state {
        State::Clean => "clean",
        State::Merge => "merge",
        State::Rebase | State::RebaseInteractive | State::RebaseMerge => "rebase",
        State::CherryPick | State::CherryPickSequence => "cherry_pick",
        State::Revert | State::RevertSequence => "revert",
        State::Bisect => "bisect",
        State::ApplyMailbox => "apply_mailbox",
        State::ApplyMailboxOrRebase => "other",
    }
}

pub fn path_bytes_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace('\\', "/")
}

/// A repository path that `path_bytes_string` keeps as it is: UTF-8 without a backslash.
pub fn exact_path(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes)
        .ok()
        .filter(|path| !path.contains('\\'))
}

fn legacy_change(workdir: &Path, relative: &str, status: FileChangeStatus) -> FileChangeJson {
    file_change_json(FileChange {
        relative_path: PathBuf::from(relative),
        absolute_path: workdir.join(relative),
        status,
    })
}

const INDEX_FLAGS: git2::Status = git2::Status::INDEX_NEW
    .union(git2::Status::INDEX_MODIFIED)
    .union(git2::Status::INDEX_DELETED)
    .union(git2::Status::INDEX_RENAMED)
    .union(git2::Status::INDEX_TYPECHANGE);
const WT_FLAGS: git2::Status = git2::Status::WT_NEW
    .union(git2::Status::WT_MODIFIED)
    .union(git2::Status::WT_DELETED)
    .union(git2::Status::WT_RENAMED)
    .union(git2::Status::WT_TYPECHANGE);

pub struct StatusSnapshot {
    pub staged: Vec<FileChangeJson>,
    pub unstaged: Vec<FileChangeJson>,
    pub changes: Vec<GitChangeJson>,
    pub untracked: Vec<GitPathJson>,
    pub conflicted: Vec<GitConflictJson>,
    pub truncated: bool,
    /// Entries left out because `StatusScan::exact_names` was set and a name was not exact.
    pub inexact: usize,
}

/// How a status scan differs from the Git rail's (the default).
#[derive(Default)]
pub struct StatusScan<'a> {
    /// Leave submodule checkouts unread.
    pub exclude_submodules: bool,
    /// When not empty, scan only these literal paths and the folders under them.
    pub pathspecs: &'a [String],
    /// Leave out entries with a name `exact_path` refuses, counting them in `inexact`.
    pub exact_names: bool,
}

pub fn read_status_snapshot(repository: &Repository) -> Result<StatusSnapshot, String> {
    read_status_snapshot_with(repository, &StatusScan::default())
}

pub fn read_status_snapshot_with(
    repository: &Repository,
    scan: &StatusScan,
) -> Result<StatusSnapshot, String> {
    let repo_workdir = repository
        .workdir()
        .ok_or_else(|| "Failed to get workdir from repository".to_string())?
        .to_path_buf();
    let workdir = dunce::canonicalize(&repo_workdir).unwrap_or_else(|_| repo_workdir.clone());
    let mut options = git2::StatusOptions::new();
    options
        .disable_pathspec_match(true)
        .include_ignored(false)
        .include_unmodified(false)
        .include_unreadable(false)
        .include_untracked(true)
        .recurse_ignored_dirs(false)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .update_index(false)
        .show(git2::StatusShow::IndexAndWorkdir);
    if scan.exclude_submodules {
        options.exclude_submodules(true);
    }
    for pathspec in scan.pathspecs {
        options.pathspec(pathspec.as_str());
    }
    let statuses = repository
        .statuses(Some(&mut options))
        .map_err(|error| format!("Failed to get statuses: {}", error))?;

    let mut snapshot = StatusSnapshot {
        staged: Vec::new(),
        unstaged: Vec::new(),
        changes: Vec::new(),
        untracked: Vec::new(),
        conflicted: Vec::new(),
        truncated: false,
        inexact: 0,
    };
    let mut new_entries = 0usize;
    for entry in statuses.iter() {
        let status = entry.status();
        if status.is_conflicted() || status.is_ignored() {
            continue;
        }
        if scan.exact_names
            && ![entry.head_to_index(), entry.index_to_workdir()]
                .into_iter()
                .flatten()
                .flat_map(|delta| [delta.old_file().path_bytes(), delta.new_file().path_bytes()])
                .chain([entry.path_bytes().into()])
                .flatten()
                .all(|bytes| exact_path(bytes).is_some())
        {
            snapshot.inexact += 1;
            continue;
        }
        let path = path_bytes_string(entry.path_bytes());
        if path.ends_with('/') && repo_workdir.join(&path).join(".git").exists() {
            continue;
        }
        let renamed = entry.head_to_index().filter(|_| status.is_index_renamed());
        let (current, old_path) = match renamed {
            Some(delta) => (
                delta
                    .new_file()
                    .path_bytes()
                    .map(path_bytes_string)
                    .unwrap_or_else(|| path.clone()),
                delta.old_file().path_bytes().map(path_bytes_string),
            ),
            None => (path.clone(), None),
        };
        let in_index = status.intersects(INDEX_FLAGS);
        let in_wt = status.intersects(WT_FLAGS);

        if let Some(old) = &old_path {
            snapshot
                .staged
                .push(legacy_change(&workdir, old, FileChangeStatus::DELETED));
            snapshot
                .staged
                .push(legacy_change(&workdir, &current, FileChangeStatus::ADDED));
        } else if in_index {
            snapshot.staged.push(legacy_change(
                &workdir,
                &current,
                if status.is_index_new() {
                    FileChangeStatus::ADDED
                } else if status.is_index_deleted() {
                    FileChangeStatus::DELETED
                } else {
                    FileChangeStatus::MODIFIED
                },
            ));
        }
        if in_wt {
            snapshot.unstaged.push(legacy_change(
                &workdir,
                &current,
                if status.is_wt_new() {
                    FileChangeStatus::ADDED
                } else if status.is_wt_deleted() {
                    FileChangeStatus::DELETED
                } else {
                    FileChangeStatus::MODIFIED
                },
            ));
        }

        if !in_index && !in_wt {
            continue;
        }
        if new_entries >= MAX_STATUS_ENTRIES {
            snapshot.truncated = true;
            continue;
        }
        if status.is_wt_new() && !in_index {
            new_entries += 1;
            snapshot.untracked.push(GitPathJson {
                absolute_path: workdir.join(&current).to_string_lossy().to_string(),
                relative_path: current,
            });
            continue;
        }
        // A staged add that was later deleted from disk (INDEX_NEW|WT_DELETED) is still a
        // pending change: it surfaces as ADDED, staged and unstaged, so it can be rolled back.
        let (relative, old_path, combined) = if let Some(old) = old_path {
            if status.is_wt_deleted() {
                (old, None, "DELETED")
            } else {
                (current, Some(old), "RENAMED")
            }
        } else if status.is_index_new() {
            (current, None, "ADDED")
        } else if status.is_index_deleted() && !status.is_wt_new() {
            (current, None, "DELETED")
        } else if status.is_wt_deleted() {
            (current, None, "DELETED")
        } else if status.is_index_typechange() || status.is_wt_typechange() {
            (current, None, "TYPECHANGE")
        } else {
            (current, None, "MODIFIED")
        };
        new_entries += 1;
        snapshot.changes.push(GitChangeJson {
            absolute_path: workdir.join(&relative).to_string_lossy().to_string(),
            relative_path: relative,
            old_path,
            status: combined.to_string(),
            staged: in_index,
            unstaged: in_wt,
        });
    }
    snapshot
        .staged
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    snapshot
        .unstaged
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    let index = repository
        .index()
        .map_err(|error| format!("Failed to read index: {}", error))?;
    if index.has_conflicts() {
        let conflicts = index
            .conflicts()
            .map_err(|error| format!("Failed to read conflicts: {}", error))?;
        for conflict in conflicts {
            let conflict =
                conflict.map_err(|error| format!("Failed to read conflict: {}", error))?;
            let Some(path) = [&conflict.our, &conflict.their, &conflict.ancestor]
                .into_iter()
                .flatten()
                .next()
                .map(|entry| path_bytes_string(&entry.path))
            else {
                continue;
            };
            if scan.exact_names
                && [&conflict.our, &conflict.their, &conflict.ancestor]
                    .into_iter()
                    .flatten()
                    .any(|entry| exact_path(&entry.path).is_none())
            {
                snapshot.inexact += 1;
                continue;
            }
            if new_entries >= MAX_STATUS_ENTRIES {
                snapshot.truncated = true;
                break;
            }
            new_entries += 1;
            snapshot.conflicted.push(GitConflictJson {
                absolute_path: workdir.join(&path).to_string_lossy().to_string(),
                relative_path: path,
                ours: conflict.our.is_some(),
                theirs: conflict.their.is_some(),
                ancestor: conflict.ancestor.is_some(),
            });
        }
    }
    Ok(snapshot)
}

/// HEAD, its branch and how far that branch is from its upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadStatus {
    pub branch: Option<String>,
    pub head_detached: bool,
    pub head_oid: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
}

pub fn head_status(repository: &Repository) -> HeadStatus {
    let head_detached = repository.head_detached().unwrap_or(false);
    let head = repository.head().ok();
    let head_oid = head
        .as_ref()
        .and_then(|head| head.target())
        .map(|oid| oid.to_string());
    let branch = head
        .as_ref()
        .filter(|head| head.is_branch())
        .and_then(|head| head.shorthand().map(ToString::to_string));
    let local_branch = branch
        .as_deref()
        .and_then(|name| repository.find_branch(name, git2::BranchType::Local).ok());
    let upstream_branch = local_branch
        .as_ref()
        .and_then(|branch| branch.upstream().ok());
    let upstream = upstream_branch
        .as_ref()
        .and_then(|upstream| upstream.name().ok().flatten().map(ToString::to_string));
    let (ahead, behind) = local_branch
        .as_ref()
        .zip(upstream_branch.as_ref())
        .and_then(|(branch, upstream)| {
            let local_oid = branch.get().peel_to_commit().ok()?.id();
            let upstream_oid = upstream.get().peel_to_commit().ok()?.id();
            repository.graph_ahead_behind(local_oid, upstream_oid).ok()
        })
        .map(|(ahead, behind)| (Some(ahead), Some(behind)))
        .unwrap_or((None, None));
    HeadStatus {
        branch,
        head_detached,
        head_oid,
        upstream,
        ahead,
        behind,
    }
}

pub fn status_for_repo(root: &Path) -> Result<GitStatusRoot, String> {
    let repository =
        Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
    let snapshot = read_status_snapshot(&repository)?;
    let head = head_status(&repository);

    Ok(GitStatusRoot {
        root: root.to_string_lossy().to_string(),
        branch: head.branch,
        head_detached: head.head_detached,
        ahead: head.ahead,
        behind: head.behind,
        staged: snapshot.staged,
        unstaged: snapshot.unstaged,
        untracked_included: true,
        head_oid: head.head_oid,
        upstream: head.upstream,
        repo_state: repo_state_name(repository.state()).to_string(),
        truncated: snapshot.truncated,
        changes: snapshot.changes,
        untracked: snapshot.untracked,
        conflicted: snapshot.conflicted,
    })
}

pub fn validate_diff_paths(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    repo_relative_paths(paths.to_vec())
}

/// Diff options both the Git rail and scripts start from: type changes reported, and the
/// given repository-relative paths taken literally.
pub fn literal_path_diff_options(paths: &[PathBuf]) -> DiffOptions {
    let mut options = DiffOptions::new();
    options.include_typechange(true);
    if !paths.is_empty() {
        options.disable_pathspec_match(true);
        for path in paths {
            options.pathspec(path.to_string_lossy().replace('\\', "/"));
        }
    }
    options
}

fn append_patch_text(patch: &mut String, content: &str, truncated: &mut bool) {
    if *truncated {
        return;
    }
    let remaining = MAX_PATCH_BYTES.saturating_sub(patch.len());
    if content.len() <= remaining {
        patch.push_str(content);
        return;
    }
    let mut end = remaining.min(content.len());
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    patch.push_str(&content[..end]);
    *truncated = true;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffBase {
    IndexToWorkdir,
    HeadToIndex,
    HeadToWorkdir,
}

impl From<bool> for DiffBase {
    fn from(staged: bool) -> Self {
        if staged {
            DiffBase::HeadToIndex
        } else {
            DiffBase::IndexToWorkdir
        }
    }
}

pub fn diff_for_repo<'a>(
    root: &Path,
    paths: impl IntoIterator<Item = &'a str>,
    base: impl Into<DiffBase>,
) -> Result<GitDiffRoot, String> {
    let repository =
        Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
    let paths = validate_diff_paths(&paths.into_iter().map(str::to_string).collect::<Vec<_>>())?;
    let base = base.into();
    let mut options = literal_path_diff_options(&paths);
    let head_tree = || {
        repository
            .head()
            .ok()
            .and_then(|head| head.peel_to_tree().ok())
    };
    let diff = if base == DiffBase::HeadToWorkdir {
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .show_untracked_content(true);
        repository
            .diff_tree_to_workdir_with_index(head_tree().as_ref(), Some(&mut options))
            .map_err(|error| format!("Failed to generate HEAD diff: {}", error))?
    } else if base == DiffBase::HeadToIndex {
        repository
            .diff_tree_to_index(head_tree().as_ref(), None, Some(&mut options))
            .map_err(|error| format!("Failed to generate staged diff: {}", error))?
    } else {
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .show_untracked_content(true);
        repository
            .diff_index_to_workdir(None, Some(&mut options))
            .map_err(|error| format!("Failed to generate unstaged diff: {}", error))?
    };

    let mut patch = String::new();
    let mut truncated = false;
    diff.print(git2::DiffFormat::Patch, |_, _, line| {
        if matches!(line.origin(), '+' | '-' | ' ') {
            let mut prefix = [0u8; 4];
            let prefix = line.origin().encode_utf8(&mut prefix);
            append_patch_text(&mut patch, prefix, &mut truncated);
        }
        append_patch_text(
            &mut patch,
            &String::from_utf8_lossy(line.content()),
            &mut truncated,
        );
        true
    })
    .map_err(|error| format!("Failed to print diff: {}", error))?;

    Ok(GitDiffRoot {
        root: root.to_string_lossy().to_string(),
        patch,
        truncated,
    })
}

#[derive(Debug, Default, Clone)]
pub struct LogFilters {
    pub scope_all: bool,
    pub reference: Option<String>,
    pub text: Option<String>,
    pub author: Option<String>,
    pub path: Option<PathBuf>,
}

impl LogFilters {
    pub fn filtering(&self) -> bool {
        self.text.is_some() || self.author.is_some() || self.path.is_some()
    }
}

/// Rejects revision input git could parse as an option or that is not a plausible ref/rev.
pub fn validate_rev_input(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("Revision must not be empty".to_string());
    }
    if value.starts_with('-') {
        return Err("Revision must not start with '-'".to_string());
    }
    if value
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err("Revision must not contain whitespace or control characters".to_string());
    }
    Ok(())
}

fn ref_label(name: &str, kind: &str) -> GitRefLabel {
    GitRefLabel {
        name: name.to_string(),
        kind: kind.to_string(),
    }
}

fn ref_kind_rank(kind: &str) -> u8 {
    match kind {
        "head" => 0,
        "local" => 1,
        "remote" => 2,
        _ => 3,
    }
}

/// Builds oid -> labels once per request. Symbolic refs (e.g. origin/HEAD) are skipped.
pub fn ref_labels_by_oid(
    repository: &Repository,
) -> Result<HashMap<git2::Oid, Vec<GitRefLabel>>, String> {
    ref_labels_where(repository, |_| true)
}

/// `ref_labels_by_oid` with labels built only for the commits `wanted` accepts.
pub fn ref_labels_where(
    repository: &Repository,
    wanted: impl Fn(git2::Oid) -> bool,
) -> Result<HashMap<git2::Oid, Vec<GitRefLabel>>, String> {
    let mut labels: HashMap<git2::Oid, Vec<GitRefLabel>> = HashMap::new();
    let references = repository
        .references()
        .map_err(|error| format!("Failed to list references: {}", error))?;
    for reference in references.flatten() {
        if reference.kind() != Some(git2::ReferenceType::Direct) {
            continue;
        }
        let Some(full) = reference.name() else {
            continue;
        };
        let (name, kind) = if let Some(name) = full.strip_prefix("refs/heads/") {
            (name, "local")
        } else if let Some(name) = full.strip_prefix("refs/remotes/") {
            (name, "remote")
        } else if let Some(name) = full.strip_prefix("refs/tags/") {
            (name, "tag")
        } else {
            continue;
        };
        let Ok(commit) = reference.peel_to_commit() else {
            continue;
        };
        if !wanted(commit.id()) {
            continue;
        }
        labels
            .entry(commit.id())
            .or_default()
            .push(ref_label(name, kind));
    }
    if let Some(head) = repository
        .head()
        .and_then(|head| head.peel_to_commit())
        .ok()
        .filter(|head| wanted(head.id()))
    {
        labels
            .entry(head.id())
            .or_default()
            .push(ref_label("HEAD", "head"));
    }
    for entry in labels.values_mut() {
        entry.sort_by(|a, b| {
            ref_kind_rank(&a.kind)
                .cmp(&ref_kind_rank(&b.kind))
                .then_with(|| a.name.cmp(&b.name))
        });
    }
    Ok(labels)
}

/// Resolves a user ref: local branch, remote branch, tag, then any revision.
pub fn resolve_commit_ref<'repo>(
    repository: &'repo Repository,
    reference: &str,
) -> Option<git2::Commit<'repo>> {
    for prefix in ["refs/heads/", "refs/remotes/", "refs/tags/"] {
        let full = format!("{}{}", prefix, reference);
        if !git2::Reference::is_valid_name(&full) {
            continue;
        }
        if let Ok(commit) = repository
            .find_reference(&full)
            .and_then(|found| found.peel_to_commit())
        {
            return Some(commit);
        }
    }
    repository
        .revparse_single(reference)
        .and_then(|object| object.peel_to_commit())
        .ok()
}

fn tree_entry_id(tree: &git2::Tree<'_>, path: &Path) -> Option<git2::Oid> {
    tree.get_path(path).ok().map(|entry| entry.id())
}

/// Whether the entry at `path` (a file or a directory) differs from the commit's first parent.
pub fn commit_touches_path(commit: &git2::Commit<'_>, path: &Path) -> bool {
    let Ok(tree) = commit.tree() else {
        return false;
    };
    let current = tree_entry_id(&tree, path);
    let parent = commit
        .parent(0)
        .ok()
        .and_then(|parent| parent.tree().ok())
        .and_then(|tree| tree_entry_id(&tree, path));
    current != parent
}

fn commit_matches(commit: &git2::Commit<'_>, filters: &LogFilters) -> bool {
    if let Some(author) = &filters.author {
        let signature = commit.author();
        let name = signature.name().unwrap_or_default().to_lowercase();
        let email = signature.email().unwrap_or_default().to_lowercase();
        if !name.contains(author.as_str()) && !email.contains(author.as_str()) {
            return false;
        }
    }
    if let Some(text) = &filters.text {
        let oid_match = text.len() >= 4
            && text.chars().all(|ch| ch.is_ascii_hexdigit())
            && commit.id().to_string().starts_with(text.as_str());
        if !oid_match
            && !commit
                .message()
                .unwrap_or_default()
                .to_lowercase()
                .contains(text.as_str())
        {
            return false;
        }
    }
    if let Some(path) = &filters.path {
        if !commit_touches_path(commit, path) {
            return false;
        }
    }
    true
}

pub fn commit_json(
    commit: &git2::Commit<'_>,
    labels: &HashMap<git2::Oid, Vec<GitRefLabel>>,
) -> GitCommitJson {
    let oid = commit.id().to_string();
    let message = commit.message().unwrap_or_default().to_string();
    let author = commit.author();
    GitCommitJson {
        short_oid: oid.chars().take(12).collect(),
        oid,
        time_ms: commit.time().seconds().saturating_mul(1000),
        author_name: author.name().unwrap_or_default().to_string(),
        author_email: author.email().unwrap_or_default().to_string(),
        message_first_line: message.lines().next().unwrap_or_default().to_string(),
        message,
        parents: commit.parent_ids().map(|oid| oid.to_string()).collect(),
        refs: labels.get(&commit.id()).cloned().unwrap_or_default(),
    }
}

pub fn log_for_repo(
    root: &Path,
    limit: usize,
    skip: usize,
    filters: &LogFilters,
) -> Result<GitLogRoot, GitReadError> {
    let repository = Repository::open(root)
        .map_err(|error| GitReadError::Internal(format!("Failed to open repo: {}", error)))?;
    let git_error = |context: &'static str| {
        move |error: git2::Error| GitReadError::Internal(format!("{}: {}", context, error))
    };
    let head = repository
        .head()
        .and_then(|head| head.peel_to_commit())
        .ok()
        .map(|commit| commit.id());
    let mut result = GitLogRoot {
        root: root.to_string_lossy().to_string(),
        commits: Vec::new(),
        head_oid: head.map(|oid| oid.to_string()),
        scan_truncated: false,
    };
    let mut revwalk = repository
        .revwalk()
        .map_err(git_error("Failed to create revwalk"))?;
    revwalk
        .set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
        .map_err(git_error("Failed to configure revwalk"))?;
    let labels = ref_labels_by_oid(&repository).map_err(GitReadError::Internal)?;
    let mut pushed = false;
    if let Some(reference) = &filters.reference {
        let commit = resolve_commit_ref(&repository, reference)
            .ok_or_else(|| GitReadError::NotFound(format!("Reference not found: {}", reference)))?;
        revwalk
            .push(commit.id())
            .map_err(git_error("Failed to walk reference"))?;
        pushed = true;
    } else {
        if let Some(head) = head {
            revwalk
                .push(head)
                .map_err(git_error("Failed to walk HEAD"))?;
            pushed = true;
        }
        if filters.scope_all {
            for oid in labels.keys() {
                revwalk
                    .push(*oid)
                    .map_err(git_error("Failed to walk references"))?;
                pushed = true;
            }
        }
    }
    if !pushed {
        return Ok(result);
    }

    let limit = limit.min(MAX_LOG_COMMITS);
    let filtering = filters.filtering();
    // The scan cap also bounds unfiltered walks, so a huge `skip` cannot force an unbounded walk.
    let mut matched = 0usize;
    let mut scanned = 0usize;
    for oid in revwalk {
        if result.commits.len() >= limit {
            break;
        }
        if scanned >= MAX_LOG_SCAN_COMMITS {
            result.scan_truncated = true;
            break;
        }
        scanned += 1;
        let oid = oid.map_err(git_error("Failed to read commit id"))?;
        let commit = repository
            .find_commit(oid)
            .map_err(git_error("Failed to read commit"))?;
        if filtering && !commit_matches(&commit, filters) {
            continue;
        }
        matched += 1;
        if matched <= skip {
            continue;
        }
        result.commits.push(commit_json(&commit, &labels));
    }
    Ok(result)
}

pub fn find_commit_by_user_oid<'repo>(
    repository: &'repo Repository,
    oid: &str,
) -> Result<git2::Commit<'repo>, GitReadError> {
    let oid = oid.trim();
    if oid.len() < 4 || oid.len() > 40 || !oid.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(GitReadError::BadRequest(
            "Commit oid must be 4-40 hex characters".to_string(),
        ));
    }
    let found = if oid.len() == 40 {
        git2::Oid::from_str(oid).and_then(|oid| repository.find_commit(oid))
    } else {
        repository.find_commit_by_prefix(oid)
    };
    found.map_err(|error| match error.code() {
        git2::ErrorCode::Ambiguous => GitReadError::BadRequest(
            "Commit oid prefix is ambiguous; use more characters".to_string(),
        ),
        _ => GitReadError::NotFound("Commit not found".to_string()),
    })
}

pub fn file_status_name(status: GitFileChangeStatus) -> &'static str {
    match status {
        GitFileChangeStatus::Added => "ADDED",
        GitFileChangeStatus::Modified => "MODIFIED",
        GitFileChangeStatus::Deleted => "DELETED",
        GitFileChangeStatus::Renamed => "RENAMED",
        GitFileChangeStatus::TypeChanged => "TYPECHANGE",
    }
}

/// Branches whose tip contains `target`: local branches first, then remote ones. Each check is
/// a graph walk, so the work is bounded by both a match cap and an examined-branch cap; either
/// cap being hit reports `truncated`.
fn branches_containing(repository: &Repository, target: git2::Oid) -> (Vec<String>, bool) {
    let mut names = Vec::new();
    let mut examined = 0usize;
    let mut truncated = false;
    'kinds: for kind in [git2::BranchType::Local, git2::BranchType::Remote] {
        let Ok(branches) = repository.branches(Some(kind)) else {
            continue;
        };
        for (branch, _) in branches.flatten() {
            if branch.get().kind() != Some(git2::ReferenceType::Direct) {
                continue;
            }
            if names.len() > MAX_CONTAINING_BRANCHES || examined >= MAX_CONTAINING_BRANCHES_EXAMINED
            {
                truncated = true;
                break 'kinds;
            }
            examined += 1;
            let Some(name) = branch.name().ok().flatten().map(ToString::to_string) else {
                continue;
            };
            let Some(tip) = branch.get().peel_to_commit().ok().map(|commit| commit.id()) else {
                continue;
            };
            if tip == target || repository.graph_descendant_of(tip, target).unwrap_or(false) {
                names.push(name);
            }
        }
    }
    if names.len() > MAX_CONTAINING_BRANCHES {
        truncated = true;
        names.truncate(MAX_CONTAINING_BRANCHES);
    }
    names.sort();
    names.dedup();
    (names, truncated)
}

pub fn commit_details_for_repo(root: &Path, oid: &str) -> Result<GitCommitDetails, GitReadError> {
    let repository = Repository::open(root)
        .map_err(|error| GitReadError::Internal(format!("Failed to open repo: {}", error)))?;
    let commit = find_commit_by_user_oid(&repository, oid)?;
    let labels = ref_labels_by_oid(&repository).map_err(GitReadError::Internal)?;
    let (files, files_truncated) =
        changed_files_for_commit_oid(&repository, commit.id(), MAX_GIT_HISTORY_FILES_PER_COMMIT)
            .map_err(GitReadError::Internal)?;
    let author = commit.author();
    let committer = commit.committer();
    let oid = commit.id().to_string();
    let (branches, branches_truncated) = branches_containing(&repository, commit.id());
    Ok(GitCommitDetails {
        root: root.to_string_lossy().to_string(),
        short_oid: oid.chars().take(12).collect(),
        oid,
        parents: commit.parent_ids().map(|oid| oid.to_string()).collect(),
        author_name: author.name().unwrap_or_default().to_string(),
        author_email: author.email().unwrap_or_default().to_string(),
        author_time_ms: author.when().seconds().saturating_mul(1000),
        committer_name: committer.name().unwrap_or_default().to_string(),
        committer_email: committer.email().unwrap_or_default().to_string(),
        committer_time_ms: committer.when().seconds().saturating_mul(1000),
        message: commit.message().unwrap_or_default().to_string(),
        refs: labels.get(&commit.id()).cloned().unwrap_or_default(),
        branches,
        branches_truncated,
        files: files
            .into_iter()
            .map(|file| GitCommitFileJson {
                status: file_status_name(file.status).to_string(),
                path: file.path,
                old_path: file.old_path,
                additions: file.additions,
                deletions: file.deletions,
                binary: file.binary,
            })
            .collect(),
        files_truncated,
    })
}

fn local_branch_json(repository: &Repository, branch: &git2::Branch<'_>) -> Option<GitBranchJson> {
    let name = branch.name().ok().flatten()?.to_string();
    let tip = branch.get().peel_to_commit().ok().map(|commit| commit.id());
    let configured = repository
        .branch_upstream_name(&format!("refs/heads/{}", name))
        .is_ok();
    let upstream_branch = branch.upstream().ok();
    let upstream = upstream_branch
        .as_ref()
        .and_then(|upstream| upstream.name().ok().flatten().map(ToString::to_string));
    let upstream_tip = upstream_branch
        .as_ref()
        .and_then(|upstream| upstream.get().peel_to_commit().ok())
        .map(|commit| commit.id());
    let (ahead, behind) = match (tip, upstream_tip) {
        (Some(local), Some(remote)) => repository
            .graph_ahead_behind(local, remote)
            .map(|(ahead, behind)| (Some(ahead), Some(behind)))
            .unwrap_or((None, None)),
        _ => (None, None),
    };
    Some(GitBranchJson {
        name,
        is_head: branch.is_head(),
        upstream,
        tip_oid: tip.map(|oid| oid.to_string()),
        ahead,
        behind,
        upstream_gone: configured && upstream_branch.is_none(),
    })
}

pub fn branches_for_repo(root: &Path) -> Result<GitBranchesRoot, String> {
    let repository =
        Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
    branches_of(&repository, root, usize::MAX).map(|(branches, _)| branches)
}

/// The candidates sorted by name, built in that order until `cap` entries are kept; a candidate
/// that does not build is skipped. The flag says that a buildable candidate was left out.
fn first_by_name<T, U>(
    mut candidates: Vec<(String, T)>,
    cap: usize,
    mut build: impl FnMut(String, T) -> Option<U>,
) -> (Vec<U>, bool) {
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    let mut built = Vec::new();
    for (name, candidate) in candidates {
        if let Some(entry) = build(name, candidate) {
            if built.len() == cap {
                return (built, true);
            }
            built.push(entry);
        }
    }
    (built, false)
}

/// The branch listing of `branches_for_repo` with at most `cap` entries of each kind, the first
/// ones by name, each built only when it is kept; the flag says that entries were left out.
pub fn branches_of(
    repository: &Repository,
    root: &Path,
    cap: usize,
) -> Result<(GitBranchesRoot, bool), String> {
    let mut local = Vec::new();
    for entry in repository
        .branches(Some(git2::BranchType::Local))
        .map_err(|error| format!("Failed to list branches: {}", error))?
    {
        let (branch, _) = entry.map_err(|error| format!("Failed to read branch: {}", error))?;
        if let Some(name) = branch.name().ok().flatten().map(ToString::to_string) {
            local.push((name, branch));
        }
    }
    let current = local
        .iter()
        .find(|(_, branch)| branch.is_head())
        .map(|(name, _)| name.clone());
    let (branches, local_cut) = first_by_name(local, cap, |_, branch| {
        local_branch_json(repository, &branch)
    });

    let mut remotes = repository
        .remotes()
        .map(|remotes| {
            remotes
                .iter()
                .flatten()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    remotes.sort();

    let mut remote = Vec::new();
    for entry in repository
        .branches(Some(git2::BranchType::Remote))
        .map_err(|error| format!("Failed to list remote branches: {}", error))?
    {
        let Ok((branch, _)) = entry else {
            continue;
        };
        if branch.get().kind() != Some(git2::ReferenceType::Direct) {
            continue;
        }
        let (Some(full), Some(name)) = (
            branch.get().name().map(ToString::to_string),
            branch.name().ok().flatten().map(ToString::to_string),
        ) else {
            continue;
        };
        remote.push((name, (full, branch)));
    }
    let (remote_branches, remote_cut) = first_by_name(remote, cap, |name, (full, branch)| {
        let tip = branch.get().peel_to_commit().ok()?;
        let remote = repository
            .branch_remote_name(&full)
            .ok()
            .and_then(|buf| buf.as_str().map(ToString::to_string))
            .or_else(|| name.split('/').next().map(ToString::to_string))
            .unwrap_or_default();
        let short_name = name
            .strip_prefix(&format!("{}/", remote))
            .unwrap_or(&name)
            .to_string();
        Some(GitRemoteBranchJson {
            name,
            remote,
            short_name,
            tip_oid: tip.id().to_string(),
        })
    });

    let mut tags = Vec::new();
    if let Ok(references) = repository.references_glob("refs/tags/*") {
        for reference in references.flatten() {
            if let Some(name) = reference
                .name()
                .and_then(|name| name.strip_prefix("refs/tags/"))
                .map(ToString::to_string)
            {
                tags.push((name, reference));
            }
        }
    }
    let (tags, tags_cut) = first_by_name(tags, cap, |name, reference| {
        reference.peel_to_commit().ok().map(|commit| GitTagJson {
            name,
            target_oid: commit.id().to_string(),
        })
    });

    let head_oid = repository
        .head()
        .and_then(|head| head.peel_to_commit())
        .ok()
        .map(|commit| commit.id().to_string());
    Ok((
        GitBranchesRoot {
            root: root.to_string_lossy().to_string(),
            current,
            branches,
            head_detached: repository.head_detached().unwrap_or(false),
            head_oid,
            remotes,
            remote_branches,
            tags,
        },
        local_cut || remote_cut || tags_cut,
    ))
}

/// File bytes as the Git rail shows them: `(binary, truncated, text)`, text cut at 2 MiB.
pub fn file_payload(bytes: Vec<u8>, size: u64) -> (bool, bool, String) {
    let truncated = bytes.len() > MAX_FILE_BYTES || size as usize > MAX_FILE_BYTES;
    let bytes = &bytes[..bytes.len().min(MAX_FILE_BYTES)];
    if bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0) {
        return (true, truncated, String::new());
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => (false, truncated, text.to_string()),
        Err(error) if truncated && error.error_len().is_none() => (
            false,
            truncated,
            String::from_utf8_lossy(&bytes[..error.valid_up_to()]).to_string(),
        ),
        Err(_) => (true, truncated, String::new()),
    }
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;
