use std::sync::Arc;
use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::BuddySpeechItem;

pub struct TourJob;

#[async_trait::async_trait]
impl BuddyJob for TourJob {
    fn id(&self) -> &str { "tour" }
    fn cooldown_seconds(&self) -> u64 { 0 }
    fn priority(&self) -> u32 { 1 }

    async fn should_run(&self, _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        ctx.onboarding.greeted && !ctx.onboarding.tour_completed
    }

    async fn execute(&self, _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>, ctx: BuddyJobContext) -> BuddyJobResult {
        BuddyJobResult {
            speech: Some(BuddySpeechItem {
                id: format!("tour-{}", chrono::Utc::now().timestamp()),
                text: "This is me on your dashboard — I track everything happening in your project. Ask me about setup, skills, or MCP anytime!".to_string(),
                mood: "excited".to_string(),
                scope: "global".to_string(),
                persistent: true,
                ttl_seconds: 0,
                dedupe_key: Some("tour".to_string()),
                created_at: chrono::Utc::now().to_rfc3339(),
                controls: vec![],
            }),
            ..Default::default()
        }
    }
}
