use std::sync::Arc;

use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::voice_service::{voice_service, SpeechIntent, VoiceCtx};

pub struct SpeakerMemoryPulseCommentaryJob;

#[async_trait::async_trait]
impl BuddyJob for SpeakerMemoryPulseCommentaryJob {
    fn id(&self) -> &str {
        "speaker_memory_pulse_commentary"
    }

    fn cooldown_seconds(&self) -> u64 {
        6 * 60 * 60
    }

    fn priority(&self) -> u32 {
        20
    }

    async fn should_run(
        &self,
        _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        ctx: &BuddyJobContext,
    ) -> bool {
        ctx.pulse.memory.total > 0
            || ctx.pulse.memory.pending_ops > 0
            || ctx.pulse.memory.duplicate_candidates > 0
            || ctx.pulse.memory.stale_conflicts > 0
    }

    async fn execute(
        &self,
        gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let summary = memory_summary(&ctx);
        let voice = voice_service().await;
        let speech = voice
            .render_speech(
                gcx,
                VoiceCtx {
                    persona: &ctx.personality,
                    identity_name: &ctx.identity_name,
                    pulse_one_liner: summary.clone(),
                    workflow_id: None,
                    workflow_summary: Some(&summary),
                },
                SpeechIntent::MemoryPulseCommentary,
            )
            .await;
        BuddyJobResult {
            speech_intent: Some(SpeechIntent::MemoryPulseCommentary),
            speech: Some(speech),
            last_result: Some(summary),
            ..Default::default()
        }
    }
}

fn memory_summary(ctx: &BuddyJobContext) -> String {
    format!(
        "memories:{}, pending:{}, duplicates:{}, conflicts:{}",
        ctx.pulse.memory.total,
        ctx.pulse.memory.pending_ops,
        ctx.pulse.memory.duplicate_candidates,
        ctx.pulse.memory.stale_conflicts
    )
}
