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
use crate::buddy::types::{BuddyAction, BuddyDraft, DraftKind, InvestigationContext, OpportunityStatus};
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

#[derive(Debug, Deserialize)]
pub struct OpportunitiesQuery {
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AcceptRequest {
    #[serde(default)]
    pub action_index: usize,
}

pub(crate) struct ActionOutcome {
    pub result: serde_json::Value,
    pub status: OpportunityStatus,
    pub handled: bool,
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
    body: Option<axum::extract::Json<AcceptRequest>>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let req = body.map(|b| b.0).unwrap_or_default();

    let opp = {
        let buddy_arc = gcx.read().await.buddy.clone();
        let lock = buddy_arc.lock().await;
        let svc = lock.as_ref().ok_or_else(|| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "buddy not initialized".into(),
            )
        })?;
        svc.opportunity_queue.get(&id).cloned().ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("opportunity not found: {}", id),
            )
        })?
    };

    let action = opp
        .proposed_actions
        .get(req.action_index)
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::BAD_REQUEST,
                format!("action_index {} out of range", req.action_index),
            )
        })?
        .clone();

    let outcome = dispatch_action(gcx.clone(), &id, &action).await?;

    if !outcome.handled {
        let action_kind = outcome
            .result
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        return Err(ScratchError::new(
            StatusCode::NOT_IMPLEMENTED,
            format!("action_not_implemented: {}", action_kind),
        ));
    }

    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy not initialized".into(),
        )
    })?;
    svc.resolve_opportunity(&id, outcome.status);
    let snap = serde_json::to_value(svc.snapshot())
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(axum::Json(serde_json::json!({
        "snapshot": snap,
        "action_result": outcome.result
    })))
}

pub(crate) async fn dispatch_action(
    gcx: Arc<ARwLock<GlobalContext>>,
    _opp_id: &str,
    action: &BuddyAction,
) -> Result<ActionOutcome, ScratchError> {
    match action {
        BuddyAction::OpenPage { page, .. } => {
            let buddy_arc = gcx.read().await.buddy.clone();
            let lock = buddy_arc.lock().await;
            if let Some(svc) = lock.as_ref() {
                svc.send_navigation(page.clone());
            }
            let nav_page = serde_json::to_value(page)
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(ActionOutcome {
                result: serde_json::json!({
                    "kind": "open_page",
                    "navigate_to": nav_page
                }),
                status: OpportunityStatus::Accepted,
                handled: true,
            })
        }
        BuddyAction::LaunchInvestigationChat { preload } => {
            let chat_id = create_investigation_chat(gcx.clone(), preload).await?;
            Ok(ActionOutcome {
                result: serde_json::json!({
                    "kind": "launch_investigation_chat",
                    "chat_id": chat_id
                }),
                status: OpportunityStatus::Accepted,
                handled: true,
            })
        }
        BuddyAction::DraftSkill { draft_id, label }
        | BuddyAction::DraftCommand { draft_id, label }
        | BuddyAction::DraftSubagent { draft_id, label }
        | BuddyAction::DraftMode { draft_id, label } => {
            let (kind, final_id) = if draft_id.is_empty() {
                let (dk, title, content) = match action {
                    BuddyAction::DraftSkill { .. } => (DraftKind::Skill, label.as_str(), "name: my-skill\ndescription: Describe when to use this skill\ncontext: Add context here"),
                    BuddyAction::DraftCommand { .. } => (DraftKind::Command, label.as_str(), "name: my-command\ndescription: Describe this command"),
                    BuddyAction::DraftSubagent { .. } => (DraftKind::Subagent, label.as_str(), "name: my-subagent\ndescription: Describe this subagent"),
                    _ => (DraftKind::Mode, label.as_str(), "title: My Mode\nprompt: Describe this mode"),
                };
                let draft =
                    synthesize_draft(gcx.clone(), dk, title.to_string(), content.to_string())
                        .await?;
                let id = draft.id.clone();
                (dk, id)
            } else {
                let dk = match action {
                    BuddyAction::DraftSkill { .. } => DraftKind::Skill,
                    BuddyAction::DraftCommand { .. } => DraftKind::Command,
                    BuddyAction::DraftSubagent { .. } => DraftKind::Subagent,
                    _ => DraftKind::Mode,
                };
                (dk, draft_id.clone())
            };
            Ok(ActionOutcome {
                result: serde_json::json!({
                    "kind": "draft",
                    "draft_kind": serde_json::to_value(kind).unwrap_or_default(),
                    "draft_id": final_id,
                    "label": label
                }),
                status: OpportunityStatus::Accepted,
                handled: true,
            })
        }
        BuddyAction::DraftAgentsMdPatch { diff } => {
            let content = if diff.is_empty() {
                "# AGENTS.md\n\nThis file provides guidance to AI agents when working with this repository.\n\n## Development Commands\n\n- **Build**: `make build`\n- **Test**: `make test`\n\n## Architecture\n\nDescribe the project architecture here.\n"
            } else {
                diff.as_str()
            };
            let draft = synthesize_draft(
                gcx.clone(),
                DraftKind::AgentsMd,
                "AGENTS.md".to_string(),
                content.to_string(),
            )
            .await?;
            Ok(ActionOutcome {
                result: serde_json::json!({
                    "kind": "draft",
                    "draft_kind": "agents_md",
                    "draft_id": draft.id
                }),
                status: OpportunityStatus::Accepted,
                handled: true,
            })
        }
        BuddyAction::DraftDefaultsChange {
            defaults_kind,
            patch,
        } => {
            let content = if patch != &serde_json::json!({}) {
                serde_json::to_string_pretty(patch).unwrap_or_default()
            } else {
                "{\n  \"chat_default_model\": \"your-provider/model-name\"\n}".to_string()
            };
            let draft = synthesize_draft(
                gcx.clone(),
                DraftKind::DefaultsModel,
                "Default Models".to_string(),
                content,
            )
            .await?;
            Ok(ActionOutcome {
                result: serde_json::json!({
                    "kind": "draft",
                    "draft_kind": "defaults_model",
                    "defaults_kind": serde_json::to_value(defaults_kind).unwrap_or_default(),
                    "draft_id": draft.id
                }),
                status: OpportunityStatus::Accepted,
                handled: true,
            })
        }
        BuddyAction::Dismiss => Ok(ActionOutcome {
            result: serde_json::json!({ "kind": "dismiss" }),
            status: OpportunityStatus::Dismissed,
            handled: true,
        }),
        _ => {
            let variant_name = match action {
                BuddyAction::DraftCustomizationChange { .. } => "draft_customization_change",
                BuddyAction::CreatePulseReport { .. } => "create_pulse_report",
                BuddyAction::OfferMarketplaceInstall { .. } => "offer_marketplace_install",
                _ => "unknown",
            };
            Ok(ActionOutcome {
                result: serde_json::json!({ "kind": "unimplemented", "action": variant_name }),
                status: OpportunityStatus::Accepted,
                handled: false,
            })
        }
    }
}

async fn synthesize_draft(
    gcx: Arc<ARwLock<GlobalContext>>,
    kind: DraftKind,
    title: String,
    content: String,
) -> Result<BuddyDraft, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy not initialized".into(),
        )
    })?;
    let draft = svc.draft_store.create(kind, title, content, String::new());
    let _ = svc.events_tx.send(BuddyEvent::DraftCreated {
        draft: draft.clone(),
    });
    Ok(draft)
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
    let mut parts =
        vec!["You are investigating a technical issue. Here is the context:\n".to_string()];
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
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy not initialized".into(),
        )
    })?;
    if svc.opportunity_queue.get(&id).is_none() {
        return Err(ScratchError::new(
            StatusCode::NOT_FOUND,
            format!("opportunity not found: {}", id),
        ));
    }
    svc.resolve_opportunity(&id, OpportunityStatus::Dismissed);
    let snap = serde_json::to_value(svc.snapshot())
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::Json(serde_json::json!({ "snapshot": snap })))
}
