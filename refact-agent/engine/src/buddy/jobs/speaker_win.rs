use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::voice_service::{voice_service, SpeechIntent, VoiceCtx};

const CARE_MILESTONES: &[u64] = &[100, 200];
const WORKFLOW_RUN_MILESTONES: &[u64] = &[10, 50, 100];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SpeakerWinLastResult {
    care_score: u64,
    workflow_runs: u64,
}

pub struct SpeakerWinJob;

#[async_trait::async_trait]
impl BuddyJob for SpeakerWinJob {
    fn id(&self) -> &str {
        "speaker_win"
    }

    fn cooldown_seconds(&self) -> u64 {
        0
    }

    fn priority(&self) -> u32 {
        19
    }

    async fn should_run(
        &self,
        _gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        ctx: &BuddyJobContext,
    ) -> bool {
        crossed_milestone(ctx).is_some()
    }

    async fn execute(
        &self,
        gcx: Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let Some(summary) = crossed_milestone(&ctx) else {
            return BuddyJobResult::default();
        };
        let voice = voice_service().await;
        let speech = voice
            .render_speech(
                gcx,
                VoiceCtx {
                    persona: &ctx.personality,
                    identity_name: &ctx.identity_name,
                    pulse_one_liner: pulse_summary(&ctx),
                    workflow_id: None,
                    workflow_summary: Some(&summary),
                },
                SpeechIntent::Win,
            )
            .await;
        BuddyJobResult {
            speech_intent: Some(SpeechIntent::Win),
            speech: Some(speech),
            last_result: serde_json::to_string(&current_state(&ctx)).ok(),
            ..Default::default()
        }
    }
}

fn crossed_milestone(ctx: &BuddyJobContext) -> Option<String> {
    let previous = previous_state(ctx);
    let current = current_state(ctx);
    if let Some(milestone) = CARE_MILESTONES
        .iter()
        .copied()
        .find(|milestone| previous.care_score < *milestone && current.care_score >= *milestone)
    {
        return Some(format!("care score reached {milestone}"));
    }
    WORKFLOW_RUN_MILESTONES
        .iter()
        .copied()
        .find(|milestone| {
            previous.workflow_runs < *milestone && current.workflow_runs >= *milestone
        })
        .map(|milestone| format!("workflow runs reached {milestone}"))
}

fn current_state(ctx: &BuddyJobContext) -> SpeakerWinLastResult {
    SpeakerWinLastResult {
        care_score: ctx.pet.evolution.care_score,
        workflow_runs: ctx.total_workflow_runs,
    }
}

fn previous_state(ctx: &BuddyJobContext) -> SpeakerWinLastResult {
    ctx.job_state
        .last_result
        .as_deref()
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or_default()
}

fn pulse_summary(ctx: &BuddyJobContext) -> String {
    format!(
        "care:{}, workflows:{}",
        ctx.pet.evolution.care_score, ctx.total_workflow_runs
    )
}
