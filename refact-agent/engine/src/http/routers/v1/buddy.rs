use axum::Extension;
use axum::extract::Path;
use axum::response::Result;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::buddy::diagnostics::DiagnosticContext;
use crate::buddy::events::BuddyEvent;
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::BuddyActivity;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyConversationMeta {
    pub chat_id: String,
    pub title: String,
    pub created_at: String,
    pub last_message_at: Option<String>,
    pub message_count: usize,
}

pub async fn handle_v1_buddy_snapshot(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    match lock.as_ref() {
        Some(service) => Ok(axum::Json(serde_json::to_value(service.snapshot()).map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?)),
        None => Ok(axum::Json(serde_json::json!({"enabled": false}))),
    }
}

pub async fn handle_v1_buddy_settings_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    match lock.as_ref() {
        Some(service) => Ok(axum::Json(serde_json::to_value(&service.settings).map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?)),
        None => Ok(axum::Json(serde_json::json!({"enabled": false}))),
    }
}

pub async fn handle_v1_buddy_settings_update(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(new_settings): axum::Json<BuddySettings>,
) -> Result<StatusCode, ScratchError> {
    let project_root = crate::files_correction::get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "no project root".to_string()))?;

    crate::buddy::settings::save_settings(&project_root, &new_settings)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let buddy_arc = gcx.read().await.buddy.clone();
    let events_tx = {
        let mut lock = buddy_arc.lock().await;
        if let Some(service) = lock.as_mut() {
            service.settings = new_settings.clone();
            Some(service.events_tx.clone())
        } else {
            None
        }
    };
    if let Some(tx) = events_tx {
        let _ = tx.send(BuddyEvent::SettingsChanged { settings: new_settings });
    }

    Ok(StatusCode::OK)
}

pub async fn handle_v1_buddy_activities(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<Vec<BuddyActivity>>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    let activities = lock
        .as_ref()
        .map(|s| s.state.recent_activities.clone())
        .unwrap_or_default();
    Ok(axum::Json(activities))
}

pub async fn handle_v1_buddy_conversations_list(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<Vec<BuddyConversationMeta>>, ScratchError> {
    let project_root = crate::files_correction::get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "no project root".to_string()))?;

    let paths = crate::buddy::storage::list_buddy_conversations(&project_root).await;
    let mut metas = Vec::new();
    for path in paths {
        let chat_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let content = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
        let title = val.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string();
        let created_at = val.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let last_message_at = val.get("last_message_at").and_then(|v| v.as_str()).map(|s| s.to_string());
        let message_count = val.get("messages").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        metas.push(BuddyConversationMeta { chat_id, title, created_at, last_message_at, message_count });
    }
    Ok(axum::Json(metas))
}

pub async fn handle_v1_buddy_conversations_create(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let project_root = crate::files_correction::get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "no project root".to_string()))?;

    let chat_id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let conv = serde_json::json!({
        "chat_id": chat_id,
        "title": "New Conversation",
        "created_at": created_at,
        "last_message_at": null,
        "messages": []
    });

    let path = project_root.join(format!(".refact/buddy/chats/conversations/{}.json", chat_id));
    crate::buddy::storage::atomic_write_json(&path, &conv)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(axum::Json(serde_json::json!({"chat_id": chat_id})))
}

pub async fn handle_v1_buddy_suggestion_dismiss(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    match lock.as_mut() {
        Some(service) => {
            service.dismiss_suggestion(&id);
            Ok(StatusCode::OK)
        }
        None => Err(ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "buddy service not initialized".to_string())),
    }
}

#[derive(Debug, Deserialize)]
pub struct DiagnosticsCollectRequest {
    pub error: String,
    pub source_file: Option<String>,
    pub tool_name: Option<String>,
    pub chat_id: Option<String>,
}

pub async fn handle_v1_buddy_diagnostics_collect(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<DiagnosticsCollectRequest>,
) -> Result<axum::Json<DiagnosticContext>, ScratchError> {
    let mut ctx = crate::buddy::diagnostics::collect_diagnostics(gcx.clone(), &req.error).await;
    ctx.source_file = req.source_file;
    ctx.tool_name = req.tool_name;
    ctx.chat_id = req.chat_id;

    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.add_diagnostic(ctx.clone());
    }

    Ok(axum::Json(ctx))
}

pub async fn handle_v1_buddy_diagnostics_list(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<Vec<DiagnosticContext>>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    let diags = lock
        .as_ref()
        .map(|s| s.recent_diagnostics.clone())
        .unwrap_or_default();
    Ok(axum::Json(diags))
}

#[derive(Debug, Deserialize)]
pub struct IssueCreateRequest {
    pub diagnostic_index: Option<usize>,
    pub error: Option<String>,
}

pub async fn handle_v1_buddy_issues_create(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<IssueCreateRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let (ctx, auto_enabled, last_issue_at, recent_errors) = {
        let buddy_arc = gcx.read().await.buddy.clone();
        let lock = buddy_arc.lock().await;
        let svc = lock.as_ref().ok_or_else(|| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "buddy service not initialized".to_string()))?;

        let ctx = if let Some(idx) = req.diagnostic_index {
            svc.recent_diagnostics.get(idx).cloned().ok_or_else(|| ScratchError::new(StatusCode::BAD_REQUEST, "diagnostic index out of range".to_string()))?
        } else if let Some(ref err) = req.error {
            let mut c = crate::buddy::diagnostics::DiagnosticContext {
                error_type: "generic".to_string(),
                error_message: err.clone(),
                source_file: None,
                tool_name: None,
                chat_id: None,
                collected_at: chrono::Utc::now().to_rfc3339(),
                severity: crate::buddy::diagnostics::DiagnosticSeverity::Medium,
            };
            c.error_type = crate::buddy::diagnostics::collect_diagnostics(gcx.clone(), err).await.error_type;
            c
        } else {
            return Err(ScratchError::new(StatusCode::BAD_REQUEST, "provide diagnostic_index or error".to_string()));
        };

        (ctx, svc.settings.auto_issue_creation, svc.last_issue_at, svc.recent_issue_errors.clone())
    };

    let result = crate::buddy::issues::create_issue(
        gcx.clone(), &ctx, auto_enabled, last_issue_at, &recent_errors,
    ).await.map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    let (url, activity) = result;

    {
        let buddy_arc = gcx.read().await.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        if let Some(svc) = lock.as_mut() {
            svc.record_issue_created(ctx.error_message.clone());
            svc.add_activity(activity);
        }
    }

    Ok(axum::Json(serde_json::json!({"url": url})))
}
