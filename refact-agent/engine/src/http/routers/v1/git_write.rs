use std::ffi::OsString;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use axum::extract::{Query, State};
use axum::response::Json;
use git2::{Index, Repository, RepositoryState};
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::git_read::{api_error, api_error_code, internal_error, repo_relative_paths, selected_roots};
use crate::app_state::AppState;
use crate::git::cli::{
    git_args, pathspec_nul_input, repo_mutation_lock, run_git_cli, GitCliError, GitCliOptions,
    GitCliOutput,
};

type HttpError = (StatusCode, Json<Value>);
type ApiResult<T> = Result<Json<T>, HttpError>;

const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const BINARY_PROBE_BYTES: usize = 8000;
const HOOK_OUTPUT_TAIL_BYTES: usize = 4096;
const COMMIT_HOOKS: [&str; 4] = [
    "pre-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
];

#[derive(Debug, Deserialize)]
pub struct GitCommitV2Request {
    pub root: String,
    pub message: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub amend: bool,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct SkippedPath {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct GitCommitV2Response {
    pub oid: String,
    pub short_oid: String,
    pub summary: String,
    pub used_git_cli: bool,
    pub hooks_ran: bool,
    pub skipped: Vec<SkippedPath>,
}

#[derive(Debug, Deserialize)]
pub struct GitRollbackRequest {
    pub root: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub delete_added_files: bool,
    #[serde(default)]
    pub delete_untracked: bool,
}

#[derive(Debug, Serialize, Default)]
pub struct GitRollbackResponse {
    pub restored: Vec<String>,
    pub unstaged: Vec<String>,
    pub deleted: Vec<String>,
    pub skipped: Vec<SkippedPath>,
}

#[derive(Debug, Deserialize)]
pub struct GitFileQuery {
    #[serde(default)]
    pub root: Option<String>,
    pub path: String,
    #[serde(default)]
    pub rev: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GitFileResponse {
    pub root: String,
    pub path: String,
    pub rev: String,
    pub exists: bool,
    pub binary: bool,
    pub truncated: bool,
    pub size: u64,
    pub content: String,
}

fn bad_request(message: impl Into<String>) -> HttpError {
    api_error(StatusCode::BAD_REQUEST, message)
}

fn git2_error(context: &str) -> impl Fn(git2::Error) -> HttpError + '_ {
    move |error| internal_error(format!("{}: {}", context, error))
}

fn cli_error(error: GitCliError) -> HttpError {
    match error {
        GitCliError::Unavailable(_) => {
            api_error(StatusCode::SERVICE_UNAVAILABLE, error.to_string())
        }
        GitCliError::Timeout(_) => api_error(StatusCode::GATEWAY_TIMEOUT, error.to_string()),
        GitCliError::Io(_) => internal_error(error.to_string()),
    }
}

fn open_repo(root: &Path) -> Result<Repository, HttpError> {
    Repository::open(root).map_err(git2_error("Failed to open repo"))
}

async fn resolve_root(app: &AppState, requested: String) -> Result<PathBuf, HttpError> {
    selected_roots(app.gcx.clone(), Some(requested))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "Git root is not an active workspace root",
            )
        })
}

async fn run_blocking<T, F>(operation: F) -> Result<T, HttpError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, HttpError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| internal_error(format!("Git operation task failed: {}", error)))?
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn skipped(path: &Path, reason: &str) -> SkippedPath {
    SkippedPath {
        path: slash_path(path),
        reason: reason.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Containment {
    Missing,
    File,
    Symlink,
    Directory,
}

fn contained_leaf(root: &Path, relative: &Path) -> Result<Containment, &'static str> {
    let components = relative
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect::<Vec<_>>();
    let Some((leaf, parents)) = components.split_last() else {
        return Err("invalid_path");
    };
    let mut cursor = root.to_path_buf();
    for component in parents.iter().chain(std::iter::once(leaf)) {
        match component {
            Component::Normal(name) if name.eq_ignore_ascii_case(".git") => return Err("git_dir"),
            Component::Normal(_) => {}
            _ => return Err("invalid_path"),
        }
    }
    for component in parents {
        cursor.push(component.as_os_str());
        match std::fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err("symlink_parent"),
            Ok(metadata) if !metadata.is_dir() => return Err("not_directory_parent"),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Containment::Missing)
            }
            Err(_) => return Err("unreadable_parent"),
        }
    }
    let canonical_root = dunce::canonicalize(root).map_err(|_| "outside_root")?;
    let canonical_parent = dunce::canonicalize(&cursor).map_err(|_| "outside_root")?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err("outside_root");
    }
    cursor.push(leaf.as_os_str());
    Ok(match std::fs::symlink_metadata(&cursor) {
        Ok(metadata) if metadata.file_type().is_symlink() => Containment::Symlink,
        Ok(metadata) if metadata.is_dir() => Containment::Directory,
        Ok(_) => Containment::File,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Containment::Missing,
        Err(_) => return Err("unreadable"),
    })
}

fn safe_remove_file(root: &Path, relative: &Path) -> Result<(), &'static str> {
    match contained_leaf(root, relative)? {
        Containment::Missing => Err("not_found"),
        Containment::Directory => Err("directory"),
        Containment::File | Containment::Symlink => {
            std::fs::remove_file(root.join(relative)).map_err(|_| "delete_failed")
        }
    }
}

fn hook_is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn commit_hooks_present(repository: &Repository, root: &Path) -> bool {
    let configured = repository
        .config()
        .ok()
        .and_then(|config| config.get_path("core.hooksPath").ok());
    let hooks_dir = match configured {
        Some(path) if path.is_absolute() => path,
        Some(path) => root.join(path),
        None => repository.commondir().join("hooks"),
    };
    COMMIT_HOOKS
        .iter()
        .any(|hook| hook_is_executable(&hooks_dir.join(hook)))
}

fn gpg_sign_enabled(repository: &Repository) -> bool {
    repository
        .config()
        .ok()
        .and_then(|config| config.get_bool("commit.gpgsign").ok())
        .unwrap_or(false)
}

fn conflict_stage_present(index: &Index, path: &Path) -> bool {
    (1..=3).any(|stage| index.get_path(path, stage).is_some())
}

async fn run_with_paths(
    root: &Path,
    args: &[&str],
    paths: &[String],
) -> Result<GitCliOutput, HttpError> {
    let mut full = git_args(args.iter().copied());
    full.push(OsString::from("--pathspec-from-file=-"));
    full.push(OsString::from("--pathspec-file-nul"));
    run_git_cli(root, &full, GitCliOptions::with_paths(paths))
        .await
        .map_err(cli_error)
}

fn cli_failure(output: &GitCliOutput, hooks: bool, action: &str) -> HttpError {
    let tail = output.combined_tail(HOOK_OUTPUT_TAIL_BYTES);
    if hooks {
        api_error_code(
            StatusCode::UNPROCESSABLE_ENTITY,
            "hook_failed",
            format!("{} rejected by git hooks: {}", action, tail),
        )
    } else {
        internal_error(format!("{} failed: {}", action, tail))
    }
}

fn file_mode_from_raw(mode: u32) -> git2::FileMode {
    match mode {
        0o100755 => git2::FileMode::BlobExecutable,
        0o120000 => git2::FileMode::Link,
        0o160000 => git2::FileMode::Commit,
        0o040000 => git2::FileMode::Tree,
        _ => git2::FileMode::Blob,
    }
}

fn head_commit(repository: &Repository) -> Result<Option<git2::Commit<'_>>, HttpError> {
    match repository.head() {
        Ok(head) => head
            .peel_to_commit()
            .map(Some)
            .map_err(git2_error("Failed to resolve HEAD commit")),
        Err(error)
            if matches!(
                error.code(),
                git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(internal_error(format!("Failed to resolve HEAD: {}", error))),
    }
}

fn require_signature(repository: &Repository) -> Result<git2::Signature<'static>, HttpError> {
    repository.signature().map_err(|_| {
        bad_request("Git user.name and user.email must be configured before committing")
    })
}

fn summary_of(message: &str) -> String {
    message
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn commit_response(
    repository: &Repository,
    used_git_cli: bool,
    hooks_ran: bool,
    skipped: Vec<SkippedPath>,
) -> Result<GitCommitV2Response, HttpError> {
    let commit =
        head_commit(repository)?.ok_or_else(|| internal_error("HEAD is missing after commit"))?;
    let oid = commit.id().to_string();
    Ok(GitCommitV2Response {
        short_oid: oid.chars().take(12).collect(),
        oid,
        summary: summary_of(commit.message().unwrap_or_default()),
        used_git_cli,
        hooks_ran,
        skipped,
    })
}

struct CommitPlan {
    paths: Vec<PathBuf>,
    untracked: Vec<PathBuf>,
    skipped: Vec<SkippedPath>,
}

fn plan_commit_paths(
    repository: &Repository,
    root: &Path,
    paths: Vec<PathBuf>,
) -> Result<CommitPlan, HttpError> {
    let index = repository
        .index()
        .map_err(git2_error("Failed to read index"))?;
    let head_tree = head_commit(repository)?
        .map(|commit| commit.tree())
        .transpose()
        .map_err(git2_error("Failed to read HEAD tree"))?;
    let mut plan = CommitPlan {
        paths: Vec::new(),
        untracked: Vec::new(),
        skipped: Vec::new(),
    };
    for path in paths {
        if conflict_stage_present(&index, &path) {
            return Err(api_error_code(
                StatusCode::CONFLICT,
                "conflict",
                format!("{} has unresolved conflicts", slash_path(&path)),
            ));
        }
        let in_index = index.get_path(&path, 0).is_some();
        let in_head = head_tree
            .as_ref()
            .is_some_and(|tree| tree.get_path(&path).is_ok());
        match contained_leaf(root, &path) {
            Err(reason) => plan.skipped.push(skipped(&path, reason)),
            Ok(Containment::Directory) => plan.skipped.push(skipped(&path, "directory")),
            Ok(Containment::Missing) if !in_index && !in_head => {
                plan.skipped.push(skipped(&path, "not_found"))
            }
            Ok(Containment::Missing) => plan.paths.push(path),
            Ok(Containment::File | Containment::Symlink) => {
                if !in_index {
                    plan.untracked.push(path.clone());
                }
                plan.paths.push(path);
            }
        }
    }
    Ok(plan)
}

struct PreparedCommit<'repo> {
    index: Index,
    tree: git2::Tree<'repo>,
    unchanged: bool,
}

fn prepare_commit_tree<'repo>(
    repository: &'repo Repository,
    root: &Path,
    head: Option<&git2::Commit<'repo>>,
    paths: &[PathBuf],
) -> Result<PreparedCommit<'repo>, HttpError> {
    let mut index = repository
        .index()
        .map_err(git2_error("Failed to read index"))?;
    for path in paths {
        let result = if root.join(path).symlink_metadata().is_ok() {
            index.add_path(path)
        } else {
            index.remove_path(path)
        };
        result.map_err(|error| {
            internal_error(format!("Failed to update {}: {}", slash_path(path), error))
        })?;
    }
    let base_tree = head
        .map(|commit| commit.tree())
        .transpose()
        .map_err(git2_error("Failed to read HEAD tree"))?;
    let mut commit_index = Index::new().map_err(git2_error("Failed to create index"))?;
    if let Some(tree) = &base_tree {
        commit_index
            .read_tree(tree)
            .map_err(git2_error("Failed to load HEAD tree"))?;
    }
    for path in paths {
        match index.get_path(path, 0) {
            Some(entry) => commit_index
                .add(&entry)
                .map_err(git2_error("Failed to stage commit entry"))?,
            None => {
                let _ = commit_index.remove_path(path);
            }
        }
    }
    let tree_id = commit_index
        .write_tree_to(repository)
        .map_err(git2_error("Failed to write commit tree"))?;
    let unchanged = match &base_tree {
        Some(tree) => tree.id() == tree_id,
        None => commit_index.is_empty(),
    };
    let tree = repository
        .find_tree(tree_id)
        .map_err(git2_error("Failed to find commit tree"))?;
    Ok(PreparedCommit {
        index,
        tree,
        unchanged,
    })
}

fn nothing_to_commit() -> HttpError {
    api_error_code(
        StatusCode::CONFLICT,
        "nothing_to_commit",
        "No changes to commit for the selected paths",
    )
}

fn commit_with_git2(
    repository: &Repository,
    root: &Path,
    message: &str,
    plan: CommitPlan,
    amend: bool,
) -> Result<GitCommitV2Response, HttpError> {
    let signature = require_signature(repository)?;
    let head = head_commit(repository)?;
    let PreparedCommit {
        mut index,
        tree,
        unchanged,
    } = prepare_commit_tree(repository, root, head.as_ref(), &plan.paths)?;
    if unchanged && !amend {
        return Err(nothing_to_commit());
    }
    match (&head, amend) {
        (Some(commit), true) => commit
            .amend(
                Some("HEAD"),
                None,
                Some(&signature),
                None,
                Some(message),
                Some(&tree),
            )
            .map_err(git2_error("Failed to amend commit"))?,
        (Some(commit), false) => repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &[commit],
            )
            .map_err(git2_error("Failed to create commit"))?,
        (None, _) => repository
            .commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
            .map_err(git2_error("Failed to create commit"))?,
    };
    index.write().map_err(git2_error("Failed to write index"))?;
    commit_response(repository, false, false, plan.skipped)
}

fn paths_as_strings(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|path| slash_path(path)).collect()
}

async fn commit_with_cli(
    root: &Path,
    message: String,
    plan: CommitPlan,
    amend: bool,
    hooks: bool,
) -> Result<GitCommitV2Response, HttpError> {
    let untracked = paths_as_strings(&plan.untracked);
    if !untracked.is_empty() {
        let output = run_with_paths(root, &["add"], &untracked).await?;
        if !output.success {
            return Err(cli_failure(&output, false, "git add"));
        }
    }
    let pathspec = tempfile::NamedTempFile::new()
        .map_err(|error| internal_error(format!("Failed to create pathspec file: {}", error)))?;
    std::fs::write(
        pathspec.path(),
        pathspec_nul_input(&paths_as_strings(&plan.paths)),
    )
    .map_err(|error| internal_error(format!("Failed to write pathspec file: {}", error)))?;
    let mut args = git_args(["commit", "--only"]);
    if amend {
        args.push("--amend".into());
    }
    args.push("-F".into());
    args.push("-".into());
    if !plan.paths.is_empty() {
        let mut from_file = OsString::from("--pathspec-from-file=");
        from_file.push(pathspec.path());
        args.push(from_file);
        args.push("--pathspec-file-nul".into());
    }
    let options = GitCliOptions {
        stdin: Some(message.into_bytes()),
        literal_pathspecs: true,
        ..GitCliOptions::default()
    };
    let result = run_git_cli(root, &args, options).await;
    let failure = match result {
        Ok(output) if output.success => None,
        Ok(output) => Some(cli_failure(&output, hooks, "git commit")),
        Err(error) => Some(cli_error(error)),
    };
    if let Some(failure) = failure {
        if !untracked.is_empty() {
            let _ = run_with_paths(root, &["reset", "-q"], &untracked).await;
        }
        return Err(failure);
    }
    let root = root.to_path_buf();
    run_blocking(move || commit_response(&open_repo(&root)?, true, hooks, plan.skipped)).await
}

async fn commit_in_progress_state(
    root: &Path,
    message: String,
    plan: CommitPlan,
    hooks: bool,
) -> Result<GitCommitV2Response, HttpError> {
    let paths = paths_as_strings(&plan.paths);
    if !paths.is_empty() {
        let output = run_with_paths(root, &["add", "-A"], &paths).await?;
        if !output.success {
            return Err(cli_failure(&output, false, "git add"));
        }
    }
    let options = GitCliOptions {
        stdin: Some(message.into_bytes()),
        ..GitCliOptions::default()
    };
    let output = run_git_cli(root, &git_args(["commit", "-F", "-"]), options)
        .await
        .map_err(cli_error)?;
    if !output.success {
        return Err(cli_failure(&output, hooks, "git commit"));
    }
    let root = root.to_path_buf();
    run_blocking(move || commit_response(&open_repo(&root)?, true, hooks, plan.skipped)).await
}

enum CommitRoute {
    Done(GitCommitV2Response),
    Cli { plan: CommitPlan, hooks: bool },
    InProgress { plan: CommitPlan, hooks: bool },
}

pub async fn handle_v1_git_commit_v2(
    State(app): State<AppState>,
    Json(request): Json<GitCommitV2Request>,
) -> ApiResult<GitCommitV2Response> {
    let message = request.message.trim().to_string();
    if message.is_empty() {
        return Err(bad_request("Commit message must not be empty"));
    }
    let message = git2::message_prettify(message.as_str(), None)
        .map_err(|error| bad_request(format!("Invalid commit message: {}", error)))?;
    if request.paths.is_empty() && !request.amend {
        return Err(bad_request("Select at least one path to commit"));
    }
    let paths = repo_relative_paths(request.paths).map_err(bad_request)?;
    let root = resolve_root(&app, request.root).await?;
    let amend = request.amend;
    let lock = repo_mutation_lock(&root);
    let _guard = lock.lock().await;

    let blocking_root = root.clone();
    let blocking_paths = paths.clone();
    let blocking_message = message.clone();
    let route = run_blocking(move || {
        let root = blocking_root;
        let repository = open_repo(&root)?;
        let hooks = commit_hooks_present(&repository, &root);
        if repository.state() != RepositoryState::Clean {
            let index = repository
                .index()
                .map_err(git2_error("Failed to read index"))?;
            if index.has_conflicts() {
                return Err(api_error_code(
                    StatusCode::CONFLICT,
                    "conflict",
                    "Resolve all conflicts before committing",
                ));
            }
            if amend {
                return Err(bad_request(
                    "Cannot amend while an operation is in progress",
                ));
            }
            require_signature(&repository)?;
            let plan = plan_commit_paths(&repository, &root, blocking_paths)?;
            if plan.paths.is_empty() {
                return Err(bad_request("None of the selected paths can be committed"));
            }
            return Ok(CommitRoute::InProgress { plan, hooks });
        }
        require_signature(&repository)?;
        if amend && head_commit(&repository)?.is_none() {
            return Err(bad_request(
                "Cannot amend: the repository has no commits yet",
            ));
        }
        let plan = plan_commit_paths(&repository, &root, blocking_paths)?;
        if hooks || gpg_sign_enabled(&repository) {
            let head = head_commit(&repository)?;
            let prepared = prepare_commit_tree(&repository, &root, head.as_ref(), &plan.paths)?;
            if prepared.unchanged && !amend {
                return Err(nothing_to_commit());
            }
            return Ok(CommitRoute::Cli { plan, hooks });
        }
        commit_with_git2(&repository, &root, &blocking_message, plan, amend).map(CommitRoute::Done)
    })
    .await?;
    match route {
        CommitRoute::Done(response) => Ok(Json(response)),
        CommitRoute::Cli { plan, hooks } => commit_with_cli(&root, message, plan, amend, hooks)
            .await
            .map(Json),
        CommitRoute::InProgress { plan, hooks } => {
            commit_in_progress_state(&root, message, plan, hooks)
                .await
                .map(Json)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RollbackKind {
    Restore,
    Added,
    Untracked,
}

fn classify_rollback(
    repository: &Repository,
    root: &Path,
    path: &Path,
    head_tree: Option<&git2::Tree<'_>>,
    index: &Index,
) -> Result<RollbackKind, &'static str> {
    if conflict_stage_present(index, path) {
        return Err("conflicted");
    }
    let leaf = contained_leaf(root, path)?;
    let in_head = head_tree.is_some_and(|tree| {
        tree.get_path(path)
            .is_ok_and(|entry| entry.kind() != Some(git2::ObjectType::Tree))
    });
    let in_index = index.get_path(path, 0).is_some();
    if in_head {
        let status = repository
            .status_file(path)
            .unwrap_or(git2::Status::CURRENT);
        return if status.is_empty() {
            Err("unchanged")
        } else {
            Ok(RollbackKind::Restore)
        };
    }
    if in_index {
        return Ok(RollbackKind::Added);
    }
    match leaf {
        Containment::Directory => Err("directory"),
        Containment::Missing => Err("unchanged"),
        Containment::File | Containment::Symlink => {
            if repository.status_should_ignore(path).unwrap_or(false) {
                Err("unchanged")
            } else {
                Ok(RollbackKind::Untracked)
            }
        }
    }
}

pub async fn handle_v1_git_rollback(
    State(app): State<AppState>,
    Json(request): Json<GitRollbackRequest>,
) -> ApiResult<GitRollbackResponse> {
    if request.paths.is_empty() {
        return Err(bad_request("Select at least one path to roll back"));
    }
    let paths = repo_relative_paths(request.paths).map_err(bad_request)?;
    let root = resolve_root(&app, request.root).await?;
    let lock = repo_mutation_lock(&root);
    let _guard = lock.lock().await;

    let classify_root = root.clone();
    let (response, restore, added, untracked) = run_blocking(move || {
        let root = classify_root;
        let repository = open_repo(&root)?;
        let index = repository
            .index()
            .map_err(git2_error("Failed to read index"))?;
        let head_tree = head_commit(&repository)?
            .map(|commit| commit.tree())
            .transpose()
            .map_err(git2_error("Failed to read HEAD tree"))?;
        let mut response = GitRollbackResponse::default();
        let (mut restore, mut added, mut untracked) = (Vec::new(), Vec::new(), Vec::new());
        for path in paths {
            match classify_rollback(&repository, &root, &path, head_tree.as_ref(), &index) {
                Ok(RollbackKind::Restore) => restore.push(path),
                Ok(RollbackKind::Added) => added.push(path),
                Ok(RollbackKind::Untracked) => untracked.push(path),
                Err(reason) => response.skipped.push(skipped(&path, reason)),
            }
        }
        Ok((response, restore, added, untracked))
    })
    .await?;
    let mut response = response;

    if !restore.is_empty() {
        let restore_paths = paths_as_strings(&restore);
        let output = run_with_paths(
            &root,
            &["restore", "--source=HEAD", "--staged", "--worktree"],
            &restore_paths,
        )
        .await?;
        if !output.success {
            return Err(cli_failure(&output, false, "git restore"));
        }
        response.restored = restore_paths;
    }

    let delete_added = request.delete_added_files;
    let delete_untracked = request.delete_untracked;
    let finish_root = root.clone();
    run_blocking(move || {
        let root = finish_root;
        if !added.is_empty() {
            let repository = open_repo(&root)?;
            let mut index = repository
                .index()
                .map_err(git2_error("Failed to read index"))?;
            for path in &added {
                index
                    .remove_path(path)
                    .map_err(git2_error("Failed to unstage path"))?;
            }
            index.write().map_err(git2_error("Failed to write index"))?;
            for path in &added {
                response.unstaged.push(slash_path(path));
                if delete_added {
                    match safe_remove_file(&root, path) {
                        Ok(()) => response.deleted.push(slash_path(path)),
                        Err("not_found") => {}
                        Err(reason) => response.skipped.push(skipped(path, reason)),
                    }
                }
            }
        }
        for path in &untracked {
            if !delete_untracked {
                response.skipped.push(skipped(path, "untracked"));
                continue;
            }
            match safe_remove_file(&root, path) {
                Ok(()) => response.deleted.push(slash_path(path)),
                Err(reason) => response.skipped.push(skipped(path, reason)),
            }
        }
        Ok(Json(response))
    })
    .await
}

fn file_payload(bytes: Vec<u8>, size: u64) -> (bool, bool, String) {
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

fn blob_payload(repository: &Repository, id: git2::Oid) -> Result<(u64, Vec<u8>), HttpError> {
    let blob = repository
        .find_blob(id)
        .map_err(git2_error("Failed to read blob"))?;
    let content = blob.content();
    Ok((
        blob.size() as u64,
        content[..content.len().min(MAX_FILE_BYTES + 1)].to_vec(),
    ))
}

fn tree_blob(
    repository: &Repository,
    tree: &git2::Tree<'_>,
    path: &Path,
) -> Result<Option<(u64, Vec<u8>)>, HttpError> {
    match tree.get_path(path) {
        Ok(entry) if entry.kind() == Some(git2::ObjectType::Blob) => {
            blob_payload(repository, entry.id()).map(Some)
        }
        _ => Ok(None),
    }
}

fn read_revision_file(
    root: &Path,
    path: &Path,
    rev: &str,
) -> Result<Option<(u64, Vec<u8>)>, HttpError> {
    let repository = open_repo(root)?;
    match rev {
        "WORKTREE" => match contained_leaf(root, path).map_err(bad_request)? {
            Containment::Missing => Ok(None),
            Containment::Directory => Err(bad_request("Path is a directory")),
            Containment::Symlink => {
                let target = std::fs::read_link(root.join(path))
                    .map_err(|error| internal_error(format!("Failed to read link: {}", error)))?;
                let bytes = slash_path(&target).into_bytes();
                Ok(Some((bytes.len() as u64, bytes)))
            }
            Containment::File => {
                let file = std::fs::File::open(root.join(path))
                    .map_err(|error| internal_error(format!("Failed to open file: {}", error)))?;
                let size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
                let mut bytes = Vec::new();
                file.take(MAX_FILE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|error| internal_error(format!("Failed to read file: {}", error)))?;
                Ok(Some((size, bytes)))
            }
        },
        "INDEX" => {
            let index = repository
                .index()
                .map_err(git2_error("Failed to read index"))?;
            match index.get_path(path, 0) {
                Some(entry) if file_mode_from_raw(entry.mode) != git2::FileMode::Commit => {
                    blob_payload(&repository, entry.id).map(Some)
                }
                _ => Ok(None),
            }
        }
        "HEAD" => match head_commit(&repository)? {
            Some(commit) => {
                let tree = commit.tree().map_err(git2_error("Failed to read tree"))?;
                tree_blob(&repository, &tree, path)
            }
            None => Ok(None),
        },
        rev => {
            let commit = repository
                .revparse_single(rev)
                .and_then(|object| object.peel_to_commit())
                .map_err(|_| api_error(StatusCode::NOT_FOUND, "Revision not found"))?;
            let tree = commit.tree().map_err(git2_error("Failed to read tree"))?;
            tree_blob(&repository, &tree, path)
        }
    }
}

pub async fn handle_v1_git_file(
    State(app): State<AppState>,
    Query(query): Query<GitFileQuery>,
) -> ApiResult<GitFileResponse> {
    let path = repo_relative_paths(vec![query.path])
        .map_err(bad_request)?
        .pop()
        .ok_or_else(|| bad_request("Path must not be empty"))?;
    let rev = query
        .rev
        .map(|rev| rev.trim().to_string())
        .filter(|rev| !rev.is_empty())
        .unwrap_or_else(|| "HEAD".to_string());
    if rev.starts_with('-') {
        return Err(bad_request("Invalid revision"));
    }
    let root = match query.root {
        Some(root) => resolve_root(&app, root).await?,
        None => selected_roots(app.gcx.clone(), None)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "No active git root"))?,
    };
    run_blocking(move || {
        let payload = read_revision_file(&root, &path, &rev)?;
        let (exists, size, (binary, truncated, content)) = match payload {
            Some((size, bytes)) => (true, size, file_payload(bytes, size)),
            None => (false, 0, (false, false, String::new())),
        };
        Ok(Json(GitFileResponse {
            root: root.to_string_lossy().to_string(),
            path: slash_path(&path),
            rev,
            exists,
            binary,
            truncated,
            size,
            content,
        }))
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::fs;

    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo(root: &Path) -> Repository {
        let repository = Repository::init(root).unwrap();
        let mut config = repository.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        config.set_bool("commit.gpgsign", false).unwrap();
        config.set_str("core.hooksPath", ".git/hooks").unwrap();
        repository
    }

    fn commit_all(repository: &Repository, files: &[(&str, &str)], message: &str) -> git2::Oid {
        let root = repository.workdir().unwrap().to_path_buf();
        let mut index = repository.index().unwrap();
        for (path, content) in files {
            let absolute = root.join(path);
            fs::create_dir_all(absolute.parent().unwrap()).unwrap();
            fs::write(&absolute, content).unwrap();
            index.add_path(Path::new(path)).unwrap();
        }
        index.write().unwrap();
        let tree = repository.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = repository.signature().unwrap();
        let parents = head_commit(repository)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>();
        let parents = parents.iter().collect::<Vec<_>>();
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .unwrap()
    }

    async fn router_for(root: &Path) -> axum::Router {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() = vec![root.to_path_buf()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        crate::http::routers::make_refact_http_server(app)
    }

    async fn send(
        router: &axum::Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(body) => {
                request = request.header("content-type", "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let response = router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    fn tree_text(repository: &Repository, commit: &git2::Commit<'_>, path: &str) -> Option<String> {
        let entry = commit.tree().unwrap().get_path(Path::new(path)).ok()?;
        let blob = repository.find_blob(entry.id()).unwrap();
        Some(String::from_utf8(blob.content().to_vec()).unwrap())
    }

    fn index_text(repository: &Repository, path: &str) -> Option<String> {
        let mut index = repository.index().unwrap();
        index.read(true).unwrap();
        let entry = index.get_path(Path::new(path), 0)?;
        let blob = repository.find_blob(entry.id).unwrap();
        Some(String::from_utf8(blob.content().to_vec()).unwrap())
    }

    #[tokio::test]
    async fn git_commit_only_commits_listed_worktree_paths_and_keeps_other_staged_entries() {
        let unborn = tempfile::tempdir().unwrap();
        let unborn_repo = init_repo(unborn.path());
        fs::write(unborn.path().join("first.txt"), "first\n").unwrap();
        fs::write(unborn.path().join("later.txt"), "later\n").unwrap();
        let router = router_for(unborn.path()).await;
        let root = unborn.path().to_string_lossy().to_string();
        let (status, body) = send(
            &router,
            "POST",
            "/v1/git/commit",
            Some(json!({"root": root, "message": "  first commit\n", "paths": ["first.txt"]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["used_git_cli"], false);
        let first = head_commit(&unborn_repo).unwrap().unwrap();
        assert_eq!(first.parent_count(), 0);
        assert_eq!(body["oid"], first.id().to_string());
        assert_eq!(
            tree_text(&unborn_repo, &first, "first.txt").as_deref(),
            Some("first\n")
        );
        assert_eq!(tree_text(&unborn_repo, &first, "later.txt"), None);

        let dir = tempfile::tempdir().unwrap();
        let repository = init_repo(dir.path());
        let base = commit_all(
            &repository,
            &[
                ("keep.txt", "keep v1\n"),
                ("mod.txt", "mod v1\n"),
                ("del.txt", "del\n"),
            ],
            "base",
        );
        fs::write(dir.path().join("keep.txt"), "keep staged\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("keep.txt")).unwrap();
        index.write().unwrap();
        fs::write(dir.path().join("mod.txt"), "mod v2\n").unwrap();
        fs::remove_file(dir.path().join("del.txt")).unwrap();
        fs::write(dir.path().join("new.txt"), "brand new\n").unwrap();
        assert_eq!(
            index_text(&repository, "keep.txt").as_deref(),
            Some("keep staged\n")
        );

        let router = router_for(dir.path()).await;
        let root = dir.path().to_string_lossy().to_string();
        let (status, body) = send(
            &router,
            "POST",
            "/v1/git/commit",
            Some(json!({"root": root, "message": "partial", "paths": ["mod.txt", "del.txt", "new.txt"]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let partial = head_commit(&repository).unwrap().unwrap();
        assert_eq!(partial.parent_ids().collect::<Vec<_>>(), vec![base]);
        assert_eq!(
            tree_text(&repository, &partial, "keep.txt").as_deref(),
            Some("keep v1\n")
        );
        assert_eq!(
            tree_text(&repository, &partial, "mod.txt").as_deref(),
            Some("mod v2\n")
        );
        assert_eq!(
            tree_text(&repository, &partial, "new.txt").as_deref(),
            Some("brand new\n")
        );
        assert_eq!(tree_text(&repository, &partial, "del.txt"), None);
        assert_eq!(
            index_text(&repository, "keep.txt").as_deref(),
            Some("keep staged\n")
        );
        assert_eq!(
            index_text(&repository, "new.txt").as_deref(),
            Some("brand new\n")
        );
        assert_eq!(index_text(&repository, "del.txt"), None);

        let (status, body) = send(
            &router,
            "POST",
            "/v1/git/commit",
            Some(json!({"root": root, "message": "again", "paths": ["mod.txt"]})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "nothing_to_commit");
        assert_eq!(
            head_commit(&repository).unwrap().unwrap().id(),
            partial.id()
        );

        let (status, body) = send(
            &router,
            "POST",
            "/v1/git/commit",
            Some(json!({"root": root, "message": "amended message", "paths": [], "amend": true})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let amended = head_commit(&repository).unwrap().unwrap();
        assert_ne!(amended.id(), partial.id());
        assert_eq!(amended.parent_ids().collect::<Vec<_>>(), vec![base]);
        assert_eq!(amended.tree_id(), partial.tree_id());
        assert_eq!(body["summary"], "amended message");
        assert_eq!(
            index_text(&repository, "keep.txt").as_deref(),
            Some("keep staged\n")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn git_rollback_restores_unstages_and_never_deletes_through_symlinked_parent() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim.txt");
        fs::write(&victim, "outside\n").unwrap();
        let repository = init_repo(dir.path());
        commit_all(
            &repository,
            &[("tracked.txt", "head\n"), ("dir/inner.txt", "inner\n")],
            "base",
        );
        fs::write(dir.path().join("tracked.txt"), "staged edit\n").unwrap();
        fs::write(dir.path().join("added.txt"), "added\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.add_path(Path::new("added.txt")).unwrap();
        index.write().unwrap();
        fs::write(dir.path().join("tracked.txt"), "worktree edit\n").unwrap();
        fs::write(dir.path().join("untracked.txt"), "scratch\n").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        assert!(victim.exists());

        let router = router_for(dir.path()).await;
        let root = dir.path().to_string_lossy().to_string();
        let (status, body) = send(
            &router,
            "POST",
            "/v1/git/rollback",
            Some(json!({"root": root, "paths": ["../escape.txt"], "delete_untracked": true})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

        let (status, body) = send(
            &router,
            "POST",
            "/v1/git/rollback",
            Some(json!({"root": root, "paths": ["added.txt", "untracked.txt"]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["unstaged"], json!(["added.txt"]));
        assert_eq!(body["deleted"], json!([]));
        assert_eq!(
            body["skipped"],
            json!([{"path": "untracked.txt", "reason": "untracked"}])
        );
        assert!(dir.path().join("added.txt").exists());
        assert!(dir.path().join("untracked.txt").exists());
        assert_eq!(index_text(&repository, "added.txt"), None);
        let mut index = repository.index().unwrap();
        index.read(true).unwrap();
        index.add_path(Path::new("added.txt")).unwrap();
        index.write().unwrap();

        let (status, body) = send(
            &router,
            "POST",
            "/v1/git/rollback",
            Some(json!({
                "root": root,
                "paths": ["tracked.txt", "added.txt", "untracked.txt", "link/victim.txt", "dir", "dir/inner.txt"],
                "delete_added_files": true,
                "delete_untracked": true,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["restored"], json!(["tracked.txt"]));
        assert_eq!(body["unstaged"], json!(["added.txt"]));
        assert_eq!(body["deleted"], json!(["added.txt", "untracked.txt"]));
        let mut skipped = body["skipped"].as_array().unwrap().clone();
        skipped.sort_by_key(|entry| entry["path"].as_str().unwrap().to_string());
        assert_eq!(
            Value::Array(skipped),
            json!([
                {"path": "dir", "reason": "directory"},
                {"path": "dir/inner.txt", "reason": "unchanged"},
                {"path": "link/victim.txt", "reason": "symlink_parent"},
            ])
        );
        assert_eq!(fs::read_to_string(&victim).unwrap(), "outside\n");
        assert!(dir.path().join("link").symlink_metadata().is_ok());
        assert_eq!(
            fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
            "head\n"
        );
        assert_eq!(
            index_text(&repository, "tracked.txt").as_deref(),
            Some("head\n")
        );
        assert_eq!(index_text(&repository, "added.txt"), None);
        assert!(!dir.path().join("added.txt").exists());
        assert!(!dir.path().join("untracked.txt").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("dir/inner.txt")).unwrap(),
            "inner\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn git_commit_with_hooks_uses_cli_and_failed_hook_leaves_head_and_index_unchanged() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let repository = init_repo(dir.path());
        let base = commit_all(&repository, &[("keep.txt", "keep v1\n")], "base");
        fs::write(dir.path().join("keep.txt"), "keep staged\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("keep.txt")).unwrap();
        index.write().unwrap();
        fs::write(dir.path().join("fresh.txt"), "fresh\n").unwrap();
        let hook = dir.path().join(".git/hooks/pre-commit");
        fs::create_dir_all(hook.parent().unwrap()).unwrap();
        fs::write(
            &hook,
            "#!/bin/sh\necho 'rejected by policy https://alice:hunter2pass@example.com/repo.git' >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();

        let router = router_for(dir.path()).await;
        let root = dir.path().to_string_lossy().to_string();
        let request = json!({"root": root, "message": "with hooks", "paths": ["fresh.txt"]});
        let (status, body) = send(&router, "POST", "/v1/git/commit", Some(request.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["code"], "hook_failed");
        let error = body["error"].as_str().unwrap();
        assert!(error.contains("rejected by policy"), "{error}");
        assert!(!error.contains("hunter2pass"), "{error}");
        assert_eq!(head_commit(&repository).unwrap().unwrap().id(), base);
        assert_eq!(index_text(&repository, "fresh.txt"), None);
        assert_eq!(
            index_text(&repository, "keep.txt").as_deref(),
            Some("keep staged\n")
        );

        let replacement = hook.with_extension("next");
        fs::write(&replacement, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o755)).unwrap();
        fs::rename(&replacement, &hook).unwrap();
        let (status, body) = send(&router, "POST", "/v1/git/commit", Some(request)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["used_git_cli"], true);
        assert_eq!(body["hooks_ran"], true);
        let commit = head_commit(&repository).unwrap().unwrap();
        assert_eq!(commit.parent_ids().collect::<Vec<_>>(), vec![base]);
        assert_eq!(
            tree_text(&repository, &commit, "fresh.txt").as_deref(),
            Some("fresh\n")
        );
        assert_eq!(
            tree_text(&repository, &commit, "keep.txt").as_deref(),
            Some("keep v1\n")
        );
        assert_eq!(
            index_text(&repository, "keep.txt").as_deref(),
            Some("keep staged\n")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn git_file_revisions_and_head_diff_use_literal_repeated_paths() {
        let dir = tempfile::tempdir().unwrap();
        let repository = init_repo(dir.path());
        let first = commit_all(
            &repository,
            &[("a*.txt", "star v1\n"), ("ab.txt", "ab v1\n")],
            "one",
        );
        commit_all(&repository, &[("a*.txt", "star v2\n")], "two");
        fs::write(dir.path().join("a*.txt"), "star index\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("a*.txt")).unwrap();
        index.write().unwrap();
        fs::write(dir.path().join("a*.txt"), "star worktree\n").unwrap();
        fs::write(dir.path().join("ab.txt"), "ab worktree\n").unwrap();
        fs::write(dir.path().join("new.txt"), "untracked body\n").unwrap();
        fs::write(dir.path().join("blob.bin"), b"\x00\x01binary").unwrap();

        let router = router_for(dir.path()).await;
        let root = dir.path().to_string_lossy().to_string();
        let encoded_root =
            url::form_urlencoded::byte_serialize(root.as_bytes()).collect::<String>();
        let file = |rev: &str, path: &str| {
            format!(
                "/v1/git/file?root={}&path={}&rev={}",
                encoded_root,
                url::form_urlencoded::byte_serialize(path.as_bytes()).collect::<String>(),
                rev
            )
        };
        for (rev, expected) in [
            ("HEAD", "star v2\n"),
            ("INDEX", "star index\n"),
            ("WORKTREE", "star worktree\n"),
            (first.to_string().as_str(), "star v1\n"),
        ] {
            let (status, body) = send(&router, "GET", &file(rev, "a*.txt"), None).await;
            assert_eq!(status, StatusCode::OK, "{rev}: {body}");
            assert_eq!(body["exists"], true, "{rev}");
            assert_eq!(body["content"], expected, "{rev}");
        }
        let (status, body) = send(&router, "GET", &file("HEAD", "missing.txt"), None).await;
        assert_eq!(
            (status, body["exists"].clone()),
            (StatusCode::OK, json!(false))
        );
        let (status, body) = send(&router, "GET", &file("WORKTREE", "blob.bin"), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            (body["binary"].clone(), body["content"].clone()),
            (json!(true), json!(""))
        );
        let (status, body) = send(&router, "GET", &file("no-such-rev", "a*.txt"), None).await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::NOT_FOUND, json!("not_found"))
        );

        let uri = format!(
            "/v1/git/diff?root={}&against=head&path=a%2A.txt&path=new.txt",
            encoded_root
        );
        let (status, body) = send(&router, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let patch = body["roots"][0]["patch"].as_str().unwrap();
        assert!(patch.contains("-star v2\n+star worktree\n"), "{patch}");
        assert!(patch.contains("+untracked body\n"), "{patch}");
        assert!(!patch.contains("ab worktree"), "{patch}");
        assert!(!patch.contains("star index"), "{patch}");
    }

    #[tokio::test]
    async fn git_status_reports_changes_untracked_conflicts_and_unchanged_legacy_lists() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let repository = init_repo(root);
        commit_all(
            &repository,
            &[
                ("conflict.txt", "base\n"),
                ("staged.txt", "s1\n"),
                ("gone.txt", "g1\n"),
                ("old.txt", "rename me\n"),
            ],
            "base",
        );
        git(root, &["checkout", "-q", "-b", "other"]);
        commit_all(&repository, &[("conflict.txt", "theirs\n")], "theirs");
        git(root, &["checkout", "-q", "-"]);
        commit_all(&repository, &[("conflict.txt", "ours\n")], "ours");
        let merge = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["merge", "-q", "--no-edit", "other"])
            .output()
            .unwrap();
        assert!(!merge.status.success());
        fs::write(root.join("staged.txt"), "s2\n").unwrap();
        git(root, &["add", "staged.txt"]);
        fs::remove_file(root.join("gone.txt")).unwrap();
        git(root, &["mv", "old.txt", "renamed.txt"]);
        fs::write(root.join("both.txt"), "b1\n").unwrap();
        git(root, &["add", "both.txt"]);
        fs::write(root.join("both.txt"), "b2\n").unwrap();
        fs::create_dir_all(root.join("u")).unwrap();
        fs::write(root.join("u/x.txt"), "x\n").unwrap();

        let router = router_for(root).await;
        let encoded_root = url::form_urlencoded::byte_serialize(root.to_string_lossy().as_bytes())
            .collect::<String>();
        let (status, body) = send(
            &router,
            "GET",
            &format!("/v1/git/status?root={}", encoded_root),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let status_root = &body["roots"][0];
        let canonical = dunce::canonicalize(root).unwrap();
        let abs = |path: &str| canonical.join(path).to_string_lossy().to_string();
        let legacy = |key: &str| {
            status_root[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| {
                    assert_eq!(
                        entry["absolute_path"],
                        abs(entry["relative_path"].as_str().unwrap())
                    );
                    (
                        entry["relative_path"].as_str().unwrap().to_string(),
                        entry["status"].as_str().unwrap().to_string(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let pairs = |items: &[(&str, &str)]| {
            items
                .iter()
                .map(|(path, status)| (path.to_string(), status.to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            legacy("staged"),
            pairs(&[
                ("both.txt", "ADDED"),
                ("old.txt", "DELETED"),
                ("renamed.txt", "ADDED"),
                ("staged.txt", "MODIFIED"),
            ])
        );
        assert_eq!(
            legacy("unstaged"),
            pairs(&[
                ("both.txt", "MODIFIED"),
                ("gone.txt", "DELETED"),
                ("u/x.txt", "ADDED")
            ])
        );
        assert_eq!(status_root["untracked_included"], true);
        assert_eq!(status_root["repo_state"], "merge");
        assert_eq!(status_root["truncated"], false);
        assert_eq!(
            status_root["head_oid"],
            head_commit(&repository).unwrap().unwrap().id().to_string()
        );
        let mut changes = status_root["changes"].as_array().unwrap().clone();
        changes.sort_by_key(|entry| entry["relative_path"].as_str().unwrap().to_string());
        let change = |path: &str, old: Option<&str>, status: &str, staged: bool, unstaged: bool| {
            json!({
                "relative_path": path,
                "absolute_path": abs(path),
                "old_path": old,
                "status": status,
                "staged": staged,
                "unstaged": unstaged,
            })
        };
        assert_eq!(
            Value::Array(changes),
            json!([
                change("both.txt", None, "ADDED", true, true),
                change("gone.txt", None, "DELETED", false, true),
                change("renamed.txt", Some("old.txt"), "RENAMED", true, false),
                change("staged.txt", None, "MODIFIED", true, false),
            ])
        );
        assert_eq!(
            status_root["untracked"],
            json!([{"relative_path": "u/x.txt", "absolute_path": abs("u/x.txt")}])
        );
        assert_eq!(
            status_root["conflicted"],
            json!([{
                "relative_path": "conflict.txt",
                "absolute_path": abs("conflict.txt"),
                "ours": true,
                "theirs": true,
                "ancestor": true,
            }])
        );
    }
}
