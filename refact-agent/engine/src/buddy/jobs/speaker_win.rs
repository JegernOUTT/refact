use crate::app_state::AppState;

use serde::{Deserialize, Serialize};

use super::super::scheduler::{
    planned_speech_runtime_event, BuddyJob, BuddyJobContext, BuddyJobResult, SpeechCandidate,
};
use super::super::voice_service::SpeechIntent;

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

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        crossed_milestone(ctx).is_some()
    }

    async fn execute(&self, _gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let Some(summary) = crossed_milestone(&ctx) else {
            return BuddyJobResult::default();
        };
        let candidate = SpeechCandidate::from_context(
            &ctx,
            SpeechIntent::Win,
            None,
            summary.clone(),
            summary,
            "Buddy win ready".to_string(),
            None,
        );
        BuddyJobResult {
            runtime_event: Some(planned_speech_runtime_event(self.id(), &candidate)),
            speech_candidate: Some(candidate),
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
