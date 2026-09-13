use crate::app_state::AppState;

use super::super::scheduler::{
    planned_speech_runtime_event, BuddyJob, BuddyJobContext, BuddyJobResult, SpeechCandidate,
};
use super::super::voice_service::SpeechIntent;

pub struct SpeakerInsightJob;

#[async_trait::async_trait]
impl BuddyJob for SpeakerInsightJob {
    fn id(&self) -> &str {
        "speaker_insight"
    }

    fn cooldown_seconds(&self) -> u64 {
        0
    }

    fn priority(&self) -> u32 {
        18
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        let seen = seen_count(ctx);
        ctx.workflow_summaries.len() > seen
    }

    async fn execute(&self, _gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let current = ctx.workflow_summaries.len();
        let seen = seen_count(&ctx);
        if current <= seen {
            return BuddyJobResult::default();
        }
        let latest = ctx
            .workflow_summaries
            .last()
            .map(|summary| {
                format!(
                    "{} finished with {}",
                    summary.workflow_id,
                    summary.last_outcome.as_deref().unwrap_or("an update")
                )
            })
            .unwrap_or_else(|| "a workflow completed".to_string());
        let workflow_id = ctx
            .workflow_summaries
            .last()
            .map(|summary| summary.workflow_id.clone());
        let candidate = SpeechCandidate::from_context(
            &ctx,
            SpeechIntent::Insight,
            workflow_id,
            latest.clone(),
            latest,
            "Buddy insight ready".to_string(),
            None,
        );
        BuddyJobResult {
            runtime_event: Some(planned_speech_runtime_event(self.id(), &candidate)),
            speech_candidate: Some(candidate),
            last_result: Some(current.to_string()),
            ..Default::default()
        }
    }
}

fn seen_count(ctx: &BuddyJobContext) -> usize {
    ctx.job_state
        .last_result
        .as_deref()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}
