use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use refact_lsp::tools::review_types::{
    ReviewDiffSummary, ReviewFinding, ReviewOutcome, ReviewReport, ReviewScopeSummary,
    ReviewSeverity,
};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

const FIXTURE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/review_fixtures");

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FixtureKind {
    Seeded,
    Clean,
    DuplicateBait,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LineRange {
    start: u32,
    end: u32,
}

/// Manifests spell the top severity `critical`, which only `ReviewSeverity::parse` still accepts.
fn deserialize_severity<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<ReviewSeverity, D::Error> {
    let raw = String::deserialize(deserializer)?;
    ReviewSeverity::parse(&raw)
        .ok_or_else(|| serde::de::Error::custom(format!("unknown severity {raw:?}")))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct SeededDefect {
    file: String,
    line_range: LineRange,
    category: String,
    #[serde(deserialize_with = "deserialize_severity")]
    severity: ReviewSeverity,
    description: String,
    marker: String,
    #[serde(default)]
    expected_check: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct FixtureManifest {
    id: String,
    kind: FixtureKind,
    scenario: String,
    description: String,
    seeded_defects: Vec<SeededDefect>,
}

#[derive(Debug, Clone, PartialEq)]
struct FixtureMetrics {
    seeded_recall: f64,
    seeded_high_severity_recall: f64,
    precision_proxy: f64,
    duplicate_rate: f64,
    clean_false_positive_count: usize,
    high_severity_hypothesis_count: usize,
}

fn fixture_directories() -> Vec<PathBuf> {
    let mut directories = fs::read_dir(FIXTURE_ROOT)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    directories.sort();
    directories
}

fn load_manifest(directory: &Path) -> FixtureManifest {
    let manifest_path = directory.join("manifest.json");
    serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap())
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", manifest_path.display()))
}

fn normalized_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn file_matches(finding_file: &str, seeded_file: &str) -> bool {
    let finding_file = normalized_path(finding_file);
    let seeded_file = normalized_path(seeded_file);
    finding_file == seeded_file || finding_file.ends_with(&format!("/{seeded_file}"))
}

fn ranges_overlap(left_start: u32, left_end: u32, right_start: u32, right_end: u32) -> bool {
    left_start <= right_end && right_start <= left_end
}

fn finding_matches_seed(finding: &ReviewFinding, seed: &SeededDefect) -> bool {
    file_matches(&finding.file, &seed.file)
        && ranges_overlap(
            finding.line_start,
            finding.line_end,
            seed.line_range.start,
            seed.line_range.end,
        )
}

fn matched_seed_count(report: &ReviewReport, manifest: &FixtureManifest) -> usize {
    manifest
        .seeded_defects
        .iter()
        .filter(|seed| {
            report
                .findings
                .iter()
                .any(|finding| finding_matches_seed(finding, seed))
        })
        .count()
}

fn seeded_recall(report: &ReviewReport, manifest: &FixtureManifest) -> f64 {
    if manifest.seeded_defects.is_empty() {
        return 1.0;
    }
    matched_seed_count(report, manifest) as f64 / manifest.seeded_defects.len() as f64
}

fn seeded_high_severity_recall(report: &ReviewReport, manifest: &FixtureManifest) -> f64 {
    let seeds = manifest
        .seeded_defects
        .iter()
        .filter(|seed| {
            matches!(
                seed.severity,
                ReviewSeverity::High | ReviewSeverity::Blocker
            )
        })
        .collect::<Vec<_>>();
    if seeds.is_empty() {
        return 1.0;
    }
    let matched = seeds
        .iter()
        .filter(|seed| {
            report
                .findings
                .iter()
                .any(|finding| finding_matches_seed(finding, seed))
        })
        .count();
    matched as f64 / seeds.len() as f64
}

fn precision_proxy(report: &ReviewReport, manifest: &FixtureManifest) -> f64 {
    let supported = report.facts();
    if supported.is_empty() {
        return 1.0;
    }
    let matched = supported
        .iter()
        .filter(|finding| {
            manifest
                .seeded_defects
                .iter()
                .any(|seed| finding_matches_seed(finding, seed))
        })
        .count();
    matched as f64 / supported.len() as f64
}

fn corroborated_precision(findings: &[(bool, bool)]) -> (f64, f64) {
    let corroborated = findings
        .iter()
        .filter(|(is_corroborated, _)| *is_corroborated)
        .collect::<Vec<_>>();
    let corroborated_precision = if corroborated.is_empty() {
        1.0
    } else {
        corroborated
            .iter()
            .filter(|(_, matches_seed)| *matches_seed)
            .count() as f64
            / corroborated.len() as f64
    };
    let overall_precision = if findings.is_empty() {
        1.0
    } else {
        findings
            .iter()
            .filter(|(_, matches_seed)| *matches_seed)
            .count() as f64
            / findings.len() as f64
    };
    (corroborated_precision, overall_precision)
}

fn ranges_are_near(left: &ReviewFinding, right: &ReviewFinding) -> bool {
    left.line_start <= right.line_end.saturating_add(5)
        && right.line_start <= left.line_end.saturating_add(5)
}

fn are_t13_duplicates(left: &ReviewFinding, right: &ReviewFinding) -> bool {
    normalized_path(&left.file) == normalized_path(&right.file) && ranges_are_near(left, right)
}

fn duplicate_violation_count(report: &ReviewReport) -> usize {
    (0..report.findings.len())
        .filter(|right| {
            (0..*right)
                .any(|left| are_t13_duplicates(&report.findings[left], &report.findings[*right]))
        })
        .count()
}

fn duplicate_rate(report: &ReviewReport, manual_near_duplicates: usize) -> f64 {
    if report.findings.is_empty() {
        return 0.0;
    }
    let duplicates = duplicate_violation_count(report)
        .saturating_add(manual_near_duplicates)
        .min(report.findings.len());
    duplicates as f64 / report.findings.len() as f64
}

fn clean_false_positive_count(report: &ReviewReport, manifest: &FixtureManifest) -> usize {
    if manifest.kind != FixtureKind::Clean {
        return 0;
    }
    report.facts().len()
}

fn high_severity_hypothesis_count(report: &ReviewReport, manifest: &FixtureManifest) -> usize {
    if manifest.kind != FixtureKind::Clean {
        return 0;
    }
    report
        .hypotheses()
        .into_iter()
        .filter(|finding| {
            matches!(
                finding.severity,
                ReviewSeverity::High | ReviewSeverity::Blocker
            )
        })
        .count()
}

fn compute_metrics(
    report: &ReviewReport,
    manifest: &FixtureManifest,
    manual_near_duplicates: usize,
) -> FixtureMetrics {
    FixtureMetrics {
        seeded_recall: seeded_recall(report, manifest),
        seeded_high_severity_recall: seeded_high_severity_recall(report, manifest),
        precision_proxy: precision_proxy(report, manifest),
        duplicate_rate: duplicate_rate(report, manual_near_duplicates),
        clean_false_positive_count: clean_false_positive_count(report, manifest),
        high_severity_hypothesis_count: high_severity_hypothesis_count(report, manifest),
    }
}

fn finding(
    file: &str,
    line_start: u32,
    line_end: u32,
    severity: ReviewSeverity,
    supported: bool,
) -> ReviewFinding {
    ReviewFinding {
        id: String::new(),
        stage: "diff".to_string(),
        model: None,
        title: "Frog finding".to_string(),
        severity,
        file: file.to_string(),
        line_start,
        line_end,
        claim: "Frog finding".to_string(),
        evidence: String::new(),
        evidence_present: supported,
        reproduction: None,
        fix: None,
        introduced_by_diff: true,
        out_of_scope: false,
        reported_by: vec![],
        locations: vec![],
        disputed: None,
    }
}

fn report(findings: Vec<ReviewFinding>) -> ReviewReport {
    ReviewReport {
        depth: "normal".to_string(),
        outcome: ReviewOutcome::Reviewed,
        scope: ReviewScopeSummary::default(),
        diff: ReviewDiffSummary::default(),
        stages: vec![],
        findings,
        duration_ms: 0,
        duplicates_merged: 0,
        scratch_dir: None,
    }
}

fn synthetic_manifest(kind: FixtureKind, seeded_defects: Vec<SeededDefect>) -> FixtureManifest {
    FixtureManifest {
        id: "synthetic_frogs".to_string(),
        kind,
        scenario: "synthetic".to_string(),
        description: "Welcome synthetic frogs.".to_string(),
        seeded_defects,
    }
}

fn seed(file: &str, start: u32, end: u32, category: &str) -> SeededDefect {
    SeededDefect {
        file: file.to_string(),
        line_range: LineRange { start, end },
        category: category.to_string(),
        severity: ReviewSeverity::High,
        description: "Seeded frog defect".to_string(),
        marker: "seeded frog defect".to_string(),
        expected_check: None,
    }
}

#[test]
fn metrics_match_only_overlapping_file_and_lines() {
    let manifest = synthetic_manifest(
        FixtureKind::Seeded,
        vec![
            seed("pond.py", 10, 20, "correctness"),
            seed("bank.py", 30, 35, "security"),
            seed("welcome.py", 40, 45, "tests"),
        ],
    );
    let candidate_report = report(vec![
        finding("fixtures/pond.py", 20, 20, ReviewSeverity::High, true),
        finding("bank.py", 36, 36, ReviewSeverity::High, true),
        finding("welcome.py", 42, 42, ReviewSeverity::High, true),
    ]);

    assert_eq!(seeded_recall(&candidate_report, &manifest), 2.0 / 3.0);
    assert_eq!(
        seeded_high_severity_recall(&candidate_report, &manifest),
        2.0 / 3.0
    );
    assert_eq!(precision_proxy(&candidate_report, &manifest), 2.0 / 3.0);
}

#[test]
fn corroborated_findings_have_at_least_overall_precision() {
    let (corroborated, overall) = corroborated_precision(&[
        (true, true),
        (true, true),
        (false, true),
        (false, false),
        (false, false),
    ]);

    assert!(corroborated >= overall);
    assert_eq!(corroborated, 1.0);
    assert_eq!(overall, 3.0 / 5.0);
}

#[test]
fn corroborated_precision_uses_perfect_empty_denominators() {
    assert_eq!(corroborated_precision(&[]), (1.0, 1.0));
    assert_eq!(corroborated_precision(&[(false, false)]), (1.0, 0.0));
}

#[test]
fn duplicate_rate_checks_t13_invariant_and_manual_hook() {
    let candidate_report = report(vec![
        finding("pond.py", 10, 12, ReviewSeverity::High, true),
        finding("pond.py", 17, 18, ReviewSeverity::Medium, true),
        finding("pond.py", 24, 25, ReviewSeverity::Medium, true),
    ]);

    assert_eq!(duplicate_violation_count(&candidate_report), 1);
    assert_eq!(duplicate_rate(&candidate_report, 0), 1.0 / 3.0);
    assert_eq!(duplicate_rate(&candidate_report, 1), 2.0 / 3.0);
}

#[test]
fn clean_metrics_count_supported_and_high_severity_hypothesis_findings() {
    let manifest = synthetic_manifest(FixtureKind::Clean, vec![]);
    let candidate_report = report(vec![
        finding("pond.py", 1, 1, ReviewSeverity::Low, true),
        finding("pond.py", 10, 10, ReviewSeverity::High, false),
        finding("pond.py", 20, 20, ReviewSeverity::Medium, false),
    ]);
    let metrics = compute_metrics(&candidate_report, &manifest, 0);

    assert_eq!(metrics.seeded_recall, 1.0);
    assert_eq!(metrics.seeded_high_severity_recall, 1.0);
    assert_eq!(metrics.precision_proxy, 0.0);
    assert_eq!(metrics.clean_false_positive_count, 1);
    assert_eq!(metrics.high_severity_hypothesis_count, 1);
}

#[test]
fn fixture_corpus_is_complete_small_and_parseable() {
    let directories = fixture_directories();
    let manifests = directories
        .iter()
        .map(|directory| load_manifest(directory))
        .collect::<Vec<_>>();
    let ids = manifests
        .iter()
        .map(|manifest| manifest.id.as_str())
        .collect::<HashSet<_>>();
    let scenarios = manifests
        .iter()
        .map(|manifest| manifest.scenario.as_str())
        .collect::<HashSet<_>>();
    let seeded_markers = manifests
        .iter()
        .flat_map(|manifest| &manifest.seeded_defects)
        .map(|defect| defect.marker.as_str())
        .collect::<Vec<_>>();

    assert_eq!(directories.len(), 16);
    assert_eq!(ids.len(), directories.len());
    assert_eq!(
        manifests
            .iter()
            .filter(|item| item.kind == FixtureKind::Seeded)
            .count(),
        11
    );
    assert_eq!(
        manifests
            .iter()
            .filter(|item| item.kind == FixtureKind::Clean)
            .count(),
        3
    );
    assert_eq!(
        manifests
            .iter()
            .filter(|item| item.kind == FixtureKind::DuplicateBait)
            .count(),
        2
    );
    for expected in [
        "off_by_one_loop_bound",
        "swallowed_fallible_operation",
        "unsafe_file_path",
        "inconsistent_sibling_api_usage",
        "changed_code_missing_test_update",
        "pure_refactor_rename",
        "comment_doc_only_change",
        "equivalent_logic_reshuffle",
        "same_defect_visible_from_two_files",
        "near_identical_copy_paste_blocks",
        "hardcoded_test_expectation",
        "stub_claimed_complete",
        "hallucinated_import",
        "weakened_assertion",
        "reward_hacked_test",
        "tautological_test",
    ] {
        assert!(scenarios.contains(expected), "missing scenario {expected}");
    }

    let mut category_counts = BTreeMap::new();
    for (directory, manifest) in directories.iter().zip(&manifests) {
        let source_paths = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.file_name().and_then(|name| name.to_str()) != Some("manifest.json"))
            .collect::<Vec<_>>();
        assert!(!source_paths.is_empty());
        let combined = source_paths
            .iter()
            .map(|path| fs::read_to_string(path).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(combined.contains("Welcome"));
        assert!(combined.lines().count() < 200);
        for defect in &manifest.seeded_defects {
            *category_counts
                .entry(defect.category.as_str())
                .or_insert(0usize) += 1;
            let source = fs::read_to_string(directory.join(&defect.file)).unwrap();
            assert!(defect.line_range.start > 0);
            assert!(defect.line_range.start <= defect.line_range.end);
            assert!(defect.line_range.end as usize <= source.lines().count());
            assert!(!defect.description.trim().is_empty());
            assert!(!defect.marker.trim().is_empty());
            if let Some(expected_check) = &defect.expected_check {
                assert!(
                    [
                        "s4_test_integrity",
                        "s5_dependencies",
                        "a3_mutation_probe",
                        "l1_diff",
                        "l2_simplicity",
                        "s1_security",
                        "s2_dead_code",
                        "s3_duplication",
                    ]
                    .contains(&expected_check.as_str()),
                    "{} has unknown expected check {expected_check}",
                    manifest.id
                );
            }
            let defect_source = source
                .lines()
                .skip(defect.line_range.start as usize - 1)
                .take((defect.line_range.end - defect.line_range.start + 1) as usize)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                defect_source.contains(&defect.marker),
                "{} marker {:?} missing from {}:{}-{}",
                manifest.id,
                defect.marker,
                defect.file,
                defect.line_range.start,
                defect.line_range.end
            );
        }
        if manifest.kind == FixtureKind::Clean {
            assert!(manifest.seeded_defects.is_empty());
            for marker in &seeded_markers {
                assert!(
                    !combined.contains(marker),
                    "clean fixture {} contains seeded-defect marker {marker:?}",
                    manifest.id
                );
            }
        }
    }
    assert_eq!(category_counts.get("correctness"), Some(&7));
    assert_eq!(category_counts.get("security"), Some(&1));
    assert_eq!(category_counts.get("consistency"), Some(&1));
    assert_eq!(category_counts.get("tests"), Some(&5));
}

fn source_files(directory: &Path) -> Vec<String> {
    let engine_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.file_name().and_then(|name| name.to_str()) != Some("manifest.json"))
        .map(|path| {
            normalized_path(
                path.strip_prefix(engine_root)
                    .unwrap()
                    .to_string_lossy()
                    .as_ref(),
            )
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn extract_review_report(markdown: &str) -> ReviewReport {
    let start = markdown
        .rfind("```json")
        .expect("review result has JSON block")
        + "```json".len();
    let remaining = &markdown[start..];
    let end = remaining.find("```").expect("review JSON block is closed");
    serde_json::from_str(remaining[..end].trim()).expect("review JSON matches ReviewReport")
}

fn manual_duplicate_scores() -> BTreeMap<String, usize> {
    std::env::var("REFACT_CODE_REVIEW_MANUAL_DUPLICATES")
        .ok()
        .map(|value| serde_json::from_str(&value).expect("manual duplicate scores are JSON"))
        .unwrap_or_default()
}

async fn execute_live_review(
    client: &reqwest::Client,
    engine_url: &str,
    model: &str,
    directory: &Path,
    manifest: &FixtureManifest,
) -> ReviewReport {
    let tool_call_id = format!("bench-{}", manifest.id);
    let payload = json!({
        "messages": [
            {"role": "user", "content": format!("Review fixture {}: {}", manifest.id, manifest.description)},
            {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": tool_call_id,
                    "type": "function",
                    "function": {
                        "name": "review",
                        "arguments": serde_json::to_string(&json!({
                            "what_to_check": manifest.description,
                            "files": source_files(directory),
                        })).unwrap(),
                    }
                }]
            }
        ],
        "n_ctx": 131072,
        "maxgen": 8192,
        "subchat_tool_parameters": {},
        "postprocess_parameters": {
            "use_ast_based_pp": true,
            "useful_background": 5.0,
            "useful_symbol_default": 10.0,
            "downgrade_parent_coef": 0.6,
            "downgrade_body_coef": 0.8,
            "comments_propagate_up_coef": 0.99,
            "close_small_gaps": true,
            "take_floor": 0.0,
            "max_files_n": 60
        },
        "model_name": model,
        "chat_id": format!("code-review-bench-{}", manifest.id),
        "style": null
    });
    let response = client
        .post(format!("{engine_url}/v1/tools-execute"))
        .json(&payload)
        .send()
        .await
        .expect("live engine accepts benchmark request");
    let status = response.status();
    let body: Value = response.json().await.expect("live engine returns JSON");
    assert!(status.is_success(), "live engine returned {status}: {body}");
    assert_eq!(body["tools_ran"], true);
    let markdown = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["tool_call_id"] == tool_call_id)
        .and_then(|message| message["content"].as_str())
        .expect("review returns a text tool result");
    extract_review_report(markdown)
}

#[tokio::test]
#[ignore]
async fn live_review_pipeline_benchmark() {
    let engine_url = std::env::var("REFACT_CODE_REVIEW_ENGINE_URL")
        .expect("REFACT_CODE_REVIEW_ENGINE_URL must point at a live refact-lsp")
        .trim_end_matches('/')
        .to_string();
    let model = std::env::var("REFACT_CODE_REVIEW_MODEL")
        .expect("REFACT_CODE_REVIEW_MODEL must name the configured benchmark model");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1_800))
        .build()
        .unwrap();
    let engine_root = url::Url::from_directory_path(env!("CARGO_MANIFEST_DIR")).unwrap();
    let initialize = client
        .post(format!("{engine_url}/v1/lsp-initialize"))
        .json(&json!({"project_roots": [engine_root.as_str()]}))
        .send()
        .await
        .expect("live engine initializes benchmark workspace");
    assert!(initialize.status().is_success());

    let duplicate_scores = manual_duplicate_scores();
    let output_dir = std::env::var("REFACT_CODE_REVIEW_OUTPUT_DIR")
        .ok()
        .map(PathBuf::from);
    if let Some(output_dir) = &output_dir {
        fs::create_dir_all(output_dir).unwrap();
    }

    println!(
        "fixture\tseeded_recall\tseeded_high_severity_recall\tprecision_proxy\tduplicate_rate\tclean_false_positives\thigh_severity_hypotheses"
    );
    for directory in fixture_directories() {
        let manifest = load_manifest(&directory);
        let report = execute_live_review(&client, &engine_url, &model, &directory, &manifest).await;
        if let Some(output_dir) = &output_dir {
            fs::write(
                output_dir.join(format!("{}.json", manifest.id)),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
        }
        let metrics = compute_metrics(
            &report,
            &manifest,
            duplicate_scores.get(&manifest.id).copied().unwrap_or(0),
        );
        println!(
            "{}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}",
            manifest.id,
            metrics.seeded_recall,
            metrics.seeded_high_severity_recall,
            metrics.precision_proxy,
            metrics.duplicate_rate,
            metrics.clean_false_positive_count,
            metrics.high_severity_hypothesis_count,
        );
    }
}
