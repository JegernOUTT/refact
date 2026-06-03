use chrono::{DateTime, Timelike, Utc};

use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, BUDDY_DAILY_DIGEST_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::buddy::settings::BuddySettings;
use crate::app_state::AppState;

pub struct BuddyDailyDigestJob;

const COOLDOWN_SECONDS: u64 = 20 * 60 * 60;
const PRIORITY: u32 = 30;

fn digest_hour(settings: &BuddySettings) -> u32 {
    settings.daily_digest_hour.unwrap_or(18).min(23) as u32
}

fn should_run_at(ctx: &BuddyJobContext, now: DateTime<Utc>) -> bool {
    now.hour() >= digest_hour(&ctx.settings)
}

fn build_daily_digest_spec(ctx: &BuddyJobContext, now: DateTime<Utc>) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(BUDDY_DAILY_DIGEST_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    let evidence = format!(
        "date={}\nproject_root={}\ndigest_hour={}",
        now.date_naive(),
        project_root,
        digest_hour(&ctx.settings)
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
        should_run_at(ctx, Utc::now())
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        execute_autonomous_spec(gcx, &ctx, build_daily_digest_spec(&ctx, Utc::now())).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::buddy::autonomous_workflows::{
        BUDDY_FRIDAY_RETRO_WORKFLOW_ID, BUDDY_IDLE_SUGGESTER_WORKFLOW_ID,
        BUDDY_PR_ISSUE_MATCHMAKER_WORKFLOW_ID,
    };
    use crate::buddy::conversation_ledger::workflow_id_to_mapping;
    use crate::buddy::types::{BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse};

    fn defaults_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates")
            .join("refact-yaml-configs")
            .join("src")
            .join("defaults")
    }

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
    async fn buddy_daily_digest_should_run_at_or_after_configured_hour() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let hour = Utc::now().hour() as u8;
        let mut ctx = test_context(dir.path());
        let job = BuddyDailyDigestJob;

        ctx.settings.daily_digest_hour = Some(hour);
        assert!(job.should_run(gcx.clone(), &ctx).await);

        ctx.settings.daily_digest_hour = Some(hour.saturating_sub(1));
        assert!(job.should_run(gcx.clone(), &ctx).await);

        if hour < 23 {
            ctx.settings.daily_digest_hour = Some(hour + 1);
            assert!(!job.should_run(gcx, &ctx).await);
        }
        assert_eq!(job.cooldown_seconds(), COOLDOWN_SECONDS);
        assert_eq!(BuddySettings::default().daily_digest_hour, Some(18));
    }

    #[test]
    fn buddy_daily_digest_runs_later_same_day_after_digest_hour() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = test_context(dir.path());
        ctx.settings.daily_digest_hour = Some(18);
        let late_same_day = chrono::DateTime::parse_from_rfc3339("2026-06-05T23:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        assert!(should_run_at(&ctx, late_same_day));
    }

    #[tokio::test]
    async fn all_4_workflow_yamls_loadable() {
        let defaults_dir = defaults_dir();
        let registry =
            crate::yaml_configs::customization_registry::load_registry_from_dir(&defaults_dir)
                .await;
        let ids = [
            BUDDY_DAILY_DIGEST_WORKFLOW_ID,
            BUDDY_FRIDAY_RETRO_WORKFLOW_ID,
            BUDDY_IDLE_SUGGESTER_WORKFLOW_ID,
            BUDDY_PR_ISSUE_MATCHMAKER_WORKFLOW_ID,
        ];

        for id in ids {
            assert!(registry.subagents.contains_key(id), "missing {id}");
            let mapping = workflow_id_to_mapping(id);
            assert_eq!(mapping.kind, "system");
            assert!(mapping.badge.is_some());
        }

        let errors = registry
            .errors
            .iter()
            .filter(|error| ids.iter().any(|id| error.file_path.contains(id)))
            .map(|error| format!("{}: {}", error.file_path, error.error))
            .collect::<Vec<_>>();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
