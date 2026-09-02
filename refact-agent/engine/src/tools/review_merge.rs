use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::tools::review_scope::{normalize_path, paths_match};
use crate::tools::review_types::{FindingLocation, ReviewFinding};

const NEAR_RANGE_LINES: u32 = 5;
const CLAIM_SIMILARITY_THRESHOLD: f64 = 0.5;

const CLAIM_STOPWORDS: &[&str] = &[
    "the", "this", "that", "these", "those", "and", "but", "for", "not", "are", "was", "were",
    "with", "from", "into", "when", "which", "while", "have", "has", "had", "its", "it's", "can",
    "will", "should", "would", "may", "might", "does", "did", "here", "there", "than", "then",
    "also", "only", "any", "all", "some", "because", "code", "issue", "problem", "bug",
];

pub fn significant_tokens(claim: &str) -> BTreeSet<String> {
    claim
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .map(|token| token.to_ascii_lowercase())
        .filter(|token| token.len() > 2 && !CLAIM_STOPWORDS.contains(&token.as_str()))
        .collect()
}

pub fn claims_are_equivalent(left: &str, right: &str) -> bool {
    let left = significant_tokens(left);
    let right = significant_tokens(right);
    let overlap = left.intersection(&right).count();
    if overlap < 2 {
        return false;
    }
    let smaller = left.len().min(right.len());
    smaller > 0 && (overlap as f64 / smaller as f64) >= CLAIM_SIMILARITY_THRESHOLD
}

fn ranges_are_near(left: &ReviewFinding, right: &ReviewFinding) -> bool {
    left.line_start <= right.line_end.saturating_add(NEAR_RANGE_LINES)
        && right.line_start <= left.line_end.saturating_add(NEAR_RANGE_LINES)
}

pub fn are_duplicates(left: &ReviewFinding, right: &ReviewFinding) -> bool {
    if !paths_match(&left.file, &right.file) {
        return false;
    }
    ranges_are_near(left, right) || claims_are_equivalent(&left.claim, &right.claim)
}

pub fn stable_finding_id(finding: &ReviewFinding) -> String {
    let tokens = significant_tokens(&finding.claim)
        .into_iter()
        .take(6)
        .collect::<Vec<_>>()
        .join("-");
    let input = format!(
        "{}|{}|{}",
        normalize_path(&finding.file),
        finding.line_start / 50,
        tokens
    );
    let digest = hex::encode(Sha256::digest(input.as_bytes()));
    format!("rf-{}", &digest[..8])
}

fn support_rank(finding: &ReviewFinding) -> u8 {
    match (finding.has_reproduction(), finding.evidence_present) {
        (true, true) => 3,
        (true, false) => 2,
        (false, true) => 1,
        (false, false) => 0,
    }
}

fn absorb(survivor: &mut ReviewFinding, other: ReviewFinding) {
    if other.severity.rank() > survivor.severity.rank() {
        survivor.severity = other.severity;
    }
    if survivor.reproduction.is_none() && other.reproduction.is_some() {
        survivor.reproduction = other.reproduction.clone();
    }
    if survivor.fix.is_none() && other.fix.is_some() {
        survivor.fix = other.fix.clone();
    }
    survivor.evidence_present |= other.evidence_present;
    survivor.introduced_by_diff |= other.introduced_by_diff;
    survivor.out_of_scope &= other.out_of_scope;
    for stage in other.reported_by.iter().chain(std::iter::once(&other.stage)) {
        if !survivor.reported_by.contains(stage) {
            survivor.reported_by.push(stage.clone());
        }
    }
    let location = FindingLocation {
        file: other.file.clone(),
        line_start: other.line_start,
        line_end: other.line_end,
    };
    let same_as_survivor = paths_match(&location.file, &survivor.file)
        && location.line_start == survivor.line_start
        && location.line_end == survivor.line_end;
    if !same_as_survivor && !survivor.locations.contains(&location) {
        survivor.locations.push(location);
    }
    for location in other.locations {
        if !survivor.locations.contains(&location) {
            survivor.locations.push(location);
        }
    }
}

pub fn merge_findings(findings: Vec<ReviewFinding>) -> (Vec<ReviewFinding>, usize) {
    let mut survivors: Vec<ReviewFinding> = Vec::with_capacity(findings.len());
    let mut merged = 0;
    for mut finding in findings {
        if finding.reported_by.is_empty() {
            finding.reported_by.push(finding.stage.clone());
        }
        match survivors
            .iter_mut()
            .find(|survivor| are_duplicates(survivor, &finding))
        {
            Some(survivor) => {
                merged += 1;
                if support_rank(&finding) > support_rank(survivor) {
                    let previous = std::mem::replace(survivor, finding);
                    absorb(survivor, previous);
                } else {
                    absorb(survivor, finding);
                }
            }
            None => survivors.push(finding),
        }
    }
    for finding in &mut survivors {
        finding.reported_by.sort();
        finding.reported_by.dedup();
        finding.id = stable_finding_id(finding);
    }
    (survivors, merged)
}

pub fn rank_findings(findings: &mut [ReviewFinding]) {
    findings.sort_by(|left, right| {
        support_rank(right)
            .cmp(&support_rank(left))
            .then_with(|| right.severity.rank().cmp(&left.severity.rank()))
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.line_start.cmp(&right.line_start))
            .then_with(|| left.line_end.cmp(&right.line_end))
            .then_with(|| left.claim.cmp(&right.claim))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::review_types::ReviewSeverity;

    fn finding(stage: &str, file: &str, line_start: u32, claim: &str) -> ReviewFinding {
        ReviewFinding {
            id: String::new(),
            stage: stage.to_string(),
            model: Some(format!("{stage}-model")),
            title: claim.to_string(),
            severity: ReviewSeverity::Medium,
            file: file.to_string(),
            line_start,
            line_end: line_start + 2,
            claim: claim.to_string(),
            evidence: "let value = 1;".to_string(),
            evidence_present: false,
            reproduction: None,
            fix: None,
            introduced_by_diff: true,
            out_of_scope: false,
            reported_by: vec![],
            locations: vec![],
            disputed: None,
        }
    }

    #[test]
    fn review_merge_collapses_two_stages_and_keeps_both_locations() {
        let mut first = finding("diff", "src/lib.rs", 10, "The error arm returns success");
        first.evidence_present = true;
        let mut second = finding("impact", "src/lib.rs", 40, "the error arm returns success again");
        second.reproduction = Some("cargo test -p thing".to_string());
        second.severity = ReviewSeverity::Blocker;
        second.fix = Some("propagate".to_string());

        let (merged, count) = merge_findings(vec![first, second]);

        assert_eq!(count, 1);
        assert_eq!(merged.len(), 1);
        let survivor = &merged[0];
        assert_eq!(survivor.stage, "impact");
        assert_eq!(survivor.reported_by, ["diff", "impact"]);
        assert_eq!(survivor.severity, ReviewSeverity::Blocker);
        assert!(survivor.evidence_present);
        assert_eq!(survivor.locations.len(), 1);
        assert_eq!(survivor.locations[0].line_start, 10);
        assert!(survivor.id.starts_with("rf-"));
    }

    #[test]
    fn review_merge_keeps_distinct_files_and_distinct_claims_apart() {
        let findings = vec![
            finding("diff", "src/a.rs", 10, "The lock is taken twice"),
            finding("diff", "src/b.rs", 10, "The lock is taken twice"),
            finding("security", "src/a.rs", 400, "Secrets are logged in plain text"),
        ];

        let (merged, count) = merge_findings(findings);

        assert_eq!(count, 0);
        assert_eq!(merged.len(), 3);
    }

    #[test]
    fn review_merge_ranking_puts_reproduced_first_then_evidence_then_severity() {
        let mut reproduced = finding("tests", "src/z.rs", 1, "reproduced claim");
        reproduced.reproduction = Some("cargo test".to_string());
        reproduced.severity = ReviewSeverity::Low;
        let mut evidenced = finding("diff", "src/a.rs", 1, "evidenced claim");
        evidenced.evidence_present = true;
        evidenced.severity = ReviewSeverity::Medium;
        let mut bare_blocker = finding("spec", "src/b.rs", 1, "bare blocker claim");
        bare_blocker.severity = ReviewSeverity::Blocker;
        let mut findings = vec![bare_blocker, evidenced, reproduced];

        rank_findings(&mut findings);

        assert_eq!(findings[0].stage, "tests");
        assert_eq!(findings[1].stage, "diff");
        assert_eq!(findings[2].stage, "spec");
    }

    #[test]
    fn review_merge_ids_are_stable_across_runs_and_line_drift() {
        let first = finding("diff", "src/lib.rs", 10, "The error arm returns success");
        let mut drifted = first.clone();
        drifted.line_start = 40;
        drifted.line_end = 42;
        let other = finding("diff", "src/lib.rs", 10, "A totally different concern entirely");

        assert_eq!(stable_finding_id(&first), stable_finding_id(&drifted));
        assert_ne!(stable_finding_id(&first), stable_finding_id(&other));
    }

    #[test]
    fn review_merge_claim_equivalence_needs_real_overlap() {
        assert!(claims_are_equivalent(
            "The retry loop never resets the counter",
            "retry loop fails to reset counter"
        ));
        assert!(!claims_are_equivalent(
            "The retry loop never resets the counter",
            "The parser accepts empty input"
        ));
    }
}
