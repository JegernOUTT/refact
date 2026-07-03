use chrono::Timelike;

use crate::app_state::AppState;
use crate::buddy::briefing::{assemble_briefing, briefing_exists, save_briefing};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};

pub struct BuddyBriefingJob;

const COOLDOWN_SECONDS: u64 = 3600;
const PRIORITY: u32 = 20;
pub const BRIEFING_READY_HOUR: u32 = 7;

fn today_local() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

#[async_trait::async_trait]
impl BuddyJob for BuddyBriefingJob {
    fn id(&self) -> &str {
        "buddy_briefing"
    }

    fn cooldown_seconds(&self) -> u64 {
        COOLDOWN_SECONDS
    }

    fn priority(&self) -> u32 {
        PRIORITY
    }

    fn records_empty_result(&self) -> bool {
        false
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        if chrono::Local::now().hour() < BRIEFING_READY_HOUR {
            return false;
        }
        !briefing_exists(&ctx.project_root, &today_local()).await
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let receipts = crate::buddy::receipts::load_receipts(&ctx.project_root).await;
        let snapshot = {
            let buddy_arc = gcx.buddy.buddy.clone();
            let lock = buddy_arc.lock().await;
            lock.as_ref().map(|svc| {
                (
                    svc.state.clone(),
                    svc.pulse.clone(),
                    svc.opportunity_queue.snapshot(),
                )
            })
        };
        let Some((state, pulse, opportunities)) = snapshot else {
            return BuddyJobResult::default();
        };
        let date = today_local();
        let briefing = assemble_briefing(&state, &pulse, &opportunities, receipts, date.clone());
        if let Err(err) = save_briefing(&ctx.project_root, &briefing).await {
            tracing::warn!("buddy: failed to save briefing: {}", err);
            return BuddyJobResult {
                last_result: Some(format!("failed:{}", date)),
                ..Default::default()
            };
        }
        let mut event = crate::buddy::actor::make_runtime_event(
            "buddy_briefing",
            "Morning briefing ready",
            self.id(),
            &format!("buddy_briefing:{}", date),
            "completed",
            Some("low"),
        );
        event.description = Some(format!(
            "{} pending card(s), {} job(s) ran, {} tokens spent yesterday-to-date",
            briefing.top_cards.len(),
            briefing.job_runs.len(),
            briefing.spend.total_tokens()
        ));
        event.bubble_policy = Some(crate::buddy::types::BuddyBubblePolicy::Ambient);
        event.controls = vec![crate::buddy::types::BuddyControl {
            id: "open-briefing".to_string(),
            label: "Open briefing".to_string(),
            action: "open_buddy".to_string(),
            action_param: None,
            style: "primary".to_string(),
        }];
        BuddyJobResult {
            runtime_event: Some(event),
            last_result: Some(date),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn briefing_job_metadata() {
        let job = BuddyBriefingJob;
        assert_eq!(job.id(), "buddy_briefing");
        assert_eq!(job.cooldown_seconds(), 3600);
        assert!(!job.records_empty_result());
    }
}
