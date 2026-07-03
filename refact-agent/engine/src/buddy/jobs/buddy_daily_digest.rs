use std::path::Path;

use chrono::{DateTime, Local, Timelike, Utc};

use crate::app_state::AppState;
use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, BUDDY_DAILY_DIGEST_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::buddy::settings::BuddySettings;

pub struct BuddyDailyDigestJob;

const COOLDOWN_SECONDS: u64 = 0;
const PRIORITY: u32 = 30;
const TRUSTED_COMMAND_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

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
        .take(10)
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
        recent_activity_evidence(ctx)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use chrono::TimeZone;
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
        let hour = Local::now().hour() as u8;
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
        let late_same_day = Local
            .with_ymd_and_hms(2026, 6, 5, 23, 0, 0)
            .single()
            .unwrap();

        assert!(should_run_at(&ctx, late_same_day));
    }

    #[test]
    fn buddy_daily_digest_dedupes_on_local_date_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = test_context(dir.path());
        ctx.settings.daily_digest_hour = Some(18);
        let now = Local
            .with_ymd_and_hms(2026, 6, 5, 23, 0, 0)
            .single()
            .unwrap();
        ctx.job_state.last_result = Some(date_key(&now));

        assert!(!should_run_at(&ctx, now));
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
