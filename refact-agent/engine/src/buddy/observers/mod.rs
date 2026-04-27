pub mod diagnostic_cluster;
pub mod git_pressure;
pub mod mcp_auth;
pub mod provider_health;
pub mod task_health;
pub mod trajectory_clutter;

use std::sync::Arc;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyPulse};
use crate::global_context::GlobalContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserverCost {
    Cheap,
    Io,
    Network,
}

pub struct ObserverContext {
    pub project_root: std::path::PathBuf,
    pub last_tick: Option<DateTime<Utc>>,
    pub now: DateTime<Utc>,
    pub current_pulse: BuddyPulse,
}

#[async_trait::async_trait]
pub trait BuddyObserver: Send + Sync {
    fn id(&self) -> &'static str;
    fn cadence_seconds(&self) -> u64;
    fn cost_class(&self) -> ObserverCost;
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
        Arc::new(diagnostic_cluster::DiagnosticClusterObserver),
        Arc::new(provider_health::ProviderHealthObserver),
        Arc::new(mcp_auth::McpAuthObserver),
    ]
}
