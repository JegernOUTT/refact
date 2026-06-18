use axum::extract::State;
use axum::extract::Path;
use axum::extract::Query;
use axum::response::Result;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};

use crate::buddy::diagnostics::DiagnosticContext;
use crate::buddy::events::BuddyEvent;
use crate::buddy::memory_lifecycle::{apply_memory_lifecycle_op_status, MemoryOpStatus};
use crate::buddy::pulse_inject::build_buddy_pulse_payload;
use crate::buddy::settings::{
    AutonomyLevel, BuddySettings, HumorLevel, ObserverToggles, MAX_PALETTE_INDEX,
};
use crate::buddy::storage::{enqueue_memory_op, load_memory_ops};
use crate::buddy::types::{BuddyActivity, BuddyCareAction, BuddyConversationEntry, BuddySuggestion};
use crate::buddy::user_activity::time_of_day_pattern;
use refact_buddy_core::user_action::UserAction;
use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;
use crate::buddy::voice_service::SpeechIntent;
use crate::app_state::AppState;
use crate::custom_error::ScratchError;

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

#[derive(Debug, Deserialize)]
pub struct UserActivityQuery {
    pub hours: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct BuddyArtifactRequest {
    pub op_id: String,
}

pub async fn handle_v1_buddy_user_action(
    State(app): State<AppState>,
    axum::Json(action): axum::Json<UserAction>,
) -> Result<StatusCode, ScratchError> {
    let user_activity = app.buddy.user_activity.clone();
    let mut ring = user_activity.lock().await;
    ring.push(action);
    if let Err(e) = ring.persist().await {
        tracing::warn!("buddy: failed to persist user activity: {}", e);
    }
    Ok(StatusCode::OK)
}

pub async fn handle_v1_buddy_user_activity(
    State(app): State<AppState>,
    Query(query): Query<UserActivityQuery>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let user_activity = app.buddy.user_activity.clone();
    let ring = user_activity.lock().await;
    let actions = ring.last_hours(query.hours.unwrap_or(24));
    let pattern = time_of_day_pattern(&actions);
    Ok(axum::Json(serde_json::json!({
        "actions": actions,
        "time_of_day_pattern": pattern,
    })))
}

pub async fn handle_v1_buddy_pulse_preview(
    State(app): State<AppState>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    Ok(axum::Json(serde_json::json!({
        "payload": build_buddy_pulse_payload(app).await
    })))
}

pub async fn handle_v1_buddy_artifacts(
    State(app): State<AppState>,
) -> Result<axum::Json<crate::buddy::memory_lifecycle::MemoryOpsState>, ScratchError> {
    let buddy_arc = app.buddy.buddy.clone();
    let lock = buddy_arc.lock().await;
    let state = lock
        .as_ref()
        .map(|service| service.memory_ops.clone())
        .unwrap_or_default();
    Ok(axum::Json(state))
}

pub async fn handle_v1_buddy_artifact_approve(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<BuddyArtifactRequest>,
) -> Result<StatusCode, ScratchError> {
    update_buddy_artifact_status(app, req.op_id, MemoryOpStatus::Approved).await
}

pub async fn handle_v1_buddy_artifact_reject(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<BuddyArtifactRequest>,
) -> Result<StatusCode, ScratchError> {
    update_buddy_artifact_status(app, req.op_id, MemoryOpStatus::Rejected).await
}

async fn update_buddy_artifact_status(
    app: AppState,
    op_id: String,
    status: MemoryOpStatus,
) -> Result<StatusCode, ScratchError> {
    let op_id = op_id.trim().to_string();
    if op_id.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "op_id is required".to_string(),
        ));
    }

    let project_root = crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "no project root".to_string(),
            )
        })?;

    let state = load_memory_ops(&project_root).await;
    let mut op = state
        .ops
        .into_iter()
        .find(|op| op.op_id == op_id)
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("artifact not found: {op_id}"),
            )
        })?;

    op.status = status;
    op.error = None;
    let updated = if status == MemoryOpStatus::Approved {
        apply_memory_lifecycle_op_status(app.clone(), &op).await
    } else {
        op
    };

    let state = enqueue_memory_op(&project_root, updated)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let buddy_arc = app.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    if let Some(service) = lock.as_mut() {
        service.memory_ops = state;
    }
    Ok(StatusCode::OK)
}

pub async fn handle_v1_buddy_snapshot(
    State(app): State<AppState>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = app.buddy.buddy.clone();
    let snapshot = {
        let lock = buddy_arc.lock().await;
        lock.as_ref().map(|service| service.snapshot())
    };
    match snapshot {
        Some(snapshot) => Ok(axum::Json(serde_json::to_value(snapshot).map_err(|e| {
            ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?)),
        None => {
            let mut payload = serde_json::json!({
                "enabled": false,
                "state": crate::buddy::state::default_buddy_state(),
                "settings": crate::buddy::settings::BuddySettings::default(),
                "recent_diagnostics": [],
                "runtime_queue": [],
                "now_playing": null,
                "active_speech": null
            });
            if let Some(project_root) = crate::files_correction::get_project_dirs(app.gcx.clone())
                .await
                .into_iter()
                .next()
            {
                attach_storage_metadata(&mut payload, &project_root);
            }
            Ok(axum::Json(payload))
        }
    }
}

pub async fn handle_v1_buddy_settings_get(
    State(app): State<AppState>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = app.buddy.buddy.clone();
    let service_settings = {
        let lock = buddy_arc.lock().await;
        lock.as_ref()
            .map(|service| (service.project_root.clone(), service.settings.clone()))
    };
    match service_settings {
        Some((project_root, settings)) => {
            let mut payload = serde_json::to_value(settings)
                .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            attach_storage_metadata(&mut payload, &project_root);
            Ok(axum::Json(payload))
        }
        None => {
            let mut payload =
                serde_json::to_value(crate::buddy::settings::BuddySettings::default()).map_err(
                    |e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                )?;
            if let Some(project_root) = crate::files_correction::get_project_dirs(app.gcx.clone())
                .await
                .into_iter()
                .next()
            {
                attach_storage_metadata(&mut payload, &project_root);
            }
            Ok(axum::Json(payload))
        }
    }
}

fn deserialize_optional_field<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserverTogglesPatch {
    pub task_health: Option<bool>,
    pub trajectory_clutter: Option<bool>,
    pub chat_pattern: Option<bool>,
    pub customization_drift: Option<bool>,
    pub memory_garden: Option<bool>,
    pub mcp_auth: Option<bool>,
    pub git_pressure: Option<bool>,
    pub diagnostic_cluster: Option<bool>,
    pub provider_health: Option<bool>,
}

impl ObserverTogglesPatch {
    fn apply_to(&self, observers: &mut ObserverToggles) {
        if let Some(v) = self.task_health {
            observers.task_health = v;
        }
        if let Some(v) = self.trajectory_clutter {
            observers.trajectory_clutter = v;
        }
        if let Some(v) = self.chat_pattern {
            observers.chat_pattern = v;
        }
        if let Some(v) = self.customization_drift {
            observers.customization_drift = v;
        }
        if let Some(v) = self.memory_garden {
            observers.memory_garden = v;
        }
        if let Some(v) = self.mcp_auth {
            observers.mcp_auth = v;
        }
        if let Some(v) = self.git_pressure {
            observers.git_pressure = v;
        }
        if let Some(v) = self.diagnostic_cluster {
            observers.diagnostic_cluster = v;
        }
        if let Some(v) = self.provider_health {
            observers.provider_health = v;
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuddySettingsRequest {
    pub enabled: Option<bool>,
    pub auto_diagnostics: Option<bool>,
    pub auto_issue_creation: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub personality_prompt: Option<Option<String>>,
    pub clear_personality_prompt: Option<bool>,
    pub autonomous_chats_enabled: Option<bool>,
    pub proactive_enabled: Option<bool>,
    pub message_observation_enabled: Option<bool>,
    pub chat_reactions_enabled: Option<bool>,
    pub housekeeping_enabled: Option<bool>,
    pub humor_enabled: Option<bool>,
    pub humor_level: Option<HumorLevel>,
    pub autonomy_level: Option<AutonomyLevel>,
    pub quiet_mode: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub daily_digest_hour: Option<Option<u8>>,
    pub observers: Option<ObserverTogglesPatch>,
    pub palette_index: Option<usize>,
}

impl BuddySettingsRequest {
    pub(crate) fn validate(&self) -> Result<(), ScratchError> {
        if let Some(pi) = self.palette_index {
            if pi > MAX_PALETTE_INDEX {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    "palette_index must be 0-7".to_string(),
                ));
            }
        }
        if let Some(Some(hour)) = self.daily_digest_hour {
            if hour > 23 {
                return Err(ScratchError::new(
                    StatusCode::BAD_REQUEST,
                    "daily_digest_hour must be null or 0-23".to_string(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn apply_to_settings(&self, settings: &mut BuddySettings) -> bool {
        let mut persona_dirty = false;
        if let Some(v) = self.enabled {
            settings.enabled = v;
        }
        if let Some(v) = self.auto_diagnostics {
            settings.auto_diagnostics = v;
        }
        if let Some(v) = self.auto_issue_creation {
            settings.auto_issue_creation = v;
        }
        if self.clear_personality_prompt.unwrap_or(false) {
            settings.personality_prompt = None;
            persona_dirty = true;
        } else if let Some(prompt) = &self.personality_prompt {
            settings.personality_prompt = prompt.clone();
            persona_dirty = true;
        }
        if let Some(v) = self.autonomous_chats_enabled {
            settings.autonomous_chats_enabled = v;
        }
        if let Some(v) = self.proactive_enabled {
            settings.proactive_enabled = v;
        }
        if let Some(v) = self.message_observation_enabled {
            settings.message_observation_enabled = v;
        }
        if let Some(v) = self.chat_reactions_enabled {
            settings.chat_reactions_enabled = v;
        }
        if let Some(v) = self.housekeeping_enabled {
            settings.housekeeping_enabled = v;
        }
        if let Some(v) = self.humor_enabled {
            settings.humor_enabled = v;
            persona_dirty = true;
        }
        if let Some(v) = self.humor_level {
            settings.humor_level = v;
            persona_dirty = true;
        }
        if let Some(v) = self.autonomy_level {
            settings.autonomy_level = v;
            persona_dirty = true;
        }
        if let Some(v) = self.quiet_mode {
            settings.quiet_mode = v;
        }
        if let Some(v) = self.daily_digest_hour {
            settings.daily_digest_hour = v;
        }
        if let Some(observers) = &self.observers {
            observers.apply_to(&mut settings.observers);
        }
        persona_dirty
    }
}

pub async fn handle_v1_buddy_settings_update(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<BuddySettingsRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    req.validate()?;

    let buddy_arc = app.buddy.buddy.clone();
    let updated = {
        let mut lock = buddy_arc.lock().await;
        if let Some(service) = lock.as_mut() {
            let was_enabled = service.settings.enabled;
            if req.apply_to_settings(&mut service.settings) {
                crate::buddy::state::mark_persona_cache_dirty();
            }
            if !was_enabled && service.settings.enabled {
                service.chat_reaction_limiter.reset();
            }
            if let Some(pi) = req.palette_index {
                service.state.identity.palette_index = pi;
                crate::buddy::state::mark_persona_cache_dirty();
                crate::buddy::state::sync_state(&mut service.state);
                service.dirty = true;
                let _ = service.events_tx.send(BuddyEvent::StateUpdated {
                    state: service.state.clone(),
                });
            }
            Some((
                service.project_root.clone(),
                service.settings.clone(),
                service.events_tx.clone(),
            ))
        } else {
            None
        }
    };
    if let Some((project_root, new_settings, tx)) = updated {
        crate::buddy::settings::save_settings(&project_root, &new_settings)
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let storage = crate::buddy::settings::storage_metadata(&project_root);
        let _ = tx.send(BuddyEvent::SettingsChanged {
            settings: new_settings.clone(),
        });
        let mut payload = serde_json::to_value(new_settings)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        attach_storage_metadata_value(&mut payload, storage);
        return Ok(axum::Json(payload));
    }

    let project_root = crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "no project root".to_string(),
            )
        })?;

    let mut new_settings = crate::buddy::settings::load_settings(&project_root).await;
    if req.apply_to_settings(&mut new_settings) {
        crate::buddy::state::mark_persona_cache_dirty();
    }
    crate::buddy::settings::save_settings(&project_root, &new_settings)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let mut payload = serde_json::to_value(new_settings)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    attach_storage_metadata(&mut payload, &project_root);
    Ok(axum::Json(payload))
}

fn attach_storage_metadata(payload: &mut serde_json::Value, project_root: &std::path::Path) {
    attach_storage_metadata_value(
        payload,
        crate::buddy::settings::storage_metadata(project_root),
    );
}

fn attach_storage_metadata_value(
    payload: &mut serde_json::Value,
    storage: crate::buddy::settings::BuddyStorageMetadata,
) {
    if let serde_json::Value::Object(map) = payload {
        map.insert("storage".to_string(), serde_json::json!(storage));
    }
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

async fn refresh_completed_quest_with_voice(
    app: AppState,
    mut lock: tokio::sync::MutexGuard<'_, Option<crate::buddy::actor::BuddyService>>,
) {
    let Some(svc) = lock.as_mut() else {
        return;
    };
    let completed = svc
        .state
        .active_quest
        .as_ref()
        .map(|quest| quest.status == "active" && quest.progress >= quest.goal)
        .unwrap_or(false);
    if !completed {
        return;
    }
    let Some(quest) = crate::buddy::state::complete_active_quest(&mut svc.state) else {
        return;
    };
    let persona = svc.state.personality.clone();
    let identity_name = svc.state.identity.name.clone();
    let pulse = svc.pulse.clone();
    let reward = quest.reward_xp;
    svc.dirty = true;
    let _ = svc.events_tx.send(BuddyEvent::StateUpdated {
        state: svc.state.clone(),
    });
    drop(lock);

    let completed = crate::buddy::actor::complete_quest_with_voice(
        app.clone(),
        quest,
        persona,
        identity_name,
        pulse,
    )
    .await;
    crate::buddy::actor::buddy_update_speech(app.clone(), completed.speech).await;
    crate::buddy::actor::buddy_apply(app.clone(), completed.mutation).await;
    if reward > 0 {
        let buddy_arc = app.buddy.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        if let Some(svc) = lock.as_mut() {
            svc.grant_xp(reward);
        }
    }
}

pub async fn handle_v1_buddy_care(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<BuddyCareRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = app.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy service not initialized".to_string(),
        )
    })?;

    let (_, message) = crate::buddy::state::apply_care_action(
        &mut svc.state,
        req.action.clone(),
        req.toy.as_deref(),
    );
    svc.refresh_active_quest();
    svc.dirty = true;
    let _ = svc.events_tx.send(BuddyEvent::StateUpdated {
        state: svc.state.clone(),
    });
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
    refresh_completed_quest_with_voice(app.clone(), lock).await;
    let snapshot = crate::buddy::actor::buddy_snapshot(app).await;

    Ok(axum::Json(serde_json::json!({
        "message": message,
        "snapshot": snapshot
    })))
}

pub async fn handle_v1_buddy_personality_reroll(
    State(app): State<AppState>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = app.buddy.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let svc = lock.as_mut().ok_or_else(|| {
        ScratchError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "buddy service not initialized".to_string(),
        )
    })?;

    crate::buddy::state::reroll_personality(&mut svc.state);
    svc.refresh_active_quest();
    svc.dirty = true;
    let _ = svc.events_tx.send(BuddyEvent::StateUpdated {
        state: svc.state.clone(),
    });
    let should_refresh_completed_quest = svc
        .state
        .active_quest
        .as_ref()
        .map(|quest| quest.status == "active" && quest.progress >= quest.goal)
        .unwrap_or(false);
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

    if should_refresh_completed_quest {
        refresh_completed_quest_with_voice(app.clone(), lock).await;
        let snapshot = crate::buddy::actor::buddy_snapshot(app).await;
        return Ok(axum::Json(serde_json::json!({
            "snapshot": snapshot
        })));
    }

    Ok(axum::Json(serde_json::json!({
        "snapshot": svc.snapshot()
    })))
}

pub async fn handle_v1_buddy_quest_dismiss(
    State(app): State<AppState>,
) -> Result<StatusCode, ScratchError> {
    let buddy_arc = app.buddy.buddy.clone();
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
    State(app): State<AppState>,
    axum::Json(req): axum::Json<BuddyQuestAcceptRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = app.buddy.buddy.clone();
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

    let title = quest.title.clone();
    let fallback_text = format!("Quest accepted: {title}. I’ll keep score from here.");
    let speech_id = format!("quest-accept-{}", quest.id);
    let dedupe_key = format!("quest_accept_{}", quest.quest_type);
    let controls = quest.controls.clone();
    let persona = svc.state.personality.clone();
    let identity_name = svc.state.identity.name.clone();
    let pulse = svc.pulse.clone();
    let workflow_id = quest.quest_type.clone();

    svc.dismiss_suggestion(&req.suggestion_id);
    crate::buddy::state::activate_quest(&mut svc.state, quest);
    svc.dirty = true;
    let _ = svc.events_tx.send(BuddyEvent::StateUpdated {
        state: svc.state.clone(),
    });
    let snapshot = svc.snapshot();
    drop(lock);

    let mut speech = crate::buddy::actor::render_buddy_speech(
        app.clone(),
        persona,
        identity_name,
        pulse,
        Some(workflow_id),
        fallback_text.clone(),
        SpeechIntent::QuestAccept,
        fallback_text,
    )
    .await;
    speech.id = speech_id;
    speech.ttl_seconds = 12;
    speech.dedupe_key = Some(dedupe_key);
    speech.controls = controls;
    crate::buddy::actor::buddy_update_speech(app.clone(), speech).await;
    let snapshot = crate::buddy::actor::buddy_snapshot(app)
        .await
        .unwrap_or(snapshot);

    Ok(axum::Json(serde_json::json!({
        "snapshot": snapshot,
        "suggestion": serde_json::to_value::<BuddySuggestion>(suggestion)
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    })))
}

pub async fn handle_v1_buddy_activities(
    State(app): State<AppState>,
) -> Result<axum::Json<Vec<BuddyActivity>>, ScratchError> {
    let buddy_arc = app.buddy.buddy.clone();
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
    State(app): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ConversationsListQuery>,
) -> Result<axum::Json<Vec<BuddyConversationEntry>>, ScratchError> {
    let project_root = crate::files_correction::get_project_dirs(app.gcx.clone())
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
    State(app): State<AppState>,
    body_bytes: axum::body::Bytes,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let gcx = app.gcx.clone();
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

    let snapshot = TrajectorySnapshot {
        goal: None,
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
        autonomous_no_confirm: false,
        version: 1,
        task_meta: None,
        worktree: None,
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
        auto_compact_enabled: None,
        frozen_request_prefix: None,
        claude_code_identity: None,
        reactive_compact_attempts: None,
        wake_up_at: None,
        waiting_for_card_ids: Vec::new(),
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
    State(app): State<AppState>,
    axum::Json(req): axum::Json<DiagnosticsCollectRequest>,
) -> Result<axum::Json<BuddyInvestigationContextResponse>, ScratchError> {
    let log_lines = crate::buddy::issues::investigation_logs(
        app.clone(),
        &req.error,
        req.collected_at.as_deref(),
    )
    .await
    .unwrap_or_else(|e| format!("Investigation logs unavailable: {}", e));
    let internal = crate::buddy::issues::investigation_internal_context(app.clone())
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
    State(app): State<AppState>,
    axum::Json(req): axum::Json<CreateSetupRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let project_root = crate::files_correction::get_project_dirs(app.gcx.clone())
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
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ScratchError> {
    let buddy_arc = app.buddy.buddy.clone();
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
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let buddy_arc = app.buddy.buddy.clone();
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
    State(app): State<AppState>,
    axum::Json(req): axum::Json<DiagnosticsCollectRequest>,
) -> Result<axum::Json<DiagnosticContext>, ScratchError> {
    let mut ctx = crate::buddy::diagnostics::collect_diagnostics(app.clone(), &req.error).await;
    ctx.source_file = req
        .source_file
        .as_deref()
        .and_then(crate::buddy::actor::redact_diagnostic_metadata);
    ctx.tool_name = req
        .tool_name
        .as_deref()
        .and_then(crate::buddy::actor::redact_diagnostic_metadata);
    ctx.chat_id = req.chat_id;
    ctx.collected_at = req.collected_at.unwrap_or(ctx.collected_at);

    let buddy_arc = app.buddy.buddy.clone();
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
    State(app): State<AppState>,
) -> Result<axum::Json<Vec<DiagnosticContext>>, ScratchError> {
    let project_root = crate::buddy::actor::latest_project_root(app.clone())
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
    State(app): State<AppState>,
    axum::Json(req): axum::Json<IssueCreateRequest>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let pre_diag = if req.diagnostic_index.is_none()
        && req.diagnostic_id.is_none()
        && req.collected_at.is_none()
    {
        match &req.error {
            Some(err) => {
                Some(crate::buddy::diagnostics::collect_diagnostics(app.clone(), err).await)
            }
            None => None,
        }
    } else {
        None
    };

    let ctx = crate::buddy::actor::resolve_diagnostic(
        app.clone(),
        req.diagnostic_index,
        req.diagnostic_id.as_deref(),
        req.collected_at.as_deref(),
        pre_diag,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    let (auto_enabled, last_issue_at, recent_errors) = {
        let buddy_arc = app.buddy.buddy.clone();
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
        app.clone(),
        &ctx,
        auto_enabled,
        manual,
        last_issue_at,
        &recent_errors,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    let (url, _activity) = result;

    Ok(axum::Json(serde_json::json!({"url": url})))
}
