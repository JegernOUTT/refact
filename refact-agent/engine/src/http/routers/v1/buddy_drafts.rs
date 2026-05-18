use axum::extract::Path;
use axum::response::Result;
use axum::extract::State;
use hyper::StatusCode;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use crate::buddy::drafts::DraftCreateError;
use crate::buddy::types::{BuddyDraft, DraftKind};
use crate::custom_error::ScratchError;

#[derive(Debug, Deserialize)]
pub struct DraftCreateRequest {
    pub title: String,
    pub yaml_or_json: String,
    pub explanation: String,
}

fn draft_create_error(err: DraftCreateError) -> ScratchError {
    ScratchError::new(StatusCode::PAYLOAD_TOO_LARGE, err.to_string())
}

pub async fn handle_v1_buddy_draft_create_skill(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let gcx = app.gcx.clone();
    create_draft(gcx, req, DraftKind::Skill).await
}

pub async fn handle_v1_buddy_draft_create_command(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let gcx = app.gcx.clone();
    create_draft(gcx, req, DraftKind::Command).await
}

pub async fn handle_v1_buddy_draft_create_subagent(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let gcx = app.gcx.clone();
    create_draft(gcx, req, DraftKind::Delegate).await
}

pub async fn handle_v1_buddy_draft_create_mode(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let gcx = app.gcx.clone();
    create_draft(gcx, req, DraftKind::Mode).await
}

pub async fn handle_v1_buddy_draft_create_agents_md(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let gcx = app.gcx.clone();
    create_draft(gcx, req, DraftKind::AgentsMd).await
}

pub async fn handle_v1_buddy_draft_create_defaults(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let gcx = app.gcx.clone();
    create_draft(gcx, req, DraftKind::DefaultsModel).await
}

pub async fn handle_v1_buddy_draft_create_hook(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let gcx = app.gcx.clone();
    create_draft(gcx, req, DraftKind::Hook).await
}

pub async fn handle_v1_buddy_draft_create_pulse_report(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<DraftCreateRequest>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let gcx = app.gcx.clone();
    create_draft(gcx, req, DraftKind::PulseReport).await
}

async fn create_draft(
    gcx: Arc<GlobalContext>,
    req: DraftCreateRequest,
    kind: DraftKind,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let buddy_arc = gcx.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy not initialized".into(),
        )
    })?;
    let draft = svc
        .create_draft(kind, req.title, req.yaml_or_json, req.explanation)
        .map_err(draft_create_error)?;
    Ok(axum::Json(draft))
}

pub async fn handle_v1_buddy_draft_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<axum::Json<BuddyDraft>, ScratchError> {
    let gcx = app.gcx.clone();
    let buddy_arc = gcx.buddy.clone();
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
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let gcx = app.gcx.clone();
    let buddy_arc = gcx.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy not initialized".into(),
        )
    })?;
    let deleted = svc.delete_draft(&id).is_some();
    Ok(axum::Json(
        serde_json::json!({ "ok": true, "deleted": deleted }),
    ))
}
