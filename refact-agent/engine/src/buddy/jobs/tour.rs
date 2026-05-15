use std::sync::Arc;

use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::BuddySpeechItem;
use crate::buddy::voice_service::{SpeechIntent, VoiceCtx, voice_service};

pub struct TourJob;

#[async_trait::async_trait]
impl BuddyJob for TourJob {
    fn id(&self) -> &str {
        "tour"
    }
    fn cooldown_seconds(&self) -> u64 {
        86400
    }
    fn priority(&self) -> u32 {
        1
    }

    async fn should_run(
        &self,
        _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        ctx: &BuddyJobContext,
    ) -> bool {
        ctx.onboarding.greeted && !ctx.onboarding.tour_completed && ctx.job_state.last_run.is_none()
    }

    async fn execute(
        &self,
        gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        _ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let fallback_text = "This is me on your dashboard — I track everything happening in your project. Ask me about setup, skills, or MCP anytime!".to_string();
        let mut speech = match crate::buddy::actor::buddy_snapshot(gcx.clone()).await {
            Some(snapshot) => {
                let pulse_one_liner = format!(
                    "{} pending ops, {} stuck tasks",
                    snapshot.pulse.memory.pending_ops, snapshot.pulse.tasks.stuck
                );
                let voice_ctx = VoiceCtx {
                    persona: &snapshot.state.personality,
                    identity_name: snapshot.state.identity.name.as_str(),
                    pulse_one_liner,
                    workflow_id: None,
                    workflow_summary: Some(&fallback_text),
                };
                voice_service()
                    .await
                    .render_speech(gcx, voice_ctx, SpeechIntent::Tour)
                    .await
            }
            None => BuddySpeechItem {
                id: format!("tour-{}", chrono::Utc::now().timestamp()),
                text: fallback_text,
                mood: "excited".to_string(),
                scope: "global".to_string(),
                persistent: false,
                ttl_seconds: 15,
                dedupe_key: Some("tour".to_string()),
                created_at: chrono::Utc::now().to_rfc3339(),
                controls: vec![],
                chat_id: None,
            },
        };
        speech.ttl_seconds = 15;
        speech.dedupe_key = Some("tour".to_string());
        BuddyJobResult {
            speech: Some(speech),
            ..Default::default()
        }
    }
}
