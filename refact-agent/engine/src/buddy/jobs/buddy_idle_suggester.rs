use std::path::Path;

use chrono::{DateTime, Duration, Utc};

use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, BUDDY_IDLE_SUGGESTER_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use refact_buddy_core::user_action::UserAction;
use crate::app_state::AppState;

pub struct BuddyIdleSuggesterJob;

const COOLDOWN_SECONDS: u64 = 30 * 60;
const PRIORITY: u32 = 32;

fn newest_action_ts(actions: &[UserAction]) -> Option<DateTime<Utc>> {
    actions.iter().map(UserAction::ts).max()
}

fn idle_window_matches(last_ts: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    let idle_for = now.signed_duration_since(last_ts);
    idle_for >= Duration::minutes(15) && idle_for <= Duration::minutes(30)
}

fn has_uncommitted_changes(project_root: &Path) -> bool {
    !crate::worktrees::git::run_git_lossy(project_root, &["status", "--porcelain"])
        .trim()
        .is_empty()
}

fn build_idle_suggester_spec(
    ctx: &BuddyJobContext,
    last_ts: DateTime<Utc>,
) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(BUDDY_IDLE_SUGGESTER_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    let idle_minutes = Utc::now()
        .signed_duration_since(last_ts)
        .num_minutes()
        .max(0);
    let evidence = format!(
        "project_root={}\nlast_activity_at={}\nidle_minutes={}\nuncommitted_changes=true",
        project_root,
        last_ts.to_rfc3339(),
        idle_minutes
    );
    AutonomousBuddyChatSpec::new(
        meta.id,
        meta.title,
        "Offer one tiny, useful next-step suggestion after a short idle window with uncommitted work.",
        evidence,
    )
    .with_display(meta.icon, meta.badge, meta.priority)
    .with_project_root(project_root)
}

async fn latest_activity_ts(gcx: AppState) -> Option<DateTime<Utc>> {
    let ring_arc = gcx.buddy.user_activity.clone();
    let ring = ring_arc.lock().await;
    newest_action_ts(&ring.snapshot())
}

#[async_trait::async_trait]
impl BuddyJob for BuddyIdleSuggesterJob {
    fn id(&self) -> &str {
        BUDDY_IDLE_SUGGESTER_WORKFLOW_ID
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

    async fn should_run(&self, gcx: AppState, ctx: &BuddyJobContext) -> bool {
        let Some(last_ts) = latest_activity_ts(gcx).await else {
            return false;
        };
        if !idle_window_matches(last_ts, Utc::now()) {
            return false;
        }
        let root = ctx.project_root.clone();
        tokio::task::spawn_blocking(move || has_uncommitted_changes(&root))
            .await
            .unwrap_or(false)
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let Some(last_ts) = latest_activity_ts(gcx.clone()).await else {
            return BuddyJobResult::default();
        };
        if !idle_window_matches(last_ts, Utc::now()) {
            return BuddyJobResult::default();
        }
        execute_autonomous_spec(
            gcx,
            &ctx,
            build_idle_suggester_spec(&ctx, last_ts),
            self.cooldown_seconds(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
