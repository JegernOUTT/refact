use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use tokio::sync::RwLock;
use tracing::debug;

use crate::buddy::types::{BuddyFactKind, BuddyOpportunity, BuddyPulse};
use crate::global_context::GlobalContext;

pub const HUMOR_BUDGET_PER_HOUR: u32 = 3;
pub const HUMOR_BATCH_TTL: Duration = Duration::hours(1);

/// A cached batch of LLM-generated one-liners for a specific fact kind.
#[derive(Debug, Clone)]
pub struct HumorBatch {
    pub fact_kind: BuddyFactKind,
    pub lines: Vec<String>,
    pub used: u8,
    pub generated_at: DateTime<Utc>,
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
    pub fn new_with_generator(generator: Arc<dyn HumorGenerator>) -> Self {
        Self {
            cache: HashMap::new(),
            used_this_hour: 0,
            hour_started_at: Utc::now(),
            generator,
        }
    }

    /// Lazily attach a humor line to `opp`. No fallback on failure.
    /// Mutates `opp.humor` in place when a line is successfully obtained.
    pub async fn attach_humor(
        &mut self,
        opp: &mut BuddyOpportunity,
        primary_kind: BuddyFactKind,
        pulse: &BuddyPulse,
        gcx: Arc<RwLock<GlobalContext>>,
    ) {
        let now = Utc::now();
        self.reset_hour_if_needed(now);
        self.cache_purge_expired(now);

        if let Some(line) = self.cache_pop_line(primary_kind) {
            opp.humor = Some(line);
            return;
        }

        if self.used_this_hour >= HUMOR_BUDGET_PER_HOUR {
            return;
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

        let lines = self
            .generator
            .generate(primary_kind, pulse_summary, gcx)
            .await;
        if lines.is_empty() {
            debug!(
                "buddy humor: generator returned no lines for {:?}",
                primary_kind
            );
            return;
        }

        let batch = HumorBatch {
            fact_kind: primary_kind,
            lines,
            used: 0,
            generated_at: now,
            expires_at: now + HUMOR_BATCH_TTL,
        };
        self.cache.insert(primary_kind, batch);
        self.used_this_hour += 1;

        if let Some(line) = self.cache_pop_line(primary_kind) {
            opp.humor = Some(line);
        }
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

    let result = match crate::subchat::run_subchat_once(gcx, "buddy_humor", messages).await {
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
