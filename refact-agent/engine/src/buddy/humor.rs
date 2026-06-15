use std::sync::Arc;
use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use tracing::debug;

use crate::buddy::types::{BuddyFactKind, BuddyPersonalityProfile, BuddyPulse};
use crate::buddy::voice_service::{SpeechIntent, VoiceCtx, voice_service};
use crate::app_state::AppState;

pub const HUMOR_BUDGET_PER_HOUR: u32 = 5;
pub const HUMOR_BATCH_TTL: Duration = Duration::hours(1);
pub const HUMOR_TIMEOUT_SECS: u64 = 8;

/// A cached batch of LLM-generated one-liners for a specific fact kind.
#[derive(Debug, Clone)]
pub struct HumorBatch {
    pub lines: Vec<String>,
    pub used: u8,
    pub expires_at: DateTime<Utc>,
}

/// Abstraction over one-liner generation, allowing injection in tests.
#[async_trait]
pub trait HumorGenerator: Send + Sync {
    async fn generate(&self, kind: BuddyFactKind, summary: String, gcx: AppState) -> Vec<String>;
}

pub struct DefaultHumorGenerator;

#[async_trait]
impl HumorGenerator for DefaultHumorGenerator {
    async fn generate(&self, kind: BuddyFactKind, summary: String, gcx: AppState) -> Vec<String> {
        generate_via_voice_service(kind, summary, gcx).await
    }
}

/// Manages the per-hour humor budget and per-kind batch cache.
pub struct HumorService {
    cache: HashMap<BuddyFactKind, HumorBatch>,
    used_this_hour: u32,
    hour_started_at: DateTime<Utc>,
    generator: Arc<dyn HumorGenerator>,
}

pub struct HumorReservation {
    primary_kind: BuddyFactKind,
    pulse_summary: String,
    generator: Arc<dyn HumorGenerator>,
}

pub enum HumorPlan {
    Ready(String),
    Generate(HumorReservation),
    Skip,
}

impl HumorReservation {
    pub async fn generate(&self, gcx: AppState) -> Vec<String> {
        tokio::time::timeout(
            tokio::time::Duration::from_secs(HUMOR_TIMEOUT_SECS),
            self.generator
                .generate(self.primary_kind, self.pulse_summary.clone(), gcx),
        )
        .await
        .unwrap_or_default()
    }
}

impl HumorService {
    /// Create a new service wired to the production LLM generator.
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            used_this_hour: 0,
            hour_started_at: Utc::now(),
            generator: Arc::new(DefaultHumorGenerator),
        }
    }

    /// Create a service with an injected generator (used in tests).
    #[cfg(test)]
    pub fn new_with_generator(generator: Arc<dyn HumorGenerator>) -> Self {
        Self {
            cache: HashMap::new(),
            used_this_hour: 0,
            hour_started_at: Utc::now(),
            generator,
        }
    }

    pub fn plan_humor(&mut self, primary_kind: BuddyFactKind, pulse: &BuddyPulse) -> HumorPlan {
        let now = Utc::now();
        self.reset_hour_if_needed(now);
        self.cache_purge_expired(now);

        if let Some(line) = self.cache_pop_line(primary_kind) {
            return HumorPlan::Ready(line);
        }

        if self.used_this_hour >= HUMOR_BUDGET_PER_HOUR {
            return HumorPlan::Skip;
        }

        let pulse_summary = format!(
            "tasks:{} recent_stuck_alerts_1h:{}, traj:{}, mem:{}, mcp:{} failing:{}, providers_ok:{}",
            pulse.tasks.total,
            pulse.tasks.recent_stuck_alert_count_1h(),
            pulse.trajectories.total,
            pulse.memory.total,
            pulse.mcp.total,
            pulse.mcp.failing,
            pulse.providers.defaults_ok,
        );

        HumorPlan::Generate(HumorReservation {
            primary_kind,
            pulse_summary,
            generator: self.generator.clone(),
        })
    }

    pub fn complete_humor(
        &mut self,
        reservation: HumorReservation,
        lines: Vec<String>,
    ) -> Option<String> {
        let now = Utc::now();
        self.reset_hour_if_needed(now);
        self.cache_purge_expired(now);

        if lines.is_empty() {
            debug!(
                "buddy humor: generator returned no lines for {:?}",
                reservation.primary_kind
            );
            return None;
        }

        if self.used_this_hour >= HUMOR_BUDGET_PER_HOUR {
            return None;
        }

        let batch = HumorBatch {
            lines,
            used: 0,
            expires_at: now + HUMOR_BATCH_TTL,
        };
        self.cache.insert(reservation.primary_kind, batch);
        self.used_this_hour += 1;

        self.cache_pop_line(reservation.primary_kind)
    }

    fn reset_hour_if_needed(&mut self, now: DateTime<Utc>) {
        if (now - self.hour_started_at) >= Duration::hours(1) {
            self.used_this_hour = 0;
            self.hour_started_at = now;
        }
    }

    fn cache_pop_line(&mut self, kind: BuddyFactKind) -> Option<String> {
        let batch = self.cache.get_mut(&kind)?;
        if batch.lines.is_empty() {
            return None;
        }
        let line = batch.lines.remove(0);
        batch.used += 1;
        if batch.lines.is_empty() {
            self.cache.remove(&kind);
        }
        Some(line)
    }

    /// Remove batches whose TTL has expired relative to `now`.
    pub(crate) fn cache_purge_expired(&mut self, now: DateTime<Utc>) {
        self.cache.retain(|_, b| b.expires_at > now);
    }
}

impl Default for HumorService {
    fn default() -> Self {
        Self::new()
    }
}

async fn generate_via_voice_service(
    kind: BuddyFactKind,
    pulse_summary: String,
    gcx: AppState,
) -> Vec<String> {
    let (persona, identity_name, pulse_one_liner) =
        match crate::buddy::actor::buddy_snapshot(gcx.clone()).await {
            Some(snapshot) => (
                snapshot.state.personality,
                snapshot.state.identity.name,
                format!(
                    "{} pending ops, {} recent stuck task alerts",
                    snapshot.pulse.memory.pending_ops,
                    snapshot.pulse.tasks.recent_stuck_alert_count_1h()
                ),
            ),
            None => (
                BuddyPersonalityProfile::default(),
                "Buddy".to_string(),
                pulse_summary.clone(),
            ),
        };
    let workflow_summary = format!("{:?}: {}", kind, pulse_summary);
    let ctx = VoiceCtx {
        persona: &persona,
        identity_name: identity_name.as_str(),
        pulse_one_liner,
        workflow_id: Some("buddy_humor"),
        workflow_summary: Some(workflow_summary.as_str()),
    };
    let speech = voice_service()
        .await
        .render_speech(gcx, ctx, SpeechIntent::Humor)
        .await;
    if speech.text.trim().is_empty() {
        vec![]
    } else {
        vec![speech.text]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn humor_generator_routes_through_voice_service() {
        let (service, renderer) = crate::buddy::voice_service::test_voice_service_with_responses(
            vec![Some("tiny joke".to_string())],
        );
        let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;

        let lines = DefaultHumorGenerator
            .generate(BuddyFactKind::TaskStuck, "tasks:1 stuck:1".to_string(), app)
            .await;

        assert_eq!(lines, vec!["tiny joke".to_string()]);
        assert_eq!(renderer.intent_kinds(), vec!["speech:humor".to_string()]);
    }
}
