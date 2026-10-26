use std::path::Path;

use chrono::{Datelike, Local, Timelike, Utc, Weekday};

use crate::app_state::AppState;
use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, BUDDY_FRIDAY_RETRO_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};

pub struct BuddyFridayRetroJob;

const COOLDOWN_SECONDS: u64 = 0;
const PRIORITY: u32 = 31;
const TRUSTED_COMMAND_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

fn digest_hour(ctx: &BuddyJobContext) -> u32 {
    ctx.settings.daily_digest_hour.unwrap_or(18).min(23) as u32
}

fn week_key_at(now: &chrono::DateTime<Local>) -> String {
    let week = now.iso_week();
    format!("{}-{:02}", week.year(), week.week())
}

fn week_key_recorded(ctx: &BuddyJobContext, key: &str) -> bool {
    ctx.job_state.last_result.as_deref() == Some(key)
}

fn result_ran(result: &BuddyJobResult) -> bool {
    result.activity.is_some()
        || result.runtime_event.is_some()
        || result.workflow_failure.is_some()
        || result.xp > 0
}

fn should_run_at(ctx: &BuddyJobContext, now: chrono::DateTime<Local>) -> bool {
    let key = week_key_at(&now);
    !week_key_recorded(ctx, &key) && now.weekday() == Weekday::Fri && now.hour() >= digest_hour(ctx)
}

async fn trusted_git_output(project_root: &Path, args: &[&str]) -> Option<String> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(project_root)
        .args(args)
        .env("PATH", TRUSTED_COMMAND_PATH)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    // Bound the git invocation so a hung repo/filesystem cannot stall the Tokio worker.
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn recent_activity_evidence(ctx: &BuddyJobContext) -> String {
    let activities = ctx
        .recent_activities
        .iter()
        .rev()
        .take(20)
        .map(|activity| {
            format!(
                "- {} [{}] {} — {}",
                activity.timestamp, activity.activity_type, activity.title, activity.description
            )
        })
        .collect::<Vec<_>>();
    if activities.is_empty() {
        "none".to_string()
    } else {
        activities.join("\n")
    }
}

async fn weekly_git_evidence(ctx: &BuddyJobContext) -> String {
    trusted_git_output(
        &ctx.project_root,
        &["log", "--oneline", "-40", "--since", "7 days ago"],
    )
    .await
    .unwrap_or_else(|| "none".to_string())
}

async fn build_friday_retro_spec(ctx: &BuddyJobContext) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(BUDDY_FRIDAY_RETRO_WORKFLOW_ID).unwrap();
    let now = Utc::now();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    let evidence = format!(
        "week_ending={}\nproject_root={}\ndigest_hour={}\n\nRecent git commits from the last 7 days:\n{}\n\nRecent Buddy activity / saved chats:\n{}",
        now.date_naive(),
        project_root,
        digest_hour(ctx),
        weekly_git_evidence(ctx).await,
        recent_activity_evidence(ctx)
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
        should_run_at(ctx, Local::now())
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let key = week_key_at(&Local::now());
        let mut result = execute_autonomous_spec(
            gcx,
            &ctx,
            build_friday_retro_spec(&ctx).await,
            self.cooldown_seconds(),
        )
        .await;
        if result_ran(&result) {
            result.last_result = Some(key);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
}
