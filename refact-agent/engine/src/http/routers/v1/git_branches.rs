//! Branch and remote endpoints (checkout/create/rename/delete, fetch/pull/push).
//!
//! Every user-supplied name/ref is validated with git2 before reaching the git CLI and is
//! never placed where git could parse it as an option. Revisions, tags and start points are
//! resolved to commit oids first and the oid hex is what the CLI receives. Mutating handlers
//! hold `repo_mutation_lock(root)` for their whole duration.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use axum::extract::State;
use axum::response::Json;
use git2::{Repository, RepositoryState};
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::git_read::{
    api_error, api_error_code, internal_error, resolve_commit_ref, validate_rev_input,
};
use super::git_write::{cli_error, open_repo, resolve_root, run_blocking};
use crate::app_state::AppState;
use crate::git::cli::{
    git_args, repo_mutation_lock, run_git_cli, GitCliOptions, GitCliOutput, NETWORK_GIT_CLI_TIMEOUT,
};

type HttpError = (StatusCode, Json<Value>);
type ApiResult<T> = Result<Json<T>, HttpError>;

const OUTPUT_TAIL_BYTES: usize = 4096;

#[derive(Debug, Deserialize)]
pub struct GitCheckoutRequest {
    pub root: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct GitCheckoutResponse {
    pub branch: Option<String>,
    pub head_oid: Option<String>,
    pub detached: bool,
}

#[derive(Debug, Deserialize)]
pub struct GitBranchCreateRequest {
    pub root: String,
    pub name: String,
    #[serde(default)]
    pub start_point: Option<String>,
    #[serde(default)]
    pub checkout: bool,
}

#[derive(Debug, Serialize)]
pub struct GitBranchCreateResponse {
    pub name: String,
    pub tip_oid: String,
    pub checked_out: bool,
}

#[derive(Debug, Deserialize)]
pub struct GitBranchRenameRequest {
    pub root: String,
    pub old_name: String,
    pub new_name: String,
}

#[derive(Debug, Serialize)]
pub struct GitBranchRenameResponse {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct GitBranchDeleteRequest {
    pub root: String,
    pub name: String,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct GitBranchDeleteResponse {
    pub deleted: String,
}

#[derive(Debug, Deserialize)]
pub struct GitFetchRequest {
    pub root: String,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub prune: bool,
}

#[derive(Debug, Deserialize)]
pub struct GitPullRequest {
    pub root: String,
}

#[derive(Debug, Deserialize)]
pub struct GitPushRequest {
    pub root: String,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub set_upstream: bool,
    #[serde(default)]
    pub force_with_lease: bool,
}

#[derive(Debug, Serialize)]
pub struct GitRemoteOpResponse {
    pub ok: bool,
    pub summary: String,
    pub output: String,
}

fn bad_request(message: impl Into<String>) -> HttpError {
    api_error(StatusCode::BAD_REQUEST, message)
}

fn conflict(code: &str, message: impl Into<String>) -> HttpError {
    api_error_code(StatusCode::CONFLICT, code, message)
}

fn git2_failure(context: &str, error: git2::Error) -> HttpError {
    internal_error(format!("{}: {}", context, error))
}

fn validate_local_branch_name(name: &str) -> Result<(), HttpError> {
    if name.is_empty() {
        return Err(bad_request("Branch name must not be empty"));
    }
    if name.starts_with('-') {
        return Err(bad_request("Branch name must not start with '-'"));
    }
    if name == "HEAD" || !git2::Branch::name_is_valid(name).unwrap_or(false) {
        return Err(bad_request(format!("Invalid branch name: {}", name)));
    }
    Ok(())
}

fn validate_remote_branch_name(name: &str) -> Result<(), HttpError> {
    if name.is_empty() || name.starts_with('-') {
        return Err(bad_request(
            "Remote branch name must be non-empty and not start with '-'",
        ));
    }
    if !name.contains('/') || !git2::Reference::is_valid_name(&format!("refs/remotes/{}", name)) {
        return Err(bad_request(format!("Invalid remote branch name: {}", name)));
    }
    Ok(())
}

fn validate_remote_name(repository: &Repository, name: &str) -> Result<(), HttpError> {
    if name.is_empty() || name.starts_with('-') || !git2::Remote::is_valid_name(name) {
        return Err(bad_request(format!("Invalid remote name: {}", name)));
    }
    repository
        .find_remote(name)
        .map(|_| ())
        .map_err(|_| api_error(StatusCode::NOT_FOUND, format!("Remote not found: {}", name)))
}

fn remote_names(repository: &Repository) -> Vec<String> {
    let mut names = repository
        .remotes()
        .map(|remotes| {
            remotes
                .iter()
                .flatten()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn local_branch_exists(repository: &Repository, name: &str) -> bool {
    repository
        .find_branch(name, git2::BranchType::Local)
        .is_ok()
}

fn upstream_ref(repository: &Repository, branch: &str) -> Option<String> {
    repository
        .branch_upstream_name(&format!("refs/heads/{}", branch))
        .ok()
        .and_then(|buf| buf.as_str().map(ToString::to_string))
}

fn current_branch(repository: &Repository) -> Option<String> {
    let head = repository.head().ok()?;
    if !head.is_branch() {
        return None;
    }
    head.shorthand().map(ToString::to_string)
}

fn head_state(repository: &Repository) -> GitCheckoutResponse {
    let detached = repository.head_detached().unwrap_or(false);
    GitCheckoutResponse {
        branch: if detached {
            None
        } else {
            current_branch(repository)
        },
        head_oid: repository
            .head()
            .and_then(|head| head.peel_to_commit())
            .ok()
            .map(|commit| commit.id().to_string()),
        detached,
    }
}

fn output_tail(output: &GitCliOutput) -> String {
    output.combined_tail(OUTPUT_TAIL_BYTES)
}

fn is_checkout_conflict(text: &str) -> bool {
    text.contains("would be overwritten by")
        || text.contains("untracked working tree files would be")
        || text.contains("Please commit your changes or stash them")
}

fn local_cli_failure(output: &GitCliOutput, action: &str) -> HttpError {
    let tail = output_tail(output);
    if is_checkout_conflict(&tail) {
        return conflict(
            "checkout_conflict",
            format!("{} would overwrite local changes: {}", action, tail),
        );
    }
    internal_error(format!("{} failed: {}", action, tail))
}

fn is_auth_failure(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "authentication failed",
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        "permission denied (publickey",
        "host key verification failed",
        "invalid username or password",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn is_missing_upstream_ref(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("couldn't find remote ref") || lower.contains("no such ref was fetched")
}

fn remote_cli_failure(output: &GitCliOutput, action: &str) -> HttpError {
    let tail = output_tail(output);
    if is_missing_upstream_ref(&tail) {
        return conflict(
            "no_upstream",
            format!(
                "{} failed: the upstream branch no longer exists on the remote: {}",
                action, tail
            ),
        );
    }
    if is_auth_failure(&tail) {
        // 502, not 401: this is the remote's credential failure, which must never be confused
        // with the engine/daemon session auth by proxies or clients.
        return api_error_code(
            StatusCode::BAD_GATEWAY,
            "auth_failed",
            format!(
                "{} failed: authentication required or rejected: {}",
                action, tail
            ),
        );
    }
    api_error_code(
        StatusCode::BAD_GATEWAY,
        "remote_error",
        format!("{} failed: {}", action, tail),
    )
}

async fn run_local(root: &Path, args: Vec<OsString>) -> Result<GitCliOutput, HttpError> {
    run_git_cli(root, &args, GitCliOptions::default())
        .await
        .map_err(cli_error)
}

async fn run_network(root: &Path, args: Vec<OsString>) -> Result<GitCliOutput, HttpError> {
    let options = GitCliOptions {
        timeout: NETWORK_GIT_CLI_TIMEOUT,
        ..GitCliOptions::default()
    };
    run_git_cli(root, &args, options).await.map_err(cli_error)
}

fn require_clean_state(repository: &Repository, action: &str) -> Result<(), HttpError> {
    if repository.state() != RepositoryState::Clean {
        return Err(conflict(
            "conflict",
            format!(
                "Finish or abort the in-progress operation before {}",
                action
            ),
        ));
    }
    Ok(())
}

fn read_head(root: &Path) -> Result<GitCheckoutResponse, HttpError> {
    Ok(head_state(&open_repo(root)?))
}

enum CheckoutPlan {
    SwitchLocal(String),
    TrackRemote { local: String, remote_ref: String },
    Detach(String),
}

fn plan_checkout(root: &Path, name: &str, kind: &str) -> Result<CheckoutPlan, HttpError> {
    let repository = open_repo(root)?;
    require_clean_state(&repository, "checking out")?;
    match kind {
        "local" => {
            validate_local_branch_name(name)?;
            if !local_branch_exists(&repository, name) {
                return Err(api_error(
                    StatusCode::NOT_FOUND,
                    format!("Local branch not found: {}", name),
                ));
            }
            Ok(CheckoutPlan::SwitchLocal(name.to_string()))
        }
        "remote" => {
            validate_remote_branch_name(name)?;
            let full = format!("refs/remotes/{}", name);
            let reference = repository.find_reference(&full).map_err(|_| {
                api_error(
                    StatusCode::NOT_FOUND,
                    format!("Remote branch not found: {}", name),
                )
            })?;
            if reference.kind() != Some(git2::ReferenceType::Direct) {
                return Err(bad_request(format!(
                    "{} is a symbolic reference and cannot be checked out",
                    name
                )));
            }
            let remote = repository
                .branch_remote_name(&full)
                .ok()
                .and_then(|buf| buf.as_str().map(ToString::to_string))
                .ok_or_else(|| bad_request(format!("Cannot determine remote of {}", name)))?;
            let local = name
                .strip_prefix(&format!("{}/", remote))
                .filter(|local| !local.is_empty())
                .ok_or_else(|| bad_request(format!("Invalid remote branch name: {}", name)))?
                .to_string();
            validate_local_branch_name(&local)?;
            if local_branch_exists(&repository, &local) {
                if upstream_ref(&repository, &local).as_deref() == Some(full.as_str()) {
                    return Ok(CheckoutPlan::SwitchLocal(local));
                }
                return Err(conflict(
                    "already_exists",
                    format!(
                        "Local branch {} already exists and does not track {}",
                        local, name
                    ),
                ));
            }
            Ok(CheckoutPlan::TrackRemote {
                local,
                remote_ref: full,
            })
        }
        "tag" => {
            if name.is_empty() || name.starts_with('-') || !git2::Tag::is_valid_name(name) {
                return Err(bad_request(format!("Invalid tag name: {}", name)));
            }
            let commit = repository
                .find_reference(&format!("refs/tags/{}", name))
                .and_then(|reference| reference.peel_to_commit())
                .map_err(|_| {
                    api_error(StatusCode::NOT_FOUND, format!("Tag not found: {}", name))
                })?;
            Ok(CheckoutPlan::Detach(commit.id().to_string()))
        }
        "revision" => {
            validate_rev_input(name).map_err(bad_request)?;
            let commit = resolve_commit_ref(&repository, name).ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    format!("Revision not found: {}", name),
                )
            })?;
            Ok(CheckoutPlan::Detach(commit.id().to_string()))
        }
        other => Err(bad_request(format!("Unsupported checkout kind: {}", other))),
    }
}

pub async fn handle_v1_git_branch_checkout(
    State(app): State<AppState>,
    Json(request): Json<GitCheckoutRequest>,
) -> ApiResult<GitCheckoutResponse> {
    let root = resolve_root(&app, request.root).await?;
    let lock = repo_mutation_lock(&root);
    let _guard = lock.lock().await;
    let plan_root = root.clone();
    let (name, kind) = (request.name, request.kind);
    let plan = run_blocking(move || plan_checkout(&plan_root, &name, &kind)).await?;
    let args = match plan {
        CheckoutPlan::SwitchLocal(name) => git_args(["switch".into(), OsString::from(name)]),
        CheckoutPlan::TrackRemote { local, remote_ref } => git_args([
            OsString::from("switch"),
            "--create".into(),
            local.into(),
            "--track".into(),
            remote_ref.into(),
        ]),
        CheckoutPlan::Detach(oid) => {
            git_args(["switch".into(), "--detach".into(), OsString::from(oid)])
        }
    };
    let output = run_local(&root, args).await?;
    if !output.success {
        return Err(local_cli_failure(&output, "git switch"));
    }
    run_blocking(move || read_head(&root)).await.map(Json)
}

pub async fn handle_v1_git_branch_create(
    State(app): State<AppState>,
    Json(request): Json<GitBranchCreateRequest>,
) -> ApiResult<GitBranchCreateResponse> {
    validate_local_branch_name(&request.name)?;
    let root = resolve_root(&app, request.root).await?;
    let lock = repo_mutation_lock(&root);
    let _guard = lock.lock().await;
    let name = request.name;
    let start_point = request.start_point.filter(|start| !start.trim().is_empty());
    let checkout = request.checkout;
    let plan_root = root.clone();
    let plan_name = name.clone();
    let tip = run_blocking(move || {
        let repository = open_repo(&plan_root)?;
        if checkout {
            require_clean_state(&repository, "checking out")?;
        }
        if local_branch_exists(&repository, &plan_name) {
            return Err(conflict(
                "already_exists",
                format!("Branch {} already exists", plan_name),
            ));
        }
        let commit = match start_point {
            Some(start) => {
                validate_rev_input(start.trim()).map_err(bad_request)?;
                resolve_commit_ref(&repository, start.trim()).ok_or_else(|| {
                    api_error(
                        StatusCode::NOT_FOUND,
                        format!("Start point not found: {}", start.trim()),
                    )
                })?
            }
            None => repository
                .head()
                .and_then(|head| head.peel_to_commit())
                .map_err(|_| bad_request("HEAD has no commit to branch from"))?,
        };
        let tip = commit.id();
        if !checkout {
            repository
                .branch(&plan_name, &commit, false)
                .map_err(|error| git2_failure("Failed to create branch", error))?;
        }
        Ok(tip.to_string())
    })
    .await?;
    if checkout {
        let output = run_local(
            &root,
            git_args([
                OsString::from("switch"),
                "--create".into(),
                name.clone().into(),
                tip.clone().into(),
            ]),
        )
        .await?;
        if !output.success {
            return Err(local_cli_failure(&output, "git switch --create"));
        }
    }
    Ok(Json(GitBranchCreateResponse {
        name,
        tip_oid: tip,
        checked_out: checkout,
    }))
}

pub async fn handle_v1_git_branch_rename(
    State(app): State<AppState>,
    Json(request): Json<GitBranchRenameRequest>,
) -> ApiResult<GitBranchRenameResponse> {
    validate_local_branch_name(&request.old_name)?;
    validate_local_branch_name(&request.new_name)?;
    let root = resolve_root(&app, request.root).await?;
    let lock = repo_mutation_lock(&root);
    let _guard = lock.lock().await;
    let (old_name, new_name) = (request.old_name, request.new_name);
    let check_root = root.clone();
    let (check_old, check_new) = (old_name.clone(), new_name.clone());
    run_blocking(move || {
        let repository = open_repo(&check_root)?;
        if !local_branch_exists(&repository, &check_old) {
            return Err(api_error(
                StatusCode::NOT_FOUND,
                format!("Local branch not found: {}", check_old),
            ));
        }
        if check_old != check_new && local_branch_exists(&repository, &check_new) {
            return Err(conflict(
                "already_exists",
                format!("Branch {} already exists", check_new),
            ));
        }
        Ok(())
    })
    .await?;
    if old_name == new_name {
        return Ok(Json(GitBranchRenameResponse { name: new_name }));
    }
    // `git branch -m` (unlike git2's rename) also rewrites HEAD of every linked worktree that
    // has the branch checked out, so no worktree is left on a dangling branch.
    let output = run_local(
        &root,
        git_args([
            OsString::from("branch"),
            "-m".into(),
            "--".into(),
            old_name.into(),
            new_name.clone().into(),
        ]),
    )
    .await?;
    if !output.success {
        let tail = output_tail(&output);
        if tail.contains("already exists") {
            return Err(conflict(
                "already_exists",
                format!("Branch {} already exists: {}", new_name, tail),
            ));
        }
        return Err(internal_error(format!("git branch -m failed: {}", tail)));
    }
    Ok(Json(GitBranchRenameResponse { name: new_name }))
}

pub async fn handle_v1_git_branch_delete(
    State(app): State<AppState>,
    Json(request): Json<GitBranchDeleteRequest>,
) -> ApiResult<GitBranchDeleteResponse> {
    validate_local_branch_name(&request.name)?;
    let root = resolve_root(&app, request.root).await?;
    let lock = repo_mutation_lock(&root);
    let _guard = lock.lock().await;
    let name = request.name;
    let check_root = root.clone();
    let check_name = name.clone();
    run_blocking(move || {
        let repository = open_repo(&check_root)?;
        let branch = repository
            .find_branch(&check_name, git2::BranchType::Local)
            .map_err(|_| {
                api_error(
                    StatusCode::NOT_FOUND,
                    format!("Local branch not found: {}", check_name),
                )
            })?;
        if branch.is_head() {
            return Err(conflict(
                "current_branch",
                format!("Cannot delete the current branch {}", check_name),
            ));
        }
        Ok(())
    })
    .await?;
    let flag = if request.force { "-D" } else { "-d" };
    let output = run_local(
        &root,
        git_args([OsString::from("branch"), flag.into(), name.clone().into()]),
    )
    .await?;
    if !output.success {
        let tail = output_tail(&output);
        if tail.contains("not fully merged") {
            return Err(conflict(
                "not_merged",
                format!("Branch {} is not fully merged: {}", name, tail),
            ));
        }
        if tail.contains("checked out at") || tail.contains("used by worktree") {
            return Err(conflict(
                "current_branch",
                format!("Branch {} is checked out in a worktree: {}", name, tail),
            ));
        }
        return Err(internal_error(format!(
            "git branch {} failed: {}",
            flag, tail
        )));
    }
    Ok(Json(GitBranchDeleteResponse { deleted: name }))
}

pub async fn handle_v1_git_fetch(
    State(app): State<AppState>,
    Json(request): Json<GitFetchRequest>,
) -> ApiResult<GitRemoteOpResponse> {
    let root = resolve_root(&app, request.root).await?;
    let lock = repo_mutation_lock(&root);
    let _guard = lock.lock().await;
    let remote = request.remote.filter(|remote| !remote.trim().is_empty());
    let check_root = root.clone();
    let check_remote = remote.clone();
    run_blocking(move || {
        let repository = open_repo(&check_root)?;
        match &check_remote {
            Some(remote) => validate_remote_name(&repository, remote),
            None if remote_names(&repository).is_empty() => {
                Err(bad_request("Repository has no remotes to fetch from"))
            }
            None => Ok(()),
        }
    })
    .await?;
    let mut args = git_args(["fetch", "--no-progress"]);
    if request.prune {
        args.push("--prune".into());
    }
    match &remote {
        Some(remote) => args.push(remote.into()),
        None => args.push("--all".into()),
    }
    let output = run_network(&root, args).await?;
    if !output.success {
        return Err(remote_cli_failure(&output, "git fetch"));
    }
    Ok(Json(GitRemoteOpResponse {
        ok: true,
        summary: match remote {
            Some(remote) => format!("Fetched {}", remote),
            None => "Fetched all remotes".to_string(),
        },
        output: output_tail(&output),
    }))
}

struct PullPlan {
    branch: String,
    before: Option<String>,
}

fn plan_pull(root: &Path) -> Result<PullPlan, HttpError> {
    let repository = open_repo(root)?;
    require_clean_state(&repository, "pulling")?;
    let branch = current_branch(&repository)
        .filter(|_| !repository.head_detached().unwrap_or(false))
        .ok_or_else(|| {
            conflict(
                "no_upstream",
                "HEAD is detached; check out a branch to pull",
            )
        })?;
    if upstream_ref(&repository, &branch).is_none() {
        return Err(conflict(
            "no_upstream",
            format!("Branch {} has no upstream configured", branch),
        ));
    }
    Ok(PullPlan {
        before: head_state(&repository).head_oid,
        branch,
    })
}

pub async fn handle_v1_git_pull(
    State(app): State<AppState>,
    Json(request): Json<GitPullRequest>,
) -> ApiResult<GitRemoteOpResponse> {
    let root = resolve_root(&app, request.root).await?;
    let lock = repo_mutation_lock(&root);
    let _guard = lock.lock().await;
    let plan_root = root.clone();
    let plan = run_blocking(move || plan_pull(&plan_root)).await?;
    let output = run_network(
        &root,
        git_args(["pull", "--no-rebase", "--ff-only", "--no-progress"]),
    )
    .await?;
    if !output.success {
        let tail = output_tail(&output);
        if tail.contains("Not possible to fast-forward")
            || tail.contains("Diverging branches")
            || tail.contains("not possible to fast-forward")
        {
            return Err(conflict(
                "not_fast_forward",
                format!(
                    "{} and its upstream have diverged; fast-forward is not possible: {}",
                    plan.branch, tail
                ),
            ));
        }
        if is_checkout_conflict(&tail) {
            return Err(conflict(
                "checkout_conflict",
                format!("Pull would overwrite local changes: {}", tail),
            ));
        }
        return Err(remote_cli_failure(&output, "git pull"));
    }
    let after_root = root.clone();
    let after = run_blocking(move || read_head(&after_root)).await?.head_oid;
    let summary = if after == plan.before {
        format!("{} is already up to date", plan.branch)
    } else {
        format!(
            "Fast-forwarded {} to {}",
            plan.branch,
            after
                .as_deref()
                .unwrap_or_default()
                .chars()
                .take(12)
                .collect::<String>()
        )
    };
    Ok(Json(GitRemoteOpResponse {
        ok: true,
        summary,
        output: output_tail(&output),
    }))
}

struct PushPlan {
    remote: String,
    refspec: String,
    summary: String,
}

fn plan_push(
    root: &Path,
    remote: Option<String>,
    branch: Option<String>,
    set_upstream: bool,
) -> Result<PushPlan, HttpError> {
    let repository = open_repo(root)?;
    let branch = branch.filter(|branch| !branch.trim().is_empty());
    let explicit_branch = branch.is_some();
    let branch = match branch {
        Some(branch) => branch,
        None => current_branch(&repository)
            .filter(|_| !repository.head_detached().unwrap_or(false))
            .ok_or_else(|| bad_request("HEAD is detached; specify a branch to push"))?,
    };
    validate_local_branch_name(&branch)?;
    if !local_branch_exists(&repository, &branch) {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            format!("Local branch not found: {}", branch),
        ));
    }
    if let Some(remote) = &remote {
        validate_remote_name(&repository, remote)?;
    }
    let local_ref = format!("refs/heads/{}", branch);
    let upstream_remote = repository
        .branch_upstream_remote(&local_ref)
        .ok()
        .and_then(|buf| buf.as_str().map(ToString::to_string));
    let upstream_merge = repository
        .branch_upstream_merge(&local_ref)
        .ok()
        .and_then(|buf| buf.as_str().map(ToString::to_string));
    // The configured upstream remote reaches the CLI as an argument: validate it exactly like an
    // explicit remote. "." (a local upstream) or an unknown/invalid name has no remote to push to.
    let upstream_remote = match upstream_remote {
        Some(name) if validate_remote_name(&repository, &name).is_ok() => Some(name),
        Some(name) if !set_upstream && remote.as_ref().is_none_or(|remote| remote == &name) => {
            return Err(conflict(
                "no_upstream",
                format!(
                    "Branch {} tracks {:?}, which is not a pushable remote; push with set_upstream",
                    branch, name
                ),
            ))
        }
        _ => None,
    };
    let explicit_target = remote.is_some();
    let remotes = remote_names(&repository);
    let (remote, destination) = match (remote, upstream_remote, upstream_merge) {
        (None, Some(upstream_remote), Some(merge)) if !set_upstream => (upstream_remote, merge),
        (Some(remote), Some(upstream_remote), Some(merge))
            if remote == upstream_remote && !set_upstream =>
        {
            (remote, merge)
        }
        (remote, _, _) if set_upstream => {
            let remote = match remote {
                Some(remote) => remote,
                None if remotes.len() == 1 => remotes[0].clone(),
                None if remotes.iter().any(|name| name == "origin") => "origin".to_string(),
                None => {
                    return Err(bad_request(format!(
                        "Specify a remote to push to; available remotes: {}",
                        remotes.join(", ")
                    )))
                }
            };
            validate_remote_name(&repository, &remote)?;
            (remote, local_ref.clone())
        }
        (Some(remote), _, _) => {
            return Err(conflict(
                "no_upstream",
                format!(
                "Branch {} has no upstream on {}; push with set_upstream. Available remotes: {}",
                branch,
                remote,
                remotes.join(", ")
            ),
            ))
        }
        (None, _, _) => {
            return Err(conflict(
                "no_upstream",
                format!(
                    "Branch {} has no upstream; push with set_upstream. Available remotes: {}",
                    branch,
                    if remotes.is_empty() {
                        "(none)".to_string()
                    } else {
                        remotes.join(", ")
                    }
                ),
            ))
        }
    };
    if !git2::Reference::is_valid_name(&destination) || !destination.starts_with("refs/heads/") {
        return Err(bad_request(format!(
            "Invalid push destination: {}",
            destination
        )));
    }
    let destination_short = destination.trim_start_matches("refs/heads/").to_string();
    // push.default=simple: an implicit push to the upstream only proceeds when the upstream
    // branch has the same name as the local one.
    if !set_upstream && !explicit_target && !explicit_branch && destination_short != branch {
        return Err(conflict(
            "upstream_mismatch",
            format!(
                "Branch {} tracks {}/{}, whose name differs; specify the branch or push with set_upstream",
                branch, remote, destination_short
            ),
        ));
    }
    Ok(PushPlan {
        summary: format!("Pushed {} to {}/{}", branch, remote, destination_short),
        refspec: format!("{}:{}", local_ref, destination),
        remote,
    })
}

pub async fn handle_v1_git_push(
    State(app): State<AppState>,
    Json(request): Json<GitPushRequest>,
) -> ApiResult<GitRemoteOpResponse> {
    let root = resolve_root(&app, request.root).await?;
    let lock = repo_mutation_lock(&root);
    let _guard = lock.lock().await;
    let plan_root: PathBuf = root.clone();
    let (remote, branch, set_upstream) = (request.remote, request.branch, request.set_upstream);
    let plan = run_blocking(move || plan_push(&plan_root, remote, branch, set_upstream)).await?;
    let mut args = git_args(["push", "--porcelain", "--no-progress"]);
    if set_upstream {
        args.push("--set-upstream".into());
    }
    if request.force_with_lease {
        args.push("--force-with-lease".into());
    }
    args.push(plan.remote.clone().into());
    args.push(plan.refspec.clone().into());
    let output = run_network(&root, args).await?;
    if !output.success {
        let tail = output_tail(&output);
        if tail.contains("[rejected]")
            || tail.contains("[remote rejected]")
            || tail.contains("non-fast-forward")
            || tail.contains("fetch first")
            || tail.contains("stale info")
        {
            return Err(conflict(
                "rejected",
                format!("Push to {} was rejected: {}", plan.remote, tail),
            ));
        }
        return Err(remote_cli_failure(&output, "git push"));
    }
    Ok(Json(GitRemoteOpResponse {
        ok: true,
        summary: plan.summary,
        output: output_tail(&output),
    }))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use axum::body::Body;
    use axum::http::Request;
    use git2::Repository;
    use hyper::StatusCode;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    fn git_output(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "Alice Dev")
            .env("GIT_AUTHOR_EMAIL", "alice@example.com")
            .env("GIT_COMMITTER_NAME", "Alice Dev")
            .env("GIT_COMMITTER_EMAIL", "alice@example.com")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn configure(root: &Path) {
        for (key, value) in [
            ("user.name", "Alice Dev"),
            ("user.email", "alice@example.com"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", ".git/no-hooks"),
            ("core.autocrlf", "false"),
            ("pull.rebase", "false"),
            ("push.default", "simple"),
        ] {
            git_output(root, &["config", key, value]);
        }
    }

    fn init(root: &Path) {
        git_output(root, &["init", "-q", "-b", "main"]);
        configure(root);
    }

    fn commit_file(root: &Path, path: &str, content: &str, message: &str) -> String {
        let absolute = root.join(path);
        fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        fs::write(absolute, content).unwrap();
        git_output(root, &["add", "--", path]);
        git_output(root, &["commit", "-q", "-m", message]);
        git_output(root, &["rev-parse", "HEAD"])
    }

    fn rev(root: &Path, name: &str) -> String {
        git_output(root, &["rev-parse", "--verify", name])
    }

    fn ref_exists(root: &Path, name: &str) -> bool {
        Repository::open(root).unwrap().find_reference(name).is_ok()
    }

    async fn router_for(root: &Path) -> axum::Router {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_vcs_roots.lock().unwrap() = vec![root.to_path_buf()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        crate::http::routers::make_refact_http_server(app)
    }

    async fn send(router: &axum::Router, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let request = match body {
            Some(body) => Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())),
            None => Request::builder().uri(uri).body(Body::empty()),
        };
        let response = router.clone().oneshot(request.unwrap()).await.unwrap();
        let status = response.status();
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    fn encoded(root: &Path) -> String {
        url::form_urlencoded::byte_serialize(root.to_string_lossy().as_bytes()).collect()
    }

    fn oids(body: &Value) -> Vec<String> {
        body["roots"][0]["commits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|commit| commit["oid"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn git_log_graph_refs_filters_commit_details_and_branch_inventory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root);
        let c1 = commit_file(root, "a.txt", "a\n", "Initial import");
        fs::create_dir_all(root.join("dir")).unwrap();
        let c2 = commit_file(root, "dir/x.txt", "x\n", "Add directory file");
        git_output(root, &["checkout", "-q", "-b", "feature", &c1]);
        let c3 = commit_file(root, "feature.txt", "f\n", "Feature work");
        git_output(root, &["checkout", "-q", "main"]);
        git_output(
            root,
            &[
                "-c",
                "user.name=Bob Builder",
                "-c",
                "user.email=bob@corp.test",
                "commit",
                "-q",
                "--allow-empty",
                "--author=Bob Builder <bob@corp.test>",
                "-m",
                "Bob tweak",
            ],
        );
        let c4 = rev(root, "HEAD");
        git_output(root, &["tag", "-a", "v1", "-m", "release", &c1]);
        git_output(
            root,
            &["remote", "add", "origin", "/nonexistent/remote.git"],
        );
        git_output(root, &["update-ref", "refs/remotes/origin/main", &c2]);
        git_output(
            root,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );
        git_output(root, &["config", "branch.main.remote", "origin"]);
        git_output(root, &["config", "branch.main.merge", "refs/heads/main"]);
        git_output(root, &["config", "branch.feature.remote", "origin"]);
        git_output(root, &["config", "branch.feature.merge", "refs/heads/gone"]);
        assert_eq!(rev(root, "v1^{commit}"), c1);
        assert_ne!(rev(root, "refs/tags/v1"), c1);

        let router = router_for(root).await;
        let log = |query: &str| format!("/v1/git/log?root={}{}", encoded(root), query);

        let (status, body) = send(&router, &log(""), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(oids(&body), vec![c4.clone(), c2.clone(), c1.clone()]);
        let head_root = &body["roots"][0];
        assert_eq!(head_root["head_oid"], c4);
        assert_eq!(head_root["scan_truncated"], false);
        let commits = head_root["commits"].as_array().unwrap();
        assert_eq!(commits[0]["parents"], json!([c2]));
        assert_eq!(commits[2]["parents"], json!([]));
        assert_eq!(
            commits[0]["refs"],
            json!([{"name": "HEAD", "kind": "head"}, {"name": "main", "kind": "local"}])
        );
        assert_eq!(
            commits[1]["refs"],
            json!([{"name": "origin/main", "kind": "remote"}])
        );
        assert_eq!(commits[2]["refs"], json!([{"name": "v1", "kind": "tag"}]));

        let (_, body) = send(&router, &log("&scope=all"), None).await;
        let mut all = oids(&body);
        all.sort();
        let mut expected = vec![c1.clone(), c2.clone(), c3.clone(), c4.clone()];
        expected.sort();
        assert_eq!(all, expected);
        let feature_commit = body["roots"][0]["commits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|commit| commit["oid"] == c3)
            .unwrap()
            .clone();
        assert_eq!(
            feature_commit["refs"],
            json!([{"name": "feature", "kind": "local"}])
        );

        for (query, expected) in [
            ("&ref=feature", vec![c3.clone(), c1.clone()]),
            ("&ref=v1", vec![c1.clone()]),
            ("&ref=origin/main", vec![c2.clone(), c1.clone()]),
            ("&author=BOB@corp", vec![c4.clone()]),
            ("&text=directory", vec![c2.clone()]),
            (&format!("&text={}", &c1[..8]), vec![c1.clone()]),
            ("&path=dir", vec![c2.clone()]),
            ("&path=a.txt", vec![c1.clone()]),
            ("&scope=all&path=feature.txt", vec![c3.clone()]),
            ("&author=alice&skip=1&limit=1", vec![c1.clone()]),
            ("&author=alice&limit=1", vec![c2.clone()]),
        ] {
            let (status, body) = send(&router, &log(query), None).await;
            assert_eq!(status, StatusCode::OK, "{query}: {body}");
            assert_eq!(oids(&body), expected, "{query}");
        }
        let (status, body) = send(&router, &log("&ref=no-such-ref"), None).await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::NOT_FOUND, json!("not_found"))
        );
        let (status, _) = send(&router, &log("&ref=--all"), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let details = |oid: &str| format!("/v1/git/commit?root={}&oid={}", encoded(root), oid);
        let (status, body) = send(&router, &details(&c1), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["parents"], json!([]));
        assert_eq!(body["author_email"], "alice@example.com");
        assert_eq!(body["refs"], json!([{"name": "v1", "kind": "tag"}]));
        assert_eq!(body["branches"], json!(["feature", "main", "origin/main"]));
        assert_eq!(
            body["files"],
            json!([{"path": "a.txt", "old_path": null, "status": "ADDED", "additions": 1, "deletions": 0, "binary": false}])
        );
        assert_eq!(body["files_truncated"], false);
        let (_, body) = send(&router, &details(&c3), None).await;
        assert_eq!(body["branches"], json!(["feature"]));
        assert_eq!(body["files"][0]["path"], "feature.txt");
        let (status, body) = send(&router, &details(&"0".repeat(40)), None).await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::NOT_FOUND, json!("not_found"))
        );

        let (status, body) = send(
            &router,
            &format!("/v1/git/branches?root={}", encoded(root)),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let branches = &body["roots"][0];
        assert_eq!(branches["current"], "main");
        assert_eq!(branches["head_detached"], false);
        assert_eq!(branches["head_oid"], c4);
        assert_eq!(branches["remotes"], json!(["origin"]));
        assert_eq!(
            branches["remote_branches"],
            json!([{"name": "origin/main", "remote": "origin", "short_name": "main", "tip_oid": c2}])
        );
        assert_eq!(branches["tags"], json!([{"name": "v1", "target_oid": c1}]));
        assert_eq!(
            branches["branches"],
            json!([
                {"name": "feature", "is_head": false, "upstream": null, "tip_oid": c3, "ahead": null, "behind": null, "upstream_gone": true},
                {"name": "main", "is_head": true, "upstream": "origin/main", "tip_oid": c4, "ahead": 1, "behind": 0, "upstream_gone": false},
            ])
        );
    }

    #[tokio::test]
    async fn git_branch_ops_refuse_current_unmerged_and_overwriting_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root);
        let base = commit_file(root, "f.txt", "base\n", "base");
        git_output(root, &["branch", "merged"]);
        git_output(root, &["checkout", "-q", "-b", "unmerged"]);
        let unmerged = commit_file(root, "f.txt", "unmerged\n", "unmerged work");
        git_output(root, &["checkout", "-q", "main"]);
        git_output(
            root,
            &["remote", "add", "origin", "/nonexistent/remote.git"],
        );
        git_output(
            root,
            &["update-ref", "refs/remotes/origin/topic", &unmerged],
        );
        git_output(root, &["update-ref", "refs/remotes/origin/merged", &base]);
        let router = router_for(root).await;
        let root_str = root.to_string_lossy().to_string();
        let post = |uri: &'static str, body: Value| {
            let router = router.clone();
            async move { send(&router, uri, Some(body)).await }
        };

        let (status, body) = post(
            "/v1/git/branch/delete",
            json!({"root": root_str, "name": "main", "force": true}),
        )
        .await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::CONFLICT, json!("current_branch")),
            "{body}"
        );
        assert!(ref_exists(root, "refs/heads/main"));

        let (status, body) = post(
            "/v1/git/branch/delete",
            json!({"root": root_str, "name": "unmerged"}),
        )
        .await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::CONFLICT, json!("not_merged")),
            "{body}"
        );
        assert_eq!(rev(root, "refs/heads/unmerged"), unmerged);
        let (status, body) = post(
            "/v1/git/branch/delete",
            json!({"root": root_str, "name": "merged"}),
        )
        .await;
        assert_eq!(
            (status, body["deleted"].clone()),
            (StatusCode::OK, json!("merged")),
            "{body}"
        );
        assert!(!ref_exists(root, "refs/heads/merged"));
        let (status, body) = post(
            "/v1/git/branch/delete",
            json!({"root": root_str, "name": "-D"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

        let (status, body) = post(
            "/v1/git/branch/rename",
            json!({"root": root_str, "old_name": "unmerged", "new_name": "renamed"}),
        )
        .await;
        assert_eq!(
            (status, body["name"].clone()),
            (StatusCode::OK, json!("renamed")),
            "{body}"
        );
        assert!(!ref_exists(root, "refs/heads/unmerged"));
        assert_eq!(rev(root, "refs/heads/renamed"), unmerged);

        fs::write(root.join("f.txt"), "local edit\n").unwrap();
        let (status, body) = post(
            "/v1/git/branch/checkout",
            json!({"root": root_str, "name": "renamed", "kind": "local"}),
        )
        .await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::CONFLICT, json!("checkout_conflict")),
            "{body}"
        );
        assert!(body["error"].as_str().unwrap().contains("f.txt"), "{body}");
        assert_eq!(
            fs::read_to_string(root.join("f.txt")).unwrap(),
            "local edit\n"
        );
        assert_eq!(
            git_output(root, &["symbolic-ref", "HEAD"]),
            "refs/heads/main"
        );
        assert_eq!(rev(root, "HEAD"), base);
        fs::write(root.join("f.txt"), "base\n").unwrap();
        git_output(root, &["branch", "-q", "doomed", &unmerged]);
        let (status, body) = post(
            "/v1/git/branch/delete",
            json!({"root": root_str, "name": "doomed"}),
        )
        .await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::CONFLICT, json!("not_merged")),
            "{body}"
        );
        let (status, body) = post(
            "/v1/git/branch/delete",
            json!({"root": root_str, "name": "doomed", "force": true}),
        )
        .await;
        assert_eq!(
            (status, body["deleted"].clone()),
            (StatusCode::OK, json!("doomed")),
            "{body}"
        );
        assert!(!ref_exists(root, "refs/heads/doomed"));
        assert_eq!(rev(root, "refs/heads/renamed"), unmerged);

        let (status, body) = post(
            "/v1/git/branch/checkout",
            json!({"root": root_str, "name": "origin/topic", "kind": "remote"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            json!({"branch": "topic", "head_oid": unmerged, "detached": false})
        );
        assert_eq!(
            git_output(root, &["config", "branch.topic.merge"]),
            "refs/heads/topic"
        );
        assert_eq!(
            git_output(root, &["config", "branch.topic.remote"]),
            "origin"
        );
        git_output(root, &["branch", "-q", "merged", &base]);
        let (status, body) = post(
            "/v1/git/branch/checkout",
            json!({"root": root_str, "name": "origin/merged", "kind": "remote"}),
        )
        .await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::CONFLICT, json!("already_exists")),
            "{body}"
        );
        assert_eq!(
            git_output(root, &["symbolic-ref", "HEAD"]),
            "refs/heads/topic"
        );
    }

    #[tokio::test]
    async fn git_fetch_pull_push_against_local_bare_remote() {
        let holder = tempfile::tempdir().unwrap();
        let bare = holder.path().join("remote.git");
        let seed = holder.path().join("seed");
        let work = holder.path().join("work");
        let other = holder.path().join("other");
        fs::create_dir_all(&seed).unwrap();
        git_output(
            holder.path(),
            &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()],
        );
        init(&seed);
        commit_file(&seed, "f.txt", "one\n", "one");
        git_output(&seed, &["push", "-q", bare.to_str().unwrap(), "main:main"]);
        for clone in [&work, &other] {
            git_output(
                holder.path(),
                &[
                    "clone",
                    "-q",
                    "-c",
                    "core.autocrlf=false",
                    bare.to_str().unwrap(),
                    clone.to_str().unwrap(),
                ],
            );
            configure(clone);
        }
        let router = router_for(&work).await;
        let root = work.to_string_lossy().to_string();
        let post = |uri: &'static str, body: Value| {
            let router = router.clone();
            async move { send(&router, uri, Some(body)).await }
        };
        let main_behind = || {
            let router = router.clone();
            let work = work.clone();
            async move {
                let (_, body) = send(
                    &router,
                    &format!("/v1/git/branches?root={}", encoded(&work)),
                    None,
                )
                .await;
                let main = body["roots"][0]["branches"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|branch| branch["name"] == "main")
                    .unwrap()
                    .clone();
                (main["ahead"].clone(), main["behind"].clone())
            }
        };

        let upstream_two = commit_file(&other, "f.txt", "two\n", "two");
        git_output(&other, &["push", "-q", "origin", "main"]);
        assert_eq!(main_behind().await, (json!(0), json!(0)));
        let (status, body) = post("/v1/git/fetch", json!({"root": root, "prune": true})).await;
        assert_eq!(
            (status, body["ok"].clone()),
            (StatusCode::OK, json!(true)),
            "{body}"
        );
        assert_eq!(main_behind().await, (json!(0), json!(1)));
        let (status, body) = post("/v1/git/pull", json!({"root": root})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(rev(&work, "HEAD"), upstream_two);
        assert_eq!(fs::read_to_string(work.join("f.txt")).unwrap(), "two\n");

        let local_three = commit_file(&work, "local.txt", "local\n", "local three");
        let upstream_three = commit_file(&other, "other.txt", "other\n", "upstream three");
        git_output(&other, &["push", "-q", "origin", "main"]);
        let (status, body) = post("/v1/git/pull", json!({"root": root})).await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::CONFLICT, json!("not_fast_forward")),
            "{body}"
        );
        assert_eq!(rev(&work, "HEAD"), local_three);
        assert!(!work.join("other.txt").exists());
        let (status, body) = post("/v1/git/push", json!({"root": root})).await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::CONFLICT, json!("rejected")),
            "{body}"
        );
        assert_eq!(rev(&bare, "refs/heads/main"), upstream_three);

        git_output(&work, &["checkout", "-q", "-b", "newb"]);
        let (status, body) = post("/v1/git/push", json!({"root": root})).await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::CONFLICT, json!("no_upstream")),
            "{body}"
        );
        assert!(body["error"].as_str().unwrap().contains("origin"), "{body}");
        assert!(!ref_exists(&bare, "refs/heads/newb"));
        let (status, body) =
            post("/v1/git/push", json!({"root": root, "set_upstream": true})).await;
        assert_eq!(
            (status, body["ok"].clone()),
            (StatusCode::OK, json!(true)),
            "{body}"
        );
        assert_eq!(rev(&bare, "refs/heads/newb"), local_three);
        assert_eq!(
            git_output(&work, &["config", "branch.newb.merge"]),
            "refs/heads/newb"
        );
        assert_eq!(rev(&bare, "refs/heads/main"), upstream_three);
    }
}
