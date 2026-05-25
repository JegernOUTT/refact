use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::buddy::autonomous_workflows::{
    autonomous_workflow_meta, BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID,
};
use crate::buddy::jobs::autonomous_chats::{
    execute_autonomous_spec, same_signal, AutonomousBuddyChatSpec,
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
}

fn serialize_scan(signal_hash: &str, scan: &CoverageScanResult) -> String {
    serde_json::to_string(&CoverageScanCache {
        scanned_at: Utc::now().timestamp(),
        signal_hash: signal_hash.to_string(),
        scan: scan.clone(),
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
    let path = if line.starts_with("R ") || line.starts_with("C ") {
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
        .filter(|path| path.ends_with(".rs"))
        .filter(|path| !path.contains("/tests/"))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths.truncate(MAX_CANDIDATES);
    paths
}

fn has_tests_dir(path: &Path) -> bool {
    path.parent()
        .map(|parent| parent.join("tests").is_dir())
        .unwrap_or(false)
}

fn has_cfg_test(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|content| content.contains("#[cfg(test)]"))
        .unwrap_or(false)
}

fn missing_test_candidate(project_root: &Path, rel: &str) -> Option<CoverageCandidate> {
    let path = project_root.join(rel);
    if has_cfg_test(&path) || has_tests_dir(&path) {
        return None;
    }
    Some(CoverageCandidate {
        path: rel.to_string(),
        status: "missing_cfg_test_or_tests_dir".to_string(),
    })
}

fn scan_test_coverage(project_root: &Path) -> CoverageScanResult {
    let candidates = modified_rust_files(project_root)
        .into_iter()
        .filter_map(|rel| missing_test_candidate(project_root, &rel))
        .collect();
    CoverageScanResult { candidates }
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

async fn current_scan(ctx: &BuddyJobContext) -> CoverageScanResult {
    if let Some((scan, _)) = scan_cache_result(ctx) {
        return scan;
    }
    let project_root = ctx.project_root.clone();
    tokio::task::spawn_blocking(move || scan_test_coverage(&project_root))
        .await
        .unwrap_or(CoverageScanResult { candidates: vec![] })
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

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        let Some(cache) = cached_scan(ctx) else {
            return true;
        };
        if !cache_is_fresh(cache.scanned_at) {
            return true;
        }
        let (scan, cached_hash) = (cache.scan, cache.signal_hash);
        if scan.candidates.is_empty() {
            return false;
        }
        let spec = build_test_coverage_spec(ctx, &scan);
        cached_hash == spec.signal_hash && !same_signal(ctx, &spec.signal_hash)
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let scan = current_scan(&ctx).await;
        if scan.candidates.is_empty() {
            return BuddyJobResult {
                last_result: Some(serialize_scan("", &scan)),
                ..Default::default()
            };
        }
        let spec = build_test_coverage_spec(&ctx, &scan);
        if same_signal(&ctx, &spec.signal_hash) {
            return BuddyJobResult::default();
        }
        let mut result = execute_autonomous_spec(gcx, &ctx, spec.clone()).await;
        if result.last_result.is_none() {
            result.last_result = Some(serialize_scan(&spec.signal_hash, &scan));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::settings::BuddySettings;
    use crate::buddy::types::{BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse};
    use std::path::Path;

    fn test_context(project_root: &Path, last_result: Option<String>) -> BuddyJobContext {
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
        }
    }

    fn init_temp_git_repo() -> (tempfile::TempDir, git2::Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        {
            let mut index = repo.index().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }
        (dir, repo)
    }

    #[tokio::test]
    async fn buddy_test_coverage_watcher_detects_missing_tests() {
        let (dir, _repo) = init_temp_git_repo();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("feature.rs"), "pub fn feature() {}\n").unwrap();
        let scan = scan_test_coverage(dir.path());
        let spec = build_test_coverage_spec(&test_context(dir.path(), None), &scan);
        let ctx = test_context(dir.path(), Some(serialize_scan(&spec.signal_hash, &scan)));
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        assert!(BuddyTestCoverageWatcherJob.should_run(gcx, &ctx).await);
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(scan.candidates[0].path, "src/feature.rs");
    }

    #[test]
    fn buddy_test_coverage_watcher_caps_to_5_candidates() {
        let (dir, _repo) = init_temp_git_repo();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        for idx in 0..7 {
            std::fs::write(src.join(format!("file_{idx}.rs")), "pub fn item() {}\n").unwrap();
        }

        let scan = scan_test_coverage(dir.path());

        assert_eq!(scan.candidates.len(), MAX_CANDIDATES);
        assert_eq!(scan.candidates[0].path, "src/file_0.rs");
        assert_eq!(scan.candidates[4].path, "src/file_4.rs");
    }
}
