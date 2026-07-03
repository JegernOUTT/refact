use chrono::{DateTime, Datelike, Local};
use std::collections::HashMap;

use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, BUDDY_REFACTOR_HUNTER_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::app_state::AppState;

pub struct BuddyRefactorHunterJob;

const COOLDOWN_SECONDS: u64 = 0;
const PRIORITY: u32 = 6;

fn week_key_at(now: &DateTime<Local>) -> String {
    let week = now.iso_week();
    format!("{}-{:02}", week.year(), week.week())
}

fn week_key() -> String {
    week_key_at(&Local::now())
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

fn top_diagnostic_counts(ctx: &BuddyJobContext) -> String {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for diag in &ctx.recent_diagnostics {
        *counts.entry(diag.error_type.clone()).or_default() += 1;
    }
    let mut counts = counts.into_iter().collect::<Vec<_>>();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
        .into_iter()
        .take(3)
        .map(|(error_type, count)| format!("{}={}", error_type, count))
        .collect::<Vec<_>>()
        .join(", ")
}

fn build_refactor_hunter_spec(ctx: &BuddyJobContext) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(BUDDY_REFACTOR_HUNTER_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    let evidence = format!(
        "project_root={}\nweek={}\ngit_uncommitted={} git_diff_lines_4h={}\ntop_diagnostics={}",
        project_root,
        week_key(),
        ctx.pulse.git.uncommitted_files,
        ctx.pulse.git.diff_lines_4h,
        top_diagnostic_counts(ctx)
    );
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

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        !week_key_recorded(ctx, &week_key())
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let key = week_key();
        let mut result = execute_autonomous_spec(
            gcx,
            &ctx,
            build_refactor_hunter_spec(&ctx),
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
    use chrono::TimeZone;
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
    async fn buddy_refactor_hunter_uses_calendar_week_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let ctx = test_context(dir.path());
        let job = BuddyRefactorHunterJob;

        assert_eq!(job.cooldown_seconds(), COOLDOWN_SECONDS);
        assert!(job.should_run(gcx, &ctx).await);
        assert_eq!(
            build_refactor_hunter_spec(&ctx).workflow_id,
            BUDDY_REFACTOR_HUNTER_WORKFLOW_ID
        );
    }

    #[test]
    fn buddy_refactor_hunter_dedupes_on_local_week_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = test_context(dir.path());
        let now = Local
            .with_ymd_and_hms(2026, 6, 5, 12, 0, 0)
            .single()
            .unwrap();
        let key = week_key_at(&now);
        ctx.job_state.last_result = Some(key.clone());

        assert_eq!(key, week_key_at(&now));
        assert!(week_key_recorded(&ctx, &key));
    }
}
