use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app_state::AppState;
use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, REFACT_COMPILE_SNIFFER_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{
    execute_autonomous_spec, parse_last_autonomous_result, signal_hash, AutonomousBuddyChatSpec,
};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::tools::tool_buddy_get_logs::{is_log_candidate, resolve_log_dir};

pub struct RefactCompileSnifferJob;

const COOLDOWN_SECONDS: u64 = 60 * 60;
const PRIORITY: u32 = 5;
const MAX_LOG_LINES: usize = 5;
const MAX_LOG_BYTES: u64 = 256 * 1024;
const MAX_LOG_AGE_SECONDS: u64 = 6 * 60 * 60;

fn modified_unix_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn log_is_fresh(path: &Path) -> bool {
    let modified = match std::fs::metadata(path).and_then(|metadata| metadata.modified()) {
        Ok(modified) => modified,
        Err(_) => return false,
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age.as_secs() <= MAX_LOG_AGE_SECONDS)
        .unwrap_or(true)
}

fn newest_log(logs_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(logs_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(is_log_candidate)
                    .unwrap_or(false)
        })
        .max_by_key(|path| modified_unix_secs(path))
}

fn tail_log_lines(path: &Path) -> Option<Vec<String>> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(MAX_LOG_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_LOG_BYTES).read_to_end(&mut bytes).ok()?;
    let mut text = String::from_utf8(bytes)
        .map_err(|err| {
            tracing::warn!("buddy compile sniffer failed to read log tail as utf8: {err}");
            err
        })
        .ok()?;
    if start > 0 {
        if let Some(pos) = text.find('\n') {
            text = text[pos + 1..].to_string();
        }
    }
    let lines = text
        .lines()
        .rev()
        .take(MAX_LOG_LINES)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Some(lines)
}

fn has_failure_signature(line: &str) -> bool {
    line.contains("error[E")
        || line.contains("could not compile")
        || line.contains("error: could not")
        || line.contains("test result: FAILED")
        || line.contains("panicked at")
        || line.contains("error[")
}

fn evidence_fingerprint(path: &Path, modified_unix: u64, tail_lines: &[String]) -> String {
    signal_hash([
        path.to_string_lossy().as_ref(),
        &modified_unix.to_string(),
        &tail_lines.join("\n"),
    ])
}

fn compile_signal_hash(fingerprint: &str) -> String {
    signal_hash([REFACT_COMPILE_SNIFFER_WORKFLOW_ID, fingerprint])
}

fn same_compile_signal(ctx: &BuddyJobContext, hash: &str) -> bool {
    parse_last_autonomous_result(ctx.job_state.last_result.as_deref())
        .map(|last| last.signal_hash == hash)
        .unwrap_or(false)
}

fn compile_error_evidence(logs_dir: &Path) -> Option<(String, String)> {
    let path = newest_log(logs_dir)?;
    if !log_is_fresh(&path) {
        return None;
    }
    let tail_lines = tail_log_lines(&path)?;
    if !tail_lines.iter().any(|line| has_failure_signature(line)) {
        return None;
    }
    let modified_unix = modified_unix_secs(&path);
    let fingerprint = evidence_fingerprint(&path, modified_unix, &tail_lines);
    Some((
        format!(
            "newest_log={}\nmodified_unix={}\nfingerprint={}\ntail_lines:\n{}",
            path.display(),
            modified_unix,
            fingerprint,
            tail_lines.join("\n")
        ),
        fingerprint,
    ))
}

fn build_compile_sniffer_spec(
    ctx: &BuddyJobContext,
    evidence: String,
    fingerprint: String,
) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(REFACT_COMPILE_SNIFFER_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    AutonomousBuddyChatSpec::new(
        meta.id,
        meta.title,
        "Triage the newest Refact rustbinary compile/test error log and inspect engine source only when needed.",
        format!("project_root={}\n{}", project_root, evidence),
    )
    .with_signal_hash(compile_signal_hash(&fingerprint))
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

    async fn should_run(&self, gcx: AppState, ctx: &BuddyJobContext) -> bool {
        let logs_dir = resolve_log_dir(&gcx.paths.cache_dir);
        let evidence = tokio::task::spawn_blocking(move || compile_error_evidence(&logs_dir))
            .await
            .unwrap_or_else(|err| {
                tracing::warn!("buddy compile sniffer should_run log scan task failed: {err}");
                None
            });
        let Some((_, fingerprint)) = evidence else {
            return false;
        };
        !same_compile_signal(ctx, &compile_signal_hash(&fingerprint))
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let logs_dir = resolve_log_dir(&gcx.paths.cache_dir);
        let evidence = tokio::task::spawn_blocking(move || compile_error_evidence(&logs_dir))
            .await
            .unwrap_or_else(|err| {
                tracing::warn!("buddy compile sniffer log scan task failed: {err}");
                None
            });
        let Some((evidence, fingerprint)) = evidence else {
            return BuddyJobResult::default();
        };
        execute_autonomous_spec(
            gcx,
            &ctx,
            build_compile_sniffer_spec(&ctx, evidence, fingerprint),
            self.cooldown_seconds(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::jobs::autonomous_chats::{serialize_last_autonomous_result, AutonomousLastResult};
    use crate::buddy::settings::BuddySettings;
    use crate::buddy::types::{BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse};

    fn test_context(project_root: &Path) -> BuddyJobContext {
        test_context_with_last_result(project_root, None)
    }

    fn test_context_with_last_result(
        project_root: &Path,
        last_result: Option<String>,
    ) -> BuddyJobContext {
        BuddyJobContext {
            identity_name: "Pixel".to_string(),
            personality: Default::default(),
            onboarding: BuddyOnboarding::default(),
            recent_diagnostics: vec![],
            project_root: project_root.to_path_buf(),
            job_state: BuddyJobState {
                last_result,
                ..Default::default()
            },
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

    async fn gcx_with_cache(cache_dir: &Path) -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache_dir.to_path_buf(),
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
        )
        .await;
        AppState::from_gcx(gcx).await
    }

    fn mark_old(path: &Path) {
        let old = filetime::FileTime::from_unix_time(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                .saturating_sub(MAX_LOG_AGE_SECONDS + 60) as i64,
            0,
        );
        filetime::set_file_mtime(path, old).unwrap();
    }

    #[tokio::test]
    async fn refact_compile_sniffer_should_run_when_recent_compile_errors_exist() {
        let dir = tempfile::tempdir().unwrap();
        let logs_dir = dir.path().join("logs");
        tokio::fs::create_dir_all(&logs_dir).await.unwrap();
        tokio::fs::write(
            logs_dir.join("rustbinary.2026-05-15"),
            "first\nsecond\nthird\nfourth\nfifth\nerror[E0425]: cannot find value",
        )
        .await
        .unwrap();
        let gcx = gcx_with_cache(dir.path()).await;
        let ctx = test_context(dir.path());

        assert!(RefactCompileSnifferJob.should_run(gcx, &ctx).await);
    }

    #[tokio::test]
    async fn refact_compile_sniffer_should_not_run_for_stale_log() {
        let dir = tempfile::tempdir().unwrap();
        let logs_dir = dir.path().join("logs");
        tokio::fs::create_dir_all(&logs_dir).await.unwrap();
        let log_path = logs_dir.join("rustbinary.2026-05-15");
        tokio::fs::write(&log_path, "error[E0425]: cannot find value")
            .await
            .unwrap();
        mark_old(&log_path);
        let gcx = gcx_with_cache(dir.path()).await;
        let ctx = test_context(dir.path());

        assert!(!RefactCompileSnifferJob.should_run(gcx, &ctx).await);
    }

    #[tokio::test]
    async fn refact_compile_sniffer_should_not_rerun_same_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let logs_dir = dir.path().join("logs");
        tokio::fs::create_dir_all(&logs_dir).await.unwrap();
        tokio::fs::write(
            logs_dir.join("rustbinary.2026-05-15"),
            "first\nsecond\nthird\nfourth\nfifth\nerror[E0425]: cannot find value",
        )
        .await
        .unwrap();
        let (evidence, fingerprint) = compile_error_evidence(&logs_dir).unwrap();
        let ctx = test_context(dir.path());
        let spec = build_compile_sniffer_spec(&ctx, evidence, fingerprint);
        let mut last = AutonomousLastResult::new(spec.signal_hash, "chat-1");
        last.status = Some("failed".to_string());
        let last_result = Some(serialize_last_autonomous_result(&last));
        let gcx = gcx_with_cache(dir.path()).await;
        let ctx = test_context_with_last_result(dir.path(), last_result);

        assert!(!RefactCompileSnifferJob.should_run(gcx, &ctx).await);
    }

    #[tokio::test]
    async fn refact_compile_sniffer_should_run_on_daemon_worker_tail_failure() {
        let dir = tempfile::tempdir().unwrap();
        let logs_dir = dir.path().join("daemon").join("logs");
        tokio::fs::create_dir_all(&logs_dir).await.unwrap();
        tokio::fs::write(
            logs_dir.join("worker-1.log"),
            "starting\nchecks\ntest result: FAILED. 0 passed; 1 failed",
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

    #[test]
    fn tail_log_lines_caps_total_bytes_from_oversized_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustbinary.2026-05-15");
        std::fs::write(
            &path,
            format!("error[E0425]: {}", "x".repeat(MAX_LOG_BYTES as usize * 2)),
        )
        .unwrap();

        let lines = tail_log_lines(&path).unwrap();

        assert_eq!(lines.len(), 1);
        assert!(lines[0].len() <= MAX_LOG_BYTES as usize);
    }

    #[test]
    fn tail_log_lines_reports_invalid_utf8_as_scan_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustbinary.2026-05-15");
        std::fs::write(&path, [0xff, 0xfe, b'\n']).unwrap();

        assert!(tail_log_lines(&path).is_none());
    }
}
