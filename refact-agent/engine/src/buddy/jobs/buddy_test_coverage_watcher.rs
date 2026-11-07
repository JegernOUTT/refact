use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::buddy::autonomous_workflows::{
    autonomous_workflow_meta, BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID,
};
use crate::buddy::jobs::autonomous_chats::{
    execute_autonomous_spec, parse_last_autonomous_result, AutonomousBuddyChatSpec,
    AutonomousLastResult,
};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::app_state::AppState;

pub struct BuddyTestCoverageWatcherJob;

const COOLDOWN_SECONDS: u64 = 4 * 60 * 60;
const PRIORITY: u32 = 6;
const MAX_CANDIDATES: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CoverageCandidate {
    path: String,
    status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CoverageScanResult {
    candidates: Vec<CoverageCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CoverageScanCache {
    scanned_at: i64,
    signal_hash: String,
    scan: CoverageScanResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    autonomous: Option<CoverageAutonomousResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CoverageAutonomousResult {
    chat_id: String,
    completed_at: String,
    #[serde(default)]
    signal_hash: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    failure_category: Option<crate::buddy::workflows::WorkflowFailureCategory>,
    #[serde(default)]
    consecutive_failures: Option<u32>,
    #[serde(default)]
    preflight_fingerprint: Option<String>,
}

impl CoverageScanCache {
    fn autonomous_result(&self) -> Option<AutonomousLastResult> {
        let last = self.autonomous.as_ref()?;
        Some(AutonomousLastResult {
            signal_hash: last
                .signal_hash
                .clone()
                .unwrap_or_else(|| self.signal_hash.clone()),
            chat_id: last.chat_id.clone(),
            completed_at: last.completed_at.clone(),
            status: last.status.clone(),
            failure_category: last.failure_category.clone(),
            consecutive_failures: last.consecutive_failures,
            preflight_fingerprint: last.preflight_fingerprint.clone(),
        })
    }
}

fn autonomous_context(mut ctx: BuddyJobContext) -> BuddyJobContext {
    ctx.job_state.last_result = cached_scan(&ctx)
        .and_then(|cache| cache.autonomous_result())
        .map(|last| super::autonomous_chats::serialize_last_autonomous_result(&last));
    ctx
}

fn serialize_scan(signal_hash: &str, scan: &CoverageScanResult) -> String {
    serde_json::to_string(&CoverageScanCache {
        scanned_at: Utc::now().timestamp(),
        signal_hash: signal_hash.to_string(),
        scan: scan.clone(),
        autonomous: None,
    })
    .unwrap_or_default()
}

fn serialize_scan_with_autonomous(
    signal_hash: &str,
    scan: &CoverageScanResult,
    last: &AutonomousLastResult,
) -> String {
    serde_json::to_string(&CoverageScanCache {
        scanned_at: Utc::now().timestamp(),
        signal_hash: signal_hash.to_string(),
        scan: scan.clone(),
        autonomous: Some(CoverageAutonomousResult {
            chat_id: last.chat_id.clone(),
            completed_at: last.completed_at.clone(),
            signal_hash: Some(last.signal_hash.clone()),
            status: last.status.clone(),
            failure_category: last.failure_category.clone(),
            consecutive_failures: last.consecutive_failures,
            preflight_fingerprint: last.preflight_fingerprint.clone(),
        }),
    })
    .unwrap_or_default()
}

fn cached_scan(ctx: &BuddyJobContext) -> Option<CoverageScanCache> {
    serde_json::from_str::<CoverageScanCache>(ctx.job_state.last_result.as_deref()?).ok()
}

fn cache_is_fresh(scanned_at: i64) -> bool {
    Utc::now().timestamp().saturating_sub(scanned_at) < COOLDOWN_SECONDS as i64
}

fn scan_cache_result(ctx: &BuddyJobContext) -> Option<(CoverageScanResult, String)> {
    cached_scan(ctx)
        .filter(|cache| cache_is_fresh(cache.scanned_at))
        .map(|cache| (cache.scan, cache.signal_hash))
}

fn parse_git_status_line(line: &str) -> Option<String> {
    if line.len() < 4 {
        return None;
    }
    let status = line.get(..2)?;
    if status.as_bytes().iter().any(|status| *status == b'D') {
        return None;
    }
    let path = if matches!(status.as_bytes().first(), Some(b'R' | b'C')) {
        line.get(3..)?
            .rsplit_once(" -> ")
            .map(|(_, to)| to)
            .unwrap_or(line.get(3..)?)
    } else {
        line.get(3..)?
    };
    let path = path.trim().trim_matches('"');
    (!path.is_empty()).then(|| path.to_string())
}

fn modified_rust_files(project_root: &Path) -> Vec<String> {
    let output = crate::worktrees::git::run_git_lossy(
        project_root,
        &["status", "--porcelain", "--untracked-files=all"],
    );
    let mut paths = output
        .lines()
        .filter_map(parse_git_status_line)
        .filter(|path| coverage_source_candidate(path))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn coverage_source_candidate(path: &str) -> bool {
    if !path.ends_with(".rs") {
        return false;
    }
    !path
        .replace('\\', "/")
        .split('/')
        .any(|part| matches!(part, "tests" | "benches" | "examples"))
}

fn normalize_rel_path(path: &str) -> String {
    path.replace('\\', "/").trim_matches('/').to_string()
}

fn crate_src_parts(rel: &str) -> Option<(String, String)> {
    let rel = normalize_rel_path(rel);
    if rel == "src" || rel.starts_with("src/") {
        return Some((String::new(), "src".to_string()));
    }
    let (crate_root, _) = rel.split_once("/src/")?;
    Some((crate_root.to_string(), format!("{crate_root}/src")))
}

fn join_rel(prefix: &str, suffix: &str) -> String {
    if prefix.is_empty() {
        suffix.to_string()
    } else {
        format!("{prefix}/{suffix}")
    }
}

fn source_has_test_coverage(rel: &str, source_content: &str, existing_paths: &[String]) -> bool {
    if source_content.contains("#[cfg(test)]") {
        return true;
    }
    let Some((crate_root, src_dir)) = crate_src_parts(rel) else {
        return false;
    };
    let rel = normalize_rel_path(rel);
    let file_name = Path::new(&rel)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if file_name.is_empty() {
        return false;
    }
    let crate_tests_file = join_rel(&join_rel(&crate_root, "tests"), file_name);
    let src_tests_rs = join_rel(&src_dir, "tests.rs");
    let src_tests_dir = format!("{}/", join_rel(&src_dir, "tests"));
    let parent_tests_dir = Path::new(&rel).parent().map(|parent| {
        format!(
            "{}/",
            join_rel(
                &normalize_rel_path(parent.to_string_lossy().as_ref()),
                "tests"
            )
        )
    });

    existing_paths
        .iter()
        .map(|path| normalize_rel_path(path))
        .any(|path| {
            path == crate_tests_file
                || path == src_tests_rs
                || format!("{path}/").starts_with(&src_tests_dir)
                || parent_tests_dir
                    .as_ref()
                    .is_some_and(|tests_dir| format!("{path}/").starts_with(tests_dir))
        })
}

fn coverage_indicator_paths(project_root: &Path, rel: &str) -> Vec<String> {
    let Some((crate_root, src_dir)) = crate_src_parts(rel) else {
        return vec![];
    };
    let rel = normalize_rel_path(rel);
    let file_name = Path::new(&rel)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let mut paths = Vec::new();
    let crate_tests_file = join_rel(&join_rel(&crate_root, "tests"), file_name);
    if project_root.join(&crate_tests_file).is_file() {
        paths.push(crate_tests_file);
    }
    let src_tests_rs = join_rel(&src_dir, "tests.rs");
    if project_root.join(&src_tests_rs).is_file() {
        paths.push(src_tests_rs);
    }
    let src_tests_dir = join_rel(&src_dir, "tests");
    if project_root.join(&src_tests_dir).is_dir() {
        paths.push(format!("{src_tests_dir}/"));
    }
    if let Some(parent) = Path::new(&rel).parent() {
        let parent_tests_dir = join_rel(
            &normalize_rel_path(parent.to_string_lossy().as_ref()),
            "tests",
        );
        if project_root.join(&parent_tests_dir).is_dir() {
            paths.push(format!("{parent_tests_dir}/"));
        }
    }
    paths
}

fn missing_test_candidate(project_root: &Path, rel: &str) -> Option<CoverageCandidate> {
    let path = project_root.join(rel);
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    let source_content = std::fs::read_to_string(&path).unwrap_or_default();
    if source_has_test_coverage(
        rel,
        &source_content,
        &coverage_indicator_paths(project_root, rel),
    ) {
        return None;
    }
    Some(CoverageCandidate {
        path: rel.to_string(),
        status: "missing_cfg_test_or_tests_dir".to_string(),
    })
}

fn scan_test_coverage(project_root: &Path) -> CoverageScanResult {
    let mut candidates = modified_rust_files(project_root)
        .into_iter()
        .filter_map(|rel| missing_test_candidate(project_root, &rel))
        .collect::<Vec<_>>();
    candidates.truncate(MAX_CANDIDATES);
    CoverageScanResult {
        candidates,
    }
}

fn render_evidence(scan: &CoverageScanResult) -> String {
    let mut lines = vec![
        "Test coverage signal:".to_string(),
        format!("- missing_test_candidates: {}", scan.candidates.len()),
    ];
    for candidate in &scan.candidates {
        lines.push(format!("- {} ({})", candidate.path, candidate.status));
    }
    lines.join("\n")
}

fn build_test_coverage_spec(
    ctx: &BuddyJobContext,
    scan: &CoverageScanResult,
) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    AutonomousBuddyChatSpec::new(
        meta.id,
        meta.title,
        "Inspect changed Rust files that appear to lack nearby tests and propose focused coverage follow-up.",
        format!("project_root={}\n{}", project_root, render_evidence(scan)),
    )
    .with_display(meta.icon, meta.badge, meta.priority)
    .with_project_root(project_root)
}

async fn current_scan(ctx: &BuddyJobContext) -> Result<CoverageScanResult, String> {
    if let Some((scan, _)) = scan_cache_result(ctx) {
        return Ok(scan);
    }
    let project_root = ctx.project_root.clone();
    tokio::task::spawn_blocking(move || scan_test_coverage(&project_root))
        .await
        .map_err(|err| format!("buddy test coverage scan task failed: {err}"))
}

#[async_trait::async_trait]
impl BuddyJob for BuddyTestCoverageWatcherJob {
    fn id(&self) -> &str {
        BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID
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
        let Some(cache) = cached_scan(ctx) else {
            return true;
        };
        if !cache_is_fresh(cache.scanned_at) {
            return true;
        }
        let scan = cache.scan;
        if scan.candidates.is_empty() {
            return false;
        }
        let spec = build_test_coverage_spec(ctx, &scan);
        super::autonomous_chats::autonomous_should_run(
            &gcx,
            &autonomous_context(ctx.clone()),
            self.id(),
            &spec.signal_hash,
            self.cooldown_seconds(),
        )
        .await
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let scan = match current_scan(&ctx).await {
            Ok(scan) => scan,
            Err(err) => {
                tracing::warn!("{err}");
                return BuddyJobResult::default();
            }
        };
        if scan.candidates.is_empty() {
            return BuddyJobResult {
                last_result: Some(serialize_scan("", &scan)),
                ..Default::default()
            };
        }
        let spec = build_test_coverage_spec(&ctx, &scan);
        let autonomous_ctx = autonomous_context(ctx);
        let mut result =
            execute_autonomous_spec(gcx, &autonomous_ctx, spec.clone(), self.cooldown_seconds())
                .await;
        if let Some(last) = parse_last_autonomous_result(result.last_result.as_deref()) {
            result.last_result = Some(serialize_scan_with_autonomous(
                &spec.signal_hash,
                &scan,
                &last,
            ));
        } else if result.last_result.is_none() {
            result.last_result = Some(serialize_scan(&spec.signal_hash, &scan));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn context() -> BuddyJobContext {
        BuddyJobContext {
            identity_name: "Pixel".into(),
            personality: Default::default(),
            onboarding: Default::default(),
            recent_diagnostics: vec![],
            project_root: std::path::PathBuf::from("/tmp/coverage-test"),
            job_state: Default::default(),
            workflow_summaries: vec![],
            total_workflow_runs: 0,
            suggestion_state: vec![],
            pet: Default::default(),
            active_quest: None,
            settings: Default::default(),
            pulse: Default::default(),
            facts: vec![],
            recent_activities: vec![],
        }
    }

    fn scan() -> CoverageScanResult {
        CoverageScanResult {
            candidates: vec![CoverageCandidate {
                path: "src/lib.rs".into(),
                status: "missing_cfg_test_or_tests_dir".into(),
            }],
        }
    }

    #[tokio::test]
    async fn serialized_coverage_cache_preserves_failures_backoff_and_deferred_no_replay() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let job = BuddyTestCoverageWatcherJob;
        for (status, old, expected_run) in [
            ("failed", false, false),
            ("failed", true, true),
            ("blocked", false, true),
            ("superseded", false, true),
            ("deferred", false, false),
        ] {
            let mut ctx = context();
            let scan = scan();
            let spec = build_test_coverage_spec(&ctx, &scan);
            let mut last = AutonomousLastResult::new(&spec.signal_hash, "retained-chat");
            last.status = Some(status.into());
            last.failure_category =
                Some(crate::buddy::workflows::WorkflowFailureCategory::ProviderTransient);
            last.consecutive_failures = Some(3);
            last.preflight_fingerprint = Some("old-model-configuration".into());
            if old {
                last.completed_at = (Utc::now() - chrono::Duration::days(2)).to_rfc3339();
            }
            ctx.job_state.last_result = Some(serialize_scan_with_autonomous(
                &spec.signal_hash,
                &scan,
                &last,
            ));
            assert_eq!(
                cached_scan(&ctx).unwrap().autonomous_result(),
                Some(last.clone())
            );
            assert_eq!(
                job.should_run(app.clone(), &ctx).await,
                expected_run,
                "{status}, old={old}"
            );
            if !expected_run {
                let result = job.execute(app.clone(), ctx).await;
                let cache: CoverageScanCache =
                    serde_json::from_str(result.last_result.as_deref().unwrap()).unwrap();
                assert_eq!(cache.autonomous_result(), Some(last));
                assert!(result.activity.is_none());
                assert!(result.workflow_failure.is_none());
            }
        }
    }

    #[test]
    fn legacy_coverage_cache_remains_historical_success() {
        let cache: CoverageScanCache = serde_json::from_value(serde_json::json!({
            "scanned_at": 0, "signal_hash": "signal", "scan": {"candidates": []},
            "autonomous": {"chat_id": "historical-chat", "completed_at": "2026-01-01T00:00:00Z"}
        }))
        .unwrap();
        assert_eq!(cache.autonomous_result().unwrap().status, None);
    }

    #[tokio::test]
    async fn coverage_blocked_execute_preserves_current_preflight_and_suppresses_unchanged_config()
    {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let job = BuddyTestCoverageWatcherJob;
        let mut ctx = context();
        let scan = scan();
        let spec = build_test_coverage_spec(&ctx, &scan);
        ctx.job_state.last_result = Some(serialize_scan(&spec.signal_hash, &scan));
        let result = job.execute(app.clone(), ctx.clone()).await;
        ctx.job_state.last_result = result.last_result;
        let last = cached_scan(&ctx).unwrap().autonomous_result().unwrap();
        assert_eq!(last.status.as_deref(), Some("blocked"));
        assert!(last.preflight_fingerprint.is_some());
        assert!(!job.should_run(app.clone(), &ctx).await);
        let mut changed = last;
        changed.preflight_fingerprint = Some("previous-config".into());
        ctx.job_state.last_result = Some(serialize_scan_with_autonomous(
            &spec.signal_hash,
            &scan,
            &changed,
        ));
        assert!(job.should_run(app, &ctx).await);
    }
}
