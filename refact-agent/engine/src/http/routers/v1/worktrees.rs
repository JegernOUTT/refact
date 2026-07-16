use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::Json;
use hyper::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use crate::agentic::generate_commit_message::generate_commit_message_by_diff;
use crate::files_correction::get_project_dirs;
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{
    CreateWorktreeRequest, CreateWorktreeResponse, DeleteWorktreeResponse, MergeWorktreeRequest,
    MergeWorktreeResponse, OpenWorktreeResponse, WorktreeCleanupPlan, WorktreeCleanupRequest,
    WorktreeCleanupResult, WorktreeDiffResponse, WorktreeInventory, WorktreeListResponse,
    WorktreeRecordView,
};

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<Value>)>;

#[derive(Debug, Deserialize)]
pub struct WorktreeQuery {
    #[serde(default)]
    pub source_workspace_root: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WorktreeDiffQuery {
    #[serde(default)]
    pub source_workspace_root: Option<String>,
    #[serde(default)]
    pub max_patch_bytes: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteWorktreeQuery {
    #[serde(default)]
    pub source_workspace_root: Option<String>,
    #[serde(default)]
    pub delete_branch: Option<bool>,
    #[serde(default)]
    pub force_referenced: Option<bool>,
}

fn api_error(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<Value>) {
    let code = match status {
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::CONFLICT => "conflict",
        _ => "worktree_error",
    };
    (
        status,
        Json(json!({ "code": code, "error": message.into() })),
    )
}

fn status_for_error(error: &str) -> StatusCode {
    let lower = error.to_lowercase();
    if lower.contains("not found") || (lower.contains("path") && lower.contains("does not exist")) {
        StatusCode::NOT_FOUND
    } else if lower.contains("conflict")
        || lower.contains("merge in progress")
        || lower.contains("uncommitted changes")
        || lower.contains("different branch")
        || lower.contains("detached head")
        || lower.contains("auto-commit")
        || lower.contains("not a git worktree")
        || lower.contains("not the git worktree root")
        || lower.contains("belongs to a different repository")
        || lower.contains("already exists")
        || lower.contains("active reference")
        || lower.contains("force_referenced")
        || lower.contains("refusing")
    {
        StatusCode::CONFLICT
    } else if lower.contains("invalid")
        || lower.contains("worktree id")
        || lower.contains("must contain only")
        || lower.contains("too long")
        || lower.contains("not a git repository")
        || lower.contains("no project root")
        || lower.contains("outside registry")
        || lower.contains("cannot be empty")
        || lower.contains("requires explicit")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

fn map_service_error(error: String) -> (StatusCode, Json<Value>) {
    api_error(status_for_error(&error), error)
}

async fn resolve_source_root(
    gcx: Arc<GlobalContext>,
    requested: Option<String>,
) -> Result<PathBuf, (StatusCode, Json<Value>)> {
    let project_dirs = get_project_dirs(gcx).await;
    if project_dirs.is_empty() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "No project root available",
        ));
    }
    match requested {
        Some(path) => {
            let requested_path = PathBuf::from(path);
            let requested_canonical =
                tokio::fs::canonicalize(&requested_path)
                    .await
                    .map_err(|e| {
                        api_error(
                            StatusCode::BAD_REQUEST,
                            format!("Invalid source workspace root: {}", e),
                        )
                    })?;
            let requested_canonical = dunce::simplified(&requested_canonical).to_path_buf();
            let mut matches = false;
            for dir in &project_dirs {
                if tokio::fs::canonicalize(dir)
                    .await
                    .map(|canonical| {
                        dunce::simplified(&canonical).to_path_buf() == requested_canonical
                    })
                    .unwrap_or(false)
                {
                    matches = true;
                    break;
                }
            }
            if matches {
                Ok(requested_canonical)
            } else {
                Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "Invalid source workspace root: not a workspace directory",
                ))
            }
        }
        None => {
            let project_root = project_dirs
                .into_iter()
                .next()
                .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "No project root available"))?;
            tokio::fs::canonicalize(project_root)
                .await
                .map(|path| dunce::simplified(&path).to_path_buf())
                .map_err(|e| {
                    api_error(
                        StatusCode::BAD_REQUEST,
                        format!("Invalid project root: {}", e),
                    )
                })
        }
    }
}

async fn service_for_request(
    gcx: Arc<GlobalContext>,
    requested: Option<String>,
) -> Result<WorktreeService, (StatusCode, Json<Value>)> {
    let cache_dir = gcx.cache_dir.clone();
    let source_root = resolve_source_root(gcx, requested).await?;
    WorktreeService::new_async(cache_dir, source_root)
        .await
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_list(
    State(app): State<AppState>,
    Query(query): Query<WorktreeQuery>,
) -> ApiResult<WorktreeListResponse> {
    let gcx = app.gcx.clone();
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    service
        .list_worktrees()
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_summary(
    State(app): State<AppState>,
    Query(query): Query<WorktreeQuery>,
) -> ApiResult<WorktreeInventory> {
    let gcx = app.gcx.clone();
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    service
        .inspect_worktrees()
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_cleanup_dry_run(
    State(app): State<AppState>,
    Query(query): Query<WorktreeQuery>,
    Json(request): Json<WorktreeCleanupRequest>,
) -> ApiResult<WorktreeCleanupPlan> {
    let gcx = app.gcx.clone();
    let requested_root = request
        .source_workspace_root
        .clone()
        .or(query.source_workspace_root);
    let service = service_for_request(gcx, requested_root).await?;
    service
        .cleanup_worktrees_dry_run(request)
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_cleanup(
    State(app): State<AppState>,
    Query(query): Query<WorktreeQuery>,
    Json(request): Json<WorktreeCleanupRequest>,
) -> ApiResult<WorktreeCleanupResult> {
    let gcx = app.gcx.clone();
    let requested_root = request
        .source_workspace_root
        .clone()
        .or(query.source_workspace_root);
    let service = service_for_request(gcx, requested_root).await?;
    service
        .cleanup_worktrees(request)
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_create(
    State(app): State<AppState>,
    Json(request): Json<CreateWorktreeRequest>,
) -> ApiResult<CreateWorktreeResponse> {
    let gcx = app.gcx.clone();
    let service = service_for_request(gcx, request.source_workspace_root.clone()).await?;
    service
        .create_worktree(request)
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WorktreeQuery>,
) -> ApiResult<WorktreeRecordView> {
    let gcx = app.gcx.clone();
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    service
        .get_worktree(&id)
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_diff(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WorktreeDiffQuery>,
) -> ApiResult<WorktreeDiffResponse> {
    let gcx = app.gcx.clone();
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    match query.max_patch_bytes {
        Some(max_patch_bytes) => service
            .diff_worktree_with_limit(&id, max_patch_bytes.max(1).min(1_000_000))
            .await
            .map(Json)
            .map_err(map_service_error),
        None => service
            .diff_worktree(&id)
            .await
            .map(Json)
            .map_err(map_service_error),
    }
}

pub async fn handle_v1_worktrees_merge(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WorktreeQuery>,
    Json(mut request): Json<MergeWorktreeRequest>,
) -> ApiResult<MergeWorktreeResponse> {
    let gcx = app.gcx.clone();
    let service = service_for_request(gcx.clone(), query.source_workspace_root).await?;
    if request.generate_commit_message
        && request
            .commit_message
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
    {
        service
            .validate_registered_worktree_checkout(&id)
            .await
            .map_err(map_service_error)?;
        if let Ok(diff) = service.diff_worktree(&id).await {
            if !diff.files.is_empty() {
                let prompt = request
                    .target_branch
                    .clone()
                    .or_else(|| diff.base_branch.clone())
                    .map(|target| format!("Merge worktree into {}", target));
                if let Ok(message) =
                    generate_commit_message_by_diff(gcx.clone(), &diff.patch, &prompt).await
                {
                    if !message.trim().is_empty() {
                        request.commit_message = Some(message);
                    }
                }
            }
        }
    }
    service
        .merge_worktree(&id, request)
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_delete(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DeleteWorktreeQuery>,
) -> ApiResult<DeleteWorktreeResponse> {
    let gcx = app.gcx.clone();
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    service
        .delete_worktree(
            &id,
            query.delete_branch.unwrap_or(false),
            query.force_referenced.unwrap_or(false),
        )
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_open(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WorktreeQuery>,
) -> ApiResult<OpenWorktreeResponse> {
    let gcx = app.gcx.clone();
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    service
        .open_worktree(&id)
        .await
        .map(Json)
        .map_err(map_service_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_for_error_maps_user_correctable_worktree_states_to_conflict() {
        for message in [
            "Worktree has uncommitted changes; set include_uncommitted=true",
            "Target workspace has uncommitted changes on a different branch",
            "Target workspace has uncommitted changes on detached HEAD",
            "Failed to auto-commit source worktree changes",
            "Branch 'refact/chat/test' already exists",
            "Worktree 'wt_1' is not a git worktree",
            "Worktree 'wt_1' path '/tmp/wt_1' is not the git worktree root",
            "Worktree 'wt_1' path '/tmp/wt_1' belongs to a different repository",
            "Worktree 'wt_1' still has 1 active reference(s) (kinds: chat); pass force_referenced=true to delete anyway.",
        ] {
            assert_eq!(status_for_error(message), StatusCode::CONFLICT, "{message}");
        }
    }

    #[test]
    fn status_for_error_maps_missing_worktree_path_to_not_found() {
        assert_eq!(
            status_for_error("Worktree 'wt_1' path '/tmp/wt_1' does not exist"),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn status_for_error_maps_invalid_worktree_id_to_bad_request() {
        for message in [
            "Worktree ID must contain only ASCII alphanumeric characters, hyphens, or underscores",
            "Worktree ID is too long",
        ] {
            assert_eq!(
                status_for_error(message),
                StatusCode::BAD_REQUEST,
                "{message}"
            );
        }
    }
}
