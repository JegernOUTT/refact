use std::sync::Arc;
use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::{BuddySpeechItem, BuddyControl};

pub struct GreetingJob;

#[async_trait::async_trait]
impl BuddyJob for GreetingJob {
    fn id(&self) -> &str { "greeting" }
    fn cooldown_seconds(&self) -> u64 { 86400 }
    fn priority(&self) -> u32 { 0 }

    async fn should_run(&self, _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>, ctx: &BuddyJobContext) -> bool {
        !ctx.onboarding.greeted || ctx.job_state.last_run.is_none()
    }

    async fn execute(&self, _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>, ctx: BuddyJobContext) -> BuddyJobResult {
        let text = if !ctx.onboarding.greeted {
            format!("Hi! I'm {}, your coding buddy! 🎉 I'll help you with errors, setup, and staying productive. Want a quick tour?", ctx.identity_name)
        } else {
            format!("Welcome back! 👋 I'm {} — ready to help.", ctx.identity_name)
        };
        let controls = if !ctx.onboarding.greeted {
            vec![
                BuddyControl { id: "tour".to_string(), label: "Take a Tour".to_string(), action: "open_buddy".to_string(), action_param: None, style: "primary".to_string() },
                BuddyControl { id: "skip".to_string(), label: "Skip".to_string(), action: "dismiss".to_string(), action_param: None, style: "secondary".to_string() },
            ]
        } else {
            vec![]
        };
        BuddyJobResult {
            speech: Some(BuddySpeechItem {
                id: format!("greeting-{}", chrono::Utc::now().timestamp()),
                text,
                mood: "happy".to_string(),
                scope: "global".to_string(),
                persistent: !ctx.onboarding.greeted,
                ttl_seconds: if ctx.onboarding.greeted { 8 } else { 0 },
                dedupe_key: Some("greeting".to_string()),
                created_at: chrono::Utc::now().to_rfc3339(),
                controls,
            }),
            ..Default::default()
        }
    }
}
