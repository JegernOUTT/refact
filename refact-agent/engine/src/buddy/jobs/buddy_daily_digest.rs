use super::{recent_activity_evidence, trusted_git_output};

use chrono::{DateTime, Local, Timelike, Utc};

use crate::app_state::AppState;
use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, BUDDY_DAILY_DIGEST_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::buddy::settings::BuddySettings;

pub struct BuddyDailyDigestJob;

const COOLDOWN_SECONDS: u64 = 0;
const PRIORITY: u32 = 30;

fn digest_hour(settings: &BuddySettings) -> u32 {
    settings.daily_digest_hour.unwrap_or(18).min(23) as u32
}

fn date_key(now: &DateTime<Local>) -> String {
    now.date_naive().to_string()
}

fn date_key_recorded(ctx: &BuddyJobContext, key: &str) -> bool {
    ctx.job_state.last_result.as_deref() == Some(key)
}

fn result_ran(result: &BuddyJobResult) -> bool {
    result.activity.is_some()
        || result.runtime_event.is_some()
        || result.workflow_failure.is_some()
        || result.xp > 0
}

fn should_run_at(ctx: &BuddyJobContext, now: DateTime<Local>) -> bool {
    let key = date_key(&now);
    !date_key_recorded(ctx, &key) && now.hour() >= digest_hour(&ctx.settings)
}

async fn daily_git_evidence(ctx: &BuddyJobContext, now: DateTime<Utc>) -> String {
    let since = format!("{} 00:00", now.date_naive());
    trusted_git_output(
        &ctx.project_root,
        &["log", "--oneline", "-20", "--since", &since],
    )
    .await
    .unwrap_or_else(|| "none".to_string())
}

async fn build_daily_digest_spec(
    ctx: &BuddyJobContext,
    now: DateTime<Utc>,
) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(BUDDY_DAILY_DIGEST_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    let evidence = format!(
        "date={}\nproject_root={}\ndigest_hour={}\n\nRecent git commits since local day start:\n{}\n\nRecent Buddy activity / saved chats:\n{}",
        now.date_naive(),
        project_root,
        digest_hour(&ctx.settings),
        daily_git_evidence(ctx, now).await,
        recent_activity_evidence(ctx, 10)
    );
    AutonomousBuddyChatSpec::new(
        meta.id,
        meta.title,
        "Summarize the day's work and log a concise end-of-day Buddy activity.",
        evidence,
    )
    .with_display(meta.icon, meta.badge, meta.priority)
    .with_project_root(project_root)
}

#[async_trait::async_trait]
impl BuddyJob for BuddyDailyDigestJob {
    fn id(&self) -> &str {
        BUDDY_DAILY_DIGEST_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        COOLDOWN_SECONDS
    }

    fn priority(&self) -> u32 {
        PRIORITY
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        should_run_at(ctx, Local::now())
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let key = date_key(&Local::now());
        let mut result = execute_autonomous_spec(
            gcx,
            &ctx,
            build_daily_digest_spec(&ctx, Utc::now()).await,
            self.cooldown_seconds(),
        )
        .await;
        if result_ran(&result) {
            result.last_result = Some(key);
        }
        result
    }
}
