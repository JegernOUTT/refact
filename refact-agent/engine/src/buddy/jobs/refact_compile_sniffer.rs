use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use glob::glob;

use crate::buddy::jobs::autonomous_chats::redact_and_cap_text;

use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, REFACT_COMPILE_SNIFFER_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::app_state::AppState;

pub struct RefactCompileSnifferJob;

const COOLDOWN_SECONDS: u64 = 60 * 60;
const PRIORITY: u32 = 5;
const MAX_LOG_LINES: usize = 5;
const MAX_LOG_BYTES: u64 = 16 * 1024;
const MAX_LOG_LINE_CHARS: usize = 1_000;

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
    let file = File::open(path).ok()?.take(MAX_LOG_BYTES);
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for line in reader.lines().take(MAX_LOG_LINES) {
        match line {
            Ok(line) => lines.push(line),
            Err(err) => {
                tracing::warn!("buddy compile sniffer failed to read log line: {err}");
                return None;
            }
        }
    }
    Some(lines)
}

fn safe_log_identifier(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("[redacted-log]")
        .to_string()
}

fn scrub_absolute_path_tokens(text: &str) -> String {
    let mut result = String::new();
    let mut in_path = false;
    for ch in text.chars() {
        if in_path {
            if ch.is_whitespace() {
                result.push(ch);
                in_path = false;
            }
            continue;
        }
        if ch == '/' {
            result.push_str("[REDACTED_PATH]");
            in_path = true;
        } else {
            result.push(ch);
        }
    }
    result
}

fn safe_log_line(line: &str) -> String {
    redact_and_cap_text(&scrub_absolute_path_tokens(line), MAX_LOG_LINE_CHARS)
}

fn safe_log_lines(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| safe_log_line(line)).collect()
}

fn compile_error_evidence(logs_dir: &Path) -> Option<String> {
    let path = newest_rustbinary_log(logs_dir)?;
    let first_lines = first_log_lines(&path)?;
    if !first_lines.iter().any(|line| line.contains("error[E")) {
        return None;
    }
    let safe_lines = safe_log_lines(&first_lines);
    Some(format!(
        "newest_log={}\nmodified_unix={}\nfirst_lines:\n{}",
        safe_log_identifier(&path),
        modified_unix_secs(&path),
        safe_lines.join("\n")
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
            .unwrap_or_else(|err| {
                tracing::warn!("buddy compile sniffer should_run log scan task failed: {err}");
                false
            })
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let logs_dir = gcx.paths.cache_dir.join("logs");
        let evidence = tokio::task::spawn_blocking(move || compile_error_evidence(&logs_dir))
            .await
            .unwrap_or_else(|err| {
                tracing::warn!("buddy compile sniffer log scan task failed: {err}");
                None
            });
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
            recent_activities: vec![],
        }
    }

    async fn gcx_with_cache(cache_dir: &Path) -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache_dir.to_path_buf(),
            std::env::temp_dir().join(format!("refact-cfg-{}", uuid::Uuid::new_v4())),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        app
    }

    fn assert_no_compile_sniffer_leaks(text: &str) {
        for raw in [
            "/home/tester/.cache/refact/logs/rustbinary.2026-05-15",
            "/home/tester/project/src/lib.rs",
            "password=plainsecret",
            "plainsecret",
            "Bearer rawbearertoken123",
            "rawbearertoken123",
            "sk-compileapikey1234567890",
        ] {
            assert!(
                !text.contains(raw),
                "raw sensitive value leaked: {raw}\n{text}"
            );
        }
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

    #[test]
    fn compile_error_evidence_redacts_paths_and_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let logs_dir = dir.path().join("logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        let path = logs_dir.join("rustbinary.2026-05-15");
        std::fs::write(
            &path,
            "error[E0425]: failed at /home/tester/project/src/lib.rs password=plainsecret\nBearer rawbearertoken123\napi_key=sk-compileapikey1234567890",
        )
        .unwrap();

        let evidence = compile_error_evidence(&logs_dir).unwrap();

        assert!(evidence.contains("newest_log=rustbinary.2026-05-15"));
        assert!(evidence.contains("[REDACTED"));
        assert_no_compile_sniffer_leaks(&evidence);
    }

    #[test]
    fn compile_sniffer_spec_prompt_contains_only_redacted_evidence() {
        let ctx = test_context(Path::new("/workspace/refact"));
        let evidence = "newest_log=rustbinary.2026-05-15\nfirst_lines:\nerror[E0425]: [REDACTED_PATH] password=[REDACTED] Bearer [REDACTED] api_key=[REDACTED]".to_string();

        let spec = build_compile_sniffer_spec(&ctx, evidence);
        let combined = format!("{}\n{}", spec.prompt, spec.evidence);

        assert!(combined.contains("rustbinary.2026-05-15"));
        assert!(combined.contains("[REDACTED"));
        assert_no_compile_sniffer_leaks(&combined);
    }

    #[test]
    fn first_log_lines_caps_total_bytes_from_oversized_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustbinary.2026-05-15");
        std::fs::write(
            &path,
            format!("error[E0425]: {}", "x".repeat(MAX_LOG_BYTES as usize * 2)),
        )
        .unwrap();

        let lines = first_log_lines(&path).unwrap();

        assert_eq!(lines.len(), 1);
        assert!(lines[0].len() <= MAX_LOG_BYTES as usize);
    }

    #[test]
    fn first_log_lines_reports_invalid_utf8_as_scan_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustbinary.2026-05-15");
        std::fs::write(&path, [0xff, 0xfe, b'\n']).unwrap();

        assert!(first_log_lines(&path).is_none());
    }
}
