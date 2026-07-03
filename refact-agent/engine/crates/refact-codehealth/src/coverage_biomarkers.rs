use crate::biomarkers::{Dimension, Finding, Severity};
use crate::coverage::CoverageReport;
use std::collections::HashMap;

const TEST_COVERAGE_CATEGORY: &str = "test_coverage";
const COVERAGE_GRADIENT_CATEGORY: &str = "coverage_gradient";
const COVERAGE_GRADIENT_WEIGHT: f64 = 4.0;

pub fn coverage_biomarkers(
    coverage: &CoverageReport,
    complexity_by_file: &HashMap<String, u32>,
    churn_by_file: &HashMap<String, u32>,
) -> Vec<Finding> {
    let mut out = Vec::new();

    for file in &coverage.files {
        let line_fraction = pct_fraction(file.lines_covered, file.lines_total);
        let branch_fraction = pct_fraction(file.branches_covered, file.branches_total);

        if file.lines_total > 0 && line_fraction < 1.0 {
            let uncovered_fraction = 1.0 - line_fraction;
            let severity = if line_fraction < 0.4 {
                Severity::High
            } else if line_fraction < 0.7 {
                Severity::Medium
            } else {
                Severity::Low
            };
            out.push(finding_with_deduction(
                "coverage_gradient",
                COVERAGE_GRADIENT_CATEGORY,
                severity,
                format!(
                    "{} has {:.0}% uncovered lines ({:.0}% line coverage); uncovered code carries proportionally more defect risk",
                    file.path,
                    uncovered_fraction * 100.0,
                    file.line_pct()
                ),
                COVERAGE_GRADIENT_WEIGHT * uncovered_fraction,
            ));
        }

        if line_fraction < 0.5 && file.lines_total >= 20 {
            let severity = if line_fraction < 0.2 {
                Severity::High
            } else if line_fraction < 0.35 {
                Severity::Medium
            } else {
                Severity::Low
            };
            out.push(finding(
                "coverage_gap",
                TEST_COVERAGE_CATEGORY,
                severity,
                format!(
                    "{} has {:.0}% line coverage ({}/{})",
                    file.path,
                    file.line_pct(),
                    file.lines_covered,
                    file.lines_total
                ),
            ));
        }

        if file.branches_total > 0 && line_fraction - branch_fraction > 0.3 {
            out.push(finding(
                "branch_coverage_lag",
                TEST_COVERAGE_CATEGORY,
                Severity::Medium,
                format!(
                    "{} has {:.0}% branch coverage vs {:.0}% line coverage; branches under-tested vs lines",
                    file.path,
                    file.branch_pct(),
                    file.line_pct()
                ),
            ));
        }

        let complexity = complexity_by_file.get(&file.path).copied().unwrap_or(0);
        let churn = churn_by_file.get(&file.path).copied().unwrap_or(0);
        if line_fraction < 0.5 && (complexity >= 10 || churn >= 5) {
            out.push(finding(
                "untested_hotspot",
                TEST_COVERAGE_CATEGORY,
                Severity::High,
                format!(
                    "{} has {:.0}% line coverage with complexity {} and churn {}",
                    file.path,
                    file.line_pct(),
                    complexity,
                    churn
                ),
            ));
        }
    }

    out.sort_by(|a, b| a.detail.cmp(&b.detail).then(a.biomarker.cmp(&b.biomarker)));
    out
}

pub fn coverage_summary(coverage: &CoverageReport) -> (f64, f64, usize) {
    let applicable_files = coverage.files.iter().filter(|f| f.lines_total > 0);
    let lines_total: u32 = applicable_files.clone().map(|f| f.lines_total).sum();
    let lines_covered: u32 = applicable_files.clone().map(|f| f.lines_covered).sum();
    let branches_total: u32 = applicable_files.clone().map(|f| f.branches_total).sum();
    let branches_covered: u32 = applicable_files.clone().map(|f| f.branches_covered).sum();
    let files_below_50 = coverage
        .files
        .iter()
        .filter(|f| f.lines_total > 0 && pct_fraction(f.lines_covered, f.lines_total) < 0.5)
        .count();

    (
        pct_percent(lines_covered, lines_total),
        pct_percent(branches_covered, branches_total),
        files_below_50,
    )
}

fn finding(biomarker: &str, category: &str, severity: Severity, detail: String) -> Finding {
    Finding {
        biomarker: biomarker.to_string(),
        category: category.to_string(),
        dimension: Dimension::Defect,
        severity,
        line: 1,
        detail,
        deduction: None,
    }
}

fn finding_with_deduction(
    biomarker: &str,
    category: &str,
    severity: Severity,
    detail: String,
    deduction: f64,
) -> Finding {
    Finding {
        deduction: Some(deduction),
        ..finding(biomarker, category, severity, detail)
    }
}

fn pct_fraction(covered: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        covered as f64 / total as f64
    }
}

fn pct_percent(covered: u32, total: u32) -> f64 {
    pct_fraction(covered, total) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::FileCoverage;

    fn report() -> CoverageReport {
        CoverageReport {
            format: "test".to_string(),
            files: vec![
                FileCoverage {
                    path: "src/gap.rs".to_string(),
                    lines_total: 100,
                    lines_covered: 30,
                    branches_total: 0,
                    branches_covered: 0,
                },
                FileCoverage {
                    path: "src/gradient.rs".to_string(),
                    lines_total: 100,
                    lines_covered: 90,
                    branches_total: 10,
                    branches_covered: 4,
                },
                FileCoverage {
                    path: "src/hotspot.rs".to_string(),
                    lines_total: 50,
                    lines_covered: 10,
                    branches_total: 0,
                    branches_covered: 0,
                },
                FileCoverage {
                    path: "src/small.rs".to_string(),
                    lines_total: 5,
                    lines_covered: 5,
                    branches_total: 2,
                    branches_covered: 2,
                },
            ],
        }
    }

    #[test]
    fn detects_coverage_biomarkers() {
        let coverage = report();
        let mut complexity = HashMap::new();
        complexity.insert("src/hotspot.rs".to_string(), 15);
        let churn = HashMap::new();

        let findings = coverage_biomarkers(&coverage, &complexity, &churn);

        assert!(findings.iter().any(|f| {
            f.biomarker == "coverage_gap"
                && f.severity == Severity::Medium
                && f.detail.contains("30%")
        }));
        assert!(findings.iter().any(|f| {
            f.biomarker == "branch_coverage_lag"
                && f.severity == Severity::Medium
                && f.detail.contains("40% branch")
        }));
        assert!(findings.iter().any(|f| {
            f.biomarker == "coverage_gradient"
                && f.category == "coverage_gradient"
                && f.severity == Severity::Low
                && f.deduction
                    .is_some_and(|deduction| (deduction - 0.4).abs() < 1e-9)
                && f.detail.contains("10% uncovered")
        }));
        assert!(findings.iter().any(|f| {
            f.biomarker == "untested_hotspot"
                && f.severity == Severity::High
                && f.detail.contains("complexity 15")
        }));
        assert!(!findings.iter().any(|f| f.detail.contains("src/small.rs")));
    }

    #[test]
    fn summarizes_coverage() {
        let coverage = report();
        let (line_pct, branch_pct, below_50) = coverage_summary(&coverage);

        assert!((line_pct - 52.941_176_470_588_24).abs() < 1e-6);
        assert!((branch_pct - 50.0).abs() < f64::EPSILON);
        assert_eq!(below_50, 2);
    }

    #[test]
    fn summary_excludes_zero_line_files_from_below_50() {
        let mut coverage = report();
        coverage.files.push(FileCoverage {
            path: "src/empty.rs".to_string(),
            lines_total: 0,
            lines_covered: 0,
            branches_total: 100,
            branches_covered: 0,
        });

        let (line_pct, branch_pct, below_50) = coverage_summary(&coverage);

        assert!((line_pct - 52.941_176_470_588_24).abs() < 1e-6);
        assert!((branch_pct - 50.0).abs() < f64::EPSILON);
        assert_eq!(below_50, 2);
    }

    #[test]
    fn continuous_gradient_deduction_tracks_uncovered_fraction() {
        let coverage = CoverageReport {
            format: "test".to_string(),
            files: vec![FileCoverage {
                path: "src/half.rs".to_string(),
                lines_total: 100,
                lines_covered: 50,
                branches_total: 0,
                branches_covered: 0,
            }],
        };

        let findings = coverage_biomarkers(&coverage, &HashMap::new(), &HashMap::new());
        let gradient = findings
            .iter()
            .find(|finding| finding.biomarker == "coverage_gradient")
            .unwrap();
        assert_eq!(gradient.category, "coverage_gradient");
        assert_eq!(gradient.severity, Severity::Medium);
        assert_eq!(gradient.deduction, Some(2.0));
    }
}
