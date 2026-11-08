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
    WorktreeRecordView, WorktreeSyncRequest,
};

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<Value>)>;

#[derive(Debug, Deserialize)]
pub struct WorktreeQuery {
    #[serde(default)]
    pub source_workspace_root: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WorktreeListQuery {
    #[serde(default)]
    pub source_workspace_root: Option<String>,
    #[serde(default)]
    pub include_branches: Option<bool>,
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
        || lower.contains("revision")
        || lower.contains("head changed")
        || lower.contains("synchronization")
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
    Query(query): Query<WorktreeListQuery>,
) -> ApiResult<WorktreeListResponse> {
    let gcx = app.gcx.clone();
    let service = service_for_request(gcx, query.source_workspace_root).await?;
    service
        .list_worktrees_with_branches(query.include_branches.unwrap_or(true))
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
                    generate_commit_message_by_diff(gcx.clone(), &diff.patch, &prompt, None).await
                {
                    if !message.trim().is_empty() {
                        request.commit_message = Some(message);
                    }
                }
            }
        }
    }
    let result = crate::tools::tool_task_merge_agent::merge_under_available_lease(
        &app,
        &service,
        &id,
        request,
        None,
        |_, _| async { Err("No post-merge check requested".into()) },
    )
    .await
    .map(|(response, _, _)| response)
    .map_err(|error| {
        if error.contains("merge_busy") || error.contains("Timed out waiting") {
            (
                StatusCode::CONFLICT,
                Json(json!({
                    "code": "merge_busy", "error": error, "retryable": true,
                    "request_id": uuid::Uuid::new_v4().to_string(),
                })),
            )
        } else {
            map_service_error(error)
        }
    });
    result.map(Json)
}

pub async fn handle_v1_worktrees_sync(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<WorktreeQuery>,
    Json(request): Json<WorktreeSyncRequest>,
) -> ApiResult<crate::tools::tool_worktree_sync::SyncResponse> {
    let service = service_for_request(app.gcx.clone(), query.source_workspace_root).await?;
    let writer_readiness = if matches!(request, WorktreeSyncRequest::Status { .. }) {
        Some(
            crate::tools::tool_worktree_sync::status_readiness(&app, &service, &id, None)
                .await
                .map_err(map_service_error)?,
        )
    } else {
        None
    };
    crate::tools::tool_worktree_sync::sync_under_lease(
        &app,
        &service,
        &id,
        request,
        "user".into(),
        None,
    )
    .await
    .map(|sync| {
        Json(crate::tools::tool_worktree_sync::SyncResponse {
            sync,
            writer_readiness,
        })
    })
    .map_err(map_service_error)
}

#[cfg(test)]
mod sync_boundary_tests {
    use super::*;

    #[test]
    fn sync_http_uses_strict_tagged_request() {
        let status: WorktreeSyncRequest =
            serde_json::from_value(json!({"action":"status"})).unwrap();
        assert!(matches!(status, WorktreeSyncRequest::Status { .. }));
        assert!(serde_json::from_value::<WorktreeSyncRequest>(
            json!({"action":"status","force":true})
        )
        .is_err());
        assert!(serde_json::from_value::<WorktreeSyncRequest>(
            json!({"action":"start","expected_source_head":"a"})
        )
        .is_err());
        assert!(serde_json::from_value::<WorktreeSyncRequest>(
            json!({"action":"abort","operation_id":"a","expected_revision":1,"force":true})
        )
        .is_ok());
    }
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
