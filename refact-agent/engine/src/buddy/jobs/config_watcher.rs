use std::sync::Arc;
use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::BuddySuggestion;

pub struct ConfigWatcherJob;

#[async_trait::async_trait]
impl BuddyJob for ConfigWatcherJob {
    fn id(&self) -> &str { "config_watcher" }
    fn cooldown_seconds(&self) -> u64 { 600 }
    fn priority(&self) -> u32 { 3 }

    async fn should_run(&self, _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>, _ctx: &BuddyJobContext) -> bool {
        true
    }

    async fn execute(&self, _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>, ctx: BuddyJobContext) -> BuddyJobResult {
        let agents_md = ctx.project_root.join("AGENTS.md");
        if !agents_md.exists() {
            return BuddyJobResult {
                suggestion: Some(BuddySuggestion {
                    id: format!("config-{}", chrono::Utc::now().timestamp()),
                    suggestion_type: "setup".to_string(),
                    title: "Project setup incomplete".to_string(),
                    description: "Your project doesn't have an AGENTS.md yet. Want me to help create one?".to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    dismissed: false,
                }),
                ..Default::default()
            };
        }
        BuddyJobResult::default()
    }
}
