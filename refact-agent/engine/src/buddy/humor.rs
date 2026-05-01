use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use tokio::sync::RwLock;
use tracing::debug;
use uuid::Uuid;

use crate::buddy::types::{BuddyFactKind, BuddyPulse};
use crate::global_context::GlobalContext;

pub const HUMOR_BUDGET_PER_HOUR: u32 = 3;
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
    async fn generate(
        &self,
        kind: BuddyFactKind,
        summary: String,
        gcx: Arc<RwLock<GlobalContext>>,
    ) -> Vec<String>;
}

/// Production generator — calls the configured `chat_buddy_model` via subchat.
pub struct DefaultHumorGenerator;

#[async_trait]
impl HumorGenerator for DefaultHumorGenerator {
    async fn generate(
        &self,
        kind: BuddyFactKind,
        summary: String,
        gcx: Arc<RwLock<GlobalContext>>,
    ) -> Vec<String> {
        generate_via_llm(kind, summary, gcx).await
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
    pub async fn generate(&self, gcx: Arc<RwLock<GlobalContext>>) -> Vec<String> {
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
            "tasks:{} stuck:{}, traj:{}, mem:{}, mcp:{} failing:{}, providers_ok:{}",
            pulse.tasks.total,
            pulse.tasks.stuck,
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

async fn generate_via_llm(
    kind: BuddyFactKind,
    pulse_summary: String,
    gcx: Arc<RwLock<GlobalContext>>,
) -> Vec<String> {
    let buddy_model = {
        let gcx_locked = gcx.read().await;
        gcx_locked
            .caps
            .as_ref()
            .map(|c| c.defaults.chat_buddy_model.clone())
            .unwrap_or_default()
    };
    if buddy_model.is_empty() {
        return vec![];
    }

    let prompt = format!(
        "Generate 3 short, friendly, situational one-liners about: {:?} in a software project.\n\
         Real state: {}. Keep each under 80 chars. No jargon. No fake events.\n\
         Output as a JSON array of 3 strings.",
        kind, pulse_summary
    );

    let messages = vec![crate::call_validation::ChatMessage::new(
        "user".to_string(),
        prompt,
    )];

    let config = match crate::subchat::resolve_subchat_config(
        gcx.clone(),
        "buddy_humor",
        true,
        Some(format!("buddy-humor-{}", Uuid::new_v4())),
        Some(format!("Humor for {:?}", kind)),
        None,
        None,
        None,
        Some(vec![]),
        1,
        false,
        None,
        "agent".to_string(),
    )
    .await
    {
        Ok(mut config) => {
            config.mode = "buddy".to_string();
            config.buddy_meta = Some(crate::buddy::types::BuddyThreadMeta {
                is_buddy_chat: true,
                buddy_chat_kind: "system".to_string(),
                workflow_id: Some("buddy_humor".to_string()),
            });
            config
        }
        Err(e) => {
            debug!("buddy humor: failed to resolve subchat config: {}", e);
            return vec![];
        }
    };

    let result = match crate::subchat::run_subchat(gcx, messages, config).await {
        Ok(r) => r,
        Err(e) => {
            debug!("buddy humor: LLM call failed: {}", e);
            return vec![];
        }
    };

    let text = match result.messages.last().and_then(|m| match &m.content {
        crate::call_validation::ChatContent::SimpleText(t) => Some(t.clone()),
        _ => None,
    }) {
        Some(t) => t,
        None => return vec![],
    };

    let parsed: Vec<String> = parse_json_array(&text);
    parsed
        .into_iter()
        .filter(|l| !l.is_empty() && l.len() <= 100)
        .take(3)
        .collect()
}

fn parse_json_array(text: &str) -> Vec<String> {
    if let Ok(lines) = serde_json::from_str::<Vec<String>>(text) {
        return lines;
    }
    let start = match text.find('[') {
        Some(i) => i,
        None => return vec![],
    };
    let end = match text.rfind(']') {
        Some(i) => i,
        None => return vec![],
    };
    if end <= start {
        return vec![];
    }
    serde_json::from_str::<Vec<String>>(&text[start..=end]).unwrap_or_default()
}
