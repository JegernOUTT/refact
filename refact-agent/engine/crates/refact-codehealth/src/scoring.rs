use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::biomarkers::{Dimension, Finding, Severity};

fn severity_deduction(s: Severity) -> f64 {
    match s {
        Severity::Info | Severity::Low => 0.3,
        Severity::Medium => 0.7,
        Severity::High => 1.2,
        Severity::Critical => 2.0,
    }
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
        "developer_congestion"
        | "low_cohesion"
        | "brain_method"
        | "bumpy_road"
        | "primitive_obsession"
        | "dry_violation" => 0.5,
        "knowledge_loss" => 0.4,
        _ => 1.0,
    }
}

/// Max total deduction allowed per category (RepoWise CATEGORY_CAPS, abs value).
/// The default cap is 1.0 (matching scoring.py `caps.get(cat, 1.0)`).
fn category_cap(category: &str) -> f64 {
    match category {
        "organizational" => 3.5,
        "structural_complexity" => 2.5,
        "test_coverage" => 2.0,
        "size_and_complexity" => 1.5,
        "duplication" => 1.0,
        "error_handling" => 0.5,
        _ => 1.0,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileScore {
    pub defect: f64,
    pub maintainability: f64,
    pub performance: f64,
}

fn score_dimension(findings: &[&Finding]) -> f64 {
    let mut per_cat: HashMap<&str, f64> = HashMap::new();
    for f in findings {
        let d = severity_deduction(f.severity) * weight_multiplier(&f.biomarker);
        *per_cat.entry(f.category.as_str()).or_insert(0.0) += d;
    }
    let total: f64 = per_cat
        .iter()
        .map(|(cat, ded)| ded.min(category_cap(cat)))
        .sum();
    (10.0 - total).clamp(1.0, 10.0)
}

/// Three orthogonal signals (RepoWise model). Defect uses the universal pool;
/// maintainability and performance use their home-dimension findings.
pub fn score_file(findings: &[Finding]) -> FileScore {
    let all: Vec<&Finding> = findings.iter().collect();
    let maint: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.dimension == Dimension::Maintainability)
        .collect();
    let perf: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.dimension == Dimension::Performance)
        .collect();
    FileScore {
        defect: score_dimension(&all),
        maintainability: score_dimension(&maint),
        performance: score_dimension(&perf),
    }
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
        }
    }

    #[test]
    fn clean_file_scores_ten() {
        let score = score_file(&[]);
        assert_eq!(score.defect, 10.0);
        assert_eq!(grade(score.defect), 'A');
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
        // Many error_handling findings can't deduct more than 0.5 total.
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
        assert!(
            (score.defect - 9.5).abs() < 1e-9,
            "error_handling cap 0.5 => 9.5, got {}",
            score.defect
        );
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
