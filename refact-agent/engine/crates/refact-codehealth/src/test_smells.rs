use crate::biomarkers::{Dimension, Finding, Severity};
use std::collections::HashSet;

pub struct AssertionBlock {
    pub function: String,
    pub start_line: usize,
    pub end_line: usize,
    pub count: usize,
}

pub struct ClonePair {
    pub file_a: String,
    pub a_start_line: usize,
    pub a_end_line: usize,
    pub file_b: String,
    pub b_start_line: usize,
    pub b_end_line: usize,
}

pub struct TestSmellInput {
    pub file_path: String,
    pub is_test_file: bool,
    pub assertion_blocks: Vec<AssertionBlock>,
    pub clones: Vec<ClonePair>,
}

pub fn test_smell_biomarkers(input: &TestSmellInput) -> Vec<Finding> {
    if !input.is_test_file {
        return Vec::new();
    }

    let mut out = Vec::new();

    for block in &input.assertion_blocks {
        if block.count < 15 {
            continue;
        }
        out.push(finding(
            "large_assertion_block",
            if block.count >= 30 {
                Severity::High
            } else {
                Severity::Medium
            },
            block.start_line,
            format!(
                "{} contains {} assertions in one block",
                block.function, block.count
            ),
        ));
    }

    if !input.clones.is_empty() && !input.assertion_blocks.is_empty() {
        let mut emitted = HashSet::new();
        for clone in &input.clones {
            let mut spans = Vec::new();
            if clone.file_a == input.file_path {
                spans.push((clone.a_start_line, clone.a_end_line, clone.file_b.as_str()));
            }
            if clone.file_b == input.file_path {
                spans.push((clone.b_start_line, clone.b_end_line, clone.file_a.as_str()));
            }
            for (cs, ce, partner) in spans {
                for block in &input.assertion_blocks {
                    let bs = block.start_line;
                    let be = block.end_line;
                    if cs <= be && bs <= ce && emitted.insert((bs, be)) {
                        out.push(finding(
                            "duplicated_assertion_block",
                            Severity::Medium,
                            bs,
                            format!(
                                "assertion block [{},{}] duplicates assertions in {}",
                                bs, be, partner
                            ),
                        ));
                    }
                }
            }
        }
    }

    out.sort_by(|a, b| a.line.cmp(&b.line).then(a.biomarker.cmp(&b.biomarker)));
    out
}

fn finding(biomarker: &str, severity: Severity, line: usize, detail: String) -> Finding {
    Finding {
        biomarker: biomarker.to_string(),
        category: "test_quality".to_string(),
        dimension: Dimension::Defect,
        severity,
        line,
        detail,
        deduction: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        assertion_blocks: Vec<AssertionBlock>,
        clones: Vec<ClonePair>,
        is_test_file: bool,
    ) -> TestSmellInput {
        TestSmellInput {
            file_path: "tests/example.rs".to_string(),
            is_test_file,
            assertion_blocks,
            clones,
        }
    }

    fn block(function: &str, start_line: usize, end_line: usize, count: usize) -> AssertionBlock {
        AssertionBlock {
            function: function.to_string(),
            start_line,
            end_line,
            count,
        }
    }

    #[test]
    fn large_assertion_block_medium_for_twenty_assertions() {
        let findings = test_smell_biomarkers(&input(
            vec![block("checks_values", 10, 35, 20)],
            vec![],
            true,
        ));

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].biomarker, "large_assertion_block");
        assert_eq!(findings[0].category, "test_quality");
        assert_eq!(findings[0].dimension, Dimension::Defect);
        assert_eq!(findings[0].severity, Severity::Medium);
        assert_eq!(findings[0].line, 10);
        assert!(findings[0].detail.contains("checks_values"));
        assert!(findings[0].detail.contains("20"));
    }

    #[test]
    fn large_assertion_block_high_for_thirty_assertions() {
        let findings = test_smell_biomarkers(&input(
            vec![block("checks_many_values", 3, 40, 30)],
            vec![],
            true,
        ));

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].biomarker, "large_assertion_block");
        assert_eq!(findings[0].severity, Severity::High);
        assert_eq!(findings[0].line, 3);
    }

    #[test]
    fn non_test_file_fires_nothing() {
        let findings = test_smell_biomarkers(&input(
            vec![block("checks_values", 10, 35, 20)],
            vec![ClonePair {
                file_a: "tests/example.rs".to_string(),
                a_start_line: 12,
                a_end_line: 20,
                file_b: "tests/other.rs".to_string(),
                b_start_line: 50,
                b_end_line: 58,
            }],
            false,
        ));

        assert!(findings.is_empty());
    }

    #[test]
    fn duplicated_assertion_block_for_overlapping_clone_span() {
        let findings = test_smell_biomarkers(&input(
            vec![block("checks_values", 10, 35, 12)],
            vec![ClonePair {
                file_a: "tests/example.rs".to_string(),
                a_start_line: 12,
                a_end_line: 20,
                file_b: "tests/other.rs".to_string(),
                b_start_line: 50,
                b_end_line: 58,
            }],
            true,
        ));

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].biomarker, "duplicated_assertion_block");
        assert_eq!(findings[0].category, "test_quality");
        assert_eq!(findings[0].dimension, Dimension::Defect);
        assert_eq!(findings[0].severity, Severity::Medium);
        assert_eq!(findings[0].line, 10);
        assert!(findings[0].detail.contains("[10,35]"));
        assert!(findings[0].detail.contains("tests/other.rs"));
    }
}
