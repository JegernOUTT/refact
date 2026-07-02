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
        execute_autonomous_spec(gcx, &ctx, build_idle_suggester_spec(&ctx, last_ts), self.cooldown_seconds()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::settings::BuddySettings;
    use crate::buddy::types::{BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse};
    use crate::buddy::user_activity::UserActivityRing;

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

    fn init_temp_git_repo() -> (tempfile::TempDir, git2::Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("tracked.txt"), "original\n").unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        drop(index);
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        drop(tree);
        (dir, repo)
    }

    async fn set_activity(gcx: AppState, root: &Path, ts: DateTime<Utc>) {
        let ring_arc = gcx.buddy.user_activity.clone();
        let mut ring = ring_arc.lock().await;
        *ring = UserActivityRing::new(root.to_path_buf(), 200);
        ring.push(UserAction::FileOpened {
            path: "src/main.rs".to_string(),
            ts,
        });
    }

    #[tokio::test]
    async fn buddy_idle_suggester_fires_in_15_to_30_min_window_with_uncommitted() {
        let (dir, _repo) = init_temp_git_repo();
        let root = dir.path();
        std::fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        set_activity(gcx.clone(), root, Utc::now() - Duration::minutes(20)).await;

        assert!(
            BuddyIdleSuggesterJob
                .should_run(gcx, &test_context(root))
                .await
        );
    }

    #[tokio::test]
    async fn buddy_idle_suggester_no_fire_without_recent_activity() {
        let (dir, _repo) = init_temp_git_repo();
        std::fs::write(dir.path().join("tracked.txt"), "changed\n").unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        assert!(
            !BuddyIdleSuggesterJob
                .should_run(gcx, &test_context(dir.path()))
                .await
        );
    }
}
