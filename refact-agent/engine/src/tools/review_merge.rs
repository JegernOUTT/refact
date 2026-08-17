use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::tools::review_types::{
    evidence_kinds, RankTier, ReviewFinding, ReviewReport, ReviewSeverity, VerificationStatus,
};

const MAX_EVIDENCE_PER_FINDING: usize = 8;
const NEAR_RANGE_LINES: u32 = 5;
const HOT_CHURN_PERCENTILE: f64 = 0.85;
const HOT_FAN_IN: usize = 8;
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileRiskFacts {
    pub churn_percentile: f64,
    pub temporal_hotspot: bool,
    pub fan_in: usize,
    pub bus_factor_risk: bool,
}

impl FileRiskFacts {
    pub fn is_hot(&self) -> bool {
        self.churn_percentile >= HOT_CHURN_PERCENTILE
            || self.temporal_hotspot
            || self.fan_in >= HOT_FAN_IN
    }

    pub fn summary(&self) -> String {
        format!(
            "churn_percentile={:.2} temporal_hotspot={} fan_in={} bus_factor_risk={}",
            self.churn_percentile, self.temporal_hotspot, self.fan_in, self.bus_factor_risk
        )
    }
}
#[derive(Debug, Clone, Default)]
pub struct RiskEnrichment {
    by_path: HashMap<String, FileRiskFacts>,
}

impl RiskEnrichment {
    pub fn insert(&mut self, path: &str, facts: FileRiskFacts) {
        self.by_path.insert(normalize_path(path), facts);
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    pub fn lookup(&self, path: &str) -> Option<&FileRiskFacts> {
        let normalized = normalize_path(path);
        if let Some(facts) = self.by_path.get(&normalized) {
            return Some(facts);
        }
        self.by_path.iter().find_map(|(key, facts)| {
            let suffix_match = normalized.ends_with(&format!("/{key}"))
                || key.ends_with(&format!("/{normalized}"));
            suffix_match.then_some(facts)
        })
    }
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

pub(crate) fn stable_finding_id(finding: &ReviewFinding) -> String {
    let excerpt = finding
        .evidence
        .iter()
        .find(|evidence| evidence.kind == evidence_kinds::EXCERPT)
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

fn bump_severity(severity: ReviewSeverity) -> ReviewSeverity {
    match severity {
        ReviewSeverity::Low => ReviewSeverity::Medium,
        ReviewSeverity::Medium => ReviewSeverity::High,
        ReviewSeverity::High | ReviewSeverity::Critical => ReviewSeverity::Critical,
    }
}
fn median_severity(mut severities: Vec<ReviewSeverity>) -> ReviewSeverity {
    debug_assert!(!severities.is_empty());
    severities.sort_by_key(severity_rank);
    severities[(severities.len().saturating_sub(1)) / 2]
}

fn priority(left: &ReviewFinding, right: &ReviewFinding) -> Ordering {
    status_rank(&left.verification_status)
        .cmp(&status_rank(&right.verification_status))
        .then_with(|| severity_rank(&left.severity).cmp(&severity_rank(&right.severity)))
        .then_with(|| left.confidence.total_cmp(&right.confidence))
}

fn ranges_are_near(left: &ReviewFinding, right: &ReviewFinding) -> bool {
    left.line1 <= right.line2.saturating_add(NEAR_RANGE_LINES)
        && right.line1 <= left.line2.saturating_add(NEAR_RANGE_LINES)
}

fn are_duplicates(left: &ReviewFinding, right: &ReviewFinding) -> bool {
    left.file == right.file && left.category == right.category && ranges_are_near(left, right)
}

fn has_execution_evidence(finding: &ReviewFinding) -> bool {
    finding.evidence.iter().any(|evidence| {
        evidence.kind == evidence_kinds::MUTATION_PROBE
            || evidence.kind == evidence_kinds::EXECUTION_OUTPUT
    })
}

pub(crate) fn compute_rank_tier(finding: &ReviewFinding) -> RankTier {
    if has_execution_evidence(finding) {
        return RankTier::ExecutionReproduced;
    }
    if finding.sources.len() >= 2 {
        return RankTier::Corroborated;
    }
    match finding.verification_status {
        VerificationStatus::Verified => RankTier::Verified,
        VerificationStatus::NeedsHumanValidation => RankTier::NeedsHumanValidation,
        VerificationStatus::Downgraded => RankTier::Downgraded,
        VerificationStatus::Unverified | VerificationStatus::Rejected => RankTier::Unverified,
    }
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

fn merge_clusters(findings: Vec<ReviewFinding>, enrichment: &RiskEnrichment) -> Vec<ReviewFinding> {
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
            let member = &findings[index];
            for evidence in &member.evidence {
                if survivor.evidence.len() == MAX_EVIDENCE_PER_FINDING {
                    break;
                }
                if !survivor.evidence.contains(evidence) {
                    survivor.evidence.push(evidence.clone());
                }
            }
            for source in &member.sources {
                if !survivor.sources.contains(source) {
                    survivor.sources.push(source.clone());
                }
            }
            if status_rank(&member.verification_status) > status_rank(&survivor.verification_status)
            {
                survivor.verification_status = member.verification_status.clone();
            }
            if member.confidence > survivor.confidence {
                survivor.confidence = member.confidence;
            }
            let marker = format!("deduped_from:{}", member.id);
            if !survivor.checks_performed.contains(&marker) {
                survivor.checks_performed.push(marker);
            }
        }

        if indices.len() > 1 {
            survivor.severity = median_severity(
                indices
                    .iter()
                    .map(|&index| findings[index].severity)
                    .collect(),
            );
        }
        if let Some(facts) = enrichment.lookup(&survivor.file) {
            if facts.is_hot() {
                let bumped = bump_severity(survivor.severity);
                if bumped != survivor.severity {
                    survivor.severity = bumped;
                    let marker = "severity_bumped:hot_path".to_string();
                    if !survivor.checks_performed.contains(&marker) {
                        survivor.checks_performed.push(marker);
                    }
                }
                survivor
                    .evidence
                    .truncate(MAX_EVIDENCE_PER_FINDING.saturating_sub(1));
                survivor
                    .evidence
                    .push(crate::tools::review_types::ReviewEvidence {
                        kind: evidence_kinds::STATIC_FACT.to_string(),
                        path: Some(survivor.file.clone()),
                        line1: None,
                        line2: None,
                        content: format!("hot file: {}", facts.summary()),
                    });
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
        right
            .rank_tier
            .rank()
            .cmp(&left.rank_tier.rank())
            .then_with(|| severity_rank(&right.severity).cmp(&severity_rank(&left.severity)))
            .then_with(|| right.confidence.total_cmp(&left.confidence))
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
pub fn finalize_review_report(report: &mut ReviewReport, enrichment: &RiskEnrichment) {
    report
        .findings
        .retain(|finding| finding.verification_status != VerificationStatus::Rejected);
    for finding in &mut report.findings {
        finding.id = stable_finding_id(finding);
    }
    report.findings = merge_clusters(std::mem::take(&mut report.findings), enrichment);
    for finding in &mut report.findings {
        finding.rank_tier = compute_rank_tier(finding);
    }
    rank_findings(&mut report.findings);
    if report.findings.is_empty() {
        let reviewer_summary = std::mem::take(&mut report.summary);
        report.summary = clean_summary(report, &reviewer_summary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::review_types::{ReviewEvidence, ReviewScopeSummary};

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
            rank_tier: RankTier::Unverified,
            sources: vec![],
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
            assumed_intent: None,
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
    fn tool_review_stable_id_ignores_claim_and_reviewer_path_spelling() {
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
    fn tool_review_stable_id_distinguishes_different_anchors() {
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
    fn tool_review_merge_collapses_overlap_and_small_gaps_but_not_categories() {
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

        finalize_review_report(&mut report, &RiskEnrichment::default());

        assert_eq!(report.findings.len(), 2);
        let correctness = report
            .findings
            .iter()
            .find(|finding| finding.category == "correctness")
            .unwrap();
        assert_eq!(correctness.claim, "The surviving failure");
        assert_eq!(correctness.severity, ReviewSeverity::High);
        assert_eq!(correctness.confidence, 0.99);
        assert!(correctness.evidence.len() <= MAX_EVIDENCE_PER_FINDING);
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
    fn tool_review_merge_unions_sources_and_marks_corroborated_tier() {
        let mut first = finding(
            "src/lib.rs",
            10,
            12,
            "correctness",
            "The branch drops errors.",
            VerificationStatus::Verified,
            ReviewSeverity::High,
            0.7,
        );
        first.sources = vec!["l1_diff@thinking".to_string()];
        let mut second = finding(
            "src/lib.rs",
            11,
            12,
            "correctness",
            "Errors are swallowed here.",
            VerificationStatus::Unverified,
            ReviewSeverity::High,
            0.6,
        );
        second.sources = vec!["l1_diff@chat2".to_string()];
        let mut third = finding(
            "src/other.rs",
            1,
            2,
            "correctness",
            "Single-source claim.",
            VerificationStatus::Verified,
            ReviewSeverity::Low,
            0.9,
        );
        third.sources = vec!["a1_repo_context@chat".to_string()];
        let mut report = report(vec![first, second, third]);

        finalize_review_report(&mut report, &RiskEnrichment::default());

        assert_eq!(report.findings.len(), 2);
        let merged = &report.findings[0];
        assert_eq!(merged.rank_tier, RankTier::Corroborated);
        assert_eq!(
            merged.sources,
            vec!["l1_diff@thinking".to_string(), "l1_diff@chat2".to_string()]
        );
        assert_eq!(merged.verification_status, VerificationStatus::Verified);
        let single = &report.findings[1];
        assert_eq!(single.rank_tier, RankTier::Verified);
        assert_eq!(single.sources, vec!["a1_repo_context@chat".to_string()]);
    }

    #[test]
    fn tool_review_merge_execution_evidence_wins_the_top_tier() {
        let mut reproduced = finding(
            "src/lib.rs",
            50,
            52,
            "correctness",
            "Reproduced by a failing script.",
            VerificationStatus::Unverified,
            ReviewSeverity::Medium,
            0.5,
        );
        reproduced.evidence.push(ReviewEvidence {
            kind: evidence_kinds::MUTATION_PROBE.to_string(),
            path: Some("src/lib.rs".to_string()),
            line1: Some(50),
            line2: Some(52),
            content: "mutated line 51, covering test stayed green".to_string(),
        });
        let corroborated_a = {
            let mut f = finding(
                "src/lib.rs",
                1,
                2,
                "security",
                "Corroborated claim.",
                VerificationStatus::Verified,
                ReviewSeverity::Critical,
                0.9,
            );
            f.sources = vec!["l1_diff@chat".to_string()];
            f
        };
        let corroborated_b = {
            let mut f = finding(
                "src/lib.rs",
                1,
                2,
                "security",
                "Corroborated claim again.",
                VerificationStatus::Verified,
                ReviewSeverity::Critical,
                0.9,
            );
            f.sources = vec!["s1_security".to_string()];
            f
        };
        let mut report = report(vec![reproduced, corroborated_a, corroborated_b]);

        finalize_review_report(&mut report, &RiskEnrichment::default());

        assert_eq!(report.findings.len(), 2);
        assert_eq!(report.findings[0].rank_tier, RankTier::ExecutionReproduced);
        assert_eq!(report.findings[0].severity, ReviewSeverity::Medium);
        assert_eq!(report.findings[1].rank_tier, RankTier::Corroborated);
    }

    #[test]
    fn tool_review_merge_hot_file_bumps_severity_once_and_caps_at_critical() {
        let mut enrichment = RiskEnrichment::default();
        enrichment.insert(
            "src/lib.rs",
            FileRiskFacts {
                churn_percentile: 0.95,
                temporal_hotspot: true,
                fan_in: 20,
                bus_factor_risk: false,
            },
        );
        let mut high = finding(
            "src/lib.rs",
            10,
            12,
            "correctness",
            "Hot file claim.",
            VerificationStatus::Verified,
            ReviewSeverity::High,
            0.8,
        );
        high.sources = vec!["l1_diff@chat".to_string()];
        let mut critical = finding(
            "src/lib.rs",
            100,
            101,
            "security",
            "Already critical.",
            VerificationStatus::Verified,
            ReviewSeverity::Critical,
            0.9,
        );
        critical.sources = vec!["s1_security".to_string()];
        let mut report = report(vec![high, critical]);

        finalize_review_report(&mut report, &enrichment);

        let bumped = report
            .findings
            .iter()
            .find(|finding| finding.category == "correctness")
            .unwrap();
        assert_eq!(bumped.severity, ReviewSeverity::Critical);
        assert!(bumped
            .checks_performed
            .contains(&"severity_bumped:hot_path".to_string()));
        assert!(bumped
            .evidence
            .iter()
            .any(|evidence| evidence.kind == evidence_kinds::STATIC_FACT));
        let capped = report
            .findings
            .iter()
            .find(|finding| finding.category == "security")
            .unwrap();
        assert_eq!(capped.severity, ReviewSeverity::Critical);
        assert!(!capped
            .checks_performed
            .contains(&"severity_bumped:hot_path".to_string()));
    }

    #[test]
    fn tool_review_rank_orders_tier_then_severity_then_confidence() {
        let mut findings = vec![];
        let mut make = |claim: &str,
                        status: VerificationStatus,
                        severity: ReviewSeverity,
                        confidence: f32,
                        sources: Vec<&str>,
                        exec: bool,
                        index: u32| {
            let mut f = finding(
                &format!("src/file{index}.rs"),
                1,
                1,
                "correctness",
                claim,
                status,
                severity,
                confidence,
            );
            f.sources = sources.into_iter().map(str::to_string).collect();
            if exec {
                f.evidence.push(ReviewEvidence {
                    kind: evidence_kinds::EXECUTION_OUTPUT.to_string(),
                    path: None,
                    line1: None,
                    line2: None,
                    content: "repro failed as claimed".to_string(),
                });
            }
            f
        };
        findings.push(make(
            "verified-low",
            VerificationStatus::Verified,
            ReviewSeverity::Low,
            0.1,
            vec!["l1"],
            false,
            0,
        ));
        findings.push(make(
            "needs-human",
            VerificationStatus::NeedsHumanValidation,
            ReviewSeverity::Critical,
            1.0,
            vec!["l1"],
            false,
            1,
        ));
        findings.push(make(
            "executed",
            VerificationStatus::Unverified,
            ReviewSeverity::Low,
            0.2,
            vec!["a3_execution"],
            true,
            2,
        ));
        findings.push(make(
            "corroborated",
            VerificationStatus::Unverified,
            ReviewSeverity::Medium,
            0.4,
            vec!["l1@chat", "l2@thinking"],
            false,
            3,
        ));
        findings.push(make(
            "downgraded",
            VerificationStatus::Downgraded,
            ReviewSeverity::Critical,
            1.0,
            vec!["l1"],
            false,
            4,
        ));
        findings.push(make(
            "verified-critical",
            VerificationStatus::Verified,
            ReviewSeverity::Critical,
            0.9,
            vec!["l1"],
            false,
            5,
        ));
        let mut report = report(findings);

        finalize_review_report(&mut report, &RiskEnrichment::default());

        assert_eq!(
            report
                .findings
                .iter()
                .map(|finding| finding.claim.as_str())
                .collect::<Vec<_>>(),
            vec![
                "executed",
                "corroborated",
                "verified-critical",
                "verified-low",
                "needs-human",
                "downgraded",
            ]
        );
    }

    #[test]
    fn tool_review_median_severity_is_lower_median() {
        assert_eq!(
            median_severity(vec![ReviewSeverity::Low, ReviewSeverity::Critical]),
            ReviewSeverity::Low
        );
        assert_eq!(
            median_severity(vec![
                ReviewSeverity::Low,
                ReviewSeverity::High,
                ReviewSeverity::Critical
            ]),
            ReviewSeverity::High
        );
        assert_eq!(
            median_severity(vec![ReviewSeverity::Medium]),
            ReviewSeverity::Medium
        );
    }

    #[test]
    fn tool_review_clean_summary_is_never_empty() {
        let mut report = report(vec![]);
        report.scope.files_reviewed.push("src/main.rs".to_string());
        report.checks_performed = vec![
            "candidate_scan".to_string(),
            "verifier_rejected:1".to_string(),
        ];

        finalize_review_report(&mut report, &RiskEnrichment::default());

        assert_eq!(
            report.summary,
            "Reviewed 2 files. Checks performed: candidate_scan, verifier_rejected:1. No verified findings."
        );
    }
}
