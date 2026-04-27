use axum::Extension;
use axum::extract::Path;
use axum::extract::Query;
use axum::response::Result;
use hyper::StatusCode;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use uuid::Uuid;

use crate::buddy::events::BuddyEvent;
use crate::buddy::types::{
    BuddyAction, InvestigationContext, OpportunityStatus,
};
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

#[derive(Debug, Deserialize)]
pub struct OpportunitiesQuery {
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AcceptRequest {
    pub action_index: usize,
    pub params: Option<serde_json::Value>,
}

pub async fn handle_v1_buddy_opportunities_list(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(query): Query<OpportunitiesQuery>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    let mut opps = match lock.as_ref() {
        Some(svc) => svc.opportunity_queue.snapshot(),
        None => vec![],
    };
    if let Some(status_filter) = &query.status {
        let allowed: Vec<&str> = status_filter.split(',').map(|s| s.trim()).collect();
        opps.retain(|o| {
            let s = serde_json::to_value(&o.status)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            allowed.contains(&s.as_str())
        });
    }
    Ok(axum::Json(serde_json::json!({ "opportunities": opps })))
}

pub async fn handle_v1_buddy_opportunity_accept(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
    axum::Json(req): axum::Json<AcceptRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let opp = {
        let buddy_arc = gcx.read().await.buddy.clone();
        let lock = buddy_arc.lock().await;
        let svc = lock.as_ref().ok_or_else(|| {
            ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "buddy not initialized".into())
        })?;
        svc.opportunity_queue
            .get(&id)
            .cloned()
            .ok_or_else(|| ScratchError::new(StatusCode::NOT_FOUND, format!("opportunity not found: {}", id)))?
    };

    let action = opp.proposed_actions.get(req.action_index).ok_or_else(|| {
        ScratchError::new(StatusCode::BAD_REQUEST, format!("action_index {} out of range", req.action_index))
    })?.clone();

    let action_result = dispatch_action(gcx.clone(), &id, &action).await?;

    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "buddy not initialized".into())
    })?;
    svc.resolve_opportunity(&id, OpportunityStatus::Accepted);
    let snap = serde_json::to_value(svc.snapshot())
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(axum::Json(serde_json::json!({
        "snapshot": snap,
        "action_result": action_result
    })))
}

async fn dispatch_action(
    gcx: Arc<ARwLock<GlobalContext>>,
    opp_id: &str,
    action: &BuddyAction,
) -> Result<serde_json::Value, ScratchError> {
    match action {
        BuddyAction::OpenPage { page, .. } => {
            let buddy_arc = gcx.read().await.buddy.clone();
            let lock = buddy_arc.lock().await;
            if let Some(svc) = lock.as_ref() {
                svc.send_navigation(page.clone());
            }
            let nav_page = serde_json::to_value(page)
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(serde_json::json!({
                "kind": "open_page",
                "navigate_to": nav_page
            }))
        }
        BuddyAction::LaunchInvestigationChat { preload } => {
            let chat_id = create_investigation_chat(gcx.clone(), preload).await?;
            Ok(serde_json::json!({
                "kind": "launch_investigation_chat",
                "chat_id": chat_id
            }))
        }
        BuddyAction::DraftSkill { draft_id, label } |
        BuddyAction::DraftCommand { draft_id, label } |
        BuddyAction::DraftSubagent { draft_id, label } |
        BuddyAction::DraftMode { draft_id, label } => {
            Ok(serde_json::json!({
                "kind": "draft",
                "draft_id": draft_id,
                "label": label
            }))
        }
        BuddyAction::Dismiss => {
            let buddy_arc = gcx.read().await.buddy.clone();
            let mut lock = buddy_arc.lock().await;
            if let Some(svc) = lock.as_mut() {
                svc.resolve_opportunity(opp_id, OpportunityStatus::Dismissed);
            }
            Ok(serde_json::json!({ "kind": "dismiss" }))
        }
        _ => Ok(serde_json::json!({ "kind": "no_op" })),
    }
}

async fn create_investigation_chat(
    gcx: Arc<ARwLock<GlobalContext>>,
    ctx: &InvestigationContext,
) -> Result<String, ScratchError> {
    let chat_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let system_msg = build_investigation_system_message(ctx);

    let snapshot = crate::chat::trajectories::TrajectorySnapshot {
        chat_id: chat_id.clone(),
        title: "Investigation".to_string(),
        model: String::new(),
        mode: "buddy".to_string(),
        tool_use: "agent".to_string(),
        messages: vec![
            crate::call_validation::ChatMessage {
                role: "system".to_string(),
                content: crate::call_validation::ChatContent::SimpleText(system_msg),
                ..Default::default()
            },
            crate::call_validation::ChatMessage {
                role: "user".to_string(),
                content: crate::call_validation::ChatContent::SimpleText(
                    ctx.initial_user_message.clone(),
                ),
                ..Default::default()
            },
        ],
        created_at: now,
        boost_reasoning: false,
        checkpoints_enabled: false,
        context_tokens_cap: None,
        include_project_info: true,
        is_title_generated: false,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
        version: 1,
        task_meta: None,
        parent_id: None,
        link_type: None,
        root_chat_id: None,
        reasoning_effort: None,
        thinking_budget: None,
        temperature: None,
        frequency_penalty: None,
        max_tokens: None,
        parallel_tool_calls: None,
        previous_response_id: None,
        active_skill: None,
        auto_enrichment_enabled: Some(true),
        buddy_meta: Some(crate::buddy::types::BuddyThreadMeta {
            is_buddy_chat: true,
            buddy_chat_kind: "investigation".to_string(),
            workflow_id: None,
        }),
    };

    crate::chat::trajectories::save_trajectory_snapshot(gcx, snapshot)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(chat_id)
}

fn build_investigation_system_message(ctx: &InvestigationContext) -> String {
    let mut parts = vec!["You are investigating a technical issue. Here is the context:\n".to_string()];
    if !ctx.fact_keys.is_empty() {
        parts.push(format!("Fact keys: {}", ctx.fact_keys.join(", ")));
    }
    if !ctx.diagnostic_ids.is_empty() {
        parts.push(format!("Diagnostic IDs: {}", ctx.diagnostic_ids.join(", ")));
    }
    if !ctx.log_excerpt.is_empty() {
        parts.push(format!("Log excerpt:\n{}", ctx.log_excerpt));
    }
    if !ctx.config_summary.is_empty() {
        parts.push(format!("Config summary:\n{}", ctx.config_summary));
    }
    parts.join("\n")
}

pub async fn handle_v1_buddy_opportunity_dismiss(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, "buddy not initialized".into())
    })?;
    if svc.opportunity_queue.get(&id).is_none() {
        return Err(ScratchError::new(StatusCode::NOT_FOUND, format!("opportunity not found: {}", id)));
    }
    svc.opportunity_queue.dismiss(&id);
    let _ = svc.events_tx.send(BuddyEvent::OpportunityResolved {
        opportunity_id: id,
        status: OpportunityStatus::Dismissed,
    });
    svc.state.opportunities = svc.opportunity_queue.snapshot();
    svc.dirty = true;
    let snap = serde_json::to_value(svc.snapshot())
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::Json(serde_json::json!({ "snapshot": snap })))
}
