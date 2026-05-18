use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use glob::glob;

use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, REFACT_COMPILE_SNIFFER_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::app_state::AppState;

pub struct RefactCompileSnifferJob;

const COOLDOWN_SECONDS: u64 = 60 * 60;
const PRIORITY: u32 = 5;
const MAX_LOG_LINES: usize = 5;

fn modified_unix_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn newest_rustbinary_log(logs_dir: &Path) -> Option<PathBuf> {
    let pattern = logs_dir.join("rustbinary.*").to_string_lossy().to_string();
    glob(&pattern)
        .ok()?
        .filter_map(Result::ok)
        .filter(|path| path.is_file())
        .max_by_key(|path| modified_unix_secs(path))
}

fn first_log_lines(path: &Path) -> Option<Vec<String>> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    Some(
        reader
            .lines()
            .take(MAX_LOG_LINES)
            .filter_map(Result::ok)
            .collect(),
    )
}

fn compile_error_evidence(logs_dir: &Path) -> Option<String> {
    let path = newest_rustbinary_log(logs_dir)?;
    let first_lines = first_log_lines(&path)?;
    if !first_lines.iter().any(|line| line.contains("error[E")) {
        return None;
    }
    Some(format!(
        "newest_log={}\nmodified_unix={}\nfirst_lines:\n{}",
        path.display(),
        modified_unix_secs(&path),
        first_lines.join("\n")
    ))
}

fn build_compile_sniffer_spec(ctx: &BuddyJobContext, evidence: String) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(REFACT_COMPILE_SNIFFER_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    AutonomousBuddyChatSpec::new(
        meta.id,
        meta.title,
        "Triage the newest Refact rustbinary compile/test error log and inspect engine source only when needed.",
        format!("project_root={}\n{}", project_root, evidence),
    )
    .with_display(meta.icon, meta.badge, meta.priority)
    .with_project_root(project_root)
}

#[async_trait::async_trait]
impl BuddyJob for RefactCompileSnifferJob {
    fn id(&self) -> &str {
        REFACT_COMPILE_SNIFFER_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        COOLDOWN_SECONDS
    }

    fn priority(&self) -> u32 {
        PRIORITY
    }

    async fn should_run(&self, gcx: AppState, _ctx: &BuddyJobContext) -> bool {
        let logs_dir = gcx.paths.cache_dir.join("logs");
        tokio::task::spawn_blocking(move || compile_error_evidence(&logs_dir).is_some())
            .await
            .unwrap_or(false)
    }

    async fn execute(
        &self,
        gcx: AppState,
        ctx: BuddyJobContext,
    ) -> BuddyJobResult {
        let logs_dir = gcx.paths.cache_dir.join("logs");
        let evidence = tokio::task::spawn_blocking(move || compile_error_evidence(&logs_dir))
            .await
            .unwrap_or(None);
        let Some(evidence) = evidence else {
            return BuddyJobResult::default();
        };
        execute_autonomous_spec(gcx, &ctx, build_compile_sniffer_spec(&ctx, evidence)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        }
    }

    async fn gcx_with_cache(cache_dir: &Path) -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.write().await.cache_dir = cache_dir.to_path_buf();
        let app = AppState::from_gcx(gcx).await;
        app
    }

    #[tokio::test]
    async fn refact_compile_sniffer_should_run_when_recent_compile_errors_exist() {
        let dir = tempfile::tempdir().unwrap();
        let logs_dir = dir.path().join("logs");
        tokio::fs::create_dir_all(&logs_dir).await.unwrap();
        tokio::fs::write(
            logs_dir.join("rustbinary.2026-05-15"),
            "error[E0425]: cannot find value\nsecond\nthird\nfourth\nfifth\nsixth",
        )
        .await
        .unwrap();
        let gcx = gcx_with_cache(dir.path()).await;
        let ctx = test_context(dir.path());

        assert!(RefactCompileSnifferJob.should_run(gcx, &ctx).await);
    }

    #[tokio::test]
    async fn refact_compile_sniffer_should_not_run_when_no_errors() {
        let dir = tempfile::tempdir().unwrap();
        let logs_dir = dir.path().join("logs");
        tokio::fs::create_dir_all(&logs_dir).await.unwrap();
        tokio::fs::write(
            logs_dir.join("rustbinary.2026-05-15"),
            "starting\nwarning: unused variable\nfinished",
        )
        .await
        .unwrap();
        let gcx = gcx_with_cache(dir.path()).await;
        let ctx = test_context(dir.path());

        assert!(!RefactCompileSnifferJob.should_run(gcx, &ctx).await);
    }
}
