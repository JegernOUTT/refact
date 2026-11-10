use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use hyper::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};
use refact_review_core::{Error, Mutation, Review};
use crate::app_state::AppState;
use super::{chat_bridge, service};

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<Value>)>;
fn error(status: StatusCode, detail: impl ToString) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"detail": detail.to_string()})))
}
fn core_error(e: Error) -> (StatusCode, Json<Value>) {
    let status = match e {
        Error::NotFound => StatusCode::NOT_FOUND,
        Error::Conflict {
            ..
        }
        | Error::IdempotencyConflict => StatusCode::CONFLICT,
        Error::Invalid(_) | Error::Limit(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error(status, e)
}
async fn resolve(
    app: &AppState,
    id: &str,
) -> Result<(std::path::PathBuf, Review), (StatusCode, Json<Value>)> {
    service::resolve_review(app, id)
        .await
        .map_err(|e| error(StatusCode::NOT_FOUND, e))
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/reviews/by-chat/:chat_id", get(by_chat))
        .route("/reviews/:id", get(get_review))
        .route("/reviews/:id/files", get(file))
        .route("/reviews/:id/history", get(history))
        .route("/reviews/:id/handoff", get(handoff))
        .route("/review-stacks/by-chat/:chat", get(stack_by_chat))
        .route("/review-stacks/by-chat/:chat/prepare", post(stack_prepare))
        .route("/review-stacks/by-chat/:chat/import", post(stack_import))
        .route("/review-stacks/by-chat/:chat/refresh", post(stack_refresh))
        .route(
            "/review-stacks/by-chat/:chat/cumulative/:member/files",
            get(stack_cumulative_file),
        )
        .route("/review-stacks/by-chat/:chat/history", get(stack_history))
        .route(
            "/review-stacks/by-chat/:chat/members/:member",
            get(stack_member),
        )
        .route(
            "/review-stacks/by-chat/:chat/cumulative/:member",
            get(stack_cumulative),
        )
        .route("/reviews/:id/delivery-status", get(delivery_status))
        .route("/reviews/:id/blobs/:digest", get(blob))
        .route("/reviews/:id/anchors", post(anchor))
        .route("/reviews/:id/mutations", post(mutate))
        .route("/reviews/:id/comments", post(comment))
        .route("/reviews/:id/activity", post(activity))
        .route("/reviews/:id/runs/:run_id/cancel", post(cancel))
}
async fn handoff(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<super::handoff::HandoffExport> {
    resolve(&app, &id).await?;
    super::handoff::export(&app, &id)
        .await
        .map(Json)
        .map_err(|e| error(StatusCode::CONFLICT, e))
}
async fn stack_by_chat(
    State(app): State<AppState>,
    Path(chat): Path<String>,
) -> ApiResult<Option<refact_review_core::StackManifest>> {
    super::stacks::get(&app, &chat)
        .await
        .map(Json)
        .map_err(|e| error(StatusCode::NOT_FOUND, e))
}
async fn stack_prepare(
    State(app): State<AppState>,
    Path(chat): Path<String>,
    Json(manifest): Json<refact_review_core::StackManifest>,
) -> ApiResult<super::stacks::PreparedStack> {
    super::stacks::prepare(&app, &chat, manifest)
        .await
        .map(Json)
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))
}
async fn stack_import(
    State(app): State<AppState>,
    Path(chat): Path<String>,
    Json(request): Json<RawStackImportRequest>,
) -> ApiResult<refact_review_core::StackManifest> {
    super::stacks::import(
        &app,
        &chat,
        super::stacks::ImportRequest {
            manifest: request.manifest,
            scope_receipt_tool_call_id: request.scope_receipt_tool_call_id,
            request_key: request.request_key,
        },
    )
    .await
    .map(Json)
    .map_err(|e| error(StatusCode::CONFLICT, e))
}
#[derive(Deserialize)]
struct RawStackImportRequest {
    manifest: refact_review_core::StackManifest,
    scope_receipt_tool_call_id: Option<String>,
    request_key: String,
}
#[derive(Deserialize)]
struct RefreshStackRequest {
    expected_revision: u64,
    request_key: String,
}
async fn stack_refresh(
    State(app): State<AppState>,
    Path(chat): Path<String>,
    Json(request): Json<RefreshStackRequest>,
) -> ApiResult<refact_review_core::StackManifest> {
    super::stacks::refresh(&app, &chat, request.request_key, request.expected_revision)
        .await
        .map(Json)
        .map_err(|e| error(StatusCode::CONFLICT, e))
}
#[derive(Deserialize)]
struct CumulativeFileQuery {
    path: String,
    expected_revision: u64,
}
async fn stack_cumulative_file(
    State(app): State<AppState>,
    Path((chat, member)): Path<(String, String)>,
    Query(query): Query<CumulativeFileQuery>,
) -> ApiResult<refact_review_core::FileView> {
    super::stacks::cumulative_file(&app, &chat, member, query.path, query.expected_revision)
        .await
        .map(Json)
        .map_err(|e| error(StatusCode::CONFLICT, e))
}
async fn stack_history(
    State(app): State<AppState>,
    Path(chat): Path<String>,
) -> ApiResult<Vec<refact_review_core::StackManifest>> {
    super::stacks::history(&app, &chat)
        .await
        .map(Json)
        .map_err(|e| error(StatusCode::NOT_FOUND, e))
}
async fn stack_member(
    State(app): State<AppState>,
    Path((chat, member)): Path<(String, String)>,
) -> ApiResult<Option<Review>> {
    super::stacks::member(&app, &chat, member)
        .await
        .map(Json)
        .map_err(|e| error(StatusCode::NOT_FOUND, e))
}
async fn stack_cumulative(
    State(app): State<AppState>,
    Path((chat, member)): Path<(String, String)>,
) -> ApiResult<Value> {
    super::stacks::cumulative(&app, &chat, member)
        .await
        .map(Json)
        .map_err(|e| error(StatusCode::CONFLICT, e))
}
async fn by_chat(
    State(app): State<AppState>,
    Path(chat): Path<String>,
) -> ApiResult<Option<Review>> {
    let root = service::trusted_root(&app, &chat)
        .await
        .map_err(|e| error(StatusCode::NOT_FOUND, e))?;
    let review = service::get_for_chat(root.clone(), chat)
        .await
        .map_err(core_error)?;
    let review = if let Some(r) = review {
        Some(
            super::runs::refresh_runs(&app, root.clone(), r)
                .await
                .map_err(|e| error(StatusCode::CONFLICT, e))?,
        )
    } else {
        None
    };
    if let Some(r) = &review {
        if let Err(e) = chat_bridge::flush_read_only(&app, root, r).await {
            tracing::warn!(error=%e,"review delivery pending");
        }
    }
    Ok(Json(review))
}
async fn get_review(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Review> {
    let (root, review) = resolve(&app, &id).await?;
    let review = super::runs::refresh_runs(&app, root.clone(), review)
        .await
        .map_err(|e| error(StatusCode::CONFLICT, e))?;
    if let Err(e) = chat_bridge::flush_read_only(&app, root, &review).await {
        tracing::warn!(error=%e,"review delivery pending");
    }
    Ok(Json(review))
}
async fn delivery_status(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<refact_review_core::ReviewDelivery>> {
    let (root, review) = resolve(&app, &id).await?;
    if let Err(e) = chat_bridge::flush_read_only(&app, root.clone(), &review).await {
        tracing::warn!(error=%e, "review delivery pending");
    }
    service::with_store(root, move |s| s.pending_deliveries(&id))
        .await
        .map(Json)
        .map_err(core_error)
}
async fn history(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<refact_review_core::ObjectRevision>> {
    let (root, _) = resolve(&app, &id).await?;
    service::with_store(root, move |s| s.object_history(&id))
        .await
        .map(Json)
        .map_err(core_error)
}
async fn blob(
    State(app): State<AppState>,
    Path((id, digest)): Path<(String, String)>,
) -> ApiResult<Vec<u8>> {
    let (root, review) = resolve(&app, &id).await?;
    let reference = review
        .materials
        .iter()
        .flat_map(|m| m.artifacts.iter())
        .chain(review.evidence.iter().flat_map(|e| match &e.source {
            refact_review_core::EvidenceSource::Executed {
                stdout,
                stderr,
                ..
            } => stdout.iter().chain(stderr.iter()).collect::<Vec<_>>(),
            _ => vec![],
        }))
        .find(|b| b.digest == digest)
        .cloned()
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "blob not referenced"))?;
    service::with_store(root, move |s| s.read_blob(&reference))
        .await
        .map(Json)
        .map_err(core_error)
}
#[derive(Deserialize)]
struct FileQuery {
    candidate_id: String,
    path: String,
}
async fn file(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<FileQuery>,
) -> ApiResult<refact_review_core::FileView> {
    let (root, review) = resolve(&app, &id).await?;
    service::file(root, review, q.candidate_id, q.path)
        .await
        .map(Json)
        .map_err(core_error)
}
async fn anchor(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<service::AnchorRequest>,
) -> ApiResult<refact_review_core::Anchor> {
    let (root, review) = resolve(&app, &id).await?;
    service::anchor(root, review, request)
        .await
        .map(Json)
        .map_err(core_error)
}
async fn mutate(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<Mutation>,
) -> ApiResult<Review> {
    let (root, review) = resolve(&app, &id).await?;
    let updated = service::user_mutate(root, review.owner_chat_id, id, request)
        .await
        .map_err(core_error)?;
    chat_bridge::invalidate(&app, &updated).await;
    Ok(Json(updated))
}
async fn comment(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<service::CommentRequest>,
) -> ApiResult<Review> {
    let (root, review) = resolve(&app, &id).await?;
    service::comment(app, root, review.owner_chat_id, id, request)
        .await
        .map(Json)
        .map_err(core_error)
}
async fn activity(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<chat_bridge::ActivityRequest>,
) -> ApiResult<Value> {
    let (root, review) = resolve(&app, &id).await?;
    chat_bridge::activity(root, &review, request)
        .await
        .map_err(core_error)?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct CancelRequest {
    expected_revision: u64,
    request_key: String,
}
async fn cancel(
    State(app): State<AppState>,
    Path((id, run_id)): Path<(String, String)>,
    Json(request): Json<CancelRequest>,
) -> ApiResult<Review> {
    let (root, review) = resolve(&app, &id).await?;
    if request.request_key.trim().is_empty() || request.request_key.len() > 256 {
        return Err(error(StatusCode::BAD_REQUEST, "invalid request key"));
    }
    let run = review
        .runs
        .iter()
        .find(|r| r.id == run_id)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "run not found"))?;
    let record = app
        .agents
        .get_any(&run.agent_id)
        .await
        .map_err(|e| error(StatusCode::CONFLICT, e))?;
    if record.parent_chat_id != review.owner_chat_id
        && record.parent_root_chat_id.as_deref() != Some(&review.owner_chat_id)
    {
        return Err(error(StatusCode::FORBIDDEN, "agent ownership mismatch"));
    }
    let mutation = Mutation {
        expected_revision: request.expected_revision,
        request_key: request.request_key,
        actor: refact_review_core::Actor {
            kind: refact_review_core::ActorKind::User,
            id: "user".into(),
        },
        at_ms: service::now_ms(),
        operation: refact_review_core::Operation::RequestRunCancel {
            run_id,
        },
    };
    let intent = service::mutate_owned(root.clone(), review.owner_chat_id, id, mutation)
        .await
        .map_err(core_error)?;
    chat_bridge::invalidate(&app, &intent).await;
    let settled = super::runs::refresh_runs(&app, root, intent)
        .await
        .map_err(|e| error(StatusCode::CONFLICT, e))?;
    chat_bridge::invalidate(&app, &settled).await;
    Ok(Json(settled))
}
