use crate::biomarkers::{Dimension, Finding, Severity};

pub const DRY_DUPLICATION_THRESHOLD: f64 = 0.08;
const MIN_CLONE_LINES: usize = 6;
const ACTIVE_CO_CHANGE: u32 = 3;
const HIGH_DUP_PCT: f64 = 0.25;

pub struct DryClonePair {
    pub file_a: String,
    pub a_start_line: usize,
    pub a_line_count: usize,
    pub file_b: String,
    pub b_start_line: usize,
    pub b_line_count: usize,
    pub co_change_count: u32,
}

pub struct DryInput {
    pub file_path: String,
    pub duplication_pct: f64,
    pub clones: Vec<DryClonePair>,
}

pub fn dry_violation(input: &DryInput) -> Vec<Finding> {
    if input.clones.is_empty() || input.duplication_pct < DRY_DUPLICATION_THRESHOLD {
        return Vec::new();
    }

    let Some(worst) = input
        .clones
        .iter()
        .filter(|clone| clone.a_line_count.max(clone.b_line_count) >= MIN_CLONE_LINES)
        .max_by_key(|clone| {
            (
                clone.co_change_count,
                clone.a_line_count.max(clone.b_line_count),
            )
        })
    else {
        return Vec::new();
    };

    let worst_lines = worst.a_line_count.max(worst.b_line_count);

    let file_is_a = worst.file_a == input.file_path;
    let partner = if file_is_a {
        &worst.file_b
    } else {
        &worst.file_a
    };
    let active = worst.co_change_count >= ACTIVE_CO_CHANGE;
    let severity = if active && input.duplication_pct >= HIGH_DUP_PCT {
        Severity::High
    } else if active || input.duplication_pct >= HIGH_DUP_PCT {
        Severity::Medium
    } else {
        Severity::Low
    };
    let line = if file_is_a {
        worst.a_start_line
    } else {
        worst.b_start_line
    };

    vec![Finding {
        biomarker: "dry_violation".to_string(),
        category: "duplication".to_string(),
        dimension: Dimension::Defect,
        severity,
        line,
        detail: format!(
            "duplication_pct={:.2}, clone_pair_count={}, worst_clone_lines={}, partner={}, worst_clone_co_change={}",
            input.duplication_pct,
            input.clones.len(),
            worst_lines,
            partner,
            worst.co_change_count
        ),
        deduction: None,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clone_pair(
        file_a: &str,
        a_start_line: usize,
        a_line_count: usize,
        file_b: &str,
        b_start_line: usize,
        b_line_count: usize,
        co_change_count: u32,
    ) -> DryClonePair {
        DryClonePair {
            file_a: file_a.to_string(),
            a_start_line,
            a_line_count,
            file_b: file_b.to_string(),
            b_start_line,
            b_line_count,
            co_change_count,
        }
    }

    #[test]
    fn active_ten_line_clone_at_high_dup_pct_fires_high() {
        let input = DryInput {
            file_path: "a.rs".to_string(),
            duplication_pct: 0.30,
            clones: vec![clone_pair("a.rs", 4, 10, "b.rs", 8, 10, 3)],
        };

        let findings = dry_violation(&input);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::High);
        assert_eq!(findings[0].biomarker, "dry_violation");
        assert_eq!(findings[0].category, "duplication");
        assert_eq!(findings[0].dimension, Dimension::Defect);
        assert_eq!(findings[0].line, 4);
        assert!(findings[0].detail.contains("duplication_pct=0.30"));
        assert!(findings[0].detail.contains("clone_pair_count=1"));
        assert!(findings[0].detail.contains("worst_clone_lines=10"));
        assert!(findings[0].detail.contains("partner=b.rs"));
        assert!(findings[0].detail.contains("worst_clone_co_change=3"));
    }

    #[test]
    fn clone_length_filter_runs_before_worst_clone_ranking() {
        let input = DryInput {
            file_path: "a.rs".to_string(),
            duplication_pct: 0.10,
            clones: vec![
                clone_pair("a.rs", 10, 5, "short.rs", 20, 5, 9),
                clone_pair("a.rs", 30, 12, "eligible.rs", 40, 12, 1),
            ],
        };

        let findings = dry_violation(&input);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 30);
        assert_eq!(findings[0].severity, Severity::Low);
        assert!(findings[0].detail.contains("partner=eligible.rs"));
        assert!(findings[0].detail.contains("worst_clone_lines=12"));
        assert!(findings[0].detail.contains("worst_clone_co_change=1"));
    }
}
