pub mod chat_pattern;
pub mod customization_drift;
pub mod diagnostic_cluster;
pub mod git_pressure;
pub mod mcp_auth;
pub mod memory_garden;
pub mod provider_health;
pub mod task_health;
pub mod trajectory_clutter;
pub mod worktree_hygiene;

use std::sync::Arc;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::buddy::settings::BuddySettings;
use crate::buddy::types::BuddyFact;
use crate::global_context::GlobalContext;

pub struct ObserverContext {
    pub project_root: std::path::PathBuf,
    pub now: DateTime<Utc>,
}

#[async_trait::async_trait]
pub trait BuddyObserver: Send + Sync {
    fn id(&self) -> &'static str;
    fn cadence_seconds(&self) -> u64;
    fn requires_setting(&self, settings: &BuddySettings) -> bool;
    async fn observe(
        &self,
        gcx: Arc<RwLock<GlobalContext>>,
        ctx: &ObserverContext,
    ) -> Vec<BuddyFact>;
}

pub struct Ephemeral<T>(T);

impl<T> Ephemeral<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn as_ref(&self) -> &T {
        &self.0
    }
}

impl<T> std::fmt::Debug for Ephemeral<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<ephemeral>")
    }
}

pub fn build_observer_registry() -> Vec<Arc<dyn BuddyObserver>> {
    vec![
        Arc::new(task_health::TaskHealthObserver),
        Arc::new(trajectory_clutter::TrajectoryClutterObserver),
        Arc::new(git_pressure::GitPressureObserver),
        Arc::new(worktree_hygiene::WorktreeHygieneObserver),
        Arc::new(diagnostic_cluster::DiagnosticClusterObserver),
        Arc::new(provider_health::ProviderHealthObserver),
        Arc::new(mcp_auth::McpAuthObserver),
        Arc::new(chat_pattern::ChatPatternObserver),
        Arc::new(customization_drift::CustomizationDriftObserver),
        Arc::new(memory_garden::MemoryGardenObserver),
    ]
}
