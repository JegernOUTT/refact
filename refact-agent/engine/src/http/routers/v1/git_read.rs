use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::extract::{Query, RawQuery, State};
use axum::response::Json;
use git2::{DiffOptions, Repository};
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app_state::AppState;
use crate::git::operations::{
    changed_files_for_commit_oid, stage_changes, unstage_changes, GitFileChangeStatus,
    MAX_GIT_HISTORY_FILES_PER_COMMIT,
};
use crate::git::{FileChange, FileChangeStatus};
use crate::global_context::GlobalContext;

const MAX_PATCH_BYTES: usize = 512 * 1024;
const MAX_LOG_COMMITS: usize = 500;
const MAX_LOG_SCAN_COMMITS: usize = 50_000;
const MAX_CONTAINING_BRANCHES: usize = 50;
const MAX_CONTAINING_BRANCHES_EXAMINED: usize = 500;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<Value>)>;

#[derive(Debug, Deserialize)]
pub struct RootQuery {
    #[serde(default)]
    pub root: Option<String>,
}

#[derive(Debug, Default)]
pub struct DiffQuery {
    pub root: Option<String>,
    pub paths: Vec<String>,
    pub staged: bool,
    pub against_head: bool,
}

fn parse_query_bool(value: &str) -> Result<bool, String> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" | "" => Ok(false),
        _ => Err(format!("Invalid boolean query value: {}", value)),
    }
}

fn parse_diff_query(raw: Option<&str>) -> Result<DiffQuery, String> {
    let mut query = DiffQuery::default();
    for (key, value) in url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "root" => query.root = Some(value.into_owned()),
            "path" => query.paths.push(value.into_owned()),
            "staged" => query.staged = parse_query_bool(&value)?,
            "against" => match value.as_ref() {
                "head" | "HEAD" => query.against_head = true,
                "index" | "" => query.against_head = false,
                other => return Err(format!("Unsupported diff base: {}", other)),
            },
            _ => {}
        }
    }
    Ok(query)
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default = "default_log_limit")]
    pub limit: usize,
    #[serde(default)]
    pub skip: usize,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PathsRequest {
    pub root: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct RootsResponse<T> {
    pub roots: Vec<T>,
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

#[derive(Debug, Deserialize)]
pub struct GitCommitDetailsQuery {
    #[serde(default)]
    pub root: Option<String>,
    pub oid: String,
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

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct StageResponse {
    pub staged: usize,
    pub skipped: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct UnstageResponse {
    pub unstaged: usize,
}

fn default_log_limit() -> usize {
    50
}

pub(super) fn api_error_code(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({ "code": code, "error": message.into() })),
    )
}

pub(super) fn api_error(
    status: StatusCode,
    message: impl Into<String>,
) -> (StatusCode, Json<Value>) {
    let code = match status {
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::SERVICE_UNAVAILABLE => "git_unavailable",
        StatusCode::GATEWAY_TIMEOUT => "timeout",
        _ => "git_error",
    };
    api_error_code(status, code, message)
}

pub(super) fn internal_error(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    api_error(StatusCode::INTERNAL_SERVER_ERROR, message)
}

fn resolve_enclosing_root(roots: &[PathBuf], requested: &Path) -> Option<PathBuf> {
    roots
        .iter()
        .filter(|root| requested.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

fn effective_git_roots_from(
    mut roots: Vec<PathBuf>,
    workspace_folders: &[PathBuf],
) -> Vec<PathBuf> {
    for folder in workspace_folders {
        let Ok(folder) = dunce::canonicalize(folder) else {
            continue;
        };
        if roots.iter().any(|root| folder.starts_with(root)) {
            continue;
        }
        let Some(workdir) = Repository::discover(&folder)
            .ok()
            .and_then(|repository| repository.workdir().map(Path::to_path_buf))
        else {
            continue;
        };
        let Ok(workdir) = dunce::canonicalize(&workdir) else {
            continue;
        };
        if !roots.contains(&workdir) {
            roots.push(workdir);
        }
    }
    roots
}

async fn effective_git_roots(
    gcx: Arc<GlobalContext>,
) -> Result<Vec<PathBuf>, (StatusCode, Json<Value>)> {
    let configured = gcx
        .documents_state
        .workspace_vcs_roots
        .lock()
        .unwrap()
        .clone();
    let mut roots = Vec::with_capacity(configured.len());
    for root in configured {
        match tokio::fs::canonicalize(&root).await {
            Ok(canonical) => roots.push(dunce::simplified(&canonical).to_path_buf()),
            Err(error) => tracing::warn!(
                "Skipping unavailable configured git root {}: {}",
                root.display(),
                error
            ),
        }
    }
    let folders = crate::files_correction::get_unscoped_project_dirs(gcx).await;
    tokio::task::spawn_blocking(move || effective_git_roots_from(roots, &folders))
        .await
        .map_err(|error| internal_error(format!("Git root discovery task failed: {}", error)))
}

fn registered_worktree_root(
    cache_dir: &Path,
    roots: &[PathBuf],
    requested: &Path,
) -> Option<PathBuf> {
    let worktrees_dir = dunce::canonicalize(cache_dir.join("worktrees")).ok()?;
    crate::files_correction::registered_worktree_path_mappings(cache_dir)
        .iter()
        .filter_map(|mapping| {
            let root = dunce::canonicalize(&mapping.root).ok()?;
            let source = dunce::canonicalize(&mapping.source_root).ok()?;
            let in_scope = root.starts_with(&worktrees_dir)
                && root != worktrees_dir
                && requested.starts_with(&root)
                && resolve_enclosing_root(roots, &source).is_some();
            (in_scope && linked_checkout_matches(&root, &source)).then_some(root)
        })
        .max_by_key(|root| root.components().count())
}

fn linked_checkout_matches(root: &Path, source: &Path) -> bool {
    let (Ok(worktree), Ok(source_repository)) =
        (Repository::open(root), Repository::discover(source))
    else {
        return false;
    };
    let workdir = worktree
        .workdir()
        .and_then(|workdir| dunce::canonicalize(workdir).ok());
    let common_dir = |repository: &Repository| dunce::canonicalize(repository.commondir()).ok();
    let worktree_common_dir = common_dir(&worktree);
    worktree.is_worktree()
        && workdir.as_deref() == Some(root)
        && worktree_common_dir.is_some()
        && worktree_common_dir == common_dir(&source_repository)
}

pub(super) async fn selected_roots(
    gcx: Arc<GlobalContext>,
    requested: Option<String>,
) -> Result<Vec<PathBuf>, (StatusCode, Json<Value>)> {
    let cache_dir = gcx.cache_dir.clone();
    let roots = effective_git_roots(gcx).await?;

    let Some(requested) = requested else {
        return Ok(roots);
    };
    let requested = tokio::fs::canonicalize(requested)
        .await
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "Git root not found"))?;
    let selected = match resolve_enclosing_root(&roots, &requested) {
        Some(root) => Some(root),
        None => tokio::task::spawn_blocking(move || {
            registered_worktree_root(&cache_dir, &roots, &requested)
        })
        .await
        .map_err(|error| internal_error(format!("Git root discovery task failed: {}", error)))?,
    };
    selected.map(|root| vec![root]).ok_or_else(|| {
        api_error(
            StatusCode::NOT_FOUND,
            "Git root is not an active workspace root",
        )
    })
}

pub(super) async fn resolve_requested_root(
    gcx: Arc<GlobalContext>,
    requested: String,
) -> Result<PathBuf, String> {
    selected_roots(gcx, Some(requested))
        .await
        .map_err(|(_, Json(error))| {
            error
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Git root not found")
                .to_string()
        })?
        .into_iter()
        .next()
        .ok_or_else(|| "Git root not found".to_string())
}

pub(super) fn repo_relative_paths(paths: Vec<String>) -> Result<Vec<PathBuf>, String> {
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

const MAX_STATUS_ENTRIES: usize = 10_000;

fn repo_state_name(state: git2::RepositoryState) -> &'static str {
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

fn path_bytes_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace('\\', "/")
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

pub(super) struct StatusSnapshot {
    pub staged: Vec<FileChangeJson>,
    pub unstaged: Vec<FileChangeJson>,
    pub changes: Vec<GitChangeJson>,
    pub untracked: Vec<GitPathJson>,
    pub conflicted: Vec<GitConflictJson>,
    pub truncated: bool,
}

pub(super) fn read_status_snapshot(repository: &Repository) -> Result<StatusSnapshot, String> {
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
    };
    let mut new_entries = 0usize;
    for entry in statuses.iter() {
        let status = entry.status();
        if status.is_conflicted() || status.is_ignored() {
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

fn status_for_repo(root: &Path) -> Result<GitStatusRoot, String> {
    let repository =
        Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
    let snapshot = read_status_snapshot(&repository)?;
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

    Ok(GitStatusRoot {
        root: root.to_string_lossy().to_string(),
        branch,
        head_detached,
        ahead,
        behind,
        staged: snapshot.staged,
        unstaged: snapshot.unstaged,
        untracked_included: true,
        head_oid,
        upstream,
        repo_state: repo_state_name(repository.state()).to_string(),
        truncated: snapshot.truncated,
        changes: snapshot.changes,
        untracked: snapshot.untracked,
        conflicted: snapshot.conflicted,
    })
}

fn validate_diff_paths(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    repo_relative_paths(paths.to_vec())
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
enum DiffBase {
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

fn diff_for_repo<'a>(
    root: &Path,
    paths: impl IntoIterator<Item = &'a str>,
    base: impl Into<DiffBase>,
) -> Result<GitDiffRoot, String> {
    let repository =
        Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
    let paths = validate_diff_paths(&paths.into_iter().map(str::to_string).collect::<Vec<_>>())?;
    let base = base.into();
    let mut options = DiffOptions::new();
    options.include_typechange(true);
    if !paths.is_empty() {
        options.disable_pathspec_match(true);
        for path in &paths {
            options.pathspec(path.to_string_lossy().replace('\\', "/"));
        }
    }
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

type HttpError = (StatusCode, Json<Value>);

#[derive(Debug, Default, Clone)]
pub(super) struct LogFilters {
    pub scope_all: bool,
    pub reference: Option<String>,
    pub text: Option<String>,
    pub author: Option<String>,
    pub path: Option<PathBuf>,
}

impl LogFilters {
    fn from_query(query: &LogQuery) -> Result<Self, HttpError> {
        let scope_all = match query.scope.as_deref().map(str::trim) {
            None | Some("") | Some("head") => false,
            Some("all") => true,
            Some(other) => {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    format!("Unsupported log scope: {}", other),
                ))
            }
        };
        let non_empty = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        };
        let reference = non_empty(&query.reference);
        if let Some(reference) = &reference {
            validate_rev_input(reference)
                .map_err(|error| api_error(StatusCode::BAD_REQUEST, error))?;
        }
        // Tree lookups need '/' separators; on Windows accept the native separator too.
        #[cfg(windows)]
        let query_path = non_empty(&query.path).map(|path| path.replace('\\', "/"));
        #[cfg(not(windows))]
        let query_path = non_empty(&query.path);
        let path = match query_path {
            Some(path) => repo_relative_paths(vec![path])
                .map_err(|error| api_error(StatusCode::BAD_REQUEST, error))?
                .pop(),
            None => None,
        };
        Ok(Self {
            scope_all,
            reference,
            text: non_empty(&query.text).map(|text| text.to_lowercase()),
            author: non_empty(&query.author).map(|author| author.to_lowercase()),
            path,
        })
    }

    fn filtering(&self) -> bool {
        self.text.is_some() || self.author.is_some() || self.path.is_some()
    }
}

/// Rejects revision input git could parse as an option or that is not a plausible ref/rev.
pub(super) fn validate_rev_input(value: &str) -> Result<(), String> {
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
pub(super) fn ref_labels_by_oid(
    repository: &Repository,
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
        labels
            .entry(commit.id())
            .or_default()
            .push(ref_label(name, kind));
    }
    if let Ok(head) = repository.head().and_then(|head| head.peel_to_commit()) {
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
pub(super) fn resolve_commit_ref<'repo>(
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

fn commit_touches_path(commit: &git2::Commit<'_>, path: &Path) -> bool {
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

fn commit_json(
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

pub(super) fn log_for_repo(
    root: &Path,
    limit: usize,
    skip: usize,
    filters: &LogFilters,
) -> Result<GitLogRoot, HttpError> {
    let repository = Repository::open(root)
        .map_err(|error| internal_error(format!("Failed to open repo: {}", error)))?;
    let git_error = |context: &'static str| {
        move |error: git2::Error| internal_error(format!("{}: {}", context, error))
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
    let labels = ref_labels_by_oid(&repository).map_err(internal_error)?;
    let mut pushed = false;
    if let Some(reference) = &filters.reference {
        let commit = resolve_commit_ref(&repository, reference).ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                format!("Reference not found: {}", reference),
            )
        })?;
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

fn find_commit_by_user_oid<'repo>(
    repository: &'repo Repository,
    oid: &str,
) -> Result<git2::Commit<'repo>, HttpError> {
    let oid = oid.trim();
    if oid.len() < 4 || oid.len() > 40 || !oid.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Commit oid must be 4-40 hex characters",
        ));
    }
    let found = if oid.len() == 40 {
        git2::Oid::from_str(oid).and_then(|oid| repository.find_commit(oid))
    } else {
        repository.find_commit_by_prefix(oid)
    };
    found.map_err(|error| match error.code() {
        git2::ErrorCode::Ambiguous => api_error(
            StatusCode::BAD_REQUEST,
            "Commit oid prefix is ambiguous; use more characters",
        ),
        _ => api_error(StatusCode::NOT_FOUND, "Commit not found"),
    })
}

fn file_status_name(status: GitFileChangeStatus) -> &'static str {
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

pub(super) fn commit_details_for_repo(
    root: &Path,
    oid: &str,
) -> Result<GitCommitDetails, HttpError> {
    let repository = Repository::open(root)
        .map_err(|error| internal_error(format!("Failed to open repo: {}", error)))?;
    let commit = find_commit_by_user_oid(&repository, oid)?;
    let labels = ref_labels_by_oid(&repository).map_err(internal_error)?;
    let (files, files_truncated) =
        changed_files_for_commit_oid(&repository, commit.id(), MAX_GIT_HISTORY_FILES_PER_COMMIT)
            .map_err(internal_error)?;
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

pub(super) fn branches_for_repo(root: &Path) -> Result<GitBranchesRoot, String> {
    let repository =
        Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
    let mut branches = Vec::new();
    for entry in repository
        .branches(Some(git2::BranchType::Local))
        .map_err(|error| format!("Failed to list branches: {}", error))?
    {
        let (branch, _) = entry.map_err(|error| format!("Failed to read branch: {}", error))?;
        branches.extend(local_branch_json(&repository, &branch));
    }
    branches.sort_by(|a, b| a.name.cmp(&b.name));
    let current = branches
        .iter()
        .find(|branch| branch.is_head)
        .map(|branch| branch.name.clone());

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

    let mut remote_branches = Vec::new();
    for entry in repository
        .branches(Some(git2::BranchType::Remote))
        .map_err(|error| format!("Failed to list remote branches: {}", error))?
    {
        let Ok((branch, _)) = entry else {
            continue;
        };
        let reference = branch.get();
        if reference.kind() != Some(git2::ReferenceType::Direct) {
            continue;
        }
        let (Some(full), Some(name)) = (
            reference.name().map(ToString::to_string),
            branch.name().ok().flatten().map(ToString::to_string),
        ) else {
            continue;
        };
        let Ok(tip) = reference.peel_to_commit() else {
            continue;
        };
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
        remote_branches.push(GitRemoteBranchJson {
            name,
            remote,
            short_name,
            tip_oid: tip.id().to_string(),
        });
    }
    remote_branches.sort_by(|a, b| a.name.cmp(&b.name));

    let mut tags = Vec::new();
    if let Ok(references) = repository.references_glob("refs/tags/*") {
        for reference in references.flatten() {
            let Some(name) = reference
                .name()
                .and_then(|name| name.strip_prefix("refs/tags/"))
                .map(ToString::to_string)
            else {
                continue;
            };
            if let Ok(commit) = reference.peel_to_commit() {
                tags.push(GitTagJson {
                    name,
                    target_oid: commit.id().to_string(),
                });
            }
        }
    }
    tags.sort_by(|a, b| a.name.cmp(&b.name));

    let head_oid = repository
        .head()
        .and_then(|head| head.peel_to_commit())
        .ok()
        .map(|commit| commit.id().to_string());
    Ok(GitBranchesRoot {
        root: root.to_string_lossy().to_string(),
        current,
        branches,
        head_detached: repository.head_detached().unwrap_or(false),
        head_oid,
        remotes,
        remote_branches,
        tags,
    })
}

fn inferred_file_changes(repository: &Repository, paths: &[PathBuf]) -> Vec<FileChange> {
    let workdir = repository.workdir().unwrap_or_else(|| Path::new(""));
    paths
        .iter()
        .map(|path| {
            let status = repository
                .status_file(path)
                .unwrap_or(git2::Status::CURRENT);
            let status = if status.is_wt_new() {
                FileChangeStatus::ADDED
            } else if status.is_wt_deleted() || !workdir.join(path).exists() {
                FileChangeStatus::DELETED
            } else {
                FileChangeStatus::MODIFIED
            };
            FileChange {
                relative_path: path.clone(),
                absolute_path: workdir.join(path),
                status,
            }
        })
        .collect()
}

async fn collect_roots<T, F>(roots: Vec<PathBuf>, operation: F) -> ApiResult<RootsResponse<T>>
where
    T: Send + 'static,
    F: Fn(&Path) -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        roots
            .iter()
            .map(|root| operation(root))
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(|error| internal_error(format!("Git operation task failed: {}", error)))?
    .map(|roots| {
        Json(RootsResponse {
            roots,
        })
    })
    .map_err(internal_error)
}

pub async fn handle_v1_git_status(
    State(app): State<AppState>,
    Query(query): Query<RootQuery>,
) -> ApiResult<RootsResponse<GitStatusRoot>> {
    let roots = selected_roots(app.gcx.clone(), query.root).await?;
    collect_roots(roots, status_for_repo).await
}

pub async fn handle_v1_git_diff(
    State(app): State<AppState>,
    RawQuery(raw): RawQuery,
) -> ApiResult<RootsResponse<GitDiffRoot>> {
    let query = parse_diff_query(raw.as_deref())
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error))?;
    validate_diff_paths(&query.paths).map_err(|error| api_error(StatusCode::BAD_REQUEST, error))?;
    let roots = selected_roots(app.gcx.clone(), query.root).await?;
    let paths = query.paths;
    let base = if query.against_head {
        DiffBase::HeadToWorkdir
    } else {
        DiffBase::from(query.staged)
    };
    collect_roots(roots, move |root| {
        diff_for_repo(root, paths.iter().map(String::as_str), base)
    })
    .await
}

pub async fn handle_v1_git_log(
    State(app): State<AppState>,
    Query(query): Query<LogQuery>,
) -> ApiResult<RootsResponse<GitLogRoot>> {
    let filters = LogFilters::from_query(&query)?;
    let roots = selected_roots(app.gcx.clone(), query.root).await?;
    let (limit, skip) = (query.limit, query.skip);
    tokio::task::spawn_blocking(move || {
        roots
            .iter()
            .map(|root| log_for_repo(root, limit, skip, &filters))
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(|error| internal_error(format!("Git operation task failed: {}", error)))?
    .map(|roots| {
        Json(RootsResponse {
            roots,
        })
    })
}

pub async fn handle_v1_git_commit_details(
    State(app): State<AppState>,
    Query(query): Query<GitCommitDetailsQuery>,
) -> ApiResult<GitCommitDetails> {
    let root = selected_roots(app.gcx.clone(), query.root)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "No active git root"))?;
    let oid = query.oid;
    tokio::task::spawn_blocking(move || commit_details_for_repo(&root, &oid))
        .await
        .map_err(|error| internal_error(format!("Git operation task failed: {}", error)))?
        .map(Json)
}

pub async fn handle_v1_git_branches(
    State(app): State<AppState>,
    Query(query): Query<RootQuery>,
) -> ApiResult<RootsResponse<GitBranchesRoot>> {
    let roots = selected_roots(app.gcx.clone(), query.root).await?;
    collect_roots(roots, branches_for_repo).await
}

pub async fn handle_v1_git_stage(
    State(app): State<AppState>,
    Json(request): Json<PathsRequest>,
) -> ApiResult<StageResponse> {
    let roots = selected_roots(app.gcx.clone(), Some(request.root)).await?;
    let root = roots.into_iter().next().ok_or_else(|| {
        api_error(
            StatusCode::NOT_FOUND,
            "Git root is not an active workspace root",
        )
    })?;
    let paths = repo_relative_paths(request.paths)
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error))?;
    let lock = crate::git::cli::repo_mutation_lock(&root);
    let _guard = lock.lock().await;
    tokio::task::spawn_blocking(move || {
        let repository =
            Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
        let changes = inferred_file_changes(&repository, &paths);
        let skipped = stage_changes(&repository, &changes, &Arc::new(AtomicBool::new(false)))?;
        Ok::<StageResponse, String>(StageResponse {
            staged: paths.len().saturating_sub(skipped),
            skipped,
        })
    })
    .await
    .map_err(|error| internal_error(format!("Git operation task failed: {}", error)))?
    .map(Json)
    .map_err(internal_error)
}

pub async fn handle_v1_git_unstage(
    State(app): State<AppState>,
    Json(request): Json<PathsRequest>,
) -> ApiResult<UnstageResponse> {
    let roots = selected_roots(app.gcx.clone(), Some(request.root)).await?;
    let root = roots.into_iter().next().ok_or_else(|| {
        api_error(
            StatusCode::NOT_FOUND,
            "Git root is not an active workspace root",
        )
    })?;
    let paths = repo_relative_paths(request.paths)
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error))?;
    let lock = crate::git::cli::repo_mutation_lock(&root);
    let _guard = lock.lock().await;
    tokio::task::spawn_blocking(move || {
        let repository =
            Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
        unstage_changes(&repository, &paths).map(|unstaged| UnstageResponse {
            unstaged,
        })
    })
    .await
    .map_err(|error| internal_error(format!("Git operation task failed: {}", error)))?
    .map(Json)
    .map_err(internal_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn init_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        (dir, repo)
    }

    async fn get_json(router: &axum::Router, uri: &str) -> (StatusCode, Value) {
        let response = router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[test]
    fn diff_for_repo_preserves_content_prefixes_and_cleans_metadata_origins() {
        let (dir, repo) = init_repo();
        let base = (1..=24)
            .map(|line| format!("line {line:02}\n"))
            .collect::<String>();
        fs::write(dir.path().join("file.txt"), &base).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("Test User", "test@example.com").unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "Initial commit",
            &tree,
            &[],
        )
        .unwrap();

        let modified = base
            .replace("line 02\n", "changed 02\n")
            .replace("line 22\n", "changed 22\n");
        fs::write(dir.path().join("file.txt"), modified).unwrap();
        fs::write(
            dir.path().join("image.bin"),
            b"\x89PNG\r\n\x1a\n\0binary payload",
        )
        .unwrap();

        let patch = diff_for_repo(dir.path(), None, false).unwrap().patch;
        let lines = patch.lines().collect::<Vec<_>>();

        assert!(lines.iter().any(|line| *line == "-line 02"));
        assert!(lines.iter().any(|line| *line == "+changed 02"));
        assert!(lines.iter().any(|line| *line == " line 03"));
        assert!(lines.iter().filter(|line| line.starts_with("@@ ")).count() >= 2);
        assert!(lines
            .iter()
            .any(|line| *line == "diff --git a/image.bin b/image.bin"));
        assert!(lines
            .iter()
            .any(|line| *line == "Binary files /dev/null and b/image.bin differ"));
        assert!(!patch.contains("Fdiff"));
        assert!(!patch.contains("BBinary"));
        assert!(!patch.contains("H@@"));
    }

    #[test]
    fn resolve_enclosing_root_matches_exact_subdir_and_rejects_outside() {
        let roots = vec![PathBuf::from("/repo"), PathBuf::from("/repo/nested")];
        assert_eq!(
            resolve_enclosing_root(&roots, Path::new("/repo")),
            Some(PathBuf::from("/repo"))
        );
        assert_eq!(
            resolve_enclosing_root(&roots, Path::new("/repo/refact-agent/engine")),
            Some(PathBuf::from("/repo"))
        );
        assert_eq!(
            resolve_enclosing_root(&roots, Path::new("/repo/nested/src")),
            Some(PathBuf::from("/repo/nested"))
        );
        assert_eq!(resolve_enclosing_root(&roots, Path::new("/outside")), None);
        assert_eq!(
            resolve_enclosing_root(&roots, Path::new("/repo-sibling")),
            None
        );
    }

    #[tokio::test]
    async fn selected_roots_maps_subdir_and_symlink_to_enclosing_root() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let active = tempfile::tempdir().unwrap();
        let subdir = active.path().join("refact-agent").join("engine");
        fs::create_dir_all(&subdir).unwrap();
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() =
            vec![active.path().to_path_buf()];
        let canonical_root = dunce::canonicalize(active.path()).unwrap();

        let selected = selected_roots(gcx.clone(), Some(subdir.to_string_lossy().to_string()))
            .await
            .unwrap();
        assert_eq!(selected, vec![canonical_root.clone()]);

        #[cfg(unix)]
        {
            let link_holder = tempfile::tempdir().unwrap();
            let link = link_holder.path().join("engine-link");
            std::os::unix::fs::symlink(&subdir, &link).unwrap();
            let selected = selected_roots(gcx.clone(), Some(link.to_string_lossy().to_string()))
                .await
                .unwrap();
            assert_eq!(selected, vec![canonical_root.clone()]);
        }

        let outside = tempfile::tempdir().unwrap();
        let error = selected_roots(gcx, Some(outside.path().to_string_lossy().to_string()))
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn auth_matrix_git_stage_rejects_non_relative_paths() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let active = tempfile::tempdir().unwrap();
        Repository::init(active.path()).unwrap();
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() =
            vec![active.path().to_path_buf()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let router = crate::http::routers::make_refact_http_server(app);

        for bad_path in ["../evil", "/etc/passwd"] {
            let body = json!({
                "root": active.path().to_string_lossy(),
                "paths": [bad_path],
            })
            .to_string();
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/git/stage")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
            let payload: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_path}");
            assert_eq!(payload["code"], "bad_request", "{bad_path}");
        }
    }
}
