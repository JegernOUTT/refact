use crate::app_state::AppState;

use super::super::scheduler::{
    planned_speech_runtime_event, BuddyJob, BuddyJobContext, BuddyJobResult, SpeechCandidate,
};
use super::super::voice_service::SpeechIntent;

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

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        ctx.pulse.memory.total > 0
            || ctx.pulse.memory.pending_ops > 0
            || ctx.pulse.memory.duplicate_candidates > 0
            || ctx.pulse.memory.stale_conflicts > 0
    }

    async fn execute(&self, _gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let summary = memory_summary(&ctx);
        let candidate = SpeechCandidate::from_context(
            &ctx,
            SpeechIntent::MemoryPulseCommentary,
            None,
            summary.clone(),
            summary.clone(),
            "Memory pulse note".to_string(),
            Some(summary.clone()),
        );
        BuddyJobResult {
            runtime_event: Some(planned_speech_runtime_event(self.id(), &candidate)),
            speech_candidate: Some(candidate),
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
