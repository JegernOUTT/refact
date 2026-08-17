use std::cmp::Ordering;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::tools::code_review_types::{ReviewFinding, ReviewReport, ReviewSeverity, VerificationStatus};

const MAX_EVIDENCE_PER_FINDING: usize = 8;

fn stable_finding_id(finding: &ReviewFinding) -> String {
    let excerpt = finding
        .evidence
        .iter()
        .find(|evidence| evidence.kind == "excerpt")
        .and_then(|evidence| evidence.path.as_deref().map(|path| (evidence, path)));
    let (excerpt_content, path) = match excerpt {
        Some((excerpt, path)) => (excerpt.content.as_str(), path),
        None => ("", finding.file.as_str()),
    };
    let canonical_path = crate::files_correction::canonicalize_normalized_path(PathBuf::from(path));
    let excerpt_digest = hex::encode(Sha256::digest(excerpt_content.as_bytes()));
    let input = format!(
        "{}|{}|{}|{}",
        finding.category,
        canonical_path.to_string_lossy().replace('\\', "/"),
        finding.line1 / 50,
        excerpt_digest
    );
    let digest = hex::encode(Sha256::digest(input.as_bytes()));
    format!("rf-{}", &digest[..8])
}

fn status_rank(status: &VerificationStatus) -> u8 {
    match status {
        VerificationStatus::Verified => 4,
        VerificationStatus::NeedsHumanValidation => 3,
        VerificationStatus::Unverified => 2,
        VerificationStatus::Downgraded => 1,
        VerificationStatus::Rejected => 0,
    }
}

fn severity_rank(severity: &ReviewSeverity) -> u8 {
    match severity {
        ReviewSeverity::Critical => 4,
        ReviewSeverity::High => 3,
        ReviewSeverity::Medium => 2,
        ReviewSeverity::Low => 1,
    }
}

fn priority(left: &ReviewFinding, right: &ReviewFinding) -> Ordering {
    status_rank(&left.verification_status)
        .cmp(&status_rank(&right.verification_status))
        .then_with(|| severity_rank(&left.severity).cmp(&severity_rank(&right.severity)))
        .then_with(|| left.confidence.total_cmp(&right.confidence))
}

fn ranges_are_near(left: &ReviewFinding, right: &ReviewFinding) -> bool {
    left.line1 <= right.line2.saturating_add(5) && right.line1 <= left.line2.saturating_add(5)
}

fn are_duplicates(left: &ReviewFinding, right: &ReviewFinding) -> bool {
    left.file == right.file && left.category == right.category && ranges_are_near(left, right)
}

fn root(parents: &mut [usize], index: usize) -> usize {
    if parents[index] != index {
        parents[index] = root(parents, parents[index]);
    }
    parents[index]
}

fn union(parents: &mut [usize], left: usize, right: usize) {
    let left_root = root(parents, left);
    let right_root = root(parents, right);
    if left_root != right_root {
        parents[right_root] = left_root;
    }
}

fn deduplicate(findings: Vec<ReviewFinding>) -> Vec<ReviewFinding> {
    let mut parents = (0..findings.len()).collect::<Vec<_>>();
    for left in 0..findings.len() {
        for right in left + 1..findings.len() {
            if are_duplicates(&findings[left], &findings[right]) {
                union(&mut parents, left, right);
            }
        }
    }

    let roots = (0..findings.len())
        .map(|index| root(&mut parents, index))
        .collect::<Vec<_>>();
    let mut survivors = Vec::new();
    for group_root in 0..findings.len() {
        let indices = roots
            .iter()
            .enumerate()
            .filter_map(|(index, root)| (*root == group_root).then_some(index))
            .collect::<Vec<_>>();
        if indices.is_empty() {
            continue;
        }

        let mut survivor_index = indices[0];
        for index in indices.iter().copied().skip(1) {
            let ordering = priority(&findings[index], &findings[survivor_index]);
            if ordering == Ordering::Greater
                || (ordering == Ordering::Equal && findings[index].id < findings[survivor_index].id)
            {
                survivor_index = index;
            }
        }

        let mut survivor = findings[survivor_index].clone();
        survivor.evidence.truncate(MAX_EVIDENCE_PER_FINDING);
        for index in indices.iter().copied() {
            if index == survivor_index {
                continue;
            }
            for evidence in &findings[index].evidence {
                if survivor.evidence.len() == MAX_EVIDENCE_PER_FINDING {
                    break;
                }
                if !survivor.evidence.contains(evidence) {
                    survivor.evidence.push(evidence.clone());
                }
            }
            let marker = format!("deduped_from:{}", findings[index].id);
            if !survivor.checks_performed.contains(&marker) {
                survivor.checks_performed.push(marker);
            }
        }
        survivors.push(survivor);
    }

    debug_assert!((0..survivors.len()).all(|left| {
        (left + 1..survivors.len())
            .all(|right| !are_duplicates(&survivors[left], &survivors[right]))
    }));
    survivors
}

fn rank_findings(findings: &mut [ReviewFinding]) {
    findings.sort_by(|left, right| {
        priority(right, left)
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.line1.cmp(&right.line1))
            .then_with(|| left.line2.cmp(&right.line2))
            .then_with(|| left.claim.cmp(&right.claim))
    });
}

fn clean_summary(report: &ReviewReport, reviewer_summary: &str) -> String {
    let file_count = report.scope.files_reviewed.len();
    let file_word = if file_count == 1 { "file" } else { "files" };
    let checks = if report.checks_performed.is_empty() {
        "none recorded".to_string()
    } else {
        report.checks_performed.join(", ")
    };
    let mut summary = format!(
        "Reviewed {file_count} {file_word}. Checks performed: {checks}. No verified findings."
    );
    if !reviewer_summary.trim().is_empty() {
        summary.push(' ');
        summary.push_str(reviewer_summary.trim());
    }
    summary
}

pub(super) fn finalize_review_report(report: &mut ReviewReport) {
    report
        .findings
        .retain(|finding| finding.verification_status != VerificationStatus::Rejected);
    for finding in &mut report.findings {
        finding.id = stable_finding_id(finding);
    }
    report.findings = deduplicate(std::mem::take(&mut report.findings));
    rank_findings(&mut report.findings);
    if report.findings.is_empty() {
        let reviewer_summary = std::mem::take(&mut report.summary);
        report.summary = clean_summary(report, &reviewer_summary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::code_review_types::{ReviewEvidence, ReviewScopeSummary};

    fn finding(
        file: &str,
        line1: u32,
        line2: u32,
        category: &str,
        claim: &str,
        status: VerificationStatus,
        severity: ReviewSeverity,
        confidence: f32,
    ) -> ReviewFinding {
        let canonical_path =
            crate::files_correction::canonicalize_normalized_path(PathBuf::from(file));
        ReviewFinding {
            id: String::new(),
            category: category.to_string(),
            severity,
            confidence,
            verification_status: status,
            file: file.to_string(),
            line1,
            line2,
            claim: claim.to_string(),
            evidence: vec![ReviewEvidence {
                kind: "excerpt".to_string(),
                path: Some(canonical_path.to_string_lossy().to_string()),
                line1: Some(line1),
                line2: Some(line2),
                content: "deterministic excerpt".to_string(),
            }],
            impact: None,
            remediation: None,
            checks_performed: vec![],
        }
    }

    fn report(findings: Vec<ReviewFinding>) -> ReviewReport {
        ReviewReport {
            scope: ReviewScopeSummary {
                files_reviewed: vec!["src/lib.rs".to_string()],
                focus: None,
                diff_base: None,
            },
            findings,
            checks_performed: vec![],
            summary: String::new(),
            pipeline: Default::default(),
        }
    }

    fn evidence(index: usize) -> ReviewEvidence {
        ReviewEvidence {
            kind: "excerpt".to_string(),
            path: Some("src/lib.rs".to_string()),
            line1: Some(index as u32),
            line2: Some(index as u32),
            content: format!("evidence-{index}"),
        }
    }

    #[test]
    fn tool_code_review_stable_id_ignores_claim_and_reviewer_path_spelling() {
        let absolute_path =
            crate::files_correction::canonicalize_normalized_path(PathBuf::from("src/lib.rs"));
        let first = finding(
            "src/lib.rs",
            110,
            112,
            "correctness",
            "The branch drops errors",
            VerificationStatus::Verified,
            ReviewSeverity::High,
            0.9,
        );
        let second = finding(
            &absolute_path.to_string_lossy(),
            110,
            112,
            "correctness",
            "Errors disappear when this branch executes",
            VerificationStatus::Verified,
            ReviewSeverity::High,
            0.9,
        );

        let first_id = stable_finding_id(&first);
        assert!(first_id.starts_with("rf-"));
        assert_eq!(first_id.len(), 11);
        assert!(first_id[3..]
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
        assert_eq!(first_id, stable_finding_id(&second));
    }

    #[test]
    fn tool_code_review_stable_id_distinguishes_different_anchors() {
        let first = finding(
            "src/lib.rs",
            10,
            12,
            "correctness",
            "The first defect",
            VerificationStatus::Verified,
            ReviewSeverity::High,
            0.9,
        );
        let mut second = finding(
            "src/lib.rs",
            20,
            22,
            "correctness",
            "The second defect",
            VerificationStatus::Verified,
            ReviewSeverity::High,
            0.9,
        );
        second.evidence[0].content = "different deterministic excerpt".to_string();

        assert_ne!(stable_finding_id(&first), stable_finding_id(&second));
    }

    #[test]
    fn tool_code_review_dedup_collapses_overlap_and_small_gaps_but_not_categories() {
        let mut overlap = finding(
            "src/lib.rs",
            10,
            20,
            "correctness",
            "The first failure",
            VerificationStatus::Unverified,
            ReviewSeverity::Critical,
            0.99,
        );
        overlap.evidence = (0..6).map(evidence).collect();
        let overlap_id = stable_finding_id(&overlap);
        let mut gap = finding(
            "src/lib.rs",
            18,
            25,
            "correctness",
            "The second failure",
            VerificationStatus::Verified,
            ReviewSeverity::Medium,
            0.5,
        );
        gap.evidence = (4..10).map(evidence).collect();
        let gap_id = stable_finding_id(&gap);
        let mut survivor = finding(
            "src/lib.rs",
            30,
            31,
            "correctness",
            "The surviving failure",
            VerificationStatus::Verified,
            ReviewSeverity::High,
            0.4,
        );
        survivor.evidence = (9..12).map(evidence).collect();
        let separate = finding(
            "src/lib.rs",
            18,
            25,
            "security",
            "The security failure",
            VerificationStatus::Verified,
            ReviewSeverity::Low,
            0.2,
        );
        let mut report = report(vec![overlap, gap, survivor, separate]);

        finalize_review_report(&mut report);

        assert_eq!(report.findings.len(), 2);
        let correctness = report
            .findings
            .iter()
            .find(|finding| finding.category == "correctness")
            .unwrap();
        assert_eq!(correctness.claim, "The surviving failure");
        assert_eq!(correctness.evidence.len(), MAX_EVIDENCE_PER_FINDING);
        assert!(correctness
            .checks_performed
            .contains(&format!("deduped_from:{overlap_id}")));
        assert!(correctness
            .checks_performed
            .contains(&format!("deduped_from:{gap_id}")));
        assert!((0..report.findings.len()).all(|left| {
            (left + 1..report.findings.len())
                .all(|right| !are_duplicates(&report.findings[left], &report.findings[right]))
        }));
    }

    #[test]
    fn tool_code_review_rank_orders_status_severity_and_confidence() {
        let cases = [
            (
                "verified-low",
                VerificationStatus::Verified,
                ReviewSeverity::Low,
                0.1,
            ),
            (
                "needs-human",
                VerificationStatus::NeedsHumanValidation,
                ReviewSeverity::Critical,
                1.0,
            ),
            (
                "unverified",
                VerificationStatus::Unverified,
                ReviewSeverity::Critical,
                1.0,
            ),
            (
                "downgraded",
                VerificationStatus::Downgraded,
                ReviewSeverity::Critical,
                1.0,
            ),
            (
                "verified-critical-low-confidence",
                VerificationStatus::Verified,
                ReviewSeverity::Critical,
                0.2,
            ),
            (
                "verified-critical-high-confidence",
                VerificationStatus::Verified,
                ReviewSeverity::Critical,
                0.9,
            ),
        ];
        let findings = cases
            .into_iter()
            .enumerate()
            .map(|(index, (claim, status, severity, confidence))| {
                finding(
                    &format!("src/{index}.rs"),
                    1,
                    1,
                    "correctness",
                    claim,
                    status,
                    severity,
                    confidence,
                )
            })
            .collect();
        let mut report = report(findings);

        finalize_review_report(&mut report);

        assert_eq!(
            report
                .findings
                .iter()
                .map(|finding| finding.claim.as_str())
                .collect::<Vec<_>>(),
            vec![
                "verified-critical-high-confidence",
                "verified-critical-low-confidence",
                "verified-low",
                "needs-human",
                "unverified",
                "downgraded",
            ]
        );
    }

    #[test]
    fn tool_code_review_clean_summary_is_never_empty() {
        let mut report = report(vec![]);
        report.scope.files_reviewed.push("src/main.rs".to_string());
        report.checks_performed = vec![
            "candidate_scan".to_string(),
            "verifier_rejected:1".to_string(),
        ];

        finalize_review_report(&mut report);

        assert_eq!(
            report.summary,
            "Reviewed 2 files. Checks performed: candidate_scan, verifier_rejected:1. No verified findings."
        );
    }
}
