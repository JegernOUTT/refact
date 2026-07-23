use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::extract::{Query, State};
use axum::response::Json;
use git2::{DiffOptions, Repository};
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app_state::AppState;
use crate::git::operations::{get_diff_statuses, list_branches, stage_changes, unstage_changes};
use crate::git::{FileChange, FileChangeStatus};
use crate::global_context::GlobalContext;

const MAX_PATCH_BYTES: usize = 512 * 1024;
const MAX_LOG_COMMITS: usize = 500;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<Value>)>;

#[derive(Debug, Deserialize)]
pub struct RootQuery {
    #[serde(default)]
    pub root: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DiffQuery {
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub staged: bool,
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default = "default_log_limit")]
    pub limit: usize,
    #[serde(default)]
    pub skip: usize,
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
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitBranchesRoot {
    pub root: String,
    pub current: Option<String>,
    pub branches: Vec<GitBranchJson>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GitBranchJson {
    pub name: String,
    pub is_head: bool,
    pub upstream: Option<String>,
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

fn api_error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    let code = match status {
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::NOT_FOUND => "not_found",
        _ => "git_error",
    };
    (
        status,
        Json(json!({ "code": code, "error": message.into() })),
    )
}

fn internal_error(message: impl Into<String>) -> (StatusCode, Json<Value>) {
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

async fn selected_roots(
    gcx: Arc<GlobalContext>,
    requested: Option<String>,
) -> Result<Vec<PathBuf>, (StatusCode, Json<Value>)> {
    let roots = effective_git_roots(gcx).await?;

    let Some(requested) = requested else {
        return Ok(roots);
    };
    let requested = tokio::fs::canonicalize(requested)
        .await
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "Git root not found"))?;
    resolve_enclosing_root(&roots, &requested)
        .map(|root| vec![root])
        .ok_or_else(|| {
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

fn repo_relative_paths(paths: Vec<String>) -> Result<Vec<PathBuf>, String> {
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

fn status_for_repo(root: &Path) -> Result<GitStatusRoot, String> {
    let repository =
        Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
    let (staged, unstaged) =
        get_diff_statuses(git2::StatusShow::IndexAndWorkdir, &repository, true)?;
    let head_detached = repository.head_detached().unwrap_or(false);
    let branch = repository
        .head()
        .ok()
        .filter(|head| head.is_branch())
        .and_then(|head| head.shorthand().map(ToString::to_string));
    let (ahead, behind) = branch
        .as_deref()
        .and_then(|name| repository.find_branch(name, git2::BranchType::Local).ok())
        .and_then(|branch| {
            let local_oid = branch.get().peel_to_commit().ok()?.id();
            let upstream_oid = branch.upstream().ok()?.get().peel_to_commit().ok()?.id();
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
        staged: staged.into_iter().map(file_change_json).collect(),
        unstaged: unstaged.into_iter().map(file_change_json).collect(),
        untracked_included: true,
    })
}

fn validate_diff_path(path: Option<&str>) -> Result<Option<PathBuf>, String> {
    path.map(|path| {
        repo_relative_paths(vec![path.to_string()]).and_then(|mut paths| {
            paths
                .pop()
                .ok_or_else(|| "Git diff path cannot be empty".to_string())
        })
    })
    .transpose()
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

fn diff_for_repo(root: &Path, path: Option<&str>, staged: bool) -> Result<GitDiffRoot, String> {
    let repository =
        Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
    let path = validate_diff_path(path)?;
    let mut options = DiffOptions::new();
    options.include_typechange(true);
    if let Some(path) = &path {
        options.pathspec(path);
    }
    let diff = if staged {
        let head_tree = repository
            .head()
            .ok()
            .and_then(|head| head.peel_to_tree().ok());
        repository
            .diff_tree_to_index(head_tree.as_ref(), None, Some(&mut options))
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
        let mut prefix = [0u8; 4];
        let prefix = line.origin().encode_utf8(&mut prefix);
        append_patch_text(&mut patch, prefix, &mut truncated);
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

fn log_for_repo(root: &Path, limit: usize, skip: usize) -> Result<GitLogRoot, String> {
    let repository =
        Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
    let mut revwalk = repository
        .revwalk()
        .map_err(|error| format!("Failed to create revwalk: {}", error))?;
    revwalk
        .set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
        .map_err(|error| format!("Failed to configure revwalk: {}", error))?;
    if let Ok(head) = repository.head().and_then(|head| head.peel_to_commit()) {
        revwalk
            .push(head.id())
            .map_err(|error| format!("Failed to walk HEAD: {}", error))?;
    } else {
        return Ok(GitLogRoot {
            root: root.to_string_lossy().to_string(),
            commits: Vec::new(),
        });
    }

    let mut commits = Vec::new();
    for oid in revwalk.skip(skip).take(limit.min(MAX_LOG_COMMITS)) {
        let oid = oid.map_err(|error| format!("Failed to read commit id: {}", error))?;
        let commit = repository
            .find_commit(oid)
            .map_err(|error| format!("Failed to read commit: {}", error))?;
        let oid = oid.to_string();
        let message = commit.message().unwrap_or_default().to_string();
        let author = commit.author();
        commits.push(GitCommitJson {
            short_oid: oid.chars().take(12).collect(),
            oid,
            time_ms: commit.time().seconds().saturating_mul(1000),
            author_name: author.name().unwrap_or_default().to_string(),
            author_email: author.email().unwrap_or_default().to_string(),
            message_first_line: message.lines().next().unwrap_or_default().to_string(),
            message,
        });
    }

    Ok(GitLogRoot {
        root: root.to_string_lossy().to_string(),
        commits,
    })
}

fn branches_for_repo(root: &Path) -> Result<GitBranchesRoot, String> {
    let repository =
        Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
    let branches = list_branches(&repository)?;
    let current = branches
        .iter()
        .find(|branch| branch.is_head)
        .map(|branch| branch.name.clone());
    Ok(GitBranchesRoot {
        root: root.to_string_lossy().to_string(),
        current,
        branches: branches
            .into_iter()
            .map(|branch| GitBranchJson {
                name: branch.name,
                is_head: branch.is_head,
                upstream: branch.upstream,
            })
            .collect(),
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
    .map(|roots| Json(RootsResponse { roots }))
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
    Query(query): Query<DiffQuery>,
) -> ApiResult<RootsResponse<GitDiffRoot>> {
    let roots = selected_roots(app.gcx.clone(), query.root).await?;
    let path = query.path;
    collect_roots(roots, move |root| {
        diff_for_repo(root, path.as_deref(), query.staged)
    })
    .await
}

pub async fn handle_v1_git_log(
    State(app): State<AppState>,
    Query(query): Query<LogQuery>,
) -> ApiResult<RootsResponse<GitLogRoot>> {
    let roots = selected_roots(app.gcx.clone(), query.root).await?;
    collect_roots(roots, move |root| {
        log_for_repo(root, query.limit, query.skip)
    })
    .await
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
    tokio::task::spawn_blocking(move || {
        let repository =
            Repository::open(root).map_err(|error| format!("Failed to open repo: {}", error))?;
        unstage_changes(&repository, &paths).map(|unstaged| UnstageResponse { unstaged })
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

    fn commit_file(repo: &Repository, root: &Path, message: &str) {
        fs::write(root.join("file.txt"), "base\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("Test User", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
            .unwrap();
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
    fn status_shapes_staged_unstaged_and_untracked_files() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "Initial commit");
        fs::write(dir.path().join("file.txt"), "changed\n").unwrap();
        fs::write(dir.path().join("new.txt"), "new\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("new.txt")).unwrap();
        index.write().unwrap();
        fs::write(dir.path().join("untracked.txt"), "untracked\n").unwrap();

        let status = status_for_repo(dir.path()).unwrap();

        assert!(!status.head_detached);
        assert!(status.branch.is_some());
        assert!(status
            .staged
            .iter()
            .any(|change| { change.relative_path == "new.txt" && change.status == "ADDED" }));
        assert!(status
            .unstaged
            .iter()
            .any(|change| { change.relative_path == "file.txt" && change.status == "MODIFIED" }));
        assert!(status
            .unstaged
            .iter()
            .any(|change| { change.relative_path == "untracked.txt" && change.status == "ADDED" }));
    }

    #[test]
    fn diff_shapes_staged_and_unstaged_path_filtered_patches() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "Initial commit");
        fs::write(dir.path().join("file.txt"), "staged\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();
        fs::write(dir.path().join("file.txt"), "unstaged\n").unwrap();

        let staged = diff_for_repo(dir.path(), Some("file.txt"), true).unwrap();
        let unstaged = diff_for_repo(dir.path(), Some("file.txt"), false).unwrap();

        assert!(staged.patch.contains("+staged"));
        assert!(!staged.patch.contains("+unstaged"));
        assert!(unstaged.patch.contains("+unstaged"));
        assert!(!staged.truncated);
        assert!(!unstaged.truncated);
    }

    #[test]
    fn log_shapes_commit_identity_author_and_message() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "Subject line\n\nMessage body");

        let log = log_for_repo(dir.path(), 50, 0).unwrap();

        assert_eq!(log.commits.len(), 1);
        assert_eq!(log.commits[0].short_oid.len(), 12);
        assert_eq!(log.commits[0].author_name, "Test User");
        assert_eq!(log.commits[0].author_email, "test@example.com");
        assert_eq!(log.commits[0].message_first_line, "Subject line");
        assert_eq!(log.commits[0].message, "Subject line\n\nMessage body");
    }

    #[test]
    fn repo_relative_paths_rejects_absolute_and_parent_paths() {
        assert!(repo_relative_paths(vec!["/tmp/file".to_string()]).is_err());
        assert!(repo_relative_paths(vec!["../file".to_string()]).is_err());
        assert_eq!(
            repo_relative_paths(vec!["src/lib.rs".to_string()]).unwrap(),
            vec![PathBuf::from("src/lib.rs")]
        );
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

    #[test]
    fn effective_git_roots_from_discovers_enclosing_repo_for_subdir_workspace() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "Initial commit");
        let subdir = dir.path().join("refact-agent").join("engine");
        fs::create_dir_all(&subdir).unwrap();
        let canonical_repo = dunce::canonicalize(dir.path()).unwrap();

        let roots = effective_git_roots_from(Vec::new(), &[subdir]);

        assert_eq!(roots, vec![canonical_repo]);
    }

    #[test]
    fn effective_git_roots_from_keeps_single_root_when_workspace_is_repo_root() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "Initial commit");
        let canonical_repo = dunce::canonicalize(dir.path()).unwrap();

        let roots =
            effective_git_roots_from(vec![canonical_repo.clone()], &[dir.path().to_path_buf()]);

        assert_eq!(roots, vec![canonical_repo]);
    }

    #[test]
    fn effective_git_roots_from_skips_non_repo_and_bare_repo_folders() {
        let plain = tempfile::tempdir().unwrap();
        let bare = tempfile::tempdir().unwrap();
        Repository::init_bare(bare.path()).unwrap();

        let roots = effective_git_roots_from(
            Vec::new(),
            &[plain.path().to_path_buf(), bare.path().to_path_buf()],
        );

        assert_eq!(roots, Vec::<PathBuf>::new());
    }

    #[test]
    fn effective_git_roots_from_dedupes_covered_and_repeated_folders() {
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "Initial commit");
        let sub_a = dir.path().join("crates").join("a");
        let sub_b = dir.path().join("crates").join("b");
        fs::create_dir_all(&sub_a).unwrap();
        fs::create_dir_all(&sub_b).unwrap();
        let canonical_repo = dunce::canonicalize(dir.path()).unwrap();

        let discovered = effective_git_roots_from(Vec::new(), &[sub_a.clone(), sub_b.clone()]);
        assert_eq!(discovered, vec![canonical_repo.clone()]);

        let with_known_root = effective_git_roots_from(
            vec![canonical_repo.clone()],
            &[dir.path().to_path_buf(), sub_a, sub_b],
        );
        assert_eq!(with_known_root, vec![canonical_repo]);
    }

    #[tokio::test]
    async fn selected_roots_discovers_enclosing_repo_for_subdir_workspace_folder() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "Initial commit");
        let subdir = dir.path().join("refact-agent").join("engine");
        fs::create_dir_all(&subdir).unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![subdir.clone()];
        let canonical_repo = dunce::canonicalize(dir.path()).unwrap();

        let all_roots = selected_roots(gcx.clone(), None).await.unwrap();
        assert_eq!(all_roots, vec![canonical_repo.clone()]);

        let status = status_for_repo(&all_roots[0]).unwrap();
        assert!(status.branch.is_some());
        let log = log_for_repo(&all_roots[0], 10, 0).unwrap();
        assert_eq!(log.commits.len(), 1);
        let branches = branches_for_repo(&all_roots[0]).unwrap();
        assert!(branches.current.is_some());

        let mapped = selected_roots(gcx.clone(), Some(subdir.to_string_lossy().to_string()))
            .await
            .unwrap();
        assert_eq!(mapped, vec![canonical_repo]);

        let outside = tempfile::tempdir().unwrap();
        let error = selected_roots(gcx, Some(outside.path().to_string_lossy().to_string()))
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn selected_roots_filters_by_canonical_active_root() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let active = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() =
            vec![active.path().to_path_buf()];

        let selected = selected_roots(
            gcx.clone(),
            Some(active.path().to_string_lossy().to_string()),
        )
        .await
        .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0], dunce::canonicalize(active.path()).unwrap());

        let error = selected_roots(gcx, Some(other.path().to_string_lossy().to_string()))
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::NOT_FOUND);
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
    async fn git_reads_skip_missing_roots_and_serve_valid_repo() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (dir, repo) = init_repo();
        commit_file(&repo, dir.path(), "Initial commit");
        let missing = dir.path().join("missing-root");
        *gcx.documents_state.workspace_folders.lock().unwrap() = Vec::new();
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() =
            vec![missing, dir.path().to_path_buf()];
        let canonical_root = dunce::canonicalize(dir.path())
            .unwrap()
            .to_string_lossy()
            .to_string();
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let router = crate::http::routers::make_refact_http_server(app);

        let (status, payload) = get_json(&router, "/v1/git/status").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["roots"].as_array().unwrap().len(), 1);
        assert_eq!(payload["roots"][0]["root"], canonical_root);
        assert!(payload["roots"][0]["branch"].is_string());

        let (status, payload) = get_json(&router, "/v1/git/branches").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["roots"].as_array().unwrap().len(), 1);
        assert_eq!(payload["roots"][0]["root"], canonical_root);
        assert!(payload["roots"][0]["current"].is_string());

        let (status, payload) = get_json(&router, "/v1/git/log").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["roots"].as_array().unwrap().len(), 1);
        assert_eq!(payload["roots"][0]["root"], canonical_root);
        assert_eq!(payload["roots"][0]["commits"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn git_read_explicit_missing_root_returns_not_found() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let holder = tempfile::tempdir().unwrap();
        let missing = holder.path().join("missing-root");
        *gcx.documents_state.workspace_folders.lock().unwrap() = Vec::new();
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() = vec![missing.clone()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let router = crate::http::routers::make_refact_http_server(app);
        let uri = format!("/v1/git/status?root={}", missing.to_string_lossy());

        let (status, payload) = get_json(&router, &uri).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(payload["code"], "not_found");
        assert_eq!(payload["error"], "Git root not found");
    }

    #[tokio::test]
    async fn git_reads_return_empty_roots_when_all_configured_roots_are_missing() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let holder = tempfile::tempdir().unwrap();
        let missing = holder.path().join("missing-root");
        *gcx.documents_state.workspace_folders.lock().unwrap() = Vec::new();
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() = vec![missing];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let router = crate::http::routers::make_refact_http_server(app);

        for uri in ["/v1/git/status", "/v1/git/branches", "/v1/git/log"] {
            let (status, payload) = get_json(&router, uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(payload, json!({ "roots": [] }), "{uri}");
        }
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
