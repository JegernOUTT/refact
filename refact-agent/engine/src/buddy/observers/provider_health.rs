use std::collections::{HashSet, VecDeque};

use chrono::{DateTime, Utc};

use crate::app_state::AppState;
use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::caps::DefaultModels;

pub struct ProviderHealthObserver;

pub const PROVIDER_RATE_LIMIT_WINDOW: chrono::Duration = chrono::Duration::minutes(60);
pub const PROVIDER_RATE_LIMIT_THRESHOLD: usize = 5;
pub const PROVIDER_RATE_LIMIT_MIN_WORKFLOWS: usize = 2;
pub const MAX_PROVIDER_RATE_LIMIT_EVENTS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProviderRateLimitEvent {
    workflow_id: String,
    occurred_at: DateTime<Utc>,
}

#[derive(Default)]
pub struct ProviderRateLimitWindow {
    events: VecDeque<ProviderRateLimitEvent>,
    was_active: bool,
    recovery_pending: bool,
}

impl ProviderRateLimitWindow {
    pub fn record_failure(&mut self, workflow_id: &str, now: DateTime<Utc>) {
        self.prune(now);
        self.events.push_back(ProviderRateLimitEvent {
            workflow_id: workflow_id.to_string(),
            occurred_at: now,
        });
        while self.events.len() > MAX_PROVIDER_RATE_LIMIT_EVENTS {
            self.events.pop_front();
        }
    }

    pub fn record_success(&mut self) -> bool {
        let was_active = self.was_active;
        self.events.clear();
        self.was_active = false;
        self.recovery_pending = was_active;
        was_active
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    fn prune(&mut self, now: DateTime<Utc>) {
        let cutoff = now - PROVIDER_RATE_LIMIT_WINDOW;
        while self
            .events
            .front()
            .is_some_and(|event| event.occurred_at < cutoff)
        {
            self.events.pop_front();
        }
    }

    fn snapshot(&mut self, now: DateTime<Utc>) -> ProviderRateLimitSnapshot {
        self.prune(now);
        let distinct_workflows = self
            .events
            .iter()
            .map(|event| event.workflow_id.as_str())
            .collect::<HashSet<_>>()
            .len();
        let active = self.events.len() >= PROVIDER_RATE_LIMIT_THRESHOLD
            && distinct_workflows >= PROVIDER_RATE_LIMIT_MIN_WORKFLOWS;
        let recovered = self.recovery_pending;
        self.recovery_pending = false;
        self.was_active = active;
        ProviderRateLimitSnapshot {
            active,
            recovered,
            failures: self.events.len(),
            distinct_workflows,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProviderRateLimitSnapshot {
    active: bool,
    recovered: bool,
    failures: usize,
    distinct_workflows: usize,
}

pub fn provider_rate_limit_fact(
    window: &mut ProviderRateLimitWindow,
    now: DateTime<Utc>,
) -> Option<BuddyFact> {
    let snapshot = window.snapshot(now);
    if !snapshot.active && !snapshot.recovered {
        return None;
    }
    Some(BuddyFact {
        kind: BuddyFactKind::ProviderRateLimited,
        key: "provider:rate_limited:global".to_string(),
        source: "provider_health",
        payload: serde_json::json!({
            "active": snapshot.active,
            "provider_or_model": "configured provider",
            "failures": snapshot.failures,
            "distinct_workflows": snapshot.distinct_workflows,
            "window_minutes": PROVIDER_RATE_LIMIT_WINDOW.num_minutes(),
            "guidance": "Wait for the limit to reset, switch the provider slot yourself, or raise the provider cap."
        }),
        seen_at: now,
        confidence: 0.98,
    })
}

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

    async fn observe(&self, gcx: AppState, ctx: &ObserverContext) -> Vec<BuddyFact> {
        let (defaults, chat_models, completion_models) = {
            let caps_state = gcx.model.caps.read().await;
            let caps = match &caps_state.caps {
                Some(c) => c.clone(),
                None => return vec![],
            };
            (
                caps.defaults.clone(),
                caps.chat_models.keys().cloned().collect::<Vec<_>>(),
                caps.completion_models.keys().cloned().collect::<Vec<_>>(),
            )
        };
        let mut facts =
            detect_provider_health_facts(&defaults, &chat_models, &completion_models, ctx.now);
        let rate_limit_fact = {
            let mut buddy = gcx.buddy.buddy.lock().await;
            buddy.as_mut().and_then(|service| {
                provider_rate_limit_fact(&mut service.provider_rate_limit_window, ctx.now)
            })
        };
        if let Some(fact) = rate_limit_fact {
            facts.push(fact);
        }
        facts
    }
}
