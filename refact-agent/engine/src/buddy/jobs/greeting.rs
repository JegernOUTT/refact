use crate::app_state::AppState;

use super::super::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult, SpeechCandidate};
use super::super::types::{BuddyControl, BuddySpeechItem};
use crate::buddy::voice_service::SpeechIntent;

fn greeting_fallback_text(ctx: &BuddyJobContext) -> String {
    if !ctx.onboarding.greeted {
        format!("Hi! I'm {}! 🎉 I'll help you with errors, setup, and staying productive. Want a quick tour?", ctx.identity_name)
    } else {
        format!(
            "Welcome back! 👋 I'm {} — ready to help.",
            ctx.identity_name
        )
    }
}

fn greeting_controls(ctx: &BuddyJobContext) -> Vec<BuddyControl> {
    if !ctx.onboarding.greeted {
        vec![
            BuddyControl {
                id: "tour".to_string(),
                label: "Take a Tour".to_string(),
                action: "open_buddy".to_string(),
                action_param: None,
                style: "primary".to_string(),
            },
            BuddyControl {
                id: "skip".to_string(),
                label: "Skip".to_string(),
                action: "dismiss".to_string(),
                action_param: None,
                style: "secondary".to_string(),
            },
        ]
    } else {
        vec![]
    }
}

fn greeting_workflow_summary(
    ctx: &BuddyJobContext,
    pulse_one_liner: &str,
    fallback: &str,
) -> String {
    ctx.workflow_summaries
        .last()
        .map(|summary| {
            let status = summary
                .last_outcome
                .as_deref()
                .or(summary.failure_summary.as_deref())
                .unwrap_or("unknown");
            format!(
                "recent workflow: {} {}; {}",
                summary.workflow_id, status, pulse_one_liner
            )
        })
        .unwrap_or_else(|| fallback.to_string())
}

pub struct GreetingJob;

#[async_trait::async_trait]
impl BuddyJob for GreetingJob {
    fn id(&self) -> &str {
        "greeting"
    }
    fn cooldown_seconds(&self) -> u64 {
        86400
    }
    fn priority(&self) -> u32 {
        0
    }

    fn records_empty_result(&self) -> bool {
        false
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        !ctx.onboarding.greeted || ctx.job_state.last_run.is_none()
    }

    async fn execute(&self, _gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let fallback_text = greeting_fallback_text(&ctx);
        let controls = greeting_controls(&ctx);
        let mut candidate = SpeechCandidate::from_context(
            &ctx,
            SpeechIntent::Greeting,
            Some("greeting".into()),
            greeting_workflow_summary(
                &ctx,
                &format!(
                    "{} pending ops, {} recent stuck task alerts",
                    ctx.pulse.memory.pending_ops,
                    ctx.pulse.tasks.recent_stuck_alert_count_1h()
                ),
                &fallback_text,
            ),
            fallback_text.clone(),
            "Buddy greeting".into(),
            None,
        );
        candidate.template = Some(BuddySpeechItem {
            editorial_delivery_required: false,
            id: format!("greeting-{}", chrono::Utc::now().timestamp()),
            text: fallback_text,
            mood: "happy".to_string(),
            scope: "global".to_string(),
            persistent: false,
            ttl_seconds: if ctx.onboarding.greeted { 8 } else { 15 },
            dedupe_key: Some("greeting".to_string()),
            created_at: chrono::Utc::now().to_rfc3339(),
            controls,
            chat_id: None,
            speech_intent: None,
        });
        BuddyJobResult {
            speech_candidate: Some(candidate),
            mark_greeted: true,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::scheduler::BuddyJobContext;
}
