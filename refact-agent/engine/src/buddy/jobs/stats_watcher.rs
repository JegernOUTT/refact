use std::sync::Arc;
use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};

pub struct StatsWatcherJob;

#[async_trait::async_trait]
impl BuddyJob for StatsWatcherJob {
    fn id(&self) -> &str { "stats_watcher" }
    fn cooldown_seconds(&self) -> u64 { 1800 }
    fn priority(&self) -> u32 { 5 }

    async fn should_run(&self, _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>, _ctx: &BuddyJobContext) -> bool {
        true
    }

    async fn execute(&self, _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>, _ctx: BuddyJobContext) -> BuddyJobResult {
        BuddyJobResult::default()
    }
}
