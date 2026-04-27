use std::sync::Arc;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::buddy::observers::{BuddyObserver, ObserverContext, ObserverCost};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::caps::DefaultModels;
use crate::global_context::GlobalContext;

pub struct ProviderHealthObserver;

pub fn detect_provider_health_facts(
    defaults: &DefaultModels,
    available_models: &[String],
    now: DateTime<Utc>,
) -> Vec<BuddyFact> {
    let mut facts = vec![];
    let fields = [
        ("chat_model", defaults.chat_default_model.as_str()),
        ("chat_buddy_model", defaults.chat_buddy_model.as_str()),
        ("chat_thinking_model", defaults.chat_thinking_model.as_str()),
    ];
    for (field, model_id) in &fields {
        if model_id.is_empty() {
            facts.push(BuddyFact {
                kind: BuddyFactKind::DefaultModelMissing,
                key: format!("provider:default_missing:{}", field),
                source: "provider_health",
                payload: serde_json::json!({ "field": field, "missing_model_id": null }),
                seen_at: now,
                confidence: 0.95,
            });
        } else if !available_models.contains(&model_id.to_string()) {
            facts.push(BuddyFact {
                kind: BuddyFactKind::BrokenModelReference,
                key: format!("provider:broken_ref:{}", field),
                source: "provider_health",
                payload: serde_json::json!({ "field": field, "missing_model_id": model_id }),
                seen_at: now,
                confidence: 0.9,
            });
        }
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

    fn cost_class(&self) -> ObserverCost {
        ObserverCost::Io
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.provider_health && settings.proactive_enabled
    }

    async fn observe(
        &self,
        gcx: Arc<RwLock<GlobalContext>>,
        _ctx: &ObserverContext,
    ) -> Vec<BuddyFact> {
        let gcx_read = gcx.read().await;
        let caps = match &gcx_read.caps {
            Some(c) => c.clone(),
            None => return vec![],
        };
        let available: Vec<String> = caps.chat_models.keys().cloned().collect();
        detect_provider_health_facts(&caps.defaults, &available, Utc::now())
    }
}
