use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::biomarkers::{Dimension, Finding, Severity};

fn severity_deduction(s: Severity) -> f64 {
    match s {
        Severity::Low => 0.3,
        Severity::Medium => 0.7,
        Severity::High => 1.2,
        Severity::Critical => 2.0,
    }
}

fn finding_base_deduction(finding: &Finding) -> f64 {
    finding
        .deduction
        .unwrap_or_else(|| severity_deduction(finding.severity))
        .max(0.0)
}

/// RepoWise's offline-calibrated per-biomarker defect-weight multipliers
/// (logistic regression over a 13-repo defect corpus). Default 1.0.
fn weight_multiplier(biomarker: &str) -> f64 {
    match biomarker {
        "co_change_scatter" => 1.8,
        "change_entropy" => 1.51,
        "ownership_risk" => 1.38,
        "nested_complexity" => 1.34,
        "complex_conditional" => 1.33,
        "large_method" => 1.25,
        "complex_method" => 1.21,
        "function_hotspot" => 1.16,
        "god_class" => 1.13,
        "untested_hotspot" => 1.3,
        "churn_risk" => 1.2,
        "code_age_volatility" => 1.1,
        "developer_congestion"
        | "low_cohesion"
        | "brain_method"
        | "bumpy_road"
        | "primitive_obsession"
        | "dry_violation"
        | "error_handling" => 0.5,
        "blocking_sync_in_async" => 0.7,
        "knowledge_loss" => 0.4,
        _ => 1.0,
    }
}

fn defect_category_cap(category: &str) -> f64 {
    match category {
        // RepoWise packages/core/src/repowise/core/analysis/health/scoring.py CATEGORY_CAPS.
        "organizational" => 3.5,
        "structural_complexity" => 2.5,
        "test_coverage" => 2.0,
        "coverage_gradient" => 2.0,
        "size_and_complexity" => 1.5,
        "duplication" => 1.0,
        "performance" => 1.0,
        "test_quality" => 0.5,
        "error_handling" => 0.5,
        _ => 1.0,
    }
}

fn maintainability_category_cap(category: &str) -> f64 {
    match category {
        // RepoWise packages/core/src/repowise/core/analysis/health/scoring.py _MAINTAINABILITY_CATEGORY_CAPS.
        "structural_complexity" => 4.0,
        "size_and_complexity" => 2.0,
        "duplication" => 2.0,
        "error_handling" => 2.0,
        _ => 1.0,
    }
}

fn performance_category_cap(category: &str) -> f64 {
    match category {
        // RepoWise packages/core/src/repowise/core/analysis/health/scoring.py _PERFORMANCE_CATEGORY_CAPS.
        "performance" => 1.0,
        _ => 1.0,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileScore {
    pub defect: f64,
    pub maintainability: f64,
    pub performance: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindingImpact {
    pub index: usize,
    pub biomarker: String,
    pub deduction: f64,
    pub capped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScoreTable {
    Defect,
    Maintainability,
    Performance,
}

fn category_cap(table: ScoreTable, category: &str) -> f64 {
    match table {
        ScoreTable::Defect => defect_category_cap(category),
        ScoreTable::Maintainability => maintainability_category_cap(category),
        ScoreTable::Performance => performance_category_cap(category),
    }
}

fn score_dimension(
    findings: &[(usize, &Finding)],
    table: ScoreTable,
    apply_defect_weights: bool,
) -> (f64, Vec<FindingImpact>) {
    let mut per_cat: BTreeMap<&str, Vec<(usize, &str, f64)>> = BTreeMap::new();
    for (index, finding) in findings {
        let multiplier = if apply_defect_weights {
            weight_multiplier(&finding.biomarker)
        } else {
            1.0
        };
        let deduction = finding_base_deduction(finding) * multiplier;
        per_cat.entry(finding.category.as_str()).or_default().push((
            *index,
            finding.biomarker.as_str(),
            deduction,
        ));
    }

    let mut total = 0.0;
    let mut impacts = Vec::new();
    for (category, entries) in per_cat {
        let raw_total = entries
            .iter()
            .map(|(_, _, deduction)| *deduction)
            .sum::<f64>();
        let cap = category_cap(table, category);
        let capped = raw_total > cap;
        let scale = if capped && raw_total > 0.0 {
            cap / raw_total
        } else {
            1.0
        };
        total += if capped { cap } else { raw_total };
        for (index, biomarker, deduction) in entries {
            impacts.push(FindingImpact {
                index,
                biomarker: biomarker.to_string(),
                deduction: deduction * scale,
                capped,
            });
        }
    }
    impacts.sort_by(|a, b| a.index.cmp(&b.index).then(a.biomarker.cmp(&b.biomarker)));
    ((10.0 - total).clamp(1.0, 10.0), impacts)
}

/// Three orthogonal signals (RepoWise model). Defect uses the universal pool;
/// maintainability and performance use their home-dimension findings.
pub fn score_file_with_impacts(findings: &[Finding]) -> (FileScore, Vec<FindingImpact>) {
    let all: Vec<(usize, &Finding)> = findings.iter().enumerate().collect();
    let maint: Vec<(usize, &Finding)> = findings
        .iter()
        .enumerate()
        .filter(|(_, f)| f.dimension == Dimension::Maintainability)
        .collect();
    let perf: Vec<(usize, &Finding)> = findings
        .iter()
        .enumerate()
        .filter(|(_, f)| f.dimension == Dimension::Performance)
        .collect();
    let (defect, impacts) = score_dimension(&all, ScoreTable::Defect, true);
    let (maintainability, _) = score_dimension(&maint, ScoreTable::Maintainability, false);
    let (performance, _) = score_dimension(&perf, ScoreTable::Performance, false);
    (
        FileScore {
            defect,
            maintainability,
            performance,
        },
        impacts,
    )
}

pub fn score_file(findings: &[Finding]) -> FileScore {
    score_file_with_impacts(findings).0
}

pub fn grade(score: f64) -> char {
    if score >= 9.0 {
        'A'
    } else if score >= 7.5 {
        'B'
    } else if score >= 6.0 {
        'C'
    } else if score >= 4.0 {
        'D'
    } else {
        'F'
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biomarkers::{Dimension, Finding, Severity};

    fn finding(biomarker: &str, category: &str, dim: Dimension, sev: Severity) -> Finding {
        Finding {
            biomarker: biomarker.to_string(),
            category: category.to_string(),
            dimension: dim,
            severity: sev,
            line: 1,
            detail: String::new(),
            deduction: None,
        }
    }

    fn finding_with_deduction(
        biomarker: &str,
        category: &str,
        dim: Dimension,
        sev: Severity,
        deduction: f64,
    ) -> Finding {
        Finding {
            deduction: Some(deduction),
            ..finding(biomarker, category, dim, sev)
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn clean_file_scores_ten() {
        let score = score_file(&[]);
        assert_eq!(score.defect, 10.0);
        assert_eq!(grade(score.defect), 'A');
    }

    #[test]
    fn score_file_regression_pinned() {
        let structural = vec![
            finding(
                "complex_method",
                "size_and_complexity",
                Dimension::Defect,
                Severity::High,
            ),
            finding(
                "god_class",
                "structural_complexity",
                Dimension::Maintainability,
                Severity::High,
            ),
        ];
        let structural_score = score_file(&structural);
        assert_close(structural_score.defect, 7.192);
        assert_close(structural_score.maintainability, 8.8);
        assert_close(structural_score.performance, 10.0);

        let perf = vec![finding(
            "io_in_loop",
            "performance",
            Dimension::Performance,
            Severity::High,
        )];
        let perf_score = score_file(&perf);
        assert_close(perf_score.defect, 9.0);
        assert_close(perf_score.maintainability, 10.0);
        assert_close(perf_score.performance, 9.0);
    }

    #[test]
    fn structural_findings_lower_defect_score() {
        let findings = vec![
            finding(
                "complex_method",
                "size_and_complexity",
                Dimension::Defect,
                Severity::High,
            ),
            finding(
                "god_class",
                "structural_complexity",
                Dimension::Maintainability,
                Severity::High,
            ),
        ];
        let score = score_file(&findings);
        assert!(score.defect < 10.0, "got {}", score.defect);
        assert!(
            score.maintainability < 10.0,
            "got {}",
            score.maintainability
        );
    }

    #[test]
    fn category_cap_limits_deduction() {
        let findings: Vec<Finding> = (0..20)
            .map(|_| {
                finding(
                    "error_handling",
                    "error_handling",
                    Dimension::Defect,
                    Severity::Low,
                )
            })
            .collect();
        let score = score_file(&findings);
        assert_close(score.defect, 9.5);
    }

    #[test]
    fn test_quality_capped_at_half_point() {
        let findings: Vec<Finding> = (0..4)
            .map(|_| {
                finding(
                    "large_assertion_block",
                    "test_quality",
                    Dimension::Defect,
                    Severity::Medium,
                )
            })
            .collect();
        let score = score_file(&findings);
        assert_close(score.defect, 9.5);
    }

    #[test]
    fn maintainability_uses_own_caps() {
        let findings: Vec<Finding> = (0..3)
            .map(|_| {
                finding(
                    "low_cohesion",
                    "structural_complexity",
                    Dimension::Maintainability,
                    Severity::Critical,
                )
            })
            .collect();
        let score = score_file(&findings);
        assert_close(score.defect, 7.5);
        assert_close(score.maintainability, 6.0);
    }

    #[test]
    fn coverage_gradient_continuous() {
        let half_uncovered = vec![finding_with_deduction(
            "coverage_gradient",
            "coverage_gradient",
            Dimension::Defect,
            Severity::Medium,
            2.0,
        )];
        let (score, impacts) = score_file_with_impacts(&half_uncovered);
        assert_close(score.defect, 8.0);
        assert_close(impacts[0].deduction, 2.0);
        assert!(!impacts[0].capped);

        let fully_uncovered = vec![finding_with_deduction(
            "coverage_gradient",
            "coverage_gradient",
            Dimension::Defect,
            Severity::High,
            4.0,
        )];
        let (score, impacts) = score_file_with_impacts(&fully_uncovered);
        assert_close(score.defect, 8.0);
        assert_close(impacts[0].deduction, 2.0);
        assert!(impacts[0].capped);
    }

    #[test]
    fn impacts_sum_to_deduction() {
        let findings = vec![
            finding(
                "complex_method",
                "size_and_complexity",
                Dimension::Defect,
                Severity::High,
            ),
            finding(
                "god_class",
                "structural_complexity",
                Dimension::Maintainability,
                Severity::High,
            ),
            finding(
                "io_in_loop",
                "performance",
                Dimension::Performance,
                Severity::High,
            ),
        ];
        let (score, impacts) = score_file_with_impacts(&findings);
        let impact_sum = impacts.iter().map(|impact| impact.deduction).sum::<f64>();
        assert_close(impact_sum, 10.0 - score.defect);
        assert_eq!(
            impacts
                .iter()
                .map(|impact| impact.index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn capped_category_proportional_attribution() {
        let findings = vec![
            finding(
                "large_assertion_block",
                "test_quality",
                Dimension::Defect,
                Severity::Medium,
            ),
            finding(
                "duplicated_assertion_block",
                "test_quality",
                Dimension::Defect,
                Severity::Medium,
            ),
        ];
        let (score, impacts) = score_file_with_impacts(&findings);
        assert_close(score.defect, 9.5);
        assert_close(impacts[0].deduction, 0.25);
        assert_close(impacts[1].deduction, 0.25);
        assert!(impacts.iter().all(|impact| impact.capped));
    }

    #[test]
    fn defect_weights_do_not_apply_to_home_dimensions() {
        let findings = vec![finding(
            "low_cohesion",
            "structural_complexity",
            Dimension::Maintainability,
            Severity::High,
        )];

        let score = score_file(&findings);

        assert_close(score.defect, 9.4);
        assert_close(score.maintainability, 8.8);
    }

    #[test]
    fn grade_thresholds() {
        assert_eq!(grade(9.5), 'A');
        assert_eq!(grade(8.0), 'B');
        assert_eq!(grade(6.5), 'C');
        assert_eq!(grade(4.5), 'D');
        assert_eq!(grade(2.0), 'F');
    }
}
