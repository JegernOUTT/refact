use chrono::{Datelike, Utc};

use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, BUDDY_REFACTOR_HUNTER_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::app_state::AppState;

pub struct BuddyRefactorHunterJob;

const COOLDOWN_SECONDS: u64 = 7 * 24 * 60 * 60;
const PRIORITY: u32 = 6;

fn week_key() -> String {
    let week = Utc::now().iso_week();
    format!("{}-{:02}", week.year(), week.week())
}

fn build_refactor_hunter_spec(ctx: &BuddyJobContext) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(BUDDY_REFACTOR_HUNTER_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    let evidence = format!("project_root={}\nweek={}", project_root, week_key());
    AutonomousBuddyChatSpec::new(
        meta.id,
        meta.title,
        "Run a weekly low-risk refactor hunt and pick one high-confidence cleanup candidate.",
        evidence,
    )
    .with_display(meta.icon, meta.badge, meta.priority)
    .with_project_root(project_root)
}

#[async_trait::async_trait]
impl BuddyJob for BuddyRefactorHunterJob {
    fn id(&self) -> &str {
        BUDDY_REFACTOR_HUNTER_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        COOLDOWN_SECONDS
    }

    fn priority(&self) -> u32 {
        PRIORITY
    }

    async fn should_run(&self, _gcx: AppState, _ctx: &BuddyJobContext) -> bool {
        true
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        execute_autonomous_spec(gcx, &ctx, build_refactor_hunter_spec(&ctx)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::settings::BuddySettings;
    use crate::buddy::types::{BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse};
    use std::path::Path;

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
    async fn buddy_refactor_hunter_respects_7d_cooldown() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let ctx = test_context(dir.path());
        let job = BuddyRefactorHunterJob;

        assert_eq!(job.cooldown_seconds(), 7 * 24 * 60 * 60);
        assert!(job.should_run(gcx, &ctx).await);
        assert_eq!(
            build_refactor_hunter_spec(&ctx).workflow_id,
            BUDDY_REFACTOR_HUNTER_WORKFLOW_ID
        );
    }
}
