use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::extract::{Query, RawQuery, State};
use axum::response::Json;
use git2::Repository;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) use refact_git::read::{repo_relative_paths, resolve_commit_ref, validate_rev_input};
use refact_git::read::{
    branches_for_repo, commit_details_for_repo, diff_for_repo, log_for_repo, status_for_repo,
    validate_diff_paths, DiffBase, GitBranchesRoot, GitCommitDetails, GitDiffRoot, GitLogRoot,
    GitReadError, GitStatusRoot, LogFilters,
};

use crate::app_state::AppState;
use crate::git::operations::{stage_changes, unstage_changes};
use crate::git::{FileChange, FileChangeStatus};
use crate::global_context::GlobalContext;

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

#[derive(Debug, Deserialize)]
pub struct GitCommitDetailsQuery {
    #[serde(default)]
    pub root: Option<String>,
    pub oid: String,
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

type HttpError = (StatusCode, Json<Value>);

fn read_error(error: GitReadError) -> HttpError {
    match error {
        GitReadError::BadRequest(message) => api_error(StatusCode::BAD_REQUEST, message),
        GitReadError::NotFound(message) => api_error(StatusCode::NOT_FOUND, message),
        GitReadError::Internal(message) => internal_error(message),
    }
}

fn log_filters(query: &LogQuery) -> Result<LogFilters, HttpError> {
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
        validate_rev_input(reference).map_err(|error| api_error(StatusCode::BAD_REQUEST, error))?;
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
    Ok(LogFilters {
        scope_all,
        reference,
        text: non_empty(&query.text).map(|text| text.to_lowercase()),
        author: non_empty(&query.author).map(|author| author.to_lowercase()),
        path,
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
    let filters = log_filters(&query)?;
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
    .map_err(read_error)
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
        .map_err(read_error)
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
