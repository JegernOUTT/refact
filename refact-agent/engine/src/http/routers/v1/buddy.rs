use axum::Extension;
use axum::extract::Path;
use axum::response::Result;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::buddy::diagnostics::DiagnosticContext;
use crate::buddy::events::BuddyEvent;
use crate::buddy::settings::MAX_PALETTE_INDEX;
use crate::buddy::types::{BuddyActivity, BuddyConversationEntry};
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

#[derive(Debug, Deserialize)]
pub struct BuddySettingsRequest {
    pub enabled: Option<bool>,
    pub auto_diagnostics: Option<bool>,
    pub auto_issue_creation: Option<bool>,
    pub personality_prompt: Option<String>,
    pub palette_index: Option<usize>,
}

pub async fn handle_v1_buddy_settings_update(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<BuddySettingsRequest>,
) -> Result<StatusCode, ScratchError> {
    if let Some(pi) = req.palette_index {
        if pi > MAX_PALETTE_INDEX {
            return Err(ScratchError::new(StatusCode::BAD_REQUEST, "palette_index must be 0-7".to_string()));
        }
    }

    let project_root = crate::files_correction::get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "no project root".to_string()))?;

    let buddy_arc = gcx.read().await.buddy.clone();
    let events_tx = {
        let mut lock = buddy_arc.lock().await;
        if let Some(service) = lock.as_mut() {
            if let Some(v) = req.enabled { service.settings.enabled = v; }
            if let Some(v) = req.auto_diagnostics { service.settings.auto_diagnostics = v; }
            if let Some(v) = req.auto_issue_creation { service.settings.auto_issue_creation = v; }
            if req.personality_prompt.is_some() { service.settings.personality_prompt = req.personality_prompt.clone(); }
            if let Some(pi) = req.palette_index {
                service.state.identity.palette_index = pi;
                service.dirty = true;
            }
            Some((service.settings.clone(), service.events_tx.clone()))
        } else {
            None
        }
    };
    if let Some((new_settings, tx)) = events_tx {
        crate::buddy::settings::save_settings(&project_root, &new_settings)
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
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

#[derive(Debug, Deserialize)]
pub struct ConversationsListQuery {
    pub kind: Option<String>,
}

pub async fn handle_v1_buddy_conversations_list(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::extract::Query(query): axum::extract::Query<ConversationsListQuery>,
) -> Result<axum::Json<Vec<BuddyConversationEntry>>, ScratchError> {
    let project_root = crate::files_correction::get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "no project root".to_string()))?;

    let kind_filter = query.kind.map(|k| k.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>());
    let entries = crate::buddy::conversation_ledger::list_all_buddy_conversations(&project_root, kind_filter).await;
    Ok(axum::Json(entries))
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

    let meta = BuddyConversationMeta {
        chat_id,
        title: "New Conversation".to_string(),
        created_at,
        last_message_at: None,
        message_count: 0,
    };
    Ok(axum::Json(serde_json::to_value(meta).map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?))
}

#[derive(Debug, Deserialize)]
pub struct CreateSetupRequest {
    pub flow: String,
    pub title: Option<String>,
}

pub async fn handle_v1_buddy_conversations_create_setup(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<CreateSetupRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let project_root = crate::files_correction::get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "no project root".to_string()))?;

    let valid_flows = ["setup", "setup_mcp", "setup_skills", "setup_commands", "setup_agents_md", "setup_subagents"];
    if !valid_flows.contains(&req.flow.as_str()) {
        return Err(ScratchError::new(StatusCode::BAD_REQUEST, format!("unknown flow: {}", req.flow)));
    }

    let chat_id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let title = req.title.unwrap_or_else(|| {
        match req.flow.as_str() {
            "setup_mcp" => "MCP Setup".to_string(),
            "setup_skills" => "Skills Setup".to_string(),
            "setup_commands" => "Commands Setup".to_string(),
            "setup_agents_md" => "AGENTS.md Setup".to_string(),
            "setup_subagents" => "Subagents Setup".to_string(),
            _ => "Project Setup".to_string(),
        }
    });
    let badge = match req.flow.as_str() {
        "setup_mcp" => "MCP Setup",
        "setup_skills" => "Skills",
        "setup_commands" => "Commands",
        "setup_agents_md" => "AGENTS.md",
        "setup_subagents" => "Subagents",
        _ => "Setup",
    };
    let conv = serde_json::json!({
        "chat_id": chat_id,
        "title": title,
        "kind": "setup",
        "flow": req.flow,
        "badge": badge,
        "created_at": created_at,
        "last_message_at": null,
        "messages": []
    });

    let path = project_root.join(format!(".refact/buddy/chats/conversations/{}.json", chat_id));
    crate::buddy::storage::atomic_write_json(&path, &conv)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(axum::Json(serde_json::json!({
        "chat_id": chat_id,
        "title": title,
        "kind": "setup",
        "flow": req.flow,
        "badge": badge,
        "created_at": created_at
    })))
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
    pub manual: Option<bool>,
}

pub async fn handle_v1_buddy_issues_create(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<IssueCreateRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let pre_diag = if req.diagnostic_index.is_none() {
        match &req.error {
            Some(err) => Some(crate::buddy::diagnostics::collect_diagnostics(gcx.clone(), err).await),
            None => None,
        }
    } else {
        None
    };

    let (ctx, auto_enabled, last_issue_at, recent_errors) = {
        let buddy_arc = gcx.read().await.buddy.clone();
        let lock = buddy_arc.lock().await;
        let svc = lock.as_ref().ok_or_else(|| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "buddy service not initialized".to_string()))?;

        let ctx = if let Some(idx) = req.diagnostic_index {
            svc.recent_diagnostics.get(idx).cloned().ok_or_else(|| ScratchError::new(StatusCode::BAD_REQUEST, "diagnostic index out of range".to_string()))?
        } else if let Some(diagnosed) = pre_diag {
            diagnosed
        } else {
            return Err(ScratchError::new(StatusCode::BAD_REQUEST, "provide diagnostic_index or error".to_string()));
        };

        (ctx, svc.settings.auto_issue_creation, svc.last_issue_at, svc.recent_issue_errors.clone())
    };

    let manual = req.manual.unwrap_or(false);
    let result = crate::buddy::issues::create_issue(
        gcx.clone(), &ctx, auto_enabled, manual, last_issue_at, &recent_errors,
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
