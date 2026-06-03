use chrono::{Datelike, Timelike, Utc, Weekday};

use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, BUDDY_FRIDAY_RETRO_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::app_state::AppState;

pub struct BuddyFridayRetroJob;

const COOLDOWN_SECONDS: u64 = 6 * 24 * 60 * 60;
const PRIORITY: u32 = 31;

fn digest_hour(ctx: &BuddyJobContext) -> u32 {
    ctx.settings.daily_digest_hour.unwrap_or(18).min(23) as u32
}

fn should_run_at(ctx: &BuddyJobContext, now: chrono::DateTime<Utc>) -> bool {
    now.weekday() == Weekday::Fri && now.hour() >= digest_hour(ctx)
}

fn build_friday_retro_spec(ctx: &BuddyJobContext) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(BUDDY_FRIDAY_RETRO_WORKFLOW_ID).unwrap();
    let now = Utc::now();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    let evidence = format!(
        "week_ending={}\nproject_root={}\ndigest_hour={}",
        now.date_naive(),
        project_root,
        digest_hour(ctx)
    );
    AutonomousBuddyChatSpec::new(
        meta.id,
        meta.title,
        "Summarize the week's wins, rough edges, and one tiny next-week improvement.",
        evidence,
    )
    .with_display(meta.icon, meta.badge, meta.priority)
    .with_project_root(project_root)
}

#[async_trait::async_trait]
impl BuddyJob for BuddyFridayRetroJob {
    fn id(&self) -> &str {
        BUDDY_FRIDAY_RETRO_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        COOLDOWN_SECONDS
    }

    fn priority(&self) -> u32 {
        PRIORITY
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        should_run_at(ctx, Utc::now())
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        execute_autonomous_spec(gcx, &ctx, build_friday_retro_spec(&ctx)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::buddy::settings::BuddySettings;
    use crate::buddy::types::{BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse};

    fn test_context(project_root: &Path) -> BuddyJobContext {
        BuddyJobContext {
            identity_name: "Pixel".to_string(),
            personality: Default::default(),
            onboarding: BuddyOnboarding::default(),
            recent_diagnostics: vec![],
            project_root: project_root.to_path_buf(),
            job_state: BuddyJobState::default(),
            workflow_summaries: vec![],
            total_workflow_runs: 0,
            suggestion_state: vec![],
            pet: BuddyPetState::default(),
            active_quest: None,
            settings: BuddySettings::default(),
            pulse: BuddyPulse::default(),
            facts: vec![],
            recent_activities: vec![],
        }
    }

    #[tokio::test]
    async fn buddy_friday_retro_should_run_on_friday_at_or_after_digest_hour() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let now = Utc::now();
        let mut ctx = test_context(dir.path());
        ctx.settings.daily_digest_hour = Some(now.hour() as u8);
        let expected = now.weekday() == Weekday::Fri;

        assert_eq!(
            BuddyFridayRetroJob.should_run(gcx.clone(), &ctx).await,
            expected
        );

        ctx.settings.daily_digest_hour = Some(now.hour().saturating_sub(1) as u8);
        assert_eq!(
            BuddyFridayRetroJob.should_run(gcx.clone(), &ctx).await,
            expected
        );

        ctx.settings.daily_digest_hour = Some(now.hour().saturating_add(1).min(23) as u8);
        assert_eq!(
            BuddyFridayRetroJob.should_run(gcx, &ctx).await,
            expected && now.hour() == 23
        );
        assert_eq!(BuddyFridayRetroJob.cooldown_seconds(), COOLDOWN_SECONDS);
    }

    #[test]
    fn buddy_friday_retro_runs_later_on_friday_after_digest_hour() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = test_context(dir.path());
        ctx.settings.daily_digest_hour = Some(18);
        let friday_late = chrono::DateTime::parse_from_rfc3339("2026-06-05T23:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        assert!(should_run_at(&ctx, friday_late));
    }
}
