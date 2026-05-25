use crate::app_state::AppState;
use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::BuddySuggestion;

pub struct ConfigWatcherJob;

#[async_trait::async_trait]
impl BuddyJob for ConfigWatcherJob {
    fn id(&self) -> &str {
        "config_watcher"
    }
    fn cooldown_seconds(&self) -> u64 {
        600
    }
    fn priority(&self) -> u32 {
        3
    }
    fn produces_suggestion(&self) -> bool {
        true
    }

    async fn should_run(&self, _gcx: AppState, _ctx: &BuddyJobContext) -> bool {
        true
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        if ctx.project_root.join("AGENTS.md").exists() {
            return BuddyJobResult::default();
        }
        let _ = gcx;
        BuddyJobResult {
            suggestion: Some(BuddySuggestion {
                id: format!("config-{}", chrono::Utc::now().timestamp()),
                suggestion_type: "setup".to_string(),
                title: "Project setup incomplete".to_string(),
                description:
                    "Your project doesn't have an AGENTS.md yet. Want me to help create one?"
                        .to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                dismissed: false,
                controls: vec![],
                quest: None,
            }),
            ..Default::default()
        }
    }
}
