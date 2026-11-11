use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use std::collections::HashSet;
use hyper::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};
use refact_review_core::{Error, Mutation, Review};
use crate::app_state::AppState;
use super::{authoring, chat_bridge, remote, service};

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
/// What the canvas reads on every poll. Provenance lists stay in the store, and of the
/// unchanged files only those the review points at are listed; the rest load on demand.
pub(crate) fn slim(mut review: Review) -> Review {
    review.privacy.clear();
    for material in &mut review.materials {
        if let Some(labels) = material.input_digests.get_mut("derived_origin_labels") {
            labels.clear();
        }
    }
    for contribution in review
        .threads
        .iter_mut()
        .flat_map(|thread| thread.contributions.iter_mut())
    {
        if let Some(context) = &mut contribution.context {
            context.privacy.clear();
        }
    }
    let mut keep: HashSet<String> = review
        .candidates
        .iter()
        .flat_map(|candidate| &candidate.files)
        .filter(|file| file.base != file.head || file.base_mode != file.head_mode)
        .map(|file| file.path.clone())
        .collect();
    let blocks = review
        .tour
        .iter()
        .flat_map(|step| std::iter::once(&step.current).chain(&step.history))
        .flat_map(|version| &version.blocks)
        .chain(
            review
                .threads
                .iter()
                .flat_map(|thread| &thread.contributions)
                .flat_map(|contribution| &contribution.blocks),
        );
    let mut anchors: Vec<&refact_review_core::Anchor> = Vec::new();
    for block in blocks {
        anchors.extend(refact_review_core::rich_blocks::block_anchors(block));
        if let refact_review_core::RichBlockContent::Diff {
            path, ..
        } = &block.content
        {
            keep.insert(path.clone());
        }
    }
    for step in &review.tour {
        anchors.extend(
            std::iter::once(&step.current)
                .chain(&step.history)
                .flat_map(|v| &v.anchors),
        );
    }
    anchors.extend(review.annotations.iter().flat_map(|item| &item.anchors));
    anchors.extend(review.threads.iter().flat_map(|thread| &thread.anchors));
    for evidence in &review.evidence {
        if let refact_review_core::EvidenceSource::Source {
            path,
            anchor,
        } = &evidence.source
        {
            keep.insert(path.clone());
            anchors.extend(anchor);
        }
    }
    for anchor in anchors {
        keep.insert(anchor.position.path.clone());
        keep.extend(anchor.history.iter().map(|position| position.path.clone()));
    }
    for candidate in &mut review.candidates {
        candidate.files.retain(|file| keep.contains(&file.path));
    }
    review
}
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/reviews/by-chat/:chat_id", get(by_chat))
        .route("/reviews/:id", get(get_review))
        .route("/reviews/:id/files", get(file))
        .route(
            "/reviews/:id/candidates/:candidate_id/files",
            get(candidate_files),
        )
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
        .route("/reviews/:id/outbox", post(outbox))
        .route("/reviews/:id/pr-sync", post(sync_comments))
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
        .map(|review| Json(review.map(slim)))
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
    Ok(Json(review.map(slim)))
}
async fn get_review(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<Review> {
    let (root, review) = resolve(&app, &id).await?;
    let review = super::runs::refresh_runs(&app, root.clone(), review)
        .await
        .map_err(|e| error(StatusCode::CONFLICT, e))?;
    if let Err(e) = chat_bridge::flush_read_only(&app, root, &review).await {
        tracing::warn!(error=%e,"review delivery pending");
    }
    Ok(Json(slim(review)))
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
/// Every file of one captured comparison, for browsing the unchanged ones.
async fn candidate_files(
    State(app): State<AppState>,
    Path((id, candidate_id)): Path<(String, String)>,
) -> ApiResult<Vec<refact_review_core::CandidateFile>> {
    let (_, review) = resolve(&app, &id).await?;
    review
        .candidates
        .into_iter()
        .find(|candidate| candidate.id == candidate_id)
        .map(|candidate| Json(candidate.files))
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "candidate not found"))
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
    Ok(Json(slim(updated)))
}
#[derive(Deserialize)]
struct CommentBody {
    #[serde(flatten)]
    request: service::CommentRequest,
    /// Also post this comment to the pull request.
    #[serde(default)]
    publish: bool,
}
async fn comment(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CommentBody>,
) -> ApiResult<Review> {
    let (root, review) = resolve(&app, &id).await?;
    let contribution = service::reply_contribution_id(&id, &body.request.request_key);
    let saved = service::comment(
        app.clone(),
        root.clone(),
        review.owner_chat_id,
        id,
        body.request,
    )
    .await
    .map_err(core_error)?;
    // Questions about a tour page are for the owning chat, never for the pull request.
    let tour_question = saved.threads.iter().any(|thread| {
        thread.target.is_some() && thread.contributions.iter().any(|c| c.id == contribution)
    });
    if !body.publish || tour_question {
        return Ok(Json(slim(saved)));
    }
    let mut target = remote::target_of(&app, root, saved).await;
    // A failed post leaves the comment marked; the next sync sends it.
    match authoring::comments::request(&target, remote::SOURCE, &contribution).await {
        Ok(marked) => {
            target.review = marked;
            remote::sync(&mut target, remote::Direction::PUSH).await;
        }
        Err(e) => tracing::warn!(error=%e, "review comment was not marked for the pull request"),
    }
    chat_bridge::invalidate(&app, &target.review).await;
    Ok(Json(slim(target.review)))
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
    Ok(Json(slim(settled)))
}
#[derive(Deserialize)]
struct OutboxRequest {
    id: String,
    #[serde(default)]
    source: Option<String>,
}
/// The user sends one finding or comment to the PR. The engine posts it to a GitHub pull
/// request itself; for any other source the owning chat is asked to post it.
async fn outbox(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<OutboxRequest>,
) -> ApiResult<Review> {
    let (root, review) = resolve(&app, &id).await?;
    let mut target = remote::target_of(&app, root, review).await;
    let source = request.source.as_deref().unwrap_or(remote::SOURCE);
    target.review = authoring::comments::request(&target, source, &request.id)
        .await
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    let report = remote::sync(&mut target, remote::Direction::PUSH).await;
    chat_bridge::invalidate(&app, &target.review).await;
    if !report.available {
        let key = format!("{}:outbox:{}", target.review.id, request.id);
        if let Err(e) = chat_bridge::notify(
            &app,
            &target.review,
            &key,
            "review.outbox",
            "The user marked a review item for the PR and it could not be posted automatically. Call review_outbox, post what is waiting to the source, then record it as published.",
        )
        .await
        {
            tracing::warn!(error=%e, "review outbox notice pending");
        }
    }
    Ok(Json(slim(target.review)))
}
#[derive(Deserialize, Default)]
struct SyncRequest {
    /// Background refresh from an open canvas: never wakes the chat.
    #[serde(default)]
    quiet: bool,
}
/// Brings new PR comments in and sends out what is marked for the PR.
async fn sync_comments(
    State(app): State<AppState>,
    Path(id): Path<String>,
    request: Option<Json<SyncRequest>>,
) -> ApiResult<remote::SyncReport> {
    let (root, review) = resolve(&app, &id).await?;
    let quiet = request.is_some_and(|Json(request)| request.quiet);
    let mut target = remote::target_of(&app, root, review).await;
    let report = remote::sync(&mut target, remote::Direction::BOTH).await;
    if !report.available && !quiet {
        let key = format!("{}:sync:{}", target.review.id, service::now_ms());
        if let Err(e) = chat_bridge::notify(
            &app,
            &target.review,
            &key,
            "review.sync",
            "The user asked to refresh the PR discussion and it could not be fetched automatically. Fetch the PR's comments and pass them to review_import_comments; then post anything review_outbox lists as waiting.",
        )
        .await
        {
            tracing::warn!(error=%e, "review sync notice pending");
        }
    }
    Ok(Json(report))
}
