use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::app_state::AppState;
use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::caps::DefaultModels;

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
            key: format!("provider:broken_ref:{}:{}", field_name, model_id),
            source: "provider_health",
            payload: serde_json::json!({ "field": payload_field, "model_id": model_id }),
            seen_at: now,
            confidence: 0.9,
        });
    }
}

fn check_optional_default_model(
    facts: &mut Vec<BuddyFact>,
    field_name: &str,
    model_id: &str,
    payload_field: &str,
    available_models: &[String],
    now: DateTime<Utc>,
) {
    if model_id.is_empty()
        || available_models
            .iter()
            .any(|available| available == model_id)
    {
        return;
    }

    facts.push(BuddyFact {
        kind: BuddyFactKind::BrokenModelReference,
        key: format!("provider:broken_ref:{}:{}", field_name, model_id),
        source: "provider_health",
        payload: serde_json::json!({ "field": payload_field, "model_id": model_id }),
        seen_at: now,
        confidence: 0.9,
    });
}

fn check_default_model_once(
    facts: &mut Vec<BuddyFact>,
    seen_broken_models: &mut HashSet<(String, String)>,
    field_name: &str,
    model_id: &str,
    payload_field: &str,
    available_models: &[String],
    now: DateTime<Utc>,
) {
    let before = facts.len();
    check_default_model(
        facts,
        field_name,
        model_id,
        payload_field,
        available_models,
        now,
    );
    let Some(fact) = facts.get(before) else {
        return;
    };
    if fact.kind != BuddyFactKind::BrokenModelReference {
        return;
    }
    if !seen_broken_models.insert((field_name.to_string(), model_id.to_string())) {
        facts.pop();
    }
}

pub fn detect_provider_health_facts(
    defaults: &DefaultModels,
    chat_models: &[String],
    completion_models: &[String],
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
            "chat_model_2",
            defaults.chat_model_2.as_str(),
            "chat_model_2",
        ),
        (
            "task_planner_agent_model",
            defaults.task_planner_agent_model.as_str(),
            "task_planner_agent_model",
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
    let mut seen_broken_models = HashSet::new();
    for (field_name, model_id, payload_field) in &chat_fields {
        check_default_model_once(
            &mut facts,
            &mut seen_broken_models,
            field_name,
            model_id,
            payload_field,
            chat_models,
            now,
        );
    }
    check_optional_default_model(
        &mut facts,
        "completion_default_model",
        defaults.completion_default_model.as_str(),
        "completion_model",
        completion_models,
        now,
    );
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
        let completion_models: Vec<String> = caps.completion_models.keys().cloned().collect();
        detect_provider_health_facts(&caps.defaults, &chat_models, &completion_models, Utc::now())
    }
}
