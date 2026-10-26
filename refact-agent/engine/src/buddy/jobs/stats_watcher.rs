use crate::app_state::AppState;

use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use super::super::types::{BuddyActivity, BuddySpeechItem, BuddySuggestion};
use crate::buddy::voice_service::{SpeechIntent, VoiceCtx, voice_service};

const WORKFLOW_MILESTONES: &[u64] = &[10, 50, 100, 500];
const RECENT_ERROR_WINDOW_SECS: i64 = 3600;

pub struct StatsWatcherJob;

#[async_trait::async_trait]
impl BuddyJob for StatsWatcherJob {
    fn id(&self) -> &str {
        "stats_watcher"
    }
    fn cooldown_seconds(&self) -> u64 {
        1800
    }
    fn priority(&self) -> u32 {
        5
    }
    fn produces_suggestion(&self) -> bool {
        true
    }
    fn runs_when_suggestions_blocked(&self) -> bool {
        true
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        stats_watcher_has_visible_output(ctx)
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let runs = ctx.total_workflow_runs;

        if runs == 0 {
            return BuddyJobResult::default();
        }

        let cutoff = chrono::Utc::now().timestamp() - RECENT_ERROR_WINDOW_SECS;
        let recent_error_count = ctx
            .recent_diagnostics
            .iter()
            .filter(|d| {
                chrono::DateTime::parse_from_rfc3339(&d.collected_at)
                    .map(|t| t.timestamp() >= cutoff)
                    .unwrap_or(false)
            })
            .count();

        if recent_error_count >= 5 {
            return BuddyJobResult {
                suggestion: Some(BuddySuggestion {
                    id: "stats-errors".to_string(),
                    suggestion_type: "error_pattern".to_string(),
                    title: format!(
                        "{} fresh error gremlins in the last hour",
                        recent_error_count
                    ),
                    description: "The log closet is rattling again. Want me to collect clues and draft an issue-shaped net?".to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    dismissed: false,
                    controls: vec![],
                    quest: None,
                }),
                last_result: Some(runs.to_string()),
                ..Default::default()
            };
        }

        let prev_runs: u64 = ctx
            .job_state
            .last_result
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        for &m in WORKFLOW_MILESTONES {
            if prev_runs < m && runs >= m {
                if crate::buddy::editorial::enabled(&gcx).await {
                    return BuddyJobResult {
                        activity: Some(BuddyActivity {
                            icon: "🎉".into(),
                            title: format!("Milestone: {} tasks completed!", m),
                            description: format!("{} workflows have finished successfully.", m),
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            activity_type: "milestone".into(),
                            chat_id: None,
                            failure_category: None,
                            failure_summary: None,
                        }),
                        last_result: Some(runs.to_string()),
                        ..Default::default()
                    };
                }
                let fallback_text = format!("We've completed {} tasks together!", m);
                let mut speech = match crate::buddy::actor::buddy_snapshot(gcx.clone()).await {
                    Some(snapshot) => {
                        let pulse_one_liner = format!(
                            "{} pending ops, {} recent stuck task alerts",
                            snapshot.pulse.memory.pending_ops,
                            snapshot.pulse.tasks.recent_stuck_alert_count_1h()
                        );
                        let voice_ctx = VoiceCtx {
                            persona: &snapshot.state.personality,
                            identity_name: snapshot.state.identity.name.as_str(),
                            pulse_one_liner,
                            workflow_id: Some("stats_watcher"),
                            workflow_summary: Some(&fallback_text),
                        };
                        voice_service()
                            .await
                            .render_speech(gcx.clone(), voice_ctx, SpeechIntent::Milestone)
                            .await
                    }
                    None => BuddySpeechItem {
                        editorial_delivery_required: false,
                        id: format!("stats-milestone-{}", m),
                        text: fallback_text,
                        mood: "happy".to_string(),
                        scope: "global".to_string(),
                        persistent: false,
                        ttl_seconds: 12,
                        dedupe_key: Some(format!("milestone_{}", m)),
                        created_at: chrono::Utc::now().to_rfc3339(),
                        controls: vec![],
                        chat_id: None,
                        speech_intent: None,
                    },
                };
                speech.ttl_seconds = 12;
                speech.dedupe_key = Some(format!("milestone_{}", m));
                let activity = BuddyActivity {
                    icon: "🎉".to_string(),
                    title: format!("Milestone: {} tasks completed!", m),
                    description: format!("{} workflows have finished successfully.", m),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    activity_type: "milestone".to_string(),
                    chat_id: None,
                    failure_category: None,
                    failure_summary: None,
                };
                return BuddyJobResult {
                    speech_intent: Some(super::super::voice_service::SpeechIntent::Milestone),
                    runtime_event: Some(super::super::scheduler::speech_runtime_event(
                        self.id(),
                        SpeechIntent::Milestone,
                        &speech,
                        format!("Milestone: {} tasks completed!", m),
                        Some(format!("{} workflows have finished successfully.", m)),
                    )),
                    speech: Some(speech),
                    activity: Some(activity),
                    last_result: Some(runs.to_string()),
                    ..Default::default()
                };
            }
        }

        BuddyJobResult {
            last_result: Some(runs.to_string()),
            ..Default::default()
        }
    }
}

fn stats_watcher_has_visible_output(ctx: &BuddyJobContext) -> bool {
    if ctx.total_workflow_runs == 0 {
        return false;
    }
    let cutoff = chrono::Utc::now().timestamp() - RECENT_ERROR_WINDOW_SECS;
    let recent_error_count = ctx
        .recent_diagnostics
        .iter()
        .filter(|d| {
            chrono::DateTime::parse_from_rfc3339(&d.collected_at)
                .map(|t| t.timestamp() >= cutoff)
                .unwrap_or(false)
        })
        .count();
    if recent_error_count >= 5 {
        return true;
    }

    let prev_runs: u64 = ctx
        .job_state
        .last_result
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    WORKFLOW_MILESTONES
        .iter()
        .any(|&m| prev_runs < m && ctx.total_workflow_runs >= m)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::buddy::types::{
        BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse, BuddySuggestion,
    };
}
