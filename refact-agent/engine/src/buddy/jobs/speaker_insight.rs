use crate::app_state::AppState;

use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::voice_service::{voice_service, SpeechIntent, VoiceCtx};

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

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
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
        let voice = voice_service().await;
        let speech = voice
            .render_speech(
                gcx,
                VoiceCtx {
                    persona: &ctx.personality,
                    identity_name: &ctx.identity_name,
                    pulse_one_liner: pulse_summary(&ctx),
                    workflow_id: ctx
                        .workflow_summaries
                        .last()
                        .map(|summary| summary.workflow_id.as_str()),
                    workflow_summary: Some(&latest),
                },
                SpeechIntent::Insight,
            )
            .await;
        BuddyJobResult {
            speech_intent: Some(SpeechIntent::Insight),
            runtime_event: Some(super::super::scheduler::speech_runtime_event(
                self.id(),
                SpeechIntent::Insight,
                &speech,
                "Buddy insight ready".to_string(),
                Some(latest),
            )),
            speech: Some(speech),
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

fn pulse_summary(ctx: &BuddyJobContext) -> String {
    format!(
        "workflows:{}, tasks:{}, memories:{}",
        ctx.total_workflow_runs, ctx.pulse.tasks.total, ctx.pulse.memory.total
    )
}
