use axum::Extension;
use axum::extract::Path;
use axum::response::Result;
use hyper::StatusCode;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::buddy::types::{BuddyDraft, DraftKind};
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

#[derive(Debug, Deserialize)]
pub struct DraftCreateRequest {
    pub title: String,
    pub yaml_or_json: String,
    pub explanation: String,
}

pub async fn handle_v1_buddy_draft_create_skill(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    create_draft(gcx, req, DraftKind::Skill).await
}

pub async fn handle_v1_buddy_draft_create_command(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    create_draft(gcx, req, DraftKind::Command).await
}

pub async fn handle_v1_buddy_draft_create_subagent(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    create_draft(gcx, req, DraftKind::Delegate).await
}

pub async fn handle_v1_buddy_draft_create_mode(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    create_draft(gcx, req, DraftKind::Mode).await
}

pub async fn handle_v1_buddy_draft_create_agents_md(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    create_draft(gcx, req, DraftKind::AgentsMd).await
}

pub async fn handle_v1_buddy_draft_create_defaults(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    create_draft(gcx, req, DraftKind::DefaultsModel).await
}

pub async fn handle_v1_buddy_draft_create_hook(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    create_draft(gcx, req, DraftKind::Hook).await
}

pub async fn handle_v1_buddy_draft_create_pulse_report(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    create_draft(gcx, req, DraftKind::PulseReport).await
}

async fn create_draft(
    gcx: Arc<ARwLock<GlobalContext>>,
    req: DraftCreateRequest,
    kind: DraftKind,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy not initialized".into(),
        )
    })?;
    let draft = svc
        .draft_store
        .create(kind, req.title, req.yaml_or_json, req.explanation);
    svc.add_draft(draft.clone());
    Ok(axum::Json(draft))
}

pub async fn handle_v1_buddy_draft_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    let svc = lock.as_ref().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy not initialized".into(),
        )
    })?;
    let draft = svc.draft_store.get(&id).cloned().ok_or_else(|| {
        ScratchError::new(StatusCode::NOT_FOUND, format!("draft not found: {}", id))
    })?;
    Ok(axum::Json(draft))
}

pub async fn handle_v1_buddy_draft_delete(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy not initialized".into(),
        )
    })?;
    svc.draft_store.delete(&id);
    Ok(axum::Json(serde_json::json!({ "ok": true })))
}
