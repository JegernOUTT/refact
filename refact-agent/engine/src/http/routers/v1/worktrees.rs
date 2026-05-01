use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query};
use axum::response::Json;
use axum::Extension;
use hyper::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock as ARwLock;

use crate::agentic::generate_commit_message::generate_commit_message_by_diff;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
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
    if lower.contains("not found") {
        StatusCode::NOT_FOUND
    } else if lower.contains("conflict") || lower.contains("merge in progress") {
        StatusCode::CONFLICT
    } else if lower.contains("invalid")
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
    gcx: Arc<ARwLock<GlobalContext>>,
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
            let requested_canonical = requested_path.canonicalize().map_err(|e| {
                api_error(
                    StatusCode::BAD_REQUEST,
                    format!("Invalid source workspace root: {}", e),
                )
            })?;
            let requested_canonical = dunce::simplified(&requested_canonical).to_path_buf();
            let matches = project_dirs.iter().any(|dir| {
                dir.canonicalize()
                    .map(|canonical| {
                        dunce::simplified(&canonical).to_path_buf() == requested_canonical
                    })
                    .unwrap_or(false)
            });
            if matches {
                Ok(requested_canonical)
            } else {
                Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "Invalid source workspace root: not a workspace directory",
                ))
            }
        }
        None => project_dirs
            .into_iter()
            .next()
            .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "No project root available"))?
            .canonicalize()
            .map(|path| dunce::simplified(&path).to_path_buf())
            .map_err(|e| {
                api_error(
                    StatusCode::BAD_REQUEST,
                    format!("Invalid project root: {}", e),
                )
            }),
    }
}

async fn service_for_request(
    gcx: Arc<ARwLock<GlobalContext>>,
    requested: Option<String>,
) -> Result<WorktreeService, (StatusCode, Json<Value>)> {
    let cache_dir = gcx.read().await.cache_dir.clone();
    let source_root = resolve_source_root(gcx, requested).await?;
    WorktreeService::new(cache_dir, source_root).map_err(map_service_error)
}

pub async fn handle_v1_worktrees_list(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(query): Query<WorktreeQuery>,
) -> ApiResult<WorktreeListResponse> {
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    service
        .list_worktrees()
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_summary(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(query): Query<WorktreeQuery>,
) -> ApiResult<WorktreeInventory> {
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    service
        .inspect_worktrees()
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_cleanup_dry_run(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(query): Query<WorktreeQuery>,
    Json(request): Json<WorktreeCleanupRequest>,
) -> ApiResult<WorktreeCleanupPlan> {
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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(query): Query<WorktreeQuery>,
    Json(request): Json<WorktreeCleanupRequest>,
) -> ApiResult<WorktreeCleanupResult> {
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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Json(request): Json<CreateWorktreeRequest>,
) -> ApiResult<CreateWorktreeResponse> {
    let service = service_for_request(gcx, request.source_workspace_root.clone()).await?;
    service
        .create_worktree(request)
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
    Query(query): Query<WorktreeQuery>,
) -> ApiResult<WorktreeRecordView> {
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    service
        .get_worktree(&id)
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_diff(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
    Query(query): Query<WorktreeDiffQuery>,
) -> ApiResult<WorktreeDiffResponse> {
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
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
    Query(query): Query<WorktreeQuery>,
    Json(mut request): Json<MergeWorktreeRequest>,
) -> ApiResult<MergeWorktreeResponse> {
    let service = service_for_request(gcx.clone(), query.source_workspace_root).await?;
    if request.generate_commit_message
        && request
            .commit_message
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
    {
        let diff = service
            .diff_worktree(&id)
            .await
            .map_err(map_service_error)?;
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
    service
        .merge_worktree(&id, request)
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_delete(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
    Query(query): Query<DeleteWorktreeQuery>,
) -> ApiResult<DeleteWorktreeResponse> {
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    service
        .delete_worktree(&id, query.delete_branch.unwrap_or(false))
        .await
        .map(Json)
        .map_err(map_service_error)
}

pub async fn handle_v1_worktrees_open(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
    Query(query): Query<WorktreeQuery>,
) -> ApiResult<OpenWorktreeResponse> {
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    service
        .open_worktree(&id)
        .await
        .map(Json)
        .map_err(map_service_error)
}
