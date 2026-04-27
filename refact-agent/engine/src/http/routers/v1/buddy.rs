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
use crate::buddy::types::{BuddyActivity, BuddyCareAction, BuddyConversationEntry, BuddySuggestion};
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

#[derive(Debug, Deserialize)]
pub struct BuddyConversationCreateRequest {
    pub title: Option<String>,
}

pub async fn handle_v1_buddy_snapshot(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    match lock.as_ref() {
        Some(service) => Ok(axum::Json(
            serde_json::to_value(service.snapshot())
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
        )),
        None => Ok(axum::Json(serde_json::json!({
            "enabled": false,
            "state": crate::buddy::state::default_buddy_state(),
            "settings": crate::buddy::settings::BuddySettings::default(),
            "recent_diagnostics": [],
            "runtime_queue": [],
            "now_playing": null,
            "active_speech": null
        }))),
    }
}

pub async fn handle_v1_buddy_settings_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    match lock.as_ref() {
        Some(service) => Ok(axum::Json(
            serde_json::to_value(&service.settings)
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
        )),
        None => Ok(axum::Json(
            serde_json::to_value(crate::buddy::settings::BuddySettings::default()).map_err(
                |e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            )?,
        )),
    }
}

#[derive(Debug, Deserialize)]
pub struct BuddySettingsRequest {
    pub enabled: Option<bool>,
    pub auto_diagnostics: Option<bool>,
    pub auto_issue_creation: Option<bool>,
    pub personality_prompt: Option<String>,
    pub clear_personality_prompt: Option<bool>,
    pub proactive_enabled: Option<bool>,
    pub palette_index: Option<usize>,
}

pub async fn handle_v1_buddy_settings_update(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<BuddySettingsRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    if let Some(pi) = req.palette_index {
        if pi > MAX_PALETTE_INDEX {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                "palette_index must be 0-7".to_string(),
            ));
        }
    }

    let project_root = crate::files_correction::get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "no project root".to_string(),
            )
        })?;

    let buddy_arc = gcx.read().await.buddy.clone();
    let events_tx = {
        let mut lock = buddy_arc.lock().await;
        if let Some(service) = lock.as_mut() {
            if let Some(v) = req.enabled {
                service.settings.enabled = v;
            }
            if let Some(v) = req.auto_diagnostics {
                service.settings.auto_diagnostics = v;
            }
            if let Some(v) = req.auto_issue_creation {
                service.settings.auto_issue_creation = v;
            }
            if req.clear_personality_prompt.unwrap_or(false) {
                service.settings.personality_prompt = None;
            } else if let Some(prompt) = &req.personality_prompt {
                service.settings.personality_prompt = Some(prompt.clone());
            }
            if let Some(v) = req.proactive_enabled {
                service.settings.proactive_enabled = v;
            }
            if let Some(pi) = req.palette_index {
                service.state.identity.palette_index = pi;
                crate::buddy::state::sync_state(&mut service.state);
                service.dirty = true;
                let _ = service.events_tx.send(BuddyEvent::StateUpdated {
                    state: service.state.clone(),
                });
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
        let _ = tx.send(BuddyEvent::SettingsChanged {
            settings: new_settings.clone(),
        });
        return Ok(axum::Json(
            serde_json::to_value(new_settings).map_err(|e| {
                ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            })?,
        ));
    }

    Ok(axum::Json(
        serde_json::to_value(crate::buddy::settings::BuddySettings::default()).map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct BuddyCareRequest {
    pub action: BuddyCareAction,
    pub toy: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BuddyQuestAcceptRequest {
    pub suggestion_id: String,
}

pub async fn handle_v1_buddy_care(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<BuddyCareRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy service not initialized".to_string(),
        )
    })?;

    let message = svc.apply_care_action(req.action.clone(), req.toy.as_deref());
    let signal_type = format!("care_{}", req.action.as_str());
    let dedupe_key = format!("care_{}", req.action.as_str());
    svc.enqueue_runtime_event(crate::buddy::actor::make_runtime_event(
        &signal_type,
        &message,
        "buddy",
        &dedupe_key,
        "info",
        None,
    ));
    svc.update_speech(crate::buddy::types::BuddySpeechItem {
        id: uuid::Uuid::new_v4().to_string(),
        text: message.clone(),
        mood: "neutral".to_string(),
        scope: "global".to_string(),
        persistent: false,
        ttl_seconds: 8,
        dedupe_key: Some(format!("care_{}", req.action.as_str())),
        created_at: chrono::Utc::now().to_rfc3339(),
        controls: vec![],
        chat_id: None,
    });

    Ok(axum::Json(serde_json::json!({
        "message": message,
        "snapshot": svc.snapshot()
    })))
}

pub async fn handle_v1_buddy_personality_reroll(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy service not initialized".to_string(),
        )
    })?;

    svc.reroll_personality();
    svc.update_speech(crate::buddy::types::BuddySpeechItem {
        id: uuid::Uuid::new_v4().to_string(),
        text: format!(
            "New vibe loaded: {} — {}",
            svc.state.personality.archetype_label, svc.state.personality.vibe
        ),
        mood: "happy".to_string(),
        scope: "global".to_string(),
        persistent: false,
        ttl_seconds: 10,
        dedupe_key: Some("personality_reroll".to_string()),
        created_at: chrono::Utc::now().to_rfc3339(),
        controls: vec![],
        chat_id: None,
    });

    Ok(axum::Json(serde_json::json!({
        "snapshot": svc.snapshot()
    })))
}

pub async fn handle_v1_buddy_quest_dismiss(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<StatusCode, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    match lock.as_mut() {
        Some(svc) => {
            svc.dismiss_quest();
            Ok(StatusCode::OK)
        }
        None => Err(ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy service not initialized".to_string(),
        )),
    }
}

pub async fn handle_v1_buddy_quest_accept(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<BuddyQuestAcceptRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy service not initialized".to_string(),
        )
    })?;

    let suggestion = svc
        .state
        .suggestion_state
        .iter()
        .find(|suggestion| suggestion.id == req.suggestion_id)
        .cloned()
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("suggestion not found: {}", req.suggestion_id),
            )
        })?;

    let quest = suggestion.quest.clone().ok_or_else(|| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("suggestion is not a quest: {}", req.suggestion_id),
        )
    })?;

    svc.accept_quest(quest);
    svc.dismiss_suggestion(&req.suggestion_id);

    Ok(axum::Json(serde_json::json!({
        "snapshot": svc.snapshot(),
        "suggestion": serde_json::to_value::<BuddySuggestion>(suggestion)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    })))
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
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "no project root".to_string(),
            )
        })?;

    let kind_filter = query.kind.map(|k| {
        k.split(',')
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>()
    });
    let entries =
        crate::buddy::conversation_ledger::list_all_buddy_conversations(&project_root, kind_filter)
            .await;
    Ok(axum::Json(entries))
}

pub async fn handle_v1_buddy_conversations_create(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: axum::body::Bytes,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let chat_id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let body = if body_bytes.is_empty() {
        BuddyConversationCreateRequest { title: None }
    } else {
        serde_json::from_slice::<BuddyConversationCreateRequest>(&body_bytes).map_err(|e| {
            ScratchError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("JSON problem: {}", e),
            )
        })?
    };
    let title = body
        .title
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "New Conversation".to_string());

    let snapshot = crate::chat::trajectories::TrajectorySnapshot {
        chat_id: chat_id.clone(),
        title: title.clone(),
        model: String::new(),
        mode: "buddy".to_string(),
        tool_use: "agent".to_string(),
        messages: vec![],
        created_at: created_at.clone(),
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
            buddy_chat_kind: "conversation".to_string(),
            workflow_id: None,
        }),
    };

    crate::chat::trajectories::save_trajectory_snapshot(gcx.clone(), snapshot)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let meta = BuddyConversationMeta {
        chat_id,
        title,
        created_at,
        last_message_at: None,
        message_count: 0,
    };
    Ok(axum::Json(serde_json::to_value(meta).map_err(|e| {
        ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?))
}

#[derive(Debug, Serialize)]
pub struct BuddyInvestigationContextResponse {
    pub logs: String,
    pub internal_context: String,
    pub repo_owner: String,
    pub repo_name: String,
}

pub async fn handle_v1_buddy_investigation_context(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<DiagnosticsCollectRequest>,
) -> Result<axum::Json<BuddyInvestigationContextResponse>, ScratchError> {
    let log_lines = crate::buddy::issues::investigation_logs(
        gcx.clone(),
        &req.error,
        req.collected_at.as_deref(),
    )
        .await
        .unwrap_or_else(|e| format!("Investigation logs unavailable: {}", e));
    let internal = crate::buddy::issues::investigation_internal_context(gcx.clone())
        .await
        .unwrap_or_else(|e| format!("Investigation context unavailable: {}", e));

    Ok(axum::Json(BuddyInvestigationContextResponse {
        logs: log_lines,
        internal_context: internal,
        repo_owner: "smallcloudai".to_string(),
        repo_name: "refact".to_string(),
    }))
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
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "no project root".to_string(),
            )
        })?;

    let valid_flows = [
        "setup",
        "setup_mcp",
        "setup_skills",
        "setup_commands",
        "setup_agents_md",
        "setup_subagents",
    ];
    if !valid_flows.contains(&req.flow.as_str()) {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("unknown flow: {}", req.flow),
        ));
    }

    let chat_id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let title = req.title.unwrap_or_else(|| match req.flow.as_str() {
        "setup_mcp" => "MCP Setup".to_string(),
        "setup_skills" => "Skills Setup".to_string(),
        "setup_commands" => "Commands Setup".to_string(),
        "setup_agents_md" => "AGENTS.md Setup".to_string(),
        "setup_subagents" => "Subagents Setup".to_string(),
        _ => "Project Setup".to_string(),
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

    let path = project_root.join(format!(
        ".refact/buddy/chats/conversations/{}.json",
        chat_id
    ));
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
        None => Err(ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy service not initialized".to_string(),
        )),
    }
}

/// Dismiss a runtime event (e.g. a frontend window_error) by its id.
/// Persists `dismissed: true` on the event so it stays hidden across reloads.
pub async fn handle_v1_buddy_runtime_dismiss(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(id): Path<String>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    match lock.as_mut() {
        Some(service) => {
            let dismissed = service.dismiss_runtime_event_by_id(&id);
            Ok(axum::Json(serde_json::json!({ "dismissed": dismissed })))
        }
        None => Err(ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy service not initialized".to_string(),
        )),
    }
}

#[derive(Debug, Deserialize)]
pub struct DiagnosticsCollectRequest {
    pub error: String,
    pub source_file: Option<String>,
    pub tool_name: Option<String>,
    pub chat_id: Option<String>,
    #[allow(dead_code)]
    pub diagnostic_id: Option<String>,
    pub collected_at: Option<String>,
}

pub async fn handle_v1_buddy_diagnostics_collect(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<DiagnosticsCollectRequest>,
) -> Result<axum::Json<DiagnosticContext>, ScratchError> {
    let mut ctx = crate::buddy::diagnostics::collect_diagnostics(gcx.clone(), &req.error).await;
    ctx.source_file = req.source_file;
    ctx.tool_name = req.tool_name;
    ctx.chat_id = req.chat_id;
    ctx.collected_at = req.collected_at.unwrap_or(ctx.collected_at);

    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(svc) = lock.as_mut() {
        svc.add_diagnostic(ctx.clone());
    }

    let mut payload = serde_json::to_value(&ctx)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if let serde_json::Value::Object(map) = &mut payload {
        map.insert(
            "diagnostic_id".to_string(),
            serde_json::json!(crate::buddy::diagnostics::diagnostic_id(&ctx)),
        );
    }
    let ctx: DiagnosticContext = serde_json::from_value(payload)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(axum::Json(ctx))
}

pub async fn handle_v1_buddy_diagnostics_list(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<axum::Json<Vec<DiagnosticContext>>, ScratchError> {
    let project_root = crate::buddy::actor::latest_project_root(gcx.clone())
        .await
        .map_err(|e| ScratchError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
    let diags = crate::buddy::storage::load_recent_diagnostics(&project_root, 100)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(axum::Json(diags))
}

#[derive(Debug, Deserialize)]
pub struct IssueCreateRequest {
    pub diagnostic_index: Option<usize>,
    pub diagnostic_id: Option<String>,
    pub collected_at: Option<String>,
    pub error: Option<String>,
    pub manual: Option<bool>,
}

pub async fn handle_v1_buddy_issues_create(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    axum::Json(req): axum::Json<IssueCreateRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let pre_diag = if req.diagnostic_index.is_none()
        && req.diagnostic_id.is_none()
        && req.collected_at.is_none()
    {
        match &req.error {
            Some(err) => {
                Some(crate::buddy::diagnostics::collect_diagnostics(gcx.clone(), err).await)
            }
            None => None,
        }
    } else {
        None
    };

    let ctx = crate::buddy::actor::resolve_diagnostic(
        gcx.clone(),
        req.diagnostic_index,
        req.diagnostic_id.as_deref(),
        req.collected_at.as_deref(),
        pre_diag,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    let (auto_enabled, last_issue_at, recent_errors) = {
        let buddy_arc = gcx.read().await.buddy.clone();
        let lock = buddy_arc.lock().await;
        let svc = lock.as_ref().ok_or_else(|| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "buddy service not initialized".to_string(),
            )
        })?;

        (
            svc.settings.auto_issue_creation,
            svc.last_issue_at,
            svc.recent_issue_errors.clone(),
        )
    };

    let manual = req.manual.unwrap_or(false);
    let result = crate::buddy::issues::create_issue(
        gcx.clone(),
        &ctx,
        auto_enabled,
        manual,
        last_issue_at,
        &recent_errors,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

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
