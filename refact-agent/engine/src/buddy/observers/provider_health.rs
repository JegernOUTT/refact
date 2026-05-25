use chrono::{DateTime, Utc};

use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::caps::DefaultModels;
use crate::app_state::AppState;

pub struct ProviderHealthObserver;

fn check_default_model(
    facts: &mut Vec<BuddyFact>,
    field_name: &str,
    model_id: &str,
    payload_field: &str,
    available_models: &[String],
    now: DateTime<Utc>,
) {
    if model_id.is_empty() {
        facts.push(BuddyFact {
            kind: BuddyFactKind::DefaultModelMissing,
            key: format!("provider:default_missing:{}", field_name),
            source: "provider_health",
            payload: serde_json::json!({ "field": payload_field, "model_id": null }),
            seen_at: now,
            confidence: 0.95,
        });
    } else if !available_models
        .iter()
        .any(|available| available == model_id)
    {
        facts.push(BuddyFact {
            kind: BuddyFactKind::BrokenModelReference,
            key: format!("provider:broken_ref:{}", field_name),
            source: "provider_health",
            payload: serde_json::json!({ "field": payload_field, "model_id": model_id }),
            seen_at: now,
            confidence: 0.9,
        });
    }
}

pub fn detect_provider_health_facts(
    defaults: &DefaultModels,
    chat_models: &[String],
    _completion_models: &[String],
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    let mut facts = vec![];
    let chat_fields = [
        (
            "chat_default_model",
            defaults.chat_default_model.as_str(),
            "chat_model",
        ),
        (
            "chat_buddy_model",
            defaults.chat_buddy_model.as_str(),
            "chat_buddy_model",
        ),
        (
            "chat_thinking_model",
            defaults.chat_thinking_model.as_str(),
            "chat_thinking_model",
        ),
        (
            "chat_light_model",
            defaults.chat_light_model.as_str(),
            "chat_light_model",
        ),
    ];
    for (field_name, model_id, payload_field) in &chat_fields {
        check_default_model(
            &mut facts,
            field_name,
            model_id,
            payload_field,
            chat_models,
            now,
        );
    }
    facts
}

#[async_trait::async_trait]
impl BuddyObserver for ProviderHealthObserver {
    fn id(&self) -> &'static str {
        "provider_health"
    }

    fn cadence_seconds(&self) -> u64 {
        300
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.provider_health && settings.proactive_enabled
    }

    async fn observe(&self, gcx: AppState, _ctx: &ObserverContext) -> Vec<BuddyFact> {
        let caps_state = gcx.model.caps.read().await;
        let caps = match &caps_state.caps {
            Some(c) => c.clone(),
            None => return vec![],
        };
        let chat_models: Vec<String> = caps.chat_models.keys().cloned().collect();
        detect_provider_health_facts(&caps.defaults, &chat_models, &[], Utc::now())
    }
}
