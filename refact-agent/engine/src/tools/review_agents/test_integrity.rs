use std::collections::BTreeSet;

use std::sync::LazyLock;
use regex::Regex;

use crate::tools::review_types::{
    evidence_kinds, RankTier, ReviewEvidence, ReviewFinding, ReviewSeverity, VerificationStatus,
};

pub const AGENT_ID: &str = "s4_test_integrity";
const MAX_FINDINGS: usize = 20;

#[derive(Debug, Clone)]
struct FileDiff {
    path: String,
    first_new_line: u32,
    added: Vec<(u32, String)>,
    removed: Vec<(u32, String)>,
}

fn parse_patch(patch: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut current: Option<FileDiff> = None;
    let mut new_line: u32 = 0;
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(prev) = current.take() {
                files.push(prev);
            }
            let path = rest
                .split_whitespace()
                .nth(1)
                .map(|p| p.trim_matches('"').trim_start_matches("b/").to_string())
                .unwrap_or_default();
            current = Some(FileDiff {
                path,
                first_new_line: 0,
                added: vec![],
                removed: vec![],
            });
            new_line = 0;
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue;
        };
        if line.starts_with("@@") {
            let start = line
                .split_whitespace()
                .find(|part| part.starts_with('+'))
                .and_then(|part| {
                    part.trim_start_matches('+')
                        .split(',')
                        .next()
                        .and_then(|v| v.parse::<u32>().ok())
                })
                .unwrap_or(1);
            new_line = start;
            if file.first_new_line == 0 {
                file.first_new_line = start;
            }
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("index ") {
            continue;
        }
        if let Some(text) = line.strip_prefix('+') {
            file.added.push((new_line, text.to_string()));
            new_line = new_line.saturating_add(1);
        } else if let Some(text) = line.strip_prefix('-') {
            file.removed.push((new_line.max(1), text.to_string()));
        } else {
            new_line = new_line.saturating_add(1);
        }
    }
    if let Some(prev) = current.take() {
        files.push(prev);
    }
    files
}

pub fn is_test_path(path: &str) -> bool {
    refact_git_intel::paths::is_test_path(path)
}

static TEST_DECL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?x)
        (\#\[(tokio::)?test\]) |
        (\bdef\s+(test_\w+)) |
        (\bfn\s+(test_\w+)) |
        (\b(it|test|describe)\s*\(\s*["'`](?P<jsname>[^"'`]+)) |
        (\bfunc\s+(Test\w+))
        "#,
    )
    .unwrap()
});

static ASSERT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bassert|\bexpect\s*\(|ASSERT_|EXPECT_|\.should\b|\brequire\.\w+\(").unwrap()
});

static SKIP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\#\[ignore|pytest\.mark\.skip|unittest\.skip|\bxit\s*\(|\bxdescribe\s*\(|\b(it|test|describe)\.skip\s*\(|\bt\.Skip\(|@Disabled|@Ignore\b",
    )
    .unwrap()
});

static TOLERANCE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(approx|tolerance|delta|epsilon|places|rel_tol|abs_tol|rtol|atol)\D{0,10}(\d+\.?\d*(?:e-?\d+)?)").unwrap()
});

static WEAK_MATCHER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"toBeTruthy\(|toBeDefined\(|toBeNull\(|not\.toThrow\(|assertTrue\s*\(|assertIsNotNone\s*\(|is\s+not\s+None",
    )
    .unwrap()
});

static STRONG_MATCHER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"toEqual\(|toStrictEqual\(|toBe\(|assertEqual|assertEquals|==").unwrap()
});

static STRING_LITERAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""([^"\\]{8,})""#).unwrap());
static NUMBER_LITERAL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(\d{4,})\b").unwrap());

const HARNESS_BASENAMES: &[&str] = &[
    "conftest.py",
    "pytest.ini",
    "tox.ini",
    "setup.cfg",
    "mocha.opts",
];
const HARNESS_STEMS: &[&str] = &["jest.config", "vitest.config", "karma.conf", ".mocharc"];

fn make_finding(
    file: &str,
    line1: u32,
    line2: u32,
    severity: ReviewSeverity,
    confidence: f32,
    claim: String,
    snippet: String,
    check: &str,
) -> ReviewFinding {
    ReviewFinding {
        id: String::new(),
        category: "tests".to_string(),
        severity,
        confidence,
        verification_status: VerificationStatus::Unverified,
        rank_tier: RankTier::Unverified,
        sources: vec![AGENT_ID.to_string()],
        file: file.to_string(),
        line1: line1.max(1),
        line2: line2.max(line1).max(1),
        claim,
        evidence: vec![ReviewEvidence {
            kind: evidence_kinds::STATIC_FACT.to_string(),
            path: Some(file.to_string()),
            line1: Some(line1.max(1)),
            line2: Some(line2.max(line1).max(1)),
            content: snippet,
        }],
        impact: None,
        remediation: None,
        checks_performed: vec![format!("s4:{check}")],
    }
}

fn test_name_from_line(line: &str) -> Option<String> {
    let caps = TEST_DECL.captures(line)?;
    for group in [4, 6, 10] {
        if let Some(name) = caps.get(group) {
            return Some(name.as_str().to_string());
        }
    }
    caps.name("jsname").map(|m| m.as_str().to_string())
}

fn extract_literals(text: &str) -> BTreeSet<String> {
    let mut literals = BTreeSet::new();
    for cap in STRING_LITERAL_RE.captures_iter(text) {
        literals.insert(cap[1].to_string());
    }
    for cap in NUMBER_LITERAL_RE.captures_iter(text) {
        let value = &cap[1];
        if !value.ends_with("000") {
            literals.insert(value.to_string());
        }
    }
    literals
}

pub fn analyze_test_integrity(patch: &str, changed_files: &[String]) -> Vec<ReviewFinding> {
    let mut findings = Vec::new();
    let diffs = parse_patch(patch);

    for diff in &diffs {
        let in_test_file = is_test_path(&diff.path);
        let anchor = diff.first_new_line.max(1);

        if in_test_file {
            for (line_no, removed) in &diff.removed {
                if let Some(name) = test_name_from_line(removed) {
                    let survives = diff
                        .added
                        .iter()
                        .any(|(_, added)| added.contains(name.as_str()));
                    if !survives {
                        findings.push(make_finding(
                            &diff.path,
                            *line_no,
                            *line_no,
                            ReviewSeverity::High,
                            0.85,
                            format!(
                                "Test `{name}` was deleted without a replacement in this diff."
                            ),
                            format!("- {}", removed.trim()),
                            "deleted_test",
                        ));
                    }
                }
            }

            let removed_asserts = diff
                .removed
                .iter()
                .filter(|(_, text)| ASSERT_RE.is_match(text))
                .count();
            let added_asserts = diff
                .added
                .iter()
                .filter(|(_, text)| ASSERT_RE.is_match(text))
                .count();
            if removed_asserts > added_asserts && removed_asserts - added_asserts >= 2 {
                findings.push(make_finding(
                    &diff.path,
                    anchor,
                    anchor,
                    ReviewSeverity::Medium,
                    0.75,
                    format!(
                        "Assertion count in `{}` dropped from {removed_asserts} removed to {added_asserts} added in this diff.",
                        diff.path
                    ),
                    format!("removed asserts: {removed_asserts}, added asserts: {added_asserts}"),
                    "assertions_reduced",
                ));
            }

            for (line_no, added) in &diff.added {
                if SKIP_RE.is_match(added) {
                    findings.push(make_finding(
                        &diff.path,
                        *line_no,
                        *line_no,
                        ReviewSeverity::High,
                        0.9,
                        format!(
                            "A test in `{}` was disabled or skipped in this diff.",
                            diff.path
                        ),
                        format!("+ {}", added.trim()),
                        "test_skipped",
                    ));
                }
            }

            let removed_tolerances: Vec<f64> = diff
                .removed
                .iter()
                .filter_map(|(_, text)| {
                    TOLERANCE_RE
                        .captures_iter(text)
                        .last()
                        .and_then(|c| c[2].parse::<f64>().ok())
                })
                .collect();
            for (line_no, added) in &diff.added {
                if let Some(caps) = TOLERANCE_RE.captures_iter(added).last() {
                    if let Ok(new_value) = caps[2].parse::<f64>() {
                        if removed_tolerances
                            .iter()
                            .any(|old| new_value > *old && *old > 0.0)
                        {
                            findings.push(make_finding(
                                &diff.path,
                                *line_no,
                                *line_no,
                                ReviewSeverity::Medium,
                                0.7,
                                format!(
                                    "A numeric test tolerance in `{}` was widened in this diff.",
                                    diff.path
                                ),
                                format!("+ {}", added.trim()),
                                "tolerance_widened",
                            ));
                            break;
                        }
                    }
                }
            }

            let removed_strong = diff
                .removed
                .iter()
                .any(|(_, text)| STRONG_MATCHER_RE.is_match(text));
            if removed_strong {
                for (line_no, added) in &diff.added {
                    if WEAK_MATCHER_RE.is_match(added) && !STRONG_MATCHER_RE.is_match(added) {
                        findings.push(make_finding(
                            &diff.path,
                            *line_no,
                            *line_no,
                            ReviewSeverity::Medium,
                            0.7,
                            format!(
                                "An exact assertion in `{}` was replaced with a weaker matcher.",
                                diff.path
                            ),
                            format!("+ {}", added.trim()),
                            "assertion_weakened",
                        ));
                        break;
                    }
                }
            }
        }
    }

    for path in changed_files {
        let basename = path.rsplit('/').next().unwrap_or(path);
        let is_harness = HARNESS_BASENAMES.contains(&basename)
            || HARNESS_STEMS.iter().any(|stem| basename.starts_with(stem));
        if is_harness {
            findings.push(make_finding(
                path,
                1,
                1,
                ReviewSeverity::Low,
                0.6,
                format!("Test harness configuration `{path}` changed in this diff."),
                format!("changed file: {path}"),
                "harness_config_changed",
            ));
        }
    }

    let snapshot_changed: Vec<&String> = changed_files
        .iter()
        .filter(|path| {
            path.contains("__snapshots__") || path.ends_with(".snap") || path.ends_with(".golden")
        })
        .collect();
    let impl_changed = changed_files
        .iter()
        .any(|path| !is_test_path(path) && !path.contains("__snapshots__"));
    if !snapshot_changed.is_empty() && impl_changed {
        let path = snapshot_changed[0];
        findings.push(make_finding(
            path,
            1,
            1,
            ReviewSeverity::Medium,
            0.65,
            format!(
                "{} snapshot file(s) were regenerated alongside implementation changes; verify the new snapshots are intended.",
                snapshot_changed.len()
            ),
            snapshot_changed
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            "snapshots_regenerated",
        ));
    }

    let mut impl_literals = BTreeSet::new();
    for diff in &diffs {
        if !is_test_path(&diff.path) {
            for (_, added) in &diff.added {
                impl_literals.extend(extract_literals(added));
            }
        }
    }
    if !impl_literals.is_empty() {
        'outer: for diff in &diffs {
            if is_test_path(&diff.path) {
                for (line_no, added) in &diff.added {
                    if !ASSERT_RE.is_match(added) {
                        continue;
                    }
                    for literal in extract_literals(added) {
                        if impl_literals.contains(&literal) {
                            findings.push(make_finding(
                                &diff.path,
                                *line_no,
                                *line_no,
                                ReviewSeverity::High,
                                0.7,
                                format!(
                                    "Test expectation in `{}` duplicates implementation literal `{literal}`; the test may encode the implementation instead of the requirement.",
                                    diff.path
                                ),
                                format!("+ {}", added.trim()),
                                "literal_overlap",
                            ));
                            break 'outer;
                        }
                    }
                }
            }
        }
    }

    findings.truncate(MAX_FINDINGS);
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(body: &str) -> String {
        body.to_string()
    }

    #[test]
    fn s4_detects_deleted_test_function() {
        let p = patch(
            "diff --git a/tests/test_pond.py b/tests/test_pond.py\n\
             --- a/tests/test_pond.py\n\
             +++ b/tests/test_pond.py\n\
             @@ -10,4 +10,1 @@\n\
             -def test_frog_jumps():\n\
             -    assert jump() == 3\n\
             -\n\
              pass\n",
        );
        let findings = analyze_test_integrity(&p, &["tests/test_pond.py".to_string()]);
        assert!(findings.iter().any(|f| f
            .checks_performed
            .contains(&"s4:deleted_test".to_string())
            && f.claim.contains("test_frog_jumps")));
    }

    #[test]
    fn s4_keeps_renamed_test_quiet_when_name_survives() {
        let p = patch(
            "diff --git a/tests/test_pond.py b/tests/test_pond.py\n\
             @@ -10,2 +10,2 @@\n\
             -def test_frog_jumps():\n\
             +def test_frog_jumps_far():  # renamed but contains test_frog_jumps\n",
        );
        let findings = analyze_test_integrity(&p, &["tests/test_pond.py".to_string()]);
        assert!(!findings
            .iter()
            .any(|f| f.checks_performed.contains(&"s4:deleted_test".to_string())));
    }

    #[test]
    fn s4_detects_added_skip_and_assertion_reduction() {
        let p = patch(
            "diff --git a/src/pond.test.ts b/src/pond.test.ts\n\
             @@ -1,6 +1,3 @@\n\
             -expect(jump()).toEqual(3);\n\
             -expect(croak()).toEqual(\"ribbit\");\n\
             -expect(swim()).toEqual(true);\n\
             +it.skip(\"jumps\", () => {});\n",
        );
        let findings = analyze_test_integrity(&p, &["src/pond.test.ts".to_string()]);
        assert!(findings
            .iter()
            .any(|f| f.checks_performed.contains(&"s4:test_skipped".to_string())));
        assert!(findings.iter().any(|f| f
            .checks_performed
            .contains(&"s4:assertions_reduced".to_string())));
    }

    #[test]
    fn s4_detects_tolerance_widening() {
        let p = patch(
            "diff --git a/tests/test_math.py b/tests/test_math.py\n\
             @@ -5,1 +5,1 @@\n\
             -    assert result == pytest.approx(1.0, rel_tol=0.001)\n\
             +    assert result == pytest.approx(1.0, rel_tol=0.5)\n",
        );
        let findings = analyze_test_integrity(&p, &["tests/test_math.py".to_string()]);
        assert!(findings.iter().any(|f| f
            .checks_performed
            .contains(&"s4:tolerance_widened".to_string())));
    }

    #[test]
    fn s4_detects_matcher_loosening() {
        let p = patch(
            "diff --git a/src/pond.test.ts b/src/pond.test.ts\n\
             @@ -3,1 +3,1 @@\n\
             -expect(depth()).toEqual(42);\n\
             +expect(depth()).toBeDefined();\n",
        );
        let findings = analyze_test_integrity(&p, &["src/pond.test.ts".to_string()]);
        assert!(findings.iter().any(|f| f
            .checks_performed
            .contains(&"s4:assertion_weakened".to_string())));
    }

    #[test]
    fn s4_detects_harness_and_snapshot_changes() {
        let findings = analyze_test_integrity(
            "",
            &[
                "conftest.py".to_string(),
                "src/pond.py".to_string(),
                "src/__snapshots__/pond.snap".to_string(),
            ],
        );
        assert!(findings.iter().any(|f| f
            .checks_performed
            .contains(&"s4:harness_config_changed".to_string())));
        assert!(findings.iter().any(|f| f
            .checks_performed
            .contains(&"s4:snapshots_regenerated".to_string())));
    }

    #[test]
    fn s4_detects_impl_literal_copied_into_test_expectation() {
        let p = patch(
            "diff --git a/src/pond.py b/src/pond.py\n\
             @@ -1,0 +1,1 @@\n\
             +MAGIC_TOKEN = \"frog-9271-secret\"\n\
             diff --git a/tests/test_pond.py b/tests/test_pond.py\n\
             @@ -1,0 +1,1 @@\n\
             +assert token() == \"frog-9271-secret\"\n",
        );
        let findings = analyze_test_integrity(
            &p,
            &["src/pond.py".to_string(), "tests/test_pond.py".to_string()],
        );
        assert!(findings.iter().any(|f| f
            .checks_performed
            .contains(&"s4:literal_overlap".to_string())));
    }

    #[test]
    fn s4_is_quiet_on_clean_diff() {
        let p = patch(
            "diff --git a/src/pond.py b/src/pond.py\n\
             @@ -1,1 +1,1 @@\n\
             -def jump(): return 2\n\
             +def jump(): return 3\n\
             diff --git a/tests/test_pond.py b/tests/test_pond.py\n\
             @@ -1,1 +1,2 @@\n\
             -assert jump() == 2\n\
             +assert jump() == 3\n\
             +assert jump() != 0\n",
        );
        let findings = analyze_test_integrity(
            &p,
            &["src/pond.py".to_string(), "tests/test_pond.py".to_string()],
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn s4_anchors_lines_in_new_file_coordinates() {
        let p = patch(
            "diff --git a/tests/test_pond.py b/tests/test_pond.py\n\
             @@ -40,2 +40,3 @@\n\
              def helper():\n\
             +    pytest.mark.skip(reason=\"later\")\n\
              x = 1\n",
        );
        let findings = analyze_test_integrity(&p, &["tests/test_pond.py".to_string()]);
        let skip = findings
            .iter()
            .find(|f| f.checks_performed.contains(&"s4:test_skipped".to_string()))
            .unwrap();
        assert_eq!(skip.line1, 41);
    }
}
