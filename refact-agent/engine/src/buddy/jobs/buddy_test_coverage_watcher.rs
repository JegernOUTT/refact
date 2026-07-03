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

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
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
        !cache
            .autonomous
            .is_some_and(|_| cache.signal_hash == spec.signal_hash)
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
        if let Some(last) = cached_scan(&ctx).and_then(|cache| {
            if cache.signal_hash != spec.signal_hash {
                return None;
            }
            cache.autonomous.map(|autonomous| AutonomousLastResult {
                signal_hash: spec.signal_hash.clone(),
                chat_id: autonomous.chat_id,
                completed_at: autonomous.completed_at,
                status: None,
                failure_category: None,
                consecutive_failures: None,
            })
        }) {
            return BuddyJobResult {
                last_result: Some(serialize_scan_with_autonomous(
                    &spec.signal_hash,
                    &scan,
                    &last,
                )),
                ..Default::default()
            };
        }
        let mut result =
            execute_autonomous_spec(gcx, &ctx, spec.clone(), self.cooldown_seconds()).await;
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
            recent_activities: vec![],
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

    #[tokio::test]
    async fn buddy_test_coverage_watcher_keeps_success_cache_shape() {
        let (dir, _repo) = init_temp_git_repo();
        let scan = CoverageScanResult {
            candidates: vec![CoverageCandidate {
                path: "src/feature.rs".to_string(),
                status: "missing_cfg_test_or_tests_dir".to_string(),
            }],
        };
        let spec = build_test_coverage_spec(&test_context(dir.path(), None), &scan);
        let last = AutonomousLastResult {
            signal_hash: spec.signal_hash.clone(),
            chat_id: "chat-a".to_string(),
            completed_at: Utc::now().to_rfc3339(),
            status: None,
            failure_category: None,
            consecutive_failures: None,
        };
        let ctx = test_context(
            dir.path(),
            Some(serialize_scan_with_autonomous(
                &spec.signal_hash,
                &scan,
                &last,
            )),
        );
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        assert!(!BuddyTestCoverageWatcherJob.should_run(gcx, &ctx).await);
        let cache = cached_scan(&ctx).expect("coverage cache parses after success");
        assert_eq!(cache.signal_hash, spec.signal_hash);
        assert_eq!(
            cache.autonomous.as_ref().map(|a| a.chat_id.as_str()),
            Some("chat-a")
        );
    }

    #[tokio::test]
    async fn buddy_test_coverage_watcher_runs_when_fresh_cache_hash_changed_without_success() {
        let (dir, _repo) = init_temp_git_repo();
        let scan = CoverageScanResult {
            candidates: vec![CoverageCandidate {
                path: "src/feature.rs".to_string(),
                status: "missing_cfg_test_or_tests_dir".to_string(),
            }],
        };
        let ctx = test_context(dir.path(), Some(serialize_scan("old-hash", &scan)));
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        assert!(BuddyTestCoverageWatcherJob.should_run(gcx, &ctx).await);
    }

    #[tokio::test]
    async fn buddy_test_coverage_watcher_refreshes_stale_success_cache() {
        let (dir, _repo) = init_temp_git_repo();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("feature.rs"), "pub fn feature() {}\n").unwrap();
        let scan = scan_test_coverage(dir.path());
        let spec = build_test_coverage_spec(&test_context(dir.path(), None), &scan);
        let old_scanned_at = Utc::now().timestamp() - (COOLDOWN_SECONDS as i64 * 2);
        let last = AutonomousLastResult {
            signal_hash: spec.signal_hash.clone(),
            chat_id: "chat-a".to_string(),
            completed_at: Utc::now().to_rfc3339(),
            status: None,
            failure_category: None,
            consecutive_failures: None,
        };
        let stale_cache = CoverageScanCache {
            scanned_at: old_scanned_at,
            signal_hash: spec.signal_hash.clone(),
            scan: scan.clone(),
            autonomous: Some(CoverageAutonomousResult {
                chat_id: last.chat_id.clone(),
                completed_at: last.completed_at.clone(),
            }),
        };
        let ctx = test_context(
            dir.path(),
            Some(serde_json::to_string(&stale_cache).unwrap()),
        );
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;

        let result = BuddyTestCoverageWatcherJob.execute(gcx, ctx).await;
        let refreshed = serde_json::from_str::<CoverageScanCache>(
            result.last_result.as_deref().expect("refreshed cache"),
        )
        .unwrap();

        assert!(refreshed.scanned_at > old_scanned_at);
        assert_eq!(refreshed.signal_hash, spec.signal_hash);
        assert_eq!(
            refreshed.autonomous.as_ref().map(|a| a.chat_id.as_str()),
            Some("chat-a")
        );
    }

    #[test]
    fn buddy_test_coverage_watcher_skips_test_bench_and_example_files() {
        let (dir, _repo) = init_temp_git_repo();
        std::fs::create_dir_all(dir.path().join("tests")).unwrap();
        std::fs::create_dir_all(dir.path().join("src/tests")).unwrap();
        std::fs::create_dir_all(dir.path().join("benches")).unwrap();
        std::fs::create_dir_all(dir.path().join("examples")).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("tests/integration.rs"),
            "#[test] fn it_works() {}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("src/tests/helper.rs"),
            "pub fn helper() {}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("benches/bench.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("examples/demo.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("src/feature.rs"), "pub fn feature() {}\n").unwrap();

        let paths = modified_rust_files(dir.path());

        assert_eq!(paths, vec!["src/feature.rs".to_string()]);
    }

    #[test]
    fn source_has_test_coverage_recognizes_supported_rust_test_layouts() {
        assert!(source_has_test_coverage(
            "src/feature.rs",
            "pub fn feature() {}\n#[cfg(test)] mod tests {}\n",
            &[]
        ));
        assert!(source_has_test_coverage(
            "src/feature.rs",
            "pub fn feature() {}\n",
            &["tests/feature.rs".to_string()]
        ));
        assert!(source_has_test_coverage(
            "crates/demo/src/feature.rs",
            "pub fn feature() {}\n",
            &["crates/demo/tests/feature.rs".to_string()]
        ));
        assert!(source_has_test_coverage(
            "src/feature.rs",
            "pub fn feature() {}\n",
            &["src/tests.rs".to_string()]
        ));
        assert!(source_has_test_coverage(
            "src/feature.rs",
            "pub fn feature() {}\n",
            &["src/tests/helper.rs".to_string()]
        ));
        assert!(!source_has_test_coverage(
            "src/feature.rs",
            "pub fn feature() {}\n",
            &["tests/other.rs".to_string()]
        ));
    }

    #[test]
    fn buddy_test_coverage_watcher_skips_deleted_rust_files() {
        let (dir, repo) = init_temp_git_repo();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("old.rs"), "pub fn old() {}\n").unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("src/old.rs")).unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let sig = git2::Signature::now("test", "test@test.com").unwrap();
            let parent = repo.head().unwrap().peel_to_commit().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "add old", &tree, &[&parent])
                .unwrap();
        }
        std::fs::remove_file(src.join("old.rs")).unwrap();

        let scan = scan_test_coverage(dir.path());

        assert!(scan.candidates.is_empty());
    }

    #[test]
    fn buddy_test_coverage_watcher_parses_renamed_and_copied_modified_paths() {
        assert_eq!(
            parse_git_status_line("RM src/old.rs -> src/new.rs").as_deref(),
            Some("src/new.rs")
        );
        assert_eq!(
            parse_git_status_line("CM src/original.rs -> src/copied.rs").as_deref(),
            Some("src/copied.rs")
        );
    }

    #[test]
    fn buddy_test_coverage_watcher_caps_after_filtering_covered_files() {
        let (dir, _repo) = init_temp_git_repo();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        for idx in 0..MAX_CANDIDATES {
            std::fs::write(
                src.join(format!("covered_{idx}.rs")),
                "#[cfg(test)] mod tests { #[test] fn covered() {} }\n",
            )
            .unwrap();
        }
        std::fs::write(src.join("uncovered.rs"), "pub fn uncovered() {}\n").unwrap();

        let scan = scan_test_coverage(dir.path());

        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(scan.candidates[0].path, "src/uncovered.rs");
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
